//! Stores to unpacked elements, string characters, associative rows and ranges.

use super::*;

impl Simulator {
    /// Store `val` into select `index` of `elem`, one element (signal `id`)
    /// of the unpacked array `arr`: a bit, or a whole sub-vector when the
    /// element is a multi-dimensional PACKED array (a lane of `logic
    /// [1:0][63:0] q [0:2]` is 64 bits wide). `None` for a label this plain
    /// 0-based / ascending mapping does not cover (negative or otherwise
    /// exotic packed labels, `logic [-1:-5][31:0] d [-2:-4]`), which the
    /// normalizing paths further down resolve.
    pub(in crate::compiler::simulator) fn store_unpacked_elem_select(
        &mut self,
        arr: &str,
        elem: &str,
        id: usize,
        index: &Expression,
        val: &Value,
    ) -> Option<bool> {
        let w = self.signal_widths[id];
        let Some(raw) = self.eval_expr(index).to_index() else {
            return Some(false); // §11.5.1: x/z index writes nothing
        };
        let pos = match self.module.ascending_packed.get(arr).copied() {
            Some((lo_l, hi_l)) if raw >= lo_l && raw <= hi_l => Some((hi_l - raw) as u32),
            Some(_) => None,
            None if raw >= 0 => Some(raw as u32),
            None => None,
        };
        let ew = self
            .module
            .packed_signal_elem_widths
            .get(arr)
            .copied()
            .unwrap_or(1)
            .max(1);
        let lo = pos
            .map(|p| p.saturating_mul(ew))
            .filter(|&lo| lo.checked_add(ew).is_some_and(|hi| hi <= w))?;
        let two_state = self.signal_two_state.get(id).copied().unwrap_or(false);
        let coerced;
        let val = if two_state && val.has_xz() {
            coerced = val.to_two_state();
            &coerced
        } else {
            val
        };
        let mut cur = self.signal_table[id].clone();
        let prev = cur.clone();
        for b in 0..ew {
            cur.set_bit((lo + b) as usize, val.get_bit(b as usize));
        }
        let changed = cur != prev;
        if changed {
            if !self.dirty_signals[id] {
                self.dirty_signals[id] = true;
                self.dirty_list.push(id);
            }
            self.dirty_any = true;
            write_sig!(self, id, cur);
            self.table_modified = true;
            self.after_signal_write(id);
            self.mark_dirty(elem);
        }
        Some(changed)
    }

    /// §6.16: a character-indexed write `s[i] = byte` through a base the
    /// flat-name string path cannot see. The string-char block in
    /// `assign_value_index` keys on `string_signals` + a resolvable flat
    /// `name`, so it misses:
    ///   - a CLASS-member string (`this.s[i]`, `c.s[i]`, `C::s[i]`,
    ///     `c.b.s[i]`), a `MemberAccess` or dotted base with no flat signal
    ///     name, and
    ///   - a string-array / string-queue ELEMENT (`sa[k][i]`, `q[k][i]`,
    ///     `s2[a][b][i]`), which arrives here with `expr` = `sa[k]` (an
    ///     `Index`), not the collection name.
    /// Both would otherwise byte-select into the packed container (bit-bash)
    /// or vanish. Reads the current string, replaces byte `i`, and stores the
    /// whole string back via `assign_value` — exactly what the flat-name block
    /// does. Returns `None` unless `expr` is provably a SCALAR string (see
    /// `string_char_receiver_is_scalar`), so callers fall through to the
    /// existing arms unchanged.
    pub(in crate::compiler::simulator) fn try_string_index_char_write(
        &mut self,
        expr: &Expression,
        index: &Expression,
        val: &Value,
    ) -> Option<bool> {
        // Only the shapes the flat-name block can't reach: a member, an
        // element, or a dotted ident (`c.s` parses as a two-segment
        // HierarchicalIdentifier when the receiver is a module-scope handle,
        // vs. the MemberAccess shape `this.s` gets inside a method). A
        // single-segment bare Ident in `string_signals` already lands in the
        // flat-name block.
        let is_member = matches!(&expr.kind, ExprKind::MemberAccess { .. });
        let is_elem = matches!(&expr.kind, ExprKind::Index { .. });
        let is_dotted_ident = matches!(&expr.kind, ExprKind::Ident(h) if h.path.len() >= 2);
        if !is_member && !is_elem && !is_dotted_ident {
            return None;
        }
        // §7.4.4 / §7.8 / §7.10: a select on a string COLLECTION, or on a
        // partial element of a multi-dimensional one, picks a whole element
        // (`c.b.q[1] = "x"`, `s2[1][2] = "x"`), not a character.
        if !self.string_char_receiver_is_scalar(expr) {
            return None;
        }
        // §7.8: a row of an associative array of collections
        // (`string m[int][string]`, `m[3]["x"] = "hi"`) is itself a
        // collection; `[k]` selects its element. Taken for a character
        // index, the write was dropped.
        if let ExprKind::Index { expr: arr, .. } = &expr.kind
            && let ExprKind::Ident(h) = &arr.kind
            && h.path.len() == 1
            && h.path[0].selects.is_empty()
        {
            let name = self.resolve_hier_name(h).into_owned();
            let name = self.instance_assoc_member(&name).unwrap_or(name);
            if self.is_associative_array(&name) && self.assoc_has_coll_elems(&name) {
                return None;
            }
        }
        let mut b = self.eval_expr(expr).sv_string_bytes();
        let i = self.eval_expr(index).to_u64().unwrap_or(0) as usize;
        let cbyte = (val.resize_for_assign(8).to_u64().unwrap_or(0) & 0xFF) as u8;
        if i < b.len() && cbyte != 0 {
            b[i] = cbyte;
            // A `Value` may carry string metadata; store the freshly rebuilt
            // string so the element keeps its string-ness.
            return Some(self.assign_value(expr, &Self::sv_string_from_bytes(&b)));
        }
        Some(false)
    }

