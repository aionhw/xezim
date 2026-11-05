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

struct Ctx<'a> {
    names: HashMap<String, Kind>,
    /// The function's own name, its implicit result variable (§13.4.1).
    fname: &'a str,
    ret: &'a Members,
    sel: &'a str,
}

fn leaf(base: &str, m: &str) -> String {
    format!("{base}__{m}")
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

/// One expression per member from a struct-valued actual or item: an ordered
/// or named pattern is taken apart, anything else is member-selected, which
/// evaluates it once per member, so it must have no effect.
fn split_value(e: &Expression, members: &Members) -> Option<Vec<Expression>> {
    if let ExprKind::AssignmentPattern(items) = &e.peel_typed_pattern().kind {
        if items
            .iter()
            .all(|it| matches!(it, AssignmentPatternItem::Ordered(_)))
        {
            if items.len() != members.len() {
                return None;
            }
            return Some(items.iter().map(|it| it.expr().clone()).collect());
        }
        let mut out = Vec::with_capacity(members.len());
        for (m, _) in members {
            let hit = items.iter().find_map(|it| match it {
                AssignmentPatternItem::Named(id, v) if id.name == *m => Some(v.clone()),
                _ => None,
            });
            out.push(hit?);
        }
        if items.len() != members.len() {
            return None;
        }
        return Some(out);
    }
    if !effect_free(e) {
        return None;
    }
    Some(
        members
            .iter()
            .map(|(m, _)| {
                let mid = Identifier {
                    name: m.clone(),
                    span: e.span,
                };
                // A plain name's member is its own leaf signal (`n.m`).
                if let ExprKind::Ident(h) = &e.kind
                    && h.root.is_none()
                    && h.path.iter().all(|s| s.selects.is_empty())
                {
                    let mut path = h.path.clone();
                    path.push(HierPathSegment {
                        name: mid,
                        selects: Vec::new(),
                    });
                    return Expression::new(
                        ExprKind::Ident(HierarchicalIdentifier {
                            root: None,
                            path,
                            span: e.span,
                            cached_signal_id: crate::ast::cache::SignalCache::new(None),
                            cached_resolved_name: crate::ast::cache::NameCache::new(),
                        }),
                        e.span,
                    );
                }
                Expression::new(
                    ExprKind::MemberAccess {
                        expr: Box::new(e.clone()),
                        member: mid,
                    },
                    e.span,
                )
            })
            .collect(),
    )
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

    /// The scalar that holds member `m` of struct name `n`.
    fn scalar_of(&self, n: &str, m: &str) -> String {
        if n == self.fname && m == self.sel {
            n.to_string()
        } else {
            leaf(n, m)
        }
    }

    fn is_struct_name(&self, n: &str) -> bool {
        n == self.fname || self.names.contains_key(n)
    }

    fn expr(&self, e: &mut Expression) -> Option<()> {
        let span = e.span;
        let replace: Option<Expression> = match &mut e.kind {
            ExprKind::MemberAccess { expr: base, member } => {
                if let Some(n) = bare_name(base) {
                    if let Some(ms) = self.members_of(n) {
                        ms.iter().any(|(m, _)| *m == member.name).then_some(())?;
                        Some(ident(&self.scalar_of(n, &member.name), span))
                    } else if self.is_struct_name(n) {
                        return None; // a member of the whole array
                    } else {
                        return self.expr(base);
                    }
                } else if let ExprKind::Index { expr: arr, index } = &mut base.kind {
                    match bare_name(arr).and_then(|n| self.names.get(n).map(|k| (n, k))) {
                        Some((n, Kind::Array(ms, _))) => {
                            ms.iter().any(|(m, _)| *m == member.name).then_some(())?;
                            let n = n.to_string();
                            self.expr(index)?;
                            Some(Expression::new(
                                ExprKind::Index {
                                    expr: Box::new(ident(&leaf(&n, &member.name), arr.span)),
                                    index: index.clone(),
                                },
                                span,
                            ))
                        }
                        Some(_) => return None,
                        None => return self.expr(base),
                    }
                } else {
                    return self.expr(base);
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
                let n = h
                    .path
                    .iter()
                    .map(|s| s.name.name.as_str())
                    .collect::<Vec<_>>();
                if h.root.is_none() && self.is_struct_name(n[0]) {
                    // A whole struct in an unsupported position, or a dotted
                    // name (`r.V`) this rewrite does not scalarize.
                    return None;
                }
                for s in &mut h.path {
                    for sel in &mut s.selects {
                        self.expr(sel)?;
                    }
                }
                None
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
                    stmts: v,
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
                stmts: self.stmts(stmts)?,
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
                            Some(e) => Some(split_value(e, &ms)?),
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
                                        name: Identifier {
                                            name: leaf(&d.name.name, m),
                                            span: d.name.span,
                                        },
                                        dimensions: Vec::new(),
                                        init,
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
            StatementKind::BlockingAssign { lvalue, rvalue } => {
                if let Some(n) = bare_name(lvalue)
                    && let Some(ms) = self.members_of(n)
                {
                    // Whole-struct write: one write per member. A struct
                    // source names its scalars directly; anything else is
                    // split and then rewritten like any expression.
                    let src: Vec<Expression> =
                        match bare_name(rvalue).and_then(|r| self.members_of(r).map(|_| r)) {
                            Some(r) => ms
                                .iter()
                                .map(|(m, _)| ident(&self.scalar_of(r, m), rvalue.span))
                                .collect(),
                            None => {
                                let mut v = split_value(rvalue, ms)?;
                                for e in &mut v {
                                    self.expr(e)?;
                                }
                                v
                            }
                        };
                    ms.iter()
                        .zip(src)
                        .map(|((m, _), e)| self.assign(self.scalar_of(n, m), e, span))
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
            StatementKind::Return(Some(e)) => {
                let pick = if let Some(n) = bare_name(e)
                    && self.members_of(n).is_some()
                {
                    ident(&self.scalar_of(n, self.sel), e.span)
                } else {
                    let parts = split_value(e, self.ret)?;
                    let i = self.ret.iter().position(|(m, _)| m == self.sel)?;
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

/// Rewrite `fd` and its actuals `args` into an equivalent function over
/// scalars whose result is member `sel` of `fd`'s struct result.
/// `struct_members` resolves a declared type to its members when it is an
/// unpacked struct of scalars, and returns None otherwise.
pub(crate) fn scalarize_member_call(
    fd: &FunctionDeclaration,
    args: &[Expression],
    sel: &str,
    struct_members: &dyn Fn(&DataType) -> Option<Members>,
) -> Option<(FunctionDeclaration, Vec<Expression>)> {
    let ret = struct_members(&fd.return_type)?;
    let sel_ty = ret.iter().find(|(m, _)| m == sel)?.1.clone();
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
                for ((m, mdt), e) in ms.iter().zip(split_value(a, &ms)?) {
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
                    for (c, v) in cols.iter_mut().zip(split_value(e, &ms)?) {
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
                StatementKind::For { body, .. } | StatementKind::Foreach { body, .. } => {
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
        sel,
    };
    let mut items = Vec::new();
    // The result's other members, should the body write them through the
    // function name (`f.I = ...`).
    for (m, mdt) in ret.iter().filter(|(m, _)| m != sel) {
        items.push(Statement::new(
            StatementKind::VarDecl {
                data_type: mdt.clone(),
                lifetime: None,
                declarators: vec![VarDeclarator {
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
    // A span of its own: per-declaration facts (the purity memo) are keyed
    // by name and span, and this body is not the declared one.
    out.span = Span::new(fd.span.end, fd.span.end);
    out.return_type = sel_ty;
    out.ports = ports;
    out.items = items.into();
    Some((out, new_args))
}
