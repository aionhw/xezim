//! Simulator expression types.

use super::*;

impl Simulator {
    /// `get_expr_type_name(expr) == Some("string")` for the identifier
    /// `expr`. A bare name stops before the class-property fallback, which
    /// only ever yields a class, enum, covergroup or type-parameter name;
    /// every other shape asks `get_expr_type_name`.
    pub(super) fn ident_type_is_string(
        &self,
        hier: &HierarchicalIdentifier,
        expr: &Expression,
    ) -> bool {
        if hier.path.len() != 1 {
            return self.get_expr_type_name(expr).is_some_and(|t| t == "string");
        }
        let name = self.resolve_hier_name(hier);
        let is_string = |t: Option<&String>| t.is_some_and(|t| t == "string");
        if self.fe_trusted_types.contains(&*name) {
            if let Some(t) = self.var_typedef_types.get(&*name) {
                return t == "string";
            }
        }
        // The first source that answers decides, in `get_expr_type_name`'s
        // order.
        let in_any_frame = self
            .method_local_base
            .last()
            .map(|&base| {
                self.local_stack
                    .get(base..)
                    .unwrap_or(&[])
                    .iter()
                    .rev()
                    .any(|m| m.contains_key(&*name))
            })
            .unwrap_or_else(|| {
                self.local_stack
                    .iter()
                    .rev()
                    .any(|m| m.contains_key(&*name))
            });
        if in_any_frame {
            if let Some(t) = self.var_class_types.get(&*name) {
                return t == "string";
            }
            if let Some(t) = self.var_typedef_types.get(&*name) {
                return t == "string";
            }
        }
        if let Some(t) = self
            .signal_name_to_id
            .get(name.as_ref())
            .and_then(|id| self.signal_type_names.get(id))
        {
            return t == "string";
        }
        if self.class_context_stack.last().is_none() {
            return is_string(self.var_class_types.get(&*name))
                || (!self.var_class_types.contains_key(&*name)
                    && is_string(self.var_typedef_types.get(&*name)));
        }
        false
    }

