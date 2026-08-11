use super::*;

#[derive(Debug, Clone)]
pub(crate) struct ClassInstance {
    pub(crate) class_name: String,
    pub(crate) properties: HashMap<String, Value>,
    /// Maps a class TYPE-parameter name (e.g. `T` in
    /// `class Mk #(type T=Base)`) to the concrete class name it was
    /// specialized with (e.g. `Base`). Populated by
    /// `instantiate_class_with_type_args`. Used so an unqualified `obj = new()`
    /// unqualified `obj = new()` whose declared type is a type parameter
    /// constructs the bound concrete class (running its real `new`). Empty
    /// for classes without type parameters / non-specialized instances.
    pub(crate) type_bindings: HashMap<String, String>,
    /// The active specialization (`(base_class, sig)`) the instance was
    /// constructed under, captured so a later VIRTUAL method call on the
    /// instance can restore it (current_spec is otherwise lost once the
    /// constructing static call returns). Used by the dispatch override in
    /// `exec_method_in_class_hierarchy`. `None` for placeholder/stub
    /// instances and unspecialized constructions.
    pub(crate) spec: Option<(String, String)>,
}

impl super::Simulator {



    pub(crate) fn class_prop_width(&self, class_name: &str, prop: &str) -> Option<u32> {
        self.class_prop_width_impl(class_name, None, prop)
    }


    pub(crate) fn class_static_get(&mut self, start_class: &str, prop: &str) -> Option<Value> {
        let key = self.static_prop_key(start_class, prop)?;
        if !self.oop.class_statics.contains_key(&key) {
            // Key head is `DeclClass` or `DeclClass#spec`; strip the spec
            // suffix to find the class whose initial value seeds the cell.
            let head = key.split("::").next().unwrap_or("");
            let decl_class = head.split('#').next().unwrap_or(head);
            // Collect the initial value: first try `properties` (static
            // class members), then `param_defaults` (class localparam
            // constants, e.g. `localparam string prefix = "..."`).
            // We must extract any expression clone before eval_expr to
            // avoid borrowing self.module and self simultaneously.
            let init_val: Option<Value> = {
                let cd = self.module.classes.get(decl_class);
                if let Some(cd) = cd {
                    if let Some(sig) = cd.properties.get(prop) {
                        Some(sig.value.clone())
                    } else {
                        // Look for class localparam constants (e.g.
                        // `localparam string prefix = "+set_verbosity="`).
                        let pd_expr = cd
                            .param_defaults
                            .iter()
                            .find(|(name, _)| name == prop)
                            .and_then(|(_, e)| e.clone());
                        pd_expr.map(|init_expr| self.eval_expr(&init_expr))
                    }
                } else {
                    None
                }
            };
            let init = init_val.unwrap_or_else(|| Value::zero(32));
            self.oop.class_statics.insert(key.clone(), init);
        }
        self.oop.class_statics.get(&key).cloned()
    }


    pub(crate) fn class_static_set(&mut self, start_class: &str, prop: &str, val: Value) -> bool {
        if let Some(key) = self.static_prop_key(start_class, prop) {
            self.oop.class_statics.insert(key, val);
            true
        } else {
            false
        }
    }


