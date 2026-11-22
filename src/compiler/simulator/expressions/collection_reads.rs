//! Reads of module memories, formal collections and collection elements.

use super::*;

impl Simulator {
    /// §23.6/§23.9, §7.4.6: a declaration-backed module memory does not
    /// require evaluating its instance prefixes as class or struct values.
    /// Task bodies keep dotted references as MemberAccess chains; probing
    /// every prefix that way before recognizing the array repeats that work
    /// recursively at each hierarchy level.
    #[inline(never)]
    pub(super) fn declared_module_memory_read(
        &mut self,
        base: &Expression,
        index: &Expression,
    ) -> Option<Value> {
        let root = Self::member_chain_root(base)?;
        // A nearer value or interface binding retains ordinary member
        // dispatch. In particular, an identically named module memory must
        // not capture a local object's or packed struct's member.
        if self.frame_scalar_shadows(root)
            || self.dyn_name_lookup(root).is_some()
            || self
                .formal_dyn
                .last()
                .is_some_and(|f| f.iter().any(|(name, _)| name == root))
            || self.class_context_stack.last().is_some_and(Option::is_some)
            || self.this_prop_hides(root)
            || self.module.classes.contains_key(root)
            || self.module.packages.contains(root)
            || self.lookup_typedef_target(root).is_some()
            || self
                .local_iface_aliases
                .last()
                .is_some_and(|f| f.contains_key(root))
            || self.iface_alias_for(root).is_some()
        {
            return None;
        }
        // Instance prefixes can also have implicit placeholder signal IDs.
        // Only a declared value shadows the instance, not that lookup entry.
        let is_value = |name: &str| {
            self.module.var_decl_types.contains_key(name)
                || self.module.parameters.contains_key(name)
        };
        if is_value(root) || self.hint_scoped(root).is_some_and(|n| is_value(&n)) {
            return None;
        }

        // Only pure, unselected names qualify. Calls, indexed receivers,
        // this/super and other runtime receivers never run during this probe.
        let mut parts = Vec::new();
        let mut current = base;
        loop {
            match &current.kind {
                ExprKind::MemberAccess { expr, member } => {
                    parts.push(member.name.as_str());
                    current = expr;
                }
                ExprKind::Ident(h)
                    if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty() =>
                {
                    parts.push(h.path[0].name.name.as_str());
                    break;
                }
                _ => return None,
            }
        }
        parts.reverse();
        let mut name = parts.join(".");
        // Match the ordinary array reader's absolute-then-instance lookup.
        if !self.module.arrays.contains_key(&name) {
            name = self.hint_scoped(&name)?;
        }
        let parent = name.rsplit_once('.')?.0;
        if self.instance_by_path(parent).is_none()
            || !self.module.var_decl_types.contains_key(&name)
            || self.module.dynamic_arrays.contains(&name)
            || self.module.queue_vars.contains(&name)
            || self.module.associative_arrays.contains_key(&name)
            || self.module.arrays_2d.contains_key(&name)
            || self.module.arrays_nd.contains_key(&name)
        {
            return None;
        }
        let width = self.module.arrays.get(&name)?.2.max(1);
        let &(first, lo, hi) = self.array_first_id.get(name.as_str())?;
        // Evaluate the selector only after binding succeeds, exactly once.
        // Use the same invalid-index and packed-arena handling as the
        // general fixed-array read below.
        let value = self.eval_expr(index);
        let Some(i) = value.to_index().filter(|_| !value.has_xz()) else {
            return Some(Value::new(width));
        };
        if !(lo..=hi).contains(&i) {
            return Some(
                self.assoc_missing_elem_value(&name)
                    .unwrap_or_else(|| Value::new(width)),
            );
        }
        let id = first + (i - lo) as usize;
        if is_packed_id(id) {
            return Some(self.packed.read(id));
        }
        let mut value = self.signal_table[id].clone();
        if self.signal_signed[id] {
            value.is_signed = true;
        }
        Some(value)
    }

