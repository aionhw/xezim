//! Simulator type metadata.

use super::*;

impl Simulator {
    /// Resolve a VarDecl's declared `DataType` to the class (or class TYPE
    /// parameter) it names — the same resolution the interpreted VarDecl
    /// exec applies before recording (§8.25 specialization via the active
    /// `type_bindings` / `current_spec`, then a typedef/alias, then the
    /// bare name).
    pub(super) fn derive_var_decl_class_name(
        &self,
        data_type: &crate::ast::types::DataType,
    ) -> Option<String> {
        if let crate::ast::types::DataType::TypeReference { name, .. } = data_type {
            // §8.25: the declared type may be a TYPE PARAMETER of the
            // enclosing parameterized class — resolve it through the active
            // specialization FIRST so the recorded type is the bound class,
            // not the literal parameter name.
            if let Some(bound) = self.resolve_type_param_binding(&name.name.name) {
                return Some(bound);
            }
            // Resolve a typedef/scoped alias (e.g.
            // `uvm_resource_types::rsrc_q_t`) to its concrete class; fall
            // back to the bare name otherwise.
            return Some(
                self.resolve_typeref_class_name(name)
                    .unwrap_or_else(|| name.name.name.clone()),
            );
        }
        None
    }

    /// Record ONLY the type metadata a VarDecl exec writes — the class type
    /// (frame overlay + `var_class_types`), `#()` type args
    /// (`var_type_args`), container kind (`var_container_types`), and the
    /// typedef/enum name (`var_typedef_types` + frame overlay). Shared by
    /// the interpreted decl path and the compiled-method entry pre-record
    /// (class-perf Step 9c) so `$cast`/`new` see the SAME type picture on
    /// both paths. `class_name` is `derive_var_decl_class_name(data_type)`;
    /// the interpreter passes its own already-derived binding.
    pub(super) fn record_var_decl_type_metadata(
        &mut self,
        data_type: &crate::ast::types::DataType,
        name: &str,
        class_name: Option<&str>,
    ) {
        // Record a class-typed local's type so a later
        // separate `name = new();` knows what to construct.
        // Covergroup-typed locals (§19.8) are recorded too:
        // get_expr_type_name consults this map, and both the
        // bare `c = new;` and `c = new();` assignment paths
        // already route a covergroup type name to
        // instantiate_covergroup — they just never saw it
        // for a procedural local.
        if let Some(cn) = class_name {
            // §8.25: `class_name` may be a resolved
            // VALUE-PARAMETERIZED specialization (e.g. `T me;`
            // where T → `base#(1)`), which is NOT a bare key in
            // `module.classes`. Accept it when its base class
            // is real, so the var's type is recorded and
            // `$cast`/`new` resolve the specialization later.
            let cn_is_class = self.module.classes.contains_key(cn)
                || self.module.covergroups.contains_key(cn)
                || (cn.contains('#')
                    && self
                        .module
                        .classes
                        .contains_key(cn.split('#').next().unwrap_or(cn)));
            if cn_is_class {
                self.record_local_class_type(name, cn);
                self.var_class_types
                    .insert(name.to_string(), cn.to_string());
                // A typedef'd local (e.g. `table_q_t rq` where
                // `table_q_t = shared#(Foo[$])`) resolves to
                // `cn` but hides the type_args inside the
                // typedef chain. Record them so a separate
                // `rq = new()` constructs the right
                // specialization with type_bindings populated
                // (§8.25 — otherwise `T` stays unbound).
                if let crate::ast::types::DataType::TypeReference { name: tref, .. } = data_type {
                    if let Some((_, Some(ta))) = self.resolve_typeref_class_with_type_args(tref) {
                        self.var_type_args.insert(name.to_string(), ta);
                    }
                }
            } else if self.resolve_type_param_binding(cn).is_some() {
                // `cn` is a class TYPE parameter (e.g. `T obj;`
                // inside a parameterized-class method). Record
                // the param name so a separate `obj = new()`
                // resolves it to the concrete class.
                //
                // `resolve_type_param_binding` covers BOTH the
                // instance path (a method's `this` instance
                // `type_bindings`) AND the static path (the
                // active specialization `current_spec`). The
                // latter matters for STATIC methods of a
                // parameterized class — e.g.
                // `callbacks#(T,CB)::get_first` declares
                // `CB cb;` with no `this`. Without the static
                // branch, `cb` was never registered, so a bare
                // `$cast(cb, ...)` assignment and any later
                // `new()` could not resolve CB, and the return
                // value came back null to the caller.
                self.record_local_class_type(name, cn);
                self.var_class_types
                    .insert(name.to_string(), cn.to_string());
            }
        }
        // Record a parameterized local's declared `#(...)` type
        // args so a SEPARATE `name = new(...)` constructs the right
        // specialization (`resource#(T) r; r=new`).
        // A typedef'd local (e.g. `table_q_t rq` where `table_q_t =
        // shared#(Foo[$])`) has EMPTY explicit type_args but
        // the args are hidden in the typedef chain — resolve
        // them so the new() populates type_bindings (§8.25).
        // Clear a stale same-named entry when this decl has none.
        let resolved_ta =
            if let crate::ast::types::DataType::TypeReference { name: tref, .. } = data_type {
                self.resolve_typeref_class_with_type_args(tref)
                    .and_then(|(_, ta)| ta)
            } else {
                None
            };
        if let crate::ast::types::DataType::TypeReference { type_args, .. } = data_type {
            if !type_args.is_empty() {
                self.var_type_args
                    .insert(name.to_string(), type_args.clone());
            } else if let Some(ta) = resolved_ta {
                self.var_type_args.insert(name.to_string(), ta);
            } else {
                self.var_type_args.remove(name);
            }
        } else {
            self.var_type_args.remove(name);
        }
        // §15.3/§15.4: record a mailbox/semaphore-typed local so
        // a later separate `name = new(bound)` allocates the right
        // container. Clear any stale same-named entry from another
        // frame (var_container_types is a flat map like
        // var_class_types) when this decl isn't a container.
        match class_name.and_then(Self::container_base) {
            Some(k) => {
                self.var_container_types
                    .insert(name.to_string(), k.to_string());
            }
            None => {
                self.var_container_types.remove(name);
            }
        }
        // LRM §6.19.6: track typedef-typed locals so
        // `local.next()`/`.first()`/etc. find their
        // enum-member list. We extract the bare type
        // name from a TypeReference data_type. Gated on
        // `plain_class_ref != Some(false)` so this recorder
        // matches the interpreter's inline recording in
        // `exec_stmt_var_decl` byte-for-byte (a name the
        // class tables mark as a param/non-class skips it).
        if let (false, crate::ast::types::DataType::TypeReference { name: tref, .. }) =
            (self.plain_class_ref(data_type) == Some(false), data_type)
        {
            let tn = tref.name.name.clone();
            // A local declared with a class TYPE PARAMETER
            // (`T e;` in a parameterized class) isn't in the
            // flat typedef/enum tables under `T`. Resolve the
            // active specialization's binding (T -> e_t) so
            // `e.first()/.next()/.last()/.prev()/.num()/`
            // names find the CONCRETE enum-member list
            // (else they fall through to a generic 0).
            let tn = if self.module.enum_members.contains_key(&tn)
                || self.module.typedefs.contains_key(&tn)
                || self.module.typedef_types.contains_key(&tn)
            {
                tn
            } else if self.current_spec.is_some() {
                self.resolve_type_param_binding(&tn).unwrap_or_else(|| tn)
            } else {
                tn
            };
            if self.module.enum_members.contains_key(&tn)
                || self.module.typedefs.contains_key(&tn)
                || self.module.typedef_types.contains_key(&tn)
            {
                self.var_typedef_types.insert(name.to_string(), tn.clone());
                // Record the frame-scoped typedef too so a
                // method-local enum type is resolvable without
                // the flat `var_class_types`/`var_typedef_types`
                // collision across unrelated frames (a same-
                // named formal `VISITOR v` in another method
                // must not shadow this `uvm_verbosity v;`
                // local). `local_typedef_type_of` reads the
                // innermost active frame. §6.19.6.
                self.record_local_typedef_type(name, &tn);
            }
        }
    }

    /// Innermost-first overlay lookup for a frame-local's class type.
    /// When inside a class method (`method_local_base` is populated), only
    /// scans frames belonging to THIS method so a caller's same-named local
    /// does not shadow an instance property of `this`.
    pub(super) fn local_class_type_of(&self, name: &str) -> Option<String> {
        let frames = if let Some(&base) = self.method_local_base.last() {
            self.local_type_stack.get(base..).unwrap_or(&[])
        } else {
            &self.local_type_stack[..]
        };
        frames.iter().rev().find_map(|(c, _)| c.get(name).cloned())
    }

    /// Innermost-first overlay lookup for a frame-local's declared
    /// typedef/enum type. Mirrors `local_class_type_of` for the typedef
    /// flavor. Kept frame-scoped so an unrelated method's same-named
    /// formal/local never shadows the currently executing method's own
    /// declaration (`local_typedef_type_of` starts from the top of
    /// `local_type_stack`, which holds only the ACTIVE call frames).
    pub(super) fn local_typedef_type_of(&self, name: &str) -> Option<String> {
        let frames = if let Some(&base) = self.method_local_base.last() {
            self.local_type_stack.get(base..).unwrap_or(&[])
        } else {
            &self.local_type_stack[..]
        };
        frames.iter().rev().find_map(|(_, t)| t.get(name).cloned())
    }

