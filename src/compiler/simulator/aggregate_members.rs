//! Simulator aggregate members.

use super::*;

impl Simulator {
    #[cold]
    #[inline(never)]
    pub(super) fn init_struct_coll_member_names(&self) -> &HashSet<String> {
        self.struct_coll_member_prop_names.get_or_init(|| {
            let mut set: HashSet<String> = HashSet::default();
            let mut work: Vec<DataType> = Vec::new();
            for cd in self.module.classes.values() {
                work.extend(cd.property_types.values().cloned());
            }
            work.extend(self.module.typedef_types.values().cloned());
            // Inline struct types of module / package / local variables
            // (`struct { int q[$]; } s;`) appear in no typedef table. Only
            // the unpacked-struct ones can contribute.
            for dt in self.module.var_decl_types.values() {
                if matches!(self.resolve_dt_ref(dt), DataType::Struct(su) if !su.packed) {
                    work.push(dt.clone());
                }
            }
            let mut budget = 65536usize;
            while let Some(dt) = work.pop() {
                if budget == 0 {
                    break;
                }
                budget -= 1;
                if let Some(su) = self.unpacked_struct_of(&dt) {
                    for m in &su.members {
                        for d in &m.declarators {
                            // §7.4.5: a fixed array OF collections
                            // (`T sp[4][$]`) names a collection per element.
                            if Self::coll_kind_of(&d.dimensions).is_some()
                                || d.dimensions.last().is_some_and(|l| {
                                    Self::coll_kind_of(std::slice::from_ref(l)).is_some()
                                })
                            {
                                set.insert(d.name.name.clone());
                            }
                        }
                        work.push(m.data_type.clone());
                    }
                }
            }
            let mask = set.iter().fold(0u64, |m, n| m | (1u64 << (n.len() & 63)));
            self.struct_coll_member_len_mask.set(mask);
            set
        })
    }

    /// The property name `class_prop_receiver` would resolve `e` to.
    pub(super) fn receiver_prop_name(e: &Expression) -> Option<&str> {
        match &e.kind {
            ExprKind::MemberAccess { member, .. } => Some(member.name.as_str()),
            ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => {
                h.path.last().map(|s| s.name.name.as_str())
            }
            _ => None,
        }
    }

