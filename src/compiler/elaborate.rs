//! Elaborator: converts parsed AST into a flat simulation model.
//! Resolves net/variable declarations, continuous assigns, always blocks.

use ahash::AHashMap as HashMap;
use ahash::AHashSet as HashSet;
use crate::ast::{Identifier, Span};
use crate::ast::decl::*;
use crate::ast::module::*;
use crate::ast::types::*;
use crate::ast::expr::*;
use crate::ast::stmt::*;
use super::value::Value;

/// A resolved signal in the simulation model.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Signal {
    pub name: String,
    pub width: u32,
    pub is_signed: bool,
    pub is_real: bool,
    pub is_const: bool,
    pub direction: Option<PortDirection>,
    pub value: Value,
    /// Name of the data type (e.g. class name).
    pub type_name: Option<String>,
}

/// A continuous assignment: assign lhs = rhs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContinuousAssignment {
    pub lhs: Expression,
    pub rhs: Expression,
}

/// An always block for combinatorial logic.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AlwaysBlock {
    pub kind: AlwaysKind,
    pub stmt: Statement,
}

/// An initial block for testbench.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InitialBlock {
    pub stmt: Statement,
}

/// Elaborated class definition.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ElaboratedClass {
    pub name: String,
    pub extends: Option<String>,
    pub properties: HashMap<String, Signal>,
    pub methods: HashMap<String, ClassMethod>,
    /// Properties marked as 'rand' or 'randc'.
    pub random_properties: HashSet<String>,
    /// Properties marked specifically as 'randc' (cyclic random).
    #[serde(default)]
    pub randc_properties: HashSet<String>,
    /// Constraints: name -> constraint declaration.
    pub constraints: HashMap<String, ClassConstraint>,
    /// Class parameters with default values, in declaration order.
    /// `(name, default_value_expr)`.
    pub param_defaults: Vec<(String, Option<crate::ast::expr::Expression>)>,
    /// `interface class` declaration — cannot be instantiated.
    #[serde(default)]
    pub is_interface: bool,
    /// Abstract (virtual) class — declared with `virtual class`. Cannot be instantiated
    /// directly.
    #[serde(default)]
    pub is_virtual: bool,
    /// Has at least one `pure virtual` method prototype.
    #[serde(default)]
    pub has_pure_virtual: bool,
    /// Names listed in the `implements` clause.
    #[serde(default)]
    pub implements: Vec<String>,
    /// Names of type parameters declared on the class.
    #[serde(default)]
    pub type_param_names: Vec<String>,
    /// Typedef names declared in the class body.
    #[serde(default)]
    pub typedef_names: Vec<String>,
}

/// DPI import metadata used by the simulator for foreign-call dispatch.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DpiImportSpec {
    pub c_name: String,
    pub property: Option<DPIProperty>,
    pub proto: DPIProto,
}

pub fn elaborate_class(c: &ClassDeclaration) -> ElaboratedClass {
    let mut properties = HashMap::new();
    let mut methods = HashMap::new();
    let mut random_properties = HashSet::new();
    let mut randc_properties = HashSet::new();
    let mut constraints = HashMap::new();
    for item in &c.items {
        match item {
            ClassItem::Property(p) => {
                let width = resolve_type_width(&p.data_type, None, None);
                let is_signed = is_type_signed(&p.data_type);
                let is_rand = p.qualifiers.contains(&ClassQualifier::Rand) || p.qualifiers.contains(&ClassQualifier::Randc);
                let is_randc = p.qualifiers.contains(&ClassQualifier::Randc);
                let is_const = p.qualifiers.contains(&ClassQualifier::Const);
                let is_real = is_type_real(&p.data_type);
                for decl in &p.declarators {
                    let mut v = if let Some(init) = &decl.init {
                        let mut val = eval_const_expr_val(init, &HashMap::new()).resize(width);
                        if is_real { val = Value::from_f64(val.to_f64()); }
                        val
                    } else if is_real {
                        Value::from_f64(0.0)
                    } else {
                        Value::new(width)
                    };
                    if is_signed { v.is_signed = true; }
                    properties.insert(decl.name.name.clone(), Signal { is_const: false,
                        name: decl.name.name.clone(),
                        width,
                        is_signed,
                        is_real,
                        direction: None,
                        value: v,
                        type_name: get_type_name(&p.data_type),
                    });
                    if is_rand {
                        random_properties.insert(decl.name.name.clone());
                    }
                    if is_randc {
                        randc_properties.insert(decl.name.name.clone());
                    }
                }
            }
            ClassItem::Method(m) => {
                let name = match &m.kind {
                    ClassMethodKind::Function(f) => f.name.name.name.clone(),
                    ClassMethodKind::Task(t) => t.name.name.name.clone(),
                    ClassMethodKind::PureVirtual(f) => f.name.name.name.clone(),
                    ClassMethodKind::Extern(f) => f.name.name.name.clone(),
                };
                methods.insert(name, m.clone());
            }
            ClassItem::Constraint(con) => {
                constraints.insert(con.name.name.clone(), con.clone());
            }
            _ => {}
        }
    }
    // Collect class parameters (name + optional default expression).
    let mut param_defaults: Vec<(String, Option<crate::ast::expr::Expression>)> = Vec::new();
    for p in &c.params {
        if let crate::ast::decl::ParameterKind::Data { assignments, .. } = &p.kind {
            for a in assignments {
                param_defaults.push((a.name.name.clone(), a.init.clone()));
            }
        }
    }
    let has_pure_virtual = c.items.iter().any(|it|
        matches!(it, ClassItem::Method(m) if matches!(m.kind, ClassMethodKind::PureVirtual(_))));
    let mut type_param_names = Vec::new();
    for p in &c.params {
        if let crate::ast::decl::ParameterKind::Type { assignments } = &p.kind {
            for a in assignments { type_param_names.push(a.name.name.clone()); }
        }
    }
    ElaboratedClass {
        name: c.name.name.clone(),
        extends: c.extends.as_ref().map(|e| e.name.name.clone()),
        properties,
        methods,
        random_properties,
        randc_properties,
        constraints,
        param_defaults,
        is_interface: c.is_interface,
        is_virtual: c.virtual_kw,
        has_pure_virtual,
        implements: c.implements.iter().map(|i| i.name.clone()).collect(),
        type_param_names,
        typedef_names: c.items.iter().filter_map(|it| match it {
            ClassItem::Typedef(td) => Some(td.name.name.clone()),
            _ => None,
        }).collect(),
    }
}

/// Elaborated module ready for simulation.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
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
    pub typedef_types: HashMap<String, DataType>,
    /// Array declarations: base_name -> (lo_index, hi_index, element_width)
    pub arrays: HashMap<String, (i64, i64, u32)>,
    /// Associative arrays: name -> true if string-keyed
    pub associative_arrays: HashMap<String, bool>,
    /// Class definitions: name -> elaborated class.
    pub classes: HashMap<String, ElaboratedClass>,
    /// Covergroup definitions: name -> AST declaration.
    pub covergroups: HashMap<String, CovergroupDeclaration>,
    /// Module-level function declarations.
    pub functions: HashMap<String, FunctionDeclaration>,
    /// Module-level task declarations.
    pub tasks: HashMap<String, TaskDeclaration>,
    /// DPI imports by SV-visible symbol name.
    pub dpi_imports: HashMap<String, DpiImportSpec>,
    /// Clocking block definitions: name -> AST declaration.
    pub clocking_blocks: HashMap<String, ClockingDeclaration>,
    /// Let declarations visible in the elaborated scope.
    pub lets: HashMap<String, LetDeclaration>,
    /// Bound interface modport views: signal -> (member -> direction).
    pub modport_views: HashMap<String, HashMap<String, PortDirection>>,
    /// Clocking block signals: block name -> (signal -> direction).
    pub clocking_signal_dirs: HashMap<String, HashMap<String, PortDirection>>,
    /// Specify path delays: destination signal name -> delay (time units).
    pub specify_delays: HashMap<String, u64>,
    /// Associative array default values.
    pub assoc_defaults: HashMap<String, Expression>,
    /// Dynamic arrays / queues (size starts at 0, not pre-allocated range).
    pub dynamic_arrays: HashSet<String>,
    /// Arrays declared with descending range (e.g. [7:0])
    pub descending_arrays: HashSet<String>,
    /// Bounded queue max sizes: name -> max element count (i.e., $:N means N+1).
    pub queue_max_sizes: HashMap<String, u32>,
    /// 2D unpacked arrays: name -> ((dim1_lo,dim1_hi),(dim2_lo,dim2_hi),elem_width).
    pub arrays_2d: HashMap<String, ((i64, i64), (i64, i64), u32)>,
    pub packages: HashSet<String>,
    /// Names of declared sequences and properties (so `@name` event control resolves).
    pub sequences: HashSet<String>,
    /// Packed struct bit-field layout: container_name -> Vec<(member_name, lsb_offset, width)>.
    /// Members are stored by bit offset so MemberAccess can slice the container.
    pub packed_struct_fields: HashMap<String, Vec<(String, u32, u32)>>,
    /// Class-typed signal parameter overrides captured from `Type #(args) name;`
    /// declarations. Signal name -> positional type_args expressions.
    pub class_type_args: HashMap<String, Vec<Expression>>,
    /// N-dimensional unpacked array shapes (N >= 3): name → Vec of (lo, hi) per dim.
    pub arrays_nd: HashMap<String, (Vec<(i64, i64)>, u32)>,
    /// Parameter init expressions that couldn't be evaluated at elaboration time
    /// (e.g. contain function calls). Simulator re-evaluates these during init.
    pub deferred_param_exprs: Vec<(String, Expression)>,
    /// Names declared as nets (wire, supply0/1, tri, etc). Variables are everything else.
    /// Used to enforce §6.5 driver-conflict rules only against variables.
    #[serde(default)]
    pub nets: HashSet<String>,
    /// Out-of-class constraint definitions: `(class_name, constraint_name)`.
    #[serde(default)]
    pub out_of_class_constraints: HashSet<(String, String)>,
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
            typedef_types: HashMap::new(),
            arrays: HashMap::new(),
            associative_arrays: HashMap::new(),
            classes: HashMap::new(),
            covergroups: HashMap::new(),
            functions: HashMap::new(),
            tasks: HashMap::new(),
            dpi_imports: HashMap::new(),
            clocking_blocks: HashMap::new(),
            lets: HashMap::new(),
            modport_views: HashMap::new(),
            clocking_signal_dirs: HashMap::new(),
            specify_delays: HashMap::new(),
            assoc_defaults: HashMap::new(),
            dynamic_arrays: HashSet::new(),
            descending_arrays: HashSet::new(),
            queue_max_sizes: HashMap::new(),
            arrays_2d: HashMap::new(),
            packages: HashSet::new(),
            sequences: HashSet::new(),
            packed_struct_fields: HashMap::new(),
            class_type_args: HashMap::new(),
            arrays_nd: HashMap::new(),
            deferred_param_exprs: Vec::new(),
            nets: HashSet::new(),
            out_of_class_constraints: HashSet::new(),
        }
    }
}

fn expr_has_call(expr: &Expression) -> bool {
    use crate::ast::expr::ExprKind;
    match &expr.kind {
        ExprKind::Call { .. } => true,
        ExprKind::Binary { left, right, .. } => expr_has_call(left) || expr_has_call(right),
        ExprKind::Unary { operand, .. } => expr_has_call(operand),
        ExprKind::Paren(e) => expr_has_call(e),
        ExprKind::Conditional { condition, then_expr, else_expr } =>
            expr_has_call(condition) || expr_has_call(then_expr) || expr_has_call(else_expr),
        _ => false,
    }
}

