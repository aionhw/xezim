//! Constant selects and packed slices of solver scalars: segments, carries, declared shapes,
//! packed dimension references and masks.

use super::*;

/// §11.5.1/§7.2.1: a constant bit or part select, or a packed-struct field,
/// of a solver scalar. The scalar is split into segments at every boundary
/// such a select uses, tied to it by the exact identity `v == Σ seg·2^lsb`;
/// a select is then a plain variable (a segment, or an auxiliary variable
/// equal to the segments it covers). Relations on selects become linear and
/// propagate, instead of waiting for a guess of the whole scalar to land on
/// the selected bits (`addr[39:16] == page` on a 40-bit `addr`).
impl Simulator {
    /// Field layout and declared lsb of every scalar, then the selects the
    /// constraints use and the segments they need.
    pub(super) fn csp_split_slices(&mut self, csp: &mut Csp, constraints: &[ClassConstraint]) {
        csp.segs.clear();
        csp.slice_vars.clear();
        csp.aux_base.clear();
        csp.fields.clear();
        csp.signed_fields.clear();
        csp.field_elems.clear();
        csp.pdims.clear();
        csp.elem_fields.clear();
        csp.lsb.clear();
        csp.ascending.clear();
        let scalars: Vec<(String, usize)> =
            csp.scalars.iter().map(|(n, &v)| (n.clone(), v)).collect();
        for (name, v) in &scalars {
            if !csp.wide.contains(v) && (csp.vars[*v].signed || csp.vars[*v].width > 64) {
                continue;
            }
            if let Some((mut fields, lsb)) = self.csp_scalar_shape(csp, name) {
                let sub_var = matches!(csp.vars[*v].key, VarKey::Sub(..));
                if (csp.wide.contains(v) || sub_var) && !fields.is_empty() {
                    self.csp_nested_fields(csp, name, &mut fields);
                }
                if !fields.is_empty() && csp.scope.is_none() {
                    for (k, e) in self.csp_field_arrays(csp, name, &fields) {
                        csp.field_elems.insert((*v, k.0, k.1), e);
                    }
                }
                if !fields.is_empty() {
                    for f in self.csp_signed_members(csp, name) {
                        csp.signed_fields.insert((*v, f));
                    }
                    csp.fields.insert(*v, fields);
                }
                if let Some((left, right)) = lsb {
                    csp.lsb.insert(*v, right);
                    if left < right {
                        csp.ascending.insert(*v);
                    }
                }
            } else if let Some((dims, elem)) = self.csp_packed_dims(csp, name, csp.vars[*v].width) {
                // §7.4.1: selects of a multi-dimensional packed variable
                // name elements of its dimensions (`csp_pdim_ref`).
                csp.pdims.insert(*v, dims);
                if let Some(f) = elem {
                    csp.elem_fields.insert(*v, f);
                }
            } else {
                // Other shapes are not split here.
                csp.lsb.insert(*v, i64::MIN);
            }
        }
        // §7.4/§11.5.1: the elements of a rand array are split the same way
        // when the constraints select bits or fields of them (`a[i][39:16]`,
        // `s[i].f`); every element shares the declared element shape.
        let arrays: Vec<(String, Vec<usize>, bool)> = csp
            .arrays
            .iter()
            .map(|(n, a)| {
                let elems: Vec<usize> = a.elems.iter().map(|e| e.1).collect();
                let wide = elems.first().is_some_and(|v| csp.wide.contains(v));
                (n.clone(), elems, !wide && (a.signed || a.width > 64))
            })
            .collect();
        for (name, elems, skip) in arrays {
            let shape = if skip {
                None
            } else {
                self.csp_scalar_shape(csp, &name)
            };
            let dims = match (&shape, elems.first()) {
                (None, Some(&e)) if !skip => self.csp_packed_dims(csp, &name, csp.vars[e].width),
                _ => None,
            };
            let signed = match &shape {
                Some((fields, _)) if !fields.is_empty() => self.csp_signed_members(csp, &name),
                _ => Vec::new(),
            };
            for v in elems {
                match &shape {
                    Some((fields, lsb)) => {
                        if !fields.is_empty() {
                            for f in &signed {
                                csp.signed_fields.insert((v, f.clone()));
                            }
                            csp.fields.insert(v, fields.clone());
                        }
                        if let Some((left, right)) = *lsb {
                            csp.lsb.insert(v, right);
                            if left < right {
                                csp.ascending.insert(v);
                            }
                        }
                    }
                    None => match &dims {
                        Some((d, elem)) => {
                            csp.pdims.insert(v, d.clone());
                            if let Some(f) = elem {
                                csp.elem_fields.insert(v, f.clone());
                            }
                        }
                        None => {
                            csp.lsb.insert(v, i64::MIN);
                        }
                    },
                }
            }
        }
        // (base) -> constant ranges, and whether some select is dynamic.
        let mut uses: HashMap<usize, (Vec<(u32, u32)>, bool)> = HashMap::default();
        let mut bound: Vec<String> = Vec::new();
        let prev_any = CSP_SCAN_ANY.with(|c| c.replace(true));
        let prev_pops = CSP_SCAN_POPS.with(|p| std::mem::take(&mut *p.borrow_mut()));
        let prev_links = CSP_SCAN_LINKS.with(|p| std::mem::take(&mut *p.borrow_mut()));
        for c in constraints {
            for it in &c.items {
                self.csp_scan_item(csp, it, &mut bound, &mut uses);
            }
        }
        CSP_SCAN_ANY.with(|c| c.set(prev_any));
        let mut pops = CSP_SCAN_POPS.with(|p| std::mem::replace(&mut *p.borrow_mut(), prev_pops));
        let links = CSP_SCAN_LINKS.with(|p| std::mem::replace(&mut *p.borrow_mut(), prev_links));
        // As for the cuts below: a loop over an array counts every element.
        for a in csp.arrays.values() {
            if a.elems.iter().any(|e| pops.contains(&e.1)) {
                pops.extend(a.elems.iter().map(|e| e.1));
            }
        }
        // An element select seen through a `foreach` index was recorded on
        // one element: give every element of that array the same cuts.
        for a in csp.arrays.values() {
            let mut all: (Vec<(u32, u32)>, bool) = (Vec::new(), false);
            for &(_, v) in &a.elems {
                if let Some(u) = uses.get(&v) {
                    all.0.extend(u.0.iter().copied());
                    all.1 |= u.1;
                }
            }
            if all.0.is_empty() && !all.1 {
                continue;
            }
            all.0.sort_unstable();
            all.0.dedup();
            for &(_, v) in &a.elems {
                uses.insert(v, all.clone());
            }
        }
        // §20.9 (#852): when another constraint selects part of a counted
        // scalar, count its bits rather than a separate whole-value pop
        // variable. Otherwise the selected segment is pinned but the
        // remaining wide segment is guessed without count propagation.
        pops.sort_unstable();
        pops.dedup();
        for &v in &pops {
            if let Some(u) = uses.get_mut(&v) {
                let w = csp.vars[v].width;
                u.0.extend((0..w).map(|bit| (bit, 1)));
            }
        }
        if !csp.wide.is_empty() {
            self.csp_wide_split(csp, &uses, &links);
        }
        for (v, (ranges, dynamic)) in uses {
            let w = csp.vars[v].width;
            if csp.vars[v].signed || w > 64 || w < 2 || csp.lsb.get(&v) == Some(&i64::MIN) {
                continue;
            }
            // Keep the problem within the solver's size.
            if csp.vars.len() + ranges.len() * 2 + if dynamic { w as usize } else { 0 } > MAX_VARS {
                continue;
            }
            let mut cuts: Vec<u32> = vec![0, w];
            if dynamic {
                cuts.extend(1..w);
            }
            for &(lo, rw) in &ranges {
                cuts.push(lo);
                cuts.push(lo + rw);
            }
            cuts.retain(|&c| c <= w);
            cuts.sort_unstable();
            cuts.dedup();
            if cuts.len() <= 2 {
                continue;
            }
            let mut segs = Vec::with_capacity(cuts.len() - 1);
            for k in 0..cuts.len() - 1 {
                let (lo, hi) = (cuts[k], cuts[k + 1]);
                let x = Self::csp_aux_var(csp, hi - lo);
                csp.aux_base.insert(x, (v, lo, hi - lo));
                segs.push((lo, hi - lo, x));
            }
            for &(lo, rw) in &ranges {
                let covered = segs
                    .iter()
                    .filter(|s| s.0 >= lo && s.0 + s.1 <= lo + rw)
                    .count();
                if covered > 1 && !csp.slice_vars.contains_key(&(v, lo, rw)) {
                    let x = Self::csp_aux_var(csp, rw);
                    csp.aux_base.insert(x, (v, lo, rw));
                    csp.slice_vars.insert((v, lo, rw), x);
                }
            }
            csp.segs.insert(v, segs);
        }
        csp.pop.clear();
        csp.pop_rev.clear();
        for x in pops {
            let w = csp.vars[x].width;
            if w == 0 || w > 64 || csp.wide.contains(&x) || csp.vars.len() >= MAX_VARS {
                continue;
            }
            // Split into single bits anyway (a select at a variable
            // position): `csp_countones` sums them instead.
            if csp
                .segs
                .get(&x)
                .is_some_and(|segs| segs.iter().all(|s| s.1 == 1))
            {
                continue;
            }
            // §20.9: `$countones` returns an int.
            csp.vars.push(CspVar {
                key: VarKey::Aux,
                width: 32,
                signed: true,
            });
            csp.dom0.push(vec![(0, w as i128)]);
            let c = csp.vars.len() - 1;
            csp.pop.insert(x, c);
            csp.pop_rev.insert(c, x);
        }
    }

