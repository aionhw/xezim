//! Second-pass `should_fail` lint.
//!
//! Additive error detection for illegal SystemVerilog constructs that the main
//! parse/elaborate path is too permissive to reject. Runs AFTER a successful
//! elaboration in `--compile` mode and, on any violation, emits an error and a
//! non-zero exit — so sv-tests `should_fail` cases are correctly rejected.
//!
//! IMPORTANT: this pass NEVER changes the existing compile/elaborate behavior.
//! It only inspects the already-built AST + elaborated module and *adds*
//! diagnostics, so a clean design cannot regress unless a check is imprecise.
//! Every check is kept conservative (only fire on a definite LRM violation),
//! and the whole pass is validated to hold the static-suite baseline (1005).

use xezim_core::ast::decl::{
    ClassDeclaration, ClassItem, ClassMethodKind, ClassQualifier, ModuleItem, TypedefDeclaration,
};
use xezim_core::ast::expr::{AssignmentPatternItem, Expression, ExprKind, NumberLiteral, RangeKind};
use xezim_core::ast::stmt::{Statement, StatementKind, VarDeclarator};
use xezim_core::ast::types::{DataType, PackedDimension, UnpackedDimension};
use xezim_core::elaborate::ElaboratedModule;
use xezim_core::SourceDefinition;

/// Run the second-pass lint over every top-level definition. Returns a list of
/// error messages (empty == clean).
pub fn lint_should_fail(defs: &[&SourceDefinition], elab: &ElaboratedModule) -> Vec<String> {
    let mut errs = Vec::new();
    for def in defs {
        match def {
            SourceDefinition::Class(c) => check_class(c, &mut errs),
            SourceDefinition::Typedef(t) => {
                check_typedef(t, elab, &mut errs);
                // Width-identifier check is restricted to TOP-LEVEL typedefs
                // (no enclosing module scope), where every value parameter is
                // global and present in `elab.parameters` — so an unresolved
                // width identifier is definitely undeclared.
                check_struct_typedef_widths(t, elab, &mut errs);
            }
            SourceDefinition::Module(m) => {
                for it in &m.items {
                    check_module_item(it, elab, &mut errs);
                }
                check_proc_net_assign(&m.items, &mut errs);
            }
            SourceDefinition::Interface(m) => {
                for it in &m.items {
                    check_module_item(it, elab, &mut errs);
                }
            }
            SourceDefinition::Program(m) => {
                for it in &m.items {
                    check_module_item(it, elab, &mut errs);
                }
            }
            SourceDefinition::Package(_) => {}
        }
    }
    errs
}

/// Classes can appear nested inside module/interface/program bodies.
fn check_module_item(item: &ModuleItem, elab: &ElaboratedModule, errs: &mut Vec<String>) {
    match item {
        ModuleItem::ClassDeclaration(c) => check_class(c, errs),
        ModuleItem::TypedefDeclaration(t) => check_typedef(t, elab, errs),
        ModuleItem::DataDeclaration(d) => {
            check_enum_type(&d.data_type, elab, errs);
            for decl in &d.declarators {
                check_array_flat_init(decl, elab, errs);
            }
        }
        ModuleItem::NetDeclaration(d) => check_enum_type(&d.data_type, elab, errs),
        ModuleItem::AlwaysConstruct(a) => {
            for_each_stmt_expr(&a.stmt, &mut |e| check_zero_slice(e, elab, errs));
        }
        ModuleItem::InitialConstruct(i) => {
            for_each_stmt_expr(&i.stmt, &mut |e| check_zero_slice(e, elab, errs));
        }
        ModuleItem::ContinuousAssign(ca) => {
            for (l, r) in &ca.assignments {
                for_each_expr(l, &mut |e| check_zero_slice(e, elab, errs));
                for_each_expr(r, &mut |e| check_zero_slice(e, elab, errs));
            }
        }
        _ => {}
    }
}

