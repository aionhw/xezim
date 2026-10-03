//! Assignment-compatibility checks (part of the second-pass should-fail lint,
//! see `should_fail_lint`).
//!
//! The elaborator converts freely between types; these LRM rules reject an
//! assignment outright:
//!
//! - §6.19.3: an enum variable takes only a value of its own enum type — a
//!   member, a variable or expression of that type, or an explicit cast.
//! - §8.15: a class handle takes only a handle of its own class or of a class
//!   derived from it; a base-class handle needs `$cast`.
//! - §7.6: an unpacked array takes only an unpacked array of the same shape
//!   whose element type is equivalent (§6.22.2), never a packed value.
//! - §8.7: `new` without brackets constructs a class (or covergroup) handle.
//! - §26.3: `P::name` names an item declared in package `P`; a task, or a
//!   void function, is not called in an expression (§13.3, §13.4.1).
//! - A declaration's range, a parameter's value, a part-select's bounds and
//!   the selects of a continuous-assignment target are constant expressions
//!   (§6.20, §7.4, §11.5.1, §10.3): they cannot read a variable or a net.
//! - §6.21: an automatic variable is not written by a nonblocking assignment
//!   and not used in a procedural continuous assignment, whose target is a
//!   whole variable (§10.6.1).
//! - §6.19: an enum's base type is an integer atom type, or an integer vector
//!   type with one packed dimension at most; §7.2.1: every member of a packed
//!   struct or union is packed.
//! - A struct member reference names a member of the struct (§7.2), and an
//!   assignment target is a variable or net, not a parameter or other
//!   constant (§6.20).
//! - §11.4.12: a replication count is a non-negative, known constant, and a
//!   zero-count replication stands only beside a sized operand of a
//!   concatenation; a concatenation takes no real operand. §6.24.1: a casting
//!   size is positive; `$signed`/`$unsigned` take an integral argument.
//! - §26.6: `export P::n` exports a name the package imported from `P` and
//!   does not declare itself.
//!
//! The same rules apply to declaration initializers, `return` values and
//! subroutine input arguments, which are assignments too (§13.5).
//!
//! Types are inferred from the AST with lexical scoping. Anything not known
//! for certain — a parameter-sized width, a hierarchical or package-scoped
//! name, a cast, a parameterized class, a class whose base is not visible —
//! is `Unknown` and never produces an error.

use std::rc::Rc;
use xezim_core::hasher::{HashMap, HashSet};

use xezim_core::SourceDefinition;
use xezim_core::ast::Span;
use xezim_core::ast::decl::{
    ClassDeclaration, ClassItem, ClassMethodKind, FunctionDeclaration, FunctionPort,
    ImportDeclaration, ModuleItem, PackageItem, ParameterDeclaration, ParameterKind,
    TaskDeclaration, TypedefDeclaration,
};
use xezim_core::ast::expr::{
    BinaryOp, ExprKind, Expression, NumberBase, NumberLiteral, RangeKind, UnaryOp,
};
use xezim_core::ast::module::PortList;
use xezim_core::ast::stmt::{ForInit, ProceduralContinuous, Statement, StatementKind};
use xezim_core::ast::types::{
    DataType, EnumType, IntegerAtomType, IntegerVectorType, Lifetime, PackedDimension,
    PortDirection, RealType, Signing, SimpleType, StructUnionKind, StructUnionType,
    UnpackedDimension,
};
use xezim_core::elaborate::ElaboratedModule;

#[derive(Clone, Debug, PartialEq)]
enum Ty {
    Unknown,
    /// Packed integral value; `bits` only when every dimension is a literal.
    Int {
        bits: Option<u64>,
        signed: bool,
        four: bool,
    },
    /// One enum declaration: `key` identifies it, `name` is for messages.
    Enum {
        key: Rc<str>,
        name: Rc<str>,
    },
    Real {
        short: bool,
    },
    Str,
    Class(Rc<str>),
    Null,
    Void,
    Struct(Rc<StructTy>),
    /// Unpacked dimensions, outermost first, around a non-unpacked element.
    Unpacked {
        dims: Vec<Dim>,
        elem: Box<Ty>,
    },
}

#[derive(Debug, PartialEq)]
struct StructTy {
    key: String,
    packed: bool,
    members: Vec<(String, Ty)>,
}

/// A folded constant: a known integer, or a value with x/z bits.
#[derive(Clone, Copy, PartialEq)]
enum CVal {
    Int(i64),
    Xz,
}

/// What a typedef names, as an enum base type (§6.19).
#[derive(Clone, Copy)]
enum BaseKind {
    Atom,
    Vector,
    Illegal(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Dim {
    Fixed(Option<u64>),
    Dynamic,
    Queue,
    Assoc,
}

impl Ty {
    fn int(bits: Option<u64>, signed: bool, four: bool) -> Ty {
        Ty::Int { bits, signed, four }
    }

    /// Integral in the §6.11.1 sense (packed), which an enum or an unpacked
    /// array can never take implicitly.
    fn is_packed_value(&self) -> bool {
        match self {
            Ty::Int { .. } | Ty::Enum { .. } => true,
            Ty::Struct(s) => s.packed,
            _ => false,
        }
    }

    fn describe(&self) -> String {
        match self {
            Ty::Int { bits, signed, four } => {
                let base = if *four { "logic" } else { "bit" };
                let sign = if *signed { " signed" } else { "" };
                match bits {
                    Some(1) => format!("{base}{sign}"),
                    Some(n) => format!("{base}{sign} [{}:0]", n - 1),
                    None => format!("{base}{sign} vector"),
                }
            }
            Ty::Enum { name, .. } => format!("enum {name}"),
            Ty::Real { short: true } => "shortreal".into(),
            Ty::Real { short: false } => "real".into(),
            Ty::Str => "string".into(),
            Ty::Class(c) => format!("class {c}"),
            Ty::Null => "null".into(),
            Ty::Void => "void".into(),
            Ty::Struct(s) => {
                if s.packed {
                    "packed struct".into()
                } else {
                    "unpacked struct".into()
                }
            }
            Ty::Unpacked { dims, elem } => {
                let mut s = elem.describe();
                s.push_str(" $");
                for d in dims {
                    match d {
                        Dim::Fixed(Some(n)) => s.push_str(&format!("[{n}]")),
                        Dim::Fixed(None) => s.push_str("[N]"),
                        Dim::Dynamic => s.push_str("[]"),
                        Dim::Queue => s.push_str("[$]"),
                        Dim::Assoc => s.push_str("[*]"),
                    }
                }
                s
            }
            Ty::Unknown => "unknown".into(),
        }
    }
}

/// Wrap `base` in unpacked dimensions (outermost first).
fn with_dims(base: Ty, dims: &[UnpackedDimension]) -> Ty {
    if dims.is_empty() || base == Ty::Unknown {
        return base;
    }
    let mut out: Vec<Dim> = dims.iter().map(unpacked_dim).collect();
    let elem = match base {
        Ty::Unpacked { dims: inner, elem } => {
            out.extend(inner);
            *elem
        }
        other => other,
    };
    Ty::Unpacked {
        dims: out,
        elem: Box::new(elem),
    }
}

fn unpacked_dim(d: &UnpackedDimension) -> Dim {
    match d {
        UnpackedDimension::Range { left, right, .. } => {
            Dim::Fixed(match (lit_i64(left), lit_i64(right)) {
                (Some(l), Some(r)) => Some(l.abs_diff(r) + 1),
                _ => None,
            })
        }
        UnpackedDimension::Expression { expr, .. } => {
            Dim::Fixed(lit_i64(expr).and_then(|n| u64::try_from(n).ok()))
        }
        UnpackedDimension::Unsized(_) => Dim::Dynamic,
        UnpackedDimension::Queue { .. } => Dim::Queue,
        UnpackedDimension::Associative { .. } => Dim::Assoc,
    }
}

/// A literal integer expression (no identifiers), folded.
fn lit_i64(e: &Expression) -> Option<i64> {
    match &e.kind {
        ExprKind::Number(NumberLiteral::Integer { value, base, .. }) => {
            let radix = match base {
                NumberBase::Decimal => 10,
                NumberBase::Binary => 2,
                NumberBase::Octal => 8,
                NumberBase::Hex => 16,
            };
            if value.contains('_') {
                i64::from_str_radix(&value.replace('_', ""), radix).ok()
            } else {
                i64::from_str_radix(value, radix).ok()
            }
        }
        ExprKind::Paren(i) => lit_i64(i),
        ExprKind::Unary {
            op: UnaryOp::Minus,
            operand,
        } => lit_i64(operand).map(|v| -v),
        ExprKind::Binary { op, left, right } => {
            let (l, r) = (lit_i64(left)?, lit_i64(right)?);
            match op {
                BinaryOp::Add => l.checked_add(r),
                BinaryOp::Sub => l.checked_sub(r),
                BinaryOp::Mul => l.checked_mul(r),
                _ => None,
            }
        }
        _ => None,
    }
}

fn packed_bits(dims: &[PackedDimension]) -> Option<u64> {
    let mut bits: u64 = 1;
    for d in dims {
        match d {
            PackedDimension::Range { left, right, .. } => {
                bits = bits.checked_mul(lit_i64(left)?.abs_diff(lit_i64(right)?) + 1)?
            }
            PackedDimension::Unsized(_) => return None,
        }
    }
    Some(bits)
}

struct Sig {
    ret: Ty,
    ports: Vec<PortSig>,
    is_task: bool,
}

struct PortSig {
    name: String,
    dir: PortDirection,
    ty: Ty,
    has_default: bool,
}

struct ClassInfo {
    base: Option<String>,
    /// Parameterized, or extends a specialization: identity is not the name.
    param: bool,
    interface: bool,
    props: HashMap<String, Ty>,
    methods: HashMap<String, Rc<Sig>>,
}

#[derive(Default)]
struct Scope {
    vars: HashMap<String, Ty>,
    types: HashMap<String, Ty>,
    subs: HashMap<String, Option<Rc<Sig>>>,
    /// Variables, nets and ports — unlike parameters, genvars and enum
    /// members, never part of a constant expression.
    nonconst: HashSet<String>,
    /// Automatic variables (§6.21).
    autos: HashSet<String>,
    /// Typedefs as enum base types.
    bases: HashMap<String, BaseKind>,
    /// Values of parameters that cannot be overridden here (localparams, and
    /// parameters of a definition nothing instantiates).
    consts: HashMap<String, CVal>,
    /// Block-level `static` declarations, which may be `localparam`s (the
    /// parser gives both one shape): neither constant nor variable here.
    unsure: HashSet<String>,
    /// A class scope whose ancestry is not fully visible: an unqualified name
    /// may be an inherited member, so a lookup that reaches it gives up.
    opaque: bool,
}

enum Layer<'a> {
    Shared(&'a Scope),
    Own(Scope),
}

impl Layer<'_> {
    fn get(&self) -> &Scope {
        match self {
            Layer::Shared(s) => s,
            Layer::Own(s) => s,
        }
    }
}

/// What the checker knows about the rest of the design.
struct Env<'e> {
    class_names: &'e HashMap<String, usize>,
    packages: &'e HashMap<String, Scope>,
    unit: Option<&'e Scope>,
    classes: &'e HashMap<String, Option<Rc<ClassInfo>>>,
    /// Every name each package declares; None when it re-exports or holds
    /// items this pass does not model.
    pkg_members: &'e HashMap<String, Option<HashSet<String>>>,
}

struct Ck<'a> {
    env: Env<'a>,
    elab: &'a ElaboratedModule,
    owner: String,
    stack: Vec<Layer<'a>>,
    cur_class: Option<String>,
    ret: Option<Ty>,
    /// Inside an automatic task or function (§6.21).
    auto_ctx: bool,
    /// The definition is declared `automatic`.
    default_auto: bool,
    /// Declarations whose legality was already reported (a type is resolved
    /// once per use).
    seen: HashSet<usize>,
    /// Parameters of the definition keep their declared values (nothing
    /// instantiates it, so nothing overrides them).
    params_fixed: bool,
    /// Inside a generate block, which may not be elaborated: counts that
    /// depend on a parameter are not judged there.
    gen_depth: u32,
    errs: Vec<String>,
}

impl<'a> Ck<'a> {
    fn new(env: Env<'a>, elab: &'a ElaboratedModule, owner: &str) -> Self {
        let mut stack = Vec::new();
        if let Some(u) = env.unit {
            stack.push(Layer::Shared(u));
        }
        Ck {
            env,
            elab,
            owner: owner.to_string(),
            stack,
            cur_class: None,
            ret: None,
            auto_ctx: false,
            default_auto: false,
            seen: HashSet::default(),
            params_fixed: false,
            gen_depth: 0,
            errs: Vec::new(),
        }
    }

    fn push(&mut self) {
        self.stack.push(Layer::Own(Scope::default()));
    }

