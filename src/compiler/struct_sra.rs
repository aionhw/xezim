//! Scalar replacement of unpacked-struct values in a pure function, so that a
//! member select of its result (`f(args).m`) inlines into bytecode.
//!
//! §6.6.7 resolution functions of struct nettypes (electrical-equivalent
//! V/I/R nets) take `input T drivers[]` and return a `T`. The nettype pass
//! emits one continuous assign per member, `n.m = res('{...}).m`, and none of
//! them compiled: the inliner binds only scalar formals, scalar-element
//! dynamic-array formals and scalar locals. Every settle re-ran the resolver
//! on the AST path, three times per node.
//!
//! The rewrite turns the function into an equivalent one over scalars, for
//! one selected member of the result:
//!
//! ```text
//! function T f(input T d[]);        function real f(input real d__V[],
//!   T r;                                             input real d__I[], ...);
//!   foreach (d[k]) g += d[k].R;       real r__V; real r__I; real r__R;
//!   r.V = g;                    =>    foreach (d__V[k]) g += d__R[k];
//!   return r;                         r__V = g;
//! endfunction                         return r__V;      // for `.V`
//! ```
//!
//! Every construct it does not recognize makes it give up (`None`), so the
//! caller keeps the existing call path; it never changes what a recognized
//! body computes.

use crate::ast::decl::{FunctionDeclaration, FunctionPort};
use crate::ast::expr::*;
use crate::ast::stmt::*;
use crate::ast::types::{DataType, PortDirection, UnpackedDimension};
use crate::ast::{Identifier, Span};
use xezim_core::hasher::HashMap;

/// Members of an unpacked struct whose members are all scalars, in
/// declaration order: (name, type).
pub(crate) type Members = Vec<(String, DataType)>;

enum Kind {
    /// `input T d[]`: one dynamic-array formal per member, with the element
    /// count of the call site's actual.
    Array(Members, usize),
    /// A struct formal or local: one scalar per member.
    Scalar(Members),
}

#[derive(Clone, Copy)]
enum ReturnMode<'a> {
    Single(&'a str),
    Multi,
}

struct Ctx<'a> {
    names: HashMap<String, Kind>,
    /// The function's own name, its implicit result variable (§13.4.1).
    fname: &'a str,
    ret: &'a Members,
    mode: ReturnMode<'a>,
    replication_allowed: &'a dyn Fn(&Members) -> bool,
}

pub(crate) struct ScalarizedMultiCall {
    pub fd: FunctionDeclaration,
    pub args: Vec<Expression>,
    /// `(dotted_member_path, scalar_var_name, member_data_type)` in declaration order.
    pub outputs: Vec<(String, String, DataType)>,
}

fn leaf(base: &str, m: &str) -> String {
    let mut out = String::with_capacity(base.len() + 2 + m.len());
    out.push_str(base);
    out.push_str("__");
    for ch in m.chars() {
        if ch == '.' {
            out.push_str("__");
        } else {
            out.push(ch);
        }
    }
    out
}

fn ident(name: &str, span: Span) -> Expression {
    Expression::new(
        ExprKind::Ident(HierarchicalIdentifier {
            root: None,
            path: vec![HierPathSegment {
                name: Identifier {
                    name: name.to_string(),
                    span,
                },
                selects: Vec::new(),
            }],
            span,
            cached_signal_id: crate::ast::cache::SignalCache::new(None),
            cached_resolved_name: crate::ast::cache::NameCache::new(),
            bound: crate::ast::binding::Bound::UNSET,
        }),
        span,
    )
}

/// A bare, select-free identifier's name.
fn bare_name(e: &Expression) -> Option<&str> {
    match &e.kind {
        ExprKind::Ident(h)
            if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty() =>
        {
            Some(h.path[0].name.name.as_str())
        }
        _ => None,
    }
}

/// True when evaluating `e` has no effect: names, literals and operators over
/// them. A call, an increment or an assignment may, so a part holding one
/// must not be dropped or evaluated once per member (§13.5).
fn effect_free(e: &Expression) -> bool {
    match &e.kind {
        ExprKind::Number(_) | ExprKind::StringLiteral(_) => true,
        ExprKind::Ident(h) => h.path.iter().all(|s| s.selects.iter().all(effect_free)),
        ExprKind::Paren(x) => effect_free(x),
        ExprKind::Unary { op, operand } => {
            !matches!(
                op,
                UnaryOp::PreIncr | UnaryOp::PreDecr | UnaryOp::PostIncr | UnaryOp::PostDecr
            ) && effect_free(operand)
        }
        ExprKind::Binary { op, left, right } => {
            *op != BinaryOp::Assign && effect_free(left) && effect_free(right)
        }
        ExprKind::Conditional {
            condition,
            then_expr,
            else_expr,
        } => effect_free(condition) && effect_free(then_expr) && effect_free(else_expr),
        ExprKind::Concatenation(v) => v.iter().all(effect_free),
        ExprKind::MemberAccess { expr, .. } => effect_free(expr),
        ExprKind::Index { expr, index } => effect_free(expr) && effect_free(index),
        ExprKind::RangeSelect {
            expr, left, right, ..
        } => effect_free(expr) && effect_free(left) && effect_free(right),
        _ => false,
    }
}

