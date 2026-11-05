//! IEEE 1800-2017 §20.6.1 `$typename`: the text naming the type of a data
//! type or of an expression.
//!
//! Declared types render as the reference simulator spells them: a 4-state
//! vector is `reg` (`logic [7:0]` reads `reg[7:0]`), but `logic` once it has
//! more than one packed or unpacked dimension, or is a member of a packed
//! aggregate; packed and unpacked ranges follow the keyword with no space and
//! are listed innermost first; unpacked ones come after a `$` (`int$[0:1]`,
//! `int$[]` for a queue or dynamic array, `int$[string]` for an associative
//! one). An integer atom is named by its keyword alone (`int unsigned` reads
//! `int`).
//!
//! An integral EXPRESSION (§11.6, §11.8.1) names its self-determined type:
//! the type of the first operand whose width, signedness and 2-/4-state
//! the result keeps (`i + 1` is `int`, `1 + i` the literal's `bit
//! signed[31:0]`), else `bit`/`reg` with the result's sign and range.

use super::*;
use crate::ast::types::{
    IntegerAtomType as A, IntegerVectorType as V, PackedDimension, RealType, Signing, SimpleType,
    StructUnionKind, TypeName, UnpackedDimension,
};

/// The self-determined type of an integral expression: width, signedness,
/// 2-/4-state, and the declared type's text when the value keeps it.
#[derive(Debug, Clone)]
pub(super) struct TnType {
    width: u32,
    signed: bool,
    four: bool,
    name: Option<String>,
}

impl TnType {
    fn generic(width: u32, signed: bool, four: bool) -> Self {
        TnType {
            width,
            signed,
            four,
            name: None,
        }
    }

    fn named(width: u32, signed: bool, four: bool, name: &str) -> Self {
        TnType {
            width,
            signed,
            four,
            name: Some(name.to_string()),
        }
    }

    /// §11.6.1 / §11.8.1: a context-determined operator's result — the wider
    /// width, signed only when every operand is, 4-state when any is. It keeps
    /// the type of the FIRST operand that has exactly those facts (an unnamed
    /// literal included: `1 + i` stays `bit signed[31:0]`).
    fn merge(ops: &[&TnType], extra_four: bool) -> Self {
        let width = ops.iter().map(|t| t.width).max().unwrap_or(1);
        let signed = ops.iter().all(|t| t.signed);
        let four = extra_four || ops.iter().any(|t| t.four);
        let name = ops
            .iter()
            .find(|t| t.width == width && t.signed == signed && t.four == four)
            .and_then(|t| t.name.clone());
        TnType {
            width,
            signed,
            four,
            name,
        }
    }

    /// §11.4.2: `/`, `%` and `**` can yield x (a zero divisor, `0 ** -1`),
    /// so the result is 4-state; an `int` becomes `integer`, any other
    /// 2-state type loses its name.
    fn forced_four(self) -> Self {
        if self.four {
            return self;
        }
        let name = (self.name.as_deref() == Some("int")).then(|| "integer".to_string());
        TnType {
            four: true,
            name,
            ..self
        }
    }

    fn unsized_string() -> Self {
        TnType::named(0, false, false, "string")
    }

    fn is_string(&self) -> bool {
        self.width == 0 && self.name.as_deref() == Some("string")
    }

    pub(super) fn render(&self) -> String {
        if let Some(n) = &self.name {
            return n.clone();
        }
        let mut s = String::from(if self.four { "reg" } else { "bit" });
        if self.signed {
            s.push_str(" signed");
        }
        if self.width > 1 {
            s.push_str(&format!("[{}:0]", self.width - 1));
        }
        s
    }
}

/// One unpacked dimension of a declared type, as `$typename` writes it.
#[derive(Debug, Clone)]
pub(super) enum TnDim {
    Fixed(i64, i64),
    /// A queue or dynamic array: `[]`.
    Unsized,
    /// An associative array, by its index type: `[string]`.
    Assoc(String),
}

/// The declared type a `$typename` operand names: the type, the unpacked
/// dimensions still unselected (outermost first), whether it is a member of
/// a packed aggregate (keeps `logic`), and the selects past the unpacked
/// dimensions (bit or part selects of the vector).
struct TnOperand {
    dt: DataType,
    dims: Vec<TnDim>,
    packed_member: bool,
    vec_selects: Vec<Expression>,
}

