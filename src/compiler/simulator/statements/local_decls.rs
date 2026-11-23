//! Declarations of local variables and parameters, and their initialization.

use super::*;

struct VarDeclShape {
    w: u32,
    class_ref: Option<bool>,
    plain_class: bool,
    plain: bool,
    two_state: bool,
    is_real_type: bool,
    decl_signed: bool,
    default_v: Value,
}

impl Simulator {
    /// Outlined from `exec_statement` so the dispatcher's stack frame
    /// stays small: this arm's locals inflated every call's frame (the
    /// dispatcher recursed with a 7.8 KB frame per level).
    #[inline(never)]
    /// §6.8/§10.7: a procedural local's declaration initializer is an
    /// assignment to the declared type — it takes the type's width (an
    /// unbased-unsized fill replicates), signedness and 2-state-ness, and a
    /// real converts. `byte unsigned v = 200` read -56 and `bit [3:0] b =
    /// 4'b1x0z` kept its x/z because the raw initializer value was stored.
    /// A `string` keeps its text unresized: resizing to the placeholder
    /// width padded it with NULs that later concatenations copied.
    pub(in crate::compiler::simulator) fn fit_local_decl_init(
        raw: Value,
        w: u32,
        data_type: &DataType,
        two_state: bool,
        is_real_type: bool,
        signed: bool,
    ) -> Value {
        if is_real_type {
            if matches!(data_type, DataType::Real { kind: crate::ast::types::RealType::ShortReal, .. }) {
                return Value::from_f64(raw.to_f64() as f32 as f64);
            }
            return if raw.is_real {
                raw
            } else {
                Value::from_f64(raw.to_f64())
            };
        }
        if matches!(
            data_type,
            crate::ast::types::DataType::Simple {
                kind: crate::ast::types::SimpleType::String,
                ..
            }
        ) {
            return raw;
        }
        let mut v = if raw.is_real {
            Self::real_to_int(raw.to_f64(), w)
        } else {
            raw.resize_for_assign(w)
        };
        if two_state && v.has_xz() {
            v = v.to_two_state();
        }
        v.is_signed = signed;
        v
    }

