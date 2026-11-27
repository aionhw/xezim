//! IEEE 1800-2023 §6.19.5: the enumeration methods `first`, `last`, `next`,
//! `prev`, `num` and `name` take the enumeration type from the receiver's
//! DECLARED type. The receiver can be any expression of that type: a
//! variable declared through a typedef alias or an array typedef, an
//! element of an array, a member of a struct or union at any depth, a
//! property reached through one or more class handles, `this.x`, a function
//! result, a member of a function result, or the result of another
//! enumeration method.
//!
//! This file resolves that declared type and answers the methods from it.
//! The value of the receiver never selects the type: a value that names no
//! member of the receiver's type has no name (§6.19.5.6), whatever other
//! enumerations of the design hold it.

use super::*;
use crate::ast::types::UnpackedDimension as UD;

/// An enumeration type as the methods (and `$cast`) need it.
pub(crate) struct EnumRecvType {
    /// The members in declaration order (`(name, pre-resolved Value)`).
    /// Each `Value` carries the member's 4-state / wide bits, base width,
    /// and base signedness.
    members: Vec<(String, Value)>,
    /// Pre-built string values returned by `.name()`, indexed like `members`.
    name_vals: Vec<Value>,
    /// O(1) lookup of 2-state low-64-bit member values (masked to `width`) -> index.
    val_to_idx: HashMap<u64, usize>,
    /// Indices in `members` whose value carries X or Z bits.
    xz_indices: Vec<usize>,
    /// The width of the base type.
    width: u32,
    /// Whether the base type is signed.
    signed: bool,
    /// Whether the base type is 2-state.
    two_state: bool,
}

impl EnumRecvType {
    fn new(members: Vec<(String, Value)>, width: u32, signed: bool, two_state: bool) -> Self {
        let width = width.max(1);
        let mask = if width >= 64 {
            u64::MAX
        } else {
            (1u64 << width) - 1
        };
        let mut name_vals = Vec::with_capacity(members.len());
        let mut val_to_idx = HashMap::default();
        let mut xz_indices = Vec::new();
        for (i, (nm, mv)) in members.iter().enumerate() {
            name_vals.push(Value::from_string(nm));
            if mv.has_xz() {
                xz_indices.push(i);
            } else if width <= 64
                && let Some(u) = mv.to_u64()
            {
                val_to_idx.entry(u & mask).or_insert(i);
            }
        }
        Self {
            members,
            name_vals,
            val_to_idx,
            xz_indices,
            width,
            signed,
            two_state,
        }
    }

    fn position(&self, v: &Value) -> Option<usize> {
        if v.has_xz() {
            let vw = v.resize_for_assign(self.width);
            return self.xz_indices.iter().copied().find(|&i| {
                self.members[i]
                    .1
                    .case_eq(&vw)
                    .to_u64()
                    == Some(1)
            });
        }
        if self.width <= 64 {
            let mask = if self.width >= 64 {
                u64::MAX
            } else {
                (1u64 << self.width) - 1
            };
            let cur = v.to_u64()? & mask;
            return self.val_to_idx.get(&cur).copied();
        }
        let vw = v.resize_for_assign(self.width);
        self.members
            .iter()
            .position(|(_, mv)| !mv.has_xz() && mv.case_eq(&vw).to_u64() == Some(1))
    }

