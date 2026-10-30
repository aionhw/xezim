//! Simulator value display.

use super::*;

impl Simulator {
    /// §21.2.1.7 / §6.19: the label of `v` when `dt` names an enum (declared
    /// bare, in a package, or through a typedef chain); `None` otherwise or
    /// for an x/z value, which matches no label.
    pub(super) fn p_enum_label(&self, v: &Value, dt: &DataType) -> Option<String> {
        let DataType::TypeReference { name: tn, .. } = dt else {
            return None;
        };
        // The name text may carry its package (`pkg::e_t`) while the table
        // is keyed by the typedef's bare name, or the reverse.
        let bare = tn.name.name.rsplit("::").next().unwrap_or(&tn.name.name);
        let members = self
            .module
            .enum_members
            .get(&tn.qualified())
            .or_else(|| self.module.enum_members.get(&tn.name.name))
            .or_else(|| self.module.enum_members.get(bare))
            .or_else(|| {
                // `import pkg::*` leaves the bare name: the one package
                // declaring it.
                let suffix = format!("::{bare}");
                let mut hits = self
                    .module
                    .enum_members
                    .iter()
                    .filter(|(k, _)| k.ends_with(&suffix));
                let first = hits.next();
                if hits.next().is_some() { None } else { first.map(|(_, m)| m) }
            })?;
        if v.has_xz() {
            return None;
        }
        let x = v.to_u64()?;
        members
            .iter()
            .find(|(_, mv)| *mv == x)
            .map(|(label, _)| label.clone())
    }

    /// `%p` of one packed leaf by its declared type, as the majority of the
    /// reference simulators prints it (#566): a 2/4-state VECTOR type
    /// (`bit`, `logic`, `reg`, with any packed dimensions) as `'h` hex with
    /// uppercase X/Z digits, a vector declared `signed` and the integer atoms
    /// (`byte`, `int`, `shortint`, `longint`, `integer`) as decimal with
    /// their declared signedness, an enum as its label, a real as a real and
    /// a string quoted.
    pub(super) fn p_leaf_value(&self, v: &Value, dt: &DataType) -> String {
        use crate::ast::types::{IntegerAtomType, Signing, SimpleType};
        if let Some(label) = self.p_enum_label(v, dt) {
            return label;
        }
        match self.resolve_dt(dt) {
            DataType::Real { .. } => Self::p_real(v.to_f64()),
            DataType::Simple {
                kind: SimpleType::String,
                ..
            } => format!("\"{}\"", v.to_sv_string()),
            DataType::IntegerAtom { kind, signing, .. } => {
                let mut x = v.clone();
                x.is_signed = match signing {
                    Some(s) => s == Signing::Signed,
                    None => kind != IntegerAtomType::Time,
                };
                if x.is_real {
                    Self::p_real(x.to_f64())
                } else {
                    x.to_dec_string()
                }
            }
            DataType::IntegerVector {
                signing: Some(Signing::Signed),
                ..
            }
            | DataType::Implicit {
                signing: Some(Signing::Signed),
                ..
            } => {
                let mut x = v.clone();
                x.is_signed = true;
                x.to_dec_string()
            }
            DataType::IntegerVector { .. } | DataType::Implicit { .. } => Self::p_hex(v),
            _ => {
                if v.is_real {
                    Self::p_real(v.to_f64())
                } else {
                    v.to_dec_string()
                }
            }
        }
    }

    /// `'h` hex of a vector value: lowercase digits, uppercase X/Z, leading
    /// zeros dropped (`'h0` for zero).
    pub(super) fn p_hex(v: &Value) -> String {
        let digits: String = v
            .to_hex()
            .chars()
            .map(|c| match c {
                'x' => 'X',
                'z' => 'Z',
                c => c,
            })
            .collect();
        let trimmed = digits.trim_start_matches('0');
        format!("'h{}", if trimmed.is_empty() { "0" } else { trimmed })
    }

    /// A real inside an aggregate: the shortest round-trip form for ordinary
    /// magnitudes, `%g`-style exponent form (six significant digits) for
    /// very small or very large ones, as the reference simulators print a
    /// reinterpreted union member (`3.21143e-322`).
    pub(super) fn p_real(x: f64) -> String {
        let a = x.abs();
        if x != 0.0 && x.is_finite() && !(1e-4..1e15).contains(&a) {
            let s = format!("{:.5e}", x);
            let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
            let m = if m.contains('.') {
                m.trim_end_matches('0').trim_end_matches('.')
            } else {
                m
            };
            let e: i32 = e.parse().unwrap_or(0);
            return format!("{m}e{}{:02}", if e < 0 { "-" } else { "+" }, e.abs());
        }
        format!("{x}")
    }

    /// Render an already-extracted `Value` interpreted as `dt` (used for the
    /// bit-slices of a packed struct, which have no signal of their own).
    pub(super) fn render_p_value_typed(&mut self, v: &Value, dt: &DataType) -> String {
        match self.resolve_dt(dt) {
            DataType::Struct(su) if su.packed => self.render_p_packed(v, &su),
            _ => self.p_leaf_value(v, dt),
        }
    }

    /// §21.2.1.7: `%p` of a call to a module function whose return type is
    /// an unpacked struct of plain members, member by member. `None` for any
    /// other argument.
    pub(super) fn render_p_struct_call(&mut self, arg: &Expression) -> Option<String> {
        let ExprKind::Call { func, .. } = &arg.kind else {
            return None;
        };
        let ExprKind::Ident(h) = &func.kind else {
            return None;
        };
        if h.path.len() != 1 {
            return None;
        }
        let rt = self
            .module
            .functions
            .get(&h.path[0].name.name)?
            .return_type
            .clone();
        let su = self.unpacked_struct_of(&rt)?;
        let plain = su.members.iter().all(|m| {
            m.declarators.iter().all(|d| d.dimensions.is_empty())
                && !matches!(self.resolve_dt_ref(&m.data_type), DataType::Struct(_))
        });
        if !plain {
            return None;
        }
        let v = self.eval_expr(arg);
        let leaves: HashMap<String, Value> =
            self.split_packed_struct("", &su, &v).into_iter().collect();
        let mut parts = Vec::new();
        for m in &su.members {
            for d in &m.declarators {
                let mut mv = leaves.get(&format!(".{}", d.name.name))?.clone();
                mv.is_signed = super::elaborate::is_type_signed_resolved(
                    &m.data_type,
                    &self.module.typedef_types,
                );
                parts.push(format!(
                    "{}:{}",
                    d.name.name,
                    self.render_p_value_typed(&mv, &m.data_type)
                ));
            }
        }
        Some(format!("'{{{}}}", parts.join(", ")))
    }

