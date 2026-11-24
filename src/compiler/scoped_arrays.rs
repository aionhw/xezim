//! Array-query operands needed to size a procedural declaration (§20.7).

use crate::ast::{
    expr::{ExprKind, Expression},
    types::{DataType, PackedDimension},
};
use xezim_core::{Value, elaborate, hasher::HashMap};

/// Only range expressions can need array metadata. Scalar declarations return
/// without allocating or scanning the design's array table.
pub(crate) fn query_operands(dt: &DataType) -> Vec<(&str, &Expression)> {
    fn dimensions<'a>(dims: &'a [PackedDimension], out: &mut Vec<(&'a str, &'a Expression)>) {
        for d in dims {
            if let PackedDimension::Range { left, right, .. } = d {
                for e in [left, right] {
                    crate::type_lint::visit_expr(e, &mut |e| {
                        if let ExprKind::SystemCall { name, args } = &e.kind
                            && matches!(
                                name.as_str(),
                                "$size"
                                    | "$left"
                                    | "$right"
                                    | "$low"
                                    | "$high"
                                    | "$increment"
                                    | "$dimensions"
                                    | "$unpacked_dimensions"
                                    | "$bits"
                            )
                            && let Some(arg) = args.first()
                        {
                            out.push((name.as_str(), arg));
                        }
                    });
                }
            }
        }
    }
    fn walk<'a>(dt: &'a DataType, out: &mut Vec<(&'a str, &'a Expression)>) {
        match dt {
            DataType::IntegerVector { dimensions: d, .. }
            | DataType::Implicit { dimensions: d, .. }
            | DataType::TypeReference { dimensions: d, .. } => dimensions(d, out),
            DataType::Struct(s) => {
                dimensions(&s.dimensions, out);
                for member in &s.members {
                    walk(&member.data_type, out);
                }
            }
            DataType::Enum(e) => {
                dimensions(&e.dimensions, out);
                if let Some(base) = &e.base_type {
                    walk(base, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(dt, &mut out);
    out
}

pub(crate) fn operand_name(expr: &Expression) -> Option<String> {
    match &expr.kind {
        ExprKind::Paren(e) => operand_name(e),
        ExprKind::MemberAccess { expr, member } => {
            Some(format!("{}.{}", operand_name(expr)?, member.name))
        }
        ExprKind::Ident(h) if h.root.is_none() && h.path.iter().all(|s| s.selects.is_empty()) => {
            Some(
                h.path
                    .iter()
                    .map(|s| s.name.name.as_str())
                    .collect::<Vec<_>>()
                    .join("."),
            )
        }
        _ => None,
    }
}

/// The caller resolves lexical names first. The temporary table contains only
/// queried arrays, so installing it never copies unrelated design metadata.
pub(crate) fn resolve_type_width(
    dt: &DataType,
    params: Option<&HashMap<String, Value>>,
    typedefs: Option<&HashMap<String, u32>>,
    arrays: &HashMap<String, (i64, i64, u32)>,
    widths: HashMap<String, u32>,
) -> u32 {
    let run = || {
        let resolve = || elaborate::resolve_type_width(dt, params, typedefs);
        if widths.is_empty() {
            resolve()
        } else {
            elaborate::with_query_widths(widths, resolve)
        }
    };
    if arrays.is_empty() {
        run()
    } else {
        elaborate::with_arrays(arrays, run)
    }
}
