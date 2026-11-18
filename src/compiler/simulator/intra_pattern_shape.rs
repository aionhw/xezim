//! The target type of each item of an assignment pattern captured by an
//! intra-assignment timing control (IEEE 1800-2023 §9.4.5, §10.9.1, §11.6.1).
//!
//! `w8 = #1 '{a4 + b4, 1}` evaluates its items when the statement runs, and
//! each is evaluated in the type of the element it is matched to, as the
//! same assignment without the delay would. The capture evaluated each item
//! at its own width, so a 4-bit `a4 + b4` lost its carry before the store.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

/// What a captured pattern item is matched to.
#[derive(Clone)]
pub(super) enum ItemShape {
    /// `depth` (at least 1) further unpacked dimensions of `elem`.
    Array { elem: DataType, depth: usize },
    /// The members of an unpacked struct.
    Struct(crate::ast::types::StructUnionType),
    /// One element of this type.
    Leaf(DataType),
}

impl Simulator {
    /// The shape of `lvalue` for its pattern: a fixed unpacked array, a row
    /// or element of one, or an unpacked struct variable. None for anything
    /// else (queues, dynamic and associative arrays, slices, member paths),
    /// whose items keep their own width.
    pub(super) fn intra_target_shape(&mut self, lvalue: &Expression) -> Option<ItemShape> {
        let mut selects = 0usize;
        let mut cur = lvalue;
        while let ExprKind::Index { expr, index } = &cur.kind {
            if matches!(index.kind, ExprKind::Range(..)) {
                return None;
            }
            selects += 1;
            cur = expr;
        }
        let ExprKind::Ident(h) = &cur.kind else {
            return None;
        };
        if h.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let name = self.resolve_hier_name(h).into_owned();
        if self.module.dynamic_arrays.contains(&name)
            || self.module.queue_vars.contains(&name)
            || self.is_associative_array(&name)
        {
            return None;
        }
        let elem = self.module.var_decl_types.get(&name)?.clone();
        let depth = self
            .module
            .unpacked_decl_dims
            .get(&name)
            .map_or(0, Vec::len);
        if depth == 0 {
            // A scalar's pattern is shaped by its packed type, earlier.
            return match self.item_shape(&elem) {
                s @ ItemShape::Struct(_) if selects == 0 => Some(s),
                _ => None,
            };
        }
        let shape = self.array_shape(elem, depth.checked_sub(selects)?);
        (!matches!(shape, ItemShape::Leaf(_))).then_some(shape)
    }

    /// The shape of one element of the array stored at `name`.
    pub(super) fn intra_element_shape(&self, name: &str) -> Option<ItemShape> {
        let elem = self.module.var_decl_types.get(name)?;
        Some(self.item_shape(elem))
    }

    /// The shape of `depth` more unpacked dimensions of `elem`.
    fn array_shape(&self, elem: DataType, depth: usize) -> ItemShape {
        if depth == 0 {
            self.item_shape(&elem)
        } else {
            ItemShape::Array { elem, depth }
        }
    }

    fn item_shape(&self, dt: &DataType) -> ItemShape {
        match self.resolve_dt(dt) {
            DataType::Struct(su) if Self::spreads_member_wise(&su) => ItemShape::Struct(su),
            _ => ItemShape::Leaf(dt.clone()),
        }
    }