    pub(crate) fn exec_method_call(&mut self, handle: usize, method_name: &str, args: &[Expression]) -> Value {
        // Ctor-time binding: a call on the object UNDER CONSTRUCTION starts
        // its search at the constructing class, not the runtime leaf.
        if method_name != "new" {
            if let Some((ch, cc)) = self.oop.ctor_class_stack.last().cloned() {
                if ch == handle && self.class_has_method(&cc, method_name) {
                    return self.exec_method_in_class_hierarchy(handle, &cc, method_name, args);
                }
            }
        }
        // IEEE 1800-2023 §9.7 process class: kill/await/suspend/resume on a
        // process handle (>= PROCESS_HANDLE_BASE). These are intercepted here,
        // before any user-class dispatch.
        if let Some(pid) = Self::proc_handle_to_pid(handle as u64) {
            match method_name {
                "status" => {
                    return Value::from_u64(self.proc_status(pid) as u64, 32);
                }
                "kill" => {
                    self.proc_kill(pid);
                    return Value::zero(32);
                }
                "await" => {
                    // Block the calling process until `pid` terminates.
                    // The continuation is the remaining statement stream —
                    // captured by the CALLER (run_process_stmts) which passes
                    // an empty args slice; the real continuation is handled
                    // at the StatementKind::MethodCall dispatch site below.
                    // For the method-call-via-exec_method_call path (used by
                    // inline task calls), we mark the suspension here.
                    //
                    // NOTE: await() needs the caller's continuation, which
                    // exec_method_call doesn't have. The actual blocking is
                    // done at the call site (see the MethodCall dispatch in
                    // run_process_stmts). If we reach here, it means await()
                    // was called in a context without continuation capture
                    // — treat as a no-op (target already terminated).
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
                _ => {} // fall through to srandom/randstate etc.
            }
        }
        // IEEE 1800-2023 §18.13/§18.14 random stability. `srandom(seed)`,
        // `get_randstate()` and `set_randstate(s)` are built-ins of BOTH the
        // §9.7 `process` class (`process::self().srandom(...)` — the token
        // handle is >= PROCESS_HANDLE_BASE) and of every class object
        // (`obj.srandom(...)`, `this.srandom(...)`). They are intercepted here,
        // ahead of user-method dispatch, unless the class actually defines an
        // override of its own.
        if matches!(method_name, "srandom" | "get_randstate" | "set_randstate")
            && !self
                .oop.heap
                .get(handle)
                .and_then(|o| o.as_ref())
                .map(|i| self.class_has_method(&i.class_name, method_name))
                .unwrap_or(false)
        {
            if let Some(v) = self.exec_rand_state_method(handle, method_name, args) {
                return v;
            }
        }
        if method_name == "randomize" {
            // §18.11: `obj.randomize(null)` is the in-line constraint CHECKER —
            // it randomizes nothing and just reports whether the object's
            // current values satisfy its active constraints.
            if Self::is_randomize_check_args(args) {
                return self.exec_randomize_check(handle, &[]);
            }
            // §18.11: a member subset — `obj.randomize(a, b)`. Each argument
            // names a property of the object, so restrict the solve to those
            // and leave the rest as state.
            let subset: Option<HashSet<String>> = if args.is_empty() {
                None
            } else {
                let mut names: HashSet<String> = HashSet::default();
                for a in args {
                    match &a.kind {
                        ExprKind::Ident(h) if h.path.len() == 1 => {
                            names.insert(h.path[0].name.name.clone());
                        }
                        _ => return self.exec_randomize(handle),
                    }
                }
                Some(names)
            };
            let saved = std::mem::replace(&mut self.randomize_subset, subset);
            let r = self.exec_randomize(handle);
            self.randomize_subset = saved;
            return r;
        }

        // Built-in mailbox / semaphore methods
        if self.ipc.mailboxes.contains_key(&handle) {
            match method_name {
                "put" => {
                    if let Some(arg) = args.first() {
                        let v = self.eval_expr(arg);
                        // LRM §15.4.2: if a blocking get is waiting on this
                        // mailbox, hand the value directly to the waiter
                        // (skipping the queue) and reschedule its
                        // continuation at the current time.
                        let waiter = self
                            .ipc.mailbox_get_waiters
                            .get_mut(&handle)
                            .and_then(|q| q.pop_front());
                        if let Some(w) = waiter {
                            let MailboxGetWaiter {
                                pid,
                                lvalue,
                                cont,
                                is_peek,
                            } = w;
                            if is_peek {
                                // peek doesn't consume — leave the item for the
                                // subsequent get/try_get (sequencer item_done).
                                self.ipc.mailboxes
                                    .get_mut(&handle)
                                    .unwrap()
                                    .push_back(v.clone());
                            }
                            self.deliver_to_mailbox_waiter(pid, &lvalue, v, cont);
                        } else {
                            self.ipc.mailboxes.get_mut(&handle).unwrap().push_back(v);
                        }
                    }
                    return Value::zero(32);
                }
                "get" | "peek" => {
                    let val = if method_name == "get" {
                        self.ipc.mailboxes.get_mut(&handle).and_then(|q| q.pop_front())
                    } else {
                        self.ipc.mailboxes.get(&handle).and_then(|q| q.front().cloned())
                    };
                    if let (Some(v), Some(arg)) = (val, args.first()) {
                        let w = self.infer_lhs_width(arg);
                        self.assign_value(arg, &v.resize(w));
                    }
                    // A consuming `get` frees a slot — admit a parked producer.
                    if method_name == "get" {
                        self.admit_mailbox_put_waiter(handle);
                    }
                    return Value::zero(32);
                }
                "try_put" => {
                    if let Some(arg) = args.first() {
                        // §15.4.1: a bounded mailbox that is full rejects try_put
                        // (returns 0). A full box has no parked get-waiter (those
                        // park only on empty), so len>=bound is the whole test.
                        let bound = self.ipc.mailbox_bound.get(&handle).copied().unwrap_or(0);
                        if bound > 0 {
                            let len = self.ipc.mailboxes.get(&handle).map(|q| q.len()).unwrap_or(0);
                            if len >= bound {
                                return Value::zero(32);
                            }
                        }
                        let v = self.eval_expr(arg);
                        let waiter = self
                            .ipc.mailbox_get_waiters
                            .get_mut(&handle)
                            .and_then(|q| q.pop_front());
                        if let Some(w) = waiter {
                            let MailboxGetWaiter {
                                pid,
                                lvalue,
                                cont,
                                is_peek,
                            } = w;
                            if is_peek {
                                self.ipc.mailboxes
                                    .get_mut(&handle)
                                    .unwrap()
                                    .push_back(v.clone());
                            }
                            self.deliver_to_mailbox_waiter(pid, &lvalue, v, cont);
                        } else {
                            self.ipc.mailboxes.get_mut(&handle).unwrap().push_back(v);
                        }
                    }
                    return Value::from_u64(1, 32);
                }
                "try_get" | "try_peek" => {
                    let val = if method_name == "try_get" {
                        self.ipc.mailboxes.get_mut(&handle).and_then(|q| q.pop_front())
                    } else {
                        self.ipc.mailboxes.get(&handle).and_then(|q| q.front().cloned())
                    };
                    if let (Some(v), Some(arg)) = (val, args.first()) {
                        let w = self.infer_lhs_width(arg);
                        self.assign_value(arg, &v.resize(w));
                        if method_name == "try_get" {
                            self.admit_mailbox_put_waiter(handle);
                        }
                        return Value::from_u64(1, 32);
                    }
                    return Value::zero(32);
                }
                "num" => {
                    let n = self.ipc.mailboxes.get(&handle).map(|q| q.len()).unwrap_or(0);
                    return Value::from_u64(n as u64, 32);
                }
                _ => {}
            }
        }
        if self.ipc.semaphores.contains_key(&handle) {
            match method_name {
                "put" => {
                    let n = args
                        .first()
                        .map(|a| self.eval_expr(a).to_u64().unwrap_or(1))
                        .unwrap_or(1) as i64;
                    *self.ipc.semaphores.get_mut(&handle).unwrap() += n;
                    self.wake_semaphore_waiters(handle);
                    return Value::zero(32);
                }
                "get" => {
                    let n = args
                        .first()
                        .map(|a| self.eval_expr(a).to_u64().unwrap_or(1))
                        .unwrap_or(1) as i64;
                    let count = self.ipc.semaphores.get_mut(&handle).unwrap();
                    if *count >= n {
                        *count -= n;
                    }
                    return Value::zero(32);
                }
                "try_get" => {
                    let n = args
                        .first()
                        .map(|a| self.eval_expr(a).to_u64().unwrap_or(1))
                        .unwrap_or(1) as i64;
                    let count = self.ipc.semaphores.get_mut(&handle).unwrap();
                    if *count >= n {
                        *count -= n;
                        return Value::from_u64(1, 32);
                    }
                    return Value::zero(32);
                }
                _ => {}
            }
        }
        let class_name = if let Some(Some(inst)) = self.oop.heap.get(handle) {
            inst.class_name.clone()
        } else {
            return Value::zero(32);
        };
        // LRM §8.7/§8.23: a STATIC method is resolved/stored separately from
        // instance methods and never dereferences `this`; a call through an
        // object handle (e.g. `req.get_type()`) must dispatch statically, even
        // when the object is live. Dispatch here (the common funnel) so every
        // instance-path route resolves static methods correctly.
        if self.is_static_method(&class_name, method_name) {
            if let Some(res) =
                self.exec_static_method(&class_name, method_name, args)
            {
                return res;
            }
        }
        self.exec_method_in_class_hierarchy(handle, &class_name, method_name, args)
    }


    pub(crate) fn class_prop_width_of(&self, handle: usize, prop: &str) -> Option<u32> {
        let cn = self.heap_obj(handle)?.class_name.clone();
        self.class_prop_width(&cn, prop)
    }


    pub(crate) fn exec_method_in_class_hierarchy(
        &mut self,
        handle: usize,
        start_class: &str,
        method_name: &str,
        args: &[Expression],
    ) -> Value {
        use crate::ast::decl::{ClassMethod, ClassMethodKind};
        // Re-entrant registration guard: a factory `register(obj)` whose
        // body calls `obj.get_type_name()` can trigger a parameterized class
        // specialization's lazy static init, which re-enters
        // `factory.register(obj)` for the SAME object. Skip the re-entrant
        // call so the outer one performs the single registration (else the
        // outer call resumes to find the type already stored -> UVM TPRGED).
        // `register`'s argument is a handle; `insert` returns false when the
        // handle is already mid-registration.
        let reg_guard_obj: Option<usize> = if method_name == "register" {
            args.first()
                .and_then(|a| self.eval_expr(a).to_u64())
                .map(|h| h as usize)
                .filter(|&h| h != 0)
        } else {
            None
        };
        if let Some(h) = reg_guard_obj {
            if !self.factory_reg_in_progress.insert(h) {
                return Value::zero(32);
            }
        }
        let mut cur_class = Some(start_class.to_string());
        while let Some(cname) = cur_class {
            // Resolve the matched method + parent-class pointer in a scoped
            // borrow, cloning AT MOST the single matched method — never the
            // whole ElaboratedClass. Cloning the entire class (every method,
            // property, constraint, param map, …) on each lookup made method
            // dispatch O(class_size × call_count); under heavy call load
            // through deep inheritance hierarchies that is pathological. Now
            // only the matched Function/Task body is ever cloned; a miss
            // (absent / extern / pure) walks up for free.
            let (method_opt, parent): (Option<ClassMethod>, Option<String>) =
                match self.module.classes.get(&cname) {
                    Some(class_def) => (
                        class_def
                            .methods
                            .get(method_name)
                            .filter(|m| matches!(
                                m.kind,
                                ClassMethodKind::Function(_) | ClassMethodKind::Task(_)
                            ))
                            .cloned(),
                        class_def.extends.clone(),
                    ),
                    None => (None, None),
                };
            cur_class = parent;
            if let Some(method) = method_opt {
                let (ports, body) = match &method.kind {
                    ClassMethodKind::Function(f) => (&f.ports, &f.items),
                    ClassMethodKind::Task(t) => (&t.ports, &t.items),
                    _ => unreachable!("non-Function/Task methods filtered out above"),
                };
                let mut locals: HashMap<String, Value> = HashMap::default();
                // §13.5.3: reorder named args into formal order and fill any
                // omitted (`.name()` / positional `,,`) slot from the formal's
                // default before positional binding below.
                let normalized = Self::normalize_call_args(ports, args);
                let args: &[Expression] = normalized.as_deref().unwrap_or(args);
                // A function may set its result via the implicit
                // return variable named after the function (`f = ...`)
                // instead of an explicit `return`.
                let fn_ret_name: Option<String> = match &method.kind {
                    ClassMethodKind::Function(f) => Some(f.name.name.name.clone()),
                    _ => None,
                };
                let ret_is_string = matches!(&method.kind,
                    ClassMethodKind::Function(f) if Self::is_string_data_type(&f.return_type));
                // A packed return type whose range references a CLASS
                // parameter (`function bit [W-1:0] mk();`) can only be
                // sized per instance — capture it for the clamp at the
                // return point below. Literal ranges resolve here (params
                // = None succeeds) and stay un-clamped as before.
                let dyn_ret_type: Option<DataType> = match &method.kind {
                    ClassMethodKind::Function(f) => {
                        let dims = match &f.return_type {
                            DataType::IntegerVector { dimensions, .. } => Some(dimensions),
                            DataType::Implicit { dimensions, .. } => Some(dimensions),
                            _ => None,
                        };
                        dims.filter(|ds| {
                            ds.iter().any(|d| {
                                matches!(d, crate::ast::types::PackedDimension::Range { left, right, .. }
                                    if crate::elaborate::const_eval_i64_with_params(left, None).is_none()
                                        || crate::elaborate::const_eval_i64_with_params(right, None).is_none())
                            })
                        })
                        .map(|_| f.return_type.clone())
                    }
                    _ => None,
                };
                self.push_queue_frame();
                // `output`/`inout`/`ref` formals copy back to the caller's
                // actual on return (e.g. `randomize_instr(output riscv_instr
                // instr, …)` writing the picked instruction to `instr_list[i]`).
                let mut output_bindings: Vec<(String, Expression)> = Vec::new();
                let mut queue_writebacks: Vec<(String, String)> = Vec::new();
                let mut array_writebacks: Vec<(String, String, i64, i64)> = Vec::new();
                let mut assoc_params: Vec<(String, String, bool)> = Vec::new();
                // Member-wise struct `output`/`inout`/`ref` formals bypass the
                // whole-value `output_bindings` path (see exec_function_call).
                let mut struct_output_writebacks: Vec<(String, Expression)> = Vec::new();
                for (i, port) in ports.iter().enumerate() {
                    let is_assoc = self.port_is_assoc_array(port);
                    // An associative-array `output`/`inout`/`ref` formal
                    // is copied back via the assoc-param signal-namespace
                    // merge, NOT the scalar `output_bindings` path (which
                    // can't represent an AA). §13.5.2.
                    if matches!(
                        port.direction,
                        PortDirection::Output | PortDirection::Inout | PortDirection::Ref
                    ) && i < args.len()
                        && !is_assoc
                    {
                        output_bindings.push((port.name.name.clone(), args[i].clone()));
                    }
                    if i < args.len() {
                        if let Some((param, caller)) = self.bind_assoc_param(port, &args[i]) {
                            let is_out = matches!(
                                port.direction,
                                PortDirection::Output
                                    | PortDirection::Inout
                                    | PortDirection::Ref
                            );
                            assoc_params.push((param, caller, is_out));
                            continue;
                        }
                        if let Some(struct_entries) = self.bind_unpacked_struct_arg(&port.name.name, &port.data_type, &args[i], &mut locals, Some(handle)) {
                            if matches!(
                                port.direction,
                                PortDirection::Output
                                    | PortDirection::Inout
                                    | PortDirection::Ref
                            ) {
                                struct_output_writebacks.extend(struct_entries);
                            }
                            continue;
                        }
                        // §6.18/§6.20.3: typedef'd / type-param-bound
                        // formal — dims live on the type (see the same
                        // resolution in exec_function_call).
                        if std::env::var("XZ_BD_DBG").is_ok() {
                            if let crate::ast::types::DataType::TypeReference { name, .. } =
                                &port.data_type
                            {
                                eprintln!(
                                    "[BDDBG] port={} tn={} in_tud={} tud_keys={:?}",
                                    port.name.name,
                                    name.name.name,
                                    self.module
                                        .typedef_unpacked_dims
                                        .contains_key(&name.name.name),
                                    self.module
                                        .typedef_unpacked_dims
                                        .keys()
                                        .collect::<Vec<_>>()
                                );
                            }
                        }
                        let eff_dims: Vec<crate::ast::types::UnpackedDimension> = if port
                            .dimensions
                            .is_empty()
                        {
                            if let crate::ast::types::DataType::TypeReference { name, .. } =
                                &port.data_type
                                {
                                    let tn = &name.name.name;
                                    // Resolve against the CALLEE's own
                                    // type bindings — `this` isn't pushed
                                    // yet while formals bind.
                                    let concrete = self
                                        .oop.heap
                                        .get(handle)
                                        .and_then(|o| o.as_ref())
                                        .and_then(|i| i.type_bindings.get(tn).cloned())
                                        .or_else(|| self.resolve_type_param_binding(tn))
                                        .unwrap_or_else(|| tn.clone());
                                    // CLASS-LOCAL typedef (`typedef int
                                    // q_t[$];` inside the class): the dims
                                    // live on the callee class's own
                                    // typedef table, walked before the
                                    // module's — a `ref q_t out_q` formal
                                    // was bound as a scalar, so the
                                    // callee's push_backs never reached
                                    // the caller (§13.5.2).
                                Self::typedef_dims_via_tables(&self.module, &cname, &concrete)
                                    .unwrap_or_default()
                                } else {
                                    Vec::new()
                                }
                            } else {
                                port.dimensions.clone()
                            };
                        if let Some(caller) = self.bind_queue_param(
                            &port.name.name,
                            &eff_dims,
                            &args[i],
                            &port.data_type,
                        ) {
                            let is_out = matches!(
                                port.direction,
                                PortDirection::Output
                                    | PortDirection::Inout
                                    | PortDirection::Ref
                            );
                            if is_out && !caller.is_empty() {
                                queue_writebacks.push((port.name.name.clone(), caller));
                            }
                            continue;
                        }
                        // §13.5.2 / §18.5.12: a FIXED unpacked-array formal
                        // (`function bit [7:0] parity(bit [7:0] a[4])`) is
                        // passed BY VALUE. Only the free-function path bound
                        // one; a class method bound it as a scalar, so the
                        // body read X and a user function taking an array —
                        // legal in a constraint — always returned 0.
                        if let Some(info) = self.bind_array_arg(port, &args[i]) {
                            if matches!(
                                port.direction,
                                PortDirection::Output
                                    | PortDirection::Inout
                                    | PortDirection::Ref
                            ) {
                                array_writebacks.push(info);
                            }
                            continue;
                        }
                    }
                    let mut val = if i < args.len() {
                        self.eval_expr(&args[i])
                    } else if let Some(def) = &port.default {
                        // Caller omitted this arg → apply the formal's
                        // default (was using 0). UVM uvm_sequence_base::
                        // start(..., int this_priority = -1): missing arg
                        // must be -1, not 0, else `this_priority < -1`
                        // (read unsigned) fatals SEQPRI.
                        self.eval_expr(def)
                    } else {
                        Value::zero(32)
                    };
                    // §13.5.1/§6.18/§10.7: a scalar INTEGRAL formal adopts its
                    // declared (possibly typedef-derived) width and signedness.
                    // Without this a class-method formal declared as a wider
                    // typedef (e.g. `uvm_integral_t` = 64-bit) bound a 32-bit
                    // actual as-is, so the high bits read x and uvm_pack_*
                    // emitted garbage for `$realtobits(shortreal)` and negative
                    // ints passed to a 64-bit packer formal. This is scoped to
                    // TYPEREFERENCE formals only: a direct `bit[W-1:0]`/`int`
                    // ports stays on the old (signedness-only) path so a width
                    // derived from a CLASS type-param (not in module.parameters)
                    // isn't mis-resolved to `1` and truncates the caller's
                    // value.
                    if matches!(
                        &port.data_type,
                        DataType::TypeReference { .. }
                    ) {
                        if let Some((pw, signed)) = self.scalar_formal_integral(&port.data_type) {
                            val = if val.is_real {
                                Self::real_to_int(val.to_f64(), pw.max(1))
                            } else if pw != val.width {
                                val.resize_for_assign(pw)
                            } else {
                                val
                            };
                            val.is_signed = signed;
                        }
                    } else if crate::compiler::elaborate::is_type_signed(&port.data_type) {
                        val.is_signed = true;
                    }
                    if let DataType::TypeReference { name: tn, .. } = &port.data_type {
                        let type_name = tn.name.name.clone();
                        if self.module.enum_members.contains_key(&type_name)
                            || self.module.typedefs.contains_key(&type_name)
                        {
                            self.var_typedef_types.insert(port.name.name.clone(), type_name);
                        } else if self.module.classes.contains_key(&type_name) {
                            self.oop.var_class_types.insert(port.name.name.clone(), type_name.clone());
                        }
                    }
                    locals.insert(port.name.name.clone(), val);
                }
                if let Some(rn) = &fn_ret_name {
                    // An UNPACKED-struct return type keeps its members in the
                    // frame under `<fn>.<member>`, like any other local of that
                    // type; without them a member write inside the body escaped
                    // the frame.
                    if let ClassMethodKind::Function(f) = &method.kind {
                        if let Some(su) = self.unpacked_struct_of(&f.return_type) {
                            for (k, w, is_real) in
                                self.unpacked_struct_leaf_keys(&rn.clone(), &su)
                            {
                                let seed = if is_real {
                                    Value::from_f64(0.0)
                                } else {
                                    Value::new(w)
                                };
                                locals.insert(k, seed);
                            }
                        }
                    }
                    // Initialise the implicit return variable to match its
                    // declared type. A STRING return must start as an empty
                    // string Value, not a 32-bit int — otherwise an implicit
                    // `funcname = {a, "b", ...}` string-concat assignment
                    // coerces to the int's 32-bit width and reads back empty.
                    let init = if let ClassMethodKind::Function(f) = &method.kind {
                        if Self::is_string_data_type(&f.return_type) {
                            Value::from_string("")
                        } else {
                            // Size the implicit return cell to the declared
                            // return width so a bit-select write
                            // `retname[i] = ...` for i >= 32 (unpack routines
                            // filling a 64-bit integral type) lands; a 32-bit
                            // cell dropped the upper half. Register the width
                            // too so a later `retname = <narrow>` zero-extends
                            // back, mirroring a typed VarDecl.
                            let rw = crate::compiler::elaborate::resolve_type_width(
                                &f.return_type,
                                Some(&self.module.parameters),
                                Some(&self.module.typedefs),
                            )
                            .max(1);
                            self.widths.insert(rn.clone(), rw);
                            // §13.4.1: type default — x for 4-state (see
                            // the module-function twin above).
                            if crate::compiler::elaborate::is_type_real(&f.return_type) {
                                Value::from_f64(0.0)
                            } else if crate::compiler::elaborate::is_type_two_state(&f.return_type) {
                                Value::zero(rw)
                            } else {
                                Value::new(rw)
                            }
                        }
                    } else {
                        Value::zero(32)
                    };
                    locals.entry(rn.clone()).or_insert(init);
                    // Register the return variable's class type so an
                    // implicit-return `funcname = new()` knows WHICH class to
                    // construct — exactly as a typed local `T t = new()` does
                    // (var_class_types is consulted when a bare `new()` has no
                    // explicit class). Without this the bare `new()` can't
                    // resolve its class and yields a broken instance whose
                    // method calls bind the wrong `this`/params. This is the
                    // `uvm_report_message::new_report_message` form
                    // (`new_report_message = new(name)`); fixing it makes the
                    // real report message's set_action/get_action persist so
                    // the report-server `$display` fires natively.
                    if let ClassMethodKind::Function(f) = &method.kind {
                        if let crate::ast::types::DataType::TypeReference { name, .. } =
                            &f.return_type
                        {
                            let cn = name.name.name.clone();
                            if self.module.classes.contains_key(&cn) {
                                self.oop.var_class_types.insert(rn.clone(), cn);
                            }
                        }
                    }
                }
                // Mark string-typed return variable and params so `s[i]`
                // does byte (character) indexing instead of bit-select.
                // §23.8: a formal's name is local to THIS frame; recording it
                // in the shared `string_signals` set must not leak past the
                // return — uvm's `pack_string(string value)` would otherwise
                // make `pack_field_int(uvm_integral_t value)` resolve `value`
                // as a string, so `value[i]` byte-selects instead of
                // bit-selecting. Track only newly-added names and remove them
                // on exit; HashSet::insert returning false (name already
                // present from an active caller) is left untouched.
                let mut frame_string_signals: Vec<String> = Vec::new();
                if let ClassMethodKind::Function(f) = &method.kind {
                    if Self::is_string_data_type(&f.return_type) {
                        if self.string_signals.insert(f.name.name.name.clone()) {
                            frame_string_signals.push(f.name.name.name.clone());
                        }
                    }
                }
                for port in ports.iter() {
                    if Self::is_string_data_type(&port.data_type) {
                        if self.string_signals.insert(port.name.name.clone()) {
                            frame_string_signals.push(port.name.name.clone());
                        }
                    }
                }
                // §6.16: a class's STRING properties must be in `string_signals`
                // so that `foreach(str[i])`, `str[i]` (char select), and
                // `str.len()` dispatch to the string paths. Walk the inheritance
                // chain from the callee's class and add each string property
                // for the duration of this frame (removed on exit below).
                {
                    let mut cur = Some(cname.clone());
                    while let Some(cn) = cur {
                        if let Some(cd) = self.module.classes.get(&cn) {
                            for sp in &cd.string_properties {
                                if self.string_signals.insert(sp.clone()) {
                                    frame_string_signals.push(sp.clone());
                                }
                            }
                            cur = cd.extends.clone();
                        } else {
                            break;
                        }
                    }
                }
                // Loop-control flags (`break`/`continue`) are frame-local:
                // save the caller's, clear them for this body, and restore
                // afterward so a `continue`/`break` inside the callee can't
                // leak out and poison the caller's subsequent statements
                // (exec_statement bails when either flag is set).
                let saved_break = self.break_flag;
                let saved_continue = self.continue_flag;
                let saved_return = self.return_flag;
                self.break_flag = false;
                self.continue_flag = false;
                self.return_flag = false;
                // LRM §25.9 virtual-interface formal-arg aliasing
                // (class-method dispatch). Resolve the actuals' vif bindings
                // NOW, while `this_stack` is still the CALLER's context
                // (the bound vif lives there), before pushing the callee's.
                let mut iface_alias_frame: HashMap<String, String> = HashMap::default();
                for (i, port) in ports.iter().enumerate() {
                    if i < args.len() {
                        if let Some((f, b)) =
                            self.vif_formal_alias(&port.data_type, &port.name.name, &args[i])
                        {
                            iface_alias_frame.insert(f, b);
                        }
                    }
                }
                // PURE_SV_LRM §8.25: a `static` member of a parameterized
                // class is per-SPECIALIZATION. Reached from an INSTANCE method
                // (e.g. `resource#(T)::my_type` via `r.get_type_handle()->
                // get_type()`) there is no `#(spec)` on the call, so
                // `current_spec` is None and static_prop_key uses the shared
                // `Class::member` cell — while an explicit `Class#(spec)::member`
                // uses `Class#spec::member`. That made get_type() !=
                // get_type_handle(), breaking resource-pool type matching.
                // Seed current_spec from the instance's own type bindings so
                // both paths key the same per-spec cell. Sig = the type params'
                // bound leaf names in declaration order (the parser now captures
                // builtin type args as Ident leaf names, so this matches
                // extract_call_spec's type_args_text).
                let saved_spec = self.current_spec.clone();
                if let Some(inst) = self.heap_obj(handle) {
                    let cn = inst.class_name.clone();
                    let bindings = inst.type_bindings.clone();
                    // Override the active spec with THIS instance's own
                    // specialization when it differs from the active
                    // spec's class. Prefer the instance's captured full
                    // specialization (`spec`), which carries BOTH type
                    // and value params as a complete sig (unlike the
                    // type_bindings-only rebuild below). The full `spec`
                    // is what restores the specialization a
                    // typedef-specialization singleton was constructed
                    // under, so a later virtual call can answer
                    // value-param lookups (the factory
                    // get_type_name chain). The type_bindings rebuild is
                    // the legacy fallback: an instance method of
                    // `resource#(int)` must key resource's
                    // per-spec cell even when called while an UNRELATED
                    // spec is active (e.g. the enclosing
                    // `config_db#(int)`, whose base `config_db`
                    // wouldn't match `resource` in static_prop_key
                    // and would fall back to the shared unspec'd cell →
                    // the get_type/get_type_handle mismatch).
                    let differs = match self.current_spec.as_ref() {
                        None => true,
                        // Switch when the base class differs OR the
                        // instance's own specialization (same base,
                        // different sig) differs. The latter is the
                        // callback typewide-recursion case:
                        // `base_comp.m_add_tw_cbs` calls
                        // `cb_pair.m_add_tw_cbs(cb)` where cb_pair is a
                        // DIFFERENT specialization's instance (e.g.
                        // `callbacks#(b_comp)`). Without restoring
                        // the instance's spec, the recursed method's
                        // `m_t_inst.m_tw_cb_q` static access keys off
                        // the caller's (base_comp) cell, so typewide
                        // callbacks never propagate to derived types.
                        Some((b, s)) => {
                            *b != cn
                                || inst
                                    .spec
                                    .as_ref()
                                    .is_some_and(|(ib, is)| {
                                        ib != b || is != s
                                    })
                        }
                    };
                    if differs {
                        if inst.spec.is_some() {
                            self.current_spec = inst.spec.clone();
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
                                    self.current_spec = Some((cn.clone(), sig_frags.join(",")));
                                }
                            }
                        }
                    }
                }
                self.oop.this_stack.push(Some(handle));
                self.local_stack.push(locals);
                // Record the `local_stack` depth BEFORE this method's own
                // frame (i.e. the count of caller frames) so that
                // `get_expr_type_name`'s `in_any_frame` check only considers
                // the current method's locals — not a caller's same-named
                // local that leaked into the flat `var_class_types` map.
                self.oop.method_local_base.push(self.local_stack.len() - 1);
                self.oop.class_context_stack.push(Some(cname.clone()));
                // §8.7 ctor-time binding (reference behavior): while class
                // C's `new` body runs, an unqualified method call on `this`
                // binds within C's own chain — a derived override must not
                // run against a partially-constructed object.
                if method_name == "new" {
                    self.oop.ctor_class_stack.push((handle, cname.clone()));
                }
                self.local_iface_aliases.push(iface_alias_frame);
                // §6.21: open a static-local sync frame so a `static`
                // local declared in the method body persists across calls.
                // (Class methods previously never opened one — only free
                // functions/tasks did — so a `static this_type m_inst;`
                // inside `get()` was re-initialized every call and class
                // factory singletons never survived.) The key itself is
                // made class/spec-aware at the declaration site above.
                self.static_local_syncs
                    .push((method_name.to_string(), Vec::new()));
                for stmt in body {
                    self.exec_statement(stmt);
                    if self.break_flag || self.return_flag {
                        break;
                    }
                }
                // Write back any static locals declared in this body before
                // the locals frame is dropped.
                self.sync_static_locals();
                // §23.8: stop leaking this frame's string formal names into
                // the global set now that the body is done (see
                // frame_string_signals).
                for n in &frame_string_signals {
                    self.string_signals.remove(n);
                }
                self.current_spec = saved_spec;
                self.local_iface_aliases.pop();
                if self.proc.parked_from_exec {
                    // The process was parked by exec_statement's Wait handler.
                    // Keep break/return flags set so they propagate up to
                    // run_process_stmts. Don't restore the saved values.
                } else {
                    self.break_flag = saved_break;
                    self.continue_flag = saved_continue;
                    self.return_flag = saved_return;
                }
                if method_name == "new" {
                    self.oop.ctor_class_stack.pop();
                }
                self.oop.class_context_stack.pop();
                self.oop.method_local_base.pop();
                // §13.4.1: a method returning an UNPACKED struct has no single
                // return cell — its members are frame leaves, so the bare name
                // read back x. Collapse them into the packed form while the
                // frame is still live, exactly as a free function does.
                let unpacked_ret_su = match &method.kind {
                    ClassMethodKind::Function(f) => self.unpacked_struct_of(&f.return_type),
                    _ => None,
                };
                if let (Some(rn), Some(su)) = (fn_ret_name.as_ref(), unpacked_ret_su.as_ref()) {
                    if self.return_value.is_none() {
                        if let Some(v) = self.pack_unpacked_struct(&rn.clone(), &su.clone()) {
                            self.return_value = Some(v);
                        }
                    }
                }
                let implicit = fn_ret_name.as_ref().and_then(|rn| {
                    let lv = self.local_stack.last().and_then(|m| m.get(rn).cloned());
                    // A `funcname = {a, b, ...}` string-concat assignment is
                    // serviced by the string-concat path, which writes the
                    // result via set_signal_value_by_name (the SIGNAL store),
                    // not the local frame — so a string return whose local is
                    // still empty has its real value in the signal store.
                    if ret_is_string
                        && lv.as_ref().is_none_or(|v| v.to_sv_string().is_empty())
                    {
                        self.get_signal_value_by_name(rn).or(lv)
                    } else {
                        lv
                    }
                });
                let mut ret = self
                    .return_value
                    .take()
                    .or(implicit)
                    .unwrap_or(Value::zero(32));
                // Per-instance return clamp for a class-parameter-sized
                // return type: `box#(4)` with `function bit [W-1:0] mk()`
                // must hand back 4 bits, not whatever width the body's
                // last assignment happened to carry.
                if let Some(rt) = &dyn_ret_type {
                    if !ret.is_real {
                        let scope = self.instance_param_scope(handle);
                        let w = resolve_type_width(rt, Some(&scope), Some(&self.module.typedefs));
                        if w > 0 && w != ret.width {
                            ret = ret.resize_for_assign(w);
                        }
                    }
                }
                // §13.4.1: a class method's return takes its DECLARED type,
                // exactly as a module-level function's does. Only the
                // parameter-sized case above was handled, so
                // `function bit [15:0] u; return -1;` handed back the
                // literal's signedness and `c.u() > 0` read it as -1.
                if let ClassMethodKind::Function(f) = &method.kind {
                    // Only PLAINLY integral declared types are stamped — a
                    // TypeReference may name a class (the return is a heap
                    // HANDLE whose "width" is meaningless and whose value
                    // must pass through untouched), and Implicit means no
                    // declared type at all. The first version of this stamp
                    // resized those and broke handle-returning methods.
                    let plainly_integral = matches!(
                        f.return_type,
                        DataType::IntegerVector { .. } | DataType::IntegerAtom { .. }
                    );
                    if plainly_integral
                        && !ret.is_real
                        && !ret_is_string
                        && dyn_ret_type.is_none()
                    {
                        let w = resolve_type_width(
                            &f.return_type,
                            Some(&self.module.parameters),
                            Some(&self.module.typedefs),
                        );
                        if w > 0 && w != ret.width {
                            ret = ret.resize(w);
                        }
                        ret.is_signed = crate::compiler::elaborate::is_type_signed(&f.return_type);
                    }
                }
                // Snapshot output/ref formal values before dropping locals.
                let writebacks: Vec<(Value, Expression)> = output_bindings
                    .iter()
                    .filter_map(|(pn, caller)| {
                        self.local_stack
                            .last()
                            .and_then(|l| l.get(pn).cloned())
                            .map(|v| (v, caller.clone()))
                    })
                    .collect();
                // Member-wise struct output formals: snapshot each member
                // local (`o.a`, `o.b`) before the frame is popped.
                let struct_wb: Vec<(Expression, Value)> = struct_output_writebacks
                    .iter()
                    .filter_map(|(local_key, caller_lval)| {
                        self.local_stack
                            .last()
                            .and_then(|l| l.get(local_key).cloned())
                            .map(|v| (caller_lval.clone(), v))
                    })
                    .collect();
                self.local_stack.pop();
                self.oop.this_stack.pop();
                self.pop_and_restore_queue_frame();
                for (param, caller) in &queue_writebacks {
                    self.writeback_queue_param(param, caller);
                }
                // §13.5.2: copy `output`/`inout`/`ref` fixed-array formals
                // back onto the caller's array (a plain `input` formal is
                // by value and must NOT write back — §18.5.12 relies on
                // that for array args to constraint functions).
                if !array_writebacks.is_empty() {
                    self.writeback_array_args(&array_writebacks);
                }
                for (v, caller) in writebacks {
                    self.assign_value(&caller, &v);
                }
                for (caller_lval, v) in struct_wb {
                    self.assign_value(&caller_lval, &v);
                }
                // §13.5.2: copy `output`/`inout`/`ref` associative-array
                // formals back onto the caller's AA (signal-namespace
                // merge), then drop the formal's temporary copy.
                for (param, caller, is_out) in std::mem::take(&mut assoc_params) {
                    if param == caller {
                        continue;
                    }
                    if is_out {
                        self.writeback_assoc_param(&param, &caller);
                    }
                    self.purge_assoc_param(&param);
                }
                if let Some(h) = reg_guard_obj {
                    self.factory_reg_in_progress.remove(&h);
                }
                return ret;
            }
        }
        if let Some(h) = reg_guard_obj {
            self.factory_reg_in_progress.remove(&h);
        }
        Value::zero(32)
    }


    pub(crate) fn class_prop_type(&self, class_name: &str, prop: &str) -> Option<String> {
        let mut cur = Some(class_name.to_string());
        while let Some(cname) = cur {
            if let Some(cd) = self.module.classes.get(&cname) {
                if let Some(sig) = cd.properties.get(prop) {
                    if let Some(t) = &sig.type_name {
                        if self.module.classes.contains_key(t) {
                            return Some(t.clone());
                        }
                    }
                    return None;
                }
                cur = cd.extends.clone();
            } else {
                break;
            }
        }
        None
    }
}

