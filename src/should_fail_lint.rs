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

use xezim_core::SourceDefinition;
use xezim_core::ast::decl::{
    ClassDeclaration, ClassItem, ClassMethodKind, ClassQualifier, ModuleItem, TypedefDeclaration,
};
use xezim_core::ast::expr::{
    AssignmentPatternItem, ExprKind, Expression, NumberBase, NumberLiteral, RangeKind,
};
use xezim_core::ast::stmt::{Statement, StatementKind, VarDeclarator};
use xezim_core::ast::types::{
    DataType, EnumType, IntegerAtomType, PackedDimension, PortDirection, Signing, UnpackedDimension,
};
use xezim_core::elaborate::ElaboratedModule;

/// Run the second-pass lint over every top-level definition. Returns a list of
/// error messages (empty == clean).
pub fn lint_should_fail(defs: &[&SourceDefinition], elab: &ElaboratedModule) -> Vec<String> {
    let mut errs = Vec::new();
    // §23.3.2: map every module/interface/program to its declared port names,
    // so a named port connection to a non-existent port can be rejected.
    let port_map = build_port_map(defs);
    let pkg_decls = package_decls(defs);
    let pkg_names: HashSet<String> = pkg_decls.values().flatten().cloned().collect();
    for def in defs {
        match def {
            SourceDefinition::Class(c) => check_class(c, &mut errs),
            SourceDefinition::Typedef(t) => {
                check_typedef(t, elab, &Default::default(), &mut errs);
                // Width-identifier check is restricted to TOP-LEVEL typedefs
                // (no enclosing module scope), where every value parameter is
                // global and present in `elab.parameters` — so an unresolved
                // width identifier is definitely undeclared.
                check_struct_typedef_widths(t, elab, &mut errs);
            }
            SourceDefinition::Module(m) => {
                let unit = Unit {
                    name: &m.name.name,
                    params: &m.params,
                    ports: &m.ports,
                    items: &m.items,
                };
                check_unit(&unit, defs, elab, &pkg_decls, &pkg_names, &mut errs);
                let mut local_types = std::collections::HashSet::new();
                collect_local_type_names(&m.params, &m.items, &mut local_types);
                check_subroutine_port_types(&m.items, &local_types, &pkg_names, elab, &mut errs);
                for it in &m.items {
                    check_module_item(it, elab, &local_types, true, &mut errs);
                }
                check_proc_net_assign(&m.ports, &m.items, &mut errs);
                check_dynarray_assign(&m.items, elab, &mut errs);
                check_stream_widths(&m.items, elab, &mut errs);
                check_wildcard_cmp(&m.items, elab, &mut errs);
                check_instantiations(&m.items, &port_map, &mut errs);
                check_implicit_ports(&m.ports, &m.items, &port_map, &mut errs);
            }
            SourceDefinition::Interface(m) => {
                let unit = Unit {
                    name: &m.name.name,
                    params: &m.params,
                    ports: &m.ports,
                    items: &m.items,
                };
                check_unit(&unit, defs, elab, &pkg_decls, &pkg_names, &mut errs);
                let mut local_types = std::collections::HashSet::new();
                collect_local_type_names(&m.params, &m.items, &mut local_types);
                check_subroutine_port_types(&m.items, &local_types, &pkg_names, elab, &mut errs);
                for it in &m.items {
                    check_module_item(it, elab, &local_types, true, &mut errs);
                }
                check_stream_widths(&m.items, elab, &mut errs);
                check_wildcard_cmp(&m.items, elab, &mut errs);
                check_instantiations(&m.items, &port_map, &mut errs);
                check_implicit_ports(&m.ports, &m.items, &port_map, &mut errs);
            }
            SourceDefinition::Program(m) => {
                let unit = Unit {
                    name: &m.name.name,
                    params: &m.params,
                    ports: &m.ports,
                    items: &m.items,
                };
                check_unit(&unit, defs, elab, &pkg_decls, &pkg_names, &mut errs);
                let mut local_types = std::collections::HashSet::new();
                collect_local_type_names(&m.params, &m.items, &mut local_types);
                check_subroutine_port_types(&m.items, &local_types, &pkg_names, elab, &mut errs);
                for it in &m.items {
                    check_module_item(it, elab, &local_types, true, &mut errs);
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
            // §29 UDPs carry no should-fail lints (truth tables only).
            SourceDefinition::Udp(_) => {}
        }
    }
    check_named_block_refs(defs, &mut errs);
    if xezim_core::sv_parser::strict_checks() {
        errs.extend(crate::type_lint::check(defs, elab));
    }
    errs
}

/// A module, interface or program, as the scope checks see it.
struct Unit<'a> {
    name: &'a str,
    params: &'a [xezim_core::ast::decl::ParameterDeclaration],
    ports: &'a PortList,
    items: &'a [ModuleItem],
}

/// The declaration and reference checks shared by modules, interfaces and
/// programs. Checks that resolve names against the elaborated design run on
/// the top unit only, whose names the elaboration holds.
fn check_unit(
    u: &Unit,
    defs: &[&SourceDefinition],
    elab: &ElaboratedModule,
    pkg_decls: &HashMap<String, HashSet<String>>,
    pkg_names: &HashSet<String>,
    errs: &mut Vec<String>,
) {
    let is_top = u.name == elab.name;
    check_param_value_refs(u.params, u.items, is_top, pkg_names, elab, errs);
    check_pattern_counts(defs, u.items, elab, is_top, errs);
    check_foreach_dims(u.items, errs);
    check_select_depth(u.items, errs);
    check_port_actuals(defs, u.items, errs);
    check_generate_block_names(u.items, errs);
    check_system_task_values(u.items, errs);
    check_wildcard_import_conflicts(u.ports, u.params, u.items, pkg_decls, errs);
    check_imported_hier_refs(defs, u.items, pkg_decls, errs);
    let classes = visible_classes(defs, u.items);
    check_inherited_local_access(&classes, u.items, errs);
    check_unspecialized_class_scope(&classes, u.items, errs);
    check_udp_instance_delays(defs, u.items, errs);
    check_member_access_roots(u.items, errs);
    check_event_arguments(u.items, errs);
    check_subroutine_range_idents(u.items, is_top, pkg_names, elab, errs);
    check_param_override_idents(u.items, is_top, pkg_names, elab, errs);
    check_cont_assign_rhs_names(u.ports, u.items, is_top, pkg_names, elab, errs);
    check_nonansi_ports_declared(u.name, u.ports, u.items, errs);
}

/// Classes can appear nested inside module/interface/program bodies.
///
/// `live` is false inside a generate branch that is not (provably) the one
/// elaborated: value-dependent checks (a zero-width select) are skipped there,
/// since §27.5 never elaborates an unselected branch.
fn check_module_item(
    item: &ModuleItem,
    elab: &ElaboratedModule,
    local_types: &std::collections::HashSet<String>,
    live: bool,
    errs: &mut Vec<String>,
) {
    match item {
        ModuleItem::ClassDeclaration(c) => check_class(c, errs),
        ModuleItem::TypedefDeclaration(t) => check_typedef(t, elab, local_types, errs),
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
            // §7.4: a net array has fixed-size dimensions only.
            for decl in &d.declarators {
                let kind = decl.dimensions.iter().find_map(|dim| match dim {
                    UnpackedDimension::Unsized(_) => Some("a dynamic array"),
                    UnpackedDimension::Queue { .. } => Some("a queue"),
                    // `[N]` with a parameter N parses as an associative
                    // dimension too; only `[*]` is certainly one.
                    UnpackedDimension::Associative {
                        data_type: None, ..
                    } => Some("an associative array"),
                    _ => None,
                });
                if let Some(kind) = kind {
                    errs.push(format!(
                        "net '{}' cannot be {kind}: a net array has fixed-size dimensions \
                         only (LRM 1800-2017 §7.4)",
                        decl.name.name
                    ));
                }
            }
        }
        ModuleItem::AlwaysConstruct(a) => {
            if live {
                for_each_stmt_expr(&a.stmt, &mut |e| check_zero_slice(e, elab, errs));
            }
            check_always_has_timing_control(a, elab, errs);
        }
        ModuleItem::InitialConstruct(i) => {
            if live {
                for_each_stmt_expr(&i.stmt, &mut |e| check_zero_slice(e, elab, errs));
            }
        }
        ModuleItem::ContinuousAssign(ca) => {
            if live {
                for (l, r) in &ca.assignments {
                    for_each_expr(l, &mut |e| check_zero_slice(e, elab, errs));
                    for_each_expr(r, &mut |e| check_zero_slice(e, elab, errs));
                }
            }
        }
        ModuleItem::FunctionDeclaration(f) => check_output_port_defaults(&f.ports, errs),
        ModuleItem::TaskDeclaration(t) => check_output_port_defaults(&t.ports, errs),
        ModuleItem::GateInstantiation(g) => check_gate_terminals(g, errs),
        // The per-item checks must SEE inside generate constructs — without
        // this recursion every rule above was bypassed by wrapping the illegal
        // code in `generate begin ... end` (same walker-coverage bug class as
        // the library resolver's).
        ModuleItem::GenerateRegion(gr) => {
            for it in &gr.items {
                check_module_item(it, elab, local_types, live, errs);
            }
        }
        ModuleItem::GenerateIf(gi) => {
            // The selected branch is known only while every condition before
            // it evaluates confidently.
            let mut decided = live;
            let mut taken = false;
            for (c, items) in &gi.branches {
                let this_live = match c {
                    _ if !decided || taken => false,
                    None => true,
                    Some(c) => match (
                        width_is_confident(c, elab),
                        xezim_core::elaborate::const_eval_i64_with_params(
                            c,
                            Some(&elab.parameters),
                        ),
                    ) {
                        (true, Some(v)) => v != 0,
                        _ => {
                            decided = false;
                            false
                        }
                    },
                };
                taken |= this_live;
                for it in items {
                    check_module_item(it, elab, local_types, this_live, errs);
                }
            }
        }
        ModuleItem::GenerateFor(gf) => {
            for it in &gf.items {
                check_module_item(it, elab, local_types, live, errs);
            }
        }
        ModuleItem::GenerateCase(gc) => {
            for arm in &gc.arms {
                for it in &arm.items {
                    check_module_item(it, elab, local_types, false, errs);
                }
            }
        }
        _ => {}
    }
}

/// §28.3–§28.9: each gate and switch primitive takes a fixed number of
/// terminals (n-input and n-output gates: two or more).
fn check_gate_terminals(g: &xezim_core::ast::decl::GateInstantiation, errs: &mut Vec<String>) {
    use xezim_core::ast::decl::GateType as G;
    let (min, max, what) = match g.gate_type {
        G::And | G::Nand | G::Or | G::Nor | G::Xor | G::Xnor => {
            (2, usize::MAX, "an output and at least one input")
        }
        G::Buf | G::Not => (2, usize::MAX, "at least one output and an input"),
        G::Bufif0 | G::Bufif1 | G::Notif0 | G::Notif1 => {
            (3, 3, "an output, an input and a control")
        }
        G::Nmos | G::Pmos | G::Rnmos | G::Rpmos => (3, 3, "an output, an input and a control"),
        G::Cmos | G::Rcmos => (4, 4, "an output, an input and two controls"),
        G::Tran | G::Rtran => (2, 2, "two inout terminals"),
        G::Tranif0 | G::Tranif1 | G::Rtranif0 | G::Rtranif1 => {
            (3, 3, "two inout terminals and a control")
        }
        G::Pullup | G::Pulldown => (1, usize::MAX, "at least one terminal"),
    };
    for inst in &g.instances {
        let n = inst.terminals.len();
        if n < min || n > max {
            errs.push(format!(
                "{:?} gate{} has {n} terminal(s), but takes {what} (LRM 1800-2017 §28)",
                g.gate_type,
                inst.name
                    .as_ref()
                    .map(|i| format!(" '{}'", i.name))
                    .unwrap_or_default()
            ));
        }
    }
}

