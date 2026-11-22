//! Auxiliary variables, propagation, enforcement of relations and wrap-around, and status.

use super::*;

impl Simulator {
    pub(super) fn csp_aux(csp: &Csp) -> Aux {
        let n = csp.vars.len();
        let mut aux = Aux {
            dists: vec![Vec::new(); n],
            preds: vec![Vec::new(); n],
            succs: vec![Vec::new(); n],
            guard_var: vec![false; n],
            user_succs: vec![Vec::new(); n],
            pop_c: vec![None; n],
            pop_x: vec![None; n],
            seg_hi: vec![Vec::new(); n],
            composite: vec![false; n],
            fun_var: vec![false; n],
        };
        for (i, d) in csp.dists.iter().enumerate() {
            aux.dists[d.var].push(i);
        }
        for &b in csp.segs.keys() {
            aux.composite[b] = aux.dists[b].is_empty();
        }
        for &x in csp.slice_vars.values() {
            aux.composite[x] = aux.dists[x].is_empty();
        }
        // A product variable is fixed by its factors (`Node::Mul`).
        for node in &csp.nodes {
            if let Node::Mul { p, .. } = node {
                aux.composite[*p] = true;
            }
        }
        // A wrapped reading (`Node::Wrap`) is fixed by its variable, which
        // is decided uniformly over its own values.
        for &(.., u) in &csp.wraps {
            aux.composite[u] = true;
        }
        // A variable split into segments (`csp_split_slices`) that some
        // relation reads as a number (`last == base + n`), itself or through
        // a select spanning several segments, is decided from its most
        // significant segment down. Deciding the low bits first leaves a
        // conflict that only they cause (`base[15:0] + n` crossing a 64 KiB
        // page) to be refuted one value of the high bits at a time.
        let mut reads = vec![0u32; n];
        for node in &csp.nodes {
            let mut ds = Vec::new();
            node.deps(&csp.srcs, &mut ds);
            ds.sort_unstable();
            ds.dedup();
            for v in ds {
                reads[v] += 1;
            }
        }
        for (&b, segs) in &csp.segs {
            if csp.wide.contains(&b) || segs.len() < 2 {
                continue;
            }
            // Each is also read by its own definition (`csp_slice_links`).
            let mut ranges: Vec<(u32, u32)> = Vec::new();
            if reads[b] >= 2 {
                ranges.push((0, csp.vars[b].width));
            }
            for (&(sb, lo, w), &x) in &csp.slice_vars {
                if sb == b && reads[x] >= 2 {
                    ranges.push((lo, w));
                }
            }
            for (lo, w) in ranges {
                let inside: Vec<(u32, usize)> = segs
                    .iter()
                    .filter(|s| s.0 >= lo && s.0 + s.1 <= lo + w)
                    .map(|s| (s.0, s.2))
                    .collect();
                for &(slo, sv) in &inside {
                    for &(tlo, tv) in &inside {
                        if tlo > slo && !aux.seg_hi[sv].contains(&tv) {
                            aux.seg_hi[sv].push(tv);
                        }
                    }
                }
            }
        }
        // A counted variable is decided after its count, so it is drawn
        // with the count's number of ones (`csp_pick_val`).
        for (&x, &c) in &csp.pop {
            // A variable under a dist draws by its weights first (§18.5.4);
            // its count then follows it.
            if !aux.dists[x].is_empty() {
                continue;
            }
            aux.pop_c[x] = Some(c);
            aux.pop_x[c] = Some((x, csp.vars[x].width));
            if !aux.preds[x].contains(&c) {
                aux.preds[x].push(c);
                aux.succs[c].push(x);
            }
        }
        // §18.5.10: a variable split into segments (`csp_split_slices`) is
        // decided through them, so an ordering on the variable, or on a
        // select of it, orders the segments it spans as well: otherwise a
        // low segment of `last_addr` is guessed before `base` and the value
        // of `last == base + n` only ever meets it by chance.
        let spread = |vs: &[usize]| -> Vec<usize> {
            let mut out = vs.to_vec();
            for &v in vs {
                let (b, lo, w) = match csp.aux_base.get(&v) {
                    Some(&x) => x,
                    None => (v, 0, csp.vars[v].width),
                };
                if let Some(segs) = csp.segs.get(&b) {
                    out.extend(
                        segs.iter()
                            .filter(|s| s.0 < lo + w && s.0 + s.1 > lo)
                            .map(|s| s.2),
                    );
                }
            }
            out.sort_unstable();
            out.dedup();
            out
        };
        for (before, after) in &csp.order {
            let (before, after) = (spread(before), spread(after));
            for &a in &after {
                for &b in &before {
                    if a != b {
                        aux.preds[a].push(b);
                        aux.succs[b].push(a);
                        aux.user_succs[b].push(a);
                    }
                }
            }
        }
        fn conds(n: &Node, srcs: &[Src], out: &mut Vec<usize>) {
            match n {
                Node::And(ns) | Node::Or(ns) => ns.iter().for_each(|m| conds(m, srcs, out)),
                Node::If {
                    cond, then, els, ..
                } => {
                    cond.deps(srcs, out);
                    conds(then, srcs, out);
                    conds(els, srcs, out);
                }
                _ => {}
            }
        }
        // A `Fun` variable is decided after the variables its value is a
        // function of, so it is narrowed rather than guessed.
        fn funs(n: &Node, srcs: &[Src], aux: &mut Aux) {
            match n {
                Node::And(ns) | Node::Or(ns) => ns.iter().for_each(|m| funs(m, srcs, aux)),
                Node::If { then, els, .. } => {
                    funs(then, srcs, aux);
                    funs(els, srcs, aux);
                }
                Node::Fun { var, other, .. } => {
                    for &d in &srcs[*other].deps {
                        if d != *var && !aux.preds[*var].contains(&d) {
                            aux.preds[*var].push(d);
                            aux.succs[d].push(*var);
                        }
                    }
                }
                _ => {}
            }
        }
        for nd in &csp.nodes {
            funs(nd, &csp.srcs, &mut aux);
        }
        // §18.5.10: an unconditional function relation, at the top or under
        // a conjunction only.
        fn top_funs(n: &Node, aux: &mut Aux) {
            match n {
                Node::And(ns) => ns.iter().for_each(|m| top_funs(m, aux)),
                Node::Fun { var, .. } => aux.fun_var[*var] = true,
                _ => {}
            }
        }
        for nd in &csp.nodes {
            top_funs(nd, &mut aux);
        }
        // `x == k*y + c` (|k| > 1) defines `x` by the others: decided after
        // them it is narrowed to its value instead of guessed among the
        // values no `y` reaches (`max == 40'h800_0000 * total - 1`). Only for
        // a solve with rand sub-objects (§18.5.9), whose sets are larger.
        if csp.has_subs {
            for nd in &csp.nodes {
                let Node::Lin {
                    lin, rel: Rel::Eq, ..
                } = nd
                else {
                    continue;
                };
                if lin.t.len() < 2 || !lin.t.iter().any(|t| t.1.abs() > 1) {
                    continue;
                }
                let units: Vec<usize> = lin
                    .t
                    .iter()
                    .filter(|t| t.1.abs() == 1)
                    .map(|t| t.0)
                    .collect();
                if units.len() != 1 {
                    continue;
                }
                let x = units[0];
                // A split `x` is decided through its segments.
                let mut xs = vec![x];
                if let Some(segs) = csp.segs.get(&x) {
                    xs.extend(segs.iter().map(|s| s.2));
                }
                xs.extend(
                    csp.slice_vars
                        .iter()
                        .filter(|((b, ..), _)| *b == x)
                        .map(|(_, &sv)| sv),
                );
                for &(d, _) in &lin.t {
                    if d == x || aux.preds[d].contains(&x) {
                        continue;
                    }
                    for &y in &xs {
                        if y != d && !aux.preds[y].contains(&d) {
                            aux.preds[y].push(d);
                            aux.succs[d].push(y);
                        }
                    }
                }
            }
        }
        let mut gv = Vec::new();
        for nd in &csp.nodes {
            conds(nd, &csp.srcs, &mut gv);
        }
        // The inputs of a `Fun` relation on a counted `$countones` count.
        fn pop_inputs(n: &Node, csp: &Csp, aux: &Aux, out: &mut Vec<usize>) {
            match n {
                Node::And(ns) | Node::Or(ns) => {
                    ns.iter().for_each(|m| pop_inputs(m, csp, aux, out))
                }
                Node::If { then, els, .. } => {
                    pop_inputs(then, csp, aux, out);
                    pop_inputs(els, csp, aux, out);
                }
                Node::Fun { var, other, .. } if aux.pop_x[*var].is_some() => {
                    out.extend(csp.srcs[*other].deps.iter().copied());
                }
                _ => {}
            }
        }
        for nd in &csp.nodes {
            pop_inputs(nd, csp, &aux, &mut gv);
        }
        for v in gv {
            aux.guard_var[v] = true;
        }
        aux
    }

