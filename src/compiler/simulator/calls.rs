//! Simulator calls.

use super::*;

impl Simulator {
    pub(super) fn eval_call(&mut self, func: &Expression, args: &[Expression]) -> Value {
        // For a `C#(params)::method()` static call, make the specialization
        // active across the whole call (so the method body's static accesses
        // key per-spec) — a wrapper here covers all of eval_call_inner's early
        // returns. No spec → call inner directly (zero overhead).
        if !Self::expr_has_specialization(func) {
            return self.eval_call_inner(func, args);
        }
        // Keep `current_spec` set while resolving the call's specialization so
        // `resolve_call_spec_params` can resolve class-local typedef args
        // (e.g. `special_comp_type`) through the enclosing spec —
        // `resolve_typedef_spec` reads `self.current_spec`.
        let saved = self.current_spec.clone();
        let lexical = self.lexical_spec();
        let extracted = self.resolve_call_spec_params(Self::extract_call_spec(func), &lexical);
        if let Some((ref base, ref sig)) = extracted {
            self.ensure_spec_statics(base, sig);
        }
        // Execute the callee with `current_spec` = the CALLER'S spec so an
        // actual arg that is an enclosing specialization's value parameter
        // resolves in the caller's scope; the call's own specialization stays
        // on the scope stack for the body/method-dispatch to key per-spec.
        // (Innermost caller last — value-param resolution walks inward.)
        if let Some(cs) = &saved {
            self.spec_scope_stack.push(cs.clone());
        }
        let pushed = saved.is_some();
        self.current_spec = extracted.clone().or(saved.clone());
        let r = self.eval_call_inner(func, args);
        self.current_spec = saved;
        if pushed {
            self.spec_scope_stack.pop();
        }
        r
    }

