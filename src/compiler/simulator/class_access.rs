//! Simulator class access.

use super::*;

impl Simulator {
    /// `%m` prefix of class `cname`: its declaring package, enclosing class
    /// chain or declaring module instance, then the class name. A class of
    /// the compilation unit keeps the object's creation scope.
    pub(super) fn class_m_root(&self, cname: &str) -> String {
        let mut path = self.class_source_name(cname).to_string();
        let mut outer = cname.to_string();
        for _ in 0..16 {
            match self
                .module
                .classes
                .get(&outer)
                .and_then(|cd| cd.enclosing.clone())
            {
                Some(enc) => {
                    path = format!("{}.{}", enc, path);
                    outer = enc;
                }
                None => break,
            }
        }
        if let Some(pkg) = self.module.class_decl_pkg.get(&outer) {
            return format!("{}.{}", pkg, path);
        }
        let creation = self
            .this_stack
            .last()
            .copied()
            .flatten()
            .and_then(|h| self.heap.get(h).and_then(|o| o.as_ref()))
            .map(|i| self.instance_relative_scope(&i.creation_scope))
            .unwrap_or_default();
        let decl_module = self
            .module
            .classes
            .get(&outer)
            .and_then(|cd| cd.declaring_module.clone());
        let inst = match decl_module {
            Some(m) if m == self.module.name => String::new(),
            Some(m) => {
                let paths: Vec<&str> = self
                    .module
                    .instances
                    .iter()
                    .filter(|i| i.def_name == m)
                    .map(|i| i.path.as_str())
                    .collect();
                let within = paths.iter().find(|p| {
                    creation == **p
                        || creation
                            .strip_prefix(**p)
                            .is_some_and(|r| r.starts_with('.'))
                });
                match (within, paths.as_slice()) {
                    (Some(p), _) => p.to_string(),
                    (None, [only]) => only.to_string(),
                    _ => creation,
                }
            }
            None => {
                // Built outside every top (a package static): no instance
                // under the multi-top wrapper to name.
                if creation.is_empty() && self.root_is_multi_top() {
                    return path;
                }
                creation
            }
        };
        // §23.6: instance-qualified class keys already contain this owner.
        // `%m` adds the instance separately, so include its path only once.
        let class_path = path
            .strip_prefix(inst.as_str())
            .and_then(|rest| rest.strip_prefix('.'))
            .unwrap_or(&path);
        format!("{}.{}", self.hier_path(&inst), class_path)
    }

    /// The `m_scope_stack` a subroutine starts with: its leaf name, marked as
    /// a subroutine frame. A package subroutine or one declared in another
    /// instance or generate block (`u.mf`, `g.f`) also carries its absolute
    /// declaring path; a bare module subroutine is named from the caller's
    /// instance.
    pub(super) fn subroutine_m_entry(
        &self,
        full: &str,
        span: crate::ast::Span,
        pkg: Option<&str>,
    ) -> Vec<String> {
        let (prefix, leaf) = match full.rsplit_once('.') {
            Some((p, l)) => (Some(p), l),
            None => (None, full),
        };
        let owner = prefix.is_none().then(|| {
            // The package whose declaration this body IS (spans compare
            // the hoisted bare copy with the package-qualified one).
            [
                pkg,
                self.module.func_decl_scope.get(leaf).map(|p| p.as_str()),
                self.module.pkg_subr_owner.get(leaf).map(|p| p.as_str()),
            ]
            .into_iter()
            .flatten()
            .find(|p| {
                let key = format!("{}::{}", p, leaf);
                self.module.functions.get(&key).map(|f| f.span) == Some(span)
                    || self.module.tasks.get(&key).map(|t| t.span) == Some(span)
            })
        });
        match (prefix, owner.flatten()) {
            (Some(p), _) => vec![
                format!("{}{}", M_ROOT_MARK, self.hier_path(p)),
                leaf.to_string(),
            ],
            (None, Some(pk)) => vec![format!("{}{}", M_ROOT_MARK, pk), leaf.to_string()],
            (None, None) => vec![format!("{}{}", M_SUBR_MARK, leaf)],
        }
    }

    pub(super) fn check_monitor(&mut self) {
        if self.monitor_paused {
            return;
        }
        if let Some((tn, args)) = self.monitor.clone() {
            self.sync_table_to_hashmap();
            // LRM §21.2.3: $monitor prints once when armed, then again only
            // when one of its monitored arguments changes — EXCLUDING the time
            // functions (otherwise it would print every time step). Snapshot
            // the non-time argument values and compare against the last print.
            // The $fmonitor* variants watch args[1..] (args[0] is the file
            // descriptor) and write to that descriptor instead of stdout.
            let is_file = tn.starts_with("$fmonitor");
            let watch: &[Expression] = if is_file {
                args.get(1..).unwrap_or(&[])
            } else {
                &args[..]
            };
            let cur: Vec<Value> = watch
                .iter()
                .filter(|a| !Self::is_monitor_time_arg(a))
                .map(|a| self.eval_expr(a))
                .collect();
            let changed = self.monitor_arg_prev.as_deref() != Some(cur.as_slice());
            if changed {
                // Reinstall the ARMING scope so a `%m` in the format string
                // names the instance that armed the monitor, not whatever the
                // slot-end check happens to run under.
                let saved_block = std::mem::take(&mut self.m_block_scope);
                self.m_block_scope = self.monitor_scope.clone();
                self.monitor_m_active = true;
                if is_file {
                    let tn2 = format!("$monitor{}", &tn["$fmonitor".len()..]);
                    let _ = self.write_file_handle_named(&args, true, &tn2);
                } else {
                    let m = self.format_args(&args, &tn);
                    self.record_output(m.clone());
                    self.stdout_writeln(&m);
                }
                self.monitor_m_active = false;
                self.m_block_scope = saved_block;
                self.monitor_arg_prev = Some(cur);
            }
        }
    }

    /// The receiver of `a.b.method(...)` as an expression, built ONCE per
    /// call site (keyed by the identifier's span) and reused: rebuilding it
    /// per call cloned every path segment string and, worse, discarded the
    /// identifier's resolved-name cache, so each call re-resolved from
    /// scratch. Same context-insensitive caching the AST node itself uses.
    pub(super) fn method_receiver_expr(
        &mut self,
        hier: &HierarchicalIdentifier,
        keep: usize,
    ) -> std::rc::Rc<Expression> {
        // The cached receiver node carries its own resolution cells, which
        // the first evaluation fills under ITS scope hint. Keyed by span
        // alone, every instance of a module executing the same source line
        // then read the first instance's resolution: `q.push_back(..)` in
        // ten sibling instances pushed into one instance's queue. The
        // scope hint is part of the key (interned, so the hot path pays one
        // string hash and no clone).
        let hint_id = match self.name_resolve_hint.borrow().as_deref() {
            None => 0,
            Some(h) => match self.method_receiver_hint_ids.get(h) {
                Some(&id) => id,
                None => {
                    let id = self.method_receiver_hint_ids.len() as u32 + 1;
                    self.method_receiver_hint_ids.insert(h.to_string(), id);
                    id
                }
            },
        };
        // The hint alone is not enough: a loop body or a display runs with
        // NO hint, and every instance's inlined copy of the same source line
        // shares one span. The inliner already prefixed the head identifier
        // per instance (`u0.q` / `u1.q`), so interning it too keeps the
        // entry per instance — `q.size()` inside a `for` in ten siblings
        // otherwise read the first sibling's queue.
        let head = hier.path[0].name.name.as_str();
        let head_id = match self.method_receiver_hint_ids.get(head) {
            Some(&id) => id,
            None => {
                let id = self.method_receiver_hint_ids.len() as u32 + 1;
                self.method_receiver_hint_ids.insert(head.to_string(), id);
                id
            }
        };
        let key = (
            hier.span.start as usize,
            hier.span.end as usize,
            keep,
            hint_id,
            head_id,
        );
        if let Some(e) = self.method_receiver_cache.get(&key) {
            return e.clone();
        }
        let base = HierarchicalIdentifier {
            root: hier.root.clone(),
            path: hier.path[..keep].to_vec(),
            span: hier.span,
            cached_signal_id: std::cell::Cell::new(None),
            cached_resolved_name: std::cell::OnceCell::new(),
        };
        let e = std::rc::Rc::new(Expression::new(ExprKind::Ident(base), hier.span));
        self.method_receiver_cache.insert(key, e.clone());
        e
    }