/// §5.10/§10.9.1: an unpacked array with an ordered assignment pattern must
/// have exactly one element per array entry. A flat C-style list (e.g.
/// `ms_t ms[1:0] = '{0,0,1,1};` for a 2-entry array) is illegal. Conservative:
/// only single-dimension arrays with an all-ordered, all-scalar pattern whose
/// element count differs from the (constant-folded) array size are flagged —
/// nested patterns, default/replication, and non-const sizes are skipped.
fn check_array_flat_init(d: &VarDeclarator, elab: &ElaboratedModule, errs: &mut Vec<String>) {
    if d.dimensions.len() != 1 {
        return;
    }
    let UnpackedDimension::Range { left, right, .. } = &d.dimensions[0] else {
        return;
    };
    let p = Some(&elab.parameters);
    let (Some(l), Some(r)) = (
        xezim_core::elaborate::const_eval_i64_with_params(left, p),
        xezim_core::elaborate::const_eval_i64_with_params(right, p),
    ) else {
        return;
    };
    let n = (l - r).abs() + 1;
    let Some(init) = &d.init else { return };
    let ExprKind::AssignmentPattern(items) = &init.kind else {
        return;
    };
    let mut m: i64 = 0;
    for it in items {
        match it {
            AssignmentPatternItem::Ordered(e) => {
                // a nested pattern/replication is a proper per-entry init, not flat
                if matches!(
                    e.kind,
                    ExprKind::AssignmentPattern(_) | ExprKind::Replication { .. }
                ) {
                    return;
                }
                m += 1;
            }
            // default/named/typed/keyed forms aren't a flat ordered list
            _ => return,
        }
    }
    if m != n {
        errs.push(format!(
            "array '{}' of size {} initialized with {} flat ordered elements — use a nested \
             assignment pattern (LRM 1800-2017 §5.10)",
            d.name.name, n, m
        ));
    }
}

/// §6.5: a net (an explicit `wire`/`tri`/... declaration) may not be the target
/// of a procedural assignment (`=`/`<=` inside always/initial) — only a
/// continuous assignment or `force`. Catches `wire w; initial w = ...;`.
/// Conservative: only explicit NetDeclarations are treated as nets (output
/// ports, where net-vs-var is ambiguous, are NOT flagged), and only `=`/`<=`
/// targets are checked (force/release/assign are separate statement kinds).
fn check_proc_net_assign(items: &[ModuleItem], errs: &mut Vec<String>) {
    use std::collections::HashSet;
    let mut nets: HashSet<String> = HashSet::new();
    for it in items {
        if let ModuleItem::NetDeclaration(nd) = it {
            for d in &nd.declarators {
                nets.insert(d.name.name.clone());
            }
        }
    }
    if nets.is_empty() {
        return;
    }
    let mut flagged: HashSet<String> = HashSet::new();
    for it in items {
        let stmt = match it {
            ModuleItem::AlwaysConstruct(a) => &a.stmt,
            ModuleItem::InitialConstruct(i) => &i.stmt,
            ModuleItem::FinalConstruct(_) => continue,
            _ => continue,
        };
        for_each_proc_assign_lhs(stmt, &mut |lv| {
            if let Some(b) = base_ident(lv) {
                if nets.contains(&b) && flagged.insert(b.clone()) {
                    errs.push(format!(
                        "net '{}' is the target of a procedural assignment (LRM 1800-2017 \
                         §6.5 — nets need a continuous assignment)",
                        b
                    ));
                }
            }
        });
    }
}

/// Root identifier of an lvalue, peeling index/part-select/member access.
fn base_ident(e: &Expression) -> Option<String> {
    match &e.kind {
        ExprKind::Ident(h) if h.path.len() == 1 => Some(h.path[0].name.name.clone()),
        ExprKind::Index { expr, .. }
        | ExprKind::RangeSelect { expr, .. }
        | ExprKind::MemberAccess { expr, .. }
        | ExprKind::Paren(expr) => base_ident(expr),
        _ => None,
    }
}