    /// Push a local frame, keeping the type overlay in lockstep.
    /// Monotonic frame-generation source for `local_gen_stack`. Each pushed
    /// frame slot gets a distinct generation so separate activations that
    /// reuse a slot index remain distinguishable (regression 3627).
    pub(super) fn next_frame_gen(&mut self) -> u64 {
        self.frame_gen += 1;
        self.frame_gen
    }

    /// Run a `StmtFallback` that names register-backed locals: they are
    /// bound in a fresh interpreter frame for the statement's duration and
    /// copied back to their registers afterwards (see `FbLocal`). A fresh
    /// frame, not the caller's: a compiled block has no interpreter frame of
    /// its own, and one that runs nested (a comb entry settling inside a
    /// task) must not see — or write — the task's locals.
    #[inline(never)]
    pub(super) fn fb_exec_stmt_with_locals(
        &mut self,
        locals: &[super::bytecode::FbLocal],
        s: &Statement,
    ) {
        let at = self.fb_frame_push(locals);
        self.exec_statement(s);
        self.fb_frame_pop(locals, at);
    }

    /// `fb_exec_stmt_with_locals` for an `EvalExprFallback`.
    #[inline(never)]
    pub(super) fn fb_eval_expr_with_locals(
        &mut self,
        locals: &[super::bytecode::FbLocal],
        e: &Expression,
        ctxw: u32,
    ) -> Value {
        let at = self.fb_frame_push(locals);
        let v = self.eval_expr_ctx(e, ctxw);
        self.fb_frame_pop(locals, at);
        v
    }

    /// Bind each carried local to its register's value, retyped to the
    /// declaration (the register may hold an unstamped flag), in a new
    /// frame. The interpreter also types a local BY NAME — `widths` fits a
    /// write, `signed_signals`/`real_signals` stamp it, `string_signals`
    /// makes it text for `%p` and concatenation — registrations an
    /// interpreted declaration makes and a compiled one never did; they are
    /// set for the statement's duration and restored after it. Returns the
    /// frame depth and the mark of the saved registrations.
    pub(super) fn fb_frame_push(&mut self, locals: &[super::bytecode::FbLocal]) -> (usize, usize) {
        use super::bytecode::LocalKind;
        let depth = self.local_stack.len();
        let mark = self.fb_meta_saves.len();

        // A compiled method's output / ref formal lives in its register; the
        // carried frame is its only copy for the statement's duration, so the
        // call's ref redirection of the name is set aside until it ends (the
        // method epilogue writes the register back as usual).
        let mut unbound = Vec::new();
        for l in locals.iter().filter(|l| l.via_binding) {
            let name = l.name.as_ref();
            let binding = self
                .ref_binding_stack
                .last_mut()
                .and_then(|m| m.remove(name));
            let alias = self.ref_alias_stack.last_mut().and_then(|m| m.remove(name));
            if binding.is_some() || alias.is_some() {
                unbound.push((name.to_string(), binding, alias));
            }
        }
        self.fb_unbound_refs.push(unbound);
        let mut f = self.take_pooled_frame();
        for l in locals {
            let v = Self::fb_fit(self.vm_regs[l.reg as usize].clone(), l.kind);
            f.insert(l.name.to_string(), v);
            let name = l.name.as_ref();
            let cur = (
                self.widths.get(name).copied(),
                self.signed_signals.contains(name),
                self.real_signals.contains(name),
                self.string_signals.contains(name),
            );
            let want = match l.kind {
                LocalKind::Int { width, signed, .. } => (Some(width), signed, false, false),
                LocalKind::Real => (Some(64), false, true, false),
                // Text is exempt from width fitting; leave `widths` alone.
                LocalKind::Str => (cur.0, false, false, true),
            };
            if cur != want {
                self.fb_meta_saves
                    .push((name.to_string(), cur.0, cur.1, cur.2, cur.3));
                self.fb_meta_apply(name, want);
            }
        }
        self.fb_frame_depths.push(depth);
        self.push_local_frame(f);
        (depth, mark)
    }

    pub(super) fn fb_meta_apply(&mut self, name: &str, m: (Option<u32>, bool, bool, bool)) {
        match m.0 {
            Some(w) => {
                self.widths.insert(name.to_string(), w);
            }
            None => {
                self.widths.remove(name);
            }
        }
        if m.1 {
            self.signed_signals.insert(name.to_string());
        } else {
            self.signed_signals.remove(name);
        }
        if m.2 {
            self.real_signals.insert(name.to_string());
        } else {
            self.real_signals.remove(name);
        }
        if m.3 {
            self.string_signals.insert(name.to_string());
        } else {
            self.string_signals.remove(name);
        }
    }

    /// Pop the frame `fb_frame_push` made and copy each local back to its
    /// register, fitted to its declaration as a compiled assignment would.
    pub(super) fn fb_frame_pop(&mut self, locals: &[super::bytecode::FbLocal], at: (usize, usize)) {
        let (depth, mark) = at;
        self.fb_frame_depths.pop();
        while self.fb_meta_saves.len() > mark {
            if let Some((name, w, sg, re, st)) = self.fb_meta_saves.pop() {
                self.fb_meta_apply(&name, (w, sg, re, st));
            }
        }
        // The statement ran to completion (no fallback that carries locals
        // can suspend, return or disable an outer scope), so exactly our
        // frame is on top; anything else is dropped rather than misread.
        while self.local_stack.len() > depth + 1 {
            self.pop_local_frame();
        }
        if self.local_stack.len() != depth + 1 {
            return;
        }
        if let Some(mut f) = self.pop_local_frame_take() {
            for l in locals {
                if let Some(v) = f.remove(l.name.as_ref()) {
                    let v = Self::fb_fit(v, l.kind);
                    let slot = &mut self.vm_regs[l.reg as usize];
                    if *slot != v {
                        *slot = v;
                    }
                }
            }
            if self.frame_pool.len() < 64 {
                f.clear();
                self.frame_pool.push(f);
            }
        }
        for (name, binding, alias) in self.fb_unbound_refs.pop().unwrap_or_default() {
            if let (Some(b), Some(m)) = (binding, self.ref_binding_stack.last_mut()) {
                m.insert(name.clone(), b);
            }
            if let (Some(a), Some(m)) = (alias, self.ref_alias_stack.last_mut()) {
                m.insert(name, a);
            }
        }
    }

    /// A value as the declared local holds it: integral at its width and
    /// signedness, a real as a real (§6.12, §10.7).
    pub(super) fn fb_fit(v: Value, kind: super::bytecode::LocalKind) -> Value {
        use super::bytecode::LocalKind;
        match kind {
            LocalKind::Int {
                width,
                signed,
                two_state,
            } => {
                let mut v = if v.is_real {
                    Self::real_to_int(v.to_f64(), width.max(1))
                } else if v.width != width || v.is_fill {
                    v.resize_for_assign(width)
                } else {
                    v
                };
                if two_state && v.has_xz() {
                    v = v.to_two_state();
                }
                v.is_signed = signed;
                v
            }
            LocalKind::Real => {
                if v.is_real {
                    v
                } else {
                    Value::from_f64(v.to_f64())
                }
            }
            LocalKind::Str => v,
        }
    }

    pub(super) fn push_local_frame(&mut self, f: HashMap<String, Value>) {
        let types = self.take_pooled_types();
        self.push_local_frame_typed(f, types);
    }

    /// §12.7.1: a variable declared in a `for` header has the header's data
    /// type. Returns its initial value, fitted to the declared width and
    /// carrying the declared signedness rather than the initializer's
    /// (§6.11.1), so `for (bit [3:0] i = 2; i > -1; i--)` compares unsigned
    /// and never runs (§11.8.1). Registers the width and signedness by name,
    /// as a block declaration does: the frame-write path fits every later
    /// write to the variable, the step included, from them. Both the
    /// synchronous `for` and the lowering for a body that blocks declare
    /// through here.
    pub(super) fn declare_for_init_var(
        &mut self,
        data_type: &DataType,
        name: &str,
        e: &Expression,
    ) -> Value {
        let v = self.eval_expr(e);
        let w = super::elaborate::resolve_type_width(
            data_type,
            Some(&self.module.parameters),
            Some(&self.module.typedefs),
        );
        self.widths.insert(name.to_string(), w);
        let mut v = v.resize(w);
        v.is_signed = self.type_is_signed_concrete(data_type);
        if v.is_signed {
            self.signed_signals.insert(name.to_string());
        } else {
            self.signed_signals.remove(name);
        }
        v
    }

    /// The frame a loop pushes for its loop variables in a process that has
    /// no call frame (see `loop_frame_gens`).
    pub(super) fn push_loop_frame(&mut self) {
        self.push_local_frame(HashMap::default());
        if let Some(&g) = self.local_gen_stack.last() {
            self.loop_frame_gens.insert(g);
        }
    }

