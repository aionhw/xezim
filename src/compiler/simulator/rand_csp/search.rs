//! Building the problem (scope, variables, translation) and the randomized search.

use super::*;

impl Simulator {
    /// The scope solve: with `sized` empty a full solve, written to the
    /// caller's variables (Ok with no sizes); otherwise the size phase.
    #[allow(clippy::type_complexity)]
    pub(super) fn csp_scope(
        &mut self,
        items: &[ConstraintItem],
        scalars: &[(String, Expression, u32, bool, Option<Vec<u64>>)],
        arrays: &[(String, String, Vec<i64>, u32, bool, Option<Vec<u64>>)],
        sized: &HashSet<String>,
    ) -> Result<Vec<(String, u64)>, CspOutcome> {
        let mut csp = Csp {
            handle: usize::MAX,
            vars: Vec::new(),
            dom0: Vec::new(),
            scalars: HashMap::default(),
            arrays: HashMap::default(),
            nd_dims: HashMap::default(),
            nd_dyn: HashMap::default(),
            srcs: Vec::new(),
            nodes: Vec::new(),
            soft_enabled: None,
            soft_priority: Vec::new(),
            soft_rank: (0, 0),
            dists: Vec::new(),
            order: Vec::new(),
            opaque: false,
            has_subs: false,
            sub_prefixes: HashMap::default(),
            dotted: false,
            scope: Some(HashMap::default()),
            segs: HashMap::default(),
            slice_vars: HashMap::default(),
            aux_base: HashMap::default(),
            fields: HashMap::default(),
            signed_fields: HashSet::default(),
            field_elems: HashMap::default(),
            elem_fields: HashMap::default(),
            pdims: HashMap::default(),
            lsb: HashMap::default(),
            ascending: HashSet::default(),
            wide: HashSet::default(),
            wide_seg: HashSet::default(),
            wide_free: Vec::new(),
            draw_free: false,
            wide_fail: false,
            pop: HashMap::default(),
            pop_rev: HashMap::default(),
            carries: Vec::new(),
            carry_accs: Vec::new(),
            wraps: Vec::new(),
            carry_base: None,
            sizes: HashMap::default(),
            size_phase: false,
            masks: Vec::new(),
        };
        let dom_of = |w: u32, s: bool, members: &Option<Vec<u64>>| -> Dom {
            match members {
                Some(m) if !m.is_empty() => {
                    dom_norm(m.iter().map(|&v| (v as i128, v as i128)).collect())
                }
                _ => {
                    let (lo, hi) = ws_range(w, s);
                    vec![(lo, hi)]
                }
            }
        };
        for (name, key, idx, w, s, members) in arrays {
            if sized.contains(name) {
                let sv = csp.vars.len();
                csp.vars.push(CspVar {
                    key: VarKey::Aux,
                    width: 32,
                    signed: true,
                });
                csp.dom0.push(vec![(0, SIZE_TOP)]);
                csp.sizes.insert(name.clone(), sv);
                csp.arrays.insert(
                    name.clone(),
                    CspArr {
                        elems: Vec::new(),
                        width: *w,
                        signed: *s,
                    },
                );
                continue;
            }
            if *w > 64 && !sized.is_empty() {
                // Not a variable of the size phase.
                continue;
            }
            if *w == 0 || (*w > 64 && members.is_some()) {
                return Err(CspOutcome::NotApplicable);
            }
            let wide = *w > 64;
            let dom = if wide {
                vec![(0, 0)]
            } else {
                dom_of(*w, *s, members)
            };
            let mut elems = Vec::with_capacity(idx.len());
            for &i in idx {
                if wide {
                    csp.wide.insert(csp.vars.len());
                }
                elems.push((i, csp.vars.len()));
                csp.vars.push(CspVar {
                    key: VarKey::Elem(format!("{}[{}]", key, i)),
                    width: *w,
                    signed: *s,
                });
                csp.dom0.push(dom.clone());
            }
            csp.arrays.insert(
                name.clone(),
                CspArr {
                    elems,
                    width: *w,
                    signed: *s,
                },
            );
        }
        csp.nd_dims = self.rand_scope_nd.clone();
        csp.nd_dyn = self.rand_scope_nd_dyn.clone();
        for (name, lv, w, s, members) in scalars {
            if *w == 0 || csp.arrays.contains_key(name) {
                return Err(CspOutcome::NotApplicable);
            }
            // §18.5: a local wider than 64 bits is solved through segments
            // of at most 64 bits, as a class member is (`csp_wide_split`).
            let wide = *w > 64;
            if wide && (members.is_some() || !sized.is_empty()) {
                // Not modelled: left out of the solve, as before.
                continue;
            }
            if wide {
                csp.wide.insert(csp.vars.len());
            }
            csp.scalars.insert(name.clone(), csp.vars.len());
            csp.vars.push(CspVar {
                key: VarKey::Prop(name.clone()),
                width: *w,
                signed: *s,
            });
            csp.dom0.push(if wide {
                vec![(0, 0)]
            } else {
                dom_of(*w, *s, members)
            });
            if let Some(l) = csp.scope.as_mut() {
                l.insert(name.clone(), lv.clone());
            }
        }
        if csp.vars.is_empty() {
            return Err(CspOutcome::NotApplicable);
        }
        csp.size_phase = !sized.is_empty();
        if std::env::var_os("XEZIM_RAND_DBG").is_some() {
            eprintln!(
                "[rand-dbg] scope solve: arrays {:?} vars {:?} nd {:?}",
                csp.arrays
                    .iter()
                    .map(|(n, a)| (n.clone(), a.elems.len(), a.width, a.signed))
                    .collect::<Vec<_>>(),
                csp.vars
                    .iter()
                    .map(|v| format!("{:?}", v.key))
                    .collect::<Vec<_>>(),
                csp.nd_dims
            );
        }
        let constraints = vec![ClassConstraint {
            is_static: false,
            is_extern: false,
            has_body: true,
            name: crate::ast::Identifier {
                name: "__inline__".to_string(),
                span: crate::ast::Span::dummy(),
            },
            items: items.to_vec(),
            span: crate::ast::Span::dummy(),
        }];
        let prev = CSP_SCOPE.with(|c| c.replace(true));
        let prev_receiver = self.rand_receiver.take();
        self.csp_split_slices(&mut csp, &constraints);
        let out = if csp.wide_fail || self.csp_translate(&mut csp, &constraints, &[]).is_none() {
            Err(CspOutcome::NotApplicable)
        } else if sized.is_empty() {
            match self.csp_run(&csp, &constraints, &[]) {
                CspOutcome::Sat => Ok(Vec::new()),
                o => Err(o),
            }
        } else {
            let mut found = self.csp_find(&csp);
            if matches!(found, Err(CspOutcome::Unsat)) && !csp.soft_priority.is_empty() {
                // §18.5.14: sizes that follow the hard constraints alone.
                csp.soft_enabled = Some(HashSet::default());
                found = match self.csp_translate(&mut csp, &constraints, &[]) {
                    Some(()) => self.csp_find(&csp),
                    None => Err(CspOutcome::NotApplicable),
                };
            }
            found.and_then(|st| {
                csp.sizes
                    .iter()
                    .map(|(n, &sv)| {
                        let k = st.fixed(sv).ok_or(CspOutcome::GaveUp)?;
                        Ok((n.clone(), k.max(0) as u64))
                    })
                    .collect()
            })
        };
        self.rand_receiver = prev_receiver;
        CSP_SCOPE.with(|c| c.set(prev));
        out
    }

