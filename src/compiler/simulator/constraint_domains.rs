//! Simulator constraint domains.

use super::*;

impl Simulator {
    /// `v` sized and signed for the static cell `key` (an initializer or a
    /// store is an assignment to it, §10.7; an unsized fill takes the
    /// cell's width, §5.7.1).
    pub(super) fn fit_static_cell(&self, key: &str, prop: &str, v: Value) -> Value {
        if v.is_real {
            return v;
        }
        match self.static_cell_width(key, prop) {
            Some((w, signed)) => {
                let mut out = if w != v.width {
                    v.resize_for_assign(w)
                } else {
                    v
                };
                out.is_signed = signed;
                out
            }
            _ => v,
        }
    }

    /// The concrete type a method LOCAL declared with a bare type-parameter
    /// name (`T tmp;`) denotes: the binding of the executing method's class
    /// parameter on `this` — carried down the `extends` chain when the
    /// method's class is an ancestor bound by `extends C #(byte)` — else the
    /// active specialization's. `None` for any other type.
    /// §8.25.1/§6.18: the concrete type a class-scope typedef names when it
    /// is reached through a SPECIALIZATION the static tables cannot see — a
    /// type parameter or a typedef of a specialization used as the scope
    /// (`R::elem_t`, `h_t::vec_t`), directly or through a class-local alias
    /// (`typedef R::elem_t et_t;`). The elaborated tables hold the DEFAULT
    /// specialization only, so `typedef T elem_t` read 32 bits for every
    /// binding. Returns the concrete form `local_type_param_concrete` gives
    /// (`byte`, `logic[5:0]`), or None for any other type.
    pub(super) fn scoped_typedef_concrete(&mut self, dt: &DataType) -> Option<String> {
        // Cheap reject first: this runs for every local declaration.
        let DataType::TypeReference { name, .. } = dt else {
            return None;
        };
        if !name.has_scope()
            && !self
                .class_member_names()
                .scoped_typedefs
                .contains(&name.name.name)
        {
            return None;
        }
        let mut cur = dt.clone();
        for _ in 0..8 {
            let DataType::TypeReference {
                name,
                dimensions,
                type_args,
                ..
            } = &cur
            else {
                return None;
            };
            if !dimensions.is_empty() || !type_args.is_empty() {
                return None;
            }
            if name.scopes.is_empty() {
                // A class-local (or module) alias of such a reference.
                if self.module.classes.contains_key(&name.name.name)
                    || !self
                        .class_member_names()
                        .scoped_typedefs
                        .contains(&name.name.name)
                {
                    return None;
                }
                cur = self.lookup_typedef_target(&name.name.name)?;
                continue;
            }
            let link = name.scopes.last()?;
            if !link.type_args.is_empty() {
                return None;
            }
            let (base, Some(sig)) = self.resolve_class_scope_prefix(&link.name.name)? else {
                return None;
            };
            let leaf = name.name.name.clone();
            let saved = self.current_spec.replace((base.clone(), sig));
            let out = self.class_typedef_concrete_in_spec(&base, &leaf);
            self.current_spec = saved;
            return out;
        }
        None
    }

    /// `scoped_typedef_concrete` with `base`'s specialization active: the
    /// concrete form of `base`'s (or an ancestor's) typedef `leaf`.
    pub(super) fn class_typedef_concrete_in_spec(
        &mut self,
        base: &str,
        leaf: &str,
    ) -> Option<String> {
        use crate::ast::types::{IntegerAtomType as A, IntegerVectorType as V, Signing};
        let mut c = Some(base.to_string());
        let mut target = None;
        while let Some(cn) = c {
            let cd = self.module.classes.get(&cn)?;
            if let Some(t) = cd.typedef_targets.get(leaf) {
                target = Some(t.clone());
                break;
            }
            c = cd.extends.clone();
        }
        match target? {
            DataType::TypeReference {
                name,
                dimensions,
                type_args,
                ..
            } if dimensions.is_empty() && type_args.is_empty() && !name.has_scope() => self
                .resolve_type_param_binding(&name.name.name)
                .map(|b| b.trim().to_string()),
            DataType::IntegerAtom { kind, signing, .. } => {
                let k = match kind {
                    A::Byte => "byte",
                    A::ShortInt => "shortint",
                    A::Int => "int",
                    A::LongInt => "longint",
                    A::Integer => "integer",
                    A::Time => "time",
                };
                // An explicitly `unsigned` atom has no keyword form here.
                (!matches!(signing, Some(Signing::Unsigned))).then(|| k.to_string())
            }
            DataType::IntegerVector {
                kind,
                signing,
                dimensions,
                ..
            } if !dimensions.is_empty() => {
                let spec_params = self.params_with_class_spec();
                let params = spec_params.as_ref().unwrap_or(&self.module.parameters);
                let mut s = String::from(match kind {
                    V::Bit => "bit",
                    V::Logic => "logic",
                    V::Reg => "reg",
                });
                if matches!(signing, Some(Signing::Signed)) {
                    s.push_str(" signed");
                }
                for d in &dimensions {
                    let crate::ast::types::PackedDimension::Range { left, right, .. } = d else {
                        return None;
                    };
                    let l = super::elaborate::const_eval_i64_with_params(left, Some(params))?;
                    let r = super::elaborate::const_eval_i64_with_params(right, Some(params))?;
                    s.push_str(&format!("[{l}:{r}]"));
                }
                Some(s)
            }
            _ => None,
        }
    }

    pub(super) fn local_type_param_concrete(&self, dt: &DataType) -> Option<String> {
        let ctx = self.class_context_stack.last()?.as_deref()?;
        self.type_param_concrete_in(dt, ctx)
    }

    /// `local_type_param_concrete` for a method of class `ctx`.
    pub(super) fn type_param_concrete_in(&self, dt: &DataType, ctx: &str) -> Option<String> {
        let DataType::TypeReference {
            name,
            dimensions,
            type_args,
            ..
        } = dt
        else {
            return None;
        };
        if !dimensions.is_empty() || !type_args.is_empty() || name.has_scope() {
            return None;
        }
        let tn = name.name.name.as_str();
        // A class or a typedef name (the common case) is no parameter.
        if !self.class_member_names().params.contains(tn)
            || self.module.classes.contains_key(tn)
            || self.module.typedefs.contains_key(tn)
            || self.module.typedef_types.contains_key(tn)
        {
            return None;
        }
        // The class declaring `tn` as a type parameter, at or above the
        // method's class.
        let mut decl: Option<&str> = None;
        let mut cur: Option<&str> = Some(ctx);
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
        let decl = decl?;
        if let Some(h) = self.this_stack.last().copied().flatten() {
            if let Some(first) = self
                .heap
                .get(h)
                .and_then(|o| o.as_ref())
                .and_then(|i| Some((i, self.module.classes.get(i.class_name.as_str())?)))
            {
                let (inst, first) = (first.0, first.1.as_ref());
                // On the stack: this runs for every `T`-typed local.
                let mut buf: [&super::elaborate::ElaboratedClass; 32] = [first; 32];
                let mut n = 0usize;
                let mut cur: Option<&super::elaborate::ElaboratedClass> = Some(first);
                while let Some(cd) = cur {
                    if n == buf.len() {
                        break;
                    }
                    buf[n] = cd;
                    n += 1;
                    let chain = &buf[..n];
                    if cd.name == decl || self.class_origin(&cd.name) == decl {
                        let k = n - 1;
                        if k == 0 {
                            if let Some(b) = inst.type_bindings.get(tn) {
                                return Some(b.clone());
                            }
                            break;
                        }
                        if let Some(b) =
                            Self::carried_type_binding(&inst.type_bindings, chain, k, tn)
                        {
                            if inst.type_bindings.get(&b).is_none()
                                && chain[0].type_param_names.iter().any(|t| *t == b)
                            {
                                return None;
                            }
                            return Some(b);
                        }
                        break;
                    }
                    cur = cd
                        .extends
                        .as_deref()
                        .and_then(|e| self.module.classes.get(e))
                        .map(|a| a.as_ref());
                }
            }
        }
        // No object (a static method): the active specialization, else the
        // one `ctx` binds its ancestor `decl` to (`class e extends
        // sb #(byte)` calling `e::f()`).
        self.resolve_type_param_binding(tn).or_else(|| {
            (ctx != decl)
                .then(|| self.static_receiver_spec(ctx, decl))
                .flatten()
                .and_then(|sp| self.resolve_type_param_with(tn, &Some(sp)))
        })
    }