/// Call `f` on the lvalue of each procedural assignment (`=`/`<=`) in a
/// statement tree.
fn for_each_proc_assign_lhs(stmt: &Statement, f: &mut dyn FnMut(&Expression)) {
    match &stmt.kind {
        StatementKind::BlockingAssign { lvalue, .. }
        | StatementKind::NonblockingAssign { lvalue, .. } => f(lvalue),
        StatementKind::If {
            then_stmt,
            else_stmt,
            ..
        } => {
            for_each_proc_assign_lhs(then_stmt, f);
            if let Some(e) = else_stmt {
                for_each_proc_assign_lhs(e, f);
            }
        }
        StatementKind::Case { items, .. } => {
            items.iter().for_each(|it| for_each_proc_assign_lhs(&it.stmt, f))
        }
        StatementKind::For { body, .. }
        | StatementKind::Foreach { body, .. }
        | StatementKind::While { body, .. }
        | StatementKind::DoWhile { body, .. }
        | StatementKind::Repeat { body, .. }
        | StatementKind::Forever { body } => for_each_proc_assign_lhs(body, f),
        StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
            stmts.iter().for_each(|s| for_each_proc_assign_lhs(s, f))
        }
        StatementKind::TimingControl { stmt, .. } | StatementKind::Wait { stmt, .. } => {
            for_each_proc_assign_lhs(stmt, f)
        }
        _ => {}
    }
}

/// §11.5.1: the width of an indexed part-select (`[base +: w]` / `[base -: w]`)
/// must be a positive constant — a width that constant-folds to 0 is illegal.
fn check_zero_slice(e: &Expression, elab: &ElaboratedModule, errs: &mut Vec<String>) {
    if let ExprKind::RangeSelect { kind, right, .. } = &e.kind {
        if matches!(kind, RangeKind::IndexedUp | RangeKind::IndexedDown) {
            if let Some(0) =
                xezim_core::elaborate::const_eval_i64_with_params(right, Some(&elab.parameters))
            {
                errs.push(
                    "indexed part-select has zero width (LRM 1800-2017 §11.5.1)".to_string(),
                );
            }
        }
    }
}

/// Visit every sub-expression of `e` (pre-order), calling `f` on each.
fn for_each_expr(e: &Expression, f: &mut dyn FnMut(&Expression)) {
    f(e);
    match &e.kind {
        ExprKind::Unary { operand, .. } => for_each_expr(operand, f),
        ExprKind::Binary { left, right, .. } => {
            for_each_expr(left, f);
            for_each_expr(right, f);
        }
        ExprKind::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            for_each_expr(condition, f);
            for_each_expr(then_expr, f);
            for_each_expr(else_expr, f);
        }
        ExprKind::Concatenation(xs) => xs.iter().for_each(|x| for_each_expr(x, f)),
        ExprKind::Replication { count, exprs } => {
            for_each_expr(count, f);
            exprs.iter().for_each(|x| for_each_expr(x, f));
        }
        ExprKind::Call { func, args } => {
            for_each_expr(func, f);
            args.iter().for_each(|x| for_each_expr(x, f));
        }
        ExprKind::SystemCall { args, .. } => args.iter().for_each(|x| for_each_expr(x, f)),
        ExprKind::Inside { expr, ranges } => {
            for_each_expr(expr, f);
            ranges.iter().for_each(|x| for_each_expr(x, f));
        }
        ExprKind::MemberAccess { expr, .. } => for_each_expr(expr, f),
        ExprKind::Index { expr, index } => {
            for_each_expr(expr, f);
            for_each_expr(index, f);
        }
        ExprKind::RangeSelect {
            expr, left, right, ..
        } => {
            for_each_expr(expr, f);
            for_each_expr(left, f);
            for_each_expr(right, f);
        }
        ExprKind::Range(a, b) => {
            for_each_expr(a, f);
            for_each_expr(b, f);
        }
        ExprKind::Paren(x) => for_each_expr(x, f),
        ExprKind::AssignExpr { lvalue, rvalue } => {
            for_each_expr(lvalue, f);
            for_each_expr(rvalue, f);
        }
        _ => {}
    }
}