    /// IEEE 1800-2023 §6.24.2: whether `src` equals a member value of this
    /// enumeration (checking 4-state X/Z bits, signedness, and width).
    pub(crate) fn matches_cast_src(&self, src: &Value) -> bool {
        if src.is_real || self.members.is_empty() {
            return false;
        }
        let w = self.width.max(src.width).max(1);
        let src_w = if src.is_fill || (self.signed && src.is_signed) || src.width >= w {
            src.resize_for_assign(w)
        } else {
            let mut z = src.clone();
            z.is_signed = false;
            z.resize_for_assign(w)
        };
        if src.has_xz() {
            return self.xz_indices.iter().any(|&i| {
                let mv = &self.members[i].1;
                if w <= mv.width {
                    mv.case_eq(&src_w).to_u64() == Some(1)
                } else {
                    let mut z = mv.clone();
                    z.is_signed = false;
                    z.resize(w).case_eq(&src_w).to_u64() == Some(1)
                        || (self.signed
                            && src.is_signed
                            && mv.resize(w).case_eq(&src_w).to_u64() == Some(1))
                }
            });
        }
        if w <= 64 {
            let Some(sv) = src_w.to_u64() else {
                return false;
            };
            let w_mask = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
            let sv = sv & w_mask;
            let e_mask = if self.width >= 64 {
                u64::MAX
            } else {
                (1u64 << self.width) - 1
            };
            // High bits above `self.width` are zero: matches zero-extended member.
            if (sv & !e_mask) == 0 && self.val_to_idx.contains_key(&(sv & e_mask)) {
                return true;
            }
            // High bits match sign extension of a negative signed enum member
            // when the source is also signed.
            if self.signed && src.is_signed && self.width < w {
                let sign_ext = w_mask & !e_mask;
                if (sv & sign_ext) == sign_ext {
                    let candidate = sv & e_mask;
                    let sign_bit = 1u64 << (self.width - 1);
                    if (candidate & sign_bit) != 0 && self.val_to_idx.contains_key(&candidate) {
                        return true;
                    }
                }
            }
            return false;
        }
        self.members.iter().any(|(_, mv)| {
            if mv.has_xz() {
                return false;
            }
            let mut z = mv.clone();
            z.is_signed = false;
            z.resize(w).case_eq(&src_w).to_u64() == Some(1)
                || (self.signed
                    && src.is_signed
                    && mv.resize(w).case_eq(&src_w).to_u64() == Some(1))
        })
    }
}

/// How many unpacked selects an expression still takes before it denotes
/// one element.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Rank {
    /// A fixed number of dimensions; `Fixed(0)` is one element.
    Fixed(usize),
    /// A queue, dynamic or associative array that has not been selected.
    Open,
    /// An element of a queue, dynamic or associative array. A further
    /// select is allowed (a queue of queues), and the type decides.
    Selected,
}

impl Rank {
    fn select(self) -> Option<Rank> {
        match self {
            Rank::Fixed(0) => None,
            Rank::Fixed(n) => Some(Rank::Fixed(n - 1)),
            Rank::Open | Rank::Selected => Some(Rank::Selected),
        }
    }

    fn selects(self, n: usize) -> Option<Rank> {
        (0..n).try_fold(self, |r, _| r.select())
    }

    fn is_element(self) -> bool {
        matches!(self, Rank::Fixed(0) | Rank::Selected)
    }

    fn of_dims(dims: &[UD]) -> Rank {
        if dims.iter().any(|d| {
            matches!(
                d,
                UD::Unsized(_) | UD::Queue { .. } | UD::Associative { .. }
            )
        }) {
            Rank::Open
        } else {
            Rank::Fixed(dims.len())
        }
    }
}

impl Simulator {
    /// Whether `m` called with `nargs` arguments can be an enumeration
    /// method (§6.19.5).
    pub(super) fn is_enum_method(m: &str, nargs: usize) -> bool {
        match m {
            "first" | "last" | "num" | "name" => nargs == 0,
            "next" | "prev" => nargs <= 1,
            _ => false,
        }
    }

    /// The result of the enumeration method `m` on the receiver `recv`, when
    /// the receiver's declared type is an enumeration. `None` for any other
    /// receiver: the call is then not an enumeration method.
    pub(super) fn enum_method_on_receiver(
        &mut self,
        recv: &Expression,
        m: &str,
        args: &[Expression],
    ) -> Option<Value> {
        let ty = self.enum_receiver_type(recv)?;
        if ty.members.is_empty() {
            return None;
        }
        // `first`, `last` and `num` do not read the receiver, but a call in
        // it still runs.
        if matches!(m, "first" | "last" | "num")
            && (matches!(recv.kind, ExprKind::Call { .. }) || Self::chain_has_call_base(recv))
        {
            let _ = self.eval_expr(recv);
        }
        Some(match m {
            "num" => Self::int_count(ty.members.len() as u64),
            "first" => ty.members[0].1.clone(),
            "last" => ty.members[ty.members.len() - 1].1.clone(),
            "name" => {
                let v = self.eval_expr(recv);
                // §6.19.5.6: a value that is not a member has the empty name.
                match ty.position(&v) {
                    Some(p) => ty.name_vals[p].clone(),
                    None => Value::from_string(""),
                }
            }
            "next" | "prev" => {
                let step = match args.first() {
                    Some(a) => {
                        let av = self.eval_expr(a);
                        if av.is_real {
                            av.to_f64().round() as i64 as u32 as u64
                        } else {
                            // §6.19.5.4: step formal is `int unsigned` (32 bits).
                            av.resize_for_assign(32).to_u64().unwrap_or(1) as u32 as u64
                        }
                    }
                    None => 1,
                };
                let v = self.eval_expr(recv);
                let n = ty.members.len();
                match ty.position(&v) {
                    Some(p) => {
                        let step = (step % n as u64) as usize;
                        let q = if m == "next" {
                            (p + step) % n
                        } else {
                            (p + n - step) % n
                        };
                        ty.members[q].1.clone()
                    }
                    // §6.19.5.3: from a value that is not a member, the
                    // default initial value of the base type.
                    None if ty.two_state => Value::zero(ty.width),
                    None => Value::new(ty.width),
                }
            }
            _ => return None,
        })
    }

