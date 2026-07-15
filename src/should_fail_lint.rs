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
use xezim_core::ast::expr::{
    AssignmentPatternItem, Expression, ExprKind, NumberBase, NumberLiteral, RangeKind,
};
use xezim_core::ast::stmt::{Statement, StatementKind, VarDeclarator};
use xezim_core::ast::types::{
    DataType, EnumType, IntegerAtomType, PackedDimension, Signing, UnpackedDimension,
};
use xezim_core::elaborate::ElaboratedModule;
use xezim_core::SourceDefinition;

/// Run the second-pass lint over every top-level definition. Returns a list of
/// error messages (empty == clean).
pub fn lint_should_fail(defs: &[&SourceDefinition], elab: &ElaboratedModule) -> Vec<String> {
    let mut errs = Vec::new();
    // §23.3.2: map every module/interface/program to its declared port names,
    // so a named port connection to a non-existent port can be rejected.
    let port_map = build_port_map(defs);
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
                check_enum_assign(&m.items, elab, &mut errs);
                check_dynarray_assign(&m.items, elab, &mut errs);
                check_stream_widths(&m.items, elab, &mut errs);
                check_wildcard_cmp(&m.items, elab, &mut errs);
                check_instantiations(&m.items, &port_map, &mut errs);
                check_implicit_ports(&m.ports, &m.items, &port_map, &mut errs);
            }
            SourceDefinition::Interface(m) => {
                for it in &m.items {
                    check_module_item(it, elab, &mut errs);
                }
                check_stream_widths(&m.items, elab, &mut errs);
                check_wildcard_cmp(&m.items, elab, &mut errs);
                check_instantiations(&m.items, &port_map, &mut errs);
                check_implicit_ports(&m.ports, &m.items, &port_map, &mut errs);
            }
            SourceDefinition::Program(m) => {
                for it in &m.items {
                    check_module_item(it, elab, &mut errs);
                }
                check_stream_widths(&m.items, elab, &mut errs);
                check_wildcard_cmp(&m.items, elab, &mut errs);
                check_program_items(&m.items, &mut errs);
                check_instantiations(&m.items, &port_map, &mut errs);
                check_implicit_ports(&m.ports, &m.items, &port_map, &mut errs);
            }
            SourceDefinition::Package(p) => {
                use xezim_core::ast::decl::PackageItem;
                for it in &p.items {
                    match it {
                        PackageItem::Class(c) => check_class(c, &mut errs),
                        PackageItem::Function(f) => check_output_port_defaults(&f.ports, &mut errs),
                        PackageItem::Task(t) => check_output_port_defaults(&t.ports, &mut errs),
                        PackageItem::Data(d) => check_enum_type(&d.data_type, elab, &mut errs),
                        PackageItem::Typedef(td) => check_enum_type(&td.data_type, elab, &mut errs),
                        _ => {}
                    }
                }
            }
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
            check_packed_dims(&d.data_type, elab, errs);
            for decl in &d.declarators {
                check_array_flat_init(decl, elab, errs);
                check_unpacked_dims(&decl.name.name, &decl.dimensions, elab, errs);
                check_new_array_target(&d.data_type, decl, errs);
            }
        }
        ModuleItem::NetDeclaration(d) => {
            check_enum_type(&d.data_type, elab, errs);
            check_packed_dims(&d.data_type, elab, errs);
        }
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
        ModuleItem::FunctionDeclaration(f) => check_output_port_defaults(&f.ports, errs),
        ModuleItem::TaskDeclaration(t) => check_output_port_defaults(&t.ports, errs),
        _ => {}
    }
}