/// Walk every expression contained in a statement (and its sub-statements).
fn for_each_stmt_expr(stmt: &Statement, f: &mut dyn FnMut(&Expression)) {
    match &stmt.kind {
        StatementKind::Expr(e) => for_each_expr(e, f),
        StatementKind::BlockingAssign { lvalue, rvalue } => {
            for_each_expr(lvalue, f);
            for_each_expr(rvalue, f);
        }
        StatementKind::NonblockingAssign {
            lvalue,
            rvalue,
            delay,
        } => {
            for_each_expr(lvalue, f);
            for_each_expr(rvalue, f);
            if let Some(d) = delay {
                for_each_expr(d, f);
            }
        }
        StatementKind::If {
            condition,
            then_stmt,
            else_stmt,
            ..
        } => {
            for_each_expr(condition, f);
            for_each_stmt_expr(then_stmt, f);
            if let Some(e) = else_stmt {
                for_each_stmt_expr(e, f);
            }
        }
        StatementKind::Case { expr, items, .. } => {
            for_each_expr(expr, f);
            for it in items {
                for_each_stmt_expr(&it.stmt, f);
            }
        }
        StatementKind::For {
            condition,
            step,
            body,
            ..
        } => {
            if let Some(c) = condition {
                for_each_expr(c, f);
            }
            step.iter().for_each(|s| for_each_expr(s, f));
            for_each_stmt_expr(body, f);
        }
        StatementKind::Foreach { array, body, .. } => {
            for_each_expr(array, f);
            for_each_stmt_expr(body, f);
        }
        StatementKind::While { condition, body }
        | StatementKind::DoWhile { body, condition } => {
            for_each_expr(condition, f);
            for_each_stmt_expr(body, f);
        }
        StatementKind::Repeat { count, body } => {
            for_each_expr(count, f);
            for_each_stmt_expr(body, f);
        }
        StatementKind::Forever { body } => for_each_stmt_expr(body, f),
        StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
            stmts.iter().for_each(|s| for_each_stmt_expr(s, f));
        }
        StatementKind::TimingControl { stmt, .. } => for_each_stmt_expr(stmt, f),
        StatementKind::Wait { condition, stmt } => {
            for_each_expr(condition, f);
            for_each_stmt_expr(stmt, f);
        }
        StatementKind::Return(Some(e)) => for_each_expr(e, f),
        _ => {}
    }
}

/// §6.19: in an enum with an explicit base type, a member whose value is a
/// *sized* literal constant must match the base-type width.
fn check_enum_type(dt: &DataType, elab: &ElaboratedModule, errs: &mut Vec<String>) {
    let DataType::Enum(et) = dt else { return };
    let Some(base) = &et.base_type else { return };
    let w = xezim_core::elaborate::resolve_type_width(
        base,
        Some(&elab.parameters),
        Some(&elab.typedefs),
    );
    if w == 0 {
        return;
    }
    for m in &et.members {
        if let Some(init) = &m.init {
            if let ExprKind::Number(NumberLiteral::Integer { size: Some(s), .. }) = &init.kind {
                if *s != w {
                    errs.push(format!(
                        "enum member '{}': sized literal width {} differs from the enum base \
                         width {} (LRM 1800-2017 §6.19)",
                        m.name.name, s, w
                    ));
                }
            }
        }
    }
}

/// §8.20: a `pure virtual` method (or any method qualified `pure`) is legal
/// only inside a *virtual* (abstract) class or an interface class. A concrete
/// class declaring one is an error.
fn check_class(c: &ClassDeclaration, errs: &mut Vec<String>) {
    if !c.virtual_kw && !c.is_interface {
        for item in &c.items {
            if let ClassItem::Method(m) = item {
                let pure = m.qualifiers.contains(&ClassQualifier::Pure)
                    || matches!(m.kind, ClassMethodKind::PureVirtual(_));
                if pure {
                    errs.push(format!(
                        "class '{}': a pure virtual method is illegal in a non-virtual, \
                         non-interface class (LRM 1800-2017 §8.20)",
                        c.name.name
                    ));
                    break;
                }
            }
        }
    }
    // Recurse into nested classes regardless of this class's kind.
    for item in &c.items {
        if let ClassItem::Class(nested) = item {
            check_class(nested, errs);
        }
    }
}

