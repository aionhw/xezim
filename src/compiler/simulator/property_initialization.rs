//! Declaration-ordered property initialization during class construction.

use super::*;

impl Simulator {
    fn with_class_initialization_scope<R>(
        &mut self,
        handle: usize,
        class: &str,
        run: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let spec = self.heap.get(handle).and_then(|o| o.as_ref()).map(|o| {
            o.spec.clone().unwrap_or_else(|| {
                (
                    o.class_name.clone(),
                    self.canonicalize_spec_sig(&o.class_name, ""),
                )
            })
        });
        let signature = spec
            .and_then(|(base, sig)| self.ancestor_spec(&base, &sig, class))
            .unwrap_or_else(|| self.canonicalize_spec_sig(class, ""));
        let saved_spec = self.current_spec.replace((class.to_string(), signature));
        self.this_stack.push(Some(handle));
        self.class_context_stack.push(Some(class.to_string()));
        self.ctor_class_stack.push((handle, class.to_string()));
        let frame = self.take_pooled_frame();
        self.push_local_frame(frame);
        self.method_local_base.push(self.local_stack.len() - 1);
        let saved_dyn = std::mem::take(&mut self.local_dyn);
        let saved_formal = std::mem::take(&mut self.formal_dyn);
        let result = run(self);
        self.local_dyn = saved_dyn;
        self.formal_dyn = saved_formal;
        self.method_local_base.pop();
        self.pop_local_frame();
        self.ctor_class_stack.pop();
        self.class_context_stack.pop();
        self.this_stack.pop();
        self.current_spec = saved_spec;
        result
    }

    /// §8.7: a class initializes its fields after its base constructor returns
    /// and before its own constructor body. Removing the entry first also
    /// prevents re-entrant callbacks from initializing a field twice.
    pub(super) fn initialize_pending_class_properties(&mut self, handle: usize, class: &str) {
        let target = self.module.classes.get(class).cloned();
        let Some(classes) = self.pending_class_initializers.get_mut(&handle) else {
            return;
        };
        let Some(index) = classes.iter().position(|cd| {
            cd.name == class
                || target
                    .as_ref()
                    .is_some_and(|t| std::sync::Arc::ptr_eq(t, cd))
        }) else {
            return;
        };
        let cd = classes.remove(index);
        if classes.is_empty() {
            self.pending_class_initializers.remove(&handle);
        }
        if cd.property_inits.is_empty() {
            return;
        }
        self.with_class_initialization_scope(handle, &cd.name, |sim| {
            sim.initialize_instance_properties_in_class(handle, &cd);
        });
    }

    pub(super) fn initialize_implicit_constructor_base(&mut self, handle: usize, class: &str) {
        let Some(cd) = self.module.classes.get(class).cloned() else {
            return;
        };
        let Some(base) = cd.extends.clone() else {
            return;
        };
        // An extends argument is written in the class declaration, outside
        // the constructor's formal-argument scope.
        self.with_class_initialization_scope(handle, class, |sim| {
            sim.exec_method_in_class_hierarchy(handle, &base, "new", &cd.extends_args);
        });
    }

    pub(super) fn try_implicit_class_constructor(
        &mut self,
        handle: usize,
        class: &str,
        args: &[Expression],
    ) -> Option<Value> {
        let target = self.module.classes.get(class);
        let cd = self
            .pending_class_initializers
            .get(&handle)?
            .iter()
            .find(|cd| {
                cd.name == class || target.is_some_and(|target| std::sync::Arc::ptr_eq(target, cd))
            })?
            .clone();
        if cd.methods.contains_key("new") {
            return None;
        }
        if let Some(base) = &cd.extends {
            if cd.extends_args.is_empty() {
                // Preserve the caller's scope while binding forwarded actuals.
                self.exec_method_in_class_hierarchy(handle, base, "new", args);
            } else {
                self.initialize_implicit_constructor_base(handle, class);
            }
        }
        self.initialize_pending_class_properties(handle, class);
        Some(Value::zero(32))
    }