    pub(super) fn pop_loop_frame(&mut self) {
        if let Some(g) = self.local_gen_stack.last() {
            // §9.3.2: a surviving fork child still owns its capture frame.
            // Keep that generation classified as a process frame after the
            // parent leaves the loop; task/call-frame lifetime rules differ.
            let inherited = self.fork_baselines.keys().any(|pid| {
                self.process_contexts
                    .get(pid)
                    .is_some_and(|ctx| ctx.local_gen_stack.contains(g))
            });
            if !inherited {
                self.loop_frame_gens.remove(g);
            }
        }
        self.pop_local_frame();
    }

    /// §6.21: is a declaration executed here a local of an `initial` /
    /// `always` / `final` block — no call frame, only loop-variable frames?
    pub(super) fn in_process_block_scope(&self) -> bool {
        // A call frame is far more common than a loop frame: test the
        // innermost frame first, and skip the lookups when no loop frame is
        // open at all.
        match self.local_gen_stack.last() {
            None => true,
            Some(_) if self.loop_frame_gens.is_empty() => false,
            Some(g) => {
                self.loop_frame_gens.contains(g)
                    && self
                        .local_gen_stack
                        .iter()
                        .all(|g| self.loop_frame_gens.contains(g))
            }
        }
    }

    /// Push a local frame together with its type overlay (the declared
    /// class / typedef names of its variables, see `local_type_stack`). A
    /// subroutine collects its formals' types while binding them and hands
    /// them over here: recording them with `record_local_*` before the push
    /// wrote them into the CALLER's overlay instead.
    pub(super) fn push_local_frame_typed(
        &mut self,
        f: HashMap<String, Value>,
        types: (HashMap<String, String>, HashMap<String, String>),
    ) {
        if self.name_stats_on {
            self.name_stats[2].set(self.name_stats[2].get() + 1);
        }
        self.local_stack.push(f);
        let fgen = self.next_frame_gen();
        self.local_gen_stack.push(fgen);
        self.local_type_stack.push(types);
    }

    /// An empty type-overlay pair from the pool (capacity retained) or a
    /// fresh one.
    pub(super) fn take_pooled_types(
        &mut self,
    ) -> (HashMap<String, String>, HashMap<String, String>) {
        self.local_type_pool.pop().unwrap_or_default()
    }

    /// The type bound to the class type parameter `dt` names under the
    /// current specialization, or `None` when `dt` names no type parameter.
    pub(super) fn formal_type_param_binding(&self, dt: &DataType) -> Option<String> {
        let DataType::TypeReference { name: tn, .. } = dt else {
            return None;
        };
        let type_name = &tn.name.name;
        if self.module.classes.contains_key(type_name)
            || self.module.enum_members.contains_key(type_name)
            || self.module.typedefs.contains_key(type_name)
        {
            return None;
        }
        self.resolve_type_param_binding(type_name)
    }

    /// Return a popped `local_type_stack` pair to the pool: only lookups ever
    /// read these maps, so a cleared map with retained capacity is as good as
    /// a fresh one.
    pub(super) fn recycle_local_types(
        &mut self,
        pair: Option<(HashMap<String, String>, HashMap<String, String>)>,
    ) {
        if let Some(mut pair) = pair {
            // A pair that never allocated is free to drop and to rebuild.
            let (c0, c1) = (pair.0.capacity(), pair.1.capacity());
            if (c0 | c1) != 0 && self.local_type_pool.len() < 64 && c0 <= 16 && c1 <= 16 {
                if !pair.0.is_empty() {
                    pair.0.clear();
                }
                if !pair.1.is_empty() {
                    pair.1.clear();
                }
                self.local_type_pool.push(pair);
            }
        }
    }

    /// Pop a local frame, keeping the type overlay in lockstep. The frame
    /// map is recycled into `frame_pool` (capacity kept, contents cleared);
    /// use `pop_local_frame_take` when the caller needs the contents.
    pub(super) fn pop_local_frame(&mut self) {
        let pair = self.local_type_stack.pop();
        self.recycle_local_types(pair);
        if let Some(g) = self.local_gen_stack.pop() {
            if self.task_string_formals.last().is_some_and(|e| e.fgen == g) {
                self.task_string_formals.pop();
            }
        }
        if let Some(mut f) = self.local_stack.pop() {
            if self.frame_pool.len() < 64 {
                f.clear();
                self.frame_pool.push(f);
            }
        }
    }

    /// Pop a local frame and hand the map to the caller (writeback reads).
    pub(super) fn pop_local_frame_take(&mut self) -> Option<HashMap<String, Value>> {
        let pair = self.local_type_stack.pop();
        self.recycle_local_types(pair);
        if let Some(g) = self.local_gen_stack.pop() {
            if self.task_string_formals.last().is_some_and(|e| e.fgen == g) {
                self.task_string_formals.pop();
            }
        }
        self.local_stack.pop()
    }

    /// Read a name from the innermost subroutine frame. Routing the frame
    /// reads through one place makes their cost measurable (XEZIM_NAME_STATS
    /// slot 4) — frames hold a mean of 1.8 entries, so whether hashing a
    /// String key is worth it at all is an empirical question.
    #[inline]
    pub(super) fn frame_get(&self, name: &str) -> Option<&Value> {
        if self.name_stats_on {
            self.name_stats[4].set(self.name_stats[4].get() + 1);
        }
        self.local_stack.last().and_then(|l| l.get(name))
    }

    /// A frame map from the pool (capacity retained) or a fresh one.
    pub(super) fn take_pooled_frame(&mut self) -> HashMap<String, Value> {
        self.frame_pool.pop().unwrap_or_default()
    }

    /// Leaf-segment index over `module.parameters` (built once).
    pub(super) fn param_leaf_index(&self) -> &HashMap<String, Vec<String>> {
        self.param_leaf_index_cell.get_or_init(|| {
            let mut idx: HashMap<String, Vec<String>> = HashMap::default();
            for name in self.module.parameters.keys() {
                let leaf = name.rsplit('.').next().unwrap_or(name.as_str());
                idx.entry(leaf.to_string()).or_default().push(name.clone());
            }
            idx
        })
    }

    /// Leaf-segment (`::`-suffix) index over `module.var_decl_types`
    /// (built once) — see `var_decl_leaf_index_cell`.
    pub(super) fn var_decl_leaf_index(&self) -> &HashMap<String, Vec<String>> {
        self.var_decl_leaf_index_cell.get_or_init(|| {
            let mut idx: HashMap<String, Vec<String>> = HashMap::default();
            for name in self.module.var_decl_types.keys() {
                let leaf = name.rsplit(':').next().unwrap_or(name.as_str());
                idx.entry(leaf.to_string()).or_default().push(name.clone());
            }
            idx
        })
    }

    pub(super) fn class_of_var(&self, vname: &str) -> Option<String> {
        // A class name IS its own class — `ClassName::static_prop` writes
        // and reads route here with vname = the class name. Without this,
        // `container::the_obj = new` fails to find the static cell and the
        // write is silently lost.
        if self.module.classes.contains_key(vname) {
            return Some(vname.to_string());
        }
        // Frame-local overlay first (see local_type_stack): the innermost
        // live frame's declaration wins over the shared global map.
        if let Some(c) = self.local_class_type_of(vname) {
            if self.module.classes.contains_key(&c) {
                return Some(c);
            }
            if c.contains('#') {
                let base = c.split('#').next().unwrap_or(&c);
                if self.module.classes.contains_key(base) {
                    return Some(c);
                }
            }
        }
        // Member of `this` (the current object) — IEEE 1800-2023 §8.14:
        // inside a class method, properties of `this` take precedence over
        // outer / unrelated procedural locals in `var_class_types`.
        // Borrow the instance's class name (no heap allocation per lookup)
        // so the probe in `class_prop_type_named` stays cheap on hot
        // class-method paths.
        if let Some(h) = self.this_stack.last().copied().flatten() {
            if let Some(inst) = self.heap.get(h).and_then(|o| o.as_ref()) {
                if let Some(tn) = self.class_prop_type_named(&inst.class_name, vname) {
                    if self.module.classes.contains_key(&tn) || tn.contains('#') {
                        return Some(tn);
                    }
                    // A property typed by a class TYPE PARAMETER comes back as
                    // the parameter's NAME, which is no class. Resolve it, as
                    // `class_prop_type_named` leaves to its callers.
                    if let Some(c) = self.bound_prop_class(inst, vname, &tn) {
                        return Some(c);
                    }
                }
            }
        }
        // Procedural locals record their class at VarDecl exec time.
        if let Some(c) = self.var_class_types.get(vname) {
            if self.module.classes.contains_key(c) {
                return Some(c.clone());
            }
            // The VarDecl may have already resolved a type parameter to a
            // value-parameterized specialization (`T me;` where T → `base#(1)`).
            // The base is a real class, so the specialization IS the var's
            // class — otherwise `class_of_var` returns None and `$cast`
            // degenerates to the permissive path (§8.25 value-param identity).
            if c.contains('#') {
                let base = c.split('#').next().unwrap_or(c);
                if self.module.classes.contains_key(base) {
                    return Some(c.clone());
                }
            }
            // A local declared with a TYPE PARAMETER as its type
            // (`T me;` inside a parameterized-class method). Resolve T
            // to its concrete specialization via the active
            // `current_spec`, so `$cast(me, obj)` and member access use
            // the correct dynamic type. Without this, `class_of_var`
            // returns None (T is not a real class) and `$cast` falls back
            // to the permissive path (always succeeds), corrupting
            // dynamic type filtering (LRM §6.20.2/§8.25).
            if let Some(resolved) = self.resolve_type_param_binding(c) {
                if self.module.classes.contains_key(&resolved) {
                    return Some(resolved);
                }
                // The resolved type may be a value-parameterized
                // specialization like `special_comp#(1)` (a type param
                // bound to a value-parameterized class). Return it WITH
                // the `#(args)` so callers like `$cast`'s cast_type_ok
                // can strip the base class for the hierarchy check and
                // compare the value params. Without this, class_of_var
                // returns None and `$cast` falls back to permissive,
                // accepting an object of type `special_comp#(2)` as a
                // `special_comp#(1)` (LRM §8.25: value-param differences
                // make distinct, non-assignable types).
                if resolved.contains('#') {
                    return Some(resolved);
                }
            }
        }
        if let Some(sig) = self.module.signals.get(vname) {
            if let Some(tn) = &sig.type_name {
                if self.module.classes.contains_key(tn) {
                    return Some(tn.clone());
                }
            }
        }
        // Module-level class handles are only in the compact signal table.
        if let Some(&id) = self.signal_name_to_id.get(vname) {
            if let Some(tn) = self.signal_type_names.get(&id) {
                if self.module.classes.contains_key(tn) {
                    return Some(tn.clone());
                }
            }
        }
        // Static class property: walk the class context hierarchy to find
        // which class declares `vname` as static. This is essential for
        // `obj.static_prop = val` inside a method body — the object is a
        // static variable, not a local or signal, so the checks above miss it.
        // Without this, the write falls through to instance storage and the
        // shared static cell is never updated.
        if let Some(Some(ctx)) = self.class_context_stack.last().cloned() {
            let mut cur = Some(ctx);
            while let Some(cname) = cur {
                if let Some(cd) = self.module.classes.get(&cname) {
                    if cd.static_properties.contains(vname) {
                        return Some(cname.clone());
                    }
                    cur = cd.extends.clone();
                } else {
                    break;
                }
            }
        }
        None
    }