/// §13.5.3 (a reference simulator restriction): a default value on an `output`/`inout`
/// subroutine port is not permitted. Flags `task t(output int j = b);`.
fn check_output_port_defaults(
    ports: &[xezim_core::ast::decl::FunctionPort],
    errs: &mut Vec<String>,
) {
    use xezim_core::ast::types::PortDirection;
    for p in ports {
        if p.default.is_some()
            && matches!(p.direction, PortDirection::Output | PortDirection::Inout)
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
/// Nets are the explicit NetDeclarations and the ANSI output ports that
/// §23.2.2.3 makes nets (`output b`, `output [3:0] c`, `output wire d`, and
/// the ports inheriting from one); `output logic q` / `output reg q` are
/// variables. Only `=`/`<=` targets are checked (force/release/assign are
/// separate statement kinds).
fn check_proc_net_assign(ports: &PortList, items: &[ModuleItem], errs: &mut Vec<String>) {
    use std::collections::HashSet;
    let mut nets: HashSet<String> = HashSet::new();
    for it in items {
        if let ModuleItem::NetDeclaration(nd) = it {
            for d in &nd.declarators {
                nets.insert(d.name.name.clone());
            }
        }
    }
    if let PortList::Ansi(ps) = ports {
        let mut prev_net = false;
        for p in ps {
            let omitted =
                p.direction.is_none() && p.net_type.is_none() && !p.var_kw && p.data_type.is_none();
            // A port that carries a variable type is a variable: the parser
            // copies a net kind into a port that inherits `output reg` from
            // its predecessor (`output reg q, r`), so the net kind alone
            // does not decide.
            let net = if omitted {
                prev_net
            } else {
                p.direction == Some(PortDirection::Output)
                    && !p.var_kw
                    && matches!(p.data_type, None | Some(DataType::Implicit { .. }))
            };
            if net {
                nets.insert(p.name.name.clone());
            }
            prev_net = net;
        }
        // A body declaration of the same name is not a net port (and is
        // diagnosed elsewhere if illegal).
        for it in items {
            if let ModuleItem::DataDeclaration(d) = it {
                for dc in &d.declarators {
                    nets.remove(&dc.name.name);
                }
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
        StatementKind::Case { items, .. } => items
            .iter()
            .for_each(|it| for_each_proc_assign_lhs(&it.stmt, f)),
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
/// Is this width expression one we can const-fold with CONFIDENCE?
///
/// `const_eval_i64_with_params` answers `Some(0)` for references it cannot
/// actually resolve — a struct-typed parameter member (`Cfg.NrEntries`)
/// routes through `eval_const_expr_val`, whose Value defaults to zero. Fed
/// straight into the §11.5.1 check that read as "width 0" and rejected
/// perfectly legal RTL: every cva6 and black-parrot configuration failed
/// elaboration on `[base +: $clog2(CVA6Cfg.<field>)]`, which cannot be
/// resolved at all without instance context (the parameter is registered
/// per-instance, e.g. `u.Cfg`). Only literals and references that genuinely
/// resolve are trusted; anything else leaves the select alone, which is the
/// right bias for a lint that produces a hard error.
fn width_is_confident(e: &Expression, elab: &ElaboratedModule) -> bool {
    match &e.kind {
        ExprKind::Number(_) => true,
        ExprKind::Paren(i) => width_is_confident(i, elab),
        ExprKind::Unary { operand, .. } => width_is_confident(operand, elab),
        ExprKind::Binary { left, right, .. } => {
            width_is_confident(left, elab) && width_is_confident(right, elab)
        }
        ExprKind::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            width_is_confident(condition, elab)
                && width_is_confident(then_expr, elab)
                && width_is_confident(else_expr, elab)
        }
        ExprKind::SystemCall { args, .. } => args.iter().all(|a| width_is_confident(a, elab)),
        ExprKind::Ident(h) => {
            let raw = h
                .path
                .iter()
                .map(|s| s.name.name.as_str())
                .collect::<Vec<_>>()
                .join(".");
            let leaf = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
            elab.parameters.contains_key(&raw)
                || elab.parameters.contains_key(leaf)
                || elab.parameters.keys().any(|k| {
                    k.rsplit('.').next() == Some(leaf) || k.rsplit("::").next() == Some(leaf)
                })
        }
        // A struct/interface member or an index: only trusted when the exact
        // dotted key was seeded (interface-port parameters do this).
        ExprKind::MemberAccess { expr: base, member } => {
            let ExprKind::Ident(h) = &base.kind else {
                return false;
            };
            let b = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
            elab.parameters
                .contains_key(&format!("{b}.{}", member.name))
                || elab
                    .parameters
                    .contains_key(&format!("{b}::{}", member.name))
        }
        _ => false,
    }
}

fn check_zero_slice(e: &Expression, elab: &ElaboratedModule, errs: &mut Vec<String>) {
    if let ExprKind::RangeSelect { kind, right, .. } = &e.kind {
        if matches!(kind, RangeKind::IndexedUp | RangeKind::IndexedDown) {
            if !width_is_confident(right, elab) {
                return;
            }
            if let Some(0) =
                xezim_core::elaborate::const_eval_i64_with_params(right, Some(&elab.parameters))
            {
                errs.push("indexed part-select has zero width (LRM 1800-2017 §11.5.1)".to_string());
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
        StatementKind::While { condition, body } | StatementKind::DoWhile { body, condition } => {
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
fn check_stream_widths(items: &[ModuleItem], elab: &ElaboratedModule, errs: &mut Vec<String>) {
    // Raw-AST width scan: suppress clamp warnings (elaboration re-resolves
    // every kept declaration and warns there; dead ones get elided instead).
    let vw =
        xezim_core::elaborate::with_width_warnings_suppressed(|| build_var_widths(items, elab));
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
    let w =
        xezim_core::elaborate::resolve_type_width(dt, Some(&elab.parameters), Some(&elab.typedefs));
    if w == 0 { None } else { Some(w) }
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

/// True when every name in this type's packed dimensions resolves in the flat
/// parameter map, so its width is genuinely known.
///
/// `resolve_type_width` SKIPS a dimension it cannot evaluate and returns the
/// width the type would have had WITHOUT it — an unresolvable base therefore
/// looks like a 1-bit one. This lint walks the raw AST, where a submodule's
/// parameter is still spelled bare (`DW`) while the map holds only the
/// instance-qualified key (`u_fifo.DW`), so `enum logic [DW-1:0] { A = 96'h0 }`
/// measured 1 bit and §6.19 REJECTED a perfectly legal design. A lint must not
/// fire on a width it could not determine.
fn dim_names_resolve(dt: &DataType, elab: &ElaboratedModule) -> bool {
    fn expr_known(e: &Expression, elab: &ElaboratedModule) -> bool {
        match &e.kind {
            ExprKind::Number(_) => true,
            ExprKind::Paren(x) => expr_known(x, elab),
            ExprKind::Unary { operand, .. } => expr_known(operand, elab),
            ExprKind::Binary { left, right, .. } => {
                expr_known(left, elab) && expr_known(right, elab)
            }
            ExprKind::Ident(h) => h
                .path
                .last()
                .is_some_and(|s| elab.parameters.contains_key(&s.name.name)),
            // Anything else is not something this lint should judge.
            _ => false,
        }
    }
    let dims = match dt {
        DataType::IntegerVector { dimensions, .. } => dimensions,
        // A non-vector base carries no dimension to be unsure about.
        _ => return true,
    };
    dims.iter().all(|d| match d {
        PackedDimension::Range { left, right, .. } => {
            expr_known(left, elab) && expr_known(right, elab)
        }
        _ => false,
    })
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
        if w != 0 && dim_names_resolve(base, elab) {
            for m in &et.members {
                if let Some(init) = &m.init {
                    if let ExprKind::Number(NumberLiteral::Integer { size: Some(s), .. }) =
                        &init.kind
                    {
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
        ExprKind::Unary {
            op: UnaryOp::Minus | UnaryOp::Plus,
            operand,
        } => operand.as_ref(),
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
            return value
                .chars()
                .any(|c| matches!(c, 'x' | 'X' | 'z' | 'Z' | '?'));
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
    // Same trap as the sized-literal check above: a base whose width names a
    // parameter this map cannot resolve measures 1 bit, and every member then
    // looks out of range (and every one of them looks equal to every other).
    if base.is_some_and(|b| !dim_names_resolve(b, elab)) {
        return;
    }
    let width = base
        .map(|b| {
            xezim_core::elaborate::resolve_type_width(
                b,
                Some(&elab.parameters),
                Some(&elab.typedefs),
            )
        })
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
    let mask: u128 = if width >= 128 {
        u128::MAX
    } else {
        (1u128 << width) - 1
    };
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
                    .is_some_and(|s| s <= width);
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
    // §8.26.4 (an interface class extending a type parameter) is a parse
    // error: the parser sees every base, the AST keeps only the first.
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
    check_class_member_names(c, errs);
    // Recurse into nested classes regardless of this class's kind.
    for item in &c.items {
        if let ClassItem::Class(nested) = item {
            check_class(nested, errs);
        }
    }
}

/// §8.3 / §6.19: a class's properties, typedefs and the constants of the
/// enums it declares share one name space (`enum {A = 10} e; typedef int A;`
/// declares `A` twice).
fn check_class_member_names(c: &ClassDeclaration, errs: &mut Vec<String>) {
    fn enum_names(dt: &DataType) -> Vec<&str> {
        match dt {
            // `A[3]` declares A0..A2, not A.
            DataType::Enum(et) => et
                .members
                .iter()
                .filter(|m| m.range.is_none())
                .map(|m| m.name.name.as_str())
                .collect(),
            _ => Vec::new(),
        }
    }
    let mut names: Vec<&str> = Vec::new();
    for item in &c.items {
        match item {
            ClassItem::Property(p) => {
                names.extend(enum_names(&p.data_type));
                names.extend(p.declarators.iter().map(|d| d.name.name.as_str()));
            }
            ClassItem::Typedef(t) if !t.forward && !matches!(t.data_type, DataType::Void(_)) => {
                names.extend(enum_names(&t.data_type));
                names.push(&t.name.name);
            }
            _ => {}
        }
    }
    let mut seen: HashSet<&str> = HashSet::new();
    for n in names {
        if !n.is_empty() && !seen.insert(n) {
            errs.push(format!(
                "class '{}': '{n}' is declared more than once (LRM 1800-2017 §8.3)",
                c.name.name
            ));
        }
    }
}

/// True if statement `s` is a bare `super.new(...)` call.
fn stmt_is_super_new(s: &Statement) -> bool {
    let StatementKind::Expr(e) = &s.kind else {
        return false;
    };
    let ExprKind::Call { func, .. } = &e.kind else {
        return false;
    };
    let ExprKind::MemberAccess { expr, member } = &func.kind else {
        return false;
    };
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

/// Collect every type NAME a module/interface/program declares itself:
/// `parameter type` / `localparam type` names (header and body) and local
/// typedef names, recursing through generate constructs. The §6.18 check
/// below runs over LIBRARY definitions that may never be elaborated, so
/// `elab.typedefs` knows nothing about their locals — without this set a
/// typedef chain rooted at a type parameter (`parameter type req_data_t =
/// logic; typedef req_data_t wbuf_data_t; typedef wbuf_data_t ...`), the
/// hpdcache/cva6 and black-parrot idiom, was reported as "base type not
/// declared" on every link of the chain.
fn collect_local_type_names(
    params: &[xezim_core::ast::decl::ParameterDeclaration],
    items: &[ModuleItem],
    out: &mut std::collections::HashSet<String>,
) {
    use xezim_core::ast::decl::ParameterKind;
    for p in params {
        if let ParameterKind::Type { assignments } = &p.kind {
            for a in assignments {
                out.insert(a.name.name.clone());
            }
        }
    }
    fn walk(items: &[ModuleItem], out: &mut std::collections::HashSet<String>) {
        use xezim_core::ast::decl::ParameterKind;
        for it in items {
            match it {
                ModuleItem::ParameterDeclaration(p) | ModuleItem::LocalparamDeclaration(p) => {
                    if let ParameterKind::Type { assignments } = &p.kind {
                        for a in assignments {
                            out.insert(a.name.name.clone());
                        }
                    }
                }
                ModuleItem::TypedefDeclaration(t) => {
                    out.insert(t.name.name.clone());
                }
                ModuleItem::GenerateRegion(g) => walk(&g.items, out),
                ModuleItem::GenerateIf(g) => {
                    for (_, branch) in &g.branches {
                        walk(branch, out);
                    }
                }
                ModuleItem::GenerateFor(g) => walk(&g.items, out),
                ModuleItem::GenerateCase(g) => {
                    for arm in &g.arms {
                        walk(&arm.items, out);
                    }
                }
                _ => {}
            }
        }
    }
    walk(items, out);
}

/// §6.18: a non-forward `typedef <T> name;` whose base type `<T>` is a bare,
/// undeclared simple identifier is an error. Conservative: only fires when the
/// base is a single-segment name (no `::`), is not a built-in keyword type, and
/// is absent from every elaborated type namespace (typedefs, classes, enums,
/// interfaces, packages, parameters).
fn check_typedef(
    t: &TypedefDeclaration,
    elab: &ElaboratedModule,
    local_types: &std::collections::HashSet<String>,
    errs: &mut Vec<String>,
) {
    if t.forward {
        return;
    }
    check_enum_type(&t.data_type, elab, errs);
    if let DataType::TypeReference { name, .. } = &t.data_type {
        // package-qualified (`pkg::T`) — out of scope for this conservative check
        if name.has_scope() {
            return;
        }
        let n = &name.name.name;
        if n.is_empty() {
            return;
        }
        if is_builtin_type(n) {
            return;
        }
        let known = local_types.contains(n.as_str())
            || elab.typedefs.contains_key(n)
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
        ExprKind::Ident(h) if h.path.len() == 1 => {
            out.push(h.path[0].name.name.clone());
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
        "bit"
            | "logic"
            | "reg"
            | "byte"
            | "shortint"
            | "int"
            | "longint"
            | "integer"
            | "time"
            | "real"
            | "shortreal"
            | "realtime"
            | "string"
            | "chandle"
            | "event"
            | "void"
            | "wire"
            | "tri"
            | "wand"
            | "wor"
            | "uwire"
            | "signed"
            | "unsigned"
            | "genvar"
            | "type"
            | "enum"
            | "struct"
            | "union"
            | "process"
            | "supply0"
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
        DataType::Real { .. }
            | DataType::Simple {
                kind: SimpleType::String,
                ..
            }
    )
}

/// One `==?`/`!=?` operand is illegal when it is a real/string literal or a
/// declared real/string variable.
fn wildcard_operand_illegal(
    e: &Expression,
    nonintegral: &std::collections::HashSet<String>,
) -> bool {
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
fn check_wildcard_cmp(items: &[ModuleItem], _elab: &ElaboratedModule, errs: &mut Vec<String>) {
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
    let scan = |e: &Expression, errs: &mut Vec<String>| {
        for_each_expr(e, &mut |x| {
            if let ExprKind::Binary { op, left, right } = &x.kind {
                if matches!(op, BinaryOp::WildcardEq | BinaryOp::WildcardNeq)
                    && (wildcard_operand_illegal(left, &nonintegral)
                        || wildcard_operand_illegal(right, &nonintegral))
                {
                    errs.push(
                        "wildcard-equality operator (==? / !=?) requires integral operands; \
                             a real or string operand is illegal (LRM 1800-2017 §11.4.6)"
                            .to_string(),
                    );
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
// §23.6 — a hierarchical name through a named statement block must name
// something that block declares.
// ---------------------------------------------------------------------------

/// `U[0].V.a` where `V` is a named `begin`/`fork` block: the name after `V`
/// is looked up in `V` itself (§23.6 downward resolution), not in the
/// scopes around it. xezim's runtime resolver falls back outward and found
/// the generate block's `a`; the reference rejects the reference ("Failed to
/// find 'a' in hierarchical name"), as does ivtest pr1988302b.
///
/// Conservative: a label is only checked when it names nothing BUT statement
/// blocks design-wide (no instance, generate block, subroutine, variable or
/// net shares it), and a block's members are over-approximated as every
/// variable and nested block label anywhere in its body.
fn check_named_block_refs(defs: &[&SourceDefinition], errs: &mut Vec<String>) {
    let mut blocks: HashMap<String, HashSet<String>> = HashMap::new();
    let mut other: HashSet<String> = HashSet::new();

    fn block_members(stmts: &[Statement], out: &mut HashSet<String>) {
        for st in stmts {
            for_each_stmt(st, &mut |s| match &s.kind {
                StatementKind::VarDecl { declarators, .. } => {
                    out.extend(declarators.iter().map(|d| d.name.name.clone()));
                }
                StatementKind::SeqBlock { name: Some(n), .. }
                | StatementKind::ParBlock { name: Some(n), .. } => {
                    out.insert(n.name.clone());
                }
                _ => {}
            });
        }
    }
    fn note_stmt(st: &Statement, blocks: &mut HashMap<String, HashSet<String>>) {
        for_each_stmt(st, &mut |s| {
            if let StatementKind::SeqBlock {
                name: Some(n),
                stmts,
            }
            | StatementKind::ParBlock {
                name: Some(n),
                stmts,
                ..
            } = &s.kind
            {
                block_members(stmts, blocks.entry(n.name.clone()).or_default());
            }
        });
    }
    fn walk_items<'a>(
        items: &'a [ModuleItem],
        stmts: &mut Vec<&'a Statement>,
        exprs: &mut Vec<&'a Expression>,
        other: &mut HashSet<String>,
    ) {
        for it in items {
            match it {
                ModuleItem::InitialConstruct(i) => stmts.push(&i.stmt),
                ModuleItem::AlwaysConstruct(a) => stmts.push(&a.stmt),
                ModuleItem::FinalConstruct(f) => stmts.push(&f.stmt),
                ModuleItem::TaskDeclaration(t) => {
                    other.insert(t.name.name.name.clone());
                    stmts.extend(t.items.iter());
                }
                ModuleItem::FunctionDeclaration(f) => {
                    other.insert(f.name.name.name.clone());
                    stmts.extend(f.items.iter());
                }
                ModuleItem::ContinuousAssign(ca) => {
                    for (l, r) in &ca.assignments {
                        exprs.push(l);
                        exprs.push(r);
                    }
                }
                ModuleItem::ModuleInstantiation(mi) => {
                    other.extend(mi.instances.iter().map(|h| h.name.name.clone()));
                }
                ModuleItem::DataDeclaration(d) => {
                    other.extend(d.declarators.iter().map(|x| x.name.name.clone()));
                }
                ModuleItem::NetDeclaration(d) => {
                    other.extend(d.declarators.iter().map(|x| x.name.name.clone()));
                }
                ModuleItem::GenerateRegion(g) => walk_items(&g.items, stmts, exprs, other),
                ModuleItem::GenerateIf(g) => {
                    other.extend(g.branch_labels.iter().flatten().cloned());
                    for (_, b) in &g.branches {
                        walk_items(b, stmts, exprs, other);
                    }
                }
                ModuleItem::GenerateFor(g) => {
                    other.extend(g.name.iter().cloned());
                    walk_items(&g.items, stmts, exprs, other);
                }
                ModuleItem::GenerateCase(g) => {
                    for arm in &g.arms {
                        other.extend(arm.label.iter().cloned());
                        walk_items(&arm.items, stmts, exprs, other);
                    }
                }
                _ => {}
            }
        }
    }
    let mut stmts: Vec<&Statement> = Vec::new();
    let mut exprs: Vec<&Expression> = Vec::new();
    for def in defs {
        let items = match def {
            SourceDefinition::Module(m) => &m.items,
            SourceDefinition::Interface(m) => &m.items,
            SourceDefinition::Program(m) => &m.items,
            _ => continue,
        };
        walk_items(items, &mut stmts, &mut exprs, &mut other);
    }
    for st in &stmts {
        note_stmt(st, &mut blocks);
    }
    blocks.retain(|label, _| !other.contains(label));
    if blocks.is_empty() {
        return;
    }
    // `U[0].V.a` parses as nested member accesses over an index; flatten the
    // scope segments (selects do not add one).
    fn segments(e: &Expression, out: &mut Vec<String>) -> bool {
        match &e.kind {
            ExprKind::Ident(h) => {
                out.extend(h.path.iter().map(|s| s.name.name.clone()));
                true
            }
            ExprKind::Index { expr, .. } => segments(expr, out),
            ExprKind::MemberAccess { expr, member } => {
                segments(expr, out) && {
                    out.push(member.name.clone());
                    true
                }
            }
            _ => false,
        }
    }
    let check = |e: &Expression, errs: &mut Vec<String>| {
        if !matches!(e.kind, ExprKind::Ident(_) | ExprKind::MemberAccess { .. }) {
            return;
        }
        let mut segs = Vec::new();
        if !segments(e, &mut segs) {
            return;
        }
        for w in segs.windows(2) {
            let Some(members) = blocks.get(&w[0]) else {
                continue;
            };
            if !members.contains(&w[1]) {
                let msg = format!(
                    "'{}' is not declared in block '{}' (hierarchical name '{}', IEEE \
                     1800-2017 §23.6)",
                    w[1],
                    w[0],
                    segs.join(".")
                );
                if !errs.contains(&msg) {
                    errs.push(msg);
                }
                return;
            }
        }
    };
    for st in &stmts {
        for_each_stmt_expr(st, &mut |e| for_each_expr(e, &mut |x| check(x, errs)));
    }
    for e in &exprs {
        for_each_expr(e, &mut |x| check(x, errs));
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
                // Same confidence gate as the §11.5.1 zero-width check: the
                // const evaluator answers 0 for references it cannot resolve
                // (struct-parameter members, per-instance type params), and
                // cva6's `logic [W-1:0] mem[$clog2(Cfg.MemWords)]`-style
                // declarations were rejected as size-0 arrays on legal RTL.
                if !width_is_confident(expr, elab) {
                    continue;
                }
                if let Some(v) = xezim_core::elaborate::const_eval_i64_with_params(expr, params) {
                    if v <= 0 {
                        errs.push(format!(
                            "array '{}' dimension size must be greater than zero (LRM 1800-2017 §7.4)",
                            name
                        ));
                    }
                }
            }
            UnpackedDimension::Queue {
                max_size: Some(bound),
                ..
            } => {
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
                    } if !scope.contains(&name.name) => {
                        errs.push(format!(
                            "implicit port connection '.{}' has no matching signal in the \
                             enclosing scope (LRM 1800-2017 §23.3.2.2)",
                            name.name
                        ));
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

/// §9.2.1: a plain `always` whose body contains NO timing control anywhere
/// (`#delay`, `@event`, `wait`, or an intra-assignment delay) can never yield —
/// executing it is an infinite zero-time loop. xezim used to classify such a
/// block as combinational with self-written vars dropped from the sensitivity
/// list, so it silently ran ONCE and never again (silent wrongness, not even a
/// hang). Commercial tools and ivtest's br991b expect a compile error.
///
/// Over-reject guards: any user TASK call is assumed potentially timed (the
/// delay may live in the task body), and `always_comb`/`always_latch`/
/// `always_ff` are governed by their own §9.2.2 rules, not this one.
fn check_always_has_timing_control(
    a: &crate::ast::decl::AlwaysConstruct,
    elab: &ElaboratedModule,
    errs: &mut Vec<String>,
) {
    use crate::ast::decl::AlwaysKind;
    if a.kind != AlwaysKind::Always {
        return;
    }
    fn stmt_may_yield(st: &crate::ast::stmt::Statement) -> bool {
        use crate::ast::expr::ExprKind;
        use crate::ast::stmt::StatementKind as SK;
        // Intra-assignment delay / event / cycle controls were canonicalized
        // to marker calls by the pre-parse rewrite (`crate::intra_delay`).
        let intra_timed = |rvalue: &crate::ast::expr::Expression| {
            matches!(&rvalue.kind, ExprKind::SystemCall { name, .. }
                if name.contains("__xz_intra_"))
        };
        match &st.kind {
            SK::TimingControl { .. } | SK::Wait { .. } | SK::WaitFork => true,
            SK::BlockingAssign { rvalue, .. } => intra_timed(rvalue),
            // An NBA with an explicit delay (`x <= #5 y`) yields time.
            SK::NonblockingAssign { delay, rvalue, .. } => delay.is_some() || intra_timed(rvalue),
            // A user task may contain the timing control — cannot prove
            // otherwise here, so treat the block as legal. A task enable
            // without parentheses (`always my_task;`) is a bare name.
            SK::Expr(e) => matches!(&e.kind, ExprKind::Call { .. } | ExprKind::Ident(_)),
            SK::SeqBlock { stmts, .. } | SK::ParBlock { stmts, .. } => {
                stmts.iter().any(stmt_may_yield)
            }
            SK::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                stmt_may_yield(then_stmt)
                    || else_stmt.as_deref().map(stmt_may_yield).unwrap_or(false)
            }
            SK::Case { items, .. } => items.iter().any(|it| stmt_may_yield(&it.stmt)),
            SK::Forever { body }
            | SK::Repeat { body, .. }
            | SK::While { body, .. }
            | SK::DoWhile { body, .. }
            | SK::For { body, .. }
            | SK::Foreach { body, .. } => stmt_may_yield(body),
            _ => false,
        }
    }
    if !stmt_may_yield(&a.stmt) {
        let at = xezim_core::elaborate::span_location(elab, a.span)
            .unwrap_or_else(|| format!("byte {}..{}", a.span.start, a.span.end));
        errs.push(format!(
            "`always` block with no timing control anywhere in its body — it can never \
yield, so simulated time cannot advance (IEEE 1800-2017 §9.2.1). Add a `#delay`, \
`@(...)`, or `wait`, or use `always_comb` (at {})",
            at
        ));
    }
}

/// package name -> the names it declares (not the ones it imports). An
/// imported one may be what a module-level reference means, so the
/// declaration checks leave those alone.
fn package_decls(defs: &[&SourceDefinition]) -> HashMap<String, HashSet<String>> {
    use xezim_core::ast::decl::{PackageItem, ParameterKind};
    let mut map = HashMap::new();
    for def in defs {
        let SourceDefinition::Package(p) = def else {
            continue;
        };
        let out: &mut HashSet<String> = map.entry(p.name.name.clone()).or_default();
        for it in &p.items {
            match it {
                PackageItem::Parameter(pd) => match &pd.kind {
                    ParameterKind::Data { assignments, .. } => {
                        out.extend(assignments.iter().map(|a| a.name.name.clone()))
                    }
                    ParameterKind::Type { assignments } => {
                        out.extend(assignments.iter().map(|a| a.name.name.clone()))
                    }
                },
                PackageItem::Data(d) => {
                    out.extend(d.declarators.iter().map(|v| v.name.name.clone()));
                    if let DataType::Enum(et) = &d.data_type {
                        out.extend(et.members.iter().map(|m| m.name.name.clone()));
                    }
                }
                PackageItem::Typedef(td) => {
                    out.insert(td.name.name.clone());
                    if let DataType::Enum(et) = &td.data_type {
                        out.extend(et.members.iter().map(|m| m.name.name.clone()));
                    }
                }
                PackageItem::Function(f) => {
                    out.insert(f.name.name.name.clone());
                }
                PackageItem::Task(t) => {
                    out.insert(t.name.name.name.clone());
                }
                PackageItem::Class(c) => {
                    out.insert(c.name.name.clone());
                }
                _ => {}
            }
        }
    }
    map
}

/// True when `n` names something the elaborated top module declares.
fn top_declares(n: &str, elab: &ElaboratedModule) -> bool {
    elab.parameters.contains_key(n)
        || elab.signals.contains_key(n)
        || elab.enum_members.contains_key(n)
        || elab.typedefs.contains_key(n)
        || elab.classes.contains_key(n)
        || elab.functions.contains_key(n)
        || elab.arrays.contains_key(n)
        || elab.arrays_2d.contains_key(n)
        || elab.arrays_nd.contains_key(n)
        || elab.associative_arrays.contains_key(n)
        || elab.dynamic_arrays.contains(n)
        || elab.queue_vars.contains(n)
        || elab.interfaces.contains(n)
        || elab.packages.contains(n)
}

/// §6.20.2: a parameter's value cannot depend on itself, directly
/// (`parameter A = A + 6;`) or through other parameters (`parameter A = B;
/// parameter B = A;`). In the top module a name declared nowhere
/// (`parameter x = y ? a : b;` without `b`) is rejected too, even in an
/// operand the value does not select. An acyclic forward reference is
/// resolved (the reference simulator rejects that too; xezim keeps it).
fn check_param_value_refs(
    params: &[xezim_core::ast::decl::ParameterDeclaration],
    items: &[ModuleItem],
    is_top: bool,
    pkg_names: &HashSet<String>,
    elab: &ElaboratedModule,
    errs: &mut Vec<String>,
) {
    use xezim_core::ast::decl::ParameterKind;
    // (name, names its value reads) of every parameter; type parameters
    // read nothing here.
    let mut deps: Vec<(&str, Vec<String>)> = Vec::new();
    let body = items.iter().filter_map(|it| match it {
        ModuleItem::ParameterDeclaration(p) | ModuleItem::LocalparamDeclaration(p) => Some(p),
        _ => None,
    });
    for pd in params.iter().chain(body) {
        match &pd.kind {
            ParameterKind::Data { assignments, .. } => {
                for a in assignments {
                    let mut ids = Vec::new();
                    if let Some(init) = &a.init {
                        collect_idents(init, &mut ids);
                    }
                    deps.push((a.name.name.as_str(), ids));
                }
            }
            ParameterKind::Type { assignments } => {
                deps.extend(
                    assignments
                        .iter()
                        .map(|a| (a.name.name.as_str(), Vec::new())),
                );
            }
        }
    }
    let index: HashMap<&str, usize> = deps.iter().enumerate().map(|(i, (n, _))| (*n, i)).collect();
    for (i, (name, ids)) in deps.iter().enumerate() {
        // Does the value of `name` lead back to `name`?
        let mut seen = vec![false; deps.len()];
        let mut stack: Vec<usize> = ids
            .iter()
            .filter(|id| !pkg_names.contains(*id))
            .filter_map(|id| index.get(id.as_str()).copied())
            .collect();
        let mut cyclic = false;
        while let Some(k) = stack.pop() {
            if k == i {
                cyclic = true;
                break;
            }
            if std::mem::replace(&mut seen[k], true) {
                continue;
            }
            stack.extend(
                deps[k]
                    .1
                    .iter()
                    .filter(|id| !pkg_names.contains(*id))
                    .filter_map(|id| index.get(id.as_str()).copied()),
            );
        }
        if cyclic {
            errs.push(format!(
                "parameter '{name}' depends on its own value (LRM 1800-2017 §6.20.2)"
            ));
        }
        if is_top {
            for id in ids {
                if !index.contains_key(id.as_str())
                    && !pkg_names.contains(id)
                    && !top_declares(id, elab)
                {
                    errs.push(format!(
                        "parameter '{name}' refers to undeclared identifier '{id}' \
                         (LRM 1800-2017 §6.20.2)"
                    ));
                }
            }
        }
    }
}

/// §13.3/§13.4: in the top module, an identifier in a function's return range
/// or in a subroutine port range must be declared (`function [w-1:0] copy;`
/// with no `w` anywhere).
fn check_subroutine_range_idents(
    items: &[ModuleItem],
    is_top: bool,
    pkg_names: &HashSet<String>,
    elab: &ElaboratedModule,
    errs: &mut Vec<String>,
) {
    if !is_top {
        return;
    }
    let check = |sub: &str, dts: Vec<&DataType>, body: &[Statement], errs: &mut Vec<String>| {
        let mut locals = HashSet::new();
        for st in body {
            if let StatementKind::VarDecl { declarators, .. } = &st.kind {
                locals.extend(declarators.iter().map(|d| d.name.name.as_str()));
            }
        }
        let mut ids = Vec::new();
        for dt in dts {
            dim_idents(dt, &mut ids);
        }
        for id in ids {
            if !locals.contains(id.as_str()) && !pkg_names.contains(&id) && !top_declares(&id, elab)
            {
                errs.push(format!(
                    "'{sub}': range refers to undeclared identifier '{id}' (LRM 1800-2017 §13.4)"
                ));
            }
        }
    };
    for it in items {
        match it {
            ModuleItem::FunctionDeclaration(f) => {
                let dts = std::iter::once(&f.return_type)
                    .chain(f.ports.iter().map(|p| &p.data_type))
                    .collect();
                check(&f.name.name.name, dts, &f.items, errs);
            }
            ModuleItem::TaskDeclaration(t) => {
                let dts = t.ports.iter().map(|p| &p.data_type).collect();
                check(&t.name.name.name, dts, &t.items, errs);
            }
            _ => {}
        }
    }
}

/// §23.2.2.1: each name in a non-ANSI port list is declared in the module
/// body with a port direction (or as an interface port). A net or variable
/// declaration alone does not make it a port.
fn check_nonansi_ports_declared(
    module: &str,
    ports: &PortList,
    items: &[ModuleItem],
    errs: &mut Vec<String>,
) {
    let PortList::NonAnsi(names) = ports else {
        return;
    };
    // An unparsable UDP falls back to a body-less module stub.
    if items.is_empty() {
        return;
    }
    let mut declared = HashSet::new();
    for it in items {
        match it {
            ModuleItem::PortDeclaration(d) => {
                declared.extend(d.declarators.iter().map(|d| d.name.name.as_str()))
            }
            // `intf_t bus;` / `intf_t.mp bus;`: a non-ANSI interface port.
            ModuleItem::DataDeclaration(d)
                if matches!(
                    d.data_type,
                    DataType::TypeReference { .. } | DataType::Interface { .. }
                ) =>
            {
                declared.extend(d.declarators.iter().map(|d| d.name.name.as_str()))
            }
            _ => {}
        }
    }
    for n in names {
        // `__xz_*` names stand in for null ports.
        if !n.name.is_empty() && !n.name.starts_with("__xz_") && !declared.contains(n.name.as_str())
        {
            errs.push(format!(
                "port '{}' of module '{module}' has no input, output or inout declaration \
                 (LRM 1800-2017 §23.2.2.1)",
                n.name
            ));
        }
    }
}

/// §10.9: an ordered assignment pattern for a struct has one item per member,
/// and for a packed array one per element of its outer dimension
/// (`bit [2:0][3:0] x = '{1, 2};` and `struct packed {int x; shortint y;
/// byte z;} s = '{1, 2};` are errors). Module-level declarations only; a
/// pattern with a replication or a non-ordered item is left alone.
fn check_pattern_counts(
    defs: &[&SourceDefinition],
    items: &[ModuleItem],
    elab: &ElaboratedModule,
    is_top: bool,
    errs: &mut Vec<String>,
) {
    let typedef = |n: &str| -> Option<DataType> {
        items
            .iter()
            .find_map(|it| match it {
                ModuleItem::TypedefDeclaration(t) if t.name.name == n => Some(t.data_type.clone()),
                _ => None,
            })
            .or_else(|| {
                defs.iter().find_map(|d| match d {
                    SourceDefinition::Typedef(t) if t.name.name == n => Some(t.data_type.clone()),
                    _ => None,
                })
            })
    };
    for it in items {
        let ModuleItem::DataDeclaration(d) = it else {
            continue;
        };
        let dt = match &d.data_type {
            DataType::TypeReference {
                name, dimensions, ..
            } if name.scopes.is_empty() && dimensions.is_empty() => match typedef(&name.name.name) {
                Some(t) => t,
                None => continue,
            },
            dt => dt.clone(),
        };
        let want: Option<i64> = match &dt {
            DataType::Struct(su)
                if matches!(su.kind, xezim_core::ast::types::StructUnionKind::Struct)
                    && su.dimensions.is_empty() =>
            {
                Some(su.members.iter().map(|m| m.declarators.len() as i64).sum())
            }
            // The outer packed dimension; its bounds are constant in the top
            // module, where parameters are known by their own names.
            DataType::IntegerVector { dimensions, .. } if is_top => match dimensions.first() {
                Some(PackedDimension::Range { left, right, .. }) => {
                    let p = Some(&elab.parameters);
                    match (
                        xezim_core::elaborate::const_eval_i64_with_params(left, p),
                        xezim_core::elaborate::const_eval_i64_with_params(right, p),
                    ) {
                        (Some(l), Some(r)) => Some((l - r).abs() + 1),
                        _ => None,
                    }
                }
                _ => None,
            },
            _ => None,
        };
        let Some(want) = want else {
            continue;
        };
        for v in &d.declarators {
            if !v.dimensions.is_empty() {
                continue;
            }
            let Some(Expression {
                kind: ExprKind::AssignmentPattern(pat),
                ..
            }) = &v.init
            else {
                continue;
            };
            let ordered = pat.iter().all(|p| {
                matches!(p, AssignmentPatternItem::Ordered(e)
                    if !matches!(e.kind, ExprKind::Replication { .. }))
            });
            if ordered && pat.len() as i64 != want {
                errs.push(format!(
                    "assignment pattern for '{}' has {} element(s) but its type has {want} \
                     (LRM 1800-2017 §10.9)",
                    v.name.name,
                    pat.len()
                ));
            }
        }
    }
}

/// §12.7.3: a foreach loop names at most one loop variable per dimension of
/// the array: its unpacked dimensions, then its packed ones (an integer atom
/// such as `int` counts as one). `logic a[10]; foreach (a[i, j])` is an error.
/// Arrays declared at module level only, and not where a local may shadow them.
fn check_foreach_dims(items: &[ModuleItem], errs: &mut Vec<String>) {
    let mut dims: HashMap<&str, usize> = HashMap::new();
    for it in items {
        let ModuleItem::DataDeclaration(d) = it else {
            continue;
        };
        let packed = match &d.data_type {
            DataType::IntegerVector { dimensions, .. } => dimensions.len(),
            DataType::IntegerAtom { .. } => 1,
            _ => continue,
        };
        for v in &d.declarators {
            dims.insert(v.name.name.as_str(), v.dimensions.len() + packed);
        }
    }
    let mut stmts: Vec<&Statement> = Vec::new();
    let mut locals: HashSet<String> = HashSet::new();
    for it in items {
        match it {
            ModuleItem::InitialConstruct(i) => stmts.push(&i.stmt),
            ModuleItem::AlwaysConstruct(a) => stmts.push(&a.stmt),
            ModuleItem::FunctionDeclaration(f) => {
                stmts.extend(f.items.iter());
                locals.extend(f.ports.iter().map(|p| p.name.name.clone()));
            }
            ModuleItem::TaskDeclaration(t) => {
                stmts.extend(t.items.iter());
                locals.extend(t.ports.iter().map(|p| p.name.name.clone()));
            }
            _ => {}
        }
    }
    let mut loops: Vec<(String, usize)> = Vec::new();
    for st in &stmts {
        for_each_stmt(st, &mut |s| match &s.kind {
            StatementKind::VarDecl { declarators, .. } => {
                locals.extend(declarators.iter().map(|d| d.name.name.clone()))
            }
            StatementKind::Foreach { array, vars, .. } => {
                if let ExprKind::Ident(h) = &array.kind
                    && h.path.len() == 1
                    && h.path[0].selects.is_empty()
                {
                    loops.push((h.path[0].name.name.clone(), vars.len()));
                }
            }
            _ => {}
        });
    }
    for (name, n) in loops {
        if let Some(&have) = dims.get(name.as_str())
            && n > have
            && !locals.contains(&name)
        {
            errs.push(format!(
                "foreach over '{name}' names {n} loop variables, but it has {have} \
                 dimension(s) (LRM 1800-2017 §12.7.3)"
            ));
        }
    }
}

/// §7.4.6 / §11.5.1: a variable or net takes one index per unpacked and
/// packed dimension (an integer atom such as `int` has one), and a part-select
/// only as the last select. More selects than that — `reg [15:0] m[3:0];
/// m[0][0][3:0]`, a bit-select of a scalar — are errors. Module-level
/// declarations only, and not where another declaration may shadow the name.
fn check_select_depth(items: &[ModuleItem], errs: &mut Vec<String>) {
    // name -> number of selectable dimensions
    let mut dims: HashMap<&str, usize> = HashMap::new();
    let mut shadowed: HashSet<&str> = HashSet::new();
    let packed = |dt: &DataType| match dt {
        DataType::IntegerVector { dimensions, .. } | DataType::Implicit { dimensions, .. } => {
            Some(dimensions.len())
        }
        DataType::IntegerAtom { .. } => Some(1),
        _ => None,
    };
    for it in items {
        match it {
            ModuleItem::DataDeclaration(d) => {
                for v in &d.declarators {
                    match packed(&d.data_type) {
                        Some(p) if !dims.contains_key(v.name.name.as_str()) => {
                            dims.insert(v.name.name.as_str(), p + v.dimensions.len());
                        }
                        _ => {
                            shadowed.insert(v.name.name.as_str());
                        }
                    }
                }
            }
            ModuleItem::NetDeclaration(d) => {
                for v in &d.declarators {
                    match packed(&d.data_type) {
                        Some(p) if !dims.contains_key(v.name.name.as_str()) => {
                            dims.insert(v.name.name.as_str(), p + v.dimensions.len());
                        }
                        _ => {
                            shadowed.insert(v.name.name.as_str());
                        }
                    }
                }
            }
            // Ports split their range over several declarations; generate
            // blocks and subroutines may redeclare a name.
            ModuleItem::PortDeclaration(d) => {
                shadowed.extend(d.declarators.iter().map(|v| v.name.name.as_str()))
            }
            ModuleItem::FunctionDeclaration(f) => {
                shadowed.extend(f.ports.iter().map(|p| p.name.name.as_str()))
            }
            ModuleItem::TaskDeclaration(t) => {
                shadowed.extend(t.ports.iter().map(|p| p.name.name.as_str()))
            }
            ModuleItem::GenerateRegion(_)
            | ModuleItem::GenerateIf(_)
            | ModuleItem::GenerateFor(_)
            | ModuleItem::GenerateCase(_) => return,
            _ => {}
        }
    }
    let mut exprs: Vec<&Expression> = Vec::new();
    let mut stmts: Vec<&Statement> = Vec::new();
    for it in items {
        match it {
            ModuleItem::ContinuousAssign(ca) => {
                for (l, r) in &ca.assignments {
                    exprs.push(l);
                    exprs.push(r);
                }
            }
            ModuleItem::NetDeclaration(d) => {
                exprs.extend(d.declarators.iter().filter_map(|v| v.init.as_ref()))
            }
            ModuleItem::DataDeclaration(d) => {
                exprs.extend(d.declarators.iter().filter_map(|v| v.init.as_ref()))
            }
            ModuleItem::InitialConstruct(i) => stmts.push(&i.stmt),
            ModuleItem::AlwaysConstruct(a) => stmts.push(&a.stmt),
            ModuleItem::FunctionDeclaration(f) => stmts.extend(f.items.iter()),
            ModuleItem::TaskDeclaration(t) => stmts.extend(t.items.iter()),
            _ => {}
        }
    }
    for st in &stmts {
        for_each_stmt(st, &mut |s| {
            if let StatementKind::VarDecl { declarators, .. } = &s.kind {
                for d in declarators {
                    dims.remove(d.name.name.as_str());
                }
            }
        });
    }
    for s in shadowed {
        dims.remove(s);
    }
    let mut report = |e: &Expression| {
        let mut n = 0usize;
        let mut cur = e;
        loop {
            match &cur.kind {
                ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => {
                    n += 1;
                    cur = expr;
                }
                _ => break,
            }
        }
        let ExprKind::Ident(h) = &cur.kind else {
            return;
        };
        if h.path.len() != 1 {
            return;
        }
        let name = h.path[0].name.name.as_str();
        n += h.path[0].selects.len();
        if let Some(&have) = dims.get(name)
            && n > have
        {
            errs.push(format!(
                "'{name}' is selected {n} time(s), but it has {have} dimension(s) \
                 (LRM 1800-2017 §7.4.6, §11.5.1)"
            ));
        }
    };
    for e in exprs {
        for_each_expr(e, &mut report);
    }
    for st in stmts {
        for_each_stmt_expr(st, &mut report);
    }
}

/// Port directions of a module, by position and by name. `None` where the
/// direction is not written (an interface port, an ANSI first port without
/// one).
fn port_directions(
    ports: &PortList,
    items: &[ModuleItem],
) -> (Vec<Option<PortDirection>>, HashMap<String, PortDirection>) {
    let mut by_name = HashMap::new();
    match ports {
        PortList::Ansi(ps) => {
            for p in ps {
                if let Some(d) = p.direction
                    && !matches!(p.data_type, Some(DataType::Interface { .. }))
                {
                    by_name.insert(p.name.name.clone(), d);
                }
            }
        }
        PortList::NonAnsi(_) => {
            for it in items {
                if let ModuleItem::PortDeclaration(d) = it {
                    for v in &d.declarators {
                        by_name.insert(v.name.name.clone(), d.direction);
                    }
                }
            }
        }
        PortList::Empty => {}
    }
    let names: Vec<&str> = match ports {
        PortList::Ansi(ps) => ps.iter().map(|p| p.name.name.as_str()).collect(),
        PortList::NonAnsi(ns) => ns.iter().map(|n| n.name.as_str()).collect(),
        PortList::Empty => Vec::new(),
    };
    let by_pos = names.iter().map(|n| by_name.get(*n).copied()).collect();
    (by_pos, by_name)
}

/// §23.3.3: an output or inout port is connected to something that can be
/// driven — a net or variable, a select of one or a concatenation of those,
/// not a literal or an operator expression — and an inout port to a net,
/// never a variable.
fn check_port_actuals(defs: &[&SourceDefinition], items: &[ModuleItem], errs: &mut Vec<String>) {
    use xezim_core::ast::decl::PortConnection;
    fn assignable(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Ident(_) | ExprKind::MemberAccess { .. } => true,
            ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => assignable(expr),
            ExprKind::Concatenation(xs) => xs.iter().all(assignable),
            ExprKind::Paren(x) => assignable(x),
            _ => false,
        }
    }
    fn root(e: &Expression) -> Option<&str> {
        match &e.kind {
            ExprKind::Ident(h) if h.path.len() == 1 => Some(h.path[0].name.name.as_str()),
            ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => root(expr),
            _ => None,
        }
    }
    let mut ports: HashSet<&str> = HashSet::new();
    let mut vars: HashSet<&str> = HashSet::new();
    for it in items {
        match it {
            ModuleItem::PortDeclaration(d) => {
                ports.extend(d.declarators.iter().map(|v| v.name.name.as_str()))
            }
            ModuleItem::DataDeclaration(d)
                if !matches!(
                    d.data_type,
                    DataType::TypeReference { .. } | DataType::Interface { .. }
                ) =>
            {
                vars.extend(d.declarators.iter().map(|v| v.name.name.as_str()))
            }
            _ => {}
        }
    }
    for it in items {
        let ModuleItem::ModuleInstantiation(mi) = it else {
            continue;
        };
        let Some((tp, titems)) = defs.iter().find_map(|d| match d {
            SourceDefinition::Module(m) if m.name.name == mi.module_name.name => {
                Some((&m.ports, &m.items))
            }
            _ => None,
        }) else {
            continue;
        };
        let (by_pos, by_name) = port_directions(tp, titems);
        for inst in &mi.instances {
            for (i, c) in inst.connections.iter().enumerate() {
                let (dir, actual, port) = match c {
                    PortConnection::Ordered(Some(e)) => {
                        (by_pos.get(i).copied().flatten(), e, format!("#{}", i + 1))
                    }
                    PortConnection::Named {
                        name,
                        expr: Some(e),
                        ..
                    } => (by_name.get(&name.name).copied(), e, name.name.clone()),
                    _ => continue,
                };
                let bad = match dir {
                    Some(PortDirection::Output) => !assignable(actual),
                    Some(PortDirection::Inout) => {
                        !assignable(actual)
                            || root(actual).is_some_and(|r| vars.contains(r) && !ports.contains(r))
                    }
                    _ => false,
                };
                if bad {
                    errs.push(format!(
                        "port {port} of instance '{}' ({}) is an {} and cannot be connected \
                         to this expression (LRM 1800-2017 §23.3.3)",
                        inst.name.name,
                        mi.module_name.name,
                        if dir == Some(PortDirection::Output) {
                            "output"
                        } else {
                            "inout, which needs a net"
                        }
                    ));
                }
            }
        }
    }
}

/// §13.3/§13.4: a subroutine port's type name must be declared
/// (`task t1; input make_me_crash i;` names no type at all).
fn check_subroutine_port_types(
    items: &[ModuleItem],
    local_types: &HashSet<String>,
    pkg_names: &HashSet<String>,
    elab: &ElaboratedModule,
    errs: &mut Vec<String>,
) {
    for it in items {
        let (sub, ports) = match it {
            ModuleItem::FunctionDeclaration(f) => (&f.name.name.name, &f.ports),
            ModuleItem::TaskDeclaration(t) => (&t.name.name.name, &t.ports),
            _ => continue,
        };
        for p in ports {
            let DataType::TypeReference { name, .. } = &p.data_type else {
                continue;
            };
            let n = &name.name.name;
            if name.has_scope() || n.is_empty() || is_builtin_type(n) {
                continue;
            }
            let known = local_types.contains(n)
                || pkg_names.contains(n)
                || elab.typedefs.contains_key(n)
                || elab.classes.contains_key(n)
                || elab.interfaces.contains(n)
                || elab.packages.contains(n)
                || elab.parameters.contains_key(n);
            if !known {
                errs.push(format!(
                    "port '{}' of '{sub}' has undeclared type '{n}' (LRM 1800-2017 §13.3)",
                    p.name.name
                ));
            }
        }
    }
}

/// §27.6: a generate block's name lives in the enclosing module's name space.
/// It cannot repeat another declaration there (`reg named; ... begin : named`),
/// the name of another generate construct's block, or a named procedural block
/// (`initial begin : block1`). Alternative branches of one if/case construct
/// may share a name.
fn check_generate_block_names(items: &[ModuleItem], errs: &mut Vec<String>) {
    let mut decls: HashSet<&str> = HashSet::new();
    let mut blocks: Vec<HashSet<&str>> = Vec::new();
    fn walk<'a>(
        items: &'a [ModuleItem],
        decls: &mut HashSet<&'a str>,
        blocks: &mut Vec<HashSet<&'a str>>,
    ) {
        for it in items {
            match it {
                ModuleItem::DataDeclaration(d) => {
                    decls.extend(d.declarators.iter().map(|v| v.name.name.as_str()))
                }
                ModuleItem::NetDeclaration(d) => {
                    decls.extend(d.declarators.iter().map(|v| v.name.name.as_str()))
                }
                ModuleItem::ModuleInstantiation(mi) => {
                    decls.extend(mi.instances.iter().map(|i| i.name.name.as_str()))
                }
                ModuleItem::InitialConstruct(i) => {
                    if let Some(n) = stmt_block_name(&i.stmt) {
                        decls.insert(n);
                    }
                }
                ModuleItem::AlwaysConstruct(a) => {
                    if let Some(n) = stmt_block_name(&a.stmt) {
                        decls.insert(n);
                    }
                }
                ModuleItem::GenerateFor(gf) => {
                    if let Some(n) = &gf.name {
                        blocks.push(HashSet::from([n.as_str()]));
                    }
                }
                ModuleItem::GenerateIf(gi) => blocks.push(
                    gi.branch_labels
                        .iter()
                        .flatten()
                        .map(|s| s.as_str())
                        .collect(),
                ),
                ModuleItem::GenerateCase(gc) => {
                    blocks.push(gc.arms.iter().filter_map(|a| a.label.as_deref()).collect())
                }
                ModuleItem::GenerateRegion(gr) => walk(&gr.items, decls, blocks),
                _ => {}
            }
        }
    }
    fn stmt_block_name(s: &Statement) -> Option<&str> {
        match &s.kind {
            StatementKind::SeqBlock { name: Some(n), .. }
            | StatementKind::ParBlock { name: Some(n), .. } => Some(n.name.as_str()),
            StatementKind::TimingControl { stmt, .. } => stmt_block_name(stmt),
            _ => None,
        }
    }
    walk(items, &mut decls, &mut blocks);
    let mut seen: HashSet<&str> = HashSet::new();
    for set in &blocks {
        for n in set {
            if decls.contains(n) || !seen.insert(n) {
                errs.push(format!(
                    "generate block name '{n}' is already declared in this module \
                     (LRM 1800-2017 §27.6)"
                ));
            }
        }
    }
}

/// §13.4 / §20: a system task has no value, so it cannot stand in an
/// expression (`val = $display;`).
fn check_system_task_values(items: &[ModuleItem], errs: &mut Vec<String>) {
    fn is_task(n: &str) -> bool {
        const TASKS: &[&str] = &[
            "$display",
            "$displayb",
            "$displayh",
            "$displayo",
            "$write",
            "$writeb",
            "$writeh",
            "$writeo",
            "$strobe",
            "$strobeb",
            "$strobeh",
            "$strobeo",
            "$monitor",
            "$monitorb",
            "$monitorh",
            "$monitoro",
            "$monitoron",
            "$monitoroff",
            "$finish",
            "$stop",
            "$fdisplay",
            "$fwrite",
            "$fstrobe",
            "$fmonitor",
            "$dumpfile",
            "$dumpvars",
            "$dumpon",
            "$dumpoff",
            "$dumpall",
            "$dumpflush",
            "$readmemb",
            "$readmemh",
            "$writememb",
            "$writememh",
            "$printtimescale",
            "$timeformat",
        ];
        TASKS.contains(&n)
    }
    let mut report = |e: &Expression| {
        if let ExprKind::SystemCall { name, .. } = &e.kind
            && is_task(name)
        {
            errs.push(format!(
                "system task '{name}' used as a function: it returns no value (LRM 1800-2017 §20)"
            ));
        }
    };
    let mut check_stmt = |st: &Statement| {
        for_each_stmt(st, &mut |s| match &s.kind {
            StatementKind::BlockingAssign { lvalue, rvalue }
            | StatementKind::NonblockingAssign { lvalue, rvalue, .. } => {
                for_each_expr(lvalue, &mut report);
                for_each_expr(rvalue, &mut report);
            }
            StatementKind::If { condition, .. } | StatementKind::While { condition, .. } => {
                for_each_expr(condition, &mut report)
            }
            _ => {}
        });
    };
    for it in items {
        match it {
            ModuleItem::InitialConstruct(i) => check_stmt(&i.stmt),
            ModuleItem::AlwaysConstruct(a) => check_stmt(&a.stmt),
            ModuleItem::FunctionDeclaration(f) => f.items.iter().for_each(&mut check_stmt),
            ModuleItem::TaskDeclaration(t) => t.items.iter().for_each(&mut check_stmt),
            _ => {}
        }
    }
}

/// §23.10: in the top module, a parameter value in an instantiation is a
/// constant expression over declared names (`foo #(ASDF) bar();` with no
/// `ASDF` anywhere).
fn check_param_override_idents(
    items: &[ModuleItem],
    is_top: bool,
    pkg_names: &HashSet<String>,
    elab: &ElaboratedModule,
    errs: &mut Vec<String>,
) {
    if !is_top {
        return;
    }
    use xezim_core::ast::decl::{ParamConnection, ParamValue};
    for it in items {
        let ModuleItem::ModuleInstantiation(mi) = it else {
            continue;
        };
        for c in mi.params.iter().flatten() {
            let (ParamConnection::Ordered(Some(ParamValue::Expr(e)))
            | ParamConnection::Named {
                value: Some(ParamValue::Expr(e)),
                ..
            }) = c
            else {
                continue;
            };
            let mut ids = Vec::new();
            collect_idents(e, &mut ids);
            for id in ids {
                if !is_builtin_type(&id) && !pkg_names.contains(&id) && !top_declares(&id, elab) {
                    errs.push(format!(
                        "parameter value for instance of '{}' refers to undeclared identifier \
                         '{id}' (LRM 1800-2017 §23.10)",
                        mi.module_name.name
                    ));
                }
            }
        }
    }
}

/// Packages a module imports: (wildcard imports, explicitly imported names).
fn module_imports(items: &[ModuleItem]) -> (Vec<&str>, HashSet<&str>) {
    let mut wild = Vec::new();
    let mut explicit = HashSet::new();
    for it in items {
        if let ModuleItem::ImportDeclaration(d) = it {
            for i in &d.items {
                match &i.item {
                    Some(n) if n.name != "*" => {
                        explicit.insert(n.name.as_str());
                    }
                    _ => wild.push(i.package.name.as_str()),
                }
            }
        }
    }
    (wild, explicit)
}

/// Names a module declares itself, at module level.
fn module_own_names<'a>(
    ports: &'a PortList,
    params: &'a [xezim_core::ast::decl::ParameterDeclaration],
    items: &'a [ModuleItem],
) -> HashSet<&'a str> {
    use xezim_core::ast::decl::ParameterKind;
    let mut out: HashSet<&str> = HashSet::new();
    match ports {
        PortList::Ansi(ps) => out.extend(ps.iter().map(|p| p.name.name.as_str())),
        PortList::NonAnsi(ns) => out.extend(ns.iter().map(|n| n.name.as_str())),
        PortList::Empty => {}
    }
    let mut add_params = |pd: &'a xezim_core::ast::decl::ParameterDeclaration,
                          out: &mut HashSet<&'a str>| {
        match &pd.kind {
            ParameterKind::Data { assignments, .. } => {
                out.extend(assignments.iter().map(|a| a.name.name.as_str()))
            }
            ParameterKind::Type { assignments } => {
                out.extend(assignments.iter().map(|a| a.name.name.as_str()))
            }
        }
    };
    for pd in params {
        add_params(pd, &mut out);
    }
    for it in items {
        match it {
            ModuleItem::DataDeclaration(d) => {
                out.extend(d.declarators.iter().map(|v| v.name.name.as_str()))
            }
            ModuleItem::NetDeclaration(d) => {
                out.extend(d.declarators.iter().map(|v| v.name.name.as_str()))
            }
            ModuleItem::PortDeclaration(d) => {
                out.extend(d.declarators.iter().map(|v| v.name.name.as_str()))
            }
            ModuleItem::ParameterDeclaration(pd) | ModuleItem::LocalparamDeclaration(pd) => {
                add_params(pd, &mut out)
            }
            ModuleItem::TypedefDeclaration(t) => {
                out.insert(t.name.name.as_str());
            }
            ModuleItem::FunctionDeclaration(f) => {
                out.insert(f.name.name.name.as_str());
            }
            ModuleItem::TaskDeclaration(t) => {
                out.insert(t.name.name.name.as_str());
            }
            ModuleItem::ModuleInstantiation(mi) => {
                out.extend(mi.instances.iter().map(|i| i.name.name.as_str()))
            }
            ModuleItem::ClassDeclaration(c) => {
                out.insert(c.name.name.as_str());
            }
            _ => {}
        }
    }
    out
}

/// Every expression in a module's processes, continuous assignments and
/// declaration initializers.
fn module_exprs<'a>(items: &'a [ModuleItem], f: &mut dyn FnMut(&'a Expression)) {
    for it in items {
        match it {
            ModuleItem::ContinuousAssign(ca) => {
                for (l, r) in &ca.assignments {
                    f(l);
                    f(r);
                }
            }
            ModuleItem::InitialConstruct(i) => collect_stmt_exprs(&i.stmt, f),
            ModuleItem::AlwaysConstruct(a) => collect_stmt_exprs(&a.stmt, f),
            _ => {}
        }
    }
}

fn collect_stmt_exprs<'a>(st: &'a Statement, f: &mut dyn FnMut(&'a Expression)) {
    match &st.kind {
        StatementKind::Expr(e) => f(e),
        StatementKind::BlockingAssign { lvalue, rvalue }
        | StatementKind::NonblockingAssign { lvalue, rvalue, .. } => {
            f(lvalue);
            f(rvalue);
        }
        StatementKind::If {
            condition,
            then_stmt,
            else_stmt,
            ..
        } => {
            f(condition);
            collect_stmt_exprs(then_stmt, f);
            if let Some(e) = else_stmt {
                collect_stmt_exprs(e, f);
            }
        }
        StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
            stmts.iter().for_each(|s| collect_stmt_exprs(s, f))
        }
        StatementKind::TimingControl { stmt, .. } => collect_stmt_exprs(stmt, f),
        StatementKind::While { condition, body } | StatementKind::DoWhile { body, condition } => {
            f(condition);
            collect_stmt_exprs(body, f);
        }
        StatementKind::For { body, .. }
        | StatementKind::Foreach { body, .. }
        | StatementKind::Repeat { body, .. }
        | StatementKind::Forever { body } => collect_stmt_exprs(body, f),
        StatementKind::Case { expr, items, .. } => {
            f(expr);
            items.iter().for_each(|it| collect_stmt_exprs(&it.stmt, f));
        }
        _ => {}
    }
}

/// §26.3: a name that two wildcard-imported packages both declare is not
/// visible through either import; referencing it without a local declaration
/// or an explicit import is an error.
fn check_wildcard_import_conflicts(
    ports: &PortList,
    params: &[xezim_core::ast::decl::ParameterDeclaration],
    items: &[ModuleItem],
    pkgs: &HashMap<String, HashSet<String>>,
    errs: &mut Vec<String>,
) {
    let (wild, explicit) = module_imports(items);
    if wild.len() < 2 {
        return;
    }
    let own = module_own_names(ports, params, items);
    let mut count: HashMap<&str, usize> = HashMap::new();
    let mut uniq: Vec<&str> = wild.clone();
    uniq.sort();
    uniq.dedup();
    for p in uniq {
        for n in pkgs.get(p).into_iter().flatten() {
            *count.entry(n.as_str()).or_default() += 1;
        }
    }
    let ambiguous =
        |n: &str| count.get(n).is_some_and(|&c| c > 1) && !own.contains(n) && !explicit.contains(n);
    let mut hits: Vec<String> = Vec::new();
    for it in items {
        if let ModuleItem::DataDeclaration(d) = it
            && let DataType::TypeReference { name, .. } = &d.data_type
            && name.scopes.is_empty()
            && ambiguous(&name.name.name)
        {
            hits.push(name.name.name.clone());
        }
    }
    let mut locals: HashSet<String> = HashSet::new();
    for it in items {
        let st = match it {
            ModuleItem::InitialConstruct(i) => &i.stmt,
            ModuleItem::AlwaysConstruct(a) => &a.stmt,
            _ => continue,
        };
        for_each_stmt(st, &mut |s| {
            if let StatementKind::VarDecl { declarators, .. } = &s.kind {
                locals.extend(declarators.iter().map(|d| d.name.name.clone()));
            }
        });
    }
    module_exprs(items, &mut |e| {
        for_each_expr(e, &mut |x| {
            if let ExprKind::Ident(h) = &x.kind
                && h.path.len() == 1
                && ambiguous(&h.path[0].name.name)
                && !locals.contains(&h.path[0].name.name)
            {
                hits.push(h.path[0].name.name.clone());
            }
        })
    });
    hits.sort();
    hits.dedup();
    for n in hits {
        errs.push(format!(
            "'{n}' is declared by more than one wildcard-imported package, so it is not \
             visible here (LRM 1800-2017 §26.3)"
        ));
    }
}

/// §26.3: a name a module only imports is not one of its members, so a
/// hierarchical reference cannot reach it through an instance (`m.x` where
/// module M does `import P::x;`).
fn check_imported_hier_refs(
    defs: &[&SourceDefinition],
    items: &[ModuleItem],
    pkgs: &HashMap<String, HashSet<String>>,
    errs: &mut Vec<String>,
) {
    let mut inst_of: HashMap<&str, &str> = HashMap::new();
    for it in items {
        if let ModuleItem::ModuleInstantiation(mi) = it {
            for i in &mi.instances {
                inst_of.insert(i.name.name.as_str(), mi.module_name.name.as_str());
            }
        }
    }
    if inst_of.is_empty() {
        return;
    }
    let mut hits: Vec<(String, String)> = Vec::new();
    module_exprs(items, &mut |e| {
        for_each_expr(e, &mut |x| {
            let (inst, member) = match &x.kind {
                ExprKind::MemberAccess { expr, member } => match &expr.kind {
                    ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                        (h.path[0].name.name.as_str(), member.name.as_str())
                    }
                    _ => return,
                },
                ExprKind::Ident(h) if h.path.len() == 2 && h.path[0].selects.is_empty() => {
                    (h.path[0].name.name.as_str(), h.path[1].name.name.as_str())
                }
                _ => return,
            };
            let Some(&module) = inst_of.get(inst) else {
                return;
            };
            let Some(m) = defs.iter().find_map(|d| match d {
                SourceDefinition::Module(m) if m.name.name == module => Some(m),
                _ => None,
            }) else {
                return;
            };
            let own = module_own_names(&m.ports, &m.params, &m.items);
            if own.contains(member)
                || m.items.iter().any(|it| {
                    matches!(
                        it,
                        ModuleItem::GenerateRegion(_)
                            | ModuleItem::GenerateIf(_)
                            | ModuleItem::GenerateFor(_)
                            | ModuleItem::GenerateCase(_)
                    )
                })
            {
                return;
            }
            let (wild, explicit) = module_imports(&m.items);
            let imported = explicit.contains(member)
                || wild
                    .iter()
                    .any(|p| pkgs.get(*p).is_some_and(|ns| ns.contains(member)));
            if imported {
                hits.push((inst.to_string(), member.to_string()));
            }
        })
    });
    for (inst, member) in hits {
        errs.push(format!(
            "'{inst}.{member}': '{member}' is only imported into the instantiated module, \
             not declared there, so it is not visible hierarchically (LRM 1800-2017 §26.3)"
        ));
    }
}

/// Every class visible to a module: its own, the top-level ones and those in
/// packages.
fn visible_classes<'a>(
    defs: &'a [&'a SourceDefinition],
    items: &'a [ModuleItem],
) -> HashMap<&'a str, &'a ClassDeclaration> {
    let mut map = HashMap::new();
    for d in defs {
        match d {
            SourceDefinition::Class(c) => {
                map.insert(c.name.name.as_str(), &**c);
            }
            SourceDefinition::Package(p) => {
                for it in &p.items {
                    if let xezim_core::ast::decl::PackageItem::Class(c) = it {
                        map.entry(c.name.name.as_str()).or_insert(c);
                    }
                }
            }
            _ => {}
        }
    }
    for it in items {
        if let ModuleItem::ClassDeclaration(c) = it {
            map.insert(c.name.name.as_str(), c);
        }
    }
    map
}

/// §8.18: a `local` member is visible only inside its own class, not in a
/// class derived from it (`$display(a_loc)` in a subclass method).
fn check_inherited_local_access(
    classes: &HashMap<&str, &ClassDeclaration>,
    items: &[ModuleItem],
    errs: &mut Vec<String>,
) {
    fn members(c: &ClassDeclaration) -> Vec<(&str, bool)> {
        let mut out = Vec::new();
        for it in &c.items {
            match it {
                ClassItem::Property(p) => {
                    let local = p.qualifiers.contains(&ClassQualifier::Local);
                    out.extend(p.declarators.iter().map(|d| (d.name.name.as_str(), local)));
                }
                ClassItem::Method(m) => {
                    let n = match &m.kind {
                        ClassMethodKind::Function(f)
                        | ClassMethodKind::PureVirtual(f)
                        | ClassMethodKind::Extern(f) => &f.name.name.name,
                        ClassMethodKind::Task(t) => &t.name.name.name,
                    };
                    out.push((n.as_str(), m.qualifiers.contains(&ClassQualifier::Local)));
                }
                ClassItem::Typedef(t) => out.push((t.name.name.as_str(), false)),
                ClassItem::Parameter(pd) => {
                    if let xezim_core::ast::decl::ParameterKind::Data { assignments, .. } = &pd.kind
                    {
                        out.extend(assignments.iter().map(|a| (a.name.name.as_str(), false)));
                    }
                }
                _ => {}
            }
        }
        out
    }
    for it in items {
        let ModuleItem::ClassDeclaration(c) = it else {
            continue;
        };
        // The first declaration of each name up the chain, and whether it
        // is an ancestor's local member.
        let mut hidden: HashSet<&str> = HashSet::new();
        let mut seen: HashSet<&str> = members(c).into_iter().map(|(n, _)| n).collect();
        let mut base = c.extends.as_ref().map(|e| e.name.name.as_str());
        let mut depth = 0;
        while let Some(b) = base
            && depth < 32
        {
            let Some(bc) = classes.get(b) else {
                // An unknown ancestor may declare anything.
                hidden.clear();
                break;
            };
            for (n, local) in members(bc) {
                if seen.insert(n) && local {
                    hidden.insert(n);
                }
            }
            base = bc.extends.as_ref().map(|e| e.name.name.as_str());
            depth += 1;
        }
        if hidden.is_empty() {
            continue;
        }
        for ci in &c.items {
            let ClassItem::Method(m) = ci else { continue };
            let (ports, body) = match &m.kind {
                ClassMethodKind::Function(f) => (&f.ports, &f.items),
                ClassMethodKind::Task(t) => (&t.ports, &t.items),
                _ => continue,
            };
            let mut locals: HashSet<String> = ports.iter().map(|p| p.name.name.clone()).collect();
            for st in body {
                for_each_stmt(st, &mut |s| {
                    if let StatementKind::VarDecl { declarators, .. } = &s.kind {
                        locals.extend(declarators.iter().map(|d| d.name.name.clone()));
                    }
                });
            }
            let mut hits: Vec<String> = Vec::new();
            for st in body {
                for_each_stmt_expr(st, &mut |e| {
                    let n = match &e.kind {
                        ExprKind::Ident(h) if h.path.len() == 1 => h.path[0].name.name.as_str(),
                        ExprKind::MemberAccess { expr, member }
                            if matches!(expr.kind, ExprKind::This) =>
                        {
                            member.name.as_str()
                        }
                        _ => return,
                    };
                    if hidden.contains(n) && !locals.contains(n) {
                        hits.push(n.to_string());
                    }
                });
            }
            hits.sort();
            hits.dedup();
            for n in hits {
                errs.push(format!(
                    "class '{}': '{n}' is a local member of a base class and is not visible \
                     in a derived class (LRM 1800-2017 §8.18)",
                    c.name.name
                ));
            }
        }
    }
}

/// §8.25.1: outside its own declaration, a parameterized class is named with
/// a parameter value list before `::` (`par_cls#()::b`), never bare.
fn check_unspecialized_class_scope(
    classes: &HashMap<&str, &ClassDeclaration>,
    items: &[ModuleItem],
    errs: &mut Vec<String>,
) {
    // Names the module declares as something other than a class.
    let mut own = module_own_names(&PortList::Empty, &[], items);
    for it in items {
        if let ModuleItem::ClassDeclaration(c) = it {
            own.remove(c.name.name.as_str());
        }
    }
    let mut hits: Vec<String> = Vec::new();
    module_exprs(items, &mut |e| {
        for_each_expr(e, &mut |x| {
            if let ExprKind::MemberAccess { expr, .. } = &x.kind
                && let ExprKind::Ident(h) = &expr.kind
                && h.path.len() == 1
                && h.path[0].selects.is_empty()
                && let n = h.path[0].name.name.as_str()
                && classes.get(n).is_some_and(|c| !c.params.is_empty())
                && !own.contains(n)
            {
                hits.push(n.to_string());
            }
        })
    });
    hits.sort();
    hits.dedup();
    for n in hits {
        errs.push(format!(
            "parameterized class '{n}' needs a parameter value list (`{n}#(...)::`) before \
             `::` (LRM 1800-2017 §8.25.1)"
        ));
    }
}

/// §29.8: a UDP instance takes at most two delays (rise and fall).
fn check_udp_instance_delays(
    defs: &[&SourceDefinition],
    items: &[ModuleItem],
    errs: &mut Vec<String>,
) {
    for it in items {
        let ModuleItem::ModuleInstantiation(mi) = it else {
            continue;
        };
        let is_udp = defs
            .iter()
            .any(|d| matches!(d, SourceDefinition::Udp(u) if u.name.name == mi.module_name.name));
        let n = mi.params.as_ref().map_or(0, |p| p.len());
        if is_udp && n > 2 {
            errs.push(format!(
                "UDP instance of '{}' has {n} delays; a UDP takes at most two \
                 (LRM 1800-2017 §29.8)",
                mi.module_name.name
            ));
        }
    }
}

/// §23.6 / §13.3: a name followed by `.member` must be a scope, an interface,
/// a struct or a class handle. A plain net, variable or port is none of these
/// (`assign y = a.b;` with `input a;`), and a subroutine's scope holds only
/// its own ports, variables and named blocks (`my_task.missing = 0;`).
fn check_member_access_roots(items: &[ModuleItem], errs: &mut Vec<String>) {
    // Names declared with a plain (vector or implicit) type.
    let mut plain: HashSet<&str> = HashSet::new();
    let mut other: HashSet<&str> = HashSet::new();
    let is_plain = |dt: &DataType| {
        matches!(
            dt,
            DataType::IntegerVector { .. }
                | DataType::Implicit { .. }
                | DataType::IntegerAtom { .. }
        )
    };
    for it in items {
        match it {
            ModuleItem::PortDeclaration(d) => {
                for v in &d.declarators {
                    if is_plain(&d.data_type) && v.dimensions.is_empty() {
                        plain.insert(v.name.name.as_str());
                    } else {
                        other.insert(v.name.name.as_str());
                    }
                }
            }
            // An array has methods (`q.size()`), so only scalars and vectors.
            ModuleItem::NetDeclaration(d) => {
                for v in &d.declarators {
                    if is_plain(&d.data_type) && v.dimensions.is_empty() {
                        plain.insert(v.name.name.as_str());
                    } else {
                        other.insert(v.name.name.as_str());
                    }
                }
            }
            ModuleItem::DataDeclaration(d) => {
                for v in &d.declarators {
                    if is_plain(&d.data_type) && v.dimensions.is_empty() {
                        plain.insert(v.name.name.as_str());
                    } else {
                        other.insert(v.name.name.as_str());
                    }
                }
            }
            ModuleItem::ModuleInstantiation(mi) => {
                other.extend(mi.instances.iter().map(|i| i.name.name.as_str()))
            }
            ModuleItem::GenerateRegion(_)
            | ModuleItem::GenerateIf(_)
            | ModuleItem::GenerateFor(_)
            | ModuleItem::GenerateCase(_) => return,
            _ => {}
        }
    }
    // Subroutine scopes: the names reachable inside each one.
    let mut subs: HashMap<&str, HashSet<String>> = HashMap::new();
    for it in items {
        let (name, ports, body) = match it {
            ModuleItem::FunctionDeclaration(f) => (&f.name.name.name, &f.ports, &f.items),
            ModuleItem::TaskDeclaration(t) => (&t.name.name.name, &t.ports, &t.items),
            _ => continue,
        };
        let mut names: HashSet<String> = ports.iter().map(|p| p.name.name.clone()).collect();
        for st in body {
            for_each_stmt(st, &mut |s| match &s.kind {
                StatementKind::VarDecl { declarators, .. } => {
                    names.extend(declarators.iter().map(|d| d.name.name.clone()))
                }
                StatementKind::SeqBlock { name: Some(n), .. }
                | StatementKind::ParBlock { name: Some(n), .. } => {
                    names.insert(n.name.clone());
                }
                _ => {}
            });
        }
        subs.insert(name.as_str(), names);
    }
    // Any name a process or subroutine redeclares may mean something else
    // where it is used.
    let mut locals: HashSet<String> = HashSet::new();
    for it in items {
        let stmts: Vec<&Statement> = match it {
            ModuleItem::InitialConstruct(i) => vec![&i.stmt],
            ModuleItem::AlwaysConstruct(a) => vec![&a.stmt],
            ModuleItem::FunctionDeclaration(f) => f.items.iter().collect(),
            ModuleItem::TaskDeclaration(t) => t.items.iter().collect(),
            _ => continue,
        };
        for st in stmts {
            for_each_stmt(st, &mut |s| {
                if let StatementKind::VarDecl { declarators, .. } = &s.kind {
                    locals.extend(declarators.iter().map(|d| d.name.name.clone()));
                }
            });
        }
    }
    let mut hits: Vec<String> = Vec::new();
    module_exprs(items, &mut |e| {
        for_each_expr(e, &mut |x| {
            let ExprKind::MemberAccess { expr, member } = &x.kind else {
                return;
            };
            let ExprKind::Ident(h) = &expr.kind else {
                return;
            };
            if h.path.len() != 1 || !h.path[0].selects.is_empty() {
                return;
            }
            let root = h.path[0].name.name.as_str();
            if locals.contains(root) {
                return;
            }
            if plain.contains(root) && !other.contains(root) && !subs.contains_key(root) {
                hits.push(format!(
                    "'{root}' is not a scope, so '{root}.{}' names nothing \
                     (LRM 1800-2017 §23.6)",
                    member.name
                ));
            } else if let Some(names) = subs.get(root)
                && !plain.contains(root)
                && !other.contains(root)
                && !names.contains(&member.name)
            {
                hits.push(format!(
                    "'{}' is not declared in subroutine '{root}' (LRM 1800-2017 §23.6)",
                    member.name
                ));
            }
        })
    });
    hits.sort();
    hits.dedup();
    errs.extend(hits);
}

/// §13.5: an `event` is not a value, so it cannot be passed to a subroutine
/// input that is not itself an event (`func(evt)` with `input arg;`).
fn check_event_arguments(items: &[ModuleItem], errs: &mut Vec<String>) {
    let mut events: HashSet<&str> = HashSet::new();
    for it in items {
        if let ModuleItem::DataDeclaration(d) = it
            && matches!(
                d.data_type,
                DataType::Simple {
                    kind: xezim_core::ast::types::SimpleType::Event,
                    ..
                }
            )
        {
            events.extend(d.declarators.iter().map(|v| v.name.name.as_str()));
        }
    }
    if events.is_empty() {
        return;
    }
    let mut subs: HashMap<&str, &[xezim_core::ast::decl::FunctionPort]> = HashMap::new();
    for it in items {
        match it {
            ModuleItem::FunctionDeclaration(f) => {
                subs.insert(f.name.name.name.as_str(), &f.ports);
            }
            ModuleItem::TaskDeclaration(t) => {
                subs.insert(t.name.name.name.as_str(), &t.ports);
            }
            _ => {}
        }
    }
    let mut hits: Vec<String> = Vec::new();
    module_exprs(items, &mut |e| {
        for_each_expr(e, &mut |x| {
            let ExprKind::Call { func, args } = &x.kind else {
                return;
            };
            let ExprKind::Ident(h) = &func.kind else {
                return;
            };
            if h.path.len() != 1 {
                return;
            }
            let Some(ports) = subs.get(h.path[0].name.name.as_str()) else {
                return;
            };
            for (a, p) in args.iter().zip(ports.iter()) {
                if let ExprKind::Ident(ah) = &a.kind
                    && ah.path.len() == 1
                    && ah.path[0].selects.is_empty()
                    && events.contains(ah.path[0].name.name.as_str())
                    && !matches!(
                        p.data_type,
                        DataType::Simple {
                            kind: xezim_core::ast::types::SimpleType::Event,
                            ..
                        }
                    )
                {
                    hits.push(format!(
                        "event '{}' passed to non-event argument '{}' of '{}' \
                         (LRM 1800-2017 §13.5)",
                        ah.path[0].name.name, p.name.name, h.path[0].name.name
                    ));
                }
            }
        })
    });
    hits.sort();
    hits.dedup();
    errs.extend(hits);
}

/// §6.10: an undeclared name implies a net only as the target of a
/// continuous assignment or as a port or terminal connection. Read on the
/// right-hand side of a module-level continuous assignment it is undeclared.
/// Top module only, whose implicit nets the elaboration lists.
fn check_cont_assign_rhs_names(
    ports: &PortList,
    items: &[ModuleItem],
    is_top: bool,
    pkg_names: &HashSet<String>,
    elab: &ElaboratedModule,
    errs: &mut Vec<String>,
) {
    fn bare(e: &Expression, out: &mut Vec<String>) {
        match &e.kind {
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                out.push(h.path[0].name.name.clone())
            }
            ExprKind::Concatenation(xs) => xs.iter().for_each(|x| bare(x, out)),
            ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => bare(expr, out),
            ExprKind::Paren(x) => bare(x, out),
            _ => {}
        }
    }
    fn reads(e: &Expression, out: &mut Vec<String>) {
        match &e.kind {
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                out.push(h.path[0].name.name.clone())
            }
            ExprKind::Unary { operand, .. } => reads(operand, out),
            ExprKind::Binary { left, right, .. } => {
                reads(left, out);
                reads(right, out);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                reads(condition, out);
                reads(then_expr, out);
                reads(else_expr, out);
            }
            ExprKind::Concatenation(xs) => xs.iter().for_each(|x| reads(x, out)),
            ExprKind::Replication { count, exprs } => {
                reads(count, out);
                exprs.iter().for_each(|x| reads(x, out));
            }
            ExprKind::Index { expr, index } => {
                reads(expr, out);
                reads(index, out);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                reads(expr, out);
                reads(left, out);
                reads(right, out);
            }
            ExprKind::Paren(x) => reads(x, out),
            ExprKind::Call { args, .. } | ExprKind::SystemCall { args, .. } => {
                args.iter().for_each(|a| reads(a, out))
            }
            _ => {}
        }
    }
    // Every name the module declares itself (an unpacked array is listed
    // with the implicit nets as a placeholder).
    fn declared(items: &[ModuleItem], out: &mut HashSet<String>) {
        use xezim_core::ast::decl::ParameterKind;
        for it in items {
            match it {
                ModuleItem::DataDeclaration(d) => {
                    out.extend(d.declarators.iter().map(|v| v.name.name.clone()))
                }
                ModuleItem::NetDeclaration(d) => {
                    out.extend(d.declarators.iter().map(|v| v.name.name.clone()))
                }
                ModuleItem::PortDeclaration(d) => {
                    out.extend(d.declarators.iter().map(|v| v.name.name.clone()))
                }
                ModuleItem::ParameterDeclaration(pd) | ModuleItem::LocalparamDeclaration(pd) => {
                    if let ParameterKind::Data { assignments, .. } = &pd.kind {
                        out.extend(assignments.iter().map(|a| a.name.name.clone()));
                    }
                }
                ModuleItem::GenerateRegion(g) => declared(&g.items, out),
                ModuleItem::GenerateFor(g) => declared(&g.items, out),
                ModuleItem::GenerateIf(g) => g.branches.iter().for_each(|(_, b)| declared(b, out)),
                ModuleItem::GenerateCase(g) => g.arms.iter().for_each(|a| declared(&a.items, out)),
                _ => {}
            }
        }
    }
    if !is_top {
        return;
    }
    let mut names = HashSet::new();
    declared(items, &mut names);
    match ports {
        PortList::Ansi(ps) => names.extend(ps.iter().map(|p| p.name.name.clone())),
        PortList::NonAnsi(ns) => names.extend(ns.iter().map(|n| n.name.clone())),
        PortList::Empty => {}
    }
    // Names an implicit net may legally come from.
    let mut implied = Vec::new();
    let mut rhs = Vec::new();
    for it in items {
        match it {
            ModuleItem::ContinuousAssign(ca) => {
                for (l, r) in &ca.assignments {
                    bare(l, &mut implied);
                    reads(r, &mut rhs);
                }
            }
            ModuleItem::ModuleInstantiation(mi) => {
                for c in mi.instances.iter().flat_map(|i| &i.connections) {
                    match c {
                        xezim_core::ast::decl::PortConnection::Ordered(Some(e))
                        | xezim_core::ast::decl::PortConnection::Named { expr: Some(e), .. } => {
                            bare(e, &mut implied)
                        }
                        _ => {}
                    }
                }
            }
            ModuleItem::GateInstantiation(g) => {
                for t in g.instances.iter().flat_map(|i| &i.terminals) {
                    bare(t, &mut implied);
                }
            }
            _ => {}
        }
    }
    rhs.sort();
    rhs.dedup();
    for n in rhs {
        if elab.implicit_nets.contains(&n)
            && !names.contains(&n)
            && !implied.contains(&n)
            && !pkg_names.contains(&n)
        {
            errs.push(format!(
                "'{n}' is read by a continuous assignment but never declared (LRM 1800-2017 §6.10)"
            ));
        }
    }
}
