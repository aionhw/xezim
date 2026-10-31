//! Simulator class members.

use super::*;

impl Simulator {
    pub(super) fn class_static_set(&mut self, start_class: &str, prop: &str, val: Value) -> bool {
        if let Some(key) = self.static_prop_key(start_class, prop) {
            let val = self.fit_static_cell(&key, prop, val);
            self.class_statics.insert(key, val);
            true
        } else {
            false
        }
    }

    /// If `name` is a bare identifier naming an associative-array property
    /// of the current class context (and `this` is a live handle), return
    /// the instance-scoped storage name `<handle>#<name>`.
    /// §7.4.2: the flat storage name of an N-D fixed-array CLASS-property
    /// element — `obj.m[i][j]` → `<handle>#m[i][j]`, any depth, `this`/bare
    /// receivers included. Every per-element cell exists (construction seeds
    /// the whole shape), but no read or write path resolved the chain: the
    /// property fell through to a packed bit-select of its unused scalar cell,
    /// so `[0][0]` read a stray bit and everything else read x — while the 1-D
    /// `obj.m[i]` beside it worked (`expr_assoc_name` covers 1-D).
    pub(super) fn class_nd_elem_name(
        &mut self,
        base: &Expression,
        outer_index: &Expression,
    ) -> Option<String> {
        if self.no_class_objects() {
            return None;
        }
        // Collect the index expressions without evaluating them. This probe
        // runs for every indexed read/write while class objects exist, and
        // must not consume a selector's side effects before establishing
        // that the receiver really owns an N-D fixed-array property (§7.4.2).
        let mut idxs: Vec<&Expression> = vec![outer_index];
        let mut cur = base;
        while let ExprKind::Index { expr: b, index } = &cur.kind {
            idxs.push(index);
            cur = b;
        }
        // The receiver: `obj.m` / `this.m` (MemberAccess or 2-seg Ident), or a
        // bare `m` inside a method.
        let (handle, member) = match &cur.kind {
            ExprKind::MemberAccess { expr: recv, member } => {
                let h = match &recv.kind {
                    ExprKind::This => self.this_stack.last().copied().flatten()?,
                    _ => self.eval_expr(recv).to_u64().map(|h| h as usize)?,
                };
                (h, member.name.clone())
            }
            // Elaboration flattens a member chain to a MULTI-SEGMENT Ident
            // (`o.sub.g` arrives as one 3-segment path, not nested
            // MemberAccess), so a receiver behind more than one handle hop
            // fell to the `_` arm below and every chained element access
            // read x / wrote nowhere. Walk the middle segments through heap
            // properties; for two segments the loop is empty and this is the
            // old arm exactly.
            ExprKind::Ident(h)
                if h.path.len() >= 2 && h.path.iter().all(|s| s.selects.is_empty()) =>
            {
                // 3+ segments: gate on the leaf naming SOME class's collection
                // member before any handle evaluation — this arm sits in the
                // hot indexed-read path, and an ungated chain attempt on every
                // hierarchical `top.dut.arr[i]` eval cost +0.4% instructions
                // on an axi4 UVM run.
                if h.path.len() > 2
                    && !self
                        .chained_coll_members()
                        .contains(h.path.last()?.name.name.as_str())
                {
                    return None;
                }
                let obj = &h.path[0].name.name;
                let mut hh = if obj == "this" || obj == "super" {
                    self.this_stack.last().copied().flatten()?
                } else {
                    self.eval_ident_handle(obj)?
                };
                for seg in &h.path[1..h.path.len() - 1] {
                    if hh == 0 {
                        return None;
                    }
                    let inst = self.heap.get(hh).and_then(|o| o.as_ref())?;
                    hh = inst
                        .properties
                        .get(&seg.name.name)
                        .and_then(|v| v.to_u64())? as usize;
                }
                (hh, h.path.last()?.name.name.clone())
            }
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                let hh = self.this_stack.last().copied().flatten()?;
                (hh, h.path[0].name.name.clone())
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
        let mut cur_cn = Some(cn);
        let mut found = false;
        while let Some(c) = cur_cn {
            let cd = self.module.classes.get(&c)?;
            if let Some((shape, _)) = cd.array_nd_properties.get(&member) {
                if shape.len() != idxs.len() {
                    return None;
                }
                found = true;
                break;
            }
            cur_cn = cd.extends.clone();
        }
        if !found {
            return None;
        }
        let mut name = format!("{}#{}", handle, member);
        for index in idxs.iter().rev() {
            let i = self.eval_expr(index).to_i64()?;
            name.push_str(&format!("[{}]", i));
        }
        Some(name)
    }

    /// §8.9: the store key of a fixed-size STATIC array member reachable
    /// from `class_name`'s inheritance chain — `{DeclaringClass}::{member}`,
    /// nearest declarer first (a subclass redeclaring the name hides the
    /// base's copy; an inherited-only member resolves to the base's key, so
    /// every subclass shares the ONE store §8.9 prescribes).
    pub(super) fn static_fixed_key_in(&self, class_name: &str, member: &str) -> Option<String> {
        let mut cur: Option<&str> = Some(class_name);
        while let Some(cn) = cur {
            let cd = self.module.classes.get(cn)?;
            if cd.static_fixed_arrays.iter().any(|(n, ..)| n == member) {
                return Some(format!("{}::{}", cn, member));
            }
            cur = cd.extends.as_deref();
        }
        None
    }

    /// §8.9: a source-spelled `Cls::member` (or the flattened `Cls.member`
    /// an inlined path can produce) naming a fixed-size static array,
    /// resolved through Cls's chain to the DECLARING class's store key —
    /// `C::B` inherited from `Base` maps to `Base::B`.
    /// §8.9: hier-shape-aware variant — `Cls::member` may reach resolution
    /// as a TWO-SEGMENT Ident whose leaf the generic resolver collapses to
    /// (`resolve_hier_name([C, R])` yields bare "R"), losing the class. Map
    /// from the ORIGINAL segments instead.
    pub(super) fn static_fixed_key_from_hier(
        &self,
        hier: &crate::ast::expr::HierarchicalIdentifier,
    ) -> Option<String> {
        match hier.path.len() {
            1 => self.static_fixed_key_for_name(&hier.path[0].name.name),
            2 if hier.path[0].selects.is_empty() => {
                let cls = &hier.path[0].name.name;
                if !self.module.classes.contains_key(cls) {
                    return None;
                }
                self.static_fixed_key_in(cls, &hier.path[1].name.name)
            }
            _ => None,
        }
    }

    pub(super) fn static_fixed_key_for_name(&self, name: &str) -> Option<String> {
        // Fast reject before building any pattern searcher: both shapes below
        // need a `::` or `.` separator, and the overwhelming majority of calls
        // pass a bare identifier. This runs under `instance_assoc_member`
        // (22.3M calls on an axi4Lite UVM run) where the two `split_once`
        // searchers cost ~3% of the whole run.
        if !name.as_bytes().iter().any(|&b| b == b':' || b == b'.') {
            return None;
        }
        let (cls, member) = name.split_once("::").or_else(|| name.split_once('.'))?;
        if member.is_empty() || member.contains(':') || member.contains('.') {
            return None;
        }
        if !self.module.classes.contains_key(cls) {
            return None;
        }
        self.static_fixed_key_in(cls, member)
    }

    /// Does `cls` REDECLARE `member` as a STATIC collection — i.e. declare it
    /// in its OWN body while an ANCESTOR also declares it? Used to distinguish
    /// a derived class that restates a base's static collection (gets its own
    /// §8.9 cell) from a class that merely INHERITS it (shares the ancestor's
    /// cell) or that is the SOLE declarer (keeps the bare store). A sole
    /// declarer must NOT report true — routing its per-class key would split
    /// storage that registration keeps bare.
    pub(super) fn class_declares_coll(&self, cls: &str, member: &str) -> bool {
        let owns = self
            .module
            .classes
            .get(cls)
            .map(|cd| cd.static_collections.iter().any(|(n, _, _)| n == member))
            .unwrap_or(false);
        if !owns {
            return false;
        }
        // Only a RESTATE counts: some ancestor must ALSO declare it.
        let mut anc = self
            .module
            .classes
            .get(cls)
            .and_then(|cd| cd.extends.clone());
        while let Some(cn) = anc {
            let cd = self.module.classes.get(&cn);
            match cd {
                Some(d) => {
                    if d.static_collections.iter().any(|(n, _, _)| n == member) {
                        return true;
                    }
                    anc = d.extends.clone();
                }
                None => break,
            }
        }
        false
    }

    /// Is `member` a STATIC queue/assoc/dynamic-array collection property of
    /// `start_class` or an ancestor (i.e. it is in `static_collections`)?
    pub(super) fn member_is_static_coll(&self, start_class: &str, member: &str) -> bool {
        let mut cur: Option<&str> = Some(start_class);
        while let Some(cn) = cur {
            if let Some(cd) = self.module.classes.get(cn) {
                if cd.static_collections.iter().any(|(n, _, _)| n == member) {
                    return true;
                }
                cur = cd.extends.as_deref();
            } else {
                break;
            }
        }
        false
    }

    /// A block-local non-string declaration clears a stale string flag for
    /// its bare name. Inside a subroutine frame the removal is logged so the
    /// frame's exit re-inserts it — an enclosing scope's `string m` must not
    /// be clobbered by a nested `int m` (§6.21/§23.8). Outside any frame
    /// (process scope) it is a plain removal, as before.
    pub(super) fn forget_string_flag(&mut self, name: &str) {
        if self.string_signals.remove(name) {
            if let Some(log) = self.string_signals_removed.last_mut() {
                log.push(name.to_string());
            }
        }
        // A non-string local declared in a task's own frame shadows a
        // same-named string formal for the rest of that frame.
        if let (Some(entry), Some(top)) = (
            self.task_string_formals.last_mut(),
            self.local_gen_stack.last(),
        ) {
            if entry.fgen == *top {
                entry.strings.retain(|n| n != name);
            }
        }
    }

    pub(super) fn record_frame_string_kind(&mut self, name: &str, is_string: bool) {
        let Some(&fgen) = self.local_gen_stack.last() else {
            return;
        };
        let pos = match self
            .task_string_formals
            .iter()
            .rposition(|e| e.fgen == fgen)
        {
            Some(pos) => pos,
            None => {
                self.task_string_formals.push(TaskFrameStrings {
                    fgen,
                    strings: Vec::new(),
                    others: Vec::new(),
                });
                self.task_string_formals.len() - 1
            }
        };
        let e = &mut self.task_string_formals[pos];
        let (keep, remove) = if is_string {
            (&mut e.strings, &mut e.others)
        } else {
            (&mut e.others, &mut e.strings)
        };
        remove.retain(|n| n != name);
        if !keep.iter().any(|n| n == name) {
            keep.push(name.to_string());
        }
    }

    /// Resolve this formal's string kind without discarding an explicit
    /// typedef scope or a declaring alias's lexical scope.
    pub(super) fn frame_formal_is_string(&self, dt: &DataType) -> bool {
        self.frame_formal_is_string_in(dt, "")
    }

    pub(super) fn frame_formal_is_string_in(&self, dt: &DataType, scope: &str) -> bool {
        if scope.is_empty() && self.plain_class_ref(dt).is_some() {
            return false;
        }
        let mut current = dt;
        let mut prefix = scope.to_string();
        let mut seen = Vec::<String>::new();
        for _ in 0..16 {
            if Self::is_string_data_type(current) {
                return true;
            }
            let DataType::TypeReference { name, .. } = current else {
                return false;
            };
            let leaf = name.name.name.as_str();
            let scoped = name.has_scope();
            let lexical = (!scoped && !prefix.is_empty()).then(|| format!("{prefix}::{leaf}"));
            let lexical = lexical.filter(|key| self.module.typedef_types.contains_key(key));
            let key = if scoped {
                name.qualified()
            } else if let Some(key) = lexical {
                key
            } else {
                // A bare runtime type parameter wins over a same-named
                // global alias. A target reached through a package alias
                // instead keeps the package's lexical type lookup.
                let parameter_site = prefix.is_empty()
                    || self
                        .module
                        .classes
                        .get(&prefix)
                        .is_some_and(|cd| cd.type_param_names.iter().any(|n| n == leaf));
                if parameter_site {
                    let bound = self.resolve_type_param_binding(leaf).or_else(|| {
                        let h = self.this_stack.last().copied().flatten()?;
                        self.heap.get(h)?.as_ref()?.type_bindings.get(leaf).cloned()
                    });
                    if let Some(bound) = bound {
                        if bound == "string" {
                            return true;
                        }
                        if seen.contains(&bound) {
                            return false;
                        }
                        seen.push(bound.clone());
                        let Some(next) = self.module.typedef_types.get(&bound) else {
                            return false;
                        };
                        prefix = bound
                            .rsplit_once("::")
                            .map_or(String::new(), |(p, _)| p.to_string());
                        current = next;
                        continue;
                    }
                }
                leaf.to_string()
            };
            if seen.contains(&key) {
                return false;
            }
            seen.push(key.clone());
            // lookup_scoped_type permits a leaf fallback. An explicitly
            // qualified declaration must not bind an unrelated leaf alias.
            let Some(next) = self.module.typedef_types.get(&key) else {
                return false;
            };
            prefix = key
                .rsplit_once("::")
                .map_or(String::new(), |(p, _)| p.to_string());
            current = next;
        }
        false
    }

    /// Reclassify a bound blocking method task after its callee context is
    /// active. Replace the entry already owned by TaskCleanup; do not push
    /// another entry or change its teardown ownership.
    pub(super) fn refresh_bound_task_string_kinds(
        &mut self,
        ports: &[crate::ast::decl::FunctionPort],
    ) {
        let Some(&fgen) = self.local_gen_stack.last() else {
            return;
        };
        let Some(pos) = self
            .task_string_formals
            .iter()
            .rposition(|e| e.fgen == fgen)
        else {
            return;
        };
        if let Some(entry) = self.task_frame_strings(fgen, ports) {
            self.task_string_formals[pos] = entry;
        }
    }

    /// The `task_string_formals` entry for a task frame of generation `fgen`
    /// with formals `ports`, or `None` when no formal needs one.
    pub(super) fn task_frame_strings(
        &self,
        fgen: u64,
        ports: &[crate::ast::decl::FunctionPort],
    ) -> Option<TaskFrameStrings> {
        let mut strings = Vec::new();
        let mut others = Vec::new();
        for port in ports {
            let n = &port.name.name;
            if self.frame_formal_is_string(&port.data_type) {
                strings.push(n.clone());
            } else {
                // A different process may declare a string of this name later.
                others.push(n.clone());
            }
        }
        (!strings.is_empty() || !others.is_empty()).then_some(TaskFrameStrings {
            fgen,
            strings,
            others,
        })
    }

    /// Does the frame-local or design name `name` hold a `string`? A task
    /// frame's own formals decide for the names they declare (see
    /// `task_string_formals`); otherwise the global `string_signals` set.
    #[inline]
    pub(super) fn frame_name_is_string(&self, name: &str) -> bool {
        match self.task_frame_string_flag(name) {
            Some(is_string) => is_string,
            None => self.string_signals.contains(name),
        }
    }

    /// §13.5 / §6.16: whether `name` is a `string` formal (`Some(true)`) or
    /// a non-string formal (`Some(false)`) of the task frame that holds it in
    /// the running process; `None` when that frame is not a task frame with a
    /// `task_string_formals` entry for it. The innermost frame declaring
    /// `name` decides, so a nested subroutine's same-named formal or local
    /// is not affected.
    #[inline]
    pub(super) fn task_frame_string_flag(&self, name: &str) -> Option<bool> {
        if self.task_string_formals.is_empty() {
            return None;
        }
        let i = self
            .local_stack
            .iter()
            .rposition(|f| f.contains_key(name))?;
        let fgen = *self.local_gen_stack.get(i)?;
        let entry = self
            .task_string_formals
            .iter()
            .rev()
            .find(|e| e.fgen == fgen)?;
        if entry.strings.iter().any(|n| n == name) {
            Some(true)
        } else if entry.others.iter().any(|n| n == name) {
            Some(false)
        } else {
            None
        }
    }

    /// Does the lvalue mention any of `names` — root identifier segments or
    /// member names — without allocating? Mirrors `collect_lvalue_target_names`.
    pub(super) fn lvalue_hits_names(lhs: &Expression, names: &HashSet<String>) -> bool {
        match &lhs.kind {
            ExprKind::Ident(h) => h.path.iter().any(|seg| names.contains(&seg.name.name)),
            ExprKind::MemberAccess { expr, member } => {
                names.contains(&member.name) || Self::lvalue_hits_names(expr, names)
            }
            ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => {
                Self::lvalue_hits_names(expr, names)
            }
            ExprKind::Concatenation(parts) => {
                parts.iter().any(|p| Self::lvalue_hits_names(p, names))
            }
            _ => false,
        }
    }

    /// Does the static collection `member` need per-DECLARING-class storage?
    /// True when TWO OR MORE distinct classes declare it (§8.9) — sibling
    /// collisions (the four `uvm_cmdline_*` classes each declaring
    /// `static … settings[$]`) and a derived class that REDECLARES a base's
    /// static collection (`d::m_q` vs `b::m_q`) alike. Inherited-ONLY access
    /// (a single declarer) keeps the bare store the runtime relies on.
    pub(super) fn static_coll_name_collides(&self, member: &str) -> bool {
        if self.colliding_static_colls_ready {
            return self.colliding_static_colls.contains(member);
        }
        let mut decls: Vec<&str> = self
            .module
            .classes
            .values()
            .filter(|cd| cd.static_collections.iter().any(|(nm, _, _)| nm == member))
            .map(|cd| cd.name.as_str())
            .collect();
        decls.sort_unstable();
        decls.dedup();
        decls.len() >= 2
    }

    /// Is a STATIC assoc collection member string-keyed? Walks `start_class`
    /// up its extends chain consulting `static_assoc_key_types`. A key type
    /// recorded as a plain TYPE PARAMETER (e.g. `static T map[K];` where the
    /// active specialization binds `K` to `string`) is resolved through the
    /// active specialization [1] before judging; statics have no instance so
    /// `current_spec` is the only binding source.
    pub(super) fn static_member_is_string_keyed(&self, start_class: &str, member: &str) -> bool {
        let mut cur = Some(start_class.to_string());
        while let Some(cn) = cur {
            if let Some(cd) = self.module.classes.get(&cn) {
                if cd
                    .static_collections
                    .iter()
                    .any(|(n, is_a, _)| n == member && *is_a)
                {
                    if let Some(kt) = cd.static_assoc_key_types.get(member) {
                        let mut concrete = self
                            .resolve_type_param_binding(kt)
                            .unwrap_or_else(|| kt.clone());
                        if let Some(c2) = self.resolve_type_param_binding(&concrete) {
                            concrete = c2;
                        }
                        if concrete == "string" {
                            return true;
                        }
                    }
                }
                cur = cd.extends.clone();
            } else {
                break;
            }
        }
        false
    }

    /// Does `start_class` or an ancestor declare `name` as a STATIC
    /// queue/assoc/dynamic-array collection (i.e. it is in
    /// `static_collections`)?
    pub(super) fn collection_is_static_in(&self, start_class: &str, name: &str) -> bool {
        let mut cur: Option<&str> = Some(start_class);
        while let Some(cn) = cur {
            if let Some(cd) = self.module.classes.get(cn) {
                if cd.static_collections.iter().any(|(n, _, _)| n == name) {
                    return true;
                }
                cur = cd.extends.as_deref();
            } else {
                break;
            }
        }
        false
    }

    /// `#[cold]` + never inlined: this hangs off a guard that runs 22M+
    /// times on a UVM run, and letting it fatten `instance_assoc_member`
    /// measurably perturbed that hot path's code generation.
    #[cold]
    #[inline(never)]
    pub(super) fn chained_member_scoped(&self, name: &str, fd: usize) -> Option<String> {
        // A dotted name with THREE or more segments can be a HANDLE CHAIN
        // to a collection member (`o.sub.g`). Every fallback that funnels
        // through here got None for it, so a chained element write landed
        // nowhere and a chained read came back x. Walk the chain through
        // heap properties; a module hierarchical path fails the first hop
        // (or the final class-membership check) and returns None exactly
        // as before. Two-segment forms keep their existing paths — the
        // last-dot probe below is a short backward scan, so they exit at
        // nearly the original cost.
        if self.no_class_objects() {
            return None;
        }
        let ld = name.rfind('.').unwrap_or(fd);
        if ld == fd {
            return None;
        }
        // The FINAL segment must be a collection member declared by SOME
        // class — kills module hierarchical paths before any handle
        // evaluation (whose static-property probe walks the class-context
        // chain on a miss). The unscanned middle keeps the '#'/'[' reject.
        let member_leaf = &name[ld + 1..];
        if name[fd..ld].bytes().any(|b| matches!(b, b'#' | b'[')) {
            return None;
        }
        if !self.chained_coll_members().contains(member_leaf) {
            return None;
        }
        let segs: Vec<&str> = name.split('.').collect();
        let mut h = if segs[0] == "this" || segs[0] == "super" {
            self.this_stack.last().copied().flatten()?
        } else {
            self.eval_ident_handle(segs[0])?
        };
        if h == 0 {
            return None;
        }
        for seg in &segs[1..segs.len() - 1] {
            let inst = self.heap.get(h).and_then(|o| o.as_ref())?;
            h = inst.properties.get(*seg).and_then(|v| v.to_u64())? as usize;
            if h == 0 {
                return None;
            }
        }
        let member = segs[segs.len() - 1];
        // The final object's CLASS must declare the member as an
        // array/collection property — this is what keeps an unrelated
        // name whose value happens to collide with a live handle from
        // hijacking the resolution.
        let mut cur = self
            .heap
            .get(h)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone());
        while let Some(c) = cur {
            let Some(cd) = self.module.classes.get(&c) else {
                break;
            };
            if cd.array_properties.contains_key(member)
                || cd.array_nd_properties.contains_key(member)
                || cd.queue_properties.contains_key(member)
                || cd.assoc_properties.contains_key(member)
            {
                return Some(format!("{}#{}", h, member));
            }
            cur = cd.extends.clone();
        }
        None
    }

