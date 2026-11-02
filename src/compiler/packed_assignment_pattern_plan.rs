//! IEEE 1800-2023 §10.9: the slices of an assignment pattern in a packed type.
//!
//! A pattern takes the type of its assignment-like context (§10.8), or its
//! own with a type prefix (`T'{...}`). Each slice holds an item, evaluated as
//! an assignment to the slice's type (§10.9.1, §10.9.2), or a constant. The
//! interpreter executes a plan; the bytecode compiler emits it.

use crate::ast::expr::{AssignmentPatternItem, ExprKind, Expression};
use crate::ast::types::{
    DataType, IntegerAtomType, IntegerVectorType, PackedDimension, Signing, StructUnionKind,
    StructUnionType, TypeName,
};
use xezim_core::Value;

/// Elements per pattern level beyond which no plan is built.
const MAX_ELEMS: usize = 1 << 16;

/// Type tables a plan is built against.
pub(crate) trait PatternTypeResolver {
    /// The target of a named type.
    fn typedef_target(&mut self, name: &TypeName) -> Option<DataType>;
    /// A constant expression (packed bounds, index keys, replication counts).
    fn const_i64(&mut self, e: &Expression) -> Option<i64>;
    /// Whether a bare `name:` key names a type (§10.9.2 type key).
    fn is_type_name(&mut self, name: &str) -> bool;
}

/// What fills one slice of a plan.
pub(crate) enum SliceSrc<'a> {
    /// An item, evaluated as an assignment to the slice's type.
    Expr(&'a Expression),
    /// Bits no item sets.
    Const(Value),
    /// `c ? p1 : p2` with pattern operands: each operand in the slice's
    /// type; an x/z condition merges both bit by bit (§11.4.11).
    Cond {
        cond: &'a Expression,
        then: Box<PackedPlan<'a>>,
        els: Box<PackedPlan<'a>>,
    },
}

/// One slice `[offset +: width]` of a plan.
pub(crate) struct Slice<'a> {
    pub offset: u32,
    pub width: u32,
    /// The slice's type is 2-state: x/z bits convert to 0.
    pub two_state: bool,
    pub src: SliceSrc<'a>,
}

/// The value of a pattern in a packed type: slices applied in order over
/// zeros. Slices overlap only for a packed union with more than one member
/// set.
pub(crate) struct PackedPlan<'a> {
    pub width: u32,
    pub signed: bool,
    pub slices: Vec<Slice<'a>>,
    /// Members or elements no rule covers (§10.9.1, §10.9.2), for a warning.
    pub uncovered: bool,
}

impl PackedPlan<'_> {
    /// Whether no two slices overlap.
    pub fn disjoint(&self) -> bool {
        let mut spans: Vec<(u32, u32)> = self.slices.iter().map(|s| (s.offset, s.width)).collect();
        spans.sort_unstable();
        spans.windows(2).all(|p| p[0].0 + p[0].1 <= p[1].0)
    }
}

/// Whether `e` is a pattern, or a parenthesised or conditional expression
/// with a pattern operand.
pub(crate) fn has_pattern(e: &Expression) -> bool {
    match &e.kind {
        ExprKind::AssignmentPattern(_) | ExprKind::TypedPattern { .. } => true,
        ExprKind::Paren(inner) => has_pattern(inner),
        ExprKind::Conditional {
            then_expr,
            else_expr,
            ..
        } => has_pattern(then_expr) || has_pattern(else_expr),
        _ => false,
    }
}

/// The plan of `e`, a pattern (or a parenthesised or conditional expression
/// with pattern operands) assigned to `ty`. None when `ty` is not a packed
/// aggregate or the pattern cannot be laid out statically.
pub(crate) fn packed_plan<'a>(
    e: &'a Expression,
    ty: &DataType,
    r: &mut dyn PatternTypeResolver,
) -> Option<PackedPlan<'a>> {
    let mut b = PlanBuilder {
        r,
        uncovered: false,
    };
    if matches!(b.view(ty)?, Shape::Leaf { .. }) {
        return None;
    }
    let width = b.width(ty)?;
    let signed = b.signed(ty);
    let mut slices = Vec::new();
    b.expr(e, ty, 0, width, &mut slices)?;
    Some(PackedPlan {
        width,
        signed,
        slices,
        uncovered: b.uncovered,
    })
}