    /// Render a packed struct's members by slicing `v`. The FIRST declared
    /// member occupies the most-significant bits (LRM §7.2.1), so offsets are
    /// assigned from the last member up.
    pub(super) fn render_p_packed(
        &mut self,
        v: &Value,
        su: &crate::ast::types::StructUnionType,
    ) -> String {
        let mut items: Vec<(String, u32, DataType)> = Vec::new();
        for m in &su.members {
            let w = super::elaborate::resolve_type_width(
                &m.data_type,
                Some(&self.module.parameters),
                Some(&self.module.typedefs),
            )
            .max(1);
            for md in &m.declarators {
                items.push((md.name.name.clone(), w, m.data_type.clone()));
            }
        }
        let is_union = matches!(su.kind, crate::ast::types::StructUnionKind::Union);
        let mut offs = vec![0u32; items.len()];
        if !is_union {
            let mut off = 0u32;
            for i in (0..items.len()).rev() {
                offs[i] = off;
                off += items[i].1;
            }
        }
        // §7.3 / §21.2.1.7 (#566, majority): a PACKED union prints its first
        // member only; an unpacked union prints every member over the shared
        // storage, a `real` member as the bits reinterpreted.
        if is_union && !su.tagged && su.packed {
            items.truncate(1);
        }
        let parts: Vec<String> = (0..items.len())
            .map(|i| {
                let (n, w, mdt) = items[i].clone();
                let mut sub = Value::zero(w.max(1));
                for b in 0..w {
                    sub.set_bit(b as usize, v.get_bit((offs[i] + b) as usize));
                }
                // A slice loses the member's signedness: an `int` member holding
                // 32'hDEADBEEF prints -559038737, not 3735928559.
                sub.is_signed = super::elaborate::is_type_signed(&mdt);
                if is_union
                    && !su.packed
                    && matches!(self.resolve_dt(&mdt), DataType::Real { .. })
                    && !v.is_real
                {
                    let bits = sub.to_u64().unwrap_or(0);
                    return format!("{}:{}", n, Self::p_real(f64::from_bits(bits)));
                }
                format!("{}:{}", n, self.render_p_value_typed(&sub, &mdt))
            })
            .collect();
        format!("'{{{}}}", parts.join(", "))
    }

    /// `%p` of the unpacked array rooted at flat name `name` whose dimensions
    /// are `lists` (declared order, outermost first): one nested `'{...}` per
    /// dimension, left bound first (IEEE 1800-2017 §21.2.1.7).
    pub(super) fn render_p_elem_lists(
        &mut self,
        name: &str,
        dt: &DataType,
        lists: &[Vec<i64>],
    ) -> String {
        let Some((first, rest)) = lists.split_first() else {
            return self.render_p_typed(name, dt);
        };
        self.p_top_level = false;
        let parts: Vec<String> = first
            .iter()
            .map(|i| {
                let elem = format!("{}[{}]", name, i);
                if rest.is_empty() {
                    // An element may itself be a collection (`int aq[2][$]`:
                    // each `aq[i]` is a queue). A LOCAL such array has no
                    // `var_decl_types` entry, so recurse through `render_p_var`
                    // when the element is a registered collection; otherwise
                    // it renders as the scalar element type.
                    if self.module.dynamic_arrays.contains(&elem)
                        || self.module.associative_arrays.contains_key(&elem)
                        || self.module.arrays.contains_key(&elem)
                    {
                        if let Some(nested) = self.render_p_var(&elem) {
                            return nested;
                        }
                    }
                }
                self.render_p_elem_lists(&elem, dt, rest)
            })
            .collect();
        format!("'{{{}}}", parts.join(", "))
    }

    /// Render the value stored at flat signal `name`, interpreted as `dt`.
    pub(super) fn render_p_typed(&mut self, name: &str, dt: &DataType) -> String {
        use crate::ast::types::SimpleType;
        // Enum member -> its label (LRM prints the name, not the encoding).
        if let DataType::TypeReference { name: tn, .. } = dt
            && (self.module.enum_members.contains_key(&tn.qualified())
                || self.module.enum_members.contains_key(&tn.name.name)
                || self
                    .module
                    .enum_members
                    .contains_key(tn.name.name.rsplit("::").next().unwrap_or("")))
            && let Some(v) = self.get_signal_value_by_name(name)
        {
            return self.p_leaf_value(&v, dt);
        }
        match self.resolve_dt(dt) {
            DataType::Struct(su) => {
                // A tagged union prints only its active member.
                if let Some(tag) = self.active_union_tag.get(name).cloned() {
                    let v = self
                        .get_signal_value_by_name(name)
                        .unwrap_or_else(|| Value::zero(1));
                    return format!("'{{{}:{}}}", tag, v.to_dec_string());
                }
                // A PACKED struct has no per-member signals — its members are
                // bit-slices of the container's own value. So does an untagged
                // UNION, packed or not: one storage, every member at bit 0 (§7.3).
                let is_untagged_union =
                    matches!(su.kind, crate::ast::types::StructUnionKind::Union) && !su.tagged;
                if su.packed || is_untagged_union {
                    let v = self
                        .get_signal_value_by_name(name)
                        .unwrap_or_else(|| Value::zero(1));
                    return self.render_p_packed(&v, &su);
                }
                self.p_top_level = false;
                let mut parts = Vec::new();
                for m in &su.members {
                    for md in &m.declarators {
                        let mbase = format!("{}.{}", name, md.name.name);
                        let rendered = if md.dimensions.is_empty() {
                            self.render_p_typed(&mbase, &m.data_type)
                        } else if let Some(lists) = self.member_dim_indices(&md.dimensions) {
                            self.render_p_elem_lists(&mbase, &m.data_type, &lists)
                        } else if md.dimensions.len() == 1
                            && Self::coll_kind_of(&md.dimensions).is_some()
                            && self.ensure_struct_member_coll(&mbase)
                        {
                            // §21.2.1.7: a dynamic member prints its live
                            // elements, like the same collection declared alone.
                            self.render_p_var(&mbase)
                                .unwrap_or_else(|| "'{}".to_string())
                        } else {
                            "'{}".to_string()
                        };
                        parts.push(format!("{}:{}", md.name.name, rendered));
                    }
                }
                format!("'{{{}}}", parts.join(", "))
            }
            DataType::Real { .. } => self
                .get_signal_value_by_name(name)
                .map(|v| format!("{}", v.to_f64()))
                .unwrap_or_else(|| "0".into()),
            DataType::Simple {
                kind: SimpleType::String,
                ..
            } => self
                .get_signal_value_by_name(name)
                .map(|v| format!("\"{}\"", v.to_sv_string()))
                .unwrap_or_else(|| "\"\"".into()),
            _ => match self.get_signal_value_by_name(name) {
                Some(v) if std::mem::take(&mut self.p_top_level) => {
                    if v.is_real {
                        format!("{}", v.to_f64())
                    } else if let Some(label) = self.p_enum_label(&v, dt) {
                        label
                    } else {
                        v.to_dec_string()
                    }
                }
                Some(v) => self.p_leaf_value(&v, dt),
                None => self.p_unwritten_leaf(name, dt),
            },
        }
    }

