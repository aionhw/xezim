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
use xezim_core::Value;
use xezim_core::ast::decl::{
    ClassDeclaration, ClassItem, ClassMethodKind, ClassQualifier, ModuleItem, ParameterDeclaration,
    ParameterKind, TypedefDeclaration,
};
use xezim_core::ast::module::ModuleDeclaration;
use xezim_core::ast::expr::{
    AssignmentPatternItem, ExprKind, Expression, NumberBase, NumberLiteral, RangeKind,
};
use xezim_core::ast::stmt::{Statement, StatementKind, VarDeclarator};
use xezim_core::ast::types::{
    DataType, EnumType, IntegerAtomType, PackedDimension, PortDirection, Signing, UnpackedDimension,
};
use xezim_core::elaborate::{ElaboratedModule, ParamEnv, ParamLookup};

/// The elaboration's parameter table type (`ElaboratedModule::parameters`).
type ParamMap = xezim_core::hasher::HashMap<String, Value>;

/// Run the second-pass lint over every top-level definition. Returns a list of
/// error messages (empty == clean).
pub fn lint_should_fail(defs: &[&SourceDefinition], elab: &ElaboratedModule) -> Vec<String> {
    let mut errs = Vec::new();
    // §23.3.2: map every module/interface/program to its declared port names,
    // so a named port connection to a non-existent port can be rejected.
    let port_map = build_port_map(defs);
    let pkg_decls = package_decls(defs);
    let pkg_names: HashSet<String> = pkg_decls.values().flatten().cloned().collect();
    // Built once per run, not per identifier and per module (#872): the last
    // segments of the parameter keys and the classes every unit sees.
    let _leaves = ParamLeaves::install(elab);
    let global = global_classes(defs);
    // One set of per-instance views, shared with the type lint's generate
    // selection (a large design has many instances and parameters).
    let gen_sel = GenerateSelection::new(elab);
    let views = &gen_sel.views;
    let elaborated = elaborated_definitions(defs, elab);
    let mut modules_by_name: HashMap<&str, &ModuleDeclaration> = HashMap::default();
    let mut unit_typedefs: HashMap<&str, &DataType> = HashMap::default();
    let mut udp_names: HashSet<&str> = HashSet::default();
    for d in defs {
        match d {
            SourceDefinition::Module(m) => {
                modules_by_name.entry(m.name.name.as_str()).or_insert(m);
            }
            SourceDefinition::Typedef(t) => {
                unit_typedefs
                    .entry(t.name.name.as_str())
                    .or_insert(&t.data_type);
            }
            SourceDefinition::Udp(u) => {
                udp_names.insert(u.name.name.as_str());
            }
            _ => {}
        }
    }
    for def in defs {
        match def {
            SourceDefinition::Class(c) => check_class(c, &mut errs),
            SourceDefinition::Typedef(t) => {
                // §3.12.1: `$unit` folds in the top's table.
                let unit = ConstScope {
                    elab,
                    top: true,
                    inst: None,
                };
                check_typedef(t, unit, &Default::default(), &mut errs);
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
                    elaborated: elaborated.as_ref().is_none_or(|e| e.contains(&m.name.name)),
                };
                check_unit(
                    &unit,
                    &modules_by_name,
                    &unit_typedefs,
                    &udp_names,
                    elab,
                    &global,
                    &pkg_decls,
                    &pkg_names,
                    &mut errs,
                );
                let mut local_types = std::collections::HashSet::new();
                collect_local_type_names(&m.params, &m.items, &mut local_types);
                check_subroutine_port_types(&m.items, &local_types, &pkg_names, elab, &mut errs);
                let scope = ConstScope::of(elab, &m.name.name);
                check_items_in_scopes(
                    &m.name.name,
                    &m.items,
                    scope,
                    views,
                    &local_types,
                    &mut errs,
                );
                check_proc_net_assign(&m.name.name, &m.ports, &m.items, elab, &mut errs);
                check_monitor_automatic(&m.items, &mut errs);
                check_unpacked_pattern_counts(&m.items, &mut errs);
                check_stream_widths(&m.items, scope, &mut errs);
                check_wildcard_cmp(&m.items, elab, &mut errs);
                if unit.elaborated {
                    check_instantiations(&m.items, &port_map, &mut errs);
                    check_implicit_ports(
                        &m.ports,
                        &m.items,
                        m.default_net_type.is_some(),
                        &port_map,
                        &mut errs,
                    );
                }
            }
            SourceDefinition::Interface(m) => {
                let unit = Unit {
                    name: &m.name.name,
                    params: &m.params,
                    ports: &m.ports,
                    items: &m.items,
                    elaborated: elaborated.as_ref().is_none_or(|e| e.contains(&m.name.name)),
                };
                check_unit(
                    &unit,
                    &modules_by_name,
                    &unit_typedefs,
                    &udp_names,
                    elab,
                    &global,
                    &pkg_decls,
                    &pkg_names,
                    &mut errs,
                );
                let mut local_types = std::collections::HashSet::new();
                collect_local_type_names(&m.params, &m.items, &mut local_types);
                check_subroutine_port_types(&m.items, &local_types, &pkg_names, elab, &mut errs);
                let scope = ConstScope::of(elab, &m.name.name);
                check_items_in_scopes(
                    &m.name.name,
                    &m.items,
                    scope,
                    views,
                    &local_types,
                    &mut errs,
                );
                check_stream_widths(&m.items, scope, &mut errs);
                check_wildcard_cmp(&m.items, elab, &mut errs);
                check_monitor_automatic(&m.items, &mut errs);
                check_unpacked_pattern_counts(&m.items, &mut errs);
                if unit.elaborated {
                    check_instantiations(&m.items, &port_map, &mut errs);
                    check_implicit_ports(
                        &m.ports,
                        &m.items,
                        m.default_net_type.is_some(),
                        &port_map,
                        &mut errs,
                    );
                }
            }
            SourceDefinition::Program(m) => {
                let unit = Unit {
                    name: &m.name.name,
                    params: &m.params,
                    ports: &m.ports,
                    items: &m.items,
                    elaborated: elaborated.as_ref().is_none_or(|e| e.contains(&m.name.name)),
                };
                check_unit(
                    &unit,
                    &modules_by_name,
                    &unit_typedefs,
                    &udp_names,
                    elab,
                    &global,
                    &pkg_decls,
                    &pkg_names,
                    &mut errs,
                );
                let mut local_types = std::collections::HashSet::new();
                collect_local_type_names(&m.params, &m.items, &mut local_types);
                check_subroutine_port_types(&m.items, &local_types, &pkg_names, elab, &mut errs);
                let scope = ConstScope::of(elab, &m.name.name);
                check_items_in_scopes(
                    &m.name.name,
                    &m.items,
                    scope,
                    views,
                    &local_types,
                    &mut errs,
                );
                check_stream_widths(&m.items, scope, &mut errs);
                check_wildcard_cmp(&m.items, elab, &mut errs);
                check_program_items(&m.items, &mut errs);
                check_monitor_automatic(&m.items, &mut errs);
                check_unpacked_pattern_counts(&m.items, &mut errs);
                if unit.elaborated {
                    check_instantiations(&m.items, &port_map, &mut errs);
                    check_implicit_ports(
                        &m.ports,
                        &m.items,
                        m.default_net_type.is_some(),
                        &port_map,
                        &mut errs,
                    );
                }
            }
            SourceDefinition::Package(p) => {
                use xezim_core::ast::decl::PackageItem;
                // A package's own bare names are not resolved here.
                let scope = ConstScope {
                    elab,
                    top: false,
                    inst: None,
                };
                for it in &p.items {
                    match it {
                        PackageItem::Class(c) => check_class(c, &mut errs),
                        PackageItem::Data(d) => check_enum_type(&d.data_type, scope, &mut errs),
                        PackageItem::Typedef(td) => {
                            check_enum_type(&td.data_type, scope, &mut errs)
                        }
                        _ => {}
                    }
                }
            }
            // §29 UDPs carry no should-fail lints (truth tables only).
            SourceDefinition::Udp(_) => {}
        }
    }
    let declared = design_declared_names(defs);
    check_class_and_package_undeclared_with_declared(defs, elab, &pkg_names, &declared, &mut errs);
    check_undeclared_type_names(defs, elab, &pkg_names, declared, &mut errs);
    check_nettype_partial_drivers(defs, elab, &mut errs);
    check_named_block_refs(defs, &mut errs);
    check_forward_typedefs(defs, elab, &mut errs);
    check_unspecialized_class_scopes(defs, &mut errs);
    if xezim_core::sv_parser::strict_checks() {
        errs.extend(crate::type_lint::check_with_selection(defs, elab, &gen_sel));
        // §23.8: a hierarchical name that does not resolve is an error.
        errs.extend(crate::hier_ref_lint::check(defs, elab));
    }
    errs
}

/// A module, interface or program, as the scope checks see it.
struct Unit<'a> {
    name: &'a str,
    params: &'a [xezim_core::ast::decl::ParameterDeclaration],
    ports: &'a PortList,
    items: &'a [ModuleItem],
    /// Some instance of the definition may be elaborated (see
    /// `elaborated_definitions`); its instantiations are judged only then.
    elaborated: bool,
}

/// The declaration and reference checks shared by modules, interfaces and
/// programs. Checks that resolve names against the elaborated design run on
/// the top unit only, whose names the elaboration holds.
fn check_unit<'a>(
    u: &Unit<'a>,
    modules_by_name: &HashMap<&str, &'a ModuleDeclaration>,
    unit_typedefs: &HashMap<&str, &'a DataType>,
    udp_names: &HashSet<&str>,
    elab: &ElaboratedModule,
    global: &HashMap<&'a str, &'a ClassDeclaration>,
    pkg_decls: &HashMap<String, HashSet<String>>,
    pkg_names: &HashSet<String>,
    errs: &mut Vec<String>,
) {
    let is_top = ConstScope::of(elab, u.name).top;
    check_param_value_refs(u.params, u.items, is_top, pkg_names, elab, errs);
    check_pattern_counts(unit_typedefs, u.items, elab, is_top, errs);
    check_foreach_dims(u.items, errs);
    check_select_depth(u.items, errs);
    if u.elaborated {
        check_port_actuals(modules_by_name, u.items, errs);
    }
    check_generate_block_names(u.items, errs);
    check_system_task_values(u.items, errs);
    check_wildcard_import_conflicts(u.name, u.ports, u.params, u.items, pkg_decls, errs);
    check_imported_hier_refs(modules_by_name, u.items, pkg_decls, errs);
    let classes = visible_classes(global, u.items);
    check_inherited_local_access(&classes, u.items, errs);
    check_udp_instance_delays(udp_names, u.items, errs);
    check_member_access_roots(u.items, errs);
    check_event_arguments(u.items, elab, errs);
    check_subroutine_range_idents(u.items, is_top, pkg_names, elab, errs);
    check_param_override_idents(u.items, is_top, pkg_names, elab, errs);
    check_cont_assign_rhs_names(u.ports, u.items, is_top, pkg_names, elab, errs);
    check_nonansi_ports_declared(u.name, u.ports, u.items, errs);
    check_interconnect_usage(u, elab, errs);
}

/// The per-item value checks of a definition, in every scope its constants
/// fold in: the top's own table, or the table of each instance of any other
/// definition (#893) — a child's constants are checked for every instance,
/// since an override may make one instance illegal. The same message from
/// several instances is reported once. A definition nothing instantiates
/// keeps the definition-only fold (`ConstScope::of`).
fn check_items_in_scopes(
    name: &str,
    items: &[ModuleItem],
    scope: ConstScope,
    views: &InstanceViews,
    local_types: &std::collections::HashSet<String>,
    errs: &mut Vec<String>,
) {
    let scoped = &views.scoped;
    let views = if scope.top { &[][..] } else { views.of(name) };
    if views.is_empty() {
        for it in items {
            check_module_item(it, scope, local_types, true, errs);
        }
        return;
    }
    let mut seen: HashSet<String> = HashSet::default();
    let mut found = Vec::new();
    for view in views {
        let scope = ConstScope::instance(scope.elab, view, scoped);
        for it in items {
            check_module_item(it, scope, local_types, true, &mut found);
        }
        for e in found.drain(..) {
            if seen.insert(e.clone()) {
                errs.push(e);
            }
        }
    }
}