impl Simulator {
    /// §20.6.1 `$typename` text of a data type with no unpacked dimensions
    /// of its own: a typedef resolves to its definition, a struct/enum
    /// typedef keeps its scoped name.
    pub(super) fn typename_of_dt(&self, dt: &DataType) -> String {
        self.tn_base_text(dt, false)
            .unwrap_or_else(|| "logic".to_string())
    }

    /// `$typename(T)` of a TYPEDEF name: its definition with the typedef's
    /// own unpacked dimensions (`typedef byte q_t[$]` reads `byte$[]`).
    pub(super) fn typename_of_typedef_name(&self, tdn: &str) -> Option<String> {
        let target = self.lookup_typedef_target(tdn)?;
        let base = match &target {
            DataType::Struct(_) | DataType::Enum(_) => {
                self.tn_named_aggregate(&target, &Self::tn_type_name(tdn))
            }
            _ => self.tn_base_text(&target, false),
        }?;
        if tdn.contains("::") {
            let dims = self
                .module
                .typedef_unpacked_dims
                .get(tdn)
                .map(|d| self.tn_dims_of(d))
                .unwrap_or_default();
            return Some(self.tn_with_dims(base, &target, &dims));
        }
        let ctx = self.class_context_stack.last().cloned().flatten();
        let dims = Self::typedef_dims_via_tables(&self.module, ctx.as_deref().unwrap_or(""), tdn)
            .map(|d| self.tn_dims_of(&d))
            .unwrap_or_default();
        Some(self.tn_with_dims(base, &target, &dims))
    }

    /// A `TypeName` for the `::`-joined spelling `n` (`pk::t`).
    fn tn_type_name(n: &str) -> TypeName {
        let span = crate::ast::Span::dummy();
        let ident = |s: &str| crate::ast::Identifier {
            name: s.to_string(),
            span,
        };
        let mut parts: Vec<&str> = n.split("::").collect();
        let leaf = parts.pop().unwrap_or(n);
        TypeName {
            scopes: parts
                .into_iter()
                .map(|p| crate::ast::types::TypeScope {
                    name: ident(p),
                    type_args: Vec::new(),
                })
                .collect(),
            name: ident(leaf),
            span,
        }
    }