    /// The declared data type of a `%p` operand that no name-based path
    /// renders: a class property through a handle, `this` or bare (and an
    /// element of a collection property), a static `Cls::p` /
    /// `C#(..)::p`, an interface or virtual-interface member, an element
    /// of any of those, and a function result (§21.2.1.7).
    pub(super) fn p_decl_type(&mut self, arg: &Expression) -> Option<DataType> {
        match &arg.kind {
            ExprKind::Paren(e) => self.p_decl_type(e),
            ExprKind::Call { func, .. } => self.call_return_type(func),
            // An element of an unpacked collection keeps the declared
            // element type (the unpacked dimensions sit on the declarator).
            ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => {
                self.p_decl_type(expr)
            }
            _ => {
                if !self.no_class_objects()
                    && let Some((handle, prop)) = self.class_prop_receiver(arg)
                    && handle != 0
                    && let Some(tn) = self.class_prop_type_name(handle, &prop)
                    && let Some(dt) = self.p_type_by_name(&tn)
                {
                    return Some(dt);
                }
                if let Some((cls, prop)) = self.p_static_prop_parts(arg)
                    && let Some(tn) = self.p_static_prop_type_name(arg, &cls, &prop)
                    && let Some(dt) = self.p_type_by_name(&tn)
                {
                    return Some(dt);
                }
                let flat = self.flat_member_name(arg)?;
                let base = flat.split('[').next().unwrap_or(&flat).to_string();
                if let Some(dt) = self.module.var_decl_types.get(&base) {
                    return Some(dt.clone());
                }
                // A subroutine formal or local of the running automatic task.
                if !base.contains('.')
                    && let Some(task) = self
                        .current_auto_task
                        .clone()
                        .or_else(|| self.current_static_task.clone())
                    && let Some(dt) = self.p_task_local_type(&task, &base)
                {
                    return Some(dt);
                }
                // A member reached through a virtual interface variable.
                if let Some((head, rest)) = base.split_once('.') {
                    let head_expr = Expression::new(
                        ExprKind::Ident(HierarchicalIdentifier {
                            root: None,
                            path: vec![HierPathSegment {
                                name: crate::ast::Identifier {
                                    name: head.to_string(),
                                    span: arg.span,
                                },
                                selects: Vec::new(),
                            }],
                            span: arg.span,
                            cached_signal_id: std::cell::Cell::new(None),
                            cached_resolved_name: std::cell::OnceCell::new(),
                        }),
                        arg.span,
                    );
                    if let Some(Some(inst)) = self.vif_operand(&head_expr)
                        && let Some(dt) = self.module.var_decl_types.get(&format!("{inst}.{rest}"))
                    {
                        return Some(dt.clone());
                    }
                }
                let ident = HierarchicalIdentifier {
                    root: None,
                    path: vec![HierPathSegment {
                        name: crate::ast::Identifier {
                            name: base.clone(),
                            span: arg.span,
                        },
                        selects: Vec::new(),
                    }],
                    span: arg.span,
                    cached_signal_id: std::cell::Cell::new(None),
                    cached_resolved_name: std::cell::OnceCell::new(),
                };
                let resolved = self.resolve_hier_name(&ident).to_string();
                if resolved != base
                    && let Some(dt) = self.module.var_decl_types.get(&resolved)
                {
                    return Some(dt.clone());
                }
                None
            }
        }
    }

    /// `Cls::p` (flat two-segment ident) or `C#(args)::p` (specialization
    /// member access) → the class and property names.
    fn p_static_prop_parts(&mut self, arg: &Expression) -> Option<(String, String)> {
        match &arg.kind {
            ExprKind::Ident(h)
                if h.path.len() == 2
                    && h.path.iter().all(|s| s.selects.is_empty())
                    && self.module.classes.contains_key(&h.path[0].name.name) =>
            {
                Some((h.path[0].name.name.clone(), h.path[1].name.name.clone()))
            }
            ExprKind::MemberAccess { expr, member } => {
                let base = match &expr.kind {
                    ExprKind::Specialization { base, .. } => base.as_ref(),
                    _ => expr.as_ref(),
                };
                let ExprKind::Ident(h) = &base.kind else {
                    return None;
                };
                let cls = h.path.iter().map(|s| s.name.name.as_str()).collect::<Vec<_>>().join("::");
                if !self.module.classes.contains_key(&cls) {
                    return None;
                }
                Some((cls, member.name.clone()))
            }
            _ => None,
        }
    }

    /// The declared type name of static property `prop` of `cls`; a type
    /// parameter is mapped through the operand's `#(...)` arguments.
    fn p_static_prop_type_name(&mut self, arg: &Expression, cls: &str, prop: &str) -> Option<String> {
        let owners = self.prop_owners(cls);
        let (_, cd) = owners.get(prop)?;
        let tn = cd.properties.get(prop)?.type_name.clone()?;
        if !cd.type_param_names.iter().any(|t| *t == tn) {
            return Some(tn);
        }
        let idx = cd.param_order.iter().position(|p| *p == tn)?;
        let text = match &arg.kind {
            ExprKind::MemberAccess { expr, .. } => match &expr.kind {
                ExprKind::Specialization { type_args_text, .. } => type_args_text.clone(),
                _ => return None,
            },
            _ => return None,
        };
        let args = Self::split_spec_args(&text);
        args.get(idx).map(|a| a.trim().to_string())
    }

    /// The declared type of formal or local `name` of task `task`.
    fn p_task_local_type(&self, task: &str, name: &str) -> Option<DataType> {
        let td = self.module.tasks.get(task)?;
        if let Some(p) = td.ports.iter().find(|p| p.name.name == name) {
            return Some(p.data_type.clone());
        }
        for st in &td.items {
            if let StatementKind::VarDecl { data_type, declarators, .. } = &st.kind
                && declarators.iter().any(|d| d.name.name == name)
            {
                return Some(data_type.clone());
            }
        }
        None
    }

    /// Is the `%p` operand a whole collection (queue, dynamic or
    /// associative array, unpacked array)? Those render element-wise by
    /// name; the typed fallback is for single packed values.
    pub(super) fn p_operand_is_collection(&mut self, arg: &Expression) -> bool {
        let Some(flat) = self.flat_member_name(arg) else {
            return false;
        };
        self.module.dynamic_arrays.contains(&flat)
            || self.module.associative_arrays.contains_key(&flat)
            || self.module.arrays.contains_key(&flat)
            || self.module.arrays_2d.contains_key(&flat)
            || self.module.arrays_nd.contains_key(&flat)
    }

