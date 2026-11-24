//! Signal, parameter, enum and package name lookup.

use super::*;

impl<'a> BytecodeCompiler<'a> {
    /// `pkg::member` of a two-segment identifier naming a package enum
    /// member, from either enum table.
    pub(super) fn pkg_enum_const_of(&self, hier: &HierarchicalIdentifier) -> Option<Value> {
        if hier.root.is_some()
            || hier.path.len() != 2
            || hier.path.iter().any(|seg| !seg.selects.is_empty())
        {
            return None;
        }
        let key = format!("{}::{}", hier.path[0].name.name, hier.path[1].name.name);
        self.pkg_enum_consts
            .get(&key)
            .or_else(|| self.pkg_enum_consts_ref.and_then(|m| m.get(&key)))
            .map(|&(val, width)| Value::from_u64(val, width.max(1)))
    }

    pub(super) fn lookup_signal_id(&self, hier: &HierarchicalIdentifier) -> Option<usize> {
        // §26.3: package code's own parameter is a constant; its own (or
        // imported) variable is that package's signal.
        if let Some(key) = self.own_pkg_key(hier) {
            return self.signal_name_to_id.get(key.as_str()).copied();
        }
        let raw = Self::hier_raw_cow(hier);
        let raw: &str = &raw;
        // Targeted override for for-loop variables — see for_loop_var_ids
        // doc + compile_for's comment for the c910 motivation.
        if !self.for_loop_var_ids.is_empty() && hier.path.len() == 1 && !raw.contains('.') {
            if let Some(&id) = self.for_loop_var_ids.get(raw) {
                return Some(id);
            }
        }
        // Scope-first for SINGLE-SEGMENT bare names — LRM §22.4 / §23.6: a
        // local declaration shadows a same-named wildcard-imported member.
        // Without this, a module's local anon-enum FINISH=2 resolves to
        // pkg mult_state_e::FINISH=4 because the flat signal_name_to_id
        // registers BOTH `FINISH` (pkg) and `<scope>.FINISH` (local).
        // A parent-rooted ident (substituted expression port actual) is an
        // absolute name — the block's child scope hint must not qualify it.
        let rooted = hier.root.is_some();
        if !raw.contains('.') && !rooted {
            if let Some(scope) = &self.scope_hint {
                if let Some(id) = with_dotted(scope, raw, |k| {
                    let id = self.signal_name_to_id.get(k).copied();
                    if id.is_none() {
                        // The instance's own port net, left out as
                        // unobserved, would have been found right here.
                        self.signal_name_to_id.check_miss(k);
                    }
                    id
                }) {
                    return Some(id);
                }
            }
        }
        if let Some(&id) = self.signal_name_to_id.get(raw) {
            return Some(id);
        }
        if !rooted {
            if let Some(scope) = &self.scope_hint {
                if let Some(id) =
                    with_dotted(scope, raw, |k| self.signal_name_to_id.get(k).copied())
                {
                    return Some(id);
                }
            }
        }
        if hier.path.len() == 1 {
            let leaf = &hier.path[0].name.name;
            if let Some(&id) = self.signal_name_to_id.get(leaf.as_str()) {
                return Some(id);
            }
        }
        // Top-prefix strip: `<top>.<rest>` → `<rest>` for cross-hierarchical
        // refs whose absolute path was baked in by xezim's port-rewriting
        // (top-level instances have no prefix in signal_name_to_id).
        if let Some(top) = &self.top_module_name {
            if let Some(stripped) = raw
                .strip_prefix(top.as_str())
                .and_then(|rest| rest.strip_prefix('.'))
            {
                if let Some(&id) = self.signal_name_to_id.get(stripped) {
                    return Some(id);
                }
            }
        }
        self.signal_name_to_id.check_miss(raw);
        None
    }

    pub(super) fn lookup_signal_id_by_name(&self, name: &str) -> Option<usize> {
        self.signal_name_to_id.get(name).copied()
    }

