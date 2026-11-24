//! Compilation of class methods.

use super::*;

impl<'a> BytecodeCompiler<'a> {
    /// class-perf method-body mode: compile a whole class-method BODY to a
    /// standalone `CompiledBlock`. All-or-nothing — if every statement lowers
    /// to VM bytecode (member access via `this`/class-handle formals, method-
    /// local registers, arithmetic/control flow, `return`), the caller may
    /// run the block instead of the AST interpreter; otherwise `None` keeps
    /// the proven AST path, byte-identical by construction.
    ///
    /// `formals`: (name, declared width) for each port — class-typed ones
    /// are additionally marked in `class_formals` so `.member` on them lowers
    /// to a heap access. `result`: (name, width, is_class) for the implicit
    /// function result (functions only; `None` for tasks). `body` is the
    /// method's statement list. `this` is always available as the receiving
    /// object handle.
    ///
    /// On success the returned block references, in order, the registers:
    /// `result`: (this_reg, result_reg, return_val_reg) where `return_val_reg`
    /// is `Some` only for functions that can `return e`. The caller seeds
    /// `this`/formal registers before executing and reads the result after.
    pub fn compile_class_method(
        &mut self,
        formals: &[(String, u32)],
        class_formals: &HashSet<String>,
        class_locals: &HashSet<String>,
        class_method_names: &HashSet<String>,
        super_method_names: &HashSet<String>,
        method_class_parent: &Option<String>,
        static_bare_targets: &HashMap<String, String>,
        method_formal_widths: &HashMap<String, Vec<u32>>,
        static_member_names: &HashSet<String>,
        class_const_names: &HashSet<String>,
        new_type_map: &HashMap<String, String>,
        scope_static_receivers: &HashSet<String>,
        handle_local_types: &HashMap<String, String>,
        string_method_names: &HashSet<String>,
        functions: &'a HashMap<String, FunctionDeclaration>,
        pkg_enum_consts: &HashMap<String, (u64, u32)>,
        method_class: &str,
        handle_member_types: &'a HashMap<String, HashMap<String, String>>,
        class_shadow_names: &HashSet<String>,
        member_safe_names: &HashSet<String>,
        bare_member_names: &HashSet<String>,
        member_class_names: &HashSet<String>,
        string_member_names: &HashSet<String>,
        coll_member_names: &HashSet<String>,
        static_coll_member_names: &HashSet<String>,
        coll_elem_members: &HashSet<String>,
        string_formals: &HashSet<String>,
        writeback_formals: &[String],
        result: Option<(&str, u32, bool, bool)>,
        ret_is_collection: bool,
        body: &[&crate::ast::stmt::Statement],
    ) -> Option<(CompiledBlock, RegId, Option<RegId>, Option<RegId>)> {
        use crate::ast::stmt::StatementKind;
        let start_len = self.insns.len();
        let start_reg = self.next_reg;
        // Enter method mode; any failure below restores the saved state.
        self.method_mode = true;
        // A statement the compiler cannot lower runs in the interpreter
        // (`StmtFallback`, carrying the register-backed locals it names) and
        // the rest of the body stays compiled; one unsupported construct —
        // a `$sformatf` in a verbose branch — used to send the whole method
        // to the interpreter. `XEZIM_METHOD_STMT_FALLBACK=0` restores that.
        self.allow_ast_fallback = method_stmt_fallback_enabled();
        self.method_writeback_names = writeback_formals.iter().cloned().collect();
        self.method_bound_names = formals.iter().map(|(n, _)| n.clone()).collect();
        self.method_shadowing_regs.clear();
        if let Some((rname, ..)) = result {
            self.method_bound_names.insert(rname.to_string());
        }
        for st in body {
            if let StatementKind::VarDecl { declarators, .. }
            | StatementKind::ParamDecl { declarators, .. } = &st.kind
            {
                self.method_bound_names
                    .extend(declarators.iter().map(|d| d.name.name.clone()));
            }
        }
        self.allow_expr_fallback = false;
        self.allow_waits = false;
        self.method_ret_is_collection = ret_is_collection;
        self.free_function_mode = method_class.starts_with("__free_fn__");

        self.method_static_locals.clear();
        self.class_shadow_names = class_shadow_names.clone();
        self.member_safe_names = member_safe_names.clone();
        self.bare_member_names = bare_member_names.clone();
        self.member_class_names = member_class_names.clone();
        self.string_formal_names = string_formals.clone();
        self.string_member_names = string_member_names.clone();
        self.coll_member_names = coll_member_names.clone();
        self.static_coll_member_names = static_coll_member_names.clone();
        self.coll_elem_member_names = coll_elem_members.clone();
        // Step 9c: the register-route `$cast` admission set — formals, the
        // result cell, and top-level body decls (their declared types are
        // pre-recorded in the runtime overlay at method entry, so
        // `cast_type_ok` sees the same picture as the AST path).
        self.cast_reg_ok_locals.clear();
        for (name, _) in formals {
            self.cast_reg_ok_locals.insert(name.clone());
        }
        if let Some((rname, _, _, _)) = result {
            self.cast_reg_ok_locals.insert(rname.to_string());
        }
        for st in body {
            if let StatementKind::VarDecl { declarators, .. }
            | StatementKind::ParamDecl { declarators, .. } = &st.kind
            {
                for d in declarators {
                    self.cast_reg_ok_locals.insert(d.name.name.clone());
                }
            }
        }

        // `this` handle lives in a dedicated register.
        let this_reg = self.alloc_reg();
        self.method_this_reg = Some(this_reg);

        self.method_class = Some(method_class.to_string());

        // Bind formals into registers. Scalar/vector ones become ordinary
        // locals; class-typed ones ALSO join `method_handle_names` so a
        // `.member` on them is a heap access. (Zero-width => width from a
        // later Resize/source; formals default to the caller's resolved
        // width when known.)
        for (name, w) in formals {
            let r = self.alloc_reg();
            // §6.16: a STRING formal has no fixed width — its slot width
            // must be 0 so a later assignment to it does NOT Resize the
            // value to the plan's placeholder width (push_element's
            // `name = {top.get_element_name(), ".", name}` padded the
            // text to 1024 bits and every .len() downstream saw 128 —
            // 7515_printer's columns blew out). Mirrors the task-call
            // formal site (`let w = if formal_is_string { 0 }`).
            let w = if string_formals.contains(name.as_str()) {
                0
            } else {
                *w
            };
            if self.local_var_regs.insert(name.clone(), (r, w)).is_some() {
                // A duplicated formal name (defensive) bails.
                self.bail("method_dup_formal");
            }
            if string_formals.contains(name.as_str()) {
                // Mark it string so `infer_lhs_width` keeps 0 instead of
                // guessing a placeholder width (see the task-path twin).
                self.local_var_is_string.insert(name.clone());
                // Typed like a string body local, so a statement fallback
                // can carry it.
                self.local_kinds.insert(r, LocalKind::Str);
            }
            if class_formals.contains(name) {
                self.method_handle_names.insert(name.clone());
            } else if w > 0 {
                let dt = self.method_formal_types.and_then(|ports| {
                    ports
                        .iter()
                        .position(|p| p.name.name == *name)
                        // An unpacked formal's register is only an argument
                        // placeholder, not a scalar containing its elements.
                        // Its selected writes need the aggregate frame binder.
                        .filter(|&i| ports[i].dimensions.is_empty())
                        .map(|i| Self::port_effective_type(ports, i).clone())
                });
                if let Some(kind) = dt.as_ref().and_then(|dt| {
                    let unpacked_alias = matches!(dt,
                        crate::ast::types::DataType::TypeReference { name, .. }
                        if self.typedef_entry(&self.typedef_coll_dims, name, None)
                            .is_some_and(|(_, dims)| !dims.is_empty()));
                    if unpacked_alias || self.class_alias_dynamic_width(dt) {
                        None
                    } else {
                        self.local_kind_of(dt, None)
                    }
                }) {
                    // §13.5.1: the binding plan has sized the formal in the
                    // receiver's specialization. Use that same width when
                    // materializing its register for an interpreter fallback.
                    let kind = match kind {
                        LocalKind::Int {
                            signed, two_state, ..
                        } => LocalKind::Int {
                            width: w,
                            signed,
                            two_state,
                        },
                        kind => kind,
                    };
                    self.local_kinds.insert(r, kind);
                    // Direct VM calls may skip the AST argument binder. Make
                    // the two-state fact true before optimizers trust this
                    // formal as an X/Z-free source for later local stores.
                    if matches!(
                        kind,
                        LocalKind::Int {
                            two_state: true,
                            ..
                        }
                    ) {
                        self.emit(Insn::DropXz(r));
                    }
                }
            }
        }
        // Step 6: class-typed LOCAL declarations the simulator pre-identified
        // (the compiler cannot see the class table). Stash the names so the
        // VarDecl arm treats them as handle locals.
        self.class_local_names = class_locals.clone();
        self.class_method_names = class_method_names.clone();
        self.super_method_names = super_method_names.clone();
        self.method_class_parent = method_class_parent.clone();
        self.static_bare_targets = static_bare_targets.clone();
        self.method_formal_widths = method_formal_widths.clone();
        // class-perf P2 (typed handle roots): declared class per handle
        // formal / class-typed local, consulted by
        // `method_handle_chain_class` so `sched.m_parent.get_pt()` admits.
        self.method_handle_local_types = handle_local_types.clone();
        self.method_string_ret_names = string_method_names.clone();
        // class-perf P2: the free-function table, for CallFreeFunction
        // lowering and string-return typing of free callees.
        self.set_functions(functions);
        self.pkg_enum_consts = pkg_enum_consts.clone();
        self.static_member_names = static_member_names.clone();
        self.class_const_names = class_const_names.clone();
        self.new_type_map = new_type_map.clone();
        self.scope_static_receivers = scope_static_receivers.clone();
        self.method_class = Some(method_class.to_string());
        self.handle_member_types = Some(handle_member_types);

        // Function result CELL: the implicit name and an explicit `return e`
        // share ONE register, so the LAST write wins — exactly the
        // interpreter's `ret = return_value.take().or(implicit)` (an explicit
        // return overrides a prior `f = ...`, and falling off the end keeps
        // the last `f =`). `method_return_val_reg` aliases `method_result_reg`
        // so the Return handler and the function-name binding land on the
        // same cell.
        let (result_reg, return_val_reg): (Option<RegId>, Option<RegId>) = match result {
            Some((rname, rw, is_class, is_string)) => {
                let r = self.alloc_reg();
                // Return-handler writes also resize to the shared cell width.
                // A CONSTRUCTOR's implicit name is `new` — a keyword. It can
                // never be legally READ as the result variable: `m = new` /
                // `m = new(...)` are ALLOCATIONS (the primary parser's KwNew
                // arm), not result-cell reads. Binding the name would
                // silently compile `m = new;` into "m = <result cell>"
                // (caught by the virtual-method-in-binary regression:
                // pre=0, post=0). Leave the name unbound: an Ident("new")
                // read then bails ident_lookup and the whole body falls to
                // the AST interpreter, which allocates.
                if rname != "new" {
                    self.local_var_regs.insert(rname.to_string(), (r, rw));
                    if !is_class && !is_string && !ret_is_collection && rw > 0 {
                        if let Some(dt) = self.method_return_type {
                            if !self.class_alias_dynamic_width(dt) {
                                if let Some(kind @ LocalKind::Int { width, .. }) =
                                    self.local_kind_of(dt, None)
                                {
                                    // Keep specialization-dependent result widths
                                    // on their existing runtime conversion path.
                                    if width == rw {
                                        self.local_kinds.insert(r, kind);
                                        // Older AST alias-return seeds may be
                                        // four-state; make this compile-time
                                        // two-state fact true before reads.
                                        if matches!(
                                            dt,
                                            crate::ast::types::DataType::TypeReference { .. }
                                        ) && matches!(
                                            kind,
                                            LocalKind::Int {
                                                two_state: true,
                                                ..
                                            }
                                        ) {
                                            self.emit(Insn::DropXz(r));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if is_class {
                        self.method_handle_names.insert(rname.to_string());
                    }
                    // class-perf Step 9f fix: a STRING result cell is a
                    // width-0 string local for infer_lhs_width — without
                    // this, `f = super.m(...)` / `f = bare_call(...)`
                    // assigned a byte-vector result at the 32-bit default
                    // and the store Resize truncated it (the report-message
                    // regression lost the first character of every composed
                    // message).
                    if is_string {
                        self.local_var_is_string.insert(rname.to_string());
                        self.local_kinds.insert(r, LocalKind::Str);
                    }
                }
                self.method_result_reg = Some(r);
                self.method_result_width = if rw > 0 { Some(rw) } else { None };
                self.method_result_is_class = is_class;
                self.method_result_is_string = is_string;
                self.method_ret_is_collection = ret_is_collection;
                self.method_return_val_reg = Some(r);
                (Some(r), Some(r))
            }
            None => (None, None),
        };

        // Compile every body statement. One the compiler cannot lower runs
        // as a statement fallback where it can carry what it touches;
        // anything else fails the whole body.
        let mut ok = true;
        for stmt in body {
            if matches!(
                stmt.kind,
                StatementKind::VarDecl { .. } | StatementKind::ParamDecl { .. }
            ) {
                // Body locals bind themselves into `local_var_regs` via the
                // existing VarDecl arm; only a shadowing/signal collision
                // fails it.
            }
            if !self.compile_stmt(stmt) {
                ok = false;
                break;
            }
        }

        if !ok {
            // Census aid: XEZIM_FALLBACK_SITES=1 — the per-site tracer only
            // covers emit_fallback; a statement that could not be lowered
            // or carried (failing the whole body) surfaces here instead.
            if std::env::var("XEZIM_FALLBACK_SITES")
                .map(|v| v != "0" && !v.is_empty())
                .unwrap_or(false)
            {
                eprintln!(
                    "[FALLBACK] compile-fail reason={} scope={}",
                    self.bail_reason.unwrap_or("?"),
                    self.scope_hint.as_deref().unwrap_or("-")
                );
            }
            // Roll back everything; leave the compiler in non-method mode.
            self.truncate_insns(start_len);
            self.next_reg = start_reg;
            self.method_mode = false;
            self.method_this_reg = None;
            self.method_handle_names.clear();
            self.method_handle_local_types.clear();
            self.method_string_ret_names.clear();
            self.pkg_enum_consts.clear();
            self.functions = None;
            self.function_scope_index = None;
            self.deferred_functions = None;
            self.deferred_tasks = None;
            self.deferred_packages = None;
            self.dpi_import_fds = None;
            self.method_class = None;
            self.handle_member_types = None;
            self.class_method_names.clear();
            self.method_formal_widths.clear();
            self.method_result_reg = None;
            self.method_return_val_reg = None;
            self.method_result_width = None;
            self.method_result_is_class = false;
            self.method_result_is_string = false;
            self.method_ret_is_collection = false;
            self.free_function_mode = false;
            self.string_formal_names.clear();
            self.string_member_names.clear();
            self.coll_member_names.clear();
            self.static_coll_member_names.clear();
            self.coll_elem_member_names.clear();
            self.cast_reg_ok_locals.clear();
            self.method_static_locals.clear();
            self.method_ret_jumps.clear();
            self.bail_reset();
            return None;
        }

        // Patch `return` jumps to the method's common exit (current end of
        // the insn stream).
        let end = self.insns.len() as u32;
        for j in std::mem::take(&mut self.method_ret_jumps) {
            match &mut self.insns[j] {
                Insn::Jump(t) => *t = end,
                _ => unreachable!("method ret jump placeholder"),
            }
        }
        // OBSERVABLE RESULT MARKER: unlike a module block, whose exports are
        // signal writes (side effects every peephole preserves), a class
        // method block's export is the RESULT REGISTER, read by the runtime
        // AFTER `exec_insns` returns. The copy-forwarding peephole would
        // otherwise see `Move(result, v)` as dead inside the block and NOP
        // it (main's `forward_copies` pass; found via the m9h_cfg
        // regression: `return 100 + id` compiled to a block that never
        // wrote the result cell). A trailing `Move(r, r)` is never itself
        // eliminated (the pass skips md == ms) yet reads `r`, keeping every
        // earlier write to the result cell alive. It sits AFTER the common
        // exit label, so every fall-through/`return` path executes it right
        // before the block ends.
        if let Some(r) = result_reg {
            self.emit(Insn::Move(r, r));
        }
        // class-perf row 2: the same OBSERVABLE-EXPORT trick for NON-INPUT
        // (output / inout / ref) formals — the runtime reads their registers
        // after `exec_insns` returns (copying them into the locals frame so
        // the interpreter epilogue's write-back reaches the caller's
        // actuals). Without a trailing self-read the copy-forwarding
        // peephole sees a body-final store to such a formal as dead (`r = 7`
        // in `f(output int r)` compiled to nothing) and the caller observes
        // the stale entry value. The marker sits after the common exit so
        // every fall-through/`return` path executes it.
        for name in writeback_formals {
            if let Some(&(fr, _)) = self.local_var_regs.get(name) {
                self.emit(Insn::Move(fr, fr));
            }
        }
        // §6.21: the same observable-export marker for STATIC locals — the
        // dispatch shell reads their registers after `exec_insns` (to
        // persist them into `static_local_vars` via the exit sync), so a
        // body-final store must not be NOP'd by copy-forwarding either.
        let static_local_slots: Vec<RegId> = self
            .method_static_locals
            .iter()
            .map(|(_, s, _)| *s)
            .collect();
        for slot in static_local_slots {
            self.emit(Insn::Move(slot, slot));
        }

        let block = self.finish();
        // Restore the caller's compiler state (finish consumes self-relevant
        // bookkeeping, so do this AFTER finish via re-entering method mode is
        // NOT done here; the caller owns a fresh compiler per method). The
        // returned block is the only artifact.
        let result_reg = result_reg?;
        Some((block, this_reg, Some(result_reg), return_val_reg))
    }
}