    /// Whether the associative array stored at `name` has collections for
    /// elements (§7.4.5): its declared shape has more dimensions below the
    /// associative one, at any scope (`<h>#m` too).
    pub(super) fn assoc_has_coll_elems(&self, name: &str) -> bool {
        self.coll_shape_at(name).is_some_and(|(shape, depth)| {
            depth == 0 && shape.dims.len() >= 2 && matches!(shape.dims[0], Dim::Assoc(_))
        })
    }

    /// §7.8/§7.10: a write into a row of an associative array of
    /// collections (`aq[6][0] = 9` on `int aq[int][$]`) whose row does not
    /// exist yet creates the row as the collection its declaration names,
    /// as `push_back` does, so the element is stored in that row (the write
    /// left a bare `aq[6][0]` cell no queue read, `%p` or `foreach` saw).
    /// Every level of the key chain is created in turn: `a["p"][7]["q"] = 9`
    /// on `byte a[string][int][string]` makes the rows `a["p"]` and
    /// `a["p"][7]`, each keyed by its own declared key type.
    /// Returns the innermost row when it is a queue or dynamic array, for
    /// the append and bounds rules; `Err` when the write must be ignored
    /// (§7.4.6, §7.5.1, §7.10.1).
    pub(super) fn ensure_assoc_row(
        &mut self,
        row_expr: &Expression,
    ) -> Result<Option<(String, Dim)>, ()> {
        // A row is a select (`m[k]`); a plain `a[i] = v` has none to create.
        if !matches!(row_expr.kind, ExprKind::Index { .. }) {
            return Ok(None);
        }
        // An associative root names its keys by level; any other root (a
        // queue of associative arrays, a local's `@l#n`) by its flat name.
        let Some(row) = self
            .nested_index_name(row_expr)
            .or_else(|| self.flat_member_name(row_expr))
        else {
            return Ok(None);
        };
        // One shape lookup serves the whole write (every element write to
        // `int d[][]` passes here): the row's own kind is its declared
        // dimension once `ensure_row` has its path in range.
        let Some((shape, depth)) = self.coll_shape_at(&row) else {
            return Ok(None);
        };
        let own = match shape.dims.get(depth) {
            Some(Dim::Fixed(_)) | None => return Ok(None),
            _ if depth == 0 => return Ok(None),
            Some(dim) => dim.clone(),
        };
        match self.row_write_ready(shape, depth, &row) {
            Some(true) => {}
            Some(false) => return Err(()),
            None if self.ensure_row(&row) => {}
            None => return Err(()),
        }
        Ok(matches!(own, Dim::Queue | Dim::Dyn).then_some((row, own)))
    }

