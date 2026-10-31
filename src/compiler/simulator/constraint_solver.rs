//! Simulator constraint solver.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// §18.14.1: `obj.randomize()` draws from the OBJECT's random stream —
    /// mark it the active stream owner for the whole solve (constraints, dist
    /// picks, randc, pre/post_randomize) so every draw routes through
    /// `cur_rng()`. Nested randomize calls stack, so an inner object still
    /// uses its own stream.
    pub(super) fn exec_randomize(&mut self, handle: usize) -> Value {
        self.begin_randomize_budget();
        self.obj_rng_stack.push(handle);
        let r = self.exec_randomize_inner(handle, &[], None);
        self.obj_rng_stack.pop();
        self.end_randomize_budget();
        r
    }

    /// §18.4/§18.5.9: randomize the object behind a rand handle member. A
    /// `randomize(a, b)` member list names the outer object's members only;
    /// the inner object draws all of its own.
    pub(super) fn randomize_nested(&mut self, sub: usize) {
        self.randomize_nested_with(sub, &[]);
    }

    /// The same, with the enclosing solve's items that constrain only this
    /// sub-object (see `pushdown_items`) joining its own set.
    pub(super) fn randomize_nested_with(&mut self, sub: usize, pushed: &[ConstraintItem]) {
        let subset = self.randomize_subset.take();
        self.randomize_depth += 1;
        self.obj_rng_stack.push(sub);
        self.exec_randomize_inner(sub, pushed, None);
        self.obj_rng_stack.pop();
        self.randomize_depth -= 1;
        self.randomize_subset = subset;
    }

    /// Every `cond -> consequent` item of the class constraints, with
    /// `Block`/`Soft` nesting flattened (top level and inside blocks).
    pub(super) fn class_implications<'a>(
        constraints: &'a [ClassConstraint],
    ) -> Vec<(&'a Expression, ImpCons<'a>)> {
        fn walk<'a>(item: &'a ConstraintItem, out: &mut Vec<(&'a Expression, ImpCons<'a>)>) {
            match item {
                ConstraintItem::Implication {
                    condition,
                    constraint,
                    ..
                } => {
                    out.push((condition, ImpCons::Item(constraint.as_ref())));
                }
                // `a -> b` inside a constraint block parses as ONE expression
                // (LogImplies), never as ConstraintItem::Implication.
                ConstraintItem::Expr(e) => {
                    if let ExprKind::Binary {
                        op: BinaryOp::LogImplies,
                        left,
                        right,
                    } = &Simulator::unparen(e).kind
                    {
                        out.push((left.as_ref(), ImpCons::Expr(right.as_ref())));
                    }
                }
                ConstraintItem::Block(items) => items.iter().for_each(|i| walk(i, out)),
                ConstraintItem::Soft(inner) => walk(inner, out),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for con in constraints {
            for it in &con.items {
                walk(it, &mut out);
            }
        }
        out
    }

    /// Rand properties named by an implication's consequent, each with the
    /// value ranges the consequent allows for it (`None` = no countable
    /// restriction). Handles `p inside {..}`, `!(p inside {..})`, `p == c`,
    /// `p != c` and the relational forms; anything else contributes no
    /// restriction. Ranges are within the property's unsigned domain.
    pub(super) fn consequent_domains_of(
        &mut self,
        cons: &ImpCons<'_>,
        rand_props: &[(String, u32)],
        out: &mut Vec<(String, Option<Vec<(i128, i128)>>)>,
    ) {
        match cons {
            ImpCons::Item(item) => self.consequent_domains(item, rand_props, out),
            ImpCons::Expr(e) => self.consequent_domains_expr(e, rand_props, out),
        }
    }

    pub(super) fn unparen(e: &Expression) -> &Expression {
        let mut cur = e;
        while let ExprKind::Paren(inner) = &cur.kind {
            cur = inner.as_ref();
        }
        cur
    }

    pub(super) fn consequent_domains_expr(
        &mut self,
        e: &Expression,
        rand_props: &[(String, u32)],
        out: &mut Vec<(String, Option<Vec<(i128, i128)>>)>,
    ) {
        let prop_of = |e: &Expression| -> Option<(String, u32)> {
            if let ExprKind::Ident(h) = &Self::unparen(e).kind {
                let n = &h.path.last()?.name.name;
                return rand_props.iter().find(|(p, _)| p == n).cloned();
            }
            None
        };
        let full = |w: u32| -> i128 {
            if w >= 127 {
                i128::MAX
            } else {
                (1i128 << w) - 1
            }
        };
        let inside_ranges = |me: &mut Self, ranges: &[Expression]| -> Vec<(i128, i128)> {
            let cr: Vec<ConstraintRange> = ranges
                .iter()
                .map(|r| match &r.kind {
                    ExprKind::Range(lo, hi) => ConstraintRange::Range {
                        lo: (**lo).clone(),
                        hi: (**hi).clone(),
                    },
                    _ => ConstraintRange::Value(r.clone()),
                })
                .collect();
            me.cons_ranges_i128(&cr)
        };
        let e = Self::unparen(e);
        // a consequent that is itself a block of `a && b` restrictions
        if let ExprKind::Binary {
            op: BinaryOp::LogAnd,
            left,
            right,
        } = &e.kind
        {
            self.consequent_domains_expr(left, rand_props, out);
            self.consequent_domains_expr(right, rand_props, out);
            return;
        }
        match &e.kind {
            ExprKind::Inside { expr, ranges } => {
                if let Some((n, _)) = prop_of(expr) {
                    let rs = inside_ranges(self, ranges);
                    out.push((n, Some(rs)));
                }
            }
            ExprKind::Unary {
                op: UnaryOp::LogNot,
                operand,
            } => {
                if let ExprKind::Inside { expr, ranges } = &Self::unparen(operand).kind {
                    if let Some((n, w)) = prop_of(expr) {
                        let mut rs = inside_ranges(self, ranges);
                        rs.sort();
                        // complement within [0, 2^w)
                        let mut comp: Vec<(i128, i128)> = Vec::new();
                        let mut cur: i128 = 0;
                        let top = full(w);
                        for (lo, hi) in rs {
                            let lo = lo.max(0);
                            if lo > cur {
                                comp.push((cur, lo - 1));
                            }
                            cur = cur.max(hi + 1);
                        }
                        if cur <= top {
                            comp.push((cur, top));
                        }
                        out.push((n, Some(comp)));
                    }
                }
            }
            ExprKind::Binary { op, left, right } => {
                let (prop, other, flipped) = match (prop_of(left), prop_of(right)) {
                    (Some(p), None) => (p, right, false),
                    (None, Some(p)) => (p, left, true),
                    _ => return,
                };
                let mut ids = HashSet::default();
                self.collect_expr_idents(other, &mut ids);
                if rand_props.iter().any(|(n, _)| ids.contains(n)) {
                    return;
                }
                let c = self.eval_expr(other).to_u128() as i128;
                let top = full(prop.1);
                let eff = if flipped {
                    match op {
                        BinaryOp::Lt => BinaryOp::Gt,
                        BinaryOp::Gt => BinaryOp::Lt,
                        BinaryOp::Leq => BinaryOp::Geq,
                        BinaryOp::Geq => BinaryOp::Leq,
                        o => *o,
                    }
                } else {
                    *op
                };
                let rs: Option<Vec<(i128, i128)>> = match eff {
                    BinaryOp::Eq => Some(vec![(c, c)]),
                    BinaryOp::Neq => Some(vec![(0, c - 1), (c + 1, top)]),
                    BinaryOp::Lt => Some(vec![(0, c - 1)]),
                    BinaryOp::Leq => Some(vec![(0, c)]),
                    BinaryOp::Gt => Some(vec![(c + 1, top)]),
                    BinaryOp::Geq => Some(vec![(c, top)]),
                    _ => None,
                };
                out.push((prop.0, rs));
            }
            _ => {}
        }
    }

    pub(super) fn consequent_domains(
        &mut self,
        item: &ConstraintItem,
        rand_props: &[(String, u32)],
        out: &mut Vec<(String, Option<Vec<(i128, i128)>>)>,
    ) {
        if let ConstraintItem::Expr(e) = item {
            return self.consequent_domains_expr(e, rand_props, out);
        }
        let prop_of = |e: &Expression| -> Option<(String, u32)> {
            if let ExprKind::Ident(h) = &Self::unparen(e).kind {
                let n = &h.path.last()?.name.name;
                return rand_props.iter().find(|(p, _)| p == n).cloned();
            }
            None
        };
        let _full = |w: u32| -> i128 {
            if w >= 127 {
                i128::MAX
            } else {
                (1i128 << w) - 1
            }
        };
        let _inside_ranges = |me: &mut Self, ranges: &[Expression]| -> Vec<(i128, i128)> {
            let cr: Vec<ConstraintRange> = ranges
                .iter()
                .map(|r| match &r.kind {
                    ExprKind::Range(lo, hi) => ConstraintRange::Range {
                        lo: (**lo).clone(),
                        hi: (**hi).clone(),
                    },
                    _ => ConstraintRange::Value(r.clone()),
                })
                .collect();
            me.cons_ranges_i128(&cr)
        };
        match item {
            ConstraintItem::Block(items) => {
                for i in items {
                    self.consequent_domains(i, rand_props, out);
                }
            }
            ConstraintItem::Soft(inner) => self.consequent_domains(inner, rand_props, out),
            ConstraintItem::Inside {
                expr,
                range,
                is_dist,
                ..
            } if !*is_dist => {
                if let Some((n, _)) = prop_of(expr) {
                    let rs = self.cons_ranges_i128(range);
                    out.push((n, Some(rs)));
                }
            }
            _ => {}
        }
    }

    /// §18.5.10 without `solve … before`: every solution is equally likely,
    /// so an antecedent whose value selects the size of the consequent
    /// space must be drawn in proportion to that size. `sel inside {0,1,2}`
    /// with `(sel==0) -> (addr inside {[512:1023]})`, `(sel==2) ->
    /// !(addr inside {[512:1535]})` gives sel=2 almost always. Picking sel
    /// uniformly and repairing addr afterwards gave one third each.
    /// Returns None when the weights carry no information (uniform pick).
    pub(super) fn weighted_antecedent_pick(
        &mut self,
        handle: usize,
        name: &str,
        width: u32,
        ranges: &[(i128, i128)],
        constraints: &[ClassConstraint],
        rand_props: &[(String, u32)],
        solved: &HashMap<String, Value>,
    ) -> Option<Value> {
        let total: i128 = ranges.iter().map(|(lo, hi)| (hi - lo + 1).max(0)).sum();
        if total <= 1 || total > 64 || width == 0 || width > 64 {
            return None;
        }
        let imps = Self::class_implications(constraints);
        if imps.is_empty() {
            return None;
        }
        // Only implications whose condition reads this property and no
        // other unsolved rand property.
        let mine: Vec<(&Expression, ImpCons<'_>)> = imps
            .into_iter()
            .filter(|(cond, _)| {
                let mut ids = HashSet::default();
                self.collect_expr_idents(cond, &mut ids);
                ids.contains(name)
                    && !rand_props
                        .iter()
                        .any(|(n, _)| n != name && ids.contains(n) && !solved.contains_key(n))
            })
            .collect();
        if mine.is_empty() {
            return None;
        }
        let cands: Vec<i128> = ranges
            .iter()
            .flat_map(|(lo, hi)| *lo..=*hi)
            .filter(|c| *c >= 0)
            .collect();
        let saved = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .and_then(|i| i.properties.get(name).cloned());
        let full = |w: u32| -> f64 { 2f64.powi(w.min(127) as i32) };
        let mut weights: Vec<f64> = Vec::with_capacity(cands.len());
        self.this_stack.push(Some(handle));
        for &c in &cands {
            if let Some(Some(inst)) = self.heap.get_mut(handle) {
                inst.properties
                    .insert(name.to_string(), Value::from_u64(c as u64, width));
            }
            // per consequent property: the allowed count under the fired
            // consequents, or the full domain when none fires for it
            let mut per: HashMap<String, f64> = HashMap::default();
            let mut feasible = true;
            for (cond, cons) in &mine {
                if !self.eval_expr(cond).is_true() {
                    continue;
                }
                let mut doms = Vec::new();
                self.consequent_domains_of(cons, rand_props, &mut doms);
                for (pn, rs) in doms {
                    if pn == name || solved.contains_key(&pn) {
                        continue;
                    }
                    let Some(rs) = rs else { continue };
                    let cnt: f64 = rs
                        .iter()
                        .map(|(lo, hi)| ((hi - lo + 1).max(0)) as f64)
                        .sum();
                    let e = per.entry(pn).or_insert(f64::INFINITY);
                    *e = e.min(cnt);
                    if cnt <= 0.0 {
                        feasible = false;
                    }
                }
            }
            let _ = full;
            weights.push(if feasible {
                per.values().product::<f64>().max(f64::MIN_POSITIVE)
            } else {
                0.0
            });
        }
        self.this_stack.pop();
        // Properties restricted for SOME candidates but not others: the
        // unrestricted candidates get the full domain for them.
        let mut restricted: HashSet<String> = HashSet::default();
        {
            self.this_stack.push(Some(handle));
            for &c in &cands {
                if let Some(Some(inst)) = self.heap.get_mut(handle) {
                    inst.properties
                        .insert(name.to_string(), Value::from_u64(c as u64, width));
                }
                for (cond, cons) in &mine {
                    if self.eval_expr(cond).is_true() {
                        let mut doms = Vec::new();
                        self.consequent_domains_of(cons, rand_props, &mut doms);
                        for (pn, rs) in doms {
                            if rs.is_some() && pn != name && !solved.contains_key(&pn) {
                                restricted.insert(pn);
                            }
                        }
                    }
                }
            }
            self.this_stack.pop();
        }
        if !restricted.is_empty() {
            // recompute with the full-domain default for unrestricted props
            self.this_stack.push(Some(handle));
            for (k, &c) in cands.iter().enumerate() {
                if weights[k] == 0.0 {
                    continue;
                }
                if let Some(Some(inst)) = self.heap.get_mut(handle) {
                    inst.properties
                        .insert(name.to_string(), Value::from_u64(c as u64, width));
                }
                let mut per: HashMap<String, f64> = restricted
                    .iter()
                    .map(|pn| {
                        let w = rand_props
                            .iter()
                            .find(|(n, _)| n == pn)
                            .map(|(_, w)| *w)
                            .unwrap_or(32);
                        (pn.clone(), full(w))
                    })
                    .collect();
                for (cond, cons) in &mine {
                    if !self.eval_expr(cond).is_true() {
                        continue;
                    }
                    let mut doms = Vec::new();
                    self.consequent_domains_of(cons, rand_props, &mut doms);
                    for (pn, rs) in doms {
                        let Some(rs) = rs else { continue };
                        if let Some(e) = per.get_mut(&pn) {
                            let cnt: f64 = rs
                                .iter()
                                .map(|(lo, hi)| ((hi - lo + 1).max(0)) as f64)
                                .sum();
                            *e = e.min(cnt);
                        }
                    }
                }
                weights[k] = per.values().product::<f64>();
            }
            self.this_stack.pop();
        }
        // restore the property
        if let Some(Some(inst)) = self.heap.get_mut(handle) {
            match saved {
                Some(v) => {
                    inst.properties.insert(name.to_string(), v);
                }
                None => {
                    inst.properties.remove(name);
                }
            }
        }
        let sum: f64 = weights.iter().sum();
        if !(sum > 0.0)
            || weights
                .iter()
                .all(|w| (*w - weights[0]).abs() <= f64::EPSILON * weights[0].abs())
        {
            return None;
        }
        use rand::RngExt;
        let mut r: f64 = self.cur_rng().random_range(0.0..sum);
        let mut chosen = cands[cands.len() - 1];
        for (k, &c) in cands.iter().enumerate() {
            if r < weights[k] {
                chosen = c;
                break;
            }
            r -= weights[k];
        }
        Some(Value::from_u64(chosen as u64, width))
    }

    /// After a property is solved, implications whose condition now holds
    /// narrow the allowed ranges of the still-unsolved properties their
    /// consequents name, so those are drawn inside the consequent instead
    /// of being repaired by retries.
    pub(super) fn narrow_from_fired_implications(
        &mut self,
        handle: usize,
        constraints: &[ClassConstraint],
        rand_props: &[(String, u32)],
        solved: &HashMap<String, Value>,
        allowed: &mut HashMap<String, Vec<(i128, i128)>>,
    ) {
        let imps = Self::class_implications(constraints);
        if imps.is_empty() {
            return;
        }
        self.this_stack.push(Some(handle));
        for (cond, cons) in imps {
            let mut ids = HashSet::default();
            self.collect_expr_idents(cond, &mut ids);
            // every rand property the condition reads must be solved
            if rand_props
                .iter()
                .any(|(n, _)| ids.contains(n) && !solved.contains_key(n))
            {
                continue;
            }
            if !ids.iter().any(|i| rand_props.iter().any(|(n, _)| n == i)) {
                continue;
            }
            if !self.eval_expr(cond).is_true() {
                continue;
            }
            let mut doms = Vec::new();
            self.consequent_domains_of(&cons, rand_props, &mut doms);
            for (pn, rs) in doms {
                let Some(rs) = rs else { continue };
                if solved.contains_key(&pn) || rs.is_empty() {
                    continue;
                }
                let entry = allowed.entry(pn).or_default();
                if entry.is_empty() {
                    *entry = rs;
                } else {
                    // intersect
                    let mut inter = Vec::new();
                    for (alo, ahi) in entry.iter() {
                        for (blo, bhi) in &rs {
                            let lo = (*alo).max(*blo);
                            let hi = (*ahi).min(*bhi);
                            if lo <= hi {
                                inter.push((lo, hi));
                            }
                        }
                    }
                    if !inter.is_empty() {
                        *entry = inter;
                    }
                }
            }
        }
        self.this_stack.pop();
    }

    /// §18.7: the solve binds names in the OBJECT's scope — the calling
    /// subroutine's locals must not shadow the object's members (a caller
    /// local `addr` made every `addr` constraint of the object read it). The
    /// inline constraints' caller-scope operands were bound beforehand
    /// (`freeze_caller_scope_refs`), so the solve runs in a fresh frame.
    pub(super) fn exec_randomize_inner(
        &mut self,
        handle: usize,
        inline: &[ConstraintItem],
        inline_src_file: Option<u32>,
    ) -> Value {
        self.push_local_frame(HashMap::default());
        self.method_local_base.push(self.local_stack.len() - 1);
        let r = self.exec_randomize_solve(handle, inline, inline_src_file);
        self.method_local_base.pop();
        self.pop_local_frame();
        r
    }

    pub(super) fn exec_randomize_solve(
        &mut self,
        handle: usize,
        inline: &[ConstraintItem],
        inline_src_file: Option<u32>,
    ) -> Value {
        // Reset the dist-pick-once tracker so each randomize call gets a
        // fresh weighted draw per `(handle, prop)`.
        self.dist_picked_once.clear();
        // §18.4: the previous call's randc draws are final — commit them by
        // dropping the tentative record, so this call's trials only ever roll
        // back their OWN picks.
        self.randc_pending.clear();
        self.randc_wide_pending.clear();
        use rand::RngExt;
        let class_name = if let Some(Some(inst)) = self.heap.get(handle) {
            inst.class_name.clone()
        } else {
            return Value::zero(32);
        };
        // §18.6.2: randomize() calls pre_randomize() first and only then
        // computes the new values — so before anything below reads the
        // rand_mode / constraint_mode state or the object's members, which
        // pre_randomize() may change for THIS call (§18.8, §18.9).
        if self.class_has_method(&class_name, "pre_randomize") {
            self.exec_pre_randomize_preserving_children(handle);
        }

        let mut rand_props = Vec::new();
        let mut constraints = Vec::new();
        // Kept parallel with `constraints`. AST spans are file-local, so the
        // defining class (or call site for an inline block) supplies the
        // retained source index.
        let mut constraint_src_files: Vec<Option<u32>> = Vec::new();
        // §18.5.14.2: inheritance depth of each collected constraint block
        // (0 = the object's own class, 1 = its parent, …). A soft constraint
        // in a DERIVED class has HIGHER priority than a conflicting soft
        // constraint it inherits, so the lower depth wins.
        let mut constraint_depth: Vec<usize> = Vec::new();
        let mut class_depth: usize = 0;
        let mut seen_constraint_names: HashSet<String> = HashSet::default();
        let mut real_rand_props: HashSet<String> = HashSet::default();
        // §18.4.3: subset of rand_props that are declared `randc`.
        let mut randc_set: HashSet<String> = HashSet::default();
        // §18.13 rand_mode/constraint_mode disabled sets for this instance.
        let rand_disabled = self
            .rand_mode_disabled
            .get(&handle)
            .cloned()
            .unwrap_or_default();
        let constraint_disabled = self
            .constraint_mode_disabled
            .get(&handle)
            .cloned()
            .unwrap_or_default();
        let con_all_off = constraint_disabled.contains("*");
        // rand prop name -> enum type name, for props whose declared type is an
        // enum. Used so randomization picks a *valid* member (e.g. a riscv_reg_t
        // field gets x0..x31, not a 32-bit garbage value).
        let mut enum_prop_types: HashMap<String, String> = HashMap::default();

        // Rand fixed-array members (`rand reg_t gpr[4]`): (scoped name, lo, hi,
        // elem_width, enum_type). Randomized element-by-element after the scalar
        // fields they may exclude (sp/tp/scratch_reg) are solved.
        let mut rand_arrays: Vec<(String, String, i64, i64, u32, Option<String>)> = Vec::new();
        // §18.4: N-D fixed rand arrays. These lived in NEITHER element pipeline
        // — not in `rand_arrays` (built from the 1-D table only) and skipped by
        // the collection pipeline (`CollKind::Fixed`) — so nothing seeded them
        // and nothing coordinated a foreach constraint with a baseline.
        let mut rand_nd_arrays: Vec<(String, String, Vec<(i64, i64)>, u32)> = Vec::new();
        // §18.4 classification input: rand prop -> its declared type name.
        let mut prop_type_names: HashMap<String, String> = HashMap::default();

        // §18.4/§18.5.9/§18.6 — rand class-handle members (`rand SubObject
        // sub_inst`). A rand handle is NOT a random value: "the object handle
        // is not changed by randomize()" — the object it points at is
        // randomized RECURSIVELY, its class's constraints are solved
        // CONCURRENTLY with this object's, and the enclosing object's
        // constraints may refer to that object's members (`sub_inst.sub_data ==
        // q[0] + 5`). This is the ONE list of such handles: it drives the
        // recursive randomize, membership in `rand_set` (so a cross-object
        // lvalue is a solvable target) and the concurrent re-validation of the
        // sub-object's own constraints after the fixpoint.
        let mut rand_obj_props: Vec<String> = Vec::new();
        // §18.3: rand props whose declared type is SIGNED (`rand integer x`).
        // Their solved values must keep `is_signed`, or a later compare
        // (`x < 0`, `x inside {[-100:100]}`) silently goes unsigned.
        let mut signed_rand_props: HashSet<String> = HashSet::default();
        let mut cur = Some(class_name.clone());
        while let Some(cname) = cur {
            if let Some(class_def) = self.module.classes.get(&cname) {
                for prop in &self.randomize_members(class_def, &rand_disabled) {
                    let enum_t = class_def
                        .properties
                        .get(prop)
                        .and_then(|sig| sig.type_name.clone())
                        .filter(|tn| self.module.enum_members.contains_key(tn));
                    // A member typed by a class (a handle, or a collection of
                    // handles) holds object references, not random values.
                    // §8.25: so does one typed by a class type parameter
                    // this object binds to a class (`rand T h;`, T = leaf).
                    let is_handle = self.prop_class_type(handle, prop).is_some();
                    // §18.4: the elements of a fixed `rand obj arr[N]` are
                    // randomized recursively like any rand handle; drawing
                    // them as integers overwrote every handle with a random
                    // number, leaving the elements pointing at no object.
                    if let Some(&(lo, hi, w)) = class_def.array_properties.get(prop)
                        && !is_handle
                    {
                        let (lo, hi) = self.inst_fixed_bounds(class_def, handle, prop, (lo, hi));
                        rand_arrays.push((
                            prop.clone(),
                            format!("{}#{}", handle, prop),
                            lo,
                            hi,
                            w,
                            enum_t.clone(),
                        ));
                    }
                    if let Some((shape, w)) = class_def.array_nd_properties.get(prop)
                        && !is_handle
                    {
                        rand_nd_arrays.push((
                            prop.clone(),
                            format!("{}#{}", handle, prop),
                            shape.clone(),
                            *w,
                        ));
                    }
                    // A dynamic array / queue / associative member has no
                    // scalar value to draw — its elements are solved by the
                    // collection pipeline below. Drawing a random scalar for
                    // it would only pollute the property map.
                    let is_coll = class_def.queue_properties.contains_key(prop)
                        || class_def.assoc_properties.contains_key(prop);
                    if is_handle {
                        // §18.4/§18.5.9: randomize the referenced object, never
                        // the handle itself (a random handle would be a dangling
                        // pointer and every cross-object constraint would fail).
                        if !rand_obj_props.contains(prop) {
                            rand_obj_props.push(prop.clone());
                        }
                        continue;
                    }
                    if let Some(sig) = class_def.properties.get(prop) {
                        if !is_coll {
                            // §8.25/§18.4: a member typed by a class TYPE
                            // parameter (`rand T r;`) draws at the width and
                            // signedness of the type this object binds to T.
                            let bound = sig
                                .type_name
                                .as_ref()
                                .filter(|t| class_def.type_param_names.contains(t))
                                .and_then(|_| {
                                    let w = self.class_prop_width_of(handle, prop)?;
                                    let b = self.class_prop_type_name(handle, prop)?;
                                    Some((w, self.bound_type_is_signed(b.trim())))
                                });
                            let (w, signed) = bound.unwrap_or((sig.width, sig.is_signed));
                            rand_props.push((prop.clone(), w));
                            if sig.is_real {
                                real_rand_props.insert(prop.clone());
                            }
                            if signed {
                                signed_rand_props.insert(prop.clone());
                            }
                            static RAND_DBG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
                            if *RAND_DBG
                                .get_or_init(|| std::env::var_os("XEZIM_RAND_DBG").is_some())
                            {
                                eprintln!(
                                    "RPROP {} w={} signed={}",
                                    prop, sig.width, sig.is_signed
                                );
                            }
                            if let Some(tn) = enum_t {
                                enum_prop_types.insert(prop.clone(), tn);
                            }
                        }
                        if let Some(tn) = &sig.type_name {
                            prop_type_names.insert(prop.clone(), tn.clone());
                        }
                    }
                    if class_def.randc_properties.contains(prop) {
                        randc_set.insert(prop.clone());
                    }
                }
                // §18.5.4 constraint inheritance: a derived class that
                // declares a constraint with the SAME name as a parent's
                // REPLACES the parent's. Walking derived → parent, dedup
                // by constraint name so the parent's same-named constraint
                // is skipped once the derived one is in. Also §18.13:
                // skip constraint blocks that have constraint_mode(0).
                for (con_name, con) in class_def.constraints.iter() {
                    // §18.5.11: a STATIC constraint block's enable state is
                    // class-wide, so it is keyed by the DECLARING class rather
                    // than by this instance.
                    let disabled = if con.is_static {
                        self.static_constraint_disabled
                            .contains(&(cname.clone(), con_name.clone()))
                            || con_all_off
                    } else {
                        con_all_off || constraint_disabled.contains(con_name)
                    };
                    if disabled {
                        // Mark name as seen so a parent same-named block is
                        // not picked up to substitute the disabled derived.
                        seen_constraint_names.insert(con_name.clone());
                        continue;
                    }
                    if seen_constraint_names.insert(con_name.clone()) {
                        constraints.push(con.clone());
                        constraint_src_files
                            .push(self.module.src_file_of_module.get(&cname).copied());
                        constraint_depth.push(class_depth);
                    }
                }
                cur = class_def.extends.clone();
                class_depth += 1;
            } else {
                break;
            }
        }
        // §18.7 — inline `randomize() with { … }` constraints join the class's
        // constraint set for THIS call (and are solved last, so they win the
        // fixpoint's final word on any variable they pin).
        if !inline.is_empty() {
            constraints.push(ClassConstraint {
                is_static: false,
                is_extern: false,
                has_body: true,
                name: crate::ast::Identifier {
                    name: "__inline__".to_string(),
                    span: crate::ast::Span::new(0, 0),
                },
                items: inline.to_vec(),
                span: crate::ast::Span::new(0, 0),
            });
            constraint_src_files.push(inline_src_file);
        }

        // §18.5.5: expand the array SLICE of a `unique {…}` list into the
        // element-wise inequalities it denotes.
        self.expand_unique_slices(handle, &mut constraints, &self.module.parameters);

        // §18.3: a constraint may read a state variable of the module the
        // class is declared in. Statements get instance-qualified when the
        // module is inlined, but the class's constraint expressions do not —
        // so from inside an instance, `mode` named a nonexistent top-level
        // signal, read x, and the solve failed. Qualify a bare ident with the
        // calling process's instance scope when ONLY the scoped signal exists.
        // Class members never match (they are skipped explicitly, and have no
        // scoped signal), so only genuine module-scope state is touched.
        if !self.current_scope.is_empty() {
            let scope = self.current_scope.clone();
            let mut member_names: HashSet<String> = HashSet::default();
            let mut cn = Some(class_name.clone());
            while let Some(c) = cn {
                if let Some(cd) = self.module.classes.get(&c) {
                    member_names.extend(cd.properties.keys().cloned());
                    cn = cd.extends.clone();
                } else {
                    break;
                }
            }
            for con in &mut constraints {
                for it in &mut con.items {
                    self.scope_qualify_constraint_item(it, &scope, &member_names);
                }
            }
        }

        // Rand collection members (dynamic arrays, queues, associative arrays).
        let rand_colls = self.collect_rand_colls(handle, &rand_disabled, &constraints);
        // `unique {c}` items naming one of those collections (§18.5.5).
        let unique_colls: Vec<RandColl> = {
            let mut out: Vec<RandColl> = Vec::new();
            fn walk(item: &ConstraintItem, out: &mut Vec<String>) {
                match item {
                    ConstraintItem::Unique { exprs, .. } => {
                        for e in exprs {
                            if let ExprKind::Ident(h) = &e.kind {
                                if let Some(s) = h.path.last() {
                                    out.push(s.name.name.clone());
                                }
                            }
                        }
                    }
                    ConstraintItem::Block(items) => {
                        for i in items {
                            walk(i, out);
                        }
                    }
                    ConstraintItem::Soft(i) => walk(i, out),
                    _ => {}
                }
            }
            let mut names: Vec<String> = Vec::new();
            for con in &constraints {
                for it in &con.items {
                    walk(it, &mut names);
                }
            }
            for n in names {
                if let Some(c) = rand_colls.iter().find(|c| c.prop == n) {
                    out.push(c.clone());
                }
            }
            out
        };

        // ---- IEEE 1800-2017 §18.4: classify the non-scalar rand properties ----
        // Object handles were already split out above (`rand_obj_props`).
        //
        // A PACKED struct/union is a raw integral type: randomize its whole bit
        // pattern at the aggregate's true width, with no enum-member restriction
        // on any enum-typed member ("the rules in 18.3 restricting the random
        // values of an enum variable shall not apply to that member").
        //
        // An UNPACKED struct is solved MEMBER BY MEMBER — only members carrying
        // `rand`/`randc` participate, and an enum member there DOES follow §18.3.
        // prop -> total packed width.
        let mut packed_agg_props: HashMap<String, u32> = HashMap::default();
        // prop -> [(member, width, is_randc, enum_type)] for the rand members.
        let mut unpacked_agg_props: Vec<(String, Vec<(String, u32, bool, Option<String>)>)> =
            Vec::new();
        // `<prop>.<member>` of the signed rand members of those.
        let mut unpacked_agg_signed: HashSet<String> = HashSet::default();
        // Sorted so the member-draw order (and hence the RNG stream) does not
        // depend on hash iteration order.
        let mut typed_props: Vec<(&String, &String)> = prop_type_names.iter().collect();
        typed_props.sort_by(|a, b| a.0.cmp(b.0));
        for (prop, tn) in typed_props {
            let Some(dt) = self.module.typedef_types.get(tn.as_str()) else {
                continue;
            };
            let DataType::Struct(su) = Self::resolve_type_ref(dt, &self.module.typedef_types)
            else {
                continue;
            };
            if Self::spreads_member_wise(&su) {
                let mut members = Vec::new();
                for m in &su.members {
                    let Some(rq) = &m.rand_qualifier else {
                        continue;
                    };
                    let is_randc = matches!(rq, crate::ast::types::RandQualifier::Randc);
                    let w = resolve_type_width(
                        &m.data_type,
                        Some(&self.module.parameters),
                        Some(&self.module.typedefs),
                    )
                    .max(1);
                    let et = if let DataType::TypeReference { name, .. } = &m.data_type {
                        Some(name.name.name.clone())
                            .filter(|n| self.module.enum_members.contains_key(n))
                    } else {
                        None
                    };
                    let signed = super::elaborate::is_type_signed_resolved(
                        &m.data_type,
                        &self.module.typedef_types,
                    );
                    for d in &m.declarators {
                        if signed {
                            unpacked_agg_signed.insert(format!("{}.{}", prop, d.name.name));
                        }
                        members.push((d.name.name.clone(), w, is_randc, et.clone()));
                    }
                }
                unpacked_agg_props.push((prop.clone(), members));
            } else {
                let (_, total) = self.packed_agg_layout(&su);
                packed_agg_props.insert(prop.clone(), total);
            }
        }
        // Object handles and unpacked structs leave the scalar rand list; a
        // packed aggregate stays but at its TRUE width (elaboration records a
        // union's width as the default 32).
        let unpacked_names: HashSet<String> =
            unpacked_agg_props.iter().map(|(p, _)| p.clone()).collect();
        rand_props.retain(|(n, _)| !rand_obj_props.contains(n) && !unpacked_names.contains(n));
        for (n, w) in rand_props.iter_mut() {
            if let Some(tw) = packed_agg_props.get(n) {
                *w = *tw;
            }
        }
        self.this_stack.push(Some(handle));
        // §18.5.12: a constraint body is evaluated in the CLASS's scope, so an
        // unqualified call in a constraint (`user_func_res == calc_hash(a, b)`)
        // must resolve to a method of this class — and a bare collection member
        // (`$size(dyn_array)`) to its instance-scoped storage. Both look at
        // `class_context_stack`; without pushing it the call fell through to the
        // "unknown name" path and silently yielded 0.
        self.class_context_stack.push(Some(class_name.clone()));
        let has_post = self.class_has_method(&class_name, "post_randomize");

        // §18.6: a failed enclosing solve restores the rand state of its
        // reachable children as well as its own scalar/collection state.
        let child_handles = self.rand_child_handles(handle, &rand_obj_props);
        let has_rand_children = !child_handles.is_empty();
        if has_rand_children {
            let child_saved = self.snapshot_rand_children(&child_handles);
            self.rand_child_snapshots
                .push(rand_scope::RandChildTransaction {
                    owner: handle,
                    children: child_saved,
                });
        }

        // ---------------------------------------------------------------
        // IEEE 1800-2017 §18.5.10 — variable ordering.
        //
        // WITHOUT `solve … before …` the LRM requires all constraints to be
        // solved SIMULTANEOUSLY: the random values are distributed over the
        // joint SOLUTION SPACE, not per-variable. For
        //     (c == 0) -> (d == 8'h00)
        // the space has 1 solution with c==0 and 256 with c==1, so c==0 must
        // come up ~1/257 of the time. xezim's propagation solver draws each
        // variable uniformly and then REPAIRS the implication, which instead
        // gives c==0 half the time — the very distortion §18.5.10 says
        // `solve c before d` is needed to obtain.
        //
        // Rejection sampling over the joint domain reproduces the LRM
        // distribution exactly: draw every rand variable uniformly, accept
        // only a fully consistent assignment. It runs as a bounded pre-pass;
        // if no sample lands, the propagation solver below still guarantees a
        // result. When `solve … before …` IS present the pre-pass is skipped,
        // so the ordered variables keep their uniform per-variable draw (which
        // is exactly what the ordering hint asks for).
        if self.rejection_sample_ok(
            &constraints,
            &rand_props,
            &randc_set,
            &real_rand_props,
            &rand_colls,
            &rand_obj_props,
        ) {
            let mut backup: HashMap<String, Value> = HashMap::default();
            if let Some(Some(inst)) = self.heap.get(handle) {
                for (n, v) in &inst.properties {
                    backup.insert(n.clone(), v.clone());
                }
            }
            for _ in 0..256 {
                if !self.take_randomize_budget(handle, &class_name, &constraints) {
                    break;
                }
                for (name, width) in &rand_props {
                    let v = if let Some(et) = enum_prop_types.get(name) {
                        let members = self
                            .module
                            .enum_members
                            .get(et)
                            .cloned()
                            .unwrap_or_default();
                        if members.is_empty() {
                            Value::zero(*width)
                        } else {
                            let i = self.rng.random_range(0..members.len());
                            Value::from_u64(members[i].1, *width)
                        }
                    } else if *width <= 64 {
                        Value::from_u64(self.rng.random(), *width)
                    } else {
                        // A `rand` property wider than 64 bits (a packed
                        // multi-dimensional array such as `u7_t [4:0][1:0]`)
                        // was "drawn" as all zeros; draw every bit, from the
                        // same stream this loop already uses.
                        let mut acc = Value::zero(*width);
                        let mut bit = 0u32;
                        while bit < *width {
                            let chunk: u64 = self.rng.random();
                            let n = (*width - bit).min(64);
                            for k in 0..n {
                                if (chunk >> k) & 1 == 1 {
                                    acc.set_bit((bit + k) as usize, LogicBit::One);
                                }
                            }
                            bit += n;
                        }
                        acc
                    };
                    if let Some(Some(inst)) = self.heap.get_mut(handle) {
                        inst.properties.insert(name.clone(), v);
                    }
                }
                let ok = constraints.iter().all(|con| {
                    con.items
                        .iter()
                        .all(|it| self.check_constraint_item(handle, it))
                });
                if ok && !self.randomize_budget_exhausted {
                    if has_post {
                        self.exec_method_call(handle, "post_randomize", &[]);
                    }
                    self.this_stack.pop();
                    self.class_context_stack.pop();
                    if has_rand_children {
                        self.rand_child_snapshots.pop();
                    }
                    return Value::from_u64(1, 32);
                }
            }
            // No sample landed — restore and fall through to propagation.
            if let Some(Some(inst)) = self.heap.get_mut(handle) {
                for (n, v) in backup {
                    inst.properties.insert(n, v);
                }
            }
        }

        // Trials in which a strict fixed-array foreach check
        // failed. Such a class re-solves the same element system every trial
        // (the array is not re-seeded), so when the check keeps failing the
        // problem is almost certainly UNSATISFIABLE — stop after a bounded
        // number of full-cost attempts and return 0, rather than paying all
        // 1000 trials (§18.6.2 requires only that we return 0).
        let mut fixed_fe_fail_streak: u32 = 0;
        // §18.6: a failed randomize() leaves the random variables unchanged.
        // Scalars come back from each trial's `backup`; collection elements
        // are written in place by the trials, so remember them up front.
        let coll_saved: Vec<(&RandColl, u64, Vec<(String, Option<Value>)>)> = rand_colls
            .iter()
            .filter(|c| !c.nested && !c.is_object_elem)
            .map(|c| {
                let elems = self
                    .coll_elem_keys(c)
                    .into_iter()
                    .map(|k| {
                        let v = self.read_coll_elem(&k);
                        (k, v)
                    })
                    .collect();
                (c, self.get_queue_size(&c.scoped), elems)
            })
            .collect();
        // The joint solver (`rand_csp`) models integral scalars and 1-D
        // arrays; randc cycles, real values, object handles and unpacked
        // aggregates stay with the trial loop.
        // §18.5.9: rand sub-objects held in handle members, and in fixed,
        // dynamic and queue arrays of handles, join the fallback joint solve
        // (`csp_add_sub`); an associative array of objects, a shared or null
        // plain handle or a member-subset call keeps them out.
        let sub_objs: Option<Vec<(String, usize)>> = if self.randomize_subset.is_some() {
            None
        } else {
            self.rand_csp_subs(handle, &class_name, &rand_obj_props, &constraints)
        };
        let mut csp_ok = randc_set.is_empty()
            && real_rand_props.is_empty()
            && (rand_obj_props.is_empty() || sub_objs.is_some())
            && unpacked_agg_props
                .iter()
                .all(|(_, ms)| ms.iter().all(|m| !m.2));
        // §18.4 (#519): multi-dimensional rand arrays join the joint solve
        // as rows (`RandColl::nd_base`), so they no longer bar it.
        // §18.4: the rand members of an unpacked struct member are solver
        // scalars of their own, stored (and named) `<prop>.<member>`.
        let (csp_rand_props, csp_signed, csp_enums) = if unpacked_agg_props.is_empty() {
            (
                rand_props.clone(),
                signed_rand_props.clone(),
                enum_prop_types.clone(),
            )
        } else {
            let mut rp = rand_props.clone();
            let mut sg = signed_rand_props.clone();
            let mut en = enum_prop_types.clone();
            for (p, members) in &unpacked_agg_props {
                for (m, w, _, et) in members {
                    let key = format!("{}.{}", p, m);
                    if unpacked_agg_signed.contains(&key) {
                        sg.insert(key.clone());
                    }
                    if let Some(t) = et {
                        en.insert(key.clone(), t.clone());
                    }
                    rp.push((key, *w));
                }
            }
            (rp, sg, en)
        };
        let sub_objs = sub_objs.unwrap_or_default();
        let mut csp_runs = 0u32;
        let mut rand_diag = rand_diag::rand_diag_enabled().then(rand_diag::RandDiag::default);
        // Enum domains apply to every unpacked collection, not only fixed
        // arrays. A packed enum's unused bit patterns are not legal draws.
        let array_enums: HashMap<String, String> = rand_colls
            .iter()
            .filter_map(|c| {
                let prop = c.nd_base.as_ref().map_or(c.prop.as_str(), |(p, _)| p);
                let tn = self.class_prop_type_name(handle, prop)?;
                self.module
                    .enum_members
                    .contains_key(&tn)
                    .then(|| (c.prop.clone(), tn))
            })
            .collect();
        // §18.5.4/§18.5.10: a set whose DISTRIBUTION the per-variable trials
        // below cannot honour (see `rand_order_sensitive`) goes to the joint
        // solver first; the trials stay the fallback.
        let mut trials = 1000;
        // The joint solver proved that the set has no solution.
        let mut csp_unsat = false;
        let has_soft = Self::rand_has_soft(&constraints);
        if csp_ok
            && rand_obj_props.is_empty()
            && !rand_colls.iter().any(|c| c.kind == CollKind::Dyn)
            && (has_soft || self.rand_order_sensitive(&constraints))
        {
            csp_runs += 1;
            let saved = self
                .heap
                .get(handle)
                .and_then(|o| o.as_ref())
                .map(|i| i.properties.clone());
            let csp_outcome = self.rand_csp_solve(
                handle,
                &constraints,
                &constraint_depth,
                &csp_rand_props,
                &csp_signed,
                &csp_enums,
                &rand_colls,
                &array_enums,
                true,
                &[],
            );
            if std::env::var_os("XEZIM_RAND_DBG").is_some() {
                eprintln!(
                    "[rand-dbg] class joint solve: {:?} (collections {:?})",
                    csp_outcome,
                    rand_colls.iter().map(|c| c.prop.clone()).collect::<Vec<_>>()
                );
            }
            match csp_outcome {
                rand_csp::CspOutcome::Sat if !self.randomize_budget_exhausted => {
                    if has_post {
                        self.exec_method_call(handle, "post_randomize", &[]);
                    }
                    self.this_stack.pop();
                    self.class_context_stack.pop();
                    if has_rand_children {
                        self.rand_child_snapshots.pop();
                    }
                    return Value::from_u64(1, 32);
                }
                out => {
                    match out {
                        rand_csp::CspOutcome::Unsat => {
                            trials = 0;
                            csp_unsat = true;
                        }
                        rand_csp::CspOutcome::NotApplicable => csp_ok = false,
                        _ => {}
                    }
                    if let (Some(p), Some(Some(inst))) = (saved, self.heap.get_mut(handle)) {
                        inst.properties = p;
                    }
                }
            }
        }
        self.rand_tight_mode = false;
        // Per rand sub-object member: the items pushed into its solve.
        let mut pushdown: HashMap<String, Vec<ConstraintItem>> = HashMap::default();
        // §18.5.14: a soft item that kept rewriting a value another item set
        // back, in a trial that then failed, conflicts with a hard item; it is
        // discarded for the remaining trials (keyed by the item's address).
        let mut soft_dropped: HashSet<usize> = HashSet::default();
        // The rand array / collection members whose element domain a `!=`
        // re-pick must respect (see `rand_elem_ranges`).
        let elem_range_props: Vec<String> = rand_arrays
            .iter()
            .map(|a| a.0.clone())
            .chain(rand_nd_arrays.iter().map(|a| a.0.clone()))
            .chain(rand_colls.iter().map(|c| c.prop.clone()))
            .collect();
        for _trial in 0..trials {
            // Ctrl-C / SIGTERM: stop searching and let the run shut down.
            if interrupt_requested() {
                self.take_interrupt();
                break;
            }
            if !self.take_randomize_budget(handle, &class_name, &constraints) {
                break;
            }
            self.rand_tight_mode = fixed_fe_fail_streak >= 4;
            // LRM §18.5.4 dist: clear the pick-once gate at each trial so a
            // failed trial doesn't permanently freeze the dist constraint.
            // Within a single trial the gate still ensures the fixpoint
            // loop draws only one weighted sample per (handle, prop).
            self.dist_picked_once.clear();
            let mut solved_props: HashMap<String, Value> = HashMap::default();
            let mut backup = HashMap::default();

            // §18.4/§18.5.9: recursively randomize every `rand` OBJECT HANDLE,
            // so the sub-object's own constraints hold before this object's
            // fixpoint runs (`leaf_inst.leaf_val` may then be re-pinned by a
            // cross-object constraint here, and `sub_object_constraints_ok`
            // below re-validates the sub-object's constraints against the FINAL
            // assignment). The handle value itself is left untouched. This is
            // the only place a rand sub-object is drawn — a second, later redraw
            // would discard the cross-object solution just computed for it.
            for p in &rand_obj_props {
                // A `rand obj arr[]` (array/queue/assoc of rand objects) is
                // NOT a single handle — its element handles live at
                // `<handle>#p[i]`. Recurse into each non-null element so the
                // objects are randomized and aliasing (`arr[1]==arr[2]`) is
                // preserved (a shared element is randomized once, and both
                // slots read the same result). §18.5.9/§18.4.1.
                let coll_name = format!("{}#{}", handle, p);
                let is_coll = self.is_associative_array(&coll_name)
                    || self
                        .heap
                        .get(handle)
                        .and_then(|o| o.as_ref())
                        .map(|i| i.class_name.clone())
                        .and_then(|cn| self.module.classes.get(&cn).cloned())
                        .map(|cd| {
                            cd.queue_properties.contains_key(p)
                                || cd.array_properties.contains_key(p)
                        })
                        .unwrap_or(false);
                if is_coll {
                    if self.is_associative_array(&coll_name) {
                        // Associative array: iterate the stored keys.
                        let prefix = format!("{}[", coll_name);
                        let keys: Vec<String> = self
                            .signals
                            .keys_with_elem_prefix(&prefix)
                            .into_iter()
                            .filter(|k| k.ends_with(']'))
                            .map(|k| k[prefix.len()..k.len() - 1].to_string())
                            .collect();
                        for key in keys {
                            if let Some(v) =
                                self.get_signal_value_by_name(&format!("{}[{}]", coll_name, key))
                            {
                                let sub = v.to_u64().unwrap_or(0) as usize;
                                if sub != 0
                                    && sub != handle
                                    && self.randomize_depth < 8
                                    && self.heap.get(sub).and_then(|o| o.as_ref()).is_some()
                                {
                                    self.randomize_nested(sub);
                                }
                            }
                        }
                    } else {
                        // Dynamic array / queue / fixed array: 0-based indices.
                        let n = self.get_queue_size(&coll_name);
                        for i in 0..n {
                            if let Some(v) =
                                self.get_signal_value_by_name(&format!("{}[{}]", coll_name, i))
                            {
                                let sub = v.to_u64().unwrap_or(0) as usize;
                                if sub != 0
                                    && sub != handle
                                    && self.randomize_depth < 8
                                    && self.heap.get(sub).and_then(|o| o.as_ref()).is_some()
                                {
                                    // §18.5.9: `arr[i].fld …` items of this
                                    // solve join element i's own solve, as
                                    // for a single rand handle below.
                                    let pushed = pushdown
                                        .entry(format!("{}[{}]", p, i))
                                        .or_insert_with(|| {
                                            self.pushdown_elem_items(p, i, &constraints)
                                        })
                                        .clone();
                                    self.randomize_nested_with(sub, &pushed);
                                }
                            }
                        }
                    }
                    continue;
                }
                let sub = self
                    .heap
                    .get(handle)
                    .and_then(|o| o.as_ref())
                    .and_then(|i| i.properties.get(p))
                    .and_then(|v| v.to_u64())
                    .unwrap_or(0) as usize;
                if sub != 0
                    && sub != handle
                    && self.randomize_depth < 8
                    && self.heap.get(sub).and_then(|o| o.as_ref()).is_some()
                {
                    let pushed = pushdown
                        .entry(p.clone())
                        .or_insert_with(|| self.pushdown_items(p, &constraints))
                        .clone();
                    self.randomize_nested_with(sub, &pushed);
                }
            }

            // §18.4: an UNPACKED struct member is solved member by member; only
            // `rand`/`randc` members participate. An enum member here DOES obey
            // §18.3 (valid members only) — unlike one inside a packed struct.
            for (p, members) in &unpacked_agg_props {
                for (m, w, is_randc, et) in members {
                    let key = format!("{}.{}", p, m);
                    let v = if *is_randc {
                        let domain = self.randc_domain(&key, *w, et.as_deref(), None);
                        if domain.is_empty() {
                            Value::from_u64(self.rng.random::<u64>(), *w)
                        } else {
                            let pick = self.pick_randc(handle, &key, &domain);
                            Value::from_u64(pick, *w)
                        }
                    } else if let Some(members) = et
                        .as_ref()
                        .and_then(|t| self.module.enum_members.get(t.as_str()))
                    {
                        let n = members.len();
                        if n == 0 {
                            Value::zero(*w)
                        } else {
                            let mv = members[self.rng.random_range(0..n)].1;
                            Value::from_u64(mv, *w)
                        }
                    } else if *w <= 64 {
                        Value::from_u64(self.rng.random::<u64>(), *w)
                    } else {
                        Value::zero(*w)
                    };
                    let r = ClassAggRef::Unpacked { handle, key, w: *w };
                    self.write_class_agg(&r, &v);
                }
            }

            // First pass: identify simple range constraints for each property.
            // §18.3: the endpoints are evaluated with EXACT SIGNED arithmetic —
            // read as u64, `int_val inside {[-100:100]}` had lo = 0xFFFF_FF9C >
            // hi = 100, i.e. an empty range, and the draw fell back to garbage.
            let mut prop_allowed_ranges: HashMap<String, Vec<(i128, i128)>> = HashMap::default();
            let target_ranges = |me: &mut Self, expr: &Expression, ranges: &[ConstraintRange]| {
                if let ExprKind::Ident(h) = &expr.kind
                    && let Some((name, width)) = h
                        .path
                        .last()
                        .and_then(|n| rand_props.iter().find(|(p, _)| p == &n.name.name))
                {
                    me.cons_ranges_for_target(ranges, *width, signed_rand_props.contains(name))
                } else {
                    me.cons_ranges_i128(ranges)
                }
            };
            for con in &constraints {
                for item in &con.items {
                    let (inside_expr, inside_ranges): (
                        Option<&Expression>,
                        Option<Vec<(i128, i128)>>,
                    ) = match item {
                        ConstraintItem::Inside {
                            expr,
                            range,
                            is_dist,
                            ..
                        } if !*is_dist => {
                            let rs = target_ranges(self, expr, range);
                            (Some(expr), Some(rs))
                        }
                        ConstraintItem::Expr(expr) => {
                            if let ExprKind::Inside {
                                expr: inner,
                                ranges,
                            } = &expr.kind
                            {
                                let cr: Vec<ConstraintRange> = ranges
                                    .iter()
                                    .map(|r| match &r.kind {
                                        ExprKind::Range(lo, hi) => ConstraintRange::Range {
                                            lo: (**lo).clone(),
                                            hi: (**hi).clone(),
                                        },
                                        _ => ConstraintRange::Value(r.clone()),
                                    })
                                    .collect();
                                let rs = target_ranges(self, inner, &cr);
                                (Some(inner.as_ref()), Some(rs))
                            } else {
                                (None, None)
                            }
                        }
                        _ => (None, None),
                    };
                    if let (Some(expr), Some(ranges)) = (inside_expr, inside_ranges) {
                        if let ExprKind::Ident(hier) = &expr.kind {
                            let name = hier.path.last().unwrap().name.name.clone();
                            if !ranges.is_empty() {
                                prop_allowed_ranges.entry(name).or_default().extend(ranges);
                            }
                        }
                    }
                }
            }
            // Plain relational bounds (`prop REL const`) become an allowed
            // RANGE too, so the picker samples the satisfying interval directly
            // instead of drawing a full-width random value and hoping the
            // fixpoint repairs it — which never lands a narrow band near 2^w
            // (issue #35: `u >= 4294967290` on a 32-bit unsigned is 6 values in
            // 4.3 billion). The bound is read in i128 with the §11.8.1 sign of
            // the comparison: an UNSIGNED prop makes it unsigned, so 0xFFFFFFFA
            // is 4294967290, not the -6 that `to_i64` would sign-extend it to.
            // Only synthesized for props that have no `inside` range (a mix
            // would be a union here, not the intended intersection — the
            // fixpoint still handles that case).
            for (name, width) in &rand_props {
                if prop_allowed_ranges.contains_key(name)
                    || enum_prop_types.contains_key(name)
                    || real_rand_props.contains(name)
                {
                    continue;
                }
                let sgn = signed_rand_props.contains(name);
                let w = (*width).min(127);
                let (dlo, dhi): (i128, i128) = if sgn {
                    (
                        -(1i128 << (w.saturating_sub(1))),
                        (1i128 << (w.saturating_sub(1))) - 1,
                    )
                } else {
                    // §18.4: an unsigned domain of width w spans [0, 2^w - 1].
                    // For w == 127 that bound is i128::MAX, but the
                    // shift-then-subtract form underflows: `1i128 << 127` is
                    // i128::MIN (bit 127 is the sign bit), so `- 1` overflows
                    // in debug builds. Use the constant directly.
                    (
                        0,
                        if w == 127 {
                            i128::MAX
                        } else {
                            (1i128 << w) - 1
                        },
                    )
                };
                let (mut lo, mut hi) = (dlo, dhi);
                let mut bounded = false;
                for con in &constraints {
                    for item in &con.items {
                        let ConstraintItem::Expr(e) = item else {
                            continue;
                        };
                        let ExprKind::Binary { op, left, right } = &e.kind else {
                            continue;
                        };
                        if !matches!(
                            op,
                            BinaryOp::Geq | BinaryOp::Leq | BinaryOp::Gt | BinaryOp::Lt
                        ) {
                            continue;
                        }
                        let is_prop = |ex: &Expression| {
                            matches!(&ex.kind,
                            ExprKind::Ident(h) if h.path.last().is_some_and(|s| &s.name.name == name))
                        };
                        let prop_left = is_prop(left);
                        let prop_right = is_prop(right);
                        if prop_left == prop_right {
                            continue; // neither side, or both — not `prop REL const`
                        }
                        let lit_side = if prop_left { right } else { left };
                        // The bound must reference no rand target (a pure const).
                        let mut ids = HashSet::default();
                        self.collect_expr_idents(lit_side, &mut ids);
                        if rand_props.iter().any(|(n, _)| ids.contains(n)) {
                            continue;
                        }
                        let bv = self.eval_expr(lit_side);
                        let bound: i128 = if sgn {
                            match bv.to_i64() {
                                Some(x) => x as i128,
                                None => continue,
                            }
                        } else {
                            bv.to_u128() as i128
                        };
                        // Normalize to the prop-on-left orientation.
                        let eff = if prop_right {
                            match op {
                                BinaryOp::Geq => BinaryOp::Leq,
                                BinaryOp::Leq => BinaryOp::Geq,
                                BinaryOp::Gt => BinaryOp::Lt,
                                BinaryOp::Lt => BinaryOp::Gt,
                                o => *o,
                            }
                        } else {
                            *op
                        };
                        match eff {
                            BinaryOp::Geq => {
                                lo = lo.max(bound);
                                bounded = true;
                            }
                            BinaryOp::Gt => {
                                lo = lo.max(bound + 1);
                                bounded = true;
                            }
                            BinaryOp::Leq => {
                                hi = hi.min(bound);
                                bounded = true;
                            }
                            BinaryOp::Lt => {
                                hi = hi.min(bound - 1);
                                bounded = true;
                            }
                            _ => {}
                        }
                    }
                }
                if bounded && lo <= hi && (lo > dlo || hi < dhi) {
                    prop_allowed_ranges.insert(name.clone(), vec![(lo, hi)]);
                }
            }
            // Published for the fixpoint: an affine solve and a `!=` re-pick
            // must land the target INSIDE its declared range, not just satisfy
            // the one item they are looking at (§18.5.12 — the constraint set is
            // solved as a whole).
            self.rand_ranges = prop_allowed_ranges.clone();

            // Second pass: identify equality "assignments"
            let mut assignments: HashMap<String, Expression> = HashMap::default();
            for con in &constraints {
                for item in &con.items {
                    // §18.5.8.1: `arr.size() == n` sizes the rand array FROM
                    // n; reading the current size back into n pinned n to
                    // the previous call's length forever.
                    if rand_colls.iter().any(|c| {
                        c.kind == CollKind::Dyn
                            && Self::item_has_size_call(item, &c.prop, self.active_rand_receiver())
                    }) {
                        continue;
                    }
                    if let ConstraintItem::Expr(expr) = item {
                        if let ExprKind::Binary {
                            op: BinaryOp::Eq,
                            left,
                            right,
                        } = &expr.kind
                        {
                            if let ExprKind::Ident(hier) = &left.kind {
                                let name = hier.path.last().unwrap().name.name.clone();
                                // check if right-hand side doesn't contain 'name' to avoid self-reference
                                assignments.insert(name, *right.clone());
                            } else if let ExprKind::Ident(hier) = &right.kind {
                                let name = hier.path.last().unwrap().name.name.clone();
                                assignments.insert(name, *left.clone());
                            }
                        }
                    }
                }
            }

            // Local copy of properties for solving
            let mut current_props = HashMap::default();
            if let Some(Some(inst)) = self.heap.get(handle) {
                for (name, val) in &inst.properties {
                    current_props.insert(name.clone(), val.clone());
                    backup.insert(name.clone(), val.clone());
                }
            }

            // LRM §18.5.9 — `solve X before Y` precedence ordering.
            // Walk every constraint block, gather (before, after) name pairs,
            // build a precedence graph keyed by rand_props indices, and
            // topologically sort `pids_to_solve` so that X's index appears
            // before Y's. Names not referenced fall back to declaration
            // order. Cycles are broken by emitting any deadlocked nodes in
            // their original order (matches what most simulators do — a
            // proper LRM-strict implementation would error on cycles, but
            // we prefer best-effort for compatibility with imperfect
            // testbenches).
            let mut solve_pairs: Vec<(String, String)> = Vec::new();
            for con in &constraints {
                Self::collect_solve_pairs(&con.items, &mut solve_pairs);
            }
            let mut pids_to_solve: Vec<usize> = (0..rand_props.len()).collect();
            let mut ante: HashSet<String> = HashSet::default();
            if !solve_pairs.is_empty() {
                let name_to_idx: HashMap<String, usize> = rand_props
                    .iter()
                    .enumerate()
                    .map(|(i, (n, _))| (n.clone(), i))
                    .collect();
                let n = rand_props.len();
                let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
                let mut in_deg: Vec<u32> = vec![0; n];
                for (b, a) in &solve_pairs {
                    if let (Some(&bi), Some(&ai)) = (name_to_idx.get(b), name_to_idx.get(a)) {
                        if bi != ai {
                            adj[bi].push(ai);
                            in_deg[ai] += 1;
                        }
                    }
                }
                // Kahn's algorithm — preserves declaration order within
                // equivalent layers by always taking the lowest-numbered
                // ready node.
                let mut topo: Vec<usize> = Vec::with_capacity(n);
                let mut emitted = vec![false; n];
                loop {
                    let next = (0..n).find(|&i| !emitted[i] && in_deg[i] == 0);
                    if let Some(idx) = next {
                        emitted[idx] = true;
                        topo.push(idx);
                        for &nei in &adj[idx] {
                            if in_deg[nei] > 0 {
                                in_deg[nei] -= 1;
                            }
                        }
                    } else {
                        break;
                    }
                }
                // Append any unsolved (cycle members) in original order.
                for i in 0..n {
                    if !emitted[i] {
                        topo.push(i);
                    }
                }
                pids_to_solve = topo;
            } else {
                // Antecedents of implications first (see weighted_antecedent_pick).
                for (cond, _) in Self::class_implications(&constraints) {
                    let mut ids = HashSet::default();
                    self.collect_expr_idents(cond, &mut ids);
                    ante.extend(
                        ids.into_iter()
                            .filter(|i| rand_props.iter().any(|(n, _)| n == i)),
                    );
                }
                if !ante.is_empty() {
                    let (first, rest): (Vec<usize>, Vec<usize>) = pids_to_solve
                        .iter()
                        .copied()
                        .partition(|&i| ante.contains(&rand_props[i].0));
                    pids_to_solve = first.into_iter().chain(rest).collect();
                }
            }
            let has_solve_before = !solve_pairs.is_empty();
            let mut progress = true;
            while !pids_to_solve.is_empty() && progress {
                progress = false;
                let mut i = 0;
                while i < pids_to_solve.len() {
                    let pid_idx = pids_to_solve[i];
                    let (name, width) = &rand_props[pid_idx];

                    // If it has an assignment, check if we can solve it
                    if let Some(expr) = assignments.get(name) {
                        let mut idents = HashSet::default();
                        self.collect_expr_idents(expr, &mut idents);
                        let ready = idents.iter().all(|id| {
                            !rand_props.iter().any(|(n, _)| n == id)
                                || solved_props.contains_key(id)
                        });

                        if ready {
                            // Temporary set props in instance for eval_expr if needed?
                            // No, eval_expr uses self.signals and self.heap[handle].properties.
                            // We need to update the instance properties during solving if eval_expr depends on them.
                            if let Some(Some(inst)) = self.heap.get_mut(handle) {
                                for (n, v) in &solved_props {
                                    inst.properties.insert(n.clone(), v.clone());
                                }
                            }

                            self.this_stack.push(Some(handle));
                            let mut val = self.eval_expr(expr);
                            self.this_stack.pop();

                            // §18.4 (measured): an equality pin lands at the
                            // property's DECLARED width/signedness — `bit
                            // [3:0] y` pinned to a negative int solves to 15.
                            // Widening happens FIRST with the pinned value's
                            // OWN signedness (an unsigned 16-bit 0x8000
                            // zero-extends to +32768, §11.8.1), then the
                            // property's signedness applies.
                            if *width > 0 && val.width != *width {
                                val = val.resize(*width);
                            }
                            val.is_signed = signed_rand_props.contains(name);
                            solved_props.insert(name.clone(), val);
                            pids_to_solve.remove(i);
                            progress = true;
                            continue;
                        }
                    } else {
                        // No assignment, pick randomly (honoring ranges if possible)
                        let mut val = Value::zero(*width);
                        if real_rand_props.contains(name) {
                            // SV-2023: real-typed rand. Scan constraints for
                            // `name >= lit` / `name <= lit` bounds and sample
                            // uniformly. Default to [0.0, 1.0) when unbound.
                            let mut lo: f64 = f64::NEG_INFINITY;
                            let mut hi: f64 = f64::INFINITY;
                            for con in &constraints {
                                for item in &con.items {
                                    if let ConstraintItem::Expr(e) = item {
                                        if let ExprKind::Binary { op, left, right } = &e.kind {
                                            let prop_on_left = matches!(&left.kind,
                                                ExprKind::Ident(h) if h.path.last()
                                                    .is_some_and(|s| &s.name.name == name));
                                            let prop_on_right = matches!(&right.kind,
                                                ExprKind::Ident(h) if h.path.last()
                                                    .is_some_and(|s| &s.name.name == name));
                                            if !prop_on_left && !prop_on_right {
                                                continue;
                                            }
                                            let lit_side = if prop_on_left { right } else { left };
                                            let lit = self.eval_expr(lit_side).to_f64();
                                            let effective_op = if prop_on_right {
                                                match op {
                                                    BinaryOp::Geq => BinaryOp::Leq,
                                                    BinaryOp::Leq => BinaryOp::Geq,
                                                    BinaryOp::Gt => BinaryOp::Lt,
                                                    BinaryOp::Lt => BinaryOp::Gt,
                                                    other => *other,
                                                }
                                            } else {
                                                *op
                                            };
                                            match effective_op {
                                                BinaryOp::Geq | BinaryOp::Gt if lit > lo => {
                                                    lo = lit;
                                                }
                                                BinaryOp::Leq | BinaryOp::Lt if lit < hi => {
                                                    hi = lit;
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                            }
                            let (lo_e, hi_e) = if !lo.is_finite() && !hi.is_finite() {
                                (0.0_f64, 1.0_f64)
                            } else if !lo.is_finite() {
                                (hi - 1.0, hi)
                            } else if !hi.is_finite() {
                                (lo, lo + 1.0)
                            } else {
                                (lo, hi)
                            };
                            let pick = if hi_e > lo_e {
                                self.cur_rng().random_range(lo_e..hi_e)
                            } else {
                                lo_e
                            };
                            val = Value::from_f64(pick);
                            solved_props.insert(name.clone(), val.clone());
                            if let Some(Some(inst)) = self.heap.get_mut(handle) {
                                inst.properties.insert(name.clone(), val);
                            }
                            pids_to_solve.remove(i);
                            progress = true;
                            continue;
                        }
                        if randc_set.contains(name) {
                            // §18.4.3 randc cycle: enumerate the prop's domain
                            // and pick from the values not yet used in this cycle.
                            let domain = self.randc_domain(
                                name,
                                *width,
                                enum_prop_types.get(name).map(|s| s.as_str()),
                                prop_allowed_ranges.get(name).map(|v| v.as_slice()),
                            );
                            let pick = if !domain.is_empty() {
                                Some(self.pick_randc(handle, name, &domain))
                            } else if (17..=32).contains(width)
                                && !enum_prop_types.contains_key(name)
                            {
                                self.pick_randc_wide(
                                    handle,
                                    name,
                                    *width,
                                    prop_allowed_ranges.get(name).map(|v| v.as_slice()),
                                )
                            } else {
                                if *width > 32 {
                                    self.warn_system_task_once(
                                        "randc>32",
                                        "Warning: a randc variable wider than 32 bits is \
                                         randomized as rand (IEEE 1800-2017 §18.4.2)",
                                    );
                                }
                                None
                            };
                            if let Some(pick) = pick {
                                val = Value::from_u64(pick, *width);
                                solved_props.insert(name.clone(), val.clone());
                                if let Some(Some(inst)) = self.heap.get_mut(handle) {
                                    inst.properties.insert(name.clone(), val);
                                }
                                pids_to_solve.remove(i);
                                progress = true;
                                continue;
                            }
                        }
                        // §18.3: an `inside`-bounded draw samples the EXACT
                        // (signed) interval and keeps the property's signedness.
                        let sgn = signed_rand_props.contains(name);
                        // A narrow antecedent with no range of its own (`bit
                        // s` in `s -> d == 0`) is weighed over its whole domain.
                        let narrow_ante = if !sgn
                            && *width <= 6
                            && ante.contains(name)
                            && !enum_prop_types.contains_key(name)
                            && !prop_allowed_ranges.contains_key(name)
                        {
                            self.weighted_antecedent_pick(
                                handle,
                                name,
                                *width,
                                &[(0, (1i128 << *width) - 1)],
                                &constraints,
                                &rand_props,
                                &solved_props,
                            )
                        } else {
                            None
                        };
                        if let Some(ranges) = prop_allowed_ranges.get(name).cloned() {
                            let weighted = if has_solve_before || sgn {
                                None
                            } else {
                                self.weighted_antecedent_pick(
                                    handle,
                                    name,
                                    *width,
                                    &ranges,
                                    &constraints,
                                    &rand_props,
                                    &solved_props,
                                )
                            };
                            if let Some(p) = weighted {
                                val = p;
                            } else if let Some(p) = self.pick_i128_range(&ranges, *width, sgn) {
                                val = p;
                            }
                        } else if let Some(p) = narrow_ante {
                            val = p;
                        } else if let Some(et) = enum_prop_types.get(name) {
                            // Enum-typed rand field: pick a valid member.
                            let n = self.module.enum_members.get(et).map_or(0, |m| m.len());
                            if n > 0 {
                                let idx = self.cur_rng().random_range(0..n);
                                let mv = self.module.enum_members.get(et).unwrap()[idx].1;
                                val = Value::from_u64(mv, *width);
                            }
                        } else {
                            val = self.random_value_of_width(*width);
                        }
                        val.is_signed = sgn;
                        solved_props.insert(name.clone(), val.clone());
                        if let Some(Some(inst)) = self.heap.get_mut(handle) {
                            inst.properties.insert(name.clone(), val);
                        }
                        self.narrow_from_fired_implications(
                            handle,
                            &constraints,
                            &rand_props,
                            &solved_props,
                            &mut prop_allowed_ranges,
                        );
                        pids_to_solve.remove(i);
                        progress = true;
                        continue;
                    }

                    i += 1;
                }
            }

            // Pick truly randomly for remaining
            for pid_idx in pids_to_solve {
                let (name, width) = &rand_props[pid_idx];
                let mut val = Value::zero(*width);
                if randc_set.contains(name) {
                    let domain = self.randc_domain(
                        name,
                        *width,
                        enum_prop_types.get(name).map(|s| s.as_str()),
                        prop_allowed_ranges.get(name).map(|v| v.as_slice()),
                    );
                    if !domain.is_empty() {
                        let pick = self.pick_randc(handle, name, &domain);
                        val = Value::from_u64(pick, *width);
                        solved_props.insert(name.clone(), val);
                        continue;
                    }
                }
                if let Some(et) = enum_prop_types.get(name) {
                    let n = self.module.enum_members.get(et).map_or(0, |m| m.len());
                    if n > 0 {
                        let idx = self.cur_rng().random_range(0..n);
                        let mv = self.module.enum_members.get(et).unwrap()[idx].1;
                        val = Value::from_u64(mv, *width);
                    }
                } else if *width <= 64 {
                    val = Value::from_u64(self.cur_rng().random(), *width);
                } else {
                    val = self.random_value_of_width(*width);
                }
                val.is_signed = signed_rand_props.contains(name);
                solved_props.insert(name.clone(), val);
            }

            // Apply solved props to instance
            if let Some(Some(inst)) = self.heap.get_mut(handle) {
                for (name, val) in &solved_props {
                    // §18.4 (measured): the solution lands at the property's
                    // DECLARED width — `bit [3:0] y` pinned equal to a
                    // negative int reads back truncated (15), not as a
                    // 32-bit -1 leaking through the 4-bit field.
                    let mut v = val.clone();
                    if !real_rand_props.contains(name) {
                        if let Some(&(_, w)) = rand_props.iter().find(|(n, _)| n == name) {
                            if w > 0 {
                                if v.width != w {
                                    v = v.resize(w);
                                }
                                v.is_signed = signed_rand_props.contains(name);
                            }
                        }
                    }
                    inst.properties.insert(name.clone(), v);
                }
            }

            // §18.5.12 (issue #4): COUPLED affine constraints (`A + B < 1000`,
            // `A < B`, `A + B == 50`) tie two rand props together, so the
            // per-variable draw above cannot satisfy them. Solve those props
            // sequentially and overwrite their independently-drawn values; the
            // acceptance check + trial retry below still guards the result
            // (an UNSAT set keeps failing and honestly returns 0).
            {
                let mut cvar_names: Vec<String> = Vec::new();
                let mut cvar_dom: Vec<(i128, i128)> = Vec::new();
                let mut cvar_w: Vec<(u32, bool)> = Vec::new();
                for (name, width) in &rand_props {
                    if enum_prop_types.contains_key(name)
                        || real_rand_props.contains(name)
                        || randc_set.contains(name)
                    {
                        continue;
                    }
                    let s = signed_rand_props.contains(name);
                    let ww = (*width).min(127);
                    let (lo, hi): (i128, i128) = if s {
                        (
                            -(1i128 << ww.saturating_sub(1)),
                            (1i128 << ww.saturating_sub(1)) - 1,
                        )
                    } else {
                        // §18.4: unsigned domain [0, 2^ww - 1]; for ww == 127
                        // that bound is i128::MAX, and the shift form
                        // underflows in debug builds.
                        (
                            0,
                            if ww == 127 {
                                i128::MAX
                            } else {
                                (1i128 << ww) - 1
                            },
                        )
                    };
                    cvar_names.push(name.clone());
                    cvar_dom.push((lo, hi));
                    cvar_w.push((*width, s));
                }
                if cvar_names.len() >= 2 {
                    let citems: Vec<ConstraintItem> = constraints
                        .iter()
                        .flat_map(|c| c.items.iter().cloned())
                        .collect();
                    let all_unsigned = cvar_w.iter().all(|(_, s)| !*s);
                    self.constraint_cmp_unsigned = all_unsigned;
                    let coupled = self.solve_coupled_affine(
                        &citems,
                        &cvar_names,
                        &cvar_dom,
                        _trial,
                        Some(handle),
                    );
                    self.constraint_cmp_unsigned = false;
                    if let Some(pairs) = coupled {
                        for (col, val) in pairs {
                            let name = cvar_names[col].clone();
                            let (w, s) = cvar_w[col];
                            let masked = if w >= 64 {
                                val as u64
                            } else {
                                (val as u64) & ((1u64 << w) - 1)
                            };
                            let mut v = Value::from_u64(masked, w);
                            v.is_signed = s;
                            solved_props.insert(name.clone(), v.clone());
                            if let Some(Some(inst)) = self.heap.get_mut(handle) {
                                inst.properties.insert(name, v);
                            }
                        }
                    }
                }
            }

            // Constraint propagation: with a random baseline now in place,
            // iteratively apply the *forced* assignments implied by equality,
            // dist/inside, and conditional (if/else, implication) constraints.
            // Conditions are re-evaluated against the current solved state each
            // pass, so chains like `init_privileged_mode` (dist) -> `if
            // (init_privileged_mode != MACHINE_MODE) ...` converge over a few
            // passes. This is what lets riscv-dv's tightly-coupled config
            // constraints be satisfied without relying on random luck.
            // §18.4: `rand` OBJECT HANDLES and UNPACKED-struct properties left
            // the scalar list (they are not solved as integrals) but they are
            // still rand — a constraint may target their MEMBERS
            // (`leaf_inst.leaf_val`, `unpacked_struct_inst.unpacked_rand_val`),
            // and `rand_member_target` gates on the receiver being rand here.
            let rand_set: HashSet<String> = rand_props
                .iter()
                .map(|(n, _)| n.clone())
                .chain(rand_obj_props.iter().cloned())
                .chain(unpacked_agg_props.iter().map(|(p, _)| p.clone()))
                .collect();
            // LRM §18.5.14.2/§18.5.14.3: soft constraints have a PRIORITY
            // order, and a higher-priority soft constraint DISCARDS a
            // conflicting lower-priority one instead of failing the solve.
            // Priority (highest first):
            //   1. a soft constraint declared in a more-DERIVED class
            //      (§18.5.14.2: a derived-class soft constraint overrides the
            //      one it inherits) — i.e. the lower inheritance depth wins;
            //   2. within one class, the soft constraint that appears LATER in
            //      the source ("trailing constraints override leading ones"),
            //      by block source position, then by item position in a block.
            // For each target variable keep only the winning soft item; the
            // losers are skipped in the fixpoint below (a base-class
            // `soft len == 10` must NOT clobber a derived `soft len == 20`).
            let mut soft_winner: HashMap<String, (usize, usize)> = HashMap::default();
            let mut soft_rank: HashMap<String, (usize, usize, usize)> = HashMap::default();
            for (bi, con) in constraints.iter().enumerate() {
                for (ii, item) in con.items.iter().enumerate() {
                    if !matches!(item, ConstraintItem::Soft(_))
                        || soft_dropped.contains(&(item as *const ConstraintItem as usize))
                    {
                        continue;
                    }
                    let v = match Self::constraint_item_target(item) {
                        Some(v) => v,
                        None => continue,
                    };
                    let rank = (constraint_depth[bi], con.span.start, ii);
                    let wins = match soft_rank.get(&v) {
                        None => true,
                        Some(&(d, s, i0)) => {
                            rank.0 < d
                                || (rank.0 == d && (rank.1 > s || (rank.1 == s && rank.2 > i0)))
                        }
                    };
                    if wins {
                        soft_rank.insert(v.clone(), rank);
                        soft_winner.insert(v, (bi, ii));
                    }
                }
            }
            // Passes in which each soft item changed a value, per repair loop.
            let mut soft_fights: HashMap<usize, u32> = HashMap::default();
            for _pass in 0..16 {
                self.refresh_elem_ranges(&elem_range_props, &constraints);
                let mut changed = false;
                for (bi, con) in constraints.iter().enumerate() {
                    for (ii, item) in con.items.iter().enumerate() {
                        let key = item as *const ConstraintItem as usize;
                        let is_soft = Self::soft_only(item);
                        if is_soft && soft_dropped.contains(&key) {
                            continue;
                        }
                        // Skip a soft constraint that lost the soft-priority
                        // contest above. One that conflicts with a hard item
                        // is found by the repair itself and discarded
                        // (`soft_dropped`); a compatible one must hold.
                        if let ConstraintItem::Soft(_) = item {
                            if let Some(v) = Self::constraint_item_target(item) {
                                if soft_winner
                                    .get(&v)
                                    .is_some_and(|&(wb, wi)| wb != bi || wi != ii)
                                {
                                    continue;
                                }
                            }
                        }
                        if self.solve_forced(handle, item, &rand_set) {
                            changed = true;
                            if is_soft {
                                *soft_fights.entry(key).or_default() += 1;
                            }
                        }
                    }
                }
                if !changed {
                    break;
                }
            }

            // ---- §18.4 collection pipeline: SIZE, then SEED, then SOLVE ----
            // Runs now, AFTER the scalar-equality propagation above. A rand
            // dynamic array whose `.size()` is tied to a rand SCALAR
            // (`m_data.size() == m.length` with `m.length == N`) is only
            // size-able once the propagation has pinned the scalar to its
            // constrained value; sizing it earlier yields the scalar's raw,
            // unconstrained draw and the `.size() == m.length` acceptance then
            // fails. `unique {}` runs last over the settled elements.

            // Push rand SCALAR enum fields off any reserved value implied by
            // `!(field inside {…})` / `field != X` exclusions. Iterated so
            // inter-field chains converge (riscv-dv: ra/sp/tp first, then
            // scratch_reg which excludes them). Without this a field can keep a
            // randomly-picked ZERO and emit an illegal `la x0, …`.
            for _xpass in 0..4 {
                let mut changed = false;
                for (name, et) in &enum_prop_types {
                    if rand_arrays.iter().any(|(p, ..)| p == name) {
                        continue; // arrays handled separately below
                    }
                    let excl = self.collect_scalar_exclusions(name, &constraints);
                    if excl.is_empty() {
                        continue;
                    }
                    let cur = self
                        .heap
                        .get(handle)
                        .and_then(|o| o.as_ref())
                        .and_then(|i| i.properties.get(name))
                        .and_then(|v| v.to_u64());
                    if let Some(cv) = cur {
                        if excl.contains(&cv) {
                            let members: Vec<u64> = self
                                .module
                                .enum_members
                                .get(et)
                                .map(|m| {
                                    m.iter()
                                        .map(|x| x.1)
                                        .filter(|v| !excl.contains(v))
                                        .collect()
                                })
                                .unwrap_or_default();
                            if !members.is_empty() {
                                let pick = members[self.cur_rng().random_range(0..members.len())];
                                let w = self
                                    .heap
                                    .get(handle)
                                    .and_then(|o| o.as_ref())
                                    .and_then(|i| i.properties.get(name))
                                    .map(|v| v.width)
                                    .unwrap_or(32);
                                if self.set_prop_if_changed(handle, name, Value::from_u64(pick, w))
                                {
                                    changed = true;
                                }
                            }
                        }
                    }
                }
                if !changed {
                    break;
                }
            }

            // Randomize rand fixed-array elements now that the scalar fields
            // their exclusions reference (sp/tp/scratch_reg) are solved.
            // each element pick a valid enum member outside the array's
            // `foreach !(elem inside {…})` exclusion, distinct across the array
            // (best-effort `unique{}`).
            for (prop, scoped, lo, hi, width, enum_t) in &rand_arrays {
                let signed = self.class_prop_signed_of(handle, prop);
                let excluded = self.collect_array_exclusions(prop, &constraints);
                let pool: Vec<u64> = if let Some(et) = enum_t {
                    self.module
                        .enum_members
                        .get(et)
                        .map(|m| {
                            m.iter()
                                .map(|x| x.1)
                                .filter(|v| !excluded.contains(v))
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                let mut used: HashSet<u64> = HashSet::default();
                for i in *lo..=*hi {
                    let v = if !pool.is_empty() {
                        // Prefer an unused pool member (unique); fall back to any.
                        let avail: Vec<u64> =
                            pool.iter().copied().filter(|v| !used.contains(v)).collect();
                        let src = if avail.is_empty() { &pool } else { &avail };
                        let pick = src[self.cur_rng().random_range(0..src.len())];
                        used.insert(pick);
                        pick
                    } else if *width <= 64 {
                        let mask = if *width >= 64 {
                            u64::MAX
                        } else {
                            (1u64 << *width) - 1
                        };
                        self.cur_rng().random::<u64>() & mask
                    } else {
                        0
                    };
                    let mut val = if !pool.is_empty() || *width <= 64 {
                        Value::from_u64(v, *width)
                    } else {
                        self.random_value_of_width(*width)
                    };
                    val.is_signed = signed;
                    self.signals.insert(format!("{}[{}]", scoped, i), val);
                }
            }

            // N-D fixed rand arrays: fresh per-element baseline over the whole
            // shape, unless a positive foreach body owns the array (same
            // skip-set as the 1-D pool pass above).
            for (prop, scoped, shape, width) in &rand_nd_arrays {
                let signed = self.class_prop_signed_of(handle, prop);
                if shape.iter().any(|&(lo, hi)| hi < lo) {
                    continue;
                }
                let mut idx: Vec<i64> = shape.iter().map(|d| d.0).collect();
                loop {
                    let suffix: String = idx.iter().map(|i| format!("[{}]", i)).collect();
                    let v = if *width <= 64 {
                        let mask = if *width >= 64 {
                            u64::MAX
                        } else {
                            (1u64 << *width) - 1
                        };
                        self.cur_rng().random::<u64>() & mask
                    } else {
                        0
                    };
                    // Elements wider than 64 bits were left at zero.
                    let mut val = if *width <= 64 {
                        Value::from_u64(v, *width)
                    } else {
                        self.random_value_of_width(*width)
                    };
                    val.is_signed = signed;
                    self.signals.insert(format!("{}{}", scoped, suffix), val);
                    let mut k = shape.len();
                    loop {
                        if k == 0 {
                            break;
                        }
                        k -= 1;
                        idx[k] += 1;
                        if idx[k] <= shape[k].1 {
                            break;
                        }
                        idx[k] = shape[k].0;
                    }
                    if idx.iter().zip(shape.iter()).all(|(i, d)| *i == d.0) {
                        break;
                    }
                }
            }

            // §18.5.5 `unique {…}`: re-enforce AFTER the rand_array pool pass
            // above, which just re-seeded every fixed-array element with fresh
            // randoms — distinctness applied during the fixpoint would not
            // otherwise survive into the final state. Both forms are re-run:
            // the whole-array `unique {arr}` item and the pairwise `!=` items a
            // multi-expression `unique {a, b, arr[0:2]}` desugars to (each `!=`
            // re-pick respects its target's declared range, so this cannot
            // knock a scalar out of its `inside` set).
            // The array draws above run AFTER the first repair pass and
            // overwrite whatever it wrote into array elements (`a[0] == 5`,
            // foreach bodies). Arrays under a foreach used to be skipped
            // instead, which left them undrawn: a zero that already satisfied
            // `e[i] < 100` was kept, and repeated randomize() calls returned
            // the same values. Draw everything, then repair the hard items and
            // the foreach items once more against the fresh draws.
            {
                fn has_foreach(item: &ConstraintItem) -> bool {
                    match item {
                        ConstraintItem::Foreach { .. } => true,
                        ConstraintItem::Block(items) => items.iter().any(has_foreach),
                        ConstraintItem::Soft(inner) => has_foreach(inner),
                        _ => false,
                    }
                }
                let mut fights: HashMap<usize, u32> = HashMap::default();
                for _pass in 0..4 {
                    self.refresh_elem_ranges(&elem_range_props, &constraints);
                    let mut changed = false;
                    for item in constraints
                        .iter()
                        .flat_map(|c| c.items.iter())
                        .chain(inline.iter())
                        .filter(|it| !matches!(it, ConstraintItem::Soft(_)) || has_foreach(it))
                    {
                        let key = item as *const ConstraintItem as usize;
                        let is_soft = Self::soft_only(item);
                        if is_soft && soft_dropped.contains(&key) {
                            continue;
                        }
                        if self.solve_forced(handle, item, &rand_set) {
                            changed = true;
                            if is_soft {
                                *fights.entry(key).or_default() += 1;
                            }
                        }
                    }
                    if !changed {
                        break;
                    }
                }
                for (k, n) in fights {
                    let e = soft_fights.entry(k).or_default();
                    *e = (*e).max(n);
                }
            }
            let mut pinned_elems: HashSet<String> = HashSet::default();
            if !rand_colls.is_empty() {
                self.solve_array_sizes(&rand_colls, &constraints);
                // Fresh random baseline for every element of a dynamic /
                // associative member (fixed arrays keep the legacy pool pass).
                // §18.4.1: a CLASS-element collection (`rand obj arr[]`) holds
                // object handles that are randomized RECURSIVELY — never drawn
                // as scalars, or the handle is overwritten with junk.
                for c in &rand_colls {
                    if c.kind == CollKind::Fixed || c.is_object_elem {
                        continue;
                    }
                    for key in self.coll_elem_keys(c) {
                        let prev = self.read_coll_elem(&key).and_then(|v| v.to_u64());
                        let members = array_enums
                            .get(&c.prop)
                            .and_then(|tn| self.module.enum_members.get(tn))
                            .filter(|m| !m.is_empty());
                        let v = if let Some(members) = members {
                            let values: Vec<u64> = members.iter().map(|m| m.1).collect();
                            let i = self.cur_rng().random_range(0..values.len());
                            Value::from_u64(values[i], c.width)
                        } else {
                            self.draw_elem(c.width, prev)
                        };
                        self.write_coll_elem(&key, v);
                    }
                }
                self.solve_coll_foreach(handle, &rand_colls, &constraints, &mut pinned_elems);
                self.solve_top_level_elems(handle, &constraints, &mut pinned_elems);
                for c in &unique_colls {
                    self.enforce_unique_coll(c, &constraints, &pinned_elems);
                }
            }
            {
                fn collect_unique_items<'a>(
                    item: &'a ConstraintItem,
                    out: &mut Vec<&'a ConstraintItem>,
                ) {
                    match item {
                        ConstraintItem::Unique { .. } => out.push(item),
                        ConstraintItem::Expr(e) => {
                            if matches!(
                                &e.kind,
                                ExprKind::Binary {
                                    op: BinaryOp::Neq,
                                    ..
                                }
                            ) {
                                out.push(item);
                            }
                        }
                        ConstraintItem::Block(items) => {
                            for it in items {
                                collect_unique_items(it, out);
                            }
                        }
                        ConstraintItem::Soft(inner) => collect_unique_items(inner, out),
                        _ => {}
                    }
                }
                let mut uniques: Vec<&ConstraintItem> = Vec::new();
                for con in &constraints {
                    for item in &con.items {
                        collect_unique_items(item, &mut uniques);
                    }
                }
                // A re-pick can introduce a NEW collision with a pair already
                // visited, so iterate to a fixpoint (bounded).
                for _pass in 0..8 {
                    let mut changed = false;
                    for item in &uniques {
                        if self.solve_forced(handle, item, &rand_set) {
                            changed = true;
                        }
                    }
                    if !changed {
                        break;
                    }
                }
            }

            // §18.5.8.2: `arr.sum() == K` cannot be met by rejection over a wide
            // domain — distribute a random valid assignment onto the elements so
            // their total lands exactly on the target (respecting per-element
            // ranges and any element already pinned by another constraint).
            if !rand_colls.is_empty() {
                self.solve_array_sum_eqs(&constraints, &rand_colls, &rand_set, &mut pinned_elems);
            }

            // §18.5.7 foreach over a PACKED vector's bits (`foreach
            // (possible[i]) if (possible[i]) pattern[i] dist …; else
            // pattern[i] == 0;`). Not a rand COLLECTION, so every collection
            // arm skips it, and `constraint_unmodeled` then excused it from
            // the satisfaction check — the guard bits were silently ignored.
            // Pin the bits deterministically here, after the draws.
            for con in &constraints {
                for item in &con.items {
                    self.apply_packed_bit_foreach(handle, item, &rand_set);
                }
            }
            for item in inline {
                self.apply_packed_bit_foreach(handle, item, &rand_set);
            }

            let mut fe_failed = false;
            let mut all_ok =
                self.rand_items_accept(handle, &constraints, &rand_colls, &mut fe_failed);
            // Repairs can overwrite the baseline. Check the implicit enum
            // domain too, including when all explicit constraints are off.
            for c in &rand_colls {
                if let Some(tn) = array_enums.get(&c.prop) {
                    let values: Vec<u64> = self.module.enum_members[tn]
                        .iter()
                        .map(|m| m.1)
                        .collect();
                    for key in self.coll_elem_keys(c) {
                        if !self
                            .read_coll_elem(&key)
                            .and_then(|v| v.to_u64())
                            .is_some_and(|v| values.contains(&v))
                        {
                            all_ok = false;
                            fe_failed = true;
                        }
                    }
                }
            }
            if !all_ok && let Some(diag) = rand_diag.as_mut() {
                self.rand_diag_tally(handle, &constraints, diag);
            }
            if fe_failed {
                fixed_fe_fail_streak += 1;
            }
            // A soft item that changed a value in two passes of one repair
            // loop was undone in between: it fights a hard item (§18.5.14).
            // Discard it and retry even an accepted trial, so a lower-priority
            // soft item on the same variable gets its turn.
            let fought: Vec<usize> = soft_fights
                .iter()
                .filter(|&(k, &n)| n >= 2 && !soft_dropped.contains(k))
                .map(|(&k, _)| k)
                .collect();
            if !fought.is_empty() {
                soft_dropped.extend(fought);
                all_ok = false;
            }

            // §18.4 concurrent solve: a `rand` object handle's OWN constraints
            // are part of this randomize()'s problem. A cross-object constraint
            // here (`leaf_inst.leaf_val == packed_struct_inst.payload + 10`)
            // re-pins the sub-object's variable, so its class constraints
            // (`leaf_val > 50`) must be re-validated against the FINAL
            // assignment — otherwise a trial that quietly violated them would be
            // accepted. Failing here retries the whole trial with a fresh draw.
            if all_ok && !rand_obj_props.is_empty() {
                all_ok = self.sub_object_constraints_ok(handle, &rand_obj_props);
            }

            if all_ok && !(has_soft && csp_ok) && !self.randomize_budget_exhausted {
                // SV semantics: randomize() calls post_randomize() on success
                // (e.g. riscv_instr builds its imm_str / formats operands here).
                if has_post {
                    self.exec_method_call(handle, "post_randomize", &[]);
                }
                self.this_stack.pop();
                self.class_context_stack.pop();
                if has_rand_children {
                    self.rand_child_snapshots.pop();
                }
                return Value::from_u64(1, 32);
            }

            // §18.5: constraints that couple many variables at once (an array
            // `sum()` with `unique` and an ordering chain) defeat the
            // per-variable repair above; solve the set jointly instead. A
            // dynamic array keeps the size this trial drew, so an outcome is
            // final only when no array is sized at random: with fixed shapes
            // every trial would hand the solver the same problem.
            if csp_ok && csp_runs < 8 {
                // §18.5.8.1: the joint solve sizes an array whose size a
                // constraint reads itself (`csp_size_phase`).
                let sized = rand_colls.iter().any(|c| {
                    c.kind == CollKind::Dyn
                        && !(sub_objs.is_empty() && Self::csp_size_solved(c, &constraints))
                });
                csp_runs += 1;
                match self.rand_csp_solve(
                    handle,
                    &constraints,
                    &constraint_depth,
                    &csp_rand_props,
                    &csp_signed,
                    &csp_enums,
                    &rand_colls,
                    &array_enums,
                    false,
                    &sub_objs,
                ) {
                    rand_csp::CspOutcome::Sat
                        if !self.randomize_budget_exhausted
                            && self.sub_object_constraints_ok(handle, &rand_obj_props) =>
                    {
                        // The joint solve rewrote the sub-objects after
                        // their own post_randomize ran: run it on the final
                        // values.
                        for (_, sub) in &sub_objs {
                            let cls = self
                                .heap
                                .get(*sub)
                                .and_then(|o| o.as_ref())
                                .map(|i| i.class_name.clone());
                            if cls.is_some_and(|c| self.class_has_method(&c, "post_randomize")) {
                                self.exec_method_call(*sub, "post_randomize", &[]);
                            }
                        }
                        if has_post {
                            self.exec_method_call(handle, "post_randomize", &[]);
                        }
                        self.this_stack.pop();
                        self.class_context_stack.pop();
                        if has_rand_children {
                            self.rand_child_snapshots.pop();
                        }
                        return Value::from_u64(1, 32);
                    }
                    rand_csp::CspOutcome::Unsat if !sized => {
                        if let Some(Some(inst)) = self.heap.get_mut(handle) {
                            for (name, val) in backup {
                                inst.properties.insert(name, val);
                            }
                        }
                        csp_unsat = true;
                        break;
                    }
                    rand_csp::CspOutcome::NotApplicable => csp_ok = false,
                    rand_csp::CspOutcome::GaveUp if !sized => csp_ok = false,
                    _ => {}
                }
                // §18.5.14/§18.6.1: with soft items present a trial is left
                // to the joint solver, which honours every soft item it can.
                // Once its runs are used up without an answer, the trials
                // judge their own assignments again (their repair drops a
                // soft item only after it fought a hard one), so an
                // assignment satisfying every hard constraint is accepted
                // rather than failing randomize() for the solver's sake.
                if csp_runs >= 8 {
                    csp_ok = false;
                }
            }
            if fixed_fe_fail_streak >= 12 {
                if let Some(Some(inst)) = self.heap.get_mut(handle) {
                    for (name, val) in backup {
                        inst.properties.insert(name, val);
                    }
                }
                break;
            }

            // On the final trial, when diagnostics are requested, report which
            // constraint items remain unsatisfiable so the solver gaps for a
            // given class are visible without a full re-run.
            if _trial == 999 && std::env::var("XEZIM_RAND_DBG").is_ok() {
                eprintln!(
                    "[rand-dbg] {} randomize FAILED after 1000 trials; {} rand props, {} constraint blocks",
                    class_name,
                    rand_props.len(),
                    constraints.len()
                );
                let mut shown = 0;
                for con in &constraints {
                    for item in &con.items {
                        if !Self::constraint_unmodeled(item)
                            && !self.check_constraint_item(handle, item)
                            && shown < 30
                        {
                            eprintln!("[rand-dbg]   FAIL: {:?}", item);
                            shown += 1;
                        }
                    }
                }
            }

            if let Some(Some(inst)) = self.heap.get_mut(handle) {
                for (name, val) in backup {
                    inst.properties.insert(name, val);
                }
            }
        }

        for (c, n, elems) in coll_saved {
            if c.kind == CollKind::Dyn {
                self.resize_coll(&c.scoped, n);
            }
            for (k, v) in elems {
                match v {
                    Some(v) => self.write_coll_elem(&k, v),
                    None => {
                        self.signals.remove(&k);
                    }
                }
            }
        }
        if let Some(diag) = &rand_diag {
            self.rand_diag_report(
                &class_name,
                &rand_props,
                &rand_colls,
                &rand_obj_props,
                &rand_disabled,
                &constraint_disabled,
                &constraints,
                &constraint_src_files,
                diag,
            );
        }
        // §18.6.3: say why the outermost call returns 0 (a budget overrun
        // has already been reported).
        if self.randomize_depth == 0 && !self.randomize_budget_exhausted {
            self.rand_fail_warn(&class_name, &constraints, &constraint_src_files, csp_unsat);
        }
        // §18.4: randomize() failed — no value was actually produced, so give
        // every tentatively-drawn randc value back to its permutation cycle.
        if has_rand_children {
            if let Some(transaction) = self.rand_child_snapshots.pop() {
                self.restore_rand_children(transaction.children);
            }
        }
        self.randc_rollback_all();
        self.this_stack.pop();
        self.class_context_stack.pop();
        Value::zero(32)
    }

    /// The trial acceptance judge of `exec_randomize_inner`: every constraint
    /// item the checker models holds. `fe_failed` reports a failed strict
    /// fixed-array foreach check.
    pub(super) fn rand_items_accept(
        &mut self,
        handle: usize,
        constraints: &[ClassConstraint],
        rand_colls: &[RandColl],
        fe_failed: &mut bool,
    ) -> bool {
        for con in constraints {
            for item in &con.items {
                if !self.rand_item_accept(handle, item, rand_colls, fe_failed) {
                    return false;
                }
            }
        }
        true
    }

    pub(super) fn rand_has_soft(constraints: &[ClassConstraint]) -> bool {
        fn contains(item: &ConstraintItem) -> bool {
            match item {
                ConstraintItem::Soft(_) => true,
                ConstraintItem::Block(items) => items.iter().any(contains),
                ConstraintItem::Foreach { item, .. } => contains(item),
                ConstraintItem::Implication { constraint, .. } => contains(constraint),
                ConstraintItem::IfElse {
                    then_item,
                    else_item,
                    ..
                } => contains(then_item) || else_item.as_ref().is_some_and(|item| contains(item)),
                _ => false,
            }
        }
        constraints.iter().any(|con| con.items.iter().any(contains))
    }

    pub(super) fn rand_item_accept(
        &mut self,
        handle: usize,
        item: &ConstraintItem,
        rand_colls: &[RandColl],
        fe_failed: &mut bool,
    ) -> bool {
        // §18.5.14: a soft item (or a foreach / block of them) is a
        // preference, never a reason to reject a trial.
        if Self::soft_only(item) {
            return true;
        }
        match item {
            ConstraintItem::Block(items) => {
                return items
                    .iter()
                    .all(|item| self.rand_item_accept(handle, item, rand_colls, fe_failed));
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                return if self.eval_expr(condition).is_true() {
                    self.rand_item_accept(handle, then_item, rand_colls, fe_failed)
                } else {
                    else_item.as_ref().is_none_or(|item| {
                        self.rand_item_accept(handle, item, rand_colls, fe_failed)
                    })
                };
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                return !self.eval_expr(condition).is_true()
                    || self.rand_item_accept(handle, constraint, rand_colls, fe_failed);
            }
            _ => {}
        }
        // Constraints over a rand COLLECTION (its `.size()`, its
        // `foreach` bodies, `unique {}` over it) ARE modeled — check
        // them strictly, so an unsatisfied one retries the trial
        // instead of being silently accepted.
        if let Some(ok) = self.coll_item_check(handle, item, rand_colls) {
            if !ok {
                return false;
            }
            return true;
        }
        if let Some(ok) = self.check_fixed_nd_unique(handle, item) {
            return ok;
        }
        // Foreach over a FIXED-shape rand array: check strictly,
        // so an unsatisfied body retries the trial and an
        // UNSATISFIABLE one makes randomize() return 0 (§18.6.2)
        // instead of being silently accepted with violating
        // values.
        if let Some(ok) = self.check_fixed_foreach_item(handle, item) {
            if !ok {
                *fe_failed = true;
                return false;
            }
            return true;
        }
        // Skip constraints the solver structurally cannot satisfy
        // (dynamic-array size/sum/element relations, foreach, solve
        // ordering) so they don't block an otherwise-valid config.
        if Self::constraint_unmodeled(item) {
            return true;
        }
        if !self.check_constraint_item(handle, item) {
            return false;
        }
        true
    }

    /// If `expr` is a bare reference to a randomizable scalar property, return
    /// its name. Used by the propagation solver to recognise the target of an
    /// equality / dist / conditional assignment.
    pub(super) fn rand_lvalue_name(
        &self,
        expr: &Expression,
        rand_set: &HashSet<String>,
    ) -> Option<String> {
        if let ExprKind::Ident(hier) = &expr.kind {
            if hier.path.len() == 1 {
                let n = &hier.path[0].name.name;
                if rand_set.contains(n) {
                    return Some(n.clone());
                }
            }
        }
        None
    }

    /// §18.7 — inside `obj.randomize() with { … }`, an ELEMENT read of a
    /// receiver-prefixed member (`obj.member[i]`, `obj.member[i:j]`) must
    /// resolve against the randomized object's per-element stores, exactly
    /// like the bare spelling `member[i]` does inside the solve. The
    /// whole-property member read (`eval_expr_member_access`) serves the
    /// object's SNAPSHOT value, and for a dynamic array / queue the
    /// elements live in the `<handle>#member[i]` cells the solver writes —
    /// so `eval_expr_ctx` evaluates the receiver-stripped chain instead.
    /// Kept out of line so the expression dispatcher stays small.
    #[inline(never)]
    pub(super) fn strip_rand_receiver_expr(&self, expr: &Expression) -> Option<Expression> {
        match &expr.kind {
            ExprKind::Index { expr: base, index } => {
                self.strip_rand_receiver_prefix(base).map(|b| {
                    Expression::new(
                        ExprKind::Index {
                            expr: Box::new(b),
                            index: index.clone(),
                        },
                        expr.span,
                    )
                })
            }
            ExprKind::RangeSelect {
                expr: base,
                kind,
                left,
                right,
            } => self.strip_rand_receiver_prefix(base).map(|b| {
                Expression::new(
                    ExprKind::RangeSelect {
                        expr: Box::new(b),
                        kind: *kind,
                        left: left.clone(),
                        right: right.clone(),
                    },
                    expr.span,
                )
            }),
            // `obj.m.size` / `obj.m.size()` and other members or methods
            // of a receiver-prefixed property: the sizing pass resizes
            // the object's own `m`, so the acceptance check must read it.
            ExprKind::MemberAccess { expr: base, member } => {
                self.strip_rand_receiver_prefix(base).map(|b| {
                    Expression::new(
                        ExprKind::MemberAccess {
                            expr: Box::new(b),
                            member: member.clone(),
                        },
                        expr.span,
                    )
                })
            }
            ExprKind::Call { func, args } => match &func.kind {
                ExprKind::MemberAccess { expr: base, member } => {
                    self.strip_rand_receiver_prefix(base).map(|b| {
                        Expression::new(
                            ExprKind::Call {
                                func: Box::new(Expression::new(
                                    ExprKind::MemberAccess {
                                        expr: Box::new(b),
                                        member: member.clone(),
                                    },
                                    func.span,
                                )),
                                args: args.clone(),
                            },
                            expr.span,
                        )
                    })
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// §18.7 — the receiver name of the in-flight `obj.randomize() with
    /// {…}`, while it may stand for the randomized object. Two limits:
    /// - only in the solve's own frame, where the inline items are
    ///   evaluated. `pre_randomize`/`post_randomize` and any method they
    ///   call run in their own frames, where `obj` is an ordinary name;
    /// - names in the inline block resolve in the object's scope first, so
    ///   when the object has a property called `obj`, `obj.member` is that
    ///   property's member, not the receiver's (`randomize_object_with`
    ///   then records no receiver).
    pub(super) fn active_rand_receiver(&self) -> Option<&str> {
        let recv = self.rand_receiver.as_deref()?;
        if self.method_local_base.last() != Some(&self.rand_receiver_base) {
            return None;
        }
        Some(recv)
    }

    /// §18.7 — inside `obj.randomize() with { … }`, a receiver-prefixed
    /// reference (`obj.member`, then indexed as `obj.member[i]`) names the
    /// randomized object's OWN property. The element and whole-element
    /// constraint paths (`coll_elem_expr_key`, `rand_whole_elem_lvalue`)
    /// key by the bare member name, and the receiver local is out of scope
    /// in the solver frame, so hand them the prefix-stripped base while the
    /// root matches the in-flight `rand_receiver`. `None` when `expr` is
    /// not such a reference (either parse shape: `MemberAccess` or a
    /// two-segment dotted `Ident`).
    pub(super) fn strip_rand_receiver_prefix(&self, expr: &Expression) -> Option<Expression> {
        let recv = self.active_rand_receiver()?;
        let member: crate::ast::Identifier = match &expr.kind {
            ExprKind::MemberAccess { expr: base, member } => {
                if Self::plain_ident_name(base).as_deref() != Some(recv) {
                    return None;
                }
                member.clone()
            }
            ExprKind::Ident(h) if h.path.len() == 2 && h.path[0].selects.is_empty() => {
                if h.path[0].name.name != recv {
                    return None;
                }
                h.path[1].name.clone()
            }
            _ => return None,
        };
        let fh = crate::ast::expr::HierarchicalIdentifier {
            root: None,
            path: vec![crate::ast::expr::HierPathSegment {
                name: member.clone(),
                selects: Vec::new(),
            }],
            span: expr.span,
            cached_signal_id: std::cell::Cell::new(None),
            cached_resolved_name: std::cell::OnceCell::new(),
        };
        Some(Expression::new(ExprKind::Ident(fh), expr.span))
    }

    /// §18.7 — an inline `obj.randomize() with { obj.member == … }` constraint
    /// refers to the randomized object's own rand property through its
    /// RECEIVER (the bare handle `obj`, possibly `obj.field`). The forcing
    /// solver keys targets by the object's bare property name, so a prefixed
    /// `obj.member` must be recognised as the bare `member` when `obj`
    /// evaluates to the object currently being randomized (`handle`).
    pub(super) fn prefixed_self_member(
        &mut self,
        expr: &Expression,
        handle: usize,
        rand_set: &HashSet<String>,
    ) -> Option<String> {
        let ExprKind::MemberAccess { expr: recv, member } = &expr.kind else {
            return None;
        };
        let prop = member.name.clone();
        if !rand_set.contains(&prop) {
            return None;
        }
        let h = self.eval_expr(recv).to_u64().unwrap_or(0) as usize;
        if h == handle {
            return Some(prop);
        }
        // Inside the randomized frame the receiver LOCAL is out of scope
        // (eval returns 0), so recognise it by name when it matches the
        // `obj.randomize() with { obj.member … }` receiver.
        if let Some(rn) = Self::plain_ident_name(recv) {
            if self.active_rand_receiver() == Some(&rn) {
                return Some(prop);
            }
        }
        None
    }

    /// §18.4: a constraint LHS that is not a bare rand property but is still a
    /// solvable target of THIS randomize(): a member of an aggregate rand
    /// property (`packed_struct_inst.payload`, `packed_union_inst.union_enum`)
    /// or a property of a `rand` OBJECT HANDLE (`leaf_inst.leaf_val`), which is
    /// solved concurrently with the enclosing object.
    pub(super) fn rand_member_target(
        &mut self,
        expr: &Expression,
        rand_set: &HashSet<String>,
    ) -> Option<RandMemberTarget> {
        if let Some(r) = self.rand_agg_select_target(expr, rand_set) {
            return Some(RandMemberTarget::Agg(r));
        }
        let (base, field) = Self::split_trailing_member(expr)?;
        // The receiver must be a `rand` property of the object being solved,
        // or an element of a rand collection of handles (`arr[i].fld`, §18.4).
        let mut root = &base;
        while let ExprKind::Index { expr: b, .. } = &root.kind {
            root = b;
        }
        let ExprKind::Ident(bh) = &root.kind else {
            return None;
        };
        if bh.path.len() != 1 || !rand_set.contains(&bh.path[0].name.name) {
            return None;
        }
        if !std::ptr::eq(root, &base) {
            // Only a collection of handles: other indexed receivers keep
            // their own element paths.
            let this = self.this_stack.last().copied().flatten()?;
            self.prop_class_type(this, &bh.path[0].name.name)?;
        } else if let Some(r) = self.class_agg_member_parts(&base, &field) {
            return Some(RandMemberTarget::Agg(r));
        }
        // `rand` object handle: resolve it and target the sub-object's property.
        let sub = self.eval_expr(&base).to_u64()? as usize;
        let sub_class = self
            .heap
            .get(sub)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone())?;
        let mut cur = Some(sub_class);
        while let Some(cn) = cur {
            let cd = self.module.classes.get(&cn)?;
            if cd.properties.contains_key(&field) {
                // §18.3/§18.13: a rand handle does not make every property of
                // its child random. Disabled fields remain state variables.
                if !cd.random_properties.contains(&field)
                    || self
                        .rand_mode_disabled
                        .get(&sub)
                        .is_some_and(|d| d.contains("*") || d.contains(&field))
                    || self.prop_class_type(sub, &field).is_some()
                {
                    return None;
                }
                return Some(RandMemberTarget::Sub(sub, field));
            }
            cur = cd.extends.clone();
        }
        None
    }

    /// §18.4: a NESTED member (`s.f1.f2`) or a SELECT (`s.f[15:0]`,
    /// `s.f[i]`) of a `rand` struct property of the object being solved —
    /// the storage it aliases. A single whole member (`s.f`) is left to
    /// `class_agg_member_parts` in `rand_member_target`.
    pub(super) fn rand_agg_select_target(
        &mut self,
        expr: &Expression,
        rand_set: &HashSet<String>,
    ) -> Option<ClassAggRef> {
        let mut cur = expr;
        let mut steps = 0usize;
        let mut has_select = false;
        loop {
            match &cur.kind {
                ExprKind::MemberAccess { expr: b, .. } => {
                    steps += 1;
                    cur = b;
                }
                ExprKind::Index { expr: b, .. } | ExprKind::RangeSelect { expr: b, .. } => {
                    steps += 1;
                    has_select = true;
                    cur = b;
                }
                _ => break,
            }
        }
        let ExprKind::Ident(h) = &cur.kind else {
            return None;
        };
        if h.path.len() != 1
            || !h.path[0].selects.is_empty()
            || !rand_set.contains(&h.path[0].name.name)
            || (steps < 2 && !has_select)
        {
            return None;
        }
        self.class_member_select_ref(expr)
    }

    /// `rand_member_target` limited to an AGGREGATE member (`s.f`, `s.f1.f2`,
    /// `s.f[3:0]`). The relational and dist/inside arms use this: a property
    /// of a rand OBJECT handle (`a.x < b.y`) stays with the cross-object
    /// joint solve, which runs the sub-object's `post_randomize` on the final
    /// values — forcing it here rewrote `a.x` after that hook had run.
    pub(super) fn rand_agg_member_target(
        &mut self,
        expr: &Expression,
        rand_set: &HashSet<String>,
    ) -> Option<RandMemberTarget> {
        match self.rand_member_target(expr, rand_set)? {
            t @ RandMemberTarget::Agg(_) => Some(t),
            RandMemberTarget::Sub(..) => None,
        }
    }

    /// Current value of a `RandMemberTarget`.
    pub(super) fn read_rand_member(&self, t: &RandMemberTarget) -> Option<Value> {
        match t {
            RandMemberTarget::Agg(r) => Some(self.read_class_agg(r)),
            RandMemberTarget::Sub(h, prop) => self
                .heap
                .get(*h)
                .and_then(|o| o.as_ref())
                .and_then(|i| i.properties.get(prop))
                .cloned(),
        }
    }

    /// A key naming a member target for per-randomize bookkeeping
    /// (`dist_picked_once`): the constraint expression's source position.
    pub(super) fn rand_member_key(expr: &Expression) -> String {
        format!("#member@{}", expr.span.start)
    }

    /// Whether the class of the object `handle` (or an ancestor) declares a
    /// property `name`.
    pub(super) fn object_declares_property(&self, handle: usize, name: &str) -> bool {
        let mut cur = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone());
        let mut guard = 0;
        while let Some(cn) = cur {
            guard += 1;
            if guard > 64 {
                return false;
            }
            let base = cn.split('#').next().unwrap_or(&cn);
            let Some(cd) = self.module.classes.get(base) else {
                return false;
            };
            if cd.properties.contains_key(name) {
                return true;
            }
            cur = cd.extends.clone();
        }
        false
    }

    /// Write a `RandMemberTarget`; reports whether the stored value changed.
    pub(super) fn set_rand_member(&mut self, t: &RandMemberTarget, val: Value) -> bool {
        match t {
            RandMemberTarget::Agg(r) => self.write_class_agg(r, &val),
            RandMemberTarget::Sub(h, prop) => {
                let w = self
                    .class_prop_width_of(*h, prop)
                    .unwrap_or(val.width)
                    .max(1);
                self.set_prop_if_changed(*h, prop, val.resize(w))
            }
        }
    }

    /// Declared width of `prop` on the object `handle` points at (class chain).
    pub(super) fn class_prop_width_of(&self, handle: usize, prop: &str) -> Option<u32> {
        let cn = &self.heap.get(handle)?.as_ref()?.class_name;
        // The object's own specialization (§8.25): a member sized by a class
        // parameter or typed by a type parameter has THIS object's width.
        self.class_prop_width_impl(cn, Some(handle), prop)
    }

    /// §18.4: re-validate the constraints declared inside every `rand` object
    /// handle of `handle` against the current (post-fixpoint) assignment.
    pub(super) fn sub_object_constraints_ok(
        &mut self,
        handle: usize,
        obj_props: &[String],
    ) -> bool {
        for sub in self.rand_child_handles(handle, obj_props) {
            let Some(sub_class) = self
                .heap
                .get(sub)
                .and_then(|o| o.as_ref())
                .map(|i| i.class_name.clone())
            else {
                continue;
            };
            let mut items: Vec<ConstraintItem> = Vec::new();
            let mut seen: HashSet<String> = HashSet::default();
            let disabled = self
                .constraint_mode_disabled
                .get(&sub)
                .cloned()
                .unwrap_or_default();
            let mut cur = Some(sub_class);
            while let Some(cn) = cur {
                let Some(cd) = self.module.classes.get(&cn) else {
                    break;
                };
                for (name, con) in cd.constraints.iter() {
                    if !seen.insert(name.clone())
                        || disabled.contains("*")
                        || disabled.contains(name)
                        || (con.is_static
                            && self
                                .static_constraint_disabled
                                .contains(&(cn.clone(), name.clone())))
                    {
                        continue;
                    }
                    items.extend(con.items.iter().cloned());
                }
                cur = cd.extends.clone();
            }
            self.class_context_stack.push(
                self.heap
                    .get(sub)
                    .and_then(|o| o.as_ref())
                    .map(|i| i.class_name.clone()),
            );
            let mut ok = true;
            for item in &items {
                if Self::constraint_unmodeled(item) {
                    continue;
                }
                if !self.check_constraint_item(sub, item) {
                    ok = false;
                    break;
                }
            }
            self.class_context_stack.pop();
            if !ok {
                return false;
            }
        }
        true
    }

    /// §18.3: whether property `prop` of the object `handle` points at is
    /// declared SIGNED (`rand integer`, `rand int`, `rand byte`, …).
    pub(super) fn class_prop_signed_of(&self, handle: usize, prop: &str) -> bool {
        let Some(Some(inst)) = self.heap.get(handle) else {
            return false;
        };
        let storage_prop = prop;
        let (start, prop) = prop
            .rsplit_once("::")
            .unwrap_or((inst.class_name.as_str(), prop));
        let mut cur = Some(start);
        while let Some(cn) = cur {
            let Some(cd) = self.module.classes.get(cn) else {
                return false;
            };
            if let Some(sig) = cd.properties.get(prop) {
                // §6.20.2: a property typed by a TYPE PARAMETER takes the
                // signedness of the type bound on this object (`byte` for
                // `comp_base #(byte)`), not of the parameter's default.
                if sig
                    .type_name
                    .as_ref()
                    .is_some_and(|t| cd.type_param_names.contains(t))
                {
                    if let Some(s) = self
                        .class_prop_type_name(handle, storage_prop)
                        .and_then(|c| self.concrete_type_signed(&c))
                    {
                        return s;
                    }
                }
                return sig.is_signed;
            }
            cur = cd.extends.as_deref();
        }
        false
    }

    /// Element width of the fixed-array / queue member `prop` declared by
    /// `cdef`, in the specialization the object `handle` was built as, when
    /// that depends on a class parameter: a packed range sized by a value
    /// parameter, or a type parameter (§8.25, §6.20.2). `None` keeps the
    /// class table's width.
    pub(super) fn spec_member_elem_width(
        &self,
        cdef: &crate::compiler::elaborate::ElaboratedClass,
        handle: usize,
        prop: &str,
    ) -> Option<u32> {
        let sig = cdef.properties.get(prop)?;
        if sig
            .type_name
            .as_ref()
            .is_some_and(|t| cdef.type_param_names.contains(t))
        {
            let concrete = self.class_prop_type_name(handle, prop)?;
            return self.concrete_type_width(&concrete);
        }
        if Self::agg_names_type_param(cdef, prop) {
            return self.class_prop_width_impl(&cdef.name, Some(handle), prop);
        }
        self.respec_packed_width(&cdef.name, handle, prop)
    }

    /// `[i][j]...` suffixes of every element of a fixed N-D shape (empty
    /// for an oversized one, which construction leaves lazy too).
    pub(super) fn nd_index_suffixes(shape: &[(i64, i64)]) -> Vec<String> {
        let total: i64 = shape.iter().map(|&(lo, hi)| (hi - lo + 1).max(0)).product();
        if shape.is_empty() || total <= 0 || total > 65536 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(total as usize);
        let mut idx: Vec<i64> = shape.iter().map(|d| d.0).collect();
        loop {
            let mut sfx = String::new();
            for v in &idx {
                sfx.push_str(&format!("[{}]", v));
            }
            out.push(sfx);
            let mut k = shape.len();
            loop {
                if k == 0 {
                    return out;
                }
                k -= 1;
                idx[k] += 1;
                if idx[k] <= shape[k].1 {
                    break;
                }
                idx[k] = shape[k].0;
            }
        }
    }

    /// Re-register the per-instance storage of collection members whose
    /// element width depends on the object's parameters — see the call in
    /// `instantiate_class_with_type_args`.
    /// §8.25: the shape of the param-sized fixed-array member `dims` for an
    /// object under construction, from the parameter values it binds (leaf
    /// first, so a derived class's parameter shadows an ancestor's), falling
    /// back to the design's parameters for any other name. `None` when a bound
    /// does not fold.
    #[inline(never)]
    pub(super) fn param_sized_shape(
        &self,
        dims: &[crate::ast::types::UnpackedDimension],
        instance: &ClassInstance,
        chain: &[std::sync::Arc<crate::compiler::elaborate::ElaboratedClass>],
    ) -> Option<Vec<(i64, i64)>> {
        use crate::ast::types::UnpackedDimension as UD;
        let mut m: HashMap<String, Value> = HashMap::default();
        for c in chain {
            for (pn, _) in &c.param_defaults {
                if let Some(v) = instance.properties.get(pn) {
                    m.entry(pn.clone()).or_insert_with(|| v.clone());
                }
            }
        }
        let mut full: Option<HashMap<String, Value>> = None;
        let mut ev = |e: &Expression| -> Option<i64> {
            if let Some(v) = super::elaborate::const_eval_i64_with_params(e, Some(&m)) {
                return Some(v);
            }
            let f = full.get_or_insert_with(|| {
                let mut f = self.module.parameters.clone();
                f.extend(m.iter().map(|(k, v)| (k.clone(), v.clone())));
                f
            });
            super::elaborate::const_eval_i64_with_params(e, Some(f))
        };
        dims.iter()
            .map(|d| match d {
                UD::Range { left, right, .. } => {
                    let (l, r) = (ev(left)?, ev(right)?);
                    Some((l.min(r), l.max(r)))
                }
                UD::Expression { expr, .. } => {
                    let n = ev(expr)?;
                    (n > 0).then_some((0, n - 1))
                }
                _ => None,
            })
            .collect()
    }

    /// §8.25: packed width of the key type `dt` under the parameter
    /// bindings `instance` carries; `None` when a range does not evaluate.
    pub(super) fn param_sized_key_width(
        &self,
        dt: &DataType,
        instance: &ClassInstance,
        chain: &[std::sync::Arc<crate::compiler::elaborate::ElaboratedClass>],
    ) -> Option<u32> {
        let (DataType::IntegerVector { dimensions, .. } | DataType::Implicit { dimensions, .. }) = dt
        else {
            return None;
        };
        let mut m = self.module.parameters.clone();
        let mut bound: HashSet<&str> = HashSet::default();
        for c in chain {
            for (pn, _) in &c.param_defaults {
                if let Some(v) = instance.properties.get(pn)
                    && bound.insert(pn.as_str())
                {
                    m.insert(pn.clone(), v.clone());
                }
            }
        }
        let mut w: u64 = 1;
        for d in dimensions {
            let crate::ast::types::PackedDimension::Range { left, right, .. } = d else {
                return None;
            };
            let l = super::elaborate::const_eval_i64_with_params(left, Some(&m))?;
            let r = super::elaborate::const_eval_i64_with_params(right, Some(&m))?;
            w = w.checked_mul(l.abs_diff(r) + 1)?;
        }
        u32::try_from(w).ok().filter(|&w| w > 0)
    }

    /// §8.25: bounds of the 1-D fixed-array member `prop` of the object
    /// `handle`. A member sized by a class value parameter has its own size
    /// per object (registered at construction); everything else has the
    /// class's `default`.
    pub(super) fn inst_fixed_bounds(
        &self,
        cdef: &crate::compiler::elaborate::ElaboratedClass,
        handle: usize,
        prop: &str,
        default: (i64, i64),
    ) -> (i64, i64) {
        if cdef.param_sized_props.is_empty() || !cdef.param_sized_props.contains_key(prop) {
            return default;
        }
        self.module
            .arrays
            .get(format!("{}#{}", handle, prop).as_str())
            .map(|&(lo, hi, _)| (lo, hi))
            .unwrap_or(default)
    }

    /// §8.25: the per-object shape of a param-sized multi-dimensional member
    /// (see `inst_fixed_bounds`); `None` for any other member.
    pub(super) fn inst_nd_shape(
        &self,
        cdef: &crate::compiler::elaborate::ElaboratedClass,
        handle: usize,
        prop: &str,
    ) -> Option<Vec<(i64, i64)>> {
        if cdef.param_sized_props.is_empty() || !cdef.param_sized_props.contains_key(prop) {
            return None;
        }
        let scoped = format!("{}#{}", handle, prop);
        if let Some(&(a, b, _)) = self.module.arrays_2d.get(scoped.as_str()) {
            return Some(vec![a, b]);
        }
        self.module
            .arrays_nd
            .get(scoped.as_str())
            .map(|(sh, _)| sh.clone())
    }

    pub(super) fn respec_member_elem_widths(
        &mut self,
        handle: usize,
        chain: &[std::sync::Arc<crate::compiler::elaborate::ElaboratedClass>],
    ) {
        for cdef in chain {
            // §8.25: the default specialization and an explicit binding can
            // have equal widths but different state domains (int/integer).
            for (prop, sig) in &cdef.properties {
                let agg = Self::agg_names_type_param(cdef, prop);
                if !(agg
                    || sig
                        .type_name
                        .as_ref()
                        .is_some_and(|t| cdef.type_param_names.contains(t)))
                    || cdef.static_properties.contains(prop)
                    || cdef.property_inits.contains_key(prop)
                    || cdef.array_properties.contains_key(prop)
                    || cdef.array_nd_properties.contains_key(prop)
                    || cdef.queue_properties.contains_key(prop)
                    || cdef.assoc_properties.contains_key(prop)
                {
                    continue;
                }
                // §8.25/§6.8: a packed struct or enum naming a type parameter
                // starts at this object's width, x unless 2-state.
                if agg {
                    let Some(w) = self.class_prop_width_impl(&cdef.name, Some(handle), prop) else {
                        continue;
                    };
                    let two_state = cdef.property_types.get(prop).is_some_and(|dt| {
                        let b = self.bind_type_params_in(dt, &|n| {
                            self.obj_type_param(handle, &cdef.name, n)
                        });
                        super::elaborate::is_type_two_state_resolved(
                            b.as_ref().unwrap_or(dt),
                            &self.module.typedef_types,
                        )
                    });
                    let mut v = if two_state {
                        Value::zero(w)
                    } else {
                        Value::new(w)
                    };
                    v.is_signed = sig.is_signed;
                    if let Some(Some(inst)) = self.heap.get_mut(handle) {
                        inst.properties.insert(prop.clone(), v);
                    }
                    continue;
                }
                if let Some(concrete) = self.class_prop_type_name(handle, prop)
                    && let Some(starts_x) = self.concrete_type_starts_x(&concrete)
                {
                    let w = self
                        .class_prop_width_impl(&cdef.name, Some(handle), prop)
                        .unwrap_or(sig.width)
                        .max(1);
                    let mut v = if starts_x {
                        Value::new(w)
                    } else {
                        Value::zero(w)
                    };
                    v.is_signed = self.class_prop_signed_of(handle, prop);
                    if let Some(Some(inst)) = self.heap.get_mut(handle) {
                        inst.properties.insert(prop.clone(), v);
                    }
                } else if let Some(su) = self
                    .class_prop_struct(handle, prop)
                    .filter(Self::spreads_member_wise)
                {
                    self.seed_class_struct_cells(handle, prop, &su);
                }
            }
            for (prop, &(lo, hi, _)) in &cdef.array_properties {
                let Some(w) = self.spec_member_elem_width(cdef, handle, prop) else {
                    continue;
                };
                if w == 0 {
                    continue;
                }
                let (lo, hi) = self.inst_fixed_bounds(cdef, handle, prop, (lo, hi));
                let scoped = format!("{}#{}", handle, prop);
                self.module.arrays.insert(scoped.clone(), (lo, hi, w));
                // §6.8: x for a 4-state element type.
                let fill = self.class_bound_array_fill(cdef, handle, prop, w);
                let su = self
                    .class_prop_struct(handle, prop)
                    .filter(Self::spreads_member_wise);
                for i in lo..=hi {
                    let en = format!("{}[{}]", scoped, i);
                    self.signals.insert(en.clone(), fill.clone());
                    self.widths.insert(en, w);
                    if let Some(su) = &su {
                        self.seed_class_struct_cells(handle, &format!("{}[{}]", prop, i), su);
                    }
                }
            }
            for (prop, (shape, _)) in &cdef.array_nd_properties {
                let Some(w) = self.spec_member_elem_width(cdef, handle, prop) else {
                    continue;
                };
                if w == 0 {
                    continue;
                }
                let own = self.inst_nd_shape(cdef, handle, prop);
                let shape = own.as_ref().unwrap_or(shape);
                let scoped = format!("{}#{}", handle, prop);
                if let Some(e) = self.module.arrays_2d.get_mut(&scoped) {
                    e.2 = w;
                }
                if let Some(e) = self.module.arrays_nd.get_mut(&scoped) {
                    e.1 = w;
                }
                let fill = self.class_bound_array_fill(cdef, handle, prop, w);
                let su = self
                    .class_prop_struct(handle, prop)
                    .filter(Self::spreads_member_wise);
                for suffix in Self::nd_index_suffixes(shape) {
                    let en = format!("{}{}", scoped, suffix);
                    if self.widths.contains_key(&en) {
                        self.signals.insert(en.clone(), fill.clone());
                        self.widths.insert(en, w);
                        if let Some(su) = &su {
                            self.seed_class_struct_cells(
                                handle,
                                &format!("{}{}", prop, suffix),
                                su,
                            );
                        }
                    }
                }
            }
            for (prop, &(width, _)) in &cdef.queue_properties {
                let Some(w) = self.spec_member_elem_width(cdef, handle, prop) else {
                    continue;
                };
                if w == width || w == 0 {
                    continue;
                }
                let scoped = format!("{}#{}", handle, prop);
                if let Some(e) = self.module.arrays.get_mut(&scoped) {
                    e.2 = w;
                }
            }
        }
    }

    /// Declared width (and signedness) of the static property cell `key`
    /// (`Decl::prop` or `Decl#<sig>::prop`), in the specialization the key
    /// names (§8.25: each specialization has its own statics). A packed range
    /// sized by a value parameter resolves against the signature's arguments;
    /// a type-parameter type takes the bound type. `None` for a real, string,
    /// class handle or anything not resolvable, which keeps the value as is.
    pub(super) fn static_cell_width(&self, key: &str, prop: &str) -> Option<(u32, bool)> {
        if let Some(hit) = self.static_cell_width_cache.borrow().get(key) {
            return *hit;
        }
        let out = self.static_cell_width_uncached(key, prop);
        self.static_cell_width_cache
            .borrow_mut()
            .insert(key.to_string(), out);
        out
    }

    pub(super) fn static_cell_width_uncached(&self, key: &str, prop: &str) -> Option<(u32, bool)> {
        let head = key.rsplit_once("::").map(|(h, _)| h).unwrap_or(key);
        let (decl, sig) = match head.split_once('#') {
            Some((d, s)) => (d, Some(s)),
            None => (head, None),
        };
        let cd = self.module.classes.get(decl)?;
        let ps = cd.properties.get(prop)?;
        if ps.is_real
            || cd.string_properties.contains(prop)
            || ps
                .type_name
                .as_ref()
                .is_some_and(|t| self.module.classes.contains_key(t))
            || cd.queue_properties.contains_key(prop)
            || cd.assoc_properties.contains_key(prop)
            || cd.array_properties.contains_key(prop)
            || cd.array_nd_properties.contains_key(prop)
        {
            return None;
        }
        let args: Vec<String> = sig.map(Self::split_spec_args).unwrap_or_default();
        let arg_of = |name: &str| -> Option<String> {
            let i = cd.param_order.iter().position(|p| p == name)?;
            args.get(i)
                .map(|a| a.trim().to_string())
                .filter(|a| !a.is_empty())
        };
        if let Some(tn) = ps
            .type_name
            .as_ref()
            .filter(|t| cd.type_param_names.contains(t))
        {
            let concrete = arg_of(tn).or_else(|| {
                cd.type_param_defaults
                    .iter()
                    .find(|(n, _)| n == tn)
                    .map(|(_, d)| d.trim().to_string())
            })?;
            let w = self.concrete_type_width(&concrete)?;
            return Some((w, self.concrete_type_signed(&concrete).unwrap_or(false)));
        }
        let pdt = self.class_prop_decl_type(cd, prop);
        let dims = match pdt {
            Some(DataType::IntegerVector { dimensions, .. }) if !dimensions.is_empty() => {
                dimensions
            }
            Some(DataType::Implicit { dimensions, .. }) if !dimensions.is_empty() => dimensions,
            // Only plain integral declarations are sized here; a typedef,
            // struct, enum, interface or container handle keeps its value.
            Some(DataType::IntegerVector { .. } | DataType::IntegerAtom { .. }) => {
                return Some((ps.width, ps.is_signed)).filter(|(w, _)| *w > 0);
            }
            _ => return None,
        };
        // Value parameters in declaration order: the signature's argument,
        // else the default evaluated against the parameters before it.
        let mut params: HashMap<String, Value> = HashMap::default();
        for (pname, pdefault) in &cd.param_defaults {
            let v = match arg_of(pname).and_then(|a| Self::parse_spec_number(&a)) {
                Some(lit) => {
                    let e = Expression::new(ExprKind::Number(lit), crate::ast::Span::dummy());
                    crate::elaborate::const_eval_i64_with_params(&e, None)
                }
                None => pdefault
                    .as_ref()
                    .and_then(|e| crate::elaborate::const_eval_i64_with_params(e, Some(&params))),
            };
            if let Some(v) = v {
                params.insert(pname.clone(), Value::from_u64(v as u64, 32));
            }
        }
        let all_resolved = dims.iter().all(|d| match d {
            crate::ast::types::PackedDimension::Range { left, right, .. } => {
                crate::elaborate::const_eval_i64_with_params(left, Some(&params)).is_some()
                    && crate::elaborate::const_eval_i64_with_params(right, Some(&params)).is_some()
            }
            _ => true,
        });
        if !all_resolved {
            return Some((ps.width, ps.is_signed)).filter(|(w, _)| *w > 0);
        }
        let dt = pdt?;
        let w = crate::elaborate::resolve_type_width(dt, Some(&params), None);
        Some((w, ps.is_signed)).filter(|(w, _)| *w > 0)
    }
}