/// §6.18: a non-forward `typedef <T> name;` whose base type `<T>` is a bare,
/// undeclared simple identifier is an error. Conservative: only fires when the
/// base is a single-segment name (no `::`), is not a built-in keyword type, and
/// is absent from every elaborated type namespace (typedefs, classes, enums,
/// interfaces, packages, parameters).
fn check_typedef(t: &TypedefDeclaration, elab: &ElaboratedModule, errs: &mut Vec<String>) {
    if t.forward {
        return;
    }
    check_enum_type(&t.data_type, elab, errs);
    if let DataType::TypeReference { name, .. } = &t.data_type {
        // package-qualified (`pkg::T`) — out of scope for this conservative check
        if name.scope.is_some() {
            return;
        }
        let n = &name.name.name;
        if n.is_empty() {
            return;
        }
        if is_builtin_type(n) {
            return;
        }
        let known = elab.typedefs.contains_key(n)
            || elab.classes.contains_key(n)
            || elab.enum_members.contains_key(n)
            || elab.interfaces.contains(n)
            || elab.packages.contains(n)
            || elab.parameters.contains_key(n);
        if !known {
            errs.push(format!(
                "typedef '{}': base type '{}' is not declared (LRM 1800-2017 §6.18)",
                t.name.name, n
            ));
        }
    }
}

/// §6.18/§7.2: a packed-struct/union typedef whose member width references a
/// bare, undeclared identifier (e.g. `typedef struct packed { reg [A-1:0] a; }`
/// with no `A` in scope). Top-level only (see caller). Conservative: only
/// arithmetic-shaped width expressions are inspected, and an identifier counts
/// as declared if it is a known value parameter or a type-ish name.
fn check_struct_typedef_widths(
    t: &TypedefDeclaration,
    elab: &ElaboratedModule,
    errs: &mut Vec<String>,
) {
    let DataType::Struct(su) = &t.data_type else {
        return;
    };
    for m in &su.members {
        let mut ids = Vec::new();
        dim_idents(&m.data_type, &mut ids);
        for id in ids {
            if !value_ident_declared(&id, elab) {
                errs.push(format!(
                    "typedef '{}': struct member width references undeclared identifier '{}' \
                     (LRM 1800-2017 §6.18)",
                    t.name.name, id
                ));
            }
        }
    }
}

fn value_ident_declared(id: &str, elab: &ElaboratedModule) -> bool {
    elab.parameters.contains_key(id)
        || elab.typedefs.contains_key(id)
        || elab.enum_members.contains_key(id)
}

/// Collect single-segment identifiers from the packed-dimension range
/// expressions of a data type (only the `[msb:lsb]` of a vector/implicit type).
fn dim_idents(dt: &DataType, out: &mut Vec<String>) {
    let dims = match dt {
        DataType::IntegerVector { dimensions, .. } => dimensions,
        DataType::Implicit { dimensions, .. } => dimensions,
        _ => return,
    };
    for d in dims {
        if let PackedDimension::Range { left, right, .. } = d {
            collect_idents(left, out);
            collect_idents(right, out);
        }
    }
}

/// Conservatively collect bare (single-segment) identifiers from an arithmetic
/// expression. Only descends pure operator/paren/conditional trees — never into
/// calls, indexing, or member access — so a function-call or array-based width
/// is never mistaken for an undeclared identifier.
fn collect_idents(e: &Expression, out: &mut Vec<String>) {
    match &e.kind {
        ExprKind::Ident(h) => {
            if h.path.len() == 1 {
                out.push(h.path[0].name.name.clone());
            }
        }
        ExprKind::Unary { operand, .. } => collect_idents(operand, out),
        ExprKind::Binary { left, right, .. } => {
            collect_idents(left, out);
            collect_idents(right, out);
        }
        ExprKind::Paren(x) => collect_idents(x, out),
        ExprKind::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            collect_idents(condition, out);
            collect_idents(then_expr, out);
            collect_idents(else_expr, out);
        }
        _ => {}
    }
}

fn is_builtin_type(n: &str) -> bool {
    matches!(
        n,
        "bit" | "logic" | "reg" | "byte" | "shortint" | "int" | "longint" | "integer"
            | "time" | "real" | "shortreal" | "realtime" | "string" | "chandle" | "event"
            | "void" | "wire" | "tri" | "wand" | "wor" | "uwire" | "signed" | "unsigned"
            | "genvar" | "type" | "enum" | "struct" | "union" | "process" | "supply0"
            | "supply1"
    )
}