    /// §20.6.1: `$typename(pk::t)` — a package-scoped TYPE operand, which
    /// parses as a two-segment name (or a member access).
    pub(super) fn typename_scoped_type(&self, arg: &Expression) -> Option<String> {
        let (pkg, leaf) = match &arg.kind {
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.len() == 2
                    && h.path.iter().all(|s| s.selects.is_empty()) =>
            {
                (h.path[0].name.name.as_str(), h.path[1].name.name.as_str())
            }
            ExprKind::MemberAccess { expr, member } => match &expr.kind {
                ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                    (h.path[0].name.name.as_str(), member.name.as_str())
                }
                _ => return None,
            },
            _ => return None,
        };
        if !self.module.packages.contains(pkg) {
            return None;
        }
        let key = format!("{}::{}", pkg, leaf);
        if !self.module.typedef_types.contains_key(&key) {
            return None;
        }
        self.typename_of_typedef_name(&key)
    }

    /// Append the unpacked dimensions, innermost first, after `$` (a
    /// `string` element takes no `$`). More than one unpacked dimension
    /// turns a `reg` element into `logic`.
    fn tn_with_dims(&self, base: String, elem: &DataType, dims: &[TnDim]) -> String {
        if dims.is_empty() {
            return base;
        }
        let mut s = if dims.len() > 1 {
            self.tn_base_text(elem, true).unwrap_or(base)
        } else {
            base
        };
        let is_string = matches!(
            self.resolve_dt_ref(elem),
            DataType::Simple {
                kind: SimpleType::String,
                ..
            }
        );
        if !is_string {
            s.push('$');
        }
        for d in dims.iter().rev() {
            match d {
                TnDim::Fixed(l, r) => s.push_str(&format!("[{}:{}]", l, r)),
                TnDim::Unsized => s.push_str("[]"),
                TnDim::Assoc(k) => s.push_str(&format!("[{}]", k)),
            }
        }
        s
    }

    fn tn_const(&self, e: &Expression) -> Option<i64> {
        crate::elaborate::const_eval_i64_with_params(e, Some(&self.module.parameters))
    }

    /// The `$typename` form of declared unpacked dimensions.
    fn tn_dims_of(&self, dims: &[UnpackedDimension]) -> Vec<TnDim> {
        dims.iter()
            .map(|d| match d {
                UnpackedDimension::Range { left, right, .. } => {
                    match (self.tn_const(left), self.tn_const(right)) {
                        (Some(l), Some(r)) => TnDim::Fixed(l, r),
                        _ => TnDim::Unsized,
                    }
                }
                UnpackedDimension::Expression { expr, .. } => match self.tn_const(expr) {
                    Some(n) => TnDim::Fixed(0, n - 1),
                    None => TnDim::Unsized,
                },
                UnpackedDimension::Unsized(_) | UnpackedDimension::Queue { .. } => TnDim::Unsized,
                UnpackedDimension::Associative { data_type, .. } => TnDim::Assoc(
                    data_type
                        .as_deref()
                        .and_then(|k| self.tn_base_text(k, false))
                        .unwrap_or_else(|| "*".to_string()),
                ),
            })
            .collect()
    }

    /// The text of a type with no unpacked dimensions; `logic_kw` keeps a
    /// 4-state vector `logic` (see the module notes). `None` for a class or
    /// an anonymous aggregate, which other paths name.
    fn tn_base_text(&self, dt: &DataType, logic_kw: bool) -> Option<String> {
        match dt {
            DataType::TypeReference { name, .. } => {
                let n = name.name.name.as_str();
                if self.module.interfaces.contains(n) {
                    return Some(format!("virtual {}", n));
                }
                if name.scopes.is_empty() && self.module.classes.contains_key(n) {
                    return None;
                }
                let target = if name.scopes.is_empty() {
                    self.lookup_typedef_target(n)
                } else {
                    self.module.typedef_types.get(&name.qualified()).cloned()
                }?;
                match &target {
                    DataType::Struct(_) | DataType::Enum(_) => {
                        self.tn_named_aggregate(&target, name)
                    }
                    _ => self.tn_base_text(&target, logic_kw),
                }
            }
            DataType::IntegerVector {
                kind,
                signing,
                dimensions,
                ..
            } => {
                let kw = match kind {
                    V::Bit => "bit",
                    _ if logic_kw || dimensions.len() > 1 => "logic",
                    _ => "reg",
                };
                let mut s = kw.to_string();
                if matches!(signing, Some(Signing::Signed)) {
                    s.push_str(" signed");
                }
                for d in dimensions.iter().rev() {
                    if let PackedDimension::Range { left, right, .. } = d {
                        match (self.tn_const(left), self.tn_const(right)) {
                            (Some(l), Some(r)) => s.push_str(&format!("[{}:{}]", l, r)),
                            _ => return Some(kw.to_string()),
                        }
                    }
                }
                Some(s)
            }
            DataType::IntegerAtom { kind, .. } => Some(
                match kind {
                    A::Byte => "byte",
                    A::ShortInt => "shortint",
                    A::Int => "int",
                    A::LongInt => "longint",
                    A::Integer => "integer",
                    A::Time => "time",
                }
                .to_string(),
            ),
            DataType::Real { kind, .. } => Some(
                match kind {
                    RealType::ShortReal => "shortreal",
                    // §6.12: `realtime` is `real`.
                    RealType::Real | RealType::RealTime => "real",
                }
                .to_string(),
            ),
            DataType::Simple { kind, .. } => Some(
                match kind {
                    SimpleType::String => "string",
                    SimpleType::Chandle => "chandle",
                    SimpleType::Event => "event",
                }
                .to_string(),
            ),
            DataType::Implicit { .. } => Some("logic".to_string()),
            // §25.9: a virtual interface renders as it is declared, the
            // modport before the parameter values (`virtual pbus_if.m
            // #(16)`). Positional values are evaluated; a named one keeps
            // its `.NAME(...)`.
            DataType::Interface {
                name,
                modport,
                type_args,
                ..
            } => {
                let mut s = format!("virtual {}", name.name);
                if let Some(mp) = modport {
                    s.push('.');
                    s.push_str(&mp.name);
                }
                if !type_args.is_empty() {
                    let render = |e: &Expression| {
                        self.tn_const(e)
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "?".to_string())
                    };
                    let parts: Vec<String> = type_args
                        .iter()
                        .map(|a| match &a.kind {
                            ExprKind::NamedArg { name, expr } => format!(
                                ".{}({})",
                                name.name,
                                expr.as_deref().map(render).unwrap_or_default()
                            ),
                            _ => render(a),
                        })
                        .collect();
                    s.push_str(&format!(" #({})", parts.join(", ")));
                }
                Some(s)
            }
            DataType::Struct(_) | DataType::Enum(_) | DataType::Void(_) => None,
        }
    }

    /// §20.6.1: a struct, union or enum typedef is named with its scope —
    /// `struct pk::s_t` for a package typedef, `struct top.s_t` for one
    /// declared in a module — after `packed` / the enum's base type.
    fn tn_named_aggregate(&self, target: &DataType, name: &TypeName) -> Option<String> {
        let n = name.name.name.as_str();
        let scoped = if !name.scopes.is_empty() {
            name.qualified()
        } else {
            let suffix = format!("::{}", n);
            let mut pk: Vec<&String> = self
                .module
                .typedef_types
                .keys()
                .filter(|k| {
                    k.ends_with(&suffix)
                        && self.module.packages.contains(&k[..k.len() - suffix.len()])
                })
                .collect();
            pk.sort();
            let mut owner: Vec<&String> = self
                .module
                .classes
                .iter()
                .filter(|(_, cd)| cd.typedef_targets.contains_key(n))
                .map(|(c, _)| c)
                .collect();
            owner.sort();
            match (pk.first(), owner.first()) {
                (Some(k), _) => k.to_string(),
                // A class-scope typedef: `C::t`.
                (None, Some(c)) => format!("{}::{}", c, n),
                (None, None) => format!("{}.{}", self.module.name, n),
            }
        };
        match target {
            DataType::Struct(su) => {
                let kw = match su.kind {
                    StructUnionKind::Struct => "struct",
                    _ => "union",
                };
                let packed = if su.packed { " packed" } else { "" };
                Some(format!("{}{} {}", kw, packed, scoped))
            }
            DataType::Enum(et) => {
                let base = match et.base_type.as_deref() {
                    Some(b) => self.tn_base_text(b, false)?,
                    None => "int".to_string(),
                };
                Some(format!("enum {} {}", base, scoped))
            }
            _ => None,
        }
    }

    /// The integral type facts of a declared type; `logic_kw` as for
    /// [`tn_base_text`]. An enum reads as its base type.
    fn tn_type_of_dt(&self, dt: &DataType, logic_kw: bool) -> Option<TnType> {
        let cur = self.resolve_dt_ref(dt);
        match cur {
            DataType::IntegerAtom { kind, signing, .. } => {
                let (w, four) = match kind {
                    A::Byte => (8, false),
                    A::ShortInt => (16, false),
                    A::Int => (32, false),
                    A::LongInt => (64, false),
                    A::Integer => (32, true),
                    A::Time => (64, true),
                };
                let signed =
                    !matches!(signing, Some(Signing::Unsigned)) && !matches!(kind, A::Time);
                Some(TnType {
                    width: w,
                    signed,
                    four,
                    name: self.tn_base_text(cur, logic_kw),
                })
            }
            DataType::IntegerVector {
                kind,
                signing,
                dimensions,
                ..
            } => {
                let mut w: u64 = 1;
                for d in dimensions {
                    if let PackedDimension::Range { left, right, .. } = d {
                        let l = self.tn_const(left)?;
                        let r = self.tn_const(right)?;
                        w *= (l - r).unsigned_abs() + 1;
                    }
                }
                Some(TnType {
                    width: w as u32,
                    signed: matches!(signing, Some(Signing::Signed)),
                    four: !matches!(kind, V::Bit),
                    name: self.tn_base_text(cur, logic_kw),
                })
            }
            DataType::Enum(et) => match et.base_type.as_deref() {
                Some(b) => self.tn_type_of_dt(b, false),
                None => Some(TnType::named(32, true, false, "int")),
            },
            DataType::Simple {
                kind: SimpleType::String,
                ..
            } => Some(TnType::unsized_string()),
            _ => None,
        }
    }

    /// The unpacked dimensions of the design or block variable `h` names
    /// (declared `dt`), outermost first: a typedef's own dims, else the
    /// registered fixed, queue, dynamic or associative shape.
    fn tn_var_dims(&self, h: &HierarchicalIdentifier, dt: &DataType) -> Vec<TnDim> {
        if let DataType::TypeReference { name, .. } = dt {
            let ctx = self.class_context_stack.last().cloned().flatten();
            if let Some(d) = Self::typedef_dims_via_tables(
                &self.module,
                ctx.as_deref().unwrap_or(""),
                &name.name.name,
            )
            .filter(|d| !d.is_empty())
            {
                return self.tn_dims_of(&d);
            }
        }
        let resolved = self.resolve_hier_name(h);
        let base = resolved.split('[').next().unwrap_or(&resolved);
        if let Some(d) = self.module.unpacked_decl_dims.get(base) {
            return d.iter().map(|&(l, r)| TnDim::Fixed(l, r)).collect();
        }
        if self.module.queue_vars.contains(base) || self.module.dynamic_arrays.contains(base) {
            return vec![TnDim::Unsized];
        }
        if let Some(&string_keyed) = self.module.associative_arrays.get(base) {
            let key = if string_keyed {
                "string".to_string()
            } else {
                self.module
                    .assoc_key_type_names
                    .get(base)
                    .cloned()
                    .unwrap_or_else(|| "int".to_string())
            };
            return vec![TnDim::Assoc(key)];
        }
        Vec::new()
    }

    /// Resolve a `$typename` operand naming a variable, a class property, a
    /// struct member or an element of one of those to its declared type.
    fn tn_operand(&mut self, e: &Expression) -> Option<TnOperand> {
        match &e.kind {
            ExprKind::Paren(inner) => self.tn_operand(inner),
            ExprKind::Ident(h) if h.root.is_none() => {
                let (last, owners) = h.path.split_last()?;
                if owners.iter().any(|s| !s.selects.is_empty()) {
                    return None;
                }
                let mut op = if owners.is_empty() {
                    let name = last.name.name.as_str();
                    let (dt, fixed, is_prop) = self.query_root_decl_type(name)?;
                    let dims = if is_prop {
                        fixed.iter().map(|&(l, r)| TnDim::Fixed(l, r)).collect()
                    } else {
                        let mut bare = h.clone();
                        bare.path = vec![crate::ast::expr::HierPathSegment {
                            name: last.name.clone(),
                            selects: Vec::new(),
                        }];
                        bare.cached_resolved_name = crate::ast::cache::NameCache::new();
                        bare.cached_signal_id = crate::ast::cache::SignalCache::new(None);
                        self.tn_var_dims(&bare, &dt)
                    };
                    TnOperand {
                        dt,
                        dims,
                        packed_member: false,
                        vec_selects: Vec::new(),
                    }
                } else {
                    let (first, rest) = owners.split_first()?;
                    let (dt, fixed, _) = self.query_root_decl_type(&first.name.name)?;
                    if !fixed.is_empty() {
                        return None;
                    }
                    let mut op = TnOperand {
                        dt,
                        dims: Vec::new(),
                        packed_member: false,
                        vec_selects: Vec::new(),
                    };
                    for seg in rest {
                        op = self.tn_member(op, &seg.name.name)?;
                    }
                    self.tn_member(op, &last.name.name)?
                };
                for s in &last.selects {
                    Self::tn_select(&mut op, s);
                }
                Some(op)
            }
            ExprKind::MemberAccess { expr, member } => {
                let op = self.tn_operand(expr)?;
                self.tn_member(op, &member.name)
            }
            ExprKind::Index { expr, index } => {
                let mut op = self.tn_operand(expr)?;
                Self::tn_select(&mut op, index);
                Some(op)
            }
            _ => None,
        }
    }

    /// Select into `op`: an unpacked dimension first, then the vector.
    fn tn_select(op: &mut TnOperand, sel: &Expression) {
        if op.vec_selects.is_empty() && !op.dims.is_empty() {
            op.dims.remove(0);
        } else {
            op.vec_selects.push(sel.clone());
        }
    }

    /// Member `m` of the (fully selected) struct or class value `op`.
    fn tn_member(&self, op: TnOperand, m: &str) -> Option<TnOperand> {
        if !op.dims.is_empty() || !op.vec_selects.is_empty() {
            return None;
        }
        let packed = matches!(self.resolve_dt_ref(&op.dt), DataType::Struct(su) if su.packed);
        let (dt, fixed) = self.member_decl_type(&op.dt, m)?;
        Some(TnOperand {
            dt,
            dims: fixed.iter().map(|&(l, r)| TnDim::Fixed(l, r)).collect(),
            packed_member: packed || op.packed_member,
            vec_selects: Vec::new(),
        })
    }

    /// §20.6.1: the `$typename` text of a variable, property or member
    /// operand (and its elements). `None` for a class handle or a shape it
    /// cannot resolve, which the caller's other paths name.
    pub(super) fn typename_operand_text(&mut self, e: &Expression) -> Option<String> {
        let op = self.tn_operand(e)?;
        if op.vec_selects.is_empty() {
            let base = match &op.dt {
                DataType::TypeReference { name, .. } => {
                    let target = if name.scopes.is_empty() {
                        self.lookup_typedef_target(&name.name.name)
                    } else {
                        self.module.typedef_types.get(&name.qualified()).cloned()
                    };
                    match target {
                        Some(t @ (DataType::Struct(_) | DataType::Enum(_))) => {
                            self.tn_named_aggregate(&t, name)?
                        }
                        _ => self.tn_base_text(&op.dt, op.packed_member)?,
                    }
                }
                _ => self.tn_base_text(&op.dt, op.packed_member)?,
            };
            return Some(self.tn_with_dims(base, &op.dt, &op.dims));
        }
        self.typename_integral_expr(e).map(|t| t.render())
    }

    /// The integral type of a variable operand after its selects.
    fn tn_operand_type(&mut self, e: &Expression) -> Option<TnType> {
        let op = self.tn_operand(e)?;
        if !op.dims.is_empty() {
            return None;
        }
        let base = self.tn_type_of_dt(&op.dt, op.packed_member)?;
        match op.vec_selects.as_slice() {
            [] => Some(base),
            [sel] => {
                let w = match &sel.kind {
                    ExprKind::Range(l, r) => {
                        let (l, r) = (self.tn_const(l)?, self.tn_const(r)?);
                        (l - r).unsigned_abs() as u32 + 1
                    }
                    _ => 1,
                };
                Some(TnType::generic(w, false, base.four))
            }
            _ => None,
        }
    }

    /// Is `e` a string operand (a literal or a `string` variable)?
    fn tn_is_string(&mut self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::StringLiteral(_) => true,
            ExprKind::Paren(i) => self.tn_is_string(i),
            ExprKind::Ident(_) | ExprKind::MemberAccess { .. } | ExprKind::Index { .. } => {
                self.tn_operand_type(e).is_some_and(|t| t.is_string())
            }
            _ => false,
        }
    }

    /// §20.6.1 / §11.6.1 / §11.8.1: the self-determined type of an integral
    /// expression. `None` for a shape it does not type.
    pub(super) fn typename_integral_expr(&mut self, e: &Expression) -> Option<TnType> {
        use crate::ast::expr::{BinaryOp as B, UnaryOp as U};
        match &e.kind {
            ExprKind::Paren(i) => self.typename_integral_expr(i),
            ExprKind::Number(NumberLiteral::Integer {
                size,
                signed,
                value,
                ..
            }) => {
                let four = value
                    .chars()
                    .any(|c| matches!(c, 'x' | 'X' | 'z' | 'Z' | '?'));
                Some(TnType::generic(size.unwrap_or(32), *signed, four))
            }
            ExprKind::Number(NumberLiteral::UnbasedUnsized(c)) => Some(TnType::generic(
                1,
                false,
                matches!(c, 'x' | 'X' | 'z' | 'Z'),
            )),
            ExprKind::Ident(_) | ExprKind::MemberAccess { .. } | ExprKind::Index { .. } => {
                self.tn_operand_type(e).filter(|t| !t.is_string())
            }
            ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } => {
                let inner = self.typename_integral_expr(expr)?;
                let w = match kind {
                    crate::ast::expr::RangeKind::Constant => {
                        (self.tn_const(left)? - self.tn_const(right)?).unsigned_abs() + 1
                    }
                    _ => self.tn_const(right)?.max(1) as u64,
                };
                Some(TnType::generic(w as u32, false, inner.four))
            }
            ExprKind::Unary { op, operand } => {
                let t = self.typename_integral_expr(operand)?;
                match op {
                    U::Plus
                    | U::Minus
                    | U::BitNot
                    | U::PreIncr
                    | U::PreDecr
                    | U::PostIncr
                    | U::PostDecr => Some(t),
                    _ => Some(TnType::generic(1, false, t.four)),
                }
            }
            ExprKind::Binary { op, left, right } => {
                let l = self.typename_integral_expr(left)?;
                let r = self.typename_integral_expr(right)?;
                match op {
                    B::Add | B::Sub | B::Mul | B::BitAnd | B::BitOr | B::BitXor | B::BitXnor => {
                        Some(TnType::merge(&[&l, &r], false))
                    }
                    B::Div | B::Mod => Some(TnType::merge(&[&l, &r], false).forced_four()),
                    // §11.4.3: a power takes its left operand's type.
                    B::Power => Some(l.forced_four()),
                    B::ShiftLeft | B::ShiftRight | B::ArithShiftLeft | B::ArithShiftRight => {
                        Some(l)
                    }
                    B::Eq
                    | B::Neq
                    | B::CaseEq
                    | B::CaseNeq
                    | B::WildcardEq
                    | B::WildcardNeq
                    | B::Lt
                    | B::Leq
                    | B::Gt
                    | B::Geq
                    | B::LogAnd
                    | B::LogOr
                    | B::LogImplies
                    | B::LogEquiv => Some(TnType::generic(1, false, l.four || r.four)),
                    _ => None,
                }
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                let c = self.typename_integral_expr(condition)?;
                let a = self.typename_integral_expr(then_expr)?;
                let b = self.typename_integral_expr(else_expr)?;
                Some(TnType::merge(&[&a, &b], c.four))
            }
            ExprKind::Concatenation(parts) => {
                if !parts.is_empty() && parts.iter().all(|p| self.tn_is_string(p)) {
                    return Some(TnType::unsized_string());
                }
                let mut w = 0u32;
                let mut four = false;
                for p in parts {
                    let t = self.typename_integral_expr(p)?;
                    w += t.width;
                    four |= t.four;
                }
                Some(TnType::generic(w, false, four))
            }
            // The reference simulator types a replication 4-state.
            ExprKind::Replication { count, exprs } => {
                let n = self.tn_const(count)?.max(0) as u32;
                let mut w = 0u32;
                for p in exprs {
                    w += self.typename_integral_expr(p)?.width;
                }
                Some(TnType::generic(n * w, false, true))
            }
            ExprKind::SystemCall { name, args } => self.tn_system_call(name, args),
            ExprKind::Call { func, .. } => {
                if self.callee_method_name_is(func, |m| matches!(m, "size" | "num" | "len")) {
                    return Some(TnType::named(32, true, false, "int"));
                }
                // A function result reads as a 2-state value of the return
                // type's width and sign.
                let ExprKind::Ident(h) = &func.kind else {
                    return None;
                };
                if h.path.len() != 1 {
                    return None;
                }
                let rt = self
                    .module
                    .functions
                    .get(&h.path[0].name.name)?
                    .return_type
                    .clone();
                let t = self.tn_type_of_dt(&rt, false)?;
                Some(TnType::generic(t.width, t.signed, false))
            }
            _ => None,
        }
    }

    fn tn_system_call(&mut self, name: &str, args: &[Expression]) -> Option<TnType> {
        match name {
            "$signed" | "$unsigned" => {
                let t = self.typename_integral_expr(args.first()?)?;
                Some(TnType::generic(t.width, name == "$signed", t.four))
            }
            // §6.24.1: a type cast names the target type.
            "$__xz_type_cast" => match &args.first()?.kind {
                ExprKind::TypeLiteral(dt) => self.tn_type_of_dt(dt, false),
                _ => None,
            },
            // A size cast keeps the operand's sign and state.
            "$__xz_size_cast" => {
                let w = self.tn_const(args.first()?)?;
                let t = self.typename_integral_expr(args.get(1)?)?;
                Some(TnType::generic(w as u32, t.signed, t.four))
            }
            // `id'(v)`: a typedef names its type, a constant sizes.
            "$__xz_named_cast" => {
                let id = match &args.first()?.kind {
                    ExprKind::Ident(h) if h.path.len() == 1 => h.path[0].name.name.clone(),
                    _ => return None,
                };
                if let Some(dt) = self.lookup_typedef_target(&id) {
                    return self.tn_type_of_dt(&dt, false);
                }
                let w = self.tn_const(args.first()?)?;
                let t = self.typename_integral_expr(args.get(1)?)?;
                Some(TnType::generic(w as u32, t.signed, t.four))
            }
            "$clog2" | "$countones" | "$countbits" | "$bits" => {
                Some(TnType::generic(32, true, false))
            }
            "$time" => Some(TnType::named(64, false, true, "time")),
            _ => None,
        }
    }
}