    /// Resolved flat name of a hierarchical identifier, BORROWED from the
    /// node's own cache whenever it has one (`Cow::Borrowed`), so the hot
    /// read paths stop allocating a String per resolution. The lifetime is
    /// the node's, not `self`'s, so callers can keep the name across `&mut
    /// self` calls. Owned only on the frame-dependent paths (per-process
    /// local-dyn rename, package-ambiguous free names) and the first
    /// computation of an uncacheable shape.
    /// `cg::type_option.<field>` — the covergroup TYPE's option table,
    /// seeded from the body's `type_option.f = v;` items and the §19.7.1
    /// defaults (weight 1, goal 90, strobe 0, merge_instances 0).
    pub(super) fn cg_type_option_get(&mut self, cg: &str, field: &str) -> Option<Value> {
        if !self.module.covergroups.contains_key(cg) {
            return None;
        }
        if !self.cg_type_options.contains_key(cg) {
            let mut t: HashMap<String, Value> = HashMap::default();
            t.insert("weight".into(), Value::from_u64(1, 32));
            t.insert("goal".into(), Value::from_u64(90, 32));
            t.insert("strobe".into(), Value::from_u64(0, 1));
            t.insert("merge_instances".into(), Value::from_u64(0, 1));
            t.insert("comment".into(), Value::from_string(""));
            let seeds: Vec<(String, Expression)> = self
                .module
                .covergroups
                .get(cg)
                .map(|d| {
                    d.items
                        .iter()
                        .filter_map(|it| match it {
                            CovergroupItem::TypeOption { name, val } => {
                                Some((name.clone(), val.clone()))
                            }
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            for (n, e) in seeds {
                let v = self.eval_expr(&e);
                t.insert(n, v);
            }
            self.cg_type_options.insert(cg.to_string(), t);
        }
        self.cg_type_options
            .get(cg)
            .and_then(|t| t.get(field))
            .cloned()
    }

    /// The covergroup instance of a `cg.option.<name>` access.
    pub(super) fn cg_option_target(&mut self, hier: &HierarchicalIdentifier) -> Option<usize> {
        if hier.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let head = Expression::new(
            ExprKind::Ident(HierarchicalIdentifier {
                root: hier.root.clone(),
                path: vec![hier.path[0].clone()],
                span: hier.span,
                cached_signal_id: std::cell::Cell::new(None),
                cached_resolved_name: std::cell::OnceCell::new(),
            }),
            hier.span,
        );
        let h = self.eval_expr(&head).to_u64()? as usize;
        self.cg_index(h)
    }

    /// §19.7 `cg.option.<name>`: the value set at run time, else by the
    /// covergroup body, else the option's default.
    pub(super) fn cg_option_read(&mut self, idx: usize, field: &str) -> Option<Value> {
        let inst = self.cg_heap.get(idx)?.as_ref()?;
        if let Some(v) = inst.options.get(field) {
            return Some(v.clone());
        }
        let body = self.module.covergroups.get(&inst.cg_name).and_then(|d| {
            d.items.iter().find_map(|it| match it {
                CovergroupItem::Option { name, val } if name == field => Some(val.clone()),
                _ => None,
            })
        });
        if let Some(e) = body {
            return Some(self.eval_expr(&e));
        }
        let n = match field {
            "weight" | "at_least" => 1,
            "goal" => 100,
            "auto_bin_max" => 64,
            "per_instance" | "detect_overlap" | "cross_num_print_missing" | "get_inst_coverage" => {
                0
            }
            "name" | "comment" => return Some(Value::from_string("")),
            _ => return None,
        };
        Some(Value::from_u64(n, 32))
    }

    /// The `[cg, type_option, field]` shape of a scoped type-option access.
    pub(super) fn cg_type_option_path(
        &self,
        hier: &HierarchicalIdentifier,
    ) -> Option<(String, String)> {
        if hier.path.len() == 3
            && hier.path.iter().all(|s| s.selects.is_empty())
            && hier.path[1].name.name == "type_option"
            && self
                .module
                .covergroups
                .contains_key(&hier.path[0].name.name)
        {
            Some((
                hier.path[0].name.name.clone(),
                hier.path[2].name.name.clone(),
            ))
        } else {
            None
        }
    }

    /// §6.20.2/§26.3 (issue #425): the qualified key `pkg::leaf` when it names
    /// a COLLECTION. Unpacked-array package localparams keep their QUALIFIED
    /// registration only — the `pkg::LIM` shape key plus per-element signals
    /// `pkg::LIM[i]` — because a bare hoist would leak the name past the
    /// import machinery (§26.2.2) and collide with a module's own same-named
    /// array; contested package variables likewise get their own `pkg::name`
    /// storage. `None` when the qualified spelling names no collection
    /// (ordinary package variables hoist bare as before).
    pub(super) fn pkg_qual_collection_key(&self, pkg: &str, leaf: &str) -> Option<String> {
        let q = format!("{}::{}", pkg, leaf);
        (self.module.arrays.contains_key(&q)
            || self.module.arrays_2d.contains_key(&q)
            || self.module.arrays_nd.contains_key(&q)
            || self.module.dynamic_arrays.contains(&q)
            || self.module.associative_arrays.contains_key(&q))
        .then_some(q)
    }

    pub(super) fn resolve_hier_name<'h>(
        &self,
        hier: &'h HierarchicalIdentifier,
    ) -> std::borrow::Cow<'h, str> {
        if self.name_stats_on {
            self.name_stats[1].set(self.name_stats[1].get() + 1);
        }
        if hier.path.len() == 1 && hier.path[0].selects.is_empty() {
            if let Some(uq) = self.dyn_name_lookup(&hier.path[0].name.name) {
                return std::borrow::Cow::Owned(uq.to_string());
            }
        }
        if !self.pkg_ambiguous_names.is_empty() && hier.path.len() == 1 {
            let leaf = hier.path[0].name.name.as_str();
            if self.pkg_ambiguous_names.contains(leaf)
                && !self
                    .local_stack
                    .last()
                    .is_some_and(|l| l.contains_key(leaf))
            {
                if let Some(Some(pkg)) = self.pkg_scope_stack.last() {
                    let qual = format!("{}::{}", pkg, leaf);
                    if self.signal_name_to_id.contains_key(qual.as_str()) {
                        return std::borrow::Cow::Owned(qual);
                    }
                } else if !self.contested_pkg_vars.is_empty()
                    && self.contested_pkg_vars.contains(leaf)
                {
                    if let Some(qual) = self.contested_pkg_var_in_scope(leaf) {
                        return std::borrow::Cow::Owned(qual);
                    }
                }
            }
        }
        if let Some(cached) = hier.cached_resolved_name.get() {
            return std::borrow::Cow::Borrowed(cached.as_str());
        }
        let owned = self.resolve_hier_name_slow(hier);
        // The slow path memoizes every cacheable shape: borrow that copy.
        if let Some(cached) = hier.cached_resolved_name.get() {
            return std::borrow::Cow::Borrowed(cached.as_str());
        }
        std::borrow::Cow::Owned(owned)
    }

    /// §26.3: the storage of a contested package variable `leaf` (see
    /// `split_contested_package_vars`) referenced bare OUTSIDE a package
    /// subroutine (those resolve in their own package before reaching here):
    /// in a method of a package class that class's package's; in a module /
    /// interface / program the package the design unit imports it from —
    /// unless the scope declares the name itself. `None` keeps the plain
    /// resolution.
    #[cold]
    #[inline(never)]
    pub(super) fn contested_pkg_var_in_scope(&self, leaf: &str) -> Option<String> {
        let qual_in = |pkg: &str| -> Option<String> {
            let q = format!("{}::{}", pkg, leaf);
            self.signal_name_to_id.contains_key(q.as_str()).then_some(q)
        };
        if let Some(Some(ctx)) = self.class_context_stack.last() {
            if let Some(pkg) = self.module.class_decl_pkg.get(ctx.as_str()) {
                return qual_in(pkg);
            }
        }
        self.scope_import_pkg(leaf, &|pkg| {
            self.signal_name_to_id
                .contains_key(format!("{}::{}", pkg, leaf).as_str())
        })
        .map(|pkg| format!("{}::{}", pkg, leaf))
    }

    /// §26.3: the package the running design unit (module / interface /
    /// program instance) imports `leaf` from, among the packages for which
    /// `has(pkg)` holds — an explicit `import P::leaf` first, else a unique
    /// wildcard import. `None` when the scope declares `leaf` itself or no
    /// single package supplies it.
    #[inline(never)]
    pub(super) fn scope_import_pkg(
        &self,
        leaf: &str,
        has: &dyn Fn(&str) -> bool,
    ) -> Option<String> {
        if self.module.def_pkg_imports.is_empty() {
            return None;
        }
        // The running design unit: the instance scope of the process.
        let scope: Option<String> = match self.activation_scope.borrow().as_deref() {
            Some(s) => Some(s.to_string()),
            None => self.name_resolve_hint.borrow().clone(),
        };
        let scope = scope.unwrap_or_default();
        let def: String = if scope.is_empty() {
            if self.module.wildcard_shadowed.contains(leaf)
                || self.module.foreign_displaced.contains(leaf)
            {
                return None;
            }
            self.module.name.clone()
        } else {
            let mut cur: &str = scope.as_str();
            loop {
                let local = format!("{}.{}", cur, leaf);
                if self.signal_name_to_id.contains_key(local.as_str())
                    || self.signals.contains_key(&local)
                {
                    return None;
                }
                if let Some(inst) = self.instance_by_path(cur) {
                    break inst.def_name.clone();
                }
                match cur.rsplit_once('.') {
                    Some((p, _)) => cur = p,
                    None => return None,
                }
            }
        };
        let imports = self.module.def_pkg_imports.get(&def)?;
        if let Some((pkg, _)) = imports
            .iter()
            .find(|(_, item)| item.as_deref() == Some(leaf))
        {
            return has(pkg).then(|| pkg.clone());
        }
        let mut hit: Option<&String> = None;
        for (pkg, item) in imports {
            if item.is_some() || !has(pkg) {
                continue;
            }
            if hit.is_some_and(|h| h != pkg) {
                return None; // §26.3: imported from two packages
            }
            hit = Some(pkg);
        }
        hit.cloned()
    }

    pub(super) fn resolve_hier_name_slow(&self, hier: &HierarchicalIdentifier) -> String {
        // Per-process local dynamic arrays: resolve the bare name through the
        // current process's per-frame rename map and BYPASS the AST-node cache.
        // The cache is shared across concurrent processes (they share AST
        // nodes), so it must not memoize a name that differs per invocation.
        // Single-segment only — `obj.arr`/`a.b.c` are never local-dyn.
        if hier.path.len() == 1 && hier.path[0].selects.is_empty() {
            if let Some(uq) = self.dyn_name_lookup(&hier.path[0].name.name) {
                return uq.to_string();
            }
        }
        // §26.3: a free name inside a package subroutine names THAT package's
        // declaration. Package-scope declarations are hoisted into the shared
        // tables under their bare name as well as `<pkg>::<name>`, and the bare
        // key is last-writer-wins across packages — so `p1::next_step`'s body
        // read p2's `step`. Redirect to the running package's own entry. Only
        // names more than one package declares are ambiguous (the set is empty
        // otherwise), a local of that name still wins, and the result is
        // deliberately NOT memoized: it depends on the call frame, not the node.
        if !self.pkg_ambiguous_names.is_empty() && hier.path.len() == 1 {
            let leaf = hier.path[0].name.name.as_str();
            if self.pkg_ambiguous_names.contains(leaf)
                && !self
                    .local_stack
                    .last()
                    .is_some_and(|l| l.contains_key(leaf))
            {
                if let Some(Some(pkg)) = self.pkg_scope_stack.last() {
                    let qual = format!("{}::{}", pkg, leaf);
                    if self.signal_name_to_id.contains_key(qual.as_str()) {
                        return qual;
                    }
                } else if !self.contested_pkg_vars.is_empty()
                    && self.contested_pkg_vars.contains(leaf)
                {
                    if let Some(qual) = self.contested_pkg_var_in_scope(leaf) {
                        return qual;
                    }
                }
            }
        }
        // §23.8: a name in a method of a class declared inside a (non-top)
        // module resolves in the instance the method runs for (the resolve
        // hint). Every instance shares the class body's AST nodes, so the
        // per-node cache below would pin the first instance's variable.
        if self.any_modclass()
            && hier.root.is_none()
            && hier.path[0].name.name != "$root"
            && !hier.path[0].name.name.contains("::")
            && self.module_class_method_active()
        {
            return self.resolve_hier_name_uncached(hier);
        }
        // Per-hier cache: first call resolves and memoizes the result on the
        // AST node; every subsequent call on the same node returns in O(1)
        // without HashMap lookups, path-join allocation, or hint bookkeeping.
        // This is the dominant win for tight loops like `ram0.mem[i][7:0] = 0`
        // where the same hier objects are re-visited thousands of times.
        // The hint side-effect is deliberately skipped on the cached path:
        // hints only steer the suffix-scan fallback for ambiguous
        // unqualified names, and a resolved (cached) name no longer needs
        // that guidance.
        if let Some(cached) = hier.cached_resolved_name.get() {
            return cached.clone();
        }
        // A parent-rooted ident (an EXPRESSION port actual substituted into a
        // child body — see core's mark_actual_rooted): its name is already the
        // absolute flattened parent name, and the executing child block's
        // scope hint must NOT redirect it to a same-named child signal.
        if hier.root.is_some() {
            let raw = hier
                .path
                .iter()
                .map(|s| s.name.name.as_str())
                .collect::<Vec<_>>()
                .join(".");
            if self.signal_name_to_id.contains_key(raw.as_str())
                || self.signals.contains_key(raw.as_str())
                || self.module.arrays.contains_key(raw.as_str())
                || self.module.arrays_2d.contains_key(raw.as_str())
                || self.module.arrays_nd.contains_key(raw.as_str())
                || self.module.dynamic_arrays.contains(raw.as_str())
                || self.module.associative_arrays.contains_key(raw.as_str())
                || self.module.struct_members.contains_key(raw.as_str())
            {
                let _ = hier.cached_resolved_name.set(raw.clone());
                return raw;
            }
            // Not a table name (parameter, iterator var, …) — fall through to
            // the normal resolution below.
        }
        // §23.6: `$root.` names the root scope and contributes nothing to
        // the flat key — strip it so `$root.tb.x` resolves exactly like the
        // absolute `tb.x` (the top-prefix strip finishes the job). Joining it
        // verbatim matched nothing: reads were 32-bit zeros, writes vanished.
        if hier.path.len() > 1 && hier.path[0].name.name == "$root" {
            let stripped: String = {
                let mut out = String::new();
                for (i, seg) in hier.path[1..].iter().enumerate() {
                    if i > 0 {
                        out.push('.');
                    }
                    out.push_str(seg.name.name.as_str());
                    // Constant segment selects are part of the flat key
                    // (`$root.tb.u_a.gl[1].x` names a for-generate scope).
                    for sel in &seg.selects {
                        if let Some(ix) = Self::try_const_u64(sel) {
                            use std::fmt::Write as _;
                            let _ = write!(out, "[{}]", ix);
                        }
                    }
                }
                out
            };
            // Do NOT memoize through the shared-node cache here — go through
            // the generic top-prefix logic below on a rebuilt name instead.
            if let Some(rest) = stripped.strip_prefix(&format!("{}.", self.module.name)) {
                if self.signal_name_to_id.contains_key(rest) || self.signals.contains_key(rest) {
                    let _ = hier.cached_resolved_name.set(rest.to_string());
                    return rest.to_string();
                }
            }
            let _ = hier.cached_resolved_name.set(stripped.clone());
            return stripped;
        }
        // §26.3: a PACKAGE-scoped reference (`P::s`, `P::s.x`) resolves to the
        // package variable's BARE storage name — the package prefix carries no
        // hierarchy at runtime. Without this, `P::s.x` searched for a signal
        // "P.s.x" and read x while the bare "s.x" member path worked. Only a
        // registered package name is stripped, and never when a same-named
        // SIGNAL exists (an instance path must win).
        if hier.path.len() >= 2
            && hier.path[0].selects.is_empty()
            && self.module.packages.contains(&hier.path[0].name.name)
            && !self
                .signal_name_to_id
                .contains_key(hier.path[0].name.name.as_str())
            && !self.signals.contains_key(&hier.path[0].name.name)
        {
            // §26.3: a contested package variable has its own storage.
            let mut qual = self.pkg_var_storage(&hier.path[0].name.name, &hier.path[1].name.name);
            // §6.20.2/§26.3 (issue #425): an UNPACKED-ARRAY package
            // localparam keeps its QUALIFIED registration only — the
            // `pkg::LIM` shape key plus per-element signals `pkg::LIM[i]` —
            // because a bare hoist would leak the name past the import
            // machinery (§26.2.2) and collide with a module's own
            // same-named array. Stripping the package prefix here resolved
            // `pkg::LIM[3]` to a bare `LIM` that exists nowhere, so the
            // element select fell through to a bit-select of a phantom
            // scalar and read 0 at run time (the const paths were already
            // fixed). Keep the qualified spelling when it names a
            // collection; package VARIABLES still hoist bare as before.
            if qual.is_none() {
                qual =
                    self.pkg_qual_collection_key(&hier.path[0].name.name, &hier.path[1].name.name);
            }
            let stripped: String = qual
                .as_deref()
                .into_iter()
                .chain(
                    hier.path[1 + qual.is_some() as usize..]
                        .iter()
                        .map(|s| s.name.name.as_str()),
                )
                .collect::<Vec<_>>()
                .join(".");
            let _ = hier.cached_resolved_name.set(stripped.clone());
            return stripped;
        }
        // §3.12.1: `$unit::x` is carried as the reserved name "$unit::x", which
        // only EXISTS where a local declaration shadows the compilation-unit
        // variable. Unshadowed, the $unit variable lives under its plain name,
        // so drop the qualifier — going through the generic resolver instead
        // would let the suffix scan bind the reference to some *other*
        // module's shadow copy.
        if let Some(bare) = hier
            .path
            .first()
            .and_then(|s| crate::sv_parser::strip_unit_scope_name(&s.name.name))
        {
            if !self.unit_scope_decl_exists(&hier.path[0].name.name) {
                let mut stripped = hier.clone();
                stripped.path[0].name.name = bare.to_string();
                stripped.cached_signal_id = std::cell::Cell::new(None);
                stripped.cached_resolved_name = std::cell::OnceCell::new();
                let resolved = self.resolve_hier_name_uncached(&stripped);
                let _ = hier.cached_resolved_name.set(resolved.clone());
                return resolved;
            }
        }
        let resolved = self.resolve_hier_name_uncached(hier);
        // OnceCell::set returns Err if already set (race) — ignore; value
        // would be identical anyway.
        let _ = hier.cached_resolved_name.set(resolved.clone());
        resolved
    }

    /// Whether the reserved `$unit::<name>` storage really exists here — i.e.
    /// this scope shadows the compilation-unit declaration, so the qualified
    /// reference has its own object. An unpacked array / queue / assoc array
    /// lives in its own table rather than in `signals`, so all of them have to
    /// be consulted: missing one made `$unit::arr[i]` fall back to the bare
    /// name and read the module's own array.
    pub(super) fn unit_scope_decl_exists(&self, name: &str) -> bool {
        self.signal_name_to_id.contains_key(name)
            || self.signals.contains_key(name)
            || self.module.arrays.contains_key(name)
            || self.module.arrays_2d.contains_key(name)
            || self.module.arrays_nd.contains_key(name)
            || self.module.dynamic_arrays.contains(name)
            || self.module.associative_arrays.contains_key(name)
            || self.module.struct_members.contains_key(name)
    }

    #[inline]
    pub(super) fn instance_by_path(&self, path: &str) -> Option<&crate::elaborate::ElabInstance> {
        self.instance_path_order
            .binary_search_by(|&idx| self.module.instances[idx].path.as_str().cmp(path))
            .ok()
            .map(|pos| &self.module.instances[self.instance_path_order[pos]])
    }

    pub(super) fn resolve_hier_name_uncached(&self, hier: &HierarchicalIdentifier) -> String {
        // §23.9 / §13.5 (#534): an unpacked-array FORMAL of the running
        // method is bound under its bare name (with a frame marker); it
        // shadows a same-named class array constant or module array, so the
        // class-scope lift below must not take the name.
        if hier.root.is_none()
            && hier.path.len() == 1
            && let Some(frame) = self.local_stack.last()
            && frame.contains_key(&format!("\u{1}array_formal:{}", hier.path[0].name.name))
        {
            return hier.path[0].name.name.clone();
        }
        // §25.3 / §23.8: a hierarchical reference through an interface port
        // names its connected instance, before ordinary suffix fallbacks.
        if !self.module.interface_port_aliases.is_empty() {
            let mut raw = String::new();
            for (i, seg) in hier.path.iter().enumerate() {
                if seg.name.name == "$root" && i == 0 {
                    continue;
                }
                if !raw.is_empty() {
                    raw.push('.');
                }
                raw.push_str(&seg.name.name);
                if i + 1 < hier.path.len() {
                    for select in &seg.selects {
                        if let Some(index) = Self::try_const_u64(select) {
                            use std::fmt::Write as _;
                            let _ = write!(raw, "[{index}]");
                        }
                    }
                }
            }
            let resolve = |name: &str| {
                let bound = xezim_core::elaborate::interface_ports::resolve(
                    name,
                    &self.module.interface_port_aliases,
                    &self.module.name,
                );
                // An enclosing hint can itself end in a port name. Rebasing
                // a relative name headed by that same port would duplicate
                // it (actual.port.member). Only accept a binding that names
                // declared storage or a subroutine; otherwise ordinary scope
                // lookup must continue (§23.8).
                let known = bound != name
                    && (self.signal_name_to_id.contains_key(bound.as_ref())
                        || self.signals.contains_key(bound.as_ref())
                        || self.module.parameters.contains_key(bound.as_ref())
                        || self.module.arrays.contains_key(bound.as_ref())
                        || self.module.arrays_2d.contains_key(bound.as_ref())
                        || self.module.arrays_nd.contains_key(bound.as_ref())
                        || self.module.dynamic_arrays.contains(bound.as_ref())
                        || self.module.associative_arrays.contains_key(bound.as_ref())
                        || self.module.struct_members.contains_key(bound.as_ref())
                        || self.module.functions.contains_key(bound.as_ref())
                        || self.module.tasks.contains_key(bound.as_ref())
                        || self.instance_by_path(bound.as_ref()).is_some());
                known.then(|| bound.into_owned())
            };
            if let Some(bound) = resolve(&raw) {
                return bound;
            }
            // A rewritten absolute name such as `p.pv` already names its
            // declaration. Prefixing the hint `p` again could turn it into
            // `p.p.pv`, where `p.p` happens to be an interface-port alias.
            let declared = raw.contains('.')
                && (self.signal_name_to_id.contains_key(raw.as_str())
                    || self.signals.contains_key(&raw)
                    || self.module.arrays.contains_key(&raw)
                    || self.module.arrays_2d.contains_key(&raw)
                    || self.module.arrays_nd.contains_key(&raw)
                    || self.module.dynamic_arrays.contains(&raw)
                    || self.module.queue_vars.contains(&raw)
                    || self.module.associative_arrays.contains_key(&raw)
                    || self.module.struct_members.contains_key(&raw)
                    || self.module.functions.contains_key(&raw)
                    || self.module.tasks.contains_key(&raw));
            if !declared
                && hier.root.is_none()
                && let Some(qualified) = self.hint_scoped(&raw)
                && let Some(bound) = resolve(&qualified)
            {
                return bound;
            }
        }
        let common_prefix_len = |a: &str, b: &str| -> usize {
            let mut n = 0usize;
            for (sa, sb) in a.split('.').zip(b.split('.')) {
                if sa == sb {
                    n += 1;
                } else {
                    break;
                }
            }
            n
        };
        fn parent_of(s: &str) -> &str {
            s.rsplit_once('.').map(|(p, _)| p).unwrap_or("")
        }
        let raw = hier
            .path
            .iter()
            .map(|s| s.name.name.as_str())
            .collect::<Vec<_>>()
            .join(".");
        // §8/§23.9: in a method of a class declared inside a module, a
        // dotted name headed by an item of the object's instance (an
        // unpacked struct, an interface or module instance) is that
        // instance's.
        if hier.path.len() >= 2 && self.modclass_depth.get() > 0 {
            let inst = self.activation_scope.borrow().clone();
            if let Some(inst) = inst.filter(|i| !i.is_empty()) {
                let head = format!("{}.{}", inst, hier.path[0].name.name);
                let full = format!("{}.{}", inst, raw);
                if self.signal_name_to_id.contains_key(full.as_str())
                    || self.signals.contains_key(&full)
                    || self.module.struct_members.contains_key(&head)
                    || self.instance_by_path(&head).is_some()
                    || self.signal_name_to_id.contains_key(head.as_str())
                    || self.module.var_decl_types.contains_key(&head)
                {
                    return full;
                }
            }
        }
        // `a.b.c` where `a.b` carries a bit layout (a nested packed struct, or
        // an untagged union whose members share one storage): the dotted name
        // IS the leaf. Without this the suffix-scan fallback below collapses it
        // to the last segment (`c`) and writes a stray module-scope signal.
        if hier.path.len() >= 2 && self.packed_leaf_of_hier(&raw).is_some() {
            return raw;
        }
        // `p.a` where `p` is an unpacked STRUCT variable: the dotted name is a
        // member access, not a hierarchical instance path. The suffix scan below
        // would collapse it to the last segment, so `p.a = 5` silently wrote an
        // unrelated module-scope signal that happened to be called `a`.
        if hier.path.len() >= 2
            && self
                .module
                .struct_members
                .contains_key(&hier.path[0].name.name)
        {
            return raw;
        }
        // LRM §25.9: if the leading segment names a virtual-interface
        // alias — a formal arg in the current task call frame, or a
        // plain `virtual <iface>` variable (`vif = bus;`) — rewrite the
        // path to use the bound interface instance name. Has to fire
        // before the class-property dispatch below so a task formal
        // `vif` doesn't get shadowed by a same-named class prop.
        if hier.path.len() >= 2 {
            let first = hier.path[0].name.name.as_str();
            if let Some(bound) = self.iface_alias_for(first) {
                let rest = hier
                    .path
                    .iter()
                    .skip(1)
                    .map(|s| s.name.name.as_str())
                    .collect::<Vec<_>>()
                    .join(".");
                return format!("{}.{}", bound, rest);
            }
        }
        // LRM §25.8 virtual-interface late-dispatch. When the inner
        // segment of `<vif>.<member>...` is a class property declared
        // `virtual <iface_t>`, replace it with the bound interface
        // instance name. Two recognised shapes:
        //   `vif.member`           → this_stack's class.virtual_iface[vif]
        //   `obj.vif.member`       → obj's class.virtual_iface[vif]
        if hier.path.len() >= 2 {
            let segs: Vec<&str> = hier.path.iter().map(|s| s.name.name.as_str()).collect();
            // Form 1: bare `vif.member.…` — `vif` looked up against
            // the current `this` instance.
            if let Some(Some(this_h)) = self.this_stack.last() {
                let cls_name = if let Some(Some(inst)) = self.heap.get(*this_h) {
                    Some(inst.class_name.clone())
                } else {
                    None
                };
                if let Some(cn) = cls_name {
                    let cls_has_vif = self.class_vif_decl(&cn, segs[0]).is_some();
                    if cls_has_vif {
                        if let Some((bound, _mp)) = self
                            .virtual_iface_bindings
                            .get(&(*this_h, segs[0].to_string()))
                        {
                            let rest = segs[1..].join(".");
                            return format!("{}.{}", bound, rest);
                        }
                    }
                }
            }
            // Form 2: `obj.vif.member.…` — `obj` is a local class
            // handle whose class has `vif` as virtual-iface.
            if hier.path.len() >= 3 {
                let obj_name = segs[0];
                let obj_handle_v = if let Some(locals) = self.local_stack.last() {
                    locals.get(obj_name).cloned()
                } else {
                    None
                }
                .or_else(|| self.get_signal_value_by_name(obj_name));
                if let Some(v) = obj_handle_v {
                    let handle = v.to_u64().unwrap_or(0) as usize;
                    if handle != 0 && handle < self.heap.len() {
                        if let Some(Some(inst)) = self.heap.get(handle) {
                            let cn = inst.class_name.clone();
                            let cls_has_vif = self.class_vif_decl(&cn, segs[1]).is_some();
                            if cls_has_vif {
                                if let Some((bound, _mp)) = self
                                    .virtual_iface_bindings
                                    .get(&(handle, segs[1].to_string()))
                                {
                                    let rest = segs[2..].join(".");
                                    return format!("{}.{}", bound, rest);
                                }
                            }
                        }
                    }
                }
            }
        }
        // Scope-qualified shadowing for SINGLE-SEGMENT bare names — LRM
        // §22.4 / §23.6: a local declaration in the enclosing scope
        // shadows a same-named member brought in via wildcard package
        // import. If `hint.name` resolves, prefer it over the global
        // bare name. Without this, two modules' anon-enum members
        // (e.g. cv32e40p_alu_div::FINISH=2 vs
        // cv32e40p_pkg::mult_state_e::FINISH=4) share one global slot;
        // whichever was registered last wins and the divider's FSM
        // truncates State_SN into a 2-bit register, losing FINISH.
        // §23.6 ordering: the current activation's own scope wins first
        // — the scope-qualified key for a scoped process, the bare
        // key for a top-scope process (its own module declares the name).
        // The resolve hint guides only names that scope does NOT declare:
        // the hint is routinely set by an unrelated hierarchical reference
        // earlier in the same block (a u_child.x = 1 write right before a
        // loop), and letting it win over the current scope's own
        // declaration resolves a simple name DOWNWARD into the instance
        // — illegal per §23.6 — which the per-hier cache then
        // froze, silently retargeting the parent's NBAs into the child.
        // Mirrors the bytecode compiler's lookup_signal_id scope-first
        // order, gated on `activation_scope`: the own-scope lookup uses
        // the ACTIVATION's scope (never the ratcheted hint, never a stale
        // current_pid), and compile-time evals — which leave
        // activation_scope None and deliberately install the correct
        // scope through the resolve hint — keep their hint-first
        // order.
        if !raw.contains('.') {
            let activation = self.activation_scope.borrow();
            match activation.as_deref() {
                // Top-scope activation: its own module's names are the
                // bare keys.
                Some("") => {
                    if self.signal_name_to_id.contains_key(raw.as_str()) {
                        return raw;
                    }
                }
                // Instance-scoped activation: the scope-qualified key.
                // When the scope does not declare the name, fall through
                // — the bare key may be another module's hoisted
                // name or a shared enum-member slot, and the hint (the
                // own scope at activation entry) plus the pre-existing
                // fallbacks own that decision.
                Some(scope) => {
                    let scoped = format!("{}.{}", scope, raw);
                    if self.signal_name_to_id.contains_key(scoped.as_str()) {
                        return scoped;
                    }
                    // §23.9: the scope's own unpacked array / queue / assoc
                    // array (kept out of the signal table) shadows a
                    // same-named one of the top module.
                    if self.modclass_depth.get() > 0 && self.unit_scope_decl_exists(&scoped) {
                        return scoped;
                    }
                    // An input-port net of this instance, left out as
                    // unobserved, would have been found right here.
                    self.signal_name_to_id.check_miss(scoped.as_str());
                }
                // No activation (build/detection eval): hint-first, the
                // order those contexts are built around. A stale
                // current_pid's scope bound `user`'s bare enum member `C`
                // to the sibling shadower's local `int C`, and a
                // testbench's bare `d` to the child formal `u1.d`
                // (self-loop, x forever).
                None => {}
            }
            if let Some(hint) = self.name_resolve_hint.borrow().as_ref() {
                let scoped = format!("{}.{}", hint, raw);
                if self.signal_name_to_id.contains_key(scoped.as_str()) {
                    return scoped;
                }
            }
        } else if !self.signal_name_to_id.contains_key(raw.as_str()) {
            // A DOTTED reference from a hinted scope (`core.seq` inside a
            // class method of an object built in `u_w`, or inside a
            // nested block of `u_w`) names a sibling instance's member:
            // try it under the hint and each parent scope (§23.6 upward
            // resolution). Only when the raw name is not itself a signal,
            // so an absolute path keeps winning.
            let hint = self.name_resolve_hint.borrow().clone();
            if let Some(hint) = hint.as_deref() {
                let mut scope = hint;
                loop {
                    let scoped = format!("{}.{}", scope, raw);
                    if self.signal_name_to_id.contains_key(scoped.as_str()) {
                        let parent = parent_of(&scoped).to_string();
                        *self.name_resolve_hint.borrow_mut() = Some(parent);
                        return scoped;
                    }
                    match scope.rsplit_once('.') {
                        Some((p, _)) => scope = p,
                        None => break,
                    }
                }
            }
        }
        // Exact dotted name match first.
        if self.signal_name_to_id.contains_key(raw.as_str()) {
            if raw.contains('.') {
                let parent = parent_of(&raw).to_string();
                *self.name_resolve_hint.borrow_mut() = Some(parent);
            }
            return raw;
        }
        // Fast paths to avoid the O(N) suffix scan below. Critical for tight
        // memory-init loops (e.g. c910 testbench wipe).
        //
        // 1) raw is a known array / queue / assoc / dynamic array name.
        // 2) raw starts with the top testbench module's name (e.g. `tb.`),
        //    but arrays/signals are keyed without that prefix. Try stripping
        //    the first segment and re-checking.
        let check_known_or_signal = |name: &str, this: &Self| -> bool {
            this.module.arrays.contains_key(name)
                || this.module.arrays_2d.contains_key(name)
                || this.module.arrays_nd.contains_key(name)
                || this.module.dynamic_arrays.contains(name)
                // An UNPACKED STRUCT has no container signal — its leaves are
                // the storage — so a hierarchical reference to the struct
                // itself (a bound checker's `host_mod.payload` read via §23.8
                // upward naming, then rendered by %p) found no "signal" and
                // fell through to x. The struct-variable registry knows the
                // container name.
                || this.module.struct_members.contains_key(name)
                // ... and when that registry is empty (submodule-local
                // typedef structs), the declared-type table still knows the
                // container by its scoped name.
                || this.module.var_decl_types.contains_key(name)
        };
        if check_known_or_signal(&raw, self) {
            if raw.contains('.') {
                let parent = parent_of(&raw).to_string();
                *self.name_resolve_hint.borrow_mut() = Some(parent);
            }
            return raw;
        }
        if let Some((head, rest)) = raw.split_once('.') {
            if (self.module.classes.contains_key(head) || self.class_of_var(head).is_none())
                && !self
                    .local_stack
                    .last()
                    .is_some_and(|f| f.contains_key(head))
                && (self.signal_name_to_id.contains_key(rest) || check_known_or_signal(rest, self))
            {
                let out = rest.to_string();
                if out.contains('.') {
                    let parent = parent_of(&out).to_string();
                    *self.name_resolve_hint.borrow_mut() = Some(parent);
                }
                return out;
            }
        }
        let leaf = hier
            .path
            .last()
            .map(|s| s.name.name.clone())
            .unwrap_or_default();
        if leaf.is_empty() {
            return raw;
        }

        // Heuristic fallback for unresolved single-segment names:
        // choose a suffix match guided by the most recent hierarchical hint.
        // Skip the O(N) suffix scan if the "leaf" already contains dots —
        // that means the parser flattened a deep hier path into one segment
        // (testbench probes like `assign x = x_soc.x_cpu_sub_system_axi.....fifo_full`),
        // and no key in signal_name_to_id can end with `.<full_dotted_path>`
        // since the table is keyed by the full path itself, not nested deeper.
        // On c910, this scan over 35.7M signals dominated time-0 settle
        // (575s / 11K probes).
        //
        // §8.5 / §23.9: inside a class method a bare name its class (or an
        // ancestor) declares as a property IS that property. The leaf scan
        // below matched any design signal with that last segment — e.g. the
        // member `h` of an unpacked-struct variable `r` (`r.h`) — so a
        // method's `h = new` constructed nothing and every later `g.h.<m>`
        // read x.
        if hier.path.len() == 1 && hier.path[0].selects.is_empty() {
            if let Some(Some(ctx)) = self.class_context_stack.last() {
                if self.prop_owners(ctx).contains_key(leaf.as_str()) {
                    return raw;
                }
            }
        }
        if hier.path.len() == 1 && !leaf.contains('.') {
            // An elided input port with this leaf could have been the match
            // this heuristic picked before the port was left out.
            if !self.module.elided_port_leaves.is_empty()
                && self.module.elided_port_leaves.contains(leaf.as_str())
            {
                names::report_elided_lookup(&leaf);
            }
            // Use leaf-name reverse index — O(1) instead of O(N) scan.
            let candidates: Vec<&str> = self
                .leaf_name_to_ids
                .get(leaf.as_str())
                .map(|ids| ids.iter().filter_map(|&id| self.name_opt(id)).collect())
                .unwrap_or_default();
            if !candidates.is_empty() {
                let hint_owned = self.name_resolve_hint.borrow().clone().unwrap_or_default();
                let mut best: Option<&str> = None;
                let mut best_score: isize = -1;
                for key in candidates {
                    let key_parent = parent_of(key);
                    let score = common_prefix_len(&hint_owned, key_parent) as isize;
                    match best {
                        None => {
                            best = Some(key);
                            best_score = score;
                        }
                        Some(prev) => {
                            let prefer = if score != best_score {
                                score > best_score
                            } else {
                                let kd = key.split('.').count();
                                let pd = prev.split('.').count();
                                kd < pd || (kd == pd && key.len() < prev.len())
                            };
                            if prefer {
                                best = Some(key);
                                best_score = score;
                            }
                        }
                    }
                }
                if let Some(k) = best {
                    let parent = parent_of(k).to_string();
                    *self.name_resolve_hint.borrow_mut() = Some(parent);
                    return k.to_string();
                }
            }
        }
        // §8.25.1: a 2-segment path `[alias, member]` where the leading
        // segment is a TYPE-DEF specialization alias for a PARAMETERIZED
        // class (`w_t = wrapper#(e_t)`) and the member is a STATIC
        // associative-array collection. Element access (`w_t::map[k]`,
        // `size()`, `foreach`) must read/store under the SAME per-spec key
        // (`wrapper#e_t::map`) that an in-method bare access uses
        // (`spec_static_coll_key`); otherwise the suffix scan below collapses
        // this to bare `map`, so an in-method write (per-spec) is never seen
        // by a module-scope read (bare). Only STATIC COLLECTIONS are
        // intercepted — scalar statics keep their own resolution path.
        if hier.path.len() == 2 {
            if let Some((base, sig)) = self.resolve_typedef_spec(&hier.path[0].name.name) {
                let member = &hier.path[1].name.name;
                if self.member_is_static_coll(&base, member) {
                    return format!(
                        "{}#{}::{}",
                        base,
                        self.canonicalize_spec_sig(&base, &sig),
                        member
                    );
                }
            }
            // §8.9: a 2-segment path `[Class, member]` where the leading
            // segment is a PLAIN (non-typedef) CLASS name and the member is a
            // STATIC collection whose bare name COLLIDES across classes.
            // Such collisions are stored per DECLARING class
            // (`{DeclClass}::{member}`), so a module-scope read like
            // `A.settings.size()` / `A.settings[k]` must resolve to that
            // per-class key, not collapse to the bare `settings` (which would
            // hit whichever sibling stored under the bare name).
            if hier.path[0].selects.is_empty() {
                let lead = hier.path[0].name.name.clone();
                // The leading segment may be a class-LOCAL through a typedef
                // alias (`this_type` inside a base method — each class
                // declares `typedef base this_type`), so resolve it to the
                // concrete class first; a real class name resolves to itself.
                let cls = self
                    .resolve_typeref_class_name_str(&lead)
                    .filter(|c| self.module.classes.contains_key(c))
                    .unwrap_or(lead.clone());
                if self.module.classes.contains_key(&cls) {
                    let member = &hier.path[1].name.name;
                    if self.member_is_static_coll(&cls, member) {
                        if let Some(key) = self.static_prop_key(&cls, member) {
                            // Per-declaring-class storage (§8.9). A static
                            // collection COLLIDES when two classes declare it;
                            // and a derived class that REDECLARES a static
                            // collection (rather than merely inheriting it) also
                            // gets its OWN cell keyed by the declaring class —
                            // `d::m_q` must not share `b::m_q` when `d` declares
                            // its own `static m_q`. Inherited-only access (the
                            // sole declarer is an ancestor) keeps the bare name,
                            // sharing the ancestor's single cell. NOT the
                            // `#spec` case here — that is
                            // handled by the typedef-alias block above and by
                            // `spec_static_coll_key`, and intercepting it would
                            // split storage for a parameterized subclass
                            // (`simple_seq_lib::typewide` reading a per-spec key
                            // that registration writes under the bare name).
                            if self.static_coll_name_collides(member)
                                || self.class_declares_coll(&cls, member)
                            {
                                return key;
                            }
                        }
                    }
                }
            }
        }

        // Multi-segment suffix fallback: for paths like "uut.picorv32_core.cpu_state",
        // look for keys ending with ".uut.picorv32_core.cpu_state", preferring the one
        // closest (by common-prefix) to the current scope hint.
        // Use leaf_name_to_ids index to narrow candidates to signals whose
        // last `.`-segment matches the leaf — turns the O(N) scan over
        // 35M signals into O(M) where M is the number of signals sharing
        // that leaf name (usually small).
        if hier.path.len() > 1 {
            let suffix = format!(".{}", raw);
            let candidates: Vec<&str> = if let Some(ids) = self.leaf_name_to_ids.get(leaf.as_str())
            {
                ids.iter()
                    .filter_map(|&id| self.name_opt(id))
                    .filter(|k: &&str| k.ends_with(&suffix) || *k == raw.as_str())
                    .collect()
            } else {
                Vec::new()
            };
            if !candidates.is_empty() {
                let hint_owned = self.name_resolve_hint.borrow().clone().unwrap_or_default();
                let mut best: Option<&str> = None;
                let mut best_score: isize = -1;
                for key in candidates {
                    let key_parent = parent_of(key);
                    let score = common_prefix_len(&hint_owned, key_parent) as isize;
                    match best {
                        None => {
                            best = Some(key);
                            best_score = score;
                        }
                        Some(prev) => {
                            let prefer = if score != best_score {
                                score > best_score
                            } else {
                                let kd = key.split('.').count();
                                let pd = prev.split('.').count();
                                kd < pd || (kd == pd && key.len() < prev.len())
                            };
                            if prefer {
                                best = Some(key);
                                best_score = score;
                            }
                        }
                    }
                }
                if let Some(k) = best {
                    let parent = parent_of(k).to_string();
                    *self.name_resolve_hint.borrow_mut() = Some(parent);
                    return k.to_string();
                }
            }
        }

        // IEEE 1800-2023 §23.8 (upward name referencing) / §23.10.1 (a module
        // inserted with `bind` resolves names in the scope of the bind
        // TARGET, i.e. through the target instance's own scope chain): a
        // hierarchical name whose first segment did not resolve as a downward
        // path is searched UPWARD through the enclosing instance scopes.
        // Starting at the executing scope (for a bound monitor: the bound
        // instance, whose parent is the bind target) and walking one segment
        // up at a time, at each ancestor scope P the first segment can match
        //   a) an instance declared in P — the reference resolves under
        //      `P.first`, or
        //   b) the MODULE DEFINITION name of P's own instance (§23.8: the
        //      first name may be "the name of a module", meaning the nearest
        //      enclosing instance of that module) — the reference resolves
        //      under P itself. With P = "" this also covers naming the
        //      top-level module.
        // xezim inlines a bound module as a regular child of the target
        // (§23.11), so this walk is exactly what lets the bound module read
        // the target's — and any enclosing scope's — signals by naming those
        // scopes (e.g. `dut_top.top_secret` from a monitor bound under
        // `dut_top.sub.core`).
        if (hier.path.len() > 1 || raw.contains('.')) && !self.module.instances.is_empty() {
            let Some((first, rest)) = raw.split_once('.') else {
                return raw;
            };
            let exists = |name: &str, this: &Self| -> bool {
                this.signal_name_to_id.contains_key(name) || check_known_or_signal(name, this)
            };
            // Does scope path `p` ("" = top) instantiate a module named `d`?
            // §23.11: bound instances preserve their hosts' source
            // definition names, including names that contain suffixes.
            let def_matches = |p: &str, d: &str, this: &Self| -> bool {
                if p.is_empty() {
                    return this.module.name == d;
                }
                this.instance_by_path(p).is_some_and(|i| i.def_name == d)
            };
            let hint_owned = self.name_resolve_hint.borrow().clone();
            // Walk from the stable executing scope first; the transient
            // resolve hint is a fallback for call sites that only set it.
            // The hint is a RATCHET (every resolution re-points it at the
            // resolved signal's parent), so a block's second upward
            // reference can start the walk from the wrong ancestor — add
            // the edge/comb block's own scope as a last resort so
            // `host.sig <= other_scope.x` still finds the host after the
            // RHS resolution moved the hint.
            let mut starts: Vec<&str> = Vec::new();
            if !self.current_scope.is_empty() {
                starts.push(self.current_scope.as_str());
            }
            if let Some(h) = hint_owned.as_deref() {
                if !h.is_empty() && !starts.contains(&h) {
                    starts.push(h);
                }
            }
            if !self.m_block_scope.is_empty() && !starts.contains(&self.m_block_scope.as_str()) {
                starts.push(self.m_block_scope.as_str());
            }
            if starts.is_empty() {
                starts.push("");
            }
            for &start in &starts {
                let mut p: &str = start;
                loop {
                    // b) `first` names the module DEFINITION of scope `p`.
                    if def_matches(p, first, self) {
                        let full = if p.is_empty() {
                            rest.to_string()
                        } else {
                            format!("{}.{}", p, rest)
                        };
                        if exists(&full, self) {
                            let parent = parent_of(&full).to_string();
                            *self.name_resolve_hint.borrow_mut() = Some(parent);
                            return full;
                        }
                    }
                    // a) `first` names an instance declared in scope `p`.
                    let inst = if p.is_empty() {
                        first.to_string()
                    } else {
                        format!("{}.{}", p, first)
                    };
                    if self.instance_by_path(&inst).is_some() {
                        let full = format!("{}.{}", inst, rest);
                        if exists(&full, self) {
                            let parent = parent_of(&full).to_string();
                            *self.name_resolve_hint.borrow_mut() = Some(parent);
                            return full;
                        }
                    }
                    if p.is_empty() {
                        break;
                    }
                    p = parent_of(p);
                }
            }
        }

        if self.signal_name_to_id.contains_key(leaf.as_str()) {
            return leaf;
        }

        // Last-resort compatibility fallback.
        leaf
    }

    /// Fast signal read avoiding String allocation.
    /// Uses cached_signal_id to remember the signal name as &str key for HashMap lookup.
    #[inline]
    pub(super) fn fast_signal_read(&self, hier: &HierarchicalIdentifier) -> Value {
        let is_ambiguous_leaf = hier.path.len() == 1 && !hier.path[0].name.name.contains('.');
        // Try cached signal ID first (O(1) Vec access)
        if let Some(id) = hier.cached_signal_id.get() {
            if !is_ambiguous_leaf {
                let mut v = self.signal_table[id].clone();
                if self.signal_signed[id] {
                    v.is_signed = true;
                }
                if self.signal_real[id] {
                    v.is_real = true;
                }
                return v;
            }
        }
        // First access: resolve name and cache ID
        let name = self.resolve_hier_name(hier);
        if let Some(&id) = self.signal_name_to_id.get(name.as_ref()) {
            hier.cached_signal_id.set(Some(id));
            let mut v = self.signal_table[id].clone();
            if self.signal_signed[id] {
                v.is_signed = true;
            }
            if self.signal_real[id] {
                v.is_real = true;
            }
            return v;
        }
        // Fallback
        let mut v = self.signals.get(&*name).cloned().unwrap_or_else(|| {
            // §7.4.6: an out-of-range element read (`mem[9]` of `mem[0:3]`
            // names no signal) yields x at the ELEMENT width, not a 1-bit
            // x that a store then zero-extends.
            let w = name
                .rfind('[')
                .and_then(|p| self.array_first_id.get(&name[..p]))
                .map(|&(first, _, _)| {
                    if is_packed_id(first) {
                        self.packed.width(first)
                    } else {
                        self.signal_widths.get(first).copied().unwrap_or(1)
                    }
                })
                .unwrap_or(1);
            Value::new(w.max(1))
        });
        if self.signed_signals.contains(&*name) {
            v.is_signed = true;
        }
        v
    }

    #[inline(always)]
    pub(super) fn vector_ref_bits(&self, src: VectorRef, width: u8) -> (u64, u64) {
        let value = &self.signal_table[src.sig_id as usize];
        value.slice_bits_swar(src.lo as usize, width as usize)
    }

    #[inline(always)]
    pub(super) fn vector_gate_eval(&self, op: &VectorGate) -> (VectorRef, u8, u64, u64) {
        let (dst, width, mut val, xz) = match *op {
            VectorGate::Buf {
                dst,
                src,
                width,
                invert,
            } => {
                let (sv, sx) = self.vector_ref_bits(src, width);
                let mask = if width == 64 {
                    u64::MAX
                } else {
                    (1u64 << width) - 1
                };
                if invert {
                    (dst, width, (!sv) & !sx & mask, sx & mask)
                } else {
                    (dst, width, sv & mask, sx & mask)
                }
            }
            VectorGate::Bin {
                dst,
                a,
                b,
                width,
                op,
                invert,
            } => {
                let (av, ax) = self.vector_ref_bits(a, width);
                let (bv, bx) = self.vector_ref_bits(b, width);
                let mask = if width == 64 {
                    u64::MAX
                } else {
                    (1u64 << width) - 1
                };
                let (rv, rx) = match op {
                    GateBin::And => {
                        let one = (av & !ax) & (bv & !bx);
                        let zero = (!av & !ax) | (!bv & !bx);
                        (one & mask, (!(one | zero)) & mask)
                    }
                    GateBin::Or => {
                        let one = (av & !ax) | (bv & !bx);
                        let zero = (!av & !ax) & (!bv & !bx);
                        (one & mask, (!(one | zero)) & mask)
                    }
                    GateBin::Xor => {
                        let unknown = (ax | bx) & mask;
                        ((av ^ bv) & !unknown & mask, unknown)
                    }
                };
                let rv = if invert { (!rv) & !rx & mask } else { rv };
                (dst, width, rv, rx)
            }
            VectorGate::Mux {
                dst,
                s,
                t,
                e,
                width,
            } => {
                let (sv, sx) = self.vector_ref_bits(s, width);
                let (tv, tx) = self.vector_ref_bits(t, width);
                let (ev, ex) = self.vector_ref_bits(e, width);
                let mask = if width == 64 {
                    u64::MAX
                } else {
                    (1u64 << width) - 1
                };
                let known_one = sv & !sx & mask;
                let known_zero = !sv & !sx & mask;
                let equal = !(tv ^ ev | tx ^ ex) & mask;
                let unknown_equal = sx & equal;
                (
                    dst,
                    width,
                    (known_one & tv) | (known_zero & ev) | (unknown_equal & tv),
                    (known_one & tx)
                        | (known_zero & ex)
                        | (unknown_equal & tx)
                        | (sx & !equal & mask),
                )
            }
        };
        let mask = if width == 64 {
            u64::MAX
        } else {
            (1u64 << width) - 1
        };
        val &= mask;
        (dst, width, val, xz & mask)
    }

    #[inline(always)]
    pub(super) fn fused_vector_commit(
        &mut self,
        dst: VectorRef,
        width: u8,
        src_v: u64,
        src_x: u64,
        sdf_any: bool,
    ) -> bool {
        let id = dst.sig_id as usize;
        let (base_v, base_x) = self.signal_table[id].raw_bits();
        let high = dst.lo + width as u32 - 1;
        let (new_v, new_x) =
            Self::compose_inline_range_bits(base_v, base_x, src_v, src_x, dst.lo, high);
        let mask = if width == 64 {
            u64::MAX
        } else {
            ((1u64 << width) - 1) << dst.lo
        };
        if (base_v ^ new_v) & mask == 0 && (base_x ^ new_x) & mask == 0 {
            return false;
        }
        // §10.6.2: a forced net keeps its forced value against its gate driver.
        if !self.forced_signals.is_empty() && self.forced_signals.contains_key(&id) {
            return false;
        }
        if sdf_any && self.sdf_delays.get(id).copied().unwrap_or(0) > 0 && self.time > 0 {
            let mut value = self.signal_table[id].clone();
            value.set_inline_bits(new_v, new_x);
            value.is_signed = self.signal_signed[id];
            self.schedule_delayed(id, value);
            return false;
        }
        self.signal_table[id].set_inline_bits(new_v, new_x);
        self.signal_table[id].is_signed = self.signal_signed[id];
        self.sync_mirror(id);
        self.table_modified = true;
        self.after_signal_write_premirrored(id);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §23.8 / §25.3: a port-scoped lookup hint cannot duplicate that port
    /// in the physical interface path, including assertion clock lookups.
    #[test]
    fn interface_port_hint_resolves_declared_clock() {
        let sim = crate::simulate(
            r#"
interface bus_if; logic clk; endinterface
module agent(bus_if bus); endmodule
module top; bus_if bus(); agent a(bus); initial #1 $finish; endmodule
module control; endmodule
"#,
            2,
        )
        .unwrap();
        let span = crate::ast::Span::new(0, 0);
        let expression =
            Simulator::hier_ident_expr(Simulator::iface_path_segs("bus.clk", span), span);
        let ExprKind::Ident(hier) = expression.kind else {
            unreachable!()
        };
        for hint in ["top.a.bus", "top.a", "top.bus"] {
            *sim.name_resolve_hint.borrow_mut() = Some(hint.to_string());
            let resolved = sim.resolve_hier_name_uncached(&hier);
            // Both the formal port path and the physical path may remain
            // valid names, but they must select the same signal storage.
            assert_eq!(
                sim.signal_name_to_id.get(resolved.as_str()),
                sim.signal_name_to_id.get("top.bus.clk"),
                "{hint}: {resolved}"
            );
            assert!(sim.signal_name_to_id.contains_key(resolved.as_str()));
        }
    }
}
