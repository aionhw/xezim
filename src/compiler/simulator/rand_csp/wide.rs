//! Random constraints on packed values wider than 64 bits.
use super::*;

/// Two-state arithmetic modulo 2^w on little-endian 64-bit words (the
/// constants of a wide relation, see `csp_wide_affine`).
fn ww_mask(a: &mut [u64], w: u32) {
    let r = w % 64;
    if r != 0
        && let Some(top) = a.get_mut(((w - 1) / 64) as usize)
    {
        *top &= (1u64 << r) - 1;
    }
}

fn ww_add(a: &[u64], b: &[u64], w: u32) -> Vec<u64> {
    let mut out = vec![0u64; a.len()];
    let mut carry = 0u128;
    for (i, o) in out.iter_mut().enumerate() {
        let t = u128::from(a[i]) + u128::from(b.get(i).copied().unwrap_or(0)) + carry;
        *o = t as u64;
        carry = t >> 64;
    }
    ww_mask(&mut out, w);
    out
}

fn ww_neg(a: &[u64], w: u32) -> Vec<u64> {
    let inv: Vec<u64> = a.iter().map(|x| !x).collect();
    let mut one = vec![0u64; a.len()];
    if let Some(f) = one.first_mut() {
        *f = 1;
    }
    ww_add(&inv, &one, w)
}

fn ww_sub(a: &[u64], b: &[u64], w: u32) -> Vec<u64> {
    ww_add(a, &ww_neg(b, w), w)
}

fn ww_cmp(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    for i in (0..a.len()).rev() {
        match a[i].cmp(&b[i]) {
            std::cmp::Ordering::Equal => {}
            o => return o,
        }
    }
    std::cmp::Ordering::Equal
}

/// A `w`-bit operand of a relation on wide variables: constant bits with
/// solver variables laid over some ranges (each read whole, unsigned and at
/// most 64 bits wide: a segment of a wide variable or a narrow variable).
#[derive(Clone, Debug)]
struct WOp {
    w: u32,
    /// Constant bits, little-endian words; zero under the variables.
    bits: Vec<u64>,
    /// (lsb, width, variable), disjoint.
    vars: Vec<(u32, u32, usize)>,
}

impl WOp {
    fn zero(w: u32) -> WOp {
        WOp {
            w,
            bits: vec![0; w.div_ceil(64) as usize],
            vars: Vec::new(),
        }
    }
    fn bit(&self, i: u32) -> bool {
        i < self.w && (self.bits[(i / 64) as usize] >> (i % 64)) & 1 == 1
    }
    fn set(&mut self, i: u32, b: bool) {
        if i < self.w {
            let m = 1u64 << (i % 64);
            if b {
                self.bits[(i / 64) as usize] |= m;
            } else {
                self.bits[(i / 64) as usize] &= !m;
            }
        }
    }
    /// §11.8.2: a constant extended to the context width, by sign when
    /// the context and the constant are both signed.
    fn konst(w: u32, v: &Value, sext: bool) -> WOp {
        let mut o = WOp::zero(w);
        let vw = v.width.max(1);
        let sign = sext && v.get_bit(vw as usize - 1) == LogicBit::One;
        for i in 0..w {
            let b = if i < vw {
                v.get_bit(i as usize) == LogicBit::One
            } else {
                sign
            };
            o.set(i, b);
        }
        o
    }
    /// The constant bits `[a, b)` (at most 64) as an integer.
    fn word(&self, a: u32, b: u32) -> u64 {
        let mut x = 0u64;
        for i in a..b {
            if self.bit(i) {
                x |= 1u64 << (i - a);
            }
        }
        x
    }
    /// `>> k` (logical): None when a variable would be cut.
    fn shr(&self, k: u64) -> Option<WOp> {
        let mut o = WOp::zero(self.w);
        if k >= u64::from(self.w) {
            return Some(o);
        }
        let k = k as u32;
        for i in 0..self.w - k {
            o.set(i, self.bit(i + k));
        }
        for &(lo, w, v) in &self.vars {
            if lo >= k {
                o.vars.push((lo - k, w, v));
            } else if lo + w > k {
                return None;
            }
        }
        Some(o)
    }
    /// `<< k`: None when a variable would be cut.
    fn shl(&self, k: u64) -> Option<WOp> {
        let mut o = WOp::zero(self.w);
        if k >= u64::from(self.w) {
            return Some(o);
        }
        let k = k as u32;
        for i in k..self.w {
            o.set(i, self.bit(i - k));
        }
        for &(lo, w, v) in &self.vars {
            if lo + w + k <= self.w {
                o.vars.push((lo + k, w, v));
            } else if lo + k < self.w {
                return None;
            }
        }
        Some(o)
    }
    /// `& m` (`or` false) or `| m` (`or` true) with a constant `m`: each
    /// variable must lie wholly under ones or wholly under zeros of `m`.
    fn mask(&self, m: &WOp, or: bool) -> Option<WOp> {
        let mut o = self.clone();
        for (a, b) in o.bits.iter_mut().zip(&m.bits) {
            if or {
                *a |= b;
            } else {
                *a &= b;
            }
        }
        o.vars.clear();
        for &(lo, w, v) in &self.vars {
            let ones = (lo..lo + w).filter(|&i| m.bit(i)).count() as u32;
            // `&`: kept under ones; `|`: kept under zeros (else the bits
            // are the constant ones already in `bits`).
            let keep = if or { ones == 0 } else { ones == w };
            let drop = if or { ones == w } else { ones == 0 };
            if keep {
                o.vars.push((lo, w, v));
            } else if !drop {
                return None;
            }
        }
        Some(o)
    }
}

/// §11.4.4/§11.4.5/§11.5.1: relations on rand variables wider than 64 bits.
/// The solver's domains are 64-bit intervals, so a wide variable is held as
/// segments of at most 64 bits, split at every multiple of 64, at the sign
/// bit of a signed variable and at every boundary its constraints use (part
/// and bit selects, struct fields, shift amounts, mask runs). A relation
/// becomes per-chunk relations on those segments: `==` an equality per
/// chunk, `!=` a disjunction, and `<`, `<=`, `>`, `>=` the lexicographic
/// order of the chunks, most significant first (signed: the sign bit
/// compares the other way). Anything else on a wide variable is not
/// modelled and leaves the class to the trials.
impl Simulator {
    /// The wide rand members among `names` that no constraint reads, or
    /// reads only through `size()` / `$size` (§7.5.1, §7.10.1). Empty when
    /// a constraint calls a user method, which may read any member.
    pub(super) fn csp_wide_unread(
        constraints: &[ClassConstraint],
        names: &[&str],
    ) -> HashSet<String> {
        let mut read: HashSet<String> = HashSet::default();
        let mut opaque = false;
        for c in constraints {
            for it in &c.items {
                Self::csp_wide_item_reads(it, &mut read, &mut opaque);
            }
        }
        if opaque {
            return HashSet::default();
        }
        // A member of an unpacked struct (`s.x`) is read through its leaf.
        names
            .iter()
            .filter(|n| {
                !read.contains(**n) && !n.rsplit_once('.').is_some_and(|p| read.contains(p.1))
            })
            .map(|n| n.to_string())
            .collect()
    }

    fn csp_wide_item_reads(it: &ConstraintItem, read: &mut HashSet<String>, opaque: &mut bool) {
        Self::csp_wide_item_reads_with(it, read, opaque, &|_| false);
    }

    fn csp_wide_item_reads_with(
        it: &ConstraintItem,
        read: &mut HashSet<String>,
        opaque: &mut bool,
        skip_member: &dyn Fn(&Expression) -> bool,
    ) {
        let ex = |e: &Expression, read: &mut HashSet<String>, opaque: &mut bool| {
            Self::csp_wide_expr_reads_with(e, read, opaque, skip_member)
        };
        match it {
            ConstraintItem::Expr(e) => ex(e, read, opaque),
            ConstraintItem::Inside {
                expr,
                range,
                dist_weights,
                ..
            } => {
                ex(expr, read, opaque);
                for r in range {
                    match r {
                        ConstraintRange::Value(v) => ex(v, read, opaque),
                        ConstraintRange::Range { lo, hi } => {
                            ex(lo, read, opaque);
                            ex(hi, read, opaque);
                        }
                    }
                }
                for w in dist_weights.iter().flatten() {
                    match w {
                        DistWeight::Each(e) | DistWeight::Total(e) => ex(e, read, opaque),
                    }
                }
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                ex(condition, read, opaque);
                Self::csp_wide_item_reads_with(constraint, read, opaque, skip_member);
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                ex(condition, read, opaque);
                Self::csp_wide_item_reads_with(then_item, read, opaque, skip_member);
                if let Some(e) = else_item {
                    Self::csp_wide_item_reads_with(e, read, opaque, skip_member);
                }
            }
            ConstraintItem::Foreach { array, item, .. } => {
                ex(array, read, opaque);
                Self::csp_wide_item_reads_with(item, read, opaque, skip_member);
            }
            ConstraintItem::Solve { before, after, .. } => {
                read.extend(before.iter().chain(after).map(|i| i.name.clone()));
            }
            ConstraintItem::Soft(i) => Self::csp_wide_item_reads_with(i, read, opaque, skip_member),
            ConstraintItem::Block(items) => {
                for i in items {
                    Self::csp_wide_item_reads_with(i, read, opaque, skip_member);
                }
            }
            ConstraintItem::Unique { exprs, .. } => {
                for e in exprs {
                    ex(e, read, opaque);
                }
            }
        }
    }