/// §13.5.3 (iverilog restriction): a default value on an `output`/`inout`
/// subroutine port is not permitted. Flags `task t(output int j = b);`.
fn check_output_port_defaults(
    ports: &[xezim_core::ast::decl::FunctionPort],
    errs: &mut Vec<String>,
) {
    use xezim_core::ast::types::PortDirection;
    for p in ports {
        if p.default.is_some() && matches!(p.direction, PortDirection::Output | PortDirection::Inout)
        {
            errs.push(format!(
                "default value on subroutine {} port '{}' is not allowed (LRM 1800-2017 §13.5.3)",
                match p.direction {
                    PortDirection::Output => "output",
                    _ => "inout",
                },
                p.name.name
            ));
        }
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

/// True if `dt` is an enum type, resolving one typedef reference against the
/// elaborated typedef table (`typedef enum {...} e; e v;`).
fn resolves_to_enum(dt: &DataType, elab: &ElaboratedModule) -> bool {
    match dt {
        DataType::Enum(_) => true,
        // An enum typedef is registered in `enum_members` (keyed by the typedef
        // name), not `typedef_types` — see elaborate::process_typedef.
        DataType::TypeReference { name, .. } => {
            elab.enum_members.contains_key(&name.name.name)
                || elab
                    .typedef_types
                    .get(&name.name.name)
                    .map_or(false, |inner| matches!(inner, DataType::Enum(_)))
        }
        _ => false,
    }
}

/// §6.19.3: the RHS of an enum assignment must itself be enum-typed or an
/// explicit cast. A *bare integer literal* (optionally parenthesised / negated)
/// is the unambiguous illegal case — enum members and same-type enum variables
/// parse as `Ident`, and a cast parses as a `SystemCall`, so neither is flagged.
fn is_bare_integer_rhs(e: &Expression) -> bool {
    match &e.kind {
        ExprKind::Number(NumberLiteral::Integer { .. }) => true,
        ExprKind::Paren(inner) => is_bare_integer_rhs(inner),
        ExprKind::Unary { operand, .. } => is_bare_integer_rhs(operand),
        _ => false,
    }
}

/// §6.19.3: reject `enum_var = <integer literal>` (no explicit cast). Covers
/// both procedural (`e = 1;`) and continuous (`assign e = 1;`) assignments.
/// Deliberately narrow — only a bare integer-literal RHS to a WHOLE enum
/// variable fires, so legal enum-member / same-type / cast assignments are
/// untouched.
fn check_enum_assign(items: &[ModuleItem], elab: &ElaboratedModule, errs: &mut Vec<String>) {
    use std::collections::HashSet;
    let mut enum_vars: HashSet<String> = HashSet::new();
    for it in items {
        match it {
            ModuleItem::DataDeclaration(d) if resolves_to_enum(&d.data_type, elab) => {
                for decl in &d.declarators {
                    enum_vars.insert(decl.name.name.clone());
                }
            }
            ModuleItem::NetDeclaration(nd) if resolves_to_enum(&nd.data_type, elab) => {
                for decl in &nd.declarators {
                    enum_vars.insert(decl.name.name.clone());
                }
            }
            _ => {}
        }
    }
    if enum_vars.is_empty() {
        return;
    }
    fn flag(
        lv: &Expression,
        rv: &Expression,
        enum_vars: &HashSet<String>,
        errs: &mut Vec<String>,
    ) {
        // Only a whole-variable target (`e = ...`), never `e[i] = ...`.
        if matches!(lv.kind, ExprKind::Ident(_)) {
            if let Some(b) = base_ident(lv) {
                if enum_vars.contains(&b) && is_bare_integer_rhs(rv) {
                    errs.push(format!(
                        "assignment to enum variable '{}' requires an explicit cast \
                         (LRM 1800-2017 §6.19.3)",
                        b
                    ));
                }
            }
        }
    }
    for it in items {
        let stmt = match it {
            ModuleItem::AlwaysConstruct(a) => &a.stmt,
            ModuleItem::InitialConstruct(i) => &i.stmt,
            _ => continue,
        };
        for_each_assign_pair(stmt, &mut |lv, rv| flag(lv, rv, &enum_vars, errs));
    }
    for it in items {
        if let ModuleItem::ContinuousAssign(ca) = it {
            for (l, r) in &ca.assignments {
                flag(l, r, &enum_vars, errs);
            }
        }
    }
}

/// A definitely-scalar RHS: an integer literal or a scalar operator expression
/// (arithmetic / shift / bitwise / comparison / reduction). Excludes idents,
/// assignment patterns, concatenations and calls — the shapes that legally
/// produce an aggregate — so only an unambiguous scalar fires the array check.
fn is_scalar_rhs(e: &Expression) -> bool {
    match &e.kind {
        ExprKind::Number(_) => true,
        ExprKind::Binary { .. } | ExprKind::Unary { .. } => true,
        ExprKind::Paren(inner) => is_scalar_rhs(inner),
        _ => false,
    }
}

/// §6.24 / §10.7: a scalar value cannot be implicitly assigned to an unpacked
/// (dynamic / queue) array target — it needs an assignment pattern or a cast.
/// Narrow: only a whole dynamic-array LHS with a clearly-scalar RHS fires.
fn check_dynarray_assign(items: &[ModuleItem], elab: &ElaboratedModule, errs: &mut Vec<String>) {
    if elab.dynamic_arrays.is_empty() {
        return;
    }
    fn flag(lv: &Expression, rv: &Expression, elab: &ElaboratedModule, errs: &mut Vec<String>) {
        if matches!(lv.kind, ExprKind::Ident(_)) {
            if let Some(b) = base_ident(lv) {
                if elab.dynamic_arrays.contains(&b) && is_scalar_rhs(rv) {
                    errs.push(format!(
                        "scalar expression cannot be implicitly cast to unpacked-array \
                         variable '{}' (LRM 1800-2017 §10.7)",
                        b
                    ));
                }
            }
        }
    }
    for it in items {
        let stmt = match it {
            ModuleItem::AlwaysConstruct(a) => &a.stmt,
            ModuleItem::InitialConstruct(i) => &i.stmt,
            _ => continue,
        };
        for_each_assign_pair(stmt, &mut |lv, rv| flag(lv, rv, elab, errs));
    }
    for it in items {
        if let ModuleItem::ContinuousAssign(ca) = it {
            for (l, r) in &ca.assignments {
                flag(l, r, elab, errs);
            }
        }
    }
}

/// Call `f(lvalue, rvalue)` for each procedural blocking/nonblocking assignment.
fn for_each_assign_pair(stmt: &Statement, f: &mut dyn FnMut(&Expression, &Expression)) {
    match &stmt.kind {
        StatementKind::BlockingAssign { lvalue, rvalue }
        | StatementKind::NonblockingAssign { lvalue, rvalue, .. } => f(lvalue, rvalue),
        StatementKind::If {
            then_stmt,
            else_stmt,
            ..
        } => {
            for_each_assign_pair(then_stmt, f);
            if let Some(e) = else_stmt {
                for_each_assign_pair(e, f);
            }
        }
        StatementKind::Case { items, .. } => {
            for item in items {
                for_each_assign_pair(&item.stmt, f);
            }
        }
        StatementKind::For { body, .. }
        | StatementKind::Foreach { body, .. }
        | StatementKind::While { body, .. }
        | StatementKind::DoWhile { body, .. }
        | StatementKind::Repeat { body, .. }
        | StatementKind::Forever { body } => for_each_assign_pair(body, f),
        StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
            for s in stmts {
                for_each_assign_pair(s, f);
            }
        }
        StatementKind::TimingControl { stmt, .. } | StatementKind::Wait { stmt, .. } => {
            for_each_assign_pair(stmt, f)
        }
        _ => {}
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

/// Visit every statement node, recursing into blocks, loops and branches.
fn for_each_stmt(stmt: &Statement, f: &mut dyn FnMut(&Statement)) {
    f(stmt);
    match &stmt.kind {
        StatementKind::If {
            then_stmt,
            else_stmt,
            ..
        } => {
            for_each_stmt(then_stmt, f);
            if let Some(e) = else_stmt {
                for_each_stmt(e, f);
            }
        }
        StatementKind::Case { items, .. } => {
            for it in items {
                for_each_stmt(&it.stmt, f);
            }
        }
        StatementKind::For { body, .. }
        | StatementKind::Foreach { body, .. }
        | StatementKind::While { body, .. }
        | StatementKind::DoWhile { body, .. }
        | StatementKind::Repeat { body, .. }
        | StatementKind::Forever { body }
        | StatementKind::TimingControl { stmt: body, .. }
        | StatementKind::Wait { stmt: body, .. } => for_each_stmt(body, f),
        StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
            stmts.iter().for_each(|s| for_each_stmt(s, f));
        }
        _ => {}
    }
}

/// §11.4.14.3: a streaming-concatenation source assigned to a *fixed-size*
/// target must not be wider than the target (a wider target is zero-padded; a
/// narrower one is an error). We compute the source width as the sum of the
/// operand widths and the target width from its (fixed integral) type.
///
/// Conservative by construction:
///  - only fixed integral targets with no unpacked dimension (dynamic arrays /
///    queues resize, so they are never an error and are skipped);
///  - the source width is taken ONLY when every operand is a plain in-scope
///    identifier of known fixed width — any unresolved operand bails the check.
fn check_stream_widths(
    items: &[ModuleItem],
    elab: &ElaboratedModule,
    errs: &mut Vec<String>,
) {
    let vw = build_var_widths(items, elab);
    for it in items {
        match it {
            ModuleItem::DataDeclaration(d) => {
                for decl in &d.declarators {
                    check_stream_decl(
                        &d.data_type,
                        decl.dimensions.is_empty(),
                        &decl.init,
                        &decl.name.name,
                        &vw,
                        elab,
                        errs,
                    );
                }
            }
            ModuleItem::AlwaysConstruct(a) => {
                for_each_stmt(&a.stmt, &mut |s| check_stmt_stream(s, &vw, elab, errs));
            }
            ModuleItem::InitialConstruct(i) => {
                for_each_stmt(&i.stmt, &mut |s| check_stmt_stream(s, &vw, elab, errs));
            }
            _ => {}
        }
    }
}

fn check_stmt_stream(
    s: &Statement,
    vw: &std::collections::HashMap<String, u32>,
    elab: &ElaboratedModule,
    errs: &mut Vec<String>,
) {
    if let StatementKind::VarDecl {
        data_type,
        declarators,
        ..
    } = &s.kind
    {
        for decl in declarators {
            check_stream_decl(
                data_type,
                decl.dimensions.is_empty(),
                &decl.init,
                &decl.name.name,
                vw,
                elab,
                errs,
            );
        }
    }
}

fn check_stream_decl(
    dt: &DataType,
    dims_empty: bool,
    init: &Option<Expression>,
    name: &str,
    vw: &std::collections::HashMap<String, u32>,
    elab: &ElaboratedModule,
    errs: &mut Vec<String>,
) {
    if !dims_empty {
        return;
    }
    let Some(init) = init else { return };
    let ExprKind::StreamOp { exprs, .. } = &init.kind else {
        return;
    };
    let Some(target_w) = fixed_int_width(dt, elab) else {
        return;
    };
    let Some(stream_w) = stream_width(exprs, vw) else {
        return;
    };
    if stream_w > target_w {
        errs.push(format!(
            "stream '{name}': source width {stream_w} exceeds fixed-size target width {target_w} \
             (LRM 1800-2017 §11.4.14.3 — streaming source wider than target)"
        ));
    }
}

/// Map of in-module scalar identifier -> fixed integral width.
fn build_var_widths(
    items: &[ModuleItem],
    elab: &ElaboratedModule,
) -> std::collections::HashMap<String, u32> {
    let mut m = std::collections::HashMap::new();
    for it in items {
        match it {
            ModuleItem::DataDeclaration(d) => {
                for decl in &d.declarators {
                    if decl.dimensions.is_empty() {
                        if let Some(w) = fixed_int_width(&d.data_type, elab) {
                            m.insert(decl.name.name.clone(), w);
                        }
                    }
                }
            }
            ModuleItem::NetDeclaration(d) => {
                for decl in &d.declarators {
                    if decl.dimensions.is_empty() {
                        if let Some(w) = fixed_int_width(&d.data_type, elab) {
                            m.insert(decl.name.name.clone(), w);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    m
}

/// Width of a fixed-size integral data type; None for non-integral or 0-width.
fn fixed_int_width(dt: &DataType, elab: &ElaboratedModule) -> Option<u32> {
    if !matches!(
        dt,
        DataType::IntegerVector { .. } | DataType::IntegerAtom { .. }
    ) {
        return None;
    }
    let w = xezim_core::elaborate::resolve_type_width(
        dt,
        Some(&elab.parameters),
        Some(&elab.typedefs),
    );
    if w == 0 {
        None
    } else {
        Some(w)
    }
}

/// Sum of streaming-concat operand widths; None if ANY operand is not a plain
/// in-scope identifier of known width (the check then bails, never flagging).
fn stream_width(exprs: &[Expression], vw: &std::collections::HashMap<String, u32>) -> Option<u32> {
    if exprs.is_empty() {
        return None;
    }
    let mut total: u32 = 0;
    for e in exprs {
        let ExprKind::Ident(h) = &e.kind else {
            return None;
        };
        if h.root.is_some() || h.path.len() != 1 || !h.path[0].selects.is_empty() {
            return None;
        }
        let w = vw.get(&h.path[0].name.name)?;
        total = total.checked_add(*w)?;
    }
    Some(total)
}

/// §6.19: in an enum with an explicit base type, a member whose value is a
/// *sized* literal constant must match the base-type width.
fn check_enum_type(dt: &DataType, elab: &ElaboratedModule, errs: &mut Vec<String>) {
    let DataType::Enum(et) = dt else { return };
    // Sized-literal-width check requires an explicit base type.
    if let Some(base) = &et.base_type {
        let w = xezim_core::elaborate::resolve_type_width(
            base,
            Some(&elab.parameters),
            Some(&elab.typedefs),
        );
        if w != 0 {
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
    }
    check_enum_values(et, elab, errs);
}

/// True if `dt` (an enum base type) is a SIGNED integral type. `bit`/`logic`/
/// `reg` vectors default to unsigned; `byte`/`shortint`/`int`/`longint`/
/// `integer` default to signed; `time` is unsigned; no base type == `int`
/// (signed). An explicit `signed`/`unsigned` qualifier wins.
fn enum_base_is_signed(base: Option<&DataType>) -> bool {
    match base {
        None => true,
        Some(DataType::IntegerVector { signing, .. })
        | Some(DataType::Implicit { signing, .. }) => matches!(signing, Some(Signing::Signed)),
        Some(DataType::IntegerAtom { kind, signing, .. }) => match signing {
            Some(Signing::Signed) => true,
            Some(Signing::Unsigned) => false,
            None => !matches!(kind, IntegerAtomType::Time),
        },
        _ => true,
    }
}

/// If `e` (after unwrapping a leading unary `+`/`-`) is a *sized* integer
/// literal, return its declared bit-size. Used to recognize an enum
/// initializer that fits its base type by construction (`-4'sd1` in a 4-bit
/// base), which must not be flagged as out of range.
fn sized_literal_size(e: &Expression) -> Option<u32> {
    use xezim_core::ast::expr::UnaryOp;
    let inner = match &e.kind {
        ExprKind::Unary { op: UnaryOp::Minus | UnaryOp::Plus, operand } => operand.as_ref(),
        _ => e,
    };
    if let ExprKind::Number(NumberLiteral::Integer { size: Some(s), .. }) = &inner.kind {
        Some(*s)
    } else {
        None
    }
}

/// True if `e` is a based literal containing an x/z digit (an unknown value).
fn expr_is_xz_number(e: &Expression) -> bool {
    if let ExprKind::Number(NumberLiteral::Integer { base, value, .. }) = &e.kind {
        if !matches!(base, NumberBase::Decimal) {
            return value.chars().any(|c| matches!(c, 'x' | 'X' | 'z' | 'Z' | '?'));
        }
    }
    false
}

/// True if `e` references a simulation-time system function ($time, $random,
/// …) anywhere — such an expression is not an elaboration-time constant.
fn expr_contains_syscall(e: &Expression) -> bool {
    let mut found = false;
    for_each_expr(e, &mut |x| {
        if matches!(&x.kind, ExprKind::SystemCall { .. }) {
            found = true;
        }
    });
    found
}

/// §6.19: value-domain rules for enum members. Only fires on IntegerVector /
/// IntegerAtom / default (`int`) bases whose width and signedness are known —
/// a typedef/struct base bails (conservative). Detects: a member value outside
/// the base type's range (explicit or auto-incremented / "inferred overflow"),
/// two members with the same value, an x/z-valued 2-state member, a
/// non-constant initializer, and an undefined/negative enum name-sequence bound.
fn check_enum_values(et: &EnumType, elab: &ElaboratedModule, errs: &mut Vec<String>) {
    let base = et.base_type.as_deref();
    // Only reason about built-in integral bases; a typedef/struct/enum base has
    // width/signedness we don't resolve here — skip to avoid false positives.
    match base {
        None
        | Some(DataType::IntegerVector { .. })
        | Some(DataType::IntegerAtom { .. })
        | Some(DataType::Implicit { .. }) => {}
        _ => return,
    }
    let width = base
        .map(|b| xezim_core::elaborate::resolve_type_width(b, Some(&elab.parameters), Some(&elab.typedefs)))
        .unwrap_or(32);
    if width == 0 || width > 64 {
        return;
    }
    let signed = enum_base_is_signed(base);
    let (min, max): (i128, i128) = if signed {
        (-(1i128 << (width - 1)), (1i128 << (width - 1)) - 1)
    } else {
        (0, (1i128 << width) - 1)
    };
    let mask: u128 = if width >= 128 { u128::MAX } else { (1u128 << width) - 1 };
    let params = Some(&elab.parameters);
    let mut next: i128 = 0;
    let mut seen: std::collections::HashMap<u128, String> = std::collections::HashMap::new();

    // One "value slot" of the enum: a plain member is one slot; a member with a
    // name range `foo[lo:hi]` is (hi-lo+1) slots, the FIRST of which takes the
    // initializer (if any) and the rest auto-increment.
    for m in &et.members {
        // Number of value slots this member contributes, plus a legality check
        // on the name-sequence bounds (§6.19: must be a defined, non-negative
        // constant).
        let count: i64 = match &m.range {
            None => 1,
            Some((lo_e, hi_e)) => {
                if expr_is_xz_number(lo_e) || expr_is_xz_number(hi_e) {
                    errs.push(format!(
                        "enum name sequence '{}' has an undefined (x/z) bound (LRM 1800-2017 §6.19)",
                        m.name.name
                    ));
                    return;
                }
                match (
                    xezim_core::elaborate::const_eval_i64_with_params(lo_e, params),
                    xezim_core::elaborate::const_eval_i64_with_params(hi_e, params),
                ) {
                    (Some(l), Some(h)) => {
                        if l < 0 || h < 0 {
                            errs.push(format!(
                                "enum name sequence '{}' has a negative or zero bound (LRM 1800-2017 §6.19)",
                                m.name.name
                            ));
                            return;
                        }
                        (h - l).abs() + 1
                    }
                    // Non-constant bound (e.g. a parameter we can't fold): bail
                    // out of value tracking rather than risk a false positive.
                    _ => return,
                }
            }
        };

        for slot in 0..count {
            // The initializer applies to the first slot only.
            let val: i128 = if slot == 0 {
                if let Some(init) = &m.init {
                    match xezim_core::elaborate::const_eval_i64_with_params(init, params) {
                        Some(v) => v as i128,
                        None => {
                            if expr_contains_syscall(init) {
                                errs.push(format!(
                                    "enum member '{}' initializer is not a constant expression (LRM 1800-2017 §6.19)",
                                    m.name.name
                                ));
                            }
                            // Value unknown — stop tracking to avoid false dup/range hits.
                            return;
                        }
                    }
                } else {
                    next
                }
            } else {
                next
            };

            let explicit = slot == 0 && m.init.is_some();
            // A SIZED literal initializer whose declared size fits the base
            // width is legal even if its signed value looks out of range — the
            // bits simply wrap into the base type (e.g. `-4'sd1` in a 4-bit
            // base == 4'b1111). Skip the numeric range check for it; the
            // sized-literal-width check above already rejects a size *mismatch*.
            let sized_fits = slot == 0
                && m.init
                    .as_ref()
                    .and_then(sized_literal_size)
                    .map_or(false, |s| s <= width);
            if !sized_fits && (val > max || val < min) {
                if explicit {
                    if val < 0 {
                        errs.push(format!(
                            "enum member '{}' has a negative value {} out of range for its base type (LRM 1800-2017 §6.19)",
                            m.name.name, val
                        ));
                    } else {
                        errs.push(format!(
                            "enum member '{}' has a value {} too large for its base type (LRM 1800-2017 §6.19)",
                            m.name.name, val
                        ));
                    }
                } else {
                    errs.push(format!(
                        "enum member '{}' has an inferred value {} that overflowed its base type (LRM 1800-2017 §6.19)",
                        m.name.name, val
                    ));
                }
            }
            let key = (val as u128) & mask;
            if let Some(prev) = seen.get(&key) {
                errs.push(format!(
                    "enum members '{}' and '{}' have the same value {} (LRM 1800-2017 §6.19)",
                    m.name.name, prev, key
                ));
            } else {
                seen.insert(key, m.name.name.clone());
            }
            next = val + 1;
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
    // Per-method checks: output/inout port defaults, and (for the constructor)
    // that `super.new(...)` is the first statement.
    let has_base = c.extends.is_some();
    for item in &c.items {
        if let ClassItem::Method(m) = item {
            let (ports, body, is_new) = match &m.kind {
                ClassMethodKind::Function(f) => {
                    (&f.ports, Some(&f.items), f.name.name.name == "new")
                }
                ClassMethodKind::Task(t) => (&t.ports, Some(&t.items), t.name.name.name == "new"),
                _ => continue,
            };
            check_output_port_defaults(ports, errs);
            if is_new && has_base {
                if let Some(stmts) = body {
                    check_super_new_first(stmts, errs);
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

/// True if statement `s` is a bare `super.new(...)` call.
fn stmt_is_super_new(s: &Statement) -> bool {
    let StatementKind::Expr(e) = &s.kind else { return false };
    let ExprKind::Call { func, .. } = &e.kind else { return false };
    let ExprKind::MemberAccess { expr, member } = &func.kind else { return false };
    if member.name != "new" {
        return false;
    }
    matches!(&expr.kind, ExprKind::Ident(h)
        if h.path.len() == 1 && h.path[0].name.name == "super")
}

/// A statement that may legally precede `super.new` (a local declaration, a
/// null statement, or a scope marker) — anything else is an executable
/// statement and makes a following `super.new` illegal.
fn is_leading_nonexec(s: &Statement) -> bool {
    matches!(
        s.kind,
        StatementKind::VarDecl { .. }
            | StatementKind::Typedef(_)
            | StatementKind::Null
            | StatementKind::ScopePop
    )
}

/// §8.15: if a constructor calls `super.new(...)`, it must be the first
/// executable statement (only local declarations may precede it).
fn check_super_new_first(stmts: &[Statement], errs: &mut Vec<String>) {
    let Some(idx) = stmts.iter().position(stmt_is_super_new) else {
        return;
    };
    if stmts[..idx].iter().any(|s| !is_leading_nonexec(s)) {
        errs.push(
            "super.new(...) must be the first statement in the constructor (LRM 1800-2017 §8.15)"
                .to_string(),
        );
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

// ---------------------------------------------------------------------------
// §11.4.6 — wildcard-equality (`==?` / `!=?`) operand-type rule.
// ---------------------------------------------------------------------------

use xezim_core::ast::expr::BinaryOp;
use xezim_core::ast::types::SimpleType;

/// True if `dt` is a real or string type (non-integral for `==?`/`!=?`).
fn is_real_or_string(dt: &DataType) -> bool {
    matches!(
        dt,
        DataType::Real { .. } | DataType::Simple { kind: SimpleType::String, .. }
    )
}

/// One `==?`/`!=?` operand is illegal when it is a real/string literal or a
/// declared real/string variable.
fn wildcard_operand_illegal(e: &Expression, nonintegral: &std::collections::HashSet<String>) -> bool {
    match &e.kind {
        ExprKind::Number(NumberLiteral::Real(_)) => true,
        ExprKind::StringLiteral(_) => true,
        ExprKind::Ident(h) => {
            h.root.is_none()
                && h.path.len() == 1
                && h.path[0].selects.is_empty()
                && nonintegral.contains(&h.path[0].name.name)
        }
        _ => false,
    }
}

/// §11.4.6: the wildcard-equality operators `==?` and `!=?` require INTEGRAL
/// operands. A real- or string-typed operand is illegal.
fn check_wildcard_cmp(items: &[ModuleItem], elab: &ElaboratedModule, errs: &mut Vec<String>) {
    // Names of real / string variables declared in this scope.
    let mut nonintegral: std::collections::HashSet<String> = std::collections::HashSet::new();
    for it in items {
        match it {
            ModuleItem::DataDeclaration(d) if is_real_or_string(&d.data_type) => {
                for decl in &d.declarators {
                    nonintegral.insert(decl.name.name.clone());
                }
            }
            ModuleItem::NetDeclaration(d) if is_real_or_string(&d.data_type) => {}
            _ => {}
        }
    }
    let mut scan = |e: &Expression, errs: &mut Vec<String>| {
        for_each_expr(e, &mut |x| {
            if let ExprKind::Binary { op, left, right } = &x.kind {
                if matches!(op, BinaryOp::WildcardEq | BinaryOp::WildcardNeq) {
                    if wildcard_operand_illegal(left, &nonintegral)
                        || wildcard_operand_illegal(right, &nonintegral)
                    {
                        errs.push(
                            "wildcard-equality operator (==? / !=?) requires integral operands; \
                             a real or string operand is illegal (LRM 1800-2017 §11.4.6)"
                                .to_string(),
                        );
                    }
                }
            }
        });
    };
    for it in items {
        match it {
            ModuleItem::ParameterDeclaration(p) | ModuleItem::LocalparamDeclaration(p) => {
                if let xezim_core::ast::decl::ParameterKind::Data { assignments, .. } = &p.kind {
                    for a in assignments {
                        if let Some(init) = &a.init {
                            scan(init, errs);
                        }
                    }
                }
            }
            ModuleItem::DataDeclaration(d) => {
                for decl in &d.declarators {
                    if let Some(init) = &decl.init {
                        scan(init, errs);
                    }
                }
            }
            ModuleItem::ContinuousAssign(ca) => {
                for (l, r) in &ca.assignments {
                    scan(l, errs);
                    scan(r, errs);
                }
            }
            ModuleItem::AlwaysConstruct(a) => {
                for_each_stmt_expr(&a.stmt, &mut |e| scan(e, errs));
            }
            ModuleItem::InitialConstruct(i) => {
                for_each_stmt_expr(&i.stmt, &mut |e| scan(e, errs));
            }
            ModuleItem::FinalConstruct(fc) => {
                for_each_stmt_expr(&fc.stmt, &mut |e| scan(e, errs));
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// §24.3 — constructs illegal inside a program block.
// ---------------------------------------------------------------------------

/// §24.3: a program block may not contain always procedures or module/gate
/// instantiations.
fn check_program_items(items: &[ModuleItem], errs: &mut Vec<String>) {
    for it in items {
        match it {
            ModuleItem::AlwaysConstruct(_) => errs.push(
                "an always procedure is not allowed in a program block (LRM 1800-2017 §24.3)"
                    .to_string(),
            ),
            ModuleItem::ModuleInstantiation(_) => errs.push(
                "a module instantiation is not allowed in a program block (LRM 1800-2017 §24.3)"
                    .to_string(),
            ),
            ModuleItem::GateInstantiation(_) => errs.push(
                "a gate instantiation is not allowed in a program block (LRM 1800-2017 §24.3)"
                    .to_string(),
            ),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// §7.4 / §7.10 — illegal packed & unpacked dimensions.
// ---------------------------------------------------------------------------

/// Packed dimensions carried directly by a data type, if any.
fn packed_dims_of(dt: &DataType) -> &[PackedDimension] {
    match dt {
        DataType::IntegerVector { dimensions, .. }
        | DataType::Implicit { dimensions, .. }
        | DataType::TypeReference { dimensions, .. } => dimensions,
        _ => &[],
    }
}

/// §7.4.1: an unsized packed dimension `[]` is not allowed on an ordinary
/// net/variable declaration.
fn check_packed_dims(dt: &DataType, _elab: &ElaboratedModule, errs: &mut Vec<String>) {
    for pd in packed_dims_of(dt) {
        if let PackedDimension::Unsized(_) = pd {
            errs.push(
                "an unsized packed dimension `[]` is not allowed here (LRM 1800-2017 §7.4.1)"
                    .to_string(),
            );
        }
    }
}

/// §7.4/§7.10: unpacked-dimension legality for a declarator.
///  - a fixed-size array dimension `[N]` must have N > 0;
///  - a queue bound `[$:N]` must be a defined, non-negative constant.
fn check_unpacked_dims(
    name: &str,
    dims: &[UnpackedDimension],
    elab: &ElaboratedModule,
    errs: &mut Vec<String>,
) {
    let params = Some(&elab.parameters);
    for d in dims {
        match d {
            UnpackedDimension::Expression { expr, .. } => {
                if let Some(v) = xezim_core::elaborate::const_eval_i64_with_params(expr, params) {
                    if v <= 0 {
                        errs.push(format!(
                            "array '{}' dimension size must be greater than zero (LRM 1800-2017 §7.4)",
                            name
                        ));
                    }
                }
            }
            UnpackedDimension::Queue { max_size: Some(bound), .. } => {
                if expr_is_xz_number(bound) {
                    errs.push(format!(
                        "queue '{}' bound must be a defined value (LRM 1800-2017 §7.10)",
                        name
                    ));
                } else {
                    match xezim_core::elaborate::const_eval_i64_with_params(bound, params) {
                        Some(v) if v < 0 => errs.push(format!(
                            "queue '{}' bound must be positive (LRM 1800-2017 §7.10)",
                            name
                        )),
                        None => errs.push(format!(
                            "queue '{}' bound must be a constant (LRM 1800-2017 §7.10)",
                            name
                        )),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// §6.8 — the dynamic-array `new[]` constructor target.
// ---------------------------------------------------------------------------

/// True if `e` is a `new`-construction expression (`new(...)` / `new[...]`),
/// which the parser lowers to `Call { func: Ident("new"), .. }`.
fn is_new_construction(e: &Expression) -> bool {
    if let ExprKind::Call { func, .. } = &e.kind {
        if let ExprKind::Ident(h) = &func.kind {
            return h.root.is_none() && h.path.len() == 1 && h.path[0].name.name == "new";
        }
    }
    false
}

/// §6.8/§7.5: the `new[n]` array constructor may only initialize a *dynamic*
/// array. Assigning it to a fixed integral (packed) variable with no unpacked
/// `[]` dimension is illegal (e.g. `logic [1:0] a = new[4];`).
fn check_new_array_target(dt: &DataType, decl: &VarDeclarator, errs: &mut Vec<String>) {
    let Some(init) = &decl.init else { return };
    if !is_new_construction(init) {
        return;
    }
    // Only reason about plainly-integral targets — a class handle legitimately
    // takes `new(...)`, and a typedef could alias a dynamic array.
    if !matches!(
        dt,
        DataType::IntegerVector { .. } | DataType::IntegerAtom { .. } | DataType::Implicit { .. }
    ) {
        return;
    }
    let has_dynamic_dim = decl
        .dimensions
        .iter()
        .any(|d| matches!(d, UnpackedDimension::Unsized(_)));
    if !has_dynamic_dim {
        errs.push(format!(
            "the `new[]` array constructor may only initialize a dynamic array, not '{}' \
             (LRM 1800-2017 §7.5)",
            decl.name.name
        ));
    }
}

// ---------------------------------------------------------------------------
// §23.3.2 — named port connection must name a real port.
// ---------------------------------------------------------------------------

use std::collections::{HashMap, HashSet};
use xezim_core::ast::module::PortList;

/// Set of declared port names for a module/interface/program, or None when the
/// port list is empty/unknown (so no connection is ever flagged against it).
fn port_names(pl: &PortList) -> Option<HashSet<String>> {
    match pl {
        PortList::Ansi(ports) => Some(ports.iter().map(|p| p.name.name.clone()).collect()),
        PortList::NonAnsi(names) => Some(names.iter().map(|n| n.name.clone()).collect()),
        PortList::Empty => None,
    }
}

/// Build name -> declared-port-name-set for every module/interface/program.
fn build_port_map(defs: &[&SourceDefinition]) -> HashMap<String, HashSet<String>> {
    let mut m = HashMap::new();
    for def in defs {
        let (name, pl) = match def {
            SourceDefinition::Module(md) => (&md.name.name, &md.ports),
            SourceDefinition::Interface(id) => (&id.name.name, &id.ports),
            SourceDefinition::Program(pd) => (&pd.name.name, &pd.ports),
            _ => continue,
        };
        if let Some(set) = port_names(pl) {
            m.insert(name.clone(), set);
        }
    }
    m
}

/// §23.3.2: a `.name(...)` (or `.name` implicit) connection in an instantiation
/// must refer to a port that the target module actually declares.
fn check_instantiations(
    items: &[ModuleItem],
    port_map: &HashMap<String, HashSet<String>>,
    errs: &mut Vec<String>,
) {
    for it in items {
        let ModuleItem::ModuleInstantiation(inst) = it else {
            continue;
        };
        let Some(ports) = port_map.get(&inst.module_name.name) else {
            continue; // unknown target (primitive, library cell, ...) — skip
        };
        for instance in &inst.instances {
            for conn in &instance.connections {
                if let xezim_core::ast::decl::PortConnection::Named { name, .. } = conn {
                    if !ports.contains(&name.name) {
                        errs.push(format!(
                            "port '{}' is not a port of module '{}' (LRM 1800-2017 §23.3.2)",
                            name.name, inst.module_name.name
                        ));
                    }
                }
            }
        }
    }
}

/// Names visible in a module scope for implicit-port matching. Returns None
/// (bailing the check) when the scope can gain names we don't track here — any
/// package import or generate construct — to avoid false positives.
fn collect_scope_names(ports: &PortList, items: &[ModuleItem]) -> Option<HashSet<String>> {
    use xezim_core::ast::decl::ParameterKind;
    let mut names: HashSet<String> = HashSet::new();
    match ports {
        PortList::Ansi(ps) => {
            for p in ps {
                names.insert(p.name.name.clone());
            }
        }
        PortList::NonAnsi(ns) => {
            for n in ns {
                names.insert(n.name.clone());
            }
        }
        PortList::Empty => {}
    }
    for it in items {
        match it {
            ModuleItem::ImportDeclaration(_)
            | ModuleItem::GenerateRegion(_)
            | ModuleItem::GenerateIf(_)
            | ModuleItem::GenerateFor(_)
            | ModuleItem::GenerateCase(_) => return None,
            ModuleItem::NetDeclaration(d) => {
                for decl in &d.declarators {
                    names.insert(decl.name.name.clone());
                }
            }
            ModuleItem::DataDeclaration(d) => {
                for decl in &d.declarators {
                    names.insert(decl.name.name.clone());
                }
            }
            ModuleItem::PortDeclaration(d) => {
                for decl in &d.declarators {
                    names.insert(decl.name.name.clone());
                }
            }
            ModuleItem::ParameterDeclaration(p) | ModuleItem::LocalparamDeclaration(p) => {
                if let ParameterKind::Data { assignments, .. } = &p.kind {
                    for a in assignments {
                        names.insert(a.name.name.clone());
                    }
                }
            }
            ModuleItem::GenvarDeclaration(g) => {
                for n in &g.names {
                    names.insert(n.name.clone());
                }
            }
            // Instance names (esp. interface instances) are valid connection
            // targets — e.g. `test_if test_intf(...)` then `.*` binding a
            // `test_if` port. Include them so those don't read as "missing".
            ModuleItem::ModuleInstantiation(mi) => {
                for inst in &mi.instances {
                    names.insert(inst.name.name.clone());
                }
            }
            _ => {}
        }
    }
    Some(names)
}

/// §23.3.2.2/§23.3.2.4: an implicit `.name` connection and a `.*` wildcard
/// connection each require a same-named signal to exist in the instantiating
/// scope. Flags a `.name` / `.*` port with no matching signal.
fn check_implicit_ports(
    ports: &PortList,
    items: &[ModuleItem],
    port_map: &HashMap<String, HashSet<String>>,
    errs: &mut Vec<String>,
) {
    let Some(scope) = collect_scope_names(ports, items) else {
        return;
    };
    for it in items {
        let ModuleItem::ModuleInstantiation(inst) = it else {
            continue;
        };
        for instance in &inst.instances {
        // Ports explicitly listed by name (`.p(...)` or the no-connect `.p()`)
        // are NOT filled by `.*` and impose no implicit-net requirement.
        let explicit: HashSet<String> = instance
            .connections
            .iter()
            .filter_map(|c| match c {
                xezim_core::ast::decl::PortConnection::Named { name, .. } => {
                    Some(name.name.clone())
                }
                _ => None,
            })
            .collect();
        for conn in &instance.connections {
            match conn {
                // Only a parenthesis-free `.name` (implicit: true) requires a
                // same-named net; `.name()` is an explicit no-connect.
                xezim_core::ast::decl::PortConnection::Named {
                    name,
                    expr: None,
                    implicit: true,
                } => {
                    if !scope.contains(&name.name) {
                        errs.push(format!(
                            "implicit port connection '.{}' has no matching signal in the \
                             enclosing scope (LRM 1800-2017 §23.3.2.2)",
                            name.name
                        ));
                    }
                }
                xezim_core::ast::decl::PortConnection::Wildcard => {
                    if let Some(tports) = port_map.get(&inst.module_name.name) {
                        let mut missing: Vec<&String> = tports
                            .iter()
                            .filter(|p| !scope.contains(*p) && !explicit.contains(*p))
                            .collect();
                        missing.sort();
                        for p in missing {
                            errs.push(format!(
                                "wildcard port connection (.*) found no matching signal for \
                                 port '{}' (LRM 1800-2017 §23.3.2.4)",
                                p
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
        }
    }
}
