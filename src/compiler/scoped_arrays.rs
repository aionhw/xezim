//! Overlay instance-qualified `module.arrays` entries onto bare names for
//! const-eval of `$size`/`$clog2` in block-local packed dimensions.

use crate::ast::types::DataType;
use xezim_core::Value;
use xezim_core::elaborate;
use xezim_core::hasher::HashMap;

/// Build an `ARRAYS_TLS` view where `scope.name` entries override bare `name`.
pub(crate) fn arrays_view_for_instance_scope(
    scope: &str,
    arrays: &HashMap<String, (i64, i64, u32)>,
) -> HashMap<String, (i64, i64, u32)> {
    let mut view = arrays.clone();
    let pref = format!("{}.", scope);
    for (k, v) in arrays {
        if let Some(rest) = k.strip_prefix(&pref)
            && !rest.contains('.')
            && !rest.contains('#')
        {
            view.insert(rest.to_string(), *v);
        }
    }
    view
}

fn instance_has_scoped_array_keys(
    scope: &str,
    arrays: &HashMap<String, (i64, i64, u32)>,
) -> bool {
    let pref = format!("{}.", scope);
    arrays.keys().any(|k| k.starts_with(&pref))
}

/// `resolve_type_width` with the current process/instance array shapes visible
/// to `$size`/`$left`/… in packed dimension const-eval.
pub(crate) fn resolve_type_width_with_instance_arrays(
    dt: &DataType,
    params: Option<&HashMap<String, Value>>,
    typedefs: Option<&HashMap<String, u32>>,
    scope: Option<&str>,
    arrays: &HashMap<String, (i64, i64, u32)>,
) -> u32 {
    let run = || elaborate::resolve_type_width(dt, params, typedefs);
    let Some(scope) = scope.filter(|s| !s.is_empty()) else {
        return run();
    };
    if !instance_has_scoped_array_keys(scope, arrays) {
        return run();
    }
    let view = arrays_view_for_instance_scope(scope, arrays);
    elaborate::with_arrays(&view, run)
}
