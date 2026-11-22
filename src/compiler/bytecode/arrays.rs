//! Local, two-dimensional and multi-dimensional unpacked arrays.

use super::*;

impl<'a> BytecodeCompiler<'a> {
    /// The operand addressing array `name`, or None (a bail) when it has no
    /// cells to address: neither a dense id range nor element signals by
    /// name. A block-local array of a process is declared at run time into
    /// by-name storage that has neither, and a compiled store into it was
    /// dropped: `for (int i ...) arr[i] = v;` left `arr` 0 (§6.21).
    ///
    /// A dynamic array or queue has no operand either (§7.5, §7.10): its
    /// `0..63` element cells are only a placeholder backing. Elements past
    /// them live in by-name storage, and a cell outside the live size keeps
    /// a stale value, so a compiled read of `q[64]` came back x (a 2-state
    /// `t += q[i]` then summed to 0) and `q[i]` past `q.size()` read an old
    /// element instead of the default (§7.4.6). A plain element read compiles
    /// to `Insn::LoadDynElem` instead; every other shape bails to the
    /// interpreter, as stores already do (`collection_store_denied`).
    fn array_operand(&mut self, name: String) -> Option<Box<ArrayOperand>> {
        if self.is_dynamic_collection(&name) {
            self.bail("dynamic_collection_elem");
            return None;
        }
        if let Some(&(first_id, lo, hi)) = self
            .array_first_id
            .and_then(|arrays| arrays.get(name.as_str()))
        {
            return Some(Box::new(ArrayOperand::Dense {
                name,
                first_id,
                lo,
                hi,
            }));
        }
        let lo = self.arrays.get(&name).map_or(0, |&(lo, _, _)| lo);
        if self
            .signal_name_to_id
            .get(format!("{name}[{lo}]").as_str())
            .is_none()
        {
            self.bail("array_without_cells");
            return None;
        }
        Some(Box::new(ArrayOperand::Named(name)))
    }

    /// `array_operand` with the index register the element instruction
    /// reads. §7.4.2: an index into an array whose range has a negative
    /// bound addresses elements by its signed value, but the element
    /// instructions read the register's bits as an unsigned number, so
    /// `arr[-1]` of `int arr[-1:1]`, or `arr[k]` with `int k = -1`, missed its
    /// element in every compiled block (a read gave x, a write was lost).
    /// Widen the index to 64 bits first: `MoveResize` sign-extends a signed
    /// index and zero-extends an unsigned one, which stays out of range.
    pub(super) fn array_operand_index(
        &mut self,
        name: String,
        idx_reg: RegId,
    ) -> Option<(Box<ArrayOperand>, RegId)> {
        let array = self.array_operand(name)?;
        let lowest = match array.as_ref() {
            ArrayOperand::Dense { lo, hi, .. } => (*lo).min(*hi),
            ArrayOperand::Named(name) => self.arrays.get(name).map_or(0, |&(lo, hi, _)| lo.min(hi)),
        };
        if lowest >= 0 {
            return Some((array, idx_reg));
        }
        let wide = self.alloc_reg();
        self.emit(Insn::MoveResize(wide, idx_reg, 64));
        Some((array, wide))
    }

