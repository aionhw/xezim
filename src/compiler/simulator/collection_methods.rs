//! Simulator collection methods.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// If `e` names a plain object-typed lvalue holding a live class handle,
    /// returns that handle; otherwise `None`. Used by `std::randomize(obj)`
    /// to dispatch to the object's own randomize rather than scalar-drawing
    /// into the pointer. Scalar variables are excluded even when their value
    /// happens to equal a valid handle: only a declared class-typed operand
    /// is treated as `obj.randomize()`.
    pub(super) fn plain_handle_operand(&mut self, e: &Expression) -> Option<usize> {
        let root = Self::expr_leaf_name(e)?;
        let obj_typed = self
            .module
            .var_decl_types
            .get(&root)
            .is_some_and(|dt| self.plain_class_ref(dt).is_some());
        if !obj_typed {
            return None;
        }
        let h = self.eval_expr(e).to_u64().unwrap_or(0) as usize;
        if h == 0 {
            return None;
        }
        let live = self
            .heap
            .get(h)
            .and_then(|x| x.as_ref())
            .map(|i| !i.class_name.is_empty())
            .unwrap_or(false);
        live.then_some(h)
    }

    /// Evaluate an array reduction (`sum`/`product`/`min`/`max`/`and`/`or`/`xor`)
    /// with a `with (item...)` expression, binding `item` to each element.
    /// LRM §7.12.2: in-place sort/rsort/unique on `arr` with the
    /// per-element key expression `filter` (binds `item` to each
    /// element). Mirrors `reduce_with`'s `item` plumbing.
    /// The unpacked-struct type of `e` IF it names a class-property struct
    /// (members stored in the heap). `None` for non-class-property operands
    /// (those are handled by the signal-namespace struct path).
    pub(super) fn class_prop_struct_of(
        &mut self,
        e: &Expression,
    ) -> Option<crate::ast::types::StructUnionType> {
        if self.no_class_objects()
            || !Self::receiver_prop_name(e).is_some_and(|p| self.struct_prop_name_possible(p))
        {
            return None;
        }
        let (h, p) = self.class_prop_receiver(e)?;
        let su = self.class_prop_struct(h, &p)?;
        Self::spreads_member_wise(&su).then_some(su)
    }

    /// Declared ELEMENT width (bits) of the collection named `name`, where
    /// `name` is a collection storage key (a `<handle>#member` class property,
    /// a bare module/local name, or a static `Owner::member` key). The width
    /// is what an element STORE normalizes toward, so it is what two stored
    /// elements must be compared at: two sides of an array `==` can live in
    /// differently-sized slots (e.g. push_back'd narrow elements kept in
    /// 32-bit storage, or a value stored from a wider local) yet still hold
    /// the same logical element value. Returns `None` when the element type
    /// cannot be resolved (caller keeps the raw `Value` comparison).
    pub(super) fn coll_elem_width(&self, name: &str) -> Option<u32> {
        // Class-property collection key `<handle>#member` (drop a trailing
        // index if a full element key was passed).
        if let Some((owner, member)) = name.split_once('#') {
            if let Ok(h) = owner.parse::<usize>() {
                let base = member.split('[').next().unwrap_or(member);
                return self.class_prop_width_of(h, base);
            }
            return None;
        }
        // Bare module/local/static-owned name: `module.arrays` records the
        // element width as the 3rd tuple member.
        self.module.arrays.get(name).map(|t| t.2)
    }

    /// True when two stored collection elements compare equal as VALUES of
    /// the collection's declared element type. The stored slots may differ in
    /// width/signedness (a 32-bit sign-extended push_back slot vs an 8-bit
    /// pattern slot, a value stored from an unsigned `logic [15:0]` local vs
    /// the solver-drawn signed element): the storage metadata is not part of
    /// the LRM value. Both sides are truncated/resized to the declared element
    /// width (so a wide slot's clean extension is masked away) and compared by
    /// `Value::is_equal` (bit equality at equal width, independent of the
    /// `is_signed` flag). The caller has already drained x/z (which must make
    /// the whole comparison neither equal nor unequal); a `None` width falls
    /// back to the raw `Value` equality.
    pub(super) fn coll_values_eq(&self, a: &Value, b: &Value, ew: Option<u32>) -> bool {
        let Some(ew) = ew else {
            return a == b;
        };
        if ew == 0 {
            return true;
        }
        if a.has_unknown() || b.has_unknown() {
            // 4-state element content at the declared width: preserve the
            // caller's x/z handling (both `==` and `!=` → 0).
            return false;
        }
        a.resize(ew).is_equal(&b.resize(ew)).to_u64() == Some(1)
    }

    /// [`Self::coll_values_eq`] over `Option<Value>` operands (an element key
    /// may be unset). Missing either side makes the pair unequal; an unknown
    /// element is treated as unequal (multi-dimensional-array reference
    /// semantics collapse an x element to a non-match).
    pub(super) fn coll_values_opt_eq(
        &self,
        a: &Option<Value>,
        b: &Option<Value>,
        ew: Option<u32>,
    ) -> bool {
        match (a.as_ref(), b.as_ref()) {
            (Some(av), Some(bv)) => self.coll_values_eq(av, bv, ew),
            _ => false,
        }
    }

    /// IEEE 1800-2017 §7.2 / §11.8.1: `==`/`!=` on two dynamic-array / queue
    /// operands compares element-by-element (after a length check). Each
    /// operand is resolved to its storage name — a bare dyn-array formal /
    /// local, or a `<handle>#member` collection property — and every element
    /// is compared. Returns `None` when neither operand is a dynamic
    /// array / queue (caller falls through to the packed-scalar path).
    pub(super) fn compare_dyn_arrays(
        &mut self,
        left: &Expression,
        right: &Expression,
        want_eq: bool,
    ) -> Option<Value> {
        let ln = self.dyn_cmp_operand_name(left)?;
        let rn = self.dyn_cmp_operand_name(right)?;
        // NO same-storage shortcut: a queue holding an x element makes BOTH
        // `q == q` and `q != q` come out 0 on the reference simulator, so
        // self-comparison must run the element walk like any other pair.
        let ls = self.get_queue_size(&ln);
        let rs = self.get_queue_size(&rn);
        if ls != rs {
            return Some(Value::from_u64(if want_eq { 0 } else { 1 }, 1));
        }
        // Width for default values when an element is unset.
        let w = self
            .module
            .arrays
            .get(&ln)
            .map(|t| t.2)
            .or_else(|| self.module.arrays.get(&rn).map(|t| t.2))
            .unwrap_or(32);
        let mut equal = true;
        for i in 0..ls {
            let lv = self
                .get_signal_value_by_name(&format!("{}[{}]", ln, i))
                .unwrap_or_else(|| Value::zero(w));
            let rv = self
                .get_signal_value_by_name(&format!("{}[{}]", rn, i))
                .unwrap_or_else(|| Value::zero(w));
            if lv.has_unknown() || rv.has_unknown() {
                // Reference behavior: an x/z element makes the comparison
                // neither equal NOR unequal — both `==` and `!=` yield 0
                // (verified: `q1 == q1` and `q1 != q1` with an 8'hxx element
                // both print 0 there; returning x instead rendered `%b` as x).
                return Some(Value::from_u64(0, 1));
            }
            // Compare at the declared element width so a differing storage
            // slot (width/signedness) does not decide the pair (the packed/
            // unpacked element-table family) — bits are what matter.
            let ew = self
                .coll_elem_width(&ln)
                .or_else(|| self.coll_elem_width(&rn));
            if !self.coll_values_eq(&lv, &rv, ew) {
                equal = false;
            }
        }
        let res = if want_eq { equal } else { !equal };
        Some(Value::from_u64(if res { 1 } else { 0 }, 1))
    }

    /// Resolve an expression to a dynamic-array / queue storage name, or None
    /// if it does not name a dynamic array / queue. Handles bare formals /
    /// locals (`q` registered in `dynamic_arrays`), and `<handle>#member`
    /// collection properties (via [`Self::expr_assoc_name`]).
    pub(super) fn dyn_cmp_operand_name(&mut self, e: &Expression) -> Option<String> {
        if let ExprKind::Ident(h) = &e.kind {
            // A method-local queue under its per-call storage key.
            if let [seg] = h.path.as_slice() {
                let bare = seg.name.name.as_str();
                if seg.selects.is_empty() {
                    if let Some(uq) = self.dyn_name_lookup(bare).filter(|uq| *uq != bare) {
                        if self.module.dynamic_arrays.contains(uq) {
                            return Some(uq.to_string());
                        }
                    }
                }
            }
            // A bare-identifier dyn array / queue formal or local registered
            // in `dynamic_arrays`. Consult `this`-scoped members first so a
            // member name hidden by an unrelated outer bare registration
            // still resolves to its own `<handle>#member` storage.
            // `resolve_hier_name_static` of one segment is the name itself.
            let registered = if self.ident_shadowed_by_non_collection(h) {
                false
            } else {
                match h.path.as_slice() {
                    [seg] => self.module.dynamic_arrays.contains(seg.name.name.as_str()),
                    _ => {
                        let n = Self::resolve_hier_name_static(h, &self.module);
                        self.module.dynamic_arrays.contains(&n)
                    }
                }
            };
            if registered {
                let n = Self::resolve_hier_name_static(h, &self.module);
                if let Some(m) = self.instance_assoc_member(&n) {
                    return Some(m);
                }
                return Some(n);
            }
        }
        let an = self.expr_assoc_name(e)?;
        if self.is_associative_array(&an) {
            return None;
        }
        Some(an)
    }

    /// Resolve an expression to an ASSOCIATIVE-array storage name, or `None`
    /// if it does not name one (the caller then falls through). Mirrors
    /// [`Self::dyn_cmp_operand_name`] but for associative arrays.
    pub(super) fn assoc_cmp_operand_name(&mut self, e: &Expression) -> Option<String> {
        let an = self.expr_assoc_name(e)?;
        if !self.is_associative_array(&an) {
            return None;
        }
        // A bare associative-array local may live under a per-call storage
        // key shadow; follow the same lookup as the dyn path.
        if let ExprKind::Ident(h) = &e.kind {
            if let [seg] = h.path.as_slice() {
                let bare = seg.name.name.as_str();
                if seg.selects.is_empty() {
                    if let Some(uq) = self.dyn_name_lookup(bare) {
                        return Some(uq.to_string());
                    }
                }
            }
        }
        Some(an)
    }

    /// IEEE 1800-2017 §7.12 / §11.4.5: `==`/`!=` on two associative arrays
    /// compares them as order-independent sets of equal (key, value) pairs:
    /// same element count, then every key of the left must exist on the right
    /// with a value equal at the declared element width (4-state element
    /// values run the caller-style x/z ambiguity). Returns `None` when neither
    /// operand is an associative array (caller falls through to the packed
    /// scalar path). A class-property associative array previously fell
    /// through to the packed-scalar path and always evaluated to x.
    pub(super) fn compare_assoc_arrays(
        &mut self,
        left: &Expression,
        right: &Expression,
        want_eq: bool,
    ) -> Option<Value> {
        let ln = self.assoc_cmp_operand_name(left)?;
        let rn = self.assoc_cmp_operand_name(right)?;
        let lkeys = self.assoc_key_strs(&ln);
        let rkeys = self.assoc_key_strs(&rn);
        let size_eq = lkeys.len() == rkeys.len();
        let mut equal = size_eq;
        if equal {
            let rset: std::collections::HashSet<&String> = rkeys.iter().collect();
            let ew = self
                .coll_elem_width(&ln)
                .or_else(|| self.coll_elem_width(&rn));
            for k in &lkeys {
                if !rset.contains(k) {
                    equal = false;
                    break;
                }
                let lv = self
                    .get_signal_value_by_name(&format!("{}[{}]", ln, k))
                    .unwrap_or_else(|| Value::zero(ew.unwrap_or(32)));
                let rv = self
                    .get_signal_value_by_name(&format!("{}[{}]", rn, k))
                    .unwrap_or_else(|| Value::zero(ew.unwrap_or(32)));
                if lv.has_unknown() || rv.has_unknown() {
                    // An unknown element makes the comparison ambiguous: both
                    // `==` and `!=` come out 0.
                    return Some(Value::from_u64(0, 1));
                }
                if !self.coll_values_eq(&lv, &rv, ew) {
                    equal = false;
                    break;
                }
            }
        }
        let res = if want_eq { equal } else { !equal };
        Some(Value::from_u64(if res { 1 } else { 0 }, 1))
    }

    /// IEEE 1800-2017 §11.4.5 / §7.2: `==` and `!=` on unpacked structs compare
    /// them member by member. Their leaves live in separate signals, so the
    /// packed-value path compared two container signals that do not exist and
    /// always yielded X. An X in any leaf makes the result X, as for vectors.
    /// `None` when either side is not an unpacked struct with storage.
    pub(super) fn compare_unpacked_structs(
        &mut self,
        lhs: &Expression,
        rhs: &Expression,
    ) -> Option<Value> {
        // An ELEMENT of a class-property collection (`q[i] == q[j]`) keeps its
        // leaves under the instance: compare leaf by leaf through the read path.
        if let Some(su) = self
            .struct_elem_type(lhs)
            .or_else(|| self.struct_elem_type(rhs))
        {
            let (mut ll, mut rl) = (Vec::new(), Vec::new());
            self.unpacked_struct_leaves("X", lhs, &su, 0, &mut ll);
            self.unpacked_struct_leaves("X", rhs, &su, 0, &mut rl);
            let mut equal = true;
            for ((_, le, ..), (_, re, ..)) in ll.iter().zip(rl.iter()) {
                let lv = self.eval_expr(le);
                let rv = self.eval_expr(re);
                if lv.has_unknown() || rv.has_unknown() {
                    return Some(Value::new(1));
                }
                if lv.is_equal(&rv).to_u64() != Some(1) {
                    equal = false;
                }
            }
            return Some(Value::from_u64(u64::from(equal), 1));
        }
        // If EITHER operand is a class-property unpacked struct (members in
        // the heap, not the signal namespace), compare member-wise by
        // appending a member segment and evaluating through the field-read
        // paths — uniform for class-prop vs class-prop and class-prop vs
        // signal. (IEEE 1800-2023 §11.4.5, §7.2, §10.6)
        let l_cp = self.class_prop_struct_of(lhs);
        let r_cp = self.class_prop_struct_of(rhs);
        if l_cp.is_some() || r_cp.is_some() {
            let su = l_cp.or(r_cp)?;
            let mut equal = true;
            for m in &su.members {
                for md in &m.declarators {
                    let lf = Self::append_member_expr(lhs, &md.name.name);
                    let rf = Self::append_member_expr(rhs, &md.name.name);
                    let lv = self.eval_expr(&lf);
                    let rv = self.eval_expr(&rf);
                    if lv.has_unknown() || rv.has_unknown() {
                        return Some(Value::new(1)); // X
                    }
                    if lv != rv {
                        equal = false;
                    }
                }
            }
            return Some(if equal {
                Value::ones(1)
            } else {
                Value::zero(1)
            });
        }
        // Signal-namespace unpacked structs (members are separate signals):
        // flatten to leaves and compare each, so a nested struct / string
        // member is read by its own leaf signal rather than a whole-struct
        // container that does not exist. An X in any leaf makes the result X.
        let (a, b) = (self.flat_member_name(lhs)?, self.flat_member_name(rhs)?);
        // Only a member-wise struct needs its type owned.
        let su = {
            let dt = self.p_elem_type_ref(&a)?;
            match self.resolve_dt_ref(&dt) {
                DataType::Struct(su) if Self::spreads_member_wise(su) => su.clone(),
                _ => return None,
            }
        };
        if !self.struct_storage_exists(&a, &su) || !self.struct_storage_exists(&b, &su) {
            return None;
        }
        let mut equal = true;
        for sfx in self.struct_leaf_suffixes(&su) {
            let av = self.get_signal_value_by_name(&format!("{}.{}", a, sfx));
            let bv = self.get_signal_value_by_name(&format!("{}.{}", b, sfx));
            match (av, bv) {
                (Some(x), Some(y)) => {
                    if x.has_unknown() || y.has_unknown() {
                        return Some(Value::new(1)); // X
                    }
                    if x != y {
                        equal = false;
                    }
                }
                _ => return Some(Value::new(1)),
            }
        }
        // §7.2.1/§11.4.5: a DYNAMIC member compares as the collection it is —
        // same size (same keys) and equal elements.
        for sfx in self.struct_coll_suffixes(&su) {
            match self.coll_contents_eq(&format!("{}.{}", a, sfx), &format!("{}.{}", b, sfx)) {
                Some(true) => {}
                Some(false) => equal = false,
                None => return Some(Value::new(1)),
            }
        }
        Some(if equal {
            Value::ones(1)
        } else {
            Value::zero(1)
        })
    }

    /// IEEE 1800-2017 §11.4.5 / §10.9: `==`, `!=`, `===`, `!==` between an
    /// unpacked aggregate and an assignment pattern. The pattern is in an
    /// assignment-like context, so it takes the type of the other operand and
    /// the comparison runs element by element (member by member for a
    /// struct), each element against its pattern item — `index:`, `N{..}`,
    /// `default:` and nested patterns resolve exactly as for an assignment.
    /// The packed-concatenation value of the pattern compared against the
    /// array's (nonexistent) container signal and always came out 0.
    /// `None` when the other operand is not an unpacked aggregate.
    pub(super) fn compare_with_pattern(
        &mut self,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
    ) -> Option<Value> {
        let (x, items) = match (&left.kind, &right.kind) {
            (ExprKind::AssignmentPattern(_), ExprKind::AssignmentPattern(_)) => return None,
            (_, ExprKind::AssignmentPattern(it)) => (left, it),
            (ExprKind::AssignmentPattern(it), _) => (right, it),
            _ => return None,
        };
        let case = matches!(op, BinaryOp::CaseEq | BinaryOp::CaseNeq);
        let want_eq = matches!(op, BinaryOp::Eq | BinaryOp::CaseEq);
        Some(match self.pattern_cmp(x, items, case)? {
            PatCmp::Equal => Value::from_u64(want_eq as u64, 1),
            PatCmp::Unequal => Value::from_u64(!want_eq as u64, 1),
            PatCmp::Unknown => Value::new(1),
            // An x/z array element: neither equal nor unequal, as for an
            // array-to-array comparison.
            PatCmp::Neither => Value::zero(1),
        })
    }

    /// The element-wise comparison of `x` with the pattern `items`; `None`
    /// when `x` is not an unpacked array, queue or struct.
    pub(super) fn pattern_cmp(
        &mut self,
        x: &Expression,
        items: &[AssignmentPatternItem],
        case: bool,
    ) -> Option<PatCmp> {
        // Fixed unpacked array of any rank: the declared dimensions, each
        // positional item filling the LEFT bound first (§10.10).
        let fixed = self
            .fixed_array_operand(x)
            .map(|(n, lo, hi)| (n, vec![(lo, hi)]));
        let fixed = fixed.or_else(|| match &x.kind {
            ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => {
                let n = self.resolve_hier_name(h).to_string();
                (self.module.arrays_2d.contains_key(&n) || self.module.arrays_nd.contains_key(&n))
                    .then(|| self.foreach_dims(&n).map(|d| (n, d)))
                    .flatten()
            }
            _ => None,
        });
        if let Some((name, dims)) = fixed {
            let dims = match self.foreach_dims(&name) {
                Some(d) if d.len() > dims.len() => d,
                _ => dims,
            };
            let descending: Vec<bool> = match self
                .module
                .unpacked_decl_dims
                .get(&name)
                .filter(|d| d.len() == dims.len())
            {
                Some(decl) => decl.iter().map(|&(l, r)| l > r).collect(),
                None => (0..dims.len())
                    .map(|k| k == 0 && self.module.descending_arrays.contains(&name))
                    .collect(),
            };
            return Some(self.pattern_cmp_dims(x, &dims, &descending, items, case));
        }
        // Queue / dynamic array: as many elements as the pattern has items.
        if !matches!(x.kind, ExprKind::Index { .. }) {
            if let Some(name) = self.dyn_cmp_operand_name(x) {
                let n = self.get_queue_size(&name) as i64;
                if n != self.pattern_ordered(items).len() as i64
                    && !items
                        .iter()
                        .any(|it| matches!(it, AssignmentPatternItem::Default(_)))
                {
                    return Some(PatCmp::Unequal);
                }
                if n == 0 {
                    return Some(PatCmp::Equal);
                }
                return Some(self.pattern_cmp_dims(x, &[(0, n - 1)], &[false], items, case));
            }
        }
        // Unpacked struct: member by member (§10.9.2 binding rules).
        let su = self.unpacked_struct_type_of(x)?;
        let mut acc = PatCmp::Equal;
        for (name, _, _, item, _) in self.bind_struct_pattern_items(&su, items) {
            let Some(item) = item else {
                return Some(PatCmp::Unequal);
            };
            let member = Self::append_member_expr(x, &name);
            acc = acc.and(self.pattern_elem_cmp(member, item, case), true);
            if acc == PatCmp::Unequal {
                break;
            }
        }
        Some(acc)
    }

    /// One dimension of [`Self::pattern_cmp`]: `x[i]` for every index of
    /// `dims[0]` against its item; a nested pattern descends one dimension,
    /// any other item covers the whole sub-array.
    pub(super) fn pattern_cmp_dims(
        &mut self,
        x: &Expression,
        dims: &[(i64, i64)],
        descending: &[bool],
        items: &[AssignmentPatternItem],
        case: bool,
    ) -> PatCmp {
        let (lo, hi) = dims[0];
        let indices: Vec<i64> = if descending.first().copied().unwrap_or(false) {
            (lo..=hi).rev().collect()
        } else {
            (lo..=hi).collect()
        };
        let elems = self.pattern_elems(items, &indices);
        let mut acc = PatCmp::Equal;
        for (k, e) in elems.into_iter().enumerate() {
            let Some(e) = e else {
                return PatCmp::Unequal;
            };
            let mut idx = Value::from_u64(indices[k] as u64, 64);
            idx.is_signed = true;
            let elem = Expression::new(
                ExprKind::Index {
                    expr: Box::new(x.clone()),
                    index: Box::new(Expression::new(
                        ExprKind::Number(NumberLiteral::Integer {
                            size: Some(64),
                            signed: true,
                            base: NumberBase::Binary,
                            value: idx.to_bin(),
                            cached_val: Cell::new(idx.inline_bits().map(|(v, xz)| (v, xz, 64))),
                        }),
                        x.span,
                    )),
                },
                x.span,
            );
            let r = if dims.len() > 1 {
                let inner = descending.get(1..).unwrap_or(&[]);
                match &e.kind {
                    ExprKind::AssignmentPattern(sub) => {
                        self.pattern_cmp_dims(&elem, &dims[1..], inner, sub, case)
                    }
                    _ => {
                        let fill = [AssignmentPatternItem::Default(e.clone())];
                        self.pattern_cmp_dims(&elem, &dims[1..], inner, &fill, case)
                    }
                }
            } else {
                self.pattern_elem_cmp(elem, e, case)
            };
            acc = acc.and(r, false);
            if acc == PatCmp::Unequal {
                break;
            }
        }
        acc
    }

    /// One leaf of a pattern comparison: `elem == item` (`===` for a case
    /// comparison) through the ordinary operator, so string, real, struct
    /// and nested-pattern elements keep their own rules.
    pub(super) fn pattern_elem_cmp(
        &mut self,
        elem: Expression,
        item: &Expression,
        case: bool,
    ) -> PatCmp {
        // §10.9: an item is assignment-like to its ELEMENT — an integral
        // item converts to the element type first, so `-1` matches a
        // `bit [7:0]` element holding 255.
        if !matches!(item.kind, ExprKind::AssignmentPattern(_))
            && !self.operand_is_string(&elem)
            && self.unpacked_struct_type_of(&elem).is_none()
        {
            let ev = self.eval_expr(&elem);
            if !ev.is_real && ev.width > 0 {
                let iv = self.eval_expr_ctx(item, ev.width);
                if !iv.is_real {
                    let iv = iv.resize_for_assign(ev.width);
                    let r = if case {
                        ev.case_eq(&iv)
                    } else {
                        ev.is_equal(&iv)
                    };
                    return Self::pat_cmp_of(&r);
                }
            }
        }
        let span = elem.span;
        let cmp = Expression::new(
            ExprKind::Binary {
                op: if case { BinaryOp::CaseEq } else { BinaryOp::Eq },
                left: Box::new(elem),
                right: Box::new(item.clone()),
            },
            span,
        );
        let v = self.eval_expr(&cmp);
        Self::pat_cmp_of(&v)
    }

    pub(super) fn pat_cmp_of(v: &Value) -> PatCmp {
        if v.has_unknown() {
            PatCmp::Unknown
        } else if v.to_u64() == Some(1) {
            PatCmp::Equal
        } else {
            PatCmp::Unequal
        }
    }

    /// The unpacked-struct type of an operand, whichever storage holds it:
    /// a class-collection element, a class property, or member signals.
    pub(super) fn unpacked_struct_type_of(
        &mut self,
        x: &Expression,
    ) -> Option<crate::ast::types::StructUnionType> {
        if let Some(su) = self.struct_elem_type(x) {
            return Some(su);
        }
        if let Some(su) = self.class_prop_struct_of(x) {
            return Some(su);
        }
        let a = self.flat_member_name(x)?;
        let dt = self.p_elem_type_ref(&a)?;
        match self.resolve_dt_ref(&dt) {
            DataType::Struct(su) if Self::spreads_member_wise(su) => Some(su.clone()),
            _ => None,
        }
    }

    /// IEEE 1800-2023 §7.3.2/§11.9: a tagged union stores ONE member at a
    /// time. Reading the active member yields the stored value; reading any
    /// other member is a run-time error.
    pub(super) fn tagged_union_member(&mut self, base: &str, member: &str) -> Option<Value> {
        let tag = self.active_union_tag.get(base)?.clone();
        let w = self.lookup_signal_width(base).unwrap_or(32);
        if tag == member {
            Some(
                self.get_signal_value_by_name(base)
                    .unwrap_or_else(|| Value::new(w)),
            )
        } else {
            let body = format!(
                "Read of inactive tagged union member '{}.{}' (active member is '{}')",
                base, member, tag
            );
            self.emit_severity_text("Error", &body);
            Some(Value::new(w))
        }
    }

    /// IEEE 1800-2017 §7.12.1 array LOCATOR methods. They select elements and
    /// RETURN a queue — none of them modifies the source array.
    pub(super) fn is_locator_method(m: &str) -> bool {
        matches!(
            m,
            "find"
                | "find_index"
                | "find_first"
                | "find_first_index"
                | "find_last"
                | "find_last_index"
                | "min"
                | "max"
                | "unique"
                | "unique_index"
        )
    }

    /// §8.10: is this BARE name, in the CURRENT scope, a class-typed
    /// receiver holding a live object? Such a receiver dispatches its USER
    /// methods — never a collection builtin — even when an outer call
    /// frame or the module scope registered a same-named queue (UVM's
    /// `paths`/`queue` locals made `paths.size()` on a fresh uvm_queue
    /// handle report another subroutine's element count).
    pub(super) fn bare_receiver_is_class_handle(&self, name: &str) -> bool {
        // The most recent DECLARATION binding wins: a method-local bound to
        // an enum typedef (`uvm_severity s;`) is an enum value even when the
        // flat `var_class_types` map holds a class type for the same bare
        // name from another scope, and even when its small integer payload
        // matches a live heap index — that exact collision dispatched UVM's
        // severity walk to heap object #1 (a uvm_callbacks_base) whenever
        // s==1, so the report summary lost its zero-count severity rows.
        if self
            .var_typedef_types
            .get(name)
            .is_some_and(|tn| self.module.enum_members.contains_key(tn))
        {
            return false;
        }
        let statically_class_typed = self.local_class_type_of(name).is_some()
            || self.this_stack.last().copied().flatten().is_some_and(|h| {
                self.class_prop_type_name(h, name).is_some_and(|tn| {
                    self.module.classes.contains_key(&tn)
                        || tn
                            .split('#')
                            .next()
                            .is_some_and(|b| self.module.classes.contains_key(b))
                })
            })
            || (self.dyn_name_lookup(name).is_none() && self.var_class_types.contains_key(name));
        statically_class_typed
            && self
                .eval_ident_handle(name)
                .is_some_and(|h| h != 0 && self.heap.get(h).and_then(|o| o.as_ref()).is_some())
    }

    /// Resolve a locator receiver's STORAGE name (§7.12.1). The parse-level
    /// name is not the storage key for two common shapes: a subroutine-local
    /// queue lives under its per-process rename, and a class-member
    /// collection lives at `<handle>#<member>` — this-relative for the bare
    /// name, or reached through object handles for `obj.member`. Without
    /// this, a locator on a class-member queue saw size 0 and every
    /// `find*`/`unique`/`min`/`max` inside a class method returned empty
    /// (UVM's `grant_queued_locks` then mistook a REQ entry for a leading
    /// lock and granted arbitration no driver asked for).
    pub(super) fn resolve_locator_storage(&self, name: &str) -> String {
        let registered = |s: &Self, n: &str| {
            s.module.arrays.contains_key(n)
                || s.module.dynamic_arrays.contains(n)
                || s.is_associative_array(n)
        };
        // §8.10: inside a method a bare name that is a collection property
        // of `this` names the object's store, ahead of a module or package
        // collection registered under the same bare name.
        if registered(self, name) {
            if !name.contains('.')
                && self.this_stack.last().copied().flatten().is_some()
                && let Some(r) = self.instance_assoc_member(name)
            {
                return r;
            }
            return name.to_string();
        }
        if !name.contains('.') {
            if let Some(r) = self.dyn_name_lookup(name) {
                return r.to_string();
            }
            if let Some(r) = self.instance_assoc_member(name) {
                return r;
            }
            return name.to_string();
        }
        // `obj.member` / `a.b.member`: walk handles through the heap.
        let segs: Vec<&str> = name.split('.').collect();
        let mut handle: usize = match segs[0] {
            "this" | "super" => self.this_stack.last().copied().flatten().unwrap_or(0),
            head => self.eval_ident_handle(head).unwrap_or(0),
        };
        if handle == 0 {
            return name.to_string();
        }
        for seg in &segs[1..segs.len() - 1] {
            let next = self
                .heap
                .get(handle)
                .and_then(|o| o.as_ref())
                .and_then(|i| i.properties.get(*seg))
                .and_then(|v| v.to_u64())
                .unwrap_or(0) as usize;
            if next == 0 {
                return name.to_string();
            }
            handle = next;
        }
        self.handle_collection_name(handle, segs[segs.len() - 1])
            .unwrap_or_else(|| name.to_string())
    }

    /// Element storage names an array reduction walks: `obj[0]..obj[size-1]`,
    /// or an associative array's populated keys in key order (§7.8) — which
    /// are not dense indices, so a reduction over one summed nothing.
    pub(super) fn reduction_elem_names(&self, obj: &str) -> Vec<String> {
        if self.is_associative_array(obj) {
            self.assoc_top_level_keys(obj)
                .into_iter()
                .map(|k| format!("{}[{}]", obj, k))
                .collect()
        } else {
            (0..self.get_queue_size(obj))
                .map(|i| format!("{}[{}]", obj, i))
                .collect()
        }
    }

    /// Does `name` store a collection a `with`-clause reduction or sort can
    /// walk? A `<handle>#<member>` class collection counts.
    pub(super) fn is_with_receiver_storage(&self, name: &str) -> bool {
        self.module.arrays.contains_key(name)
            || self.module.dynamic_arrays.contains(name)
            || name.contains('#')
            || self.is_associative_array(name)
    }

    /// `(array, method, filter)` when `e` is a locator call on an array/queue,
    /// with or without a `with (...)` clause. `q.min()` parses as a Call, while
    /// `q.min` (no parens) parses as a 2-segment identifier.
    pub(super) fn locator_call(
        &mut self,
        e: &Expression,
    ) -> Option<(String, String, Option<Expression>, Option<String>)> {
        // `(a.find with (..))[k]` parenthesizes the whole locator; strip it.
        let mut e = e;
        while let ExprKind::Paren(inner) = &e.kind {
            e = inner;
        }
        let (inner, filter) = match &e.kind {
            ExprKind::WithClause { expr, filter } => (expr.as_ref(), Some((**filter).clone())),
            _ => (e, None),
        };
        // A custom iterator name from `find(x) with (x > 3)` — the first Call
        // argument. Without capturing it the filter's `x` is unbound and the
        // method wrongly returns an empty result.
        let mut iter_name: Option<String> = None;
        let target = match &inner.kind {
            ExprKind::Call { func, args } => {
                if let Some(a0) = args.first() {
                    if let ExprKind::Ident(h) = &a0.kind {
                        if h.path.len() == 1 && h.path[0].selects.is_empty() {
                            iter_name = Some(h.path[0].name.name.clone());
                        }
                    }
                }
                func.as_ref()
            }
            _ => inner,
        };
        // Flatten a receiver of Ident / nested-MemberAccess segments to a
        // dotted path (`s.arb_q.find` parses as MemberAccess{MemberAccess}).
        fn flatten_dotted(sim: &Simulator, e: &Expression) -> Option<String> {
            match &e.kind {
                ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => {
                    Some(sim.resolve_hier_name(h).into_owned())
                }
                ExprKind::MemberAccess { expr, member } => {
                    Some(format!("{}.{}", flatten_dotted(sim, expr)?, member.name))
                }
                _ => None,
            }
        }
        let (arr, mname) = match &target.kind {
            ExprKind::MemberAccess { expr, member } => {
                let arr = match flatten_dotted(self, expr) {
                    Some(a) => a,
                    // A receiver reached through selects — a struct's
                    // dynamic member (`fa[1].q`), an element of a fixed
                    // array of queues (`q3[1]`, `s.sp[2]`) — names its
                    // store through the collection resolver.
                    None if Self::is_locator_method(&member.name) => self.expr_assoc_name(expr)?,
                    None => return None,
                };
                (arr, member.name.clone())
            }
            // `q.find` flattens to a 2-seg Ident; `s.arb_q.find` to 3-seg.
            // The last segment is the method, the rest the receiver path.
            ExprKind::Ident(h)
                if h.path.len() >= 2 && h.path.iter().all(|s| s.selects.is_empty()) =>
            {
                // §26.3: `P::q.max()` names the package queue `q`, stored
                // under its bare name (see `flat_member_name`).
                let skip = usize::from(
                    h.path.len() >= 3
                        && self.module.packages.contains(&h.path[0].name.name)
                        && !self
                            .signal_name_to_id
                            .contains_key(h.path[0].name.name.as_str())
                        && !self.signals.contains_key(&h.path[0].name.name),
                );
                let base = h.path[skip..h.path.len() - 1]
                    .iter()
                    .map(|s| s.name.name.as_str())
                    .collect::<Vec<_>>()
                    .join(".");
                (base, h.path.last().unwrap().name.name.clone())
            }
            _ => return None,
        };
        if !Self::is_locator_method(&mname) {
            return None;
        }
        let arr = self.resolve_locator_storage(&arr);
        // A `<handle>#member` / `@local#N` result is already verified as a
        // collection by its resolver; anything else must be registered.
        if arr.contains('#')
            || self.module.arrays.contains_key(&arr)
            || self.module.dynamic_arrays.contains(&arr)
            || self.is_associative_array(&arr)
        {
            Some((arr, mname, filter, iter_name))
        } else {
            None
        }
    }

    pub(super) fn locator_indices_named(
        &mut self,
        arr: &str,
        method: &str,
        filter: Option<&Expression>,
        iter_name: Option<&str>,
    ) -> Vec<usize> {
        // §7.12.1, §7.8.2: an associative array is searched in index order,
        // its elements living at `arr[key]`; the `_index` forms yield keys.
        // With integral non-negative keys the key doubles as the position
        // every caller names an element by. The positional walk below saw an
        // associative array as empty, so every locator on one (the axi4
        // AVIP scoreboard's `find_first_index` over its address tables)
        // found nothing. String or negative keys keep the old answer.
        let order: Vec<usize> = if self.is_associative_array(arr) {
            match self
                .assoc_top_level_keys(arr)
                .iter()
                .map(|k| k.parse::<usize>().ok())
                .collect::<Option<Vec<_>>>()
            {
                Some(ks) => ks,
                None => return Vec::new(),
            }
        } else {
            (0..self.get_queue_size(arr) as usize).collect()
        };
        let size = order.len();
        if size == 0 {
            return Vec::new();
        }
        let elem_su = self.queue_elem_struct(arr);
        let pushed_frame = if self.local_stack.is_empty() {
            self.push_local_frame(HashMap::default());
            true
        } else {
            false
        };
        let saved_item = self.local_stack.last().and_then(|f| f.get("item").cloned());
        let saved_index = self
            .local_stack
            .last()
            .and_then(|f| f.get("item.index").cloned());
        let iter_index = iter_name
            .filter(|nm| *nm != "item")
            .map(|nm| format!("{}.index", nm));
        let saved_alias = self.item_alias.take();
        let saved_iter = self.locator_iter.clone();
        self.locator_iter = iter_name.unwrap_or("item").to_string();

        // §7.12/§7.2.1: a PACKED-struct element binds its fields too, so
        // `find with (item.lo > 2)` slices them out of `item`.
        let pfields = if elem_su.is_none() {
            match self.module.packed_struct_fields.get(arr) {
                Some(f) => Some(f.clone()),
                // A struct's dynamic member (`s.sp[2]`): its declared type.
                None if arr.contains('.') && self.struct_colls_possible() => {
                    self.struct_member_coll_of(arr).and_then(|(_, dt, _)| {
                        super::elaborate::packed_struct_field_layout(
                            &dt,
                            &self.module.parameters,
                            &self.module.typedefs,
                            &self.module.typedef_types,
                        )
                    })
                }
                None => None,
            }
            .filter(|f| !f.is_empty())
        } else {
            None
        };
        let mut keys: Vec<i64> = Vec::with_capacity(size);
        let mut truth: Vec<bool> = Vec::with_capacity(size);
        for &i in &order {
            let elem = format!("{}[{}]", arr, i);
            if elem_su.is_some() {
                self.item_alias = Some(elem.clone());
            }
            let v = self
                .get_signal_value_by_name(&elem)
                .unwrap_or_else(|| Value::zero(32));
            if let Some(f) = self.local_stack.last_mut() {
                // §7.12.4: `item.index` (or `<iterator>.index`) is the
                // element's index, an int; it was never bound here, so
                // `find with (item == item.index)` compared against 0.
                let mut ix = Value::from_u64(i as u64, 32);
                ix.is_signed = true;
                f.insert("item".to_string(), v.clone());
                f.insert("item.index".to_string(), ix.clone());
                if let Some(nm) = iter_name {
                    if nm != "item" {
                        f.insert(nm.to_string(), v.clone());
                    }
                }
                if let Some(k) = &iter_index {
                    f.insert(k.clone(), ix);
                }
                if let Some(fields) = &pfields {
                    let it = iter_name.unwrap_or("item");
                    for (fname, off, w) in fields {
                        let slice = v.range_select((off + w - 1) as usize, *off as usize);
                        f.insert(format!("{}.{}", it, fname), slice);
                    }
                }
            }
            match filter {
                Some(f) => {
                    let fv = self.eval_expr(f);
                    truth.push(fv.is_true());
                    keys.push(fv.to_i64().unwrap_or(0));
                }
                None => {
                    truth.push(true);
                    keys.push(v.to_i64().unwrap_or(0));
                }
            }
        }

        if let (Some(fields), Some(f)) = (&pfields, self.local_stack.last_mut()) {
            let it = iter_name.unwrap_or("item");
            for (fname, _, _) in fields {
                f.remove(&format!("{}.{}", it, fname));
            }
        }
        self.item_alias = saved_alias;
        self.locator_iter = saved_iter;
        if let Some(f) = self.local_stack.last_mut() {
            match saved_item {
                Some(v) => {
                    f.insert("item".to_string(), v);
                }
                None => {
                    f.remove("item");
                }
            }
            match saved_index {
                Some(v) => {
                    f.insert("item.index".to_string(), v);
                }
                None => {
                    f.remove("item.index");
                }
            }
            if let Some(k) = &iter_index {
                f.remove(k);
            }
        }
        if pushed_frame {
            self.pop_local_frame();
        }

        let picked: Vec<usize> = match method {
            "find" | "find_index" => (0..size).filter(|&i| truth[i]).collect(),
            "find_first" | "find_first_index" => {
                (0..size).find(|&i| truth[i]).into_iter().collect()
            }
            "find_last" | "find_last_index" => {
                (0..size).rev().find(|&i| truth[i]).into_iter().collect()
            }
            "min" => (0..size).min_by_key(|&i| keys[i]).into_iter().collect(),
            "max" => (0..size).max_by_key(|&i| keys[i]).into_iter().collect(),
            "unique" | "unique_index" => {
                let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
                (0..size).filter(|&i| seen.insert(keys[i])).collect()
            }
            _ => Vec::new(),
        };
        picked.into_iter().map(|p| order[p]).collect()
    }

    /// Write a locator result into the destination queue `dst`. The `_index`
    /// forms yield the indices themselves; the rest copy the elements
    /// (member-wise for unpacked structs).
    pub(super) fn materialize_locator(
        &mut self,
        dst: &str,
        arr: &str,
        method: &str,
        idxs: &[usize],
    ) {
        let index_form = method.ends_with("_index");
        for (i, &src_i) in idxs.iter().enumerate() {
            if index_form {
                self.set_signal_value_by_name(
                    &format!("{}[{}]", dst, i),
                    Value::from_u64(src_i as u64, 32),
                );
            } else {
                self.queue_copy_elem(arr, src_i as u64, dst, i as u64);
            }
        }
        self.set_queue_size(dst, idxs.len() as u64);
    }

    /// Will `sort_with` order by the STRING text of each element's key?
    /// `expr_is_string_valued`/`call_returns_string` resolve loop-locals only
    /// when their declared type is on record, but a `q.sort with (item.f)` /
    /// `(item.m())` binds `item` (or the declared iterator) to the collection
    /// ELEMENT, whose declared type is the collection's element class/string.
    /// Resolve the key against that type so the UVM phase-hopper shape
    /// (`succ_q.sort with (item.get_full_name())`) orders lexicographically
    /// too (§11.4.8), matching bare-string sorting.
    pub(super) fn sort_key_is_string(
        &self,
        arr: &str,
        filter: &Expression,
        iter: Option<&str>,
    ) -> bool {
        use crate::ast::expr::ExprKind;
        let is_loopvar = |expr: &Expression| {
            matches!(&expr.kind, ExprKind::Ident(h) if h.path.len() == 1
                && { let nm = &h.path[0].name.name; nm == "item" || iter.is_some_and(|it| nm == it) })
        };
        // A string collection's element is itself a string.
        let elem_is_string = self.is_string_collection(arr);
        match &filter.kind {
            // Bare `item` / declared iterator over a STRING collection.
            ExprKind::Ident(h) if h.path.len() == 1 => {
                let nm = &h.path[0].name.name;
                (nm == "item" || iter.is_some_and(|it| nm == it)) && elem_is_string
            }
            // `item.<field>` — a string-typed property of the element class.
            ExprKind::MemberAccess { expr: base, member }
                if is_loopvar(base) && !elem_is_string =>
            {
                if let Some(cls) = self.elem_class_of_arr(arr) {
                    return self.class_prop_is_string(&cls, &member.name);
                }
                false
            }
            // `item.<meth>(...)` — an element method returning `string`.
            ExprKind::Call { func, .. } if is_loopvar(func) => {
                self.sort_call_key_is_string(arr, elem_is_string, func)
            }
            ExprKind::Call { func, .. } => self.sort_call_key_is_string(arr, elem_is_string, func),
            ExprKind::Paren(inner) => self.sort_key_is_string(arr, inner, iter),
            _ => false,
        }
    }

    /// Is `prop` on `cls` (or an ancestor) a STRING-typed class property?
    /// Static/declared check (no runtime handle) — `string_properties` is
    /// populated at registration for exactly this.
    pub(super) fn class_prop_is_string(&self, cls: &str, prop: &str) -> bool {
        let mut cur = Some(cls.to_string());
        while let Some(cname) = cur {
            if let Some(cd) = self.module.classes.get(&cname) {
                if cd.string_properties.contains(prop) {
                    return true;
                }
                cur = cd.extends.clone();
            } else {
                break;
            }
        }
        // Fall back to the declared property type (typedef'd strings may not
        // be in `string_properties`).
        self.class_prop_type_named(cls, prop).as_deref() == Some("string")
    }

    pub(super) fn sort_call_key_is_string(
        &self,
        arr: &str,
        elem_is_string: bool,
        func: &Expression,
    ) -> bool {
        use crate::ast::decl::ClassMethodKind;
        use crate::ast::expr::ExprKind;
        use crate::ast::types::{DataType, SimpleType};
        let ExprKind::MemberAccess { expr: base, member } = &func.kind else {
            return false;
        };
        // Only a method on the LOOP VARIABLE is element-typed here; a method
        // on some other receiver is out of scope for a collection sort.
        if !matches!(&base.kind, ExprKind::Ident(h) if h.path.len() == 1) {
            return false;
        }
        if elem_is_string {
            return false;
        }
        let Some(cls) = self.elem_class_of_arr(arr) else {
            return false;
        };
        let mn = &member.name;
        let mut cur = Some(cls);
        while let Some(cname) = cur {
            if let Some(cd) = self.module.classes.get(&cname) {
                if let Some(cm) = cd.methods.get(mn) {
                    if let ClassMethodKind::Function(fd)
                    | ClassMethodKind::Extern(fd)
                    | ClassMethodKind::PureVirtual(fd) = &cm.kind
                    {
                        return matches!(
                            &fd.return_type,
                            DataType::Simple {
                                kind: SimpleType::String,
                                ..
                            }
                        );
                    }
                    return false;
                }
                cur = cd.extends.clone();
            } else {
                break;
            }
        }
        false
    }

    /// Resolve the DECLARED ELEMENT CLASS of a collection by its (leaf)
    /// name — the name-based `collection_element_class_named` core, so the
    /// gate path costs two str slices and no synthetic `Expression`.
    pub(super) fn elem_class_of_arr(&self, arr: &str) -> Option<String> {
        let leaf = arr.rsplit('.').next().unwrap_or(arr);
        let leaf = leaf.split('[').next().unwrap_or(leaf);
        self.collection_element_class_named(leaf)
    }

    /// The CLASS an associative collection's loop variable names, when its
    /// KEY type is a class handle — `foreach (m_successors[succ])` over
    /// `bit m_successors[uvm_phase]` binds `succ` to the KEY (§12.7.3), and
    /// the element class (`bit`) says nothing about it. Consults the
    /// scope-visible key type (frame overlay, then the module-scope
    /// `assoc_key_type_names` map) and resolves it to a registered class.
    /// Returns None unless the key type names a class.
    pub(super) fn assoc_key_class_of_arr(&self, arr: &str) -> Option<String> {
        let local_kt = self.local_typedef_type_of(&format!("{}[]", arr));
        if local_kt.as_deref() == Some("") {
            return None;
        }
        let kt = local_kt.or_else(|| self.module.assoc_key_type_names.get(arr).cloned())?;
        let cls = if self.module.classes.contains_key(&kt) {
            kt
        } else {
            // A typedef'd / specialized class key (`k_t k[c_t]`).
            self.resolve_typeref_class_name_str(&kt)?
        };
        self.module.classes.contains_key(&cls).then_some(cls)
    }

    /// Like [`Self::assoc_key_class_of_arr`], but falls back to the
    /// receiver-relative class walk — for a loop inside a class method the
    /// key type lives on the property's declaring class
    /// (`assoc_key_types` / `static_assoc_key_types`), not in any
    /// module-scope map. The anchor is the foreach receiver when the array
    /// expression names one, `this` otherwise (the same chain the §6.19.6
    /// enum-key binding above walks).
    pub(super) fn assoc_key_class_of_foreach(
        &self,
        arr: &str,
        array: &Expression,
    ) -> Option<String> {
        if let Some(cls) = self.assoc_key_class_of_arr(arr) {
            return Some(cls);
        }
        let recv = match &array.kind {
            ExprKind::Index { expr: b, .. } => &**b,
            _ => array,
        };
        let recv_handle = match &recv.kind {
            ExprKind::MemberAccess { expr: inner, .. } => self.eval_handle_expr(inner),
            ExprKind::Ident(h) if h.path.len() >= 2 => self.eval_ident_handle(&h.path[0].name.name),
            _ => None,
        };
        let handle = match recv_handle {
            Some(h) => h,
            None => self.this_stack.last().copied().flatten()?,
        };
        let mut cur = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone());
        let mut seen: HashSet<String> = HashSet::default();
        while let Some(cn) = cur {
            if !seen.insert(cn.clone()) {
                break;
            }
            let Some(cd) = self.module.classes.get(&cn) else {
                break;
            };
            if let Some(kt) = cd
                .assoc_key_types
                .get(arr)
                .or_else(|| cd.static_assoc_key_types.get(arr))
            {
                let cls = if self.module.classes.contains_key(kt) {
                    kt.clone()
                } else {
                    match self.resolve_typeref_class_name_str(kt) {
                        Some(c) => c,
                        None => return None,
                    }
                };
                return self.module.classes.contains_key(&cls).then_some(cls);
            }
            cur = cd.extends.clone();
        }
        None
    }

    /// LRM §7.12.2: `q.sort()/.rsort()/.unique() with (item.field)`.
    ///
    /// Computes each element's key, then permutes the queue by INDEX rather
    /// than by value — an unpacked-struct element has no packed value to carry
    /// around, so the old "collect values, reorder, write back" shape reordered
    /// nothing. `item` binds to the element's value for scalars and to its flat
    /// name for structs (see `item_alias`).
    pub(super) fn sort_with(
        &mut self,
        arr: &str,
        method: &str,
        filter: &Expression,
        iter: Option<&str>,
    ) {
        let size = self.get_queue_size(arr) as usize;
        if size == 0 {
            return;
        }
        let elem_su = self.queue_elem_struct(arr);
        let pushed_frame = if self.local_stack.is_empty() {
            self.push_local_frame(HashMap::default());
            true
        } else {
            false
        };
        let saved_item = self.local_stack.last().and_then(|f| f.get("item").cloned());
        let saved_alias = self.item_alias.take();
        let saved_iter = self.locator_iter.clone();
        self.locator_iter = iter.unwrap_or("item").to_string();

        // §7.12.2: `q.sort/.rsort with (item.expr)` sorts by the value of the
        // filter expression. When that expression is STRING-valued (e.g.
        // `succ_q.sort with (item.get_full_name())` in the UVM phase hopper),
        // the key's correct ordering is the CHARACTER-LEXICOGRAPHIC order
        // (SV `<` on strings §11.4.8), NOT the little-endian byte-packed i64
        // that `to_i64` would yield — the numeric encoding of "uvm" sorts
        // before "common.run" even though the string compares after. Detect
        // a string-typed filter and sort on the rendered text.
        //
        // A bare `item`, or the declared iterator, has the SAME type as the
        // collection element, so sorting a string collection with `(item)`
        // is also lexicographic even though `expr_is_string_valued` can't
        // resolve the loop-local's declared type.
        let bare_elem = match &filter.kind {
            ExprKind::Ident(h) if h.path.len() == 1 => {
                let nm = &h.path[0].name.name;
                nm == "item" || iter.is_some_and(|it| nm == it)
            }
            _ => false,
        };
        let str_keys = self.expr_is_string_valued(filter)
            || (bare_elem && self.is_string_collection(arr))
            || self.sort_key_is_string(arr, filter, iter);
        let mut keys: Vec<i64> = Vec::with_capacity(size);
        let mut skeys: Vec<String> = Vec::with_capacity(size);
        for i in 0..size {
            let elem = format!("{}[{}]", arr, i);
            if elem_su.is_some() {
                self.item_alias = Some(elem.clone());
            }
            let v = self
                .get_signal_value_by_name(&elem)
                .unwrap_or_else(|| Value::zero(32));
            if let Some(f) = self.local_stack.last_mut() {
                if let Some(nm) = iter {
                    f.insert(nm.to_string(), v.clone());
                }
                f.insert("item".to_string(), v);
            }
            let kv = self.eval_expr(filter);
            if str_keys {
                skeys.push(kv.to_sv_string());
            } else {
                keys.push(kv.to_i64().unwrap_or(0));
            }
        }

        self.item_alias = saved_alias;
        self.locator_iter = saved_iter;
        if let Some(f) = self.local_stack.last_mut() {
            if let Some(nm) = iter {
                f.remove(nm);
            }
            match saved_item {
                Some(v) => {
                    f.insert("item".to_string(), v);
                }
                None => {
                    f.remove("item");
                }
            }
        }
        if pushed_frame {
            self.pop_local_frame();
        }

        let mut order: Vec<usize> = (0..size).collect();
        match method {
            // `sort_by_key` is stable, so equal keys keep their order.
            "sort" => {
                if str_keys {
                    order.sort_by_key(|&i| skeys[i].clone());
                } else {
                    order.sort_by_key(|&i| keys[i]);
                }
            }
            "rsort" => {
                if str_keys {
                    order.sort_by_key(|&i| std::cmp::Reverse(skeys[i].clone()));
                } else {
                    order.sort_by_key(|&i| std::cmp::Reverse(keys[i]));
                }
            }
            // `unique` is a LOCATOR (§7.12.1): it returns a queue and must not
            // reorder or shrink the source. Handled by the locator path.
            _ => return,
        }

        self.queue_permute(arr, &order);

        // `unique` may shrink the queue; sort/rsort preserve its length.
        let new_size = order.len() as u64;
        if new_size != size as u64 {
            self.set_queue_size(arr, new_size);
            for i in new_size..size as u64 {
                self.signals.remove(&format!("{}[{}]", arr, i));
            }
        }
    }

    pub(super) fn reduce_with(
        &mut self,
        arr: &str,
        method: &str,
        filter: &Expression,
        iter: Option<&str>,
    ) -> Value {
        // §7.8 / §7.12.3: an associative array reduces over its populated
        // keys in key order, and `item.index` is the key.
        let assoc_keys = self
            .is_associative_array(arr)
            .then(|| self.assoc_top_level_keys(arr));
        let size = match &assoc_keys {
            Some(k) => k.len() as u64,
            None => self.get_queue_size(arr),
        };
        // Ensure a local frame exists so `item` has somewhere to live.
        // Initial-block-scope calls (no enclosing function) reach here
        // with an empty local_stack, which previously silenced every
        // `item` insert and made the filter eval as 0.
        let pushed_frame = if self.local_stack.is_empty() {
            self.push_local_frame(HashMap::default());
            true
        } else {
            false
        };
        let saved_item = self.local_stack.last().and_then(|f| f.get("item").cloned());
        let saved_index = self
            .local_stack
            .last()
            .and_then(|f| f.get("item.index").cloned());
        let iter_index = iter.map(|nm| format!("{}.index", nm));
        let mut acc: Option<i64> = None;
        // §7.12.3: with a `with` clause the result type is the type of the
        // EXPRESSION, so the accumulation wraps at that width. A 1-bit
        // predicate like `q.sum with (item > 4)` therefore counts modulo 2 —
        // accumulating in a full i64 and returning 32 bits gave the raw match
        // count (3) where the LRM requires 1.
        let mut expr_w: u32 = 0;
        let mut expr_s = false;
        for i in 0..size {
            let key = assoc_keys.as_ref().map(|k| k[i as usize].as_str());
            let elem = self
                .get_signal_value_by_name(&match key {
                    Some(k) => format!("{}[{}]", arr, k),
                    None => format!("{}[{}]", arr, i),
                })
                .unwrap_or_else(|| Value::zero(32));
            // §7.12.4: `item.index` is the element's index (an int), or the
            // key of an associative array.
            let ix = match key {
                Some(k) => match k.parse::<i128>() {
                    Ok(n) => {
                        let (kw, ks) = self.assoc_index_width_for(arr).unwrap_or((32, true));
                        let mut v = Value::from_u64(n as u64, kw.clamp(1, 64));
                        v.is_signed = ks;
                        v
                    }
                    Err(_) => Value::from_string(k),
                },
                None => {
                    let mut v = Value::from_u64(i, 32);
                    v.is_signed = true;
                    v
                }
            };
            if let Some(f) = self.local_stack.last_mut() {
                if let Some(nm) = iter {
                    f.insert(nm.to_string(), elem.clone());
                }
                if let Some(k) = &iter_index {
                    f.insert(k.clone(), ix.clone());
                }
                f.insert("item".to_string(), elem);
                f.insert("item.index".to_string(), ix);
            }
            let fv = self.eval_expr(filter);
            if expr_w == 0 {
                expr_w = fv.width.max(1);
                expr_s = fv.is_signed;
            }
            let v = fv.to_i64().unwrap_or(0);
            acc = Some(match (acc, method) {
                (None, _) => v,
                (Some(a), "sum") => a + v,
                (Some(a), "product") => a * v,
                (Some(a), "min") => a.min(v),
                (Some(a), "max") => a.max(v),
                (Some(a), "and") => a & v,
                (Some(a), "or") => a | v,
                (Some(a), "xor") => a ^ v,
                (Some(a), _) => a,
            });
        }
        // Restore the prior `item` binding (if any); drop the iterator's.
        if let Some(f) = self.local_stack.last_mut() {
            if let Some(nm) = iter {
                f.remove(nm);
            }
            if let Some(k) = &iter_index {
                f.remove(k);
            }
            match saved_item {
                Some(v) => {
                    f.insert("item".to_string(), v);
                }
                None => {
                    f.remove("item");
                }
            }
            match saved_index {
                Some(v) => {
                    f.insert("item.index".to_string(), v);
                }
                None => {
                    f.remove("item.index");
                }
            }
        }
        if pushed_frame {
            self.pop_local_frame();
        }
        // Truncate to the expression's width for the accumulating reductions;
        // min/max/and/or/xor cannot exceed an operand and need no masking.
        let w = if expr_w == 0 { 32 } else { expr_w };
        let raw = acc.unwrap_or(0) as u64;
        let out = if matches!(method, "sum" | "product") && w < 64 {
            raw & ((1u64 << w) - 1)
        } else {
            raw
        };
        // §7.12.3: the result has the type of the `with` expression: its
        // width (`$bits(a.sum() with (16'(item)))` is 16) and signedness
        // (`sb.sum() with (item) == -56` over bytes holds).
        if matches!(method, "sum" | "product" | "and" | "or" | "xor") && expr_w > 0 {
            let w = expr_w.min(64);
            let mut v = Value::from_u64(if w < 64 { out & ((1u64 << w) - 1) } else { out }, w);
            v.is_signed = expr_s;
            return v;
        }
        Value::from_u64(out, w.max(32))
    }

    /// The subroutine call a method-call receiver is reached through
    /// (`f()` in `f().m()` or `f().a.b.m()`), if the receiver is a call
    /// followed only by member selects.
    pub(super) fn recv_call_node(expr: &Expression) -> Option<&Expression> {
        match &expr.kind {
            ExprKind::Call { .. } => Some(expr),
            ExprKind::MemberAccess { expr, .. } => Self::recv_call_node(expr),
            ExprKind::Paren(inner) => Self::recv_call_node(inner),
            _ => None,
        }
    }

    /// The handle a `recv_call_node`-shaped receiver names, given the call's
    /// result: walk the member selects after the call as handle properties
    /// (0 when one is not a handle).
    pub(super) fn recv_chain_handle(&self, call_handle: usize, expr: &Expression) -> usize {
        match &expr.kind {
            ExprKind::Call { .. } => call_handle,
            ExprKind::MemberAccess { expr, member } => {
                let h = self.recv_chain_handle(call_handle, expr);
                if h == 0 {
                    return 0;
                }
                self.member_handle(h, &member.name).unwrap_or(0)
            }
            ExprKind::Paren(inner) => self.recv_chain_handle(call_handle, inner),
            _ => 0,
        }
    }

    /// True if `expr` contains a function/method/system/`new` call anywhere —
    /// such initializers may recurse or have side effects, so they are not
    /// re-evaluated at instantiation.
    pub(super) fn expr_contains_call(expr: &Expression) -> bool {
        match &expr.kind {
            ExprKind::Call { .. } | ExprKind::SystemCall { .. } => true,
            ExprKind::Binary { left, right, .. } => {
                Self::expr_contains_call(left) || Self::expr_contains_call(right)
            }
            ExprKind::Unary { operand, .. } => Self::expr_contains_call(operand),
            ExprKind::Paren(inner) => Self::expr_contains_call(inner),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::expr_contains_call(condition)
                    || Self::expr_contains_call(then_expr)
                    || Self::expr_contains_call(else_expr)
            }
            ExprKind::Index { expr, index } => {
                Self::expr_contains_call(expr) || Self::expr_contains_call(index)
            }
            ExprKind::Concatenation(parts) => parts.iter().any(Self::expr_contains_call),
            ExprKind::MemberAccess { expr, .. } => Self::expr_contains_call(expr),
            _ => false,
        }
    }

    /// Bind a queue / dynamic-array FORMAL (`int q[$]`, `int d[]`, or a
    /// typedef carrying those dims) into the call frame. Returns `None` when
    /// the formal is not a queue/dynamic array, else `Some((storage, caller))`:
    /// the key the formal's body accesses resolve to, and the caller's
    /// collection to copy back to on return (empty when nothing is copied
    /// back).
    ///
    /// IEEE 1800-2023 §13.5.1: an `input` formal is a COPY. It lives under a
    /// per-call key (`@q#N`, mapped in the frame's rename table like an
    /// automatic local), so writes in the body never reach the caller — not
    /// even when the actual has the formal's own name — and recursive or
    /// concurrent calls each own their copy. An `output` formal starts empty;
    /// `output`/`inout` copy back on return.
    ///
    /// §13.5.2: a `ref` formal is an ALIAS of the actual's storage: the
    /// formal's name maps to the actual's key, so every read and write goes
    /// to the caller's collection while the call runs (two concurrently
    /// forked tasks pushing into one `ref` queue both land), and nothing is
    /// copied back.
    ///
    /// §7.10 / §13.5: a slice actual (`q[1:$]`, `q[a+:n]`) binds exactly the
    /// slice's elements; `q[n:$]` past the end and `q[3:1]` bind none.
    pub(super) fn bind_queue_param(
        &mut self,
        pname: &str,
        dims: &[crate::ast::types::UnpackedDimension],
        arg: &Expression,
        queue_data_type: &crate::ast::types::DataType,
        dir: PortDirection,
    ) -> Option<(String, String)> {
        use crate::ast::types::UnpackedDimension;
        self.meta_note(pname);
        if !matches!(
            dims.first(),
            Some(UnpackedDimension::Queue { .. }) | Some(UnpackedDimension::Unsized(_))
        ) {
            return None;
        }
        // A queue literal / assignment pattern (`{"a", "b"}` or `'{...}`) passed
        // directly as the actual: bind each top-level element as one queue
        // entry.
        let literal_elems: Option<Vec<&Expression>> = match &arg.kind {
            ExprKind::Concatenation(parts) => Some(parts.iter().collect()),
            ExprKind::AssignmentPattern(items) => Some(items.iter().map(|it| it.expr()).collect()),
            _ => None,
        };
        // When the queue's element type is a struct, copy the struct's
        // packed_struct_fields layout to the queue's name so that
        // `driver[i].field` MemberAccess on queue elements works.
        // Record the formal's ELEMENT type so the callee's `push_back` of an
        // unpacked struct writes member leaves (and the writeback copies them
        // back) — without it a `ref rec_t items[$]` came back with the right
        // SIZE and every member blank.
        self.module
            .var_decl_types
            .insert(pname.to_string(), queue_data_type.clone());
        let resolved_dt = Self::resolve_type_ref(queue_data_type, &self.module.typedef_types);
        if let crate::ast::types::DataType::Struct(su) = resolved_dt {
            if !su.packed {
                // Record the element typedef name so MemberAccess reads can
                // recover per-field `is_real` (real fields) via struct_field_layout.
                if let crate::ast::types::DataType::TypeReference { name: tn, .. } = queue_data_type
                {
                    self.var_typedef_types
                        .insert(pname.to_string(), tn.name.name.clone());
                }
                // Collect struct fields with cumulative offsets. SV packs
                // struct members with the LAST member at the LSB.
                let mut offset: u32 = 0;
                let mut fields: Vec<(String, u32, u32)> = Vec::new();
                for m in su.members.iter().rev() {
                    let fw = resolve_type_width(
                        &m.data_type,
                        Some(&self.module.parameters),
                        Some(&self.module.typedefs),
                    );
                    for d in &m.declarators {
                        fields.push((d.name.name.clone(), offset, fw));
                        offset += fw;
                    }
                }
                self.module
                    .packed_struct_fields
                    .insert(pname.to_string(), fields);
            }
        }
        if let Some(parts) = literal_elems {
            let key = self.formal_dyn_key(pname, queue_data_type);
            self.module.arrays.insert(key.clone(), (0, -1, 32));
            self.module.dynamic_arrays.insert(key.clone());
            // An element that is itself a `'{..}` pattern (a dynamic array of
            // UNPACKED structs) is scattered member-wise, not evaluated.
            for (j, part) in parts.iter().enumerate() {
                if matches!(part.kind, ExprKind::AssignmentPattern(_)) {
                    self.assign_pattern_or_leaf(&format!("{}[{}]", key, j), queue_data_type, part);
                    continue;
                }
                let v = self.eval_expr(part);
                self.set_signal_value_by_name(&format!("{}[{}]", key, j), v);
            }
            self.set_queue_size(&key, parts.len() as u64);
            return Some((key, String::new()));
        }
        if let Some((vals, w)) = self.collection_slice_values(arg) {
            let key = self.formal_dyn_key(pname, queue_data_type);
            self.module.arrays.insert(key.clone(), (0, -1, w));
            self.module.dynamic_arrays.insert(key.clone());
            let n = vals.len() as u64;
            for (j, v) in vals.into_iter().enumerate() {
                self.set_signal_value_by_name(&format!("{}[{}]", key, j), v);
            }
            self.set_queue_size(&key, n);
            return Some((key, String::new()));
        }
        let cname = self.queue_actual_storage(arg)?;
        if matches!(dir, PortDirection::Ref) {
            // §13.5.2: alias — the formal resolves to the actual's storage.
            self.map_formal_dyn(pname, &cname);
            return Some((cname, String::new()));
        }
        let size = if matches!(dir, PortDirection::Output) {
            0
        } else {
            self.get_queue_size(&cname)
        };
        let w = self.module.arrays.get(&*cname).map(|t| t.2).unwrap_or(32);
        let key = self.formal_dyn_key(pname, queue_data_type);
        // A queue of unpacked structs keeps its members as separate leaves
        // (`q[0].addr`); those are copied element-wise below.
        let elem_struct = (cname != key)
            .then(|| self.queue_elem_struct(&cname))
            .flatten();
        let vals: Vec<Value> = if elem_struct.is_some() {
            Vec::new()
        } else {
            (0..size)
                .map(|j| {
                    self.get_signal_value_by_name(&format!("{}[{}]", cname, j))
                        .unwrap_or_else(|| Value::zero(w))
                })
                .collect()
        };
        self.module.arrays.insert(key.clone(), (0, -1, w));
        self.module.dynamic_arrays.insert(key.clone());
        if let Some(su) = &elem_struct {
            for j in 0..size {
                let (src, dst) = (format!("{}[{}]", cname, j), format!("{}[{}]", key, j));
                self.copy_unpacked_struct(&dst, &src, su);
            }
        }
        for (j, v) in vals.into_iter().enumerate() {
            self.set_signal_value_by_name(&format!("{}[{}]", key, j), v);
        }
        self.set_queue_size(&key, size);
        Some((key, cname))
    }

    /// The per-call storage key of a queue / dynamic-array / associative
    /// formal (see `bind_queue_param`): an `@name#N` key like an automatic
    /// local's, entered in the callee frame's rename table by
    /// `commit_formal_dyn` and torn down with the frame.
    pub(super) fn formal_dyn_key(
        &mut self,
        pname: &str,
        dt: &crate::ast::types::DataType,
    ) -> String {
        let id = self.next_dyn_id;
        self.next_dyn_id += 1;
        let mut key = String::with_capacity(pname.len() + 12);
        key.push('@');
        key.push_str(pname);
        key.push('#');
        Self::push_i64(&mut key, id as i64);
        self.map_formal_dyn(pname, &key);
        // The element type under the per-call key too: a queue of unpacked
        // structs spreads its members by it, and a class-element queue
        // constructs its `push_back(new(..))` elements by it. A plain
        // integral / real / enum element needs no entry of its own
        // (`p_elem_type_ref` maps the key back to the formal's), which spares
        // a type clone on every call.
        let plain = matches!(
            Self::resolve_type_ref_borrowed(dt, &self.module.typedef_types),
            DataType::IntegerAtom { .. }
                | DataType::IntegerVector { .. }
                | DataType::Real { .. }
                | DataType::Enum(_)
        );
        if !plain {
            self.module.var_decl_types.insert(key.clone(), dt.clone());
        }
        if let crate::ast::types::DataType::TypeReference { name: tn, .. } = dt {
            if self.module.classes.contains_key(&tn.name.name) {
                self.module
                    .array_elem_class
                    .insert(key.clone(), tn.name.name.clone());
            }
        }
        key
    }

    /// Queue the rename-table entry `pname -> target` for the call being set
    /// up (a by-value formal's own key, or a `ref` formal's alias of the
    /// actual's storage); `commit_formal_dyn` enters it in the callee frame.
    /// The entry also SHADOWS any enclosing renamed local of the same name:
    /// without it `dyn_name_lookup` walks up and redirects the callee's
    /// formal to the caller's storage.
    pub(super) fn map_formal_dyn(&mut self, pname: &str, target: &str) {
        self.pending_formal_dyn
            .push((pname.to_string(), target.to_string()));
    }

    /// Start binding a call's formals: the callee's (empty) rename frame,
    /// just pushed by `push_queue_frame`, is held aside so every actual
    /// resolves in the CALLER's view, and the formals' entries queue up from
    /// the returned base (see `map_formal_dyn`).
    pub(super) fn begin_formal_dyn(&mut self) -> FormalDynSetup {
        let names = self.local_dyn.pop().unwrap_or_default();
        let formals = self.formal_dyn.pop().unwrap_or_default();
        (self.pending_formal_dyn.len(), names, formals)
    }

    /// Reinstate the callee frame held by `begin_formal_dyn` with the
    /// formals queued since `base` entered in it, once all the call's
    /// actuals are evaluated.
    pub(super) fn commit_formal_dyn(&mut self, (base, mut names, mut formals): FormalDynSetup) {
        for (pname, target) in self.pending_formal_dyn.drain(base..) {
            match names.iter_mut().find(|(k, _)| *k == pname) {
                Some(slot) => slot.1 = pname.clone(),
                None => names.push((pname.clone(), pname.clone())),
            }
            match formals.iter_mut().find(|(k, _)| *k == pname) {
                Some(slot) => slot.1 = target,
                None => formals.push((pname, target)),
            }
        }
        self.local_dyn.push(names);
        self.formal_dyn.push(formals);
    }

    /// The storage key of a queue / dynamic-array (or fixed unpacked array)
    /// ACTUAL: a plain name (a caller-local one under its per-process rename
    /// `@q#7`), or a member chain `obj.q` / `a.b.q` mapped to the owning
    /// object's `<handle>#q` storage. `None` when `arg` names no collection.
    pub(super) fn queue_actual_storage(&mut self, arg: &Expression) -> Option<String> {
        let cname = match &arg.kind {
            ExprKind::Ident(h) if h.path.len() == 1 => {
                let mut n = self.resolve_hier_name(h);
                // A subroutine-LOCAL queue actual lives under its
                // per-process rename (`@paths#7`). Resolving it to the bare
                // name made the formal look IDENTITY-bound to its own bare
                // storage, so the writeback was skipped and the callee's
                // results never reached the caller's local (UVM's
                // `get_full_hdl_path(paths)` returned an empty path list —
                // every backdoor access read 0).
                if let Some(r) = self.dyn_name_lookup(&n) {
                    n = std::borrow::Cow::Owned(r.to_string());
                } else if let Some(s) = self.instance_assoc_member(&n) {
                    n = std::borrow::Cow::Owned(s);
                }
                n.into_owned()
            }
            // §13.5.2: a queue/dynamic-array member of ANOTHER object passed
            // as the actual — `cb.doit(comp.q)` / `cb.doit(this.q)`, or a
            // member chain of any depth (`a.b.c.q`, parsed as a flattened
            // Ident path of plain segments). The HEAD is the base object;
            // each MIDDLE segment is a handle-valued property of the object
            // the preceding segment resolves to; the LAST segment names the
            // collection, stored per instance at `<handle>#member`.
            // (Scope-qualified names like `pkg::Class::queue` are a different
            // AST shape and resolve via static-property tables elsewhere.)
            ExprKind::Ident(h)
                if h.path.len() >= 2 && h.path.iter().all(|s| s.selects.is_empty()) =>
            {
                let head = &h.path[0].name.name;
                let leaf = h.path.last().unwrap().name.name.clone();
                let mut handle = self.eval_ident_handle(head)?;
                for seg in h.path.iter().take(h.path.len() - 1).skip(1) {
                    handle = self.member_handle(handle, &seg.name.name)?;
                }
                self.handle_collection_name(handle, &leaf)?
            }
            ExprKind::MemberAccess { expr, member } => {
                let handle = self.eval_handle_expr(expr)?;
                self.handle_collection_name(handle, &member.name)?
            }
            _ => return None,
        };
        if !self.module.arrays.contains_key(&*cname)
            && !self.module.dynamic_arrays.contains(&*cname)
        {
            return None;
        }
        Some(cname)
    }

    /// §7.10 / §7.4.6: the elements of a SLICE of an unpacked collection
    /// used as a value (`q[1:$]`, `q[a+:n]`, `fixed[1:3]`), with the
    /// element width; `None` when `arg` is not such a slice (or one this
    /// helper does not model: a descending fixed array, struct elements).
    /// For a queue or dynamic array an empty or reversed range (`q[3:1]`,
    /// `q[4:$]` on four elements) yields no elements, and bounds beyond
    /// either end are clamped (§7.10.1).
    pub(super) fn collection_slice_values(
        &mut self,
        arg: &Expression,
    ) -> Option<(Vec<Value>, u32)> {
        let ExprKind::RangeSelect {
            expr: base,
            kind,
            left,
            right,
        } = &arg.kind
        else {
            return None;
        };
        let name = self.queue_actual_storage(base)?;
        if self.module.descending_arrays.contains(&*name) || self.queue_elem_struct(&name).is_some()
        {
            return None;
        }
        let (lo_a, hi_a, w) = *self.module.arrays.get(&*name)?;
        let is_dyn = self.module.dynamic_arrays.contains(&*name);
        let upper = if is_dyn {
            self.get_queue_size(&name) as i64 - 1
        } else {
            hi_a
        };
        self.dollar_bound.push(upper);
        let l = self.eval_expr(left).to_i64();
        let r = self.eval_expr(right).to_i64();
        self.dollar_bound.pop();
        let (l, r) = (l?, r?);
        let (from, to) = match kind {
            RangeKind::Constant => (l, r),
            RangeKind::IndexedUp => (l, l + r - 1),
            RangeKind::IndexedDown => (l - r + 1, l),
        };
        let lo_b = if is_dyn { 0 } else { lo_a };
        let (from, to) = (from.max(lo_b), to.min(upper));
        let vals = if from > to {
            Vec::new()
        } else {
            (from..=to)
                .map(|idx| {
                    self.get_signal_value_by_name(&format!("{}[{}]", name, idx))
                        .unwrap_or_else(|| Value::zero(w))
                })
                .collect()
        };
        Some((vals, w))
    }

    /// §13.5.2: stage a queue FORMAL's contents under a fresh temporary
    /// name so the caller's shadowed storage can be restored BEFORE the
    /// ref/output writeback is applied.
    ///
    /// Both orders are wrong without this: writing back first lets the
    /// callee-frame restore of a same-named LOCAL overwrite the result
    /// (UVM's nested `get_full_hdl_path`, whose callee declares its own
    /// `parent_paths`), while restoring first discards the callee's formal
    /// before it can be read. Capture, restore, then apply.
    pub(super) fn stage_queue_param(&mut self, param: &str) -> String {
        let id = self.next_dyn_id;
        self.next_dyn_id += 1;
        let tmp = format!("@wb#{}", id);
        let sz_key = format!("{}.size", param);
        let pre = format!("{}[", param);
        let mut moved_keys: Vec<String> = self.signals.keys_with_elem_prefix(&pre);
        if self.signals.contains_key(&sz_key) {
            moved_keys.push(sz_key.clone());
        }
        let moved: Vec<(String, Value)> = moved_keys
            .iter()
            .filter_map(|k| self.signals.get(k).map(|v| (k, v)))
            .map(|(k, v)| {
                let nk = if **k == sz_key {
                    format!("{}.size", tmp)
                } else {
                    format!("{}{}", tmp, &k[param.len()..])
                };
                (nk, v.clone())
            })
            .collect();
        for (k, v) in moved {
            self.signals.insert(k, v);
        }
        // The size may live in the compact table rather than `signals`.
        let sz = self.get_queue_size(param);
        if let Some(e) = self.module.arrays.get(param).copied() {
            self.module.arrays.insert(tmp.clone(), e);
        }
        if self.module.dynamic_arrays.contains(param) {
            self.module.dynamic_arrays.insert(tmp.clone());
        }
        if let Some(dt) = self.module.var_decl_types.get(param).cloned() {
            self.module.var_decl_types.insert(tmp.clone(), dt);
        }
        for j in 0..sz {
            let (d, s2) = (format!("{}[{}]", tmp, j), format!("{}[{}]", param, j));
            if self.signals.contains_key(&d) {
                continue;
            }
            if let Some(v) = self.get_signal_value_by_name(&s2) {
                self.signals.insert(d, v);
            }
        }
        self.set_queue_size(&tmp, sz);
        tmp
    }

    /// Drop a staged queue temp created by `stage_queue_param`.
    pub(super) fn drop_staged_queue(&mut self, tmp: &str) {
        let sz_key = format!("{}.size", tmp);
        let pre = format!("{}[", tmp);
        let mut keys: Vec<String> = self.signals.keys_with_elem_prefix(&pre);
        if self.signals.contains_key(&sz_key) {
            keys.push(sz_key.clone());
        }
        for k in keys {
            self.signals.remove(&k);
        }
        self.module.arrays.remove(tmp);
        self.module.dynamic_arrays.remove(tmp);
        self.module.var_decl_types.remove(tmp);
    }

    /// Copy a queue param's contents back to the caller's queue on return
    /// (ref/output/inout). The caller's queue may have grown or shrunk, so its
    /// stale element signals are cleared first.
    pub(super) fn writeback_queue_param(&mut self, param: &str, caller: &str) {
        if caller.is_empty() || param == caller {
            return;
        }
        let size = self.get_queue_size(param);
        let w = self.module.arrays.get(param).map(|t| t.2).unwrap_or(32);
        let vals: Vec<Value> = (0..size)
            .map(|j| {
                self.get_signal_value_by_name(&format!("{}[{}]", param, j))
                    .unwrap_or_else(|| Value::zero(w))
            })
            .collect();
        let stale: Vec<String> = self.signals.keys_with_elem_prefix(&format!("{}[", caller));
        for k in stale {
            self.signals.remove(&k);
        }
        self.module.arrays.insert(caller.to_string(), (0, -1, w));
        self.module.dynamic_arrays.insert(caller.to_string());
        // §7.2: an element of an UNPACKED-struct queue is a set of member
        // leaves, not one value — copying `q[j]` alone moved nothing, so a
        // `ref pkt_t q[$]` came back with the right SIZE and every member
        // zero. Copy those elements member-wise.
        let elem_su = self
            .p_elem_type(param)
            .or_else(|| self.p_elem_type(caller))
            .and_then(|dt| self.unpacked_struct_of(&dt));
        if let Some(su) = elem_su {
            for j in 0..size {
                let (d, s) = (format!("{}[{}]", caller, j), format!("{}[{}]", param, j));
                self.copy_unpacked_struct(&d, &s, &su);
            }
            self.set_queue_size(caller, size);
            return;
        }
        for (j, v) in vals.into_iter().enumerate() {
            self.set_signal_value_by_name(&format!("{}[{}]", caller, j), v);
        }
        self.set_queue_size(caller, size);
    }

    /// LRM §8.4: evaluate a value argument destined for a queue/array
    /// element store. When the queue/array's element type is a class
    /// (registered via `module.array_elem_class`) and the arg is a
    /// bare `new(<args>)` call, construct an instance of that class
    /// and return its handle. Otherwise fall back to regular eval.
    pub(super) fn queue_eval_arg(&mut self, container_name: &str, arg: &Expression) -> Value {
        // §25.9: an element of a virtual-interface collection holds a vif
        // HANDLE — the instance the argument names (`vq.push_back(bus)`) or is
        // bound to (`vq.push_back(cfg.vif)`), as its handle token. Evaluating
        // the argument as a value yields neither: an interface instance has no
        // value and a vif property reads only its binding's existence.
        if self.vif_collection(container_name)
            && let Some(inst) = self.resolve_vif_rhs_name_strict(arg)
        {
            self.vif_tokens_stored = true;
            return Value::from_u64(Self::iface_handle_token(&inst), 32);
        }
        let class_name = self.module.array_elem_class.get(container_name).cloned();
        if let Some(cn) = class_name {
            // NOTE: a BARE `new` is deliberately not accepted here. `new` is
            // legal only in an assignment to a class handle (§8.7), so
            // `q.push_back(new)` is invalid SystemVerilog — the reference
            // simulator rejects it with a syntax error, and accepting it would
            // be an accepts-invalid. The `new(...)` Call form below is
            // pre-existing behavior.
            if let ExprKind::Call {
                func,
                args: call_args,
            } = &arg.kind
            {
                let is_new = match &func.kind {
                    ExprKind::Ident(h) => h.path.last().is_some_and(|s| s.name.name == "new"),
                    _ => false,
                };
                if is_new {
                    if let Some(class_def) = self.module.classes.get(&cn).cloned() {
                        return self
                            .instantiate_class_with_type_args(&class_def, call_args, None)
                            .resize(32);
                    }
                }
            }
        }
        self.eval_expr(arg)
    }

    /// §7.12.3: a reduction's result has the ELEMENT type — width AND
    /// signedness. Summing `byte fixed[4]` with u64 accumulation and a plain
    /// 32-bit result read 439 where the element-typed answer is -73 (0xB7 as
    /// signed 8-bit), and min/max compared signed elements unsigned.
    pub(super) fn array_elem_sign_width(&self, obj: &str) -> (u32, bool) {
        // An associative array (module, local or class property) has its
        // element type on record — elements need not start at key 0.
        if self.is_associative_array(obj) {
            if let Some(dt) = self.assoc_elem_decl_type(obj) {
                let w = self.assoc_elem_width(obj).unwrap_or_else(|| {
                    super::elaborate::resolve_type_width(
                        dt,
                        Some(&self.module.parameters),
                        Some(&self.module.typedefs),
                    )
                });
                let signed =
                    super::elaborate::is_type_signed_resolved(dt, &self.module.typedef_types);
                return (w.max(1), signed);
            }
        }
        let w = self
            .module
            .arrays
            .get(obj)
            .map(|&(_, _, w)| w)
            .or_else(|| {
                self.get_signal_value_by_name(&format!("{}[0]", obj))
                    .map(|v| v.width)
            })
            .unwrap_or(32)
            .max(1);
        let signed = self
            .module
            .var_decl_types
            .get(obj)
            .map(|dt| {
                use crate::ast::types::{DataType, IntegerAtomType, Signing};
                match dt {
                    DataType::IntegerAtom { kind, signing, .. } => {
                        !matches!(signing, Some(Signing::Unsigned))
                            && matches!(
                                kind,
                                IntegerAtomType::Byte
                                    | IntegerAtomType::ShortInt
                                    | IntegerAtomType::Int
                                    | IntegerAtomType::LongInt
                                    | IntegerAtomType::Integer
                            )
                    }
                    DataType::IntegerVector { signing, .. } => {
                        matches!(signing, Some(Signing::Signed))
                    }
                    _ => false,
                }
            })
            .unwrap_or(false)
            || self.signed_class_coll(obj);
        (w, signed)
    }

    /// Whether `coll` is the instance-scoped storage of a class queue /
    /// dynamic-array property with a signed integral element type (see
    /// `signed_class_colls`).
    #[inline]
    pub(super) fn signed_class_coll(&self, coll: &str) -> bool {
        !self.signed_class_colls.is_empty() && self.signed_class_colls.contains(coll)
    }

    /// §6.11.1: an element stored into a signed-element class collection
    /// takes the element type's width and signedness, not the rvalue's.
    #[inline(never)]
    pub(super) fn fit_signed_class_coll_elem(&self, coll: &str, val: Value) -> Value {
        let mut v = match self.module.arrays.get(coll) {
            Some(&(_, _, w)) if w > 0 && w != val.width => val.resize(w),
            _ => val,
        };
        v.is_signed = true;
        v
    }

    pub(super) fn eval_builtin_method(
        &mut self,
        obj_name: &str,
        mname: &str,
        args: &[Expression],
    ) -> Option<Value> {
        // One string match classifies the method up front; every check below
        // is then an integer compare, and a name this function does not
        // implement returns before the receiver-key allocation and the scans
        // (the sequential `mname == ".."` chain cost a memcmp per arm).
        let bm = BuiltinM::classify(mname);
        if bm == BuiltinM::Other {
            return None;
        }
        // Per-specialization keying for STATIC collection properties of
        // parameterized classes (e.g. `static T m_q[$]` in `uvmq#(T)`):
        // rewrite the bare member name to the spec-aware storage key so each
        // specialization gets its own element/size cells. No-op for instance
        // methods and non-static/non-collection names.
        if env_set_cached!("XEZIM_BM_CENSUS") {
            eprintln!(
                "[bm] {} {} this={}",
                obj_name,
                mname,
                self.this_stack.last().copied().flatten().is_some()
            );
        }
        let _resolved = self.spec_static_coll_key(obj_name);
        let obj_name: &str = &_resolved;
        // §8.10: inside a class method, a MEMBER collection shadows any
        // same-named module/package-scope collection. Several callers gate
        // on the bare name being registered globally before trying the
        // instance scope, so `queue.size()` inside `uvm_queue` read a phase-
        // graph function's global-by-bare-name local `queue[$]` instead of
        // `<this>#queue` — every pool queue reported a phantom size and null
        // elements (backdoor hdl paths came back empty). Rewrite centrally:
        // bare name + declared by the current class + not a true local.
        let _inst_scoped;
        let obj_name = if !obj_name
            .as_bytes()
            .iter()
            .any(|&b| matches!(b, b'#' | b'.' | b'['))
        {
            // §8.10 precedence for a BARE receiver — see
            // `bare_receiver_is_class_handle`.
            if self.bare_receiver_is_class_handle(obj_name) {
                return None;
            }
            // A bare CLASS-NAME receiver (`uvm_config_db#(T)` flattened to
            // `uvm_config_db`) is a STATIC-method call, not a collection.
            // Without this, `uvm_config_db::exists(...)` (and any other
            // class static whose name collides with a collection builtin)
            // fell through to the assoc-array `exists` handler and answered
            // 0 without ever running the class method body. Resolve the
            // specialization base (or bare name) against the class table
            // and bail so the class-static dispatch owns the call.
            if self.module.classes.contains_key(obj_name)
                || obj_name
                    .split('#')
                    .next()
                    .is_some_and(|b| self.module.classes.contains_key(b))
            {
                return None;
            }
            match self.dyn_name_lookup(obj_name) {
                None => match self.instance_assoc_member(obj_name) {
                    Some(scoped) => {
                        _inst_scoped = scoped;
                        _inst_scoped.as_str()
                    }
                    None => obj_name,
                },
                // A local under its per-call storage key: a same-named
                // registration elsewhere (another subroutine's bare local)
                // must not capture the call.
                Some(uq) if uq != obj_name => {
                    _inst_scoped = uq.to_string();
                    _inst_scoped.as_str()
                }
                Some(_) => obj_name,
            }
        } else {
            obj_name
        };
        // §6.19.6 — Enum methods first/last/next/prev/num on an enum-typed
        // signal. Look up the signal's type_name → enum_members[type_name]
        // (Vec<(name, value)> in declaration order, which IS the LRM
        // iteration order). Type info is on signal_type_names (keyed by
        // signal_id), since `self.module.signals` may not be the storage.
        // §6.19.6: first/last/num take no argument; next/prev take an OPTIONAL
        // count N (default 1) — `c.next(2)` steps two forward with wrapping.
        if (matches!(bm, BuiltinM::First | BuiltinM::Last | BuiltinM::Num) && args.is_empty())
            || (matches!(bm, BuiltinM::Next | BuiltinM::Prev) && args.len() <= 1)
        {
            let step_n: u64 = if matches!(bm, BuiltinM::Next | BuiltinM::Prev) {
                args.first()
                    .map(|a| self.eval_expr(a).to_u64().unwrap_or(1))
                    .unwrap_or(1)
            } else {
                0
            };
            let tn_opt: Option<String> = self
                .signal_name_to_id
                .get(obj_name)
                .and_then(|id| self.signal_type_names.get(id).cloned());
            // Anonymous `enum {...} v;` variables have no typedef name, so the
            // member list is keyed by the variable name itself (§6.19.6).
            // The flat signal-name lookup collides across scopes: a
            // METHOD-LOCAL `uvm_severity s;` resolved through whatever OTHER
            // `s` the table held (UVM is full of `string s;`/enum locals of
            // other types), so the walk in `reset_severity_counts()` stepped
            // through a foreign 2-member enum and cycled 0,1,0,… — the
            // missing `UVM_ERROR : 0` rows in every UVM report summary. The
            // DECLARATION recorded the local's true typedef in
            // `var_typedef_types`; that binding is the nearest scope, so it
            // takes precedence; the flat signal type and the anonymous-enum
            // key remain as fallbacks.
            let members_opt = self
                .var_typedef_types
                .get(obj_name)
                .and_then(|tn| self.module.enum_members.get(tn))
                .or_else(|| {
                    tn_opt
                        .as_ref()
                        .and_then(|tn| self.module.enum_members.get(tn))
                })
                .or_else(|| self.module.enum_members.get(obj_name))
                .cloned();
            {
                if let Some(members) = members_opt {
                    if members.is_empty() {
                        return Some(Value::zero(32));
                    }
                    // All enum members share the typedef's base width.
                    let mw = self
                        .signal_name_to_id
                        .get(members[0].0.as_str())
                        .and_then(|id| self.signal_widths.get(*id).copied())
                        .unwrap_or(32);
                    // §6.19: the result carries the ENUM's signedness (base
                    // type; default `int` is signed), so `es.first()` on
                    // `enum shortint {A=-1,...}` reads -1, not 65535.
                    let esigned = self.lookup_signal_signed(obj_name);
                    let emask: u64 = if mw >= 64 { u64::MAX } else { (1u64 << mw) - 1 };
                    let mk = |raw: u64| {
                        let mut v = Value::from_u64(raw & emask, mw);
                        v.is_signed = esigned;
                        v
                    };
                    return Some(match bm {
                        BuiltinM::Num => Self::int_count(members.len() as u64),
                        BuiltinM::First => mk(members[0].1),
                        BuiltinM::Last => mk(members[members.len() - 1].1),
                        BuiltinM::Next | BuiltinM::Prev => {
                            // Read the receiver like an ordinary identifier —
                            // LOCAL FRAME first. `get_signal_value_by_name`
                            // consults only the flat table, so a method-local
                            // (`uvm_severity s`) read whatever same-named
                            // variable that table held: `pos` never matched,
                            // next() answered the invalid-value default and
                            // UVM's severity walk cycled instead of advancing.
                            let cur_v = self
                                .get_local_or_signal(obj_name)
                                .or_else(|| self.get_signal_value_by_name(obj_name));
                            let cur_xz = cur_v.as_ref().is_some_and(|v| v.has_xz());
                            let cur = cur_v.as_ref().map(|v| v.to_u64().unwrap_or(0)).unwrap_or(0);
                            // Compare at the enum's WIDTH: the member table and
                            // the variable can hold the same value at different
                            // widths (a signed enum sign-extends on read), so a
                            // raw u64 compare never matched and every next/prev
                            // fell through to the invalid-value default.
                            let pos = if cur_xz {
                                None
                            } else {
                                members
                                    .iter()
                                    .position(|(_, v)| (*v & emask) == (cur & emask))
                            };
                            if let Some(p) = pos {
                                // LRM §6.19.6: next(N)/prev(N) step N places with
                                // wrapping (next of last -> first, prev of first
                                // -> last). N defaults to 1; N==0 returns current.
                                let len = members.len() as u64;
                                let step = (step_n % len) as usize;
                                let new_p = if bm == BuiltinM::Next {
                                    (p + step) % members.len()
                                } else {
                                    (p + members.len() - step) % members.len()
                                };
                                mk(members[new_p].1)
                            } else if self.module.two_state_signals.contains(obj_name) {
                                // §6.19.6: an invalid value maps to the base
                                // type's DEFAULT — 0 for 2-state.
                                Value::zero(mw)
                            } else {
                                // ... and x for a 4-state base.
                                Value::new(mw)
                            }
                        }
                        _ => unreachable!(),
                    });
                }
            }
        }
        // If obj_name is a mailbox/semaphore handle variable, don't treat it as
        // an array — bail (None) so the call falls through to the handle-based
        // container dispatch. The handle lives in a signal for a module/local
        // var, but in the instance's property map for a class member (e.g.
        // a container class's `local mailbox m`), so check both: a signal-only check
        // mis-routes a class-property mailbox's `num()`/`get()` to the array
        // path and reads 0.
        if matches!(
            mname,
            "num" | "put" | "get" | "peek" | "try_put" | "try_get" | "try_peek"
        ) {
            if let Some(handle) = self.resolve_container_handle(obj_name) {
                if self.mailboxes.contains_key(&handle) || self.semaphores.contains_key(&handle) {
                    return None;
                }
            }
        }
        if bm == BuiltinM::Size || bm == BuiltinM::Len {
            // An associative array's size() == num(): count populated keys.
            // (Without this it falls through to the queue/.size-shadow/string
            // paths and reads 0 for a true assoc array — sv_22.)
            if self.is_associative_array(obj_name) {
                let prefix = Self::name_with_suffix(obj_name, "[");
                // §7.8.1: an associative array of associative arrays (or of
                // queues) stores its elements under compound keys
                // (`m[A][B]`); size() counts the distinct first-level keys,
                // as num() does, not the stored leaves. Any compound key
                // tells: an associative array of queues also stores the
                // queue's own `m[A]` entry, and the key order is not fixed.
                // One borrowing pass both counts and checks.
                let mut count = 0usize;
                let mut compound = false;
                for k in self.signals.elem_keys_with_prefix(&prefix) {
                    count += 1;
                    compound = compound || k[prefix.len()..].contains("][");
                }
                if compound {
                    let n = self.assoc_top_level_keys(obj_name).len();
                    return Some(Self::int_count(n as u64));
                }
                let c2 = self.assoc_static_keys(&prefix).len();
                return Some(Self::int_count((count + c2) as u64));
            }
            if let Some(v) = self.signals.get(&Self::name_with_suffix(obj_name, ".size")) {
                return Some(Self::int_count(v.to_u64().unwrap_or(0)));
            }
            // Dynamic-array/queue size may live in the compact signal table
            // (e.g. package- or module-scope `arr[] = {...}` whose `.size`
            // entry was materialised at elaboration time). A queue with NO
            // `.size` entry anywhere has never been touched — it is empty; do
            // NOT fall through to the `arrays` bounds, which are a (0, 63)
            // registration placeholder, not a length (a fresh module-scope
            // queue read size()==64).
            if self.module.dynamic_arrays.contains(obj_name) {
                if let Some(v) =
                    self.get_signal_value_by_name(&Self::name_with_suffix(obj_name, ".size"))
                {
                    return Some(Self::int_count(v.to_u64().unwrap_or(0)));
                }
                return Some(Self::int_count(0));
            }
            if let Some((lo, hi, _)) = self.module.arrays.get(obj_name) {
                return Some(Self::int_count((hi - lo + 1) as u64));
            }
            // Fallback for strings (value may be a frame-local/parameter).
            // Counts the content bytes — including embedded/trailing NULs,
            // which §21.2.1.4 unformatted dumps legitimately contain.
            let base_val = self
                .get_local_signal_or_static(obj_name)
                .or_else(|| self.get_signal_value_by_name(obj_name));

            if let Some(base) = base_val {
                return Some(Self::int_count(base.sv_string_bytes().len() as u64));
            }
            return Some(Self::int_count(0));
        }
        // §6.16.9 str.getc(i) — the byte value of character i (0 past the end).
        // Was unimplemented (returned 0), which made string glob matchers compare
        // every char as 0==0 — exact matches passed by accident but `*`/`?`
        // wildcards were never detected.
        if bm == BuiltinM::Getc {
            if let Some(idx_arg) = args.first() {
                let idx = self.eval_expr(idx_arg).to_u64().unwrap_or(0) as usize;
                // Byte indexing, not UTF-8: a char above 0x7F is one SV byte.
                let b = self
                    .get_local_signal_or_static(obj_name)
                    .map(|v| v.sv_string_bytes())
                    .unwrap_or_default()
                    .get(idx)
                    .copied()
                    .unwrap_or(0);
                return Some(Self::sv_byte(b));
            }
            return Some(Value::zero(8));
        }
        if bm == BuiltinM::Substr {
            if let Some(first) = args.first() {
                if let Some(second) = args.get(1) {
                    let start = self.eval_expr(first).to_u64().unwrap_or(0) as usize;
                    let end = self.eval_expr(second).to_u64().unwrap_or(0) as usize;

                    let base_val = self.get_local_signal_or_static(obj_name);

                    if let Some(base) = base_val {
                        let mut highest_bit = 0;
                        for i in (0..base.width as usize).rev() {
                            if base.get_bit(i) != LogicBit::Zero {
                                highest_bit = i;
                                break;
                            }
                        }
                        let actual_len = highest_bit.div_ceil(8);
                        if actual_len > 0 && start < actual_len && end < actual_len && start <= end
                        {
                            let l = (actual_len - 1 - start) * 8 + 7;
                            let r = (actual_len - 1 - end) * 8;
                            return Some(base.range_select(l, r));
                        }
                    }
                }
            }
            return Some(Value::zero(0));
        }
        // A class-handle receiver whose class defines a queue-builtin-named
        // method (e.g. `queue#(T)::push_back/get/size`) must dispatch the
        // USER method, never be treated as a native queue. The flattened
        // `Call{Ident}` path is guarded by `prefer_user_method`; this covers
        // the MemberAccess dispatch path (audit class-b gap). Return None so
        // the class-method dispatch below takes over. A real queue/array never
        // resolves `obj_name` to a live heap instance, so this is precise.
        if matches!(
            mname,
            "push_back"
                | "push_front"
                | "pop_front"
                | "pop_back"
                | "sort"
                | "rsort"
                | "reverse"
                | "shuffle"
                | "insert"
                | "get"
                | "size"
                | "delete"
                | "first"
                | "last"
        ) {
            if let Some(h) = self
                .get_local_signal_or_static(obj_name)
                .and_then(|v| v.to_u64())
                .filter(|&h| h != 0)
            {
                let cn = self
                    .heap
                    .get(h as usize)
                    .and_then(|o| o.as_ref())
                    .map(|i| i.class_name.clone());
                if let Some(cn) = cn {
                    if self.class_has_method(&cn, mname) {
                        return None;
                    }
                }
            }
        }
        if bm == BuiltinM::PushBack {
            if let Some(arg) = args.first() {
                // LRM §8.4: if the queue's element type is a class and
                // the arg is a bare `new(...)` call, construct the
                // class instead of `eval_expr`-ing the call (which
                // returns 0 because the bare `new` has no class
                // context). The resulting handle is what gets pushed.
                let cur_size = self.get_queue_size(obj_name);
                if let Some(&max) = self.module.queue_max_sizes.get(obj_name) {
                    if cur_size >= max as u64 {
                        return Some(Value::zero(32));
                    }
                }
                let elem = Self::name_with_index(obj_name, cur_size as i64);
                self.queue_store_elem(obj_name, &elem, arg);
                self.set_queue_size(obj_name, cur_size + 1);
            }
            return Some(Value::zero(32));
        }
        if bm == BuiltinM::PushFront {
            if let Some(arg) = args.first() {
                let cur_size = self.get_queue_size(obj_name);
                if let Some(&max) = self.module.queue_max_sizes.get(obj_name) {
                    if cur_size >= max as u64 {
                        return Some(Value::zero(32));
                    }
                }
                for i in (0..cur_size).rev() {
                    self.queue_move_elem(obj_name, i, i + 1);
                }
                let head = Self::name_with_index(obj_name, 0);
                self.queue_store_elem(obj_name, &head, arg);
                self.set_queue_size(obj_name, cur_size + 1);
            }
            return Some(Value::zero(32));
        }
        if bm == BuiltinM::PopFront {
            let cur_size = self.get_queue_size(obj_name);
            if cur_size > 0 {
                let val = self.queue_elem_value(obj_name, 0);
                for i in 1..cur_size {
                    self.queue_move_elem(obj_name, i, i - 1);
                }
                self.set_queue_size(obj_name, cur_size - 1);
                return Some(val);
            }
            return Some(Value::zero(32));
        }
        if bm == BuiltinM::PopBack {
            let cur_size = self.get_queue_size(obj_name);
            if cur_size > 0 {
                let val = self.queue_elem_value(obj_name, cur_size as i64 - 1);
                self.set_queue_size(obj_name, cur_size - 1);
                return Some(val);
            }
            return Some(Value::zero(32));
        }
        if bm == BuiltinM::Sort {
            let cur_size = self.get_queue_size(obj_name) as usize;
            if cur_size > 0 {
                // §7.12.2 + §11.4.8: a STRING element collection sorts
                // lexicographically, not by its little-endian packed-byte
                // integer ("uvm" would sort before "common.run").
                let is_str = self.is_string_collection(obj_name);
                let mut elements = Vec::new();
                for i in 0..cur_size {
                    if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, i))
                    {
                        elements.push(v);
                    }
                }
                if is_str {
                    elements.sort_by(|a, b| a.to_sv_string().cmp(&b.to_sv_string()));
                } else {
                    elements.sort_by_key(|a| a.to_u64().unwrap_or(0));
                }
                for (i, v) in elements.into_iter().enumerate() {
                    self.set_signal_value_by_name(&format!("{}[{}]", obj_name, i), v);
                }
            }
            return Some(Value::zero(32));
        }
        if bm == BuiltinM::Rsort {
            let cur_size = self.get_queue_size(obj_name) as usize;
            if cur_size > 0 {
                let is_str = self.is_string_collection(obj_name);
                let mut elements = Vec::new();
                for i in 0..cur_size {
                    if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, i))
                    {
                        elements.push(v);
                    }
                }
                if is_str {
                    elements.sort_by(|a, b| b.to_sv_string().cmp(&a.to_sv_string()));
                } else {
                    elements.sort_by(|a, b| b.to_u64().unwrap_or(0).cmp(&a.to_u64().unwrap_or(0)));
                }
                for (i, v) in elements.into_iter().enumerate() {
                    self.set_signal_value_by_name(&format!("{}[{}]", obj_name, i), v);
                }
            }
            return Some(Value::zero(32));
        }
        if bm == BuiltinM::Reverse {
            let cur_size = self.get_queue_size(obj_name) as usize;
            if cur_size > 0 {
                let order: Vec<usize> = (0..cur_size).rev().collect();
                self.queue_permute(obj_name, &order);
            }
            return Some(Value::zero(32));
        }
        // LRM §7.12.1 array `shuffle()` — Fisher-Yates in place using
        // the simulator's PRNG so seeded runs are reproducible.
        if bm == BuiltinM::Shuffle {
            use rand::Rng;
            let cur_size = self.get_queue_size(obj_name) as usize;
            if cur_size > 1 {
                let mut order: Vec<usize> = (0..cur_size).collect();
                // Fisher-Yates over the index order, then move the elements.
                for i in (1..cur_size).rev() {
                    let j = self.cur_rng().gen_range(0..=i);
                    order.swap(i, j);
                }
                self.queue_permute(obj_name, &order);
            }
            return Some(Value::zero(32));
        }
        if bm == BuiltinM::Insert {
            if args.len() >= 2 {
                let iv = self.eval_expr(&args[0]);
                let cur_size = self.get_queue_size(obj_name);
                // §7.10.2.2: an index with x/z bits, a negative one or one
                // past `size()` leaves the queue unchanged.
                let idx = match iv.to_u64() {
                    Some(i)
                        if !iv.has_xz()
                            && !(iv.is_signed && iv.to_i64().is_some_and(|v| v < 0))
                            && i <= cur_size =>
                    {
                        i
                    }
                    _ => return Some(Value::zero(32)),
                };
                // Move elements member-wise: an unpacked-struct element has no
                // container signal, so a packed copy shifts nothing at all.
                for i in (idx..cur_size).rev() {
                    self.queue_move_elem(obj_name, i, i + 1);
                }
                let slot = format!("{}[{}]", obj_name, idx);
                self.queue_store_elem(obj_name, &slot, &args[1]);
                self.set_queue_size(obj_name, cur_size + 1);
            }
            return Some(Value::zero(32));
        }
        if bm == BuiltinM::Num {
            // §15.4.2: a mailbox's `num()` counts queued messages, which live in
            // `self.mailboxes` (keyed by handle), not in `obj[i]` signal slots.
            // This expression path (e.g. `m.num()` inside `$display(...)`) must
            // resolve the handle before falling back to the queue/assoc count.
            if let Some(handle_val) = self.get_signal_value_by_name(obj_name) {
                let handle = handle_val.to_u64().unwrap_or(0) as usize;
                if let Some(q) = self.mailboxes.get(&handle) {
                    return Some(Self::int_count(q.len() as u64));
                }
            }
            // Count distinct TOP-LEVEL keys: for `obj[KEY]` storage this is
            // just the element count (== queue/dynamic-array size), but for
            // an assoc-of-queue `obj[KEY][IDX]` it collapses every element of
            // a key's queue onto that one key (so `num()` reports the number
            // of associative keys, not the total element count). The
            // first-`]` extraction + dedup is uniform across plain assoc,
            // assoc-of-queue, queues, and dynamic arrays.
            return Some(Self::int_count(
                self.assoc_top_level_keys(obj_name).len() as u64
            ));
        }
        if matches!(bm, BuiltinM::Sum | BuiltinM::Product) {
            let (w, signed) = self.array_elem_sign_width(obj_name);
            let mut total: u64 = if bm == BuiltinM::Sum { 0 } else { 1 };
            for elem in self.reduction_elem_names(obj_name) {
                if let Some(v) = self.get_signal_value_by_name(&elem) {
                    let x = if signed {
                        v.to_i64().unwrap_or(0) as u64
                    } else {
                        v.to_u64().unwrap_or(0)
                    };
                    total = if bm == BuiltinM::Sum {
                        total.wrapping_add(x)
                    } else {
                        total.wrapping_mul(x)
                    };
                }
            }
            // Truncate to the element width — the accumulation wraps there.
            let mut out = Value::from_u64(
                if w < 64 {
                    total & ((1u64 << w) - 1)
                } else {
                    total
                },
                w.min(64),
            );
            out.is_signed = signed;
            return Some(out);
        }
        if matches!(bm, BuiltinM::And | BuiltinM::Or | BuiltinM::Xor) {
            let mut acc: Option<u64> = None;
            for elem in self.reduction_elem_names(obj_name) {
                if let Some(v) = self.get_signal_value_by_name(&elem) {
                    let x = v.to_u64().unwrap_or(0);
                    acc = Some(match (acc, mname) {
                        (None, _) => x,
                        (Some(a), "and") => a & x,
                        (Some(a), "or") => a | x,
                        (Some(a), "xor") => a ^ x,
                        (Some(a), _) => a,
                    });
                }
            }
            return Some(Value::from_u64(acc.unwrap_or(0), 32));
        }
        if bm == BuiltinM::Min {
            let cur_size = self.get_queue_size(obj_name) as usize;
            let mut min_val: Option<Value> = None;
            for i in 0..cur_size {
                if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, i)) {
                    let signed = self.array_elem_sign_width(obj_name).1;
                    let lt = if signed {
                        v.to_i64().unwrap_or(i64::MAX)
                            < min_val
                                .as_ref()
                                .and_then(|m| m.to_i64())
                                .unwrap_or(i64::MAX)
                    } else {
                        // The first element has nothing to compare against
                        // (`min_val` is None): it was an unconditional unwrap
                        // that panicked on every unsigned collection.
                        v.to_u64().unwrap_or(u64::MAX)
                            < min_val
                                .as_ref()
                                .and_then(|m| m.to_u64())
                                .unwrap_or(u64::MAX)
                    };
                    if min_val.is_none() || lt {
                        min_val = Some(v);
                    }
                }
            }
            return Some(min_val.unwrap_or(Value::zero(32)));
        }
        if bm == BuiltinM::Max {
            let cur_size = self.get_queue_size(obj_name) as usize;
            let mut max_val: Option<Value> = None;
            for i in 0..cur_size {
                if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, i)) {
                    let signed = self.array_elem_sign_width(obj_name).1;
                    let gt = if signed {
                        v.to_i64().unwrap_or(i64::MIN)
                            > max_val
                                .as_ref()
                                .and_then(|m| m.to_i64())
                                .unwrap_or(i64::MIN)
                    } else {
                        v.to_u64().unwrap_or(0)
                            > max_val.as_ref().and_then(|m| m.to_u64()).unwrap_or(0)
                    };
                    if max_val.is_none() || gt {
                        max_val = Some(v);
                    }
                }
            }
            return Some(max_val.unwrap_or(Value::zero(32)));
        }
        if bm == BuiltinM::Unique {
            // §7.12.1: a locator method RETURNS a queue; it must not modify the
            // source. `r = q.unique()` is handled by the locator path in
            // BlockingAssign; a bare `q.unique();` is simply a no-op.
            return Some(Value::zero(32));
        }
        if bm == BuiltinM::UniqueIndex {
            let cur_size = self.get_queue_size(obj_name) as usize;
            let mut seen = std::collections::HashSet::new();
            let mut indices = Vec::new();
            for i in 0..cur_size {
                if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, i)) {
                    let key = v.to_u64().unwrap_or(0);
                    if seen.insert(key) {
                        indices.push(i as u64);
                    }
                }
            }
            for (i, idx) in indices.iter().enumerate() {
                self.set_signal_value_by_name(
                    &format!("{}[{}]", obj_name, i),
                    Value::from_u64(*idx, 32),
                );
            }
            self.set_queue_size(obj_name, indices.len() as u64);
            return Some(Value::zero(32));
        }
        if (bm == BuiltinM::Find
            || bm == BuiltinM::FindFirst
            || bm == BuiltinM::FindLast
            || bm == BuiltinM::FindIndex
            || bm == BuiltinM::FindFirstIndex
            || bm == BuiltinM::FindLastIndex)
            && (self.module.arrays.contains_key(obj_name)
                || self.is_associative_array(obj_name)
                || self.module.dynamic_arrays.contains(obj_name))
        {
            let cur_size = self.get_queue_size(obj_name) as usize;
            let mut results = Vec::new();
            if let Some(callback) = args.first() {
                for i in 0..cur_size {
                    if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, i))
                    {
                        let idx_match = if mname.contains("index") {
                            Value::from_u64(i as u64, 32)
                        } else {
                            v.clone()
                        };
                        if let ExprKind::Binary {
                            op: BinaryOp::LogAnd,
                            ..
                        }
                        | ExprKind::Binary { op: _, .. } = &callback.kind
                        {
                            // Simple "with" clause: evaluate with item substituted
                            // For now just collect all non-zero
                            if v.to_u64().unwrap_or(0) != 0 {
                                results.push(idx_match);
                            }
                        } else {
                            results.push(idx_match);
                        }
                    }
                }
            }
            if mname.contains("first") {
                results.truncate(1);
            }
            if mname.contains("last") && !results.is_empty() {
                let last = results.pop().unwrap();
                results = vec![last];
            }
            return Some(if results.is_empty() {
                Value::zero(32)
            } else {
                results[0].clone()
            });
        }
        if bm == BuiltinM::Exists {
            if let Some(arg) = args.first() {
                let kv = self.eval_expr(arg);
                let key = self.assoc_key_str(obj_name, &kv);
                let elem_name = Self::name_with_key(obj_name, &key);
                // class-perf: the hot path (a flat key that exists) must not pay
                // for the prefix/`.size` probes that only disambiguate MAP and
                // STRUCT assoc elements. Probe the direct scalar entry first and
                // build the sacrificial Strings lazily on the miss only — every
                // entry probe here allocates a heap String (`name_with_suffix`),
                // and the common regex-cache loop looks up an existing key every
                // iteration.
                if self.signals.contains_key(&elem_name)
                    || self.signal_name_to_id.contains_key(elem_name.as_str())
                    || self.module.dynamic_arrays.contains(&elem_name)
                {
                    return Some(Value::from_u64(1, 1));
                }
                // A multidimensional associative array (`m[K1][K2]`, IEEE
                // 1800-2023 §7.8.1) stores elements under the compound key
                // `m[K1][K2]`; the bare `m[K1]` is never a direct entry. So a
                // hit is EITHER the flat 1D key OR any compound `m[K1][...]`
                // element (prefix scan). An assoc element that is itself a
                // collection is stored under `<assoc>[<key>].size` / `<assoc>[<key>][i]`.
                // A STRUCT element stores member-wise leaves (`sa[k].id`) — no
                // `sa[k]` entry ever exists.
                let found = self
                    .signals
                    .contains_key(&Self::name_with_suffix(&elem_name, ".size"))
                    || self
                        .signals
                        .any_key_with_prefix(&Self::name_with_suffix(&elem_name, "["))
                    || self
                        .signals
                        .any_key_with_prefix(&Self::name_with_suffix(&elem_name, "."));
                return Some(Value::from_u64(found as u64, 1));
            }
        }
        if bm == BuiltinM::Delete {
            if args.is_empty()
                && self.dynamic_assoc_dims.contains_key(obj_name)
                && self.module.dynamic_arrays.contains(obj_name)
            {
                self.clear_dynamic_assoc_rows(obj_name, 0);
                self.set_queue_size(obj_name, 0);
                return Some(Value::zero(32));
            }
            if self.is_associative_array(obj_name) {
                if let Some(arg) = args.first() {
                    let key = self.eval_expr(arg);
                    let key_str = self.assoc_key_str(obj_name, &key);
                    let elem_name = format!("{}[{}]", obj_name, key_str);
                    if self.signals.remove(&elem_name).is_none() {
                        // §7.9.2: an aggregate element (an unpacked struct,
                        // a nested collection) has no `aa[k]` cell of its
                        // own — it lives as member / element cells under
                        // that name (the shapes `exists` probes), and all
                        // of them go. Only reached on the direct-entry miss.
                        let member = Self::name_with_suffix(&elem_name, ".");
                        let sub = Self::name_with_suffix(&elem_name, "[");
                        let prefix = format!("{}[", obj_name);
                        for k in self.signals.keys_with_elem_prefix(&prefix) {
                            if k.starts_with(member.as_str()) || k.starts_with(sub.as_str()) {
                                self.signals.remove(&k);
                            }
                        }
                    }
                } else {
                    let prefix = format!("{}[", obj_name);
                    let keys: Vec<String> = self.signals.keys_with_elem_prefix(&prefix);
                    for k in keys {
                        self.signals.remove(&k);
                    }
                }
                return Some(Value::zero(32));
            }
            // §7.10.2.3: a queue keyed per specialization (`C#int::q`,
            // §8.25) is known by its size shadow alone.
            if self.module.arrays.contains_key(obj_name)
                || self.module.dynamic_arrays.contains(obj_name)
                || (obj_name.contains("::")
                    && self
                        .signals
                        .contains_key(&Self::name_with_suffix(obj_name, ".size")))
            {
                if let Some(arg) = args.first() {
                    let idx = self.eval_expr(arg).to_u64().unwrap_or(0) as usize;
                    let cur_size = self.get_queue_size(obj_name) as usize;
                    if idx < cur_size {
                        // Member-wise, for the same reason as `insert`.
                        for i in idx..cur_size - 1 {
                            self.queue_move_elem(obj_name, (i + 1) as u64, i as u64);
                        }
                        self.set_queue_size(obj_name, (cur_size - 1) as u64);
                    }
                } else {
                    self.set_queue_size(obj_name, 0);
                }
                return Some(Value::zero(32));
            }
        }
        if (bm == BuiltinM::First
            || bm == BuiltinM::Last
            || bm == BuiltinM::Next
            || bm == BuiltinM::Prev)
            && self.is_associative_array(obj_name)
        {
            let keys = self.assoc_top_level_keys(obj_name);
            // i128, not i64: a full-range unsigned 64-bit key
            // (18446744073709551615) is numeric too — treating it as a
            // string wrote the key's ASCII bytes into the ref arg.
            let all_numeric = keys.iter().all(|k| k.parse::<i128>().is_ok());
            if keys.is_empty() {
                return Some(Value::zero(32));
            }
            let result_key = if bm == BuiltinM::First {
                Some(keys[0].clone())
            } else if bm == BuiltinM::Last {
                Some(keys[keys.len() - 1].clone())
            } else {
                if let Some(arg) = args.first() {
                    let cur_val = self.eval_expr(arg);
                    // Canonicalize exactly like the STORE side
                    // (assoc_key_str): a signed ref var must read back
                    // "-3", not its zero-extended u64 form — otherwise
                    // next()/prev() never find the current position.
                    let cur = if all_numeric {
                        self.assoc_key_str(obj_name, &cur_val)
                    } else {
                        cur_val.to_sv_string()
                    };
                    if let Some(pos) = keys.iter().position(|k| *k == cur) {
                        if bm == BuiltinM::Next {
                            if pos + 1 < keys.len() {
                                Some(keys[pos + 1].clone())
                            } else {
                                None
                            }
                        } else {
                            if pos > 0 {
                                Some(keys[pos - 1].clone())
                            } else {
                                None
                            }
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            if let Some(key) = result_key {
                if let Some(arg) = args.first() {
                    if all_numeric {
                        let key_int = key.parse::<i128>().unwrap_or(0);
                        let w = self.infer_width(arg);
                        self.assign_value(arg, &Value::from_u64(key_int as u64, w));
                    } else {
                        let key_val = Value::from_string(&key);
                        self.assign_value(arg, &key_val);
                    }
                }
                return Some(Value::from_u64(1, 32));
            } else {
                return Some(Value::zero(32));
            }
        }
        None
    }
}