/// Classes can appear nested inside module/interface/program bodies.
///
/// `live` is false inside a generate branch that is not (provably) the one
/// elaborated: value-dependent checks (a zero-width select) are skipped there,
/// since §27.5 never elaborates an unselected branch.
fn check_module_item(
    item: &ModuleItem,
    scope: ConstScope,
    local_types: &std::collections::HashSet<String>,
    live: bool,
    errs: &mut Vec<String>,
) {
    match item {
        ModuleItem::ClassDeclaration(c) => check_class(c, errs),
        ModuleItem::TypedefDeclaration(t) => check_typedef(t, scope, local_types, errs),
        ModuleItem::DataDeclaration(d) => {
            check_enum_type(&d.data_type, scope, errs);
            check_packed_dims(&d.data_type, scope.elab, errs);
            for decl in &d.declarators {
                check_array_flat_init(decl, scope, errs);
                check_unpacked_dims(&decl.name.name, &decl.dimensions, scope, errs);
                check_new_array_target(&d.data_type, decl, errs);
            }
        }
        ModuleItem::NetDeclaration(d) => {
            check_enum_type(&d.data_type, scope, errs);
            check_packed_dims(&d.data_type, scope.elab, errs);
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
                for_each_stmt_expr(&a.stmt, &mut |e| check_zero_slice(e, scope, errs));
            }
            check_always_has_timing_control(a, scope.elab, errs);
        }
        ModuleItem::InitialConstruct(i) => {
            if live {
                for_each_stmt_expr(&i.stmt, &mut |e| check_zero_slice(e, scope, errs));
            }
        }
        ModuleItem::ContinuousAssign(ca) => {
            if live {
                for (l, r) in &ca.assignments {
                    for_each_expr(l, &mut |e| check_zero_slice(e, scope, errs));
                    for_each_expr(r, &mut |e| check_zero_slice(e, scope, errs));
                }
            }
        }
        ModuleItem::GateInstantiation(g) => check_gate_terminals(g, errs),
        // The per-item checks must SEE inside generate constructs — without
        // this recursion every rule above was bypassed by wrapping the illegal
        // code in `generate begin ... end` (same walker-coverage bug class as
        // the library resolver's).
        ModuleItem::GenerateRegion(gr) => {
            for it in &gr.items {
                check_module_item(it, scope, local_types, live, errs);
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
                    Some(c) => match (width_is_confident(c, scope), scope.value(c)) {
                        (true, Some(v)) => v != 0,
                        _ => {
                            decided = false;
                            false
                        }
                    },
                };
                taken |= this_live;
                for it in items {
                    check_module_item(it, scope, local_types, this_live, errs);
                }
            }
        }
        ModuleItem::GenerateFor(gf) => {
            for it in &gf.items {
                check_module_item(it, scope, local_types, live, errs);
            }
        }
        ModuleItem::GenerateCase(gc) => {
            for arm in &gc.arms {
                for it in &arm.items {
                    check_module_item(it, scope, local_types, false, errs);
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

/// §5.10/§10.9.1: an unpacked array with an ordered assignment pattern must
/// have exactly one element per array entry. A flat C-style list (e.g.
/// `ms_t ms[1:0] = '{0,0,1,1};` for a 2-entry array) is illegal. Conservative:
/// only single-dimension arrays with an all-ordered, all-scalar pattern whose
/// element count differs from the (constant-folded) array size are flagged —
/// nested patterns, default/replication, and non-const sizes are skipped.
fn check_array_flat_init(d: &VarDeclarator, scope: ConstScope, errs: &mut Vec<String>) {
    if d.dimensions.len() != 1 {
        return;
    }
    let UnpackedDimension::Range { left, right, .. } = &d.dimensions[0] else {
        return;
    };
    let (Some(l), Some(r)) = (scope.value(left), scope.value(right)) else {
        return;
    };
    let n = (l - r).abs() + 1;
    let Some(init) = &d.init else { return };
    let ExprKind::AssignmentPattern(items) = &init.peel_typed_pattern().kind else {
        return;
    };
    let mut m: i64 = 0;
    for it in items {
        match it {
            AssignmentPatternItem::Ordered(e) => {
                // a nested pattern/replication is a proper per-entry init, not flat
                if matches!(
                    e.kind,
                    ExprKind::AssignmentPattern(_)
                        | ExprKind::TypedPattern { .. }
                        | ExprKind::Replication { .. }
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

/// §6.5/§10.4: procedural writes require variable lvalues.
fn check_proc_net_assign(
    module: &str,
    ports: &PortList,
    items: &[ModuleItem],
    elab: &ElaboratedModule,
    errs: &mut Vec<String>,
) {
    if !items.iter().any(|item| {
        matches!(
            item,
            ModuleItem::AlwaysConstruct(_)
                | ModuleItem::InitialConstruct(_)
                | ModuleItem::FinalConstruct(_)
                | ModuleItem::FunctionDeclaration(_)
                | ModuleItem::TaskDeclaration(_)
        )
    }) {
        return;
    }
    let mut nets = HashSet::new();
    let mut implicit_output_nets = HashSet::new();
    for item in items {
        match item {
            ModuleItem::NetDeclaration(nd) => {
                nets.extend(nd.declarators.iter().map(|d| d.name.name.as_str()));
            }
            ModuleItem::DataDeclaration(d) => {
                // User nettypes use the data declaration syntax. The top's
                // elaborated net table distinguishes them from typedefs.
                for decl in &d.declarators {
                    let name = decl.name.name.as_str();
                    if d.net_type.is_some()
                        || (module == elab.name
                            && matches!(d.data_type, DataType::TypeReference { .. })
                            && elab.nets.contains(name)
                            && !elab.implicit_nets.contains(name))
                    {
                        nets.insert(name);
                    }
                }
            }
            _ => {}
        }
    }
    if let PortList::Ansi(ps) = ports {
        let mut prev_net = false;
        let mut prev_implicit_net = false;
        for p in ps {
            let omitted =
                p.direction.is_none() && p.net_type.is_none() && !p.var_kw && p.data_type.is_none();
            let implicit = p.net_type.is_none()
                && p.direction == Some(PortDirection::Output)
                && !p.var_kw
                && matches!(p.data_type, None | Some(DataType::Implicit { .. }));
            let net = if omitted {
                prev_net
            } else {
                p.net_type.is_some() && !p.var_kw || implicit
            };
            if net {
                nets.insert(p.name.name.as_str());
            }
            if implicit || (omitted && prev_implicit_net) {
                implicit_output_nets.insert(p.name.name.as_str());
            }
            prev_net = net;
            prev_implicit_net = implicit || (omitted && prev_implicit_net);
        }
    }
    if nets.is_empty() {
        return;
    }
    let mut flagged = HashSet::new();
    let mut emit = |name: &str| {
        if nets.contains(name) && flagged.insert(name.to_owned()) {
            let mut message = format!(
                "net '{name}' is the target of a procedural assignment (LRM 1800-2017 §6.5 — nets need a continuous assignment)"
            );
            if implicit_output_nets.contains(name) {
                use std::fmt::Write;
                let _ = write!(
                    message,
                    ". Output port '{name}' in module '{module}' is a net because its declaration has no explicit data type (§23.2.2.3). Use an explicit variable type such as `output logic` for procedural assignments"
                );
            }
            errs.push(message);
        }
    };
    for item in items {
        let mut locals = Vec::new();
        match item {
            ModuleItem::AlwaysConstruct(ac) => {
                visit_proc_net_writes(&ac.stmt, &mut locals, &mut emit)
            }
            ModuleItem::InitialConstruct(ic) => {
                visit_proc_net_writes(&ic.stmt, &mut locals, &mut emit)
            }
            ModuleItem::FinalConstruct(fc) => {
                visit_proc_net_writes(&fc.stmt, &mut locals, &mut emit)
            }
            ModuleItem::FunctionDeclaration(func) => {
                locals.extend(func.ports.iter().map(|port| port.name.name.as_str()));
                locals.extend(func.strict_body_ports.iter().map(|port| port.name.as_str()));
                visit_proc_net_write_list(&func.items, &mut locals, &mut emit);
            }
            ModuleItem::TaskDeclaration(task) => {
                locals.extend(task.ports.iter().map(|port| port.name.name.as_str()));
                locals.extend(task.strict_body_ports.iter().map(|port| port.name.as_str()));
                visit_proc_net_write_list(&task.items, &mut locals, &mut emit);
            }
            _ => {}
        }
    }
}

fn visit_proc_net_write_list<'a>(
    statements: &'a [Statement],
    locals: &mut Vec<&'a str>,
    emit: &mut dyn FnMut(&str),
) {
    let restore = locals.len();
    for stmt in statements {
        if let StatementKind::VarDecl { declarators, .. }
        | StatementKind::ParamDecl { declarators, .. } = &stmt.kind
        {
            locals.extend(declarators.iter().map(|decl| decl.name.name.as_str()));
        }
        visit_proc_net_writes(stmt, locals, emit);
    }
    locals.truncate(restore);
}

fn proc_net_write_target(lvalue: &Expression, locals: &[&str], emit: &mut dyn FnMut(&str)) {
    match &lvalue.kind {
        ExprKind::Concatenation(parts) => {
            for part in parts {
                proc_net_write_target(part, locals, emit);
            }
        }
        ExprKind::Ident(h) if h.path.len() == 1 && h.root.is_none() => {
            let name = h.path[0].name.name.as_str();
            if !locals.contains(&name) {
                emit(name);
            }
        }
        ExprKind::Index { expr, .. }
        | ExprKind::RangeSelect { expr, .. }
        | ExprKind::MemberAccess { expr, .. }
        | ExprKind::Paren(expr) => {
            proc_net_write_target(expr, locals, emit);
        }
        _ => {}
    }
}

fn proc_net_expr_writes(expr: &Expression, locals: &[&str], emit: &mut dyn FnMut(&str)) {
    use xezim_core::ast::expr::UnaryOp;
    xezim_core::elaborate::for_each_sub_expr(expr, &mut |node| match &node.kind {
        ExprKind::AssignExpr { lvalue, .. } => proc_net_write_target(lvalue, locals, emit),
        ExprKind::Unary {
            op: UnaryOp::PreIncr | UnaryOp::PreDecr | UnaryOp::PostIncr | UnaryOp::PostDecr,
            operand,
        } => proc_net_write_target(operand, locals, emit),
        _ => {}
    });
}

fn visit_proc_net_writes<'a>(
    stmt: &'a Statement,
    locals: &mut Vec<&'a str>,
    emit: &mut dyn FnMut(&str),
) {
    use StatementKind as K;
    use xezim_core::ast::stmt::{ForInit, ProceduralContinuous};
    match &stmt.kind {
        K::BlockingAssign { lvalue, rvalue }
        | K::NonblockingAssign { lvalue, rvalue, .. }
        | K::ProceduralContinuous(ProceduralContinuous::Assign { lvalue, rvalue }) => {
            proc_net_write_target(lvalue, locals, emit);
            proc_net_expr_writes(rvalue, locals, emit);
        }
        K::Expr(expr) => proc_net_expr_writes(expr, locals, emit),
        K::If {
            condition,
            then_stmt,
            else_stmt,
            ..
        } => {
            proc_net_expr_writes(condition, locals, emit);
            visit_proc_net_writes(then_stmt, locals, emit);
            if let Some(other) = else_stmt {
                visit_proc_net_writes(other, locals, emit);
            }
        }
        K::Case { items, .. } => {
            for item in items {
                visit_proc_net_writes(&item.stmt, locals, emit);
            }
        }
        K::For {
            init, step, body, ..
        } => {
            let restore = locals.len();
            for item in init {
                match item {
                    ForInit::VarDecl { name, .. } => locals.push(name.name.as_str()),
                    ForInit::Assign { lvalue, .. } => proc_net_write_target(lvalue, locals, emit),
                }
            }
            for expr in step {
                proc_net_expr_writes(expr, locals, emit);
            }
            visit_proc_net_writes(body, locals, emit);
            locals.truncate(restore);
        }
        K::Foreach { body, .. }
        | K::While { body, .. }
        | K::DoWhile { body, .. }
        | K::Repeat { body, .. }
        | K::Forever { body } => visit_proc_net_writes(body, locals, emit),
        K::SeqBlock { stmts, .. } | K::ParBlock { stmts, .. } => {
            visit_proc_net_write_list(stmts, locals, emit);
        }
        K::TimingControl { stmt, .. } | K::Wait { stmt, .. } => {
            visit_proc_net_writes(stmt, locals, emit);
        }
        _ => {}
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

thread_local! {
    /// The last segments (after `.` and after `::`) of every parameter key of
    /// the design being linted, built once per `lint_should_fail` run. An
    /// instance's parameter is keyed `inst.NAME` and a package one
    /// `pkg::NAME`, and splitting every key for every width identifier cost
    /// identifiers x instances (#872).
    static PARAM_LEAVES: std::cell::RefCell<Option<(usize, HashSet<String>)>> =
        const { std::cell::RefCell::new(None) };
}

/// Installs `PARAM_LEAVES` for one lint run and removes it when dropped.
struct ParamLeaves;

impl ParamLeaves {
    fn install(elab: &ElaboratedModule) -> Self {
        let mut leaves = HashSet::default();
        for k in elab.parameters.keys() {
            for leaf in [k.rsplit('.').next(), k.rsplit("::").next()]
                .into_iter()
                .flatten()
            {
                if !leaves.contains(leaf) {
                    leaves.insert(leaf.to_string());
                }
            }
        }
        PARAM_LEAVES.with(|c| *c.borrow_mut() = Some((elab as *const _ as usize, leaves)));
        ParamLeaves
    }

    /// Whether some parameter key of `elab` ends in `leaf`.
    fn contains(elab: &ElaboratedModule, leaf: &str) -> bool {
        PARAM_LEAVES.with(|c| match c.borrow().as_ref() {
            Some((id, leaves)) if *id == elab as *const _ as usize => leaves.contains(leaf),
            _ => elab
                .parameters
                .keys()
                .any(|k| k.rsplit('.').next() == Some(leaf) || k.rsplit("::").next() == Some(leaf)),
        })
    }
}

impl Drop for ParamLeaves {
    fn drop(&mut self) {
        PARAM_LEAVES.with(|c| *c.borrow_mut() = None);
    }
}

/// Folds a value check's constants in one definition (§23.9).
///
/// The top folds against `elab.parameters`. Another definition folds against
/// the parameters of ONE of its instances (`inst`, see [`InstanceViews`]):
/// the elaboration registers a child's constants under the instance path
/// (`u.N`), so a bare name in the definition is read from that table (#893).
/// Without an instance only literals and `pkg::N` (§26.3) fold; a bare name
/// there may be its own.
#[derive(Clone, Copy)]
struct ConstScope<'a> {
    elab: &'a ElaboratedModule,
    top: bool,
    inst: Option<ParamEnv<'a>>,
}

/// The parameters of every instance of each non-top definition, keyed by the
/// definition's name, each with the instance path stripped from its keys
/// (`u.v.N` of instance `u.v` is `N`). An instance's view is its own table
/// laid over `scoped`, the package and class constants (`pkg::N`), which
/// every scope reads by that spelling (§26.3); the two never share a key,
/// since a stripped key holds no `::`. Copying the constants into the table
/// of every instance instead held instances x constants entries (#721).
/// A bare top-level key is NOT part of a child's view: the child's own `N`
/// is registered as `u.N`, and a `$unit` or top-module `N` it could see is
/// told apart from nothing by the flat table.
struct InstanceViews {
    by_def: HashMap<String, Vec<ParamMap>>,
    scoped: ParamMap,
}

impl InstanceViews {
    fn build(elab: &ElaboratedModule) -> Self {
        let mut by_def: HashMap<String, Vec<ParamMap>> = HashMap::default();
        if elab.instances.is_empty() {
            return InstanceViews {
                by_def,
                scoped: ParamMap::default(),
            };
        }
        // Instance path -> (definition, index of its view in `by_def`).
        let mut slots: HashMap<&str, (&str, usize)> = HashMap::default();
        for inst in &elab.instances {
            let views = by_def.entry(inst.def_name.to_string()).or_default();
            views.push(ParamMap::default());
            slots.insert(
                inst.path.as_str(),
                (inst.def_name.as_str(), views.len() - 1),
            );
        }
        let mut scoped = ParamMap::default();
        for (key, value) in &elab.parameters {
            if key.contains("::") {
                scoped.insert(key.clone(), value.clone());
                continue;
            }
            // The longest instance path the key starts with owns it.
            let mut pos = key.len();
            while let Some(dot) = key[..pos].rfind('.') {
                if let Some(&(def, idx)) = slots.get(&key[..dot]) {
                    by_def.get_mut(def).expect("views of a definition")[idx]
                        .insert(key[dot + 1..].to_string(), value.clone());
                    break;
                }
                pos = dot;
            }
        }
        // Multiple instances with identical parameter bindings fold identically,
        // so walking each unique parameter view once suffices.
        for views in by_def.values_mut() {
            if views.len() <= 1 {
                continue;
            }
            let mut had_empty = false;
            let mut seen: HashSet<Vec<(String, Value)>> = HashSet::default();
            views.retain(|map| {
                if map.is_empty() {
                    if had_empty {
                        return false;
                    }
                    had_empty = true;
                    return true;
                }
                let mut entries: Vec<(String, Value)> =
                    map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
                seen.insert(entries)
            });
        }
        InstanceViews { by_def, scoped }
    }

    /// The own tables of the instances of `def`, empty when it is not
    /// instantiated.
    fn of(&self, def: &str) -> &[ParamMap] {
        self.by_def.get(def).map_or(&[], Vec::as_slice)
    }
}

/// §27.5: which branches of a definition's conditional generate constructs
/// the elaborated design holds. Only the branch a construct's constant
/// selects is elaborated, so a check of the definition's source must not
/// judge a branch no instance selects: `if (Full) assign o = r.missing;`
/// is legal while every instance has `Full = 0`.
pub(crate) struct GenerateSelection<'a> {
    elab: &'a ElaboratedModule,
    views: InstanceViews,
}

impl<'a> GenerateSelection<'a> {
    pub(crate) fn new(elab: &'a ElaboratedModule) -> Self {
        GenerateSelection {
            elab,
            views: InstanceViews::build(elab),
        }
    }

    /// The scope of the top, or of each elaborated instance of `def`; empty
    /// when nothing elaborates `def`.
    fn scopes(&self, def: &str) -> Vec<ConstScope<'_>> {
        let top = ConstScope::of(self.elab, def);
        if top.top {
            return vec![top];
        }
        self.views
            .of(def)
            .iter()
            .map(|v| ConstScope::instance(self.elab, v, &self.views.scoped))
            .collect()
    }

    /// The value of a condition in `scope`, when it is certain there.
    fn known(scope: ConstScope, e: &Expression) -> Option<i64> {
        if width_is_confident(e, scope) {
            scope.value(e)
        } else {
            None
        }
    }

    /// For each branch of a generate `if` of `def` (`None` is the final
    /// `else`), the instances that elaborate it, out of `reach`, the ones
    /// that elaborate the enclosing branch (None: every instance, or not
    /// known). A branch no instance in `reach` selects gets an empty set.
    /// Every branch keeps `reach` when one instance's choice is not certain,
    /// and gets None when nothing elaborates `def`.
    pub(crate) fn if_branches(
        &self,
        def: &str,
        conds: &[Option<&Expression>],
        reach: &GenReach,
    ) -> Vec<GenReach> {
        self.select(def, conds.len(), reach, |scope| {
            for (i, c) in conds.iter().enumerate() {
                match c.map(|c| Self::known(scope, c)) {
                    None => return Some(Some(i)),
                    Some(Some(0)) => {}
                    Some(Some(_)) => return Some(Some(i)),
                    Some(None) => return None,
                }
            }
            Some(None)
        })
    }

    /// For each arm of a generate `case` of `def` (an empty list of values
    /// is the `default`), the instances that elaborate it, as for
    /// [`Self::if_branches`].
    pub(crate) fn case_arms(
        &self,
        def: &str,
        selector: &Expression,
        arms: &[&[Expression]],
        reach: &GenReach,
    ) -> Vec<GenReach> {
        self.select(def, arms.len(), reach, |scope| {
            let sel = Self::known(scope, selector)?;
            for (i, vals) in arms.iter().enumerate() {
                let mut hit = false;
                for v in vals.iter() {
                    hit |= Self::known(scope, v)? == sel;
                }
                if hit {
                    return Some(Some(i));
                }
            }
            Some(arms.iter().position(|vals| vals.is_empty()))
        })
    }

    /// Splits `reach` among `n` branches by `choose`, which gives the branch
    /// one instance elaborates (`Some(None)`: none of them), or None when it
    /// is not certain.
    fn select(
        &self,
        def: &str,
        n: usize,
        reach: &GenReach,
        choose: impl Fn(ConstScope) -> Option<Option<usize>>,
    ) -> Vec<GenReach> {
        let scopes = self.scopes(def);
        if scopes.is_empty() {
            return vec![None; n];
        }
        let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
        let all: Vec<usize>;
        let members = match reach {
            Some(r) => r.as_slice(),
            None => {
                all = (0..scopes.len()).collect();
                &all
            }
        };
        for &k in members {
            let Some(&scope) = scopes.get(k) else {
                return vec![reach.clone(); n];
            };
            match choose(scope) {
                Some(Some(i)) => out[i].push(k),
                Some(None) => {}
                None => return vec![reach.clone(); n],
            }
        }
        out.into_iter().map(Some).collect()
    }
}

/// The instances of a definition (indices of its scopes in
/// `GenerateSelection`) that elaborate a generate branch; None for every
/// instance, or when that is not known.
pub(crate) type GenReach = Option<Vec<usize>>;

/// A fold in a `ConstScope`.
enum Folded {
    Value(i64),
    /// Reads a name whose value this definition does not know.
    Hidden,
    /// Not a constant expression (§11.2.1).
    NotConstant,
}

impl<'a> ConstScope<'a> {
    /// The scope of the module, interface or program `name`.
    fn of(elab: &'a ElaboratedModule, name: &str) -> Self {
        ConstScope {
            elab,
            top: name == elab.name,
            inst: None,
        }
    }

    /// The scope of one instance of a non-top definition (#893): its own
    /// table `view` over the constants every scope reads (`scoped`).
    fn instance(elab: &'a ElaboratedModule, view: &'a ParamMap, scoped: &'a ParamMap) -> Self {
        ConstScope {
            elab,
            top: false,
            inst: Some(ParamEnv::Overlay {
                base: scoped,
                top: view,
            }),
        }
    }

    /// The table a constant of this scope folds against.
    fn params(&self) -> ParamEnv<'a> {
        match self.inst {
            Some(view) if !self.top => view,
            _ => ParamEnv::Single(&self.elab.parameters),
        }
    }

    /// The parameter `name` of this scope, when the scope knows it.
    fn param(&self, name: &str) -> Option<&'a Value> {
        if self.top || self.inst.is_some() {
            match self.params() {
                ParamEnv::Single(table) => table.get(name),
                ParamEnv::Overlay { base, top } => top.get(name).or_else(|| base.get(name)),
            }
        } else {
            None
        }
    }

    fn fold(&self, e: &Expression) -> Folded {
        if !self.top {
            let known = match self.inst {
                Some(_) => width_is_confident(e, *self),
                None => self.reads_only_package_constants(e),
            };
            if !known {
                return Folded::Hidden;
            }
        }
        match xezim_core::elaborate::const_eval_i64_with_params_env(e, Some(&self.params())) {
            Some(v) => Folded::Value(v),
            None => Folded::NotConstant,
        }
    }

    fn value(&self, e: &Expression) -> Option<i64> {
        match self.fold(e) {
            Folded::Value(v) => Some(v),
            _ => None,
        }
    }

    /// The width of `dt`, 0 when it is not known here.
    fn type_width(&self, dt: &DataType) -> u32 {
        let bound_known = |e: &Expression| match self.inst {
            Some(_) => width_is_confident(e, *self),
            None => self.reads_only_package_constants(e),
        };
        let known = self.top
            || match dt {
                DataType::IntegerAtom { .. } => true,
                DataType::IntegerVector { dimensions, .. }
                | DataType::Implicit { dimensions, .. } => dimensions.iter().all(|d| {
                    matches!(d, PackedDimension::Range { left, right, .. }
                            if bound_known(left) && bound_known(right))
                }),
                _ => false,
            };
        if !known {
            return 0;
        }
        xezim_core::elaborate::resolve_type_width_env(
            dt,
            Some(&self.params()),
            Some(&self.elab.typedefs),
        )
    }

    /// Whether `e` reads no name but a package constant.
    fn reads_only_package_constants(&self, e: &Expression) -> bool {
        let only = |x: &Expression| self.reads_only_package_constants(x);
        match &e.kind {
            ExprKind::Number(_) => true,
            // `pkg::N` names one constant wherever it is read (§26.3). It
            // parses as a member of the package name.
            ExprKind::MemberAccess { expr: base, .. } => {
                let ExprKind::Ident(h) = &base.kind else {
                    return false;
                };
                let [pkg] = h.path.as_slice() else {
                    return false;
                };
                h.root.is_none()
                    && pkg.selects.is_empty()
                    && self.elab.packages.contains(&pkg.name.name)
            }
            ExprKind::Paren(x) | ExprKind::Unary { operand: x, .. } => only(x),
            ExprKind::Binary { left, right, .. } => only(left) && only(right),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => only(condition) && only(then_expr) && only(else_expr),
            ExprKind::SystemCall { args, .. } => args.iter().all(only),
            _ => false,
        }
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
///
/// In an instance scope (#893) a name is trusted only when that instance's
/// own table holds it: the leaf fallbacks of the flat table would read
/// another instance's parameter of the same name.
fn width_is_confident(e: &Expression, scope: ConstScope) -> bool {
    match &e.kind {
        ExprKind::Number(_) => true,
        ExprKind::Paren(i) => width_is_confident(i, scope),
        ExprKind::Unary { operand, .. } => width_is_confident(operand, scope),
        ExprKind::Binary { left, right, .. } => {
            width_is_confident(left, scope) && width_is_confident(right, scope)
        }
        ExprKind::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            width_is_confident(condition, scope)
                && width_is_confident(then_expr, scope)
                && width_is_confident(else_expr, scope)
        }
        ExprKind::SystemCall { args, .. } => args.iter().all(|a| width_is_confident(a, scope)),
        ExprKind::Ident(h) => {
            let raw = h
                .path
                .iter()
                .map(|s| s.name.name.as_str())
                .collect::<Vec<_>>()
                .join(".");
            let leaf = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
            let params = scope.params();
            params.contains_key(&raw)
                || params.contains_key(leaf)
                || (scope.inst.is_none() && ParamLeaves::contains(scope.elab, leaf))
        }
        // A struct/interface member or an index: only trusted when the exact
        // dotted key was seeded (interface-port parameters do this).
        ExprKind::MemberAccess { expr: base, member } => {
            let ExprKind::Ident(h) = &base.kind else {
                return false;
            };
            let b = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
            let params = scope.params();
            params.contains_key(&format!("{b}.{}", member.name))
                || params.contains_key(&format!("{b}::{}", member.name))
        }
        _ => false,
    }
}

fn check_zero_slice(e: &Expression, scope: ConstScope, errs: &mut Vec<String>) {
    if let ExprKind::RangeSelect { kind, right, .. } = &e.kind {
        if matches!(kind, RangeKind::IndexedUp | RangeKind::IndexedDown) {
            if !width_is_confident(right, scope) {
                return;
            }
            if let Some(0) = scope.value(right) {
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
fn check_stream_widths(items: &[ModuleItem], scope: ConstScope, errs: &mut Vec<String>) {
    // Raw-AST width scan: suppress clamp warnings (elaboration re-resolves
    // every kept declaration and warns there; dead ones get elided instead).
    let vw =
        xezim_core::elaborate::with_width_warnings_suppressed(|| build_var_widths(items, scope));
    for it in items {
        match it {
            ModuleItem::DataDeclaration(d) => {
                for decl in &d.declarators {
                    check_stream_decl(
                        &d.data_type,
                        decl.dimensions.is_empty(),
                        decl.init.as_deref(),
                        &decl.name.name,
                        &vw,
                        scope,
                        errs,
                    );
                }
            }
            ModuleItem::AlwaysConstruct(a) => {
                for_each_stmt(&a.stmt, &mut |s| check_stmt_stream(s, &vw, scope, errs));
            }
            ModuleItem::InitialConstruct(i) => {
                for_each_stmt(&i.stmt, &mut |s| check_stmt_stream(s, &vw, scope, errs));
            }
            _ => {}
        }
    }
}

fn check_stmt_stream(
    s: &Statement,
    vw: &std::collections::HashMap<String, u32>,
    scope: ConstScope,
    errs: &mut Vec<String>,
) {
    if let StatementKind::VarDecl {
        data_type,
        declarators,
        ..
    }
    | StatementKind::ParamDecl {
        data_type,
        declarators,
        ..
    } = &s.kind
    {
        for decl in declarators {
            check_stream_decl(
                data_type,
                decl.dimensions.is_empty(),
                decl.init.as_deref(),
                &decl.name.name,
                vw,
                scope,
                errs,
            );
        }
    }
}

fn check_stream_decl(
    dt: &DataType,
    dims_empty: bool,
    init: Option<&Expression>,
    name: &str,
    vw: &std::collections::HashMap<String, u32>,
    scope: ConstScope,
    errs: &mut Vec<String>,
) {
    if !dims_empty {
        return;
    }
    let Some(init) = init else { return };
    let ExprKind::StreamOp { exprs, .. } = &init.kind else {
        return;
    };
    let Some(target_w) = fixed_int_width(dt, scope) else {
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
    scope: ConstScope,
) -> std::collections::HashMap<String, u32> {
    let mut m = std::collections::HashMap::new();
    for it in items {
        match it {
            ModuleItem::DataDeclaration(d) => {
                for decl in &d.declarators {
                    if decl.dimensions.is_empty() {
                        if let Some(w) = fixed_int_width(&d.data_type, scope) {
                            m.insert(decl.name.name.clone(), w);
                        }
                    }
                }
            }
            ModuleItem::NetDeclaration(d) => {
                for decl in &d.declarators {
                    if decl.dimensions.is_empty() {
                        if let Some(w) = fixed_int_width(&d.data_type, scope) {
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
fn fixed_int_width(dt: &DataType, scope: ConstScope) -> Option<u32> {
    if !matches!(
        dt,
        DataType::IntegerVector { .. } | DataType::IntegerAtom { .. }
    ) {
        return None;
    }
    let w = scope.type_width(dt);
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
fn dim_names_resolve(dt: &DataType, scope: ConstScope) -> bool {
    fn expr_known(e: &Expression, params: ParamEnv) -> bool {
        match &e.kind {
            ExprKind::Number(_) => true,
            ExprKind::Paren(x) => expr_known(x, params),
            ExprKind::Unary { operand, .. } => expr_known(operand, params),
            ExprKind::Binary { left, right, .. } => {
                expr_known(left, params) && expr_known(right, params)
            }
            ExprKind::Ident(h) => h
                .path
                .last()
                .is_some_and(|s| params.contains_key(&s.name.name)),
            // Anything else is not something this lint should judge.
            _ => false,
        }
    }
    let dims = match dt {
        DataType::IntegerVector { dimensions, .. } => dimensions,
        // A non-vector base carries no dimension to be unsure about.
        _ => return true,
    };
    let params = scope.params();
    dims.iter().all(|d| match d {
        PackedDimension::Range { left, right, .. } => {
            expr_known(left, params) && expr_known(right, params)
        }
        _ => false,
    })
}

/// §6.19: in an enum with an explicit base type, a member whose value is a
/// *sized* literal constant must match the base-type width.
fn check_enum_type(dt: &DataType, scope: ConstScope, errs: &mut Vec<String>) {
    let DataType::Enum(et) = dt else { return };
    // Sized-literal-width check requires an explicit base type.
    if let Some(base) = &et.base_type {
        let w = scope.type_width(base);
        if w != 0 && dim_names_resolve(base, scope) {
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
    check_enum_values(et, scope, errs);
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
fn check_enum_values(et: &EnumType, scope: ConstScope, errs: &mut Vec<String>) {
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
    if base.is_some_and(|b| !dim_names_resolve(b, scope)) {
        return;
    }
    let width = base.map(|b| scope.type_width(b)).unwrap_or(32);
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
                match (scope.value(lo_e), scope.value(hi_e)) {
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
                    match scope.fold(init) {
                        Folded::Value(v) => v as i128,
                        Folded::Hidden => return,
                        Folded::NotConstant => {
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
            // §6.19: auto-increment follows the value AFTER casting to the
            // enum base type (e.g. `-4'sd1` on unsigned `bit [3:0]` is 15 -> 16
            // overflows; `4'd15` on `bit signed [3:0]` is -1 -> 0 fits).
            let cast_val: i128 = if signed && (key & (1u128 << (width - 1))) != 0 {
                (key as i128) - (1i128 << width)
            } else {
                key as i128
            };
            next = cast_val + 1;
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
    // Per-method check: a constructor's `super.new(...)` is its first
    // statement. (An `output`/`inout` port default is legal, §13.5.3 of
    // 1800-2023.)
    let has_base = c.extends.is_some();
    for item in &c.items {
        if let ClassItem::Method(m) = item {
            // §18.11: randomization callbacks are functions returning void
            // with no arguments, including declarations in derived classes
            // and extern prototypes. Check before dispatch can execute them.
            let (name, valid_hook) = match &m.kind {
                ClassMethodKind::Function(f)
                | ClassMethodKind::PureVirtual(f)
                | ClassMethodKind::Extern(f) => (
                    f.name.name.name.as_str(),
                    matches!(f.return_type, DataType::Void(_))
                        && f.ports.is_empty()
                        && f.strict_body_ports.is_empty(),
                ),
                ClassMethodKind::Task(t) => (t.name.name.name.as_str(), false),
            };
            if matches!(name, "pre_randomize" | "post_randomize") && !valid_hook {
                errs.push(format!(
                    "class '{}': '{}' must be a function returning void with no arguments \
                     (LRM 1800-2017 §18.11)",
                    c.name.name, name
                ));
            }
            let (body, is_new) = match &m.kind {
                ClassMethodKind::Function(f) => (Some(&f.items), f.name.name.name == "new"),
                ClassMethodKind::Task(t) => (Some(&t.items), t.name.name.name == "new"),
                _ => continue,
            };
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
            ClassItem::Nettype(nt) => {
                names.push(&nt.name.name);
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
            | StatementKind::ParamDecl { .. }
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
    scope: ConstScope,
    local_types: &std::collections::HashSet<String>,
    errs: &mut Vec<String>,
) {
    let elab = scope.elab;
    if t.forward {
        return;
    }
    check_enum_type(&t.data_type, scope, errs);
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
/// `collect_idents` without the names core's binding resolved to a
/// declaration of their own definition (§23.9), an implicit net of that
/// definition included (§6.10): those are declared where they are used.
fn collect_unbound_idents(e: &Expression, out: &mut Vec<String>) {
    match &e.kind {
        ExprKind::Ident(h) if h.path.len() == 1 => {
            if h.bound.as_decl().is_none() {
                out.push(h.path[0].name.name.clone());
            }
        }
        ExprKind::Unary { operand, .. } => collect_unbound_idents(operand, out),
        ExprKind::Binary { left, right, .. } => {
            collect_unbound_idents(left, out);
            collect_unbound_idents(right, out);
        }
        ExprKind::Paren(x) => collect_unbound_idents(x, out),
        ExprKind::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            collect_unbound_idents(condition, out);
            collect_unbound_idents(then_expr, out);
            collect_unbound_idents(else_expr, out);
        }
        _ => {}
    }
}

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
            // §15.3, §15.4, §9.7: the built-in classes of package `std`,
            // visible in every scope.
            | "process"
            | "mailbox"
            | "semaphore"
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
    // Data objects (variables, nets, ports, parameters, subroutines,
    // formals and block locals): a dotted name is a MEMBER select from such
    // a segment on (`resp.exception.valid` reads a struct field, §7.2), so
    // the walk below stops there instead of matching the next segment
    // against a same-named block label. Instance and generate names are
    // scopes and keep the walk going (`U[0].V.a`).
    let mut data: HashSet<String> = HashSet::new();
    fn param_names(params: &[ParameterDeclaration], out: &mut HashSet<String>) {
        for p in params {
            match &p.kind {
                ParameterKind::Data { assignments, .. } => {
                    out.extend(assignments.iter().map(|a| a.name.name.clone()));
                }
                ParameterKind::Type { assignments } => {
                    out.extend(assignments.iter().map(|a| a.name.name.clone()));
                }
            }
        }
    }
    fn ansi_port_names(ports: &PortList, out: &mut HashSet<String>) {
        match ports {
            PortList::Ansi(ps) => out.extend(ps.iter().map(|p| p.name.name.clone())),
            PortList::NonAnsi(ids) => out.extend(ids.iter().map(|i| i.name.clone())),
            PortList::Empty => {}
        }
    }

    fn block_members(stmts: &[Statement], out: &mut HashSet<String>) {
        for st in stmts {
            for_each_stmt(st, &mut |s| match &s.kind {
                StatementKind::VarDecl { declarators, .. }
                | StatementKind::ParamDecl { declarators, .. } => {
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
        data: &mut HashSet<String>,
    ) {
        for it in items {
            match it {
                ModuleItem::InitialConstruct(i) => stmts.push(&i.stmt),
                ModuleItem::AlwaysConstruct(a) => stmts.push(&a.stmt),
                ModuleItem::FinalConstruct(f) => stmts.push(&f.stmt),
                ModuleItem::TaskDeclaration(t) => {
                    other.insert(t.name.name.name.clone());
                    data.insert(t.name.name.name.clone());
                    data.extend(t.ports.iter().map(|p| p.name.name.clone()));
                    stmts.extend(t.items.iter());
                }
                ModuleItem::FunctionDeclaration(f) => {
                    other.insert(f.name.name.name.clone());
                    data.insert(f.name.name.name.clone());
                    data.extend(f.ports.iter().map(|p| p.name.name.clone()));
                    stmts.extend(f.items.iter());
                }
                ModuleItem::ParameterDeclaration(p) | ModuleItem::LocalparamDeclaration(p) => {
                    param_names(std::slice::from_ref(p), data)
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
                    data.extend(d.declarators.iter().map(|x| x.name.name.clone()));
                }
                ModuleItem::NetDeclaration(d) => {
                    other.extend(d.declarators.iter().map(|x| x.name.name.clone()));
                    data.extend(d.declarators.iter().map(|x| x.name.name.clone()));
                }
                ModuleItem::GenerateRegion(g) => walk_items(&g.items, stmts, exprs, other, data),
                ModuleItem::GenerateIf(g) => {
                    other.extend(g.branch_labels.iter().flatten().cloned());
                    for (_, b) in &g.branches {
                        walk_items(b, stmts, exprs, other, data);
                    }
                }
                ModuleItem::GenerateFor(g) => {
                    other.extend(g.name.iter().cloned());
                    walk_items(&g.items, stmts, exprs, other, data);
                }
                ModuleItem::GenerateCase(g) => {
                    for arm in &g.arms {
                        other.extend(arm.label.iter().cloned());
                        walk_items(&arm.items, stmts, exprs, other, data);
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
            SourceDefinition::Module(m) => {
                param_names(&m.params, &mut data);
                ansi_port_names(&m.ports, &mut data);
                &m.items
            }
            SourceDefinition::Interface(m) => {
                param_names(&m.params, &mut data);
                ansi_port_names(&m.ports, &mut data);
                &m.items
            }
            SourceDefinition::Program(m) => {
                param_names(&m.params, &mut data);
                ansi_port_names(&m.ports, &mut data);
                &m.items
            }
            _ => continue,
        };
        walk_items(items, &mut stmts, &mut exprs, &mut other, &mut data);
    }
    for st in &stmts {
        note_stmt(st, &mut blocks);
        for_each_stmt(st, &mut |s| {
            if let StatementKind::VarDecl { declarators, .. }
            | StatementKind::ParamDecl { declarators, .. } = &s.kind
            {
                data.extend(declarators.iter().map(|d| d.name.name.clone()));
            }
        });
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
            if data.contains(&w[0]) {
                break;
            }
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
    scope: ConstScope,
    errs: &mut Vec<String>,
) {
    for d in dims {
        match d {
            UnpackedDimension::Expression { expr, .. } => {
                // Same confidence gate as the §11.5.1 zero-width check: the
                // const evaluator answers 0 for references it cannot resolve
                // (struct-parameter members, per-instance type params), and
                // cva6's `logic [W-1:0] mem[$clog2(Cfg.MemWords)]`-style
                // declarations were rejected as size-0 arrays on legal RTL.
                if !width_is_confident(expr, scope) {
                    continue;
                }
                if let Some(v) = scope.value(expr) {
                    if v <= 0 {
                        errs.push(format!(
                            "array '{}' dimension size must be greater than zero (LRM 1800-2017 §7.4)",
                            name
                        ));
                    }
                }
            }
            // §7.4.2: `[N]` with a bare name parses as an associative
            // dimension keyed by the type `N`; it is a fixed size when `N`
            // names a constant of this scope and no type (#893).
            UnpackedDimension::Associative {
                data_type: Some(dt),
                ..
            } => {
                let DataType::TypeReference {
                    name: key,
                    dimensions,
                    type_args,
                    ..
                } = dt.as_ref()
                else {
                    continue;
                };
                if key.has_scope() || !dimensions.is_empty() || !type_args.is_empty() {
                    continue;
                }
                let n = key.name.name.as_str();
                if scope.elab.typedefs.contains_key(n) || scope.elab.classes.contains_key(n) {
                    continue;
                }
                if let Some(v) = scope.param(n).and_then(|v| v.to_i64()) {
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
                    match scope.fold(bound) {
                        Folded::Value(v) if v < 0 => errs.push(format!(
                            "queue '{}' bound must be positive (LRM 1800-2017 §7.10)",
                            name
                        )),
                        Folded::NotConstant => errs.push(format!(
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

/// The definitions some elaborated instance may come from: the root, the
/// definition of every elaborated instance, and whatever an instantiation in
/// one of those reaches in the source (in any generate branch, or bound in
/// with `bind`), so the set errs on the side of checking. Port connections
/// are checked at elaboration (§23.3.2, §23.3.3), so a definition outside the
/// set — another top-level module that `-s` did not select — has none of its
/// instantiations judged. None when the elaboration recorded no instances.
fn elaborated_definitions(
    defs: &[&SourceDefinition],
    elab: &ElaboratedModule,
) -> Option<HashSet<String>> {
    if elab.instances.is_empty() {
        return None;
    }
    fn instantiated(items: &[ModuleItem], out: &mut Vec<String>) {
        for it in items {
            match it {
                ModuleItem::ModuleInstantiation(mi) => out.push(mi.module_name.name.clone()),
                ModuleItem::Bind(b) => out.push(b.instantiation.module_name.name.clone()),
                ModuleItem::GenerateRegion(g) => instantiated(&g.items, out),
                ModuleItem::GenerateIf(g) => {
                    g.branches.iter().for_each(|(_, i)| instantiated(i, out))
                }
                ModuleItem::GenerateFor(g) => instantiated(&g.items, out),
                ModuleItem::GenerateCase(g) => {
                    g.arms.iter().for_each(|a| instantiated(&a.items, out))
                }
                _ => {}
            }
        }
    }
    let mut kids: HashMap<&str, Vec<String>> = HashMap::new();
    let mut bound: Vec<String> = Vec::new();
    for def in defs {
        let (name, items) = match def {
            SourceDefinition::Module(m) => (&m.name.name, &m.items),
            SourceDefinition::Interface(m) => (&m.name.name, &m.items),
            SourceDefinition::Program(m) => (&m.name.name, &m.items),
            _ => continue,
        };
        let mut out = Vec::new();
        instantiated(items, &mut out);
        bound.extend(items.iter().filter_map(|it| match it {
            ModuleItem::Bind(b) => Some(b.instantiation.module_name.name.clone()),
            _ => None,
        }));
        kids.insert(name.as_str(), out);
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut stack: Vec<String> = std::iter::once(elab.name.clone())
        .chain(elab.instances.iter().map(|i| i.def_name.to_string()))
        .chain(bound)
        .chain(
            elab.resolved_binds
                .values()
                .flatten()
                .map(|b| b.instantiation.module_name.name.clone()),
        )
        .collect();
    while let Some(n) = stack.pop() {
        if let Some(k) = kids.get(n.as_str()) {
            stack.extend(k.iter().filter(|k| !seen.contains(*k)).cloned());
        }
        seen.insert(n);
    }
    Some(seen)
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
                if let Some(name) = conn.named_port() {
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
fn collect_scope_names(
    ports: &PortList,
    items: &[ModuleItem],
    allow_implicit_nets: bool,
) -> Option<HashSet<String>> {
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
                    // §6.10: an undeclared identifier in an instance's port
                    // connection list is an implicit scalar net of this scope,
                    // so a `.name` / `.*` elsewhere in the scope binds to it.
                    if allow_implicit_nets {
                        for conn in &inst.connections {
                            match conn {
                                xezim_core::ast::decl::PortConnection::Ordered(Some(e))
                                | xezim_core::ast::decl::PortConnection::Named {
                                    expr: Some(e),
                                    ..
                                } => collect_implicit_net_names(e, &mut names),
                                xezim_core::ast::decl::PortConnection::NamedIdent {
                                    actual, ..
                                } => {
                                    names.insert(actual.name.clone());
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Some(names)
}

/// The identifiers of a port actual that §6.10 declares as implicit nets when
/// nothing else declares them: the same operand walk as elaboration's
/// `collect_implicit_net_candidates` (xezim-core), which creates those nets
/// before the instances of the scope are connected.
fn collect_implicit_net_names(e: &Expression, out: &mut HashSet<String>) {
    match &e.kind {
        ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
            out.insert(h.path[0].name.name.clone());
        }
        ExprKind::Unary { operand, .. } => collect_implicit_net_names(operand, out),
        ExprKind::Binary { left, right, .. } => {
            collect_implicit_net_names(left, out);
            collect_implicit_net_names(right, out);
        }
        ExprKind::Paren(inner) => collect_implicit_net_names(inner, out),
        ExprKind::Concatenation(parts) => {
            for p in parts {
                collect_implicit_net_names(p, out);
            }
        }
        ExprKind::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            collect_implicit_net_names(condition, out);
            collect_implicit_net_names(then_expr, out);
            collect_implicit_net_names(else_expr, out);
        }
        _ => {}
    }
}

/// §23.3.2.2/§23.3.2.4: an implicit `.name` connection and a `.*` wildcard
/// connection each require a same-named signal to exist in the instantiating
/// scope. Flags a `.name` / `.*` port with no matching signal.
fn check_implicit_ports(
    ports: &PortList,
    items: &[ModuleItem],
    allow_implicit_nets: bool,
    port_map: &HashMap<String, HashSet<String>>,
    errs: &mut Vec<String>,
) {
    let has_implicit_or_wildcard = items.iter().any(|it| {
        let ModuleItem::ModuleInstantiation(inst) = it else {
            return false;
        };
        inst.instances.iter().any(|instance| {
            instance.connections.iter().any(|conn| {
                matches!(
                    conn,
                    xezim_core::ast::decl::PortConnection::Wildcard
                        | xezim_core::ast::decl::PortConnection::Named {
                            expr: None,
                            implicit: true,
                            ..
                        }
                )
            })
        })
    });
    if !has_implicit_or_wildcard {
        return;
    }
    let Some(scope) = collect_scope_names(ports, items, allow_implicit_nets) else {
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
                .filter_map(|c| c.named_port().map(|name| name.name.clone()))
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
                            // §23.3.2.4: `.*` connects only ports that have a
                            // name; a non-ANSI null port or unnamed port
                            // expression (the parser's placeholder) is not
                            // connected and so cannot be missing.
                            let mut missing: Vec<&String> = tports
                                .iter()
                                .filter(|p| {
                                    !scope.contains(*p)
                                        && !explicit.contains(*p)
                                        && !xezim_core::elaborate::is_unnamed_port_placeholder(p)
                                })
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
                // §8.24 / §26.3: an out-of-block class method belongs to
                // its class, not to the package's imported namespace.
                PackageItem::Function(f) if f.name.scopes.is_empty() => {
                    out.insert(f.name.name.name.clone());
                }
                PackageItem::Task(t) if t.name.scopes.is_empty() => {
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
            if let StatementKind::VarDecl { declarators, .. }
            | StatementKind::ParamDecl { declarators, .. } = &st.kind
            {
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
    unit_typedefs: &HashMap<&str, &DataType>,
    items: &[ModuleItem],
    elab: &ElaboratedModule,
    is_top: bool,
    errs: &mut Vec<String>,
) {
    let mut local_typedefs: Option<HashMap<&str, &DataType>> = None;
    for it in items {
        let ModuleItem::DataDeclaration(d) = it else {
            continue;
        };
        let dt: &DataType = match &d.data_type {
            DataType::TypeReference {
                name, dimensions, ..
            } if name.scopes.is_empty() && dimensions.is_empty() => {
                let locals = local_typedefs.get_or_insert_with(|| {
                    let mut m = HashMap::default();
                    for item in items {
                        if let ModuleItem::TypedefDeclaration(t) = item {
                            m.entry(t.name.name.as_str()).or_insert(&t.data_type);
                        }
                    }
                    m
                });
                match locals
                    .get(name.name.name.as_str())
                    .or_else(|| unit_typedefs.get(name.name.name.as_str()))
                {
                    Some(t) => t,
                    None => continue,
                }
            }
            dt => dt,
        };
        let want: Option<i64> = match dt {
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
            }) = v.init.as_deref()
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
            StatementKind::VarDecl { declarators, .. }
            | StatementKind::ParamDecl { declarators, .. } => {
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
                exprs.extend(d.declarators.iter().filter_map(|v| v.init.as_deref()))
            }
            ModuleItem::DataDeclaration(d) => {
                exprs.extend(d.declarators.iter().filter_map(|v| v.init.as_deref()))
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
            if let StatementKind::VarDecl { declarators, .. }
            | StatementKind::ParamDecl { declarators, .. } = &s.kind
            {
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
fn port_directions<'a>(
    ports: &'a PortList,
    items: &'a [ModuleItem],
) -> (Vec<Option<PortDirection>>, HashMap<&'a str, PortDirection>) {
    let mut by_name = HashMap::new();
    match ports {
        PortList::Ansi(ps) => {
            for p in ps {
                if let Some(d) = p.direction
                    && !matches!(p.data_type, Some(DataType::Interface { .. }))
                {
                    by_name.insert(p.name.name.as_str(), d);
                }
            }
        }
        PortList::NonAnsi(_) => {
            for it in items {
                if let ModuleItem::PortDeclaration(d) = it {
                    for v in &d.declarators {
                        by_name.insert(v.name.name.as_str(), d.direction);
                    }
                }
            }
        }
        PortList::Empty => {}
    }
    let by_pos = match ports {
        PortList::Ansi(ps) => ps
            .iter()
            .map(|p| by_name.get(p.name.name.as_str()).copied())
            .collect(),
        PortList::NonAnsi(ns) => ns
            .iter()
            .map(|n| by_name.get(n.name.as_str()).copied())
            .collect(),
        PortList::Empty => Vec::new(),
    };
    (by_pos, by_name)
}

/// §23.3.3: an output or inout port is connected to something that can be
/// driven — a net or variable, a select of one or a concatenation of those,
/// not a literal or an operator expression — and an inout port to a net,
/// never a variable.
fn check_port_actuals<'a>(
    modules_by_name: &HashMap<&str, &'a ModuleDeclaration>,
    items: &[ModuleItem],
    errs: &mut Vec<String>,
) {
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
    #[allow(clippy::type_complexity)]
    let mut dir_cache: HashMap<&'a str, (Vec<Option<PortDirection>>, HashMap<&'a str, PortDirection>)> =
        HashMap::default();
    for it in items {
        let ModuleItem::ModuleInstantiation(mi) = it else {
            continue;
        };
        let Some(&m) = modules_by_name.get(mi.module_name.name.as_str()) else {
            continue;
        };
        let (by_pos, by_name) = dir_cache
            .entry(m.name.name.as_str())
            .or_insert_with(|| port_directions(&m.ports, &m.items));
        for inst in &mi.instances {
            for (i, c) in inst.connections.iter().enumerate() {
                let (dir, bad, port) = match c {
                    PortConnection::Ordered(Some(e)) => {
                        let dir = by_pos.get(i).copied().flatten();
                        let bad = match dir {
                            Some(PortDirection::Output) => !assignable(e),
                            Some(PortDirection::Inout) => {
                                !assignable(e)
                                    || root(e).is_some_and(|r| vars.contains(r) && !ports.contains(r))
                            }
                            _ => false,
                        };
                        (dir, bad, format!("#{}", i + 1))
                    }
                    PortConnection::Named {
                        name,
                        expr: Some(e),
                        ..
                    } => {
                        let dir = by_name.get(name.name.as_str()).copied();
                        let bad = match dir {
                            Some(PortDirection::Output) => !assignable(e),
                            Some(PortDirection::Inout) => {
                                !assignable(e)
                                    || root(e).is_some_and(|r| vars.contains(r) && !ports.contains(r))
                            }
                            _ => false,
                        };
                        (dir, bad, name.name.clone())
                    }
                    PortConnection::NamedIdent { name, actual, .. } => {
                        let dir = by_name.get(name.name.as_str()).copied();
                        let bad = matches!(dir, Some(PortDirection::Inout))
                            && vars.contains(actual.name.as_str())
                            && !ports.contains(actual.name.as_str());
                        (dir, bad, name.name.clone())
                    }
                    _ => continue,
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
// ---------------------------------------------------------------------------
// #888: undeclared identifiers in class and package parameter-override lists
// ---------------------------------------------------------------------------
//
// Three positions are visited by neither the elaborator nor any existing
// should-fail check:
//
//   * a class specialization's parameter-override list (`R #(A #(bad)) x;`,
//     `typedef R #(A #(bad)) t;`, `extends B #(bad)`, a method's return type
//     or port). `check_param_override_idents` covers module instantiations
//     only, and a nested override is worse than merely unvisited: the parser
//     keeps the inner argument list as source text
//     (`ExprKind::Specialization::type_args_text`), so no walk of the
//     expression tree can reach the name at all.
//   * a class value-parameter default (`class A #(int W = bad)`).
//     `check_param_value_refs` is fed module parameters only.
//   * a class property or package-scope variable initializer (`int m = bad;`),
//     which elaboration folds to 0. The package arm of `lint_should_fail`
//     already says as much: "a package's own bare names are not resolved here".
//
// Resolution here is whole-design rather than scoping-accurate: a name is
// reported only when it is declared NOWHERE in the design. That is the same
// "only fire on a definite LRM violation" bar the rest of this file holds, and
// it is what keeps an override that resolves through a package import, a class
// parameter, an enclosing class member or a local typedef clean.

/// Keywords and built-in spellings that are legal in a type-argument position
/// without being declared. `type_args_text` preserves the argument list as
/// source text, so these have to be recognised by spelling.
fn reserved_type_arg_name(n: &str) -> bool {
    matches!(
        n,
        "virtual"
            | "const"
            | "var"
            | "type"
            | "enum"
            | "struct"
            | "union"
            | "packed"
            | "signed"
            | "unsigned"
            | "tagged"
            | "null"
            | "this"
            | "super"
            | "new"
            | "default"
            | "mailbox"
            | "semaphore"
            | "process"
            | "rand"
            | "randc"
            | "void"
    )
}

/// Identifier runs in a preserved type-argument list (`"16, wrong_"`,
/// `"A #(W, D)"`). A run preceded by `::` is a member name owned by the scope
/// to its left and is left to that scope rather than guessed at here.
fn idents_in_type_arg_text(text: &str) -> Vec<String> {
    let cs: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        // A string actual (`holder #(int, "base_test")`) is a value, not a
        // name; its contents are never an identifier.
        if cs[i] == '"' {
            i += 1;
            while i < cs.len() {
                if cs[i] == '\\' {
                    i += 2;
                    continue;
                }
                if cs[i] == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        let word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
        // §5.7: a number is never a name. A digit starts a decimal, real or
        // time literal (`16`, `1.5e3`, `10ns`); `'` plus an optional `s` and
        // a base starts the digits of a based literal (`8'hFF`, `'b0`).
        if cs[i].is_ascii_digit() {
            while i < cs.len() && (word(cs[i]) || cs[i] == '.') {
                i += 1;
            }
            continue;
        }
        if cs[i] == '\'' {
            let mut j = i + 1;
            if j < cs.len() && matches!(cs[j], 's' | 'S') {
                j += 1;
            }
            if j < cs.len() && matches!(cs[j], 'b' | 'B' | 'o' | 'O' | 'd' | 'D' | 'h' | 'H') {
                j += 1;
                // §5.7.1: white space may separate the base from the digits.
                while j < cs.len() && cs[j].is_whitespace() {
                    j += 1;
                }
                while j < cs.len() && (word(cs[j]) || cs[j] == '?') {
                    j += 1;
                }
                i = j;
            } else {
                i += 1;
            }
            continue;
        }
        // §20: a system function (`$clog2`, `$bits`) names nothing in the design.
        if cs[i] == '$' {
            i += 1;
            while i < cs.len() && word(cs[i]) {
                i += 1;
            }
            continue;
        }
        if !(cs[i].is_alphabetic() || cs[i] == '_') {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < cs.len() && (cs[j].is_alphanumeric() || cs[j] == '_' || cs[j] == '$') {
            j += 1;
        }
        let mut k = i;
        while k > 0 && cs[k - 1].is_whitespace() {
            k -= 1;
        }
        let is_member = k >= 2 && cs[k - 1] == ':' && cs[k - 2] == ':';
        if !is_member {
            out.push(cs[i..j].iter().collect());
        }
        i = j;
    }
    out
}
/// The parser retains some type actuals as source text rather than a parsed
/// name — virtual interface types above all (`virtual I`, `virtual my_if #(T)`),
/// whose interface and parameters need interface resolution to judge. Anything
/// that is not a plain identifier is such a retention and is left alone, the
/// same guard `type_lint::check_type_actual` applies.
fn is_plain_ident(n: &str) -> bool {
    let mut ch = n.chars();
    match ch.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    n.chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

/// Every name the design declares, harvested from the AST. Whole-design on
/// purpose — see the note above. Names the elaboration already indexes
/// (`top_declares`) and package members (`pkg_names`) are tested separately.
fn design_declared_names(defs: &[&SourceDefinition]) -> HashSet<String> {
    use xezim_core::ast::decl::PackageItem;
    let mut out = HashSet::new();
    for d in defs {
        match d {
            SourceDefinition::Module(m) => {
                for p in &m.params {
                    note_param_names(p, &mut out);
                }
                note_port_list_names(&m.ports, &mut out);
                note_items_names(&m.items, &mut out);
            }
            SourceDefinition::Interface(m) => {
                for p in &m.params {
                    note_param_names(p, &mut out);
                }
                note_port_list_names(&m.ports, &mut out);
                note_items_names(&m.items, &mut out);
            }
            SourceDefinition::Program(m) => {
                for p in &m.params {
                    note_param_names(p, &mut out);
                }
                note_port_list_names(&m.ports, &mut out);
                note_items_names(&m.items, &mut out);
            }
            SourceDefinition::Package(p) => {
                out.insert(p.name.name.clone());
                for it in &p.items {
                    match it {
                        PackageItem::Parameter(pd) => note_param_names(pd, &mut out),
                        PackageItem::Data(dd) => {
                            note_data_type_names(&dd.data_type, &mut out);
                            note_declarator_names(&dd.declarators, &mut out);
                        }
                        PackageItem::Net(nd) => {
                            note_data_type_names(&nd.data_type, &mut out);
                            note_net_declarator_names(&nd.declarators, &mut out);
                        }
                        PackageItem::Typedef(td) => {
                            out.insert(td.name.name.clone());
                            note_data_type_names(&td.data_type, &mut out);
                        }
                        PackageItem::Function(f) => note_function_names(f, &mut out),
                        PackageItem::Task(t) => note_task_names(t, &mut out),
                        PackageItem::Class(c) => note_class_names(c, &mut out),
                        _ => {}
                    }
                }
            }
            SourceDefinition::Class(c) => note_class_names(c, &mut out),
            SourceDefinition::Typedef(td) => {
                out.insert(td.name.name.clone());
                note_data_type_names(&td.data_type, &mut out);
            }
            SourceDefinition::Udp(_) => {}
        }
    }
    out
}

fn note_param_names(pd: &ParameterDeclaration, out: &mut HashSet<String>) {
    match &pd.kind {
        ParameterKind::Data { assignments, .. } => {
            out.extend(assignments.iter().map(|a| a.name.name.clone()))
        }
        ParameterKind::Type { assignments } => {
            out.extend(assignments.iter().map(|a| a.name.name.clone()))
        }
    }
}

fn note_declarator_names(ds: &[VarDeclarator], out: &mut HashSet<String>) {
    out.extend(ds.iter().map(|d| d.name.name.clone()));
}

/// `NetDeclaration` keeps its own declarator type; same shape, distinct type.
fn note_net_declarator_names(
    ds: &[xezim_core::ast::decl::NetDeclarator],
    out: &mut HashSet<String>,
) {
    out.extend(ds.iter().map(|d| d.name.name.clone()));
}

fn note_data_type_names(dt: &DataType, out: &mut HashSet<String>) {
    match dt {
        DataType::Struct(su) => {
            for m in &su.members {
                out.extend(m.declarators.iter().map(|d| d.name.name.clone()));
                note_data_type_names(&m.data_type, out);
            }
        }
        DataType::Enum(et) => {
            out.extend(et.members.iter().map(|m| m.name.name.clone()));
            if let Some(b) = &et.base_type {
                note_data_type_names(b, out);
            }
        }
        _ => {}
    }
}
fn note_function_names(f: &xezim_core::ast::decl::FunctionDeclaration, out: &mut HashSet<String>) {
    out.insert(f.name.name.name.clone());
    for p in &f.ports {
        out.insert(p.name.name.clone());
        note_data_type_names(&p.data_type, out);
    }
    note_data_type_names(&f.return_type, out);
    for st in f.items.iter() {
        if let StatementKind::VarDecl { declarators, .. }
        | StatementKind::ParamDecl { declarators, .. } = &st.kind
        {
            note_declarator_names(declarators, out);
        }
    }
}

fn note_task_names(t: &xezim_core::ast::decl::TaskDeclaration, out: &mut HashSet<String>) {
    out.insert(t.name.name.name.clone());
    for p in &t.ports {
        out.insert(p.name.name.clone());
        note_data_type_names(&p.data_type, out);
    }
    for st in t.items.iter() {
        if let StatementKind::VarDecl { declarators, .. }
        | StatementKind::ParamDecl { declarators, .. } = &st.kind
        {
            note_declarator_names(declarators, out);
        }
    }
}

fn note_class_names(c: &ClassDeclaration, out: &mut HashSet<String>) {
    out.insert(c.name.name.clone());
    for p in &c.params {
        note_param_names(p, out);
    }
    for it in &c.items {
        match it {
            ClassItem::Property(pr) => {
                note_declarator_names(&pr.declarators, out);
                note_data_type_names(&pr.data_type, out);
            }
            ClassItem::Typedef(td) => {
                out.insert(td.name.name.clone());
                note_data_type_names(&td.data_type, out);
            }
            ClassItem::Nettype(nt) => {
                out.insert(nt.name.name.clone());
                note_data_type_names(&nt.data_type, out);
            }
            ClassItem::Parameter(pd) => note_param_names(pd, out),
            ClassItem::Method(m) => match &m.kind {
                ClassMethodKind::Function(f)
                | ClassMethodKind::PureVirtual(f)
                | ClassMethodKind::Extern(f) => note_function_names(f, out),
                ClassMethodKind::Task(t) => note_task_names(t, out),
            },
            ClassItem::Class(inner) => note_class_names(inner, out),
            _ => {}
        }
    }
}

fn note_items_names(items: &[ModuleItem], out: &mut HashSet<String>) {
    for it in items {
        match it {
            ModuleItem::DataDeclaration(d) => {
                note_declarator_names(&d.declarators, out);
                note_data_type_names(&d.data_type, out);
            }
            ModuleItem::NetDeclaration(d) => {
                note_net_declarator_names(&d.declarators, out);
                note_data_type_names(&d.data_type, out);
            }
            ModuleItem::PortDeclaration(d) => {
                note_declarator_names(&d.declarators, out);
                note_data_type_names(&d.data_type, out);
            }
            ModuleItem::ParameterDeclaration(pd) | ModuleItem::LocalparamDeclaration(pd) => {
                note_param_names(pd, out)
            }
            ModuleItem::TypedefDeclaration(td) => {
                out.insert(td.name.name.clone());
                note_data_type_names(&td.data_type, out);
            }
            ModuleItem::FunctionDeclaration(f) => note_function_names(f, out),
            ModuleItem::TaskDeclaration(t) => note_task_names(t, out),
            ModuleItem::ClassDeclaration(c) => note_class_names(c, out),
            ModuleItem::ModuleInstantiation(mi) => {
                out.extend(mi.instances.iter().map(|i| i.name.name.clone()))
            }
            ModuleItem::GenerateRegion(g) => note_items_names(&g.items, out),
            ModuleItem::GenerateIf(g) => {
                for (_, its) in &g.branches {
                    note_items_names(its, out);
                }
            }
            ModuleItem::GenerateFor(g) => {
                out.insert(g.var.clone());
                note_items_names(&g.items, out);
            }
            ModuleItem::GenvarDeclaration(g) => {
                out.extend(g.names.iter().map(|n| n.name.clone()));
            }
            _ => {}
        }
    }
}

fn note_port_list_names(ports: &PortList, out: &mut HashSet<String>) {
    match ports {
        PortList::Ansi(ps) => {
            for p in ps {
                out.insert(p.name.name.clone());
                if let Some(dt) = &p.data_type {
                    note_data_type_names(dt, out);
                }
            }
        }
        PortList::NonAnsi(ns) => out.extend(ns.iter().map(|n| n.name.clone())),
        PortList::Empty => {}
    }
}
/// The #888 finder: walks the positions listed at the head of this section and
/// reports every identifier declared nowhere in the design.
struct UndeclaredFinder<'a> {
    elab: &'a ElaboratedModule,
    pkg_names: &'a HashSet<String>,
    declared: &'a HashSet<String>,
    /// Identical messages reported once, whichever instance surfaced them.
    seen: HashSet<String>,
    errs: Vec<String>,
}

impl<'a> UndeclaredFinder<'a> {
    /// A name needs no declaration here if it is a keyword, a built-in
    /// spelling, a package member, something the elaboration indexes, or
    /// something the design declares anywhere.
    fn known(&self, n: &str) -> bool {
        !is_plain_ident(n)
            || n.starts_with('$')
            || is_builtin_type(n)
            || reserved_type_arg_name(n)
            || self.pkg_names.contains(n)
            || top_declares(n, self.elab)
            || self.declared.contains(n)
    }

    fn name(&mut self, n: &str, what: &str) {
        if self.known(n) {
            return;
        }
        let msg =
            format!("{what} refers to undeclared identifier '{n}' (LRM 1800-2017 §6.20.2, §8.25)");
        if self.seen.insert(msg.clone()) {
            self.errs.push(msg);
        }
    }

    fn init_idents(&mut self, init: &Expression, label: &str) {
        let mut ids = Vec::new();
        collect_unbound_idents(init, &mut ids);
        let what = format!("initializer of '{label}'");
        for n in ids {
            self.name(&n, &what);
        }
    }

    /// A parameter-override actual: `int`, `my_t`, `A #(bad)`, `.D(bad)`.
    fn actual(&mut self, e: &Expression, what_owner: &str) {
        match &e.kind {
            ExprKind::Ident(h)
                if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty() =>
            {
                let n = h.path[0].name.name.clone();
                self.name(&n, &format!("parameter override of '{what_owner}'"));
            }
            // The nested list is preserved as text, so its names have to be
            // recovered by spelling — this is the miss at the heart of #888.
            ExprKind::Specialization {
                base,
                type_args_text,
            } => {
                self.actual(base, what_owner);
                let what = format!("parameter override of '{what_owner}'");
                for t in idents_in_type_arg_text(type_args_text) {
                    self.name(&t, &what);
                }
            }
            ExprKind::TypeLiteral(dt) => self.data_type(dt, what_owner),
            ExprKind::NamedArg { expr: Some(x), .. } => self.actual(x, what_owner),
            ExprKind::Paren(x) => self.actual(x, what_owner),
            ExprKind::Unary { operand, .. } => self.actual(operand, what_owner),
            ExprKind::Binary { left, right, .. } => {
                self.actual(left, what_owner);
                self.actual(right, what_owner);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.actual(condition, what_owner);
                self.actual(then_expr, what_owner);
                self.actual(else_expr, what_owner);
            }
            _ => {}
        }
    }
    /// Recurse through a type, checking every parameter-override list it holds.
    fn data_type(&mut self, dt: &DataType, owner: &str) {
        match dt {
            DataType::TypeReference { type_args, .. } => {
                for a in type_args {
                    self.actual(a, owner);
                }
            }
            DataType::Interface { type_args, .. } => {
                for a in type_args {
                    self.actual(a, owner);
                }
            }
            DataType::Struct(su) => {
                for m in &su.members {
                    self.data_type(&m.data_type, owner);
                }
            }
            DataType::Enum(et) => {
                if let Some(b) = &et.base_type {
                    self.data_type(b, owner);
                }
            }
            _ => {}
        }
    }

    fn param_default(&mut self, pd: &ParameterDeclaration, owner: &str) {
        if let ParameterKind::Data { assignments, .. } = &pd.kind {
            for a in assignments {
                if let Some(init) = &a.init {
                    let mut ids = Vec::new();
                    collect_idents(init, &mut ids);
                    let what = format!("default value of parameter '{}' of '{owner}'", a.name.name);
                    for n in ids {
                        self.name(&n, &what);
                    }
                }
            }
        }
    }

    fn ports(&mut self, ports: &[xezim_core::ast::decl::FunctionPort], owner: &str) {
        for p in ports {
            self.data_type(&p.data_type, owner);
            if let Some(d) = &p.default {
                self.actual(d, owner);
            }
        }
    }

    fn function(&mut self, f: &xezim_core::ast::decl::FunctionDeclaration) {
        let owner = f.name.name.name.clone();
        self.data_type(&f.return_type, &owner);
        self.ports(&f.ports, &owner);
    }

    fn task(&mut self, t: &xezim_core::ast::decl::TaskDeclaration) {
        let owner = t.name.name.name.clone();
        self.ports(&t.ports, &owner);
    }

    fn class(&mut self, c: &ClassDeclaration) {
        use xezim_core::ast::decl::ParamValue;
        let owner = c.name.name.clone();
        for p in &c.params {
            self.param_default(p, &owner);
        }
        if let Some(x) = &c.extends {
            for a in &x.args {
                match a {
                    ParamValue::Expr(e) => self.actual(e, &owner),
                    ParamValue::Type(t) => self.data_type(t, &owner),
                }
            }
        }
        for it in &c.items {
            match it {
                ClassItem::Property(pr) => {
                    self.data_type(&pr.data_type, &owner);
                    for d in &pr.declarators {
                        if let Some(init) = &d.init {
                            self.init_idents(init, &format!("{}.{}", owner, d.name.name));
                        }
                    }
                }
                ClassItem::Typedef(td) => self.data_type(&td.data_type, &td.name.name),
                ClassItem::Nettype(nt) => self.data_type(&nt.data_type, &nt.name.name),
                ClassItem::Parameter(pd) => self.param_default(pd, &owner),
                ClassItem::Method(m) => match &m.kind {
                    ClassMethodKind::Function(f)
                    | ClassMethodKind::PureVirtual(f)
                    | ClassMethodKind::Extern(f) => self.function(f),
                    ClassMethodKind::Task(t) => self.task(t),
                },
                ClassItem::Class(inner) => self.class(inner),
                _ => {}
            }
        }
    }

    fn items(&mut self, items: &[ModuleItem]) {
        for it in items {
            match it {
                ModuleItem::DataDeclaration(d) => {
                    let owner = d
                        .declarators
                        .first()
                        .map(|x| x.name.name.clone())
                        .unwrap_or_default();
                    self.data_type(&d.data_type, &owner);
                    for dc in &d.declarators {
                        if let Some(init) = &dc.init {
                            self.init_idents(init, &dc.name.name);
                        }
                    }
                }
                ModuleItem::NetDeclaration(d) => {
                    let owner = d
                        .declarators
                        .first()
                        .map(|x| x.name.name.clone())
                        .unwrap_or_default();
                    self.data_type(&d.data_type, &owner);
                    for dc in &d.declarators {
                        if let Some(init) = &dc.init {
                            self.init_idents(init, &dc.name.name);
                        }
                    }
                }
                ModuleItem::PortDeclaration(d) => {
                    let owner = d
                        .declarators
                        .first()
                        .map(|x| x.name.name.clone())
                        .unwrap_or_default();
                    self.data_type(&d.data_type, &owner);
                }
                ModuleItem::TypedefDeclaration(td) => self.data_type(&td.data_type, &td.name.name),
                ModuleItem::ParameterDeclaration(pd) | ModuleItem::LocalparamDeclaration(pd) => {
                    self.param_default(pd, "")
                }
                ModuleItem::FunctionDeclaration(f) => self.function(f),
                ModuleItem::TaskDeclaration(t) => self.task(t),
                ModuleItem::ClassDeclaration(c) => self.class(c),
                ModuleItem::GenerateRegion(g) => self.items(&g.items),
                ModuleItem::GenerateIf(g) => {
                    for (_, its) in &g.branches {
                        self.items(its);
                    }
                }
                ModuleItem::GenerateFor(g) => self.items(&g.items),
                _ => {}
            }
        }
    }
}
/// The #888 entry point: report identifiers in class and package
/// parameter-override lists, class value-parameter defaults, class property
/// initializers and package-scope variable initializers that are declared
/// nowhere in the design.
pub fn check_class_and_package_undeclared(
    defs: &[&SourceDefinition],
    elab: &ElaboratedModule,
    pkg_names: &HashSet<String>,
    errs: &mut Vec<String>,
) {
    let declared = design_declared_names(defs);
    check_class_and_package_undeclared_with_declared(defs, elab, pkg_names, &declared, errs);
}

fn check_class_and_package_undeclared_with_declared(
    defs: &[&SourceDefinition],
    elab: &ElaboratedModule,
    pkg_names: &HashSet<String>,
    declared: &HashSet<String>,
    errs: &mut Vec<String>,
) {
    use xezim_core::ast::decl::PackageItem;
    let mut f = UndeclaredFinder {
        elab,
        pkg_names,
        declared,
        seen: HashSet::new(),
        errs: Vec::new(),
    };
    for d in defs {
        match d {
            SourceDefinition::Module(m) => {
                for p in &m.params {
                    f.param_default(p, &m.name.name);
                }
                f.items(&m.items);
            }
            SourceDefinition::Interface(m) => {
                for p in &m.params {
                    f.param_default(p, &m.name.name);
                }
                f.items(&m.items);
            }
            SourceDefinition::Program(m) => {
                for p in &m.params {
                    f.param_default(p, &m.name.name);
                }
                f.items(&m.items);
            }
            SourceDefinition::Package(p) => {
                for it in &p.items {
                    match it {
                        PackageItem::Parameter(pd) => f.param_default(pd, &p.name.name),
                        PackageItem::Data(dd) => {
                            for dc in &dd.declarators {
                                f.data_type(&dd.data_type, &dc.name.name);
                                if let Some(init) = &dc.init {
                                    f.init_idents(
                                        init,
                                        &format!("{}.{}", p.name.name, dc.name.name),
                                    );
                                }
                            }
                        }
                        PackageItem::Net(nd) => {
                            for dc in &nd.declarators {
                                f.data_type(&nd.data_type, &dc.name.name);
                                if let Some(init) = &dc.init {
                                    f.init_idents(
                                        init,
                                        &format!("{}.{}", p.name.name, dc.name.name),
                                    );
                                }
                            }
                        }
                        PackageItem::Typedef(td) => f.data_type(&td.data_type, &td.name.name),
                        PackageItem::Function(fn_) => f.function(fn_),
                        PackageItem::Task(t) => f.task(t),
                        PackageItem::Class(c) => f.class(c),
                        _ => {}
                    }
                }
            }
            SourceDefinition::Class(c) => f.class(c),
            SourceDefinition::Typedef(td) => f.data_type(&td.data_type, &td.name.name),
            SourceDefinition::Udp(_) => {}
        }
    }
    errs.extend(f.errs);
}

/// §10.3: a continuous assignment to a net of a user-defined nettype drives
/// the whole net, so its left-hand side shall not index or select into the
/// nettype's data type (`assign p.V = v;`, `assign p[0] = b;`). Such an
/// assignment elaborated, and each member then had a driver of its own.
fn check_nettype_partial_drivers(
    defs: &[&SourceDefinition],
    elab: &ElaboratedModule,
    errs: &mut Vec<String>,
) {
    if elab.nettype_resolvers.is_empty() {
        return;
    }
    let is_nettype = |dt: &DataType| {
        let DataType::TypeReference { name, .. } = dt else {
            return false;
        };
        if name.scopes.is_empty() {
            !elab.shadowed_nettypes.contains(&name.name.name)
                && elab.nettype_resolvers.contains_key(&name.name.name)
        } else {
            let scoped = name
                .scopes
                .iter()
                .map(|s| s.name.name.as_str())
                .chain(std::iter::once(name.name.name.as_str()))
                .collect::<Vec<_>>()
                .join("::");
            elab.nettype_resolvers.contains_key(&scoped)
        }
    };
    fn nets_in(
        items: &[ModuleItem],
        is_nettype: &dyn Fn(&DataType) -> bool,
        out: &mut HashMap<String, usize>,
    ) {
        for it in items {
            match it {
                ModuleItem::DataDeclaration(d) if is_nettype(&d.data_type) => out.extend(
                    d.declarators
                        .iter()
                        .map(|x| (x.name.name.clone(), x.dimensions.len())),
                ),
                ModuleItem::NetDeclaration(d) if is_nettype(&d.data_type) => out.extend(
                    d.declarators
                        .iter()
                        .map(|x| (x.name.name.clone(), x.dimensions.len())),
                ),
                ModuleItem::PortDeclaration(d) if is_nettype(&d.data_type) => out.extend(
                    d.declarators
                        .iter()
                        .map(|x| (x.name.name.clone(), x.dimensions.len())),
                ),
                ModuleItem::GenerateRegion(g) => nets_in(&g.items, is_nettype, out),
                ModuleItem::GenerateIf(g) => {
                    for (_, its) in &g.branches {
                        nets_in(its, is_nettype, out);
                    }
                }
                ModuleItem::GenerateFor(g) => nets_in(&g.items, is_nettype, out),
                _ => {}
            }
        }
    }
    /// §10.3: consume unpacked indices before testing whether this LHS
    /// selects into the data type of an individual atomic net.
    fn partial_target(
        e: &Expression,
        nets: &HashMap<String, usize>,
        indices: usize,
        selects_inside_value: bool,
    ) -> Option<String> {
        match &e.kind {
            ExprKind::Ident(h) if h.root.is_none() => {
                let root = h.path.first()?;
                let rank = *nets.get(&root.name.name)?;
                let partial = h.path.len() > 1
                    || selects_inside_value
                    || indices + root.selects.len() > rank
                    || root
                        .selects
                        .iter()
                        .any(|s| matches!(s.kind, ExprKind::RangeSelect { .. }));
                partial.then(|| root.name.name.clone())
            }
            ExprKind::Index { expr, .. } => {
                partial_target(expr, nets, indices + 1, selects_inside_value)
            }
            ExprKind::MemberAccess { expr, .. } | ExprKind::RangeSelect { expr, .. } => {
                partial_target(expr, nets, indices, true)
            }
            ExprKind::Concatenation(xs) => {
                xs.iter().find_map(|x| partial_target(x, nets, 0, false))
            }
            _ => None,
        }
    }
    fn walk(
        items: &[ModuleItem],
        nets: &HashMap<String, usize>,
        elab: &ElaboratedModule,
        errs: &mut Vec<String>,
    ) {
        for it in items {
            match it {
                ModuleItem::ContinuousAssign(ca) => {
                    for (l, _) in &ca.assignments {
                        if let Some(n) = partial_target(l, nets, 0, false) {
                            let at = xezim_core::elaborate::span_location(elab, l.span)
                                .map(|a| format!(" (at {a})"))
                                .unwrap_or_default();
                            errs.push(format!(
                                "continuous assignment to net '{n}' of a user-defined nettype \
                                 selects part of it; it shall drive the whole net (IEEE \
                                 1800-2017 §10.3){at}"
                            ));
                        }
                    }
                }
                ModuleItem::GenerateRegion(g) => walk(&g.items, nets, elab, errs),
                ModuleItem::GenerateIf(g) => {
                    for (_, its) in &g.branches {
                        walk(its, nets, elab, errs);
                    }
                }
                ModuleItem::GenerateFor(g) => walk(&g.items, nets, elab, errs),
                _ => {}
            }
        }
    }
    for d in defs {
        let (ports, items) = match d {
            SourceDefinition::Module(m) => (&m.ports, &m.items),
            SourceDefinition::Interface(m) => (&m.ports, &m.items),
            SourceDefinition::Program(m) => (&m.ports, &m.items),
            _ => continue,
        };
        let mut nets = HashMap::new();
        if let PortList::Ansi(ps) = ports {
            for p in ps {
                if p.data_type.as_ref().is_some_and(|dt| is_nettype(dt)) {
                    nets.insert(p.name.name.clone(), p.dimensions.len());
                }
            }
        }
        nets_in(items, &is_nettype, &mut nets);
        if !nets.is_empty() {
            walk(items, &nets, elab, errs);
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
    let add_params = |pd: &'a xezim_core::ast::decl::ParameterDeclaration,
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
    scope: &str,
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
    for &p in &uniq {
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
            if let StatementKind::VarDecl { declarators, .. }
            | StatementKind::ParamDecl { declarators, .. } = &s.kind
            {
                locals.extend(declarators.iter().map(|d| d.name.name.clone()));
            }
        });
    }
    module_exprs(items, &mut |e| {
        for_each_expr(e, &mut |x| {
            if let ExprKind::Ident(h) = &x.kind
                && h.path.len() == 1
                && h.root.is_none()
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
        let packages = uniq
            .iter()
            .filter(|p| pkgs.get(**p).is_some_and(|names| names.contains(&n)))
            .map(|p| format!("'{p}'"))
            .collect::<Vec<_>>()
            .join(", ");
        errs.push(format!(
            "'{n}' is declared by more than one wildcard-imported package ({packages}), so it is not \
             visible in scope '{scope}' (IEEE 1800-2023 §26.3)"
        ));
    }
}

/// §26.3: a name a module only imports is not one of its members, so a
/// hierarchical reference cannot reach it through an instance (`m.x` where
/// module M does `import P::x;`).
fn check_imported_hier_refs<'a>(
    modules_by_name: &HashMap<&str, &'a ModuleDeclaration>,
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
    #[allow(clippy::type_complexity)]
    let mut mod_cache: HashMap<
        &'a str,
        Option<(HashSet<&'a str>, Vec<&'a str>, HashSet<&'a str>)>,
    > = HashMap::default();
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
            let Some(&m) = modules_by_name.get(module) else {
                return;
            };
            let info = mod_cache.entry(m.name.name.as_str()).or_insert_with(|| {
                let has_gen = m.items.iter().any(|it| {
                    matches!(
                        it,
                        ModuleItem::GenerateRegion(_)
                            | ModuleItem::GenerateIf(_)
                            | ModuleItem::GenerateFor(_)
                            | ModuleItem::GenerateCase(_)
                    )
                });
                if has_gen {
                    None
                } else {
                    let own = module_own_names(&m.ports, &m.params, &m.items);
                    let (wild, explicit) = module_imports(&m.items);
                    Some((own, wild, explicit))
                }
            });
            let Some((own, wild, explicit)) = info else {
                return;
            };
            if own.contains(member) {
                return;
            }
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

/// The classes every module sees: the top-level ones and those in packages.
fn global_classes<'a>(defs: &[&'a SourceDefinition]) -> HashMap<&'a str, &'a ClassDeclaration> {
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
    map
}

/// Every class visible to a module: its own over the global ones. A module
/// that declares no class shares the global map.
fn visible_classes<'g, 'a>(
    global: &'g HashMap<&'a str, &'a ClassDeclaration>,
    items: &'a [ModuleItem],
) -> std::borrow::Cow<'g, HashMap<&'a str, &'a ClassDeclaration>> {
    let own = items.iter().filter_map(|it| match it {
        ModuleItem::ClassDeclaration(c) => Some(c),
        _ => None,
    });
    if own.clone().next().is_none() {
        return std::borrow::Cow::Borrowed(global);
    }
    let mut map = global.clone();
    for c in own {
        map.insert(c.name.name.as_str(), c);
    }
    std::borrow::Cow::Owned(map)
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
                ClassItem::Nettype(nt) => out.push((nt.name.name.as_str(), false)),
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
                    if let StatementKind::VarDecl { declarators, .. }
                    | StatementKind::ParamDecl { declarators, .. } = &s.kind
                    {
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

/// §6.18 & §8.26.2: verify forward typedefs resolve within the same scope, match
/// any explicitly specified basic data type (`enum`/`struct`/`union`/`class`/
/// `interface class`), and do not have premature `C::T` value/variable uses or
/// premature interface class `implements`/`extends` before the full definition.
fn check_forward_typedefs(
    defs: &[&SourceDefinition],
    elab: &ElaboratedModule,
    errs: &mut Vec<String>,
) {
    use std::collections::{HashMap, HashSet};
    use xezim_core::ast::decl::{
        ForwardTypedefKind, FunctionDeclaration, PackageItem, TaskDeclaration,
    };
    use xezim_core::ast::stmt::ForInit;
    use xezim_core::ast::types::StructUnionKind;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum DefKind<'a> {
        Class,
        InterfaceClass,
        Enum,
        Struct,
        Union,
        TypeRef(Option<&'a str>),
        TypeRefOpaque,
        Other,
    }

    fn classify_full_typedef<'a>(td: &'a TypedefDeclaration) -> DefKind<'a> {
        if matches!(
            td.data_type,
            DataType::Interface {
                modport: Some(_),
                ..
            }
        ) || !td.iface_selects.is_empty()
        {
            return DefKind::TypeRefOpaque;
        }
        match &td.data_type {
            DataType::Enum(_) => DefKind::Enum,
            DataType::Struct(su) => {
                if su.kind == StructUnionKind::Union {
                    DefKind::Union
                } else {
                    DefKind::Struct
                }
            }
            DataType::TypeReference { name, .. } => {
                if !name.has_scope() {
                    DefKind::TypeRef(Some(name.name.name.as_str()))
                } else {
                    DefKind::TypeRefOpaque
                }
            }
            _ => DefKind::Other,
        }
    }

    fn resolve_def_kind<'a>(
        kind: DefKind<'a>,
        defs_in_scope: &HashMap<&'a str, (usize, DefKind<'a>)>,
    ) -> DefKind<'a> {
        let mut cur = kind;
        for _ in 0..8 {
            match cur {
                DefKind::TypeRef(Some(target)) => {
                    if let Some((_, next)) = defs_in_scope.get(target) {
                        cur = *next;
                    } else {
                        return DefKind::TypeRefOpaque;
                    }
                }
                other => return other,
            }
        }
        DefKind::TypeRefOpaque
    }

    fn kind_matches(fwd: ForwardTypedefKind, actual: DefKind<'_>) -> bool {
        match fwd {
            ForwardTypedefKind::None | ForwardTypedefKind::Bare => true,
            ForwardTypedefKind::Enum => matches!(actual, DefKind::Enum | DefKind::TypeRefOpaque),
            ForwardTypedefKind::Struct => matches!(actual, DefKind::Struct | DefKind::TypeRefOpaque),
            ForwardTypedefKind::Union => matches!(actual, DefKind::Union | DefKind::TypeRefOpaque),
            ForwardTypedefKind::Class => matches!(actual, DefKind::Class | DefKind::TypeRefOpaque),
            ForwardTypedefKind::InterfaceClass => {
                matches!(actual, DefKind::InterfaceClass | DefKind::TypeRefOpaque)
            }
        }
    }

    fn fwd_kind_label(fwd: ForwardTypedefKind) -> &'static str {
        match fwd {
            ForwardTypedefKind::Enum => "enum",
            ForwardTypedefKind::Struct => "struct",
            ForwardTypedefKind::Union => "union",
            ForwardTypedefKind::Class => "class",
            ForwardTypedefKind::InterfaceClass => "interface class",
            ForwardTypedefKind::Bare | ForwardTypedefKind::None => "typedef",
        }
    }

    fn is_premature<'a>(
        first_fwd: &HashMap<&'a str, (usize, ForwardTypedefKind)>,
        first_def: &HashMap<&'a str, (usize, DefKind<'a>)>,
        name: &str,
        idx: usize,
    ) -> bool {
        first_fwd.get(name).is_some_and(|&(fidx, _)| fidx < idx)
            && first_def.get(name).is_some_and(|&(didx, _)| idx < didx)
    }

    fn check_scoped_dt<'a>(
        dt: &DataType,
        idx: usize,
        first_fwd: &HashMap<&'a str, (usize, ForwardTypedefKind)>,
        first_def: &HashMap<&'a str, (usize, DefKind<'a>)>,
        errs: &mut Vec<String>,
    ) {
        match dt {
            DataType::TypeReference { name, .. } => {
                if let Some(first_scope) = name.scopes.first() {
                    let s = first_scope.name.name.as_str();
                    if is_premature(first_fwd, first_def, s, idx) {
                        errs.push(format!(
                            "cannot select member type '{}::{}' before forward typedef '{s}' is \
                             completed (IEEE 1800-2023 §6.18)",
                            s, name.name.name
                        ));
                    }
                }
            }
            DataType::Struct(su) => {
                for m in &su.members {
                    check_scoped_dt(&m.data_type, idx, first_fwd, first_def, errs);
                }
            }
            DataType::Enum(et) => {
                if let Some(bt) = &et.base_type {
                    check_scoped_dt(bt, idx, first_fwd, first_def, errs);
                }
            }
            _ => {}
        }
    }

    fn check_stmt_dts<'a>(
        st: &Statement,
        idx: usize,
        first_fwd: &HashMap<&'a str, (usize, ForwardTypedefKind)>,
        first_def: &HashMap<&'a str, (usize, DefKind<'a>)>,
        errs: &mut Vec<String>,
    ) {
        for_each_stmt(st, &mut |s| match &s.kind {
            StatementKind::VarDecl { data_type, .. }
            | StatementKind::ParamDecl { data_type, .. } => {
                check_scoped_dt(data_type, idx, first_fwd, first_def, errs);
            }
            StatementKind::For { init, .. } => {
                for i in init {
                    if let ForInit::VarDecl { data_type, .. } = i {
                        check_scoped_dt(data_type, idx, first_fwd, first_def, errs);
                    }
                }
            }
            _ => {}
        });
    }

    fn check_func<'a>(
        f: &FunctionDeclaration,
        idx: usize,
        first_fwd: &HashMap<&'a str, (usize, ForwardTypedefKind)>,
        first_def: &HashMap<&'a str, (usize, DefKind<'a>)>,
        errs: &mut Vec<String>,
    ) {
        check_scoped_dt(&f.return_type, idx, first_fwd, first_def, errs);
        for p in &f.ports {
            check_scoped_dt(&p.data_type, idx, first_fwd, first_def, errs);
        }
        for st in f.items.iter() {
            check_stmt_dts(st, idx, first_fwd, first_def, errs);
        }
    }

    fn check_task<'a>(
        t: &TaskDeclaration,
        idx: usize,
        first_fwd: &HashMap<&'a str, (usize, ForwardTypedefKind)>,
        first_def: &HashMap<&'a str, (usize, DefKind<'a>)>,
        errs: &mut Vec<String>,
    ) {
        for p in &t.ports {
            check_scoped_dt(&p.data_type, idx, first_fwd, first_def, errs);
        }
        for st in t.items.iter() {
            check_stmt_dts(st, idx, first_fwd, first_def, errs);
        }
    }

    fn check_class_item_dts<'a>(
        c: &ClassDeclaration,
        idx: usize,
        first_fwd: &HashMap<&'a str, (usize, ForwardTypedefKind)>,
        first_def: &HashMap<&'a str, (usize, DefKind<'a>)>,
        errs: &mut Vec<String>,
    ) {
        for p in &c.params {
            if let ParameterKind::Data { data_type, .. } = &p.kind {
                check_scoped_dt(data_type, idx, first_fwd, first_def, errs);
            }
        }
        for imp in &c.implements {
            let n = imp.name.as_str();
            if is_premature(first_fwd, first_def, n, idx)
                && (first_fwd[n].1 == ForwardTypedefKind::InterfaceClass
                    || matches!(first_def.get(n), Some((_, DefKind::InterfaceClass))))
            {
                errs.push(format!(
                    "class '{}' cannot implement forward interface class '{n}' before its \
                     definition (IEEE 1800-2023 §8.26.2)",
                    c.name.name
                ));
            }
        }
        if c.is_interface
            && let Some(ext) = &c.extends
            && ext.scopes.is_empty()
        {
            let n = ext.name.name.as_str();
            if is_premature(first_fwd, first_def, n, idx)
                && (first_fwd[n].1 == ForwardTypedefKind::InterfaceClass
                    || matches!(first_def.get(n), Some((_, DefKind::InterfaceClass))))
            {
                errs.push(format!(
                    "interface class '{}' cannot extend forward interface class '{n}' before its \
                     definition (IEEE 1800-2023 §8.26.2)",
                    c.name.name
                ));
            }
        }
        for it in &c.items {
            match it {
                ClassItem::Property(pr) => {
                    check_scoped_dt(&pr.data_type, idx, first_fwd, first_def, errs);
                }
                ClassItem::Parameter(pd) => {
                    if let ParameterKind::Data { data_type, .. } = &pd.kind {
                        check_scoped_dt(data_type, idx, first_fwd, first_def, errs);
                    }
                }
                ClassItem::Method(m) => match &m.kind {
                    ClassMethodKind::Function(f)
                    | ClassMethodKind::PureVirtual(f)
                    | ClassMethodKind::Extern(f) => {
                        check_func(f, idx, first_fwd, first_def, errs);
                    }
                    ClassMethodKind::Task(t) => check_task(t, idx, first_fwd, first_def, errs),
                },
                _ => {}
            }
        }
    }

    enum ScopeEntry<'a> {
        ForwardTypedef(&'a TypedefDeclaration),
        FullTypedef(&'a TypedefDeclaration),
        Class(&'a ClassDeclaration),
        ModuleItem(&'a ModuleItem),
        PackageItem(&'a PackageItem),
        ClassItem(&'a ClassItem),
    }

    fn verify_scope<'a>(entries: Vec<ScopeEntry<'a>>, errs: &mut Vec<String>) {
        let mut first_def: HashMap<&'a str, (usize, DefKind<'a>)> = HashMap::new();
        let mut first_fwd: HashMap<&'a str, (usize, ForwardTypedefKind)> = HashMap::new();
        let mut fwds: Vec<&'a TypedefDeclaration> = Vec::new();
        for (idx, ent) in entries.iter().enumerate() {
            match ent {
                ScopeEntry::ForwardTypedef(td) => {
                    fwds.push(td);
                    first_fwd
                        .entry(td.name.name.as_str())
                        .or_insert((idx, td.forward_kind));
                }
                ScopeEntry::FullTypedef(td) => {
                    first_def
                        .entry(td.name.name.as_str())
                        .or_insert((idx, classify_full_typedef(td)));
                }
                ScopeEntry::Class(c) => {
                    let dk = if c.is_interface {
                        DefKind::InterfaceClass
                    } else {
                        DefKind::Class
                    };
                    first_def.entry(c.name.name.as_str()).or_insert((idx, dk));
                }
                _ => {}
            }
        }
        let mut checked_names: HashSet<&'a str> = HashSet::new();
        for td in fwds {
            let n = td.name.name.as_str();
            match first_def.get(n) {
                None => {
                    if checked_names.insert(n) {
                        errs.push(format!(
                            "forward typedef '{n}' does not resolve to a data type in the same \
                             scope (IEEE 1800-2023 §6.18)"
                        ));
                    }
                }
                Some((_, raw_dk)) => {
                    let resolved = resolve_def_kind(*raw_dk, &first_def);
                    if !kind_matches(td.forward_kind, resolved) {
                        errs.push(format!(
                            "forward '{} {n}' does not match its actual definition (IEEE 1800-2023 \
                             §6.18)",
                            fwd_kind_label(td.forward_kind)
                        ));
                    }
                }
            }
        }
        if first_fwd.is_empty() {
            return;
        }
        for (idx, ent) in entries.iter().enumerate() {
            match ent {
                ScopeEntry::Class(c) => {
                    check_class_item_dts(c, idx, &first_fwd, &first_def, errs);
                }
                ScopeEntry::ModuleItem(mi) => match mi {
                    ModuleItem::DataDeclaration(d) => {
                        check_scoped_dt(&d.data_type, idx, &first_fwd, &first_def, errs);
                    }
                    ModuleItem::NetDeclaration(d) => {
                        check_scoped_dt(&d.data_type, idx, &first_fwd, &first_def, errs);
                    }
                    ModuleItem::PortDeclaration(d) => {
                        check_scoped_dt(&d.data_type, idx, &first_fwd, &first_def, errs);
                    }
                    ModuleItem::ParameterDeclaration(pd) | ModuleItem::LocalparamDeclaration(pd) => {
                        if let ParameterKind::Data { data_type, .. } = &pd.kind {
                            check_scoped_dt(data_type, idx, &first_fwd, &first_def, errs);
                        }
                    }
                    ModuleItem::FunctionDeclaration(f) => {
                        check_func(f, idx, &first_fwd, &first_def, errs);
                    }
                    ModuleItem::TaskDeclaration(t) => {
                        check_task(t, idx, &first_fwd, &first_def, errs);
                    }
                    ModuleItem::InitialConstruct(i) => {
                        check_stmt_dts(&i.stmt, idx, &first_fwd, &first_def, errs);
                    }
                    ModuleItem::AlwaysConstruct(a) => {
                        check_stmt_dts(&a.stmt, idx, &first_fwd, &first_def, errs);
                    }
                    ModuleItem::FinalConstruct(f) => {
                        check_stmt_dts(&f.stmt, idx, &first_fwd, &first_def, errs);
                    }
                    ModuleItem::ClassDeclaration(c) => {
                        check_class_item_dts(c, idx, &first_fwd, &first_def, errs);
                    }
                    _ => {}
                },
                ScopeEntry::PackageItem(pi) => match pi {
                    PackageItem::Data(d) => {
                        check_scoped_dt(&d.data_type, idx, &first_fwd, &first_def, errs);
                    }
                    PackageItem::Net(d) => {
                        check_scoped_dt(&d.data_type, idx, &first_fwd, &first_def, errs);
                    }
                    PackageItem::Parameter(pd) => {
                        if let ParameterKind::Data { data_type, .. } = &pd.kind {
                            check_scoped_dt(data_type, idx, &first_fwd, &first_def, errs);
                        }
                    }
                    PackageItem::Function(f) => {
                        check_func(f, idx, &first_fwd, &first_def, errs);
                    }
                    PackageItem::Task(t) => {
                        check_task(t, idx, &first_fwd, &first_def, errs);
                    }
                    PackageItem::Class(c) => {
                        check_class_item_dts(c, idx, &first_fwd, &first_def, errs);
                    }
                    _ => {}
                },
                ScopeEntry::ClassItem(ci) => match ci {
                    ClassItem::Property(pr) => {
                        check_scoped_dt(&pr.data_type, idx, &first_fwd, &first_def, errs);
                    }
                    ClassItem::Nettype(nt) => {
                        check_scoped_dt(&nt.data_type, idx, &first_fwd, &first_def, errs);
                    }
                    ClassItem::Parameter(pd) => {
                        if let ParameterKind::Data { data_type, .. } = &pd.kind {
                            check_scoped_dt(data_type, idx, &first_fwd, &first_def, errs);
                        }
                    }
                    ClassItem::Method(m) => match &m.kind {
                        ClassMethodKind::Function(f)
                        | ClassMethodKind::PureVirtual(f)
                        | ClassMethodKind::Extern(f) => {
                            check_func(f, idx, &first_fwd, &first_def, errs);
                        }
                        ClassMethodKind::Task(t) => {
                            check_task(t, idx, &first_fwd, &first_def, errs);
                        }
                    },
                    ClassItem::Class(c) => {
                        check_class_item_dts(c, idx, &first_fwd, &first_def, errs);
                    }
                    _ => {}
                },
                ScopeEntry::ForwardTypedef(_) | ScopeEntry::FullTypedef(_) => {}
            }
        }
    }

    fn walk_class<'a>(c: &'a ClassDeclaration, errs: &mut Vec<String>) {
        let mut entries = Vec::with_capacity(c.items.len());
        for it in &c.items {
            match it {
                ClassItem::Typedef(td) if td.forward_kind != ForwardTypedefKind::None => {
                    entries.push(ScopeEntry::ForwardTypedef(td));
                }
                ClassItem::Typedef(td) => entries.push(ScopeEntry::FullTypedef(td)),
                ClassItem::Class(inner) => {
                    walk_class(inner, errs);
                    entries.push(ScopeEntry::Class(inner));
                }
                other => entries.push(ScopeEntry::ClassItem(other)),
            }
        }
        verify_scope(entries, errs);
    }

    fn flatten_generate_regions<'a>(items: &'a [ModuleItem], out: &mut Vec<&'a ModuleItem>) {
        for it in items {
            match it {
                ModuleItem::GenerateRegion(g) => flatten_generate_regions(&g.items, out),
                other => out.push(other),
            }
        }
    }

    fn walk_module_items<'a>(
        items: &'a [ModuleItem],
        non_unit_class_names: &mut HashSet<&'a str>,
        errs: &mut Vec<String>,
    ) {
        let mut flat = Vec::new();
        flatten_generate_regions(items, &mut flat);
        let mut entries = Vec::with_capacity(flat.len());
        for &it in &flat {
            match it {
                ModuleItem::TypedefDeclaration(td)
                    if td.forward_kind != ForwardTypedefKind::None =>
                {
                    entries.push(ScopeEntry::ForwardTypedef(td));
                }
                ModuleItem::TypedefDeclaration(td) => entries.push(ScopeEntry::FullTypedef(td)),
                ModuleItem::ClassDeclaration(c) => {
                    non_unit_class_names.insert(c.name.name.as_str());
                    walk_class(c, errs);
                    entries.push(ScopeEntry::Class(c));
                }
                ModuleItem::GenerateIf(gi) => {
                    for (_, branch) in &gi.branches {
                        walk_module_items(branch, non_unit_class_names, errs);
                    }
                    entries.push(ScopeEntry::ModuleItem(it));
                }
                ModuleItem::GenerateFor(gf) => {
                    walk_module_items(&gf.items, non_unit_class_names, errs);
                    entries.push(ScopeEntry::ModuleItem(it));
                }
                ModuleItem::GenerateCase(gc) => {
                    for arm in &gc.arms {
                        walk_module_items(&arm.items, non_unit_class_names, errs);
                    }
                    entries.push(ScopeEntry::ModuleItem(it));
                }
                other => entries.push(ScopeEntry::ModuleItem(other)),
            }
        }
        verify_scope(entries, errs);
    }

    let mut unit_classes: HashMap<&str, &ClassDeclaration> = HashMap::new();
    let mut non_unit_class_names: HashSet<&str> = HashSet::new();
    for d in defs {
        match d {
            SourceDefinition::Class(c) => {
                unit_classes.insert(c.name.name.as_str(), c);
                walk_class(c, errs);
            }
            SourceDefinition::Package(p) => {
                let mut entries = Vec::with_capacity(p.items.len());
                for it in &p.items {
                    match it {
                        PackageItem::Typedef(td)
                            if td.forward_kind != ForwardTypedefKind::None =>
                        {
                            entries.push(ScopeEntry::ForwardTypedef(td));
                        }
                        PackageItem::Typedef(td) => entries.push(ScopeEntry::FullTypedef(td)),
                        PackageItem::Class(c) => {
                            non_unit_class_names.insert(c.name.name.as_str());
                            walk_class(c, errs);
                            entries.push(ScopeEntry::Class(c));
                        }
                        other => entries.push(ScopeEntry::PackageItem(other)),
                    }
                }
                verify_scope(entries, errs);
            }
            SourceDefinition::Module(m) => {
                walk_module_items(&m.items, &mut non_unit_class_names, errs)
            }
            SourceDefinition::Interface(i) => {
                walk_module_items(&i.items, &mut non_unit_class_names, errs)
            }
            SourceDefinition::Program(p) => {
                walk_module_items(&p.items, &mut non_unit_class_names, errs)
            }
            SourceDefinition::Typedef(_) | SourceDefinition::Udp(_) => {}
        }
    }

    for d in defs {
        if let SourceDefinition::Typedef(td) = d
            && td.forward_kind != ForwardTypedefKind::None
        {
            let n = td.name.name.as_str();
            let unit_def = unit_classes
                .get(n)
                .map(|c| {
                    if c.is_interface {
                        DefKind::InterfaceClass
                    } else {
                        DefKind::Class
                    }
                })
                .or_else(|| {
                    (!non_unit_class_names.contains(n))
                        .then(|| {
                            elab.classes.get(n).and_then(|ec| {
                                (ec.declaring_module.is_none() && ec.enclosing.is_none()).then_some(
                                    if ec.is_interface {
                                        DefKind::InterfaceClass
                                    } else {
                                        DefKind::Class
                                    },
                                )
                            })
                        })
                        .flatten()
                });
            match unit_def {
                None => errs.push(format!(
                    "forward typedef '{n}' does not resolve to a data type in the same scope \
                     (IEEE 1800-2023 §6.18)"
                )),
                Some(dk) => {
                    if !kind_matches(td.forward_kind, dk) {
                        errs.push(format!(
                            "forward '{} {n}' does not match its actual definition (IEEE 1800-2023 \
                             §6.18)",
                            fwd_kind_label(td.forward_kind)
                        ));
                    }
                }
            }
        }
    }
}

/// §8.25.1 & §6.25: outside its own declaration, a parameterized class is named
/// with a parameter value list (`par_cls#()::member`), never bare, across both
/// type references (`PC::T`, `pkg::PC::T`) and scoped expressions (`PC::K`,
/// `PC::T'(x)`, `pkg::PC::K`).
fn check_unspecialized_class_scopes(defs: &[&SourceDefinition], errs: &mut Vec<String>) {
    use std::collections::{HashMap, HashSet};
    use xezim_core::ast::decl::{
        FunctionDeclaration, ImportDeclaration, PackageItem, TaskDeclaration,
    };
    use xezim_core::ast::stmt::ForInit;

    let mut unit_classes: HashMap<&str, &ClassDeclaration> = HashMap::new();
    let mut pkg_classes: HashMap<(&str, &str), &ClassDeclaration> = HashMap::new();
    for d in defs {
        match d {
            SourceDefinition::Class(c) => {
                unit_classes.insert(c.name.name.as_str(), c);
            }
            SourceDefinition::Package(p) => {
                for it in &p.items {
                    if let PackageItem::Class(c) = it {
                        pkg_classes.insert((p.name.name.as_str(), c.name.name.as_str()), c);
                    }
                }
            }
            _ => {}
        }
    }

    struct ScopeEnv<'a> {
        visible: HashMap<&'a str, &'a ClassDeclaration>,
        local_classes: HashSet<&'a str>,
        own: HashSet<&'a str>,
        unit_classes: &'a HashMap<&'a str, &'a ClassDeclaration>,
        pkg_classes: &'a HashMap<(&'a str, &'a str), &'a ClassDeclaration>,
    }

    fn apply_import<'a>(
        imp: &'a ImportDeclaration,
        visible: &mut HashMap<&'a str, &'a ClassDeclaration>,
        pkg_classes: &HashMap<(&'a str, &'a str), &'a ClassDeclaration>,
    ) {
        for item in &imp.items {
            let pkg = item.package.name.as_str();
            match &item.item {
                Some(sym) if sym.name != "*" => {
                    if let Some(&cls) = pkg_classes.get(&(pkg, sym.name.as_str())) {
                        visible.insert(cls.name.name.as_str(), cls);
                    }
                }
                _ => {
                    for (&(p, sym), &cls) in pkg_classes {
                        if p == pkg {
                            visible.insert(sym, cls);
                        }
                    }
                }
            }
        }
    }

    fn collect_stmt_locals<'a>(s: &'a Statement, out: &mut HashSet<&'a str>) {
        match &s.kind {
            StatementKind::VarDecl { declarators, .. }
            | StatementKind::ParamDecl { declarators, .. } => {
                out.extend(declarators.iter().map(|d| d.name.name.as_str()));
            }
            StatementKind::Typedef(td) => {
                out.insert(td.name.name.as_str());
            }
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                collect_stmt_locals(then_stmt, out);
                if let Some(e) = else_stmt {
                    collect_stmt_locals(e, out);
                }
            }
            StatementKind::Case { items, .. } => {
                for it in items {
                    collect_stmt_locals(&it.stmt, out);
                }
            }
            StatementKind::For { init, body, .. } => {
                for i in init {
                    if let ForInit::VarDecl { name, .. } = i {
                        out.insert(name.name.as_str());
                    }
                }
                collect_stmt_locals(body, out);
            }
            StatementKind::Foreach { vars, body, .. } => {
                for v in vars.iter().flatten() {
                    out.insert(v.name.as_str());
                }
                collect_stmt_locals(body, out);
            }
            StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::TimingControl { stmt: body, .. }
            | StatementKind::Wait { stmt: body, .. } => collect_stmt_locals(body, out),
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                for sub in stmts.iter() {
                    collect_stmt_locals(sub, out);
                }
            }
            _ => {}
        }
    }

    fn check_data_type<'a>(
        dt: &DataType,
        env: &ScopeEnv<'a>,
        enclosing: &[&str],
        hits: &mut Vec<String>,
    ) {
        match dt {
            DataType::TypeReference {
                name,
                dimensions,
                type_args,
                ..
            } => {
                if !name.scopes.is_empty() {
                    let s0 = name.scopes[0].name.name.as_str();
                    let mut nested_cur: Option<&ClassDeclaration> = None;
                    let mut start_nested = 0usize;
                    if name.scopes.len() >= 2
                        && !env.own.contains(s0)
                        && !env.local_classes.contains(s0)
                        && let Some(&c) = env
                            .pkg_classes
                            .get(&(s0, name.scopes[1].name.name.as_str()))
                    {
                        let l1 = &name.scopes[1];
                        if !c.params.is_empty()
                            && l1.type_args.is_empty()
                            && !name.bound.has_explicit_empty_spec(1)
                            && !enclosing.contains(&c.name.name.as_str())
                        {
                            hits.push(c.name.name.clone());
                        }
                        nested_cur = Some(c);
                        start_nested = 2;
                    } else if !env.own.contains(s0)
                        && let Some(&c) = env.visible.get(s0)
                    {
                        let l0 = &name.scopes[0];
                        if !c.params.is_empty()
                            && l0.type_args.is_empty()
                            && !name.bound.has_explicit_empty_spec(0)
                            && !enclosing.contains(&c.name.name.as_str())
                        {
                            hits.push(c.name.name.clone());
                        }
                        nested_cur = Some(c);
                        start_nested = 1;
                    }
                    let mut cur = nested_cur;
                    for (idx, link) in name.scopes.iter().enumerate().skip(start_nested) {
                        let Some(outer) = cur else { break };
                        let next = outer.items.iter().find_map(|it| match it {
                            ClassItem::Class(ic) if ic.name.name == link.name.name => Some(&**ic),
                            _ => None,
                        });
                        if let Some(ic) = next {
                            if !ic.params.is_empty()
                                && link.type_args.is_empty()
                                && !name.bound.has_explicit_empty_spec(idx)
                                && !enclosing.contains(&ic.name.name.as_str())
                            {
                                hits.push(ic.name.name.clone());
                            }
                            cur = Some(ic);
                        } else {
                            cur = None;
                        }
                    }
                }
                for s in &name.scopes {
                    for arg in &s.type_args {
                        check_expr(arg, env, enclosing, hits);
                    }
                }
                for d in dimensions {
                    if let PackedDimension::Range { left, right, .. } = d {
                        check_expr(left, env, enclosing, hits);
                        check_expr(right, env, enclosing, hits);
                    }
                }
                for arg in type_args {
                    check_expr(arg, env, enclosing, hits);
                }
            }
            DataType::Struct(su) => {
                for m in &su.members {
                    check_data_type(&m.data_type, env, enclosing, hits);
                    for d in &m.declarators {
                        check_udims(&d.dimensions, env, enclosing, hits);
                    }
                }
            }
            DataType::Enum(et) => {
                if let Some(bt) = &et.base_type {
                    check_data_type(bt, env, enclosing, hits);
                }
            }
            DataType::Interface { type_args, .. } => {
                for arg in type_args {
                    check_expr(arg, env, enclosing, hits);
                }
            }
            DataType::IntegerVector { dimensions, .. }
            | DataType::Implicit { dimensions, .. } => {
                for d in dimensions {
                    if let PackedDimension::Range { left, right, .. } = d {
                        check_expr(left, env, enclosing, hits);
                        check_expr(right, env, enclosing, hits);
                    }
                }
            }
            _ => {}
        }
    }

    fn check_udims<'a>(
        udims: &[UnpackedDimension],
        env: &ScopeEnv<'a>,
        enclosing: &[&str],
        hits: &mut Vec<String>,
    ) {
        for d in udims {
            match d {
                UnpackedDimension::Associative {
                    data_type: Some(dt), ..
                } => {
                    check_data_type(dt, env, enclosing, hits);
                }
                UnpackedDimension::Range { left, right, .. } => {
                    check_expr(left, env, enclosing, hits);
                    check_expr(right, env, enclosing, hits);
                }
                UnpackedDimension::Expression { expr, .. } => {
                    check_expr(expr, env, enclosing, hits);
                }
                UnpackedDimension::Queue {
                    max_size: Some(e), ..
                } => {
                    check_expr(e, env, enclosing, hits);
                }
                _ => {}
            }
        }
    }

    fn check_expr<'a>(
        e: &Expression,
        env: &ScopeEnv<'a>,
        enclosing: &[&str],
        hits: &mut Vec<String>,
    ) {
        check_expr_ctx(e, env, enclosing, false, hits);
    }

    fn check_expr_ctx<'a>(
        e: &Expression,
        env: &ScopeEnv<'a>,
        enclosing: &[&str],
        in_type_ctx: bool,
        hits: &mut Vec<String>,
    ) {
        match &e.kind {
            ExprKind::TypeLiteral(dt) => check_data_type(dt, env, enclosing, hits),
            ExprKind::TypedPattern { ty, pattern } => {
                check_data_type(ty, env, enclosing, hits);
                check_expr(pattern, env, enclosing, hits);
            }
            ExprKind::AssignmentPattern(items) => {
                for it in items {
                    match it {
                        AssignmentPatternItem::Typed(dt, ex) => {
                            check_data_type(dt, env, enclosing, hits);
                            check_expr(ex, env, enclosing, hits);
                        }
                        AssignmentPatternItem::Keyed(k, v) => {
                            check_expr(k, env, enclosing, hits);
                            check_expr(v, env, enclosing, hits);
                        }
                        other => check_expr(other.expr(), env, enclosing, hits),
                    }
                }
            }
            ExprKind::Specialization { base, .. } => {
                // The immediate class reference inside `base` has `#(...)` applied
                // to it; still recurse into deeper prefix qualifiers if any.
                match &base.kind {
                    ExprKind::Ident(_) => {}
                    ExprKind::MemberAccess { expr: inner, .. }
                        if matches!(&inner.kind, ExprKind::Ident(h) if h.path.len() == 1) => {}
                    ExprKind::MemberAccess { expr: inner, .. } => {
                        check_expr(inner, env, enclosing, hits);
                    }
                    _ => check_expr(base, env, enclosing, hits),
                }
            }
            ExprKind::MemberAccess { expr, member } => {
                if member.name == "new" {
                    return;
                }
                match &expr.kind {
                    ExprKind::Ident(h)
                        if h.root.is_none()
                            && h.path.len() == 1
                            && h.path[0].selects.is_empty() =>
                    {
                        let n = h.path[0].name.name.as_str();
                        if env.visible.get(n).is_some_and(|c| !c.params.is_empty())
                            && !env.own.contains(n)
                            && !enclosing.contains(&n)
                        {
                            hits.push(n.to_string());
                        }
                    }
                    ExprKind::MemberAccess {
                        expr: inner,
                        member: cls_id,
                    } if matches!(
                        &inner.kind,
                        ExprKind::Ident(h)
                            if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty()
                    ) =>
                    {
                        let ExprKind::Ident(h) = &inner.kind else {
                            unreachable!();
                        };
                        let pkg = h.path[0].name.name.as_str();
                        let cls = cls_id.name.as_str();
                        if !env.own.contains(pkg)
                            && !env.local_classes.contains(pkg)
                            && let Some(&c) = env.pkg_classes.get(&(pkg, cls))
                        {
                            if in_type_ctx && !c.params.is_empty() && !enclosing.contains(&cls) {
                                hits.push(cls.to_string());
                            }
                        } else if !env.own.contains(pkg)
                            && let Some(&outer) = env.visible.get(pkg)
                        {
                            if !outer.params.is_empty() && !enclosing.contains(&pkg) {
                                hits.push(pkg.to_string());
                            }
                            if outer.items.iter().any(|it| {
                                matches!(
                                    it,
                                    ClassItem::Class(ic)
                                        if ic.name.name == cls && !ic.params.is_empty()
                                )
                            }) && !enclosing.contains(&cls)
                            {
                                hits.push(cls.to_string());
                            }
                        } else {
                            check_expr(expr, env, enclosing, hits);
                        }
                    }
                    _ => check_expr(expr, env, enclosing, hits),
                }
            }
            ExprKind::Ident(h)
                if h.path.len() >= 2
                    && h.path.iter().all(|s| s.selects.is_empty())
                    && !h.path.last().is_some_and(|s| s.name.name == "new") =>
            {
                let first = h.path[0].name.name.as_str();
                if matches!(h.root, Some(xezim_core::ast::expr::HierRoot::Unit)) {
                    if env.unit_classes.get(first).is_some_and(|c| !c.params.is_empty())
                        && !enclosing.contains(&first)
                    {
                        hits.push(first.to_string());
                    }
                } else if h.root.is_none() && !env.own.contains(first) {
                    let second = h.path[1].name.name.as_str();
                    if !env.local_classes.contains(first)
                        && let Some(&c) = env.pkg_classes.get(&(first, second))
                    {
                        if in_type_ctx && !c.params.is_empty() && !enclosing.contains(&second) {
                            hits.push(second.to_string());
                        }
                    } else if env.visible.get(first).is_some_and(|c| !c.params.is_empty())
                        && !enclosing.contains(&first)
                    {
                        hits.push(first.to_string());
                    }
                }
            }
            ExprKind::Unary { operand, .. } => check_expr(operand, env, enclosing, hits),
            ExprKind::Binary { left, right, .. } => {
                check_expr(left, env, enclosing, hits);
                check_expr(right, env, enclosing, hits);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                check_expr(condition, env, enclosing, hits);
                check_expr(then_expr, env, enclosing, hits);
                check_expr(else_expr, env, enclosing, hits);
            }
            ExprKind::Concatenation(xs) => {
                xs.iter().for_each(|x| check_expr(x, env, enclosing, hits));
            }
            ExprKind::Replication { count, exprs } => {
                check_expr(count, env, enclosing, hits);
                exprs
                    .iter()
                    .for_each(|x| check_expr(x, env, enclosing, hits));
            }
            ExprKind::Call { func, args } => {
                check_expr(func, env, enclosing, hits);
                args.iter().for_each(|x| check_expr(x, env, enclosing, hits));
            }
            ExprKind::SystemCall { name, args } => {
                for (idx, x) in args.iter().enumerate() {
                    let is_cast_target = idx == 0 && name == "$__xz_named_cast";
                    check_expr_ctx(x, env, enclosing, is_cast_target, hits);
                }
            }
            ExprKind::NamedArg { expr: Some(ex), .. } => check_expr(ex, env, enclosing, hits),
            ExprKind::Inside { expr, ranges } => {
                check_expr(expr, env, enclosing, hits);
                ranges
                    .iter()
                    .for_each(|x| check_expr(x, env, enclosing, hits));
            }
            ExprKind::Index { expr, index } => {
                check_expr(expr, env, enclosing, hits);
                check_expr(index, env, enclosing, hits);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                check_expr(expr, env, enclosing, hits);
                check_expr(left, env, enclosing, hits);
                check_expr(right, env, enclosing, hits);
            }
            ExprKind::Range(a, b) => {
                check_expr(a, env, enclosing, hits);
                check_expr(b, env, enclosing, hits);
            }
            ExprKind::Paren(x) | ExprKind::ShallowCopy { source: x } => {
                check_expr(x, env, enclosing, hits);
            }
            ExprKind::AssignExpr { lvalue, rvalue } => {
                check_expr(lvalue, env, enclosing, hits);
                check_expr(rvalue, env, enclosing, hits);
            }
            ExprKind::WithClause { expr, filter } => {
                check_expr(expr, env, enclosing, hits);
                check_expr(filter, env, enclosing, hits);
            }
            ExprKind::RandomizeWith { call, .. } => check_expr(call, env, enclosing, hits),
            ExprKind::StreamOp {
                slice_size, exprs, ..
            } => {
                if let Some(s) = slice_size {
                    check_expr(s, env, enclosing, hits);
                }
                exprs
                    .iter()
                    .for_each(|x| check_expr(x, env, enclosing, hits));
            }
            ExprKind::Tagged { inner: Some(ex), .. } => check_expr(ex, env, enclosing, hits),
            ExprKind::Matches { expr, .. } | ExprKind::SvaClocked { body: expr, .. } => {
                check_expr(expr, env, enclosing, hits);
            }
            _ => {}
        }
    }

    fn check_stmt<'a>(
        stmt: &'a Statement,
        env: &ScopeEnv<'a>,
        enclosing: &[&str],
        hits: &mut Vec<String>,
    ) {
        for_each_stmt(stmt, &mut |s| match &s.kind {
            StatementKind::Expr(e) => check_expr(e, env, enclosing, hits),
            StatementKind::BlockingAssign { lvalue, rvalue } => {
                check_expr(lvalue, env, enclosing, hits);
                check_expr(rvalue, env, enclosing, hits);
            }
            StatementKind::NonblockingAssign {
                lvalue,
                rvalue,
                delay,
            } => {
                check_expr(lvalue, env, enclosing, hits);
                check_expr(rvalue, env, enclosing, hits);
                if let Some(d) = delay {
                    check_expr(d, env, enclosing, hits);
                }
            }
            StatementKind::If { condition, .. }
            | StatementKind::While { condition, .. }
            | StatementKind::DoWhile { condition, .. } => {
                check_expr(condition, env, enclosing, hits);
            }
            StatementKind::Case { expr, .. } => check_expr(expr, env, enclosing, hits),
            StatementKind::For {
                init,
                condition,
                step,
                ..
            } => {
                for i in init {
                    match i {
                        ForInit::VarDecl {
                            data_type, init: e, ..
                        } => {
                            check_data_type(data_type, env, enclosing, hits);
                            check_expr(e, env, enclosing, hits);
                        }
                        ForInit::Assign { lvalue, rvalue } => {
                            check_expr(lvalue, env, enclosing, hits);
                            check_expr(rvalue, env, enclosing, hits);
                        }
                    }
                }
                if let Some(c) = condition {
                    check_expr(c, env, enclosing, hits);
                }
                for st in step {
                    check_expr(st, env, enclosing, hits);
                }
            }
            StatementKind::Foreach { array, .. } => check_expr(array, env, enclosing, hits),
            StatementKind::Repeat { count, .. } => check_expr(count, env, enclosing, hits),
            StatementKind::Wait { condition, .. } => check_expr(condition, env, enclosing, hits),
            StatementKind::Return(Some(e)) => check_expr(e, env, enclosing, hits),
            StatementKind::VarDecl {
                data_type,
                declarators,
                ..
            }
            | StatementKind::ParamDecl {
                data_type,
                declarators,
                ..
            } => {
                check_data_type(data_type, env, enclosing, hits);
                for d in declarators {
                    check_udims(&d.dimensions, env, enclosing, hits);
                    if let Some(init) = &d.init {
                        check_expr(init, env, enclosing, hits);
                    }
                }
            }
            StatementKind::Typedef(td) => {
                check_data_type(&td.data_type, env, enclosing, hits);
                check_udims(&td.dimensions, env, enclosing, hits);
            }
            StatementKind::TimingControl { control, .. } => {
                if let xezim_core::ast::stmt::TimingControl::Delay(e) = control {
                    check_expr(e, env, enclosing, hits);
                }
            }
            _ => {}
        });
    }

    fn check_param_decl<'a>(
        pd: &'a ParameterDeclaration,
        env: &ScopeEnv<'a>,
        enclosing: &[&str],
        hits: &mut Vec<String>,
    ) {
        match &pd.kind {
            ParameterKind::Data {
                data_type,
                assignments,
            } => {
                check_data_type(data_type, env, enclosing, hits);
                for a in assignments {
                    check_udims(&a.dimensions, env, enclosing, hits);
                    if let Some(init) = &a.init {
                        check_expr(init, env, enclosing, hits);
                    }
                }
            }
            ParameterKind::Type { assignments } => {
                for a in assignments {
                    if let Some(dt) = &a.init {
                        check_data_type(dt, env, enclosing, hits);
                    }
                }
            }
        }
    }

    fn check_func<'a>(
        f: &'a FunctionDeclaration,
        env: &ScopeEnv<'a>,
        enclosing: &[&str],
        hits: &mut Vec<String>,
    ) {
        let out_of_block = f.name.scopes.last().map(|s| s.name.name.as_str());
        let mut enc_buf: Vec<&str> = enclosing.to_vec();
        let mut local_own = env.own.clone();
        if let Some(ooc) = out_of_block {
            enc_buf.push(ooc);
            if let Some(&cls) = env.visible.get(ooc) {
                for it in &cls.items {
                    match it {
                        ClassItem::Property(pr) => {
                            local_own.extend(pr.declarators.iter().map(|d| d.name.name.as_str()));
                        }
                        ClassItem::Typedef(td) => {
                            local_own.insert(td.name.name.as_str());
                        }
                        ClassItem::Nettype(nt) => {
                            local_own.insert(nt.name.name.as_str());
                        }
                        ClassItem::Parameter(pd) => {
                            if let ParameterKind::Data { assignments, .. } = &pd.kind {
                                local_own.extend(assignments.iter().map(|a| a.name.name.as_str()));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        for p in &f.ports {
            local_own.insert(p.name.name.as_str());
        }
        for st in f.items.iter() {
            collect_stmt_locals(st, &mut local_own);
        }
        let sub_env = ScopeEnv {
            visible: env.visible.clone(),
            local_classes: env.local_classes.clone(),
            own: local_own,
            unit_classes: env.unit_classes,
            pkg_classes: env.pkg_classes,
        };
        check_data_type(&f.return_type, &sub_env, &enc_buf, hits);
        for p in &f.ports {
            check_data_type(&p.data_type, &sub_env, &enc_buf, hits);
            check_udims(&p.dimensions, &sub_env, &enc_buf, hits);
            if let Some(def) = &p.default {
                check_expr(def, &sub_env, &enc_buf, hits);
            }
        }
        for st in f.items.iter() {
            check_stmt(st, &sub_env, &enc_buf, hits);
        }
    }

    fn check_task<'a>(
        t: &'a TaskDeclaration,
        env: &ScopeEnv<'a>,
        enclosing: &[&str],
        hits: &mut Vec<String>,
    ) {
        let out_of_block = t.name.scopes.last().map(|s| s.name.name.as_str());
        let mut enc_buf: Vec<&str> = enclosing.to_vec();
        let mut local_own = env.own.clone();
        if let Some(ooc) = out_of_block {
            enc_buf.push(ooc);
            if let Some(&cls) = env.visible.get(ooc) {
                for it in &cls.items {
                    if let ClassItem::Property(pr) = it {
                        local_own.extend(pr.declarators.iter().map(|d| d.name.name.as_str()));
                    }
                }
            }
        }
        for p in &t.ports {
            local_own.insert(p.name.name.as_str());
        }
        for st in t.items.iter() {
            collect_stmt_locals(st, &mut local_own);
        }
        let sub_env = ScopeEnv {
            visible: env.visible.clone(),
            local_classes: env.local_classes.clone(),
            own: local_own,
            unit_classes: env.unit_classes,
            pkg_classes: env.pkg_classes,
        };
        for p in &t.ports {
            check_data_type(&p.data_type, &sub_env, &enc_buf, hits);
            check_udims(&p.dimensions, &sub_env, &enc_buf, hits);
            if let Some(def) = &p.default {
                check_expr(def, &sub_env, &enc_buf, hits);
            }
        }
        for st in t.items.iter() {
            check_stmt(st, &sub_env, &enc_buf, hits);
        }
    }

    fn check_class<'a>(
        c: &'a ClassDeclaration,
        env: &ScopeEnv<'a>,
        enclosing: &[&str],
        hits: &mut Vec<String>,
    ) {
        let mut enc_buf: Vec<&str> = enclosing.to_vec();
        enc_buf.push(c.name.name.as_str());
        let mut cls_visible = env.visible.clone();
        let mut cls_local = env.local_classes.clone();
        let mut cls_own = env.own.clone();
        for p in &c.params {
            match &p.kind {
                ParameterKind::Data { assignments, .. } => {
                    cls_own.extend(assignments.iter().map(|a| a.name.name.as_str()));
                }
                ParameterKind::Type { assignments } => {
                    cls_own.extend(assignments.iter().map(|a| a.name.name.as_str()));
                }
            }
        }
        for it in &c.items {
            match it {
                ClassItem::Import(imp) => apply_import(imp, &mut cls_visible, env.pkg_classes),
                ClassItem::Class(ic) => {
                    cls_visible.insert(ic.name.name.as_str(), ic);
                    cls_local.insert(ic.name.name.as_str());
                    cls_own.remove(ic.name.name.as_str());
                }
                ClassItem::Property(pr) => {
                    cls_own.extend(pr.declarators.iter().map(|d| d.name.name.as_str()));
                }
                ClassItem::Typedef(td) => {
                    cls_own.insert(td.name.name.as_str());
                }
                ClassItem::Nettype(nt) => {
                    cls_own.insert(nt.name.name.as_str());
                }
                ClassItem::Parameter(pd) => {
                    if let ParameterKind::Data { assignments, .. } = &pd.kind {
                        cls_own.extend(assignments.iter().map(|a| a.name.name.as_str()));
                    } else if let ParameterKind::Type { assignments } = &pd.kind {
                        cls_own.extend(assignments.iter().map(|a| a.name.name.as_str()));
                    }
                }
                _ => {}
            }
        }
        let cls_env = ScopeEnv {
            visible: cls_visible,
            local_classes: cls_local,
            own: cls_own,
            unit_classes: env.unit_classes,
            pkg_classes: env.pkg_classes,
        };
        for p in &c.params {
            check_param_decl(p, &cls_env, &enc_buf, hits);
        }
        if let Some(ext) = &c.extends {
            for arg in &ext.args {
                match arg {
                    xezim_core::ast::decl::ParamValue::Expr(ex) => {
                        check_expr(ex, &cls_env, &enc_buf, hits);
                    }
                    xezim_core::ast::decl::ParamValue::Type(dt) => {
                        check_data_type(dt, &cls_env, &enc_buf, hits);
                    }
                }
            }
        }
        for it in &c.items {
            match it {
                ClassItem::Property(pr) => {
                    check_data_type(&pr.data_type, &cls_env, &enc_buf, hits);
                    for d in &pr.declarators {
                        check_udims(&d.dimensions, &cls_env, &enc_buf, hits);
                        if let Some(init) = &d.init {
                            check_expr(init, &cls_env, &enc_buf, hits);
                        }
                    }
                }
                ClassItem::Typedef(td) => {
                    check_data_type(&td.data_type, &cls_env, &enc_buf, hits);
                    check_udims(&td.dimensions, &cls_env, &enc_buf, hits);
                }
                ClassItem::Nettype(nt) => {
                    check_data_type(&nt.data_type, &cls_env, &enc_buf, hits);
                }
                ClassItem::Parameter(pd) => check_param_decl(pd, &cls_env, &enc_buf, hits),
                ClassItem::Method(m) => match &m.kind {
                    ClassMethodKind::Function(f)
                    | ClassMethodKind::PureVirtual(f)
                    | ClassMethodKind::Extern(f) => {
                        check_func(f, &cls_env, &enc_buf, hits);
                    }
                    ClassMethodKind::Task(t) => check_task(t, &cls_env, &enc_buf, hits),
                },
                ClassItem::Class(ic) => check_class(ic, &cls_env, &enc_buf, hits),
                _ => {}
            }
        }
    }

    fn check_module_items<'a>(
        ports: &'a PortList,
        params: &'a [ParameterDeclaration],
        items: &'a [ModuleItem],
        parent_env: &ScopeEnv<'a>,
        hits: &mut Vec<String>,
    ) {
        let mut visible = parent_env.visible.clone();
        let mut local_classes = parent_env.local_classes.clone();
        let mut own = parent_env.own.clone();
        for o in module_own_names(ports, params, items) {
            own.insert(o);
        }
        for it in items {
            match it {
                ModuleItem::ImportDeclaration(imp) => {
                    apply_import(imp, &mut visible, parent_env.pkg_classes);
                }
                ModuleItem::ClassDeclaration(c) => {
                    visible.insert(c.name.name.as_str(), c);
                    local_classes.insert(c.name.name.as_str());
                    own.remove(c.name.name.as_str());
                }
                ModuleItem::GenerateRegion(g) => {
                    let mut q: Vec<&ModuleItem> = g.items.iter().collect();
                    let mut idx = 0;
                    while idx < q.len() {
                        match q[idx] {
                            ModuleItem::GenerateRegion(inner) => {
                                q.extend(inner.items.iter());
                            }
                            ModuleItem::ImportDeclaration(imp) => {
                                apply_import(imp, &mut visible, parent_env.pkg_classes);
                            }
                            ModuleItem::ClassDeclaration(c) => {
                                visible.insert(c.name.name.as_str(), c);
                                local_classes.insert(c.name.name.as_str());
                                own.remove(c.name.name.as_str());
                            }
                            _ => {}
                        }
                        idx += 1;
                    }
                }
                _ => {}
            }
        }
        let env = ScopeEnv {
            visible,
            local_classes,
            own,
            unit_classes: parent_env.unit_classes,
            pkg_classes: parent_env.pkg_classes,
        };
        if let PortList::Ansi(ps) = ports {
            for p in ps {
                if let Some(dt) = &p.data_type {
                    check_data_type(dt, &env, &[], hits);
                }
                check_udims(&p.dimensions, &env, &[], hits);
                if let Some(def) = &p.default {
                    check_expr(def, &env, &[], hits);
                }
            }
        }
        for p in params {
            check_param_decl(p, &env, &[], hits);
        }
        for it in items {
            match it {
                ModuleItem::DataDeclaration(d) => {
                    check_data_type(&d.data_type, &env, &[], hits);
                    for v in &d.declarators {
                        check_udims(&v.dimensions, &env, &[], hits);
                        if let Some(init) = &v.init {
                            check_expr(init, &env, &[], hits);
                        }
                    }
                }
                ModuleItem::NetDeclaration(d) => {
                    check_data_type(&d.data_type, &env, &[], hits);
                    for v in &d.declarators {
                        check_udims(&v.dimensions, &env, &[], hits);
                        if let Some(init) = &v.init {
                            check_expr(init, &env, &[], hits);
                        }
                    }
                }
                ModuleItem::PortDeclaration(d) => {
                    check_data_type(&d.data_type, &env, &[], hits);
                    for v in &d.declarators {
                        check_udims(&v.dimensions, &env, &[], hits);
                        if let Some(init) = &v.init {
                            check_expr(init, &env, &[], hits);
                        }
                    }
                }
                ModuleItem::ParameterDeclaration(pd) | ModuleItem::LocalparamDeclaration(pd) => {
                    check_param_decl(pd, &env, &[], hits);
                }
                ModuleItem::TypedefDeclaration(td) => {
                    check_data_type(&td.data_type, &env, &[], hits);
                    check_udims(&td.dimensions, &env, &[], hits);
                }
                ModuleItem::ContinuousAssign(ca) => {
                    for (l, r) in &ca.assignments {
                        check_expr(l, &env, &[], hits);
                        check_expr(r, &env, &[], hits);
                    }
                }
                ModuleItem::InitialConstruct(i) => check_stmt(&i.stmt, &env, &[], hits),
                ModuleItem::AlwaysConstruct(a) => check_stmt(&a.stmt, &env, &[], hits),
                ModuleItem::FinalConstruct(f) => check_stmt(&f.stmt, &env, &[], hits),
                ModuleItem::FunctionDeclaration(f) => check_func(f, &env, &[], hits),
                ModuleItem::TaskDeclaration(t) => check_task(t, &env, &[], hits),
                ModuleItem::ClassDeclaration(c) => check_class(c, &env, &[], hits),
                ModuleItem::GenerateRegion(g) => {
                    check_module_items(&PortList::Empty, &[], &g.items, &env, hits);
                }
                ModuleItem::GenerateIf(gi) => {
                    for (_, branch) in &gi.branches {
                        check_module_items(&PortList::Empty, &[], branch, &env, hits);
                    }
                }
                ModuleItem::GenerateFor(gf) => {
                    check_module_items(&PortList::Empty, &[], &gf.items, &env, hits);
                }
                ModuleItem::GenerateCase(gc) => {
                    for arm in &gc.arms {
                        check_module_items(&PortList::Empty, &[], &arm.items, &env, hits);
                    }
                }
                _ => {}
            }
        }
    }

    let unit_env = ScopeEnv {
        visible: unit_classes.clone(),
        local_classes: HashSet::new(),
        own: HashSet::new(),
        unit_classes: &unit_classes,
        pkg_classes: &pkg_classes,
    };

    let mut hits: Vec<String> = Vec::new();
    for d in defs {
        match d {
            SourceDefinition::Class(c) => check_class(c, &unit_env, &[], &mut hits),
            SourceDefinition::Typedef(td) => {
                check_data_type(&td.data_type, &unit_env, &[], &mut hits);
                check_udims(&td.dimensions, &unit_env, &[], &mut hits);
            }
            SourceDefinition::Module(m) => {
                check_module_items(&m.ports, &m.params, &m.items, &unit_env, &mut hits);
            }
            SourceDefinition::Interface(i) => {
                check_module_items(&i.ports, &i.params, &i.items, &unit_env, &mut hits);
            }
            SourceDefinition::Program(p) => {
                check_module_items(&p.ports, &p.params, &p.items, &unit_env, &mut hits);
            }
            SourceDefinition::Package(p) => {
                let mut visible = unit_classes.clone();
                let mut local_classes = HashSet::new();
                for it in &p.items {
                    match it {
                        PackageItem::Import(imp) => {
                            apply_import(imp, &mut visible, &pkg_classes);
                        }
                        PackageItem::Class(c) => {
                            visible.insert(c.name.name.as_str(), c);
                            local_classes.insert(c.name.name.as_str());
                        }
                        _ => {}
                    }
                }
                let pkg_env = ScopeEnv {
                    visible,
                    local_classes,
                    own: HashSet::new(),
                    unit_classes: &unit_classes,
                    pkg_classes: &pkg_classes,
                };
                for it in &p.items {
                    match it {
                        PackageItem::Parameter(pd) => {
                            check_param_decl(pd, &pkg_env, &[], &mut hits);
                        }
                        PackageItem::Typedef(td) => {
                            check_data_type(&td.data_type, &pkg_env, &[], &mut hits);
                            check_udims(&td.dimensions, &pkg_env, &[], &mut hits);
                        }
                        PackageItem::Data(d) => {
                            check_data_type(&d.data_type, &pkg_env, &[], &mut hits);
                            for v in &d.declarators {
                                check_udims(&v.dimensions, &pkg_env, &[], &mut hits);
                                if let Some(init) = &v.init {
                                    check_expr(init, &pkg_env, &[], &mut hits);
                                }
                            }
                        }
                        PackageItem::Net(d) => {
                            check_data_type(&d.data_type, &pkg_env, &[], &mut hits);
                            for v in &d.declarators {
                                check_udims(&v.dimensions, &pkg_env, &[], &mut hits);
                                if let Some(init) = &v.init {
                                    check_expr(init, &pkg_env, &[], &mut hits);
                                }
                            }
                        }
                        PackageItem::Function(f) => check_func(f, &pkg_env, &[], &mut hits),
                        PackageItem::Task(t) => check_task(t, &pkg_env, &[], &mut hits),
                        PackageItem::Class(c) => check_class(c, &pkg_env, &[], &mut hits),
                        _ => {}
                    }
                }
            }
            SourceDefinition::Udp(_) => {}
        }
    }

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
    udp_names: &HashSet<&str>,
    items: &[ModuleItem],
    errs: &mut Vec<String>,
) {
    for it in items {
        let ModuleItem::ModuleInstantiation(mi) = it else {
            continue;
        };
        let is_udp = udp_names.contains(mi.module_name.name.as_str());
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
                StatementKind::VarDecl { declarators, .. }
                | StatementKind::ParamDecl { declarators, .. } => {
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
                if let StatementKind::VarDecl { declarators, .. }
                | StatementKind::ParamDecl { declarators, .. } = &s.kind
                {
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
fn check_event_arguments(items: &[ModuleItem], elab: &ElaboratedModule, errs: &mut Vec<String>) {
    let mut local_tds: HashMap<&str, &DataType> = HashMap::new();
    for it in items {
        if let ModuleItem::TypedefDeclaration(td) = it {
            local_tds.insert(td.name.name.as_str(), &td.data_type);
        }
    }
    fn is_event_dt<'a>(
        mut dt: &'a DataType,
        local_tds: &HashMap<&str, &'a DataType>,
        elab: &'a ElaboratedModule,
    ) -> bool {
        for _ in 0..16 {
            match dt {
                DataType::Simple {
                    kind: xezim_core::ast::types::SimpleType::Event,
                    ..
                } => return true,
                DataType::TypeReference { name, .. } => {
                    let next = if !name.scopes.is_empty() {
                        let qual = format!(
                            "{}::{}",
                            name.scopes
                                .iter()
                                .map(|s| s.name.name.as_str())
                                .collect::<Vec<_>>()
                                .join("::"),
                            name.name.name
                        );
                        elab.typedef_types
                            .get(&qual)
                            .or_else(|| elab.typedef_types.get(&name.name.name))
                    } else {
                        local_tds
                            .get(name.name.name.as_str())
                            .copied()
                            .or_else(|| elab.typedef_types.get(&name.name.name))
                    };
                    match next {
                        Some(resolved) => dt = resolved,
                        None => return false,
                    }
                }
                _ => return false,
            }
        }
        false
    }
    let mut events: HashSet<&str> = HashSet::new();
    for it in items {
        if let ModuleItem::DataDeclaration(d) = it
            && is_event_dt(&d.data_type, &local_tds, elab)
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
                    && !is_event_dt(&p.data_type, &local_tds, elab)
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
    if !is_top
        || !items
            .iter()
            .any(|it| matches!(it, ModuleItem::ContinuousAssign(_)))
    {
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
                        xezim_core::ast::decl::PortConnection::NamedIdent { actual, .. } => {
                            implied.push(actual.name.clone());
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

/// §6.21: an automatic variable may not be referenced by `$monitor` (or
/// `$monitorb/h/o`), which keeps reading its arguments after the process
/// that declared them is gone. Loop variables declared in a `for` header
/// or named by `foreach` are automatic (§12.7.1, §12.7.3), as are variables
/// declared `automatic`, and the ports and locals of an automatic task or
/// function and of every class method. The monitor printed x for them.
fn check_monitor_automatic(items: &[ModuleItem], errs: &mut Vec<String>) {
    for it in items {
        match it {
            ModuleItem::InitialConstruct(i) => monitor_walk(&i.stmt, &mut vec![], false, errs),
            ModuleItem::AlwaysConstruct(a) => monitor_walk(&a.stmt, &mut vec![], false, errs),
            ModuleItem::TaskDeclaration(t) => {
                let auto = matches!(
                    t.lifetime,
                    Some(xezim_core::ast::types::Lifetime::Automatic)
                );
                monitor_subroutine(&t.ports, &t.items, auto, errs);
            }
            ModuleItem::FunctionDeclaration(f) => {
                let auto = matches!(
                    f.lifetime,
                    Some(xezim_core::ast::types::Lifetime::Automatic)
                );
                monitor_subroutine(&f.ports, &f.items, auto, errs);
            }
            ModuleItem::ClassDeclaration(c) => monitor_class(c, errs),
            _ => {}
        }
    }
}

fn monitor_class(c: &ClassDeclaration, errs: &mut Vec<String>) {
    for ci in &c.items {
        match ci {
            ClassItem::Method(m) => match &m.kind {
                ClassMethodKind::Function(f) => monitor_subroutine(&f.ports, &f.items, true, errs),
                ClassMethodKind::Task(t) => monitor_subroutine(&t.ports, &t.items, true, errs),
                _ => {}
            },
            ClassItem::Class(inner) => monitor_class(inner, errs),
            _ => {}
        }
    }
}

fn monitor_subroutine(
    ports: &[xezim_core::ast::decl::FunctionPort],
    body: &[Statement],
    auto: bool,
    errs: &mut Vec<String>,
) {
    let mut scopes: Vec<Vec<(String, bool)>> =
        vec![ports.iter().map(|p| (p.name.name.clone(), auto)).collect()];
    for s in body {
        monitor_stmt(s, &mut scopes, auto, errs);
    }
}

fn monitor_walk(
    stmt: &Statement,
    scopes: &mut Vec<Vec<(String, bool)>>,
    auto: bool,
    errs: &mut Vec<String>,
) {
    scopes.push(Vec::new());
    monitor_stmt(stmt, scopes, auto, errs);
    scopes.pop();
}

/// Declares the statement's variables in the innermost scope and checks any
/// `$monitor` it makes. `auto` is the default lifetime of the enclosing
/// subroutine (block variables of a static context are static).
fn monitor_stmt(
    stmt: &Statement,
    scopes: &mut Vec<Vec<(String, bool)>>,
    auto: bool,
    errs: &mut Vec<String>,
) {
    use xezim_core::ast::stmt::ForInit;
    use xezim_core::ast::types::Lifetime;
    match &stmt.kind {
        StatementKind::VarDecl {
            lifetime,
            declarators,
            ..
        } => {
            let is_auto = match lifetime {
                Some(Lifetime::Automatic) => true,
                Some(Lifetime::Static) => false,
                None => auto,
            };
            if let Some(top) = scopes.last_mut() {
                for d in declarators {
                    top.push((d.name.name.clone(), is_auto));
                }
            }
        }
        StatementKind::ParamDecl { declarators, .. } => {
            if let Some(top) = scopes.last_mut() {
                for d in declarators {
                    top.push((d.name.name.clone(), false));
                }
            }
        }
        StatementKind::Expr(e) => {
            if let ExprKind::SystemCall { name, args } = &e.kind
                && matches!(
                    name.as_str(),
                    "$monitor" | "$monitorb" | "$monitorh" | "$monitoro"
                )
            {
                let mut seen: Vec<String> = Vec::new();
                for a in args {
                    let mut ids = Vec::new();
                    monitor_idents(a, &mut ids);
                    for id in ids {
                        let automatic = scopes
                            .iter()
                            .rev()
                            .find_map(|s| s.iter().rev().find(|(n, _)| *n == id).map(|(_, a)| *a))
                            .unwrap_or(false);
                        if automatic && !seen.contains(&id) {
                            errs.push(format!(
                                "automatic variable '{}' is not allowed as an argument to {} \
                                 (LRM 1800-2017 §6.21)",
                                id, name
                            ));
                            seen.push(id);
                        }
                    }
                }
            }
        }
        StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
            scopes.push(Vec::new());
            for s in stmts {
                monitor_stmt(s, scopes, auto, errs);
            }
            scopes.pop();
        }
        StatementKind::For { init, body, .. } => {
            let vars: Vec<(String, bool)> = init
                .iter()
                .filter_map(|i| match i {
                    ForInit::VarDecl { name, .. } => Some((name.name.clone(), true)),
                    ForInit::Assign { .. } => None,
                })
                .collect();
            scopes.push(vars);
            monitor_walk(body, scopes, auto, errs);
            scopes.pop();
        }
        StatementKind::Foreach { vars, body, .. } => {
            scopes.push(
                vars.iter()
                    .flatten()
                    .map(|v| (v.name.clone(), true))
                    .collect(),
            );
            monitor_walk(body, scopes, auto, errs);
            scopes.pop();
        }
        StatementKind::If {
            then_stmt,
            else_stmt,
            ..
        } => {
            monitor_walk(then_stmt, scopes, auto, errs);
            if let Some(e) = else_stmt {
                monitor_walk(e, scopes, auto, errs);
            }
        }
        StatementKind::Case { items, .. } => {
            for it in items {
                monitor_walk(&it.stmt, scopes, auto, errs);
            }
        }
        StatementKind::While { body, .. }
        | StatementKind::DoWhile { body, .. }
        | StatementKind::Repeat { body, .. }
        | StatementKind::Forever { body }
        | StatementKind::TimingControl { stmt: body, .. }
        | StatementKind::Wait { stmt: body, .. } => monitor_walk(body, scopes, auto, errs),
        _ => {}
    }
}

/// §10.9.1: an ordered assignment pattern for an unpacked array has exactly
/// one item per element of the dimension it fills (`'{1,2,3}` for a
/// `[1:4]` row is illegal). Checked for module-scope arrays whose bounds are
/// literals (no parameter can change them), assigned whole or by row;
/// patterns with `default:`, keys, member names or replication are not
/// counted. A name declared again in a procedural block is skipped.
fn check_unpacked_pattern_counts(items: &[ModuleItem], errs: &mut Vec<String>) {
    let mut arrays: std::collections::HashMap<String, Vec<(i64, i64)>> =
        std::collections::HashMap::new();
    let mut shadowed: HashSet<String> = HashSet::default();
    for it in items {
        if let ModuleItem::DataDeclaration(d) = it {
            for decl in &d.declarators {
                if let Some(dims) = literal_unpacked_dims(&decl.dimensions) {
                    if arrays
                        .insert(decl.name.name.clone(), dims.clone())
                        .is_some()
                    {
                        shadowed.insert(decl.name.name.clone());
                    }
                    // 1-D flat initializers are `check_array_flat_init`'s.
                    if dims.len() > 1
                        && let Some(init) = &decl.init
                        && let ExprKind::AssignmentPattern(pi) = &init.peel_typed_pattern().kind
                    {
                        pattern_count(&decl.name.name, pi, &dims, errs);
                    }
                }
            }
        }
    }
    if arrays.is_empty() {
        return;
    }
    let procs: Vec<&Statement> = items
        .iter()
        .filter_map(|it| match it {
            ModuleItem::InitialConstruct(i) => Some(&i.stmt),
            ModuleItem::AlwaysConstruct(a) => Some(&a.stmt),
            _ => None,
        })
        .collect();
    for s in &procs {
        for_each_stmt(s, &mut |st| {
            if let StatementKind::VarDecl { declarators, .. }
            | StatementKind::ParamDecl { declarators, .. } = &st.kind
            {
                for d in declarators {
                    shadowed.insert(d.name.name.clone());
                }
            }
            if let StatementKind::For { init, .. } = &st.kind {
                for i in init {
                    if let xezim_core::ast::stmt::ForInit::VarDecl { name, .. } = i {
                        shadowed.insert(name.name.clone());
                    }
                }
            }
        });
    }
    for s in &procs {
        for_each_assign_pair(s, &mut |lhs, rhs| {
            let ExprKind::AssignmentPattern(pi) = &rhs.peel_typed_pattern().kind else {
                return;
            };
            let mut base = lhs;
            let mut k = 0usize;
            while let ExprKind::Index { expr, index } = &base.kind {
                if matches!(
                    index.kind,
                    ExprKind::Range(..) | ExprKind::RangeSelect { .. }
                ) {
                    return;
                }
                k += 1;
                base = expr;
            }
            let ExprKind::Ident(h) = &base.kind else {
                return;
            };
            if h.root.is_some() || h.path.len() != 1 {
                return;
            }
            k += h.path[0].selects.len();
            let name = &h.path[0].name.name;
            if shadowed.contains(name) {
                return;
            }
            if let Some(dims) = arrays.get(name)
                && k < dims.len()
            {
                pattern_count(name, pi, &dims[k..], errs);
            }
        });
    }
}

fn literal_unpacked_dims(dims: &[UnpackedDimension]) -> Option<Vec<(i64, i64)>> {
    if dims.is_empty() {
        return None;
    }
    dims.iter()
        .map(|d| match d {
            UnpackedDimension::Range { left, right, .. } => {
                Some((literal_int(left)?, literal_int(right)?))
            }
            UnpackedDimension::Expression { expr, .. } => {
                let n = literal_int(expr)?;
                (n > 0).then_some((0, n - 1))
            }
            _ => None,
        })
        .collect()
}

fn literal_int(e: &Expression) -> Option<i64> {
    match &e.kind {
        ExprKind::Number(_) => xezim_core::elaborate::const_eval_i64_with_params(e, None),
        _ => None,
    }
}

fn pattern_count(
    name: &str,
    items: &[AssignmentPatternItem],
    dims: &[(i64, i64)],
    errs: &mut Vec<String>,
) {
    let Some(&(l, r)) = dims.first() else {
        return;
    };
    let mut ordered: Vec<&Expression> = Vec::with_capacity(items.len());
    for it in items {
        match it {
            AssignmentPatternItem::Ordered(e)
                if !matches!(e.kind, ExprKind::Replication { .. }) =>
            {
                ordered.push(e)
            }
            _ => return,
        }
    }
    let n = (l - r).abs() + 1;
    if ordered.len() as i64 != n {
        errs.push(format!(
            "assignment pattern for '{}' has {} element(s), but the dimension [{}:{}] has {} \
             (LRM 1800-2017 §10.9.1)",
            name,
            ordered.len(),
            l,
            r,
            n
        ));
        return;
    }
    if dims.len() > 1 {
        for e in ordered {
            if let ExprKind::AssignmentPattern(inner) = &e.peel_typed_pattern().kind {
                pattern_count(name, inner, &dims[1..], errs);
            }
        }
    }
}

/// The leading names an expression reads (`a` of `a[i].f`, and `i`),
/// including names inside index selects.
fn monitor_idents(e: &Expression, out: &mut Vec<String>) {
    for_each_expr(e, &mut |x| {
        if let ExprKind::Ident(h) = &x.kind {
            if h.root.is_none()
                && let Some(first) = h.path.first()
            {
                out.push(first.name.name.clone());
            }
            for seg in &h.path {
                for sel in &seg.selects {
                    monitor_idents(sel, out);
                }
            }
        }
    });
}

// ---------------------------------------------------------------------------
// §6.18, §8.3, §8.13 — a declaration or `extends` clause naming a type that
// is declared nowhere.
// ---------------------------------------------------------------------------
//
// `addr_constrained_txn txn = new();` with no such class in the compilation
// elaborated as an untyped handle: `randomize()` did nothing, every member
// read `x`, and the run exited 0. An undefined base class was dropped the same
// way. A type identifier shall be declared before it is used (§6.18), and a
// class handle or base class shall name a class (§8.3, §8.13).
//
// As with the #888 finder above, the pass does not resolve scopes: a name is
// reported only when NOTHING in the design declares it (no class, typedef,
// type parameter, covergroup, nettype, interface, package member, ...). A
// type that exists but is not visible from the use is left to elaboration.
// Scoped names (`pkg::T`, `C::T`) are left alone too.

/// Walks every definition twice: once harvesting the type-like names the #888
/// harvest does not reach (`collecting`), once checking the type positions.
struct UndeclaredTypeFinder<'a> {
    elab: &'a ElaboratedModule,
    pkg_names: &'a HashSet<String>,
    declared: HashSet<String>,
    collecting: bool,
    /// The definition being walked, to locate a span in the right file.
    owner: String,
    seen: HashSet<String>,
    errs: Vec<String>,
}

impl<'a> UndeclaredTypeFinder<'a> {
    fn known(&self, n: &str) -> bool {
        !is_plain_ident(n)
            || is_builtin_type(n)
            || reserved_type_arg_name(n)
            || self.pkg_names.contains(n)
            || self.declared.contains(n)
            || top_declares(n, self.elab)
            || self.elab.covergroups.contains_key(n)
            || self.elab.checker_decls.contains_key(n)
            || self.elab.nettype_resolvers.contains_key(n)
            || self.elab.forward_typedef_names.contains(n)
    }

    fn report(&mut self, n: &str, span: xezim_core::ast::Span, message: String) {
        if self.collecting || self.known(n) || !self.seen.insert(message.clone()) {
            return;
        }
        self.errs.push(xezim_core::elaborate::span_error_of(
            self.elab,
            span,
            &self.owner,
            &message,
        ));
    }

    /// The type of the declaration of `declared_name`.
    fn data_type(&mut self, dt: &DataType, declared_name: &str) {
        match dt {
            DataType::TypeReference { name, .. } if !name.has_scope() => {
                let n = name.name.name.clone();
                let message = format!(
                    "'{n}' is not a declared type or class: it is used as the type of \
                     '{declared_name}' but nothing in the design declares it \
                     (IEEE 1800-2017 §6.18, §8.3)"
                );
                self.report(&n, name.span, message);
            }
            DataType::Struct(su) => {
                for m in &su.members {
                    let member = m
                        .declarators
                        .first()
                        .map(|d| d.name.name.clone())
                        .unwrap_or_default();
                    self.data_type(&m.data_type, &member);
                }
            }
            _ => {}
        }
    }

    fn declarators(&mut self, dt: &DataType, ds: &[VarDeclarator]) {
        let first = ds.first().map(|d| d.name.name.clone()).unwrap_or_default();
        self.data_type(dt, &first);
    }

    fn typedef_name(&mut self, td: &TypedefDeclaration) {
        if self.collecting {
            self.declared.insert(td.name.name.clone());
        }
    }

    fn stmt(&mut self, s: &Statement) {
        use xezim_core::ast::stmt::ForInit;
        match &s.kind {
            StatementKind::VarDecl {
                data_type,
                declarators,
                ..
            } => self.declarators(data_type, declarators),
            StatementKind::Typedef(td) => self.typedef_name(td),
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                self.stmt(then_stmt);
                if let Some(e) = else_stmt {
                    self.stmt(e);
                }
            }
            StatementKind::Case { items, .. } => {
                for it in items {
                    self.stmt(&it.stmt);
                }
            }
            StatementKind::For { init, body, .. } => {
                for i in init {
                    if let ForInit::VarDecl {
                        data_type, name, ..
                    } = i
                    {
                        self.data_type(data_type, &name.name);
                    }
                }
                self.stmt(body);
            }
            StatementKind::ForeachTail(ft) => self.stmt(&ft.body),
            StatementKind::Foreach { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::ForeverTail { body }
            | StatementKind::RsAction { body }
            | StatementKind::TimingControl { stmt: body, .. }
            | StatementKind::Wait { stmt: body, .. } => self.stmt(body),
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                for st in stmts {
                    self.stmt(st);
                }
            }
            StatementKind::RandCase { items } => {
                for (_, st) in items {
                    self.stmt(st);
                }
            }
            StatementKind::WaitOrder { pass, fail, .. } => {
                for st in [pass, fail].into_iter().flatten() {
                    self.stmt(st);
                }
            }
            _ => {}
        }
    }

    fn ports(&mut self, ports: &[xezim_core::ast::decl::FunctionPort]) {
        for p in ports {
            self.data_type(&p.data_type, &p.name.name);
        }
    }

    fn function(&mut self, f: &xezim_core::ast::decl::FunctionDeclaration) {
        self.data_type(&f.return_type, &f.name.name.name);
        self.ports(&f.ports);
        for st in f.items.iter() {
            self.stmt(st);
        }
    }

    fn task(&mut self, t: &xezim_core::ast::decl::TaskDeclaration) {
        self.ports(&t.ports);
        for st in t.items.iter() {
            self.stmt(st);
        }
    }

    fn covergroup(&mut self, cg: &xezim_core::ast::decl::CovergroupDeclaration) {
        if self.collecting {
            self.declared.insert(cg.name.name.clone());
        }
    }

    fn class(&mut self, c: &ClassDeclaration) {
        if self.collecting {
            self.declared.insert(c.name.name.clone());
        }
        if let Some(x) = &c.extends {
            if x.scopes.is_empty() {
                let n = x.name.name.clone();
                let message = format!(
                    "class '{}' extends '{n}', but nothing in the design declares a class \
                     '{n}' (IEEE 1800-2017 §8.13)",
                    c.name.name
                );
                self.report(&n, x.span, message);
            }
        }
        for it in &c.items {
            match it {
                ClassItem::Property(pr) => self.declarators(&pr.data_type, &pr.declarators),
                ClassItem::Method(m) => match &m.kind {
                    ClassMethodKind::Function(f)
                    | ClassMethodKind::PureVirtual(f)
                    | ClassMethodKind::Extern(f) => self.function(f),
                    ClassMethodKind::Task(t) => self.task(t),
                },
                ClassItem::Typedef(td) => self.typedef_name(td),
                ClassItem::Nettype(nt) => {
                    if self.collecting {
                        self.declared.insert(nt.name.name.clone());
                    } else {
                        self.data_type(&nt.data_type, &nt.name.name);
                    }
                }
                ClassItem::Class(inner) => self.class(inner),
                ClassItem::Covergroup(cg) => self.covergroup(cg),
                _ => {}
            }
        }
    }

    fn items(&mut self, items: &[ModuleItem]) {
        for it in items {
            match it {
                ModuleItem::DataDeclaration(d) => self.declarators(&d.data_type, &d.declarators),
                ModuleItem::TypedefDeclaration(td) => self.typedef_name(td),
                ModuleItem::FunctionDeclaration(f) => self.function(f),
                ModuleItem::TaskDeclaration(t) => self.task(t),
                ModuleItem::ClassDeclaration(c) => self.class(c),
                ModuleItem::InitialConstruct(p) => self.stmt(&p.stmt),
                ModuleItem::AlwaysConstruct(p) => self.stmt(&p.stmt),
                ModuleItem::FinalConstruct(p) => self.stmt(&p.stmt),
                ModuleItem::CovergroupDeclaration(cg) => self.covergroup(cg),
                ModuleItem::NettypeDeclaration(nt) => {
                    if self.collecting {
                        self.declared.insert(nt.name.name.clone());
                    }
                }
                ModuleItem::CheckerDeclaration(ck) => {
                    if self.collecting {
                        self.declared.insert(ck.name.name.clone());
                    }
                    self.items(&ck.items);
                }
                ModuleItem::GenerateRegion(g) => self.items(&g.items),
                ModuleItem::GenerateIf(g) => {
                    for (_, its) in &g.branches {
                        self.items(its);
                    }
                }
                ModuleItem::GenerateFor(g) => self.items(&g.items),
                ModuleItem::GenerateCase(g) => {
                    for arm in &g.arms {
                        if self.collecting {
                            note_items_names(&arm.items, &mut self.declared);
                        }
                        self.items(&arm.items);
                    }
                }
                _ => {}
            }
        }
    }

    fn definitions(&mut self, defs: &[&SourceDefinition]) {
        use xezim_core::ast::decl::PackageItem;
        for d in defs {
            self.owner = d.name();
            if self.collecting {
                self.declared.insert(d.name());
            }
            match d {
                SourceDefinition::Module(m) => self.items(&m.items),
                SourceDefinition::Interface(m) => self.items(&m.items),
                SourceDefinition::Program(m) => self.items(&m.items),
                SourceDefinition::Package(p) => {
                    for it in &p.items {
                        match it {
                            PackageItem::Data(dd) => {
                                self.declarators(&dd.data_type, &dd.declarators)
                            }
                            PackageItem::Typedef(td) => self.typedef_name(td),
                            PackageItem::Function(f) => self.function(f),
                            PackageItem::Task(t) => self.task(t),
                            PackageItem::Class(c) => self.class(c),
                            PackageItem::Nettype(nt) if self.collecting => {
                                self.declared.insert(nt.name.name.clone());
                            }
                            PackageItem::Checker(ck) if self.collecting => {
                                self.declared.insert(ck.name.name.clone());
                            }
                            _ => {}
                        }
                    }
                }
                SourceDefinition::Class(c) => self.class(c),
                SourceDefinition::Typedef(_) | SourceDefinition::Udp(_) => {}
            }
        }
    }
}

/// §6.18, §8.3, §8.13: report every unscoped type name, in a variable,
/// property, subroutine-port or return-type declaration or in an `extends`
/// clause, that nothing in the design declares.
fn check_undeclared_type_names(
    defs: &[&SourceDefinition],
    elab: &ElaboratedModule,
    pkg_names: &HashSet<String>,
    declared: HashSet<String>,
    errs: &mut Vec<String>,
) {
    let mut f = UndeclaredTypeFinder {
        elab,
        pkg_names,
        declared,
        collecting: true,
        owner: String::new(),
        seen: HashSet::new(),
        errs: Vec::new(),
    };
    f.definitions(defs);
    f.collecting = false;
    f.definitions(defs);
    // `defs` iterates a hash map; keep the report order stable.
    f.errs.sort();
    errs.extend(f.errs);
}

/// §6.6.8 / §10.3: an `interconnect` net can only participate in port connections
/// and `alias` declarations; continuous assignments and procedural references
/// (aside from array/type query system functions such as `$bits`) are illegal.
fn check_interconnect_usage(u: &Unit<'_>, elab: &ElaboratedModule, errs: &mut Vec<String>) {
    use xezim_core::ast::stmt::{EventControl, ForInit, Pattern, ProceduralContinuous, TimingControl};
    use xezim_core::ast::types::NetType;

    fn collect_ic_nets<'a>(items: &'a [ModuleItem], out: &mut HashSet<&'a str>) {
        for it in items {
            match it {
                ModuleItem::NetDeclaration(n) if n.net_type == NetType::Interconnect => {
                    out.extend(n.declarators.iter().map(|d| d.name.name.as_str()));
                }
                ModuleItem::PortDeclaration(p) if p.net_type == Some(NetType::Interconnect) => {
                    out.extend(p.declarators.iter().map(|d| d.name.name.as_str()));
                }
                ModuleItem::GenerateRegion(g) => collect_ic_nets(&g.items, out),
                _ => {}
            }
        }
    }

    let mut ic_nets: HashSet<&str> = HashSet::new();
    if let PortList::Ansi(ps) = u.ports {
        for p in ps {
            if p.net_type == Some(NetType::Interconnect) {
                ic_nets.insert(p.name.name.as_str());
            }
        }
    }
    collect_ic_nets(u.items, &mut ic_nets);
    if ic_nets.is_empty() {
        return;
    }

    struct Ctx<'a, 'b> {
        ic_nets: &'a HashSet<&'a str>,
        shadows: Vec<HashSet<&'a str>>,
        elab: &'a ElaboratedModule,
        errs: &'b mut Vec<String>,
        owner: &'a str,
    }

    impl<'a, 'b> Ctx<'a, 'b> {
        fn is_shadowed(&self, name: &str) -> bool {
            self.shadows.iter().any(|s| s.contains(name))
        }

        fn push_scope(&mut self) {
            self.shadows.push(HashSet::new());
        }

        fn pop_scope(&mut self) {
            self.shadows.pop();
        }

        fn bind_name(&mut self, name: &'a str) {
            if let Some(top) = self.shadows.last_mut() {
                top.insert(name);
            }
        }

        fn bind_pattern(&mut self, pat: &'a Pattern) {
            match pat {
                Pattern::Wildcard => {}
                Pattern::Binding(id) => self.bind_name(id.name.as_str()),
                Pattern::Tagged { inner: Some(p), .. } => self.bind_pattern(p),
                Pattern::Tagged { inner: None, .. } => {}
                Pattern::Expr(e) => self.expr(e),
                Pattern::Struct(fields) => {
                    for (_, p) in fields {
                        self.bind_pattern(p);
                    }
                }
            }
        }

        fn check_pattern_exprs_and_bind(&mut self, e: &'a Expression) {
            if let ExprKind::Matches { expr, pattern } = &e.kind {
                self.expr(expr);
                self.bind_pattern(pattern);
            } else {
                self.expr(e);
            }
        }

        fn report(&mut self, name: &str, span: xezim_core::ast::Span, ctx: &str) {
            let at = xezim_core::elaborate::span_location_of(self.elab, span, self.owner)
                .map(|a| format!(" (at {a})"))
                .unwrap_or_default();
            self.errs.push(format!(
                "interconnect net '{name}' cannot be used in {ctx} (IEEE 1800-2017 §6.6.8){at}"
            ));
        }

        fn expr(&mut self, e: &'a Expression) {
            self.expr_in(e, "procedural expression");
        }

        fn expr_in(&mut self, e: &'a Expression, ctx: &str) {
            match &e.kind {
                ExprKind::Number(_)
                | ExprKind::StringLiteral(_)
                | ExprKind::TypeLiteral(_)
                | ExprKind::Dollar
                | ExprKind::Null
                | ExprKind::This
                | ExprKind::Empty => {}
                ExprKind::Ident(h) => {
                    if h.root.is_none()
                        && let Some(first) = h.path.first()
                    {
                        let n = first.name.name.as_str();
                        if self.ic_nets.contains(n) && !self.is_shadowed(n) {
                            self.report(n, h.span, ctx);
                        }
                    }
                    for seg in &h.path {
                        for sel in &seg.selects {
                            self.expr_in(sel, ctx);
                        }
                    }
                }
                ExprKind::Unary { operand, .. }
                | ExprKind::Paren(operand)
                | ExprKind::MemberAccess { expr: operand, .. }
                | ExprKind::ShallowCopy { source: operand } => self.expr_in(operand, ctx),
                ExprKind::Binary { left, right, .. }
                | ExprKind::Range(left, right)
                | ExprKind::WithClause {
                    expr: left,
                    filter: right,
                } => {
                    self.expr_in(left, ctx);
                    self.expr_in(right, ctx);
                }
                ExprKind::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                } => {
                    self.expr_in(condition, ctx);
                    self.expr_in(then_expr, ctx);
                    self.expr_in(else_expr, ctx);
                }
                ExprKind::Concatenation(xs) => {
                    for x in xs {
                        self.expr_in(x, ctx);
                    }
                }
                ExprKind::Replication { count, exprs } => {
                    self.expr_in(count, ctx);
                    for x in exprs {
                        self.expr_in(x, ctx);
                    }
                }
                ExprKind::AssignmentPattern(items) => {
                    for it in items {
                        match it {
                            AssignmentPatternItem::Keyed(k, v) => {
                                self.expr_in(k, ctx);
                                self.expr_in(v, ctx);
                            }
                            other => self.expr_in(other.expr(), ctx),
                        }
                    }
                }
                ExprKind::TypedPattern { pattern, .. } => self.expr_in(pattern, ctx),
                ExprKind::Call { func, args } => {
                    self.expr_in(func, ctx);
                    for a in args {
                        self.expr_in(a, ctx);
                    }
                }
                ExprKind::SystemCall { name, args } => {
                    if !matches!(
                        name.as_str(),
                        "$bits"
                            | "$size"
                            | "$left"
                            | "$right"
                            | "$low"
                            | "$high"
                            | "$increment"
                            | "$dimensions"
                            | "$unpacked_dimensions"
                    ) {
                        for a in args {
                            self.expr_in(a, ctx);
                        }
                    }
                }
                ExprKind::NamedArg { expr: Some(inner), .. } => self.expr_in(inner, ctx),
                ExprKind::NamedArg { expr: None, .. } => {}
                ExprKind::Inside { expr, ranges } => {
                    self.expr_in(expr, ctx);
                    for r in ranges {
                        self.expr_in(r, ctx);
                    }
                }
                ExprKind::Matches { expr, pattern } => {
                    self.expr_in(expr, ctx);
                    self.visit_pattern_exprs(pattern, ctx);
                }
                ExprKind::Specialization { base, .. } => self.expr_in(base, ctx),
                ExprKind::Index { expr, index } => {
                    self.expr_in(expr, ctx);
                    self.expr_in(index, ctx);
                }
                ExprKind::RangeSelect {
                    expr, left, right, ..
                } => {
                    self.expr_in(expr, ctx);
                    self.expr_in(left, ctx);
                    self.expr_in(right, ctx);
                }
                ExprKind::RandomizeWith { call, .. } => self.expr_in(call, ctx),
                ExprKind::AssignExpr { lvalue, rvalue } => {
                    self.expr_in(lvalue, ctx);
                    self.expr_in(rvalue, ctx);
                }
                ExprKind::StreamOp {
                    slice_size, exprs, ..
                } => {
                    if let Some(s) = slice_size {
                        self.expr_in(s, ctx);
                    }
                    for x in exprs {
                        self.expr_in(x, ctx);
                    }
                }
                ExprKind::Tagged { inner: Some(x), .. } => self.expr_in(x, ctx),
                ExprKind::Tagged { inner: None, .. } => {}
                ExprKind::SvaClocked { events, body } => {
                    for ev in events {
                        self.expr_in(&ev.expr, ctx);
                        if let Some(iff) = &ev.iff {
                            self.expr_in(iff, ctx);
                        }
                    }
                    self.expr_in(body, ctx);
                }
            }
        }

        fn visit_pattern_exprs(&mut self, pat: &'a Pattern, ctx: &str) {
            match pat {
                Pattern::Wildcard | Pattern::Binding(_) | Pattern::Tagged { inner: None, .. } => {}
                Pattern::Tagged { inner: Some(p), .. } => self.visit_pattern_exprs(p, ctx),
                Pattern::Expr(e) => self.expr_in(e, ctx),
                Pattern::Struct(fields) => {
                    for (_, p) in fields {
                        self.visit_pattern_exprs(p, ctx);
                    }
                }
            }
        }

        fn timing(&mut self, tc: &'a TimingControl) {
            match tc {
                TimingControl::Delay(e) => self.expr(e),
                TimingControl::Event(ec) => match ec {
                    EventControl::Star | EventControl::ParenStar => {}
                    EventControl::Identifier(id) => {
                        let n = id.name.as_str();
                        if self.ic_nets.contains(n) && !self.is_shadowed(n) {
                            self.report(n, id.span, "event control");
                        }
                    }
                    EventControl::HierIdentifier(e) => self.expr(e),
                    EventControl::EventExpr(evs) => {
                        for ev in evs {
                            self.expr(&ev.expr);
                            if let Some(iff) = &ev.iff {
                                self.expr(iff);
                            }
                        }
                    }
                },
            }
        }

        fn stmt(&mut self, s: &'a Statement) {
            match &s.kind {
                StatementKind::Null
                | StatementKind::ScopePop
                | StatementKind::LoopStep
                | StatementKind::DisableFork
                | StatementKind::WaitFork
                | StatementKind::Break
                | StatementKind::Continue
                | StatementKind::RsReturn
                | StatementKind::Typedef(_)
                | StatementKind::Disable(_) => {}
                StatementKind::Expr(e) | StatementKind::Coverpoint { expr: e, .. } => self.expr(e),
                StatementKind::BlockingAssign { lvalue, rvalue } => {
                    self.expr(lvalue);
                    self.expr(rvalue);
                }
                StatementKind::NonblockingAssign {
                    lvalue,
                    delay,
                    rvalue,
                } => {
                    self.expr(lvalue);
                    if let Some(d) = delay {
                        self.expr(d);
                    }
                    self.expr(rvalue);
                }
                StatementKind::If {
                    condition,
                    then_stmt,
                    else_stmt,
                    ..
                } => {
                    self.push_scope();
                    self.check_pattern_exprs_and_bind(condition);
                    self.stmt(then_stmt);
                    self.pop_scope();
                    if let Some(el) = else_stmt {
                        self.stmt(el);
                    }
                }
                StatementKind::Case { expr, items, .. } => {
                    self.expr(expr);
                    for it in items {
                        for p in &it.patterns {
                            self.expr(p);
                        }
                        self.push_scope();
                        if let Some(pat) = &it.pattern {
                            self.bind_pattern(pat);
                        }
                        if let Some(g) = &it.guard {
                            self.expr(g);
                        }
                        self.stmt(&it.stmt);
                        self.pop_scope();
                    }
                }
                StatementKind::For {
                    init,
                    condition,
                    step,
                    body,
                } => {
                    self.push_scope();
                    for fi in init {
                        match fi {
                            ForInit::VarDecl { name, init, .. } => {
                                self.expr(init);
                                self.bind_name(name.name.as_str());
                            }
                            ForInit::Assign { lvalue, rvalue } => {
                                self.expr(lvalue);
                                self.expr(rvalue);
                            }
                        }
                    }
                    if let Some(c) = condition {
                        self.expr(c);
                    }
                    for st in step {
                        self.expr(st);
                    }
                    self.stmt(body);
                    self.pop_scope();
                }
                StatementKind::Foreach { array, vars, body } => {
                    self.expr(array);
                    self.push_scope();
                    for v in vars.iter().flatten() {
                        self.bind_name(v.name.as_str());
                    }
                    self.stmt(body);
                    self.pop_scope();
                }
                StatementKind::ForeachTail(ft) => self.stmt(&ft.body),
                StatementKind::While { condition, body }
                | StatementKind::DoWhile { body, condition }
                | StatementKind::Wait {
                    condition,
                    stmt: body,
                } => {
                    self.expr(condition);
                    self.stmt(body);
                }
                StatementKind::Repeat { count, body } => {
                    self.expr(count);
                    self.stmt(body);
                }
                StatementKind::Forever { body }
                | StatementKind::ForeverTail { body }
                | StatementKind::RsAction { body } => self.stmt(body),
                StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                    self.push_scope();
                    for st in stmts {
                        self.stmt(st);
                    }
                    self.pop_scope();
                }
                StatementKind::TimingControl { control, stmt } => {
                    self.timing(control);
                    self.stmt(stmt);
                }
                StatementKind::EventTrigger { target, .. } => {
                    if let Some(t) = target {
                        self.expr(t);
                    }
                }
                StatementKind::Return(Some(e)) => self.expr(e),
                StatementKind::Return(None) => {}
                StatementKind::Assertion(a) => {
                    self.expr(&a.expr);
                    if let Some(ic) = &a.inferred_clock {
                        self.expr(&ic.expr);
                        if let Some(iff) = &ic.iff {
                            self.expr(iff);
                        }
                    }
                    if let Some(ac) = &a.action {
                        self.stmt(ac);
                    }
                    if let Some(el) = &a.else_action {
                        self.stmt(el);
                    }
                }
                StatementKind::ProceduralContinuous(pc) => match pc {
                    ProceduralContinuous::Assign { lvalue, rvalue }
                    | ProceduralContinuous::Force { lvalue, rvalue } => {
                        self.expr(lvalue);
                        self.expr(rvalue);
                    }
                    ProceduralContinuous::Deassign(lvalue)
                    | ProceduralContinuous::Release(lvalue) => self.expr(lvalue),
                },
                StatementKind::VarDecl { declarators, .. }
                | StatementKind::ParamDecl { declarators, .. } => {
                    for d in declarators {
                        if let Some(init) = &d.init {
                            self.expr(init);
                        }
                        self.bind_name(d.name.name.as_str());
                    }
                }
                StatementKind::RandCase { items } => {
                    for (w, st) in items {
                        self.expr(w);
                        self.stmt(st);
                    }
                }
                StatementKind::Cross { items, .. } => {
                    for it in items {
                        self.expr(it);
                    }
                }
                StatementKind::WaitOrder { pass, fail, .. } => {
                    if let Some(p) = pass {
                        self.stmt(p);
                    }
                    if let Some(f) = fail {
                        self.stmt(f);
                    }
                }
            }
        }

        fn walk_items(&mut self, items: &'a [ModuleItem]) {
            for it in items {
                match it {
                    ModuleItem::ContinuousAssign(ca) => {
                        for (l, r) in &ca.assignments {
                            self.expr_in(l, "continuous assignment");
                            self.expr_in(r, "continuous assignment");
                        }
                    }
                    ModuleItem::InitialConstruct(p) => self.stmt(&p.stmt),
                    ModuleItem::AlwaysConstruct(p) => self.stmt(&p.stmt),
                    ModuleItem::FinalConstruct(p) => self.stmt(&p.stmt),
                    ModuleItem::FunctionDeclaration(f) => {
                        self.push_scope();
                        self.bind_name(f.name.name.name.as_str());
                        for p in &f.ports {
                            if let Some(def) = &p.default {
                                self.expr(def);
                            }
                            self.bind_name(p.name.name.as_str());
                        }
                        for st in f.items.iter() {
                            self.stmt(st);
                        }
                        self.pop_scope();
                    }
                    ModuleItem::TaskDeclaration(t) => {
                        self.push_scope();
                        for p in &t.ports {
                            if let Some(def) = &p.default {
                                self.expr(def);
                            }
                            self.bind_name(p.name.name.as_str());
                        }
                        for st in t.items.iter() {
                            self.stmt(st);
                        }
                        self.pop_scope();
                    }
                    ModuleItem::GenerateRegion(g) => self.walk_items(&g.items),
                    ModuleItem::GenerateIf(g) => {
                        for (_, its) in &g.branches {
                            self.walk_items(its);
                        }
                    }
                    ModuleItem::GenerateFor(g) => self.walk_items(&g.items),
                    ModuleItem::GenerateCase(g) => {
                        for arm in &g.arms {
                            self.walk_items(&arm.items);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let mut ctx = Ctx {
        ic_nets: &ic_nets,
        shadows: Vec::new(),
        elab,
        errs,
        owner: u.name,
    };
    ctx.walk_items(u.items);
}

#[cfg(test)]
mod instance_view_tests {
    use super::*;

    /// §26.3 (#721): the package constants are held once for the whole lint
    /// run, not once per instance. Each instance's table holds its own
    /// parameters; the scope it folds in reads both.
    #[test]
    fn instances_share_one_table_of_package_constants() {
        let mut src = String::from("package cfg;\n");
        for i in 0..40 {
            src.push_str(&format!("  parameter int K{i} = {i};\n"));
        }
        src.push_str(
            "endpackage\n\
             module leaf #(parameter int N = 1) (); localparam int M = N + cfg::K7; endmodule\n\
             module top;\n",
        );
        for i in 0..25 {
            src.push_str(&format!("  leaf #(.N({i})) u{i} ();\n"));
        }
        src.push_str("endmodule\n");
        let (_, elab) = xezim_core::parse_and_elaborate_multi(
            &[src],
            Some("top"),
            &[],
            &["views.sv".to_string()],
            &[],
        )
        .expect("elaborate");
        let views = InstanceViews::build(&elab);
        assert_eq!(views.scoped.len(), 40);
        assert!(views.scoped.keys().all(|k| k.starts_with("cfg::K")));
        let leaves = views.of("leaf");
        assert_eq!(leaves.len(), 25);
        // 25 instances x 40 constants were 1,000 copied entries.
        let own: usize = leaves.iter().map(|view| view.len()).sum();
        assert_eq!(own, 25 * 2, "N and M of each instance");
        assert!(
            leaves
                .iter()
                .all(|view| view.keys().all(|k| !k.contains("::")))
        );
        // A scope reads its own parameter and the shared constant alike.
        let mut seen = Vec::new();
        for view in leaves {
            let scope = ConstScope::instance(&elab, view, &views.scoped);
            let n = scope.param("N").and_then(|v| v.to_i64()).expect("own N");
            let m = scope.param("M").and_then(|v| v.to_i64()).expect("own M");
            assert_eq!(m, n + 7);
            let k = scope.param("cfg::K7").expect("shared constant");
            assert!(std::ptr::eq(k, &views.scoped["cfg::K7"]));
            seen.push(n);
        }
        seen.sort_unstable();
        assert_eq!(seen, (0..25).collect::<Vec<i64>>());
    }

    #[test]
    fn identical_param_instances_deduplicate_views() {
        let src = String::from(
            "module leaf #(parameter int N = 1) (); localparam int M = N * 2; endmodule\n\
             module bare (); endmodule\n\
             module top;\n\
               leaf #(.N(4)) a0();\n\
               leaf #(.N(4)) a1();\n\
               leaf #(.N(4)) a2();\n\
               leaf #(.N(9)) b0();\n\
               bare c0();\n\
               bare c1();\n\
             endmodule\n",
        );
        let (_, elab) = xezim_core::parse_and_elaborate_multi(
            &[src],
            Some("top"),
            &[],
            &["dedup.sv".to_string()],
            &[],
        )
        .expect("elaborate");
        let views = InstanceViews::build(&elab);
        let mut leaf_ns: Vec<i64> = views
            .of("leaf")
            .iter()
            .map(|v| v["N"].to_i64().unwrap())
            .collect();
        leaf_ns.sort_unstable();
        assert_eq!(leaf_ns, vec![4, 9]);
        assert_eq!(views.of("bare").len(), 1);
    }

    #[test]
    fn lint_and_type_checks_after_dedup_and_package_fast_paths() {
        let clean_src = String::from(
            "package p_base;\n\
               typedef logic [7:0] b8_t;\n\
               localparam int K = 3;\n\
             endpackage\n\
             package p_ext;\n\
               import p_base::*;\n\
               class helper;\n\
                 static function int f(b8_t v); return int'(v) + K; endfunction\n\
               endclass\n\
             endpackage\n\
             module sub #(parameter int W = 4) (input logic [W-1:0] i, output logic [W-1:0] o);\n\
               if (W > 4) begin : g_hi\n\
                 assign o = i;\n\
               end else begin : g_lo\n\
                 assign o = ~i;\n\
               end\n\
             endmodule\n\
             module top;\n\
               import p_ext::*;\n\
               logic [3:0] a, b, c;\n\
               logic [7:0] d, e;\n\
               sub #(.W(4)) u0(.i(a), .o(b));\n\
               sub #(.W(4)) u1(.i(a), .o(c));\n\
               sub #(.W(8)) u2(.i(d), .o(e));\n\
               int r;\n\
               initial r = helper::f(8'd2);\n\
             endmodule\n",
        );
        let (clean_defs, clean_elab) = xezim_core::parse_and_elaborate_multi(
            &[clean_src],
            Some("top"),
            &[],
            &["clean.sv".to_string()],
            &[],
        )
        .expect("elaborate clean");
        let clean_refs: Vec<&SourceDefinition> = clean_defs.values().collect();
        assert!(lint_should_fail(&clean_refs, &clean_elab).is_empty());
        assert!(crate::type_lint::check(&clean_refs, &clean_elab).is_empty());

        let bad_src = String::from(
            "package p_imp;\n\
               localparam int SECRET = 9;\n\
             endpackage\n\
             module m_imp;\n\
               import p_imp::*;\n\
               int own = 1;\n\
             endmodule\n\
             module m_out(output logic o);\n\
               assign o = 1'b1;\n\
             endmodule\n\
             module top;\n\
               bit [1:0][3:0] bad_pat = '{4'h1};\n\
               m_imp u_imp();\n\
               m_out u_out(.o(1'b0));\n\
               int x;\n\
               initial x = u_imp.SECRET;\n\
             endmodule\n",
        );
        let (bad_defs, bad_elab) = xezim_core::parse_and_elaborate_multi(
            &[bad_src],
            Some("top"),
            &[],
            &["bad.sv".to_string()],
            &[],
        )
        .expect("elaborate bad");
        let bad_refs: Vec<&SourceDefinition> = bad_defs.values().collect();
        let bad_errs = lint_should_fail(&bad_refs, &bad_elab);
        assert!(bad_errs.iter().any(|e| e.contains("assignment pattern")), "{bad_errs:?}");
        assert!(bad_errs.iter().any(|e| e.contains("cannot be connected")), "{bad_errs:?}");
        assert!(bad_errs.iter().any(|e| e.contains("only imported")), "{bad_errs:?}");
    }
}