    fn resolve_var_decl_shape<'a>(
        &mut self,
        data_type: &'a DataType,
    ) -> (std::borrow::Cow<'a, DataType>, VarDeclShape) {
        // §6.20.2 / §8.25: a local typed by a TYPE PARAMETER of the method's
        // class (`T tmp;`) has the type bound in this specialization.
        let tp_param = self.local_type_param_concrete(data_type);
        // §8.25.1: a class-scope typedef reached through a specialization.
        let tp_scoped = if tp_param.is_none() {
            self.scoped_typedef_concrete(data_type)
        } else {
            None
        };
        let tp_concrete = tp_param.or_else(|| tp_scoped.clone());
        // §6.20.3/§8.25: a binding that names a design typedef (a packed
        // struct, say) is the local's declared type for the metadata below:
        // the member layout, member names and packed dimensions. Under the
        // bare `T` none was registered, so `loc.data` read 0 while `loc`
        // held the right bits.
        // So is a struct whose members name a type parameter (`typedef
        // struct packed { T inner; logic [3:0] tag; } wrap_t;`) with the
        // members bound: `w.inner` had no width in its layout.
        let bound_typedef = match tp_concrete.as_deref() {
            Some(c) => self.bound_typedef_dt(c),
            None => self.struct_with_bound_members(data_type),
        };
        let resolved_dt: std::borrow::Cow<'a, DataType> = match bound_typedef {
            Some(dt) => std::borrow::Cow::Owned(dt),
            None => std::borrow::Cow::Borrowed(data_type),
        };
        let data_type: &DataType = resolved_dt.as_ref();
        let tp_width = tp_concrete
            .as_deref()
            .and_then(|c| self.concrete_type_width(c));
        // §8.25: a local sized by a class parameter or localparam
        // (`bit [N-1:0] x;` in a method of `C #(N)`) takes the executing
        // specialization's value; the module's parameter table has no `N`
        // (#681).
        let w0 = match tp_width
            .or_else(|| self.bound_agg_width(data_type))
            .or_else(|| self.class_param_sized_width(data_type))
        {
            Some(w) => w,
            None => self.resolve_type_width_in_process_scope(data_type),
        };
        // A class-handle-typed variable holds a 32-bit handle, not
        // the class's content layout (which `resolve_type_width`
        // may report as 0 for a class with only methods/statics).
        // A 0 width is never valid for a variable.
        let w = if w0 == 0 { 32 } else { w0 };
        // A built-in scalar or a plain class type: the packed-layout,
        // struct, enum, typedef and type-parameter probes below all come
        // back empty for it (a plain class resolves to itself).
        let class_ref = self.plain_class_ref(data_type);
        let plain_class = class_ref.is_some();
        let plain = plain_class || Self::type_is_plain_scalar(data_type);
        // §6.8/§6.18: resolve typedef aliases before deciding 2- vs
        // 4-state — `uvm_reg_data_t value_adjust;` (typedef of `bit
        // [63:0]`) must initialize to 0, not x (UVM's field
        // read-modify-write ORs into it, so an x seed poisoned the
        // whole register write). A class handle defaults to null either
        // way.
        let two_state = !plain_class
            && match tp_concrete.as_deref().filter(|_| tp_width.is_some()) {
                Some(c) => self.concrete_type_two_state(c),
                None => super::elaborate::is_type_two_state_resolved(
                    data_type,
                    &self.module.typedef_types,
                ),
            };
        // LRM §8.4: an uninitialized class handle defaults to
        // `null`. Treat class-handle types and `chandle` as
        // two-state-zero so `if (h == null)` works without an
        // explicit `= null`. We detect class handles by
        // resolving the type name against `module.classes`.
        let is_class_handle = match data_type {
            crate::ast::types::DataType::TypeReference { .. } => {
                plain_class || self.type_ref_is_class_handle(data_type)
            }
            crate::ast::types::DataType::Simple {
                kind: crate::ast::types::SimpleType::Chandle,
                ..
            } => true,
            _ => false,
        };
        // LRM §8.4: a class handle defaults to `null`. A class-handle
        // type whose name is a typedef/parameterized alias (e.g.
        // `resource_types::rsrc_q_t` → `queue#(...)`) isn't
        // found directly in `module.classes`, so `is_class_handle` is
        // false — but its width resolves to 0 (no data layout). Treat a
        // 0-width type as a handle and default it to zero, so an
        // uninitialized `rsrc_q_t rq; if (rq == null) rq = new();` works
        // inside a method (else `rq` reads X, `rq==null` is X, the
        // `new()` is skipped, and X is stored — breaking resource
        // pool population).
        // A class-scoped typedef (e.g.
        // `resource_types::rsrc_q_t` → `queue#(...)`, declared
        // INSIDE a class) is registered in neither `typedefs` (the width
        // map) nor `typedef_types`, so `resolve_type_width` fell back to
        // 32 and `is_class_handle` is false. A bare TypeReference naming
        // neither a known width-typedef nor a known class is almost
        // certainly such a handle alias — default it to null so the
        // ubiquitous `rsrc_q_t rq; if (rq == null) rq = new();` idiom
        // works inside a method (else `rq` reads X, `rq==null` is X, the
        // `new()` is skipped, and X is stored — which silently broke
        // resource pool population).
        let unknown_typeref_handle = !two_state
            && !is_class_handle
            && match data_type {
                crate::ast::types::DataType::TypeReference {
                    name, dimensions, ..
                } => {
                    dimensions.is_empty()
                        && !self.module.typedefs.contains_key(&name.name.name)
                        && !self.module.typedef_types.contains_key(&name.name.name)
                }
                _ => false,
            };
        // §6.4: `real`/`shortreal` variables default to 0.0.
        let is_real_type = self.dt_is_real(data_type);
        // §6.11.3: the declared signedness every declarator's value and
        // later reads carry.
        let decl_signed = !plain_class
            && match tp_scoped.as_deref() {
                Some(c) => self.concrete_type_signed(c) == Some(true),
                None => self.type_is_signed_concrete(data_type),
            };
        // §25.9 / IEEE 1800-2023 Table 6-7: a virtual-interface handle
        // defaults to `null`, also when its type is a typedef alias
        // (`vif_t v;`) or an explicit `virtual bus_if v` declaration.
        let is_vif_handle = !is_class_handle && self.decl_is_virtual_iface(data_type);
        let mut default_v = if is_real_type {
            Value::from_f64(0.0)
        } else if two_state || is_class_handle || w0 == 0 || unknown_typeref_handle || is_vif_handle
        {
            Value::zero(w)
        } else {
            Value::new(w)
        };
        // §6.11.3: the initial cell carries its declared signing before its
        // first assignment. Frame-local stores use this stable type stamp.
        default_v.is_signed = decl_signed;
        let shape = VarDeclShape {
            w,
            class_ref,
            plain_class,
            plain,
            two_state,
            is_real_type,
            decl_signed,
            default_v,
        };
        (resolved_dt, shape)
    }

    fn init_class_or_covergroup_local(
        &mut self,
        data_type: &DataType,
        class_name: Option<&str>,
        init_expr: &Expression,
    ) -> Option<Value> {
        // §8.8: `B h = C::new(...)` builds a `C`, not the
        // declared class (the `new` forms below).
        let mut produced: Option<Value> = Self::typed_ctor_call(init_expr)
            .and_then(|(prefix, args)| self.construct_typed_ctor(prefix, args));
        if let Some(cn) = class_name.filter(|_| produced.is_none()) {
            // `T x = new(...);` / `T x = new;` — detect the
            // constructor form once; consumed by both the
            // class and covergroup branches below.
            let is_new = match &init_expr.kind {
                ExprKind::Call { func, args } => {
                    if let ExprKind::Ident(h) = &func.kind {
                        if h.path.last().is_some_and(|s| s.name.name == "new") {
                            Some(args.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                ExprKind::Ident(h) => {
                    if h.path.last().is_some_and(|s| s.name.name == "new") {
                        Some(vec![])
                    } else {
                        None
                    }
                }
                _ => None,
            };
            // §8.25: the declared type resolved to a
            // SPECIALIZED class name `Base#(args)` (a type
            // param bound to a nested specialization, e.g.
            // `T tmp = new;` with T = `queue#(string)` —
            // UVM's uvm_object_string_pool::get building
            // the hdl-paths queue). The plain-name lookup
            // below misses it, so the init fell to generic
            // eval and the local read x. Mirror the
            // assignment arm: extract the spec, carry it
            // through `current_spec`, construct the base.
            // §15.3/§15.4: `mailbox mb = new(2);` /
            // `semaphore s = new(1);` as a block-local
            // declaration — the built-in containers are not in
            // `module.classes`, so the constructor was skipped
            // and the local stayed null.
            if produced.is_none() && !self.module.classes.contains_key(cn) {
                if let (Some(kind), Some(call_args)) =
                    (Self::container_base(cn), is_new.as_ref())
                {
                    let handle = self.alloc_builtin_container(kind, call_args);
                    produced = Some(Value::from_u64(handle as u64, 32));
                }
            }
            if produced.is_none() && is_new.is_some() && !self.module.classes.contains_key(cn) {
                if let Some((base, sig)) = self.extract_spec_from_string(cn) {
                    if let Some(class_def) = self.module.classes.get(&base).cloned() {
                        let call_args = is_new.clone().unwrap();
                        self.ensure_spec_statics(&base, &sig);
                        let saved_spec = self.current_spec.clone();
                        self.current_spec = Some((base.clone(), sig));
                        produced = Some(self.instantiate_class(&class_def, &call_args));
                        self.current_spec = saved_spec;
                    }
                }
            }
            if let Some(class_def) = self.module.classes.get(cn).cloned() {
                if let Some(call_args) = is_new {
                    // §8.12 copy constructor: `T x = new src;`
                    // with a single CLASS-HANDLE argument
                    // shallow-copies `src`, it does NOT run
                    // the constructor. The assignment path
                    // handles this; the decl-init path used
                    // to fall straight through to construction
                    // (passing the handle as an ignored ctor
                    // arg), so `T x = new src;` silently ran
                    // `new()` and produced a fresh object.
                    if call_args.len() == 1
                        && matches!(
                            &call_args[0].kind,
                            ExprKind::Ident(_)
                                | ExprKind::MemberAccess { .. }
                                | ExprKind::Index { .. }
                                // `new this` in a clone
                                // method copies the
                                // CURRENT object (§8.12)
                                | ExprKind::This
                        )
                        && self.expr_is_class_handle(&call_args[0])
                    {
                        let src_h = self.eval_expr(&call_args[0]).to_u64().unwrap_or(0) as usize;
                        // §8.12: `T x = new src;` shallow-copies
                        // `src` ONLY when src is of type T (or a
                        // subclass of T). A single class-handle
                        // arg of a DIFFERENT, unrelated class is
                        // a constructor ARGUMENT, not a copy
                        // source — e.g. `factory f = new(b)` where
                        // b is a base_cls must call
                        // factory::new(b), not copy-construct b.
                        // Without this type check, the loose
                        // `expr_is_class_handle` test treated any
                        // class-typed arg as a copy source,
                        // silently skipping the constructor body
                        // (so field assignments like
                        // `reg_type = t` never ran).
                        let is_copy_src = src_h != 0
                            && matches!(self.heap.get(src_h), Some(Some(src_inst))
                                if self.class_extends(&src_inst.class_name, cn));
                        if is_copy_src {
                            produced = Some(self.copy_construct(src_h));
                        }
                    }
                    if produced.is_none() {
                        // Pass the declared type's `#(...)` args
                        // (e.g. `resource#(int) r = new()`) so
                        // the instance records its type bindings
                        // (`T -> int`) — needed for per-spec static
                        // member resolution (get_type vs
                        // get_type_handle).
                        // §8.25.1: a typedef ALIAS of a
                        // specialization (`typedef
                        // Box#(byte,8) ByteBox; ByteBox b
                        // = new;`) carries no args on the
                        // declared type — pull them from
                        // the typedef's target, else the
                        // instance silently gets the
                        // class DEFAULTS.
                        let td_args: Option<Vec<Expression>> = match data_type {
                            crate::ast::types::DataType::TypeReference {
                                name, type_args, ..
                            } if type_args.is_empty() => {
                                match self.lookup_typedef_target(&name.name.name) {
                                    Some(crate::ast::types::DataType::TypeReference {
                                        type_args: t2,
                                        ..
                                    }) if !t2.is_empty() => Some(t2.clone()),
                                    _ => None,
                                }
                            }
                            _ => None,
                        };
                        let scoped_args = self.scoped_class_type_args(data_type);
                        let ta: Option<&[Expression]> = match data_type {
                            _ if scoped_args.is_some() => scoped_args.as_deref(),
                            crate::ast::types::DataType::TypeReference { type_args, .. }
                                if !type_args.is_empty() =>
                            {
                                Some(type_args.as_slice())
                            }
                            _ => td_args.as_deref(),
                        };
                        produced = Some(self.instantiate_class_with_type_args(
                            &class_def,
                            &call_args,
                            ta,
                        ));
                    }
                }
            } else if let Some(cg_def) = self.covergroup_def_for(cn) {
                // IEEE 1800-2017 §19.8: a covergroup-typed
                // LOCAL `cg c = new();` (declared inside a
                // procedural block) must allocate a real
                // covergroup instance. This branch only
                // handled classes, so the initializer fell
                // through to generic eval, `c` never got a
                // cg_heap handle, and sample() /
                // get_inst_coverage() (and the @(event)
                // clocking registration done inside
                // instantiate_covergroup) were silent
                // no-ops — coverage always read 0.
                if let Some(call_args) = is_new {
                    produced = Some(self.instantiate_covergroup(&cg_def, &call_args));
                }
            }
        }
        produced
    }

    fn store_local_decl_value(
        &mut self,
        d: &VarDeclarator,
        data_type: &DataType,
        lifetime: &Option<crate::ast::types::Lifetime>,
        plain: bool,
        static_block: bool,
        v: Value,
    ) {
        let mut unpacked_struct_local = false;
        // A static process-block local lives in the signal table even
        // under a loop's variable frame, which its block outlives.
        let in_frame = !self.local_stack.is_empty() && !static_block;
        if in_frame && d.dimensions.is_empty() && !plain {
            if let crate::ast::types::DataType::Struct(su) = self.resolve_dt(data_type) {
                if Self::spreads_member_wise(&su) {
                    unpacked_struct_local = true;
                    let base = d.name.name.clone();
                    // §6.8 defaults per leaf (a 2-state member reads
                    // 0), and packed members alias their fields.
                    let mut leaves = Vec::new();
                    self.unpacked_struct_leaf_defaults(&base, &su, 0, &mut leaves);
                    for (k, _, mdt) in &leaves {
                        self.register_packed_leaf_layout(k, mdt);
                    }
                    // The `<base>.` marker, as `unpacked_struct_leaf_keys`.
                    self.any_struct_formal_markers = true;
                    if let Some(frame) = self.local_stack.last_mut() {
                        frame.insert(format!("{}.", base), Value::new(0));
                        for (k, dv, _) in leaves {
                            frame.insert(k, dv);
                        }
                    }
                }
            }
        }
        if unpacked_struct_local {
            // An unpacked struct has no container of its own —
            // its leaves ARE the storage. Seeding one anyway
            // shadowed them: reading the bare name returned the
            // container's x instead of the packed members, so
            // `return s;` from a struct-returning function
            // yielded x even though every member was set.
            self.local_stack
                .last_mut()
                .map(|frame| frame.remove(&d.name.name));
            // Record the declared type so type-directed
            // consumers (`%s` string-member detection, `%p`)
            // can resolve `p.member` through the typedef.
            self.module
                .var_decl_types
                .insert(d.name.name.clone(), data_type.clone());
            self.note_struct_root(&d.name.name);
            self.note_struct_type(data_type);
            // §6.21: each entry gets FRESH (empty) dynamic members.
            if self.struct_colls_possible()
                && let Some(su) = self.unpacked_struct_of(data_type)
            {
                self.fresh_struct_colls(&d.name.name, &su);
            }
            // A decl-init whose SOURCE is an unpacked-struct
            // CLASS property (`pair_t p = o.orig;`, incl. the
            // ternary form) must write the leaves member-wise —
            // the whole-value `v` computed above has nothing to
            // assemble from and left every member x.
            if let Some(init_expr) = d.init.clone() {
                self.init_unpacked_struct_local(d, data_type, &init_expr);
            }
        } else if let Some(frame) = self.local_stack.last_mut().filter(|_| in_frame) {
            frame.insert(d.name.name.clone(), v);
            if d.dimensions.is_empty() && !plain && self.is_virtual_iface_type(data_type) {
                if let Some(aliases) = self.local_vif_frame_mut() {
                    aliases.bindings.insert(d.name.name.clone(), String::new());
                }
            }
        } else {
            self.signals.insert(d.name.name.clone(), v);
            // §7.2: an unpacked-struct local of an `initial`/`always`
            // block (no call frame) gets its member leaves as
            // signals, as module-scope structs do; a member write
            // through a selected member (`t.rows[0].level = 2`) found
            // no leaf to land on and vanished.
            if d.dimensions.is_empty() && !plain {
                if let crate::ast::types::DataType::Struct(su) = self.resolve_dt(data_type) {
                    if Self::spreads_member_wise(&su) {
                        self.note_struct_root(&d.name.name);
                        self.note_struct_type(data_type);
                        let mut leaves = Vec::new();
                        self.unpacked_struct_leaf_defaults(&d.name.name, &su, 0, &mut leaves);
                        for (k, dv, mdt) in leaves {
                            self.register_packed_leaf_layout(&k, &mdt);
                            self.widths.insert(k.clone(), dv.width);
                            // §6.11.3: a signed member (`int`, `byte`) reads
                            // signed, as a module-scope leaf does.
                            if dv.is_signed {
                                self.signed_signals.insert(k.clone());
                            }
                            self.signals.insert(k, dv);
                        }
                        self.register_local_struct_member_arrays(&d.name.name, &su, 0);
                        if self.struct_colls_possible() {
                            self.fresh_struct_colls(&d.name.name, &su);
                        }
                        if let Some(init_expr) = d.init.clone() {
                            self.init_unpacked_struct_local(d, data_type, &init_expr);
                        }
                    }
                }
            }
            // §6.21/§9.3.2: an explicitly `automatic` block
            // local is a fresh variable per block entry, and a
            // fork child must capture ITS entry's value. With
            // no call frame it lives in the shared signal map,
            // so record it for `inherit_fork_child_context` to
            // copy by value at spawn time — exactly like a
            // for-init loop variable. The enclosing SeqBlock
            // pops it on scope exit.
            if matches!(lifetime, Some(crate::ast::types::Lifetime::Automatic)) {
                self.auto_loop_vars.push(d.name.name.clone());
            }
        }
        // §6.8/§25.9/§7.3.2: a virtual-interface or tagged-union
        // local's initializer is the assignment `name = init` — the
        // assignment arm binds the interface instance and builds the
        // tag + member value; evaluated as a plain expression the
        // vif read x and the union matched no tag.
        let is_vif = d.dimensions.is_empty() && !plain && self.is_virtual_iface_type(data_type);
        if let Some(init_expr) = d.init.as_ref().filter(|_| !plain) {
            // Followed through the typedef table by reference: this
            // runs for every non-scalar declaration with an
            // initializer, so no resolved-type clone.
            let via_assign = is_vif
                || match self.typedef_chain_end(data_type) {
                    crate::ast::types::DataType::Interface { .. } => true,
                    crate::ast::types::DataType::Struct(su) => su.tagged,
                    _ => false,
                };
            if via_assign {
                let lvalue = crate::ast::expr::Expression::new(
                    crate::ast::expr::ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                        root: None,
                        path: vec![crate::ast::expr::HierPathSegment {
                            name: d.name.clone(),
                            selects: Vec::new(),
                        }],
                        span: d.name.span,
                        cached_signal_id: crate::ast::cache::SignalCache::new(None),
                        cached_resolved_name: crate::ast::cache::NameCache::new(),
                        bound: crate::ast::binding::Bound::UNSET,
                    }),
                    d.name.span,
                );
                let assign = crate::ast::stmt::Statement::new(
                    crate::ast::stmt::StatementKind::BlockingAssign {
                        lvalue,
                        rvalue: init_expr.as_ref().clone(),
                    },
                    d.name.span,
                );
                self.exec_statement(&assign);
            }
        }
    }

    /// Outlined from `exec_statement` so the dispatcher's stack frame
    /// stays small: this arm's locals inflated every call's frame (the
    /// dispatcher recursed with a 7.8 KB frame per level).
    #[inline(never)]
    pub(in crate::compiler::simulator) fn exec_stmt_var_decl(
        &mut self,
        stmt: &Statement,
        data_type: &DataType,
        lifetime: &Option<crate::ast::types::Lifetime>,
        declarators: &[VarDeclarator],
    ) {
        // A frame local's metadata is keyed by its bare name; another
        // process's same-named local may own those entries now.
        if !self.local_stack.is_empty() {
            for d in declarators {
                self.claim_local_meta(&d.name.name, stmt);
            }
        }
        let (resolved_dt, shape) = self.resolve_var_decl_shape(data_type);
        let data_type: &DataType = resolved_dt.as_ref();
        let VarDeclShape {
            w,
            class_ref,
            plain_class,
            plain,
            two_state,
            is_real_type,
            decl_signed,
            default_v,
        } = shape;
        // §6.21: a local of an `initial`/`always`/`final` block is static
        // unless declared `automatic` — one variable, initialized once (its
        // initializer is not re-run when the block is entered again). Such a
        // block has no call frame; a subroutine or class method has one.
        let static_block = !matches!(lifetime, Some(crate::ast::types::Lifetime::Automatic))
            && !self.in_const_param_eval
            && self.current_static_task.is_none()
            && self.current_auto_task.is_none()
            && !matches!(self.class_context_stack.last(), Some(Some(_)))
            && self.in_process_block_scope();
        // An initializer without an explicit `static` is an error (the
        // variable is implicitly static, §6.21); such a declaration keeps
        // running each time, as an automatic one would.
        let explicit_static = matches!(lifetime, Some(crate::ast::types::Lifetime::Static));
        for d in declarators {
            // Set below for an unpacked collection; never carried over.
            self.pending_local_shape = None;
            if explicit_static
                && !self.static_local_syncs.is_empty()
                && self.hoisted_sub_static_names.contains(&d.name.name)
            {
                continue;
            }
            if static_block && (explicit_static || d.init.is_none()) {
                // Done already? (Allocation-free once it is: the activation
                // after the first.)
                let names = self
                    .static_block_decls
                    .entry((d.name.span.start, d.name.span.end))
                    .or_default();
                if names.iter().any(|n| *n == d.name.name) {
                    continue;
                }
                names.push(d.name.name.clone());
            }
            // Every metadata key this declaration writes is its name or
            // `name.member` (or a `@` per-call key, never a formal's name).
            self.meta_note(&d.name.name);
            // A fresh local declaration shadows any prior global-scoped
            // collection registration left over from another frame's
            // same-named local (subroutine locals use bare names). Reset
            // the array/queue/assoc bookkeeping so e.g. a scalar `string
            // str` after a previous `string str[$]` isn't still treated
            // as a queue.
            self.clear_shadowed_collection_meta(&d.name.name, data_type);
            // Register a *packed*-struct local's field layout so member
            // access (`p.field`) aliases into the whole variable, as
            // module-level packed-struct signals already do. Without
            // this a local `pkt_t p;` keeps `p` and `p.field` in
            // separate storage (a whole write doesn't reach members and
            // vice-versa). Unpacked structs are intentionally excluded.
            // §6.17: an `event` local becomes a real event —
            // `->e` / `@(e)` resolve through module.events. Arrays
            // and queues of events register their BASE name so
            // element refs (`ev_arr[i]`) are recognized as events.
            if matches!(
                data_type,
                crate::ast::types::DataType::Simple {
                    kind: crate::ast::types::SimpleType::Event,
                    ..
                }
            ) {
                self.module.events.insert(d.name.name.clone());
            }
            // Declared ELEMENT type of a local. `var_decl_types` holds
            // the element type for a collection, so this is the same
            // `data_type` either way — the unpacked dims live on the
            // declarator. Registering it only for SCALAR locals meant a
            // local QUEUE OF STRUCTS had no element type, so
            // `queue_elem_struct` returned None and `push_back` fell
            // back to a packed scalar copy: an unpacked struct has no
            // container signal, so every member was lost and the queue
            // yielded zeros. Module-scope queues were unaffected because
            // elaboration registers them. (A recurring name keeps its key:
            // only the value is replaced.)
            match self.module.var_decl_types.get_mut(d.name.name.as_str()) {
                Some(slot) => *slot = data_type.clone(),
                None => {
                    self.module
                        .var_decl_types
                        .insert(d.name.name.clone(), data_type.clone());
                }
            }
            // §7.4.1: a packed-of-packed local (`u8_vec16_t y;`)
            // needs its ELEMENT width registered, or `y[i]` reads
            // and writes degrade to single-BIT selects. This was the
            // long-standing AST bug behind self-checking TBs whose
            // reference functions silently agreed with equally-broken
            // DUT paths (pixel-conversion battery).
            if d.dimensions.is_empty() && !plain {
                if let Some(ew) = self.ast_decl_elem_width(data_type) {
                    if ew > 0 && w > ew && w % ew == 0 {
                        self.module
                            .packed_signal_elem_widths
                            .insert(d.name.name.clone(), ew);
                    }
                }
            }
            if d.dimensions.is_empty() && plain {
                // No element width (removed above) and no struct or enum
                // layout; only a vector's own non-normalized range is kept.
                match super::elaborate::packed_full_dims_of(data_type, &self.module.parameters) {
                    Some(fdims) => {
                        self.module
                            .packed_full_dims
                            .insert(d.name.name.clone(), fdims);
                    }
                    None => {
                        self.module.packed_full_dims.remove(&d.name.name);
                    }
                }
            } else if d.dimensions.is_empty() {
                // Packed multi-D local (`logic [1:0][3:0] m;`): record
                // the per-element width so `m[i]` is an element slice,
                // not a bit-select — module-scope decls already do.
                // The packed dims may live in the TYPEDEF rather than on
                // the declaration (`typedef T1 [3:0] T2; T2 v;`) — see
                // `typedef_one_step`. Without this the element width was
                // never recorded and `v[i]` degraded to a BIT select.
                let dt1_local = self.typedef_one_step(data_type);
                if let Some(elem_w) = super::elaborate::packed_inner_elem_width(
                    data_type,
                    &self.module.parameters,
                    &self.module.typedefs,
                )
                .or_else(|| {
                    super::elaborate::packed_inner_elem_width(
                        &dt1_local,
                        &self.module.parameters,
                        &self.module.typedefs,
                    )
                }) {
                    self.module
                        .packed_signal_elem_widths
                        .insert(d.name.name.clone(), elem_w);
                } else {
                    self.module.packed_signal_elem_widths.remove(&d.name.name);
                }
                if let Some(fdims) =
                    super::elaborate::packed_full_dims_of(data_type, &self.module.parameters)
                        .or_else(|| {
                            super::elaborate::packed_full_dims_of(
                                &dt1_local,
                                &self.module.parameters,
                            )
                        })
                        .or_else(|| {
                            super::elaborate::packed_full_dims_chained(
                                data_type,
                                &self.module.parameters,
                                &self.module.typedef_types,
                            )
                        })
                        .or_else(|| self.struct_array_full_dims(data_type))
                {
                    self.module
                        .packed_full_dims
                        .insert(d.name.name.clone(), fdims);
                } else {
                    self.module.packed_full_dims.remove(&d.name.name);
                }
                // Per-field element widths (`u.n[i]` where the member
                // is itself a packed array) — mirrors the module-scope
                // registration keyed "var.field". A member whose own type
                // is a struct carries aggregate members of its own:
                // `u.inner.arr[i]` needs `inner.arr`'s element stride to
                // place the trailing select, so the walk recurses (bounded)
                // exactly like `register_struct_member_packed_dims` does
                // for signals. Depth-1-only registration left that stride
                // unknown and the depth-2 select degraded to a 1-BIT read.
                if let crate::ast::types::DataType::Struct(su) = self.resolve_dt(data_type) {
                    let mut pending: Vec<(String, crate::ast::types::StructUnionType)> =
                        vec![(d.name.name.clone(), su)];
                    let mut budget = 4096usize;
                    while let Some((prefix, su)) = pending.pop() {
                        if budget == 0 {
                            break;
                        }
                        budget -= 1;
                        for m in &su.members {
                            let (ew, fd) = super::elaborate::packed_member_meta(
                                &m.data_type,
                                &self.module.parameters,
                                &self.module.typedefs,
                                &self.module.typedef_types,
                            );
                            for mdecl in &m.declarators {
                                let key = format!("{}.{}", prefix, mdecl.name.name);
                                // The member keys are formal metadata too
                                // (see `meta_note`).
                                self.meta_note(&key);
                                // Re-registration REPLACES: a later local
                                // reusing the same `var.field` name must not
                                // keep the earlier declaration's geometry
                                // (two block-local structs whose same-named
                                // member has different packed dims).
                                if let Some(ew) = ew {
                                    self.note_elem_width_key(&key);
                                    self.module
                                        .packed_signal_elem_widths
                                        .insert(key.clone(), ew);
                                } else {
                                    self.module.packed_signal_elem_widths.remove(&key);
                                }
                                if let Some(fd) = &fd {
                                    self.module.packed_full_dims.insert(key.clone(), fd.clone());
                                } else {
                                    self.module.packed_full_dims.remove(&key);
                                }
                                if let crate::ast::types::DataType::Struct(nested) =
                                    self.resolve_dt(&m.data_type)
                                {
                                    pending.push((key, nested));
                                }
                            }
                        }
                    }
                }
                // §6.19.6: an inline-enum LOCAL (a typedef'd enum
                // local arrives here already folded to its enum
                // type) keys its member list by its own name.
                if matches!(data_type, crate::ast::types::DataType::Enum(_)) {
                    if let Some(m) = super::elaborate::anon_enum_members_ordered(
                        data_type,
                        &self.module.parameters,
                    ) {
                        // §6.19: its member names are constants of the block.
                        let base_w = super::elaborate::resolve_type_width(
                            data_type,
                            Some(&self.module.parameters),
                            Some(&self.module.typedefs),
                        )
                        .max(1);
                        for (n, v) in &m {
                            self.declare_enum_constant(n, Value::from_u64(*v, base_w));
                        }
                        self.type_tables_gen += 1;
                        self.module.enum_members.insert(d.name.name.clone(), m);
                        if let Some(set) = self.plain_class_types.borrow_mut().as_mut() {
                            set.remove(&d.name.name);
                        }
                    }
                }
                if let Some(fields) = super::elaborate::packed_struct_field_layout(
                    data_type,
                    &self.module.parameters,
                    &self.module.typedefs,
                    &self.module.typedef_types,
                ) {
                    if !fields.is_empty() {
                        if env_set_cached!("XEZIM_PSDBG") {
                            eprintln!("[PSDBG] register packed local {}", d.name.name);
                        }
                        self.module
                            .packed_struct_fields
                            .insert(d.name.name.clone(), fields);
                    }
                }
                // Declaration-order member names for `%p` (§21.2.1.7).
                if let Some(names) =
                    super::elaborate::struct_member_names(data_type, &self.module.typedef_types)
                {
                    if !names.is_empty() {
                        self.module
                            .struct_members
                            .insert(d.name.name.clone(), names);
                    }
                }
            } else {
                // §7.2.1/§7.4: an ARRAY local of a packed struct element
                // (`s_t a [];` in a task or function) gets the element's
                // field layout, as a module-scope array does in
                // elaboration: `a[i].f` slices the element. Without it a
                // field write was dropped and a field read gave 0.
                self.local_elem_layout = None;
                if let Some(fields) = super::elaborate::packed_struct_field_layout(
                    data_type,
                    &self.module.parameters,
                    &self.module.typedefs,
                    &self.module.typedef_types,
                )
                .filter(|f| !f.is_empty())
                {
                    let prev = self
                        .module
                        .packed_struct_fields
                        .insert(d.name.name.clone(), fields.clone());
                    // An array local to an automatic task lives under a
                    // per-call key (`declare_local_dyn`).
                    self.local_elem_layout = Some((fields, d.name.name.clone(), prev));
                }
                // §7.4/§20.7: an ARRAY local of a packed multi-dimensional
                // element (`logic [7:0][3:0] pk [2][3];`) keeps its packed
                // geometry, as a module-scope array does: the element width
                // makes `pk[i][j][k]` a lane select and the full dims answer
                // `$size(pk, 3)` / `$dimensions(pk)`. Only the flat element
                // width was kept, so the packed dims read as one 32-bit one.
                match super::elaborate::packed_full_dims_of(data_type, &self.module.parameters)
                    .or_else(|| {
                        super::elaborate::packed_full_dims_chained(
                            data_type,
                            &self.module.parameters,
                            &self.module.typedef_types,
                        )
                    }) {
                    Some(fdims) if fdims.len() >= 2 => {
                        if let Some(ew) = super::elaborate::packed_inner_elem_width(
                            data_type,
                            &self.module.parameters,
                            &self.module.typedefs,
                        )
                        .or_else(|| self.ast_decl_elem_width(data_type))
                        {
                            self.module
                                .packed_signal_elem_widths
                                .insert(d.name.name.clone(), ew);
                        }
                        self.module
                            .packed_full_dims
                            .insert(d.name.name.clone(), fdims);
                        self.array_local_packed_dims.insert(d.name.name.clone());
                    }
                    // Only a name this arm registered can hold a stale
                    // geometry: the common array local pays no lookup.
                    _ if !self.array_local_packed_dims.is_empty()
                        && self.array_local_packed_dims.remove(&d.name.name) =>
                    {
                        self.module.packed_full_dims.remove(&d.name.name);
                    }
                    _ => {}
                }
            }
            // §6.18/§6.20.3: a typedef'd (or type-param-bound) local
            // carries its unpacked dims on the TYPE — `my_array_t a;`
            // declared inside a task elaborated as a scalar.
            let typedef_dims: Vec<crate::ast::types::UnpackedDimension> = if !plain_class {
                if let crate::ast::types::DataType::TypeReference { name, .. } = data_type {
                    let tn = &name.name.name;
                    let concrete = self
                        .resolve_type_param_binding(tn)
                        .unwrap_or_else(|| tn.clone());
                    self.module
                        .typedef_unpacked_dims
                        .get(&concrete)
                        .cloned()
                        .or_else(|| {
                            // CLASS-LOCAL typedef: an explicit
                            // `C::t_t v;` scope, else the class
                            // whose method is executing — a
                            // `Ph::d_t dd;` local bound as a
                            // scalar, so a later `ref` call saw
                            // no collection to write back to.
                            let start = name
                                .scopes
                                .first()
                                .map(|s| s.name.name.clone())
                                .or_else(|| self.class_context_stack.last().cloned().flatten())
                                .or_else(|| {
                                    self.this_stack.last().copied().flatten().and_then(|h| {
                                        self.heap
                                            .get(h)
                                            .and_then(|x| x.as_ref())
                                            .map(|i| i.class_name.clone())
                                    })
                                })?;
                            Self::typedef_dims_via_tables(&self.module, &start, &concrete)
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            // §7.4.5: a local with dimensions of its own (`q_t l[$];`) puts
            // them first, then the typedef's, as at module scope; the
            // typedef's were dropped, so each row of `l` held one scalar.
            let joined_dims: Vec<crate::ast::types::UnpackedDimension>;
            let dims: &Vec<crate::ast::types::UnpackedDimension> = if typedef_dims.is_empty() {
                &d.dimensions
            } else if d.dimensions.is_empty() {
                &typedef_dims
            } else {
                joined_dims = d.dimensions.iter().chain(&typedef_dims).cloned().collect();
                &joined_dims
            };
            // §7.4.2 / §8.23: a dimension sized by a constant of the
            // enclosing scope (`l[N]`, `l[N * 2]`, `l[cls::N]`) — the
            // context-free evaluation below cannot resolve it.
            let scope_sized: Option<Vec<crate::ast::types::UnpackedDimension>> =
                if dims.iter().any(Self::local_dim_needs_scope) {
                    self.scope_sized_local_dims(dims)
                } else {
                    None
                };
            let dims = scope_sized.as_ref().unwrap_or(dims);
            // §7.4.5: its declared shape, for a per-call storage key. A
            // block-scope local keeps its bare name, recorded at elaboration.
            // Fixed bounds fold from the literals `scope_sized` left.
            self.pending_local_shape = CollShape::of(dims, None, |kdt| {
                super::elaborate::coll_key_kind(
                    kdt,
                    Some(&self.module.parameters),
                    Some(&self.module.typedefs),
                    Some(&self.module.typedef_types),
                )
            });
            // §7.4.5 / §7.8: a local unpacked array whose ELEMENT is a
            // collection (`int q[3][$]`, `int v[2][string][string]`), as a
            // module-scope one is registered in elaboration: the fixed outer
            // shape, and every element its own queue / associative array.
            // Without it the trailing dimensions were dropped, so
            // `v[0]["A"] = 7` and `q[1].push_back(4)` were lost.
            if let Some(k) = super::elaborate::coll_elem_split(dims)
                && self.declare_local_coll_array(d, data_type, dims, k, w, lifetime.clone())
            {
                continue;
            }
            // A local MULTI-dimensional array was registered from its
            // first dimension only, so `int n[2][2]` became a 1-D array
            // of double-width elements: `n[1][0]` read 0 and the
            // declaration initializer could not be spread.
            if dims.len() >= 2 {
                use crate::ast::types::UnpackedDimension as UD;
                let shape: Option<Vec<(i64, i64)>> = dims
                    .iter()
                    .map(|dim| match dim {
                        UD::Range { left, right, .. } => {
                            let l = super::elaborate::const_eval_i64_with_params(left, None)?;
                            let r = super::elaborate::const_eval_i64_with_params(right, None)?;
                            Some((l.min(r), l.max(r)))
                        }
                        UD::Expression { expr, .. } => {
                            let n = super::elaborate::const_eval_i64_with_params(expr, None)?;
                            if n > 0 { Some((0, n - 1)) } else { None }
                        }
                        _ => None,
                    })
                    .collect();
                if let Some(shape) = shape.filter(|sh| {
                    sh.iter()
                        .map(|&(lo, hi)| (hi - lo + 1).max(0))
                        .product::<i64>()
                        <= (1 << 20)
                }) {
                    // §6.21: automatic like the 1-D fixed local below — a
                    // per-invocation key in a class method or automatic task.
                    let bare = d.name.name.clone();
                    let in_method = matches!(self.class_context_stack.last(), Some(Some(_)));
                    let name = if (in_method || self.current_auto_task.is_some())
                        && self.current_static_task.is_none()
                        && !matches!(lifetime, Some(crate::ast::types::Lifetime::Static))
                    {
                        self.declare_local_dyn(&bare)
                    } else {
                        bare
                    };
                    self.adopt_local_shape(&name);
                    if let Some(bounds) = self.declared_fixed_dims(dims) {
                        self.module.unpacked_decl_dims.insert(name.clone(), bounds);
                    }
                    match shape.len() {
                        2 => {
                            self.module
                                .arrays_2d
                                .insert(name.clone(), (shape[0], shape[1], w));
                        }
                        _ => {
                            self.module
                                .arrays_nd
                                .insert(name.clone(), (shape.clone(), w));
                        }
                    }
                    // Every element, row-major.
                    let mut suffixes: Vec<String> = vec![String::new()];
                    for &(lo, hi) in &shape {
                        let mut next = Vec::new();
                        for pre in &suffixes {
                            for i in lo..=hi {
                                next.push(format!("{}[{}]", pre, i));
                            }
                        }
                        suffixes = next;
                    }
                    let is_string_elem = matches!(
                        data_type,
                        crate::ast::types::DataType::Simple {
                            kind: crate::ast::types::SimpleType::String,
                            ..
                        }
                    );
                    if is_string_elem {
                        self.string_signals.insert(name.clone());
                    }
                    for sfx in suffixes {
                        let elem = format!("{}{}", name, sfx);
                        let seed = if is_string_elem {
                            Value::from_string("")
                        } else {
                            default_v.clone()
                        };
                        self.signals.insert(elem.clone(), seed);
                        self.widths.insert(elem, w);
                    }
                    self.widths.insert(name.clone(), w);
                    self.module
                        .var_decl_types
                        .insert(name.clone(), data_type.clone());
                    if let Some(init) = &d.init {
                        self.route_decl_init_via_assign(&name, init, d.name.span);
                    }
                    continue;
                }
            }
            let mut range: Option<(i64, i64)> = None;
            let mut descending = false;
            if let Some(first) = dims.first() {
                use crate::ast::types::UnpackedDimension;
                match first {
                    UnpackedDimension::Range { left, right, .. } => {
                        let l =
                            super::elaborate::const_eval_i64_with_params(left, None).unwrap_or(0);
                        let r =
                            super::elaborate::const_eval_i64_with_params(right, None).unwrap_or(0);
                        range = Some((l.min(r), l.max(r)));
                        if l > r {
                            descending = true;
                        }
                    }
                    UnpackedDimension::Expression { expr, .. } => {
                        let n =
                            super::elaborate::const_eval_i64_with_params(expr, None).unwrap_or(0);
                        if n > 0 {
                            range = Some((0, n - 1));
                        }
                    }
                    UnpackedDimension::Unsized(_) | UnpackedDimension::Queue { .. } => {
                        // Register as dynamic array / queue (initially empty).
                        // §6.21: in a class method it is automatic — each
                        // invocation owns its storage, under the per-frame
                        // key the associative and fixed-size locals use.
                        // Keyed by its bare name, two concurrent calls
                        // shared one queue and each declaration emptied the
                        // other's.
                        let bare = d.name.name.clone();
                        let in_method = matches!(self.class_context_stack.last(), Some(Some(_)));
                        let name = if (in_method || self.current_auto_task.is_some())
                            && self.current_static_task.is_none()
                            && !matches!(lifetime, Some(crate::ast::types::Lifetime::Static))
                        {
                            self.declare_local_dyn(&bare)
                        } else {
                            bare.clone()
                        };
                        self.adopt_local_shape(&name);
                        self.module.arrays.insert(name.clone(), (0, -1, w));
                        self.module.dynamic_arrays.insert(name.clone());
                        if name != bare {
                            // The element type, under the per-call key too
                            // (a queue of structs spreads its members by it).
                            self.module
                                .var_decl_types
                                .insert(name.clone(), data_type.clone());
                            if let crate::ast::types::DataType::TypeReference { name: tn, .. } =
                                data_type
                            {
                                if self.module.classes.contains_key(&tn.name.name) {
                                    self.module
                                        .array_elem_class
                                        .insert(name.clone(), tn.name.name.clone());
                                }
                            }
                        }
                        // A QUEUE-typed local (`int a[$];`, possibly with
                        // a bound `[$:N]`) keeps the queue semantics -
                        // index-at-size appends, size tracking on
                        // `q[i] = v` back-fill - not the discard-out-of-
                        // range rule of an unallocated dynamic array.
                        // Mirror the module-scope classification.
                        if matches!(first, UnpackedDimension::Queue { .. }) {
                            self.module.queue_vars.insert(name.clone());
                        }
                        self.widths.insert(name.clone(), w);
                        self.set_queue_size(&name, 0);
                        // §7.4.5: a LOCAL `T a[][16]` / `a[$][16]` —
                        // dynamic outer with fixed trailing dims —
                        // registers the 2-D/N-D shape like a module-
                        // scope one, else the trailing dims are
                        // dropped and `a[i][j]` misaddresses.
                        if dims.len() >= 2
                            && dims[1..].iter().all(|dm| {
                                matches!(
                                    dm,
                                    UnpackedDimension::Range { .. }
                                        | UnpackedDimension::Expression { .. }
                                )
                            })
                        {
                            let inner: Option<Vec<(i64, i64)>> = dims[1..]
                                .iter()
                                .map(|dm| {
                                    super::elaborate::extract_array_range(
                                        std::slice::from_ref(dm),
                                        &self.module.parameters,
                                    )
                                })
                                .collect();
                            if let Some(inner) = inner
                                .filter(|sh| sh.iter().all(|&(lo, hi)| hi >= lo && hi - lo < 4096))
                            {
                                self.module.arrays.remove(&name);
                                if inner.len() == 1 {
                                    self.module
                                        .arrays_2d
                                        .insert(name.clone(), ((0, 63), inner[0], w));
                                } else {
                                    let mut shape = vec![(0i64, 63i64)];
                                    shape.extend(inner.iter().cloned());
                                    self.module.arrays_nd.insert(name.clone(), (shape, w));
                                }
                                if let crate::ast::types::DataType::TypeReference {
                                    name: tn, ..
                                } = data_type
                                {
                                    self.module
                                        .array_elem_class
                                        .insert(name.clone(), tn.name.name.clone());
                                }
                            }
                        }
                        // A `string q[$]` queue: mark the name so its
                        // elements are treated as strings (`%p`, `%s`,
                        // concat) — the element-string check keys off
                        // the queue name being in `string_signals`. A
                        // non-string decl must UNmark: the set is keyed
                        // by bare name, so a stale entry from another
                        // frame's same-named `string q[$]` would give
                        // this queue string treatment.
                        if matches!(
                            data_type,
                            crate::ast::types::DataType::Simple {
                                kind: crate::ast::types::SimpleType::String,
                                ..
                            }
                        ) {
                            self.string_signals.insert(name.clone());
                        } else {
                            self.forget_string_flag(&name);
                        }
                        // LRM §7.10 / §7.12.1 — if an initializer
                        // is provided (e.g. `int idx[$] =
                        // q.find_first_index with (…);`),
                        // synthesise a BlockingAssign so the queue
                        // is populated via the same path as a
                        // standalone assignment. Without this the
                        // init is silently dropped and the queue
                        // stays empty.
                        if let Some(init) = &d.init {
                            self.route_decl_init_via_assign(&bare, init, d.name.span);
                        }
                        continue;
                    }
                    UnpackedDimension::Associative {
                        data_type: key_dt, ..
                    } => {
                        // Function-local associative array (`int m[int]`,
                        // `int m[string]`). Register so indexed writes/reads
                        // resolve to the signal-keyed assoc storage instead
                        // of being mistaken for a scalar.
                        let bare = d.name.name.clone();
                        let name = if matches!(lifetime, Some(crate::ast::types::Lifetime::Static))
                        {
                            bare.clone()
                        } else {
                            self.declare_local_dyn(&bare)
                        };
                        self.adopt_local_shape(&name);
                        let is_string_key = key_dt.as_ref().is_some_and(|dt| {
                            matches!(
                                dt.as_ref(),
                                crate::ast::types::DataType::Simple {
                                    kind: crate::ast::types::SimpleType::String,
                                    ..
                                }
                            )
                        });
                        // Drop any stale elements left by a prior frame's
                        // same-named local before this fresh declaration —
                        // struct-element member leaves (`m[k].x`) included.
                        let pre = format!("{}[", name);
                        let stale: Vec<String> = self.signals.keys_with_elem_prefix(&pre);
                        for k in stale {
                            self.signals.remove(&k);
                        }
                        self.module
                            .associative_arrays
                            .insert(name.clone(), is_string_key);
                        self.widths.insert(name.clone(), w);
                        // §7.8.2 / §7.9: the key's width and signedness, as
                        // elaboration records them for a module-scope array.
                        // Without them a `foreach` over a local `int a[int]`
                        // bound the negative key -5 as 4294967291, and the
                        // element read through it missed.
                        // §8.25: a key sized by a class parameter
                        // (`bit seen[bit [N-1:0]]` in a method of `C #(N)`)
                        // takes the executing specialization's width (#681).
                        let class_key = key_dt.as_deref().and_then(|k| {
                            let w = self.class_param_sized_width(k)?;
                            let signed = matches!(
                                k,
                                crate::ast::types::DataType::IntegerVector {
                                    signing: Some(crate::ast::types::Signing::Signed),
                                    ..
                                }
                            );
                            Some((w, signed))
                        });
                        match class_key.or_else(|| {
                            super::elaborate::assoc_key_type_info(
                                key_dt.as_deref(),
                                &self.module.parameters,
                                &self.module.typedefs,
                                &self.module.typedef_types,
                            )
                            .0
                        }) {
                            Some(iw) => {
                                self.module.assoc_index_widths.insert(name.clone(), iw);
                            }
                            None => {
                                self.module.assoc_index_widths.remove(&name);
                            }
                        }
                        // §6.19.6/§7.8: a NAMED key type (`int aa[e_t];`)
                        // gives a `foreach (aa[k])` index variable that type,
                        // so `k.last`/`k.next`/`k.name` resolve against the
                        // enum. Elaboration records it for module-scope
                        // arrays only. Inside a subroutine frame it goes to
                        // the frame's overlay under `aa[]` (no variable can
                        // have that name), so a same-named array elsewhere
                        // keeps its own key type; an unnamed key type records
                        // "" there to shadow an outer entry.
                        let key_tn = key_dt
                            .as_ref()
                            .and_then(|dt| match dt.as_ref() {
                                crate::ast::types::DataType::TypeReference { name: tn, .. } => {
                                    Some(tn.name.name.clone())
                                }
                                _ => None,
                            })
                            .filter(|tn| {
                                self.module.enum_members.contains_key(tn)
                                    || self.module.typedef_types.contains_key(tn)
                            });
                        if self.local_type_stack.is_empty() {
                            match key_tn {
                                Some(tn) => {
                                    self.module.assoc_key_type_names.insert(bare.clone(), tn);
                                }
                                None => {
                                    self.module.assoc_key_type_names.remove(&bare);
                                }
                            }
                        } else {
                            let slot = format!("{}[]", bare);
                            self.record_local_typedef_type(&slot, key_tn.as_deref().unwrap_or(""));
                        }
                        // Record the ELEMENT type under the renamed
                        // storage key so `all[k] = new` can resolve
                        // the element class (incl. parameterized
                        // typedefs — UVM's sort_by_precedence local
                        // `rsrc_q_t all[int]`; issue #113).
                        self.module
                            .var_decl_types
                            .insert(name.clone(), data_type.clone());
                        if let crate::ast::types::DataType::TypeReference { name: tn, .. } =
                            data_type
                        {
                            self.module
                                .array_elem_class
                                .entry(name.clone())
                                .or_insert_with(|| tn.name.name.clone());
                        }
                        // A `string m[...]` (string-VALUED assoc): mark the
                        // name so its elements render as quoted strings under
                        // `%p` (the element-string check keys off the array
                        // name being in `string_signals`). This mirrors the
                        // string-queue marking above.
                        if matches!(
                            data_type,
                            crate::ast::types::DataType::Simple {
                                kind: crate::ast::types::SimpleType::String,
                                ..
                            }
                        ) {
                            self.string_signals.insert(name.clone());
                        } else {
                            self.forget_string_flag(&name);
                        }
                        // If the element type is a known class, record it
                        // so `assoc[k] = new()` constructs the right class
                        // (ranged arrays get this at elaborate time; assoc
                        // arrays were missing it).
                        if let crate::ast::types::DataType::TypeReference { name: en, .. } =
                            data_type
                        {
                            let ecn = en.name.name.clone();
                            if self.module.classes.contains_key(&ecn) {
                                self.module.array_elem_class.insert(name.clone(), ecn);
                            }
                        }
                        // A local ASSOCIATIVE array's declaration
                        // initializer (`uvm_object_wrapper m[string] =
                        // '{"a": w, default: d};` inside a method) was
                        // silently dropped — the branch registered the
                        // array then `continue`d, so every element read
                        // back null/X even for EXPLICIT keys. Route it
                        // through a BlockingAssign (as the queue and
                        // fixed-array branches do) so the pattern-
                        // aggregate spread applies write the keyed
                        // elements and record the default fallback.
                        if let Some(init) = &d.init {
                            self.route_decl_init_via_assign(&bare, init, d.name.span);
                        }
                        continue;
                    }
                }
            }
            if let Some((lo, hi)) = range {
                // §6.21: a fixed array local to a class method is automatic —
                // each invocation owns its storage. Keyed by its bare name it
                // was one array shared by every concurrent call (two
                // sequences' `item req_c[10]` overwrote each other) and
                // re-seeded under the other's feet. Give it the per-frame
                // key the associative locals already use.
                let bare = d.name.name.clone();
                let in_method = matches!(self.class_context_stack.last(), Some(Some(_)));
                let name = if (in_method || self.current_auto_task.is_some())
                    && self.current_static_task.is_none()
                    && !matches!(lifetime, Some(crate::ast::types::Lifetime::Static))
                {
                    self.declare_local_dyn(&bare)
                } else {
                    bare.clone()
                };
                self.adopt_local_shape(&name);
                if name == bare
                    && let Some(frame) = self.local_dyn.last_mut()
                    && !frame.iter().any(|(local, _)| local == &bare)
                {
                    frame.push((bare.clone(), name.clone()));
                }
                // §7.4.4 (#519): a fixed MULTI-dimensional local keeps its
                // full shape — it was registered from its first dimension
                // as a 1-D array, and the elaborated shape of a same-named
                // local of another subroutine stayed in the other table, so
                // a later `std::randomize` read whichever shape it met first.
                // Each declaration now owns the name in exactly one table.
                let nd_shape: Option<Vec<(i64, i64)>> = if dims.len() >= 2 {
                    dims.iter()
                        .map(|dm| {
                            super::elaborate::extract_array_range(
                                std::slice::from_ref(dm),
                                &self.module.parameters,
                            )
                        })
                        .collect::<Option<Vec<_>>>()
                        .filter(|sh| sh.iter().all(|&(l, h)| h >= l && h - l < 4096))
                } else {
                    None
                };
                if let Some(shape) = nd_shape {
                    if let Some(bounds) = self.declared_fixed_dims(&dims) {
                        self.module.unpacked_decl_dims.insert(name.clone(), bounds);
                    }
                    self.module.arrays.remove(&name);
                    if shape.len() == 2 {
                        self.module
                            .arrays_2d
                            .insert(name.clone(), (shape[0], shape[1], w));
                        self.module.arrays_nd.remove(&name);
                    } else {
                        self.module.arrays_nd.insert(name.clone(), (shape, w));
                        self.module.arrays_2d.remove(&name);
                    }
                    continue;
                }
                self.module.arrays.insert(name.clone(), (lo, hi, w));
                self.module.arrays_2d.remove(&name);
                self.module.arrays_nd.remove(&name);
                if descending {
                    self.module.descending_arrays.insert(name.clone());
                }
                // A local FIXED array of STRINGS: mark the array name
                // (the element-string check keys off it, like the
                // string-queue branch) and seed every element as the
                // EMPTY string. Seeded as zero bits, an unset element
                // rendered under `%s` as a run of blanks, and the
                // declaration's `'{…}` pattern had no string-typed
                // element to land in — `string nm[3] = '{"a","b","c"}`
                // in a block printed nothing but padding.
                let is_string_elem = matches!(
                    data_type,
                    crate::ast::types::DataType::Simple {
                        kind: crate::ast::types::SimpleType::String,
                        ..
                    }
                );
                if is_string_elem {
                    self.string_signals.insert(name.clone());
                } else {
                    self.forget_string_flag(&name);
                }
                // §7.2/§7.4: an element of a local array of UNPACKED structs
                // is stored as its member leaves, as a module-scope one is —
                // every element of every member dimension (`a[1].mm[i][j]`).
                // A packed container per element had no leaf for a member
                // element select to land on.
                let elem_su = self.unpacked_struct_of(data_type);
                if elem_su.is_some() {
                    self.note_struct_root(&bare);
                    self.note_struct_root(&name);
                    self.note_struct_type(data_type);
                }
                for idx in lo..=hi {
                    let elem = format!("{}[{}]", name, idx);
                    if let Some(su) = &elem_su {
                        let mut leaves = Vec::new();
                        self.unpacked_struct_leaf_defaults(&elem, su, 0, &mut leaves);
                        for (k, dv, mdt) in leaves {
                            self.register_packed_leaf_layout(&k, &mdt);
                            self.widths.insert(k.clone(), dv.width);
                            // §6.11.3: a signed member (`int`, `byte`) reads
                            // signed, as a module-scope leaf does.
                            if dv.is_signed {
                                self.signed_signals.insert(k.clone());
                            }
                            self.signals.insert(k, dv);
                        }
                        continue;
                    }
                    let seed = if is_string_elem {
                        Value::from_string("")
                    } else {
                        default_v.clone()
                    };
                    self.signals.insert(elem.clone(), seed);
                    self.widths.insert(elem, w);
                }
                self.widths.insert(name.clone(), w);
                // Element type of a LOCAL array: the pattern spread and
                // the type-directed `%p` renderer both need it, and only
                // module-scope declarations recorded one.
                self.module
                    .var_decl_types
                    .insert(name.clone(), data_type.clone());
                // A local FIXED array's declaration initializer was
                // dropped, so `int a[4] = '{10,20,30,40};` inside a block
                // read back all zeros. Route it through an assignment,
                // as the queue branch above already does — the pattern
                // spread then applies every §10.9.1 form.
                if let Some(init) = &d.init {
                    self.route_decl_init_via_assign(&bare, init, d.name.span);
                }
            } else {
                if !matches!(lifetime, Some(crate::ast::types::Lifetime::Automatic))
                    && let Some(task_name) = self.current_static_task.clone()
                {
                    let key = format!("{}.{}", task_name, d.name.name);
                    if !self.static_task_init.insert(key) {
                        continue;
                    }
                }
                let class_name =
                    if let (true, crate::ast::types::DataType::TypeReference { name, .. }) =
                        (plain_class, data_type)
                    {
                        Some(name.name.name.clone())
                    } else if let crate::ast::types::DataType::TypeReference { name, .. } =
                        data_type
                    {
                        // §8.25: the declared type may be a TYPE
                        // PARAMETER of the enclosing parameterized class
                        // (e.g. `T obj = new(...)` inside
                        // `class C #(type T=int);`). Resolve it through
                        // the active specialization FIRST so the
                        // constructor form constructs the bound class,
                        // not a broken instance of the literal "T".
                        if let Some(bound) = self.resolve_type_param_binding(&name.name.name) {
                            Some(bound)
                        } else {
                            // Resolve a typedef/scoped alias (e.g.
                            // `uvm_resource_types::rsrc_q_t`) to its
                            // concrete class so `name = new()`
                            // constructs it; fall back to the bare
                            // name otherwise.
                            Some(
                                self.resolve_typeref_class_name(name)
                                    .unwrap_or_else(|| name.name.name.clone()),
                            )
                        }
                    } else {
                        None
                    };
                // §6.8/§7.2: an unpacked-struct local is initialized member-
                // wise below (`init_unpacked_struct_local`), which runs the
                // initializer itself. Evaluating it here too ran it twice:
                // `receipt_t r = q.pop_front();` popped two elements.
                let struct_init_later = d.init.is_some()
                    && d.dimensions.is_empty()
                    && !plain
                    && matches!(
                        self.resolve_dt(data_type),
                        crate::ast::types::DataType::Struct(su) if Self::spreads_member_wise(&su)
                    );
                let vif_init_later = d.init.is_some()
                    && d.dimensions.is_empty()
                    && !plain
                    && self.is_virtual_iface_type(data_type);
                let eff_w = w;
                let v = if let Some(init_expr) =
                    d.init.as_ref().filter(|_| !struct_init_later && !vif_init_later)
                {
                    let produced = self.init_class_or_covergroup_local(
                        data_type,
                        class_name.as_deref(),
                        init_expr,
                    );
                    match produced {
                        Some(p) => p,
                        None => {
                            // §10.8: a pattern takes the declared type.
                            let typed = if d.dimensions.is_empty() {
                                self.eval_in_packed_type(init_expr, data_type)
                            } else {
                                None
                            };
                            let raw = typed.unwrap_or_else(|| self.eval_expr_ctx(init_expr, eff_w));
                            Self::fit_local_decl_init(
                                raw,
                                eff_w,
                                data_type,
                                two_state,
                                is_real_type,
                                decl_signed,
                            )
                        }
                    }
                } else {
                    default_v.clone()
                };
                match self.widths.get_mut(d.name.name.as_str()) {
                    Some(slot) => *slot = eff_w,
                    None => {
                        self.widths.insert(d.name.name.clone(), eff_w);
                    }
                }
                self.store_local_decl_value(d, data_type, lifetime, plain, static_block, v);
                // Track `signed` scalars so reads carry `is_signed`
                // (needed for signed division/modulo, comparison, and
                // `%0d` display). A fresh decl also clears a stale
                // same-named signed flag from another frame.
                if decl_signed {
                    if !self.signed_signals.contains(d.name.name.as_str()) {
                        self.signed_signals.insert(d.name.name.clone());
                    }
                } else {
                    self.signed_signals.remove(&d.name.name);
                }
                // Track `real` locals so assignments coerce to real —
                // without this a block-local `real r; r = 3.14159;`
                // rounded through real_to_int and read back 3.0
                // (issue #23). A non-real decl clears a stale flag.
                if is_real_type {
                    self.real_signals.insert(d.name.name.clone());
                } else {
                    self.real_signals.remove(&d.name.name);
                }
                // Track `string`-typed locals so `{a, b}` concatenations
                // involving them are evaluated as string concat. A
                // non-string decl clears a stale same-named flag from
                // another frame (else `s[1]` on an `int s` would take
                // the string char-index path) — same pattern as
                // `signed_signals` above.
                if matches!(
                    data_type,
                    crate::ast::types::DataType::Simple {
                        kind: crate::ast::types::SimpleType::String,
                        ..
                    }
                ) {
                    self.string_signals.insert(d.name.name.clone());
                } else {
                    self.forget_string_flag(&d.name.name);
                }
                // §6.21: another process may declare a same-named string
                // while this frame is parked. Keep this declaration's kind
                // with the frame, independently of the global name tables.
                let is_string = self.declared_name_is_string(&d.name.name);
                self.record_frame_string_kind(&d.name.name, is_string);
                // Record a class-typed local's type so a later
                // separate `name = new();` knows what to construct.
                // Covergroup-typed locals (§19.8) are recorded too:
                // get_expr_type_name consults this map, and both the
                // bare `c = new;` and `c = new();` assignment paths
                // already route a covergroup type name to
                // instantiate_covergroup — they just never saw it
                // for a procedural local.
                if let Some(cn) = &class_name {
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
                        self.record_local_class_type(&d.name.name, cn);
                        if self.var_class_types.get(d.name.name.as_str()) != Some(cn) {
                            self.var_class_types.insert(d.name.name.clone(), cn.clone());
                        }
                        // A typedef'd local (e.g. `table_q_t rq` where
                        // `table_q_t = shared#(Foo[$])`) resolves to
                        // `cn` but hides the type_args inside the
                        // typedef chain. Record them so a separate
                        // `rq = new()` constructs the right
                        // specialization with type_bindings populated
                        // (§8.25 — otherwise `T` stays unbound).
                        if let crate::ast::types::DataType::TypeReference { name, .. } = data_type {
                            if plain_class {
                                // A plain class has no typedef to carry arguments.
                            } else if let Some((_, Some(ta))) =
                                self.resolve_typeref_class_with_type_args(name)
                            {
                                self.var_type_args.insert(d.name.name.clone(), ta);
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
                        self.record_local_class_type(&d.name.name, cn);
                        self.var_class_types.insert(d.name.name.clone(), cn.clone());
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
                let resolved_ta = match data_type {
                    crate::ast::types::DataType::TypeReference { name, .. } if !plain_class => self
                        .resolve_typeref_class_with_type_args(name)
                        .and_then(|(_, ta)| ta),
                    _ => None,
                };
                if let Some(ta) = self.scoped_class_type_args(data_type) {
                    self.var_type_args.insert(d.name.name.clone(), ta);
                } else if let crate::ast::types::DataType::TypeReference { type_args, .. } =
                    data_type
                {
                    if !type_args.is_empty() {
                        self.var_type_args
                            .insert(d.name.name.clone(), type_args.clone());
                    } else if let Some(ta) = resolved_ta {
                        self.var_type_args.insert(d.name.name.clone(), ta);
                    } else {
                        self.var_type_args.remove(&d.name.name);
                    }
                } else {
                    self.var_type_args.remove(&d.name.name);
                }
                // §15.3/§15.4: record a mailbox/semaphore-typed local so
                // a later separate `name = new(bound)` allocates the right
                // container. Clear any stale same-named entry from another
                // frame (var_container_types is a flat map like
                // var_class_types) when this decl isn't a container.
                match class_name
                    .as_deref()
                    .and_then(|t| self.builtin_container_kind(t))
                {
                    Some(k) => {
                        self.var_container_types
                            .insert(d.name.name.clone(), k.to_string());
                    }
                    None => {
                        self.var_container_types.remove(&d.name.name);
                    }
                }
                // LRM §6.19.6: track typedef-typed locals so
                // `local.next()`/`.first()`/etc. find their
                // enum-member list. We extract the bare type
                // name from a TypeReference data_type. (A plain class
                // is neither, nor a type parameter; a forward-declared one
                // is registered as its typedef here.)
                if let (false, crate::ast::types::DataType::TypeReference { name, .. }) =
                    (class_ref == Some(false), data_type)
                {
                    let tn = name.name.name.clone();
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
                        if self.var_typedef_types.get(d.name.name.as_str()) != Some(&tn) {
                            self.var_typedef_types
                                .insert(d.name.name.clone(), tn.clone());
                        }
                        // Record the frame-scoped typedef too so a
                        // method-local enum type is resolvable without
                        // the flat `var_class_types`/`var_typedef_types`
                        // collision across unrelated frames (a same-
                        // named formal `VISITOR v` in another method
                        // must not shadow this `uvm_verbosity v;
                        // local). `local_typedef_type_of` reads the
                        // innermost active frame. §6.19.6.
                        self.record_local_typedef_type(&d.name.name, &tn);
                    }
                }
            }
        }
        // §6.21 / §13.3.1: a `static` local declared inside a subroutine
        // keeps ONE persistent storage across all runtime calls, and its
        // initializer runs only once. On the FIRST call we persist the
        // just-initialized value; on later calls we restore the persisted
        // value (undoing this call's re-initialization). The final value
        // is written back on return by `sync_static_locals`. Only scalar
        // integral/real statics are tracked — arrays/queues keep per-call
        // storage.
        if matches!(lifetime, Some(crate::ast::types::Lifetime::Static))
            && !self.static_local_syncs.is_empty()
            && !self.in_const_param_eval
        {
            let sub_name = self
                .static_local_syncs
                .last()
                .map(|(n, _)| n.clone())
                .unwrap_or_default();
            for d in declarators {
                let nm = &d.name.name;
                // Hoisted declarations already use their shared global cell;
                // never create a second frame-backed scalar mirror.
                if self.hoisted_sub_static_names.contains(nm) {
                    continue;
                }
                // Skip aggregates: arrays/queues/assoc are not persisted.
                if !d.dimensions.is_empty()
                    || self.module.arrays.contains_key(nm)
                    || self.module.dynamic_arrays.contains(nm)
                    || self.module.associative_arrays.contains_key(nm)
                {
                    continue;
                }
                // Build the persistent key. A free function keeps
                // `"<subroutine>::<var>"` (its class context is None).
                // A METHOD of a parameterized class appends the active
                // specialization: `static this_type m_inst;` inside
                // `C#(P)::get()` must be one shared cell per
                // specialization (shared across calls of the SAME spec,
                // distinct across DIFFERENT specs) — IEEE 1800-2023
                // §8.25/§6.21. The class prefix also keeps two classes'
                // same-named method locals from colliding.
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
                if let Some(pv) = self.static_local_vars.get(&key).cloned() {
                    if let Some(l) = self.local_stack.last_mut() {
                        l.insert(nm.clone(), pv);
                    }
                } else {
                    let cur = self
                        .local_stack
                        .last()
                        .and_then(|l| l.get(nm))
                        .cloned()
                        .unwrap_or_else(|| Value::zero(w));
                    self.static_local_vars.insert(key.clone(), cur);
                }
                if let Some((_n, syncs)) = self.static_local_syncs.last_mut() {
                    syncs.push((nm.clone(), key.clone()));
                }
                // Route to the live shared cell only for scalar
                // integral/real statics (the re-entrant/recursive
                // case this fixes). Strings and other aggregates keep
                // their existing per-call storage semantics and must
                // not be intercepted.
                if !self.string_signals.contains(nm.as_str())
                    && !self.module.arrays.contains_key(nm.as_str())
                {
                    if let Some(&b0) = nm.as_bytes().first() {
                        self.static_local_first[b0 as usize] = true;
                    }
                    self.static_local_names.insert(nm.clone());
                    self.static_local_keys.insert(key);
                }
            }
        }
    }

    pub(super) fn exec_stmt_param_decl(
        &mut self,
        stmt: &Statement,
        data_type: &DataType,
        declarators: &[crate::ast::stmt::VarDeclarator],
    ) {
        if let crate::ast::types::DataType::Implicit {
            signing,
            dimensions,
            ..
        } = data_type
            && dimensions.is_empty()
        {
            for d in declarators {
                if !self.local_stack.is_empty() {
                    self.claim_local_meta(&d.name.name, stmt);
                }
                let mut v = match &d.init {
                    Some(init_expr) => {
                        let mut val = self.eval_expr(init_expr);
                        if val.is_fill {
                            val.is_fill = false;
                        }
                        val
                    }
                    None => Value::new(32),
                };
                if !v.is_real {
                    match signing {
                        Some(crate::ast::types::Signing::Signed) => v.is_signed = true,
                        Some(crate::ast::types::Signing::Unsigned) => v.is_signed = false,
                        None => {}
                    }
                }
                let eff_w = v.width;
                let is_real = v.is_real;
                let is_signed = !is_real && v.is_signed;
                let is_string = d
                    .init
                    .as_ref()
                    .is_some_and(|e| self.expr_is_string_valued(e));
                match self.widths.get_mut(d.name.name.as_str()) {
                    Some(slot) => *slot = eff_w,
                    None => {
                        self.widths.insert(d.name.name.clone(), eff_w);
                    }
                }
                if is_signed {
                    if !self.signed_signals.contains(d.name.name.as_str()) {
                        self.signed_signals.insert(d.name.name.clone());
                    }
                } else {
                    self.signed_signals.remove(&d.name.name);
                }
                if is_real {
                    self.real_signals.insert(d.name.name.clone());
                } else {
                    self.real_signals.remove(&d.name.name);
                }
                if is_string {
                    self.string_signals.insert(d.name.name.clone());
                } else {
                    self.forget_string_flag(&d.name.name);
                }
                self.record_frame_string_kind(&d.name.name, is_string);
                if let Some(frame) = self.local_stack.last_mut() {
                    frame.insert(d.name.name.clone(), v);
                } else {
                    self.signals.insert(d.name.name.clone(), v);
                }
            }
        } else {
            self.exec_stmt_var_decl(stmt, data_type, &None, declarators);
        }
    }

    /// Register a local array of collections (see the caller): `dims[..k]`
    /// fixed, `dims[k]` the element's collection kind. Returns false (nothing
    /// registered) when the outer shape is not a fixed one.
    #[cold]
    #[inline(never)]
    pub(in crate::compiler::simulator) fn declare_local_coll_array(
        &mut self,
        d: &VarDeclarator,
        data_type: &DataType,
        dims: &[crate::ast::types::UnpackedDimension],
        k: usize,
        w: u32,
        lifetime: Option<crate::ast::types::Lifetime>,
    ) -> bool {
        use crate::ast::types::UnpackedDimension as UD;
        if matches!(dims.first(), Some(UD::Unsized(_)))
            && dims.len() > 1
            && dims[1..]
                .iter()
                .all(|d| matches!(d, UD::Associative { .. }))
        {
            let name = if matches!(lifetime, Some(crate::ast::types::Lifetime::Static)) {
                d.name.name.clone()
            } else {
                self.declare_local_dyn(&d.name.name)
            };
            self.register_coll_name(&name, StructCollKind::Dynamic, w, false);
            self.module
                .var_decl_types
                .insert(name.clone(), data_type.clone());
            self.dynamic_assoc_dims
                .insert(name.clone(), dims[1..].to_vec());
            self.clear_dynamic_assoc_rows(&name, 0);
            self.set_queue_size(&name, 0);
            if let Some(init) = &d.init
                && let Some((n, src)) = Self::sized_new_parts(init)
            {
                self.fill_sized_new(&name, n, src);
            }
            return true;
        }
        let shape: Option<Vec<(i64, i64)>> = dims[..k]
            .iter()
            .map(|dim| match dim {
                UD::Range { left, right, .. } => {
                    let l = super::elaborate::const_eval_i64_with_params(left, None)?;
                    let r = super::elaborate::const_eval_i64_with_params(right, None)?;
                    Some((l.min(r), l.max(r)))
                }
                UD::Expression { expr, .. } => {
                    let n = super::elaborate::const_eval_i64_with_params(expr, None)?;
                    (n > 0).then_some((0, n - 1))
                }
                _ => None,
            })
            .collect();
        let Some(shape) = shape.filter(|sh| {
            sh.iter()
                .map(|&(lo, hi)| (hi - lo + 1).max(0))
                .product::<i64>()
                <= 4096
        }) else {
            return false;
        };
        let bare = d.name.name.clone();
        let name = if matches!(lifetime, Some(crate::ast::types::Lifetime::Static)) {
            bare
        } else {
            self.declare_local_dyn(&bare)
        };
        self.adopt_local_shape(&name);
        match shape.len() {
            1 => {
                self.module
                    .arrays
                    .insert(name.clone(), (shape[0].0, shape[0].1, w));
            }
            2 => {
                self.module
                    .arrays_2d
                    .insert(name.clone(), (shape[0], shape[1], w));
            }
            _ => {
                self.module
                    .arrays_nd
                    .insert(name.clone(), (shape.clone(), w));
            }
        }
        let assoc_key = match &dims[k] {
            UD::Associative { data_type: kdt, .. } => Some(kdt.as_ref().is_some_and(|dt| {
                matches!(
                    super::elaborate::resolve_typedef_chain(dt, &self.module.typedef_types),
                    DataType::Simple {
                        kind: crate::ast::types::SimpleType::String,
                        ..
                    }
                )
            })),
            _ => None,
        };
        let key_info = match &dims[k] {
            UD::Associative { data_type, .. } => super::elaborate::assoc_key_type_info(
                data_type.as_deref(),
                &self.module.parameters,
                &self.module.typedefs,
                &self.module.typedef_types,
            ),
            _ => (None, None),
        };
        let mut elems = Vec::new();
        for sfx in Self::nd_elem_suffixes(&shape) {
            let en = format!("{}{}", name, sfx);
            elems.push(en.clone());
            // A fresh declaration: drop what a prior frame's same-named
            // local left behind.
            for key in self.signals.keys_with_elem_prefix(&format!("{}[", en)) {
                self.signals.remove(&key);
            }
            match assoc_key {
                Some(is_str) => {
                    if let Some(index) = key_info.0 {
                        self.module.assoc_index_widths.insert(en.clone(), index);
                    }
                    if let Some(key_type) = &key_info.1 {
                        self.module
                            .assoc_key_type_names
                            .insert(en.clone(), key_type.clone());
                    }
                    self.module.assoc_elem_widths.insert(en.clone(), w);
                    self.module.associative_arrays.insert(en, is_str);
                }
                None => {
                    self.module.dynamic_arrays.insert(en.clone());
                    self.module.arrays.insert(en.clone(), (0, 63, w));
                    self.set_queue_size(&en, 0);
                }
            }
        }
        if name.starts_with('@') {
            self.local_coll_elems.insert(name.clone(), elems);
        }
        self.module.var_decl_types.insert(name, data_type.clone());
        true
    }

    /// Whether a local's unpacked dimension needs the enclosing scope to be
    /// sized: a size or bound that does not fold without parameters, or a
    /// `[NAME]` that parses as an associative key type but may name a
    /// constant.
    pub(in crate::compiler::simulator) fn local_dim_needs_scope(
        dim: &crate::ast::types::UnpackedDimension,
    ) -> bool {
        use super::elaborate::const_eval_i64_with_params as ce;
        use crate::ast::types::UnpackedDimension as UD;
        match dim {
            UD::Expression { expr, .. } => ce(expr, None).is_none(),
            UD::Range { left, right, .. } => ce(left, None).is_none() || ce(right, None).is_none(),
            UD::Associative {
                data_type: Some(dt),
                ..
            } => matches!(
                dt.as_ref(),
                crate::ast::types::DataType::TypeReference { dimensions, .. } if dimensions.is_empty()
            ),
            _ => false,
        }
    }

    /// §7.4.2 / §8.23: fold a local's scope-sized dimensions to literals,
    /// evaluated in the executing scope — `[N]` whose `N` is a constant and
    /// not a type, `[cls::N]`, and size/bound expressions over parameters or
    /// class constants. A dimension that does not resolve is left as is.
    #[cold]
    #[inline(never)]
    pub(in crate::compiler::simulator) fn scope_sized_local_dims(
        &mut self,
        dims: &[crate::ast::types::UnpackedDimension],
    ) -> Option<Vec<crate::ast::types::UnpackedDimension>> {
        use crate::ast::expr::{ExprKind, Expression, NumberBase, NumberLiteral};
        use crate::ast::types::UnpackedDimension as UD;
        fn lit(n: i64, span: crate::ast::Span) -> Box<Expression> {
            Box::new(Expression::new(
                ExprKind::Number(NumberLiteral::Integer {
                    size: None,
                    signed: true,
                    base: NumberBase::Decimal,
                    value: n.to_string(),
                    cached_val: Default::default(),
                }),
                span,
            ))
        }
        let normalized = super::elaborate::normalize_unpacked_dims(
            dims,
            &self.module.parameters,
            &self.module.typedef_types,
        );
        let mut changed = false;
        let mut out = Vec::with_capacity(normalized.len());
        for (orig, dim) in dims.iter().zip(normalized) {
            let folded = match &dim {
                UD::Expression { expr, span }
                    if super::elaborate::const_eval_i64_with_params(expr, None).is_none() =>
                {
                    self.eval_expr(expr)
                        .to_i64()
                        .filter(|&n| n > 0)
                        .map(|n| UD::Expression {
                            expr: lit(n, expr.span),
                            span: *span,
                        })
                }
                UD::Range { left, right, span } => {
                    let l = self.eval_expr(left).to_i64();
                    let r = self.eval_expr(right).to_i64();
                    match (l, r) {
                        (Some(l), Some(r)) => Some(UD::Range {
                            left: lit(l, left.span),
                            right: lit(r, right.span),
                            span: *span,
                        }),
                        _ => None,
                    }
                }
                // `[N]` still associative: a constant of the executing
                // scope (not a type) sizes it.
                UD::Associative {
                    data_type: Some(dt),
                    span,
                } => match dt.as_ref() {
                    crate::ast::types::DataType::TypeReference { name, .. }
                        if name.scopes.is_empty()
                            && !self.module.typedef_types.contains_key(&name.name.name)
                            && !self.module.classes.contains_key(&name.name.name)
                            && self.resolve_type_param_binding(&name.name.name).is_none() =>
                    {
                        let id = Expression::new(
                            ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                                root: None,
                                path: vec![crate::ast::expr::HierPathSegment {
                                    name: name.name.clone(),
                                    selects: Vec::new(),
                                }],
                                span: name.span,
                                cached_signal_id: crate::ast::cache::SignalCache::new(None),
                                cached_resolved_name: crate::ast::cache::NameCache::new(),
                                bound: crate::ast::binding::Bound::UNSET,
                            }),
                            name.span,
                        );
                        let v = self.eval_expr(&id);
                        (!v.is_real && v.width > 0)
                            .then(|| v.to_i64())
                            .flatten()
                            .filter(|&n| n > 0)
                            .map(|n| UD::Expression {
                                expr: lit(n, name.span),
                                span: *span,
                            })
                    }
                    _ => None,
                },
                _ => None,
            };
            // `normalize_unpacked_dims` turning `[N]` into a size counts too.
            changed |= folded.is_some()
                || matches!(
                    (orig, &dim),
                    (UD::Associative { .. }, UD::Expression { .. })
                );
            out.push(folded.unwrap_or(dim));
        }
        changed.then_some(out)
    }

    /// Member-wise declaration initializer of an unpacked-struct local
    /// (`row_t r = '{...};`, `op_s x = accesses[i];`, `pair_t p = o.orig;`).
    pub(in crate::compiler::simulator) fn init_unpacked_struct_local(
        &mut self,
        d: &VarDeclarator,
        data_type: &DataType,
        init_expr: &Expression,
    ) {
        let init_expr = init_expr.clone();
        let lv = crate::ast::expr::Expression::new(
            crate::ast::expr::ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                root: None,
                path: vec![crate::ast::expr::HierPathSegment {
                    name: crate::ast::Identifier {
                        name: d.name.name.clone(),
                        span: d.name.span,
                    },
                    selects: Vec::new(),
                }],
                span: d.name.span,
                cached_signal_id: crate::ast::cache::SignalCache::new(None),
                cached_resolved_name: crate::ast::cache::NameCache::new(),
                bound: crate::ast::binding::Bound::UNSET,
            }),
            d.name.span,
        );
        if self
            .try_decompose_struct_class_prop_read_assign(&lv, &init_expr)
            .is_none()
        {
            // §7.2: decl-init from any OTHER struct
            // source — another local, a queue/array
            // element (`op_s x = accesses[i];`, UVM's
            // reg-map bus access loop) — copies member
            // -wise exactly like the assignment arm.
            // The packed `v` stored above has no
            // leaves, so without this every member of
            // the fresh local stayed x.
            if let crate::ast::types::DataType::Struct(su) = self.resolve_dt(data_type) {
                let src = self.flat_member_name(&init_expr);
                match src {
                    Some(src) if self.struct_storage_exists(&src, &su) => {
                        self.copy_unpacked_struct(&d.name.name, &src, &su);
                    }
                    // An element of a class-property
                    // collection (UVM's `row_info row =
                    // m_rows[i];`) keeps its leaves under
                    // the instance, not the flat name.
                    _ if Self::is_struct_elem_ref(&init_expr) => {
                        self.assign_struct_memberwise(&lv, &init_expr, &su, 0);
                    }
                    // §5.10/§10.9.2: any other source (an assignment
                    // pattern, a call, a conditional) is exactly the
                    // assignment `name = init`, which gives the pattern
                    // the struct's member layout as its context. Evaluated
                    // self-determined, `'{a:5, b:6}` packed each item at
                    // 32 bits and the spread mis-placed every member
                    // (`a` read 5 << 24).
                    _ => {
                        let assign = crate::ast::stmt::Statement::new(
                            crate::ast::stmt::StatementKind::BlockingAssign {
                                lvalue: lv,
                                rvalue: init_expr,
                            },
                            d.name.span,
                        );
                        self.exec_statement(&assign);
                    }
                }
            }
        }
    }
}
