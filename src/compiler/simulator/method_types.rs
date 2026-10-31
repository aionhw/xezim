//! Simulator method types.

use super::*;

impl Simulator {
    /// Declared return width of the function `func` names, without calling it.
    /// `None` when the callee is not a plain module-scope function (a method,
    /// a class constructor, an array builtin), where the caller falls back to
    /// evaluation.
    /// `e` is a call of a collection's `size()` / `num()` (an int).
    pub(super) fn count_method_call(&mut self, e: &Expression) -> bool {
        let ExprKind::Call { func, args } = &e.kind else {
            return false;
        };
        if !args.is_empty() || !self.callee_method_name_is(func, |m| matches!(m, "size" | "num")) {
            return false;
        }
        if self.collection_method_of(func).is_some() {
            return true;
        }
        // A class member (`s.size()` in a method, `obj.s.size()`): its
        // storage is instance-scoped.
        let recv = match &func.kind {
            ExprKind::MemberAccess { expr, .. } => (**expr).clone(),
            ExprKind::Ident(h) if h.path.len() >= 2 => {
                let mut r = h.clone();
                r.path.pop();
                r.cached_signal_id = std::cell::Cell::new(None);
                r.cached_resolved_name = std::cell::OnceCell::new();
                Expression::new(ExprKind::Ident(r), func.span)
            }
            _ => return false,
        };
        self.array_operand_name(&recv).is_some()
            || self
                .expr_assoc_name(&recv)
                .is_some_and(|n| self.is_associative_array(&n))
    }

    pub(super) fn call_return_width(&mut self, func: &Expression) -> Option<u32> {
        // A queue/dyn-array method reached here through the generic Call
        // fallback and EVALUATED — so `s - q.pop_back()` popped once for the
        // width probe and once for the value, silently draining the queue.
        // Answer the value-returning builtins from the element type instead.
        //
        // GATE: only invoke `collection_method_of` (which runs
        // `flat_member_name` over the receiver on EVERY call) when the
        // callee's method name is one of the value-returning collection
        // builtins. `infer_width` probes every arithmetic/bitwise operand
        // each evaluation, so a class method used in a loop
        // (`sum += e.get_a()`) drove `flat_member_name` through the receiver
        // millions of times only to have `collection_method_of` return None.
        // Class methods (`get_a`, `compute`, …) fall through to
        // `class_method_return_width` below without that cost.
        if self.callee_method_name_is(func, |m| {
            matches!(m, "size" | "num" | "len" | "pop_front" | "pop_back")
        }) {
            if let Some((obj, mname)) = self.collection_method_of(func) {
                match mname.as_str() {
                    "size" | "num" | "len" => return Some(32),
                    "pop_front" | "pop_back" => {
                        let w = self
                            .lookup_signal_width(&format!("{}[0]", obj))
                            .or_else(|| self.module.arrays.get(&obj).map(|t| t.2));
                        if let Some(w) = w.filter(|w| *w > 0) {
                            return Some(w);
                        }
                    }
                    _ => {}
                }
            }
        }
        // A class METHOD call (`recv.m(...)`): the width comes from the
        // method's declared return type, resolved through the inheritance
        // chain — NOT by executing the method. Without this, `infer_width`
        // of a method call fell through to `eval_expr(expr).width` and RAN
        // the method purely to read its width, so any method used inside an
        // arithmetic expression (`a + obj.compute()`) executed twice — once
        // for width, once for the value — double-firing side effects and
        // doubling the per-call cost. `class_method_return_width` is
        // conservative: it returns `None` for anything it can't cheaply
        // resolve, preserving the prior fallback behaviour there.
        if let Some(w) = self.class_method_return_width(func) {
            return Some(w);
        }
        let ExprKind::Ident(h) = &func.kind else {
            return None;
        };
        if h.path.len() != 1 {
            return None;
        }
        let name = &h.path[0].name.name;
        let fd = self.module.functions.get(name)?;
        if matches!(fd.return_type, DataType::Void { .. }) {
            return None;
        }
        if matches!(fd.return_type, DataType::Real { .. }) {
            return Some(64);
        }
        let w = super::elaborate::resolve_type_width(
            &fd.return_type,
            Some(&self.module.parameters),
            Some(&self.module.typedefs),
        );
        (w > 0).then_some(w)
    }