    /// Create the variables of a translated relation's carries (`LinCx`)
    /// and their defining equalities `acc - s - 2^w·k == 0`.
    pub(super) fn csp_add_carries(
        &mut self,
        csp: &mut Csp,
        (carries, prods, wraps): (Vec<Carry>, Vec<Prod>, Vec<Wrp>),
    ) {
        // A wrapped reading is one variable, numbered before the carries
        // and products made after it (`LinCx::next`).
        let mut wraps = wraps.into_iter().peekable();
        // All are numbered in creation order (`LinCx::next`): make the
        // variables in that order.
        let mut carries = carries.into_iter().peekable();
        let mut prods = prods.into_iter().peekable();
        loop {
            let next_other = match (carries.peek(), prods.peek()) {
                (Some(c), Some(p)) => Some(c.sv.min(p.p)),
                (Some(c), None) => Some(c.sv),
                (None, Some(p)) => Some(p.p),
                (None, None) => None,
            };
            if let Some(r) = wraps.next_if(|r| next_other.is_none_or(|n| r.u < n)) {
                debug_assert_eq!(r.u, csp.vars.len());
                csp.vars.push(CspVar {
                    key: VarKey::Aux,
                    width: r.w,
                    signed: r.s,
                });
                let (lo, hi) = ws_range(r.w, r.s);
                csp.dom0.push(vec![(lo, hi)]);
                csp.wraps.push((r.v, r.w, r.s, r.u));
                csp.nodes.push(Node::Wrap {
                    v: r.v,
                    u: r.u,
                    w: r.w,
                    s: r.s,
                });
                continue;
            }
            let c = match (carries.peek(), prods.peek()) {
                (None, None) => break,
                (Some(c), Some(p)) if p.p < c.sv => None,
                (None, Some(_)) => None,
                _ => carries.next(),
            };
            let Some(c) = c else {
                let p = prods.next().expect("a pending product");
                debug_assert_eq!(p.p, csp.vars.len());
                csp.vars.push(CspVar {
                    key: VarKey::Aux,
                    width: 128,
                    signed: true,
                });
                csp.dom0.push(vec![p.r]);
                csp.nodes.push(Node::Mul {
                    p: p.p,
                    x: p.x,
                    y: p.y,
                });
                continue;
            };
            debug_assert!(csp.vars.len() >= csp.carry_base.unwrap_or(0));
            debug_assert_eq!(c.sv, csp.vars.len());
            let sv = csp.vars.len();
            csp.vars.push(CspVar {
                key: VarKey::Aux,
                width: c.w,
                signed: c.s,
            });
            csp.dom0.push(vec![c.remainder]);
            let kv = csp.vars.len();
            csp.vars.push(CspVar {
                key: VarKey::Aux,
                width: 64,
                signed: true,
            });
            csp.dom0.push(vec![c.k]);
            let terms: Vec<usize> = c.acc.t.iter().map(|t| t.0).collect();
            // `LinCx::carry` keeps 2^w within COEF_MAX.
            let lin = Lin {
                t: vec![(sv, -1), (kv, -c.modulus)],
                k: 0,
            }
            .plus(&c.acc, 1)
            .expect("carry within the coefficient bound");
            let mut deps: Vec<usize> = lin.t.iter().map(|t| t.0).collect();
            deps.sort_unstable();
            let one = Self::literal_of(&Value::from_u64(1, 1), crate::ast::Span::dummy());
            if c.modulus == 1i128 << c.w.min(64) && c.remainder == ws_range(c.w, c.s) {
                csp.carries.push((sv, kv, terms));
                csp.carry_accs.push((c.acc.clone(), c.w, c.s));
            }
            let src = self.csp_src(csp, SrcItem::Expr(one), &Env::default(), deps);
            csp.nodes.push(Node::Lin {
                lin,
                rel: Rel::Eq,
                fits: Vec::new(),
                src,
                sneg: false,
                wrap: None,
            });
        }
    }

    pub(super) fn csp_aux_var(csp: &mut Csp, w: u32) -> usize {
        csp.vars.push(CspVar {
            key: VarKey::Aux,
            width: w,
            signed: false,
        });
        let (lo, hi) = ws_range(w, false);
        csp.dom0.push(vec![(lo, hi)]);
        csp.vars.len() - 1
    }