    fn pop(&mut self) -> Scope {
        match self.stack.pop() {
            Some(Layer::Own(s)) => s,
            _ => Scope::default(),
        }
    }

    fn top(&mut self) -> &mut Scope {
        if !matches!(self.stack.last(), Some(Layer::Own(_))) {
            self.push();
        }
        match self.stack.last_mut() {
            Some(Layer::Own(s)) => s,
            _ => unreachable!(),
        }
    }

    fn lookup_var(&self, n: &str) -> Option<Ty> {
        for l in self.stack.iter().rev() {
            let s = l.get();
            if let Some(t) = s.vars.get(n) {
                return Some(t.clone());
            }
            if s.opaque {
                return None;
            }
        }
        None
    }

    fn lookup_type(&self, n: &str) -> Option<Ty> {
        for l in self.stack.iter().rev() {
            let s = l.get();
            if let Some(t) = s.types.get(n) {
                return Some(t.clone());
            }
            if s.opaque {
                return Some(Ty::Unknown);
            }
        }
        None
    }

    fn lookup_sub(&self, n: &str) -> Option<Rc<Sig>> {
        for l in self.stack.iter().rev() {
            let s = l.get();
            if let Some(t) = s.subs.get(n) {
                return t.clone();
            }
            // Any other name in scope (a variable, a type) hides it too.
            if s.opaque || s.vars.contains_key(n) || s.types.contains_key(n) {
                return None;
            }
        }
        None
    }

    fn is_class_name(&self, n: &str) -> bool {
        self.env.class_names.get(n) == Some(&1)
    }

    fn class(&self, n: &str) -> Option<&Rc<ClassInfo>> {
        self.env.classes.get(n).and_then(|c| c.as_ref())
    }

    // ---- types -------------------------------------------------------------

    fn resolve(&mut self, dt: &DataType) -> Ty {
        match dt {
            DataType::IntegerAtom { kind, signing, .. } => {
                let (bits, four) = match kind {
                    IntegerAtomType::Byte => (8, false),
                    IntegerAtomType::ShortInt => (16, false),
                    IntegerAtomType::Int => (32, false),
                    IntegerAtomType::LongInt => (64, false),
                    IntegerAtomType::Integer => (32, true),
                    IntegerAtomType::Time => (64, true),
                };
                let signed = match signing {
                    Some(Signing::Signed) => true,
                    Some(Signing::Unsigned) => false,
                    None => *kind != IntegerAtomType::Time,
                };
                Ty::int(Some(bits), signed, four)
            }
            DataType::IntegerVector {
                kind,
                signing,
                dimensions,
                ..
            } => Ty::int(
                packed_bits(dimensions),
                *signing == Some(Signing::Signed),
                *kind != IntegerVectorType::Bit,
            ),
            DataType::Implicit {
                signing,
                dimensions,
                ..
            } => Ty::int(
                packed_bits(dimensions),
                *signing == Some(Signing::Signed),
                true,
            ),
            DataType::Real { kind, .. } => Ty::Real {
                short: *kind == RealType::ShortReal,
            },
            DataType::Simple {
                kind: SimpleType::String,
                ..
            } => Ty::Str,
            DataType::Simple { .. } => Ty::Unknown,
            DataType::Void(_) => Ty::Void,
            DataType::Enum(et) => self.resolve_enum(et, None),
            DataType::Struct(su) => {
                if su.tagged || !su.dimensions.is_empty() {
                    return Ty::Unknown;
                }
                let mut members = Vec::new();
                for m in &su.members {
                    let t = self.resolve(&m.data_type);
                    for d in &m.declarators {
                        members.push((d.name.name.clone(), with_dims(t.clone(), &d.dimensions)));
                    }
                }
                if su.packed {
                    self.check_packed_members(su, &members);
                }
                Ty::Struct(Rc::new(StructTy {
                    key: format!("struct@{}", su.span.start),
                    packed: su.packed,
                    members,
                }))
            }
            DataType::TypeReference {
                name,
                dimensions,
                type_args,
                ..
            } => {
                if !dimensions.is_empty() || !type_args.is_empty() {
                    return Ty::Unknown;
                }
                let n = name.name.name.as_str();
                if let Some(scope) = name.single_scope() {
                    return self
                        .env
                        .packages
                        .get(&scope.name)
                        .and_then(|p| p.types.get(n).cloned())
                        .unwrap_or(Ty::Unknown);
                }
                if let Some(t) = self.lookup_type(n) {
                    return t;
                }
                if self.is_class_name(n) {
                    return Ty::Class(n.into());
                }
                Ty::Unknown
            }
            DataType::Interface { .. } => Ty::Unknown,
        }
    }

    fn resolve_enum(&mut self, et: &EnumType, name: Option<&str>) -> Ty {
        self.check_enum_base(et);
        if !et.dimensions.is_empty() || et.members.is_empty() {
            return Ty::Unknown;
        }
        let key: Rc<str> = format!("{}@{}", et.members[0].name.name, et.span.start).into();
        let t = Ty::Enum {
            key,
            name: name.unwrap_or("<anonymous>").into(),
        };
        // §6.19: the members are constants of the enum type, declared in the
        // scope that declares the enum.
        for m in &et.members {
            if m.range.is_none() {
                self.top().vars.insert(m.name.name.clone(), t.clone());
            }
        }
        t
    }

    fn declare_typedef(&mut self, td: &TypedefDeclaration) {
        // `typedef class C;` (parsed as a typedef of `void`) only forward-declares
        // the class; typing C as void made a call to a method returning C look
        // like a call to a void function.
        if td.forward || matches!(td.data_type, DataType::Void(_)) {
            return;
        }
        let t = match &td.data_type {
            DataType::Enum(et) => self.resolve_enum(et, Some(&td.name.name)),
            dt => self.resolve(dt),
        };
        let t = with_dims(t, &td.dimensions);
        let base = self.base_kind(td);
        let top = self.top();
        top.types.insert(td.name.name.clone(), t);
        match base {
            Some(b) => top.bases.insert(td.name.name.clone(), b),
            None => top.bases.remove(&td.name.name),
        };
    }

    fn base_kind(&self, td: &TypedefDeclaration) -> Option<BaseKind> {
        if let Some(d) = td.dimensions.first() {
            return Some(BaseKind::Illegal(match d {
                UnpackedDimension::Unsized(_) => "a dynamic array",
                UnpackedDimension::Queue { .. } => "a queue",
                UnpackedDimension::Associative { .. } => "an associative array",
                _ => "an unpacked array",
            }));
        }
        Some(match &td.data_type {
            DataType::IntegerAtom { .. } => BaseKind::Atom,
            DataType::IntegerVector { .. } => BaseKind::Vector,
            DataType::Enum(_) => BaseKind::Illegal("an enum"),
            DataType::Struct(su) if su.kind == StructUnionKind::Union => {
                BaseKind::Illegal("a union")
            }
            DataType::Struct(_) => BaseKind::Illegal("a struct"),
            DataType::Real { .. } => BaseKind::Illegal("a real type"),
            DataType::Simple {
                kind: SimpleType::String,
                ..
            } => BaseKind::Illegal("string"),
            DataType::TypeReference {
                name,
                dimensions,
                type_args,
                ..
            } if name.scopes.is_empty() && type_args.is_empty() => {
                match self.lookup_base(&name.name.name)? {
                    BaseKind::Atom if !dimensions.is_empty() => {
                        BaseKind::Illegal("an integer atom type with a packed dimension")
                    }
                    b => b,
                }
            }
            _ => return None,
        })
    }

    fn lookup_base(&self, n: &str) -> Option<BaseKind> {
        for l in self.stack.iter().rev() {
            let s = l.get();
            if s.types.contains_key(n) {
                return s.bases.get(n).copied();
            }
            if s.opaque {
                return None;
            }
        }
        None
    }