    /// The parameter value behind a signal-table TWIN: the elaborator gives
    /// an instance parameter a placeholder signal so it is readable
    /// hierarchically, and the identifier path resolved `hier` to that
    /// signal `id`. Returns the parameter's value only when one of the
    /// exact keys (raw name, scope-qualified, bare single segment) names
    /// that very signal — never the leaf-suffix index, which can match a
    /// same-named parameter of another scope.
    pub(super) fn param_twin_value(
        &self,
        hier: &HierarchicalIdentifier,
        id: usize,
    ) -> Option<Value> {
        let params = self.params?;
        let raw = Self::hier_raw_cow(hier);
        // The exact keys, in order: scope-qualified, raw, bare single
        // segment. The first one naming signal `id` decides.
        let check = |k: &str| -> Option<Option<Value>> {
            (self.signal_name_to_id.get(k).copied() == Some(id))
                .then(|| params.get(k).filter(|v| !v.is_real).cloned())
        };
        // A parent-rooted ident (a substituted port actual) is absolute:
        // `lookup_signal_id` never qualifies it with the scope, and neither
        // does this check.
        if let (Some(scope), None) = (&self.scope_hint, &hier.root) {
            if let Some(r) = with_dotted(scope, &raw, check) {
                return r;
            }
        }
        if let Some(r) = check(&raw) {
            return r;
        }
        if hier.path.len() == 1 {
            if let Some(r) = check(&hier.path[0].name.name) {
                return r;
            }
        }
        None
    }

    pub(super) fn method_enum_constant(&self, hier: &HierarchicalIdentifier) -> Option<Value> {
        if !self.method_mode
            || hier.root.is_some()
            || hier.path.len() != 1
            || !hier.path[0].selects.is_empty()
        {
            return None;
        }
        let name = hier.path[0].name.name.as_str();
        if self.local_var_regs.contains_key(name) || self.local_array_regs.contains_key(name) {
            return None;
        }
        self.method_enum_constants.get(name).cloned()
    }

    /// Method mode: `C::M` naming an enum literal of class `C` — parsed as
    /// a member access on the bare name `C`, which no local, member or
    /// signal of that name may claim.
    pub(super) fn class_scoped_enum(&self, base: &Expression, member: &str) -> Option<Value> {
        if !self.method_mode {
            return None;
        }
        let ExprKind::Ident(h) = &base.kind else {
            return None;
        };
        if h.root.is_some() || h.path.len() != 1 || !h.path[0].selects.is_empty() {
            return None;
        }
        let class = h.path[0].name.name.as_str();
        if self.local_var_regs.contains_key(class)
            || self.method_handle_names.contains(class)
            || self.bare_member_names.contains(class)
            || self.string_member_names.contains(class)
            || self.lookup_signal_id(h).is_some()
        {
            return None;
        }
        self.class_scoped_enums
            .as_ref()?
            .get(&format!("{class}::{member}"))
            .cloned()
    }

    /// §26.3: a bare name in a body declared in package P that neither the
    /// frame nor the class declares names P's own parameter or variable,
    /// else one P imports (an explicit import, or the one wildcard import
    /// that supplies it), ahead of a same-named one of the design (the
    /// interpreter's `resolve_hier_name` order): its qualified key. Inside
    /// an inlined callee P is the callee's package, and a callee declared
    /// outside a package resolves as before.
    fn own_pkg_key(&self, hier: &HierarchicalIdentifier) -> Option<String> {
        let (method_pkg, params) = self.method_pkg.as_ref()?;
        let pkg = match self.inline_pkg_stack.last() {
            Some(callee_pkg) => callee_pkg.as_deref()?,
            None => method_pkg.as_str(),
        };
        let [seg] = hier.path.as_slice() else {
            return None;
        };
        let n = seg.name.name.as_str();
        if hier.root.is_some()
            || n.contains("::")
            || self.local_var_regs.contains_key(n)
            || self.local_array_regs.contains_key(n)
            || self.local_coll_names.contains(n)
            || self.local_const_vars.contains_key(n)
            || self.method_bound_names.contains(n)
            || self.class_shadow_names.contains(n)
            || self.bare_member_names.contains(n)
            || self.string_member_names.contains(n)
            || self.class_const_names.contains(n)
        {
            return None;
        }
        let has = |q: &str| params.contains_key(q) || self.signal_name_to_id.contains_key(q);
        // Only where the bare name names something else: an uncontested
        // package parameter is also registered bare with its own value, and
        // that spelling keeps its packed shape (`C[i]` of a `[3:0][7:0]`
        // parameter selects an element).
        let contested = |q: String| -> Option<String> {
            match params.get(q.as_str()) {
                Some(v) => match params.get(n) {
                    Some(bare) => (bare != v).then_some(q),
                    // No bare parameter: only a same-named signal contests.
                    None => self.signal_name_to_id.contains_key(n).then_some(q),
                },
                // A package variable keeps its `pkg::name` spelling only
                // when another declaration took the bare name.
                None => Some(q),
            }
        };
        let key = format!("{pkg}::{n}");
        if has(&key) {
            return contested(key);
        }
        // §26.3: a name `pkg` imports (`import gp::*;` inside `pkg`).
        let imports = self.method_pkg_imports?.get(pkg)?;
        let mut wildcard: Option<String> = None;
        for (src, item) in imports {
            let q = format!("{src}::{n}");
            match item.as_deref() {
                Some(leaf) if leaf == n && has(&q) => return contested(q),
                None if has(&q) => {
                    // Two wildcard imports supplying it: no guess.
                    if wildcard.is_some() {
                        return None;
                    }
                    wildcard = Some(q);
                }
                _ => {}
            }
        }
        wildcard.and_then(contested)
    }

