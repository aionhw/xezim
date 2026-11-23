//! Assignment patterns.

use super::*;

impl<'a> BytecodeCompiler<'a> {
    /// §7.2.1: `base.member` where `base` resolves to a packed-struct SIGNAL
    /// and `member` is a field of its recorded layout — a constant bit slice
    /// of the container. The interpreter served every such read through the
    /// metadata tables at ~7us a piece; ibex's CSR and interrupt logic does
    /// ~8 of them per cycle (`mstatus_q.mpp`, `irqs_i.irq_fast`, ...), which
    /// made this the largest single fallback after the named-cast fix.
    /// Field layout of `lv` when it names a packed-struct SIGNAL directly.
    fn lvalue_struct_layout(&self, lv: &Expression) -> Option<(String, Vec<(String, u32, u32)>)> {
        let fields_tbl = self.packed_struct_fields?;
        // `arr[i] <= '{...}`: an ELEMENT of an array of packed structs carries
        // the element layout, which is keyed by the array name — same keying
        // the `arr[i].m` member store uses. Without this the pattern had no
        // layout and the whole statement fell to the AST path.
        let h = match &lv.kind {
            ExprKind::Ident(h) => h,
            ExprKind::Index { expr, .. } => match &expr.kind {
                ExprKind::Ident(h) => h,
                _ => return None,
            },
            _ => return None,
        };
        if h.root.is_some() || h.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let raw = Self::hier_raw_name(h);
        let named = self
            .scope_hint
            .as_ref()
            .map(|scope| cat_dot(scope, &raw))
            .filter(|q| fields_tbl.contains_key(q))
            .unwrap_or(raw);
        let layout = fields_tbl.get(&named)?;
        // A whole packed ARRAY of structs is keyed the same way; its pattern
        // spans the elements, not one struct.
        let struct_w = layout.iter().map(|(_, o, w)| o + w).max().unwrap_or(0);
        if matches!(lv.kind, ExprKind::Ident(_))
            && let Some(&id) = self.signal_name_to_id.get(named.as_str())
            && self.signal_widths.get(id).is_some_and(|&sw| sw > struct_w)
        {
            return None;
        }
        Some((named, layout.to_vec()))
    }

    /// The declared type of the variable `lv` names, or of an element
    /// `arr[i]` of a one-dimensional unpacked array (which records its
    /// element type), when it is packed and spans the whole signal or
    /// element.
    fn declared_pattern_ctx(&mut self, lv: &Expression) -> Option<crate::ast::types::DataType> {
        use crate::compiler::packed_assignment_pattern_plan as pp;
        let mut cur = lv;
        let mut index_count = 0usize;
        loop {
            match &cur.kind {
                ExprKind::Paren(inner) => cur = inner,
                ExprKind::Index { expr, .. } => {
                    index_count += 1;
                    cur = expr;
                }
                _ => break,
            }
        }
        let ExprKind::Ident(h) = &cur.kind else {
            return None;
        };
        if h.root.is_some() || h.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let raw = Self::hier_raw_name(h);
        let scoped = self.scope_hint.as_deref().map(|sc| cat_dot(sc, &raw));
        let (ty, unpacked, w) = [scoped.as_deref(), Some(raw.as_ref())]
            .into_iter()
            .flatten()
            .find_map(|n| {
                // A port collapsed onto the net it connects takes that
                // net's declared type (`port_aliases`, formal to actual).
                let ty = pp::declared_type(n, self.var_decl_types?, self.port_aliases)?;
                let is_multi = self.multi_dim_arrays.is_some_and(|m| m.contains(n));
                if self.arrays.contains_key(n) {
                    if is_multi {
                        return None;
                    }
                    let w = self.arrays.get(n)?.2;
                    Some((ty.clone(), 1usize, w))
                } else {
                    if is_multi {
                        return None;
                    }
                    let id = *self.signal_name_to_id.get(n)?;
                    let w = *self.signal_widths.get(id)?;
                    Some((ty.clone(), 0usize, w))
                }
            })?;
        let packed_selects = index_count.checked_sub(unpacked)?;
        if pp::packed_width(&ty, &mut BcPatternTypeResolver(self)) != Some(w) {
            return None;
        }
        pp::peel_packed_dims(&ty, packed_selects, &mut BcPatternTypeResolver(self))
    }

