//! Collection methods, method handles and class-member collections.

use super::*;

impl<'a> BytecodeCompiler<'a> {
    /// Lower an unselected module queue receiver whose storage is known at
    /// compile time. Class and subroutine-local queues use their own paths.
    pub(super) fn compile_module_queue_pop(
        &mut self,
        func: &Expression,
        args: &[Expression],
    ) -> Option<RegId> {
        // Edge blocks already have explicit sensitivity and can mutate
        // the collection through the expression fallback. Combinational
        // compilation cannot infer dependencies from this operation.
        if !self.allow_expr_fallback || self.method_mode || !args.is_empty() {
            return None;
        }
        let (receiver, method) = match &func.kind {
            ExprKind::Ident(h) if h.root.is_none() && h.path.len() >= 2 => {
                let (last, rest) = h.path.split_last()?;
                if h.path.iter().any(|part| !part.selects.is_empty()) {
                    return None;
                }
                (rest, last.name.name.as_str())
            }
            ExprKind::MemberAccess { expr, member } => {
                let ExprKind::Ident(h) = &expr.kind else {
                    return None;
                };
                if h.root.is_some() || h.path.iter().any(|part| !part.selects.is_empty()) {
                    return None;
                }
                (h.path.as_slice(), member.name.as_str())
            }
            _ => return None,
        };
        let front = match method {
            "pop_front" => true,
            "pop_back" => false,
            _ => return None,
        };
        let head = receiver.first()?.name.name.as_str();
        if self.local_var_regs.contains_key(head)
            || self.process_local_names.contains(head)
            || self.local_coll_names.contains(head)
        {
            return None;
        }
        let recv = receiver
            .iter()
            .map(|part| part.name.name.as_str())
            .collect::<Vec<_>>()
            .join(".");
        let queue = self
            .scope_hint
            .as_ref()
            .map(|scope| cat_dot(scope, &recv))
            // Inlined queues are registered as dynamic collections even
            // when the elaborator has no separate queue entry for them.
            .filter(|name| self.is_dynamic_collection(name))
            .or_else(|| self.is_dynamic_collection(&recv).then_some(recv))?;
        let dest = self.alloc_reg();
        self.emit(Insn::QueuePop(
            dest,
            Box::new(Name(queue.into_boxed_str())),
            front,
        ));
        Some(dest)
    }

    /// Must an element STORE to this name avoid the dense array fast path?
    ///
    /// A dynamic array / queue element carries a signal TWIN, so
    /// `lookup_array_name` resolved it and the store wrote that id --- but a
    /// reader of the COLLECTION does not depend on it, so nothing was ever
    /// re-evaluated and readers silently served stale (or x) data. Bailing
    /// sends the enclosing block to the interpreter, whose `assign_value` /
    /// queued-lvalue NBA commit notify correctly --- the same route
    /// ASSOCIATIVE arrays already take just above. Static arrays keep the fast
    /// path: their element ids ARE what readers depend on.
    pub(super) fn collection_store_denied(&self, hier: &HierarchicalIdentifier) -> bool {
        let (Some(d), Some(q)) = (self.dynamic_arrays, self.queue_vars) else {
            return false;
        };
        let raw = Self::hier_raw_name(hier);
        if d.contains(&raw) || q.contains(&raw) {
            return true;
        }
        if let Some(scope) = &self.scope_hint {
            let qual = cat_dot(scope, &raw);
            if d.contains(&qual) || q.contains(&qual) {
                return true;
            }
        }
        hier.path
            .last()
            .is_some_and(|s| d.contains(&s.name.name) || q.contains(&s.name.name))
    }

    /// Is `name` (an `arrays` key) a dynamic array or queue?
    pub(super) fn is_dynamic_collection(&self, name: &str) -> bool {
        self.dynamic_arrays.is_some_and(|d| d.contains(name))
            || self.queue_vars.is_some_and(|q| q.contains(name))
    }

    /// Does this identifier name an associative array (in any of the spellings
    /// the lvalue paths try)?
    pub(super) fn is_assoc_target(&self, hier: &HierarchicalIdentifier) -> bool {
        let Some(m) = self.assoc_arrays else {
            return false;
        };
        let raw = Self::hier_raw_cow(hier);
        if m.contains_key(raw.as_ref()) {
            return true;
        }
        if let Some(scope) = &self.scope_hint {
            if with_dotted(scope, &raw, |k| m.contains_key(k)) {
                return true;
            }
        }
        hier.path
            .last()
            .is_some_and(|s| m.contains_key(&s.name.name))
    }

    /// class-perf Step 9d: compile `foreach (m[k])` / `foreach (this.m[k])`
    /// / `foreach (obj.m[k])` — a class member collection iterable via the
    /// 9b element-access machinery — as a REAL bytecode loop:
    ///
    /// ```text
    ///   ForeachKeys(slot, handle_reg, member)      // key snapshot (Values)
    /// top:
    ///   pos = 0 (hoisted by the caller shape below)
    ///   ForeachNext(ok, slot, var_reg)             // pop next key → var
    ///   BranchIfFalse(ok, end)
    ///   <body>
    ///   Jump(top)
    /// end:
    /// ```
    ///
    /// `continue` patches to the ForeachNext (the var advance IS the loop
    /// head); `break` patches past the exit branch. The loop var is bound
    /// to a VM register inserted into `local_var_regs` for the loop's
    /// duration (outer binding saved/restored, like the For arm's loop
    /// vars) with `reg_var_loop_depth` raised so no body statement can
    /// fall back to AST and mis-read the register as a signal.
    ///
    /// Returns `Some(false)` when the shape is ADMITTED but any part fails
    /// to lower (all-or-nothing: the whole method falls back to the AST
    /// interpreter), `Some(true)` on success, `None` when the shape does
    /// not belong to this arm (callers keep their existing paths).
    ///
    /// Byte-parity notes:
    /// * the key snapshot and each loop-var Value are produced by the
    ///   runtime ForeachKeys/ForeachNext exec arms using the interpreter's
    ///   OWN helpers (`array_iter_keys`, `assoc_index_width_for`, the sync
    ///   foreach's key→Value conversion) — identical iteration set,
    ///   ordering, widths and signedness.
    /// * the body may MUTATE the collection; the interpreter's sync foreach
    ///   also materializes once at entry (the `ForeachTail` walker carries
    ///   the frozen `keys` vector), so a snapshot matches. Only a body
    ///   that SUSPENDS (a wait) can observe the live-size re-check of the
    ///   blocking-unroll path — impossible here (method mode has no waits;
    ///   a task call would defect to AST with the whole method).
    /// * a null dotted receiver resolves no store → NO iterations,
    ///   matching the interpreter's `expr_assoc_name` fall-through.
    /// * the loop var has NO interpreter storage; body statements that
    ///   cannot compile are what force the all-or-nothing fallback, so
    ///   register-only visibility is sound.
    pub(super) fn try_compile_member_foreach(
        &mut self,
        array: &crate::ast::expr::Expression,
        vars: &[Option<crate::ast::Identifier>],
        body: &crate::ast::stmt::Statement,
    ) -> Option<bool> {
        // Single loop variable only.
        let var = match vars {
            [Some(v)] => v.name.clone(),
            _ => return None,
        };
        // The iterable must be exactly a member-collection element-access
        // receiver (bare member / `this.member` / `obj.member`), with the
        // same collision exclusions as the 9b element reads.
        let (coll, bare, base_expr) = self.member_coll_elem_target(array)?;
        // Method mode only; module-level foreach keeps its interpreter paths.
        if !self.method_mode {
            return None;
        }
        // break/continue inside the body are exactly what the REAL loop
        // machinery below supports — do not decline them (unlike the
        // unrolled local-array arm). Body statements that cannot lower
        // fail the loop compile (all-or-nothing) below.
        let _ = body;

        // Dotted receiver: the handle register must be live before the
        // ForeachKeys insn. Bare receivers use the sentinel handle 0.
        let handle_reg = if bare {
            0
        } else {
            let inner = base_expr.clone().expect("dotted base present");
            match self.method_member_handle_reg(&inner) {
                Some(r) => r,
                None => self.compile_expr(&inner, 0)?,
            }
        };

        // Allocate the runtime key slot (an ARENA index, not a register)
        // and the loop var / ok registers. The per-slot position cursor
        // lives IN the arena entry — no register needed.
        let slot = self.next_foreach_slot();
        let var_reg = self.alloc_reg();
        let ok = self.alloc_reg();
        // A queue or dynamic array index is the 32-bit unsigned Value
        // ForeachKeys makes for a store without key facts; typed, a
        // statement fallback in the body can carry it.
        let dense = !Self::split_local_coll_key(&coll).is_some()
            && self
                .coll_owner_class(base_expr.as_ref())
                .and_then(|cls| self.method_admission.as_ref()?.instance_colls.get(&cls))
                .is_some_and(|colls| colls.dense.contains(&coll));
        if dense {
            self.local_kinds.insert(
                var_reg,
                LocalKind::Int {
                    width: 32,
                    signed: false,
                    two_state: true,
                },
            );
        }

        // Bind the loop var BEFORE the key materialization: the snapshot
        // conversion only stores into the var register, but body reads of
        // the var must resolve through local_var_regs for the whole loop.
        let saved_local = self.local_var_regs.insert(var.clone(), (var_reg, 0));
        self.reg_var_loop_depth += 1;

        self.emit(Insn::ForeachKeys(slot, handle_reg, coll.into_boxed_str()));
        let top = self.insns.len() as u32;
        self.emit(Insn::ForeachNext(ok, slot, var_reg));
        let br = self.insns.len();
        self.emit(Insn::BranchIfFalse(ok, 0));

        self.loop_break_patches.push(Vec::new());
        self.loop_continue_patches.push(Vec::new());
        let body_ok = self.compile_stmt(body);
        let cont_patches = self.loop_continue_patches.pop();
        let brk_patches = self.loop_break_patches.pop();
        if !body_ok {
            // Undo the loop var binding and the depth guard before the
            // all-or-nothing failure propagates.
            match saved_local {
                Some(b) => {
                    self.local_var_regs.insert(var.clone(), b);
                }
                None => {
                    self.local_var_regs.remove(&var);
                }
            }
            self.reg_var_loop_depth -= 1;
            self.bail("Foreach_member_body");
            return Some(false);
        }
        self.emit(Insn::Jump(top));
        let end = self.insns.len() as u32;
        if let Insn::BranchIfFalse(reg, _) = self.insns[br] {
            self.insns[br] = Insn::BranchIfFalse(reg, end);
        }
        // `continue` → the loop head (ForeachNext advances the position).
        if let Some(patches) = cont_patches {
            for idx in patches {
                self.insns[idx] = Insn::Jump(top);
            }
        }
        // `break` → past the exit branch.
        if let Some(patches) = brk_patches {
            for idx in patches {
                self.insns[idx] = Insn::Jump(end);
            }
        }

        // Restore the outer loop var binding and the depth guard.
        match saved_local {
            Some(b) => {
                self.local_var_regs.insert(var, b);
            }
            None => {
                self.local_var_regs.remove(&var);
            }
        }
        self.reg_var_loop_depth -= 1;
        Some(true)
    }