    /// The shape of each item of a pattern matched to `shape`, in item
    /// order: an array's items are its elements; a struct's positional items
    /// take the members in order and a named item its member. None where the
    /// target is not known (`default:` or a type key on a struct).
    pub(super) fn intra_item_shapes(
        &self,
        shape: &ItemShape,
        items: &[crate::ast::expr::AssignmentPatternItem],
    ) -> Vec<Option<ItemShape>> {
        use crate::ast::expr::AssignmentPatternItem as Item;
        match shape {
            ItemShape::Array { elem, depth } => items
                .iter()
                .map(|item| {
                    if matches!(item, Item::Typed(..))
                        && !matches!(
                            Self::unparen(item.expr()).kind,
                            ExprKind::AssignmentPattern(_)
                        )
                    {
                        return Some(self.item_shape(elem));
                    }
                    Some(self.array_shape(elem.clone(), depth - 1))
                })
                .collect(),
            ItemShape::Struct(su) => {
                let all_members: Vec<(&str, &DataType, ItemShape)> = su
                    .members
                    .iter()
                    .flat_map(|m| {
                        let mdt = &m.data_type;
                        m.declarators.iter().map(move |md| {
                            let shape = if md.dimensions.is_empty() {
                                self.item_shape(mdt)
                            } else {
                                ItemShape::Array {
                                    elem: mdt.clone(),
                                    depth: md.dimensions.len(),
                                }
                            };
                            (md.name.name.as_str(), mdt, shape)
                        })
                    })
                    .collect();
                let mut members: Vec<Option<(&str, ItemShape)>> = all_members
                    .iter()
                    .map(|(n, _, s)| Some((*n, s.clone())))
                    .collect();
                let type_shape = |key_dt: &DataType| -> Option<ItemShape> {
                    let mut matched: Option<ItemShape> = None;
                    for (_, mdt, sh) in &all_members {
                        let DataType::TypeReference { name, .. } = key_dt else {
                            if std::mem::discriminant(&self.resolve_dt(mdt))
                                == std::mem::discriminant(&self.resolve_dt(key_dt))
                            {
                                matched = Some(sh.clone());
                            }
                            continue;
                        };
                        let DataType::TypeReference { name: mname, .. } = mdt else {
                            continue;
                        };
                        if mname.name.name == name.name.name {
                            matched = Some(sh.clone());
                        }
                    }
                    matched
                };
                let mut pos = 0;
                items
                    .iter()
                    .map(|item| match item {
                        Item::Ordered(_) => {
                            pos += 1;
                            members.get_mut(pos - 1)?.take().map(|(_, s)| s)
                        }
                        Item::Named(n, _) => {
                            if let Some(m) = members
                                .iter_mut()
                                .find(|m| m.as_ref().is_some_and(|(mn, _)| *mn == n.name))
                            {
                                return m.take().map(|(_, s)| s);
                            }
                            self.array_pattern_type_key(item)
                                .and_then(|(dt, _)| type_shape(&dt))
                        }
                        Item::Typed(_, _) | Item::Keyed(_, _) => {
                            let (dt, _) = self.array_pattern_type_key(item)?;
                            type_shape(&dt)
                        }
                        _ => None,
                    })
                    .collect()
            }
            ItemShape::Leaf(_) => items.iter().map(|_| None).collect(),
        }
    }

    /// The captured value of the non-pattern item `e` matched to `shape`:
    /// in the element type for an element, or for a value that fills every
    /// element of a sub-array (`'{default: a + b}`). An item that may itself
    /// be an array or struct value keeps its own evaluation.
    pub(super) fn intra_item_value(&mut self, shape: Option<&ItemShape>, e: &Expression) -> Value {
        let leaf = match shape {
            Some(ItemShape::Leaf(dt)) => Some(dt.clone()),
            Some(ItemShape::Array { elem, .. })
                if !matches!(
                    Self::unparen(e).kind,
                    ExprKind::Ident(_)
                        | ExprKind::Index { .. }
                        | ExprKind::MemberAccess { .. }
                        | ExprKind::Call { .. }
                        | ExprKind::SystemCall { .. }
                ) =>
            {
                match self.item_shape(elem) {
                    ItemShape::Leaf(dt) => Some(dt),
                    _ => None,
                }
            }
            _ => None,
        };
        match leaf {
            Some(dt) => self.eval_pattern_leaf_value(&dt, e, None),
            None => self.eval_expr(e),
        }
    }
}