    pub(super) fn get_expr_type_name(&self, expr: &Expression) -> Option<String> {
        match &expr.kind {
            ExprKind::Ident(hier) => {
                let name = self.resolve_hier_name(hier);
                // §12.7.3: an ACTIVE foreach loop variable is a fresh
                // declaration whose (assoc key) type shadows everything —
                // including a same-named class property, which otherwise won
                // below and gave UVM's severity summary the type of an
                // unrelated ancestor property.
                if hier.path.len() == 1 && self.fe_trusted_types.contains(&*name) {
                    if let Some(t) = self.var_typedef_types.get(&*name) {
                        return Some(t.clone());
                    }
                }
                // §4c: `h.prop` where `h` is a class-handle VARIABLE (module
                // scope included). No composed signal carries the type; the
                // property's declared type comes from the head's runtime
                // class. Without this, `h.prop = new` could not resolve the
                // class to construct and silently stored garbage.
                // §8.15: the same for a longer chain (`n.next.next = new(3)`):
                // walk the live handles to the last owner. Without it the
                // chain had no type, `new(3)` was not a construction and the
                // write was lost.
                if hier.path.len() >= 3
                    && hier.path.iter().all(|s| s.selects.is_empty())
                    && !self.signal_name_to_id.contains_key(name.as_ref())
                    && self.bare_name_is_class_channel(hier.path[0].name.name.as_str())
                {
                    let n = hier.path.len();
                    let mut h = self.eval_ident_handle(hier.path[0].name.name.as_str());
                    for seg in &hier.path[1..n - 1] {
                        h = h
                            .filter(|&x| x != 0)
                            .and_then(|x| self.member_handle(x, &seg.name.name));
                    }
                    if let Some(cn) = h
                        .filter(|&x| x != 0)
                        .and_then(|x| self.heap.get(x))
                        .and_then(|o| o.as_ref())
                        .map(|i| i.class_name.clone())
                    {
                        if let Some(t) =
                            self.class_prop_type_named(&cn, &hier.path[n - 1].name.name)
                        {
                            return Some(t);
                        }
                    }
                }
                if hier.path.len() == 2
                    && hier.path.iter().all(|s| s.selects.is_empty())
                    && self
                        .signal_name_to_id
                        .get(
                            format!("{}.{}", hier.path[0].name.name, hier.path[1].name.name)
                                .as_str(),
                        )
                        .is_none()
                {
                    let head = hier.path[0].name.name.as_str();
                    let field = hier.path[1].name.name.as_str();
                    if let Some(h) = self.eval_ident_handle(head) {
                        if h != 0 {
                            if let Some(cn) = self
                                .heap
                                .get(h)
                                .and_then(|o| o.as_ref())
                                .map(|i| i.class_name.clone())
                            {
                                if let Some(t) = self.class_prop_type_named(&cn, field) {
                                    return Some(t);
                                }
                            }
                        }
                    }
                }
                // A genuine local of the CURRENT scope must resolve before the
                // module-signal table: `signal_name_to_id` holds module-scope
                // declarations, and a method-local shadows a same-named module
                // signal (§23.7 name resolution). Without this, a module-scope
                // `C obj;` shadowed `T obj;` inside a parameterized class
                // method, so `obj = new(...)` constructed `C` instead of `T`.
                let in_any_frame = hier.path.len() == 1
                    && self
                        .method_local_base
                        .last()
                        .map(|&base| {
                            self.local_stack
                                .get(base..)
                                .unwrap_or(&[])
                                .iter()
                                .rev()
                                .any(|m| m.contains_key(&*name))
                        })
                        // Outside any class method (initial/always blocks):
                        // locals live as signals/flat-map entries with no
                        // method base recorded, so any frame counts (there
                        // is no class property they could shadow).
                        .unwrap_or_else(|| {
                            self.local_stack
                                .iter()
                                .rev()
                                .any(|m| m.contains_key(&*name))
                        });
                if in_any_frame {
                    // The frame overlay first: `var_class_types` is one flat
                    // map, so while another process is suspended in a
                    // subroutine with a same-named formal (UVM's
                    // `finish_item(uvm_sequence_item item)`), it holds THAT
                    // formal's class, not this frame's.
                    if let Some(t) = self.local_class_type_of(&name) {
                        return Some(t);
                    }
                    // A class-typed procedural local declared in this scope.
                    if let Some(t) = self.var_class_types.get(&*name) {
                        return Some(t.clone());
                    }
                    // LRM §6.19.6: typedef-typed local — used by
                    // `local.next/.first/.last/.num/.prev` resolution.
                    if let Some(t) = self.var_typedef_types.get(&*name) {
                        return Some(t.clone());
                    }
                }
                // §8.1: inside a class method a bare name its class declares
                // is that PROPERTY, even when a module signal shares the name.
                // The signal won here, so a constructor's `b = new(7)` built
                // the module variable's class instead — whose constructor
                // built this one again, without end.
                if hier.path.len() == 1 && !in_any_frame {
                    if let Some(Some(ctx)) = self.class_context_stack.last() {
                        if let Some(t) = self.class_prop_type_named(ctx, &name) {
                            return Some(t);
                        }
                    }
                }
                if let Some(t) = self
                    .signal_name_to_id
                    .get(name.as_ref())
                    .and_then(|id| self.signal_type_names.get(id).cloned())
                {
                    return Some(t);
                }
                // §26.3: a PACKAGE-scoped reference (`P::e`) stores its signal
                // under the bare leaf — the dotted spelling resolves nothing,
                // which left `P::e.next(1)` with no type and the call fell to
                // the generic machinery (returning 0).
                if hier.path.len() >= 2 && self.module.packages.contains(&hier.path[0].name.name) {
                    let leaf = &hier.path.last().unwrap().name.name;
                    if let Some(t) = self
                        .signal_name_to_id
                        .get(leaf.as_str())
                        .and_then(|id| self.signal_type_names.get(id).cloned())
                    {
                        return Some(t);
                    }
                }
                // `var_class_types` / `var_typedef_types` are FLAT
                // accumulators that are NOT cleared on scope/method exit,
                // so a same-named local from an EARLIER method leaks in.
                // E.g. UVM `process_all_report_catchers` declares a local
                // `uvm_report_catcher catcher;`; that entry then shadows a
                // user class PROPERTY `seqprtzmb_catcher catcher;` when the
                // property's `catcher = new()` resolves its type, so the
                // base class is constructed and the derived `catch()`
                // override never runs (phasing cleanup).
                //
                // Only trust these maps when `name` is genuinely a local of
                // the current scope. Two cases are genuine:
                //  (a) `name` is in a `local_stack` frame (a method's
                //      params/locals — class methods store locals there), or
                //  (b) we are NOT inside a class method (initial/always
                //      blocks): their locals live as signals, not frames, but
                //      there is no class property they could shadow, so a
                //      flat-map entry is always a real local.
                // Inside a class method, a bare name that is NOT a frame
                // local is a class property and must resolve through
                // `class_prop_type_named`, ignoring any stale entry.
                // (`in_any_frame` was already computed above, before the
                // signal-table lookups, so a current-scope local wins over a
                // same-named module signal.)
                let in_class_method = self.class_context_stack.last().is_some();
                let trust_flat_maps = in_any_frame || !in_class_method;
                if trust_flat_maps {
                    // A class-typed procedural local declared in this scope.
                    if let Some(t) = self.var_class_types.get(&*name) {
                        return Some(t.clone());
                    }
                    // LRM §6.19.6: typedef-typed local — used by
                    // `local.next/.first/.last/.num/.prev` resolution.
                    if let Some(t) = self.var_typedef_types.get(&*name) {
                        return Some(t.clone());
                    }
                }
                // A class property referenced bare inside a method —
                // resolve its type through the current class context.
                if hier.path.len() == 1 && !in_any_frame {
                    if let Some(Some(ctx)) = self.class_context_stack.last() {
                        if let Some(t) = self.class_prop_type_named(ctx, &name) {
                            return Some(t);
                        }
                    }
                }
                // Multi-segment path resolution (e.g. `stor.handle`, `ClassName::static_prop`, `obj.field.subfield`)
                if hier.path.len() >= 2 && hier.path.iter().all(|s| s.selects.is_empty()) {
                    let root = &hier.path[0].name.name;
                    let curr_type = if let Some(t) = self.var_class_types.get(root) {
                        Some(t.clone())
                    } else if let Some(t) = self.var_typedef_types.get(root) {
                        Some(t.clone())
                    } else if self.module.classes.contains_key(root) {
                        Some(root.clone())
                    } else if let Some(Some(ctx)) = self.class_context_stack.last().cloned() {
                        self.class_prop_type_named(&ctx, root)
                    } else {
                        None
                    };
                    if let Some(mut tn) = curr_type {
                        let mut ok = true;
                        for seg in &hier.path[1..] {
                            if let Some(next) = self.lookup_type_member(&tn, &seg.name.name) {
                                tn = next;
                            } else {
                                ok = false;
                                break;
                            }
                        }
                        if ok {
                            return Some(tn);
                        }
                    }
                    let members: Vec<String> =
                        hier.path[1..].iter().map(|s| s.name.name.clone()).collect();
                    return self.struct_member_class_type(&hier.path[0].name.name, &members);
                }
                None
            }
            // LRM §6.19.6: `<enum_var>.next()` / `.prev()` / `.first()`
            // / `.last()` returns a value of the enum type. Inherit
            // the receiver's typedef name so a chained `.name()` finds
            // the member list.
            ExprKind::Call { func, args } if args.is_empty() => {
                let m = match &func.kind {
                    ExprKind::MemberAccess { member, .. } => Some(member.name.clone()),
                    ExprKind::Ident(h) if h.path.len() >= 2 => {
                        h.path.last().map(|s| s.name.name.clone())
                    }
                    _ => None,
                };
                if matches!(
                    m.as_deref(),
                    Some("next") | Some("prev") | Some("first") | Some("last")
                ) {
                    let receiver: Option<Expression> = match &func.kind {
                        ExprKind::MemberAccess { expr: recv, .. } => Some((**recv).clone()),
                        ExprKind::Ident(h) if h.path.len() >= 2 => {
                            let head = hier_prefix(h);
                            Some(Expression::new(ExprKind::Ident(head), expr.span))
                        }
                        _ => None,
                    };
                    if let Some(r) = receiver {
                        return self.get_expr_type_name(&r);
                    }
                }
                None
            }
            // LRM §8.4: `arr[i]` / `arr[i][j]` / `arr[i][j][k]` element
            // type. Walks through chained Index nodes to find the root
            // Ident, then consults `array_elem_class` for the registered
            // class name. Handles 1D, 2D, and N-D arrays.
            ExprKind::Index { expr: base, .. } => {
                let mut cur: &Expression = base.as_ref();
                while let ExprKind::Index { expr: inner, .. } = &cur.kind {
                    cur = inner.as_ref();
                }
                if let ExprKind::Ident(bh) = &cur.kind {
                    let bname = self.resolve_hier_name(bh);
                    if let Some(cn) = self.module.array_elem_class.get(&*bname).cloned() {
                        return Some(cn);
                    }
                    // §8.20 + §6.20.3: a class-MEMBER array whose element
                    // type is a TYPE PARAMETER (e.g. `PARAM pool[string]` in
                    // a generic `class Base#(type PARAM)` = `uvm_pool#(T)`).
                    // It has no compile-time `array_elem_class` entry because
                    // the element type isn't concrete until specialization.
                    // Resolve the field's type-param element type through the
                    // current `this` instance's bindings so that `pool[k] =
                    // new(...)` constructs the bound concrete class (`myitem`
                    // for `Base#(myitem)`), not a null/empty handle.
                    if let Some(h) = self.this_stack.last().copied().flatten() {
                        if let Some(inst) = self.heap.get(h).and_then(|o| o.as_ref()) {
                            let this_cn = inst.class_name.clone();
                            let base = self
                                .extract_spec_from_string(&this_cn)
                                .map(|(b, _)| b)
                                .unwrap_or(this_cn);
                            if let Some(elem_decl) = self.class_prop_type_named(&base, &bname) {
                                // Bound concrete type if the element's declared
                                // type is a type param of `base`; else the
                                // declared type itself.
                                let concrete = self
                                    .resolve_type_param_with(&elem_decl, &self.current_spec)
                                    .unwrap_or(elem_decl);
                                let cb = self
                                    .extract_spec_from_string(&concrete)
                                    .map(|(b, _)| b)
                                    .unwrap_or_else(|| concrete.clone());
                                if self.module.classes.contains_key(&cb) {
                                    return Some(concrete);
                                }
                            }
                        }
                    }
                    // §6.19/§21.2.1: an ENUM-typed collection element.
                    // `array_elem_class` only records CLASS element types, so an
                    // enum array (`numbers numa[]`) fell through to None and
                    // `.name()`/one enum method rendered the value against a
                    // GLOBAL scan, emitting a wrong (larger outer-scope) enum's
                    // label (e.g. UVM_NORADIX for 0). The base array name itself
                    // resolves to its element typedef (`numa` -> `numbers`), so
                    // return that when the kernel knows it as an enum.
                    let base_type = self.get_expr_type_name(cur);
                    if let Some(t) = base_type {
                        if self.module.enum_members.contains_key(&t) {
                            return Some(t);
                        }
                    }
                }
                None
            }
            // `ClassName::static_prop` or `obj.field` — resolve the
            // member's declared class type.
            ExprKind::MemberAccess { expr: base, member } => {
                // `this.field` — the current instance's class (its dynamic
                // type, so an inherited property resolves too).
                if matches!(base.kind, ExprKind::This) {
                    if let Some(Some(h)) = self.this_stack.last().copied() {
                        if let Some(Some(inst)) = self.heap.get(h) {
                            let cn = inst.class_name.clone();
                            return self.class_prop_type_named(&cn, &member.name);
                        }
                    }
                    return None;
                }
                if let ExprKind::Ident(bh) = &base.kind {
                    if bh.path.len() == 1 {
                        let bname = &bh.path[0].name.name;
                        // ClassName::static_prop
                        if self.module.classes.contains_key(bname) {
                            return self.class_prop_type_named(bname, &member.name);
                        }
                        // obj.field — find obj's runtime class
                        let handle = self.eval_ident_handle(bname).unwrap_or(0);
                        if handle != 0 {
                            if let Some(Some(inst)) = self.heap.get(handle) {
                                let cn = inst.class_name.clone();
                                return self.class_prop_type_named(&cn, &member.name);
                            }
                        }
                    } else if bh.path.len() >= 2 && bh.path.iter().all(|s| s.selects.is_empty()) {
                        // `ClassName::static_obj.member` — resolve the base
                        // chain's type, then look up the member.
                        if let Some(tn) = self.get_expr_type_name(base) {
                            if let Some(mt) = self.class_prop_type_named(&tn, &member.name) {
                                return Some(mt);
                            }
                        }
                    }
                }
                // A member of one element of an interface instance array
                // (`B[0].proxy`): the element's members are signals under
                // `B[0].`, carrying their declared type. Without this, the
                // lookup below found no type and `B[0].proxy = new;` stored
                // nothing.
                if let ExprKind::Index { expr: ib, index } = &base.kind {
                    if let ExprKind::Ident(bh) = &ib.kind {
                        if bh.path.iter().all(|s| s.selects.is_empty()) {
                            if let Some(k) = self.eval_scalar_self(index) {
                                let name = format!(
                                    "{}[{}].{}",
                                    self.resolve_hier_name(bh),
                                    k,
                                    member.name
                                );
                                if let Some(t) = self
                                    .signal_name_to_id
                                    .get(name.as_str())
                                    .and_then(|id| self.signal_type_names.get(id))
                                {
                                    return Some(t.clone());
                                }
                            }
                        }
                    }
                }
                if let Some(base_type) = self.get_expr_type_name(base) {
                    // §8.15: a member of a handle reached through a chain
                    // (`next.next.next = new(40)` in a method): the base's
                    // type is a class, whose declaration types the member.
                    if self.module.classes.contains_key(&base_type) {
                        if let Some(t) = self.class_prop_type_named(&base_type, &member.name) {
                            return Some(t);
                        }
                    }
                    let dt_opt = self.module.typedef_types.get(&base_type).cloned();
                    if let Some(dt) = dt_opt {
                        let resolved = Self::resolve_type_ref(&dt, &self.module.typedef_types);
                        if let DataType::Struct(su) = resolved {
                            for m in &su.members {
                                for d in &m.declarators {
                                    if d.name.name == member.name {
                                        if let DataType::TypeReference { name, .. } = &m.data_type {
                                            return Some(name.name.name.clone());
                                        }
                                        if let DataType::TypeReference { name, .. } =
                                            self.resolve_dt(&m.data_type)
                                        {
                                            return Some(name.name.name.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                let mut segs = Self::flatten_member_path(base)?;
                segs.push(member.name.clone());
                let root = segs.remove(0);
                self.struct_member_class_type(&root, &segs)
            }
            _ => None,
        }
    }

    /// Return the declared class base name and specialization type-args for a
    /// class-typed expression (a procedural local or a hier-resolved name),
    /// for `$typename`. `None` if the expression is not a class-typed
    /// variable.
    pub(super) fn class_declared_type_of(
        &self,
        expr: &Expression,
    ) -> Option<(String, Vec<Expression>)> {
        if let ExprKind::Ident(hier) = &expr.kind {
            let name = self.resolve_hier_name(hier);
            // Frame overlay first, exactly as `class_of_var` orders it (see
            // `local_type_stack`). The `var_class_types` maps below are keyed
            // by BARE name and shared by every process, so an unrelated task
            // suspended with a same-named local of another class decided this
            // name's type and `$typename` reported that class (issue #239).
            let bname = &hier.path[0].name.name;
            for key in [&*name, bname.as_str()] {
                if let Some(base) = self.local_class_type_of(key) {
                    let ta = self.var_type_args.get(key).cloned().unwrap_or_default();
                    return Some((base, ta));
                }
            }
            if let Some(base) = self.var_class_types.get(&*name) {
                let ta = self.var_type_args.get(&*name).cloned().unwrap_or_default();
                return Some((base.clone(), ta));
            }
            if let Some(base) = self.var_class_types.get(bname) {
                let ta = self.var_type_args.get(bname).cloned().unwrap_or_default();
                return Some((base.clone(), ta));
            }
        }
        None
    }

    /// Format the `$typename` string for a class type (IEEE 1800-2017 §21.7):
    /// `class <name>` with any specialization as `#(<args>)`, where a type
    /// argument renders as `class <name>` (recursive) and a value argument
    /// as its literal.
    pub(super) fn format_class_typename(&mut self, base: &str, type_args: &[Expression]) -> String {
        if !type_args
            .iter()
            .any(|a| matches!(a.kind, ExprKind::NamedArg { .. }))
        {
            let parts: Vec<String> = type_args
                .iter()
                .map(|a| self.format_typename_arg(a))
                .collect();
            return self.class_typename_with_args(base, parts);
        }
        // §8.25 named arguments render at their parameters' slots; a slot
        // the list leaves out shows its default.
        let order: Vec<String> = self
            .module
            .classes
            .get(base)
            .map(|cd| cd.param_order.clone())
            .unwrap_or_default();
        let mut slots: Vec<Option<&Expression>> = vec![None; order.len()];
        for (k, a) in type_args.iter().enumerate() {
            match &a.kind {
                ExprKind::NamedArg { name, expr } => {
                    if let (Some(i), Some(x)) =
                        (order.iter().position(|p| *p == name.name), expr.as_deref())
                    {
                        slots[i] = Some(x);
                    }
                }
                _ => {
                    if let Some(s) = slots.get_mut(k) {
                        *s = Some(a);
                    }
                }
            }
        }
        let used = slots.iter().rposition(|s| s.is_some()).map_or(0, |i| i + 1);
        let mut parts: Vec<String> = Vec::with_capacity(order.len());
        for slot in slots.iter().take(used) {
            let p = match slot {
                Some(x) => self.format_typename_arg(x),
                None => match self.typename_default_part(base, parts.len(), &parts) {
                    Some(p) => p,
                    None => break,
                },
            };
            parts.push(p);
        }
        self.class_typename_with_args(base, parts)
    }

    /// `class <name> #(<args>)` from the rendered leading arguments: the
    /// reference simulator lists every parameter, so the ones left out take
    /// their declared defaults.
    pub(super) fn class_typename_with_args(&self, base: &str, mut parts: Vec<String>) -> String {
        let head = self.class_typename_head(base);
        while let Some(p) = self.typename_default_part(base, parts.len(), &parts) {
            parts.push(p);
        }
        if parts.is_empty() {
            return head;
        }
        format!("{} #({})", head, parts.join(", "))
    }

    /// The `$typename` rendering of parameter `i` of `base` left at its
    /// declared default, `parts` holding the renderings of the parameters
    /// before it. A default naming an earlier parameter (`type U = T`,
    /// `int D = W*2`) follows that parameter's value (§6.20.2). None past
    /// the last parameter, or for a default that cannot be rendered.
    pub(super) fn typename_default_part(
        &self,
        base: &str,
        i: usize,
        parts: &[String],
    ) -> Option<String> {
        let cd = self.module.classes.get(base)?;
        let name = cd.param_order.get(i)?;
        if let Some((_, t)) = cd.type_param_defaults.iter().find(|(n, _)| n == name) {
            let t: String = t.chars().filter(|c| !c.is_whitespace()).collect();
            if let Some(j) = cd.param_order[..i.min(parts.len())]
                .iter()
                .position(|p| *p == t)
            {
                return Some(parts[j].clone());
            }
            return Some(self.format_typename_text_one(&t));
        }
        if cd.type_param_names.contains(name) {
            return Some("int".to_string());
        }
        let init = cd
            .param_defaults
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, e)| e.as_ref())?;
        match &init.kind {
            ExprKind::Number(_) => {
                super::elaborate::const_eval_i64_with_params(init, None).map(|v| v.to_string())
            }
            _ => self.param_default_fragment(cd, i, parts),
        }
    }

    /// `class <name>` for `$typename`, qualified by the declaring package
    /// (`class p::C`) as the reference simulator prints it.
    pub(super) fn class_typename_head(&self, base: &str) -> String {
        match self.module.class_decl_pkg.get(base) {
            Some(pkg) => format!("class {}::{}", pkg, self.class_source_name(base)),
            None if base.contains('.')
                && self
                    .module
                    .classes
                    .get(base)
                    .is_some_and(|cd| cd.declaring_module.is_some()) =>
            {
                format!("class {}", self.hier_path(base).replace('.', "/"))
            }
            None => format!("class {}", base),
        }
    }

    /// The name a package class was declared with. A class whose name is
    /// also declared in another package is elaborated under its qualified
    /// spelling `pkg::C` (§26.3, see `xezim_core::pkg_scope`); this strips
    /// that prefix again for output that names the class.
    pub(super) fn class_source_name<'a>(&self, base: &'a str) -> &'a str {
        self.module
            .class_decl_pkg
            .get(base)
            .and_then(|pkg| base.strip_prefix(pkg.as_str()))
            .and_then(|rest| rest.strip_prefix("::"))
            .unwrap_or(base)
    }

    /// `class_typename_with_args` from a specialization's raw `#(...)` text.
    pub(super) fn class_spec_typename(&self, base: &str, sig: Option<&str>) -> String {
        let parts = sig
            .map(|s| self.typename_args_text_parts(s))
            .unwrap_or_default();
        self.class_typename_with_args(base, parts)
    }

    /// §20.6.1 `$typename` of a class-typed operand: `this` names the class
    /// declaring the running method (with the object's specialization), any
    /// other operand its declared class type. None when the operand is not
    /// class-typed.
    pub(super) fn class_operand_typename(&mut self, arg: &Expression) -> Option<String> {
        if matches!(arg.kind, ExprKind::This) {
            let cls = self.class_context_stack.last().cloned().flatten()?;
            let cls = self.class_origin(&cls).to_string();
            let sig = self
                .this_stack
                .last()
                .copied()
                .flatten()
                .and_then(|h| self.heap.get(h))
                .and_then(|o| o.as_ref())
                .and_then(|i| i.spec.clone())
                .filter(|(b, _)| *b == cls)
                .map(|(_, s)| s);
            return Some(self.class_spec_typename(&cls, sig.as_deref()));
        }
        let tn = self.get_expr_type_name(arg)?;
        let tn = self.resolve_type_param_binding(&tn).unwrap_or(tn);
        if let Some((b, sig)) = self
            .extract_spec_from_string(&tn)
            .or_else(|| self.resolve_typedef_spec(&tn))
        {
            return Some(self.class_spec_typename(&b, Some(&sig)));
        }
        let cls = if self.module.classes.contains_key(&tn) {
            tn
        } else {
            self.resolve_simple_typedef_class(&tn)?
        };
        if let ExprKind::Ident(h) = &arg.kind {
            let n = self.resolve_hier_name(h).into_owned();
            if let Some(ta) = self.module.class_type_args.get(&n).cloned() {
                return Some(self.format_class_typename(&cls, &ta));
            }
        }
        Some(self.class_spec_typename(&cls, None))
    }

    /// Format a single specialization argument (a type or a value).
    pub(super) fn format_typename_arg(&mut self, arg: &Expression) -> String {
        match &arg.kind {
            ExprKind::Specialization {
                base,
                type_args_text,
            } => {
                let bn = self.expr_type_leaf_name(base);
                let parts = self.typename_args_text_parts(type_args_text);
                self.class_typename_with_args(&bn, parts)
            }
            ExprKind::Ident(hier) if hier.path.len() == 1 && hier.path[0].selects.is_empty() => {
                let n = &hier.path[0].name.name;
                if self.module.classes.contains_key(n) {
                    self.class_typename_head(n)
                } else if self.module.covergroups.contains_key(n.as_str()) {
                    format!("class {}", n)
                } else {
                    n.clone()
                }
            }
            ExprKind::Number(NumberLiteral::Integer { value, .. }) => value.clone(),
            ExprKind::StringLiteral(s) => format!("\"{}\"", s),
            // A package-qualified class (`p::C`).
            ExprKind::Ident(hier)
                if hier.path.len() >= 2
                    && hier.path.iter().all(|s| s.selects.is_empty())
                    && self
                        .module
                        .classes
                        .contains_key(&hier.path.last().unwrap().name.name) =>
            {
                self.class_typename_head(&hier.path.last().unwrap().name.name)
            }
            ExprKind::MemberAccess { member, .. }
                if self.module.classes.contains_key(&member.name) =>
            {
                self.class_typename_head(&member.name)
            }
            _ => {
                let v = self.eval_expr(arg);
                match v.to_u64() {
                    Some(n) => n.to_string(),
                    None => "logic".to_string(),
                }
            }
        }
    }

    /// Recursively format the raw `#(...)` text of a `Specialization` node
    /// (space-joined source tokens) into typename arguments.
    pub(super) fn typename_args_text_parts(&self, text: &str) -> Vec<String> {
        let norm: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        if norm.is_empty() {
            return Vec::new();
        }
        // Split on top-level commas (respecting ()/[] nesting).
        let mut parts = Vec::new();
        let mut depth = 0i32;
        let mut cur = String::new();
        for ch in norm.chars() {
            match ch {
                '(' | '[' => {
                    depth += 1;
                    cur.push(ch);
                }
                ')' | ']' => {
                    depth -= 1;
                    cur.push(ch);
                }
                ',' if depth == 0 => parts.push(std::mem::take(&mut cur)),
                _ => cur.push(ch),
            }
        }
        if !cur.is_empty() {
            parts.push(cur);
        }
        parts
            .iter()
            .map(|p| self.format_typename_text_one(p))
            .collect()
    }

    /// Format one raw-text type argument (after whitespace removal).
    pub(super) fn format_typename_text_one(&self, s: &str) -> String {
        if let Some(pos) = s.find("#(") {
            let base = &s[..pos];
            if s.ends_with(')') {
                let inner = &s[pos + 2..s.len() - 1];
                let base = base.rsplit("::").next().unwrap_or(base);
                return self.class_typename_with_args(base, self.typename_args_text_parts(inner));
            }
        }
        let leaf = s.rsplit("::").next().unwrap_or(s);
        if self.module.classes.contains_key(leaf) {
            self.class_typename_head(leaf)
        } else if self.module.covergroups.contains_key(s) {
            format!("class {}", s)
        } else {
            s.to_string()
        }
    }

    /// Extract the leaf identifier name from an expression's type base.
    pub(super) fn expr_type_leaf_name(&self, e: &Expression) -> String {
        match &e.kind {
            ExprKind::Ident(hier) => hier
                .path
                .last()
                .map(|s| s.name.name.clone())
                .unwrap_or_default(),
            ExprKind::Specialization { base, .. } => self.expr_type_leaf_name(base),
            _ => String::new(),
        }
    }

    /// Resolve a bare identifier to a class handle, if it currently
    /// holds one (local variable, `this` property, or static property).
    /// §8.15: `super.<prop>` names the SAME storage as `this.<prop>` — only
    /// METHOD lookup differs, and a method call is an `ExprKind::Call`, never
    /// this shape. Rewriting the leading segment here (a read or an lvalue, so
    /// never a call's callee) keeps `super.foo()` dispatching to the base while
    /// fixing property access, which otherwise flattened to the phantom name
    /// "super.p": reads returned 0 and writes vanished silently.
    /// §8.10 + §8.15: `super.<p>` where `p` is SHADOWED must reach the
    /// nearest strict-ancestor declarer's copy. Rewriting to `this.<p>` (what
    /// the unshadowed path correctly does) would re-resolve from the executing
    /// class and find the wrong copy, so the member is renamed to the
    /// qualified storage key outright.
    pub(super) fn super_rewrite(&self, e: &Expression) -> Option<Expression> {
        if !self.shadowed_prop_names.is_empty() {
            let member: Option<(&str, crate::ast::Span)> = match &e.kind {
                ExprKind::Ident(h)
                    if h.path.len() == 2
                        && h.path[0].name.name == "super"
                        && h.path.iter().all(|s| s.selects.is_empty()) =>
                {
                    Some((h.path[1].name.name.as_str(), e.span))
                }
                ExprKind::MemberAccess { expr: base, member } => match &base.kind {
                    ExprKind::Ident(bh)
                        if bh.path.len() == 1 && bh.path[0].name.name == "super" =>
                    {
                        Some((member.name.as_str(), e.span))
                    }
                    _ => None,
                },
                _ => None,
            };
            if let Some((m, span)) = member {
                if m != "new" {
                    if let Some(qkey) = self.shadowed_prop_key(m, true) {
                        return Some(Expression::new(
                            ExprKind::MemberAccess {
                                expr: Box::new(Expression::new(ExprKind::This, span)),
                                member: crate::ast::Identifier { name: qkey, span },
                            },
                            span,
                        ));
                    }
                }
            } else if let Some(r) = self.shadowed_member_rewrite(e) {
                return Some(r);
            }
        }
        Self::super_as_this(e)
    }

    /// §8.14 + §8.10: `h.p` (and `this.p`, `a.b.p`) where `p` is SHADOWED
    /// names the copy declared by the nearest class at or above the STATIC
    /// type of `h`, not the copy of the object's leaf class: a `Base` handle
    /// to a `Derived` object reads and writes `Base`'s `p`. The member is
    /// renamed to the qualified storage key (`"<Class>::p"`, see
    /// `instantiate_class_with_type_args`) when the two declarers differ.
    /// `None` keeps the bare-name path.
    /// Whether `m` is a shadowed property name (see `shadowed_prop_names`).
    #[inline]
    pub(super) fn prop_maybe_shadowed(&self, m: &str) -> bool {
        let bits = meta_key_bits(m);
        self.shadowed_prop_mask & bits == bits && self.shadowed_prop_names.contains(m)
    }

    #[inline(never)]
    pub(super) fn shadowed_member_rewrite(&self, e: &Expression) -> Option<Expression> {
        let (prefix, m, span): (Expression, &str, crate::ast::Span) = match &e.kind {
            ExprKind::Ident(h)
                if h.path.len() >= 2
                    && h.root.is_none()
                    && h.path.iter().all(|s| s.selects.is_empty()) =>
            {
                let m = h.path.last()?.name.name.as_str();
                if !self.prop_maybe_shadowed(m) || h.path[0].name.name == "super" {
                    return None;
                }
                let p = hier_prefix(h);
                let prefix = if p.path.len() == 1 && p.path[0].name.name == "this" {
                    Expression::new(ExprKind::This, e.span)
                } else {
                    Expression::new(ExprKind::Ident(p), e.span)
                };
                (prefix, m, e.span)
            }
            ExprKind::MemberAccess { expr: base, member } => {
                if !self.prop_maybe_shadowed(member.name.as_str()) {
                    return None;
                }
                let prefix = match &base.kind {
                    ExprKind::Ident(bh)
                        if bh.path.len() == 1
                            && bh.path[0].selects.is_empty()
                            && bh.path[0].name.name == "this" =>
                    {
                        Expression::new(ExprKind::This, base.span)
                    }
                    ExprKind::Ident(bh) if bh.path.iter().any(|s| !s.selects.is_empty()) => {
                        return None;
                    }
                    ExprKind::Ident(_) | ExprKind::This | ExprKind::MemberAccess { .. } => {
                        (**base).clone()
                    }
                    ExprKind::Index { .. } => (**base).clone(),
                    _ => return None,
                };
                (prefix, member.name.as_str(), e.span)
            }
            _ => return None,
        };
        // The static class of the prefix.
        let static_cls: String = if matches!(prefix.kind, ExprKind::This) {
            self.class_context_stack.last().cloned().flatten()?
        } else {
            let tn = self.get_expr_type_name(&prefix)?;
            let tn = match self.extract_spec_from_string(&tn) {
                Some((base, _)) => base,
                None => tn,
            };
            if self.module.classes.contains_key(&tn) {
                tn
            } else {
                self.resolve_simple_typedef_class(&tn)
                    .filter(|c| self.module.classes.contains_key(c))?
            }
        };
        let declarer_from = |start: &str| -> Option<String> {
            let mut cur = Some(start.to_string());
            let mut guard = 0;
            while let Some(cn) = cur {
                guard += 1;
                if guard > 64 {
                    return None;
                }
                let cd = self.module.classes.get(&cn)?;
                if cd.properties.contains_key(m) {
                    return Some(cn);
                }
                cur = cd.extends.clone();
            }
            None
        };
        let target = declarer_from(&static_cls)?;
        let handle = self.eval_handle_expr(&prefix).filter(|&h| h != 0)?;
        let inst = self.heap.get(handle)?.as_ref()?;
        let leaf = declarer_from(&inst.class_name)?;
        if leaf == target {
            return None;
        }
        let qkey = format!("{}::{}", target, m);
        if !inst.properties.contains_key(qkey.as_str()) {
            return None;
        }
        Some(Expression::new(
            ExprKind::MemberAccess {
                expr: Box::new(prefix),
                member: crate::ast::Identifier { name: qkey, span },
            },
            span,
        ))
    }

    pub(super) fn super_as_this(e: &Expression) -> Option<Expression> {
        match &e.kind {
            // Flat form, as written outside a subroutine body.
            // `super.new` / `super.new(...)` is a CONSTRUCTOR call, and the
            // parenthesis-less form parses as this very shape — rewriting it to
            // `this.new` recursed into construction until the stack blew.
            ExprKind::Ident(h)
                if h.path.len() >= 2
                    && h.path[0].name.name == "super"
                    && h.path[h.path.len() - 1].name.name != "new" =>
            {
                let mut nh = h.clone();
                nh.path[0].name.name = "this".to_string();
                nh.cached_signal_id = std::cell::Cell::new(None);
                nh.cached_resolved_name = std::cell::OnceCell::new();
                Some(Expression::new(ExprKind::Ident(nh), e.span))
            }
            // Inside a method body the same text is a MemberAccess chain.
            ExprKind::MemberAccess { expr: base, member } => {
                let ExprKind::Ident(bh) = &base.kind else {
                    return None;
                };
                if bh.path.len() != 1 || bh.path[0].name.name != "super" || member.name == "new" {
                    return None;
                }
                Some(Expression::new(
                    ExprKind::MemberAccess {
                        expr: Box::new(Expression::new(ExprKind::This, base.span)),
                        member: member.clone(),
                    },
                    e.span,
                ))
            }
            _ => None,
        }
    }

    pub(super) fn eval_ident_handle(&self, name: &str) -> Option<usize> {
        // §8.15: `super` names the SAME object as `this` — only the method
        // lookup differs, and property storage is not per-class here. Without
        // this, `super.p` resolved to no handle and flattened to a phantom
        // name: reads returned 0 and writes vanished with no diagnostic, even
        // for a uniquely-named inherited property.
        if name == "super" {
            return self.this_stack.last().copied().flatten();
        }
        if let Some(locals) = self.local_stack.last() {
            if let Some(v) = locals.get(name) {
                return v.to_u64().map(|h| h as usize);
            }
        }
        // A `static` class property must NOT be read from the instance's
        // `properties` map: each specialization's singleton carries a stale
        // construction-time copy that diverges from the shared static cell.
        // Defer to the static fallback below.
        // LRM §8.9: a static property has ONE cell shared across instances.
        let is_static_prop = self
            .class_context_stack
            .last()
            .cloned()
            .flatten()
            .map(|ctx| {
                let mut cur = Some(ctx);
                let mut found = false;
                while let Some(cn) = cur {
                    if let Some(cd) = self.module.classes.get(&cn) {
                        if cd.static_properties.contains(name) {
                            found = true;
                            break;
                        }
                        cur = cd.extends.clone();
                    } else {
                        break;
                    }
                }
                found
            })
            .unwrap_or(false);
        if !is_static_prop {
            if let Some(Some(handle)) = self.this_stack.last() {
                if let Some(Some(inst)) = self.heap.get(*handle) {
                    if let Some(v) = inst.properties.get(name) {
                        return v.to_u64().map(|h| h as usize);
                    }
                }
            }
        }
        if let Some(Some(ctx)) = self.class_context_stack.last() {
            if let Some(key) = self.static_prop_key(ctx, name) {
                if let Some(v) = self.class_statics.get(&key) {
                    return v.to_u64().map(|h| h as usize);
                }
            }
        }
        if let Some(&id) = self.signal_name_to_id.get(name) {
            return self.signal_table[id].to_u64().map(|h| h as usize);
        }
        // Module-scope / procedural-block locals live in the legacy
        // `signals` map (no signal id).
        if let Some(v) = self.signals.get(name) {
            return v.to_u64().map(|h| h as usize);
        }
        None
    }
}