    /// Run the queued nodes to a fixpoint. `Some(false)` on a conflict,
    /// `None` when the work budget is gone.
    pub(super) fn csp_propagate(
        &mut self,
        csp: &Csp,
        watch: &[Vec<usize>],
        st: &mut St,
        start: &[usize],
    ) -> Option<bool> {
        let n = csp.nodes.len();
        let mut queued = vec![false; n];
        // Bounds propagation around a cycle of relations that has no
        // solution (`a == b`, both tied to fixed, different low parts)
        // narrows by one step per round and can run through a whole
        // 40-bit range. A node re-run far more often than any chain of the
        // problem needs stops the round: the branch is abandoned, and marked
        // as cut so that it never counts as a proof of unsatisfiability.
        let stall = 1000 + 2 * csp.vars.len() as u32;
        let mut runs = vec![0u32; n];
        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        for &i in start {
            if !queued[i] {
                queued[i] = true;
                queue.push_back(i);
            }
        }
        while let Some(i) = queue.pop_front() {
            queued[i] = false;
            runs[i] += 1;
            if runs[i] > stall {
                st.cut = true;
                return Some(false);
            }
            st.work += 1;
            if st.work >= WORK_BUDGET {
                return None;
            }
            st.dirty.clear();
            if !self.csp_enforce(csp, &csp.nodes[i], st) {
                return Some(false);
            }
            let dirty = std::mem::take(&mut st.dirty);
            for v in dirty {
                for &j in &watch[v] {
                    if !queued[j] {
                        queued[j] = true;
                        queue.push_back(j);
                    }
                }
            }
        }
        Some(true)
    }