    /// Mint the next runtime key-slot index for a compiled foreach. The
    /// arena lives on the Simulator (swapped per method run so nested
    /// compiled calls never clobber an outer loop's keys); the compiler
    /// only hands out consecutive indices.
    fn next_foreach_slot(&mut self) -> u32 {
        let n = self.foreach_slot_count;
        self.foreach_slot_count += 1;
        n
    }

    pub(super) fn stmt_kind_label(stmt: &Statement) -> &'static str {
        match &stmt.kind {
            StatementKind::Null => "Stmt_Null",
            StatementKind::NonblockingAssign { .. } => "Stmt_Nba",
            StatementKind::BlockingAssign { .. } => "Stmt_Blk",
            StatementKind::If { .. } => "Stmt_If",
            StatementKind::Case { .. } => "Stmt_Case",
            StatementKind::SeqBlock { .. } => "Stmt_SeqBlock",
            StatementKind::ParBlock { .. } => "Stmt_ParBlock",
            StatementKind::Expr(_) => "Stmt_Expr",
            StatementKind::For { .. } => "Stmt_For",
            StatementKind::Foreach { .. } => "Stmt_Foreach",
            StatementKind::While { .. } => "Stmt_While",
            StatementKind::DoWhile { .. } => "Stmt_DoWhile",
            StatementKind::Repeat { .. } => "Stmt_Repeat",
            StatementKind::Forever { .. } => "Stmt_Forever",
            StatementKind::TimingControl { .. } => "Stmt_Timing",
            StatementKind::Wait { .. } => "Stmt_Wait",
            StatementKind::Assertion(_) => "Stmt_Assertion",
            StatementKind::VarDecl { .. } | StatementKind::ParamDecl { .. } => "Stmt_VarDecl",
            _ => "Stmt_other",
        }
    }

    /// Register holding `hier` when it names a block-local variable.
    pub(super) fn local_var_reg_of(
        &self,
        hier: &crate::ast::expr::HierarchicalIdentifier,
    ) -> Option<(RegId, u32)> {
        if self.local_var_regs.is_empty() || hier.path.len() != 1 {
            return None;
        }
        let seg = &hier.path[0];
        if !seg.selects.is_empty() {
            return None;
        }
        // A dotted name is normally a hierarchical reference and never a block
        // local — except for the instance-qualified spellings an inlined
        // function body uses for its OWN formals and result, which
        // `compile_pure_call` binds explicitly. `local_var_regs` holds only
        // names this compiler bound, so a hit here is by construction one of
        // those, not a design signal that happens to be dotted.
        self.local_var_regs.get(&seg.name.name).copied()
    }

    /// §13.5.1, §7.10, §10.10: does a call pass an actual where the callee
    /// declares an unpacked-array formal? Every compiled call hands each
    /// actual to the interpreter's binder as the packed value of one
    /// register, which no unpacked array is: a collection literal arrived as
    /// its packed bits split into elements (`{"nop"}` as three one-byte
    /// strings) and a returned collection as an empty one. Such a call stays
    /// on the interpreter. The formal positions are those of the class that
    /// declares the method on the receiver's chain when the receiver's class
    /// is known (`this`, `super`, a typed handle, a class scope), else the
    /// union over every class, free function and task of that name.
    pub(super) fn call_has_coll_formal_actual(
        &self,
        func: &Expression,
        args: &[Expression],
    ) -> bool {
        let (class, method): (Option<String>, &str) = match &func.kind {
            ExprKind::MemberAccess { expr: base, member } => {
                let class = match &base.kind {
                    ExprKind::This => self.method_class.clone(),
                    ExprKind::Ident(h)
                        if h.root.is_none()
                            && h.path.len() == 1
                            && h.path[0].selects.is_empty()
                            && h.path[0].name.name == "super" =>
                    {
                        self.method_class_parent.clone()
                    }
                    ExprKind::Ident(h)
                        if h.root.is_none()
                            && h.path.len() == 1
                            && h.path[0].selects.is_empty()
                            && self
                                .scope_static_receivers
                                .contains(h.path[0].name.name.as_str()) =>
                    {
                        Some(h.path[0].name.name.clone())
                    }
                    _ => self.method_handle_chain_class(base),
                };
                (class, member.name.as_str())
            }
            ExprKind::Ident(h) if h.path.len() == 1 => {
                let m = h.path[0].name.name.as_str();
                let class = self
                    .class_method_names
                    .contains(m)
                    .then(|| self.method_class.clone())
                    .flatten();
                (class, m)
            }
            ExprKind::Ident(h) if h.path.len() == 2 => (
                Some(h.path[0].name.name.clone()),
                h.path[1].name.name.as_str(),
            ),
            _ => return false,
        };
        self.coll_formal_actual(class.as_deref(), method, args.iter())
    }

    /// See `call_has_coll_formal_actual`: `class` is the receiver's declared
    /// class when known. An empty actual takes the formal's default; a named
    /// actual is checked whatever its position.
    pub(super) fn coll_formal_actual<'e>(
        &self,
        class: Option<&str>,
        method: &str,
        args: impl Iterator<Item = &'e Expression>,
    ) -> bool {
        let Some(admission) = self.method_admission.as_ref() else {
            return false;
        };
        let table = &admission.coll_formal_positions;
        if table.is_empty() {
            return false;
        }
        let mut positions: HashSet<usize> = HashSet::default();
        match class
            .map(|c| c.split('#').next().unwrap_or(c))
            .filter(|c| admission.instance_fields.contains_key(*c))
        {
            Some(base) => {
                let mut cur = Some(base);
                let mut guard = 0;
                while let Some(c) = cur
                    && guard < 128
                {
                    if let Some(pos) = table.get(&(c.to_string(), method.to_string())) {
                        positions.extend(pos.iter().copied());
                    }
                    cur = admission.class_parents.get(c).map(|s| s.as_str());
                    guard += 1;
                }
            }
            None => {
                for ((_, m), pos) in table.iter() {
                    if m == method {
                        positions.extend(pos.iter().copied());
                    }
                }
            }
        }
        if positions.is_empty() {
            return false;
        }
        args.enumerate().any(|(i, arg)| match &arg.kind {
            ExprKind::Empty => false,
            ExprKind::NamedArg { .. } => true,
            _ => positions.contains(&i),
        })
    }

    /// class-perf row 2 (non-input formals): after a call whose args were
    /// marshalled into contiguous fresh slots, copy each slot that a bare
    /// register-backed LOCAL was marshalled from back into that local.
    /// For an input formal the slot still holds the marshalled value (a
    /// callee cannot touch the caller's register file), so the Move is
    /// value-neutral; for a non-input (output/inout/ref) formal the
    /// runtime's arg-temp drain writes the callee's final value into the
    /// slot and this Move lands it in the local — the same write the AST
    /// path performs on the actual after the epilogue.
    pub(super) fn emit_arg_slot_writeback(
        &mut self,
        args: &[Expression],
        arg_start: RegId,
        class_method: Option<(&str, &str)>,
    ) -> Option<()> {
        // The VM only returns argument slots to bare register-backed locals.
        // Keep other writable actuals on the AST path, which retains their
        // lvalue expression for the interpreter's argument binding.
        if let Some((rc, method)) = class_method {
            let Some(admission) = self.method_admission.as_ref() else {
                self.bail("method_writeback_metadata");
                return None;
            };
            // Non-input formal positions (output/inout/ref) that apply to
            // THIS call. Keyed by declaring class so one class's `get(output`
            // can't spuriously decline an un-related input-only `get`.
            // §6.18, §8.25.1: a class-scope receiver that names no class (a
            // typedef of one, a type parameter) is resolved only at run time,
            // so it takes the name-global rule too.
            let base = rc.split('#').next().unwrap_or(rc);
            let affected: Option<HashSet<usize>> = if rc == UNTYPED
                || !admission.instance_fields.contains_key(base)
            {
                // Unresolvable receiver: fall back to the conservative
                // name-global rule (union over every class that declares a
                // non-input formal for this method name).
                let any: HashSet<usize> = admission
                    .writeback_positions
                    .iter()
                    .filter(|((_, m), _)| m.as_str() == method)
                    .flat_map(|(_, pos)| pos.iter().copied())
                    .collect();
                if any.is_empty() { None } else { Some(any) }
            } else {
                // Resolved receiver: walk its superclass chain and union the
                // non-input positions of every class-or-ancestor that
                // declares `method`. A non-input formal an ancestor declares
                // is inherited, so this is conservative-safe.
                let mut acc: HashSet<usize> = HashSet::default();
                let mut cur = Some(base);
                let mut guard = 0;
                while let Some(c) = cur
                    && guard < 128
                {
                    if let Some(pos) = admission
                        .writeback_positions
                        .get(&(c.to_string(), method.to_string()))
                    {
                        acc.extend(pos.iter().copied());
                    }
                    cur = admission.class_parents.get(c).map(|s| s.as_str());
                    guard += 1;
                }
                if acc.is_empty() { None } else { Some(acc) }
            };
            if let Some(positions) = affected
                && positions.iter().any(|&i| {
                    args.get(i).is_some_and(|arg| {
                        !matches!(&arg.kind, ExprKind::Ident(h) if self.local_var_reg_of(h).is_some())
                    })
                })
            {
                self.bail("method_writeback_actual");
                return None;
            }
        }
        for (i, a) in args.iter().enumerate() {
            let ExprKind::Ident(h) = &a.kind else {
                continue;
            };
            let Some((src, _)) = self.local_var_reg_of(h) else {
                continue;
            };
            let slot = (arg_start as usize + i) as RegId;
            if slot != src {
                self.emit(Insn::Move(src, slot));
            }
        }
        Some(())
    }

    pub(super) fn method_instance_field_ok(&self, base: &Expression, member: &str) -> bool {
        let Some(class) = self.method_handle_chain_class(base) else {
            return false;
        };
        self.method_admission.as_ref().is_some_and(|admission| {
            admission
                .instance_fields
                .get(&class)
                .is_some_and(|fields| fields.contains(member))
        })
    }

    /// class-perf method-mode: a member read rooted at an ELEMENT of a
    /// class-handle member collection — `m[k].data`, `this.aa[k].field`,
    /// `obj.q[i].m`. The base is an Index whose indexed name is a class
    /// member collection storing HANDLE elements. It lowers in two steps that
    /// mirror the interpreter exactly: `compile_member_coll_elem_read` emits a
    /// `LoadCollElem` whose value IS the element's heap handle, then
    /// `LoadClassMember` reads `member` off that handle. Returns None for any
    /// non-method, non-class-element, or un-admitted shape (the enclosing
    /// statement keeps the AST path).
    pub(super) fn compile_member_handle_elem_member(
        &mut self,
        base: &crate::ast::expr::Expression,
        member: &str,
    ) -> Option<RegId> {
        use crate::ast::expr::ExprKind;
        if !self.method_mode {
            return None;
        }
        let ExprKind::Index {
            expr: coll_base,
            index,
        } = &base.kind
        else {
            return None;
        };
        // Resolve the indexed base to a class-member collection and its
        // element class (`handle_member_types[method_class][coll]` is the
        // STORED element type — Some iff the collection holds handles).
        let (coll, _bare, coll_recv) = self.member_coll_elem_target(coll_base)?;
        let elem_class = self.coll_owner_class(coll_recv.as_ref()).and_then(|cls| {
            self.handle_member_types?
                .get(&cls)?
                .get(coll.as_str())
                .cloned()
        })?;
        // The leaf must be an admitted instance field of the ELEMENT class
        // (statics / non-loadable / unadmitted members stay on the AST).
        if !self.method_admission.as_ref().is_some_and(|admission| {
            admission
                .instance_fields
                .get(&elem_class)
                .is_some_and(|fields| fields.contains(member))
        }) {
            return None;
        }
        // Lower `m[k]` to the element's heap-handle register, then read
        // `member` off it. `compile_member_coll_elem_read` takes the indexed
        // BASE (`Ident(m)`, not the whole Index expr) + index and yields the
        // element value (a class handle here).
        let handle_reg = self.compile_member_coll_elem_read(coll_base, index, 0)?;
        let dest = self.alloc_reg();
        self.emit(Insn::LoadClassMember(
            dest,
            handle_reg,
            member.to_string().into_boxed_str(),
        ));
        Some(dest)
    }

    /// class-perf method-mode: when `base` denotes the receiving object
    /// (`this`) or a method-local CLASS HANDLE, return the VM register whose
    /// value is that object's heap handle — the base register for
    /// LoadClassMember/StoreClassMember. `None` means `base` is not a
    /// method-mode handle (a signal, scalar local, struct — keep the
    /// pre-existing compile paths).
    /// The receiver-handle register for a member-access BASE in method mode,
    /// EMITTING the load when the base is a bare member:
    ///
    /// * `this` → the method's `this` register;
    /// * a handle local/arg → its register;
    /// * a BARE class member Ident → allocate a register and load
    ///   `this.<name>` into it — `m.f()` must dispatch on `this.m`, NOT on
    ///   `this` (a regression here silently dispatched the outer method on
    ///   the receiver itself and lost virtual overrides).
    ///
    /// `bare_member_names` is pre-filtered to loadable instance members
    /// (never statics/arrays/strings). Returns None for anything else; the
    /// caller falls back to `compile_expr(base, 0)` for chains (after the
    /// pure `method_handle_chain_ok` gate).
    pub(super) fn method_member_handle_reg(
        &mut self,
        base: &crate::ast::expr::Expression,
    ) -> Option<RegId> {
        if !self.method_mode {
            return None;
        }
        match &base.kind {
            crate::ast::expr::ExprKind::This => self.method_this_reg,
            crate::ast::expr::ExprKind::Ident(h)
                if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty() =>
            {
                let name = h.path[0].name.name.as_str();
                if self.method_handle_names.contains(name) {
                    return self.local_var_regs.get(name).map(|(r, _)| *r);
                }
                // The member must ALSO be class-typed: a bare integral
                // member is loadable as a VALUE but is not a dispatch
                // handle (see `member_class_names`).
                if self.bare_member_names.contains(name) && self.member_class_names.contains(name) {
                    let this = self.method_this_reg?;
                    let r = self.alloc_reg();
                    self.emit(Insn::LoadClassMember(
                        r,
                        this,
                        name.to_string().into_boxed_str(),
                    ));
                    return Some(r);
                }
                None
            }
            _ => None,
        }
    }

    /// Step 6: is `expr` a chain of class-handle reads rooted at `this` / a
    /// handle local / a bare member — i.e. can `compile_expr` lower it to a
    /// plain LoadClassMember sequence yielding a handle register? A chain
    /// element's member must pass the same shadow/safe test the rvalue arm
    /// applies (statics, enclosing-class members and non-loadable types
    /// refuse). This mirrors `method_member_handle_reg` but is PURE (no insn
    /// emission), so callers can test before committing to lowering.
    pub(super) fn method_handle_chain_ok(&self, expr: &crate::ast::expr::Expression) -> bool {
        if !self.method_mode {
            return false;
        }
        match &expr.kind {
            crate::ast::expr::ExprKind::This => true,
            crate::ast::expr::ExprKind::Ident(h)
                if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty() =>
            {
                let name = h.path[0].name.name.as_str();
                // Chain roots: (a) a handle local/formal/result cell — always
                // class-typed by construction; (b) a bare member that is
                // BOTH bare-key loadable AND class-typed — an integral
                // member is a value, not a dispatch handle (see
                // `member_class_names`).
                self.method_handle_names.contains(name)
                    || (self.bare_member_names.contains(name)
                        && self.member_class_names.contains(name))
                    || self.static_member_names.contains(name)
            }
            crate::ast::expr::ExprKind::MemberAccess {
                expr: inner,
                member,
            } => {
                let m = member.name.as_str();
                let loadable =
                    !(self.class_shadow_names.contains(m) && !self.member_safe_names.contains(m));
                loadable && self.method_handle_chain_ok(inner)
            }
            crate::ast::expr::ExprKind::Index { .. } => {
                self.method_handle_chain_class(expr).is_some()
            }
            _ => false,
        }
    }

    /// class-perf Step 9e: the STATIC CLASS a handle chain rooted at
    /// `this` / a handle local / a bare member evaluates to — TYPED
    /// admission for DISPATCH receivers (`obj.meth()`) and collection
    /// receiver bases. Every MemberAccess link must be a class-typed
    /// member of the previous link's static class: the UNTYPED shadow
    /// check could not see other classes' members at all, so
    /// `m_cb.cnt.num()` admitted `cnt` (inner's ASSOC member) and lowered
    /// it to a garbage `LoadClassMember` "handle" (heartbeat regression:
    /// `uvm_heartbeat::remove` never deleted mc1). Handle
    /// formals/locals/result cells have no type here: they admit as
    /// single-hop roots (the UNTYPED marker) and CHAINS through them
    /// refuse — a conservative decline to the AST interpreter, never a
    /// wrong dispatch. The bare-member ROOT additionally keeps the
    /// bare-key-stability requirement (`bare_member_names`).
    /// §8.23: the class whose body a call to `method` binds to when the
    /// receiver's DECLARED class is `rc` and `method` is STATIC. A static
    /// method needs no object, so this is the legal dispatch for a call
    /// through a null handle (which has no dynamic class to dispatch on).
    /// `None` when no class on the declared chain declares `method` static.
    pub(super) fn declared_static_target(&self, rc: &str, method: &str) -> Option<String> {
        let adm = self.method_admission.as_ref()?;
        let mut cur = rc.split('#').next().unwrap_or(rc).to_string();
        for _ in 0..64 {
            if adm
                .static_methods
                .get(&cur)
                .is_some_and(|m| m.contains(method))
            {
                return Some(cur);
            }
            cur = adm.class_parents.get(&cur)?.clone();
        }
        None
    }

    pub(super) fn method_handle_chain_class(
        &self,
        expr: &crate::ast::expr::Expression,
    ) -> Option<String> {
        if !self.method_mode {
            return None;
        }
        match &expr.kind {
            crate::ast::expr::ExprKind::This => self.method_class.clone(),
            crate::ast::expr::ExprKind::Ident(h)
                if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty() =>
            {
                let name = h.path[0].name.name.as_str();
                // Handle locals/formals/result: a legal single-hop root.
                // class-perf P2: when the plan carried a DECLARED class for
                // the root (handle formal or class-typed local), chains
                // through it resolve STATICALLY — `sched.m_parent.get()`
                // walks `sched` -> its class -> `m_parent` -> that class.
                // Without a declared type the root stays UNTYPED and a
                // chain through it declines (conservative, as before).
                if let Some(cls) = self.method_handle_local_types.get(name) {
                    return Some(cls.clone());
                }
                if self.method_handle_names.contains(name) {
                    return Some(UNTYPED.to_string());
                }
                if self.static_member_names.contains(name) {
                    let cls = self.method_class.as_deref()?;
                    return self.handle_member_types?.get(cls)?.get(name).cloned();
                }
                if self.bare_member_names.contains(name) {
                    // Step 9e: a COLLECTION member carries its ELEMENT class
                    // in handle_member_types (element-typed reads), but it is
                    // not a scalar handle member — a chain through it must
                    // decline, not dispatch on a collection store.
                    if self.coll_member_names.contains(name)
                        || self.static_coll_member_names.contains(name)
                        || self.coll_elem_member_names.contains(name)
                    {
                        return None;
                    }
                    let cls = self.method_class.as_deref()?;
                    return self.handle_member_types?.get(cls)?.get(name).cloned();
                }
                None
            }
            // class-perf P2: a CALL result is typed when its callee
            // resolves to a known class return:
            //   * a method on a typed receiver chain — `g().self().lvl(x)`,
            //     the report macros' `uvm_get_report_object().
            //     uvm_get_report_object()` (the method's declared return
            //     class, from the plan's per-class member map);
            //   * a free function with a plain class return type
            //     (`uvm_get_report_object()`).
            crate::ast::expr::ExprKind::Call { func, .. } => {
                if let crate::ast::expr::ExprKind::MemberAccess { expr: base, member } = &func.kind
                {
                    let cur = self.method_handle_chain_class(base)?;
                    if cur == UNTYPED {
                        return None;
                    }
                    if self.coll_member_names.contains(member.name.as_str()) {
                        return None;
                    }
                    return self
                        .handle_member_types?
                        .get(&cur)?
                        .get(member.name.as_str())
                        .cloned();
                }
                let crate::ast::expr::ExprKind::Ident(h) = &func.kind else {
                    return None;
                };
                if h.root.is_some()
                    || h.path.is_empty()
                    || h.path.last().unwrap().selects.is_empty() == false
                {
                    return None;
                }
                // Package-qualified spellings (`uvm_pkg::uvm_get_report_object()`)
                // resolve by leaf against the plan's function table.
                let fname = h.path.last().unwrap().name.name.as_str();
                let fd = self.functions.as_ref()?.get(fname)?;
                if let crate::ast::types::DataType::TypeReference { name: tn, .. } = &fd.return_type
                    && !tn.name.name.contains('#')
                {
                    return Some(tn.name.name.clone());
                }
                None
            }
            crate::ast::expr::ExprKind::MemberAccess {
                expr: inner,
                member,
            } => {
                let cur = self.method_handle_chain_class(inner)?;
                if cur == UNTYPED {
                    return None;
                }
                let mname = member.name.as_str();
                // Step 9e: collection members are not scalar handle links
                // (see the bare-member arm).
                if self.coll_member_names.contains(mname)
                    || self.static_coll_member_names.contains(mname)
                    || self.coll_elem_member_names.contains(mname)
                {
                    return None;
                }
                self.handle_member_types?.get(&cur)?.get(mname).cloned()
            }
            // An ELEMENT of a member collection of handles (`p.contents[i]`,
            // `q[i]`) is typed by the collection's element class.
            crate::ast::expr::ExprKind::Index {
                expr: coll_base,
                index,
            } => {
                if matches!(index.kind, crate::ast::expr::ExprKind::Dollar) {
                    return None;
                }
                let (coll, _, recv) = self.member_coll_elem_target(coll_base)?;
                let owner = self.coll_owner_class(recv.as_ref())?;
                self.handle_member_types?
                    .get(&owner)?
                    .get(coll.as_str())
                    .cloned()
            }
            _ => None,
        }
    }

    /// class-perf Step 9b: lower a builtin-collection call on a class
    /// member collection to a `CallCollMethod`. `recv` is the receiver
    /// expression (a bare member Ident `q`, `this.q`, or a dotted chain
    /// `obj.coll`); `meth` is the builtin name; `args` the call's actuals.
    /// Returns `Some(dest_reg)` when admitted, `None` to keep the
    /// pre-existing paths (the caller falls through unchanged).
    ///
    /// Admission = arity gate + receiver gate. The METHOD list is exactly
    /// the builtins whose args are values (no ref-key write-backs, no with-
    /// clause iterators): `size`/`num`/`pop_front`/`pop_back` (0 args),
    /// `exists`/`delete`/`push_back`/`push_front` (1), `insert` (2).
    ///
    /// Receiver gate, two shapes:
    ///
    /// * BARE (`q.size()` — parse: MemberAccess{Ident(q), size}): the
    ///   runtime receiver IS `this`. The collection name must be an
    ///   instance collection (assoc/queue keys of the extends chain —
    ///   statics excluded here: dotted statics take a broken interpreter
    ///   path, so they stay AST in BOTH shapes) and must not be a block
    ///   local or signal (a local shadows the member; the compiled block
    ///   cannot see the local's storage). Runtime: `eval_builtin_method`
    ///   on the bare name re-runs the interpreter's own §8.10 rewrite
    ///   (`instance_assoc_member`), which also resolves param-bound
    ///   collections and per-spec static stores — parity by delegation.
    ///   The bare insn still carries the collection NAME (the builtin
    ///   dispatcher keys its store resolution on it); `bare == 1` tells
    ///   the exec arm to pass it straight through instead of resolving a
    ///   dotted receiver first.
    /// * DOTTED (`obj.coll.meth()` / `this.coll.meth()` — parse:
    ///   MemberAccess{MemberAccess{..}, meth}): `coll` must be an instance
    ///   collection name of the class chain AND the base must be a provably
    ///   class-typed chain (the rvalue-arm shadow/safe test — a non-class
    ///   base would change the funnel's meaning). Runtime:
    ///   `handle_collection_name(handle, coll)` mirrors the funnel's
    ///   `expr_assoc_name` receiver arm, then `eval_builtin_method` on the
    ///   scoped store. `this.q` / handle locals / members / chains are all
    ///   admitted — including the formerly BAILED `this.q.size()`.
    ///
    /// Note the AST builtin-name-collision paths that run BEFORE the
    /// member-collection dispatch are all class-name- or super-receiver-
    /// keyed (config_db statics, static methods, super calls, rand_mode,
    /// enum receivers) — none of them fires for a class member collection
    /// name, and the funnel's `expr_assoc_name`-gated member dispatch
    /// (`eval_call_inner`) routes the admitted shapes here byte-for-byte.
    pub(super) fn compile_coll_method_call(
        &mut self,
        recv: &crate::ast::expr::Expression,
        meth: &str,
        args: &[crate::ast::expr::Expression],
    ) -> Option<RegId> {
        if !matches!(
            meth,
            "size"
                | "num"
                | "pop_front"
                | "pop_back"
                | "exists"
                | "delete"
                | "push_back"
                | "push_front"
                | "insert"
        ) {
            return None;
        }
        let arity_ok = match meth {
            "size" | "num" | "pop_front" | "pop_back" => args.is_empty(),
            "exists" | "delete" | "push_back" | "push_front" => args.len() == 1,
            _ => args.len() == 2, // insert
        };
        if !arity_ok {
            return None;
        }
        // Resolve the receiver shape FIRST (pure, no emission), so a
        // rejection leaves the insn stream untouched. (A row-3 local
        // receiver is an OWNED marked key — `\x01name` — hence the Cow.)
        let (coll, bare): (std::borrow::Cow<'_, str>, bool) = match &recv.kind {
            // `q.meth()` — the receiver IS `this.q`.
            crate::ast::expr::ExprKind::Ident(h)
                if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty() =>
            {
                let name = h.path[0].name.name.as_str();
                // A block local or formal shadows the member collection.
                // (`local_var_regs` also carries the result/handle formals,
                // which are never collection names — the second test is the
                // load-bearing one for shadowing.)
                if self.local_var_regs.contains_key(name) {
                    return None;
                }
                // Row 3: a LOCAL queue / dynamic array declared in this
                // body — receiver marked so the runtime bare arm resolves
                // the interpreter's per-call `@name#id` store instead of
                // the `this` member (a same-named member is shadowed).
                if self.local_coll_names.contains(name) {
                    (std::borrow::Cow::Owned(Self::local_coll_key(name)), true)
                } else {
                    (std::borrow::Cow::Borrowed(name), true)
                }
            }
            // `obj.coll.meth()` / `this.coll.meth()` — the receiver is the
            // collection `coll` on the object the base evaluates to.
            crate::ast::expr::ExprKind::MemberAccess {
                expr: inner,
                member,
            } => {
                // `this.coll` / provably class-typed chain only — TYPED
                // (Step 9e): the base must really evaluate to a handle.
                if self.method_handle_chain_class(inner).is_none() {
                    return None;
                }
                (std::borrow::Cow::Borrowed(member.name.as_str()), false)
            }
            _ => return None,
        };
        // Instance collection (assoc/queue keys, statics excluded) — bare
        // receivers may additionally be a STATIC collection of the chain.
        // A marked local receiver is always admitted (row 3).
        let admitted = self.coll_member_names.contains(coll.as_ref())
            || (bare && self.static_coll_member_names.contains(coll.as_ref()))
            || Self::split_local_coll_key(coll.as_ref()).is_some();
        if !admitted {
            return None;
        }
        // Receiver handle register: bare => `this`; dotted => the base's
        // handle (a bare member base emits its LoadClassMember here).
        let handle_reg = if bare {
            self.method_this_reg?
        } else {
            match self.method_member_handle_reg(recv) {
                Some(r) => r,
                // `recv` is a MemberAccess chain passing chain_ok — lower it
                // through the ordinary rvalue path (loads class members).
                None => self.compile_expr(recv, 0)?,
            }
        };
        // Args as pre-evaluated values in contiguous slots (same protocol
        // as CallMethod).
        let mut arg_values: Vec<RegId> = Vec::with_capacity(args.len());
        for a in args {
            arg_values.push(self.compile_expr(a, 0)?);
        }
        let dest = self.alloc_reg();
        let n = arg_values.len() as u32;
        let arg_start = self.alloc_reg();
        for _ in 1..arg_values.len() {
            self.alloc_reg();
        }
        for (i, &v) in arg_values.iter().enumerate() {
            let slot = (arg_start as usize + i) as RegId;
            if slot != v {
                self.emit(Insn::Move(slot, v));
            }
        }
        self.emit(Insn::CallCollMethod(
            dest,
            handle_reg,
            Box::new(NamePair(coll.as_ref().into(), meth.into())),
            arg_start,
            n,
            bare as u8,
        ));
        self.emit_arg_slot_writeback(args, arg_start, None)?;
        Some(dest)
    }

    /// class-perf row 3: mark a bare name as a LOCAL collection receiver
    /// (a queue / dynamic-array declaration compiled in this method). The
    /// `\x01` prefix is an internal marker — it never appears in user
    /// names — that the runtime elem/call arms strip before resolving the
    /// interpreter's per-call `@name#id` storage key (`dyn_name_lookup`).
    fn local_coll_key(name: &str) -> String {
        format!("\u{1}{}", name)
    }

    /// class-perf collection-return store: the `local_coll_key` of a BARE
    /// local collection (`q`, a queue / unsized dynamic array declared in
    /// this method body): a one-segment Ident with no selects and no scope
    /// root, whose runtime store is the per-call `@name#id` key. Any other
    /// target yields None and keeps the AST fallback path.
    pub(super) fn local_coll_lhs_key(&self, lvalue: &Expression) -> Option<String> {
        match &lvalue.kind {
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.len() == 1
                    && h.path[0].selects.is_empty()
                    && self.local_coll_names.contains(h.path[0].name.name.as_str()) =>
            {
                Some(Self::local_coll_key(h.path[0].name.name.as_str()))
            }
            _ => None,
        }
    }

    /// The result register of a call instruction (the insns whose exec arms
    /// record a returned collection in `vm_ret_coll`).
    pub(super) fn call_insn_dest(insn: &Insn) -> Option<RegId> {
        match insn {
            Insn::CallMethod(d, ..)
            | Insn::CallScopedMethod(d, ..)
            | Insn::CallFreeFunction(d, ..)
            | Insn::CallStaticScoped(d, ..)
            | Insn::CallCollMethod(d, ..) => Some(*d),
            _ => None,
        }
    }

    /// Split a marked local-collection key back to the bare name.
    fn split_local_coll_key(key: &str) -> Option<&str> {
        key.strip_prefix('\u{1}')
    }

    /// Is `base` (select-free) a LOCAL queue / dynamic array of `string`
    /// elements that no block local or formal shadows?
    pub(super) fn local_string_coll(&self, base: &Expression) -> bool {
        match &base.kind {
            ExprKind::Ident(h) if h.root.is_none() && h.path.len() == 1 => {
                let n = h.path[0].name.name.as_str();
                h.path[0].selects.is_empty()
                    && !self.local_var_regs.contains_key(n)
                    && self.local_coll_strings.contains(n)
            }
            _ => false,
        }
    }

    /// Is `base` (select-free) a bare instance-member queue / dynamic /
    /// associative array of `string` elements (`string m_q[$];` read as
    /// `m_q[i]` in a method body) that no local or formal shadows?
    pub(super) fn member_string_coll(&self, base: &Expression) -> bool {
        match &base.kind {
            ExprKind::Ident(h) if self.method_mode && h.root.is_none() && h.path.len() == 1 => {
                let n = h.path[0].name.name.as_str();
                h.path[0].selects.is_empty()
                    && !self.local_var_regs.contains_key(n)
                    && self.member_coll_strings.contains(n)
                    && self.coll_elem_member_names.contains(n)
            }
            _ => false,
        }
    }

    pub(super) fn dt_is_string(dt: &crate::ast::types::DataType) -> bool {
        matches!(
            dt,
            crate::ast::types::DataType::Simple {
                kind: crate::ast::types::SimpleType::String,
                ..
            }
        )
    }

    /// issue-301: the select width of `arr[i][k]` — the first packed dim's
    /// ELEMENT width, when `root` admits to the member element read and the
    /// plan carries multi-dim packed dims for the member. Feeds
    /// `expr_max_width` (self-determined compare/shift contexts).
    pub(super) fn member_coll_elem_index_width(
        &self,
        root: &crate::ast::expr::Expression,
    ) -> Option<u32> {
        let (coll, _, recv) = self.member_coll_elem_target(root)?;
        if recv
            .as_ref()
            .is_some_and(|b| self.foreign_colls(b).is_some())
        {
            return None;
        }
        let dims = self.coll_elem_packed_dims.get(&coll)?;
        if dims.len() != 2 {
            return None;
        }
        Some((dims[1].0 - dims[1].1).abs() as u32 + 1)
    }

    /// issue-301: the FULL element width of a member-collection element
    /// read (`arr[i]`) when the element is a multi-dim packed array — the
    /// product of the plan's packed dims.
    pub(super) fn member_coll_elem_decl_width(
        &self,
        hier: &crate::ast::expr::HierarchicalIdentifier,
    ) -> Option<u32> {
        if hier.root.is_some() || hier.path.len() != 1 || !hier.path[0].selects.is_empty() {
            return None;
        }
        let name = hier.path[0].name.name.as_str();
        if !self.coll_elem_member_names.contains(name) {
            return None;
        }
        let dims = self.coll_elem_packed_dims.get(name)?;
        Some(dims.iter().map(|(l, r)| (l - r).abs() as u32 + 1).product())
    }

    /// The declared ELEMENT width of a member of the element-access plan
    /// set, from the mirrored plan fact (`coll_elem_widths`): fixed-array
    /// members record it in `array_properties`, queues in
    /// `queue_properties`, associative members in their property width.
    /// `explicit_this` (a `this.m[i]` / `this.m` spelling) skips the
    /// local-shadow guard — the spelling names the member unambiguously.
    pub(super) fn member_elem_plan_width(&self, name: &str, explicit_this: bool) -> Option<u32> {
        if !self.coll_elem_member_names.contains(name) {
            return None;
        }
        // A body-local declaration shadows the member (the same
        // precedence `member_coll_elem_target` applies before lowering
        // an element read): plain locals, local collections and local
        // arrays all keep the member's width off their selects.
        if !explicit_this
            && (self.local_var_regs.contains_key(name)
                || self.local_coll_names.contains(name)
                || self.local_var_array.contains_key(name))
        {
            return None;
        }
        self.coll_elem_widths.get(name).copied().filter(|&w| w > 0)
    }

    /// §7.5, §7.10: the declared element width and signedness of a LOCAL
    /// queue / dynamic array (`int a[]` in this body) named by a bare,
    /// unshadowed Ident base of an element select.
    pub(super) fn local_coll_elem_of(
        &self,
        hier: &crate::ast::expr::HierarchicalIdentifier,
    ) -> Option<(u32, Option<bool>)> {
        if hier.root.is_some() || hier.path.len() != 1 || !hier.path[0].selects.is_empty() {
            return None;
        }
        let name = hier.path[0].name.name.as_str();
        if self.local_var_regs.contains_key(name) || !self.local_coll_names.contains(name) {
            return None;
        }
        self.local_coll_elems
            .get(name)
            .copied()
            .filter(|&(w, _)| w > 0)
    }

    /// `member_elem_plan_width` for a bare or `this.`-prefixed Ident base
    /// of an element select.
    pub(super) fn member_elem_plan_width_of(
        &self,
        hier: &crate::ast::expr::HierarchicalIdentifier,
    ) -> Option<u32> {
        if hier.root.is_some() {
            return None;
        }
        match hier.path.as_slice() {
            [seg] if seg.selects.is_empty() => self.member_elem_plan_width(&seg.name.name, false),
            [this_seg, seg]
                if this_seg.selects.is_empty()
                    && seg.selects.is_empty()
                    && this_seg.name.name == "this" =>
            {
                self.member_elem_plan_width(&seg.name.name, true)
            }
            _ => None,
        }
    }

    /// Pure admission probe for member-collection ELEMENT access (Step
    /// 9b-ii): `expr` is the indexed base expression (`q`, `this.aa`,
    /// `obj.aa` — no trailing index). Returns `(collection_name, bare,
    /// dotted_base)` when the receiver shape matches the CallCollMethod
    /// receiver gate: bare = single-segment Ident, non-shadowed, in the
    /// plan's collection set (statics admitted bare-only); dotted =
    /// MemberAccess whose base passes `method_handle_chain_ok` and whose
    /// member is in the INSTANCE collection set (dotted statics stay AST —
    /// their store resolution routes through the broken zero path). No
    /// emission — safe to call speculatively.
    pub(super) fn member_coll_elem_target(
        &self,
        expr: &crate::ast::expr::Expression,
    ) -> Option<(String, bool, Option<crate::ast::expr::Expression>)> {
        match &expr.kind {
            crate::ast::expr::ExprKind::Ident(h)
                if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty() =>
            {
                let name = h.path[0].name.name.as_str();
                // A block local or formal shadows the member collection.
                if self.local_var_regs.contains_key(name) {
                    return None;
                }
                // Row 3: a LOCAL queue / dynamic array declared in this
                // body — its storage is the interpreter's per-call
                // `@name#id` key, so the marked name routes the elem
                // read/write/foreach insns through `dyn_name_lookup` —
                // the exact store and precedence the AST path uses.
                if self.local_coll_names.contains(name) {
                    return Some((Self::local_coll_key(name), true, None));
                }
                // BARE element access: the collision-excluded plan set (see
                // `PreboundCompiledMethod::coll_elem_members`).
                if !self.coll_elem_member_names.contains(name) {
                    return None;
                }
                Some((name.to_string(), true, None))
            }
            crate::ast::expr::ExprKind::MemberAccess { expr: base, member } => {
                let m = member.name.as_str();
                // A receiver of another declared class (`p.contents` with
                // `pattern p`) is admitted by THAT class's collections. A
                // STRING collection (`obj.m_q[i]`) is admitted by the
                // class's string-collection set (kept separate — see
                // `instance_string_colls`); its element loads as a real
                // string Value, so `StrOp` typing works.
                self.foreign_coll_admitted(base, m)
                    .then(|| (member.name.clone(), false, Some((**base).clone())))
            }
            // The parser's spelling of a dotted foreach array: `p.contents`.
            crate::ast::expr::ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.len() == 2
                    && h.path.iter().all(|seg| seg.selects.is_empty()) =>
            {
                let base = Self::dotted_head_expr(h);
                let m = h.path[1].name.name.as_str();
                self.foreign_coll_admitted(&base, m)
                    .then(|| (m.to_string(), false, Some(base)))
            }
            _ => None,
        }
    }

    /// `obj.m_q` — is `m_q` a collection member (integral OR string) of
    /// `obj`'s DECLARED class, resolvable through a handle chain?
    /// Returns true for the plan's integral `instance_colls` members and
    /// for string collections (`instance_string_colls`).
    fn foreign_coll_admitted(&self, base: &Expression, m: &str) -> bool {
        match self.foreign_colls(base) {
            Some(colls) => {
                colls.members.contains(m)
                    || self
                        .method_admission
                        .as_ref()
                        .and_then(|a| {
                            self.method_handle_chain_class(base)
                                .and_then(|c| a.instance_string_colls.get(&c))
                        })
                        .is_some_and(|sc| sc.contains(m))
            }
            None => {
                self.method_handle_chain_class(base).is_some() && self.coll_member_names.contains(m)
            }
        }
    }

    /// `h` without its last segment, as the expression a member access
    /// on it would have for its base.
    pub(super) fn dotted_head_expr(h: &crate::ast::expr::HierarchicalIdentifier) -> Expression {
        let mut head = h.clone();
        head.path.truncate(1);
        Expression::new(ExprKind::Ident(head), h.span)
    }

    /// The collection facts of a dotted receiver's declared class, when
    /// that class is known and is not the method's own class (whose facts
    /// the plan sets already carry).
    fn foreign_colls(&self, base: &Expression) -> Option<&ClassColls> {
        let class = self.method_handle_chain_class(base)?;
        if class == UNTYPED || self.method_class.as_deref() == Some(class.as_str()) {
            return None;
        }
        self.method_admission.as_ref()?.instance_colls.get(&class)
    }

    /// The class whose declaration describes a member collection target
    /// from `member_coll_elem_target`: the receiver's declared class for a
    /// foreign receiver, else the method's class.
    pub(super) fn coll_owner_class(&self, base: Option<&Expression>) -> Option<String> {
        if let Some(b) = base
            && self.foreign_colls(b).is_some()
        {
            return self.method_handle_chain_class(b);
        }
        self.method_class.clone()
    }

    /// class-perf Step 9b-ii: lower an ELEMENT READ of a class member
    /// collection (`q[i]`, `aa[k]`, `this.aa[k]`, `obj.aa[k]`) to
    /// `LoadCollElem`. The exec arm mirrors the interpreter's member-element
    /// read body on the resolved store (bare: `instance_assoc_member`;
    /// dotted: `handle_collection_name`), with the index VALUE in a VM
    /// register — the same Value the AST eval of the index expression
    /// produces, so key narrowing is byte-identical.
    ///
    /// REJECTED (stay AST, byte-parity hazards):
    /// * `$` as the index — `q[$]` means LAST-ELEMENT (the live queue
    ///   size via `dollar_bound`, interpreter-only); `compile_expr` cannot
    ///   lower `$` in method mode anyway, so this is a belt-and-braces
    ///   all-or-nothing guard.
    /// * an INDEX EXPRESSION mentioning a string formal, EXCEPT a whole
    ///   string-formal Ident (`aa[nm]`): the register read of the seeded
    ///   formal is byte-exact, while compound string-formal index
    ///   expressions keep the Step 8 bail (char-select / coercion
    ///   differences).
    /// * multi-dim shapes (`q[i][j]`, arrays of collections, fixed
    ///   arrays): different storage machinery (arrays_nd /
    ///   array_of_coll / `base[i]` rows), different read arms.
    pub(super) fn compile_member_coll_elem_read(
        &mut self,
        expr: &crate::ast::expr::Expression,
        index: &crate::ast::expr::Expression,
        ctx_width: u32,
    ) -> Option<RegId> {
        let Some((coll, bare, base_expr)) = self.member_coll_elem_target(expr) else {
            return None;
        };
        // `$` index: q[$] / aa[$] needs the live queue size (dollar_bound) —
        // the AST read arm owns it.
        if matches!(index.kind, crate::ast::expr::ExprKind::Dollar) {
            return None;
        }
        // A string formal anywhere in the index expression: the literal
        // baking cannot reproduce the char-select / coercion semantics —
        // EXCEPT a whole string-formal Ident (a plain register read; the
        // seeded byte-vector Value is exact, and `assoc_key_str`'s narrowing
        // is identical at runtime — the get_child unlock).
        if self.touches_string_formal(index) && !self.index_is_whole_string_formal(index) {
            return None;
        }
        // Dotted receiver: the handle register must be live BEFORE the
        // consumer insn is emitted. A base-compile failure fails the whole
        // read (the statement-level rollback cleans up).
        let elem_is_class = self
            .coll_owner_class(base_expr.as_ref())
            .and_then(|cls| {
                self.handle_member_types?
                    .get(&cls)?
                    .get(coll.as_str())
                    .map(|_| true)
            })
            .unwrap_or(false);
        let handle_reg = if bare {
            0
        } else {
            let inner = base_expr.expect("dotted base present");
            match self.method_member_handle_reg(&inner) {
                Some(r) => r,
                None => self.compile_expr(&inner, 0)?,
            }
        };
        let idx_reg = self.compile_expr(index, 0)?;
        let elem_is_string =
            Self::split_local_coll_key(&coll).is_some_and(|n| self.local_coll_strings.contains(n));
        let dest = self.alloc_reg();
        self.emit(Insn::LoadCollElem(
            dest,
            handle_reg,
            coll.into_boxed_str(),
            idx_reg,
        ));
        // §11.8.1: honor the enclosing context width like every other read
        // (the exec arm stores the raw store width; the caller resizes).
        // EXCEPTION (Step 9e): a HANDLE-typed element must pass through
        // untouched — the interpreter never resizes a class-handle read,
        // and a ctx width of 1 (the `if (refs[i] == v)` condition context)
        // zeroed the handle, killing uvm_packer's cycle detection and
        // recursing forever on cyclic object graphs (6693).
        // §6.16: neither does a STRING element — a string has no fixed width,
        // and a self-determined compare context (`q[0] == b`, sized from
        // the operands' nominal widths) truncated it to a few bits.
        if ctx_width > 0 && !elem_is_class && !elem_is_string {
            self.emit(Insn::Resize(dest, ctx_width));
        }
        Some(dest)
    }

    /// issue-301: `arr[j][k]` / `obj.aa[i][k]` — a further select on a member
    /// collection ELEMENT read whose element type is a multi-dimensional
    /// packed array (`bit [8:0][7:0] arr[3]`). Per §7.4.1, `k` selects an
    /// element of the first packed dim — an ew-bit slice of the loaded
    /// element — while the generic Index tail lowers it to a 1-bit
    /// bit-select of the loaded element (LoadCollElem + BitSelectConst),
    /// silently misreading the value. Lowers LoadCollElem (whole element,
    /// no ctx resize) then RangeSelectConst for constant `k` and
    /// Mul-offset + RangeSelectW for dynamic `k`. Deeper packed chains
    /// (dims.len() != 2), part-select `k` and OOB constants bail to the
    /// AST path (all-or-nothing), matching the interpreter's x semantics.
    pub(super) fn compile_member_coll_elem_packed_select(
        &mut self,
        expr: &crate::ast::expr::Expression,
        index: &crate::ast::expr::Expression,
        ctx_width: u32,
    ) -> Option<RegId> {
        let ExprKind::Index {
            expr: root,
            index: base_idx,
        } = &expr.kind
        else {
            return None;
        };
        // Same admission as the element read itself (bare member in the
        // plan's collision-excluded set, or a dotted `this.aa`/`obj.aa`).
        let (coll, _, recv) = self.member_coll_elem_target(root)?;
        if recv
            .as_ref()
            .is_some_and(|b| self.foreign_colls(b).is_some())
        {
            return None;
        }
        let dims = self.coll_elem_packed_dims.get(&coll)?.clone();
        if dims.len() != 2 {
            // Deeper packed chains: the remaining selects compound — bail
            // to the AST path rather than approximate.
            return None;
        }
        let (l, r) = dims[0];
        let (lo_b, hi_b) = (l.min(r), l.max(r));
        let ew: i64 = (dims[1].0 - dims[1].1).abs() + 1; // element width of dim 0
        // Load the whole element (ctx 0: the slice below carries the width).
        let elem_reg = match self.compile_member_coll_elem_read(root, base_idx, 0) {
            Some(reg) => reg,
            None => {
                // The shape matched but the element read declined ($ index,
                // string formal): keep the whole select on the AST path.
                self.bail("member_coll_elem_packed_select");
                return None;
            }
        };
        // Constant index: a fixed slice [slot*ew +: ew].
        if let Some(iv) = self.fold_const(index).and_then(|v| v.to_u64()) {
            let d = i64::try_from(iv).ok()?;
            if d < lo_b || d > hi_b {
                self.bail("member_coll_elem_packed_select");
                return None;
            }
            // §7.4.1: a descending range labels the LEFT bound as the most
            // significant element; an ascending one reverses the slot order.
            let slot = if l >= r { d - lo_b } else { hi_b - d };
            let off = u32::try_from(slot.checked_mul(ew)?).ok()?;
            let dest = self.alloc_reg();
            self.emit(Insn::RangeSelectConst(
                dest,
                elem_reg,
                off + ew as u32 - 1,
                off,
            ));
            if ctx_width > 0 {
                self.emit(Insn::Resize(dest, ctx_width));
            }
            return Some(dest);
        }
        // Dynamic index: slot = k-lo_b (descending) / hi_b-k (ascending),
        // offset = slot*ew, then `offset +: ew` (RangeSelectW keeps ew on
        // an x/z index, matching the interpreter).
        let k_reg = self.compile_expr(index, 0)?;
        let bound = self.alloc_reg();
        self.emit(Insn::LoadConst(
            bound,
            Box::new(Value::from_u64(
                (if l >= r { lo_b } else { hi_b }) as u64,
                32,
            )),
        ));
        let slot_reg = self.alloc_reg();
        if l >= r {
            self.emit(Insn::Sub(slot_reg, k_reg, bound));
        } else {
            self.emit(Insn::Sub(slot_reg, bound, k_reg));
        }
        self.emit(Insn::Resize(slot_reg, 32));
        let ew_reg = self.alloc_reg();
        self.emit(Insn::LoadConst(
            ew_reg,
            Box::new(Value::from_u64(ew as u64, 32)),
        ));
        let off_reg = self.alloc_reg();
        self.emit(Insn::Mul(off_reg, slot_reg, ew_reg));
        self.emit(Insn::Resize(off_reg, 32));
        let dest = self.alloc_reg();
        self.emit(Insn::RangeSelectW(dest, elem_reg, off_reg, ew as u32, true));
        if ctx_width > 0 {
            self.emit(Insn::Resize(dest, ctx_width));
        }
        Some(dest)
    }

    /// Declared width of an integral member collection element,
    /// for a target `member_coll_elem_target` admits.
    pub(super) fn member_coll_elem_width(
        &self,
        expr: &crate::ast::expr::Expression,
    ) -> Option<u32> {
        let (coll, _, recv) = self.member_coll_elem_target(expr)?;
        let class = self.coll_owner_class(recv.as_ref())?;
        self.method_admission
            .as_ref()?
            .instance_colls
            .get(&class)?
            .elem_widths
            .get(&coll)
            .copied()
    }

    /// Element WRITE counterpart (`q[i] = v`, `aa[k] = v`, dotted forms).
    /// Lowers to `StoreCollElem`; the exec arm re-runs the interpreter's
    /// `assign_value_index` member arm (key narrowing, width fit, queue
    /// append, waiter notify) on the resolved store. The value register
    /// arrives at its RAW width — the BlockingAssign hook runs BEFORE the
    /// `infer_lhs_width` Resize, whose Index arm would guess 1 bit and
    /// destroy the value (the interpreter fits the element width at
    /// runtime, off the live store).
    pub(super) fn compile_member_coll_elem_store(
        &mut self,
        expr: &crate::ast::expr::Expression,
        index: &crate::ast::expr::Expression,
        val_reg: RegId,
    ) -> bool {
        let Some((coll, bare, base_expr)) = self.member_coll_elem_target(expr) else {
            return false;
        };
        if matches!(index.kind, crate::ast::expr::ExprKind::Dollar) {
            return false;
        }
        if self.touches_string_formal(index) && !self.index_is_whole_string_formal(index) {
            return false;
        }
        let Some(idx_reg) = self.compile_expr(index, 0) else {
            return false;
        };
        let handle_reg = if bare {
            0
        } else {
            let inner = base_expr.expect("dotted base present");
            match self.method_member_handle_reg(&inner) {
                Some(r) => r,
                None => match self.compile_expr(&inner, 0) {
                    Some(r) => r,
                    None => return false,
                },
            }
        };
        self.emit(Insn::StoreCollElem(
            handle_reg,
            coll.into_boxed_str(),
            idx_reg,
            val_reg,
        ));
        true
    }
}
