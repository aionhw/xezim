//! Simulator compiled methods.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    pub(super) fn class_value_param_names(&self, cname: &str) -> Vec<String> {
        let mut out = Vec::new();
        let cd = match self.module.classes.get(cname) {
            Some(cd) => cd,
            None => return out,
        };
        for p in cd.param_order.iter() {
            if !cd.type_param_names.contains(p) {
                out.push(p.clone());
            }
        }
        out
    }
    pub(super) fn class_scoping_shadow_names(&self, cname: &str) -> HashSet<String> {
        let mut names = HashSet::default();
        let mut seen = HashSet::default();
        // Walk the method-class + enclosing chain, then their extends chains.
        let mut chain: Vec<String> = Vec::new();
        let mut cur = cname.to_string();
        loop {
            let Some(cd) = self.module.classes.get(&cur) else {
                break;
            };
            chain.push(cur.clone());
            match &cd.enclosing {
                Some(e) => cur = e.clone(),
                None => break,
            }
            if !seen.insert(cur.clone()) {
                break;
            }
        }
        // Also fold in each class's `extends` base chain (base members are
        // visible and could shadow a module name too).
        let mut all = chain.clone();
        let mut i = 0;
        while i < all.len() {
            let cn = all[i].clone();
            if let Some(cd) = self.module.classes.get(&cn) {
                if let Some(b) = &cd.extends {
                    if !seen.contains(b) && !all.contains(b) {
                        all.push(b.clone());
                    }
                }
            }
            i += 1;
        }
        for cn in &all {
            if let Some(cd) = self.module.classes.get(cn) {
                names.extend(cd.properties.keys().cloned());
            }
        }
        names
    }

    /// class-perf: the set of BARE idents (single segment, no selects) that
    /// resolve to `this.<member>` in a method body — a directly-declared or
    /// extends-inherited INSTANCE property of the method's class that is
    /// bare-key STABLE for every possible instance. A bare ident naming one
    /// of these compiles to `LoadClassMember(this_reg, name)` (matching the
    /// interpreter's method-member resolution). Everything else in
    /// `class_shadow_names` (shadowed members, enclosing-class members,
    /// statics) still BAILS: the compiled path cannot model shadowed storage
    /// keys nor class-static cells, so those must fall to the AST
    /// interpreter.
    pub(super) fn bare_class_member_names(&self, cname: &str) -> HashSet<String> {
        use std::collections::HashMap as StdMap;
        // Collect the method-class + its full extends chain (the classes whose
        // instance properties are visible on `this`). Enclosing-class members
        // are deliberately EXCLUDED: a bare name in a nested class does NOT
        // resolve to an enclosing class's member without `Outer::x`.
        let mut chain: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::default();
        let mut cur = cname.to_string();
        while let Some(cd) = self.module.classes.get(&cur) {
            chain.push(cur.clone());
            seen.insert(cur.clone());
            if let Some(b) = &cd.extends {
                if seen.contains(b) {
                    break;
                }
                cur = b.clone();
            } else {
                break;
            }
        }
        // Storage keys are laid out per instance: the LEAF-MOST declarer of a
        // name on the instance's chain keeps the bare key; every other
        // declarer stores under "<Class>::<name>". A bare-key read is
        // therefore only correct for ALL instances if NO class that can be
        // the leaf — i.e. cname itself or any TRANSITIVE DESCENDANT — other
        // than the chain declarer redeclares the name. Declarations on
        // UNRELATED chains are irrelevant (an instance is on one chain only):
        // UVM declares `m_parent` in uvm_component, uvm_phase, uvm_reg, ...
        // but that never collides, so a module-wide count is far too
        // conservative (it made every such getter bail).
        let mut descendant_decls: StdMap<&str, usize> = StdMap::new();
        for (dn, _) in self.module.classes.iter() {
            if seen.contains(dn) {
                continue; // chain classes were counted below
            }
            // Is `cname` on dn's ancestor chain?
            let mut a = dn.clone();
            let mut seen_a: HashSet<String> = HashSet::default();
            let mut is_desc = false;
            while let Some(ad) = self.module.classes.get(&a) {
                if !seen_a.insert(a.clone()) {
                    break;
                }
                if a == cname {
                    is_desc = true;
                    break;
                }
                match &ad.extends {
                    Some(b) => a = b.clone(),
                    None => break,
                }
            }
            if !is_desc {
                continue;
            }
            if let Some(dd) = self.module.classes.get(dn) {
                for p in dd.properties.keys() {
                    *descendant_decls.entry(p.as_str()).or_insert(0) += 1;
                }
            }
        }
        let mut names = HashSet::default();
        for cn in &chain {
            if let Some(cd) = self.module.classes.get(cn) {
                for p in cd.properties.keys() {
                    // STATIC members (and class-body localparams, which the
                    // elaborator also registers in `static_properties`) are
                    // NOT instance storage: their cell is per-class (and per
                    // SPECIALIZATION for parameterized classes), so a heap
                    // read is wrong (0 / generic value). They must stay on
                    // the interpreter path.
                    if cd.static_properties.contains(p) {
                        continue;
                    }
                    // The name must be declared EXACTLY ONCE on the instance
                    // chain rooted at this class — chain redeclarations below
                    // cname are descendant_decls, chain redeclarations above
                    // are impossible (chain classes each contribute their own
                    // names; a name appearing twice on the chain means two
                    // entries in `p`'s counting below — approximate that by
                    // tracking chain counts too).
                    let mut chain_count = 0usize;
                    for c2 in &chain {
                        if let Some(cd2) = self.module.classes.get(c2) {
                            if cd2.properties.contains_key(p.as_str()) {
                                chain_count += 1;
                            }
                        }
                    }
                    if chain_count == 1
                        && descendant_decls.get(p.as_str()).copied().unwrap_or(0) == 0
                    {
                        names.insert(p.clone());
                    }
                }
            }
        }
        // Unpacked arrays/queues/assoc members (and any other non-loadable
        // type) must NEVER be bare-resolved: the heap slot does not hold
        // their storage, and a member-access base like `queue.size()` would
        // silently become `this.size()` (infinite dispatch recursion —
        // uvm_queue::size regressed exactly this way). The nonloadable set
        // covers the whole extends chain, same as the walk above.
        for bad in self.class_nonloadable_member_names(cname) {
            names.remove(&bad);
        }
        names
    }

    /// class-perf: chain property names the compiled path must NOT lower to
    /// LoadClassMember/StoreClassMember. The heap only faithfully holds plain
    /// integral scalars and class handles; array/queue/assoc properties are
    /// keyed in their own tables, and unpacked-struct/string/real members
    /// need element-wise or text semantics the bytecode lacks. Loading such
    /// a member compiled to a bare-key heap read returned garbage (regression:
    /// array_equality_class — every `sa == rhs.sa` compared two zeros "equal").
    /// The caller merges these into the shadow-bail set so any reference bails
    /// to the AST path.
    pub(super) fn class_nonloadable_member_names(&self, cname: &str) -> HashSet<String> {
        let mut bad: HashSet<String> = HashSet::default();
        let mut seen: HashSet<String> = HashSet::default();
        let mut cur = cname.to_string();
        while let Some(cd) = self.module.classes.get(&cur) {
            if !seen.insert(cur.clone()) {
                break;
            }
            for p in cd.properties.keys().chain(cd.static_properties.iter()) {
                // Statics/localparams: per-class (or per-specialization)
                // cells the compiled path cannot address — always bail.
                // Arrays/queues/assoc: keyed in their own tables, not the
                // heap slot a bare-key LoadClassMember would read.
                let unpacked = cd.static_properties.contains(p)
                    || cd.array_properties.contains_key(p)
                    || cd.queue_properties.contains_key(p)
                    || cd.assoc_properties.contains_key(p);
                let integral_or_handle = cd
                    .property_types
                    .get(p)
                    .map(|dt| {
                        // Step 6: enum-typed properties are integral heap
                        // Values (width of the enum's base) — loadable.
                        self.scalar_formal_integral(dt).is_some()
                            || self.typeref_names_class(dt)
                            || self.typeref_names_enum(dt)
                            // class-perf (uvm_lru_cache::get): a member typed
                            // by a TypeReference that plan time cannot resolve
                            // (a TYPE PARAMETER — `DATA_T data;` in
                            // uvm_lru_cache_node — or a parameterized-class
                            // alias) is still a plain heap slot: the
                            // interpreter's dotted read fetches
                            // `properties[name]` verbatim (class_prop_width
                            // never resizes non-integral members), and
                            // LoadClassMember does the same. Specializations
                            // bind such params to class handles (the UVM LRU
                            // cache binds DATA_T=chandle), so the slot value
                            // passes through untouched. Only the heap-slot
                            // SHAPES that are NOT properties-keyed (collections
                            // above, statics above) stay refused.
                            || matches!(dt, crate::ast::types::DataType::TypeReference { .. })
                    })
                    // Unknown type (missing property_types entry): assume the
                    // safe side and refuse to lower.
                    .unwrap_or(false);
                // Virtual interfaces carry an instance-name binding outside
                // the heap Value. A typedef or type parameter may hide that
                // shape; copying the numeric cell alone loses the binding.
                // This admission set is shared across specializations, so a
                // type parameter that may carry a vif must keep the AST path.
                let vif_capable = Self::static_is_vif_capable(cd, p);
                if unpacked || vif_capable || !integral_or_handle {
                    bad.insert(p.clone());
                }
            }
            match &cd.extends {
                Some(b) => cur = b.clone(),
                None => break,
            }
        }
        bad
    }

    /// class-perf: does `dt` (a TypeReference, possibly through a typedef
    /// chain) name a CLASS in `module.classes`? Unlike `resolve_dt_ref` + a
    /// TypeReference test, this survives FORWARD typedefs (`typedef class
    /// uvm_component;` — UVM's forward declarations), which put a non-class
    /// placeholder in `typedef_types` and made `resolve_dt_ref` erase the
    /// class identity: every uvm_component/uvm_phase-returning getter then
    /// looked non-class and was skip-cached as non_scalar_ret. Each step of
    /// the walk checks the class table FIRST, so a class name short-circuits
    /// before its (placeholder) typedef entry can hide it.
    pub(super) fn typeref_names_class(&self, dt: &DataType) -> bool {
        self.typeref_class_name(dt).is_some()
    }

    /// class-perf Step 9e: the STATIC CLASS a class-typed reference names
    /// (through typedef chains), or None. Companion of `typeref_names_class`
    /// that keeps the resolved NAME so handle chains can walk it link by
    /// link.
    pub(super) fn typeref_class_name(&self, dt: &DataType) -> Option<String> {
        let mut cur = dt;
        for _ in 0..16 {
            let crate::ast::types::DataType::TypeReference { name: tn, .. } = cur else {
                return None;
            };
            // A typedef of an unpacked collection of class handles is a
            // collection, not a handle to its element class. Following only
            // typedef_types discards those unpacked dimensions.
            if self.fn_ret_coll_dims(cur).is_some() {
                return None;
            }
            if self.module.classes.contains_key(&tn.name.name) {
                return Some(tn.name.name.clone());
            }
            // class-perf P2: a PACKAGE-qualified class handle type
            // (`uvm_pkg::uvm_report_object` — the report macros' local
            // `_local_report_object_` declaration) resolves by its
            // rightmost segment against the class table.
            if let Some((_, base)) = tn.name.name.rsplit_once("::") {
                if self.module.classes.contains_key(base) {
                    return Some(base.to_string());
                }
            }
            // class-perf: a parameterized-class typedef target hoisted from a
            // class-local alias (`typedef uvm_lru_cache_node#(KEY_T,DATA_T)
            // node_type;` — the UVM LRU cache's `m_hash`/`m_begin`/`node`
            // declarations) carries its spec-args in the NAME at plan time
            // (no specialization context exists yet). The member reads off
            // such a handle (`node.data`, `node.prev.next`) only need the
            // member ADMISSION sets of the defining class, which are
            // per-class-name and inheritance-safe — strip the `#(...)`
            // fragment and resolve to the base class.
            if let Some((base, _)) = tn.name.name.split_once('#') {
                if self.module.classes.contains_key(base) {
                    return Some(base.to_string());
                }
            }
            match self.module.typedef_types.get(&tn.name.name) {
                Some(next) => cur = next,
                None => return None,
            }
        }
        None
    }

    /// class-perf Step 9e: per-class map of member name -> the member's
    /// static class type (extends-chain walked, leaf-most declarer wins).
    /// Drives TYPED handle-chain admission in the compiler: a dispatch
    /// receiver chain (`m_cb.cnt.num()`) must prove every link is
    /// class-typed. The untyped shadow-only check admitted COLLECTION
    /// members of other classes (`cnt` is inner's assoc, invisible in
    /// outer's shadow sets), lowering to a garbage LoadClassMember handle —
    /// the heartbeat regression (`uvm_heartbeat::remove` never deleted mc1).
    /// class-perf P2: method names whose return type is `string` in EVERY
    /// class that declares them (unanimous, name-global). Lets the bytecode
    /// compiler type `obj.get_full_name()` as a string CONCAT operand
    /// without per-receiver type knowledge. A name declared anywhere with a
    /// non-string return (or as a task, which has none) stays OUT — unknown,
    /// conservative, never a wrong byte-level join. Covers in-class
    /// prototypes (extern bodies match them) at any virtual level; nested
    /// classes are not walked (none in the UVM library).
    pub(super) fn class_unanimous_string_methods(&self) -> HashSet<String> {
        use crate::ast::types::DataType as DT;
        use crate::ast::types::SimpleType;
        let mut rets: HashMap<String, bool> = HashMap::default();
        for (_, cd) in self.module.classes.iter() {
            for (_, m) in cd.methods.iter() {
                let (name, is_str) = match &m.kind {
                    crate::ast::decl::ClassMethodKind::Function(f)
                    | crate::ast::decl::ClassMethodKind::Extern(f) => (
                        f.name.name.name.clone(),
                        matches!(
                            &f.return_type,
                            DT::Simple {
                                kind: SimpleType::String,
                                ..
                            }
                        ),
                    ),
                    // Task or pure-virtual prototype: a task has no return
                    // value, disqualify the name outright.
                    crate::ast::decl::ClassMethodKind::Task(t) => (t.name.name.name.clone(), false),
                    crate::ast::decl::ClassMethodKind::PureVirtual(f) => {
                        (f.name.name.name.clone(), false)
                    }
                };
                rets.entry(name)
                    .and_modify(|v| *v &= is_str)
                    .or_insert(is_str);
            }
        }
        rets.into_iter()
            .filter(|(_, v)| *v)
            .map(|(k, _)| k)
            .collect()
    }

    pub(super) fn class_handle_member_types(&self) -> HashMap<String, HashMap<String, String>> {
        let mut out: HashMap<String, HashMap<String, String>> = HashMap::default();
        for (cn, _) in self.module.classes.iter() {
            let mut members: HashMap<String, String> = HashMap::default();
            let mut seen = HashSet::default();
            let mut cur = cn.clone();
            while let Some(c) = self.module.classes.get(&cur) {
                if !seen.insert(cur.clone()) {
                    break;
                }
                for (p, dt) in c.property_types.iter() {
                    // or_insert: the most-derived declarer wins; ancestors
                    // only contribute names the chain hasn't declared.
                    // Step 9e: collection members (queue/dyn-array/assoc)
                    // whose ELEMENT type is a class also land here so the
                    // compiler can tell a HANDLE element read from an
                    // integral one — `LoadCollElem` must then never Resize
                    // to the context width (a 1-bit if-context destroyed
                    // uvm_packer::pack_object_with_meta's cycle-detection
                    // handle, infinite recursion in pack_object, 6693).
                    // The compiler's scalar-vs-collection paths consult the
                    // collection-name sets FIRST, so this entry is only
                    // visible to element-typed logic.
                    if let Some(t) = self.typeref_class_name(dt) {
                        members.entry(p.clone()).or_insert(t);
                    }
                }
                // class-perf P2: METHOD return classes join the same
                // per-class map (a property of the same name wins via
                // or_insert): a dispatch chain may continue through a
                // method RESULT — `uvm_get_report_object().
                // uvm_get_report_object().get_report_...` (the report
                // macros' default RO). Tasks and non-class returns stay
                // out; parameterized specializations are not plain class
                // names and typeref_class_name declines them.
                for (m, cm) in c.methods.iter() {
                    if let crate::ast::decl::ClassMethodKind::Function(f) = &cm.kind {
                        if let Some(t) = self.typeref_class_name(&f.return_type) {
                            if !t.contains('#') {
                                members.entry(m.clone()).or_insert(t);
                            }
                        }
                    }
                }
                match &c.extends {
                    Some(b) => cur = b.clone(),
                    None => break,
                }
            }
            out.insert(cn.clone(), members);
        }
        out
    }

    /// class-perf Step 6: does `dt` (possibly through a typedef chain)
    /// resolve to an ENUM type? Enum properties are integral heap Values
    /// (the interpreter stores them width-of-base), so they load/store like
    /// scalars; enum-RETURNING getters (uvm_phase::get_phase_type) were
    /// skip-cached non_scalar_ret because scalar_formal_integral excludes
    /// enums (its formals were never width-adapted — a RETURN has no such
    /// history).
    pub(super) fn typeref_names_enum(&self, dt: &DataType) -> bool {
        use crate::ast::types::DataType as DT;
        let mut cur = dt;
        for _ in 0..16 {
            match cur {
                DT::Enum(_) => return true,
                DT::TypeReference { name: tn, .. } => {
                    match self.module.typedef_types.get(&tn.name.name) {
                        Some(next) => cur = next,
                        None => return false,
                    }
                }
                _ => return false,
            }
        }
        false
    }

    /// (width, signed) of an enum type reached through typedefs; None when
    /// `dt` is not an enum. Width is the enum's BASE width (§6.19).
    pub(super) fn enum_type_info(&self, dt: &DataType) -> Option<(u32, bool)> {
        if !self.typeref_names_enum(dt) {
            return None;
        }
        let resolved = super::elaborate::resolve_typedef_chain(dt, &self.module.typedef_types);
        let crate::ast::types::DataType::Enum(_) = &resolved else {
            return None;
        };
        let w = super::elaborate::resolve_type_width(
            &resolved,
            Some(&self.module.parameters),
            Some(&self.module.typedefs),
        );
        if w == 0 {
            return None;
        }
        let signed =
            super::elaborate::is_type_signed_resolved(&resolved, &self.module.typedef_types);
        Some((w, signed))
    }

    /// class-perf Step 6: names of body VarDecls whose type is a CLASS
    /// (through typedefs), found recursively through nested statements.
    /// A class-typed local holds a heap HANDLE in a VM register — like a
    /// class formal — so `.member` on it lowers to a heap access.
    pub(super) fn class_typed_local_names(&self, body: &[Statement]) -> HashSet<String> {
        self.class_typed_local_types(body)
            .into_iter()
            .map(|(n, _)| n)
            .collect()
    }

    /// class-perf P2: child statements of `st`, for full-body walks
    /// (foreach loop-var typing walks into if/else/case/loop bodies the
    /// same as the VarDecl walk above).
    pub(super) fn sub_stmts(st: &Statement) -> Vec<&Statement> {
        use crate::ast::stmt::StatementKind as SK;
        let mut out: Vec<&Statement> = Vec::new();
        match &st.kind {
            SK::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                out.push(then_stmt);
                if let Some(e) = else_stmt {
                    out.push(e);
                }
            }
            SK::Case { items, .. } => out.extend(items.iter().map(|it| &it.stmt)),
            SK::For { body, .. }
            | SK::Foreach { body, .. }
            | SK::ForeachTail { body, .. }
            | SK::While { body, .. }
            | SK::DoWhile { body, .. }
            | SK::Repeat { body, .. }
            | SK::Forever { body, .. }
            | SK::ForeverTail { body, .. }
            | SK::TimingControl { stmt: body, .. }
            | SK::Wait { stmt: body, .. } => out.push(body),
            SK::SeqBlock { stmts, .. } | SK::ParBlock { stmts, .. } => out.extend(stmts.iter()),
            _ => {}
        }
        out
    }

    /// class-perf P2 (typed handle roots): the declared CLASS of each
    /// class-typed body local (`Node n;` -> `Node`), declaration order,
    /// outer scopes first (`entry().or_insert` mirrors shadowing). Feeds
    /// `method_handle_chain_class` so dispatch chains rooted at a LOCAL
    /// resolve statically instead of declining as untyped roots, and the
    /// `lhs = new` target map (`ctxt = new;` inside `else begin…end`,
    /// uvm_objection::m_drop). VarDecls at ANY statement nesting depth
    /// count (if/else/case/loop/timing-control bodies included).
    pub(super) fn class_typed_local_types(&self, body: &[Statement]) -> Vec<(String, String)> {
        fn walk(st: &Statement, sim: &Simulator, out: &mut Vec<(String, String)>) {
            use crate::ast::stmt::StatementKind as SK;
            match &st.kind {
                SK::VarDecl {
                    data_type,
                    declarators,
                    ..
                } => {
                    if let Some(cls) = sim.typeref_class_name(data_type) {
                        for d in declarators {
                            out.push((d.name.name.clone(), cls.clone()));
                        }
                    }
                }
                SK::If {
                    then_stmt,
                    else_stmt,
                    ..
                } => {
                    walk(then_stmt, sim, out);
                    if let Some(e) = else_stmt {
                        walk(e, sim, out);
                    }
                }
                SK::Case { items, .. } => {
                    for it in items {
                        walk(&it.stmt, sim, out);
                    }
                }
                SK::For { body, .. }
                | SK::Foreach { body, .. }
                | SK::ForeachTail { body, .. }
                | SK::While { body, .. }
                | SK::DoWhile { body, .. }
                | SK::Repeat { body, .. }
                | SK::Forever { body, .. }
                | SK::ForeverTail { body, .. }
                | SK::TimingControl { stmt: body, .. }
                | SK::Wait { stmt: body, .. } => walk(body, sim, out),
                SK::SeqBlock { stmts, .. } | SK::ParBlock { stmts, .. } => {
                    for s in stmts {
                        walk(s, sim, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for st in body {
            walk(st, self, &mut out);
        }
        out
    }

    pub(super) fn class_typed_member_names(&self, cname: &str) -> HashSet<String> {
        let mut out = HashSet::default();
        let mut seen = HashSet::default();
        let mut cur = cname.to_string();
        while let Some(cd) = self.module.classes.get(&cur) {
            if !seen.insert(cur.clone()) {
                break;
            }
            for (p, dt) in cd.property_types.iter() {
                if self.typeref_names_class(dt) {
                    out.insert(p.clone());
                }
            }
            match &cd.extends {
                Some(b) => cur = b.clone(),
                None => break,
            }
        }
        out
    }

    /// class-perf Step 6: members that a DOTTED member access (`base.m`)
    /// may lower to a bare-key heap access. The interpreter's dotted read
    /// AND write use the instance's runtime key (`properties["m"]`) — the
    /// same key LoadClassMember/StoreClassMember use — so ANY instance
    /// property of the method-class's extends chain is faithful, even a
    /// shadowed one (the leaf-most declarer owns the bare key both ways).
    /// NOT safe: statics/localparams (per-class cells), non-loadable types
    /// (arrays/queues/assoc/structs/strings/reals), and enclosing-class
    /// members (not instantiated on a nested-class instance).
    pub(super) fn class_member_access_safe_names(&self, cname: &str) -> HashSet<String> {
        let nonloadable = self.class_nonloadable_member_names(cname);
        let mut safe = HashSet::default();
        let mut seen = HashSet::default();
        let mut cur = cname.to_string();
        loop {
            let Some(cd) = self.module.classes.get(&cur) else {
                break;
            };
            if !seen.insert(cur.clone()) {
                break;
            }
            for p in cd.properties.keys() {
                if !nonloadable.contains(p) {
                    safe.insert(p.clone());
                }
            }
            match &cd.extends {
                Some(b) => cur = b.clone(),
                None => break,
            }
        }
        safe
    }

    /// class-perf Step 9a: string-typed instance members of `cname`'s chain.
    /// The heap stores a string property as a plain byte-vector Value and the
    /// interpreter's dotted read passes it through verbatim (`fit_class_prop`
    /// never resizes — `class_prop_width_impl` returns None for strings), so
    /// a bare/dotted READ lowers to LoadClassMember like an integral member.
    /// Statics are excluded (per-class cells).
    pub(super) fn class_string_member_names(&self, cname: &str) -> HashSet<String> {
        let mut out = HashSet::default();
        let mut seen = HashSet::default();
        let mut cur = cname.to_string();
        while let Some(cd) = self.module.classes.get(&cur) {
            if !seen.insert(cur.clone()) {
                break;
            }
            for p in cd.string_properties.iter() {
                if !cd.static_properties.contains(p) {
                    out.insert(p.clone());
                }
            }
            match &cd.extends {
                Some(b) => cur = b.clone(),
                None => break,
            }
        }
        out
    }

    /// class-perf Step 9b: member-collection names callable with a value-
    /// arg builtin in a method body of `cname`. The UNION of
    /// `assoc_properties`/`queue_properties` keys over the extends chain —
    /// instance stores (`<h>#member`), always runtime-resolvable. Statics
    /// (`static_collections`) are deliberately EXCLUDED here: a bare
    /// receiver resolves them through the interpreter's §8.10 rewrite (see
    /// the compiler field doc), and a dotted receiver must stay AST (it
    /// otherwise reaches `exec_method_call`'s broken zero path) — so a
    /// static name may only join the set at the CALL SITE, and only for a
    /// bare receiver. Fixed arrays (`array_properties`) DELIBERATELY stay
    /// OUT of this builtin-call set: they have no legal value-arg builtin
    /// methods (`hist.size()`, `push_back`, ...), so adding them here would
    /// wrongly admit such calls to `CallCollMethod`. Fixed arrays DO
    /// participate in ELEMENT access (`hist[i]`), and their names are added
    /// specifically to `class_coll_elem_member_names` (not here), so a bare
    /// element read/write lowers to `LoadCollElem`/`StoreCollElem` while
    /// builtin-call admission is untouched. Type-param-bound collections
    /// (`prop_bound_collection`) are instance-dependent and cannot join a
    /// plan set — the runtime bare path still resolves them through
    /// `instance_assoc_member`.
    pub(super) fn class_coll_member_names(&self, cname: &str) -> HashSet<String> {
        let mut out = HashSet::default();
        let mut seen = HashSet::default();
        let mut cur = cname.to_string();
        while let Some(cd) = self.module.classes.get(&cur) {
            if !seen.insert(cur.clone()) {
                break;
            }
            out.extend(cd.assoc_properties.keys().cloned());
            out.extend(cd.queue_properties.keys().cloned());
            match &cd.extends {
                Some(b) => cur = b.clone(),
                None => break,
            }
        }
        out
    }

    /// class-perf Step 9b: the `static_collections` names of `cname`'s
    /// chain — bare-receiver admission only (see
    /// `PreboundCompiledMethod::static_coll_members`).
    pub(super) fn class_static_coll_member_names(&self, cname: &str) -> HashSet<String> {
        let mut out = HashSet::default();
        let mut seen = HashSet::default();
        let mut cur = cname.to_string();
        while let Some(cd) = self.module.classes.get(&cur) {
            if !seen.insert(cur.clone()) {
                break;
            }
            out.extend(cd.static_collections.iter().map(|(n, _, _)| n.clone()));
            match &cd.extends {
                Some(b) => cur = b.clone(),
                None => break,
            }
        }
        out
    }

    /// class-perf Step 9e: METHOD names visible on `cname`'s chain — the
    /// admission set for bare this-bounded calls (`helper(2)`) in compiled
    /// bodies. Runtime re-enters the dispatcher, so any kind (functions,
    /// ctors included) is safe to admit: §13.3 keeps task calls out of
    /// function bodies, and a prototype/missing method resolves exactly as
    /// the AST funnel would.
    pub(super) fn class_method_name_set(&self, cname: &str) -> HashSet<String> {
        let mut out = HashSet::default();
        let mut seen = HashSet::default();
        let mut cur = cname.to_string();
        while let Some(cd) = self.module.classes.get(&cur) {
            if !seen.insert(cur.clone()) {
                break;
            }
            out.extend(cd.methods.keys().cloned());
            match &cd.extends {
                Some(b) => cur = b.clone(),
                None => break,
            }
        }
        // CONSTRUCTORS are excluded: `new(...)` parses as a call to the
        // name `new`, but it means ALLOCATE A FRESH OBJECT and run its
        // ctor — never `this.new` (a bare this-bounded call). Admitting it
        // would compile `m = new;` into `CallMethod(this, "new")`, which
        // recursively re-runs the ctor on the CURRENT instance and leaves
        // the target handle null (caught by the virtual-method-in-binary
        // regression: pre=0, post=0).
        out.remove("new");
        out
    }

    /// LRM 11.6 / 11.8.2: the declared input-port widths of every method
    /// visible from `cname`, keyed by method name.
    ///
    /// A call ACTUAL is CONTEXT-determined by the callee's declared formal,
    /// exactly like the operand of a comparison: with
    /// `function bit wide9(logic [8:0] v)`, the call site `wide9(a + b)`
    /// must evaluate `a + b` at 9 bits (300), not at its 8-bit self width
    /// (44).  The inline path already binds actuals with
    /// `port_effective_width`; the `CallMethod` / `CallScopedMethod`
    /// lowerings have no class table in scope and compiled every actual
    /// against `ctx_width = 0`, so the widths have to be resolved here.
    ///
    /// Non-input, dimensioned, or non-scalar-integral ports contribute 0,
    /// which is the "keep the source width" sentinel the compiler already
    /// uses — those calls behave exactly as before this resolution existed.
    /// First definition in the lexical chain wins: §8.20 binds a non-virtual
    /// bare call to the first defining class, and a virtual override may only
    /// redeclare the same prototype.
    pub(super) fn class_method_formal_widths(&self, cname: &str) -> HashMap<String, Vec<u32>> {
        let mut out = self.chain_method_formal_widths(cname);
        // `super.m` calls the PARENT's `m`, so its widths come from the
        // parent's chain, kept under `super.<m>` apart from an `m` this class
        // hides (§13.5.1).
        if let Some(parent) = self
            .module
            .classes
            .get(cname)
            .and_then(|cd| cd.extends.as_deref())
        {
            for (m, w) in self.chain_method_formal_widths(parent) {
                out.insert(format!("super.{m}"), w);
            }
        }
        out
    }

    /// `class_method_formal_widths` of the class chain from `cname` up.
    fn chain_method_formal_widths(&self, cname: &str) -> HashMap<String, Vec<u32>> {
        use crate::ast::decl::ClassMethodKind as Kind;
        use crate::ast::types::PortDirection;
        let mut out: HashMap<String, Vec<u32>> = HashMap::default();
        let mut seen: HashSet<String> = HashSet::default();
        let mut cur = cname.to_string();
        while let Some(cd) = self.module.classes.get(&cur) {
            if !seen.insert(cur.clone()) {
                break;
            }
            for (mname, md) in cd.methods.iter() {
                if out.contains_key(mname.as_str()) {
                    continue;
                }
                let ports = match &md.kind {
                    Kind::Function(f) | Kind::Extern(f) | Kind::PureVirtual(f) => &f.ports,
                    Kind::Task(t) => &t.ports,
                };
                let widths = ports
                    .iter()
                    .map(|p| {
                        if p.direction == PortDirection::Input && p.dimensions.is_empty() {
                            self.scalar_formal_integral(&p.data_type)
                                .map(|(w, _)| w)
                                .unwrap_or(0)
                        } else {
                            0
                        }
                    })
                    .collect();
                out.insert(mname.clone(), widths);
            }
            match &cd.extends {
                Some(b) => cur = b.clone(),
                None => break,
            }
        }
        out
    }

    /// class-perf Step 9f: the `super.m(...)` admission set — method
    /// names visible in the PARENT chain of `cname` (the class lexically
    /// containing the call), plus `new` (super.new is ctor chaining, always
    /// routed). Mirrors the interpreter's `routed` gate: `super.m` where
    /// only `cname` itself defines `m` does NOT route statically (the AST
    /// funnel falls through to virtual dispatch on `this`), so it must not
    /// compile here.
    pub(super) fn super_method_name_set(&self, cname: &str) -> HashSet<String> {
        let mut out = HashSet::default();
        let Some(mut cur) = self
            .module
            .classes
            .get(cname)
            .and_then(|cd| cd.extends.clone())
        else {
            return out;
        };
        let mut seen = HashSet::default();
        while let Some(cd) = self.module.classes.get(&cur) {
            if !seen.insert(cur.clone()) {
                break;
            }
            out.extend(cd.methods.keys().cloned());
            match &cd.extends {
                Some(b) => cur = b.clone(),
                None => break,
            }
        }
        out.insert("new".to_string());
        out
    }

    /// class-perf Step 9b-ii: the BARE element-access admission set —
    /// `class_coll_member_names ∪ class_static_coll_member_names` MINUS
    /// every module-scope collection-table name. The AST write funnel
    /// probes the module queue/array/assoc tables BEFORE its class-member
    /// arm, so a bare name that is also a module collection must keep its
    /// element accesses on the AST path (see
    /// `PreboundCompiledMethod::coll_elem_members`).
    /// class-perf Step 9b (+ class-perf task a): the plan set for ELEMENT
    /// read/write (`fine[i]`, `aa[k]`, `this.m[j]`, `obj.m[k]`) in a method
    /// body of `cname`. Built on the collection-element names, PLUS the
    /// class's own single-dimension FIXED-ARRAY members (`array_properties`,
    /// walked over the extends chain). Fixed arrays are deliberately NOT in
    /// `class_coll_member_names` (they have no builtin methods), but a bare
    /// or dotted element access `hist[i]` on one IS lowerable: the
    /// `LoadCollElem`/`StoreCollElem` exec arms resolve the store through
    /// `instance_assoc_member`, which — because `build_class_coll_index`
    /// already includes `array_properties` — returns the SAME `<handle>#hist`
    /// store the AST interpreter's `expr_assoc_name` uses, and read/write
    /// `signals["<handle>#hist[i]"]` byte-identically. Without this, the
    /// element-select forced a whole-method `method_class_shadow` fallback to
    /// the AST interpreter for every array-member method (measured 83% of a
    /// customer run's wall time).
    pub(super) fn class_coll_elem_member_names(&self, cname: &str) -> HashSet<String> {
        let mut out = self.class_coll_member_names(cname);
        out.extend(self.class_static_coll_member_names(cname));
        // Fixed-size instance array members (single dimension, the hist[4]
        // shape) join the ELEMENT set only — never the builtin-call set.
        let mut seen = HashSet::default();
        let mut cur = cname.to_string();
        while let Some(cd) = self.module.classes.get(&cur) {
            if !seen.insert(cur.clone()) {
                break;
            }
            for p in cd.array_properties.keys() {
                // Leave static fixed arrays on their class-qualified store
                // path (handled by class_static_coll_member_names); here we
                // admit only INSTANCE fixed-array members (a bare read
                // resolves `<handle>#p` for an instance, a static resolves a
                // different key and must keep its own admission).
                if !cd.static_properties.contains(p) {
                    out.insert(p.clone());
                }
            }
            match &cd.extends {
                Some(b) => cur = b.clone(),
                None => break,
            }
        }
        out.retain(|n| {
            !self.module.arrays.contains_key(n)
                && !self.module.arrays_2d.contains_key(n)
                && !self.module.arrays_nd.contains_key(n)
                && !self.module.dynamic_arrays.contains(n)
                && !self.module.associative_arrays.contains_key(n)
                && !self.module.queue_vars.contains(n)
        });
        out
    }

    /// issue-301: element packed dims for members whose ELEMENT type is a
    /// MULTI-dimensional packed array (`bit [8:0][7:0] arr[3]`, and the
    /// same element shape behind queues/dynamic/associative members). A
    /// further select on a loaded element (`arr[j][2]`) is a packed
    /// ELEMENT select — the compiled lowering needs the dims to slice the
    /// right ew bits instead of taking one bit. `packed_full_dims_chained`
    /// reports None for single-vector/scalar elements (and struct
    /// elements), which keep the old lowering.
    pub(super) fn class_coll_elem_packed_dims(
        &self,
        cname: &str,
    ) -> HashMap<String, Vec<(i64, i64)>> {
        let mut out = HashMap::default();
        if cname.starts_with("__free_fn__") {
            return out;
        }
        let mut seen = HashSet::default();
        let mut cur = cname.to_string();
        while let Some(cd) = self.module.classes.get(&cur) {
            if !seen.insert(cur.clone()) {
                break;
            }
            for (p, dt) in cd.property_types.iter() {
                if let Some(dims) = super::elaborate::packed_full_dims_chained(
                    dt,
                    &self.module.parameters,
                    &self.module.typedef_types,
                ) {
                    out.insert(p.clone(), dims);
                }
            }
            match &cd.extends {
                Some(b) => cur = b.clone(),
                None => break,
            }
        }
        out
    }

    /// Persistent method keys include the complete elaborated design context,
    /// the compiler version salt, and the method's resolved width signature.
    /// The shared context digest is lazy: disk caching pays once per simulator,
    /// while ordinary in-memory method caching never serializes the design.
    pub(super) fn persisted_method_key(
        &mut self,
        handle: usize,
        cname: &str,
        method_name: &str,
        formal_widths: &[u32],
        result_width: u32,
    ) -> Option<u64> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        METHOD_CACHE_COMPILER_SALT.hash(&mut h);
        // Methods embed package constants, typedef targets, foreign class
        // metadata and signal IDs, not just their own class declaration.
        // Hash the elaborated context once, only when disk caching is used.
        // Stream serialization into the hasher to avoid a design-sized buffer.
        let context = if let Some(context) = self.persisted_method_context_hash {
            context
        } else {
            struct HashWriter(std::collections::hash_map::DefaultHasher);
            impl std::io::Write for HashWriter {
                fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                    self.0.write(bytes);
                    Ok(bytes.len())
                }
                fn flush(&mut self) -> std::io::Result<()> {
                    Ok(())
                }
            }
            let mut writer = HashWriter(std::collections::hash_map::DefaultHasher::new());
            bincode::serde::encode_into_std_write(
                &self.module,
                &mut writer,
                xezim_core::xez_bincode_config(),
            )
            .ok()?;
            let mut signals: Vec<_> = self.signal_name_to_id.iter().collect();
            signals.sort_unstable_by(|a, b| a.0.cmp(b.0));
            signals.hash(&mut writer.0);
            self.signal_widths.hash(&mut writer.0);
            self.signal_signed.hash(&mut writer.0);
            let context = writer.0.finish();
            self.persisted_method_context_hash = Some(context);
            context
        };
        context.hash(&mut h);
        // §8.25: identical formal widths can still have different local T types.
        if let Some(inst) = self.heap.get(handle).and_then(|inst| inst.as_ref()) {
            inst.spec.hash(&mut h);
            let mut bindings: Vec<_> = inst.type_bindings.iter().collect();
            bindings.sort_unstable_by(|a, b| a.0.cmp(b.0));
            bindings.hash(&mut h);
        }
        self.current_spec.hash(&mut h);
        cname.hash(&mut h);
        method_name.hash(&mut h);
        formal_widths.hash(&mut h);
        result_width.hash(&mut h);
        Some(h.finish())
    }

    /// Disk probe for one method-cache entry. Any error (missing file,
    /// truncated/garbage payload, deserialization mismatch after a compiler
    /// change) is a MISS: the caller recompiles and overwrites.
    pub(super) fn load_persisted_method(
        &self,
        key: u64,
    ) -> Option<super::bytecode::PersistedMethodOutcome> {
        let dir = method_cache_dir()?;
        let bytes = std::fs::read(dir.join(format!("m{:016x}.bin", key))).ok()?;
        super::bytecode::decode_persisted_method_outcome(&bytes)
    }

    /// Best-effort atomic store (tmp file + rename; the pid suffix keeps
    /// concurrent workers from clobbering each other's tmp files). A failed
    /// write NEVER fails the simulation — the next run just recompiles.
    pub(super) fn store_persisted_method(
        &self,
        key: u64,
        outcome: &super::bytecode::PersistedMethodOutcome,
    ) {
        let Some(dir) = method_cache_dir() else {
            return;
        };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        let Ok(bytes) = bincode::serde::encode_to_vec(outcome, xezim_core::xez_bincode_config())
        else {
            return;
        };
        let tmp = dir.join(format!("m{:016x}.tmp.{}", key, std::process::id()));
        let path = dir.join(format!("m{:016x}.bin", key));
        if std::fs::write(&tmp, &bytes)
            .and_then(|_| std::fs::rename(&tmp, &path))
            .is_err()
        {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    /// Wait-free compiled execution of a task-method body dispatched from a
    /// process statement runner. The caller has already bound the task frame
    /// (`bind_task_frame` + method-this context) exactly as the scheduled
    /// path would. If the body compiles — the compiler declines every timing
    /// control — it can only run to completion synchronously, so the
    /// scheduling machinery is skipped entirely and the caller continues as
    /// if the scheduled frame had finished. Returns `None` when the body does
    /// not (yet) compile; the caller falls back to the scheduled path.
    pub(super) fn try_compiled_task_body(
        &mut self,
        handle: usize,
        mclass: &str,
        mname: &str,
        td: &TaskDeclaration,
    ) -> Option<Value> {
        if !compiled_methods_enabled() || handle == 0 {
            return None;
        }
        // A task already known never to compile (it can suspend, or its body
        // does not lower) returns before any frame bookkeeping.
        let spec_id = self.instance_spec_id(handle);
        let ids = &self.compiled_class_method_ids;
        if let (Some(&cid), Some(&mid)) = (ids.get(mclass), ids.get(mname))
            && self.compiled_method_skip.contains(&(cid, mid, spec_id))
        {
            return None;
        }
        let sync_name = self.sync_frame_name(mname);
        self.static_local_syncs.push((sync_name, Vec::new()));
        let saved_meth = if SAMPLER_READY.load(std::sync::atomic::Ordering::Relaxed) {
            sampler_register_name(fnv_name(mname), mname);
            Some(CUR_METHOD_HASH.swap(fnv_name(mname), std::sync::atomic::Ordering::Relaxed))
        } else {
            None
        };
        let saved_compiled = if SAMPLER_READY.load(std::sync::atomic::Ordering::Relaxed) {
            Some(CUR_METHOD_COMPILED.swap(false, std::sync::atomic::Ordering::Relaxed))
        } else {
            None
        };
        let r = self.try_run_compiled_method(
            handle,
            mclass,
            mname,
            &td.ports,
            &td.items,
            None,
            false,
            CompiledBodyKind::Task(td),
        );
        if let Some(prev) = saved_meth {
            CUR_METHOD_HASH.store(prev, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(prev) = saved_compiled {
            CUR_METHOD_COMPILED.store(prev, std::sync::atomic::Ordering::Relaxed);
        }
        if r.is_some() {
            // Persist static locals seeded by the block (mirrors the
            // interpreter epilogue's sync pass).
            self.sync_static_locals();
        } else if let Some((name, _)) = self.static_local_syncs.pop()
            && self.sync_name_pool.len() < 64
        {
            self.sync_name_pool.push(name);
        }
        r
    }

    /// The synthetic class name a free function compiles under:
    /// `__free_fn__<pkg>`, or `__free_fn__` for a `$unit`/module-scope
    /// function. Interned once per package, so the per-call hook below
    /// allocates nothing.
    pub(super) fn free_fn_class_name(&mut self, pkg: Option<&str>) -> std::rc::Rc<str> {
        let pkg = pkg.unwrap_or("");
        if let Some(c) = self.free_fn_class_names.get(pkg) {
            return c.clone();
        }
        let c: std::rc::Rc<str> = std::rc::Rc::from(format!("__free_fn__{pkg}"));
        self.free_fn_class_names.insert(pkg.to_string(), c.clone());
        c
    }

    /// §26.3: the package a compiled `CallFreeFunction` callee runs in. The
    /// compiler names a contested callee `P::name` (the calling package's
    /// own subroutine); any other callee runs in its declaring scope, as
    /// `exec_function_call` derives it. Memoized per call name.
    pub(super) fn free_call_pkg(
        &mut self,
        call_name: &str,
        decl_name: &str,
    ) -> Option<std::rc::Rc<str>> {
        if let Some(p) = self.free_call_pkgs.get(call_name) {
            return p.clone();
        }
        let p: Option<std::rc::Rc<str>> = match call_name.rsplit_once("::") {
            Some((pkg, _)) => Some(std::rc::Rc::from(pkg)),
            None => self
                .module
                .func_decl_scope
                .get(decl_name)
                .map(|p| std::rc::Rc::from(p.as_str())),
        };
        self.free_call_pkgs.insert(call_name.to_string(), p.clone());
        p
    }

    /// class-perf free-function tier: run a package/module-scope FUNCTION
    /// body as bytecode, reusing `try_run_compiled_method`'s entire plan +
    /// admission + block-cache machinery under a SYNTHETIC class name. The
    /// interpreter's frame for this call already holds the bound formals and
    /// the implicit return cell; the callee runs with `this == 0` (pushed
    /// null by the caller), so any member access degrades exactly as the interpreter
    /// would — and a body that touches `this`-relative names was never
    /// compilable anyway (the compiler's handle-chain gates require a
    /// class-typed root, which no free-function frame provides).
    /// Whether a subroutine's formals or body variables declare an explicit
    /// packed range other than `[N-1:0]` (a non-zero low bound or an
    /// ascending range), or whose right bound is not a constant.
    fn declares_offset_packed_range(
        &self,
        ports: &[crate::ast::decl::FunctionPort],
        body: &[crate::ast::stmt::Statement],
    ) -> bool {
        use crate::ast::stmt::StatementKind;
        use crate::ast::types::{DataType, PackedDimension};
        let offset = |dt: &DataType| {
            let dims = match dt {
                DataType::IntegerVector { dimensions, .. }
                | DataType::Implicit { dimensions, .. }
                | DataType::TypeReference { dimensions, .. } => dimensions,
                _ => return false,
            };
            dims.iter().any(|d| match d {
                PackedDimension::Range { left, right, .. } => {
                    let c = |e| {
                        super::elaborate::const_eval_i64_with_params(
                            e,
                            Some(&self.module.parameters),
                        )
                    };
                    // `[W-1:0]` keeps compiling whatever `W` is.
                    c(right) != Some(0) || c(left).is_some_and(|l| l < 0)
                }
                PackedDimension::Unsized(_) => false,
            })
        };
        fn walk(s: &crate::ast::stmt::Statement, f: &dyn Fn(&DataType) -> bool) -> bool {
            match &s.kind {
                StatementKind::VarDecl { data_type, .. } => f(data_type),
                StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                    stmts.iter().any(|s| walk(s, f))
                }
                _ => false,
            }
        }
        ports.iter().any(|p| offset(&p.data_type)) || body.iter().any(|s| walk(s, &offset))
    }

    pub(super) fn try_run_compiled_free_function(
        &mut self,
        fd: &crate::ast::decl::FunctionDeclaration,
        pkg_scope: Option<&str>,
    ) -> Option<Value> {
        if !self.hoisted_static_subroutines.is_empty()
            && self.hoisted_static_subroutines.contains(&fd.name.name.name)
        {
            return None;
        }
        let cname = self.free_fn_class_name(pkg_scope);
        let ret_is_string = Self::is_string_data_type(&fd.return_type);
        self.try_run_compiled_method(
            0,
            &cname,
            &fd.name.name.name,
            &fd.ports,
            &fd.items,
            Some(&fd.name.name.name),
            ret_is_string,
            CompiledBodyKind::Function(fd),
        )
    }

    pub(super) fn try_run_compiled_method(
        &mut self,
        handle: usize,
        cname: &str,
        method_name: &str,
        ports: &[crate::ast::decl::FunctionPort],
        body: &[crate::ast::stmt::Statement],
        fn_ret_name: Option<&str>,
        ret_is_string: bool,
        kind: CompiledBodyKind<'_>,
    ) -> Option<Value> {
        use super::bytecode::BytecodeCompiler;
        use crate::ast::types::PortDirection;
        // COLD-PATH FIRST (class-perf P0 + adaptive tiering): the tier
        // counter used to run before the skip cache — every class-function
        // call paid an inline FNV hash + HashMap probe even for the ~90% of
        // UVM methods that are skip-cached (task-form, string/void/non-scalar
        // return, ref/output formal, ...) and can NEVER run as bytecode.
        // Adaptive tiering inverts that: we intern (cid, mid) and consult the
        // skip cache FIRST (cheap u32 interner lookups + a HashSet probe, no
        // allocation on hit), so a permanently-interpreted method returns
        // before the tier bookkeeping ever runs. Only methods that are
        // plausibly compilable (not skip-cached today) reach the tier
        // counter — a much smaller, genuinely-hot set, so the counter is a
        // no-op for the long tail of cold/never-compiling methods.
        //
        // The tier key is the interned (cid, mid) pair rather than an inline
        // FNV hash: cid/mid are needed anyway for the skip + block caches,
        // so interning once removes a redundant hash-and-probe per call AND
        // drops the (harmless but present) collision approximation.
        let ids = &mut self.compiled_class_method_ids;
        let cid = match ids.get(cname) {
            Some(&id) => id,
            None => {
                let next = ids.len() as u32;
                *ids.entry(cname.to_string()).or_insert(next)
            }
        };
        let mid = match ids.get(method_name) {
            Some(&id) => id,
            None => {
                let next = ids.len() as u32;
                *ids.entry(method_name.to_string()).or_insert(next)
            }
        };
        // Phase 2: specialization id. A parameterized class's methods get a
        // per-spec plan/skip/fast key so `Cache#(string)::getv` is never
        // confused with `Cache#(int)::getv`. Non-parameterized methods always
        // yield 0 (untouched behavior).
        let spec_id = self.instance_spec_id(handle);
        // Fast decision cache: a (class, method) already decided it can never
        // run as bytecode is looked up BEFORE re-running the whole gate +
        // type-resolution (and, with adaptive tiering, before the tier
        // counter). Every decision memoized here is instance-independent
        // (scalar_formal_integral reads module scope), so a skip can never
        // flip with the instance. Without this, the THOUSANDS of negative-
        // cached UVM calls re-paid the full re-validation per call (the
        // dominant cost on the hier-gen perf workload even though the block
        // cache hits).
        if self.compiled_method_skip.contains(&(cid, mid, spec_id)) {
            return None;
        }
        // Adaptive tier counter: only reached by methods that passed the skip
        // cache, so it counts toward compilation for exactly the set that
        // might benefit. A (cid, mid) that crossed compiles (and thereafter
        // misses HM catch below is zero-cost); one that never crosses pays
        // the counter only while below the threshold, and the count is
        // keyed by u32 ids (the same key the block cache uses).
        // LOOP-HEAT BOOST (measured heat / pass C): instead of treating any
        // wait-free loop body as hot on its FIRST call — which compiles
        // short-loop UVM bodies the compiler never pays back (the maintainer's
        // measured reference-workload cost) — the boost keys off ACTUAL loop
        // heat: while a loop-candidate method (see `is_loop_candidate`) runs
        // on the AST interpreter, its loop back-edges accumulate into
        // `compiled_method_loop_heat`. A body becomes loop-hot only once
        // measured iterations reach the tier threshold. The per-call check is
        // a single HashMap lookup (same cost as the call counter) — no
        // structural statement walk on the hot path. `is_loop_candidate`
        // (pass B, memoized) confines heat-tracking to genuinely compilable
        // wait-free loop bodies. Opt-in via method_boost_enabled() (an
        // explicitly-set `XEZIM_METHOD_TIER`); the default remains the pure
        // call-count tier (flat within ±0.1% on the reference workloads).
        let tier = method_tier_threshold();
        let loop_hot = method_boost_enabled()
            && self
                .compiled_method_loop_heat
                .get(&(cid, mid, spec_id))
                .copied()
                .unwrap_or(0)
                >= tier as u64;
        if tier > 0 && !loop_hot {
            let calls = self
                .compiled_method_call_counts
                .entry((cid, mid, spec_id))
                .or_insert(0);
            if *calls < tier {
                *calls += 1;
                return None;
            }
        }
        // Step 7b: PREBOUND gate. Everything between the skip cache and the
        // block cache is a pure function of the class declaration + module
        // scope, so it runs once per (cid, mid) at plan creation. The only
        // per-instance input — the result width of a param-bounded packed
        // return — is flagged in the plan and re-resolved below.
        let pre = if let Some(p) = self.compiled_method_plans.get(&(cid, mid, spec_id)) {
            p.clone()
        } else {
            // Census aid: XEZIM_FALLBACK_SITES=1 also prints PLAN-phase
            // declines (these never reach BytecodeCompiler's tracer), so a
            // hot decliner can be attributed by method name.
            let trace_decline = |reason: &str| -> bool {
                method_trace_enabled() && {
                    eprintln!(
                        "[FALLBACK] plan-decline reason={reason} scope={cname}.{method_name}"
                    );
                    true
                }
            };
            // §23.8: a class declared inside a non-top module resolves its
            // bare names in the instance each call runs for; a compiled plan
            // binds them once, to the top-level namesake.
            if self
                .module
                .classes
                .get(cname)
                .and_then(|cd| cd.declaring_module.as_deref())
                .is_some_and(|m| m != self.module.name)
            {
                trace_decline("module-class");
                self.compiled_method_skip.insert((cid, mid, spec_id));
                return None;
            }
            // §7.4.1, §11.5.1: a formal or local declared with a packed range
            // that is not `[N-1:0]` (`input logic [12:1] a`) is selected by
            // its labels, which the compiled selects do not rebase: `a[i]`
            // read bit i, so a hot function read the wrong bit or x (#650).
            if self.declares_offset_packed_range(ports, body) {
                trace_decline("offset-packed-range");
                self.compiled_method_skip.insert((cid, mid, spec_id));
                return None;
            }
            // A wait-free TASK compiles exactly like a `function void`: the
            // compiler bails on any timing control (allow_waits=false), so a
            // task whose body compiles is purely sequential and the
            // interpreter's scheduling machinery is never needed. Tasks
            // carrying waits simply fail to compile here and keep the AST
            // path. Synthesize a void `FunctionDeclaration` so every
            // downstream decision (result-width, writeback, plan, compiler)
            // sees the ordinary function shape.
            let task_fd_storage;
            let _f = match kind {
                CompiledBodyKind::Function(f) => f,
                CompiledBodyKind::Task(t) => {
                    // A compiled task runs synchronously inside `exec_insns`;
                    // if its body (or any transitively reached callee) can
                    // park the process, the suspension can never resume into
                    // the compiled frame. Decline on the real-suspension
                    // analysis — NOT the tier counter, which measures call
                    // frequency and says nothing about this.
                    if self.method_body_can_suspend(cname, &t.ports, &t.items) {
                        trace_decline("task-suspends");
                        self.compiled_method_skip.insert((cid, mid, spec_id));
                        return None;
                    }
                    task_fd_storage = crate::ast::decl::FunctionDeclaration {
                        default_args: false,
                        lifetime: t.lifetime.clone(),
                        specifier: t.specifier.clone(),
                        return_type: crate::ast::types::DataType::Void(crate::ast::Span::dummy()),
                        name: t.name.clone(),
                        ports: t.ports.clone(),
                        items: Vec::new(),
                        endlabel: t.endlabel.clone(),
                        strict_body_ports: Vec::new(),
                        span: t.span,
                    };
                    &task_fd_storage
                }
                CompiledBodyKind::Other => {
                    trace_decline("kind");
                    self.compiled_method_skip.insert((cid, mid, spec_id));
                    return None;
                }
            };
            // Step 8: a STRING return compiles like a class handle — the
            // result Value is a byte-vector register passed through untouched
            // (width 0, no signedness). The interpreter seeds the implicit
            // return cell to `""` and returns the local's bytes verbatim;
            // the register does the same. (The signal-store fallback below
            // only services interpreter concat writes — the block computes
            // the identical bytes in-register.)
            let is_string_result = ret_is_string;
            // Tasks and plain `function void` bodies have no implicit result
            // cell; the empty name gives the plan/compiler a void shape
            // (never a live local name).
            let rname = fn_ret_name.map(str::to_string).unwrap_or_default(); // class-perf row 2: NON-INPUT formals (output / inout / ref) are
            // admitted as plain scalars (integral / string / enum / class
            // handle, `dimensions.is_empty()`): the interpreter implements
            // exactly these as copy-in/copy-out through `output_bindings`
            // (bind the actual's entry value, `assign_value` the frame local
            // back on return), and the compiled path mirrors that — seed the
            // register from the frame, copy it back at exit so the epilogue
            // write-back runs unchanged. Identity-bound aggregates (ref
            // queues/arrays/structs, and anything with dimensions) keep
            // declining: their alias semantics cannot be mirrored by a
            // register.
            let mut writeback_formals: Vec<String> = Vec::new();
            for port in ports {
                if port.direction == PortDirection::Input {
                    continue;
                }
                let shape_ok = port.dimensions.is_empty()
                    && (self.typeref_names_class(&port.data_type)
                        || Self::is_string_data_type(&port.data_type)
                        || self.enum_type_info(&port.data_type).is_some()
                        || self.scalar_formal_integral(&port.data_type).is_some());
                if !shape_ok {
                    trace_decline("noninput_formal_shape");
                    self.compiled_method_skip.insert((cid, mid, spec_id));
                    return None;
                }
                writeback_formals.push(port.name.name.clone());
            }
            // Step 9e: a VOID function (`function void f();` — DataType::
            // Void) or a CONSTRUCTOR (`function new(...)` — parsed as
            // Implicit with no dimensions) has no observable return VALUE:
            // the interpreter's epilogue reads back the implicit return
            // cell's SEED (`return_value.or(implicit)`) and callers discard
            // it. Compile with a passthrough result cell: width 0 (keep
            // source, like a string return), no resize, no signedness
            // stamp; the runtime seeds the register from the frame's
            // implicit local so a never-written cell reads back exactly the
            // AST value, and an (illegal-SV) `f = x` write even mirrors
            // last-write-wins.
            let is_void_result = matches!(&_f.return_type, crate::ast::types::DataType::Void(_))
                || (_f.name.name.name == "new"
                    && matches!(
                        &_f.return_type,
                        crate::ast::types::DataType::Implicit { dimensions, .. }
                            if dimensions.is_empty()
                    ));
            // Determine the return "kind". Either a SCALAR-INTEGRAL result (the
            // original pilot surface), a CLASS-HANDLE result (a TypeReference
            // naming a class: the value is a heap index passed through untouched),
            // or a bail. A class-handle return must NOT be resized or signedness-
            // stamped — the interpreter never touches a non-`plainly_integral`
            // (Typeref) return, so a handle must round-trip unchanged.
            let is_class_result = self.typeref_names_class(&_f.return_type);
            // class-perf collection-return: the return type resolves to a
            // queue / unsized dynamic array. The body returns via the
            // `pending_ret_collection` side channel (the compiler admits
            // ONLY collection literals); the result cell is a width-0
            // passthrough (like a class handle).
            let is_collection_result = !is_class_result
                && !is_string_result
                && !is_void_result
                && self.fn_returns_collection(&_f.return_type);
            let (static_result_width, result_signed) = if is_string_result {
                (0u32, false)
            } else if is_class_result {
                // class handle: width meaningless; pass through untouched.
                (0u32, false)
            } else if is_void_result {
                // Step 9e: void / ctor — passthrough result cell.
                (0u32, false)
            } else if is_collection_result {
                // Collection return: the VALUE leaves via the side channel;
                // the result register is never consumed by the caller.
                (0u32, false)
            } else if let Some(pair) = self.scalar_formal_integral(&_f.return_type) {
                pair
            } else if let Some(pair) = self.enum_type_info(&_f.return_type) {
                // Step 6: an enum RETURN is integral — width of the enum's base
                // type. The Return handler resizes to it like any scalar.
                pair
            } else {
                trace_decline("ret_type");
                self.compiled_method_skip.insert((cid, mid, spec_id));
                return None;
            };
            // Build the formal list: (name, width) plus a class-handle set. A
            // class-typed formal must be positively identified as a class before
            // we treat `.member` on it as heap access; any doubt keeps the whole
            // method on the AST path (safe).
            let mut formals: Vec<(String, u32)> = Vec::with_capacity(ports.len());
            let mut class_formals: HashSet<String> = Default::default();
            let mut string_formals: HashSet<String> = Default::default();
            for port in ports {
                // Forward typedefs (`typedef class uvm_component;`) make
                // resolve_dt_ref erase the class identity, so the OLD check saw
                // every UVM class formal as non-class and skip-cached the method
                // as non-scalar. The walk checks the class table first.
                let is_class = self.typeref_names_class(&port.data_type);
                // Step 8: a STRING formal binds the actual UNRESIZED in the
                // interpreter (no branch of the binding matches Simple{String})
                // — seed width 0 (keep source) and record the name so the
                // compiler bails on every non-pass-through use.
                let is_string = Self::is_string_data_type(&port.data_type);
                if is_string {
                    string_formals.insert(port.name.name.clone());
                }
                let w = if is_class {
                    0 // class handle formal; width irrelevant (heap member access)
                } else if is_string {
                    0 // string formal; raw byte-vector actual, keep source width
                } else if self.enum_type_info(&port.data_type).is_some() {
                    // An ENUM formal (uvm_severity-style typedef). The
                    // interpreter's TypeReference binding branch never
                    // width-adapts one (`scalar_formal_integral` fails, the
                    // actual keeps the caller's width; only signedness may
                    // be stamped via type_is_signed_concrete), and it
                    // records the enum typedef so runtime name/%p lookups
                    // work — all of that happens in the binding loop BEFORE
                    // this method runs, so the register just needs to keep
                    // the frame-bound value: seed width 0 (keep source),
                    // exactly like a string formal but scalar-typed.
                    0
                } else {
                    // Scalar-integral GATE (bail to AST if not): the type must be
                    // a compile-able integer vector/atom. (String formals are
                    // handled above — allowed, not skipped.)
                    use crate::ast::types::DataType as DT;
                    let Some(pw) = self.scalar_formal_integral(&port.data_type).map(|(w, _)| w)
                    else {
                        // class-perf collection-return: a TYPE-PARAM formal
                        // (`KEY_T key` — an unresolvable TypeReference: type
                        // params are never hoisted into `typedef_types`) of a
                        // collection-returning method. The interpreter's
                        // binding loop resolves it to neither integral nor
                        // class, stamps signedness via
                        // `type_is_signed_concrete` and keeps the actual's
                        // width — admit it as width-0 keep-source, exactly
                        // that parity. (Resolving the module-scope default
                        // would truncate a string actual.)
                        if is_collection_result
                            && matches!(port.data_type, DT::TypeReference { .. })
                        {
                            formals.push((port.name.name.clone(), 0));
                            continue;
                        }
                        // real/enum/collection formal: AST — memoize the skip
                        // (the decision is instance-independent: the declared
                        // formal type cannot change per instance).
                        trace_decline("formal_type");
                        self.compiled_method_skip.insert((cid, mid, spec_id));
                        return None;
                    };
                    // Seed width MIRRORS the interpreter's formal-binding branch
                    // (exec_method_in_class_hierarchy): a TypeReference / IntegerAtom
                    // / literal-packed formal resizes the actual to its declared
                    // width; a PARAMETERIZED-packed formal (`logic [W-1:0]` with a
                    // class type-param bound, non-literal) keeps the CALLER's
                    // width — `scalar_formal_integral` would wrongly read the param
                    // from module scope and truncate the seed. Width 0 = keep the
                    // source width, exactly like the AST path (which only stamps
                    // signedness there).
                    let keep_source = !matches!(port.data_type, DT::TypeReference { .. })
                        && !matches!(port.data_type, DT::IntegerAtom { .. })
                        && !Self::packed_dims_are_literal(&port.data_type);
                    if keep_source { 0 } else { pw }
                };
                if is_class {
                    class_formals.insert(port.name.name.clone());
                }
                formals.push((port.name.name.clone(), w));
            }
            // Parameterized result width must resolve to the actual instance.
            // Only a param-bounded packed return (`logic [W-1:0]` with param W,
            // non-literal bounds) needs the INSTANCE's param scope — flagged so
            // the per-call resolution below runs just for those. Integer atoms,
            // literal-bounded packed vectors and class TypeReferences are fixed.
            // String returns pass through untouched (width 0), so they are
            // never param-resolved either.
            let param_able_result = !is_class_result
                && !is_string_result
                && !is_void_result
                && !is_collection_result
                && !matches!(
                    &_f.return_type,
                    crate::ast::types::DataType::IntegerAtom { .. }
                )
                && !Self::packed_dims_are_literal(&_f.return_type);
            let pre = std::rc::Rc::new(super::bytecode::PreboundCompiledMethod {
                formal_widths: std::rc::Rc::new(formals.iter().map(|f| f.1).collect()),
                class_formals: std::rc::Rc::new(class_formals),
                return_type: std::rc::Rc::new(_f.return_type.clone()),
                fn_ret_name: rname.clone(),
                is_class_result,
                is_string_result,
                is_void_result,
                is_collection_result,
                string_formals: std::rc::Rc::new(string_formals),
                coll_members: std::rc::Rc::new(self.class_coll_member_names(cname)),
                static_coll_members: std::rc::Rc::new(self.class_static_coll_member_names(cname)),
                coll_elem_members: std::rc::Rc::new(self.class_coll_elem_member_names(cname)),
                coll_elem_packed_dims: std::rc::Rc::new(self.class_coll_elem_packed_dims(cname)),
                writeback_formals: std::rc::Rc::new(writeback_formals),
                static_result_width,
                result_signed,
                param_able_result,
                formals,
            });
            self.compiled_method_plans
                .insert((cid, mid, spec_id), pre.clone());
            pre
        };
        // Effective result width: instance-resolved for param-bounded packed
        // returns, else the static width from the plan.
        let result_width = if pre.param_able_result {
            let scope = self.instance_param_scope(handle);
            let w = resolve_type_width(&pre.return_type, Some(&scope), Some(&self.module.typedefs));
            if w > 0 { w } else { pre.static_result_width }
        } else {
            pre.static_result_width
        };

        // Intern the (class, method) identity and build the width signature —
        // the cache key. The specialization distinguishes local types even
        // with identical formal/result widths; interned ids + the plan's Rc'd
        // width Vec (an Rc clone per call, no allocation) so repeated calls to
        // a hot method lower its body ONCE and reuse the block.
        let key = (cid, mid, spec_id, pre.formal_widths.clone(), result_width);
        let entry = match self.compiled_method_block_cache.get(&key) {
            Some(super::bytecode::CompiledMethodOutcome::Block(e)) => e.clone(),
            // Negative-cached as non-compilable: skip straight to AST.
            Some(super::bytecode::CompiledMethodOutcome::Nil) => {
                return None;
            }
            None => 'disk: {
                // Persistent method cache (XEZIM_METHOD_CACHE): probe disk
                // BEFORE the admission walks and compilation — a warm run
                // must pay neither. The disk key is content-based (class
                // chain + widths + salt), not the elaboration-local memory
                // key. A miss falls through to compile-and-store below; a
                // garbage/corrupt payload is just a miss.
                let disk_key = method_cache_dir().and_then(|_| {
                    self.persisted_method_key(
                        handle,
                        cname,
                        method_name,
                        &pre.formal_widths,
                        result_width,
                    )
                });
                if let Some(k) = disk_key {
                    match self.load_persisted_method(k) {
                        Some(super::bytecode::PersistedMethodOutcome::Block {
                            block,
                            this_reg,
                            result_reg,
                        }) => {
                            let entry = super::bytecode::CompiledMethodEntry {
                                block: std::rc::Rc::new(block),
                                this_reg,
                                result_reg,
                            };
                            let rc = std::rc::Rc::new(entry);
                            self.compiled_method_block_cache.insert(
                                key,
                                super::bytecode::CompiledMethodOutcome::Block(rc.clone()),
                            );
                            break 'disk rc;
                        }
                        Some(super::bytecode::PersistedMethodOutcome::Nil) => {
                            self.compiled_method_block_cache
                                .insert(key, super::bytecode::CompiledMethodOutcome::Nil);
                            return None;
                        }
                        None => {} // miss: compile below
                    }
                }
                // Compile all-or-nothing. On any lowering failure, none the
                // key (so it is not retried per call) and fall to AST.
                let mut shadow_names = self.class_scoping_shadow_names(cname);
                // Members the heap cannot faithfully round-trip (arrays,
                // queues, assoc, structs, strings, reals) join the shadow-bail
                // set: any reference bails to AST instead of reading garbage.
                shadow_names.extend(self.class_nonloadable_member_names(cname));
                let bare_members = self.bare_class_member_names(cname);
                // Dispatch-handle members: the class-typed subset of the
                // bare set (the CallMethod gate requires BOTH — an integral
                // member like uvm_reg_field's `value` must not become a
                // dispatch base).
                let member_classes = self.class_typed_member_names(cname);
                // Dotted member accesses faithful by runtime bare key (see
                // the fn doc): statics/nonloadable/enclosing excluded.
                let member_safe = self.class_member_access_safe_names(cname);
                // Step 9a: string members — bare/dotted reads lower to
                // LoadClassMember (byte-vector Value round-trips verbatim).
                let string_members = self.class_string_member_names(cname);
                // Step 6: body VarDecls of CLASS type hold heap handles in
                // registers; the compiler needs their names (it cannot see
                // the class table).
                let class_locals = self.class_typed_local_names(body);
                // class-perf P2 (typed handle roots): declared class per
                // handle FORMAL and class-typed body LOCAL, so a dispatch
                // chain rooted at one (`sched.m_parent.get_phase_type()` —
                // uvm_phase::get_schedule) resolves STATICALLY instead of
                // refusing as an untyped root. Per-method and instance-
                // independent (declared types only), like every plan input.
                let mut handle_local_types = {
                    let mut m: HashMap<String, String> = HashMap::default();
                    for port in ports {
                        if let Some(cls) = self.typeref_class_name(&port.data_type) {
                            m.insert(port.name.name.clone(), cls);
                        }
                    }
                    for (n, t) in self.class_typed_local_types(body) {
                        m.entry(n).or_insert(t);
                    }
                    m
                };
                // Step 9e: method-name admission set for bare this-bounded
                // calls.
                let method_name_set = self.class_method_name_set(cname);
                // LRM 11.6/11.8.2: declared input widths of every callee
                // visible from this class, so the CallMethod lowerings can
                // give each ACTUAL its formal's context width.
                let method_formal_widths = self.class_method_formal_widths(cname);
                // Step 9f: parent-chain method names for `super.m` admission.
                let super_method_set = self.super_method_name_set(cname);
                // Step 9f: baked dispatch inputs — the defining class's
                // parent (super.m start class) and the §8.20 non-virtual
                // bare-call targets (first defining class per method name,
                // exactly the AST fallback's nonvirtual_target_class;
                // statics keep the virtual dispatch they have today).
                let class_parent = self
                    .module
                    .classes
                    .get(cname)
                    .and_then(|cd| cd.extends.clone());
                // Step 9g: static-member admission. Walk the parent chain
                // collecting `static_properties` (which ALSO contains class
                // localparams) and `param_defaults` (localparam names). A
                // name that exists as an INSTANCE property anywhere in the
                // chain, or a localparam (constant), is NOT admitted for
                // writes: the interpreter's own resolution for those
                // shapes is member-first / read-only, and the compiled
                // runtime walk starts at the baked defining class (= the
                // lexical class of every frame that runs this block).
                let mut static_members: HashSet<String> = HashSet::default();
                let mut const_names: HashSet<String> = HashSet::default();
                let mut instance_props: HashSet<String> = HashSet::default();
                let mut spec_scoped: HashSet<String> = HashSet::default();
                let mut vif_statics: HashSet<String> = HashSet::default();
                {
                    let mut cur = Some(cname.to_string());
                    let mut guard = 0;
                    while let Some(cn) = cur {
                        guard += 1;
                        if guard > 64 {
                            break;
                        }
                        if let Some(cd) = self.module.classes.get(&cn) {
                            let class_staticish: HashSet<String> = cd
                                .static_properties
                                .iter()
                                .chain(cd.param_defaults.iter().map(|(pn, _)| pn))
                                .cloned()
                                .collect();
                            for sp in cd.static_properties.iter() {
                                // §25.9: a static holding a virtual interface
                                // (declared so, or through a type parameter
                                // that may bind one) binds by name — the
                                // interpreter's `static_vif_bindings`.
                                if Self::static_is_vif_capable(&cd, sp) {
                                    vif_statics.insert(sp.clone());
                                } else {
                                    static_members.insert(sp.clone());
                                }
                            }
                            for (pn, _) in cd.param_defaults.iter() {
                                const_names.insert(pn.clone());
                            }
                            // `property_types` holds EVERY declared
                            // property (static ones too) — only names that
                            // are neither static nor localparam are
                            // instance properties.
                            for pn in cd.property_types.keys() {
                                if !class_staticish.contains(pn) {
                                    instance_props.insert(pn.clone());
                                }
                            }
                            // Step 9h: VALUE parameters of a PARAMETERIZED
                            // chain class are specialization-scoped at run
                            // time (ensure_spec_statics seeds them per sig);
                            // a baked LoadClassStatic(class, name) would
                            // read the default spec. Collect them here;
                            // applied below.
                            if self.class_is_parameterized(&cn) {
                                spec_scoped.extend(self.class_value_param_names(&cn));
                            }
                            cur = cd.extends.clone();
                        } else {
                            break;
                        }
                    }
                    for ip in instance_props.iter() {
                        static_members.remove(ip);
                        const_names.remove(ip);
                    }
                    // A localparam lives in BOTH sets after the union
                    // walk; it is READ-ONLY — drop it from the writable
                    // set (stores to it stay AST).
                    for lp in const_names.iter() {
                        static_members.remove(lp);
                    }
                }
                for vpn in spec_scoped.iter() {
                    static_members.remove(vpn);
                    const_names.remove(vpn);
                    shadow_names.insert(vpn.clone());
                }
                // A method naming a vif-capable static stays interpreted.
                for vs in vif_statics.iter() {
                    if !static_members.contains(vs) && !instance_props.contains(vs) {
                        shadow_names.insert(vs.clone());
                    }
                }
                let mut bare_targets: HashMap<String, String> = HashMap::default();
                for m in method_name_set.iter() {
                    if self.is_static_method(cname, m) {
                        continue;
                    }
                    if let Some(t) = self.nonvirtual_target_class(cname, m) {
                        bare_targets.insert(m.clone(), t);
                    }
                }
                // Step 9e: typed handle-chain admission inputs — the
                // method's own class and every class's handle-member types.
                // class-perf P2: flat `pkg::name` -> (value,width) snapshot
                // of the package enum table (report-macro constants).
                let pkg_enum_consts: HashMap<String, (u64, u32)> = self
                    .module
                    .package_enum_members
                    .iter()
                    .flat_map(|(pkg, members)| {
                        members
                            .iter()
                            .filter(|(_, (_, w))| *w <= 64)
                            .map(move |(m, (v, w))| (format!("{}::{}", pkg, m), (*v, *w)))
                    })
                    .collect();
                // class-perf P2: Rc snapshot of the free-function table
                // (lazy, shared across every method compile).
                let free_fn_table = self.free_fn_table.clone().unwrap_or_else(|| {
                    let rc = std::rc::Rc::new(self.module.functions.as_map().clone());
                    self.free_fn_table = Some(rc.clone());
                    rc
                });
                let handle_member_types = self.class_handle_member_types();
                // class-perf P2: FOREACH loop variables over a collection
                // MEMBER whose element is a class handle (`foreach
                // (m_successors[succ])` — uvm_phase::m_find_successor_by_name)
                // get the element class, so `succ.get_schedule()` chains
                // resolve statically. Uses the same element-class entries
                // the Step 9e map feeds; collections over LOCALS stay
                // untyped (declined). Walk at any nesting depth.
                {
                    let member_elem_classes: &HashMap<String, String> =
                        handle_member_types.get(cname).unwrap_or_else(|| {
                            static EMPTY: std::sync::OnceLock<HashMap<String, String>> =
                                std::sync::OnceLock::new();
                            EMPTY.get_or_init(HashMap::default)
                        });
                    fn member_assoc_key_class(
                        sim: &Simulator,
                        cname: &str,
                        member: &str,
                    ) -> Option<String> {
                        // A member declared with a TYPEDEF'd assoc type
                        // (`typedef bit edges_t[uvm_phase];` + `edges_t
                        // m_successors;`): the foreach loop var iterates the
                        // KEYS, whose class the typedef's unpacked dim
                        // records. Walk the chain leaf-first.
                        let mut cur = Some(cname.to_string());
                        let mut seen = HashSet::default();
                        while let Some(cn) = cur {
                            if !seen.insert(cn.clone()) {
                                break;
                            }
                            let cd = sim.module.classes.get(&cn)?;
                            if let Some(dt) = cd.property_types.get(member)
                                && let crate::ast::types::DataType::TypeReference {
                                    name: tn, ..
                                } = dt
                                && let Some(dims) =
                                    sim.module.typedef_unpacked_dims.get(&tn.name.name)
                                && let [
                                    crate::ast::types::UnpackedDimension::Associative {
                                        data_type: Some(key),
                                        ..
                                    },
                                ] = dims.as_slice()
                            {
                                return sim.typeref_class_name(key);
                            }
                            cur = cd.extends.clone();
                        }
                        None
                    }
                    fn walk_foreach(
                        st: &Statement,
                        member_elem_classes: &HashMap<String, String>,
                        m: &mut HashMap<String, String>,
                        sim: &Simulator,
                        cname: &str,
                    ) {
                        use crate::ast::stmt::StatementKind as SK;
                        if let SK::Foreach { array, vars, .. } = &st.kind {
                            if let ExprKind::Ident(h) = &array.kind
                                && h.root.is_none()
                                && h.path.len() == 1
                                && h.path[0].selects.is_empty()
                            {
                                let elem_or_key = member_elem_classes
                                    .get(h.path[0].name.name.as_str())
                                    .cloned()
                                    .or_else(|| {
                                        member_assoc_key_class(sim, cname, &h.path[0].name.name)
                                    });
                                if let Some(cls) = elem_or_key
                                    && let Some(Some(v)) = vars.first()
                                {
                                    m.entry(v.name.clone()).or_insert_with(|| cls);
                                }
                            }
                        }
                        for sub in Simulator::sub_stmts(st) {
                            walk_foreach(sub, member_elem_classes, m, sim, cname);
                        }
                    }
                    for st in body {
                        walk_foreach(
                            st,
                            member_elem_classes,
                            &mut handle_local_types,
                            self,
                            cname,
                        );
                    }
                }
                // class-perf P2: unanimous string-returning method names
                // (string concat operand typing).
                let string_method_names = self.class_unanimous_string_methods();
                // Step 9g: DECLARED class types of bare lvalues — locals
                // (from the method body's own VarDecls), static members,
                // and instance members of the enclosing chain — filtered
                // to NON-parameterized classes (the ctor's specialization
                // is runtime state). Drives `lhs = new(...)` lowering.
                let mut new_type_map: HashMap<String, String> = HashMap::default();
                for (m, t) in handle_member_types
                    .get(cname)
                    .map(|m| m.iter())
                    .unwrap_or_default()
                {
                    if !self.class_is_parameterized(t) {
                        new_type_map.insert(m.clone(), t.clone());
                    }
                }
                {
                    let mut cur = Some(cname.to_string());
                    let mut guard = 0;
                    while let Some(cn) = cur {
                        guard += 1;
                        if guard > 64 {
                            break;
                        }
                        if let Some(cd) = self.module.classes.get(&cn) {
                            for sp in cd.static_properties.iter() {
                                if let Some(dt) = cd.property_types.get(sp)
                                    && let Some(t) = self.typeref_class_name(dt)
                                    && !self.class_is_parameterized(&t)
                                {
                                    new_type_map.insert(sp.clone(), t);
                                }
                            }
                            cur = cd.extends.clone();
                        } else {
                            break;
                        }
                    }
                }
                // Step 9g locals: VarDecls at ANY nesting depth (a
                // `ctxt = new;` inside `else begin…end` — uvm_objection::
                // m_drop — has the same declared class as a top-level one).
                for (n, t) in self.class_typed_local_types(body) {
                    if !self.class_is_parameterized(&t) {
                        new_type_map.entry(n).or_insert(t);
                    }
                }
                // Step 9h: receiver names admitted for CLASS-SCOPE static
                // calls `X.m(args)` — every elaborated class name, the
                // declaring chain's typedef aliases and type parameters, and
                // module typedef aliases whose target is a TypeReference
                // (class-ish). Anything that could be a VARIABLE of the
                // frame is excluded (chain members/shadows, signals); the
                // compiler arm additionally excludes locals. Resolution
                // itself stays runtime-owned.
                let mut scope_static_receivers: HashSet<String> =
                    self.module.classes.keys().cloned().collect();
                {
                    let mut cur = Some(cname.to_string());
                    let mut guard = 0;
                    while let Some(cn) = cur {
                        guard += 1;
                        if guard > 64 {
                            break;
                        }
                        if let Some(cd) = self.module.classes.get(&cn) {
                            scope_static_receivers.extend(cd.typedef_targets.keys().cloned());
                            scope_static_receivers.extend(cd.type_param_names.iter().cloned());
                            cur = cd.extends.clone();
                        } else {
                            break;
                        }
                    }
                }
                // A free function's body (`__free_fn__<pkg>`, no class chain
                // above) admits only the elaborated class names: an alias
                // resolves against the caller's stale spec there (see the
                // compiler's Step 9h arm).
                if !cname.starts_with("__free_fn__") {
                    for (tn, dt) in self.module.typedef_types.iter() {
                        if matches!(dt, crate::ast::types::DataType::TypeReference { .. }) {
                            scope_static_receivers.insert(tn.clone());
                        }
                    }
                }
                for nm in shadow_names.iter().chain(bare_members.iter()) {
                    scope_static_receivers.remove(nm);
                }
                scope_static_receivers.retain(|n| !self.signal_name_to_id.contains_key(n.as_str()));
                let admission = self.compiled_method_admission.clone().unwrap_or_else(|| {
                    let mut facts = super::bytecode::MethodAdmission::default();
                    for (class, declaration) in &self.module.classes {
                        let mut fields = self.class_member_access_safe_names(class);
                        fields.extend(self.class_string_member_names(class));
                        let widths = fields
                            .iter()
                            .filter_map(|f| match self.class_prop_width_fixed(class, f) {
                                Some(Some(w)) if w > 0 => Some((f.clone(), w)),
                                _ => None,
                            })
                            .collect();
                        facts.instance_field_widths.insert(class.clone(), widths);
                        facts.instance_fields.insert(class.clone(), fields);
                        // Direct superclass (spec args stripped): the
                        // writeback-hierarchy walk in
                        // `emit_arg_slot_writeback` climbs this chain.
                        if let Some(parent) = self
                            .get_class_def(class)
                            .and_then(|cd| cd.extends.as_deref())
                        {
                            let parent = parent.split('#').next().unwrap_or(parent).to_string();
                            facts.class_parents.insert(class.clone(), parent);
                        }
                        for method in declaration.methods.values() {
                            use crate::ast::decl::ClassMethodKind as Kind;
                            let (name, ports) = match &method.kind {
                                Kind::Function(f) | Kind::Extern(f) | Kind::PureVirtual(f) => {
                                    (&f.name.name.name, &f.ports)
                                }
                                Kind::Task(t) => (&t.name.name.name, &t.ports),
                            };
                            for (index, port) in ports.iter().enumerate() {
                                if port.direction != PortDirection::Input {
                                    facts
                                        .writeback_positions
                                        .entry((class.clone(), name.clone()))
                                        .or_default()
                                        .insert(index);
                                }
                            }
                        }
                    }
                    // §8.14 / §8.10: a property some subclass redeclares is a
                    // separate variable, stored under `<Class>::p` in that
                    // subclass's objects. A read or write through a handle of
                    // an ancestor's static type must pick the copy per object
                    // (`shadowed_member_rewrite`), so the compiled member
                    // paths decline it for every ancestor of a redeclarer.
                    for cd in self.module.classes.values() {
                        let mut cur = cd.extends.as_deref();
                        let mut guard = 0;
                        while let Some(b) = cur
                            && guard < 64
                        {
                            guard += 1;
                            let base = b.split('#').next().unwrap_or(b);
                            let Some(bd) = self
                                .module
                                .classes
                                .get(b)
                                .or_else(|| self.module.classes.get(base))
                            else {
                                break;
                            };
                            for key in [b, base] {
                                if let Some(fields) = facts.instance_fields.get_mut(key) {
                                    for p in cd.properties.keys() {
                                        fields.remove(p);
                                    }
                                }
                            }
                            cur = bd.extends.as_deref();
                        }
                    }
                    // §8.20: non-virtual methods some subclass redeclares —
                    // the only calls whose static binding differs from the
                    // dynamic dispatch CallMethod performs.
                    let mut hidden: HashSet<&str> = HashSet::default();
                    for cd in self.module.classes.values() {
                        if cd.extends.is_none() {
                            continue;
                        }
                        for m in cd.methods.keys() {
                            if m == "new" || hidden.contains(m.as_str()) {
                                continue;
                            }
                            // Redeclared below an ancestor whose topmost
                            // declaration is not `virtual`: a handle of that
                            // ancestor's type binds statically (a `virtual`
                            // added lower down only affects handles below it).
                            let mut top_virtual = false;
                            let mut redeclares = false;
                            let mut cur = cd.extends.as_deref();
                            let mut guard = 0;
                            while let Some(b) = cur
                                && guard < 64
                            {
                                guard += 1;
                                let Some(bd) =
                                    self.module.classes.get(b.split('#').next().unwrap_or(b))
                                else {
                                    break;
                                };
                                if let Some(bm) = bd.methods.get(m) {
                                    redeclares = true;
                                    use crate::ast::decl::ClassQualifier as Q;
                                    top_virtual = bm.qualifiers.contains(&Q::Virtual)
                                        || bm.qualifiers.contains(&Q::Pure);
                                }
                                cur = bd.extends.as_deref();
                            }
                            if redeclares && !top_virtual {
                                hidden.insert(m.as_str());
                            }
                        }
                    }
                    let hidden: Vec<String> = hidden.into_iter().map(str::to_string).collect();
                    if !hidden.is_empty() {
                        let classes: Vec<String> = self.module.classes.keys().cloned().collect();
                        for class in classes {
                            for m in hidden.iter() {
                                if self.is_static_method(&class, m) {
                                    continue;
                                }
                                if let Some(t) = self.nonvirtual_target_class(&class, m) {
                                    facts
                                        .nonvirtual_targets
                                        .entry(class.clone())
                                        .or_default()
                                        .insert(m.clone(), t);
                                }
                            }
                        }
                    }
                    let facts = std::rc::Rc::new(facts);
                    self.compiled_method_admission = Some(facts.clone());
                    facts
                });
                let dpi_fds = self.dpi_import_fd_table();
                let compiled = {
                    // §6.18 / §8.25: retain lexical aliases and resolve type
                    // parameters against this receiver's specialization.
                    let mut local_typedef_types = HashMap::default();
                    let mut unresolved_type_names = HashSet::default();
                    let mut method_enum_constants = HashMap::default();
                    let mut cur = Some(cname.to_string());
                    let mut seen = HashSet::default();
                    while let Some(cn) = cur {
                        if !seen.insert(cn.clone()) {
                            break;
                        }
                        let Some(cd) = self.module.classes.get(&cn) else {
                            break;
                        };
                        for name in &cd.type_param_names {
                            let binding = self
                                .obj_type_param(handle, &cn, name)
                                .and_then(|binding| self.bound_type_dt(&binding));
                            if let Some(dt) = binding {
                                // §8.25: compile locals against this receiver's specialization.
                                local_typedef_types
                                    .entry(name.clone())
                                    .or_insert_with(|| dt.clone());
                                local_typedef_types.insert(format!("{cn}::{name}"), dt);
                            } else {
                                unresolved_type_names.insert(name.clone());
                            }
                        }
                        for (name, dt) in &cd.typedef_targets {
                            if matches!(dt, DataType::Enum(_)) {
                                if let Some(members) = self.module.enum_members.get(name) {
                                    for (member, _) in members {
                                        if !instance_props.contains(member)
                                            && !static_members.contains(member)
                                            && !const_names.contains(member)
                                            && let Some(value) = self.class_enum_member(cname, member)
                                        {
                                            method_enum_constants.entry(member.clone()).or_insert(value);
                                        }
                                    }
                                }
                            }
                            // Keep the declaring class when an inherited alias
                            // is reached by a bare name from a derived method.
                            local_typedef_types.entry(name.clone()).or_insert_with(|| {
                                let span = crate::ast::Span::dummy();
                                DataType::TypeReference {
                                    name: crate::ast::types::TypeName {
                                        scopes: vec![crate::ast::types::TypeScope {
                                            name: crate::ast::Identifier {
                                                name: cn.clone(),
                                                span,
                                            },
                                            type_args: vec![],
                                        }],
                                        name: crate::ast::Identifier {
                                            name: name.clone(),
                                            span,
                                        },
                                        span,
                                    },
                                    dimensions: vec![],
                                    type_args: vec![],
                                    span,
                                }
                            });
                            local_typedef_types.insert(format!("{cn}::{name}"), dt.clone());
                        }
                        cur = cd.extends.clone();
                    }
                    let mut compiler = BytecodeCompiler::new(
                        &self.signal_name_to_id,
                        &self.signal_signed,
                        &self.signal_widths,
                        &self.module.arrays,
                        &self.widths,
                    );
                    compiler.set_typedefs(&self.module.typedefs, &self.module.typedef_elem_widths);
                    compiler.set_typedef_types(&self.module.typedef_types);
                    compiler.method_enum_constants = method_enum_constants;
                    compiler.set_method_formal_types(ports);
                    if let CompiledBodyKind::Function(function) = kind {
                        compiler.set_method_return_type(&function.return_type);
                    }
                    compiler.set_local_typedef_types(local_typedef_types);
                    compiler.set_unresolved_type_names(unresolved_type_names);
                    compiler.method_admission = Some(admission);
                    // issue-301 plan fact: multi-dim packed coll elements.
                    compiler.coll_elem_packed_dims = (*pre.coll_elem_packed_dims).clone();
                    compiler.typedef_coll_dims = self.local_typedef_coll_dims(
                        cname,
                        body,
                        &mut compiler.typedef_coll_strings,
                    );
                    // Formal aliases need the same unpacked-shape exclusion
                    // as body-local aliases before adding scalar register facts.
                    for port in ports {
                        if let DataType::TypeReference { name, .. } = &port.data_type {
                            let dims = super::elaborate::lookup_scoped_type(
                                name,
                                &self.module.typedef_unpacked_dims,
                            )
                            .filter(|dims| !dims.is_empty())
                            .cloned()
                            .or_else(|| {
                                let owner = name
                                    .scopes
                                    .last()
                                    .map_or(cname, |scope| scope.name.name.as_str());
                                Self::typedef_dims_via_tables(&self.module, owner, &name.name.name)
                                    .filter(|dims| !dims.is_empty())
                            });
                            if let Some(dims) = dims {
                                compiler
                                    .typedef_coll_dims
                                    .insert(super::bytecode::type_name_key(name), dims);
                            }
                        }
                    }
                    compiler.set_dpi_import_fds(dpi_fds);
                    if method_trace_enabled() {
                        compiler.scope_hint = Some(format!("{}.{}", cname, method_name));
                    }
                    let body_refs: Vec<&crate::ast::stmt::Statement> = body.iter().collect();
                    let compiler_outcome = compiler.compile_class_method(
                        &pre.formals,
                        &pre.class_formals,
                        &class_locals,
                        &method_name_set,
                        &super_method_set,
                        &class_parent,
                        &bare_targets,
                        &method_formal_widths,
                        &static_members,
                        &const_names,
                        &new_type_map,
                        &scope_static_receivers,
                        &handle_local_types,
                        &string_method_names,
                        &free_fn_table,
                        &pkg_enum_consts,
                        cname,
                        &handle_member_types,
                        &shadow_names,
                        &member_safe,
                        &bare_members,
                        &member_classes,
                        &string_members,
                        &pre.coll_members,
                        &pre.static_coll_members,
                        &pre.coll_elem_members,
                        &pre.string_formals,
                        &pre.writeback_formals,
                        Some((
                            &pre.fn_ret_name,
                            result_width,
                            pre.is_class_result,
                            pre.is_string_result,
                        )),
                        pre.is_collection_result,
                        &body_refs,
                    );
                    compiler_outcome
                };

                // Step 9e: a STATIC method's frame has NO `this` — its
                // register 0 is a null seed. Any instruction that lowers
                // through the receiver (`CallMethod`/`CallCollMethod` on
                // `this`, `Load/StoreClassMember`, `Load/StoreCollElem`)
                // would silently read instance storage or dispatch
                // exec_method_call(handle=0) -> Value::zero —
                // uvm_sequence_library::m_static_check rejected every
                // registered sequence (SEQLIB/NOSEQS, 3623_rand). Statics
                // whose bodies never touch the receiver (pure formals +
                // package globals, e.g. `set_default`) still compile.
                let is_static = self.is_static_method(cname, method_name);
                let Some((block, this_reg, result_reg, _rv_reg)) = compiled else {
                    self.compiled_method_block_cache
                        .insert(key, super::bytecode::CompiledMethodOutcome::Nil);
                    if let Some(k) = disk_key {
                        self.store_persisted_method(
                            k,
                            &super::bytecode::PersistedMethodOutcome::Nil,
                        );
                    }
                    return None;
                };
                let uses_this = |i: &super::bytecode::Insn| {
                    use super::bytecode::Insn as I;
                    match i {
                        I::CallScopedMethod(_, t, ..) => *t as usize == this_reg as usize,
                        I::CallMethod(_, h, ..)
                        | I::CallCollMethod(_, h, ..)
                        | I::LoadClassMember(_, h, _)
                        | I::StoreClassMember(h, ..)
                        | I::LoadCollElem(_, h, ..)
                        | I::StoreCollElem(h, ..) => *h as usize == this_reg as usize,
                        _ => false,
                    }
                };
                // §6.21: a `static` local lives in a register for the whole
                // block and reaches the persistent store only at exit. A call
                // out of the block could re-enter this same subroutine
                // (recursion) and see the stale stored value, which the
                // interpreter's write-through cell never shows; keep such
                // bodies on the interpreter.
                let calls_out = |i: &super::bytecode::Insn| {
                    use super::bytecode::Insn as I;
                    matches!(
                        i,
                        I::CallMethod(..)
                            | I::CallScopedMethod(..)
                            | I::CallFreeFunction(..)
                            | I::CallStaticScoped(..)
                            | I::StmtFallback(..)
                            | I::EvalExprFallback(..)
                    )
                };
                let reentrant_static =
                    !block.static_locals.is_empty() && block.instructions.iter().any(calls_out);
                if (is_static && block.instructions.iter().any(uses_this)) || reentrant_static {
                    self.compiled_method_block_cache
                        .insert(key, super::bytecode::CompiledMethodOutcome::Nil);
                    if let Some(k) = disk_key {
                        self.store_persisted_method(
                            k,
                            &super::bytecode::PersistedMethodOutcome::Nil,
                        );
                    }
                    return None;
                }
                // Persist the compiled block for future runs. Blocks that
                // embed SigIds never persist (signal ids are per-design);
                // everything else in a block is name-keyed and content-
                // stable across designs that hash to the same key. The
                // `result_reg.is_some()` guard mirrors the `?` below: a
                // block this run declines to use is not persisted either,
                // so warm runs make exactly the decisions cold runs do.
                if let (Some(k), Some(rreg)) = (disk_key, result_reg) {
                    if !block.carries_sig_id() {
                        self.store_persisted_method(
                            k,
                            &super::bytecode::PersistedMethodOutcome::Block {
                                block: block.clone(),
                                this_reg,
                                result_reg: rreg,
                            },
                        );
                    }
                }
                let entry = super::bytecode::CompiledMethodEntry {
                    block: std::rc::Rc::new(block),
                    this_reg,
                    result_reg: result_reg?,
                };
                let rc = std::rc::Rc::new(entry);
                self.compiled_method_block_cache.insert(
                    key,
                    super::bytecode::CompiledMethodOutcome::Block(rc.clone()),
                );
                rc
            }
        };
        let block = &entry.block;
        let this_reg = entry.this_reg;
        let result_reg = entry.result_reg;
        if method_trace_enabled() {
            use std::sync::OnceLock;
            static SEEN: OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
                OnceLock::new();
            let key = format!("{}.{}", cname, method_name);
            SEEN.get_or_init(Default::default)
                .lock()
                .unwrap()
                .insert(key.clone())
                .then(|| eprintln!("[FALLBACK] runs-compiled scope={key}"));
        }
        // Swap OUT whatever registers the CALLER's VM is using so this
        // method's execution gets a fresh file and a nested compiled method
        // (a `CallMethod` re-entering exec_method_call above) can never
        // clobber the caller's live registers mid-`exec_insns`. Restored
        // below even if the block defects.
        let saved_vm_regs = std::mem::take(&mut self.vm_regs);
        let saved_foreach_arena = std::mem::take(&mut self.foreach_arena);
        if (block.num_regs as usize) > self.vm_regs.len() {
            self.vm_regs.resize(block.num_regs as usize, Value::zero(1));
        }
        // Seed `this` + formal registers from the bound locals frame (the
        // formals were already bound into the current frame above).
        let frame = self.local_stack.last();
        self.vm_regs[this_reg as usize] = Value::from_u64(handle as u64, 32);
        for (i, port) in ports.iter().enumerate() {
            let val = frame
                .and_then(|fr| fr.get(&port.name.name).cloned())
                .unwrap_or_else(|| Value::zero(32));
            let w = pre.formals[i].1;
            // `this` is reg `this_reg`; formals are the following registers.
            let reg = this_reg as usize + 1usize + i;
            if reg < block.num_regs as usize {
                let mut v = val;
                if w > 0 && v.width != w {
                    v = v.resize_for_assign(w);
                }
                self.vm_regs[reg] = v;
            }
        }
        // Step 9e: seed a void/ctor result cell with the frame's implicit
        // return local — the interpreter seeded it before entry and its
        // epilogue reads it back (`return_value.or(implicit)`), so a body
        // that never writes the cell returns the identical value.
        // class-perf Step 9h: a CLASS-HANDLE result cell is seeded too —
        // `$cast(funcname, obj)` fits its write against the destination
        // cell's CURRENT width, and an unseeded reg (width 1) truncated
        // the handle to its low bit (uvm_sequence_base::create_item
        // returned null under the gate).
        if pre.is_void_result || pre.is_class_result {
            let reg = result_reg as usize;
            if reg < block.num_regs as usize {
                self.vm_regs[reg] = frame
                    .and_then(|fr| fr.get(&pre.fn_ret_name).cloned())
                    .unwrap_or_else(|| Value::zero(32));
            }
        }
        // §6.21: seed the body's STATIC locals (register-backed; the
        // compiler emits no seed — see `method_static_locals`) from the
        // persistent store, or the block's recorded default on the first
        // call, and open their sync entries so the exit
        // `sync_static_locals` persists the final register values. Keys
        // mirror the AST declaration site exactly (class/spec/subroutine-
        // qualified), so the AST path and the compiled path share one cell.
        if !block.static_locals.is_empty() {
            let sub_name = self
                .static_local_syncs
                .last()
                .map(|(n, _)| n.clone())
                .unwrap_or_default();
            let mut seeds: Vec<(String, usize, Value)> = Vec::new();
            for (nm, reg, default) in &block.static_locals {
                let key = match (
                    self.class_context_stack.last().and_then(|c| c.as_ref()),
                    self.current_spec.as_ref(),
                ) {
                    (Some(cn), Some((spec_base, sig))) if spec_base == cn => {
                        format!("{}#{}::{}::{}", cn, sig, sub_name, nm)
                    }
                    (Some(cn), _) => format!("{}::{}::{}", cn, sub_name, nm),
                    _ => format!("{}::{}", sub_name, nm),
                };
                let persisted = self.static_local_vars.get(&key).cloned();
                if let Some((_n, syncs)) = self.static_local_syncs.last_mut() {
                    syncs.push((nm.clone(), key));
                }
                seeds.push((
                    nm.clone(),
                    *reg as usize,
                    persisted.unwrap_or_else(|| default.clone()),
                ));
            }
            for (nm, reg, v) in seeds {
                if reg < self.vm_regs.len() {
                    self.vm_regs[reg] = v.clone();
                }
                if let Some(fr) = self.local_stack.last_mut() {
                    fr.insert(nm, v);
                }
            }
        }
        // class-perf Step 9c: pre-record the type metadata that executing
        // the body's TOP-LEVEL VarDecls would write (class type, `#()`
        // args, container kind, typedef/enum name). The AST path records a
        // local's type when its VarDecl executes — top-level decls run at
        // method entry — so the compiled path must present the same picture
        // to `$cast`'s runtime type check (`cast_type_ok` reads
        // `class_of_var`/`enum_typename_of_var`, which consult these
        // overlays). Formals and the return cell were already recorded by
        // the binding loop above. Nested-block decls are NOT pre-recorded
        // (the interpreter records them only when the block runs); the
        // compiler declines a `$cast` with such a destination instead.
        for st in body {
            if let crate::ast::stmt::StatementKind::VarDecl {
                data_type,
                declarators,
                ..
            } = &st.kind
            {
                let cn = self.derive_var_decl_class_name(data_type);
                for d in declarators {
                    if d.dimensions.is_empty() {
                        self.record_var_decl_type_metadata(data_type, &d.name.name, cn.as_deref());
                    }
                }
            }
        }
        // class-perf Step 9h: register the implicit fn-return local's
        // CLASS type, exactly as the interpreter's binding site does
        // (`record_local_class_type` + `var_class_types` for a
        // TypeReference return). Without it a `$cast(funcname, obj)`
        // inside a compiled body (uvm_sequence_base::create_item) fails
        // its runtime type check and nulls the return — `new_report_message`-
        // style implicit returns and bare-`new()` returns keep working too.
        if !pre.fn_ret_name.is_empty()
            && let crate::ast::types::DataType::TypeReference { name, .. } = &*pre.return_type
        {
            let cn = name.name.name.clone();
            if self.module.classes.contains_key(&cn) {
                let rn = pre.fn_ret_name.clone();
                self.record_local_class_type(&rn, &cn);
                self.var_class_types.insert(rn, cn);
            }
        }
        let _prev_comp = if SAMPLER_READY.load(Ordering::Relaxed) {
            Some(CUR_METHOD_COMPILED.swap(true, Ordering::Relaxed))
        } else {
            None
        };
        // Run the block.
        self.return_flag = false;
        self.exec_insns(&block.instructions);
        // A StmtFallback for a `return …` statement executed through the
        // interpreter sets `return_value` — the compiled block's result
        // register never sees it (the compiler only bakes the Move for
        // return expressions it could LOWER). Mirror the interpreter's
        // `return_value.or(implicit)` epilogue: an explicit return wins.
        // (The interpreter seeds the implicit cell and reads it back; an
        // explicit `return x` overrode it the same way.)
        if let Some(rv) = self.return_value.take() {
            if (result_reg as usize) < self.vm_regs.len() {
                self.vm_regs[result_reg as usize] = rv;
            }
        }
        if let Some(prev) = _prev_comp {
            CUR_METHOD_COMPILED.store(prev, Ordering::Relaxed);
        }
        let result = self
            .vm_regs
            .get(result_reg as usize)
            .cloned()
            .unwrap_or_else(|| Value::zero(32));
        // class-perf row 2: copy non-INPUT formal registers back into the
        // bound locals frame BEFORE restoring the caller's register file,
        // so the interpreter's epilogue write-back (output_bindings snapshot
        // -> `assign_value` to the caller's actual) observes the body's
        // final values — exactly what the AST path's frame locals hold at
        // its own epilogue. Register width mirrors the AST path: stores in
        // the block resized to the same formal width the interpreter's
        // assignments would use, and w=0 (string/enum/class) formals keep
        // the register's raw value.
        if !pre.writeback_formals.is_empty() {
            let mut outs: Vec<(String, Value)> = Vec::new();
            for (i, port) in ports.iter().enumerate() {
                if pre.writeback_formals.iter().any(|n| n == &port.name.name) {
                    let reg = this_reg as usize + 1usize + i;
                    if let Some(v) = self.vm_regs.get(reg).cloned() {
                        outs.push((port.name.name.clone(), v));
                    }
                }
            }
            if !outs.is_empty()
                && let Some(fr) = self.local_stack.last_mut()
            {
                for (n, v) in outs {
                    fr.insert(n, v);
                }
            }
        }
        // §6.21: persist the final STATIC-local registers. The store is
        // written directly: for a scalar static the interpreter keeps
        // `static_local_vars` authoritative (write-through), so the exit
        // `sync_static_locals` deliberately skips it and a frame copy alone
        // would be dropped. The frame copy keeps the other statics' sync.
        if !block.static_locals.is_empty() {
            let keys: Vec<String> = self
                .static_local_syncs
                .last()
                .map(|(_, syncs)| {
                    block
                        .static_locals
                        .iter()
                        .map(|(nm, _, _)| {
                            syncs
                                .iter()
                                .find(|(n, _)| n == nm)
                                .map(|(_, k)| k.clone())
                                .unwrap_or_default()
                        })
                        .collect()
                })
                .unwrap_or_default();
            for (i, (nm, reg, _)) in block.static_locals.iter().enumerate() {
                let Some(v) = self.vm_regs.get(*reg as usize).cloned() else {
                    continue;
                };
                if let Some(k) = keys.get(i)
                    && !k.is_empty()
                {
                    self.static_local_vars.insert(k.clone(), v.clone());
                }
                if let Some(fr) = self.local_stack.last_mut() {
                    fr.insert(nm.clone(), v);
                }
            }
        }
        // Restore the CALLER's register file now that we're done with ours.
        self.vm_regs = saved_vm_regs;
        // Restore the caller's foreach key arena (swap-in above cleared the
        // callee's; the Vec's spare capacity is dropped with it).
        self.foreach_arena = saved_foreach_arena;
        // Clamp width (mirrors the interpreter's dyn_ret clamp) and stamp the
        // declared signedness for plainly-integral returns (mirrors its
        // §13.4.1 signedness stamp). A CLASS-HANDLE return is passed through
        // UNTOUCHED (no resize, no stamp) exactly like the interpreter, which
        // never resizes/stamps a non-`plainly_integral` (Typeref) return.
        // class-perf collection-return: the value left via
        // `pending_ret_collection` (the `RetCollection` exec arm); the result
        // register is dead — return a placeholder, untouched, like a class
        // handle. (The stale-pending fall-off-end semantics match the
        // interpreter: it only clears pending inside Return statements too.)
        let mut result = result;
        if pre.is_collection_result {
            return Some(Value::zero(32));
        }
        if result.is_real && !pre.is_class_result && !pre.is_void_result {
            result = Self::compiled_ret_from_real(&result, result_width);
        }
        if result_width > 0 && result.width != result_width {
            result = result.resize_for_assign(result_width);
        }
        if !pre.is_class_result && !pre.is_void_result && !result.is_real {
            result.is_signed = pre.result_signed;
        }
        Some(result)
    }

    /// class-perf P3: build (or permanently decline) the direct-call entry
    /// for `dclass.method`, from the cached plan + compiled block. Returns
    /// the entry when direct-callable. Pure lookup + a one-time insn scan;
    /// the result is memoized in `compiled_fast_calls`.
    pub(super) fn vm_fast_entry_for(
        &mut self,
        dclass: &str,
        method: &str,
        mdef: &std::sync::Arc<crate::ast::decl::ClassMethod>,
        spec_id: u32,
    ) -> Option<std::rc::Rc<FastCallEntry>> {
        use super::bytecode::Insn;
        let crate::ast::decl::ClassMethodKind::Function(f) = &mdef.kind else {
            // Tasks / pure-virtual prototypes never have a compiled plan;
            // no memoization needed — the plan lookup below will keep
            // returning None.
            return None;
        };
        let body: &[crate::ast::stmt::Statement] = &f.items;
        let ids = &self.compiled_class_method_ids;
        let (cid, mid) = match (ids.get(dclass), ids.get(method)) {
            (Some(&c), Some(&m)) => (c, m),
            _ => return None, // never interned: never compiled
        };
        if let Some(state) = self.compiled_fast_calls.get(&(cid, mid, spec_id)) {
            return match state {
                FastCallState::Entry(rc) => Some(rc.clone()),
                FastCallState::Never => None,
                FastCallState::Pending => None, // re-probed below
            };
        }
        let never = |sim: &mut Self| {
            sim.compiled_fast_calls
                .insert((cid, mid, spec_id), FastCallState::Never);
        };
        let Some(pre) = self
            .compiled_method_plans
            .get(&(cid, mid, spec_id))
            .cloned()
        else {
            return None; // no plan yet: this call goes the interpreter route
        };
        // Shape guards (permanent): non-input formals need the interpreter's
        // write-back epilogue; a param-bounded return needs the instance's
        // param scope; static methods dispatch through exec_static_method.
        // NOTE: a class whose methods carry a header `#(...)` list is NOT
        // excluded here: by the time this function runs, `pre` is a compiled
        // PLAN for this concrete method (the interpreter path `try_run_compiled_method`
        // already compiled and runs it), so the block is specialization-correct.
        // The body/instruction guards below (static locals, fallback insns)
        // are what genuinely forbid a frame-free run.
        if !pre.writeback_formals.is_empty()
            || pre.param_able_result
            || self.is_static_method(dclass, method)
        {
            never(self);
            return None;
        }
        // class-perf collection-return: the callee returns via the
        // `pending_ret_collection` / `vm_ret_vals` side channel. A VM->VM
        // direct call CAN carry this when `vm_try_direct_call` re-stashes the
        // block's `RetCollection` flat values into `vm_ret_vals` for the
        // caller's `AssignPendingColl`; we no longer blacklist it here. The
        // instruction guards below (static locals, fallback insns) and the
        // transport in `vm_try_direct_call` keep such a call correct.
        let key = (
            cid,
            mid,
            spec_id,
            pre.formal_widths.clone(),
            pre.static_result_width,
        );
        let Some(super::bytecode::CompiledMethodOutcome::Block(rc_entry)) =
            self.compiled_method_block_cache.get(&key).cloned()
        else {
            return None; // block not compiled yet (Pending)
        };
        // Body guards (permanent): a `static` local needs the §6.21
        // static-local sync frame the fast path does not open.
        let body_has_static_local = body.iter().any(|st| {
            fn has_static(st: &crate::ast::stmt::Statement) -> bool {
                if let crate::ast::stmt::StatementKind::VarDecl {
                    lifetime: Some(crate::ast::types::Lifetime::Static),
                    declarators,
                    ..
                } = &st.kind
                    && !declarators.is_empty()
                {
                    return true;
                }
                Simulator::sub_stmts(st).into_iter().any(has_static)
            }
            has_static(st)
        });
        if body_has_static_local {
            never(self);
            return None;
        }
        // Instruction guards (permanent): the block must be frame-free —
        // no interpreter re-entry that resolves NAMES against the binding
        // loop's frames (EvalExprFallback/StmtFallback would read formals
        // the fast path never records into `var_class_types`/
        // `class_of_var`-style metadata), no process-local loads, and no
        // `$cast` (its runtime type check reads the binding loop's
        // per-formal class/enum recordings). Note `InitLocalColl` is NOT
        // forbidden: it declares empty local-collection storage with no
        // formal reads, and the fast paths already open a `local_dyn` frame
        // (via `push_queue_frame`) so the per-call `@key` registration is
        // correct and cleaned up on exit.
        let block = rc_entry.block.clone();
        let has_forbidden = block.instructions.iter().any(|i| {
            matches!(
                i,
                Insn::EvalExprFallback(..)
                    | Insn::StmtFallback(..)
                    | Insn::LoadProcessLocal(..)
                    | Insn::Cast(..)
            )
        });
        if has_forbidden {
            never(self);
            return None;
        }
        // Per-formal coercion, replicating the interpreter's binding loop
        // (see exec_method_in_class_hierarchy): class/string formals bind
        // as passed; an integral TypeReference typedef resizes and stamps;
        // a concrete-signed type-param/enum formal keeps the caller's width
        // but stamps signed; IntegerAtom / literal-packed formals resize to
        // the declared width and stamp; other packed formals keep width.
        use crate::ast::types::DataType as DT;
        let mut coerce: Vec<(Box<str>, u32, Option<bool>)> = Vec::with_capacity(f.ports.len());
        for port in f.ports.iter() {
            let dt = &port.data_type;
            let c = if self.typeref_names_class(dt) || Self::is_string_data_type(dt) {
                (0, None)
            } else if matches!(dt, DT::TypeReference { .. }) {
                if let Some((pw, signed)) = self.scalar_formal_integral(dt) {
                    (pw, Some(signed))
                } else if self.type_is_signed_concrete(dt) {
                    (0, Some(true))
                } else {
                    (0, None)
                }
            } else if matches!(dt, DT::IntegerAtom { .. }) || Self::packed_dims_are_literal(dt) {
                let pw = super::elaborate::resolve_type_width(
                    dt,
                    Some(&self.module.parameters),
                    Some(&self.module.typedefs),
                );
                (pw.max(1), Some(super::elaborate::is_type_signed(dt)))
            } else if super::elaborate::is_type_signed(dt) {
                (0, Some(true))
            } else {
                (0, None)
            };
            coerce.push((port.name.name.clone().into_boxed_str(), c.0, c.1));
        }
        // The interpreter's implicit-return-cell init (see the binding
        // loop): string -> empty string; real -> 0.0; two-state -> zero;
        // otherwise a 4-state x of the declared width (a class/
        // TypeReference return resolves to width 0 -> 1).
        let seed = if Self::is_string_data_type(&pre.return_type) {
            Value::from_string("")
        } else {
            let rw = super::elaborate::resolve_type_width(
                &pre.return_type,
                Some(&self.module.parameters),
                Some(&self.module.typedefs),
            )
            .max(1);
            if super::elaborate::is_type_real(&pre.return_type) {
                Value::from_f64(0.0)
            } else if super::elaborate::is_type_two_state(&pre.return_type) {
                Value::zero(rw)
            } else {
                Value::new(rw)
            }
        };
        let meaningful: Vec<&Insn> = block
            .instructions
            .iter()
            .filter(|i| !matches!(i, Insn::Nop) && !matches!(i, Insn::Move(d, s) if d == s))
            .collect();
        let getter_field = if coerce.is_empty() {
            match meaningful.as_slice() {
                [Insn::LoadClassMember(d, h, field)]
                    if *h == rc_entry.this_reg && *d == rc_entry.result_reg =>
                {
                    Some(field.as_ref())
                }
                [Insn::LoadClassMember(d, h, field), Insn::Move(rd, s)]
                    if *h == rc_entry.this_reg && *rd == rc_entry.result_reg && *s == *d =>
                {
                    Some(field.as_ref())
                }
                [Insn::LoadClassMember(d, h, field), Insn::Resize(rd, _)]
                    if *h == rc_entry.this_reg && *rd == rc_entry.result_reg && *rd == *d =>
                {
                    Some(field.as_ref())
                }
                [
                    Insn::LoadClassMember(d, h, field),
                    Insn::Move(rd, s),
                    Insn::Resize(rrd, _),
                ] if *h == rc_entry.this_reg
                    && *rd == rc_entry.result_reg
                    && *s == *d
                    && *rrd == *rd =>
                {
                    Some(field.as_ref())
                }
                _ => None,
            }
        } else {
            None
        };
        let setter_field = if coerce.len() == 1 {
            match meaningful.as_slice() {
                [Insn::StoreClassMember(h, v, field)]
                    if *h == rc_entry.this_reg && *v == rc_entry.this_reg + 1 =>
                {
                    Some(field.as_ref())
                }
                [Insn::Move(v2, v1), Insn::StoreClassMember(h, v, field)]
                    if *h == rc_entry.this_reg && *v1 == rc_entry.this_reg + 1 && *v == *v2 =>
                {
                    Some(field.as_ref())
                }
                _ => None,
            }
        } else {
            None
        };
        let accessor = if let Some(f) = getter_field {
            FastAccessor::Getter(Box::from(f))
        } else if let Some(f) = setter_field {
            FastAccessor::Setter(Box::from(f))
        } else {
            FastAccessor::None
        };
        let entry = std::rc::Rc::new(FastCallEntry {
            block,
            this_reg: rc_entry.this_reg,
            result_reg: rc_entry.result_reg,
            coerce,
            seed,
            result_width: pre.static_result_width,
            result_signed: pre.result_signed,
            is_class_result: pre.is_class_result,
            is_void_result: pre.is_void_result,
            accessor,
        });
        self.compiled_fast_calls
            .insert((cid, mid, spec_id), FastCallState::Entry(entry.clone()));
        Some(entry)
    }
    /// Fast-call twin of `vm_fast_entry_for` for free (package-scope)
    /// functions compiled under the synthetic `__free_fn__<pkg>` class.
    /// Same plan gates minus the receiver-specific ones (no `this`, no
    /// accessors, no parameterized-class or static-method concerns).
    pub(super) fn vm_fast_free_fn_entry_for(
        &mut self,
        cname: &str,
        fname: &str,
        fd: &FunctionDeclaration,
    ) -> Option<std::rc::Rc<FastCallEntry>> {
        use super::bytecode::Insn;
        let ids = &self.compiled_class_method_ids;
        let (cid, mid) = match (ids.get(cname), ids.get(fname)) {
            (Some(&c), Some(&m)) => (c, m),
            _ => return None, // never interned: never compiled
        };
        if let Some(state) = self.compiled_fast_calls.get(&(cid, mid, 0)) {
            return match state {
                FastCallState::Entry(rc) => Some(rc.clone()),
                FastCallState::Never => None,
                FastCallState::Pending => None, // re-probed below
            };
        }
        let never = |sim: &mut Self| {
            sim.compiled_fast_calls
                .insert((cid, mid, 0), FastCallState::Never);
        };
        if self.compiled_method_skip.contains(&(cid, mid, 0)) {
            never(self);
            return None;
        }
        let Some(pre) = self.compiled_method_plans.get(&(cid, mid, 0)).cloned() else {
            return None; // no plan yet: this call goes the interpreter route
        };
        if !pre.writeback_formals.is_empty() {
            never(self);
            return None;
        }
        if pre.is_collection_result {
            never(self);
            return None;
        }
        // A param-able result type is fine here: a free function has no
        // receiver, so its scope is fixed (handle 0) and the slow path's
        // width resolution is fully deterministic. Mirror it so the block
        // cache key matches.
        let result_width = if pre.param_able_result {
            let scope = self.instance_param_scope(0);
            let w = super::elaborate::resolve_type_width(
                &pre.return_type,
                Some(&scope),
                Some(&self.module.typedefs),
            );
            if w > 0 { w } else { pre.static_result_width }
        } else {
            pre.static_result_width
        };
        let key = (cid, mid, 0, pre.formal_widths.clone(), result_width);
        let Some(super::bytecode::CompiledMethodOutcome::Block(rc_entry)) =
            self.compiled_method_block_cache.get(&key).cloned()
        else {
            return None; // block not compiled yet (Pending)
        };
        // Static locals are synced through interpreter frames keyed on the
        // caller's class context — a free-function fast call cannot provide
        // the callee's own scope, so keep those functions on the slow path.
        let body_has_static_local = fd.items.iter().any(|st| {
            fn has_static(st: &crate::ast::stmt::Statement) -> bool {
                if let crate::ast::stmt::StatementKind::VarDecl {
                    lifetime: Some(crate::ast::types::Lifetime::Static),
                    declarators,
                    ..
                } = &st.kind
                    && !declarators.is_empty()
                {
                    return true;
                }
                Simulator::sub_stmts(st).into_iter().any(has_static)
            }
            has_static(st)
        });
        if body_has_static_local {
            never(self);
            return None;
        }
        let block = rc_entry.block.clone();
        let has_forbidden = block.instructions.iter().any(|i| {
            matches!(
                i,
                Insn::EvalExprFallback(..)
                    | Insn::StmtFallback(..)
                    | Insn::LoadProcessLocal(..)
                    | Insn::Cast(..)
            )
        });
        if has_forbidden {
            never(self);
            return None;
        }
        use crate::ast::types::DataType as DT;
        let mut coerce: Vec<(Box<str>, u32, Option<bool>)> = Vec::with_capacity(fd.ports.len());
        for port in fd.ports.iter() {
            let dt = &port.data_type;
            let c = if self.typeref_names_class(dt) || Self::is_string_data_type(dt) {
                (0, None)
            } else if matches!(dt, DT::TypeReference { .. }) {
                if let Some((pw, signed)) = self.scalar_formal_integral(dt) {
                    (pw, Some(signed))
                } else if self.type_is_signed_concrete(dt) {
                    (0, Some(true))
                } else {
                    (0, None)
                }
            } else if matches!(dt, DT::IntegerAtom { .. }) || Self::packed_dims_are_literal(dt) {
                let pw = super::elaborate::resolve_type_width(
                    dt,
                    Some(&self.module.parameters),
                    Some(&self.module.typedefs),
                );
                (pw.max(1), Some(super::elaborate::is_type_signed(dt)))
            } else if super::elaborate::is_type_signed(dt) {
                (0, Some(true))
            } else {
                (0, None)
            };
            coerce.push((port.name.name.clone().into_boxed_str(), c.0, c.1));
        }
        let seed = if Self::is_string_data_type(&pre.return_type) {
            Value::from_string("")
        } else {
            let rw = super::elaborate::resolve_type_width(
                &pre.return_type,
                Some(&self.module.parameters),
                Some(&self.module.typedefs),
            )
            .max(1);
            if super::elaborate::is_type_real(&pre.return_type) {
                Value::from_f64(0.0)
            } else if super::elaborate::is_type_two_state(&pre.return_type) {
                Value::zero(rw)
            } else {
                Value::new(rw)
            }
        };
        let entry = std::rc::Rc::new(FastCallEntry {
            block,
            this_reg: rc_entry.this_reg,
            result_reg: rc_entry.result_reg,
            coerce,
            seed,
            result_width,
            result_signed: pre.result_signed,
            is_class_result: pre.is_class_result,
            is_void_result: pre.is_void_result,
            accessor: FastAccessor::None,
        });
        self.compiled_fast_calls
            .insert((cid, mid, 0), FastCallState::Entry(entry.clone()));
        Some(entry)
    }
    /// Register-machine fast dispatch for a compiled free function invoked
    /// from compiled code (`Insn::CallFreeFunction`). Mirrors
    /// `vm_try_direct_call` minus the receiver (free functions carry no
    /// `this`); `pkg` is the callee's package scope, pushed so that nested
    /// interpreter re-entries resolve names exactly as the slow path would.
    pub(super) fn vm_try_direct_free_fn_call(
        &mut self,
        fd: &FunctionDeclaration,
        pkg: Option<&str>,
        arg_base: usize,
        n: usize,
    ) -> Option<Value> {
        if !fast_calls_enabled() {
            return None;
        }
        let fname = fd.name.name.name.as_str();
        let cname = self.free_fn_class_name(pkg);
        // Only a callee that already compiled through the interpreter route
        // (`try_run_compiled_free_function`, which owns the tier counter and
        // the skip cache) has an entry; everything else declines here
        // without touching any counter.
        let Some(entry) = self.vm_fast_free_fn_entry_for(&cname, fname, fd) else {
            return None;
        };
        if n != entry.coerce.len() {
            return None; // default-valued formals: interpreter fills them
        }
        let mut args: Vec<Value> = Vec::with_capacity(n);
        for (i, (_, w, sgn)) in entry.coerce.iter().enumerate() {
            let Some(src) = self.vm_regs.get(arg_base + i) else {
                return None;
            };
            if src.is_real {
                return None;
            }
            let mut v = src.clone();
            if *w > 0 && v.width != *w {
                v = v.resize_for_assign(*w);
            }
            if let Some(s) = sgn {
                v.is_signed = *s;
            }
            args.push(v);
        }
        self.push_queue_frame();
        self.this_stack.push(None);
        self.class_context_stack.push(None);
        let mut locals = self.take_pooled_frame();
        for ((name, _, _), v) in entry.coerce.iter().zip(args.iter()) {
            locals.insert(name.to_string(), v.clone());
        }
        self.push_local_frame(locals);
        self.pkg_scope_stack.push(pkg.map(str::to_string));
        self.func_call_stack.push(fd.name.name.name.clone());
        let _prev_compiled = if SAMPLER_READY.load(std::sync::atomic::Ordering::Relaxed) {
            Some(CUR_METHOD_COMPILED.swap(true, std::sync::atomic::Ordering::Relaxed))
        } else {
            None
        };
        let caller_regs = std::mem::take(&mut self.vm_regs);
        let saved_foreach_arena = std::mem::take(&mut self.foreach_arena);
        let saved_return = self.return_flag;
        self.return_flag = false;
        let mut regs = vec![Value::zero(1); entry.block.num_regs as usize];
        regs[entry.this_reg as usize] = Value::from_u64(0, 32);
        for (i, v) in args.into_iter().enumerate() {
            let reg = entry.this_reg as usize + 1 + i;
            if reg < entry.block.num_regs as usize {
                regs[reg] = v;
            }
        }
        if (entry.is_void_result || entry.is_class_result)
            && (entry.result_reg as usize) < entry.block.num_regs as usize
        {
            regs[entry.result_reg as usize] = entry.seed.clone();
        }
        self.vm_regs = regs;
        self.exec_insns(&entry.block.instructions);
        let result = self
            .vm_regs
            .get(entry.result_reg as usize)
            .cloned()
            .unwrap_or_else(|| Value::zero(32));
        self.vm_regs = caller_regs;
        self.foreach_arena = saved_foreach_arena;
        if let Some(prev) = _prev_compiled {
            CUR_METHOD_COMPILED.store(prev, std::sync::atomic::Ordering::Relaxed);
        }
        self.return_flag = saved_return;
        self.func_call_stack.pop();
        self.pkg_scope_stack.pop();
        self.pop_local_frame();
        self.class_context_stack.pop();
        self.this_stack.pop();
        self.pop_and_restore_queue_frame();
        let mut result = result;
        if result.is_real && !entry.is_class_result && !entry.is_void_result {
            result = Self::compiled_ret_from_real(&result, entry.result_width);
        }
        if entry.result_width > 0 && result.width != entry.result_width {
            result = result.resize_for_assign(entry.result_width);
        }
        if !entry.is_class_result && !entry.is_void_result && !result.is_real {
            result.is_signed = entry.result_signed;
        }
        Some(result)
    }

    /// class-perf Change 3: build (or permanently decline) the direct-call
    /// entry for a NO-ARG STATIC class function, from the cached plan +
    /// compiled block. Mirrors `vm_fast_free_fn_entry_for` but (a) admits
    /// static methods — this is their home — and (b) admits bodies with
    /// `static` locals: the runner opens the exact §6.21 sync frame
    /// `try_run_compiled_method` opens, keyed on the callee's class context
    /// pushed by `exec_method_in_class_hierarchy`, so the persisted keys are
    /// identical. Memoized in `compiled_fast_static_calls`.
    pub(super) fn vm_fast_static_entry_for(
        &mut self,
        dclass: &str,
        method: &str,
        mdef: &std::sync::Arc<crate::ast::decl::ClassMethod>,
    ) -> Option<std::rc::Rc<FastCallEntry>> {
        use super::bytecode::Insn;
        let crate::ast::decl::ClassMethodKind::Function(f) = &mdef.kind else {
            return None;
        };
        // The runner here opens no formal binding and no default-arg fill:
        // a no-arg shape only.
        if !f.ports.is_empty() {
            return None;
        }
        let ids = &self.compiled_class_method_ids;
        let (cid, mid) = match (ids.get(dclass), ids.get(method)) {
            (Some(&c), Some(&m)) => (c, m),
            _ => return None, // never interned: never compiled
        };
        if let Some(state) = self.compiled_fast_static_calls.get(&(cid, mid, 0)) {
            return match state {
                FastCallState::Entry(rc) => Some(rc.clone()),
                FastCallState::Never => None,
                FastCallState::Pending => None, // re-probed below
            };
        }
        let never = |sim: &mut Self| {
            sim.compiled_fast_static_calls
                .insert((cid, mid, 0), FastCallState::Never);
        };
        if self.compiled_method_skip.contains(&(cid, mid, 0)) {
            never(self);
            return None;
        }
        let Some(pre) = self.compiled_method_plans.get(&(cid, mid, 0)).cloned() else {
            return None; // no plan yet: this call goes the interpreter route
        };
        if pre.param_able_result || pre.is_collection_result {
            never(self);
            return None;
        }
        // Only a non-parameterized defining class: the interpreter path's
        // static-local keys branch on `current_spec` matching the pushed
        // class context, and a spec (or a `#(...)` header) changes them.
        // Keeping parameterized classes on the interpreter route is the
        // safe subset — `uvm_regex_cache` and other plain singletons are
        // what this path exists for. An ACTIVE spec belonging to a
        // DIFFERENT (caller) class is harmless: the §6.21 key branch fires
        // only when `spec_base == cn`, and the runner replicates the slow
        // path's derivation verbatim.
        if self.class_is_parameterized(dclass) {
            never(self);
            return None;
        }
        let key = (
            cid,
            mid,
            0,
            pre.formal_widths.clone(),
            pre.static_result_width,
        );
        let Some(super::bytecode::CompiledMethodOutcome::Block(rc_entry)) =
            self.compiled_method_block_cache.get(&key).cloned()
        else {
            return None; // block not compiled yet (Pending)
        };
        // Instruction guards (permanent): the block must be frame-free —
        // no interpreter re-entry that resolves NAMES against the binding
        // loop's frames (EvalExprFallback/StmtFallback would read formals
        // the fast path never records into `var_class_types`/
        // `class_of_var`-style metadata), no process-local loads, and no
        // `$cast` (its runtime type check reads the binding loop's
        // per-formal class/enum recordings). `InitLocalColl` stays legal
        // for the same reason as the other fast paths.
        let block = rc_entry.block.clone();
        let has_forbidden = block.instructions.iter().any(|i| {
            matches!(
                i,
                Insn::EvalExprFallback(..)
                    | Insn::StmtFallback(..)
                    | Insn::LoadProcessLocal(..)
                    | Insn::Cast(..)
            )
        });
        if has_forbidden {
            never(self);
            return None;
        }
        let entry = std::rc::Rc::new(FastCallEntry {
            block,
            this_reg: rc_entry.this_reg,
            result_reg: rc_entry.result_reg,
            coerce: Vec::new(), // no-arg only
            seed: Value::zero(32),
            result_width: pre.static_result_width,
            result_signed: pre.result_signed,
            is_class_result: pre.is_class_result,
            is_void_result: pre.is_void_result,
            accessor: FastAccessor::None,
        });
        self.compiled_fast_static_calls
            .insert((cid, mid, 0), FastCallState::Entry(entry.clone()));
        Some(entry)
    }

    /// class-perf Change 3: direct VM->VM dispatch for `CallStaticScoped` —
    /// run a compiled NO-ARG STATIC class function's block in place,
    /// skipping the per-call `exec_class_scope_static_call` resolution walk
    /// (type-param binding, spec extraction, typedef aliases), the class-
    /// chain walk and `exec_method_in_class_hierarchy`'s frame setup, and
    /// `try_run_compiled_method`'s interning/plan/key prologue — the whole
    /// of which re-paid every call for singleton getters in hot loops
    /// (`uvm_regex_cache::get_inst`, 1293 AST samples per 5M run).
    ///
    /// Static-local correctness: opens the SAME §6.21 sync frame the slow
    /// path opens (via `exec_method_in_class_hierarchy`), pushes the SAME
    /// class context (`Some(defining_class)`), and seeds/persists through
    /// `static_local_vars` with the identical key derivation — so the AST
    /// path, the compiled slow path and this path share one cell.
    /// `None` = not eligible; the caller then runs the historic interpreter
    /// path, byte-identical.
    pub(super) fn vm_try_direct_static_call(
        &mut self,
        dclass: &str,
        method: &str,
        _arg_base: usize,
        n: usize,
    ) -> Option<Value> {
        if !fast_calls_enabled() || n != 0 || method == "new" {
            return None;
        }
        // §8.10: a class-scope `::` call names a STATIC member. A non-static
        // method reached this way must be a fatal (reference simulators reject
        // it at compile time) — never a fast-path dispatch. Route through the
        // interpreter's check so the message is emitted once, identically.
        if self.class_has_method(dclass, method) && !self.is_static_method(dclass, method) {
            return None;
        }
        // Built-in interception set (exec_method_call's early arms): keep
        // the interpreter route.
        if matches!(
            method,
            "srandom"
                | "get_randstate"
                | "set_randstate"
                | "randomize"
                | "kill"
                | "await"
                | "suspend"
                | "resume"
                | "status"
        ) {
            return None;
        }
        // Resolve the defining method WITHOUT the resolution walk: the
        // class must name a concrete class that directly defines (or
        // inherits) the method; the hierarchy walk is the memoized part.
        let md = self.method_defining_class(dclass, method)?;
        let mdef = md.method.clone();
        let cname = md.class.to_string();
        let entry = self.vm_fast_static_entry_for(&cname, method, &mdef)?;
        // --- every guard passed; state changes from here on ---
        let caller_regs = std::mem::take(&mut self.vm_regs);
        let saved_foreach_arena = std::mem::take(&mut self.foreach_arena);
        self.push_queue_frame();
        self.this_stack.push(None);
        self.class_context_stack.push(Some(cname.clone()));
        let locals = self.take_pooled_frame();
        self.push_local_frame(locals);
        // §6.21: open the static-local sync frame exactly as
        // `exec_method_in_class_hierarchy` does, then seed the body's
        // static locals from the persistent store with the identical key
        // derivation (`class_context_stack.last()` is the class just
        // pushed, `current_spec` is None for the admitted shape).
        let sync_name = self.sync_frame_name(method);
        self.static_local_syncs.push((sync_name, Vec::new()));
        // Install the callee's register file BEFORE seeding: the static-
        // local seed writes callee registers (mirrors `try_run_compiled_
        // method`, which resizes the taken file and seeds into it).
        let mut regs = vec![Value::zero(1); entry.block.num_regs as usize];
        regs[entry.this_reg as usize] = Value::from_u64(0, 32);
        if (entry.is_void_result || entry.is_class_result)
            && (entry.result_reg as usize) < entry.block.num_regs as usize
        {
            regs[entry.result_reg as usize] = entry.seed.clone();
        }
        self.vm_regs = regs;
        if !entry.block.static_locals.is_empty() {
            let sub_name = self
                .static_local_syncs
                .last()
                .map(|(n, _)| n.clone())
                .unwrap_or_default();
            let mut seeds: Vec<(String, usize, Value)> = Vec::new();
            for (nm, reg, default) in &entry.block.static_locals {
                let key = match (
                    self.class_context_stack.last().and_then(|c| c.as_ref()),
                    self.current_spec.as_ref(),
                ) {
                    (Some(cn), Some((spec_base, sig))) if spec_base == cn => {
                        format!("{}#{}::{}::{}", cn, sig, sub_name, nm)
                    }
                    (Some(cn), _) => format!("{}::{}::{}", cn, sub_name, nm),
                    _ => format!("{}::{}", sub_name, nm),
                };
                let persisted = self.static_local_vars.get(&key).cloned();
                if let Some((_n, syncs)) = self.static_local_syncs.last_mut() {
                    syncs.push((nm.clone(), key));
                }
                seeds.push((
                    nm.clone(),
                    *reg as usize,
                    persisted.unwrap_or_else(|| default.clone()),
                ));
            }
            for (nm, reg, v) in seeds {
                if reg < self.vm_regs.len() {
                    self.vm_regs[reg] = v.clone();
                }
                if let Some(fr) = self.local_stack.last_mut() {
                    fr.insert(nm, v);
                }
            }
        }
        let _prev_compiled = if SAMPLER_READY.load(std::sync::atomic::Ordering::Relaxed) {
            Some(CUR_METHOD_COMPILED.swap(true, std::sync::atomic::Ordering::Relaxed))
        } else {
            None
        };
        let saved_return = self.return_flag;
        self.return_flag = false;
        self.exec_insns(&entry.block.instructions);
        let result = self
            .vm_regs
            .get(entry.result_reg as usize)
            .cloned()
            .unwrap_or_else(|| Value::zero(32));
        // §6.21: persist the final STATIC-local registers (same derivation
        // as the exit path of `try_run_compiled_method`).
        if !entry.block.static_locals.is_empty() {
            let keys: Vec<String> = self
                .static_local_syncs
                .last()
                .map(|(_, syncs)| {
                    entry
                        .block
                        .static_locals
                        .iter()
                        .map(|(nm, _, _)| {
                            syncs
                                .iter()
                                .find(|(n, _)| n == nm)
                                .map(|(_, k)| k.clone())
                                .unwrap_or_default()
                        })
                        .collect()
                })
                .unwrap_or_default();
            for (i, (nm, reg, _)) in entry.block.static_locals.iter().enumerate() {
                let Some(v) = self.vm_regs.get(*reg as usize).cloned() else {
                    continue;
                };
                if let Some(k) = keys.get(i)
                    && !k.is_empty()
                {
                    self.static_local_vars.insert(k.clone(), v.clone());
                }
                if let Some(fr) = self.local_stack.last_mut() {
                    fr.insert(nm.clone(), v);
                }
            }
        }
        self.vm_regs = caller_regs;
        self.foreach_arena = saved_foreach_arena;
        if let Some(prev) = _prev_compiled {
            CUR_METHOD_COMPILED.store(prev, std::sync::atomic::Ordering::Relaxed);
        }
        self.return_flag = saved_return;
        self.static_local_syncs.pop();
        self.pop_local_frame();
        self.class_context_stack.pop();
        self.this_stack.pop();
        self.pop_and_restore_queue_frame();
        let mut result = result;
        if result.is_real && !entry.is_class_result && !entry.is_void_result {
            result = Self::compiled_ret_from_real(&result, entry.result_width);
        }
        if entry.result_width > 0 && result.width != entry.result_width {
            result = result.resize_for_assign(entry.result_width);
        }
        if !entry.is_class_result && !entry.is_void_result && !result.is_real {
            result.is_signed = entry.result_signed;
        }
        Some(result)
    }

    /// class-perf P3: direct VM->VM dispatch for `CallMethod` — run a
    /// compiled callee's block in place, seeding its registers from the
    /// CALLER's argument registers instead of materializing sized-hex
    /// constant Expressions and re-entering the interpreter's binding
    /// loop. `None` = not eligible (guards below); the caller then runs
    /// the historic interpreter path, byte-identical.
    ///
    /// Guards, each mirroring an `exec_method_call` behavior the fast path
    /// cannot reproduce: constructor-time binding (ctor stack), process /
    /// mailbox / semaphore / randomize built-in interception, static
    /// methods (exec_static_method dispatch), parameterized defining
    /// classes (spec-static seeding), arity mismatch (default-formal
    /// binding via normalize_call_args), real actuals (real->int
    /// coercion in the binding loop), and the tier threshold (the same
    /// counter `try_run_compiled_method` bumps).
    pub(super) fn vm_try_direct_call(
        &mut self,
        handle: usize,
        method: &str,
        arg_base: usize,
        n: usize,
    ) -> Option<Value> {
        if !fast_calls_enabled() || handle == 0 || method == "new" {
            return None;
        }
        // Built-in interception set (exec_method_call's early arms). Names
        // are rare in compiled callees; anything matched keeps the
        // interpreter route.
        if matches!(
            method,
            "srandom"
                | "get_randstate"
                | "set_randstate"
                | "randomize"
                | "kill"
                | "await"
                | "suspend"
                | "resume"
                | "status"
        ) {
            return None;
        }
        if self.mailboxes.contains_key(&handle) || self.semaphores.contains_key(&handle) {
            return None;
        }
        // §8.25 ctor-time binding: while `new` of this handle runs, calls on
        // it bind in the constructing class's chain.
        if let Some((ch, _)) = self.ctor_class_stack.last()
            && *ch == handle
        {
            return None;
        }
        let Some(Some(inst)) = self.heap.get(handle) else {
            return None;
        };
        // Dispatch on the object's class is the virtual binding (§8.20). A
        // non-virtual method that a subclass redeclares never reaches here:
        // the compiler lowers such a call to `CallScopedMethod` starting at
        // the receiver's declared class (`MethodAdmission::nonvirtual_targets`).
        let leaf = inst.class_name.clone();
        if Self::proc_handle_to_pid(handle as u64).is_some() {
            return None;
        }
        let Some(md) = self.method_defining_class(&leaf, method) else {
            return None;
        };
        // Phase 2: specialization id for the receiver's plan/fast caches.
        let spec_id = self.instance_spec_id(handle);
        if self.is_static_method(&leaf, method) {
            return None;
        }
        // Fast-path accessor check: if the method is already compiled as a
        // pure 1-statement getter or setter, execute it with zero allocations
        // or frame setup, even before the tier threshold.
        if let Some(entry) = self.vm_fast_entry_for(&md.class, method, &md.method, spec_id) {
            match &entry.accessor {
                FastAccessor::Getter(field) if n == 0 => {
                    let value = self
                        .heap
                        .get(handle)
                        .and_then(|o| o.as_ref())
                        .and_then(|inst| inst.properties.get(field.as_ref()))
                        .cloned()
                        .unwrap_or_else(|| Value::zero(1));
                    let mut result = value;
                    if result.is_real && !entry.is_class_result && !entry.is_void_result {
                        result = Self::compiled_ret_from_real(&result, entry.result_width);
                    }
                    if entry.result_width > 0 && result.width != entry.result_width {
                        result = result.resize_for_assign(entry.result_width);
                    }
                    if !entry.is_class_result && !entry.is_void_result && !result.is_real {
                        result.is_signed = entry.result_signed;
                    }
                    return Some(result);
                }
                FastAccessor::Setter(field) if n == 1 => {
                    let Some(src) = self.vm_regs.get(arg_base) else {
                        return None;
                    };
                    let fitted = {
                        let mut v = src.clone();
                        if let Some(w) = entry.coerce.first().map(|c| c.1)
                            && w > 0
                            && v.width != w
                        {
                            v = v.resize_for_assign(w);
                        }
                        self.fit_class_prop(handle, field.as_ref(), &v)
                    };
                    if let Some(o) = self.heap.get_mut(handle).and_then(|o| o.as_mut()) {
                        o.properties.insert(field.to_string(), fitted);
                    }
                    return Some(Value::zero(32));
                }
                _ => {}
            }
        }
        // Same tier counter `try_run_compiled_method` bumps: the fast path
        // may only run a callee that has already crossed the threshold.
        // Keyed by the same interned (cid, mid) ids the block cache uses.
        let tier = method_tier_threshold();
        if tier > 0 {
            let ids = &mut self.compiled_class_method_ids;
            let cid = match ids.get(md.class.as_str()) {
                Some(&id) => id,
                None => {
                    let next = ids.len() as u32;
                    *ids.entry((*md.class).clone()).or_insert(next)
                }
            };
            let mid = match ids.get(method) {
                Some(&id) => id,
                None => {
                    let next = ids.len() as u32;
                    *ids.entry(method.to_string()).or_insert(next)
                }
            };
            // class-perf loop-heat boost (measured): a body admitted by the
            // heat-based main gate may be fast-called even below the
            // call-count tier — without this the two gates diverge and a
            // heat-admitted method never reaches the VmFastEntry (falls back
            // to normal dispatch, which re-compiles it). Mirrors the main
            // gate's `loop_hot` read exactly.
            let loop_hot = method_boost_enabled()
                && self
                    .compiled_method_loop_heat
                    .get(&(cid, mid, spec_id))
                    .copied()
                    .unwrap_or(0)
                    >= tier as u64;
            if !loop_hot {
                let calls = self
                    .compiled_method_call_counts
                    .entry((cid, mid, spec_id))
                    .or_insert(0);
                if *calls < tier {
                    *calls += 1;
                    return None;
                }
            }
        }
        let Some(entry) = self.vm_fast_entry_for(&md.class, method, &md.method, spec_id) else {
            return None;
        };
        // Arity must match exactly — fewer actuals mean default-formal
        // binding, which only the interpreter path performs.
        if n != entry.coerce.len() {
            return None;
        }
        // Coerce the caller's argument registers (mirrors the binding
        // loop); a real actual needs real->int coercion — keep the
        // interpreter route.
        let mut args: Vec<Value> = Vec::with_capacity(n);
        for (i, (_, w, sgn)) in entry.coerce.iter().enumerate() {
            let Some(src) = self.vm_regs.get(arg_base + i) else {
                return None;
            };
            if src.is_real {
                return None;
            }
            let mut v = src.clone();
            if *w > 0 && v.width != *w {
                v = v.resize_for_assign(*w);
            }
            if let Some(s) = sgn {
                v.is_signed = *s;
            }
            args.push(v);
        }
        // --- every guard passed; state changes from here on ---
        self.push_queue_frame();
        self.this_stack.push(Some(handle));
        self.class_context_stack.push(Some((*md.class).clone()));
        // §8.25 (seeded callee spec): parity with the interpreter call
        // route — the direct dispatch skips exec_method_in_class_hierarchy's
        // seeding, so a callee of `C#(spec)` would resolve its bare static
        // members to the shared `C::member` cell while interpreter routes
        // use `C#spec::member` (silently split storage).
        let saved_spec = self.seed_callee_spec_from_instance(handle, &md.class);
        let mut locals = self.take_pooled_frame();
        for ((name, _, _), v) in entry.coerce.iter().zip(args.iter()) {
            locals.insert(name.to_string(), v.clone());
        }
        self.push_local_frame(locals);
        self.method_local_base.push(self.local_stack.len() - 1);
        self.local_iface_aliases.push(HashMap::default());
        let saved_m_scope = std::mem::take(&mut self.m_scope_stack);
        let saved_break = self.break_flag;
        let saved_continue = self.continue_flag;
        let saved_return = self.return_flag;
        let _prev_meth = if SAMPLER_READY.load(std::sync::atomic::Ordering::Relaxed) {
            sampler_register_name(fnv_name(method), method);
            Some(CUR_METHOD_HASH.swap(fnv_name(method), std::sync::atomic::Ordering::Relaxed))
        } else {
            None
        };
        let _prev_compiled = if SAMPLER_READY.load(std::sync::atomic::Ordering::Relaxed) {
            Some(CUR_METHOD_COMPILED.swap(true, std::sync::atomic::Ordering::Relaxed))
        } else {
            None
        };
        // Fresh register file for the callee (the caller's is swapped out,
        // exactly like `try_run_compiled_method`).
        let caller_regs = std::mem::take(&mut self.vm_regs);
        let saved_foreach_arena = std::mem::take(&mut self.foreach_arena);
        let mut regs = vec![Value::zero(1); entry.block.num_regs as usize];
        regs[entry.this_reg as usize] = Value::from_u64(handle as u64, 32);
        for (i, v) in args.into_iter().enumerate() {
            let reg = entry.this_reg as usize + 1 + i;
            if reg < entry.block.num_regs as usize {
                regs[reg] = v;
            }
        }
        if (entry.is_void_result || entry.is_class_result)
            && (entry.result_reg as usize) < entry.block.num_regs as usize
        {
            regs[entry.result_reg as usize] = entry.seed.clone();
        }
        self.vm_regs = regs;
        self.exec_insns(&entry.block.instructions);
        let result = self
            .vm_regs
            .get(entry.result_reg as usize)
            .cloned()
            .unwrap_or_else(|| Value::zero(32));
        self.vm_regs = caller_regs;
        self.foreach_arena = saved_foreach_arena;
        if let Some(prev) = _prev_meth {
            CUR_METHOD_HASH.store(prev, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(prev) = _prev_compiled {
            CUR_METHOD_COMPILED.store(prev, std::sync::atomic::Ordering::Relaxed);
        }
        self.m_scope_stack = saved_m_scope;
        self.break_flag = saved_break;
        self.continue_flag = saved_continue;
        self.return_flag = saved_return;
        self.method_local_base.pop();
        self.pop_local_frame();
        self.local_iface_aliases.pop();
        self.class_context_stack.pop();
        self.this_stack.pop();
        self.restore_spec_pooled(saved_spec);
        self.pop_and_restore_queue_frame();
        // Result clamping mirrors `try_run_compiled_method`'s tail.
        let mut result = result;
        if result.is_real && !entry.is_class_result && !entry.is_void_result {
            result = Self::compiled_ret_from_real(&result, entry.result_width);
        }
        if entry.result_width > 0 && result.width != entry.result_width {
            result = result.resize_for_assign(entry.result_width);
        }
        if !entry.is_class_result && !entry.is_void_result && !result.is_real {
            result.is_signed = entry.result_signed;
        }
        Some(result)
    }

    /// class-perf: wrap a runtime `Value` as a constant `Expression` for the
    /// interpreter's method dispatcher to bind as an actual. A non-real value
    /// becomes a sized hex integer whose cached bits round-trip exactly (the
    /// inline fast path reproduces `val`/`xz`/`width`; the text fallback keeps
    /// X/Z and width identical for >64-bit values). A real becomes a `Real`
    /// literal. This is only called from `CallMethod`'s handler, so the
    /// arg VALUES are already computed — the wrapper just lets the existing
    /// callee-binding code (defaults, ref/output write-backs, vif handling)
    /// run unchanged.
    pub(super) fn value_method_arg_expr(&self, v: &Value) -> Expression {
        use crate::ast::expr::NumberBase;
        use crate::ast::expr::NumberLiteral;
        if v.is_real {
            return Expression::new(
                ExprKind::Number(NumberLiteral::Real(v.to_f64())),
                crate::ast::Span::dummy(),
            );
        }
        // Step 8: an EMPTY string (width 0) has no hex form — the sized
        // integer below would coerce it to `1'd0` (width 1), and the callee's
        // binding would see a 1-bit value instead of an empty string. Emit a
        // string literal so eval_expr re-creates the exact width-0 bytes.
        if v.width == 0 {
            return Expression::new(
                ExprKind::StringLiteral(String::new()),
                crate::ast::Span::dummy(),
            );
        }
        let inline = v
            .inline_bits()
            .filter(|_| v.width <= 64)
            .map(|(vb, xz)| (vb, xz, v.width));
        Expression::new(
            ExprKind::Number(NumberLiteral::Integer {
                size: Some(v.width.max(1)),
                signed: v.is_signed,
                base: NumberBase::Hex,
                value: v.to_hex(),
                cached_val: std::cell::Cell::new(inline),
            }),
            crate::ast::Span::dummy(),
        )
    }

    /// class-perf row 2: argument positions of `class_name::method` whose
    /// formals are NON-INPUT plain scalars (output / inout / ref; integral,
    /// string, enum or class-handle typed, no dimensions) — exactly the set
    /// the compiled CALLEE path now admits. Used by the VM's call handlers
    /// to bind those actuals as frame temps instead of constants so the
    /// interpreter's write-back reaches the caller's arg registers.
    pub(super) fn vm_writeback_scalar_positions(
        &self,
        class_name: &str,
        method: &str,
    ) -> Vec<usize> {
        let Some(md) = self.method_defining_class(class_name, method) else {
            return Vec::new();
        };
        // §13.5: task and function formals bind the same way; a task callee
        // reached from compiled code (now that wait-free tasks compile) must
        // write its outputs back exactly like a function's.
        use crate::ast::decl::ClassMethodKind as K;
        let ports = match &md.method.kind {
            K::Function(f) | K::Extern(f) | K::PureVirtual(f) => &f.ports,
            K::Task(t) => &t.ports,
        };
        let mut pos = Vec::new();
        for (i, port) in ports.iter().enumerate() {
            if port.direction == PortDirection::Input || !port.dimensions.is_empty() {
                continue;
            }
            let ok = self.typeref_names_class(&port.data_type)
                || Self::is_string_data_type(&port.data_type)
                || self.enum_type_info(&port.data_type).is_some()
                || self.scalar_formal_integral(&port.data_type).is_some();
            if ok {
                pos.push(i);
            }
        }
        pos
    }

    /// class-perf row 2: build the actual-expressions for a VM method call.
    /// Default is the historic value-wrapper constant. For argument
    /// positions whose formal is a non-INPUT plain scalar, the actual is
    /// instead a frame TEMP `\0vmargN` (a name no SystemVerilog identifier
    /// can collide with) inserted into the CURRENT — the compiled caller's
    /// — locals frame holding the register's value: the interpreter's
    /// binding loop evaluates the Ident (exact Value, like any local), and
    /// its epilogue `assign_value` writes the temp (frame-local store,
    /// resized to the temp's seeded width — the caller local's width).
    /// Non-scalar shapes (queues/arrays/structs, identity-bound semantics)
    /// keep the constant wrapper, byte-identical to the old behavior.
    pub(super) fn vm_args_with_writeback(
        &mut self,
        class_name: &str,
        method: &str,
        base: usize,
        n: usize,
    ) -> (Vec<Expression>, bool) {
        let pos = self.vm_writeback_scalar_positions(class_name, method);
        if pos.is_empty() || self.local_stack.is_empty() {
            let mut args = Vec::with_capacity(n);
            for i in 0..n {
                let v = self
                    .vm_regs
                    .get(base + i)
                    .cloned()
                    .unwrap_or_else(|| Value::zero(32));
                args.push(self.value_method_arg_expr(&v));
            }
            return (args, false);
        }
        let mut args = Vec::with_capacity(n);
        let mut any_temp = false;
        for i in 0..n {
            let v = self
                .vm_regs
                .get(base + i)
                .cloned()
                .unwrap_or_else(|| Value::zero(32));
            if pos.contains(&i) {
                let name = format!("\u{0}vmarg{i}");
                let span = crate::ast::Span::dummy();
                let expr = Expression::new(
                    ExprKind::Ident(HierarchicalIdentifier {
                        root: None,
                        path: vec![HierPathSegment {
                            name: crate::ast::Identifier {
                                name: name.clone(),
                                span,
                            },
                            selects: Vec::new(),
                        }],
                        span,
                        cached_signal_id: std::cell::Cell::new(None),
                        cached_resolved_name: std::cell::OnceCell::new(),
                    }),
                    span,
                );
                if let Some(fr) = self.local_stack.last_mut() {
                    fr.insert(name, v);
                    any_temp = true;
                    args.push(expr);
                    continue;
                }
            }
            args.push(self.value_method_arg_expr(&v));
        }
        (args, any_temp)
    }

    /// class-perf row 2: after a VM call whose actuals included frame temps,
    /// copy each temp's final value back into its arg register (any change
    /// can only come from the interpreter's non-input write-back — input
    /// formals are callee-frame locals and cannot touch the caller's temp)
    /// and remove the temps from the frame.
    pub(super) fn vm_drain_arg_temps(&mut self, base: usize, n: usize) {
        if self.local_stack.is_empty() {
            return;
        }
        for i in 0..n {
            let name = format!("\u{0}vmarg{i}");
            let picked = self.local_stack.last_mut().and_then(|fr| fr.remove(&name));
            if let Some(v) = picked {
                if let Some(slot) = self.vm_regs.get_mut(base + i) {
                    *slot = v;
                }
            }
        }
    }

    /// Return property type names, including ENUM type names as well as class
    /// handles). Used to give `field.name()` the correct enum type — e.g. an
    /// instruction's `rd` (riscv_reg_t) must reflect against the register enum,
    /// not the largest enum in the design (riscv_instr_name_t).
    /// Resolve a receiver name to its container (mailbox/semaphore) handle.
    /// Checks the signal/local table first, then — for a bare class-member
    /// name accessed inside a method — the current `this` instance's property
    /// map. Returns the raw handle value (caller checks mailboxes/semaphores).
    pub(super) fn resolve_container_handle(&self, obj_name: &str) -> Option<usize> {
        if let Some(v) = self.get_signal_value_by_name(obj_name) {
            return Some(v.to_u64().unwrap_or(0) as usize);
        }
        let prop = obj_name.strip_prefix("this.").unwrap_or(obj_name);
        if !prop.contains('.') {
            if let Some(Some(th)) = self.this_stack.last().copied() {
                if let Some(Some(inst)) = self.heap.get(th) {
                    if let Some(v) = inst.properties.get(prop) {
                        return Some(v.to_u64().unwrap_or(0) as usize);
                    }
                }
            }
        }
        None
    }

    /// Strip parameterization/whitespace from a declared type string and
    /// return "mailbox"/"semaphore" if that is the base type, else None.
    /// `"mailbox #(REQ)"`, `"mailbox#(T)"`, `"mailbox"` -> Some("mailbox").
    pub(super) fn container_base(t: &str) -> Option<&'static str> {
        let base = t.split(['#', ' ', '(']).next().unwrap_or("").trim();
        match base {
            "mailbox" => Some("mailbox"),
            "semaphore" => Some("semaphore"),
            _ => None,
        }
    }

    /// §15.3/§15.4: if `lvalue`'s declared type is a (possibly parameterized)
    /// mailbox/semaphore, return "mailbox"/"semaphore". Consults the local
    /// container-decl map and the class-property type via the current class
    /// context FIRST (both reliable), falling back to get_expr_type_name
    /// (which only serves module-scope container signals correctly).
    /// Container kind ("mailbox"/"semaphore") of property `member` on the
    /// runtime class of the object `handle` refers to, walking the inheritance
    /// chain. An ARRAY property carries no `type_name` on its signal, so the
    /// declared type is consulted as well.
    pub(super) fn handle_prop_container_kind(
        &self,
        handle: usize,
        member: &str,
    ) -> Option<&'static str> {
        if handle == 0 {
            return None;
        }
        let mut cur = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone());
        while let Some(c) = cur {
            let Some(cd) = self.module.classes.get(&c) else {
                break;
            };
            if let Some(k) = cd
                .properties
                .get(member)
                .and_then(|s| s.type_name.as_deref())
                .and_then(Self::container_base)
            {
                return Some(k);
            }
            if let Some(crate::ast::types::DataType::TypeReference { name, .. }) =
                cd.property_types.get(member)
                && let Some(k) = Self::container_base(&name.name.name)
            {
                return Some(k);
            }
            cur = cd.extends.clone();
        }
        None
    }

    pub(super) fn lvalue_container_kind(&self, lvalue: &Expression) -> Option<&'static str> {
        // §15.3/§15.4: `mb[i] = new()` — an ELEMENT of a mailbox/semaphore
        // ARRAY has the ARRAY's container type. Neither the local map nor
        // `get_expr_type_name` resolves an INDEXED lvalue, so the construction
        // was not recognised as a container and no mailbox was ever registered
        // for the handle it went on to store. Reads then found a live-looking
        // (non-null) handle with nothing behind it, so every `put` silently
        // vanished and `num()`/`try_get` saw an empty box — while the same
        // mailbox declared as a scalar in the same scope worked, which is what
        // made this look like a scope bug rather than an indexing one.
        // Recursion terminates: an Index yields its base, and stripping the
        // selects yields an Ident that matches neither arm.
        match &lvalue.kind {
            ExprKind::Index { expr, .. } => {
                // The ARRAY's own declared type is the element's type. Neither
                // the array nor its elements carry a `type_name` on their
                // signals (only scalars do), so consult the declaration table
                // — keyed bare and scope-qualified.
                if let ExprKind::Ident(h) = &expr.kind {
                    let full = self.resolve_hier_name(h);
                    let leaf = h
                        .path
                        .last()
                        .map(|s| s.name.name.clone())
                        .unwrap_or_default();
                    for key in [full.as_ref(), leaf.as_str()] {
                        if let Some(crate::ast::types::DataType::TypeReference { name, .. }) =
                            self.module.var_decl_types.get(key)
                            && let Some(k) = Self::container_base(&name.name.name)
                        {
                            return Some(k);
                        }
                    }
                }
                if let Some(k) = self.lvalue_container_kind(expr) {
                    return Some(k);
                }
            }
            ExprKind::Ident(h) if h.path.len() == 1 && !h.path[0].selects.is_empty() => {
                let mut seg = h.path[0].clone();
                seg.selects.clear();
                let bare = HierarchicalIdentifier {
                    root: h.root.clone(),
                    path: vec![seg],
                    span: h.span,
                    cached_signal_id: std::cell::Cell::new(None),
                    cached_resolved_name: std::cell::OnceCell::new(),
                };
                let e = Expression::new(ExprKind::Ident(bare), lvalue.span);
                if let Some(k) = self.lvalue_container_kind(&e) {
                    return Some(k);
                }
            }
            // `h.mb[i] = new()` — the collection is a PROPERTY reached through
            // a class HANDLE from outside the class, so the `this.` arm below
            // does not apply and an array property carries no `type_name`.
            // Read the declared type off the receiver's runtime class.
            // Both spellings occur: a MemberAccess node, and the FLATTENED
            // multi-segment Ident the parser produces for `u.h.mb`.
            ExprKind::MemberAccess { expr: recv, member } => {
                if let ExprKind::Ident(rh) = &recv.kind
                    && let Some(hnd) = self.eval_ident_handle(&self.resolve_hier_name(rh))
                    && let Some(k) = self.handle_prop_container_kind(hnd, &member.name)
                {
                    return Some(k);
                }
            }
            ExprKind::Ident(h) if h.path.len() >= 2 => {
                let member = h.path[h.path.len() - 1].name.name.clone();
                let head = h.path[..h.path.len() - 1]
                    .iter()
                    .map(|s| s.name.name.as_str())
                    .collect::<Vec<_>>()
                    .join(".");
                if let Some(hnd) = self.eval_ident_handle(&head)
                    && let Some(k) = self.handle_prop_container_kind(hnd, &member)
                {
                    return Some(k);
                }
            }
            _ => {}
        }
        let prop: Option<String> = match &lvalue.kind {
            ExprKind::Ident(h) if h.path.len() == 1 => Some(h.path[0].name.name.clone()),
            ExprKind::MemberAccess { expr, member } if matches!(expr.kind, ExprKind::This) => {
                Some(member.name.clone())
            }
            _ => None,
        };
        if let Some(pn) = &prop {
            // 1. tracked local (procedural `mailbox m;`).
            if let Some(k) = self.var_container_types.get(pn) {
                return Self::container_base(k);
            }
            // 2. class property of the current class (walk the hierarchy).
            if let Some(Some(ctx)) = self.class_context_stack.last().cloned() {
                let mut cur = Some(ctx);
                while let Some(cn) = cur {
                    if let Some(cd) = self.module.classes.get(&cn) {
                        if let Some(sig) = cd.properties.get(pn) {
                            if let Some(k) = sig.type_name.as_deref().and_then(Self::container_base)
                            {
                                return Some(k);
                            }
                            // An ARRAY property carries no `type_name` — only
                            // scalars do — so `mailbox mb[2];` as a class member
                            // looked like a non-container and `mb[i] = new()`
                            // inside a method never allocated. Fall back to the
                            // declared type before giving up.
                            if let Some(crate::ast::types::DataType::TypeReference { name, .. }) =
                                cd.property_types.get(pn)
                                && let Some(k) = Self::container_base(&name.name.name)
                            {
                                return Some(k);
                            }
                            return None;
                        }
                        cur = cd.extends.clone();
                    } else {
                        break;
                    }
                }
            }
        }
        // 3. module-scope signal (get_expr_type_name returns "mailbox" here).
        self.get_expr_type_name(lvalue)
            .as_deref()
            .and_then(Self::container_base)
    }

    /// §7.2/§8.4: the class a CLASS-HANDLE member of an unpacked struct
    /// variable (`s.h`, `s.inner.h`) names, walked from the variable's
    /// declared type. The struct's leaves carry no type name of their own,
    /// so `s.h = new` found no class to construct and left `s.h` null.
    pub(super) fn struct_member_class_type(
        &self,
        root: &str,
        members: &[String],
    ) -> Option<String> {
        let mut dt = self.module.var_decl_types.get(root)?;
        for m in members {
            let DataType::Struct(su) = self.resolve_dt_ref(dt) else {
                return None;
            };
            dt = &su
                .members
                .iter()
                .find(|sm| sm.declarators.iter().any(|d| &d.name.name == m))?
                .data_type;
        }
        let DataType::TypeReference { name, .. } = dt else {
            return None;
        };
        let tn = &name.name.name;
        if self.module.classes.contains_key(tn) {
            return Some(tn.clone());
        }
        self.resolve_simple_typedef_class(tn)
    }

    pub(super) fn lookup_type_member(&self, type_name: &str, member: &str) -> Option<String> {
        if let Some(t) = self.class_prop_type_named(type_name, member) {
            return Some(t);
        }
        if let Some(dt) = self.module.typedef_types.get(type_name).cloned() {
            let resolved = Self::resolve_type_ref(&dt, &self.module.typedef_types);
            if let DataType::Struct(su) = resolved {
                for m in &su.members {
                    for d in &m.declarators {
                        if d.name.name == member {
                            if let DataType::TypeReference { name, .. } = &m.data_type {
                                return Some(name.name.name.clone());
                            }
                            if let DataType::TypeReference { name, .. } =
                                self.resolve_dt(&m.data_type)
                            {
                                return Some(name.name.name.clone());
                            }
                        }
                    }
                }
            }
        }
        None
    }

    pub(super) fn class_prop_type_named(&self, class_name: &str, prop: &str) -> Option<String> {
        let owners = self.prop_owners(class_name);
        {
            if let Some((_, cd)) = owners.get(prop) {
                if let Some(sig) = cd.properties.get(prop) {
                    if let Some(t) = &sig.type_name {
                        if self.module.classes.contains_key(t)
                            || self.module.enum_members.contains_key(t)
                            // A covergroup-typed property (the implicit
                            // variable a class-body covergroup declares).
                            || self.module.covergroups.contains_key(t)
                            // §6.20.3: a TYPE-parameter name passes through —
                            // the caller resolves it to the bound concrete
                            // type via the instance's type_bindings.
                            || cd.type_param_names.iter().any(|p| p == t)
                        {
                            return Some(t.clone());
                        }
                        // A class-local typedef (e.g. `this_type` =
                        // `Self#(T)`): resolve it to the concrete class so a
                        // `static this_type m_inst; m_inst = new()` property
                        // constructs the right class. `resolve_typeref_class_name`
                        // is context-aware (resolves `this_type` to the
                        // enclosing class). Without this, the typedef name
                        // matched none of the cases above, returned None, and
                        // `new()` fell through to a wrong/empty construction.
                        let tn = crate::ast::types::TypeName {
                            scopes: Vec::new(),
                            name: crate::ast::Identifier {
                                name: t.clone(),
                                span: crate::ast::Span::dummy(),
                            },
                            span: crate::ast::Span::dummy(),
                        };
                        if let Some(resolved) = self.resolve_typeref_class_name(&tn) {
                            return Some(resolved);
                        }
                    }
                    return None;
                }
            }
        }
        None
    }

    /// Reverse-map an enum integer value to its member name. With a known enum
    /// type, looks it up directly; otherwise searches all enum tables for a
    /// member with that value, preferring the largest enum (which, for riscv-dv,
    /// is the instruction-name enum the lookup is almost always for).
    /// Root collection name of a `foreach` target (`a`, `a[i]`, `obj.a`).
    pub(super) fn foreach_array_root_name(expr: &Expression) -> Option<String> {
        match &expr.kind {
            ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.clone()),
            ExprKind::Index { expr: base, .. } => Self::foreach_array_root_name(base),
            ExprKind::MemberAccess { member, .. } => Some(member.name.clone()),
            _ => None,
        }
    }

    /// Find an enum value name with the receiver's own storage names as extra list
    /// keys: an ANONYMOUS enum (or an inline-enum local) has no typedef, so
    /// its member list is keyed by the VARIABLE name (§6.19.6) — the
    /// `first/last/num` path already consulted it, `.name()` did not and
    /// fell to the design-wide by-value scan, which picks whichever enum of
    /// the largest size happens to hold the value.
    pub(super) fn enum_value_name_keyed(
        &self,
        value: u64,
        type_hint: Option<&str>,
        var_keys: &[String],
    ) -> Option<String> {
        let by_key = |k: &str| -> Option<String> {
            self.module.enum_members.get(k).and_then(|members| {
                members
                    .iter()
                    .find(|(_, v)| *v == value)
                    .map(|(n, _)| n.clone())
            })
        };
        if let Some(t) = type_hint {
            if let Some(n) = by_key(t) {
                return Some(n);
            }
        }
        for k in var_keys {
            if let Some(n) = by_key(k) {
                return Some(n);
            }
        }
        let mut best: Option<(usize, String)> = None;
        for members in self.module.enum_members.values() {
            for (n, v) in members {
                if *v == value {
                    let sz = members.len();
                    if best.as_ref().is_none_or(|(bsz, _)| sz > *bsz) {
                        best = Some((sz, n.clone()));
                    }
                }
            }
        }
        best.map(|(_, n)| n)
    }

    /// Is `e` STATICALLY a class-handle expression? Guards the §8.12
    /// copy-constructor reading of `new <src>`: construction vs copy is
    /// decided by the operand's DECLARED type, never its runtime value —
    /// an int arg (`new(i)`) whose value collides with a live heap index
    /// must construct, not shallow-copy heap object #i.
    pub(super) fn expr_is_class_handle(&self, e: &Expression) -> bool {
        if matches!(e.kind, ExprKind::This) {
            return true;
        }
        self.get_expr_type_name(e).is_some_and(|t| {
            let t = self.resolve_type_param_binding(&t).unwrap_or(t);
            self.module.classes.contains_key(&t)
        })
    }

    /// True when `expr`'s DECLARED type is a mailbox, whatever its current
    /// value is. Used to tell "null mailbox" apart from "not a mailbox at
    /// all": only the former is a fault worth reporting, and a wrong guess
    /// would abort a run over an ordinary user method named `get`.
    /// Deliberately conservative — an undeterminable type answers false.
    pub(super) fn is_declared_mailbox(&self, expr: &Expression) -> bool {
        let says_mailbox = |t: &str| t.starts_with("mailbox") || t == "mailbox";
        if let ExprKind::Ident(hier) = &expr.kind {
            let name = self.resolve_hier_name(hier);
            if self
                .var_container_types
                .get(&*name)
                .is_some_and(|k| says_mailbox(k))
            {
                return true;
            }
            // A class property keeps its declared type on the class, not on
            // any signal — `local mailbox m;` inside a driver reaches here.
            if let Some(leaf) = hier.path.last().map(|s| s.name.name.as_str()) {
                if let Some(Some(cur)) = self.class_context_stack.last() {
                    if self
                        .module
                        .classes
                        .get(cur)
                        .and_then(|cd| cd.property_types.get(leaf))
                        .and_then(|dt| match dt {
                            DataType::TypeReference { name, .. } => Some(name.name.name.clone()),
                            _ => None,
                        })
                        .is_some_and(|t| says_mailbox(&t))
                    {
                        return true;
                    }
                }
            }
        }
        self.get_expr_type_name(expr)
            .is_some_and(|t| says_mailbox(&t))
    }

    /// Element class of a bare module-scoped / package-scoped unpacked
    /// collection of a class type, resolved from the variable's DECLARED
    /// DataType. `var_decl_types` is keyed by the BASE name and stores the
    /// ELEMENT type (for `K m [string]` it holds the `TypeReference` "K"), so
    /// a `= new` assigning into an element slot can construct the right class.
    ///
    /// Without this the ARRAY-element type maps
    /// `array_elem_class` / `var_class_types` are keyed only for ranged / local
    /// arrays and class-member collections; a MODULE/PACKAGE-scope assoc
    /// array of a class (e.g. UVM's `uvm_seed_map uvm_random_seed_table_lookup
    /// [string]`) has no entry, `elem_cls` resolved the element type as unknown,
    /// and `uvm_random_seed_table_lookup[inst_id] = new;` fell through to an
    /// OPAQUE "unknown-LHS" instance, so the stored handle deref'd null the
    /// moment a seed was read back (`seed_map.seed_table` null deref at t=0).
    pub(super) fn declared_collection_elem_class(&self, name: &str) -> Option<String> {
        // Package-scoped collections (e.g. UVM's
        // `uvm_seed_map uvm_random_seed_table_lookup [string]` inside
        // `package uvm_pkg`) may be keyed under a QUALIFIED name
        // (`uvm_pkg::uvm_random_seed_table_lookup`). Try the bare name
        // first, then a leaf-name match so a bare in-scope reference finds
        // its declared type either way.
        //
        // The leaf-name match goes through a ONE-TIME index
        // (`var_decl_leaf_index`): the id-collision-in-heap gates call
        // here on every member/indexed access, and scanning the whole
        // `var_decl_types` map with an `rsplit(':')` per key was ~5.5ms
        // per access on a 370K-signal SoC — 96% of the run's CPU. The
        // index only PROPOSES candidate keys; the live map stays
        // authoritative, so runtime insertions (formal / foreach
        // metadata, always keyed by the exact bare name the direct probe
        // above already covers) cannot be missed, and a key removed since
        // the index was built (`clear_formal_metadata`) simply fails the
        // live get and is skipped, exactly like a key the old scan never
        // saw. The first candidate still present decides the answer, as
        // the old scan's first leaf match did.
        if let Some(dt) = self.module.var_decl_types.get(name) {
            return self.declared_elem_class_of_dt(dt);
        }
        let leaf = name.rsplit(':').next().unwrap_or(name);
        let candidates = self.var_decl_leaf_index().get(leaf)?;
        for key in candidates {
            if let Some(dt) = self.module.var_decl_types.get(key) {
                return self.declared_elem_class_of_dt(dt);
            }
        }
        None
    }

    /// Resolution half of `declared_collection_elem_class`: a
    /// `TypeReference` DataType naming a registered class or covergroup.
    pub(super) fn declared_elem_class_of_dt(&self, dt: &DataType) -> Option<String> {
        let cn = match dt {
            DataType::TypeReference { name, .. } => name.name.name.clone(),
            _ => return None,
        };
        if self.module.classes.contains_key(&cn) || self.module.covergroups.contains_key(&cn) {
            Some(cn)
        } else {
            None
        }
    }

    /// Storage names under which an enum receiver's member list may be keyed
    /// when it has no typedef: its resolved name and its bare leaf.
    pub(super) fn enum_receiver_keys(&self, expr: &Expression) -> Vec<String> {
        let mut keys = Vec::new();
        if let ExprKind::Ident(h) = &expr.kind {
            keys.push(self.resolve_hier_name(h).into_owned());
            if let Some(leaf) = h.path.last() {
                if h.path.len() > 1 || keys[0] != leaf.name.name {
                    keys.push(leaf.name.name.clone());
                }
            }
        }
        keys
    }
}