    /// A declared type by its (possibly package-qualified) name.
    pub(super) fn p_type_by_name(&self, tn: &str) -> Option<DataType> {
        if let Some(dt) = self.module.typedef_types.get(tn) {
            return Some(dt.clone());
        }
        if self.module.enum_members.contains_key(tn) {
            return Some(DataType::TypeReference {
                name: crate::ast::types::TypeName {
                    scopes: Vec::new(),
                    name: crate::ast::Identifier {
                        name: tn.to_string(),
                        span: crate::ast::Span::dummy(),
                    },
                    span: crate::ast::Span::dummy(),
                },
                dimensions: Vec::new(),
                type_args: Vec::new(),
                span: crate::ast::Span::dummy(),
            });
        }
        let suffix = format!("::{tn}");
        let mut hits = self
            .module
            .typedef_types
            .iter()
            .filter(|(k, _)| k.ends_with(&suffix));
        let first = hits.next();
        if hits.next().is_some() {
            return None;
        }
        first.map(|(_, dt)| dt.clone())
    }

    /// §8.9 / §21.2.1.7: `%p` of a static fixed array reached through its
    /// class scope — `Cls::S`, `C#(int)::S` — or of one element of it,
    /// `Cls::S[i]`, rendered from the declared element type. A
    /// specialization's scope is the context its type parameters resolve
    /// in (the store itself is shared by the class's specializations).
    pub(super) fn render_p_static_fixed_operand(&mut self, arg: &Expression) -> Option<String> {
        let (base, index) = match &arg.kind {
            ExprKind::Index { expr, index } => (&**expr, Some(&**index)),
            _ => (arg, None),
        };
        let class_scope = |c: &str| {
            self.module.classes.contains_key(c)
                && !self.local_stack.last().is_some_and(|l| l.contains_key(c))
        };
        // (class, its specialization, member)
        let (cls, spec, member) = match &base.kind {
            ExprKind::MemberAccess {
                expr: scope,
                member,
            } => match &scope.kind {
                ExprKind::Ident(h)
                    if h.path.len() == 1
                        && h.path[0].selects.is_empty()
                        && class_scope(&h.path[0].name.name) =>
                {
                    (h.path[0].name.name.clone(), None, member.name.clone())
                }
                ExprKind::Specialization { .. } => {
                    let spec = self.resolve_call_spec_params(
                        Self::extract_call_spec(scope),
                        &self.current_spec,
                    )?;
                    (spec.0.clone(), Some(spec), member.name.clone())
                }
                _ => return None,
            },
            // The elaborated spelling of `Cls::S`: one two-segment name.
            ExprKind::Ident(h)
                if index.is_some()
                    && h.path.len() == 2
                    && h.path.iter().all(|s| s.selects.is_empty())
                    && class_scope(&h.path[0].name.name) =>
            {
                (
                    h.path[0].name.name.clone(),
                    None,
                    h.path[1].name.name.clone(),
                )
            }
            _ => return None,
        };
        let key = self.static_fixed_key_in(&cls, &member)?;
        let saved = spec.map(|sp| std::mem::replace(&mut self.current_spec, Some(sp)));
        let out = match index {
            None => self.render_p_static_fixed(&cls, &member),
            Some(ix) => {
                let decl = key
                    .strip_suffix(member.as_str())
                    .and_then(|k| k.strip_suffix("::"))
                    .and_then(|d| self.module.classes.get(d))
                    .cloned();
                let dt = decl
                    .and_then(|cd| self.class_prop_decl_type(&cd, &member).cloned())
                    .or_else(|| self.class_member_coll_type(&key).cloned())
                    .or_else(|| self.flat_path_type(&key).map(|(d, _)| d));
                let i = self.eval_expr(ix).to_i64();
                match (dt, i) {
                    (Some(dt), Some(i)) => {
                        let elem = format!("{}[{}]", key, i);
                        Some(self.render_p_typed(&elem, &dt))
                    }
                    _ => None,
                }
            }
        };
        if let Some(prev) = saved {
            self.current_spec = prev;
        }
        out
    }

    /// §6.8 / §21.2.1.7: an UNWRITTEN member leaf (no store yet — an element
    /// of a static fixed array of structs) holds its type's default: 0 for
    /// a 2-state type, x otherwise. A type parameter (`T v`) resolves through
    /// the active specialization, else the one the store key names
    /// (`P#int::fa[0].v`).
    #[cold]
    pub(super) fn p_unwritten_leaf(&self, name: &str, dt: &DataType) -> String {
        self.p_unwritten_leaf_of(None, name, dt)
    }

    /// [`p_unwritten_leaf`] for a cell of object (or static holder)
    /// `owner`, whose own bindings resolve a type parameter first.
    #[cold]
    pub(super) fn p_unwritten_leaf_of(
        &self,
        owner: Option<usize>,
        name: &str,
        dt: &DataType,
    ) -> String {
        let mut dt = dt.clone();
        if let DataType::TypeReference { name: tn, .. } = &dt
            && tn.scopes.is_empty()
            && !self.module.typedef_types.contains_key(&tn.name.name)
        {
            let t = tn.name.name.clone();
            let inst = owner.and_then(|h| self.heap.get(h)?.as_ref());
            let keyed = name
                .split_once("::")
                .and_then(|(h, _)| h.split_once('#'))
                .map(|(b, sig)| (b.to_string(), sig.to_string()));
            let bound = inst
                .and_then(|i| i.type_bindings.get(&t).cloned())
                .or_else(|| {
                    let spec = inst.and_then(|i| i.spec.clone())?;
                    self.resolve_type_param_with(&t, &Some(spec))
                })
                .or_else(|| self.resolve_type_param_binding(&t))
                .or_else(|| keyed.and_then(|k| self.resolve_type_param_with(&t, &Some(k))));
            if let Some(d) = bound
                .filter(|b| *b != t)
                .and_then(|b| self.bound_type_dt(&b))
            {
                dt = d;
            }
        }
        if super::elaborate::is_type_two_state_resolved(&dt, &self.module.typedef_types) {
            "0".into()
        } else {
            "x".into()
        }
    }


