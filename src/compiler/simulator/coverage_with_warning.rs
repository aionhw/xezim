//! §19.6.1: a cross bin's `with (expr)` selects the product bins holding a
//! value tuple for which `expr`, over the crossed coverpoint names, is true.
//! An `expr` that names something else (a module variable, say, instead of
//! the coverpoint that samples it) cannot be evaluated over those values,
//! so the bin selects nothing. That used to happen without a word.

use super::*;

impl Simulator {
    /// Warn, once per `with` expression, that `e` of a bin of cross `cr`
    /// could not be evaluated for any value tuple of the crossed coverpoints.
    pub(super) fn cg_warn_unevaluable_with(e: &Expression, cr: &crate::ast::decl::Cross) {
        static SEEN: std::sync::Mutex<Option<HashSet<(String, usize)>>> =
            std::sync::Mutex::new(None);
        let points: Vec<&str> = cr.items.iter().map(|i| i.name.as_str()).collect();
        let cross = cr
            .name
            .as_ref()
            .map(|n| n.name.clone())
            .unwrap_or_else(|| points.join(", "));
        let key = (cross.clone(), e.span.start as usize);
        let mut seen = SEEN.lock().unwrap_or_else(|p| p.into_inner());
        if !seen.get_or_insert_with(HashSet::default).insert(key) {
            return;
        }
        eprintln!(
            "[xezim][warning] the `with` expression of a bin of cross '{}' cannot be \
             evaluated over the values of its coverpoints ({}): it names something other \
             than those coverpoints and constants, so the bin selects nothing (IEEE \
             1800-2017 §19.6.1)",
            cross,
            points.join(", ")
        );
    }
}