    /// Build the variable table: rand scalars and array elements.
    pub(super) fn csp_vars(
        &mut self,
        handle: usize,
        rand_props: &[(String, u32)],
        signed_props: &HashSet<String>,
        enum_props: &HashMap<String, String>,
        colls: &[RandColl],
        array_enums: &HashMap<String, String>,
        unread_wide: &HashSet<String>,
        // Rand sub-objects join later (`csp_add_sub`): the object needs no
        // scalar of its own.
        obj_elems: bool,
        // The dynamic arrays whose size is a variable (`csp_size_phase`).
        sized: &HashSet<String>,
    ) -> Option<Csp> {
        let mut csp = Csp {
            handle,
            vars: Vec::new(),
            dom0: Vec::new(),
            scalars: HashMap::default(),
            arrays: HashMap::default(),
            nd_dims: HashMap::default(),
            nd_dyn: HashMap::default(),
            srcs: Vec::new(),
            nodes: Vec::new(),
            soft_enabled: None,
            soft_priority: Vec::new(),
            soft_rank: (0, 0),
            dists: Vec::new(),
            order: Vec::new(),
            opaque: false,
            has_subs: false,
            sub_prefixes: HashMap::default(),
            dotted: false,
            scope: None,
            segs: HashMap::default(),
            slice_vars: HashMap::default(),
            aux_base: HashMap::default(),
            fields: HashMap::default(),
            signed_fields: HashSet::default(),
            field_elems: HashMap::default(),
            elem_fields: HashMap::default(),
            pdims: HashMap::default(),
            lsb: HashMap::default(),
            ascending: HashSet::default(),
            wide: HashSet::default(),
            wide_seg: HashSet::default(),
            wide_free: Vec::new(),
            draw_free: false,
            wide_fail: false,
            pop: HashMap::default(),
            pop_rev: HashMap::default(),
            carries: Vec::new(),
            carry_accs: Vec::new(),
            wraps: Vec::new(),
            carry_base: None,
            sizes: HashMap::default(),
            size_phase: false,
            masks: Vec::new(),
        };
        let enum_dom = |me: &Self, tn: &str| -> Option<Dom> {
            let members = me.module.enum_members.get(tn)?;
            if members.is_empty() {
                return None;
            }
            Some(dom_norm(
                members.iter().map(|m| (m.1 as i128, m.1 as i128)).collect(),
            ))
        };
        for c in colls {
            if let Some((base, dims)) = &c.nd_base {
                csp.nd_dims.insert(base.clone(), dims.clone());
            }
            // §7.4.5: a member whose elements are rows contributes its shape
            // (and its size, while sizes are solved); the rows carry the
            // values.
            if c.row_tail.is_some() {
                csp.nd_dims
                    .entry(c.prop.clone())
                    .or_insert_with(|| self.csp_row_owner_shape(c));
                csp.nd_dyn.insert(
                    c.prop.clone(),
                    if matches!(c.row_tail, Some(RowTail::Dyn)) {
                        2
                    } else {
                        1
                    },
                );
                if !sized.contains(&c.prop) {
                    continue;
                }
            }
            // §18.5.9: the objects of an array of rand handles are no
            // values; they join as sub-objects (`csp_add_sub`).
            if c.is_object_elem && !c.nested && c.kind != CollKind::Assoc {
                continue;
            }
            if c.is_object_elem || (c.nested && (c.row_tail.is_none() || c.kind == CollKind::Assoc))
            {
                return None;
            }
            if sized.contains(&c.prop) {
                // §18.5.8.1: the size is solved before any element exists.
                let sv = csp.vars.len();
                csp.vars.push(CspVar {
                    key: VarKey::Aux,
                    width: 32,
                    signed: true,
                });
                csp.dom0.push(vec![(0, SIZE_TOP)]);
                csp.sizes.insert(c.prop.clone(), sv);
                csp.arrays.insert(
                    c.prop.clone(),
                    CspArr {
                        elems: Vec::new(),
                        width: c.width,
                        signed: false,
                    },
                );
                continue;
            }
            let idx: Vec<i64> = match c.kind {
                CollKind::Fixed => (c.lo..=c.hi).collect(),
                // §18.4: randomize() keeps an associative array's keys and
                // draws a value for each element, so with integral keys its
                // elements are variables indexed by the stored keys.
                // One keyed by strings (or by integers the solver does not
                // index) stays with the trial's draw; its own items are then
                // judged by the evaluator (`csp_eval_node`), so the rest of
                // the class is still solved jointly.
                CollKind::Assoc => {
                    if self.is_string_keyed_array(&c.scoped) {
                        continue;
                    }
                    let mut keys = Vec::new();
                    let mut indexed = true;
                    for k in self.assoc_key_strs(&c.scoped) {
                        // A `foreach` key is bound as a signed 32-bit value.
                        match k.parse::<i32>() {
                            Ok(i) if i.to_string() == k => keys.push(i.into()),
                            _ => indexed = false,
                        }
                    }
                    if !indexed {
                        continue;
                    }
                    keys
                }
                CollKind::Dyn => (0..self.get_queue_size(&c.scoped) as i64).collect(),
            };
            let wide = c.width > 64;
            if wide && unread_wide.contains(&c.prop) {
                csp.wide_free.extend(
                    idx.iter()
                        .map(|i| (VarKey::Elem(c.elem_key(i)), c.width)),
                );
                continue;
            }
            let enum_type = array_enums.get(&c.prop);
            if wide && enum_type.is_some() {
                return None;
            }
            let signed = match &c.member {
                Some(m) => m.signed,
                None => self.class_prop_signed_of(handle, &c.prop),
            };
            let dom = if wide {
                // A placeholder: the elements are solved as segments.
                vec![(0, 0)]
            } else {
                enum_type
                    .and_then(|tn| enum_dom(self, tn))
                    .unwrap_or_else(|| {
                        let (lo, hi) = ws_range(c.width, signed);
                        vec![(lo, hi)]
                    })
            };
            let mut elems = Vec::with_capacity(idx.len());
            for i in idx {
                if wide {
                    csp.wide.insert(csp.vars.len());
                }
                elems.push((i, csp.vars.len()));
                csp.vars.push(CspVar {
                    key: VarKey::Elem(c.elem_key(i)),
                    width: c.width,
                    signed,
                });
                csp.dom0.push(dom.clone());
            }
            csp.arrays.insert(
                c.prop.clone(),
                CspArr {
                    elems,
                    width: c.width,
                    signed,
                },
            );
        }
        for (name, w) in rand_props {
            if csp.arrays.contains_key(name) || csp.nd_dims.contains_key(name) {
                continue;
            }
            if *w == 0 {
                return None;
            }
            let signed = signed_props.contains(name);
            let dom = if *w > 64 {
                if unread_wide.contains(name) {
                    csp.wide_free.push((VarKey::Prop(name.clone()), *w));
                    continue;
                }
                if enum_props.contains_key(name) {
                    return None;
                }
                csp.wide.insert(csp.vars.len());
                vec![(0, 0)]
            } else {
                enum_props
                    .get(name)
                    .and_then(|tn| enum_dom(self, tn))
                    .unwrap_or_else(|| {
                        let (lo, hi) = ws_range(*w, signed);
                        vec![(lo, hi)]
                    })
            };
            csp.scalars.insert(name.clone(), csp.vars.len());
            csp.vars.push(CspVar {
                key: VarKey::Prop(name.clone()),
                width: *w,
                signed,
            });
            csp.dom0.push(dom);
        }
        // An array sized 0 (by the size phase or by the caller) is a
        // problem without variables, still one to judge.
        if csp.vars.is_empty() && !obj_elems && csp.arrays.is_empty() {
            return None;
        }
        csp.dotted = csp.scalars.keys().any(|k| k.contains('.'));
        Some(csp)
    }