    /// §21.2.1.7 / §7.2: render a WHOLE unpacked-struct CLASS property
    /// (`c.first` where `first` is a `typedef struct` bound through a class
    /// type parameter, e.g. `uvm_built_in_pair #(s,s)::first`) the way a
    /// module-scope struct renders. The leaves of such a property live in the
    /// instance `PropMap` keyed `<prop>.<member>` (and, for nested members,
    /// `<prop>.<m1>.<m2>`), whereas the name-based `render_p_var` path
    /// resolves `first` as an untyped strip and printed the raw packed value
    /// (`pair : 18446743648507789112`) or `x` instead of `'{a:-100, b:-200}`.
    /// `leaf` is the PropMap key prefix (initially the bare `prop`; recurses
    /// into nested struct members).
    pub(super) fn render_p_class_prop_struct(
        &mut self,
        handle: usize,
        leaf: &str,
        su: &crate::ast::types::StructUnionType,
    ) -> String {
        let mut parts = Vec::new();
        for m in &su.members {
            for md in &m.declarators {
                let key = format!("{}.{}", leaf, md.name.name);
                // §7.10/§21.2.1.7: a dynamic / queue / associative member
                // (declared so, or through a collection typedef) is the
                // collection `<handle>#<key>`, not a cell.
                let coll_dims: Option<Vec<crate::ast::types::UnpackedDimension>> =
                    if !md.dimensions.is_empty() {
                        Some(md.dimensions.clone())
                    } else if let DataType::TypeReference {
                        name, dimensions, ..
                    } = &m.data_type
                        && dimensions.is_empty()
                        && !name.has_scope()
                    {
                        self.class_chain_typedef_dims(handle, &name.name.name)
                    } else {
                        None
                    };
                if let Some(cd) = coll_dims.as_deref()
                    && let Some(kind) = Self::coll_kind_of(cd)
                {
                    let string_key = self.string_keyed_dim(&cd[0]);
                    let ew = resolve_type_width(
                        &m.data_type,
                        Some(&self.module.parameters),
                        Some(&self.module.typedefs),
                    )
                    .max(1);
                    self.register_struct_coll(handle, &key, kind, ew, string_key);
                    let name = format!("{}#{}", handle, key);
                    let r = self
                        .render_p_var(&name)
                        .unwrap_or_else(|| "'{}".to_string());
                    parts.push(format!("{}:{}", md.name.name, r));
                    continue;
                }
                // §7.4: a member with fixed unpacked dimensions is a nested
                // list of its element cells (`<key>[i]`).
                let rendered = match self.member_dim_indices(&md.dimensions) {
                    Some(dims) if !md.dimensions.is_empty() => {
                        self.render_p_class_member_elems(handle, &key, &dims, &m.data_type)
                    }
                    _ => self.render_p_class_member_cell(handle, &key, &m.data_type),
                };
                parts.push(format!("{}:{}", md.name.name, rendered));
            }
        }
        format!("'{{{}}}", parts.join(", "))
    }

    /// The nested `'{...}` list of the elements of a fixed-array struct
    /// member stored as cells `<key>[i]...` of object `handle`.
    pub(super) fn render_p_class_member_elems(
        &mut self,
        handle: usize,
        key: &str,
        dims: &[Vec<i64>],
        dt: &DataType,
    ) -> String {
        let Some((idx, rest)) = dims.split_first() else {
            return self.render_p_class_member_cell(handle, key, dt);
        };
        let parts: Vec<String> = idx
            .iter()
            .map(|i| self.render_p_class_member_elems(handle, &format!("{}[{}]", key, i), rest, dt))
            .collect();
        format!("'{{{}}}", parts.join(", "))
    }

    /// One struct-member cell `key` of object `handle` for `%p`: a nested
    /// unpacked struct member-wise, a class handle as `null` until it is set
    /// (§8.4), anything else by its declared type.
    pub(super) fn render_p_class_member_cell(
        &mut self,
        handle: usize,
        key: &str,
        dt: &DataType,
    ) -> String {
        if let DataType::Struct(sub) = self.resolve_dt(dt)
            && Self::spreads_member_wise(&sub)
        {
            return self.render_p_class_prop_struct(handle, key, &sub);
        }
        let v = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .and_then(|i| i.properties.get(key))
            .cloned();
        if Self::dt_is_class_handle(dt, &self.module)
            && v.as_ref().is_none_or(|v| v.to_u64().unwrap_or(0) == 0)
        {
            return "null".into();
        }
        match v {
            Some(vv) => self.render_p_value_typed(&vv, dt),
            // §6.16/§6.8: an unwritten string member is the empty string.
            None if matches!(
                self.resolve_dt_ref(dt),
                DataType::Simple {
                    kind: crate::ast::types::SimpleType::String,
                    ..
                }
            ) =>
            {
                "\"\"".into()
            }
            // §6.8: an unwritten member of a 2-state type is 0.
            None => self.p_unwritten_leaf_of(Some(handle), key, dt),
        }
    }

    /// §21.2.1.7: `%p` of class property `prop` of object `handle` from its
    /// declared type — a fixed array (one or more dimensions) as a nested
    /// list of its `<handle>#prop[i]` elements, an unpacked-struct element or
    /// property member-wise from its instance cells. `None` when `prop` is not
    /// such a property of the object's class chain.
    pub(super) fn render_p_class_prop(&mut self, handle: usize, prop: &str) -> Option<String> {
        let cd = self.class_prop_declarer(handle, prop)?;
        if cd.static_properties.contains(prop) {
            // §8.9: a static fixed array has one store per declaring class.
            if cd.static_fixed_arrays.iter().any(|(n, ..)| n == prop) {
                return self.render_p_static_fixed(&cd.name, prop);
            }
            // §8.9: a static unpacked struct renders member-wise from its
            // shared holder (`handle` is the holder `class_prop_receiver`
            // gave).
            if !self.static_struct_names.is_empty()
                && self.static_struct_names.contains(prop)
                && let Some(su) = self
                    .class_prop_struct(handle, prop)
                    .filter(Self::spreads_member_wise)
            {
                return Some(self.render_p_class_prop_struct(handle, prop, &su));
            }
            return None;
        }
        let dt = self
            .class_member_coll_type(&format!("{}#{}", handle, prop))?
            .clone();
        if cd.array_of_coll_properties.contains_key(prop) {
            // §7.4.5: each element of the fixed outer shape is a collection.
            let scoped = format!("{}#{}", handle, prop);
            let (shape, _, _) = cd.array_of_coll_properties.get(prop)?.clone();
            return Some(self.render_p_coll_elems(&scoped, &shape));
        }
        let fixed =
            cd.array_properties.contains_key(prop) || cd.array_nd_properties.contains_key(prop);
        let su = match self.resolve_dt_ref(&dt) {
            DataType::Struct(su) if Self::spreads_member_wise(su) => Some(su.clone()),
            _ => None,
        };
        if !fixed {
            // A scalar unpacked struct; other scalars and collections take
            // the existing paths.
            if cd.queue_properties.contains_key(prop) || cd.assoc_properties.contains_key(prop) {
                return None;
            }
            return su.map(|su| self.render_p_class_prop_struct(handle, prop, &su));
        }
        let scoped = format!("{}#{}", handle, prop);
        let dims: Vec<(i64, i64)> = if let Some(&(lo, hi, _)) = self.module.arrays.get(&scoped) {
            vec![(lo, hi)]
        } else if let Some(&(d1, d2, _)) = self.module.arrays_2d.get(&scoped) {
            vec![d1, d2]
        } else if let Some((sh, _)) = self.module.arrays_nd.get(&scoped) {
            sh.clone()
        } else {
            return None;
        };
        if dims.iter().any(|&(lo, hi)| hi < lo || hi - lo >= 4096) {
            return None;
        }
        // §21.2.1.7: each dimension lists its elements left bound first.
        let dims = match self.module.unpacked_decl_dims.get(&scoped) {
            Some(dd) if dd.len() == dims.len() => dd.clone(),
            _ => dims,
        };
        Some(self.render_p_class_elems(handle, prop, "", &dims, &dt, su.as_ref()))
    }