    /// Bind a small fixed-shape local array to per-element registers. One
    /// dimension, constant bounds, at most 32 elements; every element starts
    /// at the element type's default. Returns false when the shape does not
    /// qualify (the caller bails to the AST path).
    pub(super) fn bind_local_array(
        &mut self,
        name: &str,
        dims: &[crate::ast::types::UnpackedDimension],
        data_type: &crate::ast::types::DataType,
        is_real: bool,
    ) -> bool {
        use crate::ast::types::UnpackedDimension as UD;
        if dims.len() != 1 {
            return false;
        }
        let (lo, hi): (i64, i64) = match &dims[0] {
            UD::Range { left, right, .. } => {
                let (Some(l), Some(r)) = (
                    self.eval_const_expr(left).map(|v| v as i64),
                    self.eval_const_expr(right).map(|v| v as i64),
                ) else {
                    return false;
                };
                (l.min(r), l.max(r))
            }
            UD::Expression { expr: e, .. } => {
                let Some(n) = self.eval_const_expr(e).map(|v| v as i64) else {
                    return false;
                };
                if n <= 0 {
                    return false;
                }
                (0, n - 1)
            }
            _ => return false,
        };
        let count = hi - lo + 1;
        if !(1..=32).contains(&count) {
            return false;
        }
        let elem_w = self.decl_width(data_type);
        if elem_w == 0 || (elem_w > 64 && !is_real) {
            return false;
        }
        let default = self.type_default_value(data_type, elem_w);
        let mut regs = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let r = self.alloc_reg();
            self.emit(Insn::LoadConst(r, Box::new(default.clone())));
            regs.push(r);
        }
        self.local_var_array.insert(
            name.to_string(),
            LocalArrayBind {
                regs,
                lo,
                elem_w,
                is_real,
                elem_signed: None,
            },
        );
        true
    }

    /// Element READ of a register-bound local array. A constant index picks
    /// the register directly; a dynamic one selects by compare/branch chain
    /// (out-of-range reads keep the element default — the type's x/0 — which
    /// matches §7.4.6 for the x case and is unreachable for well-formed
    /// loops).
    pub(super) fn compile_local_array_read(
        &mut self,
        name: &str,
        idx: &Expression,
    ) -> Option<RegId> {
        let ab = self.local_var_array.get(name)?.clone();
        if let Some(k) = self.eval_const_expr(idx) {
            let i = (k as i64).checked_sub(ab.lo)?;
            let src = *ab.regs.get(usize::try_from(i).ok()?)?;
            let r = self.alloc_reg();
            self.emit(Insn::Move(r, src));
            return Some(r);
        }
        let idx_r = self.compile_expr(idx, 0)?;
        let out = self.alloc_reg();
        let default = if ab.is_real {
            Value::from_f64(0.0)
        } else {
            Value::new(ab.elem_w)
        };
        self.emit(Insn::LoadConst(out, Box::new(default)));
        let kreg = self.alloc_reg();
        let creg = self.alloc_reg();
        for (i, &er) in ab.regs.iter().enumerate() {
            self.emit(Insn::LoadConst(
                kreg,
                Box::new(Value::from_u64((ab.lo + i as i64) as u64, 32)),
            ));
            self.emit(Insn::Eq(creg, idx_r, kreg));
            let br = self.insns.len();
            self.emit(Insn::BranchIfFalse(creg, 0));
            self.emit(Insn::Move(out, er));
            let after = self.insns.len() as u32;
            self.insns[br] = Insn::BranchIfFalse(creg, after);
        }
        Some(out)
    }

    /// Element WRITE of a register-bound local array (same chain shape as the
    /// read). The stored value is coerced per the element type: real elements
    /// convert numerically, integral ones resize.
    pub(super) fn compile_local_array_write(
        &mut self,
        name: &str,
        idx: &Expression,
        val_reg: RegId,
    ) -> Option<bool> {
        let ab = self.local_var_array.get(name)?.clone();
        // Coerce ONCE into a scratch, then move into the selected element.
        let v = self.alloc_reg();
        self.emit(Insn::Move(v, val_reg));
        if ab.is_real {
            self.emit_to_real(v);
        } else {
            self.emit(Insn::Resize(v, ab.elem_w));
            if let Some(kind) = ab
                .regs
                .first()
                .and_then(|r| self.local_kinds.get(r))
                .copied()
            {
                self.local_kinds.insert(v, kind);
                self.emit_local_fit(v);
            }
        }
        if let Some(k) = self.eval_const_expr(idx) {
            let i = (k as i64).checked_sub(ab.lo)?;
            let dst = *ab.regs.get(usize::try_from(i).ok()?)?;
            self.emit(Insn::Move(dst, v));
            return Some(true);
        }
        let idx_r = self.compile_expr(idx, 0)?;
        let kreg = self.alloc_reg();
        let creg = self.alloc_reg();
        for (i, &er) in ab.regs.iter().enumerate() {
            self.emit(Insn::LoadConst(
                kreg,
                Box::new(Value::from_u64((ab.lo + i as i64) as u64, 32)),
            ));
            self.emit(Insn::Eq(creg, idx_r, kreg));
            let br = self.insns.len();
            self.emit(Insn::BranchIfFalse(creg, 0));
            self.emit(Insn::Move(er, v));
            let after = self.insns.len() as u32;
            self.insns[br] = Insn::BranchIfFalse(creg, after);
        }
        Some(true)
    }

    /// DYNAMIC element read of a 2-D unpacked array (`a[i][j]` with
    /// non-constant indices). Elements are materialized contiguously
    /// row-major (see the simulator's arrays_2d build loop), so the read is a
    /// bounds-checked flat index over one Dense operand:
    /// `flat = (i-lo1)*ncols + (j-lo2)`; an out-of-range index in EITHER
    /// dimension yields x (§7.4.6), which the per-dim guard folds into the
    /// flat index by driving it out of the Dense operand's own range.
    pub(super) fn compile_2d_array_read(
        &mut self,
        hier: &crate::ast::expr::HierarchicalIdentifier,
        i_expr: &Expression,
        j_expr: &Expression,
    ) -> Option<RegId> {
        let (array, flat) = self.compile_2d_flat_index(hier, i_expr, j_expr)?;
        let dest = self.alloc_reg();
        self.emit(Insn::LoadArrayElem(dest, array, flat));
        Some(dest)
    }

    /// The `module.arrays_2d` entry of `hier`, as named or in the current
    /// scope, with the key it was found under.
    pub(super) fn arrays_2d_entry(
        &self,
        hier: &crate::ast::expr::HierarchicalIdentifier,
    ) -> Option<(String, Array2dShape)> {
        let raw = Self::hier_raw_name(hier);
        let arrays_2d = self.arrays_2d?;
        arrays_2d
            .get(raw.as_str())
            .map(|s| (raw.clone(), *s))
            .or_else(|| {
                self.scope_hint.as_ref().and_then(|sc| {
                    let q = cat_dot(sc, &raw);
                    arrays_2d.get(q.as_str()).map(|s| (q, *s))
                })
            })
    }

    /// Declared width of a fully indexed element of a multidimensional
    /// unpacked array (§7.4.5): the materialized cell's width when the indices
    /// are constant, the `arrays_2d` element width when they are not. Anything
    /// else (a row, a select inside an element) answers None. A string
    /// element answers 0: its value is self-determined, like a string signal.
    pub(super) fn multi_dim_unpacked_elem_width(&self, lhs: &Expression) -> Option<u32> {
        if let Some(id) = self.const_multi_dim_array_elem_signal_id(lhs) {
            return Some(self.signal_widths[id]);
        }
        let ExprKind::Index { expr, index } = &lhs.kind else {
            return None;
        };
        let (hier, idxs) = self.flatten_index_chain_exprs(expr, index)?;
        if idxs.len() != 2 {
            return None;
        }
        self.arrays_2d_entry(hier).map(|(_, (_, _, w))| w)
    }

    /// Shared row-major addressing for a DYNAMIC 2-D unpacked element
    /// (`a[i][j]`): returns the Dense operand and the register holding
    /// `flat = (i-lo1)*ncols + (j-lo2)`, with an out-of-range index in EITHER
    /// dimension forced one past the operand's range so the element access
    /// itself reports it (§7.4.6: read yields x, write is discarded).
    pub(super) fn compile_2d_flat_index(
        &mut self,
        hier: &crate::ast::expr::HierarchicalIdentifier,
        i_expr: &Expression,
        j_expr: &Expression,
    ) -> Option<(Box<ArrayOperand>, RegId)> {
        let (key, ((lo1, hi1), (lo2, hi2), _w)) = self.arrays_2d_entry(hier)?;
        if lo1 < 0 || lo2 < 0 {
            return None;
        }
        // A queue or dynamic array of rows: see `array_operand`.
        if self.is_dynamic_collection(&key) {
            self.bail("dynamic_collection_elem");
            return None;
        }
        let first_id = *self
            .signal_name_to_id
            .get(format!("{}[{}][{}]", key, lo1, lo2).as_str())?;
        let ncols = hi2 - lo2 + 1;
        let count = (hi1 - lo1 + 1) * ncols;
        let iv = self.compile_expr(i_expr, 0)?;
        let jv = self.compile_expr(j_expr, 0)?;
        // In-range test per §7.4.6. The flat index is forced to `count`
        // (one past the Dense range) when either dimension is out of
        // range, and LoadArrayElem's own bounds check turns that into x.
        let lo1_r = self.alloc_reg();
        self.emit(Insn::LoadConst(
            lo1_r,
            Box::new(Value::from_u64(lo1 as u64, 32)),
        ));
        let hi1_r = self.alloc_reg();
        self.emit(Insn::LoadConst(
            hi1_r,
            Box::new(Value::from_u64(hi1 as u64, 32)),
        ));
        let lo2_r = self.alloc_reg();
        self.emit(Insn::LoadConst(
            lo2_r,
            Box::new(Value::from_u64(lo2 as u64, 32)),
        ));
        let hi2_r = self.alloc_reg();
        self.emit(Insn::LoadConst(
            hi2_r,
            Box::new(Value::from_u64(hi2 as u64, 32)),
        ));
        let ok = self.alloc_reg();
        let t = self.alloc_reg();
        self.emit(Insn::Geq(ok, iv, lo1_r));
        self.emit(Insn::Leq(t, iv, hi1_r));
        self.emit(Insn::BitAnd(ok, ok, t));
        self.emit(Insn::Geq(t, jv, lo2_r));
        self.emit(Insn::BitAnd(ok, ok, t));
        self.emit(Insn::Leq(t, jv, hi2_r));
        self.emit(Insn::BitAnd(ok, ok, t));
        // flat = (i-lo1)*ncols + (j-lo2)
        let flat = self.alloc_reg();
        self.emit(Insn::Sub(flat, iv, lo1_r));
        let nc = self.alloc_reg();
        self.emit(Insn::LoadConst(
            nc,
            Box::new(Value::from_u64(ncols as u64, 32)),
        ));
        self.emit(Insn::Mul(flat, flat, nc));
        let jrel = self.alloc_reg();
        self.emit(Insn::Sub(jrel, jv, lo2_r));
        self.emit(Insn::Add(flat, flat, jrel));
        // guard: !ok -> flat = count (out of the Dense range -> x)
        let br = self.insns.len();
        self.emit(Insn::BranchIfFalse(ok, 0));
        let after_ok = self.insns.len() as u32 + 2;
        self.emit(Insn::Jump(after_ok));
        let oob = self.insns.len() as u32;
        self.emit(Insn::LoadConst(
            flat,
            Box::new(Value::from_u64(count as u64, 32)),
        ));
        self.insns[br] = Insn::BranchIfFalse(ok, oob);
        Some((
            Box::new(ArrayOperand::Dense {
                name: key,
                first_id,
                lo: 0,
                hi: count - 1,
            }),
            flat,
        ))
    }

    /// Resolve a fully-indexed 2D/ND unpacked-array element whose indices are
    /// compile-time constants. Elaboration materializes these cells as scalar
    /// signals named `base[i][j]...`, so generated flop arrays can use the
    /// ordinary scalar bytecode paths instead of falling back to the AST.
    pub(super) fn const_multi_dim_array_elem_signal_id(&self, expr: &Expression) -> Option<usize> {
        if !matches!(expr.kind, ExprKind::Index { .. }) {
            return None;
        }

        fn collect<'e>(
            compiler: &BytecodeCompiler<'_>,
            expr: &'e Expression,
            indices: &mut Vec<u32>,
        ) -> Option<&'e HierarchicalIdentifier> {
            match &expr.kind {
                ExprKind::Index { expr, index } => {
                    let hier = collect(compiler, expr, indices)?;
                    indices.push(compiler.eval_const_expr(index)?);
                    Some(hier)
                }
                ExprKind::Paren(inner) => collect(compiler, inner, indices),
                ExprKind::Ident(hier) => Some(hier),
                _ => None,
            }
        }

        let mut indices = Vec::new();
        let hier = collect(self, expr, &mut indices)?;
        if indices.len() < 2 || !self.is_multi_dim_array(hier) {
            return None;
        }

        let raw = Self::hier_raw_name(hier);
        let mut indexed = raw.clone();
        for index in indices {
            indexed.push('[');
            indexed.push_str(&index.to_string());
            indexed.push(']');
        }

        if !raw.contains('.') {
            if let Some(scope) = &self.scope_hint {
                if let Some(&id) = self
                    .signal_name_to_id
                    .get(format!("{}.{}", scope, indexed).as_str())
                {
                    return Some(id);
                }
            }
        }
        if let Some(&id) = self.signal_name_to_id.get(indexed.as_str()) {
            return Some(id);
        }
        if let Some(scope) = &self.scope_hint {
            if let Some(&id) = self
                .signal_name_to_id
                .get(format!("{}.{}", scope, indexed).as_str())
            {
                return Some(id);
            }
        }
        None
    }

    pub(super) fn flattened_outer_const_signal_id(&self, expr: &Expression) -> Option<usize> {
        let ExprKind::Index { expr: base, index } = &expr.kind else {
            return None;
        };
        // Generated-loop expansion has already selected the unpacked element
        // and baked its index into the hierarchical instance path. The AST
        // retains the now-redundant constant select (`flat[i][bit]`), while
        // the signal table contains `flat` as the selected packed element.
        // Accept every constant here, not only zero; the shape guards below
        // keep real unpacked and multi-dimensional packed arrays out.
        self.eval_const_expr(index)?;
        let ExprKind::Ident(hier) = &base.kind else {
            return None;
        };
        if self.lookup_array_name(hier).is_some() {
            return None;
        }
        // A multi-D PACKED base (`logic [1:0][3:0][7:0] foo`) is NOT a
        // flattening no-op: `foo[0]` selects a slice, so `foo[0][j]` must
        // not degrade to a bit-select of the whole vector (§7.4.1).
        //
        // Ask the SHAPE question, not the width question. `packed_elem_width_of`
        // filters out elem_w == 1, which is right for a single-index select
        // (element `i` of `[N-1:0][0:0]` IS physically bit `i`, so `foo[i]`
        // may fuse as a bit-select) but wrong here: `foo[i][0]` still selects
        // WITHIN element `i`. With the unit-width element filtered away this
        // guard never fired, the whole-vector signal id was returned, and the
        // caller emitted `BlockingAssignBitDyn(foo, 0)` — every `foo[i][0]`
        // wrote bit 0 regardless of `i`. `[N-1:0][1:0]` was correct precisely
        // because the filter let its width through and this guard bailed.
        if self.is_packed_multi_dim(hier) {
            return None;
        }
        // A genuine 2D/ND UNPACKED array (`logic [7:0] m [2][2]`) also carries
        // a bogus scalar signal for its base name; `m[0][j]` must select the
        // element (interpreter path), NOT bit-select that scalar.
        if self.is_multi_dim_array(hier) {
            return None;
        }
        self.lookup_signal_id(hier)
    }

    /// True when `hier`'s base name is a registered 2D/ND unpacked array.
    pub(super) fn is_multi_dim_array(&self, hier: &HierarchicalIdentifier) -> bool {
        let Some(set) = self.multi_dim_arrays else {
            return false;
        };
        let raw = Self::hier_raw_name(hier);
        if set.contains(raw.as_str()) {
            return true;
        }
        if let Some(scope) = &self.scope_hint {
            if set.contains(cat_dot(scope, &raw).as_str()) {
                return true;
            }
        }
        if hier.path.len() == 1 && set.contains(hier.path[0].name.name.as_str()) {
            return true;
        }
        false
    }

    pub(super) fn flattened_const_range_target(
        &self,
        expr: &Expression,
        left: &Expression,
        right: &Expression,
    ) -> Option<(usize, u32, u32)> {
        let ExprKind::Index { expr: base, index } = &expr.kind else {
            return None;
        };
        let outer = self.eval_const_expr(index)?;
        let ExprKind::Ident(hier) = &base.kind else {
            return None;
        };
        if self.lookup_array_name(hier).is_some() {
            return None;
        }
        if self.is_multi_dim_array(hier) {
            return None;
        }
        let id = self.lookup_signal_id(hier)?;
        let l = self.eval_const_expr(left)?;
        let r = self.eval_const_expr(right)?;
        let (lo, hi) = if l >= r { (r, l) } else { (l, r) };
        // §7.4.1: the element stride is the DECLARED element width, not the
        // slice width — `d[1][31:0]` on `logic [4:0][63:0] d` targets bits
        // [95:64], not [63:32]. The slice-width fallback stays for signals
        // with no packed metadata (the generated-loop flattening case this
        // helper was built for, where slice == element by construction).
        let (stride, base_bit) = match self.packed_elem_width_of(hier) {
            Some(decl_ew) => {
                if hi >= decl_ew {
                    return None; // slice exceeds the element: not this shape
                }
                let dim = self.packed_outer_dim(hier);
                let lsb = Self::packed_elem_lsb(dim, outer as i64, decl_ew);
                if lsb < 0 {
                    return None;
                }
                (0, lsb as u32)
            }
            None => (hi - lo + 1, 0),
        };
        let flat_lo = if stride == 0 {
            base_bit.checked_add(lo)?
        } else {
            outer.checked_mul(stride)?.checked_add(lo)?
        };
        let flat_hi = if stride == 0 {
            base_bit.checked_add(hi)?
        } else {
            outer.checked_mul(stride)?.checked_add(hi)?
        };
        if flat_hi < self.signal_widths[id] {
            Some((id, flat_hi, flat_lo))
        } else {
            None
        }
    }

    /// Signal id of a multi-dimensional unpacked array element addressed as
    /// `base[i][j]…` with CONSTANT indices. `outer_base` is the expression to
    /// the left of the final index (itself one or more Index nodes over an
    /// Ident); `last_index` is the final subscript. Returns None for a dynamic
    /// index or when no such element is registered.
    pub(super) fn multi_dim_elem_signal_id(
        &self,
        outer_base: &Expression,
        last_index: &Expression,
    ) -> Option<usize> {
        // Only nested indexing can name a multi-dim element.
        if !matches!(outer_base.kind, ExprKind::Index { .. }) {
            return None;
        }
        // §11.5.1: an x/z literal folds to 0 here; it names no element.
        if self.is_xz_literal(last_index) {
            return None;
        }
        let mut subs: Vec<u32> = vec![self.eval_const_expr(last_index)?];
        let mut cur = outer_base;
        let hier = loop {
            match &cur.kind {
                ExprKind::Index { expr: b, index: i } => {
                    if self.is_xz_literal(i) {
                        return None;
                    }
                    subs.push(self.eval_const_expr(i)?);
                    cur = b;
                }
                ExprKind::Ident(h) => break h,
                _ => return None,
            }
        };
        subs.reverse();
        let raw = Self::hier_raw_name(hier);
        let suffix: String = subs.iter().map(|i| format!("[{}]", i)).collect();
        let mut candidates = vec![format!("{}{}", raw, suffix)];
        if let Some(scope) = &self.scope_hint {
            candidates.push(format!("{}.{}{}", scope, raw, suffix));
        }
        if let Some(leaf) = hier.path.last() {
            candidates.push(format!("{}{}", leaf.name.name, suffix));
        }
        candidates
            .iter()
            .find_map(|n| self.lookup_signal_id_by_name(n.as_str()))
    }

    pub(super) fn lookup_array_name(&self, hier: &HierarchicalIdentifier) -> Option<String> {
        // An ASSOCIATIVE array is never a dense array, whatever tables its
        // name also appears in: a dense LoadArrayElem keyed by (say) a string
        // literal reads garbage. Assoc lvalues have their own path
        // (`is_assoc_target`); reads that land here must bail instead.
        if self.is_assoc_target(hier) {
            return None;
        }
        let raw = Self::hier_raw_name(hier);
        let dense = |name: &str| -> bool {
            // An ARRAY OF COLLECTIONS registers its outer shape in `arrays`
            // but each element is its own queue/dynamic/associative container
            // (`int a[2][u8_t]`), living OUTSIDE the dense cells this name
            // resolves to — a compiled LoadArrayElem read the fake backing
            // and `mem[h][addr]` compared against garbage the moment the
            // enclosing loop compiled (found when the new do-while arm
            // compiled a block the old bail had kept on the AST path). The
            // element registrations are keyed `name[lo]`, so probe that.
            let Some((lo, _, _)) = self.arrays.get(name) else {
                return false;
            };
            let elem = format!("{}[{}]", name, lo);
            !(self.assoc_arrays.is_some_and(|m| m.contains_key(&elem))
                || self.queue_vars.is_some_and(|m| m.contains(&elem))
                || self.dynamic_arrays.is_some_and(|m| m.contains(&elem)))
        };
        let canonical = self.signal_name_to_id.canonical_interface_name(&raw);
        if canonical != raw && self.arrays.contains_key(canonical.as_ref()) {
            return dense(&canonical).then(|| canonical.into_owned());
        }
        if self.arrays.contains_key(&raw) {
            return dense(&raw).then_some(raw);
        }
        if let Some(scope) = &self.scope_hint {
            let qualified = cat_dot(scope, &raw);
            if self.arrays.contains_key(&qualified) {
                return dense(&qualified).then_some(qualified);
            }
        }
        if hier.path.len() == 1 {
            let leaf = &hier.path[0].name.name;
            if self.arrays.contains_key(leaf) {
                return dense(leaf).then_some(leaf.clone());
            }
        }
        // §6.20.2/§26.3 (issue #425): an UNPACKED-ARRAY package localparam
        // registers under its QUALIFIED key only (`pkg::LIM`, elements
        // `pkg::LIM[i]`) — bare visibility stays with the import machinery,
        // unlike package variables, which hoist bare. `hier_raw_name` joins
        // with '.', so `pkg::LIM` probed as "pkg.LIM" and missed, and the
        // element read degraded to a bit-select of a phantom scalar. Probe
        // the '::'-joined two-segment spelling; a dotted hierarchical name
        // never contains '::', so this cannot capture another name.
        if hier.path.len() == 2
            && hier.path.iter().all(|s| s.selects.is_empty())
            && hier.root.is_none()
        {
            let q = format!("{}::{}", hier.path[0].name.name, hier.path[1].name.name);
            if self.arrays.contains_key(&q) {
                return dense(&q).then_some(q);
            }
        }
        // Top-prefix strip, as `resolve_signal_id` does for scalars: an
        // absolute `<top>.<inst>.<mem>` path names the array registered as
        // `<inst>.<mem>` (top-level instances carry no prefix). Without it a
        // testbench's memory-image loops (`tb.x_soc...ram0.mem[i][7:0] = ...`)
        // bailed to the statement interpreter at ~1.5 us a store.
        if let Some(top) = &self.top_module_name {
            if let Some(stripped) = raw.strip_prefix(&format!("{}.", top)) {
                if self.arrays.contains_key(stripped) {
                    return dense(stripped).then_some(stripped.to_string());
                }
            }
        }
        None
    }

    /// Does `e` name a whole unpacked array, dynamic array or queue (or a
    /// slice of one)? Conservative: any name registered as a collection in
    /// this scope or at the top counts.
    pub(super) fn names_unpacked_collection(&self, e: &Expression) -> bool {
        let e = match &e.kind {
            ExprKind::Paren(i) => i.as_ref(),
            ExprKind::RangeSelect { expr, .. } => expr.as_ref(),
            _ => e,
        };
        let ExprKind::Ident(h) = &e.kind else {
            return false;
        };
        if h.path.iter().any(|s| !s.selects.is_empty()) {
            return false;
        }
        let raw = h
            .path
            .iter()
            .map(|s| s.name.name.as_str())
            .collect::<Vec<_>>()
            .join(".");
        let is_coll = |n: &str| {
            self.arrays.contains_key(n)
                || self.arrays_2d.is_some_and(|m| m.contains_key(n))
                || self.multi_dim_arrays.is_some_and(|m| m.contains(n))
                || self.dynamic_arrays.is_some_and(|m| m.contains(n))
                || self.queue_vars.is_some_and(|m| m.contains(n))
        };
        if is_coll(&raw) {
            return true;
        }
        self.scope_hint
            .as_ref()
            .is_some_and(|scope| is_coll(&cat_dot(scope, &raw)))
    }

    /// Hoist disjoint full-range `dst[i] <= src[i]` copies out of a canonical
    /// packed loop. All slices are written in the same NBA region and the
    /// sources are side-effect-free signal reads, so a single whole-vector NBA
    /// is equivalent while avoiding loop-control and per-slice queue work.
    /// Every uncertain shape returns None and uses normal lowering.
    pub(super) fn full_range_nba_copy_plan(
        &self,
        init: &[ForInit],
        condition: Option<&Expression>,
        step: &[Expression],
        body: &Statement,
    ) -> Option<(Vec<(usize, usize, u32)>, Statement)> {
        let [
            ForInit::VarDecl {
                name: loop_id,
                init: start_expr,
                ..
            },
        ] = init
        else {
            return None;
        };
        let loop_name = loop_id.name.as_str();
        let start = self.eval_const_expr(start_expr)? as i64;
        let ExprKind::Binary {
            op: cmp,
            left,
            right: bound_expr,
        } = &condition?.kind
        else {
            return None;
        };
        if !matches!(cmp, BinaryOp::Lt | BinaryOp::Leq)
            || Self::plain_loop_ident(left) != Some(loop_name)
        {
            return None;
        }
        let bound = self.eval_const_expr(bound_expr)? as i64;
        let end = if matches!(cmp, BinaryOp::Lt) {
            bound.checked_sub(1)?
        } else {
            bound
        };
        if end < start {
            return None;
        }
        let [step_expr] = step else { return None };
        let ExprKind::Unary { op, operand } = &step_expr.kind else {
            return None;
        };
        if !matches!(op, UnaryOp::PreIncr | UnaryOp::PostIncr)
            || Self::plain_loop_ident(operand) != Some(loop_name)
        {
            return None;
        }
        let count = u32::try_from(end - start + 1).ok()?;
        if count <= 1 {
            return None;
        }

        let (block_name, stmts): (Option<_>, &[Statement]) = match &body.kind {
            StatementKind::SeqBlock { name, stmts } => (name.clone(), stmts),
            _ => (None, std::slice::from_ref(body)),
        };
        // A blocking assignment, timing control, or call-as-statement could
        // change a copied source between iterations. The target loop is all
        // delay-free NBAs, whose queued updates cannot affect later samples.
        if !stmts.iter().all(|st| {
            matches!(
                &st.kind,
                StatementKind::NonblockingAssign { delay: None, .. }
            )
        }) {
            return None;
        }

        let mut candidates: Vec<(usize, usize, usize, u32)> = Vec::new();
        for (si, st) in stmts.iter().enumerate() {
            let StatementKind::NonblockingAssign {
                lvalue,
                delay: None,
                rvalue,
            } = &st.kind
            else {
                continue;
            };
            let Some((dst_h, dst_idx)) = Self::plain_indexed_signal(lvalue) else {
                continue;
            };
            let Some((src_h, src_idx)) = Self::plain_indexed_signal(rvalue) else {
                continue;
            };
            if Self::plain_loop_ident(dst_idx) != Some(loop_name)
                || Self::plain_loop_ident(src_idx) != Some(loop_name)
            {
                continue;
            }
            if self.lookup_array_name(dst_h).is_some() || self.lookup_array_name(src_h).is_some() {
                continue;
            }
            let Some(dst_dim) = self.packed_outer_dim(dst_h) else {
                continue;
            };
            let Some(src_dim) = self.packed_outer_dim(src_h) else {
                continue;
            };
            let covers = |(l, r): (i64, i64)| {
                start == l.min(r) && end == l.max(r) && count as i64 == (l - r).abs() + 1
            };
            // A whole assignment preserves element identity only when source
            // and destination use the same declared orientation.
            if dst_dim != src_dim || !covers(dst_dim) || !covers(src_dim) {
                continue;
            }
            let Some(dst) = self.lookup_signal_id(dst_h) else {
                continue;
            };
            let Some(src) = self.lookup_signal_id(src_h) else {
                continue;
            };
            let Some(&dw) = self.signal_widths.get(dst) else {
                continue;
            };
            let Some(&sw) = self.signal_widths.get(src) else {
                continue;
            };
            let de = self.infer_lhs_width(lvalue).max(1);
            let se = self.infer_lhs_width(rvalue).max(1);
            if dw != sw
                || de != se
                || de.checked_mul(count) != Some(dw)
                || se.checked_mul(count) != Some(sw)
                || dst == src
            {
                continue;
            }
            candidates.push((si, dst, src, dw));
        }
        if candidates.is_empty() {
            return None;
        }
        let mut unique_dests: HashSet<usize> = HashSet::default();
        if candidates
            .iter()
            .any(|(_, dst, _, _)| !unique_dests.insert(*dst))
        {
            return None;
        }
        // Sampling may move ahead of the residual loop only when no copied
        // source is also written by another hoisted copy.
        let dests: HashSet<usize> = candidates.iter().map(|(_, d, _, _)| *d).collect();
        if candidates.iter().any(|(_, _, s, _)| dests.contains(s)) {
            return None;
        }
        let skipped: HashSet<usize> = candidates.iter().map(|(i, ..)| *i).collect();
        // Preserve last-NBA-wins ordering. A non-identity NBA to a candidate
        // destination may occur before or after the identity copy in the
        // original body; moving every identity copy ahead of the loop would
        // otherwise reverse one of those cases.
        for (i, st) in stmts.iter().enumerate() {
            if skipped.contains(&i) {
                continue;
            }
            let StatementKind::NonblockingAssign { lvalue, .. } = &st.kind else {
                return None;
            };
            let Some(root) = Self::plain_selected_signal_root(lvalue) else {
                return None;
            };
            let Some(id) = self.lookup_signal_id(root) else {
                return None;
            };
            if dests.contains(&id) {
                return None;
            }
        }
        let kept: Vec<Statement> = stmts
            .iter()
            .enumerate()
            .filter(|(i, _)| !skipped.contains(i))
            .map(|(_, st)| st.clone())
            .collect();
        let pruned = if matches!(&body.kind, StatementKind::SeqBlock { .. }) {
            Statement::new(
                StatementKind::SeqBlock {
                    name: block_name,
                    stmts: kept.into(),
                },
                body.span,
            )
        } else {
            Statement::new(StatementKind::Null, body.span)
        };
        let plans = candidates
            .into_iter()
            .map(|(_, d, s, w)| (d, s, w))
            .collect();
        Some((plans, pruned))
    }

    fn plain_loop_ident(expr: &Expression) -> Option<&str> {
        match &expr.kind {
            ExprKind::Ident(h) if h.root.is_none() && h.path.len() == 1 => {
                Some(h.path[0].name.name.as_str())
            }
            ExprKind::Paren(inner) => Self::plain_loop_ident(inner),
            _ => None,
        }
    }

    fn plain_indexed_signal(expr: &Expression) -> Option<(&HierarchicalIdentifier, &Expression)> {
        let ExprKind::Index { expr: base, index } = &expr.kind else {
            return None;
        };
        let ExprKind::Ident(h) = &base.kind else {
            return None;
        };
        if h.path.iter().any(|seg| !seg.selects.is_empty()) {
            return None;
        }
        Some((h, index))
    }

    fn plain_selected_signal_root(expr: &Expression) -> Option<&HierarchicalIdentifier> {
        match &expr.kind {
            ExprKind::Ident(h) if h.path.iter().all(|seg| seg.selects.is_empty()) => Some(h),
            ExprKind::Paren(inner)
            | ExprKind::Index { expr: inner, .. }
            | ExprKind::RangeSelect { expr: inner, .. } => Self::plain_selected_signal_root(inner),
            _ => None,
        }
    }
}
