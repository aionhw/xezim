//! Simulator arguments.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// `exec_let_call` in an expression with a context width: a let instance
    /// is its parenthesized body, so it takes part in the width of the
    /// expression around it (`r8 = p::sum(a4, b4)` keeps the carry). None
    /// when `func` does not name a let (or a subroutine of that name wins).
    #[cold]
    #[inline(never)]
    pub(super) fn let_call_ctx(
        &mut self,
        func: &Expression,
        args: &[Expression],
        ctx_width: u32,
    ) -> Option<Value> {
        // A bare name (imported or compilation-unit let) or `pkg::name` (the
        // parser flattens the scope into a two-segment identifier).
        let ExprKind::Ident(h) = &func.kind else {
            return None;
        };
        if h.root.is_some() || h.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let name = h.path.last()?.name.name.as_str();
        let ld = match h.path.len() {
            1 => self.module.lets.get(name)?.clone(),
            2 if self.module.packages.contains(&h.path[0].name.name) => {
                let qual = format!("{}::{}", h.path[0].name.name, name);
                self.module
                    .lets
                    .get(&qual)
                    .or_else(|| self.module.lets.get(name))?
                    .clone()
            }
            _ => return None,
        };
        if self.module.functions.contains_key(name) || self.module.classes.contains_key(name) {
            return None;
        }
        if let Some(Some(ctx)) = self.class_context_stack.last().cloned()
            && self.class_has_method(&ctx, name)
        {
            return None;
        }
        let e = ld.expand(args, func.span).ok()?;
        Some(self.eval_expr_ctx(&e, ctx_width))
    }

    /// IEEE 1800-2023 §11.13: a let the parser could not expand where it was
    /// written (a compilation-unit let, or a package let reached through an
    /// import or `pkg::`). The instance is the let body with the actuals
    /// substituted for the formals (typed formals cast, omitted ones taking
    /// their defaults), evaluated self-determined.
    pub(super) fn exec_let_call(&mut self, ld: &LetDeclaration, args: &[Expression]) -> Value {
        let span = args.first().map(|a| a.span).unwrap_or(ld.span);
        match ld.expand(args, span) {
            Ok(e) => self.eval_expr(&e),
            Err(msg) => {
                eprintln!("Error: {} (IEEE 1800-2023 §11.13)", msg);
                Value::zero(32)
            }
        }
    }

    /// Is a method's DECLARED return type a `string` AFTER resolving a TYPE
    /// PARAMETER to its concrete bound (e.g. `virtual function T do_read();`
    /// in `res#(T)`, specialized to `T=string`)? `is_string_data_type`
    /// cannot answer that — the AST holds `TypeReference("T")`, and without
    /// resolving it xezim treats the return as an integral width and
    /// truncates an N-char string to 4 chars when the method returns. Only a
    /// plainly-`string` verdict short-circuits that resize.
    pub(super) fn method_return_is_string(&self, rt: &crate::ast::types::DataType) -> bool {
        use crate::ast::types::DataType;
        if Self::is_string_data_type(rt) {
            return true;
        }
        let DataType::TypeReference { name: tn, .. } = rt else {
            return false;
        };
        // A plain class name is no type parameter, instance binding or
        // typedef (see `plain_class_ref`), so the walk below ends at once.
        if self.plain_class_ref(rt).is_some() {
            return false;
        }
        // Follow a typedef chain, then a type-parameter binding, then its
        // typedef, back to the base kind.
        let mut n = tn.name.name.to_string();
        // Guard against a self-named typedef alias (`typedef req_t #(..)
        // req_t;`) — see `p_local_is_string`.
        let mut seen: Vec<String> = Vec::new();
        loop {
            if seen.iter().any(|sn| *sn == n) || seen.len() >= 16 {
                return false;
            }
            seen.push(n.clone());
            // A type parameter in the ACTIVE specialization binds to a
            // concrete type name (`T` -> `string` / `int` / ...).
            let bound = self
                .resolve_type_param_binding(&n)
                .map(|s| s.to_string())
                .or_else(|| {
                    // `current_spec` is cleared for a BASE/inherited method
                    // reached via `super` (e.g. `super.do_read()` executes
                    // `res#(string)::do_read` with no active specialization),
                    // so fall back to the RECEIVER object's type bindings.
                    let h = self.this_stack.last().copied().flatten()?;
                    let inst = self.heap.get(h)?.as_ref()?;
                    inst.type_bindings.get(&n).cloned()
                });
            if let Some(bound) = bound {
                if bound == "string" {
                    return true;
                }
                let bdt = self.module.typedef_types.get(&bound).cloned();
                if let Some(bd) = bdt {
                    if Self::is_string_data_type(&bd) {
                        return true;
                    }
                    if let DataType::TypeReference { name: bn, .. } = &bd {
                        n = bn.name.name.to_string();
                        continue;
                    }
                }
                return false;
            }
            let bdt = self.module.typedef_types.get(&n).cloned();
            if let Some(bd) = bdt {
                if Self::is_string_data_type(&bd) {
                    return true;
                }
                if let DataType::TypeReference { name: bn, .. } = &bd {
                    n = bn.name.name.to_string();
                    continue;
                }
                return false;
            }
            return false;
        }
    }

    /// Is `dt` directly the `string` data type (no typedef/param resolution)?
    pub(super) fn is_string_data_type(dt: &crate::ast::types::DataType) -> bool {
        matches!(
            dt,
            crate::ast::types::DataType::Simple {
                kind: crate::ast::types::SimpleType::String,
                ..
            }
        )
    }

    /// IEEE 1800-2017 §12.5.4 `case (e) inside`: an item is a range, a value or
    /// an array, matched with the same rules as the `inside` OPERATOR — ranges
    /// are inclusive and x/z in an item are wildcards. `CaseInside` fell through
    /// to exact `case_eq`, so a range item compared against a garbage value and
    /// a wildcard item never matched: both silently took `default`.
    pub(super) fn case_inside_match(&mut self, val: &Value, pat: &Expression) -> bool {
        if let ExprKind::Range(lo, hi) = &pat.kind {
            // §11.4.13: `$` as the LOW bound is the type's smallest value,
            // as the HIGH bound its largest — an unbounded end always
            // matches (the Dollar eval's u32::MAX sentinel matched nothing
            // as a low bound).
            let lo_ok = matches!(lo.kind, ExprKind::Dollar)
                || val.greater_equal(&self.eval_expr(lo)).is_true();
            let hi_ok = matches!(hi.kind, ExprKind::Dollar)
                || val.less_equal(&self.eval_expr(hi)).is_true();
            return lo_ok && hi_ok;
        }
        if let Some(nm) = self.array_operand_name(pat) {
            let sz = self.get_queue_size(&nm);
            for i in 0..sz {
                if let Some(ev) = self.get_signal_value_by_name(&format!("{}[{}]", nm, i)) {
                    if val.wildcard_eq(&ev).is_true() {
                        return true;
                    }
                }
            }
            return false;
        }
        let pv = self.eval_expr(pat);
        val.wildcard_eq(&pv).is_true()
    }

    /// IEEE 1800-2017 §6.12.2: converting a real to an integral type ROUNDS to
    /// the nearest, ties away from zero, and the integer wraps to the target
    /// width (a target wider than 64 bits keeps every bit). One rule for the
    /// simulator and the elaborator: `Value::from_real_wrapped`.
    #[inline]
    pub(super) fn real_to_int(f: f64, width: u32) -> Value {
        Value::from_real_wrapped(f, width)
    }

    /// §13.4.1: how a function's value converts to its declared return type,
    /// for `ty` already resolved through its typedefs. Anything that is not a
    /// numeric scalar (a handle, string, void, unpacked aggregate or an
    /// unresolved type) keeps its value.
    pub(super) fn ret_conv_of(ty: &DataType, typedef_types: &HashMap<String, DataType>) -> RetConv {
        match ty {
            DataType::Real { .. } => RetConv::Real,
            DataType::IntegerVector { .. }
            | DataType::IntegerAtom { .. }
            | DataType::Enum(_)
            | DataType::Implicit { .. } => RetConv::Int {
                two_state: super::elaborate::is_type_two_state_resolved(ty, typedef_types),
            },
            DataType::Struct(su) if su.packed => RetConv::Int {
                two_state: super::elaborate::is_type_two_state_resolved(ty, typedef_types),
            },
            _ => RetConv::Keep,
        }
    }

    /// `ret_conv_of` for a free function's declared return type.
    pub(super) fn fn_ret_conv(&self, rt: &DataType) -> RetConv {
        let tt = &self.module.typedef_types;
        Self::ret_conv_of(super::elaborate::resolve_typedef_chain(rt, tt), tt)
    }

    /// `ret_conv_of` for a class method's declared return type: a typedef of
    /// the class (or an ancestor) wins over a same-named global one, and a
    /// type parameter takes the type bound on the running object (§8.25).
    pub(super) fn method_ret_conv(&self, rt: &DataType, cname: &str) -> RetConv {
        let tt = &self.module.typedef_types;
        if let DataType::TypeReference {
            name, dimensions, ..
        } = rt
            && !name.has_scope()
            && dimensions.is_empty()
        {
            let tn = name.name.name.as_str();
            if self.module.classes.contains_key(tn) {
                return RetConv::Keep;
            }
            if let Some(dt) = self.class_local_typedef(cname, tn) {
                return Self::ret_conv_of(super::elaborate::resolve_typedef_chain(dt, tt), tt);
            }
            if self.class_member_names().params.contains(tn) {
                return match self.type_param_concrete_in(rt, cname) {
                    Some(c) => self.concrete_ret_conv(&c),
                    None => RetConv::Keep,
                };
            }
        }
        self.fn_ret_conv(rt)
    }

    /// `ret_conv_of` for a concrete type-parameter binding, given by name.
    pub(super) fn concrete_ret_conv(&self, name: &str) -> RetConv {
        if matches!(name, "real" | "realtime" | "shortreal") {
            return RetConv::Real;
        }
        if let Some(dt) = self.module.typedef_types.get(name) {
            let tt = &self.module.typedef_types;
            return Self::ret_conv_of(super::elaborate::resolve_typedef_chain(dt, tt), tt);
        }
        if matches!(name, "bit" | "logic" | "reg")
            || atom_type_keyword_width(name).is_some()
            || vector_fragment_width(name).is_some()
        {
            return RetConv::Int {
                two_state: self.concrete_type_two_state(name),
            };
        }
        RetConv::Keep
    }

    /// §13.4.1 / §10.7: convert a returned value to the numeric kind of its
    /// declared return type, exactly as assigning it to a variable of that
    /// type would. A real converts to an integral return by rounding
    /// (§6.12.2) — before this, `function int f(); return 7.6;` handed the
    /// caller the real 7.6 (and a 64-bit return its raw f64 bits) — and an
    /// integral value converts to a real return. Width and signedness are
    /// fitted by the caller; a 2-state return also maps X/Z to 0 (§6.3.1),
    /// which the caller applies after that fit via `ret_two_state`.
    /// `width` (0 = 64) and `signed` size an integral result for a caller
    /// that does not fit it afterwards.
    #[cold]
    #[inline(never)]
    pub(super) fn ret_convert(v: Value, conv: RetConv, width: u32, signed: Option<bool>) -> Value {
        match conv {
            RetConv::Int { .. } if v.is_real => {
                let mut r = Self::real_to_int(v.to_f64(), if width > 0 { width } else { 64 });
                if let Some(sg) = signed {
                    r.is_signed = sg;
                }
                r
            }
            RetConv::Real if !v.is_real && !v.is_fill => Value::from_f64(v.to_f64()),
            _ => v,
        }
    }

    /// Width and signedness of a class method's declared return type that
    /// the method epilogue's own fits do not cover: an enum, a packed struct
    /// or a typedef (the class's own first). `(0, None)` otherwise.
    #[cold]
    #[inline(never)]
    pub(super) fn method_ret_typedef_fit(&self, rt: &DataType, cname: &str) -> (u32, Option<bool>) {
        let tt = &self.module.typedef_types;
        let dt = match rt {
            DataType::TypeReference {
                name, dimensions, ..
            } if dimensions.is_empty() => {
                let local = (!name.has_scope())
                    .then(|| self.class_local_typedef(cname, &name.name.name))
                    .flatten();
                match local {
                    Some(dt) => super::elaborate::resolve_typedef_chain(dt, tt),
                    None => super::elaborate::resolve_typedef_chain(rt, tt),
                }
            }
            DataType::Enum(_) | DataType::Struct(_) => rt,
            _ => return (0, None),
        };
        if matches!(dt, DataType::TypeReference { .. }) {
            return (0, None);
        }
        let w = resolve_type_width(
            dt,
            Some(&self.module.parameters),
            Some(&self.module.typedefs),
        );
        (w, Some(super::elaborate::is_type_signed_resolved(dt, tt)))
    }

    /// §13.4.1: a compiled method's declared return is integral (the method
    /// compiler admits no real return type), but its body can still produce
    /// a real, and a 64-bit `Resize` keeps a real as is: convert it by value,
    /// or a `longint` method returning `t / 1ns` handed back the f64 bits.
    /// A string result (width 0) keeps its value.
    #[cold]
    #[inline(never)]
    pub(super) fn compiled_ret_from_real(v: &Value, width: u32) -> Value {
        if width == 0 {
            return v.clone();
        }
        Self::real_to_int(v.to_f64(), width)
    }

    /// Whether `ret_convert` changes `v` (the cheap inline test).
    #[inline]
    pub(super) fn ret_needs_convert(v: &Value, conv: RetConv) -> bool {
        match conv {
            RetConv::Keep => false,
            RetConv::Real => !v.is_real && !v.is_fill,
            RetConv::Int { .. } => v.is_real,
        }
    }

    /// The 2-state half of the §13.4.1 return conversion: X/Z read as 0.
    #[inline]
    pub(super) fn ret_two_state(v: Value, conv: RetConv) -> Value {
        match conv {
            RetConv::Int { two_state: true } if v.has_xz() => v.to_two_state(),
            _ => v,
        }
    }

    /// The full `a::b::c` join of a pure `::`-chain expression (nested
    /// member accesses rooted at a plain identifier), or `None` when the
    /// expression is not a pure name chain. Unlike
    /// [`Self::named_cast_key`] there is NO suffix fallback — used where a
    /// partial match would capture a same-named local.
    /// §8.9/§8.25: the store of a class-scoped static array receiver of an
    /// element select — `Cls::fa` (a static fixed array: its declaring
    /// class's `Cls::fa`) or `alias_t::q` through a typedef of a
    /// specialization (that specialization's store, `C#int::q`). None for
    /// any other receiver.
    pub(super) fn class_scoped_elem_store(&mut self, e: &Expression) -> Option<String> {
        if self.class_member_names().static_or_param.is_empty() {
            return None;
        }
        let (head, member) = match &e.kind {
            ExprKind::Ident(h)
                if h.path.len() == 2
                    && h.root.is_none()
                    && h.path.iter().all(|s| s.selects.is_empty()) =>
            {
                (h.path[0].name.name.as_str(), h.path[1].name.name.as_str())
            }
            ExprKind::MemberAccess { expr, member } => match &expr.kind {
                ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                    (h.path[0].name.name.as_str(), member.name.as_str())
                }
                _ => return None,
            },
            _ => return None,
        };
        if !self.class_member_names().static_or_param.contains(member)
            || self
                .local_stack
                .last()
                .is_some_and(|l| l.contains_key(head))
            || self.signal_name_to_id.contains_key(head)
        {
            return None;
        }
        if self.module.classes.contains_key(head) {
            return self.static_fixed_key_in(head, member);
        }
        let (base, sig) = self.scope_prefix_spec(head)?;
        if let Some(k) = self.static_fixed_key_in(&base, member) {
            return Some(k);
        }
        if !self.member_is_static_coll(&base, member) {
            return None;
        }
        let saved = self.current_spec.replace((base.clone(), sig));
        let key = self.static_prop_key(&base, member);
        self.current_spec = saved;
        key
    }

    /// §8.25/§8.25.1: the specialization a class-scope prefix that is not a
    /// class name denotes — a type parameter or a typedef of a
    /// specialization (`alias_t::m`; see `resolve_class_scope_prefix`).
    /// None for any other name, after a one-probe name prefilter.
    pub(super) fn scope_prefix_spec(&self, head: &str) -> Option<(String, String)> {
        if !self.class_member_names().scope_prefixes.contains(head) {
            return None;
        }
        match self.resolve_class_scope_prefix(head)? {
            (base, Some(sig)) => Some((base, sig)),
            _ => None,
        }
    }

    /// The root name of a `a.b.c` / `a::b::c` member chain.
    pub(super) fn member_chain_root(mut e: &Expression) -> Option<&str> {
        loop {
            match &e.kind {
                ExprKind::MemberAccess { expr, .. } => e = expr,
                ExprKind::Ident(h) if h.path.len() == 1 => return Some(&h.path[0].name.name),
                _ => return None,
            }
        }
    }

    pub(super) fn expr_scoped_chain_key(e: &Expression) -> Option<String> {
        let mut links: Vec<String> = Vec::new();
        let mut cur = e;
        for _ in 0..16 {
            match &cur.kind {
                ExprKind::MemberAccess { expr, member } => {
                    links.push(member.name.clone());
                    cur = expr;
                }
                ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                    links.push(h.path[0].name.name.clone());
                    links.reverse();
                    return Some(links.join("::"));
                }
                _ => return None,
            }
        }
        None
    }

    /// The table key of a parser-lowered named cast's target (`T'(v)`,
    /// `pkg::T'(v)`, `pkg::cls::T'(v)`). A scoped name prefers its
    /// qualified registration (`pkg::T`, `pkg::cls::T`), so a same-named
    /// module-local typedef cannot capture it; a `::`-chained scope
    /// (§8.23) parses as nested member accesses and is joined the same
    /// way, trying the longest chain first and falling back to shorter
    /// suffixes then the bare leaf.
    pub(super) fn named_cast_key(&self, e: &Expression) -> Option<String> {
        // Flatten the (possibly nested) member-access chain into
        // [root, link..., leaf].
        let mut links: Vec<String> = Vec::new();
        let mut cur = e;
        loop {
            match &cur.kind {
                ExprKind::MemberAccess { expr, member } => {
                    links.push(member.name.clone());
                    cur = expr;
                }
                // `cls::N` may also arrive as one scoped identifier path.
                ExprKind::Ident(h) => {
                    links.extend(h.path.iter().rev().map(|sg| sg.name.name.clone()));
                    break;
                }
                _ => return None,
            }
        }
        links.reverse();
        let m = &self.module;
        // Try the longest join first (`pkg::cls::T`), then shorter
        // suffixes (`cls::T`), then the bare leaf — the old single-link
        // behavior is the len==2 case.
        for start in 0..links.len() {
            let key = links[start..].join("::");
            if m.typedef_types.contains_key(&key)
                || m.typedefs.contains_key(&key)
                || m.parameters.contains_key(&key)
            {
                return Some(key);
            }
        }
        links.last().cloned()
    }

    /// §6.18 / §6.24.3: a named scalar string target may be an inherited
    /// class alias, a bound type parameter, or a scoped package alias.
    pub(super) fn named_cast_is_string(&self, name: &str) -> bool {
        let classify = |dt: &DataType, scope: &str| {
            Self::is_string_data_type(dt)
                || (matches!(dt, DataType::TypeReference { .. })
                    && self.frame_formal_is_string_in(dt, scope))
        };
        let mut string = false;
        if !name.contains("::") {
            if let Some(bound) = self.resolve_type_param_binding(name) {
                string = bound == "string"
                    || self.module.typedef_types.get(&bound).is_some_and(|dt| {
                        classify(dt, bound.rsplit_once("::").map_or("", |(p, _)| p))
                    });
                return string && !self.type_is_unpacked_collection_name(name);
            }
            let mut class = self.class_context_stack.last().and_then(|c| c.as_deref());
            for _ in 0..64 {
                let Some(cd) = class.and_then(|c| self.module.classes.get(c)) else {
                    break;
                };
                if let Some(dt) = cd.typedef_targets.get(name) {
                    return classify(dt, &cd.name) && !self.type_is_unpacked_collection_name(name);
                }
                class = cd.extends.as_deref();
            }
        }
        if let Some(dt) = self.module.typedef_types.get(name) {
            string = classify(dt, name.rsplit_once("::").map_or("", |(p, _)| p));
        }
        string && !self.type_is_unpacked_collection_name(name)
    }

    /// The `1` that `++`/`--` add to or subtract from `v`, carrying `v`'s
    /// signedness: an unsigned step made the result unsigned, so `++x < 0`
    /// was false for `int x = -5`.
    pub(super) fn inc_dec_step(v: &Value) -> Value {
        let mut one = Value::from_u64(1, v.width);
        one.is_signed = v.is_signed;
        one
    }

    /// §6.24.1: the operand context width a cast to `dt` imposes — the
    /// type's width for an integral target (keyword, enum, packed struct, or
    /// a typedef chain ending in one), 0 for any other target (real, string,
    /// unpacked aggregate, class), whose operand stays self-determined.
    /// True when the typedef named `nm` (following alias chains) carries an
    /// unpacked dimension — a DYNAMIC queue `[$]`, unsized `[]`, or associative
    /// collection. Unlike `type_is_unpacked_collection`, `nm` is the typedef
    /// NAME, because `module.typedef_types` resolves a queue typedef down to its
    /// base integral element type (dropping the `[$]`), leaving the unpacked
    /// dims reachable only through `typedef_unpacked_dims`.
    pub(super) fn type_is_unpacked_collection_name(&self, nm: &str) -> bool {
        let mut cur = nm;
        for _ in 0..16 {
            if self.module.typedef_unpacked_dims.contains_key(cur) {
                return true;
            }
            match self.module.typedef_types.get(cur) {
                Some(crate::ast::types::DataType::TypeReference { name: n, .. })
                    if n.name.name != *cur =>
                {
                    cur = n.name.name.as_str();
                }
                _ => return false,
            }
        }
        false
    }

    /// True when the cast target type `dt` is an UNPACKED ARRAY / QUEUE
    /// (a dynamic `[$]`, unsized `[]`, or associative collection) rather than
    /// a packed integral type. A type cast such as `bit_q_t'(stream)` to such
    /// a target must preserve every element/bit — the result is unpacked, so
    /// collapsing it to the typedef's packed element width would throw away
    /// all but one element (see the uvm_reg_map byte-shift `bit_q_t'({<<{bits}})`).
    /// Follows typedef aliases; `cur.name` is resolved to its concrete form so
    /// `typedef bit_q_t2 bit_q_t` chains resolve too.
    pub(super) fn type_is_unpacked_collection(&self, dt: &crate::ast::types::DataType) -> bool {
        use crate::ast::types::DataType;
        let mut cur = dt;
        for _ in 0..16 {
            match cur {
                // A typedef reference: unpacked dims live on the typedef table.
                DataType::TypeReference { name, .. } => {
                    let key = name.name.name.as_str();
                    if self.module.typedef_unpacked_dims.contains_key(key) {
                        return true;
                    }
                    match self.module.typedef_types.get(key) {
                        Some(DataType::TypeReference { name: n, .. }) if n.name.name == *key => {
                            // Self-referential alias: not a collection.
                            return false;
                        }
                        Some(next) => {
                            cur = next;
                        }
                        None => return false,
                    }
                }
                _ => return false,
            }
        }
        false
    }

    pub(super) fn cast_context_width(&self, dt: &crate::ast::types::DataType) -> u32 {
        use crate::ast::types::DataType;
        let mut cur = dt;
        for _ in 0..16 {
            match cur {
                DataType::IntegerAtom { .. } | DataType::IntegerVector { .. } => break,
                DataType::Enum(_) => break,
                DataType::Struct(su) if su.packed => break,
                DataType::TypeReference { name, .. } => {
                    let key = &name.name.name;
                    match self.module.typedef_types.get(key) {
                        Some(next)
                            if !matches!(next, DataType::TypeReference { name: n, .. }
                                if &n.name.name == key) =>
                        {
                            cur = next
                        }
                        _ if self.module.typedefs.contains_key(key) => break,
                        _ => return 0,
                    }
                }
                _ => return 0,
            }
        }
        super::elaborate::resolve_type_width(
            dt,
            Some(&self.module.parameters),
            Some(&self.module.typedefs),
        )
    }

    /// Bind an ASSOCIATIVE-array actual to a formal: copy the caller's entries to
    /// `<formal>[key]` and register the formal so `.size()`/`.exists()`/indexing
    /// work inside the body. Returns `(formal, actual)`.
    ///
    /// Only tasks did this; functions bound nothing, so `arr.size()` on an assoc
    /// formal read 0.
    /// Is this formal an associative array — either via a direct unpacked
    /// `[key]` dimension (`int aa[int]`) or through an associative typedef
    /// (`typedef int edges_t[KeyType]; … ref edges_t s`), where the dim
    /// lives on the typedef rather than the port?
    pub(super) fn port_is_assoc_array(&self, port: &crate::ast::decl::FunctionPort) -> bool {
        use crate::ast::types::UnpackedDimension as UD;
        if port
            .dimensions
            .iter()
            .any(|d| matches!(d, UD::Associative { .. }))
        {
            return true;
        }
        if let crate::ast::types::DataType::TypeReference { name, .. } = &port.data_type {
            let tn = &name.name.name;
            let concrete = self
                .resolve_type_param_binding(tn)
                .unwrap_or_else(|| tn.clone());
            // A class type is never an associative array.
            if name.scopes.is_empty() && self.module.classes.contains_key(&concrete) {
                return false;
            }
            let scoped = (!name.scopes.is_empty())
                .then(|| self.module.typedef_unpacked_dims.get(&name.qualified()))
                .flatten();
            if let Some(dims) = scoped.or_else(|| self.module.typedef_unpacked_dims.get(&concrete))
            {
                return dims.iter().any(|d| matches!(d, UD::Associative { .. }));
            }
            // CLASS-LOCAL typedef (`typedef bit edges_t[Node];` inside the
            // class): the dim lives on the
            // class's own typedef table, not the module's. Walk the current
            // class context and its ancestors.
            let mut cur = self
                .class_context_stack
                .last()
                .cloned()
                .flatten()
                .or_else(|| {
                    self.this_stack.last().copied().flatten().and_then(|h| {
                        self.heap
                            .get(h)
                            .and_then(|x| x.as_ref())
                            .map(|i| i.class_name.clone())
                    })
                });
            while let Some(cn) = cur {
                if let Some(cd) = self.module.classes.get(&cn) {
                    if let Some(dims) = cd.typedef_unpacked_dims.get(&concrete) {
                        return dims.iter().any(|d| matches!(d, UD::Associative { .. }));
                    }
                    cur = cd.extends.clone();
                } else {
                    break;
                }
            }
            return false;
        }
        false
    }

    /// A formal no collection, struct, array or associative binder claims:
    /// no unpacked dimensions and a built-in integral, real or string type.
    pub(super) fn port_is_plain_scalar(port: &crate::ast::decl::FunctionPort) -> bool {
        port.dimensions.is_empty()
            && matches!(
                port.data_type,
                DataType::IntegerAtom { .. }
                    | DataType::IntegerVector { .. }
                    | DataType::Real { .. }
                    | DataType::Simple {
                        kind: crate::ast::types::SimpleType::String,
                        ..
                    }
            )
    }

    /// A declared type with no packed-array, struct, enum or typedef layout
    /// to register: an integral atom, a vector with at most one packed
    /// dimension, `real`/`shortreal`/`realtime` or `string`.
    pub(super) fn type_is_plain_scalar(dt: &DataType) -> bool {
        match dt {
            DataType::IntegerAtom { .. } | DataType::Real { .. } => true,
            DataType::IntegerVector { dimensions, .. } => dimensions.len() <= 1,
            DataType::Simple { kind, .. } => {
                matches!(kind, crate::ast::types::SimpleType::String)
            }
            _ => false,
        }
    }

    /// `plain_class_ref` of a formal with no unpacked dimensions. The
    /// collection, struct and array binders, the integral width fitting and
    /// the vif probes all come back empty for such a formal, so it binds
    /// exactly like a scalar.
    pub(super) fn port_class_ref(&self, port: &crate::ast::decl::FunctionPort) -> Option<bool> {
        if !port.dimensions.is_empty() {
            return None;
        }
        self.plain_class_ref(&port.data_type)
    }

    /// An unscoped, dimension-free reference to a plain CLASS -- a class name
    /// that no typedef (module or class-local), enum or class parameter
    /// shares, so every typedef, type-parameter and layout lookup on it
    /// misses and it resolves to the class itself: `Some(false)`.
    /// `Some(true)` when the name is also a forward `typedef class`
    /// placeholder (a `Void` typedef), which answers every layout probe the
    /// same way but registers formals and locals as typedef-typed too. The
    /// name table is built from the class and typedef tables and loses a
    /// name when a procedural `typedef` or an inline-enum local adds it.
    pub(super) fn plain_class_ref(&self, dt: &DataType) -> Option<bool> {
        let DataType::TypeReference {
            name, dimensions, ..
        } = dt
        else {
            return None;
        };
        if !dimensions.is_empty() || name.has_scope() {
            return None;
        }
        let mut cell = self.plain_class_types.borrow_mut();
        let kinds = cell.get_or_insert_with(|| {
            let params = &self.class_member_names().params;
            let mut local_typedefs: HashSet<&str> = HashSet::default();
            for cd in self.module.classes.values() {
                local_typedefs.extend(cd.typedef_names.iter().map(String::as_str));
                local_typedefs.extend(cd.typedef_targets.keys().map(String::as_str));
                local_typedefs.extend(cd.typedef_unpacked_dims.keys().map(String::as_str));
            }
            self.module
                .classes
                .keys()
                .filter_map(|n| {
                    let n = n.as_str();
                    if params.contains(n)
                        || local_typedefs.contains(n)
                        || self.module.typedef_unpacked_dims.contains_key(n)
                        || self.module.typedef_elem_widths.contains_key(n)
                        || self.module.enum_members.contains_key(n)
                    {
                        return None;
                    }
                    let forward = match (
                        self.module.typedef_types.get(n),
                        self.module.typedefs.contains_key(n),
                    ) {
                        (None, false) => false,
                        (Some(DataType::Void(_)), true) => true,
                        _ => return None,
                    };
                    Some((n.to_string(), forward))
                })
                .collect()
        });
        kinds.get(name.name.name.as_str()).copied()
    }

    /// The `k`-th name whose metadata a class-method call saves (ports in
    /// order, then the return variable), or `None` when it repeats an
    /// earlier one or `k` is past the end.
    pub(super) fn formal_meta_name<'a>(
        ports: &'a [crate::ast::decl::FunctionPort],
        ret: Option<&'a str>,
        k: usize,
    ) -> Option<&'a str> {
        let name = match ports.get(k) {
            Some(p) => p.name.name.as_str(),
            None if k == ports.len() => ret?,
            None => return None,
        };
        let earlier = &ports[..k.min(ports.len())];
        (!earlier.iter().any(|p| p.name.name == name)).then_some(name)
    }

    /// §7.2: every storage LEAF of an unpacked struct, as
    /// `(key_suffix, caller_expr, width, is_real)`. Recurses into nested
    /// unpacked structs and expands fixed member dimensions, matching the
    /// traversal `copy_unpacked_struct` uses — a struct's storage is its leaves,
    /// and anything that seeds, binds or copies one has to walk all of them.
    /// `base` is the expression the suffixes hang off; pass the actual argument
    /// to get caller-side expressions, or the formal's own name when only the
    /// keys matter.
    pub(super) fn unpacked_struct_leaves(
        &mut self,
        key_prefix: &str,
        base: &Expression,
        su: &crate::ast::types::StructUnionType,
        depth: u32,
        out: &mut Vec<(String, Expression, u32, bool)>,
    ) {
        if depth > 8 {
            return;
        }
        let mk_member = |b: &Expression, name: &str| {
            Expression::new(
                ExprKind::MemberAccess {
                    expr: Box::new(b.clone()),
                    member: crate::ast::Identifier {
                        name: name.to_string(),
                        span: b.span,
                    },
                },
                b.span,
            )
        };
        for m in &su.members {
            let mw = super::elaborate::resolve_type_width(
                &m.data_type,
                Some(&self.module.parameters),
                Some(&self.module.typedefs),
            );
            let is_real = super::elaborate::is_type_real(&m.data_type);
            let nested = self.unpacked_struct_of(&m.data_type);
            for md in &m.declarators {
                // §7.5/§7.10: a dynamic / queue / associative member is a
                // collection of its own (`struct_coll_members`), not a leaf.
                if Self::has_coll_dim(&md.dimensions) {
                    continue;
                }
                let mkey = format!("{}.{}", key_prefix, md.name.name);
                let mexpr = mk_member(base, &md.name.name);
                match self.member_elem_indices(&md.dimensions) {
                    Some(tups) => {
                        // Every element, last dimension varying fastest —
                        // `m[2][3]` addresses elements `m[i][j]`.
                        for tup in tups {
                            let mut ekey = mkey.clone();
                            let mut eexpr = mexpr.clone();
                            for i in tup {
                                ekey.push_str(&format!("[{}]", i));
                                eexpr = Expression::new(
                                    ExprKind::Index {
                                        expr: Box::new(eexpr),
                                        index: Box::new(Expression::new(
                                            ExprKind::Number(NumberLiteral::Integer {
                                                size: None,
                                                signed: true,
                                                base: NumberBase::Decimal,
                                                value: i.to_string(),
                                                cached_val: std::cell::Cell::new(None),
                                            }),
                                            base.span,
                                        )),
                                    },
                                    base.span,
                                );
                            }
                            match &nested {
                                Some(inner) => self.unpacked_struct_leaves(
                                    &ekey,
                                    &eexpr,
                                    &inner.clone(),
                                    depth + 1,
                                    out,
                                ),
                                None => out.push((ekey, eexpr, mw, is_real)),
                            }
                        }
                    }
                    None => match &nested {
                        Some(inner) => self.unpacked_struct_leaves(
                            &mkey,
                            &mexpr,
                            &inner.clone(),
                            depth + 1,
                            out,
                        ),
                        None => out.push((mkey, mexpr, mw, is_real)),
                    },
                }
            }
        }
    }

    /// Leaf keys of an unpacked struct with each leaf's §6.8 default value
    /// (0 for a 2-state member, 0.0 for a real one, x otherwise) and type.
    pub(super) fn unpacked_struct_leaf_defaults(
        &mut self,
        key_prefix: &str,
        su: &crate::ast::types::StructUnionType,
        depth: u32,
        out: &mut Vec<(String, Value, DataType)>,
    ) {
        if depth > 8 {
            return;
        }
        for m in &su.members {
            let mw = super::elaborate::resolve_type_width(
                &m.data_type,
                Some(&self.module.parameters),
                Some(&self.module.typedefs),
            );
            let mut dv = if super::elaborate::is_type_real(&m.data_type) {
                Value::from_f64(0.0)
            } else if super::elaborate::is_type_two_state_resolved(
                &m.data_type,
                &self.module.typedef_types,
            ) {
                Value::zero(mw)
            } else {
                Value::new(mw)
            };
            // §7.2: the leaf has the member's declared type, signedness
            // included, which every later store keeps (§10.7).
            if !dv.is_real {
                dv.is_signed = self.type_is_signed_concrete(&m.data_type);
            }
            let nested = self.unpacked_struct_of(&m.data_type);
            for md in &m.declarators {
                // Same keys as `unpacked_struct_leaves`: a collection member
                // has no leaf.
                if Self::has_coll_dim(&md.dimensions) {
                    continue;
                }
                let mkey = format!("{}.{}", key_prefix, md.name.name);
                let keys = match self.member_elem_indices(&md.dimensions) {
                    Some(tups) => tups
                        .iter()
                        .map(|t| format!("{}{}", mkey, Self::index_suffix(t)))
                        .collect(),
                    None => vec![mkey],
                };
                // §7.2.2: a member's declared default (`int a = 3;`) is the
                // leaf's initial value, for a block-local as for a module
                // variable (whose defaults run as elaboration-time
                // assignments). Scalar members only; the default is an
                // assignment to the member's type.
                let member_default = match (&md.init, &nested) {
                    (Some(init), None) if md.dimensions.is_empty() => {
                        let raw = self.eval_expr_ctx(init, mw.max(1));
                        let two_state = super::elaborate::is_type_two_state_resolved(
                            &m.data_type,
                            &self.module.typedef_types,
                        );
                        let is_real = super::elaborate::is_type_real(&m.data_type);
                        let signed = self.type_is_signed_concrete(&m.data_type);
                        Some(Self::fit_local_decl_init(
                            raw,
                            mw.max(1),
                            &m.data_type,
                            two_state,
                            is_real,
                            signed,
                        ))
                    }
                    _ => None,
                };
                for k in keys {
                    match &nested {
                        Some(inner) => {
                            self.unpacked_struct_leaf_defaults(&k, &inner.clone(), depth + 1, out)
                        }
                        None => out.push((
                            k,
                            member_default.clone().unwrap_or_else(|| dv.clone()),
                            m.data_type.clone(),
                        )),
                    }
                }
            }
        }
    }

    /// A packed-struct member leaf (`s.p` of `struct { pk_t p; } s`) aliases
    /// its fields into the leaf, as a packed-struct variable does.
    pub(super) fn register_packed_leaf_layout(&mut self, key: &str, dt: &DataType) {
        self.meta_note(key);
        if let Some(fields) = super::elaborate::packed_struct_field_layout(
            dt,
            &self.module.parameters,
            &self.module.typedefs,
            &self.module.typedef_types,
        ) {
            if !fields.is_empty() {
                self.module
                    .packed_struct_fields
                    .insert(key.to_string(), fields);
            }
        }
    }

    /// Leaf keys only, for seeding a frame where there is no caller expression.
    pub(super) fn unpacked_struct_leaf_keys(
        &mut self,
        base_name: &str,
        su: &crate::ast::types::StructUnionType,
    ) -> Vec<(String, u32, bool)> {
        let span = crate::ast::Span::dummy();
        let base = Expression::new(
            ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                root: None,
                path: vec![crate::ast::expr::HierPathSegment {
                    name: crate::ast::Identifier {
                        name: base_name.to_string(),
                        span,
                    },
                    selects: Vec::new(),
                }],
                span,
                cached_signal_id: std::cell::Cell::new(None),
                cached_resolved_name: std::cell::OnceCell::new(),
            }),
            span,
        );
        let mut out = Vec::new();
        self.unpacked_struct_leaves(base_name, &base, su, 0, &mut out);
        // The `<base>.` marker (no real leaf can be named that) lets the read
        // and write paths reject an unrelated expression with one hash lookup,
        // before flattening a name.
        self.any_struct_formal_markers = true;
        let mut keys: Vec<(String, u32, bool)> = vec![(format!("{}.", base_name), 0, false)];
        keys.extend(out.into_iter().map(|(k, _, w, r)| (k, w, r)));
        keys
    }

    /// §7.2: collapse an unpacked struct's member leaves into one packed value,
    /// laid out exactly like the packed form of the same type. Needed wherever
    /// a struct has to travel as a VALUE rather than as a set of named leaves —
    /// a function return being the case that matters: the callee's frame is
    /// gone by the time the caller assigns the result, so a name-to-name member
    /// copy is impossible.
    pub(super) fn pack_unpacked_struct(
        &mut self,
        base: &str,
        su: &crate::ast::types::StructUnionType,
    ) -> Option<Value> {
        let fields = self.struct_leaf_layout(base, su);
        let total = fields.iter().map(|(_, o, w, _)| o + w).max().unwrap_or(0);
        if total == 0 {
            return None;
        }
        let mut out = Value::new(total);
        let mut any = false;
        for (key, off, w, _is_real) in fields {
            let Some(v) = self.get_signal_value_by_name(&key) else {
                continue;
            };
            any = true;
            Self::copy_leaf_bits(&mut out, off, &v, 0, w);
        }
        any.then_some(out)
    }

    /// `dst[dst_off +: w] = src[src_off +: w]`, a bit past `src` reading 0
    /// (or the fill bit), as a per-bit copy would — but word by word: a
    /// struct with a 2-D packed member (`bit [256:0][511:0] wdata`) is about
    /// 150k bits, packed and spread on every call that passes it by value.
    pub(super) fn copy_leaf_bits(dst: &mut Value, dst_off: u32, src: &Value, src_off: u32, w: u32) {
        let n = if src.is_fill {
            0
        } else {
            w.min(src.width.saturating_sub(src_off))
        };
        if n > 0 {
            dst.copy_bits_from(dst_off as usize, src, src_off as usize, n as usize);
        }
        for i in n..w {
            dst.set_bit((dst_off + i) as usize, src.get_bit((src_off + i) as usize));
        }
    }

    /// Bit layout over an unpacked struct's LEAVES, as `(key, offset, width,
    /// is_real)`. Reverse declaration order with ascending offsets, matching
    /// `struct_field_layout` for a flat struct, and extended to nested members
    /// and member dimensions — which have no field of their own to lay out.
    pub(super) fn struct_leaf_layout(
        &mut self,
        base: &str,
        su: &crate::ast::types::StructUnionType,
    ) -> Vec<(String, u32, u32, bool)> {
        let mut leaves = self.unpacked_struct_leaf_keys(base, su);
        leaves.retain(|(k, _, _)| !k.ends_with('.'));
        let mut out = Vec::with_capacity(leaves.len());
        let mut off = 0u32;
        for (k, w, is_real) in leaves.into_iter().rev() {
            out.push((k, off, w, is_real));
            off += w;
        }
        out
    }

    /// §7.2/§18.4: collapse an unpacked-struct class property's per-instance
    /// cells into one packed value, laid out like `pack_unpacked_struct` so the
    /// two are interchangeable at a call boundary.
    pub(super) fn pack_class_unpacked(
        &mut self,
        handle: usize,
        prop: &str,
        su: &crate::ast::types::StructUnionType,
    ) -> Option<Value> {
        let fields = self.struct_leaf_layout("X", su);
        let total = fields.iter().map(|(_, o, w, _)| o + w).max().unwrap_or(0);
        if total == 0 {
            return None;
        }
        let mut out = Value::new(total);
        let mut any = false;
        for (key, off, w, _is_real) in fields {
            let cell = format!("{}{}", prop, &key[1..]);
            let Some(v) = self
                .heap
                .get(handle)
                .and_then(|o| o.as_ref())
                .and_then(|i| i.properties.get(&cell))
                .cloned()
            else {
                continue;
            };
            any = true;
            Self::copy_leaf_bits(&mut out, off, &v, 0, w);
        }
        any.then_some(out)
    }

    /// The inverse: scatter a packed value across an unpacked struct's leaves.
    pub(super) fn spread_into_unpacked_struct(
        &mut self,
        dst: &str,
        su: &crate::ast::types::StructUnionType,
        v: &Value,
    ) -> bool {
        let fields = self.struct_leaf_layout(dst, su);
        if fields.is_empty() {
            return false;
        }
        for (leaf, off, w, is_real) in fields {
            let mut mv = Value::new(w);
            Self::copy_leaf_bits(&mut mv, 0, v, off, w);
            if is_real {
                // `Value::from_f64` stores `f.to_bits()`, and packing copied
                // those raw IEEE-754 bits into the aggregate — so recovering
                // the member is `from_bits`, not an integer cast. The cast
                // reinterpreted the bit pattern as an integer, so a struct
                // with a `real` member assigned WHOLE (function return,
                // assignment pattern) read 4.0 back as 4.6161896e18.
                let f = f64::from_bits(mv.to_u64().unwrap_or(0));
                self.write_leaf_by_name(&leaf, Value::from_f64(f));
            } else {
                self.write_leaf_by_name(&leaf, mv);
            }
        }
        true
    }

    /// The leaves of a packed struct VALUE `v` (laid out as
    /// `pack_unpacked_struct` lays out `su`), keyed as `struct_leaf_layout`
    /// keys them under `base`.
    pub(super) fn split_packed_struct(
        &mut self,
        base: &str,
        su: &crate::ast::types::StructUnionType,
        v: &Value,
    ) -> Vec<(String, Value)> {
        let fields = self.struct_leaf_layout(base, su);
        let mut out = Vec::with_capacity(fields.len());
        for (leaf, off, w, is_real) in fields {
            let mut mv = Value::new(w);
            Self::copy_leaf_bits(&mut mv, 0, v, off, w);
            if is_real {
                mv = Value::from_f64(f64::from_bits(mv.to_u64().unwrap_or(0)));
            }
            out.push((leaf, mv));
        }
        out
    }

    /// §13.5.1/§7.2: an unpacked-struct actual that is a CALL (`f()`,
    /// `q.pop_front()`) is one value: evaluate it ONCE and hand out its
    /// leaves by their keys under `base`. Reading `arg.member` per member
    /// re-ran the call for every member — a `pop_front()` actual popped once
    /// per member and bound nothing.
    pub(super) fn call_struct_leaf_values(
        &mut self,
        base: &str,
        su: &crate::ast::types::StructUnionType,
        arg: &Expression,
    ) -> Option<HashMap<String, Value>> {
        let mut e = arg;
        while let ExprKind::Paren(i) = &e.kind {
            e = i;
        }
        if !matches!(e.kind, ExprKind::Call { .. }) {
            return None;
        }
        // Only this call's `return` may leave dynamic members staged.
        self.pending_ret_struct = None;
        let v = self.eval_expr(e);
        Some(self.split_packed_struct(base, su, &v).into_iter().collect())
    }

    /// The lvalue naming leaf `tail` (`.a`, `.inner.x`, `.arr[2].y`, as
    /// `struct_leaf_layout` spells a key after its base) under `base`.
    pub(super) fn leaf_lvalue_of(base: &Expression, tail: &str) -> Option<Expression> {
        let mut lv = base.clone();
        for seg in tail.strip_prefix('.')?.split('.') {
            match seg.split_once('[') {
                Some((name, rest)) => {
                    let mut idx = Vec::new();
                    for part in rest.split('[') {
                        idx.push(part.trim_end_matches(']').parse::<i64>().ok()?);
                    }
                    lv = Self::append_elem_path(&lv, name, &idx);
                }
                None => lv = Self::append_member_expr(&lv, seg),
            }
        }
        Some(lv)
    }

    /// The resolved struct type when `dt` is an UNPACKED struct whose members
    /// are stored one-per-leaf (§7.2), else `None`.
    pub(super) fn unpacked_struct_of(
        &self,
        dt: &DataType,
    ) -> Option<crate::ast::types::StructUnionType> {
        match self.resolve_dt_ref(dt) {
            DataType::Struct(su) if Self::spreads_member_wise(su) => Some(su.clone()),
            _ => None,
        }
    }

    /// §13.5.2: bind an UNPACKED-struct formal member-wise. Inputs copy the
    /// caller's members in; a pure output starts its members unwritten. Returns
    /// the `(frame_key, caller_member_expr)` pairs an out/inout/ref formal must
    /// copy back on return, which ride the ordinary output-binding path.
    pub(super) fn bind_unpacked_struct_formal(
        &mut self,
        port_name: &str,
        dt: &DataType,
        dir: PortDirection,
        arg: &Expression,
        locals: &mut HashMap<String, Value>,
    ) -> Option<Vec<(String, Expression)>> {
        let su = self.unpacked_struct_of(dt)?;
        // §11.4.11: a conditional actual binds its selected operand.
        let arg = self.struct_cond_operand(arg).unwrap_or(arg);
        let copies_in = matches!(
            dir,
            PortDirection::Input | PortDirection::Inout | PortDirection::Ref
        );
        let copies_out = matches!(
            dir,
            PortDirection::Output | PortDirection::Inout | PortDirection::Ref
        );
        // Walk LEAVES, not top-level members: a member that is itself an
        // unpacked struct, or that carries an unpacked dimension, has no
        // storage of its own — only the leaves beneath it do. Binding only the
        // top level left `p.inner.x` and `p.arr[i]` unbound in the callee.
        let mut leaves = Vec::new();
        self.unpacked_struct_leaves(port_name, arg, &su, 0, &mut leaves);
        self.any_struct_formal_markers = true;
        self.note_struct_root(port_name);
        locals.insert(format!("{}.", port_name), Value::zero(1));
        // A positional STRUCT pattern literal actual (`'{a, b}`) can't be read
        // member-by-member (`.a` on a pattern returns x), so bind its evaluated
        // parts positionally to the flat leaves when the shape matches.
        let leaf_count = leaves
            .iter()
            .filter(|(_, ce, _, _)| {
                matches!(
                    ce.kind,
                    ExprKind::MemberAccess { .. } | ExprKind::Index { .. }
                )
            })
            .count();
        let pattern_vals = self.eval_flat_struct_pattern(arg, leaf_count);
        // §10.9.2: any other pattern actual — one with struct-valued or
        // dynamic members — is assigned to a staging struct, which then binds
        // like a struct variable actual.
        let pattern_tmp = if pattern_vals.is_none() && matches!(dir, PortDirection::Input) {
            self.stage_struct_pattern_arg(port_name, &su, arg)
        } else {
            None
        };
        let call_vals = if copies_in && pattern_vals.is_none() && pattern_tmp.is_none() {
            self.call_struct_leaf_values(port_name, &su, arg)
        } else {
            None
        };
        // §13.5/§7.2.1: the formal's DYNAMIC members are collections of its
        // own (`v.q`): fresh per call, copied in from the actual's members
        // for input/inout/ref, and copied back on return for
        // output/inout/ref (queue / dynamic kinds, through the call's
        // queue writebacks).
        // An actual that IS the same-named variable (a formal handed on to a
        // callee whose formal has its name) shares the storage for
        // output/inout/ref and keeps its contents for an input.
        // §13.4.1: an actual that is a CALL returning the struct hands its
        // dynamic members over through the return staging
        // (`stage_ret_struct_colls`), as `dst = f()` receives them.
        // The formal's member is named like a caller variable's (`x.q`), so
        // the actual's contents are staged and installed only once every
        // actual is evaluated (`install_struct_formal_colls`): installing
        // here would clobber a caller variable a later actual still reads.
        if self.struct_colls_possible() {
            let caller_base = pattern_tmp.clone().or_else(|| self.flat_member_name(arg));
            let ret_staged = if caller_base.is_none() && call_vals.is_some() {
                self.pending_ret_struct.take()
            } else {
                None
            };
            let stage_root = format!(
                "__xz_arg_struct__{}.{}",
                self.queue_frame_saves.len(),
                port_name
            );
            for (sfx, kind, dt, sk) in self.struct_coll_members(&su) {
                let local = format!("{}.{}", port_name, sfx);
                let caller = caller_base
                    .as_ref()
                    .map(|b| format!("{}.{}", b, sfx))
                    .filter(|c| self.ensure_struct_member_coll(c));
                let same = caller.as_deref() == Some(local.as_str());
                if same {
                    if !copies_out {
                        self.snapshot_queue_local(&local);
                    }
                    self.register_struct_member_coll_as(&local, kind, &dt, sk);
                    if !copies_in {
                        self.clear_coll(&local, kind);
                    }
                    continue;
                }
                let source = caller.clone().or_else(|| {
                    ret_staged
                        .as_ref()
                        .map(|r| format!("{}.{}", r, sfx))
                        .filter(|r| {
                            self.module.dynamic_arrays.contains(r)
                                || self.module.associative_arrays.contains_key(r)
                        })
                });
                let staged = match source {
                    Some(src) if copies_in => {
                        let tmp = format!("{}.{}", stage_root, sfx);
                        self.register_struct_member_coll_as(&tmp, kind, &dt, sk);
                        self.clear_coll(&tmp, kind);
                        self.copy_coll_typed(&tmp, &src, kind, &dt);
                        Some(tmp)
                    }
                    _ => None,
                };
                self.pending_struct_coll_in
                    .push((local.clone(), kind, dt, sk, staged));
                if let Some(caller) = caller
                    && copies_out
                    && kind != StructCollKind::Assoc
                {
                    self.pending_struct_coll_wb.push((local, caller));
                }
            }
        }
        // §13.5/§7.2: each formal leaf has its member's declared signedness,
        // whatever the actual's value carries (a call's packed result is an
        // unsigned bit pattern).
        let mut decl = Vec::new();
        self.unpacked_struct_leaf_defaults(port_name, &su, 0, &mut decl);
        let signed_leaves: HashSet<String> = decl
            .into_iter()
            .filter(|(_, dv, _)| dv.is_signed)
            .map(|(k, ..)| k)
            .collect();
        let mut leaf_pos = 0usize;
        let mut backs = Vec::new();
        for (key, caller_expr, w, is_real) in leaves {
            if copies_in {
                let mut v = if let Some(pv) = &pattern_vals {
                    pv[leaf_pos].clone()
                } else if let Some(cv) = &call_vals {
                    cv.get(&key).cloned().unwrap_or_else(|| Value::new(w))
                } else if let Some(t) = &pattern_tmp {
                    self.get_signal_value_by_name(&format!("{}{}", t, &key[port_name.len()..]))
                        .unwrap_or_else(|| Value::new(w))
                } else {
                    self.eval_expr(&caller_expr)
                };
                if !v.is_real {
                    v.is_signed = signed_leaves.contains(&key);
                }
                locals.insert(key.clone(), v);
                leaf_pos += 1;
            } else {
                let mut seed = if is_real {
                    Value::from_f64(0.0)
                } else {
                    Value::new(w)
                };
                if !seed.is_real {
                    seed.is_signed = signed_leaves.contains(&key);
                }
                locals.insert(key.clone(), seed);
            }
            if copies_out {
                backs.push((key, caller_expr));
            }
        }
        Some(backs)
    }

    /// §10.9.2/§13.5.1: assign the assignment-pattern actual `arg` of the
    /// unpacked-struct input formal `port_name` to a fresh staging struct —
    /// leaves at their declared defaults, dynamic members empty — so its
    /// struct-valued items (variables, calls) bring their dynamic members
    /// along. Returns the staging struct's name; `None` for a non-pattern.
    #[cold]
    #[inline(never)]
    pub(super) fn stage_struct_pattern_arg(
        &mut self,
        port_name: &str,
        su: &crate::ast::types::StructUnionType,
        arg: &Expression,
    ) -> Option<String> {
        let ExprKind::AssignmentPattern(items) = &arg.kind else {
            return None;
        };
        if items.is_empty() {
            return None;
        }
        let tmp = format!(
            "__xz_arg_pat_{}_{}",
            self.queue_frame_saves.len(),
            port_name
        );
        let mut leaves = Vec::new();
        self.unpacked_struct_leaf_defaults(&tmp, su, 0, &mut leaves);
        for (k, dv, _) in leaves {
            if dv.is_signed {
                self.signed_signals.insert(k.clone());
            } else {
                self.signed_signals.remove(&k);
            }
            self.signals.insert(k, dv);
        }
        for (sfx, kind, dt, sk) in self.struct_coll_members(su) {
            let d = format!("{}.{}", tmp, sfx);
            self.register_struct_member_coll_as(&d, kind, &dt, sk);
            self.clear_coll(&d, kind);
        }
        self.assign_pattern_into_struct(&tmp, su, items);
        Some(tmp)
    }

    /// §13.5: install the dynamic members of the struct formals bound since
    /// `base` (see `bind_unpacked_struct_formal`) — each fresh for the call,
    /// holding the staged copy of its actual's member when it copies in. Run
    /// once all of the call's actuals are evaluated; the caller's same-named
    /// storage is snapshotted for restore when the callee returns.
    pub(super) fn install_struct_formal_colls(&mut self, base: usize) {
        if self.pending_struct_coll_in.len() <= base {
            return;
        }
        let pending: Vec<_> = self.pending_struct_coll_in.drain(base..).collect();
        for (local, kind, dt, sk, staged) in pending {
            self.snapshot_queue_local(&local);
            self.register_struct_member_coll_as(&local, kind, &dt, sk);
            self.clear_coll(&local, kind);
            if let Some(tmp) = staged {
                self.copy_coll_typed(&local, &tmp, kind, &dt);
                self.clear_coll(&tmp, kind);
            }
        }
    }

    /// The frame key for a struct-member reference of ANY depth — `s.a`,
    /// `s.inner.x`, `s.arr[2]`, `s.arr[2].x` — or `None` when the innermost
    /// frame does not own that leaf. Keyed on the flattened name, because a
    /// nested or indexed member is not a single `base.member` pair: matching
    /// only that shape left every deeper leaf reading module scope.
    pub(super) fn frame_leaf_key_of(&mut self, e: &Expression) -> Option<String> {
        // Sticky gate: `<base>.` markers are created in exactly two places —
        // `bind_unpacked_struct_formal` and `unpacked_struct_leaf_keys` (the
        // struct-return path) — and both set this flag. A design that never
        // touches unpacked-struct formals/returns (all of UVM) skips the
        // root walk and marker-string allocation on every member/index eval.
        if !self.any_struct_formal_markers {
            return None;
        }
        if !matches!(
            e.kind,
            ExprKind::MemberAccess { .. } | ExprKind::Index { .. }
        ) {
            return None;
        }
        // Cheap gate before the (index-evaluating) flatten: the frame records a
        // `<base>.` marker for each unpacked-struct variable it owns, so an
        // expression rooted anywhere else costs one hash lookup.
        let root = Self::expr_root_ident(e)?;
        // `"<root>."` marker probe without allocating: reuse one buffer.
        let mut marker = std::mem::take(&mut self.marker_key_scratch);
        marker.clear();
        marker.push_str(root);
        marker.push('.');
        let has = self
            .local_stack
            .last()
            .is_some_and(|l| l.contains_key(marker.as_str()));
        self.marker_key_scratch = marker;
        if !has {
            return None;
        }
        let flat = self.flat_member_name(e)?;
        self.local_stack
            .last()
            .is_some_and(|l| l.contains_key(&flat))
            .then_some(flat)
    }

    /// IEEE 1800-2017 §7.2/§7.4.2: the stored leaf that `base[index]` names
    /// when `base` is itself an element select below a struct member —
    /// `s.mm[i][j]`, `arr[k].mm[i][j]`, `s.inner.m3[i][j][k]`. Every element
    /// of a member with two or more unpacked dimensions is its own leaf,
    /// keyed by the full index list. The generic element arms key off the
    /// `arrays_2d`/`arrays_nd` shape of a plain identifier, which a
    /// member reached through an index (`arr[k].mm`), a synthesized
    /// member-access chain (a formal's caller expression, a member-wise
    /// copy) or a block-local struct never has; they then read `s.mm[i]` and
    /// bit-selected it. `None` unless such a leaf exists, so every other
    /// select keeps its existing path.
    pub(super) fn struct_member_elem_leaf(
        &mut self,
        base: &Expression,
        index: &Expression,
    ) -> Option<String> {
        if !matches!(base.kind, ExprKind::Index { .. })
            || !self.multidim_members_possible()
            || !Self::cond_expr_is_effect_free(index, false)
        {
            return None;
        }
        let mut cur = base;
        while let ExprKind::Index { expr, index } = &cur.kind {
            if !Self::cond_expr_is_effect_free(index, false) {
                return None;
            }
            cur = expr;
        }
        let member_path = match &cur.kind {
            ExprKind::MemberAccess { .. } => true,
            ExprKind::Ident(h) => h.path.len() >= 2,
            _ => false,
        };
        if !member_path
            || !Self::expr_root_first_seg(cur).is_some_and(|r| self.struct_root_possible(r))
        {
            return None;
        }
        let bflat = self.flat_member_name(base)?;
        let iv = self.eval_expr(index);
        if iv.has_xz() {
            return None;
        }
        let leaf = Self::name_with_index(&bflat, iv.to_i64()?);
        let exists = |sim: &Self, n: &str| {
            sim.signal_name_to_id.contains_key(n) || sim.signals.contains_key(n)
        };
        if exists(self, &leaf) {
            return Some(leaf);
        }
        // Inside an inlined instance the leaf carries the instance prefix.
        self.hint_scoped(&leaf).filter(|s| exists(self, s))
    }

    /// The leading identifier of a member/index chain (`s` in `s.arr[2].x`).
    pub(super) fn expr_root_ident(e: &Expression) -> Option<&str> {
        match &e.kind {
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                Some(h.path[0].name.name.as_str())
            }
            ExprKind::MemberAccess { expr, .. } | ExprKind::Index { expr, .. } => {
                Self::expr_root_ident(expr)
            }
            _ => None,
        }
    }

    /// If `arg` is a positional STRUCT ASSIGNMENT-PATTERN literal
    /// (`'{a, b, c}`) with exactly the given number of flat members, evaluate
    /// its parts in declaration order. `member-access` on a literal base
    /// (`('{a,b}).x`) does not evaluate in this simulator, so binding a struct
    /// formal from a literal via `arg.<member>` reads x — this lets the struct
    /// bind paths consume a literal directly. Returns `None` when `arg` is not
    /// a simple ordered pattern (caller falls back to member-access eval).
    pub(super) fn eval_flat_struct_pattern(
        &mut self,
        arg: &Expression,
        n: usize,
    ) -> Option<Vec<Value>> {
        let ExprKind::AssignmentPattern(parts) = &arg.kind else {
            return None;
        };
        if parts.len() != n {
            return None;
        }
        let mut out = Vec::with_capacity(n);
        for p in parts.iter() {
            // Positional only; keyed (`'{"name": v}`) and `default:` forms are
            // not positionally ordinal.
            if let crate::ast::expr::AssignmentPatternItem::Ordered(e) = p {
                out.push(self.eval_expr(e));
            } else {
                return None;
            }
        }
        Some(out)
    }

    pub(super) fn bind_unpacked_struct_arg(
        &mut self,
        port_name: &str,
        dt: &DataType,
        arg: &Expression,
        locals: &mut HashMap<String, Value>,
        handle: Option<usize>,
    ) -> Option<Vec<(String, Expression)>> {
        // Returns the per-member bindings — each `(local_key, caller_lvalue)` —
        // so an `output`/`inout`/`ref` formal can be written back member-wise
        // (the whole-value `output_bindings` path can't see per-member locals
        // `o.a`, `o.b`). `None` ⇒ not a member-wise struct formal.
        // Decide by borrow first: most formals are integral or class
        // handles, and cloning the declared type to learn that was a
        // per-formal, per-call copy.
        match Self::resolve_type_ref_borrowed(dt, &self.module.typedef_types) {
            DataType::Struct(su) if Self::spreads_member_wise(su) => {}
            DataType::TypeReference { .. } => {}
            _ => return None,
        }
        let dt_resolved = Self::resolve_type_ref(dt, &self.module.typedef_types);
        let dt_resolved = match dt_resolved {
            DataType::TypeReference { name, .. } => {
                let tn = &name.name.name;
                // A TYPE-PARAMETER formal (`class C #(type T); new(T f)`, or a
                // method `function int rd(T f)`) names the parameter, not a
                // typedef. Resolve it to the concrete bound type, preferring
                // the CALLEE instance's own `type_bindings` (authoritative for
                // regular method calls, where no class specialization is on
                // `current_spec`) and falling back to `current_spec` (set for
                // a constructor call). Without this the struct actual is never
                // bound member-wise and the body reads every member as x.
                let concrete = self
                    .heap
                    .get(handle.unwrap_or(0))
                    .and_then(|o| o.as_ref())
                    .and_then(|i| i.type_bindings.get(tn).cloned())
                    .or_else(|| self.resolve_type_param_binding(tn))
                    .unwrap_or_else(|| tn.clone());
                self.module
                    .typedef_types
                    .get(concrete.as_str())
                    .cloned()
                    .unwrap_or_else(|| DataType::TypeReference {
                        name: name.clone(),
                        dimensions: vec![],
                        type_args: vec![],
                        span: name.span,
                    })
            }
            other => other,
        };
        let DataType::Struct(su) = dt_resolved else {
            return None;
        };
        if !Self::spreads_member_wise(&su) {
            return None;
        }
        let mut entries = Vec::new();
        // A positional STRUCT pattern literal argument (`'{a, b}`) can't be
        // read member-by-member (`.a` on a pattern returns x), so bind its
        // evaluated parts positionally when the shape matches the flat members.
        let flat_names: Vec<String> = su
            .members
            .iter()
            .flat_map(|m| m.declarators.iter().map(|md| md.name.name.clone()))
            .collect();
        // §7.2/§13.5.2: a member with fixed unpacked dimensions, or one that
        // is itself an unpacked struct, has no storage of its own — only its
        // leaves do. Binding it as ONE value left `a.arr[i]` and
        // `a.inner.x` unbound in the method body, so bind every leaf, as a
        // module-scope subroutine's formal does. A struct of plain members
        // keeps the per-member binding below.
        let nested_leaves = su.members.iter().any(|m| {
            self.unpacked_struct_of(&m.data_type).is_some()
                || m.declarators
                    .iter()
                    .any(|md| self.member_dim_indices(&md.dimensions).is_some())
        });
        if nested_leaves {
            let mut leaves = Vec::new();
            self.unpacked_struct_leaves(port_name, arg, &su, 0, &mut leaves);
            // The `<port>.` marker routes indexed / nested member references
            // to the frame (see `frame_leaf_key_of`).
            self.any_struct_formal_markers = true;
            self.note_struct_root(port_name);
            locals.insert(format!("{}.", port_name), Value::zero(1));
            let pattern_vals = self.eval_flat_struct_pattern(arg, leaves.len());
            let call_vals = if pattern_vals.is_none() {
                self.call_struct_leaf_values(port_name, &su, arg)
            } else {
                None
            };
            for (pos, (key, caller_expr, w, _)) in leaves.into_iter().enumerate() {
                let v = match (&pattern_vals, &call_vals) {
                    (Some(pv), _) => pv[pos].clone(),
                    (None, Some(cv)) => cv.get(&key).cloned().unwrap_or_else(|| Value::new(w)),
                    (None, None) => self.eval_expr(&caller_expr),
                };
                locals.insert(key.clone(), v);
                // A top-level member writes back through the same lvalue
                // form as the per-member binding below.
                let top = key
                    .strip_prefix(port_name)
                    .and_then(|k| k.strip_prefix('.'))
                    .filter(|k| flat_names.iter().any(|n| n == k));
                let lv = match top {
                    Some(m) => Self::struct_member_lvalue(arg, m),
                    None => caller_expr,
                };
                entries.push((key, lv));
            }
            return Some(entries);
        }
        let pattern_vals = self.eval_flat_struct_pattern(arg, flat_names.len());
        let call_vals = if pattern_vals.is_none() {
            self.call_struct_leaf_values(port_name, &su, arg)
        } else {
            None
        };
        let mut member_pos = 0usize;
        for m in &su.members {
            for md in &m.declarators {
                let fname = &md.name.name;
                let local_key = format!("{}.{}", port_name, fname);
                let v = if let Some(pv) = &pattern_vals {
                    pv[member_pos].clone()
                } else if let Some(v) = call_vals.as_ref().and_then(|cv| cv.get(&local_key)) {
                    v.clone()
                } else {
                    let member_expr = Expression::new(
                        ExprKind::MemberAccess {
                            expr: Box::new(arg.clone()),
                            member: crate::ast::Identifier {
                                name: fname.clone(),
                                span: arg.span,
                            },
                        },
                        arg.span,
                    );
                    self.eval_expr(&member_expr)
                };
                member_pos += 1;
                locals.insert(local_key.clone(), v);
                // Write-back lvalue: for a plain-identifier actual emit the
                // hierarchical `Ident([s, a])` form (how source `s.a` parses)
                // so assign_value's Ident arm handles it whether the member
                // lives in the compact signal table or the runtime map; the
                // `MemberAccess` form misses the runtime-map case. Other
                // actuals (class-property `obj.prop`) wrap in `MemberAccess`,
                // which the aggregate-property path resolves.
                entries.push((local_key, Self::struct_member_lvalue(arg, fname)));
            }
        }
        Some(entries)
    }

    /// Build a member-access lvalue for the write-back of a struct formal.
    /// See `bind_unpacked_struct_arg` for why the `Ident` form is preferred.
    /// `<base>[i]` as an expression, for driving per-element decomposition.
    pub(super) fn index_expr(base: &Expression, i: i64) -> Expression {
        Expression::new(
            ExprKind::Index {
                expr: Box::new(base.clone()),
                index: Box::new(Expression::new(
                    ExprKind::Number(NumberLiteral::Integer {
                        size: (i < 0).then_some(64),
                        signed: true,
                        base: if i < 0 {
                            NumberBase::Hex
                        } else {
                            NumberBase::Decimal
                        },
                        value: if i < 0 {
                            format!("{:016x}", i as u64)
                        } else {
                            i.to_string()
                        },
                        cached_val: std::cell::Cell::new(None),
                    }),
                    base.span,
                )),
            },
            base.span,
        )
    }

    pub(super) fn struct_member_lvalue(base: &Expression, member: &str) -> Expression {
        if let ExprKind::Ident(hier) = &base.kind {
            let mut path = hier.path.clone();
            path.push(crate::ast::expr::HierPathSegment {
                name: crate::ast::Identifier {
                    name: member.to_string(),
                    span: base.span,
                },
                selects: Vec::new(),
            });
            return Expression::new(
                ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                    root: hier.root.clone(),
                    path,
                    span: base.span,
                    cached_signal_id: std::cell::Cell::new(None),
                    cached_resolved_name: std::cell::OnceCell::new(),
                }),
                base.span,
            );
        }
        Expression::new(
            ExprKind::MemberAccess {
                expr: Box::new(base.clone()),
                member: crate::ast::Identifier {
                    name: member.to_string(),
                    span: base.span,
                },
            },
            base.span,
        )
    }

    /// §13.5.2: resolve the ACTUAL of an unpacked-array formal (associative,
    /// fixed-size or multi-dimensional) to its storage key, whatever the
    /// spelling: a bare name (a caller-local, or an implicit-`this` class
    /// member under its per-process/per-instance key), an explicit member
    /// access (`c.member` / `this.member`), or a member chain (`a.b.member`).
    /// `queue_actual_storage` covers queues; the fixed-array and assoc
    /// binders share this one, so an `output`/`inout`/`ref` formal whose
    /// actual is a CLASS PROPERTY writes back to the property, not to a stray
    /// bare-named signal (`fill(prop)`).
    ///
    /// §23.6 / §26.3: a hierarchical instance or interface path (`u.A`,
    /// `m.s.A`, `i.IA`) or a package-qualified array (`pk::PA`) is not a
    /// class property: it keeps the plain hierarchical name, as before.
    pub(super) fn array_actual_storage(&mut self, arg: &Expression) -> Option<String> {
        use crate::ast::expr::ExprKind;
        let ExprKind::Ident(h) = &arg.kind else {
            // An explicit member access `c.member` / `this.member` naming an
            // unpacked-array property: evaluate the receiver to its object
            // handle and resolve `<handle>#member`.
            let ExprKind::MemberAccess { expr, member } = &arg.kind else {
                return None;
            };
            let handle = self.eval_handle_expr(expr)?;
            let cname = self.handle_collection_name(handle, &member.name)?;
            return self.is_unpacked_array_storage(&cname).then_some(cname);
        };
        let cname = if h.path.len() == 1 {
            // A bare name: a caller-local collection (under its per-process
            // rename), or an implicit-`this` unpacked-array property stored at
            // `<handle>#member` (an assoc / fixed-array formal actual
            // `fill(A)` inside the method).
            let n = self.resolve_hier_name(h);
            if let Some(r) = self.dyn_name_lookup(&n) {
                Some(r.to_string())
            } else {
                self.instance_assoc_member(&n)
                    .or_else(|| Some(n.into_owned()))
            }
        } else if h.path.iter().all(|s| s.selects.is_empty()) {
            // A flattened member chain `a.b.member` naming an unpacked-array
            // property. HEAD is the base object; middle segments are
            // handle-valued properties; the LAST names the collection, stored
            // per instance at `<handle>#member`. A head that is no class
            // handle is a hierarchical path, resolved below.
            self.chain_collection_storage(h)
        } else {
            None
        };
        if let Some(c) = cname.filter(|c| self.is_unpacked_array_storage(c)) {
            return Some(c);
        }
        Some(self.resolve_hier_name(h).into_owned())
    }

    /// `<handle>#member` storage of a select-free member chain `a.b.member`
    /// whose head is a class handle; `None` otherwise.
    pub(super) fn chain_collection_storage(
        &mut self,
        h: &crate::ast::expr::HierarchicalIdentifier,
    ) -> Option<String> {
        let head = &h.path[0].name.name;
        let leaf = &h.path.last()?.name.name;
        let mut handle = self.eval_ident_handle(head)?;
        for seg in h.path.iter().take(h.path.len() - 1).skip(1) {
            handle = self.member_handle(handle, &seg.name.name)?;
        }
        self.handle_collection_name(handle, leaf)
    }

    /// `name` is registered as unpacked-array storage (fixed, dynamic,
    /// associative or multi-dimensional).
    pub(super) fn is_unpacked_array_storage(&self, name: &str) -> bool {
        self.module.arrays.contains_key(name)
            || self.module.dynamic_arrays.contains(name)
            || self.module.associative_arrays.contains_key(name)
            || self.module.arrays_2d.contains_key(name)
            || self.module.arrays_nd.contains_key(name)
    }

    /// Bind an ASSOCIATIVE-array formal. Returns `(storage, caller, prior)`:
    /// the key the body's accesses resolve to, the caller's array, and the
    /// storage key's registration before the call. `storage == caller` marks
    /// an alias (nothing to copy back or purge).
    ///
    /// IEEE 1800-2023 §13.5.1: an `input`/`output`/`inout` formal is a COPY
    /// under a per-call key (see `formal_dyn_key`), so the body's writes stay
    /// local even when the actual has the formal's own name; `output` starts
    /// empty. §13.5.2: a `ref` formal ALIASES the actual's storage, so
    /// concurrent callers' writes all land in it.
    pub(super) fn bind_assoc_param(
        &mut self,
        port: &crate::ast::decl::FunctionPort,
        arg: &Expression,
    ) -> Option<(String, String, Option<bool>)> {
        if !self.port_is_assoc_array(port) {
            return None;
        }
        // §13.5.3 / §10.9.1: an assignment-pattern actual (a formal's default,
        // `int m[string] = '{default: 0}`) has no storage to copy from: the
        // formal starts as a fresh array holding the pattern's keyed items,
        // with its `default:` recorded for absent keys.
        if matches!(
            arg.kind,
            ExprKind::AssignmentPattern(_) | ExprKind::Concatenation(_)
        ) {
            let param = port.name.name.clone();
            let key = self.formal_dyn_key(&param, &port.data_type);
            let is_string_key = port.dimensions.iter().any(|d| match d {
                crate::ast::types::UnpackedDimension::Associative {
                    data_type: Some(kt),
                    ..
                } => matches!(
                    super::elaborate::resolve_typedef_chain(kt, &self.module.typedef_types),
                    DataType::Simple {
                        kind: crate::ast::types::SimpleType::String,
                        ..
                    }
                ),
                _ => false,
            });
            self.module
                .associative_arrays
                .insert(key.clone(), is_string_key);
            let w = resolve_type_width(
                &port.data_type,
                Some(&self.module.parameters),
                Some(&self.module.typedefs),
            );
            if w > 0 {
                self.widths.insert(key.clone(), w);
            }
            if matches!(arg.kind, ExprKind::AssignmentPattern(_)) {
                self.assign_whole_coll(&key, Some(&port.data_type), arg);
            }
            return Some((key, String::new(), None));
        }
        let caller = self.array_actual_storage(arg)?;
        let param = port.name.name.clone();
        if matches!(port.direction, PortDirection::Ref) {
            self.map_formal_dyn(&param, &caller);
            let prior = self.module.associative_arrays.get(&caller).copied();
            return Some((caller.clone(), caller, prior));
        }
        let key = self.formal_dyn_key(&param, &port.data_type);
        if !matches!(port.direction, PortDirection::Output) {
            // §13.5.1: copy every element in, including unpacked-struct
            // member leaves (`caller[k].x`).
            let prefix = format!("{}[", caller);
            let entries: Vec<(String, Value)> = self
                .signals
                .keys_with_elem_prefix(&prefix)
                .into_iter()
                .filter_map(|k| {
                    let v = self.signals.get(&k)?.clone();
                    Some((format!("{}{}", key, &k[caller.len()..]), v))
                })
                .collect();
            for (k, v) in entries {
                self.signals.insert(k, v);
            }
        }
        let is_string_key = self.is_string_keyed_array(&caller);
        self.module
            .associative_arrays
            .insert(key.clone(), is_string_key);
        if let Some(&w) = self.widths.get(caller.as_str()) {
            self.widths.insert(key.clone(), w);
        }
        if self.string_signals.contains(caller.as_str()) {
            self.string_signals.insert(key.clone());
        }
        Some((key, caller, None))
    }

    /// Drop a formal associative array's entries and registration. Restores
    /// any PRIOR registration an enclosing call had for the same name (see
    /// `bind_assoc_param`'s captured `prior`): a nested call must not leave
    /// the array unregistered after it returns just because its own formal
    /// happens to share the name — the flat `module.associative_arrays` table
    /// is shared across nesting depths.
    pub(super) fn purge_assoc_param(&mut self, param: &str, prior: Option<bool>) {
        let prefix = format!("{}[", param);
        let keys: Vec<String> = self.signals.keys_with_elem_prefix(&prefix);
        for k in keys {
            self.signals.remove(&k);
        }
        // A pattern-bound formal's `default:` (see `bind_assoc_param`).
        if !self.module.assoc_defaults.is_empty() {
            self.module.assoc_defaults.remove(param);
        }
        match prior {
            // An entry predated THIS call (it belongs to an enclosing frame
            // whose bindings are still live) — put it back.
            Some(is_string_key) => {
                self.module
                    .associative_arrays
                    .insert(param.to_string(), is_string_key);
            }
            // This call created the registration; no enclosing frame wanted
            // it — drop it.
            None => {
                self.module.associative_arrays.remove(param);
            }
        }
    }

    /// Stage an OUTPUT associative formal's element data onto a fresh temp
    /// key, so the writeback can run AFTER the caller's shadowed storage is
    /// restored. The formal's own per-call key (`@p#0`) is torn down by
    /// `pop_and_restore_queue_frame`; staging it here (before that restore)
    /// keeps the data alive for `writeback_assoc_param` to copy afterwards.
    /// Returns the temp key.
    pub(super) fn stage_assoc_formal(&mut self, param: &str) -> String {
        let id = self.next_dyn_id;
        self.next_dyn_id += 1;
        let tmp = format!("@wb#{}", id);
        let pre = format!("{}[", param);
        let moved: Vec<(String, Value)> = self
            .signals
            .keys_with_elem_prefix(&pre)
            .into_iter()
            .filter_map(|k| {
                let v = self.signals.get(&k)?.clone();
                let nk = format!("{}{}", tmp, &k[param.len()..]);
                Some((nk, v))
            })
            .collect();
        for (k, v) in moved {
            self.signals.insert(k, v);
        }
        tmp
    }

    /// Stage every `output`/`inout`/`ref` associative formal of a returning
    /// call (`(formal, caller actual, is_out)`; an identity binding, formal
    /// named like its actual, has nothing to copy) before
    /// `pop_and_restore_queue_frame` tears the formal's per-call key down.
    /// Returns `(temp key, caller)` pairs for `apply_assoc_writebacks`.
    pub(super) fn stage_assoc_writebacks<'a>(
        &mut self,
        formals: impl Iterator<Item = (&'a str, &'a str, bool)>,
    ) -> Vec<(String, String)> {
        let mut staged = Vec::new();
        for (param, caller, is_out) in formals {
            if is_out && param != caller && !caller.is_empty() {
                staged.push((self.stage_assoc_formal(param), caller.to_string()));
            }
        }
        staged
    }

    /// §13.5.2: after the caller's shadowed storage (and its associative
    /// registration) is restored, copy each staged formal onto the caller's
    /// array and drop the staged keys.
    pub(super) fn apply_assoc_writebacks(&mut self, staged: Vec<(String, String)>) {
        for (tmp, caller) in staged {
            self.writeback_assoc_param(&tmp, &caller);
            let tprefix = format!("{}[", tmp);
            for k in self.signals.keys_with_elem_prefix(&tprefix) {
                self.signals.remove(&k);
            }
            self.signals.remove(&format!("{}.size", tmp));
        }
    }

    /// Copy a `ref`/`output`/`inout` associative formal back onto the caller's
    /// array, replacing its contents.
    pub(super) fn writeback_assoc_param(&mut self, param: &str, caller: &str) {
        // When the formal and the caller's actual share a name, the body's
        // writes landed directly in the caller's signal namespace (they
        // alias). There is nothing to copy back, and the clear-then-copy
        // below would delete the caller's data. (§13.5.2 ref aliasing.)
        if param == caller {
            return;
        }
        let cprefix = format!("{}[", caller);
        let old: Vec<String> = self.signals.keys_with_elem_prefix(&cprefix);
        for k in old {
            self.signals.remove(&k);
        }
        let pprefix = format!("{}[", param);
        let entries: Vec<(String, Value)> = self
            .signals
            .keys_with_elem_prefix(&pprefix)
            .into_iter()
            .filter_map(|k| {
                let v = self.signals.get(&k)?.clone();
                Some((format!("{}{}", caller, &k[param.len()..]), v))
            })
            .collect();
        for (k, v) in entries {
            self.signals.insert(k, v);
        }
    }

    /// Copy a fixed unpacked ARRAY actual into the formal's element signals and
    /// register the formal as an array so `a[i]` resolves inside the body.
    /// Returns `(formal, actual, lo, hi)` so an `output`/`inout`/`ref` formal
    /// can be copied back on return (§13.5.2).
    ///
    /// Only `[lo:hi]` range dimensions were handled, and only for tasks, so the
    /// far commoner `int a[3]` size form silently passed nothing: the body read
    /// X and a `ref` write never reached the caller.
    pub(super) fn bind_array_arg(
        &mut self,
        port: &crate::ast::decl::FunctionPort,
        arg: &Expression,
    ) -> Option<(String, String, i64, i64)> {
        use crate::ast::types::UnpackedDimension as UD;
        if port.dimensions.is_empty() {
            return None;
        }
        let dims = super::elaborate::normalize_unpacked_dims(
            &port.dimensions,
            &self.module.parameters,
            &self.module.typedef_types,
        );
        let params = Some(&self.module.parameters);
        let (lo, hi) = match dims.first()? {
            UD::Range { left, right, .. } => {
                let l = super::elaborate::const_eval_i64_with_params(left, params)?;
                let r = super::elaborate::const_eval_i64_with_params(right, params)?;
                (l.min(r), l.max(r))
            }
            UD::Expression { expr, .. } => {
                let n = super::elaborate::const_eval_i64_with_params(expr, params)?;
                if n <= 0 {
                    return None;
                }
                (0, n - 1)
            }
            _ => return None,
        };
        // §13.5.3 / §10.9.1: an assignment-pattern actual (in practice a
        // formal's default, `byte v[2] = '{8'hA5, 8'h5A}`) has no storage to
        // copy from: register the formal and assign the pattern's items to
        // its elements, the leftmost item to the leftmost index. There is
        // nothing to copy back, so the returned caller is empty.
        if let ExprKind::AssignmentPattern(items) = &arg.kind {
            if dims.len() != 1 {
                return None;
            }
            let param = port.name.name.clone();
            let dt = &port.data_type;
            let w = super::elaborate::resolve_type_width(
                dt,
                Some(&self.module.parameters),
                Some(&self.module.typedefs),
            )
            .max(1);
            let descending = match dims.first() {
                Some(UD::Range { left, right, .. }) => {
                    super::elaborate::const_eval_i64_with_params(left, params)
                        > super::elaborate::const_eval_i64_with_params(right, params)
                }
                _ => false,
            };
            self.module.arrays.insert(param.clone(), (lo, hi, w));
            let ordered: Vec<Expression> =
                self.pattern_ordered(items).into_iter().cloned().collect();
            let default = items.iter().find_map(|it| match it {
                AssignmentPatternItem::Default(e) => Some(e.clone()),
                _ => None,
            });
            for (j, idx) in (lo..=hi).enumerate() {
                let idx = if descending { hi - (idx - lo) } else { idx };
                let Some(e) = ordered.get(j).or(default.as_ref()) else {
                    break;
                };
                let e = e.clone();
                self.assign_pattern_or_leaf(&format!("{}[{}]", param, idx), dt, &e);
            }
            return Some((param, String::new(), lo, hi));
        }
        // §18.5.12 / §13.5.2: resolve the actual's storage key across every
        // spelling — a bare name (a caller-local, or an implicit-`this` member
        // under `<handle>#<member>`), or an explicit member access `c.member` /
        // `this.member` / `a.b.member` (handled by `array_actual_storage`;
        // a fixed-array property actual `parity_bytes` previously returned
        // None here, dropping the writeback).
        let caller = self.array_actual_storage(arg)?;
        // §13.5.2 multi-dimensional formal (`int a[R][C]`, `a[X][Y][Z]`): copy
        // every element of the caller's array in under the formal's name and
        // register the same shape, so element, `foreach`, `%p`, `$size` and
        // whole-array uses inside the body see a real array; out/inout/ref
        // copy back through `writeback_array_args`. Only the first dimension
        // was handled before: a 2-D formal read x and never wrote back.
        if dims.len() >= 2 {
            let param = port.name.name.clone();
            let suffixes =
                if let Some(&((a0, a1), (b0, b1), w)) = self.module.arrays_2d.get(&*caller) {
                    self.module
                        .arrays_2d
                        .insert(param.clone(), ((a0, a1), (b0, b1), w));
                    Self::nd_elem_suffixes(&[(a0, a1), (b0, b1)])
                } else if let Some((shape, w)) = self.module.arrays_nd.get(&*caller).cloned() {
                    self.module
                        .arrays_nd
                        .insert(param.clone(), (shape.clone(), w));
                    Self::nd_elem_suffixes(&shape)
                } else {
                    return None;
                };
            self.multi_dim_array_names.insert(param.clone());
            if param != caller {
                let w = match self.module.arrays_2d.get(&param) {
                    Some(&(_, _, w)) => w,
                    None => self.module.arrays_nd.get(&param).map_or(32, |t| t.1),
                };
                let out_default = self.output_formal_elem_default(port, w);
                for sfx in &suffixes {
                    let v = match &out_default {
                        Some(d) => Some(d.clone()),
                        None => self.get_signal_value_by_name(&format!("{}{}", caller, sfx)),
                    };
                    if let Some(v) = v {
                        self.signals.insert(format!("{}{}", param, sfx), v);
                    }
                }
            }
            return Some((param, caller.to_string(), lo, hi));
        }
        if !self.module.arrays.contains_key(&*caller) {
            return None;
        }
        let param = port.name.name.clone();
        // §7.2: an element of an UNPACKED-struct array is a set of member
        // leaves. Copying `a[i]` alone brought nothing across, so a
        // `pkt_t arr[3]` formal read zero from every member.
        let elem_su = self
            .p_elem_type(&caller)
            .or_else(|| self.p_elem_type(&param))
            .and_then(|dt| self.unpacked_struct_of(&dt));
        let w = self.module.arrays.get(&*caller).map(|t| t.2).unwrap_or(32);
        let out_default = if param == caller {
            None
        } else {
            self.output_formal_elem_default(port, w)
        };
        for idx in lo..=hi {
            if let Some(su) = &elem_su {
                let d = format!("{}[{}]", param, idx);
                if out_default.is_some() {
                    let mut leaves = Vec::new();
                    self.unpacked_struct_leaf_defaults(&d, su, 0, &mut leaves);
                    for (k, dv, _) in leaves {
                        self.signals.insert(k, dv);
                    }
                    continue;
                }
                let s = format!("{}[{}]", caller, idx);
                self.copy_unpacked_struct(&d, &s, &su.clone());
                continue;
            }
            let v = match &out_default {
                Some(d) => Some(d.clone()),
                None => self.get_signal_value_by_name(&format!("{}[{}]", caller, idx)),
            };
            if let Some(v) = v {
                self.signals.insert(format!("{}[{}]", param, idx), v);
            }
        }
        self.module.arrays.insert(param.clone(), (lo, hi, w));
        Some((param, caller.to_string(), lo, hi))
    }

    /// §13.5.1: an `output` formal is not copied in: it starts at its type's
    /// default (0 for a 2-state element, x for a 4-state one, "" for a
    /// string, 0.0 for a real) and the whole formal is copied out on return,
    /// so an element the body leaves alone overwrites the caller's with
    /// that default. `None` for any other direction.
    pub(super) fn output_formal_elem_default(
        &self,
        port: &crate::ast::decl::FunctionPort,
        w: u32,
    ) -> Option<Value> {
        if !matches!(port.direction, PortDirection::Output) {
            return None;
        }
        let dt = &port.data_type;
        let resolved = super::elaborate::resolve_typedef_chain(dt, &self.module.typedef_types);
        if matches!(
            resolved,
            crate::ast::types::DataType::Simple {
                kind: crate::ast::types::SimpleType::String,
                ..
            }
        ) {
            return Some(Value::from_string(""));
        }
        if super::elaborate::is_type_real_resolved(dt, &self.module.typedef_types) {
            return Some(Value::from_f64(0.0));
        }
        if super::elaborate::is_type_two_state_resolved(dt, &self.module.typedef_types) {
            Some(Value::zero(w))
        } else {
            Some(Value::new(w))
        }
    }

    /// Copy an array formal's elements back onto the caller's array, then drop
    /// the formal's element signals.
    /// `[i][j]…` element suffixes of an N-dimensional shape, row-major.
    pub(super) fn nd_elem_suffixes(dims: &[(i64, i64)]) -> Vec<String> {
        let mut out: Vec<String> = vec![String::new()];
        for &(d0, d1) in dims {
            let (lo, hi) = (d0.min(d1), d0.max(d1));
            let mut next = Vec::with_capacity(out.len() * ((hi - lo + 1).max(0) as usize));
            for prefix in &out {
                for i in lo..=hi {
                    next.push(format!("{}[{}]", prefix, i));
                }
            }
            out = next;
        }
        out
    }

    /// Element suffixes of a name registered as a multi-dimensional array.
    pub(super) fn nd_formal_suffixes(&self, name: &str) -> Option<Vec<String>> {
        if let Some(&(a, b, _)) = self.module.arrays_2d.get(name) {
            return Some(Self::nd_elem_suffixes(&[a, b]));
        }
        if let Some((shape, _)) = self.module.arrays_nd.get(name) {
            return Some(Self::nd_elem_suffixes(shape));
        }
        None
    }

    /// After the copy-back, drop each written-back formal's elements and
    /// registrations. A class method left them behind, so a later call's
    /// same-named formal of another shape inherited them: a stale 1-D `p`
    /// turned `p[1][0]` of an `output int p[2][2]` into a 1-D select.
    pub(super) fn purge_array_formals(&mut self, wb: &[(String, String, i64, i64)]) {
        for (param, caller, ..) in wb {
            // An IDENTITY binding (formal named like the caller's actual —
            // `task bump(ref int arr[3]); ... bump(arr)`) shares the
            // caller's storage: purging the formal's keys deleted the
            // caller's whole array (audit46 a10 — post-call all-x).
            if param == caller {
                continue;
            }
            let prefix = format!("{}[", param);
            for k in self.signals.keys_with_elem_prefix(&prefix) {
                self.signals.remove(&k);
            }
            self.purge_array_formal(param);
        }
    }

    /// Drop the array registrations a formal received in `bind_array_arg`.
    pub(super) fn purge_array_formal(&mut self, param: &str) {
        self.module.arrays.remove(param);
        if self.module.arrays_2d.remove(param).is_some()
            | self.module.arrays_nd.remove(param).is_some()
        {
            self.multi_dim_array_names.remove(param);
        }
    }

    /// An unpacked-array operand as `(base, element suffixes)`: a whole
    /// N-D array (`v`), a row of one (`v[1]` -> base `v[1]`, suffixes over
    /// the remaining dimensions), or a plain fixed 1-D array. `None` for
    /// anything else (dynamic / associative / queue / scalar).
    pub(super) fn nd_array_operand(
        &mut self,
        e: &Expression,
    ) -> Option<(String, Vec<String>, bool)> {
        match &e.kind {
            ExprKind::Ident(h)
                if h.path.iter().all(|s| s.selects.is_empty())
                    && !self.ident_shadowed_by_non_collection(h) =>
            {
                let name = self.resolve_hier_name(h);
                if let Some(sfx) = self.nd_formal_suffixes(&name) {
                    return Some((name.to_string(), sfx, true));
                }
                if let Some(&(lo, hi, _)) = self.module.arrays.get(&*name) {
                    if !self.module.dynamic_arrays.contains(&*name) {
                        let desc = self.module.descending_arrays.contains(&*name);
                        let mut sfx: Vec<String> = (lo..=hi).map(|i| format!("[{}]", i)).collect();
                        if desc {
                            sfx.reverse();
                        }
                        return Some((name.to_string(), sfx, false));
                    }
                }
                None
            }
            ExprKind::Index { expr, index } => {
                let ExprKind::Ident(h) = &expr.kind else {
                    return None;
                };
                if !h.path.iter().all(|s| s.selects.is_empty()) {
                    return None;
                }
                let name = self.resolve_hier_name(h);
                let rest: Vec<(i64, i64)> =
                    if let Some(&(_, b, _)) = self.module.arrays_2d.get(&*name) {
                        vec![b]
                    } else if let Some((shape, _)) = self.module.arrays_nd.get(&*name) {
                        if shape.len() < 2 {
                            return None;
                        }
                        shape[1..].to_vec()
                    } else {
                        return None;
                    };
                let i = self.eval_expr(index).to_i64()?;
                Some((
                    format!("{}[{}]", name, i),
                    Self::nd_elem_suffixes(&rest),
                    true,
                ))
            }
            _ => None,
        }
    }

    /// `w = v` / `r = v[1]` / `v[1] = r` over multi-dimensional unpacked
    /// arrays. Returns true when it performed the copy.
    pub(super) fn copy_multi_dim_array(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
    ) -> bool {
        if self.module.arrays_2d.is_empty() && self.module.arrays_nd.is_empty() {
            return false;
        }
        let Some((lbase, lsfx, l_nd)) = self.nd_array_operand(lvalue) else {
            return false;
        };
        let Some((rbase, rsfx, r_nd)) = self.nd_array_operand(rvalue) else {
            return false;
        };
        if !(l_nd || r_nd) || (lbase == rbase && lsfx == rsfx) {
            return false;
        }
        for (ls, rs) in lsfx.iter().zip(rsfx.iter()) {
            let v = self
                .get_signal_value_by_name(&format!("{}{}", rbase, rs))
                .unwrap_or_else(|| Value::zero(32));
            self.set_signal_value_by_name(&format!("{}{}", lbase, ls), v);
        }
        true
    }

    pub(super) fn writeback_array_args(&mut self, wb: &[(String, String, i64, i64)]) {
        for (param, caller, lo, hi) in wb {
            if let Some(sfxs) = self.nd_formal_suffixes(param) {
                if param != caller {
                    for sfx in sfxs {
                        if let Some(v) = self.get_signal_value_by_name(&format!("{}{}", param, sfx))
                        {
                            self.set_signal_value_by_name(&format!("{}{}", caller, sfx), v);
                        }
                    }
                }
                continue;
            }
            // §7.2: elements of an UNPACKED-struct array are member leaves, so
            // copying `a[i]` alone wrote nothing back — an `output pkt_t a[3]`
            // formal left every member of the caller's array untouched.
            let elem_su = self
                .p_elem_type(param)
                .or_else(|| self.p_elem_type(caller))
                .and_then(|dt| self.unpacked_struct_of(&dt));
            for idx in *lo..=*hi {
                if let Some(su) = &elem_su {
                    let (d, s) = (
                        format!("{}[{}]", caller, idx),
                        format!("{}[{}]", param, idx),
                    );
                    self.copy_unpacked_struct(&d, &s, &su.clone());
                    continue;
                }
                if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", param, idx)) {
                    self.set_signal_value_by_name(&format!("{}[{}]", caller, idx), v);
                }
            }
        }
    }

    /// IEEE 1800-2017 §13.5.4 named arguments (`f(.b(2), .a(1))`) and §13.5.3
    /// default arguments. Reorders the actuals into FORMAL order and fills any
    /// omitted formal from its default. Returns `None` when there are no named
    /// arguments, so callers keep their original slice.
    ///
    /// Argument binding is positional everywhere in the simulator, so without
    /// this a `.name(expr)` actual was evaluated as an opaque expression in
    /// whatever position it happened to occupy — silently binding the wrong
    /// formal, or nothing at all.
    /// §13.5.3: the default an OMITTED argument binds through the aggregate
    /// binders, like a supplied actual: one that can be an aggregate (an
    /// assignment pattern or an unpacked concatenation), or any default of a
    /// formal with unpacked dimensions or an `output`/`inout` direction (whose
    /// default names the variable copied back to). A scalar or enum default
    /// of an input keeps the scalar path, which evaluates it in the callee's
    /// scope.
    #[inline]
    pub(super) fn default_actual(port: &crate::ast::decl::FunctionPort) -> Option<&Expression> {
        let d = port.default.as_ref()?;
        (matches!(
            d.kind,
            ExprKind::AssignmentPattern(_) | ExprKind::Concatenation(_)
        ) || !port.dimensions.is_empty()
            || matches!(port.direction, PortDirection::Output | PortDirection::Inout))
        .then_some(d)
    }

    /// §13.5.3: the values of a method's default-bound `output`/`inout`
    /// formals (`(formal, default lvalue)`), read before its frame is popped.
    #[cold]
    #[inline(never)]
    pub(super) fn snapshot_default_outs(
        &self,
        outs: &[(String, Expression)],
    ) -> Vec<(Expression, Value)> {
        outs.iter()
            .filter_map(|(pn, dflt)| {
                self.local_stack
                    .last()
                    .and_then(|l| l.get(pn).cloned())
                    .map(|v| (dflt.clone(), v))
            })
            .collect()
    }

    /// §13.5.3: copy a method's default-bound outputs back to their defaults,
    /// which name variables of the METHOD's scope (`output int o = p` is this
    /// object's `p`): applied while `this` is still current.
    #[cold]
    #[inline(never)]
    pub(super) fn apply_default_outs(&mut self, wb: Vec<(Expression, Value)>, cname: &str) {
        self.class_context_stack.push(Some(cname.to_string()));
        for (dflt, v) in wb {
            self.assign_value(&dflt, &v);
        }
        self.class_context_stack.pop();
    }

    /// §13.5.3: whether an omitted `output`/`inout` formal's default names
    /// storage the formal can be copied back to: a variable, a select or
    /// member of one, or a concatenation of those.
    pub(super) fn is_lvalue_expr(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Ident(_) => true,
            ExprKind::Index { expr, .. }
            | ExprKind::RangeSelect { expr, .. }
            | ExprKind::MemberAccess { expr, .. } => Self::is_lvalue_expr(expr),
            ExprKind::Concatenation(parts) => {
                !parts.is_empty() && parts.iter().all(Self::is_lvalue_expr)
            }
            _ => false,
        }
    }

    /// §13.5.3: normalization fills omitted named/positional slots from
    /// defaults, but their evaluation still belongs to the declaration scope.
    /// Constructed only for calls that actually require normalization.
    pub(super) fn supplied_argument_mask(
        ports: &[crate::ast::decl::FunctionPort],
        args: &[Expression],
    ) -> Vec<bool> {
        let mut supplied = vec![false; ports.len()];
        let mut pos = 0;
        for arg in args {
            match &arg.kind {
                ExprKind::NamedArg { name, expr } => {
                    if let Some(i) = ports.iter().position(|p| p.name.name == name.name) {
                        supplied[i] = expr.is_some();
                    }
                }
                ExprKind::Empty => pos += 1,
                _ => {
                    if let Some(slot) = supplied.get_mut(pos) {
                        *slot = true;
                    }
                    pos += 1;
                }
            }
        }
        supplied
    }

    pub(crate) fn normalize_call_args(
        ports: &[crate::ast::decl::FunctionPort],
        args: &[Expression],
    ) -> Option<Vec<Expression>> {
        // Normalize when there are named args (`.name(expr)`) or positional gaps
        // (`f(a, , c)` — §13.5.3 default arguments). An omitted slot (`Empty`)
        // must be filled from the formal's default, not evaluated as 0.
        if !args
            .iter()
            .any(|a| matches!(a.kind, ExprKind::NamedArg { .. } | ExprKind::Empty))
        {
            return None;
        }
        let mut bound: Vec<Option<Expression>> = vec![None; ports.len()];
        let mut pos = 0usize;
        for a in args {
            if let ExprKind::NamedArg { name, expr } = &a.kind {
                if let Some(i) = ports.iter().position(|p| p.name.name == name.name) {
                    if let Some(e) = expr {
                        bound[i] = Some((**e).clone());
                    }
                }
            } else if matches!(a.kind, ExprKind::Empty) {
                // Omitted positional argument: leave the slot unbound so it is
                // filled from the formal's default below; still consume a
                // position so later actuals align with their formals.
                pos += 1;
            } else if pos < ports.len() {
                bound[pos] = Some(a.clone());
                pos += 1;
            }
        }
        // Everything up to the last bound formal must be supplied; a gap is
        // filled from the formal's default.
        let last = bound
            .iter()
            .enumerate()
            .filter(|(i, b)| b.is_some() || ports[*i].default.is_some())
            .map(|(i, _)| i)
            .max()?;
        let zero = || {
            Expression::new(
                ExprKind::Number(crate::ast::expr::NumberLiteral::Integer {
                    size: None,
                    signed: false,
                    base: crate::ast::expr::NumberBase::Decimal,
                    value: "0".to_string(),
                    cached_val: std::cell::Cell::new(None),
                }),
                crate::ast::Span::dummy(),
            )
        };
        let mut out = Vec::with_capacity(last + 1);
        for (i, p) in ports.iter().enumerate().take(last + 1) {
            out.push(match bound[i].take() {
                Some(e) => e,
                None => p.default.clone().unwrap_or_else(zero),
            });
        }
        Some(out)
    }

    ///  §6.21: build the shared persistent-cell key for a `static` subroutine
    /// local, mirroring the key constructed at its declaration site (§8.25/
    /// §6.21 specialization-aware prefixing). `None` when no sync frame that
    /// DECLARES this name is open. The key's subroutine is the routine whose
    /// body declared the static (found by scanning the open sync frames for
    /// the one that registered `nm`) — NOT the innermost frame, which may be a
    /// nested helper (e.g. a timing `wait_for` wrapper around a revived task
    /// body) that merely surrounds the read/write without declaring `nm`.
    /// The persistent-store key an NBA must target when its lvalue is a
    /// `static` subroutine local. Resolved while the declaring frame is still
    /// current; see `NbaEntry::static_key`.
    pub(super) fn nba_static_key_for_lvalue(&self, lvalue: &Expression) -> Option<String> {
        if self.in_const_param_eval || self.static_local_names.is_empty() {
            return None;
        }
        let ExprKind::Ident(h) = &lvalue.kind else {
            return None;
        };
        if h.path.len() != 1 || !h.path[0].selects.is_empty() {
            return None;
        }
        let n = &h.path[0].name.name;
        if !self.static_local_names.contains(n.as_str()) {
            return None;
        }
        let k = self.static_local_key_for(n)?;
        if self.static_local_keys.contains(&k) {
            Some(k)
        } else {
            None
        }
    }

    pub(super) fn static_local_key_for(&self, nm: &str) -> Option<String> {
        // §13.4.2: a `static` local is ONE cell shared across calls of ITS OWN
        // subroutine — and nothing else. Only the CURRENT frame may claim the
        // name. Scanning outward (past frames that do not declare it) bound a
        // callee's ordinary automatic local to an ancestor's same-named
        // static: the callee then read the ancestor's cell instead of its own
        // initialiser AND corrupted that cell on write, compounding across
        // calls. Names like `cnt`/`i`/`idx` collide constantly, so nothing
        // about such a design looks wrong.
        //
        // The frame stores the key computed when the declaration executed;
        // reuse it rather than re-deriving one here, which also removes a
        // per-read `format!` from this path and cannot drift when the class /
        // specialization context differs between declaration and use.
        self.static_local_syncs
            .last()?
            .1
            .iter()
            .find(|(n, _)| n == nm)
            .map(|(_, key)| key.clone())
    }

    /// §6.21/§11.5.1: the `static` local of the current subroutine that a
    /// select or member write changes in place, and the key of its shared
    /// cell: `a` in `a[i]`, `a[7:0]`, `a[i*8 +: 8]`, `a.f` or `a.f[1:0]`.
    /// None for a write to the whole
    /// variable, which reaches its shared cell by itself, and for any other
    /// target, including a variable named like another subroutine's static
    /// or a block variable that hides one.
    pub(super) fn static_select_target<'a>(
        &self,
        lhs: &'a Expression,
    ) -> Option<(&'a str, String)> {
        let mut e = lhs;
        let mut sel = false;
        loop {
            match &e.kind {
                ExprKind::Index { expr, .. }
                | ExprKind::RangeSelect { expr, .. }
                | ExprKind::MemberAccess { expr, .. } => {
                    e = expr;
                    sel = true;
                }
                ExprKind::Paren(x) => e = x,
                _ => break,
            }
        }
        let ExprKind::Ident(h) = &e.kind else {
            return None;
        };
        let seg = h.path.first()?;
        if h.root.is_some() || !(sel || h.path.len() > 1 || !seg.selects.is_empty()) {
            return None;
        }
        let n = seg.name.name.as_str();
        if !n
            .as_bytes()
            .first()
            .is_some_and(|&b| self.static_local_first[b as usize])
        {
            return None;
        }
        Some((n, self.packed_local_static_key(n)?))
    }

    /// Are the indices of the select target `e` literals and variable reads
    /// alone, so that evaluating them cannot write a variable?
    pub(super) fn select_indices_plain(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Index { expr, index } => {
                Self::init_is_plain(index) && Self::select_indices_plain(expr)
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                Self::init_is_plain(left)
                    && Self::init_is_plain(right)
                    && Self::select_indices_plain(expr)
            }
            ExprKind::MemberAccess { expr, .. } | ExprKind::Paren(expr) => {
                Self::select_indices_plain(expr)
            }
            _ => Self::init_is_plain(e),
        }
    }

    /// The select target `e` with each index that is not plain (see
    /// `select_indices_plain`) evaluated once, in order, and replaced by its
    /// value (§11.4.1). An index that is x/z or does not fit in 64 bits
    /// selects nothing in a vector (§11.5.1): it sets `bad`, for the caller
    /// to discard the write, and the indices after it still run.
    pub(super) fn freeze_select_indices(&mut self, e: &Expression, bad: &mut bool) -> Expression {
        let index = |sim: &mut Self, x: &Expression, bad: &mut bool| -> Box<Expression> {
            if Self::init_is_plain(x) {
                return Box::new(x.clone());
            }
            let v = sim.eval_expr(x);
            let Some(n) = v.to_i64().filter(|_| !v.has_xz()) else {
                *bad = true;
                return Box::new(x.clone());
            };
            Box::new(Expression::new(
                ExprKind::Number(NumberLiteral::Integer {
                    size: Some(64),
                    signed: true,
                    base: NumberBase::Decimal,
                    value: n.to_string(),
                    cached_val: Cell::new(Some((n as u64, 0u64, 64u32))),
                }),
                x.span,
            ))
        };
        let kind = match &e.kind {
            ExprKind::Index { expr, index: i } => ExprKind::Index {
                expr: Box::new(self.freeze_select_indices(expr, bad)),
                index: index(self, i, bad),
            },
            ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } => ExprKind::RangeSelect {
                expr: Box::new(self.freeze_select_indices(expr, bad)),
                kind: *kind,
                left: index(self, left, bad),
                right: index(self, right, bad),
            },
            ExprKind::MemberAccess { expr, member } => ExprKind::MemberAccess {
                expr: Box::new(self.freeze_select_indices(expr, bad)),
                member: member.clone(),
            },
            ExprKind::Paren(x) => return self.freeze_select_indices(x, bad),
            _ => return e.clone(),
        };
        Expression::new(kind, e.span)
    }

    /// §6.21: copy the `static` local `name` of the current subroutine
    /// between its frame copy and its live shared cell `key`: into the cell after
    /// a write in place (`store`), out of it before one (an NBA or another
    /// activation may have written the cell since the copy was taken). A
    /// fallback frame's copy comes from a compiled method's register, which
    /// is newer than the cell, so it is never overwritten.
    pub(super) fn static_cell_sync(&mut self, name: &str, key: &str, store: bool) {
        let depth = self.local_stack.len();
        if !store && depth > 0 && self.fb_frame_depths.last() == Some(&(depth - 1)) {
            return;
        }
        let Some(copy) = self.local_stack.last_mut().and_then(|f| f.get_mut(name)) else {
            return;
        };
        if !store {
            if let Some(v) = self.static_local_vars.get(key) {
                *copy = v.clone();
            }
            return;
        }
        match self.static_local_vars.get_mut(key) {
            Some(v) => *v = copy.clone(),
            None => {
                self.static_local_vars.insert(key.to_string(), copy.clone());
            }
        }
    }

    /// A `static_local_syncs` frame name: a pooled buffer holding `name`.
    pub(super) fn sync_frame_name(&mut self, name: &str) -> String {
        let mut s = self.sync_name_pool.pop().unwrap_or_default();
        s.clear();
        s.push_str(name);
        s
    }

    /// §6.21: on subroutine return, copy each `static` local's final value from
    /// the (about-to-be-popped) local frame back into the persistent store, then
    /// close this call's sync frame. Called before `local_stack` is popped.
    pub(super) fn sync_static_locals(&mut self) {
        if let Some((name, syncs)) = self.static_local_syncs.pop() {
            if self.sync_name_pool.len() < 64 {
                self.sync_name_pool.push(name);
            }
            for (local_name, key) in syncs {
                // Live-sourced statics keep `static_local_vars` authoritative
                // (literal write-through + read-through); writing back the
                // recursion-window snapshot here would clobber the committed
                // value with a stale frame copy.
                if self.static_local_names.contains(&local_name) {
                    continue;
                }
                if let Some(v) = self.frame_get(&local_name) {
                    self.static_local_vars.insert(key, v.clone());
                }
            }
        }
    }

    /// §26.3: the package a BARE call from inside a package subroutine runs in.
    /// A sibling subroutine of the SAME package keeps that package as the scope
    /// its free names resolve in; anything else is left to its own declaring
    /// scope. Costs nothing unless some package-scope name is contested.
    pub(super) fn bare_call_pkg_scope(&self, name: &str, path_len: usize) -> Option<String> {
        if path_len != 1 || self.pkg_ambiguous_names.is_empty() {
            return None;
        }
        let pkg = self.pkg_scope_stack.last()?.as_ref()?;
        if self
            .module
            .pkg_subr_owner
            .get(name)
            .is_some_and(|o| o == pkg)
        {
            return Some(pkg.clone());
        }
        None
    }

    /// §18.16: draw one `randcase` branch with probability weight_i /
    /// sum(weights). Zero-weight branches are never taken; an all-zero (or
    /// empty) list selects nothing.
    pub(super) fn randcase_pick(&mut self, items: &[(Expression, Statement)]) -> Option<usize> {
        use rand::Rng;
        let weights: Vec<u64> = items
            .iter()
            .map(|(w, _)| self.eval_expr(w).to_u64().unwrap_or(0))
            .collect();
        let total: u64 = weights.iter().sum();
        if total == 0 {
            return None;
        }
        let mut draw = self.cur_rng().gen_range(0..total);
        for (i, w) in weights.iter().enumerate() {
            if *w == 0 {
                continue;
            }
            if draw < *w {
                return Some(i);
            }
            draw -= *w;
        }
        None
    }

    /// §12.7.2: a repeat count that is x/z or NEGATIVE executes zero times.
    pub(super) fn repeat_count(&mut self, count: &Expression) -> u64 {
        let v = self.eval_expr(count);
        if v.has_xz() {
            return 0;
        }
        // §12.7.2/§6.12.2: a real count converts to an integer by rounding
        // (ties away from zero); read as raw bits, `repeat (10.4)` spun ~2^62
        // times.
        if v.is_real {
            return v.to_f64().round().max(0.0) as u64;
        }
        if v.is_signed {
            v.to_i64().unwrap_or(0).max(0) as u64
        } else {
            v.to_u64().unwrap_or(0)
        }
    }

    /// Is `name` a module-scope function with no observable side effects?
    ///
    /// "Pure" here means: calling it, or not calling it, cannot be told apart
    /// from outside. It may read anything; it may not WRITE anything a later
    /// read could see, and it may not produce output. The walker whitelists —
    /// every construct it does not explicitly understand makes the function
    /// impure, so a wrong answer costs a missed optimization, never a wrong
    /// simulation.
    ///
    /// Impure: `ref`/`output`/`inout` formals (the caller sees the write), a
    /// write to anything not declared inside the function, any system task or
    /// task call, timing controls, assertions, and any call to a function that
    /// is itself impure. Recursion is treated as impure rather than solved.
    pub(super) fn fn_is_pure(&mut self, name: &str) -> bool {
        if let Some(&p) = self.fn_pure_cache.get(name) {
            return p;
        }
        // Guard against recursion: assume impure while we are deciding, so a
        // self- or mutually-recursive function settles on `false` instead of
        // looping.
        self.fn_pure_cache.insert(name.to_string(), false);
        let Some(fd) = self.fn_decl_rc(name) else {
            return false;
        };

        // Formals that let a write escape make it impure outright.
        for port in &fd.ports {
            if !matches!(port.direction, crate::ast::types::PortDirection::Input) {
                return false;
            }
        }

        // Names a write may legally target: the return value and anything
        // declared inside (formals included).
        let mut locals: HashSet<String> = HashSet::default();
        locals.insert(fd.name.name.name.clone());
        for port in &fd.ports {
            locals.insert(port.name.name.clone());
        }
        Self::collect_decl_names(&fd.items, &mut locals);

        let pure = self.stmts_are_pure(&fd.items, &locals);
        self.fn_pure_cache.insert(name.to_string(), pure);
        pure
    }

    /// Variable names declared anywhere inside a statement list.
    pub(super) fn collect_decl_names(stmts: &[Statement], out: &mut HashSet<String>) {
        for st in stmts {
            match &st.kind {
                StatementKind::VarDecl { declarators, .. } => {
                    for d in declarators {
                        out.insert(d.name.name.clone());
                    }
                }
                StatementKind::SeqBlock { stmts, .. } => Self::collect_decl_names(stmts, out),
                StatementKind::If {
                    then_stmt,
                    else_stmt,
                    ..
                } => {
                    Self::collect_decl_names(std::slice::from_ref(then_stmt), out);
                    if let Some(e) = else_stmt {
                        Self::collect_decl_names(std::slice::from_ref(e), out);
                    }
                }
                StatementKind::Case { items, .. } => {
                    for it in items {
                        Self::collect_decl_names(std::slice::from_ref(&it.stmt), out);
                    }
                }
                StatementKind::For { body, .. }
                | StatementKind::While { body, .. }
                | StatementKind::Repeat { body, .. }
                | StatementKind::Foreach { body, .. } => {
                    Self::collect_decl_names(std::slice::from_ref(body), out)
                }
                _ => {}
            }
        }
    }

    pub(super) fn stmts_are_pure(&mut self, stmts: &[Statement], locals: &HashSet<String>) -> bool {
        stmts.iter().all(|st| self.stmt_is_pure(st, locals))
    }

    pub(super) fn stmt_is_pure(&mut self, st: &Statement, locals: &HashSet<String>) -> bool {
        match &st.kind {
            // A declaration-owned static can be written by this call;
            // treating it as a per-call local hides observable side effects.
            StatementKind::VarDecl {
                lifetime: Some(crate::ast::types::Lifetime::Static),
                ..
            } => false,
            StatementKind::Null | StatementKind::VarDecl { .. } => true,
            StatementKind::Break | StatementKind::Continue => true,
            StatementKind::Return(e) => e.as_ref().is_none_or(|e| self.expr_is_pure(e, locals)),
            StatementKind::BlockingAssign { lvalue, rvalue } => {
                Self::lvalue_is_local(lvalue, locals)
                    && self.expr_is_pure(lvalue, locals)
                    && self.expr_is_pure(rvalue, locals)
            }
            StatementKind::SeqBlock { stmts, .. } => self.stmts_are_pure(stmts, locals),
            StatementKind::If {
                condition,
                then_stmt,
                else_stmt,
                ..
            } => {
                self.expr_is_pure(condition, locals)
                    && self.stmt_is_pure(then_stmt, locals)
                    && else_stmt
                        .as_ref()
                        .is_none_or(|e| self.stmt_is_pure(e, locals))
            }
            StatementKind::Case { expr, items, .. } => {
                self.expr_is_pure(expr, locals)
                    && items.iter().all(|it| {
                        it.patterns.iter().all(|e| self.expr_is_pure(e, locals))
                            && self.stmt_is_pure(&it.stmt, locals)
                    })
            }
            StatementKind::For {
                condition,
                step,
                body,
                ..
            } => {
                condition
                    .as_ref()
                    .is_none_or(|c| self.expr_is_pure(c, locals))
                    && step.iter().all(|e| self.expr_is_pure(e, locals))
                    && self.stmt_is_pure(body, locals)
            }
            StatementKind::While { condition, body }
            | StatementKind::DoWhile { condition, body } => {
                self.expr_is_pure(condition, locals) && self.stmt_is_pure(body, locals)
            }
            StatementKind::Repeat { count, body } => {
                self.expr_is_pure(count, locals) && self.stmt_is_pure(body, locals)
            }
            // Everything else — timing, tasks, assertions, forks, triggers,
            // nonblocking assigns — is either observable or not understood.
            _ => false,
        }
    }

    /// A write is safe only when its ultimate base name is declared inside the
    /// function. `a.b[i]` is judged by `a`.
    pub(super) fn lvalue_is_local(lv: &Expression, locals: &HashSet<String>) -> bool {
        match &lv.kind {
            ExprKind::Ident(h) => h.path.len() == 1 && locals.contains(&h.path[0].name.name),
            ExprKind::Index { expr, .. } | ExprKind::MemberAccess { expr, .. } => {
                Self::lvalue_is_local(expr, locals)
            }
            ExprKind::Paren(e) => Self::lvalue_is_local(e, locals),
            _ => false,
        }
    }

    pub(super) fn expr_is_pure(&mut self, e: &Expression, locals: &HashSet<String>) -> bool {
        match &e.kind {
            ExprKind::Number(_) | ExprKind::StringLiteral(_) | ExprKind::Ident(_) => true,
            ExprKind::Paren(x) | ExprKind::Unary { operand: x, .. } => self.expr_is_pure(x, locals),
            ExprKind::Binary { left, right, .. } => {
                self.expr_is_pure(left, locals) && self.expr_is_pure(right, locals)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.expr_is_pure(condition, locals)
                    && self.expr_is_pure(then_expr, locals)
                    && self.expr_is_pure(else_expr, locals)
            }
            ExprKind::Concatenation(xs) => xs.iter().all(|x| self.expr_is_pure(x, locals)),
            ExprKind::Replication { count, exprs } => {
                self.expr_is_pure(count, locals)
                    && exprs.iter().all(|x| self.expr_is_pure(x, locals))
            }
            ExprKind::Index { expr, index } => {
                self.expr_is_pure(expr, locals) && self.expr_is_pure(index, locals)
            }
            ExprKind::MemberAccess { expr, .. } => self.expr_is_pure(expr, locals),
            // A nested call is fine only if it is itself pure. A system call
            // ($display, $random, $time) never is.
            ExprKind::Call { func, args } => {
                let Some(fname) = Self::call_target_name(func) else {
                    return false;
                };
                args.iter().all(|a| self.expr_is_pure(a, locals)) && self.fn_is_pure(&fname)
            }
            _ => false,
        }
    }

    /// Name of a pure, module-scope function this expression calls directly,
    /// if it is one. Parentheses are transparent.
    pub(super) fn pure_call_name(&mut self, e: &Expression) -> Option<String> {
        let ExprKind::Call { func, .. } = &Self::peel_parens(e).kind else {
            return None;
        };
        let name = Self::call_target_name(func)?;
        self.fn_is_pure(&name).then_some(name)
    }

    /// Does `v` absorb the other operand of this op? `0` absorbs `&`, all-ones
    /// absorbs `|` — in both cases for EVERY value the other side could take,
    /// X and Z included (`0 & x` is 0, `1 | x` is 1). X or Z anywhere in the
    /// mask itself disqualifies it. Limited to <=64 bits, the width at which
    /// `raw_bits` sees every bit.
    pub(super) fn absorbs_operand(v: &Value, want_ones: bool) -> bool {
        if v.width == 0 || v.width > 64 {
            return false;
        }
        let (bits, xz) = v.raw_bits();
        if xz != 0 {
            return false;
        }
        let mask = if v.width == 64 {
            u64::MAX
        } else {
            (1u64 << v.width) - 1
        };
        if want_ones {
            bits & mask == mask
        } else {
            bits & mask == 0
        }
    }

    /// Strip redundant parentheses so a wrapped call is still recognised.
    pub(super) fn peel_parens(e: &Expression) -> &Expression {
        let mut cur = e;
        while let ExprKind::Paren(inner) = &cur.kind {
            cur = inner;
        }
        cur
    }

    /// Bare, unqualified name of a call target; `None` for anything dotted or
    /// computed, which this analysis does not attempt to resolve.
    pub(super) fn call_target_name(name: &Expression) -> Option<String> {
        match &name.kind {
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                Some(h.path[0].name.name.clone())
            }
            _ => None,
        }
    }
}