    /// The enumeration type the receiver `recv` is declared with.
    pub(super) fn enum_receiver_type(
        &mut self,
        recv: &Expression,
    ) -> Option<std::rc::Rc<EnumRecvType>> {
        // A plain variable or property keeps the resolution the rest of the
        // simulator uses for it (the frame's own declaration first), with a
        // typedef alias followed to the enumeration it names.
        if let ExprKind::Ident(h) = &recv.kind
            && h.path.len() == 1
            && h.path[0].selects.is_empty()
        {
            let name = h.path[0].name.name.as_str();
            if self.local_class_type_of(name).is_some() {
                return None;
            }
            if let Some(tn) = self
                .local_typedef_type_of(name)
                .or_else(|| self.get_expr_type_name(recv))
            {
                if let Some(ty) = self.enum_type_named(&tn) {
                    // A whole queue, dynamic or associative array has methods
                    // of the same names (§7.9, §7.10), and they are not these.
                    if self.plain_name_is_collection(h, name) {
                        return None;
                    }
                    return Some(ty);
                }
                let base = tn.split('#').next().unwrap_or(&tn);
                if self.module.classes.contains_key(base) {
                    return None;
                }
            }
        } else if !matches!(recv.kind, ExprKind::Index { .. })
            && let Some(n) = self.expr_assoc_name(recv)
            && (self.is_associative_array(&n) || self.module.dynamic_arrays.contains(&n))
        {
            return None;
        }
        let (dt, rank) = self.receiver_decl_type(recv, 0)?;
        if !rank.is_element() {
            return None;
        }
        self.enum_type_of(&dt).map(std::rc::Rc::new)
    }

    /// §6.19: declare the member `name` of an enumeration declared in a
    /// procedural block. The name was not known when the block was
    /// elaborated, so a read of it finds the constant in both places a
    /// name is looked up.
    pub(super) fn declare_enum_constant(&mut self, name: &str, v: Value) {
        self.module.parameters.insert(name.to_string(), v.clone());
        // The scopes that have parameters now include the constant's.
        self.decl_param_scope_cache.get_mut().note_param(name);
        self.set_signal_value_by_name(name, v);
    }

    /// Whether the plain name `name` is a whole unpacked array, queue or
    /// associative array: a variable the design tables hold as one, or a
    /// collection property of the method's class.
    fn plain_name_is_collection(&self, h: &HierarchicalIdentifier, name: &str) -> bool {
        let storage = self.resolve_hier_name(h);
        if self.module.dynamic_arrays.contains(&*storage)
            || self.module.associative_arrays.contains_key(&*storage)
            || self.foreach_dims(&storage).is_some()
        {
            return true;
        }
        if self.frame_get(name).is_some() {
            return false;
        }
        let mut cur = self.class_context_stack.last().and_then(|c| c.as_deref());
        let mut guard = 0;
        while let Some(c) = cur {
            guard += 1;
            let Some(cd) = self.module.classes.get(c).filter(|_| guard < 64) else {
                break;
            };
            if cd.properties.contains_key(name) {
                return cd.queue_properties.contains_key(name)
                    || cd.assoc_properties.contains_key(name)
                    || cd.array_of_coll_properties.contains_key(name)
                    || cd.array_properties.contains_key(name)
                    || cd.array_nd_properties.contains_key(name);
            }
            cur = cd.extends.as_deref();
        }
        false
    }

