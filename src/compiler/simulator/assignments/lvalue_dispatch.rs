//! Assignment to a target of any shape.

use super::*;

impl Simulator {
    pub(in crate::compiler::simulator) fn assign_value_inner(
        &mut self,
        lhs: &Expression,
        val: &Value,
    ) -> bool {
        // #1108: an array member of an instance-array element, or a select
        // of it (`lane[0].nest[1][j] = v`).
        if matches!(
            lhs.kind,
            ExprKind::Index { .. } | ExprKind::RangeSelect { .. } | ExprKind::MemberAccess { .. }
        ) && let Some(folded) = self.inst_array_member_canon(lhs)
        {
            return self.assign_value_inner(&folded, val);
        }
        // §19.7 `cg.option.<name> = v` on a covergroup instance.
        if let ExprKind::Ident(h) = &lhs.kind {
            if h.path.len() == 3 && h.path[1].name.name == "option" {
                if let Some(idx) = self.cg_option_target(h) {
                    if let Some(Some(inst)) = self.cg_heap.get_mut(idx) {
                        inst.options
                            .insert(h.path[2].name.name.clone(), val.clone());
                    }
                    return true;
                }
            }
        }
        if !self.module.packages.is_empty() {
            if let Some(stripped) = self.strip_package_lvalue(lhs) {
                return self.assign_value_inner(&stripped, val);
            }
        }
        if let Some(flat) = self.top_rooted_member_lvalue(lhs) {
            return self.assign_value_inner(&flat, val);
        }
        // §25.9: a just-returned vif (recorded by the Return arm) binds to
        // the FIRST assignment target after the call — `value = r.read(c)`
        // in uvm_config_db::get (issue #113). One-shot; also cleared at the
        // next function-call entry so it cannot leak further than that.
        // The marker is probed only when a `return <vif>` left one: the
        // signal-map lookup was a string hash on EVERY assignment.
        if self.vif_return_pending
            && let Some(nm) = self
                .signals
                .remove("__vif_return__")
                .map(|x| x.to_sv_string())
                .filter(|s| !s.is_empty())
        {
            self.vif_return_pending = false;
            let synth = Expression::new(
                ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                    root: None,
                    path: vec![crate::ast::expr::HierPathSegment {
                        name: crate::ast::Identifier {
                            name: nm,
                            span: crate::ast::Span::dummy(),
                        },
                        selects: Vec::new(),
                    }],
                    span: crate::ast::Span::dummy(),
                    cached_signal_id: crate::ast::cache::SignalCache::new(None),
                    cached_resolved_name: crate::ast::cache::NameCache::new(),
                    bound: crate::ast::binding::Bound::UNSET,
                }),
                crate::ast::Span::dummy(),
            );
            // §25.9/§8.9: a static virtual-interface property binds too.
            self.bind_static_vif(lhs, &synth);
            let _ = self.try_bind_virtual_iface(lhs, &synth);
        }
        // §13.5.2: a selected write through a `ref` formal lands on the
        // caller's actual; an aliased formal's local copy is refreshed so
        // any unredirected read path stays coherent.
        if self.ref_redirect_possible()
            && matches!(lhs.kind, ExprKind::MemberAccess { .. } | ExprKind::Ident(_))
            && let Some(root) = Self::expr_root_first_seg(lhs)
            && self.ref_elem_handle_formal(root)
        {
            let root = root.to_string();
            self.ref_alias_refresh_slot(&root);
        }
        if let Some(rewritten) = self.ref_formal_redirect_expr(lhs) {
            let changed = self.assign_value(&rewritten, val);
            let mut root = lhs;
            loop {
                match &root.kind {
                    ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => {
                        root = expr
                    }
                    _ => break,
                }
            }
            if let ExprKind::Ident(h) = &root.kind {
                let fname = h.path[0].name.name.clone();
                if let Some(storage) = self
                    .ref_alias_stack
                    .last()
                    .and_then(|m| m.get(&fname))
                    .cloned()
                {
                    if let Some(full) = self.ref_alias_read(&storage) {
                        if let Some(frame) = self.local_stack.last_mut() {
                            if let Some(slot) = frame.get_mut(&fname) {
                                *slot = full;
                            }
                        }
                    }
                }
            }
            return changed;
        }
        if let ExprKind::Ident(h) = &lhs.kind {
            if h.path.len() == 3 && h.path[1].name.name == "type_option" {
                if let Some((cg, field)) = self.cg_type_option_path(h) {
                    // Seed the table (defaults + body items) before the write.
                    let _ = self.cg_type_option_get(&cg, &field);
                    if let Some(t) = self.cg_type_options.get_mut(&cg) {
                        t.insert(field, val.clone());
                    }
                    return true;
                }
            }
        }
        if let Some(rewritten) = self.super_rewrite(lhs) {
            return self.assign_value(&rewritten, val);
        }
        // §25.8/§25.9: a write through a bound virtual interface lands on the
        // bound instance, whatever the chain shape.
        if let Some(rewritten) = self.vif_rebase_expr(lhs) {
            return self.assign_value(&rewritten, val);
        }
        // §11.4.12: `$` in an index is the last valid index of the collection
        // being indexed. The READ paths install `dollar_bound` before
        // evaluating; no lvalue path did, so `$` fell through to its
        // `u64::MAX` default and the write went to a nonexistent element.
        // Normalising the whole lvalue ONCE here covers every arm below —
        // plain `q[$]`, a part-select of it (`q[$][3:0]`), and nested forms —
        // rather than each arm having to remember to push the bound. Gated on
        // a cheap scan, so an lvalue without `$` is untouched.
        let dollar_free;
        let lhs: &Expression = if Self::select_mentions_dollar(lhs) {
            dollar_free = self.resolve_dollar_in_lvalue(lhs);
            &dollar_free
        } else {
            lhs
        };

        // Packed geometry applies after reference, interface and $ rewrites.
        if (self.needs_declared_packed_access(lhs) || self.needs_static_packed_access(lhs))
            && let Some(changed) = self.write_declared_packed_access(lhs, val)
        {
            return changed;
        }

        // §7.4.6 / §11.5.1: an ELEMENT index containing x or z selects nothing,
        // so the write is discarded — `a['hx] = v` must leave every element
        // alone. The index folded to 0 through `to_u64`, so it silently
        // clobbered a[0]. The right-hand side is still evaluated by the caller
        // (its side effects stand); only the store is dropped.
        if let ExprKind::Index { expr: base, index } = &lhs.kind {
            if !Self::expr_is_stream(base) {
                let iv = self.eval_expr(index);
                if iv.has_xz() && !iv.is_real {
                    let store = self.expr_assoc_name(base).or_else(|| self.flat_member_name(base));
                    if let Some(name) = store.filter(|n| self.is_queue_store(n)) {
                        self.warn_invalid_queue_write(&name, None, self.get_queue_size(&name));
                    }
                    return false;
                }
                if let ExprKind::Ident(h) = &base.kind {
                    let qn = self.resolve_hier_name(h);
                    if self.module.queue_vars.contains(&*qn) {
                        let name = qn.into_owned();
                        if self.prepare_sequence_write(&name, &iv).is_none() {
                            return false;
                        }
                    }
                }
            }
        }

        // §18.4: writing a member of a struct/union CLASS property goes to the
        // aggregate's own storage — for a packed struct/union that means
        // splicing the member's bits into the property's raw integral value, so
        // the whole/field views (and a union's overlaid members) stay coherent.
        // Both parse shapes (`MemberAccess` and a flattened `Ident([a, b])`)
        // route here, so this sits ahead of the per-kind arms. A member with
        // an index or part-select below it (`obj.pkt.mem[i] = v`,
        // `obj.pkt.mem[1][255:248] = v`) resolves through the same call — the
        // select-aware walk in `class_agg_member` splices the element's bits.
        if matches!(
            lhs.kind,
            ExprKind::MemberAccess { .. }
                | ExprKind::Ident(_)
                | ExprKind::Index { .. }
                | ExprKind::RangeSelect { .. }
        ) {
            if let Some(r) = self.class_agg_member(lhs) {
                return self.write_class_agg(&r, val);
            }
        }
        // §7.2, §8.4: a property of the object a class-handle member of an
        // unpacked-struct class property refers to (`r.h.k = v`, `o.r.h.k =
        // v`). The receiver is a struct leaf, not a handle variable, so no
        // arm below resolved it and the write was dropped.
        if matches!(lhs.kind, ExprKind::MemberAccess { .. } | ExprKind::Ident(_))
            && !self.no_class_objects()
            && self.chain_may_name_struct_prop(lhs)
            && let Some((recv, member)) = Self::split_trailing_member(lhs)
            && let Some(h) = self.struct_handle_chain_object(&recv)
            && self
                .heap
                .get(h)
                .and_then(|o| o.as_ref())
                .is_some_and(|inst| inst.properties.contains_key(&member))
        {
            let fitted = self.fit_class_prop(h, &member, val);
            if let Some(Some(inst)) = self.heap.get_mut(h) {
                inst.properties.insert(member, fitted);
                return true;
            }
        }
        // §8.9: assignment to a STATIC class property (`obj.static_prop = ...`,
        // `this.static_prop = ...`) writes the single shared cell, not per-
        // instance storage. A named reference resolves by its declared type
        // and specialization (§8.25), so a null handle works and a derived
        // object's same-named instance property is not mistaken for it; this
        // arm runs before the instance-property arms below. A non-static
        // member falls through to them.
        {
            let obj_prop: Option<(String, String)> = match &lhs.kind {
                ExprKind::Ident(h)
                    if h.path.len() == 2 && h.path.iter().all(|s| s.selects.is_empty()) =>
                {
                    Some((h.path[0].name.name.clone(), h.path[1].name.name.clone()))
                }
                ExprKind::MemberAccess { expr, member } => match &expr.kind {
                    ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                        Some((h.path[0].name.name.clone(), member.name.clone()))
                    }
                    _ => None,
                },
                _ => None,
            };
            if let Some((obj, prop)) = obj_prop {
                if obj != "this" && obj != "super" {
                    let handle = self.eval_ident_handle(&obj).unwrap_or(0);
                    let resolved = match &lhs.kind {
                        ExprKind::Ident(h) => {
                            let head_id = HierarchicalIdentifier {
                                root: h.root.clone(),
                                path: vec![h.path[0].clone()],
                                span: h.span,
                                cached_signal_id: crate::ast::cache::SignalCache::new(None),
                                cached_resolved_name: crate::ast::cache::NameCache::new(),
                                bound: crate::ast::binding::Bound::UNSET,
                            };
                            Some(self.resolve_hier_name(&head_id).into_owned())
                        }
                        ExprKind::MemberAccess { expr, .. } => match &expr.kind {
                            ExprKind::Ident(h) => Some(self.resolve_hier_name(h).into_owned()),
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some(ok) = self.static_write_via_handle(
                        handle,
                        Some(&obj),
                        resolved.as_deref(),
                        &prop,
                        val.clone(),
                    ) {
                        return ok;
                    }
                }
                let cn = if obj == "this" {
                    self.this_stack
                        .last()
                        .copied()
                        .flatten()
                        .and_then(|h| self.heap.get(h))
                        .and_then(|o| o.as_ref())
                        .map(|i| i.class_name.clone())
                } else {
                    // A named reference was resolved above, by its
                    // declared type.
                    None
                };
                if let Some(cn) = cn {
                    if self.static_prop_key(&cn, &prop).is_some() {
                        let mut v = val.clone();
                        if let Some(w) = self.class_prop_width(&cn, &prop) {
                            if (w != v.width || v.is_fill) && !v.is_real {
                                v = v.resize_for_assign(w);
                            }
                        }
                        return self.class_static_set(&cn, &prop, v);
                    }
                }
            }
        }
        // §7.8.6/§8.4: `coll[key].prop = v` where the element is a CLASS
        // HANDLE writes the heap object the handle points at — not a
        // composed `coll[key].prop` signal (which silently forks the
        // element's identity from the stored handle: writes through the
        // element stopped being visible through other handles to the same
        // object). Gated on the receiver's value resolving to a live heap
        // instance that declares the property, so struct-element member
        // writes (composed-name storage) are untouched.
        // §4c: `h.prop = v` where `h` is a MODULE-SCOPE class-handle variable
        // (either parse shape). A REAL hierarchical signal `h.prop` must win —
        // only route to the heap when no such signal exists and the head's
        // value is a live object declaring the property. Previously the write
        // landed on a phantom flattened name and silently vanished.
        {
            let head_field: Option<(String, String)> = match &lhs.kind {
                ExprKind::Ident(h2)
                    if h2.path.len() == 2 && h2.path.iter().all(|s| s.selects.is_empty()) =>
                {
                    Some((h2.path[0].name.name.clone(), h2.path[1].name.name.clone()))
                }
                ExprKind::MemberAccess { expr: recv, member } => match &recv.kind {
                    ExprKind::Ident(h2) if h2.path.len() == 1 && h2.path[0].selects.is_empty() => {
                        Some((h2.path[0].name.name.clone(), member.name.clone()))
                    }
                    _ => None,
                },
                _ => None,
            };
            if let Some((head, field)) = head_field {
                let is_local = self
                    .local_stack
                    .last()
                    .is_some_and(|l| l.contains_key(&head));
                let dotted = format!("{}.{}", head, field);
                let dotted_signal = self.signal_name_to_id.contains_key(dotted.as_str())
                    || self.signals.contains_key(&dotted);
                if !is_local && !dotted_signal {
                    // §23.7: `<top>.<sig>` from a class method writes the top
                    // module's own variable (silently dropped before).
                    if head == self.module.name
                        && (self.signal_name_to_id.contains_key(field.as_str())
                            || self.signals.contains_key(&field))
                    {
                        self.set_signal_value_by_name(&dotted, val.clone());
                        return true;
                    }
                    // id-collision-in-heap: only a head that STATICALLY denotes
                    // a class-object lvalue may reinterpret its stored value as
                    // a heap handle here. A packed-struct variable splices its
                    // members into one integral value (§7.8), so
                    // `sv.delay_counter` with `sv == 32'd1` diverted the field
                    // write into `heap[1].delay_counter` — the write vanished
                    // from the struct and clobbered an unrelated object. The
                    // gate mirrors `bare_receiver_is_class_handle`'s static
                    // conjunct (enum locals shadow the flat class map) without
                    // its runtime-liveness conjunct: the colliding value is
                    // live by construction, so only the declared shape of the
                    // name discriminates.
                    if self.bare_name_is_class_channel(&head) {
                        if let Some(h) = self.eval_ident_handle(&head) {
                            if h != 0 {
                                let holds = self
                                    .heap
                                    .get(h)
                                    .and_then(|o| o.as_ref())
                                    .is_some_and(|inst| inst.properties.contains_key(&field));
                                if holds {
                                    let fitted = self.fit_class_prop(h, &field, val);
                                    if let Some(Some(inst)) = self.heap.get_mut(h) {
                                        inst.properties.insert(field, fitted);
                                        return true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if let ExprKind::MemberAccess { expr: recv, member } = &lhs.kind {
            if let ExprKind::Index { expr: base, .. } = &recv.kind {
                // id-collision-in-heap: an indexed receiver's VALUE is a heap
                // handle only when the collection's ELEMENT type is a class
                // (`ch[i].prop = v`). A struct element's packed payload is
                // data, not a handle — rerouting it into the heap stole the
                // write from the element. Non-ident bases (method results,
                // nested selects) keep the legacy value-based check.
                let base_is_handle_container = match &base.kind {
                    ExprKind::Ident(bh) if bh.path.len() == 1 && bh.path[0].selects.is_empty() => {
                        // The element-class map also records STRUCT typedef
                        // names (elaboration keys every collection), so verify
                        // the resolved element type really is a class before
                        // treating the element's value as a heap handle.
                        self.elem_class_of_arr(&bh.path[0].name.name)
                            .is_some_and(|c| self.module.classes.contains_key(&c))
                    }
                    _ => true,
                };
                if base_is_handle_container {
                    let h = self.eval_expr(recv).to_u64().unwrap_or(0) as usize;
                    if h != 0 && h < self.heap.len() {
                        let holds = self
                            .heap
                            .get(h)
                            .and_then(|o| o.as_ref())
                            .is_some_and(|inst| inst.properties.contains_key(&member.name));
                        if holds {
                            let fitted = self.fit_class_prop(h, &member.name, val);
                            if let Some(Some(inst)) = self.heap.get_mut(h) {
                                inst.properties.insert(member.name.clone(), fitted);
                                return true;
                            }
                        }
                    }
                }
            }
        }
        // §7.4.1: `<obj>.<prop>[i] = ...` on a multi-dimensional PACKED array
        // property splices the element's bits. Without this the write fell
        // through to a bit-select path that dropped it entirely.
        if let ExprKind::Index { expr, index } = &lhs.kind {
            // `class_packed_elem_ref` takes references and bails immediately on
            // an empty heap, so the old `expr.clone()` / `index.clone()` deep-
            // copied two expression trees on EVERY indexed write — including
            // designs with no classes at all, where the callee then returned
            // None. That was pure allocator traffic on the hot array-store path
            // (a 256K-element memory fill pays it 256K times). Pass the borrows
            // and skip the call entirely when no objects exist.
            if !self.no_class_objects()
                && matches!(
                    &expr.kind,
                    ExprKind::MemberAccess { .. } | ExprKind::Ident(_) | ExprKind::This
                )
            {
                if let Some(r) = self.class_packed_elem_ref(expr, index) {
                    return self.write_class_agg(&r, val);
                }
            }
        }
        // §7.4.2/§7.4.1: an indexed/part-select write INTO an element of an
        // unpacked-array class property whose element type is a
        // multi-dimensional packed array (`d.arr[1][2] = v`,
        // `d.arr[1][8:0] = v` on `bit [8:0][7:0] arr[3]`; bare in a method
        // too). The element lives whole under the per-instance key
        // `<h>#arr[1]`, and the trailing select names a packed ELEMENT (an
        // ew-bit slice) or an element RANGE — but the generic arms below
        // treat it as a bit-select/bit-range of a phantom scalar, so the
        // write was silently dropped (bit form) or spliced at raw bit
        // offsets (range form). The read side (`class_packed_operand`)
        // already does this walk; mirror it for the store.
        if !self.no_class_objects()
            && match &lhs.kind {
                ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => matches!(
                    &expr.kind,
                    ExprKind::Index { .. } | ExprKind::MemberAccess { .. } | ExprKind::Ident(_)
                ),
                _ => false,
            }
            && let Some(r) = self.class_packed_elem_sel_write(lhs, val)
        {
            return r;
        }
        // §13.5.2: a write to an ALIASED `ref` formal writes THROUGH to the
        // caller's storage immediately, so concurrent observers of the actual
        // see it mid-call (see `ref_alias_stack`). The storage is written BY
        // NAME — evaluating the bound ident would hit the callee frame's own
        // copy when the formal is named like its actual. The local copy is
        // refreshed too, keeping any unredirected read path coherent.
        if !self.ref_alias_stack.is_empty() {
            if let ExprKind::Ident(h) = &lhs.kind {
                if h.path.len() == 1 && h.path[0].selects.is_empty() {
                    let fname = h.path[0].name.name.as_str();
                    if let Some(storage) = self
                        .ref_alias_stack
                        .last()
                        .and_then(|m| m.get(fname))
                        .cloned()
                    {
                        let fname = fname.to_string();
                        if let Some(frame) = self.local_stack.last_mut() {
                            if let Some(slot) = frame.get_mut(&fname) {
                                let fitted = if !slot.is_real && !val.is_real && slot.width > 0 {
                                    val.resize_for_assign(slot.width)
                                } else if !slot.is_real && val.is_real {
                                    // §6.12.2: a real into an integral slot rounds.
                                    Self::real_into_slot(slot, val.to_f64())
                                } else {
                                    val.clone()
                                };
                                *slot = fitted;
                            }
                        }
                        if self.vif_frame_alias(&fname).is_some() {
                            // §13.5.2: ref rebinding is visible immediately.
                            if let Some(bound) = val
                                .to_u64()
                                .and_then(|t| self.iface_by_token(t))
                                .map(str::to_string)
                            {
                                self.viface_var_aliases.insert(storage.clone(), bound);
                            } else {
                                self.viface_var_aliases.remove(&storage);
                            }
                        }
                        return self.ref_alias_write(&storage, val);
                    }
                }
            }
        }
        // §7.2/§13.4.2: a leaf of a frame-owned unpacked struct — at any depth,
        // and through an index. The arms below all resolve against the module
        // signal table, so without this the write left the frame's copy alone
        // and the matching read returned an outer scope's value.
        if !self.local_stack.is_empty() {
            if let Some(key) = self.frame_leaf_key_of(lhs) {
                let prev = self.frame_get(&key).cloned();
                let fitted = match prev.as_ref() {
                    Some(p) if p.is_real && !val.is_real => Value::from_f64(val.to_f64()),
                    Some(p) if !p.is_real && !val.is_real && p.width > 0 => val.resize_for_assign(p.width),
                    Some(p) if !p.is_real && val.is_real => Self::real_into_slot(p, val.to_f64()),
                    _ => val.clone(),
                };
                let changed = prev.as_ref() != Some(&fitted);
                // §13.5.2: a leaf of a `ref` struct formal aliased to the
                // actual's member is written through at once.
                let through = if self.ref_redirect_possible() {
                    self.ref_alias_stack
                        .last()
                        .and_then(|m| m.get(key.as_str()))
                        .cloned()
                        .map(|storage| (storage, fitted.clone()))
                } else {
                    None
                };
                if let Some(frame) = self.local_stack.last_mut() {
                    frame.insert(key, fitted);
                }
                if let Some((storage, v)) = through {
                    self.ref_alias_write(&storage, &v);
                }
                return changed;
            }
            // §7.4.1/§11.5.1: a nested select of a packed variable the frame
            // owns (`p[1][3:0] = v`, `a[i].f[3] = b`).
            if Self::select_of_select(lhs)
                && let Some(r) = self.frame_packed_select_write(lhs, val)
            {
                return r;
            }
        }
        match &lhs.kind {
            ExprKind::Ident(hier) => {
                // §7.2.1/§23.7: preserve the full registered struct path
                // before leaf-name fallback can select an unrelated observer.
                if !self.module.classes.is_empty()
                    && !self.module.packed_struct_fields.is_empty()
                    && hier.path.len() > 2
                    && hier.path.iter().all(|s| s.selects.is_empty())
                {
                    let dotted = hier
                        .path
                        .iter()
                        .map(|s| s.name.name.as_str())
                        .collect::<Vec<_>>()
                        .join(".");
                    let name = dotted
                        .strip_prefix(&format!("{}.", self.module.name))
                        .unwrap_or(&dotted);
                    if let Some((base, off, width)) = self.packed_leaf_of_hier(name)
                        && let Some(cur) = self.get_signal_value_by_name(&base)
                    {
                        let mut next = cur.clone();
                        // §7.3: a real union member stores real bits.
                        let real_member = width == 64
                            && !val.is_real
                            && name
                                .strip_prefix(&format!("{base}."))
                                .is_some_and(|f| self.union_real_member(&base, f));
                        let fitted = if real_member {
                            Value::from_f64(val.to_f64())
                        } else {
                            val.resize_for_assign(width)
                        };
                        let rhs = self.packed_member_piece(&base, name, fitted);
                        for bit in 0..width as usize {
                            next.set_bit(off as usize + bit, rhs.get_bit(bit));
                        }
                        if next == cur {
                            return false;
                        }
                        self.set_signal_value_by_name(&base, next);
                        return true;
                    }
                }
                // LRM §25.9: virtual-interface alias (task/function
                // formal frame, or a plain `virtual <iface>` variable) —
                // rewrite `vif.member` to `<bound>.member`. Must fire
                // before the local-stack/class-prop checks so `vif`
                // doesn't shadow a same-named local of unrelated type.
                if hier.path.len() >= 2 {
                    {
                        let first = hier.path[0].name.name.as_str();
                        if let Some(bound) = self.iface_alias_for(first) {
                            let rest = hier
                                .path
                                .iter()
                                .skip(1)
                                .map(|s| s.name.name.as_str())
                                .collect::<Vec<_>>()
                                .join(".");
                            let target = format!("{}.{}", bound, rest);
                            let prev = self.get_signal_value_by_name(&target);
                            let w = prev.as_ref().map(|v| v.width).unwrap_or(val.width);
                            let resized = val.resize_for_assign(w);
                            let changed = prev.as_ref() != Some(&resized);
                            self.set_signal_value_by_name(&target, resized);
                            return changed;
                        }
                    }
                }
                // A DOTTED local: an inlined-instance function's return
                // variable lives in the frame under its registered name
                // ("dut.z"), and the rewritten body assigns exactly that
                // dotted lvalue — which skipped the single-segment local
                // check below and leaked to the global map, so every
                // hier-named function returned its never-written init
                // (br_gh277b: x, formerly a masking 0).
                if hier.path.len() > 1 && hier.path.iter().all(|s| s.selects.is_empty()) {
                    if !self.local_stack.is_empty() {
                        let joined = hier
                            .path
                            .iter()
                            .map(|s| s.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join(".");
                        let last_idx = self.local_stack.len() - 1;
                        if self.local_stack[last_idx].contains_key(joined.as_str()) {
                            // §6.21/§10.7: a frame-local integral value keeps its
                            // declared width and signing on every store. Bare-name
                            // metadata may currently describe a different process's
                            // same-named declaration, so it cannot type this store.
                            let is_str = hier.path.last().is_some_and(|s| {
                                self.string_signals.contains(&s.name.name)
                                    || self.p_local_is_string(&s.name.name)
                            });
                            let fitted = if !val.is_real && !is_str {
                                if let Some(&target_w) = self.widths.get(&joined) {
                                    if val.width != target_w || val.is_fill {
                                        val.resize_for_assign(target_w)
                                    } else {
                                        val.clone()
                                    }
                                } else {
                                    val.clone()
                                }
                            } else if val.is_real
                                && !is_str
                                && let Some(p) = self.local_stack[last_idx].get(&joined)
                                && !p.is_real
                            {
                                // §6.12.2: a real into an integral leaf rounds.
                                Self::real_into_slot(p, val.to_f64())
                            } else {
                                val.clone()
                            };
                            self.local_stack[last_idx].insert(joined, fitted);
                            return true;
                        }
                    }
                }
                // §8.4: `tb.u_leaf.obj.field = v` writes the OBJECT.  The dotted
                // name used to fall through to the signal table, which resolves
                // the leaf flat and landed on tb's own `field` — the object kept
                // its old value while a read through a handle showed the new one.
                // Only taken when the parent really is a live object that declares
                // the member, so struct/net lvalues are untouched.
                if hier.path.len() >= 2 && hier.path.iter().all(|s| s.selects.is_empty()) {
                    if let Some(h) = self.hier_parent_handle(hier) {
                        let member_name = hier.path[hier.path.len() - 1].name.name.as_str();
                        if self.read_member_value(h, member_name).is_some()
                            || self
                                .static_member_target(h, None, None, member_name)
                                .is_some()
                        {
                            if let Some(ok) = self.static_write_via_handle(
                                h,
                                None,
                                None,
                                member_name,
                                val.clone(),
                            ) {
                                return ok;
                            }
                            let fitted = self.fit_class_prop(h, member_name, &val);
                            return self.set_prop_if_changed(h, member_name, fitted);
                        }
                    }
                }
                if hier.path.len() == 1 && hier.path[0].selects.is_empty() {
                    let name = &hier.path[0].name.name;
                    // Check local stack
                    if !self.local_stack.is_empty() {
                        let last_idx = self.local_stack.len() - 1;
                        if let Some((prev_is_real, local_width, local_signed)) = self.local_stack
                            [last_idx]
                            .get(name)
                            .map(|p| (p.is_real, p.width, p.is_signed))
                        {
                            // §10.7: on assignment to a sized local, resize
                            // the RHS to the local's DECLARED width so a
                            // narrow signed literal sign-extends (e.g.
                            // `logic [63:0] mask = -1` must yield all-ones).
                            // The declared width is read from the `widths`
                            // table (populated at VarDecl time), NOT from the
                            // currently-stored value — the stored width drifts
                            // as differently-sized RHS values flow through and
                            // would corrupt string/handle locals. If no
                            // declared width is recorded (method formals,
                            // pattern bindings), leave the value untouched.
                            //
                            // WIDTH truncation: a value WIDER than the
                            // declared width is TRUNCATED (e.g. `byte b; b =
                            // $urandom` keeps only the low 8 bits). STRING
                            // locals are exempt (1024-bit internal packed
                            // text).
                            let is_str = self.p_local_is_string(name);
                            let fitted = if prev_is_real && self.module.var_decl_types.get(name).is_some_and(|dt| matches!(
                                self.resolve_dt_ref(dt), DataType::Real { kind: crate::ast::types::RealType::ShortReal, .. }
                            )) {
                                Value::from_f64(val.to_f64() as f32 as f64)
                            } else if prev_is_real && !val.is_real && !is_str {
                                // §6.12.2: an integral value assigned to a REAL
                                // local converts; stored as integral bits, a
                                // `real t; t = k; h = t / 2;` divided as integers.
                                Value::from_f64(val.to_f64())
                            } else if !val.is_real && !is_str {
                                if local_width > 0 {
                                    let target_w = local_width;
                                    let mut f = if val.width != target_w || val.is_fill {
                                        val.resize_for_assign(target_w)
                                    } else {
                                        val.clone()
                                    };
                                    // §6.11.1: storage keeps the LOCAL'S
                                    // declared signedness, not the rvalue's —
                                    // same rule the id-signal path applies.
                                    // `for (i = 0; ...)` on a `bit [3:0] i`
                                    // otherwise stored the unsized literal's
                                    // SIGNED stamp, and the counter wrapped
                                    // negative at 8 (ivtest sv_foreach3 hang).
                                    f.is_signed = local_signed;
                                    f
                                } else {
                                    val.clone()
                                }
                            } else if val.is_real && !is_str && !prev_is_real {
                                // §6.12.2: a real assigned to an INTEGRAL local
                                // rounds to the local's type — the module-variable
                                // path did, the frame local kept the real, so an
                                // `int baudDivisor = freq / rate;` compared as
                                // 32.55 forever (uart AVIP baud clock never toggled).
                                let (w, signed) = self.local_stack[last_idx]
                                    .get(name)
                                    .map(|p| (p.width.max(1), p.is_signed))
                                    .unwrap_or((32, true));
                                let mut f = Self::real_to_int(val.to_f64(), w);
                                f.is_signed = signed;
                                f
                            } else {
                                val.clone()
                            };
                            self.local_stack[last_idx].insert(name.clone(), fitted.clone());
                            // §6.21: write a `static` subroutine local through
                            // to its live shared cell so other simultaneous
                            // activations (recursive phase re-entry / method)
                            // observe the mutation immediately. Skipped during
                            // elaboration-time constant-function evaluation,
                            // where each call is independent.
                            if !self.in_const_param_eval
                                && self.static_local_names.contains(name.as_str())
                                && let Some(k) = self.static_local_key_for(name)
                                && self.static_local_keys.contains(&k)
                            {
                                self.static_local_vars.insert(k, fitted);
                            }
                            return true;
                        }
                    }
                    // §8.9/§8.14: a shadowed name whose nearest declaration
                    // from the executing method's class is a STATIC is that
                    // class's shared cell, though a derived class of the
                    // object redeclares it as an instance property.
                    if self.prop_maybe_shadowed(name)
                        && let Some(ctx) = self.lexical_class()
                        && self.member_is_static_prop(&ctx, name)
                        && self.class_static_set(&ctx, name, val.clone())
                    {
                        return true;
                    }
                    // Check 'this' properties
                    if let Some(Some(handle)) = self.this_stack.last() {
                        let handle = *handle;
                        // §8.10: a base method's write to a shadowed name must
                        // land in the base's own copy, or it silently leaks
                        // into the derived's.
                        let key = self
                            .shadowed_prop_key(name, false)
                            .unwrap_or_else(|| name.clone());
                        let holds = self
                            .heap
                            .get(handle)
                            .and_then(|o| o.as_ref())
                            .is_some_and(|i| i.properties.contains_key(&key));
                        if holds {
                            let fitted = self.fit_class_prop(handle, &key, val);
                            if let Some(Some(instance)) = self.heap.get_mut(handle) {
                                instance.properties.insert(key, fitted);
                                return true;
                            }
                        }
                    }
                    // Static class property assigned bare inside a method.
                    if let Some(Some(ctx)) = self.class_context_stack.last().cloned() {
                        let nm = name.clone();
                        if self.class_static_set(&ctx, &nm, val.clone()) {
                            return true;
                        }
                    }
                } else if hier.path.len() > 1 {
                    // Property write through a class handle evaluated from the prefix
                    // (e.g. `stor.handle.val = 777`). Not for a hierarchical
                    // signal (`u_sub.sig`): its prefix is an instance, whose
                    // value is no handle.
                    if hier.path.len() >= 2
                        && !self
                            .signal_name_to_id
                            .contains_key(self.resolve_hier_name(hier).as_ref())
                    {
                        let prop_name = &hier.path.last().unwrap().name.name;
                        // id-collision-in-heap: the head's VALUE is a heap
                        // handle only when the head statically names one — a
                        // class-typed root (`h.direct`), a frame-dotted local
                        // member holding a handle (`stor.handle`), a
                        // class-typed composed signal (`mod.h`), or a
                        // handle-collection element (`ch[0]`). Struct or
                        // hierarchical heads (`ov.inner`, `u.sv`, `u_if.iv`,
                        // `q[0]` of struct elements) splice their members into
                        // integral values that must never be read back as
                        // handles.
                        let head_hier = hier_prefix(hier);
                        if self.head_denotes_class_object(&head_hier) {
                            let head_expr = Expression::new(ExprKind::Ident(head_hier), lhs.span);
                            let base_val = self.eval_expr(&head_expr);
                            let handle = base_val.to_u64().unwrap_or(0) as usize;
                            if handle != 0
                                && handle < self.heap.len()
                                && self.heap[handle].is_some()
                            {
                                let fitted = self.fit_class_prop(handle, prop_name, val);
                                return self.set_prop_if_changed(handle, prop_name, fitted);
                            }
                        }
                    }
                    // `ClassName::static_prop` — explicit static property write.
                    if hier.path.len() == 2 {
                        let cls = hier.path[0].name.name.clone();
                        let prop = hier.path[1].name.name.clone();
                        if self.module.classes.contains_key(&cls)
                            && !self
                                .signal_name_to_id
                                .contains_key(self.resolve_hier_name(hier).as_ref())
                            && self.class_static_set(&cls, &prop, val.clone())
                        {
                            return true;
                        }
                        // §8.25/§6.18: `alias_t::prop` through a typedef of a
                        // specialization writes THAT specialization's cell,
                        // the one its reads (and `C#(args)::prop`) name.
                        if !self.module.classes.contains_key(&cls)
                            && !self.signal_name_to_id.contains_key(cls.as_str())
                            && let Some((base, sig)) = self.scope_prefix_spec(&cls)
                        {
                            let saved = self.current_spec.replace((base.clone(), sig));
                            let ok = self.class_static_set(&base, &prop, val.clone());
                            self.current_spec = saved;
                            if ok {
                                return true;
                            }
                        }
                    }
                    // `ClassName::static_obj_handle.member...` (3+ segments):
                    // the head names a class whose STATIC property (segment 1)
                    // holds an object handle; the remaining segments are member
                    // writes on that object. Needed for
                    // `ClassName::obj.member = new(...)` / `= handle` so the
                    // write reaches the real heap object reached through the
                    // static-property chain.
                    if hier.path.len() >= 3 && hier.path.iter().all(|s| s.selects.is_empty()) {
                        let cls = &hier.path[0].name.name;
                        let sprop = &hier.path[1].name.name;
                        if self.module.classes.contains_key(cls)
                            && self.static_prop_key(cls, sprop).is_some()
                        {
                            let cur0 = self
                                .static_prop_key(cls, sprop)
                                .and_then(|k| self.class_statics.get(&k))
                                .and_then(|v| v.to_u64())
                                .map(|h| h as usize)
                                .unwrap_or(0);
                            if cur0 != 0 && cur0 < self.heap.len() {
                                let mut cur = cur0;
                                let mut done = false;
                                for i in 2..hier.path.len() {
                                    if cur == 0 || cur >= self.heap.len() {
                                        break;
                                    }
                                    let mname = hier.path[i].name.name.clone();
                                    if i == hier.path.len() - 1 {
                                        // If the final member is a STATIC
                                        // property of the reached object's
                                        // class, write the shared static cell
                                        // (the read path resolves it there).
                                        // Otherwise write the instance property.
                                        let cn = self
                                            .heap
                                            .get(cur)
                                            .and_then(|o| o.as_ref())
                                            .map(|inst| inst.class_name.clone());
                                        if let Some(cn) = cn {
                                            if self.static_prop_key(&cn, &mname).is_some()
                                                && self.class_static_set(&cn, &mname, val.clone())
                                            {
                                                done = true;
                                            }
                                        }
                                        if !done {
                                            let fitted = self.fit_class_prop(cur, &mname, val);
                                            if let Some(Some(inst)) = self.heap.get_mut(cur) {
                                                inst.properties.insert(mname, fitted);
                                                done = true;
                                            }
                                        }
                                    } else if let Some(h) = self.member_handle(cur, &mname) {
                                        cur = h;
                                    } else {
                                        break;
                                    }
                                }
                                if done {
                                    return true;
                                }
                            }
                        }
                    }
                    let obj_name = &hier.path[0].name.name;
                    {
                        let sub = hier.path[1..]
                            .iter()
                            .map(|s| s.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join(".");
                        if let Some(fields) =
                            self.module.packed_struct_fields.get(obj_name).cloned()
                        {
                            if let Some((_, off, w)) =
                                fields.iter().find(|(m, _, _)| m == &sub).cloned()
                            {
                                // The container may be a procedural LOCAL (a
                                // packed struct declared inside a task), which
                                // `get/set_signal_value_by_name` cannot see.
                                if let Some(cur_sig) = self.get_local_or_signal(obj_name) {
                                    let total_w = cur_sig.width;
                                    let mut cur = cur_sig.resize(total_w);
                                    let piece = val.resize_for_assign(w);
                                    for i in 0..w {
                                        let bit = piece.get_bit(i as usize);
                                        cur.set_bit((off + i) as usize, bit);
                                    }
                                    let prev = self.get_local_or_signal(obj_name);
                                    let changed = prev.as_ref() != Some(&cur);
                                    self.set_local_or_signal(obj_name, cur);
                                    return changed;
                                }
                            }
                        }
                    }
                    let obj_val = if let Some(locals) = self.local_stack.last() {
                        locals
                            .get(obj_name)
                            .cloned()
                            .or_else(|| self.get_signal_value_by_name(obj_name))
                    } else {
                        self.get_signal_value_by_name(obj_name)
                    };
                    // Mirror SV writes to the unpacked-struct member signal
                    // (e.g. `t_unpacked_struct.val_i`) so VPI-forced values
                    // and SV reads stay in sync. Both storage locations
                    // (heap property and separate signal) are written; the
                    // read path prefers the separate signal.
                    if hier.path.len() == 2 {
                        let dotted =
                            format!("{}.{}", hier.path[0].name.name, hier.path[1].name.name);
                        if self.signal_name_to_id.contains_key(dotted.as_str()) {
                            if let Some(id) = self.signal_name_to_id.get(dotted.as_str()).copied() {
                                let width = self.signal_widths[id];
                                let mut resized = if self.signal_real[id] {
                                    if val.is_real {
                                        val.clone()
                                    } else {
                                        Value::from_f64(val.to_f64())
                                    }
                                } else {
                                    if val.is_real {
                                        Self::real_to_int(val.to_f64(), width)
                                    } else {
                                        val.resize_for_assign(width)
                                    }
                                };
                                resized.is_signed = self.signal_signed[id];
                                self.set_signal_value_by_name(&dotted, resized);
                                // Fall through to also update the heap so any
                                // existing read path that looks there stays
                                // consistent. Don't `return` — let the heap
                                // write happen too.
                            }
                        }
                    }
                    if let Some(v) = obj_val {
                        let mut cur_handle = v.to_u64().unwrap_or(0) as usize;
                        for i in 1..hier.path.len() {
                            if cur_handle == 0 || cur_handle >= self.heap.len() {
                                break;
                            }
                            let member_name = &hier.path[i].name.name;
                            if i == hier.path.len() - 1 {
                                let holds = self
                                    .heap
                                    .get(cur_handle)
                                    .and_then(|o| o.as_ref())
                                    .is_some_and(|inst| inst.properties.contains_key(member_name));
                                // id-collision-in-heap: the walk may reinterpret
                                // the head's integral value as a heap handle
                                // only when the head statically names a
                                // class-object lvalue — a struct variable or a
                                // struct-element collection splices its members
                                // into plain data (§7.8) that must stay out of
                                // the heap.
                                if holds && self.bare_name_is_class_channel(obj_name) {
                                    let fitted = self.fit_class_prop(cur_handle, member_name, val);
                                    if let Some(Some(inst)) = self.heap.get_mut(cur_handle) {
                                        inst.properties.insert(member_name.clone(), fitted);
                                        return true;
                                    }
                                }
                                break;
                            }
                            if let Some(Some(inst)) = self.heap.get(cur_handle) {
                                if let Some(mval) = inst.properties.get(member_name) {
                                    cur_handle = mval.to_u64().unwrap_or(0) as usize;
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                    }
                    // FALLBACK: If the full dotted name exists as a separate signal
                    // (xezim's elaboration scheme for unpacked struct members),
                    // write to it directly.
                    let dotted_name = hier
                        .path
                        .iter()
                        .map(|s| s.name.name.as_str())
                        .collect::<Vec<_>>()
                        .join(".");
                    // Only a name WITHOUT a table slot lives in the name-keyed
                    // copy alone. After `sync_table_to_hashmap` (a `$monitor`,
                    // a dump, a `$readmemh`) every table signal has a copy
                    // there too; writing that copy loses the write at the next
                    // sync while every read uses the table (#568, §10.4.1).
                    if !self.signal_name_to_id.contains_key(dotted_name.as_str())
                        && let Some(existing_val) = self.signals.get(&dotted_name).cloned()
                    {
                        let w = existing_val.width;
                        let is_real =
                            self.real_signals.contains(&dotted_name) || existing_val.is_real;
                        let mut resized = if is_real {
                            if val.is_real {
                                val.clone()
                            } else {
                                Value::from_f64(val.to_f64())
                            }
                        } else if val.is_real {
                            // §6.12.2: round to the nearest, not truncate.
                            Self::real_to_int(val.to_f64(), w.max(1))
                        } else {
                            val.resize_for_assign(w)
                        };
                        resized.is_signed = self.signed_signals.contains(&dotted_name);
                        let changed = existing_val != resized;
                        if changed {
                            self.mark_dirty(&dotted_name);
                        }
                        self.signals.insert(dotted_name, resized);
                        return changed;
                    }
                }
                let is_ambiguous_leaf =
                    hier.path.len() == 1 && !hier.path[0].name.name.contains('.');
                if let Some(id) = hier.cached_signal_id.get() {
                    if !is_ambiguous_leaf {
                        let resized = self.fit_value_to_signal(id, val);
                        let changed = self.signal_table[id] != resized;
                        if changed {
                            self.mark_dirty_id(id);
                            write_sig!(self, id, resized);
                            self.table_modified = true;
                        }
                        return changed;
                    }
                }
                let name = self.resolve_hier_name(hier);
                if let Some(&id) = self.signal_name_to_id.get(name.as_ref()) {
                    hier.cached_signal_id.set(Some(id));
                    let resized = self.fit_value_to_signal(id, val);
                    let changed = self.signal_table[id] != resized;
                    if changed {
                        self.mark_dirty_id(id);
                        write_sig!(self, id, resized);
                        self.table_modified = true;
                    }
                    return changed;
                }
                // Fallback: signal not in signal_name_to_id (truly unknown,
                // e.g. testbench probe wires that didn't elaborate). Write
                // directly into the legacy `signals` HashMap without first
                // forcing a 35M-entry sync from signal_table — sync was the
                // dominant cost (20s × ~40 calls = 14 minutes) of c910 time-0
                // settle. We only write here, never read from self.signals
                // before this path, so the sync was unnecessary.
                // A 0 width here means the flat `widths` map was polluted
                // by a same-named variable from another scope — never a
                // valid lvalue width, so fall back to the value's width.
                // A member of a nested packed struct / untagged union aliases the
                // container's storage (§7.3): splice into it rather than create an
                // independent leaf that would stop aliasing its siblings. Its dotted
                // name parses as one hierarchical ident, so the root-keyed field
                // lookups above never matched it.
                if let Some((base, off, w)) = self.packed_leaf_of_hier(&name) {
                    if let Some(cur) = self.get_signal_value_by_name(&base) {
                        let mut next = cur.clone();
                        let mut piece = val.resize_for_assign(w);
                        if !self.mixed_packed_vars.is_empty() {
                            piece = self.packed_member_piece(&base, &name, piece);
                        }
                        for i in 0..w {
                            next.set_bit((off + i) as usize, piece.get_bit(i as usize));
                        }
                        let changed = next != cur;
                        if changed {
                            self.set_signal_value_by_name(&base, next);
                        }
                        return changed;
                    }
                }
                // LRM §10.6/§10.6.1: a target under an active force or
                // procedural continuous assign ignores ordinary procedural
                // writes until the matching release/deassign.
                if self.forced_names.contains(&*name) {
                    return false;
                }
                let width = self
                    .widths
                    .get(&*name)
                    .copied()
                    .filter(|w| *w > 0)
                    .unwrap_or(val.width);
                let is_real = self.real_signals.contains(&*name);
                // §6.16: a string has no declared length — for a `string`
                // LOCAL the widths-map entry is the 1024-bit placeholder
                // `resolve_type_width` hands the dynamic type, and resizing to
                // it truncates the FRONT of the text (a packed string keeps
                // its text in the low bits). Module-level strings are exempt
                // via `signal_is_string` on the signal-table path; this
                // map-backed store is where PROCEDURAL LOCALS land, so exempt
                // them here too. Issue #64: a 129-char `$system` command came
                // back as 128 chars shifted one byte left ("echo …" → "cho …").
                let is_str = self.string_signals.contains(&*name)
                    || hier
                        .path
                        .last()
                        .is_some_and(|s| self.string_signals.contains(&s.name.name));
                let mut resized = if is_real {
                    if val.is_real {
                        val.clone()
                    } else {
                        Value::from_f64(val.to_f64())
                    }
                } else if is_str && !val.is_real {
                    val.clone()
                } else {
                    if val.is_real {
                        Self::real_to_int(val.to_f64(), width)
                    } else {
                        val.resize_for_assign(width)
                    }
                };
                resized.is_signed = self.signed_signals.contains(&*name);
                let changed = self.signals.get(&*name).is_none_or(|p| *p != resized);
                if changed {
                    self.mark_dirty(&name);
                }
                self.signals.insert(name.to_string(), resized.clone());

                // If this is an array or queue, and we are assigning a packed value,
                // we might want to split it into elements.
                if env_set_cached!("XEZIM_A1_DBG") && name.contains('m') {
                    eprintln!(
                        "[A1DBG] whole-name write name={:?} in_arrays={} hint={:?}",
                        name,
                        self.module.arrays.contains_key(&*name),
                        self.name_resolve_hint.borrow().clone()
                    );
                }
                if let Some((lo, hi, elem_width)) = self.module.arrays.get(&*name).cloned() {
                    let num_elements = (resized.width / elem_width) as usize;
                    // For queues/dynamic arrays, we update the size
                    let is_dynamic = hi < lo || hi == 63 && lo == 0; // simplistic check for [lo:hi] vs []/[$]
                    if is_dynamic {
                        self.signals.insert(
                            format!("{}.size", name),
                            Value::from_u64(num_elements as u64, 32),
                        );
                    }
                    for i in 0..num_elements {
                        let l = (num_elements - 1 - i) * elem_width as usize
                            + (elem_width as usize - 1);
                        let r = (num_elements - 1 - i) * elem_width as usize;
                        let elem_val = resized.range_select(l, r);
                        self.signals.insert(format!("{}[{}]", name, i), elem_val);
                    }
                }

                changed
            }
            ExprKind::Index { expr, index } => self.assign_value_index(lhs, val, expr, index),
            ExprKind::RangeSelect {
                expr,
                left,
                right,
                kind,
            } => {
                // §7.4.1/§11.5.1: a part of a vector the call frame holds, at
                // the bits its declared range labels.
                if !self.local_stack.is_empty()
                    && let Some(r) =
                        self.frame_vector_select_write(expr, Some(*kind), left, Some(right), val)
                {
                    return r;
                }
                // §11.5.1: a part-select bound may be NEGATIVE, and a write
                // through a partially out-of-range select stores only the bits
                // that DO land in the vector (`bit [3:0] x; x[4:-1] <= 6'b101010`
                // leaves x = 4'b0101). Reading the bounds through `to_u64` made
                // -1 a huge unsigned index, so `l + r - 1` overflowed and
                // aborted the run. `to_i64` sign-extends only a value whose
                // type IS signed, so ordinary indices are untouched.
                // §11.5.1: an index containing x or z selects nothing — the
                // write is DISCARDED, not applied at bit 0. `to_i64` reports
                // that as None; folding it to 0 silently wrote the low bits
                // (`i = 'hx; x[i +: 2] = 2'b11` left x = 4'b0011).
                let lval_b = self.eval_expr(left);
                let rval_b = self.eval_expr(right);
                if lval_b.has_xz() || rval_b.has_xz() {
                    return false;
                }
                let (Some(mut li), Some(mut ri)) = (lval_b.to_i64(), rval_b.to_i64()) else {
                    return false;
                };
                // Offset a part-select's bit indices by the declared lower
                // bound of a PLAIN packed vector (`logic [7:4] v; v[6:5]` ->
                // storage bits [2:1]), mirroring the read path (LRM §7.4.1 /
                // §11.5.1). Without this `w[3:2] = 2'b11` on `logic [3:1] w`
                // wrote storage bits [3:2] (one off the top of a 3-bit word)
                // instead of [2:1], landing on the wrong bits.
                //
                // The multi-D element branch below (packed_signal_elem_widths)
                // applies its OWN lo_b normalization inside `lsb_of`, and the
                // ascending / unpacked-array branches index by element rather
                // than bit — so each of those must keep the RAW indices and is
                // excluded here. Bit-selects are already correct in both
                // directions; this closes the part-select WRITE gap.
                let mut elem_ascending_dim: Option<(i64, i64)> = None;
                // §7.4.1: a multidimensional packed class property, or an
                // associative-array element of such a type, selects elements
                // of its first dimension; neither has the signal tables the
                // branches below read its geometry from.
                let extra_dims = (!self.no_class_objects())
                    .then(|| self.class_prop_receiver(expr))
                    .flatten()
                    .and_then(|(h, p)| self.class_prop_packed_dims(h, &p))
                    .or_else(|| {
                        let ExprKind::Index { expr: root, .. } = &expr.kind else {
                            return None;
                        };
                        let ExprKind::Ident(h) = &root.kind else {
                            return None;
                        };
                        let name = self.resolve_hier_name(h);
                        self.is_associative_array(&name)
                            .then(|| self.module.packed_full_dims.get(&*name).cloned())
                            .flatten()
                    })
                    .filter(|d| d.len() > 1);
                let physical = extra_dims.as_ref().map(|dims| {
                    let (dl, dr) = dims[0];
                    let stride: i64 = dims[1..].iter().map(|(l, r)| (l - r).abs() + 1).product();
                    let (a, b) = match kind {
                        RangeKind::Constant => (li, ri),
                        RangeKind::IndexedUp => (li, li + ri - 1),
                        RangeKind::IndexedDown => (li, li - ri + 1),
                    };
                    let bit = |i: i64| {
                        if dl >= dr {
                            (i - dr) * stride
                        } else {
                            (dr - i) * stride
                        }
                    };
                    (bit(a).min(bit(b)), bit(a).max(bit(b)) + stride - 1)
                });
                let class_dim = if physical.is_none() {
                    self.class_prop_select_dim(expr)
                } else {
                    None
                };
                if let Some((dl, dr)) = class_dim {
                    if dl < dr {
                        li = match kind {
                            RangeKind::Constant => dr - li,
                            RangeKind::IndexedUp => dr - li - (ri - 1),
                            RangeKind::IndexedDown => dr - li + (ri - 1),
                        };
                        if matches!(kind, RangeKind::Constant) {
                            ri = dr - ri;
                        }
                    } else {
                        li -= dr;
                        if matches!(kind, RangeKind::Constant) {
                            ri -= dr;
                        }
                    }
                } else if let ExprKind::Ident(h) = &expr.kind {
                    let nm = self.resolve_hier_name(h);
                    let is_multi_d = self
                        .module
                        .packed_signal_elem_widths
                        .get(&*nm)
                        .copied()
                        .unwrap_or(0)
                        > 1;
                    let is_ascending = self.module.ascending_packed.contains_key(&*nm);
                    let is_unpacked = self.module.arrays.contains_key(&*nm);
                    if !is_multi_d && !is_ascending && !is_unpacked {
                        if let Some(&(dl, dr)) = self
                            .module
                            .packed_full_dims
                            .get(&*nm)
                            .and_then(|d| d.first())
                        {
                            let lo_b = dl.min(dr);
                            if lo_b != 0 {
                                // Offset the base/lo INDEX only. In
                                // `[base +: W]` / `[base -: W]` the right
                                // operand is a WIDTH, not an index, so it must
                                // not be shifted — only the constant `[msb:lsb]`
                                // form has two indices (LRM §7.4.1 / §11.5.1).
                                li -= lo_b;
                                if matches!(kind, RangeKind::Constant) {
                                    ri -= lo_b;
                                }
                            }
                        }
                    }
                } else if let Some((dl, dr)) = self.select_base_elem_dim(expr) {
                    // ELEMENT of an unpacked array/queue/assoc, same mapping
                    // as the read side: descending offsets the labels here;
                    // an ascending dim mirrors msb/lsb after they are formed
                    // (the plain-Ident ascending block below stays untouched).
                    if dl >= dr {
                        let lo_b = dr;
                        if lo_b != 0 {
                            li -= lo_b;
                            if matches!(kind, RangeKind::Constant) {
                                ri -= lo_b;
                            }
                        }
                    } else {
                        elem_ascending_dim = Some((dl, dr));
                    }
                }
                let (msb_i, lsb_i): (i64, i64) = match kind {
                    RangeKind::Constant => (li.max(ri), li.min(ri)),
                    RangeKind::IndexedUp => (li + (ri - 1).max(0), li),
                    RangeKind::IndexedDown => (li, li - (ri - 1).max(0)),
                };
                // The SOURCE bit for target bit `i` is `i - lsb_i`, which stays
                // signed: with lsb_i = -1 the vector's bit 0 takes source bit 1.
                let mut src_base = lsb_i;
                let mut msb_i = msb_i;
                // Ascending vector: the value's bit 0 lands on the select's
                // LAST label, which is its LOWEST physical bit, so source
                // bits count up from `phys_lo` — the same as a descending
                // select. Kept for the partial-OOB case where the physical
                // range is clamped: the loop needs the UNCLAMPED high end.
                let mut asc_phys_hi: Option<i64> = None;
                let (mut msb, mut lsb) = (msb_i.max(0) as usize, lsb_i.max(0) as usize);
                // §7.4.1 element-range WRITE on a packed multi-D base:
                // `wv[2:1] = 16'hBEEF` / `wv[1 +: 2] = …` targets whole
                // ELEMENTS. The bit-based msb/lsb above wrote a 2-bit sliver
                // instead. Scale by the registered element width, normalized
                // against the declared outer dimension (mirrors the read side).
                let mut elem_scaled = false;
                if let ExprKind::Ident(h) = &expr.kind {
                    let nm = self.resolve_hier_name(h);
                    if let Some(&elem_w) = self
                        .module
                        .packed_signal_elem_widths
                        .get(&*nm)
                        .filter(|&&w| w > 1)
                    {
                        let (lab_a, lab_b): (i64, i64) = match kind {
                            RangeKind::Constant => (li, ri),
                            RangeKind::IndexedUp => (li, li + ri - 1),
                            RangeKind::IndexedDown => (li, li - (ri - 1)),
                        };
                        let dim = self
                            .module
                            .packed_full_dims
                            .get(&*nm)
                            .and_then(|d| d.first())
                            .copied();
                        let lsb_of = |idx: i64| -> i64 {
                            match dim {
                                None => idx * elem_w as i64,
                                Some((dl, dr)) => {
                                    let (lo_b, hi_b) = (dl.min(dr), dl.max(dr));
                                    let count = hi_b - lo_b + 1;
                                    let off = idx - lo_b;
                                    let slot = if dl >= dr { off } else { count - 1 - off };
                                    slot * elem_w as i64
                                }
                            }
                        };
                        let la = lsb_of(lab_a);
                        let lb = lsb_of(lab_b);
                        // §11.5.1: map signed labels before clipping, retaining
                        // the source offset for partially out-of-range writes.
                        src_base = la.min(lb);
                        msb_i = la.max(lb) + elem_w as i64 - 1;
                        lsb = src_base.max(0) as usize;
                        msb = msb_i.max(0) as usize;
                        elem_scaled = true;
                    }
                }
                if let Some((lo, hi)) = physical {
                    src_base = lo;
                    msb_i = hi;
                    lsb = lo.max(0) as usize;
                    msb = hi.max(0) as usize;
                    elem_scaled = true;
                }
                // Ascending packed vector part-write (`logic [0:7] pa; pa[0:3]=v`):
                // labels index from the MSB end → internal [(W-1)-lsb : (W-1)-msb]
                // (LRM §7.4.1, §11.5.1).
                if !elem_scaled {
                    if let ExprKind::Ident(h) = &expr.kind {
                        let nm = self.resolve_hier_name(h);
                        if let Some((_, hi_l)) = self.module.ascending_packed.get(&*nm).copied() {
                            // Labels were kept raw above (no low-bound offset
                            // for ascending vectors); label p is physical bit
                            // `high - p`, so the select's high label is its
                            // LOW physical bit.
                            // The bounds reaching here are already resolved to a
                            // [msb:lsb] pair for every RangeKind, so one mapping
                            // serves the constant and indexed forms alike.
                            // Restricting it to the constant form wrote
                            // `aw[4 +: 4]` at the DESCENDING position.
                            // A label past the LSB gives a NEGATIVE physical
                            // low bit: keep it in `src_base` (the store loop
                            // maps source bits from there, as for a negative
                            // constant bound) and clamp only the loop bounds.
                            // A label before the MSB overruns the width; the
                            // loop's `min(width - 1)` drops those bits.
                            let phys_lo = hi_l - msb_i;
                            let phys_hi = hi_l - lsb_i;
                            src_base = phys_lo;
                            msb = phys_hi.max(0) as usize;
                            lsb = phys_lo.max(0) as usize;
                            msb_i = phys_hi;
                            asc_phys_hi = Some(phys_hi);
                        }
                    } else if let Some((dl, dr)) = elem_ascending_dim {
                        // Ascending ELEMENT dim (`logic [0:23] a[0:0];
                        // a[0][0:7] = v`): labels mirror against the right
                        // bound — physical = right - label — so the formed
                        // [msb:lsb] label pair swaps and shifts as one.
                        let _ = dl;
                        let (new_msb, new_lsb) = (
                            (dr - lsb as i64).max(0) as usize,
                            (dr - msb as i64).max(0) as usize,
                        );
                        msb = new_msb;
                        lsb = new_lsb;
                    }
                }
                // Unpacked array slice assignment: copy element-by-element
                if let ExprKind::Ident(hier) = &expr.kind {
                    let name = self.resolve_hier_name(hier);
                    if let Some(&(arr_lo, arr_hi, elem_w)) = self.module.arrays.get(&*name) {
                        let count = msb + 1 - lsb;
                        let descending = self.module.descending_arrays.contains(&*name);
                        let mut changed = false;
                        let _ = (arr_lo, descending);
                        for i in 0..count {
                            let lhs_idx = (lsb + i) as i64;
                            if lhs_idx < arr_lo || lhs_idx > arr_hi {
                                continue;
                            }
                            let elem_name = format!("{}[{}]", name, lhs_idx);
                            let new_val = if (val.width as usize) == count * elem_w as usize {
                                val.range_select((i + 1) * elem_w as usize - 1, i * elem_w as usize)
                            } else {
                                val.clone()
                            };
                            if self.get_signal_value_by_name(&elem_name).as_ref() != Some(&new_val)
                            {
                                self.set_signal_value_by_name(&elem_name, new_val);
                                changed = true;
                            }
                        }
                        return changed;
                    }
                }
                // A selected element of a fixed class-property array is stored
                // as an instance-scoped value rather than a signal-table slot.
                // Apply the part write to that element before the generic
                // signal-id path, which cannot resolve the bare property name.
                if let ExprKind::Index {
                    expr: array_expr,
                    index,
                } = &expr.kind
                {
                    let index_value = self.eval_expr(index);
                    if index_value.has_xz() && !index_value.is_real {
                        return false;
                    }
                    // §7.8.6: a select write to an ABSENT associative element
                    // reads it as the element type's default, replaces the
                    // selected bits and writes it back, creating it; the
                    // write was dropped. Its key takes the array's key form.
                    let mut assoc_default: Option<Value> = None;
                    let element_name = self.class_nd_elem_name(array_expr, index).or_else(|| {
                        // A module-scope or block-local array is not an
                        // object member: resolve the plain name too.
                        let base = self.expr_assoc_name(array_expr).or_else(|| {
                            let ExprKind::Ident(h) = &array_expr.kind else {
                                return None;
                            };
                            let n = self.resolve_hier_name(h).into_owned();
                            let n = self.instance_assoc_member(&n).unwrap_or(n);
                            self.is_associative_array(&n).then_some(n)
                        })?;
                        if self.is_associative_array(&base) {
                            let key = self.assoc_key_str(&base, &index_value);
                            assoc_default = self.assoc_missing_elem_value(&base);
                            return Some(format!("{}[{}]", base, key));
                        }
                        let index = index_value.to_i64()?;
                        Some(format!("{}[{}]", base, index))
                    });
                    if let Some(element_name) = element_name {
                        if let Some(mut current) = self
                            .get_signal_value_by_name(&element_name)
                            .or(assoc_default)
                        {
                            let width = current.width as usize;
                            let hi = msb.min(width.saturating_sub(1));
                            if oob_write_discard(msb_i, src_base, width as u32) {
                                return false;
                            }
                            let base = if src_base < 0 { src_base } else { lsb as i64 };
                            let mut changed = false;
                            if msb_i >= 0 && hi >= lsb {
                                for bit in lsb..=hi {
                                    let source = bit as i64 - base;
                                    if source < 0 {
                                        continue;
                                    }
                                    let next = val.get_bit(source as usize);
                                    if current.get_bit(bit) != next {
                                        current.set_bit(bit, next);
                                        changed = true;
                                    }
                                }
                            }
                            if changed || !self.signals.contains_key(&element_name) {
                                self.set_signal_value_by_name(&element_name, current);
                            }
                            return changed;
                        }
                    }
                }
                // §11.5.1: splice a selected class property back into the
                // object, including static and nested hierarchical receivers.
                if let ExprKind::Ident(hier) = &expr.kind
                    && let Some(h) = self.hier_parent_handle(hier)
                {
                    let member = &hier.path.last().unwrap().name.name;
                    if let Some(mut cur) = self
                        .static_read_via_handle(h, None, None, member)
                        .or_else(|| self.read_member_value(h, member))
                    {
                        if oob_write_discard(msb_i, src_base, cur.width) {
                            return false;
                        }
                        let rhs = val.resize_for_assign((msb_i - src_base + 1).max(1) as u32);
                        for bit in lsb..=msb.min(cur.width.saturating_sub(1) as usize) {
                            cur.set_bit(bit, rhs.get_bit((bit as i64 - src_base) as usize));
                        }
                        return self.set_prop_if_changed(h, member, cur);
                    }
                }
                // Fast path: resolve target signal_id directly (avoids format!
                // + HashMap lookup on every call for tight memory-init loops).
                let target_id: Option<usize> = match &expr.kind {
                    ExprKind::Ident(hier) => {
                        if self.bare_selected_storage_shadows_signal(hier) {
                            None
                        } else if let Some(id) = hier.cached_signal_id.get() {
                            Some(id)
                        } else {
                            let name = self.resolve_hier_name(hier);
                            if let Some(&id) = self.signal_name_to_id.get(name.as_ref()) {
                                hier.cached_signal_id.set(Some(id));
                                Some(id)
                            } else {
                                None
                            }
                        }
                    }
                    ExprKind::Index {
                        expr: arr_expr,
                        index,
                    } => {
                        if let ExprKind::Ident(hier) = &arr_expr.kind {
                            let name = self.resolve_hier_name(hier);
                            if self.module.arrays.contains_key(&*name) {
                                // §7.4.6: SIGNED index — a negative-lo array
                                // (`m[-2:1]`) loses its negative elements when
                                // the index goes through to_u64.
                                let idx = self.eval_expr(index).to_i64().unwrap_or(0);
                                self.get_array_elem_id(&name, idx)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(id) = target_id {
                    // Packed cell: no signal_table slot, so the whole-word
                    // path below cannot apply. Splice the bits into the cell.
                    if is_packed_id(id) {
                        let w = self.packed.width(id);
                        let mut cur = self.packed.read(id);
                        let hi = (msb as usize).min(w.saturating_sub(1) as usize);
                        if oob_write_discard(msb_i, src_base, w) {
                            return false;
                        }
                        for i in lsb..=hi {
                            let src = if src_base < 0 {
                                i as i64 + src_base
                            } else {
                                i as i64 - lsb as i64
                            };
                            if src >= 0 && (src as u32) < val.width {
                                cur.set_bit(i, val.get_bit(src as usize));
                            }
                        }
                        let changed = self.packed_store_value(id, &cur);
                        return changed;
                    }
                    let width = self.signal_widths[id] as usize;
                    // Whole-word fast path: if the range covers the whole
                    // signal, skip the per-bit loop. A NEGATIVE low bound is
                    // excluded — the clamped `lsb` is 0 there, but the source
                    // bits are SHIFTED (`x[4:-1]` puts source bit 1 in x[0]),
                    // so resizing the value and storing it whole picks the
                    // wrong bits.
                    if src_base >= 0 && lsb == 0 && msb + 1 >= width {
                        let resized = val.resize_for_assign(width as u32);
                        let changed = self.signal_table[id] != resized;
                        if changed {
                            self.mark_dirty_id(id);
                            write_sig!(self, id, resized);
                            self.table_modified = true;
                        }
                        return changed;
                    }
                    let mut changed = false;
                    let hi = msb.min(width.saturating_sub(1));
                    if oob_write_discard(msb_i, src_base, width as u32) {
                        return false;
                    }
                    // The source offset is the EFFECTIVE low bit — `lsb` after
                    // the element-scaling and ascending-declaration rewrites
                    // above, which is what the loop always used. Only a
                    // NEGATIVE low bound needs the pre-clamp value, and those
                    // rewrites never apply to one (they clamp at 0).
                    let base = if src_base < 0 { src_base } else { lsb as i64 };
                    // `msb_i < 0` means the whole select sits below bit 0 —
                    // nothing is written (the clamped msb/lsb would otherwise
                    // both read 0 and clobber the low bit).
                    if msb_i >= 0 && hi >= lsb {
                        // Width of the SELECT in bits (unclamped), so a value
                        // wider than the select is truncated to it before the
                        // in-range bits are placed (§10.7 / §11.5.1).
                        // Only for a plain vector: an element-scaled select
                        // keeps its label-level `msb_i`/`src_base`, and its
                        // value is already element-sized.
                        let sel_w = if elem_scaled {
                            0
                        } else {
                            (msb_i - src_base + 1).max(0) as u32
                        };
                        let fitted;
                        let val: &Value = if sel_w > 0 && (val.width != sel_w || val.is_fill) {
                            fitted = val.resize_for_assign(sel_w);
                            &fitted
                        } else {
                            val
                        };
                        let _ = asc_phys_hi;
                        for i in lsb..=hi {
                            let src = i as i64 - base;
                            if src < 0 {
                                continue;
                            }
                            let nb = val.get_bit(src as usize);
                            if self.signal_table[id].get_bit(i) != nb {
                                self.signal_table[id].set_bit(i, nb);
                                self.sync_mirror(id);
                                changed = true;
                            }
                        }
                    }
                    if changed {
                        self.table_modified = true;
                        self.after_signal_write(id);
                        self.mark_dirty_id(id);
                    }
                    return changed;
                }
                // Fall-through for LHS range-select on storage NOT backed by
                // signal_table — local-stack vars (`logic [3:0] r; r[3:0] = ...`
                // inside an initial/task), this.<prop> instance fields, and
                // module signals only in the slow-path `signals` HashMap.
                // Without these branches `is_equal[3:0] = {4{...}}` in
                // cv32e40p_alu would silently no-op and leave is_equal at X,
                // breaking BLT → traps → infinite handler loop on every
                // compliance test.
                if let ExprKind::Ident(hier) = &expr.kind {
                    // Class instance field / packed struct field via flattened
                    // hier path: `h.field[msb:lsb]` and `s.field[msb:lsb]` both
                    // parse as Ident([root, field]) + RangeSelect.
                    if hier.path.len() == 2 {
                        let root = hier.path[0].name.name.clone();
                        let fname = hier.path[1].name.name.clone();
                        // (b) Packed struct: slice into the parent signal at
                        //     `packed_struct_fields[root] → (fname, off, w)`.
                        if let Some(fields) = self.module.packed_struct_fields.get(&root).cloned() {
                            if let Some((_, field_off, field_w)) =
                                fields.iter().find(|(m, _, _)| m == &fname).cloned()
                            {
                                // §7.4.1: an ASCENDING member dim mirrors the
                                // bit labels (`bit [0:15] b; b[14:15]` = the
                                // LOW two bits).
                                let (lsb, msb) =
                                    if let Some(mw) = self.struct_member_ascending(&root, &fname) {
                                        let mw = mw as usize;
                                        let m2 = (mw - 1).saturating_sub(lsb);
                                        let l2 = (mw - 1).saturating_sub(msb.min(mw - 1));
                                        (l2, m2)
                                    } else {
                                        (lsb, msb)
                                    };
                                if let Some(&id) = self.signal_name_to_id.get(root.as_str()) {
                                    let total_w = self.signal_widths[id] as usize;
                                    let lo = (field_off as usize) + lsb;
                                    let hi = (field_off as usize)
                                        + msb.min((field_w as usize).saturating_sub(1));
                                    if hi < total_w {
                                        let mut cur = self.signal_table[id].clone();
                                        let mut changed = false;
                                        for i in lo..=hi {
                                            let nb = val.get_bit(i - lo);
                                            if cur.get_bit(i) != nb {
                                                cur.set_bit(i, nb);
                                                changed = true;
                                            }
                                        }
                                        if changed {
                                            self.signal_table[id] = cur;
                                            self.sync_mirror(id);
                                            self.table_modified = true;
                                            self.after_signal_write(id);
                                            self.mark_dirty(&root);
                                        }
                                        return changed;
                                    }
                                }
                                if let Some(cur_sig) = self.signals.get(&root).cloned() {
                                    let total_w = cur_sig.width as usize;
                                    let lo = (field_off as usize) + lsb;
                                    let hi = (field_off as usize)
                                        + msb.min((field_w as usize).saturating_sub(1));
                                    if hi < total_w {
                                        let mut nv = cur_sig.clone();
                                        let mut changed = false;
                                        for i in lo..=hi {
                                            let nb = val.get_bit(i - lo);
                                            if nv.get_bit(i) != nb {
                                                nv.set_bit(i, nb);
                                                changed = true;
                                            }
                                        }
                                        if changed {
                                            self.signals.insert(root.clone(), nv);
                                            self.mark_dirty(&root);
                                        }
                                        return changed;
                                    }
                                }
                            }
                        }
                        // Resolve `root` to a handle from any storage class.
                        let root_handle: Option<usize> = {
                            let from_local = self
                                .local_stack
                                .last()
                                .and_then(|m| m.get(&root))
                                .and_then(|v| v.to_u64())
                                .map(|h| h as usize)
                                .filter(|&h| h != 0);
                            let from_this = self
                                .this_stack
                                .last()
                                .copied()
                                .flatten()
                                .and_then(|h| self.heap.get(h))
                                .and_then(|o| o.as_ref())
                                .and_then(|i| i.properties.get(&root))
                                .and_then(|v| v.to_u64())
                                .map(|h| h as usize)
                                .filter(|&h| h != 0);
                            let from_sig = self
                                .signals
                                .get(&root)
                                .and_then(|v| v.to_u64())
                                .map(|h| h as usize)
                                .filter(|&h| h != 0);
                            let from_table = self
                                .signal_name_to_id
                                .get(root.as_str())
                                .and_then(|&id| self.signal_table.get(id))
                                .and_then(|v| v.to_u64())
                                .map(|h| h as usize)
                                .filter(|&h| h != 0);
                            from_local.or(from_this).or(from_sig).or(from_table)
                        };
                        if let Some(handle) = root_handle {
                            // §7.4.1: an ASCENDING property (`bit [0:15] a`)
                            // labels its bits from the MSB end, so a part-write
                            // has to be mapped the way the module-signal path
                            // already maps one. Without it `a[4:7] = 4'hF`
                            // spliced the bits a DESCENDING vector would have,
                            // even though a single-bit write to the same
                            // property landed correctly. Resolved before the
                            // mutable borrow below.
                            let (msb, lsb) = match self.class_prop_packed_shape(handle, &fname) {
                                // A multidimensional property's select was
                                // mapped to physical bits above already.
                                Some((_, total, left, right))
                                    if class_dim.is_none()
                                        && physical.is_none()
                                        && left < right
                                        && total > 0 =>
                                {
                                    let top = total as usize - 1;
                                    (top.saturating_sub(lsb), top.saturating_sub(msb))
                                }
                                _ => (msb, lsb),
                            };
                            // §11.5.1: elements below bit 0 are dropped with
                            // the value bits they would have taken.
                            let shifted;
                            let val = if physical.is_some() && src_base < 0 {
                                if msb_i < 0 {
                                    return false;
                                }
                                shifted =
                                    val.shift_right(&Value::from_u64(src_base.unsigned_abs(), 32));
                                &shifted
                            } else {
                                val
                            };
                            if let Some(Some(inst)) = self.heap.get_mut(handle) {
                                // The property takes the bits in place (§11.5.1); it is
                                // noted as written only when one changes.
                                if let Some(cur) = inst.properties.get(&fname) {
                                    let changed = Self::range_store_differs(cur, lsb, msb, val);
                                    if changed && let Some(cur) = inst.properties.get_mut(&fname) {
                                        Self::range_store(cur, lsb, msb, val);
                                    }
                                    return changed;
                                }
                            }
                        }
                    }
                    if hier.path.len() == 1 {
                        let bare = hier.path[0].name.name.clone();
                        // local_stack frame
                        if !self.local_stack.is_empty() {
                            let last_idx = self.local_stack.len() - 1;
                            if let Some(cur) = self.local_stack[last_idx].get_mut(&bare) {
                                // The local takes the bits in place.
                                return Self::range_store(cur, lsb, msb, val);
                            }
                        }
                        // this.<prop>
                        if let Some(Some(handle)) = self.this_stack.last().copied() {
                            if let Some(Some(inst)) = self.heap.get_mut(handle) {
                                // The property takes the bits in place (§11.5.1); it is
                                // noted as written only when one changes.
                                if let Some(cur) = inst.properties.get(&bare) {
                                    let changed = Self::range_store_differs(cur, lsb, msb, val);
                                    if changed && let Some(cur) = inst.properties.get_mut(&bare) {
                                        Self::range_store(cur, lsb, msb, val);
                                    }
                                    return changed;
                                }
                            }
                        }
                    }
                    // signals slow-path
                    let name = self.resolve_hier_name(hier);
                    if let Some(cur) = self.signals.get(&*name) {
                        let changed = Self::range_store_differs(cur, lsb, msb, val);
                        if changed {
                            if let Some(cur) = self.signals.get_mut(&*name) {
                                Self::range_store(cur, lsb, msb, val);
                            }
                            self.mark_dirty(&name);
                        }
                        return changed;
                    }
                }
                // MemberAccess base: `obj.field[msb:lsb]` or `s.field[msb:lsb]`.
                // Two storage classes:
                //   (a) `obj` is a class instance handle, `field` lives in
                //       `heap[handle].properties` — bit-loop into the property
                //       value.
                //   (b) `s` is a packed struct whose backing signal exists in
                //       signal_table / signals slow-path, and `field` is a
                //       slice at `packed_struct_fields[s] → (field, off, w)` —
                //       bit-loop into the parent signal at `off+lsb .. off+msb`.
                if let ExprKind::MemberAccess { expr: base, member } = &expr.kind {
                    // (a) class instance property.
                    let handle_opt: Option<usize> = match &base.kind {
                        ExprKind::This => self.this_stack.last().copied().flatten(),
                        _ => {
                            let v = self.eval_expr(base);
                            v.to_u64().map(|h| h as usize).filter(|&h| h != 0)
                        }
                    };
                    if let Some(handle) = handle_opt {
                        // id-collision-in-heap: the receiver's VALUE is a heap
                        // handle only when it statically names a class object.
                        // A part-selected member write on a struct-element
                        // collection (`q[0].f[3:0] = v`) splices plain data
                        // (§7.8) and must not be rerouted into the heap.
                        if self.receiver_may_be_handle(base) {
                            let pname = member.name.clone();
                            if let Some(Some(inst)) = self.heap.get_mut(handle) {
                                // The property takes the bits in place (§11.5.1); it is
                                // noted as written only when one changes.
                                if let Some(cur) = inst.properties.get(&pname) {
                                    let changed = Self::range_store_differs(cur, lsb, msb, val);
                                    if changed && let Some(cur) = inst.properties.get_mut(&pname) {
                                        Self::range_store(cur, lsb, msb, val);
                                    }
                                    return changed;
                                }
                            }
                        }
                    }
                    // (b) packed struct field. A struct of the running frame
                    // takes (c): it hides a module variable of the same name
                    // (§23.9), which this would write.
                    if let ExprKind::Ident(bh) = &base.kind
                        && self.frame_bound_bare_name(bh).is_none()
                    {
                        let base_name = self.resolve_hier_name(bh);
                        if let Some(fields) =
                            self.module.packed_struct_fields.get(&*base_name).cloned()
                        {
                            if let Some((_, field_off, field_w)) =
                                fields.iter().find(|(m, _, _)| m == &member.name).cloned()
                            {
                                // signal_table path
                                if let Some(&id) = self.signal_name_to_id.get(base_name.as_ref()) {
                                    let total_w = self.signal_widths[id] as usize;
                                    let lo = (field_off as usize) + lsb;
                                    let hi = (field_off as usize)
                                        + msb.min((field_w as usize).saturating_sub(1));
                                    if hi < total_w {
                                        let mut changed = false;
                                        let mut cur = self.signal_table[id].clone();
                                        for i in lo..=hi {
                                            let nb = val.get_bit(i - lo);
                                            if cur.get_bit(i) != nb {
                                                cur.set_bit(i, nb);
                                                changed = true;
                                            }
                                        }
                                        if changed {
                                            self.signal_table[id] = cur;
                                            self.sync_mirror(id);
                                            self.table_modified = true;
                                            self.after_signal_write(id);
                                            self.mark_dirty(&base_name);
                                        }
                                        return changed;
                                    }
                                }
                                // signals slow-path
                                if let Some(cur_sig) = self.signals.get(&*base_name).cloned() {
                                    let total_w = cur_sig.width as usize;
                                    let lo = (field_off as usize) + lsb;
                                    let hi = (field_off as usize)
                                        + msb.min((field_w as usize).saturating_sub(1));
                                    if hi < total_w {
                                        let mut nv = cur_sig.clone();
                                        let mut changed = false;
                                        for i in lo..=hi {
                                            let nb = val.get_bit(i - lo);
                                            if nv.get_bit(i) != nb {
                                                nv.set_bit(i, nb);
                                                changed = true;
                                            }
                                        }
                                        if changed {
                                            self.signals.insert(base_name.to_string(), nv);
                                            self.mark_dirty(&base_name);
                                        }
                                        return changed;
                                    }
                                }
                            }
                        }
                    }
                    // (c) any other member receiver — notably a struct
                    // variable or a struct-element collection whose VALUE
                    // must not be read back as a heap handle (the (a) gate
                    // above): read-modify-write the FIELD through the generic
                    // member paths — read `s.f` / `q[0].f`, splice the
                    // selected bits, write the whole field back.
                    {
                        let cur = self.eval_expr(expr);
                        let width = cur.width as usize;
                        let mut nv = cur.clone();
                        let mut changed = false;
                        for i in lsb..=msb.min(width.saturating_sub(1)) {
                            let nb = val.get_bit(i - lsb);
                            if nv.get_bit(i) != nb {
                                nv.set_bit(i, nb);
                                changed = true;
                            }
                        }
                        if changed {
                            return self.assign_value(expr, &nv);
                        }
                        return changed;
                    }
                }
                // Index base: `mat[i][msb:lsb]` for a packed multi-D vector
                // whose elements are elem_w bits each (registered in
                // `packed_signal_elem_widths`). Compute the slice offset
                // into the flat parent signal: `i*elem_w + lsb`.
                if let ExprKind::Index {
                    expr: arr_expr,
                    index,
                } = &expr.kind
                {
                    if let ExprKind::Ident(ah) = &arr_expr.kind {
                        let arr_name = self.resolve_hier_name(ah);
                        if let Some(&elem_w) = self.module.packed_signal_elem_widths.get(&*arr_name)
                        {
                            let idx = self.eval_expr(index).to_u64().unwrap_or(0) as usize;
                            let Some(base_off) =
                                self.packed_elem_lsb(&arr_name, idx as i64, elem_w)
                            else {
                                return false;
                            };
                            // Inner-dim labels mirror on an ascending
                            // `[0:W-1]` element type (§7.4.1).
                            let (Some(b_lsb), Some(b_msb)) = (
                                self.packed_inner_bit(&arr_name, lsb as i64, elem_w),
                                self.packed_inner_bit(
                                    &arr_name,
                                    msb.min((elem_w as usize).saturating_sub(1)) as i64,
                                    elem_w,
                                ),
                            ) else {
                                return false;
                            };
                            let lo = base_off + b_lsb.min(b_msb);
                            let hi = base_off + b_lsb.max(b_msb);
                            if let Some(&id) = self.signal_name_to_id.get(arr_name.as_ref()) {
                                let total_w = self.signal_widths[id] as usize;
                                if hi < total_w {
                                    let mut changed = false;
                                    let mut cur = self.signal_table[id].clone();
                                    for i in lo..=hi {
                                        let nb = val.get_bit(i - lo);
                                        if cur.get_bit(i) != nb {
                                            cur.set_bit(i, nb);
                                            changed = true;
                                        }
                                    }
                                    if changed {
                                        self.signal_table[id] = cur;
                                        self.sync_mirror(id);
                                        self.table_modified = true;
                                        self.after_signal_write(id);
                                        self.mark_dirty(&arr_name);
                                    }
                                    return changed;
                                }
                            }
                            if let Some(cur_sig) = self.signals.get(&*arr_name).cloned() {
                                let total_w = cur_sig.width as usize;
                                if hi < total_w {
                                    let mut nv = cur_sig.clone();
                                    let mut changed = false;
                                    for i in lo..=hi {
                                        let nb = val.get_bit(i - lo);
                                        if nv.get_bit(i) != nb {
                                            nv.set_bit(i, nb);
                                            changed = true;
                                        }
                                    }
                                    if changed {
                                        self.signals.insert(arr_name.to_string(), nv);
                                        self.mark_dirty(&arr_name);
                                    }
                                    return changed;
                                }
                            }
                            // `foo.a[i][msb:lsb]` where `a` is a packed
                            // multi-D MEMBER of a packed struct: no signal
                            // exists under the dotted name — splice at
                            // field_offset + i*elem_w + lsb within the ROOT
                            // signal (§7.4.1).
                            if let Some((root, leaf)) = arr_name.rsplit_once('.') {
                                let field_off =
                                    self.module.packed_struct_fields.get(root).and_then(|fs| {
                                        fs.iter().find(|(f, _, _)| f == leaf).map(|(_, o, _)| *o)
                                    });
                                if let (Some(field_off), Some(&id)) =
                                    (field_off, self.signal_name_to_id.get(root))
                                {
                                    let total_w = self.signal_widths[id] as usize;
                                    let alo = field_off as usize + lo;
                                    let ahi = field_off as usize + hi;
                                    if ahi < total_w {
                                        let mut cur = self.signal_table[id].clone();
                                        let mut changed = false;
                                        for i in alo..=ahi {
                                            let nb = val.get_bit(i - alo);
                                            if cur.get_bit(i) != nb {
                                                cur.set_bit(i, nb);
                                                changed = true;
                                            }
                                        }
                                        if changed {
                                            self.signal_table[id] = cur;
                                            self.sync_mirror(id);
                                            self.table_modified = true;
                                            self.after_signal_write(id);
                                            self.mark_dirty(root);
                                        }
                                        return changed;
                                    }
                                }
                            }
                        }
                    }
                }
                // §11.5.2 / §10.4: part-select WRITE through an UNPACKED
                // array element with no compact-table id — block-local and
                // automatic subroutine-local arrays live in slow-path
                // storage, and every handler above either requires a
                // signal_table id or a packed multi-D registration, so
                // `larr[1][7:4] = 4'hB` fell through to the silent `false`
                // and the write vanished. Read-modify-write the element
                // through the canonical by-name accessors instead.
                if let ExprKind::Index {
                    expr: arr_expr,
                    index,
                } = &expr.kind
                {
                    if let ExprKind::Ident(ah) = &arr_expr.kind {
                        let arr_name = self.resolve_hier_name(ah);
                        if !self
                            .module
                            .packed_signal_elem_widths
                            .contains_key(&*arr_name)
                        {
                            let idxv = self.eval_expr(index);
                            if let Some(idx) = idxv.to_i64().filter(|_| !idxv.has_xz()) {
                                let ename = format!("{}[{}]", arr_name, idx);
                                if let Some(cur) = self.get_signal_value_by_name(&ename) {
                                    let w = cur.width as usize;
                                    let mut nv = cur.clone();
                                    let mut changed = false;
                                    let hi2 = msb.min(w.saturating_sub(1));
                                    let base = if src_base < 0 { src_base } else { lsb as i64 };
                                    if msb_i >= 0 && hi2 >= lsb {
                                        for i in lsb..=hi2 {
                                            let src = i as i64 - base;
                                            if src < 0 {
                                                continue;
                                            }
                                            let nb = val.get_bit(src as usize);
                                            if nv.get_bit(i) != nb {
                                                nv.set_bit(i, nb);
                                                changed = true;
                                            }
                                        }
                                    }
                                    if changed {
                                        self.set_signal_value_by_name(&ename, nv);
                                        self.mark_dirty(&arr_name);
                                    }
                                    return changed;
                                }
                            }
                        }
                    }
                    // Deeper chains (array-of-queues element:
                    // `qa[0][0][31:24] = v`): the arm above resolves only ONE
                    // Index layer — build the flat element name for the whole
                    // chain and apply the same read-modify-write.
                    if let Some(ename) = self.flat_member_name(expr) {
                        if let Some(cur) = self.get_signal_value_by_name(&ename) {
                            let w = cur.width as usize;
                            let mut nv = cur.clone();
                            let mut changed = false;
                            let hi2 = msb.min(w.saturating_sub(1));
                            let base = if src_base < 0 { src_base } else { lsb as i64 };
                            if msb_i >= 0 && hi2 >= lsb {
                                for i in lsb..=hi2 {
                                    let src = i as i64 - base;
                                    if src < 0 {
                                        continue;
                                    }
                                    let nb = val.get_bit(src as usize);
                                    if nv.get_bit(i) != nb {
                                        nv.set_bit(i, nb);
                                        changed = true;
                                    }
                                }
                            }
                            if changed {
                                self.set_signal_value_by_name(&ename, nv);
                            }
                            return changed;
                        }
                    }
                }
                false
            }
            ExprKind::Concatenation(parts) => {
                let tw: u32 = parts.iter().map(|p| self.infer_width(p)).sum();
                let rv = val.resize_for_assign(tw);
                let mut off = 0usize;
                let mut changed = false;
                for part in parts.iter().rev() {
                    let pw = self.infer_width(part);
                    let pv = rv.range_select(off + pw as usize - 1, off);
                    if self.assign_value(part, &pv) {
                        changed = true;
                    }
                    off += pw as usize;
                }
                changed
            }
            ExprKind::MemberAccess { expr, member } => {
                // §7.2.1/§7.4: a field of an element of a class array of
                // packed structs (`beats[i].last = 1`, `o.beats[i].mask =
                // m`) is a slice of the element: write the element with
                // the field's bits replaced. The read side
                // (`packed_struct_elem_type`) already slices it. So is a
                // member of a packed struct nested in the element
                // (`inst[i].arch.vl = v`, §7.2.1): only a member of the
                // element itself was looked up, and the nested write was
                // dropped.
                let mut elem_expr = expr.as_ref();
                while let ExprKind::MemberAccess { expr: inner, .. } = &elem_expr.kind {
                    elem_expr = inner;
                }
                if let ExprKind::Index { .. } = &elem_expr.kind
                    && let Some(su) = self.packed_struct_elem_type(elem_expr)
                    && let Some((off, w, total)) = self.packed_member_chain_slice(&su, lhs)
                {
                    let mut elem = self.eval_expr(elem_expr);
                    if elem.width != total {
                        elem = elem.resize(total);
                    }
                    let fv = val.resize_for_assign(w);
                    for i in 0..w {
                        elem.set_bit((off + i) as usize, fv.get_bit(i as usize));
                    }
                    return self.assign_value(elem_expr, &elem);
                }
                // §13.4.2: a member of a subroutine-local / formal unpacked
                // struct is owned by the call frame. Inside a subroutine body
                // `s.a = v` parses as MemberAccess (at module level the same
                // text is a dotted Ident, which the arm above already frames),
                // and every branch below looks in the module signal table — so
                // the write left the frame's copy untouched and the matching
                // read returned an outer scope's value.
                // LRM §25.9: virtual-interface alias (formal frame or
                // plain `virtual <iface>` variable). Same as the
                // hier-Ident path above but for the MemberAccess shape.
                if let ExprKind::Ident(hier) = &expr.kind {
                    if hier.path.len() == 1 {
                        {
                            let first = hier.path[0].name.name.as_str();
                            if let Some(bound) = self.iface_alias_for(first) {
                                let target = format!("{}.{}", bound, member.name);
                                let prev = self.get_signal_value_by_name(&target);
                                let w = prev.as_ref().map(|v| v.width).unwrap_or(val.width);
                                let resized = val.resize_for_assign(w);
                                let changed = prev.as_ref() != Some(&resized);
                                self.set_signal_value_by_name(&target, resized);
                                return changed;
                            }
                        }
                    }
                }
                if let Some(bound) = self.indexed_iface_alias_for(expr) {
                    let target = format!("{}.{}", bound, member.name);
                    let prev = self.get_signal_value_by_name(&target);
                    let w = prev.as_ref().map(|v| v.width).unwrap_or(val.width);
                    let resized = val.resize_for_assign(w);
                    let changed = prev.as_ref() != Some(&resized);
                    self.set_signal_value_by_name(&target, resized);
                    return changed;
                }
                // Unpacked aggregates keep every leaf in its own signal
                // (`arr[i].m`, `c.nodes[i].m`) — there is no contiguous bit
                // layout to slice, and a `real` member has no bit offset. If the
                // flattened name names a signal, write it directly, honouring the
                // declared width / signedness / real-ness.
                if let Some(flat) = self.flat_member_name(lhs) {
                    if let Some(&id) = self.signal_name_to_id.get(flat.as_str()) {
                        let width = self.signal_widths[id];
                        let mut resized = if self.signal_real[id] {
                            if val.is_real {
                                val.clone()
                            } else {
                                Value::from_f64(val.to_f64())
                            }
                        } else if val.is_real {
                            Self::real_to_int(val.to_f64(), width)
                        } else {
                            val.resize_for_assign(width)
                        };
                        resized.is_signed = self.signal_signed[id];
                        let prev = self.get_signal_value_by_name(&flat);
                        let changed = prev.as_ref() != Some(&resized);
                        self.set_signal_value_by_name(&flat, resized);
                        return changed;
                    }
                    // Same leaf-per-member write for an element of a QUEUE /
                    // dynamic / associative collection whose element type is an
                    // unpacked struct. A top-level container happens to have
                    // table ids for its member leaves (caught above), an
                    // INSTANCE one does not — `u.q[0].latency = v` fell through
                    // every arm and vanished, so a router's per-element
                    // `q[i].latency--` never drained its queue.
                    if let Some((parent, _)) = flat.rsplit_once('.') {
                        if let Some(cont) = parent.rfind('[').map(|i| &parent[..i]) {
                            // A class-member collection stores under its
                            // instance-scoped name (`<h>#slices`), which the
                            // flattened bare name misses — `slices[0].path =
                            // v` inside a method fell through every arm and
                            // vanished (UVM's uvm_hdl_path_concat::add_path
                            // built empty slices, so every backdoor access
                            // read 0). Resolve the container through `this`
                            // before gating.
                            let registered = |s: &Self, c: &str| {
                                s.module.dynamic_arrays.contains(c)
                                    || s.module.arrays.contains_key(c)
                                    || s.module.associative_arrays.contains_key(c)
                            };
                            let (cont_s, flat_s) = if registered(self, cont) {
                                (cont.to_string(), flat.clone())
                            } else {
                                // this-relative member OR through an object
                                // handle path (`c.slices[0].path`).
                                let sc = self.resolve_locator_storage(cont);
                                if sc != cont {
                                    let f2 = format!("{}{}", sc, &flat[cont.len()..]);
                                    (sc, f2)
                                } else {
                                    // A struct element of a member array
                                    // inside a collection element
                                    // (`q[0].rows[1].m = v`): the collection
                                    // is the first select's base.
                                    let first = flat.find('[').filter(|&f| f < cont.len());
                                    match first
                                        .map(|f| (f, self.resolve_locator_storage(&flat[..f])))
                                    {
                                        Some((f, sc)) if sc != flat[..f] => {
                                            let f2 = format!("{}{}", sc, &flat[f..]);
                                            (sc, f2)
                                        }
                                        _ => (cont.to_string(), flat.clone()),
                                    }
                                }
                            };
                            if registered(self, &cont_s)
                                && self.queue_elem_struct(&cont_s).is_some()
                            {
                                let prev = self.get_signal_value_by_name(&flat_s);
                                let fitted = match prev.as_ref() {
                                    Some(p) if p.is_real && !val.is_real => {
                                        Value::from_f64(val.to_f64())
                                    }
                                    Some(p) if !p.is_real && !val.is_real => val.resize_for_assign(p.width),
                                    // §6.12.2: a real into an integral leaf rounds.
                                    Some(p) if !p.is_real && val.is_real => {
                                        Self::real_into_slot(p, val.to_f64())
                                    }
                                    _ => val.clone(),
                                };
                                let changed = prev.as_ref() != Some(&fitted);
                                self.set_signal_value_by_name(&flat_s, fitted);
                                return changed;
                            }
                        }
                    }
                    // A struct FORMAL bound member-wise lives in the call
                    // frame under its dotted key (`o.a`), not the signal
                    // table — the `Ident([o,a])` shape already checks
                    // `local_stack` (the dotted-local arm above), but the
                    // `MemberAccess{Ident(o),a}` shape missed and the body's
                    // `o.a = v` write was silently dropped. Gated on the
                    // dotted key actually being in the frame, so an ordinary
                    // top-level struct member (in `signal_name_to_id`) and a
                    // class property (handled by `class_agg_member`) never
                    // reach here.
                    if flat.contains('.') && !self.local_stack.is_empty() {
                        let last_idx = self.local_stack.len() - 1;
                        if self.local_stack[last_idx].contains_key(flat.as_str()) {
                            let fitted = if !val.is_real {
                                if let Some(&target_w) = self.widths.get(flat.as_str()) {
                                    if val.width < target_w {
                                        val.resize_for_assign(target_w)
                                    } else {
                                        val.clone()
                                    }
                                } else {
                                    val.clone()
                                }
                            } else if let Some(p) = self.local_stack[last_idx].get(flat.as_str())
                                && !p.is_real
                            {
                                // §6.12.2: a real into an integral leaf rounds.
                                Self::real_into_slot(p, val.to_f64())
                            } else {
                                val.clone()
                            };
                            self.local_stack[last_idx].insert(flat, fitted);
                            return true;
                        }
                    }
                }
                // A nested PACKED struct member inside an unpacked aggregate
                // (`arr[i].tag.vlan`): slice the parent's own signal.
                if let Some((base, off, w)) = self.packed_struct_member_slice(lhs) {
                    // The container may be a procedural LOCAL: a packed
                    // struct declared inside a task lives in the call
                    // frame, not the signal table, so a member write
                    // reached nothing and the struct read back as X.
                    if let Some(cur_sig) = self.get_local_or_signal(&base) {
                        let total_w = cur_sig.width;
                        let mut cur = cur_sig.resize(total_w);
                        let piece = val.resize_for_assign(w);
                        for i in 0..w {
                            cur.set_bit((off + i) as usize, piece.get_bit(i as usize));
                        }
                        let changed = cur != cur_sig;
                        self.set_local_or_signal(&base, cur);
                        return changed;
                    }
                }
                // LRM §25.10 — `vif_arr[i].member = ...` (indexed
                // virtual-iface property write). Index{ Ident(vif_arr), idx }
                // base shape; look up `(this_h, "vif_arr[idx]")` binding.
                if let ExprKind::Index { expr: base, index } = &expr.kind {
                    if let ExprKind::Ident(hier) = &base.kind {
                        if hier.path.len() == 1 {
                            let prop_base = hier.path[0].name.name.to_string();
                            let this_h_opt = self.this_stack.last().and_then(|t| *t);
                            if let Some(this_h) = this_h_opt {
                                let cls_name = self
                                    .heap
                                    .get(this_h)
                                    .and_then(|o| o.as_ref().map(|i| i.class_name.clone()));
                                let is_vif = cls_name.as_ref().is_some_and(|cn| {
                                    self.class_vif_decl(cn, &prop_base).is_some()
                                });
                                if is_vif {
                                    let idx = self.eval_expr(index).to_u64().unwrap_or(0);
                                    let key_prop = format!("{}[{}]", prop_base, idx);
                                    if let Some((bound_name, _modport)) = self
                                        .virtual_iface_bindings
                                        .get(&(this_h, key_prop))
                                        .cloned()
                                    {
                                        let target = format!("{}.{}", bound_name, member.name);
                                        let prev = self.get_signal_value_by_name(&target);
                                        let w = prev.as_ref().map(|v| v.width).unwrap_or(val.width);
                                        let resized = val.resize_for_assign(w);
                                        let changed = prev.as_ref() != Some(&resized);
                                        self.set_signal_value_by_name(&target, resized);
                                        return changed;
                                    }
                                }
                            }
                        }
                    }
                }
                // Associative-array element struct-member write:
                // `assoc[key].field = val` where the element is an (unpacked)
                // struct (an assoc keyed by a handle/key, element = struct). Stored
                // as the dotted signal `assoc[keystr].field`, plus a presence
                // marker `assoc[keystr]` so `exists(key)` sees the slot. Packed-
                // struct arrays are handled by the dedicated walk below (this is
                // gated on `is_associative_array`, which they are not).
                if let ExprKind::Index { .. } = &expr.kind {
                    // A member of a struct ELEMENT of a class collection:
                    // `assoc[key].field = v` (1-D) or `m[k1][k2][k3].field = v`
                    // (multidim). Stored as the dotted cell `<elem>.<field>`
                    // plus a presence marker `<elem>` so `exists(key)` sees the
                    // slot. The generalised resolver handles any index arity via
                    // the compound key `nested_index_name`.
                    if let Some((elem, su)) = self.coll_assoc_struct_elem(expr) {
                        if self.coll_field_width(&su, &member.name).is_some() {
                            let target = format!("{}.{}", elem, member.name);
                            let prev = self.get_signal_value_by_name(&target);
                            let w = prev.as_ref().map(|v| v.width).unwrap_or(val.width);
                            let resized = val.resize_for_assign(w);
                            let changed = prev.as_ref() != Some(&resized);
                            self.set_signal_value_by_name(&target, resized);
                            if self.get_signal_value_by_name(&elem).is_none() {
                                self.set_signal_value_by_name(&elem, Value::zero(1));
                            }
                            return changed;
                        }
                    }
                }
                // LRM §25.8 — `vif.member = ...` where `vif` is a virtual-
                // interface property of the current `this`'s class. Look
                // up the binding, redirect the write to
                // `<bound>.<member>`.
                //
                // `this.vif.member = ...` is the same write with an explicit
                // `this.` and must resolve identically: it parses as
                // MemberAccess(MemberAccess(This, vif), member), so match the
                // inner `this.vif` here too. Without this the write missed the
                // redirect entirely and landed on a phantom signal, leaving the
                // real interface member reading x.
                let vif_prop_name: Option<&str> = match &expr.kind {
                    ExprKind::Ident(hier) if hier.path.len() == 1 => {
                        Some(hier.path[0].name.name.as_str())
                    }
                    ExprKind::MemberAccess {
                        expr: inner,
                        member: vprop,
                    } if matches!(&inner.kind, ExprKind::This) => Some(vprop.name.as_str()),
                    _ => None,
                };
                if let Some(prop) = vif_prop_name {
                    {
                        if let Some(Some(this_h)) = self.this_stack.last() {
                            let cls_name = if let Some(Some(inst)) = self.heap.get(*this_h) {
                                Some(inst.class_name.clone())
                            } else {
                                None
                            };
                            let bound = cls_name
                                .as_ref()
                                .and_then(|cn| self.class_vif_decl(cn, prop).map(|_| ()))
                                .and(
                                    self.virtual_iface_bindings
                                        .get(&(*this_h, prop.to_string()))
                                        .cloned(),
                                );
                            if let Some((bound_name, modport_opt)) = bound {
                                // LRM §25.4 — warn (don't reject) when
                                // writing to a member declared `input` by
                                // the property's modport view. We do not
                                // block the write because the user may have
                                // a deliberate test pattern; downstream the
                                // member will still drive whatever signal
                                // `<bound>.<member>` names.
                                if let Some(mp_name) = &modport_opt {
                                    // Locate the iface type from the class's
                                    // virtual_iface_properties record.
                                    let iface_t = cls_name
                                        .as_ref()
                                        .and_then(|cn| self.class_vif_decl(cn, prop))
                                        .map(|(_, (t, _))| t.clone());
                                    if let Some(iface_t) = iface_t {
                                        if let Some(dirs) = self
                                            .module
                                            .modport_member_dirs
                                            .get(&(iface_t.clone(), mp_name.clone()))
                                        {
                                            if matches!(
                                                dirs.get(&member.name),
                                                Some(crate::ast::types::PortDirection::Input)
                                            ) {
                                                eprintln!(
                                                    "[modport] virtual {}.{} write to input member '{}' (LRM §25.4)",
                                                    iface_t, mp_name, member.name
                                                );
                                            }
                                        }
                                    }
                                }
                                let target = format!("{}.{}", bound_name, member.name);
                                let prev = self.get_signal_value_by_name(&target);
                                let w = prev.as_ref().map(|v| v.width).unwrap_or(val.width);
                                let resized = val.resize_for_assign(w);
                                let changed = prev.as_ref() != Some(&resized);
                                self.set_signal_value_by_name(&target, resized);
                                return changed;
                            }
                        }
                    }
                }
                // `ClassName#(params)::static_prop = ...` — a STATIC scalar /
                // handle property write through a class SPECIALIZATION
                // receiver (the READ path resolves it per-spec via
                // class_static_get; the write must land in the SAME per-spec
                // cell). Without this `C#(int)::n = 5` fell through to a
                // bare-name store the later read never consulted.
                if let ExprKind::Specialization { .. } = &expr.kind {
                    if let Some((base, sig)) = self
                        .resolve_call_spec_params(Self::extract_call_spec(expr), &self.current_spec)
                    {
                        if self.module.classes.contains_key(&base)
                            && !self.signal_name_to_id.contains_key(base.as_str())
                        {
                            let saved = self.current_spec.take();
                            self.current_spec = Some((base.clone(), sig));
                            let ok = self.class_static_set(&base, &member.name, val.clone());
                            self.current_spec = saved;
                            if ok {
                                return true;
                            }
                        }
                    }
                }
                // `ClassName::static_prop = ...` — explicit static write.
                if let ExprKind::Ident(hier) = &expr.kind {
                    if hier.path.len() == 1 {
                        let cls = hier.path[0].name.name.clone();
                        if self.module.classes.contains_key(&cls)
                            && !self
                                .local_stack
                                .last()
                                .is_some_and(|m| m.contains_key(&cls))
                            && !self.signal_name_to_id.contains_key(cls.as_str())
                            && !self.signals.contains_key(&cls)
                            && self.class_static_set(&cls, &member.name, val.clone())
                        {
                            return true;
                        }
                        // §8.25/§6.18: through a typedef of a specialization.
                        if !self.module.classes.contains_key(&cls)
                            && !self
                                .local_stack
                                .last()
                                .is_some_and(|m| m.contains_key(&cls))
                            && !self.signal_name_to_id.contains_key(cls.as_str())
                            && !self.signals.contains_key(&cls)
                            && let Some((base, sig)) = self.scope_prefix_spec(&cls)
                        {
                            let saved = self.current_spec.replace((base.clone(), sig));
                            let ok = self.class_static_set(&base, &member.name, val.clone());
                            self.current_spec = saved;
                            if ok {
                                return true;
                            }
                        }
                    }
                }
                // A procedural-local unpacked struct whose members were
                // materialised individually — by the dotted `Ident([s,a])`
                // lvalue arm above, or by foreach/randomize element
                // construction — lives in the runtime `signals` map as
                // standalone dotted keys (`s.a`, `s.b`), NOT as a single
                // packed whole. The whole-struct splice arm below reads the
                // (empty) whole value, splices the field into it, and writes
                // a phantom whole `s` that no member read ever consults — so
                // the actual `s.a` leaf stays at its pre-call value.
                //
                // When the dotted leaf already exists as its own key, write
                // it directly. A struct stored whole (case 1: `output`
                // formal whose actual was never pre-touched) has no such key
                // and correctly falls through to the whole-struct arm.
                if let Some(flat) = self.flat_member_name(lhs) {
                    if flat.contains('.') && self.signals.contains_key(flat.as_str()) {
                        let prev = self.signals.get(flat.as_str()).cloned();
                        let fitted = match prev.as_ref() {
                            Some(p) if p.is_real && !val.is_real => Value::from_f64(val.to_f64()),
                            Some(p) if !p.is_real && !val.is_real => val.resize_for_assign(p.width),
                            // §6.12.2: a real into an integral leaf rounds.
                            Some(p) if !p.is_real && val.is_real => {
                                Self::real_into_slot(p, val.to_f64())
                            }
                            _ => val.clone(),
                        };
                        let changed = prev.as_ref() != Some(&fitted);
                        self.signals.insert(flat, fitted);
                        return changed;
                    }
                }
                // Struct variable member write: `result.field1 = ...`
                // (BUT NOT a member of a CLASS handle: a forward-declared-
                // class formal like UVM's `uvm_reg_item rw` is recorded in
                // `var_class_types`, yet a stale same-named struct local from
                // a CALLER scope (the predictor's `uvm_reg_bus_op rw`) stays
                // in the flat `var_typedef_types`, so this arm misread the
                // class handle as a struct and spliced `status` over its low
                // bits, zeroing the heap handle inside `uvm_reg::do_predict`.
                // Never struct-splice a class variable — its members live in
                // the heap, not a packed local.
                if let ExprKind::Ident(hier) = &expr.kind {
                    if hier.path.len() == 1 {
                        let base_name = hier.path[0].name.name.as_str();
                        if self.class_of_var(base_name).is_none() {
                            let type_name = self
                                .var_typedef_types
                                .get(base_name)
                                .cloned()
                                .or_else(|| self.get_expr_type_name(expr));
                            let dt: Option<DataType> = type_name
                                .as_deref()
                                .and_then(|tn| self.module.typedef_types.get(tn).cloned());
                            if let Some(dt) = dt {
                                let resolved =
                                    Self::resolve_type_ref(&dt, &self.module.typedef_types);
                                if let Some(fields) = self.struct_field_layout(&resolved) {
                                    if let Some((_, off, w, _real)) = fields
                                        .iter()
                                        .find(|(m, _, _, _)| m == &member.name)
                                        .cloned()
                                    {
                                        let mut cur = self
                                            .get_local_or_signal(base_name)
                                            .unwrap_or_else(|| Value::zero(off + w));
                                        if (off + w) as usize > cur.width as usize {
                                            cur = cur.resize(off + w);
                                        }
                                        let piece = val.resize_for_assign(w);
                                        for i in 0..w {
                                            let bit = piece.get_bit(i as usize);
                                            cur.set_bit((off + i) as usize, bit);
                                        }
                                        self.set_local_or_signal(base_name, cur);
                                        return true;
                                    }
                                }
                            }
                        }
                    }
                }
                // packed_struct_fields for signal-based structs
                if let ExprKind::Ident(hier) = &expr.kind {
                    let name = self.resolve_hier_name(hier);
                    if let Some(fields) = self.module.packed_struct_fields.get(&*name).cloned() {
                        if let Some((_, off, w)) =
                            fields.iter().find(|(m, _, _)| m == &member.name).cloned()
                        {
                            if let Some(cur_sig) = self.get_signal_value_by_name(&name) {
                                let total_w = cur_sig.width;
                                let mut cur = cur_sig.resize(total_w);
                                // §7.3: a real union member stores real bits.
                                let piece = if w == 64
                                    && !val.is_real
                                    && self.union_real_member(&name, &member.name)
                                {
                                    Value::from_f64(val.to_f64()).resize(w)
                                } else {
                                    val.resize_for_assign(w)
                                };
                                for i in 0..w {
                                    let bit = piece.get_bit(i as usize);
                                    cur.set_bit((off + i) as usize, bit);
                                }
                                let prev = self.get_signal_value_by_name(&name);
                                let changed = prev.as_ref() != Some(&cur);
                                self.set_signal_value_by_name(&name, cur);
                                return changed;
                            }
                        }
                    }
                }
                // `arr[idx...].outer.inner...field = val` — N-dim arrays of
                // packed struct with chained MemberAccess. Walks both the
                // MemberAccess chain (collecting dotted field-path:
                // `outer.inner...field`) AND the Index chain (collecting
                // indices into `arr[i][j]...[k]`). Then looks up the dotted
                // field path in `packed_struct_fields[arr_name]` — the
                // elaborator's `flatten_subfields` already registers nested
                // fields under their full dotted path.
                {
                    let mut field_path: Vec<String> = vec![member.name.clone()];
                    let mut cur_expr: &Expression = expr;
                    // Walk inward through MemberAccess nodes; the LAST one
                    // reaches either Index (array form) or Ident (plain
                    // struct, handled by the Ident branch below).
                    loop {
                        match &cur_expr.kind {
                            ExprKind::MemberAccess { expr: b, member: m } => {
                                field_path.push(m.name.clone());
                                cur_expr = b.as_ref();
                            }
                            _ => break,
                        }
                    }
                    field_path.reverse();
                    let mut indices: Vec<u64> = Vec::new();
                    let root_name: Option<String> = loop {
                        match &cur_expr.kind {
                            ExprKind::Index { expr: b, index } => {
                                indices.push(self.eval_expr(index).to_u64().unwrap_or(0));
                                cur_expr = b.as_ref();
                            }
                            // A procedural LOCAL registers its layout under
                            // its BARE name, but inside a submodule process
                            // `resolve_hier_name` scopes a bare ident
                            // (`u.a`) — so `a[0].p = v` on a task-local
                            // packed array of structs found no layout and
                            // the element read back x, while the same code
                            // in the top module (empty scope) worked.
                            ExprKind::Ident(h) => {
                                let scoped = self.resolve_hier_name(h);
                                if !self.module.packed_struct_fields.contains_key(&*scoped)
                                    && h.path.len() == 1
                                    && h.path[0].selects.is_empty()
                                    && self
                                        .module
                                        .packed_struct_fields
                                        .contains_key(&h.path[0].name.name)
                                {
                                    break Some(h.path[0].name.name.clone());
                                }
                                break Some(scoped.to_string());
                            }
                            // §7.4.2: root may itself be a struct-member path
                            // (`main.sub_list[0].f`) — build the dotted name so
                            // the `packed_struct_fields["main.sub_list"]` layout
                            // registered at elaboration resolves it.
                            ExprKind::MemberAccess { .. } => {
                                let mut segs: Vec<String> = Vec::new();
                                let mut c = cur_expr;
                                while let ExprKind::MemberAccess { expr: b, member: m } = &c.kind {
                                    segs.push(m.name.clone());
                                    c = b.as_ref();
                                }
                                if let ExprKind::Ident(h) = &c.kind {
                                    segs.push(self.resolve_hier_name(h).into_owned());
                                    segs.reverse();
                                    break Some(segs.join("."));
                                }
                                break None;
                            }
                            _ => break None,
                        }
                    };
                    if let Some(arr_name) = root_name {
                        if !indices.is_empty() {
                            indices.reverse();
                            if let Some(fields) =
                                self.module.packed_struct_fields.get(&arr_name).cloned()
                            {
                                let field_key = field_path.join(".");
                                if let Some((_, off, w)) =
                                    fields.iter().find(|(m, _, _)| m == &field_key).cloned()
                                {
                                    let mut elem_name = arr_name.clone();
                                    for i in &indices {
                                        elem_name.push_str(&format!("[{}]", i));
                                    }
                                    if let Some(cur_sig) = self.get_signal_value_by_name(&elem_name)
                                    {
                                        let total_w = cur_sig.width;
                                        let mut cur = cur_sig.resize(total_w);
                                        let piece = val.resize_for_assign(w);
                                        for i in 0..w {
                                            let bit = piece.get_bit(i as usize);
                                            cur.set_bit((off + i) as usize, bit);
                                        }
                                        let prev = self.get_signal_value_by_name(&elem_name);
                                        let changed = prev.as_ref() != Some(&cur);
                                        self.set_signal_value_by_name(&elem_name, cur);
                                        return changed;
                                    }
                                    // §7.4.2: PACKED array of packed struct —
                                    // `foo[i]` is NOT a separate signal but a bit
                                    // slice of the single backing vector. Splice
                                    // the field at `i*elem_w + off`.
                                    {
                                        // Struct width from the field layout, and
                                        // the slot flattened over ALL declared
                                        // dims — the old form handled a single
                                        // index with `packed_signal_elem_widths`,
                                        // which is the OUTERMOST element width, so
                                        // `t [0:0][1:0]` (two dims) both failed the
                                        // arity check and would have used 2x the
                                        // struct width had it passed.
                                        let struct_w =
                                            fields.iter().map(|(_, o, w)| o + w).max().unwrap_or(0);
                                        let dims =
                                            self.module.packed_full_dims.get(&arr_name).cloned();
                                        if let Some(slot) =
                                            Self::flatten_packed_slot(&indices, dims.as_deref())
                                        {
                                            // Frame-aware: a task/function local
                                            // lives in the call frame, not the
                                            // signal table (`get_local_or_signal`
                                            // covers both).
                                            if let Some(cur_sig) =
                                                self.get_local_or_signal(&arr_name)
                                            {
                                                let total_w = cur_sig.width;
                                                let mut cur = cur_sig.resize(total_w);
                                                let abs_off = slot * struct_w + off;
                                                let piece = val.resize_for_assign(w);
                                                for i in 0..w {
                                                    cur.set_bit(
                                                        (abs_off + i) as usize,
                                                        piece.get_bit(i as usize),
                                                    );
                                                }
                                                let changed = cur != cur_sig;
                                                self.set_local_or_signal(&arr_name, cur);
                                                return changed;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // Mirror writes to the unpacked-struct member signal
                // (e.g. `t_unpacked_struct.val_i`) so VPI-forced values
                // and SV reads stay in sync. Both the heap property and the
                // separate signal are written; the read path prefers the
                // separate signal.
                if let ExprKind::Ident(hier) = &expr.kind {
                    if hier.path.len() == 1 {
                        let dotted = format!("{}.{}", hier.path[0].name.name, member.name);
                        if let Some(id) = self.signal_name_to_id.get(dotted.as_str()).copied() {
                            let width = self.signal_widths[id];
                            let mut resized = if self.signal_real[id] {
                                if val.is_real {
                                    val.clone()
                                } else {
                                    Value::from_f64(val.to_f64())
                                }
                            } else {
                                if val.is_real {
                                    Self::real_to_int(val.to_f64(), width)
                                } else {
                                    val.resize_for_assign(width)
                                }
                            };
                            resized.is_signed = self.signal_signed[id];
                            self.set_signal_value_by_name(&dotted, resized);
                            // Fall through to also update the heap so existing
                            // read paths that look there stay consistent.
                        }
                    }
                }
                // id-collision-in-heap: this last-resort dereference may
                // reinterpret the receiver's integral VALUE as a handle only
                // when the receiver statically names a class-object lvalue
                // (`obj.prop`, `trk.cfg.mode`, `ch[i].prop`). A struct
                // variable, a struct-element collection, a hierarchical ref,
                // or a `ref` formal of struct type splices plain data (§7.8) —
                // `r.delay_counter = v` on such a receiver used to land in
                // `heap[1].delay_counter` instead. Exotic receivers (method
                // results) keep the legacy value-based attempt, which still
                // requires a live object declaring the property.
                if self.receiver_may_be_handle(expr) {
                    let base = self.eval_expr(expr);
                    let handle = base.to_u64().unwrap_or(0) as usize;
                    if handle != 0 && handle < self.heap.len() && self.heap[handle].is_some() {
                        // §8.9: a static reached through a handle chain
                        // (`a.b.s = v`) is the class's shared cell.
                        if let Some(ok) = self.static_write_via_handle(
                            handle,
                            None,
                            None,
                            &member.name,
                            val.clone(),
                        ) {
                            return ok;
                        }
                        // §8.x: a class property assignment truncates/sign-
                        // extends the rvalue to the property's declared type
                        // (`byte a; a = 'hfff;` ⇒ 8-bit 0xff, read -1 when
                        // signed). fit_class_prop clamps the width;
                        // set_prop_if_changed stamps the declared signedness.
                        let fitted = self.fit_class_prop(handle, &member.name, val);
                        return self.set_prop_if_changed(handle, &member.name, fitted);
                    }
                }
                false
            }
            _ => false,
        }
    }
}