    /// §18.5: `var OP other` with `other` a function of a few variables of
    /// small domains: `other` takes one of the values its inputs' remaining
    /// combinations give, so `var` is narrowed to them (`total ==
    /// per * (n + 1)` bounds `total` before `per` and `n` are decided).
    /// True (nothing learnt) when the inputs have too many combinations.
    fn csp_fun_range(
        &mut self,
        csp: &Csp,
        st: &mut St,
        var: usize,
        op: BinaryOp,
        other: usize,
    ) -> bool {
        const MAX_COMBOS: u128 = 256;
        let deps = &csp.srcs[other].deps;
        if deps.is_empty() || deps.len() > 4 || deps.contains(&var) {
            return true;
        }
        let mut n: u128 = 1;
        for &d in deps {
            n = n.saturating_mul(dom_size(&st.d[d]));
            if n > MAX_COMBOS {
                return true;
            }
        }
        st.work += n as u64;
        let (vw, vs) = (csp.vars[var].width, csp.vars[var].signed);
        let doms: Vec<Vec<i128>> = deps
            .iter()
            .map(|&d| {
                st.d[d]
                    .iter()
                    .flat_map(|&(l, h)| l..=h)
                    .collect::<Vec<i128>>()
            })
            .collect();
        let mut idx = vec![0usize; deps.len()];
        let mut vals: Vec<(i128, i128)> = Vec::new();
        loop {
            for (k, &d) in deps.iter().enumerate() {
                self.csp_write(csp, d, doms[k][idx[k]]);
            }
            let s = &csp.srcs[other];
            let frame = self.csp_frame(&s.binds);
            self.push_local_frame(frame);
            let c = match &s.item {
                SrcItem::Expr(e) => self.eval_expr(e),
                SrcItem::Item(_) => Value::new(1),
            };
            self.pop_local_frame();
            if c.has_unknown() || c.width > 64 || (vs != c.is_signed && !(!vs && c.width <= vw)) {
                return true;
            }
            // §11.8.2: an unsigned result variable makes the comparison
            // unsigned, so a signed input is read by its zero-extended bits.
            let k = if vs && c.is_signed {
                let x = c.to_u64().unwrap_or(0);
                let sh = 64 - c.width.max(1);
                (((x << sh) as i64) >> sh) as i128
            } else {
                c.to_u64().unwrap_or(0) as i128
            };
            vals.push((k, k));
            let mut j = 0;
            loop {
                if j == idx.len() {
                    let set = dom_norm(vals);
                    const BIG: i128 = i128::MAX / 4;
                    let (lo, hi) = (set.first().unwrap().0, set.last().unwrap().1);
                    let nd = match op {
                        BinaryOp::Eq | BinaryOp::CaseEq => dom_meet(&st.d[var], &set),
                        BinaryOp::Lt => dom_clip(&st.d[var], -BIG, hi - 1),
                        BinaryOp::Leq => dom_clip(&st.d[var], -BIG, hi),
                        BinaryOp::Gt => dom_clip(&st.d[var], lo + 1, BIG),
                        BinaryOp::Geq => dom_clip(&st.d[var], lo, BIG),
                        _ => return true,
                    };
                    return st.set(var, nd);
                }
                idx[j] += 1;
                if idx[j] < doms[j].len() {
                    break;
                }
                idx[j] = 0;
                j += 1;
            }
        }
    }

