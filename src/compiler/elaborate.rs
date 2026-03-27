//! Elaborator: converts parsed AST into a flat simulation model.
//! Resolves net/variable declarations, continuous assigns, always blocks.

use ahash::AHashMap as HashMap;
use crate::ast::*;
use crate::ast::decl::*;
use crate::ast::module::*;
use crate::ast::types::*;
use crate::ast::expr::*;
use crate::ast::stmt::*;
use super::value::Value;

/// A resolved signal in the simulation model.
#[derive(Debug, Clone)]
pub struct Signal {
    pub name: String,
    pub width: u32,
    pub is_signed: bool,
    pub direction: Option<PortDirection>,
    pub value: Value,
}

/// A continuous assignment: assign lhs = rhs.
#[derive(Debug, Clone)]
pub struct ContinuousAssignment {
    pub lhs: Expression,
    pub rhs: Expression,
}

/// An always block for combinatorial logic.
#[derive(Debug, Clone)]
pub struct AlwaysBlock {
    pub kind: AlwaysKind,
    pub stmt: Statement,
}

/// An initial block for testbench.
#[derive(Debug, Clone)]
pub struct InitialBlock {
    pub stmt: Statement,
}

/// Elaborated module ready for simulation.
#[derive(Debug)]
pub struct ElaboratedModule {
    pub name: String,
    pub signals: HashMap<String, Signal>,
    pub port_order: Vec<String>,
    pub continuous_assigns: Vec<ContinuousAssignment>,
    pub always_blocks: Vec<AlwaysBlock>,
    pub initial_blocks: Vec<InitialBlock>,
    pub parameters: HashMap<String, Value>,
    /// Typedef name -> width mapping for user-defined types.
    pub typedefs: HashMap<String, u32>,
    /// Array declarations: base_name -> (lo_index, hi_index, element_width)
    pub arrays: HashMap<String, (i64, i64, u32)>,
}

impl ElaboratedModule {
    pub fn new(name: String) -> Self {
        Self {
            name,
            signals: HashMap::new(),
            port_order: Vec::new(),
            continuous_assigns: Vec::new(),
            always_blocks: Vec::new(),
            initial_blocks: Vec::new(),
            parameters: HashMap::new(),
            typedefs: HashMap::new(),
            arrays: HashMap::new(),
        }
    }
}

/// Elaborate a module declaration into a simulation model.
pub fn elaborate_module(
    module: &ModuleDeclaration,
    param_overrides: &HashMap<String, Value>,
) -> Result<ElaboratedModule, String> {
    let mut elab = ElaboratedModule::new(module.name.name.clone());

    // Process parameters
    for param in &module.params {
        if let ParameterKind::Data { data_type, assignments } = &param.kind {
            for assign in assignments {
                let mut width = resolve_type_width(data_type);
                let mut signed = is_type_signed(data_type);
                // IEEE 1800-2017 §6.20.2: Parameters with implicit type (no explicit type)
                // default to 32-bit signed integer.
                if matches!(data_type, DataType::Implicit { dimensions, .. } if dimensions.is_empty()) {
                    width = 32;
                    signed = true;
                }
                let val = if let Some(override_val) = param_overrides.get(&assign.name.name) {
                    override_val.clone()
                } else if let Some(init) = &assign.init {
                    let mut v = eval_const_expr_val(init, &elab.parameters).resize(width);
                    if signed { v.is_signed = true; }
                    v
                } else {
                    let mut v = Value::zero(width);
                    if signed { v.is_signed = true; }
                    v
                };
                elab.parameters.insert(assign.name.name.clone(), val);
            }
        }
    }

    // Process ports
    match &module.ports {
        PortList::Ansi(ports) => {
            for port in ports {
                let width = port.data_type.as_ref()
                    .map(|dt| resolve_type_width_with_params(dt, Some(&elab.parameters)))
                    .unwrap_or(1);
                let is_signed = port.data_type.as_ref()
                    .map(|dt| is_type_signed(dt))
                    .unwrap_or(false);
                let sig = Signal {
                    name: port.name.name.clone(),
                    width,
                    is_signed,
                    direction: port.direction,
                    value: Value::new(width),
                };
                elab.port_order.push(port.name.name.clone());
                elab.signals.insert(port.name.name.clone(), sig);
            }
        }
        PortList::NonAnsi(names) => {
            for name in names {
                elab.port_order.push(name.name.clone());
                // Direction/type will be declared in module body
            }
        }
        PortList::Empty => {}
    }

    // Process module items
    for item in &module.items {
        match item {
            ModuleItem::PortDeclaration(pd) => {
                let width = resolve_type_width_with_params(&pd.data_type, Some(&elab.parameters));
                let is_signed = is_type_signed(&pd.data_type);
                for decl in &pd.declarators {
                    let sig = Signal {
                        name: decl.name.name.clone(),
                        width,
                        is_signed,
                        direction: Some(pd.direction),
                        value: Value::new(width),
                    };
                    if !elab.port_order.contains(&decl.name.name) {
                        elab.port_order.push(decl.name.name.clone());
                    }
                    elab.signals.insert(decl.name.name.clone(), sig);
                }
            }
            ModuleItem::NetDeclaration(nd) => {
                let width = resolve_type_width_with_params(&nd.data_type, Some(&elab.parameters));
                let is_signed = is_type_signed(&nd.data_type);
                for decl in &nd.declarators {
                    let w = width_with_unpacked_dims(&decl.dimensions, width);
                    // supply0 → constant 0, supply1 → constant 1
                    let init_value = match nd.net_type {
                        NetType::Supply0 => Value::zero(w),
                        NetType::Supply1 => Value::ones(w),
                        _ => Value::new(w),
                    };
                    let sig = Signal {
                        name: decl.name.name.clone(),
                        width: w,
                        is_signed,
                        direction: None,
                        value: init_value,
                    };
                    elab.signals.insert(decl.name.name.clone(), sig);
                    // Wire with initializer → continuous assign (not constant eval)
                    if let Some(init_expr) = &decl.init {
                        elab.continuous_assigns.push(ContinuousAssignment {
                            lhs: make_ident_expr(&decl.name.name),
                            rhs: init_expr.clone(),
                        });
                    }
                }
            }
            ModuleItem::DataDeclaration(dd) => {
                let width = match &dd.data_type {
                    DataType::TypeReference { name, .. } => {
                        elab.typedefs.get(&name.name.name).copied().unwrap_or(resolve_type_width_with_params(&dd.data_type, Some(&elab.parameters)))
                    }
                    _ => resolve_type_width_with_params(&dd.data_type, Some(&elab.parameters)),
                };
                let is_signed = is_type_signed(&dd.data_type);
                for decl in &dd.declarators {
                    // Check for unpacked array dimensions (e.g., memory [0:255])
                    let array_range = extract_array_range(&decl.dimensions, &elab.parameters);
                    if let Some((lo, hi)) = array_range {
                        // Register this as an array for the simulator
                        elab.arrays.insert(decl.name.name.clone(), (lo, hi, width));
                        // Create individual element signals: name[lo], name[lo+1], ..., name[hi]
                        for idx in lo..=hi {
                            let elem_name = format!("{}[{}]", decl.name.name, idx);
                            let sig = Signal {
                                name: elem_name.clone(),
                                width,
                                is_signed,
                                direction: None,
                                value: Value::new(width),
                            };
                            elab.signals.insert(elem_name, sig);
                        }
                    } else {
                        let w = width;
                        let init_val = if let Some(init_expr) = &decl.init {
                            let mut rv = eval_const_expr_val(init_expr, &elab.parameters).resize(w);
                            if is_signed { rv.is_signed = true; }
                            rv
                        } else { Value::new(w) };
                        let sig = Signal {
                            name: decl.name.name.clone(),
                            width: w,
                            is_signed,
                            direction: None,
                            value: init_val,
                        };
                        elab.signals.insert(decl.name.name.clone(), sig);
                    }
                }
            }
            ModuleItem::ParameterDeclaration(pd) | ModuleItem::LocalparamDeclaration(pd) => {
                if let ParameterKind::Data { data_type, assignments } = &pd.kind {
                    let mut width = resolve_type_width(data_type);
                    let mut signed = is_type_signed(data_type);
                    // IEEE 1800-2017 §6.20.2: implicit type → signed 32-bit
                    if matches!(data_type, DataType::Implicit { dimensions, .. } if dimensions.is_empty()) {
                        width = 32;
                        signed = true;
                    }
                    for assign in assignments {
                        let val = if elab.parameters.contains_key(&assign.name.name) {
                            elab.parameters.get(&assign.name.name).cloned().unwrap_or(Value::zero(width))
                        } else if let Some(init) = &assign.init {
                            let mut v = eval_const_expr_val(init, &elab.parameters).resize(width);
                            if signed { v.is_signed = true; }
                            elab.parameters.insert(assign.name.name.clone(), v.clone());
                            v
                        } else {
                            let mut v = Value::zero(width);
                            if signed { v.is_signed = true; }
                            elab.parameters.insert(assign.name.name.clone(), v.clone());
                            v
                        };
                        let sig = Signal {
                            name: assign.name.name.clone(),
                            width, is_signed: signed,
                            direction: None, value: val,
                        };
                        elab.signals.insert(assign.name.name.clone(), sig);
                    }
                }
            }
            ModuleItem::ContinuousAssign(ca) => {
                for (lhs, rhs) in &ca.assignments {
                    elab.continuous_assigns.push(ContinuousAssignment {
                        lhs: lhs.clone(),
                        rhs: rhs.clone(),
                    });
                }
            }
            ModuleItem::GateInstantiation(gi) => {
                gate_inst_to_assigns(gi, &mut elab);
            }
            ModuleItem::AlwaysConstruct(ac) => {
                elab.always_blocks.push(AlwaysBlock {
                    kind: ac.kind,
                    stmt: ac.stmt.clone(),
                });
            }
            ModuleItem::InitialConstruct(ic) => {
                elab.initial_blocks.push(InitialBlock {
                    stmt: ic.stmt.clone(),
                });
            }
            ModuleItem::TypedefDeclaration(td) => {
                // Extract enum constants as parameters
                if let DataType::Enum(et) = &td.data_type {
                    let base_width = et.base_type.as_ref()
                        .map(|bt| resolve_type_width(bt))
                        .unwrap_or(32);
                    let mut next_val: u64 = 0;
                    for member in &et.members {
                        let val = if let Some(init) = &member.init {
                            eval_const_expr(init, &elab.parameters)
                        } else {
                            next_val
                        };
                        next_val = val + 1;
                        let v = Value::from_u64(val, base_width);
                        elab.parameters.insert(member.name.name.clone(), v.clone());
                        elab.signals.insert(member.name.name.clone(), Signal {
                            name: member.name.name.clone(),
                            width: base_width,
                            is_signed: false,
                            direction: None,
                            value: v,
                        });
                    }
                    // Register the typedef width
                    elab.typedefs.insert(td.name.name.clone(), base_width);
                } else {
                    // Non-enum typedef: resolve width from the underlying type
                    let w = resolve_type_width(&td.data_type);
                    elab.typedefs.insert(td.name.name.clone(), w);
                }
            }
            ModuleItem::FunctionDeclaration(_) | ModuleItem::TaskDeclaration(_) => {
                // Functions/tasks stored for call resolution
            }
            ModuleItem::GenerateRegion(gr) => {
                // Recursively process generate region items
                elaborate_items(&gr.items, &mut elab);
            }
            ModuleItem::GenerateIf(gi) => {
                elaborate_generate_if(&gi.branches, &mut elab);
            }
            _ => {}
        }
    }

    // IEEE 1800-2017 §6.10: Implicit nets — identifiers used in continuous assigns
    // or port connections that are not explicitly declared become implicit 1-bit wires.
    create_implicit_nets(&mut elab);

    Ok(elab)
}