enum MemberGroup {
    Leaf(String),
    Sub(String, Members),
}

fn group_members(members: &[(String, DataType)]) -> Vec<MemberGroup> {
    let mut groups: Vec<MemberGroup> = Vec::new();
    for (path, dt) in members {
        if let Some((head, tail)) = path.split_once('.') {
            if let Some(MemberGroup::Sub(prev, list)) = groups.last_mut()
                && prev == head
            {
                list.push((tail.to_string(), dt.clone()));
            } else {
                groups.push(MemberGroup::Sub(
                    head.to_string(),
                    vec![(tail.to_string(), dt.clone())],
                ));
            }
        } else {
            groups.push(MemberGroup::Leaf(path.clone()));
        }
    }
    groups
}

/// Build an expression accessing dotted `path` on effect-free `base`.
fn member_chain_expr(base: &Expression, path: &str) -> Expression {
    if let ExprKind::Ident(h) = &base.kind
        && h.root.is_none()
        && h.path.iter().all(|s| s.selects.is_empty())
    {
        let mut new_path = h.path.clone();
        for seg in path.split('.') {
            new_path.push(HierPathSegment {
                name: Identifier {
                    name: seg.to_string(),
                    span: base.span,
                },
                selects: Vec::new(),
            });
        }
        return Expression::new(
            ExprKind::Ident(HierarchicalIdentifier {
                root: None,
                path: new_path,
                span: base.span,
                cached_signal_id: crate::ast::cache::SignalCache::new(None),
                cached_resolved_name: crate::ast::cache::NameCache::new(),
                bound: crate::ast::binding::Bound::UNSET,
            }),
            base.span,
        );
    }
    let mut cur = base.clone();
    for seg in path.split('.') {
        cur = Expression::new(
            ExprKind::MemberAccess {
                expr: Box::new(cur),
                member: Identifier {
                    name: seg.to_string(),
                    span: base.span,
                },
            },
            base.span,
        );
    }
    cur
}

/// One expression per leaf member from a struct-valued actual or item (supports
/// nested unpacked structs recursively): an ordered, named, replicated, or
/// default pattern is taken apart, anything else is member-selected (only when
/// effect-free so it may be evaluated per leaf cleanly).
fn split_value(
    e: &Expression,
    members: &[(String, DataType)],
    replication_allowed: bool,
) -> Option<Vec<Expression>> {
    if members.is_empty() {
        return None;
    }
    let groups = group_members(members);
    if let ExprKind::AssignmentPattern(items) = &e.peel_typed_pattern().kind {
        let mut top_exprs: Vec<Expression> = Vec::with_capacity(groups.len());
        if replication_allowed
            && let Some(parts) =
                xezim_core::elaborate::struct_pattern_replication(items, groups.len())
        {
            top_exprs.extend(parts.into_iter().cloned());
        } else if items
            .iter()
            .all(|it| matches!(it, AssignmentPatternItem::Ordered(_)))
        {
            if items.len() != groups.len() {
                return None;
            }
            top_exprs.extend(items.iter().map(|it| it.expr().clone()));
        } else {
            let mut default_expr: Option<&Expression> = None;
            let group_names: Vec<&str> = groups
                .iter()
                .map(|g| match g {
                    MemberGroup::Leaf(n) | MemberGroup::Sub(n, _) => n.as_str(),
                })
                .collect();
            for it in items {
                match it {
                    AssignmentPatternItem::Default(v) => default_expr = Some(v),
                    AssignmentPatternItem::Named(id, _) => {
                        if !group_names.contains(&id.ident.name.as_str()) {
                            return None;
                        }
                    }
                    _ => return None,
                }
            }
            for gname in group_names {
                let mut hit: Option<Expression> = None;
                for it in items {
                    if let AssignmentPatternItem::Named(id, v) = it
                        && id.ident.name == gname
                    {
                        hit = Some(v.clone());
                    }
                }
                top_exprs.push(hit.or_else(|| default_expr.cloned())?);
            }
        }
        let mut out = Vec::with_capacity(members.len());
        for (g, expr) in groups.iter().zip(&top_exprs) {
            match g {
                MemberGroup::Leaf(_) => out.push(expr.clone()),
                MemberGroup::Sub(_, sub_ms) => {
                    out.extend(split_value(expr, sub_ms, replication_allowed)?);
                }
            }
        }
        return Some(out);
    }
    if !effect_free(e) {
        return None;
    }
    Some(
        members
            .iter()
            .map(|(m, _)| member_chain_expr(e, m))
            .collect(),
    )
}

