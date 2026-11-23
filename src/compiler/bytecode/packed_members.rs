//! Packed struct members, formal members and packed shapes.

use super::*;

impl<'a> BytecodeCompiler<'a> {
    /// If `hier` names a packed-struct member (`base.member`, where the base
    /// resolves to a container signal with a registered field layout), return
    /// `(container_signal_id, lsb_offset, member_width)`. The base may be a
    /// single segment (`s`) or already scope-qualified with a dot inside the
    /// first path segment (`d1.s`) after submodule inlining; the member is the
    /// final path segment.
    pub(super) fn packed_struct_member_target(
        &self,
        hier: &HierarchicalIdentifier,
    ) -> Option<(usize, u32, u32)> {
        let fields_map = self.packed_struct_fields?;
        if hier.path.len() < 2 || hier.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let seg = |i: usize| hier.path[i].name.name.as_str();
        // A NESTED member (`s.p.hi`, and every `union`-in-struct shape) is
        // flattened by elaboration into one dotted key — "p.hi" — stored under
        // the ROOT signal. Splitting only the last segment therefore missed
        // every member at depth >= 2 and sent it to the AST path. Walk the
        // split point from the longest base down, so a genuinely hierarchical
        // base that IS a signal (`top.dut.sig.field`) still wins over
        // reinterpreting part of it as a member path.
        for k in (1..hier.path.len()).rev() {
            let base: String = (0..k).map(seg).collect::<Vec<_>>().join(".");
            let member: String = (k..hier.path.len()).map(seg).collect::<Vec<_>>().join(".");
            // Resolve the container signal id, honoring scope_hint for a bare base.
            let Some(base_id) = self.lookup_signal_id_by_name(&base).or_else(|| {
                self.scope_hint
                    .as_ref()
                    .and_then(|sc| self.lookup_signal_id_by_name(&format!("{}.{}", sc, base)))
            }) else {
                continue;
            };
            // Field layout is keyed by both the bare and scope-qualified base name.
            let Some(fields) = fields_map.get(base.as_str()).or_else(|| {
                self.scope_hint
                    .as_ref()
                    .and_then(|sc| fields_map.get(&format!("{}.{}", sc, base)))
            }) else {
                continue;
            };
            if let Some((_, off, w)) = fields.iter().find(|(m, _, _)| *m == member) {
                return Some((base_id, *off, *w));
            }
        }
        None
    }

    /// §7.2, §7.4.2: a member select through constant element selects of a
    /// packed struct (`s.e[1].dw[0].value`). Elaboration flattens a member that
    /// is a packed array of structs into one entry per element (`e[1]`,
    /// `e[1].dw[0].value`, ...), so with constant indices the whole select is
    /// one static bit range of the container signal. Returns
    /// `(container_signal_id, layout key, member path, lsb_offset, width)`, or
    /// `None` for anything else (a dynamic or x/z index, an index outside the
    /// declared range, an unpacked root), which keeps its existing path.
    pub(super) fn packed_const_member_path_target(
        &self,
        lhs: &Expression,
    ) -> Option<(usize, String, String, u32, u32)> {
        let fields_map = self.packed_struct_fields?;
        // A local of the frame (a function's own struct) shadows a module
        // signal of the same name; it is not this signal's layout.
        if self.frame_bound_target(lhs) {
            return None;
        }
        // The selects below the root, innermost (leaf) first.
        let mut tail: Vec<String> = Vec::new();
        let mut has_index = false;
        let mut cur = lhs;
        let hier = loop {
            match &cur.kind {
                ExprKind::MemberAccess { expr, member } => {
                    tail.push(format!(".{}", member.name));
                    cur = expr;
                }
                ExprKind::Index { expr, index } => {
                    if self.is_xz_literal(index) {
                        return None;
                    }
                    tail.push(format!("[{}]", self.plain_const_index(index)?));
                    has_index = true;
                    cur = expr;
                }
                ExprKind::Paren(inner) => cur = inner,
                ExprKind::Ident(h) => break h,
                _ => return None,
            }
        };
        if !has_index {
            return None;
        }
        if hier.root.is_some() || hier.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        if self.lookup_array_name(hier).is_some() || self.is_multi_dim_array(hier) {
            return None;
        }
        tail.reverse();
        let suffix: String = tail.concat();
        let seg = |i: usize| hier.path[i].name.name.as_str();
        // As `packed_struct_member_target`: the longest base that is a
        // container signal with a layout wins. Elaboration may already have
        // folded the leading members into the identifier (`s.e` then `[1]`).
        // The container must be entered through a member (`s.e[1]`), not an
        // element (`a[1].f`, an array of structs, which has its own stores).
        for k in (1..=hier.path.len()).rev() {
            let base: String = (0..k).map(seg).collect::<Vec<_>>().join(".");
            let mut member: String = (k..hier.path.len()).map(seg).collect::<Vec<_>>().join(".");
            if !member.is_empty() {
                member.push_str(&suffix);
            } else if let Some(rest) = suffix.strip_prefix('.') {
                member = rest.to_string();
            } else {
                continue;
            }
            // The instance's own variable first, as `compile_packed_member_read`
            // resolves a base: a bare key may belong to the top module or to
            // another instance.
            let scoped = self
                .scope_hint
                .as_ref()
                .filter(|sc| !sc.is_empty() && !base.contains('.'))
                .map(|sc| format!("{}.{}", sc, base));
            let Some((base_id, key)) = scoped
                .and_then(|s| Some((self.lookup_signal_id_by_name(&s)?, s)))
                .or_else(|| Some((self.lookup_signal_id_by_name(&base)?, base.clone())))
            else {
                continue;
            };
            let Some(fields) = fields_map.get(key.as_str()) else {
                continue;
            };
            if let Some(&(_, off, w)) = fields.iter().find(|(m, _, _)| *m == member)
                && w > 0
            {
                return Some((base_id, key, member, off, w));
            }
        }
        None
    }

