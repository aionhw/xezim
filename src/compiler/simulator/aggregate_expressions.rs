//! Simulator aggregate expressions.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// §7.4.1: bit offset of ELEMENT `idx` of the packed multi-D signal
    /// `name`, normalized against the declared outer dimension — a descending
    /// Declared packed dimension a trailing bit/part select on `base` indexes
    /// into, when `base` is a chain of Index layers over an identifier —
    /// an ELEMENT of an unpacked array/queue/assoc (`logic [31:8] q[$];
    /// q[0][31:8]`), or a packed element (`logic [1:0][31:8] p; p[0][...]`).
    /// Index layers up to the collection's unpacked depth consume no packed
    /// dims; each further layer consumes one. Returns the ORIENTED (left,
    /// right) pair so callers can map labels for both a non-zero lower bound
    /// and an ascending range: physical = label - right (descending) /
    /// right - label (ascending). None for a plain Ident base (layers = 0 —
    /// the existing plain-signal paths own that) or when nothing is recorded.
    /// Packed dimensions of a CLASS PROPERTY (`rand u7_t [4:0][1:0] d;`),
    /// declared dimensions first and then a typedef's own, resolved from the
    /// property's declared type. Module-scope geometry tables are keyed by
    /// signal name, which a property never has, so without this an element
    /// select on a packed multi-dimensional property was a single-bit select
    /// of the whole value (`d[0][0]` read bit 0; `d[2][1] = 77` was dropped).
    pub(super) fn class_prop_packed_dims(
        &self,
        handle: usize,
        prop: &str,
    ) -> Option<Vec<(i64, i64)>> {
        let cn = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone())?;
        let mut cur = Some(cn);
        while let Some(c) = cur {
            let cd = self.module.classes.get(&c)?;
            // A property with UNPACKED dimensions (`bit [1:0][7:0] tbl [4]`)
            // is a collection: its index selects an element, not a slice.
            if cd.array_properties.contains_key(prop) || cd.array_nd_properties.contains_key(prop) {
                return None;
            }
            if let Some(dt) = cd.property_types.get(prop) {
                let dims = super::elaborate::packed_full_dims_chained(
                    dt,
                    &self.module.parameters,
                    &self.module.typedef_types,
                )?;
                return if dims.len() >= 2 { Some(dims) } else { None };
            }
            cur = cd.extends.clone();
        }
        None
    }

    /// If `expr` is an index chain `root[i]...[k]` whose root names a class
    /// property (a bare property of `this`, not shadowed by a local or a
    /// signal, or `obj.prop` on a handle), return the handle, the property
    /// name, and the index expressions outermost first.
    pub(super) fn prop_index_chain<'e>(
        &mut self,
        expr: &'e Expression,
    ) -> Option<(usize, String, Vec<&'e Expression>)> {
        self.prop_index_chain_inner(expr, None)
    }

    /// Same as `prop_index_chain` for an `Index` already split into its base
    /// and outermost index (the eval arm destructures the node, shadowing
    /// the whole expression).
    pub(super) fn prop_index_chain_parts<'e>(
        &mut self,
        base: &'e Expression,
        outer: &'e Expression,
    ) -> Option<(usize, String, Vec<&'e Expression>)> {
        self.prop_index_chain_inner(base, Some(outer))
    }

    pub(super) fn prop_index_chain_inner<'e>(
        &mut self,
        expr: &'e Expression,
        outer: Option<&'e Expression>,
    ) -> Option<(usize, String, Vec<&'e Expression>)> {
        let mut rev: Vec<&'e Expression> = Vec::new();
        if let Some(o) = outer {
            rev.push(o);
        }
        let mut cur = expr;
        while let ExprKind::Index { expr: b, index } = &cur.kind {
            rev.push(index);
            cur = b;
        }
        if rev.is_empty() {
            return None;
        }
        let (handle, prop) = self.prop_chain_root(cur)?;
        rev.reverse();
        Some((handle, prop, rev))
    }

    /// The object and member a plain or handle-qualified member reference
    /// names (`w`, `o.w`, `y.a.w`, `expr.w`), for the packed-property
    /// select paths.
    pub(super) fn prop_chain_root(&mut self, cur: &Expression) -> Option<(usize, String)> {
        Some(match &cur.kind {
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                let name = &h.path[0].name.name;
                if self
                    .local_stack
                    .last()
                    .is_some_and(|f| f.contains_key(name))
                    || self.signal_name_to_id.contains_key(name.as_str())
                {
                    return None;
                }
                let handle = self.this_stack.last().copied().flatten()?;
                let has = self
                    .heap
                    .get(handle)
                    .and_then(|o| o.as_ref())
                    .is_some_and(|i| i.properties.contains_key(name));
                if !has {
                    return None;
                }
                (handle, name.clone())
            }
            ExprKind::Ident(h)
                if h.path.len() == 2 && h.path.iter().all(|s| s.selects.is_empty()) =>
            {
                let obj = self.eval_ident_handle(&h.path[0].name.name)?;
                if obj == 0 || obj >= self.heap.len() {
                    return None;
                }
                (obj, h.path[1].name.name.clone())
            }
            // `y.a.w[i][j]`: a member of an object reached through a chain
            // of handle members.
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.len() > 2
                    && h.path.iter().all(|s| s.selects.is_empty()) =>
            {
                let mut obj = self.eval_ident_handle(&h.path[0].name.name)?;
                for seg in &h.path[1..h.path.len() - 1] {
                    if obj == 0 || obj >= self.heap.len() {
                        return None;
                    }
                    self.prop_class_type(obj, &seg.name.name)?;
                    obj = self.member_handle(obj, &seg.name.name)?;
                }
                if obj == 0 || obj >= self.heap.len() {
                    return None;
                }
                (obj, h.path.last()?.name.name.clone())
            }
            ExprKind::MemberAccess { expr: base, member } => {
                let obj = self.eval_handle_expr(base)?;
                if obj == 0 || obj >= self.heap.len() {
                    return None;
                }
                (obj, member.name.clone())
            }
            _ => return None,
        })
    }

    /// The packed value `prop` of `h` and its indices `idx_exprs` select
    /// into: the property itself when it is a multi-dimensional packed
    /// array, or (§7.4) an element of a fixed unpacked array property whose
    /// element type is one (`bit [3:0][7:0] arr [3]`, `arr[i][2]`), the
    /// unpacked indices consumed. Returns the value, its packed dimensions
    /// and the packed indices.
    pub(super) fn class_packed_operand(
        &mut self,
        h: usize,
        prop: &str,
        idx_exprs: &[&Expression],
    ) -> Option<(Value, Vec<(i64, i64)>, Vec<Value>, ClassPackedStore)> {
        if let Some(dims) = self.class_prop_packed_dims(h, prop) {
            let base = self.read_member_value(h, prop)?;
            let idx = idx_exprs.iter().map(|ie| self.eval_expr(ie)).collect();
            return Some((base, dims, idx, ClassPackedStore::Prop));
        }
        if idx_exprs.is_empty() {
            return None;
        }
        let (udims, _) = self.fixed_foreach_dims(h, prop)?;
        if idx_exprs.len() < udims.len() {
            return None;
        }
        let mut cn = self
            .heap
            .get(h)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone())?;
        let dt = loop {
            let cd = self.module.classes.get(&cn)?;
            if let Some(dt) = cd.property_types.get(prop) {
                break dt.clone();
            }
            cn = cd.extends.clone()?;
        };
        let pdims = super::elaborate::packed_full_dims_chained(
            &dt,
            &self.module.parameters,
            &self.module.typedef_types,
        )?;
        if pdims.len() < 2 {
            return None;
        }
        let idx: Vec<Value> = idx_exprs.iter().map(|ie| self.eval_expr(ie)).collect();
        let mut sfx = String::new();
        for v in &idx[..udims.len()] {
            sfx.push_str(&format!("[{}]", v.to_i64()?));
        }
        let elem = self.class_elem_load(h, prop, &sfx)?;
        let store = ClassPackedStore::ElemSuffix(sfx);
        Some((elem, pdims, idx[udims.len()..].to_vec(), store))
    }

    /// §7.4.2/§7.4.1: WRITE counterpart of [`Self::class_packed_operand`] —
    /// `d.arr[1][2] = v`, `d.arr[1][8:0] = v`, `d.w[i][4 +: 2] = v` (also
    /// bare `arr[1][2]` in a method). Resolves the element and its packed
    /// dims through the same walk the read uses, splices the fitted value
    /// into the addressed element(s) of the next packed dimension, and
    /// stores the element back under the same per-instance key. `None`
    /// when the lvalue is not such a member chain (the caller falls
    /// through); `Some(false)` for an x/z or out-of-range select (§11.5.1:
    /// the write selects nothing).
    pub(super) fn class_packed_elem_sel_write(
        &mut self,
        lhs: &Expression,
        val: &Value,
    ) -> Option<bool> {
        use crate::ast::expr::RangeKind;
        let (base, tail_bit, tail_range) = match &lhs.kind {
            ExprKind::Index { expr, index } => (expr, Some(index), None),
            ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } => (expr, None, Some((*kind, left, right))),
            _ => return None,
        };
        // Unpacked-index chain below the trailing select.
        let mut idx_exprs: Vec<&Expression> = Vec::new();
        let mut cur = base;
        while let ExprKind::Index { expr: b, index } = &cur.kind {
            idx_exprs.push(index);
            cur = b;
        }
        idx_exprs.reverse();
        if idx_exprs.is_empty() {
            // A single select on the property itself is
            // `class_packed_elem_ref`'s case (hooked earlier).
            return None;
        }
        let (h, prop) = self.prop_chain_root(cur)?;
        let (elem, dims, pidx, store) = self.class_packed_operand(h, &prop, &idx_exprs)?;
        let depth = pidx.len();
        let &(dl, dr) = dims.get(depth)?;
        let (lo_b, hi_b) = (dl.min(dr), dl.max(dr));
        let ew = Self::packed_chain_width(elem.width as u64, &dims, depth + 1);
        // Offset of any packed selects already consumed by the operand
        // walk (`w[i][k] = v`, property itself multi-dim): the tail indexes
        // the element `pidx` addressed, not bit 0 of the property.
        let (mut lsb, _) = Self::packed_chain_slice(elem.width as u64, &dims, &pidx)?;
        let slot = |k: i64| {
            if dl >= dr { k - lo_b } else { hi_b - k }
        };
        let (first, last, n) = match (tail_bit, tail_range) {
            (Some(ix), _) => {
                let v = self.eval_expr(ix);
                if v.has_xz() || v.is_real {
                    return Some(false); // §11.5.1: x/z index selects nothing
                }
                let k = v.to_i64()?;
                if k < lo_b || k > hi_b {
                    return Some(false); // out-of-range select: no write
                }
                (k, k, 1i64)
            }
            (_, Some((kind, left, right))) => {
                let l = self.eval_expr(left);
                let r = self.eval_expr(right);
                let (a, b) = match (l.to_i64(), r.to_i64()) {
                    (Some(a), Some(b)) if !l.has_xz() && !r.has_xz() => match kind {
                        RangeKind::Constant => (a, b),
                        RangeKind::IndexedUp => (a, a + b - 1),
                        RangeKind::IndexedDown => (a - b + 1, a),
                    },
                    _ => return Some(false),
                };
                if a < lo_b || a > hi_b || b < lo_b || b > hi_b {
                    return Some(false); // out-of-range select: no write
                }
                (a, b, (a - b).abs() + 1)
            }
            _ => return None,
        };
        lsb += slot(first).min(slot(last)) as u64 * ew;
        let out_w = (n as u64 * ew) as u32;
        if lsb + out_w as u64 > elem.width as u64 {
            return None;
        }
        let piece = val.resize(out_w);
        let mut nv = elem.clone();
        for i in 0..(out_w as usize) {
            nv.set_bit(lsb as usize + i, piece.get_bit(i));
        }
        if nv == elem {
            return Some(false);
        }
        match store {
            ClassPackedStore::Prop => {
                if let Some(Some(inst)) = self.heap.get_mut(h) {
                    inst.properties.insert(prop.to_string(), nv);
                } else {
                    return Some(false);
                }
            }
            ClassPackedStore::ElemSuffix(sfx) => {
                self.class_elem_store(h, &prop, &sfx, nv);
            }
        }
        Some(true)
    }

    /// §7.4.1/§11.5.1: a part-select of a multi-dimensional packed class
    /// property, after any element selects (`w[5][8:5]`, `w[6:5]`,
    /// `y.a.w[i][4 +: 2]`): the labels of the next dimension, each
    /// spanning one element of it. None when `base` is not such a
    /// property.
    pub(super) fn class_packed_part_select(
        &mut self,
        base: &Expression,
        kind: crate::ast::expr::RangeKind,
        left: &Expression,
        right: &Expression,
    ) -> Option<Value> {
        let mut idx_exprs: Vec<&Expression> = Vec::new();
        let mut cur = base;
        while let ExprKind::Index { expr: b, index } = &cur.kind {
            idx_exprs.push(index);
            cur = b;
        }
        idx_exprs.reverse();
        let (h, prop) = self.prop_chain_root(cur)?;
        let (base_v, dims, idx, _) = self.class_packed_operand(h, &prop, &idx_exprs)?;
        let &(dl, dr) = dims.get(idx.len())?;
        let l = self.eval_expr(left);
        let r = self.eval_expr(right);
        let count = (dl - dr).unsigned_abs() + 1;
        let ew = Self::packed_chain_width(base_v.width as u64, &dims, idx.len() + 1);
        let (first, last) = match (l.to_i64(), r.to_i64()) {
            (Some(a), Some(b)) if !l.has_xz() && !r.has_xz() => match kind {
                crate::ast::expr::RangeKind::Constant => (a, b),
                crate::ast::expr::RangeKind::IndexedUp => (a, a + b - 1),
                crate::ast::expr::RangeKind::IndexedDown => (a - b + 1, a),
            },
            _ => (i64::MIN, i64::MIN),
        };
        let n = (first - last).unsigned_abs() + 1;
        let out_w = (n * ew) as u32;
        let Some((blsb, _)) = Self::packed_chain_slice(base_v.width as u64, &dims, &idx) else {
            return Some(Value::new(out_w));
        };
        let lo_b = dl.min(dr);
        let slot = |k: i64| -> Option<u64> {
            let off = k - lo_b;
            if off < 0 || off as u64 >= count {
                return None;
            }
            Some(if dl >= dr {
                off as u64
            } else {
                count - 1 - off as u64
            })
        };
        let (Some(a), Some(b)) = (slot(first), slot(last)) else {
            return Some(Value::new(out_w));
        };
        let lsb = blsb + a.min(b) * ew;
        Some(base_v.range_select((lsb + out_w as u64 - 1) as usize, lsb as usize))
    }

    /// Is `e` an index or part-select whose base is itself an index or a
    /// member access (`p[1][3:0]`, `s.f[2]`)? The cheap shape test that
    /// gates `frame_packed_chain` on the assignment and read paths.
    #[inline(always)]
    pub(super) fn select_of_select(e: &Expression) -> bool {
        let (ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. }) = &e.kind else {
            return false;
        };
        matches!(
            expr.kind,
            ExprKind::Index { .. } | ExprKind::MemberAccess { .. } | ExprKind::Paren(_)
        )
    }

    /// A select of a select of a variable the innermost call frame owns (a
    /// task or function local, formal or return value): `p[1][3:0]`,
    /// `p[0][7]`, `p[i][j][k +: 2]`, `a[i].f[3]`, `s.m[1][7:4]`. Returns the
    /// variable's identifier and name and the selects, root first. `None`
    /// for a non-frame root. A single select is also accepted when capturing
    /// a persistent local's NBA; ordinary reads/writes use `select_of_select`.
    pub(super) fn frame_packed_chain<'e>(
        &self,
        e: &'e Expression,
    ) -> Option<(
        std::borrow::Cow<'e, Expression>,
        &'e str,
        Vec<PendingSel<'e>>,
    )> {
        let mut steps = Vec::new();
        let mut cur = e;
        loop {
            match &cur.kind {
                ExprKind::Index { expr, index } => {
                    steps.push(PendingSel::Index(index));
                    cur = expr;
                }
                // §11.5.1: only the last select may be a part-select.
                ExprKind::RangeSelect {
                    expr,
                    kind,
                    left,
                    right,
                } if steps.is_empty() => {
                    steps.push(PendingSel::Range {
                        kind: *kind,
                        left,
                        right,
                    });
                    cur = expr;
                }
                ExprKind::MemberAccess { expr, member } => {
                    steps.push(PendingSel::Member(member.name.as_str()));
                    cur = expr;
                }
                ExprKind::Paren(x) => cur = x,
                _ => break,
            }
        }
        let ExprKind::Ident(h) = &cur.kind else {
            return None;
        };
        if h.root.is_some() || h.path.is_empty() {
            return None;
        }
        let name = h.path[0].name.name.as_str();
        if !self
            .local_stack
            .last()
            .is_some_and(|f| f.contains_key(name))
            && !self.hoisted_sub_static_names.contains(name)
        {
            return None;
        }
        steps.reverse();
        let root = if h.path.len() == 1 && h.path[0].selects.is_empty() {
            std::borrow::Cow::Borrowed(cur)
        } else {
            // Elaboration flattens module-task members into an identifier
            // path; class methods retain MemberAccess nodes. Both denote
            // the same declared packed geometry (§7.4.1 / §23.9).
            let mut prefix = Vec::new();
            for (i, segment) in h.path.iter().enumerate() {
                if i != 0 {
                    prefix.push(PendingSel::Member(segment.name.name.as_str()));
                }
                prefix.extend(segment.selects.iter().map(PendingSel::Index));
            }
            prefix.extend(steps);
            steps = prefix;
            let mut identifier = h.clone();
            identifier.path.truncate(1);
            identifier.path[0].selects.clear();
            identifier.cached_signal_id = Cell::new(None);
            identifier.cached_resolved_name = std::cell::OnceCell::new();
            std::borrow::Cow::Owned(Expression::new(ExprKind::Ident(identifier), cur.span))
        };
        if steps.is_empty() {
            return None;
        }
        Some((root, name, steps))
    }

    /// `frame_packed_chain` placed on the variable's declared type: the
    /// identifier, name, selects and their `packed_hops`, and whether the
    /// type is 2-state. `None` for a collection (its elements have their own
    /// storage), a type with no packed layout, or a frame value whose width
    /// is not the declared one.
    #[allow(clippy::type_complexity)]
    pub(super) fn frame_packed_target<'e>(
        &self,
        e: &'e Expression,
    ) -> Option<(
        std::borrow::Cow<'e, Expression>,
        &'e str,
        Vec<PendingSel<'e>>,
        Vec<PackedHop>,
        bool,
    )> {
        let (root, name, steps) = self.frame_packed_chain(e)?;
        if self.module.arrays.contains_key(name)
            || self.module.dynamic_arrays.contains(name)
            || self.module.associative_arrays.contains_key(name)
            || self.module.queue_vars.contains(name)
        {
            return None;
        }
        let dt = self.module.var_decl_types.get(name)?;
        let (hops, total, two) = self.packed_hops(dt, &steps)?;
        let valid = if let Some(cur) = self.local_stack.last().and_then(|f| f.get(name)) {
            !cur.is_real && cur.width == total
        } else if self.hoisted_sub_static_names.contains(name) {
            self.get_signal_value_by_name(name)
                .is_some_and(|cur| !cur.is_real && cur.width == total)
        } else {
            false
        };
        if !valid {
            return None;
        }
        Some((root, name, steps, hops, two))
    }

    /// The packed layout of `dt`: its packed dimensions, outermost first and
    /// through typedefs, then the element below them — a packed struct or
    /// union, or `None` for a single bit — and that element's width. An
    /// integer atom is a vector `[w-1:0]`; an enum has its base type's layout
    /// (`int` by default). `None` for a type with
    /// no packed layout (class, string, real, unpacked or tagged aggregate)
    /// or a bound that is not constant.
    pub(super) fn packed_shape<'a>(
        &'a self,
        dt: &'a DataType,
    ) -> Option<(
        Vec<(i64, i64)>,
        Option<&'a crate::ast::types::StructUnionType>,
        u32,
    )> {
        use crate::ast::types::PackedDimension;
        fn bounds(
            ds: &[PackedDimension],
            params: &HashMap<String, Value>,
            out: &mut Vec<(i64, i64)>,
        ) -> Option<()> {
            for d in ds {
                let PackedDimension::Range { left, right, .. } = d else {
                    return None;
                };
                out.push((
                    super::elaborate::const_eval_i64_with_params(left, Some(params))?,
                    super::elaborate::const_eval_i64_with_params(right, Some(params))?,
                ));
            }
            Some(())
        }
        let params = &self.module.parameters;
        let typedefs: Option<&HashMap<String, u32>> = Some(&self.module.typedefs);
        let mut dims = Vec::new();
        let mut cur = dt;
        for _ in 0..32 {
            match cur {
                DataType::TypeReference {
                    name, dimensions, ..
                } => {
                    bounds(dimensions, params, &mut dims)?;
                    cur = super::elaborate::lookup_scoped_type(name, &self.module.typedef_types)?;
                }
                DataType::IntegerVector { dimensions, .. }
                | DataType::Implicit { dimensions, .. } => {
                    bounds(dimensions, params, &mut dims)?;
                    return Some((dims, None, 1));
                }
                DataType::IntegerAtom { .. } => {
                    let w = resolve_type_width(cur, Some(params), typedefs);
                    dims.push((w as i64 - 1, 0));
                    return Some((dims, None, 1));
                }
                // §6.19: the base type lays out an enum's bits, so its range
                // labels them (`enum logic [8:1]`); the default base is int.
                DataType::Enum(e) => {
                    bounds(&e.dimensions, params, &mut dims)?;
                    let Some(b) = e.base_type.as_deref() else {
                        dims.push((31, 0));
                        return Some((dims, None, 1));
                    };
                    cur = b;
                }
                DataType::Struct(su) if su.packed && !su.tagged => {
                    bounds(&su.dimensions, params, &mut dims)?;
                    let ws = su.members.iter().flat_map(|m| {
                        let w = resolve_type_width(&m.data_type, Some(params), typedefs);
                        m.declarators.iter().map(move |_| w)
                    });
                    let w = if matches!(su.kind, crate::ast::types::StructUnionKind::Union) {
                        ws.max()?
                    } else {
                        ws.sum::<u32>()
                    };
                    return Some((dims, Some(su), w));
                }
                _ => return None,
            }
        }
        None
    }

    /// §7.2.1/§7.3.1: the bit offset of member `name` within one `su` and
    /// its declared type. The first member of a packed struct is the most
    /// significant; every member of a packed union starts at bit 0.
    pub(super) fn packed_member_at<'a>(
        &self,
        su: &'a crate::ast::types::StructUnionType,
        name: &str,
    ) -> Option<(u32, &'a DataType)> {
        let union = matches!(su.kind, crate::ast::types::StructUnionKind::Union);
        let mut off = 0u32;
        for m in su.members.iter().rev() {
            let w = resolve_type_width(
                &m.data_type,
                Some(&self.module.parameters),
                Some(&self.module.typedefs),
            );
            for d in m.declarators.iter().rev() {
                if d.name.name == name {
                    return Some((if union { 0 } else { off }, &m.data_type));
                }
                off += w;
            }
        }
        None
    }

    /// Walk the selects `steps` (root first) down the declared type `dt` and
    /// record how each addresses the value it applies to, without
    /// evaluating anything: an index or part-select takes the next packed
    /// dimension (a packed struct with none left is a vector of its bits), a
    /// member moves to its field. Also returns the width of `dt`. `None`
    /// when the type has no packed layout or a step does not fit it.
    pub(super) fn packed_hops(
        &self,
        dt: &DataType,
        steps: &[PendingSel],
    ) -> Option<(Vec<PackedHop>, u32, bool)> {
        fn width(dims: &[(i64, i64)], bw: u32) -> Option<u32> {
            dims.iter().try_fold(bw, |w, &(l, r)| {
                w.checked_mul(u32::try_from((l - r).unsigned_abs() + 1).ok()?)
            })
        }
        let (mut dims, mut agg, mut bw) = self.packed_shape(dt)?;
        let total = width(&dims, bw)?;
        let mut two = super::elaborate::is_type_two_state_resolved(dt, &self.module.typedef_types);
        let mut hops = Vec::with_capacity(steps.len());
        let mut d = 0usize;
        for step in steps {
            match step {
                PendingSel::Index(_) | PendingSel::Range { .. } => {
                    if let Some(&(l, r)) = dims.get(d) {
                        d += 1;
                        hops.push(PackedHop::Dim {
                            l,
                            r,
                            ew: width(&dims[d..], bw)?,
                        });
                    } else if agg.take().is_some() {
                        hops.push(PackedHop::Dim {
                            l: bw as i64 - 1,
                            r: 0,
                            ew: 1,
                        });
                        bw = 1;
                    } else {
                        return None;
                    }
                }
                PendingSel::Member(m) => {
                    let su = agg.filter(|_| d == dims.len())?;
                    let (off, mdt) = self.packed_member_at(su, m)?;
                    let (md, ma, mb) = self.packed_shape(mdt)?;
                    let mw = resolve_type_width(
                        mdt,
                        Some(&self.module.parameters),
                        Some(&self.module.typedefs),
                    );
                    if width(&md, mb)? != mw {
                        return None;
                    }
                    hops.push(PackedHop::Field { off, w: mw });
                    two = super::elaborate::is_type_two_state_resolved(
                        mdt,
                        &self.module.typedef_types,
                    );
                    (dims, agg, bw, d) = (md, ma, mb, 0);
                }
            }
        }
        Some((hops, total, two))
    }

    /// Evaluate the selects `steps` placed by `packed_hops`, in order. Returns
    /// the width of the select and, unless an index is x/z or out of range
    /// (§11.5.1: the select then names no bits), `(lsb, src, len)`: the `len`
    /// bits it names that exist, from bit `lsb` of the variable and bit `src`
    /// of the select's value. A part-select partly out of range keeps the
    /// bits in range only.
    pub(super) fn packed_hops_eval(
        &mut self,
        hops: &[PackedHop],
        steps: &[PendingSel],
    ) -> (u32, Option<(u64, u64, u64)>) {
        fn int(sim: &mut Simulator, e: &Expression) -> Option<i64> {
            let v = sim.eval_expr(e);
            v.to_i64().filter(|_| !v.has_xz() && !v.is_real)
        }
        let mut lsb = 0u64;
        // Every index is evaluated, in order, even past an invalid one.
        let mut ok = true;
        for (k, (hop, step)) in hops.iter().zip(steps).enumerate() {
            let (l, r, ew) = match *hop {
                // A chain that ends at a member (`s.f <= v`) names all of it.
                PackedHop::Field { off, w } if k + 1 == hops.len() => {
                    let lsb = lsb + off as u64;
                    return (w, (ok && w > 0).then_some((lsb, 0, w as u64)));
                }
                PackedHop::Field { off, .. } => {
                    lsb += off as u64;
                    continue;
                }
                PackedHop::Dim { l, r, ew } => (l, r, ew),
            };
            // Labels of the first and last element selected, and the element
            // count when they are unknown.
            let (ends, elems) = match step {
                PendingSel::Index(ix) => (int(self, ix).map(|i| (i, i)), 1),
                PendingSel::Range { kind, left, right } => {
                    let (a, n) = (int(self, left), int(self, right));
                    let up = matches!(kind, RangeKind::IndexedUp);
                    match kind {
                        RangeKind::Constant => (a.zip(n), 1),
                        _ => (
                            a.zip(n.filter(|&n| n > 0)).map(|(a, n)| {
                                let b = if up {
                                    a.saturating_add(n - 1)
                                } else {
                                    a.saturating_sub(n - 1)
                                };
                                (a, b)
                            }),
                            n.unwrap_or(1),
                        ),
                    }
                }
                PendingSel::Member(_) => (None, 1),
            };
            let Some((a, b)) = ends else {
                ok = false;
                if k + 1 == hops.len() {
                    let w = elems.max(1).saturating_mul(ew as i64);
                    return (u32::try_from(w).unwrap_or(0), None);
                }
                continue;
            };
            // Slot of a label counted from the least significant element;
            // linear, so a label outside `[l:r]` lands outside `0..count`.
            let slot = |i: i64| {
                if l >= r {
                    i.saturating_sub(r)
                } else {
                    r.saturating_sub(i)
                }
            };
            let count = (l - r).abs() + 1;
            let n = i64::try_from(a.abs_diff(b))
                .unwrap_or(i64::MAX)
                .saturating_add(1);
            let lo = slot(a).min(slot(b));
            if k + 1 < hops.len() {
                ok &= lo >= 0 && lo < count;
                if ok {
                    lsb += lo as u64 * ew as u64;
                }
                continue;
            }
            let w = u32::try_from(n.saturating_mul(ew as i64)).unwrap_or(0);
            let (first, last) = (lo.max(0), lo.saturating_add(n - 1).min(count - 1));
            if !ok || w == 0 || first > last {
                return (w, None);
            }
            let ew = ew as u64;
            return (
                w,
                Some((
                    lsb + first as u64 * ew,
                    (first - lo) as u64 * ew,
                    (last - first + 1) as u64 * ew,
                )),
            );
        }
        (0, None)
    }

    /// §7.4.1/§7.2.1/§11.5.1: store `val` through a select of a select of a
    /// packed variable the innermost call frame owns (`frame_packed_chain`).
    /// The select arms resolve a nested select of a multidimensional packed
    /// array or of a packed array of structs against the module signal table
    /// only, so the same write to a task or function local was dropped:
    /// `p[1][3:0] = v` and `p[0][7] = b` left `p` unchanged. Walk the
    /// local's declared type instead and splice the bits into its frame
    /// value. `None` when `lhs` is not such a select (the arms below take
    /// it); `Some(false)` when it names no bits.
    pub(super) fn frame_packed_select_write(
        &mut self,
        lhs: &Expression,
        val: &Value,
    ) -> Option<bool> {
        let (root, _, steps, hops, two) = self.frame_packed_target(lhs)?;
        let (w, sel) = self.packed_hops_eval(&hops, &steps);
        let Some((lsb, src, len)) = sel else {
            return Some(false);
        };
        let mut v = if val.is_real {
            Self::real_to_int(val.to_f64(), w)
        } else {
            val.resize_for_assign(w)
        };
        // §6.11.2: a 2-state variable stores x/z bits as 0.
        if two && v.has_xz() {
            v = v.to_two_state();
        }
        // §6.21: selected stores use the same live cell as whole stores.
        // A frame snapshot can be stale after a recursive static-local write.
        let mut stored = self.eval_expr(root.as_ref());
        let changed = stored.copy_bits_from(lsb as usize, &v, src as usize, len as usize);
        if changed {
            self.assign_value(root.as_ref(), &stored);
        }
        Some(changed)
    }

    /// The read of `frame_packed_select_write`'s selects, through the same
    /// walk, so a read returns the bits a write stores. The generic arms
    /// place a select below a struct member at the member's raw bit
    /// positions, so `s.f[4:1]` of a member declared `[8:1]` read the wrong
    /// bits (§7.4.1). Bits outside the variable read x, or 0 in a 2-state
    /// variable (§11.5.1).
    pub(super) fn frame_packed_select_read(&mut self, e: &Expression) -> Option<Value> {
        let (root, _, steps, hops, two) = self.frame_packed_target(e)?;
        let base = self.eval_expr(root.as_ref());
        let (w, sel) = self.packed_hops_eval(&hops, &steps);
        let w = w.max(1);
        let mut out = if two { Value::zero(w) } else { Value::new(w) };
        if let Some((lsb, src, len)) = sel {
            for b in 0..len as usize {
                out.set_bit(src as usize + b, base.get_bit(lsb as usize + b));
            }
        }
        Some(out)
    }

    /// §7.4.2: the packed dimensions of a packed array of a packed struct
    /// typedef (`e_t [0:1] b`), the struct counted as one trailing `[w-1:0]`
    /// dimension, as elaboration records them for a module variable. The
    /// other shape helpers stop at the struct and record nothing, so on a
    /// local or formal an element select took every dimension as `[n-1:0]`
    /// and `b[1]` of an ascending `[0:1]` read and wrote the wrong element.
    pub(super) fn struct_array_full_dims(&self, dt: &DataType) -> Option<Vec<(i64, i64)>> {
        if !matches!(dt, DataType::TypeReference { dimensions, .. } if !dimensions.is_empty()) {
            return None;
        }
        let (mut dims, agg, bw) = self.packed_shape(dt)?;
        agg?;
        dims.push((bw as i64 - 1, 0));
        Some(dims)
    }

    /// Bit slice (lsb, width) addressed by an index chain into a packed
    /// value of `total` bits with dimensions `dims` (outermost first), using
    /// the same slot arithmetic as the signal path: descending dimensions
    /// place index `lo` at the bottom, ascending ones at the top. None when
    /// an index is out of range or X.
    pub(super) fn packed_chain_slice(
        total: u64,
        dims: &[(i64, i64)],
        idx: &[Value],
    ) -> Option<(u64, u64)> {
        let mut w = total;
        let mut lsb: i64 = 0;
        for (k, iv) in idx.iter().enumerate() {
            let (dl, dr) = *dims.get(k)?;
            let count = (dl - dr).unsigned_abs() + 1;
            if count == 0 || w % count != 0 {
                return None;
            }
            w /= count;
            if iv.has_xz() {
                return None;
            }
            let i = iv.to_i64()?;
            let lo_b = dl.min(dr);
            let off = i - lo_b;
            if off < 0 || off as u64 >= count {
                return None;
            }
            let slot = if dl >= dr {
                off
            } else {
                (count as i64 - 1) - off
            };
            lsb += slot * w as i64;
        }
        Some((lsb as u64, w))
    }

    /// Width of the sub-array addressed by `n` indices into `dims`.
    pub(super) fn packed_chain_width(total: u64, dims: &[(i64, i64)], n: usize) -> u64 {
        let mut w = total;
        for k in 0..n.min(dims.len()) {
            let (dl, dr) = dims[k];
            let c = (dl - dr).unsigned_abs() + 1;
            if c > 0 && w % c == 0 {
                w /= c;
            }
        }
        w.max(1)
    }

    /// Cold nested-select path: walk the declaration instead of guessing
    /// bit offsets from the current value. An unpacked index changes storage;
    /// a packed index changes the region within that storage.
    pub(super) fn declared_packed_type(
        &self,
        dt: &DataType,
    ) -> Option<(DataType, Vec<(i64, i64)>, u32)> {
        let width = super::elaborate::resolve_type_width(
            dt,
            Some(&self.module.parameters),
            Some(&self.module.typedefs),
        );
        if width == 0 {
            return None;
        }
        let query = super::elaborate::type_query_dims_of(
            dt,
            Some(&self.module.parameters),
            Some(&self.module.typedefs),
            Some(&self.module.typedef_types),
        );
        let mut dimensions = query.dims;
        let mut element =
            super::elaborate::resolve_typedef_chain(dt, &self.module.typedef_types).clone();
        match &mut element {
            DataType::Struct(su) if su.packed => {
                for d in &su.dimensions {
                    let crate::ast::types::PackedDimension::Range { left, right, .. } = d else {
                        return None;
                    };
                    dimensions.push((
                        super::elaborate::const_eval_i64_with_params(
                            left,
                            Some(&self.module.parameters),
                        )?,
                        super::elaborate::const_eval_i64_with_params(
                            right,
                            Some(&self.module.parameters),
                        )?,
                    ));
                }
                su.dimensions.clear();
            }
            DataType::IntegerVector { .. }
            | DataType::IntegerAtom { .. }
            | DataType::Implicit { .. } => {}
            DataType::Enum(_) => {
                // §6.19: an enum's base declaration supplies its bit labels,
                // including a typedef base and an implicit int base.
                dimensions = self.packed_shape(dt)?.0;
            }
            _ => return None,
        }
        Some((element, dimensions, width))
    }

    /// §8.9: selected static properties share the same class cell as a
    /// whole-property write, including a member reached through an object.
    pub(super) fn declared_static_packed_root(
        &mut self,
        expr: &Expression,
    ) -> Option<DeclaredStaticAccess> {
        let (class, spec, member) = match &expr.kind {
            ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => {
                if self.frame_bound_bare_name(h).is_some()
                    || (h.path.len() == 1 && self.dyn_name_lookup(&h.path[0].name.name).is_some())
                {
                    return None;
                }
                if h.path.len() == 1 {
                    let member = h.path[0].name.name.clone();
                    if let Some(class) = self.class_context_stack.last().cloned().flatten() {
                        (class, None, member)
                    } else {
                        let handle = self.this_stack.last().copied().flatten()?;
                        let (class, spec) =
                            self.static_member_target(handle, None, None, &member)?;
                        (class, spec, member)
                    }
                } else if h.path.len() == 2 {
                    let first = &h.path[0].name.name;
                    let member = &h.path[1].name.name;
                    if self.module.classes.contains_key(first) {
                        (first.clone(), None, member.clone())
                    } else {
                        let handle = self.eval_ident_handle(first)?;
                        let (class, spec) =
                            self.static_member_target(handle, Some(first), None, member)?;
                        (class, spec, member.clone())
                    }
                } else {
                    return None;
                }
            }
            ExprKind::MemberAccess { expr: recv, member } => {
                if let ExprKind::Ident(h) = &recv.kind
                    && h.path.len() == 1
                    && self.module.classes.contains_key(&h.path[0].name.name)
                {
                    (h.path[0].name.name.clone(), None, member.name.clone())
                } else {
                    let handle = match &recv.kind {
                        ExprKind::This => self.this_stack.last().copied().flatten()?,
                        ExprKind::Ident(h) if h.path.len() == 1 => {
                            self.eval_ident_handle(&h.path[0].name.name)?
                        }
                        _ => return None,
                    };
                    let (class, spec) =
                        self.static_member_target(handle, None, None, &member.name)?;
                    (class, spec, member.name.clone())
                }
            }
            _ => return None,
        };
        if !self.member_is_static_prop(&class, &member) {
            return None;
        }
        let access = (|| {
            // §8.9 / §7.4: static collections retain their collection
            // backing store; their element type is not the property's shape.
            if self.member_is_static_coll(&class, &member)
                || self.static_fixed_key_in(&class, &member).is_some()
            {
                return None;
            }
            self.static_prop_key_spec(&class, &member, spec.as_ref())?;
            let (dt, unpacked) = self.class_property_decl_type(&class, &member)?;
            if !unpacked.is_empty() {
                return None;
            }
            let (element_type, dimensions, width) = self.declared_packed_type(&dt)?;
            // Static structs have an aliased member holder; retain its existing
            // read/write paths instead of creating a second backing value.
            if matches!(element_type, DataType::Struct(_)) {
                return None;
            }
            Some(DeclaredPackedAccess {
                storage: String::new(),
                is_signed: super::elaborate::is_type_signed_resolved(
                    &dt,
                    &self.module.typedef_types,
                ),
                static_target: Some((class, spec, member)),
                offset: 0,
                width,
                dimensions,
                element_type,
                unpacked: 0,
            })
        })();
        Some(match access {
            Some(value) => DeclaredStaticAccess::Packed(value),
            None => DeclaredStaticAccess::Other,
        })
    }

    pub(super) fn declared_packed_access(
        &mut self,
        expr: &Expression,
    ) -> Option<DeclaredPackedAccess> {
        match &expr.kind {
            ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => {
                if !self.class_member_names().statics.is_empty()
                    && let Some(access) = self.declared_static_packed_root(expr)
                {
                    return match access {
                        DeclaredStaticAccess::Packed(value) => Some(value),
                        DeclaredStaticAccess::Other => None,
                    };
                }
                let name = self
                    .frame_bound_bare_name(h)
                    .map(str::to_owned)
                    .unwrap_or_else(|| self.resolve_hier_name(h).into_owned());
                // Associative keys retain their key-type conversion path.
                if self.is_associative_array(&name) {
                    return None;
                }
                let dt = self.module.var_decl_types.get(&name)?;
                let (element_type, mut dimensions, mut width) = self.declared_packed_type(dt)?;
                // §23.10: a declaration inside an instantiated module sizes its
                // dimensions with THAT instance's parameters. The declaration
                // above was evaluated against the design-wide table, where a
                // child's `N` is stored as `<inst>.N`: a bare `N` misses (the
                // shape collapses to a plain 2-bit vector, so `mv[1][0]` read
                // one bit) or hits an unrelated top-level `N`. Elaboration
                // recorded this signal's dimensions per instance; use them.
                if let Some(recorded) = self.module.packed_full_dims.get(&name)
                    && matches!(
                        element_type,
                        DataType::IntegerVector { .. } | DataType::Implicit { .. }
                    )
                {
                    let mut total: u32 = 1;
                    for &(l, r) in recorded {
                        total = total.checked_mul(u32::try_from(l.abs_diff(r) + 1).ok()?)?;
                    }
                    dimensions = recorded.clone();
                    width = total;
                }
                let unpacked = if let Some((dims, _)) = self.module.arrays_nd.get(&name) {
                    dims.len()
                } else if self.module.arrays_2d.contains_key(&name) {
                    2
                } else if self.module.arrays.contains_key(&name)
                    || self.module.dynamic_arrays.contains(&name)
                {
                    1
                } else {
                    0
                };
                Some(DeclaredPackedAccess {
                    storage: name,
                    is_signed: super::elaborate::is_type_signed_resolved(
                        dt,
                        &self.module.typedef_types,
                    ),
                    static_target: None,
                    offset: 0,
                    width,
                    dimensions,
                    element_type,
                    unpacked,
                })
            }
            ExprKind::Paren(inner) => self.declared_packed_access(inner),
            ExprKind::Index { expr: base, index } => {
                if !Self::cond_expr_is_effect_free(index, false) {
                    return None;
                }
                let mut access = self.declared_packed_access(base)?;
                let index = self.eval_expr(index).to_index()?;
                if access.unpacked > 0
                    || self.module.dynamic_arrays.contains(&access.storage)
                    || self.module.queue_vars.contains(&access.storage)
                {
                    access.storage.push('[');
                    Self::push_i64(&mut access.storage, index);
                    access.storage.push(']');
                    access.unpacked = access.unpacked.saturating_sub(1);
                    return Some(access);
                }
                let (left, right) = access
                    .dimensions
                    .first()
                    .copied()
                    .unwrap_or((access.width as i64 - 1, 0));
                if index < left.min(right) || index > left.max(right) {
                    return None;
                }
                let count = u32::try_from(left.abs_diff(right) + 1).ok()?;
                if access.width % count != 0 {
                    return None;
                }
                access.width /= count;
                access.is_signed = false;
                let slot = if left >= right {
                    index - right
                } else {
                    right - index
                };
                access.offset = access
                    .offset
                    .checked_add((slot as u32).checked_mul(access.width)?)?;
                if !access.dimensions.is_empty() {
                    access.dimensions.remove(0);
                }
                Some(access)
            }
            ExprKind::RangeSelect {
                expr: base,
                left,
                right,
                kind,
            } => {
                if !Self::cond_expr_is_effect_free(left, false)
                    || !Self::cond_expr_is_effect_free(right, false)
                {
                    return None;
                }
                let mut access = self.declared_packed_access(base)?;
                if access.unpacked != 0 {
                    return None;
                }
                let (dl, dr) = access
                    .dimensions
                    .first()
                    .copied()
                    .unwrap_or((access.width as i64 - 1, 0));
                let a = self.eval_expr(left).to_index()?;
                let b = self.eval_expr(right).to_index()?;
                let (l, r) = match kind {
                    RangeKind::Constant => (a, b),
                    RangeKind::IndexedUp if b > 0 => {
                        if dl >= dr {
                            (a.checked_add(b - 1)?, a)
                        } else {
                            (a, a.checked_add(b - 1)?)
                        }
                    }
                    RangeKind::IndexedDown if b > 0 => {
                        if dl >= dr {
                            (a, a.checked_sub(b - 1)?)
                        } else {
                            (a.checked_sub(b - 1)?, a)
                        }
                    }
                    _ => return None,
                };
                if l < dl.min(dr)
                    || l > dl.max(dr)
                    || r < dl.min(dr)
                    || r > dl.max(dr)
                    || (l != r && (l > r) != (dl > dr))
                {
                    return None;
                }
                let count = u32::try_from(dl.abs_diff(dr) + 1).ok()?;
                if access.width % count != 0 {
                    return None;
                }
                let stride = access.width / count;
                access.is_signed = false;
                let slot = if dl >= dr { r - dr } else { dr - r };
                access.offset = access
                    .offset
                    .checked_add((slot as u32).checked_mul(stride)?)?;
                access.width = u32::try_from(l.abs_diff(r) + 1).ok()?.checked_mul(stride)?;
                if access.dimensions.is_empty() {
                    access.dimensions.push((l, r));
                } else {
                    access.dimensions[0] = (l, r);
                }
                Some(access)
            }
            ExprKind::MemberAccess { expr: base, member } => {
                if !self.class_member_names().statics.is_empty()
                    && let Some(access) = self.declared_static_packed_root(expr)
                {
                    return match access {
                        DeclaredStaticAccess::Packed(value) => Some(value),
                        DeclaredStaticAccess::Other => None,
                    };
                }
                let mut access = self.declared_packed_access(base)?;
                if access.unpacked != 0 || !access.dimensions.is_empty() {
                    return None;
                }
                let DataType::Struct(su) = &access.element_type else {
                    return None;
                };
                let mut remaining = access.width;
                for m in &su.members {
                    let (element, dimensions, width) = self.declared_packed_type(&m.data_type)?;
                    for d in &m.declarators {
                        if !d.dimensions.is_empty() {
                            return None;
                        }
                        let offset = if su.kind == crate::ast::types::StructUnionKind::Union {
                            0
                        } else {
                            remaining = remaining.checked_sub(width)?;
                            remaining
                        };
                        if d.name.name == member.name {
                            access.offset = access.offset.checked_add(offset)?;
                            access.width = width;
                            access.is_signed = super::elaborate::is_type_signed_resolved(
                                &m.data_type,
                                &self.module.typedef_types,
                            );
                            access.dimensions = dimensions;
                            access.element_type = element;
                            return Some(access);
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    pub(super) fn needs_declared_packed_access(&self, expr: &Expression) -> bool {
        match &expr.kind {
            ExprKind::Index { expr: base, .. } | ExprKind::RangeSelect { expr: base, .. } => {
                match &base.kind {
                    ExprKind::MemberAccess { .. } => true,
                    ExprKind::Index { .. } => {
                        let mut root = base.as_ref();
                        while let ExprKind::Index { expr, .. } = &root.kind {
                            root = expr;
                        }
                        let ExprKind::Ident(h) = &root.kind else {
                            return false;
                        };
                        let name = self
                            .frame_bound_bare_name(h)
                            .map(std::borrow::Cow::Borrowed)
                            .unwrap_or_else(|| self.resolve_hier_name(h));
                        // Ordinary vectors in unpacked memories already use the
                        // existing fast element path. Only packed geometry needs
                        // the declaration walk below the unpacked boundary.
                        self.module
                            .packed_full_dims
                            .get(name.as_ref())
                            .is_some_and(|d| d.len() > 1)
                            || self.module.packed_struct_fields.contains_key(name.as_ref())
                    }
                    _ => false,
                }
            }
            ExprKind::MemberAccess { expr: base, .. } => {
                matches!(base.kind, ExprKind::Index { .. })
            }
            _ => false,
        }
    }

    pub(super) fn needs_static_packed_access(&self, expr: &Expression) -> bool {
        let mut base = match &expr.kind {
            ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => expr.as_ref(),
            _ => return false,
        };
        // Values are seeded lazily: an empty value map must not discard the
        // first selected write. Eligibility comes from the declarations.
        let names = self.class_member_names();
        if names.statics.is_empty() {
            return false;
        }
        let candidate = |name: &str| {
            name.as_bytes()
                .first()
                .is_some_and(|&b| names.static_first[b as usize])
                && names.statics.contains(name)
        };
        loop {
            match &base.kind {
                ExprKind::Index { expr, .. }
                | ExprKind::RangeSelect { expr, .. }
                | ExprKind::Paren(expr) => base = expr,
                _ => break,
            }
        }
        match &base.kind {
            ExprKind::Ident(h) if h.path.len() == 1 => {
                (self.class_context_stack.last().is_some_and(Option::is_some)
                    || self.this_stack.last().copied().flatten().is_some())
                    && candidate(&h.path[0].name.name)
            }
            ExprKind::Ident(h) if h.path.len() == 2 => candidate(&h.path[1].name.name),
            ExprKind::MemberAccess { member, .. } => candidate(&member.name),
            _ => false,
        }
    }

    pub(super) fn packed_local_static_key(&self, name: &str) -> Option<String> {
        if self.in_const_param_eval || !self.static_local_names.contains(name) {
            return None;
        }
        let key = self.static_local_key_for(name)?;
        self.static_local_keys.contains(&key).then_some(key)
    }

    pub(super) fn packed_local_value(&self, name: &str) -> Option<Value> {
        if let Some(key) = self.packed_local_static_key(name) {
            return self.static_local_vars.get(&key).cloned();
        }
        self.get_local_or_signal(name)
    }

    pub(super) fn read_declared_packed_access(&mut self, expr: &Expression) -> Option<Value> {
        let access = self.declared_packed_access(expr)?;
        if access.unpacked != 0 || access.width == 0 {
            return None;
        }
        let value = if let Some((class, spec, member)) = &access.static_target {
            self.static_read_in(class, spec.clone(), member)?
        } else {
            self.packed_local_value(&access.storage)?
        };
        let end = access.offset.checked_add(access.width)?;
        if end > value.width {
            return None;
        }
        let mut selected = value.range_select((end - 1) as usize, access.offset as usize);
        selected.is_signed = access.is_signed;
        Some(selected)
    }

    pub(super) fn write_declared_packed_access(
        &mut self,
        expr: &Expression,
        value: &Value,
    ) -> Option<bool> {
        let access = self.declared_packed_access(expr)?;
        if access.unpacked != 0 || access.width == 0 {
            return None;
        }
        let mut stored = if let Some((class, spec, member)) = &access.static_target {
            self.static_read_in(class, spec.clone(), member)?
        } else {
            self.packed_local_value(&access.storage)?
        };
        if access.offset.checked_add(access.width)? > stored.width {
            return None;
        }
        let before = stored.clone();
        if access.static_target.is_none() && self.forced_names.contains(&access.storage) {
            return Some(false);
        }
        let fitted = if value.has_xz()
            && super::elaborate::is_type_two_state_resolved(
                &access.element_type,
                &self.module.typedef_types,
            ) {
            value.to_two_state().resize_for_assign(access.width)
        } else {
            value.resize_for_assign(access.width)
        };
        for bit in 0..access.width {
            stored.set_bit((access.offset + bit) as usize, fitted.get_bit(bit as usize));
        }
        let changed = stored != before;
        if let Some((class, spec, member)) = access.static_target {
            self.static_write_in(&class, spec, &member, stored);
        } else {
            if let Some(key) = self.packed_local_static_key(&access.storage) {
                self.static_local_vars.insert(key, stored.clone());
            }
            self.set_local_or_signal(&access.storage, stored);
        }
        Some(changed)
    }

    pub(super) fn select_base_elem_dim(&self, base: &Expression) -> Option<(i64, i64)> {
        let mut cur = base;
        let mut layers = 0usize;
        while let ExprKind::Index { expr: inner, .. } = &cur.kind {
            cur = inner;
            layers += 1;
        }
        if layers == 0 {
            return None;
        }
        let ExprKind::Ident(h) = &cur.kind else {
            return None;
        };
        let nm = self.resolve_hier_name(h);
        let dims = self.module.packed_full_dims.get(&*nm)?;
        let mut udepth = if let Some((sh, _)) = self.module.arrays_nd.get(&*nm) {
            sh.len()
        } else if self.module.arrays_2d.contains_key(&*nm) {
            2
        } else if self.module.arrays.contains_key(&*nm)
            || self.module.dynamic_arrays.contains(&*nm)
            || self.module.associative_arrays.contains_key(&*nm)
        {
            1
        } else {
            0
        };
        // Element-is-collection (array of queues): the flat element keys
        // `nm[...]` add one more unpacked layer.
        if udepth > 0 && layers > udepth {
            let elem_prefix = format!("{}[", nm);
            if self
                .module
                .dynamic_arrays
                .iter()
                .any(|k| k.starts_with(elem_prefix.as_str()))
                || self
                    .module
                    .associative_arrays
                    .keys()
                    .any(|k| k.starts_with(elem_prefix.as_str()))
            {
                udepth += 1;
            }
        }
        if layers < udepth {
            // An unpacked SLICE (a row of a 2-D array), not a packed element.
            return None;
        }
        dims.get(layers - udepth).copied()
    }

    /// `[N-1:0]` maps idx→(idx-lo)·w, an ASCENDING `[0:N-1]` mirrors the slot
    /// order (element 0 is the TOP slot). The raw `idx*elem_w` computation is
    /// only right for the normalized descending case.
    pub(super) fn packed_elem_lsb(&self, name: &str, idx: i64, elem_w: u32) -> Option<usize> {
        let lsb = match self
            .module
            .packed_full_dims
            .get(name)
            .and_then(|d| d.first())
        {
            None => idx * elem_w as i64,
            Some(&(dl, dr)) => {
                let (lo_b, hi_b) = (dl.min(dr), dl.max(dr));
                if idx < lo_b || idx > hi_b {
                    return None;
                }
                let count = hi_b - lo_b + 1;
                let off = idx - lo_b;
                let slot = if dl >= dr { off } else { count - 1 - off };
                slot * elem_w as i64
            }
        };
        (lsb >= 0).then_some(lsb as usize)
    }

    /// Whether a packed vector/array `name` is a 2-STATE (`bit`) type. Used
    /// to relativize an out-of-range element read to a known ZERO element
    /// (which a 2-state value cannot express as x; reference simulators do
    /// the same) instead of x, for a 4-state (`logic`) array keeps x.
    pub(super) fn packed_array_two_state(&self, name: &str) -> bool {
        self.module.var_decl_types.get(name).is_some_and(|dt| {
            super::elaborate::is_type_two_state_resolved(dt, &self.module.typedef_types)
        })
    }

    /// §7.4.1: bit position of LABEL `label` within one element of packed
    /// multi-D signal `name` — mirrors an ASCENDING inner dimension
    /// (`[0:W-1]` label 0 is the element's MSB). Descending or unregistered
    /// dims return the label unchanged.
    pub(super) fn packed_inner_bit(&self, name: &str, label: i64, elem_w: u32) -> Option<usize> {
        match self
            .module
            .packed_full_dims
            .get(name)
            .and_then(|d| d.get(1))
        {
            None => (label >= 0).then_some(label as usize),
            Some(&(dl, dr)) => {
                let (lo_b, hi_b) = (dl.min(dr), dl.max(dr));
                if label < lo_b || label > hi_b {
                    return None;
                }
                let count = hi_b - lo_b + 1;
                let off = label - lo_b;
                let slot = if dl >= dr { off } else { count - 1 - off };
                let _ = elem_w;
                (slot >= 0).then_some(slot as usize)
            }
        }
    }

    /// §7.4.1: whether a packed-struct MEMBER's declared (outermost) packed
    /// dimension is ASCENDING (`bit [0:15] b`), and the member's width — bit
    /// LABELS then mirror (`b[14:15]` is the LOW two bits). None for
    /// descending/undimensioned members.
    pub(super) fn struct_member_ascending(&self, root: &str, field: &str) -> Option<u32> {
        let dt = self.module.var_decl_types.get(root)?;
        let resolved = super::elaborate::resolve_typedef_chain(dt, &self.module.typedef_types);
        let crate::ast::types::DataType::Struct(su) = resolved else {
            return None;
        };
        let leaf = field.rsplit('.').next().unwrap_or(field);
        for m in &su.members {
            if !m.declarators.iter().any(|d| d.name.name == leaf) {
                continue;
            }
            let dims = match &m.data_type {
                crate::ast::types::DataType::IntegerVector { dimensions, .. } => dimensions,
                _ => return None,
            };
            // Multi-dim members (`bit [0:7][7:0] a`) are handled by the
            // ELEMENT-scaling path, which already normalizes slot order via
            // packed_full_dims — mirroring again here corrupted the offsets.
            if dims.len() != 1 {
                return None;
            }
            if let Some(crate::ast::types::PackedDimension::Range { left, right, .. }) =
                dims.first()
            {
                let l = super::elaborate::const_eval_i64_with_params(
                    left,
                    Some(&self.module.parameters),
                )?;
                let r = super::elaborate::const_eval_i64_with_params(
                    right,
                    Some(&self.module.parameters),
                )?;
                if l < r {
                    return Some((r - l + 1) as u32);
                }
            }
            return None;
        }
        None
    }

    /// §7.2.1: whether a packed-struct MEMBER's declared type is signed —
    /// slices of the parent are unsigned bit patterns, so member reads must
    /// re-stamp the declared signedness (`struct packed { int s; ... } x;`
    /// printed `x.s` as 4294967276 instead of -20). Resolves through the
    /// root's declared struct type; nested paths use the leaf member name.
    pub(super) fn struct_member_signed(&self, root: &str, field: &str) -> bool {
        let Some(dt) = self.module.var_decl_types.get(root) else {
            return false;
        };
        let resolved = super::elaborate::resolve_typedef_chain(dt, &self.module.typedef_types);
        let crate::ast::types::DataType::Struct(su) = resolved else {
            return false;
        };
        let leaf = field.rsplit('.').next().unwrap_or(field);
        fn find_signed(
            su: &crate::ast::types::StructUnionType,
            leaf: &str,
            module: &ElaboratedModule,
        ) -> bool {
            for m in &su.members {
                for d in &m.declarators {
                    if d.name.name == leaf {
                        return super::elaborate::is_type_signed_resolved(
                            &m.data_type,
                            &module.typedef_types,
                        );
                    }
                }
                if let crate::ast::types::DataType::Struct(inner) = &m.data_type {
                    if find_signed(inner, leaf, module) {
                        return true;
                    }
                }
            }
            false
        }
        find_signed(su, leaf, &self.module)
    }

    /// Leaf identifier name of an expression, if it is a plain (possibly
    /// scoped/dotted) identifier.
    /// `std::randomize(targets) with {items}` through the joint solver
    /// (`rand_csp_solve_scope`): plain integral scalars and 1-D arrays (at
    /// their current size) are its variables. NotApplicable when a target
    /// is anything else.
    pub(super) fn std_randomize_joint(
        &mut self,
        items: &[crate::ast::decl::ConstraintItem],
        targets: &[(String, Expression)],
    ) -> rand_csp::CspOutcome {
        let (scalars, arrays) = match self.std_randomize_vars(targets, false) {
            Ok(v) => v,
            Err(o) => return o,
        };
        let out = self.rand_csp_solve_scope(items, &scalars, &arrays);
        if std::env::var_os("XEZIM_RAND_DBG").is_some() {
            eprintln!(
                "[rand-dbg] std::randomize joint: targets {:?} -> {:?}",
                targets.iter().map(|t| &t.0).collect::<Vec<_>>(),
                out
            );
        }
        out
    }

    /// §18.5.8.1: `std::randomize(n, a) with {a.size() == n + 1; …}` solves
    /// the sizes of the dynamic-array targets whose `size()` a constraint
    /// reads first, jointly with the other targets, then the elements with
    /// the arrays at those sizes. None when no size is read or the joint
    /// solver does not model the call; otherwise whether it succeeded.
    pub(super) fn std_randomize_sized(
        &mut self,
        items: &[crate::ast::decl::ConstraintItem],
        targets: &[(String, Expression)],
    ) -> Option<bool> {
        let (scalars, arrays) = self.std_randomize_vars(targets, true).ok()?;
        let sized: HashSet<String> = arrays
            .iter()
            .filter(|a| {
                (self.module.dynamic_arrays.contains(&a.1)
                    || self
                        .signals
                        .contains_key(&Self::name_with_suffix(&a.1, ".size")))
                    && Self::items_read_size(items, &a.0)
            })
            .map(|a| a.0.clone())
            .collect();
        if sized.is_empty() {
            return None;
        }
        for _ in 0..4 {
            let sizes = match self.csp_scope_sizes(items, &scalars, &arrays, &sized) {
                Ok(s) => s,
                // A relaxation of the problem has no solution.
                Err(rand_csp::CspOutcome::Unsat) => return Some(false),
                Err(_) => return None,
            };
            for (name, n) in sizes {
                if let Some(a) = arrays.iter().find(|a| a.0 == name) {
                    let key = a.1.clone();
                    self.resize_random_dyn_target(&key, n);
                }
            }
            if matches!(
                self.std_randomize_joint(items, targets),
                rand_csp::CspOutcome::Sat
            ) {
                return Some(true);
            }
        }
        None
    }

    /// Whether some item reads `name.size()`.
    pub(super) fn items_read_size(items: &[crate::ast::decl::ConstraintItem], name: &str) -> bool {
        let wrapper = [crate::ast::decl::ClassConstraint {
            is_static: false,
            is_extern: false,
            has_body: true,
            name: crate::ast::Identifier {
                name: "__inline__".to_string(),
                span: crate::ast::Span::dummy(),
            },
            items: items.to_vec(),
            span: crate::ast::Span::dummy(),
        }];
        let c = RandColl {
            prop: name.to_string(),
            scoped: name.to_string(),
            kind: CollKind::Dyn,
            width: 1,
            lo: 0,
            hi: 0,
            nested: false,
            is_object_elem: false,
        };
        Self::csp_size_solved(&c, &wrapper)
    }

    /// The joint solver's variables for `std::randomize(targets)`: plain
    /// integral scalars and 1-D arrays at their current size. An empty
    /// array is listed only with `empty` (the size phase sizes it).
    #[allow(clippy::type_complexity)]
    pub(super) fn std_randomize_vars(
        &mut self,
        targets: &[(String, Expression)],
        empty: bool,
    ) -> Result<
        (
            Vec<(String, Expression, u32, bool, Option<Vec<u64>>)>,
            Vec<(String, String, Vec<i64>, u32, bool, Option<Vec<u64>>)>,
        ),
        rand_csp::CspOutcome,
    > {
        let mut scalars = Vec::new();
        let mut arrays = Vec::new();
        for (name, lv) in targets {
            let enum_vals = |me: &Self, n: &str| {
                me.declared_enum_members(n)
                    .filter(|m| !m.is_empty())
                    .map(|m| m.iter().map(|x| x.1).collect::<Vec<u64>>())
            };
            if let Some(nm) = self.array_operand_name(lv) {
                if self.module.arrays_nd.contains_key(&nm)
                    || self.module.arrays_2d.contains_key(&nm)
                {
                    return Err(rand_csp::CspOutcome::NotApplicable);
                }
                let dynamic = self.module.dynamic_arrays.contains(&nm)
                    || self
                        .signals
                        .contains_key(&Self::name_with_suffix(&nm, ".size"));
                let idx: Vec<i64> = if dynamic {
                    (0..self.get_queue_size(&nm) as i64).collect()
                } else if let Some(&(lo, hi, _)) = self.module.arrays.get(&nm) {
                    (lo.min(hi)..=lo.max(hi)).collect()
                } else {
                    return Err(rand_csp::CspOutcome::NotApplicable);
                };
                // §18.12: a dynamic array or queue local to an automatic
                // task is stored under a per-call key (`declare_local_dyn`);
                // the constraints name it as written, so the solver knows it
                // by that name and reads and writes its elements by the key.
                let written = match &lv.kind {
                    ExprKind::Ident(h)
                        if h.root.is_none()
                            && h.path.len() == 1
                            && h.path[0].selects.is_empty()
                            && h.path[0].name.name != nm =>
                    {
                        h.path[0].name.name.clone()
                    }
                    _ => nm.clone(),
                };
                let Some(first) = idx.first() else {
                    // An empty array has no element variables.
                    if empty && dynamic {
                        let w = self
                            .module
                            .arrays
                            .get(&nm)
                            .map(|t| t.2 as u32)
                            .unwrap_or(32);
                        let ev = enum_vals(self, &nm);
                        arrays.push((written, nm, idx, w.max(1), false, ev));
                    }
                    continue;
                };
                let Some(v0) = self.get_signal_value_by_name(&format!("{}[{}]", nm, first)) else {
                    return Err(rand_csp::CspOutcome::NotApplicable);
                };
                if v0.is_real {
                    return Err(rand_csp::CspOutcome::NotApplicable);
                }
                let w = self
                    .module
                    .arrays
                    .get(&nm)
                    .map(|t| t.2 as u32)
                    .unwrap_or(v0.width);
                if w > 64 && !(empty && dynamic) {
                    // Left as drawn: a constraint reading it is judged by the
                    // final check.
                    continue;
                }
                // The declared element type decides the signedness: an
                // element value written by the per-item repair above may
                // carry a literal's (`bc[i] == 1` left a `bit` element
                // signed, and the solve then had no value 1 for it).
                let signed = match self.module.var_decl_types.get(&nm) {
                    Some(dt) => {
                        super::elaborate::is_type_signed_resolved(dt, &self.module.typedef_types)
                    }
                    None => v0.is_signed || self.signed_signals.contains(&nm),
                };
                let ev = enum_vals(self, &nm);
                arrays.push((written, nm, idx, w.max(1), signed, ev));
                continue;
            }
            if let Some((root, _)) = name.split_once('.') {
                // A field of a packed struct that is itself a target: a view
                // of that variable, not a variable of its own.
                if targets.iter().any(|(n, _)| n == root) {
                    continue;
                }
                return Err(rand_csp::CspOutcome::NotApplicable);
            }
            if !matches!(lv.kind, ExprKind::Ident(_)) {
                return Err(rand_csp::CspOutcome::NotApplicable);
            }
            let cur = self.eval_expr(lv);
            if cur.is_real {
                return Err(rand_csp::CspOutcome::NotApplicable);
            }
            // A local wider than 64 bits joins as segments (`csp_scope`).
            let ev = enum_vals(self, name);
            scalars.push((
                name.clone(),
                lv.clone(),
                cur.width.max(1),
                cur.is_signed,
                ev,
            ));
        }
        Ok((scalars, arrays))
    }

    /// The root variable a `std::randomize` target names: `v`, `s.f`,
    /// `a[i]` all name `v`/`s`/`a`.
    pub(super) fn randomize_target_root(e: &Expression) -> Option<String> {
        match &e.kind {
            ExprKind::Ident(h) => h.path.first().map(|s| s.name.name.clone()),
            ExprKind::Paren(x)
            | ExprKind::MemberAccess { expr: x, .. }
            | ExprKind::Index { expr: x, .. }
            | ExprKind::RangeSelect { expr: x, .. } => Self::randomize_target_root(x),
            _ => None,
        }
    }

    pub(super) fn expr_leaf_name(e: &Expression) -> Option<String> {
        match &e.kind {
            ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.clone()),
            ExprKind::Paren(inner) => Self::expr_leaf_name(inner),
            _ => None,
        }
    }

    /// A signed integer literal, possibly under unary minus/plus or parens —
    /// the shapes whose ===/!== widening sign-extends by consensus.
    pub(super) fn expr_is_signed_literal(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Number(crate::ast::expr::NumberLiteral::Integer { signed, .. }) => *signed,
            ExprKind::Unary { op, operand } => {
                matches!(
                    op,
                    crate::ast::expr::UnaryOp::Minus | crate::ast::expr::UnaryOp::Plus
                ) && matches!(
                    &operand.kind,
                    ExprKind::Number(crate::ast::expr::NumberLiteral::Integer { .. })
                )
            }
            ExprKind::Paren(inner) => Self::expr_is_signed_literal(inner),
            _ => false,
        }
    }

    /// A streaming concatenation, looking through parentheses.
    pub(super) fn expr_is_stream(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::StreamOp { .. } => true,
            ExprKind::Paren(inner) => Self::expr_is_stream(inner),
            _ => false,
        }
    }

    pub fn eval_expr(&mut self, expr: &Expression) -> Value {
        self.eval_expr_ctx(expr, 0)
    }

    /// Outlined from `eval_expr_ctx` (see `exec_stmt_blocking_assign`): keeps
    /// the dispatcher's stack frame small.
    #[inline(never)]
    pub(super) fn eval_expr_system_call(
        &mut self,
        expr: &Expression,
        ctx_width: u32,
        name: &String,
        args: &Vec<Expression>,
    ) -> Value {
        let mut sys_result = self.eval_expr_system_call_body(expr, ctx_width, name, args);
        // §20/§21: an `int`/`integer`-valued function returns a SIGNED
        // value (`$countones(x) - 8 < 0`, `$fgetc(fd) < 0` on EOF, and the
        // §20.6.2/§20.7 queries `$bits(a) - 200`, `$unpacked_dimensions(a)
        // - 3`). The arms build unsigned words, and several of them return
        // early; stamp the sign here, once, from the shared result-type
        // table, so every arm is covered.
        if let Some((32, true)) = super::bytecode::system_function_result(name) {
            if sys_result.width == 32 && !sys_result.is_real {
                sys_result.is_signed = true;
            }
        }
        sys_result
    }

    pub(super) fn eval_expr_system_call_body(
        &mut self,
        expr: &Expression,
        ctx_width: u32,
        name: &String,
        args: &Vec<Expression>,
    ) -> Value {
        match name.as_str() {
            COV_COND_FN => return self.cov_cond(args),
            // §9.4.5: retrieve a pre-evaluated intra-assignment RHS
            // (stashed by the suspend path; see make_intra_saved_expr).
            INTRA_SAVED_MARKER => {
                let idx = args
                    .first()
                    .map(|a| self.eval_expr(a).to_u64().unwrap_or(0))
                    .unwrap_or(0);
                self.intra_saved
                    .remove(&idx)
                    .unwrap_or_else(|| Value::zero(32))
            }
            // `@prop` on a non-event class property: true once the property
            // differs from the value captured when the wait was armed.
            VC_CHANGED_MARKER => {
                let idx = args
                    .first()
                    .map(|a| self.eval_expr(a).to_u64().unwrap_or(0))
                    .unwrap_or(0);
                let cur = args
                    .get(1)
                    .map(|a| self.eval_expr(a))
                    .unwrap_or_else(|| Value::zero(1));
                let changed = self.vc_saved.get(&idx).is_none_or(|(_, prev)| *prev != cur);
                Value::from_u64(changed as u64, 1)
            }
            // §9.4.5 marker reached from a context that cannot suspend
            // (always-block settle, function body, bytecode fallback):
            // degrade to the RHS value with the delay ignored.
            crate::intra_delay::INTRA_DELAY_MARKER => args
                .get(1)
                .map(|a| self.eval_expr(a))
                .unwrap_or_else(|| Value::zero(32)),
            // LRM §16.9.3 `$past(<sig>[, N])` — value of `<sig>` from
            // N clock cycles ago (default N=1). Only meaningful
            // inside an SVA clocked body; falls back to current
            // value when no active site (compile-time use).
            "$past" => {
                let n = args
                    .get(1)
                    .filter(|a| !matches!(a.kind, ExprKind::Null | ExprKind::Empty))
                    .map(|a| self.eval_expr(a).to_u64().unwrap_or(1) as usize)
                    .unwrap_or(1);
                if let Some((_, past)) = self.sampled_fn_via_watch(expr, args, n) {
                    return past.unwrap_or_else(|| {
                        Value::new(
                            args.first()
                                .map(|a| self.infer_width(a))
                                .unwrap_or(1)
                                .max(1),
                        )
                    });
                }
                // Inside a concurrent assertion the site's per-cycle ring is the
                // history (index N = N clock ticks ago, sampled values).
                if let Some(site_idx) = self.active_sva_site {
                    let sig_name = match args.first().map(|a| &a.kind) {
                        Some(ExprKind::Ident(h)) => self.resolve_hier_name(h).into_owned(),
                        _ => String::new(),
                    };
                    let n_cycles = args
                        .get(1)
                        .filter(|a| !matches!(a.kind, ExprKind::Null))
                        .map(|a| self.eval_expr(a).to_u64().unwrap_or(1) as usize)
                        .unwrap_or(1);
                    if let Some(ring) = self
                        .sva_sites
                        .get(site_idx)
                        .and_then(|s| s.past_snapshots.get(sig_name.as_str()))
                    {
                        if let Some(v) = ring.get(n_cycles) {
                            return v.clone();
                        }
                        // §16.9.3: no such tick yet (before the first clock):
                        // the expression's default value, not its current one.
                        let w = args.first().map(|a| self.infer_width(a)).unwrap_or(1);
                        return Value::new(w.max(1));
                    }
                }
                // §16.9.3 explicit clocking (4th arg) or default
                // clocking in a procedural context.
                {
                    let n = args
                        .get(1)
                        .filter(|a| !matches!(a.kind, ExprKind::Null))
                        .map(|a| self.eval_expr(a).to_u64().unwrap_or(1) as usize)
                        .unwrap_or(1);
                    if let Some((_, past_opt)) = self.sampled_fn_via_watch(expr, args, n) {
                        if let Some(p) = past_opt {
                            return p;
                        }
                        // Not enough history yet: LRM default-value = the
                        // operand type's default (x for 4-state), at the
                        // OPERAND's width.
                        let w = args.first().map(|a| self.infer_width(a)).unwrap_or(1);
                        return Value::new(w.max(1));
                    }
                }
                let sig_name = match args.first().map(|a| &a.kind) {
                    Some(ExprKind::Ident(h)) => self.resolve_hier_name(h),
                    _ => std::borrow::Cow::Owned(String::new()),
                };
                let n_cycles = args
                    .get(1)
                    .map(|a| self.eval_expr(a).to_u64().unwrap_or(1) as usize)
                    .unwrap_or(1);
                // `refresh_sva_past_snapshots` pushes the CURRENT
                // value to ring[0] before body eval, so the value
                // from N cycles ago lives at ring[N] (with
                // ring[0] = current, ring[1] = 1 cycle ago, …).
                let idx = n_cycles;
                if let Some(site_idx) = self.active_sva_site {
                    if let Some(ring) = self
                        .sva_sites
                        .get(site_idx)
                        .and_then(|s| s.past_snapshots.get(&*sig_name))
                    {
                        if let Some(v) = ring.get(idx) {
                            return v.clone();
                        }
                    }
                }
                self.get_signal_value_by_name(&sig_name)
                    .unwrap_or_else(|| Value::zero(32))
            }
            // LRM §16.9.3 sampled-value edge functions, all
            // expressed in terms of `$past`: `$rose(x)` = was 0,
            // is 1 now; `$fell(x)` = was 1, is 0 now;
            // `$stable(x)` = value unchanged from previous cycle;
            // `$changed(x)` = value differs from previous cycle.
            "$rose" | "$fell" | "$stable" | "$changed" => {
                // §16.9.3 explicit clocking argument (or default
                // clocking, procedural context): sample via the
                // clock-edge watch history.
                if let Some((cur, prev_opt)) = self.sampled_fn_via_watch(expr, args, 1) {
                    let result = Self::sampled_change_fn(&name, &cur, prev_opt.as_ref());
                    return if result {
                        Value::ones(1)
                    } else {
                        Value::zero(1)
                    };
                }
                let sig_name = match args.first().map(|a| &a.kind) {
                    Some(ExprKind::Ident(h)) => self.resolve_hier_name(h),
                    _ => std::borrow::Cow::Owned(String::new()),
                };
                let cur = self
                    .get_signal_value_by_name(&sig_name)
                    .unwrap_or_else(|| Value::zero(32));
                let prev_opt = self.active_sva_site.and_then(|si| {
                    self.sva_sites
                        .get(si)
                        .and_then(|s| s.past_snapshots.get(&*sig_name))
                        .and_then(|r| r.get(1).cloned())
                });
                if Self::sampled_change_fn(&name, &cur, prev_opt.as_ref()) {
                    Value::ones(1)
                } else {
                    Value::zero(1)
                }
            }
            // String-returning formatting functions. `$sformatf`/`$psprintf`
            // take a format string followed by values and return the formatted result
            // (IEEE 1800-2023 §21.3.3).
            "$sformatf" | "$psprintf" => {
                let s = self.format_args(args, "$display");
                Value::from_string(&s)
            }
            "$cast" => {
                // `$cast(dest, src)` as a function: succeeds (returns 1 and
                // assigns) iff src's object is assignment-compatible with
                // dest's declared class type. Returning 1 unconditionally
                // broke `if (!$cast(target, item))` dynamic type discrimination,
                // misrouting items to invalid branches.
                if args.len() >= 2 {
                    let v = self.eval_expr(&args[1]);
                    let ok = self.cast_type_ok(&args[0], &v);
                    if !ok && std::env::var("XEZIM_CAST_DBG").is_ok() {
                        let h = v.to_u64().unwrap_or(0) as usize;
                        let src_cls = self
                            .heap
                            .get(h)
                            .and_then(|o| o.as_ref())
                            .map(|i| i.class_name.clone())
                            .unwrap_or_else(|| "<not-an-object>".into());
                        let dest_cls = match &args[0].kind {
                            ExprKind::Ident(hh) if hh.path.len() == 1 => {
                                self.class_of_var(&hh.path[0].name.name)
                            }
                            _ => None,
                        };
                        let (ovl, glob, depth) = match &args[0].kind {
                            ExprKind::Ident(hh) if hh.path.len() == 1 => (
                                self.local_class_type_of(&hh.path[0].name.name),
                                self.var_class_types.get(&hh.path[0].name.name).cloned(),
                                self.local_type_stack.len(),
                            ),
                            _ => (None, None, 0),
                        };
                        eprintln!(
                            "[CAST] t={} FAIL src='{}' dest={:?} overlay={:?} global={:?} depth={}",
                            self.time, src_cls, dest_cls, ovl, glob, depth
                        );
                        eprintln!(
                            "[CAST]   method_local_base={:?} frames={:?}",
                            self.method_local_base,
                            self.local_type_stack
                                .iter()
                                .map(|(c, _)| c.iter().collect::<Vec<_>>())
                                .collect::<Vec<_>>()
                        );
                    }
                    if ok {
                        self.assign_value(&args[0], &v);
                        Value::from_u64(1, 32)
                    } else {
                        Value::zero(32)
                    }
                } else {
                    Value::zero(32)
                }
            }
            // §6.24.1 literal size cast `N'(x)` — lowered by the parser.
            "$__xz_size_cast" => {
                // §6.24.3: an unpacked operand packs its elements.
                if let Some(v) = self.cast_of_unpacked_operand(expr, ctx_width, name, args) {
                    return v;
                }
                let n = args
                    .first()
                    .map(|a| self.eval_expr(a).to_u64().unwrap_or(32))
                    .unwrap_or(32) as u32;
                if let Some(a) = args.get(1) {
                    // §6.24.1: the cast size is the operand's evaluation
                    // CONTEXT, as if assigned to a variable of that width —
                    // `5'(3'd7 + 3'd6)` computes the sum at 5 bits (13),
                    // not at the operands' self-determined 3 bits (5).
                    let v = self.eval_expr_ctx(a, n.max(1)).resize(n.max(1));
                    return v;
                }
                Value::zero(n.max(1))
            }
            "$clog2" => {
                let v = args
                    .first()
                    .map(|a| self.eval_expr(a).to_u64().unwrap_or(0))
                    .unwrap_or(0);
                // §20.8.1: $clog2(0) and $clog2(1) are both 0.
                Value::from_u64(
                    if v <= 1 {
                        0
                    } else {
                        64 - (v - 1).leading_zeros()
                    } as u64,
                    32,
                )
            }
            "$bits" => {
                if let Some(arg) = args.first() {
                    // §5.9: the empty string literal `""` is equivalent to
                    // `"\0"` — one character, 8 bits (its VALUE stays a
                    // width-0 zero elsewhere; only the size query sees the
                    // implicit NUL).
                    if matches!(&arg.kind, ExprKind::StringLiteral(s) if s.is_empty()) {
                        return Value::from_u64(8, 32);
                    }
                    // §20.6.2: `$bits(logic [7:0])` — a TYPE operand.
                    if let ExprKind::TypeLiteral(dt) = &arg.kind {
                        if let Some(w) = self.bound_type_param_width(dt) {
                            return Value::from_u64(w as u64, 32);
                        }
                        if let Some(w) = self.bound_agg_width(dt) {
                            return Value::from_u64(w as u64, 32);
                        }
                        // Inside a parameterized class method the type may
                        // be sized by a CLASS value parameter
                        // (`$bits(logic [W-1:0])`), which is absent from
                        // the module parameter map — the width collapsed
                        // to 1. Overlay the active specialization, and
                        // resolve a bare TYPE-PARAMETER name through its
                        // declared default AST (`parameter type T = logic
                        // [W-1:0]`), which likewise sizes from W.
                        let spec = self.params_with_class_spec();
                        let params = spec.as_ref().unwrap_or(&self.module.parameters);
                        let resolved_dt = self.class_type_param_default(dt);
                        let target = resolved_dt.as_ref().unwrap_or(dt);
                        let w = super::elaborate::resolve_type_width(
                            target,
                            Some(params),
                            Some(&self.module.typedefs),
                        )
                        .max(1);
                        return Value::from_u64(w as u64, 32);
                    }
                    // §8.23/§20.6.2: `$bits(pkg::cls::td)` — a `::`-chained
                    // scoped TYPE name parses as nested member accesses, not
                    // an Ident. Resolve it against the typedef tables by its
                    // FULL qualified key (no suffix fallback: `$bits(s.m)` on
                    // a real struct member must keep falling through to the
                    // member paths below).
                    if matches!(&arg.kind, ExprKind::MemberAccess { .. }) {
                        if let Some(key) = Self::expr_scoped_chain_key(arg) {
                            if let Some(&w) = self.module.typedefs.get(&key) {
                                return Value::from_u64(w as u64, 32);
                            }
                        }
                    }
                    if let ExprKind::Ident(hier) = &arg.kind {
                        let name = self.resolve_hier_name(hier);
                        // A bare TYPE PARAMETER of the enclosing class
                        // (`$bits(T)` with `parameter type T = logic
                        // [W-1:0]`) parses as an identifier, not a type
                        // literal — size its default against the active
                        // specialization's value parameters.
                        if let Some(leaf) = hier.path.last().map(|s| s.name.name.clone()) {
                            let as_ty = crate::ast::types::DataType::TypeReference {
                                name: crate::ast::types::TypeName {
                                    scopes: Vec::new(),
                                    name: crate::ast::Identifier {
                                        name: leaf,
                                        span: arg.span,
                                    },
                                    span: arg.span,
                                },
                                dimensions: Vec::new(),
                                type_args: Vec::new(),
                                span: arg.span,
                            };
                            if let Some(dflt) = self.class_type_param_default(&as_ty) {
                                let spec = self.params_with_class_spec();
                                let params = spec.as_ref().unwrap_or(&self.module.parameters);
                                let w = super::elaborate::resolve_type_width(
                                    &dflt,
                                    Some(params),
                                    Some(&self.module.typedefs),
                                )
                                .max(1);
                                return Value::from_u64(w as u64, 32);
                            }
                        }
                        // IEEE 1800-2017 §23.8 name resolution: inside a
                        // class method, an unqualified identifier resolves
                        // to a class member BEFORE module scope. The
                        // module-scope `arrays`/`typedefs` tables below must
                        // NOT shadow a same-named class property (a frequent
                        // collision with common names like `i`). When `this`
                        // is bound and `name` is a property of the current
                        // class (walking inheritance), size the member from
                        // its own value — exactly as `$bits(this.name)` does.
                        if let Some(this_h) = self.this_stack.last().copied().flatten() {
                            if let Some(inst) = self.heap.get(this_h).and_then(|o| o.as_ref()) {
                                let mut cur = Some(inst.class_name.clone());
                                let mut found_member = false;
                                while let Some(cn) = &cur {
                                    if let Some(cd) = self.module.classes.get(cn) {
                                        if cd.properties.contains_key(&*name) {
                                            found_member = true;
                                            break;
                                        }
                                        cur = cd.extends.clone();
                                    } else {
                                        break;
                                    }
                                }
                                if found_member {
                                    // §20.6.2: a fixed-size UNPACKED array
                                    // member is element_bits × the element
                                    // count; its per-instance storage is
                                    // `<handle>#<name>`, not one value.
                                    let scoped = format!("{}#{}", this_h, name);
                                    if !self.module.dynamic_arrays.contains(&scoped) {
                                        if let Some(&(lo, hi, ew)) = self.module.arrays.get(&scoped)
                                        {
                                            let n = (hi - lo).unsigned_abs() + 1;
                                            return Value::from_u64(n * ew as u64, 32);
                                        }
                                        if let Some(bits) = self.unpacked_nd_bits(&scoped) {
                                            return Value::from_u64(bits, 32);
                                        }
                                    }
                                    return Value::from_u64(self.eval_expr(arg).width as u64, 32);
                                }
                            }
                        }
                        // §20.6.2: a DYNAMIC array or queue is sized by its
                        // CURRENT contents — element_bits × size(), 0 when
                        // empty. Its `arrays` row holds a (0, 63)
                        // registration placeholder, which read 64 elements.
                        if self.module.dynamic_arrays.contains(&*name) {
                            if let Some(&(_, _, ew)) = self.module.arrays.get(&*name) {
                                let n = self.get_queue_size(&name);
                                return Value::from_u64(n * ew as u64, 32);
                            }
                        }
                        // Unpacked array: $bits = element_bits * product of
                        // dimension sizes (IEEE 1800-2017 §20.6.2). The
                        // signal's own Value holds only one element, so the
                        // array shape must be consulted explicitly.
                        if let Some(&(lo, hi, ew)) = self.module.arrays.get(&*name) {
                            let n = (hi - lo).unsigned_abs() + 1;
                            return Value::from_u64(n * ew as u64, 32);
                        }
                        if let Some(&((a0, a1), (b0, b1), ew)) = self.module.arrays_2d.get(&*name) {
                            let n = ((a1 - a0).unsigned_abs() + 1) * ((b1 - b0).unsigned_abs() + 1);
                            return Value::from_u64(n * ew as u64, 32);
                        }
                        if let Some((shape, ew)) = self.module.arrays_nd.get(&*name) {
                            let n: u64 = shape
                                .iter()
                                .map(|(l, h)| (h - l).unsigned_abs() + 1)
                                .product();
                            return Value::from_u64(n * *ew as u64, 32);
                        }
                        // §8.25/§6.18: a typedef of a parameterized class
                        // (`typedef logic [W-1:0] r;`) is sized by the
                        // receiver's specialization, not the elaboration-
                        // time default the table holds.
                        if let Some(this_h) = self.this_stack.last().copied().flatten()
                            && self
                                .heap
                                .get(this_h)
                                .and_then(|o| o.as_ref())
                                .is_some_and(|i| self.class_is_parameterized(&i.class_name))
                            && !self.module.typedef_unpacked_dims.contains_key(&*name)
                            && let Some(dt) = self.module.typedef_types.get(&*name).cloned()
                        {
                            let params = self.instance_param_scope(this_h);
                            let w = super::elaborate::resolve_type_width(
                                &dt,
                                Some(&params),
                                Some(&self.module.typedefs),
                            )
                            .max(1);
                            return Value::from_u64(w as u64, 32);
                        }
                        // §6.18/§23.10: a typedef declared in an INLINED
                        // child module survives only under its instance-
                        // scoped key (`dut.foo_t`) — the bare key is
                        // restored after inlining so differently
                        // parameterized instances don't collide. Resolve
                        // through the process's scope hint, outermost
                        // scope walked inward (ivtest sv_enum1).
                        let typedef_w = self.module.typedefs.get(&*name).copied().or_else(|| {
                            let mut h = self.name_resolve_hint.borrow().clone()?;
                            loop {
                                if let Some(&w) =
                                    self.module.typedefs.get(&format!("{}.{}", h, name))
                                {
                                    return Some(w);
                                }
                                match h.rfind('.') {
                                    Some(i) => h.truncate(i),
                                    None => return None,
                                }
                            }
                        });
                        if let Some(w) = typedef_w {
                            // §20.6.2: a typedef carrying UNPACKED dims is
                            // element_bits × the dimension sizes —
                            // `typedef int T[3:0]` is 128, not 32. The
                            // table holds only the element width, so the
                            // dims have to be folded in here.
                            let mult: u64 = Self::typedef_dims_via_tables(
                                &self.module,
                                self.class_context_stack
                                    .last()
                                    .cloned()
                                    .flatten()
                                    .unwrap_or_default()
                                    .as_str(),
                                &name,
                            )
                            .map(|dims| {
                                use crate::ast::types::UnpackedDimension;
                                dims.iter()
                                    .map(|d| match d {
                                        UnpackedDimension::Range { left, right, .. } => {
                                            let l = super::elaborate::const_eval_i64_with_params(
                                                left,
                                                Some(&self.module.parameters),
                                            )
                                            .unwrap_or(0);
                                            let r = super::elaborate::const_eval_i64_with_params(
                                                right,
                                                Some(&self.module.parameters),
                                            )
                                            .unwrap_or(0);
                                            (l - r).unsigned_abs() + 1
                                        }
                                        UnpackedDimension::Expression { expr, .. } => {
                                            super::elaborate::const_eval_i64_with_params(
                                                expr,
                                                Some(&self.module.parameters),
                                            )
                                            .unwrap_or(1)
                                            .max(1)
                                                as u64
                                        }
                                        // A dynamic/queue/assoc dimension has
                                        // no compile-time size; §20.6.2 leaves
                                        // $bits on those unspecified.
                                        _ => 1,
                                    })
                                    .product::<u64>()
                            })
                            .unwrap_or(1);
                            return Value::from_u64(w as u64 * mult.max(1), 32);
                        }
                        // §20.6.2: `$bits(<type>)` where the type is a bare
                        // keyword (`$bits(integer)`, `$bits(byte)`) — the
                        // keyword parses as an Ident, so map it to the
                        // self-determined width of the type.
                        let leaf = hier.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                        if let Some(w) = atom_type_keyword_width(leaf) {
                            return Value::from_u64(w as u64, 32);
                        }
                        // §8.25: `$bits(T)` where `T` is a parameterized-
                        // class TYPE parameter — resolve the active
                        // specialization's binding (instance
                        // `type_bindings` / `current_spec`) to the
                        // concrete type and size that instead.
                        if hier.path.len() == 1 {
                            if let Some(concrete) = self.resolve_type_param_binding(leaf) {
                                if let Some(w) = atom_type_keyword_width(&concrete) {
                                    return Value::from_u64(w as u64, 32);
                                }
                                if let Some(w) = vector_fragment_width(&concrete) {
                                    return Value::from_u64(w as u64, 32);
                                }
                                if let Some(w) = self.module.typedefs.get(&concrete) {
                                    return Value::from_u64(*w as u64, 32);
                                }
                            }
                        }
                    }
                    // §20.6.2: `$bits(s.arr)` on a struct member declared
                    // with UNPACKED dimensions is the WHOLE array —
                    // element_bits × element count. An unpacked struct is
                    // stored as per-element leaf signals (`s.arr[0]`, …)
                    // with no `s.arr` of its own, so neither the array
                    // tables above nor the width fallback below could see
                    // past one element.
                    if let Some(flat) = self.flat_member_name(arg) {
                        if flat.contains('.') {
                            if let Some((elem_dt, Some(lists))) = self.flat_path_type(&flat) {
                                let ew = super::elaborate::resolve_type_width(
                                    &elem_dt,
                                    Some(&self.module.parameters),
                                    Some(&self.module.typedefs),
                                )
                                .max(1);
                                let n: u64 = lists.iter().map(|l| l.len() as u64).product();
                                return Value::from_u64(ew as u64 * n, 32);
                            }
                        }
                    }
                    // §7.8: `$bits(aa[i])` — the indexed ELEMENT of an
                    // associative-array collection reports its DECLARED
                    // element width, independent of the runtime value's
                    // stored width. UVM's `uvm_pack_int/unpack_int` macros
                    // size each element by `$bits`, so without this they
                    // could pack an assoc-of-shortint as 32-bit (desync).
                    if let ExprKind::Index { expr: idx_base, .. } = &arg.kind {
                        if let Some(an) = self.expr_assoc_name(idx_base) {
                            if let Some(w) = self.assoc_elem_width(&an) {
                                return Value::from_u64(w as u64, 32);
                            }
                        }
                    }
                    // §20.6.2: `$bits(o.arr)` on an unpacked-array member of
                    // an object is the whole array (element_bits × element
                    // count, the current count for a queue or dynamic array),
                    // as for `$bits(arr)` inside the class above. The member
                    // lives in the instance-scoped `<handle>#arr` storage.
                    if !matches!(arg.kind, ExprKind::Index { .. })
                        && let Some(nm) = self.expr_assoc_name(arg).filter(|n| n.contains('#'))
                    {
                        if let Some(&(lo, hi, ew)) = self.module.arrays.get(&nm) {
                            let n = if self.module.dynamic_arrays.contains(&nm) {
                                self.get_queue_size(&nm)
                            } else {
                                (hi - lo).unsigned_abs() + 1
                            };
                            return Value::from_u64(n * ew as u64, 32);
                        }
                        // A multi-dimensional member (`logic [7:0] md[3][4]`)
                        // counts every element of every dimension.
                        if let Some(bits) = self.unpacked_nd_bits(&nm) {
                            return Value::from_u64(bits, 32);
                        }
                    }
                    // §20.6.2: `$bits(o.md)` / `$bits(this.md)` on a fixed
                    // multi-dimensional array member counts every element.
                    if matches!(arg.kind, ExprKind::MemberAccess { .. })
                        || matches!(&arg.kind, ExprKind::Ident(h) if h.path.len() == 2)
                    {
                        if let Some((h, prop)) = self.class_prop_receiver(arg)
                            && let Some(bits) =
                                self.unpacked_nd_bits(&Self::handle_member_key(h, &prop))
                        {
                            return Value::from_u64(bits, 32);
                        }
                    }
                    Value::from_u64(self.eval_expr(arg).width as u64, 32)
                } else {
                    Value::zero(32)
                }
            }
            // §28.8 internal helpers emitted by `resolve_bidirectional_switches`.
            //
            // `$__wres(a, b)` — wired-net resolution, bit by bit: a `z`
            // yields to a driven value, equal values pass through, and
            // anything else (a 0/1 conflict, or an `x`) gives `x`.
            // §6.6.2 wired-AND / wired-OR net resolution, bit by bit. A
            // `z` driver yields to a driven value (as with a plain wire);
            // otherwise the two driven bits are ANDed / ORed. `x` on
            // either side gives `x` unless the other side is the
            // controlling value (0 for AND, 1 for OR), which dominates.
            "$__wand" | "$__wor" => {
                if args.len() < 2 {
                    return Value::new(1);
                }
                let is_and = name == "$__wand";
                let a = self.eval_expr(&args[0]);
                let b = self.eval_expr(&args[1]);
                let w = a.width.max(b.width).max(1);
                let mut out = Value::zero(w);
                for i in 0..w as usize {
                    out.set_bit(i, Self::wired_logic_bit(a.get_bit(i), b.get_bit(i), is_and));
                }
                out
            }
            "$__wres" => {
                if args.len() < 2 {
                    return Value::new(1);
                }
                let a = self.eval_expr(&args[0]);
                let b = self.eval_expr(&args[1]);
                let w = a.width.max(b.width).max(1);
                let mut out = Value::zero(w);
                for i in 0..w as usize {
                    out.set_bit(i, Self::wire_resolve_bit(a.get_bit(i), b.get_bit(i)));
                }
                out
            }
            // `$__wres_tri(acc, ctl1, data1, ctl2, data2, ...)` — resolve the
            // tristate primitives driving a net (`bufif*`/`notif*`/MOS, each
            // lowered to `ctl ? data : z`) together with its other drivers
            // `acc`, bit by bit. A primitive whose control is 1 drives its
            // data, 0 drives nothing. One whose control is unknown drives
            // `L`/`H` (§28.4 Table 28-5, §28.7): its value OR z. Those are
            // applied after every known driver: the bit keeps the value both
            // possibilities agree on, else it is `x` — `H` beside a strong 1
            // is 1, beside a 0 or with nothing else it is `x`. The plain `?:`
            // merge gave `x` even where another driver settles the bit.
            "$__wres_tri" => {
                if args.len() < 3 {
                    return Value::new(1);
                }
                let acc = self.eval_expr(&args[0]);
                let mut drivers: Vec<(LogicBit, Value)> = Vec::with_capacity(args.len() / 2);
                for pair in args[1..].chunks(2) {
                    let [c, d] = pair else { break };
                    let ctl = self.eval_expr(c);
                    // The control's truth value, as the `?:` it came from.
                    let bits = (0..ctl.width.max(1) as usize).map(|i| ctl.get_bit(i));
                    let cb = if bits.clone().any(|b| b == LogicBit::One) {
                        LogicBit::One
                    } else if bits.into_iter().all(|b| b == LogicBit::Zero) {
                        LogicBit::Zero
                    } else {
                        LogicBit::X
                    };
                    if cb != LogicBit::Zero {
                        drivers.push((cb, self.eval_expr(d)));
                    }
                }
                let w = drivers
                    .iter()
                    .map(|(_, d)| d.width)
                    .fold(acc.width, u32::max)
                    .max(1);
                let bit = |v: &Value, i: usize, pad: LogicBit| {
                    if i < v.width as usize {
                        v.get_bit(i)
                    } else {
                        pad
                    }
                };
                let mut out = Value::zero(w);
                for i in 0..w as usize {
                    let mut b = bit(&acc, i, LogicBit::Z);
                    for (_, d) in drivers.iter().filter(|(c, _)| *c == LogicBit::One) {
                        b = Self::wire_resolve_bit(b, bit(d, i, LogicBit::Zero));
                    }
                    for (_, d) in drivers.iter().filter(|(c, _)| *c != LogicBit::One) {
                        if Self::wire_resolve_bit(b, bit(d, i, LogicBit::Zero)) != b {
                            b = LogicBit::X;
                        }
                    }
                    out.set_bit(i, b);
                }
                out
            }
            // §28.4 pull-strength marker. A lone pull driver just carries
            // its value; multi-driver resolution unwraps it (see
            // `resolve_multi_driver_nets`), so this identity path only runs
            // when a pullup/pulldown is the net's sole driver.
            // `$__supply`/`$__weak` mark the other §28.12 levels the same way.
            // §28.12 `$__sres(width, pair, ctl, value, ...)`: a net whose
            // drivers carry strengths, resolved by strength (see
            // `strength_bit`).
            "$__sres" => {
                let w = args
                    .first()
                    .and_then(|a| self.eval_expr(a).to_u64())
                    .unwrap_or(1)
                    .max(1) as u32;
                let mut drv: Vec<(u8, &Expression, &Expression)> = Vec::new();
                for t in args[1..].chunks(3) {
                    let [c, k, v] = t else { break };
                    let code = self.eval_expr(c).to_u64().unwrap_or(54) as u8;
                    drv.push((code, k, v));
                }
                let ds =
                    self.eval_strength_drivers(w, drv.into_iter().map(|(c, k, v)| (c, Some(k), v)));
                let mut out = Value::zero(w);
                for i in 0..w as usize {
                    out.set_bit(i, Self::strength_value_bit(Self::strength_bit(&ds, i)));
                }
                out
            }
            "$__pull" | "$__supply" | "$__weak" | "$__drive" => args
                .first()
                .map(|a| self.eval_expr(a))
                .unwrap_or_else(|| Value::new(1)),
            // Strong-over-weak resolution: `$__wres_pull(strong, weak)`.
            // The STRONG chain (all non-pull drivers, already $__wres-folded)
            // wins on every bit it drives; the WEAK pull value fills only the
            // bits where strong is z. This is the pull-up/pull-down behavior
            // — a strong driver overrides the resistor, which otherwise holds
            // the net at its pull value.
            "$__wres_pull" => {
                if args.len() < 2 {
                    return Value::new(1);
                }
                let st = self.eval_expr(&args[0]);
                let wk = self.eval_expr(&args[1]);
                let w = st.width.max(wk.width).max(1);
                let mut out = Value::zero(w);
                for i in 0..w as usize {
                    let sb = st.get_bit(i);
                    out.set_bit(i, if sb == LogicBit::Z { wk.get_bit(i) } else { sb });
                }
                out
            }
            // `$__tranif(own_self, own_other, ctl, active)` — the value of
            // THIS terminal of a bidirectional switch.
            "$__tranif" => {
                if args.len() < 4 {
                    return Value::new(1);
                }
                let me = self.eval_expr(&args[0]);
                let other = self.eval_expr(&args[1]);
                let ctl = self.eval_expr(&args[2]);
                let active_high = self.eval_expr(&args[3]).get_bit(0) == LogicBit::One;
                let w = me.width.max(other.width).max(1);
                // The switch conducts when the control matches its polarity.
                let cb = ctl.get_bit(0);
                let conducting = match (cb, active_high) {
                    (LogicBit::One, true) | (LogicBit::Zero, false) => Some(true),
                    (LogicBit::Zero, true) | (LogicBit::One, false) => Some(false),
                    _ => None, // x/z control
                };
                let mut out = Value::zero(w);
                for i in 0..w as usize {
                    let (m, o) = (me.get_bit(i), other.get_bit(i));
                    let bit = match conducting {
                        Some(true) => Self::wire_resolve_bit(m, o),
                        Some(false) => m,
                        // An unknown control isolates or connects: a bit
                        // that agrees either way passes, one that differs
                        // is unknown.
                        None => {
                            if m == o {
                                m
                            } else if m == LogicBit::Z {
                                // z here means "whatever the other side is,
                                // if connected" -> unknown.
                                LogicBit::X
                            } else {
                                LogicBit::X
                            }
                        }
                    };
                    out.set_bit(i, bit);
                }
                out
            }
            "$signed" => {
                let mut v = args
                    .first()
                    .map(|a| self.eval_expr(a))
                    .unwrap_or(Value::zero(32));
                v.is_signed = true;
                v
            }
            "$unsigned" => {
                let mut v = args
                    .first()
                    .map(|a| self.eval_expr(a))
                    .unwrap_or(Value::zero(32));
                v.is_signed = false;
                v
            }
            "$time" => Value::from_u64(self.time_in_current_unit().round() as u64, 64),
            // §3.14.2.3: $time/$realtime from a body whose DECLARING
            // scope has its own timescale (CU/class/package subroutine
            // bodies are rewritten to this at elaboration; the literal
            // argument is the scope's unit exponent).
            "$__xz_time_at" | "$__xz_realtime_at" => {
                let ue = args
                    .first()
                    .map(|a| self.eval_expr(a).to_f64() as i32)
                    .unwrap_or(self.module.timeunit_exp);
                let diff = ue - Self::secs_to_exp(self.tick_s);
                let t = if diff >= 0 {
                    self.time as f64 / 10f64.powi(diff)
                } else {
                    self.time as f64 * 10f64.powi(-diff)
                };
                if name == "$__xz_realtime_at" {
                    Value::from_f64(t)
                } else {
                    Value::from_u64(t.round() as u64, 64)
                }
            }
            "$realtime" => Value::from_f64(self.time_in_current_unit()),
            "$stime" => Value::from_u64(
                (self.time_in_current_unit().round() as u64) & 0xFFFF_FFFF,
                32,
            ),
            "$inferred_clock" | "$inferred_disable" if sv_parser::is_sv2023() => {
                Value::from_u64(0, 1)
            }
            "$global_clock" if sv_parser::is_sv2023() => Value::from_u64(0, 1),
            "$timeunit" if sv_parser::is_sv2023() => {
                let mut v = Value::from_u64(self.module.timeunit_exp as i64 as u64, 32);
                v.is_signed = true;
                v
            }
            "$timeprecision" if sv_parser::is_sv2023() => {
                let mut v = Value::from_u64(self.module.timeprecision_exp as i64 as u64, 32);
                v.is_signed = true;
                v
            }
            "$rose_gclk" | "$fell_gclk" | "$steady_gclk" | "$changing_gclk" | "$past_gclk"
            | "$future_gclk"
                if sv_parser::is_sv2023() =>
            {
                Value::from_u64(0, 1)
            }
            "$test$plusargs" => {
                sim_dbg_eprintln!("[DEBUG] eval $test$plusargs with {} args", args.len());
                let pat = match args.first().map(|a| &a.kind) {
                    Some(ExprKind::StringLiteral(s)) => s.clone(),
                    Some(_) => self.eval_expr(&args[0]).to_sv_string(),
                    None => String::new(),
                };
                Value::from_u64(self.test_plusarg(&pat) as u64, 32)
            }
            "$value$plusargs" => self.eval_value_plusargs(args),
            "$fopen" => self.open_file_handle(args),
            "$fclose" => self.close_file_handle(args),
            "$fwrite" => self.write_file_handle_named(args, false, "$write"),
            "$fwriteb" => self.write_file_handle_named(args, false, "$writeb"),
            "$fwriteh" | "$fwritex" => self.write_file_handle_named(args, false, "$writeh"),
            "$fwriteo" => self.write_file_handle_named(args, false, "$writeo"),
            "$fdisplay" => self.write_file_handle_named(args, true, "$display"),
            "$fdisplayb" => self.write_file_handle_named(args, true, "$displayb"),
            "$fdisplayh" | "$fdisplayx" => self.write_file_handle_named(args, true, "$displayh"),
            "$fdisplayo" => self.write_file_handle_named(args, true, "$displayo"),
            "$ftell" => {
                use std::io::Seek;
                let fd = args
                    .first()
                    .map(|a| self.eval_file_handle_arg(a))
                    .unwrap_or(0);
                // The LOGICAL position: bytes sitting in the pushback buffer
                // (the unread tail of a partially matched `$fscanf` line, or
                // `$ungetc`) are still unread, so they lie before the
                // position. Reporting the raw OS position made a
                // `$ftell`/`$fseek` round-trip land PAST them.
                let pushback = self.ungetc_buf.get(&fd).map_or(0, Vec::len) as u64;
                let pos = self
                    .file_handles
                    .get_mut(&fd)
                    .and_then(|f| f.stream_position().ok())
                    .unwrap_or(0);
                Value::from_u64(pos.saturating_sub(pushback), 32)
            }
            "$fseek" => {
                use std::io::{Seek, SeekFrom};
                let fd = args
                    .first()
                    .map(|a| self.eval_file_handle_arg(a))
                    .unwrap_or(0);
                let off = args
                    .get(1)
                    .map(|a| self.eval_expr(a).to_u64().unwrap_or(0) as i64)
                    .unwrap_or(0);
                let whence = args
                    .get(2)
                    .map(|a| self.eval_expr(a).to_u64().unwrap_or(0))
                    .unwrap_or(0);
                let from = match whence {
                    1 => SeekFrom::Current(off),
                    2 => SeekFrom::End(off),
                    _ => SeekFrom::Start(off as u64),
                };
                if let Some(f) = self.file_handles.get_mut(&fd) {
                    let _ = f.seek(from);
                    self.file_eof.remove(&fd);
                    // Seeking discards pushed-back characters (§21.3.4.1
                    // NOTE: "Operations like $fseek might erase any pushed
                    // back characters"). A stale pushback made a post-
                    // `$fseek` read return the pre-seek tail instead of the
                    // sought-to data.
                    self.ungetc_buf.remove(&fd);
                }
                Value::zero(32)
            }
            "$rewind" => {
                use std::io::{Seek, SeekFrom};
                let fd = args
                    .first()
                    .map(|a| self.eval_file_handle_arg(a))
                    .unwrap_or(0);
                if let Some(f) = self.file_handles.get_mut(&fd) {
                    let _ = f.seek(SeekFrom::Start(0));
                    self.file_eof.remove(&fd);
                    self.ungetc_buf.remove(&fd);
                }
                Value::zero(32)
            }
            "$ungetc" => {
                let ch = args
                    .first()
                    .map(|a| self.eval_expr(a).to_u64().unwrap_or(0) as u8)
                    .unwrap_or(0);
                let fd = args
                    .get(1)
                    .map(|a| self.eval_file_handle_arg(a))
                    .unwrap_or(0);
                self.ungetc_buf.entry(fd).or_default().push(ch);
                self.file_eof.remove(&fd);
                Value::from_u64(ch as u64, 32)
            }
            "$fgets" => {
                let fd = args
                    .get(1)
                    .map(|a| self.eval_file_handle_arg(a))
                    .unwrap_or(0);
                match self.sv_read_line(fd) {
                    Some(line) => {
                        // One char per byte (the text form of a string), so
                        // the CHAR count is the byte count (§21.3.4.2).
                        let n = line.chars().count() as u64;
                        if let Some(dst) = args.first() {
                            self.assign_value(dst, &Value::from_string(&line));
                        }
                        Value::from_u64(n, 32)
                    }
                    None => Value::zero(32),
                }
            }
            // §21.3.4.4 binary read; returns bytes read (0 on EOF).
            "$fread" => self.fread_impl(args),
            "$fscanf" => {
                let n = self.fscanf_impl(args);
                // EOF reports as -1 (§21.3.4.3); the match count otherwise.
                Value::from_u64(n as u32 as u64, 32)
            }
            "$feof" => {
                let fd = args
                    .first()
                    .map(|a| self.eval_file_handle_arg(a))
                    .unwrap_or(0);
                let flag = self.file_eof.contains(&fd) || !self.file_handles.contains_key(&fd);
                Value::from_u64(flag as u64, 32)
            }
            "$fgetc" => {
                use std::io::Read;
                let fd = args
                    .first()
                    .map(|a| self.eval_file_handle_arg(a))
                    .unwrap_or(0);
                if let Some(buf) = self.ungetc_buf.get_mut(&fd) {
                    if let Some(c) = buf.pop() {
                        return Value::from_u64(c as u64, 32);
                    }
                }
                if let Some(f) = self.file_handles.get_mut(&fd) {
                    let mut b = [0u8; 1];
                    if f.read(&mut b).unwrap_or(0) == 1 {
                        return Value::from_u64(b[0] as u64, 32);
                    }
                    self.file_eof.insert(fd);
                }
                Value::from_u64(u32::MAX as u64, 32)
            }
            // §21.3.7 `$ferror(fd, str)`: returns 0 if no error, writes a
            // description string to the 2nd argument.
            "$ferror" => {
                if let Some(out) = args.get(1) {
                    self.assign_value(out, &Value::from_string("No error"));
                }
                Value::zero(32)
            }
            "$readmemh" => self.read_memory_file(args, 16),
            "$readmemb" => self.read_memory_file(args, 2),
            "$readmemd" => self.read_memory_file(args, 10),
            // PRNG system functions.
            //  - no-seed `$urandom`/`$urandom_range` draw from the OS-entropy
            //    RNG, so each run differs (riscv-dv's source of variation).
            //  - a `seed` argument selects a DETERMINISTIC per-seed stream and
            //    advances the seed in place (IEEE 1800 §20.15.2) — same seed,
            //    same sequence.
            // §19.8: overall coverage — the mean of every covergroup
            // TYPE's coverage.
            "$get_coverage" => {
                // §19.11: the mean of the covergroup types' coverages, each
                // weighted by its `type_option.weight` (default 1).
                let names: std::collections::BTreeSet<String> = self
                    .cg_heap
                    .iter()
                    .flatten()
                    .map(|i| i.cg_name.clone())
                    .collect();
                let mut sum = 0.0f64;
                let mut total = 0.0f64;
                for n in &names {
                    let w = self
                        .cg_type_option_get(n, "weight")
                        .and_then(|v| v.to_u64())
                        .unwrap_or(1) as f64;
                    sum += w * self.calculate_type_coverage(n);
                    total += w;
                }
                Value::from_f64(if total > 0.0 { sum / total } else { 0.0 })
            }
            "$urandom" => {
                if let Some(seed_arg) = args.first() {
                    let s = self.eval_expr(seed_arg).to_u64().unwrap_or(0) as u32;
                    let next = Self::prng_next(s);
                    self.assign_value(seed_arg, &Value::from_u64(next as u64, 32));
                    Value::from_u64(next as u64, 32)
                } else {
                    Value::from_u64(self.rng_u32() as u64, 32)
                }
            }
            "$urandom_range" => {
                let a = args
                    .first()
                    .map(|e| self.eval_expr(e).to_u64().unwrap_or(0))
                    .unwrap_or(0);
                let b = args
                    .get(1)
                    .map(|e| self.eval_expr(e).to_u64().unwrap_or(0))
                    .unwrap_or(0);
                Value::from_u64(self.rng_range_u64(a.min(b), a.max(b)), 32)
            }
            // `$random` (no seed) keeps its legacy deterministic value; with a
            // seed it produces a reproducible stream (updating the seed).
            // §20.15.2 probabilistic distribution functions. Each takes an
            // inout `seed` it advances in place, so the same seed replays
            // the same stream.
            "$dist_uniform" | "$dist_normal" | "$dist_exponential" | "$dist_poisson"
            | "$dist_chi_square" | "$dist_t" | "$dist_erlang" => {
                let Some(seed_arg) = args.first() else {
                    return Value::zero(32);
                };
                let mut seed = self.eval_expr(seed_arg).to_u64().unwrap_or(0) as u32;
                // Advance the caller's seed exactly once per call.
                let next_u32 = |sd: &mut u32| -> u32 {
                    *sd = Self::prng_next(*sd);
                    *sd
                };
                let unit = |sd: &mut u32| -> f64 {
                    // (0,1) — never exactly 0, so ln() stays finite.
                    (next_u32(sd) as f64 + 1.0) / (u32::MAX as f64 + 2.0)
                };
                let arg_i = |sim: &mut Self, i: usize| -> i64 {
                    args.get(i)
                        .map(|a| sim.eval_expr(a).to_i64().unwrap_or(0))
                        .unwrap_or(0)
                };
                let out: i64 = match name.as_str() {
                    "$dist_uniform" => {
                        let lo = arg_i(self, 1);
                        let hi = arg_i(self, 2);
                        if hi <= lo {
                            lo
                        } else {
                            let span = (hi - lo + 1) as u64;
                            lo + (next_u32(&mut seed) as u64 % span) as i64
                        }
                    }
                    "$dist_normal" => {
                        let mean = arg_i(self, 1) as f64;
                        let sd = arg_i(self, 2) as f64;
                        // Box-Muller.
                        let u1 = unit(&mut seed);
                        let u2 = unit(&mut seed);
                        let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
                        (mean + sd * z).round() as i64
                    }
                    "$dist_exponential" => {
                        let mean = arg_i(self, 1).max(1) as f64;
                        (-mean * unit(&mut seed).ln()).round() as i64
                    }
                    "$dist_poisson" => {
                        let mean = arg_i(self, 1).max(0) as f64;
                        // Knuth.
                        let l = (-mean).exp();
                        let mut k = 0i64;
                        let mut p = 1.0f64;
                        loop {
                            p *= unit(&mut seed);
                            if p <= l || k > 10_000 {
                                break;
                            }
                            k += 1;
                        }
                        k
                    }
                    "$dist_chi_square" => {
                        // Sum of `df` squared standard normals.
                        let df = arg_i(self, 1).max(1);
                        let mut acc = 0.0f64;
                        for _ in 0..df.min(1000) {
                            let u1 = unit(&mut seed);
                            let u2 = unit(&mut seed);
                            let z =
                                (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
                            acc += z * z;
                        }
                        acc.round() as i64
                    }
                    "$dist_t" => {
                        let df = arg_i(self, 1).max(1);
                        let u1 = unit(&mut seed);
                        let u2 = unit(&mut seed);
                        let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
                        let mut chi = 0.0f64;
                        for _ in 0..df.min(1000) {
                            let a = unit(&mut seed);
                            let b = unit(&mut seed);
                            let zz =
                                (-2.0 * a.ln()).sqrt() * (2.0 * std::f64::consts::PI * b).cos();
                            chi += zz * zz;
                        }
                        (z / (chi / df as f64).sqrt()).round() as i64
                    }
                    _ => {
                        // $dist_erlang(seed, k, mean)
                        let k = arg_i(self, 1).max(1);
                        let mean = arg_i(self, 2).max(1) as f64;
                        let mut acc = 0.0f64;
                        for _ in 0..k.min(1000) {
                            acc += -unit(&mut seed).ln();
                        }
                        (acc * mean / k as f64).round() as i64
                    }
                };
                self.assign_value(seed_arg, &Value::from_u64(seed as u64, 32));
                let mut v = Value::from_u64(out as u64, 32);
                v.is_signed = true;
                v
            }
            "$random" => {
                if let Some(seed_arg) = args.first() {
                    let s = self.eval_expr(seed_arg).to_u64().unwrap_or(0) as u32;
                    let next = Self::prng_next(s);
                    self.assign_value(seed_arg, &Value::from_u64(next as u64, 32));
                    let mut v = Value::from_u64(next as u64, 32);
                    v.is_signed = true;
                    v
                } else {
                    // §20.15.1: no-seed `$random` draws from the global RNG
                    // (same stream as no-arg `$urandom`) and returns a SIGNED
                    // 32-bit value. It used to return a constant 0, so every
                    // `$random` stimulus was stuck at 0.
                    let mut v = Value::from_u64(self.rng_u32() as u64, 32);
                    v.is_signed = true;
                    v
                }
            }
            "$isunknown" => {
                let v = args
                    .first()
                    .map(|a| self.eval_expr(a))
                    .unwrap_or(Value::zero(1));
                Value::from_u64(v.has_xz() as u64, 1)
            }
            // §20.5: 1 when the operand is SIGNED. Was not implemented at
            // all, so it silently answered 0 for everything — which made
            // `!$is_signed(x)` pass by accident and `$is_signed(x)` fail
            // for genuinely signed operands (ivtest struct_signed,
            // struct_member_signed). Reads already stamp the declared
            // signedness onto the value (§7.2.1 covers a `struct packed
            // signed` and a `logic signed` member), so the evaluated
            // operand is the authority.
            "$is_signed" => {
                let signed = args
                    .first()
                    .map(|a| self.eval_expr(a).is_signed)
                    .unwrap_or(false);
                Value::from_u64(signed as u64, 1)
            }
            "$realtobits" => {
                // §20.5: return the 64-bit IEEE-754 pattern of the real as an
                // INTEGER bit-vector (not a real), so it can be indexed/stored
                // as bits. `v` is a real Value whose bits already are the
                // pattern; re-emit them as a plain 64-bit integer.
                let v = args
                    .first()
                    .map(|a| self.eval_expr(a))
                    .unwrap_or(Value::zero(64));
                Value::from_u64(v.to_f64().to_bits(), 64)
            }
            "$bitstoreal" => {
                // §20.5: reinterpret the 64-bit pattern as an IEEE-754 double.
                // The old pass-through left is_real=false, so the bits were
                // read as a huge integer (0x3FF0000000000000 -> 4.6e18 instead
                // of 1.0). Rebuild the real from the raw bits.
                let v = args
                    .first()
                    .map(|a| self.eval_expr(a))
                    .unwrap_or(Value::zero(64));
                Value::from_f64(f64::from_bits(v.to_u64().unwrap_or(0)))
            }
            "$itor" => {
                // §20.4: convert a (signed) integer to real. `to_u64` read a
                // negative arg as a huge unsigned value ($itor(-5) -> 4.29e9);
                // `to_i64` honors the operand's signedness so -5 -> -5.0.
                let v = args
                    .first()
                    .map(|a| self.eval_expr(a))
                    .unwrap_or(Value::zero(32));
                Value::from_f64(v.to_i64().unwrap_or(0) as f64)
            }
            "$rtoi" => {
                // §20.5: returns `integer` (a SIGNED 32-bit type), so %d
                // prints e.g. -3, not 4294967293; out of the 64-bit range it
                // is 0 (see `Value::rtoi`).
                let v = args
                    .first()
                    .map(|a| self.eval_expr(a))
                    .unwrap_or(Value::zero(64));
                Value::rtoi(v.to_f64())
            }
            "$ceil" => {
                let v = args
                    .first()
                    .map(|a| self.eval_expr(a))
                    .unwrap_or(Value::zero(64));
                Value::from_f64(v.to_f64().ceil())
            }
            "$floor" => {
                let v = args
                    .first()
                    .map(|a| self.eval_expr(a))
                    .unwrap_or(Value::zero(64));
                Value::from_f64(v.to_f64().floor())
            }
            "$sqrt" => {
                // §20.8.2: real result, matches C sqrt().
                let v = args
                    .first()
                    .map(|a| self.eval_expr(a))
                    .unwrap_or(Value::zero(64));
                Value::from_f64(v.to_f64().sqrt())
            }
            "$pow" => {
                let a = args
                    .first()
                    .map(|a| self.eval_expr(a))
                    .unwrap_or(Value::zero(64));
                let b = args
                    .get(1)
                    .map(|a| self.eval_expr(a))
                    .unwrap_or(Value::zero(64));
                Value::from_f64(a.to_f64().powf(b.to_f64()))
            }
            "$log10" => {
                // §20.8.2: real result, matches C log10().
                let v = args
                    .first()
                    .map(|a| self.eval_expr(a))
                    .unwrap_or(Value::zero(64));
                Value::from_f64(v.to_f64().log10())
            }
            "$exp" => {
                let v = args
                    .first()
                    .map(|a| self.eval_expr(a))
                    .unwrap_or(Value::zero(64));
                Value::from_f64(v.to_f64().exp())
            }
            "$ln" => {
                let v = args
                    .first()
                    .map(|a| self.eval_expr(a))
                    .unwrap_or(Value::zero(64));
                Value::from_f64(v.to_f64().ln())
            }
            "$log2" => {
                let v = args
                    .first()
                    .map(|a| self.eval_expr(a))
                    .unwrap_or(Value::zero(64));
                Value::from_f64(v.to_f64().log2())
            }
            // §20.8.2 real trigonometric and hyperbolic functions. Per
            // Table 20-4 each matches the equivalent C <math.h> function.
            // All take a real argument (two for $atan2/$hypot) and return a
            // real result. f64 methods below are direct C-library mirrors.
            "$sin" => {
                let v = real1(args, &mut |a| self.eval_expr(a));
                Value::from_f64(v.sin())
            }
            "$cos" => {
                let v = real1(args, &mut |a| self.eval_expr(a));
                Value::from_f64(v.cos())
            }
            "$tan" => {
                let v = real1(args, &mut |a| self.eval_expr(a));
                Value::from_f64(v.tan())
            }
            "$asin" => {
                let v = real1(args, &mut |a| self.eval_expr(a));
                Value::from_f64(v.asin())
            }
            "$acos" => {
                let v = real1(args, &mut |a| self.eval_expr(a));
                Value::from_f64(v.acos())
            }
            "$atan" => {
                let v = real1(args, &mut |a| self.eval_expr(a));
                Value::from_f64(v.atan())
            }
            // $atan2(y, x): first arg is y, second is x (Table 20-4).
            "$atan2" => {
                let (a, b) = real2(args, &mut |a| self.eval_expr(a));
                Value::from_f64(a.atan2(b))
            }
            "$hypot" => {
                let (a, b) = real2(args, &mut |a| self.eval_expr(a));
                Value::from_f64(a.hypot(b))
            }
            "$sinh" => {
                let v = real1(args, &mut |a| self.eval_expr(a));
                Value::from_f64(v.sinh())
            }
            "$cosh" => {
                let v = real1(args, &mut |a| self.eval_expr(a));
                Value::from_f64(v.cosh())
            }
            "$tanh" => {
                let v = real1(args, &mut |a| self.eval_expr(a));
                Value::from_f64(v.tanh())
            }
            "$asinh" => {
                let v = real1(args, &mut |a| self.eval_expr(a));
                Value::from_f64(v.asinh())
            }
            "$acosh" => {
                let v = real1(args, &mut |a| self.eval_expr(a));
                Value::from_f64(v.acosh())
            }
            "$atanh" => {
                let v = real1(args, &mut |a| self.eval_expr(a));
                Value::from_f64(v.atanh())
            }
            // §6.24.1 TYPE cast `int'(x)` / `real'(x)` — lowered by the
            // parser. A real → integral cast ROUNDS (§6.12.2); an integral
            // → real cast widens; otherwise resize to the type's width and
            // stamp its signedness.
            "$__xz_type_cast" => {
                // §6.24.3: an unpacked operand packs its elements.
                if let Some(v) = self.cast_of_unpacked_operand(expr, ctx_width, name, args) {
                    return v;
                }
                let Some(ExprKind::TypeLiteral(dt)) = args.first().map(|a| &a.kind) else {
                    return args
                        .get(1)
                        .map(|a| self.eval_expr(a))
                        .unwrap_or_else(|| Value::zero(1));
                };
                let dt = dt.clone();
                let inner_is_call =
                    matches!(args.get(1).map(|a| &a.kind), Some(ExprKind::Call { .. }));
                // §6.24.1: the result is what a variable of the cast type
                // holds after being ASSIGNED the operand, so an integral
                // target is the operand's context width — `int'(a + b)` over
                // 8-bit operands sums at 32 bits, and a streaming operand
                // left-justifies in it (§11.4.14.2).
                let ctx_w = self.cast_context_width(&dt);
                let mut v = match args.get(1) {
                    Some(a) if ctx_w > 0 && !inner_is_call => self.eval_expr_ctx(a, ctx_w),
                    Some(a) => self.eval_expr(a),
                    None => Value::zero(1),
                };
                // §6.24.1: cast of a CALL returning a dynamic array/queue
                // packs the returned elements, element 0 most significant,
                // each sliced to the target's per-element width (ivtest
                // sv_darray_function `byte_vector'(inc_array(b))`).
                if inner_is_call && self.pending_ret_collection.is_some() {
                    let w = super::elaborate::resolve_type_width(
                        &dt,
                        Some(&self.module.parameters),
                        Some(&self.module.typedefs),
                    );
                    if w > 0 {
                        let src = self.take_pending_ret_collection().unwrap();
                        if let Some(p) = self.pack_ret_collection(&src, w) {
                            v = p;
                        }
                    }
                }
                // §6.16: converting an integral value to `string` removes
                // every "\0" character — not just leading zero bytes.
                // `string'(24'hab0063)` is 2 characters, not 3.
                if matches!(
                    dt.as_ref(),
                    crate::ast::types::DataType::Simple {
                        kind: crate::ast::types::SimpleType::String,
                        ..
                    }
                ) {
                    return Self::sv_string_cast(&v);
                }
                if super::elaborate::is_type_real(&dt) {
                    return Value::from_f64(v.to_f64());
                }
                // §6.24.1: a type cast to an UNPACKED ARRAY / QUEUE target
                // (e.g. `bit_q_t'(stream)`) yields an unpacked collection.
                // The operand's self-determined PACKED representation already
                // holds every element in order; a packed `resolve_type_width`
                // resize here would truncate to the element width and drop all
                // but one element (the uvm_reg_map byte-shift uses precisely
                // this shape: `bit_q_t'({<< {bits}})` then `{<< 8 {...}}`).
                // Return the natural width and let a downstream
                // stream-to-collection assignment distribute the bits aloud.
                if self.type_is_unpacked_collection(&dt) {
                    let mut out = if v.is_real {
                        Self::real_to_int(v.to_f64(), self.cast_context_width(&dt).max(1))
                    } else {
                        v
                    };
                    out.is_signed = super::elaborate::is_type_signed(&dt);
                    return out;
                }
                let w = self.bound_type_param_width(&dt).unwrap_or_else(|| {
                    super::elaborate::resolve_type_width(
                        &dt,
                        Some(&self.module.parameters),
                        Some(&self.module.typedefs),
                    )
                    .max(1)
                });
                let mut out = if v.is_real {
                    Self::real_to_int(v.to_f64(), w)
                } else {
                    v.resize(w)
                };
                // §6.24.1: a 2-state target type takes x/z bits as 0.
                if super::elaborate::is_type_two_state_resolved(&dt, &self.module.typedef_types) {
                    out = out.to_two_state();
                }
                out.is_signed = super::elaborate::is_type_signed(&dt);
                out
            }
            // §6.24.1 `id'(x)` where `id` is a bare identifier: a TYPE cast
            // when `id` names a typedef, otherwise a SIZE cast (the constant
            // `id` gives the target width). Parser-lowered; resolved here
            // where both the typedef and parameter tables are visible.
            "$__xz_named_cast" => {
                // §6.24.3: an unpacked operand packs its elements.
                if let Some(v) = self.cast_of_unpacked_operand(expr, ctx_width, name, args) {
                    return v;
                }
                let inner_is_call =
                    matches!(args.get(1).map(|a| &a.kind), Some(ExprKind::Call { .. }));
                // §6.24.1: an integral target (typedef, enum, or a constant
                // size) is the operand's context width — see the type-cast
                // arm above.
                let target = args.first().and_then(|a| self.named_cast_key(a));
                let string_target = target
                    .as_deref()
                    .is_some_and(|name| self.named_cast_is_string(name));
                let ctx_w = if string_target {
                    0
                } else {
                    match target.as_deref() {
                        Some(nm) => {
                            if let Some(dt) = self.module.typedef_types.get(nm) {
                                self.cast_context_width(dt)
                            } else if let Some(&w) = self.module.typedefs.get(nm) {
                                w.max(1)
                            } else if self.module.parameters.contains_key(nm)
                                || self.get_signal_value_by_name(nm).is_some()
                                || self.class_value_param_in_scope(nm)
                            {
                                self.eval_expr(&args[0]).to_u64().unwrap_or(32).max(1) as u32
                            } else {
                                0
                            }
                        }
                        None => 0,
                    }
                };
                let mut inner_v = match args.get(1) {
                    Some(a) if ctx_w > 0 && !inner_is_call => self.eval_expr_ctx(a, ctx_w),
                    Some(a) => self.eval_expr(a),
                    None => Value::zero(1),
                };
                // §6.24.1: a cast whose operand is a CALL returning a
                // dynamic array/queue packs the returned elements —
                // element 0 most significant, each sliced to the target's
                // per-element width (ivtest sv_darray_function). The
                // collection was recorded at the call's `return`; the
                // target width resolves from the cast's leaf name.
                if inner_is_call && self.pending_ret_collection.is_some() {
                    let leaf_w: Option<u32> = target.as_ref().and_then(|nm| {
                        self.module
                            .typedef_types
                            .get(nm)
                            .map(|t| {
                                super::elaborate::resolve_type_width(
                                    t,
                                    Some(&self.module.parameters),
                                    Some(&self.module.typedefs),
                                )
                            })
                            .or_else(|| self.module.typedefs.get(nm).copied())
                    });
                    if let Some(w) = leaf_w.filter(|&w| w > 0) {
                        let src = self.take_pending_ret_collection().unwrap();
                        if let Some(p) = self.pack_ret_collection(&src, w) {
                            inner_v = p;
                        }
                    }
                }
                if string_target {
                    return Self::sv_string_cast(&inner_v);
                }
                if let Some(nm) = target {
                    // (a) Full typedef (carries signedness / real-ness).
                    if let Some(dt) = self.module.typedef_types.get(&nm).cloned() {
                        if super::elaborate::is_type_real(&dt) {
                            return Value::from_f64(inner_v.to_f64());
                        }
                        // §6.24.1 cast to an UNPACKED ARRAY / QUEUE typedef
                        // (e.g. `bit_q_t'(stream)`, uvm_reg_map's byte-shift).
                        // `typedef_types` resolves the typedef to its base
                        // integral type (the queue's element), so the unpacked
                        // dims are visible only under the typedef NAME. The
                        // result is unpacked: keep the operand's
                        // self-determined PACKED width rather than collapsing
                        // to the element width, or all but one element are lost.
                        if self.type_is_unpacked_collection_name(&nm) {
                            let mut out = if inner_v.is_real {
                                Self::real_to_int(
                                    inner_v.to_f64(),
                                    self.cast_context_width(&dt).max(1),
                                )
                            } else {
                                inner_v
                            };
                            out.is_signed = super::elaborate::is_type_signed_resolved(
                                &dt,
                                &self.module.typedef_types,
                            );
                            return out;
                        }
                        let w = super::elaborate::resolve_type_width(
                            &dt,
                            Some(&self.module.parameters),
                            Some(&self.module.typedefs),
                        )
                        .max(1);
                        let mut out = if inner_v.is_real {
                            Self::real_to_int(inner_v.to_f64(), w)
                        } else {
                            inner_v.resize(w)
                        };
                        // §6.24.1: a 2-state typedef target takes x/z as 0.
                        if super::elaborate::is_type_two_state_resolved(
                            &dt,
                            &self.module.typedef_types,
                        ) {
                            out = out.to_two_state();
                        }
                        out.is_signed = super::elaborate::is_type_signed_resolved(
                            &dt,
                            &self.module.typedef_types,
                        );
                        return out;
                    }
                    // (b) A typedef/enum known only by width (e.g. an enum
                    // typedef `state_e'(2)`) — resize to that width. Without
                    // this the enum name fell to the size-cast branch below
                    // and evaluated to 0.
                    if let Some(&w) = self.module.typedefs.get(&nm) {
                        let mut out = if inner_v.is_real {
                            Self::real_to_int(inner_v.to_f64(), w.max(1))
                        } else {
                            inner_v.resize(w.max(1))
                        };
                        out.is_signed = false;
                        return out;
                    }
                    // (c) SIZE cast: the identifier is a constant giving the
                    // width. Only when it actually names a parameter — an
                    // unknown identifier stays a pass-through (its old
                    // behaviour) rather than resizing to a garbage width.
                    if self.module.parameters.contains_key(&nm)
                        || self.get_signal_value_by_name(&nm).is_some()
                        || self.class_value_param_in_scope(&nm)
                    {
                        let n = self.eval_expr(&args[0]).to_u64().unwrap_or(32) as u32;
                        return inner_v.resize(n.max(1));
                    }
                    // (d) §8.25/§6.24.1: a class TYPE PARAMETER (`T'(v)`)
                    // casts to the type the active object or specialization
                    // binds to it.
                    if let Some(b) = self.resolve_type_param_binding(&nm)
                        && let Some(w) = self.concrete_type_width(b.trim())
                    {
                        let mut out = inner_v.resize(w.max(1));
                        out.is_signed = self.bound_type_is_signed(b.trim());
                        return out;
                    }
                }
                // Unknown cast target: pass the operand through unchanged.
                inner_v
            }
            "$shortrealtobits" => {
                let v = args
                    .first()
                    .map(|a| self.eval_expr(a))
                    .unwrap_or(Value::zero(32));
                let f = v.to_f64() as f32;
                Value::from_u64(f.to_bits() as u64, 32)
            }
            "$bitstoshortreal" => {
                let v = args
                    .first()
                    .map(|a| self.eval_expr(a))
                    .unwrap_or(Value::zero(32));
                let f = f32::from_bits(v.to_u64().unwrap_or(0) as u32);
                Value::from_f64(f as f64)
            }

            "$dimensions" | "$unpacked_dimensions" => {
                if let Some(arg) = args.first() {
                    // §20.7: a TYPE operand — `$dimensions(int)` is 1,
                    // `$dimensions(logic)` 0 (a scalar has no dimension),
                    // `$dimensions(bit [3:0][7:0])` 2.
                    if let Some(q) = self.type_query_operand(arg) {
                        let n = super::elaborate::type_query_bound(name, &q, 1).unwrap_or(0);
                        return Value::from_u64(n as u64, 32);
                    }
                    // A class property or a struct member path (`m`,
                    // `this.m`, `a.w`): its declared type, unpacked and
                    // packed dimensions alike. A multi-dimensional packed
                    // one is stored flattened and counted as one dimension.
                    if let Some(q) = self
                        .query_operand_decl_dims(arg)
                        .filter(|q| !q.dims.is_empty())
                    {
                        let n = super::elaborate::type_query_bound(name, &q, 1).unwrap_or(0);
                        return Value::from_u64(n as u64, 32);
                    }
                    if let ExprKind::Ident(hier) = &arg.kind {
                        let aname = self.resolve_hier_name(hier);
                        // §20.7: EVERY unpacked dimension counts, not just
                        // the first — `int m[2][3]` has two, `int n[2][3][4]`
                        // three. Only the 1-D table was consulted.
                        let unpacked_dim = self.foreach_dims(&aname).map_or(0, |d| d.len()) as u64;
                        if name == "$unpacked_dimensions" {
                            return Value::from_u64(unpacked_dim, 32);
                        }
                        let elem_w = self
                            .module
                            .arrays
                            .get(&*aname)
                            .map(|&(_, _, w)| w)
                            .or_else(|| self.module.arrays_2d.get(&*aname).map(|&(_, _, w)| w))
                            .or_else(|| self.module.arrays_nd.get(&*aname).map(|(_, w)| *w))
                            .or_else(|| {
                                self.signal_name_to_id
                                    .get(aname.as_ref())
                                    .map(|&id| self.signal_widths[id])
                            })
                            .unwrap_or(0);
                        let packed_dim: u64 = self
                            .module
                            .packed_full_dims
                            .get(&*aname)
                            .map(|d| d.len() as u64)
                            .unwrap_or(if elem_w > 1 { 1 } else { 0 });
                        let total = packed_dim + unpacked_dim;
                        return Value::from_u64(total.max(1), 32);
                    }
                }
                Value::from_u64(1, 32)
            }
            sn @ ("$left" | "$high" | "$right" | "$low" | "$size" | "$increment") => {
                let sn = sn.to_string();
                // §20.7: a TYPE operand — the query describes THAT TYPE's
                // dimensions, so it must report the DECLARED bounds of the
                // selected one (`$size(int)` 32, `$left(byte)` 7,
                // `$left(logic [15:4])` 15, `$size(bit [3:0][7:0], 2)` 8).
                // Only the whole type's width was consulted, which is wrong
                // for any non-zero-based or multi-dimensional shape — and a
                // bare type keyword / typedef NAME parses as an Ident (see
                // `parse_call_args`), so it missed this branch entirely and
                // fell through to the signal path, reporting the bounds of
                // a nonexistent signal (-1:0).
                if let Some(q) = args.first().and_then(|a| self.type_query_operand(a)) {
                    let dim = args
                        .get(1)
                        .map(|a| self.eval_expr(a).to_u64().unwrap_or(1))
                        .unwrap_or(1) as usize;
                    // §20.7: an out-of-range dimension has no bounds to
                    // report — the result is x, not 0.
                    return Self::type_query_value(&sn, &q, dim);
                }
                let dim = args
                    .get(1)
                    .map(|a| self.eval_expr(a).to_u64().unwrap_or(1))
                    .unwrap_or(1) as usize;
                // §20.7: a query of a selected sub-array (`m[0]`, `dm[1]`)
                // describes the dimensions left after the selects:
                // dimension d of `m[i]` is dimension d + 1 of `m`.
                if let Some((base, k)) = args.first().and_then(Self::peel_index_selects) {
                    let probe = Expression::new(
                        ExprKind::SystemCall {
                            name: "$unpacked_dimensions".to_string(),
                            args: vec![base.clone()],
                        },
                        base.span,
                    );
                    let nd = self.eval_expr(&probe).to_u64().unwrap_or(0) as usize;
                    if nd > k {
                        let call = Expression::new(
                            ExprKind::SystemCall {
                                name: sn.clone(),
                                args: vec![
                                    base.clone(),
                                    Self::int_lit_expr((dim + k) as i64, base.span),
                                ],
                            },
                            base.span,
                        );
                        return self.eval_expr(&call);
                    }
                }
                // §20.7 / §7.5: on a DYNAMIC array or queue the query
                // functions report the CURRENT element count, not a static
                // packed width — `$size(d)` == `d.size()`. The declared-dims
                // table below only knows fixed arrays, so a dynamic array
                // fell through to the "packed vector" branch and reported
                // its element width (or 0). `array_operand_name` also lifts
                // a class member (`dyn_array` inside a constraint, `o.q`
                // from outside) to its instance-scoped storage, which is
                // what §18.5.12 needs for `array_size == $size(dyn_array)`.
                if dim <= 1 {
                    if let Some(arg) = args.first() {
                        // §20.7: `$size` of an ASSOCIATIVE array is its
                        // number of entries, `num()` — the declared-dims
                        // table has no row for it, so it fell through to the
                        // packed-vector branch and reported the element
                        // width (32 for `int a[int]`).
                        if sn == "$size" {
                            if let Some(an) = self.assoc_operand_name(arg) {
                                return Self::int_count(self.assoc_top_level_keys(&an).len() as u64);
                            }
                        }
                        if let Some(nm) = self.array_operand_name(arg) {
                            if self.module.dynamic_arrays.contains(&nm) {
                                let size = self.get_queue_size(&nm);
                                return Self::dynamic_query_result(&sn, size);
                            }
                        }
                        // §7.11 applies to strings too: a string acts as a
                        // dynamic array of bytes, so the queries track its
                        // CURRENT length ($left 0, $right len-1) — not the
                        // 1024-bit placeholder width it fell through to.
                        // BUT a FIXED-SIZE string-array member (`string[16]`)
                        // must keep reporting its DECLARED element count, not
                        // the current (empty) string value read back from the
                        // array — `$size(a)` == 16, not 0. Resolve any
                        // instance-scoped / module-scope fixed array first
                        // and fall through to the array-dim branch below.
                        if self.expr_is_string_valued(arg) {
                            let mut is_fixed_array = false;
                            if let ExprKind::Ident(h) = &arg.kind {
                                let rname = self.resolve_hier_name(h);
                                let fixed = |m: &str| {
                                    self.module.arrays.contains_key(m)
                                        || self.module.arrays_2d.contains_key(m)
                                        || self.module.arrays_nd.contains_key(m)
                                };
                                is_fixed_array = self
                                    .instance_assoc_member(&rname)
                                    .map_or_else(|| fixed(&rname), |s| fixed(&s));
                            }
                            if !is_fixed_array {
                                let len = self.eval_expr(arg).sv_string_bytes().len() as u64;
                                return Self::dynamic_query_result(&sn, len);
                            }
                        }
                    }
                }
                if let Some(arg) = args.first() {
                    if let Some(rv) = self.member_array_query(&sn, arg, dim) {
                        return rv;
                    }
                    // Resolve the operand to the name its declared bounds
                    // are registered under. A plain / hierarchical Ident
                    // resolves as it always did; a HANDLE-QUALIFIED member
                    // resolves to the instance-scoped store below.
                    let mut aname: Option<std::borrow::Cow<str>> = None;
                    if let ExprKind::Ident(hier) = &arg.kind {
                        let mut n = self.resolve_hier_name(hier);
                        // A fixed-size ARRAY class member lives at
                        // `<handle>#<member>`; the bare name has no dims.
                        if let Some(scoped) = self.instance_assoc_member(&n) {
                            if self.module.arrays.contains_key(&scoped)
                                || self.module.arrays_2d.contains_key(&scoped)
                                || self.module.arrays_nd.contains_key(&scoped)
                            {
                                n = std::borrow::Cow::Owned(scoped);
                            }
                        }
                        // §8.9: `$size(Cls::S)` — the resolver collapses
                        // the 2-segment form to the bare leaf; map from
                        // the original segments to the qualified store.
                        if !self.module.arrays.contains_key(&*n) {
                            if let Some(k) = self.static_fixed_key_from_hier(hier) {
                                n = std::borrow::Cow::Owned(k);
                            }
                        }
                        aname = Some(n);
                    }
                    // §20.7: `$size(a.M)` on a class member array. The
                    // handle-qualified spelling parses either as a
                    // `MemberAccess`, which never entered the Ident arm at
                    // all, or as a two-segment Ident that resolves to a
                    // name carrying no registered shape. Either way no
                    // bounds were found and the query fell through to the
                    // operand's evaluated BIT WIDTH below, so every
                    // `$size(a.M)` reported 32 while the in-method
                    // `$size(M)` on the same member reported 3.
                    // `foreach (a.M[i])` already iterates the member, so
                    // the shape was known all along — only this path
                    // missed it.
                    if aname
                        .as_deref()
                        .is_none_or(|n| self.foreach_dims(n).is_none())
                    {
                        if let Some(scoped) = self.handle_member_array_name(arg) {
                            aname = Some(std::borrow::Cow::Owned(scoped));
                        }
                    }
                    if let Some(aname) = aname {
                        // Every unpacked dimension, outermost first. Only the
                        // 1-D table was consulted, so `$size(m, 2)` — and even
                        // `$size(m)` on a 2-D/N-D array — returned 0.
                        let dims = self.foreach_dims(&aname);
                        let elem_w = self
                            .module
                            .arrays
                            .get(&*aname)
                            .map(|&(_, _, w)| w)
                            .or_else(|| self.module.arrays_2d.get(&*aname).map(|&(_, _, w)| w))
                            .or_else(|| self.module.arrays_nd.get(&*aname).map(|(_, w)| *w));
                        let packed_w = if let Some(w) = elem_w {
                            w
                        } else if let Some(&id) = self.signal_name_to_id.get(aname.as_ref()) {
                            self.signal_widths[id]
                        } else {
                            // Block-local vector: width lives in the
                            // runtime maps, not the compact table.
                            self.lookup_signal_width(&aname).unwrap_or_else(|| {
                                // A class-member packed vector (`$size(o.big)`
                                // on a `bit[88:0]` field) is not in the
                                // compact tables, so the lookup returns None
                                // and the query collapsed to 0 while `$bits`
                                // reported 89. Fall back to the evaluated
                                // value's width, which resolves class
                                // members to their full bit width.
                                args.first()
                                    .map(|a| self.eval_expr(a).width as u32)
                                    .unwrap_or(0)
                            })
                        };
                        let sel = dims.as_ref().and_then(|d| d.get(dim.saturating_sub(1)));
                        let n_unpacked = dims.as_ref().map(|d| d.len()).unwrap_or(0);
                        // Declared per-dimension order (`[3:1]` -> left=3,
                        // right=1) — the arrays* tables normalize to
                        // (lo, hi) and would report the bounds flipped.
                        let decl_sel = self
                            .module
                            .unpacked_decl_dims
                            .get(&*aname)
                            .and_then(|d| d.get(dim.saturating_sub(1)))
                            .copied();
                        let (lo, hi, descending) = if let Some((l, r)) = decl_sel {
                            (l.min(r), l.max(r), l > r)
                        } else if let Some(&(l, h)) = sel {
                            let desc = dim == 1 && self.module.descending_arrays.contains(&*aname);
                            (l, h, desc)
                        } else if let Some(&(l, r)) =
                            self.module.packed_full_dims.get(&*aname).and_then(|pd| {
                                pd.get(dim.saturating_sub(1).saturating_sub(n_unpacked))
                            })
                        {
                            // §7.4.5: dimensions number unpacked-first, then
                            // packed. A multi-D packed vector (`logic
                            // [3:0][7:0]`) has per-dimension bounds in
                            // packed_full_dims (declared left,right) — without
                            // this the query collapsed to the FLATTENED vector,
                            // so `$left([3:0][7:0])` returned 31, not 3.
                            (l.min(r), l.max(r), l > r)
                        } else if let Some(q) = self
                            .query_operand_decl_dims(arg)
                            .filter(|q| q.unpacked >= n_unpacked && !q.dims.is_empty())
                        {
                            return Self::type_query_value(&sn, &q, dim);
                        } else {
                            (0i64, packed_w as i64 - 1, true)
                        };
                        let left = if descending { hi } else { lo };
                        let right = if descending { lo } else { hi };
                        let size = (hi - lo + 1).max(0) as u64;
                        let result = match sn.as_str() {
                            "$left" => left as u64,
                            "$right" => right as u64,
                            "$high" => hi as u64,
                            "$low" => lo as u64,
                            "$size" => size,
                            "$increment" => {
                                if descending {
                                    1
                                } else {
                                    0u64.wrapping_sub(1)
                                }
                            }
                            _ => 0,
                        };
                        // §20.7: array query functions return `int`
                        // (signed) — `$increment` of an ascending range is
                        // -1, and negative declared bounds print negative.
                        let mut rv = Value::from_u64(result & 0xFFFF_FFFF, 32);
                        rv.is_signed = true;
                        return rv;
                    }
                    if let Some(q) = self
                        .query_operand_decl_dims(arg)
                        .filter(|q| !q.dims.is_empty())
                    {
                        return Self::type_query_value(&sn, &q, dim);
                    }
                    let v = self.eval_expr(arg);
                    match sn.as_str() {
                        "$left" | "$high" => return Value::from_u64(v.width as u64 - 1, 32),
                        "$size" => return Value::from_u64(v.width as u64, 32),
                        "$increment" => return Value::from_u64(1, 32),
                        _ => return Value::zero(32),
                    }
                }
                Value::zero(32)
            }
            "$get_randstate" => {
                // §18.13.4: the RNG state as an implementation-defined
                // string (16 hex digits here), round-trippable via
                // $set_randstate.
                Value::from_string(&self.rng.to_state_string())
            }
            "$typename" => {
                if let Some(arg) = args.first() {
                    if let ExprKind::Ident(hier) = &arg.kind {
                        // IEEE 1800-2017 §20.6.1: $typename of a built-in
                        // type returns that type's name — $typename(int)
                        // is "int". A bare type keyword in a call argument
                        // parses as a single-segment Ident (see the
                        // §20.6.2 branch in parse_call_args).
                        if hier.path.len() == 1 {
                            let kw = hier.path[0].name.name.as_str();
                            if matches!(
                                kw,
                                "bit"
                                    | "logic"
                                    | "reg"
                                    | "byte"
                                    | "shortint"
                                    | "int"
                                    | "longint"
                                    | "integer"
                                    | "time"
                                    | "real"
                                    | "shortreal"
                                    | "realtime"
                                    | "string"
                                    | "chandle"
                                    | "event"
                            ) {
                                // §6.12: `realtime` is `real`.
                                let kw = if kw == "realtime" { "real" } else { kw };
                                return Value::from_string(kw);
                            }
                        }
                    }
                    // §20.6.1: an expression of type real — a real or time
                    // literal (§5.8: `1ns` is realtime), real arithmetic, a
                    // real-valued system or user function call.
                    if !matches!(arg.kind, ExprKind::Ident(_))
                        && (self.expr_is_real_static(arg) || self.call_returns_real(arg))
                    {
                        return Value::from_string("real");
                    }
                    // §20.6.1 / §11.6 / §11.8.1: an integral EXPRESSION names
                    // its self-determined type — the operand's own type
                    // when the result keeps its width, signedness and state
                    // (`i + 1` is `int`), else `bit`/`reg [signed][w-1:0]`.
                    // A selected variable, property or member (`a[1]`,
                    // `s.m`, `o.q[0]`) names its declared type.
                    if !matches!(&arg.kind, ExprKind::Ident(h)
                        if h.path.last().is_some_and(|sg| sg.selects.is_empty()))
                    {
                        if matches!(
                            &arg.kind,
                            ExprKind::Ident(_)
                                | ExprKind::MemberAccess { .. }
                                | ExprKind::Index { .. }
                        ) && let Some(t) = self.typename_operand_text(arg)
                        {
                            return Value::from_string(&t);
                        }
                        if let Some(t) = self.typename_integral_expr(arg) {
                            return Value::from_string(&t.render());
                        }
                    }
                    // §20.6.1 / §25.9: a virtual-interface variable or class
                    // property (spelled out or through a typedef) names
                    // its interface type, `virtual bus_if[.mp] [#(...)]`.
                    if let Some(t) = self.vif_operand_typename(arg) {
                        return Value::from_string(&t);
                    }
                    // A Specialization expression given directly, e.g.
                    // $typename(foo#(bar#(xyz),88)) — IEEE 1800-2017 §21.7.
                    if let ExprKind::Specialization {
                        base,
                        type_args_text,
                    } = &arg.kind
                    {
                        let bn = self.expr_type_leaf_name(base);
                        let parts = self.typename_args_text_parts(type_args_text);
                        return Value::from_string(&self.class_typename_with_args(&bn, parts));
                    }
                    // A class-typed variable: "class <name> #(<args>)".
                    if let Some((base, ta)) = self.class_declared_type_of(arg) {
                        return Value::from_string(&self.format_class_typename(&base, &ta));
                    }
                    if let Some(t) = self.class_operand_typename(arg) {
                        return Value::from_string(&t);
                    }
                    // §8.25: a bare TYPE PARAMETER of the active
                    // specialization (`$typename(T)` inside
                    // `ger#(type T)::do_it()`), resolved through the
                    // current spec to its CONCRETE bound type — not the
                    // literal param name. Pre-fix this fell through to the
                    // scalar-signal fallback and reported `logic` for a
                    // class type (`T=uvm_object`), hiding what the body
                    // actually specialized.
                    if let ExprKind::Ident(hier2) = &arg.kind {
                        if hier2.path.len() == 1 {
                            let tnp = &hier2.path[0].name.name;
                            if let Some(concrete) = self.resolve_type_param_binding(tnp) {
                                if concrete != *tnp {
                                    return Value::from_string(
                                        &self.typename_of_concrete(&concrete),
                                    );
                                }
                            }
                        }
                    }
                    if let Some(t) = self.typename_scoped_type(arg) {
                        return Value::from_string(&t);
                    }
                    // A bare `data_type` argument that names a TYPEDEF
                    // (`$typename(my_bits_t)`): per §20.6.1a the typedef
                    // resolves back to its built-in base, so "logic
                    // signed [15:0]" rather than the typedef name or a
                    // bare "logic".
                    if let ExprKind::Ident(hier_td) = &arg.kind {
                        if hier_td.path.len() == 1 && hier_td.path[0].selects.is_empty() {
                            let tdn = &hier_td.path[0].name.name;
                            // A typedef naming a class (or a specialization).
                            if let Some((b, sig)) = self.resolve_typedef_spec(tdn) {
                                return Value::from_string(
                                    &self.class_spec_typename(&b, Some(&sig)),
                                );
                            }
                            if let Some(c) =
                                self.resolve_simple_typedef_class(tdn).filter(|c| c != tdn)
                            {
                                return Value::from_string(&self.class_spec_typename(&c, None));
                            }
                            if self.module.typedef_types.contains_key(tdn)
                                || self.lookup_typedef_target(tdn).is_some()
                            {
                                if let Some(t) = self.typename_of_typedef_name(tdn) {
                                    return Value::from_string(&t);
                                }
                            }
                        }
                    }
                    // A bare class/covergroup type name, or a scalar signal.
                    if let ExprKind::Ident(hier) = &arg.kind {
                        if hier.path.len() == 1 {
                            let n = &hier.path[0].name.name;
                            if self.module.classes.contains_key(n)
                                || self.module.covergroups.contains_key(n.as_str())
                            {
                                return Value::from_string(&format!("class {}", n));
                            }
                        }
                        let leaf = hier.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                        // A signal or block-local var with a declared
                        // type: render it, so `logic [15:0] v` reports
                        // "logic [15:0]" (§20.6.1g preserves the range)
                        // instead of a bare "logic".
                        if let Some(t) = self.typename_operand_text(arg) {
                            return Value::from_string(&t);
                        }
                        // A var declared with a TYPEDEF'd type
                        // (`my_bits_t v1;`): render the typedef's base.
                        if let Some(tn) = self.var_typedef_types.get(leaf).cloned() {
                            if let Some(t) = self.typename_of_typedef_name(&tn) {
                                return Value::from_string(&t);
                            }
                        }
                        let name = self.resolve_hier_name(hier);
                        if self.signal_name_to_id.contains_key(name.as_ref()) {
                            return Value::from_string("logic");
                        }
                    }
                }
                Value::from_string("logic")
            }
            "$isunbounded" => {
                if let Some(arg) = args.first() {
                    if let ExprKind::Dollar = &arg.kind {
                        return Value::from_u64(1, 1);
                    }
                    let v = self.eval_expr(arg);
                    // $ is stored as all-ones in the parameter's width
                    let all_ones = if v.width >= 64 {
                        u64::MAX
                    } else {
                        (1u64 << v.width) - 1
                    };
                    Value::from_u64(if v.to_u64() == Some(all_ones) { 1 } else { 0 }, 1)
                } else {
                    Value::zero(1)
                }
            }
            "$countbits" => {
                if args.len() >= 2 {
                    let v = self.eval_expr(&args[0]);
                    // Collect target bit values from remaining args
                    let mut targets = Vec::new();
                    for arg in &args[1..] {
                        if let ExprKind::Number(NumberLiteral::UnbasedUnsized(c)) = &arg.kind {
                            targets.push(match c {
                                '0' => super::value::LogicBit::Zero,
                                '1' => super::value::LogicBit::One,
                                'x' | 'X' => super::value::LogicBit::X,
                                'z' | 'Z' => super::value::LogicBit::Z,
                                _ => super::value::LogicBit::One,
                            });
                        } else {
                            // §20.9: a control value like `1'bx`/`1'bz`/`1'b0`
                            // parses as a sized based literal (not
                            // UnbasedUnsized), so read the actual LSB LogicBit
                            // to preserve X/Z rather than collapsing via to_u64.
                            let cv = self.eval_expr(arg);
                            targets.push(cv.get_bit(0));
                        }
                    }
                    let mut count = 0u64;
                    for i in 0..v.width as usize {
                        let b = v.get_bit(i);
                        if targets.contains(&b) {
                            count += 1;
                        }
                    }
                    Value::from_u64(count, 32)
                } else {
                    Value::zero(32)
                }
            }
            "$countones" => {
                let v = args
                    .first()
                    .map(|a| self.eval_expr(a))
                    .unwrap_or(Value::zero(32));
                let mut count = 0u64;
                for i in 0..v.width as usize {
                    if v.get_bit(i) == super::value::LogicBit::One {
                        count += 1;
                    }
                }
                Value::from_u64(count, 32)
            }
            "$onehot" => {
                let v = args
                    .first()
                    .map(|a| self.eval_expr(a))
                    .unwrap_or(Value::zero(32));
                let mut count = 0u64;
                for i in 0..v.width as usize {
                    if v.get_bit(i) == super::value::LogicBit::One {
                        count += 1;
                    }
                }
                Value::from_u64(if count == 1 { 1 } else { 0 }, 1)
            }
            "$onehot0" => {
                let v = args
                    .first()
                    .map(|a| self.eval_expr(a))
                    .unwrap_or(Value::zero(32));
                let mut count = 0u64;
                for i in 0..v.width as usize {
                    if v.get_bit(i) == super::value::LogicBit::One {
                        count += 1;
                    }
                }
                Value::from_u64(if count <= 1 { 1 } else { 0 }, 1)
            }
            "$sscanf" => {
                // §21.3.4 — full conversion walk, not just a lone %d.
                if args.len() >= 2 {
                    let src_str = if let ExprKind::StringLiteral(t) = &args[0].kind {
                        t.clone()
                    } else {
                        self.eval_expr(&args[0]).to_sv_string()
                    };
                    let fmt = if let ExprKind::StringLiteral(f) = &args[1].kind {
                        f.clone()
                    } else {
                        self.eval_expr(&args[1]).to_sv_string()
                    };
                    let n = self.sv_sscanf(&src_str, &fmt, &args[2..]);
                    return Value::from_u64(n as u64, 32);
                }
                Value::zero(32)
            }
            // §20.14 `$countdrivers(net, …)` — driver counting is not
            // modeled (xezim resolves multiple drivers at settle time
            // without retaining per-driver identity). Returns 0 ("no
            // contention") and warns once.
            "$countdrivers" => {
                self.warn_system_task_once(
                    "$countdrivers",
                    "Warning: $countdrivers is not modeled — returning 0",
                );
                Value::zero(1)
            }
            // §20.15 `$q_full(q_id, status)`: system FUNCTION returning 0/1
            // and writing the status code to its second argument.
            "$q_full" => {
                let (full, status) = self.q_full_impl(args);
                self.assign_q_status(args, 1, status);
                Value::from_u64(full as u64, 32)
            }
            // §20.17.1 `$system` as a FUNCTION: returns the host shell
            // exit status (int). With no argument calls system(NULL).
            "$system" => Value::from_u64(self.system_impl(args) as u64, 32),
            // §20.17.2 `$stacktrace` as a FUNCTION: returns the call-stack
            // text as a string. Content is implementation-defined (20.17.2).
            "$stacktrace" => Value::from_string(&self.stacktrace_text()),
            // A `$name` a VPI module registered as a system FUNCTION.
            // Checked last so a builtin always wins.
            _ if vpi_systf_is_func(name) => {
                let self_ptr = self as *mut Simulator;
                vpi_call_systf(self_ptr, name, args, expr.span).unwrap_or_else(|| Value::zero(32))
            }
            // §16.9.3 / §16.5.1 `$sampled(e)`: the sampled value of `e` in
            // the current time slot — its value as of the Preponed region,
            // before any active/NBA update of the slot. Where the evaluator
            // runs after a same-slot update (e.g. a concurrent assertion's
            // action block, which runs in the reactive region), the live
            // signal is newer, so swap in the Preponed snapshot of the
            // operand's signals, evaluate against those, then restore the
            // live values. Signals with no snapshot fall back to live.
            "$sampled" => {
                let a = args.first().cloned();
                match a {
                    Some(e) => {
                        let mut ids = Vec::new();
                        self.collect_sva_signal_ids(&e, &mut ids);
                        ids.sort_unstable();
                        ids.dedup();
                        let saved = self.install_preponed(&ids);
                        let v = self.eval_expr(&e);
                        self.restore_preponed(saved);
                        v
                    }
                    None => Value::zero(32),
                }
            }
            // §38.36.3 cbUnresolvedSystf may register the name.
            _ if self.vpi_cb_mask != 0
                && self.vpi_resolve_unknown_systf(name)
                && vpi_systf_is_func(name) =>
            {
                let self_ptr = self as *mut Simulator;
                vpi_call_systf(self_ptr, name, args, expr.span).unwrap_or_else(|| Value::zero(32))
            }
            _ => {
                self.warn_unknown_system_task(name, args);
                Value::zero(32)
            }
        }
    }
}
