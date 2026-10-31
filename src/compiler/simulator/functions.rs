//! Simulator functions.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// Refcounted handle to a module-scope function declaration. Clones the
    /// AST at most once per name; every later call is a pointer copy. Returns
    /// `None` for a name this module does not declare, exactly like the map
    /// lookup it replaces.
    /// Execute a function dispatched under a hierarchical key (`mm.f()`,
    /// `pif.f()`): §23.4 keeps the body's name resolution in the CALLEE's
    /// lexical home — `name_resolve_hint` parked for the body's duration —
    /// so bare event triggers and reads bind there, not in the caller.
    /// Mirrors the flat-Ident dispatch arm's save/restore discipline.
    pub(super) fn exec_scoped_fn_call(
        &mut self,
        fd: &FunctionDeclaration,
        args: &[Expression],
        joined: &str,
    ) -> Value {
        let scope = joined.rsplit_once('.').map(|(s, _)| s.to_string());
        let saved = self.name_resolve_hint.borrow().clone();
        if let Some(sc) = &scope {
            *self.name_resolve_hint.borrow_mut() = Some(sc.clone());
        }
        let saved_ts = self.timescale_scope_override.take();
        if let Some(_sc) = &scope {
            self.timescale_scope_override = scope.clone();
        }
        let r = self.exec_function_call(fd, args);
        self.timescale_scope_override = saved_ts;
        *self.name_resolve_hint.borrow_mut() = saved;
        r
    }

    pub(super) fn fn_decl_rc(&mut self, name: &str) -> Option<std::rc::Rc<FunctionDeclaration>> {
        if let Some(rc) = self.fn_decl_cache.get(name) {
            return Some(rc.clone());
        }
        let fd = self.module.functions.get(name)?.clone();
        let rc = std::rc::Rc::new(fd);
        self.fn_decl_cache.insert(name.to_string(), rc.clone());
        Some(rc)
    }

    /// DPI import FUNCTION prototypes by SV-visible name, for the bytecode
    /// compiler: a compiled body may call an import directly (e.g.
    /// `uvm_re_compexecfree` under `uvm_re_match`), and the pure-call planner
    /// needs the import's ports to shape the `CallFreeFunction` emission. The
    /// table is immutable after elaboration, so build it once. Task imports
    /// are not callable as expressions and are excluded.
    pub(super) fn dpi_import_fd_table(
        &mut self,
    ) -> std::rc::Rc<HashMap<String, crate::ast::decl::FunctionDeclaration>> {
        if let Some(rc) = &self.dpi_import_fds {
            return rc.clone();
        }
        let mut m: HashMap<String, crate::ast::decl::FunctionDeclaration> = HashMap::default();
        for (name, spec) in &self.module.dpi_imports {
            if let crate::ast::decl::DPIProto::Function(fd) = &spec.proto {
                m.insert(name.clone(), fd.clone());
            }
        }
        let rc = std::rc::Rc::new(m);
        self.dpi_import_fds = Some(rc.clone());
        rc
    }

    /// `fn_decl_rc` for tasks: the task table is never modified after
    /// elaboration, so one shared copy per name serves every call.
    pub(super) fn task_decl_rc(&mut self, name: &str) -> Option<std::rc::Rc<TaskDeclaration>> {
        if let Some(rc) = self.task_decl_cache.get(name) {
            return Some(rc.clone());
        }
        let td = self.module.tasks.get(name)?.clone();
        let rc = std::rc::Rc::new(td);
        self.task_decl_cache.insert(name.to_string(), rc.clone());
        Some(rc)
    }

    pub(super) fn exec_function_call(
        &mut self,
        fd: &FunctionDeclaration,
        args: &[Expression],
    ) -> Value {
        // §6.21 / §13.4.2: a function's locals take ITS lifetime, not the
        // calling task's. A static function called from an automatic task
        // shares its locals across calls, so the caller's per-invocation
        // renaming (`current_auto_task`) is suspended for the call.
        if self.current_auto_task.is_none()
            || matches!(fd.lifetime, Some(crate::ast::types::Lifetime::Automatic))
        {
            return self.exec_function_call_body(fd, args);
        }
        let prev_auto = self.current_auto_task.take();
        let v = self.exec_function_call_body(fd, args);
        self.current_auto_task = prev_auto;
        v
    }

    pub(super) fn exec_function_call_body(
        &mut self,
        fd: &FunctionDeclaration,
        args: &[Expression],
    ) -> Value {
        self.signals.remove("__vif_return__");
        self.vif_return_pending = false;
        // §26.3: the package this body belongs to. Taken before anything else
        // runs, so a call appearing in an argument (evaluated below, in the
        // CALLER's scope) cannot pick it up. A scoped `pkg::f()` dispatch names
        // the package outright; a bare call to an imported one is recovered
        // from its declaring scope.
        let pkg_scope = self
            .pending_pkg_scope
            .take()
            .or_else(|| self.module.func_decl_scope.get(&fd.name.name.name).cloned());
        let normalized = Self::normalize_call_args(&fd.ports, args);
        let supplied = normalized
            .as_ref()
            .map(|_| Self::supplied_argument_mask(&fd.ports, args));
        // The formals' and return variable's metadata are written below.
        for port in fd.ports.iter() {
            self.meta_note(&port.name.name);
        }
        self.meta_note(&fd.name.name.name);
        // §7.4.1: packed-of-packed FORMALS (and the return variable) need
        // their element width registered under the bare formal name, or the
        // body's `x[i]` reads collapse to single-bit selects (same defect as
        // the VarDecl path; see the local-declaration twin).
        for port in fd.ports.iter() {
            if !port.dimensions.is_empty() {
                continue;
            }
            let w = super::elaborate::resolve_type_width(
                &port.data_type,
                Some(&self.module.parameters),
                Some(&self.module.typedefs),
            );
            if let Some(ew) = self.ast_decl_elem_width(&port.data_type) {
                if ew > 0 && w > ew && w % ew == 0 {
                    self.module
                        .packed_signal_elem_widths
                        .insert(port.name.name.clone(), ew);
                } else {
                    self.module
                        .packed_signal_elem_widths
                        .remove(&port.name.name);
                }
            } else {
                self.module
                    .packed_signal_elem_widths
                    .remove(&port.name.name);
            }
        }
        {
            let rw = super::elaborate::resolve_type_width(
                &fd.return_type,
                Some(&self.module.parameters),
                Some(&self.module.typedefs),
            );
            let rname = &fd.name.name.name;
            if let Some(ew) = self.ast_decl_elem_width(&fd.return_type) {
                if ew > 0 && rw > ew && rw % ew == 0 {
                    self.module
                        .packed_signal_elem_widths
                        .insert(rname.clone(), ew);
                } else {
                    self.module.packed_signal_elem_widths.remove(rname);
                }
            } else {
                self.module.packed_signal_elem_widths.remove(rname);
            }
        }
        let args: &[Expression] = normalized.as_deref().unwrap_or(args);
        let arg_supplied = |i: usize| {
            i < args.len()
                && supplied
                    .as_ref()
                    .is_none_or(|mask| mask.get(i).copied().unwrap_or(false))
        };

        // Dedup by borrowed name: the formal list is tiny, so a linear
        // scan beats a HashSet plus one String allocation per formal on
        // EVERY call.
        let mut metadata_names: Vec<&str> = Vec::new();
        let formal_metadata: Vec<FormalMetadataSnapshot> = fd
            .ports
            .iter()
            .map(|port| port.name.name.as_str())
            .chain(std::iter::once(fd.name.name.name.as_str()))
            .filter(|name| {
                if metadata_names.contains(name) {
                    false
                } else {
                    metadata_names.push(name);
                    true
                }
            })
            .map(|name| self.snapshot_formal_metadata(name))
            .collect();

        // Set up local scope with parameters
        let mut locals = self.take_pooled_frame();
        // The callee frame's type overlay, pushed together with `locals`.
        let mut frame_types = self.take_pooled_types();
        self.push_queue_frame();
        let formal_dyn = self.begin_formal_dyn();
        // `output`/`inout`/`ref` formals copy back to the caller's actual on
        // return (e.g. `get_int_arg_value(string s, ref int val)`).
        let mut output_bindings: Vec<(String, Expression)> = Vec::new();
        let mut default_output_bindings = Vec::new();
        // §25.9 via the __vif_local__ convention: a virtual-interface ACTUAL
        // records its interface under the formal's name (cleared otherwise so
        // a stale same-named formal from an earlier call can't leak).
        // Only a formal whose declared type can name an interface (`virtual
        // x_if v`, or a typedef that may resolve to one) can carry a vif;
        // an `int`/`bit`/`string` formal never had a key to set or clear, so
        // the per-port format + resolve + hash was pure overhead on every
        // call of every function.
        let vif_formals: Vec<(&str, Option<&Expression>)> = fd
            .ports
            .iter()
            .enumerate()
            .filter(|(_, port)| Self::formal_may_carry_vif(&port.data_type))
            .map(|(i, port)| {
                (
                    port.name.name.as_str(),
                    args.get(i).filter(|_| arg_supplied(i)),
                )
            })
            .collect();
        let vif_saved = if vif_formals.is_empty() {
            Vec::new()
        } else {
            self.vif_formals_enter(&vif_formals)
        };
        let mut array_writebacks: Vec<(String, String, i64, i64)> = Vec::new();
        let mut array_inputs: Vec<(String, String, i64, i64)> = Vec::new();
        let mut assoc_params: Vec<(String, String, bool, Option<bool>)> = Vec::new();
        let mut queue_writebacks: Vec<(String, String)> = Vec::new();
        // `output`/`inout`/`ref` STRUCT formals are bound member-wise (locals
        // `o.a`, `o.b`), so they bypass the whole-value `output_bindings`
        // path — collect their `(local_key, caller_lvalue)` pairs to copy
        // each member back to the caller's actual on return.
        let struct_output_writebacks: Vec<(String, Expression)> = Vec::new();
        for (i, port) in fd.ports.iter().enumerate() {
            let is_out = matches!(
                port.direction,
                PortDirection::Output | PortDirection::Inout | PortDirection::Ref
            );
            // §13.5.3: an omitted argument binds the formal's DEFAULT through
            // the same aggregate binders as a supplied actual. Skipping them
            // left an array / queue / associative default to the scalar path,
            // which stored it as one packed value, so `v[0]` became a bit
            // select of it.
            let actual = args
                .get(i)
                .filter(|_| arg_supplied(i))
                .or_else(|| Self::default_actual(port));
            // An ASSOCIATIVE-array formal: functions bound nothing at all, so
            // `arr.size()` inside the body read 0.
            if let Some(actual) = actual {
                if let Some((param, caller, prior)) = self.bind_assoc_param(port, actual) {
                    assoc_params.push((param, caller, is_out, prior));
                    continue;
                }
                // §13.5.2: bind member-wise, and for an output/inout/ref struct
                // formal record the copy-back — only the copy-IN existed, so a
                // function taking `output pkt_t p` left the caller's variable
                // untouched. An ARRAY of structs is not this case: it belongs to
                // the array binder below, which copies element by element.
                if port.dimensions.is_empty() {
                    if let Some(backs) = self.bind_unpacked_struct_formal(
                        &port.name.name,
                        &port.data_type,
                        port.direction,
                        actual,
                        &mut locals,
                    ) {
                        // §13.4.2: struct FORMALS bind member-wise (above) but are
                        // skipped by the plain-formal registration, so
                        // `register_formal_type_metadata` below never ran, and a
                        // whole-struct reference to the formal (`y = arg`,
                        // `return arg`) had no type, so `p_elem_type` missed it:
                        // the RHS evaluated to a zeroed container instead of the
                        // members. Register the same struct metadata here too.
                        self.register_formal_type_metadata(
                            &port.name.name,
                            &port.data_type,
                            port.dimensions.is_empty(),
                        );
                        output_bindings.extend(backs);
                        queue_writebacks.append(&mut self.pending_struct_coll_wb);
                        continue;
                    }
                }
            }
            if let Some(actual) = actual {
                // §6.18/§6.20.3: a typedef'd (or type-param-bound) formal
                // carries its unpacked dims on the TYPE, not the port —
                // `function void set(my_array_t val)` bound as a scalar.
                let eff_dims: Vec<crate::ast::types::UnpackedDimension> =
                    if port.dimensions.is_empty() {
                        if let crate::ast::types::DataType::TypeReference { name, .. } =
                            &port.data_type
                        {
                            let tn = &name.name.name;
                            let concrete = self
                                .resolve_type_param_binding(tn)
                                .unwrap_or_else(|| tn.clone());
                            self.module
                                .typedef_unpacked_dims
                                .get(&concrete)
                                .cloned()
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
                    if is_out && !caller.is_empty() {
                        queue_writebacks.push((store, caller));
                    }
                    continue;
                }
            }
            // Unpacked ARRAY formal: functions never bound one, so the body read
            // X and a `ref` write never reached the caller (§13.5.2).
            if let Some(actual) = actual {
                let prior = self.module.arrays.contains_key(&port.name.name)
                    || self.module.arrays_2d.contains_key(&port.name.name)
                    || self.module.arrays_nd.contains_key(&port.name.name);
                if let Some(info) = self.bind_array_arg(port, actual) {
                    // Keep fixed-array formals visible to class-scope lookup (§13.5).
                    locals.insert(
                        format!("\u{1}array_formal:{}", port.name.name),
                        Value::zero(1),
                    );
                    // §7.4.1 / §13.5 / §21.2.1.7: every unpacked formal
                    // retains its element type, including packed geometry.
                    self.register_formal_type_metadata(&port.name.name, &port.data_type, false);
                    if is_out {
                        if !info.1.is_empty() {
                            array_writebacks.push(info);
                        }
                    } else if !prior {
                        // An INPUT array formal is dropped on return too: left
                        // registered, a later call whose same-named formal
                        // took the scalar path found it and behaved
                        // differently (#327's "warming").
                        array_inputs.push(info);
                    }
                    continue;
                }
            }
            // §13.5.3 (1800-2023): an omitted `output`/`inout` argument with a
            // default binds the default as its actual, so `output o = a`
            // copies `o` back to `a` on return.
            if is_out {
                if let Some(actual) = actual.filter(|a| arg_supplied(i) || Self::is_lvalue_expr(a))
                {
                    if arg_supplied(i) {
                        output_bindings.push((port.name.name.clone(), actual.clone()));
                    } else {
                        default_output_bindings.push((port.name.name.clone(), actual.clone()));
                    }
                }
            }
            let mut val = if arg_supplied(i) {
                // §11.6.1/§11.8.2: the ACTUAL is context-determined by this
                // formal's declared width (see `actual_ctx_shape`), so
                // `fwide9(x + y)` over 8-bit x, y adds at 9 bits. `fit_scalar_-
                // formal_seed` below only resizes the value that is handed it,
                // which cannot restore a carry the self-determined width
                // already dropped.
                let value = match self.actual_context_width(port) {
                    Some(w) => self.eval_expr_ctx(&args[i], w),
                    None => self.eval_expr(&args[i]),
                };
                self.vif_argument_value(port, &args[i], value)
            } else if let Some(def) = &port.default {
                // §13.5.3: a default may reference EARLIER formals (`int b =
                // a + 1`). Push the partially-bound frame so they resolve —
                // evaluated bare, `a` was unknown and the default was x.
                self.in_subroutine_default_scope(None, None, |sim| {
                    sim.push_local_frame_typed(locals.clone(), frame_types.clone());
                    let value = sim.eval_formal_default(port, def);
                    let value = sim.vif_argument_value(port, def, value);
                    sim.pop_local_frame();
                    value
                })
            } else {
                Value::zero(32)
            };
            val = self.fit_scalar_formal_seed(port, val);
            if let DataType::TypeReference {
                name: tn,
                type_args,
                ..
            } = &port.data_type
            {
                let type_name = tn.name.name.clone();
                // A class name can also key the typedef WIDTH table, so the
                // class check must run FIRST — otherwise a class-typed formal
                // lands in the typedef maps and `class_of_var` misses it
                // (id-collision-in-heap receiver gating).
                if self.module.classes.contains_key(&type_name) {
                    frame_types
                        .0
                        .insert(port.name.name.clone(), type_name.clone());
                    self.var_class_types
                        .insert(port.name.name.clone(), type_name);
                    self.note_formal_type_args(&port.name.name, type_args);
                } else if self.module.enum_members.contains_key(&type_name)
                    || self.module.typedefs.contains_key(&type_name)
                {
                    frame_types
                        .1
                        .insert(port.name.name.clone(), type_name.clone());
                    self.var_typedef_types
                        .insert(port.name.name.clone(), type_name);
                } else if let Some(concrete) = self.resolve_type_param_binding(&type_name) {
                    // The formal is typed with a class TYPE PARAMETER
                    // (`IMP imp` inside a parameterized class, e.g. the
                    // uvm_tlm_*_socket `new(name, parent, IMP imp=null)`).
                    // `type_name` is not a real class name, so the branches
                    // above silently skip it and `class_of_var` falls back to
                    // a STALE same-named entry left in the flat
                    // `var_class_types` by an unrelated scope (UVM's
                    // config_db methods declare a local `imp` of concrete type
                    // `uvm_config_db_implementation_t`). The local's record is
                    // not scrubbed like a formal's, so `$cast(imp, parent)`
                    // resolved `imp` to that WRONG class and failed it
                    // (UVM/TLM2/NOIMP on a valid initiator socket). Record the
                    // resolved concrete class so `$cast`/`new` see the right
                    // type. Mirrors the local-VarDecl path (§8.25).
                    let cn_is_class = self.module.classes.contains_key(&concrete)
                        || (concrete.contains('#')
                            && self
                                .module
                                .classes
                                .contains_key(concrete.split('#').next().unwrap_or(&concrete)));
                    if cn_is_class {
                        frame_types
                            .0
                            .insert(port.name.name.clone(), concrete.clone());
                        self.var_class_types
                            .insert(port.name.name.clone(), concrete);
                    }
                }
            }
            // Same §13.3 metadata registration as the task path.
            self.register_formal_type_metadata(
                &port.name.name,
                &port.data_type,
                port.dimensions.is_empty(),
            );
            locals.insert(port.name.name.clone(), val);
        }
        // Initialize return variable (function name). Size it to the
        // declared return type's width so a bit-select write
        // `retname[i] = ...` for i >= 32 (e.g. unpack field routines
        // filling a 64-bit integral type) actually lands — a hardcoded
        // 32-bit cell silently dropped the upper half. Also register the
        // width in `self.widths` so a later `retname = <narrow>` assignment
        // zero-extends back to the declared width (mirroring a typed
        // VarDecl), instead of leaving a 32-bit value that drops the high
        // bits on subsequent bit-select writes.
        let ret_name = fd.name.name.name.clone();
        // §6.20.2 / §8.25: a method returning a TYPE PARAMETER (`function T
        // get();`) returns the type bound in this specialization.
        let ret_tp = self
            .local_type_param_concrete(&fd.return_type)
            .and_then(|c| Some((self.concrete_type_width(&c)?, c)));
        let ret_w = match &ret_tp {
            Some((w, _)) => *w,
            None => match self.class_param_sized_width(&fd.return_type) {
                Some(w) => w,
                None => super::elaborate::resolve_type_width(
                    &fd.return_type,
                    Some(&self.module.parameters),
                    Some(&self.module.typedefs),
                ),
            },
        }
        .max(1);
        self.widths.insert(ret_name.clone(), ret_w);
        // §6.11.1: the return variable carries the return type's SIGNEDNESS —
        // the width-fitted frame-write path stamps stored values from
        // `signed_signals`, so an unregistered signed return type (`function
        // integer f`) silently went unsigned and `f >>>= 3` shifted
        // logically (ivtest cfunc_assign_op_vec asr3).
        let ret_signed = match &ret_tp {
            Some((_, c)) => self.concrete_type_signed(c).unwrap_or(false),
            None => super::elaborate::is_type_signed(&fd.return_type),
        };
        if ret_signed {
            self.signed_signals.insert(ret_name.clone());
        } else {
            self.signed_signals.remove(&ret_name);
        }
        // §13.4.1: the return variable is a VARIABLE of the return type — its
        // initial value is the type default: x for a 4-state type, 0 for a
        // 2-state one. An empty `function integer y(); endfunction` returns
        // 'bx (ivtest br_gh337); seeding zero unconditionally masked that.
        let mut ret_init = if super::elaborate::is_type_real(&fd.return_type) {
            Value::from_f64(0.0)
        } else if match &ret_tp {
            Some((_, c)) => self.concrete_type_two_state(c),
            None => super::elaborate::is_type_two_state(&fd.return_type),
        } {
            Value::zero(ret_w)
        } else {
            Value::new(ret_w)
        };
        ret_init.is_signed = ret_signed;
        // §13.3.1/§13.4.1: a STATIC-lifetime function's implicit return
        // variable persists across calls — a path that exits without
        // assigning returns the PREVIOUS call's value, not a fresh default.
        // Only module-scope functions without an `automatic` qualifier
        // qualify (class methods default to automatic, §8.6).
        let static_ret_key = (!matches!(fd.lifetime, Some(crate::ast::types::Lifetime::Automatic))
            && self.module.functions.contains_key(&ret_name))
        .then(|| format!("{}::{}", pkg_scope.as_deref().unwrap_or("-"), ret_name));
        let seeded = static_ret_key
            .as_ref()
            .and_then(|k| self.static_fn_ret.get(k).cloned())
            .unwrap_or(ret_init);
        locals.insert(ret_name.clone(), seeded);
        // §13.4.1: the return variable is a variable of the return type, so it
        // needs the same structural metadata a declaration gets. It had none:
        // a `function pkt_t f(); f.a = ...;` member write found no field layout
        // for the function name and was dropped, while the identical write to a
        // local of that type worked. For an UNPACKED return type the members
        // live in the frame under `<fn>.<member>`, like any other local.
        self.register_formal_type_metadata(&ret_name, &fd.return_type, true);
        if let Some(su) = self.unpacked_struct_of(&fd.return_type) {
            for (k, seed) in self.struct_return_leaf_seeds(&ret_name, &su) {
                locals.insert(k, seed);
            }
        }
        // Register the return variable's declared type (mirroring the port
        // loop above) so a bare `funcname = new(...)` assignment can resolve
        // the class to construct: `get_expr_type_name` / the generic `new`
        // path key off `var_class_types` / `var_typedef_types`. Without this,
        // an implicit `function C f(); ... f = new();` returns null because
        // the `new` falls through to a context-free eval that yields 0.
        if let DataType::TypeReference { name: tn, .. } = &fd.return_type {
            let type_name = tn.name.name.clone();
            // Class check first — see the port loop above (a class name can
            // also key the typedef width table).
            if self.module.classes.contains_key(&type_name) {
                frame_types.0.insert(ret_name.clone(), type_name.clone());
                self.var_class_types.insert(ret_name.clone(), type_name);
            } else if self.module.enum_members.contains_key(&type_name)
                || self.module.typedefs.contains_key(&type_name)
            {
                frame_types.1.insert(ret_name.clone(), type_name.clone());
                self.var_typedef_types.insert(ret_name.clone(), type_name);
            }
        }
        // Mark string-typed return var / params for character indexing.
        // §23.8: a formal's name is local to THIS frame. Recording it in the
        // shared `string_signals` set must not leak past the return — a string
        // formal named `value` (e.g. uvm `pack_string`) would otherwise make a
        // LATER function's integral `value` (e.g. `pack_field_int`) resolve
        // as a string, so `value[i]` byte-selects instead of bit-selecting.
        // Track only the names newly added here (HashSet::insert returns false
        // when the name is already present from an active caller frame, so
        // recursion/nesting is handled) and remove exactly those on exit.
        let mut frame_string_signals: Vec<String> = Vec::new();
        // §6.21/§23.8: make the string-flag registry frame-scoped. A nested
        // frame's non-string local removes its bare name from `string_signals`
        // (the block-local decl clears stale flags), which would otherwise
        // permanently clobber an enclosing scope's `string m`; snapshot here
        // and restore below so a frame leaks neither an add nor a remove.
        self.string_signals_removed.push(Vec::new());
        // §13.4 / §6.21: a non-string formal shadows any string flag its name
        // carries from another scope for the duration of this frame, the way
        // a non-string local declaration does.
        for port in fd.ports.iter() {
            if !Self::is_string_data_type(&port.data_type) {
                self.forget_string_flag(&port.name.name);
            }
        }
        if Self::is_string_data_type(&fd.return_type) {
            if self.string_signals.insert(ret_name.clone()) {
                frame_string_signals.push(ret_name.clone());
            }
        }
        for port in fd.ports.iter() {
            if Self::is_string_data_type(&port.data_type) {
                if self.string_signals.insert(port.name.name.clone()) {
                    frame_string_signals.push(port.name.name.clone());
                }
            }
        }
        // §25.9: virtual-interface FORMALS of a free FUNCTION. Only the
        // task and class-method paths registered these aliases, so a
        // `virtual bus_if v` formal of a function was never bound — `v.data`
        // inside the body resolved to a phantom and every access read 0.
        let mut iface_alias_frame: HashMap<String, String> = HashMap::default();
        for (i, port) in fd.ports.iter().enumerate() {
            // §13.5.3: an omitted argument binds the formal's DEFAULT (§10.9.1),
            // so a virtual-interface formal with a default still aliases the
            // interface the default names (a supplied actual wins; the default
            // only fills an omitted slot).
            let arg = args
                .get(i)
                .filter(|_| arg_supplied(i))
                .or(port.default.as_ref());
            if arg.is_some() {
                if let Some((f, b)) =
                    self.vif_formal_alias(port, args.get(i).filter(|_| arg_supplied(i)), &locals)
                {
                    iface_alias_frame.insert(f, b);
                }
            }
        }
        if !iface_alias_frame.is_empty() {
            self.consume_vif_argument_return();
        }
        self.local_iface_aliases.push(iface_alias_frame);
        self.commit_formal_dyn(formal_dyn);
        self.push_local_frame_typed(locals, frame_types);
        self.open_decl_shadow_frame();
        for port in &fd.ports {
            if let DataType::TypeReference {
                name: tn,
                type_args,
                ..
            } = &port.data_type
            {
                let type_name = tn.name.name.clone();
                if self.module.classes.contains_key(&type_name) {
                    self.record_local_class_type(&port.name.name, &type_name);
                    self.note_formal_type_args(&port.name.name, type_args);
                } else if self.module.enum_members.contains_key(&type_name)
                    || self.module.typedefs.contains_key(&type_name)
                {
                    self.record_local_typedef_type(&port.name.name, &type_name);
                }
            }
        }
        self.return_value = None;
        let saved_break = self.break_flag;
        let saved_continue = self.continue_flag;
        let saved_return = self.return_flag;
        self.break_flag = false;
        self.continue_flag = false;
        self.return_flag = false;
        // §13.4: a free (module/package) function body must NOT see the
        // caller's class context. Without this shield a bare name in the body
        // that collides with a caller property resolves to `<handle>#<member>`
        // (instance_assoc_member outranks the bound formal), so the body
        // mutates the caller's property directly and the copy-out writeback
        // then clobbers it with the stale formal. Args were already bound
        // above, in the caller's context.
        self.this_stack.push(None);
        self.class_context_stack.push(None);
        // Track the running subroutine name so `%m` inside a package function
        // resolves to `<pkg>.<name>` (see the `%m` formatter). Popped after the
        // body; recursion keeps the stack ordered.
        self.func_call_stack.push(fd.name.name.name.clone());
        // §13.4: record whether THIS function returns a collection (queue /
        // unsized dynamic array) so a later `return '{...}` literal can be
        // snapshotted as a whole collection (same snapshot the Ident-return
        // path uses). Popped with `func_call_stack` below; nesting stays
        // ordered.
        self.fn_ret_collection_stack
            .push(self.fn_returns_collection(&fd.return_type));
        // §13.4.1: a function returning a queue, dynamic or associative
        // array (through a typedef) has a return VARIABLE of that type: the
        // body may fill it in place (`f.push_back(v)`, `f[k] = v`, `f = new[n]`,
        // `f = {a, b}`). Declare it as a collection local; falling off the end
        // returns it like `return f;`.
        let ret_coll = self.fn_ret_coll_dims(&fd.return_type);
        let ret_is_assoc = matches!(
            ret_coll.as_ref().and_then(|d| d.first()),
            Some(crate::ast::types::UnpackedDimension::Associative { .. })
        );
        self.fn_ret_assoc_stack.push(ret_is_assoc);
        if ret_coll.is_some() {
            // The scalar return variable seeded above gives way to the
            // collection (an element write `f[i] = v` must not bit-select it).
            if let Some(frame) = self.local_stack.last_mut() {
                frame.remove(&ret_name);
            }
            self.widths.remove(&ret_name);
            let decl = Statement::new(
                StatementKind::VarDecl {
                    data_type: fd.return_type.clone(),
                    lifetime: None,
                    declarators: vec![crate::ast::stmt::VarDeclarator {
                        name: fd.name.name.clone(),
                        dimensions: Vec::new(),
                        init: None,
                        span: fd.span,
                    }],
                },
                fd.span,
            );
            self.exec_statement(&decl);
        }
        // §3.12.1 / §23.4: actuals bind in the caller above; the body
        // resolves free names in its own lexical scope. In particular a
        // root/unit helper must not inherit a child's same-named signals.
        let body_scope = fd
            .name
            .name
            .name
            .rsplit_once('.')
            .map(|(scope, _)| scope.to_string());
        let saved_body_hint = self.name_resolve_hint.replace(body_scope.clone());
        let body_scope = body_scope.unwrap_or_default();
        let saved_body_activation = self.activation_scope.replace(Some(body_scope.clone()));
        let saved_body_scope = std::mem::replace(&mut self.current_scope, body_scope);
        self.pkg_scope_stack.push(pkg_scope.clone());
        let m_fn_entry = self.subroutine_m_entry(
            &fd.name.name.name,
            fd.span,
            self.pkg_scope_stack.last().and_then(|p| p.as_deref()),
        );
        let saved_m_scope_fn = std::mem::replace(&mut self.m_scope_stack, m_fn_entry);
        // §6.21: open a static-local sync frame keyed by this subroutine name.
        let sync_name = self.sync_frame_name(&fd.name.name.name);
        self.static_local_syncs.push((sync_name, Vec::new()));
        // class-perf free-function tier: a package/module-scope function may
        // run as bytecode exactly like a class method. `this` is null here
        // (pushed above), mirroring the interpreter's frame; the plan is
        // keyed by a synthetic class name derived from the declaring scope,
        // so class-dependent plan inputs (member admission sets, extends
        // walks) all see an empty class and every admission decision
        // degrades to the free-function rules. A compiled body hands its
        // result over as `return_value` and the epilogue below runs
        // unchanged, exactly as after an interpreted `return`.
        let compiled = if compiled_methods_enabled() {
            self.try_run_compiled_free_function(fd, pkg_scope.as_deref())
        } else {
            None
        };
        if let Some(cval) = compiled {
            self.return_value = Some(cval);
        } else {
            // Execute function body
            for stmt in &fd.items {
                self.exec_statement(stmt);
                if self.return_value.is_some() || self.return_flag {
                    break;
                }
            }
        }
        if ret_coll.is_some() && self.return_value.is_none() {
            // Hand the return variable's elements over as `return f;` does
            // (evaluating the bare name would call the function again).
            let n = self
                .dyn_name_lookup(&ret_name)
                .map(str::to_string)
                .unwrap_or_else(|| ret_name.clone());
            if self.module.dynamic_arrays.contains(&n) || self.is_associative_array(&n) {
                if ret_is_assoc {
                    self.snapshot_ret_assoc(&n);
                } else if self.queue_elem_struct(&n).is_none() {
                    const RET_SNAP: &str = "__xz_ret_coll__";
                    self.copy_whole_queue(RET_SNAP, &n);
                    self.pending_ret_collection = Some(RET_SNAP.to_string());
                } else {
                    self.pending_ret_collection = Some(n);
                }
                self.return_value = Some(Value::zero(32));
            } else if self.snapshot_ret_fixed(&n) {
                self.return_value = Some(Value::zero(32));
            }
        }
        // §23.8: stop leaking this frame's string formal names into the
        // global set now that the body is done (see frame_string_signals).
        // §6.21/§23.8: also restore the frame-scoped snapshot (see the push
        // at entry), so this frame's non-string locals can't leak a `remove`.
        self.close_decl_shadow_frame();
        for n in &frame_string_signals {
            self.string_signals.remove(n);
        }
        if let Some(removed) = self.string_signals_removed.pop() {
            for n in removed {
                self.string_signals.insert(n);
            }
        }
        self.sync_static_locals();
        *self.name_resolve_hint.borrow_mut() = saved_body_hint;
        *self.activation_scope.borrow_mut() = saved_body_activation;
        self.current_scope = saved_body_scope;
        self.apply_default_formal_outputs(&default_output_bindings);
        let vif_alias_ended = self.local_iface_aliases.pop().unwrap_or_default();
        self.func_call_stack.pop();
        self.fn_ret_collection_stack.pop();
        self.fn_ret_assoc_stack.pop();
        self.pkg_scope_stack.pop();
        self.m_scope_stack = saved_m_scope_fn;
        self.this_stack.pop();
        self.class_context_stack.pop();
        let mut result = if let Some(rv) = self.return_value.take() {
            rv
        } else if let Some(su) = self.unpacked_struct_of(&fd.return_type) {
            // §13.4.1: an UNPACKED-struct return type has no single container —
            // the function-name variable's members live in the frame as leaves.
            // Reading the bare name yielded x, so a function assigning
            // `f.a = ...` or `f = '{...}` returned nothing at all. Collapse the
            // leaves into the packed form while the frame is still live; the
            // caller's assignment scatters them back.
            self.pack_unpacked_struct(&ret_name, &su)
                .unwrap_or_else(|| Value::new(32))
        } else {
            // Return value from function-name variable
            self.local_stack
                .last()
                .and_then(|l| l.get(&ret_name).cloned())
                .unwrap_or(Value::zero(32))
        };
        // §13.4.1: the return value takes the function's declared type — a
        // `function signed [7:0]` truncates its result to 8 bits (150 -> -106),
        // a `function [3:0]` to 4 bits. The return var was sized 32 and never
        // narrowed, so `s_sum = a + b` returned the full-width sum. Resize
        // integral returns to the declared width/signedness (skip real, string,
        // and void — those aren't width-narrowed scalars).
        // An UNPACKED-struct return is already the exact width of its packed
        // leaves. `resolve_type_width` does not count a member's unpacked
        // dimensions, so resizing to it truncated a struct with an array member
        // and dropped the members laid out above the cut.
        if let Some(k) = static_ret_key {
            self.static_fn_ret.insert(k, result.clone());
        }
        let unpacked_struct_ret = self.unpacked_struct_of(&fd.return_type).is_some();
        // §13.4.1: a real returned from an integral function (or the
        // reverse) converts by value first; the fit below then sizes it.
        let ret_conv = self.fn_ret_conv(&fd.return_type);
        if Self::ret_needs_convert(&result, ret_conv) {
            result = Self::ret_convert(result, ret_conv, 0, None);
        }
        if !result.is_real
            && !unpacked_struct_ret
            && !Self::is_string_data_type(&fd.return_type)
            && !matches!(
                fd.return_type,
                crate::ast::types::DataType::Void { .. } | crate::ast::types::DataType::Real { .. }
            )
        {
            let w = super::elaborate::resolve_type_width(
                &fd.return_type,
                Some(&self.module.parameters),
                Some(&self.module.typedefs),
            );
            if w > 0 && w != result.width {
                result = result.resize(w);
            }
            result.is_signed = super::elaborate::is_type_signed(&fd.return_type);
            result = Self::ret_two_state(result, ret_conv);
        }
        // Assoc / fixed-array formals copy element-wise, but the copy is
        // applied only after the caller's shadowed storage is restored
        // below — a callee local with the same bare name as the caller's
        // actual would otherwise let the restore wipe the output writeback.
        let assoc_params = std::mem::take(&mut assoc_params);
        let array_writebacks = std::mem::take(&mut array_writebacks);
        // Capture the callee's queue formals now; the caller's shadowed
        // storage is restored below, and only then is the writeback
        // applied — see `stage_queue_param`.
        let mut staged_wb: Vec<(String, String)> = Vec::new();
        for (param, caller) in &queue_writebacks {
            if caller.is_empty() || param == caller {
                continue;
            }
            let tmp = self.stage_queue_param(param);
            staged_wb.push((tmp, caller.clone()));
        }
        // An assoc formal's per-call key (`@p#0`) is itself torn down by the
        // restore below, so its element data is staged onto a temp key while
        // the storage is still live (mirroring ``stage_queue_param``); fixed-
        // array formals keep bare keys that survive, so their writeback reads
        // ``param`` directly after the restore.
        let assoc_wb = self.stage_assoc_writebacks(
            assoc_params
                .iter()
                .map(|(p, c, out, _)| (p.as_str(), c.as_str(), *out)),
        );
        // Copy `output`/`inout`/`ref` formals back to the caller's actuals
        // before popping this frame's locals.
        let writebacks: Vec<(String, Value, Expression)> = output_bindings
            .iter()
            .filter_map(|(pn, caller)| {
                self.local_stack
                    .last()
                    .and_then(|l| l.get(pn).cloned())
                    .map(|v| (pn.clone(), v, caller.clone()))
            })
            .collect();
        // Member-wise struct output formals: snapshot each member local
        // (`o.a`, `o.b`) before the frame is popped, then write it back to
        // the caller's actual member.
        let struct_wb: Vec<(Expression, Value)> = struct_output_writebacks
            .iter()
            .filter_map(|(local_key, caller_lval)| {
                self.local_stack
                    .last()
                    .and_then(|l| l.get(local_key).cloned())
                    .map(|v| (caller_lval.clone(), v))
            })
            .collect();
        self.pop_local_frame();
        // A vif binding an out/inout formal ended with propagates to the
        // caller's actual — instance prop or another plain name — whatever
        // the actual held on entry.
        let vif_ended = if vif_saved.is_empty() {
            HashMap::default()
        } else {
            let outs: Vec<&str> = writebacks.iter().map(|(pn, _, _)| pn.as_str()).collect();
            self.vif_formals_exit(vif_saved, &vif_formals, &outs)
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
        // `return`/`break`/`continue` are frame-local — restore the caller's
        // flags so the function body can't terminate the caller's loop/block.
        self.break_flag = saved_break;
        self.continue_flag = saved_continue;
        self.return_flag = saved_return;
        self.pop_and_restore_queue_frame();
        // The caller's shadowed storage is restored above — only now can we
        // write back assoc formals (from the staged temp key) and fixed-array
        // formals (from the surviving bare keys), then purge each formal
        // registration. See the deferred-capture block before the pop.
        self.apply_assoc_writebacks(assoc_wb);
        for (param, caller, _is_out, prior) in assoc_params {
            if param == caller {
                // Formal aliases the caller's namespace; the body's writes
                // already live there. Restore the prior registration (a
                // nested same-name formal must not leave the flat
                // `module.associative_arrays` entry wrong).
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
            self.purge_assoc_param(&param, prior);
        }
        self.writeback_array_args(&array_writebacks);
        self.purge_array_formals(&array_writebacks);
        self.purge_array_formals(&array_inputs);
        for (tmp, caller) in staged_wb {
            self.writeback_queue_param(&tmp, &caller);
            self.drop_staged_queue(&tmp);
        }
        for saved in formal_metadata.into_iter().rev() {
            self.restore_formal_metadata(saved);
        }
        result
    }

    /// Execute a module-level task call with arguments.
    pub(super) fn exec_task_call(&mut self, td: &TaskDeclaration, args: &[Expression]) {
        // §26.3: the package this body belongs to — see `exec_function_call`.
        // Taken before the arguments are bound, which happens in the CALLER's
        // scope.
        let pkg_scope = self
            .pending_pkg_scope
            .take()
            .or_else(|| self.module.func_decl_scope.get(&td.name.name.name).cloned());
        let mut cleanup = self.bind_task_frame(td, args, None);
        self.enter_free_task_scope(td, &mut cleanup);
        self.open_decl_shadow_frame();
        // §23.8/§13.3.2: an instance or free task body resolves names
        // lexically, without a class-method caller's `this`/class context.
        // Its task-scope `static` locals are keyed by that context, so with
        // it each CALLING CLASS got its own copy of a cell §13.3.2 makes
        // single "regardless of the number of concurrent activations".
        // `push_instance_task_context` self-guards on the caller having a
        // `this`, so module/package callers are unaffected.
        self.push_instance_task_context(&mut cleanup);
        self.pkg_scope_stack.push(pkg_scope);
        // Execute task body
        for stmt in &td.items {
            self.exec_statement(stmt);
            if self.return_value.is_some() || self.return_flag || self.break_flag {
                break;
            }
        }
        self.pkg_scope_stack.pop();
        self.close_decl_shadow_frame();
        // §9.6.2: `disable <task>` terminates this invocation and no more —
        // the caller resumes. Clear the unwind signal here, or it would leak
        // out and keep later loops from clearing `break_flag`. An instance's
        // task is registered under its qualified name (`u0.early`) while the
        // body's `disable early` names the leaf.
        let tname = td.name.name.name.as_str();
        let tleaf = tname.rsplit('.').next().unwrap_or(tname);
        if matches!(self.disable_target.as_deref(), Some(t) if t == tname || t == tleaf) {
            self.disable_target = None;
            self.disable_check_pending = false;
            self.break_flag = false;
        }
        self.unwind_task_frame(cleanup);
    }

    /// §13.3 / §23.4: arguments bind in the caller, then the free task
    /// body resolves in its declaration's scope, including across suspension.
    pub(super) fn enter_free_task_scope(
        &mut self,
        td: &TaskDeclaration,
        cleanup: &mut TaskCleanup,
    ) {
        let scope = td
            .name
            .name
            .name
            .rsplit_once('.')
            .map(|(scope, _)| scope.to_string())
            .unwrap_or_default();
        cleanup.prev_hint = self.name_resolve_hint.borrow().clone();
        cleanup.frame_scope_hint = Some(scope.clone());
        *self.name_resolve_hint.borrow_mut() = (!scope.is_empty()).then(|| scope.clone());
        let activation = self.activation_scope.replace(Some(scope.clone()));
        let current = std::mem::replace(&mut self.current_scope, scope);
        cleanup.caller_body_scope = Some((activation, current));
    }

    /// Replay the deferred teardown of an inlined (or synchronous) task call:
    /// the work that `exec_task_call` runs after the body. Mirrors the original
    /// inline cleanup exactly.
    pub(super) fn unwind_task_frame(&mut self, mut c: TaskCleanup) {
        // §13.5.3: a default may name an earlier formal; update it before
        // capturing supplied outputs or popping the declaration's scope.
        self.apply_default_formal_outputs(&c.default_output_bindings);
        if c.frame_scope_hint.is_some() {
            *self.name_resolve_hint.borrow_mut() = c.prev_hint.clone();
        }
        if let Some((activation, scope)) = c.caller_body_scope.take() {
            *self.activation_scope.borrow_mut() = activation;
            self.current_scope = scope;
        }
        if let Some(tn) = &c.task_name {
            if let Some(set) = self.active_task_pids.get_mut(tn) {
                set.remove(&self.current_pid);
            }
        }
        // §21.2.1.7 `%m`: restore the caller's lexical scope.
        self.m_scope_stack = c.saved_m_scope;
        self.current_static_task = c.prev_static;
        self.current_auto_task = c.prev_auto;
        if c.pushed_method_this {
            self.this_stack.pop();
            self.class_context_stack.pop();
            self.method_local_base.pop();
            self.current_spec = c.saved_spec;
        }
        if c.modclass.take().is_some() {
            self.modclass_depth.set(self.modclass_depth.get() - 1);
            let scope = self
                .task_cleanup
                .iter()
                .rev()
                .find_map(|t| t.frame_scope_hint.clone())
                .or_else(|| self.process_scope_hint.get(&self.current_pid).cloned())
                .unwrap_or_default();
            *self.activation_scope.borrow_mut() = Some(scope.clone());
            self.current_scope = scope;
        }
        // §13.5.2/§25.9: preserve output identities before removing the
        // callee's binding frame, including an explicit null result.
        let vif_outputs: Vec<_> = c
            .output_bindings
            .iter()
            .filter_map(|(name, actual)| {
                self.local_iface_aliases.last()?.get(name)?;
                Some((
                    actual.clone(),
                    self.iface_alias_for(name).unwrap_or_default(),
                ))
            })
            .collect();
        self.ref_binding_stack.pop();
        self.ref_alias_stack.pop();
        self.ref_identity_stack.pop();
        self.refresh_ref_redirect_hot();
        self.local_iface_aliases.pop();
        if c.pushed_string_formals {
            self.task_string_formals.pop();
        }
        self.continue_flag = c.saved_continue;
        self.sync_static_locals();
        let locals = self.pop_local_frame_take().unwrap_or_default();
        for (key, previous) in c.saved_vif_formals {
            match previous {
                Some(v) => {
                    self.signals.insert(key, v);
                }
                None => {
                    self.signals.remove(&key);
                }
            }
        }
        for (port_name, caller_expr) in &c.output_bindings {
            if let Some(val) = locals.get(port_name) {
                self.assign_value(caller_expr, val);
            }
        }
        for (actual, bound) in vif_outputs {
            if bound.is_empty() {
                let null = Expression::new(ExprKind::Null, actual.span);
                self.bind_static_vif(&actual, &null);
                self.try_bind_virtual_iface(&actual, &null);
                self.assign_value(&actual, &Value::zero(32));
            } else {
                self.vif_bind_actual(&actual, &bound);
            }
        }
        let mut staged_wb: Vec<(String, String)> = Vec::new();
        for (param, caller) in &c.queue_writebacks.clone() {
            if caller.is_empty() || param == caller {
                continue;
            }
            let tmp = self.stage_queue_param(param);
            staged_wb.push((tmp, caller.clone()));
        }
        // Assoc / fixed-array formals copy element-wise, but the copy must be
        // APPLIED only after the caller's shadowed storage is restored below:
        // if the callee declares a local with the same bare name as the
        // caller's actual, the restore (`pop_and_restore_queue_frame`) wipes
        // the caller's bare-name elements — destroying an output/inout/ref
        // writeback done now. Mirror the function path (and the queue staging
        // above). An assoc formal's per-call key is torn down by the restore,
        // so its element data is staged onto a temp key; fixed-array formals
        // keep bare keys that survive.
        let assoc_params = std::mem::take(&mut c.assoc_params);
        let wb = c.array_writebacks.clone();
        let array_params = std::mem::take(&mut c.array_params);
        let assoc_wb = self.stage_assoc_writebacks(
            assoc_params
                .iter()
                .map(|(p, c, out)| (p.as_str(), c.as_str(), *out)),
        );
        let identity_params: std::collections::HashSet<&String> = wb
            .iter()
            .filter(|(p, c2, ..)| p == c2)
            .map(|(p, ..)| p)
            .collect();
        self.break_flag = c.saved_break || self.dpi_unwinding;
        self.return_flag = c.saved_return || self.dpi_unwinding;
        self.pop_and_restore_queue_frame();
        // The caller's shadowed storage is restored above — now apply the
        // deferred assoc (from staged temp keys) and fixed-array writebacks,
        // then purge each formal.
        self.apply_assoc_writebacks(assoc_wb);
        for (param_name, caller_name, _is_out) in assoc_params {
            // `is_out` element copy already applied from the staged temp key
            // above — the formal's per-call key is gone by now, so calling
            // ``writeback_assoc_param`` again would wipe the restored
            // caller's data. Purge the formal's entries/registration alone.
            if param_name == caller_name {
                continue; // identity binding — the keys ARE the caller's
            }
            let prefix = format!("{}[", param_name);
            let keys: Vec<String> = self.signals.keys_with_elem_prefix(&prefix);
            for k in keys {
                self.signals.remove(&k);
            }
            self.module.associative_arrays.remove(&param_name);
            if !self.module.assoc_defaults.is_empty() {
                self.module.assoc_defaults.remove(&param_name);
            }
        }
        self.writeback_array_args(&wb);
        for param_name in &array_params {
            // Identity-bound formals share the caller's storage — see the
            // function-path twin above (audit46 a10).
            if identity_params.contains(param_name) {
                continue;
            }
            let prefix = format!("{}[", param_name);
            let keys: Vec<String> = self.signals.keys_with_elem_prefix(&prefix);
            for k in keys {
                self.signals.remove(&k);
            }
            self.purge_array_formal(param_name);
        }
        for (tmp, caller) in staged_wb {
            self.writeback_queue_param(&tmp, &caller);
            self.drop_staged_queue(&tmp);
        }
        for saved in c.formal_metadata.into_iter().rev() {
            self.restore_formal_metadata(saved);
        }
    }

    /// Register the structural metadata a task/function FORMAL needs so that
    /// member and element selects on it resolve — the same registration a
    /// local `VarDecl` performs (see the packed-struct local path). Formals
    /// were bound as a flat `Value` in `local_stack` with NO metadata, so
    /// `p.field` on a struct-typed formal found no layout and read 0, while
    /// the identical struct declared as a LOCAL worked. Applies to ANSI and
    /// non-ANSI formals alike (both land here), and only to formals with no
    /// unpacked dimensions — unpacked-array formals own a separate copy-in
    /// path.
    /// ONE step through the typedef table. `resolve_typedef_chain` walks to the
    /// end and discards intermediate PACKED DIMENSIONS, so `typedef T1 [3:0] T2;`
    /// would collapse to `logic [7:0]` and the array-ness would be lost. The
    /// immediate definition is what carries the dims.
    pub(super) fn typedef_one_step(&self, dt: &DataType) -> DataType {
        if let DataType::TypeReference { name, .. } = dt {
            if let Some(inner) = self.module.typedef_types.get(&name.name.name) {
                return inner.clone();
            }
        }
        dt.clone()
    }

    /// §13.4: does a function's DECLARED return type denote a collECTION
    /// (queue, or unsized dynamic array)? A queue-typed typedef (e.g.
    /// `typedef chandle data_q[$];` for the uvm_regex_cache's
    /// `optional_data` = `DATA_T[$]`) is a `TypeReference` whose queue-ness
    /// lives in `typedef_unpacked_dims`, not the return `DataType` itself.
    /// Used by the `Return` handler to route a queue-LITERAL return value
    /// (`return '{a,b}` / `return '{}`) through the whole-collection snapshot
    /// so the caller's `q = f()` copies the full queue (an empty `'{}` must
    /// yield size 0, and `'{a,b}` must keep both elements).
    /// The unpacked dimensions of a function return type that names a
    /// queue, dynamic or associative array typedef, or a one-dimensional
    /// fixed unpacked array; `None` otherwise.
    pub(super) fn fn_ret_coll_dims(
        &self,
        dt: &DataType,
    ) -> Option<Vec<crate::ast::types::UnpackedDimension>> {
        use crate::ast::types::UnpackedDimension as UD;
        let DataType::TypeReference {
            name, dimensions, ..
        } = dt
        else {
            return None;
        };
        // A class type is never a collection.
        if !dimensions.is_empty()
            || (name.scopes.is_empty() && self.module.classes.contains_key(&name.name.name))
        {
            return None;
        }
        let start = self
            .class_context_stack
            .last()
            .and_then(|c| c.as_deref())
            .unwrap_or("");
        let dims = if name.scopes.is_empty() {
            Self::typedef_dims_via_tables(&self.module, start, &name.name.name)
        } else {
            Self::typedef_dims_via_tables(&self.module, start, &name.qualified())
                .or_else(|| Self::typedef_dims_via_tables(&self.module, start, &name.name.name))
        }?;
        (matches!(
            dims.first(),
            Some(UD::Queue { .. } | UD::Unsized(_) | UD::Associative { .. })
        ) || (dims.len() == 1
            && matches!(dims.first(), Some(UD::Range { .. } | UD::Expression { .. }))))
        .then_some(dims)
    }

    /// Is `func` a call of a module/package function whose return type is
    /// a queue, dynamic or associative array typedef?
    pub(super) fn call_returns_collection(&self, func: &Expression) -> bool {
        // §13.4: a METHOD returning a collection (`o.build().size()`,
        // `B::make().size()`): the receiver's class, or the scope class of a
        // static call, declares it. Only module functions were recognized,
        // so the collection method ran on nothing and answered 0.
        let (recv, mname): (Option<Expression>, &str) = match &func.kind {
            ExprKind::Ident(h) if h.path.len() >= 2 => {
                let mut r = h.clone();
                r.path.pop();
                r.cached_signal_id = std::cell::Cell::new(None);
                (
                    Some(Expression::new(ExprKind::Ident(r), func.span)),
                    h.path[h.path.len() - 1].name.name.as_str(),
                )
            }
            ExprKind::MemberAccess { expr, member } => {
                (Some((**expr).clone()), member.name.as_str())
            }
            _ => (None, ""),
        };
        if let Some(recv) = recv {
            let class = match &recv.kind {
                ExprKind::Ident(rh)
                    if rh.path.len() == 1
                        && self.module.classes.contains_key(&rh.path[0].name.name) =>
                {
                    Some(rh.path[0].name.name.clone())
                }
                _ => self.get_expr_type_name(&recv),
            };
            let mut cur = class;
            while let Some(cn) = cur {
                let Some(cd) = self.module.classes.get(&cn) else {
                    break;
                };
                if let Some(m) = cd.methods.get(mname) {
                    return matches!(&m.kind, crate::ast::decl::ClassMethodKind::Function(f)
                        if self.fn_ret_coll_dims(&f.return_type).is_some());
                }
                cur = cd.extends.clone();
            }
            return false;
        }
        let ExprKind::Ident(h) = &func.kind else {
            return false;
        };
        let Some(seg) = h.path.last() else {
            return false;
        };
        self.module
            .functions
            .get(&seg.name.name)
            .is_some_and(|fd| self.fn_ret_coll_dims(&fd.return_type).is_some())
    }

    pub(super) fn fn_returns_collection(&self, dt: &DataType) -> bool {
        self.fn_ret_coll_dims(dt).is_some_and(|dims| {
            !matches!(
                dims.first(),
                Some(crate::ast::types::UnpackedDimension::Associative { .. })
            )
        })
    }

    /// Does some `name.<rest>` key exist in `packed_signal_elem_widths`?
    /// Backed by the lazily-built prefix index — see `elem_dotted_bases`.
    pub(super) fn elem_base_has_dotted(&self, name: &str) -> bool {
        let mut cell = self.elem_dotted_bases.borrow_mut();
        let set = cell.get_or_insert_with(|| {
            let mut s: HashSet<String> = HashSet::default();
            for key in self.module.packed_signal_elem_widths.keys() {
                let bytes = key.as_bytes();
                for (i, b) in bytes.iter().enumerate() {
                    if *b == b'.' {
                        s.insert(key[..i].to_string());
                    }
                }
            }
            s
        });
        set.contains(name)
    }

    /// Keep the prefix index in step with a key being INSERTED into
    /// `packed_signal_elem_widths`. No-op until the index is first built —
    /// the build scan will see the key.
    pub(super) fn note_elem_width_key(&self, key: &str) {
        if !key.contains('.') {
            return;
        }
        if let Some(set) = self.elem_dotted_bases.borrow_mut().as_mut() {
            let bytes = key.as_bytes();
            for (i, b) in bytes.iter().enumerate() {
                if *b == b'.' {
                    set.insert(key[..i].to_string());
                }
            }
        }
    }

    pub(super) fn snapshot_formal_metadata(&self, name: &str) -> FormalMetadataSnapshot {
        FormalMetadataSnapshot {
            name: name.to_string(),
            parts: self.snapshot_formal_parts(name),
        }
    }

    /// Formal-metadata write barrier: called before any write to the six
    /// tables `snapshot_formal_parts` reads, with the written key. Every open
    /// class-method save (see `MetaFrame`) that names the key's base and has
    /// no snapshot yet takes it now.
    #[inline]
    pub(super) fn meta_note(&mut self, key: &str) {
        if self.meta_pending != 0 {
            // The innermost save's filter covers every open save's names.
            let bits = meta_key_bits(key);
            if self
                .meta_frames
                .last()
                .is_some_and(|f| f.cum_bloom & bits == bits)
            {
                self.meta_note_slow(key, bits);
            }
        }
    }

    #[inline(never)]
    pub(super) fn meta_note_slow(&mut self, key: &str, bits: u64) {
        // `packed_signal_elem_widths` also keys `name.member` under `name`.
        let base = key.split('.').next().unwrap_or(key);
        for i in (0..self.meta_frames.len()).rev() {
            let frame = &self.meta_frames[i];
            if frame.full || frame.unsaved_bloom & bits != bits {
                continue;
            }
            let plan = frame.plan.clone();
            let saved = frame.saved;
            let n = plan.meta_ks.len();
            if n > 64 {
                // More than 64 names: the first write saves all of them.
                if plan.meta_ks.iter().any(|&k| plan.meta_name(k) == base) {
                    for &k in &plan.meta_ks {
                        let snap = self.snapshot_formal_parts(plan.meta_name(k));
                        self.meta_frames[i].parts.push((k, snap));
                    }
                    self.meta_frames[i].full = true;
                    self.meta_pending -= 1;
                }
            } else {
                for pos in 0..n {
                    let bit = 1u64 << pos;
                    if plan.meta_bits[pos] != bits || saved & bit != 0 {
                        continue;
                    }
                    let k = plan.meta_ks[pos];
                    if plan.meta_name(k) != base {
                        continue;
                    }
                    let snap = self.snapshot_formal_parts(base);
                    let frame = &mut self.meta_frames[i];
                    frame.parts.push((k, snap));
                    frame.saved |= bit;
                    frame.unsaved_bloom = (0..n)
                        .filter(|p| frame.saved & (1u64 << p) == 0)
                        .fold(0, |acc, p| acc | plan.meta_bits[p]);
                    if frame.saved.count_ones() as usize == n {
                        frame.full = true;
                        self.meta_pending -= 1;
                    }
                    break;
                }
            }
            if self.meta_pending == 0 {
                break;
            }
        }
    }

    /// `snapshot_formal_metadata` without the owned name, for a caller that
    /// keeps the name itself.
    pub(super) fn snapshot_formal_parts(&self, name: &str) -> FormalMetaParts {
        // Fast path: no dotted `name.<member>` key exists, so the only
        // possible entry is the exact key — one lookup instead of a scan of
        // the whole design-sized map.
        let packed_element_widths = if self.elem_base_has_dotted(name) {
            self.module
                .packed_signal_elem_widths
                .iter()
                .filter(|(key, _)| {
                    key.as_str() == name
                        || key
                            .strip_prefix(name)
                            .is_some_and(|rest| rest.starts_with('.'))
                })
                .map(|(key, width)| (key.clone(), *width))
                .collect()
        } else {
            self.module
                .packed_signal_elem_widths
                .get_key_value(name)
                .map(|(key, width)| vec![(key.clone(), *width)])
                .unwrap_or_default()
        };
        FormalMetaParts {
            declared_type: self.module.var_decl_types.get(name).cloned(),
            packed_fields: self.module.packed_struct_fields.get(name).cloned(),
            packed_element_widths,
            packed_dimensions: self.module.packed_full_dims.get(name).cloned(),
            class_type: self.var_class_types.get(name).cloned(),
            typedef_type: self.var_typedef_types.get(name).cloned(),
        }
    }

    pub(super) fn clear_formal_metadata(&mut self, name: &str) {
        self.meta_note(name);
        self.module.var_decl_types.remove(name);
        self.module.packed_struct_fields.remove(name);
        self.clear_formal_elem_widths(name);
        self.module.packed_full_dims.remove(name);
        self.var_class_types.remove(name);
        self.var_typedef_types.remove(name);
    }

    /// The `packed_signal_elem_widths` part of `clear_formal_metadata`.
    pub(super) fn clear_formal_elem_widths(&mut self, name: &str) {
        self.meta_note(name);
        if self.elem_base_has_dotted(name) {
            self.module.packed_signal_elem_widths.retain(|key, _| {
                key.as_str() != name
                    && !key
                        .strip_prefix(name)
                        .is_some_and(|rest| rest.starts_with('.'))
            });
            // Every `name.…` key is gone; later calls for this base take the
            // fast path again. (Deeper stale prefixes like `name.sub` may
            // linger — over-approximation, costs a scan, never correctness.)
            if let Some(set) = self.elem_dotted_bases.borrow_mut().as_mut() {
                set.remove(name);
            }
        } else {
            self.module.packed_signal_elem_widths.remove(name);
        }
    }

    /// Start logging the declarations of the activation whose frame was just
    /// pushed (see `decl_shadow_log`).
    pub(super) fn open_decl_shadow_frame(&mut self) {
        if let Some(&frame_gen) = self.local_gen_stack.last() {
            self.decl_shadow_log.push(DeclShadowFrame {
                frame_gen,
                epoch: self.nested_run_epoch,
                saves: Vec::new(),
                fresh_strings: Vec::new(),
            });
        }
    }

    /// Record what a declaration of `name` (of type `dt`) in the logging
    /// activation is about to overwrite — once per name, and only when there
    /// is something to put back: a fresh name has no metadata, and one whose
    /// recorded type is trivially the same gets identical metadata again.
    pub(super) fn note_decl_shadow(&mut self, name: &str, dt: &DataType) {
        let Some(top) = self.decl_shadow_log.last() else {
            return;
        };
        if self.local_gen_stack.last() != Some(&top.frame_gen) {
            return;
        }
        match self.module.var_decl_types.get(name) {
            Some(old) if Self::dt_trivially_equal(old, dt) => return,
            None if !self.widths.contains_key(name) => {
                // §6.21: a fresh `string` local still adds its name to the
                // global `string_signals` set, where a later task's same-named
                // enum formal read as text (`256 >= 16` compared false). Take
                // just that flag back when the activation ends.
                if Self::is_string_data_type(dt)
                    && !self.string_signals.contains(name)
                    && let Some(top) = self.decl_shadow_log.last_mut()
                    && !top.fresh_strings.iter().any(|n| n == name)
                {
                    top.fresh_strings.push(name.to_string());
                }
                return;
            }
            _ => {}
        }
        if top.saves.iter().any(|d| d.meta.name == name) {
            return;
        }
        // No liveness filter: the displaced entries may belong to a task
        // SUSPENDED in another process (its frames are swapped out, so
        // neither `local_stack` nor the signal maps show them). A function
        // run meanwhile — a predictor's `uvm_reg_bus_op rw` while a
        // register task holds `uvm_reg_item rw` — would otherwise leave its
        // struct layout behind, and the task's `rw.status = ...` then wrote
        // a bit slice into the handle. Putting back a finished call's stale
        // entries is harmless: it is the state from before this call.
        let save = DeclShadow {
            width: self.widths.get(name).copied(),
            signed: self.signed_signals.contains(name),
            real: self.real_signals.contains(name),
            string: self.string_signals.contains(name),
            meta: self.take_formal_metadata(name),
        };
        if let Some(top) = self.decl_shadow_log.last_mut() {
            top.saves.push(save);
        }
    }

    /// Two declared types that are the same built-in or plain named type
    /// (no packed dimensions or type arguments). Anything else compares
    /// unequal.
    pub(super) fn dt_trivially_equal(a: &DataType, b: &DataType) -> bool {
        match (a, b) {
            (DataType::Simple { kind: x, .. }, DataType::Simple { kind: y, .. }) => x == y,
            (DataType::Real { kind: x, .. }, DataType::Real { kind: y, .. }) => x == y,
            (
                DataType::IntegerAtom {
                    kind: k1,
                    signing: s1,
                    ..
                },
                DataType::IntegerAtom {
                    kind: k2,
                    signing: s2,
                    ..
                },
            ) => k1 == k2 && s1 == s2,
            (
                DataType::IntegerVector {
                    kind: k1,
                    signing: s1,
                    dimensions: d1,
                    ..
                },
                DataType::IntegerVector {
                    kind: k2,
                    signing: s2,
                    dimensions: d2,
                    ..
                },
            ) => k1 == k2 && s1 == s2 && d1.is_empty() && d2.is_empty(),
            (
                DataType::TypeReference {
                    name: n1,
                    dimensions: d1,
                    type_args: t1,
                    ..
                },
                DataType::TypeReference {
                    name: n2,
                    dimensions: d2,
                    type_args: t2,
                    ..
                },
            ) => {
                n1.name.name == n2.name.name
                    && n1.qualified() == n2.qualified()
                    && d1.is_empty()
                    && d2.is_empty()
                    && t1.is_empty()
                    && t2.is_empty()
            }
            _ => false,
        }
    }

    /// `snapshot_formal_metadata` that MOVES the entries out instead of
    /// cloning them: the declaration about to run re-registers its own.
    pub(super) fn take_formal_metadata(&mut self, name: &str) -> FormalMetadataSnapshot {
        self.meta_note(name);
        let packed_element_widths = if self.elem_base_has_dotted(name) {
            let snap = self.snapshot_formal_parts(name).packed_element_widths;
            for (k, _) in &snap {
                self.module.packed_signal_elem_widths.remove(k);
            }
            snap
        } else {
            self.module
                .packed_signal_elem_widths
                .remove_entry(name)
                .into_iter()
                .collect()
        };
        FormalMetadataSnapshot {
            name: name.to_string(),
            parts: FormalMetaParts {
                declared_type: self.module.var_decl_types.remove(name),
                packed_fields: self.module.packed_struct_fields.remove(name),
                packed_element_widths,
                packed_dimensions: self.module.packed_full_dims.remove(name),
                class_type: self.var_class_types.remove(name),
                typedef_type: self.var_typedef_types.remove(name),
            },
        }
    }

    /// End the logging activation (its frame still on top): put back what
    /// its declarations displaced, unless other processes ran meanwhile.
    pub(super) fn close_decl_shadow_frame(&mut self) {
        let Some(&frame_gen) = self.local_gen_stack.last() else {
            return;
        };
        let Some(pos) = self
            .decl_shadow_log
            .iter()
            .rposition(|f| f.frame_gen == frame_gen)
        else {
            return;
        };
        // Anything above belongs to an activation that never closed (an
        // unwound body); it can no longer be restored safely.
        self.decl_shadow_log.truncate(pos + 1);
        let Some(frame) = self.decl_shadow_log.pop() else {
            return;
        };
        if frame.epoch != self.nested_run_epoch {
            return;
        }
        for n in &frame.fresh_strings {
            self.string_signals.remove(n);
        }
        self.restore_decl_shadows(frame.saves);
    }

    pub(super) fn restore_decl_shadows(&mut self, saves: Vec<DeclShadow>) {
        for d in saves.into_iter().rev() {
            let name = d.meta.name.clone();
            match d.width {
                Some(w) => {
                    self.widths.insert(name.clone(), w);
                }
                None => {
                    self.widths.remove(&name);
                }
            }
            if d.signed {
                self.signed_signals.insert(name.clone());
            } else {
                self.signed_signals.remove(&name);
            }
            if d.real {
                self.real_signals.insert(name.clone());
            } else {
                self.real_signals.remove(&name);
            }
            if d.string {
                self.string_signals.insert(name);
            } else {
                self.string_signals.remove(&name);
            }
            self.restore_formal_metadata(d.meta);
        }
    }

    pub(super) fn restore_formal_metadata(&mut self, saved: FormalMetadataSnapshot) {
        self.restore_formal_parts(&saved.name, saved.parts);
    }

    /// `restore_formal_metadata` for a snapshot taken by
    /// `snapshot_formal_parts`.
    pub(super) fn restore_formal_parts(&mut self, name: &str, saved: FormalMetaParts) {
        self.meta_note(name);
        // Each table ends up holding exactly the snapshot's entry for the
        // name: overwritten in place or removed, without the clear pass.
        fn put<V>(map: &mut HashMap<String, V>, name: &str, v: Option<V>) {
            match v {
                Some(v) => match map.get_mut(name) {
                    Some(slot) => *slot = v,
                    None => {
                        map.insert(name.to_string(), v);
                    }
                },
                None => {
                    map.remove(name);
                }
            }
        }
        put(&mut self.module.var_decl_types, name, saved.declared_type);
        put(
            &mut self.module.packed_struct_fields,
            name,
            saved.packed_fields,
        );
        let mut elem_widths = saved.packed_element_widths;
        if elem_widths.iter().all(|(key, _)| key == name) && !self.elem_base_has_dotted(name) {
            // Only the exact key can be involved: overwrite or remove it.
            match elem_widths.pop() {
                Some((key, width)) => match self.module.packed_signal_elem_widths.get_mut(name) {
                    Some(slot) => *slot = width,
                    None => {
                        self.module.packed_signal_elem_widths.insert(key, width);
                    }
                },
                None => {
                    self.module.packed_signal_elem_widths.remove(name);
                }
            }
        } else {
            self.clear_formal_elem_widths(name);
            for (key, width) in elem_widths {
                self.note_elem_width_key(&key);
                self.module.packed_signal_elem_widths.insert(key, width);
            }
        }
        put(
            &mut self.module.packed_full_dims,
            name,
            saved.packed_dimensions,
        );
        put(&mut self.var_class_types, name, saved.class_type);
        put(&mut self.var_typedef_types, name, saved.typedef_type);
    }

    pub(super) fn register_formal_type_metadata(
        &mut self,
        name: &str,
        dt: &DataType,
        no_unpacked_dims: bool,
    ) {
        // `clear_formal_metadata` notes `name` before any write.
        self.clear_formal_metadata(name);
        self.module
            .var_decl_types
            .insert(name.to_string(), dt.clone());
        if let DataType::TypeReference {
            name: type_name,
            type_args,
            ..
        } = dt
        {
            let type_name = type_name.name.name.clone();
            // Class check first — a class name can also key the typedef
            // WIDTH table, which misfiled class-typed formals into the
            // typedef maps (`class_of_var` missed them).
            if self.module.classes.contains_key(&type_name) {
                self.var_class_types.insert(name.to_string(), type_name);
                self.note_formal_type_args(name, type_args);
            } else if self.module.enum_members.contains_key(&type_name)
                || self.module.typedefs.contains_key(&type_name)
            {
                self.var_typedef_types.insert(name.to_string(), type_name);
            }
        }
        if !no_unpacked_dims {
            // §7.4.1 / §13.5: unpacked formals still have a packed element
            // type. Keep that geometry below their unpacked indices.
            if let Some(dims) = super::elaborate::packed_full_dims_chained(
                dt,
                &self.module.parameters,
                &self.module.typedef_types,
            )
            .filter(|dims| dims.len() > 1)
            {
                self.module.packed_full_dims.insert(name.to_string(), dims);
                if let Some(width) = self.ast_decl_elem_width(dt) {
                    self.module
                        .packed_signal_elem_widths
                        .insert(name.to_string(), width);
                }
            }
            return;
        }
        // A built-in scalar or a plain class has no element width, struct
        // layout or typedef-carried shape: only a vector's own
        // non-normalized range can apply.
        if Self::type_is_plain_scalar(dt) || self.plain_class_ref(dt).is_some() {
            if let Some(fdims) = super::elaborate::packed_full_dims_of(dt, &self.module.parameters)
            {
                self.module.packed_full_dims.insert(name.to_string(), fdims);
            }
            return;
        }
        let dt1 = self.typedef_one_step(dt);
        if let Some(ew) = super::elaborate::packed_inner_elem_width(
            dt,
            &self.module.parameters,
            &self.module.typedefs,
        )
        .or_else(|| {
            super::elaborate::packed_inner_elem_width(
                &dt1,
                &self.module.parameters,
                &self.module.typedefs,
            )
        }) {
            self.module
                .packed_signal_elem_widths
                .insert(name.to_string(), ew);
        }
        if let Some(fdims) = super::elaborate::packed_full_dims_of(dt, &self.module.parameters)
            .or_else(|| super::elaborate::packed_full_dims_of(&dt1, &self.module.parameters))
            .or_else(|| {
                super::elaborate::packed_full_dims_chained(
                    dt,
                    &self.module.parameters,
                    &self.module.typedef_types,
                )
            })
            .or_else(|| self.struct_array_full_dims(dt))
        {
            self.module.packed_full_dims.insert(name.to_string(), fdims);
        }
        if let DataType::Struct(su) = self.resolve_dt(dt) {
            for m in &su.members {
                if let Some(ew) = super::elaborate::packed_inner_elem_width(
                    &m.data_type,
                    &self.module.parameters,
                    &self.module.typedefs,
                ) {
                    for mdecl in &m.declarators {
                        let key = format!("{}.{}", name, mdecl.name.name);
                        self.note_elem_width_key(&key);
                        self.module.packed_signal_elem_widths.insert(key, ew);
                    }
                }
            }
        }
        if let Some(fields) = super::elaborate::packed_struct_field_layout(
            dt,
            &self.module.parameters,
            &self.module.typedefs,
            &self.module.typedef_types,
        ) {
            if !fields.is_empty() {
                self.module
                    .packed_struct_fields
                    .insert(name.to_string(), fields);
            }
        }
    }

    fn apply_default_formal_outputs(&mut self, bindings: &[(String, Expression)]) {
        let default_outputs: Vec<_> = bindings
            .iter()
            .map(|(name, actual)| {
                let value = self
                    .local_stack
                    .last()
                    .and_then(|locals| locals.get(name))
                    .cloned();
                let binding = self
                    .local_iface_aliases
                    .last()
                    .and_then(|frame| frame.get(name))
                    .cloned();
                (actual.clone(), value, binding)
            })
            .collect();
        for (actual, value, binding) in default_outputs {
            if let Some(value) = value {
                self.assign_value(&actual, &value);
            }
            if let Some(binding) = binding {
                if binding.is_empty() {
                    let null = Expression::new(ExprKind::Null, actual.span);
                    self.bind_static_vif(&actual, &null);
                    self.try_bind_virtual_iface(&actual, &null);
                    self.assign_value(&actual, &Value::zero(32));
                } else {
                    self.vif_bind_actual(&actual, &binding);
                }
            }
        }
    }

    /// §13.5.3: evaluate defaults in the declaration's receiver/specialization;
    /// supplied actuals continue to evaluate in the caller's context.
    fn in_subroutine_default_scope<R>(
        &mut self,
        receiver: Option<&(Option<usize>, String)>,
        spec: Option<(String, String)>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.this_stack
            .push(receiver.and_then(|(handle, _)| *handle));
        self.class_context_stack
            .push(receiver.map(|(_, class)| class.clone()));
        let saved = std::mem::replace(&mut self.current_spec, spec);
        let result = f(self);
        self.current_spec = saved;
        self.class_context_stack.pop();
        self.this_stack.pop();
        result
    }

    /// A method task supplies its specialization, receiver and declaring
    /// class so defaults and formal types resolve in the declaration scope.
    pub(super) fn bind_task_frame(
        &mut self,
        td: &TaskDeclaration,
        args: &[Expression],
        method_ctx: Option<(Option<(String, String)>, Option<usize>, String)>,
    ) -> TaskCleanup {
        use crate::ast::types::PortDirection;
        let type_ctx = method_ctx.as_ref().map(|(spec, _, _)| spec.clone());
        let default_spec = type_ctx.clone().flatten();
        let default_receiver = method_ctx
            .as_ref()
            .map(|(_, handle, class)| (*handle, class.clone()));
        let normalized = Self::normalize_call_args(&td.ports, args);
        let supplied = normalized
            .as_ref()
            .map(|_| Self::supplied_argument_mask(&td.ports, args));
        let args: &[Expression] = normalized.as_deref().unwrap_or(args);
        let arg_supplied = |i: usize| {
            i < args.len()
                && supplied
                    .as_ref()
                    .is_none_or(|mask| mask.get(i).copied().unwrap_or(false))
        };
        // §8.25: a formal typed by one of the class's type parameters has
        // the type bound in the CALLEE's specialization. Resolve each under
        // it now — the arguments below evaluate in the caller's context,
        // where the parameter means something else or nothing.
        let has_type_context = type_ctx.is_some();
        let saved_type_context =
            type_ctx.map(|spec| std::mem::replace(&mut self.current_spec, spec));
        let bound_types: Vec<Option<String>> = if has_type_context {
            td.ports
                .iter()
                .map(|port| self.formal_type_param_binding(&port.data_type))
                .collect()
        } else {
            vec![None; td.ports.len()]
        };
        let vif_formal_indices: Vec<usize> = td
            .ports
            .iter()
            .enumerate()
            .filter_map(|(i, port)| {
                let is_vif = if let Some(bound) = bound_types[i].as_deref() {
                    // §8.25: the callee's concrete type takes precedence over
                    // any same-named typedef in the surrounding context.
                    let explicit = bound.trim_start().starts_with("virtual");
                    let base = bound
                        .trim_start_matches("virtual")
                        .trim()
                        .split(['#', '.'])
                        .next()
                        .unwrap_or("")
                        .trim();
                    (self.module.interfaces.contains(base)
                        && (explicit
                            || !(self.module.classes.contains_key(base)
                                || self.module.covergroups.contains_key(base))))
                        || self
                            .module
                            .typedef_types
                            .get(bound)
                            .is_some_and(|dt| self.is_virtual_iface_type(dt))
                } else {
                    self.is_virtual_iface_type(&port.data_type)
                };
                is_vif.then_some(i)
            })
            .collect();
        // Resolve output handle types in the callee's specialization before
        // restoring the caller's context. Automatic formals start null.
        let output_class_indices: Vec<usize> = td
            .ports
            .iter()
            .enumerate()
            .filter_map(|(i, port)| {
                (port.direction == PortDirection::Output
                    && (method_ctx.is_some()
                        || matches!(td.lifetime, Some(crate::ast::types::Lifetime::Automatic)))
                    && self.port_class_ref(port).is_some())
                .then_some(i)
            })
            .collect();
        if let Some(saved) = saved_type_context {
            self.current_spec = saved;
        }
        // Dedup by borrowed name: the formal list is tiny, so a linear
        // scan beats a HashSet plus one String allocation per formal on
        // EVERY call.
        let mut metadata_names: Vec<&str> = Vec::new();
        let formal_metadata: Vec<FormalMetadataSnapshot> = td
            .ports
            .iter()
            .map(|port| port.name.name.as_str())
            .filter(|name| {
                if metadata_names.contains(name) {
                    false
                } else {
                    metadata_names.push(name);
                    true
                }
            })
            .map(|name| self.snapshot_formal_metadata(name))
            .collect();
        // Evaluate input args and collect output/ref arg expressions
        let mut locals = self.take_pooled_frame();
        // The callee frame's type overlay, pushed together with `locals`.
        let mut frame_types = self.take_pooled_types();
        let mut output_bindings: Vec<(String, Expression)> = Vec::new();
        let mut default_output_bindings = Vec::new();
        let mut bound_vifs: Vec<_> = td
            .ports
            .iter()
            .enumerate()
            .filter_map(|(i, port)| {
                if vif_formal_indices.contains(&i) && port.direction != PortDirection::Output {
                    // §13.5.3: an omitted argument binds the formal's DEFAULT
                    // (§10.9.1). A virtual-interface formal's default names the
                    // interface instance the body must dispatch to; resolving
                    // it alongside a supplied actual keeps the formal bound, so
                    // `show()` reads the default's interface and an `inout`/
                    // `output` default copies that binding back to the actual.
                    args.get(i)
                        .filter(|_| arg_supplied(i))
                        .and_then(|a| self.resolve_vif_rhs_name_strict(a))
                        .map(|bound| (i, bound))
                } else {
                    None
                }
            })
            .collect();
        let saved_vif_formals: Vec<_> = td
            .ports
            .iter()
            .enumerate()
            .filter(|(i, port)| {
                vif_formal_indices.contains(i) || self.vif_local_names.contains(&port.name.name)
            })
            .map(|(_, port)| {
                let key = format!("__vif_local__{}", port.name.name);
                let prior = self.signals.get(&key).cloned();
                (key, prior)
            })
            .collect();
        // §25.9 via the __vif_local__ convention: a virtual-interface ACTUAL
        // records its interface under the formal's name (cleared otherwise so
        // a stale same-named formal from an earlier call can't leak).
        for (i, port) in td.ports.iter().enumerate() {
            if !Self::formal_may_carry_vif(&port.data_type) {
                continue;
            }
            // A plain class formal cannot be bound to an interface.
            let nm = bound_vifs
                .iter()
                .find(|(index, _)| *index == i)
                .map(|(_, bound)| bound.clone());
            match nm {
                Some(nm) => {
                    self.vif_local_names.insert(port.name.name.clone());
                    self.signals.insert(
                        format!("__vif_local__{}", port.name.name),
                        Value::from_string(&nm),
                    );
                }
                // No key was ever made under this name, so the removal
                // would find nothing (the store generation still moves).
                None if !self.vif_local_names.contains(&port.name.name) => bump_store_gen(),
                None => {
                    self.signals
                        .remove(&format!("__vif_local__{}", port.name.name));
                }
            }
        }
        let mut assoc_params: Vec<(String, String, bool)> = Vec::new(); // (param_name, caller_array_name, is_out)
        let mut array_params: Vec<String> = Vec::new(); // param names with unpacked Range dim
        let mut identity_formals: Vec<String> = Vec::new(); // ref aggregates bound by identity
        self.push_queue_frame();
        let formal_dyn = self.begin_formal_dyn();
        let mut array_writebacks: Vec<(String, String, i64, i64)> = Vec::new();
        let mut queue_writebacks: Vec<(String, String)> = Vec::new();
        for (i, port) in td.ports.iter().enumerate() {
            // A type-parameter formal's bound type: a typedef (an unpacked
            // struct binds member-wise below) or a class.
            let bound = bound_types[i].as_deref();
            let data_type = match bound.and_then(|b| self.module.typedef_types.get(b)) {
                Some(dt) => std::borrow::Cow::Owned(dt.clone()),
                None => std::borrow::Cow::Borrowed(&port.data_type),
            };
            // §13.3: a formal is a variable of its declared type — give it the
            // same structural metadata a local declaration gets, or member and
            // element selects on it silently read 0.
            self.register_formal_type_metadata(
                &port.name.name,
                &data_type,
                port.dimensions.is_empty(),
            );
            if let Some(b) = bound {
                let base = b.split('#').next().unwrap_or(b);
                if self.module.classes.contains_key(base) {
                    frame_types.0.insert(port.name.name.clone(), b.to_string());
                    self.var_class_types
                        .insert(port.name.name.clone(), b.to_string());
                } else if self.module.enum_members.contains_key(b)
                    || self.module.typedefs.contains_key(b)
                {
                    frame_types.1.insert(port.name.name.clone(), b.to_string());
                }
            } else if let DataType::TypeReference { name: tn, .. } = &port.data_type {
                let type_name = &tn.name.name;
                if self.module.classes.contains_key(type_name) {
                    frame_types
                        .0
                        .insert(port.name.name.clone(), type_name.clone());
                } else if self.module.enum_members.contains_key(type_name)
                    || self.module.typedefs.contains_key(type_name)
                {
                    frame_types
                        .1
                        .insert(port.name.name.clone(), type_name.clone());
                }
            }
            // §13.5.3: an omitted argument binds the formal's DEFAULT through
            // the aggregate binders below, like a supplied actual (see
            // `exec_function_call_body`).
            let actual = args
                .get(i)
                .filter(|_| arg_supplied(i))
                .or_else(|| Self::default_actual(port));
            // §13.5, §6.18: an associative formal declared through a typedef
            // (`output aa_t a`) is one too — as for functions. Checked first: the
            // struct and queue binders below would claim a typedef of struct
            // elements.
            let is_assoc = self.port_is_assoc_array(port);
            if let Some(actual) = actual.filter(|_| is_assoc) {
                let assoc_is_out = matches!(
                    port.direction,
                    PortDirection::Output | PortDirection::Inout | PortDirection::Ref
                );
                if let Some((param_name, caller_name, _)) = self.bind_assoc_param(port, actual) {
                    assoc_params.push((param_name, caller_name, assoc_is_out));
                }
                continue;
            }
            // Unpacked array formal (`int a[2:0]` or `int a[3]`): copy the
            // caller's elements in, and copy them back for an out/inout/ref.
            if let Some(actual) = actual {
                if let Some(info) = self.bind_array_arg(port, actual) {
                    // Keep fixed-array formals visible to class-scope lookup (§13.5).
                    locals.insert(
                        format!("\u{1}array_formal:{}", port.name.name),
                        Value::zero(1),
                    );
                    if matches!(
                        port.direction,
                        PortDirection::Output | PortDirection::Inout | PortDirection::Ref
                    ) && !info.1.is_empty()
                    {
                        array_writebacks.push(info.clone());
                    }
                    array_params.push(info.0);
                    continue;
                }
            }
            // §7.10 / §13.5.2: a QUEUE or DYNAMIC-ARRAY formal. Only functions
            // bound one; a task bound nothing, so the body operated on storage
            // named after the FORMAL and an output/inout/ref never reached the
            // caller's collection — `task push(ref int q[$]); q.push_back(v);`
            // left the caller's queue empty. Worse, that formal-named storage
            // persisted across calls, so the next call saw the previous one's
            // elements.
            if let Some(actual) = actual {
                let eff_dims: Vec<crate::ast::types::UnpackedDimension> =
                    if port.dimensions.is_empty() {
                        if let (false, crate::ast::types::DataType::TypeReference { name, .. }) =
                            (self.port_class_ref(port).is_some(), &port.data_type)
                        {
                            let tn = &name.name.name;
                            let concrete = bound
                                .map(str::to_string)
                                .or_else(|| self.resolve_type_param_binding(tn))
                                .unwrap_or_else(|| tn.clone());
                            self.module
                                .typedef_unpacked_dims
                                .get(&concrete)
                                .cloned()
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
                    if matches!(
                        port.direction,
                        PortDirection::Output | PortDirection::Inout | PortDirection::Ref
                    ) && !caller.is_empty()
                    {
                        queue_writebacks.push((store, caller));
                    }
                    continue;
                }
            }
            // §13.5.2: an UNPACKED-struct formal. Tasks bound nothing at all —
            // the member leaves live in the frame under `<formal>.<member>`
            // (functions already bind them that way), so the body read x from
            // every member of an input, and an output/inout/ref struct formal
            // copied nothing back to the caller's variable.
            if let Some(actual) = actual.filter(|_| port.dimensions.is_empty()) {
                if let Some(backs) = self.bind_unpacked_struct_formal(
                    &port.name.name,
                    &data_type,
                    port.direction,
                    actual,
                    &mut locals,
                ) {
                    output_bindings.extend(backs);
                    queue_writebacks.append(&mut self.pending_struct_coll_wb);
                    continue;
                }
            }
            match port.direction {
                PortDirection::Output | PortDirection::Inout => {
                    // §13.5.3 (1800-2023): an omitted argument with a default
                    // binds the default as its actual, so `output o = a`
                    // copies `o` back to `a` on return.
                    let val = if port.direction == PortDirection::Output
                        && (vif_formal_indices.contains(&i) || output_class_indices.contains(&i))
                    {
                        Value::zero(32)
                    } else {
                        if arg_supplied(i) {
                            let value = self.eval_expr(&args[i]);
                            self.vif_argument_value(port, &args[i], value)
                        } else if let Some(default) = &port.default {
                            self.in_subroutine_default_scope(
                                default_receiver.as_ref(),
                                default_spec.clone(),
                                |sim| {
                                    sim.push_local_frame_typed(locals.clone(), frame_types.clone());
                                    let value = sim.eval_expr(default);
                                    let value = sim.vif_argument_value(port, default, value);
                                    sim.pop_local_frame();
                                    value
                                },
                            )
                        } else {
                            Value::zero(32)
                        }
                    };
                    let val = self.fit_scalar_formal_seed(port, val);
                    locals.insert(port.name.name.clone(), val);
                    if let Some(a) = actual.filter(|a| arg_supplied(i) || Self::is_lvalue_expr(a)) {
                        if arg_supplied(i) {
                            output_bindings.push((port.name.name.clone(), a.clone()));
                        } else {
                            default_output_bindings.push((port.name.name.clone(), a.clone()));
                        }
                    }
                }
                PortDirection::Ref => {
                    // §15.5: a `ref event` formal is bound by IDENTITY, not by
                    // value — `-> x` fires the caller's sync object (resolved
                    // through `ref_binding_stack`). Copying a value back on
                    // return would rewrite the event's 1-bit toggle signal with
                    // the stale value captured at call time, cancelling the
                    // edge before the main loop delivered it: the trigger
                    // "happened" and the caller's `@e` waiter still hung.
                    if matches!(
                        &port.data_type,
                        crate::ast::types::DataType::Simple {
                            kind: crate::ast::types::SimpleType::Event,
                            ..
                        }
                    ) {
                        continue;
                    }
                    static REF_DBG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
                    if *REF_DBG.get_or_init(|| std::env::var_os("XEZIM_REF_DBG").is_some()) {
                        eprintln!(
                            "[REFDBG] task-ref formal={} dims={}",
                            port.name.name,
                            port.dimensions.len()
                        );
                    }
                    // §13.5.2: a ref formal with UNPACKED dimensions
                    // (`ref int arr[3]`) binds an AGGREGATE by identity —
                    // element accesses route through the ref binding, and a
                    // scalar return copy-back clobbered the caller's whole
                    // array with the placeholder x (audit46 a10: the
                    // reference keeps '{11,12,13}).
                    if !port.dimensions.is_empty() {
                        identity_formals.push(port.name.name.clone());
                        continue;
                    }
                    let val = if arg_supplied(i) {
                        let value = self.eval_expr(&args[i]);
                        self.vif_argument_value(port, &args[i], value)
                    } else {
                        Value::zero(32)
                    };
                    locals.insert(port.name.name.clone(), val);
                    if arg_supplied(i) {
                        output_bindings.push((port.name.name.clone(), args[i].clone()));
                    }
                }
                _ => {
                    let mut val = if arg_supplied(i) {
                        let value = self.eval_expr(&args[i]);
                        self.vif_argument_value(port, &args[i], value)
                    } else if let Some(def) = &port.default {
                        // Caller omitted this arg → apply the formal's default
                        // value (was incorrectly using 0). E.g.
                        // `start(..., int priority = -1)`: a missing arg must be -1, not 0.
                        // §13.5.3: a later default may use an earlier formal.
                        self.in_subroutine_default_scope(
                            default_receiver.as_ref(),
                            default_spec.clone(),
                            |sim| {
                                sim.push_local_frame_typed(locals.clone(), frame_types.clone());
                                let value = sim.eval_formal_default(port, def);
                                let value = sim.vif_argument_value(port, def, value);
                                sim.pop_local_frame();
                                value
                            },
                        )
                    } else {
                        Value::zero(32)
                    };
                    val = self.fit_scalar_formal_seed(port, val);
                    // Preserve runtime-bound type-parameter signing, without
                    // resolving its width through a module default.
                    if !Self::type_is_plain_scalar(&port.data_type)
                        && self.type_is_signed_concrete(&port.data_type)
                    {
                        val.is_signed = true;
                    }
                    locals.insert(port.name.name.clone(), val);
                }
            }
            if arg_supplied(i)
                && let Some((_, bound)) = bound_vifs.iter().find(|(index, _)| *index == i)
            {
                locals.insert(
                    port.name.name.clone(),
                    Value::from_u64(Self::iface_handle_token(bound), 32),
                );
            }
        }
        // §13.5.3 / §25.9: defaults have been evaluated once in the
        // declaration scope. Bind their resulting token, including null, and
        // keep the task's identity store in step with its dispatch alias.
        for (i, port) in td.ports.iter().enumerate() {
            if args.get(i).filter(|_| arg_supplied(i)).is_some() || !vif_formal_indices.contains(&i)
            {
                continue;
            }
            let bound = locals
                .get(&port.name.name)
                .and_then(|v| v.to_u64())
                .filter(|&token| token != 0)
                .and_then(|token| self.iface_by_token(token))
                .map(str::to_string);
            let key = format!("__vif_local__{}", port.name.name);
            if let Some(bound) = bound {
                self.vif_local_names.insert(port.name.name.clone());
                self.signals.insert(key, Value::from_string(&bound));
                bound_vifs.push((i, bound));
            } else {
                self.signals.remove(&key);
            }
        }
        self.commit_formal_dyn(formal_dyn);
        self.push_local_frame_typed(locals, frame_types);
        // §6.21: open a static-local sync frame keyed by this task name.
        let sync_name = self.sync_frame_name(&td.name.name.name);
        self.static_local_syncs.push((sync_name, Vec::new()));
        // LRM §25.9: virtual-interface formals. When a formal's
        // declared data type names a known interface, register an
        // alias from the formal name to the caller's actual ident.
        // The alias frame is consulted by `resolve_hier_name` so
        // `vif.member` inside the task body resolves to
        // `<bound>.member` for the duration of this call.
        let mut iface_alias_frame: HashMap<String, String> = HashMap::default();
        for (i, port) in td.ports.iter().enumerate() {
            if port.dimensions.is_empty() && vif_formal_indices.contains(&i) {
                iface_alias_frame.insert(
                    port.name.name.clone(),
                    bound_vifs
                        .iter()
                        .find(|(index, _)| *index == i)
                        .map(|(_, bound)| bound.clone())
                        .unwrap_or_default(),
                );
            }
        }
        if !iface_alias_frame.is_empty() {
            // §13.5/§25.9: task bindings travel with parked process frames;
            // function bindings use their existing flat-slot writeback path.
            iface_alias_frame.insert("__xezim_task_vif_frame__".into(), String::new());
        }
        self.local_iface_aliases.push(iface_alias_frame);
        // SV-2023: redirect NBAs targeting a `ref` formal back to the caller's
        // actual variable (IEEE 1800-2023 §13.5.2).
        let mut ref_map: HashMap<String, Expression> = HashMap::default();
        // §13.5.2 true aliasing: where the actual is module-visible storage,
        // record a redirect so reads/writes of the formal go THROUGH to it
        // during the call (see `ref_alias_stack`). The callee's local frame is
        // already pushed here, so `ref_alias_target` treats the SECOND-from-
        // top frame as the caller when rejecting caller-local actuals.
        let mut alias_map: HashMap<String, String> = HashMap::default();
        for (i, port) in td.ports.iter().enumerate() {
            if matches!(port.direction, crate::ast::types::PortDirection::Ref) && arg_supplied(i) {
                if let Some(frozen) = self.ref_alias_target(&args[i]) {
                    alias_map.insert(port.name.name.clone(), frozen);
                }
                // Chained identity binding: an actual that is itself the
                // caller's identity-bound formal resolves to the caller's
                // actual (the callee's frames are not pushed yet, so the top
                // of the stacks is the caller's). Without this a formal named
                // like its actual rewrote `a` to `a` forever.
                let mut actual = args[i].clone();
                if let ExprKind::Ident(h) = &actual.kind {
                    if h.path.len() == 1 && h.path[0].selects.is_empty() {
                        let n = h.path[0].name.name.clone();
                        if self
                            .ref_identity_stack
                            .last()
                            .is_some_and(|v| v.iter().any(|x| *x == n))
                        {
                            if let Some(up) = self
                                .ref_binding_stack
                                .last()
                                .and_then(|m| m.get(&n))
                                .cloned()
                            {
                                actual = up;
                            }
                        }
                    }
                }
                ref_map.insert(port.name.name.clone(), actual);
            }
        }
        // An aliased ref writes through during the call — a return copy-out
        // would CLOBBER writes other processes made to the actual while this
        // call was suspended (observed: an observer's write overwritten by
        // the callee's stale copy).
        output_bindings.retain(|(n, _)| !alias_map.contains_key(n));
        self.ref_binding_stack.push(ref_map);
        // A `ref` formal whose actual has the SAME name (`sum(ref int cnt)`
        // called as `sum(cnt)`) cannot be steered by the alias map: the
        // redirect would rewrite the name to itself (see
        // `ref_formal_redirect_hier`). Drop its frame copy instead, so plain
        // resolution reaches the actual's storage for reads and writes.
        if let Some(frame) = self.local_stack.last_mut() {
            for (formal, storage) in &alias_map {
                if formal == storage {
                    frame.remove(formal);
                }
            }
        }
        self.ref_alias_stack.push(alias_map);
        self.ref_identity_stack.push(identity_formals);
        self.refresh_ref_redirect_hot();
        // §13.5 / §6.16: record the `string` FORMALS of this frame so `s[i]`
        // writes a character instead of hitting the frame-local bit-slice
        // arm. Kept per process, keyed by this frame's generation, rather
        // than in the global `string_signals` set (see
        // `task_string_formals`).
        let pushed_string_formals = match self.local_gen_stack.last() {
            Some(&fgen) => match self.task_frame_strings(fgen, &td.ports) {
                Some(entry) => {
                    self.task_string_formals.push(entry);
                    true
                }
                None => false,
            },
            None => false,
        };
        self.return_value = None;
        let saved_break = self.break_flag;
        let saved_continue = self.continue_flag;
        let saved_return = self.return_flag;
        self.break_flag = false;
        self.continue_flag = false;
        self.return_flag = false;
        let prev_static = self.current_static_task.take();
        if matches!(td.lifetime, Some(crate::ast::types::Lifetime::Static)) {
            self.current_static_task = Some(td.name.name.name.clone());
        }
        // §6.21: an `automatic` task's locals are per-invocation — they need
        // the same per-call array isolation class methods get (see the
        // enumerator-lifetime in `exec_stmt_var_decl`). A default (unmarked)
        // module task is static and shares its locals, so only an explicit
        // `automatic` lifetime sets this.
        let prev_auto = self.current_auto_task.take();
        if matches!(td.lifetime, Some(crate::ast::types::Lifetime::Automatic)) {
            self.current_auto_task = Some(td.name.name.name.clone());
        }
        // §9.6.2 bookkeeping: this pid is now executing an invocation of
        // the named task, so a cross-process `disable <task>` can find it.
        let tname = td.name.name.name.clone();
        self.active_task_pids
            .entry(tname.clone())
            .or_default()
            .insert(self.current_pid);
        // §21.2.1.7 `%m`: entering a task RESETS the lexical scope to just
        // this task (leaf name — the decl name may be instance-qualified),
        // saving the caller's scope for restoration on return.
        let m_entry = self.subroutine_m_entry(&tname, td.span, self.pending_pkg_scope.as_deref());
        let saved_m_scope = std::mem::replace(&mut self.m_scope_stack, m_entry);
        TaskCleanup {
            frame_scope_hint: None,
            prev_hint: None,
            caller_body_scope: None,
            saved_m_scope,
            task_name: Some(tname),
            array_writebacks,
            queue_writebacks,
            output_bindings,
            default_output_bindings,
            saved_vif_formals,
            assoc_params,
            array_params,
            pushed_string_formals,
            saved_break,
            saved_continue,
            saved_return,
            prev_static,
            prev_auto,
            pushed_method_this: false,
            saved_spec: None,
            formal_metadata,
            modclass: None,
        }
    }

    /// §23.8: an instance (module / interface) task body resolves names in
    /// its own scope — a calling class method's `this` must not be visible,
    /// or an unqualified call binds to the caller's method instead of the
    /// interface's or its imported package's subroutine. Unwound with the
    /// method frame (`pushed_method_this`).
    pub(super) fn push_instance_task_context(&mut self, cleanup: &mut TaskCleanup) {
        if cleanup.pushed_method_this || self.this_stack.last().copied().flatten().is_none() {
            return;
        }
        cleanup.saved_spec = self.current_spec.clone();
        cleanup.pushed_method_this = true;
        self.this_stack.push(None);
        self.class_context_stack.push(None);
        self.method_local_base
            .push(self.local_stack.len().saturating_sub(1));
        self.current_spec = None;
    }

    pub(super) fn push_task_method_this(
        &mut self,
        handle_opt: Option<usize>,
        mclass: String,
        cleanup: &mut TaskCleanup,
    ) {
        cleanup.saved_spec = self.current_spec.clone();
        cleanup.pushed_method_this = true;
        // A method task body starts its own `%m` chain; the caller's comes
        // back with `saved_m_scope` on return.
        self.m_scope_stack.clear();
        self.this_stack.push(handle_opt);
        self.class_context_stack.push(Some(mclass.clone()));
        // §23 / §13.4: record the base of THIS method's own locals so
        // `get_expr_type_name`'s `in_any_frame` check only sees the current
        // inlined method's locals — not a caller's same-named local that
        // leaked into the flat `var_class_types` map (e.g. a `uvm_sequence_base
        // seq` local on the sequencer shadowing a sequence subclass's `seq`
        // member, breaking virtual dispatch of `body()` in nested
        // sequence-`start()` calls). The synchronous method path pushes this
        // right after binding the locals frame; mirror it here.
        self.method_local_base
            .push(self.local_stack.len().saturating_sub(1));
        if let Some(spec) = self.task_method_spec(handle_opt, &mclass) {
            self.current_spec = Some(spec);
        }
        // §8/§23.9: see `modclass_depth`.
        if let Some(h) = handle_opt
            && self.any_modclass()
            && let Some(inst) = self.modclass_instance(h)
        {
            self.modclass_rename_frame(h, &mclass, &inst);
            self.modclass_depth.set(self.modclass_depth.get() + 1);
            *self.activation_scope.borrow_mut() = Some(inst.clone());
            // The resolve hint too, kept across a suspend (`frame_scope_hint`).
            if cleanup.frame_scope_hint.is_none() {
                cleanup.prev_hint = self.name_resolve_hint.borrow().clone();
            }
            *self.name_resolve_hint.borrow_mut() = Some(inst.clone());
            cleanup.frame_scope_hint = Some(inst.clone());
            cleanup.modclass = Some(inst);
        }
    }

    /// The class specialization a method task's body runs under — what
    /// `push_task_method_this` makes `current_spec` — or `None` when the
    /// caller's stays in force. `bind_task_frame` resolves the formals'
    /// type parameters under it before `this` is pushed.
    pub(super) fn task_method_spec(
        &self,
        handle_opt: Option<usize>,
        mclass: &str,
    ) -> Option<(String, String)> {
        let mut spec = None;
        if let Some(h) = handle_opt {
            if let Some(inst) = self.heap.get(h).and_then(|o| o.as_ref()) {
                let cn = inst.class_name.clone();
                let bindings = inst.type_bindings.clone();
                let differs = match self.current_spec.as_ref() {
                    None => true,
                    Some((b, s)) => {
                        *b != self.class_origin(&cn)
                            || inst
                                .spec
                                .as_ref()
                                .is_some_and(|(ib, is)| ib != b || is != s)
                    }
                };
                if differs {
                    if inst.spec.is_some() {
                        spec = inst.spec.clone();
                    } else if let Some(cd) = self.module.classes.get(&cn) {
                        let mut param_names = cd.param_order.clone();
                        if param_names.is_empty() {
                            param_names = cd.type_param_names.clone();
                        }
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
                                spec = Some((cn, sig_frags.join(",")));
                            }
                        }
                    }
                }
            }
        }
        // §8.25 (generic receiver specialization). The synchronous method path
        // (`exec_method_in_class_hierarchy`) seeds `current_spec` from the
        // CONCRETE receiver's extends chain when the method is DECLARED in a
        // PARAMETERIZED ancestor. The task path had only the instance's own
        // bindings, which for a non-parameterized leaf carry nothing — so a
        // virtual TASK inherited from a parameterized base (`class d extends
        // pbase;` with an inherited `task body()` that uses `T`, e.g.
        // `T::type_id::create`) resolved `T` to the unknown-type fallback,
        // `logic`, and every create returned null: the run reported UVM_ERROR 0
        // while driving no transactions at all. Mirror the function path here.
        if let Some(s) = handle_opt.and_then(|h| self.inherited_method_spec(h, mclass)) {
            spec = Some(s);
        }
        spec
    }
}