/// Create implicit 1-bit wire signals for identifiers referenced in continuous assigns
/// but not declared anywhere (IEEE 1800-2017 §6.10).
fn create_implicit_nets(elab: &mut ElaboratedModule) {
    let mut implicit_names = Vec::new();
    for ca in &elab.continuous_assigns {
        collect_ident_names(&ca.lhs, &mut implicit_names);
        collect_ident_names(&ca.rhs, &mut implicit_names);
    }
    implicit_names.sort();
    implicit_names.dedup();
    for name in implicit_names {
        if !elab.signals.contains_key(&name) && !elab.parameters.contains_key(&name) {
            elab.signals.insert(name.clone(), Signal {
                name, width: 1, is_signed: false,
                direction: None, value: Value::new(1),
            });
        }
    }
}

/// Collect all plain identifier names from an expression tree.
fn collect_ident_names(expr: &Expression, out: &mut Vec<String>) {
    match &expr.kind {
        ExprKind::Ident(hier) => {
            if hier.path.len() == 1 && hier.path[0].selects.is_empty() {
                out.push(hier.path[0].name.name.clone());
            }
        }
        ExprKind::Unary { operand, .. } => collect_ident_names(operand, out),
        ExprKind::Binary { left, right, .. } => { collect_ident_names(left, out); collect_ident_names(right, out); }
        ExprKind::Conditional { condition, then_expr, else_expr } => {
            collect_ident_names(condition, out); collect_ident_names(then_expr, out); collect_ident_names(else_expr, out);
        }
        ExprKind::Concatenation(parts) => { for p in parts { collect_ident_names(p, out); } }
        ExprKind::Replication { count, exprs } => { collect_ident_names(count, out); for e in exprs { collect_ident_names(e, out); } }
        ExprKind::Index { expr, index } => { collect_ident_names(expr, out); collect_ident_names(index, out); }
        ExprKind::RangeSelect { expr, left, right, .. } => { collect_ident_names(expr, out); collect_ident_names(left, out); collect_ident_names(right, out); }
        ExprKind::Paren(inner) => collect_ident_names(inner, out),
        ExprKind::Call { func, args } => { collect_ident_names(func, out); for a in args { collect_ident_names(a, out); } }
        ExprKind::MemberAccess { expr, .. } => collect_ident_names(expr, out),
        _ => {}
    }
}

/// Helper: process a slice of module items into the elaborated module.
/// This is extracted so it can be called recursively for generate regions.
/// Evaluate a generate-if: pick the first branch whose condition is true (or the else branch).
fn elaborate_generate_if(
    branches: &[(Option<Expression>, Vec<ModuleItem>)],
    elab: &mut ElaboratedModule,
) {
    for (cond_opt, items) in branches {
        match cond_opt {
            Some(cond) => {
                // Evaluate condition as a constant expression using current parameters
                let val = eval_const_expr(cond, &elab.parameters);
                if val != 0 {
                    elaborate_items(items, elab);
                    return;
                }
            }
            None => {
                // Unconditional else branch
                elaborate_items(items, elab);
                return;
            }
        }
    }
}