    /// Declared return width of a class METHOD call — `recv.m(...)` parsed as
    /// `MemberAccess`, or `recv.m` as a 2-segment hierarchical identifier —
    /// resolved through the inheritance chain WITHOUT executing the method.
    /// `None` for anything not cheaply resolvable (the caller then keeps its
    /// existing evaluation fallback), so this only ever widens the set of
    /// calls whose width is taken statically. See `call_return_width`.
    pub(super) fn class_method_return_width(&self, func: &Expression) -> Option<u32> {
        // Extract the receiver expression + method name. `infer_width` probes
        // every arithmetic/bitwise operand each evaluation, so keep this free
        // of allocations.
        let (recv_expr, mname): (&Expression, &str) = match &func.kind {
            ExprKind::MemberAccess { expr, member } => (expr, member.name.as_str()),
            // A BARE `f(...)` call inside a class-method frame is
            // this-bounded: its width is the return type of the method
            // resolved from the CURRENT lexical class. Without this arm the
            // width probe EVALUATED the call, so a left-nested binary like
            // f(a) + f(b) + f(c) ran its inner-left operand twice (the
            // probe of the inner calls, then the value pass) — every
            // side-effecting bare call in a left-nested sub-expression
            // fired twice (reference-validated: 3 executions, not 5).
            // Bare-name shadowing stays safe: method_ret_width returns
            // None unless the chain actually declares the method, and the
            // caller then falls through to the module-function lookup.
            ExprKind::Ident(h) if h.path.len() == 1 => {
                if let Some(Some(cn)) = self.class_context_stack.last() {
                    if let Some(w) = self.method_ret_width(cn, h.path[0].name.name.as_str()) {
                        return Some(w);
                    }
                }
                return None;
            }
            ExprKind::Ident(h) if h.path.len() == 2 => {
                // `recv.m(...)` flattened to Ident([recv, m]). Reconstruct a
                // borrowable Identifier for the receiver segment (no
                // allocation).
                let seg = h.path[0].name.name.clone();
                if let Some(cn) = self.var_class_types.get(&seg) {
                    return self.method_ret_width(cn, h.path[1].name.name.as_str());
                } else if let Some(&id) = self.signal_name_to_id.get(seg.as_str()) {
                    if let Some(cn) = self.signal_type_names.get(&id) {
                        return self.method_ret_width(cn, h.path[1].name.name.as_str());
                    }
                } else if let Some(th) = self.this_stack.last().copied().flatten() {
                    if let Some(cn) = self.prop_class_type(th, &seg) {
                        return self.method_ret_width(&cn, h.path[1].name.name.as_str());
                    }
                }
                return None;
            }
            _ => return None,
        };
        let class_name: Option<String> = match &recv_expr.kind {
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                let rn = h.path[0].name.name.clone();
                if let Some(cn) = self.var_class_types.get(&rn) {
                    Some(cn.clone())
                } else if let Some(&id) = self.signal_name_to_id.get(rn.as_str()) {
                    self.signal_type_names.get(&id).cloned()
                } else if let Some(th) = self.this_stack.last().copied().flatten() {
                    self.prop_class_type(th, &rn)
                } else {
                    None
                }
            }
            ExprKind::Index { expr, .. } => Self::collection_element_class(self, expr),
            _ => None,
        };
        let class_name = class_name?;
        self.method_ret_width(&class_name, mname)
    }

    /// Width of a class method's declared return type, resolved through the
    /// inheritance chain from `class_name` (read-only; never executes the
    /// method). `None` for a task / void / unresolvable.
    pub(super) fn method_ret_width(&self, class_name: &str, mname: &str) -> Option<u32> {
        use crate::ast::decl::ClassMethodKind;
        use crate::ast::types::DataType;
        // Walk the inheritance chain read-only to the method's return type.
        // `extends` must be cloned per level: the borrow of a `classes` entry
        // can't be carried across loop iterations (the entry is dropped at
        // each iteration end).
        let mut cur = class_name.to_string();
        while let Some(cd) = self.module.classes.get(&cur) {
            if let Some(m) = cd.methods.get(mname) {
                if let ClassMethodKind::Function(f) = &m.kind {
                    if matches!(f.return_type, DataType::Real { .. }) {
                        return Some(64);
                    }
                    if matches!(f.return_type, DataType::Void { .. }) {
                        return None;
                    }
                    let w = super::elaborate::resolve_type_width(
                        &f.return_type,
                        Some(&self.module.parameters),
                        Some(&self.module.typedefs),
                    );
                    return (w > 0).then_some(w);
                }
                return None; // task / extern / pure prototype
            }
            cur = cd.extends.clone()?;
        }
        None
    }

    /// Class type of the element of a collection-index receiver `q[i]` — the
    /// dynamic-array / queue variable `q`'s element type (a class name when
    /// the elements are class handles). Mirrors how a queue element member
    /// call `cb_q[i].pre(...)` resolves its receiver class statically.
    pub(super) fn collection_element_class(&self, arr_expr: &Expression) -> Option<String> {
        let ExprKind::Ident(h) = &arr_expr.kind else {
            return None;
        };
        if h.path.len() != 1 {
            return None;
        }
        self.collection_element_class_named(&h.path[0].name.name)
    }

    /// Name-based core of `collection_element_class` (see that fn for the
    /// resolution ladder). Split out so the id-collision gates can ask by
    /// bare name (`elem_class_of_arr`) without synthesizing an
    /// `Expression` per call — the gate path runs on every member/indexed
    /// access, and the old synthetic-identifier build cost several
    /// allocations each.
    pub(super) fn collection_element_class_named(&self, name: &str) -> Option<String> {
        if let Some(cls) = self.module.array_elem_class.get(name) {
            return Some(cls.clone());
        }
        if let Some(cls) = self.declared_collection_elem_class(name) {
            return Some(cls);
        }
        // A queue/dyn-array of a TYPEDEF'd class (`cb_type cb_q[$]` where
        // `typedef uvm_event_callback#(T) cb_type;`) has an element type that
        // names the typedef, not a registered class — `array_elem_class` and
        // `declared_collection_elem_class` miss it. Some queue declarations
        // also record an EMPTY unpacked-dimensions list on `var_decl_types`
        // (`cb_type cb_q[$]`), so resolve the (possibly typedef'd) element
        // name to its base class regardless of recorded dimensions.
        if let Some(dt) = self.module.var_decl_types.get(name).cloned() {
            if let Some(elem) = Self::collection_elem_typename(&dt) {
                if let Some(cls) = self.resolve_typeref_class_name(&elem) {
                    return Some(cls);
                }
            }
        }
        // A COLLECTION FIELD of the enclosing class (`base_cb q[$];` as a
        // member of `this`) is not registered in the module-scope tables; its
        // element class comes from the property's declared type. Walk the
        // class-context chain (leaf first) for the property, like the field
        // element-class resolution at a collection assignment, and resolve the
        // declared element type to a class.
        if let Some(Some(ctx)) = self.class_context_stack.last().cloned() {
            let mut cur = Some(ctx);
            let mut seen: HashSet<String> = HashSet::default();
            while let Some(cn) = cur {
                if !seen.insert(cn.clone()) {
                    break;
                }
                let Some(cd) = self.module.classes.get(&cn) else {
                    break;
                };
                if let Some(tn) = cd.properties.get(name).and_then(|s| s.type_name.clone()) {
                    let tn = tn.trim();
                    if let Some(cls) = self.resolve_typeref_class_name_str(tn) {
                        return Some(cls);
                    }
                }
                cur = cd.extends.clone();
            }
        }
        None
    }

    /// Resolve a class name / typedef / specialization string to a class name.
    pub(super) fn resolve_typeref_class_name_str(&self, s: &str) -> Option<String> {
        let tnname = crate::ast::types::TypeName {
            scopes: Vec::new(),
            name: crate::ast::Identifier {
                name: s.to_string(),
                span: crate::ast::Span::dummy(),
            },
            span: crate::ast::Span::dummy(),
        };
        self.resolve_typeref_class_name(&tnname)
    }

    /// Element type of an unpacked-collection DataType, or None for a scalar /
    /// non-collection type.
    pub(super) fn collection_elem_typename(
        dt: &crate::ast::types::DataType,
    ) -> Option<crate::ast::types::TypeName> {
        match dt {
            crate::ast::types::DataType::TypeReference {
                name, dimensions, ..
            } if !dimensions.is_empty() => Some(name.clone()),
            crate::ast::types::DataType::TypeReference { name, .. } => Some(name.clone()),
            _ => None,
        }
    }

    pub(super) fn infer_lhs_width(&mut self, expr: &Expression) -> u32 {
        match &expr.kind {
            ExprKind::Concatenation(p) => {
                let mut total = 0;
                for x in p {
                    total += self.infer_lhs_width(x);
                }
                total
            }
            ExprKind::Ident(h) => {
                if let Some(name) = self.frame_bound_bare_name(h) {
                    if let Some(v) = self.local_stack.last().and_then(|f| f.get(name)) {
                        return v.width.max(1);
                    }
                }
                let is_ambiguous_leaf = h.path.len() == 1 && !h.path[0].name.name.contains('.');
                if let Some(id) = h.cached_signal_id.get() {
                    if !is_ambiguous_leaf {
                        return self.signal_widths[id];
                    }
                }
                // A bare leaf may be a local or a signal, so the id cache above
                // is skipped for it and the whole ladder below re-ran on every
                // assignment. Its STATIC answers (a signal's declared width, a
                // declared local/module width) are remembered per identifier
                // span; the class-property and default answers are not.
                // Synthetic identifiers all carry the dummy (0, 0) span and
                // must not share a cache slot: a never-driven trireg's implicit
                // self-driver read a 1-bit width from another synthetic name.
                let leaf_key = if is_ambiguous_leaf && !(h.span.start == 0 && h.span.end == 0) {
                    let k = (h.span.start as usize, h.span.end as usize);
                    if let Some(&w) = self.lhs_leaf_width_cache.get(&k) {
                        return w;
                    }
                    Some(k)
                } else {
                    None
                };
                let name = self.resolve_hier_name(h);
                // §6.19 a bare-leaf LHS naming an ACTIVE subroutine/block
                // local shadows any same-named design signal — the lvalue's
                // width is the LOCAL's, not the signal's. Without this, the
                // multi-top wrapper's instance signals (`module __xezim_multi_top;
                // top top();` registers a flat width-1 slot "top") leaked into
                // `width` here, and a function-local `uvm_root top` in UVM's
                // `m_uvm_get_root` inherited that 1-bit width — so `top = new()`
                // truncated the constructed handle and `top != m_inst` fired
                // `UVM/BAD_TOP` (infinite recursion) only under multi-top.
                if h.path.len() == 1 && h.path[0].selects.is_empty() {
                    let leaf = &h.path[0].name.name;
                    let is_active_local = self
                        .local_stack
                        .last()
                        .is_some_and(|l| l.contains_key(leaf));
                    if is_active_local {
                        if let Some(w) = self.widths.get(&*name).copied() {
                            if w > 0 {
                                return w;
                            }
                        }
                    }
                }
                if let Some(&id) = self.signal_name_to_id.get(name.as_ref()) {
                    h.cached_signal_id.set(Some(id));
                    if let Some(k) = leaf_key {
                        self.lhs_leaf_width_cache.insert(k, self.signal_widths[id]);
                    }
                    return self.signal_widths[id];
                }
                // A width of 0 is never valid for an lvalue — it usually
                // means the flat `widths` map was polluted by a same-named
                // class-handle elsewhere. Ignore it and fall through.
                if let Some(w) = self.widths.get(&*name).copied() {
                    if w > 0 {
                        if let Some(k) = leaf_key {
                            self.lhs_leaf_width_cache.insert(k, w);
                        }
                        return w;
                    }
                }
                // §7.2.1: a PACKED-STRUCT MEMBER (`s.field`) is a bit slice of
                // `s`, not a signal of its own, so every lookup above misses and
                // the 32-bit default below silently truncated the assignment —
                // a 40-bit field lost its top 8 bits on every NBA. Checked
                // BEFORE the bare-leaf fallback, which would otherwise take the
                // width from an unrelated module-scope signal of the same leaf
                // name.
                if let Some((_, _, pw)) = self.packed_leaf_of_hier(&name) {
                    if pw > 0 {
                        return pw;
                    }
                }
                // §18.4/§7.2: a class aggregate-property member at any depth
                // — its width is the resolved storage's. Gated on classes
                // existing so class-free designs pay nothing, and placed
                // before the bare-leaf fallback so an unrelated module
                // signal of the same leaf name cannot win.
                if !self.no_class_objects() {
                    if let Some(r) = self.class_agg_member(expr) {
                        let w = match r {
                            ClassAggRef::Packed { w, .. }
                            | ClassAggRef::Unpacked { w, .. }
                            | ClassAggRef::UnpackedSlice { w, .. } => w,
                        };
                        if w > 0 {
                            return w;
                        }
                    }
                }
                let leaf = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                if let Some(&id) = self.signal_name_to_id.get(leaf) {
                    h.cached_signal_id.set(Some(id));
                    return self.signal_widths[id];
                }
                // A class property lives on the heap, not in the flat signal
                // maps, so without this it fell through to the 32-bit default
                // and `std::randomize(prop)` drew a full 32-bit value into a
                // 2-bit property.
                if let Some(&Some(handle)) = self.this_stack.last() {
                    if let Some(w) = self.heap_prop_width(handle, leaf) {
                        return w;
                    }
                }
                self.widths
                    .get(leaf)
                    .copied()
                    .filter(|w| *w > 0)
                    .unwrap_or(32)
            }
            ExprKind::RangeSelect {
                expr: rs_base,
                left,
                right,
                kind,
            } => {
                // Signed bounds: `x[0:-1]` is TWO bits wide. Read as u64 the
                // -1 became 0xFFFF_FFFF, `l >= r` picked the wrong branch and
                // the width came out 0 — which then resized the RHS to nothing,
                // so a partially out-of-range NBA wrote no bits at all.
                // An indexed select's width is its constant width alone: its
                // start is not evaluated here, so one with a side effect
                // (`v[i++ +: 8]`) runs only where the target is captured.
                let r = self.eval_expr(right).to_i64().unwrap_or(0);
                let count = match kind {
                    RangeKind::IndexedUp | RangeKind::IndexedDown => r.max(0) as u32,
                    RangeKind::Constant => {
                        let l = self.eval_expr(left).to_i64().unwrap_or(0);
                        (l - r).unsigned_abs() as u32 + 1
                    }
                };
                // §7.4.1: an element range on a packed multi-D lvalue is
                // count × elem_w bits wide, not count bits — the RHS was
                // resized to the raw label count and truncated.
                if let ExprKind::Ident(h) = &rs_base.kind {
                    let nm = self.resolve_hier_name(h);
                    if let Some(&ew) = self
                        .module
                        .packed_signal_elem_widths
                        .get(&*nm)
                        .filter(|&&w| w > 1)
                    {
                        return count * ew;
                    }
                }
                // §18.4/§7.2: a part-select of a class aggregate member —
                // the resolved slice's width (an indexed `+:`/`-:` on a
                // multi-dimension member is elem_w bits per element, not
                // the raw label count).
                if !self.no_class_objects() {
                    if let Some(r) = self.class_agg_member(expr) {
                        let w = match r {
                            ClassAggRef::Packed { w, .. }
                            | ClassAggRef::Unpacked { w, .. }
                            | ClassAggRef::UnpackedSlice { w, .. } => w,
                        };
                        if w > 0 {
                            return w;
                        }
                    }
                }
                count
            }
            ExprKind::Index { expr: e, index: _ } => {
                // Walk chained Index nodes to the root identifier, counting
                // depth, so a multi-D element lvalue (`a[i][j]`, `a[i][j][k]`)
                // resolves to the ELEMENT width. Falling through to 1 here
                // truncated a mailbox/class handle assigned via
                // `arr[i][j] = new()` to a single bit.
                let mut depth = 1usize;
                let mut cur: &Expression = e.as_ref();
                while let ExprKind::Index { expr: inner, .. } = &cur.kind {
                    depth += 1;
                    cur = inner.as_ref();
                }
                // §25.8: for `vif.member[i]` the root is a MemberAccess, not an
                // Ident, so the whole block below was skipped and the lvalue
                // fell through to width 1 — an NBA then resized its RHS to ONE
                // BIT before queueing it, and `vif.data[p] <= flit` wrote the
                // flit's LSB. Resolve the binding to the interface signal's
                // flat name and ask the same width questions about it.
                let root_name: Option<std::borrow::Cow<'_, str>> = match &cur.kind {
                    ExprKind::Ident(h) => Some(self.resolve_hier_name(h)),
                    ExprKind::MemberAccess { .. } => {
                        self.vif_member_flat_name(cur).map(std::borrow::Cow::Owned)
                    }
                    _ => None,
                };
                if let Some(n) = root_name {
                    if depth == 1 {
                        if let Some((_, _, w)) = self.module.arrays.get(&*n) {
                            return *w;
                        }
                        // §7.8: an associative element has the declared
                        // element width (`ma[k] = (1 << 69) - 1`).
                        let scoped = self.dyn_name_lookup(&n).map(|s| s.to_string());
                        let an = scoped.as_deref().unwrap_or(&n);
                        if self.is_associative_array(an)
                            && let Some(w) = self.assoc_elem_width(an).filter(|&w| w > 0)
                        {
                            return w;
                        }
                    }
                    if depth == 2 {
                        if let Some((_, _, w)) = self.module.arrays_2d.get(&*n) {
                            return *w;
                        }
                    }
                    if let Some((shape, w)) = self.module.arrays_nd.get(&*n) {
                        if depth == shape.len() {
                            return *w;
                        }
                    }
                    // Multi-D PACKED vector (`logic [1:0][3:0][7:0] foo`):
                    // `foo[i]`/`foo[i][j]` select a SLICE — its width is the
                    // product of the remaining dims, not 1 bit. §7.4: UNPACKED
                    // dimensions are indexed FIRST, so when the base is an
                    // unpacked array of packed vectors (`logic [1:0][7:0] v
                    // [0:1]`, `v[0][0]`) the leading indices consume the
                    // unpacked dims and only the rest index the packed ones.
                    // Without this subtraction the packed depth ran past the
                    // dim list and the lvalue fell through to width 1 — a
                    // continuous assign then resized its RHS to one bit and
                    // wrote the LSB (0x12 -> 0).
                    let num_unpacked = if let Some((d, _)) = self.module.arrays_nd.get(&*n) {
                        d.len()
                    } else if self.module.arrays_2d.contains_key(&*n) {
                        2
                    } else if self.module.arrays.contains_key(&*n)
                        || self.module.dynamic_arrays.contains(&*n)
                        || self.is_associative_array(&n)
                    {
                        1
                    } else {
                        0
                    };
                    if let Some(dims) = self.module.packed_full_dims.get(&*n) {
                        let pdepth = depth.saturating_sub(num_unpacked);
                        if pdepth < dims.len() {
                            let w: u64 = dims[pdepth..]
                                .iter()
                                .map(|(l, r)| (l - r).unsigned_abs() + 1)
                                .product();
                            if w > 1 {
                                return w as u32;
                            }
                        }
                    } else if depth == 1 {
                        if let Some(&ew) = self.module.packed_signal_elem_widths.get(&*n) {
                            if ew > 1 {
                                return ew;
                            }
                        }
                    }
                }
                // §18.4/§7.2: an element of a class aggregate member —
                // width from the resolved storage (a packed-dimension slot
                // of a struct member fell through to 1 here, resizing an
                // NBA's RHS to one bit before queueing it).
                if !self.no_class_objects() {
                    if let Some(r) = self.class_agg_member(expr) {
                        let w = match r {
                            ClassAggRef::Packed { w, .. }
                            | ClassAggRef::Unpacked { w, .. }
                            | ClassAggRef::UnpackedSlice { w, .. } => w,
                        };
                        if w > 0 {
                            return w;
                        }
                    }
                }
                1
            }
            _ => self.infer_width(expr),
        }
    }
}