    /// The enumeration the type name `tn` denotes, following typedef
    /// aliases and array typedefs (`typedef level_e alias_e;`).
    fn enum_type_named(&mut self, tn: &str) -> Option<std::rc::Rc<EnumRecvType>> {
        if self.enum_recv_cache_gen != self.type_tables_gen {
            self.enum_recv_cache.clear();
            self.enum_recv_cache_gen = self.type_tables_gen;
        }
        let ctx_key: &str = match self.class_context_stack.last() {
            Some(Some(c)) => c.as_str(),
            _ => "",
        };
        if let Some(hit) = self.enum_recv_cache.get(ctx_key).and_then(|m| m.get(tn)) {
            return hit.clone();
        }
        let found = (self.module.enum_members.contains_key(tn)
            || self.lookup_typedef_target(tn).is_some())
        .then(|| self.enum_type_of(&Self::scoped_type_ref(tn)))
        .flatten()
        .map(std::rc::Rc::new);
        self.enum_recv_cache
            .entry(ctx_key.to_string())
            .or_default()
            .insert(tn.to_string(), found.clone());
        found
    }

    fn class_ctx_typedef_target(&self, nm: &str) -> Option<DataType> {
        let mut cur = self.class_context_stack.last().and_then(|c| c.as_deref());
        let mut guard = 0;
        while let Some(c) = cur {
            guard += 1;
            let Some(cd) = self.module.classes.get(c).filter(|_| guard < 64) else {
                break;
            };
            if let Some(dt) = cd.typedef_targets.get(nm) {
                return Some(dt.clone());
            }
            cur = cd.extends.as_deref();
        }
        None
    }

    /// The enumeration `dt` denotes, through typedef aliases and array
    /// typedefs; `None` when `dt` is no enumeration.
    fn enum_type_of(&self, dt: &DataType) -> Option<EnumRecvType> {
        let mut cur = dt.clone();
        // The member table of a named enumeration, when one is reached.
        let mut named: Option<&Vec<(String, u64)>> = None;
        let mut named_key: Option<String> = None;
        for _ in 0..16 {
            match &cur {
                DataType::Enum(_) => break,
                DataType::TypeReference { name, .. } => {
                    if !name.has_scope()
                        && let Some(class_dt) = self.class_ctx_typedef_target(&name.name.name)
                    {
                        cur = class_dt;
                        continue;
                    }
                    if named.is_none() {
                        let em = if name.has_scope() {
                            self.module.enum_members.get(&name.qualified())
                        } else {
                            self.module.enum_members.get(&name.name.name)
                        };
                        if let Some(m) = em {
                            named = Some(m);
                            named_key = Some(if name.has_scope() {
                                name.qualified()
                            } else {
                                name.name.name.clone()
                            });
                        }
                    }
                    let next = if name.has_scope() {
                        super::elaborate::lookup_scoped_type(name, &self.module.typedef_types)
                            .cloned()
                    } else {
                        self.lookup_typedef_target(&name.name.name)
                    };
                    match next {
                        Some(n) => cur = n,
                        None => break,
                    }
                }
                _ => return None,
            }
        }
        if named.is_none() && !matches!(cur, DataType::Enum(_)) {
            return None;
        }
        let (width, signed, two_state) = if matches!(cur, DataType::Enum(_)) {
            let tt = &self.module.typedef_types;
            let w = super::elaborate::resolve_type_width(
                &cur,
                Some(&self.module.parameters),
                Some(&self.module.typedefs),
            );
            (
                w.max(1),
                super::elaborate::is_type_signed_resolved(&cur, tt),
                super::elaborate::is_type_two_state_resolved(&cur, tt),
            )
        } else {
            // The declaration is not in reach (a class-scoped typedef): the
            // member constants carry the base type.
            let first = named?.first()?.0.as_str();
            let w = self
                .signal_name_to_id
                .get(first)
                .and_then(|id| self.signal_widths.get(*id).copied())
                .or_else(|| self.module.parameters.get(first).map(|v| v.width))
                .unwrap_or(32)
                .max(1);
            (
                w,
                self.lookup_signal_signed(first)
                    || self.module.parameters.get(first).is_some_and(|v| v.is_signed),
                self.module.two_state_signals.contains(first),
            )
        };
        let members: Vec<(String, Value)> = match (named, &cur) {
            (Some(raw), _) => {
                let prefix = named_key.as_deref().and_then(|k| {
                    k.rfind("::")
                        .map(|i| &k[..=i + 1])
                        .or_else(|| k.rfind('.').map(|i| &k[..=i]))
                });
                raw.iter()
                    .map(|(nm, val)| {
                        let pv = prefix
                            .and_then(|pfx| self.module.parameters.get(&format!("{pfx}{nm}")))
                            .or_else(|| {
                                self.module.parameters.get(nm).filter(|mv| {
                                    mv.has_xz() || width > 64 || mv.to_u64() == Some(*val)
                                })
                            });
                        let mut v = match pv {
                            Some(mv) if !mv.is_real && (mv.has_xz() || width > 64) => {
                                mv.resize(width)
                            }
                            _ => Value::from_u64(*val, width),
                        };
                        v.is_signed = signed;
                        (nm.clone(), v)
                    })
                    .collect()
            }
            (None, DataType::Enum(et)) => {
                super::elaborate::enum_members_values(et, width, signed, &self.module.parameters)
                    .into_iter()
                    .map(|(nm, _val, v)| (nm, v))
                    .collect()
            }
            _ => return None,
        };
        Some(EnumRecvType::new(members, width, signed, two_state))
    }

