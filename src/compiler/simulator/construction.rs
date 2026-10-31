//! Simulator construction.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// Register the object-scoped dynamic or queue member `scoped` in the
    /// array tables with the shape a module-scope declaration of the same
    /// form gets: a fixed tail (`T a[][N]`, `T a[][N][M]`) makes it a 2-D or
    /// N-D array whose outer dimension is the dynamic one, so `a[i][j]`
    /// names an element instead of a bit of a scalar row (§7.4.5); any other
    /// member is a 1-D collection, rows of a dynamic tail (`T a[][]`) being
    /// collections of their own once `a[i] = new[n]` sizes them.
    pub(super) fn register_dyn_member_shape(
        &mut self,
        cdef: &crate::compiler::elaborate::ElaboratedClass,
        params: &HashMap<String, Value>,
        prop: &str,
        scoped: &str,
        width: u32,
    ) {
        let fixed_tail: Option<Vec<(i64, i64)>> =
            cdef.dynamic_tail_properties.get(prop).and_then(|tail| {
                tail.iter()
                    .map(|d| match d {
                        crate::ast::types::UnpackedDimension::Range { left, right, .. } => {
                            let l =
                                super::elaborate::const_eval_i64_with_params(left, Some(params))?;
                            let r =
                                super::elaborate::const_eval_i64_with_params(right, Some(params))?;
                            Some((l.min(r), l.max(r)))
                        }
                        crate::ast::types::UnpackedDimension::Expression { expr, .. } => {
                            match super::elaborate::const_eval_i64_with_params(expr, Some(params)) {
                                Some(n) if n > 0 => Some((0, n - 1)),
                                _ => None,
                            }
                        }
                        _ => None,
                    })
                    .collect()
            });
        match fixed_tail.as_deref() {
            Some([d2]) => {
                self.module.arrays.remove(scoped);
                self.module
                    .arrays_2d
                    .insert(scoped.to_string(), ((0, 63), *d2, width));
            }
            Some(dims) if dims.len() >= 2 => {
                self.module.arrays.remove(scoped);
                let mut shape = vec![(0, 63)];
                shape.extend_from_slice(dims);
                self.module
                    .arrays_nd
                    .insert(scoped.to_string(), (shape, width));
            }
            _ => {
                self.module
                    .arrays
                    .insert(scoped.to_string(), (0, 63, width));
            }
        }
    }

    /// Shallow copy constructor: clone `src_handle`'s instance (scalar
    /// properties plus instance-scoped queue/assoc members) into a fresh heap
    /// object. Returns the new handle.
    pub(super) fn copy_construct(&mut self, src_handle: usize) -> Value {
        let src_inst = match self.heap.get(src_handle) {
            Some(Some(i)) => i.clone(),
            _ => return Value::zero(32),
        };
        let class_name = src_inst.class_name.clone();
        let new_handle = self.heap.len();
        self.heap.push(Some(src_inst));
        // Walk the class hierarchy to clone instance-scoped collections.
        let mut classes = Vec::new();
        let mut cur = Some(class_name);
        while let Some(cn) = cur {
            if let Some(cd) = self.module.classes.get(&cn).cloned() {
                cur = cd.extends.clone();
                classes.push(cd);
            } else {
                break;
            }
        }
        for cd in &classes {
            for (prop, &(w, max)) in &cd.queue_properties {
                let s_name = format!("{}#{}", src_handle, prop);
                let n_name = format!("{}#{}", new_handle, prop);
                self.module.dynamic_arrays.insert(n_name.clone());
                if let Some(&t) = self.module.arrays_2d.get(&s_name) {
                    self.module.arrays_2d.insert(n_name.clone(), t);
                } else if let Some(t) = self.module.arrays_nd.get(&s_name).cloned() {
                    self.module.arrays_nd.insert(n_name.clone(), t);
                } else {
                    self.module.arrays.insert(n_name.clone(), (0, 63, w));
                }
                if let Some(m) = max {
                    self.module.queue_max_sizes.insert(n_name.clone(), m);
                }
                let sz = self.get_queue_size(&s_name);
                self.set_queue_size(&n_name, sz);
                for i in 0..sz {
                    if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", s_name, i)) {
                        self.set_signal_value_by_name(&format!("{}[{}]", n_name, i), v);
                    }
                }
                // §8.12/§7.4.5: rows of a dynamic outer dimension over unpacked
                // arrays (`a[i][j]`, a dynamic row's own size) are copied too,
                // each dynamic row registered under the copy.
                if cd.dynamic_tail_properties.contains_key(prop) {
                    for k in self.signals.keys_with_elem_prefix(&format!("{}[", s_name)) {
                        let Some(v) = self.signals.get(&k).cloned() else {
                            continue;
                        };
                        let nk = format!("{}{}", n_name, &k[s_name.len()..]);
                        if let Some(row) = k.strip_suffix(".size") {
                            let nrow = format!("{}{}", n_name, &row[s_name.len()..]);
                            if self.module.dynamic_arrays.contains(row) {
                                self.module.dynamic_arrays.insert(nrow.clone());
                            }
                            if let Some(&t) = self.module.arrays.get(row) {
                                self.module.arrays.insert(nrow, t);
                            }
                        }
                        self.signals.insert(nk, v);
                    }
                }
            }
            for (prop, &is_str) in &cd.assoc_properties {
                let s_pre = format!("{}#{}[", src_handle, prop);
                let n_name = format!("{}#{}", new_handle, prop);
                self.module
                    .associative_arrays
                    .insert(n_name.clone(), is_str);
                let widths = &mut self.module.assoc_index_widths;
                if let Some(&kw) = widths.get(&s_pre[..s_pre.len() - 1]) {
                    widths.insert(n_name.clone(), kw);
                }
                let entries: Vec<(String, Value)> = self
                    .signals
                    .keys_with_elem_prefix(&s_pre)
                    .into_iter()
                    .filter(|k| k.ends_with(']'))
                    .filter_map(|k| {
                        let v = self.signals.get(&k)?.clone();
                        Some((k[s_pre.len()..k.len() - 1].to_string(), v))
                    })
                    .collect();
                for (key, v) in entries {
                    self.signals.insert(format!("{}[{}]", n_name, key), v);
                }
            }
            // §8.12: a shallow copy copies every property's value — a fixed
            // array member too. Its elements live outside the instance map
            // (`<handle>#arr[i]`), so the copy had no storage at all and read
            // back 32-bit zeros.
            for prop in cd
                .array_properties
                .keys()
                .chain(cd.array_nd_properties.keys())
            {
                let s_name = format!("{}#{}", src_handle, prop);
                let n_name = format!("{}#{}", new_handle, prop);
                if let Some(&t) = self.module.arrays.get(&s_name) {
                    self.module.arrays.insert(n_name.clone(), t);
                }
                if let Some(&t) = self.module.arrays_2d.get(&s_name) {
                    self.module.arrays_2d.insert(n_name.clone(), t);
                }
                if let Some(t) = self.module.arrays_nd.get(&s_name).cloned() {
                    self.module.arrays_nd.insert(n_name.clone(), t);
                }
                for k in self.signals.keys_with_elem_prefix(&format!("{}[", s_name)) {
                    let Some(v) = self.signals.get(&k).cloned() else {
                        continue;
                    };
                    let nk = format!("{}{}", n_name, &k[s_name.len()..]);
                    if let Some(&w) = self.widths.get(&k) {
                        self.widths.insert(nk.clone(), w);
                    }
                    self.signals.insert(nk, v);
                }
            }
        }
        Value::from_u64(new_handle as u64, 32)
    }

    /// Extract the leaf identifier name from a type-argument expression
    /// (e.g. `Base` from the `#(Base)` specialization arg). Returns None for
    /// non-identifier type args (literals, complex type exprs, etc.).
    pub(super) fn leaf_ident_name(e: &Expression) -> Option<String> {
        match &e.kind {
            ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.clone()),
            // A package-scoped name (`pk::pkt_t`, `pkg::MyClass`) parses as a
            // MemberAccess, not a multi-segment Ident. Types and classes are
            // registered under their bare name, so the member IS the leaf.
            ExprKind::MemberAccess { member, .. } => Some(member.name.clone()),
            // A NAMED type connection carries its argument as a TypeLiteral
            // (`.T(int)`, `.T(logic [63:0])`). Render it to the same textual
            // form a type-parameter DEFAULT uses, so both routes name the type
            // identically.
            ExprKind::TypeLiteral(dt) => crate::elaborate::data_type_to_spec_fragment(dt),
            // A SPECIALIZED class name `C#(int)` used as a type argument.
            // Render the full `base#(args)` string (whitespace-normalized to
            // match `current_spec`/`extract_spec_from_string`), so the
            // recorded type binding can later drive a `T obj; obj = new()`
            // construction of the SPECIALIZED class — not just the base.
            ExprKind::Specialization {
                base,
                type_args_text,
            } => {
                let bn = Self::leaf_ident_name(base)?;
                Some(format!(
                    "{}#({})",
                    bn,
                    Self::normalize_spec_ws(type_args_text)
                ))
            }
            _ => None,
        }
    }

    /// True when a `#(...)` specialization argument is DEFINITELY a type
    /// name (§8.25): a bare single-segment identifier naming a built-in
    /// type keyword, a declared class, or a typedef. A literal / operator
    /// expression / call is never a type; a bare identifier that names a
    /// variable or constant is a value.
    pub(super) fn is_definite_type_arg(&self, e: &Expression) -> bool {
        // A NAMED type connection parses its builtin-type argument as a
        // TypeLiteral (`.T(int)` -> TypeLiteral), where the positional form
        // yields a bare Ident. Without this arm the literal counted as a VALUE
        // arg, so every later type arg shifted one type-parameter slot to the
        // left — `#(.T(int), .C(other))` bound `T <- other` and left `C`
        // unbound entirely.
        if matches!(&e.kind, ExprKind::TypeLiteral(_)) {
            return true;
        }
        if let ExprKind::Ident(h) = &e.kind {
            if h.path.is_empty() || h.path.iter().any(|s| !s.selects.is_empty()) {
                return false;
            }
            let n = h.path[h.path.len() - 1].name.name.as_str();
            if h.path.len() == 1 {
                return atom_type_keyword_width(n).is_some()
                    || n == "string"
                    || self.module.classes.contains_key(n)
                    || self.module.typedefs.contains_key(n)
                    || self.resolve_type_param_binding(n).is_some();
            }
            // A PACKAGE-SCOPED type name (`pk::pkt_t`) is still a type
            // argument. Only the leaf is checked, and only against declared
            // classes/typedefs — a scoped CONSTANT (`pk::WIDTH`) resolves to
            // neither and stays a value argument. Without this a scoped type
            // arg was classified as a value, so it was matched against the
            // value parameters and dropped, leaving the type parameter unbound.
            return self.is_known_type_leaf(n);
        }
        // `pk::pkt_t` parses as a MemberAccess over the package name, so the
        // multi-segment Ident arm above never sees it.
        if let ExprKind::MemberAccess { expr, member } = &e.kind {
            if matches!(&expr.kind, ExprKind::Ident(h)
                if h.path.len() == 1 && h.path[0].selects.is_empty())
            {
                return self.is_known_type_leaf(&member.name);
            }
        }
        // A SPECIALIZED class name used as a type argument — `C#(int)` in
        // `registry#(C#(int))` (`object_registry#(impl#(bitstream))`).
        // The bare Ident / MemberAccess arms above only see unspecialized names;
        // without this the specialized arg was classed as a VALUE, breaking positional
        // parameter binding — the type param went unbound and silently fell
        // back to its declared default, so a later `T obj; obj = new()` built
        // the default class instead of the specialized one.
        if let ExprKind::Specialization { base, .. } = &e.kind {
            let leaf = match &base.kind {
                ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.as_str()),
                ExprKind::MemberAccess { member, .. } => Some(member.name.as_str()),
                _ => None,
            };
            if let Some(n) = leaf {
                return self.is_known_type_leaf(n);
            }
        }
        false
    }

    /// True when `n` names a declared class or typedef — the leaf test shared
    /// by the scoped-type-argument arms of `is_definite_type_arg`.
    pub(super) fn is_known_type_leaf(&self, n: &str) -> bool {
        self.module.classes.contains_key(n)
            || self.module.typedefs.contains_key(n)
            || self.module.typedef_types.contains_key(n)
    }

    /// Map a specialization's `#(...)` argument list onto the class's
    /// parameters by NAME (§8.25). Positional lists map by slot in
    /// `param_order` (type and value parameters interleave in declaration
    /// order). A named connection (`.W(32)`) arrives as a `NamedArg` and
    /// binds by its name. A list whose names were dropped (a cache written
    /// before the parser kept them) is recovered structurally: A.4.1.1 requires
    /// a list to be all-named or all-positional, so if any slot is
    /// kind-inconsistent (a value expression on a TYPE-parameter slot, or a
    /// definite type name on a value-parameter slot) the list must have been
    /// named — pair type-name args with TYPE params and value exprs with
    /// VALUE params, each in declaration order.
    pub(super) fn class_param_arg_map(
        &self,
        class_def: &crate::compiler::elaborate::ElaboratedClass,
        ta: &[Expression],
    ) -> HashMap<String, Expression> {
        // `param_order` is serde-defaulted: a compiled module cached by an
        // older xezim has it empty. Reconstruct the legacy assumption
        // (value params first, type params after) so those caches keep
        // binding the way they used to.
        let legacy_order: Vec<String>;
        let order: &[String] = if class_def.param_order.is_empty()
            && !(class_def.param_defaults.is_empty() && class_def.type_param_names.is_empty())
        {
            legacy_order = class_def
                .param_defaults
                .iter()
                .map(|(n, _)| n.clone())
                .chain(class_def.type_param_names.iter().cloned())
                .collect();
            &legacy_order
        } else {
            &class_def.param_order
        };
        // §8.25 named assignments (`#(.W(5))`) carry their parameter's name:
        // bind each by that name, whatever its position.
        if ta
            .iter()
            .any(|a| matches!(a.kind, ExprKind::NamedArg { .. }))
        {
            let mut map: HashMap<String, Expression> = HashMap::default();
            for (i, arg) in ta.iter().enumerate() {
                match &arg.kind {
                    ExprKind::NamedArg { name, expr } => {
                        if let Some(x) = expr {
                            if order.iter().any(|p| *p == name.name) {
                                map.insert(name.name.clone(), (**x).clone());
                            }
                        }
                    }
                    _ => {
                        if let Some(p) = order.get(i) {
                            map.insert(p.clone(), arg.clone());
                        }
                    }
                }
            }
            return map;
        }
        let is_type_param = |n: &str| class_def.type_param_names.iter().any(|t| t == n);
        // Kind-consistency scan: positional unless some slot mismatches.
        let mut positional = true;
        for (i, arg) in ta.iter().enumerate() {
            let Some(pname) = order.get(i) else { break };
            if is_type_param(pname) {
                // A type arg is always a bare identifier (possibly a class
                // not yet checked as definite); anything else is a value.
                // A package-scoped name counts too, but only when it really
                // resolves to a type — `pk::pkt_t` is a type argument while
                // `pk::WIDTH` is a value.
                let ident_ok = matches!(&arg.kind, ExprKind::Ident(h)
                    if h.path.len() == 1 && h.path[0].selects.is_empty())
                    || self.is_definite_type_arg(arg);
                if !ident_ok {
                    positional = false;
                    break;
                }
            } else if self.is_definite_type_arg(arg) {
                positional = false;
                break;
            }
        }
        let mut map: HashMap<String, Expression> = HashMap::default();
        if positional {
            for (i, arg) in ta.iter().enumerate() {
                if let Some(pname) = order.get(i) {
                    map.insert(pname.clone(), arg.clone());
                }
            }
        } else {
            let type_params: Vec<&String> = order.iter().filter(|n| is_type_param(n)).collect();
            let value_params: Vec<&String> = order.iter().filter(|n| !is_type_param(n)).collect();
            let (mut ti, mut vi) = (0usize, 0usize);
            for arg in ta {
                if self.is_definite_type_arg(arg) {
                    if let Some(p) = type_params.get(ti) {
                        map.insert((*p).clone(), arg.clone());
                        ti += 1;
                    }
                } else if let Some(p) = value_params.get(vi) {
                    map.insert((*p).clone(), arg.clone());
                    vi += 1;
                }
            }
        }
        map
    }

    /// Resolve a class TYPE-parameter name through the current `this`
    /// instance's recorded bindings. Returns the concrete class name if `tn`
    /// is a type parameter bound on the active instance (e.g. `T -> Base`
    /// inside a `Mk#(Base)` method), else None (so a plain class name passes
    /// through unchanged at the call site).
    pub(super) fn resolve_type_param_binding(&self, tn: &str) -> Option<String> {
        self.resolve_type_param_with(tn, &self.current_spec)
    }

    /// Resolve a class VALUE parameter `name` from the active specialization
    /// (`current_spec`). This is the value-param counterpart of
    /// [`resolve_type_param_with`] (which handles TYPE params): a static
    /// method such as `registry#(T,"base_class")::get_type_name()`
    /// references the string value parameter `Tname`, but no instance is
    /// constructed for a static call, so the binding only exists in the
    /// specialization's argument list. Without this, the bare identifier fell
    /// through to the class default value (e.g. `<unknown>`) instead of the
    /// specialized value (`base_class`), so the factory's type-name lookup and
    /// `is_type_registered` always failed.
    ///
    /// Returns None when no specialization is active or `name` is not a value
    /// parameter (type parameters are handled by `resolve_type_param_with`).
    /// `module.parameters` overlaid with the ACTIVE class specialization's
    /// value parameters, or `None` when no class spec is active (the caller
    /// then uses `module.parameters` directly, so the common path is free).
    /// If `dt` names a TYPE PARAMETER of the active class specialization and
    /// that parameter is using its DEFAULT, return the default's declared
    /// type so the caller can size it against the current value parameters.
    /// Returns None for an explicitly-bound type param (the binding is
    /// already a concrete type name) or outside a class.
    pub(super) fn class_type_param_default(
        &self,
        dt: &crate::ast::types::DataType,
    ) -> Option<crate::ast::types::DataType> {
        use crate::ast::types::DataType;
        let DataType::TypeReference { name, .. } = dt else {
            return None;
        };
        let tn = name.name.name.as_str();
        let (base, sig) = self.current_spec.as_ref()?;
        let cd = self.module.classes.get(base)?;
        if !cd.type_param_names.iter().any(|t| t == tn) {
            return None;
        }
        // An EXPLICIT binding wins — decided by slot in the specialization's
        // argument list, not by `resolve_type_param_binding` (which falls back
        // to the default fragment and so cannot tell the two apart). Only a
        // type param left at its default is resolved from the default AST.
        if let Some(idx) = cd.param_order.iter().position(|p| p == tn) {
            let frags = Self::split_spec_args(sig);
            if let Some(f) = frags.get(idx).map(|f| f.trim()).filter(|f| !f.is_empty()) {
                // An unspecified type param is filled in with its own default
                // FRAGMENT, which is lossy for a parameterized type (`logic
                // [W-1:0]` renders as `logic`). Matching that fragment means
                // the default is in force, so prefer the exact default AST;
                // anything else is an explicit binding and wins.
                let is_default_frag = cd
                    .type_param_defaults
                    .iter()
                    .any(|(n, d)| n == tn && d.trim() == f);
                if !is_default_frag {
                    return None;
                }
            }
        }
        cd.type_param_default_types
            .iter()
            .find(|(n, _)| n == tn)
            .map(|(_, d)| d.clone())
    }

    pub(super) fn params_with_class_spec(&mut self) -> Option<HashMap<String, Value>> {
        let (base, _) = self.current_spec.clone()?;
        let names: Vec<String> = {
            let cd = self.module.classes.get(&base)?;
            cd.param_order
                .iter()
                .filter(|n| !cd.type_param_names.iter().any(|t| t == *n))
                .cloned()
                .collect()
        };
        if names.is_empty() {
            return None;
        }
        let mut m = self.module.parameters.clone();
        for n in names {
            if let Some(v) = self.resolve_value_param_from_spec(&n) {
                m.insert(n, v);
            }
        }
        Some(m)
    }

    /// Resolve a value parameter `name` against the ACTIVE specialization,
    /// and if it is not declared there, against each ENCLOSING caller
    /// specialization on `spec_scope_stack` (`eval_call` pushes the caller's
    /// spec when it enters a parameterized `C#(params)::method`). §13.5.1: an
    /// actual argument is evaluated in the CALLER's scope, so a value
    /// parameter of an enclosing specialization (e.g. `string FIELD` of
    /// `uvm_utils` passed into `uvm_config_db#(uvm_object)::get`) must
    /// resolve to its bound value even though `current_spec` is the callee
    /// while the args are bound.
    pub(super) fn resolve_value_param_from_spec(&mut self, name: &str) -> Option<Value> {
        if let Some(v) = self.resolve_value_param_in_current(name) {
            return Some(v);
        }
        // §8.25/§6.20.2: a STATIC method of a class that is not itself
        // parameterized but extends a specialization (`class d extends
        // holder #(int, "ext", 9)`) has no instance and no active
        // specialization; a parameter it inherits takes the value that
        // extends clause binds, not the base's default.
        if self.current_spec.is_none()
            && name
                .as_bytes()
                .first()
                .is_some_and(|b| self.class_member_names().param_first[*b as usize])
            && self.class_member_names().params.contains(name)
            && let Some(Some(ctx)) = self.class_context_stack.last()
            && let Some(spec) = self.inherited_param_spec(&ctx.clone())
        {
            self.current_spec = Some(spec);
            let v = self.resolve_value_param_in_current(name);
            self.current_spec = None;
            if v.is_some() {
                return v;
            }
        }
        // Not declared by the leaf/callee spec — walk the caller chain,
        // innermost first (nearest caller wins).
        let chain: Vec<(String, String)> = self.spec_scope_stack.clone();
        for cs in chain.iter().rev() {
            let saved = self.current_spec.clone();
            self.current_spec = Some(cs.clone());
            let v = self.resolve_value_param_in_current(name);
            self.current_spec = saved;
            if v.is_some() {
                return v;
            }
        }
        None
    }

    pub(super) fn resolve_value_param_in_current(&mut self, name: &str) -> Option<Value> {
        // Cycle guard: a value param's specialization argument can itself
        // be a bare name that resolves back into value-param resolution for
        // the same class/specialization (e.g. class factories, where a
        // registry class specializes its base by passing its own `Tname`
        // param through, kept symbolic through parameterized-inheritance
        // chains). Without a guard the resolution recurses infinitely and
        // blows the stack. A cyclic value param cannot be concretely resolved
        // at this site — fall back to its declared default.
        if VALPARAM_RESOLVING.with(|r| r.borrow().iter().any(|n| n == name)) {
            return None;
        }
        VALPARAM_RESOLVING.with(|r| r.borrow_mut().push(name.to_string()));
        let _guard = ValparamResolveGuard;
        let (base, sig) = self.current_spec.clone()?;
        let (idx, local_init, first_parent) = {
            let cd = self.module.classes.get(&base)?;
            // Type parameters are resolved elsewhere — don't shadow them.
            if cd.type_param_names.iter().any(|t| t == name) {
                return None;
            }
            // Match `param_order` exactly like `class_param_arg_map` (with the
            // same legacy fallback for older caches lacking `param_order`).
            let idx = if cd.param_order.is_empty()
                && !(cd.param_defaults.is_empty() && cd.type_param_names.is_empty())
            {
                cd.param_defaults
                    .iter()
                    .map(|(n, _)| n)
                    .chain(cd.type_param_names.iter())
                    .position(|p| p == name)
            } else {
                cd.param_order.iter().position(|p| p == name)
            };
            let local_init = if idx.is_none() {
                cd.param_defaults
                    .iter()
                    .find(|(n, _)| n == name)
                    .and_then(|(_, e)| e.clone())
            } else {
                None
            };
            (idx, local_init, cd.extends.clone())
        };
        let Some(idx) = idx else {
            // Not a positional class parameter — but a class-BODY localparam
            // is a class constant that may be sized from the parameters
            // (`localparam int MYW = W;`). Its property value was baked at
            // elaboration from the DEFAULTS, so every specialization read the
            // default. Re-evaluate the declared initializer with the active
            // specialization bound; the cycle guard above covers a
            // self-referential initializer.
            if let Some(init) = local_init {
                return Some(self.eval_expr(&init));
            }
            // Declared by an ANCESTOR: `return W;` in a method inherited
            // from `Base#(N*3)` runs with the LEAF spec active, and W is
            // not a leaf parameter — compute the ancestor's specialization
            // from the leaf bindings (ancestor_spec evaluates expression
            // extends-args) and read the argument there. Without this,
            // every inherited parameter read fell back to its default.
            let mut cur = first_parent;
            while let Some(anc) = cur {
                let Some((declares, ancestor_idx, ancestor_init, next_parent)) =
                    self.module.classes.get(&anc).map(|acd| {
                        (
                            acd.param_order.iter().any(|p| p == name)
                                || acd.param_defaults.iter().any(|(n, _)| n == name),
                            acd.param_order.iter().position(|p| p == name),
                            acd.param_defaults
                                .iter()
                                .find(|(n, _)| n == name)
                                .and_then(|(_, e)| e.clone()),
                            acd.extends.clone(),
                        )
                    })
                else {
                    break;
                };
                if declares {
                    let Some(asig) = self.ancestor_spec(&base, &sig, &anc) else {
                        break;
                    };
                    if let Some(aidx) = ancestor_idx {
                        let frags = Self::split_spec_args(&asig);
                        if let Some(frag) = frags.get(aidx).cloned() {
                            let saved = self.current_spec.clone();
                            self.current_spec = Some((anc.clone(), asig.clone()));
                            let v = self.eval_spec_arg_fragment(&frag);
                            self.current_spec = saved;
                            if v.is_some() {
                                return v;
                            }
                        }
                    }
                    if let Some(init) = ancestor_init {
                        let saved = self.current_spec.clone();
                        self.current_spec = Some((anc.clone(), asig));
                        let v = self.eval_expr(&init);
                        self.current_spec = saved;
                        return Some(v);
                    }
                    break;
                }
                cur = next_parent;
            }
            return None;
        };
        // Split the specialization's raw arg text on TOP-LEVEL commas (a naive
        // split would break on commas inside a string literal or a nested
        // call), then evaluate the fragment at `name`'s position.
        let frags = Self::split_spec_args(&sig);
        // A slot spelled as the parameter's own name is one a named list
        // left at its default (see `canonicalize_spec_sig`).
        if let Some(frag) = frags.get(idx).filter(|f| f.trim() != name) {
            return self.eval_spec_arg_fragment(frag);
        }
        // §6.20.2: the specialization leaves `name` at a default that could
        // not be folded into its signature (`int W = $bits(U)`) — evaluate
        // the default here, with this specialization active, so it follows
        // the parameters before it.
        let init = self
            .module
            .classes
            .get(&base)?
            .param_defaults
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, e)| e.clone())?;
        // The class's scope only: no caller local or `this` member of the
        // same name may answer for an earlier parameter.
        let frame = self.take_pooled_frame();
        self.class_context_stack.push(Some(base.clone()));
        self.this_stack.push(None);
        self.push_local_frame(frame);
        let v = self.eval_expr(&init);
        self.pop_local_frame();
        self.this_stack.pop();
        self.class_context_stack.pop();
        Some(v)
    }

    /// Fragment `idx` of `split_spec_args(sig)`, borrowed: the same
    /// top-level split, without building the list.
    pub(super) fn spec_arg_at(sig: &str, idx: usize) -> Option<&str> {
        let mut depth: i32 = 0;
        let mut in_str = false;
        let mut prev = '\0';
        let mut start = 0usize;
        let mut k = 0usize;
        for (i, ch) in sig.char_indices() {
            match (in_str, ch) {
                (false, '"') => in_str = true,
                (true, '"') if prev != '\\' => in_str = false,
                (true, _) => {}
                (false, '(' | '[' | '{') => depth += 1,
                (false, ')' | ']' | '}') => depth -= 1,
                (false, ',') if depth == 0 => {
                    if k == idx {
                        return Some(&sig[start..i]);
                    }
                    k += 1;
                    start = i + 1;
                }
                (false, _) => {}
            }
            prev = ch;
        }
        // The last fragment is listed only when it is not blank.
        let last = &sig[start..];
        (k == idx && !last.trim().is_empty()).then_some(last)
    }

    /// Split a specialization argument-list text on top-level commas,
    /// respecting string literals, parentheses, brackets and braces so that
    /// `#("a,b", f(1,2))` yields two fragments, not four.
    pub(super) fn split_spec_args(sig: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut depth: i32 = 0;
        let mut in_str = false;
        let mut prev = '\0';
        for ch in sig.chars() {
            match (in_str, ch) {
                (false, '"') => {
                    in_str = true;
                    cur.push(ch);
                }
                (true, '"') if prev != '\\' => {
                    in_str = false;
                    cur.push(ch);
                }
                (true, _) => {
                    cur.push(ch);
                }
                (false, '(' | '[' | '{') => {
                    depth += 1;
                    cur.push(ch);
                }
                (false, ')' | ']' | '}') => {
                    depth -= 1;
                    cur.push(ch);
                }
                (false, ',') if depth == 0 => {
                    out.push(std::mem::take(&mut cur));
                }
                (false, _) => {
                    cur.push(ch);
                }
            }
            prev = ch;
        }
        if !cur.trim().is_empty() {
            out.push(cur);
        }
        out
    }

    /// Evaluate a single specialization argument fragment to a `Value`.
    /// Handles the shapes that appear in value-parameter positions: string
    /// literals (factory type names like `"base_class"`), numeric
    /// literals, and bare name references (enum members / parameters), the
    /// last by constructing an identifier expression and deferring to the
    /// normal evaluator.
    pub(super) fn eval_spec_arg_fragment(&mut self, frag: &str) -> Option<Value> {
        let t = frag.trim();
        if t.is_empty() {
            return None;
        }
        let bytes = t.as_bytes();
        // String literal "..."
        if bytes[0] == b'"' && bytes.len() >= 2 && bytes[bytes.len() - 1] == b'"' {
            let inner = std::str::from_utf8(&bytes[1..bytes.len() - 1]).ok()?;
            let w = (inner.len() * 8).max(8) as u32;
            let mut val = Value::zero(w);
            for (i, byte) in inner.bytes().rev().enumerate() {
                for bit in 0..8 {
                    if (byte >> bit) & 1 == 1 && i * 8 + bit < val.width as usize {
                        val.set_bit(i * 8 + bit, LogicBit::One);
                    }
                }
            }
            return Some(val);
        }
        // Numeric literal (plain decimal or sized `'h/'d/'b/'o). Build a
        // `NumberLiteral` from the text and evaluate via `eval_number`.
        if let Some(nl) = Self::parse_spec_number(t) {
            return Some(self.eval_number(&nl));
        }
        // Bare name reference — delegate to the normal identifier evaluator
        // (resolves enum members, parameters, static constants).
        let expr = Expression::new(
            ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                root: None,
                path: vec![crate::ast::expr::HierPathSegment {
                    name: crate::ast::Identifier {
                        name: t.to_string(),
                        span: crate::ast::Span::dummy(),
                    },
                    selects: Vec::new(),
                }],
                span: crate::ast::Span::dummy(),
                cached_signal_id: std::cell::Cell::new(None),
                cached_resolved_name: std::cell::OnceCell::new(),
            }),
            crate::ast::Span::dummy(),
        );
        Some(self.eval_expr(&expr))
    }

    /// Parse a specialization-argument fragment into a `NumberLiteral` for the
    /// shapes that appear in value-parameter positions: plain decimal
    /// integers (`42`, `-5`) and optionally-sized based literals
    /// (`'h10`, `8'd3`, `16'hFF`). Returns None for anything else.
    pub(super) fn parse_spec_number(t: &str) -> Option<NumberLiteral> {
        let s = t.trim();
        // Optionally-sized based literal: `[size]'<base><digits>`.
        if let Some(pos) = s.find('\'') {
            let size_part = &s[..pos];
            let rest = &s[pos + 1..];
            let (base, val_str) = match rest.chars().next() {
                Some('b') => (NumberBase::Binary, &rest[1..]),
                Some('d') => (NumberBase::Decimal, &rest[1..]),
                Some('h') => (NumberBase::Hex, &rest[1..]),
                Some('o') => (NumberBase::Octal, &rest[1..]),
                _ => return None,
            };
            let size = if size_part.is_empty() {
                None
            } else {
                size_part.parse::<u32>().ok()
            };
            return Some(NumberLiteral::Integer {
                size,
                signed: false,
                base,
                value: val_str.trim().to_string(),
                cached_val: std::cell::Cell::new(None),
            });
        }
        // Plain decimal integer (optional leading sign).
        let neg = s.starts_with('-');
        let body = s.trim_start_matches(['+', '-']);
        if !body.is_empty() && body.bytes().all(|c| c.is_ascii_digit()) {
            return Some(NumberLiteral::Integer {
                size: None,
                signed: neg,
                base: NumberBase::Decimal,
                value: body.to_string(),
                cached_val: std::cell::Cell::new(None),
            });
        }
        None
    }

    /// Resolve a type-parameter name `tn` to its concrete type, using (in order)
    /// the current object's bindings (instance context) then a given active
    /// `spec = (base, sig)` (static context — `tn`'s position in `base`'s
    /// type_param_names picks the matching comma element of `sig`). E.g.
    /// `resource#(T)` where `T` is `config_db#(int)`'s own param → `int`.
    pub(super) fn resolve_type_param_with(
        &self,
        tn: &str,
        spec: &Option<(String, String)>,
    ) -> Option<String> {
        // Every answer below comes from `tn`'s position in some class's
        // parameter list (instance bindings are keyed by type-parameter
        // names too), so a name that no class declares as a parameter
        // resolves to nothing.
        if !self.class_member_names().params.contains(tn) {
            return None;
        }
        // When the ACTIVE specialization DIRECTLY declares `tn`, its sig
        // argument is authoritative over an instance's cached binding. The
        // cached `this.type_bindings` can be stale for a parameterized type
        // built recursively: constructing `uvm_resource#(T)` inside
        // `uvm_config_db_default_implementation_t#(T=uvm_bitstream_t)` came up
        // with the OUTER instance bound as `T ->
        // uvm_config_db_default_implementation_t#(uvm_bitstream_t)` (the
        // full specialization) instead of `T -> uvm_bitstream_t`, so the
        // child `uvm_resource` recorded the impl type and a `$cast` to
        // `uvm_resource#(uvm_bitstream_t)` failed (any UVM_agent's `is_active`
        // config read). Prefer the exact type argument of the active spec.
        if let Some((base, sig)) = spec {
            if let Some(cd) = self.get_class_def(base) {
                // The sig fragment index for an INTERLEAVED type/value param
                // list (`Mix #(int A, type T, int B)`) comes from the full
                // `param_order`, not `type_param_names` (which would wrong-
                // index `T` onto the `A` slot).
                let order: &[String] = if cd.param_order.is_empty() {
                    &cd.type_param_names
                } else {
                    &cd.param_order
                };
                if let Some(idx) = order.iter().position(|p| p == tn) {
                    // `param_order` interleaves value parameters so a TYPE
                    // parameter's fragment index is right. The name itself
                    // still has to be a type parameter. A value parameter's
                    // fragment is a number (`H` -> `32` on `my_seq #(32, 194)`).
                    // Returning it made `is_definite_type_arg` treat
                    // `item #(H, D)` as a type-argument list, the values were
                    // dropped, and `new` built the default specialization.
                    if !cd.type_param_names.iter().any(|t| t == tn) {
                        return None;
                    }
                    if let Some(v) = Self::spec_arg_at(sig, idx) {
                        let v = v.trim();
                        if !v.is_empty() {
                            return Some(v.to_string());
                        }
                    }
                }
            }
        }
        if let Some(h) = self.this_stack.last().copied().flatten() {
            if let Some(c) = self
                .heap
                .get(h)
                .and_then(|o| o.as_ref())
                .and_then(|inst| inst.type_bindings.get(tn).cloned())
            {
                return Some(c);
            }
        }
        if let Some((base, sig)) = spec {
            // The specialization's own class entry: its `extends` is the
            // bound base when that is a type parameter (§8.25).
            let entry = self.spec_entry_cow(base, sig);
            if let Some(cd) = self.get_class_def(&entry) {
                if let Some(idx) = cd.type_param_names.iter().position(|p| p == tn) {
                    if let Some(v) = Self::spec_arg_at(sig, idx) {
                        let v = v.trim();
                        if !v.is_empty() {
                            return Some(v.to_string());
                        }
                    }
                    // §8.25.4 trailing-unbound fallback: the active
                    // specialization omitted this type param (e.g.
                    // `callbacks#(T)` elides the defaulted `CB`), so the
                    // sig fragment is missing. Use the param's declared
                    // default (e.g. `CB = base_callback`). Without this, an
                    // in-body `typeid#(CB)::get()` resolves `CB` to the
                    // bare literal instead of `base_callback`, flipping the
                    // `get()` if/else and breaking the base-callback `typeid_map` write.
                    if let Some((_, default)) = cd.type_param_defaults.iter().find(|(n, _)| n == tn)
                    {
                        let d = default.trim();
                        if !d.is_empty() {
                            return Some(d.to_string());
                        }
                    }
                }
                // §8.25: `tn` is a type parameter declared in an ANCESTOR of
                // the spec's base class (e.g. `Tregistry` in
                // `registry_common`, inherited by `object_registry`).
                // Walk the extends chain to find which ancestor declares `tn`,
                // then resolve the corresponding type arg from the extends
                // clause. The arg is often `this_type` (a typedef for the
                // current specialization) or another type param of `base`.

                let mut cur_extends = cd.extends.clone();
                let mut cur_type_args = cd.extends_type_args.clone();
                let mut level = 0;
                while let Some(ancestor) = cur_extends {
                    level += 1;
                    if level > 16 {
                        break;
                    }
                    let acd = match self.module.classes.get(&ancestor) {
                        Some(c) => c,
                        None => break,
                    };
                    // Find `tn` in the ancestor's full param_order (type +
                    // value params interleaved). `extends_type_args` stores
                    // ALL args positionally, so we index by param_order.
                    if let Some(idx) = acd.param_order.iter().position(|p| p == tn) {
                        // A projection `BASE#[i]` depends on the binding one
                        // level down: the ancestor-spec path below derives it.
                        if let Some(arg) = cur_type_args
                            .get(idx)
                            .filter(|a| Self::spec_projection(a).is_none())
                        {
                            let arg = arg.trim();
                            if arg == "this_type" || arg == "this" {
                                return Some(format!("{}#({})", base, sig));
                            }
                            // Check if arg is a type param of the ORIGINAL
                            // spec base class
                            if let Some(bidx) = cd.type_param_names.iter().position(|p| p == arg) {
                                let sig_frags = Self::split_spec_args(sig);
                                if let Some(v) = sig_frags.get(bidx) {
                                    let v = v.trim();
                                    if !v.is_empty() {
                                        return Some(v.to_string());
                                    }
                                }
                            }
                            // Check if arg is a value param of the base class
                            // (resolve from the sig by position in param_order)
                            if let Some(bidx) = cd.param_order.iter().position(|p| p == arg) {
                                let sig_frags = Self::split_spec_args(sig);
                                if let Some(v) = sig_frags.get(bidx) {
                                    let v = v.trim();
                                    if !v.is_empty() {
                                        return Some(v.to_string());
                                    }
                                }
                            }
                            // Otherwise return the arg verbatim (a concrete
                            // class name like `uvm_registry_object_creator`)
                            if arg != "<unknown>" {
                                return Some(arg.to_string());
                            }
                        } else {
                            // §6.20.3: the extends clause omits `tn`, so the
                            // ancestor's specialization uses its DECLARED
                            // DEFAULT there — `class d #(int X) extends
                            // pbase;` still sees `pbase`'s `T = int`. The
                            // default may follow an earlier parameter
                            // (`type U = T`), so derive the ancestor's whole
                            // specialization and read `tn`'s slot from it.
                            if let Some(asig) = self.ancestor_spec(base, sig, &ancestor) {
                                let asig = self.canonicalize_spec_sig(&ancestor, &asig);
                                if let Some(v) = Self::spec_arg_at(&asig, idx) {
                                    let v = v.trim();
                                    if !v.is_empty() {
                                        return Some(v.to_string());
                                    }
                                }
                            }
                        }
                        break;
                    }
                    cur_extends = acd.extends.clone();
                    cur_type_args = acd.extends_type_args.clone();
                }
            }
        }
        self.inherited_type_param(tn, spec)
    }

    /// §8.25, §6.20.2: `tn` is a type parameter of an ANCESTOR of the
    /// executing method's class, bound by the `extends` clauses between them
    /// (`class d extends drv #(item)` naming `REQ`, or the defaulted
    /// `RSP = REQ`). A class that is not itself parameterized has no
    /// specialization and its objects carry no bindings, so `RSP::type_id::
    /// create()`, `REQ x = new` and `r = new` in such a class resolved `tn`
    /// to nothing and built nothing. The ancestor's specialization as the
    /// method's class binds it answers; an argument that is one of the
    /// method's class's own type parameters resolves through `spec` / `this`.
    #[cold]
    #[inline(never)]
    pub(super) fn inherited_type_param(
        &self,
        tn: &str,
        spec: &Option<(String, String)>,
    ) -> Option<String> {
        let ctx = self.class_context_stack.last()?.as_deref()?;
        let ctx_cd = self.module.classes.get(ctx)?;
        if ctx_cd.type_param_names.iter().any(|t| t == tn) {
            return None;
        }
        let mut decl: Option<&str> = None;
        let mut cur: Option<&str> = ctx_cd.extends.as_deref();
        let mut guard = 0;
        while let Some(cn) = cur {
            guard += 1;
            if guard > 64 {
                return None;
            }
            let cd = self.module.classes.get(cn)?;
            if cd.type_param_names.iter().any(|t| t == tn) {
                decl = Some(cn);
                break;
            }
            if cd.typedef_names.iter().any(|t| t == tn) {
                return None;
            }
            cur = cd.extends.as_deref();
        }
        let (base, sig) = self.static_receiver_spec(ctx, decl?)?;
        let cd = self.module.classes.get(&base)?;
        let order: &[String] = if cd.param_order.is_empty() {
            &cd.type_param_names
        } else {
            &cd.param_order
        };
        let idx = order.iter().position(|p| p == tn)?;
        let v = Self::spec_arg_at(&sig, idx)?.trim();
        if v.is_empty() {
            return None;
        }
        if ctx_cd.type_param_names.iter().any(|t| t == v) {
            return self.resolve_type_param_with(v, spec);
        }
        Some(v.to_string())
    }

    /// Resolve any type-parameter names inside an extracted call spec's sig
    /// through the ENCLOSING spec, so `resource#(T)::get_type()` invoked
    /// from inside `config_db#(int)` keys `resource#(int)` — the same
    /// per-spec cell the concrete resource uses.
    /// §8.25: the specialization against which a `C#(T)` written in the
    /// running code resolves its parameter names — the lexical class's. While
    /// a specialized static call's actuals are evaluated, `current_spec` is
    /// already the CALLEE's (`eval_call`), so `registry#(B)::add(seq#(T)::cb)`
    /// in `drv#(byte)` read `T` as `registry`'s parameter; the caller's spec
    /// is the nearest one on `spec_scope_stack` that applies to the lexical
    /// class.
    pub(super) fn lexical_spec(&self) -> Option<(String, String)> {
        let Some(Some(lex)) = self.class_context_stack.last() else {
            return self.current_spec.clone();
        };
        let applies = |(b, _): &(String, String)| b == lex || self.class_is_a(lex, b);
        if self.current_spec.as_ref().is_none_or(|c| applies(c)) {
            return self.current_spec.clone();
        }
        self.spec_scope_stack
            .iter()
            .rev()
            .find(|s| applies(s))
            .cloned()
            .or_else(|| self.current_spec.clone())
    }

    pub(super) fn resolve_call_spec_params(
        &self,
        spec: Option<(String, String)>,
        enclosing: &Option<(String, String)>,
    ) -> Option<(String, String)> {
        let (base, sig) = spec?;
        let resolved = Self::split_spec_args(&sig)
            .iter()
            .map(|part| self.resolve_spec_arg(part, enclosing))
            .collect::<Vec<_>>()
            .join(",");
        let canon_sig = self.canonicalize_spec_sig(&base, &resolved);
        Some((base, canon_sig))
    }

    /// One argument of a static call's specialization with the enclosing
    /// class's parameters substituted; recurses into named (`.N(v)`) and
    /// nested (`C#(...)`) arguments.
    pub(super) fn resolve_spec_arg(
        &self,
        part: &str,
        enclosing: &Option<(String, String)>,
    ) -> String {
        let p = part.trim();
        if let Some((n, v)) = Self::named_spec_arg(p) {
            return format!(".{}({})", n, self.resolve_spec_arg(v, enclosing));
        }
        if let Some(r) = self.resolve_type_param_with(p, enclosing) {
            return Self::normalize_spec_ws(&r);
        }
        // Top-level VALUE parameter of the enclosing class — resolve
        // to the enclosing specialization's argument at that param's
        // position. E.g. inside `Mid#(int,"hello")`, the static call
        // `Inner#(Name)::get()` must specialize Inner with "hello",
        // not the symbolic bare name `Name` (which, left symbolic,
        // loops forever in `resolve_value_param_from_spec` when
        // Inner's static method reads `Name`, or falls back to the
        // wrong default). Mirrors the value-param substitution the
        // nested-`Class#(args)` branch below already performs.
        if let Some((eb, es)) = enclosing {
            if let Some(ecd) = self.module.classes.get(eb) {
                if let Some(idx) = ecd.param_order.iter().position(|x| x == p) {
                    let frags = Self::split_spec_args(es);
                    if let Some(v) = frags.get(idx) {
                        return v.trim().to_string();
                    }
                }
            }
        }
        // Resolve a class-local TYPEDEF name (e.g. `special_comp_type`,
        // a `typedef special_comp#(N) special_comp_type;` inside
        // `special_comp#(N)`) to its concrete specialization through
        // the enclosing spec. Without this, a static initializer
        // (`register_cb(special_comp_type, special_cb_type)` →
        // `callbacks#(special_comp_type,...)::m_register_pair`)
        // keys the typeid under the bare typedef name, while the
        // matching `add`/query uses the explicit `special_comp#(1)`
        // form — the two never meet and typewide callbacks
        // cross-propagate across value specializations.
        // `resolve_typedef_spec` reads `self.current_spec`, which the
        // caller (`eval_expr_ctx`) leaves set to `enclosing` for
        // this purpose.
        if let Some((tb, ts)) = self.resolve_typedef_spec(p) {
            let ts = Self::normalize_spec_ws(&ts);
            return format!("{}#({})", tb, ts);
        }
        // Resolve a `Class#(args)` part whose args contain bare
        // type/value-parameter names of the enclosing class — e.g.
        // `special_comp#(N)` inside `special_comp#(N)`'s own
        // `run_phase`, where callback macro expands to
        // `callbacks#(special_comp#(N),CB)::get_first`. The
        // literal `N` (a value param) must resolve to the enclosing
        // specialization's value (1 or 2), else every instance's
        // do_callbacks keys the SAME `special_comp#(N)` cell —
        // distinct from the `#(1)`/`#(2)` cells that `add` and
        // `m_register_pair` populate.
        if let Some((cb, cs)) = self.extract_spec_from_string(p) {
            let r_sig = Self::split_spec_args(&cs)
                .iter()
                .map(|frag| self.resolve_spec_arg(frag, enclosing))
                .collect::<Vec<_>>()
                .join(",");
            let r_sig = self.canonicalize_spec_sig(&cb, &r_sig);
            return format!("{}#({})", cb, r_sig);
        }
        // §25.9: a virtual-interface type argument (`virtual I#(T)`), written
        // out or named through a typedef (`vif_t`): its own arguments may
        // name the enclosing class's parameters, so `cfg_db#(virtual
        // my_if#(T))` inside `mon#(int)` is `cfg_db#(virtual my_if#(int))`
        // (issue #320). Left symbolic, every specialization of the enclosing
        // class shared one `#(T)` table and none met the module's.
        let aliased = self.typedef_alias_frag(p);
        let p = aliased.as_deref().unwrap_or(p);
        if let Some((name, args, modport)) = self.parse_vif_frag(p) {
            let args = args
                .iter()
                .map(|a| self.resolve_spec_arg(a, enclosing))
                .collect();
            return self.canonical_vif_frag(&name, args, modport);
        }
        Self::normalize_spec_ws(p)
    }

    /// Split a virtual-interface type fragment — `virtual [interface]
    /// I[#(args)][.modport]` in any spacing, including the whitespace-free
    /// canonical form — into the interface name, its raw `#(...)` arguments
    /// and the modport. None for anything else.
    pub(super) fn parse_vif_frag(
        &self,
        frag: &str,
    ) -> Option<(String, Vec<String>, Option<String>)> {
        if self.module.interfaces.is_empty() {
            return None;
        }
        let rest = frag.trim().strip_prefix("virtual")?;
        let head_end = rest.find(['#', '.']).unwrap_or(rest.len());
        let head: String = rest[..head_end]
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let name = if self.module.interfaces.contains(&head) {
            head
        } else {
            let h = head.strip_prefix("interface")?;
            if !self.module.interfaces.contains(h) {
                return None;
            }
            h.to_string()
        };
        let mut tail = rest[head_end..].trim_start();
        let mut args = Vec::new();
        if let Some(t) = tail.strip_prefix('#') {
            let t = t.trim_start().strip_prefix('(')?;
            let mut depth = 1i32;
            let mut close = None;
            for (i, ch) in t.char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            close = Some(i);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let close = close?;
            args = Self::split_spec_args(&t[..close])
                .into_iter()
                .map(|a| a.trim().to_string())
                .filter(|a| !a.is_empty())
                .collect();
            tail = t[close + 1..].trim_start();
        }
        let modport = match tail.strip_prefix('.') {
            Some(m) => Some(m.chars().filter(|c| !c.is_whitespace()).collect::<String>()),
            None if tail.is_empty() => None,
            None => return None,
        };
        Some((name, args, modport))
    }

    /// The canonical (whitespace-free) key of `virtual name#(args).modport`
    /// (§6.22.1): named arguments moved to their slots and omitted trailing
    /// parameters filled with the interface's defaults, so `virtual I`,
    /// `virtual interface I#(int)` and `virtual I#(.T(int), .N(4))` key one
    /// specialization.
    pub(super) fn canonical_vif_frag(
        &self,
        name: &str,
        mut args: Vec<String>,
        modport: Option<String>,
    ) -> String {
        if let Some(defs) = self.module.interface_param_defaults.get(name) {
            if args.iter().any(|a| Self::named_spec_arg(a).is_some()) {
                let mut slots: Vec<Option<String>> = vec![None; defs.len().max(args.len())];
                for (k, a) in args.iter().enumerate() {
                    match Self::named_spec_arg(a) {
                        Some((n, v)) => {
                            if let Some(i) = defs.iter().position(|(p, _)| p == n) {
                                slots[i] = (!v.is_empty()).then(|| v.to_string());
                            }
                        }
                        None => slots[k] = Some(a.clone()),
                    }
                }
                args.clear();
                for (i, s) in slots.into_iter().enumerate() {
                    match s.or_else(|| defs.get(i).and_then(|(_, d)| d.clone())) {
                        Some(v) => args.push(v),
                        None => break,
                    }
                }
            }
            while let Some(Some(d)) = defs.get(args.len()).map(|(_, d)| d.clone()) {
                args.push(d);
            }
        }
        let mut out = format!("virtual{}", name);
        if !args.is_empty() {
            out.push_str("#(");
            out.push_str(&args.join(","));
            out.push(')');
        }
        if let Some(mp) = modport {
            out.push('.');
            out.push_str(&mp);
        }
        Self::normalize_spec_ws(&out)
    }

    /// §6.18: a typedef name used as a specialization argument stands for
    /// its target type. Rendered for the target shapes a specialization key
    /// can spell the same way when written out — a virtual interface (raw
    /// arguments, resolved by the caller) and a one-keyword built-in — so
    /// `cfg_db#(vif_t)` and `cfg_db#(virtual my_if#(int))` share statics.
    /// None for classes and every other target (their keys are unchanged).
    pub(super) fn typedef_alias_frag(&self, name: &str) -> Option<String> {
        use crate::ast::types::DataType;
        let name = name.trim();
        // §6.18: a typedef stands for its target type — including one
        // declared at MODULE scope (`typedef my_pkg::Cfg CfgTQ;`), not
        // only a class-member alias. Restrict the module-table opening to
        // plain class targets so builtin-type typedefs keep their
        // existing (verbatim) keying.
        let module_class_alias = self
            .module
            .typedef_types
            .get(name)
            .map(|dt| {
                matches!(
                    dt,
                    DataType::TypeReference {
                        dimensions,
                        type_args,
                        ..
                    } if dimensions.is_empty() && type_args.is_empty()
                )
            })
            .unwrap_or(false);
        if !self.class_member_names().alias_typedefs.contains(name)
            && !module_class_alias
        {
            return None;
        }
        let mut cur = name.to_string();
        for _ in 0..8 {
            if self.module.classes.contains_key(&cur) {
                // The chain reached a CLASS. The original name already
                // spells that class (nothing to rewrite); an alias step
                // in, though, stands for the class it names — spell it
                // so the caller canonicalizes it like a direct naming.
                return if cur != name { Some(cur) } else { None };
            }
            if self.module.typedef_unpacked_dims.contains_key(&cur) {
                return None;
            }
            let dt = self.lookup_typedef_target(&cur)?;
            match &dt {
                DataType::Interface {
                    name: iname,
                    modport,
                    type_args,
                    ..
                } => {
                    let mut out = format!("virtual {}", iname.name);
                    if !type_args.is_empty() {
                        let frags: Vec<String> = type_args
                            .iter()
                            .map(|e| match &e.kind {
                                ExprKind::Ident(h) if h.path.len() == 1 => {
                                    Some(h.path[0].name.name.clone())
                                }
                                _ => self.expr_to_spec_fragment(e),
                            })
                            .collect::<Option<_>>()?;
                        out.push_str(&format!("#({})", frags.join(",")));
                    }
                    if let Some(mp) = modport {
                        out.push('.');
                        out.push_str(&mp.name);
                    }
                    return Some(out);
                }
                // One-keyword built-ins only: a vector's key text
                // (`logicsigned[15:0]`) is read back as a type downstream
                // (`$typename(T)`), where the typedef name renders better.
                DataType::IntegerAtom { signing: None, .. }
                | DataType::Real { .. }
                | DataType::Simple { .. } => {
                    return crate::elaborate::data_type_to_spec_fragment(&dt);
                }
                DataType::TypeReference {
                    name: tn,
                    dimensions,
                    type_args,
                    ..
                } if dimensions.is_empty() && type_args.is_empty() => {
                    // Follow the target whether it is bare (`Cfg`) or
                    // scoped (`my_pkg::Cfg`): both are class keys in
                    // `module.classes`.
                    let target = if tn.has_scope() {
                        tn.qualified()
                    } else {
                        tn.name.name.clone()
                    };
                    if target == cur {
                        return None;
                    }
                    cur = target;
                }
                _ => return None,
            }
        }
        None
    }

    /// The unpacked dimensions a typedef `name` carries, in the enclosing
    /// class context (its class-local typedefs first) or the design.
    pub(super) fn typedef_unpacked_dims_of(
        &self,
        name: &str,
    ) -> Option<&Vec<crate::ast::types::UnpackedDimension>> {
        if let Some(Some(ctx)) = self.class_context_stack.last() {
            let mut cur = Some(ctx.as_str());
            let mut hops = 0;
            while let Some(cn) = cur {
                hops += 1;
                let Some(cd) = self.module.classes.get(cn).filter(|_| hops < 64) else {
                    break;
                };
                if let Some(d) = cd.typedef_unpacked_dims.get(name) {
                    return Some(d);
                }
                if cd.typedef_targets.contains_key(name) {
                    return None;
                }
                cur = cd.extends.as_deref();
            }
        }
        self.module.typedef_unpacked_dims.get(name)
    }

    /// §6.22.1: the canonical text of the type a typedef `name` stands for
    /// when it carries unpacked dimensions (`typedef T tq_t[$]` in
    /// `mon#(int)` -> `int[$]`); None for any other name. Two typedefs of
    /// MATCHING types — same element type, same kind of array, the same
    /// bounds for a fixed-size one — give the same text.
    pub(super) fn unpacked_typedef_type_key(&self, name: &str, depth: u32) -> Option<String> {
        use crate::ast::types::UnpackedDimension as U;
        if depth > 8 {
            return None;
        }
        // `pkg::td`: a package typedef is tabled under its bare name.
        let name = match name.rsplit_once("::") {
            Some((pkg, leaf))
                if self.module.packages.contains(pkg)
                    && self.typedef_unpacked_dims_of(name).is_none() =>
            {
                leaf
            }
            _ => name,
        };
        let Some(dims) = self
            .typedef_unpacked_dims_of(name)
            .filter(|d| !d.is_empty())
        else {
            // A typedef of an unpacked typedef (`typedef intq_t q3_t;`), or of
            // a type parameter bound to one, names the same type.
            let DataType::TypeReference {
                name: tn,
                dimensions,
                type_args,
                ..
            } = self.lookup_typedef_target(name)?
            else {
                return None;
            };
            if !dimensions.is_empty() || !type_args.is_empty() {
                return None;
            }
            let mut target = if tn.has_scope() {
                format!(
                    "{}::{}",
                    tn.scopes
                        .iter()
                        .map(|s| s.name.name.as_str())
                        .collect::<Vec<_>>()
                        .join("::"),
                    tn.name.name
                )
            } else {
                tn.name.name.clone()
            };
            if let Some(b) = self.resolve_type_param_binding(&target) {
                target = b;
            }
            if target == name {
                return None;
            }
            return self.unpacked_typedef_type_key(&target, depth + 1);
        };
        let elem = self.lookup_typedef_target(name)?;
        let mut out = self.type_key_of_elem(&elem, depth)?;
        let params = &self.module.parameters;
        let ev = |e: &Expression| super::elaborate::const_eval_i64_with_params(e, Some(params));
        for d in dims {
            match d {
                U::Queue { max_size: None, .. } => out.push_str("[$]"),
                U::Queue {
                    max_size: Some(m), ..
                } => out.push_str(&format!("[$:{}]", ev(m)?)),
                U::Unsized(_) => out.push_str("[]"),
                U::Associative {
                    data_type: None, ..
                } => out.push_str("[*]"),
                U::Associative {
                    data_type: Some(k), ..
                } => out.push_str(&format!("[{}]", self.type_key_of_elem(k, depth)?)),
                U::Range { left, right, .. } => {
                    out.push_str(&format!("[{}:{}]", ev(left)?, ev(right)?))
                }
                U::Expression { expr, .. } => out.push_str(&format!("[0:{}]", ev(expr)? - 1)),
            }
        }
        Some(out)
    }

    /// The canonical text of an element (or key) type for
    /// `unpacked_typedef_type_key`: a type parameter follows its binding, a
    /// typedef its target; built-in types render with their signing.
    pub(super) fn type_key_of_elem(&self, dt: &DataType, depth: u32) -> Option<String> {
        match dt {
            DataType::TypeReference {
                name,
                dimensions,
                type_args,
                ..
            } if dimensions.is_empty() && type_args.is_empty() && !name.has_scope() => {
                let mut n = name.name.name.clone();
                if let Some(b) = self.resolve_type_param_binding(&n) {
                    n = b;
                }
                if let Some(k) = self.unpacked_typedef_type_key(&n, depth + 1) {
                    return Some(k);
                }
                if !self.module.classes.contains_key(&n)
                    && let Some(t) = self.lookup_typedef_target(&n)
                    && !matches!(&t, DataType::TypeReference { name: tn, .. } if tn.name.name == n)
                    && depth < 8
                {
                    return self.type_key_of_elem(&t, depth + 1);
                }
                Some(n)
            }
            _ => crate::elaborate::data_type_to_spec_fragment(dt),
        }
    }

    /// §6.22.1: the typedef that stands, in specialization keys, for the
    /// unpacked array type the typedef `name` names: the first (by name)
    /// design-level typedef of a matching type. `db#(intq_t)` and
    /// `db#(tq_t)` (`typedef T tq_t[$]` inside `mon#(int)`) are one
    /// specialization. None when `name` is no unpacked typedef or no
    /// design-level typedef matches it.
    pub(super) fn unpacked_typedef_rep(&self, name: &str) -> Option<String> {
        if self.module.typedef_unpacked_dims.is_empty() {
            return None;
        }
        let names = self.unpacked_typedef_names.get_or_init(|| {
            let mut set: HashSet<String> = HashSet::default();
            let add = |n: &str, set: &mut HashSet<String>| {
                set.insert(n.to_string());
                if let Some((_, leaf)) = n.rsplit_once("::") {
                    set.insert(leaf.to_string());
                }
            };
            for n in self.module.typedef_unpacked_dims.keys() {
                add(n, &mut set);
            }
            for cd in self.module.classes.values() {
                for n in cd.typedef_unpacked_dims.keys() {
                    add(n, &mut set);
                }
            }
            // Aliases of those (`typedef intq_t q3_t;`), to a fixed point.
            let aliases: Vec<(&String, &String)> = self
                .module
                .typedef_types
                .iter()
                .chain(
                    self.module
                        .classes
                        .values()
                        .flat_map(|c| c.typedef_targets.iter()),
                )
                .filter_map(|(n, dt)| match dt {
                    DataType::TypeReference {
                        name: tn,
                        dimensions,
                        type_args,
                        ..
                    } if dimensions.is_empty() && type_args.is_empty() => Some((n, &tn.name.name)),
                    _ => None,
                })
                .collect();
            loop {
                let before = set.len();
                for (n, t) in &aliases {
                    if set.contains(t.as_str()) && !set.contains(n.as_str()) {
                        add(n, &mut set);
                    }
                }
                if set.len() == before {
                    break;
                }
            }
            set
        });
        if !names.contains(name)
            && !name
                .rsplit_once("::")
                .is_some_and(|(_, leaf)| names.contains(leaf))
        {
            return None;
        }
        let key = self.unpacked_typedef_type_key(name, 0)?;
        let reps = self.unpacked_typedef_reps.get_or_init(|| {
            // Typedefs declared at design level (module, package, $unit),
            // keyed by their type outside any class context.
            let mut names: Vec<&String> = self
                .module
                .typedef_unpacked_dims
                .keys()
                .filter(|n| {
                    // Not a class's own typedef (bare, or `cls::td`).
                    !self
                        .module
                        .classes
                        .values()
                        .any(|c| c.typedef_targets.contains_key(n.as_str()))
                        && !n
                            .rsplit_once("::")
                            .is_some_and(|(h, _)| self.module.classes.contains_key(h))
                })
                .collect();
            names.sort_by_key(|n| (n.contains("::"), n.as_str()));
            let mut m: HashMap<String, String> = HashMap::default();
            for n in names {
                if let Some(k) = self.unpacked_typedef_type_key(n, 0) {
                    m.entry(k).or_insert_with(|| n.clone());
                }
            }
            m
        });
        reps.get(&key).cloned()
    }

    /// Normalize layout whitespace outside of double-quoted string literals so that
    /// sig fragments that differ only in spacing (the parser's spaced
    /// reconstruction `special_comp # ( 1 )` vs a typedef-resolved
    /// `special_comp#(1)`) key to the same per-spec static cell.
    pub(super) fn normalize_spec_ws(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut in_str = false;
        let mut chars = s.char_indices().peekable();
        while let Some((index, ch)) = chars.next() {
            if ch == '"' {
                in_str = !in_str;
                out.push(ch);
            } else if in_str {
                out.push(ch);
            } else if ch.is_whitespace() {
                let mut next = index + ch.len_utf8();
                while let Some(&(i, c)) = chars.peek() {
                    if !c.is_whitespace() {
                        break;
                    }
                    next = i + c.len_utf8();
                    chars.next();
                }
                // §6.11 / §8.25: `byte unsigned` is a qualified builtin,
                // while `byteunsigned` can name an unrelated typedef.
                let word = out
                    .rsplit(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
                    .unwrap_or("");
                let integral = matches!(
                    word,
                    "byte"
                        | "shortint"
                        | "int"
                        | "longint"
                        | "integer"
                        | "time"
                        | "bit"
                        | "logic"
                        | "reg"
                );
                let tail = &s[next..];
                let signing = tail
                    .strip_prefix("unsigned")
                    .or_else(|| tail.strip_prefix("signed"))
                    .is_some_and(|rest| {
                        rest.chars()
                            .next()
                            .is_none_or(|c| !c.is_alphanumeric() && c != '_')
                    });
                if integral && signing {
                    out.push(' ');
                }
            } else {
                out.push(ch);
            }
        }
        out
    }

    pub(super) fn instantiate_class(
        &mut self,
        class_def: &crate::compiler::elaborate::ElaboratedClass,
        args: &[Expression],
    ) -> Value {
        self.instantiate_class_with_type_args(class_def, args, None)
    }

    /// Resolve a factory-requested type name to the concrete
    /// elaborated class. A class registration macro registers C under name "C", so
    /// the requested name usually IS the class name; also reverse-resolve via
    /// `type_id` typedef targets (`registry#(C,"N")` -> N) for the rare
    /// case the registered name differs. Returns the class def to construct.
    pub(super) fn pure_factory_lookup(
        &self,
        requested: &str,
    ) -> Option<std::sync::Arc<crate::compiler::elaborate::ElaboratedClass>> {
        use crate::ast::types::DataType;
        if let Some(cd) = self.module.classes.get(requested) {
            return Some(cd.clone());
        }
        for cd in self.module.classes.values() {
            // A specialization's class entry repeats its origin's `type_id`.
            if self.spec_clone_origin.contains_key(&cd.name) {
                continue;
            }
            if let Some(DataType::TypeReference {
                name, type_args, ..
            }) = cd.typedef_targets.get("type_id")
            {
                if name.name.name.contains("registry") && type_args.len() >= 2 {
                    if let ExprKind::StringLiteral(n) = &type_args[1].kind {
                        if n == requested {
                            // type_args[0] is the registered class.
                            if let Some(tc) = Self::leaf_ident_name(&type_args[0]) {
                                if let Some(c) = self.module.classes.get(&tc) {
                                    return Some(c.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Resolve a class PROPERTY's declared `#(...)` specialization args by
    /// walking the current `this` instance's class hierarchy. Locals are in
    /// `var_type_args` and module-level decls in `class_type_args`, but a
    /// class property typed with a parameterized type
    /// (`special_comp#(1) a1;`) had neither, so `this.a1 = new(...)` bound no
    /// type-args and value parameters defaulted — collapsing `#(1)`/`#(2)`
    /// to `#(0)`. Elaboration now records these in
    /// `ElaboratedClass::property_type_args`; this looks them up.
    pub(super) fn this_property_type_args(&self, prop_name: &str) -> Option<Vec<Expression>> {
        let h = self.this_stack.last().copied().flatten()?;
        let class_name = self
            .heap
            .get(h)
            .and_then(|o| o.as_ref())?
            .class_name
            .clone();
        let mut cur = self.module.classes.get(&class_name).cloned();
        while let Some(c) = cur {
            if let Some(ta) = c.property_type_args.get(prop_name) {
                return Some(ta.clone());
            }
            cur = c
                .extends
                .as_ref()
                .and_then(|e| self.module.classes.get(e).cloned());
        }
        None
    }

    /// Resolve the declared `#(...)` specialization args to use when
    /// constructing `lvalue = new(...)`. Checks module-level decls, then
    /// procedural locals, then class properties of the current `this`
    /// instance. `resolved_name` is the lvalue's `resolve_hier_name` result
    /// (used as the key for the first two); the property fallback keys off
    /// the lvalue's leaf identifier so `this.prop` and bare `prop` both work.
    pub(super) fn resolve_ctor_type_args(
        &self,
        lvalue: &Expression,
        resolved_name: Option<&str>,
    ) -> Option<Vec<Expression>> {
        if let Some(n) = resolved_name {
            if let Some(ta) = self.module.class_type_args.get(n) {
                return Some(ta.clone());
            }
            if let Some(ta) = self.var_type_args.get(n) {
                return Some(ta.clone());
            }
        }
        if let ExprKind::Ident(lh) = &lvalue.kind {
            if let Some(leaf) = lh.path.last() {
                if let Some(ta) = self.this_property_type_args(&leaf.name.name) {
                    return Some(ta);
                }
            }
        }
        None
    }

    /// When the lvalue is a class MEMBER whose declared type is a TYPEDEF
    /// ALIAS to a PARAMETERIZED class — e.g. UVM's
    /// `typedef uvm_object_string_pool#(uvm_event#(uvm_object))
    /// uvm_event_pool;  ...  uvm_event_pool event_pool;` — recover the
    /// target specialization `(base, sig)` so a later
    /// `event_pool = new(...)` binds the pool's `T` to the concrete
    /// element (`uvm_event#(uvm_object)`) instead of the class default
    /// (`uvm_object`). The field's own `#(...)` is empty (the args live on
    /// the alias target), so `get_expr_type_name` returns just the base
    /// class and `resolve_ctor_type_args` finds no args.
    ///
    /// Returns the specialization only when the member's raw declared type
    /// is a typedef alias (not a direct class/`#` spelling). Walks the
    /// `this`-instance inheritance chain so a base-class property (e.g.
    /// `uvm_component.event_pool`) resolves from a derived component.
    pub(super) fn this_member_typedef_spec(&self, lvalue: &Expression) -> Option<(String, String)> {
        let leaf = match &lvalue.kind {
            ExprKind::Ident(h) => h.path.last()?.name.name.clone(),
            ExprKind::MemberAccess { member, .. } => member.name.clone(),
            _ => return None,
        };
        let h = self.this_stack.last().copied().flatten()?;
        let this_cn = self
            .heap
            .get(h)
            .and_then(|o| o.as_ref())?
            .class_name
            .clone();
        let mut cur = Some(this_cn);
        while let Some(cn) = cur {
            let cd = self.module.classes.get(&cn)?;
            if let Some(prop) = cd.properties.get(&leaf) {
                if let Some(raw) = &prop.type_name {
                    // `raw` is the declared type name. If it is a typedef
                    // alias to a parameterized class, `resolve_typedef_spec`
                    // follows it. Base classes / non-aliases yield None.
                    if let Some(spec) = self.resolve_typedef_spec(raw) {
                        return Some(spec);
                    }
                }
                return None;
            }
            cur = cd.extends.clone();
        }
        None
    }

    /// §8.25: `obj.m = new` where `obj` is another object of a parameterized
    /// class and `m` is declared through that class's parameters (`self_t
    /// m;` with `typedef G#(T) self_t;`, or `G#(T) m;`): the specialization
    /// `(base, sig)` those parameters name on `obj`, as a method of `obj`
    /// executing `m = new` would build. `None` when the lvalue has no such
    /// receiver or the member's type takes no parameters.
    pub(super) fn receiver_member_spec(&mut self, lvalue: &Expression) -> Option<(String, String)> {
        let (recv, prop) = Self::split_trailing_member(lvalue)?;
        // Only a receiver that names an object (no call to run twice).
        fn names_object(e: &Expression) -> bool {
            match &e.kind {
                ExprKind::Ident(_) => true,
                ExprKind::MemberAccess { expr, .. } => names_object(expr),
                ExprKind::Index { expr, index } => {
                    names_object(expr)
                        && matches!(index.kind, ExprKind::Ident(_) | ExprKind::Number(_))
                }
                _ => false,
            }
        }
        if !names_object(&recv) {
            return None;
        }
        let h = self.eval_expr(&recv).to_u64()? as usize;
        if h == 0 || self.this_stack.last().copied().flatten() == Some(h) {
            return None;
        }
        let inst = self.heap.get(h)?.as_ref()?;
        if inst.spec.is_none() && inst.type_bindings.is_empty() {
            return None;
        }
        let (cls, spec) = (inst.class_name.clone(), inst.spec.clone());
        // The class in `obj`'s chain that declares `m`, and its declared type.
        let mut cur = Some(cls.as_str());
        let mut decl = None;
        for _ in 0..64 {
            let Some(cd) = cur.and_then(|c| self.module.classes.get(c)) else {
                break;
            };
            if let Some(sig) = cd.properties.get(&prop) {
                decl = Some((
                    sig.type_name.clone()?,
                    cd.property_type_args.get(&prop).cloned(),
                ));
                break;
            }
            cur = cd.extends.as_deref();
        }
        let (tn, args) = decl?;
        let saved_spec = std::mem::replace(&mut self.current_spec, spec);
        self.this_stack.push(Some(h));
        self.class_context_stack.push(Some(cls));
        let out = match args.filter(|a| !a.is_empty()) {
            // `G#(T) m;`: the arguments resolved through `obj`'s bindings.
            Some(type_args) => {
                let dt = DataType::TypeReference {
                    name: crate::ast::types::TypeName {
                        scopes: Vec::new(),
                        name: crate::ast::Identifier {
                            name: tn,
                            span: crate::ast::Span::dummy(),
                        },
                        span: crate::ast::Span::dummy(),
                    },
                    dimensions: Vec::new(),
                    type_args,
                    span: crate::ast::Span::dummy(),
                };
                self.spec_from_typedef_dt(&dt)
            }
            // `self_t m;`: a typedef of a specialization.
            None => self.resolve_typedef_spec(&tn),
        };
        self.class_context_stack.pop();
        self.this_stack.pop();
        self.current_spec = saved_spec;
        out
    }

    /// Value parameters of the running instance, for a specialization
    /// argument evaluated in that instance's scope (`item #(H, D)` inside
    /// `my_seq #(32, 194)`). A static/free frame is a boundary: its caller's
    /// object does not supply the callee's lexical parameters.
    pub(super) fn enclosing_value_params(&mut self) -> Option<HashMap<String, Value>> {
        let lexical = self.class_context_stack.last()?.as_ref()?.clone();
        let mut map: HashMap<String, Value> = HashMap::default();
        let mut names = Vec::new();
        let instance = self
            .this_stack
            .last()
            .copied()
            .flatten()
            .and_then(|h| self.heap.get(h))
            .and_then(|o| o.as_ref());
        let mut cn: Option<&str> = Some(&lexical);
        for _ in 0..64 {
            let Some(cd) = cn.and_then(|c| self.module.classes.get(c)) else {
                break;
            };
            for (n, _) in &cd.param_defaults {
                if cd.type_param_names.iter().any(|t| t == n) {
                    continue;
                }
                if !names.contains(n) {
                    names.push(n.clone());
                }
                if let Some(v) = instance.and_then(|inst| {
                    inst.properties
                        .get(&format!("{}::{}", cd.name, n))
                        .or_else(|| inst.properties.get(n))
                }) {
                    map.entry(n.clone()).or_insert_with(|| v.clone());
                }
            }
            cn = cd.extends.as_deref();
        }
        let saved = self.current_spec.clone();
        self.current_spec = self.lexical_value_spec();
        for name in names {
            if let Some(value) = self.resolve_value_param_in_current(&name) {
                map.insert(name, value);
            }
        }
        self.current_spec = saved;
        (!map.is_empty()).then_some(map)
    }

    /// IEEE 1800-2017 §6.20.2 / §8.25: the value of the declared default `e`
    /// of a parameter of `chain[0]` (the declaring class, `chain[1..]` its
    /// ancestors) for an object under construction. A default belongs to the
    /// CLASS's scope, where the parameters before it are visible with the
    /// values THIS specialization bound (`int D = W*2`) — not to the `new`
    /// call site, where `W` read x or, worse, a same-named parameter of the
    /// calling module.
    pub(super) fn eval_param_default(
        &mut self,
        e: &Expression,
        chain: &[std::sync::Arc<crate::compiler::elaborate::ElaboratedClass>],
        instance: &ClassInstance,
        leaf_spec: &Option<(String, String)>,
    ) -> Value {
        let Some(decl) = chain.first() else {
            return self.eval_expr(e);
        };
        // The parameters bound so far on this chain, the declaring class's
        // shadowing its ancestors'.
        let mut frame = self.take_pooled_frame();
        for c in chain {
            for (pn, _) in &c.param_defaults {
                if let Some(v) = instance.properties.get(pn) {
                    frame.entry(pn.clone()).or_insert_with(|| v.clone());
                }
            }
        }
        // A type parameter in the default (`int W = $bits(U)`) resolves
        // through the declaring class's specialization.
        let leaf = instance.class_name.as_str();
        let leaf_spec = leaf_spec
            .clone()
            .or_else(|| self.current_spec.clone().filter(|(b, _)| b == leaf));
        let spec = if decl.name == leaf {
            leaf_spec
        } else if let Some((_, lsig)) = leaf_spec {
            self.ancestor_spec(leaf, &lsig, &decl.name).map(|s| {
                (
                    decl.name.clone(),
                    self.canonicalize_spec_sig(&decl.name, &s),
                )
            })
        } else {
            self.static_receiver_spec(leaf, &decl.name)
        };
        let saved_spec = std::mem::replace(&mut self.current_spec, spec);
        self.class_context_stack.push(Some(decl.name.clone()));
        self.this_stack.push(None);
        self.push_local_frame(frame);
        let v = self.eval_expr(e);
        self.pop_local_frame();
        self.this_stack.pop();
        self.class_context_stack.pop();
        self.current_spec = saved_spec;
        v
    }

    /// §8.23/§8.25: the `#(...)` arguments for building `nested` — a class
    /// declared inside a parameterized class, which the elaborator gives the
    /// enclosing class's parameters first — from inside a method of the
    /// enclosing object: that object's binding of each such parameter.
    /// `None` when `nested` is no such class or `this` is not an object of
    /// the enclosing class.
    pub(super) fn enclosing_param_args(
        &self,
        nested: &crate::compiler::elaborate::ElaboratedClass,
    ) -> Option<Vec<Expression>> {
        let enc = nested.enclosing.as_deref()?;
        let enc_cd = self.module.classes.get(enc)?;
        if enc_cd.param_order.is_empty() {
            return None;
        }
        let h = self.this_stack.last().copied().flatten()?;
        let inst = self.heap.get(h)?.as_ref()?;
        if inst.class_name != enc && !self.class_extends(&inst.class_name, enc) {
            return None;
        }
        let span = crate::ast::Span::dummy();
        let mut out = Vec::new();
        for p in &nested.param_order {
            if !enc_cd.param_order.contains(p) {
                break;
            }
            let e = if enc_cd.type_param_names.contains(p) {
                let b = self.obj_type_param(h, enc, p)?;
                let b = b.trim();
                if b.chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == ':')
                {
                    Expression::new(
                        ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                            root: None,
                            path: vec![crate::ast::expr::HierPathSegment {
                                name: crate::ast::Identifier {
                                    name: b.to_string(),
                                    span,
                                },
                                selects: Vec::new(),
                            }],
                            span,
                            cached_signal_id: std::cell::Cell::new(None),
                            cached_resolved_name: std::cell::OnceCell::new(),
                        }),
                        span,
                    )
                } else {
                    Expression::new(
                        ExprKind::TypeLiteral(Box::new(self.bound_type_dt(b)?)),
                        span,
                    )
                }
            } else {
                let v = inst.properties.get(p)?;
                if v.has_xz() {
                    return None;
                }
                Expression::new(
                    ExprKind::Number(crate::ast::expr::NumberLiteral::Integer {
                        size: None,
                        signed: false,
                        base: crate::ast::expr::NumberBase::Decimal,
                        value: v.to_u64()?.to_string(),
                        cached_val: std::cell::Cell::new(None),
                    }),
                    span,
                )
            };
            out.push(e);
        }
        (!out.is_empty()).then_some(out)
    }

    pub(super) fn instantiate_class_with_type_args(
        &mut self,
        class_def: &crate::compiler::elaborate::ElaboratedClass,
        args: &[Expression],
        type_args: Option<&[Expression]>,
    ) -> Value {
        // Per-specialization static initialization for a PARAMETERIZED
        // class: run its static-call initializers (e.g. callback registration
        // macro expansions: `static bit = callbacks#(T,CB)::m_register_pair(...)`)
        // ONCE PER specialization, with `current_spec` set to this
        // specialization. A value-parameterized class is modeled as a
        // single elaborated class (value params are instance properties),
        // so its statics cannot run once at startup — the class-local
        // typedef `typedef C#(N) this_type;` needs an active spec to
        // resolve `N`, otherwise every specialization collapses onto one
        // bare-typedef typeid and typewide callbacks cross-propagate.
        // Triggering at first instantiation matches the LRM semantics
        // (a specialization's statics initialize before first use) and
        // runs before any `add`/query that needs the registration.
        // §8.23/§8.25: a nested class built without its own `#(...)` takes
        // the enclosing class's parameters from the enclosing object.
        let nested_args;
        let type_args = match type_args {
            Some(t) if !t.is_empty() => Some(t),
            _ => match self.enclosing_param_args(class_def) {
                Some(a) => {
                    nested_args = a;
                    Some(nested_args.as_slice())
                }
                None => type_args,
            },
        };
        let class_name_owned = class_def.name.clone();
        let mut computed_spec = None;
        if self.class_is_parameterized(&class_name_owned) {
            // §8.25.1: two specializations are the SAME when their parameter
            // values are the same — so an unspecialized `C c = new();` and an
            // explicit `C #(<the defaults>) c = new();` are one specialization
            // and must share one set of `static` members.
            //
            // Only an explicit `#(...)` list used to produce a spec; a bare
            // `new()` left `spec = None` and fell back to the shared
            // UNspecialized static cell, so the two spellings keyed different
            // cells. `canonicalize_spec_sig` already pads absent trailing
            // positions with their declared defaults, so handing it an empty
            // sig yields exactly the all-defaults form the explicit spelling
            // canonicalizes to.
            let raw_sig: Option<String> = match type_args {
                Some(ta) if !ta.is_empty() => {
                    let frags: Vec<String> = ta
                        .iter()
                        .filter_map(|e| self.expr_to_spec_fragment(e))
                        .collect();
                    (frags.len() == ta.len()).then(|| frags.join(","))
                }
                _ => Some(String::new()),
            };
            if let Some(raw_sig) = raw_sig {
                let sig = self.canonicalize_spec_sig(&class_name_owned, &raw_sig);
                let effective_sig = if sig.is_empty() {
                    self.module
                        .classes
                        .get(&class_name_owned)
                        .map(|cd| {
                            cd.type_param_defaults
                                .iter()
                                .map(|(_, d)| d.as_str())
                                .collect::<Vec<_>>()
                                .join(",")
                        })
                        .unwrap_or_default()
                } else {
                    sig.clone()
                };
                if !effective_sig.is_empty() {
                    computed_spec = Some((class_name_owned.clone(), effective_sig.clone()));
                    let saved = self.current_spec.take();
                    self.current_spec = Some((class_name_owned.clone(), effective_sig.clone()));
                    self.ensure_spec_statics(&class_name_owned, &effective_sig);
                    self.current_spec = saved;
                }
            }
        }
        // §8.25: a class whose base is a type parameter (`class wrap_c
        // #(type BASE = base_c) extends BASE`) has its own class entry for
        // each specialization binding another base (`wrap_c #(derived_c)`);
        // the object is an instance of that entry, so every hierarchy walk
        // from it sees the bound base. The specialization stays keyed by
        // the declared class (statics, `$typename`).
        let spec_entry = if self.is_dep_class(&class_name_owned) {
            self.spec_entry_for_new(&class_name_owned, type_args, computed_spec.as_ref())
        } else {
            None
        };
        // §8.25/§25.9: a property typed by a bare TYPE PARAMETER is a
        // virtual-interface property in every specialization whose `#(...)`
        // binds that parameter to `virtual <iface>` — e.g. the `T vif` of a
        // `uvm_driver#(T)` built through `drv#(virtual bus_if)`. The active
        // spec (an explicit `#(...)` list, or `current_spec` carried by a
        // static `create()`), not the padded default of `computed_spec`, is
        // what names the interface.
        let param_vif_entry = if self.is_dep_class(&class_name_owned) {
            None
        } else {
            let active = self
                .current_spec
                .clone()
                .filter(|(b, _)| *b == class_name_owned)
                .or_else(|| computed_spec.clone());
            active.and_then(|(b, sig)| self.ensure_param_vif_class(&b, &sig))
        };
        let class_def: &crate::compiler::elaborate::ElaboratedClass = param_vif_entry
            .as_deref()
            .or(spec_entry.as_deref())
            .unwrap_or(class_def);
        let handle = self.heap.len();
        let mut instance = ClassInstance {
            class_name: class_def.name.clone(),
            properties: PropMap::default(),
            type_bindings: HashMap::default(),
            spec: None,
            creation_scope: self.active_instance_scope(),
        };
        // §8.25: map the specialization's `#(...)` args onto the leaf
        // class's parameters BY NAME (type and value params interleave in
        // `param_order`), so value-parameter overrides land on the right
        // parameter and type args never clobber a value parameter.
        let arg_map: HashMap<String, Expression> = type_args
            .map(|ta| self.class_param_arg_map(class_def, ta))
            .unwrap_or_default();
        // The whole ancestor chain used to be DEEP-CLONED here on every
        // `new()` (ElaboratedClass includes method ASTs) — 30% of a UVM
        // testbench run. `module.classes` now stores Arcs, so the chain
        // walk below bumps refcounts; the leaf reuses its map Arc when the
        // caller's reference IS that object (pointer-verified), and only
        // an off-map def (factory-synthesized) pays a real clone.
        let leaf_from_map = self
            .module
            .classes
            .get(&class_def.name)
            .filter(|c| std::sync::Arc::as_ptr(c) == class_def as *const _)
            .cloned();
        let leaf_in_map = leaf_from_map.is_some();
        let cached_tpl = leaf_from_map.as_ref().and_then(|la| {
            self.inst_templates
                .borrow()
                .get(class_def.name.as_str())
                .filter(|t| {
                    t.chain
                        .first()
                        .is_some_and(|c0| std::sync::Arc::ptr_eq(c0, la))
                })
                .cloned()
        });
        let (tpl, owned_chain);
        let classes_to_init: &[std::sync::Arc<crate::compiler::elaborate::ElaboratedClass>] =
            if let Some(t) = cached_tpl {
                tpl = Some(t);
                &tpl.as_ref().unwrap().chain
            } else {
                let leaf_arc =
                    leaf_from_map.unwrap_or_else(|| std::sync::Arc::new(class_def.clone()));
                let mut chain = vec![leaf_arc];
                let mut cur: Option<&str> = class_def.extends.as_deref();
                let mut seen: Vec<&str> = vec![class_def.name.as_str()];
                while let Some(cname) = cur {
                    if seen.contains(&cname) {
                        break;
                    }
                    seen.push(cname);
                    if let Some(cdef) = self.module.classes.get(cname) {
                        chain.push(std::sync::Arc::clone(cdef));
                        cur = cdef.extends.as_deref();
                    } else {
                        break;
                    }
                }
                tpl = leaf_in_map.then(|| self.inst_template(&chain));
                owned_chain = chain;
                &owned_chain
            };
        let chain_has_params = match tpl.as_ref() {
            Some(t) => t.has_value_params,
            None => classes_to_init.iter().any(|c| !c.param_defaults.is_empty()),
        };
        // §8.25 / §8.13: `class D extends B #(8);` supplies B's VALUE
        // parameters. `classes_to_init` is leaf-first, so entry `i`'s
        // `extends_type_args` binds entry `i+1`'s parameters. Without this an
        // inherited property (and any method return) sized by a base parameter
        // used the base's DEFAULT — `extends base #(8)` still reported 4 bits.
        //
        // Fragments are the textual `#(...)` args: a numeric literal binds
        // directly; a bare identifier is resolved against the LEAF's own
        // parameters (`class D #(int N) extends B #(N);`), which are known
        // here from `arg_map`/defaults even though the leaf is bound last.
        let ancestor_value_args: HashMap<String, HashMap<String, Value>> = if !chain_has_params {
            HashMap::default()
        } else {
            // An actual `#(...)` argument is an expression in the
            // specialization site's scope. Inside a method that is the
            // running instance, so `item #(H, D)` sees this object's
            // value parameters. Evaluating it with an empty map used the
            // global default (32) and the new object's properties
            // disagreed with the specialization `$cast` checks.
            let enclosing_params = self.enclosing_value_params();
            let mut leaf_params: HashMap<String, Value> = HashMap::default();
            for (pname, pdefault) in class_def.param_defaults.iter() {
                // An actual argument belongs to the specialization site's
                // scope; a default to the class's, where the parameters
                // before it are visible (§6.20.2, `int B = A*2`).
                let v = match (arg_map.get(pname), pdefault) {
                    (Some(e), _) => {
                        crate::elaborate::const_eval_i64_with_params(e, enclosing_params.as_ref())
                    }
                    (None, Some(e)) => {
                        crate::elaborate::const_eval_i64_with_params(e, Some(&leaf_params))
                    }
                    (None, None) => None,
                };
                if let Some(v) = v {
                    leaf_params.insert(pname.clone(), Value::from_u64(v as u64, 32));
                }
            }
            let mut out: HashMap<String, HashMap<String, Value>> = HashMap::default();
            for (wi, pair) in classes_to_init.windows(2).enumerate() {
                let (child, parent) = (&pair[0], &pair[1]);
                if child.extends_type_args.is_empty() {
                    continue;
                }
                // The child's own parameter values, against which a bare
                // name or expression in its `extends B #(N)` resolves: the
                // leaf's, or — one level further up (`class leaf extends
                // mid #(8); class mid #(int N) extends base #(N);`) — the
                // values the hop below bound, else the child's defaults.
                // Resolving only against the leaf's parameters dropped `N`,
                // so `base`'s properties kept the default width (§8.25).
                let child_params: std::borrow::Cow<HashMap<String, Value>> = if wi == 0 {
                    std::borrow::Cow::Borrowed(&leaf_params)
                } else {
                    let mut m = out.get(&child.name).cloned().unwrap_or_default();
                    for (pname, pdefault) in child.param_defaults.iter() {
                        if m.contains_key(pname) {
                            continue;
                        }
                        if let Some(v) = pdefault
                            .as_ref()
                            .and_then(|e| crate::elaborate::const_eval_i64_with_params(e, Some(&m)))
                        {
                            m.insert(pname.clone(), Value::from_u64(v as u64, 32));
                        }
                    }
                    std::borrow::Cow::Owned(m)
                };
                let leaf_params: &HashMap<String, Value> = &child_params;
                let mut binds: HashMap<String, Value> = HashMap::default();
                for (i, pname) in parent.param_order.iter().enumerate() {
                    if parent.type_param_names.iter().any(|t| t == pname) {
                        continue;
                    }
                    let Some(frag) = child.extends_type_args.get(i) else {
                        continue;
                    };
                    let frag = frag.trim();
                    if frag.is_empty() {
                        continue;
                    }
                    if let Some((p, k)) = Self::spec_projection(frag) {
                        // `BASE#[k]`: argument k of the child's binding of its
                        // base type parameter (`wrap_c #(pbase#(8))`).
                        if let Some(v) = self
                            .child_type_binding(&class_name_owned, class_def, child, p, type_args)
                            .and_then(|b| Self::project_spec_arg(&b, k))
                            .and_then(|f| Self::parse_spec_number(&f))
                            .and_then(|lit| {
                                let e = Expression::new(
                                    ExprKind::Number(lit),
                                    crate::ast::Span::dummy(),
                                );
                                crate::elaborate::const_eval_i64_with_params(&e, None)
                            })
                        {
                            binds.insert(pname.clone(), Value::from_u64(v as u64, 32));
                        }
                        continue;
                    }
                    // §8.25/§6.20.2: a STRING value parameter bound by the
                    // extends clause (`extends holder #(int, "ext")`) — the
                    // integer folds below cannot carry it, so the instance
                    // kept the base's default.
                    if let Some(s) = frag
                        .strip_prefix('"')
                        .and_then(|r| r.strip_suffix('"'))
                        .filter(|s| !s.contains('"') && !s.contains('\\'))
                    {
                        binds.insert(pname.clone(), Value::from_string(s));
                        continue;
                    }
                    if let Some(lit) = Self::parse_spec_number(frag) {
                        let e = Expression::new(ExprKind::Number(lit), crate::ast::Span::dummy());
                        if let Some(v) = crate::elaborate::const_eval_i64_with_params(&e, None) {
                            binds.insert(pname.clone(), Value::from_u64(v as u64, 32));
                        }
                    } else if let Some(v) = leaf_params.get(frag) {
                        binds.insert(pname.clone(), v.clone());
                    } else if child.extends_args.len() == child.extends_type_args.len() {
                        // EXPRESSION arg (`extends Base#(N * 3)`): the
                        // textual fragment is neither a literal nor a bare
                        // leaf param — evaluate the parsed extends
                        // expression against the leaf's bindings. (Guarded
                        // on 1:1 alignment with the textual list, since
                        // extends_args filters non-expression items.)
                        if let Some(e) = child.extends_args.get(i) {
                            if let Some(v) =
                                crate::elaborate::const_eval_i64_with_params(e, Some(&leaf_params))
                            {
                                binds.insert(pname.clone(), Value::from_u64(v as u64, 32));
                            }
                        }
                    }
                }
                if !binds.is_empty() {
                    out.insert(parent.name.clone(), binds);
                }
            }
            out
        };
        // §8.10: a property redeclared in a derived class is a SEPARATE
        // variable — the chain has one copy per declaring class. Seeding
        // everything under the bare name collapsed them into one slot (the
        // loop runs root-to-leaf, so the derived silently overwrote the
        // base's), and a base method's write became visible to the derived —
        // including across TYPES, where a base `string` read back a derived
        // `int`'s bits. The leaf-most declarer keeps the bare key; every
        // other declarer stores under `"<Class>::<name>"`.
        // A chain from the class table has its per-property facts in a
        // template (see `InstTemplate`); an off-table definition derives them.
        let leaf_declarer: HashMap<&str, &str> = if tpl.is_some() {
            HashMap::default()
        } else {
            let mut names: HashMap<&str, &str> = HashMap::default();
            for cdef in classes_to_init {
                for name in cdef.properties.keys() {
                    names.entry(name.as_str()).or_insert(cdef.name.as_str());
                }
            }
            names
        };
        if let Some(t) = tpl.as_ref() {
            for sp in &t.shadowed_props {
                if !self.shadowed_prop_names.contains(sp.as_str()) {
                    self.shadowed_prop_mask |= meta_key_bits(sp);
                    self.shadowed_prop_names.insert(sp.clone());
                }
            }
            if t.property_seed_keys.is_none() {
                bump_store_gen();
                instance.properties = PropMap(t.base_props.clone());
            }
        }
        for (ci, cdef) in classes_to_init
            .iter()
            .enumerate()
            .rev()
            .filter(|_| chain_has_params || tpl.is_none())
        {
            // §8.10/§8.25: seed this class AFTER its ancestors' parameters,
            // so a derived property can hide a same-named base parameter.
            if let Some(t) = tpl.as_ref() {
                if let Some(keys) = &t.property_seed_keys {
                    for key in &keys[ci] {
                        instance
                            .properties
                            .insert(key.clone(), t.base_props[key].clone());
                    }
                }
            } else {
                for (prop_name, prop_sig) in cdef.properties.iter() {
                    if cdef.static_properties.contains(prop_name)
                        || cdef.assoc_properties.contains_key(prop_name)
                    {
                        continue;
                    }
                    let seed = if leaf_declarer
                        .get(prop_name.as_str())
                        .is_some_and(|l| *l != cdef.name.as_str())
                    {
                        2 | if self.prop_seeds_x(cdef, prop_name) {
                            SEED_X
                        } else {
                            0
                        }
                    } else {
                        1 | if self.prop_seeds_x(cdef, prop_name) {
                            SEED_X
                        } else {
                            0
                        }
                    };
                    let key = if seed & 3 == 2 {
                        if !self.shadowed_prop_names.contains(prop_name.as_str()) {
                            self.shadowed_prop_mask |= meta_key_bits(prop_name);
                            self.shadowed_prop_names.insert(prop_name.clone());
                        }
                        format!("{}::{}", cdef.name, prop_name)
                    } else {
                        prop_name.clone()
                    };
                    if seed & SEED_X != 0 {
                        let mut v = Value::new(prop_sig.width.max(1));
                        v.is_signed = prop_sig.is_signed;
                        instance.properties.insert(key, v);
                        continue;
                    }
                    instance.properties.insert(key, prop_sig.value.clone());
                }
            }
            // Bind class VALUE parameters (§8.25): each parameter gets the
            // specialization's argument (on the leaf class only, matched by
            // NAME through `arg_map` — never by raw slot, since type and
            // value params interleave), falling back to its declared
            // default. `param_defaults` also carries class-body localparams,
            // which are not overridable and always take their default.
            let is_leaf = cdef.name == class_def.name;
            for (pname, pdefault) in cdef.param_defaults.iter() {
                // Value param not supplied via explicit `#(...)` type_args?
                // If an active specialization (current_spec) targets THIS
                // exact class, bind the value param from it. This covers
                // `static this_type m_inst; m_inst = new()` inside a static
                // method dispatched on a typedef specialization
                // (`AliasT::get()` where `AliasT` is
                // `typedef Common#(int,"alpha") AliasT;`): the bare `new()`
                // carries no type_args, but current_spec holds the bindings.
                if is_leaf && arg_map.get(pname).is_none() {
                    let spec_matches = self
                        .current_spec
                        .as_ref()
                        .is_some_and(|(b, _)| *b == class_name_owned);
                    if spec_matches {
                        if let Some(v) = self.resolve_value_param_from_spec(pname) {
                            instance.properties.insert(pname.clone(), v);
                            continue;
                        }
                    }
                }
                // An ancestor's parameter supplied by the `extends B#(..)`
                // clause wins over its declared default.
                if !is_leaf {
                    if let Some(v) = ancestor_value_args
                        .get(&cdef.name)
                        .and_then(|m| m.get(pname))
                    {
                        instance.properties.insert(pname.clone(), v.clone());
                        continue;
                    }
                }
                if let Some(a) = arg_map.get(pname).filter(|_| is_leaf) {
                    let v = self.eval_expr(a);
                    instance.properties.insert(pname.clone(), v);
                } else if let Some(e) = pdefault {
                    let v = if matches!(e.kind, ExprKind::Number(_) | ExprKind::StringLiteral(_)) {
                        self.eval_expr(e)
                    } else {
                        self.eval_param_default(
                            e,
                            &classes_to_init[ci..],
                            &instance,
                            &computed_spec,
                        )
                    };
                    instance.properties.insert(pname.clone(), v);
                }
            }
        }
        // §8.25: a property whose packed range references a class PARAMETER was
        // seeded from the class's elaborated signature — that is, at the
        // class's DEFAULT parameter value, so `bit [W-1:0] pw` in a `P#(16)`
        // came out 8 bits wide. A later WHOLE assignment re-widened it, which
        // is why this only showed as silent data loss on bit- and part-select
        // writes above the default width (and a wrong `$bits` until the first
        // whole write). The bindings this instance actually carries are only
        // complete now, after the loop above, so re-resolve those ranges here.
        // With no parameter anywhere on the chain the bindings gathered below
        // are always empty and every property is skipped.
        let chain_has_params = classes_to_init.iter().any(|c| !c.param_defaults.is_empty());
        for cdef in classes_to_init.iter().filter(|_| chain_has_params) {
            for (prop, dt) in &cdef.property_types {
                let dt = self.class_prop_decl_type(cdef, prop).unwrap_or(dt);
                let dims = match dt {
                    DataType::IntegerVector { dimensions, .. } if !dimensions.is_empty() => {
                        dimensions
                    }
                    DataType::Implicit { dimensions, .. } if !dimensions.is_empty() => dimensions,
                    _ => continue,
                };
                // Only ranges that are NOT constant on their own can move with
                // a parameter; a literal range already elaborated correctly.
                let param_dependent = dims.iter().any(|d| {
                    matches!(d, crate::ast::types::PackedDimension::Range { left, right, .. }
                        if super::elaborate::const_eval_i64_with_params(left, None).is_none()
                            || super::elaborate::const_eval_i64_with_params(right, None).is_none())
                });
                if !param_dependent {
                    continue;
                }
                // Bindings visible to this declaring class, leaf first so a
                // derived class's parameter shadows an ancestor's.
                let mut params: HashMap<String, Value> = HashMap::default();
                let mut cur = Some(cdef.name.clone());
                let mut seen: HashSet<String> = HashSet::default();
                while let Some(cn) = cur {
                    if !seen.insert(cn.clone()) {
                        break;
                    }
                    let Some(c) = self.module.classes.get(&cn) else {
                        break;
                    };
                    for (pname, _) in &c.param_defaults {
                        if let Some(v) = instance.properties.get(pname) {
                            params.entry(pname.clone()).or_insert_with(|| v.clone());
                        }
                    }
                    cur = c.extends.clone();
                }
                if params.is_empty() {
                    continue;
                }
                let w = super::elaborate::resolve_type_width(
                    dt,
                    Some(&params),
                    Some(&self.module.typedefs),
                );
                if w == 0 {
                    continue;
                }
                if let Some(v) = instance.properties.get_mut(prop) {
                    if !v.is_real && v.width != w {
                        *v = v.clone().resize(w);
                    }
                }
            }
        }
        // Register per-instance associative-array members under an
        // instance-scoped name `<handle>#<member>` so the existing
        // signal-keyed assoc machinery gives each instance its own map.
        for cdef in classes_to_init {
            for (prop, &is_str) in &cdef.assoc_properties {
                self.module
                    .associative_arrays
                    .insert(format!("{}#{}", handle, prop), is_str);
            }
            // §8.25: a key sized by a class parameter takes this object's
            // specialization, not the default `assoc_index_props` holds.
            for (prop, kdt) in &cdef.assoc_key_decl_types {
                if let Some(kw) = self.param_sized_key_width(kdt, &instance, &classes_to_init) {
                    let signed = cdef.assoc_index_props.get(prop).is_some_and(|&(_, s)| s);
                    self.module
                        .assoc_index_widths
                        .insert(format!("{}#{}", handle, prop), (kw, signed));
                }
            }
            // Per-instance queue / dynamic-array members. Register the instance-
            // scoped name `<handle>#<member>` in the array tables (mirroring the
            // package/module array path) so push_back/size/index/foreach resolve
            // to independent storage with size starting at 0.
            for (prop, &(width, max)) in &cdef.queue_properties {
                let scoped = format!("{}#{}", handle, prop);
                if cdef.property_types.get(prop).is_some_and(|dt| {
                    matches!(
                        dt,
                        DataType::IntegerAtom { .. } | DataType::IntegerVector { .. }
                    ) && super::elaborate::is_type_signed_resolved(dt, &self.module.typedef_types)
                }) {
                    self.signed_class_colls.insert(scoped.clone());
                }
                self.module.dynamic_arrays.insert(scoped.clone());
                // The object is not on the heap yet: its value parameters
                // are read from the instance being built.
                let mut params = self.module.parameters.clone();
                for (pn, _) in &cdef.param_defaults {
                    if let Some(v) = instance.properties.get(pn) {
                        params.insert(pn.clone(), v.clone());
                    }
                }
                self.register_dyn_member_shape(cdef, &params, prop, &scoped, width);
                if let Some(m) = max {
                    self.module.queue_max_sizes.insert(scoped.clone(), m);
                }
                self.set_queue_size(&scoped, 0);
            }
            // Per-instance fixed-size array members (`reg_t gpr[4]`): register
            // the instance-scoped name as a real fixed array and default every
            // element to 0 so index reads / foreach see a populated range.
            for (prop, &(lo, hi, width)) in &cdef.array_properties {
                let scoped = format!("{}#{}", handle, prop);
                // §8.25: sized by a value parameter — this object's size.
                let (lo, hi) = match cdef
                    .param_sized_props
                    .get(prop)
                    .and_then(|d| self.param_sized_shape(d, &instance, &classes_to_init))
                {
                    Some(sh) if sh.len() == 1 => sh[0],
                    _ => (lo, hi),
                };
                self.module.arrays.insert(scoped.clone(), (lo, hi, width));
                self.note_member_decl_dims(cdef, prop, &scoped);
                // §6.8, §7.4: each element starts at its type's default — x
                // for a 4-state element; an unpacked-struct element as its
                // member leaves.
                // An unpacked-struct element keeps its member cells in the
                // instance (`sta[1].f`), where member reads and writes go.
                let (fill, elem_su) = self.class_array_elem_fill(cdef, prop, width);
                for i in lo..=hi {
                    let elem = format!("{}[{}]", scoped, i);
                    if let Some(su) = &elem_su {
                        let mut cells = Vec::new();
                        self.struct_leaf_plain_defaults(
                            &format!("{}[{}]", prop, i),
                            su,
                            0,
                            &mut cells,
                        );
                        for (k, v) in cells {
                            instance.properties.insert(k, v);
                        }
                    }
                    self.signals.insert(elem.clone(), fill.clone());
                    self.widths.insert(elem, width);
                }
            }
            // Per-instance MULTI-dim fixed array members (`test_t foo[0:3][0:7]`):
            // register the full shape so nested index access and multi-var
            // foreach resolve, and default every element to 0 (a class-handle
            // element's default is null). Oversized shapes stay lazy.
            for (prop, (shape, width)) in &cdef.array_nd_properties {
                let scoped = format!("{}#{}", handle, prop);
                // §8.25: sized by a value parameter — this object's shape.
                let own_shape = cdef
                    .param_sized_props
                    .get(prop)
                    .and_then(|d| self.param_sized_shape(d, &instance, &classes_to_init))
                    .filter(|sh| sh.len() == shape.len());
                let shape = own_shape.as_ref().unwrap_or(shape);
                self.note_member_decl_dims(cdef, prop, &scoped);
                if shape.len() == 2 {
                    self.module
                        .arrays_2d
                        .insert(scoped.clone(), (shape[0], shape[1], *width));
                } else {
                    self.module
                        .arrays_nd
                        .insert(scoped.clone(), (shape.clone(), *width));
                }
                let total: i64 = shape.iter().map(|&(lo, hi)| (hi - lo + 1).max(0)).product();
                // §6.8, §7.4: x for a 4-state element type (see above).
                let (fill, elem_su) = self.class_array_elem_fill(cdef, prop, *width);
                if total > 0 && total <= 65536 {
                    let mut idx: Vec<i64> = shape.iter().map(|d| d.0).collect();
                    'fill: loop {
                        let mut name = scoped.clone();
                        for v in &idx {
                            name = format!("{}[{}]", name, v);
                        }
                        if let Some(su) = &elem_su {
                            let mut cells = Vec::new();
                            let cell = format!("{}{}", prop, &name[scoped.len()..]);
                            self.struct_leaf_plain_defaults(&cell, su, 0, &mut cells);
                            for (k, v) in cells {
                                instance.properties.insert(k, v);
                            }
                        }
                        self.widths.insert(name.clone(), *width);
                        self.signals.insert(name, fill.clone());
                        let mut k = shape.len();
                        loop {
                            if k == 0 {
                                break 'fill;
                            }
                            k -= 1;
                            idx[k] += 1;
                            if idx[k] <= shape[k].1 {
                                break;
                            }
                            idx[k] = shape[k].0;
                        }
                    }
                }
            }
        }
        // Record concrete bindings for the leaf class's TYPE parameters
        // (e.g. `T -> Base` for `Mk#(Base)`). A later unqualified `obj =
        // new()` whose declared type is one of these params resolves through
        // these bindings to construct the right class (see the `new`
        // dispatch). Type and value params may be INTERLEAVED
        // (e.g. `component_registry#(type T, string Tname)` — type first),
        // so look each type param up by NAME in `arg_map` (which maps args by
        // `param_order` slot, or by kind for the recovered named form §8.26).
        let spec_targets_this = self
            .current_spec
            .as_ref()
            .is_some_and(|(b, _)| *b == class_name_owned);
        for tp_name in class_def.type_param_names.iter() {
            let mut bound: Option<String> = None;
            if let Some(ta) = arg_map.get(tp_name) {
                if let Some(concrete) = Self::leaf_ident_name(ta) {
                    // The type arg may itself be a type PARAMETER of the enclosing
                    // parameterized context (`uvm_resource#(T)` where `T` is
                    // config_db's own param) — resolve it to the concrete type so
                    // the instance's binding is `T -> int`, not `T -> T`.
                    //
                    // A type arg that is a SPECIALIZATION of the ENCLOSING class's
                    // own parameter (`uvm_callback_iter#(special_comp#(N),...)` on
                    // a special_comp#(N) instance) carries a bare value-parameter
                    // name `N` in its `#(...)`. leaf_ident_name returns the raw
                    // `special_comp#(N)` string, so binding `T -> "special_comp#(N)"`
                    // captures N as literal text and every later `T`-param
                    // resolution inside the iterator's methods (`first()` calling
                    // `uvm_callbacks#(T,CB)::get_first`) ends up at the un-
                    // specialized `special_comp` cell — empty typewide queue,
                    // so `get_first`/`first()` returned null. Resolve the
                    // `#(N)` args through the ACTIVE specialization here, so the
                    // binding is the concrete `special_comp#(1)`.
                    let concrete = if concrete.contains('(')
                        && matches!(ta.kind, ExprKind::Specialization { .. })
                    {
                        self.expr_to_spec_fragment(ta).unwrap_or(concrete)
                    } else {
                        concrete
                    };
                    // §25.9: `virtual my_if#(T)` binds the enclosing context's
                    // `T` inside its own `#(...)`.
                    let concrete = match self.parse_vif_frag(&concrete) {
                        Some((iname, args, modport)) => {
                            let args = args
                                .iter()
                                .map(|a| {
                                    self.resolve_type_param_binding(a)
                                        .unwrap_or_else(|| a.clone())
                                })
                                .collect();
                            self.canonical_vif_frag(&iname, args, modport)
                        }
                        None => concrete,
                    };
                    bound = Some(
                        self.resolve_type_param_binding(&concrete)
                            .unwrap_or(concrete),
                    );
                }
            } else if spec_targets_this {
                // No explicit type_args carried this param, but the active
                // specialization (`current_spec`) targets THIS class —
                // resolve the type param from it. This covers
                // `C#(int)::type_id::create()` where the factory intercept
                // constructs `C` without forwarding the `#(int)` args: the
                // outer `eval_call` already set `current_spec = (C, "int")`,
                // so `resolve_type_param_binding("T")` yields `int`.
                bound = self.resolve_type_param_binding(tp_name);
            }
            // Only when the specialization SUPPLIED nothing for this parameter.
            // If an argument was given but could not be reduced to a type name,
            // the parameter is still explicitly specialized — substituting the
            // declared default there would bind the wrong type outright (and
            // then clamp values to it), which is worse than leaving it unbound.
            if bound.is_none() && !arg_map.contains_key(tp_name) {
                // §8.25: a type parameter the specialization does not supply
                // takes its DECLARED DEFAULT — `class box #(type CLASS_T =
                // simple_payload)` instantiated as `box#(.PAYLOAD_T(int))`
                // still binds `CLASS_T -> simple_payload`. Without this the
                // parameter stayed unbound and a property declared `CLASS_T o;`
                // was never constructed by `o = new()`, reading back x; the
                // same class only worked when the default was written out
                // explicitly in the `#(...)` list.
                if let Some((_, dflt)) = class_def
                    .type_param_defaults
                    .iter()
                    .find(|(n, _)| n == tp_name)
                {
                    let dflt = dflt.trim();
                    if !dflt.is_empty() {
                        // §8.25.4(ii): a default may reference a SIBLING type
                        // parameter of the same class (`uvm_sequencer#(type
                        // REQ, type RSP=REQ)`). Resolve it through THIS
                        // specialization's own concrete bindings — the arg
                        // map first, then a parameter already bound on the
                        // instance — so RSP binds REQ's concrete argument
                        // (e.g. simple_item), not the literal name "REQ";
                        // a $cast against the specialization then matches.
                        let resolved = if class_def.type_param_names.iter().any(|n| n == dflt) {
                            arg_map
                                .get(dflt)
                                .and_then(Self::leaf_ident_name)
                                .or_else(|| instance.type_bindings.get(dflt).cloned())
                                .or_else(|| self.resolve_type_param_binding(dflt))
                                .unwrap_or_else(|| dflt.to_string())
                        } else {
                            self.resolve_type_param_binding(dflt)
                                .unwrap_or_else(|| dflt.to_string())
                        };
                        bound = Some(resolved);
                    }
                }
            }
            if let Some(resolved) = bound {
                instance.type_bindings.insert(tp_name.clone(), resolved);
            }
        }
        // Capture the active specialization on the instance when it targets
        // THIS class, so a later VIRTUAL method call can restore it
        // (`current_spec` is otherwise lost once the constructing static
        // call returns). This is what lets a typedef-specialization singleton
        // — `static this_type m_inst; m_inst = new()` inside a static method
        // dispatched on `typedef Common#(int,"alpha") AliasT;` — answer
        // value-parameter lookups in later virtual calls.
        let active_spec = self.current_spec.as_ref().or(computed_spec.as_ref());
        if let Some((b, sig)) = active_spec {
            let c_base = class_name_owned
                .split('#')
                .next()
                .unwrap_or(&class_name_owned);
            if b == c_base {
                instance.spec = Some((b.clone(), sig.clone()));
            } else if self.class_extends(b, c_base) {
                if let Some(anc_sig) = self.ancestor_spec(b, sig, c_base) {
                    let anc_sig = self.canonicalize_spec_sig(c_base, &anc_sig);
                    instance.spec = Some((c_base.to_string(), anc_sig));
                }
            }
        }
        self.heap.push(Some(Box::new(instance)));
        // §8.25 / §6.20.2: the ELEMENT width of a fixed-array or queue member
        // follows this specialization too (`bit [W-1:0] a[2]`, `T q[$]`).
        // The per-instance storage registered above used the class table's
        // default-parameter width; now that the object carries its parameter
        // and type bindings, re-register any member whose width moved.
        let chain_is_param = chain_has_params
            || classes_to_init
                .iter()
                .any(|c| !c.type_param_names.is_empty());
        if chain_is_param {
            self.respec_member_elem_widths(handle, &classes_to_init);
        }
        // Per-instance TYPE-PARAM-BOUND collection members
        // (`#(type T=int) data;` instantiated with T bound to a queue/array
        // typedef such as `typedef int a_i[];`). These live only in
        // `cd.properties` generically and never reach `queue_properties` /
        // `array_properties`, so whole-assignment / size / index / `%p` paths
        // had no array table entry and treated `data` as a scalar. Register
        // `<handle>#member` as a dynamic array once this instance's type
        // bindings (set above) resolve the parameter to a concrete collection
        // type.
        let all_candidates: Vec<(usize, String)>;
        let candidates: &[(usize, String)] = match tpl.as_ref() {
            Some(t) => &t.coll_candidates,
            None => {
                let mut v = Vec::new();
                for (ci, cdef) in classes_to_init.iter().enumerate() {
                    for (prop, sig) in &cdef.properties {
                        if !(cdef.queue_properties.contains_key(prop)
                            || cdef.assoc_properties.contains_key(prop)
                            || cdef.array_properties.contains_key(prop)
                            || cdef.array_nd_properties.contains_key(prop)
                            || cdef.static_collections.iter().any(|(n, ..)| n == prop)
                            || sig.type_name.is_none())
                        {
                            v.push((ci, prop.clone()));
                        }
                    }
                }
                all_candidates = v;
                &all_candidates
            }
        };
        for (ci, prop) in candidates {
            let cdef = &classes_to_init[*ci];
            let Some(sig) = cdef.properties.get(prop) else {
                continue;
            };
            {
                if !self.prop_bound_collection(handle, &cdef.name, prop) {
                    continue;
                }
                let raw = sig.type_name.as_deref().unwrap_or(prop);
                let concrete = self
                    .heap
                    .get(handle)
                    .and_then(|o| o.as_ref())
                    .and_then(|i| i.type_bindings.get(raw).cloned())
                    .unwrap_or_else(|| raw.to_string());
                let w = self
                    .module
                    .typedef_elem_widths
                    .get(&concrete)
                    .copied()
                    .unwrap_or(sig.width)
                    .max(1);
                let scoped = format!("{}#{}", handle, prop);
                self.module.dynamic_arrays.insert(scoped.clone());
                self.module.arrays.insert(scoped.clone(), (0, 63, w));
                self.set_queue_size(&scoped, 0);
            }
        }
        // Re-evaluate scalar property initializers against the live parameter
        // table and instance context, before the constructor runs (SV applies
        // member initializers prior to `new`'s body). `elaborate_class`
        // computed them with no params, so defaults like `= NUM_HARTS` /
        // `= XLEN` would otherwise be 0. Base-class first so derived overrides.
        self.this_stack.push(Some(handle));
        // §7.2.2: an unpacked-struct property starts with its members'
        // declared defaults (its own initializer, if any, runs below and
        // overrides them). The member cells used to stay absent and read 0.
        if let Some(t) = tpl.as_ref().filter(|t| !t.struct_cells.is_empty()) {
            if let Some(Some(inst)) = self.heap.get_mut(handle) {
                for (key, v) in &t.struct_cells {
                    inst.properties.insert(key.clone(), v.clone());
                }
            }
        }
        {
            let mut defs = tpl
                .as_ref()
                .map(|t| t.struct_defaults.clone())
                .unwrap_or_default();
            // Fixed struct arrays use heap member cells too. Their bounds may
            // depend on this object's parameters, not the cached class shape.
            for cdef in classes_to_init {
                for prop in cdef
                    .array_properties
                    .keys()
                    .chain(cdef.array_nd_properties.keys())
                {
                    let Some(su) = self
                        .class_prop_struct(handle, prop)
                        .filter(Self::spreads_member_wise)
                    else {
                        continue;
                    };
                    let shape = if let Some(&(lo, hi, _)) = cdef.array_properties.get(prop) {
                        vec![self.inst_fixed_bounds(cdef, handle, prop, (lo, hi))]
                    } else {
                        self.inst_nd_shape(cdef, handle, prop)
                            .unwrap_or_else(|| cdef.array_nd_properties[prop].0.clone())
                    };
                    for suffix in Self::nd_index_suffixes(&shape) {
                        self.struct_member_defaults(
                            &format!("{}{}", prop, suffix),
                            &su,
                            0,
                            &mut defs,
                        );
                    }
                }
            }
            for (key, init, mdt) in defs {
                let w = super::elaborate::resolve_type_width(
                    &mdt,
                    Some(&self.module.parameters),
                    Some(&self.module.typedefs),
                )
                .max(1);
                let raw = self.eval_expr_ctx(&init, w);
                let two_state =
                    super::elaborate::is_type_two_state_resolved(&mdt, &self.module.typedef_types);
                let is_real = super::elaborate::is_type_real(&mdt);
                let signed = self.type_is_signed_concrete(&mdt);
                let v = Self::fit_local_decl_init(raw, w, &mdt, two_state, is_real, signed);
                if let Some(Some(inst)) = self.heap.get_mut(handle) {
                    inst.properties.insert(key, v);
                }
            }
        }
        self.pending_class_initializers
            .insert(handle, classes_to_init.to_vec());
        self.this_stack.pop();
        let saved_spec = self.clone_spec_pooled();
        if computed_spec.is_some() {
            self.current_spec = computed_spec.clone();
        }
        self.exec_method_call(handle, "new", args);
        // A declaration without an executable constructor still initializes
        // its fields. Normal constructor dispatch drains the pending classes.
        while let Some(class) = self
            .pending_class_initializers
            .get(&handle)
            .and_then(|classes| classes.last())
            .map(|cd| cd.name.clone())
        {
            self.initialize_pending_class_properties(handle, &class);
        }
        self.restore_spec_pooled(saved_spec);
        Value::from_u64(handle as u64, 32)
    }

    /// §8.4: a method call on a null handle. At the top level (a module or
    /// testbench process body) the run aborts, like the reference simulator;
    /// inside class code (deep library code, where an emulation gap can leave
    /// a handle null) the call is reported once per site and skipped, the
    /// policy `blocking_call_on_null_receiver` and the property-read path
    /// apply.
    pub(super) fn report_null_handle_call(&mut self, method_name: &str) {
        let in_method = self.this_stack.last().copied().flatten().is_some()
            || self.class_context_stack.last().cloned().flatten().is_some()
            || !self.local_stack.is_empty();
        let m = format!(
            "[xezim][error] null handle at time {} — method `{}` called on a null \
             class handle (§8.4); the call runs nothing{}",
            self.time,
            method_name,
            if in_method { "" } else { ", aborting" }
        );
        if in_method {
            if self.null_call_reported.insert(method_name.to_string()) {
                self.record_output(m.clone());
                self.stdout_writeln(&m);
            }
            return;
        }
        self.record_output(m.clone());
        self.stdout_writeln(&m);
        self.abort_run();
    }

    pub(super) fn exec_method_call(
        &mut self,
        handle: usize,
        method_name: &str,
        args: &[Expression],
    ) -> Value {
        // Ctor-time binding: a call on the object UNDER CONSTRUCTION starts
        // its search at the constructing class, not the runtime leaf.
        if method_name != "new" {
            let ctor_class = match self.ctor_class_stack.last() {
                Some((ch, cc)) if *ch == handle && self.class_has_method(cc, method_name) => {
                    Some(cc.clone())
                }
                _ => None,
            };
            if let Some(cc) = ctor_class {
                return self.exec_method_in_class_hierarchy(handle, &cc, method_name, args);
            }
        }
        // IEEE 1800-2023 §9.7 process class: kill/await/suspend/resume on a
        // process handle (>= PROCESS_HANDLE_BASE). These are intercepted here,
        // before any user-class dispatch.
        if let Some(pid) = Self::proc_handle_to_pid(handle as u64) {
            match method_name {
                "status" => {
                    return Value::from_u64(self.proc_status(pid) as u64, 32);
                }
                "kill" => {
                    self.proc_kill(pid);
                    return Value::zero(32);
                }
                "await" => {
                    // Block the calling process until `pid` terminates.
                    // The continuation is the remaining statement stream —
                    // captured by the CALLER (run_process_stmts) which passes
                    // an empty args slice; the real continuation is handled
                    // at the StatementKind::MethodCall dispatch site below.
                    // For the method-call-via-exec_method_call path (used by
                    // inline task calls), we mark the suspension here.
                    //
                    // NOTE: await() needs the caller's continuation, which
                    // exec_method_call doesn't have. The actual blocking is
                    // done at the call site (see the MethodCall dispatch in
                    // run_process_stmts). If we reach here, it means await()
                    // was called in a context without continuation capture
                    // — treat as a no-op (target already terminated).
                    return Value::zero(32);
                }
                "suspend" => {
                    self.proc_suspend(pid);
                    return Value::zero(32);
                }
                "resume" => {
                    self.proc_resume(pid);
                    return Value::zero(32);
                }
                _ => {} // fall through to srandom/randstate etc.
            }
        }
        // IEEE 1800-2023 §18.13/§18.14 random stability. `srandom(seed)`,
        // `get_randstate()` and `set_randstate(s)` are built-ins of BOTH the
        // §9.7 `process` class (`process::self().srandom(...)` — the token
        // handle is >= PROCESS_HANDLE_BASE) and of every class object
        // (`obj.srandom(...)`, `this.srandom(...)`). They are intercepted here,
        // ahead of user-method dispatch, unless the class actually defines an
        // override of its own.
        if matches!(method_name, "srandom" | "get_randstate" | "set_randstate")
            && !self
                .heap
                .get(handle)
                .and_then(|o| o.as_ref())
                .map(|i| self.class_has_method(&i.class_name, method_name))
                .unwrap_or(false)
        {
            if let Some(v) = self.exec_rand_state_method(handle, method_name, args) {
                return v;
            }
        }
        if method_name == "randomize" {
            // §18.11: `obj.randomize(null)` is the in-line constraint CHECKER —
            // it randomizes nothing and just reports whether the object's
            // current values satisfy its active constraints.
            if Self::is_randomize_check_args(args) {
                return self.exec_randomize_check(handle, &[]);
            }
            // §18.11: a member subset — `obj.randomize(a, b)`. Each argument
            // names a property of the object, so restrict the solve to those
            // and leave the rest as state.
            let subset: Option<HashSet<String>> = if args.is_empty() {
                None
            } else {
                let mut names: HashSet<String> = HashSet::default();
                for a in args {
                    match &a.kind {
                        ExprKind::Ident(h) if h.path.len() == 1 => {
                            names.insert(h.path[0].name.name.clone());
                        }
                        _ => return self.exec_randomize(handle),
                    }
                }
                Some(names)
            };
            let saved = std::mem::replace(&mut self.randomize_subset, subset);
            let r = self.exec_randomize(handle);
            self.randomize_subset = saved;
            return r;
        }

        // Built-in mailbox / semaphore methods
        if self.mailboxes.contains_key(&handle) {
            match method_name {
                "put" => {
                    if let Some(arg) = args.first() {
                        // An UNPACKED struct has no single value to queue;
                        // stash it member-wise and queue its shadow id.
                        let v = self.mbx_message(arg);
                        // LRM §15.4.2: if a blocking get is waiting on this
                        // mailbox, hand the value directly to the waiter
                        // (skipping the queue) and reschedule its
                        // continuation at the current time.
                        // Push, then drain: a single-waiter handoff left a
                        // peek-woken item in the box while a GET waiter on the
                        // same mailbox stayed parked forever (peek+get pair on
                        // one FIFO — the sequencer get_next_item/item_done
                        // shape). The drain keeps delivering while items and
                        // waiters remain.
                        self.mailboxes.get_mut(&handle).unwrap().push_back(v);
                        self.drain_mailbox_get_waiters(handle);
                    }
                    return Value::zero(32);
                }
                "get" | "peek" => {
                    let val = if method_name == "get" {
                        self.mailboxes.get_mut(&handle).and_then(|q| q.pop_front())
                    } else {
                        self.mailboxes.get(&handle).and_then(|q| q.front().cloned())
                    };
                    if val.is_none() {
                        // A blocking get/peek reached the SYNCHRONOUS
                        // dispatcher with the box empty: this context cannot
                        // suspend, so the call returns without a value —
                        // silently wrong. The suspend-aware inliner should
                        // have caught the call chain; warn loudly (once) so
                        // the gap is visible instead of reading as data
                        // corruption.
                        static WARNED: std::sync::atomic::AtomicBool =
                            std::sync::atomic::AtomicBool::new(false);
                        if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                            eprintln!(
                                "[xezim][warning] blocking mailbox {}() on an EMPTY mailbox reached a synchronous \
                                 context that cannot suspend (t={}); the call returns without a value. \
                                 This usually means a blocking call chain was not inlined — please report.",
                                method_name, self.time
                            );
                        }
                    }
                    if let (Some(v), Some(arg)) = (&val, args.first()) {
                        self.mbx_assign_delivered(arg, v, method_name == "get");
                    }
                    // A consuming `get` frees a slot — admit a parked producer.
                    if method_name == "get" {
                        self.admit_mailbox_put_waiter(handle);
                    }
                    return Value::zero(32);
                }
                "try_put" => {
                    if let Some(arg) = args.first() {
                        // §15.4.1: a bounded mailbox that is full rejects try_put
                        // (returns 0). A full box has no parked get-waiter (those
                        // park only on empty), so len>=bound is the whole test.
                        let bound = self.mailbox_bound.get(&handle).copied().unwrap_or(0);
                        if bound > 0 {
                            let len = self.mailboxes.get(&handle).map(|q| q.len()).unwrap_or(0);
                            if len >= bound {
                                return Value::zero(32);
                            }
                        }
                        let v = self.mbx_message(arg);
                        // Push-then-drain — see the blocking `put` arm.
                        self.mailboxes.get_mut(&handle).unwrap().push_back(v);
                        self.drain_mailbox_get_waiters(handle);
                    }
                    return Value::from_u64(1, 32);
                }
                "try_get" | "try_peek" => {
                    let val = if method_name == "try_get" {
                        self.mailboxes.get_mut(&handle).and_then(|q| q.pop_front())
                    } else {
                        self.mailboxes.get(&handle).and_then(|q| q.front().cloned())
                    };
                    if let (Some(v), Some(arg)) = (&val, args.first()) {
                        self.mbx_assign_delivered(arg, v, method_name == "try_get");
                        if method_name == "try_get" {
                            self.admit_mailbox_put_waiter(handle);
                        }
                        return Value::from_u64(1, 32);
                    }
                    return Value::zero(32);
                }
                "num" => {
                    let n = self.mailboxes.get(&handle).map(|q| q.len()).unwrap_or(0);
                    return Self::int_count(n as u64);
                }
                _ => {}
            }
        }
        if self.semaphores.contains_key(&handle) {
            match method_name {
                "put" => {
                    let n = args
                        .first()
                        .map(|a| self.eval_expr(a).to_u64().unwrap_or(1))
                        .unwrap_or(1) as i64;
                    *self.semaphores.get_mut(&handle).unwrap() += n;
                    self.wake_semaphore_waiters(handle);
                    return Value::zero(32);
                }
                "get" => {
                    let n = args
                        .first()
                        .map(|a| self.eval_expr(a).to_u64().unwrap_or(1))
                        .unwrap_or(1) as i64;
                    let count = self.semaphores.get_mut(&handle).unwrap();
                    if *count >= n {
                        *count -= n;
                    }
                    return Value::zero(32);
                }
                "try_get" => {
                    let n = args
                        .first()
                        .map(|a| self.eval_expr(a).to_u64().unwrap_or(1))
                        .unwrap_or(1) as i64;
                    let count = self.semaphores.get_mut(&handle).unwrap();
                    if *count >= n {
                        *count -= n;
                        return Value::from_u64(1, 32);
                    }
                    return Value::zero(32);
                }
                _ => {}
            }
        }
        let class_name = if let Some(Some(inst)) = self.heap.get(handle) {
            inst.class_name.clone()
        } else {
            // §8.4: calling a method through a null handle dereferences
            // null; the reference simulator reports it and aborts. This
            // ran nothing and returned 0 silently, so a test whose handle
            // was never constructed printed no trace at all. A static
            // method needs no object (§8.7) — the receiver's declared
            // class dispatches it earlier; any static method of that name
            // keeps the silent path for the shapes that still reach here.
            if !self
                .module
                .classes
                .values()
                .any(|cd| cd.static_methods.contains(method_name))
            {
                self.report_null_handle_call(method_name);
            }
            return Value::zero(32);
        };
        // LRM §8.7/§8.23: a STATIC method is resolved/stored separately from
        // instance methods and never dereferences `this`; a call through an
        // object handle (e.g. `req.get_type()`) must dispatch statically, even
        // when the object is live. Dispatch here (the common funnel) so every
        // instance-path route resolves static methods correctly.
        if self.is_static_method(&class_name, method_name) {
            if let Some(res) =
                self.exec_static_method_via_object(&class_name, method_name, args, handle)
            {
                return res;
            }
        }
        if args.len() <= 1 && !self.compiled_method_plans.is_empty() && compiled_methods_enabled() {
            if let Some(md) = self.method_defining_class(&class_name, method_name) {
                let spec_id = self.instance_spec_id(handle);
                if let Some(entry) =
                    self.vm_fast_entry_for(&md.class, method_name, &md.method, spec_id)
                {
                    match &entry.accessor {
                        FastAccessor::Getter(field) if args.is_empty() => {
                            let value = self
                                .heap
                                .get(handle)
                                .and_then(|o| o.as_ref())
                                .and_then(|inst| inst.properties.get(field.as_ref()))
                                .cloned()
                                .unwrap_or_else(|| Value::zero(1));
                            let mut result = value;
                            if result.is_real && !entry.is_class_result && !entry.is_void_result {
                                result = Self::compiled_ret_from_real(&result, entry.result_width);
                            }
                            if entry.result_width > 0 && result.width != entry.result_width {
                                result = result.resize_for_assign(entry.result_width);
                            }
                            if !entry.is_class_result && !entry.is_void_result && !result.is_real {
                                result.is_signed = entry.result_signed;
                            }
                            return result;
                        }
                        FastAccessor::Setter(field) if args.len() == 1 => {
                            let arg_val = self.eval_expr(&args[0]);
                            let fitted = {
                                let mut v = arg_val;
                                if let Some(w) = entry.coerce.first().map(|c| c.1)
                                    && w > 0
                                    && v.width != w
                                {
                                    v = v.resize_for_assign(w);
                                }
                                self.fit_class_prop(handle, field.as_ref(), &v)
                            };
                            if let Some(o) = self.heap.get_mut(handle).and_then(|o| o.as_mut()) {
                                o.properties.insert(field.to_string(), fitted);
                            }
                            return Value::zero(32);
                        }
                        _ => {}
                    }
                }
            }
        }
        self.exec_method_in_class_hierarchy(handle, &class_name, method_name, args)
    }
}