    /// Could any receiver/property split of the member/index chain `e` name
    /// a struct property? Every candidate property is a member or segment
    /// name of the chain. A part-select (`obj.mem[hi:lo]`) peels like an
    /// index: the property sits below the trailing select.
    pub(super) fn chain_may_name_struct_prop(&self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::MemberAccess { expr, member } => {
                self.struct_prop_name_possible(&member.name)
                    || self.chain_may_name_struct_prop(expr)
            }
            ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => {
                self.chain_may_name_struct_prop(expr)
            }
            ExprKind::Ident(h) => h
                .path
                .iter()
                .any(|s| self.struct_prop_name_possible(&s.name.name)),
            _ => false,
        }
    }

    pub(super) fn class_prop_receiver(&mut self, base: &Expression) -> Option<(usize, String)> {
        if let ExprKind::Ident(hier) = &base.kind
            && hier.path.len() > 2
            && let Some(h) = self.hier_parent_handle(hier)
        {
            return Some((h, hier.path.last()?.name.name.clone()));
        }
        if let Some((obj, prop)) = Self::split_trailing_member(base) {
            // §8.9: `Cls::prop` of a static struct names its shared store.
            if !self.static_struct_names.is_empty()
                && self.static_struct_names.contains(prop.as_str())
                && let ExprKind::Ident(oh) = &obj.kind
                && oh.path.len() == 1
                && oh.path[0].selects.is_empty()
                && self.module.classes.contains_key(&oh.path[0].name.name)
                && !self
                    .signal_name_to_id
                    .contains_key(oh.path[0].name.name.as_str())
                && !self
                    .local_stack
                    .last()
                    .is_some_and(|l| l.contains_key(&oh.path[0].name.name))
            {
                let cls = oh.path[0].name.name.clone();
                return self.static_struct_holder(&cls, &prop).map(|h| (h, prop));
            }
            // §8.25/§8.9: `C#(args)::s` / `alias_t::s` (a typedef of a
            // specialization) names THAT specialization's static struct.
            if !self.static_struct_names.is_empty()
                && self.static_struct_names.contains(prop.as_str())
            {
                let spec = match &obj.kind {
                    ExprKind::Specialization { .. } => self.resolve_call_spec_params(
                        Self::extract_call_spec(&obj),
                        &self.current_spec,
                    ),
                    ExprKind::Ident(oh)
                        if oh.path.len() == 1
                            && oh.path[0].selects.is_empty()
                            && !self.module.classes.contains_key(&oh.path[0].name.name)
                            && !self
                                .local_stack
                                .last()
                                .is_some_and(|l| l.contains_key(&oh.path[0].name.name))
                            && self
                                .class_member_names()
                                .scope_prefixes
                                .contains(&oh.path[0].name.name) =>
                    {
                        self.scope_prefix_spec(&oh.path[0].name.name)
                    }
                    _ => None,
                };
                if let Some((base, sig)) = spec {
                    let saved = self.current_spec.replace((base.clone(), sig));
                    let h = self.static_struct_holder(&base, &prop);
                    self.current_spec = saved;
                    return h.map(|h| (h, prop));
                }
            }
            let h = self.eval_expr(&obj).to_u64()? as usize;
            if h == 0 || self.heap.get(h).and_then(|o| o.as_ref()).is_none() {
                return None;
            }
            // §8.9: so does `obj.prop`.
            if !self.static_struct_names.is_empty()
                && self.static_struct_names.contains(prop.as_str())
            {
                let cls = self.heap.get(h)?.as_ref()?.class_name.clone();
                if let Some(sh) = self.static_struct_holder(&cls, &prop) {
                    return Some((sh, prop));
                }
            }
            return Some((h, prop));
        }
        let ExprKind::Ident(h) = &base.kind else {
            return None;
        };
        if h.path.len() != 1 || !h.path[0].selects.is_empty() {
            return None;
        }
        // A local of the same name shadows the property (§8.10).
        let nm = &h.path[0].name.name;
        if self.local_stack.last().is_some_and(|l| l.contains_key(nm)) {
            return None;
        }
        let this = self
            .this_stack
            .last()
            .copied()
            .flatten()
            .filter(|&h| h != 0);
        // §8.9: a static struct inside a method (a static one has no `this`).
        if !self.static_struct_names.is_empty() && self.static_struct_names.contains(nm.as_str()) {
            let cls = match this {
                Some(h) => self
                    .heap
                    .get(h)
                    .and_then(|o| o.as_ref())
                    .map(|i| i.class_name.clone()),
                None => self.class_context_stack.last().cloned().flatten(),
            };
            if let Some(cls) = cls
                && let Some(sh) = self.static_struct_holder(&cls, nm)
            {
                return Some((sh, nm.clone()));
            }
        }
        let handle = this?;
        Some((handle, nm.clone()))
    }

    /// §8.9: the shared store of static property `prop` reached from class
    /// `start_class` when it is an UNPACKED struct (see
    /// `static_struct_holders`), created and seeded with its members' §6.8
    /// defaults on first use. `None` for any other property.
    pub(super) fn static_struct_holder(&mut self, start_class: &str, prop: &str) -> Option<usize> {
        let key = self.static_prop_key(start_class, prop)?;
        if let Some(&h) = self.static_struct_holders.get(&key) {
            // `usize::MAX`: the property is no unpacked struct.
            return (h != usize::MAX).then_some(h);
        }
        let su = (|| {
            let head = key.strip_suffix(prop)?.strip_suffix("::")?;
            let decl = head.split('#').next()?.to_string();
            let cd = self.module.classes.get(&decl)?.clone();
            // A static COLLECTION of structs (`static s_t q[$]`) keeps the
            // collection storage paths.
            if !cd.static_properties.contains(prop)
                || cd.static_collections.iter().any(|(n, ..)| n == prop)
                || cd.static_fixed_arrays.iter().any(|(n, ..)| n == prop)
                || cd.queue_properties.contains_key(prop)
                || cd.assoc_properties.contains_key(prop)
                || cd.array_properties.contains_key(prop)
                || cd.array_nd_properties.contains_key(prop)
            {
                return None;
            }
            let dt = self.class_prop_decl_type(&cd, prop)?.clone();
            match self.resolve_dt(&dt) {
                DataType::Struct(su) if Self::spreads_member_wise(&su) => Some((decl, su)),
                _ => None,
            }
        })();
        let Some((decl, su)) = su else {
            self.static_struct_holders.insert(key, usize::MAX);
            return None;
        };
        // §8.25: a specialization's holder (`P#int::one`) remembers its
        // specialization, so its type-parameter members resolve.
        let spec = key
            .split_once("::")
            .and_then(|(head, _)| head.split_once('#'))
            .map(|(b, sig)| (b.to_string(), sig.to_string()));
        let h = self.heap.len();
        self.heap.push(Some(Box::new(ClassInstance {
            class_name: decl,
            properties: PropMap::default(),
            type_bindings: HashMap::default(),
            spec,
            creation_scope: String::new(),
        })));
        self.seed_class_struct_cells(h, prop, &su);
        self.static_struct_holders.insert(key, h);
        Some(h)
    }

    /// Create the store of every static unpacked-struct property up front,
    /// so `Cls::prop.m` resolves before any object exists.
    pub(super) fn init_static_struct_holders(&mut self) {
        let props: Vec<(String, String)> = self
            .module
            .classes
            .values()
            .flat_map(|cd| {
                cd.static_properties
                    .iter()
                    .map(|p| (cd.name.clone(), p.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        for (cls, prop) in props {
            let is_struct = self.module.classes.get(&cls).cloned().is_some_and(|cd| {
                !cd.static_collections.iter().any(|(n, ..)| *n == prop)
                    && !cd.static_fixed_arrays.iter().any(|(n, ..)| *n == prop)
                    && !cd.queue_properties.contains_key(&prop)
                    && !cd.assoc_properties.contains_key(&prop)
                    && !cd.array_properties.contains_key(&prop)
                    && !cd.array_nd_properties.contains_key(&prop)
                    && self.class_prop_decl_type(&cd, &prop).is_some_and(|dt| {
                    matches!(self.resolve_dt(dt), DataType::Struct(su) if Self::spreads_member_wise(&su))
                })
            });
            if !is_struct {
                continue;
            }
            self.static_struct_names.insert(prop.clone());
            if !self.class_is_parameterized(&cls) {
                let _ = self.static_struct_holder(&cls, &prop);
            }
        }
    }

    /// §7.4.1/§11.5.1: the declared (left, right) range of the class
    /// property a select's base names (`a`, `this.a`, `obj.a`) when that
    /// range is ascending or does not end at bit 0 (`logic [31:8] a`):
    /// select labels map relative to `right`. Class properties are not in the
    /// signal tables that give a module vector its declared range, so
    /// `a[31:16]` read bits 31..16 of the 24 stored ones.
    pub(super) fn class_prop_select_dim(&mut self, base: &Expression) -> Option<(i64, i64)> {
        if self.no_class_objects() {
            return None;
        }
        let (h, prop) = match self.class_prop_receiver(base) {
            Some(r) => r,
            // Inside a solve the receiver local of `obj.randomize() with
            // { obj.p[…] … }` is out of scope; it names the object being
            // randomized (`this`).
            None => {
                let (recv, prop) = Self::split_trailing_member(base)?;
                let rn = Self::plain_ident_name(&recv)?;
                if self.active_rand_receiver() != Some(rn.as_str()) {
                    return None;
                }
                (self.this_stack.last().copied().flatten()?, prop)
            }
        };
        let class = self.heap.get(h)?.as_ref()?.class_name.clone();
        let key = (class, prop);
        if let Some(hit) = self.class_prop_dim_cache.borrow().get(&key) {
            return *hit;
        }
        let mut found: Option<(i64, i64)> = None;
        let mut cur = Some(key.0.clone());
        let mut guard = 0;
        while let Some(cn) = cur {
            guard += 1;
            if guard > 64 {
                break;
            }
            let Some(cd) = self.module.classes.get(&cn) else {
                break;
            };
            if cd.properties.contains_key(&key.1) {
                if let Some(dt) = self.class_prop_decl_type(cd, &key.1) {
                    let dt =
                        super::elaborate::resolve_typedef_chain(dt, &self.module.typedef_types);
                    if let DataType::IntegerVector { dimensions, .. } = dt {
                        if let [crate::ast::types::PackedDimension::Range { left, right, .. }] =
                            dimensions.as_slice()
                        {
                            let params = self.instance_param_scope(h);
                            let l =
                                super::elaborate::const_eval_i64_with_params(left, Some(&params))
                                    .or_else(|| {
                                        super::elaborate::const_eval_i64_with_params(
                                            left,
                                            Some(&self.module.parameters),
                                        )
                                    });
                            let r =
                                super::elaborate::const_eval_i64_with_params(right, Some(&params))
                                    .or_else(|| {
                                        super::elaborate::const_eval_i64_with_params(
                                            right,
                                            Some(&self.module.parameters),
                                        )
                                    });
                            if let (Some(l), Some(r)) = (l, r) {
                                if l < r || r != 0 {
                                    found = Some((l, r));
                                }
                            }
                        }
                    }
                }
                break;
            }
            cur = cd.extends.clone();
        }
        // A type parameter can bind differently per specialization: cache
        // only a class-chain answer that does not depend on one.
        self.class_prop_dim_cache.borrow_mut().insert(key, found);
        found
    }

    /// The declared struct/union type of class property `prop` on `handle`.
    pub(super) fn class_prop_struct(
        &self,
        handle: usize,
        prop: &str,
    ) -> Option<crate::ast::types::StructUnionType> {
        if !self.struct_prop_name_possible(prop) {
            return None;
        }
        // An ANONYMOUS inline struct property (`struct packed {..} s;`) has no
        // typedef name to look up, but its declared type is retained verbatim
        // in `property_types` — use it directly; whole-value access worked
        // while `s.f` read x without this.
        let class_name: &str = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.as_str())
            .unwrap_or("");
        // The common answer for a scalar property is "not a struct"; it
        // depends only on the class chain and the name, so it is cached
        // per class. The walk below used to clone the class name per level
        // and allocate a cycle-guard set on every property access.
        //
        // EXCEPTION: a property whose declared type is a class TYPE
        // PARAMETER (`class pairi #(type T=int); T first;`) is NOT a
        // class-chain-only answer — whether it is an unpacked struct depends
        // on the CONCRETE type this instance bound to that parameter
        // (§8.25). All specializations of one class share the same
        // `class_name` (they differ only in `type_bindings`), so a negative
        // cached for one binding (e.g. `T->int`) would poison the other
        // (`T->struct s`) — `pairi#(int).first=5` then made `pairi#(s).first`
        // read x and an equality/`%p` fall apart. Such properties skip the
        // negative cache and re-resolve the concrete binding per instance.
        let is_type_param_prop = if class_name.is_empty() {
            false
        } else {
            let mut cur: Option<&str> = Some(class_name);
            let mut tp = false;
            let mut guard = 0;
            while let Some(cn) = cur {
                guard += 1;
                if guard > 64 {
                    break;
                }
                let Some(cd) = self.module.classes.get(cn) else {
                    break;
                };
                if cd.properties.contains_key(prop) {
                    tp = cd
                        .properties
                        .get(prop)
                        .and_then(|s| s.type_name.as_ref())
                        .is_some_and(|tn| cd.type_param_names.contains(tn));
                    break;
                }
                cur = cd.extends.as_deref();
            }
            tp
        };
        if !is_type_param_prop
            && !class_name.is_empty()
            && self
                .class_prop_struct_neg
                .borrow()
                .get(class_name)
                .is_some_and(|set| set.contains(prop))
        {
            return None;
        }
        if !class_name.is_empty() {
            let mut cur: Option<&str> = Some(class_name);
            let mut hops = 0usize;
            while let Some(cn) = cur {
                hops += 1;
                if hops > 64 {
                    break;
                }
                let cd = self.module.classes.get(cn)?;
                // A COLLECTION property (assoc/queue/array — incl. a STATIC
                // one, which elaboration gates out of the per-instance maps)
                // is NOT a scalar struct: its unpacked dimension indexes
                // first (§7.4.2). Answering the element type here routed
                // `ri_tab[key].scope = v` into the struct-leaf machinery —
                // stored on the instance, invisible to exists()/num()/
                // foreach. UVM's resource pool lost every scope that way, so
                // every uvm_config_db get silently missed (issue #113).
                // ASSOC/QUEUE collections (incl. statics, which store flat
                // globally) index their unpacked dimension through the
                // COLLECTION machinery. FIXED arrays of structs stay here:
                // their `[i]` element leaves are the struct machinery's job.
                if cd.assoc_properties.contains_key(prop)
                    || cd.queue_properties.contains_key(prop)
                    || cd.static_collections.iter().any(|(n, _, _)| n == prop)
                {
                    return None;
                }
                // A fixed-size STATIC array stores globally under its bare
                // name, so its `[i]` element leaves are NOT reachable through
                // the per-instance struct machinery either.
                if cd.static_fixed_arrays.iter().any(|(n, ..)| n == prop) {
                    return None;
                }
                if let Some(DataType::Struct(su)) = cd.property_types.get(prop) {
                    // §8.25: members typed by the class's type parameters
                    // take this object's bindings (`struct packed { T a; }`).
                    if !cd.type_param_names.is_empty()
                        && let Some(DataType::Struct(b)) = self
                            .bind_type_params_in(&DataType::Struct(su.clone()), &|n| {
                                self.obj_type_param(handle, cn, n)
                            })
                    {
                        return Some(b);
                    }
                    return Some(su.clone());
                }
                if cd.properties.contains_key(prop) {
                    break;
                }
                cur = cd.extends.as_deref();
            }
        }
        let verdict = (|| {
            let tn = self.class_prop_type_name(handle, prop)?;
            let dt = self.module.typedef_types.get(tn.as_str())?;
            match Self::resolve_type_ref(dt, &self.module.typedef_types) {
                DataType::Struct(su) => Some(su),
                _ => None,
            }
        })();
        // Don't record a negative for a TYPE-PARAMETER property: the verdict
        // depends on this instance's binding, and caching a `None` under the
        // (shared) class_name would poison sibling specializations that bound
        // the parameter to an actual struct.
        if verdict.is_none() && !class_name.is_empty() && !is_type_param_prop {
            self.class_prop_struct_neg
                .borrow_mut()
                .entry(class_name.to_string())
                .or_default()
                .insert(prop.to_string());
        }
        verdict
    }

    /// The CONCRETE type name a property's declared type denotes on this
    /// instance.
    ///
    /// For an ordinary property that is just its declared type name. For one
    /// typed by a class TYPE PARAMETER (`class C #(type T); T val;`) the
    /// declared name is the parameter, so resolve it through the bindings the
    /// object was constructed with (§8.25). Without this step a property
    /// declared `STRUCT_T st;` looked up the literal name `STRUCT_T` in the
    /// typedef table, missed, and read back x for every field even though the
    /// raw storage was correct.
    ///
    /// A parameter of an ANCESTOR (`class d extends comp_base #(byte)`, the
    /// property declared in `comp_base`) is bound by the `extends` clause, not
    /// by the object's own bindings, which only name the leaf class's
    /// parameters: carry it down the chain (`carried_type_binding`). Reading
    /// the leaf's bindings alone found nothing, so `T data` kept the width of
    /// `T`'s default type in every derived class (§8.25, §6.20.2).
    pub(super) fn class_prop_type_name(&self, handle: usize, prop: &str) -> Option<String> {
        let inst = self.heap.get(handle)?.as_ref()?;
        let (owner, prop) = prop
            .rsplit_once("::")
            .map_or((None, prop), |(owner, prop)| (Some(owner), prop));
        // The chain walked so far, on the stack: this runs on property
        // stores, and a heap Vec per call showed up in the UVM A/B.
        let first = self.module.classes.get(inst.class_name.as_str())?.as_ref();
        let mut buf: [&super::elaborate::ElaboratedClass; 32] = [first; 32];
        let mut n = 0usize;
        let mut cd = first;
        loop {
            buf[n] = cd;
            n += 1;
            let chain = &buf[..n];
            if owner.is_none_or(|owner| owner == cd.name)
                && let Some(sig) = cd.properties.get(prop)
            {
                let tn = sig.type_name.as_ref()?;
                if cd.type_param_names.iter().any(|t| t == tn) {
                    let k = chain.len() - 1;
                    if k > 0 {
                        if let Some(b) =
                            Self::carried_type_binding(&inst.type_bindings, chain, k, tn)
                        {
                            // A fragment naming a type parameter of the
                            // object's own class that the object leaves
                            // unbound stays unresolved, as before.
                            if inst.type_bindings.get(&b).is_none()
                                && chain[0].type_param_names.iter().any(|t| *t == b)
                            {
                                return None;
                            }
                            return Some(b);
                        }
                        // `extends comp_base` with the argument left out:
                        // the parameter's declared default (§6.20.2).
                        if let Some(b) = inst.type_bindings.get(tn) {
                            return Some(b.clone());
                        }
                        return cd
                            .type_param_defaults
                            .iter()
                            .find(|(n, _)| n == tn)
                            .map(|(_, d)| d.trim().to_string())
                            .filter(|d| !d.is_empty() && !cd.type_param_names.contains(d));
                    }
                    return inst.type_bindings.get(tn).cloned();
                }
                return Some(tn.clone());
            }
            let e = cd.extends.as_deref()?;
            if n == buf.len() || chain.iter().any(|c| c.name == e) {
                return None;
            }
            cd = self.module.classes.get(e)?.as_ref();
        }
    }

    /// Width to clamp a value to when a property's concrete type is `name`.
    ///
    /// `None` means "do not clamp": a `string` is variable-length, a class
    /// handle is not an integral, a real is not bit-truncated, and a type we
    /// cannot resolve must keep the previous unclamped behavior rather than be
    /// truncated to a guess.
    pub(super) fn concrete_type_width(&self, name: &str) -> Option<u32> {
        if name == "string"
            || matches!(name, "real" | "realtime" | "shortreal")
            || self.module.classes.contains_key(name)
        {
            return None;
        }
        // The parser drops the packed range of a type ARGUMENT: `#(logic
        // [63:0])` reaches here as the bare name `logic`, indistinguishable
        // from a 1-bit `#(logic)`. Clamping to 1 would silently destroy 63
        // bits, so the vector keywords stay unclamped (exactly the old
        // behavior) while the self-describing atoms below are clamped.
        if matches!(name, "bit" | "logic" | "reg") {
            return None;
        }
        if let Some(w) = atom_type_keyword_width(name) {
            return Some(w);
        }
        // A vector argument that kept its literal range (`bit[5:0]`, the
        // form `extends C #(bit [5:0])` records).
        if let Some(w) = vector_fragment_width(name) {
            return Some(w);
        }
        if let Some(&w) = self.module.typedefs.get(name) {
            return Some(w).filter(|w| *w > 0);
        }
        if let Some(dt) = self.module.typedef_types.get(name) {
            let dt = Self::resolve_type_ref(dt, &self.module.typedef_types);
            if matches!(dt, DataType::Real { .. }) {
                return None;
            }
            let w = resolve_type_width(
                &dt,
                Some(&self.module.parameters),
                Some(&self.module.typedefs),
            );
            return Some(w).filter(|w| *w > 0);
        }
        None
    }

    /// §8.25/§7.4.1: the width of a type naming a class TYPE PARAMETER that
    /// the active object or specialization binds (`T`, `T [1:0]`): the bound
    /// type's width times the packed dimensions written after it. `None`
    /// when `dt` is no such type or the binding has no clamp width.
    pub(super) fn bound_type_param_width(&self, dt: &DataType) -> Option<u32> {
        let DataType::TypeReference {
            name,
            dimensions,
            type_args,
            ..
        } = dt
        else {
            return None;
        };
        if name.has_scope() || !type_args.is_empty() {
            return None;
        }
        let b = self.resolve_type_param_binding(&name.name.name)?;
        let base = self.concrete_type_width(b.trim())?;
        if dimensions.is_empty() {
            return Some(base);
        }
        let params = match self.this_stack.last().copied().flatten() {
            Some(h) => self.instance_param_scope(h),
            None => self.module.parameters.clone(),
        };
        let mut w = u64::from(base);
        for d in dimensions {
            let crate::ast::types::PackedDimension::Range { left, right, .. } = d else {
                return None;
            };
            let l = crate::elaborate::const_eval_i64_with_params(left, Some(&params))?;
            let r = crate::elaborate::const_eval_i64_with_params(right, Some(&params))?;
            w = w.checked_mul((l - r).unsigned_abs() + 1)?;
        }
        u32::try_from(w).ok()
    }

    /// Whether property `prop` of `cd` is a packed struct or union, or an
    /// enum, whose members name one of `cd`'s TYPE parameters (§8.25).
    pub(super) fn agg_names_type_param(cd: &super::elaborate::ElaboratedClass, prop: &str) -> bool {
        fn names(dt: &DataType, tps: &[String]) -> bool {
            match dt {
                DataType::TypeReference { name, .. } => {
                    !name.has_scope() && tps.contains(&name.name.name)
                }
                DataType::Struct(su) => su.members.iter().any(|m| names(&m.data_type, tps)),
                DataType::Enum(et) => et.base_type.as_deref().is_some_and(|b| names(b, tps)),
                _ => false,
            }
        }
        !cd.type_param_names.is_empty()
            && match cd.property_types.get(prop) {
                Some(dt @ DataType::Struct(su)) if su.packed => names(dt, &cd.type_param_names),
                Some(dt @ DataType::Enum(_)) => names(dt, &cd.type_param_names),
                _ => false,
            }
    }

    /// The type the object `handle` binds to type parameter `tp` of its
    /// class or of the ancestor `decl` (through the `extends` arguments).
    pub(super) fn obj_type_param(&self, handle: usize, decl: &str, tp: &str) -> Option<String> {
        let inst = self.heap.get(handle)?.as_ref()?;
        let first = self.module.classes.get(inst.class_name.as_str())?.as_ref();
        let mut buf: [&super::elaborate::ElaboratedClass; 32] = [first; 32];
        let mut n = 0usize;
        let mut cd = first;
        loop {
            buf[n] = cd;
            n += 1;
            if cd.name == decl {
                let k = n - 1;
                if k == 0 {
                    return inst.type_bindings.get(tp).cloned();
                }
                return Self::carried_type_binding(&inst.type_bindings, &buf[..n], k, tp)
                    .or_else(|| inst.type_bindings.get(tp).cloned());
            }
            let e = cd.extends.as_deref()?;
            if n == buf.len() {
                return None;
            }
            cd = self.module.classes.get(e)?.as_ref();
        }
    }

    /// §8.25: the data type a class TYPE parameter's binding `b` names: a
    /// built-in atom, a design typedef, or otherwise a vector of the bound
    /// width, signedness and value set. `None` for a binding that is no
    /// integral type this can size (a string, real or class handle).
    pub(super) fn bound_type_dt(&self, b: &str) -> Option<DataType> {
        use crate::ast::types::{IntegerAtomType as A, IntegerVectorType as V, Signing};
        let span = crate::ast::Span::dummy();
        let (base, signing) = if let Some(base) = b.strip_suffix(" unsigned") {
            (base.trim_end(), Some(Signing::Unsigned))
        } else if let Some(base) = b.strip_suffix(" signed") {
            (base.trim_end(), Some(Signing::Signed))
        } else {
            (b, None)
        };
        let atom = match base {
            "byte" => Some(A::Byte),
            "shortint" => Some(A::ShortInt),
            "int" => Some(A::Int),
            "longint" => Some(A::LongInt),
            "integer" => Some(A::Integer),
            "time" => Some(A::Time),
            _ => None,
        };
        if let Some(kind) = atom {
            return Some(DataType::IntegerAtom {
                kind,
                signing,
                span,
            });
        }
        if self.module.typedef_types.contains_key(b) {
            return Some(DataType::TypeReference {
                name: crate::ast::types::TypeName {
                    scopes: Vec::new(),
                    name: crate::ast::Identifier {
                        name: b.to_string(),
                        span,
                    },
                    span,
                },
                dimensions: Vec::new(),
                type_args: Vec::new(),
                span,
            });
        }
        let w = self.concrete_type_width(b)?;
        let num = |v: u32| {
            Box::new(Expression::new(
                ExprKind::Number(crate::ast::expr::NumberLiteral::Integer {
                    size: None,
                    signed: false,
                    base: crate::ast::expr::NumberBase::Decimal,
                    value: v.to_string(),
                    cached_val: std::cell::Cell::new(None),
                }),
                span,
            ))
        };
        Some(DataType::IntegerVector {
            kind: if self.concrete_type_two_state(b) {
                V::Bit
            } else {
                V::Logic
            },
            signing: self
                .concrete_type_signed(b)
                .unwrap_or(false)
                .then_some(Signing::Signed),
            dimensions: vec![crate::ast::types::PackedDimension::Range {
                left: num(w.saturating_sub(1)),
                right: num(0),
                span,
            }],
            span,
        })
    }

    /// §8.25/§7.2: the width of a struct, union or enum type whose members
    /// name a class TYPE parameter the active object or specialization binds
    /// (`struct packed { T a; bit b; }`). `None` for any other type, or
    /// outside a parameterized class.
    pub(super) fn bound_agg_width(&mut self, dt: &DataType) -> Option<u32> {
        if !matches!(dt, DataType::Struct(_) | DataType::Enum(_)) {
            return None;
        }
        let in_param_class = self.current_spec.is_some()
            || self
                .this_stack
                .last()
                .copied()
                .flatten()
                .and_then(|h| self.heap.get(h)?.as_ref())
                .is_some_and(|i| !i.type_bindings.is_empty());
        if !in_param_class {
            return None;
        }
        let b = self.bind_type_params_in(dt, &|n| self.resolve_type_param_binding(n))?;
        let spec = self.params_with_class_spec();
        let params = spec.as_ref().unwrap_or(&self.module.parameters);
        let w = resolve_type_width(&b, Some(params), Some(&self.module.typedefs));
        Some(w).filter(|w| *w > 0)
    }

    /// §8.25/§7.2: `dt` — a struct, union or enum whose members name class
    /// TYPE parameters (`struct packed { T a; bit b; }`) — with each such
    /// reference replaced by the type `bind` resolves it to. `None` when
    /// nothing in `dt` was replaced.
    pub(super) fn bind_type_params_in(
        &self,
        dt: &DataType,
        bind: &dyn Fn(&str) -> Option<String>,
    ) -> Option<DataType> {
        match dt {
            DataType::TypeReference {
                name,
                dimensions,
                type_args,
                ..
            } if !name.has_scope() && type_args.is_empty() => {
                let b = bind(&name.name.name)?;
                let mut rep = self.bound_type_dt(b.trim())?;
                if dimensions.is_empty() {
                    return Some(rep);
                }
                // `T [1:0]`: the written dimensions are the outer ones.
                if matches!(rep, DataType::IntegerAtom { .. }) {
                    let b = b.trim();
                    let w = self.concrete_type_width(b)?;
                    rep = self.bound_type_dt(&format!(
                        "{}{}[{}:0]",
                        if self.concrete_type_two_state(b) {
                            "bit"
                        } else {
                            "logic"
                        },
                        if self.concrete_type_signed(b) == Some(true) {
                            " signed"
                        } else {
                            ""
                        },
                        w.saturating_sub(1)
                    ))?;
                }
                match &mut rep {
                    DataType::IntegerVector { dimensions: d, .. }
                    | DataType::TypeReference { dimensions: d, .. } => {
                        let mut outer = dimensions.clone();
                        outer.append(d);
                        *d = outer;
                        Some(rep)
                    }
                    _ => None,
                }
            }
            DataType::Struct(su) => {
                let mut out = su.clone();
                let mut any = false;
                for m in &mut out.members {
                    if let Some(t) = self.bind_type_params_in(&m.data_type, bind) {
                        m.data_type = t;
                        any = true;
                    }
                }
                any.then(|| DataType::Struct(out))
            }
            DataType::Enum(et) => {
                let b = et.base_type.as_deref()?;
                let t = self.bind_type_params_in(b, bind)?;
                let mut out = et.clone();
                out.base_type = Some(Box::new(t));
                Some(DataType::Enum(out))
            }
            _ => None,
        }
    }

    /// Whether the type a class type parameter is bound to (`byte`,
    /// `bit signed [7:0]`, a typedef name) is signed.
    pub(super) fn bound_type_is_signed(&self, b: &str) -> bool {
        matches!(b, "byte" | "shortint" | "int" | "longint" | "integer")
            || b.split_whitespace().nth(1) == Some("signed")
            || self
                .module
                .typedef_types
                .get(b)
                .is_some_and(super::elaborate::is_type_signed)
    }

    /// Declared (post-type-parameter-resolution) return type of a call's
    /// callee, resolving the receiver with the same rules as the result layout.
    /// The declared return type of `name` as the first class in `cls`'s
    /// lexical chain declares it (§8.20: a non-virtual bare call binds to
    /// the first defining class; an override keeps the prototype).
    fn class_method_return_type(&self, cls: &str, name: &str) -> Option<DataType> {
        use crate::ast::decl::ClassMethodKind as Kind;
        let mut seen: HashSet<String> = HashSet::default();
        let mut cur = cls.to_string();
        while let Some(cd) = self.module.classes.get(&cur) {
            if !seen.insert(cur.clone()) {
                break;
            }
            if let Some(md) = cd.methods.get(name) {
                return match &md.kind {
                    Kind::Function(f) | Kind::Extern(f) | Kind::PureVirtual(f) => {
                        Some(f.return_type.clone())
                    }
                    Kind::Task(_) => None,
                };
            }
            match &cd.extends {
                Some(b) => cur = b.clone(),
                None => break,
            }
        }
        None
    }

    pub(super) fn call_return_type(&mut self, func: &Expression) -> Option<DataType> {
        // Split the callee into (receiver expression, method name). A method
        // call parses as EITHER `MemberAccess(recv, m)` OR a flat hier ident
        // `Ident([c, rd])`, depending on context — handle both. A single
        // bare name is a free function.
        let (recv_expr, method_name): (Option<Expression>, String) = match &func.kind {
            ExprKind::Ident(h) if h.path.len() == 1 => (None, h.path[0].name.name.clone()),
            ExprKind::Ident(h)
                if h.path.len() >= 2 && h.path.iter().all(|s| s.selects.is_empty()) =>
            {
                let mut base = h.clone();
                let last = base.path.pop()?;
                base.cached_signal_id = std::cell::Cell::new(None);
                base.cached_resolved_name = std::cell::OnceCell::new();
                (
                    Some(Expression {
                        kind: ExprKind::Ident(base),
                        span: func.span,
                        cached_width: std::cell::Cell::new(None),
                    }),
                    last.name.name,
                )
            }
            ExprKind::MemberAccess { expr: recv, member } => {
                // Ident/This receivers are re-evaluated (side-effect-free) to
                // find the instance; a CALL receiver is resolved statically
                // from its declared return type below.
                if !matches!(
                    &recv.kind,
                    ExprKind::Ident(_) | ExprKind::This | ExprKind::Call { .. }
                ) {
                    return None;
                }
                (Some((**recv).clone()), member.name.clone())
            }
            _ => return None,
        };
        let ret: DataType = match recv_expr {
            // §8.20/§23.9: a bare call inside a class method binds to the
            // enclosing class chain before any module-scope function of
            // the same name (`index >= size()` in `queue_c` read the return
            // type of an unrelated `size`, turning the compare unsigned).
            None => self
                .class_context_stack
                .last()
                .cloned()
                .flatten()
                .and_then(|cls| self.class_method_return_type(&cls, &method_name))
                .or_else(|| {
                    self.module
                        .functions
                        .get(method_name.as_str())
                        .map(|f| f.return_type.clone())
                })?,
            Some(recv) => {
                if let ExprKind::Ident(h) = &recv.kind
                    && h.path.len() == 1
                    && self.module.packages.contains(&h.path[0].name.name)
                {
                    let key = format!("{}::{}", h.path[0].name.name, method_name);
                    let function = self.module.functions.get(&key).or_else(|| {
                        self.module.functions.get(&method_name).filter(|_| {
                            self.module.pkg_subr_owner.get(&method_name)
                                == Some(&h.path[0].name.name)
                        })
                    })?;
                    return Some(function.return_type.clone());
                }
                // A CALL receiver (`o.get().rd()`) cannot be re-evaluated to
                // find its instance without running it twice — resolve the
                // receiver's DECLARED return class statically instead. No
                // instance means no type-parameter bindings, so a type-param
                // return type bails below (handle == 0 guard) rather than
                // guess.
                let (handle, class_name) = if let ExprKind::Call { func: rf, .. } = &recv.kind {
                    let rt = self.call_return_type(rf)?;
                    let DataType::TypeReference { name, .. } = rt else {
                        return None;
                    };
                    let cn = name.name.name.clone();
                    if !self.module.classes.contains_key(&cn) {
                        return None;
                    }
                    (0usize, cn)
                } else {
                    let h = self.eval_expr(&recv).to_u64()? as usize;
                    if h == 0 {
                        return None;
                    }
                    (h, self.heap.get(h)?.as_ref()?.class_name.clone())
                };
                let member = &method_name;
                // Find the method's declaring class along the extends chain.
                let mut cur = Some(class_name);
                let mut seen: HashSet<String> = HashSet::default();
                let mut found: Option<(String, DataType)> = None;
                while let Some(cn) = cur {
                    if !seen.insert(cn.clone()) {
                        break;
                    }
                    let cd = self.module.classes.get(&cn)?;
                    if let Some(m) = cd.methods.get(member.as_str()) {
                        use crate::ast::decl::ClassMethodKind;
                        let f = match &m.kind {
                            ClassMethodKind::Function(f)
                            | ClassMethodKind::PureVirtual(f)
                            | ClassMethodKind::Extern(f) => f,
                            ClassMethodKind::Task(_) => return None,
                        };
                        found = Some((cn.clone(), f.return_type.clone()));
                        break;
                    }
                    cur = cd.extends.clone();
                }
                let (decl_cn, mut ret) = found?;
                if let DataType::TypeReference {
                    name, dimensions, ..
                } = &ret
                {
                    if dimensions.is_empty() {
                        let tn = name.name.name.clone();
                        let cd = self.module.classes.get(&decl_cn)?;
                        if cd.type_param_names.iter().any(|t| t == &tn) {
                            if handle == 0 {
                                // Statically-typed receiver: no instance, no
                                // type-parameter bindings.
                                return None;
                            }
                            let bound = self
                                .heap
                                .get(handle)?
                                .as_ref()?
                                .type_bindings
                                .get(&tn)?
                                .clone();
                            ret = self.module.typedef_types.get(bound.as_str())?.clone();
                        }
                    }
                }
                ret
            }
        };
        Some(ret)
    }

    /// Resolved PACKED struct type denoted by a member-access chain rooted in
    /// a CALL (`mk().n`, `mk().n.hi`): the call's return struct, descended
    /// through each named member. `None` when the root is not a call or any
    /// step is not a packed struct — callers fall through unchanged.
    /// Cheap screen for an expression that could address a class-property
    /// aggregate: a member access, or an index/ident path that could name a
    /// property (a dotted name, or any name at all under an implicit `this`).
    pub(super) fn chain_maybe_class_agg(e: &Expression, has_this: bool) -> bool {
        match &e.kind {
            ExprKind::MemberAccess { .. } => true,
            ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => {
                Self::chain_maybe_class_agg(expr, has_this)
            }
            ExprKind::Ident(h) => h.path.len() >= 2 || has_this,
            _ => false,
        }
    }

    /// True when the base of a member/index chain is a CALL — a cheap check
    /// that evaluates nothing, used to gate the projection below.
    pub(super) fn chain_has_call_base(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::MemberAccess { expr, .. } | ExprKind::Index { expr, .. } => {
                matches!(expr.kind, ExprKind::Call { .. }) || Self::chain_has_call_base(expr)
            }
            _ => false,
        }
    }

    /// §13.4.1 / §7.2: project a member out of a call result whose return type
    /// is an UNPACKED struct — `f().i.a`, `f().arr[1]`. Such a result has no
    /// storage to name (the callee's frame is gone by now), so the member is
    /// sliced out of the packed form the return produced, using the very leaf
    /// layout that produced it. The packed-aggregate path cannot serve this:
    /// it declines member-wise structs, so these reads returned 0.
    pub(super) fn eval_unpacked_call_member(&mut self, e: &Expression) -> Option<Value> {
        let mut suffix: Vec<String> = Vec::new();
        let mut cur = e;
        let call = loop {
            match &cur.kind {
                ExprKind::MemberAccess { expr, member } => {
                    suffix.push(format!(".{}", member.name));
                    cur = expr;
                }
                ExprKind::Index { expr, index } => {
                    let i = self.eval_expr(index).to_i64()?;
                    suffix.push(format!("[{}]", i));
                    cur = expr;
                }
                ExprKind::Call { func, .. } => break func,
                _ => return None,
            }
        };
        if suffix.is_empty() {
            return None;
        }
        let ret = self.call_return_type(call)?;
        let su = self.unpacked_struct_of(&ret)?;
        suffix.reverse();
        let want = format!("X{}", suffix.concat());
        let (_, off, w, leaf_is_real) = self
            .struct_leaf_layout("X", &su)
            .into_iter()
            .find(|(k, ..)| *k == want)?;
        let v = self.eval_expr(cur);
        let mut out = v.range_select((off + w - 1) as usize, off as usize);
        // `struct_leaf_layout` already knows which leaves are `real`; that flag
        // was dropped here, so a real member projected out of a call result
        // came back as the raw f64 BIT PATTERN (1.5 read as
        // 4609434218613702656). This is the read path a §6.6.7 resolution
        // function's struct return goes through — `Tsum('{...}).field1`.
        if leaf_is_real {
            out.is_real = true;
        }
        Some(out)
    }

    /// The PACKED-struct element type of an array-typed expression —
    /// `o.arr[0]`, `cfg[x][y]`, `this.m[i]` — by stripping the index
    /// selects and resolving the container's declared element typedef.
    /// Handles a bare class-member name (via `this`) and a
    /// `<object>.<member>` receiver. `None` when the container is not a
    /// class array property or its elements are not a packed struct.
    pub(super) fn packed_struct_elem_type(
        &self,
        e: &Expression,
    ) -> Option<crate::ast::types::StructUnionType> {
        let mut base = e;
        while let ExprKind::Index { expr: b, .. } = &base.kind {
            base = b;
        }
        // The container: a bare class member (`arr[0]` inside a method),
        // an object member (`o.arr[0]` — the parser yields the dotted head as
        // an Ident with a 2-segment path), or a MemberAccess receiver.
        let (member, obj): (String, Option<usize>) = match &base.kind {
            ExprKind::Ident(h) if h.path.len() == 1 => (
                h.path[0].name.name.clone(),
                self.this_stack.last().copied().flatten(),
            ),
            ExprKind::Ident(h) if h.path.len() == 2 => {
                let hh = self
                    .eval_ident_handle(&h.path[0].name.name)
                    .filter(|&x| x != 0);
                (h.path[1].name.name.clone(), hh)
            }
            ExprKind::MemberAccess { expr: ob, member } => {
                let h = self.eval_handle_expr(ob).filter(|&h| h != 0)?;
                (member.name.clone(), Some(h))
            }
            _ => return None,
        };
        let handle = obj?;
        let inst = self.heap.get(handle)?.as_ref()?;
        let mut cur = Some(inst.class_name.clone());
        let mut guard = 0;
        while let Some(cn) = cur {
            guard += 1;
            if guard > 64 {
                return None;
            }
            let cd = self.module.classes.get(&cn)?;
            if cd.array_properties.contains_key(&member)
                || cd.array_nd_properties.contains_key(&member)
                || cd.queue_properties.contains_key(&member)
                || cd.assoc_properties.contains_key(&member)
            {
                let mut tn = cd.properties.get(&member)?.type_name.clone()?;
                // §8.25: an element typed by a class type parameter has the
                // type this object bound to it.
                if cd.type_param_names.contains(&tn) {
                    tn = self.class_prop_type_name(handle, &member)?;
                }
                let dt = self.module.typedef_types.get(tn.as_str())?;
                if let DataType::Struct(su) = Self::resolve_type_ref(dt, &self.module.typedef_types)
                {
                    if !Self::spreads_member_wise(&su) {
                        return Some(su);
                    }
                }
                return None;
            }
            cur = cd.extends.clone();
        }
        None
    }

    /// `sb.member[index]` with `member` a packed-array field (two or more
    /// packed dimensions) of the packed struct `sb` names: the element.
    #[inline(never)]
    pub(super) fn struct_field_elem_read(
        &mut self,
        sb: &Expression,
        member: &crate::ast::Identifier,
        index: &Expression,
    ) -> Option<Value> {
        let su = self.chain_base_packed_struct(sb)?;
        let m = su
            .members
            .iter()
            .find(|m| m.declarators.iter().any(|d| d.name.name == member.name))?;
        let dims = crate::compiler::elaborate::packed_full_dims_chained(
            &m.data_type,
            &self.module.parameters,
            &self.module.typedef_types,
        )?;
        if dims.len() < 2 {
            return None;
        }
        let (l, r) = dims[0];
        let n = (l - r).unsigned_abs() + 1;
        let field = Expression::new(
            ExprKind::MemberAccess {
                expr: Box::new(sb.clone()),
                member: member.clone(),
            },
            sb.span,
        );
        let fv = self.eval_expr(&field);
        let fw = u64::from(fv.width);
        if n == 0 || fw % n != 0 {
            return None;
        }
        let ew = (fw / n) as u32;
        let iv = self.eval_expr(index);
        if iv.has_xz() {
            return Some(Value::new(ew));
        }
        let k = iv.to_i64()?;
        let off = if l >= r {
            (r..=l).contains(&k).then(|| k - r)
        } else {
            (l..=r).contains(&k).then(|| r - k)
        };
        Some(match off {
            Some(o) => {
                let lo = o as u32 * ew;
                fv.range_select((lo + ew - 1) as usize, lo as usize)
            }
            None => Value::new(ew),
        })
    }

    pub(super) fn chain_base_packed_struct(
        &mut self,
        e: &Expression,
    ) -> Option<crate::ast::types::StructUnionType> {
        let resolved = match &e.kind {
            ExprKind::Call { func, .. } => {
                let ret = self.call_return_type(func)?;
                Self::resolve_type_ref(&ret, &self.module.typedef_types)
            }
            ExprKind::MemberAccess { expr, member } => {
                let Some(su) = self.chain_base_packed_struct(expr) else {
                    // §8.4/§7.2.1: a packed-struct property of a class
                    // object (`h.r` in `h.r.hdr.a`, `a[0].r.f`).
                    let h = self.eval_expr(expr).to_u64().unwrap_or(0) as usize;
                    if h == 0 || self.heap.get(h).and_then(|o| o.as_ref()).is_none() {
                        return None;
                    }
                    let su = self.class_prop_struct(h, &member.name)?;
                    return (su.packed && !Self::spreads_member_wise(&su)).then_some(su);
                };
                let m_dt = su
                    .members
                    .iter()
                    .find(|m| m.declarators.iter().any(|d| d.name.name == member.name))
                    .map(|m| m.data_type.clone())?;
                Self::resolve_type_ref(&m_dt, &self.module.typedef_types)
            }
            _ => return None,
        };
        let DataType::Struct(su) = resolved else {
            return None;
        };
        if Self::spreads_member_wise(&su) {
            return None;
        }
        Some(su)
    }

    /// §18.4 / §7.2.1: bit layout of a PACKED aggregate — `(member, offset,
    /// width)` plus the aggregate's total width. A packed STRUCT lays its LAST
    /// member at the LSB; a packed UNION overlays every member at offset 0 and
    /// is as wide as its widest member (§7.3.1).
    pub(super) fn packed_agg_layout(
        &self,
        su: &crate::ast::types::StructUnionType,
    ) -> (Vec<(String, u32, u32)>, u32) {
        self.packed_agg_layout_with(su, &self.module.parameters)
    }

    /// [`packed_agg_layout`] against an explicit parameter scope — pass
    /// [`instance_param_scope`] output so member widths that reference CLASS
    /// parameters (`bit [W-1:0] f;` in an inline-struct property) resolve.
    pub(super) fn packed_agg_layout_with(
        &self,
        su: &crate::ast::types::StructUnionType,
        params: &HashMap<String, Value>,
    ) -> (Vec<(String, u32, u32)>, u32) {
        use crate::ast::types::StructUnionKind;
        let is_union = matches!(su.kind, StructUnionKind::Union);
        let mut fields: Vec<(String, u32, u32)> = Vec::new();
        let mut offset: u32 = 0;
        let mut widest: u32 = 0;
        let members: Vec<&crate::ast::types::StructMember> = if is_union {
            su.members.iter().collect()
        } else {
            su.members.iter().rev().collect()
        };
        for m in members {
            let fw =
                resolve_type_width(&m.data_type, Some(params), Some(&self.module.typedefs)).max(1);
            // §7.2.1: reverse declarations within a member group too; the
            // first name in `bit a, b;` is more significant than the second.
            for d in m.declarators.iter().rev() {
                if is_union {
                    fields.push((d.name.name.clone(), 0, fw));
                    widest = widest.max(fw);
                } else {
                    fields.push((d.name.name.clone(), offset, fw));
                    offset += fw;
                }
            }
        }
        (
            fields,
            if is_union {
                widest.max(1)
            } else {
                offset.max(1)
            },
        )
    }

    /// §18.4: resolve `e` — `<agg_prop>.<member>` on a class object, in either
    /// Append a member segment to an expression: `obj.prop` -> `obj.prop.m`
    /// (pushes a path segment for an `Ident`, wraps in `MemberAccess`
    /// otherwise). Used to decompose a whole-struct assignment into per-field
    /// assignments that reuse the (working) field read/write machinery.
    pub(super) fn append_member_expr(expr: &Expression, member: &str) -> Expression {
        let span = expr.span;
        match &expr.kind {
            ExprKind::Ident(h) => {
                let mut h2 = h.clone();
                h2.path.push(crate::ast::expr::HierPathSegment {
                    name: crate::ast::Identifier {
                        name: member.to_string(),
                        span,
                    },
                    selects: Vec::new(),
                });
                h2.cached_signal_id = std::cell::Cell::new(None);
                h2.cached_resolved_name = std::cell::OnceCell::new();
                Expression::new(ExprKind::Ident(h2), span)
            }
            _ => Expression::new(
                ExprKind::MemberAccess {
                    expr: Box::new(expr.clone()),
                    member: crate::ast::Identifier {
                        name: member.to_string(),
                        span,
                    },
                },
                span,
            ),
        }
    }

    /// Append a literal index select `[i]` to `expr` — the per-element
    /// counterpart of `append_member_expr`, used to decompose an aggregate
    /// member that carries unpacked dimensions into per-element assignments.
    /// Always wraps in `ExprKind::Index` (the parse shape of a TRAILING
    /// `[i]`): an index parked in a path segment's `selects` is the parse
    /// shape of an INTERIOR select and is not resolved by the class-leaf /
    /// frame-leaf paths these assignments rely on.
    pub(super) fn append_index_expr(expr: &Expression, index: i64) -> Expression {
        let span = expr.span;
        let num = Expression::new(
            ExprKind::Number(crate::ast::expr::NumberLiteral::Integer {
                size: None,
                signed: false,
                base: crate::ast::expr::NumberBase::Decimal,
                value: index.to_string(),
                cached_val: std::cell::Cell::new(None),
            }),
            span,
        );
        Expression::new(
            ExprKind::Index {
                expr: Box::new(expr.clone()),
                index: Box::new(num),
            },
            span,
        )
    }

    /// Cartesian expansion of per-dimension index lists, last dimension
    /// varying fastest — the element enumeration order of
    /// `unpacked_struct_leaves` / `member_elem_indices`.
    pub(super) fn cartesian_indices(lists: &[Vec<i64>]) -> Vec<Vec<i64>> {
        let mut tuples: Vec<Vec<i64>> = vec![Vec::new()];
        for list in lists {
            let mut next = Vec::with_capacity(tuples.len() * list.len());
            for t in &tuples {
                for &i in list {
                    let mut t2 = t.clone();
                    t2.push(i);
                    next.push(t2);
                }
            }
            tuples = next;
        }
        tuples
    }

    /// `append_member_expr` followed by one literal `[i]` select per index:
    /// `expr.m[i][j]` for member `m` and indices `[i, j]`.
    pub(super) fn append_elem_path(expr: &Expression, member: &str, indices: &[i64]) -> Expression {
        let mut out = Self::append_member_expr(expr, member);
        for &i in indices {
            out = Self::append_index_expr(&out, i);
        }
        out
    }

    /// Whole-value read of an UNPACKED-STRUCT class property: assemble the
    /// member cells into one packed integral blob, laid out like
    /// `pack_unpacked_struct`/`struct_leaf_layout` (reverse declaration
    /// order, ascending offsets). The old body walked only the flat
    /// `struct_field_layout`, so a member with unpacked dimensions — whose
    /// cells are per-element (`pkt.mem[0]`, `pkt.mem[1]`, ...) — contributed
    /// nothing: a whole-property copy-out (`t = obj.pkt`) lost every array
    /// element while the by-value RETURN path (which packs through
    /// `pack_class_unpacked`) kept them. (IEEE 1800-2023 §7.2, §18.4)
    pub(super) fn read_whole_struct_class_prop(&self, handle: usize, prop: &str) -> Option<Value> {
        let su = self.class_prop_struct(handle, prop)?;
        if !Self::spreads_member_wise(&su) {
            return None;
        }
        let mut cells: Vec<(String, u32)> = Vec::new();
        self.unpacked_pack_cells(&su, "", &mut cells);
        let inst = self.heap.get(handle)?.as_ref()?;
        let mut total = 0u32;
        for (_, w) in &cells {
            total += *w;
        }
        if total == 0 {
            return None;
        }
        let mut any = false;
        let mut res = Value::zero(total);
        let mut off = 0u32;
        // Reverse declaration order: the LAST leaf sits at the LSB end.
        for (suffix, w) in cells.iter().rev() {
            let key = format!("{}{}", prop, suffix);
            if let Some(fv) = inst.properties.get(&key) {
                any = true;
                for b in 0..*w {
                    res.set_bit((off + b) as usize, fv.get_bit(b as usize));
                }
            }
            off += *w;
        }
        any.then_some(res)
    }

    /// §7.2: the LEAF CELLS of an unpacked struct for whole-value assembly,
    /// as `(key suffix, width)` in declaration order — a `&self` mirror of
    /// `struct_leaf_layout`'s traversal (which seeds frame markers and so
    /// needs `&mut`), matching the per-element cells the leaf enumeration
    /// produces for fixed member dimensions.
    pub(super) fn unpacked_pack_cells(
        &self,
        su: &crate::ast::types::StructUnionType,
        prefix: &str,
        out: &mut Vec<(String, u32)>,
    ) {
        for m in &su.members {
            let nested = self.unpacked_struct_of(&m.data_type);
            for md in &m.declarators {
                let mkey = format!("{}.{}", prefix, md.name.name);
                let mw = resolve_type_width(
                    &m.data_type,
                    Some(&self.module.parameters),
                    Some(&self.module.typedefs),
                )
                .max(1);
                match self.member_elem_indices(&md.dimensions) {
                    Some(tups) => {
                        // Element order, last dimension varying fastest —
                        // the enumeration order of `unpacked_struct_leaves`.
                        for tup in tups {
                            let ekey = format!("{}{}", mkey, Self::index_suffix(&tup));
                            match &nested {
                                Some(inner) => self.unpacked_pack_cells(inner, &ekey, out),
                                None => out.push((ekey, mw)),
                            }
                        }
                    }
                    None => match &nested {
                        Some(inner) => self.unpacked_pack_cells(inner, &mkey, out),
                        None => out.push((mkey, mw)),
                    },
                }
            }
        }
    }

    /// Resolve `expr` (an `obj.prop` or implicit-`this` `prop`) to its instance
    /// handle + property name, then assemble the whole unpacked-struct value.
    /// `None` for anything that is not a whole unpacked-struct class property.
    pub(super) fn try_read_whole_struct_class_prop(&mut self, expr: &Expression) -> Option<Value> {
        if self.no_class_objects()
            || !Self::receiver_prop_name(expr).is_some_and(|p| self.struct_prop_name_possible(p))
        {
            return None;
        }
        let (handle, prop_name) = self.class_prop_receiver(expr)?;
        self.read_whole_struct_class_prop(handle, &prop_name)
    }

    /// Decompose a whole-struct assignment whose TARGET is an unpacked-struct
    /// class property (`c.first = rhs`, `first = rhs.first` inside `copy`)
    /// into per-member field assignments `target.m = eval(rhs.m)`. This reuses
    /// the already-correct field read/write paths for BOTH signals and class
    /// properties, so the RHS is read member-by-member (`s0.a`, not the
    /// whole `s0`) and never needs a (broken) whole-struct Value. Returns
    /// `Some(())` if it handled the assignment. (IEEE 1800-2023 §10.6)
    pub(super) fn try_decompose_struct_class_prop_assign(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
    ) -> Option<()> {
        if !Self::receiver_prop_name(lvalue).is_some_and(|p| self.struct_prop_name_possible(p)) {
            return None;
        }
        let (handle, prop) = self.class_prop_receiver(lvalue)?;
        let su = self.class_prop_struct(handle, &prop)?;
        if !Self::spreads_member_wise(&su) {
            return None;
        }
        // §10.6/§13.4.1: a CALL source (`last = q.pop_front()`, `p = mk()`)
        // is evaluated once and its value spread over the leaves; the
        // member-wise read below would re-run it per member.
        let mut src = rvalue;
        while let ExprKind::Paren(i) = &src.kind {
            src = i;
        }
        if matches!(src.kind, ExprKind::Call { .. }) {
            let v = self.eval_expr(src);
            for (key, mv) in self.split_packed_struct("X", &su, &v) {
                if let Some(lv) = Self::leaf_lvalue_of(lvalue, &key[1..]) {
                    self.assign_value(&lv, &mv);
                }
            }
            return Some(());
        }
        // Member-wise assign. Nested struct/array members recurse naturally:
        // `lhs.m = eval(rhs.m)` re-enters this path (for a nested struct
        // class-property target) or the existing signal paths. A member with
        // FIXED unpacked dimensions has no single cell — its elements are
        // the storage — so it is decomposed per element (`lhs.m[i] =
        // eval(rhs.m[i])`); a dynamic/queue/associative member keeps the
        // whole-member copy. (IEEE 1800-2023 §7.2)
        for m in &su.members {
            for md in &m.declarators {
                let mname = md.name.name.as_str();
                if let Some(tups) = self.member_elem_indices(&md.dimensions) {
                    for tup in tups {
                        let lhs_f = Self::append_elem_path(lvalue, mname, &tup);
                        let rhs_f = Self::append_elem_path(rvalue, mname, &tup);
                        let v = self.eval_expr(&rhs_f);
                        self.assign_value(&lhs_f, &v);
                    }
                    continue;
                }
                let lhs_f = Self::append_member_expr(lvalue, mname);
                let rhs_f = Self::append_member_expr(rvalue, mname);
                let v = self.eval_expr(&rhs_f);
                self.assign_value(&lhs_f, &v);
            }
        }
        Some(())
    }

    /// Mirror of `try_decompose_struct_class_prop_assign` for the READ side:
    /// the assignment SOURCE is an unpacked-struct CLASS property (any
    /// target). Sees through parens and a ?: whose arms are both class-prop
    /// structs (`cond ? o.orig : o.ovrd` picks the arm by the evaluated
    /// condition — the UVM factory pair-copy shape). Member-wise
    /// `lhs.m = eval(rhs.m)` reuses the working field paths on both sides.
    /// (IEEE 1800-2023 §10.6)
    pub(super) fn try_decompose_struct_class_prop_read_assign(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
    ) -> Option<()> {
        match &rvalue.kind {
            ExprKind::Paren(inner) => {
                return self.try_decompose_struct_class_prop_read_assign(lvalue, inner);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                // Only take over when BOTH arms are class-prop structs, so a
                // mixed ternary keeps its existing evaluation.
                let t_ok = self.class_prop_struct_of(then_expr).is_some();
                let e_ok = self.class_prop_struct_of(else_expr).is_some();
                if t_ok && e_ok {
                    let c = self.eval_expr(condition);
                    let arm = if c.to_u64().unwrap_or(0) != 0 {
                        then_expr
                    } else {
                        else_expr
                    };
                    return self.try_decompose_struct_class_prop_read_assign(lvalue, arm);
                }
                return None;
            }
            _ => {}
        }
        if !Self::receiver_prop_name(rvalue).is_some_and(|p| self.struct_prop_name_possible(p)) {
            return None;
        }
        let (handle, prop) = self.class_prop_receiver(rvalue)?;
        let su = self.class_prop_struct(handle, &prop)?;
        if !Self::spreads_member_wise(&su) {
            return None;
        }
        // Same per-element decomposition of members with FIXED unpacked
        // dimensions as the write twin: `t2.mem = eval(o.pkt.mem)` copied a
        // single cell the element reads (`t2.mem[p]`) never consult, so
        // every element of a copied-out array member read back 0.
        for m in &su.members {
            for md in &m.declarators {
                let mname = md.name.name.as_str();
                // §7.2.1: a dynamic member copies as a whole collection from
                // the property's `<handle>#<prop>.<m>` store.
                let coll_dims = if !md.dimensions.is_empty() {
                    Some(md.dimensions.clone())
                } else if let DataType::TypeReference {
                    name, dimensions, ..
                } = &m.data_type
                    && dimensions.is_empty()
                    && !name.has_scope()
                {
                    self.class_chain_typedef_dims(handle, &name.name.name)
                } else {
                    None
                };
                if let Some(kind) = coll_dims.as_deref().and_then(Self::coll_kind_of) {
                    let lhs_f = Self::append_member_expr(lvalue, mname);
                    let rhs_f = Self::append_member_expr(rvalue, mname);
                    if let (Some(dst), Some(src)) = (
                        self.expr_assoc_name(&lhs_f),
                        self.class_struct_coll_name(&rhs_f),
                    ) {
                        self.clear_coll(&dst, kind);
                        self.copy_coll(&dst, &src, kind);
                        continue;
                    }
                }
                if let Some(tups) = self.member_elem_indices(&md.dimensions) {
                    for tup in tups {
                        let lhs_f = Self::append_elem_path(lvalue, mname, &tup);
                        let rhs_f = Self::append_elem_path(rvalue, mname, &tup);
                        let v = self.eval_expr(&rhs_f);
                        self.assign_value(&lhs_f, &v);
                    }
                    continue;
                }
                let lhs_f = Self::append_member_expr(lvalue, mname);
                let rhs_f = Self::append_member_expr(rvalue, mname);
                let v = self.eval_expr(&rhs_f);
                self.assign_value(&lhs_f, &v);
            }
        }
        Some(())
    }

    /// parse shape — to the storage it aliases. `None` when `e` is not an
    /// aggregate class-property member.
    pub(super) fn class_agg_member(&mut self, e: &Expression) -> Option<ClassAggRef> {
        if self.no_class_objects() || !self.chain_may_name_struct_prop(e) {
            return None;
        }
        // §7.2: a member of an UNPACKED-struct property at ANY depth, and
        // through an index — `o.n.inner.a`, `o.s.arr[1]`, `o.arr[0].a`. The
        // one-level resolution below keys such a reference on the wrong pair,
        // which left every deeper leaf sharing one cell BETWEEN OBJECTS: two
        // instances read each other's `n.inner.a`.
        if let Some(r) = self.class_unpacked_leaf(e) {
            return Some(r);
        }
        // §7.4.1: a member with an index or part-select below it. The whole
        // chain resolves through one walk, so both parse shapes — the
        // flattened ident with segment selects and the method-scope
        // MemberAccess/Index chain — get the element's bits.
        if let Some(r) = self.class_member_select_ref(e) {
            return Some(r);
        }
        let (base, field) = Self::split_trailing_member(e)?;
        self.class_agg_member_parts(&base, &field)
    }

    /// Resolve `<receiver>.<prop><suffix>` where `prop` is an unpacked struct
    /// (or an array of them) and `<suffix>` is any chain of `.member` / `[i]`.
    /// Each leaf gets its own per-instance cell keyed `<prop><suffix>`.
    pub(super) fn class_unpacked_leaf(&mut self, e: &Expression) -> Option<ClassAggRef> {
        // §11.4.2 / §11.3.6: the probe below EVALUATES every index of the
        // chain, and the real read or write evaluates them again. With a side
        // effect in an index (`items[--sp]`, `q[i++].f`) that ran it twice
        // whenever a property name of the chain COULD be a struct — e.g. any
        // `T items[N]` whose type parameter might bind a struct — so a
        // `return items[--sp];` popped two elements. Only probe such a chain
        // when one of its receivers really is an unpacked-struct property.
        if Self::chain_has_impure_index(e) && !self.chain_names_unpacked_struct_prop(e) {
            return None;
        }
        // A dotted reference is an IDENT with a multi-segment path outside a
        // method body and a MemberAccess/Index chain inside one; both shapes
        // reach here, and both used to resolve only one level below the
        // property.
        if let ExprKind::Ident(h) = &e.kind {
            return self.class_unpacked_leaf_ident(h, e.span, "");
        }
        if !matches!(
            e.kind,
            ExprKind::MemberAccess { .. } | ExprKind::Index { .. }
        ) {
            return None;
        }
        // Descend to the receiver, recording each candidate base with the
        // suffix that hangs off it.
        let mut chain: Vec<(&Expression, String)> = Vec::new();
        let mut parts: Vec<String> = Vec::new();
        let mut cur = e;
        loop {
            match &cur.kind {
                ExprKind::MemberAccess { expr, member } => {
                    parts.push(format!(".{}", member.name));
                    cur = expr;
                }
                ExprKind::Index { expr, index } => {
                    let i = self.eval_expr(index).to_i64()?;
                    parts.push(format!("[{}]", i));
                    cur = expr;
                }
                _ => break,
            }
            let mut p = parts.clone();
            p.reverse();
            chain.push((cur, p.concat()));
        }
        // Innermost receiver first: `obj` alone only names a property under an
        // implicit `this`, and that candidate is rejected when its type is not
        // an unpacked struct.
        for (base, suffix) in chain.into_iter().rev() {
            if let Some(r) = self.class_unpacked_leaf_at(base, &suffix) {
                return Some(r);
            }
            // The chain may bottom out at a DOTTED ident (`c.am.arr` beneath a
            // trailing `[1]`), whose own segments still need splitting into
            // receiver / property / leaf path.
            if let ExprKind::Ident(h) = &base.kind {
                if let Some(r) = self.class_unpacked_leaf_ident(h, base.span, &suffix) {
                    return Some(r);
                }
            }
        }
        None
    }

    /// Whether an index or select of the member/index chain `e` (not of a
    /// receiver's own sub-expressions) has a side effect.
    pub(super) fn chain_has_impure_index(e: &Expression) -> bool {
        let mut cur = e;
        loop {
            match &cur.kind {
                ExprKind::MemberAccess { expr, .. } => cur = expr,
                ExprKind::Index { expr, index } => {
                    if !Self::cond_expr_is_effect_free(index, false) {
                        return true;
                    }
                    cur = expr;
                }
                ExprKind::Ident(h) => {
                    return h.path.iter().any(|s| {
                        s.selects
                            .iter()
                            .any(|x| !Self::cond_expr_is_effect_free(x, false))
                    });
                }
                _ => return false,
            }
        }
    }

    /// Whether some receiver of the chain `e` is an unpacked-struct class
    /// property that spreads member-wise, found WITHOUT evaluating any index
    /// of the chain (see `class_unpacked_leaf`).
    #[inline(never)]
    pub(super) fn chain_names_unpacked_struct_prop(&mut self, e: &Expression) -> bool {
        let mut bases: Vec<Expression> = Vec::new();
        let mut cur = e;
        loop {
            match &cur.kind {
                ExprKind::MemberAccess { expr, .. } | ExprKind::Index { expr, .. } => {
                    cur = expr;
                    if !matches!(cur.kind, ExprKind::Ident(_)) {
                        bases.push(cur.clone());
                    }
                }
                _ => break,
            }
        }
        if let ExprKind::Ident(h) = &cur.kind {
            for k in 0..h.path.len() {
                let mut base_h = h.clone();
                base_h.path.truncate(k + 1);
                if let Some(seg) = base_h.path.last_mut() {
                    seg.selects.clear();
                }
                base_h.cached_signal_id = std::cell::Cell::new(None);
                base_h.cached_resolved_name = std::cell::OnceCell::new();
                bases.push(Expression::new(ExprKind::Ident(base_h), cur.span));
            }
        }
        for base in &bases {
            if !Self::receiver_prop_name(base).is_some_and(|p| self.struct_prop_name_possible(p)) {
                continue;
            }
            if Self::chain_has_impure_index(base) {
                // A receiver with its own side effect: keep the old probe.
                return true;
            }
            let Some((handle, prop)) = self.class_prop_receiver(base) else {
                continue;
            };
            if self
                .class_prop_struct(handle, &prop)
                .is_some_and(|su| Self::spreads_member_wise(&su))
            {
                return true;
            }
        }
        false
    }

    /// Candidate splits of a dotted identifier: each segment in turn is taken
    /// as the property, with everything after it (plus `extra`) as the leaf
    /// path. Segment 0 covers an implicit `this`.
    pub(super) fn class_unpacked_leaf_ident(
        &mut self,
        h: &crate::ast::expr::HierarchicalIdentifier,
        span: crate::ast::Span,
        extra: &str,
    ) -> Option<ClassAggRef> {
        for k in 0..h.path.len() {
            if k + 1 == h.path.len() && extra.is_empty() && h.path[k].selects.is_empty() {
                continue; // nothing left to address below the property
            }
            let mut suffix = String::new();
            let mut ok = true;
            for sel in &h.path[k].selects {
                match self.eval_expr(sel).to_i64() {
                    Some(i) => suffix.push_str(&format!("[{}]", i)),
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            for seg in &h.path[k + 1..] {
                if !ok {
                    break;
                }
                suffix.push_str(&format!(".{}", seg.name.name));
                for sel in &seg.selects {
                    match self.eval_expr(sel).to_i64() {
                        Some(i) => suffix.push_str(&format!("[{}]", i)),
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
            }
            if !ok {
                continue;
            }
            suffix.push_str(extra);
            if suffix.is_empty() {
                continue;
            }
            let mut base_h = h.clone();
            base_h.path.truncate(k + 1);
            if let Some(seg) = base_h.path.last_mut() {
                seg.selects.clear();
            }
            base_h.cached_signal_id = std::cell::Cell::new(None);
            base_h.cached_resolved_name = std::cell::OnceCell::new();
            let base = Expression::new(ExprKind::Ident(base_h), span);
            if let Some(r) = self.class_unpacked_leaf_at(&base, &suffix) {
                return Some(r);
            }
        }
        None
    }

    /// One candidate split: `base` names the property, `suffix` addresses a
    /// leaf beneath it. `None` when the property is not an unpacked struct or
    /// the suffix names no leaf of it — which is what lets the caller try
    /// successive split points without guessing.
    pub(super) fn class_unpacked_leaf_at(
        &mut self,
        base: &Expression,
        suffix: &str,
    ) -> Option<ClassAggRef> {
        if !Self::receiver_prop_name(base).is_some_and(|p| self.struct_prop_name_possible(p)) {
            return None;
        }
        let (handle, prop) = self.class_prop_receiver(base)?;
        let su = self.class_prop_struct(handle, &prop)?;
        if !Self::spreads_member_wise(&su) {
            return None;
        }
        // An ARRAY-of-structs property indexes the element first; the rest of
        // the suffix addresses a leaf of the element's type.
        let probe = if suffix.starts_with('[') {
            format!("X{}", &suffix[suffix.find(']')? + 1..])
        } else {
            format!("X{}", suffix)
        };
        let (_, w, _) = self
            .unpacked_struct_leaf_keys("X", &su)
            .into_iter()
            .find(|(k, ..)| *k == probe)?;
        Some(ClassAggRef::Unpacked {
            handle,
            key: format!("{}{}", prop, suffix),
            w: w.max(1),
        })
    }

    /// §18.4/§7.4.1: resolve an INDEXED or PART-SELECTED member of a class
    /// property's struct — `<obj>.<prop>.<mem>[i]`, `...[i][h:l]`, any member
    /// nesting depth — to the storage it aliases. `class_packed_elem_ref`
    /// resolves only a DIRECTLY-DECLARED packed-array property and
    /// `class_agg_member_parts` only a whole member with no selects, so an
    /// index below the member fell through to a plain BIT-select: the read
    /// came back as one bit and the indexed WRITE was dropped entirely (a
    /// driver class filling a packet struct element-by-element then drove
    /// all-zero data beats). Both parse shapes route here: the method-scope
    /// `MemberAccess`/`Index` chain and the module-scope flattened ident
    /// with segment selects. `None` when the chain names no struct property,
    /// the member is a COLLECTION (the collection machinery owns it), or an
    /// index is out of range or unknown.
    pub(super) fn class_member_select_ref(&mut self, e: &Expression) -> Option<ClassAggRef> {
        if self.no_class_objects() {
            return None;
        }
        // Phase 1 — STRUCTURE ONLY: peel selects and member accesses off the
        // outside WITHOUT evaluating anything; the rooted identifier below
        // contributes its segments and their selects. Nothing is evaluated
        // until a receiver/property boundary is known to resolve, so a chain
        // rooted at a plain signal or a non-struct property never pays for
        // its index expressions.
        let mut peeled: Vec<PendingSel> = Vec::new();
        let mut cur = e;
        loop {
            match &cur.kind {
                ExprKind::Index { expr, index } => {
                    peeled.push(PendingSel::Index(index));
                    cur = expr;
                }
                ExprKind::RangeSelect {
                    expr,
                    kind,
                    left,
                    right,
                } => {
                    peeled.push(PendingSel::Range {
                        kind: *kind,
                        left,
                        right,
                    });
                    cur = expr;
                }
                ExprKind::MemberAccess { expr, member } => {
                    peeled.push(PendingSel::Member(member.name.as_str()));
                    cur = expr;
                }
                _ => break,
            }
        }
        let ExprKind::Ident(h) = &cur.kind else {
            return None;
        };
        if peeled.is_empty() && h.path.iter().all(|s| s.selects.is_empty()) && h.path.len() < 4 {
            // A whole member with no selects is `class_agg_member_parts`' case
            // (via `split_trailing_member`) — do not pay the walk for it.
            // That handles ONE member below the property (`obj.prop.m`); a
            // module-scope NESTED member (`obj.prop.m.n`, four segments)
            // needs the walk, or its write went to name-keyed storage that
            // every object shared and a method never saw.
            return None;
        }
        // Root-first pending steps: every segment, then that segment's
        // selects.
        let mut pending: Vec<PendingSel> = Vec::with_capacity(h.path.len() + peeled.len());
        for seg in &h.path {
            pending.push(PendingSel::Member(seg.name.name.as_str()));
            for sel in &seg.selects {
                match &sel.kind {
                    ExprKind::RangeSelect {
                        kind, left, right, ..
                    } => {
                        pending.push(PendingSel::Range {
                            kind: *kind,
                            left,
                            right,
                        });
                    }
                    _ => {
                        pending.push(PendingSel::Index(sel));
                    }
                }
            }
        }
        pending.extend(peeled.into_iter().rev());
        // Phase 2 — RECEIVER FIRST: resolve some boundary of the leading
        // member run to a struct-valued property before any select is
        // evaluated. `class_member_select_split` re-runs this resolution
        // below — ident-handle lookups, cheap next to evaluating the index
        // expressions Phase 3 would otherwise have paid for up front.
        let mut lead = 0usize;
        while lead < pending.len() && matches!(pending[lead], PendingSel::Member(_)) {
            lead += 1;
        }
        let mut rooted = false;
        for k in 0..lead {
            if k + 1 >= pending.len() {
                break; // nothing may hang below the base
            }
            let mut bh = h.clone();
            bh.path.truncate(k + 1);
            if let Some(seg) = bh.path.last_mut() {
                seg.selects.clear();
            }
            bh.cached_signal_id = std::cell::Cell::new(None);
            bh.cached_resolved_name = std::cell::OnceCell::new();
            let base = Expression::new(ExprKind::Ident(bh), cur.span);
            let Some((handle, prop)) = self.class_prop_receiver(&base) else {
                continue;
            };
            if self.class_prop_struct(handle, &prop).is_some() {
                rooted = true;
                break;
            }
        }
        if !rooted {
            return None;
        }
        // Phase 3 — evaluate the selects (§11.5.1: an unknown index selects
        // nothing) and walk.
        let mut steps: Vec<MemberSel> = Vec::with_capacity(pending.len());
        for p in pending {
            match p {
                PendingSel::Member(m) => steps.push(MemberSel::Member(m.to_string())),
                PendingSel::Index(ix) => {
                    let iv = self.eval_expr(ix);
                    if iv.is_real || iv.has_xz() {
                        return None;
                    }
                    steps.push(MemberSel::Index(iv.to_i64()?));
                }
                PendingSel::Range { kind, left, right } => {
                    let lv = self.eval_expr(left);
                    let rv = self.eval_expr(right);
                    if lv.is_real || rv.is_real || lv.has_xz() || rv.has_xz() {
                        return None;
                    }
                    steps.push(MemberSel::Range {
                        kind,
                        l: lv.to_i64()?,
                        r: rv.to_i64()?,
                    });
                }
            }
        }
        self.class_member_select_split(h, cur.span, &steps)
    }

    /// Split the root-first `steps` built by [`class_member_select_ref`] into
    /// a receiver base — a flat ident naming `obj.prop`, or a bare `prop`
    /// under an implicit `this` — and the member path hanging below it, then
    /// resolve that path into aggregate storage. Every boundary inside the
    /// leading run of member names is a candidate: `bd.pkt.mem[i]` roots at
    /// `bd.pkt`, `pkt.mem[i]` at `this.pkt`. Boundaries are tried innermost
    /// first (matching [`class_unpacked_leaf`]'s chain order), so a bare
    /// first segment that is a local variable or a non-struct property is
    /// rejected cheaply before any receiver evaluation happens.
    pub(super) fn class_member_select_split(
        &mut self,
        h: &crate::ast::expr::HierarchicalIdentifier,
        span: crate::ast::Span,
        steps: &[MemberSel],
    ) -> Option<ClassAggRef> {
        // A boundary past the leading `Member` run would leave a select on
        // the base ident, which `class_prop_receiver` never accepts.
        let mut lead = 0usize;
        while lead < steps.len() && matches!(steps[lead], MemberSel::Member(_)) {
            lead += 1;
        }
        for k in 0..lead {
            if k + 1 >= steps.len() {
                break; // nothing may hang below the base
            }
            let mut bh = h.clone();
            bh.path.truncate(k + 1);
            if let Some(seg) = bh.path.last_mut() {
                seg.selects.clear();
            }
            bh.cached_signal_id = std::cell::Cell::new(None);
            bh.cached_resolved_name = std::cell::OnceCell::new();
            let base = Expression::new(ExprKind::Ident(bh), span);
            let Some((handle, prop)) = self.class_prop_receiver(&base) else {
                continue;
            };
            let Some(su) = self.class_prop_struct(handle, &prop) else {
                continue;
            };
            let rest = &steps[k + 1..];
            // §7.2: an unpacked struct — one cell per leaf, walked below.
            if Self::spreads_member_wise(&su) {
                if let Some(r) = self.unpacked_member_select(handle, &prop, &su, rest) {
                    return Some(r);
                }
                continue;
            }
            // §18.4: a packed struct/union — members are bit slices of the
            // property's one integral value.
            if let Some(r) = self.packed_member_select(handle, &prop, &su, rest) {
                return Some(r);
            }
        }
        None
    }

    /// Walk the member path below an UNPACKED-struct class property, where
    /// every leaf is its own per-instance cell keyed `<prop>.<m>...` — the
    /// same keys [`unpacked_struct_leaves`] enumerates. Index steps over a
    /// member's fixed unpacked dimensions step from cell to cell; once those
    /// are consumed (and on a packed-array or vector member, which is ONE
    /// cell, §7.4.1) they address packed-dimension slots inside the cell,
    /// and a part-select ends the walk with a bit slice of it. `None` when
    /// the path leaves that model: a member whose declarator carries a
    /// dynamic/queue/associative dimension (the collection machinery owns
    /// it), a select at struct level (an array-of-structs property — the
    /// suffix resolver owns it), a sub-array reference, or an index out of
    /// range.
    pub(super) fn unpacked_member_select(
        &mut self,
        handle: usize,
        prop: &str,
        su: &crate::ast::types::StructUnionType,
        rest: &[MemberSel],
    ) -> Option<ClassAggRef> {
        let inst_params = self.instance_param_scope(handle);
        let mut key = prop.to_string();
        let mut level = su.clone();
        let mut i = 0usize;
        loop {
            let MemberSel::Member(mname) = rest.get(i)? else {
                return None;
            };
            let mname = mname.as_str();
            let (dt, dims) = Self::struct_member_decl(&level, mname)?;
            if Self::coll_kind_of(&dims).is_some() {
                return None;
            }
            key.push('.');
            key.push_str(mname);
            let nested = self.unpacked_struct_of(&dt);
            // Fixed unpacked dimensions of the member: one Index step per
            // dimension, each stepping to that element's own cell. When the
            // bounds do not const-resolve, the leaf enumeration stores the
            // member as ONE cell — leave the indices to the packed-slot
            // math below instead.
            let mut consumed = 0usize;
            if !dims.is_empty() {
                if let Some(lists) = self.member_dim_indices(&dims) {
                    while consumed < dims.len() {
                        let Some(MemberSel::Index(idx)) = rest.get(i + 1 + consumed) else {
                            break;
                        };
                        if !lists.get(consumed)?.contains(idx) {
                            return None; // out of range
                        }
                        key.push_str(&format!("[{}]", idx));
                        consumed += 1;
                    }
                    if consumed < dims.len() {
                        return None; // a sub-array of the member, not one cell
                    }
                }
            }
            i += 1 + consumed;
            if let Some(inner) = nested {
                level = inner;
                continue;
            }
            // Leaf: the cell width is the member's PACKED width — its
            // declarator dimensions became `[i]` key parts above.
            let mw = resolve_type_width(
                &dt,
                Some(&self.module.parameters),
                Some(&self.module.typedefs),
            )
            .max(1);
            let tail = &rest[i..];
            if tail.is_empty() {
                return Some(ClassAggRef::Unpacked { handle, key, w: mw });
            }
            let (off, w) = self.packed_array_slice(&dt, &inst_params, mw, tail)?;
            return Some(ClassAggRef::UnpackedSlice {
                handle,
                key,
                off,
                w,
                cell_w: mw,
            });
        }
    }

    /// Walk the member path below a PACKED-struct/union class property,
    /// whose members are bit slices of the property's one integral value.
    /// Each member step descends at its field offset (every member of a
    /// union aliases offset 0, §7.3.1); index and part-select steps below
    /// the member address packed-dimension slots of its slice. `None` when
    /// the path leaves the layout: an unknown member, an unpacked dimension
    /// on a member (not a packed aggregate), a select at struct level, or a
    /// slot out of range.
    pub(super) fn packed_member_select(
        &mut self,
        handle: usize,
        prop: &str,
        su: &crate::ast::types::StructUnionType,
        rest: &[MemberSel],
    ) -> Option<ClassAggRef> {
        let inst_params = self.instance_param_scope(handle);
        let mut level = su.clone();
        let (mut fields, total) = self.packed_agg_layout_with(&level, &inst_params);
        let total = total.max(1);
        let mut off = 0u32;
        let mut w = total;
        let mut i = 0usize;
        while i < rest.len() {
            let MemberSel::Member(mname) = &rest[i] else {
                return None;
            };
            let (dt, dims) = Self::struct_member_decl(&level, mname)?;
            if !dims.is_empty() {
                return None; // an unpacked dimension: not a packed aggregate
            }
            let (_, moff, mw) = fields.iter().find(|(n, ..)| n == mname).cloned()?;
            off += moff;
            w = mw.max(1);
            match self.resolve_dt(&dt) {
                crate::ast::types::DataType::Struct(inner)
                    if !Self::spreads_member_wise(&inner) =>
                {
                    level = inner;
                    fields = self.packed_agg_layout_with(&level, &inst_params).0;
                    i += 1;
                }
                _ => {
                    // Leaf: the remaining selects address packed slots / a
                    // bit range of the member's slice.
                    let (o, sw) = self.packed_array_slice(&dt, &inst_params, w, &rest[i + 1..])?;
                    let two_state = self.mixed_member_two_state(su, &dt);
                    return Some(ClassAggRef::Packed {
                        handle,
                        prop: prop.to_string(),
                        off: off + o,
                        w: sw,
                        total,
                        two_state,
                    });
                }
            }
        }
        Some(ClassAggRef::Packed {
            handle,
            prop: prop.to_string(),
            off,
            w,
            total,
            two_state: false,
        })
    }

    /// Offset and width — within a `total_w`-bit value whose packed
    /// dimensions come from `dt` — of the element or range the remaining
    /// select steps address: `mem[1]`, `mem[1][7:4]`, `v[3:0]`, `v[2]`.
    /// `rest` may hold `Index` steps followed by at most one `Range`; a
    /// `Member` below a leaf is not a select this resolves. Slot order
    /// matches `class_packed_elem_ref`: a descending `[1:0]` puts index 1
    /// at the MS end, an ascending `[0:1]` puts index 0 there (§7.4.1).
    pub(super) fn packed_array_slice(
        &self,
        dt: &DataType,
        params: &HashMap<String, Value>,
        total_w: u32,
        rest: &[MemberSel],
    ) -> Option<(u32, u32)> {
        let dims = Self::packed_dim_bounds(dt, params)?;
        let counts: Vec<u32> = dims
            .iter()
            .map(|&(l, r)| u32::try_from((l - r).unsigned_abs() + 1).ok().unwrap_or(0))
            .collect();
        let mut off = 0u32;
        let mut span = total_w; // width the next step selects within
        let mut d = 0usize;
        let mut i = 0usize;
        while i < rest.len() {
            match &rest[i] {
                MemberSel::Index(idx) => {
                    let &(l, r) = dims.get(d)?;
                    if *idx < l.min(r) || *idx > l.max(r) {
                        return None;
                    }
                    let slot = if l >= r { idx - r } else { r - idx };
                    span = span.checked_div(counts.get(d).copied().unwrap_or(1).max(1))?;
                    off = off.checked_add((slot as u32).checked_mul(span)?)?;
                    d += 1;
                    i += 1;
                }
                MemberSel::Range { kind, l, r } => {
                    let (o, w) = Self::range_within(span, *kind, *l, *r)?;
                    return Some((off.checked_add(o)?, w));
                }
                MemberSel::Member(_) => return None,
            }
        }
        Some((off, span))
    }

    /// Const-resolved `(left, right)` of every PACKED dimension of `dt`,
    /// outermost first — `bit [1:0][7:0]` → `[(1, 0), (7, 0)]`. `None` for
    /// a dimension that does not const-evaluate under `params`, an unsized
    /// one, or a type with no packed dimensions (an integer atom
    /// contributes its own width as one range so bit-selects on it still
    /// resolve; a `real` never does).
    pub(super) fn packed_dim_bounds(
        dt: &DataType,
        params: &HashMap<String, Value>,
    ) -> Option<Vec<(i64, i64)>> {
        use crate::ast::types::{DataType as DT, PackedDimension};
        let dims: Vec<PackedDimension> = match dt {
            DT::IntegerVector { dimensions, .. } | DT::Implicit { dimensions, .. } => {
                dimensions.clone()
            }
            DT::IntegerAtom { kind, .. } => {
                let w = match kind {
                    crate::ast::types::IntegerAtomType::Byte => 8,
                    crate::ast::types::IntegerAtomType::ShortInt => 16,
                    crate::ast::types::IntegerAtomType::Int
                    | crate::ast::types::IntegerAtomType::Integer => 32,
                    crate::ast::types::IntegerAtomType::LongInt
                    | crate::ast::types::IntegerAtomType::Time => 64,
                };
                return Some(vec![(w as i64 - 1, 0)]);
            }
            DT::Struct(su) if !su.dimensions.is_empty() => su.dimensions.clone(),
            _ => return None,
        };
        if dims.is_empty() {
            return None;
        }
        let mut out = Vec::with_capacity(dims.len());
        for d in &dims {
            let PackedDimension::Range { left, right, .. } = d else {
                return None;
            };
            let l = super::elaborate::const_eval_i64_with_params(left, Some(params))?;
            let r = super::elaborate::const_eval_i64_with_params(right, Some(params))?;
            out.push((l, r));
        }
        Some(out)
    }

    /// `(offset, width)` of a part-select — `[l:r]`, `[l +: r]`, `[l -: r]` —
    /// within a `width`-bit value; `None` when it is empty or leaves the
    /// value. Bound semantics match the elaborator's part-select fold.
    pub(super) fn range_within(width: u32, kind: RangeKind, l: i64, r: i64) -> Option<(u32, u32)> {
        let (hi, lo) = match kind {
            RangeKind::Constant => (l.max(r), l.min(r)),
            RangeKind::IndexedUp => (l + r - 1, l),
            RangeKind::IndexedDown => (l, l - r + 1),
        };
        if hi < lo || lo < 0 {
            return None;
        }
        let w = (hi - lo + 1) as u32;
        let lo = lo as u32;
        let end = lo.checked_add(w)?;
        (end <= width).then_some((lo, w))
    }

    /// The declared type and declarator dimensions of member `name` in `su`:
    /// one `StructMember` covers every declarator of `bit a, b;`, so members
    /// are matched by declarator name.
    pub(super) fn struct_member_decl(
        su: &crate::ast::types::StructUnionType,
        name: &str,
    ) -> Option<(DataType, Vec<crate::ast::types::UnpackedDimension>)> {
        for m in &su.members {
            for d in &m.declarators {
                if d.name.name == name {
                    return Some((m.data_type.clone(), d.dimensions.clone()));
                }
            }
        }
        None
    }

    /// The §7.8 collection kind of a member's FIRST declarator dimension:
    /// unsized `[]` (dynamic), `[$]`/`[$:N]` (queue) or `[key]`
    /// (associative). Fixed ranges and expression sizes are NOT collections
    /// — their element cells belong to the struct machinery.
    pub(super) fn coll_kind_of(
        dims: &[crate::ast::types::UnpackedDimension],
    ) -> Option<StructCollKind> {
        use crate::ast::types::UnpackedDimension as UD;
        match dims.first()? {
            UD::Unsized(_) => Some(StructCollKind::Dynamic),
            UD::Queue { .. } => Some(StructCollKind::Queue),
            UD::Associative { .. } => Some(StructCollKind::Assoc),
            _ => None,
        }
    }

    /// Instance-scoped storage name of a COLLECTION member of a struct
    /// class property — `<handle>#<prop>.<member>` — registering the
    /// collection on first sight so the §7.8 machinery (element cells at
    /// `<name>[k]`, the `.size` shadow, queue append-on-write, `new[n]`)
    /// treats it exactly like a directly-declared collection property.
    /// All receiver shapes resolve: `bd.pkt.qmem`, `pkt.qmem` (implicit
    /// `this`) and `this.pkt.qmem`; the trailing `[k]` of an element access
    /// is the caller's to append. `None` when the expression names no
    /// struct member whose declarator carries a dynamic/queue/associative
    /// dimension (a collection mid-path belongs to the nested-collection
    /// machinery, and a packed struct cannot hold one, §7.2.4).
    pub(super) fn class_struct_coll_name(&mut self, e: &Expression) -> Option<String> {
        // Perf gate, BEFORE any allocation: the chain's trailing member name
        // must be one of the (few) names some reachable struct declares as a
        // dynamic/queue/associative member. The boundary walk below evaluates
        // a handle per candidate receiver, and this probe runs on every
        // collection-shaped name resolution.
        let trailing = match &e.kind {
            ExprKind::MemberAccess { member, .. } => member.name.as_str(),
            ExprKind::Ident(h) => h.path.last()?.name.name.as_str(),
            _ => return None,
        };
        if !self.struct_coll_member_possible(trailing) {
            return None;
        }
        if !self.no_class_objects()
            && let Some(n) = self.class_struct_coll_walk(e)
        {
            return Some(n);
        }
        // The same member of a struct held anywhere else: a variable, a
        // nested struct, or an element of an array (`aa[k].q`, `q[i].d`).
        self.struct_member_coll_name(e)
    }

    /// The receiver walk of [`class_struct_coll_name`], past its gate.
    #[inline(never)]
    pub(super) fn class_struct_coll_walk(&mut self, e: &Expression) -> Option<String> {
        // Member names, root-first: `bd.pkt.qmem` → [bd, pkt, qmem];
        // `this.pkt.qmem` and `pkt.qmem` → [pkt, qmem].
        let mut segs: Vec<String> = Vec::new();
        let mut cur = e;
        loop {
            match &cur.kind {
                ExprKind::MemberAccess { expr, member } => {
                    segs.push(member.name.clone());
                    cur = expr;
                }
                ExprKind::Ident(h) => {
                    if h.path.iter().any(|s| !s.selects.is_empty()) {
                        return None;
                    }
                    for seg in h.path.iter().rev() {
                        segs.push(seg.name.name.clone());
                    }
                    break;
                }
                ExprKind::This => break,
                _ => return None,
            }
        }
        segs.reverse();
        if segs.len() < 2 {
            return None;
        }
        // Candidate receiver boundaries, innermost first — the same order
        // `class_member_select_split` tries them.
        for k in 0..segs.len() - 1 {
            let bh = crate::ast::expr::HierarchicalIdentifier {
                root: None,
                path: segs[0..=k]
                    .iter()
                    .map(|n| crate::ast::expr::HierPathSegment {
                        name: crate::ast::Identifier {
                            name: n.clone(),
                            span: e.span,
                        },
                        selects: Vec::new(),
                    })
                    .collect(),
                span: e.span,
                cached_signal_id: std::cell::Cell::new(None),
                cached_resolved_name: std::cell::OnceCell::new(),
            };
            let base = Expression::new(ExprKind::Ident(bh), e.span);
            let Some((handle, prop)) = self.class_prop_receiver(&base) else {
                continue;
            };
            let Some(su) = self.class_prop_struct(handle, &prop) else {
                continue;
            };
            if !Self::spreads_member_wise(&su) {
                continue;
            }
            let r = self.class_struct_coll_descend(handle, &prop, &su, &segs[k + 1..]);
            if let Some(n) = r {
                return Some(n);
            }
        }
        None
    }

    /// Member descent for [`class_struct_coll_name`]: intermediates must be
    /// nested unpacked structs, the LAST segment the collection member.
    pub(super) fn class_struct_coll_descend(
        &mut self,
        handle: usize,
        prop: &str,
        su: &crate::ast::types::StructUnionType,
        tail: &[String],
    ) -> Option<String> {
        let mut level = su.clone();
        let mut key = prop.to_string();
        let mut found: Option<(DataType, StructCollKind, bool)> = None;
        for (j, mname) in tail.iter().enumerate() {
            let (dt, dims) = Self::struct_member_decl(&level, mname)?;
            let last = j + 1 == tail.len();
            match Self::coll_kind_of(&dims) {
                Some(kind) if last => {
                    key.push('.');
                    key.push_str(mname);
                    found = Some((dt, kind, self.string_keyed_dim(&dims[0])));
                    break;
                }
                Some(_) => return None, // collection mid-path: nested machinery
                None => {
                    if last {
                        break; // the final member is not a collection
                    }
                    level = self.unpacked_struct_of(&dt)?;
                    key.push('.');
                    key.push_str(mname);
                }
            }
        }
        let (dt, kind, string_key) = found?;
        let elem_w = resolve_type_width(
            &dt,
            Some(&self.module.parameters),
            Some(&self.module.typedefs),
        )
        .max(1);
        self.register_struct_coll(handle, &key, kind, elem_w, string_key);
        Some(format!("{}#{}", handle, key))
    }

    /// Put a struct-member collection on the §7.8 collection registries
    /// under its instance-scoped `<handle>#<prop>.<member>` name, mirroring
    /// what a directly-declared collection property gets: element cells at
    /// `<name>[k]`, the size shadow at `<name>.size`, queue append-on-write
    /// for the dynamic/queue kinds, key-stringified reads and default
    /// element widths for associative ones. Idempotent — the repeat access
    /// path is one set lookup.
    pub(super) fn register_struct_coll(
        &mut self,
        handle: usize,
        key: &str,
        kind: StructCollKind,
        elem_w: u32,
        string_key: bool,
    ) {
        let name = format!("{}#{}", handle, key);
        // §7.8.1: a `[string]` member keys its entries by text.
        self.register_coll_name(&name, kind, elem_w, string_key);
    }

    /// §7.2.1/§7.5/§7.8/§7.10: the collection a DYNAMIC member of an unpacked
    /// struct designates — kind, element type and whether an associative
    /// member is string-keyed — for the flattened storage name `flat`
    /// (`s.q`, `s.in.q`, `aa[k].q`, `q[2].d`, `fa[1].m`). A struct member
    /// that is a queue / dynamic / associative array is a collection object
    /// of its own, named by the path to it, wherever the struct lives: in a
    /// variable, a nested struct, or an element of any array. The answer
    /// comes from the declared types alone, so a path composed at run time
    /// (an associative key, a queue index) classifies exactly like a static
    /// one. `None` for anything else, including a collection reached through
    /// a further select (`s.q[1]` is an element, not the collection).
    pub(super) fn struct_member_coll_of(
        &self,
        flat: &str,
    ) -> Option<(StructCollKind, DataType, bool)> {
        let (mdt, dims, nidx) = self.struct_member_path_decl(flat)?;
        // §7.4.5: the member itself (`s.q`), or one ELEMENT of a fixed
        // array of collections (`s.sp[1]` of `T sp[4][$]`): every
        // dimension but the trailing collection one selected.
        if matches!(
            dims.first(),
            Some(crate::ast::types::UnpackedDimension::Unsized(_))
        ) && dims.len() > 1
            && nidx < dims.len()
            && dims[1..]
                .iter()
                .all(|d| matches!(d, crate::ast::types::UnpackedDimension::Associative { .. }))
        {
            if nidx > 0 {
                // Locate the final struct member outside array selects. A
                // selected dynamic row is a collection only while allocated;
                // an OOB write must not register a phantom associative row.
                let mut depth = 0usize;
                let mut member_start = 0usize;
                for (i, byte) in flat.bytes().enumerate() {
                    match byte {
                        b'[' => depth += 1,
                        b']' => depth = depth.saturating_sub(1),
                        b'.' if depth == 0 => member_start = i + 1,
                        _ => {}
                    }
                }
                let open = member_start + flat[member_start..].find('[')?;
                let close = open + 1 + flat[open + 1..].find(']')?;
                let index = flat[open + 1..close].parse::<i64>().ok()?;
                if index < 0 || index as u64 >= self.get_queue_size(&flat[..open]) {
                    return None;
                }
            }
            let kind = Self::coll_kind_of(&dims[nidx..=nidx])?;
            return Some((kind, mdt, self.string_keyed_dim(&dims[nidx])));
        }
        let n = dims.len();
        if n == 0 {
            return None;
        }
        // §7.8.1: an associative dimension with more after it (`int
        // m[string][string]`) makes the member (or its element) an
        // associative array of collections, stored under compound keys.
        let k = match dims
            .iter()
            .position(|d| matches!(d, crate::ast::types::UnpackedDimension::Associative { .. }))
        {
            Some(k) if k + 1 < n => k,
            _ => n - 1,
        };
        if nidx != k {
            return None;
        }
        let kind = Self::coll_kind_of(&dims[k..=k])?;
        if dims[..k]
            .iter()
            .any(|d| Self::coll_kind_of(std::slice::from_ref(d)).is_some())
        {
            return None;
        }
        Some((kind, mdt, self.string_keyed_dim(&dims[k])))
    }

    /// The declaration of the struct member the flattened path `flat` ends
    /// at (`s.in.q`, `aa[k].sp[1]`): its element type, its unpacked
    /// dimensions and how many selects the path applies to it. Every
    /// intermediate member must be selected down to one element.
    pub(super) fn struct_member_path_decl(
        &self,
        flat: &str,
    ) -> Option<(DataType, Vec<crate::ast::types::UnpackedDimension>, usize)> {
        let segs = Self::split_flat_path(flat);
        if segs.len() < 2 {
            return None;
        }
        // The root's declared type; for an array root this is already the
        // ELEMENT type, so the root's own selects are consumed.
        let root = segs[0].0.as_str();
        let mut dt: DataType = match self.module.var_decl_types.get(root) {
            Some(d) => d.clone(),
            None => self.class_member_coll_type(root)?.clone(),
        };
        let last = segs.len() - 1;
        for (j, (seg, nidx)) in segs.iter().enumerate().skip(1) {
            let su = self.unpacked_struct_of(&dt)?;
            let (mdt, dims) = Self::struct_member_decl(&su, seg)?;
            if j == last {
                return Some((mdt, dims, *nidx));
            }
            if *nidx != dims.len() {
                return None;
            }
            dt = mdt;
        }
        None
    }

    /// §12.7.3/§7.4.5: the rows of `foreach (a[i, j])` over a ONE-dimension
    /// fixed array of queues / dynamic arrays — a variable (`int a[3][$]`)
    /// or a struct member (`s.sp[i, j]` of `T sp[4][$]`): the flattened
    /// array name and its first-dimension indices in left-to-right order.
    /// Each row `a[i]` is its own collection with its own size.
    pub(super) fn fixed_rows_of_colls(&mut self, array: &Expression) -> Option<(String, Vec<i64>)> {
        let name = match &array.kind {
            ExprKind::Ident(h) if h.path.iter().any(|s| !s.selects.is_empty()) => {
                let chain = Self::hier_select_chain(h, array.span);
                self.flat_member_name(&chain)?
            }
            ExprKind::Ident(h) if h.path.len() == 1 => self.resolve_hier_name(h).into_owned(),
            _ => self.flat_member_name(array)?,
        };
        if let Some(dims) = self.foreach_dims(&name)
            && dims.len() == 1
        {
            let (lo, hi) = dims[0];
            if !self
                .module
                .dynamic_arrays
                .contains(&Self::name_with_index(&name, lo))
            {
                return None;
            }
            let mut rows: Vec<i64> = (lo..=hi).collect();
            if self.module.descending_arrays.contains(&name) {
                rows.reverse();
            }
            return Some((name, rows));
        }
        if !self.struct_colls_possible() {
            return None;
        }
        let (_, dims, nidx) = self.struct_member_path_decl(&name)?;
        if nidx != 0 || dims.len() != 2 {
            return None;
        }
        use crate::ast::types::UnpackedDimension as UD;
        if !matches!(dims[1], UD::Unsized(_) | UD::Queue { .. }) {
            return None;
        }
        let rows: Vec<i64> = match &dims[0] {
            UD::Range { left, right, .. } => {
                let l = self.eval_expr(left).to_i64()?;
                let r = self.eval_expr(right).to_i64()?;
                if l <= r {
                    (l..=r).collect()
                } else {
                    (r..=l).rev().collect()
                }
            }
            UD::Expression { expr, .. } => {
                let n = self.eval_expr(expr).to_i64()?;
                (0..n).collect()
            }
            _ => return None,
        };
        for r in &rows {
            let rn = Self::name_with_index(&name, *r);
            if !self.ensure_struct_member_coll(&rn) {
                return None;
            }
        }
        Some((name, rows))
    }

    /// §12.7.3/§7.4.5: the store and fixed outer shape of `foreach (a[i, j,
    /// k])` over an array whose fixed dimensions end in a collection — a
    /// variable (`int a[3][3][$]`) or a class property (`q`, `h.q`). The
    /// variable after the fixed ones walks each element collection.
    pub(super) fn fixed_dims_of_colls(
        &mut self,
        array: &Expression,
    ) -> Option<(String, Vec<(i64, i64)>)> {
        if let Some(r) = self.class_fixed_array_of_coll_base(array) {
            return Some(r);
        }
        let flat;
        let array = if matches!(array.kind, ExprKind::Ident(_)) {
            array
        } else {
            flat = Self::member_chain_as_hier_ident(array)?;
            &flat
        };
        let ExprKind::Ident(h) = &array.kind else {
            return None;
        };
        if h.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let name = self.resolve_hier_name(h).into_owned();
        // This arm requires fixed outer dimensions, not a collection's
        // allocated backing range.
        if self.module.dynamic_arrays.contains(&name)
            || self.module.associative_arrays.contains_key(&name)
        {
            return None;
        }
        let shape = self.foreach_dims(&name)?;
        let dims = self
            .module
            .unpacked_decl_dims
            .get(&name)
            .filter(|dims| dims.len() == shape.len())
            .cloned()
            .unwrap_or(shape);
        let first = dims
            .iter()
            .fold(name.clone(), |n, d| Self::name_with_index(&n, d.0));
        if !self.module.dynamic_arrays.contains(&first)
            && !self.module.associative_arrays.contains_key(&first)
        {
            return None;
        }
        Some((name, dims))
    }

    /// Put the struct-member collection `name` on the §7.8 collection
    /// registries (see [`register_coll_name`]) when its declared type says it
    /// is one. Returns whether `name` is a registered collection afterwards.
    pub(super) fn ensure_struct_member_coll(&mut self, name: &str) -> bool {
        if self.module.dynamic_arrays.contains(name)
            || self.module.associative_arrays.contains_key(name)
        {
            return true;
        }
        let Some((kind, dt, string_key)) = self.struct_member_coll_of(name) else {
            return false;
        };
        let elem_w = resolve_type_width(
            &dt,
            Some(&self.module.parameters),
            Some(&self.module.typedefs),
        )
        .max(1);
        self.register_coll_name(name, kind, elem_w, string_key);
        // A packed-struct element type slices `m[j].f` / a locator's
        // `item.f` through the collection's field layout.
        if !self.module.packed_struct_fields.contains_key(name) {
            self.register_packed_leaf_layout(name, &dt);
        }
        if self.owner_is_assoc_elem(name) {
            self.assoc_elem_colls.insert(name.to_string());
        }
        true
    }

    /// Whether some select of `name` that is followed by a member (`aa[k].`)
    /// selects an element of an associative array.
    pub(super) fn owner_is_assoc_elem(&self, name: &str) -> bool {
        !self.assoc_elem_owners(name).is_empty()
    }

    /// The associative-array ELEMENTS on the path `name` passes through
    /// (`aa[k]` of `aa[k].q`, outermost first): every top-level select
    /// followed by a member whose container is an associative array.
    pub(super) fn assoc_elem_owners<'n>(&self, name: &'n str) -> Vec<&'n str> {
        let bytes = name.as_bytes();
        let mut out = Vec::new();
        let mut depth = 0usize;
        let mut open = 0usize;
        for (i, &b) in bytes.iter().enumerate() {
            match b {
                b'[' => {
                    if depth == 0 {
                        open = i;
                    }
                    depth += 1;
                }
                b']' if depth > 0 => {
                    depth -= 1;
                    if depth == 0
                        && bytes.get(i + 1) == Some(&b'.')
                        && self.is_associative_array(&name[..open])
                    {
                        out.push(&name[..=i]);
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// §7.2.1/§7.8: the storage name of a dynamic member of an unpacked
    /// struct reached through a select or member chain — `aa[k].q`,
    /// `q[i].d`, `s.in.q`, `arr[2].m` — registered on first sight, so the
    /// collection machinery (methods, element cells, `.size`, concatenation
    /// operands, whole assignment) treats it like a declared collection.
    /// The trailing member name gates the work: only names some struct type
    /// declares with a dynamic dimension go further.
    pub(super) fn struct_member_coll_name(&mut self, e: &Expression) -> Option<String> {
        let flat = match &e.kind {
            ExprKind::MemberAccess { expr: base, member } => {
                if !self.struct_coll_member_possible(&member.name) {
                    return None;
                }
                // An ELEMENT of a class-property collection of structs
                // (`c.aa[k].q`, `fa[i].q` in a method) is keyed under the
                // instance (`<h>#aa[k]`).
                if matches!(base.kind, ExprKind::Index { .. })
                    && !self.no_class_objects()
                    && let Some(row) = self.coll_elem_expr_key(base)
                    && row.contains('#')
                {
                    let f = format!("{}.{}", row, member.name);
                    if self.ensure_struct_member_coll(&f) {
                        return Some(f);
                    }
                }
                if !self.root_may_hold_struct(e) {
                    return None;
                }
                self.flat_member_name(e)?
            }
            // `foreach (aa[k].q[i])` names its array as ONE hierarchical
            // identifier whose segments carry the selects.
            ExprKind::Ident(h)
                if h.path.len() >= 2
                    && h.path.last().is_some_and(|s| s.selects.is_empty())
                    && h.path.iter().any(|s| !s.selects.is_empty()) =>
            {
                let last = &h.path.last()?.name.name;
                if !self.struct_coll_member_possible(last) {
                    return None;
                }
                if !self.root_may_hold_struct(e) {
                    return None;
                }
                let chain = Self::hier_select_chain(h, e.span);
                self.flat_member_name(&chain)?
            }
            _ => return None,
        };
        if !self.ensure_struct_member_coll(&flat) {
            return None;
        }
        Some(flat)
    }

    /// `s.m.first(k)` / `last` / `next` / `prev` on an associative member of
    /// a struct (§7.9.4-§7.9.7): only for a receiver that IS a registered
    /// associative array, so a class's own `first()` is untouched.
    #[cold]
    #[inline(never)]
    pub(super) fn struct_assoc_member_walk(
        &mut self,
        hier: &crate::ast::expr::HierarchicalIdentifier,
        args: &[Expression],
    ) -> Option<Value> {
        let len = hier.path.len();
        let m = hier.path[len - 1].name.name.as_str();
        if !matches!(m, "first" | "last" | "next" | "prev") {
            return None;
        }
        let base_expr = self.method_receiver_expr(hier, len - 1);
        let ExprKind::Ident(bh) = &base_expr.kind else {
            return None;
        };
        let bn = self.resolve_hier_name(bh).into_owned();
        if !self.module.associative_arrays.contains_key(&bn) {
            return None;
        }
        let m = m.to_string();
        self.eval_builtin_method(&bn, &m, args)
    }

    /// The value of a collection element cell wherever it lives (the
    /// `signals` map was already consulted by the caller).
    #[cold]
    #[inline(never)]
    pub(super) fn table_elem_value(&self, name: &str) -> Option<Value> {
        self.get_signal_value_by_name(name)
    }

    /// Whether the variable at the root of the select / member chain `e` is
    /// declared with an unpacked-struct (element) type — the only roots whose
    /// flattened paths `struct_member_coll_of` can classify. Answered before
    /// any name is built, so a class-handle receiver (`obj.q`) costs nothing.
    pub(super) fn root_may_hold_struct(&self, e: &Expression) -> bool {
        let mut cur = e;
        loop {
            match &cur.kind {
                ExprKind::MemberAccess { expr, .. } | ExprKind::Index { expr, .. } => cur = expr,
                ExprKind::Ident(h) => {
                    let skip = usize::from(
                        h.path.len() >= 2 && self.module.packages.contains(&h.path[0].name.name),
                    );
                    let Some(seg) = h.path.get(skip) else {
                        return false;
                    };
                    let bare = seg.name.name.as_str();
                    let root = self.dyn_name_lookup(bare).unwrap_or(bare);
                    let dt = self
                        .module
                        .var_decl_types
                        .get(root)
                        .or_else(|| self.module.var_decl_types.get(bare));
                    return dt.is_some_and(|dt| {
                        matches!(self.resolve_dt_ref(dt), DataType::Struct(su) if Self::spreads_member_wise(su))
                    });
                }
                _ => return false,
            }
        }
    }

    /// The select / member-access chain a hierarchical identifier with
    /// per-segment selects (`a[k].b[i].c`) stands for.
    pub(super) fn hier_select_chain(
        h: &crate::ast::expr::HierarchicalIdentifier,
        span: crate::ast::Span,
    ) -> Expression {
        let mut cur: Option<Expression> = None;
        for seg in &h.path {
            let mut e = match cur.take() {
                None => {
                    let mut root = h.clone();
                    root.path = vec![crate::ast::expr::HierPathSegment {
                        name: seg.name.clone(),
                        selects: Vec::new(),
                    }];
                    root.cached_signal_id = std::cell::Cell::new(None);
                    root.cached_resolved_name = std::cell::OnceCell::new();
                    Expression::new(ExprKind::Ident(root), span)
                }
                Some(b) => Expression::new(
                    ExprKind::MemberAccess {
                        expr: Box::new(b),
                        member: seg.name.clone(),
                    },
                    span,
                ),
            };
            for sel in &seg.selects {
                e = Expression::new(
                    ExprKind::Index {
                        expr: Box::new(e),
                        index: Box::new(sel.clone()),
                    },
                    span,
                );
            }
            cur = Some(e);
        }
        cur.expect("non-empty hierarchical path")
    }

    /// §7.8.6: a write through a dynamic member of an associative-array
    /// ELEMENT (`aa[k].q.push_back(v)`, `aa[k].q = {...}`) creates that
    /// element. Record the presence marker `aa[k]` (the cell `exists`,
    /// `num` and `foreach` enumerate) for every associative ancestor of
    /// `name`.
    pub(super) fn mark_struct_coll_owner_present(&mut self, name: &str) {
        for elem in self.assoc_elem_owners(name) {
            if self.get_signal_value_by_name(elem).is_none() {
                self.set_signal_value_by_name(elem, Value::zero(1));
            }
        }
    }

    /// Put the collection `name` on the §7.8 collection registries, mirroring
    /// what a directly-declared collection gets: element cells at
    /// `<name>[k]`, the size shadow at `<name>.size`, queue append-on-write
    /// for the dynamic/queue kinds, key-stringified reads and default
    /// element widths for associative ones. Idempotent.
    pub(super) fn register_coll_name(
        &mut self,
        name: &str,
        kind: StructCollKind,
        elem_w: u32,
        string_key: bool,
    ) {
        let name = name.to_string();
        let known = match kind {
            StructCollKind::Dynamic => self.module.dynamic_arrays.contains(&name),
            StructCollKind::Queue => self.module.queue_vars.contains(&name),
            StructCollKind::Assoc => self.module.associative_arrays.contains_key(&name),
        };
        if known {
            return;
        }
        match kind {
            StructCollKind::Dynamic | StructCollKind::Queue => {
                self.module.arrays.insert(name.clone(), (0, -1, elem_w));
                self.module.dynamic_arrays.insert(name.clone());
                if kind == StructCollKind::Queue {
                    self.module.queue_vars.insert(name.clone());
                }
                if elem_w > 0 {
                    self.module.assoc_elem_widths.insert(name.clone(), elem_w);
                }
                if self
                    .get_signal_value_by_name(&Self::name_with_suffix(&name, ".size"))
                    .is_none()
                {
                    self.set_queue_size(&name, 0);
                }
            }
            StructCollKind::Assoc => {
                self.module
                    .associative_arrays
                    .insert(name.clone(), string_key);
                if elem_w > 0 {
                    self.module.assoc_elem_widths.insert(name, elem_w);
                }
            }
        }
    }

    /// `<obj>.<prop>[i]` where `prop` is a MULTI-DIMENSIONAL packed array
    /// (`bit [1:0][7:0] arr;`) — resolve the element to its bit slice of the
    /// property's raw integral value.
    ///
    /// A class property carries no per-element width at run time (the module
    /// scope equivalent comes from `packed_signal_elem_widths`, which only
    /// covers signals), so an index on one fell through to a plain BIT select:
    /// `arr[1]` on `16'hAABB` read back `1` instead of `8'hAA`, and an indexed
    /// WRITE was dropped entirely. The declared type retained in
    /// `ElaboratedClass::property_types` supplies the shape.
    ///
    /// Returns `None` — leaving the previous behavior in place — for anything
    /// not a resolvable multi-dimensional packed range, including an
    /// out-of-bounds index.
    pub(super) fn class_packed_elem_ref(
        &mut self,
        base: &Expression,
        index: &Expression,
    ) -> Option<ClassAggRef> {
        if self.no_class_objects() {
            return None;
        }
        // Only a packed-vector property can be this select; any other name
        // (an interface or module signal read in an assertion, say) skips
        // the receiver evaluation below.
        let prop = match &base.kind {
            ExprKind::MemberAccess { member, .. } => member.name.as_str(),
            ExprKind::Ident(h) => h.path.last()?.name.name.as_str(),
            _ => return None,
        };
        if !self.class_member_names().packed_vec_props.contains(prop) {
            return None;
        }
        let (handle, prop) = self.class_prop_receiver(base)?;
        let (elem_w, total, left, right) = self.class_prop_packed_shape(handle, &prop)?;
        let idx = self.eval_expr(index).to_i64()?;
        if idx < left.min(right) || idx > left.max(right) {
            return None;
        }
        // `[1:0]` puts index 1 at the MS end; an ascending `[0:1]` puts index 0
        // there instead (§7.4.1).
        let slot = if left >= right {
            idx - right
        } else {
            right - idx
        };
        let off = (slot as u32).checked_mul(elem_w)?;
        if off.checked_add(elem_w)? > total {
            return None;
        }
        Some(ClassAggRef::Packed {
            handle,
            prop,
            off,
            w: elem_w,
            total,
            two_state: false,
        })
    }

    /// Shape of a multi-dimensional packed-array class property as
    /// `(element_width, total_width, outer_left, outer_right)`. `None` unless
    /// the declared type is a packed vector with at least two dimensions whose
    /// bounds all resolve.
    pub(super) fn class_prop_packed_shape(
        &self,
        handle: usize,
        prop: &str,
    ) -> Option<(u32, u32, i64, i64)> {
        use crate::ast::types::PackedDimension;
        let inst = self.heap.get(handle)?.as_ref()?;
        // Class value parameters are instance properties (§8.25), so they can
        // size the range; fall back to module scope for everything else.
        //
        // Collect only the OVERRIDES here. Cloning `module.parameters` up
        // front cost a full HashMap clone on every call — including on the
        // several early-return paths below, which never look at a parameter
        // at all. On the axi4 UVM AVIP this one clone was the largest named
        // contributor to both the allocator and `memmove`. The map is now
        // materialised only if a packed dimension actually needs
        // const-evaluating, and is BORROWED outright when the class chain
        // contributes no overrides (the common case).
        let mut param_overrides: Vec<(String, Value)> = Vec::new();
        let mut cur = Some(inst.class_name.clone());
        let mut seen: HashSet<String> = HashSet::default();
        let mut decl: Option<&crate::ast::types::DataType> = None;
        while let Some(cn) = cur {
            if !seen.insert(cn.clone()) {
                break;
            }
            let cd = self.module.classes.get(&cn)?;
            // A property that ALSO has unpacked dimensions (`bit [1:0][7:0]
            // arr [4];`, a queue, an associative array) indexes its unpacked
            // dimension first (§7.4.2) — that is not this select, so leave
            // those to the collection paths.
            if cd.array_properties.contains_key(prop)
                || cd.array_nd_properties.contains_key(prop)
                || cd.queue_properties.contains_key(prop)
                || cd.assoc_properties.contains_key(prop)
                // Static collections index their unpacked dimension first
                // too (§7.4.2) — route them to the collection paths, not
                // the packed-select machinery.
                || cd.static_collections.iter().any(|(n, _, _)| n == prop)
                // A fixed-size static array indexes its unpacked dimension
                // first as well — it is an array, just a globally-stored one.
                || cd.static_fixed_arrays.iter().any(|(n, ..)| n == prop)
            {
                return None;
            }
            for (pname, _) in &cd.param_defaults {
                if let Some(v) = inst.properties.get(pname) {
                    param_overrides.push((pname.clone(), v.clone()));
                }
            }
            if let Some(dt) = cd.property_types.get(prop) {
                decl = Some(dt);
                break;
            }
            cur = cd.extends.clone();
        }
        let dims = match decl? {
            crate::ast::types::DataType::IntegerVector { dimensions, .. } => dimensions,
            crate::ast::types::DataType::Implicit { dimensions, .. } => dimensions,
            _ => return None,
        };
        // A SINGLE packed dimension is a plain vector, whose element is one
        // bit. Reads of `obj.v[i]` fell through to a generic bit-select and
        // were fine, but the WRITE had no handler at all and was silently
        // dropped — `obj.v[15] = 1'b1` left the property unchanged while the
        // part-select `obj.v[15:8] = ...` beside it worked. Resolving it here
        // gives the write a target, and the slot arithmetic below already
        // honours an ascending declaration.
        if dims.is_empty() {
            return None;
        }
        // Only now is a parameter scope needed.
        let params: std::borrow::Cow<'_, HashMap<String, Value>> = if param_overrides.is_empty() {
            std::borrow::Cow::Borrowed(&self.module.parameters)
        } else {
            let mut m = self.module.parameters.clone();
            // Leaf class first (§8.25): the walk pushed leaf overrides before
            // ancestors, and the original merge let a LATER (ancestor) insert
            // win, so replay in the same order.
            for (k, v) in &param_overrides {
                m.insert(k.clone(), v.clone());
            }
            std::borrow::Cow::Owned(m)
        };
        let bounds = |d: &PackedDimension| -> Option<(i64, i64)> {
            let PackedDimension::Range { left, right, .. } = d else {
                return None;
            };
            Some((
                crate::elaborate::const_eval_i64_with_params(left, Some(&*params))?,
                crate::elaborate::const_eval_i64_with_params(right, Some(&*params))?,
            ))
        };
        let (left, right) = bounds(&dims[0])?;
        let mut elem_w: u32 = 1;
        for d in &dims[1..] {
            let (l, r) = bounds(d)?;
            elem_w = elem_w.checked_mul(((l - r).abs() + 1) as u32)?;
        }
        let outer = ((left - right).abs() + 1) as u32;
        Some((elem_w, elem_w.checked_mul(outer)?, left, right))
    }

    /// Module parameters overlaid with the class-parameter values bound on
    /// `handle` (leaf class first, §8.25), for sizing declarations that
    /// reference either scope — e.g. a struct member `bit [W-1:0] f;` inside
    /// an inline-struct property of `box #(W)`.
    pub(super) fn instance_param_scope(&self, handle: usize) -> HashMap<String, Value> {
        let mut params = self.module.parameters.clone();
        for (n, v) in self.instance_value_params(handle) {
            params.entry(n).or_insert(v);
        }
        params
    }

    /// The class value parameters bound on object `handle`, leaf class first
    /// (§8.25): each specialization's own values, as its methods see them.
    pub(super) fn instance_value_params(&self, handle: usize) -> Vec<(String, Value)> {
        let mut out: Vec<(String, Value)> = Vec::new();
        let Some(inst) = self.heap.get(handle).and_then(|o| o.as_ref()) else {
            return out;
        };
        let mut cur = Some(inst.class_name.clone());
        let mut seen: HashSet<String> = HashSet::default();
        while let Some(cn) = cur {
            if !seen.insert(cn.clone()) {
                break;
            }
            let Some(cd) = self.module.classes.get(&cn) else {
                break;
            };
            for (pname, _) in &cd.param_defaults {
                if out.iter().any(|(n, _)| n == pname) {
                    continue;
                }
                if let Some(v) = inst.properties.get(pname) {
                    out.push((pname.clone(), v.clone()));
                }
            }
            cur = cd.extends.clone();
        }
        out
    }

    /// `(handle, prop, elem_struct)` when `lvalue` — with any trailing element
    /// select stripped — names a CLASS PROPERTY whose element type is an
    /// unpacked struct. Drives a member-wise decomposition of a whole-element
    /// write into the same cells `class_unpacked_leaf` reads back.
    /// Cheap syntactic pre-check for the class unpacked-struct array
    /// write paths: the lvalue's root identifier must name a class
    /// property declared as an unpacked struct (of any class). Computed
    /// once from `property_types`; a UVM run otherwise paid a receiver
    /// resolution on every blocking assignment (+1.3% instructions on the
    /// axi4 benchmark) for a write shape it never has.
    pub(super) fn lvalue_root_is_unpacked_struct_prop(
        &self,
        lvalue: &Expression,
        allow_bare: bool,
    ) -> bool {
        if self.no_class_objects() {
            return false;
        }
        // The common `x = ..` write: one bare identifier, no select. Only
        // the whole-array pattern site (`A = '{..}`) may pass one through.
        if !allow_bare {
            if let ExprKind::Ident(h) = &lvalue.kind {
                if h.path.len() == 1 && h.path[0].selects.is_empty() {
                    return false;
                }
            }
        }
        let set = self.unpacked_struct_prop_names.get_or_init(|| {
            let mut set: HashSet<String> = HashSet::default();
            for cd in self.module.classes.values() {
                for (name, dt) in &cd.property_types {
                    let dt = Self::resolve_type_ref(dt, &self.module.typedef_types);
                    if let DataType::Struct(su) = &dt {
                        if Self::spreads_member_wise(su) {
                            set.insert(name.clone());
                        }
                    }
                }
            }
            set
        });
        if set.is_empty() {
            return false;
        }
        let mut cur = lvalue;
        loop {
            match &cur.kind {
                ExprKind::Index { expr, .. } => cur = expr.as_ref(),
                ExprKind::Ident(h) => {
                    return h.path.len() <= 2
                        && h.path.iter().any(|seg| set.contains(&seg.name.name));
                }
                ExprKind::MemberAccess { expr, member } => {
                    if set.contains(&member.name) {
                        return true;
                    }
                    cur = expr.as_ref();
                }
                _ => return false,
            }
        }
    }

    pub(super) fn class_unpacked_array_prop_of(
        &mut self,
        lvalue: &Expression,
    ) -> Option<(usize, String, crate::ast::types::StructUnionType)> {
        if self.no_class_objects() {
            return None;
        }
        let recv = match self.class_prop_receiver(lvalue) {
            Some(r) => Some(r),
            None => match &lvalue.kind {
                ExprKind::Index { expr, .. } => self.class_prop_receiver(expr),
                ExprKind::Ident(h) if h.path.len() == 1 && !h.path[0].selects.is_empty() => {
                    let bare = crate::ast::expr::HierarchicalIdentifier {
                        root: None,
                        path: vec![crate::ast::expr::HierPathSegment {
                            name: h.path[0].name.clone(),
                            selects: Vec::new(),
                        }],
                        span: h.span,
                        cached_signal_id: std::cell::Cell::new(None),
                        cached_resolved_name: std::cell::OnceCell::new(),
                    };
                    let e = Expression::new(ExprKind::Ident(bare), lvalue.span);
                    self.class_prop_receiver(&e)
                }
                _ => None,
            },
        };
        let (handle, prop) = recv?;
        let su = self.class_prop_struct(handle, &prop)?;
        Self::spreads_member_wise(&su).then_some((handle, prop, su))
    }

    /// True when the READ path (`class_unpacked_leaf`) owns this element's
    /// leaves — i.e. they live in the instance property map as
    /// `<prop>[i].<member>`, not under an instance-scoped SIGNAL name. Used to
    /// keep whole-element / whole-array writes in the same store the reads use.
    pub(super) fn class_unpacked_elem_is_heap_owned(&mut self, lvalue: &Expression) -> bool {
        let Some((_, _, su)) = self.class_unpacked_array_prop_of(lvalue) else {
            return false;
        };
        let Some(m0) = su
            .members
            .first()
            .and_then(|m| m.declarators.first())
            .map(|d| d.name.name.clone())
        else {
            return false;
        };
        let probe = Self::append_member_expr(lvalue, &m0);
        self.class_unpacked_leaf(&probe).is_some()
    }

    /// Write one unpacked-struct value into a class-property element's leaf
    /// cells, member by member, so each lands via the (working) per-leaf path.
    pub(super) fn assign_class_unpacked_elem(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
        su: &crate::ast::types::StructUnionType,
    ) {
        let items = match &rvalue.kind {
            ExprKind::AssignmentPattern(its) => Some(its.as_slice()),
            _ => None,
        };
        let ordered: Vec<_> = items
            .unwrap_or(&[])
            .iter()
            .filter_map(|it| match it {
                AssignmentPatternItem::Ordered(e) => Some(e),
                _ => None,
            })
            .collect();
        let mut k = 0usize;
        for m in &su.members {
            for md in &m.declarators {
                let mn = md.name.name.as_str();
                let lhs_f = Self::append_member_expr(lvalue, mn);
                // §10.9.1: named and type keys take precedence over the
                // positional/default items, including constructor initializers.
                let rhs_f = if let Some(its) = items {
                    let selected = its
                        .iter()
                        .find_map(|it| match it {
                            AssignmentPatternItem::Named(id, e) if id.name == mn => Some(e),
                            _ => None,
                        })
                        .or_else(|| {
                            its.iter().find_map(|it| match it {
                                AssignmentPatternItem::Typed(dt, e)
                                    if self.pattern_type_matches(&m.data_type, dt) =>
                                {
                                    Some(e)
                                }
                                _ => None,
                            })
                        })
                        .or_else(|| ordered.get(k).copied())
                        .or_else(|| {
                            its.iter().find_map(|it| match it {
                                AssignmentPatternItem::Default(e) => Some(e),
                                _ => None,
                            })
                        });
                    let Some(e) = selected else {
                        k += 1;
                        continue;
                    };
                    e.clone()
                } else {
                    Self::append_member_expr(rvalue, mn)
                };
                if let Some(inner) = self.unpacked_struct_of(&m.data_type) {
                    self.assign_class_unpacked_elem(&lhs_f, &rhs_f, &inner);
                } else {
                    let w = resolve_type_width(
                        &m.data_type,
                        Some(&self.module.parameters),
                        Some(&self.module.typedefs),
                    )
                    .max(1);
                    let v = self.eval_expr_ctx(&rhs_f, w);
                    self.assign_value(&lhs_f, &v);
                }
                k += 1;
            }
        }
    }

    /// §7.2: `rvalue` names an unpacked-struct ELEMENT (`q[i]`, `o.q[i]`,
    /// `q[i].inner`) through selects, members and side-effect-free indices
    /// only, so each of its members can be read as `rvalue.<m>` without
    /// re-running anything.
    pub(super) fn is_struct_elem_ref(e: &Expression) -> bool {
        fn pure(e: &Expression) -> bool {
            match &e.kind {
                ExprKind::Number(_) | ExprKind::StringLiteral(_) | ExprKind::This => true,
                ExprKind::Ident(h) => h.path.iter().all(|s| s.selects.iter().all(pure)),
                ExprKind::Index { expr, index } => pure(expr) && pure(index),
                ExprKind::MemberAccess { expr, .. } | ExprKind::Paren(expr) => pure(expr),
                ExprKind::Unary { op, operand } => {
                    !matches!(
                        op,
                        UnaryOp::PreIncr | UnaryOp::PreDecr | UnaryOp::PostIncr | UnaryOp::PostDecr
                    ) && pure(operand)
                }
                ExprKind::Binary { left, right, .. } => pure(left) && pure(right),
                _ => false,
            }
        }
        let mut cur = e;
        let mut indexed = false;
        loop {
            match &cur.kind {
                ExprKind::Index { expr, index } => {
                    if !pure(index) {
                        return false;
                    }
                    indexed = true;
                    cur = expr;
                }
                ExprKind::MemberAccess { expr, .. } => cur = expr,
                ExprKind::Ident(h) => {
                    return (indexed || h.path.iter().any(|s| !s.selects.is_empty()))
                        && h.path.iter().all(|s| s.selects.iter().all(pure));
                }
                ExprKind::This => return indexed,
                _ => return false,
            }
        }
    }

    /// §7.2: copy an unpacked struct member by member, `lvalue.<m> =
    /// rvalue.<m>`, through the per-member read and write paths. Used when the
    /// source is a collection element whose leaves have no flat name — an
    /// element of a class-property queue/array lives under the instance
    /// (`<h>#q[i].<m>` or the object's property map), so the name-to-name copy
    /// found nothing and the packed fallback stored zeros. Nested unpacked
    /// structs and fixed member arrays recurse to their leaves.
    pub(super) fn assign_struct_memberwise(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
        su: &crate::ast::types::StructUnionType,
        depth: u32,
    ) {
        if depth > 8 {
            return;
        }
        let index_expr = |base: &Expression, i: i64| {
            Expression::new(
                ExprKind::Index {
                    expr: Box::new(base.clone()),
                    index: Box::new(Expression::new(
                        ExprKind::Number(NumberLiteral::Integer {
                            size: None,
                            signed: true,
                            base: NumberBase::Decimal,
                            value: i.to_string(),
                            cached_val: std::cell::Cell::new(None),
                        }),
                        base.span,
                    )),
                },
                base.span,
            )
        };
        for m in &su.members {
            let nested = self.unpacked_struct_of(&m.data_type);
            for md in &m.declarators {
                let mn = md.name.name.as_str();
                let lhs_f = Self::append_member_expr(lvalue, mn);
                let rhs_f = Self::append_member_expr(rvalue, mn);
                let pairs: Vec<(Expression, Expression)> = if md.dimensions.is_empty() {
                    vec![(lhs_f, rhs_f)]
                } else {
                    match self.member_elem_indices(&md.dimensions) {
                        Some(tups) => tups
                            .iter()
                            .map(|t| {
                                (
                                    t.iter().fold(lhs_f.clone(), |e, &i| index_expr(&e, i)),
                                    t.iter().fold(rhs_f.clone(), |e, &i| index_expr(&e, i)),
                                )
                            })
                            .collect(),
                        // Dynamic member: whole-member copy.
                        None => {
                            let v = self.eval_expr(&rhs_f);
                            self.assign_value(&lhs_f, &v);
                            continue;
                        }
                    }
                };
                for (l, r) in pairs {
                    match &nested {
                        Some(inner) => self.assign_struct_memberwise(&l, &r, inner, depth + 1),
                        None => {
                            let v = self.eval_expr(&r);
                            self.assign_value(&l, &v);
                        }
                    }
                }
            }
        }
    }

    /// The unpacked-struct type of an element reference (`q[i]`, `o.q[i]`,
    /// `fa[i]`, `q[i].inner`) whose leaves live under a class instance rather
    /// than under the flat name. `None` for anything else, including an
    /// element whose flat leaves exist (the name-based paths own those).
    pub(super) fn struct_elem_type(
        &mut self,
        e: &Expression,
    ) -> Option<crate::ast::types::StructUnionType> {
        if self.no_class_objects()
            || !Self::is_struct_elem_ref(e)
            || !self.lvalue_root_is_unpacked_struct_prop(e, false)
        {
            return None;
        }
        // A fixed-array property: leaves in the object's property map.
        if let Some((_, _, su)) = self.class_unpacked_array_prop_of(e) {
            if self.class_unpacked_elem_is_heap_owned(e) {
                return Some(su);
            }
        }
        // A queue/dynamic/associative property: leaves at `<h>#q[i].<m>`.
        let flat = self.flat_member_name(e)?;
        let cut = flat.find('[')?;
        let storage = self.resolve_locator_storage(&flat[..cut]);
        if storage == flat[..cut] {
            return None;
        }
        let mut su = self.queue_elem_struct(&storage)?;
        // Walk any members below the element (`q[i].inner`).
        let rest = &flat[cut..];
        let mut after = rest.find(']').map(|p| &rest[p + 1..])?;
        while let Some(tail) = after.strip_prefix('.') {
            let name_end = tail.find(['.', '[']).unwrap_or(tail.len());
            let (mname, more) = tail.split_at(name_end);
            if more.starts_with('[') {
                return None;
            }
            let dt = su.members.iter().find_map(|m| {
                m.declarators
                    .iter()
                    .any(|d| d.name.name == mname && d.dimensions.is_empty())
                    .then(|| m.data_type.clone())
            })?;
            su = self.unpacked_struct_of(&dt)?;
            after = more;
        }
        after.is_empty().then_some(su)
    }

    /// `(name, type)` when `e` is a bare unpacked-struct LOCAL of the current
    /// frame. Its leaves live in the frame, but reading the bare name as a
    /// value consults the module first: an `initial`-block struct of the same
    /// name answered `return r;` with its own (x) container.
    pub(super) fn frame_struct_local(
        &self,
        e: &Expression,
    ) -> Option<(String, crate::ast::types::StructUnionType)> {
        let ExprKind::Ident(h) = &e.kind else {
            return None;
        };
        if h.path.len() != 1 || !h.path[0].selects.is_empty() {
            return None;
        }
        let name = &h.path[0].name.name;
        let frame = self.local_stack.last()?;
        if frame.contains_key(name) {
            return None;
        }
        let su = self.unpacked_struct_of(self.module.var_decl_types.get(name)?)?;
        let first = self.struct_leaf_suffixes(&su).into_iter().next()?;
        frame
            .contains_key(&format!("{}.{}", name, first))
            .then(|| (name.clone(), su))
    }

    /// `<h>#q[k].arr[i]` for the flattened member-array base `q[k].arr` when
    /// `q` is a class-property collection of unpacked structs; `None`
    /// otherwise.
    pub(super) fn class_coll_elem_leaf(&self, base: &str, i: i64) -> Option<String> {
        if self.no_class_objects() {
            return None;
        }
        let cut = base.find('[')?;
        let storage = self.resolve_locator_storage(&base[..cut]);
        if storage == base[..cut] || self.queue_elem_struct(&storage).is_none() {
            return None;
        }
        Some(format!("{}{}[{}]", storage, &base[cut..], i))
    }

    /// The packed value of an unpacked-struct element reference, assembled
    /// from its leaves read one by one (laid out like `pack_unpacked_struct`).
    /// A struct travels as a packed value across a function return.
    pub(super) fn pack_struct_elem_ref(
        &mut self,
        e: &Expression,
        su: &crate::ast::types::StructUnionType,
    ) -> Value {
        let mut leaves = Vec::new();
        self.unpacked_struct_leaves("X", e, su, 0, &mut leaves);
        let total: u32 = leaves.iter().map(|l| l.2).sum();
        let mut out = Value::new(total.max(1));
        let mut off = 0u32;
        for (_, le, w, _) in leaves.into_iter().rev() {
            let v = self.eval_expr(&le);
            for i in 0..w {
                out.set_bit((off + i) as usize, v.get_bit(i as usize));
            }
            off += w;
        }
        out
    }

    /// `class_agg_member` for an already-split `<base>.<field>`.
    pub(super) fn class_agg_member_parts(
        &mut self,
        base: &Expression,
        field: &str,
    ) -> Option<ClassAggRef> {
        if self.no_class_objects()
            || !Self::receiver_prop_name(base).is_some_and(|p| self.struct_prop_name_possible(p))
        {
            return None;
        }
        let (handle, prop) = self.class_prop_receiver(base)?;
        let su = self.class_prop_struct(handle, &prop)?;
        if Self::spreads_member_wise(&su) {
            // §18.4: unpacked struct — one cell per member, keyed
            // `<prop>.<member>` in the instance's property map.
            let w = su
                .members
                .iter()
                .find(|m| m.declarators.iter().any(|d| d.name.name == field))
                .map(|m| {
                    resolve_type_width(
                        &m.data_type,
                        Some(&self.module.parameters),
                        Some(&self.module.typedefs),
                    )
                    .max(1)
                })?;
            return Some(ClassAggRef::Unpacked {
                handle,
                key: format!("{}.{}", prop, field),
                w,
            });
        }
        let inst_params = self.instance_param_scope(handle);
        let (fields, total) = self.packed_agg_layout_with(&su, &inst_params);
        let (_, off, w) = fields.iter().find(|(n, _, _)| *n == field).cloned()?;
        let two_state = su
            .members
            .iter()
            .find(|m| m.declarators.iter().any(|d| d.name.name == field))
            .is_some_and(|m| self.mixed_member_two_state(&su, &m.data_type));
        Some(ClassAggRef::Packed {
            handle,
            prop,
            off,
            w,
            total,
            two_state,
        })
    }

    /// Read the value an aggregate class-property member aliases.
    pub(super) fn read_class_agg(&self, r: &ClassAggRef) -> Value {
        match r {
            ClassAggRef::Packed {
                handle,
                prop,
                off,
                w,
                total,
                two_state,
            } => {
                let whole = self
                    .heap
                    .get(*handle)
                    .and_then(|o| o.as_ref())
                    .and_then(|i| i.properties.get(prop))
                    .cloned()
                    .unwrap_or_else(|| Value::zero(*total));
                let mut out = Value::zero((*w).max(1));
                for i in 0..*w {
                    out.set_bit(i as usize, whole.get_bit((off + i) as usize));
                }
                if *two_state && out.has_xz() {
                    out = out.to_two_state();
                }
                out
            }
            ClassAggRef::Unpacked { handle, key, w } => self
                .heap
                .get(*handle)
                .and_then(|o| o.as_ref())
                .and_then(|i| i.properties.get(key))
                .cloned()
                .unwrap_or_else(|| Value::zero((*w).max(1))),
            ClassAggRef::UnpackedSlice {
                handle,
                key,
                off,
                w,
                cell_w,
            } => {
                let cell = self
                    .heap
                    .get(*handle)
                    .and_then(|o| o.as_ref())
                    .and_then(|i| i.properties.get(key))
                    .cloned()
                    .unwrap_or_else(|| Value::zero((*cell_w).max(1)));
                let mut out = Value::zero((*w).max(1));
                for i in 0..*w {
                    out.set_bit(i as usize, cell.get_bit((off + i) as usize));
                }
                out
            }
        }
    }

    /// Write through an aggregate class-property member. For a PACKED aggregate
    /// this splices the member's bits into the property's raw integral value —
    /// which is what makes a union's members alias each other (§7.3.1) and a
    /// packed struct's whole/field views stay consistent (§18.4).
    /// Returns whether the stored value changed.
    pub(super) fn write_class_agg(&mut self, r: &ClassAggRef, val: &Value) -> bool {
        match r {
            ClassAggRef::Packed {
                handle,
                prop,
                off,
                w,
                total,
                two_state,
            } => {
                let val = &if *two_state && val.has_xz() {
                    val.to_two_state()
                } else {
                    val.clone()
                };
                let mut whole = self
                    .heap
                    .get(*handle)
                    .and_then(|o| o.as_ref())
                    .and_then(|i| i.properties.get(prop))
                    .cloned()
                    .unwrap_or_else(|| Value::zero(*total));
                if whole.width != *total {
                    whole = whole.resize(*total);
                }
                let before = whole.clone();
                for i in 0..*w {
                    whole.set_bit((off + i) as usize, val.get_bit(i as usize));
                }
                let changed = whole != before;
                if let Some(Some(inst)) = self.heap.get_mut(*handle) {
                    inst.properties.insert(prop.clone(), whole);
                }
                changed
            }
            ClassAggRef::Unpacked { handle, key, w } => {
                let nv = val.resize((*w).max(1));
                if let Some(Some(inst)) = self.heap.get_mut(*handle) {
                    let changed = inst.properties.get(key) != Some(&nv);
                    inst.properties.insert(key.clone(), nv);
                    return changed;
                }
                false
            }
            ClassAggRef::UnpackedSlice {
                handle,
                key,
                off,
                w,
                cell_w,
            } => {
                // Read-modify-write the one cell: the packed-array member (or
                // the part-selected element) is a bit field of it.
                let mut cell = self
                    .heap
                    .get(*handle)
                    .and_then(|o| o.as_ref())
                    .and_then(|i| i.properties.get(key))
                    .cloned()
                    .unwrap_or_else(|| Value::zero((*cell_w).max(1)));
                if cell.width < *cell_w {
                    cell = cell.resize(*cell_w);
                }
                let before = cell.clone();
                for i in 0..*w {
                    cell.set_bit((off + i) as usize, val.get_bit(i as usize));
                }
                let changed = cell != before;
                if let Some(Some(inst)) = self.heap.get_mut(*handle) {
                    inst.properties.insert(key.clone(), cell);
                }
                changed
            }
        }
    }

    /// A struct-typed element of a class collection. `elem` is a fully-indexed
    /// element (`m[5]`, `m[k1][k2][k3]`) whose base is an associative-array
    /// class member storing UNPACKED-struct elements. Resolves both the flat
    /// compound storage key (`<handle>#<prop>[K1]...[Kn]`, the same key the
    /// multidim/single-index element write uses) and the element struct type, so
    /// member-cell reads/writes (<key>.<member> signals, the established model
    /// for 1-D assoc-of-struct from the `assoc[key].field` path) can be used for
    /// any index arity. Returns `None` when the base isn't an assoc array whose
    /// element is an unpacked struct.
    pub(super) fn coll_assoc_struct_elem(
        &mut self,
        element: &Expression,
    ) -> Option<(String, crate::ast::types::StructUnionType)> {
        // compound key `base[K1][K2]...[Kn]` (assoc key-stringified).
        let key = self.nested_index_name(element)?;
        let scoped = key.split('[').next().unwrap_or("");
        let (hstr, prop) = scoped.split_once('#')?;
        let handle: usize = hstr.parse().ok()?;
        // `class_prop_struct` deliberately refuses COLLECTION (assoc/queue)
        // properties (issue #113: their element leaves go through the
        // collection machinery). Here the element IS the collection's struct
        // value, so resolve its declared DataType directly from the
        // instance's class-chain `property_decl_types` and require it to be
        // an unpacked struct (spread member-wise).
        let su = self.collection_elem_struct(handle, prop)?;
        if !Self::spreads_member_wise(&su) {
            return None;
        }
        Some((key, su))
    }

    /// The store of an associative-array class member reached through a
    /// handle (`h.m`, `obj.m`), for a multidimensional element.
    #[cold]
    #[inline(never)]
    pub(super) fn handle_assoc_storage_name(&mut self, e: &Expression) -> Option<String> {
        let n = self.expr_assoc_name(e)?;
        self.is_associative_array(&n).then_some(n)
    }

    /// §7.10.1: evaluate an index that names `$`, the last element of the
    /// queue or dynamic array `base`.
    #[cold]
    #[inline(never)]
    pub(super) fn eval_index_of_queue(&mut self, base: &str, index: &Expression) -> Value {
        self.dollar_bound.push(self.get_queue_size(base) as i64 - 1);
        let iv = self.eval_expr(index);
        self.dollar_bound.pop();
        iv
    }

    /// §7.10.1: the member default that an x/z-holding member leaf
    /// `<base_flat>.<member>` of a queue or dynamic-array element past the
    /// end really reads (its leaf holds no element's value).
    #[cold]
    #[inline(never)]
    pub(super) fn missing_leaf_default(
        &mut self,
        expr: &Expression,
        member: &str,
        base_flat: &str,
    ) -> Option<Value> {
        if base_flat.ends_with(']')
            && base_flat
                .rfind('[')
                .is_some_and(|cut| self.module.dynamic_arrays.contains(&base_flat[..cut]))
        {
            return self.missing_struct_elem_member(expr, member);
        }
        None
    }

    /// §7.8.6 / §7.10.1: `coll[i].member` where `coll` is an associative
    /// array, queue or dynamic array of UNPACKED structs (a module or local
    /// variable, or a class member) and element `i` does not exist reads the
    /// member type's default — x for a 4-state member, 0 for a 2-state one —
    /// without creating the element. `None` when the element exists or the
    /// shape is anything else (the regular paths read it).
    #[inline(never)]
    pub(super) fn missing_struct_elem_member(
        &mut self,
        expr: &Expression,
        member: &str,
    ) -> Option<Value> {
        let ExprKind::Index { expr: base, index } = &expr.kind else {
            return None;
        };
        // Only a name some struct collection may carry gets the lookups.
        let leaf = match &base.kind {
            ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.as_str()),
            ExprKind::MemberAccess { member, .. } => Some(member.name.as_str()),
            _ => None,
        }?;
        // (a struct-capable property or variable name; a subroutine-local
        // collection is known by its declared type only).
        if !self.struct_prop_name_possible(leaf)
            && !self.struct_root_possible(leaf)
            && !self.module.var_decl_types.get(leaf).is_some_and(|dt| {
                matches!(self.resolve_dt_ref(dt), DataType::Struct(su) if Self::spreads_member_wise(su))
            })
        {
            return None;
        }
        let an = match self.expr_assoc_name(base) {
            Some(n) => n,
            None => {
                let ExprKind::Ident(h) = &base.kind else {
                    return None;
                };
                let n = self.resolve_hier_name(h).into_owned();
                self.instance_assoc_member(&n).unwrap_or(n)
            }
        };
        let is_assoc = self.is_associative_array(&an);
        if !is_assoc && !self.module.dynamic_arrays.contains(&an) {
            return None;
        }
        let su = self.queue_elem_struct(&an)?;
        let exists = if is_assoc {
            let kv = self.eval_expr(index);
            if kv.has_xz() && !kv.is_real {
                return None;
            }
            let key = self.assoc_key_str(&an, &kv);
            let elem = Self::name_with_key(&an, &key);
            self.signals.contains_key(&elem)
                || self.signal_name_to_id.contains_key(elem.as_str())
                || self
                    .signals
                    .any_key_with_prefix(&Self::name_with_suffix(&elem, "."))
        } else {
            let size = self.get_queue_size(&an);
            self.dollar_bound.push(size as i64 - 1);
            let iv = self.eval_expr(index);
            self.dollar_bound.pop();
            match iv.to_i64() {
                Some(i) if !iv.has_xz() => i >= 0 && (i as u64) < size,
                _ => false,
            }
        };
        if exists {
            return None;
        }
        let mut defaults = Vec::new();
        self.struct_leaf_plain_defaults("", &su, 0, &mut defaults);
        let want = format!(".{member}");
        defaults
            .into_iter()
            .find(|(n, _)| *n == want)
            .map(|(_, v)| v)
    }

    /// §13.4.1 / §6.8: the initial leaves of a function's unpacked-struct
    /// return variable `ret_name` — each member starts at its type's default,
    /// x for a 4-state member and 0 for a 2-state one.
    pub(super) fn struct_return_leaf_seeds(
        &mut self,
        ret_name: &str,
        su: &crate::ast::types::StructUnionType,
    ) -> Vec<(String, Value)> {
        let mut defaults = Vec::new();
        self.struct_leaf_plain_defaults(ret_name, su, 0, &mut defaults);
        let defaults: HashMap<String, Value> = defaults.into_iter().collect();
        self.unpacked_struct_leaf_keys(ret_name, su)
            .into_iter()
            .map(|(k, w, is_real)| {
                let seed = if is_real {
                    Value::from_f64(0.0)
                } else {
                    match defaults.get(&k) {
                        Some(d) if d.width == w => d.clone(),
                        _ => Value::new(w),
                    }
                };
                (k, seed)
            })
            .collect()
    }

    /// Unpacked-struct TYPE of a collection property's element. Resolves the
    /// property's declared (element) DataType over the instance's inheritance
    /// chain, then reduces typedef aliases to its `Struct` if it is one.
    pub(super) fn collection_elem_struct(
        &self,
        handle: usize,
        prop: &str,
    ) -> Option<crate::ast::types::StructUnionType> {
        use crate::ast::types::DataType;
        let inst = self.heap.get(handle)?.as_ref()?;
        let mut cur = Some(inst.class_name.clone());
        let mut seen: HashSet<String> = HashSet::default();
        while let Some(cn) = cur {
            if !seen.insert(cn.clone()) {
                break;
            }
            let cd = self.module.classes.get(&cn)?;
            if let Some(dt) = cd.property_types.get(prop).cloned() {
                if let DataType::Struct(su) = self.resolve_dt(&dt) {
                    return Some(su.clone());
                }
                return None;
            }
            cur = cd.extends.clone();
        }
        None
    }

    /// `element` : resolved as above for a struct element in a class collection,
    /// return the unpacked struct's member width of `field`, if any.
    pub(super) fn coll_field_width(
        &self,
        su: &crate::ast::types::StructUnionType,
        field: &str,
    ) -> Option<u32> {
        su.members
            .iter()
            .find(|m| m.declarators.iter().any(|d| d.name.name == field))
            .map(|m| {
                resolve_type_width(
                    &m.data_type,
                    Some(&self.module.parameters),
                    Some(&self.module.typedefs),
                )
                .max(1)
            })
    }

    /// Spread an assignment pattern across the members of a struct-typed ELEMENT
    /// of a class collection (`m[k1][k2] = '{a,b}`), plus write a presence
    /// marker at the element key so `exists()/num()/...` see the slot. Returns
    /// whether it took care of the write.
    pub(super) fn assign_coll_struct_pattern(
        &mut self,
        lvalue: &Expression,
        items: &[AssignmentPatternItem],
    ) -> bool {
        let Some((key, su)) = self.coll_assoc_struct_elem(lvalue) else {
            return false;
        };
        if items.is_empty() {
            return false;
        }
        self.assign_pattern_into_struct(&key, &su, items);
        if self.get_signal_value_by_name(&key).is_none() {
            self.set_signal_value_by_name(&key, Value::zero(1));
        }
        true
    }
}