    fn initialize_instance_properties_in_class(
        &mut self,
        handle: usize,
        cdef: &std::sync::Arc<crate::compiler::elaborate::ElaboratedClass>,
    ) {
        // An initializer is evaluated IN the declaring class's scope, so a
        // bare reference to that class's own enum member must resolve
        // there (§6.19). Only `this_stack` was pushed, leaving the class
        // context unset, so `e_a v = DUP;` fell through to the flat
        // design-wide map and picked up an unrelated class's `DUP`.
        // §8.7: every initializer runs once, in declaration order.
        // Splitting calls from plain expressions lets later calls capture
        // stale fields; replaying readers also repeats their side effects.
        let mut inits: Vec<(String, &Expression)> = cdef
            .property_inits
            .iter()
            .map(|(k, v)| (k.clone(), v))
            .collect();
        inits.sort_by_key(|(name, _)| {
            cdef.property_order
                .iter()
                .position(|p| p == name)
                .unwrap_or(usize::MAX)
        });
        for (pname, init) in inits {
            // A STATIC member is shared across all instances (it lives in
            // `class_statics`, not per-instance), so it must NOT be
            // re-initialized / re-allocated per instance. Doing so gave each
            // phase instance its OWN `static mailbox m_phase_hopper`
            // (a fresh handle) — so `execute_phase` (this=phase) put the next
            // phase into a different mailbox than the one `m_run_phases`
            // (this=null) was parked on, and execution stalled after the
            // first phase. The static-init path (compile()) allocates the
            // single shared instance once.
            if self.initialize_class_constant_array(handle, &cdef.name, &pname) {
                continue;
            }
            if cdef.static_properties.contains(&pname) {
                continue;
            }
            // §8.10: a property this class declares but a derived class
            // redeclares lives under `<Class>::<name>` (seeded above);
            // its initializer must land there, not in the derived copy.
            let slot = {
                let mut q = std::mem::take(&mut self.inst_key_scratch);
                q.clear();
                q.push_str(&cdef.name);
                q.push_str("::");
                q.push_str(&pname);
                let shadowed = self
                    .heap
                    .get(handle)
                    .and_then(|o| o.as_ref())
                    .is_some_and(|i| i.properties.contains_key(q.as_str()));
                let slot = if shadowed { q.clone() } else { pname.clone() };
                self.inst_key_scratch = q;
                slot
            };
            // §8.7, §7.5.1: a dynamic-array property initialized with its
            // constructor (`int da[] = new[4];`) is sized here, like the
            // same assignment as the first statement of `new()`. The
            // call-bearing deferral below evaluated it as a scalar and
            // left the array empty.
            if let Some((n_expr, src_expr)) = Self::sized_new_parts(init) {
                let scoped = format!("{}#{}", handle, pname);
                if self.module.dynamic_arrays.contains(scoped.as_str()) {
                    self.fill_sized_new(&scoped, n_expr, src_expr);
                    continue;
                }
            }
            // §8.7: a mailbox/semaphore initializer allocates one container
            // for this object before its constructor body runs.
            let cont_kind = cdef
                .properties
                .get(&pname)
                .and_then(|s| s.type_name.as_deref())
                .and_then(Self::container_base);
            let (is_new, ctor_args): (bool, &[Expression]) = match &init.kind {
                ExprKind::Call { func, args } if matches!(&func.kind, ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].name.name == "new" && h.path[0].selects.is_empty()) => {
                    (true, args.as_slice())
                }
                ExprKind::Ident(h)
                    if h.path.len() == 1
                        && h.path[0].name.name == "new"
                        && h.path[0].selects.is_empty() =>
                {
                    (true, &[])
                }
                _ => (false, &[]),
            };
            if let (Some(kind), true) = (cont_kind, is_new) {
                // `semaphore s = new(1);`: the argument is the initial
                // key count (§15.3.1) or the mailbox bound (§15.4.1).
                let ch = self.alloc_builtin_container(kind, ctor_args);
                if let Some(Some(inst)) = self.heap.get_mut(handle) {
                    inst.properties.insert(slot, Value::from_u64(ch as u64, 32));
                }
                continue;
            }
            // §8.7/§8.25: construct the field's concrete declared class,
            // retaining its type-parameter and typedef specialization.
            if is_new {
                let prop_tn = self.class_prop_type_name(handle, &slot).or_else(|| {
                    cdef.properties
                        .get(&pname)
                        .and_then(|s| s.type_name.clone())
                });
                if let Some(tn) = prop_tn {
                    // §8.25: a bound type can itself name a specialization.
                    // Construct that type, rather than dispatching a bare new
                    // against the containing object.
                    if let (base, Some(sig)) = Self::strip_class_specialization(&tn)
                        && let Some(cd2) = self.module.classes.get(&base).cloned()
                    {
                        let sig = self.canonicalize_spec_sig(&base, &sig);
                        self.ensure_spec_statics(&base, &sig);
                        let saved = self.current_spec.replace((base, sig));
                        let value = self.instantiate_class(&cd2, ctor_args);
                        self.current_spec = saved;
                        if let Some(Some(inst)) = self.heap.get_mut(handle) {
                            inst.properties.insert(slot, value);
                        }
                        continue;
                    }
                    // A typedef alias to a parameterized class (UVM's
                    // `uvm_event_pool events = new("events");` in every
                    // transaction): build THAT specialization. The base
                    // lookup below dropped the type arguments, so the
                    // pool made plain `uvm_object`s and every
                    // `wait_trigger` on one returned at once.
                    if let Some((base, sig)) = self.resolve_typedef_spec(&tn) {
                        if let Some(cd2) = self.module.classes.get(&base).cloned() {
                            self.ensure_spec_statics(&base, &sig);
                            let saved_spec = self.current_spec.clone();
                            self.current_spec = Some((base, sig));
                            let v = self.instantiate_class(&cd2, ctor_args);
                            self.current_spec = saved_spec;
                            if let Some(Some(inst)) = self.heap.get_mut(handle) {
                                inst.properties.insert(slot, v);
                            }
                            continue;
                        }
                    }
                    let concrete_tn =
                        if let Some(crate::ast::types::DataType::TypeReference { name, .. }) =
                            self.lookup_typedef_target(&tn)
                        {
                            name.name.name.clone()
                        } else {
                            tn
                        };
                    if self.module.classes.contains_key(&concrete_tn) {
                        if let Some(cd2) = self.module.classes.get(&concrete_tn).cloned() {
                            // Resolve any type-parameter bindings the
                            // ctor args may need from the active spec.
                            let ta = self.this_property_type_args(&pname);
                            let v = self.instantiate_class_with_type_args(
                                &cd2,
                                ctor_args,
                                ta.as_deref(),
                            );
                            if let Some(Some(inst)) = self.heap.get_mut(handle) {
                                inst.properties.insert(slot, v);
                            }
                            continue;
                        }
                    }
                }
                continue;
            }
            // §10.9: a typed pattern of an unpacked type initializes the
            // property as an untyped one does.
            let init = self.unpacked_pattern(init);
            // §7.9.11: an associative-array property's pattern
            // initializer (`int m[int] = '{1: 5, default: -1}`) fills the
            // per-instance store and records its default.
            if let ExprKind::AssignmentPattern(items) = &init.kind
                && cdef.assoc_properties.contains_key(&pname)
            {
                let scoped = format!("{}#{}", handle, pname);
                let val_w = cdef.properties.get(&pname).map(|s| s.width).unwrap_or(32);
                self.assign_class_assoc_pattern(&scoped, val_w, items);
                continue;
            }
            // Queue / dynamic-array member initializer (`int q[$] =
            // '{1,2,3}`): the per-instance storage lives at
            // `<handle>#member`, NOT in the scalar `properties` map.
            // Evaluate the initializer and populate the queue namespace.
            let is_queue_init = cdef.queue_properties.contains_key(&pname) || {
                use std::fmt::Write as _;
                let mut q = std::mem::take(&mut self.inst_key_scratch);
                q.clear();
                let _ = write!(q, "{}#{}", handle, pname);
                let hit = self.module.dynamic_arrays.contains(q.as_str());
                self.inst_key_scratch = q;
                hit
            };
            if is_queue_init {
                let scoped_q = format!("{}#{}", handle, pname);
                // An unpacked-struct element (one with a string member,
                // say) is stored member-wise as `<prop>[i].<m>`, like the
                // fixed-array case below.
                if let ExprKind::AssignmentPattern(items) = &init.kind
                    && let Some(su) = self
                        .collection_elem_struct(handle, &pname)
                        .filter(Self::spreads_member_wise)
                {
                    self.set_queue_size(&scoped_q, items.len() as u64);
                    let base = prop_ident_expr(&pname);
                    for (i, item) in items.iter().enumerate() {
                        let elem = Self::index_expr(&base, i as i64);
                        self.assign_class_unpacked_elem(&elem, item.expr(), &su);
                    }
                    continue;
                }
                self.populate_queue_from_init(&scoped_q, &init);
                continue;
            }
            // Fixed-size UNPACKED array member initializer
            // (`int fa[4] = '{1,2,3,4}`): per-instance storage is the
            // element signals `<handle>#fa[i]` (registered above as a real
            // fixed array), NOT a scalar slot in the property map. Expand
            // the assignment pattern over the DECLARED index range, so the
            // replication form `'{4{9}}` and the `default:` fill
            // (`'{default:7}`) reach EVERY element instead of collapsing
            // into `fa[0]` as one packed value (LRM §10.9.1 / §10.10). The
            // positional `'{1,2,3,4}` path is unchanged, and non-zero lower
            // bounds (`fa[1:4]`) keep their declared indices. This shares
            // `pattern_elems`, the same expansion the general array-pattern
            // spread (`assign_pattern_array`) uses.
            if let Some(&(lo, hi, _w)) = cdef.array_properties.get(&pname) {
                let (lo, hi) = self.inst_fixed_bounds(cdef, handle, &pname, (lo, hi));
                let scoped = format!("{}#{}", handle, pname);
                if let ExprKind::AssignmentPattern(items) = &init.kind {
                    let indices: Vec<i64> = (lo..=hi).collect();
                    // §7.2/§18.4: an UNPACKED-STRUCT element has no single
                    // cell — its members live as `<prop>[i].<m>` in the
                    // instance property map, which is where the read path
                    // (`class_unpacked_leaf`) looks. Storing the element as
                    // one packed value under the signal name `<h>#<prop>[i]`
                    // wrote something nothing reads, so every element of a
                    // `localparam`/`const` array of unpacked structs came
                    // back 0 — the shape a directed-stimulus table uses.
                    // Decompose into the same cells the runtime element
                    // write now uses.
                    if let Some(su) = self.class_prop_struct(handle, &pname) {
                        if Self::spreads_member_wise(&su) {
                            let base = prop_ident_expr(&pname);
                            let elems = self.pattern_elems(items, &indices);
                            let owned: Vec<(usize, Option<Expression>)> =
                                elems.into_iter().map(|e| e.cloned()).enumerate().collect();
                            for (k, e) in owned {
                                if let Some(e) = e {
                                    let elem = Self::index_expr(&base, indices[k]);
                                    self.assign_class_unpacked_elem(&elem, &e, &su);
                                }
                            }
                            continue;
                        }
                    }
                    let elems = self.pattern_elems(items, &indices);
                    for (k, e) in elems.into_iter().enumerate() {
                        if let Some(e) = e {
                            let en = format!("{}[{}]", scoped, indices[k]);
                            let mut v = self.eval_expr(e);
                            // Each element is assigned (§10.9.1): it
                            // takes the element's width — an unsized
                            // fill `'1` too (§5.7.1).
                            if !v.is_real && !self.is_string_collection(&scoped) {
                                if let Some(&w) = self.widths.get(&en) {
                                    if w > 0 && w != v.width {
                                        v = v.resize_for_assign(w);
                                    }
                                }
                            }
                            self.set_signal_value_by_name(&en, v);
                        }
                    }
                }
                continue;
            }
            if let ExprKind::AssignmentPattern(items) = &init.kind
                && let Some(su) = self
                    .class_prop_struct(handle, &pname)
                    .filter(|su| Self::spreads_member_wise(su))
            {
                let base =
                    Self::hier_ident_expr(Self::iface_path_segs(&pname, init.span), init.span);
                self.assign_class_unpacked_elem(&base, init, &su);
                // §7.2.1: the DYNAMIC members take their items as whole
                // collections, under the instance (`<h>#s.q`).
                if self.struct_colls_possible() {
                    for (mname, _, dims, e, explicit) in self.bind_struct_pattern_items(&su, items)
                    {
                        if let (Some(e), true) = (
                            e,
                            explicit && dims.len() == 1 && Self::coll_kind_of(&dims).is_some(),
                        ) {
                            let lhs = Self::append_member_expr(&base, &mname);
                            self.assign_struct_member_coll(&lhs, e);
                        }
                    }
                }
                continue;
            }
            // §7.9.11/§8.7: an associative-array property initializer
            // (`int m[string] = '{"a": 1}`) fills the instance's own
            // entries; evaluated whole it became one scalar nobody reads.
            if matches!(init.kind, ExprKind::AssignmentPattern(_))
                && cdef.assoc_properties.contains_key(&pname)
            {
                let scoped = format!("{}#{}", handle, pname);
                if self.is_associative_array(&scoped) {
                    self.assign_whole_coll(&scoped, None, init);
                    continue;
                }
            }
            let real = cdef.properties.get(&pname).is_some_and(|sig| sig.is_real);
            // §10.8: a pattern takes the property's declared type.
            let decl_ty = if has_pattern(&init) {
                self.class_property_decl_type(&cdef.name, &pname)
                    .filter(|(_, unpacked)| unpacked.is_empty())
                    .map(|(dt, _)| dt)
            } else {
                None
            };
            self.evaluate_property_init_at_construct(handle, &slot, init, real, decl_ty);
        }
    }

    /// §8.7 / §10.7: evaluate a scalar property initializer once, in its
    /// declared assignment context, while constructing the object.
    fn evaluate_property_init_at_construct(
        &mut self,
        handle: usize,
        pname: &str,
        init: &Expression,
        real: bool,
        decl_ty: Option<DataType>,
    ) {
        // An initializer may construct another object. Bound malformed
        // self-recursive declarations without re-running any initializer.
        if self.construct_init_depth >= 64 {
            return;
        }
        let width = self.heap_prop_width(handle, pname);
        self.construct_init_depth += 1;
        let typed = decl_ty.and_then(|dt| self.eval_in_packed_type(init, &dt));
        let value = match (typed, width) {
            (Some(v), _) => v,
            (None, Some(width)) => self.eval_expr_ctx(init, width),
            (None, None) => self.eval_expr(init),
        };
        self.construct_init_depth -= 1;
        let value = if real {
            Value::from_f64(value.to_f64())
        } else if let Some(width) = width {
            self.fit_pattern_item_value(value, width)
        } else {
            value
        };
        let value = self.fit_class_prop(handle, pname, &value);
        if let Some(Some(inst)) = self.heap.get_mut(handle) {
            inst.properties.insert(pname.to_string(), value);
        }
    }
}

fn prop_ident_expr(name: &str) -> Expression {
    Expression::new(
        ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
            root: None,
            path: vec![crate::ast::expr::HierPathSegment {
                name: crate::ast::Identifier {
                    name: name.to_string(),
                    span: crate::ast::Span::dummy(),
                },
                selects: Vec::new(),
            }],
            span: crate::ast::Span::dummy(),
            cached_signal_id: crate::ast::cache::SignalCache::new(None),
            cached_resolved_name: crate::ast::cache::NameCache::new(),
        }),
        crate::ast::Span::dummy(),
    )
}