    /// Every collection/array member name declared by ANY class, built once.
    /// The cheap pre-filter for chained-handle resolution: a leaf that is in
    /// no class cannot be a member chain, so no handle evaluation (whose
    /// static-property probe walks the class-context chain on a miss) runs
    /// for module hierarchical names.
    pub(super) fn chained_coll_members(&self) -> &HashSet<String> {
        self.chained_coll_member_names.get_or_init(|| {
            let mut set: HashSet<String> = HashSet::default();
            for cd in self.module.classes.values() {
                for k in cd
                    .array_properties
                    .keys()
                    .chain(cd.array_nd_properties.keys())
                    .chain(cd.queue_properties.keys())
                    .chain(cd.assoc_properties.keys())
                {
                    set.insert(k.clone());
                }
            }
            set
        })
    }

    /// §20.7: the instance-scoped store (`<handle>#<member>`) behind a
    /// HANDLE-QUALIFIED array member operand — `a.M`, `this.M`. `new`
    /// registers every fixed-size array property under that name in the
    /// `arrays`/`arrays_2d`/`arrays_nd` tables, so once the operand resolves
    /// to it the ordinary declared-bounds path answers the query.
    ///
    /// Both spellings the parser produces are accepted: an explicit
    /// `MemberAccess`, and the flattened two-segment `Ident` — the same pair
    /// `assign_class_collection_pattern` already resolves for whole-array
    /// writes. Returns None unless the object's class, or an ancestor, really
    /// declares `member` as a fixed-size array, which leaves a module
    /// hierarchical path and a packed (non-array) member on their existing
    /// paths.
    pub(super) fn handle_member_array_name(&self, expr: &Expression) -> Option<String> {
        let (handle, member) = match &expr.kind {
            ExprKind::MemberAccess { expr: base, member } => {
                let h = match &base.kind {
                    ExprKind::This => self.this_stack.last().copied().flatten()?,
                    ExprKind::Ident(b) if b.path.len() == 1 && b.path[0].selects.is_empty() => {
                        self.eval_ident_handle(&b.path[0].name.name)?
                    }
                    _ => return None,
                };
                (h, member.name.as_str())
            }
            ExprKind::Ident(hi)
                if hi.path.len() == 2 && hi.path.iter().all(|s| s.selects.is_empty()) =>
            {
                let base = &hi.path[0].name.name;
                let h = if base == "this" || base == "super" {
                    self.this_stack.last().copied().flatten()?
                } else {
                    self.eval_ident_handle(base)?
                };
                (h, hi.path[1].name.name.as_str())
            }
            _ => return None,
        };
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
            if cd.array_properties.contains_key(member)
                || cd.array_nd_properties.contains_key(member)
            {
                return Some(format!("{}#{}", handle, member));
            }
            cur = cd.extends.clone();
        }
        None
    }

    /// `format!("{}#{}", handle, member)`: the instance-scoped store name of
    /// a collection member, built without the formatting machinery.
    pub(super) fn handle_member_key(handle: usize, member: &str) -> String {
        let mut out = String::with_capacity(member.len() + 21);
        Self::push_i64(&mut out, handle as i64);
        out.push('#');
        out.push_str(member);
        out
    }

    pub(super) fn instance_assoc_member(&self, name: &str) -> Option<String> {
        // §8.9: `Cls::member` / flattened `Cls.member` spellings of a static
        // fixed array resolve to the declaring class's store BEFORE the
        // dotted-name guard below can reject them.
        if let Some(k) = self.static_fixed_key_for_name(name) {
            return Some(k);
        }
        // §23.9 / §13.5 (#534): an unpacked-array FORMAL of the running
        // method is bound under its bare name (a module-array row plus a
        // frame marker) and shadows a same-named class member; it must not
        // be lifted to the object's storage. The array-row test keeps the
        // marker probe off the common path.
        if self.module.arrays.contains_key(name)
            && let Some(frame) = self.local_stack.last()
            && frame.contains_key(&format!("\u{1}array_formal:{name}"))
        {
            return None;
        }
        // This guard sees every call (22M+ on a UVM run): keep the ORIGINAL
        // vectorizable one-pass `any` for the overwhelmingly common bare
        // names (a manual classify loop here measured +0.5% instructions on
        // an axi4 UVM run), and only when a special byte exists rescan to
        // tell a rejected spelling from a possible handle chain.
        if name.bytes().any(|b| matches!(b, b'#' | b'.' | b'[')) {
            let mut first_dot: Option<usize> = None;
            for (i, b) in name.bytes().enumerate() {
                match b {
                    b'#' | b'[' => return None,
                    b'.' => {
                        first_dot = Some(i);
                        break;
                    }
                    _ => {}
                }
            }
            let fd = first_dot?;
            return self.chained_member_scoped(name, fd);
        }
        // Every store below is a collection some class declares under this
        // name (or types with a parameter or a dimensioned typedef).
        if !self.coll_member_possible(name) {
            return None;
        }
        // §8.10 / §13.3: the running subroutine's own formal or automatic
        // local of this name (an entry in its rename frame) shadows a
        // same-named property — `uvm_packer::put_bits(ref bit bitstream[])`
        // must read its formal, not the class's `static bit bitstream[]`.
        if self.dyn_name_lookup_innermost(name).is_some() {
            return None;
        }
        // §8.9: inside a STATIC method there is no `this`; the lexical class
        // context still resolves a bare static-array member to its store.
        let Some(handle) = self
            .this_stack
            .last()
            .copied()
            .flatten()
            .filter(|&h| h != 0)
        else {
            let ctx = self.class_context_stack.last().cloned().flatten()?;
            // A static QUEUE/ASSOC/dynamic-ARRAY property of a (possibly
            // parameterized) class, accessed BARE inside a static method
            // (e.g. uvm_config_db's `static pool m_rsc[comp]`): route to its
            // per-specialization store (`spec_static_coll_key` folds the
            // active spec), so an ELEMENT read/write `m[k]` and the builtin
            // ops (size/exists/...) agree on ONE store. Without this the
            // element path resolved to the bare name ─ a DIFFERENT store from
            // the per-spec key builtins use ─ so `m[k]=v` stored its value
            // where the later `m[k]`/`exists` never looked (and vice-versa).
            if self.collection_is_static_in(&ctx, name) {
                return Some(self.spec_static_coll_key(name).into_owned());
            }
            return self.static_fixed_key_in(&ctx, name);
        };
        // Use the runtime class of `this` (from the heap) rather than the
        // lexical class_context_stack.  When a bare method call (e.g. `body()`)
        // is inlined without updating class_context_stack, the stack still
        // holds the CALLER's class (e.g. "uvm_sequence_base"), so a foreach
        // over a member array declared in the concrete subclass ("my_seq")
        // would fail to resolve.  The heap always knows the actual type.
        // Borrowed walk: the owned form allocated the start class name and
        // one String per ancestor level, on a path that runs 22.3M times on
        // an axi4Lite UVM run.
        let ctx: &str = self
            .heap
            .get(handle)
            .and_then(|x| x.as_ref())
            .map(|i| i.class_name.as_str())?;
        // Per-(class, name) verdict: every step below except a type-parameter
        // binding depends on the class tables only.
        loop {
            {
                let cache = self.member_coll_cache.borrow();
                if let Some(v) = cache.get(ctx).and_then(|m| m.get(name)) {
                    match v {
                        MemberCollKind::Coll(None) => {
                            return Some(Self::handle_member_key(handle, name));
                        }
                        MemberCollKind::Coll(Some(owner)) => {
                            return Some(format!("{}::{}", owner, name));
                        }
                        MemberCollKind::StaticColl => {
                            return Some(self.spec_static_coll_key_in_class(name, ctx));
                        }
                        MemberCollKind::Plain { raws, unbound } => {
                            let bound = !raws.is_empty()
                                && self.heap.get(handle).and_then(|o| o.as_ref()).is_some_and(
                                    |i| {
                                        !i.type_bindings.is_empty()
                                            && raws.iter().any(|r| i.type_bindings.contains_key(r))
                                    },
                                );
                            if !bound {
                                return unbound.then(|| Self::handle_member_key(handle, name));
                            }
                            break;
                        }
                    }
                }
            }
            let v = self.member_coll_verdict(ctx, name);
            self.member_coll_cache
                .borrow_mut()
                .entry(ctx.to_string())
                .or_default()
                .insert(name.to_string(), v);
        }
        // Flattened index first: it answers the class-only part of the
        // decision (the four property maps and the static-fixed-array list,
        // in ancestry order) in one lookup. A property is declared once, so a
        // name can never be BOTH a static fixed array and a type-parameter-
        // bound collection in the same class — which is what lets the
        // handle-dependent `prop_bound_collection` probe move to the miss
        // path below instead of running at every level.
        if let Some(kind) = self.class_coll_lookup(ctx, name) {
            return Some(match kind {
                None => Self::handle_member_key(handle, name),
                Some(owner) => format!("{}::{}", owner, name),
            });
        }
        // §8.25: a STATIC collection member (registered in `static_collections`, so
        // absent from `assoc_properties`/`queue_properties`/`class_coll_index`)
        // accessed BARE from an INSTANCE method must still resolve to the class's
        // shared static store — the same cell a static method (`spec_static_coll_key`)
        // and a `ClassName::m[...]` spelling use. Without this the element write
        // `m[k]=v` and later `m[k]`/`m.exists(k)` resolve to DIFFERENT stores
        // (the write auto-created a fresh empty cell), so values read back `x`/0
        // and `exists` is false — uvm_config_db's `m_waiters` waiter map broke
        // exactly this way. Uses the runtime class of `this` as the concrete ctx.
        if self.collection_is_static_in(ctx, name) {
            return Some(self.spec_static_coll_key_in_class(name, ctx));
        }
        // Miss: only the per-instance type-binding case can still match.
        if self
            .class_non_member_cache
            .borrow()
            .get(ctx)
            .is_some_and(|set| set.contains(name))
        {
            return None;
        }
        let mut cur: Option<&str> = Some(ctx);
        let mut declared = false;
        while let Some(cn) = cur {
            let cd = self.module.classes.get(cn)?;
            declared |= cd.properties.contains_key(name);
            if self.prop_bound_collection(handle, cn, name) {
                return Some(Self::handle_member_key(handle, name));
            }
            cur = cd.extends.as_deref();
        }
        if !declared {
            // Not a property anywhere in the chain: the verdict cannot
            // depend on this instance's type bindings, so it holds for
            // every object of the class.
            self.class_non_member_cache
                .borrow_mut()
                .entry(ctx.to_string())
                .or_default()
                .insert(name.to_string());
        }
        None
    }

    /// Build (once per class) the flattened member -> kind map described on
    /// `class_coll_index`. Walks derived -> base inserting only absent names,
    /// so the first level that declares a name wins, and within a level the
    /// per-instance collections take precedence over static fixed arrays —
    /// exactly the order the original level-by-level walk used.
    /// Look a member up in the flattened index, building it on first use.
    /// The lookup happens INSIDE the borrow so the common (miss / per-instance)
    /// path clones nothing — an earlier version handed back an `Arc` clone,
    /// i.e. an atomic increment on all 22.3M calls, which measured worse than
    /// the work it saved.
    pub(super) fn class_coll_lookup(
        &self,
        class_name: &str,
        member: &str,
    ) -> Option<Option<String>> {
        {
            let b = self.class_coll_index.borrow();
            if let Some(idx) = b.get(class_name) {
                return idx.get(member).cloned();
            }
        }
        self.build_class_coll_index(class_name);
        let b = self.class_coll_index.borrow();
        b.get(class_name).and_then(|idx| idx.get(member).cloned())
    }

    pub(super) fn build_class_coll_index(&self, class_name: &str) {
        let mut map: HashMap<String, Option<String>> = HashMap::default();
        let mut cur: Option<&str> = Some(class_name);
        while let Some(cn) = cur {
            let Some(cd) = self.module.classes.get(cn) else {
                break;
            };
            for k in cd
                .assoc_properties
                .keys()
                .chain(cd.queue_properties.keys())
                .chain(cd.array_properties.keys())
                .chain(cd.array_nd_properties.keys())
            {
                map.entry(k.clone()).or_insert(None);
            }
            for (n, ..) in cd.static_fixed_arrays.iter() {
                map.entry(n.clone()).or_insert_with(|| Some(cn.to_string()));
            }
            cur = cd.extends.as_deref();
        }
        self.class_coll_index
            .borrow_mut()
            .insert(class_name.to_string(), map);
    }

    /// The class-only part of `instance_assoc_member` for `name` inside a
    /// method of runtime class `ctx` (see `MemberCollKind`).
    pub(super) fn member_coll_verdict(&self, ctx: &str, name: &str) -> MemberCollKind {
        if let Some(kind) = self.class_coll_lookup(ctx, name) {
            return MemberCollKind::Coll(kind);
        }
        if self.collection_is_static_in(ctx, name) {
            return MemberCollKind::StaticColl;
        }
        let mut raws: Vec<String> = Vec::new();
        let mut unbound = false;
        let mut cur: Option<&str> = Some(ctx);
        while let Some(cn) = cur {
            let Some(cd) = self.module.classes.get(cn) else {
                break;
            };
            unbound = self.prop_bound_collection_in(None, cn, name);
            if let Some(raw) = self
                .prop_raw_ty_cache
                .borrow()
                .get(cn)
                .and_then(|m| m.get(name))
                .cloned()
                .flatten()
            {
                if !raws.contains(&raw) {
                    raws.push(raw);
                }
            }
            if unbound {
                break;
            }
            cur = cd.extends.as_deref();
        }
        MemberCollKind::Plain { raws, unbound }
    }

    /// Does `class_name` or an ancestor declare `member` as an associative
    /// array or queue/dynamic-array (both stored per-instance as `<h>#<m>`)?
    /// Is `member` of the object at `handle` a collection BY TYPE BINDING —
    /// a property declared with a type parameter that this instance binds to
    /// a typedef carrying a dynamic/queue unpacked dimension (§6.20.3)?
    pub(super) fn prop_bound_collection(
        &self,
        handle: usize,
        class_name: &str,
        member: &str,
    ) -> bool {
        let bindings = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|i| &i.type_bindings);
        self.prop_bound_collection_in(bindings, class_name, member)
    }

    /// `prop_bound_collection` under the given instance type bindings
    /// (`None`: no binding applies).
    pub(super) fn prop_bound_collection_in(
        &self,
        bindings: Option<&HashMap<String, String>>,
        class_name: &str,
        member: &str,
    ) -> bool {
        // Hit path without allocations: the cached raw type and the
        // instance's binding are borrowed, and the dimension verdict is
        // read by `&str`.
        {
            let cache = self.prop_raw_ty_cache.borrow();
            if let Some(hit) = cache.get(class_name).and_then(|m| m.get(member)) {
                let Some(raw) = hit.as_deref() else {
                    return false;
                };
                let concrete: &str = bindings
                    .and_then(|b| b.get(raw))
                    .map(String::as_str)
                    .unwrap_or(raw);
                if let Some(&v) = self.collection_dim_cache.borrow().get(concrete) {
                    return v;
                }
            }
        }
        let raw_ty: Option<String> = {
            let hit = self
                .prop_raw_ty_cache
                .borrow()
                .get(class_name)
                .and_then(|m| m.get(member))
                .cloned();
            if let Some(hit) = hit {
                hit
            } else {
                let mut cur: Option<&str> = Some(class_name);
                let computed: Option<String> = loop {
                    let Some(cn) = cur else { break None };
                    match self.module.classes.get(cn) {
                        Some(cd) => {
                            if let Some(sig) = cd.properties.get(member) {
                                break sig.type_name.clone();
                            }
                            cur = cd.extends.as_deref();
                        }
                        None => break None,
                    }
                };
                self.prop_raw_ty_cache
                    .borrow_mut()
                    .entry(class_name.to_string())
                    .or_default()
                    .insert(member.to_string(), computed.clone());
                computed
            }
        };
        let Some(raw) = raw_ty else { return false };
        // A type-param binding resolves first; otherwise the raw name may
        // itself be an array/queue typedef (`my_array_t data;` — the class
        // property elaboration is dim-blind for typedef'd types).
        let concrete = bindings.and_then(|b| b.get(&raw).cloned()).unwrap_or(raw);
        if let Some(&hit) = self.collection_dim_cache.borrow().get(&concrete) {
            return hit;
        }
        // A typedef name may itself be a class-LOCAL typedef, which
        // lives in the declaring class's `typedef_unpacked_dims`, NOT the
        // module-level map. Search BOTH: module-level first (package
        // typedefs), then every class's local typedefs for a matching name
        // with an unsized/queue first dimension (IEEE 1800-2023 §6.18).
        let is_collection_dim = |dims: &Vec<crate::ast::types::UnpackedDimension>| {
            dims.first().is_some_and(|d| {
                matches!(
                    d,
                    crate::ast::types::UnpackedDimension::Unsized(_)
                        | crate::ast::types::UnpackedDimension::Queue { .. }
                        | crate::ast::types::UnpackedDimension::Associative { .. }
                )
            })
        };
        let verdict = if let Some(dims) = self.module.typedef_unpacked_dims.get(&concrete) {
            is_collection_dim(dims)
        } else {
            self.module
                .classes
                .values()
                .find_map(|cd| cd.typedef_unpacked_dims.get(&concrete))
                .is_some_and(|dims| is_collection_dim(dims))
        };
        self.collection_dim_cache
            .borrow_mut()
            .insert(concrete, verdict);
        verdict
    }

    /// Can `member` be a collection store of an object or class
    /// (`class_assoc_member`, `prop_bound_collection`, `static_fixed_key_in`,
    /// `instance_assoc_member`)? Only if some class declares it as an
    /// associative/queue/array property, a static collection or a static
    /// fixed array, or types it with a class parameter or a typedef carrying
    /// unpacked dimensions. Rebuilt after a procedural `typedef`.
    pub(super) fn coll_member_possible(&self, member: &str) -> bool {
        let mut cell = self.coll_member_names.borrow_mut();
        let set = cell.get_or_insert_with(|| {
            let params = &self.class_member_names().params;
            let mut dimmed: HashSet<&str> = self
                .module
                .typedef_unpacked_dims
                .keys()
                .map(String::as_str)
                .collect();
            for cd in self.module.classes.values() {
                dimmed.extend(cd.typedef_unpacked_dims.keys().map(String::as_str));
            }
            let mut names: HashSet<String> = HashSet::default();
            for cd in self.module.classes.values() {
                names.extend(cd.assoc_properties.keys().cloned());
                names.extend(cd.queue_properties.keys().cloned());
                names.extend(cd.array_properties.keys().cloned());
                names.extend(cd.array_nd_properties.keys().cloned());
                names.extend(cd.static_collections.iter().map(|(n, ..)| n.clone()));
                names.extend(cd.static_fixed_arrays.iter().map(|(n, ..)| n.clone()));
                for (p, sig) in &cd.properties {
                    if sig
                        .type_name
                        .as_deref()
                        .is_some_and(|t| params.contains(t) || dimmed.contains(t))
                    {
                        names.insert(p.clone());
                    }
                }
            }
            names
        });
        set.contains(member)
    }

    pub(super) fn class_assoc_member(&self, class_name: &str, member: &str) -> bool {
        // Walk the ancestry by BORROW. The owned form allocated a String for
        // the start class and another per level (`extends.clone()`); this
        // helper sits under `instance_assoc_member`, which runs 22.3M times
        // on an axi4Lite UVM run.
        let mut cur: Option<&str> = Some(class_name);
        while let Some(cn) = cur {
            match self.module.classes.get(cn) {
                Some(cd) => {
                    // §7.4.2: a fixed multi-dimensional member (`int m [2][3]`)
                    // stores under `<handle>#m` as a 1-D one does; without
                    // it every whole-member write (`c.m = a`, a pattern, a
                    // row `c.m[1] = '{..}`) resolved nowhere and was dropped.
                    if cd.assoc_properties.contains_key(member)
                        || cd.queue_properties.contains_key(member)
                        || cd.array_properties.contains_key(member)
                        || cd.array_nd_properties.contains_key(member)
                    {
                        return true;
                    }
                    cur = cd.extends.as_deref();
                }
                None => return false,
            }
        }
        false
    }

    /// Resolve an instance-scoped collection (`<handle>#member`) from a known
    /// heap handle and member name. Returns Some only if `member` is an
    /// associative-array / queue / dynamic-array property of the object's
    /// class. Shared by the MemberAccess-Index base arm and the flattened
    /// `Ident([arr[idx], member])` arm of `expr_assoc_name` so that
    /// `arr[i].coll` and `foreach (arr[i].coll[k])` (parsed as a flattened
    /// Ident) both resolve the CORRECT element's collection — without this,
    /// `foreach (successors[s].m_predecessors[pred])` iterates
    /// `this.m_predecessors` instead of `successors[s].m_predecessors`, so
    /// the graph reads the wrong object's edges.
    pub(super) fn handle_collection_name(&self, handle: usize, member: &str) -> Option<String> {
        if handle == 0 {
            return None;
        }
        let cn = self
            .heap
            .get(handle)
            .and_then(|x| x.as_ref())
            .map(|i| i.class_name.clone())?;
        if self.class_assoc_member(&cn, member) || self.prop_bound_collection(handle, &cn, member) {
            Some(format!("{}#{}", handle, member))
        } else {
            // §8.9: a fixed-size static array member of the receiver's class
            // chain names the class-qualified shared store.
            self.static_fixed_key_in(&cn, member)
        }
    }

    /// Instance-scoped storage name for an associative-array access whose
    /// base expression is either a bare member (`m` — uses `this`) or a
    /// member of another object (`obj.m`). Returns `<handle>#<member>`.
    pub(super) fn expr_assoc_name(&mut self, expr: &Expression) -> Option<String> {
        // §7.8: a collection MEMBER OF A STRUCT property (`obj.pkt.qmem[k]`,
        // `pkt.qmem.push_back(v)`, `pkt.dmem = new[n]`) stores under the
        // instance-scoped `<handle>#<prop>.<member>`, registered on first
        // sight — before the direct-property arms below, which cannot see
        // through the struct member path.
        if let Some(n) = self.class_struct_coll_name(expr) {
            return Some(n);
        }
        // `this.m` / `obj.m` (either parse shape) on an object resolves to a
        // store only for a member some class can hold as a collection.
        let obj_member_name = match &expr.kind {
            // Every receiver shape but a class scope or specialization (the
            // static-collection arms) — checked by NAME before the arms below
            // evaluate a nested, indexed or call receiver: a name no class can
            // hold as a collection resolves to no store whatever the object.
            ExprKind::MemberAccess { expr: base, member } => match &base.kind {
                ExprKind::Ident(bh)
                    if bh.path.len() == 1
                        && self.module.classes.contains_key(&bh.path[0].name.name) =>
                {
                    None
                }
                ExprKind::Specialization { .. } => None,
                _ => Some(member.name.as_str()),
            },
            ExprKind::Ident(h) if h.path.len() == 2 && h.path[0].selects.is_empty() => {
                Some(h.path[1].name.name.as_str())
            }
            _ => None,
        };
        if obj_member_name.is_some_and(|m| !self.coll_member_possible(m)) {
            // `foreach (s.sp[2][k])`: a selected struct member may still
            // name an element of a fixed array of collections (§7.4.5).
            if let ExprKind::Ident(h) = &expr.kind
                && h.path.len() == 2
                && !h.path[1].selects.is_empty()
                && self.indexed_colls_live()
            {
                return self.selected_ident_coll_name(expr);
            }
            return None;
        }
        // `obj`/`member` pair → instance-scoped name if `member` is an
        // associative property of the object the handle points at.
        let obj_member = |sim: &Self, obj: &str, member: &str| -> Option<String> {
            // `this.member` — resolve via the current `this` handle, not a
            // variable lookup. Without this, an explicit-`this` associative or
            // queue write (`this.m_successors[k]=1`, as UVM's phase graph and
            // many class methods use) fails to resolve to `<handle>#member` and
            // is silently dropped, while the bare `member[k]=1` form works.
            let handle = if obj == "this" || obj == "super" {
                sim.this_stack.last().copied().flatten().unwrap_or(0)
            } else {
                sim.eval_ident_handle(obj).unwrap_or(0)
            };
            if handle == 0 {
                return None;
            }
            let cn = sim
                .heap
                .get(handle)
                .and_then(|x| x.as_ref())
                .map(|i| i.class_name.clone())?;
            if sim.class_assoc_member(&cn, member) {
                Some(Self::handle_member_key(handle, member))
            } else if sim.prop_bound_collection(handle, &cn, member) {
                // §6.20.3: `T data;` where THIS instance binds T to an
                // array/queue typedef — a per-spec collection property.
                Some(Self::handle_member_key(handle, member))
            } else {
                // §8.9: a fixed-size static array member names the
                // class-qualified shared store.
                sim.static_fixed_key_in(&cn, member)
            }
        };
        match &expr.kind {
            // Explicit `this.member` / `obj.member` MemberAccess forms — the
            // flattened-Ident arms below never see them, so a
            // `this.data = val` whole-collection write went unresolved.
            ExprKind::MemberAccess { expr: base, member } => match &base.kind {
                ExprKind::This => obj_member(self, "this", &member.name),
                ExprKind::Ident(bh) if bh.path.len() == 1 => {
                    // A typedef-ALIAS base (`typedef rt#(int) rt_i;` then
                    // `rt_i::all` as a whole-collection RHS) — not a class
                    // name and not an object: resolve the alias's per-spec
                    // static-collection key like the Specialization arm.
                    if !self.module.classes.contains_key(&bh.path[0].name.name)
                        && !self
                            .signal_name_to_id
                            .contains_key(bh.path[0].name.name.as_str())
                        && self
                            .current_spec
                            .as_ref()
                            .is_none_or(|(b, _)| b != &bh.path[0].name.name)
                    {
                        if let Some((base, sig)) = self.resolve_typedef_spec(&bh.path[0].name.name)
                        {
                            if self.member_is_static_coll(&base, &member.name) {
                                let saved = self.current_spec.take();
                                self.current_spec = Some((base.clone(), sig.clone()));
                                let key = self.static_prop_key(&base, &member.name);
                                self.current_spec = saved;
                                return key;
                            }
                        }
                    }
                    // `ClassName::static_collection.first(ref k)` — when the
                    // base is a CLASS name (not an object variable) and the
                    // member is a static associative-array collection, return
                    // the bare member name (static collections are stored
                    // globally by bare name). Without this, obj_member treats
                    // the class name as an object handle (0 → None), and the
                    // assoc-array first()/next() iteration silently fails.
                    if self.module.classes.contains_key(&bh.path[0].name.name) {
                        // §8.25: an EXPLICITLY parameterized access
                        // `config_db#(int)::m_rsc.exists(c)`/`.num()` arrives
                        // here as a bare `Ident(config_db)` receiver (the outer
                        // value-context eval strips the `Specialization`
                        // wrapper before the builtin method dispatches). The
                        // active `current_spec` still carries `(config_db,
                        // int)`, so resolve the per-spec static-collection key
                        // the same way the `Specialization` arm below does;
                        // otherwise the read lands on bare `m_rsc` while the
                        // matching write went to `config_db#(int)::m_rsc`, and
                        // `exists()`/`num()` report empty.
                        if let Some((cb, cs)) = &self.current_spec {
                            if cb == &bh.path[0].name.name
                                && self.member_is_static_coll(cb, &member.name)
                            {
                                let sb = cb.clone();
                                let ss = cs.clone();
                                let saved = self.current_spec.take();
                                self.current_spec = Some((sb.clone(), ss));
                                let key = self.static_prop_key(&sb, &member.name);
                                self.current_spec = saved;
                                return key;
                            }
                        }
                        if self.is_associative_array(&member.name) {
                            return Some(member.name.clone());
                        }
                        // §8.9: `ClassName::S[i]` — the class-qualified
                        // shared store of a fixed-size static array,
                        // resolved through the chain (a subclass scope may
                        // name an inherited static).
                        if let Some(k) =
                            self.static_fixed_key_in(&bh.path[0].name.name, &member.name)
                        {
                            return Some(k);
                        }
                    }
                    obj_member(self, &bh.path[0].name.name, &member.name)
                }
                // `arr[i].member` — base is an array INDEX expression.
                // Evaluate the element to a class handle and resolve the
                // instance-scoped collection `<handle>#member` (an
                // associative array / queue / dynamic array owned by that
                // object). Without this, traversal `successors[s].m_predecessors`
                // resolves to nothing, so the sibling graph reads the wrong
                // object's edges. Restricted to an Index base (not every complex
                // base) so `a.b.member` / `f().member` keep their prior fall-through
                // behavior. Scalar members are unaffected (class_assoc_member
                // returns false → None → caller falls through to the property read).
                ExprKind::Index { .. } => {
                    let h = self.eval_expr(base).to_u64().unwrap_or(0) as usize;
                    self.handle_collection_name(h, &member.name)
                }
                // Nested member access: `a.b.c` where `a.b` is itself a
                // MemberAccess (e.g. `ctxt.obj_ref.queue_member`). Evaluate
                // the base to get the intermediate object handle, then resolve
                // the instance-scoped collection on it. This is critical for
                // nested queue property mutation (`c.obj.queue.push_back(c)`) —
                // without it, push_back on a nested queue property silently fails.
                ExprKind::MemberAccess { .. } => {
                    let h = self.eval_expr(base).to_u64().unwrap_or(0) as usize;
                    if h != 0 {
                        self.handle_collection_name(h, &member.name)
                    } else {
                        None
                    }
                }
                // A function-call base (`get_h().member`,
                // `factory.get().queue`): evaluate the call to the object
                // handle it returns and resolve the instance-scoped
                // collection on it. Without this, both the WRITE
                // (`f().member[k] = v`) and the READ (`f().member.exists(k)`)
                // resolved `member` to nothing, so writes were silently
                // dropped and reads came back empty.
                ExprKind::Call { .. } => {
                    let h = self.eval_expr(base).to_u64().unwrap_or(0) as usize;
                    if h != 0 {
                        self.handle_collection_name(h, &member.name)
                    } else {
                        None
                    }
                }
                // `ClassName#(params)::static_coll` — a parameterized-class
                // static collection (queue/assoc/array) reached through a
                // class SPECIALIZATION receiver rather than `this`/an object.
                // Resolve the per-spec storage key exactly like
                // `spec_static_coll_key` does, so the assoc/queue store name
                // matches the one builtin ops (push_back/size/...) use and is
                // isolated per specialization. Without this the index/assoc
                // read AND write both resolved to None, so `C#(int)::m["a"]=v`
                // (and uvm_config_db's static per-component pool) was silently
                // dropped on write and read back empty.
                ExprKind::Specialization { .. } => {
                    let Some((b, s)) = self.resolve_call_spec_params(
                        Self::extract_call_spec(base),
                        &self.current_spec,
                    ) else {
                        return None;
                    };
                    if self.member_is_static_coll(&b, &member.name) {
                        let saved = self.current_spec.take();
                        self.current_spec = Some((b.clone(), s.clone()));
                        let key = self.static_prop_key(&b, &member.name);
                        self.current_spec = saved;
                        return key;
                    }
                    None
                }
                _ => None,
            },
            ExprKind::Ident(h) if h.path.len() == 1 => {
                // §7.4.5: `foreach (q3[1][j])` names an element of a fixed
                // array of collections as one identifier carrying the select.
                if !h.path[0].selects.is_empty() && self.indexed_colls_live() {
                    return self.selected_ident_coll_name(expr);
                }
                self.instance_assoc_member(&h.path[0].name.name)
            }
            // Flattened `obj.member` as a 2-segment hierarchical Ident.
            ExprKind::Ident(h) if h.path.len() == 2 => {
                // `foreach (s.sp[2][k])`: the same, through a struct member.
                if !h.path[1].selects.is_empty() && self.indexed_colls_live() {
                    return self.selected_ident_coll_name(expr);
                }
                // §8.25.1: a leading segment that is a TYPEDEF ALIAS of a
                // parameterized-class specialization (`typedef rt#(int) rt_i;`
                // then `rt_i::all.push_back(v)` — the call path strips the
                // specialization into `current_spec` and leaves a 2-segment
                // `rt_i.all`) resolves to the per-spec static-collection key,
                // like the `Specialization` and class-name arms above.
                if h.path[0].selects.is_empty()
                    && !self.module.classes.contains_key(&h.path[0].name.name)
                    && !self
                        .signal_name_to_id
                        .contains_key(h.path[0].name.name.as_str())
                    && self
                        .current_spec
                        .as_ref()
                        .is_none_or(|(b, _)| b != &h.path[0].name.name)
                {
                    if let Some((base, sig)) = self.resolve_typedef_spec(&h.path[0].name.name) {
                        if self.member_is_static_coll(&base, &h.path[1].name.name) {
                            let saved = self.current_spec.take();
                            self.current_spec = Some((base.clone(), sig.clone()));
                            let key = self.static_prop_key(&base, &h.path[1].name.name);
                            self.current_spec = saved;
                            return key;
                        }
                    }
                }
                // `arr[i].member` flattened to `Ident([arr[selects], member])` —
                // the parser keeps the index in `path[0].selects`. Evaluate
                // the indexed element to a handle and resolve its collection
                // (the bare `obj_member` path below only works for a plain
                // object name and would resolve `arr` itself, not `arr[i]`).
                if !h.path[0].selects.is_empty() {
                    // Build `arr[i]` (or `arr[i][j]`) as a nested Index
                    // expression — eval_expr resolves an Index base to the
                    // element HANDLE, whereas a flattened
                    // `Ident([seg(arr, selects=[i])])` reads 0 for a
                    // dynamic-array-of-handles element.
                    let arr_ident = Expression::new(
                        ExprKind::Ident(HierarchicalIdentifier {
                            root: h.root.clone(),
                            path: vec![HierPathSegment {
                                name: h.path[0].name.clone(),
                                selects: Vec::new(),
                            }],
                            span: h.span,
                            cached_signal_id: std::cell::Cell::new(None),
                            cached_resolved_name: std::cell::OnceCell::new(),
                        }),
                        h.span,
                    );
                    let mut base_expr = arr_ident;
                    for sel in &h.path[0].selects {
                        base_expr = Expression::new(
                            ExprKind::Index {
                                expr: Box::new(base_expr),
                                index: Box::new(sel.clone()),
                            },
                            h.span,
                        );
                    }
                    let hd = self.eval_expr(&base_expr).to_u64().unwrap_or(0) as usize;
                    return self.handle_collection_name(hd, &h.path[1].name.name);
                }
                obj_member(self, &h.path[0].name.name, &h.path[1].name.name)
            }
            // Flattened `ClassName::static_handle.member` (3+ segments, e.g.
            // a singleton property reached through a static handle `m_inst`,
            // parsed as `Ident[svc, m_inst, m_children]`). The `len==2` arm only
            // covers a variable base; a static-handle head needs the static cell
            // resolved first, else this access keys to a stale/empty store and
            // reads as if unset.
            ExprKind::Ident(h)
                if h.path.len() >= 3 && self.module.classes.contains_key(&h.path[0].name.name) =>
            {
                let n = h.path.len();
                let mut handle = match self
                    .static_prop_key(&h.path[0].name.name, &h.path[1].name.name)
                    .and_then(|k| self.class_statics.get(&k))
                    .and_then(|v| v.to_u64())
                {
                    Some(x) => x as usize,
                    None => return None,
                };
                // Walk any intermediate object segments (`Class::h.a.b.member`).
                for seg in &h.path[2..n - 1] {
                    if handle == 0 {
                        return None;
                    }
                    handle = match self
                        .heap
                        .get(handle)
                        .and_then(|o| o.as_ref())
                        .and_then(|i| i.properties.get(&seg.name.name))
                        .and_then(|v| v.to_u64())
                    {
                        Some(x) => x as usize,
                        None => return None,
                    };
                }
                if handle == 0 {
                    return None;
                }
                let member = &h.path[n - 1].name.name;
                let cn = self
                    .heap
                    .get(handle)
                    .and_then(|x| x.as_ref())
                    .map(|i| i.class_name.clone())?;
                if self.class_assoc_member(&cn, member) {
                    Some(format!("{}#{}", handle, member))
                } else {
                    None
                }
            }
            // Flattened nested OBJECT handle: `a.mid.base` (path.len() >= 3).
            // The receiver is an object chain (`a.mid`), not a static handle.
            // Evaluate the complete receiver, including intermediate selects
            // (`sys.targets[i].ports`), and scope the leaf collection to it.
            // §12.7.3: foreach must use that collection's bounds, not the
            // bounds of an earlier array in the receiver chain.
            ExprKind::Ident(h)
                if h.path.len() >= 3
                    && !self.module.classes.contains_key(&h.path[0].name.name)
                    && h.path.last().is_some_and(|s| s.selects.is_empty()) =>
            {
                let n = h.path.len();
                let member = h.path[n - 1].name.name.clone();
                let recv = hier_prefix(h);
                let recv_expr = if recv.path.iter().any(|s| !s.selects.is_empty()) {
                    Self::hier_select_chain(&recv, h.span)
                } else {
                    Expression::new(ExprKind::Ident(recv), h.span)
                };
                let hh = self.eval_handle_expr(&recv_expr).unwrap_or(0);
                if hh != 0 {
                    if let Some(cn) = self
                        .heap
                        .get(hh)
                        .and_then(|x| x.as_ref())
                        .map(|i| i.class_name.clone())
                    {
                        if self.class_assoc_member(&cn, &member) {
                            return Some(format!("{}#{}", hh, member));
                        }
                        if self.prop_bound_collection(hh, &cn, &member) {
                            return Some(format!("{}#{}", hh, member));
                        }
                    }
                }
                None
            }
            // Gated by name first: resolving evaluates the selects.
            ExprKind::Index { .. } if self.indexed_colls_live() => {
                self.indexed_coll_name_gated(expr)
            }
            _ => None,
        }
    }

    /// §7.4.5: an ELEMENT of a fixed array of collections is a collection of
    /// its own — `q3[1]` of `int q3[3][$]`, `s.sp[1]` / `fa[i].sp[1]` of a
    /// struct member `T sp[4][$]`. The flattened element name is the store
    /// the element-wise paths (`push_back`, `size`, `q3[1][j]`) already use.
    pub(super) fn indexed_coll_roots(&self) -> &HashSet<String> {
        self.indexed_coll_roots.get_or_init(|| {
            let mut set: HashSet<String> = HashSet::default();
            let names = self
                .module
                .dynamic_arrays
                .iter()
                .chain(self.module.associative_arrays.keys());
            for n in names {
                if let Some((root, _)) = n.split_once('[')
                    && !root.contains('#')
                    && (self.module.var_decl_types.contains_key(root)
                        || self.module.arrays.contains_key(root)
                        || self.module.arrays_2d.contains_key(root)
                        || self.module.arrays_nd.contains_key(root))
                {
                    // §12.7.3 / §23.8: a hierarchical module array is a
                    // declared root too; leaf names are only probe hints.
                    set.insert(root.to_string());
                    if let Some((_, leaf)) = root.rsplit_once('.') {
                        set.insert(leaf.to_string());
                    }
                }
            }
            let mut work: Vec<DataType> = Vec::new();
            for cd in self.module.classes.values() {
                // A class property that is itself a fixed array of
                // collections (`int q[3][$]`): its elements are stored
                // per object (see `class_array_of_coll_key`).
                set.extend(cd.array_of_coll_properties.keys().cloned());
                work.extend(cd.property_types.values().cloned());
            }
            work.extend(self.module.typedef_types.values().cloned());
            work.extend(self.module.var_decl_types.values().cloned());
            let mut budget = 65536usize;
            while let Some(dt) = work.pop() {
                if budget == 0 {
                    break;
                }
                budget -= 1;
                if let Some(su) = self.unpacked_struct_of(&dt) {
                    for m in &su.members {
                        for d in &m.declarators {
                            if d.dimensions.len() >= 2
                                && d.dimensions.last().is_some_and(|l| {
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
            self.indexed_coll_mask.set(mask);
            set
        })
    }

    /// Whether any fixed array of collections exists (also before the
    /// lazily built root set is known): the one-load gate in front of
    /// every indexed-collection probe.
    #[inline(always)]
    pub(super) fn indexed_colls_live(&self) -> bool {
        self.indexed_coll_mask.get() != 0 || self.indexed_coll_roots.get().is_none()
    }

    /// Whether `name` may be one of [`indexed_coll_roots`]: the length
    /// mask rejects most names before any hashing.
    #[inline]
    pub(super) fn indexed_coll_root(&self, name: &str) -> bool {
        let roots = match self.indexed_coll_roots.get() {
            Some(r) => r,
            None => self.indexed_coll_roots(),
        };
        self.indexed_coll_mask.get() & (1u64 << (name.len() & 63)) != 0 && roots.contains(name)
    }

    /// Whether the select `e` (`q3[0]`, `s.sp[1]`) may name an element of
    /// a fixed array of collections: the name it selects from is one
    /// [`indexed_coll_roots`] lists. Allocation-free.
    #[inline]
    pub(super) fn may_name_indexed_coll(&self, e: &Expression) -> bool {
        let ExprKind::Index { expr: base, .. } = &e.kind else {
            return false;
        };
        if self.indexed_coll_mask.get() == 0 && self.indexed_coll_roots.get().is_some() {
            return false;
        }
        let mut cur = base.as_ref();
        while let ExprKind::Index { expr, .. } = &cur.kind {
            cur = expr;
        }
        let name = match &cur.kind {
            ExprKind::Ident(h) => match h.path.last() {
                Some(seg) => seg.name.name.as_str(),
                None => return false,
            },
            ExprKind::MemberAccess { member, .. } => member.name.as_str(),
            _ => return false,
        };
        self.indexed_coll_root(name)
    }

    /// §7.4.5/§7.6/§7.10: `q3[0] = q3[1]`, `s.sp[3] = '{..}`, `r = q3[1]` —
    /// a whole-collection assignment whose target or source is an element
    /// of a fixed array of collections. The scalar element store paths
    /// would treat the element as one value. Returns whether it assigned.
    #[cold]
    #[inline(never)]
    pub(super) fn try_indexed_coll_assign(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
    ) -> bool {
        if self.may_name_indexed_coll(lvalue) {
            let Some(name) = self.indexed_coll_name(lvalue) else {
                return false;
            };
            let dt = self
                .struct_member_coll_of(&name)
                .map(|(_, dt, _)| dt)
                .or_else(|| {
                    let root = name.split_once('[').map_or(name.as_str(), |(r, _)| r);
                    self.module.var_decl_types.get(root).cloned()
                });
            // §7.12.1: a locator method's result queue.
            if let Some((arr, method, filter, iter)) = self.locator_call(rvalue) {
                let idxs =
                    self.locator_indices_named(&arr, &method, filter.as_ref(), iter.as_deref());
                self.materialize_locator(&name, &arr, &method, &idxs);
                return true;
            }
            return self.assign_whole_coll(&name, dt.as_ref(), rvalue);
        }
        if self.may_name_indexed_coll(rvalue)
            && let Some(dst) = self.expr_assoc_name(lvalue).or_else(|| match &lvalue.kind {
                ExprKind::Ident(h) => Some(self.resolve_hier_name(h).into_owned()),
                _ => None,
            })
            && (self.module.dynamic_arrays.contains(&dst) || self.is_associative_array(&dst))
            && let Some(src) = self.indexed_coll_name(rvalue)
        {
            if src == dst {
                return true;
            }
            if self.is_associative_array(&dst) {
                self.copy_assoc_entries(&dst, &src);
            } else {
                self.copy_whole_queue(&dst, &src);
            }
            return true;
        }
        false
    }

    /// [`indexed_coll_name`] behind the name gate (resolving evaluates the
    /// selects, so only a select of a listed root goes further).
    #[cold]
    #[inline(never)]
    pub(super) fn indexed_coll_name_gated(&mut self, e: &Expression) -> Option<String> {
        if !self.may_name_indexed_coll(e) {
            return None;
        }
        self.indexed_coll_name(e)
    }

    /// `q3[1]` / `s.sp[2]` written as one hierarchical identifier whose
    /// segments carry the selects (the `foreach` array shape).
    #[cold]
    #[inline(never)]
    pub(super) fn selected_ident_coll_name(&mut self, e: &Expression) -> Option<String> {
        let ExprKind::Ident(h) = &e.kind else {
            return None;
        };
        if !self.indexed_coll_root(&h.path.last()?.name.name) {
            return None;
        }
        let chain = Self::hier_select_chain(h, e.span);
        self.indexed_coll_name(&chain)
    }

    #[cold]
    #[inline(never)]
    pub(super) fn indexed_coll_name(&mut self, e: &Expression) -> Option<String> {
        if let Some(n) = self.flat_member_name(e)
            && (self.module.dynamic_arrays.contains(&n)
                || self.module.associative_arrays.contains_key(&n)
                || (self.struct_colls_possible() && self.ensure_struct_member_coll(&n)))
        {
            return Some(n);
        }
        // An element of a class property that is a fixed array of
        // collections lives under the object-scoped key `<h>#q[i]`.
        self.class_fixed_array_of_coll_key(e)
    }

    /// Evaluate a scalar expression to an i64 in a `&self` context (literals,
    /// const exprs, and simple loop/local/signal variables). Used to resolve
    /// array indices when computing instance-scoped member names.
    /// Immutable resolution of an array-index expression to a `Value` (for the
    /// `&self` handle path). Covers the forms that appear as assoc keys: a
    /// string literal, a local/signal variable (e.g. a `string name`), else a
    /// scalar. Used to key string-keyed assoc element handle reads.
    pub(super) fn eval_index_value_self(&self, e: &Expression) -> Option<Value> {
        match &e.kind {
            ExprKind::StringLiteral(s) => Some(Value::from_string(s)),
            ExprKind::Ident(h) if h.path.len() == 1 => {
                let n = &h.path[0].name.name;
                self.local_stack
                    .last()
                    .and_then(|f| f.get(n).cloned())
                    .or_else(|| self.get_signal_value_by_name(n))
            }
            _ => self
                .eval_scalar_self(e)
                .map(|i| Value::from_u64(i as u64, 32)),
        }
    }

    /// True when `e` is built only from literals, plain names, and
    /// arithmetic — the shapes the elaboration-time constant folder can
    /// settle without any per-call setup. A call, system call, `$bits(type)`,
    /// or member access is left to the runtime paths: for those the folder
    /// clones the whole function table (or typedef table) into a throwaway
    /// elaboration shell on every attempt, and at runtime the attempt almost
    /// always fails anyway (the function needs `this` or a live argument).
    /// On the axi4 UVM base test that throwaway was 6.5 % of all samples.
    pub(super) fn const_fold_shape(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Number(_) => true,
            ExprKind::Ident(h) => h.path.iter().all(|s| s.selects.is_empty()),
            ExprKind::Unary { operand, .. } => Self::const_fold_shape(operand),
            ExprKind::Paren(inner) => Self::const_fold_shape(inner),
            ExprKind::Binary { left, right, .. } => {
                Self::const_fold_shape(left) && Self::const_fold_shape(right)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::const_fold_shape(condition)
                    && Self::const_fold_shape(then_expr)
                    && Self::const_fold_shape(else_expr)
            }
            _ => false,
        }
    }

    pub(super) fn eval_scalar_self(&self, e: &Expression) -> Option<i64> {
        if Self::const_fold_shape(e) {
            if let Some(v) = super::elaborate::const_eval_i64_with_params(e, None) {
                return Some(v);
            }
        }
        if let ExprKind::Ident(h) = &e.kind {
            if h.path.len() == 1 {
                let name = &h.path[0].name.name;
                if let Some(locals) = self.local_stack.last() {
                    if let Some(v) = locals.get(name) {
                        return v.to_u64().map(|x| x as i64);
                    }
                }
                return self
                    .get_signal_value_by_name(name)
                    .and_then(|v| v.to_u64())
                    .map(|x| x as i64);
            }
        }
        None
    }

    /// §8.9: whether `member`, looked up from class `cn` up its chain, is a
    /// static property. The nearest declaration wins, so an instance
    /// property of a derived class hides a same-named static of its base.
    pub(super) fn member_is_static_prop(&self, cn: &str, member: &str) -> bool {
        if !self.class_member_names().statics.contains(member) {
            return false;
        }
        let mut cur: Option<&str> = Some(cn);
        let mut guard = 0;
        while let Some(c) = cur {
            guard += 1;
            let Some(cd) = self.module.classes.get(c) else {
                return false;
            };
            if cd.static_properties.contains(member) {
                return true;
            }
            if cd.properties.contains_key(member) || guard > 64 {
                return false;
            }
            cur = cd.extends.as_deref();
        }
        false
    }

    /// The `#(...)` specialization signature of class `class_name` given
    /// its declared arguments (none: the default specialization), keyed the
    /// way an object constructed with those arguments keys its statics.
    /// None for a class without parameters or arguments it cannot spell.
    pub(super) fn spec_sig_for_args(
        &self,
        class_name: &str,
        type_args: Option<&[Expression]>,
    ) -> Option<String> {
        self.spec_sig_for_args_of(None, class_name, type_args)
    }

    /// `spec_sig_for_args` for a declaration made in the class of object
    /// `owner` (see `concrete_spec_frags_of`).
    pub(super) fn spec_sig_for_args_of(
        &self,
        owner: Option<usize>,
        class_name: &str,
        type_args: Option<&[Expression]>,
    ) -> Option<String> {
        if !self.class_is_parameterized(class_name) {
            return None;
        }
        let raw_sig = match type_args {
            Some(ta) if !ta.is_empty() => {
                // A bare parameter name stays a name here, for
                // `concrete_spec_frags_of` to bind in its own scope.
                let params = self.scope_params(owner);
                let frags: Vec<String> = ta
                    .iter()
                    .filter_map(|e| match &e.kind {
                        ExprKind::Ident(h)
                            if h.path.len() == 1
                                && params.iter().any(|(n, _)| *n == h.path[0].name.name) =>
                        {
                            Some(h.path[0].name.name.clone())
                        }
                        _ => self.expr_to_spec_fragment(e),
                    })
                    .collect();
                if frags.len() != ta.len() {
                    return None;
                }
                self.concrete_spec_frags_of(owner, frags)?.join(",")
            }
            _ => String::new(),
        };
        let sig = self.canonicalize_spec_sig(class_name, &raw_sig);
        let effective = if sig.is_empty() {
            self.module
                .classes
                .get(class_name)
                .map(|cd| {
                    cd.type_param_defaults
                        .iter()
                        .map(|(_, d)| d.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default()
        } else {
            sig
        };
        (!effective.is_empty()).then_some(effective)
    }

    /// Record a class-typed formal's declared `#(...)` arguments, as a local
    /// declaration records its own (§8.25: they name its specialization).
    pub(super) fn note_formal_type_args(&mut self, name: &str, type_args: &[Expression]) {
        if !type_args.is_empty() {
            self.var_type_args
                .insert(name.to_string(), type_args.to_vec());
        } else if !self.var_type_args.is_empty() {
            self.var_type_args.remove(name);
        }
    }

    /// The `#(...)` arguments variable `var` was declared with: a procedural
    /// local or formal of the running frame, a module-scope variable, or a
    /// property of `this`.
    pub(super) fn declared_type_args(
        &self,
        var: &str,
        resolved: Option<&str>,
    ) -> Option<Vec<Expression>> {
        let in_frame = self.local_stack.last().is_some_and(|f| f.contains_key(var));
        if in_frame {
            if let Some(ta) = self.var_type_args.get(var) {
                return Some(ta.clone());
            }
        }
        if let Some(ta) = self.this_property_type_args(var) {
            return Some(ta);
        }
        if let Some(ta) = resolved.and_then(|r| self.module.class_type_args.get(r)) {
            return Some(ta.clone());
        }
        if let Some(ta) = self.module.class_type_args.get(var) {
            return Some(ta.clone());
        }
        self.var_type_args.get(var).cloned()
    }

    /// The class variable `var` is declared with, and the specialization
    /// that declaration names (§8.25), or None for a class without
    /// parameters. The specialization is spelled `(class, canonical sig)`.
    pub(super) fn declared_class_own_spec(
        &self,
        var: &str,
        resolved: Option<&str>,
    ) -> Option<(String, Option<(String, String)>)> {
        if let Some((b, spec, _)) = self.this_property_decl(var) {
            return Some((b, spec));
        }
        let c = match self.class_of_var(var).or_else(|| {
            resolved
                .filter(|r| *r != var)
                .and_then(|r| self.class_of_var(r))
        }) {
            Some(c) => c,
            None => {
                // A variable declared through a typedef of a specialization.
                let t = self.var_class_types.get(var).cloned().or_else(|| {
                    self.signal_name_to_id
                        .get(resolved.unwrap_or(var))
                        .or_else(|| self.signal_name_to_id.get(var))
                        .and_then(|id| self.signal_type_names.get(id))
                        .cloned()
                })?;
                let (b, sig) = self.resolve_typedef_spec(&t)?;
                if !self.module.classes.contains_key(&b) {
                    return None;
                }
                let spec = self
                    .concrete_spec_sig(&sig)
                    .map(|s| (b.clone(), self.canonicalize_spec_sig(&b, &s)));
                return Some((b, spec));
            }
        };
        let (base, args) = Self::strip_class_specialization(&c);
        if !self.module.classes.contains_key(&base) {
            return None;
        }
        let sig = match args {
            Some(a) => self
                .concrete_spec_sig(&a)
                .map(|a| self.canonicalize_spec_sig(&base, &a)),
            None => {
                let ta = self.declared_type_args(var, resolved);
                if let Some(ta) = ta.as_deref().filter(|a| !a.is_empty()) {
                    self.spec_sig_for_args(&base, Some(ta))
                } else {
                    let typedef_sig = self
                        .local_typedef_type_of(var)
                        .or_else(|| self.var_typedef_types.get(var).cloned())
                        .or_else(|| {
                            self.signal_name_to_id
                                .get(resolved.unwrap_or(var))
                                .or_else(|| self.signal_name_to_id.get(var))
                                .and_then(|id| self.signal_type_names.get(id))
                                .cloned()
                        })
                        .and_then(|t| self.resolve_typedef_spec(&t))
                        .filter(|(b, _)| b == &base)
                        .and_then(|(b, sig)| {
                            self.concrete_spec_sig(&sig)
                                .map(|s| self.canonicalize_spec_sig(&b, &s))
                        });
                    typedef_sig.or_else(|| self.spec_sig_for_args(&base, None))
                }
            }
        };
        let spec = sig.map(|s| (base.clone(), s));
        Some((base, spec))
    }

    /// A declared specialization's argument fragments with every parameter
    /// of the running class context (`this_type`, `C#(T)` inside `C`)
    /// replaced by its binding there. None when one cannot be resolved: the
    /// spelling then names no concrete specialization.
    pub(super) fn concrete_spec_frags(&self, frags: Vec<String>) -> Option<Vec<String>> {
        self.concrete_spec_frags_of(None, frags)
    }

    /// `concrete_spec_frags` for a declaration made in the class of object
    /// `owner` (a property's type): its parameters resolve through that
    /// object's bindings, or the running context's when it is `this`.
    pub(super) fn concrete_spec_frags_of(
        &self,
        owner: Option<usize>,
        frags: Vec<String>,
    ) -> Option<Vec<String>> {
        let this_h = self.this_stack.last().copied().flatten();
        let foreign = owner.filter(|o| Some(*o) != this_h);
        let params = self.scope_params(owner);
        frags
            .into_iter()
            .map(|f| {
                let t = f.trim();
                match params.iter().find(|(n, _)| *n == t) {
                    Some((_, true)) => match foreign {
                        Some(o) => self
                            .heap
                            .get(o)
                            .and_then(|i| i.as_ref())
                            .and_then(|i| i.type_bindings.get(t).cloned()),
                        None => self.resolve_type_param_binding(t),
                    },
                    Some((_, false)) => {
                        // A value parameter: the object's own value, held
                        // as a property of it.
                        let o = foreign.or(this_h)?;
                        let v = self.heap.get(o)?.as_ref()?.properties.get(t)?;
                        (!v.is_real && v.to_u64().is_some())
                            .then(|| v.to_u64().unwrap().to_string())
                    }
                    // §25.9: a virtual-interface argument names the parameters
                    // inside its own `#(...)` (`db#(virtual my_if#(T)) h;`).
                    None => match self.parse_vif_frag(t) {
                        Some((iname, args, modport)) => {
                            let args = self.concrete_spec_frags_of(owner, args)?;
                            Some(self.canonical_vif_frag(&iname, args, modport))
                        }
                        None => Some(f),
                    },
                }
            })
            .collect()
    }

    /// The parameters (name, is-type) in scope of a declaration made in the
    /// class of object `owner`, or in the running class context.
    pub(super) fn scope_params(&self, owner: Option<usize>) -> Vec<(String, bool)> {
        let this_h = self.this_stack.last().copied().flatten();
        let scope: Option<String> = match owner.filter(|o| Some(*o) != this_h) {
            Some(o) => self
                .heap
                .get(o)
                .and_then(|i| i.as_ref())
                .map(|i| i.class_name.clone()),
            None => self.class_context_stack.last().cloned().flatten(),
        };
        let mut params: Vec<(String, bool)> = Vec::new();
        let mut cur: Option<&str> = scope.as_deref();
        let mut guard = 0;
        while let Some(c) = cur {
            guard += 1;
            let Some(cd) = self.module.classes.get(c) else {
                break;
            };
            params.extend(cd.type_param_names.iter().map(|n| (n.clone(), true)));
            params.extend(cd.param_defaults.iter().map(|(n, _)| (n.clone(), false)));
            if guard > 64 {
                break;
            }
            cur = cd.extends.as_deref();
        }
        params
    }

    /// `concrete_spec_frags` over a comma-joined signature.
    pub(super) fn concrete_spec_sig(&self, sig: &str) -> Option<String> {
        let frags: Vec<String> = Self::split_spec_args(sig)
            .into_iter()
            .map(|a| a.trim().to_string())
            .collect();
        Some(self.concrete_spec_frags(frags)?.join(","))
    }

    /// `declared_class_own_spec`, where a class without parameters of its
    /// own that extends a specialization takes its inherited statics from
    /// that one: what a static member reached through `var` refers to.
    pub(super) fn declared_class_spec(
        &self,
        var: &str,
        resolved: Option<&str>,
    ) -> Option<(String, Option<(String, String)>)> {
        let (base, spec) = self.declared_class_own_spec(var, resolved)?;
        let spec = spec.or_else(|| self.inherited_param_spec(&base));
        Some((base, spec))
    }

    /// §8.25: the declared type of a `$cast` destination, spelled
    /// `Class#(sig)` when the class has parameters, for a variable declared
    /// anywhere: a local, a formal, a module-scope variable, a class property
    /// (`obj.p`, `this.p`, bare `p` in a method) or through a typedef. An
    /// unspecialized declaration names the default specialization, except
    /// inside the class's own declaration, where it is left unspecialized.
    /// None when the destination is not a recognized class variable.
    pub(super) fn cast_dest_declared_type(&self, dest: &Expression) -> Option<String> {
        let spelled =
            |sim: &Self, base: String, spec: Option<(String, String)>, explicit: bool| match spec {
                Some((_, sig)) if explicit || !sim.inside_class_decl_of(&base) => {
                    format!("{base}#({sig})")
                }
                _ => base,
            };
        match &dest.kind {
            ExprKind::Ident(hh) if hh.path.len() == 1 => {
                let n = &hh.path[0].name.name;
                let dt = self.class_of_var(n);
                // A class without parameters, a type parameter or an
                // already-specialized spelling keeps the caller's handling.
                if let Some(c) = &dt {
                    if c.contains('#')
                        || !self.module.classes.contains_key(c)
                        || !self.class_is_parameterized(c)
                    {
                        return dt;
                    }
                }
                if let Some((b, spec, explicit)) = self.this_property_decl(n) {
                    return Some(spelled(self, b, spec, explicit));
                }
                let resolved = self.resolve_hier_name(hh);
                let explicit = self
                    .declared_type_args(n, Some(resolved.as_ref()))
                    .is_some()
                    || dt.is_none();
                match self.declared_class_own_spec(n, Some(resolved.as_ref())) {
                    Some((base, spec)) => Some(spelled(self, base, spec, explicit)),
                    None => dt,
                }
            }
            ExprKind::Ident(hh) if hh.path.len() == 2 && hh.path[0].selects.is_empty() => {
                let h = self.eval_ident_handle(&hh.path[0].name.name)?;
                self.cast_dest_property_type(h, &hh.path[1].name.name)
                    .map(|(b, spec, explicit)| spelled(self, b, spec, explicit))
            }
            // A DEEPER flattened path (`sq.p_sequencer.r0` parses as
            // Ident([sq, p_sequencer, r0])): walk every prefix segment to an
            // object handle, then resolve the leaf property like the 2-segment
            // arm. Without this such a dest resolved to nothing and the $cast
            // was permissive (§8.22).
            ExprKind::Ident(hh)
                if hh.path.len() > 2 && hh.path.iter().all(|s| s.selects.is_empty()) =>
            {
                let mut handle: Option<usize> = None;
                for (i, seg) in hh.path[..hh.path.len() - 1].iter().enumerate() {
                    let name = &seg.name.name;
                    if i == 0 {
                        handle = self
                            .eval_ident_handle(name)
                            .map(|h| h as usize)
                            .filter(|&h| h != 0 && h < self.heap.len())
                            .or_else(|| {
                                self.local_stack
                                    .last()
                                    .and_then(|m| m.get(name.as_str()))
                                    .and_then(|v| v.to_u64())
                                    .map(|h| h as usize)
                                    .filter(|&h| h != 0 && h < self.heap.len())
                            });
                    } else if let Some(h) = handle {
                        handle = self
                            .heap
                            .get(h)
                            .and_then(|o| o.as_ref())
                            .and_then(|inst| inst.properties.get(name.as_str()))
                            .and_then(|v| v.to_u64())
                            .map(|h| h as usize)
                            .filter(|&h| h != 0 && h < self.heap.len());
                    }
                    if handle.is_none() {
                        return None;
                    }
                }
                let h = handle?;
                self.cast_dest_property_type(h, &hh.path.last().unwrap().name.name)
                    .map(|(b, spec, explicit)| spelled(self, b, spec, explicit))
            }
            ExprKind::MemberAccess { expr: base, member } => {
                let h = self.eval_handle_expr(base)?;
                self.cast_dest_property_type(h, &member.name)
                    .map(|(b, spec, explicit)| spelled(self, b, spec, explicit))
            }
            _ => None,
        }
    }

    /// The declared class, specialization and whether it was spelled out,
    /// of property `prop` of the object `handle`.
    pub(super) fn cast_dest_property_type(
        &self,
        handle: usize,
        prop: &str,
    ) -> Option<(String, Option<(String, String)>, bool)> {
        let inst = self.heap.get(handle)?.as_ref()?;
        let mut cur = Some(inst.class_name.as_str());
        let mut guard = 0;
        while let Some(cn) = cur {
            guard += 1;
            let cd = self.module.classes.get(cn)?;
            if let Some(sig) = cd.properties.get(prop) {
                let tn = sig.type_name.as_deref()?;
                // A typedef local to the owner's class (`this_type`) names
                // the owner's own specialization: bind its arguments there.
                if let Some(DataType::TypeReference {
                    name, type_args, ..
                }) = self.class_local_typedef(&inst.class_name, tn)
                {
                    let b = name.name.name.as_str();
                    if self.module.classes.contains_key(b) {
                        let spec = self
                            .spec_sig_for_args_of(Some(handle), b, Some(type_args.as_slice()))
                            .map(|s| (b.to_string(), s));
                        return Some((b.to_string(), spec, true));
                    }
                }
                if self.module.classes.contains_key(tn) {
                    let ta = cd.property_type_args.get(prop);
                    let spec = self
                        .spec_sig_for_args_of(Some(handle), tn, ta.map(|v| v.as_slice()))
                        .map(|s| (tn.to_string(), s));
                    return Some((tn.to_string(), spec, ta.is_some_and(|v| !v.is_empty())));
                }
                // The property's declared type is a TYPE PARAMETER of the
                // owning class (`RSP r0;` in `sqr_t#(type REQ, type RSP=REQ)`;
                // the `uvm_declare_p_sequencer`-style member's sibling). A
                // $cast INTO such a member must compare the source against
                // the parameter's CONCRETE binding on this instance —
                // without this the dest type stayed unresolved and the cast
                // was permissive, letting a wrong class slip through (§8.22).
                if cd.type_param_names.iter().any(|n| n == tn) {
                    if let Some(b) = inst.type_bindings.get(tn).cloned() {
                        if let Some((base, sig)) = self.extract_spec_from_string(&b) {
                            if self.module.classes.contains_key(&base) {
                                let b0 = base.clone();
                                return Some((base, Some((b0, sig)), true));
                            }
                        } else if self.module.classes.contains_key(&b) {
                            return Some((b, None, false));
                        }
                    }
                }
                let (b, s) = self.resolve_typedef_spec(tn)?;
                if !self.module.classes.contains_key(&b) {
                    return None;
                }
                let frags: Vec<String> = Self::split_spec_args(&s)
                    .into_iter()
                    .map(|a| a.trim().to_string())
                    .collect();
                let spec = self
                    .concrete_spec_frags_of(Some(handle), frags)
                    .map(|f| (b.clone(), self.canonicalize_spec_sig(&b, &f.join(","))));
                return Some((b, spec, true));
            }
            if guard > 64 {
                return None;
            }
            cur = cd.extends.as_deref();
        }
        None
    }

    /// The target of typedef `tn` declared in class `cn` or an ancestor.
    pub(super) fn class_local_typedef(&self, cn: &str, tn: &str) -> Option<&DataType> {
        let mut cur: Option<&str> = Some(cn);
        let mut guard = 0;
        while let Some(c) = cur {
            guard += 1;
            let cd = self.module.classes.get(c)?;
            if let Some(dt) = cd.typedef_targets.get(tn) {
                return Some(dt);
            }
            if guard > 64 {
                return None;
            }
            cur = cd.extends.as_deref();
        }
        None
    }

    /// `cast_dest_property_type` for a bare name inside a method that is a
    /// property of `this` and not a local or formal of the running frame.
    pub(super) fn this_property_decl(
        &self,
        var: &str,
    ) -> Option<(String, Option<(String, String)>, bool)> {
        let h = self.this_stack.last().copied().flatten()?;
        if self.local_stack.last().is_some_and(|f| f.contains_key(var)) {
            return None;
        }
        self.cast_dest_property_type(h, var)
    }

    /// Whether the running code is inside the declaration of class `base`
    /// or of a class derived from it, where the bare class name need not
    /// mean its default specialization.
    pub(super) fn inside_class_decl_of(&self, base: &str) -> bool {
        match self.class_context_stack.last() {
            Some(Some(ctx)) => self.class_is_a(ctx, base),
            _ => false,
        }
    }

    /// §8.9: the class and specialization whose static `member` an object
    /// reference names. The reference's declared type decides (`var`, when
    /// the reference is a variable), so a static is reachable through a null
    /// handle; a live object of an unrelated recorded type, or an object
    /// reached through a chain, uses the object's own class. None when
    /// `member` is not a static property of that class.
    pub(super) fn static_member_target(
        &self,
        handle: usize,
        var: Option<&str>,
        resolved: Option<&str>,
        member: &str,
    ) -> Option<(String, Option<(String, String)>)> {
        if !self.class_member_names().statics.contains(member) {
            return None;
        }
        let live = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .filter(|_| handle != 0);
        let decl = var.and_then(|v| self.declared_class_spec(v, resolved));
        let (cn, spec) = match (live, decl) {
            (Some(inst), Some((dc, dspec))) if self.class_is_a(&inst.class_name, &dc) => {
                // The object knows its specialization; the declaration's
                // spelling may still name type parameters (`this_type h;`).
                let from_obj = match &inst.spec {
                    Some((b, sig)) if *b == dc => Some((b.clone(), sig.clone())),
                    Some((b, sig)) => self
                        .ancestor_spec(b, sig, &dc)
                        .map(|a| (dc.clone(), self.canonicalize_spec_sig(&dc, &a))),
                    None => None,
                };
                let spec = from_obj.or(dspec);
                // The object's own entry for the declared class: a class
                // whose base is a type parameter has one per specialization
                // (`wrap_c<derived_c>`), each with its own base chain.
                let entry = self.chain_entry_of(&inst.class_name, &dc).unwrap_or(dc);
                (entry, spec)
            }
            (Some(inst), _) => (inst.class_name.clone(), inst.spec.clone()),
            (None, Some((dc, spec))) => {
                // A class whose base is a type parameter: the declared
                // specialization's own entry carries its base chain.
                let entry = match &spec {
                    Some((_, sig)) if self.is_dep_class(&dc) => {
                        let args: Vec<String> = Self::split_spec_args(sig)
                            .into_iter()
                            .map(|a| a.trim().to_string())
                            .collect();
                        let e = self.spec_class_name(&dc, &args);
                        if self.module.classes.contains_key(&e) {
                            e
                        } else {
                            dc
                        }
                    }
                    _ => dc,
                };
                (entry, spec)
            }
            (None, None) => return None,
        };
        self.member_is_static_prop(&cn, member)
            .then_some((cn, spec))
    }

    /// The nearest class entry on `start`'s chain made from class `origin`.
    pub(super) fn chain_entry_of(&self, start: &str, origin: &str) -> Option<String> {
        let mut cur: Option<&str> = Some(start);
        let mut guard = 0;
        while let Some(c) = cur {
            guard += 1;
            if guard > 64 {
                return None;
            }
            if self.class_origin(c) == origin {
                return Some(c.to_string());
            }
            cur = self
                .module
                .classes
                .get(c)
                .and_then(|cd| cd.extends.as_deref());
        }
        None
    }

    /// §8.9: read static property `member` through an object reference (see
    /// `static_member_target`).
    pub(super) fn static_read_via_handle(
        &mut self,
        handle: usize,
        var: Option<&str>,
        resolved: Option<&str>,
        member: &str,
    ) -> Option<Value> {
        let (cn, spec) = self.static_member_target(handle, var, resolved, member)?;
        self.static_read_in(&cn, spec, member)
    }

    /// Read static `member` of class `cn` in specialization `spec`.
    pub(super) fn static_read_in(
        &mut self,
        cn: &str,
        spec: Option<(String, String)>,
        member: &str,
    ) -> Option<Value> {
        let saved = spec.map(|s| std::mem::replace(&mut self.current_spec, Some(s)));
        let v = self.class_static_get(cn, member);
        if let Some(s) = saved {
            self.current_spec = s;
        }
        v
    }

    /// §8.9: the class and specialization of the static `member` reached
    /// through property `prop` of object `owner` when that property holds
    /// null: the property's declared type decides.
    pub(super) fn null_property_static_target(
        &self,
        owner: usize,
        prop: &str,
        member: &str,
    ) -> Option<(String, Option<(String, String)>)> {
        let (b, spec, _) = self.cast_dest_property_type(owner, prop)?;
        if !self.member_is_static_prop(&b, member) {
            return None;
        }
        let spec = spec.or_else(|| self.inherited_param_spec(&b));
        Some((b, spec))
    }

    /// For a handle-chain expression `a.b.p` (either parse shape), the live
    /// object `a.b` and the property name `p`.
    pub(super) fn handle_chain_owner(&self, e: &Expression) -> Option<(usize, String)> {
        let live = |h: usize| h != 0 && self.heap.get(h).is_some_and(|o| o.is_some());
        match &e.kind {
            ExprKind::MemberAccess {
                expr: owner,
                member,
            } => {
                let o = self.eval_handle_expr(owner).filter(|&h| live(h))?;
                Some((o, member.name.clone()))
            }
            ExprKind::Ident(h)
                if h.path.len() >= 2 && h.path.iter().all(|s| s.selects.is_empty()) =>
            {
                let n = h.path.len();
                let mut o = self.eval_ident_handle(&h.path[0].name.name)?;
                for seg in &h.path[1..n - 1] {
                    if !live(o) {
                        return None;
                    }
                    o = self.member_handle(o, &seg.name.name)?;
                }
                live(o).then(|| (o, h.path[n - 1].name.name.clone()))
            }
            _ => None,
        }
    }

    /// §8.9: `a.b.s = v` / `f.g.s = v` — a static property at the end of a
    /// chain of handles is the shared cell of the last object's class. A
    /// single-variable receiver (`h.s`) resolves by its declared type in the
    /// `assign_value` arm instead. None when `lhs` is not such a write.
    pub(super) fn assign_static_through_chain(
        &mut self,
        lhs: &Expression,
        val: &Value,
    ) -> Option<bool> {
        let statics = &self.class_member_names().statics;
        if statics.is_empty() {
            return None;
        }
        let live = |sim: &Self, h: usize| h != 0 && sim.heap.get(h).is_some_and(|o| o.is_some());
        // (receiver handle, or the live owner and property holding null)
        let (h, null_in, member) = match &lhs.kind {
            ExprKind::MemberAccess { expr: recv, member } => {
                if !statics.contains(member.name.as_str())
                    || matches!(&recv.kind, ExprKind::Ident(rh) if rh.path.len() == 1)
                {
                    return None;
                }
                match self.eval_handle_expr(recv).filter(|&h| live(self, h)) {
                    Some(h) => (h, None, &member.name),
                    None => {
                        let (o, p) = self.handle_chain_owner(recv)?;
                        (0, Some((o, p)), &member.name)
                    }
                }
            }
            ExprKind::Ident(hier)
                if hier.path.len() >= 3 && hier.path.iter().all(|s| s.selects.is_empty()) =>
            {
                let n = hier.path.len();
                if !statics.contains(hier.path[n - 1].name.name.as_str())
                    || self.module.classes.contains_key(&hier.path[0].name.name)
                {
                    return None;
                }
                let mut h = self.eval_ident_handle(&hier.path[0].name.name)?;
                let mut null_in = None;
                for (k, seg) in hier.path[1..n - 1].iter().enumerate() {
                    if !live(self, h) {
                        return None;
                    }
                    let next = self.member_handle(h, &seg.name.name).unwrap_or(0);
                    if !live(self, next) && k == n - 3 {
                        null_in = Some((h, seg.name.name.clone()));
                    }
                    h = next;
                }
                (h, null_in, &hier.path[n - 1].name.name)
            }
            _ => return None,
        };
        if live(self, h) {
            return self.static_write_via_handle(h, None, None, member, val.clone());
        }
        let (owner, prop) = null_in?;
        let (cn, spec) = self.null_property_static_target(owner, &prop, member)?;
        Some(self.static_write_in(&cn, spec, member, val.clone()))
    }

    /// §8.9: write static property `member` through an object reference
    /// (see `static_member_target`). None when it is not a static.
    pub(super) fn static_write_via_handle(
        &mut self,
        handle: usize,
        var: Option<&str>,
        resolved: Option<&str>,
        member: &str,
        val: Value,
    ) -> Option<bool> {
        let (cn, spec) = self.static_member_target(handle, var, resolved, member)?;
        Some(self.static_write_in(&cn, spec, member, val))
    }

    /// Write static `member` of class `cn` in specialization `spec`.
    pub(super) fn static_write_in(
        &mut self,
        cn: &str,
        spec: Option<(String, String)>,
        member: &str,
        val: Value,
    ) -> bool {
        let cn = cn.to_string();
        let mut v = val;
        if let Some(w) = self.class_prop_width(&cn, member) {
            if w != v.width && !v.is_real {
                v = v.resize_for_assign(w);
            }
        }
        let saved = spec.map(|s| std::mem::replace(&mut self.current_spec, Some(s)));
        let ok = self.class_static_set(&cn, member, v);
        if let Some(s) = saved {
            self.current_spec = s;
        }
        ok
    }

    /// Read `handle.member` as a VALUE (handle or scalar): instance property
    /// first, then the instance's CLASS static cell. Mirrors `member_handle`
    /// for the value-reading path used by `eval_expr`.
    pub(super) fn read_member_value(&self, handle: usize, member: &str) -> Option<Value> {
        let inst = self.heap.get(handle).and_then(|o| o.as_ref());
        if let Some(v) = inst.and_then(|i| i.properties.get(member).cloned()) {
            return Some(v);
        }
        let cn = inst.map(|i| i.class_name.clone())?;
        let spec = inst.and_then(|i| i.spec.as_ref());
        let key = self.static_prop_key_spec(&cn, member, spec)?;
        self.class_statics.get(&key).cloned()
    }

    /// Resolve a handle for `handle.member`: try the instance property first,
    /// then fall back to the instance's CLASS static cell (§8.25: a static
    /// property accessed through an instance handle refers to the class's
    /// shared static). Returns None if neither holds a handle.
    pub(super) fn member_handle(&self, handle: usize, member: &str) -> Option<usize> {
        let inst = self.heap.get(handle).and_then(|o| o.as_ref());
        if let Some(h) = inst
            .and_then(|i| i.properties.get(member))
            .and_then(|v| v.to_u64())
            .map(|h| h as usize)
        {
            return Some(h);
        }
        let cn = inst.map(|i| i.class_name.clone())?;
        let spec = inst.and_then(|i| i.spec.as_ref());
        let key = self.static_prop_key_spec(&cn, member, spec)?;
        self.class_statics
            .get(&key)
            .and_then(|v| v.to_u64())
            .map(|h| h as usize)
    }

    /// §13.3: selected object receivers must enter the suspend-aware task
    /// path, not execute a blocking task through the synchronous fallback.
    pub(super) fn eval_live_object_receiver(&mut self, expr: &Expression) -> Option<usize> {
        let handle = self.eval_expr(expr).to_u64()? as usize;
        (handle != 0
            && self.heap.get(handle).is_some_and(|o| o.is_some())
            && !self.mailboxes.contains_key(&handle)
            && !self.semaphores.contains_key(&handle))
        .then_some(handle)
    }

    /// Resolve a handle-valued expression (`h`, `arr[i]`) to its heap handle in
    /// a `&self` context, applying instance scoping for member-array bases.
    pub(super) fn eval_handle_expr(&self, expr: &Expression) -> Option<usize> {
        match &expr.kind {
            // `this` resolves to the current object handle. Needed so
            // `this.member` selects/associative-writes resolve to
            // `<handle>#member` (e.g. `this.m_successors[k]=1`).
            ExprKind::This => self.this_stack.last().copied().flatten(),
            ExprKind::Ident(h) if h.path.len() == 1 => self.eval_ident_handle(&h.path[0].name.name),
            // Flattened multi-segment handle path (`a.b.c`): resolve the head
            // handle, then walk the remaining segments as property reads.
            ExprKind::Ident(h) if h.path.len() >= 2 => {
                let mut handle = self.eval_ident_handle(&h.path[0].name.name)?;
                for seg in &h.path[1..] {
                    if handle == 0 {
                        return None;
                    }
                    handle = self.member_handle(handle, &seg.name.name)?;
                }
                Some(handle)
            }
            // `<expr>.member` — resolve the base to a handle, then read the
            // member property (also a handle). Enables deep object chains
            // like `obj.sub_obj.queue`.
            ExprKind::MemberAccess { expr: base, member } => {
                // `ClassName::static_handle` base — the static property holds
                // an object handle. The recursive `eval_handle_expr(base)`
                // below would treat the class name as a variable (via
                // `eval_ident_handle`) and fail. Resolve the static cell here
                // so that `ClassName::h.assoc_member` (e.g. a singleton
                // reached through a static `m_inst`) yields the same
                // handle a local copy of that handle would, and the assoc
                // store keys to `<handle>#member` consistently.
                if let ExprKind::Ident(bh) = &base.kind {
                    if bh.path.len() == 1 && self.module.classes.contains_key(&bh.path[0].name.name)
                    {
                        return self
                            .static_prop_key(&bh.path[0].name.name, &member.name)
                            .and_then(|key| self.class_statics.get(&key))
                            .and_then(|v| v.to_u64())
                            .map(|h| h as usize);
                    }
                }
                let bh = self.eval_handle_expr(base)?;
                if bh == 0 {
                    return None;
                }
                self.member_handle(bh, &member.name)
            }
            ExprKind::Index { expr: base, index } => match &base.kind {
                ExprKind::Ident(bh) if bh.path.len() == 1 => {
                    let bname = bh.path[0].name.name.as_str();
                    let scoped = self
                        .instance_assoc_member(bname)
                        .unwrap_or_else(|| bname.to_string());
                    self.coll_elem_handle(&scoped, bname, index)
                }
                // An element of a collection PROPERTY reached through a
                // handle — `e.ag[0]`, `a.b.ag[i]` (either parse shape).
                ExprKind::MemberAccess { expr: ob, member } => {
                    let oh = self.eval_handle_expr(ob).filter(|&h| h != 0)?;
                    let coll = self.handle_collection_name(oh, &member.name)?;
                    self.coll_elem_handle(&coll, &member.name, index)
                }
                ExprKind::Ident(bh)
                    if bh.path.len() >= 2 && bh.path.iter().all(|s| s.selects.is_empty()) =>
                {
                    let mut oh = self.eval_ident_handle(&bh.path[0].name.name)?;
                    let n = bh.path.len();
                    for seg in &bh.path[1..n - 1] {
                        oh = self.member_handle(oh, &seg.name.name).filter(|&h| h != 0)?;
                    }
                    let member = &bh.path[n - 1].name.name;
                    let coll = self.handle_collection_name(oh, member)?;
                    self.coll_elem_handle(&coll, member, index)
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// The handle held by element `index` of the collection stored as `coll`
    /// (`bare` is its declared name, which fixes the key type).
    pub(super) fn coll_elem_handle(
        &self,
        coll: &str,
        bare: &str,
        index: &Expression,
    ) -> Option<usize> {
        // A STRING-keyed assoc element (`rtab[string]`) must key by the
        // string, not a truncated scalar — otherwise `assoc[key].method()`
        // (a method on the element class handle) resolves to the wrong/empty
        // slot → handle 0 → null receiver. This is string-keyed resource
        // table lookup (`rtab[name].get(i)`).
        let string_keyed = self.is_string_keyed_array(coll) || self.is_string_keyed_array(bare);
        let key_str = if string_keyed {
            self.eval_index_value_self(index)
                .map(|v| v.to_sv_string())
                .unwrap_or_default()
        } else {
            self.eval_scalar_self(index)?.to_string()
        };
        let key = format!("{}[{}]", coll, key_str);
        self.get_signal_value_by_name(&key)
            .and_then(|v| v.to_u64())
            .map(|h| h as usize)
    }

    /// Does `class_name` or any ancestor declare a method `name`?
    /// SV §13.4.1: a function with no arguments may be invoked without
    /// parentheses (`port.size`, `obj.num`). True iff `class_name` or an
    /// ancestor declares a FUNCTION `name` with no formal args — so a
    /// member read `obj.name` that matches no property should dispatch to
    /// it. Tasks and arg-bearing functions are excluded (a bare read can't
    /// supply arguments and a task yields no value). Port checks rely on this
    /// (`if (port.size < 1)`).
    pub(super) fn class_parameterless_function(&self, class_name: &str, name: &str) -> bool {
        use crate::ast::decl::ClassMethodKind;
        let mut cur = Some(class_name.to_string());
        while let Some(cname) = cur {
            if let Some(cd) = self.module.classes.get(&cname) {
                if let Some(m) = cd.methods.get(name) {
                    return matches!(&m.kind, ClassMethodKind::Function(f) if f.ports.is_empty() || f.ports.iter().all(|p| p.default.is_some()));
                }
                cur = cd.extends.clone();
            } else {
                break;
            }
        }
        false
    }

    /// Does `obj.name` (a parenthesless member reference) denote a CLASS-
    /// method invocation per LRM §13.4.1/§18.11? This is true when the class
    /// declares a parameterless function `name`, OR when `name` is a built-in
    /// no-arg method of every class OBJECT.
    ///
    /// UVM relies on the latter for `assert(t1.randomize & t2.randomize)`.
    /// `randomize` is NOT a declared class method (the source never lists the
    /// implicit built-in from `class_object`), so `class_parameterless_function`
    /// correctly returns false for it — but a bare `obj.randomize` must still
    /// dispatch to `exec_method_call(.., "randomize", [])`, which routes to
    /// the §18.11 solver, so both operands of the `&` actually randomize.
    pub(super) fn class_bare_method(&self, class_name: &str, name: &str) -> bool {
        self.class_parameterless_function(class_name, name) || matches!(name, "randomize")
    }

    pub(super) fn class_has_method(&self, class_name: &str, name: &str) -> bool {
        if let Some(cached) = self
            .class_has_method_memo
            .borrow()
            .get(class_name)
            .and_then(|methods| methods.get(name))
        {
            return *cached;
        }
        // Whole-chain member-name set (frozen after elaboration): a miss
        // answers definitively, so cache the negative like the walk does.
        if !self.class_member_names().methods.contains(name) {
            self.class_has_method_memo
                .borrow_mut()
                .entry(class_name.to_string())
                .or_default()
                .insert(name.to_string(), false);
            return false;
        }
        // A function of the class tables alone (fixed once
        // `sanitize_class_hierarchy` has run; `compile` clears this memo then).
        if let Some(&hit) = self
            .has_method_cache
            .borrow()
            .get(class_name)
            .and_then(|m| m.get(name))
        {
            return hit;
        }
        let v = self.class_has_method_walk(class_name, name);
        self.has_method_cache
            .borrow_mut()
            .entry(class_name.to_string())
            .or_default()
            .insert(name.to_string(), v);
        v
    }

    pub(super) fn class_has_method_walk(&self, class_name: &str, name: &str) -> bool {
        let mut cur: Option<&str> = Some(class_name);
        let mut found = false;
        while let Some(cname) = cur {
            if let Some(cd) = self.module.classes.get(cname) {
                if cd.methods.contains_key(name) {
                    found = true;
                    break;
                }
                cur = cd.extends.as_deref();
            } else {
                break;
            }
        }
        self.class_has_method_memo
            .borrow_mut()
            .entry(class_name.to_string())
            .or_default()
            .insert(name.to_string(), found);
        found
    }

    /// Does receiver expression `recv` denote a class handle whose type — or
    /// any ancestor via `extends` — declares a method named `mname`?
    ///
    /// Used to keep the §6.16 built-in *string* methods (`compare`,
    /// `icompare`, `putc`, `itoa`, `tolower`, `atoi`, …) from shadowing a
    /// USER-DEFINED class method of the same name (IEEE 1800-2023 §8.4/§6.16).
    /// Without this guard `obj.compare(other)` on a class declaring `compare`
    /// would silently run the lexicographic `string::compare` builtin
    /// (returning -1/0/1) instead of the user-defined class method.
    /// Walks the inheritance chain (`class_has_method`) so inherited methods
    /// are detected.
    pub(super) fn class_expr_has_method(&self, recv: &Expression, mname: &str) -> bool {
        self.get_expr_type_name(recv)
            .as_deref()
            .map(|tn| self.class_has_method(tn, mname))
            .unwrap_or(false)
    }

    /// Is class `derived` the same as, or a subclass of, `base`? Walks the
    /// `extends` chain; compares parameter-stripped names (IEEE 1800-2023 §8.25).
    pub(super) fn typedef_unroll(&self, name: &str) -> String {
        // Follow an `extends` target / derived class name through any typedef
        // indirection to a real class base (specialization-stripped). e.g.
        // `class seqA extends simple_seq;` where
        // `typedef uvm_sequence #(simple_item) simple_seq;` — the heritage
        // walk must continue from `uvm_sequence` (whose `extends` is
        // `uvm_sequence_base`). Without this, a sequence declared through a
        // typedef base was never recognized as a uvm_sequence_base and every
        // sequence-library add failed `$cast` ([BAD_SEQ_TYPE]).
        use crate::ast::types::DataType;
        let mut cur = name;
        let mut guard = 0;
        while guard < 16 {
            guard += 1;
            let stripped = cur.split('#').next().unwrap_or(cur);
            if self.module.classes.contains_key(stripped) {
                return stripped.to_string();
            }
            match self.module.typedef_types.get(cur) {
                Some(DataType::TypeReference { name, .. }) => {
                    let base = name.name.name.split('#').next().unwrap_or(&name.name.name);
                    if self.module.classes.contains_key(base) {
                        return base.to_string();
                    }
                    cur = base;
                }
                _ => return stripped.to_string(),
            }
        }
        cur.to_string()
    }

    pub(super) fn class_is_a(&self, derived: &str, base: &str) -> bool {
        // Class names are keyed bare (the parser keeps only the leaf segment
        // of a scoped name), so the comparison strips just the `#(params)`
        // specialization suffix, not a package prefix.
        let strip = |s: &str| s.split('#').next().unwrap_or(s).to_string();
        let base = strip(base);
        let mut cur = Some(self.typedef_unroll(derived));
        let mut guard = 0;
        while let Some(c) = cur {
            guard += 1;
            if guard > 128 {
                break;
            }
            if strip(&c) == base || self.class_origin(&c) == base {
                return true;
            }
            let nxt = self
                .module
                .classes
                .get(&c)
                .or_else(|| self.module.classes.get(&strip(&c)))
                .and_then(|cd| cd.extends.clone())
                .map(|e| self.typedef_unroll(&e));
            cur = nxt;
        }
        false
    }

    /// `$cast(dest, src)` dynamic type check: succeeds if `src`'s object class
    /// is assignment-compatible with `dest`'s declared class type. Permissive
    /// when types are unknown (null src, non-class handle, untracked dest type)
    /// so only a definite type mismatch fails (IEEE 1800-2023 §8.16).
    pub(super) fn cast_type_ok(&self, dest: &Expression, src: &Value) -> bool {
        self.cast_type_ok_ex(dest, src, None)
    }

    /// class-perf P2: `ovr` is the dest local's compile-time DECLARED class
    /// (threaded through CastInsn for nested-block handle locals the
    /// runtime overlay never typed). It replaces the overlay lookup in the
    /// Ident arm; all downstream checks (type-param resolution, value-param
    /// specialization, hierarchy) are unchanged.
    pub(super) fn cast_type_ok_ex(
        &self,
        dest: &Expression,
        src: &Value,
        ovr: Option<&str>,
    ) -> bool {
        // §6.24.1: casting to an ENUM succeeds only when the value is one of
        // its members. Checked before the class logic, whose "not a live class
        // object" escape hatch would otherwise wave every integer through.
        if let ExprKind::Ident(hh) = &dest.kind {
            if hh.path.len() == 1 {
                // Resolve the destination variable's ENUM type name through
                // every available registry (module signal/local, recorded
                // method-local decl, or a property of the current `this`
                // object) and validate the source against that enum's members.
                if let Some(tn) = self.enum_typename_of_var(&hh.path[0].name.name) {
                    if let Some(members) = self.module.enum_members.get(&tn) {
                        // An x or z bit matches no member.
                        if src.has_xz() {
                            return false;
                        }
                        return match src.to_u64() {
                            Some(x) => members.iter().any(|(_, mv)| *mv == x),
                            None => false,
                        };
                    }
                }
            }
        }
        let h = src.to_u64().unwrap_or(0) as usize;
        if h == 0 {
            return true; // null assigns through
        }
        let src_class = match self.heap.get(h).and_then(|o| o.as_ref()) {
            Some(i) => i.class_name.clone(),
            None => return true, // not a live class object — don't enforce
        };
        // A module-scope class handle is not in `var_class_types` (only
        // procedural locals are), so it used to fall into the permissive
        // `None` branch and EVERY downcast reported success.
        let dest_type = match &dest.kind {
            ExprKind::Ident(hh) if hh.path.len() == 1 => match ovr {
                Some(c) => Some(c.to_string()),
                None => self.cast_dest_declared_type(dest),
            },
            _ => self.cast_dest_declared_type(dest),
        };
        match dest_type {
            Some(dt) => {
                // The dest may be a TYPE PARAMETER (e.g. `REQ param_t;` inside
                // uvm_sequencer_param_base::send_request's `$cast(param_t, t)`).
                // "REQ" is not a real class, so class_is_a would spuriously fail
                // (SQRSNDREQCAST). Resolve it to the concrete class via the
                // current instance's bindings; if it can't be resolved to a known
                // class, don't enforce (stay permissive like the None case).
                //
                // When the resolved type is ALREADY a value-parameterized
                // specialization form (e.g. `special_comp#(1)` — produced by
                // class_of_var resolving a type param bound to a
                // value-parameterized class), it is NOT a type-param name and
                // resolve_type_param_binding returns None. Detect the `#` and
                // pass it through directly; strip_class_specialization below
                // extracts the base class for the hierarchy check and retains
                // the args for the value-param comparison. Without this, the
                // `None` fallback made `$cast` permissive and accepted an
                // object of `special_comp#(2)` as a `special_comp#(1)`.
                let raw = if self.module.classes.contains_key(&dt) || dt.contains('#') {
                    dt.clone()
                } else {
                    match self.resolve_type_param_binding(&dt) {
                        Some(c) => c,
                        None => return true,
                    }
                };
                // The resolved type may be a VALUE-PARAMETERIZED
                // specialization like `special_comp#(2)` (a type param bound
                // to a parameterized class). Strip the `#(args)` to get the
                // base class for the hierarchy check, and retain the args
                // text for the value-param comparison below. Without this,
                // `special_comp#(2)` is not in `module.classes`, so the dest
                // type is unknown and `$cast` falls back to permissive —
                // leaking a `special_comp#(2)` typewide callback onto a
                // `special_comp#(1)` instance (a UVM callbacks specialization case).
                let (base, spec_args_from_name) = Self::strip_class_specialization(&raw);
                // If the dest's base class is genuinely unknown, stay
                // permissive like the None case.
                if !self.module.classes.contains_key(&base) {
                    return true;
                }
                let resolved = base;
                // Hierarchy check: the src instance must be `resolved` or a
                // subclass of it, else the downcast must fail.
                let is_a = self.class_is_a(&src_class, &resolved);
                if !is_a {
                    return false;
                }
                // §8.25: specializations of a class whose base is a type
                // parameter are distinct class entries; the source must
                // derive from the destination's own specialization.
                if self.is_dep_class(&resolved)
                    && !self.cast_dep_spec_ok(
                        &src_class,
                        &resolved,
                        spec_args_from_name.as_deref(),
                        dest,
                    )
                {
                    return false;
                }
                // Value-parameter specialization check: if the dest class
                // has value parameters, src and dest must agree on every
                // one (e.g. $cast(me[special_comp#(2)], a1[#1]) must fail).
                // Dest value bindings come from the resolved name's `#(...)`
                // (type-param-dest case) or the dest var's declared
                // `#(...)` type_args (direct-decl case).
                let vp_ok =
                    self.cast_value_params_ok(&resolved, spec_args_from_name.as_deref(), dest, h);
                let tp_ok =
                    self.cast_type_params_ok(&resolved, spec_args_from_name.as_deref(), dest, h);
                vp_ok && tp_ok
            }
            None => {
                // §8.25: a destination declared through a typedef of a
                // specialization of a class whose base is a type parameter
                // (`typedef wrap_c #(derived_c) wd_t; wd_t t;`) takes only
                // that specialization; any other unknown type stays
                // permissive.
                self.dep_typedefs.is_empty()
                    || self.cast_dep_typedef_ok(&src_class, dest).unwrap_or(true)
            }
        }
    }
    /// §8.25: `src_class` derives from the destination's own specialization
    /// of `dep_base` class `resolved` (each is a distinct class entry).
    #[cold]
    #[inline(never)]
    pub(super) fn cast_dep_spec_ok(
        &self,
        src_class: &str,
        resolved: &str,
        from_name: Option<&str>,
        dest: &Expression,
    ) -> bool {
        let want = self.spec_class_name(resolved, &self.cast_dest_spec_args(from_name, dest));
        self.class_chain_has_entry(src_class, &want)
    }

    /// `$cast` to a destination declared through a typedef of a `dep_base`
    /// specialization; None when the destination is not one.
    #[cold]
    #[inline(never)]
    pub(super) fn cast_dep_typedef_ok(&self, src_class: &str, dest: &Expression) -> Option<bool> {
        let ExprKind::Ident(hh) = &dest.kind else {
            return None;
        };
        if hh.path.len() != 1 {
            return None;
        }
        let v = &hh.path[0].name.name;
        let t = self.var_class_types.get(v).cloned().or_else(|| {
            self.signal_name_to_id
                .get(v)
                .and_then(|id| self.signal_type_names.get(id))
                .cloned()
        })?;
        if !self.dep_typedefs.contains(&t) {
            return None;
        }
        let (b, sig) = self.resolve_typedef_spec(&t)?;
        if !self.dep_base.contains_key(&b) {
            return None;
        }
        let args: Vec<String> = Self::split_spec_args(&sig)
            .into_iter()
            .map(|a| a.trim().to_string())
            .collect();
        let want = self.spec_class_name(&b, &args);
        Some(self.class_chain_has_entry(src_class, &want))
    }

    /// The `#(...)` arguments a `$cast` destination's type carries: the
    /// resolved type name's, else the variable's declared ones (none for an
    /// unspecialized declaration, which is the default specialization).
    pub(super) fn cast_dest_spec_args(
        &self,
        from_name: Option<&str>,
        dest: &Expression,
    ) -> Vec<String> {
        let text = match from_name {
            Some(a) => Some(a.to_string()),
            None => match &dest.kind {
                ExprKind::Ident(h) if h.path.len() == 1 => {
                    let dvar = &h.path[0].name.name;
                    self.var_type_args
                        .get(dvar)
                        .or_else(|| self.module.class_type_args.get(dvar))
                        .and_then(|ta| {
                            let frags: Vec<String> = ta
                                .iter()
                                .filter_map(|e| self.expr_to_spec_fragment(e))
                                .collect();
                            (frags.len() == ta.len()).then(|| frags.join(","))
                        })
                }
                _ => None,
            },
        };
        text.map(|t| {
            Self::split_spec_args(&t)
                .into_iter()
                .map(|a| a.trim().to_string())
                .collect()
        })
        .unwrap_or_default()
    }

    /// Whether class entry `entry` is `start` or one of its ancestors, by
    /// exact entry name.
    pub(super) fn class_chain_has_entry(&self, start: &str, entry: &str) -> bool {
        let mut cur: Option<&str> = Some(start);
        let mut guard = 0;
        while let Some(c) = cur {
            guard += 1;
            if guard > 128 {
                break;
            }
            if c == entry {
                return true;
            }
            cur = self
                .module
                .classes
                .get(c)
                .and_then(|cd| cd.extends.as_deref());
        }
        false
    }

    /// Parse a value-parameter spec fragment (decimal or based literal)
    /// to a `u64`, for the `$cast` value-param comparison.
    pub(super) fn spec_fragment_to_u64(t: &str) -> Option<u64> {
        let n = Self::parse_spec_number(t)?;
        match n {
            crate::ast::expr::NumberLiteral::Integer { base, value, .. } => {
                let v = value.replace('_', "");
                let negative = v.starts_with('-');
                let digits = v.trim_start_matches('-').trim_start_matches('+');
                let parsed = match base {
                    crate::ast::expr::NumberBase::Decimal => {
                        u64::from_str_radix(digits, 10).ok()?
                    }
                    crate::ast::expr::NumberBase::Hex => u64::from_str_radix(digits, 16).ok()?,
                    crate::ast::expr::NumberBase::Octal => u64::from_str_radix(digits, 8).ok()?,
                    crate::ast::expr::NumberBase::Binary => u64::from_str_radix(digits, 2).ok()?,
                };
                Some(if negative { 0 } else { parsed })
            }
            _ => None,
        }
    }

    /// Split a possibly-specialized type name `"Class#(args)"` into
    /// `("Class", Some("args"))`. A bare `"Class"` yields
    /// `("Class", None)`. Used by `cast_type_ok` to recognize a type param
    /// bound to a value-parameterized specialization.
    pub(super) fn strip_class_specialization(s: &str) -> (String, Option<String>) {
        if let Some(idx) = s.find("#(") {
            let base = s[..idx].to_string();
            let rest = &s[idx + 2..];
            let args = rest.strip_suffix(')').unwrap_or(rest);
            (base, Some(args.to_string()))
        } else if let Some(idx) = s.find('#') {
            (s[..idx].to_string(), None)
        } else {
            (s.to_string(), None)
        }
    }

    /// Value-parameter specialization check for `$cast`. If `dest_class` has
    /// value parameters, the src instance (heap handle `src_handle`) and the
    /// dest's expected specialization must agree on every value param, else
    /// the cast fails (e.g. `$cast(me[special_comp#(2)], a1[#1])`). Returns
    /// `true` (permissive) when the comparison can't be determined — no
    /// value params, missing instance data, or no dest specialization text.
    pub(super) fn cast_value_params_ok(
        &self,
        dest_class: &str,
        spec_args_from_name: Option<&str>,
        dest: &Expression,
        src_handle: usize,
    ) -> bool {
        let cd = match self.module.classes.get(dest_class) {
            Some(c) => c.clone(),
            None => return true,
        };
        // Value-param names in declaration order (param_order minus type
        // params). param_order is serde-defaulted; reconstruct the legacy
        // value-first order for cached modules.
        let legacy_order: Vec<String>;
        let order: &[String] = if cd.param_order.is_empty()
            && !(cd.param_defaults.is_empty() && cd.type_param_names.is_empty())
        {
            legacy_order = cd
                .param_defaults
                .iter()
                .map(|(n, _)| n.clone())
                .chain(cd.type_param_names.iter().cloned())
                .collect();
            &legacy_order
        } else {
            &cd.param_order
        };
        let is_type_param = |n: &str| cd.type_param_names.iter().any(|t| t == n);
        let value_positions: Vec<(usize, String)> = order
            .iter()
            .enumerate()
            .filter(|(_, n)| !is_type_param(n))
            .map(|(i, n)| (i, n.clone()))
            .collect();
        if value_positions.is_empty() {
            return true;
        }
        // Dest specialization args: prefer the resolved name's `#(...)`
        // (type-param-dest case), else the dest var's declared type_args
        // (direct-decl case).
        let dest_args_text = match spec_args_from_name {
            Some(a) => Some(a.to_string()),
            None => {
                if let ExprKind::Ident(h) = &dest.kind {
                    if h.path.len() == 1 {
                        let dvar = &h.path[0].name.name;
                        self.var_type_args.get(dvar).and_then(|ta| {
                            let frags: Vec<String> = ta
                                .iter()
                                .filter_map(|e| self.expr_to_spec_fragment(e))
                                .collect();
                            if frags.is_empty() {
                                None
                            } else {
                                Some(frags.join(","))
                            }
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        };
        let dest_args: Vec<String> = match dest_args_text {
            Some(t) => Self::split_spec_args(&t)
                .into_iter()
                .map(|s| s.trim().to_string())
                .collect(),
            None => return true, // can't determine dest specialization → permissive
        };
        let src_inst = match self.heap.get(src_handle).and_then(|o| o.as_ref()) {
            Some(i) => i,
            None => return true,
        };
        for (pos, vp) in &value_positions {
            let dfrag = match dest_args.get(*pos) {
                Some(s) => s.as_str(),
                None => continue,
            };
            let sval = match src_inst.properties.get(vp) {
                Some(v) => v,
                None => continue,
            };
            // Compare the dest fragment (a literal) against the src value.
            let dfrag_t = dfrag.trim();
            if dfrag_t.is_empty() {
                continue;
            }
            let matches =
                if let Some(s) = dfrag_t.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                    sval.to_string() == *s
                } else if let Some(dn) = Self::spec_fragment_to_u64(dfrag_t) {
                    sval.to_u64() == Some(dn)
                } else {
                    true // non-literal fragment (e.g. a param name) → can't compare
                };
            if !matches {
                return false;
            }
        }
        true
    }

    /// Type-parameter specialization check for `$cast` (LRM 1800-2023 §8.25.1).
    /// Two distinct instantiations of a parameterized class are the SAME type
    /// iff their parameters are the same; distinct type arguments make the two
    /// instantiations DIFFERENT, unrelated types, so a `$cast` between them
    /// must FAIL even when the type arguments are subclasses/siblings of a
    /// common base or are unrelated builtin types.
    ///
    /// `uvm_config_wrapper::set` stores a `uvm_resource#(uvm_object_wrapper)`
    /// default sequence; the sequencer's `start_phase_sequence` does
    /// `$cast(sbr, rsrc)` with `sbr : uvm_resource#(uvm_sequence_base)`. Those
    /// are distinct instantiations (type params `uvm_object_wrapper` vs
    /// `uvm_sequence_base`), so the cast must FAIL and fall through to the
    /// `uvm_object_wrapper` branch that creates + starts the sequence. Before
    /// this check the cast only compared the base-class hierarchy (both are
    /// `uvm_resource`) and the value params (none), so it wrongly returned
    /// success, the sequence was never started, and its body messages never
    /// ran (message count 0).
    ///
    /// Mirrors `cast_value_params_ok`: for each TYPE param position the dest
    /// declares, the src instance's `type_bindings` must carry the same
    /// concrete type, else the cast fails. Permissive (`true`) when the
    /// comparison can't be determined (no type params, no dest args, or no src
    /// binding) — preserving the behaviour for non-specialized declarations.
    pub(super) fn cast_type_params_ok(
        &self,
        dest_class: &str,
        spec_args_from_name: Option<&str>,
        dest: &Expression,
        src_handle: usize,
    ) -> bool {
        let cd = match self.module.classes.get(dest_class) {
            Some(c) => c.clone(),
            None => return true,
        };
        if cd.type_param_names.is_empty() {
            return true; // no type params to compare
        }
        // parameter order: type and value params interleave. Reuse the legacy
        // order reconstruction exactly as the value-param checker does.
        let legacy_order: Vec<String>;
        let order: &[String] = if cd.param_order.is_empty()
            && !(cd.param_defaults.is_empty() && cd.type_param_names.is_empty())
        {
            legacy_order = cd
                .param_defaults
                .iter()
                .map(|(n, _)| n.clone())
                .chain(cd.type_param_names.iter().cloned())
                .collect();
            &legacy_order
        } else {
            &cd.param_order
        };
        let is_type_param = |n: &str| cd.type_param_names.iter().any(|t| t == n);
        let type_positions: Vec<(usize, String)> = order
            .iter()
            .enumerate()
            .filter(|(_, n)| is_type_param(n))
            .map(|(i, n)| (i, n.clone()))
            .collect();

        // Dest type-argument text: prefer the resolved name's `#(...)`
        // (type-param-dest case), else the dest var's declared type_args.
        let dest_args_text = match spec_args_from_name {
            Some(a) => Some(a.to_string()),
            None => {
                if let ExprKind::Ident(h) = &dest.kind {
                    if h.path.len() == 1 {
                        let dvar = &h.path[0].name.name;
                        self.var_type_args.get(dvar).and_then(|ta| {
                            let frags: Vec<String> = ta
                                .iter()
                                .filter_map(|e| self.expr_to_spec_fragment(e))
                                .collect();
                            if frags.is_empty() {
                                None
                            } else {
                                Some(frags.join(","))
                            }
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        };
        let dest_args: Vec<String> = match dest_args_text {
            Some(t) => Self::split_spec_args(&t)
                .into_iter()
                .map(|s| s.trim().to_string())
                .collect(),
            None => return true, // can't determine dest type args → permissive
        };
        let src_inst = match self.heap.get(src_handle).and_then(|o| o.as_ref()) {
            Some(i) => i,
            None => return true,
        };
        // Strip a `#(…)`/value-spec suffix and scope prefix for a canonical
        // type-argument comparison. Keep the argument's own type args intact
        // (they are part of the type's identity).
        fn canon(s: &str) -> String {
            s.trim()
                .split("::")
                .last()
                .unwrap_or_else(|| s.trim())
                .trim_matches('`')
                .to_string()
        }
        // A source class without parameters of its own binds the
        // destination's type parameters through its extends chain
        // (`class leaf extends pbase#(7, shortint)`), not on the object.
        let chain_args: Vec<String> = if self.class_is_parameterized(&src_inst.class_name) {
            Vec::new()
        } else {
            self.static_receiver_spec(&src_inst.class_name, dest_class)
                .map(|(_, sig)| {
                    Self::split_spec_args(&self.canonicalize_spec_sig(dest_class, &sig))
                        .into_iter()
                        .map(|a| a.trim().to_string())
                        .collect()
                })
                .unwrap_or_default()
        };
        // The base-class type parameter of a class whose base is a type
        // parameter was checked by entry (`cast_dep_spec_ok`): the source may
        // reach the destination's specialization at any level of its chain
        // (`wrap_c#(wrap_c#(d))` is a `wrap_c#(d)`), while its binding of the
        // name is the outermost one's.
        let dep_tp = self.dep_type_param(dest_class);
        for (pos, tp) in &type_positions {
            if dep_tp.as_deref() == Some(tp.as_str()) {
                continue;
            }
            let dfrag = match dest_args.get(*pos) {
                Some(s) => s.trim(),
                None => continue,
            };
            if dfrag.is_empty() {
                continue;
            }
            let sval = match src_inst
                .type_bindings
                .get(tp)
                .or_else(|| chain_args.get(*pos).filter(|a| !a.is_empty()))
            {
                Some(v) => v,
                None => continue,
            };
            // A dest arg that is a bare TYPE-PARAM NAME of `dest_class` (e.g.
            // `this_type = pair#(T1,T2)` inside `pair` — the typedef was
            // recorded textually with the parameter names, not the concrete
            // specialization) is a PLACEHOLDER, not the actual type. Resolve
            // it to the concrete type bound in the current context (a method
            // of `pair#(txn,txn)` binds `T1`->`txn`); comparing the bare
            // name `T1` against the src instance's concrete `txn` would
            // wrongly fail every `$cast` to one's own `this_type`
            // (§8.25 two specializations === same iff their params are same).
            let dfrag_resolved = if cd.type_param_names.iter().any(|n| n == dfrag) {
                self.resolve_type_param_binding(dfrag)
                    .unwrap_or_else(|| dfrag.to_string())
            } else {
                dfrag.to_string()
            };
            let dfrag_resolved = dfrag_resolved.trim().to_string();
            // A typedef spelling and the class it stands for are the same
            // type (§6.18): the dest fragment may come from the
            // specialization signature's canonical spelling while the
            // instance binding records the source spelling (`gp_t` vs
            // `uvm_tlm_generic_payload`), or vice versa. Follow typedefs
            // on both sides before comparing.
            let dfrag_resolved = self
                .typedef_alias_frag(&dfrag_resolved)
                .unwrap_or(dfrag_resolved);
            let sval_resolved = self
                .typedef_alias_frag(sval)
                .unwrap_or_else(|| sval.to_string());
            if canon(&dfrag_resolved) != canon(&sval_resolved) {
                return false;
            }
        }
        true
    }

    /// The enclosing class while a static method runs (a null `this` on top
    /// of the method-context stack), for resolving bare method names.
    pub(super) fn static_class_context(&self) -> Option<String> {
        match self.this_stack.last() {
            Some(None) => self.class_context_stack.last().cloned().flatten(),
            _ => None,
        }
    }

    /// Find a class TASK method named `method` in `start_class` or an ancestor.
    /// Returns its declaration + the defining class name. Used by the process
    /// scheduler to INLINE a blocking method call so its internal waits
    /// (`forever @clk`, `#delay`, `wait`, sequence handshakes) suspend the
    /// process instead of spinning. Returns None for functions (they cannot
    /// consume time, so they keep the synchronous path) or unknown methods.
    /// The task `method` as declared on `start_class` or its nearest
    /// ancestor, with the declaring class. The method is `Task`-kind.
    pub(super) fn resolve_class_task(
        &self,
        start_class: &str,
        method: &str,
    ) -> Option<(Arc<crate::ast::decl::ClassMethod>, String)> {
        use crate::ast::decl::ClassMethodKind;
        let mut cur: Option<&str> = Some(start_class);
        while let Some(cn) = cur {
            let cd = self.module.classes.get(cn)?;
            if let Some(m) = cd.methods.get(method) {
                if let ClassMethodKind::Task(_) = &m.kind {
                    let m = self.cached_class_method(cn, method)?;
                    return Some((m, cn.to_string()));
                }
                return None;
            }
            cur = cd.extends.as_deref();
        }
        None
    }

    /// Is `name` a static method of `class_name` or any ancestor?
    pub(super) fn is_static_method(&self, class_name: &str, name: &str) -> bool {
        if let Some(cached) = self
            .is_static_method_memo
            .borrow()
            .get(class_name)
            .and_then(|methods| methods.get(name))
        {
            return *cached;
        }
        // Whole-chain static-name set (frozen): a miss answers definitively.
        if !self.class_member_names().static_methods.contains(name) {
            self.is_static_method_memo
                .borrow_mut()
                .entry(class_name.to_string())
                .or_default()
                .insert(name.to_string(), false);
            return false;
        }
        let mut cur: Option<&str> = Some(class_name);
        let mut found = false;
        while let Some(cname) = cur {
            if let Some(cd) = self.module.classes.get(cname) {
                if cd.static_methods.contains(name) {
                    found = true;
                    break;
                }
                cur = cd.extends.as_deref();
            } else {
                break;
            }
        }
        self.is_static_method_memo
            .borrow_mut()
            .entry(class_name.to_string())
            .or_default()
            .insert(name.to_string(), found);
        found
    }

    /// Resolve the DECLARED (static) class of a receiver expression used for
    /// a static method call. Handles a plain variable (`req), a bare `this`,
    /// and a member chain rooted at a known object (`this.req`) — where the
    /// member's declared type comes from the owning class, not any runtime
    /// value (which may be null). This lets `uvm_create`'s `req.get_type()`
    /// resolve the `trans` wrapper even when the handle is null.
    pub(super) fn receiver_static_class(
        &self,
        expr: &Expression,
        runtime_handle: usize,
    ) -> Option<String> {
        use crate::ast::expr::ExprKind;
        // A live receiver: prefer its runtime class (covers derived objects).
        if runtime_handle != 0 {
            if let Some(cn) = self
                .heap
                .get(runtime_handle)
                .and_then(|o| o.as_ref())
                .map(|i| i.class_name.clone())
            {
                return Some(cn);
            }
        }
        match &expr.kind {
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                let name = &h.path[0].name.name;
                if name == "this" || name == "super" {
                    return None; // handled via builtin-dispatch elsewhere
                }
                let c = self.class_of_var(name);
                let th = self.this_stack.last().copied().flatten();
                let pt = th.and_then(|hh| self.prop_class_type(hh, name));
                // For a bare member of the current `this` object (`seq.get_type()`
                // where `seq` is a declared field of the running sequence), the
                // member's declared type from the object's class chain is
                // authoritative. `class_of_var` is only a loose heuristic for
                // locals/module vars and can drift to a base type (e.g. resolves
                // the `seq` field to `uvm_sequence_item`) — prefer the property
                // lookup when it resolves, else fall back to the var class.
                pt.or_else(|| c)
            }
            // `this.req.get_type()` (receiver = `this.req`) — resolve the
            // declared member type through the current `this` class.
            ExprKind::MemberAccess { expr: base, member } => match &base.kind {
                ExprKind::Ident(b) if b.path.len() == 1 && b.path[0].selects.is_empty() => {
                    let bname = &b.path[0].name.name;
                    if bname == "this" || bname == "super" {
                        self.this_stack
                            .last()
                            .copied()
                            .flatten()
                            .and_then(|th| self.prop_class_type(th, &member.name))
                    } else {
                        // `s.req.getv()`: receiver `expr` = `s.req`. Resolve
                        // the base object `s` to its runtime handle and read
                        // the member's declared type off its class (with
                        // type-param binding), exactly like `this.req`.
                        let lv = self.local_stack.last().and_then(|m| m.get(bname));
                        let sh = lv
                            .and_then(|v| v.to_u64())
                            .map(|h| h as usize)
                            .or_else(|| self.eval_ident_handle(bname))
                            .filter(|&h| h != 0 && h < self.heap.len())
                            .unwrap_or(0);
                        if sh != 0 {
                            self.prop_class_type(sh, &member.name)
                        } else {
                            self.class_of_var(bname).and_then(|bc| {
                                self.prop_class_type_from_class(&bc, &member.name, None)
                            })
                        }
                    }
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// §8.25: the `(base, sig)` specialization a variable's DECLARED type
    /// names — a direct `C#(args)` (with named arguments normalized to
    /// positional order), or a typedef for a specialization. None for a
    /// plain class or an unresolvable type. Used to seed `current_spec`
    /// when a static method is dispatched through a null receiver whose
    /// declared type is specialized, so the method body's bare
    /// self-references denote the current specialization.
    pub(super) fn spec_from_declared_type(&self, decl: &str) -> Option<(String, String)> {
        let cand = self
            .positional_spec_text(decl)
            .unwrap_or_else(|| decl.to_string());
        if let Some((b, s)) = self.extract_spec_from_string(&cand) {
            return Some((b, s));
        }
        let (b, s) = self.resolve_typedef_spec(decl)?;
        let full = format!("{}#({})", b, s);
        let norm = self.positional_spec_text(&full).unwrap_or(full);
        self.extract_spec_from_string(&norm)
    }

    /// §8.25: the specialization a NULL receiver's declared type names, for
    /// seeding `current_spec` around a static-method dispatch. A procedural
    /// local's `var_class_types` entry holds the BARE class name (its `#(...)`
    /// arguments live in `var_type_args`), so for a variable receiver
    /// `declared_class_own_spec` — which spells the declaration's own
    /// specialization — is authoritative; the declared-type TEXT fallback
    /// covers `this.member` property types that carry `base#(sig)` spellings.
    pub(super) fn spec_for_null_receiver(
        &self,
        var: Option<&str>,
        decl: &str,
    ) -> Option<(String, String)> {
        if let Some(v) = var {
            if let Some((_, Some(spec))) = self.declared_class_own_spec(v, None) {
                return Some(spec);
            }
        }
        self.spec_from_declared_type(decl)
    }

    /// `Name.method(args)` where `Name` names a CLASS-scope entity — the
    /// shared resolution for the AST member-call path and the compiled
    /// `Insn::CallStaticScoped` executor (class-perf Step 9h). Resolution
    /// order (mirrors the original inline block): an active type-parameter
    /// binding (possibly a `base#sig` specialization), an embedded
    /// specialization in the name itself, a plain class name, a
    /// parameterized typedef alias (§8.25.1), then a simple typedef alias
    /// (§6.18). `Name::new(...)` constructs explicitly. Returns None when
    /// nothing resolves (callers keep their fallback order).
    pub(super) fn exec_class_scope_static_call(
        &mut self,
        name: &str,
        mname: &str,
        args: &[Expression],
    ) -> Option<Value> {
        // Type-parameter used as a class name: `Tregistry::get()`.

        // Resolve the type param to a concrete class/specialization
        // from the active spec before dispatching.
        if let Some(resolved) = self.resolve_type_param_binding(name) {
            if let Some((base, sig)) = self.extract_spec_from_string(&resolved) {
                self.ensure_spec_statics(&base, &sig);
                let saved = self.current_spec.take();
                if let Some(cs) = &saved {
                    self.spec_scope_stack.push(cs.clone());
                }
                self.current_spec = Some((base.clone(), sig));
                let res = self.exec_static_method(&base, mname, args);
                let had_saved = saved.is_some();
                self.current_spec = saved;
                if had_saved {
                    self.spec_scope_stack.pop();
                }
                if let Some(v) = res {
                    return Some(v);
                }
            } else if self.module.classes.contains_key(&resolved) {
                if let Some(res) = self.exec_static_method(&resolved, mname, args) {
                    return Some(res);
                }
            }
        }
        if let Some((base, sig)) = self.extract_spec_from_string(name) {
            self.ensure_spec_statics(&base, &sig);
            let saved = self.current_spec.take();
            if let Some(cs) = &saved {
                self.spec_scope_stack.push(cs.clone());
            }
            self.current_spec = Some((base.clone(), sig));
            let res = self.exec_static_method(&base, mname, args);
            let had_saved = saved.is_some();
            self.current_spec = saved;
            if had_saved {
                self.spec_scope_stack.pop();
            }
            if let Some(v) = res {
                return Some(v);
            }
        } else if self.module.classes.contains_key(name) {
            if let Some(res) = self.exec_static_method(name, mname, args) {
                return Some(res);
            }
            // `ClassName::new(...)` — explicit constructor call.
            if mname == "new" {
                if let Some(cd) = self.module.classes.get(name).cloned() {
                    return Some(self.instantiate_class(&cd, args));
                }
            }
        }
        // §8.25.1 typedef specialization alias on the dot-access
        // form (see the flattened-path handler for details).
        if let Some((base, sig)) = self.resolve_typedef_spec(name) {
            self.ensure_spec_statics(&base, &sig);
            let saved = self.current_spec.take();
            if let Some(cs) = &saved {
                self.spec_scope_stack.push(cs.clone());
            }
            self.current_spec = Some((base.clone(), sig));
            let res = self.exec_static_method(&base, mname, args);
            let had_saved = saved.is_some();
            self.current_spec = saved;
            if had_saved {
                self.spec_scope_stack.pop();
            }
            if let Some(v) = res {
                return Some(v);
            }
        }
        // §6.18/§8.25.1: simple class typedef (no specialization
        // args) — `typedef base alias;` ... `alias::method()`.
        // `resolve_typedef_spec` skips these; resolve the alias
        // to its target class and dispatch.
        if let Some(resolved) = self.resolve_simple_typedef_class(name) {
            if let Some(res) = self.exec_static_method(&resolved, mname, args) {
                return Some(res);
            }
            if mname == "new" {
                if let Some(cd) = self.module.classes.get(&resolved).cloned() {
                    return Some(self.instantiate_class(&cd, args));
                }
            }
        }
        None
    }

    /// §8.23/§8.25: the class a class-scope prefix `name` (the `name` of
    /// `name::member`) denotes, with its specialization: a class TYPE
    /// PARAMETER bound through the active specialization or `this`, a
    /// `C#(..)` text, a parameterized typedef at module or class scope
    /// (§8.25.1), a plain class typedef (§6.18), or a class. The same
    /// resolution order `exec_class_scope_static_call` dispatches a call
    /// with. None when `name` names no class.
    pub(super) fn resolve_class_scope_prefix(
        &self,
        name: &str,
    ) -> Option<(String, Option<String>)> {
        let bound = self.resolve_type_param_binding(name);
        let n = bound.as_deref().unwrap_or(name);
        if let Some((b, s)) = self.extract_spec_from_string(n) {
            return Some((b, Some(s)));
        }
        if self.module.classes.contains_key(n) {
            return Some((n.to_string(), None));
        }
        if let Some((b, s)) = self.resolve_typedef_spec(n) {
            return Some((b, Some(s)));
        }
        self.resolve_simple_typedef_class(n)
            .filter(|c| self.module.classes.contains_key(c))
            .map(|c| (c, None))
    }

    /// §13.5.1/§8.25: the value of a parenthesis-free class-scope reference
    /// `name::member` whose prefix is not itself a class name (a type
    /// parameter, a typedef, a specialization; see
    /// `resolve_class_scope_prefix`). A no-argument static FUNCTION is a call
    /// with an empty argument list, so its value is the call's return; a
    /// value parameter or class localparam reads the specialization's
    /// value; a static property reads the specialization's cell. Everything
    /// runs with the prefix's specialization active, exactly as the
    /// parenthesised call does. None when `member` is none of these.
    pub(super) fn class_scope_root(mut expr: &Expression) -> Option<&str> {
        loop {
            match &expr.kind {
                ExprKind::MemberAccess { expr: inner, .. }
                | ExprKind::Paren(inner)
                | ExprKind::Specialization { base: inner, .. } => expr = inner,
                ExprKind::Ident(h) => return h.path.first().map(|s| s.name.name.as_str()),
                _ => return None,
            }
        }
    }

    pub(super) fn class_scope_chain(expr: &Expression) -> Option<String> {
        match &expr.kind {
            ExprKind::Ident(h)
                if !h.path.is_empty() && h.path.iter().all(|s| s.selects.is_empty()) =>
            {
                Some(
                    h.path
                        .iter()
                        .map(|s| s.name.name.as_str())
                        .collect::<Vec<_>>()
                        .join("::"),
                )
            }
            ExprKind::MemberAccess { expr, member } => Some(format!(
                "{}::{}",
                Self::class_scope_chain(expr)?,
                member.name
            )),
            ExprKind::Specialization { base, .. } | ExprKind::Paren(base) => {
                Self::class_scope_chain(base)
            }
            _ => None,
        }
    }

    pub(super) fn class_scope_member_value(&mut self, name: &str, member: &str) -> Option<Value> {
        let (base, sig) = self.resolve_class_scope_prefix(name)?;
        let is_fn = self.is_static_method(&base, member)
            && self.class_parameterless_function(&base, member);
        let saved = sig.as_ref().map(|s| {
            self.ensure_spec_statics(&base, s);
            let saved = self.current_spec.replace((base.clone(), s.clone()));
            if let Some(cs) = &saved {
                self.spec_scope_stack.push(cs.clone());
            }
            saved
        });
        let v = if is_fn {
            self.exec_static_method(&base, member, &[])
        } else {
            sig.as_ref()
                .and_then(|_| self.resolve_value_param_in_current(member))
                .or_else(|| self.class_static_get(&base, member))
        };
        if let Some(saved) = saved {
            if saved.is_some() {
                self.spec_scope_stack.pop();
            }
            self.current_spec = saved;
        }
        v
    }

    /// §8.25: `C#(args)::P` / `C#(args)::LP` — a value parameter or class
    /// localparam of class `cls` read while `cls`'s own specialization is
    /// active (the stripped `Specialization` wrapper set it) takes that
    /// specialization's value, not the declared default the shared static
    /// cell holds. None when no specialization of `cls` is active or
    /// `member` is no parameter of it.
    pub(super) fn active_spec_param_value(&mut self, cls: &str, member: &str) -> Option<Value> {
        if !self.current_spec.as_ref().is_some_and(|(b, _)| b == cls) {
            return None;
        }
        let declares = self.module.classes.get(cls).is_some_and(|cd| {
            !cd.type_param_names.iter().any(|t| t == member)
                && (cd.param_order.iter().any(|p| p == member)
                    || cd.param_defaults.iter().any(|(n, _)| n == member))
        });
        if !declares {
            return None;
        }
        self.resolve_value_param_in_current(member)
    }

    /// Execute `ClassName::method(args)` — a static method call with no
    /// instance handle. Returns None if the method is not found.
    pub(super) fn exec_static_method(
        &mut self,
        class_name: &str,
        method_name: &str,
        args: &[Expression],
    ) -> Option<Value> {
        // Constructors (`new`) are never static — they always require an
        // instance to be allocated first. If we dispatch them through the
        // static path, the constructor body executes with `this`=0 (null),
        // writing `m_leaf_name`, `m_inst_id` etc. to the null handle so the
        // object is never properly initialised. All call-sites that need
        // constructor dispatch have an explicit `if m == "new" { instantiate }`
        // fallback, so returning None here lets them reach it.
        if method_name == "new" {
            return None;
        }
        if self.class_has_method(class_name, method_name)
            && !self.is_static_method(class_name, method_name)
        {
            // §8.23: inside a non-static method of `class_name` or of a
            // class derived from it, `Class::m()` names `Class`'s `m` on the
            // implicit `this`, bound non-virtually (like `super.m()`).
            let this_h = self.this_stack.last().copied().flatten().unwrap_or(0);
            let this_cls = self
                .heap
                .get(this_h)
                .and_then(|o| o.as_ref())
                .map(|i| i.class_name.clone());
            if let Some(rt) = this_cls
                && this_h != 0
                && self.class_is_a(&rt, class_name)
            {
                let mut cur = Some(class_name.to_string());
                while let Some(cname) = cur {
                    let Some((has, parent)) = self
                        .module
                        .classes
                        .get(&cname)
                        .map(|cd| (cd.methods.contains_key(method_name), cd.extends.clone()))
                    else {
                        break;
                    };
                    if has {
                        return Some(self.exec_method_in_class_hierarchy(
                            this_h,
                            &cname,
                            method_name,
                            args,
                        ));
                    }
                    cur = parent.map(|p| p.split('#').next().unwrap_or(&p).to_string());
                }
            }
            // §8.10: anywhere else a class-scope `::` access names a STATIC
            // member; a non-static method reached this way has no instance.
            // The reference simulator rejects it; report it once, fatally.
            if self
                .nonstatic_scope_reported
                .insert((class_name.to_string(), method_name.to_string()))
            {
                let msg = format!(
                    "Illegal to access non-static method '{}' outside its class scope: {}::{} requires an instance",
                    method_name, class_name, method_name
                );
                self.emit_severity_text("Fatal", &msg);
                self.fatal_finish_number = Some(1);
                self.finished = true;
            }
            return Some(Value::zero(32));
        }
        let mut cur = Some(class_name.to_string());
        while let Some(cname) = cur {
            let Some((has_method, parent)) = self
                .module
                .classes
                .get(&cname)
                .map(|cd| (cd.methods.contains_key(method_name), cd.extends.clone()))
            else {
                break;
            };
            if has_method {
                // §8.25: a static reached through a CONCRETE receiver but
                // declared in a PARAMETERIZED ancestor (`ext1::ID()` where
                // `ext1 extends base#(ext1)`) has no `#(spec)` on the call and
                // no instance; seed current_spec from the receiver's extends
                // chain so the body's BARE static member keys per-
                // specialization. Generic: applies to any concrete receiver of
                // a parameterized method (`derive_static_spec` is the shared
                // path; this is its method-dispatch home).
                if let Some(spec) = self.derive_static_spec(class_name, &cname) {
                    let saved = self.current_spec.clone();
                    self.current_spec = Some(spec);
                    let v = self.exec_method_in_class_hierarchy(0, &cname, method_name, args);
                    self.current_spec = saved;
                    return Some(v);
                }
                let v = self.exec_method_in_class_hierarchy(0, &cname, method_name, args);
                return Some(v);
            }
            cur = parent;
        }
        None
    }

    /// §8.25 + §8.10: a static method called through an object handle
    /// (`c.depth()`) belongs to the object's specialization: `c` of type
    /// `St#(2)` runs `St#(2)::depth`, whose bare parameter `N` reads 2, not
    /// the default. The object carries its specialization in `spec`; for a
    /// method inherited from a parameterized ancestor the ancestor's binding
    /// is derived from it. A null handle (or an object built without a
    /// specialization) keeps the plain class dispatch.
    #[inline(never)]
    pub(super) fn exec_static_method_via_object(
        &mut self,
        class_name: &str,
        method_name: &str,
        args: &[Expression],
        handle: usize,
    ) -> Option<Value> {
        let spec = (handle != 0)
            .then(|| self.heap.get(handle).and_then(|o| o.as_ref()))
            .flatten()
            .and_then(|inst| inst.spec.clone())
            .and_then(|(base, sig)| {
                // The class in the receiver's chain that declares the method.
                let mut cur = Some(class_name.to_string());
                let mut declaring = None;
                while let Some(cn) = cur {
                    let cd = self.module.classes.get(&cn)?;
                    if cd.methods.contains_key(method_name) {
                        declaring = Some(cn);
                        break;
                    }
                    cur = cd.extends.clone();
                }
                let declaring = declaring?;
                if self.class_origin(&declaring) == base {
                    Some((base, sig))
                } else {
                    self.inherited_method_spec(handle, &declaring)
                }
            });
        match spec {
            Some(spec) => {
                let saved = std::mem::replace(&mut self.current_spec, Some(spec));
                let v = self.exec_static_method(class_name, method_name, args);
                self.current_spec = saved;
                v
            }
            None => self.exec_static_method(class_name, method_name, args),
        }
    }

    /// Strip `Specialization` wrappers from the call/member shape so dispatch
    /// matching (which looks for Ident/MemberAccess chains) sees the same shape
    /// as before the `Specialization` AST node existed. The `#(...)` spec text
    /// is preserved in the original AST for per-spec static keying (handled at
    /// the static-dispatch sites); this only normalizes shape.
    pub(super) fn expr_has_specialization(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Specialization { .. } => true,
            ExprKind::MemberAccess { expr, .. } => Self::expr_has_specialization(expr),
            _ => false,
        }
    }

    /// Extract `(base_class, sig)` from a `C#(params)::...` expression's
    /// `Specialization` node (the spec sits on the member/ident chain's base).
    pub(super) fn extract_call_spec(e: &Expression) -> Option<(String, String)> {
        match &e.kind {
            ExprKind::Specialization {
                base,
                type_args_text,
            } => {
                // The specialized base may be a flat `Ident([pkg, Class])`
                // — or, when a package qualifier precedes the specialization,
                // a nested `MemberAccess(Ident([pkg]), Class)`. Both name the
                // same class; take the FINAL name either way — for the MA
                // chain that is the OUTERMOST member (the segment the
                // `#(...)` applies to), for the flat Ident its last segment
                // (`uvc::env#(virtual I)::create` must key on `env`).
                match &base.kind {
                    ExprKind::MemberAccess { member, .. } => {
                        Some((member.name.clone(), type_args_text.clone()))
                    }
                    ExprKind::Ident(h) => h
                        .path
                        .last()
                        .map(|s| (s.name.name.clone(), type_args_text.clone())),
                    _ => None,
                }
            }
            ExprKind::MemberAccess { expr, .. } => Self::extract_call_spec(expr),
            _ => None,
        }
    }

    pub(super) fn strip_spec_shape(e: &Expression) -> Expression {
        match &e.kind {
            ExprKind::Specialization { base, .. } => Self::strip_spec_shape(base),
            ExprKind::MemberAccess { expr, member } => Expression::new(
                ExprKind::MemberAccess {
                    expr: Box::new(Self::strip_spec_shape(expr)),
                    member: member.clone(),
                },
                e.span,
            ),
            _ => e.clone(),
        }
    }

    /// Flatten a `pkg::Class#(args)::m2::m1(...)` callee — written as a nested
    /// `MemberAccess(...MemberAccess(Ident(pkg) | Specialization of it, m2), m1)`
    /// because the mid-chain `#(args)` defeats the parser's flat-Ident
    /// flattening of `::` scope chains — into the equivalent flat
    /// `Ident([pkg, Class, m2, m1])`. Returns `None` unless every link is a
    /// bare select-free name and the flattened path has >= 3 segments — the
    /// shape only a `::` chain with a mid-chain specialization produces
    /// (plain dotted chains are guarded at the call site by the
    /// package/class check on the first segment).
    pub(super) fn flatten_scoped_member_chain(
        mut e: &Expression,
        last: &crate::ast::Identifier,
    ) -> Option<Expression> {
        // Fast reject without allocating: 99.9% of MemberAccess callees are
        // `this.m()`, `obj.m()` (1-segment Ident), or `call().m()`, which have
        // < 3 segments or a non-Ident root.
        match &e.kind {
            ExprKind::Ident(h) if h.path.len() < 2 => return None,
            ExprKind::MemberAccess { .. }
            | ExprKind::Specialization { .. }
            | ExprKind::Ident(_) => {}
            _ => return None,
        }
        let mut members: Vec<&crate::ast::Identifier> = vec![last];
        let span = e.span;
        loop {
            match &e.kind {
                ExprKind::MemberAccess { expr, member } => {
                    members.push(member);
                    e = expr;
                }
                ExprKind::Specialization { base, .. } => {
                    e = base;
                }
                ExprKind::Ident(h) => {
                    if h.root.is_some()
                        || h.path.is_empty()
                        || h.path.len() + members.len() < 3
                        || h.path.iter().any(|s| !s.selects.is_empty())
                    {
                        return None;
                    }
                    let mut path: Vec<HierPathSegment> =
                        Vec::with_capacity(h.path.len() + members.len());
                    path.extend(h.path.iter().cloned());
                    path.extend(members.into_iter().rev().map(|m| HierPathSegment {
                        name: m.clone(),
                        selects: Vec::new(),
                    }));
                    return Some(Expression::new(
                        ExprKind::Ident(HierarchicalIdentifier {
                            root: None,
                            path,
                            span,
                            cached_signal_id: std::cell::Cell::new(None),
                            cached_resolved_name: std::cell::OnceCell::new(),
                        }),
                        span,
                    ));
                }
                _ => return None,
            }
        }
    }

    /// Resolution of `C::type_id::create`.
    /// A class `C` registers itself with a class factory via a typedef
    /// `typedef <registry>#(T,"N") type_id;`. The net effect of
    /// `C::type_id::create(...)` is to construct the registered type `T`
    /// (the first type argument). Returns the leaf class name `T` when
    /// `class_name`'s `type_id` typedef is such a `*registry*` TypeReference,
    /// else `None`.
    pub(super) fn resolve_type_id_target_class(&self, class_name: &str) -> Option<String> {
        let cd = self.module.classes.get(class_name)?;
        let dt = cd.typedef_targets.get("type_id")?;
        if let DataType::TypeReference {
            name, type_args, ..
        } = dt
        {
            if name.name.name.contains("registry") {
                if let Some(first) = type_args.first() {
                    // The first type arg is the registered class. It can be:
                    //   1. A plain `Ident` (non-parameterized class): my_comp
                    //   2. A `Specialization` (parameterized class): C#(T)
                    //      — e.g. `resource_db_default_impl_t#(T)`
                    //        whose `type_id` typedef is
                    //        `object_registry#(resource_db_default_impl_t#(T), "...")`
                    // Extract the leaf class name from whichever form.
                    let leaf_opt = match &first.kind {
                        ExprKind::Ident(hier) => Some(hier.path.last().unwrap().name.name.clone()),
                        ExprKind::Specialization { base, .. } => {
                            if let ExprKind::Ident(hier) = &base.kind {
                                Some(hier.path.last().unwrap().name.name.clone())
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    if let Some(leaf) = leaf_opt {
                        // The registered type is often a class-local typedef
                        // alias, not the class itself — e.g. a class declaring
                        //   typedef Sequencer#(REQ,RSP) this_type;
                        // so type_id's first arg is `this_type`. Resolve one
                        // level through the class's own typedef table to the
                        // underlying class name (else `this_type` is not a
                        // class and the factory create returns null → ports
                        // are never built → null handle build error).
                        if let Some(DataType::TypeReference { name: real, .. }) =
                            cd.typedef_targets.get(&leaf)
                        {
                            return Some(real.name.name.clone());
                        }
                        return Some(leaf);
                    }
                }
            }
        }
        None
    }
}