fn elaborate_items(items: &[ModuleItem], elab: &mut ElaboratedModule) {
    for item in items {
        match item {
            ModuleItem::PortDeclaration(pd) => {
                let width = resolve_type_width_with_params(&pd.data_type, Some(&elab.parameters));
                let is_signed = is_type_signed(&pd.data_type);
                for decl in &pd.declarators {
                    let sig = Signal {
                        name: decl.name.name.clone(), width, is_signed,
                        direction: Some(pd.direction), value: Value::new(width),
                    };
                    elab.signals.insert(decl.name.name.clone(), sig);
                    elab.port_order.push(decl.name.name.clone());
                }
            }
            ModuleItem::NetDeclaration(nd) => {
                let width = resolve_type_width_with_params(&nd.data_type, Some(&elab.parameters));
                let is_signed = is_type_signed(&nd.data_type);
                for decl in &nd.declarators {
                    let init_value = match nd.net_type {
                        NetType::Supply0 => Value::zero(width),
                        NetType::Supply1 => Value::ones(width),
                        _ => Value::new(width),
                    };
                    let sig = Signal {
                        name: decl.name.name.clone(), width, is_signed,
                        direction: None, value: init_value,
                    };
                    elab.signals.insert(decl.name.name.clone(), sig);
                    if let Some(init_expr) = &decl.init {
                        elab.continuous_assigns.push(ContinuousAssignment {
                            lhs: make_ident_expr(&decl.name.name),
                            rhs: init_expr.clone(),
                        });
                    }
                }
            }
            ModuleItem::DataDeclaration(dd) => {
                let width = match &dd.data_type {
                    DataType::TypeReference { name, .. } => {
                        elab.typedefs.get(&name.name.name).copied().unwrap_or(resolve_type_width_with_params(&dd.data_type, Some(&elab.parameters)))
                    }
                    _ => resolve_type_width_with_params(&dd.data_type, Some(&elab.parameters)),
                };
                let is_signed = is_type_signed(&dd.data_type);
                for decl in &dd.declarators {
                    let array_range = extract_array_range(&decl.dimensions, &elab.parameters);
                    if let Some((lo, hi)) = array_range {
                        elab.arrays.insert(decl.name.name.clone(), (lo, hi, width));
                        for idx in lo..=hi {
                            let elem_name = format!("{}[{}]", decl.name.name, idx);
                            let sig = Signal { name: elem_name.clone(), width, is_signed, direction: None, value: Value::new(width) };
                            elab.signals.insert(elem_name, sig);
                        }
                    } else {
                        let init_val = if let Some(init_expr) = &decl.init {
                            let mut rv = eval_const_expr_val(init_expr, &elab.parameters).resize(width);
                            if is_signed { rv.is_signed = true; }
                            rv
                        } else { Value::new(width) };
                        let sig = Signal { name: decl.name.name.clone(), width, is_signed, direction: None, value: init_val };
                        elab.signals.insert(decl.name.name.clone(), sig);
                    }
                }
            }
            ModuleItem::ParameterDeclaration(pd) | ModuleItem::LocalparamDeclaration(pd) => {
                if let ParameterKind::Data { data_type, assignments } = &pd.kind {
                    let mut width = resolve_type_width_with_params(data_type, Some(&elab.parameters));
                    let signed = is_type_signed(data_type);
                    if matches!(data_type, DataType::Implicit { dimensions, .. } if dimensions.is_empty()) { width = 32; }
                    for assign in assignments {
                        if !elab.parameters.contains_key(&assign.name.name) {
                            let val = if let Some(init) = &assign.init {
                                let mut v = eval_const_expr_val(init, &elab.parameters).resize(width);
                                if signed { v.is_signed = true; }
                                v
                            } else { Value::zero(width) };
                            elab.parameters.insert(assign.name.name.clone(), val);
                        }
                    }
                }
            }
            ModuleItem::ContinuousAssign(ca) => {
                for (lhs, rhs) in &ca.assignments {
                    elab.continuous_assigns.push(ContinuousAssignment { lhs: lhs.clone(), rhs: rhs.clone() });
                }
            }
            ModuleItem::GateInstantiation(gi) => {
                gate_inst_to_assigns(gi, elab);
            }
            ModuleItem::AlwaysConstruct(ac) => {
                elab.always_blocks.push(AlwaysBlock { kind: ac.kind, stmt: ac.stmt.clone() });
            }
            ModuleItem::InitialConstruct(ic) => {
                elab.initial_blocks.push(InitialBlock { stmt: ic.stmt.clone() });
            }
            ModuleItem::TypedefDeclaration(td) => {
                if let DataType::Enum(et) = &td.data_type {
                    let base_width = et.base_type.as_ref()
                        .map(|bt| resolve_type_width(bt))
                        .unwrap_or(32);
                    let mut next_val: u64 = 0;
                    for member in &et.members {
                        let val = if let Some(init) = &member.init {
                            eval_const_expr(init, &elab.parameters)
                        } else { next_val };
                        next_val = val + 1;
                        let v = Value::from_u64(val, base_width);
                        elab.parameters.insert(member.name.name.clone(), v.clone());
                        elab.signals.insert(member.name.name.clone(), Signal {
                            name: member.name.name.clone(), width: base_width,
                            is_signed: false, direction: None, value: v,
                        });
                    }
                    elab.typedefs.insert(td.name.name.clone(), base_width);
                } else {
                    let w = resolve_type_width(&td.data_type);
                    elab.typedefs.insert(td.name.name.clone(), w);
                }
            }
            ModuleItem::GenerateRegion(gr) => {
                elaborate_items(&gr.items, elab);
            }
            ModuleItem::GenerateIf(gi) => {
                elaborate_generate_if(&gi.branches, elab);
            }
            _ => {}
        }
    }
}

/// Resolve the width of a data type.
pub fn resolve_type_width(dt: &DataType) -> u32 {
    resolve_type_width_with_params(dt, None)
}

pub fn resolve_type_width_with_params(dt: &DataType, params: Option<&HashMap<String, Value>>) -> u32 {
    match dt {
        DataType::IntegerVector { dimensions, .. } => {
            if dimensions.is_empty() { return 1; }
            let mut total = 1u32;
            for dim in dimensions {
                if let PackedDimension::Range { left, right, .. } = dim {
                    if let (Some(l), Some(r)) = (const_eval_i64_with_params(left, params), const_eval_i64_with_params(right, params)) {
                        let w = (l - r).abs() + 1;
                        total *= w as u32;
                    }
                }
            }
            total
        }
        DataType::IntegerAtom { kind, .. } => match kind {
            IntegerAtomType::Byte => 8,
            IntegerAtomType::ShortInt => 16,
            IntegerAtomType::Int => 32,
            IntegerAtomType::LongInt => 64,
            IntegerAtomType::Integer => 32,
            IntegerAtomType::Time => 64,
        },
        DataType::Real { .. } => 64,
        DataType::Implicit { dimensions, .. } => {
            if dimensions.is_empty() { return 1; }
            let mut total = 1u32;
            for dim in dimensions {
                if let PackedDimension::Range { left, right, .. } = dim {
                    if let (Some(l), Some(r)) = (const_eval_i64_with_params(left, params), const_eval_i64_with_params(right, params)) {
                        let w = (l - r).abs() + 1;
                        total *= w as u32;
                    }
                }
            }
            total
        }
        DataType::TypeReference { dimensions, .. } => {
            if dimensions.is_empty() { 32 } // Default for user-defined types
            else { 32 }
        }
        DataType::Void(_) => 0,
        _ => 32,
    }
}

/// Check if a data type is signed.
pub fn is_type_signed(dt: &DataType) -> bool {
    match dt {
        DataType::IntegerVector { signing, .. } => matches!(signing, Some(Signing::Signed)),
        DataType::IntegerAtom { kind, signing, .. } => {
            if let Some(s) = signing { return matches!(s, Signing::Signed); }
            // Default signedness per LRM
            matches!(kind, IntegerAtomType::Byte | IntegerAtomType::ShortInt |
                     IntegerAtomType::Int | IntegerAtomType::LongInt | IntegerAtomType::Integer)
        }
        _ => false,
    }
}

/// Simple constant expression evaluator for dimension ranges.
fn const_eval_u64(expr: &Expression) -> Option<u64> {
    // Try i64 first, then convert
    const_eval_i64(expr).map(|v| v as u64)
}

fn const_eval_i64(expr: &Expression) -> Option<i64> {
    const_eval_i64_with_params(expr, None)
}

