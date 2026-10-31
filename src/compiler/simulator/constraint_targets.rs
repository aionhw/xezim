//! Simulator constraint targets.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// §18.4.3 Compute the domain of a randc property: every value the
    /// variable could possibly take, intersected with any `inside` ranges
    /// from constraints. For enum-typed props the domain is the enum
    /// members; for bit/integer types it's `0 .. 2^width - 1`. A wider type
    /// is enumerated only when an `inside` range narrows it; otherwise it
    /// cycles through `pick_randc_wide` (empty domain here).
    pub(super) fn randc_domain(
        &self,
        _name: &str,
        width: u32,
        enum_type: Option<&str>,
        ranges: Option<&[(i128, i128)]>,
    ) -> Vec<u64> {
        let base: Vec<u64> = if let Some(et) = enum_type {
            self.module
                .enum_members
                .get(et)
                .map(|m| m.iter().map(|x| x.1).collect())
                .unwrap_or_default()
        } else if width > 16
            && let Some(rs) = ranges
            && rs.iter().map(|(lo, hi)| (hi - lo + 1).max(0)).sum::<i128>() <= 1 << 16
        {
            // A wide randc held to a narrow `inside` range: enumerate the
            // range itself (the counter walk of `pick_randc_wide` would
            // skip most of the width per value).
            let max = (1i128 << width.min(64)) - 1;
            let mut vs: Vec<u64> = rs
                .iter()
                .flat_map(|&(lo, hi)| lo.max(0)..=hi.min(max))
                .map(|v| v as u64)
                .collect();
            vs.sort_unstable();
            vs.dedup();
            return vs;
        } else if width > 0 && width <= 16 {
            let max = if width >= 64 {
                u64::MAX
            } else {
                (1u64 << width) - 1
            };
            (0..=max).collect()
        } else {
            return Vec::new();
        };
        if let Some(rs) = ranges {
            base.into_iter()
                .filter(|v| {
                    rs.iter()
                        .any(|(lo, hi)| (*v as i128) >= *lo && (*v as i128) <= *hi)
                })
                .collect()
        } else {
            base
        }
    }

    /// Pick a value from `domain` not yet drawn in the current randc cycle
    /// for (handle, name); record it TENTATIVELY (see `randc_pending`). When
    /// every value has been drawn the used-set is reset and a fresh cycle
    /// begins.
    ///
    /// §18.4: "randc variables are cyclic — they iterate over all the values
    /// in the random set before repeating any value". The solver may retry a
    /// trial many times before a solution satisfies the other constraints; a
    /// retried (or abandoned) trial must NOT consume a value out of the
    /// permutation cycle, or the cycle stops being a permutation. So a pick
    /// made earlier in this same randomize() call is rolled back first.
    pub(super) fn pick_randc(&mut self, handle: usize, name: &str, domain: &[u64]) -> u64 {
        use rand::Rng;
        let key = (handle, name.to_string());
        // Undo this call's earlier tentative pick for the same variable.
        self.randc_rollback(&key);
        let used = self.randc_used.entry(key.clone()).or_default();
        let mut avail: Vec<u64> = domain
            .iter()
            .copied()
            .filter(|v| !used.contains(v))
            .collect();
        // `prev` is only Some when this pick wraps the cycle, so the rollback
        // of a failed trial can put the exhausted cycle back.
        let mut prev: Option<HashSet<u64>> = None;
        if avail.is_empty() {
            prev = Some(used.clone());
            used.clear();
            avail = domain.to_vec();
        }
        let pick = avail[self.cur_rng().gen_range(0..avail.len())];
        self.randc_used.get_mut(&key).unwrap().insert(pick);
        self.randc_pending.insert(key, (pick, prev));
        pick
    }

    /// §18.4.2: the next value of a 17- to 32-bit randc cycle, by a keyed
    /// bijection over `[0, 2^width)` and a counter: every value once per
    /// cycle, a fresh key (from the object's generator) per cycle. Values
    /// outside `ranges` are skipped (a narrow range is enumerated exactly by
    /// `randc_domain` instead). Recorded tentatively, like `pick_randc`.
    /// None when no value of the width lies in the ranges.
    pub(super) fn pick_randc_wide(
        &mut self,
        handle: usize,
        name: &str,
        width: u32,
        ranges: Option<&[(i128, i128)]>,
    ) -> Option<u64> {
        use rand::Rng;
        let key = (handle, name.to_string());
        self.randc_rollback(&key);
        let size = 1u64 << width;
        let state = self.randc_wide.get(&key).copied();
        let (mut perm_key, mut next) = match state {
            Some((k, n, w)) if w == width => (k, n),
            _ => (self.cur_rng().r#gen::<u64>(), 0),
        };
        self.randc_wide_pending.insert(key.clone(), state);
        let mut steps = 0u64;
        loop {
            if next >= size {
                perm_key = self.cur_rng().r#gen::<u64>();
                next = 0;
            }
            let v = Self::randc_permute(next, perm_key, width);
            next += 1;
            steps += 1;
            if ranges.is_none_or(|rs| {
                rs.iter()
                    .any(|&(lo, hi)| (v as i128) >= lo && (v as i128) <= hi)
            }) {
                self.randc_wide.insert(key, (perm_key, next, width));
                return Some(v);
            }
            if steps > size {
                self.randc_rollback(&key);
                return None;
            }
        }
    }

    /// A bijection of `[0, 2^width)` chosen by `key`: xor with the key,
    /// odd multiplications and xor-shifts, each invertible modulo 2^width.
    fn randc_permute(x: u64, key: u64, width: u32) -> u64 {
        let mask = if width >= 64 {
            u64::MAX
        } else {
            (1u64 << width) - 1
        };
        let half = width.div_ceil(2).max(1);
        let third = (width / 3).max(1);
        let mut v = (x ^ key) & mask;
        v = v.wrapping_mul(0x9E37_79B9_7F4A_7C15) & mask;
        v ^= v >> half;
        v = v.wrapping_mul(0xBF58_476D_1CE4_E5B9) & mask;
        v ^= (key >> 32) & mask;
        v ^= v >> third;
        v = v.wrapping_mul(0x94D0_49BB_1331_11EB) & mask;
        v ^= v >> half;
        v
    }

    pub(super) fn randc_rollback(&mut self, key: &(usize, String)) {
        if let Some(prev) = self.randc_wide_pending.remove(key) {
            match prev {
                Some(st) => {
                    self.randc_wide.insert(key.clone(), st);
                }
                None => {
                    self.randc_wide.remove(key);
                }
            }
        }
        if let Some((pick, prev)) = self.randc_pending.remove(key) {
            if let Some(p) = prev {
                self.randc_used.insert(key.clone(), p);
            } else if let Some(u) = self.randc_used.get_mut(key) {
                u.remove(&pick);
            }
        }
    }

    pub(super) fn randc_rollback_all(&mut self) {
        let mut keys: Vec<(usize, String)> = self.randc_pending.keys().cloned().collect();
        keys.extend(self.randc_wide_pending.keys().cloned());
        for k in &keys {
            self.randc_rollback(k);
        }
    }

    // =====================================================================
    // IEEE 1800-2017 §18.4 / §18.5.9 / §18.5.10 / §18.5.11 — randomization of
    // COLLECTION members (dynamic arrays, queues, associative arrays, and
    // dynamic arrays of dynamic arrays), `unique {}` over them, cross-object
    // (`rand` handle) constraints, static constraint blocks, and `solve …
    // before …` variable ordering.
    //
    // Storage model (matches the rest of the simulator): a collection member
    // `m` of the object at `h` lives in `self.signals` under the scoped names
    //     <h>#m.size          element count (dynamic array / queue)
    //     <h>#m[i]            element i
    //     <h>#m[i].size       element count of the inner array (2-D dynamic)
    //     <h>#m[i][j]         element j of inner array i
    // Associative arrays are sparse: only the populated `<h>#m[key]` keys
    // exist (there is no `.size` shadow).
    // =====================================================================

    /// §18.5.11: when `name` is a STATIC constraint block of `handle`'s class
    /// chain, return the class that DECLARES it. A static constraint block is
    /// shared by every instance, so its `constraint_mode()` state is one
    /// class-wide flag rather than a per-instance one.
    pub(super) fn static_constraint_owner(&self, handle: usize, name: &str) -> Option<String> {
        let mut cur = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone());
        while let Some(cn) = cur {
            let cd = self.module.classes.get(&cn)?;
            if let Some(c) = cd.constraints.get(name) {
                return if c.is_static { Some(cn) } else { None };
            }
            cur = cd.extends.clone();
        }
        None
    }

    /// §18.5.11 helper shared by both `constraint_mode` receiver shapes:
    /// route a static constraint block's mode to the class-wide flag.
    /// Returns `Some(query_result)` when the target was a static block.
    pub(super) fn static_constraint_mode(
        &mut self,
        handle: usize,
        target: Option<&str>,
        enable: Option<bool>,
    ) -> Option<Value> {
        let name = target?;
        let owner = self.static_constraint_owner(handle, name)?;
        let key = (owner, name.to_string());
        match enable {
            None => {
                let disabled = self.static_constraint_disabled.contains(&key);
                Some(Value::from_u64(if disabled { 0 } else { 1 }, 32))
            }
            Some(true) => {
                self.static_constraint_disabled.remove(&key);
                Some(Value::zero(32))
            }
            Some(false) => {
                self.static_constraint_disabled.insert(key);
                Some(Value::zero(32))
            }
        }
    }

    /// Declared class name of a class property, if the property is a class
    /// HANDLE (`rand SubObject sub_inst;`). §18.5.9/§18.6: a rand handle is
    /// not a random *value* — the object it points at is randomized, and the
    /// enclosing object's constraints may refer to its members.
    pub(super) fn prop_class_type(&self, handle: usize, prop: &str) -> Option<String> {
        self.prop_class_binding(handle, prop)
            .filter(|t| self.module.classes.contains_key(t))
    }

    /// `prop_class_type`, also answering a binding that names a specialized
    /// class (`P#(7)`), which is no key of the class table.
    pub(super) fn prop_class_or_spec_type(&self, handle: usize, prop: &str) -> Option<String> {
        self.prop_class_binding(handle, prop).filter(|t| {
            self.module.classes.contains_key(t)
                || self
                    .extract_spec_from_string(t)
                    .is_some_and(|(b, _)| self.module.classes.contains_key(&b))
        })
    }

    /// The declared type of the class property `prop` of object `handle`,
    /// with a type parameter bound through the object (§8.25).
    fn prop_class_binding(&self, handle: usize, prop: &str) -> Option<String> {
        let mut cur = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone());
        while let Some(cn) = cur {
            let cd = self.module.classes.get(&cn)?;
            if let Some(sig) = cd.properties.get(prop) {
                let mut tn = sig.type_name.clone();
                // A member whose declared type is a class TYPE PARAMETER
                // (e.g. `REQ req` in `uvm_sequence #(trans)`) is not itself a
                // registered class — `classes.contains_key` below would drop
                // it. Resolve the type param to its concrete binding on the
                // owning instance where possible.
                if let Some(raw) = &tn {
                    let owner = self.heap.get(handle).and_then(|o| o.as_ref());
                    let inst_binding = owner.and_then(|i| i.type_bindings.get(raw).cloned());
                    // Resolve a member type param against the instance's OWN
                    // class specialization FIRST (its concrete `extends #(...)`
                    // args), then the ambient `current_spec`. `current_spec` is
                    // a single mutable field that concurrent forks leave set to
                    // whatever method ran last — e.g. a bot sequencer while a
                    // top sequence's `req` is being resolved — so it must not
                    // outrank the instance's own chain. `ancestor_type_param_`
                    // binding walks `owner.class_name` up to the declaring class
                    // and is authoritative for a live instance.
                    // The owner's own specialization (`W #(B) wb`) comes
                    // before the ambient one: inside a method of `W #(A)`,
                    // `other.h` of a `W #(B)` is a `B` (#596).
                    let own_spec = owner.and_then(|i| i.spec.clone()).filter(|(b, _)| *b == cn);
                    let resolved = inst_binding
                        .clone()
                        .or_else(|| self.ancestor_type_param_binding(handle, &cn, raw))
                        .or_else(|| {
                            own_spec
                                .is_some()
                                .then(|| self.resolve_type_param_with(raw, &own_spec))
                                .flatten()
                        })
                        .or_else(|| self.resolve_type_param_binding(raw));
                    if let Some(r) = resolved {
                        tn = Some(r);
                    }
                }
                return tn;
            }
            cur = cd.extends.clone();
        }
        None
    }

    /// The class a member assignment target (`o.h`, `this.h`, or an element
    /// `o.arr[0]` of a collection of handles) holds, with a type parameter
    /// bound by the object that owns the member (§8.25).
    pub(super) fn member_target_class(&self, lvalue: &Expression) -> Option<String> {
        let mut lvalue = lvalue;
        let mut indexed = false;
        while let ExprKind::Index { expr, .. } = &lvalue.kind {
            lvalue = expr;
            indexed = true;
        }
        let (owner, prop) = match &lvalue.kind {
            ExprKind::MemberAccess { expr, member } => {
                (self.eval_handle_expr(expr)?, member.name.as_str())
            }
            ExprKind::Ident(h)
                if h.path.len() == 2 && h.path.iter().all(|s| s.selects.is_empty()) =>
            {
                (
                    self.eval_ident_handle(&h.path[0].name.name)?,
                    h.path[1].name.name.as_str(),
                )
            }
            _ => return None,
        };
        if owner == 0 {
            return None;
        }
        // A collection of handles names a class only through an element
        // (`o.arr[0] = new()`); the member itself takes `new[n]`.
        let mut cur = self
            .heap
            .get(owner)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone());
        while let Some(cn) = cur {
            let cd = self.module.classes.get(&cn)?;
            if cd.properties.contains_key(prop) {
                let coll = cd.array_properties.contains_key(prop)
                    || cd.array_nd_properties.contains_key(prop)
                    || cd.queue_properties.contains_key(prop)
                    || cd.assoc_properties.contains_key(prop);
                if coll != indexed {
                    return None;
                }
                break;
            }
            cur = cd.extends.clone();
        }
        self.prop_class_or_spec_type(owner, prop)
    }

    /// Class-chain member-type lookup without a live instance handle (used
    /// when the receiver's base object is null but its DECLARED class is
    /// known). Mirrors `prop_class_type`; type params resolve via current_spec
    /// or the owner handle when supplied.
    pub(super) fn prop_class_type_from_class(
        &self,
        cls: &str,
        prop: &str,
        owner: Option<usize>,
    ) -> Option<String> {
        let mut cur: Option<&str> = Some(cls);
        while let Some(cn) = cur {
            let cd = self.module.classes.get(cn)?;
            if let Some(sig) = cd.properties.get(prop) {
                let mut tn = sig.type_name.clone();
                if let Some(raw) = &tn {
                    let resolved = owner
                        .and_then(|h| self.ancestor_type_param_binding(h, &cn, raw))
                        .or_else(|| self.resolve_type_param_binding(raw));
                    if let Some(r) = resolved {
                        tn = Some(r);
                    }
                }
                return tn.filter(|t| self.module.classes.contains_key(t));
            }
            cur = cd.extends.as_deref();
        }
        None
    }

    /// Bind a class TYPE PARAMETER (`raw`) declared by an ANCESTOR class
    /// (`declaring_class`) for an instance whose concrete specialization comes
    /// from its OWN `extends` clause. `uvm_sequence #(REQ)` declares `REQ req`;
    /// a derived `my_seq extends seq_base #(trans)` must bind `REQ -> trans`
    /// even when no `current_spec` is active (e.g. inside an inlined body).
    /// Walks the instance's class up to `declaring_class`, substituting each
    /// `extends #(...)` argument positionally onto the parent's params.
    pub(super) fn ancestor_type_param_binding(
        &self,
        handle: usize,
        declaring_class: &str,
        raw: &str,
    ) -> Option<String> {
        let owner = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone())?;
        let mut binds: HashMap<String, String> = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|i| i.type_bindings.clone())
            .unwrap_or_default()
            .into_iter()
            .collect();
        self.ancestor_type_param_walk(&owner, declaring_class, raw, &mut binds)
    }

    pub(super) fn ancestor_type_param_walk(
        &self,
        cur: &str,
        target: &str,
        raw: &str,
        binds: &mut HashMap<String, String>,
    ) -> Option<String> {
        let cd = self.module.classes.get(cur)?;
        let parent = cd.extends.clone()?;
        let pcd = self.module.classes.get(&parent)?;
        let mut nb = binds.clone();
        for (i, pname) in pcd.param_order.iter().enumerate() {
            if let Some(arg) = cd.extends_type_args.get(i) {
                let arg = arg.trim();
                if arg.is_empty() {
                    continue;
                }
                let sub = if Self::spec_projection(arg).is_some() {
                    match Self::carry_extends_arg(arg, &|p| binds.get(p).cloned()) {
                        Some(v) => v,
                        None => continue,
                    }
                } else {
                    nb.get(arg).cloned().unwrap_or_else(|| arg.to_string())
                };
                nb.insert(pname.clone(), sub);
            }
        }
        if parent == target {
            nb.get(raw).cloned().filter(|s| !s.is_empty())
        } else {
            self.ancestor_type_param_walk(&parent, target, raw, &mut nb)
        }
    }

    /// §18.11/§18.13: the members of one class that this `randomize()`
    /// draws — its `rand`/`randc` members whose rand_mode is on or, for
    /// `randomize(a, b)`, exactly the members named, declared `rand` or not
    /// (the argument list overrides rand_mode for the call).
    pub(super) fn randomize_members(
        &self,
        cd: &crate::compiler::elaborate::ElaboratedClass,
        rand_disabled: &HashSet<String>,
    ) -> Vec<String> {
        match &self.randomize_subset {
            Some(sub) => {
                let mut out: Vec<String> = cd
                    .random_properties
                    .iter()
                    .filter(|p| sub.contains(*p))
                    .cloned()
                    .collect();
                out.extend(
                    cd.property_order
                        .iter()
                        .filter(|p| sub.contains(*p) && !cd.random_properties.contains(*p))
                        .cloned(),
                );
                out
            }
            None if rand_disabled.contains("*") => Vec::new(),
            None => cd
                .random_properties
                .iter()
                .filter(|p| !rand_disabled.contains(*p))
                .cloned()
                .collect(),
        }
    }

    /// Every rand collection member of `handle`'s class chain (those
    /// `randomize_members` draws), classified by kind. Fixed unpacked arrays
    /// are included so `unique {}` / strict checking can see them, but their
    /// element seeding stays with the legacy enum-pool pass.
    pub(super) fn collect_rand_colls(
        &self,
        handle: usize,
        rand_disabled: &HashSet<String>,
        constraints: &[ClassConstraint],
    ) -> Vec<RandColl> {
        let mut out: Vec<RandColl> = Vec::new();
        let mut seen: HashSet<String> = HashSet::default();
        let mut cur = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone());
        while let Some(cn) = cur {
            let Some(cd) = self.module.classes.get(&cn) else {
                break;
            };
            for prop in &self.randomize_members(cd, rand_disabled) {
                if !seen.insert(prop.clone()) {
                    continue;
                }
                let scoped = format!("{}#{}", handle, prop);
                // The element type of a collection member: for `obj arr[]` this
                // is the class name `obj`, so the elements are object handles
                // (randomized recursively, never as scalars). For scalar/
                // packed element types `type_name` is None or a non-class.
                // §8.25: an element type that is a class type parameter is a
                // class when this object binds the parameter to one.
                let is_object_elem = self.prop_class_type(handle, prop).is_some();
                // §8.25/§6.20.2: the element width in this object's
                // specialization. The class tables hold the default's, so a
                // `rand T beats[]` bound wider than T's default solved its
                // elements at the default width and a field select of the
                // bound type fell outside them (#576).
                let spec_w = self.spec_member_elem_width(cd, handle, prop);
                // §18.4 (#519): a multi-dimensional member joins as one row
                // per prefix of its leading dimensions, each keyed like a
                // 1-D array (`m[0]`, elements `m[0][j]`), with the base's
                // bounds carried for the `foreach` levels above the row.
                if cd.array_nd_properties.contains_key(prop) {
                    let dims: Option<Vec<(i64, i64)>> =
                        if let Some(&(d0, d1, _)) = self.module.arrays_2d.get(&scoped) {
                            Some(vec![d0, d1])
                        } else {
                            self.module.arrays_nd.get(&scoped).map(|(sh, _)| sh.clone())
                        };
                    let w = self
                        .module
                        .arrays_2d
                        .get(&scoped)
                        .map(|t| t.2)
                        .or_else(|| self.module.arrays_nd.get(&scoped).map(|t| t.1))
                        .or(spec_w)
                        .unwrap_or(32)
                        .max(1);
                    if let Some(dims) = dims
                        && dims.len() >= 2
                    {
                        let (lo, hi) = dims[dims.len() - 1];
                        let (lo, hi) = (lo.min(hi), lo.max(hi));
                        let mut prefixes: Vec<String> = vec![String::new()];
                        for &(l, r) in &dims[..dims.len() - 1] {
                            let mut next = Vec::new();
                            for p in &prefixes {
                                for i in l.min(r)..=l.max(r) {
                                    next.push(format!("{p}[{i}]"));
                                }
                            }
                            prefixes = next;
                        }
                        for p in prefixes {
                            out.push(RandColl {
                                prop: format!("{prop}{p}"),
                                scoped: format!("{scoped}{p}"),
                                kind: CollKind::Fixed,
                                width: w,
                                lo,
                                hi,
                                nested: false,
                                is_object_elem,
                                nd_base: Some((prop.clone(), dims.clone())),
                                row_tail: None,
                            });
                        }
                    }
                    continue;
                }
                if let Some(&(lo, hi, w)) = cd.array_properties.get(prop) {
                    let (lo, hi) = self.inst_fixed_bounds(cd, handle, prop, (lo, hi));
                    out.push(RandColl {
                        prop: prop.clone(),
                        scoped,
                        kind: CollKind::Fixed,
                        width: spec_w.unwrap_or(w).max(1),
                        lo,
                        hi,
                        nested: false,
                        is_object_elem,
                        nd_base: None,
                        row_tail: None,
                    });
                } else if cd.assoc_properties.contains_key(prop) {
                    let w = spec_w
                        .or_else(|| cd.properties.get(prop).map(|s| s.width))
                        .unwrap_or(32)
                        .max(1);
                    out.push(RandColl {
                        prop: prop.clone(),
                        scoped,
                        kind: CollKind::Assoc,
                        width: w,
                        lo: 0,
                        hi: 0,
                        nested: false,
                        is_object_elem,
                        nd_base: None,
                        row_tail: None,
                    });
                } else if let Some(&(w, _cap)) = cd.queue_properties.get(prop) {
                    // §7.4.5: dynamic rows are sized row by row (`nested`).
                    let row_tail = self.dyn_row_tail(cd, handle, prop);
                    let nested = matches!(row_tail, Some(RowTail::Dyn))
                        || Self::coll_is_nested(prop, constraints);
                    out.push(RandColl {
                        prop: prop.clone(),
                        scoped,
                        kind: CollKind::Dyn,
                        width: spec_w.unwrap_or(w).max(1),
                        lo: 0,
                        hi: 0,
                        nested,
                        is_object_elem,
                        nd_base: None,
                        row_tail,
                    });
                }
            }
            cur = cd.extends.clone();
        }
        out
    }

    /// A dynamic array is 2-D (`bit [7:0] m[][]`) when the constraints iterate
    /// it with two loop variables (`foreach (m[i, j])`) or size its rows
    /// (`foreach (m[i]) m[i].size() == …`). Class elaboration only records the
    /// OUTER unpacked dimension, so the constraint set is the shape oracle.
    pub(super) fn coll_is_nested(prop: &str, constraints: &[ClassConstraint]) -> bool {
        fn base_name(e: &Expression) -> Option<String> {
            match &e.kind {
                ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.clone()),
                ExprKind::Index { expr, .. } => base_name(expr),
                ExprKind::MemberAccess { expr, .. } => base_name(expr),
                _ => None,
            }
        }
        // A `.size()` call whose receiver is an ELEMENT of `prop` (`prop[i]`).
        fn row_size_call(e: &Expression, prop: &str) -> bool {
            match &e.kind {
                ExprKind::Call { func, .. } => match &func.kind {
                    ExprKind::MemberAccess { expr, member } => {
                        member.name == "size"
                            && matches!(&expr.kind, ExprKind::Index { expr: b, .. }
                                if base_name(b).as_deref() == Some(prop))
                    }
                    _ => false,
                },
                ExprKind::Binary { left, right, .. } => {
                    row_size_call(left, prop) || row_size_call(right, prop)
                }
                ExprKind::Paren(i) => row_size_call(i, prop),
                _ => false,
            }
        }
        fn walk(item: &ConstraintItem, prop: &str) -> bool {
            match item {
                ConstraintItem::Foreach {
                    array,
                    vars,
                    item: body,
                    ..
                } => {
                    if base_name(array).as_deref() == Some(prop) && vars.len() >= 2 {
                        return true;
                    }
                    walk(body, prop)
                }
                ConstraintItem::Expr(e) => row_size_call(e, prop),
                ConstraintItem::Inside { expr, .. } => row_size_call(expr, prop),
                ConstraintItem::Block(items) => items.iter().any(|i| walk(i, prop)),
                ConstraintItem::Soft(i) => walk(i, prop),
                ConstraintItem::Implication { constraint, .. } => walk(constraint, prop),
                ConstraintItem::IfElse {
                    then_item,
                    else_item,
                    ..
                } => walk(then_item, prop) || else_item.as_ref().is_some_and(|e| walk(e, prop)),
                _ => false,
            }
        }
        constraints
            .iter()
            .any(|c| c.items.iter().any(|i| walk(i, prop)))
    }

    /// Outer element count of a collection in its CURRENT state.
    pub(super) fn coll_outer_size(&self, c: &RandColl) -> u64 {
        match c.kind {
            CollKind::Fixed => (c.hi - c.lo + 1).max(0) as u64,
            CollKind::Dyn => self.get_queue_size(&c.scoped),
            CollKind::Assoc => self.assoc_key_strs(&c.scoped).len() as u64,
        }
    }

    /// The populated keys of an associative-array member, in numeric (or
    /// lexicographic, for string keys) order — the §7.8.4 iteration order.
    pub(super) fn assoc_key_strs(&self, scoped: &str) -> Vec<String> {
        let prefix = format!("{}[", scoped);
        let mut keys: Vec<String> = self
            .signals
            .keys_with_elem_prefix(&prefix)
            .into_iter()
            .filter(|k| k.ends_with(']'))
            .map(|k| k[prefix.len()..k.len() - 1].to_string())
            // A nested inner element (`m[0][1]`) or row-size shadow is not a key.
            .filter(|k| !k.contains('[') && !k.contains(']'))
            .collect();
        if self.is_string_keyed_array(scoped) {
            keys.sort();
        } else {
            keys.sort_by_key(|k| k.parse::<i64>().unwrap_or(0));
        }
        keys
    }

    /// Every element storage key of a collection, in index order.
    pub(super) fn coll_elem_keys(&self, c: &RandColl) -> Vec<String> {
        match c.kind {
            CollKind::Fixed => (c.lo..=c.hi)
                .map(|i| format!("{}[{}]", c.scoped, i))
                .collect(),
            CollKind::Assoc => self
                .assoc_key_strs(&c.scoped)
                .into_iter()
                .map(|k| format!("{}[{}]", c.scoped, k))
                .collect(),
            CollKind::Dyn => {
                let n = self.get_queue_size(&c.scoped);
                // §7.4.5: every element of every fixed row (`a[i][j]`).
                if let Some(RowTail::Fixed(inner)) = &c.row_tail {
                    let mut keys: Vec<String> =
                        (0..n).map(|i| format!("{}[{}]", c.scoped, i)).collect();
                    for &(lo, hi) in inner {
                        keys = keys
                            .iter()
                            .flat_map(|k| (lo..=hi).map(move |j| format!("{}[{}]", k, j)))
                            .collect();
                    }
                    return keys;
                }
                if !c.nested {
                    return (0..n).map(|i| format!("{}[{}]", c.scoped, i)).collect();
                }
                let mut out = Vec::new();
                for i in 0..n {
                    let row = format!("{}[{}]", c.scoped, i);
                    let m = self.get_queue_size(&row);
                    for j in 0..m {
                        out.push(format!("{}[{}]", row, j));
                    }
                }
                out
            }
        }
    }

    /// Resolve an expression that names a collection ELEMENT (`m[i]`,
    /// `m[i][j]`, `obj.m[k]`) to its scoped storage key. Loop variables must
    /// already be bound on `local_stack`. Returns None when the base is not a
    /// collection.
    /// §7.4.5 class property that is an ARRAY OF COLLECTIONS
    /// (`int q[2][2][$]`, `int d[3][]`, `int a[2][int]`): `q[i][j]` names
    /// one element collection, stored under the object-scoped flat key
    /// `h#q[i][j]` (core registers a queue/assoc property per element).
    /// Returns that key when `e` supplies exactly the outer indices; with
    /// fewer or more indices the caller's element paths apply.
    pub(super) fn class_array_of_coll_key(&mut self, e: &Expression) -> Option<String> {
        self.class_coll_elem_key(e, false)
    }

    /// [`class_array_of_coll_key`] restricted to a FIXED outer shape, with
    /// every index inside its declared bounds: an element that exists for
    /// as long as the object does.
    pub(super) fn class_fixed_array_of_coll_key(&mut self, e: &Expression) -> Option<String> {
        self.class_coll_elem_key(e, true)
    }

    /// The object store `<h>#q` and outer shape of a class property that is
    /// a FIXED array of collections, named without selects (`q`, `this.q`,
    /// `h.q`): what a multi-variable `foreach (q[i, j, k])` walks.
    pub(super) fn class_fixed_array_of_coll_base(
        &mut self,
        base: &Expression,
    ) -> Option<(String, Vec<(i64, i64)>)> {
        if self.no_class_objects() {
            return None;
        }
        let (handle, member, _, shape) = self.class_coll_prop(base, true)?;
        Some((format!("{}#{}", handle, member), shape?))
    }

    fn class_coll_elem_key(&mut self, e: &Expression, fixed_only: bool) -> Option<String> {
        let mut idxs: Vec<&Expression> = Vec::new();
        let mut cur = e;
        while let ExprKind::Index { expr: b, index } = &cur.kind {
            idxs.push(index);
            cur = b;
        }
        if idxs.is_empty() || self.no_class_objects() {
            return None;
        }
        idxs.reverse();
        let (handle, member, outer_len, bounds) = self.class_coll_prop(cur, fixed_only)?;
        if outer_len != idxs.len() {
            return None;
        }
        let mut key = format!("{}#{}", handle, member);
        for (d, ix) in idxs.into_iter().enumerate() {
            let i = self.eval_expr(ix).to_i64().unwrap_or(0);
            if let Some(b) = &bounds
                && !(b[d].0.min(b[d].1)..=b[d].0.max(b[d].1)).contains(&i)
            {
                return None;
            }
            key.push('[');
            key.push_str(&i.to_string());
            key.push(']');
        }
        Some(key)
    }

    /// The receiver handle, member name and outer dimension count of the
    /// class collection property `base` names; with `fixed_only`, only a
    /// fixed array of collections, and its declared outer shape.
    #[allow(clippy::type_complexity)]
    fn class_coll_prop(
        &mut self,
        base: &Expression,
        fixed_only: bool,
    ) -> Option<(usize, String, usize, Option<Vec<(i64, i64)>>)> {
        let (handle, member): (usize, String) = match &base.kind {
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                let nm = &h.path[0].name.name;
                if self.local_stack.last().is_some_and(|l| l.contains_key(nm)) {
                    return None;
                }
                (self.this_stack.last().copied().flatten()?, nm.clone())
            }
            ExprKind::MemberAccess { expr: obj, member } => {
                let h = self.eval_expr(obj).to_u64()? as usize;
                (h, member.name.clone())
            }
            ExprKind::Ident(h) if h.path.len() == 2 && h.path[0].selects.is_empty() => {
                let obj = &h.path[0].name.name;
                let hd = if obj == "this" || obj == "super" {
                    self.this_stack.last().copied().flatten().unwrap_or(0)
                } else {
                    self.eval_ident_handle(obj).unwrap_or(0)
                };
                (hd, h.path[1].name.name.clone())
            }
            _ => return None,
        };
        if handle == 0 {
            return None;
        }
        let cn = self
            .heap
            .get(handle)
            .and_then(|x| x.as_ref())
            .map(|i| i.class_name.clone())?;
        let mut cur_cls: Option<String> = Some(cn);
        while let Some(c) = cur_cls {
            let Some(cd) = self.module.classes.get(&c) else {
                break;
            };
            if let Some((shape, _, _)) = cd.array_of_coll_properties.get(&member) {
                let bounds = fixed_only.then(|| {
                    cd.array_prop_decl_dims
                        .get(&member)
                        .cloned()
                        .unwrap_or_else(|| shape.clone())
                });
                return Some((handle, member, shape.len(), bounds));
            }
            if !fixed_only
                && !cd.dynamic_assoc_properties.is_empty()
                && cd.dynamic_assoc_properties.contains_key(&member)
            {
                return Some((handle, member, 1, None));
            }
            // §7.4.5: a dynamic outer dimension over dynamic or queue rows
            // (`int d[][]`): `d[i]` names one row collection.
            if !fixed_only
                && cd.dynamic_tail_properties.get(&member).is_some_and(|t| {
                    matches!(
                        t.first(),
                        Some(
                            crate::ast::types::UnpackedDimension::Unsized(_)
                                | crate::ast::types::UnpackedDimension::Queue { .. }
                        )
                    )
                })
            {
                return Some((handle, member, 1, None));
            }
            cur_cls = cd.extends.clone();
        }
        None
    }

    /// Whether the collection element `key` (`<h>#m[i]`, or `m[i]` of
    /// `handle`) belongs to a `rand` member of its object's class.
    pub(super) fn coll_key_is_rand(&self, handle: usize, key: &str) -> bool {
        let (h, rest) = match key.split_once('#') {
            Some((h, r)) => (h.parse::<usize>().unwrap_or(handle), r),
            None => (handle, key),
        };
        self.prop_is_rand(h, rest.split('[').next().unwrap_or(""))
    }

    /// Whether `prop` is declared `rand`/`randc` in the class of `h` or a
    /// base class.
    pub(super) fn prop_is_rand(&self, h: usize, prop: &str) -> bool {
        let mut cur = self
            .heap
            .get(h)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone());
        while let Some(cn) = cur {
            let Some(cd) = self.module.classes.get(&cn) else {
                return false;
            };
            if cd.random_properties.contains(prop) {
                return true;
            }
            cur = cd.extends.clone();
        }
        false
    }

    pub(super) fn coll_elem_expr_key(&mut self, e: &Expression) -> Option<String> {
        if let Some(k) = self.class_array_of_coll_key(e) {
            return Some(k);
        }
        let ExprKind::Index { expr: base, index } = &e.kind else {
            return None;
        };
        // Row of a 2-D dynamic array: the base is itself an element key.
        if let Some(row) = self.coll_elem_expr_key(base) {
            if self.module.dynamic_arrays.contains(&row) {
                let i = self.eval_expr(index).to_i64().unwrap_or(0);
                return Some(format!("{}[{}]", row, i));
            }
            // §7.8: a row of a multidimensional associative array
            // (`o.p["k"][3]` of `int p[string][int][$]`) is keyed by value.
            if self.is_associative_array(&row) {
                let kv = self.eval_expr(index);
                return Some(format!("{}[{}]", row, self.assoc_key_str(&row, &kv)));
            }
            return None;
        }
        let bn = match self.strip_rand_receiver_prefix(base) {
            Some(b) => self.expr_assoc_name(&b)?,
            None => self.expr_assoc_name(base)?,
        };
        let idx_val = self.eval_expr(index);
        let key = if self.is_associative_array(&bn) {
            self.assoc_key_str(&bn, &idx_val)
        } else {
            idx_val.to_i64().unwrap_or(0).to_string()
        };
        Some(format!("{}[{}]", bn, key))
    }

    /// Write one collection element. The instance-scoped key (`<h>#m[i]`) is
    /// the storage of record; an ALREADY-EXISTING compact-table alias
    /// (`m[i]`, for an array that was also materialised there) is kept in step.
    /// A missing alias is never created — that would shadow an unrelated
    /// module-scope array of the same name.
    pub(super) fn write_coll_elem(&mut self, key: &str, mut val: Value) {
        // Solver draws and repairs inherit the element declaration, not the
        // temporary value's signedness. Size shadows are not element values.
        if !val.is_real
            && key.ends_with(']')
            && let Some((owner, element)) = key.split_once('#')
            && let Ok(handle) = owner.parse::<usize>()
            && let Some((prop, _)) = element.split_once('[')
            && self.class_prop_width_of(handle, prop).is_some()
        {
            val.is_signed = self.class_prop_signed_of(handle, prop);
        }
        self.signals.insert(key.to_string(), val.clone());
        if let Some((_h, bare)) = key.split_once('#') {
            if self.signal_name_to_id.contains_key(bare) {
                self.set_signal_value_by_name(bare, val);
            }
        }
    }

    pub(super) fn read_coll_elem(&self, key: &str) -> Option<Value> {
        self.signals.get(key).cloned()
    }

    /// §18.5 — solve the `.size()` constraints of every rand dynamic array /
    /// queue, BEFORE any element is solved: the element set does not exist
    /// until the array has a length. Rows of a 2-D dynamic array are sized
    /// from the `foreach (m[i]) m[i].size() == …` body.
    pub(super) fn solve_array_sizes(
        &mut self,
        colls: &[RandColl],
        constraints: &[ClassConstraint],
    ) {
        for c in colls {
            if c.kind != CollKind::Dyn {
                continue;
            }
            let cur = self.get_queue_size(&c.scoped);
            let n = match self.pick_size(&c.prop, None, constraints) {
                Some(n) => n,
                None => cur,
            };
            if c.row_tail.is_some() {
                self.resize_row_owner(c, n);
            } else {
                self.resize_coll(&c.scoped, n);
            }
            self.module.dynamic_arrays.insert(c.scoped.clone());
            if c.nested {
                for i in 0..n {
                    let row = format!("{}[{}]", c.scoped, i);
                    let cur_row = self.get_queue_size(&row);
                    let m = self
                        .pick_size(&c.prop, Some(i as i64), constraints)
                        .unwrap_or(cur_row);
                    self.resize_coll(&row, m);
                    self.module.dynamic_arrays.insert(row);
                }
            }
        }
    }

    /// The row shape of the dynamic member `prop` of `cd` declared over
    /// unpacked rows (`T a[][N]`, `T a[][]`); `None` for any other member or
    /// a deeper nesting the solver does not model.
    pub(super) fn dyn_row_tail(
        &self,
        cd: &crate::compiler::elaborate::ElaboratedClass,
        handle: usize,
        prop: &str,
    ) -> Option<RowTail> {
        use crate::ast::types::UnpackedDimension as U;
        let tail = cd.dynamic_tail_properties.get(prop)?;
        if let [U::Unsized(_) | U::Queue { .. }] = tail.as_slice() {
            return Some(RowTail::Dyn);
        }
        let params = self.instance_param_scope(handle);
        let dims: Option<Vec<(i64, i64)>> = tail
            .iter()
            .map(|d| match d {
                U::Range { left, right, .. } => {
                    let l = super::elaborate::const_eval_i64_with_params(left, Some(&params))?;
                    let r = super::elaborate::const_eval_i64_with_params(right, Some(&params))?;
                    Some((l.min(r), l.max(r)))
                }
                U::Expression { expr, .. } => {
                    match super::elaborate::const_eval_i64_with_params(expr, Some(&params)) {
                        Some(n) if n > 0 => Some((0, n - 1)),
                        _ => None,
                    }
                }
                _ => None,
            })
            .collect();
        dims.map(RowTail::Fixed)
    }

    /// Resize the dynamic member `c` whose elements are rows: rows past the
    /// new end lose their storage, and new dynamic rows are registered as
    /// empty collections of their own.
    pub(super) fn resize_row_owner(&mut self, c: &RandColl, n: u64) {
        let old = self.get_queue_size(&c.scoped);
        for i in n..old {
            let row = format!("{}[{}]", c.scoped, i);
            self.clear_dyn_row(&row);
            self.signals.remove(&row);
            self.signals.remove(&format!("{}.size", row));
        }
        if c.row_tail == Some(RowTail::Dyn) {
            for i in old.min(n)..n {
                let row = format!("{}[{}]", c.scoped, i);
                self.module.dynamic_arrays.insert(row.clone());
                self.module.arrays.insert(row.clone(), (0, 63, c.width));
                self.set_queue_size(&row, 0);
            }
        }
        self.set_queue_size(&c.scoped, n);
    }

    /// Set a dynamic array's length, dropping any element storage past the
    /// new end (a stale key would otherwise resurface as a phantom element).
    pub(super) fn resize_coll(&mut self, scoped: &str, n: u64) {
        let old = self.get_queue_size(scoped);
        if old > n {
            for i in n..old {
                self.signals.remove(&format!("{}[{}]", scoped, i));
            }
        }
        self.set_queue_size(scoped, n);
    }

    /// Pick a length for `prop` (or for row `row` of it) from the constraint
    /// set. `None` when the array's size is unconstrained — the caller keeps
    /// the current length (§18.4: an unconstrained rand dynamic array is not
    /// re-shaped by xezim; only its elements are drawn).
    pub(super) fn pick_size(
        &mut self,
        prop: &str,
        row: Option<i64>,
        constraints: &[ClassConstraint],
    ) -> Option<u64> {
        use rand::Rng;
        let mut exact: Option<u64> = None;
        let mut ranges: Vec<(u64, u64)> = Vec::new();
        let mut lo_b: u64 = 0;
        let mut hi_b: u64 = 64;
        let mut bounded = false;
        // Every size expression is evaluated WHILE the foreach index of a row
        // constraint is still bound, so `m[i].size() == i + 2` resolves.
        let items = self.size_constraint_items(prop, row, constraints);
        for kind in items {
            match kind {
                SizeCon::Eq(n) => exact = Some(n),
                SizeCon::In(rs) => ranges.extend(rs),
                SizeCon::Rel(op, v) => {
                    bounded = true;
                    match op {
                        BinaryOp::Lt => hi_b = hi_b.min(v.saturating_sub(1)),
                        BinaryOp::Leq => hi_b = hi_b.min(v),
                        BinaryOp::Gt => lo_b = lo_b.max(v + 1),
                        BinaryOp::Geq => lo_b = lo_b.max(v),
                        _ => {}
                    }
                }
            }
        }
        if let Some(n) = exact {
            return Some(n.min(1 << 16));
        }
        if !ranges.is_empty() {
            let (l, h) = ranges[self.rng.gen_range(0..ranges.len())];
            return Some(if h > l { self.rng.gen_range(l..=h) } else { l });
        }
        if bounded && lo_b <= hi_b {
            return Some(if hi_b > lo_b {
                self.rng.gen_range(lo_b..=hi_b)
            } else {
                lo_b
            });
        }
        None
    }

    /// Collect the `.size()` constraints that apply to `prop` (when `row` is
    /// None) or to row `row` of it (when Some — the `foreach (m[i])
    /// m[i].size() == …` shape, evaluated with `i` bound).
    pub(super) fn size_constraint_items(
        &mut self,
        prop: &str,
        row: Option<i64>,
        constraints: &[ClassConstraint],
    ) -> Vec<SizeCon> {
        let mut out: Vec<SizeCon> = Vec::new();
        // Snapshot the items to walk without borrowing self.
        let mut work: Vec<(ConstraintItem, Option<i64>)> = Vec::new();
        for con in constraints {
            for it in &con.items {
                work.push((it.clone(), None));
            }
        }
        while let Some((item, _bind)) = work.pop() {
            match item {
                ConstraintItem::Block(items) => {
                    for i in items {
                        work.push((i, None));
                    }
                }
                ConstraintItem::Soft(i) => work.push((*i, None)),
                ConstraintItem::Foreach {
                    array,
                    vars,
                    item: body,
                    ..
                } => {
                    // Row sizing lives inside `foreach (m[i]) m[i].size() …`.
                    if row.is_none() {
                        continue;
                    }
                    if Self::foreach_base_name(&array).as_deref() != Some(prop) {
                        continue;
                    }
                    let Some(idx_name) = vars
                        .first()
                        .and_then(|v| v.as_ref().map(|i| i.name.clone()))
                    else {
                        continue;
                    };
                    let mut frame: HashMap<String, Value> = HashMap::default();
                    frame.insert(idx_name, Self::signed_loop_val(row.unwrap()));
                    self.push_local_frame(frame);
                    let mut inner = self.size_cons_in_item(&body, prop, true);
                    self.pop_local_frame();
                    out.append(&mut inner);
                }
                other => {
                    if row.is_none() {
                        let mut got = self.size_cons_in_item(&other, prop, false);
                        out.append(&mut got);
                    }
                }
            }
        }
        out
    }

    /// Extract size constraints from one (non-foreach) item. `row` selects
    /// between a whole-array receiver (`m.size()`) and a row receiver
    /// (`m[i].size()`).
    pub(super) fn size_cons_in_item(
        &mut self,
        item: &ConstraintItem,
        prop: &str,
        row: bool,
    ) -> Vec<SizeCon> {
        let mut out: Vec<SizeCon> = Vec::new();
        let recv = self.active_rand_receiver().map(str::to_owned);
        let recv = recv.as_deref();
        match item {
            ConstraintItem::Block(items) => {
                for i in items {
                    out.append(&mut self.size_cons_in_item(i, prop, row));
                }
            }
            ConstraintItem::Soft(i) => out.append(&mut self.size_cons_in_item(i, prop, row)),
            // §18.5.6/§18.5.7: a `.size()` constraint nested under an
            // implication or if/else never reached the size solver — only
            // Block/Soft/Inside/Expr had arms, so the queue got an arbitrary
            // size and `randomize()` still returned 1. Descend into the guarded
            // branches and take the size bound from whichever the condition
            // selects; an unevaluable condition contributes nothing rather than
            // guessing.
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                if self.eval_expr(condition).is_true() {
                    out.append(&mut self.size_cons_in_item(constraint, prop, row));
                }
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                if self.eval_expr(condition).is_true() {
                    out.append(&mut self.size_cons_in_item(then_item, prop, row));
                } else if let Some(e) = else_item {
                    out.append(&mut self.size_cons_in_item(e, prop, row));
                }
            }
            ConstraintItem::Inside {
                expr,
                range,
                is_dist,
                ..
            } if !*is_dist && Self::is_size_call(expr, prop, row, recv) => {
                let rs = range.clone();
                out.push(SizeCon::In(self.eval_range_list(&rs)));
            }
            ConstraintItem::Expr(e) => match &e.kind {
                // §18.5.6: `guard -> q.size() == N`. The parser's expression
                // grammar consumes `->` as the LOW-precedence LogImplies
                // OPERATOR before the constraint parser can see it, so this
                // arrives as one Expr — never as ConstraintItem::Implication —
                // and the size bound under it was silently dropped: the queue
                // got an arbitrary size and randomize() still returned 1.
                ExprKind::Binary {
                    op: BinaryOp::LogImplies,
                    left,
                    right,
                } => {
                    if self.eval_expr(left).is_true() {
                        let inner = ConstraintItem::Expr((**right).clone());
                        out.append(&mut self.size_cons_in_item(&inner, prop, row));
                    }
                }
                ExprKind::Binary { op, left, right } => {
                    let (sz_side, other, flip) = if Self::is_size_call(left, prop, row, recv) {
                        (true, right, false)
                    } else if Self::is_size_call(right, prop, row, recv) {
                        (true, left, true)
                    } else {
                        (false, right, false)
                    };
                    if sz_side {
                        let eff = if flip {
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
                        let other = (**other).clone();
                        match eff {
                            BinaryOp::Eq => {
                                if let Some(n) = self.eval_expr(&other).to_u64() {
                                    out.push(SizeCon::Eq(n));
                                }
                            }
                            BinaryOp::Lt | BinaryOp::Leq | BinaryOp::Gt | BinaryOp::Geq => {
                                let v = self.eval_expr(&other).to_u64().unwrap_or(0);
                                out.push(SizeCon::Rel(eff, v));
                            }
                            _ => {}
                        }
                    }
                }
                ExprKind::Inside {
                    expr: inner,
                    ranges,
                } if Self::is_size_call(inner, prop, row, recv) => {
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
                    out.push(SizeCon::In(self.eval_range_list(&cr)));
                }
                _ => {}
            },
            _ => {}
        }
        out
    }

    /// Evaluate a constraint range list to concrete inclusive (lo, hi) pairs.
    pub(super) fn eval_range_list(&mut self, ranges: &[ConstraintRange]) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        for r in ranges {
            match r {
                ConstraintRange::Value(e) => {
                    if let Some(u) = self.eval_expr(e).to_u64() {
                        out.push((u, u));
                    }
                }
                ConstraintRange::Range { lo, hi } => {
                    let l = self.eval_expr(lo).to_u64().unwrap_or(0);
                    let h = self.eval_expr(hi).to_u64().unwrap_or(l);
                    out.push((l.min(h), l.max(h)));
                }
            }
        }
        out
    }

    /// `m.size()` (row=false) / `m[i].size()` (row=true) receiver test.
    /// `recv` is the active `obj.randomize() with {…}` receiver
    /// (`active_rand_receiver`): §18.7, `obj.m.size()` in the inline block
    /// sizes the object's own `m`.
    pub(super) fn is_size_call(e: &Expression, prop: &str, row: bool, recv: Option<&str>) -> bool {
        // The built-in array `size` method (§7.4.2) may be written either as a
        // no-argument call (`q.size()`) or as a bare member access (`q.size`).
        // Both denote the same method; a constraint like `q.size inside
        // {[1:11]}` parses to the member-access form, so the matcher must
        // accept both shapes.
        let arr = match &e.kind {
            ExprKind::Call { func, args } if args.is_empty() => match &func.kind {
                ExprKind::MemberAccess { expr, member } if member.name == "size" => expr,
                _ => return false,
            },
            ExprKind::MemberAccess { expr, member } if member.name == "size" => expr,
            _ => return false,
        };
        let names_prop = |b: &Expression| match &b.kind {
            ExprKind::Ident(h) => h.path.last().is_some_and(|s| s.name.name == prop),
            ExprKind::MemberAccess { expr, member } => {
                member.name == prop
                    && recv.is_some()
                    && Self::plain_ident_name(expr).as_deref() == recv
            }
            _ => false,
        };
        match (&arr.kind, row) {
            (ExprKind::Index { expr: b, .. }, true) => names_prop(b),
            (_, false) => names_prop(arr),
            _ => false,
        }
    }

    /// Bare array name of a `foreach (arr[i])` / `foreach (arr[i, j])` header.
    pub(super) fn foreach_base_name(array: &Expression) -> Option<String> {
        match &array.kind {
            ExprKind::Index { expr, .. } => Self::foreach_base_name(expr),
            ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.clone()),
            ExprKind::MemberAccess { member, .. } => Some(member.name.clone()),
            _ => None,
        }
    }

    /// Draw a fresh random value for an unconstrained collection element.
    /// A drawn value that repeats the element's PREVIOUS content is re-rolled
    /// a few times: `randomize()` must visibly re-draw every rand element
    /// (§18.4), and a pre-seeded associative entry that came back identical is
    /// indistinguishable from "the solver never touched it".
    pub(super) fn draw_elem(&mut self, width: u32, prev: Option<u64>) -> Value {
        use rand::Rng;
        let mask = if width >= 64 {
            u64::MAX
        } else {
            (1u64 << width) - 1
        };
        let mut v = self.rng.r#gen::<u64>() & mask;
        if mask > 0 {
            for _ in 0..8 {
                if Some(v) != prev {
                    break;
                }
                v = self.rng.r#gen::<u64>() & mask;
            }
        }
        if width <= 64 {
            return Value::from_u64(v, width);
        }
        // §18.4: an element wider than 64 bits is drawn across its FULL
        // width. `from_u64` zero-fills every bit above 63, so routing a
        // wide element through it silently pinned the high half to 0 — the
        // low word still looked freshly drawn, and only a check of the high
        // bits (`wda[i][95:64] != 0`) could catch it. Keep the re-rolled
        // low word above, then draw the remaining bits fresh.
        let mut acc = Value::from_u64(v, width);
        let mut bit = 64u32;
        while bit < width {
            let chunk: u64 = self.rng.r#gen();
            let n = (width - bit).min(64);
            for k in 0..n {
                if (chunk >> k) & 1 == 1 {
                    acc.set_bit((bit + k) as usize, LogicBit::One);
                }
            }
            bit += n;
        }
        acc
    }

    /// The value ranges an element of `prop` is restricted to by
    /// `foreach (prop[i]) prop[i] inside {…}` bodies. Used by the `unique {}`
    /// repair so a distinct re-pick stays inside the element's legal domain.
    pub(super) fn elem_allowed_ranges(
        &mut self,
        prop: &str,
        constraints: &[ClassConstraint],
    ) -> Vec<(u64, u64)> {
        let mut out: Vec<(u64, u64)> = Vec::new();
        let mut ranges_expr: Vec<Vec<ConstraintRange>> = Vec::new();
        fn walk(item: &ConstraintItem, prop: &str, out: &mut Vec<Vec<ConstraintRange>>) {
            match item {
                ConstraintItem::Expr(e) => {
                    if let ExprKind::Inside {
                        expr: inner,
                        ranges,
                    } = &e.kind
                    {
                        let is_elem = matches!(&inner.kind, ExprKind::Index { expr: b, .. }
                            if matches!(&b.kind, ExprKind::Ident(h)
                                if h.path.last().is_some_and(|s| s.name.name == prop)));
                        if is_elem {
                            out.push(
                                ranges
                                    .iter()
                                    .map(|r| match &r.kind {
                                        ExprKind::Range(lo, hi) => ConstraintRange::Range {
                                            lo: (**lo).clone(),
                                            hi: (**hi).clone(),
                                        },
                                        _ => ConstraintRange::Value(r.clone()),
                                    })
                                    .collect(),
                            );
                        }
                    }
                }
                ConstraintItem::Inside {
                    expr,
                    range,
                    is_dist,
                    ..
                } => {
                    let is_elem = matches!(&expr.kind, ExprKind::Index { expr: b, .. }
                        if matches!(&b.kind, ExprKind::Ident(h)
                            if h.path.last().is_some_and(|s| s.name.name == prop)));
                    if is_elem && !*is_dist {
                        out.push(range.clone());
                    }
                }
                ConstraintItem::Block(items) => {
                    for i in items {
                        walk(i, prop, out);
                    }
                }
                ConstraintItem::Soft(i) => walk(i, prop, out),
                ConstraintItem::Foreach {
                    array, item: body, ..
                } if Simulator::foreach_base_name(array).as_deref() == Some(prop) => {
                    walk(body, prop, out);
                }
                _ => {}
            }
        }
        for con in constraints {
            for it in &con.items {
                walk(it, prop, &mut ranges_expr);
            }
        }
        for rs in ranges_expr {
            for r in &rs {
                match r {
                    ConstraintRange::Value(e) => {
                        if let Some(u) = self.eval_expr(e).to_u64() {
                            out.push((u, u));
                        }
                    }
                    ConstraintRange::Range { lo, hi } => {
                        let l = self.eval_expr(lo).to_u64().unwrap_or(0);
                        let h = self.eval_expr(hi).to_u64().unwrap_or(l);
                        out.push((l.min(h), l.max(h)));
                    }
                }
            }
        }
        out
    }

    /// §18.5.7 — solve every `foreach` constraint over a rand COLLECTION
    /// (dynamic array / queue / associative array / 2-D dynamic array).
    /// For each element: first try the structural repair (`elem == expr`,
    /// `elem inside {…}`, `elem < K`), then fall back to a bounded
    /// generate-and-test over the element's domain, which covers arbitrary
    /// bodies such as `q[i] % 2 == 0`. Returns the keys pinned by an equality
    /// (they must survive the later `unique {}` repair).
    pub(super) fn solve_coll_foreach(
        &mut self,
        handle: usize,
        colls: &[RandColl],
        constraints: &[ClassConstraint],
        pinned: &mut HashSet<String>,
    ) {
        // (prop, vars, body) for every foreach over a collection we manage.
        let mut jobs: Vec<(
            RandColl,
            Vec<Option<crate::ast::Identifier>>,
            ConstraintItem,
        )> = Vec::new();
        fn walk(
            sim: &mut Simulator,
            item: &ConstraintItem,
            colls: &[RandColl],
            jobs: &mut Vec<(
                RandColl,
                Vec<Option<crate::ast::Identifier>>,
                ConstraintItem,
            )>,
        ) {
            match item {
                ConstraintItem::Foreach {
                    array,
                    vars,
                    item: body,
                    ..
                } => {
                    if let Some(name) = Simulator::foreach_base_name(array) {
                        if let Some(c) = colls.iter().find(|c| c.prop == name) {
                            if c.kind != CollKind::Fixed {
                                jobs.push((c.clone(), vars.clone(), (**body).clone()));
                            }
                        }
                    }
                }
                ConstraintItem::Block(items) => {
                    for i in items {
                        walk(sim, i, colls, jobs);
                    }
                }
                ConstraintItem::Soft(i) => walk(sim, i, colls, jobs),
                // §18.5.6/§18.5.7: collect foreach repairs only from the
                // branch active for the current scalar assignment.
                ConstraintItem::IfElse {
                    condition,
                    then_item,
                    else_item,
                    ..
                } => {
                    if sim.cons_expr_true(condition) {
                        walk(sim, then_item, colls, jobs);
                    } else if let Some(other) = else_item {
                        walk(sim, other, colls, jobs);
                    }
                }
                ConstraintItem::Implication {
                    condition,
                    constraint,
                    ..
                } => {
                    if sim.cons_expr_true(condition) {
                        walk(sim, constraint, colls, jobs);
                    }
                }
                _ => {}
            }
        }
        for con in constraints {
            for it in &con.items {
                walk(self, it, colls, &mut jobs);
            }
        }
        for (c, vars, body) in jobs {
            let idx_names: Vec<Option<String>> = vars
                .iter()
                .map(|v| v.as_ref().map(|i| i.name.clone()))
                .collect();
            match c.kind {
                CollKind::Assoc => {
                    for key in self.assoc_key_strs(&c.scoped) {
                        let mut frame: HashMap<String, Value> = HashMap::default();
                        if let Some(Some(n)) = idx_names.first() {
                            let kv = if self.is_string_keyed_array(&c.scoped) {
                                Value::from_string(&key)
                            } else {
                                Value::from_u64(key.parse::<i64>().unwrap_or(0) as u64, 32)
                            };
                            frame.insert(n.clone(), kv);
                        }
                        self.push_local_frame(frame);
                        let ek = format!("{}[{}]", c.scoped, key);
                        if self.solve_one_elem(handle, &body, &ek, c.width, &c.prop, constraints) {
                            pinned.insert(ek);
                        }
                        self.pop_local_frame();
                    }
                }
                CollKind::Dyn | CollKind::Fixed => {
                    let outer = self.coll_outer_size(&c);
                    for i in 0..outer as i64 {
                        if c.nested && idx_names.len() >= 2 {
                            let row = format!("{}[{}]", c.scoped, i);
                            let inner = self.get_queue_size(&row) as i64;
                            for j in 0..inner {
                                let mut frame: HashMap<String, Value> = HashMap::default();
                                if let Some(Some(n)) = idx_names.first() {
                                    frame.insert(n.clone(), Self::signed_loop_val(i));
                                }
                                if let Some(Some(n)) = idx_names.get(1) {
                                    frame.insert(n.clone(), Self::signed_loop_val(j));
                                }
                                self.push_local_frame(frame);
                                let ek = format!("{}[{}]", row, j);
                                if self.solve_one_elem(
                                    handle,
                                    &body,
                                    &ek,
                                    c.width,
                                    &c.prop,
                                    constraints,
                                ) {
                                    pinned.insert(ek);
                                }
                                self.pop_local_frame();
                            }
                            continue;
                        }
                        let mut frame: HashMap<String, Value> = HashMap::default();
                        if let Some(Some(n)) = idx_names.first() {
                            frame.insert(n.clone(), Self::signed_loop_val(i));
                        }
                        self.push_local_frame(frame);
                        // A 1-var foreach over a 2-D array constrains the ROW
                        // (its `.size()`), already handled by the sizing pass.
                        if !c.nested {
                            let ek = format!("{}[{}]", c.scoped, i);
                            if self.solve_one_elem(
                                handle,
                                &body,
                                &ek,
                                c.width,
                                &c.prop,
                                constraints,
                            ) {
                                pinned.insert(ek);
                            }
                        }
                        self.pop_local_frame();
                    }
                }
            }
        }
    }

    /// Solve one element against a `foreach` body. Returns true when the body
    /// PINS the element to an exact value (an equality) — such an element must
    /// not be moved by the later `unique {}` repair.
    pub(super) fn solve_one_elem(
        &mut self,
        handle: usize,
        body: &ConstraintItem,
        elem_key: &str,
        width: u32,
        prop: &str,
        constraints: &[ClassConstraint],
    ) -> bool {
        let pinned = self.force_elem_item(handle, body, elem_key, width);
        // Structural repair may not cover the body (`q[i] % 2 == 0`,
        // `q[i] > q[i-1]`, …). Fall back to generate-and-test over the
        // element's domain — the bounded backstop that keeps randomize()
        // honest for bodies the pattern matcher does not model.
        if self.item_holds(handle, body) {
            return pinned;
        }
        let allowed = self.elem_allowed_ranges(prop, constraints);
        let saved = self.read_coll_elem(elem_key);
        for _ in 0..256 {
            // +xezim_rand_timeout deadline was armed at randomize() entry but
            // only checked at trial boundaries — a single element's retry
            // loop could grind unboundedly past it. Each retry costs a full
            // interpreted constraint check, so polling the clock here is
            // negligible and bounds the loop by the budget.
            if self.rand_past_deadline() {
                break;
            }
            let cand = if allowed.is_empty() {
                self.draw_elem(width, None)
            } else {
                use rand::Rng;
                let (l, h) = allowed[self.rng.gen_range(0..allowed.len())];
                let v = if h > l { self.rng.gen_range(l..=h) } else { l };
                Value::from_u64(v, width)
            };
            self.write_coll_elem(elem_key, cand);
            if self.item_holds(handle, body) {
                return pinned;
            }
        }
        // Exhaustive sweep of a small domain — guarantees a solution exists
        // (or proves there is none) rather than leaving a random value.
        if width <= 16 {
            for v in 0..(1u64 << width) {
                if self.rand_past_deadline() {
                    break;
                }
                self.write_coll_elem(elem_key, Value::from_u64(v, width));
                if self.item_holds(handle, body) {
                    return pinned;
                }
            }
        }
        if let Some(s) = saved {
            self.write_coll_elem(elem_key, s);
        }
        pinned
    }

    /// Structural repair of a `foreach` body for one element: `elem == expr`
    /// pins it, `elem inside {…}` / `elem < K` move it into range. Returns
    /// true when an EQUALITY pinned the element.
    pub(super) fn force_elem_item(
        &mut self,
        handle: usize,
        item: &ConstraintItem,
        elem_key: &str,
        width: u32,
    ) -> bool {
        match item {
            ConstraintItem::Block(items) => {
                let mut pinned = false;
                for i in items {
                    if self.force_elem_item(handle, i, elem_key, width) {
                        pinned = true;
                    }
                }
                pinned
            }
            ConstraintItem::Soft(i) => self.force_elem_item(handle, i, elem_key, width),
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } if self.eval_expr(condition).is_true() => {
                self.force_elem_item(handle, constraint, elem_key, width)
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                if self.eval_expr(condition).is_true() {
                    self.force_elem_item(handle, then_item, elem_key, width)
                } else if let Some(e) = else_item {
                    self.force_elem_item(handle, e, elem_key, width)
                } else {
                    false
                }
            }
            ConstraintItem::Inside {
                expr,
                range,
                is_dist,
                dist_weights,
                ..
            } => {
                if self.coll_elem_expr_key(expr).as_deref() != Some(elem_key) {
                    return false;
                }
                // §18.5.4: the dynamic-array baseline was freshly drawn;
                // a foreach dist item must draw from its weighted support,
                // not rely on rejection of full-width random element values.
                if *is_dist {
                    if let Some(picked) = self.pick_dist_value(None, range, dist_weights) {
                        self.write_coll_elem(elem_key, picked.resize(width));
                    }
                    return false;
                }
                let cur = self
                    .read_coll_elem(elem_key)
                    .unwrap_or_else(|| Value::zero(width));
                if !self.value_in_ranges(&cur, range) {
                    if let Some(p) = self.pick_from_ranges(range, width, cur.is_signed) {
                        self.write_coll_elem(elem_key, p);
                    }
                }
                false
            }
            ConstraintItem::Expr(e) => match &e.kind {
                // The parser keeps a parenthesised constraint expression
                // wrapped; unwrap so `(q[i] == i)` is solved like `q[i] == i`.
                ExprKind::Paren(inner) => {
                    let it = ConstraintItem::Expr((**inner).clone());
                    self.force_elem_item(handle, &it, elem_key, width)
                }
                ExprKind::Inside {
                    expr: inner,
                    ranges,
                } => {
                    if self.coll_elem_expr_key(inner).as_deref() != Some(elem_key) {
                        return false;
                    }
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
                    let cur = self
                        .read_coll_elem(elem_key)
                        .unwrap_or_else(|| Value::zero(width));
                    if !self.value_in_ranges(&cur, &cr) {
                        if let Some(p) = self.pick_from_ranges(&cr, width, cur.is_signed) {
                            self.write_coll_elem(elem_key, p);
                        }
                    }
                    false
                }
                ExprKind::Binary { op, left, right } => {
                    let on_l = self.coll_elem_expr_key(left).as_deref() == Some(elem_key);
                    let on_r = self.coll_elem_expr_key(right).as_deref() == Some(elem_key);
                    if on_l == on_r {
                        return false; // cross-element or unrelated
                    }
                    let (other, eff) = if on_l {
                        (right, *op)
                    } else {
                        let m = match op {
                            BinaryOp::Lt => BinaryOp::Gt,
                            BinaryOp::Gt => BinaryOp::Lt,
                            BinaryOp::Leq => BinaryOp::Geq,
                            BinaryOp::Geq => BinaryOp::Leq,
                            o => *o,
                        };
                        (left, m)
                    };
                    match eff {
                        BinaryOp::Eq => {
                            let v = self.eval_expr(other).resize(width);
                            self.write_coll_elem(elem_key, v);
                            true
                        }
                        BinaryOp::Lt | BinaryOp::Leq | BinaryOp::Gt | BinaryOp::Geq => {
                            use rand::Rng;
                            let b = self.eval_expr(other).to_i64().unwrap_or(0);
                            let cur = self
                                .read_coll_elem(elem_key)
                                .and_then(|v| v.to_i64())
                                .unwrap_or(0);
                            let (mut lo, mut hi) = (
                                0i64,
                                if width >= 63 {
                                    i64::MAX
                                } else {
                                    (1i64 << width) - 1
                                },
                            );
                            match eff {
                                BinaryOp::Lt => hi = hi.min(b.saturating_sub(1)),
                                BinaryOp::Leq => hi = hi.min(b),
                                BinaryOp::Gt => lo = lo.max(b.saturating_add(1)),
                                BinaryOp::Geq => lo = lo.max(b),
                                _ => {}
                            }
                            if lo > hi || (cur >= lo && cur <= hi) {
                                return false;
                            }
                            let p = if lo == hi {
                                lo
                            } else {
                                self.rng.gen_range(lo..=hi)
                            };
                            self.write_coll_elem(elem_key, Value::from_u64(p as u64, width));
                            false
                        }
                        _ => false,
                    }
                }
                _ => {
                    let _ = handle;
                    false
                }
            },
            _ => false,
        }
    }

    /// Truth of one constraint item (a `foreach` BODY, or a plain item)
    /// against the CURRENT solved state — the acceptance test of the
    /// generate-and-test element search. Loop variables must already be bound
    /// on `local_stack`.
    pub(super) fn item_holds(&mut self, handle: usize, item: &ConstraintItem) -> bool {
        self.check_constraint_item(handle, item)
    }

    /// Constraints that name a collection ELEMENT outside any `foreach`
    /// (`q[0] == 8'd99`, typically from an inline `randomize() with {…}`, and
    /// `q[0] inside {…}`). Elements pinned by an equality are recorded so the
    /// `unique {}` repair leaves them alone.
    pub(super) fn solve_top_level_elems(
        &mut self,
        handle: usize,
        constraints: &[ClassConstraint],
        pinned: &mut HashSet<String>,
    ) {
        let mut items: Vec<ConstraintItem> = Vec::new();
        fn walk(sim: &mut Simulator, item: &ConstraintItem, out: &mut Vec<ConstraintItem>) {
            match item {
                ConstraintItem::Block(is) => {
                    for i in is {
                        walk(sim, i, out);
                    }
                }
                ConstraintItem::Soft(i) => walk(sim, i, out),
                // §18.5.6/§18.5.7: the post-draw element repair must also
                // restore equalities in the active conditional branch.
                ConstraintItem::IfElse {
                    condition,
                    then_item,
                    else_item,
                    ..
                } => {
                    if sim.cons_expr_true(condition) {
                        walk(sim, then_item, out);
                    } else if let Some(other) = else_item {
                        walk(sim, other, out);
                    }
                }
                ConstraintItem::Implication {
                    condition,
                    constraint,
                    ..
                } => {
                    if sim.cons_expr_true(condition) {
                        walk(sim, constraint, out);
                    }
                }
                ConstraintItem::Foreach { .. } | ConstraintItem::Unique { .. } => {}
                other => out.push(other.clone()),
            }
        }
        for con in constraints {
            for it in &con.items {
                walk(self, it, &mut items);
            }
        }
        for item in items {
            // Resolve the element key of whichever side names an element.
            let key = match &item {
                ConstraintItem::Expr(e) => match &e.kind {
                    ExprKind::Binary { left, right, .. } => self
                        .coll_elem_expr_key(left)
                        .or_else(|| self.coll_elem_expr_key(right)),
                    ExprKind::Inside { expr, .. } => self.coll_elem_expr_key(expr),
                    _ => None,
                },
                ConstraintItem::Inside { expr, .. } => self.coll_elem_expr_key(expr),
                _ => None,
            };
            let Some(key) = key else { continue };
            let width = self
                .read_coll_elem(&key)
                .map(|v| v.width)
                .unwrap_or(32)
                .max(1);
            if self.force_elem_item(handle, &item, &key, width) {
                pinned.insert(key);
            }
        }
    }

    /// §18.5.8.2 — recognize `<arr>.sum()` (with or without an identity `with`
    /// clause) as an array reduction over a rand collection. Returns the bare
    /// array name and, for the `with` form, the filter to identity-check.
    /// Three parse shapes are matched (see `WithClause` handling in `eval`).
    pub(super) fn sum_reduction_array(e: &Expression) -> Option<(String, Option<Expression>)> {
        match &e.kind {
            ExprKind::Paren(inner) => Self::sum_reduction_array(inner),
            ExprKind::WithClause { expr, filter } => {
                Self::plain_sum_array(expr).map(|n| (n, Some((**filter).clone())))
            }
            _ => Self::plain_sum_array(e).map(|n| (n, None)),
        }
    }

    /// The bare array name of a `<arr>.sum()` call (no `with`).
    pub(super) fn plain_sum_array(e: &Expression) -> Option<String> {
        match &e.kind {
            ExprKind::Call { func, args } if args.is_empty() => Self::plain_sum_array(func),
            ExprKind::MemberAccess { expr, member } if member.name == "sum" => {
                if let ExprKind::Ident(h) = &expr.kind {
                    return h.path.last().map(|s| s.name.name.clone());
                }
                None
            }
            ExprKind::Ident(h)
                if h.path.len() >= 2 && h.path.last().is_some_and(|s| s.name.name == "sum") =>
            {
                Some(h.path[h.path.len() - 2].name.name.clone())
            }
            _ => None,
        }
    }

    /// A reduction `with` filter is identity when `filter(item) == item` for a
    /// spread of probe values — this covers `item`, `(item)`, `int'(item)` and
    /// `unsigned'(item)` without matching the (lowered) cast AST directly.
    pub(super) fn filter_is_identity(&mut self, filter: &Expression) -> bool {
        self.push_local_frame(HashMap::default());
        let mut ok = true;
        for &p in &[0u64, 1, 7, 255, 1000] {
            if let Some(f) = self.local_stack.last_mut() {
                f.insert("item".to_string(), Value::from_u64(p, 32));
            }
            if self.eval_expr(filter).to_u64() != Some(p) {
                ok = false;
                break;
            }
        }
        self.pop_local_frame();
        ok
    }

    /// §18.5.8.2 — if `item` is `<arr>.sum() == <const>` (either operand order)
    /// over a single rand collection, return the collection and the constant
    /// target. The non-sum side must be a compile/solve-time constant (it may
    /// not reference any rand variable), so the sum has one solvable target.
    pub(super) fn array_sum_eq_target(
        &mut self,
        item: &ConstraintItem,
        colls: &[RandColl],
        rand_names: &HashSet<String>,
    ) -> Option<(RandColl, i128)> {
        let ConstraintItem::Expr(e) = item else {
            return None;
        };
        let ExprKind::Binary {
            op: BinaryOp::Eq,
            left,
            right,
        } = &e.kind
        else {
            return None;
        };
        for (sum_side, const_side) in [(left, right), (right, left)] {
            let Some((name, filter)) = Self::sum_reduction_array(sum_side) else {
                continue;
            };
            let Some(c) = colls.iter().find(|c| c.prop == name).cloned() else {
                continue;
            };
            if let Some(f) = &filter {
                if !self.filter_is_identity(f) {
                    continue;
                }
            }
            // The target side must be a stable constant — no rand variable and
            // no element of a managed collection may appear on it.
            let mut ids: HashSet<String> = HashSet::default();
            self.collect_expr_idents(const_side, &mut ids);
            if ids
                .iter()
                .any(|n| rand_names.contains(n) || colls.iter().any(|c| &c.prop == n))
            {
                continue;
            }
            // The target may be wider than 64 bits (`ws.sum() == 96'h1_…`):
            // `to_i64` reads only the low word of a wide Value, which turned
            // 2^64 into 0 and made the distributor below pin the sum to a
            // wrong, always-failing target. Read the full 128 bits (the
            // distributor works in i128); anything wider or unknown is left
            // to the trial checker.
            let tv = self.eval_expr(const_side);
            if tv.width > 128 || tv.has_xz() {
                continue;
            }
            let mut raw = tv.to_u128();
            if tv.is_signed && tv.width > 0 && (raw >> (tv.width - 1)) & 1 == 1 {
                raw |= !0u128 << (tv.width - 1);
            }
            return Some((c, raw as i128));
        }
        None
    }

    /// §18.5.8.2 — solve `<arr>.sum() == K` by DISTRIBUTING a random valid
    /// assignment over the array's elements (rejection sampling cannot find a
    /// point-target sum in a wide domain). Elements already pinned by another
    /// constraint keep their value and reduce the residual target; the rest are
    /// each drawn inside their `foreach … inside {…}` range (or the full type
    /// width) so the total lands exactly on `K`. Infeasible targets are left
    /// untouched — the trial checker then rejects and retries.
    pub(super) fn distribute_array_sum(
        &mut self,
        c: &RandColl,
        target: i128,
        constraints: &[ClassConstraint],
        pinned: &mut HashSet<String>,
    ) {
        let keys = self.coll_elem_keys(c);
        let ranges = self.elem_allowed_ranges(&c.prop, constraints);
        let (lo, hi): (i128, i128) = if ranges.is_empty() {
            // The element's full domain is 2^width - 1, which for a wide
            // (>64-bit) element exceeds u64::MAX — capping there made every
            // wide point-target above 2^64 look infeasible, so the distributor
            // never ran and the trial loop ground through its retries.
            let hi = if c.width >= 127 {
                i128::MAX
            } else {
                (1i128 << c.width) - 1
            };
            (0, hi)
        } else {
            (
                ranges.iter().map(|(l, _)| *l as i128).min().unwrap(),
                ranges.iter().map(|(_, h)| *h as i128).max().unwrap(),
            )
        };
        self.distribute_sum_over(&keys, target, lo, hi, c.width, pinned);
    }

    /// Core of the array-sum solver: assign `keys` so their total equals
    /// `target`, each element inside `[lo, hi]`. Already-pinned keys keep their
    /// value and reduce the residual; infeasible targets leave the array as-is.
    pub(super) fn distribute_sum_over(
        &mut self,
        keys: &[String],
        target: i128,
        lo: i128,
        hi: i128,
        width: u32,
        pinned: &mut HashSet<String>,
    ) {
        use rand::Rng;
        if keys.is_empty() {
            return;
        }
        let mut fixed_sum: i128 = 0;
        let mut free: Vec<String> = Vec::new();
        for k in keys {
            if pinned.contains(k) {
                // A pinned wide element contributes its full 128 bits —
                // `to_u64` would count only the low word and the residual
                // handed to the free elements would be wrong.
                let v = self.read_coll_elem(k).map(|v| v.to_u128()).unwrap_or(0);
                fixed_sum = fixed_sum.saturating_add(v as i128);
            } else {
                free.push(k.clone());
            }
        }
        if free.is_empty() {
            return;
        }
        let n = free.len() as i128;
        let want = target.saturating_sub(fixed_sum);
        // An element of 127 bits or more has the domain cap i128::MAX, so
        // `n * hi` overflows: the bounds saturate, which only says the true
        // bound is further out still.
        if want < n.saturating_mul(lo) || want > n.saturating_mul(hi) {
            return; // infeasible — leave for the trial checker to reject
        }
        let cap = hi.saturating_sub(lo);
        let mut rem = want.saturating_sub(n.saturating_mul(lo)); // 0 ..= n*cap
        let mut order: Vec<usize> = (0..free.len()).collect();
        for i in (1..order.len()).rev() {
            let j = self.cur_rng().gen_range(0..=i);
            order.swap(i, j);
        }
        for (pos, &idx) in order.iter().enumerate() {
            let remaining_after = (order.len() - pos - 1) as i128;
            let give = if remaining_after == 0 {
                rem
            } else {
                // Keep the residual absorbable by the still-unassigned elements.
                let lo_b = rem
                    .saturating_sub(remaining_after.saturating_mul(cap))
                    .max(0);
                let hi_b = rem.min(cap);
                if hi_b <= lo_b {
                    lo_b
                } else {
                    lo_b + self.cur_rng().gen_range(0..=(hi_b - lo_b))
                }
            };
            rem -= give;
            // A distributed share can exceed u64::MAX for wide elements
            // (`lo + give` counts up to the 2^width domain); `from_u64`
            // would truncate it back to the low word and the sum would
            // silently miss its target.
            let share = lo + give;
            let val = if width > 64 {
                Value::from_u128(share as u128, width)
            } else {
                Value::from_u64(share as u64, width)
            };
            self.write_coll_elem(&free[idx], val);
            pinned.insert(free[idx].clone());
        }
    }

    /// §18.5.8.2 — run `distribute_array_sum` for every `arr.sum() == K`
    /// equality in the constraint set (flattening `{}`/`soft` wrappers).
    pub(super) fn solve_array_sum_eqs(
        &mut self,
        constraints: &[ClassConstraint],
        colls: &[RandColl],
        rand_names: &HashSet<String>,
        pinned: &mut HashSet<String>,
    ) {
        let mut items: Vec<ConstraintItem> = Vec::new();
        fn walk(it: &ConstraintItem, out: &mut Vec<ConstraintItem>) {
            match it {
                ConstraintItem::Block(is) => {
                    for i in is {
                        walk(i, out);
                    }
                }
                ConstraintItem::Soft(i) => walk(i, out),
                other => out.push(other.clone()),
            }
        }
        for con in constraints {
            for it in &con.items {
                walk(it, &mut items);
            }
        }
        for it in &items {
            if let Some((c, k)) = self.array_sum_eq_target(it, colls, rand_names) {
                self.distribute_array_sum(&c, k, constraints, pinned);
            } else if let Some((c, idxs, k)) =
                self.explicit_elem_sum_eq_target(it, colls, rand_names)
            {
                // `a[0] + a[1] + a[2] == K`: distribute over just the named
                // elements, honoring any `foreach … inside {…}` element range.
                let ranges = self.elem_allowed_ranges(&c.prop, constraints);
                let (lo, hi): (i128, i128) = if ranges.is_empty() {
                    let hi = if c.width >= 64 {
                        u64::MAX
                    } else {
                        (1u64 << c.width) - 1
                    };
                    (0, hi as i128)
                } else {
                    (
                        ranges.iter().map(|(l, _)| *l as i128).min().unwrap(),
                        ranges.iter().map(|(_, h)| *h as i128).max().unwrap(),
                    )
                };
                let keys: Vec<String> = idxs
                    .iter()
                    .map(|i| format!("{}[{}]", c.scoped, i))
                    .collect();
                self.distribute_sum_over(&keys, k, lo, hi, c.width, pinned);
            }
        }
    }

    /// §18.5.8 — recognize `a[c0] + a[c1] + … == K`: a `+`-chain of
    /// CONSTANT-index selects of ONE rand collection compared to a constant.
    /// Returns the collection, the (distinct) element indices, and the target.
    pub(super) fn explicit_elem_sum_eq_target(
        &mut self,
        item: &ConstraintItem,
        colls: &[RandColl],
        rand_names: &HashSet<String>,
    ) -> Option<(RandColl, Vec<i64>, i128)> {
        let ConstraintItem::Expr(e) = item else {
            return None;
        };
        let ExprKind::Binary {
            op: BinaryOp::Eq,
            left,
            right,
        } = &e.kind
        else {
            return None;
        };
        // Collect the index selects on a `+`-chain over a single array `name`.
        // Index expressions are gathered as-is, then const-evaluated below.
        fn chain<'a>(
            e: &'a Expression,
            name: &mut Option<String>,
            idxs: &mut Vec<&'a Expression>,
        ) -> bool {
            match &e.kind {
                ExprKind::Paren(i) => chain(i, name, idxs),
                ExprKind::Binary {
                    op: BinaryOp::Add,
                    left,
                    right,
                } => chain(left, name, idxs) && chain(right, name, idxs),
                ExprKind::Index { expr, index } => {
                    let ExprKind::Ident(h) = &expr.kind else {
                        return false;
                    };
                    let arr = match h.path.last() {
                        Some(s) => s.name.name.clone(),
                        None => return false,
                    };
                    match name {
                        Some(n) if *n != arr => return false,
                        _ => *name = Some(arr),
                    }
                    idxs.push(index);
                    true
                }
                _ => false,
            }
        }
        for (sum_side, const_side) in [(left, right), (right, left)] {
            let mut name: Option<String> = None;
            let mut idx_exprs: Vec<&Expression> = Vec::new();
            if !chain(sum_side, &mut name, &mut idx_exprs) || idx_exprs.len() < 2 {
                continue;
            }
            let mut idxs: Vec<i64> = Vec::new();
            let mut all_const = true;
            for ie in &idx_exprs {
                match self.eval_expr(ie).to_i64() {
                    Some(v) => idxs.push(v),
                    None => {
                        all_const = false;
                        break;
                    }
                }
            }
            if !all_const {
                continue;
            }
            // Distinct indices only — a repeated element is not a simple sum.
            let mut sorted = idxs.clone();
            sorted.sort_unstable();
            sorted.dedup();
            if sorted.len() != idxs.len() {
                continue;
            }
            let Some(nm) = name else { continue };
            let Some(c) = colls.iter().find(|c| c.prop == nm).cloned() else {
                continue;
            };
            let mut ids: HashSet<String> = HashSet::default();
            self.collect_expr_idents(const_side, &mut ids);
            if ids
                .iter()
                .any(|n| rand_names.contains(n) || colls.iter().any(|c| &c.prop == n))
            {
                continue;
            }
            let Some(k) = self.eval_expr(const_side).to_i64() else {
                continue;
            };
            return Some((c, idxs, k as i128));
        }
        None
    }

    /// §18.5.9 — an lvalue reaching THROUGH a rand class-handle member
    /// (`sub_inst.sub_data`). Returns the sub-object's handle and the member
    /// name, so a cross-object equality can be forced onto it.
    pub(super) fn cross_obj_lvalue(
        &mut self,
        e: &Expression,
        rand_set: &HashSet<String>,
    ) -> Option<(usize, String)> {
        let (obj, field) = match &e.kind {
            ExprKind::Ident(h) if h.path.len() == 2 => {
                (h.path[0].name.name.clone(), h.path[1].name.name.clone())
            }
            ExprKind::MemberAccess { expr: base, member } => match &base.kind {
                ExprKind::Ident(h) if h.path.len() == 1 => {
                    (h.path[0].name.name.clone(), member.name.clone())
                }
                _ => return None,
            },
            _ => return None,
        };
        let this = self.this_stack.last().copied().flatten()?;
        // §18.4/§18.5.9: a rand field in a non-rand (or disabled) handle
        // remains state for this solve. Do not repair equalities by mutating
        // configuration objects reached through such handles.
        if !rand_set.contains(&obj) {
            return None;
        }
        self.prop_class_type(this, &obj)?;
        let sub = self
            .heap
            .get(this)
            .and_then(|o| o.as_ref())
            .and_then(|i| i.properties.get(&obj))
            .and_then(|v| v.to_u64())
            .unwrap_or(0) as usize;
        if sub == 0 || self.heap.get(sub).and_then(|o| o.as_ref()).is_none() {
            return None;
        }
        // Only a rand member of the sub-object may be forced.
        if !self.object_rand_set(sub).contains(&field) {
            return None;
        }
        Some((sub, field))
    }

    /// §18.5.5 / §18.5.9 — `unique {c}` over a rand collection: make every
    /// element pairwise distinct. Elements pinned by an equality constraint
    /// keep their value; the rest are re-picked from the element's legal
    /// domain (the `foreach … inside {…}` ranges, when any).
    pub(super) fn enforce_unique_coll(
        &mut self,
        c: &RandColl,
        constraints: &[ClassConstraint],
        pinned: &HashSet<String>,
    ) {
        use rand::Rng;
        let keys = self.coll_elem_keys(c);
        if keys.len() < 2 {
            return;
        }
        let allowed = self.elem_allowed_ranges(&c.prop, constraints);
        let domain: Vec<(u64, u64)> = if allowed.is_empty() {
            let hi = if c.width >= 64 {
                u64::MAX
            } else {
                (1u64 << c.width) - 1
            };
            vec![(0, hi)]
        } else {
            allowed
        };
        let total: u64 = domain.iter().map(|(l, h)| h - l + 1).sum();
        if total < keys.len() as u64 {
            return; // provably impossible — leave for the checker to reject
        }
        let mut used: HashSet<u64> = HashSet::default();
        // Pinned elements claim their value first, so a free element never
        // steals it and forces the pinned one to move.
        for k in &keys {
            if pinned.contains(k) {
                if let Some(v) = self.read_coll_elem(k).and_then(|v| v.to_u64()) {
                    used.insert(v);
                }
            }
        }
        for k in &keys {
            if pinned.contains(k) {
                continue;
            }
            let cur = self.read_coll_elem(k).and_then(|v| v.to_u64());
            let in_domain = cur.is_some_and(|v| domain.iter().any(|(l, h)| v >= *l && v <= *h));
            if let Some(v) = cur {
                if in_domain && used.insert(v) {
                    continue; // already distinct and legal
                }
            }
            // Re-pick: random draws from the legal domain, then an exhaustive
            // sweep so a tight domain still converges.
            let mut pick: Option<u64> = None;
            for _ in 0..256 {
                let (l, h) = domain[self.rng.gen_range(0..domain.len())];
                let v = if h > l { self.rng.gen_range(l..=h) } else { l };
                if !used.contains(&v) {
                    pick = Some(v);
                    break;
                }
            }
            if pick.is_none() {
                'sweep: for (l, h) in &domain {
                    let mut v = *l;
                    loop {
                        if !used.contains(&v) {
                            pick = Some(v);
                            break 'sweep;
                        }
                        if v == *h {
                            break;
                        }
                        v += 1;
                    }
                }
            }
            if let Some(v) = pick {
                used.insert(v);
                self.write_coll_elem(k, Value::from_u64(v, c.width));
            }
        }
    }

    /// Strict acceptance test for a constraint item that targets a rand
    /// COLLECTION. `None` means "not one of ours" — the caller falls back to
    /// the legacy (unmodeled-skipping) check.
    ///
    /// Covers, per §18.4/§18.5.5/§18.5.7:
    ///   * `a.size() == / inside / < …` on a sized dynamic array,
    ///   * `foreach (a[i]) …` and `foreach (a[i, j]) …` bodies, element by
    ///     element with the loop variables bound,
    ///   * `unique {a}` distinctness.
    pub(super) fn coll_item_check(
        &mut self,
        handle: usize,
        item: &ConstraintItem,
        colls: &[RandColl],
    ) -> Option<bool> {
        match item {
            ConstraintItem::Unique { exprs, .. } => {
                let name = match exprs.first().map(|e| &e.kind) {
                    Some(ExprKind::Ident(h)) => h.path.last().map(|s| s.name.name.clone())?,
                    _ => return None,
                };
                let c = colls.iter().find(|c| c.prop == name)?.clone();
                Some(self.coll_is_unique(&c))
            }
            ConstraintItem::Foreach {
                array,
                vars,
                item: body,
                ..
            } => {
                let name = Self::foreach_base_name(array)?;
                let c = colls.iter().find(|c| c.prop == name)?.clone();
                if c.kind == CollKind::Fixed {
                    return None; // legacy path owns fixed rand arrays
                }
                let idx: Vec<Option<String>> = vars
                    .iter()
                    .map(|v| v.as_ref().map(|i| i.name.clone()))
                    .collect();
                let body = (**body).clone();
                Some(self.check_coll_foreach(handle, &c, &idx, &body))
            }
            ConstraintItem::Expr(_) | ConstraintItem::Inside { .. } => {
                // A `.size()` constraint on a collection we sized is checkable.
                let names: Vec<String> = colls
                    .iter()
                    .filter(|c| c.kind == CollKind::Dyn)
                    .map(|c| c.prop.clone())
                    .collect();
                let recv = self.active_rand_receiver();
                let touches = names
                    .iter()
                    .any(|n| Self::item_has_size_call(item, n, recv));
                if !touches {
                    return None;
                }
                Some(self.check_constraint_item(handle, item))
            }
            _ => None,
        }
    }

    /// Does the item contain a `<prop>.size()` / `<prop>[i].size()` call?
    pub(super) fn item_has_size_call(
        item: &ConstraintItem,
        prop: &str,
        recv: Option<&str>,
    ) -> bool {
        fn in_expr(e: &Expression, prop: &str, recv: Option<&str>) -> bool {
            if Simulator::is_size_call(e, prop, false, recv)
                || Simulator::is_size_call(e, prop, true, recv)
            {
                return true;
            }
            match &e.kind {
                ExprKind::Binary { left, right, .. } => {
                    in_expr(left, prop, recv) || in_expr(right, prop, recv)
                }
                ExprKind::Unary { operand, .. } => in_expr(operand, prop, recv),
                ExprKind::Paren(i) => in_expr(i, prop, recv),
                ExprKind::Inside { expr, ranges } => {
                    in_expr(expr, prop, recv) || ranges.iter().any(|r| in_expr(r, prop, recv))
                }
                _ => false,
            }
        }
        match item {
            ConstraintItem::Expr(e) => in_expr(e, prop, recv),
            ConstraintItem::Inside { expr, .. } => in_expr(expr, prop, recv),
            _ => false,
        }
    }

    /// Evaluate a `foreach` body against every element of a collection.
    pub(super) fn check_coll_foreach(
        &mut self,
        handle: usize,
        c: &RandColl,
        idx_names: &[Option<String>],
        body: &ConstraintItem,
    ) -> bool {
        let mut ok = true;
        match c.kind {
            CollKind::Assoc => {
                for key in self.assoc_key_strs(&c.scoped) {
                    let mut frame: HashMap<String, Value> = HashMap::default();
                    if let Some(Some(n)) = idx_names.first() {
                        let kv = if self.is_string_keyed_array(&c.scoped) {
                            Value::from_string(&key)
                        } else {
                            Value::from_u64(key.parse::<i64>().unwrap_or(0) as u64, 32)
                        };
                        frame.insert(n.clone(), kv);
                    }
                    self.push_local_frame(frame);
                    let good = self.item_holds(handle, body);
                    self.pop_local_frame();
                    if !good {
                        ok = false;
                        break;
                    }
                }
            }
            // §7.4.5: rows with fixed dimensions (`foreach (a[i, j, k])` on
            // `T a[][2][3]`): every loop variable walks its dimension.
            CollKind::Dyn if matches!(c.row_tail, Some(RowTail::Fixed(_))) => {
                let mut dims = vec![(0, self.coll_outer_size(c) as i64 - 1)];
                if let Some(RowTail::Fixed(inner)) = &c.row_tail {
                    dims.extend_from_slice(inner);
                }
                dims.truncate(idx_names.len().max(1));
                if dims.iter().any(|&(lo, hi)| hi < lo) {
                    return true;
                }
                let mut at: Vec<i64> = dims.iter().map(|d| d.0).collect();
                loop {
                    let mut frame: HashMap<String, Value> = HashMap::default();
                    for (d, n) in idx_names.iter().take(dims.len()).enumerate() {
                        if let Some(n) = n {
                            frame.insert(n.clone(), Self::signed_loop_val(at[d]));
                        }
                    }
                    self.push_local_frame(frame);
                    let good = self.item_holds(handle, body);
                    self.pop_local_frame();
                    if !good {
                        return false;
                    }
                    let mut d = dims.len();
                    loop {
                        if d == 0 {
                            return true;
                        }
                        d -= 1;
                        if at[d] < dims[d].1 {
                            at[d] += 1;
                            break;
                        }
                        at[d] = dims[d].0;
                    }
                }
            }
            _ => {
                let outer = self.coll_outer_size(c) as i64;
                'outer: for i in 0..outer {
                    let inner_n = if c.nested && idx_names.len() >= 2 {
                        self.get_queue_size(&format!("{}[{}]", c.scoped, i)) as i64
                    } else {
                        1
                    };
                    for j in 0..inner_n {
                        let mut frame: HashMap<String, Value> = HashMap::default();
                        if let Some(Some(n)) = idx_names.first() {
                            frame.insert(n.clone(), Self::signed_loop_val(i));
                        }
                        if c.nested && idx_names.len() >= 2 {
                            if let Some(Some(n)) = idx_names.get(1) {
                                frame.insert(n.clone(), Self::signed_loop_val(j));
                            }
                        }
                        self.push_local_frame(frame);
                        let good = self.item_holds(handle, body);
                        self.pop_local_frame();
                        if !good {
                            ok = false;
                            break 'outer;
                        }
                    }
                }
            }
        }
        ok
    }

    /// §18.5.4/§18.5.10 — does the DISTRIBUTION of this constraint set need
    /// the joint solver? A `solve … before` ordering, a `dist` inside an
    /// `if`/`->`, or a `dist` on a variable that another constraint also
    /// reads: the trials draw each variable on its own and repair the rest,
    /// which ignores the ordering and skews the weights (a repair lands on
    /// values of whichever item the other constraints leave room for).
    pub(super) fn rand_order_sensitive(&self, constraints: &[ClassConstraint]) -> bool {
        fn has_any(it: &ConstraintItem) -> bool {
            match it {
                ConstraintItem::Solve { .. } => true,
                ConstraintItem::Inside { is_dist, .. } => *is_dist,
                ConstraintItem::Implication { constraint, .. } => has_any(constraint),
                ConstraintItem::IfElse {
                    then_item,
                    else_item,
                    ..
                } => has_any(then_item) || else_item.as_ref().is_some_and(|e| has_any(e)),
                ConstraintItem::Foreach { item, .. } | ConstraintItem::Soft(item) => has_any(item),
                ConstraintItem::Block(items) => items.iter().any(has_any),
                ConstraintItem::Expr(_) | ConstraintItem::Unique { .. } => false,
            }
        }
        if !constraints.iter().any(|c| c.items.iter().any(has_any)) {
            return false;
        }
        // §18.5.4: a `dist` over values wider than 64 bits is weighed by the
        // joint solver; the trial loop compares only 64 bits of each item.
        fn wide_dist(it: &ConstraintItem) -> bool {
            let wide = |e: &Expression| matches!(&e.kind, ExprKind::Number(NumberLiteral::Integer { size: Some(n), .. }) if *n > 64);
            match it {
                ConstraintItem::Inside {
                    is_dist: true,
                    range,
                    ..
                } => range.iter().any(|r| match r {
                    ConstraintRange::Value(v) => wide(v),
                    ConstraintRange::Range { lo, hi } => wide(lo) || wide(hi),
                }),
                ConstraintItem::Implication { constraint, .. } => wide_dist(constraint),
                ConstraintItem::IfElse {
                    then_item,
                    else_item,
                    ..
                } => wide_dist(then_item) || else_item.as_ref().is_some_and(|e| wide_dist(e)),
                ConstraintItem::Foreach { item, .. } | ConstraintItem::Soft(item) => {
                    wide_dist(item)
                }
                ConstraintItem::Block(items) => items.iter().any(wide_dist),
                _ => false,
            }
        }
        if constraints.iter().any(|c| c.items.iter().any(wide_dist)) {
            return true;
        }
        // (dist targets, names read elsewhere); true when decided early.
        fn walk(
            me: &Simulator,
            it: &ConstraintItem,
            guarded: bool,
            dist: &mut HashSet<String>,
            other: &mut HashSet<String>,
        ) -> bool {
            match it {
                ConstraintItem::Solve { .. } => true,
                ConstraintItem::Inside {
                    expr,
                    range,
                    is_dist: true,
                    ..
                } => {
                    // The variable the dist draws: a bare rand property, an
                    // element of one, or (§18.4) a member or a select of a
                    // member of an aggregate one (`s.f`, `s.f[3:0]`) — keyed
                    // by its root property. The per-variable trials pick a
                    // member's dist with its weights; the joint solver treats
                    // the aggregate as one scalar and drew it uniformly.
                    // A member target is keyed by its member PATH (`s.f`,
                    // selects dropped) and judged against member paths below:
                    // a constraint on a sibling member (`s.g < 10`) does not
                    // couple it, and routing such a set to the joint solver
                    // lost the weights.
                    let base = match Simulator::member_path_key(expr) {
                        Some(k) if k.contains('.') => format!("{}{}", MEMBER_DIST_MARK, k),
                        _ => {
                            let base = match &Simulator::unparen(expr).kind {
                                ExprKind::Index { expr: b, .. } => Simulator::plain_ident_name(b),
                                _ => Simulator::plain_ident_name(expr),
                            };
                            let Some(base) = base else {
                                return true;
                            };
                            base
                        }
                    };
                    if guarded {
                        return true;
                    }
                    dist.insert(base);
                    for r in range {
                        match r {
                            ConstraintRange::Value(e) => me.collect_expr_idents(e, other),
                            ConstraintRange::Range { lo, hi } => {
                                me.collect_expr_idents(lo, other);
                                me.collect_expr_idents(hi, other);
                            }
                        }
                    }
                    false
                }
                ConstraintItem::Implication {
                    condition,
                    constraint,
                    ..
                } => {
                    me.collect_expr_idents(condition, other);
                    walk(me, constraint, true, dist, other)
                }
                ConstraintItem::IfElse {
                    condition,
                    then_item,
                    else_item,
                    ..
                } => {
                    me.collect_expr_idents(condition, other);
                    walk(me, then_item, true, dist, other)
                        || else_item
                            .as_ref()
                            .is_some_and(|e| walk(me, e, true, dist, other))
                }
                ConstraintItem::Foreach { item, .. } | ConstraintItem::Soft(item) => {
                    walk(me, item, guarded, dist, other)
                }
                ConstraintItem::Block(items) => items
                    .iter()
                    .fold(false, |hit, i| walk(me, i, guarded, dist, other) || hit),
                _ => {
                    me.collect_item_idents(it, other);
                    false
                }
            }
        }
        let (mut dist, mut other) = (HashSet::default(), HashSet::default());
        for con in constraints {
            for it in &con.items {
                if walk(self, it, false, &mut dist, &mut other) {
                    return true;
                }
            }
        }
        let member_dists: Vec<&str> = dist
            .iter()
            .filter_map(|d: &String| d.strip_prefix(MEMBER_DIST_MARK))
            .collect();
        if !member_dists.is_empty() {
            let mut paths: Vec<String> = Vec::new();
            for con in constraints {
                for it in &con.items {
                    Self::collect_item_member_paths(it, &mut paths);
                }
            }
            // Coupled: another reference to the same member, to a member
            // below it, or to an aggregate that contains it.
            let coupled = |k: &str| {
                paths.iter().any(|o| {
                    o == k
                        || k.strip_prefix(o.as_str())
                            .is_some_and(|r| r.starts_with('.'))
                        || o.strip_prefix(k).is_some_and(|r| r.starts_with('.'))
                })
            };
            if member_dists.iter().any(|k| coupled(k)) {
                return true;
            }
        }
        dist.iter().any(|d| other.contains(d))
    }

    /// `root.m1.m2` for a member chain (flat ident or `MemberAccess`, index
    /// and part selects dropped), `root` for a bare or indexed name.
    pub(super) fn member_path_key(e: &Expression) -> Option<String> {
        let mut cur = Self::unparen(e);
        let mut members: Vec<&str> = Vec::new();
        loop {
            match &cur.kind {
                ExprKind::Index { expr: b, .. } | ExprKind::RangeSelect { expr: b, .. } => cur = b,
                ExprKind::MemberAccess { expr: b, member } => {
                    members.push(member.name.as_str());
                    cur = b;
                }
                _ => break,
            }
        }
        let ExprKind::Ident(h) = &cur.kind else {
            return None;
        };
        if h.root.is_some() {
            return None;
        }
        let mut parts: Vec<&str> = h.path.iter().map(|s| s.name.name.as_str()).collect();
        parts.extend(members.into_iter().rev());
        Some(parts.join("."))
    }

    /// Every member path (`member_path_key`) a constraint item reads, except
    /// a `dist`'s own target (its range bounds count).
    pub(super) fn collect_item_member_paths(item: &ConstraintItem, out: &mut Vec<String>) {
        fn expr(e: &Expression, out: &mut Vec<String>) {
            match &e.kind {
                ExprKind::Ident(_) | ExprKind::MemberAccess { .. } => {
                    if let Some(k) = Simulator::member_path_key(e) {
                        out.push(k);
                    }
                    if let ExprKind::Ident(h) = &e.kind {
                        for seg in &h.path {
                            for sel in &seg.selects {
                                expr(sel, out);
                            }
                        }
                    }
                }
                ExprKind::Index { expr: b, index } => {
                    expr(b, out);
                    expr(index, out);
                }
                ExprKind::RangeSelect {
                    expr: b,
                    left,
                    right,
                    ..
                } => {
                    expr(b, out);
                    expr(left, out);
                    expr(right, out);
                }
                ExprKind::Unary { operand, .. } => expr(operand, out),
                ExprKind::Binary { left, right, .. } => {
                    expr(left, out);
                    expr(right, out);
                }
                ExprKind::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                } => {
                    expr(condition, out);
                    expr(then_expr, out);
                    expr(else_expr, out);
                }
                ExprKind::Paren(i) => expr(i, out),
                ExprKind::Concatenation(v) => v.iter().for_each(|x| expr(x, out)),
                ExprKind::Call { func, args } => {
                    expr(func, out);
                    args.iter().for_each(|x| expr(x, out));
                }
                ExprKind::SystemCall { args, .. } => args.iter().for_each(|x| expr(x, out)),
                ExprKind::Inside { expr: x, ranges } => {
                    expr(x, out);
                    ranges.iter().for_each(|r| expr(r, out));
                }
                ExprKind::Range(a, b) => {
                    expr(a, out);
                    expr(b, out);
                }
                _ => {}
            }
        }
        fn ranges(rs: &[ConstraintRange], out: &mut Vec<String>) {
            for r in rs {
                match r {
                    ConstraintRange::Value(e) => expr(e, out),
                    ConstraintRange::Range { lo, hi } => {
                        expr(lo, out);
                        expr(hi, out);
                    }
                }
            }
        }
        match item {
            ConstraintItem::Expr(e) => expr(e, out),
            ConstraintItem::Inside {
                expr: x,
                range,
                is_dist,
                ..
            } => {
                if !*is_dist {
                    expr(x, out);
                }
                ranges(range, out);
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                expr(condition, out);
                Simulator::collect_item_member_paths(constraint, out);
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                expr(condition, out);
                Simulator::collect_item_member_paths(then_item, out);
                if let Some(e) = else_item {
                    Simulator::collect_item_member_paths(e, out);
                }
            }
            ConstraintItem::Foreach { array, item, .. } => {
                expr(array, out);
                Simulator::collect_item_member_paths(item, out);
            }
            ConstraintItem::Soft(i) => Simulator::collect_item_member_paths(i, out),
            ConstraintItem::Block(items) => items
                .iter()
                .for_each(|i| Simulator::collect_item_member_paths(i, out)),
            ConstraintItem::Unique { exprs, .. } => exprs.iter().for_each(|e| expr(e, out)),
            ConstraintItem::Solve { .. } => {}
        }
    }

    /// §18.5.10 — is uniform rejection sampling the right solver for this
    /// class? It reproduces the LRM's solution-space distribution exactly, but
    /// only where every constraint is (a) checkable and (b) free of the
    /// mechanisms whose semantics are NOT "uniform over the solution space":
    ///
    ///   * `solve X before Y`      — the ordering hint deliberately REPLACES
    ///                               solution-space weighting (§18.5.10); the
    ///                               propagation solver already draws X first.
    ///   * `dist`                  — weighted by construction (§18.5.4).
    ///   * `soft`                  — may legally be violated (§18.5.14), so a
    ///                               reject-on-violation loop would over-reject.
    ///   * `randc`                 — cycles, not independent draws (§18.4.3).
    ///   * collections / handles   — solved by the dedicated pipeline.
    ///   * `real`                  — not a finite domain.
    ///
    /// Requiring at least one CONDITIONAL item keeps the pre-pass off the hot
    /// path of classes whose constraints are independent per variable, where
    /// propagation already yields the correct (uniform) distribution.
    pub(super) fn rejection_sample_ok(
        &self,
        constraints: &[ClassConstraint],
        rand_props: &[(String, u32)],
        randc_set: &HashSet<String>,
        real_rand_props: &HashSet<String>,
        colls: &[RandColl],
        handles: &[String],
    ) -> bool {
        if rand_props.is_empty()
            || !colls.is_empty()
            || !handles.is_empty()
            || !randc_set.is_empty()
            || !real_rand_props.is_empty()
        {
            return false;
        }
        // A wide domain makes uniform rejection hopeless; the propagation
        // solver handles those (and is what the tests expect there).
        if rand_props.iter().any(|(_, w)| *w > 16) {
            return false;
        }
        let mut has_cond = false;
        fn scan(item: &ConstraintItem, has_cond: &mut bool) -> bool {
            match item {
                ConstraintItem::Solve { .. } | ConstraintItem::Soft(_) => false,
                ConstraintItem::Inside { is_dist, .. } => !*is_dist,
                ConstraintItem::Implication { constraint, .. } => {
                    *has_cond = true;
                    scan(constraint, has_cond)
                }
                ConstraintItem::IfElse {
                    then_item,
                    else_item,
                    ..
                } => {
                    *has_cond = true;
                    scan(then_item, has_cond)
                        && else_item.as_ref().is_none_or(|e| scan(e, has_cond))
                }
                ConstraintItem::Block(items) => items.iter().all(|i| scan(i, has_cond)),
                ConstraintItem::Foreach { .. } | ConstraintItem::Unique { .. } => false,
                ConstraintItem::Expr(e) => {
                    if matches!(
                        &e.kind,
                        ExprKind::Binary {
                            op: BinaryOp::LogImplies,
                            ..
                        }
                    ) {
                        *has_cond = true;
                    }
                    !Simulator::expr_unmodeled(e)
                }
            }
        }
        for con in constraints {
            for it in &con.items {
                if !scan(it, &mut has_cond) {
                    return false;
                }
            }
        }
        has_cond
    }

    /// True when every element of the collection holds a distinct value.
    pub(super) fn coll_is_unique(&mut self, c: &RandColl) -> bool {
        let keys = self.coll_elem_keys(c);
        let mut seen: HashSet<u64> = HashSet::default();
        for k in keys {
            let v = self
                .read_coll_elem(&k)
                .and_then(|v| v.to_u64())
                .unwrap_or(0);
            if !seen.insert(v) {
                return false;
            }
        }
        true
    }

    pub(super) fn randomize_budget_setting(&self, key: &str, default: u64) -> u64 {
        self.plusargs
            .iter()
            .find_map(|arg| {
                arg.trim_start_matches('+')
                    .strip_prefix(key)
                    .and_then(|value| value.parse::<u64>().ok())
            })
            .filter(|value| *value != 0)
            .unwrap_or(default)
    }

    pub(super) fn begin_randomize_budget(&mut self) {
        if self.randomize_budget_depth == 0 {
            let trials = self.randomize_budget_setting("xezim_rand_trials=", 1000);
            let timeout = self.randomize_budget_setting("xezim_rand_timeout=", 30);
            self.randomize_budget_remaining = trials;
            self.randomize_budget_deadline =
                Some(std::time::Instant::now() + std::time::Duration::from_secs(timeout));
            self.randomize_budget_exhausted = false;
        }
        self.randomize_budget_depth += 1;
    }

    pub(super) fn end_randomize_budget(&mut self) {
        self.randomize_budget_depth = self.randomize_budget_depth.saturating_sub(1);
        if self.randomize_budget_depth == 0 {
            self.randomize_budget_deadline = None;
        }
    }

    /// True when the current randomize() call has passed its
    /// `+xezim_rand_timeout` deadline (no deadline armed → false). Used
    /// inside long per-element retry/sweep loops so a single trial cannot
    /// grind unboundedly past the budget.
    pub(super) fn rand_past_deadline(&self) -> bool {
        self.randomize_budget_deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
    }

    pub(super) fn take_randomize_budget(
        &mut self,
        handle: usize,
        class_name: &str,
        constraints: &[ClassConstraint],
    ) -> bool {
        let timed_out = self
            .randomize_budget_deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline);
        if self.randomize_budget_exhausted || timed_out || self.randomize_budget_remaining == 0 {
            if !self.randomize_budget_exhausted {
                self.randomize_budget_exhausted = true;
                let mut failed_names = Vec::new();
                for constraint in constraints {
                    if constraint.items.iter().any(|item| {
                        !Self::constraint_unmodeled(item)
                            && !self.check_constraint_item(handle, item)
                    }) {
                        failed_names.push(constraint.name.name.clone());
                    }
                }
                failed_names.extend(self.failed_child_constraint_names(handle));
                failed_names.sort();
                failed_names.dedup();
                let names = failed_names.join(", ");
                eprintln!(
                    "[xezim][error] randomize budget exhausted in class {}; last failed constraints: {}; returning 0 and restoring prior values",
                    class_name,
                    if names.is_empty() { "<none>" } else { &names }
                );
            }
            return false;
        }
        self.randomize_budget_remaining -= 1;
        true
    }
}

#[cfg(test)]
mod randc_permute_tests {
    use super::Simulator;

    /// The wide-randc cycle visits every value of its width exactly once.
    #[test]
    fn permute_is_a_bijection() {
        for width in 17..=20u32 {
            let n = 1usize << width;
            let mut seen = vec![false; n];
            for x in 0..n as u64 {
                let v = Simulator::randc_permute(x, 0x1234_5678_9abc_def1, width) as usize;
                assert!(v < n && !seen[v], "width {width}: {x} -> {v}");
                seen[v] = true;
            }
        }
    }
}