    /// Make `n` hold under the current domains; false on a conflict.
    fn csp_enforce(&mut self, csp: &Csp, n: &Node, st: &mut St) -> bool {
        match n {
            Node::True => true,
            Node::False => false,
            Node::Lin {
                lin,
                rel,
                fits,
                src,
                sneg,
                wrap,
            } => {
                if !st.fits_hold(fits) {
                    if let Some(wr) = *wrap
                        && st.fits_hold_mod(&fits[..wr.n], wr.w)
                        && let Some(ok) = match rel {
                            Rel::Eq => Self::csp_wrap_eq(lin, wr.w, st),
                            _ => Self::csp_wrap_rel(lin, *rel, fits, wr, st),
                        }
                    {
                        return ok;
                    }
                    return !self.csp_src_fixed(csp, st, *src)
                        || self.csp_eval_src(csp, st, *src) != *sneg;
                }
                match rel {
                    Rel::Le => st.prop_bound(lin, true),
                    Rel::Eq => st.prop_bound(lin, true) && st.prop_bound(lin, false),
                    Rel::Ne => {
                        let mut free: Option<(usize, i128)> = None;
                        let mut rest = lin.k;
                        for &(v, c) in &lin.t {
                            match st.fixed(v) {
                                Some(x) => rest += c * x,
                                None if free.is_none() => free = Some((v, c)),
                                None => return true,
                            }
                        }
                        match free {
                            None => rest != 0,
                            Some((v, c)) => {
                                if (-rest) % c != 0 {
                                    return true;
                                }
                                let x = -rest / c;
                                let nd = dom_minus(&st.d[v], &vec![(x, x)]);
                                st.set(v, nd)
                            }
                        }
                    }
                }
            }
            Node::In {
                lin,
                set,
                neg,
                fits,
                src,
                wrap,
            } => {
                if !st.fits_hold(fits) {
                    if let Some(wr) = *wrap
                        && st.fits_hold_mod(&fits[..wr.n], wr.w)
                        && let Some(ok) = Self::csp_wrap_set(lin, set, *neg, wr.w, wr.s, st)
                    {
                        return ok;
                    }
                    return !self.csp_src_fixed(csp, st, *src)
                        || self.csp_eval_src(csp, st, *src) != *neg;
                }
                if let [(v, c)] = lin.t[..] {
                    if c == 1 || c == -1 {
                        let allowed: Dom = if c == 1 {
                            set.iter().map(|&(l, h)| (l - lin.k, h - lin.k)).collect()
                        } else {
                            dom_norm(set.iter().map(|&(l, h)| (lin.k - h, lin.k - l)).collect())
                        };
                        let nd = if *neg {
                            dom_minus(&st.d[v], &allowed)
                        } else {
                            dom_meet(&st.d[v], &allowed)
                        };
                        return st.set(v, nd);
                    }
                }
                let (lo, hi) = st.lin_bounds(lin);
                if lo == hi {
                    return dom_has(set, lo) != *neg;
                }
                if *neg || set.is_empty() {
                    return !set.is_empty() || *neg;
                }
                let (smin, smax) = (set[0].0, set[set.len() - 1].1);
                let (Some(up), Some(dn)) = (lin.clone().shift(-smax), lin.clone().shift(-smin))
                else {
                    return true;
                };
                st.prop_bound(&up, true) && st.prop_bound(&dn, false)
            }
            Node::AllDiff(vs) => {
                let mut taken: Vec<(i128, i128)> = Vec::new();
                for &v in vs {
                    if let Some(x) = st.fixed(v) {
                        taken.push((x, x));
                    }
                }
                let before = taken.len();
                let taken = dom_norm(taken);
                if dom_size(&taken) < before as u128 {
                    return false;
                }
                let mut free = 0u128;
                let mut pool: Vec<(i128, i128)> = Vec::new();
                for &v in vs {
                    if st.fixed(v).is_some() {
                        continue;
                    }
                    let nd = dom_minus(&st.d[v], &taken);
                    if !st.set(v, nd) {
                        return false;
                    }
                    free += 1;
                    pool.extend(st.d[v].iter().copied());
                }
                // pigeonhole: the free variables need that many values
                free == 0 || dom_size(&dom_norm(pool)) >= free
            }
            Node::Mul { p, x, y } => Self::csp_mul_enforce(*p, *x, *y, st),
            Node::Wrap { v, u, w, s } => Self::csp_wrap_enforce(*v, *u, *w, *s, st),
            Node::Pop { x, c, w } => {
                if let Some(v) = st.fixed(*x) {
                    let n = Self::bits_at(v, *w).count_ones() as i128;
                    let nd = dom_meet(&st.d[*c], &vec![(n, n)]);
                    return st.set(*c, nd);
                }
                // A small domain of x: keep the values whose count is still
                // possible, and the counts some value still has.
                if dom_size(&st.d[*x]) <= 64 {
                    let mut xs = Vec::new();
                    let mut ns = Vec::new();
                    for &(l, h) in &st.d[*x] {
                        for v in l..=h {
                            let n = Self::bits_at(v, *w).count_ones() as i128;
                            if dom_has(&st.d[*c], n) {
                                xs.push((v, v));
                                ns.push((n, n));
                            }
                        }
                    }
                    return st.set(*x, dom_norm(xs)) && st.set(*c, dom_norm(ns));
                }
                // Otherwise bound the count by x's range: a value in [lo, hi]
                // with 0 <= lo has at most max(ones(hi), bits(hi) - 1) ones,
                // and at least one when lo > 0.
                let (lo, hi) = st.bounds(*x);
                if lo >= 0 {
                    let h = hi as u128;
                    let bl = 128 - h.leading_zeros();
                    let cmax = h.count_ones().max(bl.saturating_sub(1)) as i128;
                    let nd = dom_clip(&st.d[*c], (lo > 0) as i128, cmax);
                    return st.set(*c, nd);
                }
                true
            }
            Node::Eval { src, neg } => {
                if self.csp_src_fixed(csp, st, *src) {
                    return self.csp_eval_src(csp, st, *src) != *neg;
                }
                self.csp_eval_forward(csp, st, *src, *neg)
            }
            Node::Fun {
                var,
                op,
                other,
                whole,
                neg,
            } => {
                if !self.csp_src_fixed(csp, st, *other) {
                    if csp.srcs[*other].stale {
                        return true;
                    }
                    return self.csp_fun_range(csp, st, *var, *op, *other);
                }
                if self.csp_src_fixed(csp, st, *whole) {
                    return self.csp_eval_src(csp, st, *whole) != *neg;
                }
                let c = self.csp_eval_value(csp, st, *other);
                if c.has_unknown() {
                    // §11.4.4: a relation with an x/z operand is not true.
                    return false;
                }
                let (vw, vs) = (csp.vars[*var].width, csp.vars[*var].signed);
                let k: i128 = match (vs, c.is_signed) {
                    (false, false) => match c.to_u64() {
                        Some(x) => x as i128,
                        None => return true,
                    },
                    (true, true) if c.width <= 64 => {
                        let x = c.to_u64().unwrap_or(0);
                        let sh = 64 - c.width.max(1);
                        (((x << sh) as i64) >> sh) as i128
                    }
                    // §11.8.1/§11.8.2: with an unsigned side the relation is
                    // unsigned, at the wider width, and each operand is
                    // zero-extended. A signed variable that cannot be
                    // negative (a `$countones` count against `2**size`)
                    // keeps its value; the other side, at least as wide,
                    // is its own self-determined value.
                    (true, false) if st.bounds(*var).0 >= 0 && c.width >= vw => match c.to_u64() {
                        Some(x) => x as i128,
                        None => return true,
                    },
                    // A signed variable that can be negative reads as its
                    // bits unsigned, `var + 2^vw` below zero: the values
                    // that read as an allowed one (`row_bytes ==
                    // ((w + 15) / 16) * 128` with an `int row_bytes`, #994).
                    (true, false) if vw <= 64 => {
                        let Some(k) = c.to_u64().map(i128::from) else {
                            return true;
                        };
                        let (top, half) = ((1i128 << vw) - 1, 1i128 << (vw - 1).min(63));
                        let read: Dom = match op {
                            BinaryOp::Eq | BinaryOp::CaseEq => vec![(k, k)],
                            BinaryOp::Neq | BinaryOp::CaseNeq => {
                                dom_minus(&vec![(0, top)], &vec![(k, k)])
                            }
                            BinaryOp::Lt => vec![(0, k - 1)],
                            BinaryOp::Leq => vec![(0, k)],
                            BinaryOp::Gt => vec![(k + 1, top)],
                            _ => vec![(k, top)],
                        };
                        let read = dom_clip(&read, 0, top);
                        let mut vals = dom_clip(&read, 0, half - 1);
                        vals.extend(
                            dom_clip(&read, half, top)
                                .into_iter()
                                .map(|(a, b)| (a - (top + 1), b - (top + 1))),
                        );
                        let nd = dom_meet(&st.d[*var], &dom_norm(vals));
                        return st.set(*var, nd);
                    }
                    // §11.8.2: an unsigned variable makes the relation
                    // unsigned; the signed side is zero-extended to it.
                    (false, true) if c.width <= vw => match c.to_u64() {
                        Some(x) => x as i128,
                        None => return true,
                    },
                    // Mixed signedness: leave it to the final judgement.
                    _ => return true,
                };
                let _ = vw;
                const BIG: i128 = i128::MAX / 4;
                let nd = match op {
                    BinaryOp::Eq | BinaryOp::CaseEq => dom_meet(&st.d[*var], &vec![(k, k)]),
                    BinaryOp::Neq | BinaryOp::CaseNeq => dom_minus(&st.d[*var], &vec![(k, k)]),
                    BinaryOp::Lt => dom_clip(&st.d[*var], -BIG, k - 1),
                    BinaryOp::Leq => dom_clip(&st.d[*var], -BIG, k),
                    BinaryOp::Gt => dom_clip(&st.d[*var], k + 1, BIG),
                    _ => dom_clip(&st.d[*var], k, BIG),
                };
                st.set(*var, nd)
            }
            Node::And(ns) => ns.iter().all(|m| self.csp_enforce(csp, m, st)),
            Node::Or(ns) => {
                let mut open: Option<&Node> = None;
                let mut n_open = 0;
                for m in ns {
                    match self.csp_status(csp, m, st) {
                        Some(true) => return true,
                        Some(false) => {}
                        None => {
                            n_open += 1;
                            open = Some(m);
                        }
                    }
                }
                match n_open {
                    0 => false,
                    1 => self.csp_enforce(csp, open.unwrap(), st),
                    _ => true,
                }
            }
            Node::If {
                cond,
                ncond,
                then,
                els,
            } => match self.csp_status(csp, cond, st) {
                Some(true) => self.csp_enforce(csp, then, st),
                Some(false) => self.csp_enforce(csp, els, st),
                None => {
                    if self.csp_status(csp, then, st) == Some(false) {
                        return self.csp_enforce(csp, ncond, st);
                    }
                    if self.csp_status(csp, els, st) == Some(false) {
                        return self.csp_enforce(csp, cond, st);
                    }
                    // A branch that is not refuted outright may still fail
                    // once enforced (two bounds of one wrapped side that
                    // cross): enforce each on its own, take the other when
                    // one fails, and keep what both leave of a variable they
                    // both narrow. Otherwise the condition's variable, or a
                    // variable both branches bound alike, is refuted one
                    // value at a time. Not nested: a probe takes no probes.
                    if st.probing {
                        return true;
                    }
                    let then_d = self.csp_probe(csp, &[cond, then], st);
                    let els_d = self.csp_probe(csp, &[ncond, els], st);
                    match (then_d, els_d) {
                        (None, None) => false,
                        (None, Some(_)) => {
                            self.csp_enforce(csp, ncond, st) && self.csp_enforce(csp, els, st)
                        }
                        (Some(_), None) => {
                            self.csp_enforce(csp, cond, st) && self.csp_enforce(csp, then, st)
                        }
                        (Some(t), Some(e)) => {
                            for (v, dt) in t {
                                if let Some((_, de)) = e.iter().find(|(u, _)| *u == v) {
                                    let mut both = dt;
                                    both.extend(de.iter().copied());
                                    let nd = dom_meet(&st.d[v], &dom_norm(both));
                                    if !st.set(v, nd) {
                                        return false;
                                    }
                                }
                            }
                            true
                        }
                    }
                }
            },
        }
    }