    /// §26.3: the package that declares the subroutine `name` being inlined
    /// into package code (`None` outside package code, or for a subroutine
    /// declared outside a package): `P` of a `P::name` call, else the package
    /// whose `P::name` entry is this very declaration.
    pub(super) fn inlined_callee_pkg(
        &self,
        name: &str,
        span: crate::ast::Span,
        task: bool,
    ) -> Option<String> {
        self.method_pkg.as_ref()?;
        if let Some((pkg, _)) = name.rsplit_once("::") {
            return Some(pkg.to_string());
        }
        let suffix = format!("::{name}");
        let owner = |k: &String| k.strip_suffix(suffix.as_str()).map(str::to_string);
        if task {
            let t = self.tasks?;
            t.iter()
                .find(|(k, td)| k.ends_with(suffix.as_str()) && td.span == span)
                .and_then(|(k, _)| owner(k))
        } else {
            let f = self.functions?;
            f.iter()
                .find(|(k, fd)| k.ends_with(suffix.as_str()) && fd.span == span)
                .and_then(|(k, _)| owner(k))
        }
    }

    /// `own_pkg_key` when it names a parameter: its key and value.
    fn own_pkg_param(&self, hier: &HierarchicalIdentifier) -> Option<(String, &'a Value)> {
        let key = self.own_pkg_key(hier)?;
        let (_, params) = self.method_pkg.as_ref()?;
        let value = params.get(key.as_str())?;
        Some((key, value))
    }

    /// `e` with its root spelled `P::name` when `own_pkg_key` answers it, so
    /// a select or member of the parameter takes the qualified name's shape.
    pub(super) fn own_pkg_qualified(&self, e: &Expression) -> Option<Expression> {
        self.method_pkg.as_ref()?;
        let mut root = e;
        while let ExprKind::Index { expr, .. }
        | ExprKind::RangeSelect { expr, .. }
        | ExprKind::MemberAccess { expr, .. } = &root.kind
        {
            root = &**expr;
        }
        let ExprKind::Ident(h) = &root.kind else {
            return None;
        };
        let key = self.own_pkg_key(h)?;
        let mut q = e.clone();
        let mut root = &mut q;
        loop {
            root = match &mut root.kind {
                ExprKind::Index { expr, .. }
                | ExprKind::RangeSelect { expr, .. }
                | ExprKind::MemberAccess { expr, .. } => &mut **expr,
                ExprKind::Ident(h) => {
                    h.path[0].name.name = key;
                    h.cached_signal_id = crate::ast::cache::SignalCache::new(None);
                    h.cached_resolved_name = crate::ast::cache::NameCache::new();
                    h.bound = crate::ast::binding::Bound::UNSET;
                    return Some(q);
                }
                _ => return None,
            };
        }
    }