    /// §8.9/§21.2.1.7: `%p` of the static fixed array `prop` reached from
    /// class `cls`: its declaring class's store, an array of unpacked structs
    /// member-wise from the `key[i].<m>` cells. None when no class of the
    /// chain declares `prop` a static fixed array.
    pub(super) fn render_p_static_fixed(&mut self, cls: &str, prop: &str) -> Option<String> {
        let key = self.static_fixed_key_in(cls, prop)?;
        let decl = key.strip_suffix(prop)?.strip_suffix("::")?;
        let cd = self.module.classes.get(decl)?.clone();
        let &(_, lo, hi, _) = cd.static_fixed_arrays.iter().find(|(n, ..)| n == prop)?;
        if let Some(dt) = self.class_prop_decl_type(&cd, prop).cloned()
            && matches!(self.resolve_dt(&dt), DataType::Struct(su) if Self::spreads_member_wise(&su))
            && hi >= lo
            && hi - lo < 4096
        {
            return Some(self.render_p_dims(&key, &[(lo, hi)], &dt));
        }
        self.render_p_var(&key)
    }

    /// `%p` of an element of a fixed-array class property of unpacked structs
    /// (`c.sta[1]`, `sta[i][j]` in a method): its member cells live in the
    /// instance map under `sta[1].<member>`.
    pub(super) fn render_p_class_struct_elem(&mut self, arg: &Expression) -> Option<String> {
        let mut idx: Vec<&Expression> = Vec::new();
        let mut base = arg;
        while let ExprKind::Index { expr, index } = &base.kind {
            idx.push(index);
            base = expr;
        }
        if idx.is_empty() {
            return None;
        }
        idx.reverse();
        let (handle, prop) = self.class_prop_receiver(base)?;
        let cd = self.class_prop_declarer(handle, &prop)?;
        if cd.static_properties.contains(&prop)
            || !(cd.array_properties.contains_key(&prop)
                || cd.array_nd_properties.contains_key(&prop))
        {
            return None;
        }
        let ndims = cd
            .array_nd_properties
            .get(&prop)
            .map(|(sh, _)| sh.len())
            .unwrap_or(1);
        if idx.len() != ndims {
            return None;
        }
        let su = self
            .class_prop_struct(handle, &prop)
            .filter(Self::spreads_member_wise)?;
        let mut key = prop;
        for e in idx {
            let i = self.eval_expr(e).to_i64()?;
            key.push_str(&format!("[{}]", i));
        }
        Some(self.render_p_class_prop_struct(handle, &key, &su))
    }

    /// The class on `handle`'s `extends` chain (leaf first) that declares
    /// property `prop`.
    pub(super) fn class_prop_declarer(
        &self,
        handle: usize,
        prop: &str,
    ) -> Option<Arc<crate::compiler::elaborate::ElaboratedClass>> {
        let mut cur = Some(self.heap.get(handle)?.as_ref()?.class_name.as_str());
        let mut hops = 0;
        while let Some(c) = cur {
            let cd = self.module.classes.get(c)?;
            if cd.property_types.contains_key(prop) || cd.properties.contains_key(prop) {
                return Some(cd.clone());
            }
            hops += 1;
            if hops > 64 {
                break;
            }
            cur = cd.extends.as_deref();
        }
        None
    }

    /// `%p` of a fixed outer shape whose elements are collections.
    pub(super) fn render_p_coll_elems(&mut self, base: &str, dims: &[(i64, i64)]) -> String {
        let Some((&(lo, hi), rest)) = dims.split_first() else {
            return self.render_p_var(base).unwrap_or_else(|| "'{}".to_string());
        };
        let parts: Vec<String> = (lo..=hi)
            .map(|i| self.render_p_coll_elems(&format!("{}[{}]", base, i), rest))
            .collect();
        format!("'{{{}}}", parts.join(", "))
    }

    /// §21.2.1.7: one member of a whole-object `%p` (`'{name:value, ...}`):
    /// arrays, structs and collections from their declared type, an enum by
    /// its label, a null handle as `null`.
    pub(super) fn render_p_object_member(
        &mut self,
        handle: usize,
        prop: &str,
        v: Option<&Value>,
        is_str: bool,
    ) -> Option<String> {
        if let Some(s) = self.render_p_class_prop(handle, prop) {
            return Some(s);
        }
        let cd = self.class_prop_declarer(handle, prop)?;
        if cd.queue_properties.contains_key(prop) || cd.assoc_properties.contains_key(prop) {
            return self.render_p_var(&format!("{}#{}", handle, prop));
        }
        let v = v?;
        if is_str {
            return Some(Self::render_p_value(v, true));
        }
        let is_handle = cd
            .properties
            .get(prop)
            .and_then(|sig| sig.type_name.as_deref())
            .is_some_and(|tn| self.module.classes.contains_key(tn));
        if is_handle && v.to_u64() == Some(0) {
            return Some("null".to_string());
        }
        match cd.property_types.get(prop) {
            Some(dt) if !is_handle && !v.is_real => {
                let dt = self.class_prop_elem_type(&cd, prop).unwrap_or(dt).clone();
                Some(self.render_p_value_typed(v, &dt))
            }
            _ => Some(Self::render_p_value(v, false)),
        }
    }

    pub(super) fn render_p_class_elems(
        &mut self,
        handle: usize,
        prop: &str,
        sfx: &str,
        dims: &[(i64, i64)],
        dt: &DataType,
        su: Option<&crate::ast::types::StructUnionType>,
    ) -> String {
        let Some((&(lo, hi), rest)) = dims.split_first() else {
            return match su {
                Some(su) => {
                    self.render_p_class_prop_struct(handle, &format!("{}{}", prop, sfx), su)
                }
                None => {
                    let v = self
                        .get_signal_value_by_name(&format!("{}#{}{}", handle, prop, sfx))
                        .unwrap_or_else(|| Value::new(1));
                    self.render_p_value_typed(&v, dt)
                }
            };
        };
        // `(left, right)`: a descending dimension lists `left` down.
        let idx: Vec<i64> = if lo <= hi {
            (lo..=hi).collect()
        } else {
            (hi..=lo).rev().collect()
        };
        let parts: Vec<String> = idx
            .into_iter()
            .map(|i| {
                self.render_p_class_elems(handle, prop, &format!("{}[{}]", sfx, i), rest, dt, su)
            })
            .collect();
        format!("'{{{}}}", parts.join(", "))
    }

    pub(super) fn render_p_value(v: &Value, is_string: bool) -> String {
        if is_string {
            format!("\"{}\"", v.to_sv_string())
        } else if v.is_real {
            format!("{}", v.to_f64())
        } else {
            v.to_dec_string()
        }
    }

