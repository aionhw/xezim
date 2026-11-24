//! Assignment to indexed targets and package- or top-rooted member targets.

use super::*;

impl Simulator {
    /// Outlined from `assign_value_inner` (see `exec_stmt_blocking_assign`): keeps
    /// the dispatcher's stack frame small.
    #[inline(never)]
    pub(in crate::compiler::simulator) fn assign_value_index(
        &mut self,
        lhs: &Expression,
        val: &Value,
        expr: &Box<Expression>,
        index: &Box<Expression>,
    ) -> bool {
        if let Some(r) = self.try_string_index_char_write(expr, index, val) {
            return r;
        }
        if let Some(r) = self.assoc_elem_bit_write(expr, index, val) {
            return r;
        }
        // §7.4.1/§11.5.1: a bit of a vector the call frame holds, at the bit
        // its declared range labels.
        if !self.local_stack.is_empty()
            && let Some(r) = self.frame_vector_select_write(expr, None, index, None, val)
        {
            return r;
        }
        if self.has_coll_rows {
            match self.ensure_assoc_row(expr) {
                Err(()) => return false,
                Ok(Some((row, dim))) => {
                    // §7.10.1: a write at index == size APPENDS to a queue
                    // row (`q["x"][5][0] = 7` on an empty row makes it
                    // '{7}); one past it, or at or past a dynamic row's
                    // size (§7.5.1), is ignored.
                    let iv = self.eval_expr(index);
                    let sz = self.get_queue_size(&row);
                    match iv.to_u64().filter(|_| !iv.has_xz()) {
                        Some(i) if i < sz => {}
                        Some(i) if i == sz && dim == Dim::Queue => {
                            self.set_queue_size(&row, sz + 1)
                        }
                        _ => {
                            if dim == Dim::Queue {
                                self.warn_invalid_queue_write(&row, iv.to_index(), sz);
                            }
                            return false;
                        }
                    }
                }
                Ok(None) => {}
            }
        }
        // §11.5.1: a hierarchical class property's selected element lives
        // in the heap/shared static cell, rather than a synthesized signal.
        if let ExprKind::Ident(hier) = &expr.kind
            && let Some(h) = self.hier_parent_handle(hier)
        {
            let prop = &hier.path.last().unwrap().name.name;
            let cur = self
                .static_read_via_handle(h, None, None, prop)
                .or_else(|| self.read_member_value(h, prop));
            if let Some(mut cur) = cur {
                let idx = self.eval_expr(index);
                if idx.has_xz() {
                    return false;
                }
                let range = if let Some(dims) = self.class_prop_packed_dims(h, prop) {
                    Self::packed_chain_slice(cur.width as u64, &dims, &[idx])
                } else {
                    idx.to_i64()
                        .filter(|&i| i >= 0 && i < cur.width as i64)
                        .map(|i| (i as u64, 1))
                };
                let Some((lo, width)) = range else {
                    return false;
                };
                let rhs = val.resize_for_assign(width as u32);
                for bit in 0..width as usize {
                    cur.set_bit(lo as usize + bit, rhs.get_bit(bit));
                }
                return self.set_prop_if_changed(h, prop, cur);
            }
        }
        // §7.4.1 / §11.5.1: BIT write into an ELEMENT of an unpacked
        // array/queue/assoc whose declared packed range needs label
        // mapping (`logic [31:8] a[0:0]; a[0][8] = b` — storage bit
        // 0, not 8; ascending mirrors). The arms below all treat the
        // trailing index as a raw storage offset, so the write landed
        // on the wrong bit. Width must match the dim exactly — a
        // multi-packed element's trailing index selects a SLICE and
        // belongs to the packed-element machinery below.
        if let Some((dl, dr)) = self.select_base_elem_dim(expr) {
            if dl < dr || dl.min(dr) != 0 {
                let dim_w = (dl - dr).unsigned_abs() + 1;
                if let Some(en) = self.flat_member_name(expr) {
                    if let Some(cur) = self.get_signal_value_by_name(&en) {
                        if cur.width as u64 == dim_w {
                            let idx_v = self.eval_expr(index);
                            if idx_v.has_xz() {
                                return false; // §11.5.1: x/z index selects nothing
                            }
                            let l = idx_v.to_i64().unwrap_or(i64::MIN);
                            let phys = if dl >= dr { l - dr } else { dr - l };
                            if phys < 0 || phys >= cur.width as i64 {
                                return false; // out-of-range bit write discarded
                            }
                            let nb = val.get_bit(0);
                            let changed = cur.get_bit(phys as usize) != nb;
                            if changed {
                                let mut nv = cur;
                                nv.set_bit(phys as usize, nb);
                                self.set_signal_value_by_name(&en, nv);
                            }
                            return changed;
                        }
                    }
                }
            }
        }
        // §7.4.1: packed ELEMENT write on a plain packed-of-packed
        // variable (`u8_vec16_t y; y[i] = v;`) — splice
        // [i*ew +: ew], honoring the by-NAME storage of
        // subroutine locals and formals. MUST run before the
        // collection fallback below, which would otherwise invent a
        // flat `y[i]` entry disconnected from the packed value (the
        // pixel-conversion battery's silent-agreement bug).
        if let ExprKind::Ident(h) = &expr.kind {
            if h.path.len() == 1 && h.path[0].selects.is_empty() {
                let base = self.resolve_hier_name(h);
                let bare = &h.path[0].name.name;
                let elem_key = if self.module.packed_signal_elem_widths.contains_key(&*base) {
                    Some(base.to_string())
                } else if self
                    .module
                    .packed_signal_elem_widths
                    .contains_key(bare.as_str())
                {
                    Some(bare.clone())
                } else {
                    None
                };
                // Collections keep their own semantics.
                let is_collection = self.module.arrays.contains_key(&*base)
                    || self.module.arrays.contains_key(bare.as_str())
                    || self.module.dynamic_arrays.contains(&*base)
                    || self.module.associative_arrays.contains_key(&*base);
                if let Some(pkey) = elem_key.filter(|_| !is_collection) {
                    let elem_w = *self.module.packed_signal_elem_widths.get(&pkey).unwrap();
                    let idx_v = self.eval_expr(index);
                    if idx_v.has_xz() {
                        return false; // §11.5.1: x/z index selects nothing
                    }
                    let idx = idx_v.to_i64().unwrap_or(i64::MIN);
                    // A variable stored whole in one known place — the call
                    // frame or a table signal — takes the element in place.
                    // Reading the vector out, setting the element's bits and
                    // storing it back copied and compared the whole vector
                    // several times for each element written: a loop filling
                    // a 1-Mibit buffer spent its time there.
                    if let Some(store) = self.whole_var_store(&base) {
                        let width = match store {
                            VarStore::Frame => self.frame_get(&base).map_or(0, |v| v.width),
                            VarStore::Table(id) => self.signal_table[id].width,
                        };
                        if width > elem_w {
                            if let Some(lo) = self
                                .packed_elem_lsb(&pkey, idx, elem_w)
                                .filter(|lo| lo + (elem_w as usize) <= width as usize)
                            {
                                let piece = val.resize_for_assign(elem_w);
                                match store {
                                    VarStore::Frame => {
                                        if let Some(cur) = self
                                            .local_stack
                                            .last_mut()
                                            .and_then(|l| l.get_mut(&*base))
                                        {
                                            return cur.copy_bits_from(
                                                lo,
                                                &piece,
                                                0,
                                                elem_w as usize,
                                            );
                                        }
                                    }
                                    VarStore::Table(id) => {
                                        if let Some(changed) =
                                            self.splice_signal_bits(id, lo, &piece)
                                        {
                                            if changed {
                                                self.touch_elem_base(&base);
                                            }
                                            return changed;
                                        }
                                    }
                                }
                            } else {
                                return false;
                            }
                        }
                    }
                    // Frame-aware storage: subroutine locals live in
                    // the call frame, module vars in the signal maps —
                    // get/set_local_or_signal covers both.
                    let storage_key = if self.get_local_or_signal(&base).is_some() {
                        Some(base.to_string())
                    } else if self.get_local_or_signal(bare).is_some() {
                        Some(bare.clone())
                    } else {
                        None
                    };
                    if let Some(key) = storage_key {
                        if let Some(mut cur) = self.get_local_or_signal(&key) {
                            if cur.width > elem_w {
                                self.select_store_copies += u64::from(cur.width > 64);
                                let piece = val.resize_for_assign(elem_w);
                                let Some(lo) = self.packed_elem_lsb(&pkey, idx, elem_w) else {
                                    return false;
                                };
                                if lo + (elem_w as usize) <= cur.width as usize {
                                    let prev = cur.clone();
                                    for i in 0..(elem_w as usize) {
                                        cur.set_bit(lo + i, piece.get_bit(i));
                                    }
                                    let changed = cur != prev;
                                    if changed {
                                        self.set_local_or_signal(&key, cur);
                                    }
                                    return changed;
                                }
                                return false;
                            }
                        }
                    }
                }
            }
        }
        // GitHub #86 fast path: `mem[i] = v` on a module-scope 1-D
        // FIXED unpacked array resolves to a contiguous signal id with
        // no string work (`array_first_id`), skipping the guard chain
        // below (packed-struct slice, class ND, queue, assoc) that a
        // memory fill paid per element (~2 µs each, all of it name
        // formatting + hashing). Guards: no live call frames (a
        // task/function local could shadow the name), and the name
        // must not be a queue/dynamic/associative collection —
        // `module.arrays` gives those a fake 0..63 backing range, and
        // their writes need the collection semantics below.
        if self.local_stack.is_empty() && self.no_class_objects() {
            if let ExprKind::Ident(h) = &expr.kind {
                if h.path.iter().all(|seg| seg.selects.is_empty()) {
                    let name = self.resolve_hier_name(h);
                    if let Some(&(first_id, lo, hi)) = self.array_first_id.get(name.as_ref()) {
                        if !self.module.queue_vars.contains(&*name)
                            && !self.module.dynamic_arrays.contains(&*name)
                            && !self.module.associative_arrays.contains_key(&*name)
                            // string elements are DYNAMIC-length; the
                            // resize-to-declared-width below would
                            // truncate them (a 200-char element came
                            // back 16 chars) — the general path stores
                            // strings unresized.
                            && !self.module.string_signals.contains(&*name)
                        {
                            let idx = self.eval_expr(index).to_index().unwrap_or(i64::MIN);
                            if idx >= lo && idx <= hi {
                                let id = first_id + (idx - lo) as usize;
                                return self.fast_signal_write_id(id, val);
                            }
                            // out of range: fall through to the general
                            // path for its existing diagnostics/no-op.
                        }
                    }
                }
            }
        }
        // §7.4.2: `arr[i].field[k] = v` where `arr` is a PACKED array
        // of packed structs. `arr[i]` is not a signal of its own — it
        // is a slice of one backing vector — so the write must splice
        // at `slot*struct_w + field_off + k*member_elem_w`.
        //
        // The MemberAccess arm below already handles `arr[i].field = v`
        // (no trailing index), but this form's outermost node is an
        // Index, so it never reached that code and the write was
        // silently DROPPED — the read path resolved it correctly all
        // along, which is what made the value look like it was never
        // driven rather than never stored.
        if let Some((sig, lsb, w)) = self.packed_struct_elem_slice(expr, index) {
            if let Some(cur_sig) = self.get_signal_value_by_name(&sig) {
                let mut cur = cur_sig.clone();
                let piece = val.resize_for_assign(w);
                let mut changed = false;
                for i in 0..w {
                    let nb = piece.get_bit(i as usize);
                    if cur.get_bit((lsb + i) as usize) != nb {
                        cur.set_bit((lsb + i) as usize, nb);
                        changed = true;
                    }
                }
                if changed {
                    self.set_signal_value_by_name(&sig, cur);
                }
                return changed;
            }
        }

        // Element of an unpacked ARRAY OF QUEUES: `q[i][k] = v` writes
        // element k of the queue `q[i]` (§7.4.5), not a bit of a scalar.
        // §7.4.2: a write to an N-D fixed-array class-property
        // element lands in its per-instance cell, like the read.
        if !self.no_class_objects() {
            if let Some(nm) = self.class_nd_elem_name(expr, index) {
                let prev = self.get_signal_value_by_name(&nm);
                let fitted = match prev.as_ref() {
                    Some(p) if !p.is_real && !val.is_real && p.width > 0 => val.resize_for_assign(p.width),
                    // §6.12.2: a real into an integral element rounds.
                    Some(p) if !p.is_real && val.is_real => Self::real_into_slot(p, val.to_f64()),
                    _ => val.clone(),
                };
                let changed = prev.as_ref() != Some(&fitted);
                if changed {
                    self.signals.insert(nm, fitted);
                }
                return changed;
            }
        }

        // §7.4.2: `m[i][j] = v` on a class-property dynamic array of dynamic
        // arrays writes element `j` of row `<h>#m[i]`, the key reads use. The
        // arms below took `[j]` for a bit-select of the row's own cell.
        if matches!(expr.kind, ExprKind::Index { .. })
            && !self.no_class_objects()
            && let Some(row) = self.coll_elem_expr_key(expr)
            && row.contains('#')
            && self.module.dynamic_arrays.contains(&row)
            && self.coll_shape_at(&row).is_some_and(|(shape, depth)| {
                matches!(shape.dims.get(depth), Some(Dim::Dyn))
            })
        {
            let Some(j) = self.eval_expr(index).to_index() else {
                return false;
            };
            if j < 0 || j as u64 >= self.get_queue_size(&row) {
                return false;
            }
            let key = format!("{}[{}]", row, j);
            let elem_is_string = self.is_string_collection(&row)
                || self.is_string_collection(&key)
                || self
                    .p_elem_type_ref(&row)
                    .is_some_and(|dt| Self::is_string_data_type(&dt));
            let prev = self.signals.get(&key);
            let elem_dt = || {
                self.p_elem_type_ref(&row)
                    .or_else(|| self.class_member_coll_type(&row).map(std::borrow::Cow::Borrowed))
            };
            let declared_slot = || -> Option<Value> {
                let w = self
                    .module
                    .arrays
                    .get(&row)
                    .map(|&(_, _, w)| w)
                    .or_else(|| self.assoc_store_ctx_width(&row))?;
                let signed = self
                    .p_elem_type_ref(&row)
                    .or_else(|| self.class_member_coll_type(&row).map(std::borrow::Cow::Borrowed))
                    .is_some_and(|dt| {
                        super::elaborate::is_type_signed_resolved(&dt, &self.module.typedef_types)
                    });
                let mut p = Value::zero(w.max(1));
                p.is_signed = signed;
                Some(p)
            };
            let mut v = if elem_is_string {
                val.clone()
            } else if val.is_real {
                if let Some(p) = prev {
                    if p.is_real {
                        Value::from_f64(val.to_f64())
                    } else {
                        Self::real_into_slot(p, val.to_f64())
                    }
                } else if elem_dt().is_some_and(|dt| self.dt_is_real(&dt)) {
                    Value::from_f64(val.to_f64())
                } else if let Some(p) = declared_slot() {
                    Self::real_into_slot(&p, val.to_f64())
                } else {
                    val.clone()
                }
            } else if let Some(p) = prev {
                if !p.is_real && p.width > 0 {
                    val.resize_for_assign(p.width)
                } else if p.is_real {
                    Value::from_f64(val.to_f64())
                } else {
                    val.clone()
                }
            } else if let Some(w) = self
                .module
                .arrays
                .get(&row)
                .map(|&(_, _, w)| w)
                .or_else(|| self.assoc_store_ctx_width(&row))
            {
                val.resize_for_assign(w.max(1))
            } else {
                val.clone()
            };
            if let Some(dt) = elem_dt() {
                if super::elaborate::is_type_two_state_resolved(&dt, &self.module.typedef_types) {
                    v = v.to_two_state();
                }
            }
            let changed = self.signals.get(&key) != Some(&v);
            if changed {
                self.write_coll_elem(&key, v);
                self.touch_queue(&row);
            }
            return changed;
        }
        // Element write into an ARRAY-OF-COLLECTIONS class property
        // (`expected[1][0][k] = v`, `dq[1][2] = v`, `aa[0][key] = v`):
        // the base names one element collection (see
        // `class_array_of_coll_key`); write its element.
        if let Some(row) = self.class_array_of_coll_key(expr) {
            if self.module.dynamic_arrays.contains(&row) {
                self.dollar_bound.push(self.get_queue_size(&row) as i64 - 1);
                let iv = self.eval_expr(index);
                self.dollar_bound.pop();
                let Some(k) = self.prepare_sequence_write(&row, &iv) else {
                    return false;
                };
                let elem = format!("{}[{}]", row, k);
                // §10.6/§5.7.1: the element takes the row's element width
                // (a `'1` fill fills every bit of a struct element).
                let val = match self.module.arrays.get(&row) {
                    Some(&(_, _, w)) if w != val.width || val.is_fill => val.resize_for_assign(w),
                    _ => val.clone(),
                };
                let prev = self.get_signal_value_by_name(&elem);
                let changed = prev.as_ref() != Some(&val);
                self.set_signal_value_by_name(&elem, val);
                return changed;
            }
            if self.is_associative_array(&row) {
                let kv = self.eval_expr(index);
                let key = self.assoc_key_str(&row, &kv);
                let elem = format!("{}[{}]", row, key);
                let prev = self.get_signal_value_by_name(&elem);
                let changed = prev.as_ref() != Some(val);
                self.set_signal_value_by_name(&elem, val.clone());
                return changed;
            }
        }
        if let Some(qn) = self.indexed_queue_base(expr) {
            // `$` is the last valid index of THIS queue.
            self.dollar_bound.push(self.get_queue_size(&qn) as i64 - 1);
            let iv = self.eval_expr(index);
            self.dollar_bound.pop();
            let Some(k) = self.prepare_sequence_write(&qn, &iv) else {
                return false;
            };
            let elem = format!("{}[{}]", qn, k);
            // §6.12.2: a real rounds to the element type.
            let conv;
            let val = match self.fit_real_nested_elem(expr, val) {
                Some(v) => {
                    conv = v;
                    &conv
                }
                None => val,
            };
            let prev = self.get_signal_value_by_name(&elem);
            let changed = prev.as_ref() != Some(val);
            self.set_signal_value_by_name(&elem, val.clone());
            return changed;
        }
        // `a[i][key] = v` on an array of associative arrays.
        if let Some(elem) = self.indexed_assoc_elem(expr, index) {
            let conv;
            let val = match self.fit_real_nested_elem(expr, val) {
                Some(v) => {
                    conv = v;
                    &conv
                }
                None => val,
            };
            let prev = self.get_signal_value_by_name(&elem);
            let changed = prev.as_ref() != Some(val);
            self.set_signal_value_by_name(&elem, val.clone());
            return changed;
        }
        // Multidimensional associative-array write
        // `m[k1][k2]...[kn] = v`: store under the flat compound key
        // `m[K1][K2]...[Kn]` (see `multidim_assoc_elem`). Without this
        // the write is silently lost and later reads / `.exists()` /
        // recursion guards see nothing (IEEE 1800-2023 §7.8.1).
        if let Some((_, key)) = self.multidim_assoc_elem(expr, index) {
            let conv;
            let val = match self.fit_real_nested_elem(expr, val) {
                Some(v) => {
                    conv = v;
                    &conv
                }
                None => val,
            };
            let changed = self.signals.get(&key).is_none_or(|p| *p != *val);
            if changed {
                self.signals.insert(key, val.clone());
            }
            return changed;
        }
        // `s.arr[i] = v` where `arr` is an UNPACKED ARRAY MEMBER of an
        // unpacked struct. Its elements are individual signals
        // (`s.arr[0]` …) and the BASE `s.arr` is not a signal at all,
        // so every arm here missed and the write degraded into a
        // bit-select of a phantom scalar — silently discarded. The
        // member then read back x forever, which made a scoreboard
        // built on such a struct compare x against every DUT output.
        //
        // Guarded on the base having no signal of its own, so an
        // ordinary packed-vector bit-write (`v[3] = 1`, where `v` IS a
        // signal) never takes this path.
        {
            // `may_create` only for a plain identifier base: that is
            // the procedural-local struct case, where nothing is
            // pre-registered. A MemberAccess base (`q[0].arr[i]`) must
            // find an EXISTING element — creating one there would
            // hijack class-member arrays and virtual-interface writes,
            // which own their own storage.
            let (base, may_create) = match &expr.kind {
                // §8.4: `h.v[i]` with `h` a class handle names the object's
                // property, which the handle arms below write. The resolved
                // name is the bare property name, and a same-named module
                // array's element would take the write here.
                ExprKind::Ident(h)
                    if h.path.len() >= 2
                        && !self.no_class_objects()
                        && self.class_of_var(&h.path[0].name.name).is_some() =>
                {
                    (None, false)
                }
                ExprKind::Ident(h) => (Some(self.resolve_hier_name(h)), true),
                ExprKind::MemberAccess { .. } => (
                    self.flat_member_name(expr).map(std::borrow::Cow::Owned),
                    false,
                ),
                _ => (None, false),
            };
            // A member of a PACKED struct is a bit-slice of the
            // parent signal, not element storage — `foo.a[5] = v` on
            // `struct packed { bit [7:0][7:0] a; ... } foo;` must
            // splice into `foo`. Creating a phantom "foo.a[5]" here
            // swallowed the write silently (the RANGE form worked,
            // which is how it hid). Skip so it reaches the packed
            // element/bit-select arms below.
            let base_is_packed_member = |sim: &Self, b: &str| -> bool {
                if sim.module.packed_signal_elem_widths.contains_key(b) {
                    return true;
                }
                b.rsplit_once('.').is_some_and(|(root, leaf)| {
                    sim.module.packed_struct_fields.get(root).is_some_and(|fs| {
                        fs.iter()
                            .any(|(f, _, _)| f == leaf || f.starts_with(&format!("{}.", leaf)))
                    })
                })
            };
            // §8.3 / §23.9: inside a method, a BARE array name that the
            // object (or its class's static store) declares is the
            // member — a same-named module-level array's elements, found
            // in the signal table first, took `a[i] = v` from a method's
            // `foreach (a[i])` and the object's `a` stayed 0 (sibling of
            // #519 / #534: the name-keyed store must not outrank the
            // member lift that the read side and `expr_assoc_name` use).
            let member_shadows = |sim: &Self, b: &str| -> bool {
                !b.contains('.')
                    && matches!(sim.this_stack.last(), Some(Some(_)))
                    && sim.instance_assoc_member(b).is_some()
            };
            if let Some(base) = base.filter(|b| !base_is_packed_member(self, b)) {
                // An ASSOCIATIVE member (`s.aa["k"] = v`) is keyed, not
                // indexed: the keyed write below formats its key (§7.8).
                if !self.signal_name_to_id.contains_key(base.as_ref())
                    && !self.signals.contains_key(&*base)
                    && !self.module.associative_arrays.contains_key(base.as_ref())
                    && !member_shadows(self, &base)
                {
                    let Some(i) = self.eval_expr(index).to_index() else {
                        return false;
                    };
                    // §7.4.1: a bit-select into a FRAME-local unpacked-struct
                    // MEMBER leaf (`rw_access.byte_en[z] = ...` inside a
                    // procedural block / foreach): the member is pre-
                    // registered as its own leaf in the current call/local
                    // frame (`rw_access.byte_en`), NOT as a registered design
                    // signal, so the generic arms below would mint a phantom
                    // `rw_access.byte_en[z]` element that no whole-member read
                    // consults — the write silently vanished and the reg
                    // bit-bash predictor saw byte-enable 0. Presence of the
                    // leaf (`base` in the frame) is the discriminator: a packed
                    // member is one leaf (splice the bit); an unpacked array
                    // member is per-element leaves (`s.arr[i]`), so `base` is
                    // not itself a leaf and the phantom-element path below
                    // still applies.
                    if i >= 0
                        && self
                            .local_stack
                            .last()
                            .is_some_and(|f| f.contains_key(base.as_ref()))
                        && !self.frame_name_is_string(&base)
                    {
                        // The frame's value takes the bit in place: a copy
                        // of the whole local for each bit written made a
                        // bit loop over a wide local quadratic.
                        let bit = i as usize;
                        if let Some(cur) = self
                            .local_stack
                            .last_mut()
                            .and_then(|f| f.get_mut(base.as_ref()))
                            && bit < cur.width as usize
                        {
                            let nb = val.bits_first();
                            let changed = cur.get_bit(bit) != nb;
                            if changed {
                                cur.set_bit(bit, nb);
                            }
                            return changed;
                        }
                    }
                    let elem = Self::name_with_index(&base, i);
                    // A module-level struct has its element signals
                    // pre-registered. A PROCEDURAL-LOCAL one does not —
                    // nothing is registered for it at all — so create the
                    // element on first write. Restricted to a member path
                    // (`base` contains a dot), so a plain local vector's
                    // bit-write is never diverted here.
                    if self.signal_name_to_id.contains_key(elem.as_str())
                        || self.signals.contains_key(&elem)
                        || (may_create && base.contains('.'))
                    {
                        // A block-local fixed array's element is fitted to
                        // its declared type (see `fit_block_local_array_elem`).
                        let fitted = self.fit_block_local_array_elem(&base, &elem, val.clone());
                        let prev = self.get_signal_value_by_name(&elem);
                        let changed = prev.as_ref() != Some(&fitted);
                        self.set_signal_value_by_name(&elem, fitted);
                        return changed;
                    }
                    // An element of a CLASS-PROPERTY collection of structs
                    // (`q[0].arr[i]` in a method) keeps its leaves under the
                    // instance (`<h>#q[0].arr[i]`).
                    if let Some(elem) = self.class_coll_elem_leaf(&base, i) {
                        let prev = self.get_signal_value_by_name(&elem);
                        let changed = prev.as_ref() != Some(val);
                        self.set_signal_value_by_name(&elem, val.clone());
                        return changed;
                    }
                }
            }
        }
        // BIT-select write into a PACKED-STRUCT MEMBER
        // (`t.f2[3] = 1'b1`): the member is a bit-slice of the parent
        // signal, and no arm below handles a MemberAccess base — the
        // write fell through and vanished (the RANGE form worked,
        // which is how it hid). Read the member slice, set the bit,
        // splice back through the central by-name setter.
        if matches!(&expr.kind, ExprKind::MemberAccess { .. })
            || matches!(&expr.kind, ExprKind::Ident(h) if h.path.len() > 1)
        {
            if let Some(base) = self.flat_member_name(expr) {
                // Only the plain single-descending-dimension member:
                // a MULTI-DIM member (`foo.a[5]` on bit [7:0][7:0])
                // selects an ELEMENT, and an ascending member mirrors
                // its bit labels — both belong to the arms below.
                let multi_dim = self
                    .module
                    .packed_signal_elem_widths
                    .get(&base)
                    .is_some_and(|&ew| ew > 1)
                    || self
                        .module
                        .packed_full_dims
                        .get(&base)
                        .is_some_and(|d| d.len() > 1);
                let ascending = base
                    .rsplit_once('.')
                    .is_some_and(|(root, leaf)| self.struct_member_ascending(root, leaf).is_some());
                if !multi_dim && !ascending && self.packed_leaf_of_hier(&base).is_some() {
                    let idx_v = self.eval_expr(index);
                    if idx_v.has_xz() {
                        return false; // §11.5.1: x/z index selects nothing
                    }
                    let i = idx_v.to_i64().unwrap_or(0);
                    if let Some(mut cur) = self.get_signal_value_by_name(&base) {
                        if i >= 0 && (i as u32) < cur.width {
                            let prev = cur.clone();
                            cur.set_bit(i as usize, val.get_bit(0));
                            let changed = cur != prev;
                            if changed {
                                self.set_signal_value_by_name(&base, cur);
                            }
                            return changed;
                        }
                        return false; // out-of-range bit write discarded
                    }
                }
            }
        }
        // Ascending packed vector bit-write (`logic [0:7] pa; pa[i]=b`):
        // label i targets internal bit (W-1)-i (LRM §7.4.1, §11.5.1).
        if let ExprKind::Ident(h) = &expr.kind {
            let nm = self.resolve_hier_name(h);
            if let Some((lo_l, hi_l)) = self.module.ascending_packed.get(&*nm).copied() {
                if let Some(&id) = self.signal_name_to_id.get(nm.as_ref()) {
                    // §11.5.1: an x/z or out-of-range label writes nothing.
                    let Some(i) = self.eval_expr(index).to_index() else {
                        return false;
                    };
                    if i >= lo_l && i <= hi_l {
                        let pos = (hi_l - i) as usize;
                        let mut cur = self.signal_table[id].clone();
                        let prev = cur.clone();
                        cur.set_bit(pos, val.get_bit(0));
                        let changed = cur != prev;
                        if changed {
                            self.signal_table[id] = cur;
                            self.sync_mirror(id);
                            self.table_modified = true;
                            self.after_signal_write(id);
                            self.mark_dirty(&nm);
                        }
                        return changed;
                    }
                }
            }
        }
        // Class associative-array member element write — base may be
        // a bare member (`m[k]=v`) or another object's (`obj.m[k]=v`).
        // Not a BIT of an associative element whose element is a plain
        // integral (`m[k][3] = 1`): that is no collection, and storing the
        // value under `m[k][3]` dropped the bit (§11.5.1).
        let elem_bit = |me: &mut Self| -> bool {
            let ExprKind::Index { expr: b, .. } = &expr.kind else {
                return false;
            };
            let ExprKind::Ident(h) = &b.kind else {
                return false;
            };
            let n = me.resolve_hier_name(h).into_owned();
            let n = me.instance_assoc_member(&n).unwrap_or(n);
            me.is_associative_array(&n)
                && !me.assoc_has_coll_elems(&n)
                && me.assoc_missing_elem_value(&n).is_some()
        };
        if let Some(an) = self.expr_assoc_name(expr).filter(|_| !elem_bit(self)) {
            let idx_val = self.eval_expr(index);
            let idx_str = self.assoc_key_str(&an, &idx_val);
            let elem_name = Self::name_with_key(&an, &idx_str);
            let is_fixed_array = self.module.arrays.contains_key(&an)
                && !self.module.dynamic_arrays.contains(&an)
                && !self.module.queue_vars.contains(&an);
            if !self.is_associative_array(&an) && !is_fixed_array
                && self.prepare_sequence_write(&an, &idx_val).is_none()
            {
                return false;
            }
            // §10.7: fit to the DECLARED element width when one was
            // recorded; class-member collections often have none, in
            // which case the value is stored as-is (unchanged).
            let mut fitted = match self.assoc_elem_width(&an) {
                Some(w) if (w != val.width || val.is_fill) && !val.is_real => val.resize_for_assign(w),
                // §6.12.2: a real into an INTEGRAL element rounds to the
                // declared width (`assoc_store_ctx_width` is None for a real
                // element, which keeps the value).
                Some(_) if val.is_real => match self.assoc_store_ctx_width(&an) {
                    Some(w) => Self::real_to_int(val.to_f64(), w),
                    None => val.clone(),
                },
                _ => self.fit_block_local_array_elem(&an, &elem_name, val.clone()),
            };
            // The element takes its declared signedness, not the RHS's
            // (`byte m[int]; m[k] = 8'hF0` is -16).
            if let Some(sg) = self
                .assoc_elem_signed(&an)
                .filter(|_| !fitted.is_real && self.is_associative_array(&an))
            {
                fitted.is_signed = sg;
            } else if !self.is_associative_array(&an) {
                // §7.10/§10.7: a queue or dynamic-array element takes the
                // element type (an `int unsigned` element of -1 is 2**32-1).
                fitted = self.fit_queue_push_elem(&an, fitted);
                if !fitted.is_real && !fitted.is_signed && self.signed_class_coll(&an) {
                    fitted = self.fit_signed_class_coll_elem(&an, fitted);
                }
            }
            let changed = self.signals.get(&elem_name) != Some(&fitted);
            self.signals.insert(elem_name, fitted);
            if changed {
                self.touch_queue(&an);
            }
            return changed;
        }
        // N-dimensional (N >= 3) unpacked array element assignment
        {
            let mut cur = expr.as_ref();
            let mut rev_idxs: Vec<&Expression> = vec![index];
            while let ExprKind::Index {
                expr: inner_e,
                index: inner_i,
            } = &cur.kind
            {
                rev_idxs.push(inner_i.as_ref());
                cur = inner_e.as_ref();
            }
            if let ExprKind::Ident(hier) = &cur.kind {
                let base_name = self.multidim_array_store(hier, cur, |s, n| s.module.arrays_nd.contains_key(n));
                if let Some((shape, _w)) = self.module.arrays_nd.get(&*base_name).cloned() {
                    if rev_idxs.len() == shape.len() {
                        let mut name = base_name.clone();
                        for i in (0..rev_idxs.len()).rev() {
                            let v = self.eval_expr(rev_idxs[i]).to_u64().unwrap_or(0) as i64;
                            name = std::borrow::Cow::Owned(format!("{}[{}]", name, v));
                        }
                        if let Some(&id) = self.signal_name_to_id.get(name.as_ref()) {
                            let width = self.signal_widths[id];
                            let resized = val.resize_for_assign(width);
                            let changed = self.signal_table[id] != resized;
                            if changed {
                                if !self.dirty_signals[id] {
                                    self.dirty_signals[id] = true;
                                    self.dirty_list.push(id);
                                }
                                self.dirty_any = true;
                                write_sig!(self, id, resized);
                                self.table_modified = true;
                            }
                            return changed;
                        }
                        let changed = self.signals.get(&*name).is_none_or(|p| *p != *val);
                        if changed {
                            self.signals.insert(name.to_string(), val.clone());
                            self.mark_dirty(&name);
                        }
                        return changed;
                    }
                }
                // §7.4.5 / §11.5.1: one index MORE than a 2-D/N-D array has
                // unpacked dimensions selects INSIDE one element
                // (`logic [2:0] m [1:0][1:0]; m[i][j][k] = b`): the element
                // `m[i][j]` is a signal of its own and `[k]` a bit of it (a
                // slice, for a multi-D packed element). No arm took that
                // shape, so the write was silently dropped, procedurally and
                // as a continuous assign alike. A 2-D class-member element
                // has no compact id (it lives only in the by-name store),
                // so the id lookup alone dropped `row[i][j][k] = 1` inside a
                // method (#1331) — fall back to a by-name RMW bit write.
                if rev_idxs.len() >= 3 {
                    let depth = match self.module.arrays_nd.get(&*base_name) {
                        Some((shape, _)) => shape.len(),
                        None if self.module.arrays_2d.contains_key(&*base_name) => 2,
                        None => 0,
                    };
                    if depth >= 2 && rev_idxs.len() == depth + 1 {
                        let mut elem = base_name.to_string();
                        for ix in rev_idxs[1..].iter().rev() {
                            let v = self.eval_expr(ix);
                            let Some(k) = v.to_i64().filter(|_| !v.has_xz()) else {
                                return false; // §7.4.6: an x/z index writes nothing
                            };
                            elem.push_str(&format!("[{}]", k));
                        }
                        if let Some(&id) = self.signal_name_to_id.get(elem.as_str()) {
                            if let Some(changed) =
                                self.store_unpacked_elem_select(&base_name, &elem, id, index, val)
                            {
                                return changed;
                            }
                        } else if !self.module.dynamic_arrays.contains(&elem)
                            && !self.is_associative_array(&elem)
                            && !self.signals.contains_key(&format!("{}.size", elem))
                        {
                            // 2-D MEMBER element: RMW the bit through the
                            // canonical by-name accessors, defaulting a
                            // never-written element to zero at its declared
                            // width.
                            let ew = match self.module.arrays_2d.get(&*base_name) {
                                Some(&(_, _, w)) => w as u32,
                                None => self
                                    .module
                                    .arrays_nd
                                    .get(&*base_name)
                                    .map(|(_, w)| *w as u32)
                                    .unwrap_or(0),
                            };
                            if let Some(cur) = self
                                .get_signal_value_by_name(&elem)
                                .or_else(|| (ew > 0).then(|| Value::from_u64(0, ew)))
                            {
                                let bv = self.eval_expr(index);
                                if let Some(b) = bv.to_i64().filter(|_| !bv.has_xz()) {
                                    if b >= 0 && (b as u32) < cur.width {
                                        let mut nv = cur.clone();
                                        let nb = val.get_bit(0);
                                        let changed = nv.get_bit(b as usize) != nb;
                                        if changed {
                                            nv.set_bit(b as usize, nb);
                                            self.set_signal_value_by_name(&elem, nv);
                                            self.mark_dirty(&elem);
                                        }
                                        return changed;
                                    }
                                    // Out-of-range bit write: no-op.
                                    return false;
                                }
                            }
                        }
                    }
                }
            }
        }
        // §7.4.1: bit-select of an element of a 1-D UNPACKED array
        // (`logic [1:0] q [0:1]; q[i][j] = b`). Each element is its own
        // signal (`q[i]`), so this is a BIT write into that signal — not
        // a 2-D array index. The `arrays_2d` arm below never matched it
        // and every other arm fell through, so the write was silently
        // DROPPED: a pipeline written as `vld_q[i+1][j] <= vld_q[i][j]`
        // never advanced and its output stayed x forever.
        if let ExprKind::Index {
            expr: inner_expr,
            index: inner_idx,
        } = &expr.kind
        {
            if let ExprKind::Ident(hier) = &inner_expr.kind {
                let mut name = self.resolve_hier_name(hier);
                if !self.module.arrays.contains_key(&*name) {
                    if let Some(s) = self.instance_assoc_member(&name) {
                        name = std::borrow::Cow::Owned(s);
                    }
                }
                // Only a plain unpacked array: an `arrays_2d` entry is a
                // genuine two-dimensional index and is handled below.
                if self.module.arrays.contains_key(&*name)
                    && !self.module.arrays_2d.contains_key(&*name)
                    && !self.module.arrays_nd.contains_key(&*name)
                {
                    let i = self.eval_expr(inner_idx).to_i64().unwrap_or(0);
                    let elem = format!("{}[{}]", name, i);
                    if let Some(&id) = self.signal_name_to_id.get(elem.as_str()) {
                        if let Some(changed) =
                            self.store_unpacked_elem_select(&name, &elem, id, index, val)
                        {
                            return changed;
                        }
                    }
                }
                // §7.4.1 / §10.4: bit-select write into an element of
                // an UNREGISTERED (block-local / automatic
                // subroutine-local) unpacked array. The guarded path
                // above needs a module `arrays` registration + compact
                // id; a block-local `larr[1][2] = 1'b1` had neither,
                // and every later arm either targets arrays_2d or
                // invents a bogus "larr[1][2]" key that element reads
                // never see — the write silently vanished. RMW the
                // element through the canonical by-name accessors.
                // Block-local arrays DO appear in `module.arrays`
                // (the declaration registers them at runtime, after
                // the compact tables were built), so the only reliable
                // discriminator is the missing compact id itself.
                // Collection elements (array-of-queue, where `[j]`
                // selects a sub-element, not a bit) were already taken
                // by the earlier queue/assoc branches — and are
                // excluded again here for safety.
                {
                    let iv = self.eval_expr(inner_idx);
                    if let Some(i) = iv.to_i64().filter(|_| !iv.has_xz()) {
                        let elem = format!("{}[{}]", name, i);
                        // Only a RUNTIME-declared element (no compact
                        // id) takes this route: a registered element
                        // that fell through above did so deliberately
                        // (negative packed labels etc.) and must reach
                        // the pre-existing normalizing arms below.
                        let elem_is_collection = self.module.dynamic_arrays.contains(&elem)
                            || self.is_associative_array(&elem);
                        if !elem_is_collection
                            && !self.signal_name_to_id.contains_key(elem.as_str())
                        {
                            if let Some(cur) = self.get_signal_value_by_name(&elem) {
                                let bv = self.eval_expr(index);
                                if let Some(b) = bv.to_i64().filter(|_| !bv.has_xz()) {
                                    if b >= 0 && (b as u32) < cur.width {
                                        let mut nv = cur.clone();
                                        let nb = val.get_bit(0);
                                        let changed = nv.get_bit(b as usize) != nb;
                                        if changed {
                                            nv.set_bit(b as usize, nb);
                                            self.set_signal_value_by_name(&elem, nv);
                                            self.mark_dirty(&elem);
                                        }
                                        return changed;
                                    }
                                    // Out-of-range bit write on a
                                    // runtime-declared element: no-op.
                                    return false;
                                }
                            }
                        }
                    }
                }
            }
        }
        // 2D array element assignment: mem[i][j] = val
        if let ExprKind::Index {
            expr: inner_expr,
            index: inner_idx,
        } = &expr.kind
        {
            if let ExprKind::Ident(hier) = &inner_expr.kind {
                let mut name = self.multidim_array_store(hier, inner_expr, |s, n| s.module.arrays_2d.contains_key(n));
                // A bare name written from inside a SUBMODULE — e.g. the
                // body of a `foreach` in an `always_comb`, evaluated from
                // the settle path where no scope hint is installed and the
                // per-node name cache may already hold the unscoped form.
                // Try the hint, then a UNIQUE suffix match, so the write
                // lands instead of being silently dropped.
                if !self.module.arrays_2d.contains_key(&*name) && !name.contains('.') {
                    let hinted = self
                        .name_resolve_hint
                        .borrow()
                        .as_ref()
                        .map(|h| format!("{}.{}", h, name));
                    if let Some(h) = hinted.filter(|h| self.module.arrays_2d.contains_key(h)) {
                        name = std::borrow::Cow::Owned(h);
                    } else {
                        let suffix = format!(".{}", name);
                        let mut hit: Option<String> = None;
                        let mut many = false;
                        for k in self.module.arrays_2d.keys() {
                            if k.ends_with(&suffix) {
                                if hit.is_some() {
                                    many = true;
                                    break;
                                }
                                hit = Some(k.clone());
                            }
                        }
                        if let (Some(k), false) = (hit, many) {
                            name = std::borrow::Cow::Owned(k);
                        }
                    }
                }
                if self.module.arrays_2d.contains_key(&*name) {
                    // §7.4.5 / §11.5.1: unpacked indices are signed;
                    // an unknown or out-of-bounds index performs no write.
                    let (Some(i), Some(j)) = (
                        self.eval_expr(inner_idx).to_index(),
                        self.eval_expr(index).to_index(),
                    ) else {
                        return false;
                    };
                    let &((il, ih), (jl, jh), _) = &self.module.arrays_2d[&*name];
                    let (outer_lo, outer_hi) = if self.module.dynamic_arrays.contains(&*name) {
                        // The 2-D table's initial allocation is a placeholder;
                        // new[N] sets the live outer dimension (§7.5.1).
                        (0, self.get_queue_size(&name) as i64 - 1)
                    } else {
                        (il.min(ih), il.max(ih))
                    };
                    if i < outer_lo || i > outer_hi || j < jl.min(jh) || j > jl.max(jh) {
                        return false;
                    }
                    let elem_name = format!("{}[{}][{}]", name, i, j);
                    if let Some(&id) = self.signal_name_to_id.get(elem_name.as_str()) {
                        let width = self.signal_widths[id];
                        let resized = val.resize_for_assign(width);
                        let changed = self.signal_table[id] != resized;
                        if changed {
                            if !self.dirty_signals[id] {
                                self.dirty_signals[id] = true;
                                self.dirty_list.push(id);
                            }
                            self.dirty_any = true;
                            write_sig!(self, id, resized);
                            self.table_modified = true;
                        }
                        return changed;
                    }
                    let changed = self.signals.get(&elem_name).is_none_or(|p| *p != *val);
                    if changed {
                        self.signals.insert(elem_name.clone(), val.clone());
                        self.mark_dirty(&elem_name);
                        // §9.2.2.2: a queue/dyn/assoc ELEMENT has no
                        // signal id of its own — comb readers depend on
                        // the collection's `.size` proxy, so mark it or
                        // `always_comb` blocks reading `q[i]` never
                        // re-fire on an element write.
                        if let Some(b) = elem_name.find('[').map(|i| &elem_name[..i]) {
                            if self.module.dynamic_arrays.contains(b)
                                || self.module.associative_arrays.contains_key(b)
                            {
                                let b = b.to_string();
                                self.touch_queue(&b);
                            }
                        }
                    }
                    return changed;
                }
            }
        }
        // §7.4.2: an element of a struct member with two or more unpacked
        // dimensions is its own leaf (see `struct_member_elem_leaf`).
        let elem_leaf = if matches!(expr.kind, ExprKind::Index { .. })
            && self.multidim_members != Some(false)
        {
            self.struct_member_elem_leaf(expr, index)
        } else {
            None
        };
        if let Some(leaf) = elem_leaf {
            if let Some(&id) = self.signal_name_to_id.get(leaf.as_str()) {
                let resized = self.fit_value_to_signal(id, val);
                let changed = self.signal_table[id] != resized;
                if changed {
                    if !self.dirty_signals[id] {
                        self.dirty_signals[id] = true;
                        self.dirty_list.push(id);
                    }
                    self.dirty_any = true;
                    write_sig!(self, id, resized);
                    self.table_modified = true;
                }
                return changed;
            }
            let fitted = match self.signals.get(&leaf) {
                Some(p) if p.is_real && !val.is_real => Value::from_f64(val.to_f64()),
                Some(p) if !p.is_real && val.is_real => Self::real_into_slot(p, val.to_f64()),
                Some(p) if !p.is_real && p.width > 0 => {
                    let mut f = val.resize_for_assign(p.width);
                    f.is_signed = p.is_signed;
                    f
                }
                _ => val.clone(),
            };
            let changed = self.signals.get(&leaf) != Some(&fitted);
            if changed {
                self.signals.insert(leaf.clone(), fitted);
                self.mark_dirty(&leaf);
            }
            return changed;
        }
        // Nested packed multi-D element write (`a[i][j] = v` on
        // `reg [1:0][15:0][7:0] a;`) — splice the slice into the flat
        // backing vector, normalizing each index against its declared
        // dimension bounds (ascending / non-zero-based / negative,
        // LRM §7.4.1).
        if let Some((base, lo_opt, w)) = self.packed_nested_select(expr, index) {
            let lo = match lo_opt {
                Some(lo) => lo,
                None => {
                    return false; // out-of-range write dropped §11.5.1
                }
            };
            if let Some(cur_sig) = self.get_signal_value_by_name(&base) {
                let total_w = cur_sig.width as usize;
                if lo + w as usize <= total_w {
                    let mut nv = cur_sig.clone();
                    let piece = val.resize_for_assign(w);
                    let mut changed = false;
                    for i in 0..w as usize {
                        let nb = piece.get_bit(i);
                        if nv.get_bit(lo + i) != nb {
                            nv.set_bit(lo + i, nb);
                            changed = true;
                        }
                    }
                    if changed {
                        self.set_signal_value_by_name(&base, nv);
                        self.mark_dirty(&base);
                    }
                    return changed;
                }
            }
        }
        // Packed multi-D element bit-select write: `p2d[i][j] = bit`
        // where `p2d` is `logic [..][W-1:0]`. The inner Index selects
        // element i (a W-bit slice at bits [i*W +: W]); the outer index
        // j selects bit j within it. Resolve to the flat bit offset
        // i*W + j and write a single bit (LRM §7.4.1, §11.5.1). Without
        // this the lvalue's `expr` is an Index (not an Ident), so the
        // single-index path below is skipped and the write is lost.
        if let ExprKind::Index {
            expr: inner_expr,
            index: outer_idx,
        } = &expr.kind
        {
            if let ExprKind::Ident(hier) = &inner_expr.kind {
                let base = self.resolve_hier_name(hier);
                if let Some(&elem_w) = self.module.packed_signal_elem_widths.get(&*base) {
                    if let Some(&id) = self.signal_name_to_id.get(base.as_ref()) {
                        let total_w = self.signal_widths[id] as usize;
                        let i = self.eval_expr(outer_idx).to_u64().unwrap_or(0) as usize;
                        let j = self.eval_expr(index).to_u64().unwrap_or(0) as usize;
                        let lo = i * (elem_w as usize) + j;
                        if lo < total_w {
                            let mut cur = self.signal_table[id].clone();
                            let prev = cur.clone();
                            cur.set_bit(lo, val.get_bit(0));
                            let changed = cur != prev;
                            if changed {
                                self.signal_table[id] = cur;
                                self.sync_mirror(id);
                                self.table_modified = true;
                                self.after_signal_write(id);
                                self.mark_dirty(&base);
                            }
                            return changed;
                        }
                    }
                    // Block-local packed multi-D var: lives in the
                    // runtime map, not the compact table.
                    if let Some(cur) = self.get_signal_value_by_name(&base) {
                        let total_w = cur.width as usize;
                        let i = self.eval_expr(outer_idx).to_u64().unwrap_or(0) as usize;
                        let j = self.eval_expr(index).to_u64().unwrap_or(0) as usize;
                        let lo = i * (elem_w as usize) + j;
                        if lo < total_w {
                            let mut nv = cur.clone();
                            nv.set_bit(lo, val.get_bit(0));
                            let changed = nv != cur;
                            if changed {
                                self.set_signal_value_by_name(&base, nv);
                            }
                            return changed;
                        }
                    }
                }
            }
        }
        // LRM §25.8 — `vif.member[i] = ...` (bit-select write
        // through a virtual-interface property). The lvalue parses as
        // Index { expr: MemberAccess { Ident(vif_prop), signal_member },
        //          index }.  The scalar-write path (ExprKind::MemberAccess
        // arm below) handles `vif.member = ...` but misses the indexed
        // form, causing all bit-select writes to vif-bound signals to be
        // silently lost.  Detect and redirect here before the generic
        // flat-name path, which has no knowledge of vif bindings.
        if let ExprKind::MemberAccess {
            expr: ma_base,
            member: ma_member,
        } = &expr.kind
        {
            if let ExprKind::Ident(ma_hier) = &ma_base.kind {
                if ma_hier.path.len() == 1 {
                    let vif_prop = ma_hier.path[0].name.name.as_str();
                    let sig_member = ma_member.name.as_str();
                    // Resolve virtual-interface binding (same lookup as
                    // the scalar MemberAccess arm below ~line 20401).
                    let binding = self
                        .this_stack
                        .last()
                        .copied()
                        .flatten()
                        .and_then(|this_h| {
                            let cls_name = self
                                .heap
                                .get(this_h)
                                .and_then(|o| o.as_ref())
                                .map(|i| i.class_name.clone())?;
                            self.class_vif_decl(&cls_name, vif_prop).map(|_| ()).and(
                                self.virtual_iface_bindings
                                    .get(&(this_h, vif_prop.to_string()))
                                    .cloned(),
                            )
                        });
                    if let Some((bound_name, _modport)) = binding {
                        let target = format!("{}.{}", bound_name, sig_member);
                        let idx = self.eval_expr(index).to_u64().unwrap_or(0) as usize;
                        if let Some(prev) = self.get_signal_value_by_name(&target) {
                            let w = prev.width as usize;
                            if idx < w {
                                let nb = val.get_bit(0);
                                let old = prev.get_bit(idx);
                                let changed = old != nb;
                                if changed {
                                    let mut nv = prev.clone();
                                    nv.set_bit(idx, nb);
                                    self.set_signal_value_by_name(&target, nv);
                                }
                                return changed;
                            }
                        }
                    }
                }
            }
        }
        // The array base may be a flat/hier Ident (`mem[i]`,
        // rewritten submodule refs) OR a MemberAccess chain
        // (`u_h.mem[q]` from a parent scope) — the latter never
        // matched here, so hierarchical element writes were silently
        // dropped while reads (which go through flat_member_name)
        // worked.
        // Element write into a packed multi-dimensional CLASS property:
        // read-modify-write the addressed slice of the whole value.
        if let Some((h, prop, idx_exprs)) = self.prop_index_chain(lhs) {
            if let Some(dims) = self.class_prop_packed_dims(h, &prop) {
                if idx_exprs.len() <= dims.len() {
                    let idx: Vec<Value> = idx_exprs.iter().map(|ie| self.eval_expr(ie)).collect();
                    if let Some(cur) = self.read_member_value(h, &prop) {
                        if let Some((lsb, w)) =
                            Self::packed_chain_slice(cur.width as u64, &dims, &idx)
                        {
                            let piece = val.resize_for_assign(w as u32);
                            let mut nv = cur.clone();
                            for i in 0..(w as usize) {
                                nv.set_bit(lsb as usize + i, piece.get_bit(i));
                            }
                            let changed = nv != cur;
                            if changed {
                                if let Some(Some(inst)) = self.heap.get_mut(h) {
                                    inst.properties.insert(prop, nv);
                                }
                            }
                            return changed;
                        }
                        return false;
                    }
                }
            }
        }
        let (base_name, hier_opt): (
            Option<std::borrow::Cow<str>>,
            Option<&crate::ast::expr::HierarchicalIdentifier>,
        ) = match &expr.kind {
            ExprKind::Ident(hier) => (Some(self.resolve_hier_name(hier)), Some(hier)),
            ExprKind::MemberAccess { .. } => (
                self.flat_member_name(expr).map(std::borrow::Cow::Owned),
                None,
            ),
            _ => (None, None),
        };
        if let Some(mut name) = base_name {
            if let Some(scoped) = self.instance_assoc_member(&name) {
                name = std::borrow::Cow::Owned(scoped);
            }
            // §8.9: `Cls::S[i] = v` reaches here with the resolver's
            // bare-leaf collapse of the 2-segment form; the original
            // segments name the qualified store.
            if !self.module.arrays.contains_key(&*name) {
                if let Some(h) = hier_opt {
                    if let Some(k) = self.static_fixed_key_from_hier(h) {
                        name = std::borrow::Cow::Owned(k);
                    }
                }
            }
            // A bare name inside a SUBMODULE process resolves under
            // the process's instance scope (name_resolve_hint), like
            // scalar reads/writes already do.
            if !self.module.arrays.contains_key(&*name) && !self.is_associative_array(&name) {
                if let Some(scoped) = self.hint_scoped(&name) {
                    if self.module.arrays.contains_key(&scoped)
                        || self.is_associative_array(&scoped)
                    {
                        name = std::borrow::Cow::Owned(scoped);
                    }
                }
            }
            let idx_val = self.eval_expr(index);
            // §7.8.2: an associative index containing x/z is invalid.
            // The write is ignored (with a warning) — it must not fold
            // the unknown bits away and clobber a real element.
            if self.is_associative_array(&name) && idx_val.has_xz() {
                self.warn_invalid_assoc_index(&name);
                return false;
            }
            let idx_str = if self.is_associative_array(&name) {
                self.assoc_key_str(&name, &idx_val)
            } else {
                idx_val.to_u64().unwrap_or(0).to_string()
            };
            // Check if this is an array element assignment
            if self.module.arrays.contains_key(&*name) || self.is_associative_array(&name) {
                // Compact-resolver fast path for 1D arrays: compute
                // signal_id directly without `format!()` + HashMap
                // lookup. Required for correctness now that 1D
                // array elements are no longer materialised as
                // per-element entries in signal_name_to_id.
                if let Some(&(first_id, lo, hi)) = self.array_first_id.get(name.as_ref()) {
                    let idx = idx_val.to_u64().unwrap_or(0) as i64;
                    if idx >= lo && idx <= hi {
                        let id = first_id + (idx - lo) as usize;
                        if is_packed_id(id) {
                            return self.fast_signal_write_id(id, val);
                        }
                        let width = self.signal_widths[id];
                        // §6.16: a string element has no declared
                        // length — the width is a placeholder.
                        let mut resized = if self.signal_is_string.get(id).copied().unwrap_or(false)
                        {
                            val.clone()
                        } else {
                            val.resize_for_assign(width)
                        };
                        // §6.11.2/§10.7: a 2-state element (`int a[4]`)
                        // stores an x or z bit of the RHS as 0. Stored
                        // verbatim, `a[k] = a[k] * 3 + x_bit` kept x.
                        if self.signal_two_state.get(id).copied().unwrap_or(false) {
                            resized = resized.to_two_state();
                        }
                        // Adopt the element's declared signedness — a
                        // signed RHS (e.g. an `integer`) stored verbatim
                        // made `reg [15:0] a[]` elements read as signed.
                        resized.is_signed = self.signal_signed[id];
                        let changed = self.signal_table[id] != resized;
                        if changed {
                            if !self.dirty_signals[id] {
                                self.dirty_signals[id] = true;
                                self.dirty_list.push(id);
                            }
                            self.dirty_any = true;
                            write_sig!(self, id, resized);
                            self.table_modified = true;
                        }
                        return changed;
                    }
                }
                let elem_name = format!("{}[{}]", name, idx_str);
                if let Some(&id) = self.signal_name_to_id.get(elem_name.as_str()) {
                    let width = self.signal_widths[id];
                    let mut resized = if self.signal_is_string.get(id).copied().unwrap_or(false) {
                        val.clone()
                    } else {
                        val.resize_for_assign(width)
                    };
                    // §6.11.2: a 2-state element takes x/z bits as 0.
                    if self.signal_two_state.get(id).copied().unwrap_or(false) {
                        resized = resized.to_two_state();
                    }
                    resized.is_signed = self.signal_signed[id];
                    let changed = self.signal_table[id] != resized;
                    if changed {
                        if !self.dirty_signals[id] {
                            self.dirty_signals[id] = true;
                            self.dirty_list.push(id);
                        }
                        self.dirty_any = true;
                        write_sig!(self, id, resized);
                        self.table_modified = true;
                    }
                    return changed;
                }
                // Fallback: slow path / associative array.
                // §10.7: an assoc element has no entry in the typed
                // signal table, so without the elaborator's recorded
                // element width the RHS was stored at ITS OWN size —
                // `logic [3:0] aa[string]; aa["k"] = 8'hEF` kept all
                // eight bits, and `$bits` reported the stored width
                // rather than the declared one.
                // §6.16: a string element has no fixed length — the
                // recorded width is only the elaborator's placeholder,
                // and fitting to it would chop the text.
                let elem_is_string = self.is_string_collection(&name);
                let mut fitted = match self.assoc_elem_width(&name) {
                    Some(w) if (w != val.width || val.is_fill) && !val.is_real && !elem_is_string => {
                        val.resize_for_assign(w)
                    }
                    // §6.12.2: a real into an INTEGRAL element rounds to the
                    // declared width (see the fast path above).
                    Some(_) if val.is_real && !elem_is_string => {
                        match self.assoc_store_ctx_width(&name) {
                            Some(w) => Self::real_to_int(val.to_f64(), w),
                            None => val.clone(),
                        }
                    }
                    _ => val.clone(),
                };
                if !elem_is_string {
                    fitted = self.fit_block_local_array_elem(&name, &elem_name, fitted);
                }
                // The element takes its declared signedness, not the RHS's
                // (`bit [3:0] aa[int]; aa[k] = 9` is 9, not -7).
                if let Some(sg) = self.assoc_elem_signed(&name).filter(|_| {
                    !fitted.is_real && !elem_is_string && self.is_associative_array(&name)
                }) {
                    fitted.is_signed = sg;
                } else if !fitted.is_real && !fitted.is_signed && self.signed_class_coll(&name) {
                    fitted = self.fit_signed_class_coll_elem(&name, fitted);
                }
                if self.warn_x && self.time > 0 {
                    let prev = self.signals.get(&elem_name).cloned();
                    self.warn_x_note_named(&elem_name, prev.as_ref(), &fitted);
                }
                let changed = self.signals.get(&elem_name).is_none_or(|p| *p != fitted);
                if changed {
                    sim_dbg_eprintln!("[DEBUG] signal {} changed to {:?}", elem_name, fitted);
                    self.signals.insert(elem_name.clone(), fitted);
                    self.mark_dirty(&elem_name);
                    // §9.2.2.2: comb readers of `aa[k]` depend on the
                    // collection's `.size` proxy, not the per-key
                    // entry — mark it or they never re-fire.
                    if self.is_associative_array(&name)
                        || self.module.dynamic_arrays.contains(&*name)
                    {
                        self.touch_queue(&name);
                    }
                }
                return changed;
            }
            // String character assignment: `s[i] = "."` replaces the
            // i-th character (byte). `s[0]` is the leftmost char, which
            // sits in the most-significant byte of the packed value.
            if self.frame_name_is_string(&name) {
                // Replace the i-th character of the string CONTENT (not
                // a byte of the fixed-width container, which would write
                // the wrong position and grow the string). Mirrors putc.
                let mut b = self.eval_expr(expr).sv_string_bytes();
                let i = idx_val.to_u64().unwrap_or(0) as usize;
                let cbyte = (val.resize_for_assign(8).to_u64().unwrap_or(0) & 0xFF) as u8;
                if i < b.len() && cbyte != 0 {
                    b[i] = cbyte;
                    return self.assign_value(expr, &Self::sv_string_from_bytes(&b));
                }
                return false;
            }
            // Packed multi-D signal element assignment (LRM §7.4.1):
            // `w[i] = val` writes an elem_w-bit slice into bits
            // [i*elem_w +: elem_w], not a single bit. Triggered when
            // either `name` (resolved hier) or the dotted-Ident form
            // (`pp.words`) is in packed_signal_elem_widths.
            let pkt_key: Option<String> = {
                if self.module.packed_signal_elem_widths.contains_key(&*name) {
                    Some(name.to_string())
                } else if let ExprKind::Ident(h) = &expr.kind {
                    if h.path.len() >= 2 {
                        let dotted: String = h
                            .path
                            .iter()
                            .map(|s| s.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join(".");
                        if self.module.packed_signal_elem_widths.contains_key(&dotted) {
                            Some(dotted)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            if let Some(pkey) = pkt_key {
                let elem_w = *self.module.packed_signal_elem_widths.get(&pkey).unwrap();
                let idx = idx_val.to_u64().unwrap_or(0) as usize;
                // Resolve underlying signal by either `name` or `pkey`.
                let sig_id = self
                    .signal_name_to_id
                    .get(name.as_ref())
                    .copied()
                    .or_else(|| self.signal_name_to_id.get(pkey.as_str()).copied());
                if let Some(id) = sig_id {
                    let total_w = self.signal_widths[id];
                    let mut cur = self.signal_table[id].clone();
                    let piece = val.resize_for_assign(elem_w);
                    // Normalized for ascending / non-zero-based outer
                    // dims (§7.4.1) — raw idx*w mirrored [0:N-1] slots.
                    let Some(lo) = self.packed_elem_lsb(&pkey, idx as i64, elem_w) else {
                        return false;
                    };
                    if lo + (elem_w as usize) <= total_w as usize {
                        let prev = cur.clone();
                        for i in 0..(elem_w as usize) {
                            cur.set_bit(lo + i, piece.get_bit(i));
                        }
                        let changed = cur != prev;
                        if changed {
                            self.signal_table[id] = cur;
                            self.sync_mirror(id);
                            self.table_modified = true;
                            self.after_signal_write(id);
                            self.mark_dirty(&pkey);
                        }
                        return changed;
                    }
                }
                // By-NAME storage (function/task locals, formals):
                // the same elem splice against the runtime signal map.
                // Without this a `u8_vec16_t y; y[i] = v;` inside a
                // subroutine degraded to a single-BIT write (the
                // pixel-conversion battery's silent-wrong-agreement
                // bug: reference models and DUT shared the defect).
                {
                    let key = if self.get_signal_value_by_name(&name).is_some() {
                        Some(name.to_string())
                    } else if self.get_signal_value_by_name(&pkey).is_some() {
                        Some(pkey.clone())
                    } else {
                        None
                    };
                    if let Some(key) = key {
                        if let Some(mut cur) = self.get_signal_value_by_name(&key) {
                            let piece = val.resize_for_assign(elem_w);
                            let Some(lo) = self.packed_elem_lsb(&pkey, idx as i64, elem_w) else {
                                return false;
                            };
                            if lo + (elem_w as usize) <= cur.width as usize {
                                let prev = cur.clone();
                                for i in 0..(elem_w as usize) {
                                    cur.set_bit(lo + i, piece.get_bit(i));
                                }
                                let changed = cur != prev;
                                if changed {
                                    self.set_signal_value_by_name(&key, cur);
                                }
                                return changed;
                            }
                            return false;
                        }
                    }
                }
                // Fallback: for struct field write where the underlying
                // signal is the whole struct (not the field by name),
                // route via packed_struct_fields field offset.
                if let ExprKind::Ident(h) = &expr.kind {
                    if h.path.len() == 2 {
                        let struct_var = &h.path[0].name.name;
                        let field_name = &h.path[1].name.name;
                        if let Some(fields) = self
                            .module
                            .packed_struct_fields
                            .get(struct_var.as_str())
                            .cloned()
                        {
                            if let Some((_, field_off, _field_w)) =
                                fields.iter().find(|(m, _, _)| m == field_name).cloned()
                            {
                                if let Some(&sid) = self.signal_name_to_id.get(struct_var.as_str())
                                {
                                    let total_w = self.signal_widths[sid];
                                    let mut cur = self.signal_table[sid].clone();
                                    let piece = val.resize_for_assign(elem_w);
                                    let lo = (field_off as usize) + idx * (elem_w as usize);
                                    if lo + (elem_w as usize) <= total_w as usize {
                                        let prev = cur.clone();
                                        for i in 0..(elem_w as usize) {
                                            cur.set_bit(lo + i, piece.get_bit(i));
                                        }
                                        let changed = cur != prev;
                                        if changed {
                                            self.signal_table[sid] = cur;
                                            self.sync_mirror(sid);
                                            self.table_modified = true;
                                            self.after_signal_write(sid);
                                            self.mark_dirty(struct_var);
                                        }
                                        return changed;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // Fall back to bit select assignment
            let idx = idx_val.to_u64().unwrap_or(0) as usize;
            // Bit-select write on a LOCAL packed vector (e.g.
            // `bit [7:0] seen; seen[i] = 1'b1;` inside an initial /
            // task). Locals live in local_stack, not signal_table —
            // mutate the Value in place.
            if let (false, Some(hier)) = (self.local_stack.is_empty(), hier_opt) {
                let last_idx = self.local_stack.len() - 1;
                if let Some(cur) = self.local_stack[last_idx].get(&hier.path[0].name.name) {
                    if idx < cur.width as usize {
                        let nb = val.bits_first();
                        let old = cur.get_bit(idx);
                        let c = old != nb;
                        if c {
                            let mut nv = cur.clone();
                            nv.set_bit(idx, nb);
                            self.local_stack[last_idx].insert(hier.path[0].name.name.clone(), nv);
                        }
                        return c;
                    }
                }
            }
            // Bit-select on a `this.<prop>` instance field.
            if let (Some(Some(handle)), Some(hier)) = (self.this_stack.last().copied(), hier_opt) {
                if hier.path.len() == 1 {
                    let pname = hier.path[0].name.name.clone();
                    if let Some(Some(inst)) = self.heap.get_mut(handle) {
                        if let Some(cur) = inst.properties.get(&pname).cloned() {
                            if idx < cur.width as usize {
                                let nb = val.bits_first();
                                let old = cur.get_bit(idx);
                                let c = old != nb;
                                if c {
                                    let mut nv = cur.clone();
                                    nv.set_bit(idx, nb);
                                    inst.properties.insert(pname, nv);
                                }
                                return c;
                            }
                        }
                    }
                }
            }
            // Bit select needs signal_table
            if let Some(&id) = self.signal_name_to_id.get(name.as_ref()) {
                if idx < self.signal_widths[id] as usize {
                    let nb = val.bits_first();
                    let old = self.signal_table[id].get_bit(idx);
                    let c = old != nb;
                    if c {
                        self.signal_table[id].set_bit(idx, nb);
                        self.sync_mirror(id);
                        self.table_modified = true;
                        self.after_signal_write(id);
                        self.mark_dirty(&name);
                    }
                    return c;
                }
            }
            // Slow-path bit-select on signals HashMap (block-local
            // packed vectors hoisted to module signals without a
            // signal_id, e.g. `bit [7:0] x;` declared inside an
            // initial begin/task). Without this branch the write
            // would silently no-op.
            if let Some(cur) = self.signals.get(&*name).cloned() {
                if idx < cur.width as usize {
                    let nb = val.bits_first();
                    let old = cur.get_bit(idx);
                    let c = old != nb;
                    if c {
                        let mut nv = cur;
                        nv.set_bit(idx, nb);
                        self.signals.insert(name.to_string(), nv);
                        self.mark_dirty(&name);
                    }
                    return c;
                }
            }
            // Bit-select write into a FRAME-owned (block-local) struct
            // /vector member leaf. A whole-member store (`s.m = v`) puts
            // the leaf in the current call/local frame under `s.m`, but the
            // indexed form (`s.m[i] = v`) only probed the signal maps above
            // and silently dropped the write — `uvm_reg_bus_op.byte_en[z] =
            // be[…]` in uvm_reg_map::do_bus_access left every byte_en 0, so
            // the reg bit-bash predictor saw a disabled byte and failed.
            if let Some(cur) = self
                .local_stack
                .last()
                .and_then(|f| f.get(name.as_ref()))
                .cloned()
            {
                if idx < cur.width as usize {
                    let nb = val.bits_first();
                    let old = cur.get_bit(idx);
                    let c = old != nb;
                    if c {
                        let mut nv = cur;
                        nv.set_bit(idx, nb);
                        if let Some(frame) = self.local_stack.last_mut() {
                            frame.insert(name.to_string(), nv);
                        }
                    }
                    return c;
                }
            }
        }
        false
    }

    pub(in crate::compiler::simulator) fn class_collection_member_select_steps<'e>(
        &self,
        expr: &'e Expression,
    ) -> Option<(
        &'e Expression,
        crate::ast::types::StructUnionType,
        Vec<PendingSel<'e>>,
    )> {
        let mut steps = Vec::new();
        let mut saw_member = false;
        let mut cur = expr;
        loop {
            if saw_member && matches!(cur.kind, ExprKind::Index { .. }) {
                let shadowed_local = !self.local_stack.is_empty() && {
                    let mut root = cur;
                    while let ExprKind::Index { expr: b, .. } = &root.kind {
                        root = b;
                    }
                    matches!(&root.kind, ExprKind::Ident(h) if h.path.len() == 1
                        && h.path[0].selects.is_empty()
                        && self.local_stack.last().is_some_and(|l| l.contains_key(&h.path[0].name.name)))
                };
                if !shadowed_local && let Some(su) = self.packed_struct_elem_type(cur) {
                    steps.reverse();
                    return Some((cur, su, steps));
                }
            }
            match &cur.kind {
                ExprKind::Paren(inner) => cur = inner,
                ExprKind::Index { expr: base, index } => {
                    steps.push(PendingSel::Index(index));
                    cur = base;
                }
                ExprKind::RangeSelect {
                    expr: base,
                    kind,
                    left,
                    right,
                } => {
                    steps.push(PendingSel::Range {
                        kind: *kind,
                        left,
                        right,
                    });
                    cur = base;
                }
                ExprKind::MemberAccess { expr: base, member } => {
                    saw_member = true;
                    steps.push(PendingSel::Member(&member.name));
                    cur = base;
                }
                _ => return None,
            }
        }
    }

    pub(in crate::compiler::simulator) fn class_collection_member_select_write(
        &mut self,
        lhs: &Expression,
        value: &Value,
    ) -> Option<bool> {
        let (elem, su, steps) = self.class_collection_member_select_steps(lhs)?;
        let ty = DataType::Struct(su);
        let (hops, width, two_state) = self.packed_hops(&ty, &steps)?;
        let (selected_width, range) = self.packed_hops_eval(&hops, &steps);
        let Some((lsb, src, len)) = range else {
            return Some(false);
        };
        let mut current = self.eval_expr(elem).resize(width);
        let fitted = if two_state && value.has_xz() {
            value.to_two_state()
        } else {
            value.clone()
        }
        .resize_for_assign(selected_width);
        current.copy_bits_from(lsb as usize, &fitted, src as usize, len as usize);
        Some(self.assign_value(elem, &current))
    }

    pub(in crate::compiler::simulator) fn class_collection_member_select_read(
        &mut self,
        expr: &Expression,
    ) -> Option<Value> {
        let (elem, su, steps) = self.class_collection_member_select_steps(expr)?;
        let ty = DataType::Struct(su);
        let (hops, width, two_state) = self.packed_hops(&ty, &steps)?;
        let (selected_width, range) = self.packed_hops_eval(&hops, &steps);
        let out_w = selected_width.max(1);
        let mut out = if two_state {
            Value::zero(out_w)
        } else {
            Value::new(out_w)
        };
        if let Some((lsb, src, len)) = range {
            let current = self.eval_expr(elem).resize(width);
            out.copy_bits_from(src as usize, &current, lsb as usize, len as usize);
            if two_state && out.has_xz() {
                out = out.to_two_state();
            }
        }
        Some(out)
    }

    /// `pkg::x`, `pkg::arr[i]`, `pkg::v[h:l]` as an assignment TARGET: package
    /// variables live in the table under their bare name (the read side
    /// strips the package prefix and re-enters, see `eval_expr_ctx`), but a
    /// write under a task/function frame went through the dotted-name path
    /// as `pkg.x` and vanished. Returns the target with the prefix stripped.
    pub(in crate::compiler::simulator) fn strip_package_lvalue(
        &self,
        lhs: &Expression,
    ) -> Option<Expression> {
        let is_pkg = |name: &str| {
            self.module.packages.contains(name)
                && !self.signal_name_to_id.contains_key(name)
                && !self.signals.contains_key(name)
                && !self.module.classes.contains_key(name)
        };
        match &lhs.kind {
            ExprKind::MemberAccess { expr, member } => {
                if let ExprKind::Ident(h) = &expr.kind {
                    if h.path.len() == 1
                        && h.path[0].selects.is_empty()
                        && is_pkg(&h.path[0].name.name)
                    {
                        let mut ident = h.clone();
                        ident.path[0].name = member.clone();
                        // §26.3: a contested package variable has its own
                        // per-package storage under `<pkg>::<name>`.
                        if let Some(q) = self.pkg_var_storage(&h.path[0].name.name, &member.name) {
                            ident.path[0].name.name = q;
                        }
                        // The stripped name names a NEW signal and must resolve
                        // under its own column, not the field the package
                        // qualifier's node carried. `h` here is the
                        // `Ident(<pkg>)` reference, which — once the package
                        // name has itself been resolved/memoized (a
                        // `(pkg).static` element read hashes the package) —
                        // holds `cached_resolved_name == <pkg>`. Cloning it
                        // preserves that cache, so the stripped `<member>`
                        // wrongly resolves back to the PACKAGE name, and every
                        // element read returns all-x. Reset the cache so the
                        // new name resolves from scratch.
                        ident.cached_resolved_name = crate::ast::cache::NameCache::new();
                        return Some(Expression::new(ExprKind::Ident(ident), lhs.span));
                    }
                }
                None
            }
            ExprKind::Ident(h)
                if h.path.len() >= 2
                    && h.path[0].selects.is_empty()
                    && is_pkg(&h.path[0].name.name) =>
            {
                let mut ident = h.clone();
                ident.path.remove(0);
                // §26.3: a contested package variable's own storage.
                if let Some(q) = self.pkg_var_storage(&h.path[0].name.name, &h.path[1].name.name) {
                    ident.path[0].name.name = q;
                }
                // See the MemberAccess arm above: the cloned node inherits
                // `cached_resolved_name` from the `<pkg>::<member>` reference;
                // reset it so the bare `<member>` resolves fresh.
                ident.cached_resolved_name = crate::ast::cache::NameCache::new();
                Some(Expression::new(ExprKind::Ident(ident), lhs.span))
            }
            ExprKind::Index { expr, index } => {
                let base = self.strip_package_lvalue(expr)?;
                Some(Expression::new(
                    ExprKind::Index {
                        expr: Box::new(base),
                        index: index.clone(),
                    },
                    lhs.span,
                ))
            }
            ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } => {
                let base = self.strip_package_lvalue(expr)?;
                Some(Expression::new(
                    ExprKind::RangeSelect {
                        expr: Box::new(base),
                        kind: *kind,
                        left: left.clone(),
                        right: right.clone(),
                    },
                    lhs.span,
                ))
            }
            _ => None,
        }
    }

    /// §23.6: an absolute path written inside a subroutine body (`tb.u.sig`,
    /// `tb.u.sig[3:0]`, `tb.u.mem[i]`) arrives as a MemberAccess chain, and
    /// the lvalue arms looked it up in a signal table that keys the top
    /// module's contents without its name — the write was silently dropped.
    /// Rebuild it as the flat Ident elaboration gives module-scope code,
    /// keeping the select wrappers, so it resolves the same way.
    pub(in crate::compiler::simulator) fn top_rooted_member_lvalue(
        &self,
        e: &Expression,
    ) -> Option<Expression> {
        match &e.kind {
            ExprKind::Index { expr, index } => {
                let inner = self.top_rooted_member_lvalue(expr)?;
                Some(Expression::new(
                    ExprKind::Index {
                        expr: Box::new(inner),
                        index: index.clone(),
                    },
                    e.span,
                ))
            }
            ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } => {
                let inner = self.top_rooted_member_lvalue(expr)?;
                Some(Expression::new(
                    ExprKind::RangeSelect {
                        expr: Box::new(inner),
                        kind: kind.clone(),
                        left: left.clone(),
                        right: right.clone(),
                    },
                    e.span,
                ))
            }
            ExprKind::MemberAccess { .. } => {
                let mut root = e;
                while let ExprKind::MemberAccess { expr, .. } = &root.kind {
                    root = expr;
                }
                let ExprKind::Ident(h) = &root.kind else {
                    return None;
                };
                let top = self.module.name.as_str();
                if h.root.is_some()
                    || h.path.len() != 1
                    || !h.path[0].selects.is_empty()
                    || h.path[0].name.name != top
                    || self.local_stack.last().is_some_and(|l| l.contains_key(top))
                    || self.signal_name_to_id.contains_key(top)
                    || self.signals.contains_key(top)
                {
                    return None;
                }
                let path = Self::flatten_member_path(e)?
                    .into_iter()
                    .map(|name| crate::ast::expr::HierPathSegment {
                        name: crate::ast::Identifier { name, span: e.span },
                        selects: Vec::new(),
                    })
                    .collect();
                Some(Expression::new(
                    ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                        root: None,
                        path,
                        span: e.span,
                        cached_signal_id: crate::ast::cache::SignalCache::new(None),
                        cached_resolved_name: crate::ast::cache::NameCache::new(),
                        bound: crate::ast::binding::Bound::UNSET,
                    }),
                    e.span,
                ))
            }
            _ => None,
        }
    }
}