    pub(super) fn lookup_param_value(&self, hier: &HierarchicalIdentifier) -> Option<Value> {
        // §23.9: a loop-local index hides a module parameter of the same name.
        // Only the typed loop binding may supply its value during unrolling.
        if hier.root.is_none()
            && let [seg] = hier.path.as_slice()
            && self.local_const_vars.contains_key(&seg.name.name)
        {
            return None;
        }
        if let Some(value) = self.method_enum_constant(hier) {
            return Some(value);
        }
        if let Some((_, value)) = self.own_pkg_param(hier)
            && hier.path[0].selects.is_empty()
        {
            return Some(value.clone());
        }
        let params = self.params?;
        let raw = Self::hier_raw_cow(hier);
        let raw: &str = &raw;
        if let Some(v) = params.get(raw) {
            return Some(v.clone());
        }
        // §26.3: in the body of an inlined `pkg::f`, a bare name with no bare
        // parameter entry is `pkg`'s own parameter.
        if hier.root.is_none()
            && let [seg] = hier.path.as_slice()
            && seg.selects.is_empty()
            && let Some((pkg, _)) = self.inlining_stack.last().and_then(|n| n.rsplit_once("::"))
            && let Some(v) = params.get(format!("{pkg}::{}", seg.name.name).as_str())
        {
            return Some(v.clone());
        }
        // §26.3: `pkg::P` (a two-segment path) is the package's own
        // parameter, keyed `pkg::P`; the suffix match below would take any
        // bare `P` — another package's, imported here.
        if hier.root.is_none()
            && hier.path.len() == 2
            && hier.path.iter().all(|s| s.selects.is_empty())
        {
            let q = format!("{}::{}", hier.path[0].name.name, hier.path[1].name.name);
            if let Some(v) = params.get(q.as_str()) {
                return Some(v.clone());
            }
            // §26.3: a package ENUM member has no parameter entry under its
            // qualified spelling; take it from the enum table before any
            // bare fallback can pick another package's member.
            if let Some(v) = self.pkg_enum_const_of(hier) {
                return Some(v);
            }
        }
        if let Some(scope) = &self.scope_hint {
            if let Some(v) = with_dotted(scope, raw, |k| params.get(k).cloned()) {
                return Some(v);
            }
        }
        if hier.path.len() == 1 {
            if let Some(v) = params.get(&hier.path[0].name.name) {
                return Some(v.clone());
            }
        }
        // Suffix-match: bare `CARRY_CHAIN` may be stored as
        // `top.uut.picorv32_core.pcpi_mul.CARRY_CHAIN`. Only accept if a
        // single param key matches — multiple matches are ambiguous.
        //
        // Any match in either direction shares the LAST dotted segment with
        // `raw`, so the leaf index narrows the scan to same-leaf keys.
        let is_match = |name: &str| -> bool {
            let raw_has_key_suffix = raw.len() >= name.len()
                && raw.ends_with(name)
                && (raw.len() == name.len() || raw.as_bytes()[raw.len() - name.len() - 1] == b'.');
            let key_has_raw_suffix = name.len() >= raw.len()
                && name.ends_with(raw)
                && (name.len() == raw.len() || name.as_bytes()[name.len() - raw.len() - 1] == b'.');
            raw_has_key_suffix || key_has_raw_suffix
        };
        let mut found: Option<&Value> = None;
        if let Some(idx) = self.param_leaf_idx {
            let leaf = raw.rsplit('.').next().unwrap_or(raw);
            for name in idx.get(leaf).map(|v| v.as_slice()).unwrap_or(&[]) {
                if is_match(name) {
                    if found.is_some() {
                        return None;
                    }
                    found = params.get(name);
                }
            }
        } else {
            for (name, value) in params {
                if is_match(name) {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(value);
                }
            }
        }
        found.cloned()
    }

    pub(super) fn expr_to_signal_id(&self, expr: &Expression) -> Option<usize> {
        match &expr.kind {
            ExprKind::Ident(hier) => {
                // class-perf method mode, §8.10 scope order: a bare Ident in
                // a method body resolves in the CLASS scope first — it must
                // never name a module signal behind the method's back (a
                // `c++` on a class member once silently incremented a
                // same-named module variable instead). Class-scope names
                // take their own Load/StoreClassMember paths; shadow names
                // without a loadable path keep the all-or-nothing bail.
                if self.method_mode
                    && hier.root.is_none()
                    && hier.path.len() == 1
                    && hier.path[0].selects.is_empty()
                    && self
                        .class_shadow_names
                        .contains(hier.path[0].name.name.as_str())
                {
                    return None;
                }
                self.lookup_signal_id(hier)
            }
            ExprKind::Paren(inner) => self.expr_to_signal_id(inner),
            _ => None,
        }
    }
}