    /// §7.8.6/§11.5.1: a bit write into an associative element whose
    /// element type is integral (`m[k][3] = 1`, `sb["k"][2] = 1`), absent or
    /// not: read it (an absent one as the element type's default), set the
    /// bit and write it back. An absent element was created with the bit
    /// dropped, and a present one's bit went under a bogus `m[k][3]` key.
    /// None when `expr` is not such an element.
    pub(super) fn assoc_elem_bit_write(
        &mut self,
        expr: &Expression,
        index: &Expression,
        val: &Value,
    ) -> Option<bool> {
        let ExprKind::Index {
            expr: inner_expr,
            index: inner_idx,
        } = &expr.kind
        else {
            return None;
        };
        let ExprKind::Ident(hier) = &inner_expr.kind else {
            return None;
        };
        let name = self.resolve_hier_name(hier).into_owned();
        let name = self.instance_assoc_member(&name).unwrap_or(name);
        if !self.is_associative_array(&name) || self.assoc_has_coll_elems(&name) {
            return None;
        }
        let default = self.assoc_missing_elem_value(&name)?;
        let kv = self.eval_expr(inner_idx);
        if kv.has_xz() && !kv.is_real && !self.is_string_keyed_array(&name) {
            self.warn_invalid_assoc_index(&name);
            return Some(false);
        }
        let bv = self.eval_expr(index);
        let Some(b) = bv.to_i64().filter(|_| !bv.has_xz()) else {
            return Some(false);
        };
        let key = self.assoc_key_str(&name, &kv);
        let elem = format!("{}[{}]", name, key);
        // A row that is itself a collection (`aq[k]` of `int aq[int][$]`):
        // `[j]` selects its element, which the collection paths handle.
        if self.is_associative_array(&elem)
            || self.module.dynamic_arrays.contains(&elem)
            || self
                .signals
                .contains_key(&Self::name_with_suffix(&elem, ".size"))
        {
            return None;
        }
        let fresh = !self.signals.contains_key(&elem);
        let mut cur = self.get_signal_value_by_name(&elem).unwrap_or(default);
        let phys = match self.select_base_elem_dim(expr) {
            Some((dl, dr)) if dl < dr => dr - b,
            Some((dl, dr)) => b - dl.min(dr),
            None => b,
        };
        // §7.4.1: with a multi-dimensional packed element (`logic [3:0][7:0]
        // p [int]`), `p[k][j]` selects a whole slice of the leading packed
        // dimension, so write `stride` bits at slot `phys`.
        let stride = self
            .module
            .packed_full_dims
            .get(&name)
            .filter(|d| d.len() > 1)
            .map_or(1, |d| {
                d[1..]
                    .iter()
                    .map(|&(l, r)| (l - r).unsigned_abs() as usize + 1)
                    .product()
            });
        let Some(lsb) = usize::try_from(phys)
            .ok()
            .and_then(|p| p.checked_mul(stride))
            .filter(|l| {
                l.checked_add(stride)
                    .is_some_and(|e| e <= cur.width as usize)
            })
        else {
            return Some(false);
        };
        let mut changed = false;
        for i in 0..stride {
            let nb = val.get_bit(i);
            changed |= cur.get_bit(lsb + i) != nb;
            cur.set_bit(lsb + i, nb);
        }
        if changed || fresh {
            self.set_signal_value_by_name(&elem, cur);
            self.mark_dirty(&elem);
            self.touch_queue(&name);
        }
        Some(changed)
    }

    /// Whether storing `val`'s bits (bit 0 up) over `cur[lsb..=msb]`, clipped
    /// to `cur`'s width, changes a bit.
    pub(super) fn range_store_differs(cur: &Value, lsb: usize, msb: usize, val: &Value) -> bool {
        let top = msb.min((cur.width as usize).saturating_sub(1));
        (lsb..=top).any(|i| cur.get_bit(i) != val.get_bit(i - lsb))
    }

    /// Store `val`'s bits (bit 0 up) over `cur[lsb..=msb]`, clipped to
    /// `cur`'s width, in the stored value itself. Returns whether a bit
    /// changed.
    pub(super) fn range_store(cur: &mut Value, lsb: usize, msb: usize, val: &Value) -> bool {
        let top = msb.min((cur.width as usize).saturating_sub(1));
        let mut changed = false;
        for i in lsb..=top {
            let nb = val.get_bit(i - lsb);
            if cur.get_bit(i) != nb {
                cur.set_bit(i, nb);
                changed = true;
            }
        }
        changed
    }
}
