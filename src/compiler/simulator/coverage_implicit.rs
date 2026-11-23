//! Implicit coverpoints of crossed variables (§19.6).
//!
//! A variable named in a cross that has no coverpoint of its own gets an
//! implicit coverpoint over it, as if `k: coverpoint k;` were declared. That
//! coverpoint belongs to the covergroup, not to the cross: it has no guard,
//! so a cross's `iff` does not gate it, and it is sampled once per
//! `sample()` however many crosses name the variable.

use super::*;

impl Simulator {
    /// The expression an implicit coverpoint samples: the crossed name. A
    /// dotted item is a hierarchical reference (XEZIM_RELAX_CROSS_HIER_ITEM);
    /// an escaped name (`\a.b`) keeps its dot unless the switch made it a
    /// path.
    pub(super) fn cg_implicit_point_expr(id: &crate::ast::Identifier) -> Expression {
        let path: Vec<&str> = if sv_parser::RELAX_CROSS_HIER_ITEM.get() {
            id.name.split('.').collect()
        } else {
            vec![id.name.as_str()]
        };
        Expression::new(
            ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                root: None,
                path: path
                    .into_iter()
                    .map(|n| crate::ast::expr::HierPathSegment {
                        name: crate::ast::Identifier {
                            name: n.to_string(),
                            span: id.span,
                        },
                        selects: Vec::new(),
                    })
                    .collect(),
                span: id.span,
                cached_signal_id: crate::ast::cache::SignalCache::new(None),
                cached_resolved_name: crate::ast::cache::NameCache::new(),
                bound: crate::ast::binding::Bound::UNSET,
            }),
            id.span,
        )
    }

    /// Record one hit on each implicit coverpoint of the covergroup, skipping
    /// any that `stop()` turned off.
    pub(super) fn cg_sample_implicit_points(
        &mut self,
        handle: usize,
        def: &crate::ast::decl::CovergroupDeclaration,
        stopped_items: &HashSet<String>,
    ) {
        let names = Self::cg_implicit_points(def);
        if names.is_empty() {
            return;
        }
        let mut hits: Vec<(String, Value)> = Vec::new();
        for it in &def.items {
            let CovergroupItem::Cross(cr) = it else {
                continue;
            };
            for id in &cr.items {
                if !names.contains(&id.name)
                    || stopped_items.contains(&id.name)
                    || hits.iter().any(|(n, _)| n == &id.name)
                {
                    continue;
                }
                let v = self.eval_expr(&Self::cg_implicit_point_expr(id));
                hits.push((id.name.clone(), v));
            }
        }
        if let Some(Some(inst)) = self.cg_heap.get_mut(handle) {
            for (name, v) in hits {
                *inst
                    .point_hits
                    .entry(name)
                    .or_default()
                    .entry(v)
                    .or_insert(0) += 1;
            }
        }
    }
}