    /// The element of `name` (store `store_name`) at the evaluated index
    /// `idx_val`: the interpreter's element read of a fixed, dynamic or
    /// associative array once the operand is resolved. Also the slow path of
    /// a compiled `LoadDynElem`, which must read exactly what this reads.
    /// A collection formal stores its unpacked elements under the activation's
    /// key. Read the element there before applying its declared packed bounds.
    pub(super) fn formal_collection_packed_read(&mut self, expr: &Expression) -> Option<Value> {
        let (h, selectors) = Self::index_chain(expr)?;
        let [seg] = h.path.as_slice() else { return None };
        let store = self.dyn_name_lookup(&seg.name.name)?;
        if !(self.module.queue_vars.contains(store) || self.module.dynamic_arrays.contains(store)
            || self.is_associative_array(store))
            || self.coll_shape_at(store).is_some_and(|(shape, _)| shape.dims.len() != 1)
        {
            return None;
        }
        let dims = self.module.packed_full_dims.get(store)?;
        let (row, packed) = selectors.split_first()?;
        if packed.is_empty() || packed.len() > dims.len() { return None; }
        let width: u32 = dims.iter().map(|(a,b)| a.abs_diff(*b) as u32 + 1).product();
        let remaining: u32 = dims[packed.len()..].iter().map(|(a,b)| a.abs_diff(*b) as u32 + 1).product();
        let dims = dims.clone();
        let store = store.to_owned();
        let has_bound = self.module.dynamic_arrays.contains(&store)
            && Self::expr_contains_dollar(row);
        if has_bound {
            self.dollar_bound.push(self.get_queue_size(&store) as i64 - 1);
        }
        let key = self.eval_expr(row);
        if has_bound {
            self.dollar_bound.pop();
        }
        let elem = self.coll_elem_read(&store, &store, key, width);
        let indices: Vec<_> = packed.iter().map(|sel| self.eval_expr(sel)).collect();
        Some(match Self::packed_chain_slice(u64::from(width), &dims, &indices) {
            Some((lo, w)) => elem.range_select((lo + w - 1) as usize, lo as usize),
            None => Value::new(remaining),
        })
    }

