//! String operands, string methods and string formals.

use super::*;

impl<'a> BytecodeCompiler<'a> {
    /// Static-only heuristic: does this expression CLEARLY produce a string?
    /// Used to bail string-concat to the interpreter (which has byte-level
    /// concat semantics). We can only see syntactic clues at compile time —
    /// the full `string_signals` set lives on the simulator, not the
    /// bytecode compiler. A string-literal operand is always a string; a
    /// `$sformatf` / `$psprintf` call returns a string, as does a
    /// `string'(x)` cast (§6.24.3) and a function-LOCAL `string` variable
    /// (`out` in {out, string'(c)}). Other bare idents whose declared type
    /// we don't have here remain false — those cases get folded into the
    /// bit-vector concat path, which is the existing behavior. The
    /// interpreter side's special-case is what carries the LRM-correct
    /// path when the compiler can't see the type.
    pub(super) fn expr_is_string_concat_operand(&self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::StringLiteral(_) => true,
            ExprKind::Paren(inner) => self.expr_is_string_concat_operand(inner),
            ExprKind::Concatenation(parts) => {
                parts.iter().any(|p| self.expr_is_string_concat_operand(p))
            }
            ExprKind::SystemCall { name, args } => match name.as_str() {
                "$sformatf" | "$psprintf" => true,
                // §6.24.3: `string'(x)` compiles to a string value (ToStr),
                // so it is a string operand of an §11.4.12 concat.
                "$__xz_type_cast" | "$__xz_named_cast" => self.cast_arg_targets_string(args),
                _ => false,
            },
            ExprKind::Ident(h) => {
                if self.local_var_is_string.contains(&Self::hier_raw_name(h)) {
                    return true;
                }
                if let Some(set) = self.string_signals {
                    let last = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                    if set.contains(last) {
                        return true;
                    }
                    // Try scope-qualified form too.
                    if let Some(scope) = &self.scope_hint {
                        let q = format!("{}.{}", scope, last);
                        if set.contains(&q) {
                            return true;
                        }
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// Is this expression statically known to produce a STRING value?
    /// (Literal, `string` signal, `string`-bound formal, or a nested
    /// `$sformatf`.) Conservative: false means "unknown", not "packed".
    /// §5.9 / §6.16: literals acquire textual ordering only in a string context.
    pub(super) fn expr_has_string_type_static(&self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::StringLiteral(_) => false,
            ExprKind::Paren(inner) => self.expr_has_string_type_static(inner),
            ExprKind::Concatenation(parts) => {
                parts.iter().any(|e| self.expr_has_string_type_static(e))
            }
            _ => self.expr_is_string_static(e),
        }
    }

    pub(super) fn expr_is_string_static(&self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::StringLiteral(_) => true,
            // §7.10 / §6.16: an element of a string queue / dynamic array
            // (`q[0]` on `string q[$]`), local or instance member, is a
            // string.
            ExprKind::Index { expr: base, .. } => {
                if self.local_string_coll(base) || self.member_string_coll(base) {
                    return true;
                }
                // `obj.m_q[i]`: a referenced object's string-collection
                // element is a string too (§6.16/§7.10) — the member
                // collection may be on a class other than the method's own.
                self.foreign_string_coll_member(base).is_some()
            }
            ExprKind::Paren(inner) => self.expr_is_string_static(inner),
            ExprKind::SystemCall { name, args } => match name.as_str() {
                "$sformatf" | "$psprintf" => true,
                // §6.24.3: `string'(x)` compiles to a string value (ToStr).
                "$__xz_type_cast" | "$__xz_named_cast" => self.cast_arg_targets_string(args),
                _ => false,
            },
            ExprKind::MemberAccess { expr: base, member } if self.method_mode => {
                self.dotted_string_member(base, &member.name)
            }
            ExprKind::Ident(h) => {
                let raw = Self::hier_raw_name(h);
                if self.local_var_is_string.contains(&raw) {
                    return true;
                }
                // `d.name` spelled as one dotted identifier.
                if self.method_mode
                    && h.root.is_none()
                    && h.path.len() == 2
                    && h.path.iter().all(|seg| seg.selects.is_empty())
                    && self.dotted_string_member(&Self::dotted_head_expr(h), &h.path[1].name.name)
                {
                    return true;
                }
                // class-perf P2: a bare STRING MEMBER read is a string
                // operand in method mode (`{"top.", nm}`).
                if self.method_mode
                    && h.root.is_none()
                    && h.path.len() == 1
                    && h.path[0].selects.is_empty()
                    && !self.local_var_regs.contains_key(raw.as_str())
                    && self.string_member_names.contains(raw.as_str())
                {
                    return true;
                }
                let leaf = h.path.last().map(|p| p.name.name.as_str()).unwrap_or("");
                self.string_signals
                    .is_some_and(|ss| ss.contains(&raw) || ss.contains(leaf))
            }
            ExprKind::Concatenation(parts) => {
                !parts.is_empty() && parts.iter().all(|p| self.expr_is_string_static(p))
            }
            ExprKind::Call { func, .. } => {
                // class-perf P2: a CLASS-METHOD call whose method name
                // unanimously returns string (`obj.get_full_name()`,
                // bare `get_name()`, `this.get_name()`) is a string
                // operand. Name-global unanimity can never mistype a
                // non-string callee (a colliding non-string declaration
                // keeps the name out of the set and this arm declined).
                if self.method_mode {
                    let mname: Option<&str> = match &func.kind {
                        ExprKind::Ident(h) if h.path.len() >= 2 => {
                            h.path.last().map(|s| s.name.name.as_str())
                        }
                        ExprKind::Ident(h) if h.path.len() == 1 => {
                            Some(h.path[0].name.name.as_str())
                        }
                        ExprKind::MemberAccess { member, .. } => Some(member.name.as_str()),
                        _ => None,
                    };
                    if let Some(m) = mname
                        && self.method_string_ret_names.contains(m)
                    {
                        return true;
                    }
                }
                // `recv.substr(..)` on a member-access receiver.
                if let ExprKind::MemberAccess { expr: recv, member } = &func.kind {
                    return matches!(member.name.as_str(), "substr" | "toupper" | "tolower")
                        && self.expr_is_string_static(recv);
                }
                let ExprKind::Ident(h) = &func.kind else {
                    return false;
                };
                // String method with a string result on a string receiver.
                if h.path.len() >= 2 {
                    let m = h.path.last().unwrap().name.name.as_str();
                    if matches!(m, "substr" | "toupper" | "tolower") {
                        let mut recv = h.clone();
                        recv.path.pop();
                        let recv_expr = Expression::new(ExprKind::Ident(recv), func.span);
                        if self.expr_is_string_static(&recv_expr) {
                            return true;
                        }
                    }
                }
                let raw = Self::hier_raw_name(h);
                self.functions
                    .and_then(|f| {
                        f.get(&raw)
                            .or_else(|| raw.rsplit('.').next().and_then(|l| f.get(l)))
                    })
                    .is_some_and(|fd| {
                        matches!(
                            fd.return_type,
                            crate::ast::types::DataType::Simple {
                                kind: crate::ast::types::SimpleType::String,
                                ..
                            }
                        )
                    })
            }
            _ => false,
        }
    }

    /// Method mode: is `base.member` a string field of `base`'s declared
    /// class, read through an admitted handle chain?
    fn dotted_string_member(&self, base: &Expression, member: &str) -> bool {
        if !self.method_handle_chain_ok(base) || !self.method_instance_field_ok(base, member) {
            return false;
        }
        self.method_handle_chain_class(base).is_some_and(|class| {
            self.method_admission.as_ref().is_some_and(|admission| {
                admission
                    .instance_string_fields
                    .get(&class)
                    .is_some_and(|fields| fields.contains(member))
            })
        })
    }

    /// Method mode: is `base.member` a STRING-COLLECTION member (queue /
    /// dynamic / associative array of `string`) of `base`'s DECLARED class,
    /// read through an admitted handle chain? Mirrors `dotted_string_member`
    /// for collection members, so `obj.m_q[i]` (a referenced object's string
    /// collection element) is typed as a string by `string_cmp` instead of
    /// falling to the integral X-padded compare (§6.16/§7.10).
    fn dotted_string_coll(&self, base: &Expression, member: &str) -> bool {
        // The declared class's string-collection set already leaves out
        // statics and redeclared names; such a member is no scalar field
        // (`instance_fields`), so that set does not admit it.
        if !self.method_handle_chain_ok(base) {
            return false;
        }
        let class = self.method_handle_chain_class(base);
        if class.as_deref() == Some(self.method_class.as_deref().unwrap_or("")) {
            return false;
        }
        class.is_some_and(|class| {
            self.method_admission.as_ref().is_some_and(|admission| {
                admission
                    .instance_string_colls
                    .get(&class)
                    .is_some_and(|colls| colls.contains(member))
            })
        })
    }

    /// If `expr` is an admitted, select-free reference to a foreign
    /// object's string-collection member (`a1.q`, `this.holder.q`), return
    /// the member name; else None. Consumes the two AST spellings a dotted
    /// collection element (`a1.q[i]`) reaches: `MemberAccess { expr: a1,
    /// member: q }` and the dotted two-segment Ident `a1.q`.
    fn foreign_string_coll_member(&self, expr: &Expression) -> Option<String> {
        match &expr.kind {
            ExprKind::MemberAccess { expr: base, member } => self
                .dotted_string_coll(base, member.name.as_str())
                .then(|| member.name.clone()),
            ExprKind::Ident(h)
                if self.method_mode
                    && h.root.is_none()
                    && h.path.len() == 2
                    && h.path.iter().all(|seg| seg.selects.is_empty()) =>
            {
                let base = Self::dotted_head_expr(h);
                let member = h.path[1].name.name.as_str();
                self.dotted_string_coll(&base, member)
                    .then(|| h.path[1].name.name.to_string())
            }
            _ => None,
        }
    }

    /// `recv.method(args)` where the receiver is a statically-known STRING
    /// (dotted-Ident call shape). Returns the receiver as its own Ident
    /// expression plus the method leaf, or None when the shape/typing does
    /// not match.
    pub(super) fn string_method_shape<'e>(
        &self,
        func: &'e Expression,
        span: crate::ast::Span,
    ) -> Option<(Expression, &'e str)> {
        // Two syntactic forms reach here: a dotted ident `s.getc` (the
        // elaborator's normal shape) OR a bare `MemberAccess { expr: s,
        // member: getc }` (how the method-mode loop/expression compiler
        // sees some string-verb calls). Both mean the same thing; a
        // string-typed receiver with a supported verb is a pure StrOp.
        match &func.kind {
            ExprKind::Ident(h) => {
                if h.root.is_some() || h.path.len() < 2 {
                    return None;
                }
                if h.path.iter().any(|seg| !seg.selects.is_empty()) {
                    return None;
                }
                let method = h.path.last().unwrap().name.name.as_str();
                let mut recv = h.clone();
                recv.path.pop();
                let recv_expr = Expression::new(ExprKind::Ident(recv), span);
                if !self.expr_is_string_static(&recv_expr) {
                    return None;
                }
                Some((recv_expr, method))
            }
            ExprKind::MemberAccess { expr: recv, member } => {
                let recv_expr = (**recv).clone();
                // A string field read through a handle (`d.name`,
                // `p.q[i].name`) is a string receiver too.
                if let ExprKind::MemberAccess {
                    expr: base,
                    member: field,
                } = &recv_expr.kind
                {
                    if !self.dotted_string_member(base, &field.name) {
                        return None;
                    }
                    return Some((recv_expr, member.name.as_str()));
                }
                // Otherwise it must resolve as a bare string object (a
                // string signal/local/formal).
                let ExprKind::Ident(h) = &recv_expr.kind else {
                    return None;
                };
                if h.root.is_some()
                    || h.path.is_empty()
                    || h.path.iter().any(|seg| !seg.selects.is_empty())
                {
                    return None;
                }
                let method = member.name.as_str();
                if !self.expr_is_string_static(&recv_expr) {
                    return None;
                }
                Some((recv_expr, method))
            }
            _ => None,
        }
    }

    /// Compile a PURE string method call to a `StrOp`. Mutators (putc, the
    /// *toa family) are handled at statement level, not here.
    pub(super) fn compile_string_method(
        &mut self,
        func: &Expression,
        args: &[Expression],
        span: crate::ast::Span,
    ) -> Option<RegId> {
        let (recv, method) = self.string_method_shape(func, span)?;
        let (kind, nargs) = match method {
            "len" => (StrOpKind::Len, 0),
            "getc" => (StrOpKind::GetC, 1),
            "substr" => (StrOpKind::Substr, 2),
            "toupper" => (StrOpKind::ToUpper, 0),
            "tolower" => (StrOpKind::ToLower, 0),
            "compare" => (StrOpKind::Compare, 1),
            "icompare" => (StrOpKind::ICompare, 1),
            "atoi" => (StrOpKind::AToI, 0),
            "atohex" => (StrOpKind::AToHex, 0),
            "atooct" => (StrOpKind::AToOct, 0),
            "atobin" => (StrOpKind::AToBin, 0),
            _ => return None,
        };
        if args.len() != nargs {
            return None;
        }
        let start = self.insns.len();
        let start_reg = self.next_reg;
        let mut regs: Vec<RegId> = Vec::with_capacity(1 + nargs);
        let Some(r) = self.compile_expr(&recv, 0) else {
            return None;
        };
        regs.push(r);
        for a in args {
            match self.compile_expr(a, 0) {
                Some(r) => regs.push(r),
                None => {
                    self.truncate_insns(start);
                    self.next_reg = start_reg;
                    return None;
                }
            }
        }
        let dst = self.alloc_reg();
        self.emit(Insn::StrOp(dst, kind, Box::new(regs)));
        Some(dst)
    }

    pub(super) fn signal_is_string_name(&self, hier: &HierarchicalIdentifier) -> bool {
        let raw = Self::hier_raw_name(hier);
        let leaf = hier.path.last().map(|p| p.name.name.as_str()).unwrap_or("");
        self.string_signals
            .is_some_and(|ss| ss.contains(&raw) || ss.contains(leaf))
    }

    /// Parse a `$sformatf` template into segments, or None when any piece
    /// falls outside the natively-supported subset (specs d/b/h/x/o/s/c and
    /// `%%`; optional `-` flag and numeric width; no `+`, no precision, no
    /// t/p/m/u/z/e/f/g/v). The consumed-argument COUNT must be settled here
    /// so the compile arm can match it against the actual list.
    pub(super) fn parse_format_template(fmt: &str) -> Option<(Vec<FmtSeg>, usize)> {
        let mut segs: Vec<FmtSeg> = Vec::new();
        let mut lit = String::new();
        let mut nargs = 0usize;
        let mut chars = fmt.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '%' {
                lit.push(c);
                continue;
            }
            let mut left = false;
            if chars.peek() == Some(&'-') {
                left = true;
                chars.next();
            }
            if chars.peek() == Some(&'+') {
                return None;
            }
            let mut wstr = String::new();
            while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                wstr.push(chars.next().unwrap());
            }
            if chars.peek() == Some(&'.') {
                return None;
            }
            let spec = chars.next()?;
            if spec == '%' {
                if left || !wstr.is_empty() {
                    return None;
                }
                lit.push('%');
                continue;
            }
            let spec_lc = spec.to_ascii_lowercase();
            if !matches!(spec_lc, 'd' | 'b' | 'h' | 'x' | 'o' | 's' | 'c') {
                return None;
            }
            let width = if wstr.is_empty() {
                None
            } else {
                Some(wstr.parse::<u32>().ok()?)
            };
            if !lit.is_empty() {
                segs.push(FmtSeg::Lit(std::mem::take(&mut lit)));
            }
            segs.push(FmtSeg::Spec {
                spec: spec_lc,
                width,
                left,
                str_valued: false,
            });
            nargs += 1;
        }
        if !lit.is_empty() {
            segs.push(FmtSeg::Lit(lit));
        }
        Some((segs, nargs))
    }

    /// Step 8 (string formals): true if the expression mentions a
    /// STRING-typed formal anywhere. Conservative by design — any Ident
    /// node whose name matches bails the enclosing lowering, so a field
    /// access whose member name collides with a string formal just keeps
    /// the method on the AST interpreter (all-or-nothing, safe).
    /// Step 9b-ii: is `e` EXACTLY a whole string-formal Ident (`aa[name]`)?
    /// Such an index is a plain register read whose seeded byte-vector Value
    /// is byte-identical to the interpreter's local-frame read, so the
    /// member-collection element paths admit it where the general
    /// string-formal guards bail (see the `string_formal_index` relaxation).
    pub(super) fn index_is_whole_string_formal(&self, e: &Expression) -> bool {
        matches!(
            &e.kind,
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.len() == 1
                    && h.path[0].selects.is_empty()
                    && self.string_formal_names.contains(h.path[0].name.name.as_str())
        )
    }

    /// class-perf P2: is this operand string-UNSAFE at the point where an
    /// operator coerces to 1-bit (`&&`/`||`/`!`)? A string-touching operand
    /// that is ITSELF an equality compare (or `!` of one, or a `&&`/`||`
    /// of such) already reduced to 1 bit through the string-safe compare
    /// path — `parent==null && name=="__top__"` compiles. A string VALUE
    /// used directly (`s && x`) stays unsafe.
    pub(super) fn string_operand_unsafe(&self, e: &Expression) -> bool {
        use crate::ast::expr::ExprKind;
        match &e.kind {
            ExprKind::Paren(inner) => self.string_operand_unsafe(inner),
            ExprKind::Unary {
                op: crate::ast::expr::UnaryOp::LogNot,
                operand,
            } => self.string_operand_unsafe(operand),
            ExprKind::Binary { op, left, right }
                if matches!(
                    op,
                    BinaryOp::Eq | BinaryOp::Neq | BinaryOp::CaseEq | BinaryOp::CaseNeq
                ) =>
            {
                false
            }
            // Any other operator node (relational/arithmetic/logical,
            // `&&`/`||` above included) coerces its OPERANDS, not their
            // innards: recurse. A string-valued leaf still trips the base
            // case below (`s < v`, `s1 + s2` stay bailed).
            ExprKind::Binary { left, right, .. } => {
                self.string_operand_unsafe(left) || self.string_operand_unsafe(right)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.string_operand_unsafe(condition)
                    || self.string_operand_unsafe(then_expr)
                    || self.string_operand_unsafe(else_expr)
            }
            ExprKind::Concatenation(parts) => parts.iter().any(|p| self.string_operand_unsafe(p)),
            // A CALL is only string-unsafe through its RESULT type — a
            // string FORMAL as an argument flows by value into the callee
            // and the operator sees the (integral) return
            // (`get_report_verbosity_level(sev, id) < verbosity`). A
            // string-returning callee (`get_name()`) in a non-eq operator
            // stays unsafe.
            ExprKind::Call { .. } | ExprKind::SystemCall { .. } => self.expr_is_string_static(e),
            _ => self.touches_string_formal(e),
        }
    }

    /// Is every string-formal use in the integral expression `e` a length
    /// query (`s[s.len()-1]`)? The length is an `int`, so such an index
    /// needs none of the string-to-integral coercion a formal's text would.
    pub(super) fn string_formal_len_only(&self, e: &Expression) -> bool {
        use crate::ast::expr::ExprKind;
        match &e.kind {
            ExprKind::Call { func, args } if args.is_empty() => match &func.kind {
                ExprKind::MemberAccess { expr, member } if member.name == "len" => {
                    matches!(&expr.kind, ExprKind::Ident(h)
                        if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty())
                        || !self.touches_string_formal(e)
                }
                _ => !self.touches_string_formal(e),
            },
            ExprKind::Binary { left, right, .. } => {
                self.string_formal_len_only(left) && self.string_formal_len_only(right)
            }
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                self.string_formal_len_only(operand)
            }
            _ => !self.touches_string_formal(e),
        }
    }

    pub(super) fn touches_string_formal(&self, e: &Expression) -> bool {
        if self.string_formal_names.is_empty() {
            return false;
        }
        use crate::ast::expr::ExprKind;
        match &e.kind {
            ExprKind::Ident(h) => {
                h.root.is_none()
                    && h.path.len() == 1
                    && h.path[0].selects.is_empty()
                    && self
                        .string_formal_names
                        .contains(h.path[0].name.name.as_str())
            }
            ExprKind::Paren(inner)
            | ExprKind::Unary { operand: inner, .. }
            | ExprKind::MemberAccess { expr: inner, .. }
            | ExprKind::Index { expr: inner, .. }
            | ExprKind::RangeSelect { expr: inner, .. } => self.touches_string_formal(inner),
            ExprKind::Binary { left, right, .. } => {
                self.touches_string_formal(left) || self.touches_string_formal(right)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.touches_string_formal(condition)
                    || self.touches_string_formal(then_expr)
                    || self.touches_string_formal(else_expr)
            }
            ExprKind::Concatenation(parts) => parts.iter().any(|p| self.touches_string_formal(p)),
            ExprKind::TypedPattern { pattern, .. } => self.touches_string_formal(pattern),
            ExprKind::AssignmentPattern(items) => items.iter().any(|p| match p {
                crate::ast::expr::AssignmentPatternItem::Ordered(v) => {
                    self.touches_string_formal(v)
                }
                crate::ast::expr::AssignmentPatternItem::Named(_, v) => {
                    self.touches_string_formal(v)
                }
                crate::ast::expr::AssignmentPatternItem::Typed(_, v) => {
                    self.touches_string_formal(v)
                }
                crate::ast::expr::AssignmentPatternItem::Default(v) => {
                    self.touches_string_formal(v)
                }
                crate::ast::expr::AssignmentPatternItem::Keyed(k, v) => {
                    self.touches_string_formal(k) || self.touches_string_formal(v)
                }
            }),
            ExprKind::Replication { count, exprs } => {
                self.touches_string_formal(count)
                    || exprs.iter().any(|p| self.touches_string_formal(p))
            }
            ExprKind::Call { func, args } => {
                self.touches_string_formal(func)
                    || args.iter().any(|a| self.touches_string_formal(a))
            }
            _ => false,
        }
    }
}