    /// §6.19 `enum_base_type`: an integer atom type, an integer vector type
    /// with at most one packed dimension, or a typedef of either (an atom
    /// typedef taking no packed dimension).
    fn check_enum_base(&mut self, et: &EnumType) {
        let Some(base) = &et.base_type else {
            return;
        };
        let why = match &**base {
            DataType::IntegerVector { dimensions, .. } if dimensions.len() > 1 => {
                Some("a vector with more than one packed dimension")
            }
            DataType::Real { .. } => Some("a real type"),
            DataType::Simple {
                kind: SimpleType::String,
                ..
            } => Some("string"),
            DataType::TypeReference {
                name,
                dimensions,
                type_args,
                ..
            } if name.scopes.is_empty() && type_args.is_empty() => {
                match self.lookup_base(&name.name.name) {
                    Some(BaseKind::Atom) if !dimensions.is_empty() => {
                        Some("an integer atom type with a packed dimension")
                    }
                    Some(BaseKind::Illegal(w)) => Some(w),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(w) = why
            && self.seen.insert(et.span.start)
        {
            self.report(
                et.span,
                format!(
                    "an enum base type must be an integer atom or vector type, not {w} \
                     (IEEE 1800-2017 §6.19)"
                ),
            );
        }
    }

    /// §7.2.1: only packed types go in a packed struct or union.
    fn check_packed_members(&mut self, su: &StructUnionType, members: &[(String, Ty)]) {
        let kind = match su.kind {
            StructUnionKind::Struct => "struct",
            StructUnionKind::Union => "union",
        };
        let decls = su.members.iter().flat_map(|m| {
            let simple = matches!(m.data_type, DataType::Simple { .. });
            m.declarators.iter().map(move |d| (d, simple))
        });
        for ((d, simple), (name, t)) in decls.zip(members) {
            let why = match t {
                Ty::Unpacked { dims, .. } => Some(match dims[0] {
                    Dim::Dynamic => "a dynamic array",
                    Dim::Queue => "a queue",
                    Dim::Assoc => "an associative array",
                    Dim::Fixed(_) => "an unpacked array",
                }),
                Ty::Real { .. } => Some("a real"),
                Ty::Str => Some("a string"),
                Ty::Class(_) => Some("a class handle"),
                Ty::Struct(s) if !s.packed => Some("an unpacked struct"),
                _ if simple => Some("an event or chandle"),
                _ => None,
            };
            if let Some(w) = why
                && self.seen.insert(d.span.start)
            {
                self.report(
                    d.span,
                    format!(
                        "member '{name}' of a packed {kind} must be of a packed type, not {w} \
                         (IEEE 1800-2017 §7.2.1)"
                    ),
                );
            }
        }
    }

    fn declare_param(&mut self, pd: &ParameterDeclaration) {
        match &pd.kind {
            ParameterKind::Data {
                data_type,
                assignments,
            } => {
                // A real parameter is typed (a concatenation takes no real
                // operand); other parameters stay untyped here.
                let t = match data_type {
                    DataType::Real { .. } => self.resolve(data_type),
                    _ => Ty::Unknown,
                };
                for a in assignments {
                    let v = a.init.as_ref().and_then(|e| self.eval_const(e));
                    // A negative default of a parameter (`width_p = -1`) is a
                    // placeholder that every instantiation overrides; library
                    // modules elaborated on their own keep it.
                    let v = match v {
                        _ if !pd.local && !self.params_fixed => None,
                        Some(CVal::Int(n)) if n < 0 && !pd.local => None,
                        v => v,
                    };
                    let top = self.top();
                    top.vars.insert(a.name.name.clone(), t.clone());
                    match v {
                        Some(v) => top.consts.insert(a.name.name.clone(), v),
                        None => top.consts.remove(&a.name.name),
                    };
                }
            }
            ParameterKind::Type { assignments } => {
                for a in assignments {
                    self.top().types.insert(a.name.name.clone(), Ty::Unknown);
                }
            }
        }
    }

    fn declare_import(&mut self, imp: &ImportDeclaration) {
        let packages = self.env.packages;
        for it in &imp.items {
            let Some(p) = packages.get(&it.package.name) else {
                continue;
            };
            match &it.item {
                // §26.3: a wildcard import is visible only where no local
                // declaration of the name exists — a layer under this scope.
                None => {
                    let own = self.pop();
                    self.stack.push(Layer::Shared(p));
                    self.stack.push(Layer::Own(own));
                }
                Some(n) => {
                    let n = &n.name;
                    if let Some(t) = p.vars.get(n) {
                        let t = t.clone();
                        self.top().vars.insert(n.clone(), t);
                        if p.nonconst.contains(n) {
                            self.top().nonconst.insert(n.clone());
                        }
                    }
                    if let Some(t) = p.types.get(n) {
                        let t = t.clone();
                        self.top().types.insert(n.clone(), t);
                        // An imported enum type brings its members along (§26.3
                        // needs them imported explicitly, but they are the same
                        // constants, so typing them is safe).
                    }
                    if let Some(s) = p.subs.get(n) {
                        let s = s.clone();
                        self.top().subs.insert(n.clone(), s);
                    }
                }
            }
        }
    }

    fn sig_of_function(&mut self, f: &FunctionDeclaration) -> Rc<Sig> {
        let ret = self.resolve(&f.return_type);
        let ports = self.port_sigs(&f.ports, &f.items);
        Rc::new(Sig {
            ret,
            ports,
            is_task: false,
        })
    }

    fn sig_of_task(&mut self, t: &TaskDeclaration) -> Rc<Sig> {
        let ports = self.port_sigs(&t.ports, &t.items);
        Rc::new(Sig {
            ret: Ty::Void,
            ports,
            is_task: true,
        })
    }

    /// A non-ANSI port redeclared in the body (`input [31:0] x; T x;`) has
    /// the redeclaration's type.
    fn port_sigs(&mut self, ports: &[FunctionPort], body: &[Statement]) -> Vec<PortSig> {
        let mut redecl: HashMap<&str, (&DataType, &[UnpackedDimension])> = HashMap::default();
        for s in body {
            if let StatementKind::VarDecl {
                data_type,
                declarators,
                ..
            } = &s.kind
            {
                for d in declarators {
                    redecl.insert(d.name.name.as_str(), (data_type, &d.dimensions));
                }
            }
        }
        ports
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let redeclared = redecl.get(p.name.name.as_str()).copied();
                let (dt, dims) = redeclared.unwrap_or((&p.data_type, &p.dimensions));
                let t = if redeclared.is_none() && inherits_port_type(ports, i) {
                    Ty::Unknown
                } else {
                    self.resolve(dt)
                };
                PortSig {
                    name: p.name.name.clone(),
                    dir: p.direction,
                    ty: with_dims(t, dims),
                    has_default: p.default.is_some(),
                }
            })
            .collect()
    }

    /// Record every declaration of a module-like or generate scope in the
    /// top layer, in source order.
    fn declare_items(&mut self, items: &[ModuleItem]) {
        for it in items {
            match it {
                ModuleItem::TypedefDeclaration(td) => self.declare_typedef(td),
                ModuleItem::ParameterDeclaration(pd) | ModuleItem::LocalparamDeclaration(pd) => {
                    self.declare_param(pd)
                }
                ModuleItem::ImportDeclaration(imp) => self.declare_import(imp),
                ModuleItem::DataDeclaration(d) => {
                    let t = self.resolve(&d.data_type);
                    for dc in &d.declarators {
                        let vt = with_dims(t.clone(), &dc.dimensions);
                        self.declare_value(&dc.name.name, vt);
                    }
                }
                ModuleItem::NetDeclaration(n) => {
                    let t = self.resolve(&n.data_type);
                    for dc in &n.declarators {
                        let vt = with_dims(t.clone(), &dc.dimensions);
                        self.declare_value(&dc.name.name, vt);
                    }
                }
                ModuleItem::PortDeclaration(pd) => {
                    let t = self.resolve(&pd.data_type);
                    for dc in &pd.declarators {
                        let vt = with_dims(t.clone(), &dc.dimensions);
                        self.declare_value(&dc.name.name, vt);
                    }
                }
                ModuleItem::GenvarDeclaration(g) => {
                    for n in &g.names {
                        self.top()
                            .vars
                            .insert(n.name.clone(), Ty::int(Some(32), true, false));
                    }
                }
                ModuleItem::FunctionDeclaration(f) if f.name.scopes.is_empty() => {
                    let s = self.sig_of_function(f);
                    self.top().subs.insert(f.name.name.name.clone(), Some(s));
                }
                ModuleItem::TaskDeclaration(t) if t.name.scopes.is_empty() => {
                    let s = self.sig_of_task(t);
                    self.top().subs.insert(t.name.name.name.clone(), Some(s));
                }
                // A let is called like a function: hide any outer binding.
                ModuleItem::LetDeclaration(l) => {
                    self.top().subs.insert(l.name.name.clone(), None);
                }
                ModuleItem::ClassDeclaration(c) => {
                    self.top().types.remove(&c.name.name);
                }
                _ => {}
            }
        }
        // DPI imports: their names must not resolve to an outer subroutine.
        for it in items {
            if let ModuleItem::DPIImport(d) = it {
                if let Some(n) = dpi_name(d) {
                    self.top().subs.insert(n, None);
                }
            }
        }
    }

    fn check_ansi_port_dims(&mut self, ports: &PortList) {
        if let PortList::Ansi(ps) = ports {
            for p in ps {
                if let Some(dt) = &p.data_type {
                    self.check_const_dims(&p.name.name, dt, &p.dimensions);
                }
            }
        }
    }

    fn declare_ansi_ports(&mut self, ports: &PortList) {
        if let PortList::Ansi(ps) = ports {
            for p in ps {
                let t = match &p.data_type {
                    Some(dt) => self.resolve(dt),
                    None => Ty::Unknown,
                };
                let t = with_dims(t, &p.dimensions);
                self.declare_value(&p.name.name, t);
            }
        }
    }

    /// Build the ClassInfo of `c`, declared in the current scope.
    fn class_info(&mut self, c: &ClassDeclaration) -> ClassInfo {
        self.push();
        for p in &c.params {
            self.declare_param(p);
        }
        for it in &c.items {
            match it {
                ClassItem::Typedef(td) => self.declare_typedef(td),
                ClassItem::Parameter(pd) => self.declare_param(pd),
                ClassItem::Import(imp) => self.declare_import(imp),
                _ => {}
            }
        }
        let mut props = HashMap::default();
        let mut methods = HashMap::default();
        for it in &c.items {
            match it {
                ClassItem::Property(p) => {
                    let t = self.resolve(&p.data_type);
                    for d in &p.declarators {
                        props.insert(d.name.name.clone(), with_dims(t.clone(), &d.dimensions));
                    }
                }
                ClassItem::Method(m) => match &m.kind {
                    ClassMethodKind::Function(f)
                    | ClassMethodKind::PureVirtual(f)
                    | ClassMethodKind::Extern(f) => {
                        let s = self.sig_of_function(f);
                        methods.insert(f.name.name.name.clone(), s);
                    }
                    ClassMethodKind::Task(t) => {
                        let s = self.sig_of_task(t);
                        methods.insert(t.name.name.name.clone(), s);
                    }
                },
                _ => {}
            }
        }
        self.pop();
        let base = c.extends.as_ref().map(|e| e.name.name.clone());
        ClassInfo {
            base,
            param: !c.params.is_empty() || c.extends.as_ref().is_some_and(|e| !e.args.is_empty()),
            interface: c.is_interface,
            props,
            methods,
        }
    }

    /// Walk a class's ancestry: Some(chain) when every class on it is known
    /// and unparameterized.
    fn chain(&self, name: &str) -> Option<Vec<Rc<ClassInfo>>> {
        let mut out = Vec::new();
        let mut cur = name.to_string();
        loop {
            let ci = self.class(&cur)?.clone();
            if ci.param || out.len() > 64 {
                return None;
            }
            let next = ci.base.clone();
            out.push(ci);
            match next {
                Some(b) => cur = b,
                None => return Some(out),
            }
        }
    }

    /// §8.15: is class `sub` the class `sup` or derived from it?
    fn derives(&self, sub: &str, sup: &str) -> Option<bool> {
        let sup_ci = self.class(sup)?;
        if sup_ci.param || sup_ci.interface {
            return None;
        }
        let chain = self.chain(sub)?;
        let mut cur = sub.to_string();
        for ci in &chain {
            if cur == sup {
                return Some(true);
            }
            if let Some(b) = &ci.base {
                cur = b.clone();
            }
        }
        Some(cur == sup)
    }

    fn class_prop(&self, class: &str, member: &str) -> Ty {
        let Some(chain) = self.chain(class) else {
            return Ty::Unknown;
        };
        for ci in chain {
            if let Some(t) = ci.props.get(member) {
                return t.clone();
            }
        }
        Ty::Unknown
    }

    fn class_method(&self, class: &str, member: &str) -> Option<Rc<Sig>> {
        for ci in self.chain(class)? {
            if let Some(s) = ci.methods.get(member) {
                return Some(s.clone());
            }
        }
        None
    }

    // ---- expressions -------------------------------------------------------

    fn ty_of(&self, e: &Expression) -> Ty {
        match &e.kind {
            ExprKind::Number(NumberLiteral::Integer {
                size,
                signed,
                base,
                value,
                ..
            }) => Ty::int(
                Some(size.unwrap_or(32) as u64),
                *signed || (size.is_none() && *base == NumberBase::Decimal),
                value
                    .chars()
                    .any(|c| matches!(c, 'x' | 'X' | 'z' | 'Z' | '?')),
            ),
            ExprKind::Number(NumberLiteral::Real(_)) => Ty::Real { short: false },
            ExprKind::Ident(h) => {
                if h.root.is_some() || h.path.is_empty() {
                    return Ty::Unknown;
                }
                let first = &h.path[0];
                let mut t = if first.name.name == "this" {
                    match &self.cur_class {
                        Some(c) => Ty::Class(c.as_str().into()),
                        None => return Ty::Unknown,
                    }
                } else {
                    match self.lookup_var(&first.name.name) {
                        Some(t) => t,
                        None => return Ty::Unknown,
                    }
                };
                for s in &first.selects {
                    t = self.index_ty(t, s);
                }
                for seg in &h.path[1..] {
                    t = self.member_ty(t, &seg.name.name);
                    for s in &seg.selects {
                        t = self.index_ty(t, s);
                    }
                }
                t
            }
            ExprKind::Index { expr, index } => {
                let t = self.ty_of(expr);
                self.index_ty(t, index)
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => match self.ty_of(expr) {
                Ty::Unpacked { mut dims, elem } => {
                    let n = match (lit_i64(left), lit_i64(right)) {
                        (Some(l), Some(r)) => Some(l.abs_diff(r) + 1),
                        _ => None,
                    };
                    dims[0] = Dim::Fixed(n);
                    Ty::Unpacked { dims, elem }
                }
                t if t.is_packed_value() => Ty::int(None, false, four_state(&t)),
                _ => Ty::Unknown,
            },
            ExprKind::MemberAccess { expr, member } => {
                let t = self.ty_of(expr);
                self.member_ty(t, &member.name)
            }
            ExprKind::Unary { op, operand } => {
                let t = self.ty_of(operand);
                match op {
                    UnaryOp::LogNot
                    | UnaryOp::BitAnd
                    | UnaryOp::BitNand
                    | UnaryOp::BitOr
                    | UnaryOp::BitNor
                    | UnaryOp::BitXor
                    | UnaryOp::BitXnor
                        if t.is_packed_value() =>
                    {
                        Ty::int(Some(1), false, four_state(&t))
                    }
                    UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitNot => match t {
                        Ty::Real { .. } if *op != UnaryOp::BitNot => t,
                        t if t.is_packed_value() => {
                            let (bits, signed) = int_shape(&t);
                            Ty::int(bits, signed, four_state(&t))
                        }
                        _ => Ty::Unknown,
                    },
                    _ => Ty::Unknown,
                }
            }
            ExprKind::Binary { op, left, right } => {
                let (l, r) = (self.ty_of(left), self.ty_of(right));
                let numeric = |t: &Ty| t.is_packed_value() || matches!(t, Ty::Real { .. });
                match op {
                    BinaryOp::Eq
                    | BinaryOp::Neq
                    | BinaryOp::CaseEq
                    | BinaryOp::CaseNeq
                    | BinaryOp::WildcardEq
                    | BinaryOp::WildcardNeq
                    | BinaryOp::LogAnd
                    | BinaryOp::LogOr
                    | BinaryOp::LogImplies
                    | BinaryOp::LogEquiv
                    | BinaryOp::Lt
                    | BinaryOp::Leq
                    | BinaryOp::Gt
                    | BinaryOp::Geq
                        if l != Ty::Unknown && r != Ty::Unknown =>
                    {
                        Ty::int(Some(1), false, false)
                    }
                    BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod
                    | BinaryOp::Power
                    | BinaryOp::BitAnd
                    | BinaryOp::BitOr
                    | BinaryOp::BitXor
                    | BinaryOp::BitXnor
                        if numeric(&l) && numeric(&r) =>
                    {
                        if matches!(l, Ty::Real { .. }) || matches!(r, Ty::Real { .. }) {
                            Ty::Real { short: false }
                        } else {
                            let (_, ls) = int_shape(&l);
                            let (_, rs) = int_shape(&r);
                            Ty::int(None, ls && rs, four_state(&l) || four_state(&r))
                        }
                    }
                    BinaryOp::ShiftLeft
                    | BinaryOp::ShiftRight
                    | BinaryOp::ArithShiftLeft
                    | BinaryOp::ArithShiftRight
                        if l.is_packed_value() && numeric(&r) =>
                    {
                        let (bits, signed) = int_shape(&l);
                        Ty::int(bits, signed, four_state(&l))
                    }
                    _ => Ty::Unknown,
                }
            }
            ExprKind::Conditional {
                then_expr,
                else_expr,
                ..
            } => {
                let (a, b) = (self.ty_of(then_expr), self.ty_of(else_expr));
                if a == b { a } else { Ty::Unknown }
            }
            ExprKind::Call { func, .. } => match &func.kind {
                ExprKind::Ident(h)
                    if h.root.is_none()
                        && h.path.len() == 1
                        && h.path[0].selects.is_empty()
                        && h.path[0].name.name != "new" =>
                {
                    match self.lookup_sub(&h.path[0].name.name) {
                        Some(s) => s.ret.clone(),
                        None => Ty::Unknown,
                    }
                }
                _ => Ty::Unknown,
            },
            ExprKind::Null => Ty::Null,
            ExprKind::This => match &self.cur_class {
                Some(c) => Ty::Class(c.as_str().into()),
                None => Ty::Unknown,
            },
            _ => Ty::Unknown,
        }
    }

    fn index_ty(&self, t: Ty, index: &Expression) -> Ty {
        if matches!(index.kind, ExprKind::Range(..)) {
            return Ty::Unknown;
        }
        match t {
            Ty::Unpacked { mut dims, elem } => {
                dims.remove(0);
                if dims.is_empty() {
                    *elem
                } else {
                    Ty::Unpacked { dims, elem }
                }
            }
            t if t.is_packed_value() => Ty::int(None, false, four_state(&t)),
            _ => Ty::Unknown,
        }
    }

    fn member_ty(&self, t: Ty, member: &str) -> Ty {
        match t {
            Ty::Struct(s) => s
                .members
                .iter()
                .find(|(n, _)| n == member)
                .map(|(_, t)| t.clone())
                .unwrap_or(Ty::Unknown),
            Ty::Class(c) => self.class_prop(&c, member),
            _ => Ty::Unknown,
        }
    }

    // ---- checks ------------------------------------------------------------

    /// §7.2: `s.m` names a member of struct `s`.
    fn check_struct_members(&mut self, e: &Expression) {
        let mut missing: Vec<(String, Span)> = Vec::new();
        visit_expr(e, &mut |x| {
            let (base, members): (Ty, Vec<&xezim_core::ast::Identifier>) = match &x.kind {
                ExprKind::MemberAccess { expr, member } => (self.ty_of(expr), vec![member]),
                ExprKind::Ident(h)
                    if h.root.is_none()
                        && h.path.len() > 1
                        && h.path[0].selects.is_empty()
                        && h.path[0].name.name != "this" =>
                {
                    match self.lookup_var(&h.path[0].name.name) {
                        Some(t) => (t, h.path[1..].iter().map(|s| &s.name).collect()),
                        None => return,
                    }
                }
                _ => return,
            };
            let mut t = base;
            for m in members {
                if let Ty::Struct(st) = &t
                    && !st.members.iter().any(|(n, _)| *n == m.name)
                {
                    missing.push((m.name.clone(), m.span));
                    return;
                }
                t = self.member_ty(t, &m.name);
            }
        });
        for (m, span) in missing {
            self.report(
                span,
                format!("'{m}' is not a member of the struct or union (IEEE 1800-2017 §7.2)"),
            );
        }
    }

    /// A parameter, genvar or enum member (not a variable, and not a
    /// block-level `static` that may be either).
    fn is_definite_const(&self, n: &str) -> bool {
        for l in self.stack.iter().rev() {
            let s = l.get();
            if s.vars.contains_key(n) {
                return !s.nonconst.contains(n) && !s.unsure.contains(n);
            }
            if s.opaque {
                return false;
            }
        }
        false
    }

    /// §6.20: a parameter, localparam, genvar or enum member is a constant and
    /// cannot be assigned.
    fn check_target_is_variable(&mut self, lv: &Expression) {
        let mut cur = lv;
        loop {
            match &cur.kind {
                ExprKind::Index { expr, .. }
                | ExprKind::RangeSelect { expr, .. }
                | ExprKind::MemberAccess { expr, .. }
                | ExprKind::Paren(expr) => cur = expr,
                ExprKind::Concatenation(xs) => {
                    for x in xs {
                        self.check_target_is_variable(x);
                    }
                    return;
                }
                ExprKind::Ident(h) if h.root.is_none() && !h.path.is_empty() => {
                    let n = &h.path[0].name.name;
                    if h.path.len() == 1 && self.is_definite_const(n) {
                        self.report(
                            h.path[0].name.span,
                            format!(
                                "'{n}' is a parameter or other constant, not a variable, and \
                                 cannot be assigned (IEEE 1800-2017 §6.20)"
                            ),
                        );
                    }
                    return;
                }
                _ => return,
            }
        }
    }

    fn report(&mut self, span: Span, msg: String) {
        let loc = xezim_core::elaborate::span_location_of(self.elab, span, &self.owner);
        self.errs.push(match loc {
            Some(l) => format!("{l}: {msg}"),
            None => msg,
        });
    }

    /// `target = rhs` where `target` has type `lt`; `what` names the target
    /// (built only for a report).
    fn check_value(&mut self, lt: &Ty, rhs: &Expression, what: impl FnOnce() -> String) {
        if is_new_call(rhs) {
            // `new(args)` and `new[n](init)` share one AST shape; only the
            // bare `new` is certainly a class constructor.
            if matches!(rhs.kind, ExprKind::Ident(_))
                && matches!(
                    lt,
                    Ty::Int { .. }
                        | Ty::Enum { .. }
                        | Ty::Real { .. }
                        | Ty::Str
                        | Ty::Unpacked { .. }
                )
            {
                let what = what();
                self.report(
                    rhs.span,
                    format!(
                        "'new' can only be assigned to a class or covergroup handle, not to \
                         {what} of type {} (IEEE 1800-2017 §8.7)",
                        lt.describe()
                    ),
                );
            }
            return;
        }
        let rt = match typed_new_class(rhs) {
            Some(c) if self.is_class_name(c) => Ty::Class(c.into()),
            Some(_) => return,
            None => self.ty_of(rhs),
        };
        if let Some(why) = self.incompatible(lt, &rt) {
            let what = what();
            self.report(
                rhs.span,
                format!(
                    "cannot assign {} to {what} of type {}: {why}",
                    rt.describe(),
                    lt.describe()
                ),
            );
        }
    }

    /// Why a value of type `rt` cannot be assigned to type `lt` (None when it
    /// can, or when that is not certain).
    fn incompatible(&self, lt: &Ty, rt: &Ty) -> Option<String> {
        match lt {
            Ty::Enum { key, .. } => match rt {
                Ty::Enum { key: k2, .. } if k2 != key => Some(
                    "an enum takes only its own members, values of its own type, or an explicit \
                     cast (IEEE 1800-2017 §6.19.3)"
                        .into(),
                ),
                Ty::Int { .. } | Ty::Real { .. } => Some(
                    "an enum takes only its own members, values of its own type, or an explicit \
                     cast (IEEE 1800-2017 §6.19.3)"
                        .into(),
                ),
                _ => None,
            },
            Ty::Class(a) => match rt {
                Ty::Class(b) if self.derives(b, a) == Some(false) => Some(format!(
                    "class {b} is not {a} or a class derived from it; a base-class handle \
                     needs $cast (IEEE 1800-2017 §8.15)"
                )),
                _ => None,
            },
            Ty::Unpacked { dims: ld, elem: le } => match rt {
                t if t.is_packed_value() || matches!(t, Ty::Real { .. } | Ty::Str) => Some(
                    "an unpacked array takes only an unpacked array (IEEE 1800-2017 §7.6)".into(),
                ),
                Ty::Unpacked { dims: rd, elem: re } => {
                    if ld.contains(&Dim::Assoc) || rd.contains(&Dim::Assoc) {
                        return None;
                    }
                    if ld.len() != rd.len() {
                        return Some(format!(
                            "the arrays have {} and {} unpacked dimensions (IEEE 1800-2017 §7.6)",
                            ld.len(),
                            rd.len()
                        ));
                    }
                    for (a, b) in ld.iter().zip(rd.iter()) {
                        if let (Dim::Fixed(Some(x)), Dim::Fixed(Some(y))) = (a, b)
                            && x != y
                        {
                            return Some(format!(
                                "the arrays have {x} and {y} elements (IEEE 1800-2017 §7.6)"
                            ));
                        }
                    }
                    // An enum array takes no plain integral array (§6.19.3);
                    // an enum array into an integral one is accepted, as by
                    // the reference simulator.
                    let enum_from_int =
                        matches!(**le, Ty::Enum { .. }) && matches!(**re, Ty::Int { .. });
                    if enum_from_int || equivalent(le, re) == Some(false) {
                        return Some(format!(
                            "element types {} and {} are not equivalent (IEEE 1800-2017 \
                             §6.22.2, §7.6)",
                            le.describe(),
                            re.describe()
                        ));
                    }
                    None
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn check_assign(&mut self, lv: &Expression, rv: &Expression) {
        self.check_const_selects(lv);
        self.check_struct_members(lv);
        self.check_target_is_variable(lv);
        // `{r1, r2} = v` with a real `r1` is a concatenation too.
        self.check_operands(lv);
        let lt = self.ty_of(lv);
        if lt == Ty::Unknown {
            return;
        }
        self.check_value(&lt, rv, || format!("'{}'", expr_name(lv)));
    }

    /// §13.5: argument binding — too many or unbound arguments, and an input
    /// argument is an assignment to its formal.
    /// §13.5: argument binding — an argument past the last formal, or a
    /// formal left without an actual and without a default, is an error; an
    /// input argument is an assignment to its formal.
    fn check_call(&mut self, name: &str, sig: &Sig, args: &[Expression], span: Span) {
        let mut bound = vec![false; sig.ports.len()];
        let mut pos = 0usize;
        for a in args {
            let (i, e) = match &a.kind {
                ExprKind::NamedArg { name: an, expr } => {
                    match sig.ports.iter().position(|p| p.name == an.name) {
                        Some(i) => (i, expr.as_deref()),
                        None => return,
                    }
                }
                _ => {
                    pos += 1;
                    (pos - 1, Some(a))
                }
            };
            let Some(p) = sig.ports.get(i) else {
                self.report(
                    span,
                    format!(
                        "too many arguments to '{name}': it has {} formal argument(s) \
                         (IEEE 1800-2017 §13.5)",
                        sig.ports.len()
                    ),
                );
                return;
            };
            let Some(e) = e.filter(|e| !matches!(e.kind, ExprKind::Empty)) else {
                continue;
            };
            bound[i] = true;
            if p.dir == PortDirection::Input {
                let t = p.ty.clone();
                self.check_value(&t, e, || format!("argument '{}' of '{name}'", p.name));
            }
        }
        if let Some(p) = sig
            .ports
            .iter()
            .zip(&bound)
            .find_map(|(p, b)| (!b && !p.has_default).then_some(p))
        {
            self.report(
                span,
                format!(
                    "no actual argument for formal '{}' of '{name}', which has no default \
                     (IEEE 1800-2017 §13.5)",
                    p.name
                ),
            );
        }
    }

    fn walk_expr(&mut self, e: &Expression) {
        self.check_const_selects(e);
        self.check_struct_members(e);
        self.check_operands(e);
        self.walk_value(e, true);
    }

    /// Walk `e`; `value` is false for a call made as a statement.
    fn walk_value(&mut self, e: &Expression, value: bool) {
        match &e.kind {
            ExprKind::Call { func, args } => {
                for a in args {
                    self.walk_value(a, true);
                }
                match &func.kind {
                    ExprKind::Ident(h)
                        if h.root.is_none()
                            && h.path.len() == 1
                            && h.path[0].selects.is_empty() =>
                    {
                        let n = &h.path[0].name.name;
                        if n != "new"
                            && let Some(sig) = self.lookup_sub(n)
                        {
                            self.check_call(n, &sig, args, e.span);
                            self.check_call_kind(n, &sig, value, e.span);
                        }
                    }
                    ExprKind::MemberAccess { expr, member } if self.package_of(expr).is_some() => {
                        let pkg = self.package_of(expr).unwrap_or_default();
                        let n = format!("{pkg}::{}", member.name);
                        if !self.check_package_member(pkg, &member.name, member.span) {
                            return;
                        }
                        let scope = self.env.packages.get(pkg);
                        match scope.and_then(|p| p.subs.get(&member.name)) {
                            Some(Some(sig)) => {
                                let sig = sig.clone();
                                self.check_call(&n, &sig, args, e.span);
                                self.check_call_kind(&n, &sig, value, e.span);
                            }
                            Some(None) => {}
                            None if scope.is_some_and(|p| p.vars.contains_key(&member.name)) => {
                                self.report(
                                    member.span,
                                    format!(
                                        "'{n}' is not a function or task (IEEE 1800-2017 §13.4)"
                                    ),
                                );
                            }
                            None => {}
                        }
                    }
                    ExprKind::MemberAccess { expr, member } => {
                        self.walk_value(expr, true);
                        if let Ty::Class(c) = self.ty_of(expr)
                            && let Some(sig) = self.class_method(&c, &member.name)
                        {
                            self.check_call(&member.name, &sig, args, e.span);
                        }
                    }
                    _ => {}
                }
            }
            ExprKind::AssignExpr { lvalue, rvalue } => {
                self.walk_value(rvalue, true);
                self.check_assign(lvalue, rvalue);
            }
            ExprKind::Unary { operand, .. } => self.walk_value(operand, true),
            ExprKind::Binary { left, right, .. } => {
                self.walk_value(left, true);
                self.walk_value(right, true);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.walk_value(condition, true);
                self.walk_value(then_expr, true);
                self.walk_value(else_expr, true);
            }
            ExprKind::Concatenation(xs) => xs.iter().for_each(|x| self.walk_value(x, true)),
            // `void'(f())` is still a call made as a statement.
            ExprKind::Paren(x) => self.walk_value(x, value),
            ExprKind::Index { expr, index } => {
                self.walk_value(expr, true);
                self.walk_value(index, true);
            }
            ExprKind::MemberAccess { expr, member } => match self.package_of(expr) {
                Some(pkg) => {
                    self.check_package_member(pkg, &member.name, member.span);
                }
                None => self.walk_value(expr, true),
            },
            ExprKind::SystemCall { args, .. } => args.iter().for_each(|x| self.walk_value(x, true)),
            _ => {}
        }
    }

    /// §13.3/§13.4.1: a task, or a void function, called where a value is
    /// needed.
    fn check_call_kind(&mut self, name: &str, sig: &Sig, value: bool, span: Span) {
        if !value {
            return;
        }
        if sig.is_task {
            self.report(
                span,
                format!("task '{name}' cannot be called in an expression (IEEE 1800-2017 §13.3)"),
            );
        } else if sig.ret == Ty::Void {
            self.report(
                span,
                format!(
                    "void function '{name}' has no value to use in an expression \
                     (IEEE 1800-2017 §13.4.1)"
                ),
            );
        }
    }

    /// `P` in `P::name` when it is a package (not a class or a variable).
    fn package_of<'e>(&self, e: &'e Expression) -> Option<&'e str> {
        let ExprKind::Ident(h) = &e.kind else {
            return None;
        };
        if h.root.is_some() || h.path.len() != 1 || !h.path[0].selects.is_empty() {
            return None;
        }
        let n = h.path[0].name.name.as_str();
        (self.env.pkg_members.contains_key(n)
            && !self.env.class_names.contains_key(n)
            && self.lookup_var(n).is_none()
            && self.lookup_type(n).is_none())
        .then_some(n)
    }

    /// §26.3: `P::name` must name an item declared in package `P`. False when
    /// it does not (and the error is reported).
    fn check_package_member(&mut self, pkg: &str, name: &str, span: Span) -> bool {
        let Some(Some(members)) = self.env.pkg_members.get(pkg) else {
            return true;
        };
        if members.contains(name) {
            return true;
        }
        self.report(
            span,
            format!("'{name}' is not declared in package '{pkg}' (IEEE 1800-2017 §26.3)"),
        );
        false
    }

    fn declare_var(&mut self, dt: &DataType, name: &str, dims: &[UnpackedDimension]) -> Ty {
        let t = with_dims(self.resolve(dt), dims);
        self.declare_value(name, t.clone());
        t
    }

    /// A variable, net or port of type `t`.
    fn declare_value(&mut self, name: &str, t: Ty) {
        let top = self.top();
        top.vars.insert(name.to_string(), t);
        top.nonconst.insert(name.to_string());
    }

    fn declare_unsure(&mut self, name: &str, t: Ty) {
        let top = self.top();
        top.vars.insert(name.to_string(), t);
        top.unsure.insert(name.to_string());
    }

    fn mark_auto(&mut self, name: &str) {
        self.top().autos.insert(name.to_string());
    }

    fn is_auto(&self, n: &str) -> bool {
        for l in self.stack.iter().rev() {
            let s = l.get();
            if s.vars.contains_key(n) {
                return s.autos.contains(n);
            }
            if s.opaque {
                return false;
            }
        }
        false
    }

    /// §6.21: an automatic variable is not written by a nonblocking
    /// assignment.
    fn check_nba_target(&mut self, lv: &Expression) {
        if let ExprKind::Ident(h) = &lv.kind
            && h.root.is_none()
            && h.path.len() == 1
            && h.path[0].selects.is_empty()
            && self.is_auto(&h.path[0].name.name)
        {
            self.report(
                lv.span,
                format!(
                    "automatic variable '{}' cannot be written by a nonblocking assignment \
                     (IEEE 1800-2017 §6.21)",
                    h.path[0].name.name
                ),
            );
        }
    }

    /// §10.6: `assign` / `force` inside a procedure. The `assign` target is a
    /// whole variable (§10.6.1), and neither side reads an automatic variable.
    fn check_proc_continuous(&mut self, pc: &ProceduralContinuous) {
        let (lv, rv) = match pc {
            ProceduralContinuous::Assign { lvalue, rvalue } => {
                if has_select(lvalue) {
                    self.report(
                        lvalue.span,
                        "a procedural assign target must be a whole variable, not a bit-, \
                         part- or element-select (IEEE 1800-2017 §10.6.1)"
                            .into(),
                    );
                }
                (lvalue, rvalue)
            }
            ProceduralContinuous::Force { lvalue, rvalue } => (lvalue, rvalue),
            _ => return,
        };
        let mut autos: Vec<(String, Span)> = Vec::new();
        for e in [lv, rv] {
            visit_expr(e, &mut |x| {
                if let ExprKind::Ident(h) = &x.kind
                    && h.root.is_none()
                    && h.path.len() == 1
                    && self.is_auto(&h.path[0].name.name)
                {
                    autos.push((h.path[0].name.name.clone(), x.span));
                }
            });
        }
        if let Some((n, span)) = autos.into_iter().next() {
            self.report(
                span,
                format!(
                    "automatic variable '{n}' cannot be used in a procedural continuous \
                     assignment (IEEE 1800-2017 §10.6)"
                ),
            );
        }
    }

    /// Is a simple name a variable, net or port (not a constant)?
    fn is_nonconst(&self, n: &str) -> bool {
        for l in self.stack.iter().rev() {
            let s = l.get();
            if s.vars.contains_key(n) {
                return s.nonconst.contains(n);
            }
            if s.opaque {
                return false;
            }
        }
        false
    }

    /// The first variable, net or port a constant expression reads. System
    /// function arguments (`$bits(v)`), dotted names (`intf.W`) and called
    /// function names are skipped.
    fn first_nonconst(&self, e: &Expression) -> Option<(String, Span)> {
        let sub = |x: &Expression| self.first_nonconst(x);
        match &e.kind {
            ExprKind::Ident(h) => {
                if h.root.is_some() || h.path.len() != 1 {
                    return None;
                }
                let n = &h.path[0].name.name;
                if self.is_nonconst(n) {
                    return Some((n.clone(), h.path[0].name.span));
                }
                h.path[0].selects.iter().find_map(sub)
            }
            ExprKind::Unary { operand, .. } => sub(operand),
            ExprKind::Binary { left, right, .. } => sub(left).or_else(|| sub(right)),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => sub(condition)
                .or_else(|| sub(then_expr))
                .or_else(|| sub(else_expr)),
            ExprKind::Paren(x) => sub(x),
            ExprKind::Index { expr, index } => sub(expr).or_else(|| sub(index)),
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => sub(expr).or_else(|| sub(left)).or_else(|| sub(right)),
            ExprKind::Range(a, b) => sub(a).or_else(|| sub(b)),
            ExprKind::Concatenation(xs) => xs.iter().find_map(sub),
            ExprKind::Replication { count, exprs } => {
                sub(count).or_else(|| exprs.iter().find_map(sub))
            }
            ExprKind::Call { args, .. } => args.iter().find_map(sub),
            _ => None,
        }
    }

    fn report_nonconst(&mut self, e: &Expression, what: &dyn Fn() -> String, section: &str) {
        if let Some((n, span)) = self.first_nonconst(e) {
            let what = what();
            self.report(
                span,
                format!(
                    "{what} must be a constant expression, but '{n}' is a variable or net \
                     (IEEE 1800-2017 {section})"
                ),
            );
        }
    }

    /// §6.9.1/§7.4: every dimension of a declaration is a constant range.
    fn check_const_dims(&mut self, name: &str, dt: &DataType, dims: &[UnpackedDimension]) {
        let packed: &[PackedDimension] = match dt {
            DataType::IntegerVector { dimensions, .. }
            | DataType::Implicit { dimensions, .. }
            | DataType::TypeReference { dimensions, .. } => dimensions,
            _ => &[],
        };
        let what = || format!("the range of '{name}'");
        for d in packed {
            if let PackedDimension::Range { left, right, .. } = d {
                self.report_nonconst(left, &what, "§6.9.1");
                self.report_nonconst(right, &what, "§6.9.1");
            }
        }
        for d in dims {
            match d {
                UnpackedDimension::Range { left, right, .. } => {
                    self.report_nonconst(left, &what, "§7.4");
                    self.report_nonconst(right, &what, "§7.4");
                }
                UnpackedDimension::Expression { expr, .. } => {
                    self.report_nonconst(expr, &what, "§7.4")
                }
                _ => {}
            }
        }
    }

    fn check_param_values(&mut self, pd: &ParameterDeclaration) {
        if let ParameterKind::Data { assignments, .. } = &pd.kind {
            for a in assignments {
                if let Some(init) = &a.init {
                    let what = || format!("the value of parameter '{}'", a.name.name);
                    self.report_nonconst(init, &what, "§6.20");
                    self.check_operands(init);
                }
            }
        }
    }

    /// Fold a constant expression over literals and fixed parameters.
    fn eval_const(&self, e: &Expression) -> Option<CVal> {
        match &e.kind {
            ExprKind::Number(NumberLiteral::Integer { value, .. })
                if value
                    .chars()
                    .any(|c| matches!(c, 'x' | 'X' | 'z' | 'Z' | '?')) =>
            {
                Some(CVal::Xz)
            }
            ExprKind::Number(_) => lit_i64(e).map(CVal::Int),
            ExprKind::Paren(x) => self.eval_const(x),
            ExprKind::Unary {
                op: UnaryOp::Minus,
                operand,
            } => match self.eval_const(operand)? {
                CVal::Int(v) => Some(CVal::Int(v.checked_neg()?)),
                CVal::Xz => Some(CVal::Xz),
            },
            ExprKind::Binary { op, left, right } => {
                let (l, r) = (self.eval_const(left)?, self.eval_const(right)?);
                let (CVal::Int(l), CVal::Int(r)) = (l, r) else {
                    return Some(CVal::Xz);
                };
                Some(CVal::Int(match op {
                    BinaryOp::Add => l.checked_add(r)?,
                    BinaryOp::Sub => l.checked_sub(r)?,
                    BinaryOp::Mul => l.checked_mul(r)?,
                    _ => return None,
                }))
            }
            ExprKind::Ident(h)
                if self.gen_depth == 0
                    && h.root.is_none()
                    && h.path.len() == 1
                    && h.path[0].selects.is_empty() =>
            {
                let n = &h.path[0].name.name;
                for l in self.stack.iter().rev() {
                    let s = l.get();
                    if s.vars.contains_key(n) {
                        return s.consts.get(n).copied();
                    }
                    if s.opaque {
                        return None;
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// §11.4.12 operand rules (replications, real operands) and §6.24.1 size
    /// casts, everywhere in `e`.
    fn check_operands(&mut self, e: &Expression) {
        let mut errs: Vec<(Span, String)> = Vec::new();
        self.operand_errors(e, false, &mut errs);
        for (span, msg) in errs {
            self.report(span, msg);
        }
    }

    /// `sized_sibling`: `e` is an operand of a concatenation that also has an
    /// operand of positive size.
    fn operand_errors(&self, e: &Expression, sized_sibling: bool, out: &mut Vec<(Span, String)>) {
        let zero = |x: &Expression| self.zero_width(x);
        let operands = |xs: &[Expression], out: &mut Vec<(Span, String)>| {
            let sized = xs.iter().any(|x| !zero(x));
            for x in xs {
                if matches!(self.ty_of(x), Ty::Real { .. }) {
                    out.push((
                        x.span,
                        "a real value cannot be an operand of a concatenation \
                         (IEEE 1800-2017 §11.4.12)"
                            .into(),
                    ));
                }
                self.operand_errors(x, sized, out);
            }
        };
        match &e.kind {
            ExprKind::Concatenation(xs) => operands(xs, out),
            ExprKind::Replication { count, exprs } => {
                match self.eval_const(count) {
                    Some(CVal::Xz) => out.push((
                        count.span,
                        "a replication count cannot contain x or z bits \
                         (IEEE 1800-2017 §11.4.12.1)"
                            .into(),
                    )),
                    Some(CVal::Int(n)) if n < 0 => out.push((
                        count.span,
                        format!(
                            "a replication count cannot be negative ({n}) \
                             (IEEE 1800-2017 §11.4.12.1)"
                        ),
                    )),
                    // §11.4.12.2: a string replication may be zero (the
                    // empty string).
                    Some(CVal::Int(0))
                        if !sized_sibling
                            && !exprs.iter().any(|x| {
                                matches!(x.kind, ExprKind::StringLiteral(_))
                                    || self.ty_of(x) == Ty::Str
                            }) =>
                    {
                        out.push((
                            e.span,
                            "a zero replication must be an operand of a concatenation with an \
                         operand of positive size (IEEE 1800-2017 §11.4.12.1)"
                                .into(),
                        ))
                    }
                    _ => {}
                }
                operands(exprs, out);
            }
            ExprKind::SystemCall { name, args } => {
                let size = match name.as_str() {
                    "$__xz_size_cast" | "$__xz_named_cast" => args.first(),
                    _ => None,
                };
                if let Some(sz) = size
                    && matches!(self.eval_const(sz), Some(CVal::Xz) | Some(CVal::Int(..=0)))
                {
                    out.push((
                        sz.span,
                        "a casting size must be a positive constant (IEEE 1800-2017 §6.24.1)"
                            .into(),
                    ));
                }
                if matches!(name.as_str(), "$signed" | "$unsigned")
                    && args
                        .first()
                        .is_some_and(|a| matches!(self.ty_of(a), Ty::Real { .. }))
                {
                    out.push((
                        e.span,
                        format!(
                            "{name} takes an integral argument, not a real \
                             (IEEE 1800-2017 §11.7)"
                        ),
                    ));
                }
                for a in args {
                    self.operand_errors(a, false, out);
                }
            }
            ExprKind::Unary { operand, .. } => self.operand_errors(operand, false, out),
            ExprKind::Binary { left, right, .. } => {
                self.operand_errors(left, false, out);
                self.operand_errors(right, false, out);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.operand_errors(condition, false, out);
                self.operand_errors(then_expr, false, out);
                self.operand_errors(else_expr, false, out);
            }
            ExprKind::Paren(x) => self.operand_errors(x, sized_sibling, out),
            ExprKind::Call { args, .. } => {
                for a in args {
                    self.operand_errors(a, false, out);
                }
            }
            ExprKind::Index { expr, index } => {
                self.operand_errors(expr, false, out);
                self.operand_errors(index, false, out);
            }
            _ => {}
        }
    }

    /// A zero-count replication, or a concatenation of only such.
    fn zero_width(&self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Replication { count, .. } => self.eval_const(count) == Some(CVal::Int(0)),
            ExprKind::Concatenation(xs) => !xs.is_empty() && xs.iter().all(|x| self.zero_width(x)),
            ExprKind::Paren(x) => self.zero_width(x),
            _ => false,
        }
    }

    /// §11.5.1: a part-select's bounds, and an indexed part-select's width,
    /// are constant expressions — of a packed value; a queue slice `q[a:b]`
    /// takes variable bounds (§7.10.1).
    fn check_const_selects(&mut self, e: &Expression) {
        let mut found: Vec<(&Expression, &'static str)> = Vec::new();
        visit_expr(e, &mut |x| {
            let ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } = &x.kind
            else {
                return;
            };
            if !self.ty_of(expr).is_packed_value() {
                return;
            }
            if *kind == RangeKind::Constant {
                found.push((left, "a part-select bound"));
                found.push((right, "a part-select bound"));
            } else {
                found.push((right, "an indexed part-select width"));
            }
        });
        for (b, what) in found {
            self.report_nonconst(b, &|| what.to_string(), "§11.5.1");
        }
    }

    /// §10.3 (A.8.5): the selects of a continuous-assignment target are
    /// constant expressions (a variable target is held to this too by the
    /// reference simulator).
    fn check_cont_lvalue(&mut self, lv: &Expression) {
        let mut selects: Vec<&Expression> = Vec::new();
        let mut cur = lv;
        loop {
            match &cur.kind {
                ExprKind::Index { expr, index } => {
                    selects.push(index);
                    cur = expr;
                }
                ExprKind::RangeSelect { expr, left, .. } => {
                    selects.push(left);
                    cur = expr;
                }
                ExprKind::Ident(h) if h.root.is_none() && h.path.len() == 1 => {
                    let n = &h.path[0].name.name;
                    selects.extend(h.path[0].selects.iter());
                    let what = || format!("a select of '{n}' in a continuous assignment target");
                    for sel in selects {
                        self.report_nonconst(sel, &what, "§10.3");
                    }
                    return;
                }
                ExprKind::Concatenation(xs) => {
                    for x in xs {
                        self.check_cont_lvalue(x);
                    }
                    return;
                }
                _ => return,
            }
        }
    }

    fn walk_stmt(&mut self, s: &Statement) {
        match &s.kind {
            StatementKind::Expr(e) => {
                self.check_const_selects(e);
                self.walk_value(e, false)
            }
            StatementKind::BlockingAssign { lvalue, rvalue } => {
                self.walk_expr(rvalue);
                self.check_assign(lvalue, rvalue);
            }
            StatementKind::NonblockingAssign { lvalue, rvalue, .. } => {
                self.walk_expr(rvalue);
                self.check_nba_target(lvalue);
                self.check_assign(lvalue, rvalue);
            }
            StatementKind::If {
                condition,
                then_stmt,
                else_stmt,
                ..
            } => {
                self.walk_expr(condition);
                self.walk_stmt(then_stmt);
                if let Some(e) = else_stmt {
                    self.walk_stmt(e);
                }
            }
            StatementKind::Case { expr, items, .. } => {
                self.walk_expr(expr);
                for it in items {
                    self.walk_stmt(&it.stmt);
                }
            }
            StatementKind::For {
                init,
                condition,
                step,
                body,
            } => {
                self.push();
                for i in init {
                    match i {
                        ForInit::VarDecl {
                            data_type,
                            name,
                            init,
                        } => {
                            let t = self.declare_var(data_type, &name.name, &[]);
                            self.mark_auto(&name.name);
                            self.check_value(&t, init, || format!("'{}'", name.name));
                        }
                        ForInit::Assign { lvalue, rvalue } => self.check_assign(lvalue, rvalue),
                    }
                }
                if let Some(c) = condition {
                    self.walk_expr(c);
                }
                for e in step {
                    self.walk_expr(e);
                }
                self.walk_stmt(body);
                self.pop();
            }
            StatementKind::Foreach { vars, body, .. } => {
                self.push();
                for v in vars.iter().flatten() {
                    self.declare_value(&v.name, Ty::Unknown);
                    self.mark_auto(&v.name);
                }
                self.walk_stmt(body);
                self.pop();
            }
            StatementKind::While { condition, body }
            | StatementKind::DoWhile { body, condition } => {
                self.walk_expr(condition);
                self.walk_stmt(body);
            }
            StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::TimingControl { stmt: body, .. }
            | StatementKind::Wait { stmt: body, .. } => self.walk_stmt(body),
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                self.push();
                for st in stmts {
                    self.walk_stmt(st);
                }
                self.pop();
            }
            StatementKind::VarDecl {
                data_type,
                declarators,
                lifetime,
            } => {
                let auto = match lifetime {
                    Some(Lifetime::Automatic) => true,
                    Some(Lifetime::Static) => false,
                    None => self.auto_ctx,
                };
                let t = self.resolve(data_type);
                for d in declarators {
                    self.check_const_dims(&d.name.name, data_type, &d.dimensions);
                    let vt = with_dims(t.clone(), &d.dimensions);
                    if let Some(init) = &d.init {
                        self.walk_expr(init);
                        self.check_value(&vt, init, || format!("'{}'", d.name.name));
                    }
                    // A block-level `localparam` parses as a `static`
                    // declaration, so such a name may be a constant.
                    if *lifetime == Some(Lifetime::Static) {
                        self.declare_unsure(&d.name.name, vt);
                    } else {
                        self.declare_value(&d.name.name, vt);
                    }
                    if auto {
                        self.mark_auto(&d.name.name);
                    }
                }
            }
            StatementKind::Typedef(td) => self.declare_typedef(td),
            StatementKind::ProceduralContinuous(pc) => self.check_proc_continuous(pc),
            StatementKind::Return(Some(e)) => {
                self.walk_expr(e);
                if let Some(rt) = self.ret.clone() {
                    self.check_value(&rt, e, || "the return value".to_string());
                }
            }
            StatementKind::RandCase { items } => {
                for (_, st) in items {
                    self.walk_stmt(st);
                }
            }
            _ => {}
        }
    }

    fn walk_ports(&mut self, ports: &[FunctionPort]) {
        for p in ports {
            self.check_const_dims(&p.name.name, &p.data_type, &p.dimensions);
            let t = self.declare_var(&p.data_type, &p.name.name, &p.dimensions);
            if self.auto_ctx {
                self.mark_auto(&p.name.name);
            }
            if let Some(d) = &p.default {
                self.check_value(&t, d, || format!("'{}'", p.name.name));
            }
        }
        for (i, p) in ports.iter().enumerate() {
            if inherits_port_type(ports, i) {
                self.top().vars.insert(p.name.name.clone(), Ty::Unknown);
            }
        }
    }

    /// §6.21: class methods, and subroutines of an `automatic` definition,
    /// default to automatic.
    fn subroutine_is_auto(&self, lifetime: Option<Lifetime>) -> bool {
        match lifetime {
            Some(l) => l == Lifetime::Automatic,
            None => self.cur_class.is_some() || self.default_auto,
        }
    }

    fn walk_function(&mut self, f: &FunctionDeclaration) {
        let saved_auto = self.auto_ctx;
        self.auto_ctx = self.subroutine_is_auto(f.lifetime);
        self.push();
        self.check_const_dims(&f.name.name.name, &f.return_type, &[]);
        let rt = self.resolve(&f.return_type);
        self.walk_ports(&f.ports);
        // §13.4.1: the function name is a variable of the return type.
        if rt != Ty::Void {
            self.declare_value(&f.name.name.name, rt.clone());
            if self.auto_ctx {
                self.mark_auto(&f.name.name.name);
            }
        }
        let saved = self
            .ret
            .replace(if rt == Ty::Void { Ty::Unknown } else { rt });
        for s in &f.items {
            self.walk_stmt(s);
        }
        self.ret = saved;
        self.pop();
        self.auto_ctx = saved_auto;
    }

    fn walk_task(&mut self, t: &TaskDeclaration) {
        let saved_auto = self.auto_ctx;
        self.auto_ctx = self.subroutine_is_auto(t.lifetime);
        self.push();
        self.walk_ports(&t.ports);
        let saved = self.ret.take();
        for s in &t.items {
            self.walk_stmt(s);
        }
        self.ret = saved;
        self.pop();
        self.auto_ctx = saved_auto;
    }

    fn walk_class(&mut self, c: &ClassDeclaration) {
        if !c.params.is_empty() {
            return;
        }
        let name = c.name.name.as_str();
        let mut scope = Scope::default();
        match self.chain(name) {
            Some(chain) => {
                for ci in chain.iter().rev() {
                    for (n, t) in &ci.props {
                        scope.vars.insert(n.clone(), t.clone());
                        scope.nonconst.insert(n.clone());
                    }
                    for (n, s) in &ci.methods {
                        scope.subs.insert(n.clone(), Some(s.clone()));
                    }
                }
            }
            None => scope.opaque = true,
        }
        self.stack.push(Layer::Own(scope));
        for it in &c.items {
            match it {
                ClassItem::Typedef(td) => self.declare_typedef(td),
                ClassItem::Parameter(pd) => self.declare_param(pd),
                ClassItem::Import(imp) => self.declare_import(imp),
                _ => {}
            }
        }
        let saved = self.cur_class.replace(name.to_string());
        for it in &c.items {
            match it {
                ClassItem::Property(p) => {
                    let t = self.resolve(&p.data_type);
                    for d in &p.declarators {
                        if let Some(init) = &d.init {
                            let vt = with_dims(t.clone(), &d.dimensions);
                            self.check_value(&vt, init, || format!("'{}'", d.name.name));
                        }
                    }
                }
                ClassItem::Method(m) => match &m.kind {
                    ClassMethodKind::Function(f) => self.walk_function(f),
                    ClassMethodKind::Task(t) => self.walk_task(t),
                    _ => {}
                },
                ClassItem::Class(inner) => self.walk_class(inner),
                _ => {}
            }
        }
        self.cur_class = saved;
        self.pop();
    }

    fn walk_items(&mut self, items: &[ModuleItem]) {
        for it in items {
            match it {
                ModuleItem::DataDeclaration(d) => {
                    let t = self.resolve(&d.data_type);
                    for dc in &d.declarators {
                        self.check_const_dims(&dc.name.name, &d.data_type, &dc.dimensions);
                        if let Some(init) = &dc.init {
                            let vt = with_dims(t.clone(), &dc.dimensions);
                            self.walk_expr(init);
                            self.check_value(&vt, init, || format!("'{}'", dc.name.name));
                        }
                    }
                }
                ModuleItem::NetDeclaration(n) => {
                    let t = self.resolve(&n.data_type);
                    for dc in &n.declarators {
                        self.check_const_dims(&dc.name.name, &n.data_type, &dc.dimensions);
                        if let Some(init) = &dc.init {
                            let vt = with_dims(t.clone(), &dc.dimensions);
                            self.walk_expr(init);
                            self.check_value(&vt, init, || format!("'{}'", dc.name.name));
                        }
                    }
                }
                ModuleItem::ContinuousAssign(ca) => {
                    for (l, r) in &ca.assignments {
                        self.walk_expr(r);
                        self.check_cont_lvalue(l);
                        self.check_assign(l, r);
                    }
                }
                ModuleItem::PortDeclaration(pd) => {
                    for dc in &pd.declarators {
                        self.check_const_dims(&dc.name.name, &pd.data_type, &dc.dimensions);
                    }
                }
                ModuleItem::ParameterDeclaration(pd) | ModuleItem::LocalparamDeclaration(pd) => {
                    self.check_param_values(pd)
                }
                ModuleItem::AlwaysConstruct(a) => self.walk_stmt(&a.stmt),
                ModuleItem::InitialConstruct(i) => self.walk_stmt(&i.stmt),
                ModuleItem::FinalConstruct(f) => self.walk_stmt(&f.stmt),
                ModuleItem::FunctionDeclaration(f) if f.name.scopes.is_empty() => {
                    self.walk_function(f)
                }
                ModuleItem::TaskDeclaration(t) if t.name.scopes.is_empty() => self.walk_task(t),
                ModuleItem::ClassDeclaration(c) => self.walk_class(c),
                ModuleItem::GenerateRegion(g) => self.walk_block(&g.items, None),
                ModuleItem::GenerateIf(g) => {
                    for (_, items) in &g.branches {
                        self.walk_block(items, None);
                    }
                }
                ModuleItem::GenerateFor(g) => self.walk_block(&g.items, Some(&g.var)),
                ModuleItem::GenerateCase(g) => {
                    for arm in &g.arms {
                        self.walk_block(&arm.items, None);
                    }
                }
                _ => {}
            }
        }
    }

    fn walk_block(&mut self, items: &[ModuleItem], genvar: Option<&str>) {
        self.gen_depth += 1;
        self.push();
        if let Some(g) = genvar {
            self.top()
                .vars
                .insert(g.to_string(), Ty::int(Some(32), true, false));
        }
        self.declare_items(items);
        self.walk_items(items);
        self.pop();
        self.gen_depth -= 1;
    }
}

/// §13.3: a subroutine port written without a data type takes the previous
/// port's type (`f(string src, dest[$])`) unless its direction is explicit, and
/// the AST does not record which; its type is left unknown.
fn inherits_port_type(ports: &[FunctionPort], i: usize) -> bool {
    i > 0
        && matches!(&ports[i].data_type, DataType::Implicit { signing: None, dimensions, .. }
            if dimensions.is_empty())
}

fn four_state(t: &Ty) -> bool {
    match t {
        Ty::Int { four, .. } => *four,
        _ => true,
    }
}

fn int_shape(t: &Ty) -> (Option<u64>, bool) {
    match t {
        Ty::Int { bits, signed, .. } => (*bits, *signed),
        _ => (None, false),
    }
}

/// §6.22.2 type equivalence of array elements: Some(false) only when the two
/// types are definitely not equivalent.
fn equivalent(a: &Ty, b: &Ty) -> Option<bool> {
    match (a, b) {
        (
            Ty::Int {
                bits: ab,
                signed: asg,
                four: af,
            },
            Ty::Int {
                bits: bb,
                signed: bsg,
                four: bf,
            },
        ) => {
            if asg != bsg || af != bf {
                return Some(false);
            }
            match (ab, bb) {
                (Some(x), Some(y)) => Some(x == y),
                _ => None,
            }
        }
        (Ty::Real { short: x }, Ty::Real { short: y }) => Some(x == y),
        (Ty::Real { .. }, Ty::Int { .. }) | (Ty::Int { .. }, Ty::Real { .. }) => Some(false),
        (Ty::Str, Ty::Str) => Some(true),
        (Ty::Str, Ty::Int { .. } | Ty::Real { .. })
        | (Ty::Int { .. } | Ty::Real { .. }, Ty::Str) => Some(false),
        (Ty::Enum { key: x, .. }, Ty::Enum { key: y, .. }) => Some(x == y),
        _ => None,
    }
}

fn is_new_call(e: &Expression) -> bool {
    let id = match &e.kind {
        ExprKind::Call { func, .. } => func,
        _ => e,
    };
    matches!(&id.kind, ExprKind::Ident(h)
        if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty()
            && h.path[0].name.name == "new")
}

/// `C::new` / `C::new(args)`: the class it constructs.
fn typed_new_class(e: &Expression) -> Option<&str> {
    let m = match &e.kind {
        ExprKind::Call { func, .. } => func,
        _ => e,
    };
    match &m.kind {
        ExprKind::MemberAccess { expr, member } if member.name == "new" => match &expr.kind {
            ExprKind::Ident(h)
                if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty() =>
            {
                Some(h.path[0].name.name.as_str())
            }
            _ => None,
        },
        _ => None,
    }
}

fn expr_name(e: &Expression) -> String {
    match &e.kind {
        ExprKind::Ident(h) => h
            .path
            .iter()
            .map(|s| {
                if s.selects.is_empty() {
                    s.name.name.clone()
                } else {
                    format!("{}[..]", s.name.name)
                }
            })
            .collect::<Vec<_>>()
            .join("."),
        ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => {
            format!("{}[..]", expr_name(expr))
        }
        ExprKind::MemberAccess { expr, member } => format!("{}.{}", expr_name(expr), member.name),
        ExprKind::Paren(x) => expr_name(x),
        _ => "target".into(),
    }
}

fn dpi_name(d: &xezim_core::ast::decl::DPIImport) -> Option<String> {
    use xezim_core::ast::decl::DPIProto;
    Some(match &d.proto {
        DPIProto::Function(f) => f.name.name.name.clone(),
        DPIProto::Task(t) => t.name.name.name.clone(),
    })
}

/// Run the checks over every definition; returns the error messages.
pub fn check(defs: &[&SourceDefinition], elab: &ElaboratedModule) -> Vec<String> {
    // Phase 0: class names (a name declared twice is ambiguous and ignored).
    let mut class_names: HashMap<String, usize> = HashMap::default();
    fn count_items(items: &[ModuleItem], out: &mut HashMap<String, usize>) {
        for it in items {
            match it {
                ModuleItem::ClassDeclaration(c) => count_class(c, out),
                ModuleItem::GenerateRegion(g) => count_items(&g.items, out),
                ModuleItem::GenerateIf(g) => {
                    g.branches.iter().for_each(|(_, i)| count_items(i, out))
                }
                ModuleItem::GenerateFor(g) => count_items(&g.items, out),
                ModuleItem::GenerateCase(g) => {
                    g.arms.iter().for_each(|a| count_items(&a.items, out))
                }
                _ => {}
            }
        }
    }
    fn count_class(c: &ClassDeclaration, out: &mut HashMap<String, usize>) {
        *out.entry(c.name.name.clone()).or_default() += 1;
        for it in &c.items {
            if let ClassItem::Class(inner) = it {
                count_class(inner, out);
            }
        }
    }
    for d in defs {
        match d {
            SourceDefinition::Module(m) => count_items(&m.items, &mut class_names),
            SourceDefinition::Interface(m) => count_items(&m.items, &mut class_names),
            SourceDefinition::Program(m) => count_items(&m.items, &mut class_names),
            SourceDefinition::Class(c) => count_class(c, &mut class_names),
            SourceDefinition::Package(p) => {
                for it in &p.items {
                    if let PackageItem::Class(c) = it {
                        count_class(c, &mut class_names);
                    }
                }
            }
            _ => {}
        }
    }

    let pkg_members: HashMap<String, Option<HashSet<String>>> = defs
        .iter()
        .filter_map(|d| match d {
            SourceDefinition::Package(p) => {
                Some((p.name.name.clone(), package_member_names(&p.items)))
            }
            _ => None,
        })
        .collect();

    // Phase 1: package scopes (twice, so a package sees the ones it imports).
    let no_classes: HashMap<String, Option<Rc<ClassInfo>>> = HashMap::default();
    let empty_pkgs: HashMap<String, Scope> = HashMap::default();
    let mut unit = Scope::default();
    {
        let env = Env {
            class_names: &class_names,
            packages: &empty_pkgs,
            unit: None,
            classes: &no_classes,
            pkg_members: &pkg_members,
        };
        let mut ck = Ck::new(env, elab, "");
        ck.push();
        for d in defs {
            if let SourceDefinition::Typedef(td) = d {
                ck.declare_typedef(td);
            }
        }
        unit = ck.pop();
    }
    let mut packages: HashMap<String, Scope> = HashMap::default();
    for _pass in 0..2 {
        let mut next: HashMap<String, Scope> = HashMap::default();
        for d in defs {
            if let SourceDefinition::Package(p) = d {
                let env = Env {
                    class_names: &class_names,
                    packages: &packages,
                    unit: Some(&unit),
                    classes: &no_classes,
                    pkg_members: &pkg_members,
                };
                let mut ck = Ck::new(env, elab, &p.name.name);
                ck.push();
                declare_package_items(&mut ck, &p.items);
                next.insert(p.name.name.clone(), ck.pop());
            }
        }
        packages = next;
    }

    // Phase 2: class declarations, resolved in the scope that declares them.
    let mut classes: HashMap<String, Option<Rc<ClassInfo>>> = HashMap::default();
    {
        let env = || Env {
            class_names: &class_names,
            packages: &packages,
            unit: Some(&unit),
            classes: &no_classes,
            pkg_members: &pkg_members,
        };
        let mut found: Vec<(String, ClassInfo)> = Vec::new();
        for d in defs {
            match d {
                SourceDefinition::Module(m) => {
                    let mut ck = Ck::new(env(), elab, &m.name.name);
                    ck.push();
                    for p in &m.params {
                        ck.declare_param(p);
                    }
                    ck.declare_items(&m.items);
                    class_infos_items(&mut ck, &m.items, &mut found);
                }
                SourceDefinition::Interface(m) => {
                    let mut ck = Ck::new(env(), elab, &m.name.name);
                    ck.push();
                    for p in &m.params {
                        ck.declare_param(p);
                    }
                    ck.declare_items(&m.items);
                    class_infos_items(&mut ck, &m.items, &mut found);
                }
                SourceDefinition::Program(m) => {
                    let mut ck = Ck::new(env(), elab, &m.name.name);
                    ck.push();
                    for p in &m.params {
                        ck.declare_param(p);
                    }
                    ck.declare_items(&m.items);
                    class_infos_items(&mut ck, &m.items, &mut found);
                }
                SourceDefinition::Package(p) => {
                    let mut ck = Ck::new(env(), elab, &p.name.name);
                    ck.push();
                    declare_package_items(&mut ck, &p.items);
                    for it in &p.items {
                        if let PackageItem::Class(c) = it {
                            class_info_rec(&mut ck, c, &mut found);
                        }
                    }
                }
                SourceDefinition::Class(c) => {
                    let mut ck = Ck::new(env(), elab, &c.name.name);
                    class_info_rec(&mut ck, c, &mut found);
                }
                _ => {}
            }
        }
        for (n, ci) in found {
            if class_names.get(&n) == Some(&1) {
                classes.insert(n, Some(Rc::new(ci)));
            }
        }
    }

    // Modules something instantiates (their parameters may be overridden).
    let mut instantiated: HashSet<String> = HashSet::default();
    fn instances(items: &[ModuleItem], out: &mut HashSet<String>) {
        for it in items {
            match it {
                ModuleItem::ModuleInstantiation(mi) => {
                    out.insert(mi.module_name.name.clone());
                }
                ModuleItem::GenerateRegion(g) => instances(&g.items, out),
                ModuleItem::GenerateIf(g) => g.branches.iter().for_each(|(_, i)| instances(i, out)),
                ModuleItem::GenerateFor(g) => instances(&g.items, out),
                ModuleItem::GenerateCase(g) => g.arms.iter().for_each(|a| instances(&a.items, out)),
                _ => {}
            }
        }
    }
    for d in defs {
        match d {
            SourceDefinition::Module(m) => instances(&m.items, &mut instantiated),
            SourceDefinition::Interface(m) => instances(&m.items, &mut instantiated),
            SourceDefinition::Program(m) => instances(&m.items, &mut instantiated),
            _ => {}
        }
    }

    // Phase 3: walk every body.
    let mut errs = Vec::new();
    let env = || Env {
        class_names: &class_names,
        packages: &packages,
        unit: Some(&unit),
        classes: &classes,
        pkg_members: &pkg_members,
    };
    for d in defs {
        let mut ck;
        match d {
            SourceDefinition::Module(m) => {
                ck = Ck::new(env(), elab, &m.name.name);
                ck.default_auto = m.lifetime == Some(Lifetime::Automatic);
                ck.params_fixed = !instantiated.contains(&m.name.name);
                ck.push();
                for p in &m.params {
                    ck.declare_param(p);
                }
                ck.declare_ansi_ports(&m.ports);
                ck.declare_items(&m.items);
                ck.check_ansi_port_dims(&m.ports);
                for p in &m.params {
                    ck.check_param_values(p);
                }
                ck.walk_items(&m.items);
            }
            SourceDefinition::Interface(m) => {
                ck = Ck::new(env(), elab, &m.name.name);
                ck.default_auto = m.lifetime == Some(Lifetime::Automatic);
                ck.params_fixed = !instantiated.contains(&m.name.name);
                ck.push();
                for p in &m.params {
                    ck.declare_param(p);
                }
                ck.declare_ansi_ports(&m.ports);
                ck.declare_items(&m.items);
                ck.check_ansi_port_dims(&m.ports);
                for p in &m.params {
                    ck.check_param_values(p);
                }
                ck.walk_items(&m.items);
            }
            SourceDefinition::Program(m) => {
                ck = Ck::new(env(), elab, &m.name.name);
                ck.default_auto = m.lifetime == Some(Lifetime::Automatic);
                ck.params_fixed = !instantiated.contains(&m.name.name);
                ck.push();
                for p in &m.params {
                    ck.declare_param(p);
                }
                ck.declare_ansi_ports(&m.ports);
                ck.declare_items(&m.items);
                ck.check_ansi_port_dims(&m.ports);
                for p in &m.params {
                    ck.check_param_values(p);
                }
                ck.walk_items(&m.items);
            }
            SourceDefinition::Package(p) => {
                ck = Ck::new(env(), elab, &p.name.name);
                check_package_exports(&mut ck, &p.items);
                ck.default_auto = p.lifetime == Some(Lifetime::Automatic);
                ck.push();
                declare_package_items(&mut ck, &p.items);
                for it in &p.items {
                    match it {
                        PackageItem::Function(f) if f.name.scopes.is_empty() => ck.walk_function(f),
                        PackageItem::Task(t) if t.name.scopes.is_empty() => ck.walk_task(t),
                        PackageItem::Class(c) => ck.walk_class(c),
                        _ => {}
                    }
                }
            }
            SourceDefinition::Class(c) => {
                ck = Ck::new(env(), elab, &c.name.name);
                ck.walk_class(c);
            }
            SourceDefinition::Typedef(td) => {
                ck = Ck::new(env(), elab, &td.name.name);
                ck.push();
                ck.declare_typedef(td);
            }
            _ => continue,
        }
        errs.append(&mut ck.errs);
    }
    errs
}

fn declare_package_items(ck: &mut Ck<'_>, items: &[PackageItem]) {
    for it in items {
        match it {
            PackageItem::Typedef(td) => ck.declare_typedef(td),
            PackageItem::Parameter(pd) => ck.declare_param(pd),
            PackageItem::Import(imp) => ck.declare_import(imp),
            PackageItem::Data(d) => {
                let t = ck.resolve(&d.data_type);
                for dc in &d.declarators {
                    let vt = with_dims(t.clone(), &dc.dimensions);
                    ck.declare_value(&dc.name.name, vt);
                }
            }
            PackageItem::Function(f) if f.name.scopes.is_empty() => {
                let s = ck.sig_of_function(f);
                ck.top().subs.insert(f.name.name.name.clone(), Some(s));
            }
            PackageItem::Task(t) if t.name.scopes.is_empty() => {
                let s = ck.sig_of_task(t);
                ck.top().subs.insert(t.name.name.name.clone(), Some(s));
            }
            PackageItem::DPIImport(d) => {
                if let Some(n) = dpi_name(d) {
                    ck.top().subs.insert(n, None);
                }
            }
            _ => {}
        }
    }
}

fn class_info_rec(ck: &mut Ck<'_>, c: &ClassDeclaration, out: &mut Vec<(String, ClassInfo)>) {
    let ci = ck.class_info(c);
    out.push((c.name.name.clone(), ci));
    for it in &c.items {
        if let ClassItem::Class(inner) = it {
            class_info_rec(ck, inner, out);
        }
    }
}

fn class_infos_items(ck: &mut Ck<'_>, items: &[ModuleItem], out: &mut Vec<(String, ClassInfo)>) {
    for it in items {
        match it {
            ModuleItem::ClassDeclaration(c) => class_info_rec(ck, c, out),
            ModuleItem::GenerateRegion(g) => class_infos_items(ck, &g.items, out),
            ModuleItem::GenerateIf(g) => {
                for (_, items) in &g.branches {
                    class_infos_items(ck, items, out);
                }
            }
            ModuleItem::GenerateFor(g) => class_infos_items(ck, &g.items, out),
            ModuleItem::GenerateCase(g) => {
                for a in &g.arms {
                    class_infos_items(ck, &a.items, out);
                }
            }
            _ => {}
        }
    }
}

/// Every name package items declare (None when the package re-exports or
/// holds an item this pass does not model).
fn package_member_names(items: &[PackageItem]) -> Option<HashSet<String>> {
    let mut out = HashSet::default();
    let mut ranged = false;
    let mut enum_members = |dt: &DataType, out: &mut HashSet<String>| {
        if let DataType::Enum(et) = dt {
            // `A[3]` declares A0..A2; not modeled here.
            ranged |= et.members.iter().any(|m| m.range.is_some());
            out.extend(et.members.iter().map(|m| m.name.name.clone()));
        }
    };
    for it in items {
        match it {
            PackageItem::Parameter(pd) => match &pd.kind {
                ParameterKind::Data {
                    data_type,
                    assignments,
                } => {
                    enum_members(data_type, &mut out);
                    out.extend(assignments.iter().map(|a| a.name.name.clone()));
                }
                ParameterKind::Type { assignments } => {
                    out.extend(assignments.iter().map(|a| a.name.name.clone()))
                }
            },
            PackageItem::Typedef(td) => {
                out.insert(td.name.name.clone());
                enum_members(&td.data_type, &mut out);
            }
            PackageItem::Function(f) => {
                out.insert(f.name.name.name.clone());
            }
            PackageItem::Task(t) => {
                out.insert(t.name.name.name.clone());
            }
            PackageItem::Data(d) => {
                enum_members(&d.data_type, &mut out);
                out.extend(d.declarators.iter().map(|dc| dc.name.name.clone()));
            }
            PackageItem::Class(c) => {
                out.insert(c.name.name.clone());
            }
            PackageItem::Covergroup(cg) => {
                out.insert(cg.name.name.clone());
            }
            PackageItem::DPIImport(d) => {
                out.extend(dpi_name(d));
            }
            PackageItem::Let(l) => {
                out.insert(l.name.name.clone());
            }
            PackageItem::Checker(c) => {
                out.insert(c.name.name.clone());
            }
            PackageItem::Nettype(n) => {
                out.insert(n.name.name.clone());
            }
            PackageItem::Property(pr) => {
                out.insert(pr.name.name.clone());
            }
            PackageItem::Sequence(sq) => {
                out.insert(sq.name.name.clone());
            }
            PackageItem::Import(_) | PackageItem::DPIExport(_) | PackageItem::TimeunitsDecl(_) => {}
            PackageItem::Null | PackageItem::Export(_) => return None,
        }
    }
    (!ranged).then_some(out)
}

/// Call `f` on `e` and every sub-expression of it.
fn visit_expr<'e>(e: &'e Expression, f: &mut dyn FnMut(&'e Expression)) {
    f(e);
    match &e.kind {
        ExprKind::Unary { operand, .. } => visit_expr(operand, f),
        ExprKind::Binary { left, right, .. } => {
            visit_expr(left, f);
            visit_expr(right, f);
        }
        ExprKind::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            visit_expr(condition, f);
            visit_expr(then_expr, f);
            visit_expr(else_expr, f);
        }
        ExprKind::Concatenation(xs) => xs.iter().for_each(|x| visit_expr(x, f)),
        ExprKind::Replication { count, exprs } => {
            visit_expr(count, f);
            exprs.iter().for_each(|x| visit_expr(x, f));
        }
        ExprKind::AssignmentPattern(items) => items.iter().for_each(|i| visit_expr(i.expr(), f)),
        ExprKind::Call { func, args } => {
            visit_expr(func, f);
            args.iter().for_each(|x| visit_expr(x, f));
        }
        ExprKind::SystemCall { args, .. } => args.iter().for_each(|x| visit_expr(x, f)),
        ExprKind::NamedArg { expr: Some(x), .. } => visit_expr(x, f),
        ExprKind::Inside { expr, ranges } => {
            visit_expr(expr, f);
            ranges.iter().for_each(|x| visit_expr(x, f));
        }
        ExprKind::MemberAccess { expr, .. } => visit_expr(expr, f),
        ExprKind::Index { expr, index } => {
            visit_expr(expr, f);
            visit_expr(index, f);
        }
        ExprKind::RangeSelect {
            expr, left, right, ..
        } => {
            visit_expr(expr, f);
            visit_expr(left, f);
            visit_expr(right, f);
        }
        ExprKind::Range(a, b) => {
            visit_expr(a, f);
            visit_expr(b, f);
        }
        ExprKind::Paren(x) => visit_expr(x, f),
        ExprKind::AssignExpr { lvalue, rvalue } => {
            visit_expr(lvalue, f);
            visit_expr(rvalue, f);
        }
        ExprKind::Ident(h) => {
            for seg in &h.path {
                seg.selects.iter().for_each(|x| visit_expr(x, f));
            }
        }
        _ => {}
    }
}

/// A bit-, part- or element-select anywhere in an lvalue.
fn has_select(e: &Expression) -> bool {
    match &e.kind {
        ExprKind::Index { .. } | ExprKind::RangeSelect { .. } => true,
        ExprKind::Ident(h) => h.path.iter().any(|s| !s.selects.is_empty()),
        ExprKind::Concatenation(xs) => xs.iter().any(has_select),
        ExprKind::Paren(x) => has_select(x),
        _ => false,
    }
}

/// §26.6: `export P::n` needs `n` imported from `P` (by name, or by a
/// wildcard import of `P`) and not declared in this package.
fn check_package_exports(ck: &mut Ck<'_>, items: &[PackageItem]) {
    let local: HashSet<String> = local_package_names(items);
    let mut imported: Vec<(&str, Option<&str>)> = Vec::new();
    for it in items {
        if let PackageItem::Import(imp) = it {
            for i in &imp.items {
                imported.push((
                    i.package.name.as_str(),
                    i.item.as_ref().map(|n| n.name.as_str()),
                ));
            }
        }
    }
    for it in items {
        let PackageItem::Export(exp) = it else {
            continue;
        };
        for e in &exp.items {
            let Some(n) = &e.item else {
                continue;
            };
            let pkg = e.package.name.as_str();
            if pkg == "*" || n.name == "*" {
                continue;
            }
            let why = if local.contains(&n.name) {
                "is declared in this package"
            } else if !imported
                .iter()
                .any(|(p, i)| *p == pkg && i.is_none_or(|i| i == n.name))
            {
                "was not imported"
            } else {
                continue;
            };
            ck.report(
                n.span,
                format!(
                    "cannot export {pkg}::{}: the name {why} (IEEE 1800-2017 §26.6)",
                    n.name
                ),
            );
        }
    }
}

/// Names a package declares itself (not the ones it imports).
fn local_package_names(items: &[PackageItem]) -> HashSet<String> {
    let mut out = HashSet::default();
    for it in items {
        match it {
            PackageItem::Parameter(pd) => match &pd.kind {
                ParameterKind::Data { assignments, .. } => {
                    out.extend(assignments.iter().map(|a| a.name.name.clone()))
                }
                ParameterKind::Type { assignments } => {
                    out.extend(assignments.iter().map(|a| a.name.name.clone()))
                }
            },
            PackageItem::Typedef(td) => {
                out.insert(td.name.name.clone());
            }
            PackageItem::Function(f) => {
                out.insert(f.name.name.name.clone());
            }
            PackageItem::Task(t) => {
                out.insert(t.name.name.name.clone());
            }
            PackageItem::Data(d) => out.extend(d.declarators.iter().map(|dc| dc.name.name.clone())),
            PackageItem::Class(c) => {
                out.insert(c.name.name.clone());
            }
            _ => {}
        }
    }
    out
}
