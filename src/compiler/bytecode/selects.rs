//! Index chains, element selects and part-selects.

use super::*;

impl<'a> BytecodeCompiler<'a> {
    /// Walk a chain of `Index` nodes down to its root identifier. Returns the
    /// root and the index EXPRESSIONS outermost-first; indices need not be
    /// constant.
    pub(super) fn flatten_index_chain_exprs<'e>(
        &self,
        base: &'e Expression,
        index: &'e Expression,
    ) -> Option<(&'e HierarchicalIdentifier, Vec<&'e Expression>)> {
        let mut idxs: Vec<&Expression> = vec![index];
        let mut cur = base;
        loop {
            match &cur.kind {
                ExprKind::Index { expr, index } => {
                    idxs.push(index);
                    cur = expr.as_ref();
                }
                ExprKind::Ident(h) => {
                    idxs.reverse();
                    return Some((h, idxs));
                }
                _ => return None,
            }
        }
    }

    /// Width of a chained select `d[i][j]...` on a multi-D packed signal: the
    /// product of the dimensions the chain leaves unconsumed. The indices may
    /// be dynamic — the selected width depends only on how many levels are
    /// indexed. `packed_full_dims_of` lists every packed dimension down to the
    /// bit (element types included), so the product of them all must equal
    /// the signal's width; any other layout, and every unpacked, associative
    /// or collection root, answers None and keeps its existing path.
    pub(super) fn chained_packed_select_width(&self, lhs: &Expression) -> Option<u32> {
        let ExprKind::Index { expr, index } = &lhs.kind else {
            return None;
        };
        let (hier, idxs) = self.flatten_index_chain_exprs(expr, index)?;
        if idxs.len() < 2
            || self.lookup_array_name(hier).is_some()
            || self.is_multi_dim_array(hier)
            || self.is_assoc_target(hier)
        {
            return None;
        }
        let id = self.lookup_signal_id(hier)?;
        let dims = self.packed_full_dims_of(hier)?;
        if dims.len() < idxs.len() {
            return None;
        }
        let counts: Vec<u64> = dims
            .iter()
            .map(|&(l, r)| (l.max(r) - l.min(r) + 1) as u64)
            .collect();
        if counts.iter().product::<u64>() != self.signal_widths[id] as u64 {
            return None;
        }
        u32::try_from(counts[idxs.len()..].iter().product::<u64>()).ok()
    }

    /// Walk a chain of constant `Index` nodes down to its root identifier.
    /// Returns the root and the indices outermost-first.
    fn flatten_const_index_chain<'e>(
        &self,
        base: &'e Expression,
        index: &Expression,
    ) -> Option<(&'e HierarchicalIdentifier, Vec<i64>)> {
        let mut idxs = vec![self.eval_const_expr(index)? as i64];
        let mut cur = base;
        loop {
            match &cur.kind {
                ExprKind::Index { expr, index } => {
                    idxs.push(self.eval_const_expr(index)? as i64);
                    cur = expr.as_ref();
                }
                ExprKind::Ident(h) => {
                    idxs.reverse();
                    return Some((h, idxs));
                }
                _ => return None,
            }
        }
    }

    /// Declared dimensions of a chain's root, if registered.
    fn chain_root_dims(&self, hier: &HierarchicalIdentifier) -> Option<Vec<(i64, i64)>> {
        self.packed_full_dims_of(hier).cloned()
    }

    /// Emit a chained packed element select whose indices are NOT all
    /// constant — `a[i][j][3]` inside a loop. The selected WIDTH is still
    /// static (it depends only on how many dimensions were consumed), so only
    /// the offset needs computing at run time:
    /// `off = Σ slot_k * (product of the counts below level k)`.
    ///
    /// Without this, a dynamic chain fell through to the plain bit-select path
    /// and read x — the shape a `for (i) for (j) vld[i][j][0]` checker loop
    /// produces, which is why such loops had to be hand-unrolled to work.
    pub(super) fn emit_chained_packed_slice_dyn(
        &mut self,
        base: &Expression,
        index: &Expression,
    ) -> Option<RegId> {
        let (hier, idx_exprs) = self.flatten_index_chain_exprs(base, index)?;
        if idx_exprs.len() < 2 || self.is_unpacked_chain_root(hier) {
            return None;
        }
        let dims = self.chain_root_dims(hier)?;
        if dims.len() < idx_exprs.len() {
            return None;
        }
        let counts: Vec<i64> = dims.iter().map(|(l, r)| (l - r).abs() + 1).collect();
        let width: i64 = counts[idx_exprs.len()..].iter().product();
        if width <= 0 {
            return None;
        }
        // Compile every index first; bail before emitting any accumulation if
        // one of them cannot be compiled.
        let mut idx_regs = Vec::with_capacity(idx_exprs.len());
        for e in &idx_exprs {
            idx_regs.push(self.compile_expr(e, 0)?);
        }
        let root_sig = self.lookup_signal_id(hier);
        let lo_reg = self.emit_packed_chain_offset(&dims, idx_regs)?;
        // A signal root is sliced IN PLACE. `compile_expr_root_of` copied
        // the whole vector into a register first, so a loop reading one
        // byte of a 512-bit table cloned all 512 bits on every iteration —
        // the copy, not the select, was the cost.
        if let Some(sig) = root_sig {
            let dest = self.alloc_reg();
            self.emit(Insn::LoadSignalRangeDyn(
                dest,
                as_sig_id(sig),
                lo_reg,
                width as u32,
            ));
            return Some(dest);
        }
        let root = self.compile_expr_root_of(base)?;
        Some(self.emit_dyn_slice(root, lo_reg, width as u32))
    }

    /// `src[lo +: width]` with a run-time `lo`. §11.5.1: an x/z index reads
    /// x at the element's width; a computed upper bound is x as well and
    /// lost it (`00x` for `s.f0[i][j]` of 3-bit elements).
    pub(super) fn emit_dyn_slice(&mut self, src: RegId, lo_reg: RegId, width: u32) -> RegId {
        let dest = self.alloc_reg();
        self.emit(Insn::RangeSelectW(dest, src, lo_reg, width, true));
        dest
    }

    /// Run-time bit offset of the packed element that `idx_regs` (outermost
    /// first) select out of a vector with packed dimensions `dims`:
    /// `off = Σ slot_k * (product of the counts below level k)`.
    pub(super) fn emit_packed_chain_offset(
        &mut self,
        dims: &[(i64, i64)],
        idx_regs: Vec<RegId>,
    ) -> Option<RegId> {
        let counts: Vec<i64> = dims.iter().map(|(l, r)| (l - r).abs() + 1).collect();
        let mut off_reg: Option<RegId> = None;
        for (k, idx_reg) in idx_regs.into_iter().enumerate() {
            let (l, r) = dims[k];
            let (lo_b, hi_b) = (l.min(r), l.max(r));
            let elem_w: i64 = counts[k + 1..].iter().product();
            // §7.4.1: descending labels the LEFT bound most-significant, so the
            // slot counts up from the low bound; ascending reverses it.
            let slot = if l >= r {
                if lo_b == 0 {
                    idx_reg
                } else {
                    let c = self.alloc_reg();
                    self.emit(Insn::LoadConst(
                        c,
                        Box::new(Value::from_u64(lo_b as u64, 32)),
                    ));
                    let d = self.alloc_reg();
                    self.emit(Insn::Sub(d, idx_reg, c));
                    d
                }
            } else {
                let c = self.alloc_reg();
                self.emit(Insn::LoadConst(
                    c,
                    Box::new(Value::from_u64(hi_b as u64, 32)),
                ));
                let d = self.alloc_reg();
                self.emit(Insn::Sub(d, c, idx_reg));
                d
            };
            let term = if elem_w == 1 {
                slot
            } else {
                let w = self.alloc_reg();
                self.emit(Insn::LoadConst(
                    w,
                    Box::new(Value::from_u64(elem_w as u64, 32)),
                ));
                let t = self.alloc_reg();
                self.emit(Insn::Mul(t, slot, w));
                t
            };
            off_reg = Some(match off_reg {
                None => term,
                Some(acc) => {
                    let a = self.alloc_reg();
                    self.emit(Insn::Add(a, acc, term));
                    a
                }
            });
        }
        off_reg
    }

    /// `(lsb, width)` of a chained packed element select, from the root's
    /// declared dimensions. None unless there are at least TWO indices and the
    /// root has enough registered dimensions — a single-level select keeps its
    /// existing, separately-tested path.
    pub(super) fn chained_packed_slice(
        &self,
        base: &Expression,
        index: &Expression,
    ) -> Option<(u32, u32)> {
        let (hier, idxs) = self.flatten_const_index_chain(base, index)?;
        if idxs.len() < 2 || self.is_unpacked_chain_root(hier) {
            return None;
        }
        let dims: Vec<(i64, i64)> = self.chain_root_dims(hier)?;
        if dims.len() < idxs.len() {
            return None;
        }
        Self::packed_chain_offset(&dims, &idxs)
    }

    /// `(lsb, width)` of the packed element that the constant `idxs`
    /// (outermost first) select out of a vector with packed dimensions
    /// `dims`; None when an index is outside its declared range.
    pub(super) fn packed_chain_offset(dims: &[(i64, i64)], idxs: &[i64]) -> Option<(u32, u32)> {
        let counts: Vec<i64> = dims.iter().map(|(l, r)| (l - r).abs() + 1).collect();
        let mut off: i64 = 0;
        for (k, &d) in idxs.iter().enumerate() {
            let (l, r) = dims[k];
            let (lo_b, hi_b) = (l.min(r), l.max(r));
            if d < lo_b || d > hi_b {
                return None;
            }
            // §7.4.1: a descending range labels the LEFT bound as the most
            // significant element; an ascending one reverses the slot order.
            let slot = if l >= r { d - lo_b } else { hi_b - d };
            let elem_w: i64 = counts[k + 1..].iter().product();
            off = off.checked_add(slot.checked_mul(elem_w)?)?;
        }
        let width: i64 = counts[idxs.len()..].iter().product();
        Some((u32::try_from(off).ok()?, u32::try_from(width).ok()?))
    }

    /// Compile the ROOT identifier of an index chain (the whole backing
    /// vector), so a chained slice can be taken out of it.
    pub(super) fn compile_expr_root_of(&mut self, e: &Expression) -> Option<RegId> {
        let mut cur = e;
        while let ExprKind::Index { expr, .. } = &cur.kind {
            cur = expr.as_ref();
        }
        let root = cur.clone();
        self.compile_expr(&root, 0)
    }

    /// True when a chain's root is an unpacked array (one-, two- or
    /// more-dimensional): its first indices address elements, never the
    /// packed dimensions `packed_full_dims` lists for the element.
    fn is_unpacked_chain_root(&self, hier: &HierarchicalIdentifier) -> bool {
        self.lookup_array_name(hier).is_some() || self.is_multi_dim_array(hier)
    }

    /// §7.4.1, §7.4.5: how many indices of the chain `base[index]` address
    /// the root's UNPACKED dimensions, when the indices after them select into
    /// a multi-dimensional packed element (`d[1][0]` on
    /// `logic [1:0][7:0] d [2]`, `b[0][1]` on `le_t [1:0] b [2]` with `le_t`
    /// a packed struct): 2 for a two-dimensional array, the length of the
    /// constant prefix naming a stored element for a deeper one, 1 for a
    /// one-dimensional array; with the element's packed dimensions. None for
    /// any other chain. The flag is false when the element's packed range
    /// labels are not known here (see `elem_packed_dims`).
    pub(super) fn unpacked_elem_select_depth(
        &self,
        base: &Expression,
        index: &Expression,
    ) -> Option<(usize, ElemPackedDims)> {
        let (hier, idxs) = self.flatten_index_chain_exprs(base, index)?;
        if self.is_assoc_target(hier) {
            return None;
        }
        let n = idxs.len();
        let (depth, elem_w) = if let Some((_, shape)) = self.arrays_2d_entry(hier) {
            (2, Some(shape.2))
        } else if self.is_multi_dim_array(hier) {
            (1..n).find_map(|k| match Self::chain_prefix(base, n - 1 - k) {
                Some(Expression {
                    kind: ExprKind::Index { expr, index },
                    ..
                }) => self
                    .multi_dim_elem_signal_id(expr, index)
                    .map(|id| (k, self.signal_widths.get(id).copied())),
                _ => None,
            })?
        } else {
            let name = self.lookup_array_name(hier)?;
            (1, self.arrays.get(&name).map(|&(_, _, w)| w))
        };
        let (dims, labelled) = self.elem_packed_dims(hier, elem_w)?;
        (dims.len() >= 2 && n > depth && n - depth <= dims.len())
            .then_some((depth, (dims, labelled)))
    }

    /// §7.4.1: the packed dimensions of an element of the unpacked array
    /// `hier`, outermost first, down to single bits, and whether their range
    /// labels are the declared ones. `packed_full_dims` lists the packed
    /// ARRAY dimensions; a packed struct element adds none, so when the
    /// element (`elem_w` bits) is wider than their product the rest is one
    /// more dimension of that width. When no dimensions are recorded (a
    /// single `[N-1:0]` range, or any range of a typedef-named struct type)
    /// the packed element width still gives the shape, but not whether the
    /// range is `[N-1:0]`, `[0:N-1]` or offset, so the labels are not known.
    fn elem_packed_dims(
        &self,
        hier: &HierarchicalIdentifier,
        elem_w: Option<u32>,
    ) -> Option<ElemPackedDims> {
        let (mut dims, labelled) = match self.packed_full_dims_of(hier) {
            Some(d) => (d.clone(), true),
            None => {
                let (ew, w) = (self.packed_elem_width_of(hier)?, elem_w?);
                if ew <= 1 || w <= ew || !w.is_multiple_of(ew) {
                    return None;
                }
                (vec![(i64::from(w / ew) - 1, 0)], false)
            }
        };
        if let Some(w) = elem_w.map(i64::from) {
            let count: i64 = dims.iter().map(|(l, r)| (l - r).abs() + 1).product();
            if count > 0 && w > count && w % count == 0 {
                dims.push((w / count - 1, 0));
            }
        }
        Some((dims, labelled))
    }

    /// True for a chain `base[index]` over an unpacked array of three or
    /// more dimensions where no constant prefix of the indices names a
    /// stored element (`a3[i][j][k]`, `a3[i][j][k][b]`): elements are
    /// separate signals reached by their flat names, and a run-time index
    /// has none.
    pub(super) fn nd_chain_names_no_element(&self, base: &Expression, index: &Expression) -> bool {
        let Some((hier, idxs)) = self.flatten_index_chain_exprs(base, index) else {
            return false;
        };
        let n = idxs.len();
        if n < 2 || !self.is_multi_dim_array(hier) || self.arrays_2d_entry(hier).is_some() {
            return false;
        }
        if self.multi_dim_elem_signal_id(base, index).is_some() {
            return false;
        }
        !(1..n).any(|k| match Self::chain_prefix(base, n - 1 - k) {
            Some(Expression {
                kind: ExprKind::Index { expr, index },
                ..
            }) => self.multi_dim_elem_signal_id(expr, index).is_some(),
            _ => false,
        })
    }

    /// A literal with x or z bits (`'x`, `2'b1z`), possibly parenthesized.
    pub(super) fn is_xz_literal(&self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Number(n) => self.eval_number_static(n).is_some_and(|v| v.has_xz()),
            ExprKind::Paren(i) => self.is_xz_literal(i),
            _ => false,
        }
    }

    /// The node `strip` `Index` levels below `e` (`e` itself for 0).
    fn chain_prefix(e: &Expression, strip: usize) -> Option<&Expression> {
        let mut cur = e;
        for _ in 0..strip {
            let ExprKind::Index { expr, .. } = &cur.kind else {
                return None;
            };
            cur = expr;
        }
        Some(cur)
    }

    /// Read `base[index]` whose first `depth` indices name an unpacked
    /// element and the rest select a packed element inside it: the element
    /// through its own read path, then a constant or run-time slice of it.
    pub(super) fn compile_unpacked_elem_packed_select(
        &mut self,
        base: &Expression,
        index: &Expression,
        depth: usize,
        dims: &[(i64, i64)],
    ) -> Option<RegId> {
        let (_, idxs) = self.flatten_index_chain_exprs(base, index)?;
        let packed_idxs = &idxs[depth..];
        let counts: Vec<i64> = dims.iter().map(|(l, r)| (l - r).abs() + 1).collect();
        let width = u32::try_from(counts[packed_idxs.len()..].iter().product::<i64>()).ok()?;
        let elem_expr = Self::chain_prefix(base, idxs.len() - depth - 1)?;
        let elem = self.compile_expr(elem_expr, 0)?;
        // §11.5.1: an x/z index reads x. `eval_const_expr` folds an x/z
        // literal to 0, so such an index is never taken as a constant; the
        // run-time path below carries the x through the offset.
        let consts: Option<Vec<i64>> = packed_idxs
            .iter()
            .map(|e| {
                (!self.is_xz_literal(e))
                    .then(|| self.eval_const_expr(e).map(|v| v as i64))
                    .flatten()
            })
            .collect();
        if let Some(consts) = consts {
            let dest = self.alloc_reg();
            match Self::packed_chain_offset(dims, &consts) {
                Some((lo, w)) => self.emit(Insn::RangeSelectConst(dest, elem, lo + w - 1, lo)),
                // §11.5.1: an index outside the declared range reads x.
                None => self.emit(Insn::LoadConst(dest, Box::new(Value::new(width)))),
            }
            return Some(dest);
        }
        let mut idx_regs = Vec::with_capacity(packed_idxs.len());
        for e in packed_idxs {
            idx_regs.push(self.compile_expr(e, 0)?);
        }
        let lo_reg = self.emit_packed_chain_offset(dims, idx_regs)?;
        Some(self.emit_dyn_slice(elem, lo_reg, width))
    }

    /// The base's registered packed ELEMENT width (>1), if it is a
    /// multi-dimensional packed vector (`logic [3:0][7:0] x`).
    /// True when `hier` names a packed MULTI-DIMENSIONAL signal, regardless of
    /// its element width. `packed_elem_width_of` answers "how wide is an
    /// element, if that width changes codegen" and so drops unit-width
    /// elements; callers that need "is this a packed array at all" — where a
    /// further select indexes INSIDE an element — must use this instead.
    pub(super) fn is_packed_multi_dim(&self, hier: &HierarchicalIdentifier) -> bool {
        let raw = Self::hier_raw_name(hier);
        self.packed_elem_widths
            .and_then(|m| {
                self.packed_shape_at(m, &raw).copied().or_else(|| {
                    hier.path
                        .last()
                        .and_then(|s| m.get(s.name.name.as_str()).copied())
                })
            })
            .is_some()
    }

    pub(super) fn packed_elem_width_of(&self, hier: &HierarchicalIdentifier) -> Option<u32> {
        let raw = Self::hier_raw_cow(hier);
        let top_rest = self.top_module_name.as_ref()
            .and_then(|top| raw.strip_prefix(top.as_str()).and_then(|s| s.strip_prefix('.')));
        let rooted = hier.root.is_some() || top_rest.is_some();
        let raw = top_rest.unwrap_or(raw.as_ref());
        self.packed_elem_widths
            .and_then(|m| {
                self.packed_shape_at(m, raw)
                    .copied()
                    // Inside an inlined instance the name is spelled bare
                    // while the table holds it under the instance path.
                    .or_else(|| {
                        if rooted { return None; }
                        self.scope_hint
                            .as_ref()
                            .and_then(|sc| with_dotted(sc, &raw, |k| m.get(k).copied()))
                    })
                    // The bare-leaf fallback is for a SINGLE-segment name only.
                    // Applying it to `inp.sram_renA` (a packed-struct member
                    // of a port) matched an unrelated `logic [3:0][1:0]
                    // sram_renA` declared in another module, and the
                    // member's bit-select compiled as that array's two-bit
                    // element select: bits 2 and 3 read x.
                    .or_else(|| {
                        if hier.path.len() != 1 {
                            return None;
                        }
                        hier.path
                            .last()
                            .and_then(|s| m.get(s.name.name.as_str()).copied())
                    })
            })
            .filter(|&w| w > 1)
    }

    /// §7.4.1: physical LSB offset of declared bit index 0 does not exist for
    /// a non-zero-based vector — `logic [3:1] w` stores declared bit 1 at
    /// physical offset 0. Returns the declared range's LOW bound when it is
    /// non-zero (descending ranges only; ascending is handled elsewhere), so
    /// write emission can rebase declared indices the way the read path
    /// already does. Every `*AssignRange`/`*AssignBitDyn` the compiler emits
    /// carries PHYSICAL offsets by contract — the interpreter and the JIT
    /// both index raw bits.
    /// Emit `idx_reg - declared_low_bound` when the vector is non-zero-based;
    /// pass-through otherwise. Used by every dynamic-index WRITE emission.
    pub(super) fn emit_rebased_index(
        &mut self,
        hier: &HierarchicalIdentifier,
        idx_reg: RegId,
    ) -> RegId {
        let base_lo = self.declared_low_bound(hier);
        if base_lo == 0 {
            return idx_reg;
        }
        let base_reg = self.alloc_reg();
        self.emit(Insn::LoadConst(
            base_reg,
            Box::new(Value::from_u64(base_lo as u64, 32)),
        ));
        let adj = self.alloc_reg();
        self.emit(Insn::Sub(adj, idx_reg, base_reg));
        adj
    }

    pub(super) fn declared_low_bound(&self, hier: &HierarchicalIdentifier) -> i64 {
        self.packed_outer_dim(hier)
            .map(|(dl, dr)| dl.min(dr))
            .unwrap_or(0)
    }

    /// ASCENDING declared range (`logic [0:23]`): bit/part labels mirror
    /// against the right bound — the low-bound rebase the emitted insns use
    /// cannot express that, so such selects must run on the AST interpreter.
    pub(super) fn dim_is_ascending(d: Option<(i64, i64)>) -> bool {
        matches!(d, Some((l, r)) if l < r)
    }

    /// Trailing bit/part select whose base is an Index CHAIN over a signal
    /// whose packed dims need label mapping — an ELEMENT of an unpacked
    /// collection declared `logic [31:8]`/`logic [0:23]` style, or a packed
    /// element under such an inner dim. The emitted select/store insns index
    /// raw physical bits, so these shapes must bail to the AST interpreter
    /// (which maps labels via `select_base_elem_dim`).
    fn elem_chain_needs_ast(&self, base: &Expression) -> bool {
        let mut cur = base;
        let mut layers = 0usize;
        while let ExprKind::Index { expr: inner, .. } = &cur.kind {
            cur = inner;
            layers += 1;
        }
        if layers == 0 {
            return false;
        }
        let ExprKind::Ident(h) = &cur.kind else {
            return false;
        };
        let raw = Self::hier_raw_name(h);
        let leaf = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
        let Some(dims) = self
            .packed_full_dims
            .and_then(|m| m.get(raw.as_str()).or_else(|| m.get(leaf)))
        else {
            return false;
        };
        let is_coll = |n: &str| -> bool {
            self.array_first_id.is_some_and(|m| m.contains_key(n))
                || self.multi_dim_arrays.is_some_and(|s| s.contains(n))
                || self.dynamic_arrays.is_some_and(|s| s.contains(n))
                || self.queue_vars.is_some_and(|s| s.contains(n))
                || self.assoc_arrays.is_some_and(|m| m.contains_key(n))
        };
        let needs = |d: &(i64, i64)| d.0 < d.1 || d.0.min(d.1) != 0;
        if is_coll(raw.as_str()) || is_coll(leaf) {
            // The exact unpacked depth is not knowable here — be
            // conservative: any mapping dim forces the AST path.
            dims.iter().any(needs)
        } else {
            dims.get(layers).is_some_and(needs)
        }
    }

    /// Combined guard for the plain bit/range select and store arms: bail
    /// when the base needs a label mapping those arms do not emit. A packed
    /// multi-D Ident base is excluded — its element machinery normalizes
    /// slots itself (including ascending outer dims).
    /// §7.4.1 declared dimension of whatever a select applies to: the vector
    /// itself for `sig[..]`, or the ELEMENT an index chain lands on for
    /// `sig[i][..]`. `None` when the layout is not on file, which leaves the
    /// historical zero-based reading in place.
    pub(super) fn sel_base_dim(&self, base: &Expression) -> Option<(i64, i64)> {
        let mut cur = base;
        let mut layers = 0usize;
        while let ExprKind::Index { expr: inner, .. } = &cur.kind {
            cur = inner;
            layers += 1;
        }
        if let Some((_, m)) = self.formal_struct_member(cur) {
            return match &m.elems {
                Some(dims) => dims.get(layers).copied(),
                None if layers == 0 => m.dim,
                None => None,
            };
        }
        let ExprKind::Ident(h) = &cur.kind else {
            return None;
        };
        let dims = self.packed_full_dims_of(h)?;
        if layers == 0 {
            return dims.first().copied();
        }
        // An element of an UNPACKED array keeps the interpreter path: writes
        // to `arr[i][label]` do not map the label there (they land at
        // `label * width`, while every read maps it), so compiling the read
        // alone would pair a mapped read with an unmapped write. The plain
        // packed bases below have no such gap.
        if self.lookup_array_name(h).is_some() || self.is_multi_dim_array(h) {
            return None;
        }
        dims.get(layers).copied()
    }

    /// §7.4.3, §11.5.1: a part-select picks whole ELEMENTS of the declared
    /// dimension that follows the indices already applied — `a[1:0]` on
    /// `logic [3:0][7:0] a` is two 8-bit elements, and `a[0][5:0]` on
    /// `logic [3:0][5:0][7:0] a` is six of them, 48 bits, not bits 5..0.
    /// Returns that dimension and the width in bits of one of its elements,
    /// for a packed variable or packed-struct member and for an element of a
    /// one- or two-dimensional unpacked array. None when the select addresses
    /// single bits (the last dimension) or the layout is not on file; those
    /// keep their bit-level paths.
    pub(super) fn part_select_elem(&self, base: &Expression) -> Option<((i64, i64), u32)> {
        let mut cur = base;
        let mut layers = 0usize;
        while let ExprKind::Index { expr: inner, .. } = &cur.kind {
            cur = inner;
            layers += 1;
        }
        let ExprKind::Ident(h) = &cur.kind else {
            return None;
        };
        if self.formal_struct_member(cur).is_some()
            || self.frame_bound_target(base)
            || self.is_assoc_target(h)
            || self.collection_store_denied(h)
        {
            return None;
        }
        // Unpacked indices come first and select a whole stored element.
        let (depth, total) = if let Some((_, shape)) = self.arrays_2d_entry(h) {
            (2, shape.2)
        } else if self.is_multi_dim_array(h) {
            return None;
        } else if let Some(name) = self.lookup_array_name(h) {
            (1, self.arrays.get(&name)?.2)
        } else {
            let total = match self.lookup_signal_id(h) {
                Some(id) => self.signal_widths[id],
                None => self.packed_struct_member_target(h)?.2,
            };
            (0, total)
        };
        let level = layers.checked_sub(depth)?;
        // A plain vector selects bits; skip the layout copy below.
        if depth == 0
            && self.packed_elem_width_of(h).is_none()
            && self
                .packed_full_dims_of(h)
                .is_none_or(|d| d.len() <= level + 1)
        {
            return None;
        }
        let (dims, labelled) = self.elem_packed_dims(h, Some(total))?;
        // Without the declared labels only the widths are known, which is
        // enough for the outermost dimension of a variable — its select
        // arms assume `[N-1:0]` already — but not to place a deeper slice.
        if !labelled && layers > 0 {
            return None;
        }
        let counts: Vec<u64> = dims
            .iter()
            .map(|&(l, r)| (l.max(r) - l.min(r) + 1) as u64)
            .collect();
        if dims.len() <= level + 1 || counts.iter().product::<u64>() != u64::from(total) {
            return None;
        }
        let stride = u32::try_from(counts[level + 1..].iter().product::<u64>()).ok()?;
        (stride > 1).then_some((dims[level], stride))
    }

    /// Did a select base compile to nothing but an interpreter call?
    ///
    /// An identifier the compiler cannot resolve reaches `emit_expr_fallback`,
    /// which asks the AST interpreter for one scalar. That is right for a
    /// scalar, and silently wrong for an AGGREGATE: a class property holding
    /// an unpacked array evaluates to a single value, and selecting an element
    /// or a bit of that read 0 rather than the element — `bk.arr[i]` inside an
    /// `always` block accumulated nothing while `$display` of the same
    /// expression printed the right number, because the display goes through
    /// the interpreter, which walks the whole select.
    ///
    /// The interpreter evaluates the entire select correctly, and the base was
    /// already going to enter it once, so handing it the whole expression
    /// costs the same one call.
    pub(super) fn sel_base_is_interpreted(&self, from: usize, base: &Expression) -> bool {
        matches!(
            base.kind,
            ExprKind::Ident(_) | ExprKind::MemberAccess { .. }
        ) && self.insns[from..]
            .iter()
            .any(|i| matches!(i, Insn::EvalExprFallback(..)))
    }

    pub(super) fn sel_base_needs_ast(&self, base: &Expression) -> bool {
        match &base.kind {
            ExprKind::Ident(h) => {
                self.packed_elem_width_of(h).filter(|&w| w > 1).is_none()
                    && Self::dim_is_ascending(self.packed_outer_dim(h))
            }
            ExprKind::Index { .. } => self.elem_chain_needs_ast(base),
            _ => false,
        }
    }
}