fn const_eval_i64_with_params(expr: &Expression, params: Option<&HashMap<String, Value>>) -> Option<i64> {
    match &expr.kind {
        ExprKind::Number(NumberLiteral::Integer { value, base, .. }) => {
            let clean: String = value.chars().filter(|c| *c != '_').collect();
            match base {
                NumberBase::Decimal => clean.parse::<i64>().ok(),
                NumberBase::Hex => u64::from_str_radix(&clean, 16).ok().map(|v| v as i64),
                NumberBase::Binary => u64::from_str_radix(&clean, 2).ok().map(|v| v as i64),
                NumberBase::Octal => u64::from_str_radix(&clean, 8).ok().map(|v| v as i64),
            }
        }
        ExprKind::Unary { op: UnaryOp::Minus, operand } => {
            const_eval_i64_with_params(operand, params).map(|v| -v)
        }
        ExprKind::Unary { op: UnaryOp::Plus, operand } => {
            const_eval_i64_with_params(operand, params)
        }
        ExprKind::Binary { op: BinaryOp::Sub, left, right, .. } => {
            let l = const_eval_i64_with_params(left, params)?;
            let r = const_eval_i64_with_params(right, params)?;
            Some(l.wrapping_sub(r))
        }
        ExprKind::Binary { op: BinaryOp::Add, left, right, .. } => {
            let l = const_eval_i64_with_params(left, params)?;
            let r = const_eval_i64_with_params(right, params)?;
            Some(l.wrapping_add(r))
        }
        ExprKind::Binary { op: BinaryOp::Mul, left, right, .. } => {
            let l = const_eval_i64_with_params(left, params)?;
            let r = const_eval_i64_with_params(right, params)?;
            Some(l.wrapping_mul(r))
        }
        ExprKind::Binary { op: BinaryOp::ShiftLeft | BinaryOp::ArithShiftLeft, left, right, .. } => {
            let l = const_eval_i64_with_params(left, params)?;
            let r = const_eval_i64_with_params(right, params)?;
            Some(l.wrapping_shl(r as u32))
        }
        ExprKind::Binary { op: BinaryOp::ShiftRight, left, right, .. } => {
            let l = const_eval_i64_with_params(left, params)?;
            let r = const_eval_i64_with_params(right, params)?;
            Some(l.wrapping_shr(r as u32))
        }
        ExprKind::Binary { op: BinaryOp::BitOr, left, right, .. } => {
            let l = const_eval_i64_with_params(left, params)?;
            let r = const_eval_i64_with_params(right, params)?;
            Some(l | r)
        }
        ExprKind::Binary { op: BinaryOp::BitAnd, left, right, .. } => {
            let l = const_eval_i64_with_params(left, params)?;
            let r = const_eval_i64_with_params(right, params)?;
            Some(l & r)
        }
        ExprKind::Paren(inner) => const_eval_i64_with_params(inner, params),
        ExprKind::Ident(hier) => {
            if let Some(p) = params {
                let name = hier.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                p.get(name).and_then(|v| v.to_u64()).map(|v| v as i64)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Extract array range from unpacked dimensions. Returns Some((lo, hi)) for
/// `[lo:hi]` or `[size]` (which means [0:size-1]).
fn extract_array_range(dims: &[crate::ast::types::UnpackedDimension], params: &HashMap<String, Value>) -> Option<(i64, i64)> {
    use crate::ast::types::UnpackedDimension;
    if dims.is_empty() { return None; }
    match &dims[0] {
        UnpackedDimension::Range { left, right, .. } => {
            let l = const_eval_i64_with_params(left, Some(params)).unwrap_or(0);
            let r = const_eval_i64_with_params(right, Some(params)).unwrap_or(0);
            let lo = l.min(r);
            let hi = l.max(r);
            Some((lo, hi))
        }
        UnpackedDimension::Expression { expr, .. } => {
            let size = const_eval_i64_with_params(expr, Some(params)).unwrap_or(0);
            if size > 0 { Some((0, size - 1)) } else { None }
        }
        _ => None,
    }
}

fn width_with_unpacked_dims(dims: &[crate::ast::types::UnpackedDimension], base_width: u32) -> u32 {
    // For now, just return base width (arrays not fully supported in sim)
    base_width
}

/// Evaluate a constant expression (for enum values, parameter defaults, etc.)
fn eval_const_expr(expr: &Expression, params: &HashMap<String, Value>) -> u64 {
    eval_const_expr_val(expr, params).to_u64().unwrap_or(0)
}

/// Evaluate a constant expression, returning a full Value (preserving width/sign).
fn eval_const_expr_val(expr: &Expression, params: &HashMap<String, Value>) -> Value {
    match &expr.kind {
        ExprKind::Number(num) => {
            match num {
                NumberLiteral::Integer { size, signed, base, value, .. } => {
                    let w = size.unwrap_or(32);
                    let r = match base {
                        NumberBase::Binary => 2, NumberBase::Octal => 8,
                        NumberBase::Hex => 16, NumberBase::Decimal => 10,
                    };
                    let mut v = Value::from_str_radix(&value.replace('_', ""), r, w);
                    v.is_signed = *signed;
                    v
                }
                NumberLiteral::Real(f) => Value::from_u64(*f as u64, 64),
                NumberLiteral::UnbasedUnsized(c) => match c {
                    '0' => Value::zero(1), '1' => Value::from_u64(1, 1), _ => Value::new(1),
                },
            }
        }
        ExprKind::Ident(hier) => {
            let name = hier.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
            params.get(name).cloned().unwrap_or(Value::zero(32))
        }
        ExprKind::Binary { op, left, right } => {
            let l = eval_const_expr_val(left, params);
            let r = eval_const_expr_val(right, params);
            match op {
                BinaryOp::Add => l.add(&r),
                BinaryOp::Sub => l.sub(&r),
                BinaryOp::Mul => l.mul(&r),
                BinaryOp::Div => l.div(&r),
                BinaryOp::ShiftLeft | BinaryOp::ArithShiftLeft => l.shift_left(&r),
                BinaryOp::ShiftRight => l.shift_right(&r),
                BinaryOp::BitOr => l.bitwise_or(&r),
                BinaryOp::BitAnd => l.bitwise_and(&r),
                _ => Value::zero(32),
            }
        }
        ExprKind::Unary { op, operand } => {
            let v = eval_const_expr_val(operand, params);
            match op {
                UnaryOp::Minus => { let mut r = Value::zero(v.width).sub(&v).resize(v.width); r.is_signed = true; r }
                UnaryOp::Plus => v,
                UnaryOp::BitNot => v.bitwise_not(),
                UnaryOp::LogNot => v.logic_not(),
                _ => v,
            }
        }
        ExprKind::Paren(inner) => eval_const_expr_val(inner, params),
        ExprKind::Conditional { condition, then_expr, else_expr } => {
            let c = eval_const_expr_val(condition, params);
            if c.is_true() { eval_const_expr_val(then_expr, params) }
            else { eval_const_expr_val(else_expr, params) }
        }
        _ => Value::zero(32),
    }
}

/// Inline module instantiations: replace instances with their continuous assigns and always blocks.
/// Handles recursive/multi-level hierarchies by walking all levels depth-first.
pub fn inline_instantiations(
    elab: &mut ElaboratedModule,
    modules: &HashMap<String, &crate::ast::module::ModuleDeclaration>,
) -> Result<(), String> {
    let module_name = elab.name.clone();
    let top_mod = match modules.get(&module_name) {
        Some(m) => *m,
        None => return Err(format!("Top module '{}' not found in module map", module_name)),
    };
    // Recursively inline starting from the top module's items
    inline_module_items(elab, top_mod, "", modules)
}

/// Recursively inline all instantiations found in `source_mod`, using `prefix` for signal naming.
/// Flatten module items by resolving generate-if/else and generate regions.
/// Returns all effective items after evaluating generate conditions.
fn collect_effective_items(items: &[ModuleItem], params: &HashMap<String, Value>) -> Vec<ModuleItem> {
    let mut result = Vec::new();
    for item in items {
        match item {
            ModuleItem::GenerateRegion(gr) => {
                result.extend(collect_effective_items(&gr.items, params));
            }
            ModuleItem::GenerateIf(gi) => {
                let mut matched = false;
                for (cond, branch_items) in &gi.branches {
                    if let Some(cond_expr) = cond {
                        let val = eval_const_expr(cond_expr, params);
                        if val != 0 {
                            result.extend(collect_effective_items(branch_items, params));
                            matched = true;
                            break;
                        }
                    } else {
                        // Unconditional else branch
                        result.extend(collect_effective_items(branch_items, params));
                        matched = true;
                        break;
                    }
                }
                let _ = matched;
            }
            other => result.push(other.clone()),
        }
    }
    result
}

fn inline_module_items(
    elab: &mut ElaboratedModule,
    source_mod: &crate::ast::module::ModuleDeclaration,
    prefix: &str,
    modules: &HashMap<String, &crate::ast::module::ModuleDeclaration>,
) -> Result<(), String> {
    use crate::ast::decl::*;
    use crate::ast::module::*;
    use crate::ast::expr::*;
    use super::value::Value;

    // First pass: collect typedef names from this module so we can distinguish them from instantiations
    let mut local_typedefs: std::collections::HashSet<String> = std::collections::HashSet::new();
    for item in &source_mod.items {
        if let ModuleItem::TypedefDeclaration(td) = item {
            local_typedefs.insert(td.name.name.clone());
        }
    }

    for item in &source_mod.items {
        if let ModuleItem::ModuleInstantiation(inst) = item {
            let sub_mod_name = &inst.module_name.name;
            let sub_mod = match modules.get(sub_mod_name) {
                Some(m) => *m,
                None => {
                    // Skip if this is a typedef name (user-defined type parsed as instantiation)
                    if elab.typedefs.contains_key(sub_mod_name) || local_typedefs.contains(sub_mod_name) {
                        continue;
                    }
                    let context = if prefix.is_empty() {
                        " ".to_string()
                    } else {
                        format!(" (as '{}') ", prefix.trim_end_matches('.'))
                    };
                    let available = modules.keys().cloned().collect::<Vec<_>>().join(", ");
                    return Err(format!(
                        "Module '{}' not found. Instantiated{}but not defined in any source file. Available modules: {}",
                        sub_mod_name, context, available
                    ));
                }
            };

            for hier_inst in &inst.instances {
                let inst_name = &hier_inst.name.name;
                let inst_prefix = if prefix.is_empty() {
                    format!("{}.", inst_name)
                } else {
                    format!("{}{}.", prefix, inst_name)
                };

                // Build port mapping: sub_port_name -> parent_expression
                let mut port_map: HashMap<String, Expression> = HashMap::new();

                let sub_port_names: Vec<String> = match &sub_mod.ports {
                    PortList::Ansi(ports) => ports.iter().map(|p| p.name.name.clone()).collect(),
                    PortList::NonAnsi(names) => names.iter().map(|n| n.name.clone()).collect(),
                    PortList::Empty => Vec::new(),
                };

                for (i, conn) in hier_inst.connections.iter().enumerate() {
                    match conn {
                        PortConnection::Named { name, expr } => {
                            if let Some(e) = expr {
                                // Rewrite the parent expression with the current prefix context
                                // (for nested: parent expressions reference the enclosing scope)
                                let rewritten = if prefix.is_empty() {
                                    e.clone()
                                } else {
                                    rewrite_expr_all(e, prefix)
                                };
                                port_map.insert(name.name.clone(), rewritten);
                            }
                        }
                        PortConnection::Ordered(Some(e)) => {
                            if i < sub_port_names.len() {
                                let rewritten = if prefix.is_empty() {
                                    e.clone()
                                } else {
                                    rewrite_expr_all(e, prefix)
                                };
                                port_map.insert(sub_port_names[i].clone(), rewritten);
                            }
                        }
                        _ => {}
                    }
                }

                // Process sub-module parameters (e.g., #(parameter W=8))
                // Build override map from instantiation #(.WIDTH(8))
                let mut param_overrides: HashMap<String, Value> = HashMap::new();
                if let Some(pconns) = &inst.params {
                    let param_names: Vec<String> = sub_mod.params.iter().filter_map(|p| {
                        if let ParameterKind::Data { assignments, .. } = &p.kind {
                            assignments.first().map(|a| a.name.name.clone())
                        } else { None }
                    }).collect();
                    for (i, pc) in pconns.iter().enumerate() {
                        match pc {
                            ParamConnection::Named { name, value: Some(expr) } => {
                                let v = eval_const_expr_val(expr, &elab.parameters);
                                param_overrides.insert(name.name.clone(), v);
                            }
                            ParamConnection::Ordered(Some(expr)) => {
                                if i < param_names.len() {
                                    let v = eval_const_expr_val(expr, &elab.parameters);
                                    param_overrides.insert(param_names[i].clone(), v);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                for param in &sub_mod.params {
                    if let ParameterKind::Data { data_type, assignments } = &param.kind {
                        let mut width = resolve_type_width(data_type);
                        if matches!(data_type, DataType::Implicit { dimensions, .. } if dimensions.is_empty()) {
                            width = 32;
                        }
                        for assign in assignments {
                            let val = if let Some(ovr) = param_overrides.get(&assign.name.name) {
                                ovr.clone().resize(width)
                            } else if let Some(init) = &assign.init {
                                eval_const_expr_val(init, &elab.parameters).resize(width)
                            } else {
                                Value::zero(width)
                            };
                            elab.parameters.insert(assign.name.name.clone(), val);
                        }
                    }
                }
                // Process parameter/localparam items inside the sub-module body
                for sub_item in &sub_mod.items {
                    if let ModuleItem::ParameterDeclaration(pd) | ModuleItem::LocalparamDeclaration(pd) = sub_item {
                        if let ParameterKind::Data { data_type, assignments } = &pd.kind {
                            let mut width = resolve_type_width_with_params(data_type, Some(&elab.parameters));
                            if matches!(data_type, DataType::Implicit { dimensions, .. } if dimensions.is_empty()) {
                                width = 32;
                            }
                            for assign in assignments {
                                if !elab.parameters.contains_key(&assign.name.name) {
                                    let val = if let Some(init) = &assign.init {
                                        eval_const_expr_val(init, &elab.parameters).resize(width)
                                    } else {
                                        Value::zero(width)
                                    };
                                    elab.parameters.insert(assign.name.name.clone(), val);
                                }
                            }
                        }
                    }
                }

                // Declare sub-module port signals
                match &sub_mod.ports {
                    PortList::Ansi(ports) => {
                        for port in ports {
                            let width = port.data_type.as_ref()
                                .map(|dt| resolve_type_width_with_params(dt, Some(&elab.parameters)))
                                .unwrap_or(1);
                            let sig_name = format!("{}{}", inst_prefix, port.name.name);
                            elab.signals.insert(sig_name.clone(), Signal {
                                name: sig_name, width,
                                is_signed: port.data_type.as_ref().map(|dt| is_type_signed(dt)).unwrap_or(false),
                                direction: port.direction,
                                value: Value::new(width),
                            });
                        }
                    }
                    PortList::NonAnsi(_names) => {
                        // For non-ANSI, port signals are declared via PortDeclaration items
                        // in the module body. Process those here.
                        let effective_items = collect_effective_items(&sub_mod.items, &elab.parameters);
                        for sub_item in &effective_items {
                            if let ModuleItem::PortDeclaration(pd) = sub_item {
                                let width = resolve_type_width_with_params(&pd.data_type, Some(&elab.parameters));
                                let is_signed = is_type_signed(&pd.data_type);
                                for decl in &pd.declarators {
                                    let sig_name = format!("{}{}", inst_prefix, decl.name.name);
                                    elab.signals.insert(sig_name.clone(), Signal {
                                        name: sig_name, width, is_signed,
                                        direction: Some(pd.direction),
                                        value: Value::new(width),
                                    });
                                }
                            }
                        }
                    }
                    PortList::Empty => {}
                }

                // Declare internal nets/vars (including from generate blocks)
                let effective_decl_items = collect_effective_items(&sub_mod.items, &elab.parameters);
                for sub_item in &effective_decl_items {
                    match sub_item {
                        ModuleItem::NetDeclaration(nd) => {
                            let width = resolve_type_width_with_params(&nd.data_type, Some(&elab.parameters));
                            for decl in &nd.declarators {
                                let sig_name = format!("{}{}", inst_prefix, decl.name.name);
                                let init_value = match nd.net_type {
                                    NetType::Supply0 => Value::zero(width),
                                    NetType::Supply1 => Value::ones(width),
                                    _ => Value::new(width),
                                };
                                elab.signals.insert(sig_name.clone(), Signal {
                                    name: sig_name, width,
                                    is_signed: is_type_signed(&nd.data_type),
                                    direction: None, value: init_value,
                                });
                            }
                        }
                        ModuleItem::DataDeclaration(dd) => {
                            let width = resolve_type_width_with_params(&dd.data_type, Some(&elab.parameters));
                            let is_signed = is_type_signed(&dd.data_type);
                            for decl in &dd.declarators {
                                let base_name = decl.name.name.clone();
                                let sig_name = format!("{}{}", inst_prefix, base_name);
                                let array_range = extract_array_range(&decl.dimensions, &elab.parameters);
                                if let Some((lo, hi)) = array_range {
                                    elab.arrays.insert(sig_name.clone(), (lo, hi, width));
                                    for idx in lo..=hi {
                                        let elem_name = format!("{}[{}]", sig_name, idx);
                                        elab.signals.insert(elem_name.clone(), Signal {
                                            name: elem_name, width, is_signed,
                                            direction: None, value: Value::new(width),
                                        });
                                    }
                                } else {
                                    let init_val = if let Some(init_expr) = &decl.init {
                                        eval_const_expr_val(init_expr, &elab.parameters).resize(width)
                                    } else { Value::new(width) };
                                    elab.signals.insert(sig_name.clone(), Signal {
                                        name: sig_name, width, is_signed,
                                        direction: None, value: init_val,
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }

                // Port connection assigns
                // Build a direction map for port signals (needed for non-ANSI ports)
                let port_directions: HashMap<String, PortDirection> = {
                    let mut dirs = HashMap::new();
                    match &sub_mod.ports {
                        PortList::Ansi(ports) => {
                            for port in ports {
                                if let Some(dir) = port.direction {
                                    dirs.insert(port.name.name.clone(), dir);
                                }
                            }
                        }
                        PortList::NonAnsi(_) => {
                            let effective_items = collect_effective_items(&sub_mod.items, &elab.parameters);
                            for sub_item in &effective_items {
                                if let ModuleItem::PortDeclaration(pd) = sub_item {
                                    for decl in &pd.declarators {
                                        dirs.insert(decl.name.name.clone(), pd.direction);
                                    }
                                }
                            }
                        }
                        PortList::Empty => {}
                    }
                    dirs
                };

                for (port_name, parent_expr) in &port_map {
                    let sub_sig_name = format!("{}{}", inst_prefix, port_name);
                    let sub_expr = make_ident_expr(&sub_sig_name);
                    match port_directions.get(port_name) {
                        Some(PortDirection::Input) | Some(PortDirection::Inout) => {
                            elab.continuous_assigns.push(ContinuousAssignment {
                                lhs: sub_expr, rhs: parent_expr.clone(),
                            });
                        }
                        Some(PortDirection::Output) => {
                            elab.continuous_assigns.push(ContinuousAssignment {
                                lhs: parent_expr.clone(), rhs: sub_expr,
                            });
                        }
                        _ => {
                            // Unknown direction (not declared) — treat as input
                            elab.continuous_assigns.push(ContinuousAssignment {
                                lhs: sub_expr, rhs: parent_expr.clone(),
                            });
                        }
                    }
                }

                // Collect sub-module local signal names (ports + internal nets/vars)
                // Only these should be prefixed during rewriting
                let mut local_names: std::collections::HashSet<String> = std::collections::HashSet::new();
                match &sub_mod.ports {
                    PortList::Ansi(ports) => {
                        for port in ports { local_names.insert(port.name.name.clone()); }
                    }
                    PortList::NonAnsi(names) => {
                        for name in names { local_names.insert(name.name.clone()); }
                    }
                    PortList::Empty => {}
                }
                for sub_item in &effective_decl_items {
                    match sub_item {
                        ModuleItem::NetDeclaration(nd) => { for d in &nd.declarators { local_names.insert(d.name.name.clone()); } }
                        ModuleItem::DataDeclaration(dd) => { for d in &dd.declarators { local_names.insert(d.name.name.clone()); } }
                        ModuleItem::PortDeclaration(pd) => { for d in &pd.declarators { local_names.insert(d.name.name.clone()); } }
                        _ => {}
                    }
                }

                // Inline the sub-module's continuous assigns (rewrite signal names with inst_prefix)
                // Collect all effective items from the sub-module, resolving generates
                let effective_items = collect_effective_items(&sub_mod.items, &elab.parameters);
                for sub_item in &effective_items {
                    if let ModuleItem::ContinuousAssign(ca) = sub_item {
                        for (lhs, rhs) in &ca.assignments {
                            let new_lhs = rewrite_expr(lhs, &inst_prefix, &port_map, &local_names);
                            let new_rhs = rewrite_expr(rhs, &inst_prefix, &port_map, &local_names);
                            elab.continuous_assigns.push(ContinuousAssignment {
                                lhs: new_lhs, rhs: new_rhs,
                            });
                        }
                    }
                    // Gate-level primitives → continuous assigns with rewritten signals
                    if let ModuleItem::GateInstantiation(gi) = sub_item {
                        let assigns = gate_inst_to_assign_pairs(gi);
                        for (lhs, rhs) in assigns {
                            let new_lhs = rewrite_expr(&lhs, &inst_prefix, &port_map, &local_names);
                            let new_rhs = rewrite_expr(&rhs, &inst_prefix, &port_map, &local_names);
                            elab.continuous_assigns.push(ContinuousAssignment {
                                lhs: new_lhs, rhs: new_rhs,
                            });
                        }
                    }
                    // Wire declarations with initializers → continuous assigns
                    if let ModuleItem::NetDeclaration(nd) = sub_item {
                        for decl in &nd.declarators {
                            if let Some(init_expr) = &decl.init {
                                let lhs_name = format!("{}{}", inst_prefix, decl.name.name);
                                let new_lhs = make_ident_expr(&lhs_name);
                                let new_rhs = rewrite_expr(init_expr, &inst_prefix, &port_map, &local_names);
                                elab.continuous_assigns.push(ContinuousAssignment {
                                    lhs: new_lhs, rhs: new_rhs,
                                });
                            }
                        }
                    }
                    if let ModuleItem::AlwaysConstruct(ac) = sub_item {
                        elab.always_blocks.push(super::elaborate::AlwaysBlock {
                            kind: ac.kind,
                            stmt: rewrite_stmt(&ac.stmt, &inst_prefix, &port_map, &local_names),
                        });
                    }
                    if let ModuleItem::InitialConstruct(ic) = sub_item {
                        elab.initial_blocks.push(super::elaborate::InitialBlock {
                            stmt: rewrite_stmt(&ic.stmt, &inst_prefix, &port_map, &local_names),
                        });
                    }
                    // Propagate enum constants from sub-module typedefs
                    if let ModuleItem::TypedefDeclaration(td) = sub_item {
                        if let DataType::Enum(et) = &td.data_type {
                            let base_width = et.base_type.as_ref()
                                .map(|bt| resolve_type_width(bt)).unwrap_or(32);
                            let mut next_val: u64 = 0;
                            for member in &et.members {
                                let val = if let Some(init) = &member.init {
                                    eval_const_expr(init, &elab.parameters)
                                } else { next_val };
                                next_val = val + 1;
                                let v = Value::from_u64(val, base_width);
                                // Enum constants are global (not prefixed)
                                if !elab.parameters.contains_key(&member.name.name) {
                                    elab.parameters.insert(member.name.name.clone(), v.clone());
                                    elab.signals.insert(member.name.name.clone(), Signal {
                                        name: member.name.name.clone(), width: base_width,
                                        is_signed: false, direction: None, value: v,
                                    });
                                }
                            }
                            elab.typedefs.insert(td.name.name.clone(), base_width);
                        }
                    }
                }

                // *** RECURSE: inline any sub-sub-module instantiations ***
                inline_module_items(elab, sub_mod, &inst_prefix, modules)?;
            }
        }
    }
    Ok(())
}

/// Create an identifier expression from a signal name.
/// Convert gate-level primitive instantiations to continuous assigns.
/// Adds the resulting assigns directly to the elaborated module.
fn gate_inst_to_assigns(gi: &crate::ast::decl::GateInstantiation, elab: &mut ElaboratedModule) {
    for (lhs, rhs) in gate_inst_to_assign_pairs(gi) {
        elab.continuous_assigns.push(ContinuousAssignment { lhs, rhs });
    }
}

/// Convert gate-level primitive instantiation to (lhs, rhs) pairs for continuous assigns.
/// For `and g(out, in1, in2)` → `assign out = in1 & in2`
fn gate_inst_to_assign_pairs(
    gi: &crate::ast::decl::GateInstantiation,
) -> Vec<(crate::ast::expr::Expression, crate::ast::expr::Expression)> {
    use crate::ast::decl::GateType;
    use crate::ast::expr::*;
    use crate::ast::Span;

    let mut result = Vec::new();

    for inst in &gi.instances {
        if inst.terminals.len() < 2 { continue; }

        match gi.gate_type {
            // N-input gates: first terminal is output, rest are inputs
            // out = in1 OP in2 OP in3 ...
            GateType::And | GateType::Nand | GateType::Or | GateType::Nor |
            GateType::Xor | GateType::Xnor => {
                let out = &inst.terminals[0];
                let inputs = &inst.terminals[1..];
                if inputs.is_empty() { continue; }

                let op = match gi.gate_type {
                    GateType::And | GateType::Nand => BinaryOp::BitAnd,
                    GateType::Or | GateType::Nor => BinaryOp::BitOr,
                    GateType::Xor | GateType::Xnor => BinaryOp::BitXor,
                    _ => unreachable!(),
                };

                // Build chain: in1 OP in2 OP in3 ...
                let mut expr = inputs[0].clone();
                for inp in &inputs[1..] {
                    expr = Expression::new(
                        ExprKind::Binary { op, left: Box::new(expr), right: Box::new(inp.clone()) },
                        Span::dummy(),
                    );
                }

                // For nand/nor/xnor: invert the result
                if matches!(gi.gate_type, GateType::Nand | GateType::Nor | GateType::Xnor) {
                    expr = Expression::new(
                        ExprKind::Unary { op: UnaryOp::BitNot, operand: Box::new(expr) },
                        Span::dummy(),
                    );
                }

                result.push((out.clone(), expr));
            }

            // buf: output = input (may have multiple outputs)
            // buf (out1, out2, ..., in)  — last terminal is input
            GateType::Buf => {
                let input = inst.terminals.last().unwrap();
                for out in &inst.terminals[..inst.terminals.len() - 1] {
                    result.push((out.clone(), input.clone()));
                }
            }

            // not: output = ~input (may have multiple outputs)
            // not (out1, out2, ..., in)
            GateType::Not => {
                let input = inst.terminals.last().unwrap();
                let inv = Expression::new(
                    ExprKind::Unary { op: UnaryOp::BitNot, operand: Box::new(input.clone()) },
                    Span::dummy(),
                );
                for out in &inst.terminals[..inst.terminals.len() - 1] {
                    result.push((out.clone(), inv.clone()));
                }
            }

            // bufif0/bufif1/notif0/notif1: tri-state — approximate as pass-through
            // bufif1 (out, in, ctrl) → assign out = ctrl ? in : 1'bz
            // For simulation simplicity, treat as: out = in (ignoring the enable)
            GateType::Bufif0 | GateType::Bufif1 | GateType::Notif0 | GateType::Notif1 => {
                if inst.terminals.len() >= 3 {
                    let out = &inst.terminals[0];
                    let input = &inst.terminals[1];
                    let _ctrl = &inst.terminals[2];
                    let expr = if matches!(gi.gate_type, GateType::Notif0 | GateType::Notif1) {
                        Expression::new(
                            ExprKind::Unary { op: UnaryOp::BitNot, operand: Box::new(input.clone()) },
                            Span::dummy(),
                        )
                    } else {
                        input.clone()
                    };
                    result.push((out.clone(), expr));
                }
            }
        }
    }

    result
}

fn make_ident_expr(name: &str) -> crate::ast::expr::Expression {
    use crate::ast::expr::*;
    use crate::ast::{Identifier, Span};
    Expression::new(
        ExprKind::Ident(HierarchicalIdentifier {
            root: None,
            path: vec![HierPathSegment {
                name: Identifier { name: name.to_string(), span: Span::dummy() },
                selects: Vec::new(),
            }],
            span: Span::dummy(),
            cached_signal_id: std::cell::Cell::new(None),
        }),
        Span::dummy(),
    )
}

/// Rewrite all identifiers in an expression with prefix (used for parent-scope port connections).
fn rewrite_expr_all(
    expr: &crate::ast::expr::Expression,
    prefix: &str,
) -> crate::ast::expr::Expression {
    // Create an "everything" local_names set by using a set that always matches
    let all = AllNames;
    rewrite_expr_impl(expr, prefix, &HashMap::new(), &all)
}

/// Rewrite an expression: prefix only local signal identifiers with the instance prefix.
fn rewrite_expr(
    expr: &crate::ast::expr::Expression,
    prefix: &str,
    port_map: &HashMap<String, crate::ast::expr::Expression>,
    local_names: &std::collections::HashSet<String>,
) -> crate::ast::expr::Expression {
    rewrite_expr_impl(expr, prefix, port_map, local_names)
}

/// Trait for checking if a name should be prefixed.
trait NameSet { fn contains_name(&self, name: &str) -> bool; }
impl NameSet for std::collections::HashSet<String> { fn contains_name(&self, name: &str) -> bool { self.contains(name) } }
struct AllNames;
impl NameSet for AllNames { fn contains_name(&self, _: &str) -> bool { true } }

fn rewrite_expr_impl(
    expr: &crate::ast::expr::Expression,
    prefix: &str,
    port_map: &HashMap<String, crate::ast::expr::Expression>,
    local_names: &dyn NameSet,
) -> crate::ast::expr::Expression {
    use crate::ast::expr::*;
    use crate::ast::{Identifier, Span};

    let new_kind = match &expr.kind {
        ExprKind::Ident(hier) => {
            let name = hier.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
            if local_names.contains_name(name) {
                let new_name = format!("{}{}", prefix, name);
                ExprKind::Ident(HierarchicalIdentifier {
                    root: None,
                    path: vec![HierPathSegment {
                        name: Identifier { name: new_name, span: Span::dummy() },
                        selects: hier.path.last().map(|s| s.selects.clone()).unwrap_or_default(),
                    }],
                    span: Span::dummy(),
                    cached_signal_id: std::cell::Cell::new(None),
                })
            } else {
                // Not a local signal (e.g., enum constant) — keep as-is
                expr.kind.clone()
            }
        }
        ExprKind::Binary { op, left, right } => ExprKind::Binary {
            op: *op,
            left: Box::new(rewrite_expr_impl(left, prefix, port_map, local_names)),
            right: Box::new(rewrite_expr_impl(right, prefix, port_map, local_names)),
        },
        ExprKind::Unary { op, operand } => ExprKind::Unary {
            op: *op,
            operand: Box::new(rewrite_expr_impl(operand, prefix, port_map, local_names)),
        },
        ExprKind::Conditional { condition, then_expr, else_expr } => ExprKind::Conditional {
            condition: Box::new(rewrite_expr_impl(condition, prefix, port_map, local_names)),
            then_expr: Box::new(rewrite_expr_impl(then_expr, prefix, port_map, local_names)),
            else_expr: Box::new(rewrite_expr_impl(else_expr, prefix, port_map, local_names)),
        },
        ExprKind::Concatenation(parts) => ExprKind::Concatenation(
            parts.iter().map(|p| rewrite_expr_impl(p, prefix, port_map, local_names)).collect()
        ),
        ExprKind::Replication { count, exprs } => ExprKind::Replication {
            count: Box::new(rewrite_expr_impl(count, prefix, port_map, local_names)),
            exprs: exprs.iter().map(|e| rewrite_expr_impl(e, prefix, port_map, local_names)).collect(),
        },
        ExprKind::Index { expr: e, index } => ExprKind::Index {
            expr: Box::new(rewrite_expr_impl(e, prefix, port_map, local_names)),
            index: Box::new(rewrite_expr_impl(index, prefix, port_map, local_names)),
        },
        ExprKind::RangeSelect { expr: e, kind, left, right } => ExprKind::RangeSelect {
            expr: Box::new(rewrite_expr_impl(e, prefix, port_map, local_names)),
            kind: *kind,
            left: Box::new(rewrite_expr_impl(left, prefix, port_map, local_names)),
            right: Box::new(rewrite_expr_impl(right, prefix, port_map, local_names)),
        },
        ExprKind::Paren(inner) => ExprKind::Paren(Box::new(rewrite_expr_impl(inner, prefix, port_map, local_names))),
        ExprKind::Call { func, args } => ExprKind::Call {
            func: Box::new(rewrite_expr_impl(func, prefix, port_map, local_names)),
            args: args.iter().map(|a| rewrite_expr_impl(a, prefix, port_map, local_names)).collect(),
        },
        ExprKind::SystemCall { name, args } => ExprKind::SystemCall {
            name: name.clone(),
            args: args.iter().map(|a| rewrite_expr_impl(a, prefix, port_map, local_names)).collect(),
        },
        // Literals and constants pass through unchanged
        other => other.clone(),
    };
    Expression::new(new_kind, expr.span)
}

/// Rewrite a statement: prefix identifiers.
/// Rewrite signal names inside a TimingControl during module inlining.
fn rewrite_timing_control(
    control: &crate::ast::stmt::TimingControl,
    prefix: &str,
    port_map: &HashMap<String, crate::ast::expr::Expression>,
    local_names: &std::collections::HashSet<String>,
) -> crate::ast::stmt::TimingControl {
    use crate::ast::stmt::*;
    match control {
        TimingControl::Delay(expr) => TimingControl::Delay(rewrite_expr(expr, prefix, port_map, local_names)),
        TimingControl::Event(ec) => TimingControl::Event(match ec {
            EventControl::Star | EventControl::ParenStar => ec.clone(),
            EventControl::Identifier(id) => {
                if local_names.contains(&id.name) {
                    EventControl::Identifier(crate::ast::Identifier {
                        name: format!("{}{}", prefix, id.name),
                        span: id.span,
                    })
                } else {
                    ec.clone()
                }
            }
            EventControl::EventExpr(exprs) => {
                EventControl::EventExpr(exprs.iter().map(|ee| EventExpr {
                    edge: ee.edge,
                    expr: rewrite_expr(&ee.expr, prefix, port_map, local_names),
                    iff: ee.iff.as_ref().map(|e| rewrite_expr(e, prefix, port_map, local_names)),
                    span: ee.span,
                }).collect())
            }
        }),
    }
}

fn rewrite_stmt(
    stmt: &crate::ast::stmt::Statement,
    prefix: &str,
    port_map: &HashMap<String, crate::ast::expr::Expression>,
    local_names: &std::collections::HashSet<String>,
) -> crate::ast::stmt::Statement {
    use crate::ast::stmt::*;
    let new_kind = match &stmt.kind {
        StatementKind::BlockingAssign { lvalue, rvalue } => StatementKind::BlockingAssign {
            lvalue: rewrite_expr(lvalue, prefix, port_map, local_names),
            rvalue: rewrite_expr(rvalue, prefix, port_map, local_names),
        },
        StatementKind::NonblockingAssign { lvalue, delay, rvalue } => StatementKind::NonblockingAssign {
            lvalue: rewrite_expr(lvalue, prefix, port_map, local_names),
            delay: delay.as_ref().map(|d| rewrite_expr(d, prefix, port_map, local_names)),
            rvalue: rewrite_expr(rvalue, prefix, port_map, local_names),
        },
        StatementKind::If { unique_priority, condition, then_stmt, else_stmt } => StatementKind::If {
            unique_priority: *unique_priority,
            condition: rewrite_expr(condition, prefix, port_map, local_names),
            then_stmt: Box::new(rewrite_stmt(then_stmt, prefix, port_map, local_names)),
            else_stmt: else_stmt.as_ref().map(|s| Box::new(rewrite_stmt(s, prefix, port_map, local_names))),
        },
        StatementKind::SeqBlock { name, stmts } => StatementKind::SeqBlock {
            name: name.clone(),
            stmts: stmts.iter().map(|s| rewrite_stmt(s, prefix, port_map, local_names)).collect(),
        },
        StatementKind::Case { unique_priority, kind, expr, items } => StatementKind::Case {
            unique_priority: *unique_priority,
            kind: *kind,
            expr: rewrite_expr(expr, prefix, port_map, local_names),
            items: items.iter().map(|ci| CaseItem {
                patterns: ci.patterns.iter().map(|p| rewrite_expr(p, prefix, port_map, local_names)).collect(),
                is_default: ci.is_default,
                stmt: rewrite_stmt(&ci.stmt, prefix, port_map, local_names),
                span: ci.span,
            }).collect(),
        },
        StatementKind::Expr(e) => StatementKind::Expr(rewrite_expr(e, prefix, port_map, local_names)),
        StatementKind::TimingControl { control, stmt: inner } => StatementKind::TimingControl {
            control: rewrite_timing_control(control, prefix, port_map, local_names),
            stmt: Box::new(rewrite_stmt(inner, prefix, port_map, local_names)),
        },
        StatementKind::For { init, condition, step, body } => StatementKind::For {
            init: init.iter().map(|fi| match fi {
                ForInit::Assign { lvalue, rvalue } => ForInit::Assign {
                    lvalue: rewrite_expr(lvalue, prefix, port_map, local_names),
                    rvalue: rewrite_expr(rvalue, prefix, port_map, local_names),
                },
                ForInit::VarDecl { data_type, name, init: init_expr } => ForInit::VarDecl {
                    data_type: data_type.clone(),
                    name: name.clone(),
                    init: rewrite_expr(init_expr, prefix, port_map, local_names),
                },
            }).collect(),
            condition: condition.as_ref().map(|c| rewrite_expr(c, prefix, port_map, local_names)),
            step: step.iter().map(|s| rewrite_expr(s, prefix, port_map, local_names)).collect(),
            body: Box::new(rewrite_stmt(body, prefix, port_map, local_names)),
        },
        StatementKind::While { condition, body } => StatementKind::While {
            condition: rewrite_expr(condition, prefix, port_map, local_names),
            body: Box::new(rewrite_stmt(body, prefix, port_map, local_names)),
        },
        StatementKind::Forever { body } => StatementKind::Forever {
            body: Box::new(rewrite_stmt(body, prefix, port_map, local_names)),
        },
        other => other.clone(),
    };
    Statement::new(new_kind, stmt.span)
}