    /// The declared type of the expression `e` and how many unpacked
    /// selects it still takes. An array typedef in the type is counted in
    /// the rank, so the type returned is the element type.
    fn receiver_decl_type(&mut self, e: &Expression, depth: u32) -> Option<(DataType, Rank)> {
        if depth > 32 {
            return None;
        }
        match &e.kind {
            ExprKind::Paren(inner) => self.receiver_decl_type(inner, depth + 1),
            ExprKind::SystemCall { name, args }
                if (name == "$__xz_named_cast" || name == "$__xz_type_cast") && args.len() == 2 =>
            {
                let tn = self.named_cast_key(&args[0])?;
                if !self.module.enum_members.contains_key(&tn)
                    && self.lookup_typedef_target(&tn).is_none()
                {
                    return None;
                }
                Some(self.with_typedef_rank(Self::scoped_type_ref(&tn), Rank::Fixed(0)))
            }
            ExprKind::This => {
                let cn = self.class_context_stack.last().cloned().flatten()?;
                Some((Self::class_type_ref(&cn), Rank::Fixed(0)))
            }
            ExprKind::Index { expr, .. } => {
                let (dt, rank) = self.receiver_decl_type(expr, depth + 1)?;
                Some((dt, rank.select()?))
            }
            ExprKind::MemberAccess { expr, member } => {
                let (dt, rank) = self.receiver_decl_type(expr, depth + 1)?;
                if !rank.is_element() {
                    return None;
                }
                self.member_decl(&dt, &member.name)
            }
            ExprKind::Call { func, args } => {
                // §6.19.5: `first`, `last`, `next` and `prev` return a value
                // of the receiver's own type.
                let chained = match &func.kind {
                    ExprKind::MemberAccess { expr, member } => {
                        Some(((**expr).clone(), member.name.clone()))
                    }
                    ExprKind::Ident(h)
                        if h.path.len() >= 2
                            && h.path.last().is_some_and(|s| s.selects.is_empty()) =>
                    {
                        let m = h.path.last()?.name.name.clone();
                        Some(((*self.method_receiver_expr(h, h.path.len() - 1)).clone(), m))
                    }
                    _ => None,
                };
                if let Some((recv, m)) = chained
                    && m != "num"
                    && m != "name"
                    && Self::is_enum_method(&m, args.len())
                    && let Some((dt, rank)) = self.receiver_decl_type(&recv, depth + 1)
                    && rank.is_element()
                    && self.enum_type_of(&dt).is_some()
                {
                    return Some((dt, Rank::Fixed(0)));
                }
                let rt = self.call_return_type(func)?;
                Some(self.with_typedef_rank(rt, Rank::Fixed(0)))
            }
            ExprKind::Ident(h) => self.ident_decl_type(e, h),
            _ => None,
        }
    }