    /// The domains enforcing `ns` in turn leaves to the variables it
    /// narrows, None when it fails; the domains are restored either way.
    fn csp_probe(&mut self, csp: &Csp, ns: &[&Node], st: &mut St) -> Option<Vec<(usize, Dom)>> {
        let (mark, dmark) = (st.trail.len(), st.dirty.len());
        st.probing = true;
        let ok = ns.iter().all(|n| self.csp_enforce(csp, n, st));
        st.probing = false;
        let out = ok.then(|| {
            let mut vs: Vec<usize> = st.trail[mark..].iter().map(|t| t.0).collect();
            vs.sort_unstable();
            vs.dedup();
            vs.into_iter().map(|v| (v, st.d[v].clone())).collect()
        });
        st.undo(mark);
        st.dirty.truncate(dmark);
        out
    }

    /// `u` is `v` wrapped into `(w, s)` (`Node::Wrap`): each period of `v`'s
    /// domain maps onto `u`'s type range, and `u`'s values back onto every
    /// period `v`'s bounds reach (at most a few: `v` is a variable of `w`
    /// bits or fewer).
    fn csp_wrap_enforce(v: usize, u: usize, w: u32, s: bool, st: &mut St) -> bool {
        let m = 1i128 << w;
        let (rlo, rhi) = ws_range(w, s);
        let (vlo, vhi) = st.bounds(v);
        if vlo > vhi {
            return false;
        }
        let (j0, j1) = (floor_div(vlo - rlo, m), floor_div(vhi - rlo, m));
        if j1 - j0 > 4 {
            return true;
        }
        let mut to_u: Vec<(i128, i128)> = Vec::new();
        for j in j0..=j1 {
            let k = j * m;
            to_u.extend(
                dom_clip(&st.d[v], rlo + k, rhi + k)
                    .iter()
                    .map(|&(a, b)| (a - k, b - k)),
            );
        }
        let nu = dom_meet(&st.d[u], &dom_norm(to_u));
        if !st.set(u, nu) {
            return false;
        }
        let mut to_v: Vec<(i128, i128)> = Vec::new();
        for j in j0..=j1 {
            let k = j * m;
            to_v.extend(st.d[u].iter().map(|&(a, b)| (a + k, b + k)));
        }
        let nv = dom_meet(&st.d[v], &dom_norm(to_v));
        st.set(v, nv)
    }