    /// `(property, value, is_string)` of a class object, in DECLARATION order
    /// with base classes first — what `%p` prints for a class handle (§21.2.1.7).
    pub(super) fn class_object_members(
        &self,
        handle: usize,
    ) -> Option<Vec<(String, Option<Value>, bool)>> {
        let inst = self.heap.get(handle)?.as_ref()?;
        // Walk the inheritance chain, then emit base-first.
        let mut chain: Vec<String> = Vec::new();
        let mut cur = Some(inst.class_name.clone());
        while let Some(c) = cur {
            if chain.contains(&c) {
                break; // defensive: cyclic extends
            }
            cur = self.module.classes.get(&c).and_then(|k| k.extends.clone());
            chain.push(c);
        }
        chain.reverse();
        let mut out = Vec::new();
        let mut seen: HashSet<String> = HashSet::default();
        for c in &chain {
            if let Some(k) = self.module.classes.get(c) {
                for p in &k.property_order {
                    if seen.insert(p.clone()) {
                        let is_str = k.string_properties.contains(p);
                        if let Some(v) = inst.properties.get(p) {
                            out.push((p.clone(), Some(v.clone()), is_str));
                        } else if k.queue_properties.contains_key(p)
                            || k.assoc_properties.contains_key(p)
                            || k.array_of_coll_properties.contains_key(p)
                        {
                            // A collection member's storage is outside the
                            // instance map (`<handle>#<member>`).
                            out.push((p.clone(), None, is_str));
                        }
                    }
                }
            }
        }
        Some(out)
    }

    /// `(member_name, member_lvalue)` for a struct variable, in DECLARATION
    /// order — what `%p` must print (LRM §21.2.1.7). `None` for a non-struct
    /// target or one whose member order wasn't recorded.
    pub(super) fn struct_members_ordered(
        &self,
        a: &Expression,
    ) -> Option<Vec<(String, Expression)>> {
        let ExprKind::Ident(h) = &a.kind else {
            return None;
        };
        if h.path.len() != 1 {
            return None;
        }
        let vname = &h.path[0].name.name;
        let names = self.module.struct_members.get(vname)?;
        if names.is_empty() {
            return None;
        }
        let mut out = Vec::with_capacity(names.len());
        for fname in names {
            let mut fh = h.clone();
            fh.path.push(crate::ast::expr::HierPathSegment {
                name: crate::ast::Identifier {
                    name: fname.clone(),
                    span: h.span,
                },
                selects: Vec::new(),
            });
            fh.cached_signal_id = std::cell::Cell::new(None);
            fh.cached_resolved_name = std::cell::OnceCell::new();
            out.push((
                fname.clone(),
                Expression {
                    kind: ExprKind::Ident(fh),
                    span: a.span,
                    cached_width: std::cell::Cell::new(None),
                },
            ));
        }
        Some(out)
    }

    /// Expand a scope-randomize target that names a struct variable into its
    /// per-field targets `(resolved_name, field_lvalue)`; returns `None` for a
    /// non-struct target. A single flat assign can't reach an *unpacked*
    /// struct's members (no contiguous bit layout), and a *local* packed
    /// struct's whole/field views aren't aliased — so scope randomize operates
    /// per field. It also lets field-level inline constraints (`pkt.f < N`)
    /// match, since each field target resolves to the same name the constraint
    /// LHS does.
    /// §18.4: true when `a` names a variable whose type is a PACKED struct or
    /// (untagged) union — an integral type that can be constrained/randomized
    /// as one raw value, with its fields aliasing slices of that value.
    pub(super) fn is_packed_struct_var(&self, a: &Expression) -> bool {
        let ExprKind::Ident(h) = &a.kind else {
            return false;
        };
        if h.path.len() != 1 {
            return false;
        }
        let vname = &h.path[0].name.name;
        if self.module.arrays.contains_key(vname)
            || self.module.dynamic_arrays.contains(vname)
            || self.module.associative_arrays.contains_key(vname)
        {
            return false;
        }
        self.module
            .packed_struct_fields
            .get(vname)
            .is_some_and(|f| !f.is_empty())
    }

    pub(super) fn expand_struct_target(&self, a: &Expression) -> Option<Vec<(String, Expression)>> {
        let ExprKind::Ident(h) = &a.kind else {
            return None;
        };
        if h.path.len() != 1 {
            return None;
        }
        let vname = h.path[0].name.name.clone();
        // An ARRAY of structs is not a struct: `%p` must print it as an element
        // list, and scope-randomize handles it via the array path. Never treat
        // it as a single aggregate here.
        if self.module.arrays.contains_key(&vname)
            || self.module.dynamic_arrays.contains(&vname)
            || self.module.associative_arrays.contains_key(&vname)
        {
            return None;
        }
        // Field names: a packed struct is registered in `packed_struct_fields`;
        // an unpacked struct's members exist only as individual signals named
        // `<var>.<field>`, so discover those by a direct-child prefix scan.
        let field_names: Vec<String> =
            if let Some(fields) = self.module.packed_struct_fields.get(&vname) {
                if fields.is_empty() {
                    return None;
                }
                fields.iter().map(|(f, _, _)| f.clone()).collect()
            } else {
                let prefix = format!("{}.", vname);
                // A member that is an UNPACKED ARRAY exists as one signal per
                // element (`s.arr[0]` …), so its leaves legitimately contain
                // `[`. Excluding them made array members invisible to every
                // consumer of this function — most visibly struct ASSIGNMENT,
                // where `b = a;` copied the scalar members and left the array
                // ones x. Nested members (`inner.b`, `inner.b[0]`) still carry
                // a `.` and stay excluded; they are not direct children.
                let mut fs: Vec<String> = self
                    .signal_name_to_id
                    .names_with_prefix(&prefix)
                    .iter()
                    .filter_map(|k| k.strip_prefix(prefix.as_str()))
                    .filter(|rest| !rest.contains('.'))
                    .map(|rest| rest.to_string())
                    .collect();
                // Natural order, so `arr[2]` precedes `arr[10]` rather than
                // sorting lexicographically (only affects display order).
                fs.sort_by_key(|f| match f.split_once('[') {
                    Some((base, idx)) => (
                        base.to_string(),
                        idx.trim_end_matches(']').parse::<i64>().unwrap_or(0),
                    ),
                    None => (f.clone(), -1),
                });
                fs.dedup();
                if fs.is_empty() {
                    return None;
                }
                fs
            };
        let mut out = Vec::with_capacity(field_names.len());
        for fname in &field_names {
            let mut fh = h.clone();
            fh.path.push(crate::ast::expr::HierPathSegment {
                name: crate::ast::Identifier {
                    name: fname.clone(),
                    span: h.span,
                },
                selects: Vec::new(),
            });
            fh.cached_signal_id = std::cell::Cell::new(None);
            fh.cached_resolved_name = std::cell::OnceCell::new();
            let name = self.resolve_hier_name(&fh).into_owned();
            let fexpr = Expression {
                kind: ExprKind::Ident(fh),
                span: a.span,
                cached_width: std::cell::Cell::new(None),
            };
            out.push((name, fexpr));
        }
        Some(out)
    }