    pub(super) fn csp_translate(
        &mut self,
        csp: &mut Csp,
        constraints: &[ClassConstraint],
        depths: &[usize],
    ) -> Option<()> {
        csp.srcs.clear();
        csp.nodes.clear();
        csp.dists.clear();
        csp.order.clear();
        csp.soft_priority.clear();
        csp.opaque = false;
        csp.carries.clear();
        csp.carry_accs.clear();
        csp.wraps.clear();
        csp.masks.clear();
        match csp.carry_base {
            Some(b) => {
                csp.vars.truncate(b);
                csp.dom0.truncate(b);
            }
            None => csp.carry_base = Some(csp.vars.len()),
        }
        let env = Env::default();
        self.csp_slice_links(csp);
        // §18.5: establish unconditional scalar bounds before dependent arithmetic.
        // Keep soft, conditional and foreach bounds in their original scope.
        fn leaves<'a>(items: &'a [ConstraintItem], out: &mut Vec<&'a ConstraintItem>) {
            for item in items {
                match item {
                    ConstraintItem::Block(items) => leaves(items, out),
                    item => out.push(item),
                }
            }
        }
        let mut ordered = Vec::new();
        for (index, con) in constraints.iter().enumerate() {
            let mut items = Vec::new();
            leaves(&con.items, &mut items);
            for item in items {
                ordered.push((index, item, self.csp_unconditional_bound(csp, item, &env)));
            }
        }
        let mut dist_order = Vec::new();
        for (position, (index, it, _)) in ordered
            .iter()
            .enumerate()
            .filter(|(_, (_, _, bound))| *bound)
            .chain(
                ordered
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, _, bound))| !*bound),
            )
        {
            let con = &constraints[*index];
            csp.soft_rank = if con.name.name == "__inline__" {
                (0, usize::MAX)
            } else {
                (
                    depths
                        .get(*index)
                        .copied()
                        .unwrap_or(usize::MAX - 1)
                        .saturating_add(1),
                    con.span.start as usize,
                )
            };
            let dist_start = csp.dists.len();
            let Some(n) = self.csp_item(csp, it, &env) else {
                if std::env::var_os("XEZIM_RAND_DBG").is_some() {
                    eprintln!("[rand-dbg] joint solve does not model: {:?}", it);
                }
                return None;
            };
            dist_order.extend(std::iter::repeat_n(position, csp.dists.len() - dist_start));
            if !matches!(n, Node::True) {
                csp.nodes.push(n);
            }
        }
        // Sampling uses the first active distribution. Keep its original
        // registration order even when its hard membership bound moved first.
        let mut dists: Vec<_> = dist_order
            .into_iter()
            .zip(std::mem::take(&mut csp.dists))
            .collect();
        dists.sort_by_key(|(position, _)| *position);
        csp.dists = dists.into_iter().map(|(_, dist)| dist).collect();
        // §18.5: top-level conjunctions can be watched independently.
        // Rechecking a whole foreach after each element decision otherwise
        // repeats every element's constraint for every decision.
        fn separate(n: Node, out: &mut Vec<Node>) {
            match n {
                Node::And(nodes) => {
                    for n in nodes {
                        separate(n, out);
                    }
                }
                Node::True => {}
                n => out.push(n),
            }
        }
        let nodes = std::mem::take(&mut csp.nodes);
        for n in nodes {
            separate(n, &mut csp.nodes);
        }
        Some(())
    }

    /// A simple, hard bound that can be translated without bounds from other
    /// constraints. Only called for leaves of the unconditional conjunction.
    fn csp_unconditional_bound(&self, csp: &Csp, item: &ConstraintItem, env: &Env) -> bool {
        let scalar = |e: &Expression| {
            self.csp_member(csp, Self::unparen(e))
                .is_some_and(|name| csp.scalars.contains_key(&name))
        };
        let free = |e: &Expression| {
            // A solver-free call can still consume randomness or have other
            // effects. Keep its evaluation in the ordinary translation pass.
            let mut calls = false;
            Self::walk_operands(e, &mut |e| {
                calls |= matches!(
                    e.kind,
                    ExprKind::Call { .. }
                        | ExprKind::SystemCall { .. }
                        | ExprKind::WithClause { .. }
                        | ExprKind::Unary {
                            op: UnaryOp::PreIncr
                                | UnaryOp::PreDecr
                                | UnaryOp::PostIncr
                                | UnaryOp::PostDecr,
                            ..
                        }
                );
            });
            !calls && self.csp_free(csp, e, env)
        };
        match item {
            ConstraintItem::Expr(e) => match &Self::unparen(e).kind {
                ExprKind::Binary {
                    op: BinaryOp::Eq | BinaryOp::Lt | BinaryOp::Leq | BinaryOp::Gt | BinaryOp::Geq,
                    left,
                    right,
                } => (scalar(left) && free(right)) || (free(left) && scalar(right)),
                ExprKind::Inside { expr, ranges } => scalar(expr) && ranges.iter().all(free),
                _ => false,
            },
            ConstraintItem::Inside {
                expr,
                range,
                dist_weights,
                ..
            } => {
                scalar(expr)
                    && range.iter().all(|r| match r {
                        ConstraintRange::Value(e) => free(e),
                        ConstraintRange::Range { lo, hi } => free(lo) && free(hi),
                    })
                    && dist_weights.iter().all(|w| match w {
                        Some(DistWeight::Each(e) | DistWeight::Total(e)) => free(e),
                        None => true,
                    })
            }
            _ => false,
        }
    }

    /// Search with restarts, then write and verify a solution.
    pub(super) fn csp_run(
        &mut self,
        csp: &Csp,
        constraints: &[ClassConstraint],
        colls: &[RandColl],
    ) -> CspOutcome {
        let st = match self.csp_find(csp) {
            Ok(st) => st,
            Err(o) => return o,
        };
        let nvars = csp.vars.len();
        for v in 0..nvars {
            if csp.wide.contains(&v) {
                // Written through its segments.
                continue;
            }
            let x = st.fixed(v).unwrap_or(0);
            self.csp_write(csp, v, x);
        }
        let accepted = if csp.scope.is_some() {
            let items: Vec<ConstraintItem> = constraints
                .iter()
                .flat_map(|c| c.items.iter().cloned())
                .collect();
            self.inline_constraints_satisfied(&items)
        } else {
            self.rand_items_accept(csp.handle, constraints, colls, &mut false)
        };
        if accepted {
            if csp.draw_free {
                self.csp_draw_free(csp);
            }
            CspOutcome::Sat
        } else {
            if std::env::var_os("XEZIM_RAND_DBG").is_some() {
                eprintln!("[rand-dbg] joint solve found a solution the checker rejects");
            }
            CspOutcome::GaveUp
        }
    }

    /// Search with restarts: the final state, every variable fixed.
    pub(super) fn csp_find(&mut self, csp: &Csp) -> Result<St, CspOutcome> {
        let nvars = csp.vars.len();
        let mut watch: Vec<Vec<usize>> = vec![Vec::new(); nvars];
        for (i, n) in csp.nodes.iter().enumerate() {
            let mut ds = Vec::new();
            n.deps(&csp.srcs, &mut ds);
            ds.sort_unstable();
            ds.dedup();
            for v in ds {
                watch[v].push(i);
            }
        }
        let aux = Self::csp_aux(csp);
        let mut budget = NODE_BUDGET;
        // A large array needs a complete assignment before restarting.
        // Auxiliary segments of a complex object graph retain the existing
        // restart interval; their count does not represent independent draws.
        let mut run_budget: u64 = csp
            .arrays
            .values()
            .map(|a| a.elems.len() as u64)
            .sum::<u64>()
            .max(1000);
        let mut st = St {
            d: csp.dom0.clone(),
            trail: Vec::new(),
            dirty: Vec::new(),
            work: 0,
            est_work: 0,
            cut: false,
            level_tries: 0,
            level_cut: false,
            probing: false,
        };
        let mut level_tries: u32 = 16;
        loop {
            st.d.clone_from(&csp.dom0);
            st.trail.clear();
            st.dirty.clear();
            // §18.5.9: a solve with rand sub-objects abandons a decision
            // level after a few refuted values, a little later each run.
            st.level_tries = if csp.has_subs { level_tries } else { 0 };
            st.level_cut = false;
            let mut run = run_budget.min(budget);
            let r = self.csp_search(csp, &aux, &watch, &mut st, &mut run);
            budget -= run_budget.min(budget) - run;
            match r {
                Some(true) => break,
                Some(false) if st.level_cut && budget > 0 && st.work < WORK_BUDGET => {
                    level_tries = level_tries.saturating_mul(2);
                    run_budget *= 2;
                }
                Some(false) if st.cut => return Err(CspOutcome::GaveUp),
                Some(false) => return Err(CspOutcome::Unsat),
                None if budget == 0 || st.work >= WORK_BUDGET => {
                    if std::env::var_os("XEZIM_RAND_DBG").is_some() {
                        eprintln!(
                            "[rand-dbg] joint solve gave up: nodes={} work={}",
                            NODE_BUDGET - budget,
                            st.work
                        );
                    }
                    return Err(CspOutcome::GaveUp);
                }
                None => run_budget *= 2,
            }
        }
        Ok(st)
    }

    /// Depth-first search: `Some(true)` solved (every domain fixed),
    /// `Some(false)` exhausted, `None` out of budget.
    fn csp_search(
        &mut self,
        csp: &Csp,
        aux: &Aux,
        watch: &[Vec<usize>],
        st: &mut St,
        budget: &mut u64,
    ) -> Option<bool> {
        let all: Vec<usize> = (0..csp.nodes.len()).collect();
        match self.csp_propagate(csp, watch, st, &all) {
            Some(true) => {}
            Some(false) => return Some(false),
            None => return None,
        }
        // (variable, value, trail mark) per decision level
        let mut stack: Vec<(usize, i128, usize)> = Vec::new();
        // Refuted values per decision level. With rand sub-objects the
        // problem is large and a wrong early decision can leave a wide
        // variable with thousands of values that all fail: after a few
        // refutations the level is abandoned (a cut, not a proof) and the
        // search backs up further or restarts.
        let mut refuted: Vec<u32> = Vec::new();
        loop {
            let Some(v) = self.csp_pick_var(csp, aux, st) else {
                return Some(true);
            };
            if *budget == 0 {
                return None;
            }
            *budget -= 1;
            let val = self.csp_pick_val(csp, aux, watch, st, v);
            stack.push((v, val, st.trail.len()));
            st.dirty.clear();
            st.set(v, vec![(val, val)]);
            let dirty = std::mem::take(&mut st.dirty);
            match self.csp_propagate(csp, watch, st, &Self::csp_watchers(watch, &dirty)) {
                Some(true) => continue,
                Some(false) => {}
                None => return None,
            }
            // Conflict: undo decisions until `x != v` leaves a consistent state.
            loop {
                let Some((v, val, mark)) = stack.pop() else {
                    return Some(false);
                };
                st.undo(mark);
                if st.level_tries > 0 {
                    let d = stack.len();
                    refuted.resize(d + 1, 0);
                    refuted[d] += 1;
                    if refuted[d] > st.level_tries && dom_size(&st.d[v]) > 2 {
                        refuted.truncate(d);
                        st.cut = true;
                        st.level_cut = true;
                        continue;
                    }
                }
                if *budget == 0 {
                    return None;
                }
                *budget -= 1;
                st.dirty.clear();
                // Each refuted value splits the domain once more, and the
                // split domain is copied again on every later exclusion — a
                // wide variable refuted value by value costs quadratic time.
                st.work += st.d[v].len() as u64;
                if st.work >= WORK_BUDGET {
                    return None;
                }
                let nd = dom_minus(&st.d[v], &vec![(val, val)]);
                if !st.set(v, nd) {
                    continue;
                }
                let dirty = std::mem::take(&mut st.dirty);
                match self.csp_propagate(csp, watch, st, &Self::csp_watchers(watch, &dirty)) {
                    Some(true) => break,
                    Some(false) => continue,
                    None => return None,
                }
            }
        }
    }

    fn csp_watchers(watch: &[Vec<usize>], vars: &[usize]) -> Vec<usize> {
        let mut out: Vec<usize> = vars
            .iter()
            .flat_map(|&v| watch[v].iter().copied())
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The next variable to decide: one whose `solve … before` predecessors
    /// are all fixed; among those a variable under a dist in force first and
    /// one whose dist still waits on its guard last; then the smallest
    /// domain, ties broken at random.
    fn csp_pick_var(&mut self, csp: &Csp, aux: &Aux, st: &St) -> Option<usize> {
        let mut best: Option<(u8, u128, usize)> = None;
        let mut ties = 0u32;
        let mut composite = None;
        for (v, d) in st.d.iter().enumerate() {
            if dom_fixed(d).is_some() {
                continue;
            }
            // Decided through its segments (see `Aux::composite`).
            if aux.composite[v] {
                composite.get_or_insert(v);
                continue;
            }
            let blocked = aux.preds[v].iter().any(|&p| st.fixed(p).is_none())
                || aux.seg_hi[v].iter().any(|&p| st.fixed(p).is_none());
            let tier = if aux.dists[v].is_empty() {
                1
            } else {
                match self.csp_dist_state(csp, aux, st, v) {
                    (Some(_), _) => 0,
                    (None, true) => 2,
                    (None, false) => 1,
                }
            };
            let rank = blocked as u8 * 3 + tier;
            let n = dom_size(d);
            match best {
                Some((br, bn, _)) if (rank, n) > (br, bn) => {}
                Some((br, bn, _)) if (rank, n) == (br, bn) => {
                    ties += 1;
                    if self.cur_rng().gen_range(0..=ties) == 0 {
                        best = Some((rank, n, v));
                    }
                }
                _ => {
                    best = Some((rank, n, v));
                    ties = 0;
                }
            }
        }
        best.map(|b| b.2).or(composite)
    }

    /// The dist in force on `v` (every guard entailed), and whether some
    /// dist on `v` still has an open guard.
    fn csp_dist_state(&mut self, csp: &Csp, aux: &Aux, st: &St, v: usize) -> (Option<usize>, bool) {
        let mut open = false;
        for &di in &aux.dists[v] {
            let mut all = true;
            for g in &csp.dists[di].guards {
                match self.csp_status(csp, g, st) {
                    Some(true) => {}
                    Some(false) => {
                        all = false;
                        break;
                    }
                    None => {
                        all = false;
                        open = true;
                    }
                }
            }
            if all {
                return (Some(di), open);
            }
        }
        (None, open)
    }

    /// A value for `v`: by the dist in force (§18.5.4: an item in proportion
    /// to its weight among the items the domain still meets, then a value of
    /// that item), by weighing (`csp_weigh`) for a condition variable, else
    /// uniformly over the domain.
    fn csp_pick_val(
        &mut self,
        csp: &Csp,
        aux: &Aux,
        watch: &[Vec<usize>],
        st: &mut St,
        v: usize,
    ) -> i128 {
        if !aux.dists[v].is_empty() {
            if let (Some(di), _) = self.csp_dist_state(csp, aux, st, v) {
                let mut live: Vec<(Dom, f64)> = Vec::new();
                let mut total = 0.0;
                for &(lo, hi, w) in &csp.dists[di].items {
                    let part = dom_clip(&st.d[v], lo, hi);
                    if !part.is_empty() {
                        total += w;
                        live.push((part, w));
                    }
                }
                if total > 0.0 {
                    let mut r = self.cur_rng().gen_range(0.0..total);
                    let mut pick = live.len() - 1;
                    for (i, (_, w)) in live.iter().enumerate() {
                        if r < *w {
                            pick = i;
                            break;
                        }
                        r -= w;
                    }
                    let part = &live[pick].0;
                    let k = self.cur_rng().gen_range(0..dom_size(part));
                    return dom_nth(part, k);
                }
            }
        }
        // §18.5.10 / §20.9: a `$countones` count is drawn in proportion to
        // the values of its variable that have that many ones (C(w, n)), as
        // a uniform draw of the variable would; the variable then draws
        // exactly that many ones.
        if let Some((_, w)) = aux.pop_x[v] {
            if let Some(n) = self.csp_pop_count(&st.d[v], w) {
                return n;
            }
        }
        if let Some(c) = aux.pop_c[v] {
            let w = csp.vars[v].width;
            for _ in 0..64 {
                let n = match st.fixed(c) {
                    Some(n) => n,
                    None => match self.csp_pop_count(&st.d[c], w) {
                        Some(n) => n,
                        None => break,
                    },
                };
                // The ones go below the top of a non-negative range.
                let (lo, hi) = st.bounds(v);
                let span = if lo >= 0 {
                    (128 - (hi as u128).leading_zeros()).min(w)
                } else {
                    w
                };
                if n < 0 || n > span as i128 {
                    break;
                }
                let x = self.csp_pop_draw(w, span, n as u32, csp.vars[v].signed);
                if dom_has(&st.d[v], x) {
                    return x;
                }
            }
        }
        if let Some(x) = self.csp_mask_draw(csp, st, v) {
            return x;
        }
        // A size bounded only below stays near that bound (see `SIZE_OPEN`).
        if st.bounds(v).1 == SIZE_TOP && csp.sizes.values().any(|&sv| sv == v) {
            let lo = st.bounds(v).0;
            let part = dom_clip(&st.d[v], lo, lo + SIZE_OPEN - 1);
            let k = self.cur_rng().gen_range(0..dom_size(&part));
            return dom_nth(&part, k);
        }
        let size = dom_size(&st.d[v]);
        if aux.guard_var[v] && size <= EST_MAX_DOM && st.est_work < EST_BUDGET {
            if let Some(x) = self.csp_weigh(csp, aux, watch, st, v) {
                return x;
            }
        }
        let k = self.cur_rng().gen_range(0..size);
        dom_nth(&st.d[v], k)
    }

    /// A count of ones for a `w`-bit variable from `dom`, weighed by the
    /// number of `w`-bit values with that count, C(w, n). None when `dom`
    /// holds no count in [0, w].
    fn csp_pop_count(&mut self, dom: &Dom, w: u32) -> Option<i128> {
        let w = w.min(64);
        let mut binom = vec![1f64; w as usize + 1];
        for n in 1..=w as usize {
            binom[n] = binom[n - 1] * (w as usize + 1 - n) as f64 / n as f64;
        }
        let cand: Vec<(i128, f64)> = dom_clip(dom, 0, w as i128)
            .iter()
            .flat_map(|&(l, h)| l..=h)
            .map(|n| (n, binom[n as usize]))
            .collect();
        let total: f64 = cand.iter().map(|c| c.1).sum();
        if cand.is_empty() || total <= 0.0 {
            return None;
        }
        let mut r = self.cur_rng().gen_range(0.0..total);
        for &(n, wt) in &cand {
            if r < wt {
                return Some(n);
            }
            r -= wt;
        }
        cand.last().map(|c| c.0)
    }

    /// log2 of the solutions a domain stands for: its size, or for a
    /// `$countones` count the values of its variable with one of its
    /// counts, Σ C(w, n).
    fn csp_log_size(aux: &Aux, u: usize, d: &Dom) -> f64 {
        let Some((_, w)) = aux.pop_x[u] else {
            return (dom_size(d) as f64).log2();
        };
        let w = w.min(64) as usize;
        let mut binom = 1f64;
        let mut total = 0f64;
        for n in 0..=w {
            if n > 0 {
                binom = binom * (w + 1 - n) as f64 / n as f64;
            }
            if dom_has(d, n as i128) {
                total += binom;
            }
        }
        total.log2()
    }

    /// A `w`-bit value with exactly `n` ones at random positions among its
    /// low `span` bits, read as the variable's value (sign-extended when it
    /// is signed).
    fn csp_pop_draw(&mut self, w: u32, span: u32, n: u32, signed: bool) -> i128 {
        let w = w.min(64);
        let mut pos: Vec<u32> = (0..span.min(w)).collect();
        let mut bits: u64 = 0;
        for k in 0..(n as usize).min(pos.len()) {
            let j = self.cur_rng().gen_range(k..pos.len());
            pos.swap(k, j);
            bits |= 1u64 << pos[k];
        }
        if signed && w > 0 && (bits >> (w - 1)) & 1 == 1 {
            bits as i128 - (1i128 << w)
        } else {
            bits as i128
        }
    }

    /// §18.5.10: without an ordering every solution is equally likely, so a
    /// value is drawn in proportion to the solutions it leaves — estimated as
    /// the product of the other domains after propagating it. Variables
    /// under a dist (their draw follows the weights, not the count) and the
    /// variables `v` is solved before do not count. None when the estimate
    /// ran out of budget.
    fn csp_weigh(
        &mut self,
        csp: &Csp,
        aux: &Aux,
        watch: &[Vec<usize>],
        st: &mut St,
        v: usize,
    ) -> Option<i128> {
        let mut after = vec![false; csp.vars.len()];
        let mut todo: Vec<usize> = aux.user_succs[v].clone();
        while let Some(u) = todo.pop() {
            if !after[u] {
                after[u] = true;
                todo.extend(aux.user_succs[u].iter().copied());
            }
        }
        // A variable drawn by a dist in force counts no values; one whose
        // dist still waits on its guard does (`if (!cfg) total dist {…}`:
        // `cfg == 0` leaves `total` five values, `cfg == 1` all of them).
        let mut drawn = vec![false; csp.vars.len()];
        for (u, ds) in aux.dists.iter().enumerate() {
            if !ds.is_empty() {
                drawn[u] = self.csp_dist_state(csp, aux, st, u).0.is_some();
            }
        }
        let vals: Vec<i128> = st.d[v].iter().flat_map(|&(l, h)| l..=h).collect();
        let mut logs: Vec<f64> = Vec::with_capacity(vals.len());
        let mut seen = vec![false; csp.vars.len()];
        let mut touched: Vec<usize> = Vec::new();
        for &x in &vals {
            let mark = st.trail.len();
            let w0 = st.work;
            st.dirty.clear();
            st.set(v, vec![(x, x)]);
            let dirty = std::mem::take(&mut st.dirty);
            let r = self.csp_propagate(csp, watch, st, &Self::csp_watchers(watch, &dirty));
            let mut lg = f64::NEG_INFINITY;
            if r == Some(true) {
                lg = 0.0;
                for (u, old) in &st.trail[mark..] {
                    let u = *u;
                    // A variable decided through its segments (`Aux::composite`)
                    // holds no values of its own — its domain is the shadow the
                    // slice link (`v == Σ seg << lo`) keeps of them — so it
                    // counts nothing here: its narrowing is the segments'
                    // narrowing again. Worse, it leaks back the freedom of a
                    // variable `v` is solved before (§18.5.10): `solve pkt.psel
                    // before pkt.paddr` excludes the `paddr` segment from the
                    // estimate, but the whole-`pkt` variable still narrows with
                    // it, and weighing `pkt.psel` by that narrowing draws the
                    // `psel` with the most `paddr` values every time.
                    // §18.5.10: a function of other variables holds one value
                    // per solution of its inputs; its narrowing is theirs. A
                    // `$countones` count still counts: it stands for the
                    // values of the variable it counts (`csp_log_size`).
                    if u == v
                        || after[u]
                        || drawn[u]
                        || seen[u]
                        || aux.composite[u]
                        || (aux.fun_var[u] && aux.pop_x[u].is_none())
                    {
                        continue;
                    }
                    seen[u] = true;
                    touched.push(u);
                    lg += Self::csp_log_size(aux, u, &st.d[u]) - Self::csp_log_size(aux, u, old);
                }
                for u in touched.drain(..) {
                    seen[u] = false;
                }
            }
            st.undo(mark);
            st.est_work += st.work - w0;
            st.work = w0;
            r?;
            logs.push(lg);
        }
        let top = logs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if top == f64::NEG_INFINITY {
            return None;
        }
        let ws: Vec<f64> = logs.iter().map(|l| (l - top).exp2()).collect();
        let total: f64 = ws.iter().sum();
        let mut r = self.cur_rng().gen_range(0.0..total);
        for (i, w) in ws.iter().enumerate() {
            if r < *w {
                return Some(vals[i]);
            }
            r -= w;
        }
        Some(vals[vals.len() - 1])
    }
}