    /// Does the RECEIVER of this member call resolve to a live object whose
    /// class chain implements `mname` with a real body? Used to gate the
    /// pure-SV factory/sequencer bridges below: they predate real UVM
    /// execution and must not shadow an actual implementation.
    pub(super) fn receiver_implements_method(&mut self, recv: &Expression, mname: &str) -> bool {
        let h = self
            .eval_handle_expr(recv)
            .or_else(|| self.eval_expr(recv).to_u64().map(|v| v as usize))
            .unwrap_or(0);
        if h == 0 {
            return false;
        }
        let Some(cn) = self
            .heap
            .get(h)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone())
        else {
            return false;
        };
        self.class_has_method(&cn, mname)
    }

    /// §26.3 / §35.5.4: preserve a package import's declaring scope while
    /// ordinary objects and local declarations retain their receiver meaning.
    pub(super) fn package_dpi_call_key(&self, func: &Expression) -> Option<String> {
        let package_dpi = match &func.kind {
            ExprKind::MemberAccess { expr, member } => match &expr.kind {
                ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() =>
                    Some((h.path[0].name.name.as_str(), member.name.as_str())),
                _ => None,
            },
            ExprKind::Ident(h) if h.path.len() == 2
                && h.path.iter().all(|s| s.selects.is_empty()) =>
                Some((h.path[0].name.name.as_str(), h.path[1].name.name.as_str())),
            _ => None,
        };
        if let Some((pkg, name)) = package_dpi
            && self.module.packages.contains(pkg)
            && !self.this_prop_hides(pkg)
            && !self.local_stack.last().is_some_and(|m| m.contains_key(pkg))
            && !self.signal_name_to_id.contains_key(pkg)
        {
            return Some(format!("{pkg}::{name}"));
        }
        None
    }

    pub(super) fn eval_call_inner(&mut self, func: &Expression, args: &[Expression]) -> Value {
        if let Some(key) = self.package_dpi_call_key(func) {
            if let Some(value) = self.exec_dpi_import_call(&key, args) {
                return value;
            }
        }
        // §7.5.2, §7.9.1, §7.10.2: a collection method on a function's
        // returned queue, dynamic or associative array (`f().size()`).
        if let ExprKind::MemberAccess { expr: recv, member } = &func.kind
            && let ExprKind::Call {
                func: rf,
                args: rargs,
            } = &recv.kind
            && matches!(
                member.name.as_str(),
                "size"
                    | "num"
                    | "sum"
                    | "product"
                    | "and"
                    | "or"
                    | "xor"
                    | "exists"
                    | "first"
                    | "last"
                    | "min"
                    | "max"
            )
            && self.call_returns_collection(rf)
        {
            self.pending_ret_collection = None;
            let _ = self.eval_call(rf, rargs);
            if let Some(src) = self.take_pending_ret_collection()
                && let Some(v) = self.eval_builtin_method(&src, &member.name, args)
            {
                return v;
            }
        }
        // Normalize away Specialization wrappers for dispatch shape-matching
        // (the `#(...)` text is retained in the original AST + `current_spec`
        // for per-spec static keying). Keeps pre-Specialization dispatch exact.
        let func_norm;
        let func = if Self::expr_has_specialization(func) {
            func_norm = Self::strip_spec_shape(func);
            &func_norm
        } else {
            func
        };
        // §26.3: a method call on a PACKAGE-scoped variable — `P::c.f1(x)`
        // flattens to Ident([P, c, f1]) — dispatches exactly like the
        // unqualified `c.f1(x)`: the package prefix contributes nothing at
        // runtime (the variable's storage is registered under its bare name).
        // Strip it and re-enter, so every receiver shape (class handle, enum,
        // collection) resolves through the ordinary paths. Only a REGISTERED
        // package name is stripped, and only when the remaining path still
        // has a receiver + method.
        if let ExprKind::Ident(hier) = &func.kind {
            if hier.path.len() >= 3
                && hier.path[0].selects.is_empty()
                && self.module.packages.contains(&hier.path[0].name.name)
            {
                let mut stripped = hier.clone();
                stripped.path.remove(0);
                stripped.cached_signal_id = std::cell::Cell::new(None);
                let f = Expression::new(ExprKind::Ident(stripped), func.span);
                return self.eval_call_inner(&f, args);
            }
        }
        // §23.6/§27.6: a subroutine of an instance inside a generate scope or
        // an instance array — ,
        // — is registered under its evaluated path. Such a callee parses as
        // one Ident with selects, which the receiver arms below took for an
        // object handle: a function call read 0 and a task call was dropped.
        let selected_path_callee = match &func.kind {
            ExprKind::Ident(hier) => (hier.path.len() >= 2
                && hier.path.iter().any(|s| !s.selects.is_empty()))
            .then(|| hier.path.last().unwrap().name.name.as_str()),
            ExprKind::MemberAccess { expr: recv, member } => {
                Self::member_chain_has_index(recv).then_some(member.name.as_str())
            }
            _ => None,
        };
        if let Some(m) = selected_path_callee {
            if self.class_member_names().subroutine_suffixes.contains(m) {
                if let Some(full) = self
                    .hier_segments(func)
                    .and_then(|segs| self.scoped_subroutine_key(&segs.join(".")))
                {
                    return self.call_scoped_subroutine(&full, args);
                }
            }
        }
        // §8.25.3: `pkg::Class#(args)::member::method(...)` — the `#(args)`
        // specialization mid-chain breaks the parser's flat-Ident flattening
        // of `::` scope chains, so the callee arrives as a nested MemberAccess
        // over `Ident([pkg, Class])`. Collapse that shape to the flat Ident
        // (`#(...)` is already active as `current_spec` from eval_call) so the
        // package-qualified static-call dispatch — which keys on flattened
        // paths — sees it. Only when the first segment names a package or
        // class (not a local/signal), so plain dotted receivers are untouched.
        if let ExprKind::MemberAccess { expr, member } = &func.kind {
            if let Some(flat) = Self::flatten_scoped_member_chain(expr, member) {
                if let ExprKind::Ident(fh) = &flat.kind {
                    let n0 = fh.path[0].name.name.as_str();
                    // A middle segment that is a STATIC COLLECTION member of
                    // the named class (e.g. `Class#(T)::m_coll.exists(k)` —
                    // `m_coll` is data, not a scope) means this is a builtin
                    // call on an explicitly-parameterized static-collection
                    // RECEIVER. Flattening it to a scope chain loses the
                    // receiver evaluation and the per-spec static key; the
                    // MemberAccess-receiver builtin path (expr_assoc_name's
                    // current_spec arm) handles it correctly.
                    let cls_for_members = if self.module.classes.contains_key(n0) {
                        n0
                    } else if fh.path.len() > 2 {
                        fh.path[1].name.name.as_str()
                    } else {
                        n0
                    };
                    let mid_is_scope_chain = fh.path[1..fh.path.len() - 1].iter().all(|s| {
                        !self.member_is_static_coll(cls_for_members, &s.name.name)
                            && !self.is_associative_array(&s.name.name)
                    });
                    if !self.local_stack.last().is_some_and(|m| m.contains_key(n0))
                        && !self.signal_name_to_id.contains_key(n0)
                        && (self.module.packages.contains(n0)
                            || self.module.classes.contains_key(n0))
                        && mid_is_scope_chain
                    {
                        return self.eval_call_inner(&flat, args);
                    }
                }
            }
        }
        // A subroutine declared in a named generate-for scope is addressed as
        // `block[index].task(...)`. Elaboration stores each iteration under
        // that exact hierarchical key.
        if let ExprKind::MemberAccess { expr: base, member } = &func.kind {
            if let ExprKind::Index { expr: scope, index } = &base.kind {
                if let ExprKind::Ident(h) = &scope.kind {
                    if h.path.len() == 1 && h.path[0].selects.is_empty() {
                        if let Some(idx) = self.eval_scalar_self(index) {
                            let name = format!("{}[{}].{}", h.path[0].name.name, idx, member.name);
                            if let Some(fd) = self.fn_decl_rc(&name) {
                                return self.exec_function_call(&fd, args);
                            }
                            if let Some(td) = self.task_decl_rc(&name) {
                                self.exec_task_call(&td, args);
                                return Value::zero(32);
                            }
                        }
                    }
                }
            }
        }
        // §25.7/§7.10: a call whose callee is a plain dotted MemberAccess
        // chain — an INTERFACE subroutine (`bi.dbl(4)`) or a collection
        // builtin on an interface/instance member (`bi.q.push_back(7)`) —
        // exactly as written inside a subroutine body. At module scope the
        // same source arrives as one flat Ident (elaboration rewrites it) and
        // the flat dispatch joins the names, so it worked; the MemberAccess
        // spelling had no handler and silently evaluated to 0 / dropped the
        // mutation.
        if let ExprKind::MemberAccess { expr: recv, member } = &func.kind {
            let mut joined = String::new();
            if let Some(nsegs) = Self::push_member_path(recv, &mut joined) {
                let recv_len = joined.len();
                if nsegs > 0 {
                    joined.push('.');
                }
                joined.push_str(&member.name);
                // Every subroutine key below is `joined` or ends in
                // `.joined`.
                let may_be_subroutine = self
                    .class_member_names()
                    .subroutine_suffixes
                    .contains(joined.as_str());
                // Interface (or generate-scope) subroutine under its
                // hierarchical key.
                if may_be_subroutine {
                    if let Some(fd) = self.fn_decl_rc(&joined) {
                        // §23.4 lexical home: park the callee's scope (the
                        // join minus the trailing member) so a bare `-> e`
                        // inside fires the CALLEE's own event — same
                        // discipline as the flat-Ident dispatch arm below.
                        return self.exec_scoped_fn_call(&fd, args, &joined);
                    }
                    if let Some(td) = self.task_decl_rc(&joined) {
                        let scope = joined.rsplit_once('.').map(|(s, _)| s.to_string());
                        let saved = self.name_resolve_hint.borrow().clone();
                        if let Some(sc) = &scope {
                            *self.name_resolve_hint.borrow_mut() = Some(sc.clone());
                        }
                        let saved_ts = self.timescale_scope_override.take();
                        if let Some(_sc) = &scope {
                            self.timescale_scope_override = scope.clone();
                        }
                        self.exec_task_call(&td, args);
                        self.timescale_scope_override = saved_ts;
                        *self.name_resolve_hint.borrow_mut() = saved;
                        return Value::zero(32);
                    }
                }
                // The same key under the resolution hint and its parents:
                // `core.get_seq()` from a class method of an object built in
                // `u_w` is `u_w.core.get_seq` (issue #155). Class bodies are
                // not instance-rewritten, so the bare join misses.
                if may_be_subroutine {
                    let hint = self.name_resolve_hint.borrow().clone();
                    if let Some(hint) = hint.as_deref() {
                        let mut scope = hint;
                        loop {
                            let scoped = format!("{}.{}", scope, joined);
                            if let Some(fd) = self.fn_decl_rc(&scoped) {
                                return self.exec_scoped_fn_call(&fd, args, &scoped);
                            }
                            if let Some(td) = self.task_decl_rc(&scoped) {
                                let saved = self.name_resolve_hint.borrow().clone();
                                *self.name_resolve_hint.borrow_mut() = Some(scope.to_string());
                                let saved_ts = self.timescale_scope_override.take();
                                self.timescale_scope_override = Some(scope.to_string());
                                self.exec_task_call(&td, args);
                                self.timescale_scope_override = saved_ts;
                                *self.name_resolve_hint.borrow_mut() = saved;
                                return Value::zero(32);
                            }
                            match scope.rsplit_once('.') {
                                Some((p, _)) => scope = p,
                                None => break,
                            }
                        }
                    }
                }
                // §23.6/§23.8: a FULL hierarchical path rooted at the top
                // module (`tb.u.f()`, `$root.tb.u.f()`) written in a class
                // method. Instance subroutines are registered without the top
                // name, so neither join above matched and the call read 0 /
                // an empty string (issue #320).
                if joined.starts_with("$root.")
                    || joined
                        .strip_prefix(self.module.name.as_str())
                        .is_some_and(|r| r.starts_with('.'))
                {
                    if let Some(full) = self.scoped_subroutine_key(&joined) {
                        return self.call_scoped_subroutine(&full, args);
                    }
                }
                // Collection builtin on the flattened receiver name (the
                // receiver's segments plus the member are always two or more).
                {
                    let mut recv_name = joined[..recv_len].to_string();
                    // A class-qualified STATIC collection receiver
                    // (`ClassName::staticColl.push_back(x)` / `.size()`)
                    // flattens to the DOT form `ClassName.staticColl`, which
                    // never matches the dynamic-array/assoc membership tests
                    // below (those see the bare registered name), so a
                    // `push_back`/`size` reached from a non-instance context
                    // (a static fn / module static init — e.g. `m_do_cl_init`
                    // pushing `+uvm_set_severity`) was silently dropped.
                    // Rewrite the receiver: a SIBLING-COLLIDING collection
                    // stores per-DECLARING class (`ClassName::coll`, matching
                    // `spec_static_coll_key`); a non-colliding one keeps the
                    // bare name its in-class accessors and store use.
                    if let Some((cls, coll)) = recv_name.split_once('.') {
                        if self.module.classes.contains_key(cls)
                            && self.member_is_static_coll(cls, coll)
                        {
                            recv_name = if self.static_coll_name_collides(coll) {
                                self.static_prop_key(cls, coll)
                                    .unwrap_or_else(|| coll.to_string())
                            } else if self.class_is_parameterized(cls) {
                                // PARAMETERIZED class: statics live
                                // per-specialization, resolved from the
                                // qualified form via `current_spec` (§8.25);
                                // collapsing to the bare name would drop the
                                // specialization and miss the store (e.g.
                                // `config_db#(int)::m_rsc` flattens to
                                // `config_db.m_rsc`, `#(int)` stripped).
                                recv_name
                            } else {
                                coll.to_string()
                            };
                        }
                    }
                    if (self.module.dynamic_arrays.contains(&recv_name)
                        || self.module.associative_arrays.contains_key(&recv_name)
                        || self.module.arrays.contains_key(&recv_name))
                        // §8.4/§23.9: in a method, a property of `this`
                        // (`s.q` of a struct property `s`) hides a
                        // same-named module variable.
                        && !self.this_prop_hides(
                            recv_name.split(['.', '[']).next().unwrap_or(&recv_name),
                        )
                    {
                        if let Some(res) = self.eval_builtin_method(&recv_name, &member.name, args)
                        {
                            return res;
                        }
                    }
                }
            }
        }
        // Intercept framework method calls.
        if let ExprKind::MemberAccess { expr, member } = &func.kind {
            let mname = member.name.as_str();

            // Package-qualified static method call: `pkg::Class::method(args)`
            // parses as MemberAccess { MemberAccess { Ident(pkg), Class }, method }.
            // Without this, the inner MemberAccess evaluates `pkg` as a local/signal
            // (fails), and the outer call never dispatches (e.g. fully-qualified
            // package constructors returning null objects).
            if let ExprKind::MemberAccess {
                expr: inner,
                member: class_id,
            } = &expr.kind
            {
                if let ExprKind::Ident(inner_hier) = &inner.kind {
                    if inner_hier.path.len() == 1 {
                        let pkg = &inner_hier.path[0].name.name;
                        let cls = class_id.name.clone();
                        if !self
                            .local_stack
                            .last()
                            .is_some_and(|m| m.contains_key(pkg.as_str()))
                            && !self.signal_name_to_id.contains_key(pkg.as_str())
                            && self.module.classes.contains_key(&cls)
                        {
                            if let Some(res) = self.exec_static_method(&cls, mname, args) {
                                return res;
                            }
                            if mname == "new" {
                                if let Some(cd) = self.module.classes.get(&cls).cloned() {
                                    return self.instantiate_class(&cd, args);
                                }
                            }
                        }
                        // §8.23 / §6.18: `Class::typedef_alias::method(args)` —
                        // the middle segment is a TYPEDEF declared inside
                        // `Class` (e.g. `typedef registry#(...) type_id;` or a plain
                        // `typedef Holder type_id;`), not a class itself. Resolve
                        // the alias to its target class and dispatch the static
                        // method there. Without this, `base_class::type_id::get()`
                        // (and `type_id::create`) never reached the registry's `get()`,
                        // so factory singleton lookups returned null and identity checks failed.
                        else if !self
                            .local_stack
                            .last()
                            .is_some_and(|m| m.contains_key(pkg.as_str()))
                            && !self.signal_name_to_id.contains_key(pkg.as_str())
                            && self.module.classes.contains_key(pkg.as_str())
                        {
                            // Non-parameterized class-scoped typedef alias.
                            if let Some(resolved) =
                                self.resolve_class_member_typedef_class(pkg, &cls)
                            {
                                if let Some(res) = self.exec_static_method(&resolved, mname, args) {
                                    return res;
                                }
                                if mname == "new" {
                                    if let Some(cd) = self.module.classes.get(&resolved).cloned() {
                                        return self.instantiate_class(&cd, args);
                                    }
                                }
                            }
                            // Parameterized class-scoped typedef alias — e.g.
                            // `typedef object_registry#(T,"N") type_id;` inside
                            // class T, called as `T::type_id::get()`. The broken
                            // generic fallback lost the return value for class-typed
                            // returns, so the factory's singleton identity check
                            // failed. Dispatch on the resolved specialization with
                            // its statics materialised.
                            if let Some((base, sig)) =
                                self.resolve_class_member_typedef_spec(pkg, &cls)
                            {
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
                                    return v;
                                }
                            }
                        }
                        // §6.18/§8.23: `Alias::typedef::method(args)` where the
                        // FIRST segment `pkg` is a MODULE-LEVEL TYPEDEF ALIAS to a
                        // class (`typedef base_class#(9) BaseType;` or a chain of
                        // plain class aliases),
                        // and `cls` is a member typedef of that class
                        // (`BaseType::type_id::get()` where `base_class` declares
                        // `typedef uvm_object_registry#(base_class#(P)) type_id;`).
                        // The existing branches only handle `pkg` being a package or
                        // a real class name, so this 3-segment path never reached
                        // the registry's `get()` and returned null (factory/registry
                        // parameterized-object tests: type_id identity and factory
                        // registration checks). Resolve `pkg` to its class spec, set
                        // the active specialization so the member typedef's type args
                        // bind their params, resolve the member typedef, and dispatch.
                        else if !self
                            .local_stack
                            .last()
                            .is_some_and(|m| m.contains_key(pkg.as_str()))
                            && !self.signal_name_to_id.contains_key(pkg.as_str())
                        {
                            if let Some((alias_base, alias_sig)) =
                                self.resolve_typedef_spec(pkg).or_else(|| {
                                    let hop = self.resolve_plain_alias_chain(pkg)?;
                                    self.resolve_typedef_spec(&hop)
                                        .or_else(|| Some((hop, String::new())))
                                })
                                && self.module.classes.contains_key(&alias_base)
                            {
                                // Resolve the member typedef within the alias's
                                // specialization. Set current_spec so type args that
                                // reference the class's own params resolve to the
                                // alias's concrete values.
                                let saved = self.current_spec.take();
                                self.current_spec = Some((alias_base.clone(), alias_sig.clone()));
                                let member =
                                    self.resolve_class_member_typedef_spec(&alias_base, &cls);
                                let mem_class_only =
                                    self.resolve_class_member_typedef_class(&alias_base, &cls);
                                self.current_spec = saved;
                                let target: Option<(String, String)> = match member {
                                    Some(m) => Some(m),
                                    None => mem_class_only.map(|c| (c, String::new())),
                                };
                                if let Some((target_base, target_sig)) = target {
                                    if !target_sig.is_empty() {
                                        self.ensure_spec_statics(&target_base, &target_sig);
                                    }
                                    let saved_spec = self.current_spec.take();
                                    if !target_sig.is_empty() {
                                        if let Some(cs) = &saved_spec {
                                            self.spec_scope_stack.push(cs.clone());
                                        }
                                        self.current_spec =
                                            Some((target_base.clone(), target_sig.clone()));
                                    }
                                    let res = self.exec_static_method(&target_base, mname, args);
                                    let had_spec = saved_spec.is_some() && !target_sig.is_empty();
                                    self.current_spec = saved_spec;
                                    if had_spec {
                                        self.spec_scope_stack.pop();
                                    }
                                    if let Some(v) = res {
                                        return v;
                                    }
                                    if mname == "new" && target_sig.is_empty() {
                                        if let Some(cd) =
                                            self.module.classes.get(&target_base).cloned()
                                        {
                                            return self.instantiate_class(&cd, args);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // IEEE 1800-2017 §8.15: `super.m(...)` binds STATICALLY to the
            // method visible in the parent of the class LEXICALLY containing
            // the call — never through virtual dispatch on the object's
            // dynamic type. This must run BEFORE every method-name-keyed
            // interceptor below (mailbox `get`/`put`, analysis-port `write`,
            // collection builtins, ...), otherwise a user method whose name
            // collides with a builtin (e.g. `super.get()`) is swallowed by
            // the builtin path instead of running the base-class body.
            // Guarded on the parent chain actually defining the method (or
            // `new`, whose ctor-chaining path exec_super_method_call already
            // owns) so `super.randomize()`-style builtin calls keep their
            // legacy routing.
            if let ExprKind::Ident(hier) = &expr.kind {
                if hier.path.len() == 1 && hier.path[0].name.name == "super" {
                    if let Some(handle) = self.this_stack.last().copied().flatten() {
                        let routed = mname == "new"
                            || self
                                .super_call_parent()
                                .is_some_and(|p| self.class_has_method(&p, mname));
                        if routed {
                            return self.exec_super_method_call(handle, mname, args);
                        }
                    }
                }
            }

            // §18.13 rand_mode / constraint_mode
            // Receiver forms:
            //   obj.x.rand_mode(0/1)        — disable/enable rand var x
            //   obj.cb.constraint_mode(0/1) — disable/enable constraint cb
            //   obj.rand_mode(0/1)          — apply to ALL rand props
            //   obj.constraint_mode(0/1)    — apply to ALL constraints
            // 0-arg query form (`var.rand_mode()`) returns 1 if enabled, 0
            // if disabled.
            if matches!(mname, "rand_mode" | "constraint_mode") {
                // Receiver can be `obj` (Ident) or `obj.member` (MemberAccess).
                let (handle_opt, target_name): (Option<usize>, Option<String>) = match &expr.kind {
                    ExprKind::Ident(h) if h.path.len() == 1 => {
                        // Single-name receiver: could be the object itself,
                        // OR an `this`-local class property — only the
                        // former applies modes globally to the object.
                        let name = &h.path[0].name.name;
                        let this_h = self.this_stack.last().copied().flatten();
                        // §8.11/§18.8: a bare name that DECLARES a non-class
                        // property of `this` (and no local shadows it) names
                        // that random variable, whatever value it holds. Its
                        // value used to be tried as a handle first, so an
                        // `id` holding 1 switched off object #1 instead.
                        let this_scalar_prop = this_h.is_some_and(|th| {
                            !self
                                .local_stack
                                .last()
                                .is_some_and(|f| f.contains_key(name))
                                && self.object_declares_property(th, name)
                                && self.prop_class_type(th, name).is_none()
                        });
                        let h_val = if this_scalar_prop {
                            0
                        } else {
                            self.eval_expr(expr).to_u64().unwrap_or(0) as usize
                        };
                        if h_val != 0 && self.heap.get(h_val).and_then(|x| x.as_ref()).is_some() {
                            (Some(h_val), None)
                        } else {
                            // Treat as `this.<name>`, i.e. specific rand prop
                            // or constraint name on the current this.
                            (this_h, Some(name.clone()))
                        }
                    }
                    ExprKind::MemberAccess {
                        expr: outer,
                        member: inner_m,
                    } => {
                        let h_val = self.eval_expr(outer).to_u64().unwrap_or(0) as usize;
                        if h_val != 0 {
                            (Some(h_val), Some(inner_m.name.clone()))
                        } else {
                            (None, None)
                        }
                    }
                    _ => (None, None),
                };
                if let Some(h) = handle_opt {
                    // §18.5.11: a static constraint block's mode is class-wide.
                    if mname == "constraint_mode" {
                        let en = if args.is_empty() {
                            None
                        } else {
                            Some(self.eval_expr(&args[0]).to_u64().unwrap_or(0) != 0)
                        };
                        if let Some(v) = self.static_constraint_mode(h, target_name.as_deref(), en)
                        {
                            return v;
                        }
                    }
                    let enable = if args.is_empty() {
                        // Query form: return current state.
                        let table = if mname == "rand_mode" {
                            &self.rand_mode_disabled
                        } else {
                            &self.constraint_mode_disabled
                        };
                        let disabled = table.get(&h).is_some_and(|s| {
                            if let Some(ref n) = target_name {
                                s.contains(n)
                            } else {
                                !s.is_empty()
                            }
                        });
                        return Value::from_u64(if disabled { 0 } else { 1 }, 32);
                    } else {
                        self.eval_expr(&args[0]).to_u64().unwrap_or(0) != 0
                    };
                    self.set_object_random_mode(h, mname == "rand_mode", target_name.as_deref(), enable);
                    return Value::zero(32);
                }
            }

            // `std::randomize(vars [with {...}])` — SV scope randomization (the
            // `std::` static scope). Assign each listed variable a random value
            // (best-effort; inline `with` constraints are not solved). Returns 1
            // so DV_CHECK_STD_RANDOMIZE_FATAL proceeds.
            if mname == "randomize" {
                if let ExprKind::Ident(h) = &expr.kind {
                    if h.path.len() == 1 && h.path[0].name.name == "std" {
                        return self.exec_std_randomize(args);
                    }
                }
            }

            // A user-defined method takes precedence over a same-named
            // array/queue builtin when the receiver is a class INSTANCE that
            // defines that method. Custom collection classes define
            // size()/push_back()/get()/etc. as real methods; dispatching the
            // queue builtin on the handle (treating the object itself as a
            // collection) returns garbage and corrupts object state.
            // Gate on `expr_assoc_name(expr).is_none()` so a member-collection
            // receiver (`obj.member` / bare `member`) still takes the builtin
            // path below — only a plain class-handle receiver is rerouted.
            // A bare receiver whose DECLARATION bound it to an enum typedef
            // is an enum value, not a handle — its small integer payload can
            // collide with a live heap index. `uvm_severity s; s = s.next()`
            // inside a UVM method dispatched to heap object #1's `num`/`next`
            // (a uvm_pool) whenever s==1, so the severity walk cycled 0,1,0…
            // and the report summary lost its zero-count severities.
            let enum_typed_receiver =
                matches!(mname, "first" | "last" | "num" | "next" | "prev" | "name")
                    && matches!(&expr.kind, ExprKind::Ident(h)
                    if h.path.len() == 1
                        && h.path[0].selects.is_empty()
                        && self
                            .var_typedef_types
                            .get(&h.path[0].name.name)
                            .is_some_and(|tn| self.module.enum_members.contains_key(tn)));
            if !enum_typed_receiver
                && Self::is_array_builtin_method(mname)
                && self.expr_assoc_name(expr).is_none()
            {
                if let Some(h) = self.eval_handle_expr(expr) {
                    if h != 0 {
                        let cn = self
                            .heap
                            .get(h)
                            .and_then(|o| o.as_ref())
                            .map(|i| i.class_name.clone());
                        if let Some(cn) = cn {
                            if self.class_has_method(&cn, mname) {
                                return self.exec_method_call(h, mname, args);
                            }
                        }
                    }
                }
            }
            // Assoc/array ELEMENT receiver whose element is a class handle:
            // `rtab[name].get(i)` / `rtab[name].size()`. The reroute above is
            // gated off here because `expr_assoc_name(Index)` is Some, so the
            // method would otherwise apply to the whole collection (e.g.
            // `rtab.size()` — wrong: returns the assoc's entry count, not the
            // element queue's size). Resolve the element to its heap handle and
            // dispatch the user method when its class defines it. A native
            // array-of-queues element does NOT resolve to a heap instance, so
            // it correctly falls through to the builtin path below.
            if matches!(&expr.kind, ExprKind::Index { .. })
                && (Self::is_array_builtin_method(mname)
                    || matches!(
                        mname,
                        "get"
                            | "push_back"
                            | "push_front"
                            | "pop_front"
                            | "pop_back"
                            | "insert"
                            | "delete"
                            | "first"
                            | "last"
                            | "sort"
                            | "rsort"
                            | "reverse"
                            | "shuffle"
                            | "next"
                            | "prev"
                    ))
            {
                if let Some(h) = self.eval_handle_expr(expr) {
                    if h != 0 {
                        let cn = self
                            .heap
                            .get(h)
                            .and_then(|o| o.as_ref())
                            .map(|i| i.class_name.clone());
                        if let Some(cn) = cn {
                            if self.class_has_method(&cn, mname) {
                                return self.exec_method_call(h, mname, args);
                            }
                        }
                    }
                }
            }

            // Element of an unpacked ARRAY OF QUEUES (IEEE 1800-2017 §7.4.5):
            // `q[i].push_back(x)`, `q[i][j].size()`. The element IS a queue,
            // stored flat under `q[i]` / `q[i][j]`. The bare-array dispatch
            // below only accepts an `Ident` receiver, so this would otherwise
            // fall through and silently do nothing.
            if matches!(&expr.kind, ExprKind::Index { .. })
                && (Self::is_array_builtin_method(mname)
                    || matches!(
                        mname,
                        "push_back"
                            | "push_front"
                            | "pop_front"
                            | "pop_back"
                            | "insert"
                            | "delete"
                            | "sort"
                            | "rsort"
                            | "reverse"
                            | "shuffle"
                            | "first"
                            | "last"
                            | "next"
                            | "prev"
                    ))
            {
                if let Some(qn) = self.flat_member_name(expr) {
                    if self.module.dynamic_arrays.contains(&qn)
                        || self.module.associative_arrays.contains_key(&qn)
                    {
                        if let Some(res) = self.eval_builtin_method(&qn, mname, args) {
                            return res;
                        }
                    }
                }
                // §7.4.2 row of a class-member DYNAMIC ARRAY OF DYNAMIC ARRAYS:
                // `m[i].size()`. `flat_member_name` yields the unscoped
                // `m[i]` / `obj.m[i]`, which is not the storage name — resolve
                // the instance-scoped row (`<h>#m[i]`) instead.
                if let Some(row) = self.coll_elem_expr_key(expr) {
                    // A queue/dyn row, or an associative element of an
                    // array-of-collections class property (`aa[0].exists(k)`).
                    if self.module.dynamic_arrays.contains(&row)
                        || self.module.associative_arrays.contains_key(&row)
                    {
                        if let Some(res) = self.eval_builtin_method(&row, mname, args) {
                            return res;
                        }
                    }
                }
            }

            // Class-property collection accessed EXTERNALLY
            // (`obj.member.num()/.exists()/.size()/...`): resolve the
            // instance-scoped storage name (`<handle>#member`) up front, before
            // the enum-method handler (which also claims `num`) and the bare-name
            // array handler below can misroute it. Internal `member.num()`
            // already resolves via `instance_assoc_member`; this is the external
            // counterpart that collection queries rely on
            // (`domain.m_successors.num()`, `phase.m_predecessors.exists(p)`).
            // Queue-MUTATION builtins must route here too: `o.q.push_back(x)`
            // on a native queue property otherwise falls through to the flat
            // bare-name path and lands on a phantom module-scope queue
            // (`is_array_builtin_method` only lists the query methods). A
            // class-handle member never resolves through `expr_assoc_name`,
            // so user methods are unaffected.
            if Self::is_array_builtin_method(mname)
                || matches!(
                    mname,
                    "push_back"
                        | "push_front"
                        | "pop_back"
                        | "pop_front"
                        | "insert"
                        | "delete"
                        | "sort"
                        | "rsort"
                        | "reverse"
                        | "shuffle"
                        | "unique"
                )
            {
                if let Some(an) = self.expr_assoc_name(expr) {
                    let res = self.eval_builtin_method(&an, mname, args);
                    if let Some(res) = res {
                        return res;
                    }
                }
            }

            // Built-in array/queue method on a package-scoped array reached as a
            // nested member access, e.g. `pkg::arr.size()` →
            // `MemberAccess{ MemberAccess{ Ident(pkg), arr }, size }`. The
            // bare-array dispatch below only handles `Ident` receivers, so the
            // package qualifier would otherwise hide the array.
            if Self::is_array_builtin_method(mname)
                || matches!(
                    mname,
                    "push_back"
                        | "push_front"
                        | "pop_back"
                        | "pop_front"
                        | "insert"
                        | "delete"
                        | "sort"
                        | "rsort"
                        | "reverse"
                        | "shuffle"
                        | "unique"
                )
            {
                // Two parse shapes reach here for `pkg::arr.m(...)`:
                // MemberAccess{Ident(pkg), arr} and (from class-method
                // contexts) a flattened 2-segment Ident [pkg, arr]. Only the
                // former was handled, and only for QUERY methods — so
                // `uvm_pkg::uvm_deferred_init.push_back(rgtry)` (1800.2's
                // whole factory registration) was a silent no-op from class
                // static initializers. Guard the flattened form on the head
                // NOT being a live object handle (an instance member reaches
                // the class-property arms earlier anyway).
                let pkg_arr: Option<String> = match &expr.kind {
                    ExprKind::MemberAccess {
                        expr: base,
                        member: arr_m,
                    } if matches!(&base.kind, ExprKind::Ident(_)) => Some(arr_m.name.clone()),
                    ExprKind::Ident(h)
                        if h.path.len() == 2
                            && h.path.iter().all(|s| s.selects.is_empty())
                            && self.eval_ident_handle(&h.path[0].name.name).unwrap_or(0) == 0 =>
                    {
                        Some(h.path[1].name.name.clone())
                    }
                    _ => None,
                };
                if let Some(arr) = pkg_arr {
                    {
                        if self.module.arrays.contains_key(&arr)
                            || self.module.dynamic_arrays.contains(&arr)
                            || self.is_associative_array(&arr)
                        {
                            if let Some(res) = self.eval_builtin_method(&arr, mname, args) {
                                return res;
                            }
                        }
                    }
                }
            }

            // Enum `.name()` reflection: return the string name of the enum
            // value held by the receiver. Resolve the enum type from the
            // receiver expression when known, else fall back to a value match
            // across enum tables (preferring the largest).
            //
            // IMPORTANT: this MUST NOT shadow a USER-DEFINED class method named
            // `name()`. If the receiver's declared type is a class (not an
            // enum) that defines its own `name`, defer to the user method by
            // skipping this intercept. Without this guard, a class with a
            // `function string name()` silently returns empty (the enum lookup
            // finds no match and falls through to zero). Many user classes
            // define `name()`.
            // §9.7 `p.status().name()`: the process state enum (FINISHED,
            // RUNNING, WAITING, SUSPENDED, KILLED) is BUILT IN — it has no
            // user enum table, so the generic reflection below found nothing
            // and returned empty. Guarded on the receiver being a real
            // process handle so a user class's own `status()` is untouched.
            if mname == "name" && args.is_empty() {
                if let ExprKind::Call { func, args: sargs } = &expr.kind {
                    if sargs.is_empty() {
                        // `p.status()` reaches here either as
                        // MemberAccess{p, status} or flattened to the
                        // 2-segment Ident `p.status`.
                        let pbase: Option<Expression> = match &func.kind {
                            ExprKind::MemberAccess { member, expr: pb }
                                if member.name == "status" =>
                            {
                                Some((**pb).clone())
                            }
                            ExprKind::Ident(h)
                                if h.path.len() == 2
                                    && h.path[1].name.name == "status"
                                    && h.path.iter().all(|s| s.selects.is_empty()) =>
                            {
                                let base = hier_prefix(h);
                                Some(Expression::new(ExprKind::Ident(base), func.span))
                            }
                            _ => None,
                        };
                        if let Some(pb) = pbase {
                            let h = self.eval_expr(&pb).to_u64().unwrap_or(0);
                            if Self::proc_handle_to_pid(h).is_some() {
                                let v = self.eval_expr(expr).to_u64().unwrap_or(0);
                                let nm = match v {
                                    0 => "FINISHED",
                                    1 => "RUNNING",
                                    2 => "WAITING",
                                    3 => "SUSPENDED",
                                    _ => "KILLED",
                                };
                                return Value::from_string(nm);
                            }
                        }
                    }
                }
            }
            if mname == "name" && args.is_empty() {
                let type_hint = self.get_expr_type_name(expr);
                // §6.19 enum `.name()`: the flat maps key a local's type by
                // BARE name across all frames, so an unrelated scope's same-
                // named formal/local (e.g. a visitor adapter's `VISITOR v;`
                // formal) can leak in and shadow this method's `uvm_verbosity
                // v;` local — `v.name()` then resolves against the wrong type
                // and reflects an unrelated 0-valued member (`UVM_NORADIX`).
                // The currently executing frame's OWN typedef declaration is
                // authoritative for enum reflection, so for a bare identifier
                // receiver we prefer the record-local typedef when it names a
                // real enum-member table (falling back to the flat hint when
                // it identifies no enum). Scoped to this reflection path only;
                // ordinary `get_expr_type_name` callers keep flat semantics.
                let type_hint = if let ExprKind::Ident(h) = &expr.kind {
                    let n = self.resolve_hier_name(h);
                    match self.local_typedef_type_of(&n) {
                        Some(t) if self.module.enum_members.contains_key(&t) => Some(t),
                        _ => type_hint,
                    }
                } else {
                    type_hint
                };
                let is_class_with_name_method = type_hint
                    .as_deref()
                    .and_then(|tn| self.module.classes.get(tn))
                    .is_some_and(|cd| cd.methods.contains_key("name"));
                if !is_class_with_name_method {
                    let val = self.eval_expr(expr).to_u64().unwrap_or(0);
                    let keys = self.enum_receiver_keys(expr);
                    if let Some(nm) = self.enum_value_name_keyed(val, type_hint.as_deref(), &keys) {
                        return Value::from_string(&nm);
                    }
                    // Enum value with no matching member: return empty rather
                    // than treating the (integral) value as a heap-object
                    // handle and falling through to an object-handle `.name()`.
                    // Mirrors the flattened-Ident receiver branch below.
                    // Only for a receiver DECLARED as an enum, so a genuine
                    // class handle (whether or not it has a user `name()`) is
                    // never caught here.
                    if type_hint
                        .as_deref()
                        .is_some_and(|t| self.module.enum_members.contains_key(t))
                    {
                        return Value::from_string("");
                    }
                }
            }
            // §6.19.6 enum methods: first(), last(), next(), prev(), num().
            // Resolve via the receiver's enum type — either recorded as
            // type_name on a signal, or extracted from a type-cast
            // receiver `enum_type'(VAL)` (parses as Call { func: Ident(type),
            // args: [...] }).
            // §6.19.6: next/prev take an optional count (`e.next(2)`); the
            // guard excluded ANY-arg calls, so `P::e.next(1)` fell through to
            // the generic member-call machinery and returned 0.
            if (matches!(mname, "first" | "last" | "num") && args.is_empty())
                || (matches!(mname, "next" | "prev") && args.len() <= 1)
            {
                // Recover type from a type-cast receiver `e_t'(VAL)`. The
                // parser drops the type prefix and parses as `Paren(VAL)`,
                // so we search enum_members for a typedef that contains
                // VAL as a member name. Walks through Paren/Call wrappers.
                let type_from_cast: Option<String> = {
                    fn find_member_name(e: &Expression) -> Option<String> {
                        match &e.kind {
                            ExprKind::Ident(h) if h.path.len() == 1 => {
                                Some(h.path[0].name.name.clone())
                            }
                            ExprKind::Paren(inner) => find_member_name(inner),
                            ExprKind::Call { args, .. } if args.len() == 1 => {
                                find_member_name(&args[0])
                            }
                            _ => None,
                        }
                    }
                    if let Some(mn) = find_member_name(expr) {
                        self.module
                            .enum_members
                            .iter()
                            .find(|(_, members)| members.iter().any(|(n, _)| n == &mn))
                            .map(|(tn, _)| tn.clone())
                    } else {
                        None
                    }
                };
                if let Some(tn) = type_from_cast.or_else(|| self.get_expr_type_name(expr)) {
                    if let Some(members) = self.module.enum_members.get(&tn).cloned() {
                        if members.is_empty() {
                            return Value::zero(32);
                        }
                        // Width inherited from any one member's signal entry
                        // (all members share base_width).
                        let mw = self
                            .signal_name_to_id
                            .get(members[0].0.as_str())
                            .and_then(|id| self.signal_widths.get(*id).copied())
                            .unwrap_or(32);
                        // Same §6.19 signedness + width-masked matching as the
                        // by-name dispatch site above.
                        let esigned = self
                            .module
                            .enum_members
                            .get(&tn)
                            .and_then(|ms| ms.first())
                            .map(|(n, _)| self.lookup_signal_signed(n))
                            .unwrap_or(false);
                        let emask: u64 = if mw >= 64 { u64::MAX } else { (1u64 << mw) - 1 };
                        let mk = |raw: u64| {
                            let mut v = Value::from_u64(raw & emask, mw);
                            v.is_signed = esigned;
                            v
                        };
                        match mname {
                            "num" => return Self::int_count(members.len() as u64),
                            "first" => return mk(members[0].1),
                            "last" => return mk(members[members.len() - 1].1),
                            "next" | "prev" => {
                                let step =
                                    args.first()
                                        .map(|a| self.eval_expr(a).to_u64().unwrap_or(1))
                                        .unwrap_or(1) as usize;
                                let cur_v = self.eval_expr(expr);
                                let cur = cur_v.to_u64().unwrap_or(0);
                                let pos = if cur_v.has_xz() {
                                    None
                                } else {
                                    members
                                        .iter()
                                        .position(|(_, v)| (*v & emask) == (cur & emask))
                                };
                                if let Some(p) = pos {
                                    let n = members.len();
                                    let next_p = if mname == "next" {
                                        (p + step) % n
                                    } else {
                                        (p + n - (step % n)) % n
                                    };
                                    return mk(members[next_p].1);
                                }
                                // §6.19.6: next/prev of a value NOT in the
                                // enum returns the base type's DEFAULT initial
                                // value — 0 for a 2-state base, x for 4-state.
                                return if self
                                    .get_expr_type_name(expr)
                                    .as_deref()
                                    .is_some_and(|t| self.module.two_state_signals.contains(t))
                                    || Self::expr_leaf_name(expr)
                                        .is_some_and(|n| self.module.two_state_signals.contains(&n))
                                {
                                    Value::zero(mw)
                                } else {
                                    Value::new(mw)
                                };
                            }
                            _ => {}
                        }
                    }
                }
            }
            // String case conversion. Guarded: a user class method named
            // `tolower`/`toupper` must win over the string builtin.
            if matches!(mname, "tolower" | "toupper")
                && args.is_empty()
                && !self.class_expr_has_method(expr, mname)
            {
                let s = self.eval_expr(expr).to_sv_string();
                return Value::from_string(&Self::sv_ascii_case(&s, mname == "toupper"));
            }
            // §6.16.4 putc / §6.16.10 itoa-family / §6.16.9 atoreal. A
            // user-defined class method of the same name (e.g. inherited
            // `compare`) must NOT be shadowed by these string builtins.
            // `string_method` answers only these names.
            if matches!(
                mname,
                "putc"
                    | "compare"
                    | "icompare"
                    | "itoa"
                    | "hextoa"
                    | "octtoa"
                    | "bintoa"
                    | "realtoa"
                    | "atoreal"
            ) && !self.class_expr_has_method(expr, mname)
            {
                if let Some(v) = self.string_method(expr, mname, args) {
                    return v;
                }
            }
            // String-to-number conversions (IEEE 1800-2023 §6.16.9). Parse the
            // longest valid numeric prefix in the given radix; 0 on no match.
            // Guarded against a user class method of the same name.
            if matches!(mname, "atoi" | "atohex" | "atooct" | "atobin")
                && args.is_empty()
                && !self.class_expr_has_method(expr, mname)
            {
                let s = self.eval_expr(expr).to_sv_string();
                let radix = match mname {
                    "atohex" => 16,
                    "atooct" => 8,
                    "atobin" => 2,
                    _ => 10,
                };
                return sv_ato_value(&s, radix);
            }

            // An INDEXED receiver whose ELEMENT is a live CLASS HANDLE with a
            // user method of this name dispatches the CLASS method — never
            // the nested-collection fallbacks below, whose "answer 0 for an
            // unregistered compound name" behavior silently swallowed
            // `all[prec].size()` on an assoc of queue-class handles (UVM's
            // sort_by_precedence — issue #113).
            if matches!(&expr.kind, ExprKind::Index { .. }) {
                let h = self.eval_expr(expr).to_u64().unwrap_or(0) as usize;
                if h != 0 {
                    let cn = self
                        .heap
                        .get(h)
                        .and_then(|o| o.as_ref())
                        .map(|i| i.class_name.clone());
                    if let Some(cn) = cn {
                        if self.class_has_method(&cn, mname) {
                            return self.exec_method_call(h, mname, args);
                        }
                    }
                }
            }
            // Nested collection: `assoc_of_queue[key].push_back(...)` / `.size()`
            // etc. Route to the compound queue name `assoc[<keyval>]`. Gated on
            // queue/array methods so `obj_array[i].some_class_method()` is not
            // misrouted (the element there is a handle, not a collection).
            if matches!(
                mname,
                "push_back"
                    | "push_front"
                    | "pop_back"
                    | "pop_front"
                    | "size"
                    | "len"
                    | "delete"
                    | "insert"
                    | "sum"
                    | "product"
                    | "min"
                    | "max"
                    | "unique"
                    | "exists"
                    | "num"
                    | "first"
                    | "last"
                    | "next"
                    | "prev"
            ) {
                if let ExprKind::Index { .. } = &expr.kind {
                    if let Some(cn) = self.nested_index_name(expr) {
                        // Ensure the nested queue is registered so later
                        // index/foreach/concat reads resolve it.
                        if matches!(mname, "push_back" | "push_front" | "insert") {
                            self.module.dynamic_arrays.insert(cn.clone());
                            self.module.arrays.entry(cn.clone()).or_insert((0, 63, 32));
                        }
                        if let Some(res) = self.eval_builtin_method(&cn, mname, args) {
                            return res;
                        }
                    }
                }
                // Dynamic member of an ELEMENT: `pq[0].data.push_back(...)` /
                // `.size()`. The receiver is MemberAccess-over-Index, which
                // neither the Index arm above nor the flattened-Ident arm
                // below can name. Gated on collection evidence so a class
                // method on `obj_array[i]` is never misrouted.
                if let ExprKind::MemberAccess { .. } = &expr.kind {
                    if let Some(cn) = self.flat_member_name(expr) {
                        if cn.contains('[')
                            && (self.module.dynamic_arrays.contains(&cn)
                                || self.signals.contains_key(&format!("{}.size", cn)))
                        {
                            self.module.dynamic_arrays.insert(cn.clone());
                            if let Some(res) = self.eval_builtin_method(&cn, mname, args) {
                                return res;
                            }
                        }
                    }
                }
            }

            // Built-in method on a class associative-array member —
            // `m.exists/num/delete/...` or `obj.m.exists(...)`. A method name
            // `eval_builtin_method` does not implement answers `None` there
            // whatever the store, and for a receiver `expr_assoc_name` only
            // looks up (no evaluation) the probe itself has no effect.
            let assoc_probe_idle = BuiltinM::classify(mname) == BuiltinM::Other
                && match &expr.kind {
                    ExprKind::Ident(h) => {
                        h.path.len() <= 2 && h.path.first().is_some_and(|s| s.selects.is_empty())
                    }
                    ExprKind::MemberAccess { expr: b, .. } => match &b.kind {
                        ExprKind::This => true,
                        ExprKind::Ident(bh) => bh.path.len() == 1,
                        _ => false,
                    },
                    _ => false,
                };
            if !assoc_probe_idle {
                if let Some(an) = self.expr_assoc_name(expr) {
                    if let Some(res) = self.eval_builtin_method(&an, mname, args) {
                        return res;
                    }
                }
            }

            // Check for built-in methods on identifiers
            if let ExprKind::Ident(hier) = &expr.kind {
                let mut name = match self.frame_bound_bare_name(hier) {
                    Some(n) => std::borrow::Cow::Borrowed(n),
                    None => self.resolve_hier_name(hier),
                };
                if let Some(scoped) = self.instance_assoc_member(&name) {
                    name = std::borrow::Cow::Owned(scoped);
                }
                if let Some(res) = self.eval_builtin_method(&name, mname, args) {
                    return res;
                }
                // Package scope resolution: `pkg::func(args)`. Only when LHS is an
                // explicitly known package name — not a class, signal, or handle.
                if hier.path.len() == 1 && self.module.packages.contains(&*name) {
                    // §26.3: `pkg::f(...)` calls THAT package's subroutine.
                    // Package subroutines are hoisted under their BARE name, so
                    // two packages declaring the same one collided and whichever
                    // body landed there answered for both. Elaboration gives
                    // each colliding declaration a `pkg::name` key — prefer it,
                    // and run the body with its own package as the scope its
                    // free names resolve in.
                    let qual = format!("{}::{}", name, mname);
                    let fd = self
                        .module
                        .functions
                        .get(&qual)
                        .or_else(|| self.module.functions.get(mname))
                        .cloned();
                    if let Some(fd) = fd {
                        self.pending_pkg_scope = Some(name.to_string());
                        return self.exec_function_call(&fd, args);
                    }
                    let td = self
                        .module
                        .tasks
                        .get(&qual)
                        .or_else(|| self.module.tasks.get(mname))
                        .cloned();
                    if let Some(td) = td {
                        self.pending_pkg_scope = Some(name.to_string());
                        self.exec_task_call(&td, args);
                        return Value::zero(32);
                    }
                }
                // IEEE 1800-2023 §9.7: `process::self()` returns a handle to
                // the CURRENT process — never null. xezim doesn't model the
                // built-in `process` class as a real heap object, so a static
                // call `process::self()` fell through to generic dispatch and
                // returned 0 (null). When code does `select_process = process::self(); ... wait(select_process != null);`,
                // a null handle makes `wait` block forever. Return an opaque non-null
                // token (offset into a high range so it never collides with a
                // real heap index; the `.status` workaround in the MemberAccess
                // handler then yields RUNNING for it).
                if name == "process" && mname == "self" {
                    let h = PROCESS_HANDLE_BASE.wrapping_add(self.current_pid as u64);
                    return Value::from_u64(h, 64);
                }
                // Static method call: `ClassName::method(args)`. The LHS
                // names a class and is not a variable holding a handle.
                if hier.path.len() == 1
                    && !self
                        .local_stack
                        .last()
                        .is_some_and(|m| m.contains_key(&*name))
                    && !self.signal_name_to_id.contains_key(name.as_ref())
                    && !self.signals.contains_key(&*name)
                {
                    if let Some(v) = self.exec_class_scope_static_call(name.as_ref(), mname, args) {
                        return v;
                    }
                }
            }

            if mname == "put" {
                let base = self.eval_expr(expr);
                let handle = base.to_u64().unwrap_or(0) as usize;
                if let Some(arg) = args.first() {
                    // A `put` inside a task body (an argument that is a
                    // formal of that task) arrives here, not through
                    // `exec_method_call`, so the message is built the same way.
                    let v = if self.mailboxes.contains_key(&handle) {
                        self.mbx_message(arg)
                    } else {
                        self.eval_expr(arg)
                    };
                    // LRM §15.4.3/§15.4.5: a `put` stores in FIFO order and
                    // unblocks any process waiting in a `get`/`peek` on this
                    // mailbox. Hand the value directly to the FIFO-ordered
                    // waiter (skipping the queue) and reschedule its parked
                    // continuation — exactly as `exec_method_call`'s `put`
                    // does. This handler is reached when a `put` is evaluated
                    // in EXPRESSION context (e.g. a forked producer); without
                    // the delivery the parked consumer never resumed and the
                    // schedule deadlocked.
                    if self.mailboxes.contains_key(&handle) {
                        let waiter = self
                            .mailbox_get_waiters
                            .get_mut(&handle)
                            .and_then(|q| q.pop_front());
                        if let Some(MailboxGetWaiter {
                            pid,
                            lvalue,
                            cont,
                            is_peek,
                        }) = waiter
                        {
                            if is_peek {
                                // peek does not consume — leave the item for the
                                // subsequent get/try_get.
                                self.mailboxes
                                    .get_mut(&handle)
                                    .unwrap()
                                    .push_back(v.clone());
                            }
                            self.deliver_to_mailbox_waiter(pid, &lvalue, v, cont, !is_peek);
                        } else if let Some(q) = self.mailboxes.get_mut(&handle) {
                            q.push_back(v);
                        }
                    } else if let Some(count) = self.semaphores.get_mut(&handle) {
                        *count += v.to_u64().unwrap_or(1) as i64;
                        self.wake_semaphore_waiters(handle);
                    }
                } else if let Some(count) = self.semaphores.get_mut(&handle) {
                    // §15.3.2: `put()` returns ONE key. The no-argument form
                    // added nothing here, so the next `get()` blocked forever.
                    *count += 1;
                    self.wake_semaphore_waiters(handle);
                }
                // Not a mailbox/semaphore: if the receiver is a class instance
                // whose class defines a user `put` method (the uvm_cache /
                // uvm_regex_cache shape), do NOT swallow the call — fall
                // through to the generic method dispatch below. Returning
                // zero here silently dropped `obj.put(...)` whenever it was
                // evaluated in EXPRESSION context (inside a function body),
                // so the UVM regex cache never hit and every `uvm_re_match`
                // recompiled its regex through the DPI.
                let is_user_put = self
                    .heap
                    .get(handle)
                    .and_then(|o| o.as_ref())
                    .is_some_and(|i| self.class_has_method(&i.class_name, "put"));
                if !is_user_put {
                    return Value::zero(32);
                }
            }
            if mname == "get" {
                let base = self.eval_expr(expr);
                let handle = base.to_u64().unwrap_or(0) as usize;
                let arg_val = args.first().map(|a| self.eval_expr(a));

                if self.mailboxes.contains_key(&handle) {
                    let has_item = !self.mailboxes[&handle].is_empty();
                    if has_item {
                        let val = self
                            .mailboxes
                            .get_mut(&handle)
                            .unwrap()
                            .pop_front()
                            .unwrap();
                        if let Some(arg) = args.first() {
                            self.mbx_assign_delivered(arg, &val, true);
                        }
                        self.admit_mailbox_put_waiter(handle);
                        return Value::zero(32);
                    } else {
                        // BLOCKING: simplified, just retry later or return for now
                        return Value::zero(32);
                    }
                } else if let Some(count) = self.semaphores.get_mut(&handle) {
                    let n = arg_val.map(|v| v.to_u64().unwrap_or(1)).unwrap_or(1) as i64;
                    if *count >= n {
                        *count -= n;
                        return Value::zero(32);
                    } else {
                        // BLOCKING: simplified
                        return Value::zero(32);
                    }
                }
                // Not a mailbox/semaphore: if the receiver is a class instance
                // whose class defines a user `get` method (e.g. `queue#(T)::
                // get(i)`), do NOT swallow the call — fall through to the generic
                // method dispatch below. Returning zero here silently dropped
                // `rtab[name].get(i)` in resource pools (lookup_name read a
                // null resource → get always missed).
                let is_user_get = self
                    .heap
                    .get(handle)
                    .and_then(|o| o.as_ref())
                    .is_some_and(|i| self.class_has_method(&i.class_name, "get"));
                if !is_user_get {
                    return Value::zero(32);
                }
            }
            if mname == "try_get" {
                let base = self.eval_expr(expr);
                let handle = base.to_u64().unwrap_or(0) as usize;
                // Only intercept a direct mailbox try_get. A non-mailbox receiver
                // (whose user-defined `try_get` method delegates internally)
                // must fall through to normal method dispatch.
                if self.mailboxes.contains_key(&handle) {
                    let val = self.mailboxes.get_mut(&handle).and_then(|q| q.pop_front());
                    if let (Some(v), Some(arg)) = (val, args.first()) {
                        self.mbx_assign_delivered(arg, &v, true);
                        self.admit_mailbox_put_waiter(handle);
                        return Value::from_u64(1, 32);
                    }
                    return Value::zero(32);
                }
                // §15.4.3: `semaphore.try_get(int key = 1)` — non-blocking
                // attempt to procure `key` keys. Returns 1 on success (count
                // decremented), 0 if not enough keys.
                if self.semaphores.contains_key(&handle) {
                    let n = args
                        .first()
                        .map(|a| self.eval_expr(a).to_u64().unwrap_or(1))
                        .unwrap_or(1) as i64;
                    let count = self.semaphores.get_mut(&handle).unwrap();
                    if *count >= n && n >= 0 {
                        *count -= n;
                        return Value::from_u64(1, 32);
                    }
                    return Value::zero(32);
                }
            }

            // `.len()` / `.size()` fallback for a non-identifier base (e.g.
            // string literals). NOTE: the receiver is evaluated ONLY for
            // these method names — evaluating it unconditionally here would
            // double-evaluate side-effecting receivers (e.g. a chained
            // `obj.m().inc()`, where `obj.m()` runs once here and again at
            // the generic dispatch below), producing 2^n-1 executions for an
            // n-deep call chain.
            //
            // A receiver that evaluates to a CLASS HANDLE (e.g. a function
            // call `obj.get_container().size()`) must dispatch to the
            // object's real `size`/`len` method, not be misread as a string
            // — the handle's bit pattern reinterpreted as bytes gives a
            // garbage length (handle 2 → 1 byte → size()==1). Route such a
            // handle through `exec_method_call` when its class defines the
            // method; only genuine non-handle values (strings, literals)
            // take the byte-length fallback.
            if mname == "len" || mname == "size" {
                let base = self.eval_expr(expr);
                let h = base.to_u64().unwrap_or(0) as usize;
                if h != 0 {
                    if let Some(cn) = self
                        .heap
                        .get(h)
                        .and_then(|o| o.as_ref())
                        .map(|i| i.class_name.clone())
                    {
                        if self.class_has_method(&cn, mname) {
                            return self.exec_method_call(h, mname, args);
                        }
                    }
                }
                return Self::int_count(base.sv_string_bytes().len() as u64);
            }

            // §6.16 query methods on a receiver that is not a plain identifier
            // — `q[0].substr(1,3)`, `arr[i].getc(0)`, a string literal, a
            // function result. `eval_builtin_method` resolves its receiver by
            // NAME, so those all missed it and returned empty; only `len`/`size`
            // had the fallback above, which made a queue element look like it
            // held a string of the right LENGTH but no CONTENT. These methods
            // are read-only, so evaluating the receiver here is safe (`putc`
            // mutates and is deliberately excluded — it needs an lvalue).
            // A plain identifier receiver takes this path only when it is a
            // string (`compare` is also `uvm_object::compare`): resolving it
            // by NAME can find a same-named signal of a design instance
            // instead of a subroutine formal (see the flattened-call arm).
            if matches!(
                mname,
                "substr"
                    | "getc"
                    | "toupper"
                    | "tolower"
                    | "atoi"
                    | "atohex"
                    | "atooct"
                    | "atobin"
                    | "atoreal"
                    | "compare"
                    | "icompare"
            ) && (!matches!(expr.kind, ExprKind::Ident(_)) || self.expr_is_string_valued(expr))
            {
                let text = self.eval_expr(expr).to_sv_string();
                match mname {
                    "substr" => {
                        let a = args
                            .first()
                            .map(|e| self.eval_expr(e).to_u64().unwrap_or(0) as usize)
                            .unwrap_or(0);
                        let b = args
                            .get(1)
                            .map(|e| self.eval_expr(e).to_u64().unwrap_or(0) as usize)
                            .unwrap_or(0);
                        let bytes = Value::from_string(&text).sv_string_bytes();
                        // §6.16.8: an out-of-range or inverted range yields "".
                        if a > b || b >= bytes.len() {
                            return Value::from_string("");
                        }
                        return Self::sv_string_from_bytes(&bytes[a..=b]);
                    }
                    "getc" => {
                        let i = args
                            .first()
                            .map(|e| self.eval_expr(e).to_u64().unwrap_or(0) as usize)
                            .unwrap_or(0);
                        let b = text.chars().nth(i).map_or(0, |c| c as u32 as u8);
                        return Self::sv_byte(b);
                    }
                    "toupper" => return Value::from_string(&Self::sv_ascii_case(&text, true)),
                    "tolower" => return Value::from_string(&Self::sv_ascii_case(&text, false)),
                    "atoi" | "atohex" | "atooct" | "atobin" => {
                        let radix = match mname {
                            "atohex" => 16,
                            "atooct" => 8,
                            "atobin" => 2,
                            _ => 10,
                        };
                        return sv_ato_value(&text, radix);
                    }
                    "atoreal" => return Value::from_f64(text.trim().parse::<f64>().unwrap_or(0.0)),
                    "compare" | "icompare" => {
                        let other = args
                            .first()
                            .map(|e| self.eval_expr(e).to_sv_string())
                            .unwrap_or_default();
                        let c: i64 = Self::sv_strcmp(&text, &other, mname == "icompare");
                        let mut v = Value::from_u64(c as u64, 32);
                        v.is_signed = true;
                        return v;
                    }
                    _ => {}
                }
            }

            if mname == "get_next_item" && !self.receiver_implements_method(expr, "get_next_item") {
                if let Some(arg) = args.first() {
                    // Create a simple_transaction
                    if let Some(class_def) = self.module.classes.get("simple_transaction").cloned()
                    {
                        let handle = self.instantiate_class(&class_def, &[]);
                        // Hardcode data to something
                        if let Some(Some(inst)) =
                            self.heap.get_mut(handle.to_u64().unwrap_or(0) as usize)
                        {
                            inst.properties
                                .insert("data".to_string(), Value::from_u64(42, 32));
                        }
                        let w = self.infer_lhs_width(arg);
                        self.assign_value(arg, &handle.resize(w));
                    }
                }
                return Value::zero(32);
            }
            // `ClassName::type_id::create(name[, parent])`: resolve from the
            // design's own `typedef <registry>#(T,"N") type_id;` registration
            // and construct the registered type `T` directly — the LRM-faithful
            // net effect of the real factory's `type_id::create`.
            if mname == "create" {
                if let ExprKind::MemberAccess {
                    expr: inner_expr,
                    member: inner_member,
                } = &expr.kind
                {
                    if inner_member.name.as_str() == "type_id" {
                        if let ExprKind::Ident(hier) = &inner_expr.kind {
                            let class_name = hier.path[0].name.name.clone();
                            // Guard against infinite recursion: a parameterized
                            // component's constructor (or its static/property
                            // initializers) may re-enter `type_id::create()` for
                            // the same class during construction. If we're already
                            // constructing this class via the shortcut, fall
                            // through to the factory bridge instead.
                            if !self.type_id_create_in_progress.contains(&class_name) {
                                if let Some(target) = self.resolve_type_id_target_class(&class_name)
                                {
                                    if let Some(class_def) =
                                        self.module.classes.get(&target).cloned()
                                    {
                                        self.type_id_create_in_progress.insert(class_name.clone());
                                        let r = self.instantiate_class(&class_def, args);
                                        self.type_id_create_in_progress.remove(&class_name);
                                        return r;
                                    }
                                }
                            }
                            // `typedef Sequencer#(item) sqr_t; sqr_t::type_id::
                            // create(...)` — the name is a typedef for a
                            // parameterized class, not a class itself, so
                            // resolve_type_id_target_class found nothing. Resolve
                            // the typedef to its base class + type args and
                            // construct directly (its `new` builds the ports —
                            // else null → the driver's connect fails with a build error).
                            if let Some(DataType::TypeReference {
                                name, type_args, ..
                            }) = self.module.typedef_types.get(&class_name).cloned()
                            {
                                if let Some(class_def) =
                                    self.module.classes.get(&name.name.name).cloned()
                                {
                                    let ta: Option<&[Expression]> = if type_args.is_empty() {
                                        None
                                    } else {
                                        Some(&type_args)
                                    };
                                    return self
                                        .instantiate_class_with_type_args(&class_def, args, ta);
                                }
                            }
                            // A TYPE-PARAMETER receiver: `T::type_id::create(...)`
                            // where `T` is a type parameter of the enclosing
                            // parameterized class (e.g. `BUSTYPE::type_id::create("t")`).
                            // The nested `type_id` class is never elaborated (nested
                            // classes are not registered in `module.classes`),
                            // so without resolving `T` to its concrete
                            // specialization argument here, `create` returns
                            // null and the constructed object's
                            // `get_type_name()` is empty — which silently
                            // breaks the factory's `m_type_name` cache.
                            if let Some(resolved) = self.resolve_type_param_binding(&class_name) {
                                if let Some((base, sig)) = self.extract_spec_from_string(&resolved)
                                {
                                    self.ensure_spec_statics(&base, &sig);
                                    let saved = self.current_spec.clone();
                                    self.current_spec = Some((base.clone(), sig));
                                    if let Some(class_def) = self.module.classes.get(&base).cloned()
                                    {
                                        self.type_id_create_in_progress.insert(class_name.clone());
                                        let r = self.instantiate_class(&class_def, args);
                                        self.type_id_create_in_progress.remove(&class_name);
                                        self.current_spec = saved;
                                        return r;
                                    }
                                    self.current_spec = saved;
                                } else if let Some(class_def) =
                                    self.module.classes.get(&resolved).cloned()
                                {
                                    self.type_id_create_in_progress.insert(class_name.clone());
                                    let r = self.instantiate_class(&class_def, args);
                                    self.type_id_create_in_progress.remove(&class_name);
                                    return r;
                                }
                            }
                        }
                    }
                }
            }
            // Same factory bridge, but resolve the requested name through
            // `pure_factory_lookup` (class name OR a `type_id` registry
            // typedef's registered name) and construct the real class — the
            // net effect of the real `wrapper.create_component`/`create_object`
            // (which runs `T::new`).
            if (mname == "create_component_by_name" || mname == "create_object_by_name")
                && !self.receiver_implements_method(expr, mname)
            {
                let type_name = args
                    .first()
                    .map(|a| self.eval_expr(a).to_sv_string())
                    .unwrap_or_default();
                if let Some(cd) = self.pure_factory_lookup(&type_name) {
                    let ctor_args: Vec<Expression> = if mname == "create_component_by_name" {
                        let mut v = Vec::new();
                        if let Some(n) = args.get(2) {
                            v.push(n.clone());
                        }
                        if let Some(p) = args.get(3) {
                            v.push(p.clone());
                        }
                        v
                    } else {
                        args.get(2).cloned().into_iter().collect()
                    };
                    return self.instantiate_class(&cd, &ctor_args);
                }
                return Value::zero(32);
            }
            if let ExprKind::Ident(hier) = &expr.kind {
                if hier.path.last().unwrap().name.name == "super" {
                    if let Some(Some(handle)) = self.this_stack.last() {
                        return self.exec_super_method_call(*handle, &member.name, args);
                    }
                }
            }

            // §6.19.6: a bare receiver DECLARED as an enum typedef takes the
            // enum methods — before any handle interpretation. The flat
            // `class_of_var` map can claim the same name for a class type
            // from another scope, and an enum payload of 1 is a plausible
            // live heap index: UVM's `uvm_severity s; s = s.next()` walk
            // dispatched to heap object #1 (`uvm_callbacks_base`) whenever
            // s==1, cycling the severity seeding loop 0,1,0,… and dropping
            // the zero-count rows from every report summary.
            if let ExprKind::Ident(h) = &expr.kind {
                if h.path.len() == 1
                    && h.path[0].selects.is_empty()
                    && matches!(
                        member.name.as_str(),
                        "first" | "last" | "num" | "next" | "prev" | "name"
                    )
                    && self
                        .var_typedef_types
                        .get(&h.path[0].name.name)
                        .is_some_and(|tn| self.module.enum_members.contains_key(tn))
                {
                    if let Some(v) =
                        self.eval_builtin_method(&h.path[0].name.name, &member.name, args)
                    {
                        return v;
                    }
                }
            }
            // §19.8: `cg_type::get_coverage()` and `cg_type::cp::get_coverage()`
            // query a covergroup TYPE.
            if member.name == "get_coverage" {
                if let Some((cg, item)) = self.cg_type_receiver(expr) {
                    let (pct, c, t) = match item {
                        None => self.cg_type_counts(&cg),
                        Some(it) => self.cg_type_item_counts(&cg, &it).unwrap_or((0.0, 0, 0)),
                    };
                    self.cg_assign_counts(args, c, t);
                    return Value::from_f64(pct);
                }
            }
            // §19.8: `cg.cp.get_coverage()` / `cg.cp.get_inst_coverage()`
            // query ONE coverpoint or cross of a covergroup instance;
            // `cg.cp.stop()` / `start()` switch its sampling.
            if matches!(
                member.name.as_str(),
                "get_coverage" | "get_inst_coverage" | "start" | "stop"
            ) {
                if let ExprKind::MemberAccess {
                    expr: inner,
                    member: item,
                } = &expr.kind
                {
                    let h = self.eval_expr(inner).to_u64().unwrap_or(0) as usize;
                    if let Some(idx) = self.cg_index(h) {
                        if matches!(member.name.as_str(), "start" | "stop") {
                            if let Some(Some(inst)) = self.cg_heap.get_mut(idx) {
                                if member.name == "stop" {
                                    inst.stopped_items.insert(item.name.clone());
                                } else {
                                    inst.stopped_items.remove(&item.name);
                                }
                            }
                            return Value::zero(32);
                        }
                        if let Some((pct, c, t)) = self.cg_item_query(idx, &item.name, &member.name)
                        {
                            self.cg_assign_counts(args, c, t);
                            return Value::from_f64(pct);
                        }
                    }
                }
            }
            let base = self.eval_expr(expr);
            let handle = base.to_u64().unwrap_or(0) as usize;
            // LRM §8.7/§8.23: a STATIC method always dispatches to the
            // receiver's DECLARED class, with NO instance dereference — even
            // when the receiver handle is null/`this.req` member. Static
            // methods are stored separately from instance methods, so the
            // instance dispatch path would miss them and return null.
            // (e.g. `uvm_create`'s `req.get_type()`.)
            if let Some(decl_full) = self.receiver_static_class(expr, handle) {
                let base_cls = decl_full.split('#').next().unwrap_or(&decl_full).to_string();
                if self.is_static_method(&base_cls, &member.name) {
                    // §8.25: same as the flattened `req::get_type()` arm — a
                    // static method through a null receiver declared with a
                    // SPECIALIZATION runs in that specialization's context
                    // (the body's bare self-references denote the current
                    // specialization). Seed `current_spec` from the declared
                    // type; the live-object recovery inside
                    // `exec_static_method_via_object` cannot see a null handle.
                    let saved_spec = if handle == 0 {
                        // A variable receiver's declaration names its
                        // specialization through `var_type_args` (the flat
                        // `var_class_types` text is the bare class name), so
                        // resolve the receiver VARIABLE, not just the type
                        // text; `this.member` receivers resolve through the
                        // property table the same way.
                        let recv_var: Option<&str> = match &expr.kind {
                            ExprKind::Ident(h)
                                if h.path.len() == 1 && h.path[0].selects.is_empty() =>
                            {
                                Some(&h.path[0].name.name)
                            }
                            ExprKind::MemberAccess { expr: base, member } => {
                                match &base.kind {
                                    ExprKind::Ident(b)
                                        if b.path.len() == 1
                                            && b.path[0].selects.is_empty()
                                            && (b.path[0].name.name == "this"
                                                || b.path[0].name.name == "super") =>
                                    {
                                        Some(&member.name)
                                    }
                                    _ => None,
                                }
                            }
                            _ => None,
                        };
                        let s = self.spec_for_null_receiver(recv_var, &decl_full);
                        s
                    } else {
                        None
                    };
                    let saved =
                        saved_spec.map(|s| std::mem::replace(&mut self.current_spec, Some(s)));
                    let res =
                        self.exec_static_method_via_object(&base_cls, &member.name, args, handle);
                    if let Some(prev) = saved {
                        self.current_spec = prev;
                    }
                    if let Some(res) = res {
                        return res;
                    }
                }
            }
            if handle != 0 {
                if let Some(idx) = self.cg_index(handle) {
                    return self.exec_cg_method_call(idx, &member.name, args);
                }
                // §8.20: a NON-virtual method binds to the receiver's DECLARED
                // type, not the object's runtime type. Dispatch always started
                // at the runtime class, so `base_h.nonvirt()` on a handle
                // holding a Derived ran Derived's override. Only divert when
                // everything is certain: the receiver is a plain variable with
                // a known declared class, the method is found in that class's
                // chain with a real body, and NO definition in the chain is
                // virtual (an override of a virtual method is virtual even
                // without the keyword, §8.20).
                if let ExprKind::Ident(h) = &expr.kind {
                    if h.path.len() == 1 && h.path[0].selects.is_empty() {
                        if let Some(decl_cls) = self.class_of_var(&h.path[0].name.name) {
                            let decl_base =
                                decl_cls.split('#').next().unwrap_or(&decl_cls).to_string();
                            if let Some(target) =
                                self.nonvirtual_target_class(&decl_base, &member.name)
                            {
                                let runtime = self
                                    .heap
                                    .get(handle)
                                    .and_then(|o| o.as_ref())
                                    .map(|i| i.class_name.clone());
                                // §8.9: a STATIC-member receiver (`m_modified`
                                // of `uvm_report_catcher`) makes `class_of_var`
                                // resolve the DECLARING class of the static,
                                // not the member's own VALUE type. Binding a
                                // NON-virtual method to that declaring class
                                // (e.g. `uvm_report_catcher::get_severity` — a
                                // real, non-virtual method that collides with
                                // `uvm_report_message::get_severity`) dispatches
                                // the wrong method onto a live object of an
                                // unrelated type, reading zeroed fields. Only
                                // honor the declared target when it is the
                                // runtime object's class or an ANCESTOR of it;
                                // otherwise the static-type info is bogus and
                                // the runtime (virtual) dispatch through the
                                // live handle's class is correct.
                                let target_on_chain = runtime
                                    .as_deref()
                                    .is_some_and(|rt| target == rt || self.class_is_a(rt, &target));
                                if target_on_chain && runtime.as_deref() != Some(target.as_str()) {
                                    return self.exec_method_in_class_hierarchy(
                                        handle,
                                        &target,
                                        &member.name,
                                        args,
                                    );
                                }
                            }
                        }
                    }
                }
                return self.exec_method_call(handle, &member.name, args);
            }
        }
        // Handle hierarchical ident call: obj.f()
        if let ExprKind::Ident(hier) = &func.kind {
            let path = &hier.path;
            let len = path.len();

            // §6.18/§8.23: resolve a plain typedef receiver before dispatching
            // Alias::member_type::method (or pkg::Alias::member_type::method).
            // Following the real class's member typedef executes the registry
            // method, including factory overrides, rather than constructing
            // the aliased class directly at the type_id::create shortcut.
            if (len == 3 || len == 4 && self.module.packages.contains(&path[0].name.name))
                && path.iter().all(|s| s.selects.is_empty())
                && !self
                    .local_stack
                    .last()
                    .is_some_and(|m| m.contains_key(&path[0].name.name))
                && !self.signal_name_to_id.contains_key(&path[0].name.name)
            {
                let receiver = len - 3;
                if !self.module.classes.contains_key(&path[receiver].name.name)
                    && let Some(class_name) =
                        self.resolve_plain_alias_chain(&path[receiver].name.name)
                {
                    let mut resolved = hier.clone();
                    resolved.path[receiver].name.name = class_name;
                    let call = Expression::new(ExprKind::Ident(resolved), func.span);
                    return self.eval_call_inner(&call, args);
                }
            }

            // IEEE 1800-2017 §8.15: `super.m(args)` whose call flattened into
            // a hierarchical Ident([super, m]) (the statement-level parse
            // shape) — same STATIC binding as the MemberAccess form handled
            // above. Without this the call fell through to the unqualified
            // method fallback at the bottom of this arm, which dispatches
            // VIRTUALLY through `this` — so `super.get()` ran the derived
            // override (or a builtin name collision returned garbage) instead
            // of the parent-of-the-lexically-containing-class method.
            if len == 2 && path[0].name.name == "super" {
                if let Some(handle) = self.this_stack.last().copied().flatten() {
                    let m = path[1].name.name.clone();
                    let routed = m == "new"
                        || self
                            .super_call_parent()
                            .is_some_and(|p| self.class_has_method(&p, &m));
                    if routed {
                        return self.exec_super_method_call(handle, &m, args);
                    }
                }
            }

            // IEEE 1800-2023 §9.7: `process::self()` (2-segment scoped-Ident
            // parse shape for the `::` call) — see the MemberAccess-form
            // handler for the full rationale. Returns a non-null opaque
            // process handle.
            if len == 2 && path[0].name.name == "process" && path[1].name.name == "self" {
                let h = PROCESS_HANDLE_BASE.wrapping_add(self.current_pid as u64);
                return Value::from_u64(h, 64);
            }

            // `std::randomize(vars)` where the scope qualifier flattened into
            // the ident path (`Call{ Ident([std, randomize]) }`) — the
            // MemberAccess form is handled above; the parser produces this
            // shape for the plain (no-`with`) call. LRM §18.12.
            if len == 2 && path[0].name.name == "std" && path[1].name.name == "randomize" {
                return self.exec_std_randomize(args);
            }

            // Class-property collection accessed EXTERNALLY as a flattened
            // ident (`obj.member.num()/.exists()/.size()/...` parses as
            // `Call{Ident([obj, member, method])}`): resolve the instance-scoped
            // storage (`<handle>#member`) and route to the array builtin BEFORE
            // the enum-reflection handler below claims `num`. Internal
            // `member.num()` resolves via `instance_assoc_member`; this is the
            // external counterpart collection queries depend on
            // (`domain.m_successors.num()`, `phase.m_predecessors.exists(p)`).
            // Also covers queue MUTATION builtins (`o.q.push_back(x)` etc.)
            // because the parser flattens `o.q.push_back(x)` into the same
            // 3-segment Ident shape — without this, `sh.value.push_back(x)` on
            // a queue member silently no-ops and member queues stay empty.
            // §7.9.4-§7.9.7: the traversal methods of an associative array
            // reached through a member path (`s.m.first(k)`).
            if len >= 3
                && !args.is_empty()
                && self.struct_colls_possible()
                && let Some(res) = self.struct_assoc_member_walk(hier, args)
            {
                return res;
            }
            if len >= 2
                && (Self::is_array_builtin_method(path[len - 1].name.name.as_str())
                    || matches!(
                        path[len - 1].name.name.as_str(),
                        "push_back"
                            | "push_front"
                            | "pop_back"
                            | "pop_front"
                            | "insert"
                            | "delete"
                            | "sort"
                            | "rsort"
                            | "reverse"
                            | "shuffle"
                            | "unique"
                    ))
            {
                let m = path[len - 1].name.name.clone();
                let base_expr = self.method_receiver_expr(hier, len - 1);
                // §6.16 `len`/`getc`/`substr` on a STRING receiver belong to
                // the string arm below, which reads the receiver's value.
                // Resolving it here by name found a same-named signal of a
                // design instance instead of a subroutine formal.
                let string_method_on_string = matches!(m.as_str(), "len" | "getc" | "substr")
                    && self.expr_is_string_valued(&base_expr);
                if !string_method_on_string {
                    if let Some(an) = self.expr_assoc_name(&base_expr) {
                        if let Some(res) = self.eval_builtin_method(&an, &m, args) {
                            return res;
                        }
                    }
                    // Module/package-scope array (NOT a class-instance member):
                    // `expr_assoc_name` only resolves class members, so a plain
                    // module-level `int aa[string]` / queue / dynamic array would
                    // otherwise fall through to the enum/0 path. Resolve the bare
                    // name and dispatch the builtin directly. (sv_22 assoc num()/size())
                    if let ExprKind::Ident(bh) = &base_expr.kind {
                        let bn = self.resolve_hier_name(bh);
                        if self.is_associative_array(&bn)
                            || self.module.arrays.contains_key(&*bn)
                            || self.module.dynamic_arrays.contains(&*bn)
                        {
                            if let Some(res) = self.eval_builtin_method(&bn, &m, args) {
                                return res;
                            }
                        }
                    }
                }
            }

            // §6.16 string methods (len/size/getc/substr) and §6.16.6
            // (compare/icompare/atoreal) on a hierarchical-ident receiver
            // (`u_if.data.len()`, `top.dut.cfg.substr(0,3)`). They were
            // excluded from `is_array_builtin_method`'s collection block
            // above (a string field is not a collection) and from the
            // no-arg string block below (its `args.is_empty()` gate), so
            // they fell through to the generic tail, which mis-derived the
            // receiver as just `path[0]` instead of the full
            // `path[0..len-1]` (IEEE 1800-2023 §23.6 — the receiver of
            // `a.b.m()` is `a.b`, not `a`; the parser flattens the call
            // into `Ident([a, b, m])`).
            //
            // `len`/`size`/`getc`/`substr` dispatch by the resolved
            // hierarchical name (their handlers read the value via
            // `get_local_signal_or_static`, which finds the hierarchical
            // signal); `compare`/`icompare`/`atoreal` evaluate the receiver
            // by value. The USER-METHOD guard is essential: a class instance
            // whose class defines a same-named method (e.g. a `container`
            // with its own `len()`) must dispatch to the real method, not
            // the string builtin — without it `c.len()` on a container
            // local returned the byte length of the handle instead of
            // calling `container::len`. The guard evaluates the receiver to
            // a handle and asks `class_has_method` (value-based, mirroring
            // the MemberAccess `len`/`size` fallback).
            if len >= 2
                && matches!(
                    path[len - 1].name.name.as_str(),
                    "len" | "size" | "getc" | "substr" | "compare" | "icompare" | "atoreal"
                )
            {
                let m = path[len - 1].name.name.clone();
                let base_expr = self.method_receiver_expr(hier, len - 1);
                let recv = self.eval_expr(&base_expr);
                let recv_h = recv.to_u64().unwrap_or(0) as usize;
                let is_user_method = recv_h != 0
                    && self
                        .heap
                        .get(recv_h)
                        .and_then(|o| o.as_ref())
                        .is_some_and(|i| self.class_has_method(&i.class_name, &m));
                if !is_user_method {
                    // §6.16: `len`/`getc`/`substr` are string-only methods;
                    // the receiver's value is already in hand. Re-resolving
                    // it by NAME found a same-named signal of a design
                    // instance (an interface's `string name`) instead of a
                    // class method's formal `name`, so UVM's resource-name
                    // check read the wrong string.
                    if matches!(m.as_str(), "len" | "getc" | "substr") && !recv.has_xz() {
                        let bytes = recv.sv_string_bytes();
                        let arg = |this: &mut Self, k: usize| -> Option<i64> {
                            args.get(k)
                                .map(|a| this.eval_expr(a).to_i64().unwrap_or(-1))
                        };
                        match m.as_str() {
                            "len" => return Self::int_count(bytes.len() as u64),
                            "getc" => {
                                let i = arg(self, 0).unwrap_or(-1);
                                let b = if i >= 0 {
                                    bytes.get(i as usize).copied().unwrap_or(0)
                                } else {
                                    0
                                };
                                return Self::sv_byte(b);
                            }
                            _ => {
                                let (i, j) =
                                    (arg(self, 0).unwrap_or(-1), arg(self, 1).unwrap_or(-1));
                                let out: &[u8] = if i < 0 || j < i || j as usize >= bytes.len() {
                                    &[]
                                } else {
                                    &bytes[i as usize..=j as usize]
                                };
                                return Self::sv_string_from_bytes(out);
                            }
                        }
                    }
                    if matches!(m.as_str(), "len" | "size" | "getc" | "substr") {
                        if let ExprKind::Ident(bh) = &base_expr.kind {
                            let bn = self.resolve_hier_name(bh);
                            if self.get_local_signal_or_static(&bn).is_some()
                                || self.get_signal_value_by_name(&bn).is_some()
                            {
                                if let Some(res) = self.eval_builtin_method(&bn, &m, args) {
                                    return res;
                                }
                            }
                        }
                        // Fallback: the receiver was already evaluated as
                        // `recv` (e.g. a class-property string `a.str` whose
                        // member name `str` is not a standalone signal).
                        // Compute len/size directly from the string value bytes.
                        if matches!(m.as_str(), "len" | "size") {
                            let bytes = recv.sv_string_bytes();
                            return Self::int_count(bytes.len() as u64);
                        }
                    } else if let Some(v) = self.string_method(&base_expr, &m, args) {
                        return v;
                    }
                }
            }

            // Enum `.name()` / string `.tolower()`/`.toupper()` where the
            // receiver flattened into the ident path (`v.name()` ->
            // Ident([v, name])). The MemberAccess form is handled earlier; this
            // covers the common local-variable case.
            //
            // §6.16 string methods on a flattened receiver — `s.putc(0, "J");`
            // as a statement parses this way, which is why the mutators looked
            // like no-ops even though the MemberAccess path handled them.
            if len >= 2
                && matches!(
                    path[len - 1].name.name.as_str(),
                    "putc"
                        | "itoa"
                        | "hextoa"
                        | "octtoa"
                        | "bintoa"
                        | "realtoa"
                        | "atoreal"
                        | "compare"
                        | "icompare"
                )
            {
                let m = path[len - 1].name.name.clone();
                let base_expr = self.method_receiver_expr(hier, len - 1);
                // Guarded: a user class method (e.g. UVM's inherited
                // `uvm_object::compare`) must NOT be shadowed by the string
                // builtins dispatched by `string_method`.
                if !self.class_expr_has_method(&base_expr, &m) {
                    if let Some(v) = self.string_method(&base_expr, &m, args) {
                        return v;
                    }
                }
            }
            if len >= 2
                && args.is_empty()
                && matches!(
                    path[len - 1].name.name.as_str(),
                    "name"
                        | "tolower"
                        | "toupper"
                        | "atoi"
                        | "atohex"
                        | "atooct"
                        | "atobin"
                        | "first"
                        | "last"
                        | "next"
                        | "prev"
                        | "num"
                )
            {
                let m = path[len - 1].name.name.clone();
                let base_expr = self.method_receiver_expr(hier, len - 1);
                // LRM §6.19.6: enum reflection on a hier-Ident receiver.
                // Same logic as the MemberAccess branch above — but the
                // parser flattens `c.next()` into `Call{Ident([c,next])}`
                // for the common local-var case, so we mirror the
                // resolution here.
                if matches!(m.as_str(), "first" | "last" | "next" | "prev" | "num") {
                    if let Some(tn) = self.get_expr_type_name(&base_expr) {
                        if let Some(members) = self.module.enum_members.get(&tn).cloned() {
                            if members.is_empty() {
                                return Value::zero(32);
                            }
                            let mw = self
                                .module
                                .signals
                                .get(&members[0].0)
                                .map(|s| s.width)
                                .unwrap_or(32);
                            match m.as_str() {
                                "num" => {
                                    return Self::int_count(members.len() as u64);
                                }
                                "first" => {
                                    return Value::from_u64(members[0].1, mw);
                                }
                                "last" => {
                                    return Value::from_u64(members[members.len() - 1].1, mw);
                                }
                                "next" | "prev" => {
                                    let cur = self.eval_expr(&base_expr).to_u64().unwrap_or(0);
                                    let pos = members.iter().position(|(_, v)| *v == cur);
                                    if let Some(p) = pos {
                                        let np = if m == "next" {
                                            if p + 1 >= members.len() { 0 } else { p + 1 }
                                        } else if p == 0 {
                                            members.len() - 1
                                        } else {
                                            p - 1
                                        };
                                        return Value::from_u64(members[np].1, mw);
                                    }
                                    return Value::zero(mw);
                                }
                                _ => {}
                            }
                        }
                    }
                }
                // §15.4.2: `m.num()` on a mailbox handle (flattened to
                // `Ident([m, num])`) is not an enum reflection — resolve the
                // handle and return the queued-message count. Without this,
                // `num` falls into the `atoi`-style else-branch below and
                // returns garbage (the test saw `num()` == 0).
                if m == "num" {
                    let h = self.eval_expr(&base_expr).to_u64().unwrap_or(0) as usize;
                    if let Some(q) = self.mailboxes.get(&h) {
                        return Self::int_count(q.len() as u64);
                    }
                }
                if m == "name" {
                    // Guard: a USER-DEFINED class method named `name()` must
                    // NOT be shadowed by the enum-reflection intercept. If
                    // the receiver's declared type is a class defining `name`,
                    // fall through to normal method dispatch. Without this,
                    // `c.name()` on a class object returns empty (the enum
                    // fallback below always returns ""). Many user classes define
                    // `name()`.
                    let hint = self.get_expr_type_name(&base_expr);
                    let is_class_name = hint
                        .as_deref()
                        .and_then(|tn| self.module.classes.get(tn))
                        .is_some_and(|cd| cd.methods.contains_key("name"));
                    if !is_class_name {
                        let val = self.eval_expr(&base_expr).to_u64().unwrap_or(0);
                        let keys = self.enum_receiver_keys(&base_expr);
                        if let Some(nm) = self.enum_value_name_keyed(val, hint.as_deref(), &keys) {
                            return Value::from_string(&nm);
                        }
                        // Enum value with no matching member: return empty rather
                        // than falling through to an object-handle `.name()`.
                        return Value::from_string("");
                    }
                    // else: class with a user `name()` method — fall through
                    // to the static/virtual method dispatch below.
                } else if (m == "tolower" || m == "toupper")
                    && !self.class_expr_has_method(&base_expr, &m)
                {
                    let s = self.eval_expr(&base_expr).to_sv_string();
                    return Value::from_string(&Self::sv_ascii_case(&s, m == "toupper"));
                } else if matches!(m.as_str(), "atoi" | "atohex" | "atooct" | "atobin")
                    && !self.class_expr_has_method(&base_expr, &m)
                {
                    // atoi / atohex / atooct / atobin only — the enum-method
                    // names (first/last/next/prev/num) that also enter this
                    // block must NOT fall through to the string-to-number
                    // parse: a non-enum receiver (e.g. a class object whose
                    // type defines a user `num()` method) must reach normal
                    // method dispatch below. Without this guard,
                    // `holder::the_pool.num()` parsed the object handle as a
                    // decimal string and returned 0, silently swallowing the
                    // user method call.
                    let s = self.eval_expr(&base_expr).to_sv_string();
                    let radix = match m.as_str() {
                        "atohex" => 16,
                        "atooct" => 8,
                        "atobin" => 2,
                        _ => 10,
                    };
                    return sv_ato_value(&s, radix);
                }
            }

            // Intercept report info and enabled routines
            if len == 1 {
                let name = &path[0].name.name;
                if name.starts_with('$') {
                    return match name.as_str() {
                        "$time" => Value::from_u64(self.time_in_current_unit().round() as u64, 64),
                        "$realtime" => Value::from_f64(self.time_in_current_unit()),
                        "$stime" => Value::from_u64(
                            (self.time_in_current_unit().round() as u64) & 0xFFFF_FFFF,
                            32,
                        ),
                        "$inferred_clock" | "$inferred_disable" if sv_parser::is_sv2023() => {
                            Value::from_u64(0, 1)
                        }
                        "$global_clock" if sv_parser::is_sv2023() => Value::from_u64(0, 1),
                        "$timeunit" if sv_parser::is_sv2023() => {
                            let mut v = Value::from_u64(self.module.timeunit_exp as i64 as u64, 32);
                            v.is_signed = true;
                            v
                        }
                        "$timeprecision" if sv_parser::is_sv2023() => {
                            let mut v =
                                Value::from_u64(self.module.timeprecision_exp as i64 as u64, 32);
                            v.is_signed = true;
                            v
                        }
                        "$rose_gclk" | "$fell_gclk" | "$steady_gclk" | "$changing_gclk"
                        | "$past_gclk" | "$future_gclk"
                            if sv_parser::is_sv2023() =>
                        {
                            Value::from_u64(0, 1)
                        }
                        "$test$plusargs" => {
                            let pat = match args.first().map(|a| &a.kind) {
                                Some(ExprKind::StringLiteral(s)) => s.clone(),
                                Some(_) => self.eval_expr(&args[0]).to_sv_string(),
                                None => String::new(),
                            };
                            Value::from_u64(self.test_plusarg(&pat) as u64, 32)
                        }
                        "$value$plusargs" => self.eval_value_plusargs(args),
                        "$fopen" => self.open_file_handle(args),
                        "$fclose" => self.close_file_handle(args),
                        "$fwrite" => self.write_file_handle_named(args, false, "$write"),
                        "$fdisplay" => self.write_file_handle_named(args, true, "$display"),
                        "$readmemh" => self.read_memory_file(args, 16),
                        "$readmemb" => self.read_memory_file(args, 2),
                        "$display" | "$displayb" | "$displayh" | "$displayo" | "$write"
                        | "$writeb" | "$writeh" | "$writeo" => {
                            self.exec_system_task(name, args);
                            Value::zero(32)
                        }
                        _ => Value::zero(32),
                    };
                }
            }

            // Counterpart of the flattened `...,type_id,create` path
            // (the MemberAccess form is handled in eval_call_inner's call site
            // above). Resolve `C::type_id::create` from `C`'s own registry
            // typedef and construct the registered target class.  When `C` is a
            // typedef alias (e.g. `typedef base_seq#(T) my_sqr`) there is no
            // class entry for `my_sqr`; fall through to `typedef_types` to
            // resolve the alias and instantiate the parameterised target.
            if len >= 3
                && path[len - 1].name.name == "create"
                && path[len - 2].name.name == "type_id"
            {
                let class_name = path[len - 3].name.name.clone();
                // NOTE: the plain (non-parameterized, non-typedef-alias) form
                // `C::type_id::create(...)` is deliberately NOT intercepted
                // here so it falls through to the real UVM factory
                // (`common_type::create` -> `create_*_by_type`), which applies
                // type/instance overrides and bumps each override's `used`
                // counter. A direct `instantiate_class` (the old shortcut)
                // bypassed the factory entirely, so an override on `C` was
                // silently ignored and `factory.print()` reported `used 0`.
                // The MemberAccess-form shortcut above keeps the
                // parameterized `C#(P)::type_id::create` and type-parameter
                // receiver (`T::type_id::create`) direct paths.
                if let Some(DataType::TypeReference {
                    name, type_args, ..
                }) = self.module.typedef_types.get(&class_name).cloned()
                {
                    if let Some(class_def) = self.module.classes.get(&name.name.name).cloned() {
                        let ta: Option<&[Expression]> = if type_args.is_empty() {
                            None
                        } else {
                            Some(&type_args)
                        };
                        return self.instantiate_class_with_type_args(&class_def, args, ta);
                    }
                }
            }

            // §6.19.6: enum method with ARGS through a scoped/flattened path —
            // `P::e.next(1)` flattens to `[P, e, next]`. The no-parens form is
            // handled at the hier-read site; the CALL form fell through to the
            // generic machinery and returned 0. Dispatch on the second-to-last
            // segment exactly like the array-builtin branch below.
            if hier.path.len() >= 2 {
                let m = hier.path.last().unwrap().name.name.as_str();
                if matches!(m, "first" | "last" | "next" | "prev" | "num") {
                    let bare = hier.path[hier.path.len() - 2].name.name.clone();
                    let is_enum_var = self
                        .type_name_of_var(&bare)
                        .map(|tn| self.module.enum_members.contains_key(&tn))
                        .unwrap_or(false)
                        || self.module.enum_members.contains_key(&bare);
                    if is_enum_var {
                        let m = m.to_string();
                        if let Some(v) = self.eval_builtin_method(&bare, &m, args) {
                            return v;
                        }
                    }
                }
            }
            // Built-in array/queue method reached through a scoped or flattened
            // path, e.g. `pkg::arr.size()` flattens to `[pkg, arr, size]` — the
            // array is the second-to-last segment, not `path[0]`. Without this
            // the receiver would wrongly resolve to the package name.
            if hier.path.len() >= 2 {
                let m = hier.path.last().unwrap().name.name.as_str();
                if Self::is_array_builtin_method(m)
                    || matches!(
                        m,
                        "push_back"
                            | "push_front"
                            | "pop_back"
                            | "pop_front"
                            | "insert"
                            | "delete"
                            | "sort"
                            | "rsort"
                            | "reverse"
                            | "shuffle"
                            | "unique"
                    )
                {
                    let arr = hier.path[hier.path.len() - 2].name.name.clone();
                    // `o.q.push_back(x)` flattens to `[o, q, push_back]`: when
                    // `q` is a native collection PROPERTY it lives under
                    // `<handle>#q`, so walk the handle chain to the
                    // second-to-last segment FIRST — a same-named module-scope
                    // collection must not shadow the object's own member.
                    // Without this the fallthrough dispatches on `path[0]` and
                    // pushes onto a phantom queue named after the OBJECT. A
                    // class-handle member (e.g. custom `queue` object) fails
                    // `class_assoc_member`, so user methods still dispatch
                    // below; a package head (`pkg::arr.size()`) resolves no
                    // handle and falls to the plain-name dispatch.
                    if hier.path.len() >= 3 {
                        let head = hier.path[0].name.name.as_str();
                        let mut handle = if head == "this" || head == "super" {
                            self.this_stack.last().copied().flatten().unwrap_or(0)
                        } else {
                            self.eval_ident_handle(head).unwrap_or(0)
                        };
                        for seg in &hier.path[1..hier.path.len() - 2] {
                            if handle == 0 {
                                break;
                            }
                            handle = self
                                .heap
                                .get(handle)
                                .and_then(|o| o.as_ref())
                                .and_then(|i| i.properties.get(&seg.name.name))
                                .and_then(|v| v.to_u64())
                                .unwrap_or(0) as usize;
                        }
                        if handle != 0 {
                            let cn = self
                                .heap
                                .get(handle)
                                .and_then(|x| x.as_ref())
                                .map(|i| i.class_name.clone());
                            if let Some(cn) = cn {
                                if self.class_assoc_member(&cn, &arr) {
                                    let scoped = format!("{}#{}", handle, arr);
                                    if let Some(res) = self.eval_builtin_method(&scoped, m, args) {
                                        return res;
                                    }
                                }
                            }
                        }
                    }
                    // A collection reached through a hierarchical path —
                    // `tif.q.push_back(x)` flattens to Ident([tif, q,
                    // push_back]); the FULL prefix must be tried BEFORE the
                    // bare second-to-last segment, else a stale same-named
                    // local collection (`int q[$]` from another block)
                    // swallows the call (issue-#17 sibling, same shape as
                    // the mailbox fix below).
                    if hier.path.len() >= 3 {
                        let full = hier.path[..hier.path.len() - 1]
                            .iter()
                            .map(|s| s.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join(".");
                        if self.module.arrays.contains_key(&full)
                            || self.module.dynamic_arrays.contains(&full)
                            || self.is_associative_array(&full)
                        {
                            if let Some(res) = self.eval_builtin_method(&full, m, args) {
                                return res;
                            }
                        }
                    }
                    if self.module.arrays.contains_key(&arr)
                        || self.module.dynamic_arrays.contains(&arr)
                        || self.is_associative_array(&arr)
                    {
                        if let Some(res) = self.eval_builtin_method(&arr, m, args) {
                            return res;
                        }
                    }
                }
            }

            if hier.path.len() > 1 {
                let obj_name = &hier.path[0].name.name;
                let method_name = &hier.path.last().unwrap().name.name;

                // `ClassName::static_collection.first(ref k)` flattened into
                // `Ident([class, collection, method])`: when path[0] is a
                // CLASS name (not an object) and path[1] is a static assoc-
                // array collection, route the builtin method to the
                // collection's bare name (static collections are stored
                // globally by bare name). Without this, obj_name resolves to
                // the class name and first()/next() iteration silently fails.
                if hier.path.len() >= 3
                    && self.module.classes.contains_key(obj_name)
                    && self.is_associative_array(&hier.path[1].name.name)
                    && matches!(
                        method_name.as_str(),
                        "first" | "last" | "next" | "prev" | "num" | "size" | "exists" | "delete"
                    )
                {
                    let coll = hier.path[1].name.name.clone();
                    if let Some(res) = self.eval_builtin_method(&coll, method_name, args) {
                        return res;
                    }
                }

                // §18.13 rand_mode / constraint_mode flattened into a hier
                // Ident: `obj.x.rand_mode(0)` parses as Ident([o, x, rand_mode])
                // rather than nested MemberAccess.
                if matches!(method_name.as_str(), "rand_mode" | "constraint_mode") {
                    let mname = method_name.clone();
                    // Resolve obj_name to a heap handle.
                    let h_val = self
                        .get_signal_value_by_name(obj_name)
                        .map(|v| v.to_u64().unwrap_or(0) as usize)
                        .unwrap_or(0);
                    if h_val != 0 && self.heap.get(h_val).and_then(|x| x.as_ref()).is_some() {
                        // Middle segments name the targeted prop/constraint;
                        // empty = whole-object mode.
                        let target_name: Option<String> = if hier.path.len() >= 3 {
                            Some(
                                hier.path[1..hier.path.len() - 1]
                                    .iter()
                                    .map(|s| s.name.name.as_str())
                                    .collect::<Vec<_>>()
                                    .join("."),
                            )
                        } else {
                            None
                        };
                        // §18.5.11: a static constraint block's mode is shared
                        // by every instance of the class — route it to the
                        // class-wide flag, not this instance's set.
                        if mname == "constraint_mode" {
                            let en = if args.is_empty() {
                                None
                            } else {
                                Some(self.eval_expr(&args[0]).to_u64().unwrap_or(0) != 0)
                            };
                            if let Some(v) =
                                self.static_constraint_mode(h_val, target_name.as_deref(), en)
                            {
                                return v;
                            }
                        }
                        if args.is_empty() {
                            let table = if mname == "rand_mode" {
                                &self.rand_mode_disabled
                            } else {
                                &self.constraint_mode_disabled
                            };
                            let disabled = table.get(&h_val).is_some_and(|s| {
                                if let Some(ref n) = target_name {
                                    s.contains(n) || s.contains("*")
                                } else {
                                    !s.is_empty()
                                }
                            });
                            return Value::from_u64(if disabled { 0 } else { 1 }, 32);
                        }
                        let enable = self.eval_expr(&args[0]).to_u64().unwrap_or(0) != 0;
                        self.set_object_random_mode(h_val, mname == "rand_mode", target_name.as_deref(), enable);
                        return Value::zero(32);
                    }
                }

                // Static method call `ClassName::method(args)` when the
                // call flattened into a 2-segment hierarchical Ident.
                if hier.path.len() == 2
                    && !self
                        .local_stack
                        .last()
                        .is_some_and(|m| m.contains_key(obj_name))
                    && !self.signal_name_to_id.contains_key(obj_name.as_str())
                    && !self.signals.contains_key(obj_name)
                {
                    // Type-parameter used as a class name: `Tregistry::get()`.
                    // Resolve the type param to a concrete class/specialization
                    // from the active spec before dispatching.
                    if let Some(resolved) = self.resolve_type_param_binding(obj_name) {
                        // Check if it's a specialized class (e.g.
                        // `object_registry#(base_class,"base_class")`)
                        if let Some((base, sig)) = self.extract_spec_from_string(&resolved) {
                            self.ensure_spec_statics(&base, &sig);
                            let saved = self.current_spec.take();
                            if let Some(cs) = &saved {
                                self.spec_scope_stack.push(cs.clone());
                            }
                            self.current_spec = Some((base.clone(), sig));
                            let m = method_name.clone();
                            let res = self.exec_static_method(&base, &m, args);
                            let had_saved = saved.is_some();
                            self.current_spec = saved;
                            if had_saved {
                                self.spec_scope_stack.pop();
                            }
                            if let Some(v) = res {
                                return v;
                            }
                        } else if self.module.classes.contains_key(&resolved) {
                            let cls = resolved;
                            let m = method_name.clone();
                            if let Some(res) = self.exec_static_method(&cls, &m, args) {
                                return res;
                            }
                        }
                    }
                    if let Some((base, sig)) = self.extract_spec_from_string(obj_name) {
                        self.ensure_spec_statics(&base, &sig);
                        let saved = self.current_spec.take();
                        if let Some(cs) = &saved {
                            self.spec_scope_stack.push(cs.clone());
                        }
                        self.current_spec = Some((base.clone(), sig));
                        let m = method_name.clone();
                        let res = self.exec_static_method(&base, &m, args);
                        let had_saved = saved.is_some();
                        self.current_spec = saved;
                        if had_saved {
                            self.spec_scope_stack.pop();
                        }
                        if let Some(v) = res {
                            return v;
                        }
                    } else if self.module.classes.contains_key(obj_name) {
                        let cls = obj_name.clone();
                        let m = method_name.clone();
                        if let Some(res) = self.exec_static_method(&cls, &m, args) {
                            return res;
                        }
                        if m == "new" {
                            if let Some(cd) = self.module.classes.get(&cls).cloned() {
                                return self.instantiate_class(&cd, args);
                            }
                        }
                    }
                    // §8.25.1 typedef specialization alias:
                    // `AliasT::method(args)` where `AliasT` is a
                    // `typedef Common#(int,"alpha") AliasT;`. Resolve to the
                    // base class and make the specialization active so
                    // value-parameter lookups in the method body bind. Only
                    // genuine parameterized-class specializations are
                    // intercepted (resolve_typedef_spec returns None for plain
                    // aliases / non-class typedefs), preserving prior
                    // fallthrough behavior.
                    if let Some((base, sig)) = self.resolve_typedef_spec(obj_name) {
                        let m = method_name.clone();
                        self.ensure_spec_statics(&base, &sig);
                        let saved = self.current_spec.take();
                        if let Some(cs) = &saved {
                            self.spec_scope_stack.push(cs.clone());
                        }
                        self.current_spec = Some((base.clone(), sig));
                        let res = self.exec_static_method(&base, &m, args);
                        let had_saved = saved.is_some();
                        self.current_spec = saved;
                        if had_saved {
                            self.spec_scope_stack.pop();
                        }
                        if let Some(v) = res {
                            return v;
                        }
                    }
                    // §6.18/§8.25.1: simple class typedef (no specialization
                    // args) — `typedef base alias;` ... `alias::method()`.
                    if let Some(resolved) = self.resolve_simple_typedef_class(obj_name) {
                        let m = method_name.clone();
                        if let Some(res) = self.exec_static_method(&resolved, &m, args) {
                            return res;
                        }
                        if m == "new" {
                            if let Some(cd) = self.module.classes.get(&resolved).cloned() {
                                return self.instantiate_class(&cd, args);
                            }
                        }
                    }
                }

                // §15.3/§15.4: a container reached through a flattened
                // hierarchical path — `tif.ping_mbox.put(x)` parses as
                // Ident([tif, ping_mbox, put]) — never matched the
                // `path[0]`-keyed dispatch below, so the put/get was a
                // silent no-op (issue #17's interface-mailbox test). Resolve
                // the FULL receiver prefix; a mailbox/semaphore handle
                // routes to the handle-based container dispatch. (A blocking
                // get on an empty mailbox never reaches here — the
                // statement-level parking path already handles it.)
                if hier.path.len() >= 3
                    && matches!(
                        method_name.as_str(),
                        "num" | "put" | "get" | "peek" | "try_put" | "try_get" | "try_peek"
                    )
                {
                    let head = hier_prefix(hier);
                    let recv = Expression::new(ExprKind::Ident(head), func.span);
                    let handle = self.eval_expr(&recv).to_u64().unwrap_or(0) as usize;
                    if handle != 0
                        && (self.mailboxes.contains_key(&handle)
                            || self.semaphores.contains_key(&handle))
                    {
                        return self.exec_method_call(handle, method_name, args);
                    }
                }
                // Package-qualified static method call:
                // `pkg::Class::method(args)` flattens to a 3-segment
                // Ident. If path[0] is NOT a local/signal (it's a package
                // or scope name) and path[1] IS a known class, dispatch as
                // a 2-segment `Class::method` static call. Without this,
                // `uvm_pkg::uvm_report_message::new_report_message()` falls
                // through to the heap-walk handler, which tries to look up
                // "uvm_pkg" as a local handle, gets 0, and returns null —
                // which is why UVM macros that use the fully-qualified form
                // silently produced null objects (e.g. the 10print test's
                // msg was never populated).
                if hier.path.len() >= 3 {
                    let pkg = hier.path[0].name.name.as_str();
                    let cls = &hier.path[1].name.name;
                    let mname3 = hier.path.last().unwrap().name.name.clone();
                    // §8.22 nested classes: `A::B::C::method(...)` — the
                    // elaborator registers nested classes under SCOPED keys
                    // (`Outer::Inner`, `Outer::Inner::Inner2`, ...). Resolve
                    // the LONGEST scoped-class prefix of the path (excluding
                    // the trailing method segment) and dispatch the static
                    // there. Only genuine nested-class chains match (the
                    // joined key must exist), so `pkg::Class::method` and
                    // `Class::typedef::method` keep their existing arms
                    // below.
                    if hier.path.iter().all(|s| s.selects.is_empty())
                        && !self.local_stack.last().is_some_and(|m| m.contains_key(pkg))
                        && !self.signal_name_to_id.contains_key(pkg)
                    {
                        for j in (1..hier.path.len() - 1).rev() {
                            let key = hier.path[..=j]
                                .iter()
                                .map(|s| s.name.name.clone())
                                .collect::<Vec<_>>()
                                .join("::");
                            if j >= 2 && self.module.classes.contains_key(&key) {
                                if let Some(res) = self.exec_static_method(&key, &mname3, args) {
                                    return res;
                                }
                            }
                        }
                    }
                    if !self.local_stack.last().is_some_and(|m| m.contains_key(pkg))
                        && !self.signal_name_to_id.contains_key(pkg)
                        && self.module.classes.contains_key(cls)
                    {
                        if let Some(res) = self.exec_static_method(cls, &mname3, args) {
                            return res;
                        }
                        if mname3 == "new" {
                            if let Some(cd) = self.module.classes.get(cls).cloned() {
                                return self.instantiate_class(&cd, args);
                            }
                        }
                    } else if !self.local_stack.last().is_some_and(|m| m.contains_key(pkg))
                        && !self.signal_name_to_id.contains_key(pkg)
                        && self.module.classes.contains_key(pkg)
                    {
                        // §6.18/§8.23: `Class::typedef_alias::method(args)` —
                        // the middle segment is a TYPEDEF declared inside
                        // `Class`, not a class itself. Resolve the alias to
                        // its target class/specialization and dispatch the
                        // static method there — generic for any alias name.
                        // Non-parameterized alias: `typedef base alias;`.
                        if let Some(resolved) = self.resolve_class_member_typedef_class(pkg, cls) {
                            if let Some(res) = self.exec_static_method(&resolved, &mname3, args) {
                                return res;
                            }
                            if mname3 == "new" {
                                if let Some(cd) = self.module.classes.get(&resolved).cloned() {
                                    return self.instantiate_class(&cd, args);
                                }
                            }
                        }
                        // Parameterized alias: `typedef registry#(T,N) alias;`.
                        if let Some((base, sig)) = self.resolve_class_member_typedef_spec(pkg, cls)
                        {
                            self.ensure_spec_statics(&base, &sig);
                            let saved = self.current_spec.take();
                            if let Some(cs) = &saved {
                                self.spec_scope_stack.push(cs.clone());
                            }
                            self.current_spec = Some((base.clone(), sig));
                            let res = self.exec_static_method(&base, &mname3, args);
                            let had_saved = saved.is_some();
                            self.current_spec = saved;
                            if had_saved {
                                self.spec_scope_stack.pop();
                            }
                            if let Some(v) = res {
                                return v;
                            }
                        }
                    }
                }
                // §19.8 `cg_type::get_coverage()` / `cg_type::cp::get_coverage()`
                // flatten to Ident([cg_type, (cp,) get_coverage]).
                if matches!(hier.path.len(), 2 | 3)
                    && hier
                        .path
                        .last()
                        .is_some_and(|s| s.name.name == "get_coverage")
                    && hier.path.iter().all(|s| s.selects.is_empty())
                {
                    let recv = Expression::new(
                        ExprKind::Ident(HierarchicalIdentifier {
                            root: None,
                            path: hier.path[..hier.path.len() - 1].to_vec(),
                            span: hier.span,
                            cached_signal_id: std::cell::Cell::new(None),
                            cached_resolved_name: std::cell::OnceCell::new(),
                        }),
                        func.span,
                    );
                    if let Some((cg, item)) = self.cg_type_receiver(&recv) {
                        let (pct, c, t) = match item {
                            None => self.cg_type_counts(&cg),
                            Some(it) => self.cg_type_item_counts(&cg, &it).unwrap_or((0.0, 0, 0)),
                        };
                        self.cg_assign_counts(args, c, t);
                        return Value::from_f64(pct);
                    }
                }
                // IEEE 1800-2023 §8.15/§13.5.5: a method reached through a
                // flattened multi-segment path — `o.in.getval()` parses as
                // Ident([o, in, getval]). The receiver is the FULL prefix
                // `path[0..len-1]`, NOT `path[0]`: each intermediate segment
                // is a class-handle property that must be dereferenced to
                // reach the object that actually OWNS the method. Without
                // this, the fallthrough below resolved only `path[0]` (the
                // OUTER object), bound `this` to it, and looked the method up
                // there — so `o.in.getval()` ran getval on `o` (Outer has no
                // getval) and silently returned 0; `o.in.setval(x)` wrote
                // nothing. Walk the prefix through the heap to the real owner
                // and dispatch on it. Path[0] may be a local, a signal, or
                // `this`; intermediate segments are heap-object properties.
                // (The mailbox/array-builtin/rand_mode special cases above
                // already `return` for their own >=3-segment shapes, so this
                // only catches ordinary user methods on nested objects.)
                if hier.path.len() >= 3 {
                    let head = hier.path[0].name.name.as_str();
                    let (mut handle, loop_start) = if head == "this" || head == "super" {
                        (self.this_stack.last().copied().flatten().unwrap_or(0), 1)
                    } else if self.module.classes.contains_key(head) {
                        // `ClassName::static_prop.method()`: path[0] is a
                        // CLASS, not an object variable. Resolve the static
                        // property (path[1]) handle via static_prop_key, then
                        // walk path[2..] (skip the already-resolved path[1]).
                        // Without this, methods called on objects held in
                        // static properties never bind `this` and silently
                        // no-op — instance member writes are lost.
                        let h = self
                            .static_prop_key(head, &hier.path[1].name.name)
                            .and_then(|k| self.class_statics.get(&k))
                            .and_then(|v| v.to_u64())
                            .map(|h| h as usize)
                            .unwrap_or(0);
                        (h, 2)
                    } else {
                        (self.eval_ident_handle(head).unwrap_or(0), 1)
                    };
                    let last_seg = hier.path.len() - 1;
                    for (si, seg) in hier.path[loop_start..last_seg].iter().enumerate() {
                        if handle == 0 {
                            break;
                        }
                        // §19.8: `cg.cp.get_coverage()` — the walk reached a
                        // covergroup and the next segment names one of its
                        // coverpoints or crosses.
                        if loop_start + si + 1 == last_seg
                            && matches!(method_name.as_str(), "get_coverage" | "get_inst_coverage")
                        {
                            if let Some(idx) = self.cg_index(handle) {
                                if let Some((pct, c, t)) =
                                    self.cg_item_query(idx, &seg.name.name, method_name)
                                {
                                    self.cg_assign_counts(args, c, t);
                                    return Value::from_f64(pct);
                                }
                            }
                        }
                        handle = self
                            .heap
                            .get(handle)
                            .and_then(|o| o.as_ref())
                            .and_then(|i| i.properties.get(&seg.name.name))
                            .and_then(|v| v.to_u64())
                            .unwrap_or(0) as usize;
                    }
                    // The walked-to property may be a covergroup instance
                    // (`e.cg.sample()`, `e.cg.get_inst_coverage()`).
                    if let Some(idx) = self.cg_index(handle) {
                        return self.exec_cg_method_call(idx, method_name, args);
                    }
                    // The walked-to receiver may be an opaque PROCESS handle
                    // (≥ PROCESS_HANDLE_BASE) — e.g. `obj.watcher_proc.kill()`.
                    // exec_method_call intercepts kill/await/suspend/resume on
                    // process tokens; without this the guard below dropped the
                    // call (handle >= heap.len()) and a killed watcher kept
                    // running. §5.7.1 process methods.
                    if handle != 0
                        && ((handle < self.heap.len() && self.heap[handle].is_some())
                            || handle as u64 >= PROCESS_HANDLE_BASE)
                    {
                        return self.exec_method_call(handle, method_name, args);
                    }
                }
                let obj_val = if let Some(locals) = self.local_stack.last() {
                    locals.get(obj_name).cloned().or_else(|| {
                        if let Some(&id) = self.signal_name_to_id.get(obj_name.as_str()) {
                            Some(self.signal_table[id].clone())
                        } else {
                            self.signals.get(obj_name).cloned()
                        }
                    })
                } else {
                    if let Some(&id) = self.signal_name_to_id.get(obj_name.as_str()) {
                        Some(self.signal_table[id].clone())
                    } else {
                        self.signals.get(obj_name).cloned()
                    }
                };
                // A class instance's USER method ALWAYS takes precedence over a
                // same-named collection builtin: custom pool/queue classes define real
                // exists()/num()/get()/push_back()/size()/... methods. Without
                // this, `pool.exists(k)` / `q.push_back(x)` dispatched the builtin
                // (a collection op on the handle itself → no-op/0), so
                // pool and resource queue ops all silently failed. Gate on the
                // receiver being a LIVE class instance defining the method — a
                // non-builtin name makes eval_builtin_method return None anyway,
                // so this only changes the builtin-name-collision cases.
                let prefer_user_method = obj_val
                    .as_ref()
                    .and_then(|v| v.to_u64())
                    .map(|h| h as usize)
                    .filter(|&h| h != 0 && h < self.heap.len())
                    .and_then(|h| self.heap[h].as_ref())
                    .is_some_and(|i| self.class_has_method(&i.class_name, method_name));
                if !prefer_user_method {
                    if let Some(res) = self.eval_builtin_method(obj_name, method_name, args) {
                        return res;
                    }
                }
                if let Some(v) = obj_val {
                    let handle = v.to_u64().unwrap_or(0) as usize;
                    // LRM §8.7/§8.23: a STATIC method always dispatches to the
                    // receiver's class with NO instance dereference — even when
                    // the receiver handle is null (how `uvm_create` obtains the
                    // type wrapper: `tr.get_type()` before `tr` is allocated).
                    // Static methods are not in `cd.methods`, so the instance
                    // dispatch below would miss them and return null. Prefer the
                    // RUNTIME class of a live receiver, else the DECLARED type.
                    let decl_cls_opt = self.class_of_var(obj_name).or_else(|| {
                        self.this_stack
                            .last()
                            .copied()
                            .flatten()
                            .and_then(|th| self.prop_class_type(th, obj_name))
                    });
                    let base_cls = if handle != 0 {
                        self.heap
                            .get(handle)
                            .and_then(|o| o.as_ref())
                            .map(|i| i.class_name.clone())
                            .or_else(|| decl_cls_opt.clone())
                    } else {
                        decl_cls_opt
                    };
                    if let Some(decl_full) = base_cls {
                        let base_cls =
                            decl_full.split('#').next().unwrap_or(&decl_full).to_string();
                        let m = method_name.clone();
                        if self.is_static_method(&base_cls, &m) {
                            // §8.25: a static method called through a NULL
                            // receiver whose declared type is a SPECIALIZATION
                            // (`req::get_type()` where req is declared
                            // `C#(T)` — `uvm_create_on`'s shape) runs in that
                            // specialization's context: the body's bare
                            // self-references (the `C` type argument of a
                            // member typedef like
                            // `uvm_object_registry#(C,"C") type_id`, written by
                            // `uvm_object_utils(C)`) denote the CURRENT
                            // specialization, not the default. The live-object
                            // spec recovery in `exec_static_method_via_object`
                            // cannot see a null handle, so seed `current_spec`
                            // from the DECLARED type here.
                            let saved_spec = if handle == 0 {
                                self.spec_for_null_receiver(Some(obj_name.as_str()), &decl_full)
                            } else {
                                None
                            };
                            let saved = saved_spec
                                .map(|s| std::mem::replace(&mut self.current_spec, Some(s)));
                            let res =
                                self.exec_static_method_via_object(&base_cls, &m, args, handle);
                            if let Some(prev) = saved {
                                self.current_spec = prev;
                            }
                            if let Some(res) = res {
                                return res;
                            }
                        }
                    }
                    if handle != 0 {
                        // IEEE 1800-2023 §9.7 + §18.14: `process::self()` yields an
                        // opaque token, NOT a heap object, so the heap gate below
                        // can never fire for it — `p.srandom(seed)` /
                        // `p.get_randstate()` / `p.set_randstate(s)` on a
                        // `process` variable (which parses as the flattened
                        // Ident([p, method]) call shape) silently returned 0.
                        // Route them to the per-process random stream.
                        if handle as u64 >= PROCESS_HANDLE_BASE {
                            if let Some(res) =
                                self.exec_rand_state_method(handle, method_name, args)
                            {
                                return res;
                            }
                            // IEEE 1800-2023 §9.7: process-class methods
                            // (kill/await/suspend/resume/status) on a process
                            // handle. await() blocking is handled at the
                            // statement level (needs the continuation); the
                            // other four are fire-and-forget.
                            if let Some(pid) = Self::proc_handle_to_pid(handle as u64) {
                                match method_name.as_str() {
                                    "status" => {
                                        return Value::from_u64(self.proc_status(pid) as u64, 32);
                                    }
                                    "kill" => {
                                        self.proc_kill(pid);
                                        return Value::zero(32);
                                    }
                                    "suspend" => {
                                        self.proc_suspend(pid);
                                        return Value::zero(32);
                                    }
                                    "resume" => {
                                        self.proc_resume(pid);
                                        return Value::zero(32);
                                    }
                                    _ => {}
                                }
                            }
                        }
                        if let Some(idx) = self.cg_index(handle) {
                            return self.exec_cg_method_call(idx, method_name, args);
                        }
                        if handle < self.heap.len() && self.heap[handle].is_some() {
                            // §8.20: non-virtual methods bind to the DECLARED
                            // type of the receiver (see nonvirtual_target_class;
                            // the flattened `Ident([obj, method])` shape is the
                            // common parse for `obj.m()`, so the MemberAccess-
                            // site check alone never fired).
                            if let Some(decl_cls) = self.class_of_var(obj_name) {
                                let decl_base =
                                    decl_cls.split('#').next().unwrap_or(&decl_cls).to_string();
                                if let Some(target) =
                                    self.nonvirtual_target_class(&decl_base, method_name)
                                {
                                    let runtime = self
                                        .heap
                                        .get(handle)
                                        .and_then(|o| o.as_ref())
                                        .map(|i| i.class_name.clone());
                                    // §8.20: a non-virtual method binds to the
                                    // DECLARED type of the receiver. `decl_cls`
                                    // here can be a STALE value from the flat
                                    // `var_class_types` map when a same-named
                                    // local in another scope (e.g. a UVM
                                    // method's `uvm_factory f`) overwrote the
                                    // user variable's recorded type (here
                                    // `uvm_factory` polluted to
                                    // `uvm_default_report_server`). Binding
                                    // then dispatches `f.print()` to an
                                    // unrelated class's `print` (e.g.
                                    // `uvm_object`), not the object's real
                                    // method. Only honor the resolved target
                                    // when it is the runtime object's class or
                                    // an ANCESTOR of it; otherwise the declared
                                    // type is bogus and the runtime (virtual)
                                    // dispatch through the live class is
                                    // correct.
                                    let target_on_chain = runtime.as_deref().is_some_and(|rt| {
                                        target == rt.to_string() || self.class_is_a(rt, &target)
                                    });
                                    if target_on_chain
                                        && runtime.as_deref() != Some(target.as_str())
                                    {
                                        return self.exec_method_in_class_hierarchy(
                                            handle,
                                            &target,
                                            method_name,
                                            args,
                                        );
                                    }
                                }
                            }
                            return self.exec_method_call(handle, method_name, args);
                        }
                    }
                }
            }
            // LRM §25.5.4: a subroutine of an interface INSTANCE called
            // through the instance — `bus.snoop()` flattens to
            // Ident([bus, snoop]) while elaboration inlined the
            // interface's function under the instance-prefixed key
            // ("bus.snoop"). Look the FULL dotted name up; the inlined
            // body still names interface members bare (`data`), so run
            // it with the instance path as the name-resolve hint (the
            // sibling-scope mechanism scoped hierarchical reads already
            // use) so `data` resolves to `bus.data`. A leading
            // virtual-interface alias (`vif.snoop()`, LRM §25.9) is
            // rewritten to its bound instance first, giving per-instance
            // dispatch when the vif is re-bound.
            if hier.path.len() >= 2 && hier.path.iter().all(|s| s.selects.is_empty()) {
                let head = hier.path[0].name.name.as_str();
                // §25.9: rebase has already resolved a rooted interface
                // receiver. Reapplying a same-named class alias would expand
                // `sys_if.port.get_tx()` to `sys_if.port.port.get_tx()`.
                let head_owned = if hier.root.is_none() {
                    self.iface_alias_for(head)
                } else {
                    None
                }
                .unwrap_or_else(|| head.to_string());
                let mut segs: Vec<&str> = vec![head_owned.as_str()];
                segs.extend(hier.path[1..].iter().map(|s| s.name.name.as_str()));
                let full = segs.join(".");
                // Nested cross-module call: `l.deep()` invoked from inside a
                // subroutine already running in scope `m` flattens to
                // `m.l.deep`. When the bare dotted name is not itself a known
                // subroutine, retry with the caller's resolve-hint scope
                // prepended so the enclosing instance is honored.
                // §23.8: and from there upward through each enclosing scope
                // (`sib.report()` in a child names its parent's sibling).
                let full = self.scoped_subroutine_key(&full).unwrap_or(full);
                let scope = full.rsplit_once('.').map(|(s, _)| s.to_string());
                // Refcounted declaration (see `fn_decl_rc`): this arm cloned the
                // whole FunctionDeclaration AST on every scoped call.
                if let Some(fd) = self.fn_decl_rc(&full) {
                    let saved = self.name_resolve_hint.borrow().clone();
                    *self.name_resolve_hint.borrow_mut() = scope.clone();
                    // §20.3: `$time`/`$realtime`/`%m` inside a subroutine reference
                    // the module where it is DEFINED — i.e. the instance we
                    // dispatched into — not the caller. Point the timescale scope
                    // at that instance for the duration of the body.
                    let saved_ts = self.timescale_scope_override.take();
                    self.timescale_scope_override = scope.clone();
                    let r = self.exec_function_call(&fd, args);
                    self.timescale_scope_override = saved_ts;
                    *self.name_resolve_hint.borrow_mut() = saved;
                    return r;
                }
                if let Some(td) = self.task_decl_rc(&full) {
                    let saved = self.name_resolve_hint.borrow().clone();
                    *self.name_resolve_hint.borrow_mut() = scope.clone();
                    let saved_ts = self.timescale_scope_override.take();
                    self.timescale_scope_override = scope.clone();
                    self.exec_task_call(&td, args);
                    self.timescale_scope_override = saved_ts;
                    *self.name_resolve_hint.borrow_mut() = saved;
                    return Value::zero(32);
                }
                // A METHOD on an object reached by a hierarchical path
                // (`u_w.p.peek()`, the UVM-MS proxy pattern of issue #155):
                // the callee parses as one identifier path, no subroutine
                // has that flat name, and the call used to fall through the
                // remaining fallbacks to a silent 0. Evaluate the prefix path
                // as a variable; a live class handle takes the call.
                {
                    let recv = hier_prefix(hier);
                    let recv_expr = Expression::new(ExprKind::Ident(recv), func.span);
                    let h = self.eval_expr(&recv_expr).to_u64().unwrap_or(0) as usize;
                    if h != 0
                        && (self.heap.get(h).and_then(|o| o.as_ref()).is_some()
                            || h as u64 >= PROCESS_HANDLE_BASE)
                    {
                        let mname = hier.path.last().unwrap().name.name.clone();
                        return self.exec_method_call(h, &mname, args);
                    }
                    // §8.4: `c.m()` on a class variable that holds null. The
                    // remaining fallbacks (static call, constructor, DPI,
                    // builtin) cannot take it, so it returned a silent 0 and
                    // ran nothing; the reference simulator reports the null
                    // dereference. A static method never needs the object.
                    if h == 0
                        && hier.path.len() == 2
                        && let Some(decl) = self.class_of_var(&hier.path[0].name.name)
                    {
                        let base = decl.split('#').next().unwrap_or(&decl).to_string();
                        let mname = hier.path[1].name.name.clone();
                        if self.class_has_method(&base, &mname)
                            && !self.is_static_method(&base, &mname)
                        {
                            self.report_null_handle_call(&mname);
                            return Value::zero(32);
                        }
                    }
                    // A covergroup of another instance (`u0.cg.sample()`).
                    if let Some(idx) = self.cg_index(h) {
                        let mname = hier.path.last().unwrap().name.name.clone();
                        return self.exec_cg_method_call(idx, &mname, args);
                    }
                }
            }
            // Handle static/constructor call: class_name::f() or new()
            let name = &hier.path.last().unwrap().name.name;
            if let Some(class_def) = self.module.classes.get(name).cloned() {
                return self.instantiate_class(&class_def, args);
            }
            if let Some(cg_def) = self.covergroup_def_for(name) {
                return self.instantiate_covergroup(&cg_def, args);
            }
            // DPI import call — through an instance path (`u_a.c_fn()`), the
            // call runs in that instance's scope (§35.5.3).
            let via: Option<String> = (hier.path.len() > 1)
                .then(|| {
                    hier.path[..hier.path.len() - 1]
                        .iter()
                        .map(|s| s.name.name.as_str())
                        .collect::<Vec<_>>()
                        .join(".")
                })
                .filter(|p| self.module.instances.iter().any(|i| &i.path == p));
            if let Some(v) = self.exec_dpi_import_call_in(name, args, via.as_deref()) {
                return v;
            }
            // Unqualified call inside a class method — resolve to a method
            // of the current class context. Dispatches virtually through
            // `this` when in an instance method, or as a static method
            // when there is no instance handle.
            if let Some(Some(ctx)) = self.class_context_stack.last().cloned() {
                // A bare `randomize()`/`srandom()`/`get_randstate()`/
                // `set_randstate()` inside a class method is an implicit
                // built-in, not a declared method — `class_has_method` reports
                // false for it (the source never lists it), so the bare call
                // used to fall through and silently return 0, while the
                // qualified `this.randomize()` (member-access path) reached
                // `exec_method_call` -> the §18.11 solver correctly. Route the
                // built-in names through `exec_method_call` too (it intercepts
                // them ahead of user dispatch), restoring bare `randomize()`
                // inside `new`/functions (the reference simulator honors it).
                if self.class_has_method(&ctx, name)
                    || matches!(
                        name.as_str(),
                        "randomize" | "srandom" | "get_randstate" | "set_randstate"
                    )
                {
                    let this_handle = self.this_stack.last().copied().flatten().unwrap_or(0);
                    if this_handle != 0 {
                        // IEEE 1800-2017 §8.20: a NON-virtual method (which
                        // includes every `local` method — `local` methods
                        // cannot be overridden, so a subclass same-name method
                        // HIDES rather than overrides) binds STATICALLY to the
                        // lexical class that owns the call site, NOT the
                        // runtime class of `this`. Without this, a per-class
                        // `local` helper that shares a name across an inheritance
                        // chain (e.g. a field-op dispatcher generated by class
                        // utility macros) was dispatched virtually: a call to it
                        // inside the BASE class's body ran the DERIVED class's
                        // copy, so base-class field macros never executed for a
                        // subclass instance. That broke the whole field-op chain
                        // (print/copy/compare/set): inherited fields were
                        // invisible and field-driven auto-configuration of
                        // inherited fields silently no-op'd. `ctx` is the
                        // lexical class (pushed by
                        // exec_method_in_class_hierarchy / super-call), so this
                        // is the unqualified analogue of the qualified
                        // `obj.m()` non-virtual binding above.
                        if let Some(target) = self.nonvirtual_target_class(&ctx, name) {
                            let runtime = self
                                .heap
                                .get(this_handle)
                                .and_then(|o| o.as_ref())
                                .map(|i| i.class_name.clone());
                            if runtime.as_deref() != Some(target.as_str()) {
                                return self.exec_method_in_class_hierarchy(
                                    this_handle,
                                    &target,
                                    name,
                                    args,
                                );
                            }
                        }
                        return self.exec_method_call(this_handle, name, args);
                    }
                    if let Some(v) = self.exec_static_method(&ctx, name, args) {
                        return v;
                    }
                }
            }
            // §26.3: `pkg::f(...)` — the parser flattens the scope chain, so it
            // arrives here as Ident([pkg, f]) and used to be resolved by the
            // bare leaf below. Package subroutines are hoisted under their BARE
            // name, so two packages declaring the same one collided and a
            // single body answered for both spellings. Elaboration gives each
            // colliding declaration a `pkg::name` key — prefer it. Either way
            // the body runs with its own package as the scope its free names
            // resolve in (`pending_pkg_scope`).
            if hier.path.len() == 2
                && hier.path.iter().all(|s| s.selects.is_empty())
                && self.module.packages.contains(&hier.path[0].name.name)
                && !self
                    .signal_name_to_id
                    .contains_key(hier.path[0].name.name.as_str())
                && !self.signals.contains_key(&hier.path[0].name.name)
            {
                let pkg = hier.path[0].name.name.clone();
                let qual = format!("{}::{}", pkg, name);
                let fd = self
                    .module
                    .functions
                    .get(&qual)
                    .or_else(|| self.module.functions.get(name))
                    .cloned();
                if let Some(fd) = fd {
                    self.pending_pkg_scope = Some(pkg);
                    return self.exec_function_call(&fd, args);
                }
                let td = self
                    .module
                    .tasks
                    .get(&qual)
                    .or_else(|| self.module.tasks.get(name))
                    .cloned();
                if let Some(td) = td {
                    self.pending_pkg_scope = Some(pkg);
                    self.exec_task_call(&td, args);
                    return Value::zero(32);
                }
                // §11.13: `pkg::my_let(...)`.
                let ld = self
                    .module
                    .lets
                    .get(&qual)
                    .or_else(|| self.module.lets.get(name))
                    .cloned();
                if let Some(ld) = ld {
                    return self.exec_let_call(&ld, args);
                }
            }
            // §26.3: a BARE call inside a package subroutine binds to the
            // enclosing package's own subroutine first — `p2::twice` calling
            // `next_step` must reach p2's copy, not whichever package's body
            // holds the shared bare key. Only contested names have a qualified
            // entry; an uncontested bare key already is the right body.
            if hier.path.len() == 1
                && !self.pkg_ambiguous_names.is_empty()
                && self.pkg_ambiguous_names.contains(name.as_str())
            {
                // Outside a package subroutine, the calling design unit's
                // import of the name decides (§26.3: `import pc::who;` binds
                // `who()` to pc's even when another package declares one).
                let owner = match self.pkg_scope_stack.last() {
                    Some(Some(pkg)) => Some(pkg.clone()),
                    // Only a call made directly in a design unit's procedural
                    // code consults its imports: inside any subroutine or
                    // method the lookup below would run per call, and library
                    // code (UVM) calls contested names on hot paths.
                    _ if self.class_context_stack.last().is_some() => None,
                    _ => {
                        let n = name.to_string();
                        self.scope_import_pkg(&n, &|pkg| {
                            let q = format!("{}::{}", pkg, n);
                            self.module.functions.get(&q).is_some()
                                || self.module.tasks.contains_key(&q)
                        })
                    }
                };
                if let Some(pkg) = owner {
                    let qual = format!("{}::{}", pkg, name);
                    if let Some(fd) = self.fn_decl_rc(&qual) {
                        self.pending_pkg_scope = Some(pkg);
                        return self.exec_function_call(&fd, args);
                    }
                    if let Some(td) = self.task_decl_rc(&qual) {
                        self.pending_pkg_scope = Some(pkg);
                        self.exec_task_call(&td, args);
                        return Value::zero(32);
                    }
                }
            }
            // §23.8: a bare call in a method of a class declared INSIDE a
            // module names that module's subroutine — the instance the method
            // resolves in (the resolve hint: the object's creation scope) —
            // before any same-named one of the top or the compilation unit.
            if hier.path.len() == 1 {
                if let Some(full) = self.class_module_subroutine_key(name) {
                    return self.call_scoped_subroutine(&full, args);
                }
            }
            // Module-level function call
            if let Some(fd) = self.fn_decl_rc(name) {
                self.pending_pkg_scope = self.bare_call_pkg_scope(name, hier.path.len());
                return self.exec_function_call(&fd, args);
            }
            // Module-level let call
            if let Some(ld) = self.module.lets.get(name).cloned() {
                return self.exec_let_call(&ld, args);
            }
            // Module-level task call
            if let Some(td) = self.task_decl_rc(name) {
                self.pending_pkg_scope = self.bare_call_pkg_scope(name, hier.path.len());
                self.exec_task_call(&td, args);
                return Value::zero(32);
            }
            // A module-class method naming an instance subroutine that
            // resolved nowhere: report the skipped call instead of dropping
            // it silently.
            if hier.path.len() == 1
                && self.module_class_method_active()
                && self
                    .class_member_names()
                    .subroutine_suffixes
                    .contains(name.as_str())
            {
                self.warn_system_task_once(
                    &format!("call:{}", name),
                    &format!(
                        "Warning: call to '{}' in a method of a class declared in a module \
                         resolved to no subroutine of its instance; call skipped",
                        name
                    ),
                );
            }
        }
        Value::zero(32)
    }
}