fn expr_mentions_prefix(e: &Expression, prefix: &str, fname: &str) -> bool {
    match &e.kind {
        ExprKind::Ident(h) => h.path.iter().any(|s| {
            let n = s.name.name.as_str();
            n == fname || n.starts_with(prefix)
        }) || h.path.iter().any(|s| {
            s.selects
                .iter()
                .any(|sel| expr_mentions_prefix(sel, prefix, fname))
        }),
        ExprKind::Paren(x) | ExprKind::Unary { operand: x, .. } => {
            expr_mentions_prefix(x, prefix, fname)
        }
        ExprKind::Binary { left, right, .. } => {
            expr_mentions_prefix(left, prefix, fname) || expr_mentions_prefix(right, prefix, fname)
        }
        ExprKind::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            expr_mentions_prefix(condition, prefix, fname)
                || expr_mentions_prefix(then_expr, prefix, fname)
                || expr_mentions_prefix(else_expr, prefix, fname)
        }
        ExprKind::Index { expr, index } => {
            expr_mentions_prefix(expr, prefix, fname) || expr_mentions_prefix(index, prefix, fname)
        }
        ExprKind::RangeSelect {
            expr, left, right, ..
        } => {
            expr_mentions_prefix(expr, prefix, fname)
                || expr_mentions_prefix(left, prefix, fname)
                || expr_mentions_prefix(right, prefix, fname)
        }
        ExprKind::MemberAccess { expr, .. } => expr_mentions_prefix(expr, prefix, fname),
        ExprKind::Concatenation(xs) | ExprKind::SystemCall { args: xs, .. } => {
            xs.iter().any(|x| expr_mentions_prefix(x, prefix, fname))
        }
        ExprKind::Replication { count, exprs } => {
            expr_mentions_prefix(count, prefix, fname)
                || exprs.iter().any(|x| expr_mentions_prefix(x, prefix, fname))
        }
        ExprKind::Call { func, args } => {
            expr_mentions_prefix(func, prefix, fname)
                || args.iter().any(|x| expr_mentions_prefix(x, prefix, fname))
        }
        _ => false,
    }
}