/// Whether `ty` is a packed type (a pattern assigned to it is one value).
pub(crate) fn is_packed(ty: &DataType, r: &mut dyn PatternTypeResolver) -> bool {
    PlanBuilder {
        r,
        uncovered: false,
    }
    .view(ty)
    .is_some()
}

/// The width of the packed type `ty`.
pub(crate) fn packed_width(ty: &DataType, r: &mut dyn PatternTypeResolver) -> Option<u32> {
    PlanBuilder {
        r,
        uncovered: false,
    }
    .width(ty)
}

/// A struct or union type with the layout `(path, offset, width)` records
/// at `prefix`, members `logic` vectors unless they have members of their
/// own; overlapping members make it a union.
pub(crate) fn layout_struct_type(layout: &[(String, u32, u32)], prefix: &str) -> Option<DataType> {
    use crate::ast::types::{StructDeclarator, StructMember};
    let pfx = if prefix.is_empty() {
        String::new()
    } else {
        format!("{prefix}.")
    };
    let mut kids: Vec<(&str, u32, u32)> = layout
        .iter()
        .filter_map(|(k, o, w)| {
            let leaf = k.strip_prefix(&pfx)?;
            (!leaf.is_empty() && !leaf.contains(['.', '['])).then_some((leaf, *o, *w))
        })
        .collect();
    if kids.is_empty() {
        return None;
    }
    kids.sort_by(|a, b| b.1.cmp(&a.1));
    let lo = kids.iter().map(|k| k.1).min()?;
    let hi = kids.iter().map(|k| k.1 + k.2).max()?;
    let union = kids.iter().map(|k| k.2).sum::<u32>() > hi - lo;
    let span = crate::ast::Span::dummy();
    let members = kids
        .iter()
        .map(|&(leaf, _, w)| StructMember {
            rand_qualifier: None,
            data_type: layout_struct_type(layout, &format!("{pfx}{leaf}"))
                .unwrap_or_else(|| logic_vector(w)),
            declarators: vec![StructDeclarator {
                name: crate::ast::Identifier {
                    name: leaf.to_string(),
                    span,
                },
                dimensions: Vec::new(),
                init: None,
                span,
            }],
            span,
        })
        .collect();
    Some(DataType::Struct(StructUnionType {
        kind: if union {
            StructUnionKind::Union
        } else {
            StructUnionKind::Struct
        },
        packed: true,
        tagged: false,
        soft: false,
        signing: None,
        members,
        dimensions: Vec::new(),
        span,
    }))
}

/// A literal integer expression.
fn int_lit(v: i64) -> Expression {
    use crate::ast::expr::{NumberBase, NumberLiteral, UnaryOp};
    let span = crate::ast::Span::dummy();
    let lit = Expression::new(
        ExprKind::Number(NumberLiteral::Integer {
            size: None,
            signed: true,
            base: NumberBase::Decimal,
            value: v.unsigned_abs().to_string(),
            cached_val: std::cell::Cell::new(None),
        }),
        span,
    );
    if v < 0 {
        Expression::new(
            ExprKind::Unary {
                op: UnaryOp::Minus,
                operand: Box::new(lit),
            },
            span,
        )
    } else {
        lit
    }
}

/// `logic [w-1:0]` (`logic` for one bit).
fn logic_vector(w: u32) -> DataType {
    packed_dims_vector(IntegerVectorType::Logic, &[(i64::from(w) - 1, 0)])
}

/// A `bit`/`logic` vector with the packed dimensions `dims`; a single
/// `[0:0]` is the scalar.
pub(crate) fn packed_dims_vector(kind: IntegerVectorType, dims: &[(i64, i64)]) -> DataType {
    let span = crate::ast::Span::dummy();
    let dimensions = if dims == [(0, 0)] {
        Vec::new()
    } else {
        dims.iter()
            .map(|&(l, r)| PackedDimension::Range {
                left: Box::new(int_lit(l)),
                right: Box::new(int_lit(r)),
                span,
            })
            .collect()
    };
    DataType::IntegerVector {
        kind,
        signing: None,
        dimensions,
        span,
    }
}