    /// id-collision-in-heap guard, bare-name form: can this single,
    /// select-free identifier denote a class-object lvalue in the current
    /// scope? The heap-divert arms in `assign_value` and the ident read path
    /// reinterpreted the VALUE stored under a head as a heap handle whenever
    /// it happened to be a small live index — but a packed-struct variable
    /// splices its members into one integral value (IEEE 1800-2017 §7.8), so
    /// `sv.delay_counter` with `sv == 32'd1` diverted a plain struct-field
    /// write into `heap[1].delay_counter`. Mirrors
    /// `bare_receiver_is_class_handle`'s static conjunct (an enum local
    /// shadows the flat class map) WITHOUT its runtime-liveness conjunct —
    /// the colliding value is live by construction, so only the declared
    /// shape of the name discriminates (§8.4 class-handle dereference).
    pub(super) fn bare_name_is_class_channel(&self, name: &str) -> bool {
        if name == "this" || name == "super" {
            return true;
        }
        // The declared typedef of `name` in the ACTIVE frames: the flat
        // `var_typedef_types` map keeps whatever an earlier, finished
        // activation declared under the name (`uvm_severity s;` in a report
        // method), which turned every later `s.f = v` through an unrelated
        // class handle `s` into a dropped write.
        let declared_td = if self.local_stack.is_empty() {
            self.var_typedef_types.get(name).cloned()
        } else {
            self.local_typedef_type_of(name)
        };
        if declared_td
            .as_ref()
            .is_some_and(|tn| self.module.enum_members.contains_key(tn))
        {
            return false;
        }
        if self.class_of_var(name).is_some() {
            return true;
        }
        // A typedef ALIAS of a class (`typedef C alias_t; alias_t al;`, or a
        // parameterized `typedef pc#(16) pc16_t; pc16_t pp;`): the name's
        // declared type resolves through the alias to the class, so the
        // bare name IS a class-object channel. Procedural locals carry
        // their declared typedef name in `var_typedef_types`;
        // module/package-scope variables are only in `var_decl_types`.
        // (id-collision-in-heap review regression 2 — resolved here, in
        // the classification helper, not in `class_of_var`.)
        let declared_tn = declared_td.or_else(|| {
            self.module
                .var_decl_types
                .get(name)
                .and_then(|dt| match dt {
                    DataType::TypeReference { name, .. } => Some(name.name.name.clone()),
                    _ => None,
                })
        });
        if declared_tn
            .as_deref()
            .and_then(|tn| self.resolve_typeref_class_name_str(tn))
            .is_some()
        {
            return true;
        }
        // Class-PROPERTY rung (§6.20.2/§8.25): a bare name that is a property
        // of the enclosing class context is a class-object channel when the
        // property's CONCRETE type is a class. `class_prop_type_named` passes
        // a type-parameter name through (§6.20.3), so resolve it against the
        // running method's instance bindings first: a property declared with a
        // class TYPE PARAMETER (`class p #(type CFG = cfg_c); CFG a;`) named no
        // class, so `a.member = v` failed this receiver gate, fell out of the
        // MemberAccess lvalue arm, and the write was silently dropped.
        if let Some(ctx) = self.class_context_stack.last().cloned().flatten()
            && let Some(tn) = self.class_prop_type_named(&ctx, name)
        {
            let mut concrete = tn;
            if let Some(h) = self
                .this_stack
                .last()
                .copied()
                .flatten()
                .filter(|&h| h != 0)
            {
                let inst = self.heap.get(h).and_then(|o| o.as_ref());
                if let Some(inst) = inst {
                    if let Some(bound) = inst.type_bindings.get(&concrete) {
                        concrete = bound.clone();
                    } else if let Some(bound) =
                        self.carried_ancestor_param(&ctx, &inst.type_bindings, &concrete)
                    {
                        concrete = bound;
                    }
                }
            }
            if self.resolve_typeref_class_name_str(&concrete).is_some() {
                return true;
            }
        }
        false
    }

    /// id-collision-in-heap guard, shared name-only segment ladder: do
    /// these dotted segments denote a class-object lvalue? Rungs, in
    /// order: a `this`/`super` root; a one-segment bare class channel or
    /// handle-collection element; a class-channel root; a frame-dotted
    /// local member holding a handle (`stor.handle`); a registered
    /// class-typed composed signal (`mod.h`); a struct member whose
    /// DECLARED type is a class (`dotted_name_declared_class`); a
    /// class-channel leaf. `head_denotes_class_object` (selects-aware)
    /// and `receiver_may_be_handle` (MemberAccess flattening) layer their
    /// shape-specific discrimination on top of this ladder.
    pub(super) fn dotted_segments_denote_class_object(&self, segs: &[&str]) -> bool {
        if segs.is_empty() {
            return false;
        }
        let first = segs[0];
        if first == "this" || first == "super" {
            return true;
        }
        if segs.len() == 1 {
            return self.bare_name_is_class_channel(first)
                || self
                    .elem_class_of_arr(first)
                    .is_some_and(|c| self.module.classes.contains_key(&c));
        }
        if self.bare_name_is_class_channel(first) {
            return true;
        }
        // A composed declaration is authoritative. A same-named class
        // variable in the observer cannot turn a struct/net into a handle.
        if segs.len() > 1 {
            let dotted = segs.join(".");
            let name = dotted
                .strip_prefix(&format!("{}.", self.module.name))
                .unwrap_or(&dotted);
            if let Some(dt) = self.module.var_decl_types.get(name) {
                return self
                    .declared_typeref_name(dt)
                    .and_then(|tn| self.resolve_typeref_class_name_str(&tn))
                    .is_some();
            }
            if let Some(sig) = self.module.signals.get(name) {
                return sig
                    .type_name
                    .as_deref()
                    .and_then(|tn| self.resolve_typeref_class_name_str(tn))
                    .is_some();
            }
        }
        let dotted = segs.join(".");
        if self
            .local_stack
            .last()
            .is_some_and(|l| l.contains_key(&dotted))
        {
            return true;
        }
        if let Some(&id) = self.signal_name_to_id.get(dotted.as_str()) {
            if let Some(tn) = self.signal_type_names.get(&id) {
                if self.module.classes.contains_key(tn) {
                    return true;
                }
            }
        }
        if let Some(sig) = self.module.signals.get(dotted.as_str()) {
            if let Some(tn) = &sig.type_name {
                if self.module.classes.contains_key(tn) {
                    return true;
                }
            }
        }
        // A struct member whose DECLARED type is a class (`stor.handle` —
        // an unpacked-struct member holding a handle).
        if self.dotted_name_declared_class(segs).is_some() {
            return true;
        }
        self.bare_name_is_class_channel(segs[segs.len() - 1])
    }

