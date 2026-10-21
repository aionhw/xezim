//! Simulator assertions.

use super::*;

impl Simulator {
    /// LRM §16.5: edge-detect each registered SVA clock, then for each
    /// fired clock site:
    ///   1. Drain any deferred consequents whose cycle counter reaches 0.
    ///   2. Evaluate the body's antecedent (or whole body if not a
    ///      `|->`/`|=>`/`##N` shape).
    ///   3. If `|=>`, push consequent with cycles_remaining=1.
    ///      If `lhs ##N rhs` (Binary{HashHash, …}), push rhs with N.
    ///      If `|->`, tally `!lhs || rhs` immediately.
    /// LRM §16.5 — run a concurrent assertion's pass/fail action block at a
    /// clock fire (`assert property (...) <pass> else <fail>;`). Cloned out of
    /// the site first so the exec holds no borrow on `sva_sites`. A vacuous pass
    /// must NOT run the pass action, so callers pass `passed=false`/skip there.
    /// §16.8: expand named sequence / property instances (no formals)
    /// nested anywhere in an SVA body, so `a |=> s` sees `s`'s body.
    /// A named sequence/property body without its `@(clk)` wrapper.
    pub(super) fn sva_unclocked(e: &Expression) -> Expression {
        match &e.kind {
            ExprKind::SvaClocked { body, .. } => (**body).clone(),
            _ => e.clone(),
        }
    }

    /// §16.5.1: the operands of a named property or sequence are sampled like
    /// those of an assertion written inline. The parser spells `u.st` as a
    /// member access, which the elaborator folds into a hierarchical name in
    /// the top-level process bodies and in every declaration of an inlined
    /// instance, but not in a top-level or package declaration. Unfolded, a
    /// reference rooted at the top module (`t.u.st`) is not among the signals
    /// a concurrent assertion samples, so it read the value the clock edge's
    /// nonblocking assignments had just written, and a hierarchical clock
    /// (`@(posedge u.clk)`) named no signal, so the property never ticked.
    /// Fold those bodies the same way. A body without a member
    /// access is left alone: rebuilding it changes nothing but where it lives
    /// in memory, which cost 0.6% more instructions on a design with 288
    /// named properties.
    pub(super) fn fold_named_sva_members(module: &mut ElaboratedModule) {
        if !module.property_decls.values().any(Self::has_member_access) {
            return;
        }
        // §3.12.1: keep `$unit::x` where the scope shadows `x`, as the
        // elaborator's pass over the process bodies does.
        let unit: std::collections::HashSet<String> = module
            .signals
            .keys()
            .filter(|k| k.starts_with("$unit::"))
            .cloned()
            .collect();
        for body in module.property_decls.values_mut() {
            if Self::has_member_access(body) {
                *body = super::elaborate::rewrite_expr(
                    body,
                    "",
                    &HashMap::default(),
                    &unit,
                    &HashMap::default(),
                );
            }
        }
    }

    /// Does `e` hold a member access (`u.st`), clocked bodies included?
    pub(super) fn has_member_access(e: &Expression) -> bool {
        let mut hit = false;
        Self::walk_operands(e, &mut |x| match &x.kind {
            ExprKind::MemberAccess { .. } => hit = true,
            ExprKind::SvaClocked { events, body } => {
                hit |= Self::has_member_access(body)
                    || events.iter().any(|ev| {
                        Self::has_member_access(&ev.expr)
                            || ev.iff.as_ref().is_some_and(Self::has_member_access)
                    });
            }
            ExprKind::NamedArg { expr, .. } => {
                hit |= expr.as_deref().is_some_and(Self::has_member_access)
            }
            _ => {}
        });
        hit
    }

    /// §16.5.1: a reference into a generate block or an instance-array
    /// element (`blk[1].sig`, `blk[1].mem[2]`) parses as a member access over
    /// a select, which the elaborator leaves alone even in a process body.
    /// Unfolded, the assertion neither sampled it nor kept a `$past` history
    /// of it. Fold each one with constant selects that names a signal or an
    /// unpacked array into the flat name the scope's own code uses
    /// (`blk[1].sig`).
    pub(super) fn sva_fold_scopes(&self, e: &mut Expression) {
        if matches!(e.kind, ExprKind::MemberAccess { .. }) {
            if let Some(name) = self.sva_scope_name(e) {
                let span = e.span;
                let seg = crate::ast::expr::HierPathSegment {
                    name: crate::ast::Identifier { name, span },
                    selects: Vec::new(),
                };
                *e = Self::hier_ident_expr(vec![seg], span);
                return;
            }
        }
        let mut sub = |x: &mut Expression| self.sva_fold_scopes(x);
        match &mut e.kind {
            ExprKind::Unary { operand: x, .. }
            | ExprKind::Paren(x)
            | ExprKind::MemberAccess { expr: x, .. }
            | ExprKind::SvaClocked { body: x, .. } => sub(x),
            ExprKind::Binary { left, right, .. }
            | ExprKind::Index {
                expr: left,
                index: right,
            }
            | ExprKind::Range(left, right) => {
                sub(left);
                sub(right);
            }
            ExprKind::Conditional {
                condition: a,
                then_expr: b,
                else_expr: c,
            }
            | ExprKind::RangeSelect {
                expr: a,
                left: b,
                right: c,
                ..
            } => {
                sub(a);
                sub(b);
                sub(c);
            }
            ExprKind::Concatenation(xs)
            | ExprKind::SystemCall { args: xs, .. }
            | ExprKind::Call { args: xs, .. } => xs.iter_mut().for_each(&mut sub),
            ExprKind::Replication {
                count: x,
                exprs: xs,
            }
            | ExprKind::Inside {
                expr: x,
                ranges: xs,
            } => {
                sub(x);
                xs.iter_mut().for_each(&mut sub);
            }
            _ => {}
        }
    }

    /// The storage `blk[1].sig` names (see `sva_fold_scopes`): `None` unless
    /// the chain has a select, every select is a constant (a generate-block
    /// or instance-array index fixed at elaboration), and it names a signal
    /// or an unpacked array. A variable index (`a[i].f`) is evaluated at
    /// every tick, so folding it would freeze the element it picked.
    pub(super) fn sva_scope_name(&self, e: &Expression) -> Option<String> {
        let chain = Self::member_chain_as_hier_ident(e)?;
        let ExprKind::Ident(h) = &chain.kind else {
            return None;
        };
        let mut sels = h.path.iter().flat_map(|s| &s.selects).peekable();
        if sels.peek().is_none()
            || !sels.all(|x| super::elaborate::const_eval_i64_with_params(x, None).is_some())
        {
            return None;
        }
        let flat = self.member_chain_ident(e)?;
        let ExprKind::Ident(h) = &flat.kind else {
            return None;
        };
        let path = h
            .path
            .iter()
            .map(|s| s.name.name.as_str())
            .collect::<Vec<_>>()
            .join(".");
        // §23.8: the name is looked up in the assertion's scope, then in each
        // enclosing one (`cg[1].y` in instance `c` is `c.cg[1].y`).
        let scope = self.active_instance_scope();
        let mut up = scope.as_str();
        loop {
            let name = match up {
                "" => path.clone(),
                _ => format!("{up}.{path}"),
            };
            if self.signal_name_to_id.contains_key(name.as_str())
                || self.array_first_id.contains_key(name.as_str())
                || self.module.arrays.contains_key(&name)
                || self.module.arrays_2d.contains_key(&name)
                || self.module.arrays_nd.contains_key(&name)
            {
                return Some(name);
            }
            if up.is_empty() {
                return None;
            }
            up = up.rsplit_once('.').map_or("", |(p, _)| p);
        }
    }