    /// §11.6.1: an index whose value does not depend on expression width: a
    /// literal, a parameter or a bound constant, optionally negated or in
    /// parentheses. An expression such as `1'b1 + 1'b1` is self-determined
    /// (it wraps to 0) and `eval_const_bound` does not model that, so it is
    /// left to the interpreter. So is a local of the running frame (§23.9: a
    /// function's `int I` hides a module's `parameter I`), unless it is an
    /// unrolled loop's index, bound to a constant for this iteration.
    ///
    /// A literal or parameter is read at its own value (§11.5.1: an index
    /// keeps its full width; `eval_const_bound` narrows to 32 bits, so
    /// `e[64'h1_0000_0001]` was `e[1]`). One with an x or z bit, or one wider
    /// than 64 bits, is left to the interpreter like any other index this
    /// path cannot place.
    pub(super) fn plain_const_index(&self, e: &Expression) -> Option<i64> {
        match &e.kind {
            ExprKind::Paren(inner) => self.plain_const_index(inner),
            // §11.4.3, §11.8.2: the negation is at the literal's own width
            // and signedness, so it wraps there: `-4'sd8` is -8, `-4'd3` is
            // 13, `-1` is -1.
            ExprKind::Unary {
                op: UnaryOp::Minus,
                operand,
            } if matches!(operand.kind, ExprKind::Number(_)) => {
                let ExprKind::Number(n) = &operand.kind else {
                    return None;
                };
                let v = self.eval_number_static(n)?;
                let x = Self::exact_index_value(&v)?;
                if v.width == 0 || v.width > 63 {
                    return None;
                }
                let m = 1i128 << v.width;
                let mut r = (-i128::from(x)).rem_euclid(m);
                if v.is_signed && r >= m / 2 {
                    r -= m;
                }
                i64::try_from(r).ok()
            }
            ExprKind::Number(n) => Self::exact_index_value(&self.eval_number_static(n)?),
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                let name = h.path[0].name.name.as_str();
                if h.root.is_none()
                    && let Some(v) = self.local_const_vars.get(name)
                {
                    return Self::exact_index_value(v);
                }
                if let Some(&v) = self.const_var_binds.get(name) {
                    return Some(v as i64);
                }
                if self.frame_bound_target(e) {
                    return None;
                }
                Self::exact_index_value(&self.lookup_param_value(h)?)
            }
            _ => None,
        }
    }

    /// The value of a constant index of at most 64 bits with no x or z bit,
    /// signed or unsigned as declared, when it fits in an `i64`.
    pub(super) fn exact_index_value(v: &Value) -> Option<i64> {
        if v.has_xz() || v.width > 64 {
            return None;
        }
        if v.is_signed {
            v.to_i64()
        } else {
            v.to_u64().and_then(|u| i64::try_from(u).ok())
        }
    }

    /// Resolve a packed-struct container and clone its flattened field layout.
    /// The clone keeps later instruction emission free to mutably borrow the
    /// compiler without retaining a borrow into elaboration metadata.
    pub(super) fn packed_struct_layout_for_hier(
        &self,
        hier: &HierarchicalIdentifier,
    ) -> Option<(String, Vec<(String, u32, u32)>)> {
        if hier.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let fields_map = self.packed_struct_fields?;
        let raw = Self::hier_raw_name(hier);
        let mut candidates = Vec::with_capacity(3);
        candidates.push(raw.clone());
        if !raw.contains('.')
            && let Some(scope) = &self.scope_hint
        {
            candidates.push(cat_dot(scope, &raw));
        }
        if let Some(leaf) = hier.path.last() {
            candidates.push(leaf.name.name.clone());
        }
        for candidate in candidates {
            if let Some(fields) = fields_map.get(candidate.as_str()) {
                return Some((candidate, fields.clone()));
            }
        }
        None
    }

    /// Compile either a packed-struct signal or a selected element of a packed
    /// array of structs, retaining the root layout used to interpret members.
    pub(super) fn compile_packed_struct_value(
        &mut self,
        expr: &Expression,
    ) -> Option<(
        RegId,
        String,
        HierarchicalIdentifier,
        Vec<(String, u32, u32)>,
    )> {
        let mut unwrapped = expr;
        while let ExprKind::Paren(inner) = &unwrapped.kind {
            unwrapped = inner;
        }
        // §7.4.1: an element of an unpacked array of any rank
        // (`s3[1][0]`), not only of a one-dimensional one; its read path
        // resolves the element.
        let mut root = unwrapped;
        let mut levels = 0;
        while let ExprKind::Index { expr: base, .. } = &root.kind {
            root = base;
            levels += 1;
        }
        let ExprKind::Ident(hier) = &root.kind else {
            return None;
        };
        if levels > 1 && !self.is_multi_dim_array(hier) {
            return None;
        }
        let hier = hier.clone();
        let (key, fields) = self.packed_struct_layout_for_hier(&hier)?;
        let value = self.compile_expr(unwrapped, 0)?;
        Some((value, key, hier, fields))
    }

    /// The flattened member layout of a packed-struct formal, computed as the
    /// interpreter's `register_formal_type_metadata` does. A 4-state struct
    /// with 2-state members is left out: those members read x/z as 0, which
    /// a slice does not do (see `mixed_packed_vars`).
    pub(super) fn packed_formal_layout(
        &self,
        dt: &crate::ast::types::DataType,
        width: u32,
    ) -> Option<Vec<(String, FormalMember)>> {
        use crate::compiler::elaborate as el;
        let types = self.typedef_types?;
        let fields = el::packed_struct_field_layout(dt, self.params?, self.typedefs?, types)?;
        if fields.is_empty() || fields.iter().any(|&(_, off, w)| w == 0 || off + w > width) {
            return None;
        }
        let two_state = el::is_type_two_state_resolved(dt, types);
        if !two_state && Self::has_two_state_member(dt, types) {
            return None;
        }
        Some(
            fields
                .into_iter()
                .filter_map(|(n, off, width)| {
                    let (signed, dim, elems) = self.struct_member_type(dt, &n)?;
                    Some((
                        n,
                        FormalMember {
                            off,
                            width,
                            signed,
                            dim,
                            elems,
                            two_state,
                        },
                    ))
                })
                .collect(),
        )
    }

    /// Signedness and declared range of the member at dotted `path` (`p.hi`)
    /// of a packed struct. None when it, or a member on the way to it, has
    /// a dimension that does not span its bits (a packed array of wider
    /// elements): a select on it picks elements, not bits. The exception is
    /// a member declared directly as a multi-dimensional packed array of a
    /// built-in vector type (`logic [3:0][4:0] sh`): its dimensions come
    /// back as the third item, and its elements are unsigned (§7.4.1).
    #[allow(clippy::type_complexity)]
    fn struct_member_type(
        &self,
        dt: &crate::ast::types::DataType,
        path: &str,
    ) -> Option<(bool, Option<(i64, i64)>, Option<Box<[(i64, i64)]>>)> {
        use crate::compiler::elaborate as el;
        let (params, typedefs, types) = (self.params?, self.typedefs?, self.typedef_types?);
        let crate::ast::types::DataType::Struct(su) = el::resolve_typedef_chain(dt, types) else {
            return None;
        };
        let (head, rest) = path
            .split_once('.')
            .map_or((path, None), |(h, r)| (h, Some(r)));
        let m = su
            .members
            .iter()
            .find(|m| m.declarators.iter().any(|d| d.name.name == head))?;
        let q = el::type_query_dims_of(&m.data_type, Some(params), Some(typedefs), Some(types));
        let w = el::resolve_type_width(&m.data_type, Some(params), Some(typedefs));
        let signed = || el::is_type_signed_resolved(&m.data_type, types);
        let dim = match q.dims.len() {
            0 => q.whole,
            1 => Some(q.dims[0]),
            _ => {
                let direct = matches!(
                    m.data_type,
                    crate::ast::types::DataType::IntegerVector { ref dimensions, .. }
                        if dimensions.len() == q.dims.len()
                );
                let bits = q
                    .dims
                    .iter()
                    .try_fold(1u64, |a, &(l, r)| a.checked_mul(l.abs_diff(r) + 1));
                return (rest.is_none() && direct && bits == Some(u64::from(w)))
                    .then(|| (signed(), None, Some(q.dims.into_boxed_slice())));
            }
        };
        match dim {
            Some((l, r)) if l.abs_diff(r) + 1 != u64::from(w) => return None,
            None if w != 1 => return None,
            _ => {}
        }
        match rest {
            Some(r) => self.struct_member_type(&m.data_type, r),
            None => Some((signed(), dim, None)),
        }
    }

    fn has_two_state_member(
        dt: &crate::ast::types::DataType,
        types: &HashMap<String, crate::ast::types::DataType>,
    ) -> bool {
        use crate::compiler::elaborate as el;
        let crate::ast::types::DataType::Struct(su) = el::resolve_typedef_chain(dt, types) else {
            return false;
        };
        su.members.iter().any(|m| {
            el::is_type_two_state_resolved(&m.data_type, types)
                || Self::has_two_state_member(&m.data_type, types)
        })
    }

    /// The dotted name of a select-free member chain (`c.p.hi`), whether it
    /// arrives as nested member accesses or folded into one identifier, as
    /// in an instance's rewritten function body.
    fn member_chain_name(expr: &Expression) -> Option<String> {
        match &expr.kind {
            ExprKind::Ident(hier)
                if hier.root.is_none() && hier.path.iter().all(|s| s.selects.is_empty()) =>
            {
                Some(Self::hier_raw_name(hier))
            }
            ExprKind::MemberAccess { expr, member } => {
                Some(cat_dot(&Self::member_chain_name(expr)?, &member.name))
            }
            _ => None,
        }
    }

    /// The member of a register-bound packed-struct formal that `expr`
    /// reads (`c.a`, `c.p.hi`), with the formal's register.
    pub(super) fn formal_struct_member(&self, expr: &Expression) -> Option<(RegId, &FormalMember)> {
        if self.local_var_struct.is_empty() {
            return None;
        }
        let dotted = Self::member_chain_name(expr)?;
        let mut cut = dotted.len();
        while let Some(i) = dotted[..cut].rfind('.') {
            let (base, member) = (&dotted[..i], &dotted[i + 1..]);
            if let Some((slot, fields)) = self.local_var_struct.get(base)
                && self.local_var_regs.get(base).map(|b| b.0) == Some(*slot)
            {
                let (_, m) = fields.iter().find(|f| f.0 == member)?;
                return Some((*slot, m));
            }
            cut = i;
        }
        None
    }

    /// §7.2.1: a packed struct is a vector whose members are fixed bit
    /// ranges, so a member of a packed-struct formal is a constant slice of
    /// its register, with the member's own signedness.
    pub(super) fn compile_formal_struct_member(&mut self, expr: &Expression) -> Option<RegId> {
        let (slot, m) = self.formal_struct_member(expr)?;
        let (off, w, signed) = (m.off, m.width, m.signed);
        let dest = self.alloc_reg();
        self.emit(Insn::RangeSelectConst(dest, slot, off + w - 1, off));
        if signed {
            self.emit(Insn::SetSigned(dest));
        }
        Some(dest)
    }

    /// An index chain (`c.sh[i]`, `c.m3[i][j]`) on a multi-dimensional
    /// packed-array member of a register-bound packed-struct formal: the
    /// formal's register, the member, and the indices, outermost first.
    pub(super) fn formal_member_chain<'e>(
        &self,
        expr: &'e Expression,
    ) -> Option<(RegId, &FormalMember, Vec<&'e Expression>)> {
        let mut cur = expr;
        let mut idx = Vec::new();
        while let ExprKind::Index { expr: inner, index } = &cur.kind {
            idx.push(&**index);
            cur = inner;
        }
        if idx.is_empty() {
            return None;
        }
        let (slot, m) = self.formal_struct_member(cur)?;
        m.elems.as_ref()?;
        idx.reverse();
        Some((slot, m, idx))
    }

    /// §7.4.1, §11.5.1: an element (or a bit) of a packed-array member of a
    /// packed-struct formal is a slice of the member, and the member a
    /// constant slice of the formal's register. Each index selects from the
    /// element the previous one selected, so an index outside its own
    /// dimension (or an x/z one) reads x instead of a neighbouring element.
    /// The elements are unsigned. `None` when `expr` is not such a chain;
    /// `Some(None)` when it is but does not compile (the caller bails).
    pub(super) fn compile_formal_member_elem(
        &mut self,
        expr: &Expression,
    ) -> Option<Option<RegId>> {
        let (slot, m, idx) = self.formal_member_chain(expr)?;
        let (off, width, two_state) = (m.off, m.width, m.two_state);
        let dims: Vec<(i64, i64)> = m.elems.as_deref()?.to_vec();
        if idx.len() > dims.len() {
            return Some(None);
        }
        let mut cur = self.alloc_reg();
        self.emit(Insn::RangeSelectConst(cur, slot, off + width - 1, off));
        for (j, index) in idx.iter().enumerate() {
            let (l, r) = dims[j];
            let size = l.abs_diff(r) + 1;
            let ew = Self::formal_member_chain_width_of(&dims, j + 1);
            let dest = self.alloc_reg();
            if let Some(label) = self.fold_const(index).and_then(|v| v.to_index()) {
                let phys = Self::label_to_phys((l, r), label);
                if phys < 0 || phys as u64 >= size {
                    // Out of range: x, or 0 for a 2-state struct (§7.4.6);
                    // the interpreter decides.
                    return Some(None);
                }
                let lo = phys as u32 * ew;
                self.emit(Insn::RangeSelectConst(dest, cur, lo + ew - 1, lo));
            } else {
                // A wider index could alias into range in the 32-bit
                // position arithmetic; a 2-state struct reads 0, not x,
                // out of range. Both stay on the interpreter.
                if two_state || self.expr_max_width(index) > 32 {
                    return Some(None);
                }
                let Some(i) = self.compile_expr(index, 0) else {
                    return Some(None);
                };
                // Physical element position: `label - lo` on a descending
                // range, `hi - label` on an ascending one.
                let pos = self.alloc_reg();
                let k = self.alloc_reg();
                let (lo_b, hi_b) = (l.min(r), l.max(r));
                if l >= r {
                    self.emit(Insn::LoadConst(
                        k,
                        Box::new(Value::from_u64(lo_b as u32 as u64, 32)),
                    ));
                    self.emit(Insn::Sub(pos, i, k));
                } else {
                    self.emit(Insn::LoadConst(
                        k,
                        Box::new(Value::from_u64(hi_b as u32 as u64, 32)),
                    ));
                    self.emit(Insn::Sub(pos, k, i));
                }
                self.emit(Insn::Resize(pos, 32));
                let ew_reg = self.alloc_reg();
                self.emit(Insn::LoadConst(
                    ew_reg,
                    Box::new(Value::from_u64(u64::from(ew), 32)),
                ));
                let lo = self.alloc_reg();
                self.emit(Insn::Mul(lo, pos, ew_reg));
                self.emit(Insn::Resize(lo, 32));
                self.emit(Insn::RangeSelectW(dest, cur, lo, ew, true));
            }
            cur = dest;
        }
        Some(Some(cur))
    }

    /// Is `base` of a part-select a packed-array member of a struct formal,
    /// or an element of it above its last dimension?
    pub(super) fn formal_member_elem_range(&self, base: &Expression) -> bool {
        let mut cur = base;
        let mut layers = 0usize;
        while let ExprKind::Index { expr: inner, .. } = &cur.kind {
            cur = inner;
            layers += 1;
        }
        self.formal_struct_member(cur)
            .and_then(|(_, m)| m.elems.as_deref())
            .is_some_and(|dims| layers + 1 != dims.len())
    }

    /// The width of what a `formal_member_chain` of `depth` indices
    /// selects: an element of the dimension it stops at, or one bit.
    pub(super) fn formal_member_chain_width_of(dims: &[(i64, i64)], depth: usize) -> u32 {
        dims[depth.min(dims.len())..]
            .iter()
            .map(|&(l, r)| (l.abs_diff(r) + 1) as u32)
            .product()
    }

    /// Metadata for a packed-array member of a packed struct. The elaborated
    /// tables key member dimensions as `<container>.<member>`.
    fn packed_member_array_shape(
        &self,
        root_key: &str,
        hier: &HierarchicalIdentifier,
        member: &str,
    ) -> Option<(u32, Option<(i64, i64)>)> {
        let widths = self.packed_elem_widths?;
        for candidate in self.packed_member_keys(root_key, hier, member) {
            if let Some(&elem_w) = widths.get(candidate.as_str()).filter(|&&w| w > 0) {
                let dim = self
                    .packed_full_dims
                    .and_then(|dims| dims.get(candidate.as_str()))
                    .and_then(|dims| dims.first())
                    .copied();
                return Some((elem_w, dim));
            }
        }
        None
    }

    /// The names a packed struct member's metadata may be registered under.
    fn packed_member_keys(
        &self,
        root_key: &str,
        hier: &HierarchicalIdentifier,
        member: &str,
    ) -> Vec<String> {
        let raw = Self::hier_raw_name(hier);
        let mut candidates = Vec::with_capacity(4);
        candidates.push(format!("{}.{}", root_key, member));
        candidates.push(format!("{}.{}", raw, member));
        if !raw.contains('.')
            && let Some(scope) = &self.scope_hint
        {
            candidates.push(format!("{}.{}.{}", scope, raw, member));
        }
        if let Some(leaf) = hier.path.last() {
            candidates.push(format!("{}.{}", leaf.name.name, member));
        }
        candidates
    }

    /// §7.4.1: `container.member[i][j]…`, two or more indices into a
    /// multi-dimensional packed member of a packed struct (`sa[1].f0[3][7]`
    /// on `t3_t [3:0][7:0] f0` of an array of structs). The one-index form
    /// slices one element of the member's outer dimension; a second index
    /// then took one BIT of that slice instead of an element of the next
    /// dimension. The member's dimensions down to single bits place every
    /// index.
    pub(super) fn compile_packed_member_index_chain(
        &mut self,
        base: &Expression,
        index: &Expression,
    ) -> Option<RegId> {
        let mut idx_exprs: Vec<&Expression> = vec![index];
        let mut cur = base;
        while let ExprKind::Index { expr, index } = &cur.kind {
            idx_exprs.push(index);
            cur = expr;
        }
        if idx_exprs.len() < 2 {
            return None;
        }
        idx_exprs.reverse();
        let ExprKind::MemberAccess { expr: root, member } = &cur.kind else {
            return None;
        };
        let (root_value, root_key, hier, fields) = self.compile_packed_struct_value(root)?;
        let &(_, field_offset, field_w) = fields.iter().find(|(n, _, _)| n == &member.name)?;
        let all_dims = self.packed_full_dims?;
        let dims: Vec<(i64, i64)> = self
            .packed_member_keys(&root_key, &hier, &member.name)
            .iter()
            .find_map(|k| all_dims.get(k.as_str()))?
            .clone();
        let counts: Vec<i64> = dims.iter().map(|(l, r)| (l - r).abs() + 1).collect();
        if idx_exprs.len() > dims.len() || counts.iter().product::<i64>() != i64::from(field_w) {
            return None;
        }
        let width = u32::try_from(counts[idx_exprs.len()..].iter().product::<i64>()).ok()?;
        // §11.5.1: an x/z literal index folds to 0 as a constant; the
        // run-time path carries the x through the offset.
        let consts: Option<Vec<i64>> = idx_exprs
            .iter()
            .map(|e| {
                (!self.is_xz_literal(e))
                    .then(|| self.eval_const_expr(e).map(|v| v as i64))
                    .flatten()
            })
            .collect();
        if let Some(consts) = consts {
            let dest = self.alloc_reg();
            match Self::packed_chain_offset(&dims, &consts) {
                Some((lo, w)) => {
                    let lo = field_offset + lo;
                    self.emit(Insn::RangeSelectConst(dest, root_value, lo + w - 1, lo));
                }
                // §11.5.1: an index outside the declared range reads x.
                None => self.emit(Insn::LoadConst(dest, Box::new(Value::new(width)))),
            }
            return Some(dest);
        }
        let mut idx_regs = Vec::with_capacity(idx_exprs.len());
        for e in idx_exprs {
            idx_regs.push(self.compile_expr(e, 0)?);
        }
        let mut lo_reg = self.emit_packed_chain_offset(&dims, idx_regs)?;
        if field_offset != 0 {
            let off = self.alloc_reg();
            self.emit(Insn::LoadConst(
                off,
                Box::new(Value::from_u64(u64::from(field_offset), 32)),
            ));
            let sum = self.alloc_reg();
            self.emit(Insn::Add(sum, lo_reg, off));
            lo_reg = sum;
        }
        Some(self.emit_dyn_slice(root_value, lo_reg, width))
    }

    /// Emit a dynamic element/field slice from a packed struct container.
    fn emit_packed_member_slice(
        &mut self,
        root: RegId,
        index: &Expression,
        dim: Option<(i64, i64)>,
        stride: u32,
        base_offset: u32,
        width: u32,
    ) -> Option<RegId> {
        let index = self.compile_expr(index, 0)?;
        let slot = self.emit_packed_slot_index(dim, index);
        let stride_reg = self.alloc_reg();
        self.emit(Insn::LoadConst(
            stride_reg,
            Box::new(Value::from_u64(stride as u64, 32)),
        ));
        let dynamic_offset = self.alloc_reg();
        self.emit(Insn::Mul(dynamic_offset, slot, stride_reg));
        let lo = if base_offset == 0 {
            dynamic_offset
        } else {
            let base = self.alloc_reg();
            self.emit(Insn::LoadConst(
                base,
                Box::new(Value::from_u64(base_offset as u64, 32)),
            ));
            let lo = self.alloc_reg();
            self.emit(Insn::Add(lo, dynamic_offset, base));
            lo
        };
        let hi = if width == 1 {
            lo
        } else {
            let delta = self.alloc_reg();
            self.emit(Insn::LoadConst(
                delta,
                Box::new(Value::from_u64((width - 1) as u64, 32)),
            ));
            let hi = self.alloc_reg();
            self.emit(Insn::Add(hi, lo, delta));
            hi
        };
        let dest = self.alloc_reg();
        self.emit(Insn::RangeSelect(dest, root, hi, lo));
        Some(dest)
    }

    /// Compile `container.array_member[index]` when the member is a packed
    /// array embedded in a packed struct.
    pub(super) fn compile_packed_member_index(
        &mut self,
        base: &Expression,
        index: &Expression,
    ) -> Option<RegId> {
        let ExprKind::MemberAccess { expr: root, member } = &base.kind else {
            return None;
        };
        // `a.m1[i].m2[j]`: the container is itself an element of an
        // array-of-struct member. Slice that element out first, then the
        // member lane within it (offsets inside the element are relative to
        // the element-0 layout). Without this the inner member read compiled
        // to a value and the outer index took ONE BIT of it.
        if let ExprKind::Index {
            expr: mid,
            index: idx1,
        } = &root.kind
            && let ExprKind::MemberAccess {
                expr: r2,
                member: m1,
            } = &mid.kind
        {
            let (v, key, hier, fields) = self.compile_packed_struct_value(r2)?;
            let &(_, m1_off, _) = fields.iter().find(|(n, _, _)| n == &m1.name)?;
            let (stride1, dim1) = self.packed_member_array_shape(&key, &hier, &m1.name)?;
            let elem = self.emit_packed_member_slice(v, idx1, dim1, stride1, m1_off, stride1)?;
            let (_, mem_off, _) = Self::elem_zero_field(&fields, &m1.name, &member.name, stride1)?;
            let rel = mem_off.checked_sub(m1_off)?;
            let nested = format!("{}.{}", m1.name, member.name);
            let (elem_w, dim2) = self.packed_member_array_shape(&key, &hier, &nested)?;
            return self.emit_packed_member_slice(elem, index, dim2, elem_w, rel, elem_w);
        }
        let (root_value, root_key, hier, fields) = self.compile_packed_struct_value(root)?;
        let (_, field_offset, _) = fields.iter().find(|(name, _, _)| name == &member.name)?;
        let (elem_w, dim) = self.packed_member_array_shape(&root_key, &hier, &member.name)?;
        self.emit_packed_member_slice(root_value, index, dim, elem_w, *field_offset, elem_w)
    }

    /// Compile `container.array_member[index].field` using the flattened
    /// packed-struct layout plus the member array's element stride.
    pub(super) fn compile_indexed_packed_member(
        &mut self,
        indexed: &Expression,
        leaf: &str,
    ) -> Option<RegId> {
        let ExprKind::Index { expr: base, index } = &indexed.kind else {
            return None;
        };
        // Inside an inlined instance the member access is folded into the
        // identifier (`cw.loc.descriptors[i].region` arrives as the single
        // path `cw.loc.descriptors`), so the shape below never matched and
        // every such read ran interpreted.
        if let ExprKind::Ident(hier) = &base.kind {
            if hier.root.is_some()
                || hier.path.len() < 2
                || hier.path.iter().any(|s| !s.selects.is_empty())
            {
                return None;
            }
            let mut root_hier = hier.clone();
            let member = root_hier.path.pop()?.name.name;
            return self.compile_indexed_folded_member(&root_hier, &member, index, leaf);
        }
        let ExprKind::MemberAccess { expr: root, member } = &base.kind else {
            return None;
        };
        let (root_value, root_key, hier, fields) = self.compile_packed_struct_value(root)?;
        let (elem_w, dim) = self.packed_member_array_shape(&root_key, &hier, &member.name)?;
        let (_, field_offset, field_width) =
            Self::elem_zero_field(&fields, &member.name, leaf, elem_w)?;
        self.emit_packed_member_slice(root_value, index, dim, elem_w, field_offset, field_width)
    }

    /// Layout of `<member>[i].<leaf>` in the element at the member's base
    /// (slot 0, the stride origin). Elaboration registers an array-of-struct
    /// member under two conventions: element-relative (`descriptors.region`,
    /// plain struct nets) or expanded per element (`descriptors[0].region`,
    /// packed arrays of structs). §7.4.1: element 0 sits at the base only
    /// for a descending range; in an ascending one (`hdr_t [0:2] slots`) it
    /// is the most significant element, so the leaf's offset is reduced to
    /// its place within one `stride`-bit element above the member's base.
    fn elem_zero_field(
        fields: &[(String, u32, u32)],
        member: &str,
        leaf: &str,
        stride: u32,
    ) -> Option<(String, u32, u32)> {
        let rel = format!("{}.{}", member, leaf);
        let per_elem = format!("{}[0].{}", member, leaf);
        let (name, off, w) = fields
            .iter()
            .find(|(name, _, _)| name == &rel || name == &per_elem)
            .cloned()?;
        match fields.iter().find(|(n, _, _)| n == member) {
            Some(&(_, base, _)) if stride > 0 && off >= base => {
                Some((name, base + (off - base) % stride, w))
            }
            _ => Some((name, off, w)),
        }
    }

    /// `<path>.<member>[i].<leaf>` where the container and member arrive as
    /// one folded identifier: split the member off the path, resolve the
    /// container's layout under the remaining name, and slice like the
    /// member-access shape.
    pub(super) fn compile_indexed_folded_member(
        &mut self,
        root_hier: &HierarchicalIdentifier,
        member: &str,
        index: &Expression,
        leaf: &str,
    ) -> Option<RegId> {
        if root_hier.root.is_some() || root_hier.path.is_empty() {
            return None;
        }
        // The container may itself be an element select (`grid[1]`): the
        // layout is keyed by the bare name, the value is the element.
        let mut bare = root_hier.clone();
        for seg in &mut bare.path {
            seg.selects.clear();
        }
        let (root_key, fields) = self.packed_struct_layout_for_hier(&bare)?;
        let (elem_w, dim) = self.packed_member_array_shape(&root_key, &bare, member)?;
        let (_, field_offset, field_width) = Self::elem_zero_field(&fields, member, leaf, elem_w)?;
        let root = if root_hier.path.iter().any(|s| !s.selects.is_empty()) {
            let e = Expression::new(ExprKind::Ident(root_hier.clone()), root_hier.span);
            self.compile_expr(&e, 0)?
        } else {
            let base_id = self.lookup_signal_id_by_name(&root_key)?;
            let root = self.alloc_reg();
            self.emit(Insn::LoadSignal(root, as_sig_id(base_id)));
            root
        };
        self.emit_packed_member_slice(root, index, dim, elem_w, field_offset, field_width)
    }

    /// The declared OUTERMOST packed dimension `(left, right)` of the signal
    /// named by `hier`, if recorded. Same raw / last-segment lookup the
    /// `packed_elem_widths` sites use.
    pub(super) fn packed_outer_dim(&self, hier: &HierarchicalIdentifier) -> Option<(i64, i64)> {
        self.packed_full_dims_of(hier)
            .and_then(|d| d.first().copied())
    }

    pub(super) fn record_local_packed_type(
        &mut self,
        reg: RegId,
        dt: &crate::ast::types::DataType,
    ) {
        let (Some(params), Some(types)) = (self.params, self.typedef_types) else {
            return;
        };
        if crate::compiler::elaborate::packed_full_dims_chained(dt, params, types)
            .is_some_and(|dims| dims.len() > 1)
        {
            self.local_packed_types.insert(reg, dt.clone());
        }
    }

    /// §7.4.1/§11.5.1: register-backed packed typedefs retain every
    /// dimension, including the bounds checked for each chained index.
    pub(super) fn compile_typed_packed_chain(
        &mut self,
        base: &Expression,
        last: &Expression,
    ) -> Option<RegId> {
        let mut indices = vec![last];
        let mut root = base;
        while let ExprKind::Index { expr, index } = &root.kind {
            indices.push(index);
            root = expr;
        }
        if indices.len() < 2 {
            return None;
        }
        let ExprKind::Ident(h) = &root.kind else {
            return None;
        };
        let (reg, _) = self.local_var_reg_of(h)?;
        let dt = self.local_packed_types.get(&reg)?.clone();
        let dims = crate::compiler::elaborate::packed_full_dims_chained(
            &dt,
            self.params?,
            self.typedef_types?,
        )?;
        if dims.len() < indices.len() || dims.len() < 2 {
            return None;
        }
        let width = dims[indices.len()..]
            .iter()
            .try_fold(1u64, |n, (l, r)| n.checked_mul(l.abs_diff(*r) + 1))?;
        let width = u32::try_from(width).ok()?;
        let default =
            if crate::compiler::elaborate::is_type_two_state_resolved(&dt, self.typedef_types?) {
                Value::zero(width)
            } else {
                Value::new(width)
            };
        indices.reverse();
        let value = self.compile_expr(root, 0)?;
        let consts: Option<Vec<i64>> = indices
            .iter()
            .map(|e| {
                (!self.is_xz_literal(e))
                    .then(|| self.fold_const(e).and_then(|v| v.to_i64()))
                    .flatten()
            })
            .collect();
        let dest = self.alloc_reg();
        if let Some(consts) = consts {
            match Self::packed_chain_offset(&dims, &consts) {
                Some((lo, w)) => self.emit(Insn::RangeSelectConst(dest, value, lo + w - 1, lo)),
                None => self.emit(Insn::LoadConst(dest, Box::new(default))),
            }
            return Some(dest);
        }
        let mut regs = Vec::new();
        for index in &indices {
            if self.expr_max_width(index) > 64 {
                return None;
            }
            regs.push(self.compile_expr(index, 0)?);
        }
        self.emit(Insn::LoadConst(dest, Box::new(default)));
        let mut guards = Vec::new();
        for ((expr, reg), &(left, right)) in indices.iter().zip(&regs).zip(&dims) {
            let iw = if self.expr_max_width(expr) >= 64 {
                65
            } else {
                64
            };
            let idx = self.alloc_reg();
            self.emit(Insn::Move(idx, *reg));
            self.emit(Insn::Resize(idx, iw));
            self.emit(Insn::SetSigned(idx));
            for (kind, bound) in [
                (CmpKind::Geq, left.min(right)),
                (CmpKind::Leq, left.max(right)),
            ] {
                let mut v = Value::from_u64(bound as u64, 64);
                v.is_signed = true;
                let constant = self.alloc_reg();
                self.emit(Insn::LoadConst(constant, Box::new(v.resize(iw))));
                let tmp = self.alloc_reg();
                guards.push(self.insns.len());
                self.emit(Insn::CmpBranch(kind, idx, constant, tmp, 0));
            }
        }
        let offset = self.emit_packed_chain_offset(&dims, regs)?;
        let selected = self.emit_dyn_slice(value, offset, width);
        self.emit(Insn::Move(dest, selected));
        let end = self.insns.len() as u32;
        for guard in guards {
            if let Insn::CmpBranch(_, _, _, _, target) = &mut self.insns[guard] {
                *target = end;
            }
        }
        Some(dest)
    }

    /// The unpacked bank owns the packed dimensions of each materialized
    /// memory cell. A bracket in an indexed instance prefix is not a cell.
    pub(super) fn packed_shape_at<'t, T>(
        &self,
        table: &'t HashMap<String, T>,
        name: &str,
    ) -> Option<&'t T> {
        table.get(name).or_else(|| {
            let (base, idx) = name.strip_suffix(']')?.rsplit_once('[')?;
            let shape = table.get(base)?;
            let idx = idx.parse::<i64>().ok()?;
            let &(lo, hi, _) = self.arrays.get(base)?;
            ((lo.min(hi)..=lo.max(hi)).contains(&idx) && self.signal_name_to_id.contains_key(name))
                .then_some(shape)
        })
    }

    /// Declared packed dimensions of `hier`: exact name, then the name under
    /// the current instance scope, then — for a SINGLE-segment name only —
    /// the bare leaf. A multi-segment name (a struct member, a hierarchical
    /// reference) must never match an unrelated same-named declaration
    /// elsewhere in the design (see `packed_elem_width_of`).
    pub(super) fn packed_full_dims_of(
        &self,
        hier: &HierarchicalIdentifier,
    ) -> Option<&'a Vec<(i64, i64)>> {
        let raw = Self::hier_raw_cow(hier);
        let m = self.packed_full_dims?;
        // §23.6 / §7.4.1: absolute names use the same top-prefix
        // normalization as signal lookup, including their packed geometry.
        let top_rest = self.top_module_name.as_ref()
            .and_then(|top| raw.strip_prefix(top.as_str()).and_then(|s| s.strip_prefix('.')));
        let rooted = hier.root.is_some() || top_rest.is_some();
        let raw = top_rest.unwrap_or(raw.as_ref());
        self.packed_shape_at(m, raw)
            .or_else(|| {
                if rooted { return None; }
                self.scope_hint
                    .as_ref()
                    .and_then(|sc| with_dotted(sc, &raw, |k| m.get(k)))
            })
            .or_else(|| {
                if hier.path.len() != 1 {
                    return None;
                }
                hier.path.last().and_then(|s| m.get(s.name.name.as_str()))
            })
    }

    /// LSB bit offset of packed element `idx` given the declared outer
    /// dimension. `[N-1:0]` reduces to the historical `idx * elem_w`.
    pub(super) fn packed_elem_lsb(dim: Option<(i64, i64)>, idx: i64, elem_w: u32) -> i64 {
        let Some((l, r)) = dim else {
            return idx * elem_w as i64;
        };
        let (lo_b, hi_b) = (l.min(r), l.max(r));
        let count = hi_b - lo_b + 1;
        let off = idx - lo_b;
        // A descending range labels the LEFT bound as the most-significant
        // element; an ascending one reverses the slot order (§7.4.1).
        let slot = if l >= r { off } else { count - 1 - off };
        slot * elem_w as i64
    }

    /// Emit the register holding the element's *slot* index (already
    /// normalized to 0-based, LSB-first) for a DYNAMIC index. Returns the
    /// original register unchanged for a normalized `[N-1:0]` range.
    pub(super) fn emit_packed_slot_index(
        &mut self,
        dim: Option<(i64, i64)>,
        idx_reg: RegId,
    ) -> RegId {
        let Some((l, r)) = dim else { return idx_reg };
        let (lo_b, hi_b) = (l.min(r), l.max(r));
        if l >= r {
            if lo_b == 0 {
                return idx_reg; // normalized [N-1:0]
            }
            let lo_reg = self.alloc_reg();
            self.emit(Insn::LoadConst(
                lo_reg,
                Box::new(Value::from_u64(lo_b as u64, 32)),
            ));
            let out = self.alloc_reg();
            self.emit(Insn::Sub(out, idx_reg, lo_reg));
            out
        } else {
            // ascending: slot = (count-1) - (idx - lo_b) = (count-1+lo_b) - idx
            let count = hi_b - lo_b + 1;
            let k = self.alloc_reg();
            self.emit(Insn::LoadConst(
                k,
                Box::new(Value::from_u64((count - 1 + lo_b) as u64, 32)),
            ));
            let out = self.alloc_reg();
            self.emit(Insn::Sub(out, k, idx_reg));
            out
        }
    }

    /// Bit offset `slot * ew + bit` of bit `bit` of the `ew`-bit element in
    /// run-time slot `slot` (see `emit_packed_slot_index`).
    pub(super) fn emit_slot_bit(&mut self, slot: RegId, ew: u32, bit: u32) -> RegId {
        let w = self.alloc_reg();
        self.emit(Insn::LoadConst(w, Box::new(Value::from_u64(ew as u64, 32))));
        let scaled = self.alloc_reg();
        self.emit(Insn::Mul(scaled, slot, w));
        if bit == 0 {
            return scaled;
        }
        let out = self.alloc_reg();
        self.emit(Insn::BinOpConst(
            out,
            scaled,
            Box::new(Value::from_u64(bit as u64, 32)),
            BinOpConstKind::Add,
        ));
        out
    }

    /// §11.5.1: the run-time base `base` (compiled from `left`) of a
    /// part-select of packed elements, widened to the 32 bits of the slot
    /// arithmetic it feeds. `Resize` sign-extends a signed register, so
    /// `bit signed [7:0] j = -1` stays label -1, as an `int` -1 is; added to
    /// 32-bit constants as 8 bits it read as 255.
    pub(super) fn widen_part_select_base(&mut self, left: &Expression, base: RegId) -> RegId {
        if self.lrm_self_width(left) >= 32 {
            return base;
        }
        let wide = self.alloc_reg();
        self.emit(Insn::MoveResize(wide, base, 32));
        wide
    }

    pub(super) fn compile_packed_member_read(
        &mut self,
        hier: &HierarchicalIdentifier,
    ) -> Option<RegId> {
        let (base_id, key, member, off, w) = self.packed_member_split(hier)?;
        let root = self.alloc_reg();
        self.emit(Insn::LoadSignal(root, as_sig_id(base_id)));
        let dest = self.alloc_reg();
        self.emit(Insn::RangeSelectConst(dest, root, off + w - 1, off));
        // §7.2.1: the member keeps its declared type's sign.
        if self.packed_member_signed(&key, &member) == Some(true) {
            self.emit(Insn::SetSigned(dest));
        }
        Some(dest)
    }

    /// §7.2, §7.2.1: the packed-structure variable a dotted identifier reads
    /// a member of, as `(signal id, layout key, member path, lsb, width)`.
    /// Elaboration lists a nested member (`s.inner.f`) under the ROOT
    /// variable as one dotted path (`inner.f`), so the split point is walked
    /// from the longest base down, as `packed_struct_member_target` does for
    /// a write: a base that is itself a signal (`top.dut.s`) wins over
    /// reading part of it as a member path. At each split the base is
    /// resolved scope-qualified first, then flat. A register block's
    /// `hwif_out.<reg>.<field>.value` read through more than one member used
    /// to go to the interpreter (`ident_lookup`).
    pub(super) fn packed_member_split(
        &self,
        hier: &HierarchicalIdentifier,
    ) -> Option<(usize, String, String, u32, u32)> {
        let fields_tbl = self.packed_struct_fields?;
        if hier.root.is_some() || hier.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let raw = Self::hier_raw_name(hier);
        let splits: Vec<usize> = raw.match_indices('.').map(|(i, _)| i).collect();
        for &dot in splits.iter().rev() {
            let (base, member) = (&raw[..dot], &raw[dot + 1..]);
            let scoped = self
                .scope_hint
                .as_ref()
                .map(|scope| format!("{}.{}", scope, base))
                .and_then(|q| Some((*self.signal_name_to_id.get(q.as_str())?, q)));
            let Some((base_id, key)) = scoped.or_else(|| {
                let id = *self.signal_name_to_id.get(base)?;
                Some((id, base.to_string()))
            }) else {
                continue;
            };
            let Some(layout) = fields_tbl.get(&key).or_else(|| fields_tbl.get(base)) else {
                continue;
            };
            if let Some(&(_, off, w)) = layout.iter().find(|(m, _, _)| m == member) {
                // A nested member is compiled only when its declared sign is
                // known (§7.2.1); otherwise it keeps the interpreter's path.
                if w == 0
                    || (member.contains('.') && self.packed_member_signed(&key, member).is_none())
                {
                    return None;
                }
                return Some((base_id, key, member.to_string(), off, w));
            }
        }
        None
    }

    /// §7.4.3: the width of one element of the outer packed dimension of the
    /// plain packed variable `base`, from its declared type and its signal
    /// width — 1 for a vector, 11 for `lane_t [-9:9] x`. None for any other
    /// shape (an unpacked array, a member, a local), which keeps its
    /// interpreted path.
    pub(super) fn packed_part_select_elem_width(&self, base: &Expression) -> Option<u32> {
        use crate::ast::types::{DataType as D, PackedDimension as P};
        let ExprKind::Ident(h) = &base.kind else {
            return None;
        };
        if h.root.is_some()
            || h.path.len() != 1
            || !h.path[0].selects.is_empty()
            || self.local_var_reg_of(h).is_some()
            || self.lookup_array_name(h).is_some()
            || self.is_multi_dim_array(h)
        {
            return None;
        }
        let total = *self.signal_widths.get(self.lookup_signal_id(h)?)?;
        let raw = Self::hier_raw_name(h);
        let scoped = self
            .scope_hint
            .as_ref()
            .map(|scope| format!("{}.{}", scope, raw));
        let (scoped, raw) = (scoped.as_deref(), raw.as_str());
        let declared = self
            .var_decl_types
            .and_then(|m| scoped.and_then(|q| m.get(q)).or_else(|| m.get(raw)));
        let (Some(mut dt), Some(types)) = (declared, self.typedef_types) else {
            // No declaration on record (a net, say): the packed shape tables
            // decide, and a name in neither is a plain vector.
            if let Some(&ew) = self.packed_elem_widths.and_then(|m| {
                scoped.and_then(|q| self.packed_shape_at(m, q)).or_else(|| self.packed_shape_at(m, raw))
            }) {
                return (ew > 0).then_some(ew);
            }
            if self
                .packed_full_dims
                .and_then(|m| {
                    scoped.and_then(|q| self.packed_shape_at(m, q)).or_else(|| self.packed_shape_at(m, raw))
                })
                .is_some_and(|dims| dims.len() > 1)
            {
                return None;
            }
            return Some(1);
        };
        // The outer dimension: the declaration's first packed range, through
        // plain typedef aliases; a declaration with none selects bits.
        let mut outer: Option<&P> = None;
        for _ in 0..64 {
            match dt {
                D::TypeReference {
                    name, dimensions, ..
                } => {
                    if let Some(first) = dimensions.first() {
                        outer = Some(first);
                        break;
                    }
                    dt = crate::compiler::elaborate::lookup_scoped_type(name, types)?;
                }
                D::IntegerVector { dimensions, .. } | D::Implicit { dimensions, .. } => {
                    outer = dimensions.first();
                    break;
                }
                D::IntegerAtom { .. } | D::Struct(_) | D::Enum(_) => break,
                _ => return None,
            }
        }
        let count = match outer {
            None => total,
            Some(P::Range { left, right, .. }) => {
                let (l, r) = (self.eval_const_bound(left)?, self.eval_const_bound(right)?);
                (l - r).unsigned_abs() as u32 + 1
            }
            Some(_) => return None,
        };
        (count > 0 && total % count == 0).then(|| total / count)
    }

    /// §7.4.1: whether `expr` (an index chain on a plain packed variable)
    /// selects an element whose type is a named signed type — see
    /// `signed_named_packed_elem_depth`.
    pub(super) fn packed_named_select_signed(&mut self, expr: &Expression) -> bool {
        let (Some(decls), Some(types)) = (self.var_decl_types, self.typedef_types) else {
            return false;
        };
        let mut root = expr;
        let mut selects = 0usize;
        while let ExprKind::Index { expr: base, .. } = &root.kind {
            selects += 1;
            root = base;
        }
        // A member of an indexed packed struct keeps the member declaration.
        // Its container index does not consume a dimension of the member.
        if let ExprKind::MemberAccess { expr: container, member } = &root.kind
            && let ExprKind::Index { expr: base, .. } = &container.kind
            && let ExprKind::Ident(h) = &base.kind
            && self.local_var_reg_of(h).is_none()
        {
            let raw = Self::hier_raw_name(h);
            let dt = self.scope_hint.as_ref()
                .and_then(|scope| decls.get(&format!("{scope}.{raw}")))
                .or_else(|| decls.get(&raw));
            if let Some(crate::ast::types::DataType::Struct(st)) = dt
                .map(|dt| crate::compiler::elaborate::resolve_typedef_chain(dt, types))
                && st.packed
                && let Some(field) = st.members.iter().find(|m|
                    m.declarators.iter().any(|d| d.name.name == member.name))
            {
                return signed_named_packed_elem_depth(&field.data_type, types) == Some(selects);
            }
        }
        let ExprKind::Ident(h) = &root.kind else {
            return false;
        };
        if h.root.is_some()
            || h.path.iter().any(|s| !s.selects.is_empty())
            || self.local_var_reg_of(h).is_some()
            || self.lookup_array_name(h).is_some()
            || self.is_multi_dim_array(h)
        {
            return false;
        }
        let raw = Self::hier_raw_name(h);
        if let Some(&depth) = self.named_elem_depth.get(&raw) {
            return depth == Some(selects);
        }
        let scoped = self
            .scope_hint
            .as_ref()
            .map(|scope| format!("{}.{}", scope, raw));
        let depth = scoped
            .as_deref()
            .and_then(|q| declared_type_of(q, decls, types))
            .or_else(|| declared_type_of(&raw, decls, types))
            .and_then(|dt| signed_named_packed_elem_depth(dt, types));
        self.named_elem_depth.insert(raw, depth);
        depth == Some(selects)
    }

    /// §6.11.2: can a range store into the packed container `var` (signal
    /// `id`) skip the 2-state conversion? A 2-state variable stores x/z as 0
    /// across its whole value on the interpreter's path, which a range store
    /// does not do, so the compiled store is taken only when neither the
    /// runtime's mark for the signal nor its declared type is 2-state. Without
    /// the runtime table the answer is no, which keeps the interpreter.
    pub(super) fn packed_container_four_state(&self, id: usize, var: &str) -> bool {
        let Some(marks) = self.signal_two_state else {
            return false;
        };
        if marks.get(id).copied().unwrap_or(true) {
            return false;
        }
        let (Some(decls), Some(types)) = (self.var_decl_types, self.typedef_types) else {
            return true;
        };
        let leaf = var.rsplit_once('.').map_or(var, |(_, leaf)| leaf);
        decls
            .get(var)
            .or_else(|| decls.get(leaf))
            .is_none_or(|dt| !crate::compiler::elaborate::is_type_two_state_resolved(dt, types))
    }

    /// §7.2.1, §6.11.3: the signedness of `member` (a dotted path) of the
    /// packed structure or union variable `var` holds, from its declared
    /// type. None when the declaration or the member cannot be resolved.
    pub(super) fn packed_member_signed(&self, var: &str, member: &str) -> Option<bool> {
        use crate::ast::types::DataType as D;
        let decls = self.var_decl_types?;
        let types = self.typedef_types?;
        let leaf_var = var.rsplit_once('.').map_or(var, |(_, leaf)| leaf);
        let mut dt = decls.get(var).or_else(|| decls.get(leaf_var))?;
        for segment in member.split('.') {
            let D::Struct(su) = crate::compiler::elaborate::resolve_typedef_chain(dt, types) else {
                return None;
            };
            if !su.packed {
                return None;
            }
            dt = &su
                .members
                .iter()
                .find(|m| m.declarators.iter().any(|d| d.name.name == segment))?
                .data_type;
        }
        let resolved = crate::compiler::elaborate::resolve_typedef_chain(dt, types);
        match resolved {
            D::IntegerAtom { .. } | D::IntegerVector { .. } | D::Enum(_) | D::Struct(_) => Some(
                crate::compiler::elaborate::is_type_signed_resolved(resolved, types),
            ),
            _ => None,
        }
    }

    /// The packed-structure variable and member path a dotted identifier
    /// names (`s.f`, `s.inner.f`), as `compile_packed_member_read` reads it.
    pub(super) fn packed_member_of_hier(
        &self,
        hier: &HierarchicalIdentifier,
    ) -> Option<(String, String)> {
        self.packed_member_split(hier)
            .map(|(_, key, member, _, _)| (key, member))
    }
}