    /// id-collision-in-heap guard, multi-segment head form (selects
    /// preserved) — used by the write-side prefix-eval arm. The head denotes
    /// a class object through any of:
    ///   1. a `this`/`super` root,
    ///   2. a class-typed ROOT variable — the head names a property chain
    ///      of class handles (`h.direct.tag`),
    ///   3. a frame-dotted local member holding a handle (`stor.handle`,
    ///      the §4c automatic-struct precedent),
    ///   4. a registered class-typed composed signal (module-scope unpacked
    ///      struct member holding a handle, `mod.h`),
    ///   5. a class-typed LEAF — a handle-collection element (`ch[0]`) or a
    ///      bare class-typed leaf (`mod.h`).
    /// Anything else (`ov.inner`, `u.sv`, `u_if.iv`, `q[0]` of struct
    /// elements) names struct/hierarchical storage whose spliced member
    /// value must NOT be read back as a heap handle. The name-only rungs
    /// delegate to `dotted_segments_denote_class_object`; the single-segment
    /// and leaf rungs stay local because a segment list cannot express
    /// selects (a selected name is a collection ELEMENT, classified by its
    /// element class, never by the collection variable's own channel test).
    pub(super) fn head_denotes_class_object(&self, head: &HierarchicalIdentifier) -> bool {
        if head.path.is_empty() {
            return false;
        }
        let first = head.path[0].name.name.as_str();
        if first == "this" || first == "super" {
            return true;
        }
        if head.path.len() == 1 {
            let seg = &head.path[0];
            if seg.selects.is_empty() {
                return self.bare_name_is_class_channel(first);
            }
            return self
                .elem_class_of_arr(&seg.name.name)
                .is_some_and(|c| self.module.classes.contains_key(&c));
        }
        let head_segs: Vec<&str> = head.path.iter().map(|s| s.name.name.as_str()).collect();
        if self.dotted_segments_denote_class_object(&head_segs) {
            return true;
        }
        // Selects-aware leaf rung: a selected leaf (`stor.q[0]`) is a
        // collection element — its ELEMENT class decides.
        let leaf = head.path.last().unwrap();
        if !leaf.selects.is_empty() {
            return self
                .elem_class_of_arr(&leaf.name.name)
                .is_some_and(|c| self.module.classes.contains_key(&c));
        }
        false
    }

    /// id-collision-in-heap guard, receiver-expression form: may this
    /// receiver's VALUE legitimately be read back as a heap handle? Used by
    /// the late write-side fall-throughs (the MemberAccess tail divert and
    /// the part-select member base) whose only liveness evidence was the
    /// receiver's integral value — the exact misread this bug fixes. Static
    /// name shapes are discriminated (`this`, a class-channel identifier, a
    /// handle-collection element, a chained class receiver); exotic receivers
    /// (method results, concatenations) keep the legacy value-based attempt,
    /// which still requires a live object declaring the property.
    pub(super) fn receiver_may_be_handle(&mut self, recv: &Expression) -> bool {
        match &recv.kind {
            ExprKind::This => true,
            ExprKind::Ident(h) => self.head_denotes_class_object(h),
            ExprKind::Index { expr: base, .. } => match &base.kind {
                ExprKind::Ident(bh) if bh.path.len() == 1 && bh.path[0].selects.is_empty() => self
                    .elem_class_of_arr(&bh.path[0].name.name)
                    .is_some_and(|c| self.module.classes.contains_key(&c)),
                _ => true,
            },
            ExprKind::MemberAccess { expr, .. } => {
                // A frame-dotted local member holding a handle
                // (`stor.handle`, the §4c automatic-struct precedent) or a
                // registered class-typed composed signal (`mod.h`) — the
                // flattened head forms go through the shared
                // `dotted_segments_denote_class_object` ladder (the same
                // rungs `head_denotes_class_object` uses for cases 3/4/6)
                // before the structural recursion. A dot-free flat name
                // keeps only the composed-signal table probes: the
                // ladder's single-segment rung answers a different
                // question there.
                if let Some(flat) = self.flat_member_name(recv) {
                    if flat.contains('.') {
                        let segs: Vec<&str> = flat.split('.').collect();
                        if self.dotted_segments_denote_class_object(&segs) {
                            return true;
                        }
                    } else {
                        if let Some(&id) = self.signal_name_to_id.get(flat.as_str()) {
                            if let Some(tn) = self.signal_type_names.get(&id) {
                                if self.module.classes.contains_key(tn) {
                                    return true;
                                }
                            }
                        }
                        if let Some(sig) = self.module.signals.get(flat.as_str()) {
                            if let Some(tn) = &sig.type_name {
                                if self.module.classes.contains_key(tn) {
                                    return true;
                                }
                            }
                        }
                    }
                }
                self.receiver_may_be_handle(expr)
            }
            _ => true,
        }
    }