    /// `p == x * y` (`Node::Mul`): `p` within the product of the factors'
    /// bounds, and a factor within `p / a` once the other is fixed at `a`.
    fn csp_mul_enforce(p: usize, x: usize, y: usize, st: &mut St) -> bool {
        if x == y {
            // §11.4.3: a square's factors are one variable. Invert each
            // product interval through integer roots, retaining both signs.
            let mut products = Vec::new();
            for &(lo, hi) in &st.d[x] {
                let (Some(a), Some(b)) = (lo.checked_mul(lo), hi.checked_mul(hi)) else {
                    return true;
                };
                products.push((if lo <= 0 && hi >= 0 { 0 } else { a.min(b) }, a.max(b)));
            }
            let domain = dom_meet(&st.d[p], &dom_norm(products));
            if !st.set(p, domain) {
                return false;
            }
            let mut roots = Vec::new();
            for &(lo, hi) in &st.d[p] {
                if hi < 0 {
                    continue;
                }
                let lo = lo.max(0) as u128;
                let hi = hi as u128;
                let mut first = lo.isqrt();
                if first * first < lo {
                    first += 1;
                }
                let last = hi.isqrt();
                if first <= last {
                    roots.push((first as i128, last as i128));
                    roots.push((-(last as i128), -(first as i128)));
                }
            }
            let domain = dom_meet(&st.d[x], &dom_norm(roots));
            return st.set(x, domain);
        }
        let ((xa, xb), (ya, yb)) = (st.bounds(x), st.bounds(y));
        let cs = [
            xa.checked_mul(ya),
            xa.checked_mul(yb),
            xb.checked_mul(ya),
            xb.checked_mul(yb),
        ];
        if let [Some(a), Some(b), Some(c), Some(d)] = cs {
            let nd = dom_clip(&st.d[p], a.min(b).min(c).min(d), a.max(b).max(c).max(d));
            if !st.set(p, nd) {
                return false;
            }
        }
        let (pa, pb) = st.bounds(p);
        for (f, o) in [(x, y), (y, x)] {
            let Some(a) = st.fixed(f) else {
                continue;
            };
            if a == 0 {
                // `p` already holds 0 alone; the other factor is free.
                continue;
            }
            let (lo, hi) = if a > 0 {
                (ceil_div(pa, a), floor_div(pb, a))
            } else {
                (ceil_div(pb, a), floor_div(pa, a))
            };
            let nd = dom_clip(&st.d[o], lo, hi);
            if !st.set(o, nd) {
                return false;
            }
        }
        true
    }

    /// §11.4.5/§11.8.2: an equality compared at `w` bits holds exactly when
    /// its two sides agree modulo 2^w, so a side that wraps (`a[39:16] ==
    /// b + c` with a 24-bit sum that carries out) still pins the one
    /// variable left open: `x == r (mod 2^w)` keeps the values of the
    /// residue `r` in its domain. None when that does not apply (several
    /// open variables, a coefficient other than ±1, too many candidates).
    fn csp_wrap_eq(lin: &Lin, w: u32, st: &mut St) -> Option<bool> {
        if w == 0 || w > 64 {
            return None;
        }
        let m: i128 = 1i128 << w;
        let mut free: Option<(usize, i128)> = None;
        let mut rest = lin.k;
        for &(v, c) in &lin.t {
            match st.fixed(v) {
                Some(x) => rest += c * x,
                None if free.is_none() => free = Some((v, c)),
                None => return None,
            }
        }
        let Some((v, c)) = free else {
            return Some(rest.rem_euclid(m) == 0);
        };
        if c != 1 && c != -1 {
            return None;
        }
        // c·x ≡ -rest, and c is its own inverse
        let r = (-rest * c).rem_euclid(m);
        let (lo, hi) = st.bounds(v);
        let mut x = r + ceil_div(lo - r, m) * m;
        let mut cands: Vec<(i128, i128)> = Vec::new();
        while x <= hi {
            if cands.len() >= 8 {
                return None;
            }
            cands.push((x, x));
            x += m;
        }
        let nd = dom_meet(&st.d[v], &cands);
        Some(st.set(v, nd))
    }