    /// §10.8: the type of an assignment target, for the pattern `rvalue`
    /// assigned to it: a registered packed struct (or member of one), or a
    /// vector with recorded packed dimensions.
    pub(super) fn lvalue_pattern_ctx(
        &mut self,
        lv: &Expression,
        rvalue: &Expression,
    ) -> Option<crate::ast::types::DataType> {
        use crate::compiler::packed_assignment_pattern_plan as pp;
        if !pp::has_pattern(rvalue) {
            return None;
        }
        if let Some(ty) = self.declared_pattern_ctx(lv) {
            return Some(ty);
        }
        let full_dims = self.packed_full_dims;
        let member_dims = |root: &str, path: &str| {
            full_dims.and_then(|t| t.get(&format!("{root}.{path}")).map(|d| d.to_vec()))
        };
        if let Some((root, layout)) = self.lvalue_struct_layout(lv) {
            return pp::layout_struct_type(&layout, "", &|p| member_dims(&root, p));
        }
        let ExprKind::Ident(h) = &lv.kind else {
            return None;
        };
        if h.root.is_some() || h.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let raw = Self::hier_raw_name(h);
        let scoped = self.scope_hint.as_deref().map(|sc| cat_dot(sc, &raw));
        let names: Vec<&str> = scoped
            .iter()
            .map(String::as_str)
            .chain([raw.as_ref()])
            .collect();
        // A member of a declared variable takes its declared member type.
        if let Some(types) = self.var_decl_types {
            for name in &names {
                let mut split = name.len();
                while let Some(dot) = name[..split].rfind('.') {
                    split = dot;
                    if let Some(dt) = pp::declared_type(&name[..split], types, self.port_aliases) {
                        return pp::member_path_type(
                            dt,
                            &name[split + 1..],
                            &mut BcPatternTypeResolver(self),
                        );
                    }
                }
            }
        }
        if let Some(fields) = self.packed_struct_fields {
            for name in &names {
                let mut split = name.len();
                while let Some(dot) = name[..split].rfind('.') {
                    split = dot;
                    if let Some(layout) = fields.get(&name[..split]) {
                        let root = &name[..split];
                        return pp::layout_struct_type(layout, &name[split + 1..], &|p| {
                            member_dims(root, p)
                        });
                    }
                }
            }
        }
        let dims = self.packed_full_dims?;
        names
            .iter()
            .find_map(|n| dims.get(*n))
            .filter(|d| !d.is_empty())
            .map(|d| pp::packed_dims_vector(crate::ast::types::IntegerVectorType::Logic, d))
    }

    /// §10.9: compile a typed pattern, or an untyped one (possibly under a
    /// parenthesis or conditional) in the context type `ctx`, from its
    /// layout plan. A typed pattern with no plan falls back to the
    /// interpreter for that expression alone.
    pub(super) fn compile_pattern(
        &mut self,
        expr: &Expression,
        ctx_width: u32,
        ctx: Option<crate::ast::types::DataType>,
    ) -> Option<RegId> {
        use crate::compiler::packed_assignment_pattern_plan as pp;
        let (src, ty) = match (&expr.kind, ctx) {
            (ExprKind::TypedPattern { ty, pattern }, _) => (pattern.as_ref(), (**ty).clone()),
            (_, Some(ty)) => (expr, ty),
            _ => return None,
        };
        let start = self.insns.len();
        let start_reg = self.next_reg;
        // An element no item covers is an error (§10.9.1, §10.9.2): leave the
        // pattern to the interpreter, which warns.
        let plan =
            pp::packed_plan(src, &ty, &mut BcPatternTypeResolver(self)).filter(|p| !p.uncovered);
        if let Some(r) = plan.and_then(|p| self.compile_pattern_plan(&p)) {
            return Some(r);
        }
        self.truncate_insns(start);
        self.next_reg = start_reg;
        // A packed typed pattern with no plan evaluates in the interpreter;
        // an unpacked one has no packed value, so its statement stays there.
        if matches!(expr.kind, ExprKind::TypedPattern { .. })
            && pp::is_packed(&ty, &mut BcPatternTypeResolver(self))
        {
            return self.emit_expr_fallback(expr, ctx_width, "typed_pattern");
        }
        self.bail("Expr_AssignmentPattern");
        None
    }