    /// id-collision-in-heap guard supplement: the declared CLASS type of a
    /// dotted name chain, resolved from the root's declaration through
    /// `lookup_type_member` (the same walk `get_expr_type_name` uses for
    /// `stor.handle`-style paths). An unpacked-struct MEMBER holding a class
    /// handle resolves to its class, while a packed-struct field chain
    /// (`ov_r.inner`), a hierarchical prefix (`u2.sv_sub`) or a struct
    /// typedef leaf resolves to a non-class or nothing — exactly the
    /// discrimination the value-based heap diverts need.
    pub(super) fn dotted_name_declared_class(&self, segs: &[&str]) -> Option<String> {
        if segs.len() < 2 {
            return None;
        }
        let root = segs[0];
        let mut tn = if let Some(t) = self.var_class_types.get(root) {
            t.clone()
        } else if let Some(t) = self.var_typedef_types.get(root) {
            t.clone()
        } else if self.module.classes.contains_key(root) {
            root.to_string()
        } else if let Some(Some(ctx)) = self.class_context_stack.last().cloned() {
            self.class_prop_type_named(&ctx, root)?
        } else if let Some(dt) = self.module.var_decl_types.get(root) {
            // Module/package-scope root (id-collision-in-heap review
            // regression 1): `S gx;` — the root is not a procedural local
            // or formal, so no runtime map above records it; its DECLARED
            // type starts the member walk instead.
            self.declared_typeref_name(dt)?
        } else {
            // A package-qualified module-scope declaration is keyed under
            // its qualified name (`pkg::gx`), so the bare root misses the
            // direct probe above — mirror `declared_collection_elem_class`'s
            // leaf-index fallback so those roots start the walk too.
            let leaf = root.rsplit(':').next().unwrap_or(root);
            let mut hit = None;
            if let Some(keys) = self.var_decl_leaf_index().get(leaf) {
                for key in keys {
                    if let Some(dt) = self.module.var_decl_types.get(key) {
                        if let Some(t) = self.declared_typeref_name(dt) {
                            hit = Some(t);
                            break;
                        }
                    }
                }
            }
            hit?
        };
        for seg in &segs[1..] {
            tn = self.lookup_type_member(&tn, seg)?;
        }
        if self.module.classes.contains_key(&tn) {
            Some(tn)
        } else if tn.contains('#') {
            let base = tn.split('#').next().unwrap_or(&tn);
            if self.module.classes.contains_key(base) {
                Some(tn)
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Module-scope root rung of `dotted_name_declared_class`: the DECLARED
    /// `DataType` of a module/package-scope variable (`S gx;`), whose
    /// `TypeReference` name starts the member walk. A typedef ALIAS of a
    /// class (`typedef C alias_t; alias_t al;`) resolves to that class first
    /// so `al.cfg.f` walks the class's own members; anything else (a struct
    /// typedef) walks under its own name, where `lookup_type_member`'s
    /// struct arm or the final class check rejects non-class chains.
    pub(super) fn declared_typeref_name(&self, dt: &DataType) -> Option<String> {
        let DataType::TypeReference { name, .. } = dt else {
            return None;
        };
        let tn = name.name.name.clone();
        Some(self.resolve_typeref_class_name_str(&tn).unwrap_or(tn))
    }

    /// id-collision-in-heap guard, name-only form for the read-side prefix
    /// walk, which collapses selects when it builds candidate names: a
    /// one-segment prefix may have been selected (`ch[0].x`), so handle-
    /// collection element channels are accepted alongside bare class
    /// channels. Struct collections still fail (their element typedef is
    /// not a class). The ladder itself is shared with the write-side
    /// guards (`dotted_segments_denote_class_object`).
    pub(super) fn dotted_prefix_is_class_channel(&self, segs: &[&str]) -> bool {
        self.dotted_segments_denote_class_object(segs)
    }

    /// Follow a typedef chain to the underlying type.
    pub(super) fn resolve_dt(&self, dt: &DataType) -> DataType {
        self.resolve_dt_ref(dt).clone()
    }

    /// `resolve_dt` as a borrow: the result is either `dt` itself or an entry
    /// of the typedef table, so read-only callers (type predicates, `matches!`)
    /// need no clone.
    pub(super) fn resolve_dt_ref<'a>(&'a self, dt: &'a DataType) -> &'a DataType {
        let mut cur = dt;
        for _ in 0..16 {
            if let DataType::TypeReference { name, .. } = cur {
                if let Some(next) = self.module.typedef_types.get(&name.name.name) {
                    cur = next;
                    continue;
                }
            }
            break;
        }
        cur
    }

    /// §20.6.1 `$typename` of a virtual-interface operand: a variable (module
    /// scope or local) or a class property `h.p` whose declared type is
    /// `virtual <iface>` or a typedef chain ending there. None otherwise.
    pub(super) fn vif_operand_typename(&mut self, arg: &Expression) -> Option<String> {
        let ExprKind::Ident(hier) = &arg.kind else {
            return None;
        };
        if hier.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let dt = match hier.path.len() {
            1 => self
                .module
                .var_decl_types
                .get(hier.path[0].name.name.as_str())
                .cloned(),
            2 => {
                let h = self.eval_ident_handle(hier.path[0].name.name.as_str())?;
                let cn = self
                    .heap
                    .get(h)
                    .and_then(|o| o.as_ref())
                    .map(|i| i.class_name.clone())?;
                let prop = hier.path[1].name.name.as_str();
                let mut cur = Some(cn);
                let mut found = None;
                while let Some(c) = cur {
                    let cd = self.module.classes.get(&c)?;
                    if let Some(dt) = cd.property_types.get(prop) {
                        found = Some(dt.clone());
                        break;
                    }
                    cur = cd.extends.clone();
                }
                found
            }
            _ => None,
        }?;
        if !Self::dt_is_virtual_iface(&dt, &self.module) {
            return None;
        }
        // Parameter values evaluate in the calling scope, which also sees a
        // `localparam` of a top wrapped for elaboration (`top.P`).
        let resolved =
            super::elaborate::resolve_typedef_chain(&dt, &self.module.typedef_types).clone();
        if let DataType::Interface {
            name,
            modport,
            type_args,
            ..
        } = &resolved
        {
            if !type_args.is_empty() {
                let mut s = format!("virtual {}", name.name);
                if let Some(mp) = modport {
                    s.push('.');
                    s.push_str(&mp.name);
                }
                let mut parts = Vec::with_capacity(type_args.len());
                for a in type_args {
                    let (named, e) = match &a.kind {
                        ExprKind::NamedArg { name, expr } => (Some(&name.name), expr.as_deref()),
                        _ => (None, Some(a)),
                    };
                    let v = match e {
                        Some(e) => match self.eval_expr(e).to_i64() {
                            Some(v) => v.to_string(),
                            None => "?".to_string(),
                        },
                        None => String::new(),
                    };
                    parts.push(match named {
                        Some(n) => format!(".{}({})", n, v),
                        None => v,
                    });
                }
                s.push_str(&format!(" #({})", parts.join(", ")));
                return Some(s);
            }
        }
        Some(self.typename_of_dt(&resolved))
    }

    /// `$typename` of a resolved CONCRETE type string (a type parameter's
    /// binding, from `resolve_type_param_binding`): `class <n>` for a class /
    /// covergroup (or a class specialization), or the base keyword with the
    /// packed range for a built-in / typedef'd vector.
    pub(super) fn typename_of_concrete(&self, concrete: &str) -> String {
        // A class (or a specialization `foo#(...)` whose base is a class).
        if self.module.classes.contains_key(concrete)
            || self.module.covergroups.contains_key(concrete)
        {
            return format!("class {}", concrete);
        }
        if let Some(n) = concrete.split('#').next() {
            if self.module.classes.contains_key(n) {
                // A specialization (`box#(byte)`): every parameter listed,
                // spelled as for a declared class type.
                if let Some((b, sig)) = self.extract_spec_from_string(concrete) {
                    return self.class_spec_typename(&b, Some(&sig));
                }
            }
            if self.module.classes.contains_key(n) || self.module.covergroups.contains_key(n) {
                return format!("class {}", concrete);
            }
        }
        // A typedef of a built-in / vector, e.g. `uvm_bitstream_t` ->
        // `logic signed [4095:0]`. Resolve the target and render it.
        if self.module.typedef_types.contains_key(concrete)
            || self.lookup_typedef_target(concrete).is_some()
        {
            let target = self.lookup_typedef_target(concrete).unwrap_or_else(|| {
                crate::ast::types::DataType::Implicit {
                    signing: None,
                    dimensions: Vec::new(),
                    span: crate::ast::Span::dummy(),
                }
            });
            return self.typename_of_dt(&target);
        }
        // Otherwise a bare built-in keyword (`string`, `int`, ...) or the
        // type name itself.
        concrete.to_string()
    }

    /// Constant indices of ONE fixed unpacked dimension in DECLARED order,
    /// left bound first: `[1:0]` gives `[1, 0]`, `[0:1]` and `[2]` give
    /// `[0, 1]`. That is an unpacked array's element order (IEEE 1800-2017
    /// §7.4.2): an assignment pattern fills the left bound first (§10.10)
    /// and `%p` prints it first (§21.2.1.7). `None` for a dynamic, queue or
    /// associative dimension, or one whose bounds do not resolve.
    pub(super) fn fixed_dim_indices(
        &self,
        dim: &crate::ast::types::UnpackedDimension,
    ) -> Option<Vec<i64>> {
        use crate::ast::types::UnpackedDimension as UD;
        // `m[N]` (N a parameter) parses as an associative dim keyed by "type" N.
        let dims = super::elaborate::normalize_unpacked_dims(
            std::slice::from_ref(dim),
            &self.module.parameters,
            &self.module.typedef_types,
        );
        let p = Some(&self.module.parameters);
        match dims.first()? {
            UD::Expression { expr, .. } => {
                match super::elaborate::const_eval_i64_with_params(expr, p) {
                    Some(n) if n > 0 && n <= 4096 => Some((0..n).collect()),
                    _ => None,
                }
            }
            UD::Range { left, right, .. } => {
                let l = super::elaborate::const_eval_i64_with_params(left, p)?;
                let r = super::elaborate::const_eval_i64_with_params(right, p)?;
                if (l - r).abs() >= 4096 {
                    None
                } else if l <= r {
                    Some((l..=r).collect())
                } else {
                    Some((r..=l).rev().collect())
                }
            }
            _ => None,
        }
    }

    /// IEEE 1800-2017 §7.4.2: the SHAPE of a struct member's unpacked
    /// dimensions — one constant index list per dimension, outermost first,
    /// each in declared order. `bit [7:0] mm [2][2]` has two dimensions and
    /// its elements are `mm[i][j]`; every one of them is its own leaf.
    ///
    /// This is the one place a member's element layout comes from: leaf
    /// enumeration, defaults, whole-struct copy, member-wise assignment,
    /// assignment patterns and `%p` all walk it. It used to report only the
    /// FIRST dimension, so everything past `mm[i]` was invisible to those
    /// consumers and a member with two or more unpacked dimensions was lost
    /// by every copy, pattern, formal and local.
    ///
    /// `None` when the member has no unpacked dimension, when any dimension
    /// is not a fixed range, or when the element count would explode.
    pub(super) fn member_dim_indices(
        &self,
        dims: &[crate::ast::types::UnpackedDimension],
    ) -> Option<Vec<Vec<i64>>> {
        if dims.is_empty() {
            return None;
        }
        let mut out: Vec<Vec<i64>> = Vec::with_capacity(dims.len());
        for d in dims {
            out.push(self.fixed_dim_indices(d)?);
        }
        let total: u64 = out.iter().map(|l| l.len() as u64).product();
        if total == 0 || total > (1 << 20) {
            return None;
        }
        Some(out)
    }

    /// IEEE 1800-2017 §20.7 `$left`/`$right`/`$low`/`$high`/`$size`/
    /// `$increment` of an unpacked-struct MEMBER array that has no registered
    /// shape — a member of a block-local, formal or array-element struct, or
    /// a sub-array (`$size(s.mm[1])`) — answered from the member's declared
    /// dimensions. `None` for anything else, including a member whose shape
    /// is registered (the general path answers it).
    /// §20.7: an array query answered from a type's dimensions — a signed
    /// `int`, or x for a dimension the type does not have.
    pub(super) fn type_query_value(
        sn: &str,
        q: &super::elaborate::TypeQueryDims,
        dim: usize,
    ) -> Value {
        let Some(r) = super::elaborate::type_query_bound(sn, q, dim) else {
            let mut xv = Value::new(32);
            xv.is_signed = true;
            return xv;
        };
        let mut rv = Value::from_u64((r as u64) & 0xFFFF_FFFF, 32);
        rv.is_signed = true;
        rv
    }

    /// §20.7, §7.4.5: the dimensions of an array-query operand read from its
    /// DECLARED type — unpacked first, then packed — for an operand whose
    /// storage registers no packed shape: a class property (`m`, `this.m`,
    /// `o.m`) or a member of a subroutine formal or local struct (`a.w`). A
    /// multi-dimensional packed variable there is stored as one flattened
    /// vector, so the queries reported the flattened width (`$size(a.wdata)`
    /// 131584 for `bit [256:0][511:0]`, not 257). A bare design variable is
    /// left to its registered shape. `None` when a step of the path is
    /// unknown or carries a select, or an inner step has an unpacked
    /// dimension.
    #[cold]
    #[inline(never)]
    pub(super) fn query_operand_decl_dims(
        &self,
        arg: &Expression,
    ) -> Option<super::elaborate::TypeQueryDims> {
        fn path<'e>(e: &'e Expression, out: &mut Vec<&'e str>) -> Option<()> {
            match &e.kind {
                ExprKind::Ident(h) => {
                    for s in &h.path {
                        if !s.selects.is_empty() {
                            return None;
                        }
                        out.push(s.name.name.as_str());
                    }
                    Some(())
                }
                ExprKind::This => {
                    out.push("this");
                    Some(())
                }
                ExprKind::MemberAccess { expr, member } => {
                    path(expr, out)?;
                    out.push(member.name.as_str());
                    Some(())
                }
                _ => None,
            }
        }
        let mut segs: Vec<&str> = Vec::new();
        path(arg, &mut segs)?;
        let (first, rest) = segs.split_first()?;
        let (mut dt, mut udims) = if *first == "this" {
            let ctx = self.class_context_stack.last()?.as_deref()?;
            (Self::class_type_ref(ctx), Vec::new())
        } else {
            let (dt, udims, is_prop) = self.query_root_decl_type(first)?;
            if rest.is_empty() && !is_prop {
                return None;
            }
            (dt, udims)
        };
        for seg in rest {
            if !udims.is_empty() {
                return None;
            }
            (dt, udims) = self.member_decl_type(&dt, seg)?;
        }
        if let DataType::TypeReference { name, .. } = &dt
            && self.module.classes.contains_key(&name.name.name)
        {
            return None;
        }
        let inner = super::elaborate::type_query_dims_of(
            &dt,
            Some(&self.module.parameters),
            Some(&self.module.typedefs),
            Some(&self.module.typedef_types),
        );
        let unpacked = udims.len();
        udims.extend(inner.dims);
        Some(super::elaborate::TypeQueryDims {
            dims: udims,
            unpacked,
            whole: inner.whole,
        })
    }

    /// A `TypeReference` naming class `cn`.
    pub(super) fn class_type_ref(cn: &str) -> DataType {
        let span = crate::ast::Span::dummy();
        DataType::TypeReference {
            name: crate::ast::types::TypeName {
                scopes: Vec::new(),
                name: crate::ast::Identifier {
                    name: cn.to_string(),
                    span,
                },
                span,
            },
            dimensions: Vec::new(),
            type_args: Vec::new(),
            span,
        }
    }

    /// The declared type and unpacked dimensions of the variable `name`
    /// heading an array-query operand (see `query_operand_decl_dims`): a
    /// formal or local of the running subroutine, else a property of the
    /// method's class (flagged `true`), else a design variable.
    pub(super) fn query_root_decl_type(
        &self,
        name: &str,
    ) -> Option<(DataType, Vec<(i64, i64)>, bool)> {
        let frames = match self.method_local_base.last() {
            Some(&base) => self.local_stack.get(base..).unwrap_or(&[]),
            None => &self.local_stack[..],
        };
        let marker = format!("{}.", name);
        let in_frame = frames
            .iter()
            .rev()
            .any(|m| m.contains_key(name) || m.contains_key(marker.as_str()));
        if in_frame {
            if let Some(t) = self.local_typedef_type_of(name) {
                return Some((
                    self.module.typedef_types.get(&t)?.clone(),
                    Vec::new(),
                    false,
                ));
            }
            if let Some(c) = self.local_class_type_of(name) {
                return Some((Self::class_type_ref(&c), Vec::new(), false));
            }
        } else if let Some(Some(ctx)) = self.class_context_stack.last()
            && let Some((dt, udims)) = self.class_property_decl_type(ctx, name)
        {
            return Some((dt, udims, true));
        }
        if let Some(dt) = self.module.var_decl_types.get(name) {
            return Some((dt.clone(), Vec::new(), false));
        }
        self.var_class_types
            .get(name)
            .map(|c| (Self::class_type_ref(c), Vec::new(), false))
    }

    /// The declared type and unpacked dimensions of member `seg` of a value
    /// of type `dt`: a property of a class, or a member of a struct.
    pub(super) fn member_decl_type(
        &self,
        dt: &DataType,
        seg: &str,
    ) -> Option<(DataType, Vec<(i64, i64)>)> {
        if let DataType::TypeReference { name, .. } = dt {
            let cn = name.name.name.as_str();
            let base = cn.split('#').next().unwrap_or(cn);
            if self.module.classes.contains_key(base) {
                return self.class_property_decl_type(base, seg);
            }
        }
        let DataType::Struct(su) = self.resolve_dt(dt) else {
            return None;
        };
        let (mdt, dims) = su.members.iter().find_map(|m| {
            m.declarators
                .iter()
                .find(|md| md.name.name == seg)
                .map(|md| (m.data_type.clone(), md.dimensions.clone()))
        })?;
        let udims = if dims.is_empty() {
            Vec::new()
        } else {
            self.member_dim_indices(&dims)?
                .iter()
                .map(|l| Some((*l.first()?, *l.last()?)))
                .collect::<Option<Vec<_>>>()?
        };
        Some((mdt, udims))
    }

    /// The declared type and fixed unpacked dimensions of property `prop` of
    /// class `cn` or an ancestor; `None` for a queue, dynamic, associative or
    /// parameter-sized property.
    pub(super) fn class_property_decl_type(
        &self,
        cn: &str,
        prop: &str,
    ) -> Option<(DataType, Vec<(i64, i64)>)> {
        let mut cur = Some(cn);
        let mut guard = 0;
        while let Some(c) = cur {
            guard += 1;
            if guard > 64 {
                return None;
            }
            let cd = self.module.classes.get(c)?;
            if cd.properties.contains_key(prop) {
                if cd.queue_properties.contains_key(prop)
                    || cd.assoc_properties.contains_key(prop)
                    || cd.array_of_coll_properties.contains_key(prop)
                    || cd.param_sized_props.contains_key(prop)
                {
                    return None;
                }
                let udims = if let Some((shape, _)) = cd.array_nd_properties.get(prop) {
                    shape.clone()
                } else if let Some(&(lo, hi, _)) = cd.array_properties.get(prop) {
                    vec![(lo, hi)]
                } else {
                    Vec::new()
                };
                return Some((cd.property_types.get(prop)?.clone(), udims));
            }
            cur = cd.extends.as_deref();
        }
        None
    }

    pub(super) fn member_array_query(
        &mut self,
        sn: &str,
        arg: &Expression,
        dim: usize,
    ) -> Option<Value> {
        match &arg.kind {
            ExprKind::Index { .. } | ExprKind::MemberAccess { .. } => {}
            ExprKind::Ident(h) if h.path.len() >= 2 => {}
            _ => return None,
        }
        if !Self::expr_root_first_seg(arg).is_some_and(|r| self.struct_root_possible(r)) {
            return None;
        }
        let flat = self.flat_member_name(arg)?;
        if !flat.contains('.') || self.foreach_dims(&flat).is_some() {
            return None;
        }
        let (_, lists) = self.flat_path_type(&flat)?;
        let list = lists?.get(dim.checked_sub(1)?)?.clone();
        let (left, right) = (*list.first()?, *list.last()?);
        let (lo, hi) = (left.min(right), left.max(right));
        let result = match sn {
            "$left" => left,
            "$right" => right,
            "$low" => lo,
            "$high" => hi,
            "$size" => hi - lo + 1,
            "$increment" => {
                if left >= right {
                    1
                } else {
                    -1
                }
            }
            _ => return None,
        };
        let mut rv = Value::from_u64((result as u64) & 0xFFFF_FFFF, 32);
        rv.is_signed = true;
        Some(rv)
    }

    /// Every element of a member with fixed unpacked dimensions, as one index
    /// tuple each, in element order (last dimension varying fastest) — the
    /// cartesian product of [`member_dim_indices`].
    pub(super) fn member_elem_indices(
        &self,
        dims: &[crate::ast::types::UnpackedDimension],
    ) -> Option<Vec<Vec<i64>>> {
        self.member_dim_indices(dims)
            .map(|lists| Self::cartesian_indices(&lists))
    }

    /// Cheap gate for the member-wise struct paths that must flatten a name
    /// first: can a variable whose name (or instance-local last segment, or
    /// leading scope segment) is `root` hold an UNPACKED struct — itself, as
    /// an array / queue / associative element, or as a block-local? One hash
    /// probe, so the ordinary element selects and nonblocking assignments of
    /// a design pay nothing for the struct handling.
    pub(super) fn struct_root_possible(&mut self, root: &str) -> bool {
        if !self.struct_root_names_built {
            self.struct_root_names_built = true;
            let names: Vec<String> = self
                .module
                .var_decl_types
                .iter()
                .filter(|(_, dt)| {
                    matches!(self.resolve_dt_ref(dt),
                        DataType::Struct(su) if Self::spreads_member_wise(su))
                })
                .map(|(n, _)| n.clone())
                .collect();
            for n in names {
                self.note_struct_root(&n);
            }
        }
        self.struct_root_names.contains(root)
    }

    /// Record `name` for `struct_root_possible`: the full name, its first
    /// segment and its last segment (an instance-local variable is written
    /// under its bare name inside the instance).
    pub(super) fn note_struct_root(&mut self, name: &str) {
        let first = name.split(['.', '[']).next().unwrap_or(name);
        let last = name.rsplit('.').next().unwrap_or(name);
        let last = last.split('[').next().unwrap_or(last);
        for n in [name, first, last] {
            if !self.struct_root_names.contains(n) {
                self.struct_root_names.insert(n.to_string());
            }
        }
    }

    /// Does any unpacked struct type of the design have a member with two or
    /// more unpacked dimensions? Only such a member has element leaves that
    /// `struct_member_elem_leaf` must find, so a design without one skips
    /// that name flattening entirely. Computed once over the declared
    /// variable, typedef and class-property types; a block-local struct of
    /// a new anonymous type updates it through `note_struct_type`.
    #[inline]
    pub(super) fn multidim_members_possible(&mut self) -> bool {
        match self.multidim_members {
            Some(b) => b,
            None => self.compute_multidim_members(),
        }
    }

    #[cold]
    pub(super) fn compute_multidim_members(&mut self) -> bool {
        let found = self
            .module
            .var_decl_types
            .values()
            .chain(self.module.typedef_types.values())
            .chain(
                self.module
                    .classes
                    .values()
                    .flat_map(|c| c.property_types.values()),
            )
            .any(|dt| self.dt_has_multidim_member(dt, 0));
        self.multidim_members = Some(found);
        found
    }

    /// `multidim_members_possible` bookkeeping for a type declared at run
    /// time (a block-local struct).
    pub(super) fn note_struct_type(&mut self, dt: &DataType) {
        if self.multidim_members == Some(false) && self.dt_has_multidim_member(dt, 0) {
            self.multidim_members = Some(true);
        }
    }

    pub(super) fn dt_has_multidim_member(&self, dt: &DataType, depth: u32) -> bool {
        if depth > 8 {
            return false;
        }
        let DataType::Struct(su) = self.resolve_dt_ref(dt) else {
            return false;
        };
        su.members.iter().any(|m| {
            m.declarators.iter().any(|d| d.dimensions.len() >= 2)
                || self.dt_has_multidim_member(&m.data_type, depth + 1)
        })
    }

    /// The first path segment of the identifier at the root of a
    /// member/index chain (`s` in `s.mm[1][0]`, `arr` in `arr[2].x`).
    pub(super) fn expr_root_first_seg(e: &Expression) -> Option<&str> {
        match &e.kind {
            ExprKind::Ident(h) => h.path.first().map(|s| s.name.name.as_str()),
            ExprKind::MemberAccess { expr, .. } | ExprKind::Index { expr, .. } => {
                Self::expr_root_first_seg(expr)
            }
            _ => None,
        }
    }

    /// `[i][j]...` for an element index tuple — the key suffix of a member
    /// element's leaf.
    pub(super) fn index_suffix(tup: &[i64]) -> String {
        let mut s = String::with_capacity(tup.len() * 4);
        for &i in tup {
            s.push('[');
            Self::push_i64(&mut s, i);
            s.push(']');
        }
        s
    }

    /// Write one flattened leaf, honouring the declared width / signedness /
    /// real-ness when the signal is registered. An associative-array element
    /// has no pre-registered signal, so it is created at its natural width.
    pub(super) fn write_leaf_by_name(&mut self, flat: &str, val: Value) {
        if let Some(&id) = self.signal_name_to_id.get(flat) {
            let width = self.signal_widths[id];
            let mut resized = if self.signal_real[id] {
                if val.is_real {
                    val
                } else {
                    Value::from_f64(val.to_f64())
                }
            } else if val.is_real {
                Self::real_to_int(val.to_f64(), width)
            } else {
                val.resize(width)
            };
            resized.is_signed = self.signal_signed[id];
            self.set_signal_value_by_name(flat, resized);
            return;
        }
        self.set_signal_value_by_name(flat, val);
    }

    // =====================================================================
    // IEEE 1800-2017 §18.4 — aggregate CLASS properties
    //
    // A class property whose type is a struct/union needs its members to be
    // reachable both from OUTSIDE (`bus.packed_struct_inst.payload`) and from
    // INSIDE the class (a bare `packed_struct_inst.payload` in a constraint).
    // Class properties live in `heap[h].properties` (a name → Value map), not
    // in the signal table, so the ordinary struct paths — which key off flat
    // signal names — never reached them: reads returned X and the constraint
    // solver had nothing to pin.
    //
    // §18.4 also fixes the SEMANTICS: a packed struct/union is a raw integral
    // type, so its members alias bit slices of ONE value (a union's members
    // all alias offset 0), and "the rules in 18.3 restricting the random
    // values of an enum variable shall not apply" to an enum member of one.
    // An UNPACKED struct is solved member by member, each member its own cell.
    // =====================================================================

    /// Split `e` into its receiver expression and trailing member name,
    /// accepting BOTH parse shapes: `MemberAccess{Ident(a), b}` (what a
    /// constraint body produces) and a flattened `Ident([a, b])`.
    pub(super) fn split_trailing_member(e: &Expression) -> Option<(Expression, String)> {
        match &e.kind {
            ExprKind::MemberAccess { expr, member } => {
                Some(((**expr).clone(), member.name.clone()))
            }
            ExprKind::Ident(h) if h.path.len() >= 2 => {
                if h.path.iter().any(|s| !s.selects.is_empty()) {
                    return None;
                }
                let mut base = h.clone();
                let last = base.path.pop()?;
                base.cached_signal_id = std::cell::Cell::new(None);
                base.cached_resolved_name = std::cell::OnceCell::new();
                Some((
                    Expression {
                        kind: ExprKind::Ident(base),
                        span: e.span,
                        cached_width: std::cell::Cell::new(None),
                    },
                    last.name.name,
                ))
            }
            _ => None,
        }
    }

    /// Resolve a receiver expression to the `(handle, property)` pair it names:
    /// a bare `prop` inside a class method/constraint (implicit `this`), or an
    /// `<obj>.prop` access from outside.
    /// A packed integer vector whose every dimension is a literal range
    /// (`[15:0]`, not `[W-1:0]`): its width is known without any parameter
    /// or class type-argument lookup. A scalar `bit`/`logic`/`reg` (no
    /// dimension) is one bit wide (§6.11): a class-method formal `bit a`
    /// bound to the literal `1` kept the actual's 32 bits, so `%b` printed
    /// 32 digits and `a` given 2 read nonzero (#272).
    pub(super) fn packed_dims_are_literal(dt: &DataType) -> bool {
        use crate::ast::types::PackedDimension;
        match dt {
            DataType::IntegerVector { dimensions, .. } => dimensions.iter().all(|d| match d {
                PackedDimension::Range { left, right, .. } => {
                    matches!(left.kind, ExprKind::Number(_))
                        && matches!(right.kind, ExprKind::Number(_))
                }
                PackedDimension::Unsized(_) => false,
            }),
            _ => false,
        }
    }

    /// Superset of the property names `class_prop_struct` can answer for: a
    /// declared (or typedef-resolved) struct type, or a type-parameter type
    /// whose binding may be a struct. Class and typedef tables are fixed at
    /// run time, so any other name is never a struct property and its
    /// receiver need not be evaluated to find that out.
    pub(super) fn struct_prop_name_possible(&self, prop: &str) -> bool {
        let set = self.struct_capable_names();
        !set.is_empty() && set.contains(prop)
    }

    pub(super) fn struct_capable_names(&self) -> &HashSet<String> {
        self.struct_capable_prop_names.get_or_init(|| {
            let mut set: HashSet<String> = HashSet::default();
            for cd in self.module.classes.values() {
                for (name, dt) in &cd.property_types {
                    // A static of a class-local struct typedef (`typedef
                    // struct {..} e_t; static e_t one;` in a parameterized
                    // class) through the class chain first.
                    let dt = if cd.static_properties.contains(name) {
                        self.class_prop_decl_type(cd, name).unwrap_or(dt)
                    } else {
                        dt
                    };
                    let dt = Self::resolve_type_ref(dt, &self.module.typedef_types);
                    if matches!(dt, DataType::Struct(_)) {
                        set.insert(name.clone());
                    }
                }
                for (name, sig) in &cd.properties {
                    let Some(tn) = sig.type_name.as_ref() else {
                        continue;
                    };
                    let capable = cd.type_param_names.contains(tn)
                        || self
                            .module
                            .typedef_types
                            .get(tn.as_str())
                            .is_some_and(|dt| {
                                matches!(
                                    Self::resolve_type_ref(dt, &self.module.typedef_types),
                                    DataType::Struct(_)
                                )
                            });
                    if capable {
                        set.insert(name.clone());
                    }
                }
            }
            set
        })
    }

    /// Could `member` be a dynamic/queue/associative member of a struct some
    /// class property resolves to? The class/typedef tables are fixed at run
    /// time, so the answer is computed once.
    #[inline]
    pub(super) fn struct_coll_member_possible(&self, member: &str) -> bool {
        let names = self.struct_coll_member_names();
        !names.is_empty()
            && self.struct_coll_member_len_mask.get() & (1u64 << (member.len() & 63)) != 0
            && names.contains(member)
    }

    /// Whether any struct type declares a dynamic member at all — the gate
    /// every struct-collection path checks first.
    #[inline]
    pub(super) fn struct_colls_possible(&self) -> bool {
        !self.struct_coll_member_names().is_empty()
    }

    /// The set [`struct_coll_member_possible`] consults: member names that
    /// some reachable struct type declares with a dynamic/queue/associative
    /// dimension, nested structs included. The walk seeds from every class
    /// property's declared type and every named typedef — a type-parameter
    /// binding can only name a typedef (inline struct type arguments are not
    /// resolvable), and `resolve_dt_ref` resolves through exactly these
    /// tables, so the set is a sound superset of what the probe can match.
    #[inline]
    pub(super) fn struct_coll_member_names(&self) -> &HashSet<String> {
        match self.struct_coll_member_prop_names.get() {
            Some(set) => set,
            None => self.init_struct_coll_member_names(),
        }
    }
}