    /// §11.4.13/§11.8.2: `lin` computed at `w` bits of signedness `s` is
    /// its value wrapped into that type, so with one variable left open
    /// (`c + 4000 inside {[4003:4004]}`, `2 * c inside {[10:11]}`) keep the
    /// values whose wrapped result lies in `set` (outside it with `neg`).
    /// Without this a relation whose operand may overflow waited for the
    /// evaluator until every variable was guessed. None when that does not
    /// apply (several open variables, too many candidates).
    fn csp_wrap_set(lin: &Lin, set: &Dom, neg: bool, w: u32, s: bool, st: &mut St) -> Option<bool> {
        if w == 0 || w > 64 {
            return None;
        }
        let m: i128 = 1i128 << w;
        let (rlo, rhi) = ws_range(w, s);
        let mut free: Option<(usize, i128)> = None;
        let mut rest = lin.k;
        for &(v, c) in &lin.t {
            match st.fixed(v) {
                Some(x) => rest += c * x,
                None if free.is_none() => free = Some((v, c)),
                None => return None,
            }
        }
        let target: Dom = if neg {
            dom_minus(&vec![(rlo, rhi)], set)
        } else {
            dom_clip(set, rlo, rhi)
        };
        let Some((v, c)) = free else {
            return Some(dom_has(&target, rlo + (rest - rlo).rem_euclid(m)));
        };
        let (lo, hi) = st.bounds(v);
        if lo > hi {
            return Some(false);
        }
        const MAX_CANDS: usize = 64;
        let mut allowed: Vec<(i128, i128)> = Vec::new();
        if c == 1 || c == -1 {
            // y = c·x + rest over x in [lo, hi] meets the target once per
            // multiple of 2^w it spans.
            let (ylo, yhi) = if c == 1 {
                (lo + rest, hi + rest)
            } else {
                (rest - hi, rest - lo)
            };
            let (j0, j1) = (floor_div(ylo - rlo, m), floor_div(yhi - rlo, m));
            if j1 - j0 > 8 {
                return None;
            }
            for j in j0..=j1 {
                for &(a, b) in &target {
                    let (a, b) = ((a + j * m).max(ylo), (b + j * m).min(yhi));
                    if a <= b {
                        allowed.push(if c == 1 {
                            (a - rest, b - rest)
                        } else {
                            (rest - b, rest - a)
                        });
                    }
                }
            }
        } else {
            // c·x ≡ t - rest (mod 2^w) for each target value t: solvable
            // when 2^tz(c) divides it, then x is fixed modulo 2^(w - tz).
            if dom_size(&target) > MAX_CANDS as u128 {
                return None;
            }
            let tz = c.trailing_zeros().min(w);
            if tz == w {
                return Some(dom_has(&target, rlo + (rest - rlo).rem_euclid(m)));
            }
            let g = 1i128 << tz;
            let mg = m >> tz;
            let a = ((c >> tz).rem_euclid(mg)) as u128;
            // Newton iteration: the inverse of an odd `a` modulo 2^128.
            let mut inv: u128 = a;
            for _ in 0..7 {
                inv = inv.wrapping_mul(2u128.wrapping_sub(a.wrapping_mul(inv)));
            }
            let mask = (mg - 1) as u128;
            for &(ta, tb) in &target {
                for t in ta..=tb {
                    let r = (t - rest).rem_euclid(m);
                    if r % g != 0 {
                        continue;
                    }
                    let x0 = (((r / g) as u128).wrapping_mul(inv) & mask) as i128;
                    let mut x = x0 + ceil_div(lo - x0, mg) * mg;
                    while x <= hi {
                        if allowed.len() >= MAX_CANDS {
                            return None;
                        }
                        allowed.push((x, x));
                        x += mg;
                    }
                }
            }
        }
        let nd = dom_meet(&st.d[v], &dom_norm(allowed));
        Some(st.set(v, nd))
    }

    /// §11.8.2: a relation (`<=`, `!=`, …) compares its two sides each
    /// wrapped at `w` bits, so once one side is fixed it is a set the other
    /// side's wrapped value must lie in (`c + v0 >= v0 + 3` once `v0` is
    /// known), handed to `csp_wrap_set`. None when both sides are open.
    fn csp_wrap_rel(lin: &Lin, rel: Rel, fits: &[Fit], wr: Wrap, st: &mut St) -> Option<bool> {
        let (la, lb) = (&fits.get(wr.n)?.lin, &fits.get(wr.n + 1)?.lin);
        let (w, s, sg) = (wr.w, wr.s, wr.sg);
        if w == 0 || w > 64 {
            return None;
        }
        let m: i128 = 1i128 << w;
        let (rlo, rhi) = ws_range(w, s);
        let wrapv = |y: i128| rlo + (y - rlo).rem_euclid(m);
        // `lin` is sg·(la - lb) + δ
        let delta = lin.k - sg * (la.k - lb.k);
        let value = |l: &Lin, st: &St| -> Option<i128> {
            let mut x = l.k;
            for &(v, c) in &l.t {
                x += c * st.fixed(v)?;
            }
            Some(x)
        };
        let holds = |d: i128| match rel {
            Rel::Le => d <= 0,
            Rel::Eq => d == 0,
            Rel::Ne => d != 0,
        };
        let (va, vb) = (value(la, st), value(lb, st));
        // The open side `x` and the other side's wrapped value `k`:
        // sg·(x - k) + δ (left open) or sg·(k - x) + δ (right open).
        let (x, k, left) = match (va, vb) {
            (Some(a), Some(b)) => return Some(holds(sg * (wrapv(a) - wrapv(b)) + delta)),
            (None, Some(b)) => (la, wrapv(b), true),
            (Some(a), None) => (lb, wrapv(a), false),
            (None, None) => {
                // Both sides open. A side whose bounds stay inside the type
                // holds its own value, one of `[kmin, kmax]`: the other
                // side's wrapped value must meet the relation with one of
                // them, a bound rather than a value (`(a - 8'h70) <= top -
                // rb` bounds `a` before `rb` is known, #994).
                let in_type = |l: &Lin| {
                    let (a, b) = st.lin_bounds(l);
                    (a >= rlo && b <= rhi).then_some((a, b))
                };
                let (x, (kmin, kmax), left) = match (in_type(lb), in_type(la)) {
                    (Some(r), _) => (la, r, true),
                    (None, Some(r)) => (lb, r, false),
                    (None, None) => return None,
                };
                let set = match rel {
                    Rel::Le => {
                        let r = if (sg == 1) == left {
                            (rlo, kmax - delta)
                        } else {
                            (kmin + delta, rhi)
                        };
                        if r.0 <= r.1 { vec![r] } else { Vec::new() }
                    }
                    Rel::Eq if left => vec![(kmin - sg * delta, kmax - sg * delta)],
                    Rel::Eq => vec![(kmin + sg * delta, kmax + sg * delta)],
                    Rel::Ne => return None,
                };
                return Self::csp_wrap_set(x, &dom_norm(set), false, w, s, st);
            }
        };
        let (set, neg) = match rel {
            Rel::Le => {
                // x <= k - δ when the open side counts up, else x >= k + δ
                let up = (sg == 1) == left;
                let r = if up {
                    (rlo, k - delta)
                } else {
                    (k + delta, rhi)
                };
                (if r.0 <= r.1 { vec![r] } else { Vec::new() }, false)
            }
            Rel::Eq | Rel::Ne => {
                // sg·(±(x - k)) + δ == 0: x == k ∓ sg·δ
                let t = if left { k - sg * delta } else { k + sg * delta };
                (vec![(t, t)], rel == Rel::Ne)
            }
        };
        Self::csp_wrap_set(x, &set, neg, w, s, st)
    }