/// A unified representation of a module or interface.
#[derive(Debug, Clone, Copy)]
pub enum Definition<'a> {
    Module(&'a ModuleDeclaration),
    Interface(&'a crate::ast::module::InterfaceDeclaration),
    Program(&'a crate::ast::module::ProgramDeclaration),
    Class(&'a crate::ast::decl::ClassDeclaration),
    Covergroup(&'a crate::ast::decl::CovergroupDeclaration),
    Package(&'a crate::ast::module::PackageDeclaration),
    Typedef(&'a crate::ast::decl::TypedefDeclaration),
}

impl<'a> Definition<'a> {
    pub fn name(&self) -> &str {
        match self {
            Definition::Module(m) => &m.name.name,
            Definition::Interface(i) => &i.name.name,
            Definition::Program(p) => &p.name.name,
            Definition::Class(c) => &c.name.name,
            Definition::Covergroup(cg) => &cg.name.name,
            Definition::Package(p) => &p.name.name,
            Definition::Typedef(t) => &t.name.name,
        }
    }

    pub fn params(&self) -> &[ParameterDeclaration] {
        match self {
            Definition::Module(m) => &m.params,
            Definition::Interface(i) => &i.params,
            Definition::Program(p) => &p.params,
            Definition::Class(c) => &c.params,
            Definition::Covergroup(_) | Definition::Package(_) | Definition::Typedef(_) => &[],
        }
    }

    pub fn ports(&self) -> &PortList {
        match self {
            Definition::Module(m) => &m.ports,
            Definition::Interface(i) => &i.ports,
            Definition::Program(p) => &p.ports,
            Definition::Class(_) | Definition::Covergroup(_) | Definition::Package(_) | Definition::Typedef(_) => &PortList::Empty,
        }
    }
        pub fn items(&self) -> &[ModuleItem] {
        match self {
        Definition::Module(m) => &m.items,
        Definition::Interface(i) => &i.items,
        Definition::Program(p) => &p.items,
        Definition::Class(_) | Definition::Covergroup(_) | Definition::Package(_) | Definition::Typedef(_) => &[],
        }
        }
        }

fn get_type_name(dt: &DataType) -> Option<String> {
    match dt {
        DataType::TypeReference { name, .. } => Some(name.name.name.clone()),
        DataType::Interface { name, .. } => Some(name.name.clone()),
        _ => None,
    }
}

fn dpi_proto_sv_name(proto: &DPIProto) -> String {
    match proto {
        DPIProto::Function(fd) => fd.name.name.name.clone(),
        DPIProto::Task(td) => td.name.name.name.clone(),
    }
}

fn register_dpi_import(di: &DPIImport, elab: &mut ElaboratedModule) -> Result<(), String> {
    let sv_name = dpi_proto_sv_name(&di.proto);
    if elab.dpi_imports.contains_key(&sv_name) {
        return Err(format!("Duplicate DPI import declaration '{}'", sv_name));
    }
    let c_name = di.c_name.clone().unwrap_or_else(|| sv_name.clone());
    elab.dpi_imports.insert(sv_name, DpiImportSpec {
        c_name,
        property: di.property,
        proto: di.proto.clone(),
    });
    Ok(())
}

fn is_const_expr(expr: &Expression, params: &HashMap<String, Value>) -> bool {
    match &expr.kind {
        ExprKind::Number(_) | ExprKind::StringLiteral(_) => true,
        ExprKind::Ident(hier) => {
            let name = hier.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
            params.contains_key(name)
        }
        ExprKind::Unary { operand, .. } => is_const_expr(operand, params),
        ExprKind::Binary { left, right, .. } => is_const_expr(left, params) && is_const_expr(right, params),
        ExprKind::Conditional { condition, then_expr, else_expr } => is_const_expr(condition, params) && is_const_expr(then_expr, params) && is_const_expr(else_expr, params),
        ExprKind::Concatenation(parts) => parts.iter().all(|p| is_const_expr(p, params)),
        ExprKind::Paren(inner) => is_const_expr(inner, params),
        _ => false, // Calls (new()) etc. are not constant
    }
}

/// Elaborate a module or interface declaration into a simulation model.
pub fn elaborate_module(
    module: Definition,
    param_overrides: &HashMap<String, Value>,
) -> Result<ElaboratedModule, String> {
    elaborate_module_with_defs(module, param_overrides, None, &[], &[])
}

pub fn process_typedef(td: &TypedefDeclaration, elab: &mut ElaboratedModule) {
    if let DataType::Enum(et) = &td.data_type {
        let base_width = et.base_type.as_ref()
            .map(|bt| resolve_type_width(bt, Some(&elab.parameters), Some(&elab.typedefs)))
            .unwrap_or(32);
        let mut next_val: u64 = 0;
        for member in &et.members {
            let val = if let Some(init) = &member.init {
                eval_const_expr(init, &elab.parameters)
            } else { next_val };
            next_val = val.wrapping_add(1);
            let v = Value::from_u64(val, base_width);
            elab.parameters.insert(member.name.name.clone(), v.clone());
            elab.signals.insert(member.name.name.clone(), Signal { is_const: false,
                name: member.name.name.clone(),
                width: base_width,
                is_signed: false,
                is_real: false,
                direction: None,
                value: v,
                type_name: None,
            });
        }
        // Register the typedef width
        elab.typedefs.insert(td.name.name.clone(), base_width);
    } else {
        // Non-enum typedef: resolve width from the underlying type
        let w = resolve_type_width(&td.data_type, Some(&elab.parameters), Some(&elab.typedefs));
        elab.typedefs.insert(td.name.name.clone(), w);
        elab.typedef_types.insert(td.name.name.clone(), td.data_type.clone());
    }
}

fn resolve_interface_modport_view(
    interface_name: &str,
    modport_name: &str,
    all_defs: Option<&HashMap<String, Definition>>,
) -> Option<HashMap<String, PortDirection>> {
    let defs = all_defs?;
    let idef = match defs.get(interface_name) {
        Some(Definition::Interface(i)) => i,
        _ => return None,
    };
    for item in &idef.items {
        if let ModuleItem::ModportDeclaration(md) = item {
            for mp in &md.items {
                if mp.name.name == modport_name {
                    let mut dirs = HashMap::new();
                    for p in &mp.ports {
                        dirs.insert(p.name.name.clone(), p.direction);
                    }
                    return Some(dirs);
                }
            }
        }
    }
    None
}

fn validate_class_constraint_expr(expr: &Expression, allowed: &HashSet<String>) -> Result<(), String> {
    match &expr.kind {
        ExprKind::Ident(hier) => {
            if hier.path.len() == 1 {
                let n = &hier.path[0].name.name;
                if n != "this" && n != "super" && n != "new" && !allowed.contains(n) {
                    return Err(format!("Undeclared identifier '{}' in class constraint", n));
                }
            }
        }
        ExprKind::Unary { operand, .. } => validate_class_constraint_expr(operand, allowed)?,
        ExprKind::Binary { left, right, .. } => {
            validate_class_constraint_expr(left, allowed)?;
            validate_class_constraint_expr(right, allowed)?;
        }
        ExprKind::Conditional { condition, then_expr, else_expr } => {
            validate_class_constraint_expr(condition, allowed)?;
            validate_class_constraint_expr(then_expr, allowed)?;
            validate_class_constraint_expr(else_expr, allowed)?;
        }
        ExprKind::Concatenation(parts) => {
            for p in parts {
                validate_class_constraint_expr(p, allowed)?;
            }
        }
        ExprKind::Replication { count, exprs } => {
            validate_class_constraint_expr(count, allowed)?;
            for e in exprs {
                validate_class_constraint_expr(e, allowed)?;
            }
        }
        ExprKind::Index { expr, index } => {
            validate_class_constraint_expr(expr, allowed)?;
            validate_class_constraint_expr(index, allowed)?;
        }
        ExprKind::RangeSelect { expr, left, right, .. } => {
            validate_class_constraint_expr(expr, allowed)?;
            validate_class_constraint_expr(left, allowed)?;
            validate_class_constraint_expr(right, allowed)?;
        }
        ExprKind::Inside { expr, ranges } => {
            validate_class_constraint_expr(expr, allowed)?;
            for r in ranges {
                validate_class_constraint_expr(r, allowed)?;
            }
        }
        ExprKind::Range(lo, hi) => {
            validate_class_constraint_expr(lo, allowed)?;
            validate_class_constraint_expr(hi, allowed)?;
        }
        ExprKind::Paren(inner) => validate_class_constraint_expr(inner, allowed)?,
        ExprKind::Call { func: _, args } => {
            // Don't validate the callee identifier: it resolves to a function/method
            // (including class methods, package functions, built-ins) that may not be
            // in the property-name allowed set.
            for a in args {
                validate_class_constraint_expr(a, allowed)?;
            }
        }
        ExprKind::SystemCall { args, .. } => {
            for a in args {
                validate_class_constraint_expr(a, allowed)?;
            }
        }
        ExprKind::MemberAccess { expr, .. } => validate_class_constraint_expr(expr, allowed)?,
        _ => {}
    }
    Ok(())
}

fn validate_constraint_item_names(item: &ConstraintItem, allowed: &HashSet<String>) -> Result<(), String> {
    match item {
        ConstraintItem::Expr(expr) => validate_class_constraint_expr(expr, allowed)?,
        ConstraintItem::Inside { expr, range, .. } => {
            validate_class_constraint_expr(expr, allowed)?;
            for r in range {
                match r {
                    ConstraintRange::Value(e) => validate_class_constraint_expr(e, allowed)?,
                    ConstraintRange::Range { lo, hi } => {
                        validate_class_constraint_expr(lo, allowed)?;
                        validate_class_constraint_expr(hi, allowed)?;
                    }
                }
            }
        }
        ConstraintItem::Implication { condition, constraint, .. } => {
            validate_class_constraint_expr(condition, allowed)?;
            validate_constraint_item_names(constraint, allowed)?;
        }
        ConstraintItem::IfElse { condition, then_item, else_item, .. } => {
            validate_class_constraint_expr(condition, allowed)?;
            validate_constraint_item_names(then_item, allowed)?;
            if let Some(ei) = else_item {
                validate_constraint_item_names(ei, allowed)?;
            }
        }
        ConstraintItem::Foreach { array, vars, item, .. } => {
            validate_class_constraint_expr(array, allowed)?;
            let mut inner = allowed.clone();
            for v in vars {
                if let Some(id) = v {
                    inner.insert(id.name.clone());
                }
            }
            validate_constraint_item_names(item, &inner)?;
        }
        ConstraintItem::Soft(inner) => validate_constraint_item_names(inner, allowed)?,
        ConstraintItem::Block(items) => {
            for it in items {
                validate_constraint_item_names(it, allowed)?;
            }
        }
        ConstraintItem::Solve { before, after, .. } => {
            for id in before {
                if !allowed.contains(&id.name) {
                    return Err(format!("Undeclared identifier '{}' in class constraint", id.name));
                }
            }
            for id in after {
                if !allowed.contains(&id.name) {
                    return Err(format!("Undeclared identifier '{}' in class constraint", id.name));
                }
            }
        }
    }
    Ok(())
}

fn collect_class_member_names(
    c: &ClassDeclaration,
    all_defs: Option<&HashMap<String, Definition>>,
    allowed: &mut HashSet<String>,
    seen: &mut HashSet<String>,
) {
    if !seen.insert(c.name.name.clone()) {
        return;
    }
    for item in &c.items {
        match item {
            ClassItem::Property(p) => {
                for d in &p.declarators {
                    allowed.insert(d.name.name.clone());
                }
            }
            ClassItem::Parameter(pd) => match &pd.kind {
                ParameterKind::Data { assignments, .. } => {
                    for a in assignments {
                        allowed.insert(a.name.name.clone());
                    }
                }
                ParameterKind::Type { assignments } => {
                    for a in assignments {
                        allowed.insert(a.name.name.clone());
                    }
                }
            },
            ClassItem::Method(m) => {
                let name = match &m.kind {
                    ClassMethodKind::Function(f) => &f.name.name.name,
                    ClassMethodKind::Task(t) => &t.name.name.name,
                    ClassMethodKind::PureVirtual(f) => &f.name.name.name,
                    ClassMethodKind::Extern(f) => &f.name.name.name,
                };
                allowed.insert(name.clone());
            }
            ClassItem::Typedef(td) => {
                allowed.insert(td.name.name.clone());
            }
            _ => {}
        }
    }
    for p in &c.params {
        match &p.kind {
            ParameterKind::Data { assignments, .. } => {
                for a in assignments {
                    allowed.insert(a.name.name.clone());
                }
            }
            ParameterKind::Type { assignments } => {
                for a in assignments {
                    allowed.insert(a.name.name.clone());
                }
            }
        }
    }
    if let Some(ext) = &c.extends {
        if let Some(defs) = all_defs {
            if let Some(Definition::Class(parent)) = defs.get(&ext.name.name) {
                collect_class_member_names(parent, all_defs, allowed, seen);
            }
        }
    }
}

fn validate_class_constraints(
    c: &ClassDeclaration,
    all_defs: Option<&HashMap<String, Definition>>,
) -> Result<(), String> {
    let mut allowed = HashSet::new();
    let mut seen = HashSet::new();
    collect_class_member_names(c, all_defs, &mut allowed, &mut seen);
    for item in &c.items {
        if let ClassItem::Constraint(con) = item {
            for it in &con.items {
                validate_constraint_item_names(it, &allowed)?;
            }
        }
    }
    Ok(())
}

pub fn elaborate_module_with_defs(
    module: Definition,
    param_overrides: &HashMap<String, Value>,
    all_defs: Option<&HashMap<String, Definition>>,
    top_level_imports: &[ImportDeclaration],
    top_level_lets: &[LetDeclaration],
) -> Result<ElaboratedModule, String> {
    let mut elab = ElaboratedModule::new(module.name().to_string());

    // Process top-level typedefs and other global definitions from all_defs
    if let Some(defs) = all_defs {
        for def in defs.values() {
            match def {
                Definition::Typedef(td) => { process_typedef(td, &mut elab); }
                Definition::Class(c) => {
                    validate_class_constraints(c, Some(defs))?;
                    elab.classes.insert(c.name.name.clone(), elaborate_class(c));
                }
                Definition::Covergroup(cg) => { elab.covergroups.insert(cg.name.name.clone(), (*cg).clone()); }
                Definition::Package(p) => {
                    elab.packages.insert(p.name.name.clone());
                    // Hoist package functions/tasks for `pkg::f(...)` resolution.
                    // Skip framework packages with very large APIs.
                    if p.name.name != "uvm_pkg" {
                        for item in &p.items {
                            match item {
                                crate::ast::decl::PackageItem::Function(f) => {
                                    elab.functions.entry(f.name.name.name.clone()).or_insert_with(|| f.clone());
                                }
                                crate::ast::decl::PackageItem::Task(t) => {
                                    elab.tasks.entry(t.name.name.name.clone()).or_insert_with(|| t.clone());
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Process top-level imports
    for imp in top_level_imports {
        if let Some(defs) = all_defs {
            process_import(imp, &mut elab, defs)?;
        }
    }

    for l in top_level_lets {
        elab.lets.insert(l.name.name.clone(), l.clone());
    }

    // Process parameters
    for param in module.params() {
        if let ParameterKind::Data { data_type, assignments } = &param.kind {
            for assign in assignments {
                let mut width = resolve_type_width(data_type, Some(&elab.parameters), Some(&elab.typedefs));
                let mut signed = is_type_signed(data_type);
                let mut is_real = is_type_real(data_type);

                // IEEE 1800-2017 §6.20.2: Parameters with implicit type (no explicit type)
                // default to 32-bit signed integer.
                if matches!(data_type, DataType::Implicit { dimensions, .. } if dimensions.is_empty()) {
                    // Check if the initialization value is real. If so, parameter is real.
                    let init_is_real = if let Some(override_val) = param_overrides.get(&assign.name.name) {
                        override_val.is_real
                    } else if let Some(init) = &assign.init {
                        eval_const_expr_val(init, &elab.parameters).is_real
                    } else { false };

                    if init_is_real {
                        width = 64;
                        is_real = true;
                    } else {
                        width = 32;
                        signed = true;
                    }
                }

                let mut val = if let Some(override_val) = param_overrides.get(&assign.name.name) {
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

                if is_real {
                    val = Value::from_f64(val.to_f64());
                }

                elab.parameters.insert(assign.name.name.clone(), val);
            }
        }
    }

    // Process ports
    match module.ports() {
        PortList::Ansi(ports) => {
            for port in ports {
                let modport_view = match port.data_type.as_ref() {
                    Some(DataType::Interface { name, modport: Some(mp), .. }) => {
                        resolve_interface_modport_view(&name.name, &mp.name, all_defs)
                    }
                    _ => None,
                };
                let width = port.data_type.as_ref()
                    .map(|dt| resolve_type_width(dt, Some(&elab.parameters), Some(&elab.typedefs)))
                    .unwrap_or(1);
                let is_signed = port.data_type.as_ref()
                    .map(|dt| is_type_signed(dt))
                    .unwrap_or(false);
                let is_real = port.data_type.as_ref().map(is_type_real).unwrap_or(false);
                let sig = Signal { is_const: false,
                    name: port.name.name.clone(),
                    width,
                    is_signed,
                    is_real,
                    direction: port.direction,
                    value: if is_real { Value::from_f64(0.0) } else { Value::new(width) },
                    type_name: port.data_type.as_ref().and_then(get_type_name),
                };
                elab.port_order.push(port.name.name.clone());
                elab.signals.insert(port.name.name.clone(), sig);
                if let Some(view) = modport_view {
                    elab.modport_views.insert(port.name.name.clone(), view);
                }
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

    // Process items
    if let Definition::Package(p) = module {
        for item in &p.items {
            match item {
                crate::ast::decl::PackageItem::Typedef(td) => {
                    process_typedef(td, &mut elab);
                }
                crate::ast::decl::PackageItem::Parameter(pd) => {
                    if let ParameterKind::Data { data_type, assignments } = &pd.kind {
                        let width = resolve_type_width(data_type, Some(&elab.parameters), Some(&elab.typedefs));
                        let is_signed = is_type_signed(data_type);
                        for assign in assignments {
                            if let Some(init) = &assign.init {
                                let mut v = eval_const_expr_val(init, &elab.parameters).resize(width);
                                if is_signed { v.is_signed = true; }
                                elab.parameters.insert(assign.name.name.clone(), v);
                            }
                        }
                    }
                }
                crate::ast::decl::PackageItem::Class(c) => {
                    validate_class_constraints(c, all_defs)?;
                    elab.classes.insert(c.name.name.clone(), elaborate_class(c));
                }
                crate::ast::decl::PackageItem::Let(l) => {
                    elab.lets.insert(l.name.name.clone(), l.clone());
                }
                crate::ast::decl::PackageItem::DPIImport(di) => {
                    register_dpi_import(di, &mut elab)?;
                }
                _ => {}
            }
        }
    }

    // Pre-pass: collect user-defined nettype names so variables declared with
    // those types can be classified as nets (§6.6.7 — nettype resolution permits
    // multiple continuous drivers). Also register each nettype's width as a
    // typedef so TypeReference lookups resolve correctly.
    let mut user_nettypes: HashSet<String> = HashSet::new();
    for item in module.items() {
        if let ModuleItem::NettypeDeclaration(nd) = item {
            user_nettypes.insert(nd.name.name.clone());
            let w = resolve_type_width(&nd.data_type, Some(&elab.parameters), Some(&elab.typedefs));
            elab.typedefs.insert(nd.name.name.clone(), w);
        }
    }

    for item in module.items() {
        match item {
            ModuleItem::PortDeclaration(pd) => {
                let port_modport_view = match &pd.data_type {
                    DataType::Interface { name, modport: Some(mp), .. } => {
                        resolve_interface_modport_view(&name.name, &mp.name, all_defs)
                    }
                    _ => None,
                };
                let width = resolve_type_width(&pd.data_type, Some(&elab.parameters), Some(&elab.typedefs));
                let is_signed = is_type_signed(&pd.data_type);
                let is_real = is_type_real(&pd.data_type);
                for decl in &pd.declarators {
                    if elab.signals.contains_key(&decl.name.name) || elab.parameters.contains_key(&decl.name.name) {
                        return Err(format!("Duplicate declaration of '{}'", decl.name.name));
                    }
                    let sig = Signal { is_const: false,
                        name: decl.name.name.clone(),
                        width,
                        is_signed,
                        is_real,
                        direction: Some(pd.direction),
                        value: if is_real { Value::from_f64(0.0) } else { Value::new(width) },
                        type_name: get_type_name(&pd.data_type),
                    };
                    if !elab.port_order.contains(&decl.name.name) {
                        elab.port_order.push(decl.name.name.clone());
                    }
                    elab.signals.insert(decl.name.name.clone(), sig);
                    if let Some(view) = &port_modport_view {
                        elab.modport_views.insert(decl.name.name.clone(), view.clone());
                    }
                }
            }
            ModuleItem::NetDeclaration(nd) => {
                let width = resolve_type_width(&nd.data_type, Some(&elab.parameters), Some(&elab.typedefs));
                let is_signed = is_type_signed(&nd.data_type);
                let is_real = is_type_real(&nd.data_type);
                for decl in &nd.declarators {
                    if elab.signals.contains_key(&decl.name.name) || elab.parameters.contains_key(&decl.name.name) {
                        return Err(format!("Duplicate declaration of '{}'", decl.name.name));
                    }
                    let w = width_with_unpacked_dims(&decl.dimensions, width);
                    // supply0 → constant 0, supply1 → constant 1
                    let init_value = match nd.net_type {
                        NetType::Supply0 => Value::zero(w),
                        NetType::Supply1 => Value::ones(w),
                        _ => if is_real { Value::from_f64(0.0) } else { Value::new(w) },
                    };
                    let sig = Signal { is_const: false,
                        name: decl.name.name.clone(),
                        width: w,
                        is_signed,
                        is_real,
                        direction: None,
                        value: init_value,
                        type_name: get_type_name(&nd.data_type),
                    };
                    elab.signals.insert(decl.name.name.clone(), sig);
                    elab.nets.insert(decl.name.name.clone());
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
                // User-defined nettype → classify as net (allow multiple continuous drivers).
                if let DataType::TypeReference { name, .. } = &dd.data_type {
                    if user_nettypes.contains(&name.name.name) {
                        for decl in &dd.declarators {
                            elab.nets.insert(decl.name.name.clone());
                        }
                    }
                }
                let data_modport_view = match &dd.data_type {
                    DataType::Interface { name, modport: Some(mp), .. } => {
                        resolve_interface_modport_view(&name.name, &mp.name, all_defs)
                    }
                    _ => None,
                };
                let width = match &dd.data_type {
                    DataType::TypeReference { name, .. } => {
                        elab.typedefs.get(&name.name.name).copied().unwrap_or(resolve_type_width(&dd.data_type, Some(&elab.parameters), Some(&elab.typedefs)))
                    }
                    _ => resolve_type_width(&dd.data_type, Some(&elab.parameters), Some(&elab.typedefs)),
                };
                if let DataType::TypeReference { type_args, .. } = &dd.data_type {
                    if !type_args.is_empty() {
                        for decl in &dd.declarators {
                            elab.class_type_args.insert(decl.name.name.clone(), type_args.clone());
                        }
                    }
                }
                let is_signed = is_type_signed(&dd.data_type);
                for decl in &dd.declarators {
                    if elab.signals.contains_key(&decl.name.name) || elab.parameters.contains_key(&decl.name.name) {
                        return Err(format!("Duplicate declaration of '{}'", decl.name.name));
                    }
                    if let Some(UnpackedDimension::Associative { data_type: key_dt, .. }) = decl.dimensions.first() {
                        let is_string_key = key_dt.as_ref().map_or(false, |dt| matches!(dt.as_ref(), DataType::Simple { kind: SimpleType::String, .. }));
                        elab.associative_arrays.insert(decl.name.name.clone(), is_string_key);
                        if let Some(init_expr) = &decl.init {
                            if let ExprKind::AssignmentPattern(items) = &init_expr.kind {
                                for item in items {
                                    if let crate::ast::expr::AssignmentPatternItem::Default(def_expr) = item {
                                        elab.assoc_defaults.insert(decl.name.name.clone(), def_expr.clone());
                                    }
                                }
                            }
                        }
                    }
                    let is_dynamic_dim = decl.dimensions.first().map_or(false, |d| matches!(d, UnpackedDimension::Unsized(_) | UnpackedDimension::Queue { .. }));
                    if is_dynamic_dim {
                        elab.dynamic_arrays.insert(decl.name.name.clone());
                    }
                    if let Some(UnpackedDimension::Queue { max_size: Some(ms), .. }) = decl.dimensions.first() {
                        let n = const_eval_i64_with_params(ms, Some(&elab.parameters)).unwrap_or(0);
                        if n >= 0 { elab.queue_max_sizes.insert(decl.name.name.clone(), (n + 1) as u32); }
                    }
                    // Check for 2D unpacked array (e.g., mem [0:1023][0:3])
                    if decl.dimensions.len() == 2 {
                        let r1 = if let UnpackedDimension::Range { left, right, .. } = &decl.dimensions[0] {
                            let l = const_eval_i64_with_params(left, Some(&elab.parameters)).unwrap_or(0);
                            let r = const_eval_i64_with_params(right, Some(&elab.parameters)).unwrap_or(0);
                            Some((l.min(r), l.max(r)))
                        } else { None };
                        let r2 = if let UnpackedDimension::Range { left, right, .. } = &decl.dimensions[1] {
                            let l = const_eval_i64_with_params(left, Some(&elab.parameters)).unwrap_or(0);
                            let r = const_eval_i64_with_params(right, Some(&elab.parameters)).unwrap_or(0);
                            Some((l.min(r), l.max(r)))
                        } else { None };
                        if let (Some((lo1, hi1)), Some((lo2, hi2))) = (r1, r2) {
                            elab.arrays_2d.insert(decl.name.name.clone(), ((lo1, hi1), (lo2, hi2), width));
                            let is_real = is_type_real(&dd.data_type);
                            for i in lo1..=hi1 {
                                for j in lo2..=hi2 {
                                    let elem_name = format!("{}[{}][{}]", decl.name.name, i, j);
                                    let sig = Signal { is_const: dd.const_kw,
                                        name: elem_name.clone(),
                                        width,
                                        is_signed,
                                        is_real,
                                        direction: None,
                                        value: default_value_for_type(&dd.data_type, width),
                                        type_name: get_type_name(&dd.data_type),
                                    };
                                    elab.signals.insert(elem_name, sig);
                                }
                            }
                            continue;
                        }
                    }
                    // Check for N-dimensional unpacked array (N >= 3)
                    if decl.dimensions.len() >= 3
                        && decl.dimensions.iter().all(|d| matches!(d, UnpackedDimension::Range { .. } | UnpackedDimension::Expression { .. }))
                    {
                        let mut shape: Vec<(i64, i64)> = Vec::new();
                        for d in &decl.dimensions {
                            match d {
                                UnpackedDimension::Range { left, right, .. } => {
                                    let l = const_eval_i64_with_params(left, Some(&elab.parameters)).unwrap_or(0);
                                    let r = const_eval_i64_with_params(right, Some(&elab.parameters)).unwrap_or(0);
                                    shape.push((l.min(r), l.max(r)));
                                }
                                UnpackedDimension::Expression { expr, .. } => {
                                    let n = const_eval_i64_with_params(expr, Some(&elab.parameters)).unwrap_or(0);
                                    shape.push((0, (n - 1).max(0)));
                                }
                                _ => {}
                            }
                        }
                        elab.arrays_nd.insert(decl.name.name.clone(), (shape.clone(), width));
                        let is_real = is_type_real(&dd.data_type);
                        fn enumerate(dims: &[(i64, i64)], prefix: String, out: &mut Vec<String>) {
                            if dims.is_empty() { out.push(prefix); return; }
                            let (lo, hi) = dims[0];
                            for i in lo..=hi {
                                enumerate(&dims[1..], format!("{}[{}]", prefix, i), out);
                            }
                        }
                        let mut names = Vec::new();
                        enumerate(&shape, decl.name.name.clone(), &mut names);
                        for elem_name in names {
                            let sig = Signal { is_const: dd.const_kw,
                                name: elem_name.clone(),
                                width,
                                is_signed,
                                is_real,
                                direction: None,
                                value: default_value_for_type(&dd.data_type, width),
                                type_name: get_type_name(&dd.data_type),
                            };
                            elab.signals.insert(elem_name, sig);
                        }
                        continue;
                    }
                    // Check for unpacked array dimensions (e.g., memory [0:255])
                    let array_range = extract_array_range(&decl.dimensions, &elab.parameters);
                    if let Some((lo, hi)) = array_range {
                        // Register this as an array for the simulator
                        elab.arrays.insert(decl.name.name.clone(), (lo, hi, width));
                        // Track descending arrays (left > right in the declaration)
                        if let Some(UnpackedDimension::Range { left, right, .. }) = decl.dimensions.first() {
                            let l = const_eval_i64_with_params(left, Some(&elab.parameters)).unwrap_or(0);
                            let r = const_eval_i64_with_params(right, Some(&elab.parameters)).unwrap_or(0);
                            if l > r { elab.descending_arrays.insert(decl.name.name.clone()); }
                        }
                        let is_real = is_type_real(&dd.data_type);
                        // Create individual element signals: name[lo], name[lo+1], ..., name[hi]
                        for idx in lo..=hi {
                            let elem_name = format!("{}[{}]", decl.name.name, idx);
                            let sig = Signal { is_const: dd.const_kw,
                                name: elem_name.clone(),
                                width,
                                is_signed,
                                is_real,
                                direction: None,
                                value: default_value_for_type(&dd.data_type, width),
                                type_name: get_type_name(&dd.data_type),
                            };
                            elab.signals.insert(elem_name, sig);
                        }
                        if let Some(init_expr) = &decl.init {
                            let init_items: Vec<&Expression> = match &init_expr.kind {
                                ExprKind::AssignmentPattern(items) => items.iter().map(|i| i.expr()).collect(),
                                ExprKind::Concatenation(exprs) => exprs.iter().collect(),
                                _ => vec![],
                            };
                            if !init_items.is_empty() {
                                let mut stmts: Vec<Statement> = Vec::new();
                                for (i, item_expr) in init_items.iter().enumerate() {
                                    let idx_i = lo + i as i64;
                                    let lval = Expression::new(ExprKind::Index {
                                        expr: Box::new(make_ident_expr(&decl.name.name)),
                                        index: Box::new(Expression::new(ExprKind::Number(crate::ast::expr::NumberLiteral::Integer { size: None, signed: false, base: crate::ast::expr::NumberBase::Decimal, value: idx_i.to_string(), cached_val: std::cell::Cell::new(None) }), Span::dummy())),
                                    }, Span::dummy());
                                    stmts.push(Statement::new(StatementKind::BlockingAssign {
                                        lvalue: lval,
                                        rvalue: (*item_expr).clone(),
                                    }, Span::dummy()));
                                }
                                if is_dynamic_dim {
                                    let size_name = format!("{}.size", decl.name.name);
                                    let size_sig = Signal { is_const: false, name: size_name.clone(), width: 32, is_signed: false, is_real: false, direction: None, value: Value::from_u64(init_items.len() as u64, 32), type_name: None };
                                    elab.signals.insert(size_name, size_sig);
                                }
                                elab.initial_blocks.push(InitialBlock {
                                    stmt: Statement::new(StatementKind::SeqBlock { name: None, stmts }, Span::dummy()),
                                });
                            } else if !is_dynamic_dim {
                                elab.initial_blocks.push(InitialBlock {
                                    stmt: Statement::new(StatementKind::BlockingAssign {
                                        lvalue: make_ident_expr(&decl.name.name),
                                        rvalue: init_expr.clone(),
                                    }, Span::dummy()),
                                });
                            }
                        }
                    } else {
                        let is_real = is_type_real(&dd.data_type);
                        let w = width;
                        let (init_val, procedural_init) = if let Some(init_expr) = &decl.init {
                            if is_const_expr(init_expr, &elab.parameters) {
                                let mut rv = eval_const_expr_val(init_expr, &elab.parameters).resize(w);
                                if is_signed { rv.is_signed = true; }
                                if is_real { rv = Value::from_f64(rv.to_f64()); }
                                (rv, None)
                            } else {
                                (default_value_for_type(&dd.data_type, w), Some(init_expr.clone()))
                            }
                        } else { (default_value_for_type(&dd.data_type, w), None) };
                        
                        let sig = Signal { is_const: dd.const_kw,
                            name: decl.name.name.clone(),
                            width: w,
                            is_signed,
                            is_real,
                            direction: None,
                            value: init_val,
                            type_name: get_type_name(&dd.data_type),
                        };
                        elab.signals.insert(decl.name.name.clone(), sig);
                        if let Some(view) = &data_modport_view {
                            elab.modport_views.insert(decl.name.name.clone(), view.clone());
                        }
                        
                        if let Some(expr) = procedural_init {
                            elab.initial_blocks.push(InitialBlock {
                                stmt: Statement::new(StatementKind::BlockingAssign {
                                    lvalue: make_ident_expr(&decl.name.name),
                                    rvalue: expr,
                                }, decl.name.span),
                            });
                        }
                        // Unpacked-struct member default initializers:
                        //   struct { bit [3:0] lo = c; ... } p1;
                        // Packed structs forbid member defaults (IEEE 7.2.2).
                        let dt_resolved: &DataType = if let DataType::TypeReference { name, .. } = &dd.data_type {
                            elab.typedef_types.get(&name.name.name).unwrap_or(&dd.data_type)
                        } else { &dd.data_type };
                        // Recursively flatten nested struct/union members so multi-segment
                        // paths like u.s.a resolve via a single packed_struct_fields lookup.
                        fn flatten_subfields(dt: &DataType, params: &HashMap<String, Value>, typedefs: &HashMap<String, u32>, typedef_types: &HashMap<String, DataType>) -> Option<Vec<(String, u32, u32)>> {
                            let resolved = if let DataType::TypeReference { name, .. } = dt {
                                typedef_types.get(&name.name.name).unwrap_or(dt)
                            } else { dt };
                            if let DataType::Struct(su) = resolved {
                                let is_union = matches!(su.kind, StructUnionKind::Union);
                                let mut raw: Vec<(String, u32, DataType)> = Vec::new();
                                for member in &su.members {
                                    let mw = resolve_type_width(&member.data_type, Some(params), Some(typedefs));
                                    for mdecl in &member.declarators {
                                        raw.push((mdecl.name.name.clone(), mw, member.data_type.clone()));
                                    }
                                }
                                let mut out: Vec<(String, u32, u32)> = Vec::new();
                                if is_union {
                                    for (mn, mw, mdt) in &raw {
                                        out.push((mn.clone(), 0, *mw));
                                        if let Some(subs) = flatten_subfields(mdt, params, typedefs, typedef_types) {
                                            for (sn, so, sw) in subs { out.push((format!("{}.{}", mn, sn), so, sw)); }
                                        }
                                    }
                                } else {
                                    let mut offset: u32 = 0;
                                    for (mn, mw, mdt) in raw.iter().rev() {
                                        out.push((mn.clone(), offset, *mw));
                                        if let Some(subs) = flatten_subfields(mdt, params, typedefs, typedef_types) {
                                            for (sn, so, sw) in subs { out.push((format!("{}.{}", mn, sn), offset + so, sw)); }
                                        }
                                        offset += mw;
                                    }
                                }
                                Some(out)
                            } else { None }
                        }
                        if let Some(fields) = flatten_subfields(dt_resolved, &elab.parameters, &elab.typedefs, &elab.typedef_types) {
                            if !fields.is_empty() {
                                elab.packed_struct_fields.insert(decl.name.name.clone(), fields);
                            }
                        }
                        if let DataType::Struct(su) = dt_resolved {
                            let is_union = matches!(su.kind, StructUnionKind::Union);
                            if su.packed {
                                for member in &su.members {
                                    for mdecl in &member.declarators {
                                        if mdecl.init.is_some() {
                                            return Err(format!(
                                                "Packed struct member '{}.{}' cannot have a default value (IEEE 7.2.2)",
                                                decl.name.name, mdecl.name.name
                                            ));
                                        }
                                    }
                                }
                                // packed_struct_fields already populated by flatten_subfields above.
                            }
                            if !su.packed {
                                // Pre-register member signals with their declared widths,
                                // so later assignments from wider rvalues don't widen them.
                                for member in &su.members {
                                    let mw = resolve_type_width(&member.data_type, Some(&elab.parameters), Some(&elab.typedefs));
                                    let ms = is_type_signed(&member.data_type);
                                    for mdecl in &member.declarators {
                                        let sname = format!("{}.{}", decl.name.name, mdecl.name.name);
                                        elab.signals.entry(sname.clone()).or_insert(Signal {
                                            is_const: false,
                                            name: sname,
                                            width: mw,
                                            is_signed: ms,
                                            is_real: false,
                                            direction: None,
                                            value: Value::new(mw),
                                            type_name: None,
                                        });
                                    }
                                }
                                let mut stmts: Vec<Statement> = Vec::new();
                                for member in &su.members {
                                    for mdecl in &member.declarators {
                                        if let Some(init) = &mdecl.init {
                                            let lval = Expression::new(ExprKind::MemberAccess {
                                                expr: Box::new(make_ident_expr(&decl.name.name)),
                                                member: mdecl.name.clone(),
                                            }, Span::dummy());
                                            stmts.push(Statement::new(StatementKind::BlockingAssign {
                                                lvalue: lval,
                                                rvalue: init.clone(),
                                            }, Span::dummy()));
                                        }
                                    }
                                }
                                if !stmts.is_empty() {
                                    elab.initial_blocks.push(InitialBlock {
                                        stmt: Statement::new(StatementKind::SeqBlock { name: None, stmts }, Span::dummy()),
                                    });
                                }
                            }
                        }
                    }
                }
            }
            ModuleItem::ParameterDeclaration(pd) | ModuleItem::LocalparamDeclaration(pd) => {
                if let ParameterKind::Data { data_type, assignments } = &pd.kind {
                    let mut width = resolve_type_width(data_type, Some(&elab.parameters), Some(&elab.typedefs));
                    let mut signed = is_type_signed(data_type);
                    let mut is_real = is_type_real(data_type);
                    // IEEE 1800-2017 §6.20.2: implicit type → signed 32-bit
                    if matches!(data_type, DataType::Implicit { dimensions, .. } if dimensions.is_empty()) {
                        width = 32;
                        signed = true;
                    }
                    for assign in assignments {
                        if elab.signals.contains_key(&assign.name.name) || elab.parameters.contains_key(&assign.name.name) {
                            return Err(format!("Duplicate declaration of '{}'", assign.name.name));
                        }
                        let mut current_width = width;
                        let mut current_is_real = is_real;
                        let mut current_signed = signed;

                        if matches!(data_type, DataType::Implicit { dimensions, .. } if dimensions.is_empty()) {
                            let init_is_real = if elab.parameters.contains_key(&assign.name.name) {
                                elab.parameters.get(&assign.name.name).map(|v| v.is_real).unwrap_or(false)
                            } else if let Some(init) = &assign.init {
                                eval_const_expr_val(init, &elab.parameters).is_real
                            } else { false };

                            if init_is_real {
                                current_width = 64;
                                current_is_real = true;
                                current_signed = false;
                            }
                        }

                        let mut val = if elab.parameters.contains_key(&assign.name.name) {
                            elab.parameters.get(&assign.name.name).cloned().unwrap_or(Value::zero(current_width))
                        } else if let Some(init) = &assign.init {
                            if expr_has_call(init) {
                                elab.deferred_param_exprs.push((assign.name.name.clone(), init.clone()));
                                let mut v = Value::zero(current_width);
                                if current_signed { v.is_signed = true; }
                                v
                            } else {
                                let mut v = eval_const_expr_val(init, &elab.parameters).resize(current_width);
                                if current_signed { v.is_signed = true; }
                                v
                            }
                        } else {
                            let mut v = Value::zero(current_width);
                            if current_signed { v.is_signed = true; }
                            v
                        };

                        if current_is_real {
                            val = Value::from_f64(val.to_f64());
                        }

                        if !elab.parameters.contains_key(&assign.name.name) {
                            elab.parameters.insert(assign.name.name.clone(), val.clone());
                        }

                        // Also add as a signal so it can be read in expressions
                        elab.signals.insert(assign.name.name.clone(), Signal { is_const: false,
                            name: assign.name.name.clone(),
                            width: current_width,
                            is_signed: current_signed,
                            is_real: current_is_real,
                            direction: None,
                            value: val,
                            type_name: get_type_name(data_type),
                        });
                    }
                }
            }
            ModuleItem::TypedefDeclaration(td) => {
                process_typedef(td, &mut elab);
            }
            ModuleItem::FunctionDeclaration(fd) => {
                if matches!(fd.return_type, DataType::Void(_)) {
                    fn check_void_return(s: &crate::ast::stmt::Statement) -> Result<(), String> {
                        use crate::ast::stmt::StatementKind as SK;
                        match &s.kind {
                            SK::Return(Some(_)) => Err("void function must not return a value".into()),
                            SK::SeqBlock { stmts, .. } | SK::ParBlock { stmts, .. } => {
                                for st in stmts { check_void_return(st)?; }
                                Ok(())
                            }
                            SK::If { then_stmt, else_stmt, .. } => {
                                check_void_return(then_stmt)?;
                                if let Some(eb) = else_stmt { check_void_return(eb)?; }
                                Ok(())
                            }
                            SK::For { body, .. } | SK::While { body, .. } | SK::DoWhile { body, .. }
                            | SK::Repeat { body, .. } | SK::Forever { body } | SK::Foreach { body, .. } => check_void_return(body),
                            SK::TimingControl { stmt, .. } | SK::Wait { stmt, .. } => check_void_return(stmt),
                            SK::Case { items, .. } => { for it in items { check_void_return(&it.stmt)?; } Ok(()) }
                            _ => Ok(()),
                        }
                    }
                    for it in &fd.items { check_void_return(it)?; }
                }
                fn check_fn_fork(s: &crate::ast::stmt::Statement) -> Result<(), String> {
                    use crate::ast::stmt::StatementKind as SK;
                    match &s.kind {
                        SK::ParBlock { join_type, stmts, .. } => {
                            if !matches!(join_type, crate::ast::stmt::JoinType::JoinNone) {
                                return Err("only fork-join_none is permitted inside a function".into());
                            }
                            for st in stmts { check_fn_fork(st)?; }
                            Ok(())
                        }
                        SK::SeqBlock { stmts, .. } => { for st in stmts { check_fn_fork(st)?; } Ok(()) }
                        SK::If { then_stmt, else_stmt, .. } => {
                            check_fn_fork(then_stmt)?;
                            if let Some(eb) = else_stmt { check_fn_fork(eb)?; }
                            Ok(())
                        }
                        SK::For { body, .. } | SK::While { body, .. } | SK::DoWhile { body, .. }
                        | SK::Repeat { body, .. } | SK::Forever { body } | SK::Foreach { body, .. } => check_fn_fork(body),
                        SK::TimingControl { stmt, .. } | SK::Wait { stmt, .. } => check_fn_fork(stmt),
                        SK::Case { items, .. } => { for it in items { check_fn_fork(&it.stmt)?; } Ok(()) }
                        _ => Ok(()),
                    }
                }
                for it in &fd.items { check_fn_fork(it)?; }
                elab.functions.insert(fd.name.name.name.clone(), fd.clone());
            }
            ModuleItem::TaskDeclaration(td) => {
                fn check_no_return_in_fork(s: &crate::ast::stmt::Statement, in_fork: bool) -> Result<(), String> {
                    use crate::ast::stmt::StatementKind as SK;
                    match &s.kind {
                        SK::Return(_) if in_fork => Err("illegal return from fork".into()),
                        SK::ParBlock { stmts, .. } => { for st in stmts { check_no_return_in_fork(st, true)?; } Ok(()) }
                        SK::SeqBlock { stmts, .. } => { for st in stmts { check_no_return_in_fork(st, in_fork)?; } Ok(()) }
                        SK::If { then_stmt, else_stmt, .. } => {
                            check_no_return_in_fork(then_stmt, in_fork)?;
                            if let Some(eb) = else_stmt { check_no_return_in_fork(eb, in_fork)?; }
                            Ok(())
                        }
                        SK::For { body, .. } | SK::While { body, .. } | SK::DoWhile { body, .. }
                        | SK::Repeat { body, .. } | SK::Forever { body } | SK::Foreach { body, .. } => check_no_return_in_fork(body, in_fork),
                        SK::TimingControl { stmt, .. } | SK::Wait { stmt, .. } => check_no_return_in_fork(stmt, in_fork),
                        SK::Case { items, .. } => { for it in items { check_no_return_in_fork(&it.stmt, in_fork)?; } Ok(()) }
                        _ => Ok(()),
                    }
                }
                for it in &td.items { check_no_return_in_fork(it, false)?; }
                elab.tasks.insert(td.name.name.name.clone(), td.clone());
            }
            ModuleItem::ContinuousAssign(ca) => {
                for (lhs, rhs) in &ca.assignments {
                    elab.continuous_assigns.push(ContinuousAssignment { lhs: lhs.clone(), rhs: rhs.clone() });
                }
            }
            ModuleItem::AlwaysConstruct(ac) => {
                elab.always_blocks.push(AlwaysBlock { kind: ac.kind, stmt: ac.stmt.clone() });
            }
            ModuleItem::InitialConstruct(ic) => {
                elab.initial_blocks.push(InitialBlock { stmt: ic.stmt.clone() });
            }
            ModuleItem::GenerateRegion(gr) => {
                // Recursively process generate region items
                elaborate_items(&gr.items, &mut elab, all_defs)?;
            }
            ModuleItem::GenerateIf(gi) => {
                elaborate_generate_if(&gi.branches, &mut elab, all_defs)?;
            }
            ModuleItem::GenerateFor(gf) => {
                elaborate_generate_for(gf, &mut elab, all_defs)?;
            }
            ModuleItem::CovergroupDeclaration(cg) => {
                elab.covergroups.insert(cg.name.name.clone(), cg.clone());
            }
            ModuleItem::ClockingDeclaration(cd) => {
                let mut dirs = HashMap::new();
                for s in &cd.signals {
                    dirs.insert(s.name.name.clone(), s.direction);
                }
                elab.clocking_signal_dirs.insert(cd.name.name.clone(), dirs);
                elab.clocking_blocks.insert(cd.name.name.clone(), cd.clone());
            }
            ModuleItem::ClassDeclaration(cd) => {
                validate_class_constraints(cd, all_defs)?;
                elab.classes.insert(cd.name.name.clone(), elaborate_class(cd));
            }
            ModuleItem::LetDeclaration(ld) => {
                elab.lets.insert(ld.name.name.clone(), ld.clone());
            }
            ModuleItem::SequenceDeclaration(sd) => {
                elab.sequences.insert(sd.name.name.clone());
            }
            ModuleItem::PropertyDeclaration(pd) => {
                elab.sequences.insert(pd.name.name.clone());
            }
            ModuleItem::SpecifyBlock(sb) => {
                for p in &sb.paths {
                    let d = eval_const_expr(&p.delay, &elab.parameters);
                    elab.specify_delays.insert(p.dst.name.clone(), d);
                }
            }
            ModuleItem::ModuleInstantiation(inst) => {
                for hi in &inst.instances {
                    // Register the instance name so it's recognized during validation.
                    // It will be fully elaborated during inlining.
                    if !elab.signals.contains_key(&hi.name.name) {
                        elab.signals.insert(hi.name.name.clone(), Signal {
                            is_const: false,
                            name: hi.name.name.clone(),
                            width: 1,
                            is_signed: false,
                            is_real: false,
                            direction: None,
                            value: Value::new(1),
                            type_name: Some(inst.module_name.name.clone()),
                        });
                    }
                }
            }
            ModuleItem::ImportDeclaration(imp) => {
                if let Some(defs) = all_defs {
                    process_import(imp, &mut elab, defs)?;
                }
            }
            ModuleItem::DPIImport(di) => {
                register_dpi_import(di, &mut elab)?;
            }
            ModuleItem::OutOfClassConstraint { class_name, constraint_name } => {
                elab.out_of_class_constraints.insert((class_name.clone(), constraint_name.clone()));
            }
            _ => {}
        }
    }

    // User-defined nettype driver resolution: collapse multiple continuous
    // drivers on a nettype variable into a single OR-combined assign. This
    // approximates the common `resolve_or` resolver; other resolvers are not
    // modeled, so last-driver-wins behavior applies via the final `|` fold.
    {
        let mut nettype_vars: HashSet<String> = HashSet::new();
        for (name, sig) in &elab.signals {
            if let Some(tn) = &sig.type_name {
                if user_nettypes.contains(tn) { nettype_vars.insert(name.clone()); }
            }
        }
        if !nettype_vars.is_empty() {
            let mut grouped: HashMap<String, Vec<Expression>> = HashMap::new();
            let mut kept: Vec<ContinuousAssignment> = Vec::new();
            for ca in elab.continuous_assigns.drain(..) {
                if let Some(n) = simple_lhs_name(&ca.lhs) {
                    if nettype_vars.contains(&n) {
                        grouped.entry(n).or_default().push(ca.rhs);
                        continue;
                    }
                }
                kept.push(ca);
            }
            for (name, rhses) in grouped {
                let mut iter = rhses.into_iter();
                let mut acc = iter.next().unwrap();
                for rhs in iter {
                    let span = acc.span;
                    acc = Expression {
                        kind: ExprKind::Binary {
                            op: crate::ast::expr::BinaryOp::BitOr,
                            left: Box::new(acc),
                            right: Box::new(rhs),
                        },
                        span,
                    };
                }
                kept.push(ContinuousAssignment { lhs: make_ident_expr(&name), rhs: acc });
            }
            elab.continuous_assigns = kept;
        }
    }

    // IEEE 1800-2017 §6.10: Implicit nets — identifiers used in continuous assigns
    // or port connections that are not explicitly declared become implicit 1-bit wires.
    create_implicit_nets(&mut elab);

    // Validate that all identifiers in procedural blocks are declared.
    for ib in &elab.initial_blocks { validate_stmt_idents(&ib.stmt, &elab, &mut HashSet::new())?; }
    for ab in &elab.always_blocks { validate_stmt_idents(&ab.stmt, &elab, &mut HashSet::new())?; }
    for ca in &elab.continuous_assigns {
        validate_expr_idents(&ca.lhs, &elab, &HashSet::new())?;
        validate_expr_idents(&ca.rhs, &elab, &HashSet::new())?;
    }

    // IEEE 1800-2017 §6.5: a variable cannot have multiple continuous drivers,
    // nor mix continuous and procedural drivers.
    validate_driver_conflicts(&elab)?;

    // IEEE 1800-2017 §8.21/§8.26: class instantiation legality.
    validate_class_usage(&elab)?;

    Ok(elab)
}

fn expr_is_new(expr: &Expression) -> bool {
    match &expr.kind {
        ExprKind::Ident(hier) => hier.path.len() == 1 && hier.path[0].name.name == "new",
        ExprKind::Call { func, .. } => {
            if let ExprKind::Ident(hier) = &func.kind {
                return hier.path.len() == 1 && hier.path[0].name.name == "new";
            }
            false
        }
        _ => false,
    }
}

fn check_new_assignment(lvalue: &Expression, rvalue: &Expression, elab: &ElaboratedModule) -> Result<(), String> {
    if !expr_is_new(rvalue) { return Ok(()); }
    let name = match simple_lhs_name(lvalue) { Some(n) => n, None => return Ok(()) };
    let type_name = elab.signals.get(&name).and_then(|s| s.type_name.clone());
    if let Some(tn) = type_name {
        if let Some(cls) = elab.classes.get(&tn) {
            if cls.is_interface {
                return Err(format!("Cannot instantiate interface class '{}'", tn));
            }
            if cls.is_virtual || cls.has_pure_virtual {
                return Err(format!("Cannot instantiate abstract class '{}'", tn));
            }
        }
    }
    Ok(())
}

fn walk_stmt_for_class_new(stmt: &Statement, elab: &ElaboratedModule) -> Result<(), String> {
    match &stmt.kind {
        StatementKind::BlockingAssign { lvalue, rvalue } | StatementKind::NonblockingAssign { lvalue, rvalue, .. } => {
            check_new_assignment(lvalue, rvalue, elab)?;
        }
        StatementKind::If { then_stmt, else_stmt, .. } => {
            walk_stmt_for_class_new(then_stmt, elab)?;
            if let Some(eb) = else_stmt { walk_stmt_for_class_new(eb, elab)?; }
        }
        StatementKind::Case { items, .. } => { for it in items { walk_stmt_for_class_new(&it.stmt, elab)?; } }
        StatementKind::For { body, .. } | StatementKind::Foreach { body, .. } |
        StatementKind::While { body, .. } | StatementKind::DoWhile { body, .. } |
        StatementKind::Repeat { body, .. } | StatementKind::Forever { body } => walk_stmt_for_class_new(body, elab)?,
        StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
            for s in stmts { walk_stmt_for_class_new(s, elab)?; }
        }
        StatementKind::TimingControl { stmt, .. } | StatementKind::Wait { stmt, .. } => walk_stmt_for_class_new(stmt, elab)?,
        _ => {}
    }
    Ok(())
}

fn data_type_kind_name(dt: &DataType) -> String {
    match dt {
        DataType::Void(_) => "void".to_string(),
        DataType::IntegerAtom { kind, signing, .. } => format!("atom:{:?}:{:?}", kind, signing),
        DataType::IntegerVector { kind, signing, dimensions, .. } => format!("vec:{:?}:{:?}:{}", kind, signing, dimensions.len()),
        DataType::Real { kind, .. } => format!("real:{:?}", kind),
        DataType::Simple { kind, .. } => format!("simple:{:?}", kind),
        DataType::TypeReference { name, .. } => format!("tref:{}", name.name.name),
        DataType::Interface { name, .. } => format!("iface:{}", name.name),
        DataType::Struct(_) => "struct".to_string(),
        DataType::Enum(_) => "enum".to_string(),
        DataType::Implicit { .. } => "implicit".to_string(),
    }
}

fn validate_class_usage(elab: &ElaboratedModule) -> Result<(), String> {
    // §8.26.4: `implements T` where T is a class type parameter is illegal.
    for cls in elab.classes.values() {
        for imp in &cls.implements {
            if cls.type_param_names.iter().any(|n| n == imp) {
                return Err(format!("Class '{}' cannot implement type parameter '{}'", cls.name, imp));
            }
        }
    }
    // §8.26.6.1: multiple interface-class implementations that declare the same
    // method name with conflicting return types cannot be satisfied by a
    // single concrete method.
    for cls in elab.classes.values() {
        if cls.implements.len() < 2 { continue; }
        let mut seen: HashMap<String, String> = HashMap::new();
        for iname in &cls.implements {
            let iface = match elab.classes.get(iname) { Some(c) => c, None => continue };
            for (mname, m) in &iface.methods {
                let ret = match &m.kind {
                    ClassMethodKind::Function(f) | ClassMethodKind::PureVirtual(f) | ClassMethodKind::Extern(f) =>
                        data_type_kind_name(&f.return_type),
                    ClassMethodKind::Task(_) => "task".to_string(),
                };
                match seen.get(mname) {
                    Some(prev) if prev != &ret => {
                        return Err(format!("Class '{}' has conflicting return types for inherited method '{}'", cls.name, mname));
                    }
                    None => { seen.insert(mname.clone(), ret); }
                    _ => {}
                }
            }
        }
    }
    // §8.21/§8.26.5: reject instantiating an abstract or interface class.
    for ib in &elab.initial_blocks { walk_stmt_for_class_new(&ib.stmt, elab)?; }
    for ab in &elab.always_blocks { walk_stmt_for_class_new(&ab.stmt, elab)?; }

    // §8.26.3: typedefs declared in an interface class are NOT inherited by
    // classes that implement it. Flag a method signature that references a
    // bare typedef that only exists inside an implemented interface class.
    for cls in elab.classes.values() {
        if cls.implements.is_empty() { continue; }
        // Gather typedef names contributed only by implemented interfaces.
        let mut iface_only_typedefs: HashSet<String> = HashSet::new();
        for iname in &cls.implements {
            if let Some(iface) = elab.classes.get(iname) {
                for t in &iface.typedef_names { iface_only_typedefs.insert(t.clone()); }
            }
        }
        // Remove anything the class itself (or its extends chain) defines,
        // plus names reachable through module-level typedefs.
        for t in &cls.typedef_names { iface_only_typedefs.remove(t); }
        let mut cur = cls.extends.clone();
        let mut guard = 0;
        while let Some(base) = cur {
            guard += 1; if guard > 32 { break; }
            if let Some(b) = elab.classes.get(&base) {
                for t in &b.typedef_names { iface_only_typedefs.remove(t); }
                cur = b.extends.clone();
            } else { break; }
        }
        for t in elab.typedefs.keys() { iface_only_typedefs.remove(t); }
        if iface_only_typedefs.is_empty() { continue; }
        for m in cls.methods.values() {
            let func = match &m.kind {
                ClassMethodKind::Function(f) | ClassMethodKind::PureVirtual(f) | ClassMethodKind::Extern(f) => Some(f),
                _ => None,
            };
            if let Some(f) = func {
                for p in &f.ports {
                    if let DataType::TypeReference { name, .. } = &p.data_type {
                        if name.scope.is_some() { continue; }
                        if iface_only_typedefs.contains(&name.name.name) {
                            return Err(format!(
                                "Class '{}' method '{}' references type '{}' — typedefs from implemented interfaces are not inherited",
                                cls.name, f.name.name.name, name.name.name));
                        }
                    }
                }
            }
        }
    }

    // §18.6.3, §18.8, §18.9: `randomize`, `rand_mode`, and `constraint_mode`
    // are built-in methods and cannot be overridden by a user class.
    const RESERVED_METHODS: &[&str] = &["randomize", "rand_mode", "constraint_mode"];
    for cls in elab.classes.values() {
        for reserved in RESERVED_METHODS {
            if cls.methods.contains_key(*reserved) {
                return Err(format!(
                    "Class '{}' cannot override built-in method '{}'", cls.name, reserved));
            }
        }
    }

    // §18.5.1: `extern constraint c;` must be accompanied by an out-of-class
    // definition `constraint ClassName::c { ... }`.
    for cls in elab.classes.values() {
        for (cname, con) in &cls.constraints {
            if con.is_extern && !con.has_body {
                let defined = elab.out_of_class_constraints
                    .contains(&(cls.name.clone(), cname.clone()));
                if !defined {
                    return Err(format!(
                        "Class '{}' declares extern constraint '{}' with no external definition",
                        cls.name, cname));
                }
            }
        }
    }

    // §18.5.4, §18.5.10, §18.5.14: randc variables cannot appear in dist
    // expressions, solve..before lists, or soft constraints.
    for cls in elab.classes.values() {
        for con in cls.constraints.values() {
            for item in &con.items {
                check_randc_restrictions(item, &cls.randc_properties, &cls.name)?;
            }
        }
    }

    Ok(())
}

fn check_randc_restrictions(item: &ConstraintItem, randc: &HashSet<String>, cls: &str) -> Result<(), String> {
    if randc.is_empty() { return Ok(()); }
    match item {
        ConstraintItem::Inside { expr, is_dist: true, .. } => {
            if let Some(n) = simple_expr_name(expr) {
                if randc.contains(&n) {
                    return Err(format!(
                        "Class '{}': dist constraint cannot be applied to randc variable '{}'", cls, n));
                }
            }
        }
        ConstraintItem::Solve { before, after, .. } => {
            for id in before.iter().chain(after.iter()) {
                if randc.contains(&id.name) {
                    return Err(format!(
                        "Class '{}': randc variable '{}' cannot appear in solve..before", cls, id.name));
                }
            }
        }
        ConstraintItem::Soft(inner) => {
            collect_soft_randc(inner, randc, cls)?;
        }
        ConstraintItem::Block(items) => {
            for i in items { check_randc_restrictions(i, randc, cls)?; }
        }
        ConstraintItem::Implication { constraint, .. } => {
            check_randc_restrictions(constraint, randc, cls)?;
        }
        ConstraintItem::IfElse { then_item, else_item, .. } => {
            check_randc_restrictions(then_item, randc, cls)?;
            if let Some(e) = else_item { check_randc_restrictions(e, randc, cls)?; }
        }
        ConstraintItem::Foreach { item, .. } => {
            check_randc_restrictions(item, randc, cls)?;
        }
        _ => {}
    }
    Ok(())
}

fn collect_soft_randc(item: &ConstraintItem, randc: &HashSet<String>, cls: &str) -> Result<(), String> {
    // Any randc variable referenced inside a soft constraint is illegal.
    let mut names: HashSet<String> = HashSet::new();
    collect_constraint_idents(item, &mut names);
    for n in &names {
        if randc.contains(n) {
            return Err(format!(
                "Class '{}': soft constraint cannot reference randc variable '{}'", cls, n));
        }
    }
    Ok(())
}

fn collect_constraint_idents(item: &ConstraintItem, out: &mut HashSet<String>) {
    match item {
        ConstraintItem::Expr(e) => collect_expr_idents(e, out),
        ConstraintItem::Inside { expr, range, .. } => {
            collect_expr_idents(expr, out);
            for r in range {
                match r {
                    ConstraintRange::Value(v) => collect_expr_idents(v, out),
                    ConstraintRange::Range { lo, hi } => {
                        collect_expr_idents(lo, out); collect_expr_idents(hi, out);
                    }
                }
            }
        }
        ConstraintItem::Implication { condition, constraint, .. } => {
            collect_expr_idents(condition, out);
            collect_constraint_idents(constraint, out);
        }
        ConstraintItem::IfElse { condition, then_item, else_item, .. } => {
            collect_expr_idents(condition, out);
            collect_constraint_idents(then_item, out);
            if let Some(e) = else_item { collect_constraint_idents(e, out); }
        }
        ConstraintItem::Foreach { item, .. } => collect_constraint_idents(item, out),
        ConstraintItem::Soft(inner) => collect_constraint_idents(inner, out),
        ConstraintItem::Block(items) => for i in items { collect_constraint_idents(i, out); },
        ConstraintItem::Solve { .. } => {}
    }
}

fn collect_expr_idents(expr: &Expression, out: &mut HashSet<String>) {
    use crate::ast::expr::ExprKind;
    match &expr.kind {
        ExprKind::Ident(h) => {
            if let Some(s) = h.path.first() { out.insert(s.name.name.clone()); }
        }
        ExprKind::Binary { left, right, .. } => {
            collect_expr_idents(left, out); collect_expr_idents(right, out);
        }
        ExprKind::Unary { operand, .. } => collect_expr_idents(operand, out),
        ExprKind::Paren(e) => collect_expr_idents(e, out),
        ExprKind::Conditional { condition, then_expr, else_expr } => {
            collect_expr_idents(condition, out);
            collect_expr_idents(then_expr, out);
            collect_expr_idents(else_expr, out);
        }
        _ => {}
    }
}

fn simple_expr_name(expr: &Expression) -> Option<String> {
    use crate::ast::expr::ExprKind;
    match &expr.kind {
        ExprKind::Ident(h) if h.path.len() == 1 => Some(h.path[0].name.name.clone()),
        ExprKind::Paren(e) => simple_expr_name(e),
        _ => None,
    }
}

fn simple_lhs_name(expr: &Expression) -> Option<String> {
    match &expr.kind {
        ExprKind::Ident(hier) if hier.path.len() == 1 && hier.path[0].selects.is_empty() => {
            Some(hier.path[0].name.name.clone())
        }
        ExprKind::Paren(inner) => simple_lhs_name(inner),
        _ => None,
    }
}

fn collect_written_idents(stmt: &Statement, out: &mut HashSet<String>) {
    match &stmt.kind {
        StatementKind::BlockingAssign { lvalue, .. } | StatementKind::NonblockingAssign { lvalue, .. } => {
            if let Some(n) = simple_lhs_name(lvalue) { out.insert(n); }
        }
        StatementKind::If { then_stmt, else_stmt, .. } => {
            collect_written_idents(then_stmt, out);
            if let Some(eb) = else_stmt { collect_written_idents(eb, out); }
        }
        StatementKind::Case { items, .. } => {
            for item in items { collect_written_idents(&item.stmt, out); }
        }
        StatementKind::For { body, init, .. } => {
            for fi in init { if let ForInit::Assign { lvalue, .. } = fi {
                if let Some(n) = simple_lhs_name(lvalue) { out.insert(n); }
            }}
            collect_written_idents(body, out);
        }
        StatementKind::Foreach { body, .. } => collect_written_idents(body, out),
        StatementKind::While { body, .. } | StatementKind::DoWhile { body, .. } => collect_written_idents(body, out),
        StatementKind::Repeat { body, .. } => collect_written_idents(body, out),
        StatementKind::Forever { body } => collect_written_idents(body, out),
        StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
            for s in stmts { collect_written_idents(s, out); }
        }
        StatementKind::TimingControl { stmt, .. } => collect_written_idents(stmt, out),
        StatementKind::Wait { stmt, .. } => collect_written_idents(stmt, out),
        _ => {}
    }
}

fn validate_driver_conflicts(elab: &ElaboratedModule) -> Result<(), String> {
    let mut ca_lhs: HashMap<String, u32> = HashMap::new();
    for ca in &elab.continuous_assigns {
        if let Some(n) = simple_lhs_name(&ca.lhs) {
            if elab.signals.contains_key(&n) && !elab.nets.contains(&n) {
                let c = ca_lhs.entry(n.clone()).or_insert(0);
                *c += 1;
                if *c == 2 {
                    return Err(format!("Variable '{}' has multiple continuous drivers", n));
                }
            }
        }
    }
    let mut proc_written: HashSet<String> = HashSet::new();
    for ab in &elab.always_blocks { collect_written_idents(&ab.stmt, &mut proc_written); }
    for ib in &elab.initial_blocks { collect_written_idents(&ib.stmt, &mut proc_written); }
    for ca in &elab.continuous_assigns {
        if let Some(n) = simple_lhs_name(&ca.lhs) {
            if proc_written.contains(&n) && elab.signals.contains_key(&n) && !elab.nets.contains(&n) {
                return Err(format!("Variable '{}' has both continuous and procedural drivers", n));
            }
        }
    }
    Ok(())
}

fn validate_stmt_idents(stmt: &Statement, elab: &ElaboratedModule, locals: &mut HashSet<String>) -> Result<(), String> {
    match &stmt.kind {
        StatementKind::BlockingAssign { lvalue, rvalue } | StatementKind::NonblockingAssign { lvalue, rvalue, .. } => {
            if let ExprKind::Ident(hier) = &lvalue.kind {
                let name = if hier.path.len() == 1 {
                    Some(hier.path[0].name.name.clone())
                } else {
                    // Hierarchical name: join segments
                    Some(hier.path.iter().map(|s| s.name.name.as_str()).collect::<Vec<_>>().join("."))
                };
                if let Some(n) = name {
                    if let Some(sig) = elab.signals.get(&n) {
                        if sig.is_const {
                            return Err(format!("Illegal write to constant identifier '{}'", n));
                        }
                        if sig.direction == Some(PortDirection::Input) {
                            return Err(format!("Illegal write to input identifier '{}'", n));
                        }
                    }
                }
            }
            if let ExprKind::MemberAccess { expr, member } = &lvalue.kind {
                if let ExprKind::Ident(hier) = &expr.kind {
                    if hier.path.len() == 1 {
                        let base = &hier.path[0].name.name;
                        if let Some(view) = elab.modport_views.get(base) {
                            if view.get(&member.name) == Some(&PortDirection::Input) {
                                return Err(format!("Illegal write to input identifier '{}.{}'", base, member.name));
                            }
                        }
                        if let Some(dirs) = elab.clocking_signal_dirs.get(base) {
                            if dirs.get(&member.name) == Some(&PortDirection::Input) {
                                return Err(format!("Illegal write to input identifier '{}.{}'", base, member.name));
                            }
                        }
                    }
                }
            }
            validate_expr_idents(lvalue, elab, locals)?;
            validate_expr_idents(rvalue, elab, locals)?;
        }
        StatementKind::If { condition, then_stmt, else_stmt, .. } => {
            validate_expr_idents(condition, elab, locals)?;
            validate_stmt_idents(then_stmt, elab, locals)?;
            if let Some(eb) = else_stmt { validate_stmt_idents(eb, elab, locals)?; }
        }
        StatementKind::Case { expr, items, .. } => {
            validate_expr_idents(expr, elab, locals)?;
            for item in items {
                for p in &item.patterns { validate_expr_idents(p, elab, locals)?; }
                validate_stmt_idents(&item.stmt, elab, locals)?;
            }
        }
        StatementKind::For { init, condition, step, body } => {
            let mut for_locals = Vec::new();
            for fi in init { match fi {
                ForInit::VarDecl { name, init: e, .. } => {
                    validate_expr_idents(e, elab, locals)?;
                    locals.insert(name.name.clone());
                    for_locals.push(name.name.clone());
                }
                ForInit::Assign { lvalue, rvalue } => {
                    validate_expr_idents(lvalue, elab, locals)?;
                    validate_expr_idents(rvalue, elab, locals)?;
                }
            }}
            if let Some(c) = condition { validate_expr_idents(c, elab, locals)?; }
            for s in step { validate_expr_idents(s, elab, locals)?; }
            validate_stmt_idents(body, elab, locals)?;
            for n in for_locals { locals.remove(&n); }
        }
        StatementKind::Foreach { array, body, vars } => {
            validate_expr_idents(array, elab, locals)?;
            let mut foreach_locals = Vec::new();
            for v in vars {
                if let Some(id) = v {
                    locals.insert(id.name.clone());
                    foreach_locals.push(id.name.clone());
                }
            }
            validate_stmt_idents(body, elab, locals)?;
            for n in foreach_locals { locals.remove(&n); }
        }
        StatementKind::While { condition, body } | StatementKind::DoWhile { body, condition } => {
            validate_expr_idents(condition, elab, locals)?;
            validate_stmt_idents(body, elab, locals)?;
        }
        StatementKind::Repeat { count, body } => {
            validate_expr_idents(count, elab, locals)?;
            validate_stmt_idents(body, elab, locals)?;
        }
        StatementKind::Forever { body } => validate_stmt_idents(body, elab, locals)?,
        StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
            for s in stmts { validate_stmt_idents(s, elab, locals)?; }
        }
        StatementKind::TimingControl { control, stmt } => {
            match control {
                TimingControl::Delay(e) => validate_expr_idents(e, elab, locals)?,
                TimingControl::Event(ev) => validate_event_idents(ev, elab, locals)?,
            }
            validate_stmt_idents(stmt, elab, locals)?;
        }
        StatementKind::Expr(e) => validate_expr_idents(e, elab, locals)?,
        StatementKind::Wait { condition, stmt } => {
            validate_expr_idents(condition, elab, locals)?;
            validate_stmt_idents(stmt, elab, locals)?;
        }
        StatementKind::Assertion(a) => {
            validate_expr_idents(&a.expr, elab, locals)?;
            if let Some(s) = &a.action { validate_stmt_idents(s, elab, locals)?; }
            if let Some(s) = &a.else_action { validate_stmt_idents(s, elab, locals)?; }
        }
        StatementKind::Return(e) => { if let Some(expr) = e { validate_expr_idents(expr, elab, locals)?; } }
        StatementKind::VarDecl { declarators, .. } => {
            for d in declarators {
                if let Some(init) = &d.init { validate_expr_idents(init, elab, locals)?; }
                locals.insert(d.name.name.clone());
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_expr_idents(expr: &Expression, elab: &ElaboratedModule, locals: &HashSet<String>) -> Result<(), String> {
    match &expr.kind {
        ExprKind::Ident(hier) => {
            // Only check plain identifiers for now (hierarchical might be valid across modules)
            if hier.path.len() == 1 {
                let name = &hier.path[0].name.name;
                if name == "new" || name.starts_with('$') || name == "super" || name == "this" {
                    return Ok(());
                }
                if !elab.signals.contains_key(name) && !elab.parameters.contains_key(name) &&
                   !elab.functions.contains_key(name) && !elab.tasks.contains_key(name) &&
                   !elab.dpi_imports.contains_key(name) &&
                   !elab.arrays.contains_key(name) && !elab.associative_arrays.contains_key(name) &&
                   !elab.arrays_2d.contains_key(name) && !elab.arrays_nd.contains_key(name) &&
                   !elab.classes.contains_key(name) && !elab.typedefs.contains_key(name) &&
                   !elab.clocking_blocks.contains_key(name) && !elab.lets.contains_key(name) &&
                   !elab.sequences.contains(name) &&
                   !locals.contains(name) {
                   return Err(format!("Undeclared identifier '{}'", name));
                }            }
        }
        ExprKind::Unary { operand, .. } => validate_expr_idents(operand, elab, locals)?,
        ExprKind::Binary { left, right, .. } => { validate_expr_idents(left, elab, locals)?; validate_expr_idents(right, elab, locals)?; }
        ExprKind::Conditional { condition, then_expr, else_expr } => {
            validate_expr_idents(condition, elab, locals)?;
            validate_expr_idents(then_expr, elab, locals)?;
            validate_expr_idents(else_expr, elab, locals)?;
        }
        ExprKind::Concatenation(parts) => { for p in parts { validate_expr_idents(p, elab, locals)?; } }
        ExprKind::Replication { count, exprs } => {
            validate_expr_idents(count, elab, locals)?;
            for e in exprs { validate_expr_idents(e, elab, locals)?; }
        }
        ExprKind::Index { expr, index } => {
            if let ExprKind::Ident(hier) = &expr.kind {
                if hier.path.len() == 1 {
                    if let Some(sig) = elab.signals.get(&hier.path[0].name.name) {
                        if sig.is_real {
                            return Err(format!("Bit-select of real variable '{}' is not allowed", sig.name));
                        }
                    }
                }
            }
            if let ExprKind::Ident(hier) = &index.kind {
                if hier.path.len() == 1 {
                    if let Some(sig) = elab.signals.get(&hier.path[0].name.name) {
                        if sig.is_real {
                            return Err(format!("Real variable '{}' cannot be used as bit-select index", sig.name));
                        }
                    }
                }
            }
            validate_expr_idents(expr, elab, locals)?;
            validate_expr_idents(index, elab, locals)?;
        }
        ExprKind::RangeSelect { expr, left, right, .. } => {
            if let ExprKind::Ident(hier) = &expr.kind {
                if hier.path.len() == 1 {
                    if let Some(sig) = elab.signals.get(&hier.path[0].name.name) {
                        if sig.is_real {
                            return Err(format!("Part-select of real variable '{}' is not allowed", sig.name));
                        }
                    }
                }
            }
            validate_expr_idents(expr, elab, locals)?;
            validate_expr_idents(left, elab, locals)?;
            validate_expr_idents(right, elab, locals)?;
        }
        ExprKind::Paren(inner) => validate_expr_idents(inner, elab, locals)?,
        ExprKind::Call { func, args } => {
            validate_expr_idents(func, elab, locals)?;
            for a in args { validate_expr_idents(a, elab, locals)?; }
        }
        ExprKind::SystemCall { name, args } => {
            // Args can be scope/module/instance references (not value lookups)
            // for dump/coverage/scope-info system tasks.
            let skip = matches!(
                name.as_str(),
                "$dumpvars" | "$dumpfile" | "$dumpports" | "$dumpportsoff"
                    | "$dumpportson" | "$dumpportsflush" | "$dumpportsall"
                    | "$dumpportslimit" | "$printtimescale" | "$timeformat"
                    | "$coverage_control" | "$coverage_get" | "$coverage_get_max"
                    | "$coverage_merge" | "$coverage_save" | "$get_coverage"
                    | "$set_coverage_db_name" | "$load_coverage_db"
            );
            if !skip {
                for a in args { validate_expr_idents(a, elab, locals)?; }
            }
        }
        ExprKind::MemberAccess { expr, .. } => {
            // Skip validation when LHS is a bare ident matching a known package name
            // (e.g. `pkg::name`); those are scope refs, not value lookups.
            if let ExprKind::Ident(hier) = &expr.kind {
                if hier.path.len() == 1 && elab.packages.contains(&hier.path[0].name.name) {
                    // skip
                } else {
                    validate_expr_idents(expr, elab, locals)?;
                }
            } else {
                validate_expr_idents(expr, elab, locals)?;
            }
        }
        ExprKind::WithClause { expr, filter } => {
            validate_expr_idents(expr, elab, locals)?;
            let mut with_locals = locals.clone();
            with_locals.insert("item".to_string());
            validate_expr_idents(filter, elab, &with_locals)?;
        }
        _ => {}
    }
    Ok(())
}

fn validate_event_idents(ev: &EventControl, elab: &ElaboratedModule, locals: &HashSet<String>) -> Result<(), String> {
    match ev {
        EventControl::EventExpr(exprs) => {
            for ee in exprs {
                if ee.edge.is_some() {
                    if let ExprKind::Ident(hier) = &ee.expr.kind {
                        if hier.path.len() == 1 {
                            if let Some(sig) = elab.signals.get(&hier.path[0].name.name) {
                                if sig.is_real {
                                    return Err(format!("Edge event on real variable '{}' is not allowed", sig.name));
                                }
                            }
                        }
                    }
                }
                validate_expr_idents(&ee.expr, elab, locals)?;
            }
        }
        EventControl::Identifier(id) => {
            if !elab.signals.contains_key(&id.name) && !elab.parameters.contains_key(&id.name)
                && !elab.sequences.contains(&id.name) && !locals.contains(&id.name)
            {
                return Err(format!("Undeclared identifier '{}'", id.name));
            }
        }
        EventControl::HierIdentifier(e) => validate_expr_idents(e, elab, locals)?,
        _ => {}
    }
    Ok(())
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
            elab.signals.insert(name.clone(), Signal { is_const: false,
                name: name.clone(), width: 1, is_signed: false,
                direction: None, value: Value::new(1),
                is_real: false, type_name: None,
            });
            elab.nets.insert(name);
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
fn elaborate_items(items: &[ModuleItem], elab: &mut ElaboratedModule, all_defs: Option<&HashMap<String, Definition>>) -> Result<(), String> {
    for item in items {
        match item {
            ModuleItem::PortDeclaration(pd) => {
                let port_modport_view = match &pd.data_type {
                    DataType::Interface { name, modport: Some(mp), .. } => {
                        resolve_interface_modport_view(&name.name, &mp.name, all_defs)
                    }
                    _ => None,
                };
                let width = resolve_type_width(&pd.data_type, Some(&elab.parameters), Some(&elab.typedefs));
                let is_signed = is_type_signed(&pd.data_type);
                let is_real = is_type_real(&pd.data_type);
                for decl in &pd.declarators {
                    if elab.signals.contains_key(&decl.name.name) || elab.parameters.contains_key(&decl.name.name) {
                        return Err(format!("Duplicate declaration of '{}'", decl.name.name));
                    }
                    let sig = Signal { is_const: false,
                        name: decl.name.name.clone(), width, is_signed,
                        direction: Some(pd.direction), value: if is_real { Value::from_f64(0.0) } else { Value::new(width) },
                        is_real, type_name: get_type_name(&pd.data_type),
                    };
                    elab.signals.insert(decl.name.name.clone(), sig);
                    elab.port_order.push(decl.name.name.clone());
                    if let Some(view) = &port_modport_view {
                        elab.modport_views.insert(decl.name.name.clone(), view.clone());
                    }
                }
            }
            ModuleItem::NetDeclaration(nd) => {
                let width = resolve_type_width(&nd.data_type, Some(&elab.parameters), Some(&elab.typedefs));
                let is_signed = is_type_signed(&nd.data_type);
                let is_real = is_type_real(&nd.data_type);
                for decl in &nd.declarators {
                    let init_value = match nd.net_type {
                        NetType::Supply0 => Value::zero(width),
                        NetType::Supply1 => Value::ones(width),
                        _ => if is_real { Value::from_f64(0.0) } else { Value::new(width) },
                    };
                    let sig = Signal { is_const: false,
                        name: decl.name.name.clone(), width, is_signed,
                        direction: None, value: init_value,
                        is_real, type_name: get_type_name(&nd.data_type),
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
                let data_modport_view = match &dd.data_type {
                    DataType::Interface { name, modport: Some(mp), .. } => {
                        resolve_interface_modport_view(&name.name, &mp.name, all_defs)
                    }
                    _ => None,
                };
                let width = match &dd.data_type {
                    DataType::TypeReference { name, .. } => {
                        elab.typedefs.get(&name.name.name).copied().unwrap_or(resolve_type_width(&dd.data_type, Some(&elab.parameters), Some(&elab.typedefs)))
                    }
                    _ => resolve_type_width(&dd.data_type, Some(&elab.parameters), Some(&elab.typedefs)),
                };
                if let DataType::TypeReference { type_args, .. } = &dd.data_type {
                    if !type_args.is_empty() {
                        for decl in &dd.declarators {
                            elab.class_type_args.insert(decl.name.name.clone(), type_args.clone());
                        }
                    }
                }
                let is_signed = is_type_signed(&dd.data_type);
                let is_real = is_type_real(&dd.data_type);
                for decl in &dd.declarators {
                    if elab.signals.contains_key(&decl.name.name) || elab.parameters.contains_key(&decl.name.name) {
                        return Err(format!("Duplicate declaration of '{}'", decl.name.name));
                    }
                    if let Some(UnpackedDimension::Associative { data_type: key_dt, .. }) = decl.dimensions.first() {
                        let is_string_key = key_dt.as_ref().map_or(false, |dt| matches!(dt.as_ref(), DataType::Simple { kind: SimpleType::String, .. }));
                        elab.associative_arrays.insert(decl.name.name.clone(), is_string_key);
                    }
                    let is_dynamic_dim = decl.dimensions.first().map_or(false, |d| matches!(d, UnpackedDimension::Unsized(_) | UnpackedDimension::Queue { .. }));
                    if is_dynamic_dim {
                        elab.dynamic_arrays.insert(decl.name.name.clone());
                    }
                    let array_range = extract_array_range(&decl.dimensions, &elab.parameters);
                    if let Some((lo, hi)) = array_range {
                        elab.arrays.insert(decl.name.name.clone(), (lo, hi, width));
                        if let Some(UnpackedDimension::Range { left, right, .. }) = decl.dimensions.first() {
                            let l = const_eval_i64_with_params(left, Some(&elab.parameters)).unwrap_or(0);
                            let r = const_eval_i64_with_params(right, Some(&elab.parameters)).unwrap_or(0);
                            if l > r { elab.descending_arrays.insert(decl.name.name.clone()); }
                        }
                        for idx in lo..=hi {
                            let elem_name = format!("{}[{}]", decl.name.name, idx);
                            let sig = Signal { is_const: dd.const_kw,
                                name: elem_name.clone(), width, is_signed, is_real, direction: None,
                                value: default_value_for_type(&dd.data_type, width),
                                type_name: get_type_name(&dd.data_type)
                            };
                            elab.signals.insert(elem_name, sig);
                        }
                        if let Some(init_expr) = &decl.init {
                            let init_items: Vec<&Expression> = match &init_expr.kind {
                                ExprKind::AssignmentPattern(items) => items.iter().map(|i| i.expr()).collect(),
                                ExprKind::Concatenation(exprs) => exprs.iter().collect(),
                                _ => vec![],
                            };
                            if !init_items.is_empty() {
                                let mut stmts: Vec<Statement> = Vec::new();
                                for (i, item_expr) in init_items.iter().enumerate() {
                                    let idx_i = lo + i as i64;
                                    let lval = Expression::new(ExprKind::Index {
                                        expr: Box::new(make_ident_expr(&decl.name.name)),
                                        index: Box::new(Expression::new(ExprKind::Number(crate::ast::expr::NumberLiteral::Integer { size: None, signed: false, base: crate::ast::expr::NumberBase::Decimal, value: idx_i.to_string(), cached_val: std::cell::Cell::new(None) }), Span::dummy())),
                                    }, Span::dummy());
                                    stmts.push(Statement::new(StatementKind::BlockingAssign {
                                        lvalue: lval,
                                        rvalue: (*item_expr).clone(),
                                    }, Span::dummy()));
                                }
                                if is_dynamic_dim {
                                    let size_name = format!("{}.size", decl.name.name);
                                    let size_sig = Signal { is_const: false, name: size_name.clone(), width: 32, is_signed: false, is_real: false, direction: None, value: Value::from_u64(init_items.len() as u64, 32), type_name: None };
                                    elab.signals.insert(size_name, size_sig);
                                }
                                elab.initial_blocks.push(InitialBlock {
                                    stmt: Statement::new(StatementKind::SeqBlock { name: None, stmts }, Span::dummy()),
                                });
                            }
                        }
                    } else {
                        let init_val = if let Some(init_expr) = &decl.init {
                            let mut rv = eval_const_expr_val(init_expr, &elab.parameters).resize(width);
                            if is_signed { rv.is_signed = true; }
                            if is_real { rv = Value::from_f64(rv.to_f64()); }
                            rv
                        } else {
                            default_value_for_type(&dd.data_type, width)
                        };
                        let sig = Signal { is_const: dd.const_kw, name: decl.name.name.clone(), width, is_signed, is_real, direction: None, value: init_val, type_name: get_type_name(&dd.data_type) };
                        elab.signals.insert(decl.name.name.clone(), sig);
                        if let Some(view) = &data_modport_view {
                            elab.modport_views.insert(decl.name.name.clone(), view.clone());
                        }
                    }
                }
            }
            ModuleItem::ParameterDeclaration(pd) | ModuleItem::LocalparamDeclaration(pd) => {
                if let ParameterKind::Data { data_type, assignments } = &pd.kind {
                    let mut width = resolve_type_width(data_type, Some(&elab.parameters), Some(&elab.typedefs));
                    let signed = is_type_signed(data_type);
                    if matches!(data_type, DataType::Implicit { dimensions, .. } if dimensions.is_empty()) { width = 32; }
                    for assign in assignments {
                        if elab.signals.contains_key(&assign.name.name) || elab.parameters.contains_key(&assign.name.name) {
                            return Err(format!("Duplicate declaration of '{}'", assign.name.name));
                        }
                        if !elab.parameters.contains_key(&assign.name.name) {
                            let val = if let Some(init) = &assign.init {
                                let mut v = eval_const_expr_val(init, &elab.parameters).resize(width);
                                if signed { v.is_signed = true; }
                                v
                            } else { Value::zero(width) };
                            elab.parameters.insert(assign.name.name.clone(), val.clone());
                            elab.signals.insert(assign.name.name.clone(), Signal { is_const: false,
                                name: assign.name.name.clone(), width, is_signed: signed,
                                direction: None, value: val, is_real: is_type_real(data_type), type_name: get_type_name(data_type),
                            });
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
            ModuleItem::ModuleInstantiation(inst) => {
                for hi in &inst.instances {
                    if !elab.signals.contains_key(&hi.name.name) {
                        elab.signals.insert(hi.name.name.clone(), Signal {
                            is_const: false,
                            name: hi.name.name.clone(), width: 1,
                            is_signed: false, direction: None, value: Value::new(1), type_name: Some(inst.module_name.name.clone()),
                            is_real: false,
                        });
                    }
                }
            }
            ModuleItem::TypedefDeclaration(td) => {
                process_typedef(td, elab);
            }
            ModuleItem::GenerateRegion(gr) => {
                elaborate_items(&gr.items, elab, all_defs)?;
            }
            ModuleItem::GenerateIf(gi) => {
                elaborate_generate_if(&gi.branches, elab, all_defs)?;
            }
            ModuleItem::GenerateFor(gf) => {
                elaborate_generate_for(gf, elab, all_defs)?;
            }

            ModuleItem::ClassDeclaration(cd) => {
                validate_class_constraints(cd, all_defs)?;
                elab.classes.insert(cd.name.name.clone(), elaborate_class(cd));
            }
            ModuleItem::ClockingDeclaration(cd) => {
                let mut dirs = HashMap::new();
                for s in &cd.signals {
                    dirs.insert(s.name.name.clone(), s.direction);
                }
                elab.clocking_signal_dirs.insert(cd.name.name.clone(), dirs);
                elab.clocking_blocks.insert(cd.name.name.clone(), cd.clone());
            }
            ModuleItem::LetDeclaration(ld) => {
                elab.lets.insert(ld.name.name.clone(), ld.clone());
            }
            ModuleItem::SequenceDeclaration(sd) => {
                elab.sequences.insert(sd.name.name.clone());
            }
            ModuleItem::PropertyDeclaration(pd) => {
                elab.sequences.insert(pd.name.name.clone());
            }
            ModuleItem::SpecifyBlock(sb) => {
                for p in &sb.paths {
                    let d = eval_const_expr(&p.delay, &elab.parameters);
                    elab.specify_delays.insert(p.dst.name.clone(), d);
                }
            }
            ModuleItem::FunctionDeclaration(fd) => {
                if matches!(fd.return_type, DataType::Void(_)) {
                    fn check_void_return(s: &crate::ast::stmt::Statement) -> Result<(), String> {
                        use crate::ast::stmt::StatementKind as SK;
                        match &s.kind {
                            SK::Return(Some(_)) => Err("void function must not return a value".into()),
                            SK::SeqBlock { stmts, .. } | SK::ParBlock { stmts, .. } => {
                                for st in stmts { check_void_return(st)?; }
                                Ok(())
                            }
                            SK::If { then_stmt, else_stmt, .. } => {
                                check_void_return(then_stmt)?;
                                if let Some(eb) = else_stmt { check_void_return(eb)?; }
                                Ok(())
                            }
                            SK::For { body, .. } | SK::While { body, .. } | SK::DoWhile { body, .. }
                            | SK::Repeat { body, .. } | SK::Forever { body } | SK::Foreach { body, .. } => check_void_return(body),
                            SK::TimingControl { stmt, .. } | SK::Wait { stmt, .. } => check_void_return(stmt),
                            SK::Case { items, .. } => { for it in items { check_void_return(&it.stmt)?; } Ok(()) }
                            _ => Ok(()),
                        }
                    }
                    for it in &fd.items { check_void_return(it)?; }
                }
                fn check_fn_fork(s: &crate::ast::stmt::Statement) -> Result<(), String> {
                    use crate::ast::stmt::StatementKind as SK;
                    match &s.kind {
                        SK::ParBlock { join_type, stmts, .. } => {
                            if !matches!(join_type, crate::ast::stmt::JoinType::JoinNone) {
                                return Err("only fork-join_none is permitted inside a function".into());
                            }
                            for st in stmts { check_fn_fork(st)?; }
                            Ok(())
                        }
                        SK::SeqBlock { stmts, .. } => { for st in stmts { check_fn_fork(st)?; } Ok(()) }
                        SK::If { then_stmt, else_stmt, .. } => {
                            check_fn_fork(then_stmt)?;
                            if let Some(eb) = else_stmt { check_fn_fork(eb)?; }
                            Ok(())
                        }
                        SK::For { body, .. } | SK::While { body, .. } | SK::DoWhile { body, .. }
                        | SK::Repeat { body, .. } | SK::Forever { body } | SK::Foreach { body, .. } => check_fn_fork(body),
                        SK::TimingControl { stmt, .. } | SK::Wait { stmt, .. } => check_fn_fork(stmt),
                        SK::Case { items, .. } => { for it in items { check_fn_fork(&it.stmt)?; } Ok(()) }
                        _ => Ok(()),
                    }
                }
                for it in &fd.items { check_fn_fork(it)?; }
                elab.functions.insert(fd.name.name.name.clone(), fd.clone());
            }
            ModuleItem::TaskDeclaration(td) => {
                fn check_no_return_in_fork(s: &crate::ast::stmt::Statement, in_fork: bool) -> Result<(), String> {
                    use crate::ast::stmt::StatementKind as SK;
                    match &s.kind {
                        SK::Return(_) if in_fork => Err("illegal return from fork".into()),
                        SK::ParBlock { stmts, .. } => { for st in stmts { check_no_return_in_fork(st, true)?; } Ok(()) }
                        SK::SeqBlock { stmts, .. } => { for st in stmts { check_no_return_in_fork(st, in_fork)?; } Ok(()) }
                        SK::If { then_stmt, else_stmt, .. } => {
                            check_no_return_in_fork(then_stmt, in_fork)?;
                            if let Some(eb) = else_stmt { check_no_return_in_fork(eb, in_fork)?; }
                            Ok(())
                        }
                        SK::For { body, .. } | SK::While { body, .. } | SK::DoWhile { body, .. }
                        | SK::Repeat { body, .. } | SK::Forever { body } | SK::Foreach { body, .. } => check_no_return_in_fork(body, in_fork),
                        SK::TimingControl { stmt, .. } | SK::Wait { stmt, .. } => check_no_return_in_fork(stmt, in_fork),
                        SK::Case { items, .. } => { for it in items { check_no_return_in_fork(&it.stmt, in_fork)?; } Ok(()) }
                        _ => Ok(()),
                    }
                }
                for it in &td.items { check_no_return_in_fork(it, false)?; }
                elab.tasks.insert(td.name.name.name.clone(), td.clone());
            }
            ModuleItem::ImportDeclaration(imp) => {
                if let Some(defs) = all_defs {
                    process_import(imp, elab, defs)?;
                }
            }
            ModuleItem::DPIImport(di) => {
                register_dpi_import(di, elab)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Evaluate a generate-if: pick the first branch whose condition is true (or the else branch).
fn elaborate_generate_if(branches: &[(Option<Expression>, Vec<ModuleItem>)], elab: &mut ElaboratedModule, all_defs: Option<&HashMap<String, Definition>>) -> Result<(), String> {
    for (cond, items) in branches {
        match cond {
            Some(c) => {
                if !is_const_expr(c, &elab.parameters) {
                    return Err(format!("Generate if condition must be a constant expression"));
                }
                let val = eval_const_expr(c, &elab.parameters);
                if val != 0 {
                    return elaborate_items(items, elab, all_defs);
                }
            }
            None => {
                // Unconditional else branch
                return elaborate_items(items, elab, all_defs);
            }
        }
    }
    Ok(())
}

fn elaborate_generate_for(gf: &GenerateFor, elab: &mut ElaboratedModule, all_defs: Option<&HashMap<String, Definition>>) -> Result<(), String> {
    let var = &gf.var;
    let mut i = gf.init_val;
    for _ in 0..10000 {
        elab.parameters.insert(var.clone(), Value::from_u64(i as u64, 32));
        let cond_val = eval_const_expr(&gf.cond, &elab.parameters);
        if cond_val == 0 { break; }
        elaborate_items(&gf.items, elab, all_defs)?;
        // Evaluate increment: handle i++, i=i+1, etc.
        match &gf.incr.kind {
            ExprKind::Unary { op: UnaryOp::PostIncr, .. } | ExprKind::Unary { op: UnaryOp::PreIncr, .. } => { i += 1; }
            ExprKind::Unary { op: UnaryOp::PostDecr, .. } | ExprKind::Unary { op: UnaryOp::PreDecr, .. } => { i -= 1; }
            _ => {
                // Try to evaluate as expression (e.g. i = i + 1 expanded by parser)
                let new_val = eval_const_expr(&gf.incr, &elab.parameters) as i64;
                if new_val == i { i += 1; } else { i = new_val; }
            }
        }
    }
    elab.parameters.remove(var);
    Ok(())
}

/// Resolve the width of a data type.
pub fn resolve_type_width(
    dt: &DataType,
    params: Option<&HashMap<String, Value>>,
    typedefs: Option<&HashMap<String, u32>>
) -> u32 {
    match dt {
        DataType::IntegerVector { dimensions, .. } => {
            if dimensions.is_empty() { return 1; }
            let mut total = 1u32;
            for dim in dimensions {
                if let PackedDimension::Range { left, right, .. } = dim {
                    let lv = const_eval_i64_with_params(left, params);
                    let rv = const_eval_i64_with_params(right, params);
                    if let (Some(l), Some(r)) = (lv, rv) {
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
                    let lv = const_eval_i64_with_params(left, params);
                    let rv = const_eval_i64_with_params(right, params);
                    if let (Some(l), Some(r)) = (lv, rv) {
                        let w = (l - r).abs() + 1;
                        total *= w as u32;
                    }
                }
            }
            total
        }
        DataType::TypeReference { name, dimensions, .. } => {
            let mut base_width = if let Some(td) = typedefs {
                td.get(&name.name.name).copied().unwrap_or(32)
            } else {
                32
            };
            if !dimensions.is_empty() {
                for dim in dimensions {
                    if let PackedDimension::Range { left, right, .. } = dim {
                        if let (Some(l), Some(r)) = (const_eval_i64_with_params(left, params), const_eval_i64_with_params(right, params)) {
                            let w = (l - r).abs() + 1;
                            base_width *= w as u32;
                        }
                    }
                }
            }
            base_width
        }
        DataType::Simple { kind, .. } => match kind {
            SimpleType::String => 1024, // Dynamic string, allocate 128 chars max
            SimpleType::Chandle => 64,
            SimpleType::Event => 1,
        },
        DataType::Enum(e) => {
            if let Some(bt) = &e.base_type {
                resolve_type_width(bt, params, typedefs)
            } else {
                32
            }
        }
        DataType::Struct(s) => {
            let is_union = matches!(s.kind, StructUnionKind::Union);
            let mut total = 0u32;
            let mut max_w = 0u32;
            let mut member_count = 0u32;
            for member in &s.members {
                let mw = resolve_type_width(&member.data_type, params, typedefs);
                total += mw * member.declarators.len() as u32;
                for _ in &member.declarators {
                    if mw > max_w { max_w = mw; }
                    member_count += 1;
                }
            }
            if is_union {
                if s.tagged {
                    let tag_w = (member_count.max(2) - 1).next_power_of_two().trailing_zeros().max(1);
                    max_w + tag_w
                } else { max_w }
            } else { total }
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
            match kind {
                IntegerAtomType::Byte | IntegerAtomType::ShortInt | IntegerAtomType::Int | IntegerAtomType::LongInt | IntegerAtomType::Integer => true,
                IntegerAtomType::Time => false,
            }
        }
        DataType::Implicit { signing, .. } => matches!(signing, Some(Signing::Signed)),
        DataType::Real { .. } => true,
        DataType::Struct(su) => matches!(su.signing, Some(Signing::Signed)),
        _ => false,
    }
}

pub fn is_type_real(dt: &DataType) -> bool {
    matches!(dt, DataType::Real { .. })
}

/// Returns the default value for a type: 0 for 2-state types, X for 4-state types.
fn default_value_for_type(dt: &DataType, width: u32) -> Value {
    if is_type_real(dt) { return Value::from_f64(0.0); }
    if is_type_two_state(dt) { Value::zero(width) } else { Value::new(width) }
}

/// Returns true for 2-state types (bit, byte, shortint, int, longint) whose default is 0.
pub fn is_type_two_state(dt: &DataType) -> bool {
    match dt {
        DataType::IntegerVector { kind, .. } => matches!(kind, IntegerVectorType::Bit),
        DataType::IntegerAtom { kind, .. } => matches!(kind,
            IntegerAtomType::Byte | IntegerAtomType::ShortInt | IntegerAtomType::Int | IntegerAtomType::LongInt),
        DataType::Real { .. } => true,
        _ => false,
    }
}

pub fn const_eval_i64_with_params(expr: &Expression, params: Option<&HashMap<String, Value>>) -> Option<i64> {
    match &expr.kind {
        ExprKind::Number(NumberLiteral::Integer { value, base, .. }) => {
            let r = match base { NumberBase::Binary => 2, NumberBase::Octal => 8, NumberBase::Hex => 16, NumberBase::Decimal => 10 };
            i64::from_str_radix(&value.replace('_', ""), r).ok()
        }
        ExprKind::Number(NumberLiteral::UnbasedUnsized('0')) => Some(0),
        ExprKind::Number(NumberLiteral::UnbasedUnsized('1')) => Some(1),
        ExprKind::Ident(hier) => {
            if let Some(p) = params {
                let name = hier.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                p.get(name).and_then(|v| v.to_i64())
            } else { None }
        }
        ExprKind::Binary { op, left, right } => {
            let l = const_eval_i64_with_params(left, params)?;
            let r = const_eval_i64_with_params(right, params)?;
            match op {
                BinaryOp::Add => Some(l + r),
                BinaryOp::Sub => Some(l - r),
                BinaryOp::Mul => Some(l * r),
                BinaryOp::Div => if r != 0 { Some(l / r) } else { None },
                BinaryOp::ShiftLeft | BinaryOp::ArithShiftLeft => Some(l << r),
                BinaryOp::ShiftRight | BinaryOp::ArithShiftRight => Some(l >> r),
                _ => None,
            }
        }
        ExprKind::Unary { op, operand } => {
            let v = const_eval_i64_with_params(operand, params)?;
            match op {
                UnaryOp::Minus => Some(-v),
                UnaryOp::Plus => Some(v),
                _ => None,
            }
        }
        ExprKind::Paren(e) => const_eval_i64_with_params(e, params),
        ExprKind::SystemCall { name, args } if name == "$clog2" => {
            if let Some(arg) = args.first() {
                let val = const_eval_i64_with_params(arg, params)?;
                if val <= 1 { Some(0) }
                else {
                    let mut res = 0;
                    let mut tmp = val - 1;
                    while tmp > 0 {
                        tmp >>= 1;
                        res += 1;
                    }
                    Some(res)
                }
            } else { None }
        }
        _ => None,
    }
}

/// Extract array range from unpacked dimensions. Returns Some((lo, hi)) for
/// `[lo:hi]` or `[size]` (which means [0:size-1]).
fn extract_array_range(dims: &[crate::ast::types::UnpackedDimension], params: &HashMap<String, Value>) -> Option<(i64, i64)> {
    if dims.is_empty() { return None; }
    match &dims[0] {
        crate::ast::types::UnpackedDimension::Range { left, right, .. } => {
            let l = const_eval_i64_with_params(left, Some(params)).unwrap_or(0);
            let r = const_eval_i64_with_params(right, Some(params)).unwrap_or(0);
            let lo = l.min(r);
            let hi = l.max(r);
            Some((lo, hi))
        }
        crate::ast::types::UnpackedDimension::Expression { expr, .. } => {
            let size = const_eval_i64_with_params(expr, Some(params)).unwrap_or(0);
            if size > 0 { Some((0, size - 1)) } else { None }
        }
        crate::ast::types::UnpackedDimension::Unsized(_) | 
        crate::ast::types::UnpackedDimension::Queue { .. } => {
            // For dynamic arrays and queues, allocate a fixed-size buffer for simulation
            Some((0, 63))
        }
        crate::ast::types::UnpackedDimension::Associative { .. } => {
            // Associative arrays are purely dynamic
            None
        }
        _ => None,
    }
}

fn width_with_unpacked_dims(dims: &[crate::ast::types::UnpackedDimension], base_width: u32) -> u32 {
    if dims.is_empty() { return base_width; }
    let mut total_elements = 1u32;
    for dim in dims {
        match dim {
            crate::ast::types::UnpackedDimension::Range { left, right, .. } => {
                let l = const_eval_i64_with_params(left, None).unwrap_or(0);
                let r = const_eval_i64_with_params(right, None).unwrap_or(0);
                total_elements *= ((l - r).abs() + 1) as u32;
            }
            crate::ast::types::UnpackedDimension::Expression { expr, .. } => {
                let size = const_eval_i64_with_params(expr, None).unwrap_or(0);
                total_elements *= size.max(1) as u32;
            }
            crate::ast::types::UnpackedDimension::Unsized(_) | 
            crate::ast::types::UnpackedDimension::Queue { .. } |
            crate::ast::types::UnpackedDimension::Associative { .. } => {
                total_elements *= 64;
            }
        }
    }
    base_width * total_elements
}

/// Evaluate a constant expression (for enum values, parameter defaults, etc.)
fn eval_const_expr(expr: &Expression, params: &HashMap<String, Value>) -> u64 {
    eval_const_expr_val(expr, params).to_u64().unwrap_or(0)
}

/// Evaluate a constant expression, returning a full Value (preserving width/sign).
fn eval_const_expr_val(expr: &Expression, params: &HashMap<String, Value>) -> Value {
    let res = match &expr.kind {
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
                NumberLiteral::Real(f) => Value::from_f64(*f),
                NumberLiteral::UnbasedUnsized(c) => match c {
                    '0' => Value::zero(1), '1' => Value::from_u64(1, 1), _ => Value::new(1),
                },
            }
        }
        ExprKind::StringLiteral(s) => Value::from_string(s),
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
                BinaryOp::Mod => l.modulo(&r),
                BinaryOp::Power => l.power(&r),
                BinaryOp::Eq => l.is_equal(&r),
                BinaryOp::Neq => l.is_not_equal(&r),
                BinaryOp::Lt => l.less_than(&r),
                BinaryOp::Leq => l.less_equal(&r),
                BinaryOp::Gt => l.greater_than(&r),
                BinaryOp::Geq => l.greater_equal(&r),
                BinaryOp::ShiftLeft | BinaryOp::ArithShiftLeft => l.shift_left(&r),
                BinaryOp::ShiftRight => l.shift_right(&r),
                BinaryOp::BitOr => l.bitwise_or(&r),
                BinaryOp::BitAnd => l.bitwise_and(&r),
                BinaryOp::BitXor => l.bitwise_xor(&r),
                BinaryOp::BitXnor => l.bitwise_xor(&r).bitwise_not(),
                BinaryOp::LogOr => l.logic_or(&r),
                BinaryOp::LogAnd => l.logic_and(&r),
                BinaryOp::ArithShiftRight => l.arith_shift_right(&r),
                _ => Value::zero(32),
            }
        }
        ExprKind::Unary { op, operand } => {
            let v = eval_const_expr_val(operand, params);
            match op {
                UnaryOp::Minus => v.negate(),
                UnaryOp::Plus => v,
                UnaryOp::BitNot => v.bitwise_not(),
                UnaryOp::LogNot => v.logic_not(),
                _ => v,
            }
        }
        ExprKind::Dollar => Value::from_u64(u32::MAX as u64, 32),
        ExprKind::Paren(inner) => eval_const_expr_val(inner, params),
        ExprKind::SystemCall { name, args } if name == "$clog2" => {
            if let Some(arg) = args.first() {
                let v = eval_const_expr_val(arg, params);
                let val = v.to_u64().unwrap_or(0);
                if val <= 1 { Value::from_u64(0, 32) }
                else {
                    let mut res = 0;
                    let mut tmp = val - 1;
                    while tmp > 0 {
                        tmp >>= 1;
                        res += 1;
                    }
                    Value::from_u64(res, 32)
                }
            } else { Value::zero(32) }
        }
        ExprKind::Conditional { condition, then_expr, else_expr } => {
            let c = eval_const_expr_val(condition, params);
            if c.is_true() { eval_const_expr_val(then_expr, params) }
            else { eval_const_expr_val(else_expr, params) }
        }
        ExprKind::Concatenation(parts) => {
            let mut r = Value::zero(0);
            for p in parts.iter().rev() {
                r = eval_const_expr_val(p, params).concat_with(&r);
            }
            r
        }
        ExprKind::Replication { count, exprs } => {
            let n = eval_const_expr_val(count, params).to_u64().unwrap_or(1) as usize;
            let mut inner = Value::zero(0);
            for p in exprs.iter().rev() {
                inner = eval_const_expr_val(p, params).concat_with(&inner);
            }
            let mut r = Value::zero(0);
            for _ in 0..n { r = inner.clone().concat_with(&r); }
            r
        }
        _ => Value::zero(32),
    };
    // eprintln!("[DEBUG] eval_const_expr_val: {:?} -> {}", expr, res.to_dec_string());
    res
}

/// Inline module instantiations: replace instances with their continuous assigns and always blocks.
/// Handles recursive/multi-level hierarchies by walking all levels depth-first.
pub fn inline_instantiations(
    elab: &mut ElaboratedModule,
    definitions: &HashMap<String, Definition>,
) -> Result<(), String> {
    // Populate class and covergroup definitions from global scope
    for (name, def) in definitions {
        match def {
            Definition::Class(c) => { elab.classes.insert(name.clone(), elaborate_class(c)); }
            Definition::Covergroup(cg) => { elab.covergroups.insert(name.clone(), (*cg).clone()); }
            Definition::Package(p) => {
                elab.packages.insert(name.clone());
                for item in &p.items {
                    match item {
                        crate::ast::decl::PackageItem::Class(c) => {
                            elab.classes.insert(c.name.name.clone(), elaborate_class(c));
                        }
                        crate::ast::decl::PackageItem::Typedef(td) => {
                            process_typedef(td, elab);
                        }
                        crate::ast::decl::PackageItem::Parameter(pd) => {
                            if let ParameterKind::Data { data_type, assignments } = &pd.kind {
                                let mut width = resolve_type_width(data_type, Some(&elab.parameters), Some(&elab.typedefs));
                                let mut is_signed = is_type_signed(data_type);
                                if matches!(data_type, DataType::Implicit { dimensions, .. } if dimensions.is_empty()) {
                                    width = 32;
                                    is_signed = true;
                                }
                                for assign in assignments {
                                    if let Some(init) = &assign.init {
                                        let mut v = eval_const_expr_val(init, &elab.parameters).resize(width);
                                        if is_signed { v.is_signed = true; }
                                        elab.parameters.insert(assign.name.name.clone(), v);
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let module_name = elab.name.clone();
    let top_def = match definitions.get(&module_name) {
        Some(m) => *m,
        None => return Err(format!("Top module '{}' not found in module map", module_name)),
    };
    // Recursively inline starting from the top module's items
    let top_params = elab.parameters.clone();
    inline_module_items(elab, top_def, "", definitions, &mut HashMap::new(), &top_params)?;

    // Identify interface instances at top level
    let mut top_interface_names = HashSet::new();
    for item in top_def.items() {
        if let ModuleItem::ModuleInstantiation(inst) = item {
            if definitions.get(&inst.module_name.name).map_or(false, |d| matches!(d, Definition::Interface(_))) {
                for hi in &inst.instances {
                    top_interface_names.insert(hi.name.name.clone());
                }
            }
        }
    }

    // Final rewrite of all blocks to convert MemberAccess to HierarchicalIdentifier and handle local signals
    let local_names = elab.signals.keys().cloned().collect::<std::collections::HashSet<_>>();
    let port_map = HashMap::new();
    let mut interface_map = HashMap::new();
    for name in top_interface_names {
        interface_map.insert(name.clone(), name);
    }
    let prefix = "";

    for block in &mut elab.always_blocks {
        block.stmt = rewrite_stmt(&block.stmt, prefix, &port_map, &local_names, &interface_map);
    }
    for block in &mut elab.initial_blocks {
        block.stmt = rewrite_stmt(&block.stmt, prefix, &port_map, &local_names, &interface_map);
    }
    for assign in &mut elab.continuous_assigns {
        assign.lhs = rewrite_expr(&assign.lhs, prefix, &port_map, &local_names, &interface_map);
        assign.rhs = rewrite_expr(&assign.rhs, prefix, &port_map, &local_names, &interface_map);
    }

    Ok(())
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

fn is_interface_type(dt: &DataType, definitions: &HashMap<String, Definition>) -> bool {
    match dt {
        DataType::TypeReference { name, .. } => {
            if definitions.contains_key(&name.name.name) { return true; }
            false
        }
        DataType::Interface { name, .. } => {
            if definitions.contains_key(&name.name) { return true; }
            false
        }
        _ => false,
    }
}

fn inline_module_items(
    elab: &mut ElaboratedModule,
    source_def: Definition,
    prefix: &str,
    definitions: &HashMap<String, Definition>,
    interface_map: &mut HashMap<String, String>,
    local_params: &HashMap<String, Value>,
) -> Result<(), String> {
    // First pass: collect typedef names from this module so we can distinguish them from instantiations
    let mut local_typedefs: std::collections::HashSet<String> = std::collections::HashSet::new();
    for item in source_def.items() {
        if let ModuleItem::TypedefDeclaration(td) = item {
            local_typedefs.insert(td.name.name.clone());
        }
    }

    // Identify interface ports in the current module/interface
    let mut interface_ports: std::collections::HashSet<String> = std::collections::HashSet::new();
    match source_def.ports() {
        PortList::Ansi(ports) => {
            for port in ports {
                if let Some(dt) = &port.data_type {
                    if is_interface_type(dt, definitions) {
                        interface_ports.insert(port.name.name.clone());
                    }
                }
            }
        }
        _ => {} // Non-ANSI might need deeper check but skipping for now
    }

    let effective_source_items = collect_effective_items(source_def.items(), local_params);
    for item in &effective_source_items {
        if let ModuleItem::ModuleInstantiation(inst) = item {
            let sub_mod_name = &inst.module_name.name;
            let sub_mod = match definitions.get(sub_mod_name) {
                Some(m) => *m,
                None => {
                    // Check if it's a typedef-based variable declaration (happens if parser was unsure)
                    if elab.typedefs.contains_key(sub_mod_name) || local_typedefs.contains(sub_mod_name) {
                        let width = elab.typedefs.get(sub_mod_name).copied().unwrap_or(32);
                        let is_real = sub_mod_name == "real";
                        for hi in &inst.instances {
                            let sig_name = format!("{}{}", prefix, hi.name.name);
                            elab.signals.insert(sig_name.clone(), Signal { is_const: false,
                                name: sig_name, width, is_signed: is_real, direction: None,
                                value: if is_real { Value::from_f64(0.0) } else { Value::new(width) },
                                is_real, type_name: Some(sub_mod_name.clone()),
                            });
                        }
                        continue;
                    }
                    return Err(format!("Module '{}' instantiated but not found", sub_mod_name));
                }
            };

            for hi in &inst.instances {
                let inst_name = &hi.name.name;
                let inst_prefix = format!("{}{}.", prefix, inst_name);
                let mut scoped_eval_params = elab.parameters.clone();
                if !prefix.is_empty() {
                    for (k, v) in &elab.parameters {
                        if let Some(local_name) = k.strip_prefix(prefix) {
                            if !local_name.contains('.') {
                                scoped_eval_params.insert(local_name.to_string(), v.clone());
                            }
                        }
                    }
                }

                // Build port map and interface map
                let mut port_map = HashMap::new();
                let mut sub_interface_map = HashMap::new();

                // Identify interface ports in the sub-module
                let mut sub_interface_ports: std::collections::HashSet<String> = std::collections::HashSet::new();
                match sub_mod.ports() {
                    PortList::Ansi(ports) => {
                        for port in ports {
                            if let Some(dt) = &port.data_type {
                                if is_interface_type(dt, definitions) {
                                    sub_interface_ports.insert(port.name.name.clone());
                                }
                            }
                        }
                    }
                    _ => {}
                }

                // Local names of the CURRENT (parent) module — bare names of
                // signals declared in this scope. Used when rewriting port
                // connection parent expressions so bare identifiers get
                // prefixed with the current scope. Without this, a port
                // connection like `.mrd(mrd)` inside wrapper would be stored
                // in port_map as a bare `mrd`, and later substitutions into
                // the sub-module would insert a bare (unresolvable) name.
                let parent_local_names: std::collections::HashSet<String> = elab.signals.keys()
                    .filter_map(|s| {
                        if prefix.is_empty() {
                            if !s.contains('.') { Some(s.clone()) } else { None }
                        } else {
                            s.strip_prefix(prefix).and_then(|rest| {
                                if !rest.is_empty() && !rest.contains('.') { Some(rest.to_string()) } else { None }
                            })
                        }
                    })
                    .collect();

                if !hi.connections.is_empty() {
                    match &hi.connections[0] { // Simplification: check if first is wildcard
                        PortConnection::Wildcard => {
                            // Wildcard: connect all ports to same-named signals in parent
                            match sub_mod.ports() {
                                PortList::Ansi(ports) => {
                                    for port in ports {
                                        let name = &port.name.name;
                                        let parent_name = format!("{}{}", prefix, name);
                                        if sub_interface_ports.contains(name) {
                                            sub_interface_map.insert(name.clone(), parent_name.clone());
                                        } else {
                                            port_map.insert(name.clone(), make_ident_expr(&parent_name));
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        _ => {
                            for (i, conn) in hi.connections.iter().enumerate() {
                                match conn {
                                    PortConnection::Named { name, expr } => {
                                        if let Some(e) = expr {
                                            let rewritten_e = rewrite_expr(e, prefix, &HashMap::new(), &parent_local_names, interface_map);
                                            if sub_interface_ports.contains(&name.name) {
                                                if let ExprKind::Ident(hier) = &rewritten_e.kind {
                                                    let if_full_path = hier.path.iter().map(|s| s.name.name.as_str()).collect::<Vec<_>>().join(".");
                                                    sub_interface_map.insert(name.name.clone(), if_full_path);
                                                }
                                            } else {
                                                port_map.insert(name.name.clone(), rewritten_e);
                                            }
                                        }
                                    }
                                    PortConnection::Ordered(expr) => {
                                        if let Some(e) = expr {
                                            let rewritten_e = rewrite_expr(e, prefix, &HashMap::new(), &parent_local_names, interface_map);
                                            if let Some(port) = sub_mod.ports().get(i) {
                                                let port_name = port.name();
                                                if sub_interface_ports.contains(port_name) {
                                                    if let ExprKind::Ident(hier) = &rewritten_e.kind {
                                                        let if_full_path = hier.path.iter().map(|s| s.name.name.as_str()).collect::<Vec<_>>().join(".");
                                                        sub_interface_map.insert(port_name.to_string(), if_full_path);
                                                    }
                                                } else {
                                                    port_map.insert(port_name.to_string(), rewritten_e);
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }

                // Resolve parameters for the sub-module
                let mut sub_params = HashMap::new();
                if let Some(param_conns) = &inst.params {
                    for (i, conn) in param_conns.iter().enumerate() {
                        match conn {
                            ParamConnection::Named { name, value } => {
                                if let Some(ParamValue::Expr(v)) = value {
                                    let mut val = eval_const_expr_val(v, &scoped_eval_params);
                                    // Check if target parameter is real or implicit real
                                    for p_decl in sub_mod.params() {
                                        if let ParameterKind::Data { data_type, assignments } = &p_decl.kind {
                                            if assignments.iter().any(|a| a.name.name == name.name) {
                                                if is_type_real(data_type) {
                                                    val = Value::from_f64(val.to_f64());
                                                } else if matches!(data_type, DataType::Implicit { dimensions, .. } if dimensions.is_empty()) {
                                                    if val.is_real {
                                                        val = Value::from_f64(val.to_f64());
                                                    }
                                                }
                                                break;
                                            }
                                        }
                                    }
                                    sub_params.insert(name.name.clone(), val);
                                }
                            }
                            ParamConnection::Ordered(value) => {
                                if let Some(ParamValue::Expr(v)) = value {
                                    if let Some(p_decl) = sub_mod.params().get(i) {
                                        if let ParameterKind::Data { data_type, assignments } = &p_decl.kind {
                                            let mut val = eval_const_expr_val(v, &scoped_eval_params);
                                            if is_type_real(data_type) {
                                                val = Value::from_f64(val.to_f64());
                                            } else if matches!(data_type, DataType::Implicit { dimensions, .. } if dimensions.is_empty()) {
                                                if val.is_real {
                                                    val = Value::from_f64(val.to_f64());
                                                }
                                            }
                                            sub_params.insert(assignments[0].name.name.clone(), val);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Internal parameter map for resolving default parameters that depend on each other
                let mut sub_local_params = sub_params.clone();
                
                // Helper to add parameters from a list of items
                let add_params_from_items = |items: &[ModuleItem], local_map: &mut HashMap<String, Value>| {
                    let effective_items = collect_effective_items(items, local_map);
                    for item in &effective_items {
                        if let ModuleItem::ParameterDeclaration(pd) | ModuleItem::LocalparamDeclaration(pd) = item {
                            if let ParameterKind::Data { data_type, assignments } = &pd.kind {
                                for assign in assignments {
                                    if !local_map.contains_key(&assign.name.name) {
                                        if let Some(init) = &assign.init {
                                            let mut val = eval_const_expr_val(init, local_map);
                                            if is_type_real(data_type) {
                                                val = Value::from_f64(val.to_f64());
                                            } else if matches!(data_type, DataType::Implicit { dimensions, .. } if dimensions.is_empty()) {
                                                if val.is_real {
                                                    val = Value::from_f64(val.to_f64());
                                                }
                                            }
                                            local_map.insert(assign.name.name.clone(), val);
                                        }
                                    }
                                }
                            }
                        }
                    }
                };

                // 1. Parameters from port list
                for p_decl in sub_mod.params() {
                    if let ParameterKind::Data { data_type, assignments } = &p_decl.kind {
                        for assign in assignments {
                            if !sub_local_params.contains_key(&assign.name.name) {
                                if let Some(init) = &assign.init {
                                    let mut val = eval_const_expr_val(init, &sub_local_params);
                                    if is_type_real(data_type) {
                                        val = Value::from_f64(val.to_f64());
                                    } else if matches!(data_type, DataType::Implicit { dimensions, .. } if dimensions.is_empty()) {
                                        if val.is_real {
                                            val = Value::from_f64(val.to_f64());
                                        }
                                    }
                                    sub_local_params.insert(assign.name.name.clone(), val);
                                }
                            }
                        }
                    }
                }
                
                // 2. Parameters from module items
                add_params_from_items(sub_mod.items(), &mut sub_local_params);

                // Inline all resolved parameters into global map with prefix
                for (name, val) in &sub_local_params {
                    let full_name = format!("{}{}", inst_prefix, name);
                    elab.parameters.insert(full_name.clone(), val.clone());
                    // Also add as a signal for simulation access
                    elab.signals.insert(full_name.clone(), Signal { is_const: false,
                        name: full_name,
                        width: val.width,
                        is_signed: val.is_signed,
                        is_real: val.is_real,
                        direction: None,
                        value: val.clone(),
                        type_name: None,
                    });
                }

                // Declare sub-module port signals
                // Build a param map that includes both elab.parameters (global,
                // full-name keys) and sub_local_params (unprefixed short names)
                // so port type widths like `[W-1:0]` resolve against the sub's
                // overridden parameter values.
                let port_type_params = {
                    let mut m = elab.parameters.clone();
                    for (k, v) in &sub_local_params {
                        m.insert(k.clone(), v.clone());
                    }
                    m
                };
                match sub_mod.ports() {
                    PortList::Ansi(ports) => {
                        for port in ports {
                            if sub_interface_ports.contains(&port.name.name) { continue; }
                            let width = port.data_type.as_ref()
                                .map(|dt| resolve_type_width(dt, Some(&port_type_params), Some(&elab.typedefs)))
                                .unwrap_or(1);
                            let sig_name = format!("{}{}", inst_prefix, port.name.name);
                            let is_real = port.data_type.as_ref().map(is_type_real).unwrap_or(false);
                            elab.signals.insert(sig_name.clone(), Signal { is_const: false,
                                name: sig_name, width,
                                is_signed: port.data_type.as_ref().map(|dt| is_type_signed(dt)).unwrap_or(false),
                                is_real,
                                direction: port.direction,
                                value: if is_real { Value::from_f64(0.0) } else { Value::new(width) },
                                type_name: port.data_type.as_ref().and_then(get_type_name),
                            });
                        }
                    }
                    PortList::NonAnsi(_names) => {
                        let effective_items = collect_effective_items(sub_mod.items(), &sub_local_params);
                        for sub_item in &effective_items {
                            if let ModuleItem::PortDeclaration(pd) = sub_item {
                                if is_interface_type(&pd.data_type, definitions) { continue; }
                                let width = resolve_type_width(&pd.data_type, Some(&sub_local_params), Some(&elab.typedefs));
                                let is_signed = is_type_signed(&pd.data_type);
                                for decl in &pd.declarators {
                                    let sig_name = format!("{}{}", inst_prefix, decl.name.name);
                                    elab.signals.insert(sig_name.clone(), Signal { is_const: false,
                                        name: sig_name, width, is_signed,
                                        direction: Some(pd.direction),
                                        value: Value::new(width),
                                        is_real: is_type_real(&pd.data_type), type_name: get_type_name(&pd.data_type),
                                    });
                                }
                            }
                        }
                    }
                    PortList::Empty => {}
                }

                // Declare internal nets/vars (including from generate blocks)
                let effective_decl_items = collect_effective_items(sub_mod.items(), &sub_local_params);
                // Merge elab.parameters with sub_local_params so unprefixed
                // param references in type widths/dimensions resolve correctly
                let sub_merged_params = {
                    let mut m = elab.parameters.clone();
                    for (k, v) in &sub_local_params {
                        m.insert(k.clone(), v.clone());
                    }
                    m
                };
                for sub_item in &effective_decl_items {
                    if let ModuleItem::TypedefDeclaration(td) = sub_item {
                        if let DataType::Enum(et) = &td.data_type {
                            let base_width = et.base_type.as_ref()
                                .map(|bt| resolve_type_width(bt, Some(&sub_merged_params), Some(&elab.typedefs)))
                                .unwrap_or(32);
                            let mut next_val: u64 = 0;
                            for member in &et.members {
                                let val = if let Some(init) = &member.init {
                                    eval_const_expr(init, &sub_merged_params)
                                } else { next_val };
                                next_val = val.wrapping_add(1);
                                let v = Value::from_u64(val, base_width);
                                elab.parameters.insert(member.name.name.clone(), v.clone());
                                elab.signals.insert(member.name.name.clone(), Signal { is_const: false,
                                    name: member.name.name.clone(), width: base_width,
                                    is_signed: false, direction: None, value: v, type_name: None,
                                    is_real: false,
                                });
                            }
                            elab.typedefs.insert(td.name.name.clone(), base_width);
                        } else {
                            let w = resolve_type_width(&td.data_type, Some(&sub_merged_params), Some(&elab.typedefs));
                            elab.typedefs.insert(td.name.name.clone(), w);
                        }
                    }
                }
                for sub_item in &effective_decl_items {
                    match sub_item {
                        ModuleItem::NetDeclaration(nd) => {
                            let width = resolve_type_width(&nd.data_type, Some(&sub_merged_params), Some(&elab.typedefs));
                            for decl in &nd.declarators {
                                let sig_name = format!("{}{}", inst_prefix, decl.name.name);
                                let init_value = match nd.net_type {
                                    NetType::Supply0 => Value::zero(width),
                                    NetType::Supply1 => Value::ones(width),
                                    _ => Value::new(width),
                                };
                                elab.signals.insert(sig_name.clone(), Signal { is_const: false,
                                    name: sig_name, width,
                                    is_signed: is_type_signed(&nd.data_type),
                                    is_real: is_type_real(&nd.data_type),
                                    direction: None, value: init_value,
                                    type_name: get_type_name(&nd.data_type),
                                });                            }
                        }
                        ModuleItem::DataDeclaration(dd) => {
                            let width = match &dd.data_type {
                                DataType::TypeReference { name, .. } => {
                                    elab.typedefs.get(&name.name.name).copied()
                                        .unwrap_or(resolve_type_width(&dd.data_type, Some(&sub_merged_params), Some(&elab.typedefs)))
                                }
                                _ => resolve_type_width(&dd.data_type, Some(&sub_merged_params), Some(&elab.typedefs)),
                            };
                            let is_signed = is_type_signed(&dd.data_type);
                            for decl in &dd.declarators {
                                let base_name = decl.name.name.clone();
                                let sig_name = format!("{}{}", inst_prefix, base_name);
                                let array_range = extract_array_range(&decl.dimensions, &sub_merged_params);
                                if let Some((lo, hi)) = array_range {
                                    elab.arrays.insert(sig_name.clone(), (lo, hi, width));
                                    for idx in lo..=hi {
                                        let elem_name = format!("{}[{}]", sig_name, idx);
                                        elab.signals.insert(elem_name.clone(), Signal { is_const: dd.const_kw,
                                            name: elem_name, width, is_signed,
                                            direction: None, value: Value::new(width),
                                            is_real: is_type_real(&dd.data_type), type_name: get_type_name(&dd.data_type),
                                        });
                                    }
                                } else {
                                    let init_val = if let Some(init_expr) = &decl.init {
                                        eval_const_expr_val(init_expr, &sub_merged_params).resize(width)
                                    } else { Value::new(width) };
                                    elab.signals.insert(sig_name.clone(), Signal { is_const: dd.const_kw,
                                        name: sig_name, width, is_signed,
                                        direction: None, value: init_val,
                                        is_real: is_type_real(&dd.data_type), type_name: get_type_name(&dd.data_type),
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }

                // Port connection assigns
                let port_directions: HashMap<String, PortDirection> = {
                    let mut dirs = HashMap::new();
                    match sub_mod.ports() {
                        PortList::Ansi(ports) => {
                            for port in ports {
                                if let Some(dir) = port.direction {
                                    dirs.insert(port.name.name.clone(), dir);
                                }
                            }
                        }
                        PortList::NonAnsi(_) => {
                            let effective_items = collect_effective_items(sub_mod.items(), &sub_merged_params);
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
                    if sub_interface_ports.contains(port_name) { continue; }
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
                            elab.continuous_assigns.push(ContinuousAssignment {
                                lhs: sub_expr, rhs: parent_expr.clone(),
                            });
                        }
                    }
                }

                // Collect sub-module local signal names
                let mut local_names: std::collections::HashSet<String> = std::collections::HashSet::new();
                // Include module parameter port list names (e.g. picorv32 #(parameter PROGADDR_RESET = ...))
                for p_decl in sub_mod.params() {
                    if let ParameterKind::Data { assignments, .. } = &p_decl.kind {
                        for assign in assignments { local_names.insert(assign.name.name.clone()); }
                    }
                }
                match sub_mod.ports() {
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
                        ModuleItem::FunctionDeclaration(fd) => { local_names.insert(fd.name.name.name.clone()); }
                        ModuleItem::TaskDeclaration(td) => { local_names.insert(td.name.name.name.clone()); }
                        ModuleItem::ModuleInstantiation(inst) => {
                            if elab.typedefs.contains_key(&inst.module_name.name) || local_typedefs.contains(&inst.module_name.name) {
                                for hi in &inst.instances { local_names.insert(hi.name.name.clone()); }
                            }
                        }
                        ModuleItem::ParameterDeclaration(pd) | ModuleItem::LocalparamDeclaration(pd) => {
                            if let ParameterKind::Data { assignments, .. } = &pd.kind {
                                for assign in assignments { local_names.insert(assign.name.name.clone()); }
                            }
                        }
                        _ => {}
                    }
                }

                // Build a rewrite_port_map that excludes output ports.
                // Output ports should use the local prefixed name (inst_prefix + port_name)
                // rather than the parent expression, because:
                //   - Input ports: the sub-module reads from the parent → use parent expr
                //   - Output ports: the sub-module writes to its local reg → use prefixed local name
                //     (a continuous assign parent = local handles the connection)
                let rewrite_port_map: HashMap<String, Expression> = port_map.iter()
                    .filter(|(name, _)| {
                        !matches!(port_directions.get(name.as_str()), Some(PortDirection::Output))
                    })
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();

                // Inline the sub-module's continuous assigns
                let effective_items = collect_effective_items(sub_mod.items(), &sub_merged_params);
                for sub_item in &effective_items {
                    if let ModuleItem::FunctionDeclaration(fd) = sub_item {
                        let mut new_fd = fd.clone();
                        new_fd.name.name.name = format!("{}{}", inst_prefix, fd.name.name.name);
                        for p in &mut new_fd.ports {
                            if let Some(def) = &p.default {
                                p.default = Some(rewrite_expr(def, &inst_prefix, &rewrite_port_map, &local_names, &sub_interface_map));
                            }
                        }
                        new_fd.items = fd.items.iter()
                            .map(|s| rewrite_stmt(s, &inst_prefix, &rewrite_port_map, &local_names, &sub_interface_map))
                            .collect();
                        elab.functions.insert(new_fd.name.name.name.clone(), new_fd);
                    }
                    if let ModuleItem::TaskDeclaration(td) = sub_item {
                        let mut new_td = td.clone();
                        new_td.name.name.name = format!("{}{}", inst_prefix, td.name.name.name);
                        for p in &mut new_td.ports {
                            if let Some(def) = &p.default {
                                p.default = Some(rewrite_expr(def, &inst_prefix, &rewrite_port_map, &local_names, &sub_interface_map));
                            }
                        }
                        new_td.items = td.items.iter()
                            .map(|s| rewrite_stmt(s, &inst_prefix, &rewrite_port_map, &local_names, &sub_interface_map))
                            .collect();
                        elab.tasks.insert(new_td.name.name.name.clone(), new_td);
                    }
                    if let ModuleItem::ContinuousAssign(ca) = sub_item {
                        for (lhs, rhs) in &ca.assignments {
                            let new_lhs = rewrite_expr(lhs, &inst_prefix, &rewrite_port_map, &local_names, &sub_interface_map);
                            let new_rhs = rewrite_expr(rhs, &inst_prefix, &rewrite_port_map, &local_names, &sub_interface_map);
                            elab.continuous_assigns.push(ContinuousAssignment {
                                lhs: new_lhs, rhs: new_rhs,
                            });
                        }
                    }
                    if let ModuleItem::GateInstantiation(gi) = sub_item {
                        let assigns = gate_inst_to_assign_pairs(gi);
                        for (lhs, rhs) in assigns {
                            let new_lhs = rewrite_expr(&lhs, &inst_prefix, &rewrite_port_map, &local_names, &sub_interface_map);
                            let new_rhs = rewrite_expr(&rhs, &inst_prefix, &rewrite_port_map, &local_names, &sub_interface_map);
                            elab.continuous_assigns.push(ContinuousAssignment {
                                lhs: new_lhs, rhs: new_rhs,
                            });
                        }
                    }
                    if let ModuleItem::NetDeclaration(nd) = sub_item {
                        for decl in &nd.declarators {
                            if let Some(init_expr) = &decl.init {
                                let lhs_name = format!("{}{}", inst_prefix, decl.name.name);
                                let new_lhs = make_ident_expr(&lhs_name);
                                let new_rhs = rewrite_expr(init_expr, &inst_prefix, &rewrite_port_map, &local_names, &sub_interface_map);
                                elab.continuous_assigns.push(ContinuousAssignment {
                                    lhs: new_lhs, rhs: new_rhs,
                                });
                            }
                        }
                    }
                    if let ModuleItem::SpecifyBlock(sb) = sub_item {
                        for p in &sb.paths {
                            let dst_expr = rewrite_expr(
                                &make_ident_expr(&p.dst.name),
                                &inst_prefix,
                                &rewrite_port_map,
                                &local_names,
                                &sub_interface_map,
                            );
                            if let ExprKind::Ident(hier) = &dst_expr.kind {
                                let dst_name = hier.path.iter().map(|s| s.name.name.as_str()).collect::<Vec<_>>().join(".");
                                let d = eval_const_expr(&p.delay, &elab.parameters);
                                elab.specify_delays.insert(dst_name, d);
                            }
                        }
                    }
                    if let ModuleItem::AlwaysConstruct(ac) = sub_item {
                        elab.always_blocks.push(super::elaborate::AlwaysBlock {
                            kind: ac.kind,
                            stmt: rewrite_stmt(&ac.stmt, &inst_prefix, &rewrite_port_map, &local_names, &sub_interface_map),
                        });
                    }
                    if let ModuleItem::InitialConstruct(ic) = sub_item {
                        elab.initial_blocks.push(super::elaborate::InitialBlock {
                            stmt: rewrite_stmt(&ic.stmt, &inst_prefix, &rewrite_port_map, &local_names, &sub_interface_map),
                        });
                    }
                }

                // Recurse into sub-module instantiations
                inline_module_items(elab, sub_mod, &inst_prefix, definitions, &mut sub_interface_map, &sub_merged_params)?;
            }
        }
    }
    Ok(())
}

fn gate_inst_to_assign_pairs(gi: &GateInstantiation) -> Vec<(Expression, Expression)> {
    let mut pairs = Vec::new();
    for inst in &gi.instances {
        if inst.terminals.len() < 2 { continue; }
        let out = inst.terminals[0].clone();
        let in1 = inst.terminals[1].clone();
        match gi.gate_type {
            GateType::And => {
                let mut rhs = in1;
                for i in 2..inst.terminals.len() {
                    rhs = Expression::new(ExprKind::Binary { op: BinaryOp::BitAnd, left: Box::new(rhs), right: Box::new(inst.terminals[i].clone()) }, out.span);
                }
                pairs.push((out, rhs));
            }
            GateType::Or => {
                let mut rhs = in1;
                for i in 2..inst.terminals.len() {
                    rhs = Expression::new(ExprKind::Binary { op: BinaryOp::BitOr, left: Box::new(rhs), right: Box::new(inst.terminals[i].clone()) }, out.span);
                }
                pairs.push((out, rhs));
            }
            GateType::Not => {
                let rhs = Expression::new(ExprKind::Unary { op: UnaryOp::BitNot, operand: Box::new(in1) }, out.span);
                pairs.push((out, rhs));
            }
            _ => {}
        }
    }
    pairs
}

fn gate_inst_to_assigns(gi: &GateInstantiation, elab: &mut ElaboratedModule) {
    let pairs = gate_inst_to_assign_pairs(gi);
    for (lhs, rhs) in pairs {
        elab.continuous_assigns.push(ContinuousAssignment { lhs, rhs });
    }
}

fn make_ident_expr(name: &str) -> Expression {
    Expression::new(ExprKind::Ident(HierarchicalIdentifier {
        root: None,
        path: vec![HierPathSegment { name: Identifier { name: name.to_string(), span: Span::dummy() }, selects: Vec::new() }],
        span: Span::dummy(),
        cached_signal_id: std::cell::Cell::new(None),
    }), Span::dummy())
}

fn rewrite_expr(expr: &Expression, prefix: &str, port_map: &HashMap<String, Expression>, local_names: &std::collections::HashSet<String>, interface_map: &HashMap<String, String>) -> Expression {
    rewrite_expr_impl(expr, prefix, port_map, local_names, interface_map)
}

fn rewrite_expr_impl(expr: &Expression, prefix: &str, port_map: &HashMap<String, Expression>, local_names: &std::collections::HashSet<String>, interface_map: &HashMap<String, String>) -> Expression {
    let new_kind = match &expr.kind {
        ExprKind::Ident(hier) => {
            if hier.root.is_some() { return expr.clone(); }
            if hier.path.is_empty() { return expr.clone(); }
            let name = &hier.path[0].name.name;
            if let Some(if_prefix) = interface_map.get(name) {
                let mut new_hier = hier.clone();
                new_hier.path[0].name.name = if_prefix.clone();
                return Expression::new(ExprKind::Ident(new_hier), expr.span);
            }
            if let Some(mapped) = port_map.get(name) {
                return mapped.clone();
            }
            if local_names.contains(name) {
                let mut new_hier = hier.clone();
                new_hier.path[0].name.name = format!("{}{}", prefix, name);
                ExprKind::Ident(new_hier)
            } else {
                expr.kind.clone()
            }
        }
        ExprKind::Unary { op, operand } => ExprKind::Unary {
            op: *op,
            operand: Box::new(rewrite_expr_impl(operand, prefix, port_map, local_names, interface_map)),
        },
        ExprKind::Binary { op, left, right } => ExprKind::Binary {
            op: *op,
            left: Box::new(rewrite_expr_impl(left, prefix, port_map, local_names, interface_map)),
            right: Box::new(rewrite_expr_impl(right, prefix, port_map, local_names, interface_map)),
        },
        ExprKind::Conditional { condition, then_expr, else_expr } => ExprKind::Conditional {
            condition: Box::new(rewrite_expr_impl(condition, prefix, port_map, local_names, interface_map)),
            then_expr: Box::new(rewrite_expr_impl(then_expr, prefix, port_map, local_names, interface_map)),
            else_expr: Box::new(rewrite_expr_impl(else_expr, prefix, port_map, local_names, interface_map)),
        },
        ExprKind::Concatenation(parts) => ExprKind::Concatenation(
            parts.iter().map(|p| rewrite_expr_impl(p, prefix, port_map, local_names, interface_map)).collect(),
        ),
        ExprKind::Replication { count, exprs } => ExprKind::Replication {
            count: Box::new(rewrite_expr_impl(count, prefix, port_map, local_names, interface_map)),
            exprs: exprs.iter().map(|e| rewrite_expr_impl(e, prefix, port_map, local_names, interface_map)).collect(),
        },
        ExprKind::Index { expr: base, index } => ExprKind::Index {
            expr: Box::new(rewrite_expr_impl(base, prefix, port_map, local_names, interface_map)),
            index: Box::new(rewrite_expr_impl(index, prefix, port_map, local_names, interface_map)),
        },
        ExprKind::RangeSelect { expr: base, kind, left, right } => ExprKind::RangeSelect {
            expr: Box::new(rewrite_expr_impl(base, prefix, port_map, local_names, interface_map)),
            kind: *kind,
            left: Box::new(rewrite_expr_impl(left, prefix, port_map, local_names, interface_map)),
            right: Box::new(rewrite_expr_impl(right, prefix, port_map, local_names, interface_map)),
        },
        ExprKind::MemberAccess { expr: base, member } => {
            let rewritten_base = rewrite_expr_impl(base, prefix, port_map, local_names, interface_map);
            if let ExprKind::Ident(mut hier) = rewritten_base.kind {
                hier.path.push(HierPathSegment {
                    name: member.clone(),
                    selects: Vec::new(),
                });
                ExprKind::Ident(hier)
            } else {
                ExprKind::MemberAccess {
                    expr: Box::new(rewritten_base),
                    member: member.clone(),
                }
            }
        }
        ExprKind::Paren(inner) => ExprKind::Paren(Box::new(rewrite_expr_impl(inner, prefix, port_map, local_names, interface_map))),
        ExprKind::Call { func, args } => ExprKind::Call {
            func: Box::new(rewrite_expr_impl(func, prefix, port_map, local_names, interface_map)),
            args: args.iter().map(|a| rewrite_expr_impl(a, prefix, port_map, local_names, interface_map)).collect(),
        },
        ExprKind::SystemCall { name, args } => ExprKind::SystemCall {
            name: name.clone(),
            args: args.iter().map(|a| rewrite_expr_impl(a, prefix, port_map, local_names, interface_map)).collect(),
        },
        other => other.clone(),
    };
    Expression::new(new_kind, expr.span)
}

fn rewrite_stmt(stmt: &Statement, prefix: &str, port_map: &HashMap<String, Expression>, local_names: &std::collections::HashSet<String>, interface_map: &HashMap<String, String>) -> Statement {
    let new_kind = match &stmt.kind {
        StatementKind::BlockingAssign { lvalue, rvalue } => StatementKind::BlockingAssign {
            lvalue: rewrite_expr(lvalue, prefix, port_map, local_names, interface_map),
            rvalue: rewrite_expr(rvalue, prefix, port_map, local_names, interface_map),
        },
        StatementKind::NonblockingAssign { lvalue, delay, rvalue } => StatementKind::NonblockingAssign {
            lvalue: rewrite_expr(lvalue, prefix, port_map, local_names, interface_map),
            delay: delay.as_ref().map(|d| rewrite_expr(d, prefix, port_map, local_names, interface_map)),
            rvalue: rewrite_expr(rvalue, prefix, port_map, local_names, interface_map),
        },
        StatementKind::Expr(expr) => StatementKind::Expr(rewrite_expr(expr, prefix, port_map, local_names, interface_map)),
        StatementKind::If { unique_priority, condition, then_stmt, else_stmt } => StatementKind::If {
            unique_priority: *unique_priority,
            condition: rewrite_expr(condition, prefix, port_map, local_names, interface_map),
            then_stmt: Box::new(rewrite_stmt(then_stmt, prefix, port_map, local_names, interface_map)),
            else_stmt: else_stmt.as_ref().map(|s| Box::new(rewrite_stmt(s, prefix, port_map, local_names, interface_map))),
        },
        StatementKind::Case { unique_priority, kind, expr, items } => StatementKind::Case {
            unique_priority: *unique_priority,
            kind: *kind,
            expr: rewrite_expr(expr, prefix, port_map, local_names, interface_map),
            items: items.iter().map(|item| CaseItem {
                patterns: item.patterns.iter().map(|p| rewrite_expr(p, prefix, port_map, local_names, interface_map)).collect(),
                is_default: item.is_default,
                stmt: rewrite_stmt(&item.stmt, prefix, port_map, local_names, interface_map),
                span: item.span,
            }).collect(),
        },
        StatementKind::For { init, condition, step, body } => StatementKind::For {
            init: init.iter().map(|fi| match fi {
                ForInit::VarDecl { data_type, name, init } => ForInit::VarDecl {
                    data_type: data_type.clone(),
                    name: name.clone(),
                    init: rewrite_expr(init, prefix, port_map, local_names, interface_map),
                },
                ForInit::Assign { lvalue, rvalue } => ForInit::Assign {
                    lvalue: rewrite_expr(lvalue, prefix, port_map, local_names, interface_map),
                    rvalue: rewrite_expr(rvalue, prefix, port_map, local_names, interface_map),
                },
            }).collect(),
            condition: condition.as_ref().map(|c| rewrite_expr(c, prefix, port_map, local_names, interface_map)),
            step: step.iter().map(|s| rewrite_expr(s, prefix, port_map, local_names, interface_map)).collect(),
            body: Box::new(rewrite_stmt(body, prefix, port_map, local_names, interface_map)),
        },
        StatementKind::While { condition, body } => StatementKind::While {
            condition: rewrite_expr(condition, prefix, port_map, local_names, interface_map),
            body: Box::new(rewrite_stmt(body, prefix, port_map, local_names, interface_map)),
        },
        StatementKind::Repeat { count, body } => StatementKind::Repeat {
            count: rewrite_expr(count, prefix, port_map, local_names, interface_map),
            body: Box::new(rewrite_stmt(body, prefix, port_map, local_names, interface_map)),
        },
        StatementKind::Forever { body } => StatementKind::Forever {
            body: Box::new(rewrite_stmt(body, prefix, port_map, local_names, interface_map)),
        },
        StatementKind::TimingControl { control, stmt: body } => StatementKind::TimingControl {
            control: match control {
                TimingControl::Delay(e) => TimingControl::Delay(rewrite_expr(e, prefix, port_map, local_names, interface_map)),
                TimingControl::Event(ev) => TimingControl::Event(rewrite_event_control(ev, prefix, port_map, local_names, interface_map)),
            },
            stmt: Box::new(rewrite_stmt(body, prefix, port_map, local_names, interface_map)),
        },
        StatementKind::SeqBlock { name, stmts } => StatementKind::SeqBlock {
            name: name.clone(),
            stmts: stmts.iter().map(|s| rewrite_stmt(s, prefix, port_map, local_names, interface_map)).collect(),
        },
        StatementKind::EventTrigger { nonblocking, name, span } => StatementKind::EventTrigger {
            nonblocking: *nonblocking,
            name: Identifier {
                name: if let Some(mapped) = port_map.get(&name.name) {
                    if let ExprKind::Ident(h) = &mapped.kind { h.path[0].name.name.clone() } else { name.name.clone() }
                } else if local_names.contains(&name.name) {
                    format!("{}.{}", prefix, name.name)
                } else {
                    name.name.clone()
                },
                span: name.span,
            },
            span: *span,
        },
        StatementKind::ParBlock { name, stmts, join_type } => StatementKind::ParBlock {
            name: name.clone(),
            stmts: stmts.iter().map(|s| rewrite_stmt(s, prefix, port_map, local_names, interface_map)).collect(),
            join_type: *join_type,
        },
        other => other.clone(),
    };
    Statement::new(new_kind, stmt.span)
}

fn rewrite_event_control(ev: &EventControl, prefix: &str, port_map: &HashMap<String, Expression>, local_names: &std::collections::HashSet<String>, interface_map: &HashMap<String, String>) -> EventControl {
    match ev {
        EventControl::Identifier(id) => {
            let name = if let Some(mapped) = port_map.get(&id.name) {
                if let ExprKind::Ident(h) = &mapped.kind { h.path[0].name.name.clone() } else { id.name.clone() }
            } else if local_names.contains(&id.name) {
                format!("{}.{}", prefix, id.name)
            } else {
                id.name.clone()
            };
            EventControl::Identifier(Identifier { name, span: id.span })
        }
        EventControl::HierIdentifier(expr) => EventControl::HierIdentifier(rewrite_expr(expr, prefix, port_map, local_names, interface_map)),
        EventControl::EventExpr(exprs) => EventControl::EventExpr(exprs.iter().map(|e| {
            EventExpr {
                edge: e.edge,
                expr: rewrite_expr(&e.expr, prefix, port_map, local_names, interface_map),
                iff: e.iff.as_ref().map(|i| rewrite_expr(i, prefix, port_map, local_names, interface_map)),
                span: e.span,
            }
        }).collect()),
        other => other.clone(),
    }
}
fn process_import(imp: &ImportDeclaration, elab: &mut ElaboratedModule, defs: &HashMap<String, Definition>) -> Result<(), String> {
    for ii in &imp.items {
        let pkg_name = &ii.package.name;
        if let Some(Definition::Package(pkg)) = defs.get(pkg_name) {
            if let Some(sym) = &ii.item {
                let sym_name = &sym.name;
                let mut found = false;
                for pi in &pkg.items {
                    match pi {
                        PackageItem::Parameter(pd) => {
                            if let ParameterKind::Data { data_type, assignments } = &pd.kind {
                                for assign in assignments {
                                    if &assign.name.name == sym_name {
                                        let mut width = resolve_type_width(data_type, Some(&elab.parameters), Some(&elab.typedefs));
                                        let mut signed = is_type_signed(data_type);
                                        let is_real = is_type_real(data_type);
                                        if matches!(data_type, DataType::Implicit { .. }) {
                                            width = 32;
                                            signed = true;
                                        }
                                        let v = if let Some(init) = &assign.init {
                                            let mut v = eval_const_expr_val(init, &elab.parameters).resize(width);
                                            if signed { v.is_signed = true; }
                                            if is_real { v = Value::from_f64(v.to_f64()); }
                                            v
                                        } else { Value::zero(width) };
                                        elab.parameters.insert(assign.name.name.clone(), v.clone());
                                        elab.signals.insert(assign.name.name.clone(), Signal {
                                            is_const: false, name: assign.name.name.clone(),
                                            width, is_signed: signed, is_real, direction: None,
                                            value: v, type_name: get_type_name(data_type),
                                        });
                                        found = true;
                                        break;
                                    }
                                }
                            }
                        }
                        PackageItem::Typedef(td) => {
                            if &td.name.name == sym_name {
                                process_typedef(td, elab);
                                found = true;
                            }
                        }
                        PackageItem::Function(fd) => {
                            if &fd.name.name.name == sym_name {
                                elab.functions.insert(fd.name.name.name.clone(), fd.clone());
                                found = true;
                            }
                        }
                        PackageItem::Task(td) => {
                            if &td.name.name.name == sym_name {
                                elab.tasks.insert(td.name.name.name.clone(), td.clone());
                                found = true;
                            }
                        }
                        PackageItem::DPIImport(di) => {
                            if &dpi_proto_sv_name(&di.proto) == sym_name {
                                register_dpi_import(di, elab)?;
                                found = true;
                            }
                        }
                        PackageItem::Class(c) => {
                            if &c.name.name == sym_name {
                                elab.classes.insert(c.name.name.clone(), elaborate_class(c));
                                found = true;
                            }
                        }
                        PackageItem::Data(dd) => {
                            if dd.declarators.iter().any(|decl| &decl.name.name == sym_name) {
                                let width = match &dd.data_type {
                                    DataType::TypeReference { name, .. } => {
                                        elab.typedefs.get(&name.name.name).copied().unwrap_or(resolve_type_width(&dd.data_type, Some(&elab.parameters), Some(&elab.typedefs)))
                                    }
                                    _ => resolve_type_width(&dd.data_type, Some(&elab.parameters), Some(&elab.typedefs)),
                                };
                                let is_signed = is_type_signed(&dd.data_type);
                                let is_real = is_type_real(&dd.data_type);
                                for decl in &dd.declarators {
                                    if &decl.name.name == sym_name {
                                        let v = if let Some(init) = &decl.init {
                                            eval_const_expr_val(init, &elab.parameters).resize(width)
                                        } else { Value::zero(width) };
                                        elab.signals.insert(decl.name.name.clone(), Signal {
                                            is_const: dd.const_kw, name: decl.name.name.clone(),
                                            width, is_signed, is_real, direction: None,
                                            value: v, type_name: get_type_name(&dd.data_type),
                                        });
                                    }
                                }
                                found = true;
                            }
                        }
                        _ => {}
                    }
                    if found { break; }
                }
                if !found {
                    return Err(format!("Symbol '{}' not found in package '{}'", sym_name, pkg_name));
                }
            } else {
                // Wildcard import
                for pi in &pkg.items {
                    match pi {
                        PackageItem::Parameter(pd) => {
                            if let ParameterKind::Data { data_type, assignments } = &pd.kind {
                                let mut width = resolve_type_width(data_type, Some(&elab.parameters), Some(&elab.typedefs));
                                let mut signed = is_type_signed(data_type);
                                let is_real = is_type_real(data_type);
                                if matches!(data_type, DataType::Implicit { .. }) {
                                    width = 32;
                                    signed = true;
                                }
                                for assign in assignments {
                                    if let Some(init) = &assign.init {
                                        let mut v = eval_const_expr_val(init, &elab.parameters).resize(width);
                                        if signed { v.is_signed = true; }
                                        if is_real { v = Value::from_f64(v.to_f64()); }
                                        elab.parameters.insert(assign.name.name.clone(), v.clone());
                                        elab.signals.insert(assign.name.name.clone(), Signal {
                                            is_const: false, name: assign.name.name.clone(),
                                            width, is_signed: signed, is_real, direction: None,
                                            value: v, type_name: get_type_name(data_type),
                                        });
                                    }
                                }
                            }
                        }
                        PackageItem::Typedef(td) => {
                            process_typedef(td, elab);
                        }
                        PackageItem::Function(fd) => {
                            elab.functions.insert(fd.name.name.name.clone(), fd.clone());
                        }
                        PackageItem::Task(td) => {
                            elab.tasks.insert(td.name.name.name.clone(), td.clone());
                        }
                        PackageItem::DPIImport(di) => {
                            register_dpi_import(di, elab)?;
                        }
                        PackageItem::Class(c) => {
                            elab.classes.insert(c.name.name.clone(), elaborate_class(c));
                        }
                        PackageItem::Data(dd) => {
                            let width = match &dd.data_type {
                                DataType::TypeReference { name, .. } => {
                                    elab.typedefs.get(&name.name.name).copied().unwrap_or(resolve_type_width(&dd.data_type, Some(&elab.parameters), Some(&elab.typedefs)))
                                }
                                _ => resolve_type_width(&dd.data_type, Some(&elab.parameters), Some(&elab.typedefs)),
                            };
                            let is_signed = is_type_signed(&dd.data_type);
                            let is_real = is_type_real(&dd.data_type);
                            for decl in &dd.declarators {
                                let v = if let Some(init) = &decl.init {
                                    eval_const_expr_val(init, &elab.parameters).resize(width)
                                } else { Value::zero(width) };
                                elab.signals.insert(decl.name.name.clone(), Signal {
                                    is_const: dd.const_kw, name: decl.name.name.clone(),
                                    width, is_signed, is_real, direction: None,
                                    value: v, type_name: get_type_name(&dd.data_type),
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
        } else {
            return Err(format!("Package '{}' not found", pkg_name));
        }
    }
    Ok(())
}