    /// §18.3: enum-member domain of the DECLARED type of `name` — a scalar
    /// variable, an array/queue name (whose `var_decl_types` entry is already
    /// the ELEMENT type) or a flattened struct-member path (`s.op`). Follows
    /// typedef alias chains so an aliased enum still resolves. `None` (or an
    /// empty domain, which callers must treat the same) for non-enum types.
    pub(super) fn declared_enum_members(&self, name: &str) -> Option<Vec<(String, u64)>> {
        if let Some(tn) = self.type_name_of_var(name) {
            if let Some(m) = self.module.enum_members.get(&tn) {
                return Some(m.clone());
            }
        }
        let mut cur = self.p_elem_type(name)?;
        for _ in 0..8 {
            let DataType::TypeReference { name: tn, .. } = &cur else {
                break;
            };
            let tn = tn.name.name.clone();
            if let Some(m) = self.module.enum_members.get(&tn) {
                return Some(m.clone());
            }
            match self.module.typedef_types.get(&tn) {
                Some(next) => cur = next.clone(),
                None => break,
            }
        }
        None
    }

    /// `std::randomize(v1, v2, ...)` — assign each listed lvalue a fresh random
    /// value sized to its inferred width. Struct targets are randomized member
    /// by member. Best-effort: inline `with` constraints are solved separately
    /// in `eval_randomize_with`. Always reports success.
    ///
    /// §18.3: every path here (scalar, struct field, array/queue element) must
    /// draw an ENUM-typed destination from its DECLARED member list only — a
    /// raw width-masked draw yields encodings outside the member set (an
    /// 8-member `enum bit [3:0]` came back 8..15 nearly half the time) and
    /// breaks `.name()` on the result. Constraints in a `with {}` clause still
    /// refine the value afterwards, exactly as for a plain integral target.
    pub(super) fn exec_std_randomize(&mut self, args: &[Expression]) -> Value {
        use rand::RngExt;
        for a in args {
            // IEEE 1800-2017 §18.12: an OBJECT handle argument
            // (`std::randomize(obj)`) randomizes the referenced object's
            // constraint model, exactly like `obj.randomize()` — it must NOT
            // be scalar-drawn into the pointer (that overwrites the object
            // handle with a random integer, corrupting the instance so
            // subsequent member/collection access silently resolves to a
            // different or invalid store). Only plain handle-typed lvalues
            // qualify; scalars whose value happens to look like a handle are
            // left to the scalar draw below.
            if let Some(h) = self.plain_handle_operand(a) {
                self.randomize_object_with(h, Self::plain_ident_name(a), &[], &[]);
                continue;
            }
            // Struct target: randomize each field individually. An enum FIELD
            // (`struct { op_e op; int x; }`) draws a declared member.
            if let Some(fields) = self.expand_struct_target(a) {
                for (name, fexpr) in &fields {
                    let w = self.infer_lhs_width(fexpr).max(1);
                    let members = self.declared_enum_members(name).filter(|m| !m.is_empty());
                    let rv = if let Some(ms) = members {
                        let i = self.cur_rng().random_range(0..ms.len());
                        Value::from_u64(ms[i].1, w)
                    } else if w <= 64 {
                        let mask = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
                        Value::from_u64(self.cur_rng().random::<u64>() & mask, w)
                    } else {
                        Value::zero(w)
                    };
                    self.assign_value(fexpr, &rv);
                }
                continue;
            }
            // Array/queue target: randomize each element (a scalar assign to the
            // array name would corrupt it). A `with { foreach … inside {…} }`
            // clause, if present, then refines each element via Foreach below.
            // An enum ELEMENT type (`op_e arr[4]`, `op_e q[$]`) draws declared
            // members — `var_decl_types[nm]` is already the element type.
            if let Some(nm) = self.array_operand_name(a) {
                let size = self.get_queue_size(&nm);
                let w = self
                    .module
                    .arrays
                    .get(&nm)
                    .map(|t| t.2)
                    .unwrap_or(32)
                    .max(1);
                let members = self.declared_enum_members(&nm).filter(|m| !m.is_empty());
                for i in 0..size {
                    let rv = if let Some(ms) = &members {
                        let k = self.cur_rng().random_range(0..ms.len());
                        Value::from_u64(ms[k].1, w)
                    } else {
                        let mask = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
                        Value::from_u64(self.cur_rng().random::<u64>() & mask, w)
                    };
                    self.set_signal_value_by_name(&format!("{}[{}]", nm, i), rv);
                }
                continue;
            }
            // Scalar enum-typed variable (§18.3, commit 18b8534) — now via
            // `declared_enum_members`, which also chases typedef aliases.
            if let ExprKind::Ident(h) = &a.kind {
                if h.path.last().map(|s| s.selects.is_empty()).unwrap_or(false) {
                    let bare = h.path.last().unwrap().name.name.clone();
                    let members = self.declared_enum_members(&bare).filter(|m| !m.is_empty());
                    if let Some(members) = members {
                        let w = self.infer_lhs_width(a).max(1);
                        let i = self.cur_rng().random_range(0..members.len());
                        let rv = Value::from_u64(members[i].1, w);
                        self.assign_value(a, &rv);
                        continue;
                    }
                }
            }
            // A multi-dimensional unpacked array (`u7_t e[5][2]`) is not in
            // `arrays` (1-D) and was never drawn at all; draw every element
            // by its flat name.
            if let ExprKind::Ident(h) = &a.kind {
                if h.path.iter().all(|s| s.selects.is_empty()) {
                    let nm = self.resolve_hier_name(h).into_owned();
                    let shape: Option<(Vec<(i64, i64)>, u32)> =
                        if let Some((sh, w)) = self.module.arrays_nd.get(&nm) {
                            Some((sh.clone(), *w))
                        } else if let Some(&(d1, d2, w)) = self.module.arrays_2d.get(&nm) {
                            Some((vec![d1, d2], w))
                        } else {
                            None
                        };
                    if let Some((shape, w)) = shape {
                        let w = w.max(1);
                        let mut suffixes: Vec<String> = vec![String::new()];
                        for &(lo, hi) in &shape {
                            let (lo, hi) = (lo.min(hi), lo.max(hi));
                            let mut next = Vec::new();
                            for sfx in &suffixes {
                                for k in lo..=hi {
                                    next.push(format!("{}[{}]", sfx, k));
                                }
                            }
                            suffixes = next;
                        }
                        for sfx in suffixes {
                            let rv = self.random_value_of_width(w);
                            self.set_signal_value_by_name(&format!("{}{}", nm, sfx), rv);
                        }
                        continue;
                    }
                }
            }
            let w = self.infer_lhs_width(a).max(1);
            // A packed target wider than 64 bits (`u7_t [4:0][1:0] d` is 70
            // bits) used to be "drawn" as all zeros; draw every bit.
            let rv = self.random_value_of_width(w);
            self.assign_value(a, &rv);
        }
        Value::from_u64(1, 32)
    }
}
