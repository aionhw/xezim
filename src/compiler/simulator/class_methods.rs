//! Simulator class methods.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    pub(super) fn stmts_call_super_new(stmts: &[Statement]) -> bool {
        stmts.iter().any(Self::stmt_calls_super_new)
    }

    pub(super) fn stmt_calls_super_new(s: &Statement) -> bool {
        use crate::ast::stmt::StatementKind as SK;
        let expr_is_super_new = |e: &Expression| -> bool {
            let ExprKind::Call { func, .. } = &e.kind else {
                return false;
            };
            match &func.kind {
                // Flattened form: `super.new` as a single dotted Ident.
                ExprKind::Ident(h) => {
                    h.path.first().is_some_and(|s| s.name.name == "super")
                        && h.path.last().is_some_and(|s| s.name.name == "new")
                }
                // Common form: `super.new(...)` parses as a MemberAccess whose
                // base is the `super` keyword and member is `new`.
                ExprKind::MemberAccess { expr, member } => {
                    member.name == "new"
                        && matches!(&expr.kind, ExprKind::Ident(h)
                            if h.path.last().is_some_and(|s| s.name.name == "super"))
                }
                _ => false,
            }
        };
        match &s.kind {
            SK::Expr(e) => expr_is_super_new(e),
            SK::SeqBlock { stmts, .. } | SK::ParBlock { stmts, .. } => {
                Self::stmts_call_super_new(stmts)
            }
            SK::TimingControl { stmt, .. } => Self::stmt_calls_super_new(stmt),
            SK::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                Self::stmt_calls_super_new(then_stmt)
                    || else_stmt
                        .as_ref()
                        .is_some_and(|e| Self::stmt_calls_super_new(e))
            }
            _ => false,
        }
    }

    pub(super) fn super_call_parent(&self) -> Option<String> {
        let ctx = self
            .class_context_stack
            .last()
            .cloned()
            .flatten()
            .or_else(|| {
                self.this_stack.last().copied().flatten().and_then(|h| {
                    self.heap
                        .get(h)
                        .and_then(|o| o.as_ref())
                        .map(|i| i.class_name.clone())
                })
            })?;
        self.module
            .classes
            .get(&ctx)
            .and_then(|cd| cd.extends.clone())
    }

    /// IEEE 1800-2017 §8.15: run `method_name` starting the resolution walk
    /// at the parent of the calling context's class and execute it
    /// NON-virtually (`exec_method_in_class_hierarchy` runs the first body
    /// found walking UP the extends chain; it never re-dispatches through
    /// the object's dynamic class).
    pub(super) fn exec_super_method_call(
        &mut self,
        handle: usize,
        method_name: &str,
        args: &[Expression],
    ) -> Value {
        // §8.15: bind from the parent of the class lexically containing the
        // call (see super_call_parent), then execute non-virtually.
        if let Some(pname) = self.super_call_parent() {
            return self.exec_method_in_class_hierarchy(handle, &pname, method_name, args);
        }
        Value::zero(32)
    }

    /// §8.20: the class whose definition a NON-virtual call through a handle
    /// of declared type `decl_cls` must bind to — the first class from
    /// `decl_cls` upward that defines `m` with a real body. `None` when the
    /// method is virtual anywhere in the chain (an override of a virtual
    /// method is implicitly virtual), has no body in the chain (extern /
    /// pure — other machinery owns those), is a constructor, or the chain is
    /// not fully resolvable — all of which keep runtime dispatch.
    pub(super) fn nonvirtual_target_class(&self, decl_cls: &str, m: &str) -> Option<String> {
        if m == "new" {
            return None;
        }
        // A function of the class tables alone: memoized per (class, method).
        if let Some(hit) = self
            .nonvirtual_target_cache
            .borrow()
            .get(decl_cls)
            .and_then(|by_m| by_m.get(m))
        {
            return hit.clone();
        }
        let v = self.nonvirtual_target_class_walk(decl_cls, m);
        self.nonvirtual_target_cache
            .borrow_mut()
            .entry(decl_cls.to_string())
            .or_default()
            .insert(m.to_string(), v.clone());
        v
    }

    pub(super) fn nonvirtual_target_class_walk(&self, decl_cls: &str, m: &str) -> Option<String> {
        use crate::ast::decl::{ClassMethodKind, ClassQualifier};
        let mut target: Option<String> = None;
        let mut cur = Some(decl_cls.to_string());
        let mut guard = 0;
        while let Some(cname) = cur {
            guard += 1;
            if guard > 32 {
                return None;
            }
            let cd = self.module.classes.get(&cname)?;
            if let Some(method) = cd.methods.get(m) {
                if method.qualifiers.contains(&ClassQualifier::Virtual)
                    || method.qualifiers.contains(&ClassQualifier::Pure)
                {
                    return None; // virtual: runtime dispatch is correct
                }
                match &method.kind {
                    ClassMethodKind::Function(_) | ClassMethodKind::Task(_) => {
                        if target.is_none() {
                            target = Some(cd.name.clone());
                        }
                        // Keep walking: a virtual declaration higher up makes
                        // every override virtual.
                    }
                    _ => return None, // extern / pure prototype
                }
            }
            cur = cd
                .extends
                .clone()
                .map(|e: String| e.split('#').next().unwrap_or(e.as_str()).to_string());
        }
        target
    }

    pub(super) fn cached_class_method(
        &self,
        class_name: &str,
        method_name: &str,
    ) -> Option<Arc<crate::ast::decl::ClassMethod>> {
        // A specialization's class entry shares its origin's method bodies.
        let class_name = self.class_origin(class_name);
        if let Some(cached) = self
            .class_method_cache
            .borrow()
            .get(class_name)
            .and_then(|methods| methods.get(method_name))
        {
            return cached.clone();
        }
        let method = self
            .module
            .classes
            .get(class_name)
            .and_then(|class_def| class_def.methods.get(method_name))
            .filter(|method| {
                matches!(
                    method.kind,
                    crate::ast::decl::ClassMethodKind::Function(_)
                        | crate::ast::decl::ClassMethodKind::Task(_)
                )
            })
            .cloned()
            .map(Arc::new);
        self.class_method_cache
            .borrow_mut()
            .entry(class_name.to_string())
            .or_default()
            .insert(method_name.to_string(), method.clone());
        method
    }

    pub(super) fn class_member_names(&self) -> &ClassMemberNames {
        self.class_member_names_cell.get_or_init(|| {
            let mut statics: HashSet<String> = HashSet::default();
            let mut vif_props: HashSet<String> = HashSet::default();
            let mut methods: HashSet<String> = HashSet::default();
            let mut tasks: HashSet<String> = HashSet::default();
            let mut packed_vec_props: HashSet<String> = HashSet::default();
            let mut static_methods: HashSet<String> = HashSet::default();
            let mut params: HashSet<String> = HashSet::default();
            let mut assoc_props: HashSet<String> = HashSet::default();
            let mut string_props: HashSet<String> = HashSet::default();
            let mut static_or_param: HashSet<String> = HashSet::default();
            let mut constant_arrays: HashSet<String> = HashSet::default();
            let mut string_methods: HashSet<String> = HashSet::default();
            for cd in self.module.classes.values() {
                for (name, cm) in cd.methods.iter() {
                    if let crate::ast::decl::ClassMethodKind::Function(fd)
                    | crate::ast::decl::ClassMethodKind::Extern(fd)
                    | crate::ast::decl::ClassMethodKind::PureVirtual(fd) = &cm.kind
                    {
                        if Self::is_string_data_type(&fd.return_type) {
                            string_methods.insert(name.clone());
                        }
                    }
                }
                string_props.extend(cd.string_properties.iter().cloned());
                statics.extend(cd.static_properties.iter().cloned());
                vif_props.extend(cd.virtual_iface_properties.keys().cloned());
                methods.extend(cd.methods.keys().cloned());
                tasks.extend(
                    cd.methods
                        .iter()
                        .filter(|(_, m)| {
                            matches!(m.kind, crate::ast::decl::ClassMethodKind::Task(_))
                        })
                        .map(|(n, _)| n.clone()),
                );
                for (n, dt) in &cd.property_types {
                    if let DataType::IntegerVector { dimensions, .. }
                    | DataType::Implicit { dimensions, .. } = dt
                    {
                        if !dimensions.is_empty() {
                            packed_vec_props.insert(n.clone());
                        }
                    }
                }
                constant_arrays.extend(cd.array_properties.keys().filter(|name| {
                    cd.param_order.contains(*name)
                        || (cd.properties.get(*name).is_some_and(|s| s.is_const)
                            && !cd.property_order.contains(*name))
                }).cloned());
                constant_arrays.extend(cd.static_fixed_arrays.iter().map(|(name, ..)| name.clone()));
                static_methods.extend(cd.static_methods.iter().cloned());
                params.extend(cd.type_param_names.iter().cloned());
                params.extend(cd.param_order.iter().cloned());
                assoc_props.extend(cd.assoc_properties.keys().cloned());
                static_or_param.extend(cd.static_properties.iter().cloned());
                static_or_param.extend(cd.param_defaults.iter().map(|(n, _)| n.clone()));
                static_or_param.extend(cd.array_properties.keys().filter(|name| {
                    cd.properties.get(*name).is_some_and(|s| s.is_const)
                        && !cd.property_order.contains(*name)
                }).cloned());
            }
            let mut subroutine_suffixes: HashSet<String> = HashSet::default();
            for k in self.module.functions.keys().chain(self.module.tasks.keys()) {
                subroutine_suffixes.insert(k.clone());
                for (i, b) in k.bytes().enumerate() {
                    if b == b'.' {
                        subroutine_suffixes.insert(k[i + 1..].to_string());
                    }
                }
            }
            let alias_target = |dt: &DataType| match dt {
                DataType::Interface { .. }
                | DataType::IntegerAtom { signing: None, .. }
                | DataType::Real { .. }
                | DataType::Simple { .. } => true,
                DataType::TypeReference {
                    dimensions,
                    type_args,
                    ..
                } => dimensions.is_empty() && type_args.is_empty(),
                _ => false,
            };
            let mut alias_typedefs: HashSet<String> = self
                .module
                .typedef_types
                .iter()
                .filter(|(_, dt)| alias_target(dt))
                .map(|(n, _)| n.clone())
                .collect();
            for cd in self.module.classes.values() {
                alias_typedefs.extend(
                    cd.typedef_targets
                        .iter()
                        .filter(|(_, dt)| alias_target(dt))
                        .map(|(n, _)| n.clone()),
                );
            }
            // Typedefs (module or class scope) aliasing a CLASS-SCOPED type
            // (`typedef R::elem_t et_t;`) — see `scoped_typedef_concrete`.
            let scoped_target = |dt: &DataType| {
                matches!(dt, DataType::TypeReference { name, .. } if name.has_scope())
            };
            let mut scoped_typedefs: HashSet<String> = self
                .module
                .typedef_types
                .iter()
                .filter(|(_, dt)| scoped_target(dt))
                .map(|(n, _)| n.clone())
                .collect();
            for cd in self.module.classes.values() {
                scoped_typedefs.extend(
                    cd.typedef_targets
                        .iter()
                        .filter(|(_, dt)| scoped_target(dt))
                        .map(|(n, _)| n.clone()),
                );
            }
            // Names that can prefix `name::member` without being a class
            // key: class parameters and typedef names (module or class
            // scope) — see `class_scope_member_value`.
            let mut scope_prefixes: HashSet<String> = params.clone();
            scope_prefixes.extend(self.module.typedef_types.keys().cloned());
            for cd in self.module.classes.values() {
                scope_prefixes.extend(cd.typedef_targets.keys().cloned());
            }
            let mut ident_slow: HashSet<String> = HashSet::default();
            ident_slow.extend(statics.iter().cloned());
            ident_slow.extend(vif_props.iter().cloned());
            ident_slow.extend(self.struct_capable_names().iter().cloned());
            ident_slow.insert("UVM_ACTIVE".to_string());
            ident_slow.insert("UVM_PASSIVE".to_string());
            let mut param_first = [false; 256];
            for n in &params {
                if let Some(&b) = n.as_bytes().first() {
                    param_first[b as usize] = true;
                }
            }
            let mut static_first = [false; 256];
            for n in &statics {
                if let Some(&b) = n.as_bytes().first() { static_first[b as usize] = true; }
            }
            let mut vif_first = [false; 256];
            for n in &vif_props {
                if let Some(&b) = n.as_bytes().first() {
                    vif_first[b as usize] = true;
                }
            }
            ClassMemberNames {
                ident_slow,
                statics,
                static_first,
                vif_props,
                vif_first,
                methods,
                tasks,
                packed_vec_props,
                string_methods,
                static_methods,
                subroutine_suffixes,
                alias_typedefs,
                scoped_typedefs,
                scope_prefixes,
                param_first,
                params,
                assoc_props,
                string_props,
                static_or_param,
                constant_arrays,
            }
        })
    }

    /// The instantiation template of the class chain `chain` (leaf first).
    pub(super) fn inst_template(
        &self,
        chain: &[Arc<crate::compiler::elaborate::ElaboratedClass>],
    ) -> std::rc::Rc<InstTemplate> {
        let leaf = chain[0].name.as_str();
        if let Some(t) = self.inst_templates.borrow().get(leaf) {
            if t.chain.len() == chain.len()
                && t.chain.iter().zip(chain).all(|(a, b)| Arc::ptr_eq(a, b))
            {
                return t.clone();
            }
        }
        let mut leaf_declarer: HashMap<&str, &str> = HashMap::default();
        for cdef in chain {
            // leaf-first: the first declarer seen is the leaf-most.
            for p in cdef.properties.keys() {
                leaf_declarer
                    .entry(p.as_str())
                    .or_insert(cdef.name.as_str());
            }
        }
        let has_value_params = chain.iter().any(|c| !c.param_defaults.is_empty());
        let mut base_props: HashMap<String, Value> = HashMap::default();
        let mut shadowed_props: Vec<String> = Vec::new();
        // Parameter defaults are evaluated in a frame of the declaring
        // class's parameter names (§8.25). Only a descendant property with
        // one of those names can change that frame or overwrite its seed.
        let interleaved_seeds = has_value_params
            && chain.iter().enumerate().any(|(ci, cdef)| {
                cdef.param_defaults.iter().any(|(name, _)| {
                    chain[..ci]
                        .iter()
                        .any(|derived| derived.properties.contains_key(name))
                })
            });
        let mut property_seed_keys = interleaved_seeds.then(|| vec![Vec::new(); chain.len()]);
        for (ci, cdef) in chain.iter().enumerate().rev() {
            for (prop_name, prop_sig) in cdef.properties.iter() {
                if cdef.static_properties.contains(prop_name)
                    || cdef.assoc_properties.contains_key(prop_name)
                {
                    continue;
                }
                let redeclared = leaf_declarer
                    .get(prop_name.as_str())
                    .is_some_and(|l| *l != cdef.name.as_str());
                let key = if redeclared {
                    if !shadowed_props.contains(prop_name) {
                        shadowed_props.push(prop_name.clone());
                    }
                    format!("{}::{}", cdef.name, prop_name)
                } else {
                    prop_name.clone()
                };
                let v = if self.prop_seeds_x(cdef, prop_name) {
                    let mut x = Value::new(prop_sig.width.max(1));
                    x.is_signed = prop_sig.is_signed;
                    x
                } else {
                    prop_sig.value.clone()
                };
                if let Some(keys) = &mut property_seed_keys {
                    keys[ci].push(key.clone());
                }
                base_props.insert(key, v);
            }
        }
        let mut coll_candidates = Vec::new();
        for (ci, cdef) in chain.iter().enumerate() {
            for (prop, sig) in &cdef.properties {
                if cdef.queue_properties.contains_key(prop)
                    || cdef.assoc_properties.contains_key(prop)
                    || cdef.array_properties.contains_key(prop)
                    || cdef.array_nd_properties.contains_key(prop)
                    || cdef.static_collections.iter().any(|(n, ..)| n == prop)
                    // `prop_bound_collection` answers false without one.
                    || sig.type_name.is_none()
                {
                    continue;
                }
                coll_candidates.push((ci, prop.clone()));
            }
        }
        let mut struct_defaults = Vec::new();
        for cdef in chain.iter().rev() {
            for (prop, dt) in &cdef.property_types {
                if cdef.static_properties.contains(prop)
                    || cdef.property_inits.contains_key(prop)
                    || !cdef.properties.contains_key(prop)
                    || cdef.array_properties.contains_key(prop)
                    || cdef.queue_properties.contains_key(prop)
                    || cdef.assoc_properties.contains_key(prop)
                {
                    continue;
                }
                // Typedef chain by reference first: almost every property
                // is not a struct, and this runs once per class chain.
                if let DataType::Struct(su) = self.typedef_chain_end(dt) {
                    if !su.packed {
                        self.struct_member_defaults(prop, su, 0, &mut struct_defaults);
                    }
                }
            }
        }
        // §6.8 / §7.2: every member cell of an unpacked-struct property starts
        // at its type's default; an absent cell read back 0.
        let mut struct_cells = Vec::new();
        for cdef in chain.iter().rev() {
            for (prop, dt) in &cdef.property_types {
                if cdef.static_properties.contains(prop)
                    || cdef.property_inits.contains_key(prop)
                    || !cdef.properties.contains_key(prop)
                    || cdef.array_properties.contains_key(prop)
                    || cdef.array_nd_properties.contains_key(prop)
                    || cdef.array_of_coll_properties.contains_key(prop)
                    || cdef.queue_properties.contains_key(prop)
                    || cdef.assoc_properties.contains_key(prop)
                    || leaf_declarer
                        .get(prop.as_str())
                        .is_some_and(|l| *l != cdef.name.as_str())
                {
                    continue;
                }
                let dt = self.class_prop_elem_type(cdef, prop).unwrap_or(dt);
                if let DataType::Struct(su) = self.typedef_chain_end(dt) {
                    self.struct_leaf_plain_defaults(prop, su, 0, &mut struct_cells);
                }
            }
        }
        let t = std::rc::Rc::new(InstTemplate {
            chain: chain.to_vec(),
            base_props,
            property_seed_keys,
            shadowed_props,
            has_value_params,
            coll_candidates,
            struct_defaults,
            struct_cells,
        });
        self.inst_templates
            .borrow_mut()
            .insert(leaf.to_string(), t.clone());
        t
    }

    /// `dt` followed through bare typedef names in `typedef_types`, by
    /// reference (no resolved-type clone). Stops at the first non-alias.
    pub(super) fn typedef_chain_end<'a>(&'a self, dt: &'a DataType) -> &'a DataType {
        let mut cur = dt;
        for _ in 0..16 {
            match cur {
                DataType::TypeReference { name, .. } if name.scopes.is_empty() => {
                    match self.module.typedef_types.get(&name.name.name) {
                        Some(next) if !std::ptr::eq(next, cur) => cur = next,
                        _ => break,
                    }
                }
                _ => break,
            }
        }
        cur
    }

    /// The §7.2.2 member defaults of an unpacked struct rooted at `prefix`
    /// (nested unpacked structs recurse), as `(cell key, default, type)`.
    pub(super) fn struct_member_defaults(
        &self,
        prefix: &str,
        su: &crate::ast::types::StructUnionType,
        depth: u32,
        out: &mut Vec<(String, Expression, DataType)>,
    ) {
        if depth > 8 || !Self::spreads_member_wise(su) {
            return;
        }
        for m in &su.members {
            let nested = match self.resolve_dt(&m.data_type) {
                DataType::Struct(inner) => Some(inner),
                _ => None,
            };
            for md in &m.declarators {
                if !md.dimensions.is_empty() {
                    continue;
                }
                let key = format!("{}.{}", prefix, md.name.name);
                match (&nested, &md.init) {
                    (Some(inner), _) => self.struct_member_defaults(&key, inner, depth + 1, out),
                    (None, Some(init)) => out.push((key, init.clone(), m.data_type.clone())),
                    (None, None) => {}
                }
            }
        }
    }

    /// §6.8 Table 6-7, §8.7: whether scalar property `prop` of `cdef` must be
    /// seeded with x. Elaboration seeds a property of a NAMED type (`l4_t m;`,
    /// an enum, a packed struct) with 0, since it cannot tell a typedef from a
    /// class handle; the typedef table can. Collections and initialized
    /// properties keep their elaborated seed.
    pub(super) fn prop_seeds_x(
        &self,
        cdef: &crate::compiler::elaborate::ElaboratedClass,
        prop: &str,
    ) -> bool {
        let Some(sig) = cdef.properties.get(prop) else {
            return false;
        };
        sig.type_name.is_some()
            && !sig.is_real
            && !cdef.property_inits.contains_key(prop)
            && !cdef.queue_properties.contains_key(prop)
            && !cdef.array_properties.contains_key(prop)
            && !cdef.array_nd_properties.contains_key(prop)
            && !cdef.array_of_coll_properties.contains_key(prop)
            && !cdef.string_properties.contains(prop)
            && !cdef.virtual_iface_properties.contains_key(prop)
            && self
                .class_prop_elem_type(cdef, prop)
                .is_some_and(|dt| self.type_starts_x(dt))
    }

    /// §6.8, §7.4: the initial value of each element of class array property
    /// `prop` (x for a 4-state element type, else 0), plus the element's
    /// unpacked struct type when its elements are stored member-wise.
    pub(super) fn class_array_elem_fill(
        &self,
        cdef: &crate::compiler::elaborate::ElaboratedClass,
        prop: &str,
        width: u32,
    ) -> (Value, Option<crate::ast::types::StructUnionType>) {
        let Some(dt) = self.class_prop_elem_type(cdef, prop) else {
            return (Value::zero(width), None);
        };
        let su = match self.resolve_dt_ref(dt) {
            DataType::Struct(su) if Self::spreads_member_wise(su) => Some(su.clone()),
            _ => None,
        };
        let fill = if su.is_none() && self.type_starts_x(dt) {
            Value::new(width)
        } else {
            Value::zero(width)
        };
        (fill, su)
    }

    pub(super) fn class_bound_array_fill(
        &self,
        cdef: &crate::compiler::elaborate::ElaboratedClass,
        handle: usize,
        prop: &str,
        width: u32,
    ) -> Value {
        if let Some(concrete) = self.class_prop_type_name(handle, prop)
            && let Some(starts_x) = self.concrete_type_starts_x(&concrete)
        {
            return if starts_x {
                Value::new(width)
            } else {
                Value::zero(width)
            };
        }
        self.class_array_elem_fill(cdef, prop, width).0
    }

    /// Seed the member leaves of the unpacked-struct element `elem` with their
    /// §6.8 defaults (see `struct_leaf_plain_defaults`).
    pub(super) fn seed_struct_elem_cells(
        &mut self,
        elem: &str,
        su: &crate::ast::types::StructUnionType,
    ) {
        let mut cells = Vec::new();
        self.struct_leaf_plain_defaults(elem, su, 0, &mut cells);
        for (k, v) in cells {
            if !v.is_real {
                self.widths.insert(k.clone(), v.width);
            }
            self.signals.insert(k, v);
        }
        let mut defaults = Vec::new();
        self.struct_member_defaults(elem, su, 0, &mut defaults);
        for (key, init, dt) in defaults {
            self.assign_pattern_or_leaf(&key, &dt, &init);
        }
    }

    pub(super) fn seed_class_struct_cells(
        &mut self,
        handle: usize,
        prop: &str,
        su: &crate::ast::types::StructUnionType,
    ) {
        let mut cells = Vec::new();
        self.struct_leaf_plain_defaults(prop, su, 0, &mut cells);
        if let Some(Some(inst)) = self.heap.get_mut(handle) {
            for (key, value) in cells {
                inst.properties.insert(key, value);
            }
        }
    }

    /// §6.8, §7.5.1: whether a new element of the collection stored at
    /// `name` starts at x. A class member (`<handle>#da`) takes its type from
    /// the class; anything else from its declaration.
    pub(super) fn coll_elem_starts_x(&self, name: &str) -> bool {
        if let Some((h, member)) = name.split_once('#')
            && let Ok(handle) = h.parse::<usize>()
            && let Some(concrete) =
                self.class_prop_type_name(handle, member.split('[').next().unwrap_or(member))
            && let Some(starts_x) = self.concrete_type_starts_x(&concrete)
        {
            return starts_x;
        }
        if let Some(dt) = self.class_member_coll_type(name) {
            return self.type_starts_x(dt);
        }
        self.p_elem_type_ref(name)
            .is_some_and(|dt| self.type_starts_x(&dt))
    }

    /// The declared element type of the class-member collection stored at
    /// `<handle>#<member>` (or an element of one, `<handle>#<member>[i]`),
    /// from the object's class chain. `None` for any other name.
    pub(super) fn class_member_coll_type(&self, name: &str) -> Option<&DataType> {
        // A static member's store `Cls::member` (`Cls#<spec>::member`).
        // A member of an element (`q[0].m`) is no collection of this
        // property's element type.
        if let Some((cls, member)) = name.split_once("::") {
            if member.contains('.') {
                return None;
            }
            let cls = cls.split('#').next()?;
            let member = member.split('[').next()?;
            let cd = self.module.classes.get(cls)?;
            return self.class_prop_elem_type(cd, member);
        }
        let (h, member) = name.split_once('#')?;
        let handle: usize = h.parse().ok()?;
        if member.contains('.') {
            return None;
        }
        let member = member.split('[').next()?;
        if member.is_empty() {
            return None;
        }
        let mut cur = self
            .heap
            .get(handle)?
            .as_ref()
            .map(|i| i.class_name.as_str());
        let mut hops = 0;
        while let Some(cn) = cur {
            let cd = self.module.classes.get(cn)?;
            if cd.property_types.contains_key(member) {
                if cd
                    .properties
                    .get(member)
                    .and_then(|sig| sig.type_name.as_ref())
                    .is_some_and(|name| cd.type_param_names.contains(name))
                    && let Some(concrete) = self.class_prop_type_name(handle, member)
                    && let Some(dt) = self.module.typedef_types.get(&concrete)
                {
                    return Some(dt);
                }
                return self.class_prop_elem_type(cd, member);
            }
            hops += 1;
            if hops > 64 {
                break;
            }
            cur = cd.extends.as_deref();
        }
        None
    }

    /// §6.8 Table 6-7: whether a variable of `dt` held as ONE value starts
    /// at x — a 4-state integral type (`logic`, `reg`, `integer`, `time`, an
    /// enum over one of them, a packed struct or union with a 4-state member)
    /// — rather than 0. Strings, reals, handles, unresolved type names and
    /// unpacked structs (stored as member leaves) answer false.
    pub(super) fn type_starts_x(&self, dt: &DataType) -> bool {
        let r = self.resolve_dt_ref(dt);
        match r {
            DataType::IntegerVector { .. }
            | DataType::IntegerAtom { .. }
            | DataType::Implicit { .. }
            | DataType::Enum(_) => {
                !super::elaborate::is_type_two_state_resolved(r, &self.module.typedef_types)
            }
            DataType::Struct(su) if !Self::spreads_member_wise(su) => {
                !super::elaborate::is_type_two_state_resolved(r, &self.module.typedef_types)
            }
            _ => false,
        }
    }

    /// The declared (element) type of property `prop` of class `cd`, with a
    /// class-local typedef (`typedef logic [3:0] n_t; n_t m[2];`) followed
    /// into the class's own typedef table.
    pub(super) fn class_prop_elem_type<'a>(
        &'a self,
        cd: &'a crate::compiler::elaborate::ElaboratedClass,
        prop: &str,
    ) -> Option<&'a DataType> {
        self.class_prop_decl_type(cd, prop)
    }

    /// §6.8 / §8.25: an integral type parameter takes its bound type's default.
    /// Non-integral and unresolved bindings keep their existing storage paths.
    pub(super) fn concrete_type_starts_x(&self, name: &str) -> Option<bool> {
        match name {
            "logic" | "reg" | "integer" | "time" => Some(true),
            "bit" | "byte" | "shortint" | "int" | "longint" => Some(false),
            _ if vector_fragment_width(name).is_some() => Some(!name.starts_with("bit")),
            _ => self
                .module
                .typedef_types
                .get(name)
                .and_then(|dt| match self.resolve_dt_ref(dt) {
                    DataType::IntegerVector { .. }
                    | DataType::IntegerAtom { .. }
                    | DataType::Implicit { .. }
                    | DataType::Enum(_) => Some(self.type_starts_x(dt)),
                    DataType::Struct(su) if !Self::spreads_member_wise(su) => {
                        Some(self.type_starts_x(dt))
                    }
                    _ => None,
                }),
        }
    }

    /// §6.8 / §7.2: the initial cells of an unpacked struct stored member-wise
    /// under `prefix` — `(prefix.member[...], default)` per leaf, x for a
    /// 4-state member and 0 for a 2-state one (0.0 for a real; a
    /// non-integral member gets no cell). Same keys as `unpacked_struct_leaves`; member initializers are
    /// not evaluated here (callers apply them on top).
    pub(super) fn struct_leaf_plain_defaults(
        &self,
        prefix: &str,
        su: &crate::ast::types::StructUnionType,
        depth: u32,
        out: &mut Vec<(String, Value)>,
    ) {
        if depth > 8 || !Self::spreads_member_wise(su) {
            return;
        }
        for m in &su.members {
            let nested = match self.resolve_dt_ref(&m.data_type) {
                DataType::Struct(inner) if Self::spreads_member_wise(inner) => Some(inner),
                _ => None,
            };
            let dv = if nested.is_some() {
                Value::zero(0)
            } else if super::elaborate::is_type_real(self.resolve_dt_ref(&m.data_type)) {
                Value::from_f64(0.0)
            } else if !matches!(
                self.resolve_dt_ref(&m.data_type),
                DataType::IntegerVector { .. }
                    | DataType::IntegerAtom { .. }
                    | DataType::Implicit { .. }
                    | DataType::Enum(_)
                    | DataType::Struct(_)
            ) {
                // A string, handle or other non-integral member keeps the
                // storage paths' own default.
                continue;
            } else {
                let mw = super::elaborate::resolve_type_width(
                    &m.data_type,
                    Some(&self.module.parameters),
                    Some(&self.module.typedefs),
                )
                .max(1);
                let mut v = if self.type_starts_x(&m.data_type) {
                    Value::new(mw)
                } else {
                    Value::zero(mw)
                };
                v.is_signed = super::elaborate::is_type_signed_resolved(
                    &m.data_type,
                    &self.module.typedef_types,
                );
                v
            };
            for md in &m.declarators {
                let mkey = format!("{}.{}", prefix, md.name.name);
                let keys: Vec<String> = match self.member_elem_indices(&md.dimensions) {
                    Some(tups) => tups
                        .iter()
                        .map(|t| format!("{}{}", mkey, Self::index_suffix(t)))
                        .collect(),
                    None if md.dimensions.is_empty() => vec![mkey],
                    // A dynamic / queue / associative member has no cells.
                    None => continue,
                };
                for k in keys {
                    match nested {
                        Some(inner) => self.struct_leaf_plain_defaults(&k, inner, depth + 1, out),
                        None => out.push((k, dv.clone())),
                    }
                }
            }
        }
    }

    /// The call plan of `method` (see `MethodCallPlan`).
    pub(super) fn method_call_plan(
        &self,
        method: &Arc<crate::ast::decl::ClassMethod>,
    ) -> std::rc::Rc<MethodCallPlan> {
        let key = Arc::as_ptr(method) as usize;
        if let Some(p) = self.method_plans.borrow().get(&key) {
            return p.clone();
        }
        let plan = std::rc::Rc::new(self.build_method_call_plan(method));
        self.method_plans.borrow_mut().insert(key, plan.clone());
        plan
    }

    pub(super) fn build_method_call_plan(
        &self,
        method: &Arc<crate::ast::decl::ClassMethod>,
    ) -> MethodCallPlan {
        use crate::ast::decl::ClassMethodKind;
        let ports = match &method.kind {
            ClassMethodKind::Function(f) => &f.ports[..],
            ClassMethodKind::Task(t) => &t.ports[..],
            _ => &[],
        };
        let ports: Vec<PortPlan> = ports
            .iter()
            .map(|port| {
                let class_ref = self.port_class_ref(port);
                let dt = &port.data_type;
                let fixed_width = (matches!(dt, DataType::IntegerAtom { .. })
                    || Self::packed_dims_are_constant(dt))
                .then(|| {
                    super::elaborate::resolve_type_width(
                        dt,
                        Some(&self.module.parameters),
                        Some(&self.module.typedefs),
                    )
                });
                let never_vif = match dt {
                    DataType::Interface { .. } => false,
                    DataType::TypeReference { .. } => self.plain_class_ref(dt).is_some(),
                    _ => true,
                };
                PortPlan {
                    plain_scalar: Self::port_is_plain_scalar(port),
                    class_ref,
                    fixed_width,
                    signed: super::elaborate::is_type_signed(dt),
                    two_state: port.dimensions.is_empty()
                        && matches!(
                            dt,
                            DataType::IntegerAtom { .. }
                                | DataType::IntegerVector { .. }
                                | DataType::Implicit { .. }
                        )
                        && super::elaborate::is_type_two_state(dt),
                    never_vif,
                }
            })
            .collect();
        let ret = match &method.kind {
            ClassMethodKind::Function(f) => {
                let rt = &f.return_type;
                let is_string = match rt {
                    DataType::TypeReference { .. } => {
                        self.plain_class_ref(rt).is_some().then_some(false)
                    }
                    _ => Some(Self::is_string_data_type(rt)),
                };
                let dims = match rt {
                    DataType::IntegerVector { dimensions, .. } => Some(dimensions),
                    DataType::Implicit { dimensions, .. } => Some(dimensions),
                    _ => None,
                };
                let dyn_ret = dims
                    .filter(|ds| {
                        ds.iter().any(|d| {
                            matches!(d, crate::ast::types::PackedDimension::Range { left, right, .. }
                                if crate::elaborate::const_eval_i64_with_params(left, None).is_none()
                                    || crate::elaborate::const_eval_i64_with_params(right, None).is_none())
                        })
                    })
                    .map(|_| rt.clone());
                let class = match rt {
                    DataType::TypeReference { name, .. }
                        if self.module.classes.contains_key(&name.name.name) =>
                    {
                        Some(name.name.name.clone())
                    }
                    _ => None,
                };
                Some(RetPlan {
                    is_string,
                    decl_string: Self::is_string_data_type(rt),
                    dyn_ret,
                    unpacked: self.unpacked_struct_of(rt),
                    width: super::elaborate::resolve_type_width(
                        rt,
                        Some(&self.module.parameters),
                        Some(&self.module.typedefs),
                    ),
                    signed: super::elaborate::is_type_signed(rt),
                    real: super::elaborate::is_type_real(rt),
                    two_state: super::elaborate::is_type_two_state(rt),
                    class,
                    collection: self.fn_returns_collection(rt),
                })
            }
            _ => None,
        };
        let all_plain = ports
            .iter()
            .all(|p: &PortPlan| p.plain_scalar || p.class_ref.is_some());
        let ret_name = match &method.kind {
            ClassMethodKind::Function(f) => Some(f.name.name.name.as_str()),
            _ => None,
        };
        let port_decls = match &method.kind {
            ClassMethodKind::Function(f) => &f.ports[..],
            ClassMethodKind::Task(t) => &t.ports[..],
            _ => &[],
        };
        let mut meta_ks = Vec::new();
        let mut meta_bloom = 0u64;
        let mut meta_bits = Vec::new();
        for k in 0..=port_decls.len() {
            if let Some(n) = Self::formal_meta_name(port_decls, ret_name, k) {
                meta_ks.push(k);
                meta_bloom |= meta_key_bits(n);
                meta_bits.push(meta_key_bits(n));
            }
        }
        MethodCallPlan {
            _method: method.clone(),
            type_gen: self.type_tables_gen,
            ports,
            ret,
            all_plain,
            meta_ks,
            meta_bloom,
            meta_bits,
            vif_absent: std::cell::Cell::new((usize::MAX, false)),
        }
    }

    /// §8.25: seed `current_spec` from a receiver instance (and, for a
    /// method declared in a parameterized ancestor, the receiver's derived
    /// ancestor spec) so the callee's BARE static member accesses key the
    /// same per-specialization cell regardless of dispatch route. Extracted
    /// from `exec_method_in_class_hierarchy`, which seeds for interpreter
    /// calls; the VM's direct dispatch (`vm_try_direct_call`) skips that
    /// prologue and must seed identically or a compiled callee of
    /// `C#(spec)` resolves bare statics to the shared `C::member` cell
    /// while every interpreter route uses `C#spec::member` — storage
    /// silently splits. Returns the saved spec for restore.
    pub(super) fn seed_callee_spec_from_instance(
        &mut self,
        handle: usize,
        def_class: &str,
    ) -> Option<(String, String)> {
        let saved = self.clone_spec_pooled();
        if let Some(inst) = self.heap.get(handle).and_then(|o| o.as_ref()) {
            let cn = &inst.class_name;
            let bindings = &inst.type_bindings;
            // Override the active spec with THIS instance's own
            // specialization when it differs from the active
            // spec's class. Prefer the instance's captured full
            // specialization (`spec`), which carries BOTH type
            // and value params as a complete sig (unlike the
            // type_bindings-only rebuild below). The full `spec`
            // is what restores the specialization a
            // typedef-specialization singleton was constructed
            // under, so a later virtual call can answer
            // value-param lookups (the factory
            // get_type_name chain). The type_bindings rebuild is
            // the legacy fallback: an instance method of
            // `resource#(int)` must key resource's
            // per-spec cell even when called while an UNRELATED
            // spec is active (e.g. the enclosing
            // `config_db#(int)`, whose base `config_db`
            // wouldn't match `resource` in static_prop_key
            // and would fall back to the shared unspec'd cell →
            // the get_type/get_type_handle mismatch).
            let differs = match self.current_spec.as_ref() {
                None => true,
                // Switch when the base class differs OR the
                // instance's own specialization (same base,
                // different sig) differs. The latter is the
                // callback typewide-recursion case:
                // `base_comp.m_add_tw_cbs` calls
                // `cb_pair.m_add_tw_cbs(cb)` where cb_pair is a
                // DIFFERENT specialization's instance (e.g.
                // `callbacks#(b_comp)`). Without restoring
                // the instance's spec, the recursed method's
                // `m_t_inst.m_tw_cb_q` static access keys off
                // the caller's (base_comp) cell, so typewide
                // callbacks never propagate to derived types.
                Some((b, s)) => {
                    b != self.class_origin(cn)
                        || inst
                            .spec
                            .as_ref()
                            .is_some_and(|(ib, is)| ib != b || is != s)
                }
            };
            if differs {
                if let Some(src) = inst.spec.as_ref() {
                    // Copied into the current buffers when there are
                    // any (same value as a fresh clone).
                    match self.current_spec.as_mut() {
                        Some(dst) => {
                            dst.0.clone_from(&src.0);
                            dst.1.clone_from(&src.1);
                        }
                        None => self.current_spec = Some(src.clone()),
                    }
                } else if let Some(cd) = self.module.classes.get(cn) {
                    let param_names: &[String] = if cd.param_order.is_empty() {
                        &cd.type_param_names
                    } else {
                        &cd.param_order
                    };
                    if !param_names.is_empty() {
                        let sig_frags: Vec<String> = param_names
                            .iter()
                            .filter_map(|p| {
                                if let Some(b) = bindings.get(p) {
                                    Some(b.clone())
                                } else if let Some(v) = inst.properties.get(p) {
                                    Some(v.to_string())
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if sig_frags.len() == param_names.len() {
                            self.current_spec = Some((cn.clone(), sig_frags.join(",")));
                        }
                    }
                }
            }
        }
        // §8.25 (generic receiver specialization): when a method is
        // reached on a CONCRETE receiver (static, `ext1::ID()`; or
        // instance, `x3.get_type_handle()`) but is DECLARED in a
        // PARAMETERIZED ancestor (`uvm_tlm_extension#(ext1)`), the
        // receiver's `#(...)` specialization is not captured anywhere
        // once the body runs a BARE static member like `m_singleton`.
        // Seed current_spec from the concrete receiver's extends chain
        // so the body's inner static access keys per specialization.
        if let Some(spec) = self.inherited_method_spec(handle, def_class) {
            self.current_spec = Some(spec);
        }
        saved
    }

    /// A copy of `current_spec` in recycled buffers (see `recycle_spec`):
    /// the save half of a save/restore around a call, without allocating.
    pub(super) fn clone_spec_pooled(&mut self) -> Option<(String, String)> {
        let (b, s) = self.current_spec.as_ref()?;
        let mut buf = self.spec_buf_pool.pop().unwrap_or_default();
        buf.0.clone_from(b);
        buf.1.clone_from(s);
        Some(buf)
    }

    /// Put `saved` back as `current_spec`, keeping the displaced value's
    /// buffers for the next `clone_spec_pooled`.
    pub(super) fn restore_spec_pooled(&mut self, saved: Option<(String, String)>) {
        if let Some(old) = std::mem::replace(&mut self.current_spec, saved) {
            if self.spec_buf_pool.len() < 64 {
                self.spec_buf_pool.push(old);
            }
        }
    }

    /// The class on `start_class`'s chain that defines function/task
    /// `method_name`, with the method; memoized per (start class, method).
    pub(super) fn method_defining_class(
        &self,
        start_class: &str,
        method_name: &str,
    ) -> Option<MethodDef> {
        if let Some(hit) = self
            .method_def_cache
            .borrow()
            .get(start_class)
            .and_then(|m| m.get(method_name))
        {
            return hit.clone();
        }
        let mut found = None;
        let mut probe: Option<&str> = Some(start_class);
        while let Some(cn) = probe {
            if let Some(m) = self.cached_class_method(cn, method_name) {
                found = Some(MethodDef {
                    class: std::rc::Rc::new(cn.to_string()),
                    method: m,
                    string_props: self.class_string_props_of(cn),
                    plan: std::rc::Rc::new(std::cell::RefCell::new(None)),
                });
                break;
            }
            probe = self
                .module
                .classes
                .get(cn)
                .and_then(|class_def| class_def.extends.as_deref());
        }
        self.method_def_cache
            .borrow_mut()
            .entry(start_class.to_string())
            .or_default()
            .insert(method_name.to_string(), found.clone());
        found
    }

    /// The string properties of `class_name` and its ancestors.
    pub(super) fn class_string_props_of(&self, class_name: &str) -> std::rc::Rc<Vec<String>> {
        if let Some(hit) = self.class_string_props.borrow().get(class_name) {
            return hit.clone();
        }
        let mut props: Vec<String> = Vec::new();
        let mut cur: Option<&str> = Some(class_name);
        while let Some(cn) = cur {
            let Some(cd) = self.module.classes.get(cn) else {
                break;
            };
            props.extend(cd.string_properties.iter().cloned());
            cur = cd.extends.as_deref();
        }
        let props = std::rc::Rc::new(props);
        self.class_string_props
            .borrow_mut()
            .insert(class_name.to_string(), props.clone());
        props
    }

    /// Whether `class_name` or an ancestor declares a property (of any
    /// kind: scalar, queue, associative, static, ...) named `prop`.
    ///
    /// Scopes the flat, name-global `string_signals` set inside
    /// `expr_is_string_valued`: a bare property reference in a class
    /// method shadows same-named module-scope signals (§6.21), and the
    /// flat set cannot distinguish specializations of a parameterized
    /// class — a string-typed `T val` formal of `uvm_resource_db#(string)`
    /// poisons the bare name for `uvm_resource#(<enum>)`'s `val` property,
    /// which routed an enum `val == t` through the TEXT comparison and
    /// made `X == 0` compare equal (skipping `uvm_resource#(T)::write`'s
    /// `if (val == t) return;` early-out, so the stored value stayed X).
    pub(super) fn class_declares_property(&self, class_name: &str, prop: &str) -> bool {
        let mut cur: Option<&str> = Some(class_name);
        while let Some(cn) = cur {
            let Some(cd) = self.module.classes.get(cn) else {
                break;
            };
            if cd.properties.contains_key(prop)
                || cd.queue_properties.contains_key(prop)
                || cd.assoc_properties.contains_key(prop)
                || cd.array_properties.contains_key(prop)
                || cd.array_nd_properties.contains_key(prop)
                || cd.static_properties.contains(prop)
            {
                return true;
            }
            cur = cd.extends.as_deref();
        }
        false
    }

    pub(super) fn exec_method_in_class_hierarchy(
        &mut self,
        handle: usize,
        start_class: &str,
        method_name: &str,
        args: &[Expression],
    ) -> Value {
        use crate::ast::decl::ClassMethodKind;
        if method_name == "new"
            && let Some(value) = self.try_implicit_class_constructor(handle, start_class, args)
        {
            return value;
        }
        // Re-entrant registration guard: a factory `register(obj)` whose
        // body calls `obj.get_type_name()` can trigger a parameterized class
        // specialization's lazy static init, which re-enters
        // `factory.register(obj)` for the SAME object. Skip the re-entrant
        // call so the outer one performs the single registration (else the
        // outer call resumes to find the type already stored -> UVM TPRGED).
        // `register`'s argument is a handle; `insert` returns false when the
        // handle is already mid-registration.
        let reg_guard_obj: Option<usize> = if method_name == "register" {
            args.first()
                .and_then(|a| self.eval_expr(a).to_u64())
                .map(|h| h as usize)
                .filter(|&h| h != 0)
        } else {
            None
        };
        if let Some(h) = reg_guard_obj {
            if !self.factory_reg_in_progress.insert(h) {
                return Value::zero(32);
            }
        }
        // Find the DEFINING class by borrow before doing any work: the old
        // form allocated `start_class.to_string()` plus one `extends.clone()`
        // per ancestor level on every method call, and UVM dispatch is the
        // hottest path in a testbench run (4.5% of the run's memcmp sat under
        // this function). The search is pure lookup, so it can hold a borrow;
        // only the class that actually defines the method is materialized.
        let found = self.method_defining_class(start_class, method_name);
        let mut found_method = found.as_ref().map(|d| d.method.clone());
        let found_plan = found.as_ref().map(|d| d.plan.clone());
        let found_string_props = found.as_ref().map(|d| d.string_props.clone());
        let mut cur_class: Option<std::rc::Rc<String>> = found.map(|d| d.class);
        while let Some(cname_rc) = cur_class {
            let cname: &String = &cname_rc;
            let method_opt = found_method.take();
            cur_class = None;
            if let Some(method) = method_opt {
                let (ports, body) = match &method.kind {
                    ClassMethodKind::Function(f) => (&f.ports, &f.items),
                    ClassMethodKind::Task(t) => (&t.ports, &t.items),
                    _ => unreachable!("non-Function/Task methods filtered out above"),
                };
                // The plan rides on the memoized definition; it is (re)built
                // when missing or when a run-time `typedef` moved the type
                // tables since.
                let cached_plan = found_plan
                    .as_ref()
                    .and_then(|c| c.borrow().clone())
                    .filter(|p| p.type_gen == self.type_tables_gen);
                let plan = match cached_plan {
                    Some(p) => p,
                    None => {
                        self.method_plans
                            .borrow_mut()
                            .remove(&(Arc::as_ptr(&method) as usize));
                        let p = self.method_call_plan(&method);
                        if let Some(c) = found_plan.as_ref() {
                            *c.borrow_mut() = Some(p.clone());
                        }
                        p
                    }
                };
                let mut locals: HashMap<String, Value> = self.take_pooled_frame();
                // The callee frame's type overlay, pushed together with `locals`.
                let mut frame_types = self.take_pooled_types();
                // §13.5.3: reorder named args into formal order and fill any
                // omitted (`.name()` / positional `,,`) slot from the formal's
                // default before positional binding below.
                let normalized = Self::normalize_call_args(ports, args);
                let supplied = normalized
                    .as_ref()
                    .map(|_| Self::supplied_argument_mask(ports, args));
                let args: &[Expression] = normalized.as_deref().unwrap_or(args);
                let arg_supplied = |i: usize| {
                    i < args.len()
                        && supplied
                            .as_ref()
                            .is_none_or(|mask| mask.get(i).copied().unwrap_or(false))
                };
                // A function may set its result via the implicit
                // return variable named after the function (`f = ...`)
                // instead of an explicit `return`.
                let fn_ret_name: Option<&str> = match &method.kind {
                    ClassMethodKind::Function(f) => Some(f.name.name.name.as_str()),
                    _ => None,
                };
                // Saved by position (see `formal_meta_name`) into a pooled
                // vector: no name copies and no allocation per call.
                // The formals' and return variable's metadata are saved when
                // first written (see `MetaFrame`).
                let meta_open = !plan.meta_ks.is_empty();
                if meta_open {
                    let parts = self.formal_parts_pool.pop().unwrap_or_default();
                    let cum_bloom =
                        plan.meta_bloom | self.meta_frames.last().map_or(0, |f| f.cum_bloom);
                    self.meta_frames.push(MetaFrame {
                        plan: plan.clone(),
                        saved: 0,
                        full: false,
                        parts,
                        cum_bloom,
                        unsaved_bloom: plan.meta_bloom,
                    });
                    self.meta_pending += 1;
                }
                let ret_is_string = match (&method.kind, plan.ret.as_ref()) {
                    (ClassMethodKind::Function(_), Some(r))
                        if plan.type_gen == self.type_tables_gen =>
                    {
                        match r.is_string {
                            Some(s) => s,
                            None => match &method.kind {
                                ClassMethodKind::Function(f) => {
                                    self.method_return_is_string(&f.return_type)
                                }
                                _ => false,
                            },
                        }
                    }
                    (ClassMethodKind::Function(f), _) => {
                        self.method_return_is_string(&f.return_type)
                    }
                    _ => false,
                };
                // A packed return type whose range references a CLASS
                // parameter (`function bit [W-1:0] mk();`) can only be
                // sized per instance — capture it for the clamp at the
                // return point below. Literal ranges resolve here (params
                // = None succeeds) and stay un-clamped as before.
                let dyn_ret_type: Option<DataType> =
                    plan.ret.as_ref().and_then(|r| r.dyn_ret.clone());
                self.push_queue_frame();
                let formal_dyn = self.begin_formal_dyn();
                // `output`/`inout`/`ref` formals copy back to the caller's
                // actual on return (e.g. `randomize_instr(output riscv_instr
                // instr, …)` writing the picked instruction to `instr_list[i]`).
                let mut output_bindings: Vec<(String, Expression)> = Vec::new();
                // §25.9 __vif_local__: record vif actuals under formal names.
                // A plain scalar or class formal cannot be bound to an
                // interface; its actual is not resolved (a stale key under its
                // name is still cleared).
                let plan_fresh = plan.type_gen == self.type_tables_gen;
                // All formals plain and none ever keyed a `__vif_local__`:
                // `vif_formals_enter` would only move the store generation
                // (once per formal, i.e. once) and save nothing.
                let vif_len0 = self.vif_local_names.len();
                let vif_fast = plan_fresh && plan.all_plain && {
                    let (len, absent) = plan.vif_absent.get();
                    if len == vif_len0 {
                        absent
                    } else {
                        let absent = ports
                            .iter()
                            .all(|p| !self.vif_local_names.contains(&p.name.name));
                        plan.vif_absent.set((vif_len0, absent));
                        absent
                    }
                };
                let vif_formals: Vec<(&str, Option<&Expression>)> = if vif_fast {
                    Vec::new()
                } else {
                    ports
                        .iter()
                        .enumerate()
                        .map(|(i, port)| {
                            let pp = &plan.ports[i];
                            let plain = pp.plain_scalar
                                || if plan_fresh {
                                    pp.class_ref.is_some()
                                } else {
                                    self.port_class_ref(port).is_some()
                                };
                            (
                                port.name.name.as_str(),
                                args.get(i).filter(|_| arg_supplied(i)).filter(|_| !plain),
                            )
                        })
                        .collect()
                };
                let vif_saved = if vif_fast {
                    if !ports.is_empty() {
                        bump_store_gen();
                    }
                    Vec::new()
                } else {
                    self.vif_formals_enter(&vif_formals)
                };
                let mut queue_writebacks: Vec<(String, String)> = Vec::new();
                let mut array_writebacks: Vec<(String, String, i64, i64)> = Vec::new();
                let mut array_inputs: Vec<(String, String, i64, i64)> = Vec::new();
                let mut default_out_bindings: Vec<(String, Expression)> = Vec::new();
                let mut assoc_params: Vec<(String, String, bool, Option<bool>)> = Vec::new();
                // Member-wise struct `output`/`inout`/`ref` formals bypass the
                // whole-value `output_bindings` path (see exec_function_call).
                let mut struct_output_writebacks: Vec<(String, Expression)> = Vec::new();
                for (i, port) in ports.iter().enumerate() {
                    // A plain scalar or class formal: none of the struct,
                    // collection, array or width probes below can claim it.
                    let pp = &plan.ports[i];
                    let class_ref = if plan.type_gen == self.type_tables_gen {
                        pp.class_ref
                    } else {
                        self.port_class_ref(port)
                    };
                    let plain_class = class_ref.is_some();
                    let plain = plain_class || pp.plain_scalar;
                    if !plain && matches!(self.resolve_dt_ref(&port.data_type), DataType::Struct(_))
                    {
                        self.register_formal_type_metadata(
                            &port.name.name,
                            &port.data_type,
                            port.dimensions.is_empty(),
                        );
                    }
                    let is_assoc = !plain && self.port_is_assoc_array(port);
                    // §13.5.3: an omitted argument binds the formal's DEFAULT
                    // through the aggregate binders below, like a supplied
                    // actual (see `exec_function_call_body`).
                    let actual = args
                        .get(i)
                        .filter(|_| arg_supplied(i))
                        .or_else(|| Self::default_actual(port));
                    // An associative-array `output`/`inout`/`ref` formal
                    // is copied back via the assoc-param signal-namespace
                    // merge, NOT the scalar `output_bindings` path (which
                    // can't represent an AA). §13.5.2.
                    if matches!(
                        port.direction,
                        PortDirection::Output | PortDirection::Inout | PortDirection::Ref
                    ) && !is_assoc
                    {
                        if arg_supplied(i) {
                            output_bindings.push((port.name.name.clone(), args[i].clone()));
                        } else if let Some(d) = port
                            .default
                            .as_ref()
                            .filter(|d| port.dimensions.is_empty() && Self::is_lvalue_expr(d))
                        {
                            // §13.5.3: a default is in the METHOD's scope, so
                            // `output int o = p` names this object's `p`:
                            // copied back while `this` is still current.
                            default_out_bindings.push((port.name.name.clone(), d.clone()));
                        }
                    }
                    if let Some(actual) = actual.filter(|_| !plain) {
                        if let Some((param, caller, prior)) = self.bind_assoc_param(port, actual) {
                            let is_out = matches!(
                                port.direction,
                                PortDirection::Output | PortDirection::Inout | PortDirection::Ref
                            );
                            assoc_params.push((param, caller, is_out, prior));
                            continue;
                        }
                        // §13.5.2: a formal with an unpacked QUEUE / array
                        // dimension is a COLLECTION of structs, not a
                        // member-wise struct formal — binding it here made
                        // `ref rec_t items[$]` a scalar struct, so the
                        // callee's push_backs never reached the caller.
                        let struct_arg_binding = if port.dimensions.is_empty() {
                            self.bind_unpacked_struct_arg(
                                &port.name.name,
                                &port.data_type,
                                actual,
                                &mut locals,
                                Some(handle),
                            )
                        } else {
                            None
                        };
                        if let Some(struct_entries) = struct_arg_binding {
                            // §13.5.2: a struct METHOD formal binds member-wise (above) but
                            // skips the scalar formal registration below, so `p_elem_type`
                            // misses it and a whole-struct reference to the formal reads a
                            // zeroed container. Register the same struct type metadata the
                            // free-function path does — resolving a type-PARAMETER formal
                            // (`function void f(T item)` with struct `T`) to its concrete
                            // bound struct, like `bind_unpacked_struct_arg` does.
                            let reg_dt =
                                if let DataType::TypeReference { name: tn, .. } = &port.data_type {
                                    let cname = self
                                        .heap
                                        .get(handle)
                                        .and_then(|o| o.as_ref())
                                        .and_then(|i| i.type_bindings.get(&tn.name.name).cloned())
                                        .or_else(|| self.resolve_type_param_binding(&tn.name.name))
                                        .unwrap_or_else(|| tn.name.name.clone());
                                    self.module
                                        .typedef_types
                                        .get(&cname)
                                        .cloned()
                                        .unwrap_or_else(|| port.data_type.clone())
                                } else {
                                    port.data_type.clone()
                                };
                            self.register_formal_type_metadata(
                                &port.name.name,
                                &reg_dt,
                                port.dimensions.is_empty(),
                            );
                            if matches!(
                                port.direction,
                                PortDirection::Output | PortDirection::Inout | PortDirection::Ref
                            ) {
                                struct_output_writebacks.extend(struct_entries);
                            }
                            continue;
                        }
                        // §6.18/§6.20.3: typedef'd / type-param-bound
                        // formal — dims live on the type (see the same
                        // resolution in exec_function_call).
                        // getenv is NOT free: this probe ran once per
                        // typedef'd formal of every method call (0.85% of an
                        // AVIP run). Latch it once.
                        static BD_DBG: OnceLock<bool> = OnceLock::new();
                        if *BD_DBG.get_or_init(|| std::env::var("XEZIM_BD_DBG").is_ok()) {
                            if let crate::ast::types::DataType::TypeReference { name, .. } =
                                &port.data_type
                            {
                                eprintln!(
                                    "[BDDBG] port={} tn={} in_tud={} tud_keys={:?}",
                                    port.name.name,
                                    name.name.name,
                                    self.module
                                        .typedef_unpacked_dims
                                        .contains_key(&name.name.name),
                                    self.module.typedef_unpacked_dims.keys().collect::<Vec<_>>()
                                );
                            }
                        }
                        let eff_dims: Vec<crate::ast::types::UnpackedDimension> =
                            if port.dimensions.is_empty() {
                                if let crate::ast::types::DataType::TypeReference { name, .. } =
                                    &port.data_type
                                {
                                    let tn = &name.name.name;
                                    // Resolve against the CALLEE's own
                                    // type bindings — `this` isn't pushed
                                    // yet while formals bind.
                                    let concrete = self
                                        .heap
                                        .get(handle)
                                        .and_then(|o| o.as_ref())
                                        .and_then(|i| i.type_bindings.get(tn).cloned())
                                        .or_else(|| self.resolve_type_param_binding(tn))
                                        .unwrap_or_else(|| tn.clone());
                                    // CLASS-LOCAL typedef (`typedef int
                                    // q_t[$];` inside the class): the dims
                                    // live on the callee class's own
                                    // typedef table, walked before the
                                    // module's — a `ref q_t out_q` formal
                                    // was bound as a scalar, so the
                                    // callee's push_backs never reached
                                    // the caller (§13.5.2).
                                    Self::typedef_dims_via_tables(&self.module, &cname, &concrete)
                                        .unwrap_or_default()
                                } else {
                                    Vec::new()
                                }
                            } else {
                                port.dimensions.clone()
                            };
                        if let Some((store, caller)) = self.bind_queue_param(
                            &port.name.name,
                            &eff_dims,
                            actual,
                            &port.data_type,
                            port.direction,
                        ) {
                            let is_out = matches!(
                                port.direction,
                                PortDirection::Output | PortDirection::Inout | PortDirection::Ref
                            );
                            if is_out && !caller.is_empty() {
                                queue_writebacks.push((store, caller));
                            }
                            continue;
                        }
                        // §13.5.2 / §18.5.12: a FIXED unpacked-array formal
                        // (`function bit [7:0] parity(bit [7:0] a[4])`) is
                        // passed BY VALUE. Only the free-function path bound
                        // one; a class method bound it as a scalar, so the
                        // body read X and a user function taking an array —
                        // legal in a constraint — always returned 0.
                        let prior = self.module.arrays.contains_key(&port.name.name)
                            || self.module.arrays_2d.contains_key(&port.name.name)
                            || self.module.arrays_nd.contains_key(&port.name.name);
                        if let Some(info) = self.bind_array_arg(port, actual) {
                            // Keep fixed-array formals visible to class-scope lookup (§13.5).
                            locals.insert(
                                format!("\u{1}array_formal:{}", port.name.name),
                                Value::zero(1),
                            );
                            if matches!(
                                port.direction,
                                PortDirection::Output | PortDirection::Inout | PortDirection::Ref
                            ) {
                                if !info.1.is_empty() {
                                    array_writebacks.push(info);
                                }
                            } else if !prior {
                                // An input array formal is dropped on return
                                // too (#327's "warming"; see the function path).
                                array_inputs.push(info);
                            }
                            continue;
                        }
                    }
                    // §11.6.1/§11.8.2: a call ACTUAL is CONTEXT-DETERMINED by
                    // the callee's declared formal, exactly like the operand of
                    // a comparison (see `actual_ctx_shape`). Gated to the shape
                    // `resize_for_assign` already understands, and the width is
                    // read from the cached plan rather than recomputed.
                    let actual_ctx = if arg_supplied(i) && Self::actual_ctx_shape(port) {
                        plan.ports[i].fixed_width.filter(|w| *w > 0)
                    } else {
                        None
                    };
                    // Class methods have automatic lifetime. An output handle
                    // starts null on every call; it never copies in the caller's
                    // previous object (including when the actual is a default).
                    let mut val = if plain_class && port.direction == PortDirection::Output {
                        Value::zero(32)
                    } else if arg_supplied(i) {
                        let value = match actual_ctx {
                            Some(w) => self.eval_expr_ctx(&args[i], w),
                            None => self.eval_expr(&args[i]),
                        };
                        self.vif_argument_value(port, &args[i], value)
                    } else if let Some(def) = &port.default {
                        // Caller omitted this arg → apply the formal's
                        // default (was using 0). UVM uvm_sequence_base::
                        // start(..., int this_priority = -1): missing arg
                        // must be -1, not 0, else `this_priority < -1`
                        // (read unsigned) fatals SEQPRI.
                        //
                        // §13.5.3: a default is evaluated at call time IN
                        // THE CALLEE'S SCOPE, so it may reference `this` /
                        // call another method on the receiving instance
                        // (`uvm_event#(T)::trigger(T data =
                        // get_default_data())`). The instance's `this` and
                        // class context are only pushed AFTER this binding
                        // loop, so evaluate the default with them pushed
                        // (and the partially-bound formals visible) so a
                        // `this`-dependent method default resolves instead
                        // of silently evaluating to the type's zero value.
                        self.this_stack.push(Some(handle));
                        self.class_context_stack.push(Some(cname.clone()));
                        self.push_local_frame_typed(locals.clone(), frame_types.clone());
                        let v = self.eval_formal_default(port, def);
                        let v = self.vif_argument_value(port, def, v);
                        self.pop_local_frame();
                        self.class_context_stack.pop();
                        self.this_stack.pop();
                        v
                    } else {
                        Value::zero(32)
                    };
                    // §13.5.1/§6.18/§10.7: a scalar INTEGRAL formal adopts its
                    // declared (possibly typedef-derived) width and signedness.
                    // Without this a class-method formal declared as a wider
                    // typedef (e.g. `uvm_integral_t` = 64-bit) bound a 32-bit
                    // actual as-is, so the high bits read x and uvm_pack_*
                    // emitted garbage for `$realtobits(shortreal)` and negative
                    // ints passed to a 64-bit packer formal. This is scoped to
                    // TYPEREFERENCE formals only: a direct `bit[W-1:0]`/`int`
                    // ports stays on the old (signedness-only) path so a width
                    // derived from a CLASS type-param (not in module.parameters)
                    // isn't mis-resolved to `1` and truncates the caller's
                    // value.
                    if matches!(&port.data_type, DataType::TypeReference { .. }) {
                        if plain_class {
                            // A class handle is bound as passed.
                        } else if let Some((pw, signed)) =
                            self.scalar_formal_integral(&port.data_type)
                        {
                            val = if val.is_real {
                                Self::real_to_int(val.to_f64(), pw.max(1))
                            } else if pw != val.width {
                                val.resize_for_assign(pw)
                            } else {
                                val
                            };
                            val.is_signed = signed;
                            if val.has_xz()
                                && super::elaborate::is_type_two_state_resolved(
                                    &port.data_type,
                                    &self.module.typedef_types,
                                )
                            {
                                val = val.to_two_state();
                            }
                        } else if self.type_is_signed_concrete(&port.data_type) {
                            // A TYPE-PARAMETER formal (`T t` with `T`->`int`)
                            // isn't a resolvable integral typedef, so
                            // `scalar_formal_integral` falls through; still
                            // stamp the resolved signedness so `%p`/`%0d` on a
                            // `T=int` formal renders signed like the reference.
                            val.is_signed = true;
                        }
                    } else if matches!(&port.data_type, DataType::IntegerAtom { .. })
                        || Self::packed_dims_are_literal(&port.data_type)
                    {
                        // A built-in integer formal (`int`, `byte`, ...) or a
                        // packed vector with LITERAL bounds (`logic signed
                        // [15:0]`) has a fixed width that no class
                        // type-parameter can alter, so widen the actual FIRST
                        // and stamp the signedness after: marking a 2-bit
                        // unsigned actual signed and letting the frame widen it
                        // later sign-extended `2'b10` into -2 for every such
                        // method formal.
                        let pp = &plan.ports[i];
                        let pw = match pp.fixed_width {
                            Some(w) if plan.type_gen == self.type_tables_gen => w,
                            _ => super::elaborate::resolve_type_width(
                                &port.data_type,
                                Some(&self.module.parameters),
                                Some(&self.module.typedefs),
                            ),
                        };
                        if val.is_real {
                            val = Self::real_to_int(val.to_f64(), pw.max(1));
                        } else if pw > 0 && pw != val.width {
                            val = val.resize_for_assign(pw);
                        }
                        val.is_signed = pp.signed;
                    } else if plan.ports[i].signed {
                        val.is_signed = true;
                    }
                    if plan.ports[i].two_state && val.has_xz() {
                        val = val.to_two_state();
                    }
                    if let DataType::TypeReference {
                        name: tn,
                        type_args,
                        ..
                    } = &port.data_type
                    {
                        let type_name = tn.name.name.clone();
                        // A plain class is a class here; a forward-declared
                        // one is registered as its typedef. Class check
                        // first — see the exec_function_call port loop (a
                        // class name can also key the typedef width table,
                        // which misfiled class-typed formals into the
                        // typedef maps).
                        if self.module.classes.contains_key(&type_name) {
                            // A real class — whether decl'd inline or via a
                            // forward `typedef class` placeholder (`class_ref`
                            // Some(true)) — names a CLASS OBJECT, so record
                            // it in the class overlay (`frame_types.0`),
                            // not just the typedef tables. Otherwise a
                            // forward-declared-class formal (e.g. UVM's
                            // `uvm_process_guard_base guard`) is classified
                            // as a plain typedef and `class_of_var` returns
                            // None, so `receiver_may_be_handle` fails and a
                            // class-handle member write is silently dropped.
                            frame_types
                                .0
                                .insert(port.name.name.clone(), type_name.clone());
                            self.note_formal_type_args(&port.name.name, type_args);
                            if self.var_class_types.get(port.name.name.as_str()) != Some(&type_name)
                            {
                                self.meta_note(&port.name.name);
                                self.var_class_types
                                    .insert(port.name.name.clone(), type_name.clone());
                            }
                            // A forward-declared placeholder keeps its typedef
                            // alias so width/layout lookups still resolve it the
                            // same way (the pre-merge code also recorded it as a
                            // typedef).
                            if self.module.typedefs.contains_key(&type_name)
                                || self.module.enum_members.contains_key(&type_name)
                            {
                                frame_types
                                    .1
                                    .insert(port.name.name.clone(), type_name.clone());
                            }
                        } else if self.module.enum_members.contains_key(&type_name)
                            || self.module.typedefs.contains_key(&type_name)
                        {
                            frame_types
                                .1
                                .insert(port.name.name.clone(), type_name.clone());
                            // An entry already holding this type is left as
                            // is: rewriting it changes nothing, and an
                            // untouched name needs no metadata save.
                            if self.var_typedef_types.get(port.name.name.as_str())
                                != Some(&type_name)
                            {
                                self.meta_note(&port.name.name);
                                self.var_typedef_types
                                    .insert(port.name.name.clone(), type_name);
                            }
                        } else if let Some(concrete) = self
                            .heap
                            .get(handle)
                            .and_then(|o| o.as_ref())
                            .and_then(|i| i.type_bindings.get(&type_name).cloned())
                            .or_else(|| self.resolve_type_param_binding(&type_name))
                        {
                            // The receiver's own binding first: `current_spec`
                            // is not switched to the receiver until the body
                            // runs, and may still name another specialization
                            // (one a blocked process left behind), so `T` of
                            // `fifo #(item)` resolved as `fifo #(msg_t)`'s
                            // struct and the class handle `t` went unrecorded.
                            //
                            // See the identical branch in exec_function_call's
                            // port loop: the formal is typed with a class TYPE
                            // PARAMETER (`IMP imp`), not a real class name. It
                            // used to be skipped, so `class_of_var` fell back
                            // to a STALE same-named `var_class_types` entry from
                            // an unrelated scope (UVM's config_db methods)
                            // and `$cast(imp, parent)` resolved `imp` to the
                            // WRONG class — failing a valid TLM initiator
                            // socket with UVM/TLM2/NOIMP. Record the resolved
                            // concrete class so the cast/new sees the right
                            // type (mirrors the local-VarDecl path, §8.25).
                            let cn_is_class = self.module.classes.contains_key(&concrete)
                                || (concrete.contains('#')
                                    && self.module.classes.contains_key(
                                        concrete.split('#').next().unwrap_or(&concrete),
                                    ));
                            if cn_is_class {
                                frame_types
                                    .0
                                    .insert(port.name.name.clone(), concrete.clone());
                                if self.var_class_types.get(port.name.name.as_str())
                                    != Some(&concrete)
                                {
                                    self.meta_note(&port.name.name);
                                    self.var_class_types
                                        .insert(port.name.name.clone(), concrete);
                                }
                            }
                        }
                    }
                    locals.insert(port.name.name.clone(), val);
                }
                if let Some(rn) = fn_ret_name {
                    // An UNPACKED-struct return type keeps its members in the
                    // frame under `<fn>.<member>`, like any other local of that
                    // type; without them a member write inside the body escaped
                    // the frame.
                    if let ClassMethodKind::Function(f) = &method.kind {
                        let su = match plan.ret.as_ref() {
                            Some(r) if plan.type_gen == self.type_tables_gen => r.unpacked.clone(),
                            _ => self.unpacked_struct_of(&f.return_type),
                        };
                        if let Some(su) = su {
                            for (k, seed) in self.struct_return_leaf_seeds(rn, &su) {
                                locals.insert(k, seed);
                            }
                        }
                    }
                    // Initialise the implicit return variable to match its
                    // declared type. A STRING return must start as an empty
                    // string Value, not a 32-bit int — otherwise an implicit
                    // `funcname = {a, "b", ...}` string-concat assignment
                    // coerces to the int's 32-bit width and reads back empty.
                    let mut init = if let (ClassMethodKind::Function(f), Some(rp)) =
                        (&method.kind, plan.ret.as_ref())
                    {
                        if rp.decl_string {
                            Value::from_string("")
                        } else {
                            // Size the implicit return cell to the declared
                            // return width so a bit-select write
                            // `retname[i] = ...` for i >= 32 (unpack routines
                            // filling a 64-bit integral type) lands; a 32-bit
                            // cell dropped the upper half. Register the width
                            // too so a later `retname = <narrow>` zero-extends
                            // back, mirroring a typed VarDecl.
                            let rw = if plan.type_gen == self.type_tables_gen {
                                rp.width
                            } else {
                                super::elaborate::resolve_type_width(
                                    &f.return_type,
                                    Some(&self.module.parameters),
                                    Some(&self.module.typedefs),
                                )
                            }
                            .max(1);
                            match self.widths.get_mut(rn) {
                                Some(slot) => *slot = rw,
                                None => {
                                    self.widths.insert(rn.to_string(), rw);
                                }
                            }
                            if rp.signed {
                                if !self.signed_signals.contains(rn) {
                                    self.signed_signals.insert(rn.to_string());
                                }
                            } else {
                                self.signed_signals.remove(rn);
                            }
                            // §13.4.1: type default — x for 4-state (see
                            // the module-function twin above).
                            if rp.real {
                                Value::from_f64(0.0)
                            } else if rp.two_state {
                                Value::zero(rw)
                            } else {
                                Value::new(rw)
                            }
                        }
                    } else {
                        Value::zero(32)
                    };
                    if let Some(rp) = plan.ret.as_ref()
                        && rp.class.is_none()
                        && !rp.decl_string
                        && !rp.collection
                    {
                        init.is_signed = rp.signed;
                    }
                    if !locals.contains_key(rn) {
                        locals.insert(rn.to_string(), init);
                    }
                    // Register the return variable's class type so an
                    // implicit-return `funcname = new()` knows WHICH class to
                    // construct — exactly as a typed local `T t = new()` does
                    // (var_class_types is consulted when a bare `new()` has no
                    // explicit class). Without this the bare `new()` can't
                    // resolve its class and yields a broken instance whose
                    // method calls bind the wrong `this`/params. This is the
                    // `uvm_report_message::new_report_message` form
                    // (`new_report_message = new(name)`); fixing it makes the
                    // real report message's set_action/get_action persist so
                    // the report-server `$display` fires natively.
                    if let Some(cn) = plan.ret.as_ref().and_then(|r| r.class.as_deref()) {
                        frame_types.0.insert(rn.to_string(), cn.to_string());
                        if self.var_class_types.get(rn).map(String::as_str) != Some(cn) {
                            self.meta_note(rn);
                            self.var_class_types.insert(rn.to_string(), cn.to_string());
                        }
                    }
                }
                // Loop-control flags (`break`/`continue`) are frame-local:
                // save the caller's, clear them for this body, and restore
                // afterward so a `continue`/`break` inside the callee can't
                // leak out and poison the caller's subsequent statements
                // (exec_statement bails when either flag is set).
                let saved_break = self.break_flag;
                let saved_continue = self.continue_flag;
                let saved_return = self.return_flag;
                self.break_flag = false;
                self.continue_flag = false;
                self.return_flag = false;
                // LRM §25.9 virtual-interface formal-arg aliasing
                // (class-method dispatch). Resolve the actuals' vif bindings
                // NOW, while `this_stack` is still the CALLER's context
                // (the bound vif lives there), before pushing the callee's.
                let mut iface_alias_frame: HashMap<String, String> = HashMap::default();
                let plan_fresh = plan.type_gen == self.type_tables_gen;
                for (i, port) in ports.iter().enumerate() {
                    if !(plan_fresh && plan.ports[i].never_vif) {
                        // §13.5.3: an omitted argument binds the formal's
                        // DEFAULT, so a virtual-interface method formal with a
                        // default still aliases the interface it names.
                        let arg = args
                            .get(i)
                            .filter(|_| arg_supplied(i))
                            .or(port.default.as_ref());
                        if arg.is_some() {
                            if let Some((f, b)) = self.vif_formal_alias(
                                port,
                                args.get(i).filter(|_| arg_supplied(i)),
                                &locals,
                            ) {
                                iface_alias_frame.insert(f, b);
                            }
                        }
                    }
                }
                if !iface_alias_frame.is_empty() {
                    self.consume_vif_argument_return();
                }
                // PURE_SV_LRM §8.25: a `static` member of a parameterized
                // class is per-SPECIALIZATION. Reached from an INSTANCE method
                // (e.g. `resource#(T)::my_type` via `r.get_type_handle()->
                // get_type()`) there is no `#(spec)` on the call, so
                // `current_spec` is None and static_prop_key uses the shared
                // `Class::member` cell — while an explicit `Class#(spec)::member`
                // uses `Class#spec::member`. That made get_type() !=
                // get_type_handle(), breaking resource-pool type matching.
                // Seed current_spec from the instance's own type bindings so
                // both paths key the same per-spec cell. Sig = the type params'
                // bound leaf names in declaration order (the parser now captures
                // builtin type args as Ident leaf names, so this matches
                // extract_call_spec's type_args_text).
                // §8.25 (seeded callee spec — see
                // seed_callee_spec_from_instance): interpreter-route parity
                // for per-specialization statics.
                let saved_spec = self.seed_callee_spec_from_instance(handle, &cname);
                self.this_stack.push(Some(handle));
                // §23.8 / §23.10.1: hierarchical names in the method body
                // resolve upward from the object's CREATION scope, not the
                // caller's. Installed as the resolve hint (the upward walk
                // tries `current_scope` first, then the hint), so a proxy
                // object built inside a bound module finds its binder's
                // host signals even when the call comes from the top level.
                // §8/§23.9: an object of a class declared inside a module
                // runs in its instance (`modclass_instance`), whichever scope
                // made the call.
                let modclass_inst = if self.any_modclass() {
                    self.modclass_instance(handle)
                } else {
                    None
                };
                let saved_resolve_hint = {
                    // `creation_scope` is kept in `%m` form (`tb.u_w`, top
                    // module first) for the method's own `%m`; the
                    // resolution hint must be instance-relative (`u_w`,
                    // the key form of every signal). Installing the `%m`
                    // form made every hinted lookup miss, so a method of an
                    // object built inside `u_w` could not read `u_w`'s own
                    // variables or reach its sibling instances (issue #155).
                    let birth = match &modclass_inst {
                        Some(inst) => inst.clone(),
                        None => self.instance_relative_scope(
                            self.heap
                                .get(handle)
                                .and_then(|o| o.as_ref())
                                .map(|i| i.creation_scope.as_str())
                                .unwrap_or_default(),
                        ),
                    };
                    if birth.is_empty() {
                        None
                    } else {
                        let prev = self.name_resolve_hint.borrow().clone();
                        *self.name_resolve_hint.borrow_mut() = Some(birth);
                        Some(prev)
                    }
                };
                self.commit_formal_dyn(formal_dyn);
                // Its collections (kept out of the name resolution above)
                // through the call's rename frame, under every formal and
                // local of the body.
                let saved_modclass = modclass_inst.map(|inst| {
                    self.modclass_rename_frame(handle, &cname, &inst);
                    self.modclass_depth.set(self.modclass_depth.get() + 1);
                    self.activation_scope.replace(Some(inst))
                });
                self.push_local_frame_typed(locals, frame_types);
                self.open_decl_shadow_frame();
                // Record the `local_stack` depth BEFORE this method's own
                // frame (i.e. the count of caller frames) so that
                // `get_expr_type_name`'s `in_any_frame` check only considers
                // the current method's locals — not a caller's same-named
                // local that leaked into the flat `var_class_types` map.
                self.method_local_base.push(self.local_stack.len() - 1);
                // §8.7 ctor-time binding (reference behavior): while class
                // C's `new` body runs, an unqualified method call on `this`
                // binds within C's own chain — a derived override must not
                // run against a partially-constructed object.
                if method_name == "new" {
                    self.ctor_class_stack.push((handle, cname.clone()));
                    if self
                        .pending_class_initializers
                        .get(&handle)
                        .is_some_and(|classes| {
                            classes.iter().any(|cd| {
                                cd.name == *cname
                                    || self
                                        .module
                                        .classes
                                        .get(cname.as_str())
                                        .is_some_and(|target| std::sync::Arc::ptr_eq(target, cd))
                            })
                        })
                        && !self.constructor_calls_super_new(cname)
                    {
                        self.initialize_implicit_constructor_base(handle, cname);
                        self.initialize_pending_class_properties(handle, cname);
                    }
                }
                // A function returning a bare type-parameter name: the
                // width and signedness of the type bound on this object, for
                // the return fit below (§6.20.2).
                let tp_ret_fit: Option<(u32, Option<bool>)> = match &method.kind {
                    ClassMethodKind::Function(f) => match &f.return_type {
                        DataType::TypeReference { name, .. }
                            if self.class_member_names().params.contains(&name.name.name) =>
                        {
                            self.type_param_concrete_in(&f.return_type, &cname)
                                .and_then(|c| {
                                    Some((
                                        self.concrete_type_width(&c)?,
                                        self.concrete_type_signed(&c),
                                    ))
                                })
                        }
                        _ => None,
                    },
                    _ => None,
                };
                // §13.4.1: the numeric kind of the declared return type, for
                // the value conversion at the return below.
                let ret_conv = match (&method.kind, plan.ret.as_ref()) {
                    // A class-handle return (the common UVM shape): no lookup.
                    (ClassMethodKind::Function(_), Some(r))
                        if r.class.is_some() && plan.type_gen == self.type_tables_gen =>
                    {
                        RetConv::Keep
                    }
                    (ClassMethodKind::Function(f), _) if method_name != "new" => {
                        self.method_ret_conv(&f.return_type, &cname)
                    }
                    _ => RetConv::Keep,
                };
                // A STATIC call (no object) of a function whose return is
                // sized by a class parameter: the call's specialization
                // sizes it, there being no instance to read it from.
                let static_dyn_w: Option<u32> = match &dyn_ret_type {
                    Some(rt) if self.heap.get(handle).and_then(|o| o.as_ref()).is_none() => {
                        self.class_context_stack.push(Some(cname.clone()));
                        let w = self.class_param_sized_width(rt);
                        self.class_context_stack.pop();
                        w
                    }
                    _ => None,
                };
                self.class_context_stack.push(Some(cname.clone()));
                self.local_iface_aliases.push(iface_alias_frame);
                // §13.4: a FUNCTION method may return a collection (queue /
                // unsized dynamic array) via a LITERAL (`return '{}` /
                // `return '{...}`). The free-function `Return` handler routes
                // those through `fn_ret_collection_stack`, so push the flag
                // here too — otherwise the shared `StatementKind::Return`
                // lost the whole collection (an empty `'{}` came back as a
                // one-element queue when assigned to a queue variable).
                let method_ret_collection = match (&method.kind, plan.ret.as_ref()) {
                    (ClassMethodKind::Function(_), Some(r))
                        if plan.type_gen == self.type_tables_gen =>
                    {
                        r.collection
                    }
                    (ClassMethodKind::Function(f), _) => self.fn_returns_collection(&f.return_type),
                    _ => false,
                };
                self.fn_ret_collection_stack.push(method_ret_collection);
                // §13.4/§7.9.9: and whether it returns an ASSOCIATIVE array,
                // so `return w;` snapshots the local before the frame is
                // torn down (as for a free function); the caller read an
                // empty array.
                let method_ret_assoc = match &method.kind {
                    ClassMethodKind::Function(f) => matches!(
                        self.fn_ret_coll_dims(&f.return_type)
                            .as_ref()
                            .and_then(|d| d.first()),
                        Some(crate::ast::types::UnpackedDimension::Associative { .. })
                    ),
                    _ => false,
                };
                self.fn_ret_assoc_stack.push(method_ret_assoc);
                // §13.4.1: a method returning a queue, dynamic or associative
                // array has a return VARIABLE of that type, which the body may
                // fill in place (`build[k] = v`); declare it as a collection
                // local, as for a free function.
                let method_ret_coll: Option<crate::ast::decl::FunctionDeclaration> =
                    match &method.kind {
                        ClassMethodKind::Function(f)
                            if fn_ret_name.is_some()
                                && self.fn_ret_coll_dims(&f.return_type).is_some() =>
                        {
                            Some(f.clone())
                        }
                        _ => None,
                    };
                if let (Some(f), Some(rn)) = (&method_ret_coll, fn_ret_name) {
                    if let Some(frame) = self.local_stack.last_mut() {
                        frame.remove(rn);
                    }
                    self.widths.remove(rn);
                    let decl = Statement::new(
                        StatementKind::VarDecl {
                            data_type: f.return_type.clone(),
                            lifetime: None,
                            declarators: vec![crate::ast::stmt::VarDeclarator {
                                name: f.name.name.clone(),
                                dimensions: Vec::new(),
                                init: None,
                                span: f.span,
                            }],
                        },
                        f.span,
                    );
                    self.exec_statement(&decl);
                }
                // §6.21: open a static-local sync frame so a `static`
                // local declared in the method body persists across calls.
                // (Class methods previously never opened one — only free
                // functions/tasks did — so a `static this_type m_inst;`
                // inside `get()` was re-initialized every call and class
                // factory singletons never survived.) The key itself is
                // made class/spec-aware at the declaration site above.
                let sync_name = self.sync_frame_name(method_name);
                self.static_local_syncs.push((sync_name, Vec::new()));
                // §21.2.1.7: the method body starts its own `%m` chain (see
                // `m_path`); the caller's named blocks are not its scope.
                let saved_m_scope = std::mem::take(&mut self.m_scope_stack);
                // Sampled-method profiler: publish this method as in-flight
                // so a sampler thread can build a wall-time histogram. Use a
                // save/restore guard so nested method calls (which re-enter
                // this function) surface the INNERMOST active method while
                // running, then restore the caller's on the way out. Gated by
                // SAMPLER_READY so normal runs pay only a predicted branch.
                let _prev_meth = if SAMPLER_READY.load(Ordering::Relaxed) {
                    sampler_register_name(fnv_name(method_name), method_name);
                    Some(CUR_METHOD_HASH.swap(fnv_name(method_name), Ordering::Relaxed))
                } else {
                    None
                };
                // Planner split: publish whether THIS method's body runs as
                // bytecode (true) or the AST interpreter (false), restored
                // alongside _prev_meth above when the method returns.
                let _prev_compiled = if SAMPLER_READY.load(Ordering::Relaxed) {
                    Some(CUR_METHOD_COMPILED.swap(false, Ordering::Relaxed))
                } else {
                    None
                };
                // class-perf pilot: a fully-lowered Function body may run as
                // bytecode instead of the AST interpreter. ALL-OR-NOTHING:
                // `try_run_compiled_method` returns None unless every
                // statement lowered without a fallback, in which case the
                // value is byte-identical to the interpreter by construction
                // (the compiled block reads `this` members and locals from
                // the same heap / register file the AST path uses). Gated off
                // by default.
                // Wait-free tasks compile like functions, after the same
                // call-count tier: compiling every task on its first call
                // paid a whole plan + compile for each of the many tasks a
                // UVM run calls only a few times.
                let mut ast_string_cleanup = None;
                if let Some(cval) = compiled_methods_enabled()
                    .then(|| {
                        self.try_run_compiled_method(
                            handle,
                            cname,
                            method_name,
                            ports,
                            body,
                            fn_ret_name,
                            ret_is_string,
                            CompiledBodyKind::of(&method.kind),
                        )
                    })
                    .flatten()
                {
                    self.return_value = Some(cval);
                } else {
                    // §6.16/§13.5: classify the callee's formal types under
                    // its active specialization, independently of global
                    // string flags. Common frame teardown removes this entry.
                    if let Some(&fgen) = self.local_gen_stack.last() {
                        if let Some(entry) = self.task_frame_strings(fgen, ports) {
                            self.task_string_formals.push(entry);
                        }
                    }
                    // Mark string-typed return variable, params, and class
                    // properties only when the AST interpreter actually runs
                    // this body (compiled bytecode never reads `string_signals`).
                    let mut frame_string_signals: Vec<String> = Vec::new();
                    self.string_signals_removed.push(Vec::new());
                    if let ClassMethodKind::Function(f) = &method.kind {
                        for port in ports.iter() {
                            if !Self::is_string_data_type(&port.data_type) {
                                self.forget_string_flag(&port.name.name);
                            }
                        }
                        if plan.ret.as_ref().is_some_and(|r| r.decl_string) {
                            if self.string_signals.insert(f.name.name.name.clone()) {
                                frame_string_signals.push(f.name.name.name.clone());
                            }
                        }
                    }
                    for port in ports.iter() {
                        if Self::is_string_data_type(&port.data_type) {
                            if self.string_signals.insert(port.name.name.clone()) {
                                frame_string_signals.push(port.name.name.clone());
                            }
                        }
                    }
                    let frame_string_props = found_string_props
                        .clone()
                        .unwrap_or_else(|| self.class_string_props_of(cname));
                    let mut frame_string_prop_mask: u64 = 0;
                    let mut frame_string_prop_more: Vec<usize> = Vec::new();
                    for (i, sp) in frame_string_props.iter().enumerate() {
                        if !self.string_signals.contains(sp) {
                            self.string_signals.insert(sp.clone());
                            if i < 64 {
                                frame_string_prop_mask |= 1u64 << i;
                            } else {
                                frame_string_prop_more.push(i);
                            }
                        }
                    }
                    // class-perf loop-heat boost (pass C): while this method
                    // body runs on the AST interpreter, attribute its loop
                    // back-edges to it so the measured-heat gate can admit
                    // repeat-loop bodies WITHOUT compiling short UVM loops.
                    // Only wait-free loop candidates are tracked (memoized via
                    // is_loop_candidate / pass B), and only when the boost is
                    // enabled; when off the heat stack stays empty and
                    // `record_ast_loop_heat` is a no-op, keeping the default
                    // path flat.
                    let heat_key = if compiled_methods_enabled() && method_boost_enabled() {
                        let kind_of = CompiledBodyKind::of(&method.kind);
                        match (
                            self.compiled_class_method_ids.get(cname.as_str()).copied(),
                            self.compiled_class_method_ids.get(method_name).copied(),
                        ) {
                            (Some(c), Some(m)) => {
                                let key = (c, m, self.instance_spec_id(handle));
                                if self.is_loop_candidate(key, cname, &kind_of, body) {
                                    Some(key)
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        }
                    } else {
                        None
                    };
                    if let Some(k) = heat_key {
                        self.compiled_method_heat_stack.push(k);
                    }
                    for stmt in body {
                        self.exec_statement(stmt);
                        if self.break_flag || self.return_flag {
                            break;
                        }
                    }
                    if heat_key.is_some() {
                        self.compiled_method_heat_stack.pop();
                    }
                    ast_string_cleanup = Some((
                        frame_string_signals,
                        frame_string_props,
                        frame_string_prop_mask,
                        frame_string_prop_more,
                    ));
                } // end else: AST interpreter path
                if let Some(prev) = _prev_meth {
                    CUR_METHOD_HASH.store(prev, Ordering::Relaxed);
                }
                if let Some(prev) = _prev_compiled {
                    CUR_METHOD_COMPILED.store(prev, Ordering::Relaxed);
                }
                // Write back any static locals declared in this body before
                // the locals frame is dropped.
                self.sync_static_locals();
                self.fn_ret_collection_stack.pop();
                self.fn_ret_assoc_stack.pop();
                // Declaration snapshots describe this activation's string
                // flags. Restore them before reinstating the caller's flags
                // (§6.21/§23.8), or an integral shadow can erase the caller's
                // string classification after it has already been restored.
                self.close_decl_shadow_frame();
                if let Some((
                    frame_string_signals,
                    frame_string_props,
                    frame_string_prop_mask,
                    frame_string_prop_more,
                )) = ast_string_cleanup
                {
                    for n in &frame_string_signals {
                        self.string_signals.remove(n);
                    }
                    let mut mask = frame_string_prop_mask;
                    while mask != 0 {
                        let i = mask.trailing_zeros() as usize;
                        mask &= mask - 1;
                        self.string_signals.remove(&frame_string_props[i]);
                    }
                    for &i in &frame_string_prop_more {
                        self.string_signals.remove(&frame_string_props[i]);
                    }
                    if let Some(removed) = self.string_signals_removed.pop() {
                        for n in removed {
                            self.string_signals.insert(n);
                        }
                    }
                }
                self.restore_spec_pooled(saved_spec);
                if let Some(prev) = saved_resolve_hint {
                    *self.name_resolve_hint.borrow_mut() = prev;
                }
                if let Some(prev) = saved_modclass {
                    self.modclass_depth.set(self.modclass_depth.get() - 1);
                    *self.activation_scope.borrow_mut() = prev;
                }
                let vif_alias_ended = self.local_iface_aliases.pop().unwrap_or_default();
                if self.parked_from_exec {
                    // The process was parked by exec_statement's Wait handler.
                    // Keep break/return flags set so they propagate up to
                    // run_process_stmts. Don't restore the saved values.
                } else {
                    self.break_flag = saved_break;
                    self.continue_flag = saved_continue;
                    self.return_flag = saved_return;
                }
                if method_name == "new" {
                    self.ctor_class_stack.pop();
                    if let Some((caller_handle, caller_class)) =
                        self.ctor_class_stack.last().cloned()
                        && caller_handle == handle
                    {
                        self.initialize_pending_class_properties(handle, &caller_class);
                    }
                }
                self.m_scope_stack = saved_m_scope;
                self.class_context_stack.pop();
                self.method_local_base.pop();
                // §13.4.1: a method returning an UNPACKED struct has no single
                // return cell — its members are frame leaves, so the bare name
                // read back x. Collapse them into the packed form while the
                // frame is still live, exactly as a free function does.
                let unpacked_ret_su = match (&method.kind, plan.ret.as_ref()) {
                    (ClassMethodKind::Function(_), Some(r))
                        if plan.type_gen == self.type_tables_gen =>
                    {
                        r.unpacked.clone()
                    }
                    (ClassMethodKind::Function(f), _) => self.unpacked_struct_of(&f.return_type),
                    _ => None,
                };
                if let (Some(rn), Some(su)) = (fn_ret_name, unpacked_ret_su.as_ref()) {
                    if self.return_value.is_none() {
                        if let Some(v) = self.pack_unpacked_struct(rn, &su.clone()) {
                            self.return_value = Some(v);
                        }
                    }
                }
                // Falling off the end of a collection-returning method hands
                // its return variable over as `return f;` does.
                if let (Some(_), Some(rn)) = (&method_ret_coll, fn_ret_name)
                    && self.return_value.is_none()
                {
                    let n = self
                        .dyn_name_lookup(rn)
                        .map(str::to_string)
                        .unwrap_or_else(|| rn.to_string());
                    if self.is_associative_array(&n) {
                        self.snapshot_ret_assoc(&n);
                        self.return_value = Some(Value::zero(32));
                    } else if self.module.dynamic_arrays.contains(&n) {
                        if self.queue_elem_struct(&n).is_none() {
                            const RET_SNAP: &str = "__xz_ret_coll__";
                            self.copy_whole_queue(RET_SNAP, &n);
                            self.pending_ret_collection = Some(RET_SNAP.to_string());
                        } else {
                            self.pending_ret_collection = Some(n);
                        }
                        self.return_value = Some(Value::zero(32));
                    }
                }
                let implicit = fn_ret_name.and_then(|rn| {
                    let lv = self.local_stack.last().and_then(|m| m.get(rn).cloned());
                    // A `funcname = {a, b, ...}` string-concat assignment is
                    // serviced by the string-concat path, which writes the
                    // result via set_signal_value_by_name (the SIGNAL store),
                    // not the local frame — so a string return whose local is
                    // still empty has its real value in the signal store.
                    if ret_is_string && lv.as_ref().is_none_or(|v| v.to_sv_string().is_empty()) {
                        self.get_signal_value_by_name(rn).or(lv)
                    } else {
                        lv
                    }
                });
                let mut ret = self
                    .return_value
                    .take()
                    .or(implicit)
                    .unwrap_or(Value::zero(32));
                if Self::ret_needs_convert(&ret, ret_conv)
                    && !ret_is_string
                    && let ClassMethodKind::Function(f) = &method.kind
                {
                    let (w, sg) = self.method_ret_typedef_fit(&f.return_type, &cname);
                    ret = Self::ret_convert(ret, ret_conv, w, sg);
                }
                // Per-instance return clamp for a class-parameter-sized
                // return type: `box#(4)` with `function bit [W-1:0] mk()`
                // must hand back 4 bits, not whatever width the body's
                // last assignment happened to carry.
                if let Some(rt) = &dyn_ret_type {
                    if !ret.is_real {
                        let w = match static_dyn_w {
                            Some(w) => w,
                            None => {
                                let scope = self.instance_param_scope(handle);
                                resolve_type_width(rt, Some(&scope), Some(&self.module.typedefs))
                            }
                        };
                        if w > 0 && w != ret.width {
                            ret = ret.resize_for_assign(w);
                        }
                    }
                }
                // §6.20.2 / §8.25: a return typed by a TYPE PARAMETER
                // (`function T get();`) takes the type bound on this object
                // — `return '1` is eight 1s for `T = byte`, not one.
                if let Some((w, sg)) = tp_ret_fit.filter(|_| !ret.is_real) {
                    if w != ret.width || ret.is_fill {
                        ret = ret.resize_for_assign(w);
                    }
                    if let Some(sg) = sg {
                        ret.is_signed = sg;
                    }
                }
                // §13.4.1: a class method's return takes its DECLARED type,
                // exactly as a module-level function's does. Only the
                // parameter-sized case above was handled, so
                // `function bit [15:0] u; return -1;` handed back the
                // literal's signedness and `c.u() > 0` read it as -1.
                if let ClassMethodKind::Function(f) = &method.kind {
                    // Only PLAINLY integral declared types are stamped — a
                    // TypeReference may name a class (the return is a heap
                    // HANDLE whose "width" is meaningless and whose value
                    // must pass through untouched), and Implicit means no
                    // declared type at all. The first version of this stamp
                    // resized those and broke handle-returning methods.
                    let plainly_integral = matches!(
                        f.return_type,
                        DataType::IntegerVector { .. } | DataType::IntegerAtom { .. }
                    );
                    if plainly_integral && !ret.is_real && !ret_is_string && dyn_ret_type.is_none()
                    {
                        let w = match plan.ret.as_ref() {
                            Some(r) if plan.type_gen == self.type_tables_gen => r.width,
                            _ => resolve_type_width(
                                &f.return_type,
                                Some(&self.module.parameters),
                                Some(&self.module.typedefs),
                            ),
                        };
                        if w > 0 && w != ret.width {
                            ret = ret.resize(w);
                        }
                        ret.is_signed = plan.ret.as_ref().map_or_else(
                            || super::elaborate::is_type_signed(&f.return_type),
                            |r| r.signed,
                        );
                    }
                }
                ret = Self::ret_two_state(ret, ret_conv);
                // Snapshot output/ref formal values before dropping locals.
                let writebacks: Vec<(String, Value, Expression)> = output_bindings
                    .iter()
                    .filter_map(|(pn, caller)| {
                        self.local_stack
                            .last()
                            .and_then(|l| l.get(pn).cloned())
                            .map(|v| (pn.clone(), v, caller.clone()))
                    })
                    .collect();
                // Member-wise struct output formals: snapshot each member
                // local (`o.a`, `o.b`) before the frame is popped.
                let struct_wb: Vec<(Expression, Value)> = struct_output_writebacks
                    .iter()
                    .filter_map(|(local_key, caller_lval)| {
                        self.local_stack
                            .last()
                            .and_then(|l| l.get(local_key).cloned())
                            .map(|v| (caller_lval.clone(), v))
                    })
                    .collect();
                let default_wb = if default_out_bindings.is_empty() {
                    Vec::new()
                } else {
                    self.snapshot_default_outs(&default_out_bindings)
                };
                self.pop_local_frame();
                if !default_wb.is_empty() {
                    self.apply_default_outs(default_wb, &cname);
                }
                self.this_stack.pop();
                // Capture the callee's formals, restore the caller's
                // shadowed storage, then apply — see `stage_queue_param`.
                let mut staged_wb: Vec<(String, String)> = Vec::new();
                for (param, caller) in &queue_writebacks {
                    if caller.is_empty() || param == caller {
                        continue;
                    }
                    let tmp = self.stage_queue_param(param);
                    staged_wb.push((tmp, caller.clone()));
                }
                // An assoc OUTPUT formal's per-call key is torn down by the
                // restore below, so stage its element data onto a temp key
                // now; the writeback applies after the restore (the caller's
                // shadowed storage is then put back — doing the copy now
                // would let a same-name callee local wipe it).
                let assoc_params = std::mem::take(&mut assoc_params);
                let assoc_wb = self.stage_assoc_writebacks(
                    assoc_params
                        .iter()
                        .map(|(p, c, out, _)| (p.as_str(), c.as_str(), *out)),
                );
                self.pop_and_restore_queue_frame();
                for (tmp, caller) in staged_wb {
                    self.writeback_queue_param(&tmp, &caller);
                    self.drop_staged_queue(&tmp);
                }
                // The caller's shadowed storage is now restored — apply the
                // deferred assoc writebacks (from staged temp keys).
                self.apply_assoc_writebacks(assoc_wb);
                for (param, caller, _is_out, prior) in assoc_params {
                    if param == caller {
                        // Identity binding — the body's writes landed directly
                        // in the caller's namespace; just restore the prior
                        // registration (a nested same-name formal must not
                        // leave the flat `module.associative_arrays` entry
                        // clobbered).
                        match prior {
                            Some(v) => {
                                self.module.associative_arrays.insert(param.clone(), v);
                            }
                            None => {
                                self.module.associative_arrays.remove(&param);
                            }
                        }
                        continue;
                    }
                    // `is_out` element copy already applied from the staged
                    // temp key above — the formal's per-call key is gone now,
                    // so calling ``writeback_assoc_param`` would wipe the
                    // restored caller's data. Purge alone.
                    self.purge_assoc_param(&param, prior);
                }
                // §13.5.2: copy `output`/`inout`/`ref` fixed-array formals
                // back onto the caller's array (a plain `input` formal is
                // by value and must NOT write back — §18.5.12 relies on
                // that for array args to constraint functions).
                if !array_writebacks.is_empty() {
                    self.writeback_array_args(&array_writebacks);
                    self.purge_array_formals(&array_writebacks);
                }
                if !array_inputs.is_empty() {
                    self.purge_array_formals(&array_inputs);
                }
                let outs: Vec<&str> = writebacks.iter().map(|(pn, _, _)| pn.as_str()).collect();
                let vif_ended = if !vif_fast {
                    self.vif_formals_exit(vif_saved, &vif_formals, &outs)
                } else if self.vif_local_names.len() == vif_len0 {
                    // Still no formal name keyed: the exit only moves the
                    // store generation again.
                    if !ports.is_empty() {
                        bump_store_gen();
                    }
                    HashMap::default()
                } else {
                    // The body keyed some name: replay the general exit from
                    // the (empty) entry state.
                    let vf: Vec<(&str, Option<&Expression>)> =
                        ports.iter().map(|p| (p.name.name.as_str(), None)).collect();
                    let saved: Vec<(String, Option<Value>)> =
                        ports.iter().map(|_| (String::new(), None)).collect();
                    self.vif_formals_exit(saved, &vf, &outs)
                };
                for (pn, v, caller) in writebacks {
                    self.vif_writeback(
                        &caller,
                        &v,
                        vif_alias_ended
                            .get(&pn)
                            .or_else(|| vif_ended.get(&pn))
                            .map(String::as_str),
                    );
                }
                for (caller_lval, v) in struct_wb {
                    self.assign_value(&caller_lval, &v);
                }
                if let Some(frame) = meta_open.then(|| self.meta_frames.pop()).flatten() {
                    if !frame.full {
                        self.meta_pending -= 1;
                    }
                    // The saved names in the eager order (last position
                    // first); a name never written still holds its entry
                    // state.
                    let mut formal_parts = frame.parts;
                    formal_parts.sort_by_key(|(k, _)| *k);
                    while let Some((k, parts)) = formal_parts.pop() {
                        let n = match ports.get(k) {
                            Some(p) => p.name.name.as_str(),
                            None => fn_ret_name.unwrap_or_default(),
                        };
                        self.restore_formal_parts(n, parts);
                    }
                    if self.formal_parts_pool.len() < 64 {
                        self.formal_parts_pool.push(formal_parts);
                    }
                }
                if let Some(h) = reg_guard_obj {
                    self.factory_reg_in_progress.remove(&h);
                }
                return ret;
            }
        }
        if let Some(h) = reg_guard_obj {
            self.factory_reg_in_progress.remove(&h);
        }
        Value::zero(32)
    }
}