    pub(super) fn sva_expand_named(&self, e: &Expression, depth: u32) -> Expression {
        if depth > 16 {
            return e.clone();
        }
        match &e.kind {
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                let name = h.path[0].name.name.as_str();
                if self.module.sequences.contains(name)
                    && self
                        .module
                        .property_params
                        .get(name)
                        .map_or(true, |p| p.is_empty())
                {
                    if let Some(body) = self.module.property_decls.get(name) {
                        let inner = Self::sva_unclocked(body);
                        return self.sva_expand_named(&inner, depth + 1);
                    }
                }
                e.clone()
            }
            // `s(x, y)`: a named sequence / property with formals — the
            // body with the actuals substituted.
            ExprKind::Call { func, args } => {
                let name = match &func.kind {
                    ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                        Some(h.path[0].name.name.clone())
                    }
                    _ => None,
                };
                if let Some(name) = name {
                    if let Some(body) = self.module.property_decls.get(&name).cloned() {
                        let formals = self
                            .module
                            .property_params
                            .get(&name)
                            .cloned()
                            .unwrap_or_default();
                        let mut map: HashMap<String, Expression> = HashMap::default();
                        for (i, f) in formals.iter().enumerate() {
                            if let Some(actual) = args.get(i) {
                                map.insert(f.clone(), actual.clone());
                            }
                        }
                        let body = super::elaborate::rewrite_expr(
                            &body,
                            "",
                            &map,
                            &std::collections::HashSet::new(),
                            &HashMap::default(),
                        );
                        let inner = Self::sva_unclocked(&body);
                        return self.sva_expand_named(&inner, depth + 1);
                    }
                }
                Expression::new(
                    ExprKind::Call {
                        func: func.clone(),
                        args: args
                            .iter()
                            .map(|a| self.sva_expand_named(a, depth))
                            .collect(),
                    },
                    e.span,
                )
            }
            ExprKind::SystemCall { name, args } => Expression::new(
                ExprKind::SystemCall {
                    name: name.clone(),
                    args: args
                        .iter()
                        .map(|a| self.sva_expand_named(a, depth))
                        .collect(),
                },
                e.span,
            ),
            ExprKind::Binary { op, left, right } => Expression::new(
                ExprKind::Binary {
                    op: op.clone(),
                    left: Box::new(self.sva_expand_named(left, depth)),
                    right: Box::new(self.sva_expand_named(right, depth)),
                },
                e.span,
            ),
            ExprKind::Unary { op, operand } => Expression::new(
                ExprKind::Unary {
                    op: op.clone(),
                    operand: Box::new(self.sva_expand_named(operand, depth)),
                },
                e.span,
            ),
            ExprKind::Paren(inner) => Expression::new(
                ExprKind::Paren(Box::new(self.sva_expand_named(inner, depth))),
                e.span,
            ),
            _ => e.clone(),
        }
    }

    /// §16.9.2 delay bounds of a `##` count: `N`, `[m:n]`, `[m:$]`.
    pub(super) fn sva_delay_bounds(&mut self, d: &Expression) -> (u64, u64) {
        match &d.kind {
            ExprKind::Range(lo, hi) => self.sva_bounds_pair(lo, hi),
            ExprKind::Dollar => (0, u64::MAX),
            ExprKind::Paren(inner) => self.sva_delay_bounds(inner),
            _ => {
                let n = self.eval_expr(d).to_u64().unwrap_or(1);
                (n, n)
            }
        }
    }

    pub(super) fn sva_bounds_pair(&mut self, lo: &Expression, hi: &Expression) -> (u64, u64) {
        let l = self.eval_expr(lo).to_u64().unwrap_or(0);
        let h = if matches!(hi.kind, ExprKind::Dollar) {
            u64::MAX
        } else {
            self.eval_expr(hi).to_u64().unwrap_or(l)
        };
        (l, h.max(l))
    }

    pub(super) fn sva_concat(a: SvaSeq, b: SvaSeq) -> SvaSeq {
        let mut items = match a {
            SvaSeq::Concat(v) => v,
            other => vec![other],
        };
        match b {
            SvaSeq::Concat(v) => items.extend(v),
            other => items.push(other),
        }
        SvaSeq::Concat(items)
    }

    /// A sequence expression (§16.9) as an `SvaSeq`: `a ##1 b ##[2:3] c`
    /// (parsed `SeqAnd(SeqAnd(a, ##1 b), ##[2:3] c)`) becomes
    /// `Concat[a, Delay(1,1), b, Delay(2,3), c]`.
    pub(super) fn sva_seq_from_expr(&mut self, e: &Expression) -> SvaSeq {
        use crate::ast::expr::{BinaryOp, UnaryOp};
        match &e.kind {
            ExprKind::Paren(inner) => self.sva_seq_from_expr(inner),
            ExprKind::Binary { op, left, right } => match op {
                BinaryOp::SeqAnd => {
                    let l = self.sva_seq_from_expr(left);
                    let r = self.sva_seq_from_expr(right);
                    Self::sva_concat(l, r)
                }
                BinaryOp::HashHash => {
                    let (mn, mx) = self.sva_delay_bounds(left);
                    let r = self.sva_seq_from_expr(right);
                    Self::sva_concat(SvaSeq::Delay(mn, mx), r)
                }
                BinaryOp::SeqOr => SvaSeq::Or(
                    Box::new(self.sva_seq_from_expr(left)),
                    Box::new(self.sva_seq_from_expr(right)),
                ),
                BinaryOp::SvaAnd => SvaSeq::And(
                    Box::new(self.sva_seq_from_expr(left)),
                    Box::new(self.sva_seq_from_expr(right)),
                ),
                BinaryOp::Intersect => SvaSeq::Intersect(
                    Box::new(self.sva_seq_from_expr(left)),
                    Box::new(self.sva_seq_from_expr(right)),
                ),
                BinaryOp::Throughout => {
                    SvaSeq::Throughout((**left).clone(), Box::new(self.sva_seq_from_expr(right)))
                }
                BinaryOp::Within => SvaSeq::Within(
                    Box::new(self.sva_seq_from_expr(left)),
                    Box::new(self.sva_seq_from_expr(right)),
                ),
                _ => SvaSeq::Bool(e.clone()),
            },
            // A trailing bare `##N`: a delay with nothing to check.
            ExprKind::Unary { op, operand } if matches!(op, UnaryOp::HashHash) => {
                let (mn, mx) = self.sva_delay_bounds(operand);
                Self::sva_concat(
                    SvaSeq::Delay(mn, mx),
                    SvaSeq::Bool(SvaNfaBuilder::true_lit(e.span)),
                )
            }
            ExprKind::SystemCall { name, args } if name.starts_with("$sva_") => {
                let bounds = |this: &mut Self| -> (u64, u64) {
                    match (args.get(1), args.get(2)) {
                        (Some(lo), Some(hi)) => this.sva_bounds_pair(lo, hi),
                        _ => (1, 1),
                    }
                };
                match (name.as_str(), args.first()) {
                    ("$sva_rep_consec", Some(s)) => {
                        let (min, max) = bounds(self);
                        SvaSeq::Rep {
                            seq: Box::new(self.sva_seq_from_expr(s)),
                            min,
                            max,
                        }
                    }
                    ("$sva_rep_goto", Some(s)) => {
                        let (min, max) = bounds(self);
                        SvaSeq::Goto {
                            cond: s.clone(),
                            min,
                            max,
                        }
                    }
                    ("$sva_rep_noncon", Some(s)) => {
                        let (min, max) = bounds(self);
                        SvaSeq::NonCon {
                            cond: s.clone(),
                            min,
                            max,
                        }
                    }
                    ("$sva_first_match", Some(s)) => {
                        SvaSeq::FirstMatch(Box::new(self.sva_seq_from_expr(s)))
                    }
                    ("$sva_strong", Some(s)) | ("$sva_weak", Some(s)) => self.sva_seq_from_expr(s),
                    // §16.10 / §16.11 `(seq, item...)`: the items run when
                    // `seq` matches.
                    ("$sva_match", Some(s)) => {
                        let items = args[1..].iter().map(|a| self.sva_match_item(a)).collect();
                        Self::sva_concat(self.sva_seq_from_expr(s), SvaSeq::Assign(items))
                    }
                    // §16.10: a sequence's local variable initialisers run
                    // when its evaluation starts.
                    ("$sva_locals", Some(_)) => {
                        let (body, inits) = args.split_last().unwrap();
                        let items: Vec<SvaMatchItem> =
                            inits.iter().map(|a| self.sva_match_item(a)).collect();
                        Self::sva_concat(SvaSeq::Assign(items), self.sva_seq_from_expr(body))
                    }
                    _ => SvaSeq::Bool(e.clone()),
                }
            }
            _ => SvaSeq::Bool(e.clone()),
        }
    }

    pub(super) fn sva_build_seq(&mut self, e: &Expression) -> SvaNfa {
        let seq = self.sva_seq_from_expr(e);
        SvaNfaBuilder::build(&seq)
    }

    /// Does the expression use a sequence/property operator (so a `not` in
    /// front of it is property negation rather than a boolean `!`)?
    pub(super) fn sva_is_temporal(e: &Expression) -> bool {
        use crate::ast::expr::{BinaryOp, UnaryOp};
        match &e.kind {
            ExprKind::Paren(inner) => Self::sva_is_temporal(inner),
            ExprKind::Binary { op, left, right } => {
                matches!(
                    op,
                    BinaryOp::HashHash
                        | BinaryOp::SeqAnd
                        | BinaryOp::SvaAnd
                        | BinaryOp::SeqOr
                        | BinaryOp::Intersect
                        | BinaryOp::Throughout
                        | BinaryOp::Within
                        | BinaryOp::OrMinusArrow
                        | BinaryOp::OrFatArrow
                ) || Self::sva_is_temporal(left)
                    || Self::sva_is_temporal(right)
            }
            ExprKind::Unary { op, operand } => {
                matches!(
                    op,
                    UnaryOp::HashHash | UnaryOp::SEventually | UnaryOp::SAlways
                ) || Self::sva_is_temporal(operand)
            }
            ExprKind::SystemCall { name, .. } => name.starts_with("$sva_"),
            _ => false,
        }
    }

    /// A §16.10 / §16.11 match item: an assignment (`v = e`, `v += e`,
    /// `v++`) to a local variable slot bound by `sva_bind_locals`, or a
    /// subroutine call.
    pub(super) fn sva_match_item(&self, e: &Expression) -> SvaMatchItem {
        use crate::ast::expr::{BinaryOp, UnaryOp};
        let slot_of = |lv: &Expression| -> Option<(usize, u32, bool)> {
            let k = Self::sva_lv_slot(lv)?;
            let (w, signed, _) = self.sva_lv_types.get(k).copied()?;
            Some((k, w, signed))
        };
        let one = |span| SvaNfaBuilder::true_lit(span);
        match &e.kind {
            ExprKind::Paren(inner) => return self.sva_match_item(inner),
            ExprKind::AssignExpr { lvalue, rvalue }
            | ExprKind::Binary {
                op: BinaryOp::Assign,
                left: lvalue,
                right: rvalue,
            } => {
                if let Some(slot) = slot_of(lvalue) {
                    return SvaMatchItem {
                        slot: Some(slot),
                        expr: (**rvalue).clone(),
                    };
                }
            }
            ExprKind::Unary { op, operand }
                if matches!(
                    op,
                    UnaryOp::PostIncr | UnaryOp::PreIncr | UnaryOp::PostDecr | UnaryOp::PreDecr
                ) =>
            {
                if let Some(slot) = slot_of(operand) {
                    let bop = if matches!(op, UnaryOp::PostIncr | UnaryOp::PreIncr) {
                        BinaryOp::Add
                    } else {
                        BinaryOp::Sub
                    };
                    return SvaMatchItem {
                        slot: Some(slot),
                        expr: Expression::new(
                            ExprKind::Binary {
                                op: bop,
                                left: operand.clone(),
                                right: Box::new(one(e.span)),
                            },
                            e.span,
                        ),
                    };
                }
            }
            _ => {}
        }
        SvaMatchItem {
            slot: None,
            expr: e.clone(),
        }
    }

    /// The slot of a local variable read bound by `sva_bind_locals`.
    pub(super) fn sva_lv_slot(e: &Expression) -> Option<usize> {
        match &e.kind {
            ExprKind::Paren(inner) => Self::sva_lv_slot(inner),
            ExprKind::SystemCall { name, args } if name == SVA_SIG_MARKER && args.len() == 2 => {
                match &args[0].kind {
                    ExprKind::Number(NumberLiteral::Integer { value, .. }) => value.parse().ok(),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// §16.10: run `init` (a property's local variable initialisers) when
    /// each evaluation of `node` starts — ahead of its leading sequences.
    pub(super) fn sva_inject_init(node: &mut SvaNode, init: &[SvaMatchItem]) {
        let inject = |nfa: &mut SvaNfa| {
            nfa.states
                .push(vec![SvaEdge::Assign(init.to_vec(), nfa.start)]);
            nfa.start = nfa.states.len() - 1;
        };
        match node {
            SvaNode::Seq(nfa, _) => inject(nfa),
            SvaNode::Impl { ante, .. } | SvaNode::Followed { ante, .. } => inject(ante),
            SvaNode::Not(inner) => Self::sva_inject_init(inner, init),
            SvaNode::And(a, b) | SvaNode::Or(a, b) => {
                Self::sva_inject_init(a, init);
                Self::sva_inject_init(b, init);
            }
            // Not modelled: initialisers ahead of `s_eventually` /
            // `s_always` of a boolean.
            SvaNode::Eventually(_) | SvaNode::Always(_) => {}
        }
    }

    /// Compile a property body (§16.12) into an `SvaNode`.
    pub(super) fn sva_compile_node(&mut self, e: &Expression) -> SvaNode {
        use crate::ast::expr::{BinaryOp, UnaryOp};
        match &e.kind {
            ExprKind::Paren(inner) => self.sva_compile_node(inner),
            // §16.10: a property's local variables; their initialisers run
            // at the start of each evaluation.
            ExprKind::SystemCall { name, args } if name == "$sva_locals" && !args.is_empty() => {
                let (body, inits) = args.split_last().unwrap();
                let items: Vec<SvaMatchItem> =
                    inits.iter().map(|a| self.sva_match_item(a)).collect();
                let mut node = self.sva_compile_node(body);
                if !items.is_empty() {
                    Self::sva_inject_init(&mut node, &items);
                }
                node
            }
            ExprKind::SystemCall { name, args }
                if matches!(
                    name.as_str(),
                    "$sva_follow_overlap" | "$sva_follow_nonoverlap"
                ) && args.len() == 2 =>
            {
                SvaNode::Followed {
                    ante: self.sva_build_seq(&args[0]),
                    overlap: name == "$sva_follow_overlap",
                    cons: Box::new(self.sva_compile_node(&args[1])),
                }
            }
            ExprKind::Binary { op, left, right }
                if matches!(op, BinaryOp::OrMinusArrow | BinaryOp::OrFatArrow) =>
            {
                let ante = self.sva_build_seq(left);
                SvaNode::Impl {
                    ante,
                    overlap: matches!(op, BinaryOp::OrMinusArrow),
                    cons: Box::new(self.sva_compile_node(right)),
                }
            }
            // §16.12.8 `property iff (guard)` — vacuous pass when guard is false.
            ExprKind::Binary { op, left, right } if matches!(op, BinaryOp::Iff) =>
            {
                // `property iff guard` ≡ `guard |-> property` (overlapping implication)
                let ante = self.sva_build_seq(right);
                SvaNode::Impl {
                    ante,
                    overlap: true,
                    cons: Box::new(self.sva_compile_node(left)),
                }
            }
            ExprKind::Binary { op, left, right }
                if matches!(op, BinaryOp::SvaAnd)
                    && (Self::sva_has_property_op(left) || Self::sva_has_property_op(right)) =>
            {
                SvaNode::And(
                    Box::new(self.sva_compile_node(left)),
                    Box::new(self.sva_compile_node(right)),
                )
            }
            ExprKind::Binary { op, left, right }
                if matches!(op, BinaryOp::SeqOr)
                    && (Self::sva_has_property_op(left) || Self::sva_has_property_op(right)) =>
            {
                SvaNode::Or(
                    Box::new(self.sva_compile_node(left)),
                    Box::new(self.sva_compile_node(right)),
                )
            }
            ExprKind::Unary { op, operand }
                if matches!(op, UnaryOp::LogNot) && Self::sva_is_temporal(operand) =>
            {
                SvaNode::Not(Box::new(self.sva_compile_node(operand)))
            }
            ExprKind::Unary { op, operand } if matches!(op, UnaryOp::SEventually) => {
                SvaNode::Eventually((**operand).clone())
            }
            ExprKind::Unary { op, operand } if matches!(op, UnaryOp::SAlways) => {
                SvaNode::Always((**operand).clone())
            }
            // §16.12.2 `strong(seq)` / `weak(seq)` as a property.
            ExprKind::SystemCall { name, args }
                if matches!(name.as_str(), "$sva_strong" | "$sva_weak") && args.len() == 1 =>
            {
                let nfa = self.sva_build_seq(&args[0]);
                SvaNode::Seq(nfa, name == "$sva_strong")
            }
            _ => SvaNode::Seq(self.sva_build_seq(e), false),
        }
    }

    /// Whether one `and`/`or` operand fails at the end of simulation: a
    /// resolved Pass/Vacuous holds, a resolved Fail already failed, and a
    /// still-pending operand fails iff its own evaluation would fail once the
    /// clock stops.
    fn sva_op_fails(resolved: &Option<SvaOutcome>, node: &SvaNode, st: &SvaState) -> bool {
        match resolved {
            Some(SvaOutcome::Fail) => true,
            Some(_) => false, // Pass / Vacuous
            None => Self::sva_fails_at_end(node, st),
        }
    }

    /// Whether a pending property fails at the end of simulation (§16.12.2–3).
    /// A weak sequence holds when the clock stops; property `not` inverts
    /// that result just as it inverts a completed operand's result.
    pub(super) fn sva_fails_at_end(node: &SvaNode, st: &SvaState) -> bool {
        match (node, st) {
            (SvaNode::Seq(_, strong), SvaState::Seq(_)) => *strong,
            (SvaNode::Not(inner), SvaState::Not(ist)) => !Self::sva_fails_at_end(inner, ist),
            (SvaNode::Eventually(_), SvaState::Leaf(_)) => true,
            (SvaNode::Impl { cons: cn, .. }, SvaState::Impl { cons, .. })
            | (SvaNode::Followed { cons: cn, .. }, SvaState::Followed { cons, .. }) => {
                // A nonoverlapped consequent still waiting for its first
                // clock tick has not started an evaluation yet.
                cons.iter()
                    .any(|(delay, cs)| *delay == 0 && Self::sva_fails_at_end(cn, cs))
            }
            // §16.12.6: `p && q` (And) fails at end when EITHER operand fails;
            // `p or q` (Or) fails only when BOTH operands fail. An operand
            // already resolved to Pass/Vacuous can't fail; one already
            // resolved to Fail already failed; one still pending fails iff
            // its own evaluation would fail once the clock stops.
            (SvaNode::And(na, nb), SvaState::And { a, b, ra, rb }) => {
                Self::sva_op_fails(ra, na, a) || Self::sva_op_fails(rb, nb, b)
            }
            (SvaNode::Or(na, nb), SvaState::Or { a, b, ra, rb }) => {
                Self::sva_op_fails(ra, na, a) && Self::sva_op_fails(rb, nb, b)
            }
            _ => false,
        }
    }

    /// End of simulation (§16.12.2–3): settle pending failures before final
    /// procedures inspect the assertion actions' results.
    pub(super) fn sva_end_of_sim(&mut self) {
        for i in 0..self.sva_sites.len() {
            // Guards and fail actions may call functions after `$finish`:
            // lift the flag as final procedures do.
            let was_finished = self.finished;
            self.finished = false;
            let disabled = self.sva_site_disabled(i);
            let node = self.sva_sites[i].node.clone();
            let span_key = self.sva_sites[i].stat_key;
            let attempts = std::mem::take(&mut self.sva_sites[i].attempts);
            if disabled {
                self.finished = was_finished;
                continue;
            }
            let n_fail = attempts
                .iter()
                .filter(|st| Self::sva_fails_at_end(&node, st))
                .count();
            let prev_active = self.active_sva_site.replace(i);
            for _ in 0..n_fail {
                self.sva_tally(i, span_key, false);
            }
            self.finished = was_finished;
            self.active_sva_site = prev_active;
        }
    }

    /// §16.12: `disable iff` uses the current value in the assertion's
    /// scope, independently of its sampling clock.
    pub(super) fn sva_site_disabled(&mut self, i: usize) -> bool {
        let Some(guard) = self.sva_sites[i].disable.take() else {
            return false;
        };
        let prev_active = self.active_sva_site.replace(i);
        let prev_scope = self.activation_scope.borrow().clone();
        *self.activation_scope.borrow_mut() = Some(self.sva_sites[i].scope.clone());
        let disabled = self.eval_expr(&guard).is_true();
        self.sva_sites[i].disable = Some(guard);
        self.active_sva_site = prev_active;
        *self.activation_scope.borrow_mut() = prev_scope;
        disabled
    }

    pub(super) fn sva_cancel_disabled(&mut self, i: usize) -> bool {
        let disabled = self.sva_site_disabled(i);
        if disabled {
            self.sva_sites[i].attempts.clear();
            self.sva_sites[i].matured.clear();
        }
        disabled
    }

    /// Does the expression contain a property-only operator (implication,
    /// `not` of a sequence, `s_eventually`, ...)? Decides whether an
    /// `and`/`or` joins properties or sequences.
    pub(super) fn sva_has_property_op(e: &Expression) -> bool {
        use crate::ast::expr::{BinaryOp, UnaryOp};
        match &e.kind {
            ExprKind::Paren(inner) => Self::sva_has_property_op(inner),
            ExprKind::SystemCall { name, args } if name == "$sva_locals" => {
                args.last().is_some_and(Self::sva_has_property_op)
            }
            ExprKind::SystemCall { name, .. }
                if matches!(
                    name.as_str(),
                    "$sva_follow_overlap" | "$sva_follow_nonoverlap"
                ) =>
            {
                true
            }
            ExprKind::Binary { op, left, right } => {
                matches!(op, BinaryOp::OrMinusArrow | BinaryOp::OrFatArrow)
                    || (matches!(op, BinaryOp::SvaAnd | BinaryOp::SeqOr)
                        && (Self::sva_has_property_op(left) || Self::sva_has_property_op(right)))
            }
            ExprKind::Unary { op, operand } => {
                matches!(op, UnaryOp::SEventually | UnaryOp::SAlways)
                    || (matches!(op, UnaryOp::LogNot) && Self::sva_is_temporal(operand))
            }
            _ => false,
        }
    }

    /// Split a site body into its `disable iff` guard and compiled property.
    pub(super) fn sva_compile_site(&mut self, body: &Expression) -> (Option<Expression>, SvaNode) {
        let mut inner = body;
        while let ExprKind::Paren(p) = &inner.kind {
            inner = p;
        }
        if let ExprKind::Binary { op, left, right } = &inner.kind {
            if matches!(op, crate::ast::expr::BinaryOp::SvaDisableIff) {
                let node = self.sva_compile_node(right);
                return (Some((**left).clone()), node);
            }
        }
        (None, self.sva_compile_node(inner))
    }

    pub(super) fn sva_new_state(node: &SvaNode, env: &SvaEnv) -> SvaState {
        match node {
            SvaNode::Seq(..) => SvaState::Seq(SvaRun::with_env(env.clone())),
            SvaNode::Not(inner) => SvaState::Not(Box::new(Self::sva_new_state(inner, env))),
            SvaNode::Impl { .. } => SvaState::Impl {
                ante: SvaRun::with_env(env.clone()),
                ante_alive: true,
                matched_any: false,
                cons: Vec::new(),
            },
            SvaNode::Followed { .. } => SvaState::Followed {
                ante: SvaRun::with_env(env.clone()),
                ante_alive: true,
                cons: Vec::new(),
            },
            SvaNode::And(a, b) => SvaState::And {
                a: Box::new(Self::sva_new_state(a, env)),
                b: Box::new(Self::sva_new_state(b, env)),
                ra: None,
                rb: None,
            },
            SvaNode::Or(a, b) => SvaState::Or {
                a: Box::new(Self::sva_new_state(a, env)),
                b: Box::new(Self::sva_new_state(b, env)),
                ra: None,
                rb: None,
            },
            SvaNode::Eventually(_) | SvaNode::Always(_) => SvaState::Leaf(env.clone()),
        }
    }

    /// Evaluate `e` for a thread whose local variables (§16.10) are `env`:
    /// they are installed as `sva_lv` for the reads of their slot markers.
    pub(super) fn sva_eval_env(&mut self, e: &Expression, env: &mut SvaEnv) -> Value {
        if env.is_empty() {
            return self.eval_expr(e);
        }
        std::mem::swap(&mut self.sva_lv, env);
        let v = self.eval_expr(e);
        std::mem::swap(&mut self.sva_lv, env);
        v
    }

    /// Run match items on a copy of `env` (§16.10 assignments in order,
    /// §16.11 subroutine calls for their side effects).
    pub(super) fn sva_run_match_items(&mut self, items: &[SvaMatchItem], env: &SvaEnv) -> SvaEnv {
        let mut env = env.clone();
        for it in items {
            match it.slot {
                Some((k, w, signed)) => {
                    // §11.6.1: the right-hand side of an assignment is sized
                    // with its target, so `x = a + b` keeps the carry when
                    // `x` is wider than `a` and `b`.
                    std::mem::swap(&mut self.sva_lv, &mut env);
                    let v = self.eval_expr_ctx(&it.expr, w);
                    std::mem::swap(&mut self.sva_lv, &mut env);
                    let mut v = v.resize_for_assign(w);
                    v.is_signed = signed;
                    if let Some(slot) = env.get_mut(k) {
                        *slot = v;
                    }
                }
                None => {
                    let stmt = Statement::new(StatementKind::Expr(it.expr.clone()), it.expr.span);
                    std::mem::swap(&mut self.sva_lv, &mut env);
                    self.exec_statement(&stmt);
                    std::mem::swap(&mut self.sva_lv, &mut env);
                }
            }
        }
        env
    }

    /// §16.10: the local variables after both operands of an `and` /
    /// `intersect` matched: `a`'s, with every slot `b` assigned (changed from
    /// the values the compound was entered with) taken from `b`.
    pub(super) fn sva_merge_env(a: &SvaEnv, b: &SvaEnv, base: &SvaEnv) -> SvaEnv {
        let mut out = a.clone();
        for (k, v) in b.iter().enumerate() {
            if base.get(k) != Some(v) {
                if let Some(o) = out.get_mut(k) {
                    *o = v.clone();
                }
            }
        }
        out
    }

    /// Advance a sequence automaton by one clock tick (the first call is the
    /// start tick). Returns the local variables of each thread that matched
    /// at this tick (none: no match), and whether the run is still alive.
    /// With `first_match` the run stops at its first match.
    pub(super) fn sva_run_advance(
        &mut self,
        nfa: &SvaNfa,
        run: &mut SvaRun,
        first_match: bool,
    ) -> (Vec<SvaEnv>, bool) {
        let mut work: Vec<(usize, SvaEnv)> = Vec::new();
        let mut subs: Vec<SvaSubRun> = Vec::new();
        if !run.started {
            run.started = true;
            work.push((nfa.start, std::mem::take(&mut run.init_env)));
        } else {
            for (pc, env) in run.pcs.drain(..) {
                for e in &nfa.states[pc] {
                    if let SvaEdge::Tick(t) = e {
                        work.push((*t, env.clone()));
                    }
                }
            }
            for mut sub in run.subs.drain(..) {
                let (m, alive) = self.sva_sub_advance(nfa, &mut sub);
                for env in m {
                    work.push((sub.target, env));
                }
                if alive {
                    subs.push(sub);
                }
            }
        }
        // A state is visited once per distinct set of local values; a run
        // with no locals (every env empty) needs only the state.
        let mut visited = vec![false; nfa.states.len()];
        let mut seen: Vec<(usize, SvaEnv)> = Vec::new();
        let mut tick_pcs: Vec<(usize, SvaEnv)> = Vec::new();
        let mut matched: Vec<SvaEnv> = Vec::new();
        while let Some((pc, mut env)) = work.pop() {
            if env.is_empty() {
                if visited[pc] {
                    continue;
                }
                visited[pc] = true;
            } else {
                if seen.iter().any(|(p, e)| *p == pc && *e == env) {
                    continue;
                }
                seen.push((pc, env.clone()));
            }
            if pc == nfa.accept {
                if !matched.contains(&env) {
                    matched.push(env.clone());
                }
                if first_match {
                    break;
                }
            }
            for ei in 0..nfa.states[pc].len() {
                match &nfa.states[pc][ei] {
                    SvaEdge::Eps(t) => work.push((*t, env.clone())),
                    SvaEdge::Check(c, t) => {
                        if self.sva_eval_env(c, &mut env).is_true() {
                            work.push((*t, env.clone()));
                        }
                    }
                    SvaEdge::Tick(_) => {
                        if !tick_pcs.iter().any(|(p, e)| *p == pc && *e == env) {
                            tick_pcs.push((pc, env.clone()));
                        }
                    }
                    SvaEdge::Assign(items, t) => {
                        let items = items.clone();
                        let next = self.sva_run_match_items(&items, &env);
                        work.push((*t, next));
                    }
                    SvaEdge::Sub(comp, t) => {
                        let n = match &**comp {
                            SvaCompound::And(..) | SvaCompound::Intersect(..) => 2,
                            _ => 1,
                        };
                        let mut sub = SvaSubRun {
                            at: (pc, ei),
                            target: *t,
                            runs: vec![SvaRun::with_env(env.clone()); n],
                            ever: [false, false],
                            env: env.clone(),
                            ever_env: [Vec::new(), Vec::new()],
                            ticks: 0,
                            repeated: Vec::new(),
                        };
                        let (m, alive) = self.sva_sub_advance(nfa, &mut sub);
                        for menv in m {
                            work.push((*t, menv));
                        }
                        if alive {
                            subs.push(sub);
                        }
                    }
                }
            }
        }
        if !matched.is_empty() && first_match {
            run.pcs.clear();
            run.subs.clear();
            return (matched, false);
        }
        run.pcs = tick_pcs;
        run.subs = subs;
        let alive = !run.pcs.is_empty() || !run.subs.is_empty();
        (matched, alive)
    }

    /// Advance one compound edge: the local variables of each match at this
    /// tick, and whether it is still alive.
    pub(super) fn sva_sub_advance(
        &mut self,
        nfa: &SvaNfa,
        sub: &mut SvaSubRun,
    ) -> (Vec<SvaEnv>, bool) {
        let SvaEdge::Sub(comp, _) = &nfa.states[sub.at.0][sub.at.1] else {
            return (Vec::new(), false);
        };
        match &**comp {
            SvaCompound::Delay(min, max) => {
                let out = if sub.ticks >= *min {
                    vec![sub.env.clone()]
                } else {
                    Vec::new()
                };
                let alive = *max == u64::MAX || sub.ticks < *max;
                sub.ticks = sub.ticks.saturating_add(1);
                (out, alive)
            }
            SvaCompound::Repeat(unit, min, max) => {
                let mut out = Vec::new();
                if sub.ticks == 0 {
                    sub.ticks = 1;
                    if *min == 0 {
                        out.push(sub.env.clone());
                    }
                    if *max > 0 {
                        sub.repeated.push((0, SvaRun::with_env(sub.env.clone())));
                    }
                }
                let mut keep = Vec::new();
                for (count, mut run) in std::mem::take(&mut sub.repeated) {
                    let (matches, alive) = self.sva_run_advance(unit, &mut run, false);
                    if alive {
                        keep.push((count, run));
                    }
                    for env in matches {
                        let next = count.saturating_add(1);
                        if next >= *min {
                            out.push(env.clone());
                        }
                        if *max == u64::MAX || next < *max {
                            let next = if *max == u64::MAX {
                                next.min((*min).max(1))
                            } else {
                                next
                            };
                            keep.push((next, SvaRun::with_env(env)));
                        }
                    }
                }
                sub.repeated = keep;
                (out, !sub.repeated.is_empty())
            }
            SvaCompound::FirstMatch(inner) => {
                let (m, alive) = self.sva_run_advance(inner, &mut sub.runs[0], true);
                if !m.is_empty() {
                    (m, false)
                } else {
                    (m, alive)
                }
            }
            SvaCompound::Throughout(e, inner) => {
                let mut env = std::mem::take(&mut sub.env);
                let holds = self.sva_eval_env(e, &mut env).is_true();
                sub.env = env;
                if !holds {
                    return (Vec::new(), false);
                }
                self.sva_run_advance(inner, &mut sub.runs[0], false)
            }
            SvaCompound::And(n1, n2) => {
                let (m1, a1) = self.sva_run_advance(n1, &mut sub.runs[0], false);
                let (m2, a2) = self.sva_run_advance(n2, &mut sub.runs[1], false);
                let mut out: Vec<SvaEnv> = Vec::new();
                if !m1.is_empty() && !m2.is_empty() {
                    out.push(Self::sva_merge_env(&m1[0], &m2[0], &sub.env));
                } else if !m1.is_empty() && sub.ever[1] {
                    out.push(Self::sva_merge_env(&m1[0], &sub.ever_env[1], &sub.env));
                } else if !m2.is_empty() && sub.ever[0] {
                    out.push(Self::sva_merge_env(&sub.ever_env[0], &m2[0], &sub.env));
                }
                if let Some(e) = m1.into_iter().next() {
                    sub.ever[0] = true;
                    sub.ever_env[0] = e;
                }
                if let Some(e) = m2.into_iter().next() {
                    sub.ever[1] = true;
                    sub.ever_env[1] = e;
                }
                let dead = (!a1 && !sub.ever[0]) || (!a2 && !sub.ever[1]) || (!a1 && !a2);
                (out, !dead)
            }
            SvaCompound::Intersect(n1, n2) => {
                let (m1, a1) = self.sva_run_advance(n1, &mut sub.runs[0], false);
                let (m2, a2) = self.sva_run_advance(n2, &mut sub.runs[1], false);
                let out = if !m1.is_empty() && !m2.is_empty() {
                    vec![Self::sva_merge_env(&m1[0], &m2[0], &sub.env)]
                } else {
                    Vec::new()
                };
                (out, a1 && a2)
            }
        }
    }

    /// Advance one attempt by one clock tick; `Some` when it is decided.
    pub(super) fn sva_advance(&mut self, node: &SvaNode, st: &mut SvaState) -> Option<SvaOutcome> {
        match (node, st) {
            (SvaNode::Seq(nfa, _), SvaState::Seq(run)) => {
                let (m, alive) = self.sva_run_advance(nfa, run, true);
                if !m.is_empty() {
                    Some(SvaOutcome::Pass)
                } else if !alive {
                    Some(SvaOutcome::Fail)
                } else {
                    None
                }
            }
            (SvaNode::Not(inner), SvaState::Not(ist)) => match self.sva_advance(inner, ist) {
                Some(SvaOutcome::Fail) => Some(SvaOutcome::Pass),
                Some(_) => Some(SvaOutcome::Fail),
                None => None,
            },
            (
                SvaNode::Impl {
                    ante,
                    overlap,
                    cons: cons_node,
                },
                SvaState::Impl {
                    ante: arun,
                    ante_alive,
                    matched_any,
                    cons,
                },
            ) => {
                // Every match of the antecedent starts one evaluation of the
                // consequent, with the local variables of that match.
                let mut fresh: Vec<SvaState> = Vec::new();
                if *ante_alive {
                    let (m, alive) = self.sva_run_advance(ante, arun, false);
                    *ante_alive = alive;
                    if !m.is_empty() {
                        *matched_any = true;
                        for env in &m {
                            fresh.push(Self::sva_new_state(cons_node, env));
                        }
                    }
                }
                let mut failed = false;
                let mut keep: Vec<(u32, SvaState)> = Vec::new();
                for (mut delay, mut cst) in cons.drain(..) {
                    if delay > 0 {
                        delay -= 1;
                    }
                    if delay > 0 {
                        keep.push((delay, cst));
                        continue;
                    }
                    match self.sva_advance(cons_node, &mut cst) {
                        Some(SvaOutcome::Fail) => failed = true,
                        Some(_) => {}
                        None => keep.push((0, cst)),
                    }
                }
                for mut cst in fresh {
                    if *overlap {
                        match self.sva_advance(cons_node, &mut cst) {
                            Some(SvaOutcome::Fail) => failed = true,
                            Some(_) => {}
                            None => keep.push((0, cst)),
                        }
                    } else {
                        keep.push((1, cst));
                    }
                }
                *cons = keep;
                if failed {
                    cons.clear();
                    *ante_alive = false;
                    return Some(SvaOutcome::Fail);
                }
                if !*ante_alive && cons.is_empty() {
                    Some(if *matched_any {
                        SvaOutcome::Pass
                    } else {
                        SvaOutcome::Vacuous
                    })
                } else {
                    None
                }
            }
            (
                SvaNode::Followed {
                    ante,
                    overlap,
                    cons: cons_node,
                },
                SvaState::Followed {
                    ante: arun,
                    ante_alive,
                    cons,
                },
            ) => {
                // Unlike implication, one successful match suffices. Failure of
                // one consequent must not discard other possible match points.
                let mut fresh = Vec::new();
                if *ante_alive {
                    let (matches, alive) = self.sva_run_advance(ante, arun, false);
                    *ante_alive = alive;
                    for env in matches {
                        fresh.push(Self::sva_new_state(cons_node, &env));
                    }
                }
                // §16.12.9 / §16.14.8: a vacuous success decides the
                // attempt at this tick too. Check every evaluation at this
                // tick so a simultaneous nonvacuous success takes priority.
                let mut vacuous = false;
                let mut keep = Vec::new();
                for (mut delay, mut cst) in cons.drain(..) {
                    delay = delay.saturating_sub(1);
                    if delay > 0 {
                        keep.push((delay, cst));
                        continue;
                    }
                    match self.sva_advance(cons_node, &mut cst) {
                        Some(SvaOutcome::Pass) => return Some(SvaOutcome::Pass),
                        Some(SvaOutcome::Vacuous) => vacuous = true,
                        Some(SvaOutcome::Fail) => {}
                        None => keep.push((0, cst)),
                    }
                }
                for mut cst in fresh {
                    if *overlap {
                        match self.sva_advance(cons_node, &mut cst) {
                            Some(SvaOutcome::Pass) => return Some(SvaOutcome::Pass),
                            Some(SvaOutcome::Vacuous) => vacuous = true,
                            Some(SvaOutcome::Fail) => {}
                            None => keep.push((0, cst)),
                        }
                    } else {
                        keep.push((1, cst));
                    }
                }
                *cons = keep;
                if vacuous {
                    Some(SvaOutcome::Vacuous)
                } else if !*ante_alive && cons.is_empty() {
                    Some(SvaOutcome::Fail)
                } else {
                    None
                }
            }
            (SvaNode::And(na, nb), SvaState::And { a, b, ra, rb }) => {
                if ra.is_none() {
                    *ra = self.sva_advance(na, a);
                }
                if rb.is_none() {
                    *rb = self.sva_advance(nb, b);
                }
                if *ra == Some(SvaOutcome::Fail) || *rb == Some(SvaOutcome::Fail) {
                    return Some(SvaOutcome::Fail);
                }
                match (*ra, *rb) {
                    (Some(x), Some(y)) => {
                        Some(if x == SvaOutcome::Vacuous && y == SvaOutcome::Vacuous {
                            SvaOutcome::Vacuous
                        } else {
                            SvaOutcome::Pass
                        })
                    }
                    _ => None,
                }
            }
            (SvaNode::Or(na, nb), SvaState::Or { a, b, ra, rb }) => {
                if ra.is_none() {
                    *ra = self.sva_advance(na, a);
                }
                if rb.is_none() {
                    *rb = self.sva_advance(nb, b);
                }
                if *ra == Some(SvaOutcome::Pass) || *rb == Some(SvaOutcome::Pass) {
                    return Some(SvaOutcome::Pass);
                }
                if *ra == Some(SvaOutcome::Vacuous) || *rb == Some(SvaOutcome::Vacuous) {
                    return Some(SvaOutcome::Vacuous);
                }
                match (*ra, *rb) {
                    (Some(_), Some(_)) => Some(SvaOutcome::Fail),
                    _ => None,
                }
            }
            (SvaNode::Eventually(e), SvaState::Leaf(env)) => {
                if self.sva_eval_env(e, env).is_true() {
                    Some(SvaOutcome::Pass)
                } else {
                    None
                }
            }
            (SvaNode::Always(e), SvaState::Leaf(env)) => {
                if self.sva_eval_env(e, env).is_true() {
                    None
                } else {
                    Some(SvaOutcome::Fail)
                }
            }
            _ => Some(SvaOutcome::Vacuous),
        }
    }

    fn sva_event_value(&mut self, event: &SvaClockEvent) -> Value {
        match &event.signal {
            Some(name) => self
                .get_signal_value_by_name(name)
                .unwrap_or_else(|| Value::all_x(1)),
            None => self.eval_expr(&event.event.expr),
        }
    }

    /// §9.4.2: edge controls inspect the LSB; an unqualified expression
    /// event notices any value change, including changes above bit zero.
    pub(super) fn sva_events_fired(&mut self, events: &mut [SvaClockEvent]) -> bool {
        let mut fired = false;
        for event in events {
            let current = self.sva_event_value(event);
            let bit = |v: &Value| match v.get_bit(0) {
                LogicBit::Zero => 0,
                LogicBit::One => 1,
                LogicBit::X => 2,
                LogicBit::Z => 3,
            };
            let previous = bit(&event.previous);
            let now = bit(&current);
            let changed = match event.event.edge {
                Some(Edge::Posedge) => (previous == 0 && now != 0) || (previous >= 2 && now == 1),
                Some(Edge::Negedge) => (previous == 1 && now != 1) || (previous >= 2 && now == 0),
                Some(Edge::Edge) => previous != now && (previous < 2 || now < 2),
                None => current != event.previous,
            };
            event.previous = current;
            if changed
                && event
                    .event
                    .iff
                    .as_ref()
                    .is_none_or(|g| self.eval_expr(g).is_true())
            {
                fired = true;
            }
        }
        fired
    }

    /// LRM §16.5: register the clocked site of a concurrent assertion
    /// statement on its first execution: expand named sequences and
    /// properties, bind its local variables (§16.10) and, for a procedural
    /// assertion, the automatic variables it captures (§16.14.6.1), and
    /// compile the property.
    pub(super) fn register_sva_site(
        &mut self,
        a: &crate::ast::stmt::AssertionStatement,
        events: Vec<EventExpr>,
        body: Expression,
    ) {
        use crate::ast::stmt::AssertionKind;
        let span_key = a.span.start;
        let kind: u8 = match a.kind {
            AssertionKind::Assert => 0,
            AssertionKind::Assume => 1,
            AssertionKind::Cover => 2,
        };
        // Named sequence / property instances INSIDE the body (`a |=> s`)
        // expand here, once.
        let mut body_expanded = self.sva_expand_named(&body, 0);
        self.sva_fold_scopes(&mut body_expanded);
        let mut lv_init: Vec<Value> = Vec::new();
        let mut lv_types =
            self.sva_bind_locals(&mut body_expanded, &HashMap::default(), &mut lv_init);
        let procedural = a.procedural;
        let mut captured: Vec<(usize, String)> = Vec::new();
        if procedural {
            self.sva_capture_proc_locals(
                &mut body_expanded,
                &mut captured,
                &mut lv_init,
                &mut lv_types,
            );
        }
        // LRM §16.5.1: collect the ids of every signal referenced in the
        // property body so their Preponed (slot-entry) values can be sampled
        // when the assertion fires using the sampled-value overlay.
        let mut sampled_ids = Vec::new();
        self.collect_sva_signal_ids(&body_expanded, &mut sampled_ids);
        // §16.5.1: an element of an unpacked array is sampled too. A select
        // with constant indices names its cell now; one whose indices read
        // signals names it again in each Preponed region, from their sampled
        // values (`sva_sel_cells`), so only the cells read are copied, never
        // a whole memory. An index the Preponed region cannot evaluate (a
        // local variable, a sampled-value function, a call) samples every
        // cell of its array.
        let mut sels = Vec::new();
        self.collect_sva_sels(&body_expanded, &mut sels);
        let mut var_sels = Vec::new();
        for mut e in sels {
            match Self::sva_sel_kind(&e) {
                Some(true) => {
                    self.sva_cells(&e, &mut sampled_ids);
                }
                Some(false) => {
                    self.sva_resolve_signals(&mut e);
                    let dims = Self::sva_elem(&e).and_then(|(h, _)| {
                        let n = self.resolve_hier_name(h);
                        self.array_first_id.get(n.as_ref()).copied()
                    });
                    var_sels.push((e, dims));
                }
                None => self.sva_all_cells(&e, &mut sampled_ids),
            }
        }
        sampled_ids.sort_unstable();
        sampled_ids.dedup();
        let fixed = sampled_ids.len();
        for (e, _) in &var_sels {
            self.sva_cells(e, &mut sampled_ids);
        }
        // A site registered mid-slot (a procedural assertion, while its
        // procedure runs in the Active region) has no slot-entry samples yet:
        // take the values now, before this slot's nonblocking updates. In
        // time slot 0 (where the declarative sites register, after the time-0
        // settle has run the continuous assignments) a slot-entry value is
        // the default sampled value (§16.5.1).
        for &id in &sampled_ids {
            if !self.sva_preponed.contains_key(&id) {
                if let Some(v) = self.slot0_default(id).or_else(|| self.sva_cell_value(id)) {
                    self.sva_preponed.insert(id, v);
                }
            }
        }
        // For the first site these are the slot's only samples: a later delta
        // of the slot must not retake them (`refresh_sva_preponed`).
        if self.sva_sites.is_empty() {
            self.sva_preponed_slot = Some(self.time);
        }
        let mut past_names = Vec::new();
        self.collect_past_arg_names(&body_expanded, &mut past_names);
        // A signal is sampled once per property clock, however many
        // sampled-value calls reference it.
        past_names.sort_unstable();
        past_names.dedup();
        self.sva_resolve_signals(&mut body_expanded);
        self.sva_lv_types = lv_types;
        let (disable, node) = self.sva_compile_site(&body_expanded);
        self.sva_lv_types = Vec::new();
        let scope = self.active_instance_scope();
        // §16.15: without a `disable iff` of its own, the scope's default
        // disable condition applies.
        let explicit_disable = disable.is_some();
        let disable_scope = self.m_path();
        let disable = disable.or_else(|| self.sva_default_disable_for(&disable_scope));
        let mut m_chain = self.m_scope_stack.clone();
        if let Some(l) = &a.label {
            m_chain.push(l.name.clone());
        }
        let action_pid = self.next_pid;
        self.next_pid += 1;
        self.process_scope_hint.insert(action_pid, scope.clone());
        let ctl = CtlSite::new(CtlIdent {
            scope: self.instance_relative_scope(&self.m_path()),
            label: a.label.as_ref().map(|l| l.name.clone()),
            atype: assert_ctl::AT_CONCURRENT,
            dtype: [
                assert_ctl::DT_ASSERT,
                assert_ctl::DT_ASSUME,
                assert_ctl::DT_COVER,
            ][kind as usize],
        });
        let src = self.scope_src_file(Some(scope.as_str()));
        // A procedural assertion registers while its procedure runs, which
        // for an edge-triggered procedure is at the tick that starts its
        // first attempt: take that edge as seen (§16.14.6).
        let mut clock_events = Vec::with_capacity(events.len());
        for mut event in events {
            self.sva_fold_scopes(&mut event.expr);
            self.sva_resolve_signals(&mut event.expr);
            if let Some(guard) = &mut event.iff {
                self.sva_resolve_signals(guard);
            }
            let signal = match &event.expr.kind {
                ExprKind::Ident(h) if h.path.iter().all(|p| p.selects.is_empty()) => {
                    Some(self.resolve_hier_name(h).into_owned())
                }
                _ => None,
            };
            let mut clock_event = SvaClockEvent {
                event,
                signal,
                previous: Value::all_x(1),
            };
            clock_event.previous = self.sva_event_value(&clock_event);
            if procedural {
                let bit = clock_event.previous.get_bit(0);
                let opposite = match (clock_event.event.edge, bit) {
                    (Some(Edge::Posedge), LogicBit::One) => Some(LogicBit::Zero),
                    (Some(Edge::Negedge), LogicBit::Zero) => Some(LogicBit::One),
                    (None | Some(Edge::Edge), LogicBit::Zero) => Some(LogicBit::One),
                    (None | Some(Edge::Edge), LogicBit::One) => Some(LogicBit::Zero),
                    _ => None,
                };
                if let Some(bit) = opposite {
                    clock_event.previous.set_bit(0, bit);
                }
            }
            clock_events.push(clock_event);
        }
        let clock_signal = clock_events
            .first()
            .and_then(|ev| ev.signal.clone())
            .unwrap_or_default();
        let edge = match clock_events.first().and_then(|ev| ev.event.edge) {
            Some(Edge::Posedge) => 0,
            Some(Edge::Negedge) => 1,
            _ => 2,
        };
        self.sva_sites.push(SvaClockedSite {
            span_key,
            stat_key: Self::assert_stat_key(span_key, src),
            stat_loc: (src, span_key),
            dedup_scope: self.current_scope.clone(),
            kind,
            all_matches: a.is_sequence,
            clock_signal,
            edge,
            events: clock_events,
            past_names,
            node: std::sync::Arc::new(node),
            disable,
            fired: false,
            attempts: Vec::new(),
            past_snapshots: HashMap::default(),
            pass_action: a.action.as_deref().cloned(),
            fail_action: a.else_action.as_deref().cloned(),
            scope,
            action_pid,
            m_chain,
            sampled_ids,
            sels: var_sels,
            fixed,
            lv_init,
            procedural,
            captured,
            matured: Vec::new(),
            explicit_disable,
            disable_scope,
            ctl,
        });
        // §16.9.3: the assertion's leading clock also clocks sampled calls
        // used only in its action blocks; history starts before the first tick.
        {
            let cid = self
                .signal_name_to_id
                .get(self.sva_sites.last().unwrap().clock_signal.as_str())
                .copied()
                .unwrap_or(usize::MAX);
            let ec = match edge {
                1 => 2,
                2 => 0,
                _ => 1,
            };
            let saved_scope = self.activation_scope.borrow().clone();
            *self.activation_scope.borrow_mut() =
                Some(self.sva_sites.last().unwrap().scope.clone());
            let old_registering = self.sampled_registering;
            let old_site = self.active_sva_site.replace(self.sva_sites.len() - 1);
            self.sampled_registering = true;
            let body_stmt = Statement::new(
                StatementKind::Expr(body_expanded.clone()),
                body_expanded.span,
            );
            self.record_sampled_sites(&body_stmt, ec, cid);
            self.scan_sampled_expr(&body_expanded);
            for action in [&a.action, &a.else_action].into_iter().flatten() {
                let first = self.sampled_watches.len();
                self.record_sampled_sites(action, ec, cid);
                self.scan_sampled_stmt(action);
                for watch in &mut self.sampled_watches[first..] {
                    watch.action_gate = true;
                }
            }
            self.sampled_registering = old_registering;
            self.active_sva_site = old_site;
            *self.activation_scope.borrow_mut() = saved_scope;
        }
    }

    /// Tally one finished attempt on a site: cover sites count matches only.
    pub(super) fn sva_tally(&mut self, site_idx: usize, span_key: usize, outcome: bool) {
        let kind = self.sva_sites[site_idx].kind;
        let loc = self.sva_sites[site_idx].stat_loc;
        let stat = self
            .assertion_stats
            .entry(span_key)
            .or_insert_with(|| AssertionStat {
                kind,
                loc: Some(loc),
                pass_count: 0,
                fail_count: 0,
            });
        stat.kind = kind;
        if outcome {
            stat.pass_count += 1;
            self.fire_sva_action(site_idx, true);
        } else if kind != 2 {
            stat.fail_count += 1;
            self.fire_sva_action(site_idx, false);
        }
    }

    pub(super) fn fire_sva_action(&mut self, site_idx: usize, passed: bool) {
        // §20.12 PassOff/FailOff, decided when the attempt finishes.
        if self.assert_ctl_on {
            let st = self.sva_site_ctl(site_idx);
            if !(if passed {
                st.pass_action(false)
            } else {
                st.fail_action()
            }) {
                return;
            }
        }
        let Some((stmt, scope, pid, m_chain)) = self.sva_sites.get(site_idx).and_then(|s| {
            let act = if passed {
                s.pass_action.clone()
            } else {
                // §16.3: a failure with no `else` clause calls `$error`
                // (a cover has no failure action).
                s.fail_action.clone().or_else(|| {
                    (s.kind != 2).then(|| {
                        let span = crate::ast::Span::dummy();
                        Statement::new(
                            StatementKind::Expr(Expression::new(
                                ExprKind::SystemCall {
                                    name: "$error".to_string(),
                                    args: vec![Expression::new(
                                        ExprKind::StringLiteral(ASSERT_DEFAULT_ERROR.to_string()),
                                        span,
                                    )],
                                },
                                span,
                            )),
                            span,
                        )
                    })
                })
            };
            act.map(|a| (a, s.scope.clone(), s.action_pid, s.m_chain.clone()))
        }) else {
            return;
        };
        let saved_pid = std::mem::replace(&mut self.current_pid, pid);
        let saved_scope = std::mem::replace(&mut self.current_scope, scope);
        let saved_block_scope = std::mem::take(&mut self.m_block_scope);
        let saved_block_active = std::mem::replace(&mut self.m_block_active, false);
        let saved_m_scope = std::mem::replace(&mut self.m_scope_stack, m_chain);
        // §16.14.1/§6.21: action-block declarations are local to the
        // action, including names that shadow the declaring module's signals.
        self.push_local_frame(HashMap::default());
        self.push_queue_frame();
        self.open_decl_shadow_frame();
        self.exec_statement(&stmt);
        self.close_decl_shadow_frame();
        self.pop_and_restore_queue_frame();
        self.pop_local_frame();
        self.current_pid = saved_pid;
        self.current_scope = saved_scope;
        self.m_block_scope = saved_block_scope;
        self.m_block_active = saved_block_active;
        self.m_scope_stack = saved_m_scope;
    }

    /// The root identifier of select chain `e` (`m` of `m[1][j][3:0]`, `sa`
    /// of `sa[2].x`), when it is a plain name.
    pub(super) fn sva_sel_root(e: &Expression) -> Option<&HierarchicalIdentifier> {
        match &e.kind {
            ExprKind::Index { expr, .. }
            | ExprKind::RangeSelect { expr, .. }
            | ExprKind::MemberAccess { expr, .. } => Self::sva_sel_root(expr),
            ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => Some(h),
            _ => None,
        }
    }

    /// §16.5.1: collect the selects of fixed-size unpacked arrays in a
    /// concurrent assertion's body, outermost only (`p[1][2][3:0]`, not
    /// `p[1]` as well), and those in their indices (`b[0]` of `a[b[0]]`).
    pub(super) fn collect_sva_sels(&self, e: &Expression, out: &mut Vec<Expression>) {
        let chain = matches!(
            e.kind,
            ExprKind::Index { .. } | ExprKind::RangeSelect { .. } | ExprKind::MemberAccess { .. }
        );
        if chain
            && Self::sva_sel_root(e).is_some_and(|h| {
                let n = self.resolve_hier_name(h);
                self.array_first_id.contains_key(n.as_ref())
                    || self.module.arrays.contains_key(n.as_ref())
                    || self.module.arrays_2d.contains_key(n.as_ref())
                    || self.module.arrays_nd.contains_key(n.as_ref())
            })
        {
            out.push(e.clone());
            let mut cur = e;
            loop {
                match &cur.kind {
                    ExprKind::Index { expr, index } => {
                        self.collect_sva_sels(index, out);
                        cur = expr;
                    }
                    ExprKind::RangeSelect {
                        expr, left, right, ..
                    } => {
                        self.collect_sva_sels(left, out);
                        self.collect_sva_sels(right, out);
                        cur = expr;
                    }
                    ExprKind::MemberAccess { expr, .. } => cur = expr,
                    _ => return,
                }
            }
        }
        let mut sub = |x: &Expression| self.collect_sva_sels(x, out);
        match &e.kind {
            ExprKind::Unary { operand: x, .. }
            | ExprKind::Paren(x)
            | ExprKind::MemberAccess { expr: x, .. } => sub(x),
            ExprKind::Binary { left, right, .. } => {
                sub(left);
                sub(right);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                sub(condition);
                sub(then_expr);
                sub(else_expr);
            }
            ExprKind::Index { expr, index } => {
                sub(expr);
                sub(index);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                sub(expr);
                sub(left);
                sub(right);
            }
            ExprKind::Concatenation(v)
            | ExprKind::SystemCall { args: v, .. }
            | ExprKind::Call { args: v, .. } => v.iter().for_each(&mut sub),
            ExprKind::Replication { count, exprs } => {
                sub(count);
                exprs.iter().for_each(&mut sub);
            }
            ExprKind::Inside { expr, ranges } => {
                sub(expr);
                ranges.iter().for_each(&mut sub);
            }
            // The bounds of an `inside` range (`[a[0]:a[1]]`).
            ExprKind::Range(lo, hi) => {
                sub(lo);
                sub(hi);
            }
            ExprKind::SvaClocked { body, .. } => sub(body),
            _ => {}
        }
    }

    /// The indices of select chain `e`: constant (`Some(true)`), reading
    /// signals the Preponed region can evaluate (`Some(false)`), or reading
    /// something it cannot, such as a local variable, a sampled-value
    /// function or a call (`None`).
    pub(super) fn sva_sel_kind(e: &Expression) -> Option<bool> {
        let mut konst = true;
        let mut ok = true;
        let mut check = |x: &Expression| {
            Self::walk_operands(x, &mut |y| match &y.kind {
                ExprKind::Number(_)
                | ExprKind::Paren(_)
                | ExprKind::Unary { .. }
                | ExprKind::Binary { .. }
                | ExprKind::Conditional { .. } => {}
                ExprKind::Ident(_)
                | ExprKind::Index { .. }
                | ExprKind::RangeSelect { .. }
                | ExprKind::MemberAccess { .. } => konst = false,
                _ => ok = false,
            })
        };
        let mut cur = e;
        loop {
            match &cur.kind {
                ExprKind::Index { expr, index } => {
                    check(index);
                    cur = expr;
                }
                ExprKind::RangeSelect {
                    expr, left, right, ..
                } => {
                    check(left);
                    check(right);
                    cur = expr;
                }
                ExprKind::MemberAccess { expr, .. } => cur = expr,
                _ => break,
            }
        }
        ok.then_some(konst)
    }

    /// The storage name select chain `e` reads (`m[1][2]`, `sa[2].x`), its
    /// indices evaluated now; pushes the id of every cell on the way (`p[1]`
    /// for `p[1][2]` of an array of packed vectors; `sa[2]` and `sa[2].x`;
    /// each cell of a slice).
    pub(super) fn sva_cells(&mut self, e: &Expression, out: &mut Vec<usize>) -> Option<String> {
        let name = match &e.kind {
            ExprKind::Ident(h) => return Some(self.resolve_hier_name(h).into_owned()),
            ExprKind::Index { expr, index } => {
                let base = self.sva_cells(expr, out)?;
                let i = self.eval_expr(index);
                if i.has_xz() {
                    return None;
                }
                Self::name_with_index(&base, i.to_i64()?)
            }
            ExprKind::MemberAccess { expr, member } => {
                let mut base = self.sva_cells(expr, out)?;
                base.push('.');
                base.push_str(&member.name);
                base
            }
            ExprKind::RangeSelect {
                expr,
                left,
                right,
                kind,
            } => {
                let base = self.sva_cells(expr, out)?;
                // A part-select of a cell reads that cell.
                if self.resolve_signal_id(&base).is_some() {
                    return Some(base);
                }
                // A slice of an unpacked array (an array port's connection,
                // `a[1:2]`) reads the cells in it; no single name follows.
                let (l, r) = (self.eval_expr(left), self.eval_expr(right));
                if l.has_xz() || r.has_xz() {
                    return None;
                }
                let (Some(l), Some(r)) = (l.to_i64(), r.to_i64()) else {
                    return None;
                };
                // §11.5.1 permits out-of-range selects. Form the bounds in
                // wider arithmetic before clipping so even an unused
                // assertion with an extreme index cannot overflow here.
                let (l, r) = (i128::from(l), i128::from(r));
                let (lo, hi) = match kind {
                    RangeKind::Constant => (l.min(r), l.max(r)),
                    RangeKind::IndexedUp if r > 0 => (l, l + r - 1),
                    RangeKind::IndexedDown if r > 0 => (l - r + 1, l),
                    _ => return None,
                };
                // Only the indices the array has.
                let (alo, ahi) = self.sva_slice_bounds(&base)?;
                for i in lo.max(i128::from(alo))..=hi.min(i128::from(ahi)) {
                    if let Some(id) =
                        self.resolve_signal_id(&Self::name_with_index(&base, i as i64))
                    {
                        out.push(id);
                    }
                }
                return None;
            }
            _ => return None,
        };
        if let Some(id) = self.resolve_signal_id(&name) {
            out.push(id);
        }
        Some(name)
    }

    /// The index range of the unpacked array or sub-array `base` (`a`, or
    /// `m[1]` of a two-dimensional `m`).
    pub(super) fn sva_slice_bounds(&self, base: &str) -> Option<(i64, i64)> {
        if let Some(&(_, lo, hi)) = self.array_first_id.get(base) {
            return Some((lo, hi));
        }
        if let Some(&(l, r, _)) = self.module.arrays.get(base) {
            return Some((l.min(r), l.max(r)));
        }
        let (root, _) = base.split_once('[')?;
        let &(_, (l, r), _) = self.module.arrays_2d.get(root)?;
        Some((l.min(r), l.max(r)))
    }

    /// Every cell of the array select chain `e` reads, for an index the
    /// Preponed region cannot evaluate.
    pub(super) fn sva_all_cells(&self, e: &Expression, out: &mut Vec<usize>) {
        let Some(h) = Self::sva_sel_root(e) else {
            return;
        };
        let root = self.resolve_hier_name(h);
        if let Some(&(first, lo, hi)) = self.array_first_id.get(root.as_ref()) {
            if hi >= lo {
                out.extend(first..=first + (hi - lo) as usize);
            }
        }
        let pre = Self::name_with_suffix(&root, "[");
        out.extend(
            self.signal_name_to_id
                .iter()
                .filter(|(n, _)| n.starts_with(&pre))
                .map(|(_, &id)| id),
        );
    }

    /// LRM §16.5.1 — walk an SVA property body and collect the ids of every
    /// referenced signal (base ids of index/part-selects included). These are
    /// the signals whose Preponed slot-entry values must be sampled when the
    /// clocked assertion fires. Over-collection is harmless — an id that never
    /// changes on the clock edge has an identical preponed and live value.
    pub(super) fn collect_sva_signal_ids(&self, expr: &Expression, out: &mut Vec<usize>) {
        match &expr.kind {
            ExprKind::Ident(h) => {
                // A field may be parsed as a hierarchical identifier instead
                // of MemberAccess. Sample its nearest owning aggregate too.
                let name = self.resolve_hier_name(h);
                let mut prefix = name.as_ref();
                loop {
                    if let Some(&id) = self.signal_name_to_id.get(prefix) {
                        out.push(id);
                    }
                    match prefix.rsplit_once('.') {
                        Some((p, _)) => prefix = p,
                        None => break,
                    }
                }
            }
            ExprKind::MemberAccess { expr: base, .. } => {
                // Packed fields may have mirrored leaf signals. Sample them
                // together with the aggregate that owns the selected bits.
                if let Some(parts) = Self::flatten_member_path(expr) {
                    if let Some(&id) = self.signal_name_to_id.get(parts.join(".").as_str()) {
                        out.push(id);
                    }
                }
                self.collect_sva_signal_ids(base, out);
            }
            ExprKind::Unary { operand, .. } => self.collect_sva_signal_ids(operand, out),
            ExprKind::Binary { left, right, .. } => {
                self.collect_sva_signal_ids(left, out);
                self.collect_sva_signal_ids(right, out);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.collect_sva_signal_ids(condition, out);
                self.collect_sva_signal_ids(then_expr, out);
                self.collect_sva_signal_ids(else_expr, out);
            }
            ExprKind::Paren(inner) => self.collect_sva_signal_ids(inner, out),
            ExprKind::Index { expr: base, index } => {
                // The base identifier is the sampled signal; also walk the
                // index (it may itself reference a sampled signal).
                if let Some(id) = self.get_lhs_signal_id(base) {
                    out.push(id);
                }
                self.collect_sva_signal_ids(base, out);
                self.collect_sva_signal_ids(index, out);
            }
            ExprKind::RangeSelect {
                expr: base,
                left,
                right,
                ..
            } => {
                if let Some(id) = self.get_lhs_signal_id(base) {
                    out.push(id);
                }
                self.collect_sva_signal_ids(base, out);
                self.collect_sva_signal_ids(left, out);
                self.collect_sva_signal_ids(right, out);
            }
            ExprKind::Concatenation(exprs) => {
                for e in exprs {
                    self.collect_sva_signal_ids(e, out);
                }
            }
            ExprKind::Replication { count, exprs } => {
                self.collect_sva_signal_ids(count, out);
                for e in exprs {
                    self.collect_sva_signal_ids(e, out);
                }
            }
            ExprKind::SystemCall { name, args } => {
                // `$past/$rose/$fell/$stable/$changed` already read from the
                // per-site ring buffers; their bare argument must NOT be
                // preponed here (that would double-sample). Skip a leading
                // bare-Ident argument to those calls; still walk the rest.
                let skip_first = matches!(
                    name.as_str(),
                    "$past" | "$rose" | "$fell" | "$stable" | "$changed"
                );
                for (i, a) in args.iter().enumerate() {
                    if skip_first && i == 0 && matches!(a.kind, ExprKind::Ident(_)) {
                        continue;
                    }
                    self.collect_sva_signal_ids(a, out);
                }
            }
            ExprKind::Call { args, .. } => {
                for a in args {
                    self.collect_sva_signal_ids(a, out);
                }
            }
            ExprKind::Inside { expr, ranges } => {
                self.collect_sva_signal_ids(expr, out);
                for r in ranges {
                    self.collect_sva_signal_ids(r, out);
                }
            }
            // The bounds of an `inside` range are sampled too (§16.5.1).
            ExprKind::Range(lo, hi) => {
                self.collect_sva_signal_ids(lo, out);
                self.collect_sva_signal_ids(hi, out);
            }
            ExprKind::SvaClocked { body, .. } => self.collect_sva_signal_ids(body, out),
            _ => {}
        }
    }

    /// LRM §16.5.1 — capture the Preponed (slot-entry) value of every
    /// SVA-referenced signal. Called once at the top of each time slot,
    /// before the active/NBA regions run, so `sva_preponed[id]` holds the
    /// value as of the start of the slot. No-op when no clocked sites exist.
    /// LRM §14.4 — capture the Preponed (slot-entry) value of every
    /// clocking-block INPUT signal, keyed by name, before the active/NBA
    /// regions run. `tick_clocking_blocks` reads these on a posedge so the
    /// `#1step` input skew samples pre-edge values. No-op with no clocking
    /// blocks.
    pub(super) fn refresh_clocking_preponed(&mut self) {
        if self.clocking_meta.is_empty() || self.clocking_preponed_slot == Some(self.time) {
            return;
        }
        self.clocking_preponed_slot = Some(self.time);
        let names: Vec<String> = self
            .clocking_meta
            .values()
            .flat_map(|(_, sigs)| {
                sigs.iter()
                    .filter(|(_, is_input)| *is_input)
                    .map(|(n, _)| n.clone())
            })
            .collect();
        for n in names {
            if let Some(v) = self.get_signal_value_by_name(&n) {
                self.clocking_preponed.insert(n, v);
            }
        }
        if !self.clocking_hist.is_empty() {
            self.record_clocking_hist();
        }
    }

    /// §14.4 `input #N`: at slot entry, the value of each tracked net is its
    /// value at the end of the previous slot; record it against that slot
    /// when it changed, and drop entries no skew can reach any more.
    pub(super) fn record_clocking_hist(&mut self) {
        let now = self.time;
        let prev = self.clocking_hist_slot;
        if prev == Some(now) {
            return;
        }
        self.clocking_hist_slot = Some(now);
        let nets: Vec<String> = self.clocking_hist.keys().cloned().collect();
        for n in nets {
            let Some(v) = self.get_signal_value_by_name(&n) else {
                continue;
            };
            let Some(h) = self.clocking_hist.get_mut(&n) else {
                continue;
            };
            let Some(prev) = prev else {
                h.init = Some(v);
                continue;
            };
            let last = h.changes.back().map(|(_, v)| v).or(h.init.as_ref());
            if last != Some(&v) {
                h.changes.push_back((prev, v));
            }
            let floor = now.saturating_sub(h.span);
            while h.changes.len() >= 2 && h.changes[1].0 < floor {
                h.changes.pop_front();
            }
        }
    }

    /// §14.4: the value net `n` had at the START of time `at` — its value at
    /// the end of the last slot before `at` (a change made at `at` itself is
    /// not yet visible to an `input #N` sample taken then).
    pub(super) fn clocking_hist_value(&self, n: &str, at: u64) -> Option<Value> {
        let h = self.clocking_hist.get(n)?;
        for (t, v) in h.changes.iter().rev() {
            if *t < at {
                return Some(v.clone());
            }
        }
        h.init.clone()
    }

    pub(super) fn refresh_sva_preponed(&mut self) {
        // Once per slot (§4.4.2.1): a later delta of the slot keeps them.
        if self.sva_sites.is_empty() || self.sva_preponed_slot == Some(self.time) {
            return;
        }
        self.sva_preponed_slot = Some(self.time);
        self.sva_preponed.clear();
        for i in 0..self.sva_sites.len() {
            if !self.sva_sites[i].sels.is_empty() {
                self.sva_sel_cells(i);
            }
            for j in 0..self.sva_sites[i].sampled_ids.len() {
                let id = self.sva_sites[i].sampled_ids[j];
                if id < self.signal_table.len() {
                    self.sva_preponed.insert(id, self.signal_table[id].clone());
                } else if is_packed_id(id) {
                    self.sva_preponed.insert(id, self.packed.read(id));
                }
            }
        }
    }

    /// The value of cell `id`, in the signal table or the packed arena.
    pub(super) fn sva_cell_value(&self, id: usize) -> Option<Value> {
        if is_packed_id(id) {
            return Some(self.packed.read(id));
        }
        self.signal_table.get(id).cloned()
    }

    /// §16.5.1: rename the cells site `i`'s variable selects read, from
    /// their indices' values now, which are the sampled ones.
    #[inline(never)]
    pub(super) fn sva_sel_cells(&mut self, i: usize) {
        let sels = std::mem::take(&mut self.sva_sites[i].sels);
        let mut ids = std::mem::take(&mut self.sva_sites[i].sampled_ids);
        ids.truncate(self.sva_sites[i].fixed);
        // No class context, as at a tick (`tick_sva_sites`).
        let this = std::mem::take(&mut self.this_stack);
        let cls = std::mem::take(&mut self.class_context_stack);
        for (e, dims) in &sels {
            // An element of a 1-D array, the shape of nearly every select:
            // its cell by arithmetic.
            if let (Some((first, lo, hi)), Some((_, idx))) = (*dims, Self::sva_elem(e)) {
                let i = self.eval_expr(idx);
                if let Some(i) = i.to_i64().filter(|_| !i.has_xz())
                    && (lo..=hi).contains(&i)
                {
                    ids.push(first + (i - lo) as usize);
                }
                continue;
            }
            self.sva_cells(e, &mut ids);
        }
        self.this_stack = this;
        self.class_context_stack = cls;
        self.sva_sites[i].sels = sels;
        self.sva_sites[i].sampled_ids = ids;
    }

    /// The array and the index of `mem[k]`, or of a part-select of it.
    pub(super) fn sva_elem(e: &Expression) -> Option<(&HierarchicalIdentifier, &Expression)> {
        match &e.kind {
            ExprKind::RangeSelect { expr, .. } => Self::sva_elem(expr),
            ExprKind::Index { expr, index } => match &expr.kind {
                ExprKind::Ident(h) => Some((h, index)),
                _ => None,
            },
            _ => None,
        }
    }

    /// Swap in the Preponed sample for each `id` that has one cached; returns
    /// the saved live values for `restore_preponed`.
    pub(super) fn install_preponed(&mut self, ids: &[usize]) -> Vec<(usize, Value)> {
        let mut saved: Vec<(usize, Value)> = Vec::new();
        for &id in ids {
            // Only cells of the signal table or the packed arena are sampled.
            if let Some(pre) = self.sva_preponed.get(&id) {
                let pre = pre.clone();
                if id < self.signal_table.len() {
                    saved.push((id, std::mem::replace(&mut self.signal_table[id], pre)));
                } else {
                    // Past a `force` too: a forced cell's value can change
                    // within the slot like any other.
                    saved.push((id, self.packed.read(id)));
                    let (v, x) = pre.raw_bits();
                    self.packed.put_raw(id, v, x);
                }
            }
        }
        saved
    }

    /// Restore the live values swapped out by `install_preponed`.
    pub(super) fn restore_preponed(&mut self, saved: Vec<(usize, Value)>) {
        for (id, v) in saved.into_iter().rev() {
            if id < self.signal_table.len() {
                self.signal_table[id] = v;
                self.sync_mirror(id);
            } else {
                let (v, x) = v.raw_bits();
                self.packed.put_raw(id, v, x);
            }
        }
    }

    // Inlined, so a time slot of a design without assertions pays only the
    // two tests below.
    #[inline(always)]
    pub(super) fn tick_sva_sites(&mut self) {
        if self.finished {
            return;
        }
        if self.sva_sites.is_empty() {
            return;
        }
        // Concurrent assertions live in modules and interfaces (§16.14),
        // never in a class: evaluate them without the class context the last
        // process left behind, whose `this` made every signal read in a
        // property probe the object's members first.
        let saved_this = std::mem::take(&mut self.this_stack);
        let saved_cls = std::mem::take(&mut self.class_context_stack);
        self.finish_action_sampled_watches();
        self.tick_sva_sites_inner();
        self.this_stack = saved_this;
        self.class_context_stack = saved_cls;
    }

    /// §4.4.2: the Observed region of a delta. It follows the whole active
    /// region set, the Post-NBA cbReadWriteSynch routines included, so with
    /// a cbNBASynch or cbReadWriteSynch due in this slot `sva_post_nba`
    /// finishes that set first; inside it, a delta's Observed region only
    /// records clock events. Inline in both callers, so a design without
    /// assertions pays one emptiness check per delta.
    #[inline(always)]
    pub(super) fn sva_observed(&mut self) {
        if self.sva_sites.is_empty() {
            return;
        }
        if self.sva_post_nba_busy || self.vpi_synch_due() {
            self.sva_post_nba();
        } else {
            self.tick_sva_sites();
        }
    }

    /// Record the assertion clocks' events now, for a tick deferred until
    /// the slot's due cbReadWriteSynch routines have run (`sva_post_nba`): an
    /// edge such a routine moves back still starts its attempts.
    pub(super) fn note_sva_edges(&mut self) {
        if self.finished || self.sva_sites.is_empty() {
            return;
        }
        let saved_this = std::mem::take(&mut self.this_stack);
        let saved_cls = std::mem::take(&mut self.class_context_stack);
        self.sva_note_edges();
        self.this_stack = saved_this;
        self.class_context_stack = saved_cls;
    }

    /// Mark each site whose clock event happened since the last call, with
    /// its `iff` guard checked then.
    pub(super) fn sva_note_edges(&mut self) {
        for i in 0..self.sva_sites.len() {
            let mut events = std::mem::take(&mut self.sva_sites[i].events);
            let fired = self.sva_events_fired(&mut events);
            self.sva_sites[i].events = events;
            if !fired {
                self.sva_cancel_disabled(i);
                continue;
            }
            self.sva_sites[i].fired = true;
            self.sva_owed = true;
        }
    }

    pub(super) fn tick_sva_sites_inner(&mut self) {
        // §16.14.6.2: the procedural instances queued so far mature (no
        // flush point can drop them any more); each starts an attempt at the
        // next tick of its leading clock.
        if !self.sva_proc_pending.is_empty() {
            for (idx, _, env) in std::mem::take(&mut self.sva_proc_pending) {
                if let Some(site) = self.sva_sites.get_mut(idx) {
                    site.matured.push(env);
                }
            }
        }
        // The clock events since the last call, and those a deferred tick
        // recorded (`note_sva_edges`).
        self.sva_note_edges();
        self.sva_owed = false;
        // Use indices so we can call &mut self methods inside the loop
        // without holding a borrow on self.sva_sites.
        for i in 0..self.sva_sites.len() {
            if self.finished {
                break;
            }
            // Only the sites whose clock event was recorded tick.
            if !std::mem::take(&mut self.sva_sites[i].fired) {
                continue;
            }
            let span_key = self.sva_sites[i].stat_key;
            let site_loc = self.sva_sites[i].stat_loc;
            let site_kind = self.sva_sites[i].kind;
            let node = self.sva_sites[i].node.clone();
            // LRM §16.5.1: the referenced-signal ids whose Preponed samples
            // this fire's predicates must read through the sampled-value overlay.
            let sampled_ids = std::mem::take(&mut self.sva_sites[i].sampled_ids);
            let prev_active = self.active_sva_site.replace(i);
            let prev_activation_scope = self.activation_scope.borrow().clone();
            *self.activation_scope.borrow_mut() = Some(self.sva_sites[i].scope.clone());
            // LRM §16.9.3: refresh the past-value snapshots BEFORE anything
            // evaluates, so `$past(<sig>)` in a consequent that comes due
            // this tick reads the previous cycle's sample too.
            self.refresh_sva_past_snapshots(i, &sampled_ids);
            // On a sampling edge, refresh sampled-value history before
            // evaluating a disable condition that refers to that history.
            if self.sva_cancel_disabled(i) {
                self.sva_sites[i].sampled_ids = sampled_ids;
                self.active_sva_site = prev_active;
                *self.activation_scope.borrow_mut() = prev_activation_scope;
                continue;
            }
            let saved = self.install_preponed(&sampled_ids);
            let mut attempts = std::mem::take(&mut self.sva_sites[i].attempts);
            // §20.12 Off/Kill: no new attempt; those in flight go on.
            let starts = !self.assert_ctl_on || self.sva_site_ctl(i).enabled();
            if self.sva_sites[i].procedural {
                // §16.14.6: one attempt per matured instance, none otherwise.
                for env in std::mem::take(&mut self.sva_sites[i].matured) {
                    if starts {
                        attempts.push(Self::sva_new_state(&node, &env));
                    }
                }
            } else if starts {
                attempts.push(Self::sva_new_state(&node, &self.sva_sites[i].lv_init));
            }
            let mut results: Vec<SvaOutcome> = Vec::new();
            let mut live: Vec<SvaState> = Vec::with_capacity(attempts.len());
            let all_matches = self.sva_sites[i].all_matches;
            for mut st in attempts.drain(..) {
                if all_matches {
                    let step = match (&*node, &mut st) {
                        (SvaNode::Seq(nfa, _), SvaState::Seq(run)) => {
                            Some(self.sva_run_advance(nfa, run, false))
                        }
                        _ => None,
                    };
                    if let Some((matched, alive)) = step {
                        if !matched.is_empty() {
                            results.push(SvaOutcome::Pass);
                        }
                        if alive {
                            live.push(st);
                        }
                        continue;
                    }
                }
                match self.sva_advance(&node, &mut st) {
                    Some(r) => results.push(r),
                    None => live.push(st),
                }
            }
            // §16.7: an attempt with a long legal delay remains live;
            // dropping it at a fixed queue depth changes assertion results.
            self.sva_sites[i].attempts = live;
            self.restore_preponed(saved);
            self.sva_sites[i].sampled_ids = sampled_ids;
            for r in results {
                match r {
                    SvaOutcome::Pass => self.sva_tally(i, span_key, true),
                    SvaOutcome::Fail => self.sva_tally(i, span_key, false),
                    SvaOutcome::Vacuous => {
                        let stat =
                            self.assertion_stats
                                .entry(span_key)
                                .or_insert_with(|| AssertionStat {
                                    kind: site_kind,
                                    loc: Some(site_loc),
                                    pass_count: 0,
                                    fail_count: 0,
                                });
                        stat.kind = site_kind;
                        // §16.12.1: holds, but no pass action unless a
                        // §20.12 PassOn enabled it for vacuous successes.
                        if site_kind != 2 {
                            stat.pass_count += 1;
                            if self.assert_ctl_on && self.sva_site_ctl(i).pass_action(true) {
                                self.fire_sva_action(i, true);
                            }
                        }
                    }
                }
            }
            self.active_sva_site = prev_active;
            *self.activation_scope.borrow_mut() = prev_activation_scope;
        }
    }

    /// Collect bare-signal `$past` argument names into the site's
    /// snapshot ring, pushing this cycle's sampled value and trimming to a
    /// max depth. Each unique signal gets its own ring; index 0 = this
    /// cycle, index N = N cycles ago.
    pub(super) fn refresh_sva_past_snapshots(&mut self, site_idx: usize, sampled_ids: &[usize]) {
        const PAST_MAX_DEPTH: usize = 16;
        // Deduplicated at registration: a duplicate name would make ring[1]
        // another copy of the current cycle instead of the prior one.
        if self.sva_sites[site_idx].past_names.is_empty() {
            return;
        }
        let names = std::mem::take(&mut self.sva_sites[site_idx].past_names);
        for name in &names {
            // The sampled (Preponed) value when the site tracks the signal.
            let pre = self
                .signal_name_to_id
                .get(name.as_str())
                .copied()
                .filter(|id| sampled_ids.contains(id))
                .and_then(|id| self.sva_preponed.get(&id).cloned());
            let v = pre
                .or_else(|| self.get_signal_value_by_name(name))
                .unwrap_or_else(|| Value::zero(32));
            let snaps = &mut self.sva_sites[site_idx].past_snapshots;
            let ring = match snaps.get_mut(name.as_str()) {
                Some(r) => r,
                None => snaps.entry(name.clone()).or_default(),
            };
            ring.push_front(v);
            while ring.len() > PAST_MAX_DEPTH {
                ring.pop_back();
            }
        }
        self.sva_sites[site_idx].past_names = names;
    }

    /// Replace each plain signal read in a concurrent assertion's body with
    /// an `SVA_SIG_MARKER` read of its signal id, resolved once in the
    /// registering scope: the generic identifier path re-ran its class,
    /// local and interface probes on every clock of every assertion. The
    /// argument of a sampled-value function stays a name (its history is
    /// keyed by name), as does the base of a select.
    pub(super) fn sva_resolve_signals(&self, e: &mut Expression) {
        match &mut e.kind {
            ExprKind::Ident(h) => {
                if h.root.is_some() || h.path.iter().any(|s| !s.selects.is_empty()) {
                    return;
                }
                let name = self.resolve_hier_name(h).into_owned();
                let Some(&id) = self.signal_name_to_id.get(name.as_str()) else {
                    return;
                };
                if self.module.arrays.contains_key(&name)
                    || self.module.arrays_2d.contains_key(&name)
                    || self.module.arrays_nd.contains_key(&name)
                    || self.module.dynamic_arrays.contains(&name)
                    || self.module.associative_arrays.contains_key(&name)
                    || self.string_signals.contains(&name)
                {
                    return;
                }
                let span = e.span;
                *e = Expression::new(
                    ExprKind::SystemCall {
                        name: SVA_SIG_MARKER.to_string(),
                        args: vec![Expression::new(
                            ExprKind::Number(NumberLiteral::Integer {
                                size: None,
                                signed: false,
                                base: NumberBase::Decimal,
                                value: id.to_string(),
                                cached_val: Cell::new(None),
                            }),
                            span,
                        )],
                    },
                    span,
                );
            }
            ExprKind::SystemCall { name, args } => {
                if matches!(
                    name.as_str(),
                    "$past" | "$rose" | "$fell" | "$stable" | "$changed" | "$sampled"
                ) {
                    return;
                }
                // The width or type of a cast (`W'(x)`) is read by name.
                let skip = usize::from(matches!(
                    name.as_str(),
                    "$__xz_size_cast" | "$__xz_named_cast" | "$__xz_type_cast"
                ));
                for a in args.iter_mut().skip(skip) {
                    self.sva_resolve_signals(a);
                }
            }
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                self.sva_resolve_signals(operand)
            }
            ExprKind::Binary { left, right, .. } => {
                self.sva_resolve_signals(left);
                self.sva_resolve_signals(right);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.sva_resolve_signals(condition);
                self.sva_resolve_signals(then_expr);
                self.sva_resolve_signals(else_expr);
            }
            ExprKind::Concatenation(parts) => {
                for p in parts {
                    self.sva_resolve_signals(p);
                }
            }
            ExprKind::Index { index, .. } => self.sva_resolve_signals(index),
            ExprKind::SvaClocked { .. } => {}
            _ => {}
        }
    }

    /// The value behind an `SVA_SIG_MARKER` read.
    pub(super) fn sva_sig_read(&self, args: &[Expression]) -> Value {
        if args.len() == 2 {
            return self.sva_lv_read(args);
        }
        let id = match args.first().map(|a| &a.kind) {
            Some(ExprKind::Number(NumberLiteral::Integer { value, .. })) => {
                value.parse::<usize>().unwrap_or(usize::MAX)
            }
            _ => usize::MAX,
        };
        let Some(v) = self.signal_table.get(id) else {
            return Value::new(1);
        };
        let mut v = v.clone();
        if self.signal_signed[id] {
            v.is_signed = true;
        }
        if self.signal_real[id] {
            v.is_real = true;
        }
        v
    }

    /// The width of an `SVA_SIG_MARKER` read: its signal's declared width, or
    /// for a local variable (§16.10) its slot's.
    pub(super) fn sva_sig_width(&self, args: &[Expression]) -> Option<u32> {
        let k = match &args.first()?.kind {
            ExprKind::Number(NumberLiteral::Integer { value, .. }) => {
                value.parse::<usize>().ok()?
            }
            _ => return None,
        };
        if args.len() == 2 {
            return self.sva_lv.get(k).map(|v| v.width);
        }
        self.signal_widths.get(k).copied()
    }

    /// §16.10: a local variable read (a two-argument `SVA_SIG_MARKER`: the
    /// slot, then 1) from the thread being evaluated.
    #[inline(never)]
    pub(super) fn sva_lv_read(&self, args: &[Expression]) -> Value {
        let k = match &args[0].kind {
            ExprKind::Number(NumberLiteral::Integer { value, .. }) => {
                value.parse::<usize>().unwrap_or(usize::MAX)
            }
            _ => usize::MAX,
        };
        self.sva_lv.get(k).cloned().unwrap_or_else(|| Value::new(1))
    }

    /// The marker read of local variable slot `k` (see `sva_lv_read`).
    pub(super) fn sva_lv_marker(k: usize, span: crate::ast::Span) -> Expression {
        let num = |v: usize| {
            Expression::new(
                ExprKind::Number(NumberLiteral::Integer {
                    size: None,
                    signed: false,
                    base: NumberBase::Decimal,
                    value: v.to_string(),
                    cached_val: Cell::new(None),
                }),
                span,
            )
        };
        Expression::new(
            ExprKind::SystemCall {
                name: SVA_SIG_MARKER.to_string(),
                args: vec![num(k), num(1)],
            },
            span,
        )
    }

    /// §16.10: give each local variable declared in `e` (by a property or a
    /// sequence instance: `$sva_locals(<decl>..., <body>)`) its own slot, and
    /// turn every reference to it in its scope into a slot marker read. The
    /// declarations become initialising assignments (dropped when there are
    /// none); the slot types go to `sva_lv_types`, their initial values to
    /// `init`.
    pub(super) fn sva_bind_locals(
        &self,
        e: &mut Expression,
        names: &HashMap<String, usize>,
        init: &mut Vec<Value>,
    ) -> Vec<(u32, bool, bool)> {
        let mut types = Vec::new();
        self.sva_bind_locals_in(e, names, init, &mut types);
        types
    }

    pub(super) fn sva_bind_locals_in(
        &self,
        e: &mut Expression,
        names: &HashMap<String, usize>,
        init: &mut Vec<Value>,
        types: &mut Vec<(u32, bool, bool)>,
    ) {
        let span = e.span;
        match &mut e.kind {
            ExprKind::SystemCall { name, args } if name == "$sva_locals" && !args.is_empty() => {
                let mut body = args.pop().unwrap();
                let mut scope = names.clone();
                let mut assigns: Vec<Expression> = Vec::new();
                for decl in args.iter() {
                    let ExprKind::SystemCall { args: d, .. } = &decl.kind else {
                        continue;
                    };
                    let (Some(ExprKind::TypeLiteral(dt)), Some(ExprKind::Ident(h))) =
                        (d.first().map(|x| &x.kind), d.get(1).map(|x| &x.kind))
                    else {
                        continue;
                    };
                    let Some(seg) = h.path.last() else {
                        continue;
                    };
                    let w = super::elaborate::resolve_type_width(
                        dt,
                        Some(&self.module.parameters),
                        Some(&self.module.typedefs),
                    )
                    .max(1);
                    let signed = self.type_is_signed_concrete(dt);
                    let two_state = super::elaborate::is_type_two_state(dt);
                    let k = init.len();
                    let mut v = if two_state {
                        Value::zero(w)
                    } else {
                        Value::new(w)
                    };
                    v.is_signed = signed;
                    init.push(v);
                    types.push((w, signed, two_state));
                    if let Some(iv) = d.get(2) {
                        let mut iv = iv.clone();
                        self.sva_bind_locals_in(&mut iv, &scope, init, types);
                        assigns.push(Expression::new(
                            ExprKind::AssignExpr {
                                lvalue: Box::new(Self::sva_lv_marker(k, decl.span)),
                                rvalue: Box::new(iv),
                            },
                            decl.span,
                        ));
                    }
                    scope.insert(seg.name.name.clone(), k);
                }
                self.sva_bind_locals_in(&mut body, &scope, init, types);
                if assigns.is_empty() {
                    *e = body;
                } else {
                    assigns.push(body);
                    *args = assigns;
                }
            }
            ExprKind::Ident(h) => {
                if names.is_empty() || h.root.is_some() || h.path.len() != 1 {
                    return;
                }
                let Some(&k) = names.get(h.path[0].name.name.as_str()) else {
                    return;
                };
                let selects = std::mem::take(&mut h.path[0].selects);
                let mut out = Self::sva_lv_marker(k, span);
                for mut sel in selects {
                    self.sva_bind_locals_in(&mut sel, names, init, types);
                    out = match sel.kind {
                        ExprKind::Range(l, r) => Expression::new(
                            ExprKind::RangeSelect {
                                expr: Box::new(out),
                                kind: crate::ast::expr::RangeKind::Constant,
                                left: l,
                                right: r,
                            },
                            span,
                        ),
                        _ => Expression::new(
                            ExprKind::Index {
                                expr: Box::new(out),
                                index: Box::new(sel),
                            },
                            span,
                        ),
                    };
                }
                *e = out;
            }
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                self.sva_bind_locals_in(operand, names, init, types)
            }
            ExprKind::Binary { left, right, .. }
            | ExprKind::AssignExpr {
                lvalue: left,
                rvalue: right,
            }
            | ExprKind::Range(left, right) => {
                self.sva_bind_locals_in(left, names, init, types);
                self.sva_bind_locals_in(right, names, init, types);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.sva_bind_locals_in(condition, names, init, types);
                self.sva_bind_locals_in(then_expr, names, init, types);
                self.sva_bind_locals_in(else_expr, names, init, types);
            }
            ExprKind::Call { args, .. }
            | ExprKind::SystemCall { args, .. }
            | ExprKind::Concatenation(args) => {
                for a in args {
                    self.sva_bind_locals_in(a, names, init, types);
                }
            }
            ExprKind::Replication { count, exprs } => {
                self.sva_bind_locals_in(count, names, init, types);
                for a in exprs {
                    self.sva_bind_locals_in(a, names, init, types);
                }
            }
            ExprKind::Index { expr, index } => {
                self.sva_bind_locals_in(expr, names, init, types);
                self.sva_bind_locals_in(index, names, init, types);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                self.sva_bind_locals_in(expr, names, init, types);
                self.sva_bind_locals_in(left, names, init, types);
                self.sva_bind_locals_in(right, names, init, types);
            }
            ExprKind::Inside { expr, ranges } => {
                self.sva_bind_locals_in(expr, names, init, types);
                for r in ranges {
                    self.sva_bind_locals_in(r, names, init, types);
                }
            }
            ExprKind::MemberAccess { expr, .. } => {
                self.sva_bind_locals_in(expr, names, init, types)
            }
            ExprKind::SvaClocked { body, .. } => self.sva_bind_locals_in(body, names, init, types),
            _ => {}
        }
    }

    /// §16.14.6.1: the automatic variables of the running procedure that a
    /// procedural concurrent assertion reads become slots too, filled with
    /// their values each time the statement executes (`captured`).
    pub(super) fn sva_capture_proc_locals(
        &self,
        e: &mut Expression,
        captured: &mut Vec<(usize, String)>,
        init: &mut Vec<Value>,
        types: &mut Vec<(u32, bool, bool)>,
    ) {
        let span = e.span;
        match &mut e.kind {
            ExprKind::Ident(h) => {
                if h.root.is_some() || h.path.len() != 1 {
                    return;
                }
                let name = h.path[0].name.name.clone();
                let k = if let Some((k, _)) = captured.iter().find(|(_, n)| *n == name) {
                    *k
                } else {
                    let Some(v) = self.local_stack.last().and_then(|m| m.get(name.as_str())) else {
                        return;
                    };
                    let k = init.len();
                    init.push(v.clone());
                    types.push((v.width, v.is_signed, false));
                    captured.push((k, name));
                    k
                };
                let selects = std::mem::take(&mut h.path[0].selects);
                let mut out = Self::sva_lv_marker(k, span);
                for mut sel in selects {
                    self.sva_capture_proc_locals(&mut sel, captured, init, types);
                    out = match sel.kind {
                        ExprKind::Range(l, r) => Expression::new(
                            ExprKind::RangeSelect {
                                expr: Box::new(out),
                                kind: crate::ast::expr::RangeKind::Constant,
                                left: l,
                                right: r,
                            },
                            span,
                        ),
                        _ => Expression::new(
                            ExprKind::Index {
                                expr: Box::new(out),
                                index: Box::new(sel),
                            },
                            span,
                        ),
                    };
                }
                *e = out;
            }
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                self.sva_capture_proc_locals(operand, captured, init, types)
            }
            ExprKind::Binary { left, right, .. }
            | ExprKind::AssignExpr {
                lvalue: left,
                rvalue: right,
            }
            | ExprKind::Range(left, right) => {
                self.sva_capture_proc_locals(left, captured, init, types);
                self.sva_capture_proc_locals(right, captured, init, types);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.sva_capture_proc_locals(condition, captured, init, types);
                self.sva_capture_proc_locals(then_expr, captured, init, types);
                self.sva_capture_proc_locals(else_expr, captured, init, types);
            }
            ExprKind::Call { args, .. }
            | ExprKind::SystemCall { args, .. }
            | ExprKind::Concatenation(args) => {
                for a in args {
                    self.sva_capture_proc_locals(a, captured, init, types);
                }
            }
            ExprKind::Index { expr, index } => {
                self.sva_capture_proc_locals(expr, captured, init, types);
                self.sva_capture_proc_locals(index, captured, init, types);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                self.sva_capture_proc_locals(expr, captured, init, types);
                self.sva_capture_proc_locals(left, captured, init, types);
                self.sva_capture_proc_locals(right, captured, init, types);
            }
            ExprKind::Inside { expr, ranges } => {
                self.sva_capture_proc_locals(expr, captured, init, types);
                for r in ranges {
                    self.sva_capture_proc_locals(r, captured, init, types);
                }
            }
            _ => {}
        }
    }

    /// §16.15: the `default disable iff` that governs an assertion in
    /// `scope` — declared there or in an enclosing generate scope of the same
    /// module instance.
    pub(super) fn sva_default_disable_for(&self, scope: &str) -> Option<Expression> {
        if self.sva_default_disable.is_empty() {
            return None;
        }
        let top = self.module.name.as_str();
        let mut cur = scope;
        loop {
            if let Some(g) = self.sva_default_disable.get(cur) {
                return Some(g.clone());
            }
            // A module instance's default does not reach its children.
            let rel = cur
                .strip_prefix(top)
                .map(|r| r.trim_start_matches('.'))
                .unwrap_or(cur);
            if rel.is_empty() || self.module.instances.iter().any(|i| i.path == rel) {
                return None;
            }
            match cur.rfind('.') {
                Some(i) => cur = &cur[..i],
                None => return None,
            }
        }
    }

    /// Walk `expr` and append every bare signal name referenced by a
    /// `$past(<signal>[, N])` system call to `out`. Used to refresh
    /// per-site snapshots once per clock fire.
    pub(super) fn collect_past_arg_names(&self, expr: &Expression, out: &mut Vec<String>) {
        match &expr.kind {
            ExprKind::SystemCall { name, args }
                if matches!(
                    name.as_str(),
                    "$past" | "$rose" | "$fell" | "$stable" | "$changed"
                ) =>
            {
                if let Some(arg) = args.first() {
                    if let ExprKind::Ident(h) = &arg.kind {
                        out.push(self.resolve_hier_name(h).into_owned());
                    }
                }
                for a in args {
                    self.collect_past_arg_names(a, out);
                }
            }
            ExprKind::SystemCall { args, .. } => {
                for a in args {
                    self.collect_past_arg_names(a, out);
                }
            }
            ExprKind::Binary { left, right, .. } => {
                self.collect_past_arg_names(left, out);
                self.collect_past_arg_names(right, out);
            }
            ExprKind::Unary { operand, .. } => {
                self.collect_past_arg_names(operand, out);
            }
            ExprKind::Paren(inner) => {
                self.collect_past_arg_names(inner, out);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.collect_past_arg_names(condition, out);
                self.collect_past_arg_names(then_expr, out);
                self.collect_past_arg_names(else_expr, out);
            }
            ExprKind::Call { args, .. } => {
                for a in args {
                    self.collect_past_arg_names(a, out);
                }
            }
            ExprKind::SvaClocked { body, .. } => {
                self.collect_past_arg_names(body, out);
            }
            _ => {}
        }
    }
}