/// One level of a packed type, as a pattern sees it.
enum Shape {
    /// A packed dimension `[left:right]` over `elem`.
    Array {
        left: i64,
        right: i64,
        elem: DataType,
    },
    /// A packed struct or union, without packed dimensions.
    Struct(StructUnionType),
    /// A non-aggregate.
    Leaf { width: u32 },
}

/// A `key:` of a pattern item: a bare name or an index expression.
#[derive(Clone, Copy)]
enum Key<'a> {
    Name(&'a crate::ast::Identifier),
    Expr(&'a Expression),
}

/// Keys of one pattern level, classified by §10.9.1/§10.9.2.
#[derive(Default, Clone)]
struct Keys<'a> {
    ordered: Vec<&'a Expression>,
    keyed: Vec<(Key<'a>, &'a Expression)>,
    types: Vec<(DataType, &'a Expression)>,
    default: Option<&'a Expression>,
}

struct PlanBuilder<'r> {
    r: &'r mut dyn PatternTypeResolver,
    uncovered: bool,
}

impl PlanBuilder<'_> {
    /// Writes for `e` assigned to the slice `[offset +: width]` of type `ty`.
    fn expr<'a>(
        &mut self,
        e: &'a Expression,
        ty: &DataType,
        offset: u32,
        width: u32,
        out: &mut Vec<Slice<'a>>,
    ) -> Option<()> {
        match &e.kind {
            ExprKind::Paren(inner) if has_pattern(inner) => {
                self.expr(inner, ty, offset, width, out)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } if has_pattern(e) => {
                let arm = |b: &mut Self, x: &'a Expression| -> Option<Box<PackedPlan<'a>>> {
                    let mut slices = Vec::new();
                    b.expr(x, ty, 0, width, &mut slices)?;
                    Some(Box::new(PackedPlan {
                        width,
                        signed: false,
                        slices,
                        uncovered: false,
                    }))
                };
                let then = arm(self, then_expr)?;
                let els = arm(self, else_expr)?;
                out.push(Slice {
                    offset,
                    width,
                    two_state: false,
                    src: SliceSrc::Cond {
                        cond: condition,
                        then,
                        els,
                    },
                });
                Some(())
            }
            ExprKind::AssignmentPattern(items) if !matches!(self.view(ty)?, Shape::Leaf { .. }) => {
                let keys = self.keys(items, ty)?;
                self.level(ty, &keys, offset, out)
            }
            _ => {
                let two_state = self.two_state(ty);
                out.push(Slice {
                    offset,
                    width,
                    two_state,
                    src: SliceSrc::Expr(e),
                });
                Some(())
            }
        }
    }

    /// Writes for one pattern level of `ty` at `offset`.
    fn level<'a>(
        &mut self,
        ty: &DataType,
        keys: &Keys<'a>,
        offset: u32,
        out: &mut Vec<Slice<'a>>,
    ) -> Option<()> {
        match self.view(ty)? {
            Shape::Array { left, right, elem } => {
                let n = usize::try_from(left.abs_diff(right).checked_add(1)?).ok()?;
                if n > MAX_ELEMS {
                    return None;
                }
                let ew = self.width(&elem)?;
                let mut explicit: Vec<Option<&'a Expression>> = vec![None; n];
                for (pos, e) in keys.ordered.iter().enumerate().take(n) {
                    explicit[pos] = Some(e);
                }
                if !keys.ordered.is_empty() && keys.ordered.len() != n {
                    self.uncovered = true;
                }
                for (k, e) in &keys.keyed {
                    let idx = match k {
                        Key::Expr(k) => self.r.const_i64(k)?,
                        Key::Name(id) => self.r.const_i64(&ident_expr(id))?,
                    };
                    let pos = if left >= right {
                        left - idx
                    } else {
                        idx - left
                    };
                    *explicit.get_mut(usize::try_from(pos).ok()?)? = Some(e);
                }
                // The first item is the leftmost index, the most significant
                // element (§7.4.1).
                for (pos, e) in explicit.into_iter().enumerate() {
                    let at = offset + (n - 1 - pos) as u32 * ew;
                    match e {
                        Some(e) => self.expr(e, &elem, at, ew, out)?,
                        None => self.unkeyed(keys, &elem, at, ew, out)?,
                    }
                }
                Some(())
            }
            Shape::Struct(su) => {
                let members: Vec<(&str, &DataType)> = su
                    .members
                    .iter()
                    .flat_map(|m| {
                        m.declarators
                            .iter()
                            .map(move |d| (d.name.name.as_str(), &m.data_type))
                    })
                    .collect();
                let mut widths = Vec::with_capacity(members.len());
                for (_, t) in &members {
                    widths.push(self.width(t)?);
                }
                if !keys.ordered.is_empty() && keys.ordered.len() != members.len() {
                    self.uncovered = true;
                }
                let union = su.kind == StructUnionKind::Union;
                let total: u32 = if union {
                    widths.iter().copied().max().unwrap_or(0)
                } else {
                    widths.iter().sum()
                };
                let mut below = total;
                let mut covered = false;
                for (i, (name, mty)) in members.iter().enumerate() {
                    let w = widths[i];
                    let at = if union {
                        offset
                    } else {
                        below -= w;
                        offset + below
                    };
                    let explicit = if keys.ordered.is_empty() {
                        keys.keyed.iter().rev().find_map(|(k, e)| {
                            let hit = match k {
                                Key::Name(id) => id.name == *name,
                                Key::Expr(k) => matches!(&k.kind, ExprKind::Ident(h)
                                    if h.path.len() == 1 && h.path[0].name.name == *name),
                            };
                            hit.then_some(*e)
                        })
                    } else {
                        keys.ordered.get(i).copied()
                    };
                    match explicit {
                        Some(e) => self.expr(e, mty, at, w, out)?,
                        // A union takes the members a key sets (§7.3.1).
                        None if union => {
                            if keys.types.is_empty() && keys.default.is_none() {
                                continue;
                            }
                            self.unkeyed(keys, mty, at, w, out)?;
                        }
                        None => self.unkeyed(keys, mty, at, w, out)?,
                    }
                    if union {
                        covered = true;
                        // §7.3.1: bits of a soft union above the member take
                        // the union's uninitialized value, x for 4-state.
                        if w < total {
                            let pad = if self.two_state(&DataType::Struct(su.clone())) {
                                Value::zero(total - w)
                            } else {
                                Value::new(total - w)
                            };
                            out.push(Slice {
                                offset: offset + w,
                                width: total - w,
                                two_state: false,
                                src: SliceSrc::Const(pad),
                            });
                        }
                    }
                }
                if union && !covered {
                    self.uncovered = true;
                    out.push(Slice {
                        offset,
                        width: total,
                        two_state: false,
                        src: SliceSrc::Const(Value::zero(total)),
                    });
                }
                Some(())
            }
            Shape::Leaf { .. } => None,
        }
    }

    /// A member or element no positional, member or index key sets: a
    /// matching type key (the last one, §10.9.2), else `default:` (descending
    /// into an aggregate that does not take the value whole), else zero:
    /// every element shall be covered (§10.9.1, §10.9.2).
    fn unkeyed<'a>(
        &mut self,
        keys: &Keys<'a>,
        ty: &DataType,
        offset: u32,
        width: u32,
        out: &mut Vec<Slice<'a>>,
    ) -> Option<()> {
        let mut typed = None;
        for (t, e) in keys.types.iter().rev() {
            if self.types_match(t, ty) {
                typed = Some(*e);
                break;
            }
        }
        if let Some(e) = typed {
            return self.expr(e, ty, offset, width, out);
        }
        if (!keys.types.is_empty() || keys.default.is_some())
            && self.default_descends(ty, keys.default)
        {
            let sub = Keys {
                types: keys.types.clone(),
                default: keys.default,
                ..Keys::default()
            };
            return self.level(ty, &sub, offset, out);
        }
        if let Some(d) = keys.default {
            return self.expr(d, ty, offset, width, out);
        }
        self.uncovered = true;
        out.push(Slice {
            offset,
            width,
            two_state: false,
            src: SliceSrc::Const(Value::zero(width)),
        });
        Some(())
    }

    /// §10.9.1, §10.9.2: type and default keys descend into `ty` unless it
    /// takes the default whole: a simple bit vector, a non-aggregate, or a
    /// value of `ty`'s own type.
    fn default_descends(&mut self, ty: &DataType, default: Option<&Expression>) -> bool {
        if let Some(d) = default {
            match &d.kind {
                ExprKind::AssignmentPattern(_) => return false,
                ExprKind::TypedPattern { ty: own, .. } if self.types_match(own, ty) => {
                    return false;
                }
                _ => {}
            }
        }
        match self.view(ty) {
            Some(Shape::Struct(_)) => true,
            Some(Shape::Array { elem, .. }) => {
                !matches!(self.view(&elem), Some(Shape::Leaf { width: 1 }))
            }
            _ => false,
        }
    }

    /// Sort the items of a level of `ty`. A `name:` key that names no
    /// member of a struct `ty`, and names a type, is a type key.
    fn keys<'a>(&mut self, items: &'a [AssignmentPatternItem], ty: &DataType) -> Option<Keys<'a>> {
        let members: Vec<String> = match self.view(ty)? {
            Shape::Struct(su) => su
                .members
                .iter()
                .flat_map(|m| m.declarators.iter().map(|d| d.name.name.clone()))
                .collect(),
            _ => Vec::new(),
        };
        let mut keys = Keys::default();
        for it in items {
            match it {
                AssignmentPatternItem::Ordered(e) => match &e.kind {
                    // §10.9.1, §10.9.2: `'{n{a, b}}` repeats its items.
                    ExprKind::Replication { count, exprs } => {
                        let c = usize::try_from(self.r.const_i64(count)?).ok()?;
                        if c.checked_mul(exprs.len())? > MAX_ELEMS {
                            return None;
                        }
                        for _ in 0..c {
                            keys.ordered.extend(exprs.iter());
                        }
                    }
                    _ => keys.ordered.push(e),
                },
                AssignmentPatternItem::Default(e) => keys.default = Some(e),
                AssignmentPatternItem::Typed(t, e) => keys.types.push((t.clone(), e)),
                AssignmentPatternItem::Named(id, e) => {
                    if !members.contains(&id.name) && self.r.is_type_name(&id.name) {
                        keys.types.push((type_ref(&id.name), e));
                    } else {
                        keys.keyed.push((Key::Name(id), e));
                    }
                }
                AssignmentPatternItem::Keyed(k, e) => {
                    let ident = match &k.kind {
                        ExprKind::Ident(h) if h.path.len() == 1 && h.root.is_none() => {
                            Some(h.path[0].name.name.as_str())
                        }
                        _ => None,
                    };
                    match ident {
                        Some(n) if !members.iter().any(|m| m == n) && self.r.is_type_name(n) => {
                            keys.types.push((type_ref(n), e))
                        }
                        _ => keys.keyed.push((Key::Expr(k), e)),
                    }
                }
            }
        }
        Some(keys)
    }

    fn dim(&mut self, d: &PackedDimension) -> Option<(i64, i64)> {
        let PackedDimension::Range { left, right, .. } = d else {
            return None;
        };
        Some((self.r.const_i64(left)?, self.r.const_i64(right)?))
    }

    /// The outermost level of `dt`.
    fn view(&mut self, dt: &DataType) -> Option<Shape> {
        let mut cur = dt.clone();
        for _ in 0..64 {
            match cur {
                DataType::TypeReference {
                    name,
                    dimensions,
                    type_args,
                    span,
                } => {
                    if let Some((d, rest)) = dimensions.split_first() {
                        let (left, right) = self.dim(d)?;
                        let elem = DataType::TypeReference {
                            name,
                            dimensions: rest.to_vec(),
                            type_args,
                            span,
                        };
                        return Some(Shape::Array { left, right, elem });
                    }
                    cur = self.r.typedef_target(&name)?;
                }
                DataType::IntegerVector {
                    kind,
                    signing,
                    dimensions,
                    span,
                } => {
                    let Some((d, rest)) = dimensions.split_first() else {
                        return Some(Shape::Leaf { width: 1 });
                    };
                    let (left, right) = self.dim(d)?;
                    let elem = DataType::IntegerVector {
                        kind,
                        signing: if rest.is_empty() { None } else { signing },
                        dimensions: rest.to_vec(),
                        span,
                    };
                    return Some(Shape::Array { left, right, elem });
                }
                DataType::Implicit {
                    signing,
                    dimensions,
                    span,
                } => {
                    cur = DataType::IntegerVector {
                        kind: IntegerVectorType::Logic,
                        signing,
                        dimensions,
                        span,
                    };
                }
                // §6.11: an integer atom is a vector `[w-1:0]` of bits.
                DataType::IntegerAtom { kind, .. } => {
                    let (w, four_state) = atom_shape(kind);
                    let elem = DataType::IntegerVector {
                        kind: if four_state {
                            IntegerVectorType::Logic
                        } else {
                            IntegerVectorType::Bit
                        },
                        signing: None,
                        dimensions: Vec::new(),
                        span: crate::ast::Span::dummy(),
                    };
                    return Some(Shape::Array {
                        left: i64::from(w) - 1,
                        right: 0,
                        elem,
                    });
                }
                DataType::Struct(mut su) => {
                    if !su.packed {
                        return None;
                    }
                    if su.dimensions.is_empty() {
                        return Some(Shape::Struct(su));
                    }
                    let d = su.dimensions.remove(0);
                    let (left, right) = self.dim(&d)?;
                    return Some(Shape::Array {
                        left,
                        right,
                        elem: DataType::Struct(su),
                    });
                }
                DataType::Enum(mut e) => {
                    if e.dimensions.is_empty() {
                        let width = match e.base_type.as_deref() {
                            Some(b) => self.width(b)?,
                            None => 32,
                        };
                        return Some(Shape::Leaf { width });
                    }
                    let d = e.dimensions.remove(0);
                    let (left, right) = self.dim(&d)?;
                    return Some(Shape::Array {
                        left,
                        right,
                        elem: DataType::Enum(e),
                    });
                }
                _ => return None,
            }
        }
        None
    }

    /// The bit width of packed `dt`.
    fn width(&mut self, dt: &DataType) -> Option<u32> {
        match self.view(dt)? {
            Shape::Leaf { width } => Some(width),
            Shape::Array { left, right, elem } => {
                let n = u32::try_from(left.abs_diff(right) + 1).ok()?;
                n.checked_mul(self.width(&elem)?)
            }
            Shape::Struct(su) => {
                let union = su.kind == StructUnionKind::Union;
                let mut total: u32 = 0;
                for m in &su.members {
                    let w = self.width(&m.data_type)?;
                    for _ in &m.declarators {
                        total = if union {
                            total.max(w)
                        } else {
                            total.checked_add(w)?
                        };
                    }
                }
                Some(total)
            }
        }
    }

    /// Whether `dt` is signed: a `signed` packed struct, union or vector, or
    /// a signed integer atom (§6.11, §7.2.1).
    fn signed(&mut self, dt: &DataType) -> bool {
        let mut cur = dt.clone();
        for _ in 0..64 {
            match cur {
                DataType::TypeReference {
                    ref name,
                    ref dimensions,
                    ..
                } if dimensions.is_empty() => match self.r.typedef_target(name) {
                    Some(t) => cur = t,
                    None => return false,
                },
                DataType::IntegerVector { signing, .. } | DataType::Implicit { signing, .. } => {
                    return signing == Some(Signing::Signed);
                }
                DataType::IntegerAtom { kind, signing, .. } => {
                    return match signing {
                        Some(s) => s == Signing::Signed,
                        None => kind != IntegerAtomType::Time,
                    };
                }
                DataType::Struct(su) => {
                    return su.dimensions.is_empty() && su.signing == Some(Signing::Signed);
                }
                _ => return false,
            }
        }
        false
    }

    /// Whether every bit of `dt` is 2-state (§6.11.2).
    fn two_state(&mut self, dt: &DataType) -> bool {
        let mut cur = dt.clone();
        for _ in 0..64 {
            match cur {
                DataType::TypeReference { ref name, .. } => match self.r.typedef_target(name) {
                    Some(t) => cur = t,
                    None => return false,
                },
                DataType::IntegerVector { kind, .. } => return kind == IntegerVectorType::Bit,
                DataType::IntegerAtom { kind, .. } => return !atom_shape(kind).1,
                DataType::Enum(e) => match e.base_type {
                    Some(b) => cur = *b,
                    None => return true,
                },
                DataType::Struct(su) => {
                    return su.members.iter().all(|m| self.two_state(&m.data_type));
                }
                _ => return false,
            }
        }
        false
    }

    /// §6.22.1 matching types, as used by pattern type keys: named types
    /// match their targets, a simple bit vector matches a built-in of the
    /// same width, signing and state, and a struct, union or enum matches
    /// only its own declaration.
    fn types_match(&mut self, a: &DataType, b: &DataType) -> bool {
        match (self.signature(a), self.signature(b)) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        }
    }

    fn signature(&mut self, dt: &DataType) -> Option<String> {
        let mut out = String::new();
        let mut cur = dt.clone();
        for _ in 0..64 {
            match cur {
                DataType::TypeReference {
                    ref name,
                    ref dimensions,
                    ..
                } if dimensions.is_empty() => {
                    cur = self.r.typedef_target(name)?;
                }
                DataType::IntegerVector {
                    kind,
                    signing,
                    ref dimensions,
                    ..
                } if dimensions.len() <= 1 => {
                    let state = if kind == IntegerVectorType::Bit { 2 } else { 4 };
                    let signed = signing == Some(Signing::Signed);
                    match dimensions.first() {
                        Some(d) => {
                            let (l, r) = self.dim(d)?;
                            out.push_str(&format!("v{state}{signed}[{l}:{r}]"));
                        }
                        None => out.push_str(&format!("b{state}{signed}")),
                    }
                    return Some(out);
                }
                DataType::Implicit {
                    signing,
                    dimensions,
                    span,
                } => {
                    cur = DataType::IntegerVector {
                        kind: IntegerVectorType::Logic,
                        signing,
                        dimensions,
                        span,
                    };
                }
                DataType::IntegerAtom { kind, signing, .. } => {
                    let (w, four_state) = atom_shape(kind);
                    let signed = match signing {
                        Some(s) => s == Signing::Signed,
                        None => kind != IntegerAtomType::Time,
                    };
                    let state = if four_state { 4 } else { 2 };
                    out.push_str(&format!("v{state}{signed}[{}:0]", w - 1));
                    return Some(out);
                }
                DataType::Real { kind, .. } => {
                    out.push_str(&format!("r{kind:?}"));
                    return Some(out);
                }
                DataType::Simple { kind, .. } => {
                    out.push_str(&format!("s{kind:?}"));
                    return Some(out);
                }
                DataType::Struct(su) if su.dimensions.is_empty() => {
                    out.push_str(&format!("u{}:{}", su.span.start, su.span.end));
                    return Some(out);
                }
                DataType::Enum(e) if e.dimensions.is_empty() => {
                    out.push_str(&format!("e{}:{}", e.span.start, e.span.end));
                    return Some(out);
                }
                // A packed array: its outer dimension, then its element.
                other => match self.view(&other)? {
                    Shape::Array { left, right, elem } => {
                        out.push_str(&format!("[{left}:{right}]"));
                        cur = elem;
                    }
                    _ => return None,
                },
            }
        }
        None
    }
}