    /// Width of `dt` — a packed range sized by a class VALUE parameter
    /// (`function bit [W-1:0] f();`) — inside a method of the executing
    /// class with no object, i.e. a static method (§8.25): the active
    /// specialization's values, else those the class binds through its
    /// `extends`, else the declared defaults. `None` for any other type,
    /// or outside a class.
    pub(super) fn class_param_sized_width(&mut self, dt: &DataType) -> Option<u32> {
        // Outside a class (every RTL function call) there is nothing to do.
        if !matches!(self.class_context_stack.last(), Some(Some(_))) {
            return None;
        }
        let dims = match dt {
            DataType::IntegerVector { dimensions, .. } | DataType::Implicit { dimensions, .. }
                if !dimensions.is_empty() =>
            {
                dimensions
            }
            _ => return None,
        };
        let range_ok = |d: &crate::ast::types::PackedDimension,
                        p: Option<&HashMap<String, Value>>| match d {
            crate::ast::types::PackedDimension::Range { left, right, .. } => {
                crate::elaborate::const_eval_i64_with_params(left, p).is_some()
                    && crate::elaborate::const_eval_i64_with_params(right, p).is_some()
            }
            _ => true,
        };
        // A literal range needs nothing here.
        if dims.iter().all(|d| range_ok(d, None)) {
            return None;
        }
        let ctx = self.class_context_stack.last().cloned().flatten()?;
        let spec = match &self.current_spec {
            Some((b, _)) if *b == ctx || self.class_extends(&ctx, b) => self.current_spec.clone(),
            _ => self.inherited_param_spec(&ctx),
        };
        let saved = std::mem::replace(&mut self.current_spec, spec);
        let from_spec = self.params_with_class_spec();
        self.current_spec = saved;
        let mut m = from_spec.unwrap_or_else(|| self.module.parameters.clone());
        let mut cur = Some(ctx);
        let mut guard = 0;
        while let Some(cn) = cur {
            guard += 1;
            if guard > 64 {
                break;
            }
            let Some(cd) = self.module.classes.get(&cn) else {
                break;
            };
            for (pname, pdefault) in &cd.param_defaults {
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
            cur = cd.extends.clone();
        }
        if !dims.iter().all(|d| range_ok(d, Some(&m))) {
            return None;
        }
        Some(crate::elaborate::resolve_type_width(
            dt,
            Some(&m),
            Some(&self.module.typedefs),
        ))
        .filter(|w| *w > 0)
    }

    /// Whether the type named by a type-parameter binding is 2-state.
    pub(super) fn concrete_type_two_state(&self, name: &str) -> bool {
        let base = name
            .strip_suffix(" unsigned")
            .or_else(|| name.strip_suffix(" signed"))
            .unwrap_or(name)
            .trim_end();
        match base {
            "byte" | "shortint" | "int" | "longint" | "bit" => return true,
            "logic" | "reg" | "integer" | "time" => return false,
            _ => {}
        }
        if vector_fragment_width(name).is_some() {
            return name.starts_with("bit");
        }
        self.module.typedef_types.get(name).is_some_and(|dt| {
            super::elaborate::is_type_two_state_resolved(dt, &self.module.typedef_types)
        })
    }

    /// The declared type of property `prop` of `cd`, with a CLASS-LOCAL
    /// typedef followed to its target (`typedef bit [W-1:0] word_t; word_t
    /// w;`): the target's packed range is what depends on the class
    /// parameters (§6.18, §8.25). A design-level typedef, or any other type,
    /// comes back as declared.
    pub(super) fn class_prop_decl_type<'a>(
        &'a self,
        cd: &'a super::elaborate::ElaboratedClass,
        prop: &str,
    ) -> Option<&'a DataType> {
        let mut dt = cd.property_types.get(prop)?;
        for _ in 0..8 {
            let DataType::TypeReference {
                name,
                dimensions,
                type_args,
                ..
            } = dt
            else {
                break;
            };
            if !dimensions.is_empty() || !type_args.is_empty() || name.has_scope() {
                break;
            }
            let tn = name.name.name.as_str();
            let mut hit: Option<&'a DataType> = None;
            let mut cur: Option<&'a super::elaborate::ElaboratedClass> = Some(cd);
            let mut guard = 0;
            while let Some(c) = cur {
                guard += 1;
                if guard > 64 {
                    break;
                }
                if let Some(t) = c.typedef_targets.get(tn) {
                    hit = Some(t);
                    break;
                }
                cur = c
                    .extends
                    .as_deref()
                    .and_then(|e| self.module.classes.get(e))
                    .map(|a| a.as_ref());
            }
            match hit {
                Some(t) => dt = t,
                None => break,
            }
        }
        Some(dt)
    }

    /// Whether `name` is a VALUE parameter (or body localparam) of the class
    /// whose scope is executing, or of one of its ancestors. A size cast
    /// `W'(5)` in a class (§6.24.1) names one of these: it lives on the
    /// object, not in the module parameter table, so the cast passed the
    /// operand through unsized.
    pub(super) fn class_value_param_in_scope(&self, name: &str) -> bool {
        let Some(Some(ctx)) = self.class_context_stack.last() else {
            return false;
        };
        let mut cur: Option<&str> = Some(ctx.as_str());
        let mut guard = 0;
        while let Some(cn) = cur {
            guard += 1;
            if guard > 64 {
                break;
            }
            let Some(cd) = self.module.classes.get(cn) else {
                break;
            };
            if cd.param_defaults.iter().any(|(n, _)| n == name) {
                return true;
            }
            cur = cd.extends.as_deref();
        }
        false
    }

    /// Signedness of the integral type a type parameter is bound to, by
    /// name: a built-in atom or a typedef. `None` when it cannot be told.
    pub(super) fn concrete_type_signed(&self, name: &str) -> Option<bool> {
        if let Some(base) = name.strip_suffix(" unsigned") {
            if atom_type_keyword_width(base.trim_end()).is_some()
                || vector_fragment_width(name).is_some()
            {
                return Some(false);
            }
        }
        if let Some(base) = name.strip_suffix(" signed") {
            if atom_type_keyword_width(base.trim_end()).is_some() {
                return Some(true);
            }
        }
        match name {
            "byte" | "shortint" | "int" | "longint" | "integer" => return Some(true),
            "bit" | "logic" | "reg" | "time" => return Some(false),
            _ => {}
        }
        if vector_fragment_width(name).is_some() {
            return Some(name.contains("signed") && !name.contains("unsigned"));
        }
        let dt = self.module.typedef_types.get(name)?;
        let dt = Self::resolve_type_ref(dt, &self.module.typedef_types);
        Some(super::elaborate::is_type_signed_resolved(
            &dt,
            &self.module.typedef_types,
        ))
    }

    /// Set property `name` on `handle` to `val`, returning whether the value
    /// actually changed (drives propagation fixpoint detection).
    ///
    /// §18.3: a solved value inherits the property's declared SIGNEDNESS. The
    /// solver builds values with `Value::from_u64` (unsigned), so without this
    /// stamp a `rand integer` came back unsigned and every later compare
    /// (`int_val < 0`, `int_val inside {[-100:100]}`, the satisfaction judge)
    /// silently read it as a huge positive number.
    /// §18.5.7: a `foreach` whose iterated variable is a PACKED vector — a
    /// state vector whose bits guard per-bit constraints on a rand property
    /// (`if (possible[i]) pattern[i] dist {…}; else pattern[i] == 0;`). Class
    /// collections do not cover it, so nothing applied the body. Iterate the
    /// bits with the index bound, evaluate the guards against the instance,
    /// and pin the constrained bits directly in the property map.
    pub(super) fn apply_packed_bit_foreach(
        &mut self,
        handle: usize,
        item: &ConstraintItem,
        rand_set: &HashSet<String>,
    ) {
        use crate::ast::decl::ConstraintItem as CI;
        match item {
            CI::Block(items) => {
                for it in items {
                    self.apply_packed_bit_foreach(handle, it, rand_set);
                }
            }
            CI::Soft(inner) => self.apply_packed_bit_foreach(handle, inner, rand_set),
            CI::Foreach {
                array,
                vars,
                item: body,
                ..
            } => {
                let Some(base) = Self::foreach_base_name(array) else {
                    return;
                };
                // A rand collection / fixed rand array is owned by the
                // collection pipeline; only a PACKED scalar property (or
                // plain state vector) takes this path.
                let cls = match self.heap.get(handle).and_then(|o| o.as_ref()) {
                    Some(i) => i.class_name.clone(),
                    None => return,
                };
                let is_fixed_array = self
                    .module
                    .classes
                    .get(&cls)
                    .is_some_and(|cd| cd.array_properties.contains_key(&base));
                if is_fixed_array {
                    return;
                }
                let w = match self
                    .heap
                    .get(handle)
                    .and_then(|o| o.as_ref())
                    .and_then(|i| i.properties.get(&base))
                    .map(|v| v.width)
                {
                    Some(w) if w > 1 => w,
                    _ => return,
                };
                let idx_var: Option<String> = vars
                    .first()
                    .and_then(|v| v.as_ref().map(|id| id.name.clone()));
                let body = (**body).clone();
                for i in 0..w as i64 {
                    let mut frame: HashMap<String, Value> = HashMap::default();
                    if let Some(iv) = &idx_var {
                        frame.insert(iv.clone(), Self::signed_loop_val(i));
                    }
                    self.push_local_frame(frame);
                    self.apply_bit_body(handle, &body, rand_set);
                    self.pop_local_frame();
                }
            }
            _ => {}
        }
    }

    /// One iteration of a packed-bit foreach body: guards evaluated against
    /// the instance, `prop[i] == K` and `prop[i] dist/inside {…}` pinned by
    /// writing the bit into the property.
    pub(super) fn apply_bit_body(
        &mut self,
        handle: usize,
        item: &ConstraintItem,
        rand_set: &HashSet<String>,
    ) {
        use crate::ast::decl::ConstraintItem as CI;
        match item {
            CI::Block(items) => {
                for it in items {
                    self.apply_bit_body(handle, it, rand_set);
                }
            }
            CI::Soft(inner) => self.apply_bit_body(handle, inner, rand_set),
            CI::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                if self.eval_expr(condition).is_true() {
                    let t = (**then_item).clone();
                    self.apply_bit_body(handle, &t, rand_set);
                } else if let Some(ei) = else_item {
                    let e = (**ei).clone();
                    self.apply_bit_body(handle, &e, rand_set);
                }
            }
            CI::Implication {
                condition,
                constraint,
                ..
            } => {
                if self.eval_expr(condition).is_true() {
                    let c = (**constraint).clone();
                    self.apply_bit_body(handle, &c, rand_set);
                }
            }
            CI::Expr(e) => {
                if let ExprKind::Binary {
                    op: BinaryOp::Eq,
                    left,
                    right,
                } = &e.kind
                {
                    if let Some((prop, bit)) = self.rand_prop_bit_select(left, rand_set) {
                        let v = self.eval_expr(right);
                        self.write_prop_bit(handle, &prop, bit, v.is_true());
                    } else if let Some((prop, bit)) = self.rand_prop_bit_select(right, rand_set) {
                        let v = self.eval_expr(left);
                        self.write_prop_bit(handle, &prop, bit, v.is_true());
                    }
                }
            }
            CI::Inside {
                expr,
                range,
                is_dist,
                dist_weights,
                ..
            } => {
                if let Some((prop, bit)) = self.rand_prop_bit_select(expr, rand_set) {
                    let picked = if *is_dist && !dist_weights.is_empty() {
                        self.pick_dist_value(None, range, dist_weights)
                    } else {
                        self.pick_from_ranges(range, 1, false)
                    };
                    if let Some(v) = picked {
                        self.write_prop_bit(handle, &prop, bit, v.is_true());
                    }
                }
            }
            _ => {}
        }
    }

    /// `Some((prop, bit_index))` when `e` is `prop[<const-evaluable idx>]` of a
    /// rand property.
    pub(super) fn rand_prop_bit_select(
        &mut self,
        e: &Expression,
        rand_set: &HashSet<String>,
    ) -> Option<(String, u32)> {
        let ExprKind::Index { expr, index } = &e.kind else {
            return None;
        };
        let ExprKind::Ident(h) = &expr.kind else {
            return None;
        };
        if h.path.len() != 1 || !h.path[0].selects.is_empty() {
            return None;
        }
        let prop = h.path[0].name.name.clone();
        if !rand_set.contains(&prop) {
            return None;
        }
        let idx = (**index).clone();
        let i = self.eval_expr(&idx).to_i64()?;
        u32::try_from(i).ok().map(|b| (prop, b))
    }

    /// Set bit `bit` of instance property `prop` to `on`.
    pub(super) fn write_prop_bit(&mut self, handle: usize, prop: &str, bit: u32, on: bool) {
        if let Some(Some(inst)) = self.heap.get_mut(handle) {
            if let Some(cur) = inst.properties.get(prop) {
                let mut v = cur.clone();
                if (bit as usize) < v.width as usize {
                    v.set_bit(
                        bit as usize,
                        if on { LogicBit::One } else { LogicBit::Zero },
                    );
                    inst.properties.insert(prop.to_string(), v);
                }
            }
        }
    }

    pub(super) fn set_prop_if_changed(&mut self, handle: usize, name: &str, val: Value) -> bool {
        // §11.6.1/§18.5: a value the solver forces is stored at the property's
        // declared width, like any assignment. Kept at the forcing
        // expression's width (`ma == 40'h... * tot - 1` into an `int
        // unsigned`, `b == 32'd300` into a `byte`, a `!=` re-pick drawn at the
        // other operand's 40 bits into a 1-bit `bit`), the check then compared
        // the oversized value and accepted an unsatisfiable set.
        let val = self.fit_class_prop(handle, name, &val);
        // §8.10: a STATIC property is one shared variable — a write through
        // `this.prop` / `handle.prop` must land in the class-static cell, not
        // create a per-instance copy (which then shadowed the static on every
        // later read from THAT instance while every other instance still saw
        // the old value).
        // Only genuine `static` members — param_defaults (class localparams)
        // also satisfy static_prop_key but are constants — and only when no
        // nearer instance property hides it.
        let static_of = match self.heap.get(handle) {
            Some(Some(inst)) if self.member_is_static_prop(&inst.class_name, name) => {
                Some((inst.class_name.clone(), inst.spec.clone()))
            }
            _ => None,
        };
        if let Some((cn, spec)) = static_of {
            if let Some(key) = self.static_prop_key_spec(&cn, name, spec.as_ref()) {
                let changed = self.class_statics.get(&key) != Some(&val);
                if changed {
                    self.class_statics.insert(key, val);
                }
                return changed;
            }
        }
        if let Some(Some(inst)) = self.heap.get_mut(handle) {
            if inst.properties.get(name).is_none_or(|cur| *cur != val) {
                inst.properties.insert(name.to_string(), val);
                return true;
            }
        }
        false
    }

    // ---------------------------------------------------------------------
    // IEEE 1800-2017 §18.3 / §18.5.12 — EXACT constraint arithmetic.
    //
    // The constraint solver and the satisfaction judge must agree on what an
    // expression means, and both must respect SIGNEDNESS. Two bugs came from
    // evaluating constraint operands the way the procedural interpreter does:
    //
    //  * §18.3 — `int_val inside {[-100:100]}` read its endpoints as u64, so
    //    `-100` became 0xFFFF_FF9C and sorted ABOVE 100: the range was empty,
    //    the draw was garbage, and later compares went unsigned.
    //  * §11.6.1/§18.5.12 — an operand evaluated at its SELF-determined width
    //    wraps: `(r_val - 8'd10) * (l_val + 8'd5)` computed at 8 bits is 0 for
    //    r_val=12, l_val=123 (256 mod 256), so a FALSE "satisfied" slipped
    //    past the generate-and-test backstop. The LRM extends every operand of
    //    a comparison to the context width max(w_lhs, w_rhs) FIRST.
    //
    // `exact_int` therefore evaluates a constraint operand into i128 without
    // truncating at each step. That is legitimate for +, -, * (they are ring
    // operations mod 2^W, so masking once at the context width is identical to
    // masking at every step); `/` and `%` are not, so their operands are
    // reduced to their own self-determined width first.
    // ---------------------------------------------------------------------

    /// Two's-complement bit pattern of `v` at `w` bits.
    pub(super) fn bits_at(v: i128, w: u32) -> u128 {
        if w >= 128 {
            v as u128
        } else {
            (v as u128) & ((1u128 << w) - 1)
        }
    }

    /// Reinterpret `v`'s `w`-bit pattern as signed / unsigned per `signed`.
    pub(super) fn as_ctx(v: i128, w: u32, signed: bool) -> i128 {
        let bits = Self::bits_at(v, w);
        if signed && w > 0 && w < 128 && (bits >> (w - 1)) & 1 == 1 {
            (bits as i128) - (1i128 << w)
        } else {
            bits as i128
        }
    }

    /// Exact integer view of a constraint operand: `(value, width, is_signed)`.
    /// `None` for anything the exact model cannot represent (X/Z, real, > 64
    /// bits) — every caller then falls back to the ordinary evaluator.
    pub(super) fn exact_int(&mut self, e: &Expression) -> Option<(i128, u32, bool)> {
        let mut leaves = Vec::new();
        let (w, s) = self.exact_leaves(e, &mut leaves)?;
        let v = self.exact_fold(e, &leaves, &mut 0, w, s)?;
        Some((v, w, s))
    }

    /// §11.8.2 — the leaves of a context-determined operand tree, evaluated
    /// once in fold order as `(bits, width, signed)`, and the tree's own
    /// `(width, signed)`. The expression's signedness governs how EVERY leaf
    /// is extended, so it has to be known before any operand is converted.
    pub(super) fn exact_leaves(
        &mut self,
        e: &Expression,
        out: &mut Vec<(u64, u32, bool)>,
    ) -> Option<(u32, bool)> {
        match &e.kind {
            ExprKind::Paren(inner) => self.exact_leaves(inner, out),
            ExprKind::Unary {
                op: UnaryOp::Plus | UnaryOp::Minus,
                operand,
            } => self.exact_leaves(operand, out),
            ExprKind::Unary { .. } => None,
            ExprKind::Binary { op, left, right } => match op {
                BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Mod
                | BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor => {
                    let (lw, ls) = self.exact_leaves(left, out)?;
                    let (rw, rs) = self.exact_leaves(right, out)?;
                    let w = lw.max(rw).max(1);
                    if w > 64 {
                        return None;
                    }
                    // §11.8.1: unsigned as soon as EITHER operand is.
                    Some((w, ls && rs))
                }
                // The shift amount is self-determined and never affects the
                // result's width or sign; `exact_fold` evaluates it on its own.
                BinaryOp::ShiftLeft
                | BinaryOp::ArithShiftLeft
                | BinaryOp::ShiftRight
                | BinaryOp::ArithShiftRight => self.exact_leaves(left, out),
                _ => None,
            },
            // Everything else (identifiers, `$signed({1'b0, l_val})`, packed
            // struct members, array elements, ternaries, function calls) goes
            // through the ordinary evaluator, which already gets width and
            // signedness right for a LEAF.
            _ => {
                let v = self.eval_expr(e);
                if v.is_real || v.has_xz() || v.width == 0 || v.width > 64 {
                    return None;
                }
                let raw = v.to_u64()?;
                out.push((raw, v.width, v.is_signed));
                Some((v.width, v.is_signed))
            }
        }
    }

    /// Fold an operand tree whose leaves `exact_leaves` collected, in the
    /// context `(w, s)`. Each leaf is extended per the CONTEXT signedness;
    /// intermediate +, -, * results stay exact (they are ring operations mod
    /// 2^w, so the caller masks once) — masking them at a subtree's own width
    /// turned `a + b + c == 6` over 4-bit operands into a mod-16 compare.
    pub(super) fn exact_fold(
        &mut self,
        e: &Expression,
        leaves: &[(u64, u32, bool)],
        k: &mut usize,
        w: u32,
        s: bool,
    ) -> Option<i128> {
        match &e.kind {
            ExprKind::Paren(inner) => self.exact_fold(inner, leaves, k, w, s),
            ExprKind::Unary { op, operand } => {
                let v = self.exact_fold(operand, leaves, k, w, s)?;
                Some(if *op == UnaryOp::Minus { -v } else { v })
            }
            ExprKind::Binary { op, left, right } => {
                let l = self.exact_fold(left, leaves, k, w, s)?;
                if matches!(
                    op,
                    BinaryOp::ShiftLeft
                        | BinaryOp::ArithShiftLeft
                        | BinaryOp::ShiftRight
                        | BinaryOp::ArithShiftRight
                ) {
                    // §11.4.10: the amount is always read as unsigned.
                    let (a, aw, _) = self.exact_int(right)?;
                    let r = Self::bits_at(a, aw);
                    if r >= 64 {
                        return None;
                    }
                    return Some(match op {
                        BinaryOp::ShiftRight => (Self::bits_at(l, w) >> r) as i128,
                        BinaryOp::ArithShiftRight => Self::as_ctx(l, w, s) >> r,
                        _ => Self::as_ctx(l, w, s).checked_shl(r as u32)?,
                    });
                }
                let r = self.exact_fold(right, leaves, k, w, s)?;
                Some(match op {
                    BinaryOp::Add => l.checked_add(r)?,
                    BinaryOp::Sub => l.checked_sub(r)?,
                    BinaryOp::Mul => l.checked_mul(r)?,
                    // Not ring operations: reduce to the context width first.
                    BinaryOp::Div | BinaryOp::Mod => {
                        let (a, b) = (Self::as_ctx(l, w, s), Self::as_ctx(r, w, s));
                        if b == 0 {
                            return None;
                        }
                        if *op == BinaryOp::Div { a / b } else { a % b }
                    }
                    BinaryOp::BitAnd => (Self::bits_at(l, w) & Self::bits_at(r, w)) as i128,
                    BinaryOp::BitOr => (Self::bits_at(l, w) | Self::bits_at(r, w)) as i128,
                    BinaryOp::BitXor => (Self::bits_at(l, w) ^ Self::bits_at(r, w)) as i128,
                    _ => return None,
                })
            }
            _ => {
                let (raw, lw, _) = *leaves.get(*k)?;
                *k += 1;
                Some(Self::as_ctx(raw as i128, lw, s))
            }
        }
    }

    /// §11.6.1 — judge a constraint comparison with both operands extended to
    /// the CONTEXT width max(w_lhs, w_rhs) and signedness. `None` when the
    /// exact model does not apply (X/Z, real, unsupported operator).
    pub(super) fn exact_cmp(
        &mut self,
        op: &BinaryOp,
        left: &Expression,
        right: &Expression,
    ) -> Option<bool> {
        // §5.7.1: an unsized `'0`/`'1` fills the width of its context, which
        // the integer leaves below cannot express (`y == '1` compared with
        // 1); the ordinary evaluation sizes it.
        let mut fill = false;
        for side in [left, right] {
            Self::walk_operands(side, &mut |x| {
                if matches!(
                    x.kind,
                    ExprKind::Number(crate::ast::expr::NumberLiteral::UnbasedUnsized(_))
                ) {
                    fill = true;
                }
            });
        }
        if fill {
            return None;
        }
        let (mut ll, mut rl) = (Vec::new(), Vec::new());
        let (lw, ls) = self.exact_leaves(left, &mut ll)?;
        let (rw, rs) = self.exact_leaves(right, &mut rl)?;
        let w = lw.max(rw).max(1);
        if w > 64 {
            return None;
        }
        let signed = ls && rs;
        let lv = self.exact_fold(left, &ll, &mut 0, w, signed)?;
        let rv = self.exact_fold(right, &rl, &mut 0, w, signed)?;
        let lb = Self::bits_at(lv, w);
        let rb = Self::bits_at(rv, w);
        Some(match op {
            BinaryOp::Eq | BinaryOp::CaseEq => lb == rb,
            BinaryOp::Neq | BinaryOp::CaseNeq => lb != rb,
            _ => {
                let (a, b) = if signed {
                    (Self::as_ctx(lv, w, true), Self::as_ctx(rv, w, true))
                } else {
                    (lb as i128, rb as i128)
                };
                match op {
                    BinaryOp::Lt => a < b,
                    BinaryOp::Leq => a <= b,
                    BinaryOp::Gt => a > b,
                    BinaryOp::Geq => a >= b,
                    _ => return None,
                }
            }
        })
    }

    /// §18.3 — the (lo, hi) pairs of an `inside`/`dist` range list, evaluated
    /// with exact SIGNED arithmetic so `[-100:100]` is the 201-value interval
    /// it denotes and not an empty one. A queue/array operand contributes each
    /// of its elements. `None` endpoints are skipped rather than coerced.
    pub(super) fn cons_ranges_i128(&mut self, ranges: &[ConstraintRange]) -> Vec<(i128, i128)> {
        self.cons_ranges_i128_at(ranges, 0)
    }

    /// `cons_ranges_i128` for an operand of `ctx_w` bits. §11.4.13/§11.8.2:
    /// each bound is computed at the wider of that width and its own, and
    /// wraps there — `exact_int` leaves `+`, `-`, `*` unreduced, so
    /// `-2147483648` (the negation of a 32-bit -2^31) read +2^31 and
    /// `[-2147483648:10]` was an empty range.
    pub(super) fn cons_ranges_i128_at(
        &mut self,
        ranges: &[ConstraintRange],
        ctx_w: u32,
    ) -> Vec<(i128, i128)> {
        self.cons_ranges_i128_ws(ranges, ctx_w).0
    }

    /// `cons_ranges_i128_at`, also giving the items' joint (width, signed)
    /// (§11.4.13: the widest item; signed only when every item is), or None
    /// for that when an item could not be read exactly.
    #[allow(clippy::type_complexity)]
    pub(super) fn cons_ranges_i128_ws(
        &mut self,
        ranges: &[ConstraintRange],
        ctx_w: u32,
    ) -> (Vec<(i128, i128)>, Option<(u32, bool)>) {
        struct Bound<'a> {
            expr: Option<&'a Expression>,
            leaves: Vec<(u64, u32, bool)>,
            raw: u64,
            width: u32,
            signed: bool,
            dollar: bool,
        }
        fn bound<'a>(me: &mut Simulator, e: &'a Expression) -> Option<Bound<'a>> {
            let mut leaves = Vec::new();
            let (width, signed) = me.exact_leaves(e, &mut leaves)?;
            Some(Bound {
                expr: Some(e),
                leaves,
                raw: 0,
                width,
                signed,
                dollar: matches!(e.kind, ExprKind::Dollar),
            })
        }
        let mut bounds: Vec<(Bound<'_>, Option<Bound<'_>>)> = Vec::new();
        let mut ws: Option<(u32, bool)> = Some((1, true));
        let note = |ws: &mut Option<(u32, bool)>, b: &Bound<'_>| {
            if !b.dollar
                && let Some((aw, as_)) = ws
            {
                *aw = (*aw).max(b.width);
                *as_ &= b.signed;
            }
        };
        // Cache each expression's leaves once. Its joint comparison type must
        // be known before folding arithmetic or dropping an inverted range.
        // 8'(148) is signed in isolation but becomes 148 in an unsigned set.
        for r in ranges {
            match r {
                ConstraintRange::Value(e) => {
                    if let Some(nm) = self.array_operand_name(e) {
                        for i in 0..self.get_queue_size(&nm) {
                            if let Some(ev) =
                                self.get_signal_value_by_name(&format!("{}[{}]", nm, i))
                                && let Some(raw) = ev.to_u64()
                            {
                                let b = Bound {
                                    expr: None,
                                    leaves: Vec::new(),
                                    raw,
                                    width: ev.width,
                                    signed: ev.is_signed,
                                    dollar: false,
                                };
                                note(&mut ws, &b);
                                bounds.push((b, None));
                            }
                        }
                        continue;
                    }
                    if let Some(b) = bound(self, e) {
                        note(&mut ws, &b);
                        bounds.push((b, None));
                    } else {
                        ws = None;
                    }
                }
                ConstraintRange::Range { lo, hi } => {
                    let (Some(l), Some(h)) = (bound(self, lo), bound(self, hi)) else {
                        ws = None;
                        continue;
                    };
                    note(&mut ws, &l);
                    note(&mut ws, &h);
                    bounds.push((l, Some(h)));
                }
            }
        }
        let at = |me: &mut Self, b: &Bound<'_>| -> Option<i128> {
            let (w, s) = if b.dollar {
                (b.width, b.signed)
            } else {
                let (w, s) = ws.unwrap_or((b.width, b.signed));
                (w.max(ctx_w), s)
            };
            let v = if let Some(e) = b.expr {
                me.exact_fold(e, &b.leaves, &mut 0, w, s)?
            } else {
                Self::as_ctx(b.raw as i128, b.width, s)
            };
            Some(if b.dollar || w == 0 || w > 64 {
                v
            } else {
                Self::as_ctx(v, w, s)
            })
        };
        let mut out = Vec::with_capacity(bounds.len());
        let mut folded_all = true;
        for (lo, hi) in bounds {
            let Some(l) = at(self, &lo) else {
                folded_all = false;
                continue;
            };
            let Some(h) = hi.as_ref().map_or(Some(l), |b| at(self, b)) else {
                folded_all = false;
                continue;
            };
            if l <= h || lo.dollar || hi.as_ref().is_some_and(|b| b.dollar) {
                out.push((l.min(h), l.max(h)));
            }
        }
        (out, folded_all.then_some(ws).flatten())
    }

    /// Map a comparison set into the target's representable domain before
    /// sampling. Wider signed constants cannot alias narrow target values;
    /// unsigned items can name a signed value by its sign-extended bit pattern.
    pub(super) fn cons_ranges_for_target(
        &mut self,
        ranges: &[ConstraintRange],
        width: u32,
        signed: bool,
    ) -> Vec<(i128, i128)> {
        let (rs, items_ws) = self.cons_ranges_i128_ws(ranges, width);
        Self::fit_cons_ranges_for_target(rs, items_ws, width, signed)
    }

    pub(super) fn fit_cons_ranges_for_target(
        rs: Vec<(i128, i128)>,
        items_ws: Option<(u32, bool)>,
        width: u32,
        signed: bool,
    ) -> Vec<(i128, i128)> {
        if width == 0 || width > 64 {
            return rs;
        }
        if signed && let Some((iw, false)) = items_ws {
            let w = iw.max(width).min(64);
            let half = 1i128 << (width - 1);
            let period = 1i128 << w;
            let mut out = Vec::new();
            for (l, h) in rs {
                let (l, h) = (Self::bits_at(l, w) as i128, Self::bits_at(h, w) as i128);
                if l > h {
                    continue;
                }
                let (a, b) = (l.max(0), h.min(half - 1));
                if a <= b {
                    out.push((a, b));
                }
                let (a, b) = (l.max(period - half), h.min(period - 1));
                if a <= b {
                    out.push((a - period, b - period));
                }
            }
            return out;
        }
        // Match the existing membership judge's signed interpretation when
        // a negative endpoint is present, including legacy unsigned targets.
        let signed = signed || rs.iter().any(|&(l, h)| l < 0 || h < 0);
        let (lo, hi) = if signed {
            let half = 1i128 << (width - 1);
            (-half, half - 1)
        } else {
            (0, (1i128 << width) - 1)
        };
        rs.into_iter()
            .filter_map(|(l, h)| {
                let (l, h) = (l.max(lo), h.min(hi));
                (l <= h).then_some((l, h))
            })
            .collect()
    }

    /// Exact-signed membership test used by every `inside`/`dist` solve path.
    pub(super) fn value_in_ranges(&mut self, val: &Value, ranges: &[ConstraintRange]) -> bool {
        let (rs, items_ws) = self.cons_ranges_i128_ws(ranges, val.width);
        let Some(raw) = val.to_u64() else {
            return false;
        };
        if val.has_xz() {
            return false;
        }
        // §11.4.13/§11.8.1: a signed value against an unsigned item is
        // compared unsigned, at the width of all operands. The reference
        // simulator's solver extends the value by its own sign first, then
        // compares the bits: `byte b inside {8'd255}` holds for -1, and
        // `shortint s inside {32'hffff}` holds for no value.
        if val.is_signed
            && let Some((iw, false)) = items_ws
        {
            let w = iw.max(val.width).min(64);
            let v = Self::bits_at(Self::as_ctx(raw as i128, val.width, true), w) as i128;
            return rs.iter().any(|&(l, h)| {
                let (l, h) = (Self::bits_at(l, w) as i128, Self::bits_at(h, w) as i128);
                l <= h && v >= l && v <= h
            });
        }
        // A negative endpoint can only be met by a signed reading of the value
        // (an `integer` whose is_signed was dropped by an earlier draw).
        let signed = val.is_signed || rs.iter().any(|(l, h)| *l < 0 || *h < 0);
        let v = Self::as_ctx(raw as i128, val.width, signed);
        rs.iter().any(|(l, h)| v >= *l && v <= *h)
    }

    /// Pick a concrete value satisfying a constraint range list (used for
    /// `inside`/`dist` targets). Ranges are exact and signed; the result is
    /// re-encoded at `width` and carries `signed` so later compares stay
    /// signed. Returns None if every endpoint failed to evaluate.
    pub(super) fn pick_from_ranges(
        &mut self,
        ranges: &[ConstraintRange],
        width: u32,
        signed: bool,
    ) -> Option<Value> {
        let choices = self.cons_ranges_for_target(ranges, width, signed);
        self.pick_i128_range(&choices, width, signed)
    }

    /// Uniformly pick a value from one of `choices` (an exact `[lo:hi]` list).
    /// §18.5: a uniformly random value of ANY width. `Value::from_u64` tops
    /// out at 64 bits, so wider fields are filled 64 bits at a time — the
    /// callers previously had a bare `if width <= 64` with no else, leaving a
    /// wide rand field at `Value::zero(width)` while `randomize()` still
    /// reported success (a `rand logic [127:0]` was always 0).
    pub(super) fn random_value_of_width(&mut self, width: u32) -> Value {
        use rand::RngExt;
        if width == 0 {
            return Value::zero(0);
        }
        if width <= 64 {
            let r: u64 = self.cur_rng().random();
            return Value::from_u64(r, width);
        }
        let mut acc = Value::zero(width);
        let mut bit = 0u32;
        while bit < width {
            let chunk: u64 = self.cur_rng().random();
            let n = (width - bit).min(64);
            for k in 0..n {
                if (chunk >> k) & 1 == 1 {
                    acc.set_bit((bit + k) as usize, LogicBit::One);
                }
            }
            bit += n;
        }
        acc
    }

    pub(super) fn pick_i128_range(
        &mut self,
        choices: &[(i128, i128)],
        width: u32,
        signed: bool,
    ) -> Option<Value> {
        use rand::RngExt;
        let v = if let [(lo, hi)] = choices {
            // Retain the existing RNG calls for the common one-interval case.
            if lo > hi {
                return None;
            }
            let _ = self.cur_rng().random_range(0..1usize);
            if hi > lo {
                self.cur_rng().random_range(*lo..=*hi)
            } else {
                *lo
            }
        } else {
            // An inside list is a union of values, not a weighted list of
            // items. Overlap and duplicate entries contribute no extra mass.
            let mut sorted: Vec<_> = choices.iter().copied().filter(|(l, h)| l <= h).collect();
            sorted.sort_unstable();
            let mut union: Vec<(i128, i128)> = Vec::with_capacity(sorted.len());
            for (l, h) in sorted {
                if let Some(last) = union.last_mut()
                    && l <= last.1.saturating_add(1)
                {
                    last.1 = last.1.max(h);
                } else {
                    union.push((l, h));
                }
            }
            if union.is_empty() {
                return None;
            }
            // Inclusive cardinalities use unsigned modular arithmetic: zero
            // denotes exactly 2^128 values, possible only for the full domain.
            // No signed subtraction, capped span or saturating weight is used.
            let size = |l: i128, h: i128| (h as u128).wrapping_sub(l as u128).wrapping_add(1);
            let total = union
                .iter()
                .fold(0u128, |n, &(l, h)| n.wrapping_add(size(l, h)));
            let mut rank = if total == 0 {
                self.cur_rng().random::<u128>()
            } else {
                self.cur_rng().random_range(0..total)
            };
            let mut chosen = None;
            for (l, h) in union {
                let n = size(l, h);
                if n == 0 || rank < n {
                    chosen = Some(l.wrapping_add(rank as i128));
                    break;
                }
                rank -= n;
            }
            chosen?
        };
        let mut out = if width <= 64 {
            Value::from_u64(Self::bits_at(v, width) as u64, width)
        } else {
            Value::from_u128(Self::bits_at(v, width.min(128)), width)
        };
        out.is_signed = signed;
        Some(out)
    }

    /// Apply the assignments forced by a single constraint item against the
    /// current solved state, returning whether anything changed. Equality pins
    /// the rand variable to the other side; inside/dist pins it into the
    /// allowed set (only if not already satisfying); if/else and implication
    /// recurse into the branch selected by the live condition value.
    /// Collect the set of values excluded from a rand array's elements by
    /// `foreach (arr[i]) !(arr[i] inside {SET})` constraints. The exclusion
    /// values (enum members and other rand scalar fields like sp/tp/scratch_reg)
    /// are evaluated against the *current* solved state, so this must run after
    /// those scalars are assigned. Other rand arrays in the set (e.g. pmp_reg)
    /// are enumerated element-by-element.
    pub(super) fn collect_array_exclusions(
        &mut self,
        array_prop: &str,
        constraints: &[ClassConstraint],
    ) -> HashSet<u64> {
        let mut excluded: HashSet<u64> = HashSet::default();
        // Find Foreach constraints whose iterated array is `array_prop`.
        fn array_base_name(e: &Expression) -> Option<String> {
            match &e.kind {
                ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.clone()),
                _ => None,
            }
        }
        // Recursively walk a constraint body for `!(x inside {ranges})`.
        let mut neg_inside_ranges: Vec<Vec<Expression>> = Vec::new();
        fn walk(item: &ConstraintItem, out: &mut Vec<Vec<Expression>>) {
            match item {
                ConstraintItem::Expr(e) => {
                    if let ExprKind::Unary {
                        op: UnaryOp::LogNot,
                        operand,
                    } = &e.kind
                    {
                        let inside = match &operand.kind {
                            ExprKind::Inside { ranges, .. } => Some(ranges),
                            ExprKind::Paren(p) => match &p.kind {
                                ExprKind::Inside { ranges, .. } => Some(ranges),
                                _ => None,
                            },
                            _ => None,
                        };
                        if let Some(ranges) = inside {
                            out.push(ranges.clone());
                        }
                    }
                }
                ConstraintItem::Block(items) => {
                    for it in items {
                        walk(it, out);
                    }
                }
                ConstraintItem::Soft(inner) => walk(inner, out),
                _ => {}
            }
        }
        for con in constraints {
            for item in &con.items {
                if let ConstraintItem::Foreach {
                    array, item: body, ..
                } = item
                {
                    if array_base_name(array).as_deref() == Some(array_prop) {
                        walk(body.as_ref(), &mut neg_inside_ranges);
                    }
                }
            }
        }
        // Evaluate every excluded operand. An operand that is itself a rand
        // array (queue/array storage) contributes each of its elements.
        for ranges in neg_inside_ranges {
            for r in &ranges {
                if let Some(nm) = self.array_operand_name(r) {
                    let sz = self.get_queue_size(&nm);
                    for i in 0..sz {
                        if let Some(ev) = self.get_signal_value_by_name(&format!("{}[{}]", nm, i)) {
                            if let Some(u) = ev.to_u64() {
                                excluded.insert(u);
                            }
                        }
                    }
                } else if let Some(u) = self.eval_expr(r).to_u64() {
                    excluded.insert(u);
                }
            }
        }
        excluded
    }

    /// Collect values excluded from a rand SCALAR field by top-level
    /// `!(field inside {SET})` and `field != X` constraints (evaluated against
    /// the current solved state). Used to push a rand enum field off a reserved
    /// value (e.g. riscv-dv `scratch_reg`/`sp`/`tp`/`ra` must avoid ZERO/GP/…).
    pub(super) fn collect_scalar_exclusions(
        &mut self,
        field: &str,
        constraints: &[ClassConstraint],
    ) -> HashSet<u64> {
        fn is_field(e: &Expression, field: &str) -> bool {
            matches!(&e.kind, ExprKind::Ident(h)
                if h.path.len() == 1 && h.path[0].name.name == field)
        }
        // Gather the operand expressions to exclude.
        let mut to_eval: Vec<Expression> = Vec::new();
        fn walk(item: &ConstraintItem, field: &str, out: &mut Vec<Expression>) {
            match item {
                ConstraintItem::Expr(e) => match &e.kind {
                    ExprKind::Unary {
                        op: UnaryOp::LogNot,
                        operand,
                    } => {
                        let inside = match &operand.kind {
                            ExprKind::Inside { expr, ranges } => Some((expr, ranges)),
                            ExprKind::Paren(p) => match &p.kind {
                                ExprKind::Inside { expr, ranges } => Some((expr, ranges)),
                                _ => None,
                            },
                            _ => None,
                        };
                        if let Some((inner, ranges)) = inside {
                            if is_field(inner, field) {
                                out.extend(ranges.iter().cloned());
                            }
                        }
                    }
                    ExprKind::Binary {
                        op: BinaryOp::Neq,
                        left,
                        right,
                    } => {
                        if is_field(left, field) {
                            out.push((**right).clone());
                        } else if is_field(right, field) {
                            out.push((**left).clone());
                        }
                    }
                    _ => {}
                },
                ConstraintItem::Block(items) => {
                    for it in items {
                        walk(it, field, out);
                    }
                }
                ConstraintItem::Soft(inner) => walk(inner, field, out),
                _ => {}
            }
        }
        for con in constraints {
            for item in &con.items {
                walk(item, field, &mut to_eval);
            }
        }
        let mut excluded: HashSet<u64> = HashSet::default();
        for e in to_eval {
            if let Some(nm) = self.array_operand_name(&e) {
                let sz = self.get_queue_size(&nm);
                for i in 0..sz {
                    if let Some(ev) = self.get_signal_value_by_name(&format!("{}[{}]", nm, i)) {
                        if let Some(u) = ev.to_u64() {
                            excluded.insert(u);
                        }
                    }
                }
            } else if let Some(u) = self.eval_expr(&e).to_u64() {
                excluded.insert(u);
            }
        }
        excluded
    }

    /// Declared enum type of a class property, walking the `extends` chain
    /// from the instance's dynamic class. Used by the `!=` solver arm so a
    /// re-pick on an enum-typed field draws a *valid* member rather than a
    /// raw width-masked random.
    pub(super) fn prop_enum_type(&self, handle: usize, prop: &str) -> Option<String> {
        let mut cur = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone());
        let mut guard = 0;
        while let Some(cname) = cur {
            guard += 1;
            if guard > 64 {
                break;
            }
            if let Some(class_def) = self.module.classes.get(&cname) {
                if let Some(sig) = class_def.properties.get(prop) {
                    return sig
                        .type_name
                        .clone()
                        .filter(|tn| self.module.enum_members.contains_key(tn));
                }
                cur = class_def.extends.clone();
            } else {
                break;
            }
        }
        None
    }

    /// The rand SCALAR properties that appear in `e`, in source order of first
    /// appearance — the candidate targets of an algebraic solve. Only bare
    /// single-segment identifiers qualify (a `sub.field` is not a scalar
    /// property of THIS object).
    pub(super) fn affine_candidates(
        &self,
        e: &Expression,
        rand_set: &HashSet<String>,
    ) -> Vec<String> {
        fn walk(e: &Expression, out: &mut Vec<String>) {
            match &e.kind {
                ExprKind::Ident(h) if h.path.len() == 1 => {
                    let n = &h.path[0].name.name;
                    if !out.contains(n) {
                        out.push(n.clone());
                    }
                }
                ExprKind::Unary { operand, .. } => walk(operand, out),
                ExprKind::Binary { left, right, .. } => {
                    walk(left, out);
                    walk(right, out);
                }
                ExprKind::Paren(inner) => walk(inner, out),
                ExprKind::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                } => {
                    walk(condition, out);
                    walk(then_expr, out);
                    walk(else_expr, out);
                }
                ExprKind::Concatenation(es) => {
                    for x in es {
                        walk(x, out);
                    }
                }
                ExprKind::Call { args, .. } | ExprKind::SystemCall { args, .. } => {
                    for a in args {
                        walk(a, out);
                    }
                }
                _ => {}
            }
        }
        let mut names: Vec<String> = Vec::new();
        walk(e, &mut names);
        names.retain(|n| rand_set.contains(n));
        names
    }

    /// IEEE 1800-2017 §18.5.12 — solve an ALGEBRAIC equality for one rand
    /// variable.
    ///
    /// The propagation solver only ever recognised a BARE identifier on one
    /// side of `==`, so `int_val + $signed({1'b0, l_val}) == 32'd50` and
    /// `(r_val - 8'd10) * (l_val + 8'd5) == 16'd0` were never solved: every
    /// trial failed the satisfaction check and randomize() returned 0.
    ///
    /// Both sides are AFFINE in a single rand variable `t` once the other
    /// variables are held at their current draw, so probe each side at t = 0,
    /// 1, 2 with exact i128 arithmetic (`exact_int`). If the three probes are
    /// collinear, the equation reduces to `a·t == b` and has at most one exact
    /// integer solution. A solution outside `t`'s domain — or outside its
    /// declared `inside` range — is rejected and the next variable in the
    /// expression is tried, so `int_val + l_val == 50` can be solved for
    /// EITHER side depending on which one lands in range.
    ///
    /// This never guesses: the assignment it writes is re-validated by the full
    /// satisfaction pass, so an approximate solve costs a retry, never a wrong
    /// answer.
    pub(super) fn solve_affine_eq(
        &mut self,
        handle: usize,
        left: &Expression,
        right: &Expression,
        rand_set: &HashSet<String>,
    ) -> bool {
        // §18.5: a `.size()` of a rand dynamic array is not a settled integral
        // operand during the scalar fixpoint — it is empty until sized, so
        // treating `arr.size() == N` as an affine equation in `N` would force
        // `N := arr.size()` (= 0) and clobber a sibling `N == K`. The array
        // sizing solver owns this coupling.
        if self.is_rand_dyn_size_expr(handle, left) || self.is_rand_dyn_size_expr(handle, right) {
            return false;
        }
        let mut cands = self.affine_candidates(left, rand_set);
        for c in self.affine_candidates(right, rand_set) {
            if !cands.contains(&c) {
                cands.push(c);
            }
        }
        if cands.is_empty() {
            return false;
        }
        // Best in-domain-but-out-of-range fallback, used only if no candidate
        // solves fully in range.
        let mut fallback: Option<(String, Value)> = None;
        for name in cands {
            let Some(orig) = self
                .heap
                .get(handle)
                .and_then(|o| o.as_ref())
                .and_then(|i| i.properties.get(&name))
                .cloned()
            else {
                continue;
            };
            let width = orig.width.max(1);
            if width > 64 {
                continue;
            }
            let signed = orig.is_signed || self.class_prop_signed_of(handle, &name);
            // d(x) = lhs(x) - rhs(x) at t = x.
            let mut d: [i128; 3] = [0; 3];
            let mut ok = true;
            for (k, probe) in [0u64, 1, 2].iter().enumerate() {
                let mut pv = Value::from_u64(*probe, width);
                pv.is_signed = signed;
                if let Some(Some(inst)) = self.heap.get_mut(handle) {
                    inst.properties.insert(name.clone(), pv);
                }
                let (Some((lv, ..)), Some((rv, ..))) =
                    (self.exact_int(left), self.exact_int(right))
                else {
                    ok = false;
                    break;
                };
                d[k] = lv - rv;
            }
            // Restore before deciding — a probe must never leak.
            if let Some(Some(inst)) = self.heap.get_mut(handle) {
                inst.properties.insert(name.clone(), orig.clone());
            }
            if !ok {
                continue;
            }
            let a = d[1] - d[0];
            // Not affine in this variable (a product of two rand vars, a
            // division, …) — the three probes are not collinear.
            if d[2] - d[1] != a || a == 0 {
                continue;
            }
            let b = -d[0];
            if b % a != 0 {
                continue; // no integer solution
            }
            let t = b / a;
            // §18.3: the solution must be representable in the variable.
            let (lo, hi) = if signed {
                (-(1i128 << (width - 1)), (1i128 << (width - 1)) - 1)
            } else {
                (0i128, (1i128 << width) - 1)
            };
            if t < lo || t > hi {
                continue;
            }
            let mut val = Value::from_u64(Self::bits_at(t, width) as u64, width);
            val.is_signed = signed;
            // Prefer a solution that also satisfies the variable's own
            // `inside` range; otherwise remember it and try the next variable.
            let in_range = self
                .rand_ranges
                .get(&name)
                .is_none_or(|rs| rs.iter().any(|(l, h)| t >= *l && t <= *h));
            if in_range {
                return self.set_prop_if_changed(handle, &name, val);
            }
            if fallback.is_none() {
                fallback = Some((name, val));
            }
        }
        if let Some((name, val)) = fallback {
            return self.set_prop_if_changed(handle, &name, val);
        }
        false
    }

    /// §18.5 — force a relational constraint (`x > K`, `K <= x`) whose rand
    /// side currently violates it. The target is re-picked uniformly from the
    /// interval the relation implies, intersected with its declared `inside`
    /// range. Nothing happens when the constraint already holds, so this cannot
    /// oscillate; when neither side is a solvable rand target it is left to the
    /// generate-and-test backstop.
    pub(super) fn force_relational(
        &mut self,
        handle: usize,
        op: &BinaryOp,
        left: &Expression,
        right: &Expression,
        rand_set: &HashSet<String>,
    ) -> bool {
        match self.exact_cmp(op, left, right) {
            Some(true) => return false, // already satisfied
            None => return false,       // X/Z or not modelled — let the check decide
            Some(false) => {}
        }
        // Mirror the operator: `K < x` constrains x from below.
        let mirrored = match op {
            BinaryOp::Lt => BinaryOp::Gt,
            BinaryOp::Leq => BinaryOp::Geq,
            BinaryOp::Gt => BinaryOp::Lt,
            BinaryOp::Geq => BinaryOp::Leq,
            other => *other,
        };
        // Which side is the rand target, and what does the relation say about
        // it? A bare rand property first; then (§18.4) a member or a select
        // of a member of an aggregate rand property.
        enum RelTarget {
            Prop(String),
            Member(RandMemberTarget),
        }
        let (target, bound_expr, op_on_target) =
            if let Some(v) = self.rand_lvalue_name(left, rand_set) {
                (RelTarget::Prop(v), right, *op)
            } else if let Some(v) = self.rand_lvalue_name(right, rand_set) {
                (RelTarget::Prop(v), left, mirrored)
            } else if let Some(t) = self.rand_agg_member_target(left, rand_set) {
                (RelTarget::Member(t), right, *op)
            } else if let Some(t) = self.rand_agg_member_target(right, rand_set) {
                (RelTarget::Member(t), left, mirrored)
            } else {
                return self.force_sum_bound(handle, op, left, right, rand_set);
            };
        let Some((bound, _, bs)) = self.exact_int(bound_expr) else {
            return false;
        };
        let cur = match &target {
            RelTarget::Prop(name) => self
                .heap
                .get(handle)
                .and_then(|o| o.as_ref())
                .and_then(|i| i.properties.get(name))
                .cloned(),
            RelTarget::Member(t) => self.read_rand_member(t),
        };
        let Some(cur) = cur else {
            return false;
        };
        let width = cur.width.max(1);
        if width > 64 {
            return false;
        }
        let prop_signed = match &target {
            RelTarget::Prop(name) => self.class_prop_signed_of(handle, name),
            RelTarget::Member(_) => false,
        };
        let signed = (cur.is_signed || prop_signed) && bs;
        let (dom_lo, dom_hi) = if signed {
            (-(1i128 << (width - 1)), (1i128 << (width - 1)) - 1)
        } else {
            (0i128, (1i128 << width) - 1)
        };
        let (mut lo, mut hi) = match op_on_target {
            BinaryOp::Lt => (dom_lo, bound - 1),
            BinaryOp::Leq => (dom_lo, bound),
            BinaryOp::Gt => (bound + 1, dom_hi),
            BinaryOp::Geq => (bound, dom_hi),
            _ => return false,
        };
        lo = lo.max(dom_lo);
        hi = hi.min(dom_hi);
        if lo > hi {
            return false; // unsatisfiable for this variable — retry the trial
        }
        // Intersect with the target's declared `inside` range (§18.5.12).
        let mut choices: Vec<(i128, i128)> = Vec::new();
        let ranges = match &target {
            RelTarget::Prop(name) => self.rand_ranges.get(name).cloned(),
            RelTarget::Member(_) => None,
        };
        if let Some(rs) = ranges {
            for (rl, rh) in rs {
                let (l, h) = (rl.max(lo), rh.min(hi));
                if l <= h {
                    choices.push((l, h));
                }
            }
        }
        if choices.is_empty() {
            choices.push((lo, hi));
        }
        let Some(p) = self.pick_i128_range(&choices, width, signed) else {
            return false;
        };
        match &target {
            RelTarget::Prop(name) => self.set_prop_if_changed(handle, name, p),
            RelTarget::Member(t) => self.set_rand_member(t, p),
        }
    }

    /// §18.5: `t1 + t2 + … <= K` (or `<`, or mirrored) where every term is a
    /// rand variable or member — bare, or zero-extended as `{1'b0, x}` — so
    /// each term is non-negative and the bound is met by lowering terms. A
    /// sum of struct members (`{1'b0, regs.MODE.NUM_HOSTS} + {1'b0,
    /// regs.MODE.FIRST_HOST} <= 1`) had no repair at all: only a lucky draw
    /// passed, so randomize() exhausted its trials. Terms are visited in
    /// random order; one that still fits the remaining budget keeps its value
    /// (so a term another constraint pins stays put), any other is redrawn
    /// within the budget. Called only when the relation is violated.
    pub(super) fn force_sum_bound(
        &mut self,
        handle: usize,
        op: &BinaryOp,
        left: &Expression,
        right: &Expression,
        rand_set: &HashSet<String>,
    ) -> bool {
        enum Term {
            Prop(String, u32),
            Member(RandMemberTarget, u32),
        }
        fn leaves<'a>(e: &'a Expression, out: &mut Vec<&'a Expression>) {
            match &e.kind {
                ExprKind::Paren(i) => leaves(i, out),
                ExprKind::Binary {
                    op: BinaryOp::Add,
                    left,
                    right,
                } => {
                    leaves(left, out);
                    leaves(right, out);
                }
                _ => out.push(e),
            }
        }
        // The side holding the sum, and the inclusive upper bound on it.
        let (sum_side, bound_side, strict) = match op {
            BinaryOp::Leq => (left, right, false),
            BinaryOp::Lt => (left, right, true),
            BinaryOp::Geq => (right, left, false),
            BinaryOp::Gt => (right, left, true),
            _ => return false,
        };
        let mut parts: Vec<&Expression> = Vec::new();
        leaves(sum_side, &mut parts);
        if parts.len() < 2 {
            return false;
        }
        let mut terms: Vec<Term> = Vec::with_capacity(parts.len());
        for p in parts {
            // `{1'b0, x}` / `{2'b00, x}`: zero bits above a single target.
            let target = match &Self::unparen(p).kind {
                ExprKind::Concatenation(items) => {
                    let Some((last, zeros)) = items.split_last() else {
                        return false;
                    };
                    let all_zero = zeros.iter().all(|z| {
                        matches!(z.kind, ExprKind::Number(_))
                            && self.eval_expr(z).to_u64() == Some(0)
                    });
                    if !all_zero {
                        return false;
                    }
                    last
                }
                _ => p,
            };
            if let Some(name) = self.rand_lvalue_name(target, rand_set) {
                let w = self.class_prop_width_of(handle, &name).unwrap_or(0);
                if w == 0 || w > 63 || self.class_prop_signed_of(handle, &name) {
                    return false;
                }
                terms.push(Term::Prop(name, w));
            } else if let Some(t) = self.rand_agg_member_target(target, rand_set) {
                let w = self.read_rand_member(&t).map(|v| v.width).unwrap_or(0);
                if w == 0 || w > 63 {
                    return false;
                }
                terms.push(Term::Member(t, w));
            } else {
                return false;
            }
        }
        let Some((k, _, _)) = self.exact_int(bound_side) else {
            return false;
        };
        let mut budget: i128 = if strict { k - 1 } else { k };
        if budget < 0 {
            return false; // unsatisfiable for non-negative terms
        }
        use rand::seq::SliceRandom;
        let mut order: Vec<usize> = (0..terms.len()).collect();
        order.shuffle(self.cur_rng());
        let mut changed = false;
        for i in order {
            let (cur, w) = match &terms[i] {
                Term::Prop(n, w) => (
                    self.heap
                        .get(handle)
                        .and_then(|o| o.as_ref())
                        .and_then(|inst| inst.properties.get(n))
                        .and_then(|v| v.to_u64()),
                    *w,
                ),
                Term::Member(t, w) => (self.read_rand_member(t).and_then(|v| v.to_u64()), *w),
            };
            let max = ((1u128 << w) - 1) as i128;
            let cap = budget.min(max);
            let keep = cur.is_some_and(|c| (c as i128) <= cap);
            let v = if keep {
                cur.unwrap() as i128
            } else {
                use rand::RngExt;
                let v = self.cur_rng().random_range(0..=cap as u64) as i128;
                let nv = Value::from_u64(v as u64, w);
                changed |= match &terms[i] {
                    Term::Prop(n, _) => self.set_prop_if_changed(handle, n, nv),
                    Term::Member(t, _) => self.set_rand_member(t, nv),
                };
                v
            };
            budget -= v;
        }
        changed
    }

    /// Is `e` a `.size()` (`.size` / `.size()`) reference to a rand DYNAMIC
    /// array / queue of the object `handle`? Used by the constraint solver so
    /// an equality between a rand scalar and an as-yet-unsized collection
    /// length does not force the scalar from the collection's current (empty)
    /// size — the sizing phase owns that coupling.
    pub(super) fn is_rand_dyn_size_expr(&self, handle: usize, e: &Expression) -> bool {
        let arr = match &e.kind {
            ExprKind::Call { func, args } if args.is_empty() => match &func.kind {
                ExprKind::MemberAccess { expr, member } if member.name == "size" => expr,
                _ => return false,
            },
            ExprKind::MemberAccess { expr, member } if member.name == "size" => expr,
            _ => return false,
        };
        let name = match &arr.kind {
            ExprKind::Ident(h) if h.path.len() == 1 => &h.path[0].name.name,
            // §18.7: `obj.m.size()` in `obj.randomize() with {…}`.
            ExprKind::MemberAccess { expr, member }
                if self.active_rand_receiver().is_some()
                    && Self::plain_ident_name(expr).as_deref() == self.active_rand_receiver() =>
            {
                &member.name
            }
            _ => return false,
        };
        let cn = match self.heap.get(handle).and_then(|o| o.as_ref()) {
            Some(inst) => inst.class_name.clone(),
            None => return false,
        };
        let mut cur = Some(cn);
        while let Some(c) = cur {
            let Some(cd) = self.module.classes.get(&c) else {
                return false;
            };
            if cd.queue_properties.contains_key(name) {
                return true;
            }
            cur = cd.extends.clone();
        }
        false
    }

    /// Does a binary constraint expression couple a rand DYNAMIC-array length
    /// through `.size()` on either operand? Used so such an item is not
    /// blocked from (and not folded into) the coupled scalar solve.
    pub(super) fn expr_involves_dyn_size(&self, handle: usize, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Binary { left, right, .. } => {
                self.is_rand_dyn_size_expr(handle, left)
                    || self.is_rand_dyn_size_expr(handle, right)
            }
            _ => false,
        }
    }

    /// §18.5 — a WHOLE (member-free) reference to an element of a rand
    /// FIXED or DYNAMIC array property, e.g. `cfg[0][0]`. Returns `(name,
    /// suffix)` where `name` is the bare rand property and `suffix` is the
    /// `[i][j]…` index chain, matching `class_elem_store`'s `name+suffix`
    /// key. Unlike `coll_elem_expr_key` this peels an arbitrary N-D index
    /// chain for unpacked FIXED arrays, so a whole-struct cross-element
    /// equality like `cfg[0][0] == cfg[1][0]` (the 5446 reg-map coupling)
    /// is recognised on both sides.
    pub(super) fn rand_whole_elem_lvalue(
        &mut self,
        handle: usize,
        expr: &Expression,
        rand_set: &HashSet<String>,
    ) -> Option<(String, String)> {
        // Peel trailing Index nodes, collecting `[i]` from the outside in.
        let mut suffix = String::new();
        let mut base = expr;
        while let ExprKind::Index { expr: b, index } = &base.kind {
            let i = self.eval_expr(index).to_i64()?;
            suffix = format!("[{}]{}", i, suffix);
            base = b;
        }
        if suffix.is_empty() {
            return None;
        }
        let peeled;
        let base = match self.strip_rand_receiver_prefix(base) {
            Some(b) => {
                peeled = b;
                &peeled
            }
            None => base,
        };
        let ExprKind::Ident(h) = &base.kind else {
            return None;
        };
        if h.path.len() != 1 || !rand_set.contains(&h.path[0].name.name) {
            return None;
        }
        let name = h.path[0].name.name.clone();
        // Any array-shaped rand property (packed, fixed, or dynamic).
        if self.class_prop_packed_dims(handle, &name).is_some()
            || self.fixed_foreach_dims(handle, &name).is_some()
            || self
                .module
                .dynamic_arrays
                .contains(&format!("{}#{}", handle, name))
        {
            return Some((name, suffix));
        }
        None
    }

    pub(super) fn solve_forced(
        &mut self,
        handle: usize,
        item: &ConstraintItem,
        rand_set: &HashSet<String>,
    ) -> bool {
        match item {
            ConstraintItem::Expr(e) => match &e.kind {
                // A parenthesised constraint expression — `(a -> (b == 0))`,
                // and, crucially, the CONSEQUENT of an implication, which the
                // parser keeps wrapped: `(control_bit == 0) -> (data_byte ==
                // 0)`. Without unwrapping, the consequent was never forced and
                // the trial loop simply re-drew until the antecedent was false,
                // which silently turned every implication into a rejection
                // sample (§18.5.10: that is the behavior `solve … before …` is
                // supposed to REPLACE).
                ExprKind::Paren(inner) => {
                    let it = ConstraintItem::Expr((**inner).clone());
                    self.solve_forced(handle, &it, rand_set)
                }
                // `antecedent -> consequent`: when the antecedent holds, the
                // consequent becomes a forced sub-constraint.
                ExprKind::Binary {
                    op: BinaryOp::LogImplies,
                    left,
                    right,
                } => {
                    if self.eval_expr(left).is_true() {
                        let cons = ConstraintItem::Expr((**right).clone());
                        return self.solve_forced(handle, &cons, rand_set);
                    }
                    false
                }
                // §18.5.12 — a CONJUNCTION is two constraints: solve both.
                // Without this the consequent of `(pkt_type == START) ->
                // (pkt_hdr.valid == 1'b1 && pkt_hdr.tag > 3'd4)` was never
                // forced and only random luck could satisfy the implication.
                ExprKind::Binary {
                    op: BinaryOp::LogAnd,
                    left,
                    right,
                } => {
                    let a = self.solve_forced(
                        handle,
                        &ConstraintItem::Expr((**left).clone()),
                        rand_set,
                    );
                    let b = self.solve_forced(
                        handle,
                        &ConstraintItem::Expr((**right).clone()),
                        rand_set,
                    );
                    a || b
                }
                // §18.5.12 — a disjunction needs at least one satisfiable
                // branch. Preserve an already-valid draw; otherwise try each
                // branch in source order and stop as soon as the complete
                // expression becomes true.
                ExprKind::Binary {
                    op: BinaryOp::LogOr,
                    left,
                    right,
                } => {
                    if self.cons_expr_true(e) {
                        return false;
                    }
                    let mut changed = self.solve_forced(
                        handle,
                        &ConstraintItem::Expr((**left).clone()),
                        rand_set,
                    );
                    if self.cons_expr_true(e) {
                        return changed;
                    }
                    changed |= self.solve_forced(
                        handle,
                        &ConstraintItem::Expr((**right).clone()),
                        rand_set,
                    );
                    changed
                }
                // §18.5 relational constraint (`tag > 3'd4`, `leaf_val > 8'd50`).
                // When the current draw already satisfies it, leave it alone;
                // otherwise re-pick the rand side uniformly from the half-open
                // interval the relation implies, intersected with the target's
                // declared `inside` range. Judged with exact signed arithmetic.
                ExprKind::Binary {
                    op: op @ (BinaryOp::Lt | BinaryOp::Leq | BinaryOp::Gt | BinaryOp::Geq),
                    left,
                    right,
                } => self.force_relational(handle, op, left, right, rand_set),
                ExprKind::Binary {
                    op: BinaryOp::Eq,
                    left,
                    right,
                } => {
                    // Already satisfied under the exact (context-width) reading
                    // the satisfaction judge uses — do not churn.
                    if self.exact_cmp(&BinaryOp::Eq, left, right) == Some(true) {
                        return false;
                    }
                    if let Some(v) = self.rand_lvalue_name(left, rand_set) {
                        if self.rand_lvalue_name(right, rand_set).is_none() {
                            // §18.5: do not force a rand scalar from a rand
                            // DYNAMIC-ARRAY `.size()` (`arr.size() == N`): the
                            // array length is not settled yet, so its current
                            // size is 0 and forcing `N := arr.size()` clobbers
                            // a sibling constraint (`N == K`). The collection
                            // sizing solver owns the coupling and sizes the
                            // array to N once N is final.
                            if !self.is_rand_dyn_size_expr(handle, right) {
                                let val = self.eval_expr(right);
                                return self.set_prop_if_changed(handle, &v, val);
                            }
                        }
                    }
                    if let Some(v) = self.rand_lvalue_name(right, rand_set) {
                        if !self.is_rand_dyn_size_expr(handle, left) {
                            let val = self.eval_expr(left);
                            return self.set_prop_if_changed(handle, &v, val);
                        }
                    }
                    // §18.7 — an inline `obj.randomize() with { obj.member == … }`
                    // prefixed reference to the randomized object's own property.
                    if let Some(v) = self.prefixed_self_member(left, handle, rand_set) {
                        if !self.is_rand_dyn_size_expr(handle, right) {
                            let val = self.eval_expr(right);
                            return self.set_prop_if_changed(handle, &v, val);
                        }
                    }
                    if let Some(v) = self.prefixed_self_member(right, handle, rand_set) {
                        if !self.is_rand_dyn_size_expr(handle, left) {
                            let val = self.eval_expr(left);
                            return self.set_prop_if_changed(handle, &v, val);
                        }
                    }
                    // §18.4: the target may be a member of an aggregate rand
                    // property, or a property of a `rand` object handle solved
                    // concurrently with this one.
                    if let Some(t) = self.rand_member_target(left, rand_set) {
                        let val = self.eval_expr(right);
                        return self.set_rand_member(&t, val);
                    }
                    if let Some(t) = self.rand_member_target(right, rand_set) {
                        let val = self.eval_expr(left);
                        return self.set_rand_member(&t, val);
                    }
                    // §18.5.9 global (cross-object) constraint: the lvalue is a
                    // rand member of a class-handle member of `this`
                    // (`sub_inst.sub_data == test_queue[0] + 8'd5`). Both sides
                    // are solved simultaneously per the LRM; here the enclosing
                    // object's variables are already drawn, so the sub-object's
                    // variable takes the implied value.
                    if let Some((sub, field)) = self.cross_obj_lvalue(left) {
                        let val = self.eval_expr(right);
                        return self.set_prop_if_changed(sub, &field, val);
                    }
                    if let Some((sub, field)) = self.cross_obj_lvalue(right) {
                        let val = self.eval_expr(left);
                        return self.set_prop_if_changed(sub, &field, val);
                    }
                    // §18.5 — whole-element equality between two elements of a
                    // rand FIXED/DYNAMIC array property: `cfg[0][0] ==
                    // cfg[1][0]` (the 5446 reg-map cross-element coupling).
                    // Neither operand is a bare scalar rand prop nor a member
                    // target, so copy RHS's element into LHS's element; the
                    // surrounding fixpoint de-couples it from the member
                    // `inside` repairs, and every trial is re-validated by the
                    // satisfaction pass.
                    let le = self.rand_whole_elem_lvalue(handle, left, rand_set);
                    let re = self.rand_whole_elem_lvalue(handle, right, rand_set);
                    if le.is_some() && re.is_some() {
                        let lk = le.as_ref().map(|(a, b)| (a.clone(), b.clone()));
                        let rk = re.as_ref().map(|(a, b)| (a.clone(), b.clone()));
                        if lk == rk {
                            return false; // `cfg[0][0]==cfg[0][0]`: tautology
                        }
                        let (la, ls) = le.unwrap();
                        let (ra, rs) = re.unwrap();
                        let src = self
                            .class_elem_load(handle, &ra, &rs)
                            .or_else(|| self.get_signal_value_by_name(&format!("{}{}", ra, rs)))
                            .unwrap_or_else(|| Value::zero(64));
                        self.class_elem_store(handle, &la, &ls, src.clone());
                        self.set_signal_value_by_name(&format!("{}{}", la, ls), src);
                        return true;
                    }
                    // A rand COLLECTION element (`q[0] == 8'd99`).
                    let lk = self.coll_elem_expr_key(left);
                    let rk = self.coll_elem_expr_key(right);
                    if lk.is_some() != rk.is_some() {
                        let (key, other) = if let Some(k) = lk {
                            (k, right)
                        } else {
                            (rk.unwrap(), left)
                        };
                        let w = self
                            .read_coll_elem(&key)
                            .map(|v| v.width)
                            .unwrap_or(32)
                            .max(1);
                        let val = self.eval_expr(other).resize(w);
                        let changed = self
                            .read_coll_elem(&key)
                            .is_none_or(|c| c.to_u64() != val.to_u64());
                        self.write_coll_elem(&key, val);
                        return changed;
                    }
                    // §18.5.12 — neither side is a bare rand target: the
                    // equality is ALGEBRAIC (`int_val + $signed({1'b0, l_val})
                    // == 32'd50`, `(r_val - 8'd10) * (l_val + 8'd5) == 16'd0`).
                    // Solve it as an affine equation in one rand variable.
                    self.solve_affine_eq(handle, left, right, rand_set)
                }
                // `a != b` (the desugared form of `unique {…}`, §18.5.5, and
                // plain inequality constraints): when the two sides are
                // currently equal, re-pick one rand side to a fresh value
                // that differs from the other side. The surrounding fixpoint
                // loop re-checks every pair each pass, so transitive
                // collisions introduced by a re-pick converge over passes.
                ExprKind::Binary {
                    op: BinaryOp::Neq,
                    left,
                    right,
                } => {
                    let lv = self.eval_expr(left);
                    let rv = self.eval_expr(right);
                    if lv.to_u64() != rv.to_u64() {
                        return false; // already satisfied
                    }
                    use rand::RngExt;
                    let avoid = lv.to_u64().unwrap_or(0);
                    // §18.5.5: a `unique {a, b, arr[0:2]}` pair may name an
                    // ARRAY ELEMENT. Re-picking the element leaves the scalar's
                    // own `inside` range intact, so an element target is
                    // preferred over a scalar one (re-picking `unique_a` here
                    // used to blow up `unique_a inside {[1:10]}`).
                    for side in [right, left] {
                        let Some(key) = self.coll_elem_expr_key(side) else {
                            continue;
                        };
                        // §18.3: an element of a state collection
                        // (`x != q[i]`, `q` not rand) is never assigned.
                        if !self.coll_key_is_rand(handle, &key) {
                            continue;
                        }
                        let w = self
                            .read_coll_elem(&key)
                            .map(|v| v.width)
                            .unwrap_or(8)
                            .max(1);
                        let mask = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
                        // §18.5.12: re-pick inside the element's hard domain,
                        // or the `inside` item and this one undo each other.
                        let prop = key
                            .split_once('#')
                            .map_or(key.as_str(), |(_, b)| b)
                            .split('[')
                            .next()
                            .unwrap_or("");
                        let domain = self.rand_elem_ranges.get(prop).cloned().unwrap_or_default();
                        if !domain.is_empty() {
                            for _try in 0..32 {
                                let (lo, hi) = domain[self.cur_rng().random_range(0..domain.len())];
                                let v = if hi > lo {
                                    self.cur_rng().random_range(lo..=hi)
                                } else {
                                    lo
                                };
                                let cand = (v as u64) & mask;
                                if cand != avoid & mask {
                                    self.write_coll_elem(&key, Value::from_u64(cand, w));
                                    return true;
                                }
                            }
                            continue;
                        }
                        for _try in 0..32 {
                            let cand = self.cur_rng().random::<u64>() & mask;
                            if cand != avoid {
                                self.write_coll_elem(&key, Value::from_u64(cand, w));
                                return true;
                            }
                        }
                    }
                    // Prefer re-picking a rand side (right first so that a
                    // `rand != const` constraint with the const on the left
                    // still resolves).
                    let (target, other_val) =
                        if let Some(v) = self.rand_lvalue_name(right, rand_set) {
                            (v, lv)
                        } else if let Some(v) = self.rand_lvalue_name(left, rand_set) {
                            (v, rv)
                        } else {
                            return false; // neither side is rand — nothing to force
                        };
                    let width = other_val.width.max(1);
                    let avoid = other_val.to_u64().unwrap_or(0);
                    // Enum-typed field: re-pick among valid members only.
                    if let Some(et) = self.prop_enum_type(handle, &target) {
                        let members: Vec<u64> = self
                            .module
                            .enum_members
                            .get(&et)
                            .map(|m| m.iter().map(|x| x.1).filter(|v| *v != avoid).collect())
                            .unwrap_or_default();
                        if members.is_empty() {
                            return false;
                        }
                        let pick = members[self.cur_rng().random_range(0..members.len())];
                        return self.set_prop_if_changed(
                            handle,
                            &target,
                            Value::from_u64(pick, width),
                        );
                    }
                    // §18.5.12: the re-pick must stay inside the target's own
                    // declared range — the constraint set is solved as a whole.
                    let signed = self.class_prop_signed_of(handle, &target);
                    if let Some(ranges) = self.rand_ranges.get(&target).cloned() {
                        let avoid_i = Self::as_ctx(avoid as i128, width, signed);
                        for _try in 0..32 {
                            let Some(p) = self.pick_i128_range(&ranges, width, signed) else {
                                break;
                            };
                            if p.to_u64() != Some(avoid)
                                && Self::as_ctx(p.to_u64().unwrap_or(0) as i128, width, signed)
                                    != avoid_i
                            {
                                return self.set_prop_if_changed(handle, &target, p);
                            }
                        }
                        return false; // range exhausted — a retry will re-draw
                    }
                    for _try in 0..32 {
                        let cand = if width >= 64 {
                            self.cur_rng().random::<u64>()
                        } else {
                            self.cur_rng().random::<u64>() & ((1u64 << width) - 1)
                        };
                        if cand != avoid {
                            return self.set_prop_if_changed(
                                handle,
                                &target,
                                Value::from_u64(cand, width),
                            );
                        }
                    }
                    false
                }
                ExprKind::Inside {
                    expr: inner,
                    ranges,
                } => {
                    let cr = || -> Vec<ConstraintRange> {
                        ranges
                            .iter()
                            .map(|r| match &r.kind {
                                ExprKind::Range(lo, hi) => ConstraintRange::Range {
                                    lo: (**lo).clone(),
                                    hi: (**hi).clone(),
                                },
                                _ => ConstraintRange::Value(r.clone()),
                            })
                            .collect()
                    };
                    if let Some(v) = self.rand_lvalue_name(inner, rand_set) {
                        let cr = cr();
                        let cur = self.eval_expr(inner);
                        if self.value_in_ranges(&cur, &cr) {
                            return false;
                        }
                        let width = cur.width.max(1);
                        if let Some(picked) = self.pick_from_ranges(&cr, width, cur.is_signed) {
                            return self.set_prop_if_changed(handle, &v, picked);
                        }
                    } else if let Some(t) = self.rand_agg_member_target(inner, rand_set) {
                        // §18.4: a member (or a select of one) of an aggregate
                        // rand property, as in the `ConstraintItem::Inside` arm.
                        let cr = cr();
                        let Some(cur) = self.read_rand_member(&t) else {
                            return false;
                        };
                        if self.value_in_ranges(&cur, &cr) {
                            return false;
                        }
                        let width = cur.width.max(1);
                        if let Some(picked) = self.pick_from_ranges(&cr, width, cur.is_signed) {
                            return self.set_rand_member(&t, picked);
                        }
                    }
                    false
                }
                _ => false,
            },
            ConstraintItem::Inside {
                expr,
                range,
                is_dist,
                dist_weights,
                ..
            } => {
                if let Some(v) = self.rand_lvalue_name(expr, rand_set) {
                    let cur = self.eval_expr(expr);
                    let is_weighted = *is_dist && !dist_weights.is_empty();
                    // For non-weighted `inside`, if the current value already
                    // satisfies the constraint we skip re-picking. For
                    // weighted `dist`, we pick EXACTLY ONCE per randomize
                    // call (gated by `dist_picked_once`) and then skip
                    // subsequent iterations of the fixpoint loop. Without
                    // the gate the empirical distribution biases toward the
                    // higher-weighted values because each iteration re-rolls
                    // and the loop tends to exit on a run of "no-change"
                    // picks — which favors the high-probability outcome.
                    if is_weighted {
                        let key = (handle, v.clone());
                        if self.dist_picked_once.contains(&key) {
                            return false;
                        }
                        let width = cur.width.max(1);
                        // §18.5.4/§5.7.1: a dist item's value comes from an
                        // unsized decimal literal, which is SIGNED. Narrowed to
                        // the target's width that flipped an MSB-set value
                        // negative (`5` in a 3-bit `bit` field read as -3), the
                        // membership check then rejected it and the trial
                        // retried until a low value was drawn — so a `dist`
                        // never produced its upper values. The pick takes the
                        // TARGET's signedness, not the literal's.
                        let signed = cur.is_signed;
                        let picked = self
                            .pick_dist_value(Some(key.clone()), range, dist_weights)
                            .map(|p| {
                                let mut p = p.resize(width);
                                p.is_signed = signed;
                                p
                            });
                        if let Some(p) = picked {
                            self.dist_picked_once.insert(key);
                            return self.set_prop_if_changed(handle, &v, p);
                        }
                        return false;
                    }
                    if self.value_in_ranges(&cur, range) {
                        return false;
                    }
                    let width = cur.width.max(1);
                    let picked = self.pick_from_ranges(range, width, cur.is_signed);
                    if let Some(p) = picked {
                        return self.set_prop_if_changed(handle, &v, p);
                    }
                } else if let Some(t) = self.rand_agg_member_target(expr, rand_set) {
                    // §18.4/§18.5.4: the same for a MEMBER (or a select of a
                    // member) of an aggregate rand property, or a property of a
                    // rand object handle. Without it the struct was drawn as
                    // one uniform value and the item only filtered trials, so
                    // a narrow range on a wide member never passed.
                    let Some(cur) = self.read_rand_member(&t) else {
                        return false;
                    };
                    let width = cur.width.max(1);
                    if *is_dist && !dist_weights.is_empty() {
                        let key = (handle, Self::rand_member_key(expr));
                        if self.dist_picked_once.contains(&key) {
                            return false;
                        }
                        let picked = self
                            .pick_dist_value(Some(key.clone()), range, dist_weights)
                            .map(|p| {
                                let mut p = p.resize(width);
                                p.is_signed = cur.is_signed;
                                p
                            });
                        if let Some(p) = picked {
                            self.dist_picked_once.insert(key);
                            return self.set_rand_member(&t, p);
                        }
                        return false;
                    }
                    if self.value_in_ranges(&cur, range) {
                        return false;
                    }
                    if let Some(p) = self.pick_from_ranges(range, width, cur.is_signed) {
                        return self.set_rand_member(&t, p);
                    }
                }
                false
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                if self.eval_expr(condition).is_true() {
                    self.solve_forced(handle, then_item, rand_set)
                } else if let Some(ei) = else_item {
                    self.solve_forced(handle, ei, rand_set)
                } else {
                    false
                }
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } if self.eval_expr(condition).is_true() => {
                self.solve_forced(handle, constraint, rand_set)
            }
            ConstraintItem::Block(items) => {
                let mut changed = false;
                for it in items {
                    if self.solve_forced(handle, it, rand_set) {
                        changed = true;
                    }
                }
                changed
            }
            ConstraintItem::Soft(inner) => self.solve_forced(handle, inner, rand_set),
            // §18.5.7 — `foreach (arr[i]) <body>`: bind `i` to each valid
            // index, substitute it in the body, and solve. The array's
            // size is resolved from `array_properties` (fixed-size rand
            // arrays) or `arrays` (unpacked dim).
            ConstraintItem::Foreach {
                array,
                vars,
                item: body,
                ..
            } => {
                // `foreach (arr[i]) soft …`: the caller already decided this
                // soft item applies (`soft_dropped`, §18.5.14), so repair its
                // body like a hard one.
                let body = match body.as_ref() {
                    ConstraintItem::Soft(inner) => inner,
                    _ => body,
                };
                // Resolve the array's bare property name (`arr[i]` → "arr").
                let arr_name = match &array.kind {
                    ExprKind::Index { expr: b, .. } => {
                        if let ExprKind::Ident(h) = &b.kind {
                            if h.path.len() == 1 {
                                Some(h.path[0].name.name.clone())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    ExprKind::Ident(h) if h.path.len() == 1 => Some(h.path[0].name.name.clone()),
                    _ => None,
                };
                let arr_name = match arr_name {
                    Some(n) => n,
                    None => {
                        // §18.4/§18.5.8: an array MEMBER of a rand struct
                        // property (`foreach (u.arr[i]) u.arr[i] inside …`).
                        // Its elements are member targets the body's own
                        // items resolve, so solve the body per index.
                        let Some(dims) = self.member_foreach_dims(handle, array) else {
                            return false;
                        };
                        let names: Vec<String> = vars
                            .iter()
                            .filter_map(|v| v.as_ref().map(|id| id.name.clone()))
                            .collect();
                        if names.is_empty() || names.len() > dims.len() {
                            return false;
                        }
                        let mut changed = false;
                        self.for_each_index(&names, &dims, |sim| {
                            if sim.solve_forced(handle, body, rand_set) {
                                changed = true;
                            }
                            None::<()>
                        });
                        return changed;
                    }
                };
                // §18.5.8: 1-D and N-D fixed shapes alike, walking the
                // inheritance chain (`fixed_foreach_dims`). Only the 1-D table
                // was consulted originally and only vars[0] bound, so a
                // `foreach (m[i,j])` constraint found no range and was dropped
                // silently — randomize() still returned 1 and every element
                // came out unconstrained.
                let Some((dims, elem_w)) = self
                    .fixed_foreach_dims(handle, &arr_name)
                    .or_else(|| self.packed_prop_foreach_dims(handle, &arr_name))
                else {
                    // §18.5.8.1: a loop over a state vector (`bit [0:3]
                    // it;`) only supplies indices: solve the body once per
                    // index, as an unrolled loop would.
                    if rand_set.contains(&arr_name) || self.prop_is_rand(handle, &arr_name) {
                        return false;
                    }
                    let names: Vec<String> = vars
                        .iter()
                        .filter_map(|v| v.as_ref().map(|id| id.name.clone()))
                        .collect();
                    let Some(dims) = self.state_foreach_dims(handle, array, &arr_name, names.len())
                    else {
                        return false;
                    };
                    if names.is_empty() || names.len() > dims.len() {
                        return false;
                    }
                    let mut changed = false;
                    self.for_each_index(&names, &dims, |sim| {
                        if sim.solve_forced(handle, body, rand_set) {
                            changed = true;
                        }
                        None::<()>
                    });
                    return changed;
                };
                // One loop variable per leading dimension (§12.7.3).
                let idx_names: Vec<String> = vars
                    .iter()
                    .filter_map(|v| v.as_ref().map(|id| id.name.clone()))
                    .collect();
                if idx_names.is_empty() || idx_names.len() > dims.len() {
                    return false;
                }
                let used = &dims[..idx_names.len()];
                if used.iter().any(|&(lo, hi)| hi < lo) {
                    return false;
                }
                let mut changed = false;
                let mut idx: Vec<i64> = used.iter().map(|d| d.0).collect();
                // §18.4: the elements of a rand collection of handles are
                // object references, never values to draw. Solve the body
                // with the iterators bound, so `arr[i].fld == …` reaches the
                // member of element i's object.
                if self.prop_class_type(handle, &arr_name).is_some() {
                    loop {
                        let frame: HashMap<String, Value> = idx_names
                            .iter()
                            .zip(&idx)
                            .map(|(nm, &i)| (nm.clone(), Self::signed_loop_val(i)))
                            .collect();
                        self.push_local_frame(frame);
                        if self.solve_forced(handle, body, rand_set) {
                            changed = true;
                        }
                        self.pop_local_frame();
                        let mut k = used.len();
                        loop {
                            if k == 0 {
                                return changed;
                            }
                            k -= 1;
                            idx[k] += 1;
                            if idx[k] <= used[k].1 {
                                break;
                            }
                            idx[k] = used[k].0;
                        }
                    }
                }
                loop {
                    // Bind every index and address the element by its full
                    // index path — `arr[i][j]`, not just the first dimension.
                    let mut frame: HashMap<String, Value> = HashMap::default();
                    let mut suffix = String::new();
                    for (k, nm) in idx_names.iter().enumerate() {
                        frame.insert(nm.clone(), Self::signed_loop_val(idx[k]));
                        suffix.push_str(&format!("[{}]", idx[k]));
                    }
                    self.push_local_frame(frame);
                    let mut elem_rand_set = rand_set.clone();
                    elem_rand_set.insert(format!("{}{}", arr_name, suffix));
                    let mut hint: Vec<(i64, i64)> = Vec::new();
                    self.body_inside_ranges(body, &arr_name, &mut hint);
                    self.elem_inside_hint = hint;
                    if self.solve_forced_array_elem(
                        handle,
                        body.as_ref(),
                        &elem_rand_set,
                        &arr_name,
                        &suffix,
                    ) {
                        changed = true;
                    }
                    // Structural repair covers equality/relational/inside
                    // shapes. Anything else the body demands of THIS element
                    // (`% 2 == 0`, bit tests, …) falls to generate-and-test —
                    // drawn from the body's own `inside` ranges, since a
                    // full-width draw would virtually never land in a narrow
                    // one. Cross-element couplings that no local draw can fix
                    // are left to the trial checker, which retries or fails
                    // the randomize() (§18.6.2).
                    if idx_names.len() == dims.len()
                        && elem_w >= 1
                        && elem_w <= 64
                        && !Self::constraint_unmodeled(body)
                        && !self.item_holds(handle, body)
                    {
                        use rand::RngExt;
                        let mut ranges: Vec<(i64, i64)> = Vec::new();
                        self.body_inside_ranges(body, &arr_name, &mut ranges);
                        let scoped_key = format!("{}#{}{}", handle, arr_name, suffix);
                        let elem_name = format!("{}{}", arr_name, suffix);
                        let saved = self.signals.get(&scoped_key).cloned();
                        let signed = self.class_prop_signed_of(handle, &arr_name);
                        let mut ok = false;
                        // 24 draws: a satisfiable body (parity, bit tests)
                        // lands in a handful; an impossible one burns every
                        // draw for every pass of every trial, so the cap is
                        // what bounds the UNSAT give-up time.
                        for _ in 0..24 {
                            let mut cand = if ranges.is_empty() {
                                let mask = if elem_w >= 64 {
                                    u64::MAX
                                } else {
                                    (1u64 << elem_w) - 1
                                };
                                Value::from_u64(self.cur_rng().random::<u64>() & mask, elem_w)
                            } else {
                                let (l, h) = ranges[self.cur_rng().random_range(0..ranges.len())];
                                let v = if h > l {
                                    self.cur_rng().random_range(l..=h)
                                } else {
                                    l
                                };
                                Value::from_u64(v as u64, elem_w)
                            };
                            cand.is_signed = signed;
                            self.signals.insert(scoped_key.clone(), cand.clone());
                            // The unscoped form only for a module-level array
                            // of that name (#519 B3: a bare copy minted for a
                            // class property took a later object's write).
                            if self.module.arrays.contains_key(&arr_name)
                                || self.signal_name_to_id.contains_key(elem_name.as_str())
                            {
                                self.set_signal_value_by_name(&elem_name, cand);
                            }
                            if self.item_holds(handle, body) {
                                ok = true;
                                changed = true;
                                break;
                            }
                        }
                        if !ok {
                            if let Some(sv) = saved {
                                self.signals.insert(scoped_key.clone(), sv.clone());
                                if self.module.arrays.contains_key(&arr_name)
                                    || self.signal_name_to_id.contains_key(elem_name.as_str())
                                {
                                    self.set_signal_value_by_name(&elem_name, sv);
                                }
                            }
                        }
                    }
                    self.elem_inside_hint.clear();
                    self.pop_local_frame();
                    let mut k = used.len();
                    loop {
                        if k == 0 {
                            return changed;
                        }
                        k -= 1;
                        idx[k] += 1;
                        if idx[k] <= used[k].1 {
                            break;
                        }
                        idx[k] = used[k].0;
                    }
                }
            }
            // §18.5.5 `unique {arr}` over a whole rand fixed array: make the
            // elements pairwise distinct. (Multi-expression `unique` lists
            // were already desugared to pairwise `!=` by the parser; only
            // the single-expression whole-array form reaches this variant.)
            ConstraintItem::Unique { exprs, .. } => {
                let prop = match exprs.first().map(|e| &e.kind) {
                    Some(ExprKind::Ident(h)) if h.path.len() == 1 => h.path[0].name.name.clone(),
                    _ => return false,
                };
                if !rand_set.contains(&prop) {
                    return false;
                }
                // Element range/width from array_properties, walking the
                // inheritance chain (same lookup as the Foreach arm).
                let mut range: Option<(i64, i64, u32)> = None;
                let mut cn = match self
                    .heap
                    .get(handle)
                    .and_then(|x| x.as_ref())
                    .map(|i| i.class_name.clone())
                {
                    Some(c) => c,
                    None => return false,
                };
                loop {
                    if let Some(cd) = self.module.classes.get(&cn) {
                        if let Some(&(lo, hi, w)) = cd.array_properties.get(&prop) {
                            let (lo, hi) = self.inst_fixed_bounds(cd, handle, &prop, (lo, hi));
                            range = Some((lo, hi, w));
                            break;
                        }
                        if let Some(parent) = cd.extends.clone() {
                            cn = parent;
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                // Every element as its index suffix: `[i]` for a 1-D array,
                // `[i][j]…` for an N-D one (§18.5.5: `unique {m}` covers all
                // the elements of a multi-dimensional array; only the 1-D
                // table was consulted, so a 2-D `unique` repaired nothing).
                let (suffixes, w): (Vec<String>, u32) = match range {
                    Some((lo, hi, w)) => ((lo..=hi).map(|i| format!("[{}]", i)).collect(), w),
                    None => match self.fixed_foreach_dims(handle, &prop) {
                        Some((dims, w)) if dims.len() > 1 => (Self::index_suffixes(&dims), w),
                        _ => return false,
                    },
                };
                // Read the elements (scoped per-instance key first — where
                // exec_randomize writes the baseline — then unscoped).
                let read_elem = |s: &Self, sfx: &str| -> Option<Value> {
                    s.signals
                        .get(&format!("{}#{}{}", handle, prop, sfx))
                        .cloned()
                        .or_else(|| s.signals.get(&format!("{}{}", prop, sfx)).cloned())
                };
                let enum_pool: Vec<u64> = self
                    .prop_enum_type(handle, &prop)
                    .and_then(|et| self.module.enum_members.get(&et).cloned())
                    .map(|m| m.iter().map(|x| x.1).collect())
                    .unwrap_or_default();
                let mut used: HashSet<u64> = HashSet::default();
                let mut changed = false;
                use rand::RngExt;
                for sfx in &suffixes {
                    let cur = read_elem(self, sfx).and_then(|v| v.to_u64());
                    let needs_repick = match cur {
                        Some(cv) => !used.insert(cv),
                        None => true,
                    };
                    if !needs_repick {
                        continue;
                    }
                    // Duplicate (or unreadable) element: re-pick a value not
                    // yet used — from the enum pool when the element type is
                    // an enum, otherwise width-masked random with retries.
                    let pick = if !enum_pool.is_empty() {
                        let avail: Vec<u64> = enum_pool
                            .iter()
                            .copied()
                            .filter(|v| !used.contains(v))
                            .collect();
                        if avail.is_empty() {
                            continue; // domain exhausted — leave as-is
                        }
                        Some(avail[self.cur_rng().random_range(0..avail.len())])
                    } else {
                        let mask = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
                        (0..64)
                            .map(|_| self.cur_rng().random::<u64>() & mask)
                            .find(|c| !used.contains(c))
                    };
                    if let Some(p) = pick {
                        used.insert(p);
                        let v = Value::from_u64(p, w);
                        self.signals
                            .insert(format!("{}#{}{}", handle, prop, sfx), v.clone());
                        self.set_signal_value_by_name(&format!("{}{}", prop, sfx), v);
                        changed = true;
                    }
                }
                changed
            }
            _ => false,
        }
    }

    /// Foreach helper: given the loop index already bound on the local frame,
    /// solve a constraint body for `arr[i]`. The element's flat name
    /// `arr[<i>]` is the target rather than the bare `arr`.
    /// True when `e` is a chain of index selects rooted at the bare name
    /// `arr` — `m[i]` and `m[i][j]` alike. One level of `Index` was not enough
    /// for a 2-D foreach body.
    pub(super) fn index_chain_root_is(e: &Expression, arr: &str) -> bool {
        match &e.kind {
            ExprKind::Index { expr: b, .. } => match &b.kind {
                ExprKind::Ident(h) => h.path.len() == 1 && h.path[0].name.name == arr,
                _ => Self::index_chain_root_is(b, arr),
            },
            _ => false,
        }
    }

    pub(super) fn solve_forced_array_elem(
        &mut self,
        handle: usize,
        item: &ConstraintItem,
        rand_set: &HashSet<String>,
        arr_name: &str,
        idx: &str,
    ) -> bool {
        // §18.5.7 fixed-array foreach over a PACKED-STRUCT element: the body
        // constrains a MEMBER of the element (`foreach (cfg[y]) cfg[y].au_bytes
        // inside {1,2,4,8,16}`). `index_chain_root_is` only matches a bare
        // `arr[i]`, so these fell through to the generate-and-test fallback,
        // which draws the WHOLE element uniformly and can never hit a narrow
        // member range — randomize() failed. Repair the member slice directly.
        if let Some(changed) = self.solve_forced_array_elem_member(handle, item, arr_name, idx) {
            return changed;
        }
        // Handle `arr[i] inside { lo:hi }` directly so the range is
        // evaluated with the bound `i`. Other shapes can be added later.
        // Also handle the dedicated ConstraintItem::Inside variant.
        if let ConstraintItem::Inside {
            expr: inner, range, ..
        } = item
        {
            let inside_arr = Self::index_chain_root_is(inner, arr_name);
            if inside_arr {
                let elem_name = format!("{}{}", arr_name, idx);
                // A CLASS array element lives under the instance-scoped key;
                // the bare name only exists for a module-level array.
                let cur = self
                    .class_elem_load(handle, arr_name, idx)
                    .or_else(|| self.get_signal_value_by_name(&elem_name))
                    .unwrap_or_else(|| Value::zero(32));
                if self.value_in_ranges(&cur, range) {
                    return false;
                }
                let width = cur.width.max(32);
                if let Some(picked) = self.pick_from_ranges(range, width, cur.is_signed) {
                    self.class_elem_store(handle, arr_name, idx, picked.clone());
                    self.set_signal_value_by_name(&elem_name, picked.clone());
                    return true;
                }
            }
        }
        match item {
            ConstraintItem::Expr(e) => match &e.kind {
                ExprKind::Inside {
                    expr: inner,
                    ranges,
                } => {
                    let inside_arr = Self::index_chain_root_is(inner, arr_name);
                    if !inside_arr {
                        return false;
                    }
                    let elem_name = format!("{}{}", arr_name, idx);
                    let cr: Vec<ConstraintRange> = ranges
                        .iter()
                        .map(|r| match &r.kind {
                            ExprKind::Range(lo, hi) => ConstraintRange::Range {
                                lo: (**lo).clone(),
                                hi: (**hi).clone(),
                            },
                            _ => ConstraintRange::Value(r.clone()),
                        })
                        .collect();
                    let scoped_cur = format!("{}#{}{}", handle, arr_name, idx);
                    let cur = self
                        .get_signal_value_by_name(&scoped_cur)
                        .or_else(|| self.get_signal_value_by_name(&elem_name))
                        .unwrap_or_else(|| Value::zero(32));
                    if self.value_in_ranges(&cur, &cr) {
                        return false;
                    }
                    let width = cur.width.max(32);
                    if let Some(picked) = self.pick_from_ranges(&cr, width, cur.is_signed) {
                        // Write to the scoped per-element key (`<handle>#arr[i]`)
                        // — the place exec_randomize wrote the random baseline.
                        self.class_elem_store(handle, arr_name, idx, picked.clone());
                        // The unscoped form only for a MODULE-level array of
                        // that name: for a class property it minted a phantom
                        // bare `a[i]` that a later method's `a[i] = v` of
                        // another object then wrote instead of its own
                        // element (#519 B3).
                        if self.module.arrays.contains_key(arr_name)
                            || self.signal_name_to_id.contains_key(elem_name.as_str())
                        {
                            self.set_signal_value_by_name(&elem_name, picked);
                        }
                        return true;
                    }
                    false
                }
                // §18.5.7 equality / relational element constraints in a class
                // foreach body — `foreach (arr[i]) arr[i] == i + 5` (issue
                // #28's shape) or `arr[i] < K`. The loop var is bound in the
                // caller's local frame, so index and bound expressions
                // evaluate with the concrete `i`.
                ExprKind::Binary { op, left, right } => {
                    // Element ref: `arr[<index-expr that evals to idx>]`.
                    // The full index PATH of an element ref rooted at `arr`,
                    // matching the `[i]`/`[i][j]` suffix the caller iterates.
                    fn elem_index(
                        s: &mut Simulator,
                        e: &Expression,
                        arr_name: &str,
                    ) -> Option<String> {
                        let ExprKind::Index { expr: b, index } = &e.kind else {
                            return None;
                        };
                        let here = format!("[{}]", s.eval_expr(index).to_i64()?);
                        match &b.kind {
                            ExprKind::Ident(h)
                                if h.path.len() == 1 && h.path[0].name.name == arr_name =>
                            {
                                Some(here)
                            }
                            _ => elem_index(s, b, arr_name).map(|outer| outer + &here),
                        }
                    }
                    let on_l = elem_index(self, left, arr_name).as_deref() == Some(idx);
                    let on_r = elem_index(self, right, arr_name).as_deref() == Some(idx);
                    if on_l == on_r {
                        return false; // cross-element or unrelated — skip
                    }
                    // Normalize to `arr[i] OP bound`.
                    let (bound_side, eff) = if on_l {
                        (right, *op)
                    } else {
                        let m = match op {
                            BinaryOp::Lt => BinaryOp::Gt,
                            BinaryOp::Gt => BinaryOp::Lt,
                            BinaryOp::Leq => BinaryOp::Geq,
                            BinaryOp::Geq => BinaryOp::Leq,
                            other => *other,
                        };
                        (left, m)
                    };
                    let elem_name = format!("{}{}", arr_name, idx);
                    let cur = self
                        .get_signal_value_by_name(&elem_name)
                        .or_else(|| self.class_elem_load(handle, arr_name, idx));
                    let w = cur.as_ref().map(|v| v.width).unwrap_or(32).max(1);
                    let write_elem = |s: &mut Self, v: Value| {
                        s.class_elem_store(handle, arr_name, idx, v.clone());
                        if s.module.arrays.contains_key(arr_name)
                            || s.signal_name_to_id.contains_key(elem_name.as_str())
                        {
                            s.set_signal_value_by_name(&elem_name, v);
                        }
                    };
                    match eff {
                        BinaryOp::Eq => {
                            let v = self.eval_expr(bound_side).resize(w);
                            let changed = cur
                                .as_ref()
                                .map(|c| c.to_u64() != v.to_u64())
                                .unwrap_or(true);
                            write_elem(self, v);
                            changed
                        }
                        BinaryOp::Lt | BinaryOp::Leq | BinaryOp::Gt | BinaryOp::Geq => {
                            use rand::RngExt;
                            let b = self.eval_expr(bound_side).to_i64().unwrap_or(0);
                            let cur_i = cur.and_then(|v| v.to_i64()).unwrap_or(0);
                            // The element type's domain (§6.11): [0, 2^w - 1],
                            // or [-2^(w-1), 2^(w-1) - 1] for a signed element —
                            // an unsigned one rejected every negative value
                            // (`q[i] >= -1` re-picked a legal -1 to >= 0).
                            let signed = self.class_prop_signed_of(handle, arr_name);
                            let (mut lo, mut hi) = if signed {
                                if w >= 64 {
                                    (i64::MIN, i64::MAX)
                                } else {
                                    (-(1i64 << (w - 1)), (1i64 << (w - 1)) - 1)
                                }
                            } else {
                                (0i64, if w >= 63 { i64::MAX } else { (1i64 << w) - 1 })
                            };
                            match eff {
                                BinaryOp::Lt => hi = hi.min(b.saturating_sub(1)),
                                BinaryOp::Leq => hi = hi.min(b),
                                BinaryOp::Gt => lo = lo.max(b.saturating_add(1)),
                                BinaryOp::Geq => lo = lo.max(b),
                                _ => {}
                            }
                            // Intersect with the body's own `inside` ranges for
                            // this element — a repick outside them only trades
                            // one violation for another.
                            if !self.elem_inside_hint.is_empty() {
                                let mut best: Option<(i64, i64)> = None;
                                for &(rl, rh) in &self.elem_inside_hint {
                                    let (l2, h2) = (lo.max(rl), hi.min(rh));
                                    if l2 <= h2 {
                                        best = Some(match best {
                                            None => (l2, h2),
                                            Some((bl, bh)) => (bl.min(l2), bh.max(h2)),
                                        });
                                    }
                                }
                                match best {
                                    Some((l2, h2)) => {
                                        lo = l2;
                                        hi = h2;
                                    }
                                    None => return false,
                                }
                            }
                            if lo > hi {
                                return false; // unsatisfiable here
                            }
                            // Tight mode sets the bound even when the current
                            // draw already satisfies this item: the `inside`
                            // repair ran first and left a random value, and
                            // keeping it is what exhausts a long chain.
                            if !self.rand_tight_mode && cur_i >= lo && cur_i <= hi {
                                return false; // already OK
                            }
                            let p = if lo == hi {
                                lo
                            } else if self.rand_tight_mode {
                                // Minimal/maximal feasible: leaves the most
                                // room for the elements that depend on this one.
                                match eff {
                                    BinaryOp::Gt | BinaryOp::Geq => lo,
                                    _ => hi,
                                }
                            } else {
                                self.cur_rng().random_range(lo..=hi)
                            };
                            if p == cur_i {
                                return false;
                            }
                            let mut v = Value::from_u64(p as u64, w);
                            v.is_signed = signed;
                            write_elem(self, v);
                            true
                        }
                        _ => false,
                    }
                }
                _ => false,
            },
            ConstraintItem::Block(items) => {
                let mut changed = false;
                for it in items {
                    if self.solve_forced_array_elem(handle, it, rand_set, arr_name, idx) {
                        changed = true;
                    }
                }
                changed
            }
            // §18.5.7: a CONDITIONAL body item — `if (i == j) arr[i] % 2 == 0;
            // else …` — was silently dropped here (only Block/Inside/Expr had
            // arms), so every index-dependent branch of a fixed-array foreach
            // went unenforced and unchecked. The loop indices are bound on the
            // local frame, so the condition evaluates per element.
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                if self.eval_expr(condition).is_true() {
                    self.solve_forced_array_elem(handle, then_item, rand_set, arr_name, idx)
                } else if let Some(ei) = else_item {
                    self.solve_forced_array_elem(handle, ei, rand_set, arr_name, idx)
                } else {
                    false
                }
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } if self.eval_expr(condition).is_true() => {
                self.solve_forced_array_elem(handle, constraint, rand_set, arr_name, idx)
            }
            ConstraintItem::Soft(inner) => {
                self.solve_forced_array_elem(handle, inner, rand_set, arr_name, idx)
            }
            _ => false,
        }
    }

    /// Repair a packed-struct MEMBER of a fixed-array element inside a foreach
    /// body — `cfg[i].au_bytes inside {lo:hi}` or `cfg[i].byte_addressing == 1`.
    /// `index_chain_root_is` (used by the bare-element arms) is blind to the
    /// `MemberAccess` wrapping the `Index`, so these were previously dropped;
    /// the generate-and-test fallback then drew the whole element uniformly
    /// and could never land a narrow member sub-range. Returns `None` if `item`
    /// is not a member-of-element Inside/equality constraint (caller falls
    /// through), otherwise `Some(changed)` — the member slice is re-picked
    /// within the constraint range and spliced back into the element.
    pub(super) fn solve_forced_array_elem_member(
        &mut self,
        handle: usize,
        item: &ConstraintItem,
        arr_name: &str,
        idx: &str,
    ) -> Option<bool> {
        // Decompose a target of the shape `<arr>[<i>].<m1>.….<mk>` into the
        // index suffix, the member-name chain, and the element Index expr.
        fn split(
            s: &mut Simulator,
            e: &Expression,
            arr_name: &str,
        ) -> Option<(String, Vec<String>, Expression)> {
            let ExprKind::MemberAccess { expr: base, member } = &e.kind else {
                return None;
            };
            let mut names = vec![member.name.clone()];
            // `base` is the ELEMENT (`cfg[x][y]`) when the member chain has a
            // single level; returned so packed_struct_elem_type can resolve
            // the element's struct layout.
            let elem_expr = (**base).clone();
            let mut cur = base;
            while let ExprKind::MemberAccess {
                expr: b2,
                member: m2,
            } = &cur.kind
            {
                names.push(m2.name.clone());
                cur = b2;
            }
            names.reverse();
            // Peel the nested multi-dimension Index chain, collecting each
            // `[i]`; reversed to the element's left-to-right `[x][y]` order.
            let mut suffixes: Vec<String> = Vec::new();
            while let ExprKind::Index { expr: b, index } = &cur.kind {
                suffixes.push(format!("[{}]", s.eval_expr(index).to_i64()?));
                cur = b;
            }
            suffixes.reverse();
            let ExprKind::Ident(h) = &cur.kind else {
                return None;
            };
            if !(h.path.len() == 1 && h.path[0].name.name == arr_name) {
                return None;
            }
            Some((suffixes.concat(), names, elem_expr))
        }

        // Which member is constrained, and by what? Returns (elem, idx-suffix,
        // member chain, ranges) — or equality bound.
        enum Op {
            Inside(Vec<ConstraintRange>),
            Eq(Expression),
        }
        let (target, op) = match item {
            ConstraintItem::Inside {
                expr: inner, range, ..
            } => (Box::new((*inner).clone()), Op::Inside(range.clone())),
            ConstraintItem::Expr(e) => match &e.kind {
                ExprKind::Inside {
                    expr: inner,
                    ranges,
                } => {
                    let cr: Vec<ConstraintRange> = ranges
                        .iter()
                        .map(|r| match &r.kind {
                            ExprKind::Range(lo, hi) => ConstraintRange::Range {
                                lo: (**lo).clone(),
                                hi: (**hi).clone(),
                            },
                            _ => ConstraintRange::Value(r.clone()),
                        })
                        .collect();
                    (Box::new((**inner).clone()), Op::Inside(cr))
                }
                ExprKind::Binary {
                    op: BinaryOp::Eq,
                    left,
                    right,
                } => {
                    // Normalize <member> == bound on either side.
                    let (m_side, b_side) = if split(self, left, arr_name).is_some() {
                        (left, right)
                    } else if split(self, right, arr_name).is_some() {
                        (right, left)
                    } else {
                        return None;
                    };
                    (Box::new((**m_side).clone()), Op::Eq((**b_side).clone()))
                }
                _ => return None,
            },
            _ => return None,
        };
        let (idxs, names, elem_expr) = split(self, &target, arr_name)?;
        if idxs != idx {
            return None; // not this element's turn — another index handles it
        }
        // Resolve the whole struct element type + member bit layout. Must use
        // the same width source as the member READ path (packed_agg_layout), so
        // the splice lands on the bits that member-selections observe.
        let su = self.packed_struct_elem_type(&elem_expr)?;
        if Self::spreads_member_wise(&su) {
            return None;
        }
        let (fields, _total) = self.packed_agg_layout(&su);
        let (off, w) = {
            let f = fields.iter().find(|(m, _, _)| m == &names[0])?;
            // Nested struct member-of-member targets (aper.outer.inner) are not
            // covered by the flat `packed_agg_layout` here — degenerate to the
            // caller's fallback rather than splice the wrong bits.
            if names.len() > 1 {
                return None;
            }
            (f.1, f.2)
        };
        let elem = self
            .class_elem_load(handle, arr_name, &idxs)
            .unwrap_or_else(|| Value::zero((off + w).max(1)));
        let signed = self.class_prop_signed_of(handle, arr_name);
        let mcur = elem.range_select((off + w - 1) as usize, off as usize);
        let changed = match op {
            Op::Inside(cr) => {
                if self.value_in_ranges(&mcur, &cr) {
                    return None;
                }
                let width = mcur.width.max(1);
                match self.pick_from_ranges(&cr, width, mcur.is_signed) {
                    Some(picked) => {
                        let same = picked.to_u64() == mcur.to_u64();
                        self.splice_elem_member(elem, handle, arr_name, &idxs, off, w, picked);
                        !same
                    }
                    None => return None,
                }
            }
            Op::Eq(bound) => {
                let bv = self.eval_expr(&bound).resize(mcur.width.max(1));
                let same = bv.to_u64() == mcur.to_u64();
                let mut p = bv;
                p.is_signed = signed;
                self.splice_elem_member(elem, handle, arr_name, &idxs, off, w, p);
                !same
            }
        };
        Some(changed)
    }

    /// Bit-splice `member_val` (width `w`) into `elem` at bit offset `off` and
    /// store the element back under `<handle>#<arr>[<idx>]`.
    pub(super) fn splice_elem_member(
        &mut self,
        mut elem: Value,
        handle: usize,
        arr_name: &str,
        idx: &str,
        off: u32,
        w: u32,
        member_val: Value,
    ) {
        for i in 0..w {
            elem.set_bit((off + i) as usize, member_val.get_bit(i as usize));
        }
        self.class_elem_store(handle, arr_name, idx, elem);
    }

    /// The FIXED shape of a class rand array member, walking the inheritance
    /// chain: 1-D from `array_properties`, N-D from `array_nd_properties`.
    /// Returns (per-dimension bounds, element width).
    /// foreach geometry of a packed multi-dimensional class property
    /// (`bit [6:0] [4:0][1:0] d`): the packed dims normalised to (lo, hi)
    /// and the width of one addressed element.
    pub(super) fn packed_prop_foreach_dims(
        &self,
        handle: usize,
        arr_name: &str,
    ) -> Option<(Vec<(i64, i64)>, u32)> {
        let pd = self.class_prop_packed_dims(handle, arr_name)?;
        let total = self.read_member_value(handle, arr_name)?.width as u64;
        let ew = Self::packed_chain_width(total, &pd, pd.len()) as u32;
        Some((pd.iter().map(|&(l, r)| (l.min(r), l.max(r))).collect(), ew))
    }

    pub(super) fn parse_idx_suffix(idx: &str) -> Option<Vec<Value>> {
        let mut out = Vec::new();
        for part in idx.split('[').skip(1) {
            let n: i64 = part.strip_suffix(']')?.trim().parse().ok()?;
            out.push(Value::from_u64(n as u64, 32));
        }
        Some(out)
    }

    /// Read one element of a class array property addressed by an index
    /// suffix (`[1][0]`): the per-element `<handle>#arr[1][0]` signal for
    /// unpacked arrays, a bit slice of the property for packed ones.
    pub(super) fn class_elem_load(
        &self,
        handle: usize,
        arr_name: &str,
        idx: &str,
    ) -> Option<Value> {
        if let Some(dims) = self.class_prop_packed_dims(handle, arr_name) {
            let base = self.read_member_value(handle, arr_name)?;
            let iv = Self::parse_idx_suffix(idx)?;
            let (lsb, w) = Self::packed_chain_slice(base.width as u64, &dims, &iv)?;
            return Some(base.range_select((lsb + w - 1) as usize, lsb as usize));
        }
        self.get_signal_value_by_name(&format!("{}#{}{}", handle, arr_name, idx))
    }

    pub(super) fn class_elem_store(
        &mut self,
        handle: usize,
        arr_name: &str,
        idx: &str,
        mut v: Value,
    ) {
        if let Some(dims) = self.class_prop_packed_dims(handle, arr_name) {
            let (Some(cur), Some(iv)) = (
                self.read_member_value(handle, arr_name),
                Self::parse_idx_suffix(idx),
            ) else {
                return;
            };
            let Some((lsb, w)) = Self::packed_chain_slice(cur.width as u64, &dims, &iv) else {
                return;
            };
            let piece = v.resize(w as u32);
            let mut nv = cur;
            for i in 0..(w as usize) {
                nv.set_bit(lsb as usize + i, piece.get_bit(i));
            }
            if let Some(Some(inst)) = self.heap.get_mut(handle) {
                inst.properties.insert(arr_name.to_string(), nv);
            }
            return;
        }
        v.is_signed = self.class_prop_signed_of(handle, arr_name);
        self.signals
            .insert(format!("{}#{}{}", handle, arr_name, idx), v);
    }

    /// §18.5.8 over a MEMBER of a struct property (`foreach (u.arr[i])`):
    /// the member's fixed unpacked bounds, one `(lo, hi)` per dimension.
    /// `None` unless `array` is a member chain under a struct property of
    /// `handle` ending at a member with fixed (range or size) dimensions.
    pub(super) fn member_foreach_dims(
        &mut self,
        handle: usize,
        array: &Expression,
    ) -> Option<Vec<(i64, i64)>> {
        let (root, members) = Self::foreach_member_chain(array)?;
        self.struct_chain_foreach_dims(handle, &root, &members)
    }

    /// The loop bounds of the struct-member chain `root.m1…mn` of `handle`
    /// (`root` a struct property): the last member's fixed unpacked
    /// dimensions, or, with none, its packed dimensions when it is a
    /// multi-dimensional packed array (§7.4.1).
    pub(super) fn struct_chain_foreach_dims(
        &mut self,
        handle: usize,
        root: &str,
        members: &[String],
    ) -> Option<Vec<(i64, i64)>> {
        use crate::ast::types::UnpackedDimension as UD;
        let mut level = self.class_prop_struct(handle, root)?;
        let (last, path) = members.split_last()?;
        for m in path {
            let (dt, dims) = Self::struct_member_decl(&level, m)?;
            if !dims.is_empty() {
                return None;
            }
            match self.resolve_dt(&dt) {
                DataType::Struct(inner) => level = inner,
                _ => return None,
            }
        }
        let (dt, dims) = Self::struct_member_decl(&level, last)?;
        if dims.is_empty() {
            let params = self.instance_param_scope(handle);
            let pd = crate::compiler::elaborate::packed_full_dims_chained(
                &dt,
                &params,
                &self.module.typedef_types,
            )?;
            return Some(pd.iter().map(|&(l, r)| (l.min(r), l.max(r))).collect());
        }
        let mut out = Vec::with_capacity(dims.len());
        for d in &dims {
            match d {
                UD::Range { left, right, .. } => {
                    let l = self.eval_expr(left).to_i64()?;
                    let r = self.eval_expr(right).to_i64()?;
                    out.push((l.min(r), l.max(r)));
                }
                UD::Expression { expr, .. } => {
                    let n = self.eval_expr(expr).to_i64()?;
                    if n <= 0 {
                        return None;
                    }
                    out.push((0, n - 1));
                }
                _ => return None,
            }
        }
        Some(out)
    }

    /// The loop bounds (lo, hi) of every dimension of `root.m1…mn` when
    /// `root` holds a class handle (as may the members after it) and the
    /// rest names a fixed unpacked or a packed multi-dimensional array of
    /// that object, or a member of one of its struct properties.
    pub(super) fn sub_member_foreach_dims(
        &mut self,
        handle: usize,
        root: &str,
        members: &[String],
    ) -> Option<Vec<(i64, i64)>> {
        self.prop_class_type(handle, root)?;
        let mut h = self.member_handle(handle, root)?;
        let mut rest = members;
        while rest.len() > 1 && self.prop_class_type(h, &rest[0]).is_some() {
            h = self.member_handle(h, &rest[0])?;
            rest = &rest[1..];
        }
        if !matches!(self.heap.get(h), Some(Some(_))) {
            return None;
        }
        match rest {
            [last] => self
                .fixed_foreach_dims(h, last)
                .or_else(|| self.packed_prop_foreach_dims(h, last))
                .map(|d| d.0),
            // A member of a struct property of the sub-object (`a.s.f`).
            [first, more @ ..] => self.struct_chain_foreach_dims(h, first, more),
            [] => None,
        }
    }

    /// The `(root, members)` of a foreach array that names a member of a
    /// struct property, in either parse shape: `Index{MemberAccess…}` (inside
    /// a method) or a flat `Ident` `[u, arr]` (a constraint block keeps the
    /// loop variables apart; a select may sit on the last segment).
    pub(super) fn foreach_member_chain(array: &Expression) -> Option<(String, Vec<String>)> {
        match &array.kind {
            ExprKind::Index { expr: base, .. } => {
                let mut members: Vec<String> = Vec::new();
                let mut cur = base.as_ref();
                while let ExprKind::MemberAccess { expr: b, member } = &cur.kind {
                    members.push(member.name.clone());
                    cur = b;
                }
                let root = Self::plain_ident_name(cur)?;
                if members.is_empty() {
                    return None;
                }
                members.reverse();
                Some((root, members))
            }
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.len() >= 2
                    && h.path[..h.path.len() - 1]
                        .iter()
                        .all(|s| s.selects.is_empty()) =>
            {
                Some((
                    h.path[0].name.name.clone(),
                    h.path[1..].iter().map(|s| s.name.name.clone()).collect(),
                ))
            }
            _ => None,
        }
    }

    /// Run `f` once per index tuple of `dims` (the first `names.len()`
    /// dimensions), with the loop variables bound in a fresh frame. Stops
    /// early when `f` returns `Some`, passing that back.
    pub(super) fn for_each_index<T>(
        &mut self,
        names: &[String],
        dims: &[(i64, i64)],
        mut f: impl FnMut(&mut Self) -> Option<T>,
    ) -> Option<T> {
        let used = &dims[..names.len().min(dims.len())];
        if used.iter().any(|&(lo, hi)| hi < lo) {
            return None;
        }
        let mut idx: Vec<i64> = used.iter().map(|d| d.0).collect();
        loop {
            let frame: HashMap<String, Value> = names
                .iter()
                .zip(&idx)
                .map(|(nm, &i)| (nm.clone(), Self::signed_loop_val(i)))
                .collect();
            self.push_local_frame(frame);
            let r = f(self);
            self.pop_local_frame();
            if r.is_some() {
                return r;
            }
            let mut k = used.len();
            loop {
                if k == 0 {
                    return None;
                }
                k -= 1;
                idx[k] += 1;
                if idx[k] <= used[k].1 {
                    break;
                }
                idx[k] = used[k].0;
            }
        }
    }

    /// `[i][j]…` for every index tuple of `dims`, first dimension slowest.
    pub(super) fn index_suffixes(dims: &[(i64, i64)]) -> Vec<String> {
        let mut out = vec![String::new()];
        for &(lo, hi) in dims {
            let mut next = Vec::with_capacity(out.len() * ((hi - lo + 1).max(0) as usize));
            for pre in &out {
                for i in lo..=hi {
                    next.push(format!("{}[{}]", pre, i));
                }
            }
            out = next;
        }
        out
    }

    /// §18.5.5: strict check of `unique {m}` over a whole MULTI-dimensional
    /// fixed rand array (1-D arrays are rand collections, checked by
    /// `coll_item_check`). Without it the item was unmodeled: a duplicate
    /// draw was accepted, and so was a pigeonhole-unsatisfiable array.
    pub(super) fn check_fixed_nd_unique(
        &mut self,
        handle: usize,
        item: &ConstraintItem,
    ) -> Option<bool> {
        let ConstraintItem::Unique { exprs, .. } = item else {
            return None;
        };
        let [e] = exprs.as_slice() else {
            return None;
        };
        let prop = Self::plain_ident_name(e)?;
        let (dims, _) = self.fixed_foreach_dims(handle, &prop)?;
        if dims.len() < 2 {
            return None;
        }
        let mut seen: HashSet<u64> = HashSet::default();
        for sfx in Self::index_suffixes(&dims) {
            let v = self
                .class_elem_load(handle, &prop, &sfx)
                .or_else(|| self.get_signal_value_by_name(&format!("{}{}", prop, sfx)))?;
            if v.has_xz() || !seen.insert(v.to_u64()?) {
                return Some(false);
            }
        }
        Some(true)
    }

    pub(super) fn fixed_foreach_dims(
        &self,
        handle: usize,
        arr_name: &str,
    ) -> Option<(Vec<(i64, i64)>, u32)> {
        let mut cn = self
            .heap
            .get(handle)
            .and_then(|x| x.as_ref())
            .map(|i| i.class_name.clone())?;
        loop {
            let cd = self.module.classes.get(&cn)?;
            if let Some(&(lo, hi, w)) = cd.array_properties.get(arr_name) {
                let (lo, hi) = self.inst_fixed_bounds(cd, handle, arr_name, (lo, hi));
                return Some((vec![(lo, hi)], w));
            }
            if let Some(sh) = self.inst_nd_shape(cd, handle, arr_name) {
                if let Some((_, w)) = cd.array_nd_properties.get(arr_name) {
                    return Some((sh, *w));
                }
            }
            if let Some((shape, w)) = cd.array_nd_properties.get(arr_name) {
                return Some((shape.clone(), *w));
            }
            cn = cd.extends.clone()?;
        }
    }

    /// The `inside {…}` ranges the foreach BODY imposes on the current
    /// element, evaluated with the loop indices bound — the candidate domain
    /// for the generate-and-test backstop (a full-width draw would virtually
    /// never land in a narrow range like `[1:500]`).
    pub(super) fn body_inside_ranges(
        &mut self,
        item: &ConstraintItem,
        arr_name: &str,
        out: &mut Vec<(i64, i64)>,
    ) {
        match item {
            ConstraintItem::Block(items) => {
                for it in items {
                    self.body_inside_ranges(it, arr_name, out);
                }
            }
            ConstraintItem::Soft(i) => self.body_inside_ranges(i, arr_name, out),
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                if self.eval_expr(condition).is_true() {
                    self.body_inside_ranges(then_item, arr_name, out);
                } else if let Some(ei) = else_item {
                    self.body_inside_ranges(ei, arr_name, out);
                }
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                if self.eval_expr(condition).is_true() {
                    self.body_inside_ranges(constraint, arr_name, out);
                }
            }
            ConstraintItem::Inside {
                expr,
                range,
                is_dist,
                ..
            } if !*is_dist && Self::index_chain_root_is(expr, arr_name) => {
                for r in range {
                    match r {
                        ConstraintRange::Value(e) => {
                            if let Some(v) = self.eval_expr(e).to_i64() {
                                out.push((v, v));
                            }
                        }
                        ConstraintRange::Range { lo, hi } => {
                            if let (Some(l), Some(h)) =
                                (self.eval_expr(lo).to_i64(), self.eval_expr(hi).to_i64())
                            {
                                out.push((l, h));
                            }
                        }
                    }
                }
            }
            ConstraintItem::Expr(e) => {
                if let ExprKind::Inside { expr, ranges } = &e.kind {
                    if Self::index_chain_root_is(expr, arr_name) {
                        for r in ranges.clone() {
                            match &r.kind {
                                ExprKind::Range(lo, hi) => {
                                    if let (Some(l), Some(h)) =
                                        (self.eval_expr(lo).to_i64(), self.eval_expr(hi).to_i64())
                                    {
                                        out.push((l, h));
                                    }
                                }
                                _ => {
                                    if let Some(v) = self.eval_expr(&r).to_i64() {
                                        out.push((v, v));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Strict per-element check of a `foreach` over a FIXED-shape rand array
    /// member. None = not such a foreach (or one this checker cannot judge:
    /// fewer loop vars than dimensions, or an unmodeled body) — the caller's
    /// other checkers/skips then apply unchanged. Some(ok) is authoritative.
    /// A `foreach` array that names a member of the object itself (`m`,
    /// `this.m`), not one reached through another handle.
    pub(super) fn foreach_plain_member(array: &Expression) -> bool {
        match &array.kind {
            ExprKind::Ident(h) => {
                h.root.is_none()
                    && h.path.iter().all(|s| s.selects.is_empty())
                    && (h.path.len() == 1 || (h.path.len() == 2 && h.path[0].name.name == "this"))
            }
            ExprKind::MemberAccess { expr, .. } => matches!(expr.kind, ExprKind::This),
            _ => false,
        }
    }

    /// §18.5.8.1: the index ranges of `foreach (name[i…])` over a plain
    /// member that is not an unpacked fixed array: a packed vector (`bit
    /// [3:0] it;`, one range per loop variable, leftmost dimension first,
    /// §12.7.3) or a queue / dynamic array whose current size bounds the
    /// loop. Such a loop is as much a constraint as one over a rand array,
    /// so the trial judge must check it rather than skip it. None for any
    /// other shape (a path through another object, an associative array).
    pub(in crate::compiler) fn state_foreach_dims(
        &self,
        handle: usize,
        array: &Expression,
        name: &str,
        nvars: usize,
    ) -> Option<Vec<(i64, i64)>> {
        if !Self::foreach_plain_member(array) || nvars == 0 {
            return None;
        }
        let mut cur = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone());
        while let Some(cn) = cur {
            let cd = self.module.classes.get(&cn)?;
            if cd.assoc_properties.contains_key(name)
                || cd.array_properties.contains_key(name)
                || cd.array_nd_properties.contains_key(name)
            {
                return None;
            }
            if cd.queue_properties.contains_key(name) {
                if nvars != 1 {
                    return None;
                }
                let n = self.get_queue_size(&format!("{}#{}", handle, name)) as i64;
                return Some(vec![(0, n - 1)]);
            }
            if let Some(dt) = self.class_prop_decl_type(cd, name) {
                let p = &self.module.parameters;
                let dims = match super::elaborate::packed_full_dims_chained(
                    dt,
                    p,
                    &self.module.typedef_types,
                ) {
                    Some(d) => d,
                    // A single dimension: an ordinary vector.
                    None => {
                        let dt =
                            super::elaborate::resolve_typedef_chain(dt, &self.module.typedef_types);
                        let DataType::IntegerVector { dimensions, .. } = dt else {
                            return None;
                        };
                        let [crate::ast::types::PackedDimension::Range { left, right, .. }] =
                            dimensions.as_slice()
                        else {
                            return None;
                        };
                        let l = super::elaborate::const_eval_i64_with_params(left, Some(p))?;
                        let r = super::elaborate::const_eval_i64_with_params(right, Some(p))?;
                        vec![(l, r)]
                    }
                };
                if dims.len() < nvars {
                    return None;
                }
                return Some(
                    dims[..nvars]
                        .iter()
                        .map(|&(l, r)| (l.min(r), l.max(r)))
                        .collect(),
                );
            }
            cur = cd.extends.clone();
        }
        None
    }

    pub(super) fn check_fixed_foreach_item(
        &mut self,
        handle: usize,
        item: &ConstraintItem,
    ) -> Option<bool> {
        let ConstraintItem::Foreach {
            array,
            vars,
            item: body,
            ..
        } = item
        else {
            return None;
        };
        let idx_names: Vec<String> = vars
            .iter()
            .filter_map(|v| v.as_ref().map(|id| id.name.clone()))
            .collect();
        // An array MEMBER of a struct property (`foreach (u.arr[i])`).
        if Self::foreach_member_chain(array).is_some() {
            if let Some(dims) = self.member_foreach_dims(handle, array) {
                // §12.7.3: fewer loop variables than dimensions iterate the
                // leading ones.
                if vars.iter().any(|v| v.is_none())
                    || idx_names.len() > dims.len()
                    || Self::foreach_body_unmodeled(body)
                {
                    return None;
                }
                let body = (**body).clone();
                let failed = self.for_each_index(&idx_names, &dims, |sim| {
                    (!sim.item_holds(handle, &body)).then_some(())
                });
                return Some(failed.is_none());
            }
        }
        // §18.5.9: a loop over an array member of a rand sub-object
        // (`foreach (a.w[i])`, `w` unpacked or packed): every element the
        // loop names is checked.
        if let Some((root, members)) = Self::foreach_member_chain(array)
            && let Some(dims) = self.sub_member_foreach_dims(handle, &root, &members)
        {
            if vars.iter().any(|v| v.is_none())
                || idx_names.len() > dims.len()
                || Self::foreach_body_unmodeled(body)
            {
                return None;
            }
            let body = (**body).clone();
            let failed = self.for_each_index(&idx_names, &dims, |sim| {
                (!sim.item_holds(handle, &body)).then_some(())
            });
            return Some(failed.is_none());
        }
        let arr_name = Self::foreach_base_name(array)?;
        let dims = match self.fixed_foreach_dims(handle, &arr_name) {
            Some((dims, _)) => dims,
            None => self.state_foreach_dims(handle, array, &arr_name, idx_names.len())?,
        };
        if idx_names.len() != dims.len() {
            return None;
        }
        if Self::foreach_body_unmodeled(body) {
            return None;
        }
        if dims.iter().any(|&(lo, hi)| hi < lo) {
            return Some(true);
        }
        let body = (**body).clone();
        let mut idx: Vec<i64> = dims.iter().map(|d| d.0).collect();
        loop {
            let mut frame: HashMap<String, Value> = HashMap::default();
            for (k, nm) in idx_names.iter().enumerate() {
                frame.insert(nm.clone(), Self::signed_loop_val(idx[k]));
            }
            self.push_local_frame(frame);
            let ok = self.item_holds(handle, &body);
            self.pop_local_frame();
            if !ok {
                return Some(false);
            }
            let mut k = dims.len();
            loop {
                if k == 0 {
                    return Some(true);
                }
                k -= 1;
                idx[k] += 1;
                if idx[k] <= dims[k].1 {
                    break;
                }
                idx[k] = dims[k].0;
            }
        }
    }

    /// True when a constraint item references constructs the solver/checker
    /// cannot model (dynamic-array `.size()`/`.sum()`/`.exists()` etc., foreach
    /// iteration, solve-ordering). Such items are skipped in the satisfiability
    /// check so they cannot spuriously fail randomization.
    /// Walk a list of constraint items collecting every `solve X before Y`
    /// pair (LRM §18.5.9). Pushes `(X_name, Y_name)` into `out`.
    /// Recurses into nested blocks; ignores items inside `if`/`implication`
    /// (those are runtime-conditional and would need solver-time
    /// dependency analysis to honor correctly).
    /// LRM §18.5.14: extract the bare target variable name of a
    /// constraint item, for the common single-variable shapes
    /// (`x == K`, `x < K`, `x inside {…}`, etc.). Used to decide
    /// whether a `soft` constraint is overridden by a hard
    /// constraint on the same variable. Returns None for
    /// multi-variable / structural items.
    /// Rebuild `rand_elem_ranges` for `props` against the current values.
    pub(super) fn refresh_elem_ranges(
        &mut self,
        props: &[String],
        constraints: &[ClassConstraint],
    ) {
        self.rand_elem_ranges.clear();
        for p in props {
            let mut out = Vec::new();
            for con in constraints {
                for it in &con.items {
                    self.collect_elem_domain(it, p, &mut out);
                }
            }
            if !out.is_empty() {
                self.rand_elem_ranges.insert(p.clone(), out);
            }
        }
    }

    /// The hard `inside` ranges `item` puts on the elements of `prop`
    /// (`prop[i] inside {…}`, under a `foreach` over it, a block, or the
    /// branch of a conditional that holds now). Soft items set no domain.
    pub(super) fn collect_elem_domain(
        &mut self,
        item: &ConstraintItem,
        prop: &str,
        out: &mut Vec<(i128, i128)>,
    ) {
        let on_elem = |e: &Expression| {
            matches!(&e.kind, ExprKind::Index { expr: b, .. }
                if matches!(&b.kind, ExprKind::Ident(h)
                    if h.path.len() == 1 && h.path[0].name.name == prop))
        };
        let mut ranges = |me: &mut Self, rs: &[ConstraintRange]| {
            for r in rs {
                let (lo, hi) = match r {
                    ConstraintRange::Value(e) => (e, e),
                    ConstraintRange::Range { lo, hi } => (lo, hi),
                };
                let to_i = |v: Value| {
                    if v.is_signed {
                        v.to_i64().map(i128::from)
                    } else {
                        v.to_u64().map(i128::from)
                    }
                };
                if let (Some(l), Some(h)) = (to_i(me.eval_expr(lo)), to_i(me.eval_expr(hi))) {
                    out.push((l.min(h), l.max(h)));
                }
            }
        };
        match item {
            ConstraintItem::Expr(e) => {
                if let ExprKind::Inside {
                    expr: inner,
                    ranges: rs,
                } = &e.kind
                    && on_elem(inner)
                {
                    let cr: Vec<ConstraintRange> = rs
                        .iter()
                        .map(|r| match &r.kind {
                            ExprKind::Range(lo, hi) => ConstraintRange::Range {
                                lo: (**lo).clone(),
                                hi: (**hi).clone(),
                            },
                            _ => ConstraintRange::Value(r.clone()),
                        })
                        .collect();
                    ranges(self, &cr);
                }
            }
            ConstraintItem::Inside {
                expr,
                range,
                is_dist,
                ..
            } if !*is_dist && on_elem(expr) => ranges(self, range),
            ConstraintItem::Block(items) => {
                for i in items {
                    self.collect_elem_domain(i, prop, out);
                }
            }
            ConstraintItem::Foreach {
                array, item: body, ..
            } if Self::foreach_base_name(array).as_deref() == Some(prop) => {
                self.collect_elem_domain(body, prop, out);
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                if self.eval_expr(condition).is_true() {
                    self.collect_elem_domain(then_item, prop, out);
                } else if let Some(e) = else_item {
                    self.collect_elem_domain(e, prop, out);
                }
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } if self.eval_expr(condition).is_true() => {
                self.collect_elem_domain(constraint, prop, out);
            }
            _ => {}
        }
    }

    /// A constraint item made only of soft constraints: `soft …`, or a
    /// `foreach` / block whose every item is one (`foreach (q[i]) soft q[i]
    /// == -1;`).
    pub(super) fn soft_only(item: &ConstraintItem) -> bool {
        match item {
            ConstraintItem::Soft(_) => true,
            ConstraintItem::Foreach { item, .. } => Self::soft_only(item),
            ConstraintItem::Block(items) => !items.is_empty() && items.iter().all(Self::soft_only),
            _ => false,
        }
    }

    pub(super) fn constraint_item_target(item: &ConstraintItem) -> Option<String> {
        fn ident_of(e: &Expression) -> Option<String> {
            match &e.kind {
                ExprKind::Ident(h) if h.path.len() == 1 => Some(h.path[0].name.name.clone()),
                ExprKind::Paren(inner) => ident_of(inner),
                _ => None,
            }
        }
        match item {
            ConstraintItem::Expr(e) => match &e.kind {
                ExprKind::Binary { left, .. } => ident_of(left),
                ExprKind::Paren(inner) => {
                    Self::constraint_item_target(&ConstraintItem::Expr((**inner).clone()))
                }
                _ => None,
            },
            ConstraintItem::Inside { expr, .. } => ident_of(expr),
            ConstraintItem::Soft(inner) => Self::constraint_item_target(inner),
            _ => None,
        }
    }

    /// IEEE 1800-2017 §18.5.5 — `unique {a, b, c, arr[0:2]}`.
    ///
    /// The parser desugars a multi-expression `unique` list into the pairwise
    /// inequalities `e[i] != e[j]`, but an element of that list may be an array
    /// SLICE, and `a != arr[0:2]` is not a constraint on anything: the slice
    /// stands for its three ELEMENTS. The uniqueness of `arr[0]` against `a`,
    /// and of the slice's elements against each other, was therefore never
    /// enforced — the test passed only when the random draw happened to be
    /// collision-free.
    ///
    /// Rewrite each such inequality into
    ///   `other != arr[i]` for every i in the slice, plus
    ///   `arr[i] != arr[j]` for every pair inside the slice (emitted once).
    /// §18.3 — see the call site in `exec_randomize_inner`: rewrite a bare
    /// ident in a constraint to `<scope>.<ident>` when the bare spelling names
    /// no signal but the scoped one does. `skip` holds the class hierarchy's
    /// property names, which always resolve through the object instead.
    pub(super) fn scope_qualify_constraint_item(
        &self,
        item: &mut ConstraintItem,
        scope: &str,
        skip: &HashSet<String>,
    ) {
        match item {
            ConstraintItem::Expr(e) => self.scope_qualify_expr(e, scope, skip),
            ConstraintItem::Inside { expr, range, .. } => {
                self.scope_qualify_expr(expr, scope, skip);
                for r in range {
                    match r {
                        ConstraintRange::Value(e) => self.scope_qualify_expr(e, scope, skip),
                        ConstraintRange::Range { lo, hi } => {
                            self.scope_qualify_expr(lo, scope, skip);
                            self.scope_qualify_expr(hi, scope, skip);
                        }
                    }
                }
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                self.scope_qualify_expr(condition, scope, skip);
                self.scope_qualify_constraint_item(constraint, scope, skip);
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                self.scope_qualify_expr(condition, scope, skip);
                self.scope_qualify_constraint_item(then_item, scope, skip);
                if let Some(ei) = else_item {
                    self.scope_qualify_constraint_item(ei, scope, skip);
                }
            }
            ConstraintItem::Foreach {
                array, vars, item, ..
            } => {
                self.scope_qualify_expr(array, scope, skip);
                // Iterator variables shadow everything inside the body.
                let mut inner = skip.clone();
                inner.extend(vars.iter().flatten().map(|v| v.name.clone()));
                self.scope_qualify_constraint_item(item, scope, &inner);
            }
            ConstraintItem::Soft(inner) => self.scope_qualify_constraint_item(inner, scope, skip),
            ConstraintItem::Block(items) => {
                for it in items {
                    self.scope_qualify_constraint_item(it, scope, skip);
                }
            }
            ConstraintItem::Unique { exprs, .. } => {
                for e in exprs {
                    self.scope_qualify_expr(e, scope, skip);
                }
            }
            ConstraintItem::Solve { .. } => {}
        }
    }

    pub(super) fn scope_qualify_expr(
        &self,
        e: &mut Expression,
        scope: &str,
        skip: &HashSet<String>,
    ) {
        match &mut e.kind {
            ExprKind::Ident(hier) => {
                if hier.root.is_none() && hier.path.len() == 1 {
                    let n = hier.path[0].name.name.clone();
                    if !n.starts_with('$')
                        && !skip.contains(&n)
                        && !self.signal_name_to_id.contains_key(n.as_str())
                        && !self.module.parameters.contains_key(&n)
                    {
                        let scoped = format!("{}.{}", scope, n);
                        if self.signal_name_to_id.contains_key(scoped.as_str()) {
                            hier.path[0].name.name = scoped;
                            hier.cached_signal_id.set(None);
                            hier.cached_resolved_name = std::cell::OnceCell::new();
                        }
                    }
                }
                for seg in &mut hier.path {
                    for sel in &mut seg.selects {
                        self.scope_qualify_expr(sel, scope, skip);
                    }
                }
            }
            ExprKind::Unary { operand, .. } => self.scope_qualify_expr(operand, scope, skip),
            ExprKind::Binary { left, right, .. } => {
                self.scope_qualify_expr(left, scope, skip);
                self.scope_qualify_expr(right, scope, skip);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.scope_qualify_expr(condition, scope, skip);
                self.scope_qualify_expr(then_expr, scope, skip);
                self.scope_qualify_expr(else_expr, scope, skip);
            }
            ExprKind::Concatenation(parts) => {
                for p in parts {
                    self.scope_qualify_expr(p, scope, skip);
                }
            }
            ExprKind::Replication { count, exprs } => {
                self.scope_qualify_expr(count, scope, skip);
                for x in exprs {
                    self.scope_qualify_expr(x, scope, skip);
                }
            }
            ExprKind::Call { args, .. } | ExprKind::SystemCall { args, .. } => {
                // The callee name is left alone — only its arguments can name
                // state variables.
                for a in args {
                    self.scope_qualify_expr(a, scope, skip);
                }
            }
            ExprKind::Inside { expr, ranges } => {
                self.scope_qualify_expr(expr, scope, skip);
                for r in ranges {
                    self.scope_qualify_expr(r, scope, skip);
                }
            }
            ExprKind::MemberAccess { expr, .. } => self.scope_qualify_expr(expr, scope, skip),
            ExprKind::Index { expr, index } => {
                self.scope_qualify_expr(expr, scope, skip);
                self.scope_qualify_expr(index, scope, skip);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                self.scope_qualify_expr(expr, scope, skip);
                self.scope_qualify_expr(left, scope, skip);
                self.scope_qualify_expr(right, scope, skip);
            }
            ExprKind::Range(a, b) => {
                self.scope_qualify_expr(a, scope, skip);
                self.scope_qualify_expr(b, scope, skip);
            }
            ExprKind::Paren(inner) => self.scope_qualify_expr(inner, scope, skip),
            _ => {}
        }
    }

    pub(super) fn expand_unique_slices(
        constraints: &mut [ClassConstraint],
        params: &HashMap<String, Value>,
    ) {
        fn idx_expr(i: i64, span: crate::ast::Span) -> Expression {
            Expression::new(
                ExprKind::Number(NumberLiteral::Integer {
                    size: Some(32),
                    signed: true,
                    base: NumberBase::Decimal,
                    value: i.to_string(),
                    cached_val: Cell::new(None),
                }),
                span,
            )
        }
        fn elem(arr: &Expression, i: i64, span: crate::ast::Span) -> Expression {
            Expression::new(
                ExprKind::Index {
                    expr: Box::new(arr.clone()),
                    index: Box::new(idx_expr(i, span)),
                },
                span,
            )
        }
        fn neq(l: Expression, r: Expression, span: crate::ast::Span) -> ConstraintItem {
            ConstraintItem::Expr(Expression::new(
                ExprKind::Binary {
                    op: BinaryOp::Neq,
                    left: Box::new(l),
                    right: Box::new(r),
                },
                span,
            ))
        }
        // (array name, lo, hi) whose intra-slice pairs have already been emitted.
        fn walk(
            item: &mut ConstraintItem,
            params: &HashMap<String, Value>,
            seen: &mut HashSet<(String, i64, i64)>,
        ) {
            match item {
                ConstraintItem::Block(items) => {
                    for it in items.iter_mut() {
                        walk(it, params, seen);
                    }
                }
                ConstraintItem::Soft(inner) => walk(inner, params, seen),
                ConstraintItem::Expr(e) => {
                    let ExprKind::Binary {
                        op: BinaryOp::Neq,
                        left,
                        right,
                    } = &e.kind
                    else {
                        return;
                    };
                    let span = e.span;
                    let slice_of = |x: &Expression| -> Option<(Expression, String, i64, i64)> {
                        let ExprKind::RangeSelect {
                            expr, left, right, ..
                        } = &x.kind
                        else {
                            return None;
                        };
                        let ExprKind::Ident(h) = &expr.kind else {
                            return None;
                        };
                        if h.path.len() != 1 {
                            return None;
                        }
                        let lo = Simulator::const_expr_u64(left, params)? as i64;
                        let hi = Simulator::const_expr_u64(right, params)? as i64;
                        Some((
                            (**expr).clone(),
                            h.path[0].name.name.clone(),
                            lo.min(hi),
                            lo.max(hi),
                        ))
                    };
                    let (arr, name, lo, hi, other) = match (slice_of(left), slice_of(right)) {
                        (Some((a, n, lo, hi)), None) => (a, n, lo, hi, (**right).clone()),
                        (None, Some((a, n, lo, hi))) => (a, n, lo, hi, (**left).clone()),
                        // Two slices, or none — nothing to expand here.
                        _ => return,
                    };
                    if hi < lo || hi - lo > 1024 {
                        return;
                    }
                    let mut out: Vec<ConstraintItem> = Vec::new();
                    for i in lo..=hi {
                        out.push(neq(other.clone(), elem(&arr, i, span), span));
                    }
                    if seen.insert((name, lo, hi)) {
                        for i in lo..=hi {
                            for j in (i + 1)..=hi {
                                out.push(neq(elem(&arr, i, span), elem(&arr, j, span), span));
                            }
                        }
                    }
                    *item = ConstraintItem::Block(out);
                }
                _ => {}
            }
        }
        let mut seen: HashSet<(String, i64, i64)> = HashSet::default();
        for con in constraints.iter_mut() {
            for it in con.items.iter_mut() {
                walk(it, params, &mut seen);
            }
        }
    }

    pub(super) fn collect_solve_pairs(items: &[ConstraintItem], out: &mut Vec<(String, String)>) {
        for it in items {
            match it {
                ConstraintItem::Solve { before, after, .. } => {
                    for b in before {
                        for a in after {
                            out.push((b.name.clone(), a.name.clone()));
                        }
                    }
                }
                ConstraintItem::Block(inner) => Self::collect_solve_pairs(inner, out),
                ConstraintItem::Soft(inner) => Self::collect_solve_pairs(&[(**inner).clone()], out),
                _ => {}
            }
        }
    }

    /// `constraint_unmodeled` for the body of a `foreach` the trial judge
    /// checks element by element: a nested `foreach` is judged too (see the
    /// `Foreach` arm of `check_constraint_item_impl`), so it is unmodeled
    /// only when its own body is.
    pub(super) fn foreach_body_unmodeled(item: &ConstraintItem) -> bool {
        match item {
            ConstraintItem::Foreach { item, .. } => Self::foreach_body_unmodeled(item),
            ConstraintItem::Block(items) => items.iter().any(Self::foreach_body_unmodeled),
            ConstraintItem::Soft(inner) => Self::foreach_body_unmodeled(inner),
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => Self::expr_unmodeled(condition) || Self::foreach_body_unmodeled(constraint),
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                Self::expr_unmodeled(condition)
                    || Self::foreach_body_unmodeled(then_item)
                    || else_item
                        .as_ref()
                        .is_some_and(|e| Self::foreach_body_unmodeled(e))
            }
            other => Self::constraint_unmodeled(other),
        }
    }

    pub(super) fn constraint_unmodeled(item: &ConstraintItem) -> bool {
        match item {
            ConstraintItem::Foreach { .. }
            | ConstraintItem::Solve { .. }
            // `unique {arr}` is enforced by its solve_forced arm; the
            // post-hoc checker has no element-set view, so skip it there.
            | ConstraintItem::Unique { .. } => true,
            ConstraintItem::Expr(e) => Self::expr_unmodeled(e),
            ConstraintItem::Inside { expr, .. } => Self::expr_unmodeled(expr),
            ConstraintItem::Implication { condition, constraint, .. } => {
                Self::expr_unmodeled(condition) || Self::constraint_unmodeled(constraint)
            }
            ConstraintItem::IfElse { condition, then_item, else_item, .. } => {
                Self::expr_unmodeled(condition)
                    || Self::constraint_unmodeled(then_item)
                    || else_item.as_ref().is_some_and(|e| Self::constraint_unmodeled(e))
            }
            ConstraintItem::Block(items) => items.iter().any(Self::constraint_unmodeled),
            ConstraintItem::Soft(inner) => Self::constraint_unmodeled(inner),
        }
    }

    /// Recursively detect aggregate/array method calls (`.size`, `.sum`, …) that
    /// the constraint engine does not model.
    pub(super) fn expr_unmodeled(expr: &Expression) -> bool {
        match &expr.kind {
            ExprKind::Call { func, args } => {
                if let ExprKind::MemberAccess { member, .. } = &func.kind {
                    // NOTE: `sum` is deliberately absent — a `.sum() == K`
                    // equality is now solved by `distribute_array_sum` and must
                    // be CHECKED (so an infeasible target retries/fails instead
                    // of silently passing). The rest stay unmodeled.
                    if matches!(
                        member.name.as_str(),
                        "size"
                            | "exists"
                            | "len"
                            | "product"
                            | "min"
                            | "max"
                            | "num"
                            | "and"
                            | "or"
                            | "xor"
                    ) {
                        return true;
                    }
                }
                Self::expr_unmodeled(func) || args.iter().any(Self::expr_unmodeled)
            }
            ExprKind::Binary { left, right, .. } => {
                Self::expr_unmodeled(left) || Self::expr_unmodeled(right)
            }
            ExprKind::Unary { operand, .. } => Self::expr_unmodeled(operand),
            ExprKind::Paren(inner) => Self::expr_unmodeled(inner),
            ExprKind::Inside { expr, ranges } => {
                Self::expr_unmodeled(expr) || ranges.iter().any(Self::expr_unmodeled)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::expr_unmodeled(condition)
                    || Self::expr_unmodeled(then_expr)
                    || Self::expr_unmodeled(else_expr)
            }
            _ => false,
        }
    }

    /// The `constraint_operand_name` of every member-select operand in
    /// `expr` (`collect_expr_idents` names only the base of `obj.m`).
    pub(super) fn collect_member_operand_names(
        &self,
        expr: &Expression,
        out: &mut HashSet<String>,
    ) {
        match &expr.kind {
            ExprKind::MemberAccess { .. } => {
                if let Some(n) = self.constraint_operand_name(expr) {
                    out.insert(n);
                }
            }
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                self.collect_member_operand_names(operand, out)
            }
            ExprKind::Binary { left, right, .. } => {
                self.collect_member_operand_names(left, out);
                self.collect_member_operand_names(right, out);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.collect_member_operand_names(condition, out);
                self.collect_member_operand_names(then_expr, out);
                self.collect_member_operand_names(else_expr, out);
            }
            ExprKind::Concatenation(exprs) => {
                for e in exprs {
                    self.collect_member_operand_names(e, out);
                }
            }
            ExprKind::Call { args, .. } | ExprKind::SystemCall { args, .. } => {
                for a in args {
                    self.collect_member_operand_names(a, out);
                }
            }
            ExprKind::Index { expr, index } => {
                self.collect_member_operand_names(expr, out);
                self.collect_member_operand_names(index, out);
            }
            _ => {}
        }
    }

    pub(super) fn collect_expr_idents(&self, expr: &Expression, idents: &mut HashSet<String>) {
        match &expr.kind {
            ExprKind::Ident(hier) => {
                if let Some(seg) = hier.path.last() {
                    idents.insert(seg.name.name.clone());
                }
            }
            ExprKind::Unary { operand, .. } => self.collect_expr_idents(operand, idents),
            ExprKind::Binary { left, right, .. } => {
                self.collect_expr_idents(left, idents);
                self.collect_expr_idents(right, idents);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.collect_expr_idents(condition, idents);
                self.collect_expr_idents(then_expr, idents);
                self.collect_expr_idents(else_expr, idents);
            }
            ExprKind::Paren(inner) => self.collect_expr_idents(inner, idents),
            ExprKind::Concatenation(exprs) => {
                for e in exprs {
                    self.collect_expr_idents(e, idents);
                }
            }
            ExprKind::Call { args, .. } => {
                for a in args {
                    self.collect_expr_idents(a, idents);
                }
            }
            // §6.24.1: size casts are lowered to `$__xz_size_cast(N, inner)`
            // SystemCalls — without this arm a cast-wrapped operand looked
            // identifier-free to the constraint solver (issue #29).
            ExprKind::SystemCall { args, .. } => {
                for a in args {
                    self.collect_expr_idents(a, idents);
                }
            }
            ExprKind::Index { expr, index } => {
                self.collect_expr_idents(expr, idents);
                self.collect_expr_idents(index, idents);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                self.collect_expr_idents(expr, idents);
                self.collect_expr_idents(left, idents);
                self.collect_expr_idents(right, idents);
            }
            ExprKind::MemberAccess { expr, .. } => {
                self.collect_expr_idents(expr, idents);
            }
            _ => {}
        }
    }

    /// IEEE 1800-2017 §11.6.1/§18.5.12 — is a constraint EXPRESSION satisfied?
    ///
    /// Judged with the same exact, context-width arithmetic the solver uses
    /// (`exact_int`). The procedural evaluator computes each operand at its
    /// SELF-determined width, so `(r_val - 8'd10) * (l_val + 8'd5) == 16'd0`
    /// wrapped at 8 bits and reported "satisfied" for r_val = 12, l_val = 123
    /// (product 256 ≡ 0 mod 256) — a wrong answer sailing straight through the
    /// generate-and-test backstop. The LRM extends both operands of the `==` to
    /// max(8, 16) = 16 bits FIRST, where 256 ≠ 0.
    ///
    /// Anything the exact model cannot represent (X/Z, real, > 64 bits,
    /// non-arithmetic operators) falls back to the ordinary evaluator.
    pub(super) fn cons_expr_true(&mut self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Paren(inner) => self.cons_expr_true(inner),
            ExprKind::Unary {
                op: UnaryOp::LogNot,
                operand,
            } => !self.cons_expr_true(operand),
            ExprKind::Binary {
                op: BinaryOp::LogAnd,
                left,
                right,
            } => self.cons_expr_true(left) && self.cons_expr_true(right),
            ExprKind::Binary {
                op: BinaryOp::LogOr,
                left,
                right,
            } => self.cons_expr_true(left) || self.cons_expr_true(right),
            ExprKind::Binary {
                op: BinaryOp::LogImplies,
                left,
                right,
            } => !self.cons_expr_true(left) || self.cons_expr_true(right),
            ExprKind::Binary {
                op:
                    op @ (BinaryOp::Eq
                    | BinaryOp::Neq
                    | BinaryOp::Lt
                    | BinaryOp::Leq
                    | BinaryOp::Gt
                    | BinaryOp::Geq),
                left,
                right,
            } => match self.exact_cmp(op, left, right) {
                Some(b) => b,
                None => self.eval_expr(e).is_true(),
            },
            ExprKind::Inside { expr, ranges } => {
                let cr: Vec<ConstraintRange> = ranges
                    .iter()
                    .map(|r| match &r.kind {
                        ExprKind::Range(lo, hi) => ConstraintRange::Range {
                            lo: (**lo).clone(),
                            hi: (**hi).clone(),
                        },
                        _ => ConstraintRange::Value(r.clone()),
                    })
                    .collect();
                // §11.4.13: at the width of all operands (see
                // `inside_joint_width`).
                let val = match self.inside_joint_width_cr(expr, &cr) {
                    Some(w) => self.eval_expr_ctx(expr, w),
                    None => self.eval_expr(expr),
                };
                if val.has_xz() || val.width > 64 || val.is_real {
                    return self.eval_expr(e).is_true();
                }
                self.value_in_ranges(&val, &cr)
            }
            _ => self.eval_expr(e).is_true(),
        }
    }

    pub(super) fn check_constraint_item(&mut self, handle: usize, item: &ConstraintItem) -> bool {
        self.this_stack.push(Some(handle));
        let ok = self.check_constraint_item_impl(item);
        self.this_stack.pop();
        ok
    }

    pub(super) fn check_constraint_item_impl(&mut self, item: &ConstraintItem) -> bool {
        match item {
            ConstraintItem::Expr(expr) => self.cons_expr_true(expr),
            ConstraintItem::Inside { expr, range, .. } => {
                // §18.3: exact signed membership — see `value_in_ranges`,
                // at the width of all operands (§11.4.13).
                let val = match self.inside_joint_width_cr(expr, range) {
                    Some(w) => self.eval_expr_ctx(expr, w),
                    None => self.eval_expr(expr),
                };
                if !val.has_xz() && val.width <= 64 && !val.is_real {
                    return self.value_in_ranges(&val, range);
                }
                let val_u = val.to_u64();
                for r in range {
                    match r {
                        ConstraintRange::Value(e) => {
                            // Compare numerically rather than by raw Value
                            // equality (which is width-sensitive — a 4-bit `1`
                            // is unequal to a 32-bit literal `1`).
                            let v_u = self.eval_expr(e).to_u64();
                            if val_u.is_some() && val_u == v_u {
                                return true;
                            }
                        }
                        ConstraintRange::Range { lo, hi } => {
                            let l = self.eval_expr(lo);
                            let h = self.eval_expr(hi);
                            if val.greater_equal(&l).is_true() && val.less_equal(&h).is_true() {
                                return true;
                            }
                        }
                    }
                }
                false
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } if self.eval_expr(condition).is_true() => self.check_constraint_item_impl(constraint),
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                if self.eval_expr(condition).is_true() {
                    self.check_constraint_item_impl(then_item)
                } else if let Some(ei) = else_item {
                    self.check_constraint_item_impl(ei)
                } else {
                    true
                }
            }
            ConstraintItem::Block(items) => {
                for it in items {
                    if !self.check_constraint_item_impl(it) {
                        return false;
                    }
                }
                true
            }
            ConstraintItem::Foreach {
                array,
                vars,
                item: body,
                ..
            } => self.check_member_foreach(array, vars, body),
            _ => true,
        }
    }

    /// §18.5.8.1: `foreach (m[i…]) body` over a plain member of the object
    /// being checked (`this_stack`), judged element by element: an unpacked
    /// fixed array, a packed vector, a queue or a dynamic array. A loop
    /// nested in another loop's body (`foreach (a[i]) foreach (b[j]) …`)
    /// reaches the checker only here, and used to pass unseen. A shape the
    /// checker cannot enumerate still passes, as before.
    pub(super) fn check_member_foreach(
        &mut self,
        array: &Expression,
        vars: &[Option<crate::ast::Identifier>],
        body: &ConstraintItem,
    ) -> bool {
        let Some(Some(handle)) = self.this_stack.last().copied() else {
            return true;
        };
        let Some(name) = Self::foreach_base_name(array) else {
            return true;
        };
        // Only a plain member (`m`, `this.m`): `state_foreach_dims` checks
        // that, and a fixed array must pass the same test.
        let Some(dims) = (match self.state_foreach_dims(handle, array, &name, vars.len()) {
            Some(d) => Some(d),
            None if Self::foreach_plain_member(array) => {
                self.fixed_foreach_dims(handle, &name).map(|d| d.0)
            }
            None => None,
        }) else {
            return true;
        };
        if dims.len() != vars.len() || Self::foreach_body_unmodeled(body) {
            return true;
        }
        if dims.iter().any(|&(lo, hi)| hi < lo) {
            return true;
        }
        let mut idx: Vec<i64> = dims.iter().map(|d| d.0).collect();
        // The enclosing loop's indices stay visible (`a[i][j*2+1]` in a loop
        // nested in `foreach (a[i])`): the frame extends the current one.
        let outer = self.local_stack.last().cloned().unwrap_or_default();
        loop {
            let mut frame = outer.clone();
            for (k, v) in vars.iter().enumerate() {
                if let Some(v) = v {
                    frame.insert(v.name.clone(), Self::signed_loop_val(idx[k]));
                }
            }
            self.push_local_frame(frame);
            let ok = self.check_constraint_item_impl(body);
            self.pop_local_frame();
            if !ok {
                return false;
            }
            let mut k = dims.len();
            loop {
                if k == 0 {
                    return true;
                }
                k -= 1;
                idx[k] += 1;
                if idx[k] <= dims[k].1 {
                    break;
                }
                idx[k] = dims[k].0;
            }
        }
    }

    /// IEEE 1800-2017 §8.15: the class a `super.m(...)` call resolves
    /// against — the PARENT of the class LEXICALLY containing the call, not
    /// the parent of the object's dynamic type. During method execution
    /// `class_context_stack` holds the DEFINING class of the running method
    /// (`exec_method_in_class_hierarchy` pushes the class where the body was
    /// found while walking the extends chain, not the receiver's class), so
    /// an inherited method's `super` chain correctly starts from its own
    /// defining class's parent (e.g. Grand::sup2's `super.get()` binds to
    /// Derived::get even on a Grand object).
    /// Does class `class_name`'s own `new` body call `super.new(...)`
    /// explicitly? (§8.7 — the first statement, but scan a few nesting levels
    /// to be safe.) Used to decide whether an IMPLICIT super.new must be
    /// injected; explicit chains are left untouched.
    pub(super) fn constructor_calls_super_new(&self, class_name: &str) -> bool {
        use crate::ast::decl::ClassMethodKind;
        let Some(cd) = self.module.classes.get(class_name) else {
            return false;
        };
        let Some(m) = cd.methods.get("new") else {
            return false;
        };
        let items = match &m.kind {
            ClassMethodKind::Function(f) => &f.items,
            ClassMethodKind::Task(t) => &t.items,
            _ => return false,
        };
        Self::stmts_call_super_new(items)
    }
}
