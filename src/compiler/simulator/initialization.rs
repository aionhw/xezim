//! Simulator initialization.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// `XEZIM_MEM_CENSUS` end-of-run class-heap line: live object slots,
    /// property entries, and the heap bytes of their names.
    pub fn class_heap_census(&self) -> String {
        let mut objs = 0usize;
        let mut props = 0usize;
        let mut key_bytes = 0usize;
        let mut name_bytes = 0usize;
        for inst in self.heap.iter().flatten() {
            objs += 1;
            props += inst.properties.len();
            key_bytes += inst.properties.keys().map(|k| k.len()).sum::<usize>();
            name_bytes += inst.class_name.len() + inst.creation_scope.len();
        }
        format!(
            "class heap: {} slots, {} live objects, {} properties ({:.1} MB of property-name bytes, {:.1} MB of class/scope name bytes)",
            self.heap.len(),
            objs,
            props,
            key_bytes as f64 / 1048576.0,
            name_bytes as f64 / 1048576.0
        )
    }

    /// Safe accessor for `id_to_name`. Returns "" for an id without a
    /// registered name (a bulk-memory cell); the element of a virtual-name
    /// array gets its `base[idx]` name (see `name_opt`).
    #[inline]
    pub(super) fn name_for_id(&self, id: usize) -> &str {
        self.name_opt(id).unwrap_or(EMPTY_NAME)
    }

    /// The registered name of signal `id`: its stored name, or — for an
    /// element of a virtual-name array (see `NameMap`) — `base[idx]`,
    /// built on first request and cached.
    #[inline]
    pub(super) fn name_opt(&self, id: usize) -> Option<&str> {
        if let Some(n) = self.id_to_name.get(id) {
            return Some(n);
        }
        self.virtual_elem_name(id)
    }

    #[inline(never)]
    pub(super) fn virtual_elem_name(&self, id: usize) -> Option<&str> {
        if let Some(b) = self.elem_name_cache.borrow().get(&id) {
            let p: *const str = &**b;
            // SAFETY: cache entries are never removed or replaced, and a
            // `Box<str>`'s bytes do not move when the map rehashes, so the
            // string lives as long as `self`.
            return Some(unsafe { &*p });
        }
        let name = self.signal_name_to_id.virtual_name_of(id)?;
        let mut cache = self.elem_name_cache.borrow_mut();
        let b = cache.entry(id).or_insert_with(|| name.into_boxed_str());
        let p: *const str = &**b;
        // SAFETY: as above.
        Some(unsafe { &*p })
    }

    /// IEEE 1800-2017 §18.3 / §18.4 — hoist CLASS-LOCAL typedefs into the
    /// module type tables.
    ///
    /// A type declared inside a class body
    ///     class C;
    ///       typedef enum bit [1:0] { START, DATA, STOP } type_e;
    ///       typedef struct packed { bit valid; bit [2:0] tag; } header_s;
    ///       rand type_e   pkt_type;
    ///       rand header_s pkt_hdr;
    ///     endclass
    /// is recorded by elaboration only in `ElaboratedClass::typedef_targets`;
    /// `module.enum_members` / `module.typedef_types` / `module.typedefs` never
    /// learn about it. Two §18 bugs follow directly:
    ///
    ///  * §18.3 — `pkt_type` has no enum-member list, so randomize() draws a
    ///    raw 2-bit pattern and can hand back an ILLEGAL encoding (3) instead of
    ///    one of the declared members.
    ///  * §18.4 — `pkt_hdr` has no packed layout, so `pkt_hdr.valid` misses the
    ///    aggregate bit-slice path entirely, falls through to the generic
    ///    member-access lookup (which treats the struct's bit pattern as an
    ///    object handle) and reads X.
    ///
    /// Republish each class-local typedef under its bare name. `or_insert`
    /// semantics: a module/package type of the same name always wins.
    pub(super) fn hoist_class_local_typedefs(module: &mut ElaboratedModule) {
        // Class iteration order is not source order, so collect first and apply
        // deterministically (by class name) — two classes may declare a
        // same-named local type and the winner must not depend on hash order.
        let mut classes: Vec<String> = module.classes.keys().cloned().collect();
        classes.sort();
        for cn in classes {
            let Some(cd) = module.classes.get(&cn) else {
                continue;
            };
            let mut targets: Vec<(String, DataType)> = cd
                .typedef_targets
                .iter()
                .map(|(n, dt)| (n.clone(), dt.clone()))
                .collect();
            targets.sort_by(|a, b| a.0.cmp(&b.0));
            for (name, dt) in targets {
                match &dt {
                    DataType::Enum(et) => {
                        if module.enum_members.contains_key(&name) {
                            continue;
                        }
                        let base_width = et
                            .base_type
                            .as_ref()
                            .map(|bt| {
                                resolve_type_width(
                                    bt,
                                    Some(&module.parameters),
                                    Some(&module.typedefs),
                                )
                            })
                            .unwrap_or(32)
                            .max(1);
                        // §6.19.2: a member with no `= expr` continues from the
                        // previous member's value + 1, starting at 0.
                        let mut next: u64 = 0;
                        let mut members: Vec<(String, u64)> = Vec::new();
                        for m in &et.members {
                            let val = match &m.init {
                                Some(init) => {
                                    Self::const_expr_u64(init, &module.parameters).unwrap_or(next)
                                }
                                None => next,
                            };
                            next = val.wrapping_add(1);
                            members.push((m.name.name.clone(), val));
                        }
                        module.typedefs.insert_if_absent(name.clone(), base_width);
                        module.enum_members.insert(name, members);
                    }
                    _ => {
                        let w = resolve_type_width(
                            &dt,
                            Some(&module.parameters),
                            Some(&module.typedefs),
                        )
                        .max(1);
                        module.typedefs.insert_if_absent(name.clone(), w);
                        module.typedef_types.entry(name).or_insert(dt);
                    }
                }
            }
        }
    }

    /// §6.18/§7.2.1: a struct MEMBER declared with a collection typedef
    /// (`typedef addr_t aq_t[$]; struct { aq_t al; } s;`) is the same
    /// dynamic member as `addr_t al[$]`. Every struct-collection path reads
    /// a member's kind from its DECLARATOR dimensions, so rewrite such
    /// members, once, into that direct form in every struct type the
    /// runtime resolves: module/package typedefs, variable types, and the
    /// class property and local typedef types. Otherwise the member was one
    /// element-wide scalar: `aa[k].al.size()` read 0, a pattern item
    /// `al: {1, 2}` kept its last element, and `%p` printed a number.
    pub(super) fn inline_struct_member_coll_typedefs(module: &mut ElaboratedModule) {
        if module.typedef_unpacked_dims.is_empty()
            && module
                .classes
                .values()
                .all(|cd| cd.typedef_unpacked_dims.is_empty())
        {
            return;
        }
        let mut owner_of: HashMap<String, String> = HashMap::default();
        {
            let mut classes: Vec<&String> = module.classes.keys().collect();
            classes.sort();
            for cn in classes {
                for tn in module.classes[cn].typedef_targets.keys() {
                    owner_of.entry(tn.clone()).or_insert_with(|| cn.clone());
                }
            }
        }
        let mut keys: Vec<String> = module.typedef_types.keys().cloned().collect();
        keys.sort();
        for k in keys {
            // A class-scoped key (`C::t`, `P::C::t`) resolves from C.
            if !matches!(module.typedef_types.get(&k), Some(DataType::Struct(su)) if !su.packed) {
                continue;
            }
            // A class-local typedef hoisted under its bare name resolves
            // from its class.
            let start = match owner_of.get(k.as_str()) {
                Some(c) => c.as_str(),
                None => k.rsplit_once("::").map_or("", |(scope, _)| {
                    scope.rsplit_once("::").map_or(scope, |(_, c)| c)
                }),
            };
            if let Some(dt) = module.typedef_types.get(&k)
                && let Some(nd) = Self::inline_member_coll_dt(module, start, dt, 0)
            {
                module.typedef_types.insert(k, nd);
            }
        }
        let mut keys: Vec<String> = module.var_decl_types.keys().cloned().collect();
        keys.sort();
        for k in keys {
            if let Some(dt) = module.var_decl_types.get(&k)
                && let Some(nd) = Self::inline_member_coll_dt(module, "", dt, 0)
            {
                module.var_decl_types.insert(k, nd);
            }
        }
        let mut classes: Vec<String> = module.classes.keys().cloned().collect();
        classes.sort();
        for cn in classes {
            let Some(cd) = module.classes.get(&cn) else {
                continue;
            };
            let props: Vec<(String, DataType)> = cd
                .property_types
                .iter()
                .filter_map(|(n, dt)| {
                    Self::inline_member_coll_dt(module, &cn, dt, 0).map(|d| (n.clone(), d))
                })
                .collect();
            let tds: Vec<(String, DataType)> = cd
                .typedef_targets
                .iter()
                .filter_map(|(n, dt)| {
                    Self::inline_member_coll_dt(module, &cn, dt, 0).map(|d| (n.clone(), d))
                })
                .collect();
            if props.is_empty() && tds.is_empty() {
                continue;
            }
            if let Some(cd) = module.classes.get_mut(&cn).map(std::sync::Arc::make_mut) {
                cd.property_types.extend(props);
                cd.typedef_targets.extend(tds);
            }
        }
    }

    /// The rewrite [`inline_struct_member_coll_typedefs`] makes to one type:
    /// `Some` with the members of an unpacked struct (nested inline structs
    /// included) that name a collection typedef spelled directly, `None`
    /// when nothing changes.
    pub(super) fn inline_member_coll_dt(
        module: &ElaboratedModule,
        start: &str,
        dt: &DataType,
        depth: usize,
    ) -> Option<DataType> {
        let DataType::Struct(su) = dt else {
            return None;
        };
        if su.packed || depth > 8 {
            return None;
        }
        let mut out: Option<crate::ast::types::StructUnionType> = None;
        for (i, m) in su.members.iter().enumerate() {
            let mut nm: Option<crate::ast::types::StructMember> = None;
            if let Some(nd) = Self::inline_member_coll_dt(module, start, &m.data_type, depth + 1) {
                let mut c = m.clone();
                c.data_type = nd;
                nm = Some(c);
            } else if let DataType::TypeReference {
                name, dimensions, ..
            } = &m.data_type
                && dimensions.is_empty()
                && m.declarators.iter().all(|d| d.dimensions.is_empty())
            {
                let from = name.scopes.last().map_or(start, |s| s.name.name.as_str());
                if let Some(dims) = Self::typedef_dims_via_tables(module, from, &name.name.name)
                    && Self::coll_kind_of(&dims).is_some()
                    && let Some(elem) = Self::typedef_elem_via_tables(module, from, name)
                {
                    let mut c = m.clone();
                    c.data_type = elem;
                    for d in &mut c.declarators {
                        d.dimensions = dims.clone();
                    }
                    nm = Some(c);
                }
            }
            if let Some(c) = nm {
                out.get_or_insert_with(|| su.clone()).members[i] = c;
            }
        }
        out.map(DataType::Struct)
    }

    /// The ELEMENT type a dimensioned typedef names (its `typedef_types` /
    /// class `typedef_targets` entry), resolved the way
    /// [`typedef_dims_via_tables`] resolves the dimensions.
    pub(super) fn typedef_elem_via_tables(
        module: &ElaboratedModule,
        start_class: &str,
        tn: &crate::ast::types::TypeName,
    ) -> Option<DataType> {
        let bare = tn.name.name.as_str();
        let mut cur: Option<&str> = Some(start_class);
        while let Some(cn) = cur {
            let Some(cd) = module.classes.get(cn) else {
                break;
            };
            if cd.typedef_unpacked_dims.contains_key(bare) {
                return cd.typedef_targets.get(bare).cloned();
            }
            cur = cd.extends.as_deref();
        }
        if !tn.scopes.is_empty()
            && let Some(dt) = module.typedef_types.get(&tn.qualified())
        {
            return Some(dt.clone());
        }
        module.typedef_types.get(bare).cloned()
    }

    /// §6.18: the dimensioned typedefs the locals of `body` are declared
    /// with, keyed by the type's source spelling (see
    /// `bytecode::type_name_key`). A qualified name resolves from its
    /// innermost class scope; an unqualified one from `cname`'s chain, then
    /// the module/package tables.
    pub(super) fn local_typedef_coll_dims(
        &self,
        cname: &str,
        body: &[Statement],
        strings: &mut HashSet<String>,
    ) -> HashMap<String, Vec<crate::ast::types::UnpackedDimension>> {
        let mut out = HashMap::default();
        let mut stack: Vec<&Statement> = body.iter().collect();
        while let Some(st) = stack.pop() {
            if let StatementKind::VarDecl {
                data_type: DataType::TypeReference { name: tn, .. },
                declarators,
                ..
            } = &st.kind
                && declarators.iter().any(|d| d.dimensions.is_empty())
            {
                let key = super::bytecode::type_name_key(tn);
                if !out.contains_key(&key) {
                    let start = tn.scopes.last().map_or(cname, |s| s.name.name.as_str());
                    if let Some(d) =
                        Self::typedef_dims_via_tables(&self.module, start, &tn.name.name)
                            .filter(|d| !d.is_empty())
                    {
                        // §6.16: a collection of `string` elements
                        // (`typedef string q_t[$]`) reads strings.
                        let elem = Self::typedef_elem_via_tables(&self.module, start, tn);
                        let elem = match elem {
                            Some(DataType::TypeReference { name, .. }) => {
                                self.module.typedef_types.get(&name.name.name).cloned()
                            }
                            e => e,
                        };
                        if matches!(
                            elem,
                            Some(DataType::Simple {
                                kind: crate::ast::types::SimpleType::String,
                                ..
                            })
                        ) {
                            strings.insert(key.clone());
                        }
                        out.insert(key, d);
                    }
                }
            }
            stack.extend(Self::sub_stmts(st));
        }
        out
    }

    /// Unpacked dims carried by the typedef `tn`, resolved the way §6.18
    /// scoping does: the class `start_class` and its ancestors first (their
    /// `typedef_unpacked_dims`, exported by core dd4e161), then the
    /// module/package table, following typedef alias chains. `None` when `tn`
    /// is not a dimensioned typedef anywhere in scope.
    pub(super) fn typedef_dims_via_tables(
        module: &ElaboratedModule,
        start_class: &str,
        tn0: &str,
    ) -> Option<Vec<crate::ast::types::UnpackedDimension>> {
        let mut tn = tn0.to_string();
        for _ in 0..8 {
            let mut cur: Option<&str> = Some(start_class);
            while let Some(cn) = cur {
                let Some(cd) = module.classes.get(cn) else {
                    break;
                };
                if let Some(d) = cd.typedef_unpacked_dims.get(&tn) {
                    return Some(d.clone());
                }
                cur = cd.extends.as_deref();
            }
            if let Some(d) = module.typedef_unpacked_dims.get(&tn) {
                return Some(d.clone());
            }
            match module.typedef_types.get(&tn) {
                Some(DataType::TypeReference {
                    name, dimensions, ..
                }) if dimensions.is_empty() => {
                    tn = name.name.name.clone();
                }
                _ => break,
            }
        }
        None
    }

    /// §6.18: classify class PROPERTIES declared with a typedef'd collection
    /// type (`typedef int q_t[$]; … q_t m_q;`) into the
    /// queue/assoc/array-property maps. `elaborate_class` resolves such dims
    /// only from the class's OWN local typedefs; a MODULE/package-level
    /// typedef (or one inherited from an ancestor class) left the property a
    /// scalar. The member then got no per-instance `<handle>#<member>`
    /// storage: `push_back` inside a method landed on an unregistered bare
    /// global, `foreach` over the member missed the data entirely and
    /// iterated a junk fallback, and a `ref`-writeback caller saw one zero
    /// element instead of the pushed contents.
    /// §7.4.1: a packed-struct MEMBER that is itself a packed multi-D vector
    /// (`struct packed { bit [7:0][7:0] a; ... } foo;`) needs `foo.a` in the
    /// packed-element tables, or `foo.a[5] = v` degrades to a single-BIT
    /// select (and, worse, silently no-ops through the struct fallback while
    /// the RANGE form `foo.a[2:1]` works — which is exactly how it hid).
    /// Derives each member's element shape from the declared struct type and
    /// registers the dotted path alongside the flattened field layout.
    pub(super) fn register_struct_member_packed_dims(module: &mut ElaboratedModule) {
        use crate::ast::types::DataType;
        let vars: Vec<String> = module.packed_struct_fields.keys().cloned().collect();
        for var in vars {
            let Some(dt) = module.var_decl_types.get(&var).cloned() else {
                continue;
            };
            // Walk NESTED packed structs, not just the top level: a member whose
            // own type is a struct (or a packed array of structs) carries members
            // of its own, and `s.outer[i].inner[k]` needs `inner`'s element
            // stride to place the trailing select. Registering only one level
            // deep left that stride unknown, so the select silently read bit 0.
            let mut pending: Vec<(String, DataType)> = vec![(var, dt)];
            let mut budget = 4096usize;
            while let Some((prefix, dt)) = pending.pop() {
                if budget == 0 {
                    break;
                }
                budget -= 1;
                let resolved = match &dt {
                    DataType::TypeReference { name, .. } => module
                        .typedef_types
                        .get(&name.name.name)
                        .cloned()
                        .unwrap_or(dt.clone()),
                    _ => dt.clone(),
                };
                let DataType::Struct(su) = resolved else {
                    continue;
                };
                if !su.packed {
                    continue;
                }
                for member in &su.members {
                    let ew = super::elaborate::packed_inner_elem_width(
                        &member.data_type,
                        &module.parameters,
                        &module.typedefs,
                    );
                    let fdims = super::elaborate::packed_full_dims_of(
                        &member.data_type,
                        &module.parameters,
                    );
                    for mdecl in &member.declarators {
                        let key = format!("{}.{}", prefix, mdecl.name.name);
                        if let Some(ew) = ew {
                            module
                                .packed_signal_elem_widths
                                .entry(key.clone())
                                .or_insert(ew);
                        }
                        if let Some(fd) = &fdims {
                            module
                                .packed_full_dims
                                .entry(key.clone())
                                .or_insert_with(|| fd.clone());
                        }
                        pending.push((key, member.data_type.clone()));
                    }
                }
            }
        }
    }

    /// §26.3: the storage name of package `pkg`'s variable `name` when the
    /// bare name is contested (see `split_contested_package_vars`), else
    /// `None` (the bare hoisted entry is the package's own).
    pub(super) fn pkg_var_storage(&self, pkg: &str, name: &str) -> Option<String> {
        if self.contested_pkg_vars.is_empty() || !self.contested_pkg_vars.contains(name) {
            return None;
        }
        let q = format!("{}::{}", pkg, name);
        self.signal_name_to_id.contains_key(q.as_str()).then_some(q)
    }

    /// §26.3: a package variable whose bare name another package also
    /// declares, or that a module-scope declaration displaces, gets its own
    /// storage under `<pkg>::<name>`. Package declarations are hoisted into
    /// ONE design-wide bare namespace, so `pa::cnt` and `pc::cnt` were the
    /// same cell (and `base_p::v` read the module's own `v`). The qualified
    /// cells are added as signals here, a call-initialized one's static
    /// initializer is retargeted to it, and the bare names are returned so
    /// name resolution can route through the qualified cells.
    pub(super) fn split_contested_package_vars(module: &mut ElaboratedModule) -> HashSet<String> {
        let mut owners: HashMap<&str, usize> = HashMap::default();
        for key in module.pkg_var_decls.keys() {
            if let Some((_, base)) = key.split_once("::") {
                *owners.entry(base).or_default() += 1;
            }
        }
        let contested: HashSet<String> = owners
            .into_iter()
            .filter(|(base, n)| {
                *n >= 2
                    || module.wildcard_shadowed.contains(*base)
                    || module.foreign_displaced.contains(*base)
                    // A package first hoisted after the local declaration
                    // never needed to displace its signal during elaboration.
                    || module.foreign_package_names.contains(*base)
            })
            .map(|(b, _)| b.to_string())
            .collect();
        // §15.5 / §26.3: package events have the same synchronization identity
        // through scoped and imported spellings. Prefer owner-specific type
        // metadata; old artifacts can safely use the bare type only if unique.
        let mut package_events = Vec::new();
        for qualified in module.pkg_var_decls.keys() {
            let Some((_, leaf)) = qualified.split_once("::") else {
                continue;
            };
            let dt = module.var_decl_types.get(qualified).or_else(|| {
                (!contested.contains(leaf))
                    .then(|| module.var_decl_types.get(leaf))
                    .flatten()
            });
            if dt.is_some_and(|dt| {
                matches!(
                    super::elaborate::resolve_typedef_chain(dt, &module.typedef_types),
                    DataType::Simple {
                        kind: crate::ast::types::SimpleType::Event,
                        ..
                    }
                )
            }) {
                package_events.push((qualified.clone(), leaf.to_string()));
            }
        }
        for (qualified, leaf) in package_events {
            module.events.insert(qualified.clone());
            if !contested.contains(&leaf) {
                module.events.insert(leaf);
            }
        }
        if contested.is_empty() {
            return contested;
        }
        let mut keys: Vec<String> = module
            .pkg_var_decls
            .keys()
            .filter(|k| {
                k.split_once("::")
                    .is_some_and(|(_, b)| contested.contains(b))
            })
            .cloned()
            .collect();
        keys.sort();
        for k in keys {
            if let Some(sig) = module.pkg_var_decls.get(&k).cloned() {
                let (_, base) = k.split_once("::").unwrap();
                if let Some(ty) = module.var_decl_types.get(base).cloned() {
                    module.var_decl_types.entry(k.clone()).or_insert(ty);
                }
                if module.string_signals.contains(base) {
                    module.string_signals.insert(k.clone());
                }
                if module.two_state_signals.contains(base) {
                    module.two_state_signals.insert(k.clone());
                }
                module.signals.insert(k, sig);
            }
        }
        for (idx, q) in module.pkg_var_init_blocks.clone() {
            let Some((_, base)) = q.split_once("::") else {
                continue;
            };
            if !contested.contains(base) {
                continue;
            }
            if let Some(b) = module.static_init_blocks.get_mut(idx) {
                if let StatementKind::BlockingAssign { lvalue, .. } = &mut b.stmt.kind {
                    *lvalue = Expression::new(
                        ExprKind::Ident(HierarchicalIdentifier {
                            root: None,
                            path: vec![crate::ast::expr::HierPathSegment {
                                name: crate::ast::Identifier {
                                    name: q.clone(),
                                    span: lvalue.span,
                                },
                                selects: Vec::new(),
                            }],
                            span: lvalue.span,
                            cached_signal_id: std::cell::Cell::new(None),
                            cached_resolved_name: std::cell::OnceCell::new(),
                        }),
                        lvalue.span,
                    );
                }
            }
        }
        contested
    }

    pub(super) fn classify_typedef_collection_properties(module: &mut ElaboratedModule) {
        use crate::ast::types::{SimpleType, UnpackedDimension as UD};
        let mut classes: Vec<String> = module.classes.keys().cloned().collect();
        classes.sort();
        let params = module.parameters.clone();
        for cn in classes {
            let Some(cd) = module.classes.get(&cn) else {
                continue;
            };
            // (prop, width, is_static, dims), gathered immutably first.
            let mut found: Vec<(String, u32, bool, Vec<UD>)> = Vec::new();
            for (pname, sig) in &cd.properties {
                if cd.queue_properties.contains_key(pname)
                    || cd.assoc_properties.contains_key(pname)
                    || cd.array_properties.contains_key(pname)
                    || cd.array_nd_properties.contains_key(pname)
                    || cd.static_collections.iter().any(|(n, ..)| n == pname)
                    || cd.static_fixed_arrays.iter().any(|(n, ..)| n == pname)
                {
                    continue;
                }
                let Some(tn) = &sig.type_name else { continue };
                let Some(dims) = Self::typedef_dims_via_tables(module, &cn, tn) else {
                    continue;
                };
                found.push((
                    pname.clone(),
                    sig.width.max(1),
                    cd.static_properties.contains(pname),
                    dims,
                ));
            }
            if found.is_empty() {
                continue;
            }
            found.sort_by(|a, b| a.0.cmp(&b.0));
            let Some(cd) = module.classes.get_mut(&cn).map(std::sync::Arc::make_mut) else {
                continue;
            };
            for (pname, w, is_static, dims) in found {
                // Mirrors elaborate_class's `effective_dims` classification,
                // but with the live parameter table for const bounds.
                if is_static {
                    match dims.first() {
                        Some(UD::Associative { .. }) => {
                            cd.static_collections.push((pname, true, w));
                        }
                        Some(UD::Queue { .. }) | Some(UD::Unsized(_)) => {
                            cd.static_collections.push((pname, false, w));
                        }
                        // Fixed-size static array reached through a typedef
                        // (`typedef int arr_t[3]; static arr_t S;`) — same
                        // shared-store treatment as the direct declaration
                        // (see `static_fixed_arrays` in elaborate.rs).
                        Some(UD::Expression { expr, .. }) => {
                            if let Some(n) =
                                super::elaborate::const_eval_i64_with_params(expr, Some(&params))
                            {
                                if n > 0 {
                                    cd.static_fixed_arrays.push((pname, 0, n - 1, w));
                                }
                            }
                        }
                        Some(UD::Range { left, right, .. }) => {
                            if let (Some(l), Some(r)) = (
                                super::elaborate::const_eval_i64_with_params(left, Some(&params)),
                                super::elaborate::const_eval_i64_with_params(right, Some(&params)),
                            ) {
                                cd.static_fixed_arrays.push((pname, l.min(r), l.max(r), w));
                            }
                        }
                        _ => {}
                    }
                    continue;
                }
                // Multi-dim fixed member via typedef dims: record the FULL
                // shape (the `first()`-only match below drops inner dims).
                if dims.len() >= 2 {
                    let shape: Option<Vec<(i64, i64)>> = dims
                        .iter()
                        .map(|dm| match dm {
                            UD::Range { left, right, .. } => match (
                                super::elaborate::const_eval_i64_with_params(left, Some(&params)),
                                super::elaborate::const_eval_i64_with_params(right, Some(&params)),
                            ) {
                                (Some(l), Some(r)) => Some((l.min(r), l.max(r))),
                                _ => None,
                            },
                            UD::Expression { expr, .. } => {
                                match super::elaborate::const_eval_i64_with_params(
                                    expr,
                                    Some(&params),
                                ) {
                                    Some(n) if n > 0 => Some((0, n - 1)),
                                    _ => None,
                                }
                            }
                            _ => None,
                        })
                        .collect();
                    if let Some(shape) = shape {
                        cd.array_nd_properties.insert(pname, (shape, w));
                        continue;
                    }
                }
                match dims.first() {
                    Some(UD::Associative {
                        data_type: key_dt, ..
                    }) => {
                        let is_string_key = key_dt.as_ref().is_some_and(|dt| {
                            matches!(
                                dt.as_ref(),
                                DataType::Simple {
                                    kind: SimpleType::String,
                                    ..
                                }
                            )
                        });
                        cd.assoc_properties.insert(pname, is_string_key);
                    }
                    Some(UD::Queue { max_size, .. }) => {
                        cd.queue_property_names.insert(pname.clone());
                        let cap = max_size
                            .as_ref()
                            .and_then(|e| {
                                super::elaborate::const_eval_i64_with_params(e, Some(&params))
                            })
                            .map(|n| (n + 1).max(1) as u32);
                        cd.queue_properties.insert(pname, (w, cap));
                    }
                    Some(UD::Unsized(_)) => {
                        cd.queue_properties.insert(pname, (w, None));
                    }
                    Some(UD::Expression { expr, .. }) => {
                        match super::elaborate::const_eval_i64_with_params(expr, Some(&params)) {
                            Some(n) if n > 0 => {
                                cd.array_properties.insert(pname, (0, n - 1, w));
                            }
                            _ => {
                                cd.queue_properties.insert(pname, (w, None));
                            }
                        }
                    }
                    Some(UD::Range { left, right, .. }) => {
                        if let (Some(l), Some(r)) = (
                            super::elaborate::const_eval_i64_with_params(left, Some(&params)),
                            super::elaborate::const_eval_i64_with_params(right, Some(&params)),
                        ) {
                            cd.array_properties.insert(pname, (l.min(r), l.max(r), w));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Minimal constant folder for an enum member's `= expr` initializer at
    /// construction time (before any `Simulator` exists): literals, named
    /// parameters/enum constants already registered, and the arithmetic that
    /// shows up in enum encodings.
    pub(super) fn const_expr_u64(e: &Expression, params: &HashMap<String, Value>) -> Option<u64> {
        match &e.kind {
            ExprKind::Number(NumberLiteral::Integer {
                size,
                base,
                value,
                cached_val,
                ..
            }) => {
                let w = size.unwrap_or(32);
                if let Some((vb, xz, cw)) = cached_val.get() {
                    if cw == w {
                        return Value::from_inline(vb, xz, w).to_u64();
                    }
                }
                let r = match base {
                    NumberBase::Binary => 2,
                    NumberBase::Octal => 8,
                    NumberBase::Hex => 16,
                    NumberBase::Decimal => 10,
                };
                Value::from_str_radix(value, r, w).to_u64()
            }
            ExprKind::Paren(inner) => Self::const_expr_u64(inner, params),
            ExprKind::Ident(h) => params
                .get(&h.path.last()?.name.name)
                .and_then(|v| v.to_u64()),
            ExprKind::Unary { op, operand } => {
                let v = Self::const_expr_u64(operand, params)?;
                match op {
                    UnaryOp::Minus => Some(v.wrapping_neg()),
                    UnaryOp::Plus => Some(v),
                    UnaryOp::BitNot => Some(!v),
                    _ => None,
                }
            }
            ExprKind::Binary { op, left, right } => {
                let l = Self::const_expr_u64(left, params)?;
                let r = Self::const_expr_u64(right, params)?;
                match op {
                    BinaryOp::Add => Some(l.wrapping_add(r)),
                    BinaryOp::Sub => Some(l.wrapping_sub(r)),
                    BinaryOp::Mul => Some(l.wrapping_mul(r)),
                    BinaryOp::Div if r != 0 => Some(l / r),
                    BinaryOp::Mod if r != 0 => Some(l % r),
                    BinaryOp::ShiftLeft | BinaryOp::ArithShiftLeft => {
                        Some(l.checked_shl(r as u32).unwrap_or(0))
                    }
                    BinaryOp::ShiftRight | BinaryOp::ArithShiftRight => {
                        Some(l.checked_shr(r as u32).unwrap_or(0))
                    }
                    BinaryOp::BitOr => Some(l | r),
                    BinaryOp::BitAnd => Some(l & r),
                    BinaryOp::BitXor => Some(l ^ r),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub(super) fn materialize_implicit_contassign_nets(module: &mut ElaboratedModule) {
        fn ident_raw(expr: &Expression) -> Option<String> {
            if let ExprKind::Ident(hier) = &expr.kind {
                Some(
                    hier.path
                        .iter()
                        .map(|s| s.name.name.as_str())
                        .collect::<Vec<_>>()
                        .join("."),
                )
            } else {
                None
            }
        }

        fn ident_leaf(expr: &Expression) -> Option<String> {
            if let ExprKind::Ident(hier) = &expr.kind {
                if hier.path.len() == 1 {
                    let name = hier.path[0].name.name.clone();
                    if !name.contains('.') && !name.contains('[') {
                        return Some(name);
                    }
                }
            }
            None
        }

        fn parent_n(name: &str, n: usize) -> Option<String> {
            let mut cur = name.rsplit_once('.').map(|(p, _)| p.to_string())?;
            for _ in 0..n {
                cur = cur.rsplit_once('.').map(|(p, _)| p.to_string())?;
            }
            Some(cur)
        }

        fn best_scope_for_leaves(
            leaves: &HashSet<String>,
            existing: &HashSet<&str>,
            parents_by_leaf: &HashMap<&str, Vec<&str>>,
        ) -> Option<String> {
            let mut best_parent: Option<String> = None;
            let mut best_score = 0usize;
            let mut best_depth = 0usize;
            for leaf in leaves {
                let Some(parents) = parents_by_leaf.get(leaf.as_str()) else {
                    continue;
                };
                for parent in parents {
                    let mut score = 0usize;
                    for candidate_leaf in leaves {
                        let candidate = format!("{}.{}", parent, candidate_leaf);
                        if existing.contains(candidate.as_str()) {
                            score += 1;
                        }
                    }
                    let depth = parent.split('.').count();
                    if score > best_score
                        || (score == best_score && depth > best_depth)
                        || (score == best_score
                            && depth == best_depth
                            && best_parent.as_ref().is_none_or(|p| parent.len() > p.len()))
                    {
                        best_parent = Some(parent.to_string());
                        best_score = score;
                        best_depth = depth;
                    }
                }
            }
            if best_parent.is_some() {
                best_parent
            } else {
                None
            }
        }

        fn collect_lhs_leaves(expr: &Expression, out: &mut HashSet<String>) {
            match &expr.kind {
                ExprKind::Ident(hier) => {
                    let name = hier
                        .path
                        .iter()
                        .map(|s| s.name.name.as_str())
                        .collect::<Vec<_>>()
                        .join(".");
                    out.insert(name);
                }
                ExprKind::Index { expr: base, .. } | ExprKind::RangeSelect { expr: base, .. } => {
                    collect_lhs_leaves(base, out);
                }
                ExprKind::Concatenation(items) => {
                    for item in items {
                        collect_lhs_leaves(item, out);
                    }
                }
                ExprKind::Paren(inner) => collect_lhs_leaves(inner, out),
                _ => {}
            }
        }

        // Borrowed from the module's maps (no copy of every name); built in
        // the same insertion order, so the sets iterate, and the parent
        // lists come out, exactly as owned copies would.
        let mut existing: HashSet<&str> = module.signals.keys().map(String::as_str).collect();
        existing.extend(module.parameters.keys().map(String::as_str));
        let mut parents_by_leaf: HashMap<&str, Vec<&str>> = HashMap::default();
        for &full_name in &existing {
            if let Some((parent, leaf)) = full_name.rsplit_once('.') {
                parents_by_leaf.entry(leaf).or_default().push(parent);
            }
        }

        let mut to_create: HashSet<String> = HashSet::default();
        for ca in &module.continuous_assigns {
            let mut leaves = HashSet::default();
            let mut reads = HashSet::default();
            let mut writes = HashSet::default();
            Self::collect_expr_reads(&ca.rhs, module, &mut reads);
            collect_lhs_leaves(&ca.lhs, &mut writes);
            for name in reads.iter().chain(writes.iter()) {
                if !name.contains('.') && !name.contains('[') {
                    leaves.insert(name.clone());
                }
            }
            // Only a bare (unscoped) leaf can need an implicit net; a
            // flattened instance's assigns name none.
            if leaves.is_empty() {
                continue;
            }
            let lhs_raw = ident_raw(&ca.lhs);
            let rhs_raw = ident_raw(&ca.rhs);
            let lhs_leaf = ident_leaf(&ca.lhs);
            let rhs_leaf = ident_leaf(&ca.rhs);

            let scope = if lhs_leaf.is_none() && rhs_leaf.is_some() {
                lhs_raw.as_deref().and_then(|raw| parent_n(raw, 1))
            } else if lhs_leaf.is_some() && rhs_leaf.is_none() {
                rhs_raw.as_deref().and_then(|raw| parent_n(raw, 1))
            } else {
                best_scope_for_leaves(&leaves, &existing, &parents_by_leaf)
            };

            let Some(scope) = scope else {
                continue;
            };
            for leaf in leaves {
                if existing.contains(leaf.as_str()) {
                    continue;
                }
                let qualified = format!("{}.{}", scope, leaf);
                if !existing.contains(qualified.as_str()) {
                    to_create.insert(qualified);
                }
            }
        }

        drop(parents_by_leaf);
        drop(existing);
        for name in to_create {
            module.signals.insert(
                name.clone(),
                Signal {
                    name,
                    width: 1,
                    is_signed: false,
                    is_real: false,
                    is_const: false,
                    direction: None,
                    value: Value::new(1),
                    type_name: None,
                },
            );
        }
    }

    /// §9.4.2: an edge term that SELECTS into a signal — `posedge vec[1]`,
    /// or the C910 PLIC shape `posedge arr[i][j]` (bit j of unpacked element
    /// i) from an inlined `.clk(arr[i][j])` port connection — cannot be
    /// expressed by the signal-id edge machinery, which watches a whole
    /// signal's LSB. The base-name fallback silently watched the WRONG BIT
    /// (or, for an unpacked base, resolved to no signal at all and the term
    /// was dropped — a dead clock: C910's PLIC IE registers never latched
    /// and cmark hung on the retire watchdog).
    ///
    /// Rewrite each such term to a synthesized 1-bit alias net driven by the
    /// select expression: `assign __xz_edgesel<N> = arr[i][j];` and
    /// `posedge __xz_edgesel<N>`. The alias updates in the same delta as its
    /// operands (ordinary comb entry), so edge timing is unchanged. Only
    /// constant indices rewrite; anything else keeps the old behavior.
    pub(super) fn rewrite_edge_select_sensitivities(
        module: &mut ElaboratedModule,
        lazy_ok: bool,
    ) -> Vec<DeferredAlways> {
        // A literal, or a parameter expression (`v[W-1]`: an elaboration-time
        // constant, §6.20) under `+ - *`.
        fn const_index(
            e: &Expression,
            params: &HashMap<String, Value>,
            scope: &str,
        ) -> Option<i64> {
            match &e.kind {
                ExprKind::Number(NumberLiteral::Integer { value, .. }) => {
                    value.replace('_', "").parse::<i64>().ok()
                }
                ExprKind::Paren(inner) => const_index(inner, params, scope),
                ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => {
                    let raw = h
                        .path
                        .iter()
                        .map(|s| s.name.name.as_str())
                        .collect::<Vec<_>>()
                        .join(".");
                    let v = (!scope.is_empty())
                        .then(|| params.get(&format!("{}.{}", scope, raw)))
                        .flatten()
                        .or_else(|| params.get(&raw))?;
                    if v.has_xz() {
                        return None;
                    }
                    i64::try_from(v.to_u64()?).ok()
                }
                ExprKind::Binary { op, left, right } => {
                    let l = const_index(left, params, scope)?;
                    let r = const_index(right, params, scope)?;
                    match op {
                        BinaryOp::Add => l.checked_add(r),
                        BinaryOp::Sub => l.checked_sub(r),
                        BinaryOp::Mul => l.checked_mul(r),
                        _ => None,
                    }
                }
                _ => None,
            }
        }
        // Peel an Index chain down to its base Ident; indices returned
        // OUTERMOST-LAST (declaration order). The flag tells a parent-rooted
        // base (a substituted port actual, see `mark_actual_rooted`), which
        // names the absolute signal and must not be tried under the block's
        // own scope.
        fn peel<'a>(
            mut e: &'a Expression,
            idxs: &mut Vec<i64>,
            params: &HashMap<String, Value>,
            scope: &str,
        ) -> Option<(String, bool)> {
            loop {
                match &e.kind {
                    ExprKind::Paren(inner) => e = inner,
                    ExprKind::Index { expr, index } => {
                        idxs.push(const_index(index, params, scope)?);
                        e = expr;
                    }
                    ExprKind::Ident(h) => {
                        return Some((
                            h.path
                                .iter()
                                .map(|s| s.name.name.as_str())
                                .collect::<Vec<_>>()
                                .join("."),
                            h.root.is_some(),
                        ));
                    }
                    _ => return None,
                }
            }
        }
        let mk_ident = |name: &str, span: crate::ast::Span| -> Expression {
            Expression::new(
                ExprKind::Ident(HierarchicalIdentifier {
                    root: None,
                    path: vec![HierPathSegment {
                        name: crate::ast::Identifier {
                            name: name.to_string(),
                            span,
                        },
                        selects: Vec::new(),
                    }],
                    span,
                    cached_signal_id: std::cell::Cell::new(None),
                    cached_resolved_name: std::cell::OnceCell::new(),
                }),
                span,
            )
        };
        let mut aliases: HashMap<String, String> = HashMap::default(); // canon expr -> alias
        // Per vector: the bit ranges whose bits are always equal (see
        // `replicated_bit_ranges`), computed on first use.
        let mut repl_cache: HashMap<String, Vec<(i64, i64)>> = HashMap::default();
        let mut writer_index: Option<edgesel_writers::WriterIndex> = None;
        let mut new_assigns: Vec<super::elaborate::ContinuousAssignment> = Vec::new();
        let mut new_signals: Vec<String> = Vec::new();
        let dbg = std::env::var_os("XEZIM_DUMP_EDGE_SENS").is_some();
        // #7 keeps child always blocks PENDING (un-rewritten) until
        // classification — but the select-on-port shape this pass fixes is
        // only visible AFTER the per-instance rewrite. Each pending block is
        // taken out in order into the returned list: one of the `@(...)
        // body` shape whose body holds no further event control stays lazy
        // (only its header is rewritten here, and the terms this pass
        // replaces are recorded as patches); any other is materialized.
        let pending = std::mem::take(&mut module.pending_always);
        let mut always = std::mem::take(&mut module.always_blocks);
        let mut deferred: Vec<DeferredAlways> = Vec::with_capacity(pending.len());
        for p in pending {
            if lazy_ok && Self::lazy_always_eligible(&p.source) {
                if let Some(header) = p.materialize_header() {
                    deferred.push(DeferredAlways::Lazy {
                        lazy: LazyAlways {
                            src: p,
                            patches: Vec::new(),
                        },
                        header: Some(Box::new(header)),
                        patched: Vec::new(),
                    });
                    continue;
                }
            }
            deferred.push(DeferredAlways::Eager(Self::materialize_pending_always(p)));
        }
        let params = &module.parameters;
        let headers = always
            .iter_mut()
            .map(|ab| (&mut ab.stmt, ab.scope.clone(), None))
            .chain(deferred.iter_mut().map(|d| {
                match d {
                    DeferredAlways::Eager(ab) => (&mut ab.stmt, ab.scope.clone(), None),
                    DeferredAlways::Lazy {
                        lazy,
                        header,
                        patched,
                    } => (
                        &mut **header
                            .as_mut()
                            .expect("header kept until Simulator::new ends"),
                        lazy.src.scope(),
                        Some(patched),
                    ),
                }
            }));
        for (stmt, scope, mut patched) in headers {
            let StatementKind::TimingControl {
                control: TimingControl::Event(EventControl::EventExpr(exprs)),
                ..
            } = &mut stmt.kind
            else {
                if dbg {
                    eprintln!(
                        "[EDGESEL] block scope='{}' not a TimingControl/EventExpr: {:?}",
                        scope,
                        std::mem::discriminant(&stmt.kind)
                    );
                }
                continue;
            };
            for (term, ee) in exprs.iter_mut().enumerate() {
                // Port substitution leaves `@(dq[3])` as a select of the
                // connection (`bus[15:8][3]`); spell such a term as the net
                // bit it reads first (see `event_bits`).
                if let Some(f) =
                    event_bits::fold_event_term(module, &ee.expr, ee.edge.is_some(), &scope)
                {
                    ee.expr = f;
                    if let Some(p) = patched.as_mut() {
                        p.push(term);
                    }
                }
                if !matches!(ee.edge, Some(Edge::Posedge) | Some(Edge::Negedge)) {
                    continue;
                }
                let mut idxs: Vec<i64> = Vec::new();
                let Some((base, rooted)) = peel(&ee.expr, &mut idxs, params, &scope) else {
                    if dbg {
                        eprintln!(
                            "[EDGESEL] scope='{}' peel failed: {:?}",
                            scope, ee.expr.kind
                        );
                    }
                    continue;
                };
                if dbg {
                    eprintln!("[EDGESEL] scope='{}' base={} idxs={:?}", scope, base, idxs);
                }
                if idxs.is_empty() {
                    continue;
                }
                idxs.reverse(); // declaration order: base[idxs[0]][idxs[1]]…
                // The base must name a real signal or unpacked array (either
                // spelled as-is post-inline or under the block's scope);
                // events keep their own element machinery.
                // §23.3.3: `.wl(wl[1])` into a child whose own port is also
                // `wl` substitutes the ROOTED `wl[1]`; under the child's scope
                // it spelled the child's 1-bit port net, and the edge watched
                // bit 1 of it — never rising.
                let spellings: Vec<String> = if scope.is_empty() || rooted {
                    vec![base.clone()]
                } else {
                    vec![format!("{}.{}", scope, base), base.clone()]
                };
                let mut resolved: Option<String> = None;
                let mut is_event = false;
                'sp: for sp in &spellings {
                    let mut name = sp.clone();
                    if module.events.contains(name.as_str()) {
                        is_event = true;
                        break;
                    }
                    for &ix in &idxs {
                        name = format!("{}[{}]", name, ix);
                        if module.events.contains(name.as_str()) {
                            is_event = true;
                            break 'sp;
                        }
                    }
                    if module.signals.contains_key(sp.as_str())
                        || module.arrays.contains_key(sp.as_str())
                    {
                        resolved = Some(sp.clone());
                        break;
                    }
                }
                if is_event {
                    continue;
                }
                let Some(resolved) = resolved else {
                    if dbg {
                        let leaf = base.rsplit('.').next().unwrap_or(&base);
                        let sig_like: Vec<&String> = module
                            .signals
                            .keys()
                            .filter(|k| k.ends_with(leaf))
                            .take(3)
                            .collect();
                        let arr_like: Vec<&String> = module
                            .arrays
                            .keys()
                            .filter(|k| k.ends_with(leaf))
                            .take(3)
                            .collect();
                        eprintln!(
                            "[EDGESEL] scope='{}' base '{}' unresolved; signals~{:?} arrays~{:?}",
                            scope, base, sig_like, arr_like
                        );
                    }
                    continue;
                };
                // Skip the shape that already works: ONE index on an
                // unpacked array whose element IS the watched signal.
                if idxs.len() == 1 && module.arrays.contains_key(resolved.as_str()) {
                    continue;
                }
                // §10.3.2: bits written by one `assign v[A +: N] = {N{b}}`
                // (one-bit `b`, no other driver) are equal at every moment
                // and change in the same delta, so an edge on any of them is
                // an edge on the range's lowest bit at the same time. Edge
                // terms on such bits share ONE alias instead of one per bit.
                if idxs.len() == 1 && module.signals.contains_key(resolved.as_str()) {
                    let ranges = repl_cache.entry(resolved.clone()).or_insert_with(|| {
                        let index = writer_index
                            .get_or_insert_with(|| edgesel_writers::WriterIndex::build(module));
                        Self::replicated_bit_ranges(module, &resolved, index)
                    });
                    if let Some(&(lo, _)) = ranges
                        .iter()
                        .find(|(lo, hi)| *lo <= idxs[0] && idxs[0] <= *hi)
                    {
                        if dbg && lo != idxs[0] {
                            eprintln!(
                                "[EDGESEL] {}[{}] shares the alias of bit {}",
                                resolved, idxs[0], lo
                            );
                        }
                        idxs[0] = lo;
                    }
                }
                let canon = format!("{}|{:?}", resolved, idxs);
                let span = ee.expr.span;
                // The alias RHS is the ORIGINAL select expression (with the
                // base respelled to the resolved name) — the cont-assign
                // evaluator handles array-element and bit selects natively.
                let mut rhs = mk_ident(&resolved, span);
                for &ix in &idxs {
                    rhs = Expression::new(
                        ExprKind::Index {
                            expr: Box::new(rhs),
                            index: Box::new(Expression::new(
                                ExprKind::Number(NumberLiteral::Integer {
                                    size: None,
                                    signed: false,
                                    base: NumberBase::Decimal,
                                    value: ix.to_string(),
                                    cached_val: std::cell::Cell::new(None),
                                }),
                                span,
                            )),
                        },
                        span,
                    );
                }
                let alias = aliases.entry(canon).or_insert_with(|| {
                    let alias = format!("__xz_edgesel{}", new_signals.len());
                    new_signals.push(alias.clone());
                    new_assigns.push(super::elaborate::ContinuousAssignment {
                        origin: None,
                        lhs: mk_ident(&alias, span),
                        rhs,
                        delay: 0,
                        rhs_parent_scoped: false,
                        delay_fall: None,
                        delay_off: None,
                    });
                    alias
                });
                if dbg {
                    eprintln!(
                        "[EDGESEL] scope='{}' {}{:?} -> {}",
                        scope, resolved, idxs, alias
                    );
                }
                ee.expr = mk_ident(alias, ee.expr.span);
                if let Some(p) = patched.as_mut() {
                    p.push(term);
                }
            }
        }
        for name in new_signals {
            module.signals.insert(
                name.clone(),
                Signal {
                    name,
                    width: 1,
                    is_signed: false,
                    is_real: false,
                    is_const: false,
                    direction: None,
                    value: Value::new(1),
                    type_name: None,
                },
            );
        }
        module.continuous_assigns.extend(new_assigns);
        module.always_blocks = always;
        deferred
    }

    /// Bit ranges of the packed vector `v` that hold one replicated bit:
    /// each range is written by exactly one continuous assignment of the
    /// form `v[lo..=hi] = {N{b}}` with `b` one bit wide and `N` the range
    /// width, and no other continuous assignment writes any of its bits
    /// (a net may have several drivers, which resolve; then the bits need
    /// not be equal). Within such a range every bit is the same value at
    /// every moment, written by the same evaluation. Both the elaborated
    /// assignments and the sub-module ones still pending materialization
    /// are searched; a pending one is rewritten only when its target can
    /// name `v` (same leaf name, directly or through a port connection).
    pub(super) fn replicated_bit_ranges(
        module: &ElaboratedModule,
        v: &str,
        index: &edgesel_writers::WriterIndex,
    ) -> Vec<(i64, i64)> {
        use edgesel_writers::{base_ident, leaf};
        fn const_val(e: &Expression, params: &HashMap<String, Value>) -> Option<i64> {
            match &e.kind {
                ExprKind::Number(NumberLiteral::Integer { value, .. }) => {
                    value.replace('_', "").parse::<i64>().ok()
                }
                ExprKind::Paren(inner) => const_val(inner, params),
                ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => {
                    let raw = h
                        .path
                        .iter()
                        .map(|s| s.name.name.as_str())
                        .collect::<Vec<_>>()
                        .join(".");
                    let v = params.get(&raw)?;
                    if v.has_xz() {
                        return None;
                    }
                    i64::try_from(v.to_u64()?).ok()
                }
                ExprKind::Binary { op, left, right } => {
                    let l = const_val(left, params)?;
                    let r = const_val(right, params)?;
                    match op {
                        BinaryOp::Add => l.checked_add(r),
                        BinaryOp::Sub => l.checked_sub(r),
                        BinaryOp::Mul => l.checked_mul(r),
                        _ => None,
                    }
                }
                _ => None,
            }
        }
        let params = &module.parameters;
        let vleaf = leaf(v);
        // §10.6.2: a `force` of one bit makes it differ from the others.
        if index.forced.contains(vleaf) {
            return Vec::new();
        }
        // (lo, hi, replicated) per write of `v`; None = a write of unknown
        // extent (concatenation target, dynamic select).
        let mut writes: Vec<Option<(i64, i64, bool)>> = Vec::new();
        let mut classify = |lhs: &Expression, rhs: Option<&Expression>| {
            let whole = |e: &Expression| -> Option<String> {
                match &e.kind {
                    ExprKind::Paren(inner) => {
                        base_ident(inner).filter(|_| matches!(inner.kind, ExprKind::Ident(_)))
                    }
                    ExprKind::Ident(_) => base_ident(e),
                    _ => None,
                }
            };
            let one_bit = |e: &Expression| -> bool {
                let mut e = e;
                while let ExprKind::Paren(inner) = &e.kind {
                    e = inner;
                }
                match &e.kind {
                    ExprKind::Index { expr, index } => {
                        whole(expr).is_some() && const_val(index, params).is_some()
                    }
                    ExprKind::Ident(_) => whole(e)
                        .and_then(|n| module.signals.get(n.as_str()).map(|s| s.width == 1))
                        .unwrap_or(false),
                    _ => false,
                }
            };
            match &lhs.kind {
                ExprKind::Concatenation(parts) => {
                    if parts.iter().any(|p| base_ident(p).as_deref() == Some(v)) {
                        writes.push(None);
                    }
                }
                ExprKind::Ident(_) | ExprKind::Paren(_) if whole(lhs).as_deref() == Some(v) => {
                    writes.push(Some((i64::MIN, i64::MAX, false)));
                }
                ExprKind::Index { expr, index } if whole(expr).as_deref() == Some(v) => {
                    match const_val(index, params) {
                        Some(i) => writes.push(Some((i, i, false))),
                        None => writes.push(None),
                    }
                }
                ExprKind::RangeSelect {
                    expr,
                    kind,
                    left,
                    right,
                } if whole(expr).as_deref() == Some(v) => {
                    let (Some(l), Some(r)) = (const_val(left, params), const_val(right, params))
                    else {
                        writes.push(None);
                        return;
                    };
                    let (lo, hi) = match kind {
                        RangeKind::Constant => (l.min(r), l.max(r)),
                        RangeKind::IndexedUp => (l, l + r - 1),
                        RangeKind::IndexedDown => (l - r + 1, l),
                    };
                    let repl = rhs.is_some_and(|rhs| match &rhs.kind {
                        ExprKind::Replication { count, exprs } => {
                            exprs.len() == 1
                                && const_val(count, params) == Some(hi - lo + 1)
                                && one_bit(&exprs[0])
                        }
                        _ => false,
                    });
                    writes.push(Some((lo, hi, repl)));
                }
                _ => {
                    // Any other target naming `v` (a member or a nested
                    // select): extent unknown.
                    if base_ident(lhs).as_deref() == Some(v) {
                        writes.push(None);
                    }
                }
            }
        };
        let none: Vec<usize> = Vec::new();
        for &i in index.cont.get(vleaf).unwrap_or(&none) {
            let ca = &module.continuous_assigns[i];
            classify(&ca.lhs, Some(&ca.rhs));
        }
        for &i in index.pend.get(vleaf).unwrap_or(&none) {
            let p = &module.pending_cont_assign[i];
            let src = &*p.lhs_source;
            let may_name_v = |e: &Expression| -> bool {
                base_ident(e).is_some_and(|b| {
                    leaf(&b) == vleaf
                        || p.ctx
                            .port_map
                            .get(&b)
                            .is_some_and(|m| base_ident(m).is_some_and(|mb| leaf(&mb) == vleaf))
                })
            };
            let hit = match &src.kind {
                ExprKind::Concatenation(parts) => parts.iter().any(may_name_v),
                _ => may_name_v(src),
            };
            if !hit {
                continue;
            }
            let rw = |e: &Expression| {
                super::elaborate::rewrite_expr(
                    e,
                    &p.ctx.prefix,
                    &p.ctx.port_map,
                    &p.ctx.local_names,
                    &p.ctx.interface_map,
                )
            };
            let lhs = rw(src);
            let rhs = matches!(p.rhs_source.kind, ExprKind::Replication { .. })
                .then(|| rw(&p.rhs_source));
            classify(&lhs, rhs.as_ref());
        }
        if writes.iter().any(|w| w.is_none()) {
            return Vec::new();
        }
        let writes: Vec<(i64, i64, bool)> = writes.into_iter().flatten().collect();
        let mut out = Vec::new();
        for (i, &(lo, hi, repl)) in writes.iter().enumerate() {
            if !repl || hi <= lo {
                continue;
            }
            let shared = writes
                .iter()
                .enumerate()
                .any(|(j, &(l2, h2, _))| j != i && l2 <= hi && lo <= h2);
            if !shared {
                out.push((lo, hi));
            }
        }
        out
    }

    /// Can a pending always block stay lazy through `Simulator::new` (see
    /// `LazyAlways`)? Its source must be `@(...) body` with no timing control
    /// anywhere in the body that the computed-edge pass there would visit:
    /// then that pass, like the edge-select pass, can only touch the header
    /// (`rename_process_shadowed_locals` only ever renames a `begin..end`
    /// body's own locals, which this shape does not have at its top).
    pub(super) fn lazy_always_eligible(src: &Statement) -> bool {
        fn has_timing(st: &Statement) -> bool {
            match &st.kind {
                StatementKind::TimingControl { .. } => true,
                StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                    stmts.iter().any(has_timing)
                }
                StatementKind::If {
                    then_stmt,
                    else_stmt,
                    ..
                } => has_timing(then_stmt) || else_stmt.as_deref().is_some_and(has_timing),
                StatementKind::For { body, .. }
                | StatementKind::Foreach { body, .. }
                | StatementKind::While { body, .. }
                | StatementKind::DoWhile { body, .. }
                | StatementKind::Repeat { body, .. }
                | StatementKind::Forever { body, .. } => has_timing(body),
                StatementKind::Wait { stmt, .. } => has_timing(stmt),
                StatementKind::Case { items, .. } => items.iter().any(|it| has_timing(&it.stmt)),
                _ => false,
            }
        }
        match &src.kind {
            StatementKind::TimingControl {
                control: TimingControl::Event(_),
                stmt: body,
            } => !has_timing(body),
            _ => false,
        }
    }

    /// Whether a 1-D array's cells may live in the packed arena
    /// (`PackedMem`): a plain integral VARIABLE element of 1..=64 bits.
    /// Everything whose storage semantics the arena does not model stays an
    /// ordinary array — strings (dynamic length), reals, class handles,
    /// chandles, events, unpacked structs, interfaces, the queue / dynamic /
    /// associative collections `module.arrays` also lists, and nets (driver
    /// resolution, z initialization).
    pub(super) fn packed_admissible(module: &ElaboratedModule, base: &str, w: u32) -> bool {
        if w == 0 || w > 64 {
            return false;
        }
        let leaf = base.rsplit('.').next().unwrap_or(base);
        if module.queue_vars.contains(base)
            || module.dynamic_arrays.contains(base)
            || module.associative_arrays.contains_key(base)
            || module.string_signals.contains(base)
            || module.string_signals.contains(leaf)
            || module.nets.contains(base)
            || module.z_init_signals.contains(base)
        {
            return false;
        }
        // The declaration must be the array's OWN: an array-of-queue element
        // registered under a compound base (`aq[0]`) finds its type only by
        // stripping the index, and is a collection, not a memory.
        let Some(dt) = module.var_decl_types.get(base) else {
            return false;
        };
        match super::elaborate::resolve_typedef_chain(dt, &module.typedef_types) {
            DataType::IntegerVector { .. }
            | DataType::IntegerAtom { .. }
            | DataType::Enum(_)
            | DataType::Implicit { .. } => true,
            DataType::Struct(su) => su.packed,
            _ => false,
        }
    }

    pub fn new(mut module: ElaboratedModule, max_time: u64) -> Self {
        let phase_total = std::time::Instant::now();
        // The simulator counts ticks of `module.tick_s` (the finest timescale
        // precision; 1 ns by default). Delays were pre-scaled to ticks at
        // elaboration. `max_time` is supplied in nanoseconds, so convert it to
        // ticks here (no change when tick_s == 1 ns).
        let tick_s = module.tick_s;
        let max_time = {
            let scale = (1e-9_f64 / tick_s).round();
            (max_time as f64 * scale.max(1.0)) as u64
        };
        // X-init coercion: when XEZIM_INIT_ZERO=1, signals matching one of
        // XEZIM_INIT_ZERO_PATHS (comma-separated substring filters) start
        // at 0 instead of X. Use this for designs where intentional `1'bx`
        // defaults would lock the sim (c910 IFU pcfifo, etc.) without
        // affecting testbench-internal regs that rely on X-init semantics.
        let init_zero = std::env::var("XEZIM_INIT_ZERO")
            .map(|v| v == "1")
            .unwrap_or(false);
        let init_zero_paths: Vec<String> = std::env::var("XEZIM_INIT_ZERO_PATHS")
            .ok()
            .map(|s| {
                s.split(',')
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        // Static signals + parameters live exclusively in the indexed
        // signal_table / signal_name_to_id / parallel Vecs below. The
        // legacy `signals`/`widths`/`signed_signals`/`real_signals`
        // HashMaps stay empty after construction and are used only as an
        // overflow store for *dynamically* created entries (queue
        // elements, foreach loop vars, in-process var decls, $cast
        // targets). Eliminating the bulk-populate saves ~150–250 MB on
        // c910-scale designs (585K signals × 2 stores × name + Value
        // duplication).
        let signals = SignalMap {
            map: HashMap::default(),
            elems: HashMap::default(),
            elem_gens: HashMap::default(),
            elem_gen_counter: 0,
        };
        let widths: HashMap<String, u32> = HashMap::default();
        let signed_signals: HashSet<String> = HashSet::default();
        let real_signals: HashSet<String> = HashSet::default();

        let phase_materialize = crate::WallTimer::now();
        Self::materialize_implicit_contassign_nets(&mut module);
        event_bits::complete_ascending_ranges(&mut module);
        // Lazy always blocks (see `LazyAlways`) everywhere except where a
        // pass reads every always-block body before classification: code
        // coverage instruments them, and opt-in signal placement walks them.
        let lazy_always_ok = code_cov::code_coverage().is_none()
            && std::env::var("XEZIM_PLACE_SIGNALS").ok().as_deref() != Some("1")
            && !matches!(std::env::var("XEZIM_LAZY_ALWAYS").as_deref(), Ok("0"));
        let mut deferred_always =
            Self::rewrite_edge_select_sensitivities(&mut module, lazy_always_ok);
        // §18.3/§18.4: publish class-local `typedef enum`/`typedef struct`
        // types into the module type tables so a rand property declared with
        // one gets its enum-member domain / packed bit layout.
        // The elaborator already publishes qualified (and sometimes bare)
        // class-local aliases. A generic `typedef C#(N) own_type` is not a
        // reference to C's DEFAULT specialization; resolve it only when its
        // declaring class has concrete parameter bindings.
        let generic_alias_spans: HashSet<(&str, usize, usize)> = module
            .classes
            .values()
            .filter(|cd| !cd.param_order.is_empty() || !cd.type_param_names.is_empty())
            .flat_map(|cd| cd.typedef_targets.iter())
            .filter_map(|(alias, dt)| match dt {
                DataType::TypeReference { span, .. } => {
                    Some((alias.as_str(), span.start, span.end))
                }
                _ => None,
            })
            .collect();
        let referenced_class_typedefs = module
            .typedef_types
            .iter()
            .filter_map(|(key, dt)| {
                let DataType::TypeReference { span, .. } = dt else {
                    return None;
                };
                let generic_local = match key.rsplit_once("::") {
                    Some((owner, alias)) => module
                        .classes
                        .get(owner)
                        .or_else(|| {
                            module
                                .classes
                                .get(owner.rsplit("::").next().unwrap_or(owner))
                        })
                        .is_some_and(|cd| {
                            (!cd.param_order.is_empty() || !cd.type_param_names.is_empty())
                                && cd.typedef_targets.contains_key(alias)
                        }),
                    // A real module/package alias has a separate declaration span.
                    None => generic_alias_spans.contains(&(key.as_str(), span.start, span.end)),
                };
                (!generic_local).then(|| dt.clone())
            })
            .collect();
        Self::hoist_class_local_typedefs(&mut module);
        // §6.18/§7.2.1: struct members typed by a collection typedef take
        // the direct `elem m[$]` form every struct-collection path reads.
        Self::inline_struct_member_coll_typedefs(&mut module);
        // §6.18: a property typed by a module/package-level (or inherited)
        // collection typedef must be classified as a queue/assoc/array member
        // — elaborate_class only saw the class's OWN local typedefs.
        Self::classify_typedef_collection_properties(&mut module);
        let contested_pkg_vars = Self::split_contested_package_vars(&mut module);
        Self::register_struct_member_packed_dims(&mut module);
        Self::fold_named_sva_members(&mut module);
        // §9.2.2.2: give every dynamic array / queue a REAL `<q>.size` signal.
        // It doubles as the queue's change proxy in the comb dependency graph:
        // sensitivity registration resolves it to an id, and every mutation
        // (push/pop/delete, element write) marks it dirty — without it a comb
        // block reading `q.size()` or `q[0]` never re-fired on queue changes.
        // §6.21: rename process-body block locals that shadow a module-scope
        // name — the flattening process executor has no block-scope boundary,
        // so a frameless local landed on the module signal itself (see
        // `rename_process_shadowed_locals`).
        {
            // `rename_process_shadowed_locals` only ever asks whether a
            // process body's own top-level local is taken, so the set holds
            // just those locals (the ones a module symbol or a second
            // process also claims) instead of a copy of every module name.
            let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
            let module_claims = |nm: &str| {
                module.signals.contains_key(nm)
                    || module.arrays.contains_key(nm)
                    || module.dynamic_arrays.contains(nm)
                    || module.associative_arrays.contains_key(nm)
            };
            // §6.21 extended: two processes declaring the SAME frameless
            // block-local name (neither shadowing any module symbol) would
            // share one `signals[name]` slot and last-write-win — e.g.
            // `int v` in two initial blocks cross-read each other's value.
            // The flattening executor has no per-process block-scope frame for
            // frameless locals, so alpha-rename a frameless local like a
            // shadow when the SAME name is a frameless local of a DIFFERENT
            // process (reference-validated localsame.sv: p1 must read its own
            // `v=10`, not p2's `v=1`). Single-process locals stay as-is to
            // avoid perturbing auto-loop-var / signal-name bookkeeping.
            fn frameless_locals(stmt: &Statement) -> std::collections::HashSet<String> {
                let mut out = std::collections::HashSet::new();
                Simulator::for_each_decl_name(stmt, &mut |name, _| {
                    out.insert(name.to_string());
                });
                out
            }
            // §26.3: module and package subroutine bodies spell `P::m` the
            // way the process bodies do (see `flatten_pkg_member_refs`).
            if !module.packages.is_empty() {
                let pkgs: std::collections::HashSet<String> =
                    module.packages.iter().cloned().collect();
                let flat = |items: &[Statement]| -> Vec<Statement> {
                    items
                        .iter()
                        .map(|s| super::elaborate::flatten_pkg_member_refs(s, &pkgs))
                        .collect()
                };
                for td in module.tasks.values_mut() {
                    td.items = flat(&td.items);
                }
                let fnames: Vec<String> = module.functions.keys().cloned().collect();
                for n in fnames {
                    if let Some(mut fd) = module.functions.remove(&n) {
                        fd.items = flat(&fd.items);
                        module.functions.insert(n, fd);
                    }
                }
            }
            // §6.21/§9.3.4: a collection local of a NESTED block (or of a
            // module task / function body) that shadows a module item or a
            // local of an enclosing block is alpha-renamed too, innermost
            // first, so the top-level pass below no longer sees it
            // redeclared. Collections have no frame of their own outside
            // class methods and automatic tasks: under its bare name a local
            // `int fa [3]` was the module's `fa`.
            {
                let mut n = 0usize;
                let claims = |nm: &str| module_claims(nm) || module.queue_vars.contains(nm);
                let mut bodies: Vec<&mut Statement> = Vec::new();
                bodies.extend(module.initial_blocks.iter_mut().map(|b| &mut b.stmt));
                bodies.extend(module.always_blocks.iter_mut().map(|b| &mut b.stmt));
                bodies.extend(deferred_always.iter_mut().filter_map(|d| match d {
                    DeferredAlways::Eager(ab) => Some(&mut ab.stmt),
                    DeferredAlways::Lazy { .. } => None,
                }));
                bodies.extend(module.final_blocks.iter_mut().map(|b| &mut b.stmt));
                let mut renamed: Vec<(usize, Statement)> = Vec::new();
                for (k, b) in bodies.iter().enumerate() {
                    if !Self::shadow_rename_possible(b, &claims) {
                        continue;
                    }
                    let mut st = (**b).clone();
                    if Self::rename_nested_shadowed(
                        &mut st,
                        &claims,
                        &mut Vec::new(),
                        &mut n,
                        "b",
                        0,
                        "",
                        &mut Vec::new(),
                    ) {
                        renamed.push((k, st));
                    }
                }
                for (k, st) in renamed {
                    *bodies[k] = st;
                }
                // §6.21: a process's static block variables are created once
                // (see `exec_stmt_var_decl`), so two blocks of one process
                // declaring the same name — `begin int a; ... end begin int
                // unsigned a; ... end` — need storage (and a declared type)
                // of their own: rename the nested ones.
                for b in bodies.iter_mut() {
                    let mut counts: HashMap<&str, usize> = HashMap::default();
                    Self::for_each_decl_name(b, &mut |nm, _| *counts.entry(nm).or_insert(0) += 1);
                    let dups: HashSet<String> = counts
                        .into_iter()
                        .filter(|&(_, c)| c >= 2)
                        .map(|(nm, _)| nm.to_string())
                        .collect();
                    if dups.is_empty() {
                        continue;
                    }
                    Self::rename_nested_shadowed(
                        b,
                        &|nm| dups.contains(nm),
                        &mut Vec::new(),
                        &mut n,
                        "pb",
                        0,
                        "",
                        &mut Vec::new(),
                    );
                }
                drop(bodies);
                let wrap = |items: &[Statement], span| Statement {
                    kind: StatementKind::SeqBlock {
                        name: None,
                        stmts: items.to_vec(),
                    },
                    span,
                };
                // §6.21: the variables of a static task or function keep
                // their values between calls.
                // (A class method, here when defined out of its class, is
                // automatic, §8.6.)
                for td in module.tasks.values_mut() {
                    if td.lifetime != Some(crate::ast::types::Lifetime::Automatic)
                        && td.name.scopes.is_empty()
                    {
                        Self::mark_static_locals(&mut td.items);
                    }
                }
                let fkeys: Vec<String> = module
                    .functions
                    .iter()
                    .filter(|(_, fd)| {
                        fd.lifetime != Some(crate::ast::types::Lifetime::Automatic)
                            && fd.name.scopes.is_empty()
                            && Self::static_locals_to_mark(&fd.items)
                    })
                    .map(|(k, _)| k.clone())
                    .collect();
                for key in fkeys {
                    if let Some(fd) = module.functions.get(&key) {
                        let mut fd = fd.clone();
                        Self::mark_static_locals(&mut fd.items);
                        module.functions.insert(key, fd);
                    }
                }
                let mut tdone = Vec::new();
                for (key, td) in module.tasks.iter() {
                    if !td
                        .items
                        .iter()
                        .any(|s| Self::shadow_rename_possible(s, &claims))
                    {
                        continue;
                    }
                    let mut st = wrap(&td.items, td.span);
                    if Self::rename_nested_shadowed(
                        &mut st,
                        &claims,
                        &mut Vec::new(),
                        &mut n,
                        "b",
                        1,
                        key,
                        &mut Vec::new(),
                    ) {
                        tdone.push((key.clone(), st));
                    }
                }
                let mut fdone = Vec::new();
                for (key, fd) in module.functions.iter() {
                    if !fd
                        .items
                        .iter()
                        .any(|s| Self::shadow_rename_possible(s, &claims))
                    {
                        continue;
                    }
                    let mut st = wrap(&fd.items, fd.span);
                    if Self::rename_nested_shadowed(
                        &mut st,
                        &claims,
                        &mut Vec::new(),
                        &mut n,
                        "b",
                        1,
                        key,
                        &mut Vec::new(),
                    ) {
                        fdone.push((key.clone(), st));
                    }
                }
                for (key, st) in tdone {
                    if let (Some(td), StatementKind::SeqBlock { stmts, .. }) =
                        (module.tasks.get_mut(&key), st.kind)
                    {
                        td.items = stmts;
                    }
                }
                for (key, st) in fdone {
                    if let (Some(fd), StatementKind::SeqBlock { stmts, .. }) =
                        (module.functions.get(&key), st.kind)
                    {
                        let mut fd = fd.clone();
                        fd.items = stmts;
                        module.functions.insert(key, fd);
                    }
                }
            }
            let mut per_proc_counts: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            let mut all_proc = Vec::new();
            all_proc.extend(module.initial_blocks.iter().map(|b| &b.stmt));
            all_proc.extend(module.always_blocks.iter().map(|b| &b.stmt));
            // A lazy block (`@(...) body`) has no top-level locals.
            all_proc.extend(deferred_always.iter().filter_map(|d| match d {
                DeferredAlways::Eager(ab) => Some(&ab.stmt),
                DeferredAlways::Lazy { .. } => None,
            }));
            all_proc.extend(module.final_blocks.iter().map(|b| &b.stmt));
            for st in all_proc.iter() {
                for nm in frameless_locals(st) {
                    *per_proc_counts.entry(nm).or_insert(0) += 1;
                }
            }
            for (nm, c) in per_proc_counts {
                if c >= 2 || module_claims(&nm) {
                    taken.insert(nm);
                }
            }
            // §6.21: collisions also include a local inside another
            // process's nested block. Rename those declarations innermost first.
            let mut nested_number = 0;
            for b in module
                .initial_blocks
                .iter_mut()
                .map(|b| &mut b.stmt)
                .chain(module.always_blocks.iter_mut().map(|b| &mut b.stmt))
                .chain(deferred_always.iter_mut().filter_map(|d| match d {
                    DeferredAlways::Eager(ab) => Some(&mut ab.stmt),
                    DeferredAlways::Lazy { .. } => None,
                }))
                .chain(module.final_blocks.iter_mut().map(|b| &mut b.stmt))
            {
                Self::rename_nested_shadowed(
                    b,
                    &|n| taken.contains(n),
                    &mut Vec::new(),
                    &mut nested_number,
                    "pc",
                    0,
                    "",
                    &mut Vec::new(),
                );
            }
            for (i, ib) in module.initial_blocks.iter_mut().enumerate() {
                if let Some(new_stmt) = super::elaborate::rename_process_shadowed_locals(
                    &ib.stmt,
                    &taken,
                    &format!("i{}", i),
                ) {
                    ib.stmt = new_stmt;
                }
            }
            let n_always = module.always_blocks.len();
            let deferred_eager =
                deferred_always
                    .iter_mut()
                    .enumerate()
                    .filter_map(|(k, d)| match d {
                        DeferredAlways::Eager(ab) => Some((n_always + k, ab)),
                        DeferredAlways::Lazy { .. } => None,
                    });
            for (i, ab) in module
                .always_blocks
                .iter_mut()
                .enumerate()
                .chain(deferred_eager)
            {
                if let Some(new_stmt) = super::elaborate::rename_process_shadowed_locals(
                    &ab.stmt,
                    &taken,
                    &format!("a{}", i),
                ) {
                    ab.stmt = new_stmt;
                }
            }
            for (i, fb) in module.final_blocks.iter_mut().enumerate() {
                if let Some(new_stmt) = super::elaborate::rename_process_shadowed_locals(
                    &fb.stmt,
                    &taken,
                    &format!("f{}", i),
                ) {
                    fb.stmt = new_stmt;
                }
            }
        }
        // §9.4.2: `always @(posedge <expression>)`. Sensitivity is built from
        // the identifier NAMES appearing in the event expression, so a COMPUTED
        // term (`a & 1'b1`, `~b`, `c ? x : y`) contributed no names at all: the
        // block armed on nothing and never ran, silently. Sensitizing its leaf
        // identifiers instead would be wrong — `posedge (~b)` must fire when
        // `~b` RISES, i.e. when `b` falls — so give each computed term a hidden
        // 1-bit net driven by a continuous assign and put the edge on that. The
        // net carries the expression's own value, which is exactly what the
        // edge is defined against; truncating it to one bit also matches
        // §9.4.2's rule that an edge on a multi-bit expression tracks its LSB.
        {
            use crate::ast::stmt::{EventControl as EC, TimingControl as TC};
            fn computed_edge_term(e: &crate::ast::expr::Expression) -> bool {
                match &e.kind {
                    ExprKind::Paren(inner) => computed_edge_term(inner),
                    ExprKind::Binary { .. }
                    | ExprKind::Unary { .. }
                    | ExprKind::Conditional { .. } => true,
                    _ => false,
                }
            }
            let mk_ident = |nm: &str, span| {
                Expression::new(
                    ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                        root: None,
                        path: vec![crate::ast::expr::HierPathSegment {
                            name: crate::ast::Identifier {
                                name: nm.to_string(),
                                span,
                            },
                            selects: Vec::new(),
                        }],
                        span,
                        cached_signal_id: std::cell::Cell::new(None),
                        cached_resolved_name: std::cell::OnceCell::new(),
                    }),
                    span,
                )
            };
            let mut synth: Vec<(String, Expression)> = Vec::new();
            // An event control is not only an always HEADER: `@(posedge (~b))`
            // also appears mid-block, inside a fork arm, a loop body, or a
            // branch. Rewriting only the header left every one of those armed
            // on nothing, so walk the whole process body.
            fn rewrite_edges(
                stmt: &mut Statement,
                tag: &str,
                n: &mut usize,
                synth: &mut Vec<(String, Expression)>,
                mk_ident: &dyn Fn(&str, crate::ast::Span) -> Expression,
                computed: &dyn Fn(&Expression) -> bool,
            ) {
                use crate::ast::stmt::{EventControl as EC, TimingControl as TC};
                match &mut stmt.kind {
                    StatementKind::TimingControl {
                        control,
                        stmt: inner,
                    } => {
                        if let TC::Event(EC::EventExpr(exprs)) = control {
                            for ee in exprs.iter_mut() {
                                if ee.edge.is_none() || !computed(&ee.expr) {
                                    continue;
                                }
                                let nm = format!("__xz_edge_{}_{}", tag, *n);
                                *n += 1;
                                let span = ee.expr.span;
                                synth.push((nm.clone(), ee.expr.clone()));
                                ee.expr = mk_ident(&nm, span);
                            }
                        }
                        rewrite_edges(inner, tag, n, synth, mk_ident, computed);
                    }
                    StatementKind::SeqBlock { stmts, .. }
                    | StatementKind::ParBlock { stmts, .. } => {
                        for st in stmts.iter_mut() {
                            rewrite_edges(st, tag, n, synth, mk_ident, computed);
                        }
                    }
                    StatementKind::If {
                        then_stmt,
                        else_stmt,
                        ..
                    } => {
                        rewrite_edges(then_stmt, tag, n, synth, mk_ident, computed);
                        if let Some(e) = else_stmt {
                            rewrite_edges(e, tag, n, synth, mk_ident, computed);
                        }
                    }
                    StatementKind::For { body, .. }
                    | StatementKind::Foreach { body, .. }
                    | StatementKind::While { body, .. }
                    | StatementKind::DoWhile { body, .. }
                    | StatementKind::Repeat { body, .. }
                    | StatementKind::Forever { body, .. } => {
                        rewrite_edges(body, tag, n, synth, mk_ident, computed);
                    }
                    StatementKind::Wait { stmt: inner, .. } => {
                        rewrite_edges(inner, tag, n, synth, mk_ident, computed);
                    }
                    StatementKind::Case { items, .. } => {
                        for it in items.iter_mut() {
                            rewrite_edges(&mut it.stmt, tag, n, synth, mk_ident, computed);
                        }
                    }
                    _ => {}
                }
            }
            let mut n = 0usize;
            for (bi, ab) in module.always_blocks.iter_mut().enumerate() {
                let tag = format!("a{}", bi);
                rewrite_edges(
                    &mut ab.stmt,
                    &tag,
                    &mut n,
                    &mut synth,
                    &mk_ident,
                    &computed_edge_term,
                );
            }
            // The deferred blocks continue the numbering. A lazy block's
            // header is the only part this pass can touch (see
            // `lazy_always_eligible`); record which terms it replaces.
            let n_always = module.always_blocks.len();
            for (k, d) in deferred_always.iter_mut().enumerate() {
                let tag = format!("a{}", n_always + k);
                let stmt = match d {
                    DeferredAlways::Eager(ab) => &mut ab.stmt,
                    DeferredAlways::Lazy {
                        header, patched, ..
                    } => {
                        let header = &mut **header.as_mut().expect("header kept until here");
                        if let StatementKind::TimingControl {
                            control: TC::Event(EC::EventExpr(exprs)),
                            ..
                        } = &header.kind
                        {
                            for (i, ee) in exprs.iter().enumerate() {
                                if ee.edge.is_some() && computed_edge_term(&ee.expr) {
                                    patched.push(i);
                                }
                            }
                        }
                        header
                    }
                };
                rewrite_edges(
                    stmt,
                    &tag,
                    &mut n,
                    &mut synth,
                    &mk_ident,
                    &computed_edge_term,
                );
            }
            for (bi, ib) in module.initial_blocks.iter_mut().enumerate() {
                let tag = format!("i{}", bi);
                rewrite_edges(
                    &mut ib.stmt,
                    &tag,
                    &mut n,
                    &mut synth,
                    &mk_ident,
                    &computed_edge_term,
                );
            }
            for (bi, fb) in module.final_blocks.iter_mut().enumerate() {
                let tag = format!("f{}", bi);
                rewrite_edges(
                    &mut fb.stmt,
                    &tag,
                    &mut n,
                    &mut synth,
                    &mk_ident,
                    &computed_edge_term,
                );
            }
            for (nm, rhs) in synth {
                module
                    .signals
                    .entry(nm.clone())
                    .or_insert_with(|| super::elaborate::Signal {
                        name: nm.clone(),
                        width: 1,
                        is_signed: false,
                        is_real: false,
                        is_const: false,
                        direction: None,
                        value: Value::new(1),
                        type_name: None,
                    });
                module
                    .continuous_assigns
                    .push(super::elaborate::ContinuousAssignment {
                        origin: None,
                        lhs: mk_ident(&nm, rhs.span),
                        rhs,
                        delay: 0,
                        rhs_parent_scoped: false,
                        delay_fall: None,
                        delay_off: None,
                    });
            }
        }
        // The passes over always-block headers are done: keep only the
        // replaced terms of each lazy block's header.
        for d in deferred_always.iter_mut() {
            if let DeferredAlways::Lazy {
                lazy,
                header,
                patched,
            } = d
            {
                let header = header.take().expect("header kept until here");
                if let StatementKind::TimingControl {
                    control: TimingControl::Event(EventControl::EventExpr(exprs)),
                    ..
                } = header.kind
                {
                    patched.sort_unstable();
                    patched.dedup();
                    lazy.patches = patched
                        .drain(..)
                        .map(|i| (i, exprs[i].expr.clone()))
                        .collect();
                }
            }
        }
        // §8.4/§25.9: an unassigned `virtual <iface>` VARIABLE holds the null
        // handle, like any handle type — not x. Registered as an ordinary
        // 4-state signal it defaulted to x, so the standard guard
        // `if (vif == null) $fatal(...)` evaluated x, took the else branch,
        // and the "not connected" check silently passed. (Class properties
        // already default to 0, which is why only the module/block-scope
        // variable form misbehaved.) The type may be a typedef alias of a
        // virtual interface (`typedef virtual bus_if vif_t;`, from the module,
        // a package or `$unit`, possibly a typedef of a typedef): IEEE
        // 1800-2023 Table 6-7 gives it the same `null` default. Each element
        // of an unpacked array of such handles starts null too.
        // The element storage of an array is built further down; its bases
        // are zeroed there (see `vif_vars` below).
        // §8.4: a class handle starts null as well, so an unpacked array of
        // class handles joins this set (a scalar one is zeroed elsewhere).
        let vif_vars: std::collections::HashSet<String> = module
            .var_decl_types
            .iter()
            .filter(|(_, dt)| {
                Self::dt_is_virtual_iface(dt, &module) || Self::dt_is_class_handle(dt, &module)
            })
            .map(|(n, _)| n.clone())
            .collect();
        for n in &vif_vars {
            if let Some(sig) = module.signals.get_mut(n) {
                if sig.value.has_xz() {
                    sig.value = Value::zero(sig.value.width.max(1));
                }
            }
        }
        // The `.size` signal doubles as a collection's comb-dependency proxy —
        // associative arrays need one too, or element reads have nothing to
        // depend on (§9.2.2.2).
        let queue_names: Vec<String> = module
            .dynamic_arrays
            .iter()
            .cloned()
            .chain(
                module
                    .associative_arrays
                    .keys()
                    // A `[N]` unpacked dim first registers as associative and
                    // may ALSO be a fixed array by now — those have per-element
                    // ids and need no proxy, and giving one a `.size` signal
                    // broke the fixed array's own name resolution.
                    .filter(|n| {
                        !module.arrays.contains_key(*n)
                            && !module.arrays_2d.contains_key(*n)
                            && !module.arrays_nd.contains_key(*n)
                    })
                    .cloned(),
            )
            .collect();
        for q in queue_names {
            let sz_name = format!("{}.size", q);
            let sig =
                module
                    .signals
                    .entry(sz_name.clone())
                    .or_insert_with(|| super::elaborate::Signal {
                        name: sz_name,
                        width: 32,
                        is_signed: false,
                        is_real: false,
                        is_const: false,
                        direction: None,
                        value: Value::zero(32),
                        type_name: None,
                    });
            // It is also what a paren-less `q.size` / `a.num` reads, and
            // those return `int` (§7.5.2, §7.9.1, §7.10.2.1): SIGNED.
            sig.is_signed = true;
            sig.value.is_signed = true;
        }
        let materialize_ms = phase_materialize.elapsed().as_secs_f64() * 1000.0;
        crate::rss_trace("sim::new materialize");

        // Collect just *names* from the two source maps and sort them
        // for deterministic id assignment. We avoid an intermediate
        // `Vec<(name, Value, …)>` because cloning each Value into that
        // Vec would peak-spike RSS by exactly the amount we just saved
        // by skipping the legacy bulk-populate.
        // Borrowed from the two maps (they outlive the id loop below), so
        // naming 426k signals costs no string copies before the interned
        // `Arc<str>` each one gets there.
        let mut names: Vec<std::borrow::Cow<'_, str>> =
            Vec::with_capacity(module.signals.len() + module.parameters.len());
        let phase_names = std::time::Instant::now();
        for name in module.signals.keys() {
            names.push(std::borrow::Cow::Borrowed(name.as_str()));
        }
        // Port directions for the VPI layer (`vpiPort` iteration and
        // `vpiDirection`), keyed by the interned signal name: filled in the
        // static-signal pass below, which visits every `module.signals`
        // entry before that map is freed.
        let mut vpi_port_directions: HashMap<Arc<str>, PortDirection> = HashMap::default();
        for name in module.parameters.keys() {
            // Parameter and signal can share a name — dedup after sort.
            names.push(std::borrow::Cow::Borrowed(name.as_str()));
        }
        // Same order as `sort()` (equal names are the same text, so their
        // relative order cannot matter), without re-comparing the long
        // shared hierarchical prefixes.
        names::sort_lexicographic(&mut names);
        names.dedup();
        // Signal placement (`XEZIM_PLACE_SIGNALS=1`, opt-in; measured NEGATIVE
        // on c906 memcpy: L2 misses +1.1%, cycles +0.35% — the greedy
        // source-order walk scatters declaration-adjacent bus bits more than
        // it gathers operands; a locality-preserving order over the real
        // entry graph is the form still to try): reorder the
        // scalar names so that the operands of a continuous assign or always
        // block get ids next to the signals it writes. Ids are handed out in
        // this order below, and every per-signal table follows the id, so an
        // entry's operands then share cache lines instead of sitting at
        // declaration-order distance (c906: median per-entry operand span 299
        // ids, 71% of operands are other entries' outputs; the run is
        // L2-miss bound at ~23 misses per evaluation). Arrays keep their own
        // contiguous element ids (allocated separately below).
        if std::env::var("XEZIM_PLACE_SIGNALS").ok().as_deref() == Some("1") {
            names = Self::place_signal_names(
                &module,
                names.into_iter().map(|n| n.into_owned()).collect(),
            )
            .into_iter()
            .map(std::borrow::Cow::Owned)
            .collect();
        }
        let names_ms = phase_names.elapsed().as_secs_f64() * 1000.0;
        crate::rss_trace("sim::new names");

        // §26.3: package-scope names that MORE THAN ONE package declares. Such
        // a name is hoisted into the shared tables under its bare spelling as
        // well as `<pkg>::<name>`, and the bare key is last-writer-wins across
        // packages — so inside a package subroutine it has to be read (or
        // called) through the running package's own entry. Every other name
        // resolves identically either way, so the set stays empty for
        // practically every design and both lookups short-circuit.
        let mut pkg_ambiguous_names: HashSet<String> = contested_pkg_vars.clone();
        {
            let mut first_owner: HashMap<&str, &str> = HashMap::default();
            for key in module.parameters.keys() {
                let Some((pkg, base)) = key.split_once("::") else {
                    continue;
                };
                // `::` also keys class-scoped entries; only a package prefix
                // counts here.
                if !module.packages.contains(pkg) {
                    continue;
                }
                match first_owner.get(base) {
                    None => {
                        first_owner.insert(base, pkg);
                    }
                    Some(prev) if *prev != pkg => {
                        pkg_ambiguous_names.insert(base.to_string());
                    }
                    Some(_) => {}
                }
            }
            // Elaboration only mints a qualified subroutine key when a second
            // package claims the name, so every one of these is contested.
            for key in module.functions.keys().chain(module.tasks.keys()) {
                let Some((pkg, base)) = key.split_once("::") else {
                    continue;
                };
                if module.packages.contains(pkg) {
                    pkg_ambiguous_names.insert(base.to_string());
                }
            }
        }

        // §26.3: package enum members under their qualified spelling, for
        // the bytecode compiler (see `Simulator::pkg_enum_consts`).
        let pkg_enum_consts: HashMap<String, (u64, u32)> = module
            .package_enum_members
            .iter()
            .flat_map(|(pkg, members)| {
                members
                    .iter()
                    .filter(|(_, (_, w))| *w > 0 && *w <= 64)
                    .map(move |(m, (v, w))| (format!("{}::{}", pkg, m), (*v, *w)))
            })
            .collect();

        let n = names.len();
        let mut signal_name_to_id = NameMap::with_capacity(n);
        signal_name_to_id
            .set_interface_aliases(module.name.clone(), module.interface_port_aliases.clone());
        // Ports the elaborator left out as unobserved: a lookup that misses
        // is checked against them (see `names::ElidedPorts`).
        if module.elided_port_count > 0 {
            chatter!(
                "[ELIDE] {} unobserved input-port nets left out",
                module.elided_port_count
            );
        }
        if !module.elided_port_hashes.is_empty() {
            signal_name_to_id.set_elided(Some(Arc::new(names::ElidedPorts::new(std::mem::take(
                &mut module.elided_port_hashes,
            )))));
        }
        let mut leaf_name_to_ids: HashMap<Arc<str>, Vec<usize>> = HashMap::default();
        let mut id_to_name = IdNames::with_capacity(n);
        let mut signal_table: Vec<Value> = Vec::with_capacity(n);
        let mut signal_widths_vec: Vec<u32> = Vec::with_capacity(n);
        let mut signal_signed_vec: Vec<bool> = Vec::with_capacity(n);
        let mut signal_real_vec: Vec<bool> = Vec::with_capacity(n);
        let mut signal_type_names: HashMap<usize, String> = HashMap::default();
        let phase_static = std::time::Instant::now();
        // Drain `names` so each String moves into the Arc allocation
        // and is freed promptly, instead of being kept alongside the
        // Arc<str> copies until the end of construction.
        for (id, name) in names.drain(..).enumerate() {
            let name: &str = &name;
            let arc_name: Arc<str> = Arc::from(name);
            signal_name_to_id.insert(arc_name.clone(), id);
            // Build leaf-name reverse index (skip array-element style names
            // ending with `]`, which are added below and don't participate
            // in bare-leaf lookups).
            let leaf = name.rsplit_once('.').map(|(_, l)| l).unwrap_or(name);
            if !leaf.is_empty() && !leaf.ends_with(']') {
                // Most leaves repeat (`q` in every instance): allocate the
                // key only for a leaf not seen yet.
                match leaf_name_to_ids.get_mut(leaf) {
                    Some(ids) => ids.push(id),
                    None => {
                        leaf_name_to_ids.insert(Arc::from(leaf), vec![id]);
                    }
                }
            }
            // Fetch value/metadata directly from the source map. The
            // signal entry takes precedence over a same-named parameter.
            let sig = module.signals.get(name);
            if let Some(dir) = sig.and_then(|sig| sig.direction) {
                vpi_port_directions.insert(arc_name.clone(), dir);
            }
            id_to_name.push(arc_name);
            if let Some(sig) = sig {
                let mut val = sig.value.clone();
                if sig.is_signed {
                    val.is_signed = true;
                }
                if sig.is_real {
                    val.is_real = true;
                }
                // XEZIM_INIT_ZERO=1: coerce all-X initial values to zero.
                // Default (off) preserves Verilog 4-state semantics. Industry
                // simulators offer the same knob as a register-init option.
                // Needed for designs (like c910) that pump explicit `1'bx`
                // through default arms and rely on hardware-side gating that
                // doesn't model in 4-state RTL sim.
                // Construction-time signal coercion RETIRED (Aug 2026): it was
                // invisible to the event system and forced nets into unreachable
                // states. Register init now happens via real t=0 writes in
                // `init_registers` (XEZIM_INIT_REG, with XEZIM_INIT_ZERO as a
                // deprecated alias); array/memory zeroing lives on below.
                let _ = &init_zero_paths;
                sim_dbg_eprintln!(
                    "[DEBUG] Simulator::new signal {} = {} (signed={})",
                    name,
                    val.to_dec_string(),
                    sig.is_signed
                );
                signal_table.push(val);
                signal_widths_vec.push(sig.width);
                signal_signed_vec.push(sig.is_signed);
                signal_real_vec.push(sig.is_real);
                if let Some(ref tn) = sig.type_name {
                    signal_type_names.insert(id, tn.clone());
                }
            } else if let Some(val) = module.parameters.get(name) {
                sim_dbg_eprintln!(
                    "[DEBUG] Simulator::new parameter {} = {} (signed={})",
                    name,
                    val.to_dec_string(),
                    val.is_signed
                );
                signal_table.push(val.clone());
                signal_widths_vec.push(val.width);
                signal_signed_vec.push(val.is_signed);
                signal_real_vec.push(false);
            } else {
                // Should not happen — name came from one of the two maps.
                signal_table.push(Value::new(1));
                signal_widths_vec.push(1);
                signal_signed_vec.push(false);
                signal_real_vec.push(false);
            }
        }
        // §14.13 clocking-sample event signals. `always @(cb.sig)` /
        // `@(cb.sig)` must fire AT the clocking edge and ONLY when the newly
        // sampled value differs from the previous sample — neither the net's
        // own edges (which can occur between clock events) nor the clock
        // itself expresses that. Give every clocking INPUT a hidden mirror
        // signal that `tick_clocking_blocks` writes with each fresh sample;
        // the ordinary edge machinery then delivers exactly the right events.
        // Name is unreachable from SV source, so it cannot collide.
        for (cb_name, cd) in module.clocking_blocks.iter() {
            for csig in cd.signals.iter() {
                let is_input = matches!(
                    csig.direction,
                    crate::ast::types::PortDirection::Input
                        | crate::ast::types::PortDirection::Inout
                );
                if !is_input {
                    continue;
                }
                let net = csig
                    .bound_to
                    .as_ref()
                    .and_then(Self::flatten_member_path)
                    .map(|segs| segs.join("."))
                    .unwrap_or_else(|| csig.name.name.clone());
                let width = signal_name_to_id
                    .get(net.as_str())
                    .map(|&id| signal_widths_vec[id])
                    .unwrap_or(1);
                let hidden = format!("{}{}.{}", CB_SAMPLE_PREFIX, cb_name, csig.name.name);
                if signal_name_to_id.contains_key(hidden.as_str()) {
                    continue;
                }
                let id = signal_table.len();
                let arc_name: Arc<str> = Arc::from(hidden.as_str());
                signal_name_to_id.insert(arc_name.clone(), id);
                id_to_name.push(arc_name);
                signal_table.push(Value::new(width));
                signal_widths_vec.push(width);
                signal_signed_vec.push(false);
                signal_real_vec.push(false);
            }
        }
        // §10.11 alias unification: repoint each aliased name at the
        // canonical net's slot — ONE storage, N names. Width agreement was
        // checked at elaboration.
        for (canon, other) in module.alias_pairs.iter() {
            let (Some(&cid), Some(&oid)) = (
                signal_name_to_id.get(canon.as_str()),
                signal_name_to_id.get(other.as_str()),
            ) else {
                continue;
            };
            if cid != oid {
                if let Some(slot) = signal_name_to_id.get_mut(other.as_str()) {
                    *slot = cid;
                }
                let _ = oid; // the orphan slot keeps its default value, unreferenced
            }
        }
        let static_ms = phase_static.elapsed().as_secs_f64() * 1000.0;
        crate::rss_trace("sim::new static signals");
        // Phase 2: synthesize per-element entries for unpacked arrays.
        // Elaborate skips the per-element Signal inserts (memory-as-array
        // fix) because every element shares the same width/signed/real
        // attributes — we rebuild them here from the compact arrays
        // metadata, writing directly into signal_table + signal_name_to_id
        // + the parallel Vecs. The legacy `signals` / `widths` /
        // `signed_signals` / `real_signals` HashMaps are left empty for
        // array elements; all hot-path and fallback accesses go through
        // signal_name_to_id first, so the HashMap miss on fallback paths
        // is harmless. Saves hundreds of MB of HashMap entries on designs
        // with large testbench memories.
        // For very large arrays (e.g. testbench memory models with 2M
        // elements per ram), skip per-element insertion into
        // `signal_name_to_id` and `id_to_name`. The bytecode hot path
        // (LoadArrayElem / NbaAssignArray) and `fast_signal_write` use
        // the `array_first_id` fast-path resolver instead, so functional
        // correctness is preserved while saving 70+ B per element ×
        // millions of elements (3+ GB on c910). Threshold defaults to
        // 1024 — small enough that E902/picorv32 (which have arrays of
        // 64K element 32-bit ROM) drop their bloat too, but large enough
        // that ordinary 16/32-entry register files keep their per-element
        // names for VCD/probe convenience.
        // Threshold above which 1D-array per-element entries are NOT
        // registered in `signal_name_to_id` / `id_to_name`. Default
        // 100,000 keeps E902-class memories (65,536 × 32-bit
        // mem_inst_temp) named (so any legacy code path that goes
        // through signal_name_to_id resolution still works) while
        // still pruning c910-class testbench RAMs (16 × 2,097,152 =
        // 33 M elements, ~3 GB savings on c910 hello).
        let large_array_threshold: usize = std::env::var("XEZIM_LARGE_ARRAY_NAME_THRESHOLD")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(100_000);
        // Arrays of at least this many cells get VIRTUAL element names (see
        // `NameMap`); smaller ones store theirs.
        // `XEZIM_VIRTUAL_NAME_MIN_CELLS` overrides (0 virtualizes every
        // named 1-D array).
        let virtual_name_min_cells: usize = std::env::var("XEZIM_VIRTUAL_NAME_MIN_CELLS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(257);
        // For unnamed (skipped-large-array) elements, we don't push to
        // id_to_name at all. id_to_name ends up shorter than signal_table.
        // Helpers like `name_for_id(id)` synthesize the name on demand
        // from `array_first_id` for ids >= id_to_name.len().  Saves 16 B
        // per skipped element (~528 MB on c910).  `unnamed_arc` is kept
        // around as a stable shared Arc<""> for any remaining call sites
        // that want a borrow without unwrapping; not used in the loop.
        let _unnamed_arc: Arc<str> = Arc::from("");
        // When XEZIM_INIT_ZERO=1, array elements (which use Value::new = all-X)
        // also start at zero. c910 has memory-style PRF/AIQ/ROB arrays whose
        // X-init leaks through the forwarding network; zeroing only `module.signals`
        // (the prior behavior) is insufficient.
        // Memory/array element init: cheap construction-time zeroing (arrays
        // are read via element ops that never edge-trigger, so coercion is
        // sound here — the analogue of commercial memory-init options). Enabled by
        // XEZIM_INIT_MEM=0 or the legacy XEZIM_INIT_ZERO=1 alias.
        let init_mem = std::env::var("XEZIM_INIT_MEM").ok().as_deref() == Some("0") || init_zero;
        let array_init = if init_mem { Value::zero } else { Value::new };
        // PERF: takes `&str`, not `String` — the callers build each element
        // name into a reused scratch buffer, so the only allocation left per
        // element is the `Arc<str>` the tables actually keep (previously a
        // throwaway `format!` String was allocated and freed as well, 1.56 M
        // times on c906).
        let push_elem_named = |name: &str,
                               w: u32,
                               sig_table: &mut Vec<Value>,
                               widths_vec: &mut Vec<u32>,
                               signed_vec: &mut Vec<bool>,
                               real_vec: &mut Vec<bool>,
                               name_to_id: &mut NameMap,
                               names: &mut IdNames,
                               register_name: bool,
                               _placeholder: &Arc<str>| {
            let id = sig_table.len();
            if register_name {
                let arc: Arc<str> = Arc::from(name);
                name_to_id.insert(arc.clone(), id);
                names.push(arc);
            } else {
                // No stored name, but keep `id_to_name` aligned with ids.
                names.push_gap(1, false);
            }
            sig_table.push(array_init(w));
            widths_vec.push(w);
            signed_vec.push(false);
            real_vec.push(false);
        };
        let push_elem = |name: &str,
                         w: u32,
                         sig_table: &mut Vec<Value>,
                         widths_vec: &mut Vec<u32>,
                         signed_vec: &mut Vec<bool>,
                         real_vec: &mut Vec<bool>,
                         name_to_id: &mut NameMap,
                         names: &mut IdNames| {
            push_elem_named(
                name,
                w,
                sig_table,
                widths_vec,
                signed_vec,
                real_vec,
                name_to_id,
                names,
                true,
                &_unnamed_arc,
            );
        };
        // Static signals/parameters reserved only `n` slots above; unpacked
        // arrays can add millions more signal IDs. Reserve once so large
        // memory construction does not repeatedly grow/copy the SoA vectors.
        // Sanity guard: an unpacked array whose index range exceeds 1 G
        // elements (1 << 30) is never a real design — it's the symptom of a
        // parameter that resolved to 0 so a `[N-1:0]` dimension underflowed to
        // ~u32::MAX. Materializing it would allocate one Signal per element
        // (tens of GB) and OOM the process. Refuse with a clear error instead
        // of crashing. Real large memories (c910 ~2 M entries) are far below
        // this. Checked per array — a single oversized array is the pathology;
        // many legitimate banks may sum higher.
        const MAX_ARRAY_ELEMENTS: usize = 1 << 30;
        let check_array_size = |name: &str, count: usize| {
            if count > MAX_ARRAY_ELEMENTS {
                eprintln!(
                    "[xezim][error] array '{}' has {} elements (> {} = 1G index limit); \
                     refusing to allocate — a dimension parameter likely resolved to 0, \
                     underflowing `[N-1:0]` to a near-u32::MAX range",
                    name, count, MAX_ARRAY_ELEMENTS
                );
                std::process::exit(1);
            }
        };
        let mut array_elem_count: usize = 0;
        // How many of those elements will actually be REGISTERED by name
        // below (1-D arrays at or under `large_array_threshold` keep their
        // per-element names; 2-D / N-D always do). Used only to size
        // `signal_name_to_id` / `id_to_name` up front — without it those two
        // grow from `n` to ~1.5 M entries on c906 by repeated doubling, and
        // every rehash re-hashes every `Arc<str>` key.
        let mut named_elem_count: usize = 0;
        for (name, &(lo, hi, _)) in module.arrays.iter() {
            let count = (hi - lo + 1).max(0) as usize;
            check_array_size(name, count);
            array_elem_count = array_elem_count.saturating_add(count);
            // Only collection elements are stored names; a plain array's
            // element names are virtual (see the 1-D loop below).
            if count <= large_array_threshold
                && (count < virtual_name_min_cells
                    || name.ends_with(']')
                    || module.queue_vars.contains(name.as_str())
                    || module.dynamic_arrays.contains(name.as_str())
                    || module.associative_arrays.contains_key(name.as_str()))
            {
                named_elem_count = named_elem_count.saturating_add(count);
            }
        }
        for (name, &((lo1, hi1), (lo2, hi2), _)) in module.arrays_2d.iter() {
            let d1 = (hi1 - lo1 + 1).max(0) as usize;
            let d2 = (hi2 - lo2 + 1).max(0) as usize;
            let count = d1.saturating_mul(d2);
            check_array_size(name, count);
            array_elem_count = array_elem_count.saturating_add(count);
            named_elem_count = named_elem_count.saturating_add(count);
        }
        for (name, (shape, _)) in module.arrays_nd.iter() {
            let count = shape.iter().fold(1usize, |acc, &(lo, hi)| {
                acc.saturating_mul((hi - lo + 1).max(0) as usize)
            });
            check_array_size(name, count);
            array_elem_count = array_elem_count.saturating_add(count);
            named_elem_count = named_elem_count.saturating_add(count);
        }
        // Free the elaboration-time `signals` map BEFORE allocating the
        // (potentially multi-GB) array-element storage below — its data
        // (per-signal width, signed/real, type_name) is already projected into
        // the parallel signal_widths/signal_signed/signal_real/
        // signal_type_names vectors + signal_name_to_id, which the sim uses
        // instead. Clearing it here (rather than at end of construction) keeps
        // its ~one-long-name-string-plus-Signal-per-entry footprint out of the
        // peak-RSS window during the big array reserve.
        // Toggle coverage needs the declarations too (see
        // `cov_toggle_candidates`); empty unless the run asked for it.
        let code_cov_toggle_names = Self::cov_toggle_candidates(&module);
        // Snapshot array-element values before the map is dropped: the array
        // builder below re-registers these names onto freshly zeroed slots.
        let array_elem_inits: Vec<(String, Value)> = module
            .signals
            .iter()
            .filter(|(k, _)| k.ends_with(']'))
            .filter(|(k, _)| {
                k.rsplit_once('[')
                    .is_some_and(|(b, _)| module.arrays.contains_key(b))
            })
            .map(|(k, s)| (k.clone(), s.value.clone()))
            .collect();
        let arrays_with_elab_values: HashSet<String> = array_elem_inits
            .iter()
            .filter_map(|(name, _)| name.rsplit_once('[').map(|(base, _)| base.to_string()))
            .collect();
        module.signals = Default::default();
        // Return those pages to the OS before the array storage below claims
        // fresh memory (see `release_free_memory`).
        xezim_core::release_free_memory();
        signal_table.reserve(array_elem_count);
        signal_widths_vec.reserve(array_elem_count);
        signal_signed_vec.reserve(array_elem_count);
        signal_real_vec.reserve(array_elem_count);
        signal_name_to_id.reserve(named_elem_count);
        id_to_name.reserve(named_elem_count);

        // 1D unpacked arrays: track `(first_id, lo, hi)` per array in
        // `array_first_id` for the bytecode hot path (LoadArrayElem /
        // NbaAssignArray) which then resolves a cell to a signal_id via
        // a single arithmetic op instead of `format!()` + HashMap lookup.
        // Per-element entries are still pushed into `signal_name_to_id` /
        // `id_to_name` so the slow ad-hoc paths (assign_value fallthrough,
        // get_signal_value_by_name, $readmemh, VCD dump, etc.) keep
        // working unchanged. The compact resolver is a *fast path*, not a
        // replacement for the per-element entries.
        let mut array_first_id: HashMap<Arc<str>, (usize, i64, i64)> =
            HashMap::with_capacity_and_hasher(module.arrays.len(), Default::default());
        let phase_arrays_1d = crate::WallTimer::now();
        let _ = (
            "[ARR] arrays={} arrays_2d={} arrays_nd={} signals_pre={} names_pre={}",
            module.arrays.len(),
            module.arrays_2d.len(),
            module.arrays_nd.len(),
            module.signals.len(),
            n,
        );
        // Sort by base name so signal_id allocation is deterministic
        // across runs (HashMap iteration is non-deterministic, which
        // makes downstream block-dispatch order race-dependent in
        // designs like c910 where many arrays share the same scope).
        let mut arrays_sorted: Vec<(&String, &(i64, i64, u32))> = module.arrays.iter().collect();
        arrays_sorted.sort_by(|a, b| a.0.cmp(b.0));
        // Scratch buffer for per-element names (see `push_elem_named`).
        let mut elem_name_buf = String::with_capacity(96);
        // Memory census (`XEZIM_MEM_CENSUS=1`): size the word-aggregation
        // opportunity before changing any representation. Every array cell
        // currently costs a full `Value` plus one entry in each per-signal side
        // table (width/signed/real/inline-bits/has_xz/dirty), which for a
        // byte-wide RAM is ~60 B to carry 8 bits of data. Packed word storage
        // would cost `ceil(w/8)` bytes for the value and the same for the x/z
        // mask. Reports per-array and in total so the go/no-go is a number, not
        // an argument -- same census-first shape the AOT template dedup used.
        let mem_census = std::env::var_os("XEZIM_MEM_CENSUS").is_some();
        let mut census: Vec<(String, usize, u32)> = Vec::new();
        let packed_mem = packed_mem_enabled();
        let mut packed_mem_arena = PackedMem::default();
        // Reserve the arena once for every array that can be admitted (an
        // upper bound: rejected arrays leave their share untouched, so it
        // never becomes resident). Growing by doubling instead copied the
        // whole arena at each step and briefly held two copies of it.
        if packed_mem {
            let bound: usize = module
                .arrays
                .values()
                .map(|&(lo, hi, w)| ((hi - lo + 1).max(0) as usize, w))
                .filter(|&(count, w)| count > large_array_threshold && (1..=64).contains(&w))
                .map(|(count, w)| {
                    count.saturating_mul(2 * (w as usize).div_ceil(8).next_power_of_two())
                        + PACKED_PAD
                })
                .fold(0usize, usize::saturating_add);
            packed_mem_arena.reserve(bound);
        }
        let mut packed_arrays: std::collections::HashSet<String> =
            std::collections::HashSet::default();
        let mut packed_cells: usize = 0;
        for (base, &(lo, hi, w)) in arrays_sorted {
            let count = (hi - lo + 1).max(0) as usize;
            if mem_census && count > 0 {
                census.push((base.clone(), count, w));
            }
            let register_names = count <= large_array_threshold;
            // Resolve element traits before packed admission: arena ids have
            // no entries in the ordinary signed/two-state side tables.
            let elem_dt = module.var_decl_types.get(base).or_else(|| {
                let mut b: &str = base;
                loop {
                    match b.rfind('[') {
                        Some(p) => {
                            b = &b[..p];
                            if let Some(dt) = module.var_decl_types.get(b) {
                                break Some(dt);
                            }
                        }
                        None => break None,
                    }
                }
            });
            let elem_signed = elem_dt.is_some_and(|dt| {
                super::elaborate::is_type_signed_resolved(dt, &module.typedef_types)
            });
            // Packed admission (see `PackedMem`): an array big enough to
            // forgo per-element names whose element is a plain integral
            // variable of at most 64 bits. Cells get ids above PACKED_BASE
            // and no `signal_table` / side-table entries at all — that
            // omission IS the saving. Everything with storage semantics the
            // arena does not model stays an ordinary array: strings (dynamic
            // length), reals, class handles and other non-integral elements,
            // queue / dynamic / associative collections (which `module.arrays`
            // also lists), nets, and arrays already holding elaboration
            // values (parameters).
            // Two-state-ness mirrors the ordinary array exactly: elements
            // zero-initialize when the BASE is a 2-state signal and drop x/z
            // on stores when the base OR its leaf is (see the fixups below).
            // An array where the two tests disagree stays ordinary.
            let ts_base = module.two_state_signals.contains(base.as_str());
            let ts_any = ts_base
                || module
                    .two_state_signals
                    .contains(base.rsplit('.').next().unwrap_or(base));
            if packed_mem
                && !register_names
                && count > 0
                && ts_base == ts_any
                && !arrays_with_elab_values.contains(base)
                && Self::packed_admissible(&module, base, w)
            {
                let init = if ts_base {
                    Value::zero(w)
                } else {
                    array_init(w)
                };
                if let Some(first_id) =
                    packed_mem_arena.admit(count, w, elem_signed, ts_base, init.raw_bits())
                {
                    array_first_id.insert(Arc::from(base.as_str()), (first_id, lo, hi));
                    packed_arrays.insert(base.clone());
                    packed_cells += count;
                    continue;
                }
            }
            let first_id = signal_table.len();
            array_first_id.insert(Arc::from(base.as_str()), (first_id, lo, hi));
            // A named array's element names are VIRTUAL (see `NameMap`):
            // `base[idx]` resolves arithmetically and is never stored.
            // Collections keep real per-element entries — their element
            // names are enumerated by prefix and their fake 0..63 backing
            // range is not the collection's extent — and so does an
            // array-of-collection element registered under a compound base
            // (`aq[0]`; a generate-scope bracket inside the path is fine).
            // Small arrays keep stored names too: their few entries cost
            // little, and a testbench's (a UVM BFM's 256-entry scoreboards)
            // are read by name at run time, where a stored name is one hash
            // probe and a virtual one two.
            // Only an array with a declaration of its own is virtual: a
            // struct member's array (`s.arr`) and a parameter array holding
            // elaboration values are reached through their element names by
            // prefix walks and the restore below, which keep stored names.
            let virtual_names = register_names
                && count >= virtual_name_min_cells
                && module.var_decl_types.contains_key(base.as_str())
                && !arrays_with_elab_values.contains(base.as_str())
                && !base.ends_with(']')
                && !module.queue_vars.contains(base.as_str())
                && !module.dynamic_arrays.contains(base.as_str())
                && !module.associative_arrays.contains_key(base.as_str());
            if register_names && !virtual_names {
                for idx in lo..=hi {
                    // Same bytes `format!("{}[{}]", base, idx)` produced, into
                    // a buffer reused across the whole array.
                    elem_name_buf.clear();
                    elem_name_buf.push_str(base);
                    elem_name_buf.push('[');
                    Self::push_i64(&mut elem_name_buf, idx);
                    elem_name_buf.push(']');
                    push_elem_named(
                        elem_name_buf.as_str(),
                        w,
                        &mut signal_table,
                        &mut signal_widths_vec,
                        &mut signal_signed_vec,
                        &mut signal_real_vec,
                        &mut signal_name_to_id,
                        &mut id_to_name,
                        true,
                        &_unnamed_arc,
                    );
                }
            } else {
                // Large arrays deliberately omit per-element names, and a
                // virtual-name array stores none. Extend the SoA vectors
                // directly instead of formatting strings that will be
                // discarded; array_first_id still gives each cell a stable
                // contiguous signal_id.
                signal_table.extend(std::iter::repeat_n(array_init(w), count));
                signal_widths_vec.extend(std::iter::repeat_n(w, count));
                signal_signed_vec.extend(std::iter::repeat_n(false, count));
                signal_real_vec.extend(std::iter::repeat_n(false, count));
                id_to_name.push_gap(count, virtual_names);
                if virtual_names {
                    signal_name_to_id.add_virtual_array(Arc::from(base.as_str()), first_id, lo, hi);
                }
            }
            // §6.11.1: the array's elements inherit the declared element type's
            // signedness (`byte foo[8]` elements are signed). The push helpers
            // default every element to unsigned, so a signed-element array would
            // otherwise read its cells as unsigned. An array-of-queue registers
            // each element as its own 1-D array under a COMPOUND base
            // ("aq[0]") while var_decl_types is keyed by the declaration name
            // ("aq") — strip index suffixes until the declaration is found,
            // or `int aq[2][$]` elements read back unsigned.
            if let Some(dt) = elem_dt {
                if super::elaborate::is_type_signed_resolved(dt, &module.typedef_types) {
                    for id in first_id..signal_table.len() {
                        signal_signed_vec[id] = true;
                    }
                }
                // Element REALNESS inheritance (`real T[4]`): the push
                // helpers default every element to integral, so the
                // two-state lowering admitted real arrays and read/wrote
                // their elements as raw f64 BITS (a compiled `T[k] + x`
                // returned 4.6e18-looking garbage while the AST path was
                // right). Everything keying on per-element `signal_real`
                // needs this — same shape as the §6.11.1 signedness loop.
                if super::elaborate::is_type_real_resolved(dt, &module.typedef_types) {
                    for id in first_id..signal_table.len() {
                        signal_real_vec[id] = true;
                    }
                }
            }
        }
        // An array whose elements already carry values from elaboration — an
        // unpacked-array PARAMETER (`u32_t A[N] = {a, b}`, §6.20.2) — must keep
        // them: the loops above zero-fill every element they create.
        // Elements the loop above just zero-filled, but which carry a value from
        // elaboration (an unpacked-array PARAMETER, §6.20.2): restore them. Their
        // names were re-pointed at fresh array slots when the array was built.
        // §6.8: elements of an array with a 2-STATE element type default to
        // 0, not X — `bit [7:0] mem[4]; mem[i]++` must count. (Scalar decls
        // already zero via default_value_for_type; the element storage built
        // above is X-filled.) Runs BEFORE the elaboration-value restore so a
        // parameter array's real init values still win.
        for base in module.two_state_signals.iter() {
            if let Some(&(first_id, lo, hi)) = array_first_id.get(base.as_str()) {
                if is_packed_id(first_id) {
                    continue;
                }
                let n = (hi - lo + 1).max(0) as usize;
                for id in first_id..first_id + n {
                    signal_table[id] = Value::zero(signal_widths_vec[id]);
                }
            }
        }
        // §25.9 / Table 6-7: every element of an unpacked array of virtual
        // interfaces (`vif_t va[2][3];`, each dimension registered under a
        // compound base `va[0]`) starts as the null handle.
        if !vif_vars.is_empty() {
            for (base, &(first_id, lo, hi)) in array_first_id.iter() {
                let decl = base.split('[').next().unwrap_or(base);
                if is_packed_id(first_id) || !vif_vars.contains(decl) {
                    continue;
                }
                let n = (hi - lo + 1).max(0) as usize;
                for id in first_id..first_id + n {
                    signal_table[id] = Value::zero(signal_widths_vec[id]);
                }
            }
        }
        for base in module.z_init_signals.iter() {
            if let Some(&(first_id, lo, hi)) = array_first_id.get(base.as_str()) {
                if is_packed_id(first_id) {
                    continue;
                }
                let n = (hi - lo + 1).max(0) as usize;
                for id in first_id..first_id + n {
                    signal_table[id] = Value::all_z(signal_widths_vec[id]);
                }
            }
        }
        if packed_mem && packed_cells > 0 && (mem_census || crate::verbose()) {
            eprintln!(
                "[PACKED-MEM] arrays={} cells={} arena={:.2} MB (vs {:.2} MB unpacked)",
                packed_arrays.len(),
                packed_cells,
                packed_mem_arena.bytes() as f64 / 1e6,
                (packed_cells * (std::mem::size_of::<Value>() + 24)) as f64 / 1e6,
            );
        }
        if mem_census {
            // Per-cell cost today: the Value itself plus the fixed-size
            // per-signal side tables that are indexed by signal_id.
            const SIDE_BYTES: usize = 4 /*width*/ + 1 /*signed*/ + 1 /*real*/
                + 16 /*inline_bits*/ + 1 /*has_xz*/ + 1 /*dirty*/;
            let per_cell_now = std::mem::size_of::<Value>() + SIDE_BYTES;
            census.sort_by(|a, b| b.1.cmp(&a.1));
            let (mut tot_cells, mut tot_now, mut tot_packed) = (0usize, 0usize, 0usize);
            eprintln!(
                "[MEM-CENSUS] Value={}B side={}B per_cell={}B",
                std::mem::size_of::<Value>(),
                SIDE_BYTES,
                per_cell_now
            );
            for (name, count, w) in census.iter().take(20) {
                let packed = ((*w as usize).div_ceil(8)) * 2 * count;
                eprintln!(
                    "[MEM-CENSUS] {:>12} cells x {:>4}b  now {:>7.1} MB  packed {:>6.1} MB  {:>5.1}x  {}",
                    count,
                    w,
                    (count * per_cell_now) as f64 / 1e6,
                    packed as f64 / 1e6,
                    (count * per_cell_now) as f64 / packed.max(1) as f64,
                    name,
                );
            }
            for (_, count, w) in &census {
                tot_cells += count;
                tot_now += count * per_cell_now;
                tot_packed += ((*w as usize).div_ceil(8)) * 2 * count;
            }
            eprintln!(
                "[MEM-CENSUS] TOTAL arrays={} cells={} now={:.2} GB packed={:.2} GB ratio={:.1}x",
                census.len(),
                tot_cells,
                tot_now as f64 / 1e9,
                tot_packed as f64 / 1e9,
                tot_now as f64 / tot_packed.max(1) as f64,
            );
            // Named (per-element names registered) vs unnamed 1-D arrays by
            // cell-count band and element width: sizes the next admission
            // question — how many cells still live as named signals.
            let bands: [(usize, usize, &str); 5] = [
                (0, 16, "<=16"),
                (17, 256, "17..256"),
                (257, 4096, "257..4K"),
                (4097, large_array_threshold, "4K..named"),
                (large_array_threshold + 1, usize::MAX, "unnamed"),
            ];
            for (lo_b, hi_b, label) in bands {
                let mut by_w = [(0usize, 0usize); 5];
                for (_, count, w) in &census {
                    if *count < lo_b || *count > hi_b {
                        continue;
                    }
                    let k = match *w {
                        0..=8 => 0,
                        9..=16 => 1,
                        17..=32 => 2,
                        33..=64 => 3,
                        _ => 4,
                    };
                    by_w[k].0 += 1;
                    by_w[k].1 += count;
                }
                eprintln!(
                    "[MEM-CENSUS] arrays {:>10}: w<=8 {}/{}  w<=16 {}/{}  w<=32 {}/{}  w<=64 {}/{}  wide {}/{} (arrays/cells)",
                    label,
                    by_w[0].0,
                    by_w[0].1,
                    by_w[1].0,
                    by_w[1].1,
                    by_w[2].0,
                    by_w[2].1,
                    by_w[3].0,
                    by_w[3].1,
                    by_w[4].0,
                    by_w[4].1
                );
            }
        }
        let arrays_1d_ms = phase_arrays_1d.elapsed().as_secs_f64() * 1000.0;
        crate::rss_trace("sim::new arrays 1d");
        let phase_arrays_other = std::time::Instant::now();
        let mut arrays_2d_sorted: Vec<(&String, &((i64, i64), (i64, i64), u32))> =
            module.arrays_2d.iter().collect();
        arrays_2d_sorted.sort_by(|a, b| a.0.cmp(b.0));
        for (base, &((lo1, hi1), (lo2, hi2), w)) in arrays_2d_sorted {
            let first_id = signal_table.len();
            for i in lo1..=hi1 {
                for j in lo2..=hi2 {
                    push_elem(
                        format!("{}[{}][{}]", base, i, j).as_str(),
                        w,
                        &mut signal_table,
                        &mut signal_widths_vec,
                        &mut signal_signed_vec,
                        &mut signal_real_vec,
                        &mut signal_name_to_id,
                        &mut id_to_name,
                    );
                }
            }
            // §25.9: a virtual-interface element starts null (see the 1-D
            // loop).
            if vif_vars.contains(base.as_str()) {
                for id in first_id..signal_table.len() {
                    signal_table[id] = Value::zero(signal_widths_vec[id]);
                }
            }
            // §6.11.1: same element-signedness inheritance the 1-D loop
            // applies — without it `int a2[2][2]; a2[0][0] = -1` read back
            // unsigned (4294967295, and `a2[0][0] < 0` was FALSE).
            if module.var_decl_types.get(base).is_some_and(|dt| {
                super::elaborate::is_type_signed_resolved(dt, &module.typedef_types)
            }) {
                for id in first_id..signal_table.len() {
                    signal_signed_vec[id] = true;
                }
            }
            // Element REALNESS inheritance — see the 1-D loop.
            if module.var_decl_types.get(base).is_some_and(|dt| {
                super::elaborate::is_type_real_resolved(dt, &module.typedef_types)
            }) {
                for id in first_id..signal_table.len() {
                    signal_real_vec[id] = true;
                }
            }
        }
        let mut arrays_nd_sorted: Vec<(&String, &(Vec<(i64, i64)>, u32))> =
            module.arrays_nd.iter().collect();
        arrays_nd_sorted.sort_by(|a, b| a.0.cmp(b.0));
        for (base, (shape, w)) in arrays_nd_sorted {
            fn enumerate(dims: &[(i64, i64)], prefix: String, out: &mut Vec<String>) {
                if dims.is_empty() {
                    out.push(prefix);
                    return;
                }
                let (lo, hi) = dims[0];
                for i in lo..=hi {
                    enumerate(&dims[1..], format!("{}[{}]", prefix, i), out);
                }
            }
            let mut names = Vec::new();
            enumerate(shape, base.clone(), &mut names);
            let first_id = signal_table.len();
            for name in names {
                push_elem(
                    name.as_str(),
                    *w,
                    &mut signal_table,
                    &mut signal_widths_vec,
                    &mut signal_signed_vec,
                    &mut signal_real_vec,
                    &mut signal_name_to_id,
                    &mut id_to_name,
                );
            }
            if vif_vars.contains(base.as_str()) {
                for id in first_id..signal_table.len() {
                    signal_table[id] = Value::zero(signal_widths_vec[id]);
                }
            }
            // §6.11.1: element-signedness inheritance (see the 1-D / 2-D
            // loops above).
            if module.var_decl_types.get(base).is_some_and(|dt| {
                super::elaborate::is_type_signed_resolved(dt, &module.typedef_types)
            }) {
                for id in first_id..signal_table.len() {
                    signal_signed_vec[id] = true;
                }
            }
            // Element REALNESS inheritance — see the 1-D loop.
            if module.var_decl_types.get(base).is_some_and(|dt| {
                super::elaborate::is_type_real_resolved(dt, &module.typedef_types)
            }) {
                for id in first_id..signal_table.len() {
                    signal_real_vec[id] = true;
                }
            }
        }
        // Elements that carry a value from ELABORATION (an unpacked-array
        // parameter, §6.20.2). Deliberately after the 2-D / N-D creation
        // loops: those rebuild storage for their names, so restoring earlier
        // left a multi-dimensional parameter array zero-filled.
        for (nm, v) in array_elem_inits {
            if let Some(&id) = signal_name_to_id.get(nm.as_str()) {
                signal_table[id] = v;
            }
        }
        // §6.8 again for the 2-D / N-D element storage (named slots).
        for base in module.two_state_signals.iter() {
            if !module.arrays_2d.contains_key(base) && !module.arrays_nd.contains_key(base) {
                continue;
            }
            let prefix = format!("{}[", base);
            for (name, &id) in signal_name_to_id.iter() {
                if name.starts_with(prefix.as_str()) {
                    signal_table[id] = Value::zero(signal_widths_vec[id]);
                }
            }
        }
        for base in module.z_init_signals.iter() {
            if !module.arrays_2d.contains_key(base) && !module.arrays_nd.contains_key(base) {
                continue;
            }
            let prefix = format!("{}[", base);
            for (name, &id) in signal_name_to_id.iter() {
                if name.starts_with(prefix.as_str()) {
                    signal_table[id] = Value::all_z(signal_widths_vec[id]);
                }
            }
        }
        let arrays_other_ms = phase_arrays_other.elapsed().as_secs_f64() * 1000.0;
        crate::rss_trace("sim::new arrays 2d/nd");
        // num_signals = total signal_id count. id_to_name may be SHORTER
        // (we skip pushing for unnamed large-array elements) but
        // signal_table grows for every element, so use signal_table.len().
        let num_signals = signal_table.len();
        // §6.11.1/§10.7: mark 2-state signal ids so implicit conversions of a
        // 4-state RHS drop X/Z at the destination. `two_state_signals` stores
        // bare declaration names, while signal_name_to_id keys are scoped and
        // array elements carry `[idx]`; match on the leaf base of each name.
        let mut signal_two_state = vec![false; num_signals];
        // §6.16: mark string-typed ids so assignment does not clamp them to the
        // placeholder width the elaborator assigns a dynamic type.
        let mut signal_is_string = vec![false; num_signals];
        // §28.4 vs §10.3: mark ids driven by a lowered gate primitive, so only
        // those get the `buf` truth table's z->x. Same leaf/base matching as
        // the 2-state pass.
        let mut signal_gate_driven = vec![false; num_signals];
        let want_two_state = !module.two_state_signals.is_empty();
        let want_string = !module.string_signals.is_empty();
        let want_gate = !module.gate_driven_nets.is_empty();
        // ONE walk of `signal_name_to_id` (1.56 M entries on c906) instead of
        // three. The three marking passes asked independent questions about the
        // same name but each re-walked the map — chasing every `Arc<str>` out
        // of cache again — and each recomputed the identical `leaf`/`base`
        // split, i.e. a `memrchr` plus a `memchr` per name PER PASS. The
        // per-name predicates below are byte-for-byte the ones each pass used
        // (note the 2-state test deliberately does not consult `leaf`), and the
        // `array_first_id` fixups that follow are unchanged.
        //
        // An ELEMENT of a string collection is a string too. `string_signals`
        // holds the base name (`q`, `aa`), while the table holds `q[2]`, so the
        // subscript is stripped before matching — otherwise a `string q[$]`
        // element kept being clamped to the placeholder width while a plain
        // `string` variable no longer was.
        let is_str_name = |n: &str| -> bool {
            let leaf = n.rsplit('.').next().unwrap_or(n);
            let base = leaf.split('[').next().unwrap_or(leaf);
            module.string_signals.contains(n)
                || module.string_signals.contains(leaf)
                || module.string_signals.contains(base)
        };
        if want_two_state || want_string || want_gate {
            // The three sets are small next to the name table, so an exact
            // (length, last byte) prefilter answers almost every probe
            // without hashing the whole name.
            let two_state_f = ShapeFilter::new(module.two_state_signals.iter().map(String::as_str));
            let string_f = ShapeFilter::new(module.string_signals.iter().map(String::as_str));
            let gate_f = ShapeFilter::new(module.gate_driven_nets.iter().map(String::as_str));
            let two_state_has =
                |s: &str| two_state_f.may_contain(s) && module.two_state_signals.contains(s);
            let string_has = |s: &str| string_f.may_contain(s) && module.string_signals.contains(s);
            let gate_has = |s: &str| gate_f.may_contain(s) && module.gate_driven_nets.contains(s);
            for (name, &id) in signal_name_to_id.iter() {
                if id >= num_signals {
                    continue;
                }
                let name_ref: &str = name.as_ref();
                let (leaf, base) = leaf_and_base(name_ref);
                if want_two_state && (two_state_has(name_ref) || two_state_has(base)) {
                    signal_two_state[id] = true;
                }
                if want_string && (string_has(name_ref) || string_has(leaf) || string_has(base)) {
                    signal_is_string[id] = true;
                }
                if want_gate && (gate_has(name_ref) || gate_has(leaf) || gate_has(base)) {
                    signal_gate_driven[id] = true;
                }
            }
        }
        if want_two_state {
            for (base_name, &(first_id, lo, hi)) in array_first_id.iter() {
                let base_ref: &str = base_name.as_ref();
                let leaf = base_ref.rsplit('.').next().unwrap_or(base_ref);
                if module.two_state_signals.contains(base_ref)
                    || module.two_state_signals.contains(leaf)
                {
                    let n = (hi - lo + 1).max(0) as usize;
                    for id in first_id..(first_id + n).min(num_signals) {
                        signal_two_state[id] = true;
                    }
                }
            }
        }
        // A struct member's packed element width is registered under the
        // DECLARATION name (`ref_pipe.wdata`), but an unpacked array of structs
        // stores each element as its own signal (`ref_pipe[2].wdata`). Without a
        // key for the concrete signal, `ref_pipe[i].wdata[j]` degraded from a
        // lane slice to a 1-BIT select — silently comparing one bit against a
        // whole lane. Alias every element name to its declaration's entry.
        //
        // Skipped wholesale when the width map is empty: with nothing to alias
        // TO, every lookup below misses and no alias is ever produced, so the
        // walk over all 1.56 M names is pure cost. The `]`-shape test also
        // moved ahead of the `contains_key` probe — both outcomes are the same
        // `continue`, and a name with no `]` can never alias, so it no longer
        // pays for a full-name hash.
        if !module.packed_signal_elem_widths.is_empty() {
            let mut aliases: Vec<(String, u32)> = Vec::new();
            for name in signal_name_to_id.keys() {
                let name_ref: &str = name.as_ref();
                // `base[i].member…` -> `base.member…`
                let Some(br) = name_ref.find(']') else {
                    continue;
                };
                let Some(bl) = name_ref[..br].rfind('[') else {
                    continue;
                };
                if !name_ref[br..].starts_with("].") {
                    continue;
                }
                if module.packed_signal_elem_widths.contains_key(name_ref) {
                    continue;
                }
                let deindexed = format!("{}{}", &name_ref[..bl], &name_ref[br + 1..]);
                // The map is keyed by the DECLARATION name, which carries no
                // instance scope, so try the de-indexed name and then each
                // scope-stripped suffix of it.
                let mut cand: &str = &deindexed;
                let ew = loop {
                    if let Some(&ew) = module.packed_signal_elem_widths.get(cand) {
                        break Some(ew);
                    }
                    match cand.find('.') {
                        // Stop before stripping the member itself.
                        Some(i) if cand[i + 1..].contains('.') => cand = &cand[i + 1..],
                        _ => break None,
                    }
                };
                if let Some(ew) = ew {
                    aliases.push((name_ref.to_string(), ew));
                }
            }
            for (k, ew) in aliases {
                module.packed_signal_elem_widths.insert(k, ew);
            }
        }
        if want_gate {
            // Elements of a virtual-name array (see `NameMap`) have no stored
            // name for the per-name pass above: test each element name the
            // way that pass would have.
            let mut elem = String::new();
            for (base, first_id, lo, hi) in signal_name_to_id.virtual_arrays() {
                let leaf_base = base.rsplit('.').next().unwrap_or(base);
                let whole = module.gate_driven_nets.contains(leaf_base);
                for idx in lo..=hi {
                    let id = first_id + (idx - lo) as usize;
                    if id >= num_signals {
                        break;
                    }
                    if !whole {
                        use std::fmt::Write as _;
                        elem.clear();
                        let _ = write!(elem, "{}[{}]", base, idx);
                        let leaf = elem.rsplit('.').next().unwrap_or(elem.as_str());
                        if !module.gate_driven_nets.contains(elem.as_str())
                            && !module.gate_driven_nets.contains(leaf)
                        {
                            continue;
                        }
                    }
                    signal_gate_driven[id] = true;
                }
            }
        }
        if want_string {
            // Compact-allocated array elements have no per-element name entry;
            // mark their whole id range from the base.
            for (base_name, &(first_id, lo, hi)) in array_first_id.iter() {
                if !is_str_name(base_name.as_ref()) {
                    continue;
                }
                let n = (hi - lo + 1).max(0) as usize;
                for id in first_id..(first_id + n).min(num_signals) {
                    signal_is_string[id] = true;
                }
            }
        }
        // prev_{val,xz} represent "before time 0" state. Per IEEE 1800,
        // variable initializers `reg x = <v>;` are equivalent to
        // initial-block assignments, so X→<v> at t=0 must generate an edge
        // event for @(posedge x) etc. Initialize to all-X (val=0,
        // xz=mask(width)) so the first check_edges at t=0 detects these
        // initializer-driven transitions.
        // prev_val/prev_xz only need to cover NAMED signal_ids (i.e.
        // those reachable via signal_name_to_id, which is the only
        // edge-block registration path). The cells of an unnamed bulk
        // memory never become edge signals, and a waiter on one compares
        // against its own captured value, so sizing prev_{val,xz} to the
        // named-id bound instead of signal_table.len() saves 16 B per
        // trailing bulk cell. It must be the bound, not the name COUNT: a
        // bulk memory sorting before a named array pushes that array's ids
        // past the count.
        let named_count = id_to_name.named_id_bound();
        let phase_prev = std::time::Instant::now();
        let prev_val: Vec<u64> = vec![0u64; named_count];
        let mut prev_xz: Vec<u64> = vec![0u64; named_count];
        let mut prev_wide: HashMap<usize, Value> = HashMap::default();
        for id in 0..named_count {
            let w = signal_widths_vec[id];
            let mask = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
            prev_xz[id] = mask;
            if w > 64 {
                prev_wide.insert(id, Value::new(w));
            }
        }
        let prev_ms = phase_prev.elapsed().as_secs_f64() * 1000.0;
        crate::rss_trace("sim::new prev snapshot");
        // Free `module.signals` now that the indexed signal_table /
        // signal_widths / signal_signed / signal_real / signal_type_names
        // have absorbed everything we need. On c910 this releases the
        // ~110 MB of `Signal` structs (name + Value + bools + type_name)
        // plus their HashMap overhead. `module.parameters` is kept
        // because `resolve_type_width` still resolves dimension
        // expressions against it at runtime (per step-2's caller fix).
        let phase_drop = std::time::Instant::now();
        module.signals = HashMap::default();
        let drop_ms = phase_drop.elapsed().as_secs_f64() * 1000.0;
        crate::rss_trace("sim::new drop");

        // Pre-compute X/Z bitmap for the JIT inline prelude. Default
        // `Value::new(w)` returns an all-X reg, so this starts mostly 1s
        // and gets cleared as `write_sig!` is called during init/settle.
        let signal_has_xz_init: Vec<u8> = signal_table
            .iter()
            .map(|v| if v.raw_bits().1 != 0 { 1u8 } else { 0u8 })
            .collect();
        let multi_dim_array_names: HashSet<String> = module
            .arrays_2d
            .keys()
            .chain(module.arrays_nd.keys())
            .cloned()
            .collect();
        let mut instance_path_order: Vec<usize> = (0..module.instances.len()).collect();
        instance_path_order
            .sort_unstable_by(|&a, &b| module.instances[a].path.cmp(&module.instances[b].path));
        // Sized up front: this ends up with one entry per DISTINCT leaf, and on
        // c906 nearly every one of the 1.56 M names has its own (array elements
        // carry `[idx]` in the leaf). Growing from empty rehashes the table a
        // dozen times, and hashbrown stores only a 7-bit tag, so each of those
        // resizes re-hashes every `Arc<str>` key's full string.
        let mut scope_parents_by_leaf: HashMap<Arc<str>, Vec<Arc<str>>> =
            HashMap::with_capacity_and_hasher(
                signal_name_to_id.explicit_len() + module.arrays.len(),
                Default::default(),
            );
        // PERF: this walks every named signal (1.56 M on c906) and used to
        // allocate TWO fresh `Arc<str>` per name — one for the leaf (which the
        // `entry` then throws away whenever the leaf is already present) and
        // one for the parent scope. Both repeat massively: there are only as
        // many distinct parents as there are instance scopes. Intern the
        // parents and only allocate a leaf Arc on first sight. The stored
        // values are identical strings either way (they are compared and
        // sorted by CONTENT below and in `best_scope_from_leaf_index`), so
        // sharing the allocations is invisible to behaviour.
        //
        // The walk goes over `id_to_name` (every key of `signal_name_to_id`,
        // in id order, i.e. sorted, with a name re-registered by an array
        // counted twice — the dedup below absorbs that): consecutive names
        // then mostly share their parent, which is reused without a lookup.
        let mut parent_intern: HashMap<Arc<str>, Arc<str>> = HashMap::default();
        let mut last_parent: Option<Arc<str>> = None;
        for full_name in id_to_name
            .iter()
            .map(|(_, name)| name.as_ref())
            .chain(module.arrays.keys().map(String::as_str))
            .chain(module.arrays_2d.keys().map(String::as_str))
            .chain(module.arrays_nd.keys().map(String::as_str))
        {
            let Some(dot) = full_name.bytes().rposition(|b| b == b'.') else {
                continue;
            };
            let (parent, leaf) = (&full_name[..dot], &full_name[dot + 1..]);
            let parent_arc = match &last_parent {
                Some(a) if a.as_ref() == parent => a.clone(),
                _ => {
                    let a = match parent_intern.get(parent) {
                        Some(a) => a.clone(),
                        None => {
                            let a: Arc<str> = Arc::from(parent);
                            parent_intern.insert(a.clone(), a.clone());
                            a
                        }
                    };
                    last_parent = Some(a.clone());
                    a
                }
            };
            match scope_parents_by_leaf.get_mut(leaf) {
                Some(v) => v.push(parent_arc),
                None => {
                    scope_parents_by_leaf.insert(Arc::from(leaf), vec![parent_arc]);
                }
            }
        }
        drop(last_parent);
        drop(parent_intern);
        for parents in scope_parents_by_leaf.values_mut() {
            // A leaf seen under exactly one scope — the common case here, since
            // every distinct array-element name has its own leaf — is already
            // sorted and already unique. Id order usually leaves the rest
            // sorted too.
            if parents.len() > 1 {
                if !parents.is_sorted() {
                    parents.sort_unstable();
                }
                parents.dedup();
            }
        }

        // §6.24.1 named casts: pre-resolve every statically-known cast
        // target (typedef, enum, width-only typedef) to (width, signed) so
        // the bytecode compiler can emit Resize/SetSigned instead of bailing
        // the whole expression to the interpreter. On ibex this cast appears
        // ~8x per cycle and its interpreted evaluation was 43% of a 3M-cycle
        // run. Real-typed targets are deliberately absent (interpreter path).
        let cast_widths: HashMap<String, (u32, bool)> = {
            let mut m: HashMap<String, (u32, bool)> = HashMap::default();
            for (name, dt) in &module.typedef_types {
                if super::elaborate::is_type_real(dt) {
                    continue;
                }
                let w = super::elaborate::resolve_type_width(
                    dt,
                    Some(&module.parameters),
                    Some(&module.typedefs),
                )
                .max(1);
                m.insert(
                    name.clone(),
                    (
                        w,
                        super::elaborate::is_type_signed_resolved(dt, &module.typedef_types),
                    ),
                );
            }
            for (name, &w) in module.typedefs.iter() {
                m.entry(name.clone()).or_insert((w.max(1), false));
            }
            m
        };
        let (hoisted_sub_static_names, hoisted_static_subroutines) =
            Self::hoist_subroutine_statics(&mut module);
        let mut sim = Self {
            prev_val,
            prev_xz,
            prev_wide,
            snap_gen: 0,
            edge_signal_names: HashSet::default(),
            edge_signal_ids: Vec::new(),
            preponed_ids: Vec::new(),
            edge_blocks_by_sig: Vec::new(),
            edge_blocks_with_iff: Vec::new(),
            edge_iff_denied: Vec::new(),
            signals,
            vpi_port_directions,
            widths,
            signed_signals,
            array_local_packed_dims: HashSet::default(),
            local_elem_layout: None,
            real_signals,
            multi_dim_array_names,
            forced_signals: HashMap::default(),
            annex_n_seed: 0,
            has_coll_rows: false,
            partial_forces: HashMap::default(),
            pending_release_ids: Vec::new(),
            prop_raw_ty_cache: std::cell::RefCell::new(HashMap::default()),
            typedef_target_cache: std::cell::RefCell::new(HashMap::default()),
            collection_dim_cache: std::cell::RefCell::new(HashMap::default()),
            any_struct_formal_markers: false,
            pattern_scratch_seq: 0,
            pattern_scratch_free: Vec::new(),
            force_epoch_gen: 0,
            force_refresh_done_gen: 0,
            class_coll_index: std::cell::RefCell::new(HashMap::default()),
            class_non_member_cache: std::cell::RefCell::new(HashMap::default()),
            member_coll_cache: std::cell::RefCell::new(HashMap::default()),
            class_prop_struct_neg: std::cell::RefCell::new(HashMap::default()),
            name_stats: Default::default(),
            name_stats_on: std::env::var("XEZIM_NAME_STATS").is_ok(),
            frame_pool: Vec::new(),
            fb_meta_saves: Vec::new(),
            fb_frame_depths: Vec::new(),
            param_leaf_index_cell: std::cell::OnceCell::new(),
            var_decl_leaf_index_cell: std::cell::OnceCell::new(),
            forced_names: HashSet::default(),
            static_fn_ret: HashMap::default(),
            active_force_exprs: Vec::new(),
            active_force_signal_epochs: vec![0; signal_table.len()],
            active_force_evals: 0,
            active_force_skips: 0,
            signal_has_xz: signal_has_xz_init,
            signal_commit_plan: Vec::new(),
            signal_inline_bits: Vec::new(),
            jit_nba_side_queue: Vec::new(),
            jit_nba_side_len: 0,
            signal_table,
            packed: packed_mem_arena,
            signal_name_to_id,
            scope_parents_by_leaf,
            array_elem_ids: HashMap::default(),
            array_first_id,
            leaf_name_to_ids,
            id_to_name,
            elem_name_cache: RefCell::new(HashMap::default()),
            instance_path_order,
            signal_widths: signal_widths_vec,
            signal_signed: signal_signed_vec,
            signal_real: signal_real_vec,
            signal_two_state,
            signal_gate_driven,
            signal_is_string,
            warn_x: warn_x_enabled(),
            warn_x_seen: std::cell::RefCell::new(HashSet::default()),
            warn_x_pending: std::cell::RefCell::new(Vec::new()),
            warn_x_seen_named: std::cell::RefCell::new(HashSet::default()),
            warn_x_count: 0,
            warn_x_drivers: None,
            cont_driven: HashSet::default(),
            collapsed_port_children: HashSet::default(),
            collapse_settled: None,
            deferred_always,
            signal_type_names,
            time: 0,
            output: Vec::new(),
            capture_output: true,
            finished: false,
            saw_fatal: false,
            fatal_finish_number: None,
            error_count: 0,
            compile_errors: Vec::new(),
            stuck_clock_aborted: false,
            compiled: false,
            monitor: None,
            monitor_paused: false,
            warned_system_tasks: HashSet::default(),
            monitor_prev: HashMap::default(),
            monitor_arg_prev: None,
            monitor_scope: String::new(),
            monitor_m_suffix: String::new(),
            monitor_m_active: false,
            pending_strobes: Vec::new(),
            pending_observed: Vec::new(),
            deferred_asserts: Vec::new(),
            assert_ctl_on: false,
            assert_ctl: None,
            typedef_layouts_by_width: None,
            class_prop_dim_cache: std::cell::RefCell::new(HashMap::default()),
            sva_sites: Vec::new(),
            sva_lv: Vec::new(),
            sva_lv_types: Vec::new(),
            sva_proc_pending: Vec::new(),
            sva_default_disable: HashMap::default(),
            active_sva_site: None,
            sva_preponed: HashMap::default(),
            sva_preponed_slot: None,
            sva_owed: false,
            sva_post_nba_busy: false,
            clocking_snapshots: HashMap::default(),
            clocking_array_leaves: HashSet::default(),
            clocking_meta: HashMap::default(),
            clocking_output_pending: HashMap::default(),
            clocking_prev_clock: HashMap::default(),
            clocking_tick_cache: Vec::new(),
            buf_collapse_write_sets: None,
            packed_array_copy_values: HashMap::default(),
            clocking_edge: HashMap::default(),
            clocking_mirror_dirty: false,
            clocking_cycle_pending: HashMap::default(),
            clocking_sig_alias: HashMap::default(),
            clocking_out_skew: HashMap::default(),
            clocking_sig_out_skew: HashMap::default(),
            clocking_in_skew: HashMap::default(),
            clocking_sig_in_skew: HashMap::default(),
            clocking_last_edge: HashMap::default(),
            default_clocking_cb: None,
            clocking_preponed: HashMap::default(),
            clocking_preponed_slot: None,
            clocking_hist: HashMap::default(),
            clocking_hist_slot: None,
            clocking_inouts: HashMap::default(),
            clocking_vis: HashMap::default(),
            deferred_clocking_conts: Vec::new(),
            deferred_clocking_guarded: Vec::new(),
            pending_reactive: Vec::new(),
            program_pids: HashSet::default(),
            reactive_conts: Vec::new(),
            in_reactive_drain: false,
            class_assoc_defaults: HashMap::default(),
            mixed_packed_vars: HashSet::default(),
            static_struct_holders: HashMap::default(),
            static_struct_names: HashSet::default(),
            bc_packed_fields: None,
            active_union_tag: HashMap::default(),
            max_time,
            tick_s,
            time_format: None,
            // Zero-delay settle iteration cap. 100 was too low for real
            // RTL: a RIPPLE chain of continuous assigns needs one iteration
            // per stage, and the C910 SoC's PLIC builds a 128-stage prefix-OR
            // (`plic_kid_busif.v`: each stage ORs the previous slice), so the
            // design genuinely reports `max_iters=129`. Capping at 100 left
            // the chain at x, the interrupt priorities never resolved and
            // CoreMark died on the testbench's "no instructions retired"
            // watchdog. The cap only bounds a TRUE zero-delay loop, and the
            // deep case is rare — that same run averaged 1.007 iterations per
            // settle call — so a higher cap costs nothing in practice.
            // The real fix is levelized evaluation, which needs one pass.
            settle_limit: 1000,
            settle_limit_hits: 0,
            settle_limit_last_time: 0,
            proc_settle_defer: false,
            deferred_proc_entries: Vec::new(),
            t0_starts_remaining: 0,
            rand_scope_nd: HashMap::default(),
            rand_scope_nd_dyn: HashMap::default(),
            rand_scope_row_owners: HashMap::default(),
            p_top_level: false,
            null_call_reported: Default::default(),
            cascade_limit: std::env::var("XEZIM_CASCADE_LIMIT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(8),
            // sdf_delays: lazy — most designs don't have SDF annotations.
            // First annotated signal triggers a resize-to-num_signals via
            // the helper used by sdf and specify_delays paths.
            sdf_annotation: None,
            sdf_select: None,
            sdf_delays: Vec::new(),
            delayed_updates: Vec::new(),
            delayed_nba: Vec::new(),
            delayed_nba_queue: Vec::new(),
            packed_nba: Vec::new(),
            udp_runtime: Vec::new(),
            has_udp: false,
            timing_checks: Vec::new(),
            timing_watches: Vec::new(),
            has_timing_checks: false,
            module_paths: None,
            timing_pending: Vec::new(),
            timing_timers: Vec::new(),
            module,
            dpi_libraries: Vec::new(),
            dpi_bindings: HashMap::default(),
            dpi_unsupported: HashSet::default(),
            dpi_unresolved: HashSet::default(),
            nonstatic_scope_reported: HashSet::default(),
            dpi_bind_tried: HashSet::default(),
            uvm_dpi: Default::default(),
            heap: vec![None], // index 0 is null
            class_gc_heap_len: 0,
            class_gc_live: 0,
            class_gc_released: 0,
            class_gc_runs: 0,
            virtual_iface_bindings: HashMap::default(),
            static_vif_bindings: HashMap::default(),
            static_vif_names: HashSet::default(),
            vif_capable_statics: std::cell::OnceCell::new(),
            may_bind_static_vif: true,
            static_vif_array_names: HashSet::default(),
            recv_call_memo: Vec::new(),
            vif_tokens_stored: false,
            tb_cache: task_suspend::CycleMemo::default(),
            ts: task_suspend::TsState::default(),
            local_iface_aliases: Vec::new(),
            viface_var_aliases: HashMap::default(),
            last_vif_return: None,
            item_alias: None,
            locator_iter: "item".to_string(),
            dist_picked_once: HashSet::default(),
            rand_ranges: HashMap::default(),
            rand_elem_ranges: HashMap::default(),
            class_statics: StaticMap::default(),
            class_param_arrays: HashMap::default(),
            local_type_stack: Vec::new(),
            current_spec: None,
            spec_scope_stack: Vec::new(),
            gate_fall_delay_by_id: HashMap::default(),
            gate_off_delay_by_id: HashMap::default(),
            hint_string_pool: Vec::new(),
            cycle_mode: match std::env::var("XEZIM_CYCLE_MODE").as_deref() {
                Ok("cycle") => true,
                Ok("event") | Err(_) => false,
                Ok(other) => {
                    eprintln!(
                        "[xezim][warning] XEZIM_CYCLE_MODE={other:?} is not `event` or `cycle`; using `event`"
                    );
                    false
                }
            },
            clock_tree_by_root: HashMap::default(),
            is_clock_tree_entry: Vec::new(),
            gate_lane: Vec::new(),
            gate_ops: Vec::new(),
            gate_obs: Vec::new(),
            gate_queue: Vec::new(),
            gate_queued: Vec::new(),
            gate_lane_valid: false,
            iface_instance_paths: std::cell::OnceCell::new(),
            iface_by_token: std::cell::OnceCell::new(),
            iface_path_parts: std::cell::OnceCell::new(),
            quiet_bits: Vec::new(),
            quiet_valid: false,
            prof_quiet_stores: 0,
            prof_observed_stores: 0,
            #[cfg(feature = "jit")]
            ts_jit: None,
            #[cfg(feature = "jit")]
            ts_jit_fns: Vec::new(),
            #[cfg(feature = "jit")]
            ts_jit_on: std::env::var_os("XEZIM_TS_JIT").is_some(),
            #[cfg(feature = "jit")]
            ts_jit_evals: Vec::new(),
            #[cfg(feature = "jit")]
            ts_jit_hot: std::env::var("XEZIM_TS_JIT_HOT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(64),
            is_clock_tree_signal: Vec::new(),
            prof_clock_tree_evals: 0,
            spec_prop_is_dyn: std::cell::RefCell::new(HashMap::default()),
            spec_prop_width_cache: std::cell::RefCell::new(HashMap::default()),
            static_cell_width_cache: std::cell::RefCell::new(HashMap::default()),
            type_id_create_in_progress: HashSet::default(),
            initialized_spec_statics: std::collections::HashSet::default(),
            referenced_class_typedefs,
            factory_reg_in_progress: std::collections::HashSet::default(),
            ctor_class_stack: Vec::new(),
            reported_null_derefs: std::collections::HashSet::default(),
            pending_spec_inits: Vec::new(),
            spec_init_depth: 0,
            construct_init_depth: 0,
            pending_class_initializers: HashMap::default(),
            var_class_types: HashMap::default(),
            var_type_args: HashMap::default(),
            var_container_types: HashMap::default(),
            var_typedef_types: HashMap::default(),
            local_dyn: Vec::new(),
            formal_dyn: Vec::new(),
            next_dyn_id: 0,
            string_signals: HashSet::default(),
            queue_frame_saves: Vec::new(),
            mailboxes: HashMap::default(),
            mbx_msg_seq: 0,
            mbx_free_ids: Vec::new(),
            mbx_patterns: HashMap::default(),
            mailbox_bound: HashMap::default(),
            mailbox_put_waiters: HashMap::default(),
            rand_mode_disabled: HashMap::default(),
            constraint_mode_disabled: HashMap::default(),
            static_constraint_disabled: HashSet::default(),
            randomize_depth: 0,
            rand_fail_reports: HashMap::default(),
            rand_fail_hinted: false,
            randomize_budget_depth: 0,
            randomize_budget_remaining: 0,
            randomize_budget_deadline: None,
            randomize_budget_exhausted: false,
            rand_child_snapshots: Vec::new(),
            randc_used: HashMap::default(),
            randc_pending: HashMap::default(),
            randc_wide: HashMap::default(),
            randc_wide_pending: HashMap::default(),
            dist_decks: HashMap::default(),
            semaphores: HashMap::default(),
            cg_heap: vec![None],
            assertion_stats: HashMap::default(),
            assertion_labels: HashMap::default(),
            assert_wrap_label: None,
            code_cov: None,
            code_cov_hits: Vec::new(),
            code_cov_toggle_names,
            this_stack: vec![],
            local_stack: vec![],
            block_scope_saves: Vec::new(),
            hoisted_scope: (0, 0),
            trampolined_pids: HashSet::default(),
            local_gen_stack: Vec::new(),
            class_context_stack: vec![],
            method_local_base: vec![],
            class_method_cache: std::cell::RefCell::new(HashMap::default()),
            class_has_method_memo: std::cell::RefCell::new(HashMap::default()),
            is_static_method_memo: std::cell::RefCell::new(HashMap::default()),
            cg_this: None,
            join_waiters: Vec::new(),
            process_parents: HashMap::default(),
            process_contexts: HashMap::default(),
            process_scope_hint: HashMap::default(),
            process_m_scope: HashMap::default(),
            process_m_label: HashMap::default(),
            process_origin: HashMap::default(),
            trace_sched: std::env::var("XEZIM_TRACE_SCHED").ok().and_then(|v| {
                let v = v.trim();
                if v.is_empty() || v == "0" {
                    return None;
                }
                if v == "1" || v.eq_ignore_ascii_case("all") {
                    return Some((0, u64::MAX));
                }
                if let Some((a, b)) = v.split_once(':') {
                    return Some((
                        a.trim().parse().unwrap_or(0),
                        b.trim().parse().unwrap_or(u64::MAX),
                    ));
                }
                v.parse().ok().map(|t| (t, t))
            }),
            timescale_scope_override: None,
            pending_ret_collection: None,
            vm_ret_coll: None,
            vm_ret_vals: None,
            fn_ret_collection_stack: Vec::new(),
            fn_ret_assoc_stack: Vec::new(),
            fn_ret_ctx_stack: Vec::new(),
            current_scope: String::new(),
            m_block_scope: String::new(),
            m_block_active: false,
            edge_block_scope_id: Vec::new(),
            m_block_scope_id: u32::MAX,
            func_call_stack: Vec::new(),
            pkg_scope_stack: Vec::new(),
            pending_pkg_scope: None,
            pkg_ambiguous_names,
            pkg_enum_consts,
            contested_pkg_vars,
            m_scope_stack: Vec::new(),
            static_local_vars: HashMap::default(),
            static_local_names: HashSet::default(),
            hoisted_sub_static_names,
            hoisted_static_subroutines,
            static_local_first: [false; 256],
            static_local_keys: HashSet::default(),
            static_local_syncs: Vec::new(),
            in_const_param_eval: false,
            return_value: None,
            string_signals_removed: Vec::new(),
            task_string_formals: Vec::new(),
            decl_shadow_log: Vec::new(),
            nested_run_epoch: 0,
            cond_read_union: HashSet::default(),
            cond_read_union_dirty: false,
            colliding_static_colls: HashSet::default(),
            colliding_static_colls_ready: false,
            rng: SvRng::from_seed(SvRng::DEFAULT_SEED),
            proc_rng: HashMap::default(),
            obj_rng: HashMap::default(),
            obj_rng_stack: Vec::new(),
            rand_receiver: None,
            rand_receiver_base: 0,
            randomize_subset: None,
            shadowed_prop_names: HashSet::default(),
            shadowed_prop_mask: 0,
            signed_class_colls: HashSet::default(),
            dep_base: HashMap::default(),
            dep_default: HashMap::default(),
            dep_names: Vec::new(),
            spec_entry_cache: std::cell::RefCell::new(HashMap::default()),
            spec_clone_origin: HashMap::default(),
            dep_typedefs: HashSet::default(),
            inst_templates: std::cell::RefCell::new(HashMap::default()),
            inst_key_scratch: String::new(),
            settling: false,
            in_edge_block: false,
            in_edge_cont: false,
            edge_pass_depth: 0,
            bitsel_edge_sens: HashMap::default(),
            bitsel_sid_bits: Vec::new(),
            edge_triggered_bitmap: Vec::new(),
            edge_triggered_list: Vec::new(),
            edge_fired_snap: Vec::new(),
            edge_fired_wide: Vec::new(),
            edge_prefilter_seen: Vec::new(),
            edge_prefilter_generation: 0,
            edge_parallel_work: Vec::new(),
            edge_sequential_work: Vec::new(),
            nba_queue: Vec::new(),
            pending_nba_triggers: Vec::new(),
            pending_nba_instance_triggers: Vec::new(),
            active_task_pids: HashMap::default(),
            event_aliases: HashMap::default(),
            nba_fast: Vec::new(),
            // Named signals stay on the dense hot path. Unnamed large-array
            // elements use NbaFastIndex's sparse tail.
            nba_fast_index: NbaFastIndex::with_dense_prefix(named_count),
            edge_blocks: Arc::new(Vec::new()),
            compiled_edge_blocks: Vec::new(),
            jit_fns: Vec::new(),
            #[cfg(feature = "jit")]
            comb_jit_fns: Vec::new(),
            #[cfg(feature = "jit")]
            comb_tpl_fns: Vec::new(),
            #[cfg(feature = "jit")]
            edge_tpl_fns: Vec::new(),
            #[cfg(feature = "jit")]
            edge_tpl_map_blob: Vec::new(),
            #[cfg(feature = "jit")]
            comb_tpl_map_blob: Vec::new(),
            #[cfg(feature = "jit")]
            comb_jit_strikes: Vec::new(),
            #[cfg(feature = "jit")]
            prof_comb_jit: [0; 2],
            jit_module: None,
            edge_block_parallel: Vec::new(),
            edge_block_partition: Vec::new(),
            edge_block_partition_count: 0,
            edge_block_exec_counts: Vec::new(),
            edge_block_exec_ns: Vec::new(),
            signal_toggle_count: Vec::new(),
            nba_count_per_block: Vec::new(),
            sensitivity_trigger_count: Vec::new(),
            cross_block_wakeup_count: Vec::new(),
            local_quiescence_iterations: Vec::new(),
            edge_block_stats_enabled: std::env::var("XEZIM_PROFILE_REPORT").ok().as_deref()
                == Some("1")
                || std::env::var("XEZIM_EDGE_BLOCK_STATS").ok().as_deref() == Some("1"),
            edge_block_scope: Vec::new(),
            edge_block_needs_hint: Vec::new(),
            edge_block_light: Vec::new(),
            edge_block_insn_len: Vec::new(),
            vm_regs: Vec::new(),
            foreach_arena: Vec::new(),
            clock_generators: Vec::new(),
            clock_gen_src: Vec::new(),
            t0_timing_free: HashSet::default(),
            t0_pid_ord: HashMap::default(),
            fast_delay_always: HashMap::default(),
            proc_fsm: HashMap::default(),
            fsm_start_pc: 0,
            fsm_suspend: None,
            fsm_precision_diff: 0,
            event_queue: TimingWheel::new(),
            next_pid: 0,
            current_pid: 0,
            frame_gen: 0,
            dollar_bound: Vec::new(),
            break_flag: false,
            continue_flag: false,
            return_flag: false,
            rs_return_flag: false,
            parked_from_exec: false,
            exec_park_cont: None,
            dpi_export_depth: 0,
            dpi_task_names: HashSet::default(),
            dpi_tasks: HashMap::default(),
            dpi_export_cstrings: HashMap::default(),
            dpi_export_seq: 0,
            slot_batches: Vec::new(),
            dpi_cur_task: None,
            dpi_unwinding: false,
            next_dpi_task: 1,
            sync_wakes: HashSet::default(),
            ended_waiting_in_c: false,
            next_sync_wake: 0,
            mcd_names: HashMap::default(),
            foreach_replay_parks: HashMap::default(),
            foreach_replay_limit: std::env::var("XEZIM_FOREACH_REPLAY_LIMIT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(64),
            disable_target: None,
            disable_check_pending: false,
            named_block_frames: HashMap::default(),
            fork_spawn_cont: HashMap::default(),
            fork_spawn_prune_at: 1024,
            pending_block_disable: HashMap::default(),
            disable_labels: HashMap::default(),
            fork_block_children: HashMap::default(),
            warned_assoc_index: HashSet::default(),
            warned_delay_spikes: HashSet::default(),
            t0_delay_deferred: HashSet::default(),
            suspended_loop_frames: HashMap::default(),
            process_loop_bytecode: HashMap::default(),
            process_cond_bytecode: HashMap::default(),
            killed_pids: HashSet::default(),
            finished_pids: HashSet::default(),
            suspended_pids: HashSet::default(),
            suspended_proc_info: HashMap::default(),
            await_waiters: Vec::new(),
            auto_loop_vars: Vec::new(),
            ref_binding_stack: Vec::new(),
            ref_alias_stack: Vec::new(),
            ref_identity_stack: Vec::new(),
            pending_formal_dyn: Vec::new(),
            ref_redirect_hot: false,
            cg_type_options: HashMap::default(),
            method_receiver_cache: HashMap::default(),
            hint_key_scratch: std::cell::RefCell::new(String::new()),
            vif_key_scratch: std::cell::RefCell::new(String::new()),
            spec_frag_fold_cache: RefCell::new(HashMap::default()),
            param_leaf_names: std::cell::OnceCell::new(),
            unpacked_typedef_reps: std::cell::OnceCell::new(),
            unpacked_typedef_names: std::cell::OnceCell::new(),
            marker_key_scratch: String::new(),
            unpacked_struct_prop_names: std::cell::OnceCell::new(),
            struct_root_names: HashSet::default(),
            struct_root_names_built: false,
            multidim_members: None,
            struct_capable_prop_names: std::cell::OnceCell::new(),
            struct_coll_member_prop_names: std::cell::OnceCell::new(),
            indexed_coll_roots: std::cell::OnceCell::new(),
            indexed_coll_mask: std::cell::Cell::new(0),
            struct_coll_member_len_mask: std::cell::Cell::new(0),
            assoc_elem_colls: HashSet::default(),
            local_coll_elems: HashMap::default(),
            pending_local_shape: None,
            local_coll_rows_live: false,
            local_coll_shapes: HashMap::default(),
            dynamic_assoc_dims: HashMap::default(),
            pending_struct_coll_wb: Vec::new(),
            pending_ret_struct: None,
            class_member_names_cell: std::cell::OnceCell::new(),
            vif_local_names: HashSet::default(),
            method_def_cache: std::cell::RefCell::new(HashMap::default()),
            string_keyed_member_cache: std::cell::RefCell::new(HashMap::default()),
            nonvirtual_target_cache: std::cell::RefCell::new(HashMap::default()),
            has_method_cache: std::cell::RefCell::new(HashMap::default()),
            covergroup_leaf_names: std::cell::OnceCell::new(),
            class_string_props: std::cell::RefCell::new(HashMap::default()),
            class_enclosing_cache: std::cell::RefCell::new(HashMap::default()),
            plain_class_types: std::cell::RefCell::new(None),
            coll_member_names: std::cell::RefCell::new(None),
            queue_key_scratch: String::new(),
            prop_owner_index: std::cell::RefCell::new(HashMap::default()),
            prop_width_memo: std::cell::RefCell::new(HashMap::default()),
            formal_parts_pool: Vec::new(),
            queue_save_pool: Vec::new(),
            local_dyn_pool: Vec::new(),
            queue_name_pool: Vec::new(),
            spec_buf_pool: Vec::new(),
            method_plans: std::cell::RefCell::new(HashMap::default()),
            meta_frames: Vec::new(),
            meta_pending: 0,
            type_tables_gen: 0,
            local_type_pool: Vec::new(),
            sync_name_pool: Vec::new(),
            dyn_key_scratch: String::new(),
            method_receiver_hint_ids: HashMap::default(),
            compiled_class_method_ids: HashMap::default(),
            compiled_method_block_cache: HashMap::default(),
            compiled_method_admission: None,
            persisted_method_context_hash: None,
            compiled_method_plans: HashMap::default(),
            compiled_method_skip: HashSet::default(),
            compiled_method_call_counts: HashMap::default(),
            compiled_method_boost_qual: HashMap::default(),
            compiled_method_heat_stack: Vec::new(),
            compiled_method_loop_heat: HashMap::default(),
            compiled_fast_calls: HashMap::default(),
            compiled_fast_static_calls: HashMap::default(),
            compiled_spec_intern: HashMap::default(),
            typeref_class_memo: std::cell::RefCell::new(HashMap::default()),
            task_cleanup: Vec::new(),
            condition_waiters: Vec::new(),
            ready_condition_waiters: Vec::new(),
            cond_waiter_reads: HashMap::default(),
            cond_waiter_conditions: HashMap::default(),
            cond_waiter_gate: HashMap::default(),
            cond_repass_recheck: HashMap::default(),
            inactive_queue: Vec::new(),
            nba_region_waiters: Vec::new(),
            cond_progress: 0,
            mailbox_get_waiters: HashMap::default(),
            fork_baselines: HashMap::default(),
            fork_collection_captures: HashMap::default(),
            fork_capture_refs: HashMap::default(),
            fork_gen_pids: HashMap::default(),
            fork_pid_gens: HashMap::default(),
            fork_local_scopes: HashMap::default(),
            deferred_collection_cleanup: HashSet::default(),
            fork_signal_captures: HashMap::default(),
            semaphore_get_waiters: HashMap::default(),
            intra_saved: HashMap::default(),
            intra_saved_next: 0,
            vc_saved: HashMap::default(),
            event_triggered_time: EventStampStore::default(),
            event_waiters: Vec::new(),
            instance_event_waiters: Vec::new(),
            late_resume_epoch: 0,
            fe_trusted_types: HashSet::default(),
            pid_src_file: HashMap::default(),
            cg_event_waiters: Vec::new(),
            cg_named_event_waiters: HashMap::default(),
            cg_instance_event_waiters: HashMap::default(),
            triggered_conts_buf: Vec::new(),
            sig_wake_rank: HashMap::default(),
            wake_rank_built: false,
            name_wake_rank: HashMap::default(),
            wake_rank_buf: Vec::new(),
            cg_tree_roots_buf: Vec::new(),
            deferred_comb_pool: Vec::new(),
            cur_fsm_writes: None,
            fsm_clobber_cache: Vec::new(),
            wave: wave_enabled(),
            vcd_file: None,
            vcd_writer: None,
            vcd_trace: Vec::new(),
            dump_dirty: Vec::new(),
            dump_dirty_mark: Vec::new(),
            dump_dirty_epoch: 1,
            vcd_id_to_trace: Vec::new(),
            vcd_event_indices: Vec::new(),
            dump_dirty_active: false,
            dump_full: false,
            vcd_enabled: false,
            vcd_last_time: u64::MAX,
            vcd_prev_signals: Vec::new(),
            vcd_filter_scopes: Vec::new(),
            vcd_dump_depth: 0,
            vcd_var_kinds: Vec::new(),
            vcd_event_last: Vec::new(),
            vcd_limit: None,
            vcd_bytes: 0,
            vcd_limit_hit: false,
            bsp_shadow_checks: 0,
            stdout_sink: None,
            xtrace_file: None,
            xtrace_scopes: Vec::new(),
            xtrace_writer: None,
            xtrace_trace: Vec::new(),
            xtrace_id_to_trace: Vec::new(),
            xtrace_prev_signals: Vec::new(),
            xtrace_real: Vec::new(),
            xtrace_string: Vec::new(),
            xtrace_snapshot_pending: false,
            xtrace_events: Vec::new(),
            xtrace_event_last: Vec::new(),
            xtrace_last_time: 0,
            xtrace_dirty_steps: 0,
            xtrace_from_ns: 0,
            xtrace_to_ns: u64::MAX,
            xtrace_from_t: 0,
            xtrace_to_t: u64::MAX,
            fst_file: None,
            fst_scopes: Vec::new(),
            fst_writer: None,
            cast_widths,
            fn_decl_cache: HashMap::default(),
            free_fn_table: None,
            class_handle_member_types_cache: None,
            dpi_import_fds: None,
            free_fn_class_names: HashMap::default(),
            free_call_pkgs: HashMap::default(),
            task_decl_cache: HashMap::default(),

            fn_pure_cache: HashMap::default(),
            elem_dotted_bases: RefCell::new(None),
            fst_trace: Vec::new(),
            fst_id_to_trace: Vec::new(),
            fst_prev_signals: Vec::new(),
            fst_events: Vec::new(),
            fst_event_last: Vec::new(),
            comb_entries: Vec::new(),
            prepared_comb_cache_path: None,
            comb_level: Vec::new(),
            comb_has_nba: Vec::new(),
            comb_par_safe: Vec::new(),
            bsp_buckets: Vec::new(),
            bsp_next_seed: Vec::new(),
            bsp_width_buckets: [0; 6],
            bsp_widths_on: std::env::var_os("XEZIM_BSP_WIDTHS").is_some(),
            bsp_settle: std::env::var_os("XEZIM_BSP_SETTLE").is_some(),
            bsp_par: std::env::var_os("XEZIM_BSP_PAR").is_some(),
            bsp_par_threshold: std::env::var("XEZIM_BSP_PAR_THRESHOLD")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(256),
            bsp_shadow: std::env::var_os("XEZIM_BSP_SHADOW").is_some(),
            comb_unresolved_idx: Vec::new(),
            coact_pred_off: Vec::new(),
            coact_pred_ent: Vec::new(),
            coact_edge_count: Vec::new(),
            coact_epoch: Vec::new(),
            coact_enabled: false,
            comb_time0_idx: Vec::new(),
            comb_time0_fired: false,
            comb_time0_deferred: Vec::new(),
            comb_time0_deferred_done: false,
            comb_dep_offsets: Vec::new(),
            ts_comb: Vec::new(),
            comb_plan: Vec::new(),
            ts_arena: Vec::new(),
            ts_hdr: Vec::new(),
            ts_edge: Vec::new(),
            ts_regs: Vec::new(),
            ts_xregs: Vec::new(),
            ts_hdr_tsx: Vec::new(),
            tsx_on: std::env::var("XEZIM_TS_X").ok().as_deref() != Some("0"),
            prof_tsx_evals: 0,
            ts_wregs: Vec::new(),
            ts_wregs8: Vec::new(),
            prof_ts_evals: 0,
            prof_ts_bail_warnx: 0,
            prof_ts_bail_forced: 0,
            prof_ts_bail_xread: 0,
            prof_ts_bail_abort: 0,
            ts_exec_aborted: false,
            ts_xread_bail: false,
            ts_save_list: Vec::new(),
            ts_cur_eidx: u32::MAX,
            settle_entries_view: (std::ptr::null(), 0),
            #[cfg(feature = "opcode-census")]
            ts_xbail_by_entry: HashMap::default(),
            ts_edge_abortn: Vec::new(),
            ts_sleep: std::collections::BinaryHeap::new(),
            ts_sleep_recs: Vec::new(),
            ts_sleep_free: Vec::new(),
            ts_comb_xb: Vec::new(),
            ts_edge_xb: Vec::new(),
            ts_asleep_comb: Vec::new(),
            ts_asleep_edge: Vec::new(),
            ts_last_xbail: false,
            ts_comb_abortn: Vec::new(),
            comb_plan_abortn: Vec::new(),
            profile_report: std::env::var("XEZIM_PROFILE_REPORT").ok().as_deref() == Some("1"),
            proc_fsm_mode: match std::env::var("XEZIM_PROC_FSM").as_deref() {
                Ok("1") => 1,
                Ok("0") => 0,
                _ => 2,
            },
            comb_dep_entries: Vec::new(),
            comb_dep_edges: Vec::new(),
            bit_sens_pending: Vec::new(),
            bit_sens_on: !matches!(std::env::var("XEZIM_BIT_SENS").as_deref(), Ok("0")),
            ctl_gates: Vec::new(),
            ctl_sig: Vec::new(),
            ctl_dsts: Vec::new(),
            ctl_rearm_pending: Vec::new(),
            ctl_on: !matches!(std::env::var("XEZIM_CTL_MASK").as_deref(), Ok("0"))
                && std::env::var_os("XEZIM_BSP_SETTLE").is_none()
                && std::env::var_os("XEZIM_BSP_SHADOW").is_none(),
            ts_rec0: (0, 0),
            ts_rec_n: 0,
            ts_recs: Vec::new(),
            dirty_signals: vec![false; num_signals],
            dirty_list: Vec::new(),
            dirty_any: false,
            ts_direct_writes: false,
            table_modified: false,
            toggled_clock_positions: Vec::new(),
            edge_scan_scanned: 0,
            edge_scan_changed: 0,
            edge_scan_stats: std::env::var_os("XEZIM_EDGE_SCAN_STATS").is_some(),
            soa_shadow: std::env::var_os("XEZIM_ARRAY_SOA_SHADOW").is_some(),
            soa_read_ok: Vec::new(),
            census_enabled: std::env::var_os("XEZIM_OPCODE_CENSUS").is_some(),
            census_counts: Vec::new(),
            census_pairs: Vec::new(),
            #[cfg(feature = "opcode-census")]
            ts_census: std::collections::HashMap::new(),
            sig_to_edge_pos: Vec::new(),
            edge_group_of: Vec::new(),
            edge_group_memo: Vec::new(),
            edge_group_generation: 0,
            edge_group_count: 0,
            clktree_probe: std::env::var_os("XEZIM_CLKTREE_PROBE").is_some(),
            clktree_probe_hits: 0,
            clktree_probe_mismatch: 0,
            clktree_probe_sids: Vec::new(),
            changed_edge_pos: Vec::new(),
            edge_pos_scratch: Vec::new(),
            edge_pos_seen: Vec::new(),
            edge_exec_wrote: Vec::new(),
            edge_exec_seen: Vec::new(),
            proc_depth: 0,
            deferred_comb: Vec::new(),
            in_edge_rescan: false,
            edge_rescan_block_hits: HashMap::default(),
            stall_edge_block_hits: HashMap::default(),
            // ON by default since the shadow validator (XEZIM_DIRTY_EDGE_SHADOW,
            // which drives the sim by full scan and asserts every firing edge
            // was captured) reported zero misses across the C906 CoreMark run,
            // ibex simple_system, the C910 SoC, three UVM testbenches and the
            // whole regression suite. `XEZIM_DIRTY_EDGE=0` restores the full
            // per-tick scan.
            // Documented as (0/1): `is_some()` made `XEZIM_DIRTY_EDGE=0`
            // ENABLE the mode, the opposite of what the value says.
            dirty_edge: std::env::var("XEZIM_DIRTY_EDGE")
                .map(|v| v != "0" && !v.is_empty())
                .unwrap_or(true),
            dirty_edge_shadow: std::env::var("XEZIM_DIRTY_EDGE_SHADOW")
                .map(|v| v != "0" && !v.is_empty())
                .unwrap_or(false),
            prof_clocks_only_detect: 0,
            is_edge_signal_non_clock: Vec::new(),
            nba_touched_edge_non_clock: false,
            prof_nba_elided: 0,
            event_measure: false,
            event_skip: false,
            event_after: 0,
            edge_block_data_reads: Vec::new(),
            edge_block_gateable: Vec::new(),
            edge_block_arm_only: Vec::new(),
            edge_block_wide_read: Vec::new(),
            arm_census: if std::env::var_os("XEZIM_ARM_CENSUS").is_some() {
                Some(Vec::new())
            } else {
                None
            },
            sig_last_change: Vec::new(),
            flop_last_fire: Vec::new(),
            edge_block_reads_flat: Vec::new(),
            edge_block_reads_meta: Vec::new(),
            edge_block_snap_flat: Vec::new(),
            edge_block_off: Vec::new(),
            edge_block_snap_valid: Vec::new(),
            edge_block_change_streak: Vec::new(),
            edge_block_skip_streak: Vec::new(),
            event_heal: 16,
            event_healed: 0,
            edge_block_epoch_probe_left: Vec::new(),
            armed_edge: std::env::var("XEZIM_ARMED_EDGE").ok().as_deref() != Some("0")
                || std::env::var_os("XEZIM_ARMED_EDGE_SHADOW").is_some(),
            armed_edge_shadow: std::env::var_os("XEZIM_ARMED_EDGE_SHADOW").is_some(),
            edge_fire_trace: std::env::var("XEZIM_EDGE_FIRE_TRACE")
                .ok()
                .filter(|s| !s.is_empty()),
            edge_fire_match: None,
            edge_fire_watch: Vec::new(),
            edge_fire_watch_init: false,
            edge_dispatch_phase: "active",
            value_trace_ids: None,
            value_trace_pending: std::cell::RefCell::new(Vec::new()),
            value_trace_emitted: 0,
            value_trace_limit: std::env::var("XEZIM_VALUE_TRACE_LIMIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(20_000),
            armed_input_bitmap: Vec::new(),
            armed_input_ranges: Vec::new(),
            armed_input_blocks: Vec::new(),
            edge_block_armed: Vec::new(),
            edge_block_retired: Vec::new(),
            armed_fast_skips: 0,
            armed_shadow_checks: 0,
            event_phase: 0,
            event_would_skip: 0,
            event_gateable_total: 0,
            event_epoch_fast_exec: 0,
            event_snapshot_checks: 0,
            settle_calls: 0,
            prof_settle_writes: 0,
            prof_settle_dep_edges: 0,
            packed_blocking_fill_collapses: 0,
            settle_triggered: Vec::new(),
            settle_dirty_ids: Vec::new(),
            settle_prev_values: Vec::new(),
            settle_triggered_list: Vec::new(),
            loop_iters: 0,
            prof_edge_detect: 0,
            prof_edge_exec: 0,
            prof_par_dispatch_partition: 0,
            prof_par_dispatch_legacy: 0,
            prof_edge_waiters: 0,
            prof_edge_cg: 0,
            prof_waiter_iters: 0,
            prof_edges_fired: 0,
            prof_insns_executed: 0,
            par_cal_phase: 0,
            par_cal_ticks: 0,
            par_cal_seq_ns: 0,
            par_cal_seq_insns: 0,
            par_cal_par_ns: 0,
            par_cal_par_insns: 0,
            par_cal_use_parallel: false,
            prof_par_ticks: 0,
            prof_par_blocks: 0,
            forever_depth: 0,
            forever_cont_cache: HashMap::default(),
            settle_inject_bits: Vec::new(),
            settle_inject_sum: Vec::new(),
            vif_return_pending: false,
            lhs_leaf_width_cache: HashMap::default(),
            blocking_cont_cache: HashMap::default(),
            class_task_frame_cache: HashMap::default(),
            forever_sens_cache: HashMap::default(),
            exec_cache_pids: HashSet::default(),
            constraint_cmp_unsigned: false,
            stall_iters: 0,
            zero_delay_defer_pending: false,
            stall_time: 0,
            stall_pid_hits: HashMap::default(),
            stall_limit: std::env::var("XEZIM_STALL_LIMIT")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(100_000),
            prof_fallback_insns: 0,
            profile_timing: std::env::var("XEZIM_PROFILE_TIMING").ok().as_deref() == Some("1")
                || std::env::var("XEZIM_PROFILE_REPORT").ok().as_deref() == Some("1"),
            trace_comb_paths: std::env::var("XEZIM_COMB_PATHS").ok().as_deref() == Some("1"),
            comb_path_counts: Vec::new(),
            comb_path_last: None,
            edge_path_counts: Vec::new(),
            comb_eval_counts: Vec::new(),
            comb_eval_last_time: Vec::new(),
            prof_fallback_by_reason: HashMap::default(),
            prof_settle_dc_ns: 0,
            prof_settle_ca_ns: 0,
            prof_settle_ab_ns: 0,
            prof_settle_dc_count: 0,
            prof_settle_repass_evals: 0,
            prof_entry_hist: [0; 12],
            prof_entry_counts: Vec::new(),
            prof_sampler: None,
            prof_cur: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            cone_chain_member: Vec::new(),
            cone_chains: Vec::new(),
            prof_settle_ca_count: 0,
            prof_settle_ab_count: 0,
            entry_evals: 0,
            settle_iters: 0,
            max_settle_iters: 0,
            activity_counts: Vec::new(),
            // signal_toggle_counts: lazy-allocated when activity_mon is
            // turned on by the caller (after Simulator::new returns).
            // Saves 8 × num_signals bytes (288 MB on c910). Access is
            // already gated by self.activity_mon.
            signal_toggle_counts: Vec::new(),
            activity_mon: false,
            trace_always: std::env::var("XEZIM_TRACE_ALWAYS")
                .ok()
                .map(|v| if v == "1" { String::new() } else { v }),
            plusargs: Vec::new(),
            run_seed: None,

            // Default to a single-element argv so vpi_get_vlog_info
            // always returns at least argc=1 / argv[0]="xezim" even
            // when callers forget to call set_args. Real callers
            // (lib.rs / main.rs) overwrite this with the actual CLI.
            vpi_arg_cstrings: Vec::new(),
            vpi_argv: Vec::new(),
            dpi_scopes: HashMap::default(),
            prof_psettle_calls: 0,
            prof_psettle_deferred: 0,
            dpi_value_change_cbs: HashMap::default(),
            chained_coll_member_names: std::cell::OnceCell::new(),
            elem_inside_hint: Vec::new(),
            rand_tight_mode: false,
            vpi_cb: Box::default(),
            vpi_cb_mask: 0,
            vpi_slot_started: u64::MAX,
            stdout_unsynced: false,
            vpi_self_ptr: std::ptr::null_mut(),
            file_handles: HashMap::default(),
            checkpoint_ops: Vec::new(),
            checkpoint_save_at: Vec::new(),
            checkpoint_restored: false,
            checkpoint_muted: false,
            checkpoint_audit: None,
            file_writes_pending: false,
            file_eof: HashSet::default(),
            ungetc_buf: HashMap::default(),
            queues: HashMap::default(),
            static_task_init: HashSet::default(),
            static_block_decls: HashMap::default(),
            loop_frame_gens: HashSet::default(),
            current_static_task: None,
            current_auto_task: None,
            name_resolve_hint: RefCell::new(None),
            activation_scope: RefCell::new(None),
            modclass_depth: std::cell::Cell::new(0),
            modclass_colls: RefCell::new(HashMap::default()),
            modclass_insts: RefCell::new(HashMap::default()),
            multi_modclass: std::cell::OnceCell::new(),
            has_modclass: std::cell::Cell::new(None),
            assoc_static_keys_cache: RefCell::new(HashMap::default()),
            assoc_keys_cache: RefCell::new(HashMap::default()),
            sampled_watches: Vec::new(),
            sampled_defaults: HashMap::default(),
            time0_settled: false,
            sampled_watch_site: HashMap::default(),
            sampled_site_clock: HashMap::default(),
            proc_sampled_blocks: Vec::new(),
            sampled_registering: false,
            pending_t0_triggers: Vec::new(),
            in_pre_process_settle: false,
        };
        sim.has_coll_rows = !sim.module.coll_shapes.is_empty()
            || sim
                .module
                .classes
                .values()
                .any(|c| !c.prop_coll_shapes.is_empty());
        // Static event names get stable IDs before simulation; class, local
        // and selected event identities can still be interned on first use.
        // Interning alone must not make an event appear already triggered.
        let mut event_names: Vec<_> = sim.module.events.iter().collect();
        event_names.sort_unstable();
        for name in event_names {
            sim.event_triggered_time.intern(name);
        }
        let total_ms = phase_total.elapsed().as_secs_f64() * 1000.0;
        if total_ms > 100.0 {
            chatter!(
                "[PHASE] simulator construction: {:.1}ms (materialize {:.1}, names {:.1}, static {:.1}, arrays_1d {:.1}, arrays_other {:.1}, prev {:.1}, drop {:.1}; signals {}, named {})",
                total_ms,
                materialize_ms,
                names_ms,
                static_ms,
                arrays_1d_ms,
                arrays_other_ms,
                prev_ms,
                drop_ms,
                num_signals,
                named_count
            );
        }
        // Per-signal memory accounting (XEZIM_MEM_CENSUS=1). The array cells
        // are already handled by the packed arena; this measures what is left,
        // which the reverse-engineering notes rank as the single biggest gap
        // (memory 59x vs speed 2.6x).
        //
        // READ THE NUMBERS AS CAPACITY, NOT RESIDENT MEMORY. The signal-side
        // vectors are reserved for the FULL pre-arena signal count (35,114,425
        // on the C906 SoC) but only ~1.56 M entries are ever pushed, so the
        // tail is untouched pages that never enter RSS: this census totals
        // 1.5 GB where the process actually resides in ~0.98 GB. Reclaiming
        // the excess with shrink_to_fit was MEASURED AND REVERTED — it copies
        // the live prefix, transiently doubling the touched set, and took peak
        // RSS 981 -> 1021 MB with wall 15.1 -> 16.6 s. Right-sizing the
        // reservation up front would avoid the address space but buys little
        // resident memory.
        if std::env::var("XEZIM_MEM_CENSUS").is_ok() {
            let n = sim.signal_table.len();
            let mb = |b: usize| b as f64 / (1024.0 * 1024.0);
            let name_bytes: usize = sim.id_to_name.iter().map(|(_, s)| s.len()).sum();
            let leaf_vec_bytes: usize = sim
                .leaf_name_to_ids
                .values()
                .map(|v| v.capacity() * std::mem::size_of::<usize>() + 24)
                .sum();
            let scope_vec_bytes: usize = sim
                .scope_parents_by_leaf
                .values()
                .map(|v| v.capacity() * std::mem::size_of::<Arc<str>>() + 24)
                .sum();
            let hm_entry = |entries: usize, k: usize, v: usize| -> usize {
                // hashbrown: (K,V) plus one control byte, at ~87.5% max load.
                ((k + v + 1) as f64 * entries as f64 / 0.875) as usize
            };
            let rows: Vec<(&str, usize)> = vec![
                (
                    "signal_table (Value)",
                    sim.signal_table.capacity() * std::mem::size_of::<Value>(),
                ),
                (
                    "id_to_name ptrs",
                    sim.id_to_name.ptr_capacity() * std::mem::size_of::<Arc<str>>(),
                ),
                ("id_to_name STRING BYTES", name_bytes),
                (
                    "signal_name_to_id map",
                    hm_entry(sim.signal_name_to_id.explicit_len(), 16, 8),
                ),
                (
                    "leaf_name_to_ids map+vecs",
                    hm_entry(sim.leaf_name_to_ids.len(), 16, 24) + leaf_vec_bytes,
                ),
                (
                    "scope_parents_by_leaf",
                    hm_entry(sim.scope_parents_by_leaf.len(), 16, 24) + scope_vec_bytes,
                ),
                ("signal_inline_bits", sim.signal_inline_bits.capacity() * 16),
                (
                    "prev_val + prev_xz",
                    (sim.prev_val.capacity() + sim.prev_xz.capacity()) * 8,
                ),
                (
                    "active_force_signal_epochs",
                    sim.active_force_signal_epochs.capacity() * 8,
                ),
                ("signal_widths", sim.signal_widths.capacity() * 4),
                (
                    "signal flag vecs (5x bool)",
                    sim.signal_signed.capacity()
                        + sim.signal_real.capacity()
                        + sim.signal_two_state.capacity()
                        + sim.signal_gate_driven.capacity()
                        + sim.signal_is_string.capacity(),
                ),
                ("dirty_signals", sim.dirty_signals.capacity()),
            ];
            let total: usize = rows.iter().map(|(_, b)| *b).sum();
            eprintln!("[MEM-CENSUS] === per-signal structures, {} signals ===", n);
            let mut sorted = rows.clone();
            sorted.sort_by_key(|(_, b)| std::cmp::Reverse(*b));
            for (name, bytes) in sorted {
                eprintln!(
                    "[MEM-CENSUS] {:>9.1} MB {:>5.1}%  {:>6.0} B/sig  {}",
                    mb(bytes),
                    100.0 * bytes as f64 / total.max(1) as f64,
                    bytes as f64 / n.max(1) as f64,
                    name
                );
            }
            eprintln!(
                "[MEM-CENSUS] len vs CAPACITY: signal_table {}/{} | widths {}/{} | signed {}/{} | inline_bits {}/{} | prev_val {}/{} | dirty {}/{}",
                sim.signal_table.len(),
                sim.signal_table.capacity(),
                sim.signal_widths.len(),
                sim.signal_widths.capacity(),
                sim.signal_signed.len(),
                sim.signal_signed.capacity(),
                sim.signal_inline_bits.len(),
                sim.signal_inline_bits.capacity(),
                sim.prev_val.len(),
                sim.prev_val.capacity(),
                sim.dirty_signals.len(),
                sim.dirty_signals.capacity(),
            );
            eprintln!(
                "[MEM-CENSUS] TOTAL accounted {:.1} MB ({:.0} B/signal); mean name length {:.1} chars",
                mb(total),
                total as f64 / n.max(1) as f64,
                name_bytes as f64 / n.max(1) as f64
            );
            // The RETAINED elaborated design, field by field. Serialized size
            // is a stand-in for heap size (same strings, same trees, minus
            // allocator and container slack), which is what ranks the fields.
            let m = &sim.module;
            let ser = |v: &dyn erased_size::Ser| v.size();
            let mut fields: Vec<(&str, usize)> = vec![
                ("always_blocks", ser(&m.always_blocks)),
                ("initial_blocks", ser(&m.initial_blocks)),
                ("continuous_assigns", ser(&m.continuous_assigns)),
                ("functions", ser(&m.functions)),
                ("tasks", ser(&m.tasks)),
                ("classes", ser(&m.classes)),
                ("var_decl_types", ser(&m.var_decl_types)),
                ("decl_sites", ser(&m.decl_sites)),
                ("instances", ser(&m.instances)),
                ("parameters", ser(&m.parameters)),
                ("arrays", ser(&m.arrays)),
                ("typedef_types", ser(&m.typedef_types)),
                ("source_texts", ser(&m.source_texts)),
                ("nets", ser(&m.nets)),
                ("two_state_signals", ser(&m.two_state_signals)),
                ("string_signals", ser(&m.string_signals)),
                ("packed_struct_fields", ser(&m.packed_struct_fields)),
                ("port_aliases", ser(&m.port_aliases)),
                ("resolved_net_kinds", ser(&m.resolved_net_kinds)),
                ("typed_decls", ser(&m.typed_decls)),
                ("udp_instances", ser(&m.udp_instances)),
                ("timing_checks", ser(&m.timing_checks)),
                ("gate_driven_nets", ser(&m.gate_driven_nets)),
                ("unpacked_decl_dims", ser(&m.unpacked_decl_dims)),
                ("packed_full_dims", ser(&m.packed_full_dims)),
                (
                    "packed_signal_elem_widths",
                    ser(&m.packed_signal_elem_widths),
                ),
                ("static_init_blocks", ser(&m.static_init_blocks)),
                ("final_blocks", ser(&m.final_blocks)),
                ("specify_delays", ser(&m.specify_delays)),
                ("module_paths", ser(&m.module_paths)),
            ];
            fields.sort_by_key(|(_, b)| std::cmp::Reverse(*b));
            let tot: usize = fields.iter().map(|(_, b)| *b).sum();
            eprintln!(
                "[MEM-CENSUS] === retained elaborated module (serialized bytes), {:.1} MB ===",
                mb(tot)
            );
            for (name, bytes) in fields.iter().take(20) {
                eprintln!("[MEM-CENSUS] {:>9.1} MB  {}", mb(*bytes), name);
            }
        }
        // Collapse identity port nets before anything resolves names to ids:
        // per-node id caches make later re-pointing unreliable.
        let collapse_t = std::time::Instant::now();
        sim.collapse_identity_port_nets();
        if std::env::var_os("XEZIM_COMPILE_PHASES").is_some() {
            eprintln!(
                "[COMPILE-PHASE] identity port-net collapse (construction): {:.1}ms",
                collapse_t.elapsed().as_secs_f64() * 1000.0
            );
        }
        sim.load_dpi_libraries();
        sim.bind_all_dpi_imports();
        sim.dpi_tasks_setup();
        // VPI modules register their $systf's before simulation starts. The
        // startup routines run with ACTIVE_SIMULATOR set, so a routine that
        // resolves a handle sees a built design.
        let vpi_paths = configured_vpi_libs();
        if !vpi_paths.is_empty() {
            let self_ptr = &mut sim as *mut Simulator;
            ACTIVE_SIMULATOR.with(|cell| cell.set(self_ptr));
            let prev_global =
                GLOBAL_ACTIVE_SIMULATOR.swap(self_ptr, std::sync::atomic::Ordering::AcqRel);
            let mut libs = std::mem::take(&mut sim.dpi_libraries);
            vpi_run_startup_routines(&mut libs, &vpi_paths);
            sim.dpi_libraries = libs;
            ACTIVE_SIMULATOR.with(|cell| cell.set(std::ptr::null_mut()));
            GLOBAL_ACTIVE_SIMULATOR.store(prev_global, std::sync::atomic::Ordering::Release);
        }
        sim
    }

    pub fn set_plusargs(&mut self, plusargs: &[String]) {
        self.plusargs = plusargs.to_vec();
        sim_dbg_eprintln!("[DEBUG] plusargs set: {:?}", self.plusargs);
        // Seeding. The default is SvRng::DEFAULT_SEED — a run is reproducible
        // unless it asks not to be, so a failing random test can be re-run
        // bit-for-bit. `+seed=<n>` selects another stream; `+seed=random` draws
        // one from system entropy and PRINTS it, since a seed you cannot see is
        // a run you cannot replay.
        for a in &self.plusargs {
            if let Some(v) = a.strip_prefix("seed=").or_else(|| a.strip_prefix("+seed=")) {
                let v = v.trim();
                if v.eq_ignore_ascii_case("random") {
                    use rand::Rng;
                    let seed = rand::rngs::StdRng::from_entropy().r#gen::<u64>();
                    eprintln!("[xezim] random seed: {} (replay with +seed={})", seed, seed);
                    self.rng = SvRng::from_seed(seed);
                    self.run_seed = Some(seed);
                } else if let Ok(seed) = v.parse::<u64>() {
                    self.rng = SvRng::from_seed(seed);
                    self.run_seed = Some(seed);
                } else {
                    eprintln!(
                        "[xezim][warning] ignoring malformed +seed={} (want an integer or 'random')",
                        v
                    );
                }
            }
        }
    }

    /// Reuse the runtime-neutral combinational worklist associated with an
    /// elaborated-design cache entry. Designs with UDP or annotated timing
    /// state bypass this cache in `build_comb_entries`.
    pub fn set_prepared_comb_cache_path(&mut self, path: Option<PathBuf>) {
        self.prepared_comb_cache_path = path;
    }

    /// Store the full CLI invocation (binary name + args + plusargs)
    /// for `vpi_get_vlog_info` to hand back to VPI/DPI callers. The strings
    /// are duplicated into `CString`s so the raw `*mut c_char` pointers in
    /// `vpi_argv` remain valid for the simulator's lifetime across multiple
    /// argument query calls.
    pub fn set_args(&mut self, argv: &[String]) {
        self.vpi_arg_cstrings.clear();
        self.vpi_argv.clear();
        // Empty argv still needs at least argv[0] = "xezim" so VPI/DPI
        // tool-info queries (which read product + argv[0]) don't blow
        // up. If the caller passed an empty list we still want a
        // sensible default.
        if argv.is_empty() {
            let c = CString::new("xezim").unwrap_or_default();
            self.vpi_argv.push(c.as_ptr() as *mut libc::c_char);
            self.vpi_arg_cstrings.push(c);
            sim_dbg_eprintln!("[VPI] set_args: defaulted to ['xezim']");
            return;
        }
        for s in argv {
            match CString::new(s.as_str()) {
                Ok(c) => {
                    self.vpi_argv.push(c.as_ptr() as *mut libc::c_char);
                    self.vpi_arg_cstrings.push(c);
                }
                Err(_) => {
                    // Embedded NUL — replace with empty string and push
                    // a copy with the NUL truncated.
                    let truncated = s.split('\0').next().unwrap_or("");
                    let c = CString::new(truncated).unwrap_or_default();
                    self.vpi_argv.push(c.as_ptr() as *mut libc::c_char);
                    self.vpi_arg_cstrings.push(c);
                }
            }
        }
        sim_dbg_eprintln!(
            "[VPI] set_args: {} entries (argv0={:?})",
            self.vpi_argv.len(),
            argv.first().map(String::as_str)
        );
    }

    /// Return the count of registered value-change callbacks.
    /// Test-only — used by the `dpi_uvm_test` integration test.
    #[doc(hidden)]
    pub fn dpi_value_change_cb_count(&self) -> usize {
        self.dpi_value_change_cbs.values().map(|v| v.len()).sum()
    }

    /// Return the count of cached DPI scopes.
    /// Test-only — used by the `dpi_uvm_test` integration test.
    #[doc(hidden)]
    pub fn dpi_scope_count(&self) -> usize {
        self.dpi_scopes.len()
    }
}
