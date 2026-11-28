//! Signal ids, element ids and flattened names of assignment targets.

use super::*;

impl Simulator {
    /// Resolve a fully-qualified signal name (possibly an array element
    /// like `"tb.foo.mem[5]"`) to a signal_id. Tries the dense
    /// `signal_name_to_id` HashMap first; if that misses and the name has
    /// the form `<base>[<idx>]`, parses the suffix and uses
    /// `array_first_id` to compute the id without ever materialising
    /// per-element HashMap entries. Returns None if neither path resolves.
    #[inline]
    pub(in crate::compiler::simulator) fn resolve_signal_id(&self, name: &str) -> Option<usize> {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            return Some(id);
        }
        resolve_array_elem_id(name, &self.array_first_id)
            .or_else(|| resolve_nd_array_elem_id(name, &self.array_nd_first_id))
    }

    /// Look up flat signal_id for array element `name[idx]`. With the
    /// compact `array_first_id` resolver, 1D arrays compute the id directly
    /// as `first_id + (idx - lo)` — no per-element HashMap entry is needed.
    /// 2D/ND arrays still fall through to the legacy `array_elem_ids` cache
    /// path which iterates signal_name_to_id; those arrays are small in
    /// practice on real designs.
    pub(in crate::compiler::simulator) fn get_array_elem_id(
        &mut self,
        name: &str,
        idx: i64,
    ) -> Option<usize> {
        // Fast path: 1D array via compact resolver.
        if let Some(&(first_id, lo, hi)) = self.array_first_id.get(name) {
            if idx >= lo && idx <= hi {
                return Some(first_id + (idx - lo) as usize);
            }
            return None;
        }
        let (arr_lo, arr_hi) = match self.module.arrays.get(name) {
            Some(&(lo, hi, _)) => (lo, hi),
            None => return None,
        };
        if idx < arr_lo || idx > arr_hi {
            return None;
        }
        let offset = (idx - arr_lo) as usize;
        if !self.array_elem_ids.contains_key(name) {
            let size = (arr_hi - arr_lo + 1) as usize;
            let mut v = vec![-1i64; size];
            for i in 0..size {
                let elem_idx = arr_lo + i as i64;
                let elem_name = format!("{}[{}]", name, elem_idx);
                if let Some(&id) = self.signal_name_to_id.get(elem_name.as_str()) {
                    v[i] = id as i64;
                }
            }
            self.array_elem_ids.insert(name.to_string(), v);
        }
        let v = self.array_elem_ids.get(name).unwrap();
        let id = v[offset];
        if id < 0 { None } else { Some(id as usize) }
    }

    /// `format!("{}#{}::{}", class, sig, member)` (with a sig) or
    /// `format!("{}::{}", class, member)` without the formatting machinery.
    pub(in crate::compiler::simulator) fn spec_member_key(
        class: &str,
        sig: Option<&str>,
        member: &str,
    ) -> String {
        let mut out =
            String::with_capacity(class.len() + sig.map_or(0, |s| s.len() + 1) + member.len() + 2);
        out.push_str(class);
        if let Some(sig) = sig {
            out.push('#');
            out.push_str(sig);
        }
        out.push_str("::");
        out.push_str(member);
        out
    }

    /// `format!("{}.{}", hint, name)` under the current scope hint, if any,
    /// built without cloning the hint.
    pub(in crate::compiler::simulator) fn hint_scoped(&self, name: &str) -> Option<String> {
        let hint = self.name_resolve_hint.borrow();
        let h = hint.as_deref()?;
        let mut out = String::with_capacity(h.len() + 1 + name.len());
        out.push_str(h);
        out.push('.');
        out.push_str(name);
        Some(out)
    }

    pub(in crate::compiler::simulator) fn resolve_type_width_in_process_scope(
        &mut self,
        dt: &crate::ast::types::DataType,
    ) -> u32 {
        use crate::compiler::scoped_arrays;
        let mut view = HashMap::default();
        let mut widths = HashMap::default();
        for (query, arg) in scoped_arrays::query_operands(dt) {
            let Some(name) = scoped_arrays::operand_name(arg) else {
                continue;
            };
            let lexical = self.dyn_name_lookup(&name).map(str::to_string);
            let scope = self
                .name_resolve_hint
                .borrow()
                .clone()
                .or_else(|| self.process_scope_hint.get(&self.current_pid).cloned());
            let canonical = lexical.clone()
                .or_else(|| {
                    scope.as_ref().and_then(|scope| {
                        let key = format!("{scope}.{name}");
                        self.module.arrays.contains_key(&key).then_some(key)
                    })
                })
                .or_else(|| self.array_operand_name(arg))
                .or_else(|| self.module.arrays.contains_key(&name).then(|| name.clone()))
                .or_else(|| {
                    let key = name.replace('.', "::");
                    self.module.arrays.contains_key(&key).then_some(key)
                });
            if let Some((key, shape)) = canonical.and_then(|key| {
                self.module
                    .arrays
                    .get(&key)
                    .copied()
                    .map(|shape| (key, shape))
            }) {
                if query == "$bits" && !self.module.dynamic_arrays.contains(&key)
                    && !self.module.associative_arrays.contains_key(&key)
                    && let Ok(count) = u32::try_from(shape.0.abs_diff(shape.1) + 1)
                    && let Some(total) = shape.2.checked_mul(count)
                {
                    widths.insert(name.clone(), total);
                }
                view.insert(name, shape);
                view.insert(key, shape);
            } else if query == "$bits" {
                let width = if let Some(key) = lexical {
                    self.widths.get(&key).copied()
                        .or_else(|| self.get_signal_value_by_name(&key).map(|v| v.width))
                } else if let ExprKind::Ident(h) = &arg.kind {
                    self.frame_bound_bare_name(h)
                        .and_then(|key| self.widths.get(key).copied())
                        .or_else(|| {
                            self.local_stack
                                .iter()
                                .rev()
                                .find_map(|frame| frame.get(&name))
                                .map(|value| value.width)
                        })
                        .or_else(|| {
                            let stored = self.resolve_hier_name(h);
                            self.widths.get(stored.as_ref()).copied()
                                .or_else(|| self.get_signal_value_by_name(&stored).map(|v| v.width))
                        })
                } else {
                    None
                };
                if let Some(width) = width {
                    widths.insert(name, width);
                }
            }
        }
        scoped_arrays::resolve_type_width(
            dt,
            Some(&self.module.parameters),
            Some(&self.module.typedefs),
            &view,
            widths,
        )
    }

    /// `format!("{}{}", base, suffix)` without the formatting machinery.
    pub(in crate::compiler::simulator) fn name_with_suffix(base: &str, suffix: &str) -> String {
        let mut out = String::with_capacity(base.len() + suffix.len());
        out.push_str(base);
        out.push_str(suffix);
        out
    }

    /// `format!("{}[{}]", base, key)` for an already-rendered key.
    pub(in crate::compiler::simulator) fn name_with_key(base: &str, key: &str) -> String {
        let mut out = String::with_capacity(base.len() + key.len() + 2);
        out.push_str(base);
        out.push('[');
        out.push_str(key);
        out.push(']');
        out
    }

    /// Flatten an `Ident` / `Index` / `MemberAccess` chain into the dotted-and-
    /// indexed signal name the elaborator pre-registers for unpacked aggregates
    /// (`c.nodes[1].status`, `arr[0].tag`). `None` for shapes it can't flatten
    /// (non-constant index, ranges, calls, …).
    /// `<base>[<i>]` without the `core::fmt` machinery — this runs once per
    /// dimension per element access on procedural array loops, where it and
    /// its allocations were the single hottest thing in the profile.
    pub(in crate::compiler::simulator) fn name_with_index(base: &str, i: i64) -> String {
        let mut out = String::with_capacity(base.len() + 14);
        out.push_str(base);
        out.push('[');
        Self::push_i64(&mut out, i);
        out.push(']');
        out
    }

    pub(in crate::compiler::simulator) fn push_i64(out: &mut String, i: i64) {
        if i < 0 {
            out.push('-');
        }
        let mut n = i.unsigned_abs();
        let mut buf = [0u8; 20];
        let mut k = buf.len();
        loop {
            k -= 1;
            buf[k] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        out.push_str(std::str::from_utf8(&buf[k..]).unwrap_or("0"));
    }

    pub(in crate::compiler::simulator) fn flat_member_name(
        &mut self,
        e: &Expression,
    ) -> Option<String> {
        match &e.kind {
            ExprKind::Ident(h) => {
                if h.path.iter().any(|s| !s.selects.is_empty()) {
                    return None;
                }
                // Apply the local-dyn-array rename so callers (e.g. `%p`
                // queue formatting, indexed writes) hit the process-unique
                // storage key instead of the bare name. `resolve_hier_name`
                // already does this, but `flat_member_name` bypasses it.
                if h.path.len() == 1 {
                    if let Some(uq) = self.dyn_name_lookup(&h.path[0].name.name) {
                        return Some(uq.to_string());
                    }
                    // §8/§23.9: in a method of a class declared inside a
                    // module, the instance's own struct / variable.
                    if self.modclass_depth.get() > 0
                        && let Some(q) = self.modclass_item(&h.path[0].name.name)
                    {
                        return Some(q);
                    }
                    // §8.9/§8.25: inside a STATIC method of a parameterized
                    // class, a bare static collection names this
                    // specialization's store (`C#int::q`), the one its
                    // builtin methods and `C#(int)::q[i].m` use, so an
                    // element write `q[k] = s` spreads its members there.
                    if let Some(k) = self.static_method_coll_key(&h.path[0].name.name) {
                        return Some(k);
                    }
                }
                // §26.3: strip a PACKAGE prefix — `P::s` flattens to the bare
                // storage name `s` (see resolve_hier_name), so member paths
                // like `P::s.x` become `s.x`.
                let skip = usize::from(
                    h.path.len() >= 2
                        && self.module.packages.contains(&h.path[0].name.name)
                        && !self
                            .signal_name_to_id
                            .contains_key(h.path[0].name.name.as_str())
                        && !self.signals.contains_key(&h.path[0].name.name),
                );
                // §23.6: `$root.top.x` is the absolute `top.x`, which the
                // top module's own storage spells `x`.
                let skip = if h.path.len() >= 3 && h.path[0].name.name == "$root" {
                    1 + usize::from(h.path[1].name.name == self.module.name)
                } else {
                    skip
                };
                if skip == 1
                    && h.path.len() >= 2
                    && let Some(mut key) =
                        self.pkg_qual_collection_key(&h.path[0].name.name, &h.path[1].name.name)
                {
                    for seg in &h.path[2..] {
                        key.push('.');
                        key.push_str(&seg.name.name);
                    }
                    return Some(key);
                }
                let segs = &h.path[skip..];
                // The overwhelmingly common shape is a single segment; the
                // Vec + join below allocates twice for it. This is the
                // innermost call of every nested-index name build, so it runs
                // once per element access in a procedural array loop.
                if segs.len() == 1 {
                    return Some(segs[0].name.name.clone());
                }
                Some(
                    segs.iter()
                        .map(|s| s.name.name.as_str())
                        .collect::<Vec<_>>()
                        .join("."),
                )
            }
            ExprKind::Index { expr, index } => {
                let base = match &expr.kind {
                    // `this.q[i]`: name the receiver like `obj.q[i]`; the
                    // storage resolvers map `this.q` through the handle.
                    ExprKind::MemberAccess { expr: recv, member }
                        if matches!(recv.kind, ExprKind::This)
                            && self.this_stack.last().copied().flatten().is_some() =>
                    {
                        format!("this.{}", member.name)
                    }
                    _ => match self.class_scoped_elem_store(expr) {
                        Some(k) => k,
                        None => self.flat_member_name(expr)?,
                    },
                };
                // §7.10.1: `$` names the last element of the queue indexed.
                let iv = if Self::expr_contains_dollar(index) {
                    self.eval_index_of_queue(&base, index)
                } else {
                    self.eval_expr(index)
                };
                // §8.10: inside a method a bare property name is the
                // object's store, keyed in ITS format, even when a package
                // variable of the same name is registered under the bare name.
                // §7.8: an ASSOC element key must use the canonical store
                // format — a STRING key rendered via to_i64 collapsed to its
                // byte value ("sa[k]" became "sa[107]"), so writes and the
                // registered read paths disagreed on the element's name.
                if self.is_associative_array(&base) {
                    if self.this_stack.last().copied().flatten().is_some()
                        && let Some(st) = self.instance_assoc_member(&base)
                        && self.is_associative_array(&st)
                    {
                        let key = self.assoc_key_str(&st, &iv);
                        return Some(format!("{}[{}]", base, key));
                    }
                    let key = self.assoc_key_str(&base, &iv);
                    let mut out = String::with_capacity(base.len() + key.len() + 2);
                    out.push_str(&base);
                    out.push('[');
                    out.push_str(&key);
                    out.push(']');
                    return Some(out);
                }
                // A CLASS-MEMBER associative array is registered only under
                // its instance-scoped name (`<h>#aa`); key it by that store's
                // format too, or `aa["k"]` in a method names element `aa[107]`
                // while every read keys it `aa[k]`.
                if !self.no_class_objects()
                    && !self.module.arrays.contains_key(&base)
                    && !self.module.dynamic_arrays.contains(&base)
                {
                    let st = self.resolve_locator_storage(&base);
                    if st != base && self.is_associative_array(&st) {
                        let key = self.assoc_key_str(&st, &iv);
                        return Some(format!("{}[{}]", base, key));
                    }
                }
                let i = iv.to_index()?;
                // `format!` drags in the whole core::fmt machinery, which the
                // profile showed dominating this path. A nested index builds
                // one of these per dimension, per access.
                Some(Self::name_with_index(&base, i))
            }
            // §8.25: `C#(args)::m` names the static property `m` of THAT
            // specialization, whose store is keyed per specialization
            // (`C#args::m`, the key a static method of the class uses), so
            // `C#(int)::q[0].f` reaches the element members the class wrote.
            // A static FIXED array keeps one store per declaring class
            // (`C::m`), the one its methods use.
            ExprKind::MemberAccess { expr, member }
                if matches!(expr.kind, ExprKind::Specialization { .. }) =>
            {
                let spec = self
                    .resolve_call_spec_params(Self::extract_call_spec(expr), &self.current_spec)?;
                self.ensure_spec_statics(&spec.0, &spec.1);
                if let Some(k) = self.static_fixed_key_in_spec(&spec.0, &member.name, Some(&spec))
                {
                    return Some(k);
                }
                let saved = self.current_spec.replace(spec.clone());
                let key = self.static_prop_key(&spec.0, &member.name);
                self.current_spec = saved;
                key
            }
            ExprKind::MemberAccess { expr, member } => {
                let mut base = self.flat_member_name(expr)?;
                base.reserve(member.name.len() + 1);
                base.push('.');
                base.push_str(&member.name);
                Some(base)
            }
            _ => None,
        }
    }
}