    /// Every name `e` mentions (conservatively: each segment of a path).
    fn csp_wide_expr_reads_with(
        e: &Expression,
        read: &mut HashSet<String>,
        opaque: &mut bool,
        skip_member: &dyn Fn(&Expression) -> bool,
    ) {
        const ARRAY_METHODS: &[&str] = &[
            "sum",
            "product",
            "and",
            "or",
            "xor",
            "min",
            "max",
            "unique",
            "unique_index",
            "find",
            "find_index",
            "find_first",
            "find_first_index",
            "find_last",
            "find_last_index",
            "exists",
            "num",
        ];
        // A plain member path: its `size()` reads no element.
        let plain = |x: &Expression| match &x.kind {
            ExprKind::Ident(h) => h.path.iter().all(|s| s.selects.is_empty()),
            ExprKind::MemberAccess { expr, .. } => {
                matches!(expr.kind, ExprKind::This | ExprKind::Ident(_))
            }
            _ => false,
        };
        match &e.kind {
            ExprKind::Number(_)
            | ExprKind::StringLiteral(_)
            | ExprKind::TypeLiteral(_)
            | ExprKind::Dollar
            | ExprKind::Null
            | ExprKind::This
            | ExprKind::Empty => {}
            ExprKind::Ident(h) => {
                let mut sel = Vec::new();
                for (i, s) in h.path.iter().enumerate() {
                    if i == 0 || !skip_member(e) {
                        read.insert(s.name.name.clone());
                    }
                    sel.extend(s.selects.iter());
                }
                for x in sel {
                    Self::csp_wide_expr_reads_with(x, read, opaque, skip_member);
                }
            }
            ExprKind::MemberAccess { expr, member } => {
                if !skip_member(e) {
                    read.insert(member.name.clone());
                }
                Self::csp_wide_expr_reads_with(expr, read, opaque, skip_member);
            }
            ExprKind::Call { func, args } => {
                match &func.kind {
                    ExprKind::MemberAccess { expr, member } => {
                        if member.name == "size" && args.is_empty() && plain(expr) {
                            return;
                        }
                        if !ARRAY_METHODS.contains(&member.name.as_str()) {
                            *opaque = true;
                            return;
                        }
                        Self::csp_wide_expr_reads_with(expr, read, opaque, skip_member);
                    }
                    ExprKind::Ident(h)
                        if h.path.len() >= 2 && h.path.iter().all(|s| s.selects.is_empty()) =>
                    {
                        let m = h.path.last().unwrap().name.name.as_str();
                        if m == "size" && args.is_empty() {
                            return;
                        }
                        if !ARRAY_METHODS.contains(&m) {
                            *opaque = true;
                            return;
                        }
                        for s in &h.path[..h.path.len() - 1] {
                            read.insert(s.name.name.clone());
                        }
                    }
                    _ => {
                        *opaque = true;
                        return;
                    }
                }
                for a in args {
                    Self::csp_wide_expr_reads_with(a, read, opaque, skip_member);
                }
            }
            ExprKind::SystemCall { name, args } => {
                if name == "$size" && args.len() == 1 && plain(&args[0]) {
                    return;
                }
                for a in args {
                    Self::csp_wide_expr_reads_with(a, read, opaque, skip_member);
                }
            }
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                Self::csp_wide_expr_reads_with(operand, read, opaque, skip_member)
            }
            ExprKind::Binary { left, right, .. } | ExprKind::Range(left, right) => {
                Self::csp_wide_expr_reads_with(left, read, opaque, skip_member);
                Self::csp_wide_expr_reads_with(right, read, opaque, skip_member);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::csp_wide_expr_reads_with(condition, read, opaque, skip_member);
                Self::csp_wide_expr_reads_with(then_expr, read, opaque, skip_member);
                Self::csp_wide_expr_reads_with(else_expr, read, opaque, skip_member);
            }
            ExprKind::Concatenation(xs) => xs
                .iter()
                .for_each(|x| Self::csp_wide_expr_reads_with(x, read, opaque, skip_member)),
            ExprKind::Replication { count, exprs } => {
                Self::csp_wide_expr_reads_with(count, read, opaque, skip_member);
                exprs
                    .iter()
                    .for_each(|x| Self::csp_wide_expr_reads_with(x, read, opaque, skip_member));
            }
            ExprKind::Inside { expr, ranges } => {
                Self::csp_wide_expr_reads_with(expr, read, opaque, skip_member);
                ranges
                    .iter()
                    .for_each(|x| Self::csp_wide_expr_reads_with(x, read, opaque, skip_member));
            }
            ExprKind::Index { expr, index } => {
                Self::csp_wide_expr_reads_with(expr, read, opaque, skip_member);
                Self::csp_wide_expr_reads_with(index, read, opaque, skip_member);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                Self::csp_wide_expr_reads_with(expr, read, opaque, skip_member);
                Self::csp_wide_expr_reads_with(left, read, opaque, skip_member);
                Self::csp_wide_expr_reads_with(right, read, opaque, skip_member);
            }
            ExprKind::WithClause { expr, filter } => {
                Self::csp_wide_expr_reads_with(expr, read, opaque, skip_member);
                Self::csp_wide_expr_reads_with(filter, read, opaque, skip_member);
            }
            _ => *opaque = true,
        }
    }

    /// Split every wide variable into segments: at each multiple of 64,
    /// the sign bit, and the boundaries its constraints use (`uses`).
    pub(super) fn csp_wide_split(
        &mut self,
        csp: &mut Csp,
        uses: &HashMap<usize, (Vec<(u32, u32)>, bool)>,
        links: &[(usize, u32, usize, u32, u32)],
    ) {
        let mut wides: Vec<usize> = csp.wide.iter().copied().collect();
        wides.sort_unstable();
        let mut cuts_of: HashMap<usize, std::collections::BTreeSet<u32>> = HashMap::default();
        for &v in &wides {
            let w = csp.vars[v].width;
            let mut cuts: std::collections::BTreeSet<u32> = [0, w].into_iter().collect();
            if csp.vars[v].signed {
                cuts.insert(w - 1);
            }
            if let Some((ranges, _)) = uses.get(&v) {
                for &(lo, rw) in ranges {
                    cuts.extend([lo, lo + rw].into_iter().filter(|&c| c <= w));
                }
            }
            cuts_of.insert(v, cuts);
        }
        // Two wide ranges a relation compares (`b.w == a.w`), and the
        // elements of one wide array, are cut alike: the relation is then a
        // relation per segment (`csp_wide_chunks`).
        let mut pairs: Vec<(usize, u32, usize, u32, u32)> = links
            .iter()
            .copied()
            .filter(|l| cuts_of.contains_key(&l.0) && cuts_of.contains_key(&l.2))
            .collect();
        for a in csp.arrays.values() {
            if let Some(&(_, e0)) = a.elems.first()
                && cuts_of.contains_key(&e0)
            {
                let w = csp.vars[e0].width;
                pairs.extend(a.elems.iter().skip(1).map(|e| (e0, 0, e.1, 0, w)));
            }
        }
        let align = |cuts_of: &mut HashMap<usize, std::collections::BTreeSet<u32>>| {
            for _ in 0..8 {
                let mut changed = false;
                for &(a, la, b, lb, w) in &pairs {
                    for (x, lx, y, ly) in [(a, la, b, lb), (b, lb, a, la)] {
                        let add: Vec<u32> = cuts_of[&x]
                            .range(lx + 1..lx + w)
                            .map(|&c| c - lx + ly)
                            .collect();
                        let cy = cuts_of.get_mut(&y).unwrap();
                        for c in add {
                            changed |= cy.insert(c);
                        }
                    }
                }
                if !changed {
                    break;
                }
            }
        };
        if !pairs.is_empty() {
            align(&mut cuts_of);
        }
        // A stretch no constraint divides is cut at the multiples of 64
        // inside it, so a field or select stays one segment.
        for cuts in cuts_of.values_mut() {
            let v: Vec<u32> = cuts.iter().copied().collect();
            for p in v.windows(2) {
                if p[1] - p[0] > 64 {
                    cuts.extend((p[0] / 64 + 1..).map(|k| k * 64).take_while(|&c| c < p[1]));
                }
            }
        }
        if !pairs.is_empty() {
            align(&mut cuts_of);
        }
        let mut plan: Vec<(usize, Vec<u32>)> = Vec::with_capacity(wides.len());
        let mut total = csp.vars.len();
        for v in wides {
            let cuts: Vec<u32> = cuts_of[&v].iter().copied().collect();
            total += cuts.len() - 1;
            plan.push((v, cuts));
        }
        if total > MAX_VARS {
            csp.wide_fail = true;
            return;
        }
        for (v, cuts) in plan {
            let mut segs = Vec::with_capacity(cuts.len() - 1);
            for k in 0..cuts.len() - 1 {
                let (lo, hi) = (cuts[k], cuts[k + 1]);
                let x = Self::csp_aux_var(csp, hi - lo);
                csp.aux_base.insert(x, (v, lo, hi - lo));
                csp.wide_seg.insert(x);
                segs.push((lo, hi - lo, x));
            }
            // A select spanning several segments (`f[k]` over `f[k][4:0]`)
            // is a variable of its own, tied to them (`csp_slice_links`).
            if csp.has_subs
                && let Some((ranges, _)) = uses.get(&v)
            {
                for &(lo, rw) in ranges {
                    let covered = segs
                        .iter()
                        .filter(|s| s.0 >= lo && s.0 + s.1 <= lo + rw)
                        .count();
                    if rw <= 64
                        && covered > 1
                        && !csp.slice_vars.contains_key(&(v, lo, rw))
                        && csp.vars.len() < MAX_VARS
                    {
                        let x = Self::csp_aux_var(csp, rw);
                        csp.aux_base.insert(x, (v, lo, rw));
                        csp.slice_vars.insert((v, lo, rw), x);
                    }
                }
            }
            csp.segs.insert(v, segs);
        }
    }

    /// The object declaring solver scalar `name` and its member name there:
    /// the sub-object for a `prop.x` variable (`csp_add_sub`), the object
    /// being solved otherwise.
    pub(super) fn csp_owner<'a>(csp: &'a Csp, name: &'a str) -> (usize, &'a str) {
        if let Some(VarKey::Sub(h, n)) = csp.scalars.get(name).map(|&v| &csp.vars[v].key) {
            return (*h, n.as_str());
        }
        // A sub-object collection keeps its actual `<handle>#member[i]`
        // storage keys. Packed element fields/dimensions belong to that
        // declaring object's class, just as a sub-object scalar does.
        if let Some(VarKey::Elem(key)) = csp
            .arrays
            .get(name)
            .and_then(|a| a.elems.first())
            .map(|&(_, v)| &csp.vars[v].key)
            && let Some((h, member)) = key.split_once('#')
            && let Ok(h) = h.parse()
        {
            return (h, member.split_once('[').map_or(member, |(p, _)| p));
        }
        (csp.handle, name)
    }

    /// §7.4.5: the fields of struct scalar `name` (nested ones included)
    /// that are packed arrays of elements, as ((lsb, width), (left, right,
    /// element width)) of their first dimension.
    #[allow(clippy::type_complexity)]
    pub(super) fn csp_field_arrays(
        &self,
        csp: &Csp,
        name: &str,
        fields: &[(String, u32, u32)],
    ) -> Vec<((u32, u32), (i64, i64, u32))> {
        let (owner, pname) = Self::csp_owner(csp, name);
        let Some(su) = self.class_prop_struct(owner, pname) else {
            return Vec::new();
        };
        let params = self.instance_param_scope(owner);
        let mut out = Vec::new();
        self.csp_field_arrays_walk(&su, &params, 0, fields, &mut out, 0);
        out
    }

    #[allow(clippy::type_complexity)]
    fn csp_field_arrays_walk(
        &self,
        su: &crate::ast::types::StructUnionType,
        params: &HashMap<String, Value>,
        base: u32,
        layout: &[(String, u32, u32)],
        out: &mut Vec<((u32, u32), (i64, i64, u32))>,
        depth: u32,
    ) {
        if depth > 8 {
            return;
        }
        for m in &su.members {
            for d in &m.declarators {
                let Some(&(_, lo, w)) = layout.iter().find(|f| f.0 == d.name.name) else {
                    continue;
                };
                if let DataType::Struct(inner) =
                    Self::resolve_type_ref(&m.data_type, &self.module.typedef_types)
                {
                    if inner.packed {
                        let (sub, _) = self.packed_agg_layout_with(&inner, params);
                        self.csp_field_arrays_walk(&inner, params, base + lo, &sub, out, depth + 1);
                    }
                    continue;
                }
                let Some(dims) = crate::compiler::elaborate::packed_full_dims_chained(
                    &m.data_type,
                    params,
                    &self.module.typedef_types,
                ) else {
                    continue;
                };
                if dims.len() < 2 {
                    continue;
                }
                let (l, r) = dims[0];
                let n = (l - r).unsigned_abs() + 1;
                if n == 0 || u64::from(w) % n != 0 {
                    continue;
                }
                out.push(((base + lo, w), (l, r, (u64::from(w) / n) as u32)));
            }
        }
    }

    /// §7.2.1: the fields of the packed structs nested in a wide struct
    /// variable, listed by their dotted paths (`a.b`) after its own fields.
    pub(super) fn csp_nested_fields(
        &self,
        csp: &Csp,
        name: &str,
        fields: &mut Vec<(String, u32, u32)>,
    ) {
        let (owner, name) = Self::csp_owner(csp, name);
        let Some(su) = self.class_prop_struct(owner, name) else {
            return;
        };
        let params = self.instance_param_scope(owner);
        let top: Vec<(String, u32, u32)> = fields.clone();
        self.csp_nested_walk(&su, &params, "", 0, &top, fields, 0);
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn csp_nested_walk(
        &self,
        su: &crate::ast::types::StructUnionType,
        params: &HashMap<String, Value>,
        prefix: &str,
        base: u32,
        layout: &[(String, u32, u32)],
        out: &mut Vec<(String, u32, u32)>,
        depth: u32,
    ) {
        if depth > 8 {
            return;
        }
        for m in &su.members {
            let DataType::Struct(inner) =
                Self::resolve_type_ref(&m.data_type, &self.module.typedef_types)
            else {
                continue;
            };
            if !inner.packed {
                continue;
            }
            let (sub, _) = self.packed_agg_layout_with(&inner, params);
            for d in &m.declarators {
                let Some(&(_, lo, _)) = layout.iter().find(|f| f.0 == d.name.name) else {
                    continue;
                };
                let path = format!("{}{}.", prefix, d.name.name);
                for (n, l, w) in &sub {
                    out.push((format!("{}{}", path, n), base + lo + l, *w));
                }
                self.csp_nested_walk(&inner, params, &path, base + lo, &sub, out, depth + 1);
            }
        }
    }

    /// Boundaries a wide variable needs beyond its selects: the bits a
    /// constant shift keeps and the runs of a constant mask.
    pub(super) fn csp_wide_scan(
        &mut self,
        csp: &Csp,
        e: &Expression,
        bound: &[String],
        uses: &mut HashMap<usize, (Vec<(u32, u32)>, bool)>,
    ) {
        let ExprKind::Binary { op, left, right } = &Self::unparen(e).kind else {
            return;
        };
        let scan_env = csp_scan_env();
        let kval = |me: &mut Self, x: &Expression| -> Option<Value> {
            let mut reads_bound = false;
            Self::walk_operands(x, &mut |y| {
                if let ExprKind::Ident(h) = &y.kind
                    && h.path.len() == 1
                    && bound.iter().any(|b| *b == h.path[0].name.name)
                {
                    reads_bound = true;
                }
            });
            if reads_bound || !me.csp_free(csp, x, &scan_env) {
                return None;
            }
            let v = me.csp_const_any(x, &scan_env);
            (!v.has_xz() && !v.is_real && v.width > 0).then_some(v)
        };
        let mut konst = |me: &mut Self, x: &Expression| -> Option<i64> {
            let v = kval(me, x)?;
            if v.width > 64 {
                return None;
            }
            v.to_i64()
        };
        match op {
            BinaryOp::Add | BinaryOp::Sub => {
                // Word arithmetic needs 64-bit boundaries relative to each
                // wide operand. Preserve narrower declared fields elsewhere:
                // splitting those would detach their dist variables.
                for x in [left, right] {
                    let Some((v, lo, w, _)) = self.csp_slice_base_k(csp, x, &mut konst) else {
                        continue;
                    };
                    if csp.wide.contains(&v) {
                        uses.entry(v)
                            .or_default()
                            .0
                            .extend((64..w).step_by(64).map(|k| (lo + k, 0)));
                    }
                }
            }
            BinaryOp::ShiftRight
            | BinaryOp::ShiftLeft
            | BinaryOp::ArithShiftRight
            | BinaryOp::ArithShiftLeft => {
                let Some(k) = konst(self, right) else { return };
                let Some((v, lo, w, _)) = self.csp_slice_base_k(csp, left, &mut konst) else {
                    return;
                };
                if !csp.wide.contains(&v) || k <= 0 || k >= i64::from(w) {
                    return;
                }
                let k = k as u32;
                let r = if matches!(op, BinaryOp::ShiftRight | BinaryOp::ArithShiftRight) {
                    (lo + k, w - k)
                } else {
                    (lo, w - k)
                };
                uses.entry(v).or_default().0.push(r);
            }
            BinaryOp::BitAnd | BinaryOp::BitOr => {
                for (x, m) in [(left, right), (right, left)] {
                    let Some(mv) = kval(self, m) else { continue };
                    let Some((v, lo, w, whole)) = self.csp_slice_base_k(csp, x, &mut konst) else {
                        continue;
                    };
                    if !csp.wide.contains(&v) {
                        continue;
                    }
                    let sext = whole && csp.vars[v].signed && mv.is_signed;
                    let mw = WOp::konst(w, &mv, sext);
                    let u = uses.entry(v).or_default();
                    let mut i = 0;
                    while i < w {
                        let b = mw.bit(i);
                        let start = i;
                        while i < w && mw.bit(i) == b {
                            i += 1;
                        }
                        u.0.push((lo + start, i - start));
                    }
                }
            }
            // §11.4.5/§11.4.4: a relation between two wide ranges is
            // decided segment by segment once both are cut alike.
            BinaryOp::Eq
            | BinaryOp::Neq
            | BinaryOp::CaseEq
            | BinaryOp::CaseNeq
            | BinaryOp::Lt
            | BinaryOp::Leq
            | BinaryOp::Gt
            | BinaryOp::Geq => {
                let mut bits = |me: &mut Self, x: &Expression| -> Option<(usize, u32, u32)> {
                    let x = Self::unparen(x);
                    if let Some((v, r)) = me.csp_select_ref(csp, x, &mut konst) {
                        let (lo, w) = r?;
                        return csp.wide.contains(&v).then_some((v, lo, w));
                    }
                    let (v, lo, w, _) = me.csp_slice_base_k(csp, x, &mut konst)?;
                    csp.wide.contains(&v).then_some((v, lo, w))
                };
                if let (Some(a), Some(b)) = (bits(self, left), bits(self, right))
                    && (a.0, a.1) != (b.0, b.1)
                {
                    CSP_SCAN_LINKS
                        .with(|l| l.borrow_mut().push((a.0, a.1, b.0, b.1, a.2.min(b.2))));
                }
            }
            _ => {}
        }
    }

    /// An item the dependency scan could not fully analyse may read a wide
    /// variable: it calls a method, or names one other than through
    /// `size()` (an array's size is fixed while it is solved).
    pub(super) fn csp_may_read_wide(&self, csp: &Csp, item: &SrcItem, _env: &Env) -> bool {
        let mut read = HashSet::default();
        let mut opaque = false;
        let skip = |e: &Expression| self.csp_state_record_path(csp, e);
        match item {
            SrcItem::Expr(e) => Self::csp_wide_expr_reads_with(e, &mut read, &mut opaque, &skip),
            SrcItem::Item(it) => Self::csp_wide_item_reads_with(it, &mut read, &mut opaque, &skip),
        }
        Self::csp_wide_named(csp, &read, opaque)
    }

    /// The variables `e` reads, when it reads wide variables only through
    /// fields and selects: their segments. False when it reads a whole
    /// wide variable, or something the scan cannot follow.
    pub(super) fn csp_wide_deps(
        &mut self,
        csp: &Csp,
        e: &Expression,
        env: &Env,
        out: &mut Vec<usize>,
    ) -> bool {
        if matches!(
            e.kind,
            ExprKind::RangeSelect { .. }
                | ExprKind::Index { .. }
                | ExprKind::MemberAccess { .. }
                | ExprKind::Ident(_)
        ) && let Some((v, lo, w, whole)) = self.csp_wide_bits(csp, e, env)
        {
            if whole {
                return false;
            }
            let Some(segs) = csp.segs.get(&v) else {
                return false;
            };
            out.extend(
                segs.iter()
                    .filter(|s| s.0 < lo + w && s.0 + s.1 > lo)
                    .map(|s| s.2),
            );
            return true;
        }
        if !self.csp_wide_touch(csp, e, env) {
            return self.csp_refs(csp, e, env, out);
        }
        let all = |me: &mut Self, xs: &[&Expression], out: &mut Vec<usize>| {
            xs.iter().all(|x| me.csp_wide_deps(csp, x, env, out))
        };
        match &e.kind {
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                all(self, &[operand], out)
            }
            ExprKind::Binary { left, right, .. } => all(self, &[left, right], out),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => all(self, &[condition, then_expr, else_expr], out),
            ExprKind::Concatenation(xs) => {
                let xs: Vec<&Expression> = xs.iter().collect();
                all(self, &xs, out)
            }
            ExprKind::Inside { expr, ranges } => {
                let mut xs: Vec<&Expression> = vec![expr];
                xs.extend(ranges.iter());
                all(self, &xs, out)
            }
            ExprKind::Range(a, b) => all(self, &[a, b], out),
            // A select at a position not known yet (`f[id]`, `f[id][7:0]`)
            // reads the bits of what it selects from, and its index.
            ExprKind::Index { expr, index } => self.csp_wide_sel_deps(csp, expr, index, env, out),
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => all(self, &[expr, left, right], out),
            ExprKind::Ident(_) => match Self::csp_split_last_select(e) {
                Some((b, i)) => self.csp_wide_sel_deps(csp, &b, &i, env, out),
                None => false,
            },
            _ => false,
        }
    }

    /// `base[index]` for `csp_wide_deps`: the select's bits and index;
    /// under `CSP_DYN_WHOLE`, every segment of a whole wide `base`.
    fn csp_wide_sel_deps(
        &mut self,
        csp: &Csp,
        base: &Expression,
        index: &Expression,
        env: &Env,
        out: &mut Vec<usize>,
    ) -> bool {
        if CSP_DYN_WHOLE.with(|c| c.get())
            && let Some((v, _, _, true)) = self.csp_wide_bits(csp, base, env)
            && let Some(segs) = csp.segs.get(&v)
        {
            out.extend(segs.iter().map(|s| s.2));
            return self.csp_wide_deps(csp, index, env, out);
        }
        self.csp_wide_deps(csp, base, env, out) && self.csp_wide_deps(csp, index, env, out)
    }

    /// `e` may read a wide variable (see `csp_may_read_wide`).
    pub(super) fn csp_wide_touch(&self, csp: &Csp, e: &Expression, _env: &Env) -> bool {
        let mut read = HashSet::default();
        let mut opaque = false;
        Self::csp_wide_expr_reads_with(e, &mut read, &mut opaque, &|x| {
            self.csp_state_record_path(csp, x)
        });
        Self::csp_wide_named(csp, &read, opaque)
    }

    /// §18.5: a field of a non-rand packed-record collection is state,
    /// even if its leaf name matches a separate wide rand property.
    fn csp_state_record_path(&self, csp: &Csp, e: &Expression) -> bool {
        let mut indexed = false;
        let mut x = e;
        let root = loop {
            match &Self::unparen(x).kind {
                ExprKind::MemberAccess { expr, .. } => x = expr,
                ExprKind::Index { expr, .. } => {
                    indexed = true;
                    x = expr;
                }
                ExprKind::Ident(h) if h.root.is_none() && !h.path.is_empty() => {
                    indexed |= h.path.iter().any(|s| !s.selects.is_empty());
                    break &h.path[0].name.name;
                }
                _ => return false,
            }
        };
        if !indexed
            || csp.scope.is_some()
            || csp.scalars.contains_key(root)
            || csp.arrays.contains_key(root)
        {
            return false;
        }
        let Some(dt) = self.csp_prop_decl_type(csp.handle, root) else {
            return false;
        };
        matches!(super::elaborate::resolve_typedef_chain(&dt,&self.module.typedef_types),DataType::Struct(su) if su.packed)
    }

    fn csp_wide_named(csp: &Csp, read: &HashSet<String>, opaque: bool) -> bool {
        fn leaf(n: &str) -> &str {
            n.rsplit_once('.').map_or(n, |p| p.1)
        }
        opaque
            || csp
                .scalars
                .iter()
                .any(|(n, v)| csp.wide.contains(v) && read.contains(leaf(n)))
            || csp.arrays.iter().any(|(n, a)| {
                a.elems.first().is_some_and(|e| csp.wide.contains(&e.1)) && read.contains(n)
            })
    }

    /// The wide variable and bit range `e` names: Some((v, lsb, width,
    /// whole)) for a wide variable (`x`, `this.x`, `a[k]`), a field of one,
    /// or a constant select of either.
    fn csp_wide_bits(
        &mut self,
        csp: &Csp,
        e: &Expression,
        env: &Env,
    ) -> Option<(usize, u32, u32, bool)> {
        let mut konst = |me: &mut Self, x: &Expression| -> Option<i64> {
            if !me.csp_free(csp, x, env) {
                return None;
            }
            me.csp_const(x, env)?.to_i64()
        };
        if let Some((v, r)) = self.csp_select_ref(csp, e, &mut konst) {
            if !csp.wide.contains(&v) {
                return None;
            }
            let (lo, w) = r?;
            return Some((v, lo, w, false));
        }
        let (v, lo, w, whole) = self.csp_slice_base_k(csp, e, &mut konst)?;
        csp.wide.contains(&v).then_some((v, lo, w, whole))
    }

    /// §7.12.3: identify an unfiltered reduction at its element width.
    fn csp_wide_sum_array(&mut self, csp: &Csp, e: &Expression) -> Option<CspArr> {
        let ExprKind::Call { func, args } = &Self::unparen(e).kind else {
            return None;
        };
        let ExprKind::MemberAccess { expr, member } = &func.kind else {
            return None;
        };
        if member.name != "sum" || !args.is_empty() {
            return None;
        }
        let name = self.csp_member(csp, expr)?;
        let arr = csp.arrays.get(&name)?;
        (arr.width > 64 && !arr.signed && !csp.sizes.contains_key(&name)).then(|| arr.clone())
    }

    /// §7.12.3: sum wide elements with a carry between their shared segments.
    /// The final carry is discarded: the reduction wraps at the element width.
    fn csp_wide_sum_op(
        &mut self,
        csp: &mut Csp,
        e: &Expression,
        w: u32,
        s: bool,
        env: &Env,
    ) -> Option<WOp> {
        let Some(arr) = self.csp_wide_sum_array(csp, e) else {
            return self.csp_wide_op(csp, e, w, s, env);
        };
        if arr.width > w {
            return None;
        }
        let mut out = WOp::zero(w);
        let Some(&(_, first)) = arr.elems.first() else {
            return Some(out);
        };
        let shape = csp.segs.get(&first)?.clone();
        let mut chunks = Vec::with_capacity(shape.len());
        // Validate all inputs before allocating a defining relation.
        for &(lo, width, _) in &shape {
            let terms = arr
                .elems
                .iter()
                .map(|(_, v)| {
                    csp.segs
                        .get(v)?
                        .iter()
                        .find(|x| x.0 == lo && x.1 == width)
                        .map(|x| (x.2, 1))
                })
                .collect::<Option<Vec<_>>>()?;
            chunks.push((lo, width, terms));
        }
        let mut carry = None;
        for (lo, width, mut terms) in chunks {
            if let Some(v) = carry {
                terms.push((v, 1));
            }
            let value = Self::csp_aux_var(csp, width);
            out.vars.push((lo, width, value));
            terms.push((value, -1));
            let next = Self::csp_aux_var(csp, 64);
            csp.dom0[next] = vec![(0, arr.elems.len() as i128)];
            terms.push((next, -(1i128 << width)));
            let node = self.csp_wide_lin(csp, &Lin { t: terms, k: 0 }, &Lin::konst(0), 0, Rel::Eq);
            csp.nodes.push(node);
            carry = Some(next);
        }
        Some(out)
    }

    /// Self-determined (width, signed) of an operand of a wide relation.
    fn csp_wide_ws(&mut self, csp: &Csp, e: &Expression, env: &Env) -> Option<(u32, bool)> {
        let e = Self::unparen(e);
        if self.csp_free(csp, e, env) {
            if matches!(
                e.kind,
                ExprKind::Number(crate::ast::expr::NumberLiteral::UnbasedUnsized(_))
            ) {
                // §5.7.1: `'0`/`'1` take the width of the context.
                return Some((1, false));
            }
            let v = self.csp_const_any(e, env);
            return (!v.has_xz() && !v.is_real && v.width > 0).then_some((v.width, v.is_signed));
        }
        if let Some(arr) = self.csp_wide_sum_array(csp, e) {
            return Some((arr.width, arr.signed));
        }
        match &e.kind {
            // §11.6.1: a shift has the width and sign of its left operand.
            ExprKind::Binary {
                op:
                    BinaryOp::ShiftRight
                    | BinaryOp::ShiftLeft
                    | BinaryOp::ArithShiftRight
                    | BinaryOp::ArithShiftLeft,
                left,
                ..
            } => return self.csp_wide_ws(csp, left, env),
            // §11.6.1/§11.8.1: `+ - & |` take the larger width, signed
            // only when both operands are.
            ExprKind::Binary {
                op: BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::Add | BinaryOp::Sub,
                left,
                right,
            } => {
                let (a, b) = (
                    self.csp_wide_ws(csp, left, env)?,
                    self.csp_wide_ws(csp, right, env)?,
                );
                return Some((a.0.max(b.0), a.1 && b.1));
            }
            ExprKind::Unary {
                op: UnaryOp::Minus,
                operand,
            } => return self.csp_wide_ws(csp, operand, env),
            _ => {}
        }
        if let Some((v, _, w, whole)) = self.csp_wide_bits(csp, e, env) {
            // §11.5.1: a select (and here a struct field) is unsigned.
            return Some((w, whole && csp.vars[v].signed));
        }
        let a = self.csp_ae(csp, e, env)?;
        Some(Self::csp_ws(&a))
    }

    /// §11.4.3: wide addition/subtraction carries between 64-bit words.
    /// Narrow products use their established bounds before joining those words.
    fn csp_wide_arithmetic_op(
        &mut self,
        csp: &mut Csp,
        e: &Expression,
        w: u32,
        s: bool,
        env: &Env,
    ) -> Option<WOp> {
        // Narrow arithmetic on fields of a wide struct already has an
        // affine model. Word carries there add artificial search decisions
        // to large object graphs; reserve them for an actually wide context.
        if w <= 64 {
            return self.csp_wide_op(csp, e, w, s, env);
        }
        if let Some(o) = self.csp_wide_sum_op(csp, e, w, s, env) {
            return Some(o);
        }
        let e = Self::unparen(e);
        if let ExprKind::Binary {
            op: op @ (BinaryOp::Add | BinaryOp::Sub),
            left,
            right,
        } = &e.kind
        {
            let l = self.csp_wide_arithmetic_op(csp, left, w, s, env)?;
            let r = self.csp_wide_arithmetic_op(csp, right, w, s, env)?;
            let sub = *op == BinaryOp::Sub;
            let mut out = WOp::zero(w);
            let mut carry = None;
            let word = |o: &WOp, lo: u32, width: u32| -> Option<Lin> {
                let mut t = Vec::new();
                for &(a, b, v) in &o.vars {
                    if a >= lo && a < lo + width {
                        if a + b > lo + width {
                            return None;
                        }
                        t.push((v, 1i128 << (a - lo)));
                    } else if a < lo && a + b > lo {
                        return None;
                    }
                }
                Some(Lin {
                    t,
                    k: o.word(lo, lo + width) as i128,
                })
            };
            for lo in (0..w).step_by(64) {
                let width = (w - lo).min(64);
                let (a, b) = (word(&l, lo, width)?, word(&r, lo, width)?);
                let mut acc = Lin {
                    t: a.t,
                    k: a.k + if sub { -b.k } else { b.k },
                };
                acc.t
                    .extend(b.t.into_iter().map(|(v, c)| (v, if sub { -c } else { c })));
                if let Some(v) = carry {
                    acc.t.push((v, 1));
                }
                let value = Self::csp_aux_var(csp, width);
                out.vars.push((lo, width, value));
                let next = csp.vars.len();
                csp.vars.push(CspVar {
                    key: VarKey::Aux,
                    width: 64,
                    signed: true,
                });
                csp.dom0.push(vec![(-1, 1)]);
                acc.t.push((value, -1));
                acc.t.push((next, -(1i128 << width)));
                let n = self.csp_wide_lin(csp, &acc, &Lin::konst(0), 0, Rel::Eq);
                csp.nodes.push(n);
                carry = Some(next);
            }
            return Some(out);
        }
        if s || w <= 64 {
            return None;
        }
        let a = self.csp_ae(csp, e, env)?;
        let mut cx = LinCx::new(csp);
        let mut fits = Vec::new();
        let lin = Self::csp_lin(&a, w, s, &mut fits, &mut cx)?;
        let (lo, hi) = cx.constrained_range(&lin)?;
        if lo < 0
            || hi > u64::MAX as i128
            || !fits.iter().all(|f| {
                cx.constrained_range(&f.lin)
                    .is_some_and(|(a, b)| a >= f.lo && b <= f.hi)
            })
        {
            return None;
        }
        let pending = cx.take();
        self.csp_add_carries(csp, pending);
        let v = if let ([(v, 1)], 0) = (lin.t.as_slice(), lin.k) {
            *v
        } else {
            let v = Self::csp_aux_var(csp, 64);
            csp.dom0[v] = vec![(lo, hi)];
            let n = self.csp_wide_lin(csp, &lin, &Lin::var(v), 0, Rel::Eq);
            csp.nodes.push(n);
            v
        };
        let mut out = WOp::zero(w);
        out.vars.push((0, 64, v));
        Some(out)
    }

    /// `e` as a `w`-bit operand in a context of signedness `s` (§11.8.2).
    fn csp_wide_op(
        &mut self,
        csp: &Csp,
        e: &Expression,
        w: u32,
        s: bool,
        env: &Env,
    ) -> Option<WOp> {
        let e = Self::unparen(e);
        if self.csp_free(csp, e, env) {
            if let ExprKind::Number(crate::ast::expr::NumberLiteral::UnbasedUnsized(c)) = &e.kind {
                let mut o = WOp::zero(w);
                match c {
                    '0' => {}
                    '1' => (0..w).for_each(|i| o.set(i, true)),
                    _ => return None,
                }
                return Some(o);
            }
            let real = Self::csp_obj_real(e);
            let expr = real.as_ref().unwrap_or(e);
            let frame = self.csp_frame(&env.binds);
            self.push_local_frame(frame);
            let v = self.eval_expr_typed(expr, w, !s);
            self.pop_local_frame();
            if v.has_xz() || v.is_real || v.width == 0 || v.width > w {
                return None;
            }
            return Some(WOp::konst(w, &v, s && v.is_signed));
        }
        match &e.kind {
            ExprKind::Binary {
                op:
                    op @ (BinaryOp::ShiftRight
                    | BinaryOp::ShiftLeft
                    | BinaryOp::ArithShiftRight
                    | BinaryOp::ArithShiftLeft),
                left,
                right,
            } => {
                // §11.4.10: the shift amount is self-determined; `>>>` of a
                // signed operand fills with its sign.
                if !self.csp_free(csp, right, env) {
                    return None;
                }
                let k = self.csp_const(right, env)?.to_u64()?;
                if *op == BinaryOp::ArithShiftRight && s {
                    return None;
                }
                let x = self.csp_wide_op(csp, left, w, s, env)?;
                return if matches!(op, BinaryOp::ShiftRight | BinaryOp::ArithShiftRight) {
                    x.shr(k)
                } else {
                    x.shl(k)
                };
            }
            ExprKind::Binary {
                op: op @ (BinaryOp::BitAnd | BinaryOp::BitOr),
                left,
                right,
            } => {
                let (x, m) = if self.csp_free(csp, right, env) {
                    (left, right)
                } else if self.csp_free(csp, left, env) {
                    (right, left)
                } else {
                    return None;
                };
                let m = self.csp_wide_op(csp, m, w, s, env)?;
                let x = self.csp_wide_op(csp, x, w, s, env)?;
                return x.mask(&m, *op == BinaryOp::BitOr);
            }
            _ => {}
        }
        if let Some((v, lo, bw, whole)) = self.csp_wide_bits(csp, e, env) {
            if bw > w || (whole && csp.vars[v].signed && s && bw < w) {
                // A sign-extended wide operand is not modelled.
                return None;
            }
            let mut o = WOp::zero(w);
            for &(slo, sw, x) in csp.segs.get(&v)? {
                if slo >= lo && slo + sw <= lo + bw {
                    o.vars.push((slo - lo, sw, x));
                } else if slo < lo + bw && slo + sw > lo {
                    return None;
                }
            }
            return Some(o);
        }
        // A narrow unsigned variable (or a segment of one), zero-extended.
        match self.csp_ae(csp, e, env)? {
            Ae::Var(x, vw, false) if vw <= 64 && vw <= w => {
                let mut o = WOp::zero(w);
                o.vars.push((0, vw, x));
                Some(o)
            }
            _ => None,
        }
    }

    /// The chunks of a relation between `l` and `r`, most significant
    /// first: (left, right, is the sign bit), each side `k + Σ c·x`. A
    /// chunk holds whole variables and at most 64 bits; None when the two
    /// sides' variables overlap so that no such chunks exist.
    #[allow(clippy::type_complexity)]
    fn csp_wide_chunks(l: &WOp, r: &WOp, s: bool) -> Option<Vec<(Lin, Lin, bool)>> {
        let w = l.w;
        let mut b: Vec<u32> = (0..w).step_by(64).collect();
        b.push(w);
        let all = || l.vars.iter().chain(r.vars.iter());
        for &(lo, vw, _) in all() {
            b.push(lo);
            b.push(lo + vw);
        }
        if s {
            b.push(w - 1);
        }
        b.sort_unstable();
        b.dedup();
        b.retain(|&x| !all().any(|&(lo, vw, _)| lo < x && x < lo + vw));
        if s && !b.contains(&(w - 1)) {
            return None;
        }
        if b.len() > 513 {
            return None;
        }
        let side = |o: &WOp, a: u32, c: u32| -> Lin {
            Lin {
                t: o.vars
                    .iter()
                    .filter(|p| p.0 >= a && p.0 < c)
                    .map(|p| (p.2, 1i128 << (p.0 - a)))
                    .collect(),
                k: o.word(a, c) as i128,
            }
        };
        let mut out = Vec::with_capacity(b.len());
        for win in b.windows(2).rev() {
            let (a, c) = (win[0], win[1]);
            if c - a > 64 {
                return None;
            }
            out.push((side(l, a, c), side(r, a, c), s && a == w - 1));
        }
        Some(out)
    }

    /// `a - b + k REL 0` as a node, decided at once when the declared
    /// domains already decide it.
    fn csp_wide_lin(&mut self, csp: &mut Csp, a: &Lin, b: &Lin, k: i128, rel: Rel) -> Node {
        let mut t: Vec<(usize, i128)> =
            a.t.iter()
                .copied()
                .chain(b.t.iter().map(|&(v, c)| (v, -c)))
                .collect();
        t.sort_unstable_by_key(|x| x.0);
        let mut m: Vec<(usize, i128)> = Vec::with_capacity(t.len());
        for (v, c) in t {
            match m.last_mut() {
                Some(l) if l.0 == v => l.1 += c,
                _ => m.push((v, c)),
            }
        }
        m.retain(|x| x.1 != 0);
        let lin = Lin {
            t: m,
            k: a.k - b.k + k,
        };
        let (mut lo, mut hi) = (lin.k, lin.k);
        for &(v, c) in &lin.t {
            let d = &csp.dom0[v];
            let (x, y) = (d.first().map_or(0, |p| p.0), d.last().map_or(0, |p| p.1));
            if c > 0 {
                lo += c * x;
                hi += c * y;
            } else {
                lo += c * y;
                hi += c * x;
            }
        }
        let decided = match rel {
            Rel::Eq if lo == 0 && hi == 0 => Some(true),
            Rel::Eq if lo > 0 || hi < 0 => Some(false),
            Rel::Ne if lo == 0 && hi == 0 => Some(false),
            Rel::Ne if lo > 0 || hi < 0 => Some(true),
            Rel::Le if hi <= 0 => Some(true),
            Rel::Le if lo > 0 => Some(false),
            _ => None,
        };
        match decided {
            Some(true) => Node::True,
            Some(false) => Node::False,
            None => {
                let deps: Vec<usize> = lin.t.iter().map(|x| x.0).collect();
                let one = Self::literal_of(&Value::from_u64(1, 1), crate::ast::Span::dummy());
                let src = self.csp_src(csp, SrcItem::Expr(one), &Env::default(), deps);
                Node::Lin {
                    lin,
                    rel,
                    fits: Vec::new(),
                    src,
                    sneg: false,
                    wrap: None,
                }
            }
        }
    }

    pub(super) fn csp_wide_and(v: Vec<Node>) -> Node {
        if v.iter().any(|n| matches!(n, Node::False)) {
            return Node::False;
        }
        Self::csp_and(v.into_iter().filter(|n| !matches!(n, Node::True)).collect())
    }

    pub(super) fn csp_wide_or(v: Vec<Node>) -> Node {
        if v.iter().any(|n| matches!(n, Node::True)) {
            return Node::True;
        }
        let mut v: Vec<Node> = v
            .into_iter()
            .filter(|n| !matches!(n, Node::False))
            .collect();
        match v.len() {
            0 => Node::False,
            1 => v.pop().unwrap(),
            _ => Node::Or(v),
        }
    }

    /// `left op right` with a wide operand (see the impl docs). None when
    /// the shape is not modelled.
    pub(super) fn csp_wide_rel(
        &mut self,
        csp: &mut Csp,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
        env: &Env,
        neg: bool,
    ) -> Option<Node> {
        let op = if neg {
            match op {
                BinaryOp::Eq | BinaryOp::CaseEq => BinaryOp::Neq,
                BinaryOp::Neq | BinaryOp::CaseNeq => BinaryOp::Eq,
                BinaryOp::Lt => BinaryOp::Geq,
                BinaryOp::Leq => BinaryOp::Gt,
                BinaryOp::Gt => BinaryOp::Leq,
                BinaryOp::Geq => BinaryOp::Lt,
                _ => return None,
            }
        } else {
            op
        };
        // §11.6.1/§11.8.1: both sides take the larger width, and are signed
        // only when both are.
        let (wl, sl) = self.csp_wide_ws(csp, left, env)?;
        let (wr, sr) = self.csp_wide_ws(csp, right, env)?;
        let (w, s) = (wl.max(wr), sl && sr);
        // An operand computed by `+`/`-` from a variable (a constant one,
        // `-80'sd10`, is just a constant).
        let arith = |me: &mut Self, csp: &Csp, e: &Expression| {
            Self::csp_wide_arith(e) && !me.csp_free(csp, e, env)
        };
        if (arith(self, csp, left) || arith(self, csp, right))
            && let Some(n) = self.csp_wide_affine_rel(csp, op, left, right, w, s, env)
        {
            return Some(n);
        }
        let l = self.csp_wide_arithmetic_op(csp, left, w, s, env)?;
        let r = self.csp_wide_arithmetic_op(csp, right, w, s, env)?;
        let chunks = Self::csp_wide_chunks(&l, &r, s)?;
        Some(match op {
            BinaryOp::Eq | BinaryOp::CaseEq => {
                let v = chunks
                    .iter()
                    .map(|(a, b, _)| self.csp_wide_lin(csp, a, b, 0, Rel::Eq))
                    .collect();
                Self::csp_wide_and(v)
            }
            BinaryOp::Neq | BinaryOp::CaseNeq => {
                let v = chunks
                    .iter()
                    .map(|(a, b, _)| self.csp_wide_lin(csp, a, b, 0, Rel::Ne))
                    .collect();
                Self::csp_wide_or(v)
            }
            BinaryOp::Lt => self.csp_wide_lex(csp, &chunks, false, true),
            BinaryOp::Leq => self.csp_wide_lex(csp, &chunks, false, false),
            BinaryOp::Gt => self.csp_wide_lex(csp, &chunks, true, true),
            BinaryOp::Geq => self.csp_wide_lex(csp, &chunks, true, false),
            _ => return None,
        })
    }

    /// Is `e` a sum, difference or negation (the shapes `csp_wide_affine`
    /// reads)?
    fn csp_wide_arith(e: &Expression) -> bool {
        matches!(
            Self::unparen(e).kind,
            ExprKind::Binary {
                op: BinaryOp::Add | BinaryOp::Sub,
                ..
            } | ExprKind::Unary {
                op: UnaryOp::Minus,
                ..
            }
        )
    }

    /// §11.4.3/§11.6.1: `e` in the context `(w, s)` as `±x + k` modulo
    /// 2^w, `x` being at most one operand that `csp_wide_op` reads as plain
    /// variable bits (a wide variable, a select of one, a narrow variable)
    /// and `k` the sum of the constant operands, extended to `w` bits. The
    /// operand comes back with whether it is negated; None for a constant.
    #[allow(clippy::type_complexity)]
    fn csp_wide_affine<'e>(
        &mut self,
        csp: &Csp,
        e: &'e Expression,
        w: u32,
        s: bool,
        env: &Env,
    ) -> Option<(Option<(&'e Expression, bool)>, Vec<u64>)> {
        let e = Self::unparen(e);
        if self.csp_free(csp, e, env) {
            let o = self.csp_wide_op(csp, e, w, s, env)?;
            return o.vars.is_empty().then_some((None, o.bits));
        }
        match &e.kind {
            ExprKind::Binary {
                op: op @ (BinaryOp::Add | BinaryOp::Sub),
                left,
                right,
            } => {
                let (xa, ka) = self.csp_wide_affine(csp, left, w, s, env)?;
                let (xb, kb) = self.csp_wide_affine(csp, right, w, s, env)?;
                let sub = *op == BinaryOp::Sub;
                let x = match (xa, xb.map(|(x, n)| (x, n ^ sub))) {
                    (Some(_), Some(_)) => return None,
                    (a, b) => a.or(b),
                };
                let k = if sub {
                    ww_sub(&ka, &kb, w)
                } else {
                    ww_add(&ka, &kb, w)
                };
                Some((x, k))
            }
            ExprKind::Unary {
                op: UnaryOp::Minus,
                operand,
            } => {
                let (x, k) = self.csp_wide_affine(csp, operand, w, s, env)?;
                Some((x.map(|(x, n)| (x, !n)), ww_neg(&k, w)))
            }
            _ => {
                let o = self.csp_wide_op(csp, e, w, s, env)?;
                if o.vars.is_empty() || o.bits.iter().any(|&b| b != 0) {
                    return None;
                }
                Some((Some((e, false)), vec![0; o.bits.len()]))
            }
        }
    }

    /// `left op right` where one side is `±x + k` (modulo 2^w, see
    /// `csp_wide_affine`) and the other a constant `c`: the values of
    /// `±x + k` that satisfy `op c` are one interval (two for `!=`), and
    /// moving `k` and the sign across turns each into an interval of `x` —
    /// two when it wraps past 2^w. A signed context compares `y ^ msb`
    /// unsigned (§11.4.4). So `w + 80'd5 == 80'd2` is `w == 2^80 - 3`.
    #[allow(clippy::too_many_arguments)]
    fn csp_wide_affine_rel(
        &mut self,
        csp: &mut Csp,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
        w: u32,
        s: bool,
        env: &Env,
    ) -> Option<Node> {
        let (xl, kl) = self.csp_wide_affine(csp, left, w, s, env)?;
        let (xr, kr) = self.csp_wide_affine(csp, right, w, s, env)?;
        let (x, neg, k, c, op) = match (xl, xr) {
            (Some((x, n)), None) => (x, n, kl, kr, op),
            (None, Some((x, n))) => {
                let op = match op {
                    BinaryOp::Lt => BinaryOp::Gt,
                    BinaryOp::Leq => BinaryOp::Geq,
                    BinaryOp::Gt => BinaryOp::Lt,
                    BinaryOp::Geq => BinaryOp::Leq,
                    o => o,
                };
                (x, n, kr, kl, op)
            }
            _ => return None,
        };
        // A signed operand narrower than the signed context is sign-extended
        // there; its bits are not one interval of the extended value.
        let (xw, xs) = self.csp_wide_ws(csp, x, env)?;
        if s && xs && xw < w {
            return None;
        }
        let nw = w.div_ceil(64) as usize;
        let zero = vec![0u64; nw];
        let mut max = vec![u64::MAX; nw];
        ww_mask(&mut max, w);
        let mut bias = zero.clone();
        if s {
            bias[((w - 1) / 64) as usize] = 1u64 << ((w - 1) % 64);
        }
        let one = {
            let mut o = zero.clone();
            o[0] = 1;
            o
        };
        // The biased value `y' = y + bias` of `y = ±x + k` lies in `ys`.
        let cb = ww_add(&c, &bias, w);
        let below = (cb != zero).then(|| ww_sub(&cb, &one, w));
        let above = (cb != max).then(|| ww_add(&cb, &one, w));
        let ys: Vec<(Vec<u64>, Vec<u64>)> = match op {
            BinaryOp::Eq | BinaryOp::CaseEq => vec![(cb.clone(), cb.clone())],
            BinaryOp::Neq | BinaryOp::CaseNeq => below
                .map(|b| (zero.clone(), b))
                .into_iter()
                .chain(above.map(|a| (a, max.clone())))
                .collect(),
            BinaryOp::Lt => below.map(|b| (zero.clone(), b)).into_iter().collect(),
            BinaryOp::Leq => vec![(zero.clone(), cb.clone())],
            BinaryOp::Gt => above.map(|a| (a, max.clone())).into_iter().collect(),
            BinaryOp::Geq => vec![(cb.clone(), max.clone())],
            _ => return None,
        };
        let kb = ww_add(&k, &bias, w);
        let mut alts = Vec::new();
        let span = left.span;
        let lit = |v: &[u64]| Self::literal_of(&Value::from_words(v, w), span);
        let mut ivs = Vec::new();
        for (a, b) in ys {
            // `y' = x + kb` gives `x = y' - kb`; `y' = kb - x` gives
            // `x = kb - y'` (decreasing, so the ends swap).
            let (p, q) = if neg {
                (ww_sub(&kb, &b, w), ww_sub(&kb, &a, w))
            } else {
                (ww_sub(&a, &kb, w), ww_sub(&b, &kb, w))
            };
            if ww_cmp(&p, &q) == std::cmp::Ordering::Greater {
                ivs.push((p, max.clone()));
                ivs.push((zero.clone(), q));
            } else {
                ivs.push((p, q));
            }
        }
        // Several intervals: their disjunction acts only once all but one
        // are excluded, so a search drawing the variable's 64-bit segments
        // at random would almost never meet one. Its most significant
        // segment (bits `lo..w` of `x`) lies in the union of the intervals'
        // `>> lo` images, which propagates at once.
        let mut proj = None;
        if ivs.len() > 1
            && let Some(o) = self.csp_wide_op(csp, x, w, false, env)
            && let Some(&(lo, vw, v)) = o.vars.iter().max_by_key(|t| t.0)
            && lo + vw == w
            && lo < 128
        {
            let top = |x: &[u64]| -> i128 {
                let mut r: u128 = 0;
                for i in lo..w {
                    if (x[(i / 64) as usize] >> (i % 64)) & 1 == 1 {
                        r |= 1u128 << (i - lo);
                    }
                }
                r as i128
            };
            let set = dom_norm(ivs.iter().map(|(p, q)| (top(p), top(q))).collect());
            let one = Self::literal_of(&Value::from_u64(1, 1), crate::ast::Span::dummy());
            let src = self.csp_src(csp, SrcItem::Expr(one), &Env::default(), vec![v]);
            proj = Some(Node::In {
                lin: Lin::var(v),
                set,
                neg: false,
                fits: Vec::new(),
                src,
                wrap: None,
            });
        }
        {
            for (p, q) in ivs {
                let n = if p == q {
                    self.csp_wide_rel(csp, BinaryOp::Eq, x, &lit(&p), env, false)?
                } else {
                    let mut both = Vec::new();
                    if p != zero {
                        both.push(self.csp_wide_rel(
                            csp,
                            BinaryOp::Geq,
                            x,
                            &lit(&p),
                            env,
                            false,
                        )?);
                    }
                    if q != max {
                        both.push(self.csp_wide_rel(
                            csp,
                            BinaryOp::Leq,
                            x,
                            &lit(&q),
                            env,
                            false,
                        )?);
                    }
                    Self::csp_wide_and(both)
                };
                alts.push(n);
            }
        }
        let set = Self::csp_wide_or(alts);
        Some(match proj {
            Some(p) => Self::csp_wide_and(vec![set, p]),
            None => set,
        })
    }

    /// `l < r` (`strict`) or `l <= r` over chunks, most significant first;
    /// with `swap`, `r < l` / `r <= l`. Built from the least significant
    /// chunk up: `le_i && (lt_i || (eq_i && rest))`. The implied `le_i`
    /// propagates bounds while the disjunction is still open.
    fn csp_wide_lex(
        &mut self,
        csp: &mut Csp,
        chunks: &[(Lin, Lin, bool)],
        swap: bool,
        strict: bool,
    ) -> Node {
        let mut rest = if strict { Node::False } else { Node::True };
        for (a, b, sign) in chunks.iter().rev() {
            let (a, b) = if swap { (b, a) } else { (a, b) };
            // A set sign bit is the smaller value (§11.4.4).
            let lt = if *sign {
                self.csp_wide_lin(csp, b, a, 1, Rel::Le)
            } else {
                self.csp_wide_lin(csp, a, b, 1, Rel::Le)
            };
            let le = if *sign {
                self.csp_wide_lin(csp, b, a, 0, Rel::Le)
            } else {
                self.csp_wide_lin(csp, a, b, 0, Rel::Le)
            };
            let eq = self.csp_wide_lin(csp, a, b, 0, Rel::Eq);
            let or = Self::csp_wide_or(vec![lt, Self::csp_wide_and(vec![eq, rest])]);
            rest = Self::csp_wide_and(vec![le, or]);
        }
        rest
    }

    /// `x == other` with `x` a whole wide variable and `other` an expression
    /// over other variables the segment model cannot express (`a ^ b`): once
    /// every variable `other` reads is fixed it is evaluated, and each
    /// segment of `x` is fixed to its bits of the value (one `Fun` per
    /// segment), instead of the segments being guessed.
    pub(super) fn csp_wide_fun(
        &mut self,
        csp: &mut Csp,
        whole: &Expression,
        x: &Expression,
        other: &Expression,
        env: &Env,
    ) -> Option<Node> {
        let (v, lo, bw, is_whole) = self.csp_wide_bits(csp, x, env)?;
        if !is_whole || lo != 0 || bw != csp.vars[v].width {
            return None;
        }
        let segs = csp.segs.get(&v)?.clone();
        let mut deps = Vec::new();
        if !self.csp_wide_fun_deps(csp, other, env, &mut deps) {
            return None;
        }
        if deps.is_empty() || segs.iter().any(|s| deps.contains(&s.2) || s.2 == v) {
            return None;
        }
        deps.sort_unstable();
        deps.dedup();
        let span = whole.span;
        let mut all = deps.clone();
        all.extend(segs.iter().map(|s| s.2));
        all.sort_unstable();
        all.dedup();
        let whole_src = self.csp_src(csp, SrcItem::Expr(whole.clone()), env, all);
        let mut nodes = Vec::with_capacity(segs.len());
        for &(slo, sw, sv) in &segs {
            let mask = if sw >= 64 { u64::MAX } else { (1u64 << sw) - 1 };
            let shifted = Expression::new(
                ExprKind::Binary {
                    op: BinaryOp::ShiftRight,
                    left: Box::new(Expression::new(
                        ExprKind::Paren(Box::new(other.clone())),
                        span,
                    )),
                    right: Box::new(Self::literal_of(&Value::from_u64(slo as u64, 32), span)),
                },
                span,
            );
            let seg_e = Expression::new(
                ExprKind::Binary {
                    op: BinaryOp::BitAnd,
                    left: Box::new(shifted),
                    right: Box::new(Self::literal_of(&Value::from_u64(mask, sw.max(1)), span)),
                },
                span,
            );
            let other_src = self.csp_src(csp, SrcItem::Expr(seg_e), env, deps.clone());
            nodes.push(Node::Fun {
                var: sv,
                op: BinaryOp::Eq,
                other: other_src,
                whole: whole_src,
                neg: false,
            });
        }
        Some(Self::csp_and(nodes))
    }

    /// The variables `e` reads for `csp_wide_fun`: the segments under a
    /// wide variable, a select or field of one, else what `csp_refs` finds.
    fn csp_wide_fun_deps(
        &mut self,
        csp: &Csp,
        e: &Expression,
        env: &Env,
        out: &mut Vec<usize>,
    ) -> bool {
        if matches!(
            e.kind,
            ExprKind::RangeSelect { .. }
                | ExprKind::Index { .. }
                | ExprKind::MemberAccess { .. }
                | ExprKind::Ident(_)
        ) && let Some((v, lo, w, _)) = self.csp_wide_bits(csp, e, env)
        {
            let Some(segs) = csp.segs.get(&v) else {
                return false;
            };
            out.extend(
                segs.iter()
                    .filter(|s| s.0 < lo + w && s.0 + s.1 > lo)
                    .map(|s| s.2),
            );
            return true;
        }
        match &e.kind {
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                self.csp_wide_fun_deps(csp, operand, env, out)
            }
            ExprKind::Binary { left, right, .. } => {
                self.csp_wide_fun_deps(csp, left, env, out)
                    && self.csp_wide_fun_deps(csp, right, env, out)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.csp_wide_fun_deps(csp, condition, env, out)
                    && self.csp_wide_fun_deps(csp, then_expr, env, out)
                    && self.csp_wide_fun_deps(csp, else_expr, env, out)
            }
            ExprKind::Concatenation(xs) => {
                xs.iter().all(|x| self.csp_wide_fun_deps(csp, x, env, out))
            }
            _ if self.csp_free(csp, e, env) => true,
            _ if !self.csp_wide_touch(csp, e, env) => self.csp_refs(csp, e, env, out),
            _ => false,
        }
    }

    /// §11.4.13: `expr inside {…}` with a wide operand, as the disjunction
    /// of `expr == v` and `lo <= expr <= hi`.
    pub(super) fn csp_wide_inside(
        &mut self,
        csp: &mut Csp,
        expr: &Expression,
        ranges: &[ConstraintRange],
        env: &Env,
        neg: bool,
    ) -> Option<Node> {
        let mut alts = Vec::with_capacity(ranges.len());
        for r in ranges {
            match r {
                ConstraintRange::Value(v) => {
                    if self
                        .csp_member(csp, v)
                        .is_some_and(|n| csp.arrays.contains_key(&n))
                    {
                        return None;
                    }
                    alts.push(self.csp_wide_rel(csp, BinaryOp::Eq, expr, v, env, neg)?);
                }
                ConstraintRange::Range { lo, hi } => {
                    if matches!(lo.kind, ExprKind::Dollar) || matches!(hi.kind, ExprKind::Dollar) {
                        return None;
                    }
                    let a = self.csp_wide_rel(csp, BinaryOp::Geq, expr, lo, env, neg)?;
                    let b = self.csp_wide_rel(csp, BinaryOp::Leq, expr, hi, env, neg)?;
                    alts.push(if neg {
                        Self::csp_wide_or(vec![a, b])
                    } else {
                        Self::csp_wide_and(vec![a, b])
                    });
                }
            }
        }
        let set = if neg {
            Self::csp_wide_and(alts)
        } else {
            Self::csp_wide_or(alts)
        };
        if neg || !matches!(set, Node::Or(_)) {
            return Some(set);
        }
        // `x inside {K1, K2, …}` over constants: each segment of `x` takes
        // the bits of one of the values there. That domain restriction is
        // implied by the disjunction, which alone only acts once all but
        // one alternative are excluded, so a search drawing 64-bit segments
        // at random would almost never meet a value.
        let mut proj = vec![set];
        if let Some(seg) = self.csp_wide_value_projection(csp, expr, ranges, env) {
            proj.extend(seg);
        }
        Some(Self::csp_wide_and(proj))
    }

    /// The per-segment domains `expr inside {K1, K2, …}` implies when every
    /// item is a constant value: segment `x[a+:n]` in `{Ki[a+:n]}`.
    fn csp_wide_value_projection(
        &mut self,
        csp: &mut Csp,
        expr: &Expression,
        ranges: &[ConstraintRange],
        env: &Env,
    ) -> Option<Vec<Node>> {
        let (w, s) = self.csp_wide_ws(csp, expr, env)?;
        let mut consts = Vec::with_capacity(ranges.len());
        for r in ranges {
            let ConstraintRange::Value(v) = r else {
                return None;
            };
            let (vw, vs) = self.csp_wide_ws(csp, v, env)?;
            let (cw, cs) = (w.max(vw), s && vs);
            if cw != w {
                return None;
            }
            if !self.csp_free(csp, v, env) {
                return None;
            }
            consts.push(self.csp_wide_op(csp, v, cw, cs, env)?);
        }
        let x = self.csp_wide_op(csp, expr, w, s, env)?;
        let mut out = Vec::new();
        for &(lo, vw, v) in &x.vars {
            let set = dom_norm(
                consts
                    .iter()
                    .map(|c| {
                        let k = c.word(lo, lo + vw) as i128;
                        (k, k)
                    })
                    .collect(),
            );
            let one = Self::literal_of(&Value::from_u64(1, 1), crate::ast::Span::dummy());
            let src = self.csp_src(csp, SrcItem::Expr(one), &Env::default(), vec![v]);
            out.push(Node::In {
                lin: Lin::var(v),
                set,
                neg: false,
                fits: Vec::new(),
                src,
                wrap: None,
            });
        }
        Some(out)
    }

    /// §18.4: the wide variables left out of the solve take random values
    /// of their full width.
    pub(super) fn csp_draw_free(&mut self, csp: &Csp) {
        for (key, w) in &csp.wide_free {
            let mut acc = Value::zero(*w);
            let mut bit = 0u32;
            while bit < *w {
                let chunk: u64 = self.cur_rng().r#gen();
                let n = (*w - bit).min(64);
                for k in 0..n {
                    if (chunk >> k) & 1 == 1 {
                        acc.set_bit((bit + k) as usize, LogicBit::One);
                    }
                }
                bit += n;
            }
            match key {
                VarKey::Prop(n) => {
                    if let Some(Some(inst)) = self.heap.get_mut(csp.handle) {
                        if let Some(old) = inst.properties.get(n) {
                            acc.is_signed = old.is_signed;
                        }
                        inst.properties.insert(n.clone(), acc);
                    }
                }
                VarKey::Elem(k) => self.write_coll_elem(k, acc),
                VarKey::Sub(..) | VarKey::Aux => {}
            }
        }
    }
}