    /// Packed-struct fields and declared (left, right) of scalar `name`.
    /// Struct fields use zero-based storage; unsupported shapes return None.
    /// The declared type of member `name` of the object `owner`, searched
    /// up its class chain; a class type parameter stands for the type this
    /// object bound to it.
    pub(super) fn csp_prop_decl_type(&self, owner: usize, name: &str) -> Option<DataType> {
        // §18.4: a member of an unpacked-struct property (`rg.item[0]`) has
        // the declared type its member path leads to.
        if let Some((root, tail)) = name.split_once('.') {
            let mut dt = self.csp_prop_decl_type(owner, root)?;
            for part in tail.split('.') {
                let member = part.split('[').next()?;
                let (next, unpacked) = self.member_decl_type(&dt, member)?;
                let selects = part.bytes().filter(|&b| b == b'[').count();
                if selects != unpacked.len() {
                    return None;
                }
                dt = next;
            }
            return Some(dt);
        }
        let class = self
            .heap
            .get(owner)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone());
        let mut cur = class;
        let mut found = None;
        while let Some(cn) = cur {
            let Some(cd) = self.module.classes.get(&cn) else {
                break;
            };
            if let Some(t) = self.class_prop_decl_type(cd, name) {
                found = Some(t.clone());
                // §8.25: an element typed by a class type parameter
                // (`rand T beats[];`) has the shape of the type this
                // object bound to it.
                if let DataType::TypeReference {
                    name: tn,
                    dimensions,
                    ..
                } = t
                    && dimensions.is_empty()
                    && !tn.has_scope()
                    && cd.type_param_names.contains(&tn.name.name)
                    && let Some(b) = self.class_prop_type_name(owner, name)
                    && let Some(bt) = self.module.typedef_types.get(b.as_str())
                {
                    found = Some(bt.clone());
                }
                break;
            }
            cur = cd.extends.clone();
        }
        found
    }

    /// §7.4.1: the packed dimensions (left, right), outermost first, of a
    /// solver scalar or array `name` declared as a multi-dimensional packed
    /// vector (`bit [8:0][7:0] w`) whose dimensions make up `width` bits.
    /// For a packed array of packed structs (`cand_t [0:2] c`, §7.4.1) the
    /// dimensions are the array's — the struct supplies the element width —
    /// and the element's field layout comes along (`Csp::elem_fields`).
    #[allow(clippy::type_complexity)]
    fn csp_packed_dims(
        &self,
        csp: &Csp,
        name: &str,
        width: u32,
    ) -> Option<(Vec<(i64, i64)>, Option<Vec<(String, u32, u32)>>)> {
        let (dt, params) = if csp.scope.is_some() {
            let name = self.dyn_name_lookup(name).unwrap_or(name);
            (
                self.module.var_decl_types.get(name)?.clone(),
                std::borrow::Cow::Borrowed(&self.module.parameters),
            )
        } else {
            let (owner, name) = Self::csp_owner(csp, name);
            (
                self.csp_prop_decl_type(owner, name)?,
                std::borrow::Cow::Owned(self.instance_param_scope(owner)),
            )
        };
        if let Some(su) = self.packed_struct_with_dims(&dt) {
            if !su.packed || su.dimensions.is_empty() {
                return None;
            }
            let dims = Self::packed_range_bounds(&su.dimensions, params.as_ref())?;
            let (mut fields, elem_w) = self.packed_agg_layout_with(&su, params.as_ref());
            let total = Self::packed_struct_array_width(&su, elem_w.max(1), params.as_ref())?;
            if total != width {
                return None;
            }
            let top = fields.clone();
            self.csp_nested_walk(&su, params.as_ref(), "", 0, &top, &mut fields, 0);
            return Some((dims, Some(fields)));
        }
        let dims = crate::compiler::elaborate::packed_full_dims_chained(
            &dt,
            params.as_ref(),
            &self.module.typedef_types,
        )?;
        let bits = dims.iter().try_fold(1u64, |acc, &(l, r)| {
            acc.checked_mul((l - r).unsigned_abs().checked_add(1)?)
        })?;
        (dims.len() >= 2 && bits == u64::from(width)).then_some((dims, None))
    }

    #[allow(clippy::type_complexity)]
    fn csp_scalar_shape(
        &mut self,
        csp: &Csp,
        name: &str,
    ) -> Option<(Vec<(String, u32, u32)>, Option<(i64, i64)>)> {
        let dt: Option<DataType> = if csp.scope.is_some() {
            // A collection local to an automatic task is registered under
            // its per-call key (`declare_local_dyn`).
            let name = self.dyn_name_lookup(name).unwrap_or(name);
            if let Some(f) = self.module.packed_struct_fields.get(name) {
                return Some((f.to_vec(), None));
            }
            self.module.var_decl_types.get(name).cloned()
        } else {
            // A sub-object's member is declared by the sub-object's class.
            let (owner, name) = Self::csp_owner(csp, name);
            // A row of a multi-dimensional member (`a[1]`) has its
            // element type (§7.4.5, #519).
            let name = match name.split_once('[') {
                Some((b, _)) if csp.nd_dims.contains_key(b) => b,
                _ => name,
            };
            if let Some(su) = self.class_prop_struct(owner, name) {
                // §7.4.1: a packed ARRAY of structs is `csp_packed_dims`'
                // shape — its fields are per element, not of the whole.
                if !su.packed || !su.dimensions.is_empty() {
                    return None;
                }
                let params = self.instance_param_scope(owner);
                let (fields, _) = self.packed_agg_layout_with(&su, &params);
                return Some((fields, None));
            }
            self.csp_prop_decl_type(owner, name)
        };
        let Some(dt) = dt else {
            return Some((Vec::new(), Some((0, 0))));
        };
        let dt = crate::compiler::elaborate::resolve_typedef_chain(&dt, &self.module.typedef_types)
            .clone();
        if let Some(f) = crate::compiler::elaborate::packed_struct_field_layout(
            &dt,
            &self.module.parameters,
            &self.module.typedefs,
            &self.module.typedef_types,
        ) {
            return Some((f, None));
        }
        match &dt {
            DataType::IntegerVector { dimensions, .. } => match dimensions.as_slice() {
                [] => Some((Vec::new(), Some((0, 0)))),
                [crate::ast::types::PackedDimension::Range { left, right, .. }] => {
                    let l = crate::compiler::elaborate::const_eval_i64_with_params(
                        left,
                        Some(&self.module.parameters),
                    )?;
                    let r = crate::compiler::elaborate::const_eval_i64_with_params(
                        right,
                        Some(&self.module.parameters),
                    )?;
                    Some((Vec::new(), Some((l, r))))
                }
                _ => None,
            },
            _ => Some((Vec::new(), Some((0, 0)))),
        }
    }

    /// §7.2.1: the members of the packed struct `name` is declared with
    /// that are signed, by their dotted path from it (`s`, `a.x`): their
    /// selects read signed although the struct itself is unsigned.
    fn csp_signed_members(&mut self, csp: &Csp, name: &str) -> Vec<String> {
        let su: Option<crate::ast::types::StructUnionType> = if csp.scope.is_some() {
            let name = self.dyn_name_lookup(name).unwrap_or(name);
            self.module
                .var_decl_types
                .get(name)
                .cloned()
                .and_then(|dt| {
                    match crate::compiler::elaborate::resolve_typedef_chain(
                        &dt,
                        &self.module.typedef_types,
                    ) {
                        DataType::Struct(su) => Some(su.clone()),
                        _ => None,
                    }
                })
        } else {
            let (owner, name) = Self::csp_owner(csp, name);
            let name = match name.split_once('[') {
                Some((b, _)) if csp.nd_dims.contains_key(b) => b,
                _ => name,
            };
            self.class_prop_struct(owner, name)
                .map(|su| (*su).clone())
                .or_else(|| {
                    let dt = self.csp_prop_decl_type(owner, name)?;
                    match crate::compiler::elaborate::resolve_typedef_chain(
                        &dt,
                        &self.module.typedef_types,
                    ) {
                        DataType::Struct(su) => Some(su.clone()),
                        _ => None,
                    }
                })
        };
        let mut out = Vec::new();
        let Some(su) = su.filter(|su| su.packed) else {
            return out;
        };
        let mut todo = vec![(String::new(), su, 0u32)];
        while let Some((prefix, su, depth)) = todo.pop() {
            for m in &su.members {
                let tt = &self.module.typedef_types;
                let signed = crate::compiler::elaborate::is_type_signed_resolved(&m.data_type, tt);
                let inner =
                    match crate::compiler::elaborate::resolve_typedef_chain(&m.data_type, tt) {
                        DataType::Struct(n) if n.packed && n.dimensions.is_empty() && depth < 8 => {
                            Some(n.clone())
                        }
                        _ => None,
                    };
                for d in &m.declarators {
                    let path = format!("{prefix}{}", d.name.name);
                    if signed {
                        out.push(path.clone());
                    }
                    if let Some(n) = &inner {
                        todo.push((format!("{path}."), n.clone(), depth + 1));
                    }
                }
            }
        }
        out
    }

    fn csp_scan_item(
        &mut self,
        csp: &Csp,
        it: &ConstraintItem,
        bound: &mut Vec<String>,
        uses: &mut HashMap<usize, (Vec<(u32, u32)>, bool)>,
    ) {
        let mut scan = |me: &mut Self, e: &Expression, bound: &mut Vec<String>| {
            me.csp_scan_expr(csp, e, bound, uses)
        };
        match it {
            ConstraintItem::Expr(e) => scan(self, e, bound),
            ConstraintItem::Inside { expr, range, .. } => {
                scan(self, expr, bound);
                for r in range {
                    match r {
                        ConstraintRange::Value(e) => scan(self, e, bound),
                        ConstraintRange::Range { lo, hi } => {
                            scan(self, lo, bound);
                            scan(self, hi, bound);
                        }
                    }
                }
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                scan(self, condition, bound);
                self.csp_scan_item(csp, constraint, bound, uses);
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                scan(self, condition, bound);
                self.csp_scan_item(csp, then_item, bound, uses);
                if let Some(e) = else_item {
                    self.csp_scan_item(csp, e, bound, uses);
                }
            }
            ConstraintItem::Foreach {
                array, vars, item, ..
            } => {
                // §18.5.8.1: a loop over a fixed-shape state member (a
                // vector used as an iterator) is translated one index at a
                // time; with wide variables, scan it that way too, so a
                // select at a position computed from the index (`r.f[i*18 +
                // 8]`) is a constant select of its own segment.
                if !csp.wide.is_empty()
                    && let [Some(iv)] = vars.as_slice()
                    && let Some(idx) = self.csp_foreach_indices(
                        csp,
                        array,
                        Self::foreach_base_name(array).as_deref(),
                    )
                {
                    for i in idx {
                        CSP_SCAN_BINDS.with(|b| {
                            b.borrow_mut()
                                .push((iv.name.clone(), Self::signed_loop_val(i)))
                        });
                        self.csp_scan_item(csp, item, bound, uses);
                        CSP_SCAN_BINDS.with(|b| b.borrow_mut().pop());
                    }
                    return;
                }
                let mark = bound.len();
                bound.extend(vars.iter().flatten().map(|v| v.name.clone()));
                self.csp_scan_item(csp, item, bound, uses);
                bound.truncate(mark);
            }
            ConstraintItem::Soft(x) => self.csp_scan_item(csp, x, bound, uses),
            ConstraintItem::Block(xs) => {
                for x in xs {
                    self.csp_scan_item(csp, x, bound, uses);
                }
            }
            ConstraintItem::Unique { exprs, .. } => {
                for e in exprs {
                    scan(self, e, bound);
                }
            }
            ConstraintItem::Solve { .. } => {}
        }
    }

    fn csp_scan_expr(
        &mut self,
        csp: &Csp,
        e: &Expression,
        bound: &mut Vec<String>,
        uses: &mut HashMap<usize, (Vec<(u32, u32)>, bool)>,
    ) {
        let bound_now = bound.clone();
        let scan_env = csp_scan_env();
        let mut konst = |me: &mut Self, x: &Expression| -> Option<i64> {
            let mut reads_bound = false;
            Self::walk_operands(x, &mut |y| {
                if let ExprKind::Ident(h) = &y.kind {
                    if h.path.len() == 1 && bound_now.iter().any(|b| *b == h.path[0].name.name) {
                        reads_bound = true;
                    }
                }
            });
            if reads_bound || !me.csp_free(csp, x, &scan_env) {
                return None;
            }
            me.csp_const(x, &scan_env)?.to_i64()
        };
        // §7.4.5: a part of an element at a position not known yet
        // (`f[id][23:16]`): that part of every element.
        if let ExprKind::RangeSelect {
            expr: rb,
            kind: crate::ast::expr::RangeKind::Constant,
            left,
            right,
        } = &Self::unparen(e).kind
            && let Some((b, idx)) = Self::csp_split_last_select(Self::unparen(rb))
            && konst(self, &idx).is_none()
            && let (Some(l), Some(r)) = (konst(self, left), konst(self, right))
            && let Some((v, all)) = self.csp_field_elem_all(csp, &b, &mut konst)
        {
            let (plo, phi) = (l.min(r), l.max(r));
            let u = uses.entry(v).or_default();
            for x in &all {
                if phi < i64::from(x.2) && plo >= 0 {
                    u.0.push((x.1 + plo as u32, (phi - plo + 1) as u32));
                }
            }
        }
        // §7.4.5: an element of a packed-array field at a position not
        // known yet may be any element: cut at every element.
        let mut elem_dyn = false;
        // §7.4.1: every range a select of a multi-dimensional packed
        // variable may name is a select of it.
        if let Some(p) = self.csp_pdim_ref(csp, e, &mut konst) {
            elem_dyn = true;
            if let Some(p) = p
                && p.depth > 0
            {
                uses.entry(p.v).or_default().0.extend(p.pos);
            }
        }
        if !elem_dyn
            && let Some((b, idx)) = Self::csp_split_last_select(Self::unparen(e))
            && konst(self, &idx).is_none()
            && let Some((v, all)) = self.csp_field_elem_all(csp, &b, &mut konst)
        {
            uses.entry(v)
                .or_default()
                .0
                .extend(all.iter().map(|x| (x.1, x.2)));
            elem_dyn = true;
        }
        // §7.4.1/§12.7.3: a field of an element of a packed array of structs
        // at an index not known yet (`foreach (c[i]) c[i].valid == 1`): that
        // field of every element (#928). A whole-variable dynamic select
        // instead left the solver one opaque vector it could not search.
        if !elem_dyn && let Some((p, flo, fw)) = self.csp_pdim_field_parts(csp, e, &mut konst) {
            elem_dyn = true;
            uses.entry(p.v)
                .or_default()
                .0
                .extend(p.pos.iter().map(|&(lo, _)| (lo + flo, fw)));
        }
        if !elem_dyn && let Some((v, r)) = self.csp_select_ref(csp, e, &mut konst) {
            let u = uses.entry(v).or_default();
            match r {
                Some(r) => u.0.push(r),
                None => u.1 = true,
            }
        }
        // `$countones(x)` of a whole variable of at most 64 bits is a count
        // variable (see `Csp::pop`). Of a select, or of a wider variable, it
        // is the sum of single bits (see `csp_countones`).
        if let ExprKind::SystemCall { name, args } = &Self::unparen(e).kind
            && name == "$countones"
            && let [x] = args.as_slice()
        {
            // §20.9: the ones of a concatenation are those of its parts.
            let mut parts = vec![x];
            while let Some(x) = parts.pop() {
                if let ExprKind::Concatenation(xs) = &Self::unparen(x).kind {
                    parts.extend(xs.iter().rev());
                    continue;
                }
                let Some((v, lo, w, whole)) = self.csp_slice_base_k(csp, x, &mut konst) else {
                    continue;
                };
                if w == 0 {
                    continue;
                }
                if whole && lo == 0 && w == csp.vars[v].width && w <= 64 && !csp.wide.contains(&v) {
                    CSP_SCAN_POPS.with(|p| p.borrow_mut().push(v));
                } else if csp.vars.len() + w as usize <= MAX_VARS {
                    let u = uses.entry(v).or_default();
                    u.0.extend((lo..lo + w).map(|b| (b, 1)));
                }
            }
        }
        // `x & MASK`: the runs of ones in the mask are selects of x.
        if let ExprKind::Binary {
            op: BinaryOp::BitAnd,
            left,
            right,
        } = &Self::unparen(e).kind
        {
            for (x, m) in [(left, right), (right, left)] {
                let Some(mask) = konst(self, m) else { continue };
                if let Some((v, lo, w, _)) = self.csp_slice_base_k(csp, x, &mut konst) {
                    let u = uses.entry(v).or_default();
                    for (rl, rw) in Self::mask_runs(mask as u64, w) {
                        u.0.push((lo + rl, rw));
                    }
                }
            }
        }
        // §11.4.10: `x >> K` is the select `x[w-1:K]` (see `csp_shr_slice`).
        if let ExprKind::Binary {
            op: BinaryOp::ShiftRight,
            left,
            right,
        } = &Self::unparen(e).kind
            && let Some(k) = konst(self, right)
            && let Some((v, lo, w, _)) = self.csp_slice_base_k(csp, left, &mut konst)
            && k > 0
            && k < i64::from(w)
        {
            uses.entry(v)
                .or_default()
                .0
                .push((lo + k as u32, w - k as u32));
        }
        if !csp.wide.is_empty() {
            self.csp_wide_scan(csp, e, &bound_now, uses);
        }
        let mut subs: Vec<&Expression> = Vec::new();
        match &e.kind {
            ExprKind::Ident(h) => {
                for seg in &h.path {
                    subs.extend(seg.selects.iter());
                }
            }
            ExprKind::Unary { operand, .. } => subs.push(operand),
            ExprKind::Binary { left, right, .. } => {
                subs.push(left);
                subs.push(right);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => subs.extend([condition.as_ref(), then_expr, else_expr]),
            ExprKind::Concatenation(parts) => subs.extend(parts.iter()),
            ExprKind::Replication { count, exprs } => {
                subs.push(count);
                subs.extend(exprs.iter());
            }
            ExprKind::Call { func, args } => {
                subs.push(func);
                subs.extend(args.iter());
            }
            ExprKind::SystemCall { args, .. } => subs.extend(args.iter()),
            ExprKind::Inside { expr, ranges } => {
                subs.push(expr);
                subs.extend(ranges.iter());
            }
            ExprKind::MemberAccess { expr, .. } | ExprKind::Paren(expr) => subs.push(expr),
            ExprKind::Index { expr, index } => {
                subs.push(expr);
                subs.push(index);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => subs.extend([expr.as_ref(), left, right]),
            ExprKind::Range(a, b) => {
                subs.push(a);
                subs.push(b);
            }
            ExprKind::WithClause { expr, filter } => {
                subs.push(expr);
                subs.push(filter);
            }
            _ => {}
        }
        for x in subs {
            self.csp_scan_expr(csp, x, bound, uses);
        }
    }

    /// The bits of a solver scalar `e` names: Some((base, Some((lsb, w))))
    /// for a constant select or field, Some((base, None)) for a select
    /// whose position is not constant, None when `e` is no select or field
    /// of a scalar (a whole scalar included). Select indices count from the
    /// declared lsb of a whole variable, from 0 within a field.
    pub(super) fn csp_select_ref(
        &mut self,
        csp: &Csp,
        e: &Expression,
        konst: &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
    ) -> Option<(usize, Option<(u32, u32)>)> {
        let e = Self::unparen(e);
        let pick = |me: &mut Self,
                    base: &Expression,
                    konst: &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
                    lo_hi: &mut dyn FnMut(
            &mut Self,
            &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
        ) -> Option<Option<(i64, i64)>>|
         -> Option<(usize, Option<(u32, u32)>)> {
            let (v, blo, bw, whole) = me.csp_slice_base_k(csp, base, konst)?;
            let origin = if whole {
                csp.lsb.get(&v).copied().unwrap_or(0)
            } else {
                0
            };
            let Some((lo, w)) = lo_hi(me, konst)? else {
                return Some((v, None));
            };
            let lo = if whole && csp.ascending.contains(&v) {
                origin - lo - w + 1
            } else {
                lo - origin
            };
            if lo < 0 || w < 1 || lo + w > bw as i64 {
                return None;
            }
            Some((v, Some((blo + lo as u32, w as u32))))
        };
        if let Some(p) = self.csp_pdim_ref(csp, e, konst) {
            let p = p?;
            if p.depth == 0 {
                return None;
            }
            return Some((p.v, p.known.then(|| p.pos[0])));
        }
        if let Some(r) = self.csp_field_elem(csp, e, konst) {
            return Some(r);
        }
        // `c[i].f` of a packed array of structs: a field of an element whose
        // index may still be a loop variable (then a dynamic select of `c`).
        if let Some(r) = self.csp_pdim_field(csp, e, konst) {
            return Some(r);
        }
        match &e.kind {
            ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } => pick(self, expr, konst, &mut |me, konst| {
                let (l, r) = (konst(me, left), konst(me, right));
                Some(match kind {
                    crate::ast::expr::RangeKind::Constant => match (l, r) {
                        (Some(l), Some(r)) => {
                            let (v, _, _, whole) = me.csp_slice_base_k(csp, expr, konst)?;
                            let ascending = whole && csp.ascending.contains(&v);
                            if (ascending && l <= r) || (!ascending && l >= r) {
                                Some((l.min(r), (l - r).abs() + 1))
                            } else {
                                return None;
                            }
                        }
                        _ => None,
                    },
                    crate::ast::expr::RangeKind::IndexedUp => {
                        let w = r?;
                        l.map(|b| (b, w))
                    }
                    crate::ast::expr::RangeKind::IndexedDown => {
                        let w = r?;
                        l.map(|b| (b - w + 1, w))
                    }
                })
            }),
            ExprKind::Index { expr, index } => pick(self, expr, konst, &mut |me, konst| {
                Some(konst(me, index).map(|i| (i, 1)))
            }),
            // `x[i]` held as a select on the identifier's last segment.
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.last().is_some_and(|s| s.selects.len() == 1)
                    && h.path[..h.path.len() - 1]
                        .iter()
                        .all(|s| s.selects.is_empty()) =>
            {
                let mut bh = h.clone();
                let idx = bh.path.last_mut().unwrap().selects.pop().unwrap();
                let base = Expression::new(ExprKind::Ident(bh), e.span);
                pick(self, &base, konst, &mut |me, konst| {
                    Some(konst(me, &idx).map(|i| (i, 1)))
                })
            }
            _ => {
                let (v, lo, w, whole) = self.csp_slice_base_k(csp, e, konst)?;
                (!whole).then_some((v, Some((lo, w))))
            }
        }
    }

    /// §7.4.1/§11.5.1: `e` as a select of a multi-dimensional packed
    /// variable (`csp.pdims`): `w`, `w[i]`, `w[i][j]`, `w[i][3:0]`,
    /// `w[5:2]`, `w[i+:2]`. Each index or part-select applies to the next
    /// unpacked-from-the-left dimension, in that dimension's declared
    /// labels; an index not constant here (a `foreach` index while the
    /// constraints are scanned) yields every element it could name. None
    /// when `e` is not rooted at such a variable; Some(None) when it is but
    /// the select is not modelled (out of range, a select after a
    /// part-select, a non-constant part-select width).
    pub(super) fn csp_pdim_ref(
        &mut self,
        csp: &Csp,
        e: &Expression,
        konst: &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
    ) -> Option<Option<PdimRef>> {
        use crate::ast::expr::RangeKind;
        if csp.pdims.is_empty() {
            return None;
        }
        let e = Self::unparen(e);
        let whole = |v: usize| {
            csp.pdims.contains_key(&v).then(|| PdimRef {
                v,
                pos: vec![(0, csp.vars[v].width)],
                depth: 0,
                known: true,
            })
        };
        if let Some(n) = self.csp_member(csp, e)
            && let Some(&v) = csp.scalars.get(&n)
        {
            return whole(v).map(Some);
        }
        if let Some(v) = self.csp_elem_ref(csp, e, konst) {
            return whole(v).map(Some);
        }
        enum Sel<'a> {
            Index(&'a Expression),
            Part(RangeKind, &'a Expression, &'a Expression),
        }
        let peeled;
        let (base, sel): (&Expression, Sel) = match &e.kind {
            ExprKind::Index { expr, index } => (expr, Sel::Index(index)),
            ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } => (expr, Sel::Part(*kind, left, right)),
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.last().is_some_and(|s| !s.selects.is_empty())
                    && h.path[..h.path.len() - 1]
                        .iter()
                        .all(|s| s.selects.is_empty()) =>
            {
                let mut bh = h.clone();
                let idx = bh.path.last_mut().unwrap().selects.pop().unwrap();
                if matches!(idx.kind, ExprKind::Range(..)) {
                    return None;
                }
                bh.cached_signal_id = crate::ast::cache::SignalCache::new(None);
                bh.cached_resolved_name = crate::ast::cache::NameCache::new();
                peeled = (Expression::new(ExprKind::Ident(bh), e.span), idx);
                (&peeled.0, Sel::Index(&peeled.1))
            }
            _ => return None,
        };
        let Some(b) = self.csp_pdim_ref(csp, base, konst)? else {
            return Some(None);
        };
        let Some(&(l, r)) = csp.pdims.get(&b.v).and_then(|d| d.get(b.depth)) else {
            return Some(None);
        };
        let n = (l - r).unsigned_abs() + 1;
        let cw = u64::from(b.pos[0].1);
        if n > cw || cw % n != 0 {
            return Some(None);
        }
        let ew = (cw / n) as u32;
        // The offset of label `k` from the least significant element.
        let off = |k: i64| -> Option<u32> {
            let o = if l >= r { k - r } else { r - k };
            (0..n as i64).contains(&o).then_some(o as u32)
        };
        let (lab_lo, lab_hi) = (l.min(r), l.max(r));
        // The element offset ranges (first, count) the select may name.
        let mut runs: Vec<(u32, u32)> = Vec::new();
        let mut known = b.known;
        let depth;
        match sel {
            Sel::Index(ix) => {
                depth = b.depth + 1;
                match konst(self, ix) {
                    Some(k) => match off(k) {
                        Some(o) => runs.push((o, 1)),
                        None => return Some(None),
                    },
                    None => {
                        known = false;
                        runs.extend((0..n as u32).map(|o| (o, 1)));
                    }
                }
            }
            Sel::Part(kind, a, c) => {
                depth = usize::MAX;
                // The labels [first, last] the part-select covers.
                let span = |s: i64, cnt: i64, up: bool| -> (i64, i64) {
                    if up {
                        (s, s + cnt - 1)
                    } else {
                        (s - cnt + 1, s)
                    }
                };
                let mut labels: Vec<(i64, i64)> = Vec::new();
                match kind {
                    RangeKind::Constant => {
                        let (Some(x), Some(y)) = (konst(self, a), konst(self, c)) else {
                            return Some(None);
                        };
                        // §11.5.1: a part-select keeps the declared direction.
                        if x != y && (x > y) != (l > r) {
                            return Some(None);
                        }
                        labels.push((x.min(y), x.max(y)));
                    }
                    RangeKind::IndexedUp | RangeKind::IndexedDown => {
                        let up = kind == RangeKind::IndexedUp;
                        let Some(cnt) = konst(self, c).filter(|&c| c >= 1) else {
                            return Some(None);
                        };
                        match konst(self, a) {
                            Some(s0) => labels.push(span(s0, cnt, up)),
                            None => {
                                known = false;
                                for s0 in lab_lo..=lab_hi {
                                    let (x, y) = span(s0, cnt, up);
                                    if x >= lab_lo && y <= lab_hi {
                                        labels.push((x, y));
                                    }
                                }
                            }
                        }
                    }
                }
                for (x, y) in labels {
                    let (Some(ox), Some(oy)) = (off(x), off(y)) else {
                        return Some(None);
                    };
                    runs.push((ox.min(oy), ox.max(oy) - ox.min(oy) + 1));
                }
                if runs.is_empty() {
                    return Some(None);
                }
            }
        }
        if b.pos.len() * runs.len() > 4096 {
            return Some(None);
        }
        let pos = b
            .pos
            .iter()
            .flat_map(|&(lo, _)| runs.iter().map(move |&(o, k)| (lo + o * ew, k * ew)))
            .collect();
        Some(Some(PdimRef {
            v: b.v,
            pos,
            depth,
            known,
        }))
    }

    /// §7.4.1/§7.2.1: `e` as a field of one element of a packed array of
    /// packed structs (`c[0].valid`, `this.c[1].sub.mode`, `m[1][2].f`, in
    /// either parse shape — segment selects on a flat identifier, or a
    /// `MemberAccess` chain over an index): (base variable, lsb, width).
    /// Some((base, None)) when the field exists but an element index is not
    /// constant (a `foreach` variable while the constraints are scanned): a
    /// select at a position not known yet, like `csp_select_ref`'s. None
    /// unless every dimension of the element is indexed and the dotted
    /// field path is in the element layout (`Csp::elem_fields`).
    fn csp_pdim_field(
        &mut self,
        csp: &Csp,
        e: &Expression,
        konst: &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
    ) -> Option<(usize, Option<(u32, u32)>)> {
        let (p, flo, fw) = self.csp_pdim_field_parts(csp, e, konst)?;
        if !p.known {
            return Some((p.v, None));
        }
        Some((p.v, Some((p.pos[0].0 + flo, fw))))
    }

    /// `csp_pdim_field`'s parts: the element reference (every element it may
    /// name, when an index is not known yet) and the field's offset in the
    /// element and width.
    fn csp_pdim_field_parts(
        &mut self,
        csp: &Csp,
        e: &Expression,
        konst: &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
    ) -> Option<(PdimRef, u32, u32)> {
        if csp.elem_fields.is_empty() {
            return None;
        }
        // Member names peeled from the outside in; the element below them.
        let mut outer: Vec<&str> = Vec::new();
        let mut cur = Self::unparen(e);
        while let ExprKind::MemberAccess { expr, member } = &cur.kind {
            outer.push(member.name.as_str());
            cur = Self::unparen(expr);
        }
        let mut path: Vec<&str> = Vec::new();
        let base: Expression = match &cur.kind {
            ExprKind::Ident(h) if h.root.is_none() => {
                // The last segment WITH selects is the element; the segments
                // after it are member names.
                let k = h.path.iter().rposition(|s| !s.selects.is_empty())?;
                if h.path[..k].iter().any(|s| !s.selects.is_empty()) {
                    return None;
                }
                path.extend(h.path[k + 1..].iter().map(|s| s.name.name.as_str()));
                let mut bh = h.clone();
                bh.path.truncate(k + 1);
                bh.cached_signal_id = crate::ast::cache::SignalCache::new(None);
                bh.cached_resolved_name = crate::ast::cache::NameCache::new();
                Expression::new(ExprKind::Ident(bh), cur.span)
            }
            ExprKind::Index { .. } => cur.clone(),
            _ => return None,
        };
        path.extend(outer.iter().rev());
        if path.is_empty() {
            return None;
        }
        let p = self.csp_pdim_ref(csp, &base, konst)??;
        let fields = csp.elem_fields.get(&p.v)?;
        if p.depth != csp.pdims.get(&p.v)?.len() {
            return None;
        }
        let key = path.join(".");
        let &(_, flo, fw) = fields.iter().find(|(n, ..)| *n == key)?;
        Some((p, flo, fw))
    }

    /// §7.4.1/§18.5.8.1: `e` as a slice of a solver variable, placed by the
    /// declared geometry of the class property it selects into — a field of
    /// an element of a packed-array member of a packed struct (`g.item[1].key`),
    /// or a field of a member of an unpacked-struct property (`rg.item[0].key`,
    /// the variable `rg.item[0]`). Only indices the solver already knows are
    /// frozen, never a trial value of a random selector; a bound `foreach`
    /// variable is one of them.
    fn csp_declared_member_slice(
        &mut self,
        csp: &Csp,
        e: &Expression,
        konst: &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
    ) -> Option<(usize, u32, u32, bool)> {
        fn freeze(
            me: &mut Simulator,
            e: &Expression,
            k: &mut dyn FnMut(&mut Simulator, &Expression) -> Option<i64>,
        ) -> Option<Expression> {
            let mut out = e.clone();
            match &mut out.kind {
                ExprKind::Ident(h) => {
                    for seg in &mut h.path {
                        for i in &mut seg.selects {
                            let n = k(me, i)?;
                            *i = Simulator::literal_of(&Simulator::signed_loop_val(n), i.span);
                        }
                    }
                }
                ExprKind::MemberAccess { expr, .. } => **expr = freeze(me, expr, k)?,
                ExprKind::Index { expr, index } => {
                    **expr = freeze(me, expr, k)?;
                    let n = k(me, index)?;
                    **index = Simulator::literal_of(&Simulator::signed_loop_val(n), index.span);
                }
                ExprKind::This => {}
                _ => return None,
            }
            Some(out)
        }
        if csp.scope.is_some() || !self.chain_may_name_struct_prop(e) {
            return None;
        }
        let target = freeze(self, e, konst)?;
        let (owner, prop, off, width) = match self.class_agg_member(&target)? {
            ClassAggRef::Packed {
                handle,
                prop,
                off,
                w,
                ..
            } => (handle, prop, off, w),
            ClassAggRef::UnpackedSlice {
                handle,
                key,
                off,
                w,
                ..
            } => (handle, key, off, w),
            ClassAggRef::Unpacked { handle, key, w } => (handle, key, 0, w),
        };
        let name = if owner == csp.handle {
            prop
        } else {
            format!("{}.{}", csp.sub_prefixes.get(&owner)?, prop)
        };
        let &v = csp.scalars.get(&name)?;
        if self.csp_var_no_slices(csp, v) || off.checked_add(width)? > csp.vars[v].width {
            return None;
        }
        Some((v, off, width, off == 0 && width == csp.vars[v].width))
    }

    /// `e` as (base, lsb, width, whole): a whole scalar, a field, or a select
    /// of either.
    pub(super) fn csp_slice_base_k(
        &mut self,
        csp: &Csp,
        e: &Expression,
        konst: &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
    ) -> Option<(usize, u32, u32, bool)> {
        let e = Self::unparen(e);
        if let Some(slice) = self.csp_declared_member_slice(csp, e, konst) {
            return Some(slice);
        }
        let var_of = |me: &Self, n: &str| -> Option<usize> {
            let &v = csp.scalars.get(n)?;
            (!me.csp_var_no_slices(csp, v)).then_some(v)
        };
        let field = |csp: &Csp, v: usize, f: &str| -> Option<(u32, u32)> {
            csp.fields
                .get(&v)?
                .iter()
                .find(|(n, ..)| n == f)
                .map(|&(_, lo, w)| (lo, w))
        };
        if let Some(p) = self.csp_pdim_ref(csp, e, konst) {
            let p = p.filter(|p| p.known)?;
            let (lo, w) = p.pos[0];
            return Some((p.v, lo, w, p.depth == 0));
        }
        // `c[k].f`: a field of one element of a packed array of structs.
        if let Some((v, pos)) = self.csp_pdim_field(csp, e, konst) {
            let (lo, w) = pos?;
            return Some((v, lo, w, false));
        }
        // `x`, `this.x`, `recv.x`: the scalar itself.
        if let Some(n) = self.csp_member(csp, e) {
            if let Some(v) = var_of(self, &n) {
                return Some((v, 0, csp.vars[v].width, true));
            }
        }
        // `a[k]`: an element of a solver array, as a whole.
        if let Some(v) = self.csp_elem_ref(csp, e, konst) {
            return Some((v, 0, csp.vars[v].width, true));
        }
        // `f[k]`: an element of a packed-array field.
        if let Some((v, Some((lo, w)))) = self.csp_field_elem(csp, e, konst) {
            return Some((v, lo, w, false));
        }
        match &e.kind {
            ExprKind::Ident(h)
                if h.root.is_none() && h.path.iter().all(|s| s.selects.is_empty()) =>
            {
                let names: Vec<&str> = h.path.iter().map(|s| s.name.name.as_str()).collect();
                // object-first binding: `p.f` is field f of the scalar p;
                // `this.p.f` / `recv.p.f` likewise.
                let start = if names.len() >= 2
                    && (names[0] == "this" || self.rand_receiver.as_deref() == Some(names[0]))
                    && !csp.scalars.contains_key(names[0])
                {
                    1
                } else {
                    0
                };
                // A rand sub-object's member `a.x` or back-referenced member
                // `p[0].item.s` at any depth is one variable.
                let mut found = None;
                for split in (start + 1..=names.len()).rev() {
                    let prefix = &names[start..split];
                    let joined = prefix.join(".");
                    if let Some(v) = var_of(self, &joined) {
                        found = Some((v, split));
                        break;
                    }
                    if (csp.has_subs || csp.dotted) && prefix.len() >= 2 {
                        let owned: Vec<String> =
                            prefix.iter().map(|s| (*s).to_string()).collect();
                        if let Some(n) = self.csp_member_through_handles(csp, &owned)
                            && let Some(v) = var_of(self, &n)
                        {
                            found = Some((v, split));
                            break;
                        }
                    }
                }
                let (v, start) = found?;
                if names.len() == start {
                    return Some((v, 0, csp.vars[v].width, true));
                }
                // A nested field (`s.a.b`) is listed by its dotted path
                // (wide and sub-object variables, `csp_nested_fields`).
                let (lo, w) = field(csp, v, &names[start..].join("."))?;
                Some((v, lo, w, false))
            }
            ExprKind::MemberAccess { expr, member } => {
                let (v, lo, _, whole) = self.csp_slice_base_k(csp, expr, konst)?;
                if !whole {
                    let mut path = Self::csp_member_path(e)?;
                    if path[0] == "this" || self.rand_receiver.as_deref() == Some(path[0].as_str())
                    {
                        path.remove(0);
                    }
                    let mut start = None;
                    for split in (1..path.len()).rev() {
                        let prefix = &path[..split];
                        if csp.scalars.get(&prefix.join(".")) == Some(&v) {
                            start = Some(split);
                            break;
                        }
                        if (csp.has_subs || csp.dotted)
                            && prefix.len() >= 2
                            && self
                                .csp_member_through_handles(csp, prefix)
                                .is_some_and(|n| csp.scalars.get(&n) == Some(&v))
                        {
                            start = Some(split);
                            break;
                        }
                    }
                    let start = start?;
                    if path.len() <= start {
                        return None;
                    }
                    let (flo, fw) = field(csp, v, &path[start..].join("."))?;
                    return Some((v, flo, fw, false));
                }
                let (flo, fw) = field(csp, v, &member.name)?;
                Some((v, lo + flo, fw, false))
            }
            _ => None,
        }
    }

    /// `e` as an element select of a packed-array field (`f[k]`, held as
    /// an index or as the select of the identifier's last segment): its base
    /// expression and index. None for anything else.
    pub(super) fn csp_split_last_select(e: &Expression) -> Option<(Expression, Expression)> {
        match &e.kind {
            ExprKind::Index { expr, index } => Some(((**expr).clone(), (**index).clone())),
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.len() >= 2
                    && h.path.last().is_some_and(|s| s.selects.len() == 1)
                    && h.path[..h.path.len() - 1]
                        .iter()
                        .all(|s| s.selects.is_empty()) =>
            {
                let mut bh = h.clone();
                let idx = bh.path.last_mut().unwrap().selects.pop().unwrap();
                bh.cached_signal_id = crate::ast::cache::SignalCache::new(None);
                bh.cached_resolved_name = crate::ast::cache::NameCache::new();
                Some((Expression::new(ExprKind::Ident(bh), e.span), idx))
            }
            _ => None,
        }
    }

    /// §7.4.5: `f[k]` with `f` a field that is a packed array of elements
    /// (`csp.field_elems`): Some((base, Some((lsb, width)))) for a constant
    /// `k` naming an element, Some((base, None)) for a position not known
    /// yet, None when `e` is no such select.
    fn csp_field_elem(
        &mut self,
        csp: &Csp,
        e: &Expression,
        konst: &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
    ) -> Option<(usize, Option<(u32, u32)>)> {
        if csp.field_elems.is_empty() {
            return None;
        }
        let (b, idx) = Self::csp_split_last_select(Self::unparen(e))?;
        let (v, lo, w, whole) = self.csp_slice_base_k(csp, &b, konst)?;
        if whole {
            return None;
        }
        let &(l, r, ew) = csp.field_elems.get(&(v, lo, w))?;
        let Some(k) = konst(self, &idx) else {
            return Some((v, None));
        };
        let off = if l >= r {
            (r..=l).contains(&k).then(|| k - r)
        } else {
            (l..=r).contains(&k).then(|| r - k)
        };
        Some((v, off.map(|o| (lo + o as u32 * ew, ew))))
    }

    /// The element positions and bit ranges of the packed-array field `b`
    /// names, if it is one: (base, [(index, lsb, width)]).
    #[allow(clippy::type_complexity)]
    pub(super) fn csp_field_elem_all(
        &mut self,
        csp: &Csp,
        b: &Expression,
        konst: &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
    ) -> Option<(usize, Vec<(i64, u32, u32)>)> {
        if csp.field_elems.is_empty() {
            return None;
        }
        let (v, lo, w, whole) = self.csp_slice_base_k(csp, b, konst)?;
        if whole {
            return None;
        }
        let &(l, r, ew) = csp.field_elems.get(&(v, lo, w))?;
        let ks: Vec<i64> = if l >= r {
            (r..=l).collect()
        } else {
            (l..=r).collect()
        };
        Some((
            v,
            ks.into_iter()
                .map(|k| {
                    let o = if l >= r { k - r } else { r - k };
                    (k, lo + o as u32 * ew, ew)
                })
                .collect(),
        ))
    }

    /// The element variable `e` names: `a[k]` with `a` a solver array and
    /// `k` constant (held as an index, or as the select of the identifier's
    /// last segment when another select follows: `a[k][7:0]`). While the
    /// constraints are scanned an index that is not constant stands for the
    /// first element (see `CSP_SCAN_ANY`).
    fn csp_elem_ref(
        &mut self,
        csp: &Csp,
        e: &Expression,
        konst: &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
    ) -> Option<usize> {
        if csp.arrays.is_empty() {
            return None;
        }
        if let Some((n, idx)) = Self::csp_struct_elem(csp, e) {
            let a = csp.arrays.get(&n)?;
            let v = match konst(self, &idx) {
                Some(i) => a.elems.iter().find(|x| x.0 == i)?.1,
                None if CSP_SCAN_ANY.with(|c| c.get()) => a.elems.first()?.1,
                None => return None,
            };
            return (!self.csp_var_no_slices(csp, v)).then_some(v);
        }
        let mut elem = |me: &mut Self, base: &Expression, idx: &Expression| -> Option<usize> {
            let n = me
                .csp_member(csp, base)
                .filter(|n| csp.arrays.contains_key(n))
                .or_else(|| me.csp_nd_row(csp, base, konst))?;
            let a = csp.arrays.get(&n)?;
            let v = match konst(me, idx) {
                Some(i) => a.elems.iter().find(|x| x.0 == i)?.1,
                None if CSP_SCAN_ANY.with(|c| c.get()) => a.elems.first()?.1,
                None => return None,
            };
            (!me.csp_var_no_slices(csp, v)).then_some(v)
        };
        match &e.kind {
            ExprKind::Index { expr, index } => elem(self, expr, index),
            // One select names an element of a 1-D array; several name an
            // element of a multi-dimensional one through its row (#519).
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.last().is_some_and(|s| !s.selects.is_empty())
                    && h.path[..h.path.len() - 1]
                        .iter()
                        .all(|s| s.selects.is_empty()) =>
            {
                let mut bh = h.clone();
                let idx = bh.path.last_mut().unwrap().selects.pop().unwrap();
                let base = Expression::new(ExprKind::Ident(bh), e.span);
                elem(self, &base, &idx)
            }
            _ => None,
        }
    }

    /// §18.4 (#519): the row of a multi-dimensional solver array that `e`
    /// names with constant indices (`a[i]` under a bound `i`, `a[i][j]`
    /// for a 3-D array): the base name followed by the indices, when a
    /// solver array of that name exists.
    /// The row of a multi-dimensional solver array an index chain with
    /// constant (or loop-bound) indices names (`a[1]`, `a[i]`).
    pub(super) fn csp_row_of(&mut self, csp: &Csp, e: &Expression, env: &Env) -> Option<String> {
        if !matches!(e.kind, ExprKind::Index { .. }) {
            return None;
        }
        let mut konst = |me: &mut Self, x: &Expression| -> Option<i64> {
            if !me.csp_free(csp, x, env) {
                return None;
            }
            me.csp_const(x, env)?.to_i64()
        };
        self.csp_nd_row(csp, e, &mut konst)
    }

    fn csp_nd_row(
        &mut self,
        csp: &Csp,
        e: &Expression,
        konst: &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
    ) -> Option<String> {
        if csp.nd_dims.is_empty() {
            return None;
        }
        let (base, consts) = self.csp_nd_prefix_k(csp, e, konst)?;
        if consts.is_empty() {
            return None;
        }
        let mut row = base;
        for c in consts {
            row.push_str(&format!("[{c}]"));
        }
        csp.arrays.contains_key(&row).then_some(row)
    }

    /// The multi-dimensional base `e` indexes and the constant indices
    /// applied to it, outermost first (`a[1][0]` → (`a`, [1, 0])).
    pub(super) fn csp_nd_prefix_k(
        &mut self,
        csp: &Csp,
        e: &Expression,
        konst: &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
    ) -> Option<(String, Vec<i64>)> {
        match &e.kind {
            ExprKind::Index { expr, index } => {
                let (b, mut cs) = self.csp_nd_prefix_k(csp, expr, konst)?;
                cs.push(konst(self, index)?);
                Some((b, cs))
            }
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.last().is_some_and(|s| !s.selects.is_empty())
                    && h.path[..h.path.len() - 1]
                        .iter()
                        .all(|s| s.selects.is_empty()) =>
            {
                let mut bh = h.clone();
                let sels = std::mem::take(&mut bh.path.last_mut().unwrap().selects);
                let base = Expression::new(ExprKind::Ident(bh), e.span);
                let (b, mut cs) = self.csp_nd_prefix_k(csp, &base, konst)?;
                for s in &sels {
                    cs.push(konst(self, s)?);
                }
                Some((b, cs))
            }
            _ => {
                let n = self.csp_member(csp, e)?;
                csp.nd_dims.contains_key(&n).then_some((n, Vec::new()))
            }
        }
    }

    /// The element variable a chain of constant indexes on a
    /// multi-dimensional solver array names (`a[1][0]`, `a[i][j]` under
    /// bound loop variables), without evaluating anything.
    pub(super) fn nd_chain_elem(&self, csp: &Csp, e: &Expression, env: &Env) -> Option<usize> {
        let mut consts: Vec<i64> = Vec::new();
        let mut cur = e;
        loop {
            match &cur.kind {
                ExprKind::Index { expr, index } => {
                    consts.push(Self::nd_const_index(index, env)?);
                    cur = expr;
                }
                ExprKind::Paren(i) => cur = i,
                _ => break,
            }
        }
        consts.reverse();
        let base = match &cur.kind {
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.last().is_some_and(|s| !s.selects.is_empty())
                    && h.path[..h.path.len() - 1]
                        .iter()
                        .all(|s| s.selects.is_empty()) =>
            {
                let mut bh = h.clone();
                let sels = std::mem::take(&mut bh.path.last_mut().unwrap().selects);
                let mut lead: Vec<i64> = Vec::new();
                for s in &sels {
                    lead.push(Self::nd_const_index(s, env)?);
                }
                lead.extend(consts);
                consts = lead;
                self.csp_member(csp, &Expression::new(ExprKind::Ident(bh), cur.span))?
            }
            _ => self.csp_member(csp, cur)?,
        };
        if !csp.nd_dims.contains_key(&base) || consts.is_empty() {
            return None;
        }
        let mut row = base;
        for c in &consts[..consts.len() - 1] {
            row.push_str(&format!("[{c}]"));
        }
        csp.arrays
            .get(&row)?
            .elems
            .iter()
            .find(|x| x.0 == consts[consts.len() - 1])
            .map(|x| x.1)
    }

    /// A constant index of a multi-dimensional element reference: a
    /// literal, or a loop variable the environment binds.
    pub(super) fn nd_const_index(x: &Expression, env: &Env) -> Option<i64> {
        match &x.kind {
            ExprKind::Paren(i) => Self::nd_const_index(i, env),
            ExprKind::Number(NumberLiteral::Integer { .. }) => {
                let v = crate::compiler::elaborate::const_eval_i64_with_params(x, None)?;
                Some(v)
            }
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => env
                .binds
                .iter()
                .rev()
                .find(|b| b.0 == h.path[0].name.name)
                .and_then(|b| b.1.to_i64()),
            // `a[i-1]`, `a[i+1]` beside `a[i]` in a `foreach`.
            ExprKind::Binary {
                op: op @ (BinaryOp::Add | BinaryOp::Sub),
                left,
                right,
            } => {
                let (l, r) = (
                    Self::nd_const_index(left, env)?,
                    Self::nd_const_index(right, env)?,
                );
                if *op == BinaryOp::Add {
                    l.checked_add(r)
                } else {
                    l.checked_sub(r)
                }
            }
            _ => None,
        }
    }

    /// §18.4: `a[k].m` (a flat path or a member of an element select) where
    /// the solve holds member `m` of the unpacked-struct elements of `a` as
    /// the array `a.m`: that name and the index `k`.
    pub(super) fn csp_struct_elem(csp: &Csp, e: &Expression) -> Option<(String, Expression)> {
        let strip_this = |path: &[crate::ast::expr::HierPathSegment]| -> Vec<crate::ast::expr::HierPathSegment> {
            match path.split_first() {
                Some((t, rest)) if t.name.name == "this" && t.selects.is_empty() => rest.to_vec(),
                _ => path.to_vec(),
            }
        };
        let (base, idx, member) = match &e.kind {
            ExprKind::MemberAccess { expr, member } => match &expr.kind {
                ExprKind::Index { expr: b, index } => {
                    let ExprKind::Ident(h) = &b.kind else {
                        return None;
                    };
                    let segs = strip_this(&h.path);
                    let [a] = segs.as_slice() else {
                        return None;
                    };
                    if h.root.is_some() || !a.selects.is_empty() {
                        return None;
                    }
                    (a.name.name.clone(), (**index).clone(), member.name.clone())
                }
                ExprKind::Ident(h) if h.root.is_none() => {
                    let segs = strip_this(&h.path);
                    let [a] = segs.as_slice() else {
                        return None;
                    };
                    let [k] = a.selects.as_slice() else {
                        return None;
                    };
                    (a.name.name.clone(), k.clone(), member.name.clone())
                }
                _ => return None,
            },
            ExprKind::Ident(h) if h.root.is_none() => {
                let segs = strip_this(&h.path);
                let [a, m] = segs.as_slice() else {
                    return None;
                };
                let [k] = a.selects.as_slice() else {
                    return None;
                };
                if !m.selects.is_empty() {
                    return None;
                }
                (a.name.name.clone(), k.clone(), m.name.name.clone())
            }
            _ => return None,
        };
        let n = format!("{}.{}", base, member);
        csp.arrays.contains_key(&n).then_some((n, idx))
    }

    /// A scalar that is not split (no select arithmetic for its shape).
    fn csp_var_no_slices(&self, csp: &Csp, v: usize) -> bool {
        csp.lsb.get(&v) == Some(&i64::MIN)
    }

    /// The structural identities: each split scalar equals the sum of its
    /// segments, each multi-segment select the sum of the segments it covers.
    pub(super) fn csp_slice_links(&mut self, csp: &mut Csp) {
        let mut links: Vec<Lin> = Vec::new();
        for (&v, segs) in &csp.segs {
            if csp.wide.contains(&v) {
                // No value of its own: it is its segments.
                continue;
            }
            let mut t: Vec<(usize, i128)> = vec![(v, 1)];
            for &(lo, _, x) in segs {
                t.push((x, -(1i128 << lo)));
            }
            links.push(Lin { t, k: 0 });
        }
        for (&(v, lo, w), &x) in &csp.slice_vars {
            let Some(segs) = csp.segs.get(&v) else {
                continue;
            };
            let mut t: Vec<(usize, i128)> = vec![(x, 1)];
            for &(slo, sw, sx) in segs {
                if slo >= lo && slo + sw <= lo + w {
                    t.push((sx, -(1i128 << (slo - lo))));
                }
            }
            links.push(Lin { t, k: 0 });
        }
        let mut pops: Vec<(usize, usize)> = csp.pop.iter().map(|(&x, &c)| (x, c)).collect();
        pops.sort_unstable();
        for (x, c) in pops {
            let w = csp.vars[x].width;
            csp.nodes.push(Node::Pop { x, c, w });
        }
        for mut lin in links {
            lin.t.sort_unstable_by_key(|x| x.0);
            let deps: Vec<usize> = lin.t.iter().map(|x| x.0).collect();
            let one = Self::literal_of(&Value::from_u64(1, 1), crate::ast::Span::dummy());
            let src = self.csp_src(csp, SrcItem::Expr(one), &Env::default(), deps);
            csp.nodes.push(Node::Lin {
                lin,
                rel: Rel::Eq,
                fits: Vec::new(),
                src,
                sneg: false,
                wrap: None,
            });
        }
    }

    /// Maximal runs of ones in the low `w` bits of `m`: (lsb, width).
    fn mask_runs(m: u64, w: u32) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        let mut i = 0u32;
        while i < w.min(64) {
            if (m >> i) & 1 == 1 {
                let start = i;
                while i < w.min(64) && (m >> i) & 1 == 1 {
                    i += 1;
                }
                out.push((start, i - start));
            } else {
                i += 1;
            }
        }
        out
    }

    /// `(x & M) == C` with constant `M`, `C` and `x` a scalar, field or
    /// select: each run of ones in `M` pins those bits of `x` to `C`'s, and a
    /// `C` bit outside `M` makes it unsatisfiable. None for another shape.
    pub(super) fn csp_masked_eq(
        &mut self,
        csp: &mut Csp,
        whole: &Expression,
        masked: &Expression,
        other: &Expression,
        env: &Env,
    ) -> Option<Node> {
        let ExprKind::Binary {
            op: BinaryOp::BitAnd,
            left,
            right,
        } = &Self::unparen(masked).kind
        else {
            return None;
        };
        if !self.csp_free(csp, other, env) {
            return None;
        }
        let c = self.csp_const(other, env)?.to_u64()?;
        for (x, m) in [(left, right), (right, left)] {
            if !self.csp_free(csp, m, env) {
                continue;
            }
            let Some(mask) = self.csp_const(m, env).and_then(|v| v.to_u64()) else {
                continue;
            };
            let mut konst = |me: &mut Self, y: &Expression| -> Option<i64> {
                if !me.csp_free(csp, y, env) {
                    return None;
                }
                me.csp_const(y, env)?.to_i64()
            };
            let Some((v, lo, w, _)) = self.csp_slice_base_k(csp, x, &mut konst) else {
                continue;
            };
            let wm = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
            let mask = mask & wm;
            if c & !mask != 0 {
                return Some(Node::False);
            }
            let mut nodes = Vec::new();
            for (rl, rw) in Self::mask_runs(mask, w) {
                let (blo, bw) = (lo + rl, rw);
                let var = if blo == 0 && bw == csp.vars[v].width {
                    Some(v)
                } else {
                    csp.segs.get(&v).and_then(|segs| {
                        segs.iter()
                            .find(|s| s.0 == blo && s.1 == bw)
                            .map(|s| s.2)
                            .or_else(|| csp.slice_vars.get(&(v, blo, bw)).copied())
                    })
                };
                let var = var?;
                let want = ((c >> rl) & if rw >= 64 { u64::MAX } else { (1u64 << rw) - 1 }) as i128;
                let src = self.csp_src(csp, SrcItem::Expr(whole.clone()), env, vec![var]);
                nodes.push(Node::Lin {
                    lin: Lin {
                        t: vec![(var, 1)],
                        k: -want,
                    },
                    rel: Rel::Eq,
                    fits: Vec::new(),
                    src,
                    sneg: false,
                    wrap: None,
                });
            }
            return Some(Self::csp_and(nodes));
        }
        None
    }

    /// `(x & M) == C` where `M` or `C` reads other variables, such as
    /// `(addr & ((1 << m) - 1)) == ((1 << m) - 1)`: the evaluator judges it
    /// once `x` is fixed, and a uniform `x` meets `k` pinned bits one time in
    /// 2**k. Noted in `Csp::masks` so the value of `x` is drawn with those
    /// bits set (`csp_mask_draw`).
    pub(super) fn csp_note_mask(
        &mut self,
        csp: &mut Csp,
        whole: &Expression,
        masked: &Expression,
        other: &Expression,
        env: &Env,
    ) {
        if env.mask_note == Some(false) {
            return;
        }
        let ExprKind::Binary {
            op: BinaryOp::BitAnd,
            left,
            right,
        } = &Self::unparen(masked).kind
        else {
            return;
        };
        for (x, m) in [(left, right), (right, left)] {
            let mut konst = |me: &mut Self, y: &Expression| -> Option<i64> {
                if !me.csp_free(csp, y, env) {
                    return None;
                }
                me.csp_const(y, env)?.to_i64()
            };
            let Some((var, lo, w, _)) = self.csp_slice_base_k(csp, x, &mut konst) else {
                continue;
            };
            // §5.7.1: a `'0`/`'1` takes the comparison's width. One that is
            // a whole operand gets it in `csp_mask_draw`; a nested one would
            // be evaluated at its own width.
            let nested = |e: &Expression| Self::csp_fill(e).is_none() && Self::csp_has_fill(e);
            if nested(m) || nested(other) {
                return;
            }
            let mut deps = Vec::new();
            if !self.csp_refs(csp, m, env, &mut deps) || !self.csp_refs(csp, other, env, &mut deps)
            {
                return;
            }
            deps.sort_unstable();
            deps.dedup();
            let base_of = |d: usize| csp.aux_base.get(&d).map_or(d, |b| b.0);
            if deps.is_empty()
                || deps.iter().any(|&d| base_of(d) == var)
                || csp.vars[var].signed
                || csp.vars[var].width > 64
            {
                return;
            }
            let src = self.csp_src(csp, SrcItem::Expr(whole.clone()), env, deps.clone());
            csp.masks.push(MaskHint {
                var,
                lo,
                w,
                mask: (**m).clone(),
                want: other.clone(),
                deps,
                src,
                guards: env.guards.clone(),
            });
            return;
        }
    }

    /// The digit of `e` when it is an unbased unsized literal (`'1`).
    fn csp_fill(e: &Expression) -> Option<char> {
        match &Self::unparen(e).kind {
            ExprKind::Number(crate::ast::expr::NumberLiteral::UnbasedUnsized(c)) => Some(*c),
            _ => None,
        }
    }

    /// Whether `e` may contain an unbased unsized literal (`'0`, `'1`, ...).
    /// A form not walked here counts as containing one.
    fn csp_has_fill(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Number(n) => {
                matches!(n, crate::ast::expr::NumberLiteral::UnbasedUnsized(_))
            }
            ExprKind::Ident(_) => false,
            ExprKind::Index { expr, index } => {
                Self::csp_has_fill(expr) || Self::csp_has_fill(index)
            }
            ExprKind::Paren(x) | ExprKind::Unary { operand: x, .. } => Self::csp_has_fill(x),
            ExprKind::Binary { left, right, .. } => {
                Self::csp_has_fill(left) || Self::csp_has_fill(right)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => [condition, then_expr, else_expr]
                .iter()
                .any(|x| Self::csp_has_fill(x)),
            ExprKind::Concatenation(xs) => xs.iter().any(Self::csp_has_fill),
            _ => true,
        }
    }

    /// A value of `v` drawn uniformly from its domain with the bits that the
    /// `Csp::masks` in force pin set to their values. In force: every guard
    /// holds and `M` and `C` are fixed. None when no mask is in force or no
    /// draw lands in the domain.
    pub(super) fn csp_mask_draw(&mut self, csp: &Csp, st: &St, v: usize) -> Option<i128> {
        if csp.masks.is_empty() {
            return None;
        }
        let (base, off, width) = csp
            .aux_base
            .get(&v)
            .copied()
            .unwrap_or((v, 0, csp.vars[v].width));
        if width > 64 || csp.vars[v].signed {
            return None;
        }
        let ones = |w: u32| if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let (mut pin, mut val) = (0u64, 0u64);
        for h in csp.masks.iter().filter(|h| h.var == base) {
            if !st.all_fixed(&h.deps)
                || !h
                    .guards
                    .iter()
                    .all(|g| self.csp_status(csp, g, st) == Some(true))
            {
                continue;
            }
            let val_of = |me: &mut Self, e: &Expression| match Self::csp_fill(e) {
                Some('0') => Some(0),
                Some('1') => Some(ones(h.w)),
                Some(_) => None,
                None => me.csp_eval_in(csp, st, h.src, e).to_u64(),
            };
            let m = val_of(self, &h.mask)? & ones(h.w);
            let c = val_of(self, &h.want)?;
            if c & !m != 0 {
                return None;
            }
            let (m, c) = (m.checked_shl(h.lo)?, c.checked_shl(h.lo)?);
            let (m, c) = ((m >> off) & ones(width), (c >> off) & ones(width));
            if (val ^ c) & pin & m != 0 {
                return None;
            }
            pin |= m;
            val = (val & !m) | (c & m);
        }
        if pin == 0 {
            return None;
        }
        // The values in 0..=n whose pinned bits equal `val`.
        let upto = |n: i128| -> u128 {
            if n < 0 {
                return 0;
            }
            let n = n as u64;
            let mut c = 0u128;
            for b in (0..64).rev() {
                let free_below = (!pin & ones(b)).count_ones();
                let pinned = pin >> b & 1 == 1;
                if n >> b & 1 == 1 {
                    if !pinned || val >> b & 1 == 0 {
                        c += 1u128 << free_below;
                    }
                    if pinned && val >> b & 1 == 0 {
                        return c;
                    }
                } else if pinned && val >> b & 1 == 1 {
                    return c;
                }
            }
            c + 1
        };
        let d = &st.d[v];
        let n: u128 = d.iter().map(|&(l, h)| upto(h) - upto(l - 1)).sum();
        if n == 0 {
            return None;
        }
        let mut k = self.cur_rng().gen_range(0..n);
        for &(l, h) in d {
            let base = upto(l - 1);
            let here = upto(h) - base;
            if k >= here {
                k -= here;
                continue;
            }
            let (mut lo, mut hi) = (l, h);
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if upto(mid) - base > k {
                    hi = mid;
                } else {
                    lo = mid + 1;
                }
            }
            return Some(lo);
        }
        None
    }

    /// The value of `e` (a part of source `src`'s constraint) with the fixed
    /// variables written and the source's foreach indices bound.
    fn csp_eval_in(&mut self, csp: &Csp, st: &St, src: usize, e: &Expression) -> Value {
        let s = &csp.srcs[src];
        for &v in &s.deps {
            if let Some(x) = st.fixed(v) {
                self.csp_write(csp, v, x);
            }
        }
        let frame = self.csp_frame(&s.binds);
        self.push_local_frame(frame);
        let v = self.eval_expr(e);
        self.pop_local_frame();
        v
    }

    /// A select or field of a split scalar as its solver variable.
    pub(super) fn csp_slice_ae(&mut self, csp: &Csp, e: &Expression, env: &Env) -> Option<Ae> {
        let mut konst = |me: &mut Self, x: &Expression| -> Option<i64> {
            if !me.csp_free(csp, x, env) {
                return None;
            }
            me.csp_const(x, env)?.to_i64()
        };
        let (v, r) = self.csp_select_ref(csp, e, &mut konst)?;
        let (lo, w) = r?;
        if lo == 0 && w == csp.vars[v].width {
            // `v[31:8]` of `logic [31:8] v`: the whole variable.
            return Some(Self::csp_var_ae(csp, v));
        }
        let segs = csp.segs.get(&v)?;
        let a = if let Some(&(_, _, x)) = segs.iter().find(|s| s.0 == lo && s.1 == w) {
            Ae::Var(x, w, false)
        } else if let Some(&x) = csp.slice_vars.get(&(v, lo, w)) {
            Ae::Var(x, w, false)
        } else {
            // §11.5.1: a field or select can span several segments when
            // another constraint selects bits inside it. Reassemble all
            // its bits so arithmetic relations still propagate through it.
            if w > 64 {
                return None;
            }
            let mut end = lo;
            let mut terms = Vec::new();
            for &(slo, sw, x) in segs {
                if slo < lo || slo >= lo + w {
                    continue;
                }
                if slo != end || slo + sw > lo + w {
                    return None;
                }
                terms.push(Ae::Shl(
                    Box::new(Ae::Var(x, sw, false)),
                    u64::from(slo - lo),
                ));
                end += sw;
            }
            if end != lo + w {
                return None;
            }
            Ae::Sum { terms, w, s: false }
        };
        // §7.2.1, §11.5.1: a signed member reads its bits signed (`p.s <
        // 0` of a `bit signed [3:0] s`); a bit or part select stays
        // unsigned.
        if Self::csp_signed_field(csp, e, v, lo, w) {
            return Some(Ae::Cast {
                inner: Box::new(a),
                iw: w,
                is: false,
                w,
                s: true,
            });
        }
        Some(a)
    }

    /// Whether `e`, selecting bits `[lo +: w]` of `v`, names a signed
    /// member of it (`csp_signed_members`) rather than a select of bits.
    fn csp_signed_field(csp: &Csp, e: &Expression, v: usize, lo: u32, w: u32) -> bool {
        let leaf = match &Self::unparen(e).kind {
            ExprKind::MemberAccess { member, .. } => member.name.as_str(),
            ExprKind::Ident(h)
                if h.path.len() >= 2 && h.path.iter().all(|s| s.selects.is_empty()) =>
            {
                h.path.last().map_or("", |s| s.name.name.as_str())
            }
            _ => return false,
        };
        csp.fields.get(&v).is_some_and(|fs| {
            fs.iter().any(|(n, flo, fw)| {
                (*flo, *fw) == (lo, w)
                    && (n == leaf || n.strip_suffix(leaf).is_some_and(|p| p.ends_with('.')))
                    && csp.signed_fields.contains(&(v, n.clone()))
            })
        })
    }
}