    /// `receiver_decl_type` of a (possibly dotted and selected) name.
    fn ident_decl_type(
        &mut self,
        e: &Expression,
        h: &HierarchicalIdentifier,
    ) -> Option<(DataType, Rank)> {
        // A variable, or a member path below a struct variable, by its
        // storage name; an instance prefix is part of that name.
        if let Some(flat) = self.flat_member_name(e)
            && let Some(found) = self.flat_decl_type(&flat)
        {
            return Some(found);
        }
        // A path through class handles, or below a local the frame types.
        let first = h.path.first()?;
        let (mut dt, mut rank) = self.root_decl_type(&first.name.name)?;
        rank = rank.selects(first.selects.len())?;
        for seg in &h.path[1..] {
            if !rank.is_element() {
                return None;
            }
            let (mdt, mrank) = self.member_decl(&dt, &seg.name.name)?;
            dt = mdt;
            rank = mrank.selects(seg.selects.len())?;
        }
        Some((dt, rank))
    }

    /// The declared type at the storage path `flat` (`v`, `arr[2]`,
    /// `s.inner.lvl`, `sarr[1].lvl`) of a variable the design tables hold.
    fn flat_decl_type(&self, flat: &str) -> Option<(DataType, Rank)> {
        let (dt, arr) = self.flat_path_type(flat)?;
        let selected = flat.ends_with(']');
        let rank = if arr.as_ref().is_some_and(|a| !a.is_empty()) {
            Rank::Fixed(arr.map_or(0, |a| a.len()))
        } else if self.module.dynamic_arrays.contains(flat)
            || self.module.associative_arrays.contains_key(flat)
        {
            Rank::Open
        } else if !selected && let Some(dims) = self.foreach_dims(flat) {
            Rank::Fixed(dims.len())
        } else if selected {
            // Selects were applied; whether they reached an element is the
            // declaration's to say, and the tables answered above for the
            // dimensions that are left.
            match self.partial_index_dims(flat) {
                Some(d) => Rank::Fixed(d.len()),
                None => Rank::Selected,
            }
        } else {
            Rank::Fixed(0)
        };
        // The tables hold the dimensions of an array typedef already.
        Some((self.element_type(dt), rank))
    }

    /// `dt` without the array typedefs in front of its element type.
    fn element_type(&self, dt: DataType) -> DataType {
        let mut cur = dt;
        for _ in 0..16 {
            let DataType::TypeReference { name, .. } = &cur else {
                break;
            };
            let key = if name.has_scope() {
                name.qualified()
            } else {
                name.name.name.clone()
            };
            if self
                .typedef_unpacked_dims_of(&key)
                .is_none_or(|d| d.is_empty())
            {
                break;
            }
            let next = if name.has_scope() {
                super::elaborate::lookup_scoped_type(name, &self.module.typedef_types).cloned()
            } else {
                self.lookup_typedef_target(&key)
            };
            match next {
                Some(n) => cur = n,
                None => break,
            }
        }
        cur
    }

    /// `dt` as an element type, with the dimensions of the array typedefs
    /// in front of it added to `rank`.
    fn with_typedef_rank(&self, dt: DataType, rank: Rank) -> (DataType, Rank) {
        let mut cur = dt;
        let mut rank = rank;
        for _ in 0..16 {
            let DataType::TypeReference { name, .. } = &cur else {
                break;
            };
            let key = if name.has_scope() {
                name.qualified()
            } else {
                name.name.name.clone()
            };
            let Some(dims) = self
                .typedef_unpacked_dims_of(&key)
                .filter(|d| !d.is_empty())
            else {
                break;
            };
            rank = match (rank, Rank::of_dims(dims)) {
                (Rank::Fixed(a), Rank::Fixed(b)) => Rank::Fixed(a + b),
                (Rank::Selected, Rank::Fixed(0)) => Rank::Selected,
                (Rank::Selected, Rank::Fixed(b)) => Rank::Fixed(b),
                _ => Rank::Open,
            };
            let next = if name.has_scope() {
                super::elaborate::lookup_scoped_type(name, &self.module.typedef_types).cloned()
            } else {
                self.lookup_typedef_target(&key)
            };
            match next {
                Some(n) => cur = n,
                None => break,
            }
        }
        (cur, rank)
    }