    /// Entailed (`Some(true)`), refuted (`Some(false)`) or open.
    pub(super) fn csp_status(&mut self, csp: &Csp, n: &Node, st: &St) -> Option<bool> {
        match n {
            Node::True => Some(true),
            Node::False => Some(false),
            Node::Lin {
                lin,
                rel,
                fits,
                src,
                sneg,
                ..
            } => {
                if !st.fits_hold(fits) {
                    if self.csp_src_fixed(csp, st, *src) {
                        return Some(self.csp_eval_src(csp, st, *src) != *sneg);
                    }
                    return None;
                }
                let (lo, hi) = st.lin_bounds(lin);
                let eq = if lo == 0 && hi == 0 {
                    Some(true)
                } else if lo > 0 || hi < 0 {
                    Some(false)
                } else {
                    None
                };
                match rel {
                    Rel::Le if hi <= 0 => Some(true),
                    Rel::Le if lo > 0 => Some(false),
                    Rel::Le => None,
                    Rel::Eq => eq,
                    Rel::Ne => eq.map(|b| !b),
                }
            }
            Node::In {
                lin,
                set,
                neg,
                fits,
                src,
                ..
            } => {
                if !st.fits_hold(fits) {
                    if self.csp_src_fixed(csp, st, *src) {
                        return Some(self.csp_eval_src(csp, st, *src) != *neg);
                    }
                    return None;
                }
                let (lo, hi) = st.lin_bounds(lin);
                if lo == hi {
                    return Some(dom_has(set, lo) != *neg);
                }
                if dom_clip(set, lo, hi).is_empty() {
                    return Some(*neg);
                }
                if set.iter().any(|&(l, h)| l <= lo && hi <= h) {
                    return Some(!*neg);
                }
                None
            }
            Node::Wrap { v, u, w, s } => {
                let (rlo, _) = ws_range(*w, *s);
                let wv = rlo + (st.fixed(*v)? - rlo).rem_euclid(1i128 << *w);
                if !dom_has(&st.d[*u], wv) {
                    return Some(false);
                }
                st.fixed(*u).map(|_| true)
            }
            Node::Mul { p, x, y } => {
                let xy = st.fixed(*x)?.checked_mul(st.fixed(*y)?)?;
                if !dom_has(&st.d[*p], xy) {
                    return Some(false);
                }
                st.fixed(*p).map(|_| true)
            }
            Node::Pop { x, c, w } => {
                let n = Self::bits_at(st.fixed(*x)?, *w).count_ones() as i128;
                if !dom_has(&st.d[*c], n) {
                    return Some(false);
                }
                st.fixed(*c).map(|_| true)
            }
            Node::AllDiff(vs) => {
                if !st.all_fixed(vs) {
                    return None;
                }
                let mut xs: Vec<i128> = vs.iter().map(|&v| st.fixed(v).unwrap()).collect();
                xs.sort_unstable();
                Some(xs.windows(2).all(|w| w[0] != w[1]))
            }
            Node::Eval { src, neg }
            | Node::Fun {
                whole: src, neg, ..
            } => {
                if self.csp_src_fixed(csp, st, *src) {
                    Some(self.csp_eval_src(csp, st, *src) != *neg)
                } else {
                    None
                }
            }
            Node::And(ns) => {
                let mut all = true;
                for m in ns {
                    match self.csp_status(csp, m, st) {
                        Some(false) => return Some(false),
                        Some(true) => {}
                        None => all = false,
                    }
                }
                all.then_some(true)
            }
            Node::Or(ns) => {
                let mut none = true;
                for m in ns {
                    match self.csp_status(csp, m, st) {
                        Some(true) => return Some(true),
                        Some(false) => {}
                        None => none = false,
                    }
                }
                none.then_some(false)
            }
            Node::If {
                cond, then, els, ..
            } => match self.csp_status(csp, cond, st) {
                Some(true) => self.csp_status(csp, then, st),
                Some(false) => self.csp_status(csp, els, st),
                None => {
                    let (t, e) = (
                        self.csp_status(csp, then, st),
                        self.csp_status(csp, els, st),
                    );
                    if t.is_some() && t == e { t } else { None }
                }
            },
        }
    }

    /// A constraint left to the evaluator with one variable still open, of
    /// a few values: keep the values for which it holds (`(base &
    /// ((1 << sz) - 1)) == 0` leaves the aligned low bits of `base` once
    /// `sz` and its high bits are fixed). Without this the search guessed
    /// the last variable and judged the guess, and a conflict caused by an
    /// earlier decision was refuted one value of a later one at a time.
    fn csp_eval_forward(&mut self, csp: &Csp, st: &mut St, src: usize, neg: bool) -> bool {
        if csp.srcs[src].stale {
            return true;
        }
        let mut open = None;
        for &v in &csp.srcs[src].deps {
            if st.fixed(v).is_none() {
                if open.is_some() {
                    return true;
                }
                open = Some(v);
            }
        }
        let Some(v) = open else {
            return true;
        };
        let n = dom_size(&st.d[v]);
        if n > EST_MAX_DOM || csp.wide.contains(&v) || csp.segs.contains_key(&v) {
            return true;
        }
        // Writing the fixed variable it is part of would overwrite it.
        if let Some(&(b, ..)) = csp.aux_base.get(&v)
            && csp.srcs[src].deps.contains(&b)
        {
            return true;
        }
        st.work += n as u64;
        let vals: Vec<i128> = (0..n).map(|k| dom_nth(&st.d[v], k)).collect();
        let mut keep = Vec::with_capacity(vals.len());
        for x in vals {
            self.csp_write(csp, v, x);
            if self.csp_eval_src(csp, st, src) != neg {
                keep.push((x, x));
            }
        }
        st.set(v, dom_norm(keep))
    }
}