/// Width and 4-state-ness of an integer atom (§6.11).
fn atom_shape(kind: IntegerAtomType) -> (u32, bool) {
    match kind {
        IntegerAtomType::Byte => (8, false),
        IntegerAtomType::ShortInt => (16, false),
        IntegerAtomType::Int => (32, false),
        IntegerAtomType::LongInt => (64, false),
        IntegerAtomType::Integer => (32, true),
        IntegerAtomType::Time => (64, true),
    }
}

fn ident_expr(id: &crate::ast::Identifier) -> Expression {
    use crate::ast::expr::{HierPathSegment, HierarchicalIdentifier};
    Expression::new(
        ExprKind::Ident(HierarchicalIdentifier {
            root: None,
            path: vec![HierPathSegment {
                name: id.clone(),
                selects: Vec::new(),
            }],
            span: id.span,
            cached_signal_id: std::cell::Cell::new(None),
            cached_resolved_name: std::cell::OnceCell::new(),
        }),
        id.span,
    )
}

fn type_ref(name: &str) -> DataType {
    let span = crate::ast::Span::dummy();
    DataType::TypeReference {
        name: TypeName {
            scopes: Vec::new(),
            name: crate::ast::Identifier {
                name: name.to_string(),
                span,
            },
            span,
        },
        dimensions: Vec::new(),
        type_args: Vec::new(),
        span,
    }
}