    /// The declared type of the variable `name` that heads a receiver: a
    /// local or formal of the running subroutine, a property of the
    /// method's class, or a design variable holding a class handle.
    fn root_decl_type(&self, name: &str) -> Option<(DataType, Rank)> {
        let frames = match self.method_local_base.last() {
            Some(&base) => self.local_stack.get(base..).unwrap_or(&[]),
            None => &self.local_stack[..],
        };
        let marker = format!("{}.", name);
        let in_frame = frames
            .iter()
            .rev()
            .any(|m| m.contains_key(name) || m.contains_key(marker.as_str()));
        let storage = self.dyn_name_lookup(name).unwrap_or(name);
        let table_rank = |sim: &Self| -> Option<Rank> {
            if sim.module.dynamic_arrays.contains(storage)
                || sim.module.associative_arrays.contains_key(storage)
            {
                Some(Rank::Open)
            } else {
                sim.foreach_dims(storage).map(|d| Rank::Fixed(d.len()))
            }
        };
        if in_frame {
            let dt = if let Some(c) = self.local_class_type_of(name) {
                Self::class_type_ref(&c)
            } else if let Some(t) = self.local_typedef_type_of(name).filter(|t| !t.is_empty()) {
                Self::class_type_ref(&t)
            } else {
                self.module.var_decl_types.get(name)?.clone()
            };
            return Some(match table_rank(self) {
                Some(r) => (self.element_type(dt), r),
                None => self.with_typedef_rank(dt, Rank::Fixed(0)),
            });
        }
        if let Some(Some(ctx)) = self.class_context_stack.last()
            && let Some(found) = self.class_property_decl(ctx, name)
        {
            return Some(found);
        }
        let dt = match self.module.var_decl_types.get(storage) {
            Some(dt) => dt.clone(),
            None => Self::class_type_ref(self.var_class_types.get(name)?),
        };
        Some(match table_rank(self) {
            Some(r) => (self.element_type(dt), r),
            None => self.with_typedef_rank(dt, Rank::Fixed(0)),
        })
    }

    /// The declared type of member `m` of a value of type `dt`: a property
    /// of a class, or a member of a struct or union.
    fn member_decl(&self, dt: &DataType, m: &str) -> Option<(DataType, Rank)> {
        if let DataType::TypeReference { name, .. } = dt {
            let cn = name.name.name.as_str();
            let cn = self
                .resolve_type_param_binding(cn)
                .unwrap_or_else(|| cn.to_string());
            let base = cn.split('#').next().unwrap_or(&cn);
            if self.module.classes.contains_key(base) {
                return self.class_property_decl(base, m);
            }
        }
        let DataType::Struct(su) = self.resolve_dt_ref(dt) else {
            return None;
        };
        let (mdt, dims) = su.members.iter().find_map(|sm| {
            sm.declarators
                .iter()
                .find(|d| d.name.name == m)
                .map(|d| (sm.data_type.clone(), &d.dimensions))
        })?;
        Some(self.with_typedef_rank(mdt, Rank::of_dims(dims)))
    }

    /// The declared element type and rank of property `prop` of class `cn`
    /// or an ancestor. A typedef of the declaring class is replaced by its
    /// target, so the type reads the same outside the class.
    fn class_property_decl(&self, cn: &str, prop: &str) -> Option<(DataType, Rank)> {
        let mut cur = Some(cn);
        let mut guard = 0;
        while let Some(c) = cur {
            guard += 1;
            if guard > 64 {
                return None;
            }
            let cd = self.module.classes.get(c)?;
            if let Some(dt) = cd.property_types.get(prop) {
                let rank = if cd.queue_properties.contains_key(prop)
                    || cd.assoc_properties.contains_key(prop)
                    || cd.array_of_coll_properties.contains_key(prop)
                {
                    Some(Rank::Open)
                } else if let Some((shape, _)) = cd.array_nd_properties.get(prop) {
                    Some(Rank::Fixed(shape.len()))
                } else if cd.array_properties.contains_key(prop) {
                    Some(Rank::Fixed(1))
                } else {
                    None
                };
                let mut dt = dt.clone();
                if let DataType::TypeReference { name, .. } = &dt
                    && !name.has_scope()
                    && let Some(target) = cd.typedef_targets.get(&name.name.name)
                    && cd
                        .typedef_unpacked_dims
                        .get(&name.name.name)
                        .is_none_or(|d| d.is_empty())
                {
                    dt = target.clone();
                }
                return Some(match rank {
                    Some(r) => (self.element_type(dt), r),
                    None => self.with_typedef_rank(dt, Rank::Fixed(0)),
                });
            }
            cur = cd.extends.as_deref();
        }
        None
    }
}