    pub(in crate::compiler::simulator) fn coll_elem_read(
        &mut self,
        name: &str,
        store_name: &str,
        idx_val: Value,
        ctx_width: u32,
    ) -> Value {
        // A side-effecting selector bypasses speculative
        // probes; validate its single evaluated result here
        // before either fixed-array storage lookup.
        if !self.is_associative_array(&name) && idx_val.has_xz() {
            if let Some(ew) = self.module.arrays.get(&*name).map(|a| a.2) {
                return Value::new(ew.max(1));
            }
        }
        // §7.9 / §7.10.4: an element of a queue or dynamic
        // array at or past its current SIZE does not exist, so
        // reading it yields the element type's default. Every
        // storage lookup below addresses cells that a shrink
        // left behind — `delete()`, `delete(i)` and
        // `pop_front`/`pop_back` only move `.size` — so a
        // size-4 queue that had held 8 returned its old byte
        // at `q[5]` instead of x. A queue is registered in
        // `array_first_id` over a placeholder range (0..63),
        // NOT its live length, so the bound comes from
        // `.size`. Only the cell lookups are skipped; the
        // element-default logic is the one a miss already
        // takes, so the element type still decides the default
        // (x / 0 / null / "").
        let out_of_range = !self.is_associative_array(&name)
            && self.module.dynamic_arrays.contains(&*name)
            && idx_val
                .to_i64()
                .is_none_or(|i| i < 0 || i as u64 >= self.get_queue_size(&store_name));
        // Fast path for 1-D arrays: compute signal_id directly
        // via `array_first_id`. Critical for LARGE arrays
        // (e.g. xezim_mem[0:4M]) where per-element names are
        // NOT registered in `signal_name_to_id` — the
        // string-keyed fallback below would return X for
        // every element, silently corrupting reads.
        if !self.is_associative_array(&name) {
            if let Some(&(first_id, lo, hi)) = self.array_first_id.get(name) {
                // §7.4.6: SIGNED index — negative-lo arrays.
                // §11.5.1: an x/z index reads an all-x element.
                let Some(idx) = idx_val.to_index() else {
                    let ew = self
                        .module
                        .arrays
                        .get(&*name)
                        .map(|a| a.2)
                        .unwrap_or(ctx_width.max(1));
                    return Value::new(ew.max(1));
                };
                if !out_of_range && idx >= lo && idx <= hi {
                    let eid = first_id + (idx - lo) as usize;
                    // Packed-arena cell: no signal_table slot
                    // (found by the #147 race test's AST-path
                    // `$display("%h", mem[5])` — the clone
                    // below panicked on the arena id).
                    if is_packed_id(eid) {
                        return self.packed.read(eid);
                    }
                    let mut v = self.signal_table[eid].clone();
                    if self.signal_signed[eid] {
                        v.is_signed = true;
                    }
                    return v;
                }
                // §7.4.6 / Table 7-1: an invalid fixed-array
                // index yields the element type's default,
                // fitted to its width and signedness.
                if !self.module.dynamic_arrays.contains(&*name) {
                    if let Some(ew) = self.module.arrays.get(&*name).map(|a| a.2) {
                        return self
                            .assoc_missing_elem_value(&name)
                            .unwrap_or_else(|| Value::new(ew.max(1)));
                    }
                }
            }
        }
        // §7.8.2: reading with an x/z index yields the element
        // type's default rather than folding to another key.
        if self.is_associative_array(&name) && idx_val.has_xz() {
            self.warn_invalid_assoc_index(&name);
            return Value::zero(ctx_width.max(1));
        }
        let idx_str = if self.is_associative_array(&name) {
            self.assoc_key_str(&name, &idx_val)
        } else {
            // §7.4.6: match the declared (signed) element
            // spelling — `nmem[-2]`, not `nmem[4294967294]`.
            idx_val.to_i64().unwrap_or(0).to_string()
        };
        let elem_name = Self::name_with_key(&store_name, &idx_str);
        if !out_of_range && let Some(&eid) = self.signal_name_to_id.get(elem_name.as_str()) {
            let mut v = self.signal_table[eid].clone();
            if self.signal_signed[eid] {
                v.is_signed = true;
            }
            return v;
        }
        let cell = if out_of_range {
            None
        } else {
            self.signals.get(&elem_name).cloned()
        };
        let mut v = if let Some(sv) = cell {
            sv
        } else if let Some(def_expr) = self.module.assoc_defaults.get(&*name).cloned() {
            self.eval_expr(&def_expr)
        } else {
            // §7.8.6: reading a NONEXISTENT element yields the
            // element type's default initial value — 0 for a
            // 2-state type, x for a 4-state one — at the
            // ELEMENT's width. A flat 1-bit x was wrong twice
            // over: `int aa[int]; aa[7]` read x instead of 0,
            // and even where x was right the 1-bit width made
            // `%h` print a single `x` for what should be `xx`.
            // (The read still must not CREATE the element —
            // `num()` is unchanged, which is why this is a
            // value-only fallback.) The declared type also
            // gives a subroutine-local array its default (it
            // read a 1-bit x) and the element's signedness,
            // so `aa[k] -= 5` on a new `int` key is -5.
            if let Some(v) = self.coll_missing_elem_value(&name) {
                v
            } else {
                let ew = self.assoc_elem_width(&name).unwrap_or(1).max(1);
                if self.module.two_state_signals.contains(&*name)
                    || name
                        .rsplit('.')
                        .next()
                        .is_some_and(|l| self.module.two_state_signals.contains(l))
                {
                    Value::zero(ew)
                } else {
                    Value::new(ew)
                }
            }
        };
        if self.signed_signals.contains(&elem_name) {
            v.is_signed = true;
        }
        return v;
    }
}