impl Ctx<'_> {
    fn members_of(&self, n: &str) -> Option<&Members> {
        if n == self.fname {
            return Some(self.ret);
        }
        match self.names.get(n)? {
            Kind::Scalar(m) => Some(m),
            Kind::Array(..) => None,
        }
    }

    fn sub_members_of(&self, n: &str, prefix: &str) -> Option<Members> {
        let ms = self.members_of(n)?;
        let pfx = format!("{prefix}.");
        let sub: Members = ms
            .iter()
            .filter_map(|(m, dt)| m.strip_prefix(&pfx).map(|r| (r.to_string(), dt.clone())))
            .collect();
        (!sub.is_empty()).then_some(sub)
    }

    /// Peel nested `MemberAccess` (and any select-free multi-segment `Ident`)
    /// into `(root_expr, "m1.m2....")`.
    fn peel_member_chain<'e>(mut e: &'e Expression) -> (&'e Expression, Vec<&'e str>) {
        let mut chain = Vec::new();
        while let ExprKind::MemberAccess { expr, member } = &e.kind {
            chain.push(member.name.as_str());
            e = expr;
        }
        chain.reverse();
        (e, chain)
    }

    /// If `e` names `(struct_var, dotted_path_opt)` belonging to `self.names`/`fname`.
    fn struct_target_path(&self, e: &Expression) -> Option<(String, Option<String>)> {
        let (base, chain) = Self::peel_member_chain(e);
        match &base.kind {
            ExprKind::Ident(h) if h.root.is_none() && h.path.iter().all(|s| s.selects.is_empty()) => {
                let first = h.path.first()?.name.name.as_str();
                if self.members_of(first).is_none() {
                    return None;
                }
                let mut segs: Vec<&str> = h.path[1..].iter().map(|s| s.name.name.as_str()).collect();
                segs.extend(chain);
                if segs.is_empty() {
                    Some((first.to_string(), None))
                } else {
                    Some((first.to_string(), Some(segs.join("."))))
                }
            }
            _ => None,
        }
    }

    /// The scalar that holds member `m` of struct name `n`.
    fn scalar_of(&self, n: &str, m: &str) -> String {
        match self.mode {
            ReturnMode::Single(sel) if n == self.fname && m == sel => n.to_string(),
            _ => leaf(n, m),
        }
    }

    fn is_struct_name(&self, n: &str) -> bool {
        n == self.fname || self.names.contains_key(n)
    }

    fn expr(&self, e: &mut Expression) -> Option<()> {
        let span = e.span;
        let replace: Option<Expression> = match &mut e.kind {
            ExprKind::MemberAccess {
                expr: direct_base,
                member,
            } => {
                let outer_member = member.name.as_str();
                let (root, chain) = Self::peel_member_chain(direct_base);
                let mut full_chain = chain;
                full_chain.push(outer_member);
                if let ExprKind::Ident(h) = &root.kind
                    && h.root.is_none()
                    && h.path.iter().all(|s| s.selects.is_empty())
                {
                    let n = h.path[0].name.name.as_str();
                    if let Some(ms) = self.members_of(n) {
                        let mut segs: Vec<&str> =
                            h.path[1..].iter().map(|s| s.name.name.as_str()).collect();
                        segs.extend(full_chain);
                        let joined = segs.join(".");
                        ms.iter().any(|(m, _)| *m == joined).then_some(())?;
                        Some(ident(&self.scalar_of(n, &joined), span))
                    } else if self.is_struct_name(n) {
                        return None; // a member of the whole array
                    } else {
                        self.expr(direct_base)?;
                        None
                    }
                } else if let ExprKind::Index { expr: arr, index } = &root.kind {
                    match bare_name(arr).and_then(|n| self.names.get(n).map(|k| (n, k))) {
                        Some((n, Kind::Array(ms, _))) => {
                            let joined = full_chain.join(".");
                            ms.iter().any(|(m, _)| *m == joined).then_some(())?;
                            let n = n.to_string();
                            let mut idx = index.clone();
                            self.expr(&mut idx)?;
                            Some(Expression::new(
                                ExprKind::Index {
                                    expr: Box::new(ident(&leaf(&n, &joined), arr.span)),
                                    index: idx,
                                },
                                span,
                            ))
                        }
                        Some(_) => return None,
                        None => {
                            self.expr(direct_base)?;
                            None
                        }
                    }
                } else {
                    self.expr(direct_base)?;
                    None
                }
            }
            // `d.size()` / `d.num()`: the call site fixes the element count
            // (§7.5.2), so it folds to a constant.
            ExprKind::Call { func, args } => {
                if let ExprKind::MemberAccess { expr: base, member } = &func.kind
                    && let Some(n) = bare_name(base)
                    && let Some(Kind::Array(_, len)) = self.names.get(n)
                {
                    if !matches!(member.name.as_str(), "size" | "num") || !args.is_empty() {
                        return None;
                    }
                    Some(Expression::new(
                        ExprKind::Number(NumberLiteral::Integer {
                            size: None,
                            signed: true,
                            base: NumberBase::Decimal,
                            value: len.to_string(),
                            cached_val: crate::ast::cache::LiteralCache::new(None),
                        }),
                        span,
                    ))
                } else {
                    self.expr(func)?;
                    for a in args {
                        self.expr(a)?;
                    }
                    None
                }
            }
            ExprKind::Ident(h) => {
                let n = h.path[0].name.name.as_str();
                if h.root.is_none() && self.is_struct_name(n) {
                    if h.path.len() >= 2
                        && h.path.iter().all(|s| s.selects.is_empty())
                        && let Some(ms) = self.members_of(n)
                    {
                        let joined = h.path[1..]
                            .iter()
                            .map(|s| s.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join(".");
                        if ms.iter().any(|(m, _)| *m == joined) {
                            Some(ident(&self.scalar_of(n, &joined), span))
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                } else {
                    for s in &mut h.path {
                        for sel in &mut s.selects {
                            self.expr(sel)?;
                        }
                    }
                    None
                }
            }
            ExprKind::Number(_) | ExprKind::StringLiteral(_) => None,
            ExprKind::Unary { operand, .. } => {
                self.expr(operand)?;
                None
            }
            ExprKind::Binary { left, right, .. } => {
                self.expr(left)?;
                self.expr(right)?;
                None
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.expr(condition)?;
                self.expr(then_expr)?;
                self.expr(else_expr)?;
                None
            }
            ExprKind::Paren(inner) => {
                self.expr(inner)?;
                None
            }
            ExprKind::Index { expr: a, index } => {
                if bare_name(a).is_some_and(|n| self.is_struct_name(n)) {
                    return None; // a whole struct element
                }
                self.expr(a)?;
                self.expr(index)?;
                None
            }
            ExprKind::RangeSelect {
                expr: a,
                left,
                right,
                ..
            } => {
                self.expr(a)?;
                self.expr(left)?;
                self.expr(right)?;
                None
            }
            ExprKind::Concatenation(xs) => {
                for x in xs {
                    self.expr(x)?;
                }
                None
            }
            ExprKind::Replication { count, exprs } => {
                self.expr(count)?;
                for x in exprs {
                    self.expr(x)?;
                }
                None
            }
            ExprKind::SystemCall { args, .. } => {
                for a in args {
                    self.expr(a)?;
                }
                None
            }
            _ => return None,
        };
        if let Some(r) = replace {
            *e = r;
        }
        Some(())
    }

    /// Statements, possibly expanding one into several.
    fn stmts(&self, list: &[Statement]) -> Option<Vec<Statement>> {
        let mut out = Vec::with_capacity(list.len());
        for s in list {
            out.extend(self.stmt(s)?);
        }
        Some(out)
    }

    fn one(&self, s: &Statement) -> Option<Box<Statement>> {
        let mut v = self.stmt(s)?;
        Some(Box::new(if v.len() == 1 {
            v.pop().unwrap()
        } else {
            Statement::new(
                StatementKind::SeqBlock {
                    name: None,
                    stmts: v.into(),
                },
                s.span,
            )
        }))
    }

    fn assign(&self, lhs: String, rhs: Expression, span: Span) -> Statement {
        Statement::new(
            StatementKind::BlockingAssign {
                lvalue: ident(&lhs, span),
                rvalue: rhs,
            },
            span,
        )
    }

    fn stmt(&self, s: &Statement) -> Option<Vec<Statement>> {
        let span = s.span;
        let mk = |k| vec![Statement::new(k, span)];
        Some(match &s.kind {
            StatementKind::Null => vec![s.clone()],
            StatementKind::SeqBlock { name, stmts } => mk(StatementKind::SeqBlock {
                name: name.clone(),
                stmts: self.stmts(stmts)?.into(),
            }),
            StatementKind::VarDecl {
                data_type,
                lifetime,
                declarators,
            } => {
                let mut out = Vec::new();
                for d in declarators {
                    if let Some(ms) = self.members_of(&d.name.name) {
                        let ms = ms.clone();
                        let inits = match &d.init {
                            Some(e) => Some(split_value(e, &ms, (self.replication_allowed)(&ms))?),
                            None => None,
                        };
                        for (i, (m, mdt)) in ms.iter().enumerate() {
                            let init = match &inits {
                                Some(v) => {
                                    let mut e = v[i].clone();
                                    self.expr(&mut e)?;
                                    Some(e)
                                }
                                None => None,
                            };
                            out.push(Statement::new(
                                StatementKind::VarDecl {
                                    data_type: mdt.clone(),
                                    lifetime: *lifetime,
                                    declarators: vec![VarDeclarator {
                                        decl: crate::ast::binding::Bound::UNSET,
                                        name: Identifier {
                                            name: leaf(&d.name.name, m),
                                            span: d.name.span,
                                        },
                                        dimensions: Vec::new(),
                                        init: init.map(Box::new),
                                        span: d.span,
                                    }],
                                },
                                span,
                            ));
                        }
                    } else {
                        let mut d = d.clone();
                        if let Some(e) = &mut d.init {
                            self.expr(e)?;
                        }
                        out.push(Statement::new(
                            StatementKind::VarDecl {
                                data_type: data_type.clone(),
                                lifetime: *lifetime,
                                declarators: vec![d],
                            },
                            span,
                        ));
                    }
                }
                out
            }
            StatementKind::ParamDecl {
                is_local,
                data_type,
                declarators,
            } => {
                let mut out_decls = Vec::with_capacity(declarators.len());
                for d in declarators {
                    let mut d = d.clone();
                    if let Some(e) = &mut d.init {
                        self.expr(e)?;
                    }
                    out_decls.push(d);
                }
                vec![Statement::new(
                    StatementKind::ParamDecl {
                        is_local: *is_local,
                        data_type: data_type.clone(),
                        declarators: out_decls,
                    },
                    span,
                )]
            }
            StatementKind::BlockingAssign { lvalue, rvalue } => {
                let target_struct = self.struct_target_path(lvalue).and_then(|(n, pfx_opt)| {
                    match pfx_opt {
                        None => Some((n.clone(), None, self.members_of(&n)?.clone())),
                        Some(pfx) => {
                            let ms = self.members_of(&n)?;
                            if ms.iter().any(|(m, _)| m == &pfx) {
                                None
                            } else {
                                Some((n.clone(), Some(pfx.clone()), self.sub_members_of(&n, &pfx)?))
                            }
                        }
                    }
                });
                if let Some((n, pfx_opt, ms)) = target_struct {
                    let dst_name = |m: &str| match &pfx_opt {
                        None => self.scalar_of(&n, m),
                        Some(p) => self.scalar_of(&n, &format!("{p}.{m}")),
                    };
                    let src_struct = self.struct_target_path(rvalue).and_then(|(r, rpfx)| {
                        let rms = match &rpfx {
                            None => self.members_of(&r)?.clone(),
                            Some(rp) => self.sub_members_of(&r, rp)?,
                        };
                        (rms.len() == ms.len() && rms.iter().zip(&ms).all(|(a, b)| a.0 == b.0))
                            .then_some((r, rpfx))
                    });
                    let src: Vec<Expression> = match src_struct {
                        Some((r, rpfx)) => ms
                            .iter()
                            .map(|(m, _)| {
                                let sm = match &rpfx {
                                    None => self.scalar_of(&r, m),
                                    Some(rp) => self.scalar_of(&r, &format!("{rp}.{m}")),
                                };
                                ident(&sm, rvalue.span)
                            })
                            .collect(),
                        None => {
                            let mut v = split_value(rvalue, &ms, (self.replication_allowed)(&ms))?;
                            for e in &mut v {
                                self.expr(e)?;
                            }
                            v
                        }
                    };
                    ms.iter()
                        .zip(src)
                        .map(|((m, _), e)| self.assign(dst_name(m), e, span))
                        .collect()
                } else {
                    let (mut l, mut r) = (lvalue.clone(), rvalue.clone());
                    self.expr(&mut l)?;
                    self.expr(&mut r)?;
                    mk(StatementKind::BlockingAssign {
                        lvalue: l,
                        rvalue: r,
                    })
                }
            }
            StatementKind::Return(None) => vec![s.clone()],
            StatementKind::Return(Some(e)) => match self.mode {
                ReturnMode::Single(sel) => {
                    let pick = if let Some(n) = bare_name(e)
                        && self.members_of(n).is_some()
                    {
                        ident(&self.scalar_of(n, sel), e.span)
                    } else {
                        let parts = split_value(e, self.ret, (self.replication_allowed)(self.ret))?;
                        let i = self.ret.iter().position(|(m, _)| m == sel)?;
                        // §10.9.2: every item of the pattern is evaluated; only
                        // the selected one is kept here.
                        if parts
                            .iter()
                            .enumerate()
                            .any(|(j, p)| j != i && !effect_free(p))
                        {
                            return None;
                        }
                        let mut v = parts[i].clone();
                        self.expr(&mut v)?;
                        v
                    };
                    mk(StatementKind::Return(Some(pick)))
                }
                ReturnMode::Multi => {
                    if bare_name(e) == Some(self.fname) {
                        mk(StatementKind::Return(None))
                    } else if let Some(r) = bare_name(e)
                        && self.members_of(r).is_some_and(|rms| {
                            rms.len() == self.ret.len()
                                && rms.iter().zip(self.ret).all(|(a, b)| a.0 == b.0)
                        })
                    {
                        let mut out: Vec<Statement> = self
                            .ret
                            .iter()
                            .map(|(m, _)| {
                                self.assign(
                                    leaf(self.fname, m),
                                    ident(&self.scalar_of(r, m), e.span),
                                    span,
                                )
                            })
                            .collect();
                        out.push(Statement::new(StatementKind::Return(None), span));
                        out
                    } else {
                        let mut parts =
                            split_value(e, self.ret, (self.replication_allowed)(self.ret))?;
                        let fname_pfx = format!("{}__", self.fname);
                        for v in &mut parts {
                            self.expr(v)?;
                            if expr_mentions_prefix(v, &fname_pfx, self.fname) {
                                return None;
                            }
                        }
                        let mut out: Vec<Statement> = self
                            .ret
                            .iter()
                            .zip(parts)
                            .map(|((m, _), v)| self.assign(leaf(self.fname, m), v, span))
                            .collect();
                        out.push(Statement::new(StatementKind::Return(None), span));
                        out
                    }
                }
            },
            StatementKind::If {
                unique_priority,
                condition,
                then_stmt,
                else_stmt,
            } => {
                let mut c = condition.clone();
                self.expr(&mut c)?;
                mk(StatementKind::If {
                    unique_priority: *unique_priority,
                    condition: c,
                    then_stmt: self.one(then_stmt)?,
                    else_stmt: match else_stmt {
                        Some(e) => Some(self.one(e)?),
                        None => None,
                    },
                })
            }
            StatementKind::Case {
                unique_priority,
                kind,
                expr,
                items,
            } => {
                let mut ce = expr.clone();
                self.expr(&mut ce)?;
                let mut out_items = Vec::with_capacity(items.len());
                for item in items {
                    if item.pattern.is_some() || item.guard.is_some() {
                        return None;
                    }
                    let mut pats = item.patterns.clone();
                    for p in &mut pats {
                        self.expr(p)?;
                    }
                    out_items.push(CaseItem {
                        patterns: pats,
                        is_default: item.is_default,
                        stmt: *self.one(&item.stmt)?,
                        span: item.span,
                        pattern: None,
                        guard: None,
                    });
                }
                mk(StatementKind::Case {
                    unique_priority: *unique_priority,
                    kind: *kind,
                    expr: ce,
                    items: out_items,
                })
            }
            StatementKind::While { condition, body } => {
                let mut c = condition.clone();
                self.expr(&mut c)?;
                mk(StatementKind::While {
                    condition: c,
                    body: self.one(body)?,
                })
            }
            StatementKind::DoWhile { body, condition } => {
                let mut c = condition.clone();
                self.expr(&mut c)?;
                mk(StatementKind::DoWhile {
                    body: self.one(body)?,
                    condition: c,
                })
            }
            StatementKind::Repeat { count, body } => {
                let mut c = count.clone();
                self.expr(&mut c)?;
                mk(StatementKind::Repeat {
                    count: c,
                    body: self.one(body)?,
                })
            }
            StatementKind::Foreach { array, vars, body } => {
                let arr = match bare_name(array).and_then(|n| self.names.get(n).map(|k| (n, k))) {
                    Some((n, Kind::Array(ms, _))) => ident(&leaf(n, &ms[0].0), array.span),
                    Some(_) => return None,
                    None => {
                        let mut a = array.clone();
                        self.expr(&mut a)?;
                        a
                    }
                };
                mk(StatementKind::Foreach {
                    array: arr,
                    vars: vars.clone(),
                    body: self.one(body)?,
                })
            }
            StatementKind::For {
                init,
                condition,
                step,
                body,
            } => {
                let mut init = init.clone();
                for i in &mut init {
                    match i {
                        ForInit::VarDecl { init, .. } => self.expr(init)?,
                        ForInit::Assign { lvalue, rvalue } => {
                            self.expr(lvalue)?;
                            self.expr(rvalue)?;
                        }
                    }
                }
                let mut condition = condition.clone();
                if let Some(c) = &mut condition {
                    self.expr(c)?;
                }
                let mut step = step.clone();
                for e in &mut step {
                    self.expr(e)?;
                }
                mk(StatementKind::For {
                    init,
                    condition,
                    step,
                    body: self.one(body)?,
                })
            }
            StatementKind::Expr(e) => {
                let mut e = e.clone();
                self.expr(&mut e)?;
                mk(StatementKind::Expr(e))
            }
            _ => return None,
        })
    }
}

fn scalarize_impl(
    fd: &FunctionDeclaration,
    args: &[Expression],
    mode: ReturnMode<'_>,
    struct_members: &dyn Fn(&DataType) -> Option<Members>,
    replication_allowed: &dyn Fn(&Members) -> bool,
) -> Option<(FunctionDeclaration, Vec<Expression>, Members)> {
    let ret = struct_members(&fd.return_type)?;
    if let ReturnMode::Single(sel) = mode
        && !ret.iter().any(|(m, _)| m == sel)
    {
        return None;
    }
    if fd.ports.len() != args.len() {
        return None;
    }
    let mut names: HashMap<String, Kind> = HashMap::default();
    let mut ports: Vec<FunctionPort> = Vec::new();
    let mut new_args: Vec<Expression> = Vec::new();
    for (p, a) in fd.ports.iter().zip(args) {
        let Some(ms) = struct_members(&p.data_type) else {
            ports.push(p.clone());
            new_args.push(a.clone());
            continue;
        };
        if !matches!(p.direction, PortDirection::Input) {
            return None;
        }
        match p.dimensions.as_slice() {
            [] => {
                for ((m, mdt), e) in ms
                    .iter()
                    .zip(split_value(a, &ms, replication_allowed(&ms))?)
                {
                    ports.push(FunctionPort {
                        data_type: mdt.clone(),
                        name: Identifier {
                            name: leaf(&p.name.name, m),
                            span: p.name.span,
                        },
                        default: None,
                        ..p.clone()
                    });
                    new_args.push(e);
                }
                names.insert(p.name.name.clone(), Kind::Scalar(ms));
            }
            [UnpackedDimension::Unsized(_)] => {
                let ExprKind::AssignmentPattern(items) = &a.kind else {
                    return None;
                };
                let mut cols: Vec<Vec<AssignmentPatternItem>> = vec![Vec::new(); ms.len()];
                for it in items {
                    let AssignmentPatternItem::Ordered(e) = it else {
                        return None;
                    };
                    for (c, v) in
                        cols.iter_mut()
                            .zip(split_value(e, &ms, replication_allowed(&ms))?)
                    {
                        c.push(AssignmentPatternItem::Ordered(v));
                    }
                }
                for ((m, mdt), col) in ms.iter().zip(cols) {
                    ports.push(FunctionPort {
                        data_type: mdt.clone(),
                        name: Identifier {
                            name: leaf(&p.name.name, m),
                            span: p.name.span,
                        },
                        default: None,
                        ..p.clone()
                    });
                    new_args.push(Expression::new(ExprKind::AssignmentPattern(col), a.span));
                }
                names.insert(p.name.name.clone(), Kind::Array(ms, items.len()));
            }
            _ => return None,
        }
    }
    // Struct locals, declared anywhere in the body.
    fn collect(
        list: &[Statement],
        f: &mut dyn FnMut(&DataType, &VarDeclarator) -> Option<()>,
    ) -> Option<()> {
        for s in list {
            match &s.kind {
                StatementKind::VarDecl {
                    data_type,
                    declarators,
                    ..
                } => {
                    for d in declarators {
                        f(data_type, d)?;
                    }
                }
                StatementKind::SeqBlock { stmts, .. } => collect(stmts, f)?,
                StatementKind::If {
                    then_stmt,
                    else_stmt,
                    ..
                } => {
                    collect(std::slice::from_ref(then_stmt), f)?;
                    if let Some(e) = else_stmt {
                        collect(std::slice::from_ref(e), f)?;
                    }
                }
                StatementKind::Case { items, .. } => {
                    for item in items {
                        collect(std::slice::from_ref(&item.stmt), f)?;
                    }
                }
                StatementKind::For { body, .. }
                | StatementKind::Foreach { body, .. }
                | StatementKind::While { body, .. }
                | StatementKind::DoWhile { body, .. }
                | StatementKind::Repeat { body, .. } => {
                    collect(std::slice::from_ref(body), f)?
                }
                _ => {}
            }
        }
        Some(())
    }
    collect(&fd.items, &mut |dt, d| {
        if let Some(ms) = struct_members(dt) {
            if !d.dimensions.is_empty() || names.contains_key(&d.name.name) {
                return None;
            }
            names.insert(d.name.name.clone(), Kind::Scalar(ms));
        }
        Some(())
    })?;
    let fname = fd.name.name.name.clone();
    if names.contains_key(&fname) {
        return None;
    }
    // The scalars are named `<name>__<member>`; a body that already declares
    // such a name would have the two merged, so give up instead.
    let mut taken: Vec<String> = fd.ports.iter().map(|p| p.name.name.clone()).collect();
    collect(&fd.items, &mut |_, d| {
        taken.push(d.name.name.clone());
        Some(())
    })?;
    fn loop_vars(list: &[Statement], out: &mut Vec<String>) {
        for s in list {
            match &s.kind {
                StatementKind::For { init, body, .. } => {
                    for i in init {
                        if let ForInit::VarDecl { name, .. } = i {
                            out.push(name.name.clone());
                        }
                    }
                    loop_vars(std::slice::from_ref(body), out);
                }
                StatementKind::Foreach { vars, body, .. } => {
                    out.extend(vars.iter().flatten().map(|v| v.name.clone()));
                    loop_vars(std::slice::from_ref(body), out);
                }
                StatementKind::While { body, .. }
                | StatementKind::DoWhile { body, .. }
                | StatementKind::Repeat { body, .. } => {
                    loop_vars(std::slice::from_ref(body), out);
                }
                StatementKind::SeqBlock { stmts, .. } => loop_vars(stmts, out),
                StatementKind::If {
                    then_stmt,
                    else_stmt,
                    ..
                } => {
                    loop_vars(std::slice::from_ref(then_stmt), out);
                    if let Some(e) = else_stmt {
                        loop_vars(std::slice::from_ref(e), out);
                    }
                }
                StatementKind::Case { items, .. } => {
                    for item in items {
                        loop_vars(std::slice::from_ref(&item.stmt), out);
                    }
                }
                _ => {}
            }
        }
    }
    loop_vars(&fd.items, &mut taken);
    let mut generated: Vec<String> = ret.iter().map(|(m, _)| leaf(&fname, m)).collect();
    for (n, k) in &names {
        let ms = match k {
            Kind::Array(ms, _) | Kind::Scalar(ms) => ms,
        };
        generated.extend(ms.iter().map(|(m, _)| leaf(n, m)));
    }
    if generated.iter().any(|g| taken.contains(g)) {
        return None;
    }
    let ctx = Ctx {
        names,
        fname: &fname,
        ret: &ret,
        mode,
        replication_allowed,
    };
    let mut items = Vec::new();
    // Declare `fname__m` result variables at the top of the body: in Single mode
    // only non-`sel` fields need locals (the selected field uses `fname` / ret_slot);
    // in Multi mode ALL fields are declared so `compile_pure_body` initializes
    // them with exact types and `fn_is_pure_in_ext` recognizes them as local state.
    for (m, mdt) in &ret {
        if let ReturnMode::Single(sel) = mode
            && m == sel
        {
            continue;
        }
        items.push(Statement::new(
            StatementKind::VarDecl {
                data_type: mdt.clone(),
                lifetime: None,
                declarators: vec![VarDeclarator {
                    decl: crate::ast::binding::Bound::UNSET,
                    name: Identifier {
                        name: leaf(&fname, m),
                        span: fd.span,
                    },
                    dimensions: Vec::new(),
                    init: None,
                    span: fd.span,
                }],
            },
            fd.span,
        ));
    }
    items.extend(ctx.stmts(&fd.items)?);
    let mut out = fd.clone();
    // Distinct spans keep purity-memo keys separate from the original body
    // (`start..end`) and between Single (`end..end`) and Multi (`start..start`).
    if let ReturnMode::Single(sel) = mode {
        out.span = Span::new(fd.span.end, fd.span.end);
        out.return_type = ret.iter().find(|(m, _)| m == sel)?.1.clone();
    } else {
        out.span = Span::new(fd.span.start, fd.span.start);
        out.return_type = DataType::Void(fd.span);
    }
    out.ports = ports;
    out.items = items.into();
    Some((out, new_args, ret))
}

/// Rewrite `fd` and its actuals `args` into an equivalent function over
/// scalars whose result is member `sel` of `fd`'s struct result.
pub(crate) fn scalarize_member_call(
    fd: &FunctionDeclaration,
    args: &[Expression],
    sel: &str,
    struct_members: &dyn Fn(&DataType) -> Option<Members>,
    replication_allowed: &dyn Fn(&Members) -> bool,
) -> Option<(FunctionDeclaration, Vec<Expression>)> {
    let (out, new_args, _) =
        scalarize_impl(fd, args, ReturnMode::Single(sel), struct_members, replication_allowed)?;
    Some((out, new_args))
}

/// Rewrite `fd` and its actuals `args` into a single void-return body over
/// scalars writing all leaf outputs `(dotted_member, scalar_var, member_dt)`,
/// so multi-member struct calls execute ONCE instead of N times.
pub(crate) fn scalarize_multi_call(
    fd: &FunctionDeclaration,
    args: &[Expression],
    struct_members: &dyn Fn(&DataType) -> Option<Members>,
    replication_allowed: &dyn Fn(&Members) -> bool,
) -> Option<ScalarizedMultiCall> {
    let fname = fd.name.name.name.as_str();
    let (out, new_args, ret) =
        scalarize_impl(fd, args, ReturnMode::Multi, struct_members, replication_allowed)?;
    let outputs = ret
        .into_iter()
        .map(|(m, dt)| {
            let sname = leaf(fname, &m);
            (m, sname, dt)
        })
        .collect();
    Some(ScalarizedMultiCall {
        fd: out,
        args: new_args,
        outputs,
    })
}