    /// Emit a pattern plan: each slice compiled at its own width (§10.9.2),
    /// adjacent constant slices folded, then one Concat, most significant
    /// slice first.
    fn compile_pattern_plan(
        &mut self,
        plan: &crate::compiler::packed_assignment_pattern_plan::PackedPlan<'_>,
    ) -> Option<RegId> {
        use crate::compiler::packed_assignment_pattern_plan::SliceSrc;
        if !plan.disjoint() {
            return None;
        }
        let mut slices: Vec<_> = plan.slices.iter().collect();
        slices.sort_by(|a, b| b.offset.cmp(&a.offset));
        let mut regs: Vec<RegId> = Vec::with_capacity(slices.len());
        let mut pending: Option<Value> = None;
        let flush = |c: &mut Self, pending: &mut Option<Value>, regs: &mut Vec<RegId>| {
            if let Some(v) = pending.take() {
                let r = c.alloc_reg();
                c.emit(Insn::LoadConst(r, Box::new(v)));
                regs.push(r);
            }
        };
        for s in slices {
            let r = match &s.src {
                SliceSrc::Const(v) => {
                    pending = Some(match pending.take() {
                        Some(p) => p.concat_with(v),
                        None => v.clone(),
                    });
                    continue;
                }
                SliceSrc::Expr(e) => {
                    flush(self, &mut pending, &mut regs);
                    let src = self.compile_expr(e, s.width)?;
                    let r = self.alloc_reg();
                    self.emit(Insn::Move(r, src));
                    self.emit(Insn::Resize(r, s.width));
                    if s.two_state {
                        self.emit(Insn::DropXz(r));
                    }
                    r
                }
                SliceSrc::Repeat { expr, elem, count } => {
                    flush(self, &mut pending, &mut regs);
                    let src = self.compile_expr(expr, *elem)?;
                    let r = self.alloc_reg();
                    self.emit(Insn::Move(r, src));
                    self.emit(Insn::Resize(r, *elem));
                    if s.two_state {
                        self.emit(Insn::DropXz(r));
                    }
                    let dest = self.alloc_reg();
                    self.emit(Insn::Replicate(dest, r, *count));
                    dest
                }
                SliceSrc::Cond { cond, then, els } => {
                    // Both operands run (Select merges them on an x/z
                    // condition), so neither may have an effect.
                    if Self::plan_has_effect(then) || Self::plan_has_effect(els) {
                        return None;
                    }
                    flush(self, &mut pending, &mut regs);
                    let c = self.compile_expr(cond, 0)?;
                    let t = self.compile_pattern_plan(then)?;
                    let e = self.compile_pattern_plan(els)?;
                    let r = self.alloc_reg();
                    self.emit(Insn::Select(r, c, t, e));
                    r
                }
            };
            regs.push(r);
        }
        flush(self, &mut pending, &mut regs);
        let dest = self.alloc_reg();
        self.emit(Insn::Concat(dest, Box::new(regs)));
        if plan.signed {
            self.emit(Insn::SetSigned(dest));
        } else {
            self.emit(Insn::ClearSigned(dest));
        }
        Some(dest)
    }

    fn plan_has_effect(
        plan: &crate::compiler::packed_assignment_pattern_plan::PackedPlan<'_>,
    ) -> bool {
        use crate::compiler::packed_assignment_pattern_plan::SliceSrc;
        plan.slices.iter().any(|s| match &s.src {
            SliceSrc::Expr(e) | SliceSrc::Repeat { expr: e, .. } => Self::expr_has_effect(e),
            SliceSrc::Const(_) => false,
            SliceSrc::Cond { cond, then, els } => {
                Self::expr_has_effect(cond)
                    || Self::plan_has_effect(then)
                    || Self::plan_has_effect(els)
            }
        })
    }
}
