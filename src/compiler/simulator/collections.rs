//! Simulator collections.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

/// Values and shape owned by one scope-randomize target before a trial.
struct StdArraySnapshot {
    name: String,
    size: Option<u64>,
    elems: Vec<(i64, Value)>,
}

impl Simulator {
    /// Declared element width of an associative array, when the elaborator
    /// recorded one.
    ///
    /// Deliberately conservative: the map is keyed by DECLARATION name, and
    /// guessing wrong silently corrupts data (a too-narrow width truncates a
    /// good value), whereas guessing nothing merely restores the older
    /// store-as-is behaviour. So a CLASS-member collection (`<handle>#m`) is
    /// never resolved through its bare leaf — a class's `logic [7:0] m[string]`
    /// would otherwise pick up an unrelated module-scope `logic [3:0] m[string]`
    /// and truncate to 4 bits. Only an exact match, or an instance-scoped
    /// `inst.name` whose leaf is the declaration, is trusted.
    /// Is `name` (a collection, or one of its elements) string-typed? Used to
    /// keep §6.16's "no declared length" rule for elements as well as scalars.
    pub(super) fn is_string_collection(&self, name: &str) -> bool {
        let leaf = name.rsplit('.').next().unwrap_or(name);
        let base = leaf.split('[').next().unwrap_or(leaf);
        self.string_signals.contains(name)
            || self.string_signals.contains(leaf)
            || self.string_signals.contains(base)
    }

    /// The declared INTEGRAL element type of an associative array, from its
    /// declaration: a module / subroutine-local array's `var_decl_types`
    /// entry, or the class property for `<handle>#<member>`. `None` for an
    /// unknown or non-integral element type (string, real, class handle,
    /// aggregate).
    pub(super) fn assoc_elem_decl_type(&self, name: &str) -> Option<&DataType> {
        // A class collection is `<handle>#<member>`; a renamed subroutine
        // local (`@h#3`) also carries a `#` but is keyed like a module array.
        let class_member = name
            .split_once('#')
            .and_then(|(h, m)| h.parse::<usize>().ok().map(|h| (h, m)));
        let dt: &DataType = if let Some((handle, member)) = class_member {
            let mut cur = self.heap.get(handle)?.as_ref()?.class_name.as_str();
            // An element of a multi-dimensional member (`<h>#m[1]`).
            let member = member.split('[').next().unwrap_or(member);
            let mut found = None;
            for _ in 0..32 {
                let cd = self.module.classes.get(cur)?;
                if let Some(dt) = cd.property_types.get(member) {
                    found = Some(dt);
                    break;
                }
                cur = cd.extends.as_deref()?;
            }
            found?
        } else {
            let leaf = name.rsplit('.').next().unwrap_or(name);
            self.module
                .var_decl_types
                .get(name)
                .or_else(|| self.module.var_decl_types.get(leaf))
                // §7.4.5: an element array of an array of associative arrays
                // (`v[0]` of `int v[2][string][string]`) has the root's type.
                .or_else(|| {
                    let root = name.split('[').next().unwrap_or(name);
                    (root.len() < name.len() && !root.contains('.'))
                        .then(|| self.module.var_decl_types.get(root))
                        .flatten()
                })?
        };
        // §7.2.1: a packed struct or union is one integral value too.
        matches!(
            self.resolve_dt_ref(dt),
            DataType::IntegerVector { .. } | DataType::IntegerAtom { .. } | DataType::Enum(_)
        )
        .then_some(dt)
        .or_else(|| {
            matches!(self.resolve_dt_ref(dt), DataType::Struct(su) if su.packed).then_some(dt)
        })
    }

    /// Declared signedness of an associative array's integral elements.
    pub(super) fn assoc_elem_signed(&self, name: &str) -> Option<bool> {
        self.assoc_elem_decl_type(name)
            .map(|dt| super::elaborate::is_type_signed_resolved(dt, &self.module.typedef_types))
    }

    /// §7.8.6: the value a read of a NONEXISTENT associative-array element
    /// yields — the element type's default, x for a 4-state integral type and
    /// 0 for a 2-state one, at the element's width and signedness. `None`
    /// when the element type is unknown or not integral; the caller keeps its
    /// own fallback.
    pub(super) fn assoc_missing_elem_value(&self, name: &str) -> Option<Value> {
        use crate::ast::types::{IntegerAtomType as IAT, IntegerVectorType as IVT};
        let dt = self.assoc_elem_decl_type(name)?;
        let four_state = match self.resolve_dt_ref(dt) {
            DataType::IntegerVector { kind, .. } => !matches!(kind, IVT::Bit),
            DataType::IntegerAtom { kind, .. } => matches!(kind, IAT::Integer | IAT::Time),
            r => !super::elaborate::is_type_two_state_resolved(r, &self.module.typedef_types),
        };
        let w = self
            .assoc_elem_width(name)
            .or_else(|| self.widths.get(name).copied())
            .unwrap_or_else(|| {
                super::elaborate::resolve_type_width(
                    dt,
                    Some(&self.module.parameters),
                    Some(&self.module.typedefs),
                )
            })
            .max(1);
        let mut v = if four_state {
            Value::new(w)
        } else {
            Value::zero(w)
        };
        v.is_signed = super::elaborate::is_type_signed_resolved(dt, &self.module.typedef_types);
        Some(v)
    }

    /// §6.21/§10.7: an element of a FIXED unpacked array declared in a
    /// procedural block has no slot in the typed signal table; its declared
    /// width is recorded per element (`exec_stmt_var_decl`). It is assigned
    /// like any variable of the element type: fitted to that width, with the
    /// declared signedness and, for a 2-state type, x/z bits as 0. Stored at
    /// the RHS's own size, `logic [1:0] s[4] = '{0,1,2,3}` held 32-bit signed
    /// ints, so `s[2]` read -2 and `s[2] == 2` was false. Anything else (an
    /// associative collection, a string or real element) is returned
    /// unchanged. A formal or dynamic element can take its width from the
    /// array registration when no per-element width was installed.
    pub(super) fn fit_block_local_array_elem(
        &self,
        name: &str,
        elem_name: &str,
        val: Value,
    ) -> Value {
        if val.is_real || self.is_associative_array(name) || self.is_string_collection(name) {
            return val;
        }
        let width = self
            .widths
            .get(elem_name)
            .copied()
            .filter(|w| *w > 0)
            .or_else(|| {
                self.module
                    .dynamic_arrays
                    .contains(name)
                    .then(|| self.widths.get(name).copied().filter(|w| *w > 0))
                    .flatten()
            })
            .or_else(|| self.module.arrays.get(name).map(|shape| shape.2))
            .or_else(|| self.module.arrays_2d.get(name).map(|shape| shape.2))
            .or_else(|| self.module.arrays_nd.get(name).map(|shape| shape.1));
        let Some(w) = width else {
            return val;
        };
        let Some(dt) = self.module.var_decl_types.get(name) else {
            return val;
        };
        // Integral element types only: a class handle or struct element
        // keeps its own representation.
        if !matches!(
            super::elaborate::resolve_typedef_chain(dt, &self.module.typedef_types),
            crate::ast::types::DataType::IntegerVector { .. }
                | crate::ast::types::DataType::IntegerAtom { .. }
                | crate::ast::types::DataType::Enum(_)
        ) {
            return val;
        }
        let mut fitted = if val.width != w {
            val.resize_for_assign(w)
        } else {
            val
        };
        if super::elaborate::is_type_two_state_resolved(dt, &self.module.typedef_types) {
            fitted = fitted.to_two_state();
        }
        fitted.is_signed = self.type_is_signed_concrete(dt);
        fitted
    }

    pub(super) fn assoc_elem_width(&self, name: &str) -> Option<u32> {
        if let Some(&w) = self.module.assoc_elem_widths.get(name) {
            return Some(w);
        }
        // §7.8 / §7.13: a class collection element is stored under the scoped
        // `<handle>#<member>` name. The declared element width for an
        // associative array property isn't recorded in `assoc_elem_widths`
        // (that holds élaboration-scoped names), but the per-instance class
        // property's `Signal.width` IS the element type's width (array dims
        // aren't folded into it), so — resize to DECLARED width rather than
        // the runtime value's width.
        if let Some((hstr, member)) = name.split_once('#') {
            let handle: usize = match hstr.parse() {
                Ok(h) => h,
                // A renamed subroutine-local array (`@h#3`): its declaration
                // recorded the element width under that unique name.
                Err(_) => {
                    return self
                        .widths
                        .get(name)
                        .copied()
                        .filter(|_| self.is_associative_array(name));
                }
            };
            if let Some(w) = self.class_prop_width_of(handle, member) {
                return Some(w);
            }
            return None;
        }
        let leaf = name.rsplit('.').next().unwrap_or(name);
        self.module.assoc_elem_widths.get(leaf).copied()
    }

    /// §8.25: a per-specialization static collection key
    /// (`Class#spec::member`, e.g. `seqlib#int::m_typewide_sequences`)
    /// produced by `spec_static_coll_key` / the typed-qualifier rewrite. The
    /// BARE `member` may be registered in `module.dynamic_arrays`, but the
    /// per-spec key itself is not — and when a DERIVED class re-declares the
    /// same static (`seqlib_RST extends seqlib#(int); static … m_…;`), even the
    /// bare member may not sit in `module.dynamic_arrays`. Element reads
    /// (`Class#spec::m[i]`) and `foreach` must still treat the key as a
    /// queue/dynamic collection (the mirror of `is_associative_array`'s
    /// `#…::` branch). ASSOC collections are handled separately by
    /// `is_associative_array`, so this need only confirm the member is a
    /// static collection reached through a per-spec key.
    pub(super) fn is_per_spec_dynamic_static(&self, name: &str) -> bool {
        if let Some((head, sig_member)) = name.split_once('#')
            && let Some((_, member)) = sig_member.rsplit_once("::")
            && self.member_is_static_coll(head, member)
        {
            return true;
        }
        false
    }

    /// §7.9.1: `aa.size` / `aa.num` written without parentheses on an
    /// associative array counts its keys. The collection's `aa.size` signal
    /// is only a comb-dependency proxy for an associative array (it always
    /// holds 0), and the dotted-name lookups found it before any
    /// associative-array handling, so every paren-less `size` read 0.
    pub(super) fn assoc_size_member(&self, expr: &Expression) -> Option<Value> {
        let name = match &expr.kind {
            ExprKind::MemberAccess { expr: b, member }
                if matches!(member.name.as_str(), "size" | "num") =>
            {
                let ExprKind::Ident(h) = &b.kind else {
                    return None;
                };
                self.resolve_hier_name(h).into_owned()
            }
            ExprKind::Ident(h)
                if h.path.len() >= 2
                    && h.path.last().is_some_and(|s| {
                        s.selects.is_empty() && matches!(s.name.name.as_str(), "size" | "num")
                    }) =>
            {
                let full = self.resolve_hier_name(h);
                let base = full
                    .strip_suffix(".size")
                    .or_else(|| full.strip_suffix(".num"))?;
                base.to_string()
            }
            _ => return None,
        };
        let name = if self.is_associative_array(&name) {
            name
        } else {
            let scoped = self.instance_assoc_member(&name)?;
            if !self.is_associative_array(&scoped) {
                return None;
            }
            scoped
        };
        Some(Value::from_u64(
            self.assoc_top_level_keys(&name).len() as u64,
            32,
        ))
    }

    pub(super) fn is_associative_array(&self, name: &str) -> bool {
        if self.module.associative_arrays.contains_key(name) {
            return true;
        }
        // Every remaining branch needs a `#` (scoped member) or a trailing
        // `]` (nested element): one byte scan replaces up to three char
        // searches, a second map probe and a `format!` on the hot path.
        if !name.ends_with(']') && !name.as_bytes().contains(&b'#') {
            return false;
        }
        // parameterized assoc array read 0 / garbage (`first()` dropped the
        // whole store). Confirm the member after the last `::` is a
        // registered static assoc array of the class before the `#`.
        // NB: this is distinct from an INSTANCE member store `handle#member`
        // (numeric handle, no `::`), handled by the `#` branch above.
        if let Some((head, member)) = name.split_once('#')
            && let Some((_, member2)) = member.rsplit_once("::")
            && self.member_is_static_coll(head, member2)
            && self.module.associative_arrays.contains_key(member2)
        {
            return true;
        }
        // Per-instance class member: `<handle>#<member>`.
        if let Some(pos) = name.find('#') {
            if let Ok(handle) = name[..pos].parse::<usize>() {
                let member = &name[pos + 1..];
                if let Some(Some(inst)) = self
                    .heap
                    .get(handle)
                    .filter(|_| self.class_member_names().assoc_props.contains(member))
                {
                    // Borrowed chain walk: no String clone per ancestor.
                    let mut cur: Option<&str> = Some(inst.class_name.as_str());
                    while let Some(cn) = cur {
                        if let Some(cd) = self.module.classes.get(cn) {
                            if cd.assoc_properties.contains_key(member) {
                                return true;
                            }
                            cur = cd.extends.as_deref();
                        } else {
                            break;
                        }
                    }
                }
            }
        }
        // Nested associative-array ELEMENT (`base[K1][K2]...[Kn]` with n >= 1):
        // the receiver is the sub-array obtained by selecting a key from a
        // MULTIDIMENSIONAL associative array (IEEE 1800 §7.8.1) — e.g.
        // UVM's `m_recur_states[rhs]` where `m_recur_states[rhs].first(lhs)`
        // is called on the nested sub-array. The storage name is
        // `base[K1]...[Kn]`; strip trailing `[key]` segments to the root
        // declaration, then confirm a stored key exists under the compound
        // prefix (proving the selected element is itself a collection, not a
        // scalar). Without this the copier's cycle-detection `first()/exists()`
        // on a nested assoc silently returns "not found", so deep-clone of a
        // cyclic object graph recurses forever (stack overflow).
        if self.is_nested_assoc_elem(name) {
            return true;
        }
        false
    }

    /// Nested associative-array receiver: `name` is `base[K1]...[Kn]` (n >= 1),
    /// `base` is a registered associative array, and the selected element is
    /// ITSELF an associative array (a multidimensional assoc, IEEE 1800 §7.8.1)
    /// — i.e. at least one stored key exists under the compound prefix
    /// `name[..` (`g[1][2]` proves `g[1]` is a collection). This is exactly the
    /// shape UVM's copier uses (`m_recur_states[rhs][lhs][policy]`, then
    /// `m_recur_states[rhs].first(lhs)`), whose cycle guard the deep-clone of
    /// a CYCLIC object graph depends on. An empty nested sub-array can't be
    /// proven from a stored key, but `first`/`num`/`size` on an empty array
    /// return 0 anyway.
    ///
    /// §7.4.4: a declared collection answers from its declaration — the
    /// queue row `m[0]` of `int m[int][$]` is no associative array however
    /// many `m[0][i]` cells it holds (a pop leaves its vacated cell, #765).
    /// The stored-key proof remains for undeclared stores.
    pub(super) fn is_nested_assoc_elem(&self, name: &str) -> bool {
        matches!(self.declared_dim(name), Some(Dim::Assoc(_)))
    }

    /// §7.4.4: the kind of the collection stored as `name` — its declared
    /// dimension at the depth `name` selects to (`m[k]` of `int m[int][$]`
    /// is the queue `dims[1]`). `None` for an undeclared store, or a select
    /// past the unpacked dimensions (a bit select, §11.5.1).
    pub(super) fn declared_dim(&self, name: &str) -> Option<&Dim> {
        let (shape, depth) = self.coll_shape_at(name)?;
        let dim = shape.dims.get(depth)?;
        // §7.8.6 Table 7-1: a row past the size of a queue or dynamic array
        // above it does not exist, so has no kind (`v[1]` of a one-row
        // `int v[][string]` reads as an empty array).
        let mut row = name;
        for parent_dim in shape.dims[..depth].iter().rev() {
            let p = row.rfind('[')?;
            if matches!(parent_dim, Dim::Queue | Dim::Dyn) {
                let idx = row[p + 1..row.len() - 1].parse::<u64>().ok()?;
                if idx >= self.get_queue_size(&row[..p]) {
                    return None;
                }
            }
            row = &row[..p];
        }
        Some(dim)
    }

    /// The declared [`CollShape`] of the collection `name` belongs to, and
    /// how many selects `name` applies below its root: a root `m`, a row
    /// `m[k1]..[kn]`, an object's `<h>#m[..]`, a class static `C::m` /
    /// `C#spec::m`, a package `p::m`, or a local `@m#n`. Borrowed lookups
    /// only, one per level.
    pub(super) fn coll_shape_at(&self, name: &str) -> Option<(&CollShape, usize)> {
        let mut root = name;
        let mut depth = 0;
        loop {
            if let Some(shape) = self.root_coll_shape(root) {
                return Some((shape, depth));
            }
            if !root.ends_with(']') {
                return None;
            }
            root = &root[..root.rfind('[')?];
            depth += 1;
        }
    }

    /// The declared shape stored under the unselected name `root`.
    fn root_coll_shape(&self, root: &str) -> Option<&CollShape> {
        if let Some(shape) = self.module.coll_shapes.get(root) {
            return Some(shape);
        }
        if root.starts_with('@') {
            return self.local_coll_shapes.get(root);
        }
        // Most stores are plain names: one byte scan each rules out the
        // static/package and object-property forms below.
        let bytes = root.as_bytes();
        // A class static (`C::m`, `C#spec::m`) or a package item (`p::m`,
        // recorded under its bare name).
        if bytes.contains(&b':')
            && let Some((owner, member)) = root.rsplit_once("::")
        {
            let class = owner.split('#').next().unwrap_or(owner);
            return self
                .class_prop_shape(class, member)
                .or_else(|| self.module.coll_shapes.get(member));
        }
        // An object's property `<h>#m`.
        if !bytes.contains(&b'#') {
            return None;
        }
        let (h, member) = root.split_once('#')?;
        if !self.class_member_names().shaped_props.contains(member) {
            return None;
        }
        let handle = h.parse::<usize>().ok()?;
        let class = self.heap.get(handle)?.as_ref()?.class_name.as_str();
        self.class_prop_shape(class, member)
    }

    /// The shape of property `member` as declared by `class` or the nearest
    /// class it extends.
    /// Make the declared row `name` (`m[k1]..[kn]`) exist for a write — an
    /// assignment to it or a mutating method on it — creating each absent
    /// level of its path as its declared kind. The parent's kind decides
    /// whether an absent row may be created: an associative array allocates
    /// the entry (§7.8.7), a queue grows only at `$+1` (§7.10.1), and a
    /// dynamic or fixed array must already hold the index — writing past it
    /// is ignored (§7.4.5, §7.5.1). The keys come from `name`, so no index
    /// is evaluated again (§11.4.1). Returns false when the write must be
    /// ignored; true for an undeclared store, which the legacy paths own.
    pub(super) fn ensure_row(&mut self, name: &str) -> bool {
        let Some((shape, depth)) = self.coll_shape_at(name) else {
            return true;
        };
        if depth == 0 || depth > shape.dims.len() {
            return true;
        }
        let dims = shape.dims.clone();
        // The path's prefixes, outermost first: `m`, `m[k1]`, ..., `name`.
        let mut ends = Vec::with_capacity(depth + 1);
        let mut end = name.len();
        ends.push(end);
        for _ in 0..depth {
            end = match name[..end].rfind('[') {
                Some(p) => p,
                None => return true,
            };
            ends.push(end);
        }
        ends.reverse();
        let root = &name[..ends[0]];
        let w = self
            .module
            .assoc_elem_widths
            .get(root)
            .or_else(|| self.module.arrays.get(root).map(|t| &t.2))
            .copied()
            .unwrap_or(32);
        for level in 1..=depth {
            let (parent, row) = (&name[..ends[level - 1]], &name[..ends[level]]);
            let mut registered = self.module.dynamic_arrays.contains(row)
                || self.module.associative_arrays.contains_key(row);
            // A queue or dynamic parent holds the rows below its size, however
            // many it has registered (elaboration registers placeholders).
            match &dims[level - 1] {
                Dim::Queue | Dim::Dyn => {
                    let idx = row[parent.len() + 1..row.len() - 1].parse::<u64>();
                    let size = self.get_queue_size(parent);
                    match idx {
                        Ok(i) if i < size => {}
                        Ok(i) if i == size && dims[level - 1] == Dim::Queue => {
                            // A new row starts empty, whatever an earlier row
                            // at this index left behind.
                            self.set_queue_size(parent, size + 1);
                            registered = false;
                        }
                        _ => return false,
                    }
                }
                Dim::Assoc(_) | Dim::Fixed => {}
            }
            if registered {
                continue;
            }
            match dims.get(level) {
                Some(Dim::Queue | Dim::Dyn) => {
                    self.module.dynamic_arrays.insert(row.to_string());
                    self.module
                        .arrays
                        .entry(row.to_string())
                        .or_insert((0, 63, w));
                    if !registered
                        || !self
                            .signals
                            .contains_key(&Self::name_with_suffix(row, ".size"))
                    {
                        self.set_queue_size(row, 0);
                    }
                }
                Some(Dim::Assoc(k)) => {
                    let is_str = *k == KeyKind::String;
                    self.module
                        .associative_arrays
                        .insert(row.to_string(), is_str);
                }
                _ => {}
            }
        }
        true
    }

    fn class_prop_shape(&self, class: &str, member: &str) -> Option<&CollShape> {
        let mut cd = self.module.classes.get(class)?;
        for _ in 0..64 {
            if let Some(shape) = cd.prop_coll_shapes.get(member) {
                return Some(shape);
            }
            cd = self.module.classes.get(cd.extends.as_deref()?)?;
        }
        None
    }

    /// True if any stored signal name starts with `prefix`. Mirrors the prefix
    /// scanning that `assoc_top_level_keys` uses (checks both `signals` ids and
    /// the `name->id` index), so a nested assoc element (stored as
    /// `base[k1][k2]...`) is visible.
    pub(super) fn signal_name_prefix_present(&self, prefix: &str) -> bool {
        self.signals.any_key_with_prefix(prefix) || !self.assoc_static_keys(prefix).is_empty()
    }

    /// Resolve a `TypeName` (possibly a scoped/class-local typedef alias) to the
    /// concrete class it ultimately names, if any. Consults, in order: a direct
    /// class match; a scoped class typedef (`resource_types::rsrc_q_t` →
    /// `class resource_types`'s `typedef_targets["rsrc_q_t"]`); the
    /// module-level `typedef_types`; and an unscoped search of every class's
    /// `typedef_targets`. Used both to default a typedef'd handle to `null` and
    /// to let `rsrc_q_t rq = new()` construct the right class (`queue`).
    pub(super) fn resolve_typeref_class_name(
        &self,
        tref: &crate::ast::types::TypeName,
    ) -> Option<String> {
        // Memoized on (name, scope, class context): the miss path below
        // scans every class's typedef table, and the same type name is
        // resolved on every property access / call that carries it. The
        // class table only grows (lazy specializations), so an entry is
        // keyed to the table size it was computed against.
        let ctx_key: &str = match self.class_context_stack.last() {
            Some(Some(c)) => c.as_str(),
            _ => "",
        };
        let scope_key_owned = if tref.scopes.is_empty() {
            String::new()
        } else {
            tref.scope_prefix()
        };
        let scope_key: &str = &scope_key_owned;
        // A specialization's own class entry (`wrap_c<derived_c>`) is no
        // name any source can spell, so making one does not stale an entry.
        let ncls = self.module.classes.len() - self.spec_clone_origin.len();
        {
            let memo = self.typeref_class_memo.borrow();
            if let Some((n, hit)) = memo
                .get(tref.name.name.as_str())
                .and_then(|by_scope| by_scope.get(scope_key))
                .and_then(|by_ctx| by_ctx.get(ctx_key))
            {
                if *n == ncls {
                    return hit.clone();
                }
            }
        }
        let out = self.resolve_typeref_class_name_uncached(tref);
        self.typeref_class_memo
            .borrow_mut()
            .entry(tref.name.name.clone())
            .or_default()
            .entry(scope_key.to_string())
            .or_default()
            .insert(ctx_key.to_string(), (ncls, out.clone()));
        out
    }

    pub(super) fn resolve_typeref_class_name_uncached(
        &self,
        tref: &crate::ast::types::TypeName,
    ) -> Option<String> {
        use crate::ast::types::DataType;
        let base_class = |s: &str| -> Option<String> {
            if self.module.classes.contains_key(s) {
                return Some(s.to_string());
            }
            // `queue#(...)` → base `queue`
            let b = s.split('#').next().unwrap_or(s);
            if b != s && self.module.classes.contains_key(b) {
                return Some(b.to_string());
            }
            None
        };
        let nm = &tref.name.name;
        if let Some(c) = base_class(nm) {
            return Some(c);
        }
        // Find the typedef's target DataType.
        let mut target: Option<DataType> = None;
        for link in tref.scopes.iter().rev() {
            if let Some(cls) = self.module.classes.get(&link.name.name) {
                target = cls.typedef_targets.get(nm).cloned();
                if target.is_some() {
                    break;
                }
            }
        }
        // A bare class-local typedef referenced inside a method (e.g.
        // `this_type` inside `Wrapper::get()`) must resolve to the ENCLOSING
        // class's typedef. This check MUST precede the module-level
        // `typedef_types` lookup: class-local typedefs with common names
        // (e.g. `this_type`, declared in every parameterized class) leak
        // into the module-level table where the LAST class processed wins
        // arbitrarily — so `this_type` inside `Wrapper::get()` would resolve
        // to `Common::this_type` instead of `Wrapper::this_type`. Walk the
        // enclosing class context up the inheritance chain.
        if target.is_none() {
            if let Some(Some(ctx)) = self.class_context_stack.last().cloned() {
                let mut cur = Some(ctx);
                while let Some(cname) = cur {
                    if let Some(cls) = self.module.classes.get(&cname) {
                        if let Some(dt) = cls.typedef_targets.get(nm).cloned() {
                            target = Some(dt);
                            break;
                        }
                        cur = cls.extends.clone();
                    } else {
                        break;
                    }
                }
            }
        }
        if target.is_none() {
            target = self.module.typedef_types.get(nm).cloned();
        }
        if target.is_none() {
            // Unscoped: a class-local typedef referenced bare from a method of
            // the same/derived class with no class context (e.g. an
            // out-of-block declaration). Search every class's typedef_targets.
            for cls in self.module.classes.values() {
                if let Some(dt) = cls.typedef_targets.get(nm) {
                    target = Some(dt.clone());
                    break;
                }
            }
        }
        match target {
            Some(DataType::TypeReference { name, .. }) => {
                if name.name.name == *nm {
                    return None; // self-reference guard
                }
                self.resolve_typeref_class_name_uncached(&name)
            }
            _ => None,
        }
    }

    /// Like `resolve_typeref_class_name`, but also returns the parameterized
    /// type args from the typedef chain. A typedef like `table_q_t =
    /// shared#(rsrc_sv_q_t)` (possibly through multiple alias layers)
    /// resolves to `("shared", Some([rsrc_sv_q_t]))`. This lets a
    /// typedef'd local `table_q_t rq` carry the type args forward to `rq =
    /// new()` so the instance's type_bindings are populated — otherwise `T`
    /// stays unbound and collection builtins can't see a queue/array member.
    pub(super) fn resolve_typeref_class_with_type_args(
        &self,
        tref: &crate::ast::types::TypeName,
    ) -> Option<(String, Option<Vec<Expression>>)> {
        use crate::ast::types::DataType;
        let base_class = |s: &str| -> Option<String> {
            if self.module.classes.contains_key(s) {
                return Some(s.to_string());
            }
            let b = s.split('#').next().unwrap_or(s);
            if b != s && self.module.classes.contains_key(b) {
                return Some(b.to_string());
            }
            None
        };
        let nm = &tref.name.name;
        // Direct class match (possibly parameterized) — extract type_args.
        if let Some(c) = base_class(nm) {
            let ta = match self
                .module
                .typedef_types
                .get(nm)
                .or_else(|| self.module.typedef_types.get(nm))
            {
                Some(DataType::TypeReference { type_args, .. }) if !type_args.is_empty() => {
                    Some(type_args.clone())
                }
                _ => None,
            };
            return Some((c, ta));
        }
        // Walk the typedef chain, tracking type_args from the deepest layer
        // that has them. `table_q_t` → `rsrc_shared_q_t` → `shared#(...)`.
        let mut cur_name = nm.clone();
        let mut found_ta: Option<Vec<Expression>> = None;
        for _ in 0..16 {
            let target = self.lookup_typedef_target(&cur_name);
            let dt = match target {
                Some(dt) => dt,
                None => break,
            };
            match dt {
                DataType::TypeReference {
                    name: inner,
                    type_args,
                    ..
                } => {
                    if !type_args.is_empty() {
                        found_ta = Some(type_args.clone());
                    }
                    if let Some(c) = base_class(&inner.name.name) {
                        return Some((c, found_ta));
                    }
                    cur_name = inner.name.name.clone();
                }
                _ => break,
            }
        }
        None
    }

    /// §6.18 compile-time assignment type-check: assigning a non-class value
    /// (an enum member or integer literal) to a class-handle variable is
    /// illegal — only a class handle, `null`, `this` or `super` may be
    /// assigned. The reference elaborator rejects this at compile time;
    /// xezim runs the procedural body at time 0, so without
    /// this check a `// UVM TEST COMPILE-TIME FAILURE` test would silently
    /// compile. Record a compile error and stop before simulation.
    ///
    /// Deliberately conservative and side-effect free, so it can never alter
    /// normal simulation:
    ///   * only fires when the LHS is a bare identifier naming a class-handle
    ///     variable (a module signal whose declared type is a class, or a block
    ///     local declared with a class type) AND the RHS is a bare identifier
    ///     that is a user-declared enum member, or an integer literal;
    ///   * it reads only declared types and the enum-member tables, and uses
    ///     the RAW identifier leaf name — it must never call resolve_hier_name,
    ///     because prepopulating that shared per-AST-node cache with a
    ///     process-scope-free resolution poisons name resolution later at
    ///     simulation time (regression: struct-with-class-handle copy).
    pub(super) fn check_class_handle_assignment_type(&mut self) {
        use crate::ast::expr::ExprKind as EK;
        use crate::ast::stmt::StatementKind as SK;

        // Every declared enum member (all enum typedefs + all package enums).
        let mut enum_lits: std::collections::HashSet<String> = std::collections::HashSet::default();
        for members in self.module.enum_members.values() {
            for (nm, _) in members {
                enum_lits.insert(nm.clone());
            }
        }
        for members in self.module.package_enum_members.values() {
            for nm in members.keys() {
                enum_lits.insert(nm.clone());
            }
        }

        fn is_class_data_type(dt: &crate::ast::types::DataType, sim: &Simulator) -> bool {
            let mut cur = dt;
            let mut depth = 0;
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::default();
            while let crate::ast::types::DataType::TypeReference { name, .. } = cur {
                let nm = name.name.name.as_str();
                if !seen.insert(nm.to_string()) {
                    return false;
                }
                if sim.module.classes.contains_key(nm) {
                    return true;
                }
                match sim.module.typedef_types.get(nm) {
                    Some(nxt) => cur = nxt,
                    None => return sim.module.interfaces.contains(nm),
                }
                depth += 1;
                if depth > 8 {
                    return false;
                }
            }
            false
        }
        fn rhs_is_non_class(
            rv: &crate::ast::expr::Expression,
            el: &std::collections::HashSet<String>,
        ) -> bool {
            match &rv.kind {
                EK::Ident(rh) if rh.path.len() == 1 => {
                    let leaf = rh.path[0].name.name.as_str();
                    !matches!(leaf, "null" | "this" | "super") && el.contains(leaf)
                }
                EK::Number(_) => true,
                _ => false,
            }
        }
        fn walk(
            stmt: &crate::ast::stmt::Statement,
            sim: &Simulator,
            el: &std::collections::HashSet<String>,
            locals: &mut std::collections::HashSet<String>,
            errs: &mut Vec<String>,
        ) {
            match &stmt.kind {
                SK::VarDecl {
                    data_type,
                    declarators,
                    ..
                } => {
                    if is_class_data_type(data_type, sim) {
                        for d in declarators {
                            locals.insert(d.name.name.clone());
                        }
                    }
                }
                SK::BlockingAssign { lvalue, rvalue }
                | SK::NonblockingAssign { lvalue, rvalue, .. } => {
                    let EK::Ident(lhier) = &lvalue.kind else {
                        return;
                    };
                    // Raw leaf name only — see the method doc: resolve_hier_name
                    // must not run here. Block-locals and module signals are both
                    // keyed by their declared (leaf) name.
                    let name = lhier.path[0].name.name.clone();
                    let is_class = lhier.path.len() == 1
                        && lhier.path[0].selects.is_empty()
                        && (locals.contains(&name)
                            || sim
                                .module
                                .signals
                                .get(&name)
                                .filter(|s| {
                                    s.type_name
                                        .as_ref()
                                        .map(|t| sim.module.classes.contains_key(t))
                                        .unwrap_or(false)
                                })
                                .is_some());
                    if is_class && rhs_is_non_class(rvalue, el) {
                        let loc = sim
                            .span_file_line_in(
                                rvalue.span,
                                sim.module.src_file_of_module.get(&sim.module.name).copied(),
                            )
                            .unwrap_or_else(|| "<source>".to_string());
                        errs.push(format!(
                            "{}: illegal assignment of a non-class value to class handle '{}' (only a class handle or null is allowed)",
                            loc, name
                        ));
                    }
                }
                SK::If {
                    then_stmt,
                    else_stmt,
                    ..
                } => {
                    let mut in_ = locals.clone();
                    walk(then_stmt, sim, el, &mut in_, errs);
                    if let Some(e) = else_stmt {
                        walk(e, sim, el, &mut in_, errs);
                    }
                }
                SK::Case { items, .. } => {
                    for it in items {
                        let mut in_ = locals.clone();
                        walk(&it.stmt, sim, el, &mut in_, errs);
                    }
                }
                SK::SeqBlock { stmts, .. } | SK::ParBlock { stmts, .. } => {
                    let mut in_ = locals.clone();
                    for st in stmts {
                        walk(st, sim, el, &mut in_, errs);
                    }
                }
                SK::For { init, body, .. } => {
                    let mut in_ = locals.clone();
                    for i in init {
                        if let crate::ast::stmt::ForInit::VarDecl {
                            data_type, name, ..
                        } = i
                        {
                            if is_class_data_type(data_type, sim) {
                                in_.insert(name.name.clone());
                            }
                        }
                    }
                    walk(body, sim, el, &mut in_, errs);
                }
                SK::Foreach { body, .. }
                | SK::While { body, .. }
                | SK::DoWhile { body, .. }
                | SK::Repeat { body, .. }
                | SK::Forever { body, .. }
                | SK::TimingControl { stmt: body, .. }
                | SK::Wait { stmt: body, .. } => {
                    let mut in_ = locals.clone();
                    walk(body, sim, el, &mut in_, errs);
                }
                _ => {}
            }
        }

        let mut errs: Vec<String> = Vec::new();
        let mut locals: std::collections::HashSet<String> = std::collections::HashSet::default();
        for ib in &self.module.initial_blocks {
            walk(&ib.stmt, self, &enum_lits, &mut locals, &mut errs);
        }
        for ib in &self.module.static_init_blocks {
            walk(&ib.stmt, self, &enum_lits, &mut locals, &mut errs);
        }
        for ib in &self.module.program_initial_blocks {
            walk(&ib.stmt, self, &enum_lits, &mut locals, &mut errs);
        }
        for ab in &self.module.always_blocks {
            walk(&ab.stmt, self, &enum_lits, &mut locals, &mut errs);
        }
        for fb in &self.module.final_blocks {
            walk(&fb.stmt, self, &enum_lits, &mut locals, &mut errs);
        }
        if !errs.is_empty() {
            std::mem::swap(&mut self.compile_errors, &mut errs);
        }
    }

    /// True if a TypeReference data type names a class handle (directly, via a
    /// parameterized base, or through a typedef chain). Defaults such an
    /// uninitialized handle to `null` (LRM §8.4) instead of X.
    pub(super) fn type_ref_is_class_handle(&self, data_type: &crate::ast::types::DataType) -> bool {
        if let crate::ast::types::DataType::TypeReference { name, .. } = data_type {
            self.resolve_typeref_class_name(name).is_some()
        } else {
            false
        }
    }

    /// Context-aware lookup of a typedef name's target `DataType`, mirroring
    /// `resolve_typeref_class_name`'s resolution order: enclosing class
    /// context first (handles class-local typedefs like `this_type`, which
    /// leak into the module table where the last-elaborated class wins), then
    /// the module-level table, then any class-local typedef table.
    pub(super) fn lookup_typedef_target(&self, nm: &str) -> Option<crate::ast::types::DataType> {
        // Class/typedef tables are elaboration-static and the result depends
        // only on the lexical class context + name — memoize. The miss case
        // otherwise linear-scans every class's typedefs on EVERY call.
        let ctx_key: &str = match self.class_context_stack.last() {
            Some(Some(c)) => c.as_str(),
            _ => "",
        };
        if let Some(hit) = self
            .typedef_target_cache
            .borrow()
            .get(ctx_key)
            .and_then(|m| m.get(nm))
        {
            return hit.clone();
        }
        let out = self.lookup_typedef_target_uncached(nm);
        self.typedef_target_cache
            .borrow_mut()
            .entry(ctx_key.to_string())
            .or_default()
            .insert(nm.to_string(), out.clone());
        out
    }
    pub(super) fn lookup_typedef_target_uncached(
        &self,
        nm: &str,
    ) -> Option<crate::ast::types::DataType> {
        if let Some(Some(ctx)) = self.class_context_stack.last().cloned() {
            let mut cur = Some(ctx);
            while let Some(cname) = cur {
                if let Some(cls) = self.module.classes.get(&cname) {
                    if let Some(dt) = cls.typedef_targets.get(nm).cloned() {
                        return Some(dt);
                    }
                    cur = cls.extends.clone();
                } else {
                    break;
                }
            }
        }
        if let Some(dt) = self.module.typedef_types.get(nm).cloned() {
            return Some(dt);
        }
        for cls in self.module.classes.values() {
            if let Some(dt) = cls.typedef_targets.get(nm).cloned() {
                return Some(dt);
            }
        }
        None
    }

    /// Serialize a specialization argument `Expression` back into the textual
    /// fragment form consumed by `split_spec_args` + `eval_spec_arg_fragment`:
    /// string literals keep their quotes, integers yield their decimal text,
    /// and bare identifiers yield the name (type-param names, enum members,
    /// and typedef names like `this_type` pass through verbatim and are
    /// resolved downstream).
    ///
    /// Context-aware: a bare `Ident` arg is resolved against the ENCLOSING
    /// context so the sig carries CONCRETE values/types, never a bare
    /// value-parameter name. This is essential for class-local typedef
    /// specializations like `typedef Common#(this_type, Tname) common_type;`
    /// inside `Wrapper#(int,"my_type")`: serializing `Tname` verbatim would
    /// later trigger a mutual recursion (bare value-param eval →
    /// `resolve_value_param_from_spec` → re-eval the same name). Resolving
    /// `this_type`→`Wrapper` and `Tname`→`"my_type"` at build time avoids it.
    pub(super) fn expr_to_spec_fragment(&self, e: &Expression) -> Option<String> {
        use crate::ast::expr::{ExprKind, NumberLiteral};
        match &e.kind {
            // A data-declaration type arg that is not a bare name
            // (`P#(bit [7:0]) x;`) arrives as a TypeLiteral.
            ExprKind::TypeLiteral(dt) => crate::elaborate::data_type_to_spec_fragment(dt),
            ExprKind::StringLiteral(s) => Some(format!("\"{}\"", s)),
            // A named assignment keeps its name (`.W(5)`); the class's
            // `canonicalize_spec_sig` moves it to that parameter's slot.
            ExprKind::NamedArg {
                name,
                expr: Some(x),
            } => Some(format!(
                ".{}({})",
                name.name,
                self.expr_to_spec_fragment(x)?
            )),
            ExprKind::AssignmentPattern(_) => self.class_array_arg_fragment(e),
            ExprKind::Number(NumberLiteral::Integer { value, .. }) => Some(value.clone()),
            ExprKind::Number(NumberLiteral::Real(r)) => Some(format!("{}", r)),
            ExprKind::Number(NumberLiteral::UnbasedUnsized(c)) => Some(c.to_string()),
            ExprKind::Specialization {
                base,
                type_args_text,
            } => {
                let (b_name, b_sig) = self
                    .resolve_call_spec_params(Self::extract_call_spec(e), &self.current_spec)
                    .unwrap_or_else(|| {
                        let name = if let ExprKind::Ident(h) = &base.kind {
                            h.path
                                .last()
                                .map(|s| s.name.name.clone())
                                .unwrap_or_default()
                        } else {
                            String::new()
                        };
                        (name, type_args_text.clone())
                    });
                if !b_name.is_empty() {
                    let canon_sig = self.canonicalize_spec_sig(&b_name, &b_sig);
                    Some(format!("{}#({})", b_name, canon_sig))
                } else {
                    None
                }
            }
            // §8.25: a value argument matches by value. A constant expression
            // of literals and package parameters (`pkg::N + pkg::K`) is
            // keyed by the number it folds to, like the literal itself.
            ExprKind::Binary { .. }
            | ExprKind::Unary { .. }
            | ExprKind::Paren(_)
            | ExprKind::Conditional { .. }
            | ExprKind::MemberAccess { .. } => self.fold_package_const_expr(e),
            ExprKind::Ident(hier)
                if hier.path.len() == 2
                    && self
                        .package_param_key(&format!(
                            "{}::{}",
                            hier.path[0].name.name, hier.path[1].name.name
                        ))
                        .is_some_and(|k| self.module.parameters.contains_key(k)) =>
            {
                self.fold_package_const_expr(e)
            }
            ExprKind::Ident(hier) => {
                let nm = &hier.path.last()?.name.name;
                // Resolve a type/class alias (e.g. `this_type`) to the
                // concrete class name in the enclosing context.
                let synth = crate::ast::types::TypeName {
                    scopes: Vec::new(),
                    name: crate::ast::Identifier {
                        name: nm.clone(),
                        span: crate::ast::Span::dummy(),
                    },
                    span: crate::ast::Span::dummy(),
                };
                // Resolve a typedef alias to a PARAMETERIZED class with its
                // specialization args intact (`BaseType` → `base_class#(9)`),
                // not just the bare base (`resolve_typeref_class_name` drops
                // the `#(...)`). A `$cast` destination typed via such an
                // alias recorded the alias name in `var_type_args`; without
                // the args the type-param comparison compared `base_class`
                // against the instance's `base_class#(9)` binding and wrongly
                // rejected a cast to one's own registry specialization
                // (factory/registry parameterized-object tests).
                if let Some(cls) = self.resolve_typeref_class_name(&synth) {
                    if let Some((base, Some(ta))) =
                        self.resolve_typeref_class_with_type_args(&synth)
                    {
                        // Rebuild the class with its args if the rich
                        // resolution has them (canonicalize the sig).
                        if !ta.is_empty() {
                            let frags: Vec<String> = ta
                                .iter()
                                .filter_map(|e| self.expr_to_spec_fragment(e))
                                .collect();
                            if frags.len() == ta.len() {
                                let sig = self.canonicalize_spec_sig(&base, &frags.join(","));
                                if !sig.is_empty() {
                                    return Some(format!("{}#({})", base, sig));
                                }
                            }
                        }
                    }
                    return Some(cls);
                }
                // Resolve a value parameter (e.g. `Tname`) to its literal
                // value from the active specialization — NON-recursively
                // (only literal fragments, never a bare name).
                if let Some(v) = self.read_value_param_literal(nm) {
                    return Some(v);
                }
                // Resolve an ENCLOSING TYPE PARAMETER (e.g. `W#(T)` where
                // `T` is the surrounding class's type param, bound to a
                // concrete type) to its current binding. The specialization
                // sig must carry the CONCRETE type here — leaving the bare
                // param name lets `canonicalize_spec_sig` mistake it for
                // THIS class's own same-named parameter and clobber it with
                // that parameter's declared default, so a nested generic
                // member (`S#(T) s;` inside `W#(T)`) recorded `W#(int)`
                // instead of `W#(int_arr)` and every downstream type-param
                // resolution of `T` came back as the default, dropping a
                // `T`-typed queue formal. (A genuine class/typedef name
                // never reaches here — it resolved above — so this only
                // fires for type-parameter references.)
                if let Some(resolved) = self.resolve_type_param_binding(nm) {
                    if self.module.classes.contains_key(&resolved)
                        || self.module.typedef_types.contains_key(&resolved)
                    {
                        return Some(resolved);
                    }
                }
                // Otherwise keep the name verbatim (an enum member, resolved
                // downstream).
                Some(nm.clone())
            }
            _ => None,
        }
    }

    /// NON-recursive read of a value parameter's LITERAL value from the active
    /// specialization (`current_spec`). Unlike `resolve_value_param_from_spec`,
    /// this only returns when the fragment is a literal (string or number) —
    /// never a bare name — so it cannot trigger the
    /// eval-expr ↔ resolve_value_param_from_spec recursion. Used by
    /// `expr_to_spec_fragment` to bake concrete values into a typedef-spec sig.
    pub(super) fn read_value_param_literal(&self, name: &str) -> Option<String> {
        let (base, sig) = self.current_spec.as_ref()?;
        let cd = self.get_class_def(base)?;
        if cd.type_param_names.iter().any(|t| t == name) {
            return None;
        }
        let legacy_order: Vec<String>;
        let order: &[String] = if cd.param_order.is_empty()
            && !(cd.param_defaults.is_empty() && cd.type_param_names.is_empty())
        {
            legacy_order = cd
                .param_defaults
                .iter()
                .map(|(n, _)| n.clone())
                .chain(cd.type_param_names.iter().cloned())
                .collect();
            &legacy_order
        } else {
            &cd.param_order
        };
        let idx = order.iter().position(|p| p == name)?;
        let frags = Self::split_spec_args(sig);
        let frag = frags.get(idx)?;
        let t = frag.trim();
        if t.is_empty() {
            return None;
        }
        let bytes = t.as_bytes();
        let is_string = bytes.len() >= 2 && bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"';
        if is_string || Self::parse_spec_number(t).is_some() {
            Some(t.to_string())
        } else {
            None
        }
    }

    /// Resolve a typedef name that aliases a parameterized-class
    /// specialization into the `(base_class, sig)` pair used as a
    /// `current_spec`. For `typedef Common#(int,"alpha") AlphaT;` returns
    /// `Some(("Common", "int,\"alpha\""))`. Returns `None` unless the
    /// typedef is a `TypeReference` to a known parameterized class WITH a
    /// non-empty arg list (so plain class aliases and non-class typedefs keep
    /// their existing fallthrough behavior — this only intercepts genuine
    /// typedef specializations).
    /// Resolve a simple class typedef (NO specialization args) to its
    /// target class name: `typedef base alias_t;` ... `alias_t::method()`
    /// → `base`. Returns None for non-class typedefs, typedefs WITH
    /// specialization args (handled by `resolve_typedef_spec`), or names
    /// that aren't typedefs at all.
    ///
    /// This is the §6.18/§8.25.1 plain-alias case: a typedef inside a
    /// class body that names another class with no `#(...)` args. Callback
    /// infrastructure relies on this heavily — every
    /// `typed_callbacks#(T)` and `callbacks#(T,CB)` declares
    /// `typedef callbacks_base super_type;` and then calls
    /// `super_type::m_initialize()` to set up the shared static pool.
    /// Without this resolution, those calls silently no-op and the pool
    /// stays null.
    pub(super) fn resolve_simple_typedef_class(&self, name: &str) -> Option<String> {
        use crate::ast::types::DataType;
        let dt = self.lookup_typedef_target(name)?;
        if let DataType::TypeReference {
            name: tn,
            type_args,
            ..
        } = &dt
        {
            if !type_args.is_empty() {
                return None;
            }
            let synth = crate::ast::types::TypeName {
                scopes: Vec::new(),
                name: crate::ast::Identifier {
                    name: tn.name.name.clone(),
                    span: crate::ast::Span::dummy(),
                },
                span: crate::ast::Span::dummy(),
            };
            return self.resolve_typeref_class_name(&synth);
        }
        None
    }

    /// Resolve a TYPEDEF declared as a member of `class_name` to the target
    /// CLASS it names — the class-scoped analogue of
    /// `resolve_simple_typedef_class` (which is module-scoped). Used to
    /// dispatch `Class::typedef::static_method(...)`: the middle segment is a
    /// typedef alias (e.g. `typedef registry#(...) type_id;` or a plain
    /// `typedef Holder type_id;`) rather than a class, so the static-call
    /// dispatcher must follow the alias to the real class. Returns `None` for
    /// parameterized aliases (those go through `resolve_typedef_spec`) or
    /// non-class targets.
    pub(super) fn resolve_class_member_typedef_class(
        &self,
        class_name: &str,
        member: &str,
    ) -> Option<String> {
        use crate::ast::types::DataType;
        let cd = self.module.classes.get(class_name)?;
        let dt = cd.typedef_targets.get(member)?;
        if let DataType::TypeReference {
            name: tn,
            type_args,
            ..
        } = dt
        {
            if !type_args.is_empty() {
                return None;
            }
            let synth = crate::ast::types::TypeName {
                scopes: Vec::new(),
                name: crate::ast::Identifier {
                    name: tn.name.name.clone(),
                    span: crate::ast::Span::dummy(),
                },
                span: crate::ast::Span::dummy(),
            };
            return self.resolve_typeref_class_name(&synth);
        }
        None
    }

    /// Follow plain `typedef A B;` hops from `name` to the class they name.
    /// Stops early at the first alias in the chain that names a
    /// parameterized specialization (`typedef C#(8) A;`) and returns that
    /// alias, so the caller keeps its `#(...)` arguments.
    pub(super) fn resolve_plain_alias_chain(&self, name: &str) -> Option<String> {
        use crate::ast::types::DataType;
        let mut cur = name.to_string();
        for _ in 0..16 {
            let DataType::TypeReference {
                name: tn,
                type_args,
                ..
            } = self.lookup_typedef_target(&cur)?
            else {
                return None;
            };
            if !type_args.is_empty() {
                return (cur != name).then_some(cur);
            }
            let next = tn.name.name;
            if self.module.classes.contains_key(&next) {
                return Some(next);
            }
            if next == cur {
                return None;
            }
            cur = next;
        }
        None
    }

    pub(super) fn resolve_typedef_spec(&self, name: &str) -> Option<(String, String)> {
        let first = self.lookup_typedef_target(name)?;
        if let Some(spec) = self.spec_from_typedef_dt(&first) {
            return Some(spec);
        }
        let DataType::TypeReference {
            name: target,
            type_args,
            ..
        } = first
        else {
            return None;
        };
        if !type_args.is_empty() {
            return None;
        }
        let mut current = if target.scopes.is_empty() {
            target.name.name
        } else {
            target
                .scopes
                .iter()
                .map(|s| s.name.name.as_str())
                .chain(std::iter::once(target.name.name.as_str()))
                .collect::<Vec<_>>()
                .join("::")
        };
        if self.module.classes.contains_key(&current) {
            return None;
        }
        let mut visited = HashSet::default();
        visited.insert(name.to_string());
        for _ in 0..64 {
            if !visited.insert(current.clone()) {
                return None;
            }
            let dt = self.lookup_typedef_target(&current)?;
            if let Some(spec) = self.spec_from_typedef_dt(&dt) {
                return Some(spec);
            }
            let DataType::TypeReference {
                name, type_args, ..
            } = dt
            else {
                return None;
            };
            if !type_args.is_empty() {
                return None;
            }
            current = name
                .scopes
                .iter()
                .map(|s| s.name.name.as_str())
                .chain(std::iter::once(name.name.name.as_str()))
                .collect::<Vec<_>>()
                .join("::");
        }
        None
    }

    /// Class-scoped analogue of `resolve_typedef_spec`: resolve a TYPEDEF
    /// declared as a member of `class_name` (e.g.
    /// `typedef object_registry#(T,"N") type_id;` inside class `T`) to its
    /// specialization `(base, sig)`. Used to dispatch
    /// `Class::typedef::static_method(...)` when the typedef is parameterized —
    /// the middle segment names a typedef alias, not a class, so the static-call
    /// dispatcher must follow the alias to the real specialization.
    pub(super) fn resolve_class_member_typedef_spec(
        &self,
        class_name: &str,
        member: &str,
    ) -> Option<(String, String)> {
        let cd = self.module.classes.get(class_name)?;
        let dt = cd.typedef_targets.get(member)?;
        self.spec_from_typedef_dt(dt)
    }

    /// Build a specialization `(base_class, signature)` from a typedef's
    /// `DataType` (a `TypeReference` with type args). Shared by the module-
    /// scoped `resolve_typedef_spec` and the class-scoped
    /// `resolve_class_member_typedef_spec`. Returns `None` for non-parameterized
    /// typedefs or bases that don't resolve to a real class.
    pub(super) fn spec_from_typedef_dt(
        &self,
        dt: &crate::ast::types::DataType,
    ) -> Option<(String, String)> {
        use crate::ast::types::DataType;
        if let DataType::TypeReference {
            name: tn,
            type_args,
            ..
        } = dt
        {
            if type_args.is_empty() {
                return None;
            }
            let base = tn.name.name.clone();
            // The base must resolve to a real class (directly or after one
            // typedef hop, e.g. `this_type` -> `Wrapper`).
            let base = if self.module.classes.contains_key(&base) {
                base
            } else {
                let synth = crate::ast::types::TypeName {
                    scopes: Vec::new(),
                    name: crate::ast::Identifier {
                        name: base.clone(),
                        span: crate::ast::Span::dummy(),
                    },
                    span: crate::ast::Span::dummy(),
                };
                self.resolve_typeref_class_name(&synth)?
            };
            if !self.module.classes.contains_key(&base) {
                return None;
            }
            // Defer to the pre-supported framework machinery for framework
            // infrastructure classes (registry, callbacks, config_db,
            // typed_callbacks, ...). Their static methods (`get`,
            // `get_type`, `create`) are serviced by dedicated singleton /
            // factory / callback handling that builds correctly-wired
            // objects; dispatching them through the generic static-method
            // path constructs unwired instances and the later lookups
            // resolve to null ("get_type not implemented", "Null callback",
            // factory type-mismatch). User classes (the §8.25.1 / factory-
            // delegation pattern under test) are still dispatched here.
            // The `uvm_` prefix is the well-defined boundary between user
            // code and the pre-supported library classes.
            let frags: Vec<String> = type_args
                .iter()
                .filter_map(|e| {
                    // Check for `this_type` BEFORE expr_to_spec_fragment
                    // resolves it to just the base class name (without the
                    // specialization suffix).
                    if let ExprKind::Ident(h) = &e.kind {
                        if h.path.len() == 1 {
                            let nm = &h.path[0].name.name;
                            if nm == "this_type" || nm == "this" {
                                if let Some((b, s)) = &self.current_spec {
                                    return Some(format!("{}#({})", b, s));
                                }
                            }
                            // A type arg that is a TYPEDEF of a parameterized
                            // class (`cb_type cb_q[$]` where `typedef
                            // uvm_event_callback#(T) cb_type;`): expand it via
                            // `resolve_typedef_spec` so the base's own type
                            // args are carried (and their bare params resolved
                            // through `current_spec`). `expr_to_spec_fragment`
                            // on the raw typedef ident otherwise returns ONLY
                            // the base class name — dropping the `#(T)`, so
                            // the missing param is afterwards filled with its
                            // DECLARED DEFAULT (`uvm_object`) instead of the
                            // enclosing `T` (`string`). That made the string /
                            // `uvm_object` event callbacks key differently and
                            // the string event's `get_all` missed its callback.
                            if self.module.typedef_types.contains_key(nm.as_str())
                                || self.lookup_typedef_target(nm.as_str()).is_some()
                            {
                                if let Some((tb, ts)) = self.resolve_typedef_spec(&nm) {
                                    return Some(format!("{}#({})", tb, ts));
                                }
                            }
                        }
                    }
                    let frag = self.expr_to_spec_fragment(e)?;
                    // §8.25: a bare SELF-REFERENCE — the argument names the
                    // class whose specialization context we are resolving in
                    // (`uvm_object_registry#(C,"C") type_id`, written by
                    // `uvm_object_utils(C)`) — denotes the CURRENT
                    // specialization `C#(sig)`, not the default. Without
                    // this, a static method entered through a specialization
                    // (`C#(T2)::get_type()`, or `v.get_type()` on a null `v`
                    // declared `C#(T2)`) keyed the registry on the default
                    // `C`, so the factory created default-specialized objects
                    // and the `$cast` in `uvm_create_on` correctly rejected
                    // them.
                    //
                    // Only inside that class's own declaration (or a derived
                    // one): elsewhere the bare name IS the default
                    // specialization — a non-parameterized class whose typedef
                    // names `C` must not read `C#(sig)` just because its method
                    // happens to run under an active `C#(sig)` context.
                    if let Some((b, s)) = &self.current_spec {
                        if frag == *b && self.inside_class_decl_of(b) {
                            return Some(format!("{}#({})", b, s));
                        }
                    }
                    // Resolve bare type-param names (e.g. `T`, `this_type`)
                    // through the active specialization so the typedef's
                    // specialization carries concrete values.
                    if frag == "this_type" || frag == "this" {
                        if let Some((b, s)) = &self.current_spec {
                            return Some(format!("{}#({})", b, s));
                        }
                    }
                    let frag = self.substitute_spec_params(&frag);
                    // Check if frag is a type/value param of the spec base
                    if let Some((b, s)) = &self.current_spec {
                        if let Some(cd) = self.get_class_def(b) {
                            if cd.type_param_names.iter().any(|t| t == &frag)
                                || cd.param_order.iter().any(|t| t == &frag)
                            {
                                if let Some(resolved) =
                                    self.resolve_type_param_with(&frag, &self.current_spec)
                                {
                                    return Some(resolved);
                                }
                                // Try value param resolution
                                let idx = cd.param_order.iter().position(|t| t == &frag);
                                if let Some(idx) = idx {
                                    let sig_frags = Self::split_spec_args(s);
                                    if let Some(v) = sig_frags.get(idx) {
                                        return Some(v.trim().to_string());
                                    }
                                }
                            }
                        }
                    }
                    Some(frag)
                })
                .collect();
            let sig = frags.join(",");
            let canon_sig = self.canonicalize_spec_sig(&base, &sig);
            Some((base.clone(), canon_sig))
        } else {
            None
        }
    }

    pub(super) fn is_string_keyed_array(&self, name: &str) -> bool {
        match self.module.associative_arrays.get(name) {
            Some(true) => return true,
            // §7.8.1: a NESTED element of a multidimensional associative
            // array (`v[0][A]` of `int v[2][string][string]`) is keyed like
            // the array its compound key starts at (see
            // `multidim_assoc_elem`), so `v[0]["A"].exists("B")` looks up the
            // key the write stored.
            None if name.ends_with(']') => {
                // §7.8.1: a declared level has its own key type, not its
                // ancestor's (`x["a"]` of `int x[string][int][$]` is keyed
                // by `int`). A named key type falls through.
                match self.declared_dim(name) {
                    Some(Dim::Assoc(KeyKind::String)) => return true,
                    Some(Dim::Assoc(KeyKind::Integral { .. } | KeyKind::Wildcard)) => {
                        return false;
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        // A per-specialization STATIC collection storage key
        // (`Class#spec::member`, e.g. `wrapper#e_t::map`) rewritten by
        // `spec_static_coll_key` / `obj_member` for a static assoc array whose
        // KEY is `string` (or a type parameter bound to `string`). The bare
        // `map` is registered above (string-keyed after startup seeds the
        // flag), but the per-spec name `Class#spec::map` has no
        // `module.associative_arrays` entry — and its `parse::<usize>()` fails
        // the instance `##` branch below — so without this it fell into the
        // NUMERIC branch of `assoc_key_str` and stored string keys as hashed
        // integers (consistent for /set/get but corrupting foreach()).
        if let Some((head, member)) = name.split_once('#')
            && let Some((_, member2)) = member.rsplit_once("::")
            && let Some(cls) = head.split_once('#').map(|(c, _)| c).or(Some(head))
        {
            if self.static_member_is_string_keyed(cls, member2) {
                return true;
            }
        }

        // Per-instance class member `<handle>#<member>`: the declared key may
        // be a TYPE PARAMETER the instance binds to `string` (`uvm_pool
        // #(string, T)` — the elaboration-time flag is false because KEY is
        // unresolved there). Without this, the same logical key took the
        // NUMERIC branch (<=64-bit values) or the STRING branch (wider ones)
        // in `assoc_key_str` depending on the VALUE's width — so
        // `pool["RTL"]` stored under one storage key and a later lookup with
        // a declared-width-padded copy of the same string missed it (UVM's
        // m_hdl_paths_pool: has_hdl_path() false right after
        // add_hdl_path_slice, killing every backdoor access).
        let Some(pos) = name.find('#') else {
            return false;
        };
        let Ok(handle) = name[..pos].parse::<usize>() else {
            return false;
        };
        let member = &name[pos + 1..];
        let Some(Some(inst)) = self.heap.get(handle) else {
            return false;
        };
        if let Some(v) = self.member_string_keyed_unbound(&inst.class_name, member) {
            return v;
        }
        // Type-param name → concrete, carried down the extends chain (each
        // hop rebinds the parent's params from `extends_type_args`). The
        // chain is walked to the declaring class first; a name is then
        // carried back to the instance's own bindings on demand.
        let mut chain: Vec<&super::elaborate::ElaboratedClass> = Vec::new();
        let mut cur: Option<&str> = Some(inst.class_name.as_str());
        while let Some(cn) = cur {
            if chain.len() == 16 {
                return false;
            }
            let Some(cd) = self.module.classes.get(cn) else {
                return false;
            };
            chain.push(cd.as_ref());
            if let Some(&is_str) = cd.assoc_properties.get(member) {
                if is_str {
                    return true;
                }
                let Some(kt) = cd.assoc_key_types.get(member) else {
                    return false;
                };
                let k = chain.len() - 1;
                let carried =
                    |name: &str| Self::carried_type_binding(&inst.type_bindings, &chain, k, name);
                let mut concrete = carried(kt).unwrap_or_else(|| kt.clone());
                if let Some(c2) = carried(&concrete) {
                    concrete = c2;
                }
                if concrete == "string" {
                    return true;
                }
                if let Some(dt) = self.module.typedef_types.get(&concrete) {
                    if matches!(
                        super::elaborate::resolve_typedef_chain(dt, &self.module.typedef_types),
                        DataType::Simple {
                            kind: crate::ast::types::SimpleType::String,
                            ..
                        }
                    ) {
                        return true;
                    }
                }
                return false;
            }
            cur = cd.extends.as_deref();
        }
        false
    }

    /// Resolve type parameter `param` — declared by an ANCESTOR of
    /// `class_name` (e.g. the `REQ req;` of a `uvm_driver#(REQ)` base) —
    /// for a concrete instance of `class_name` carrying `bindings`.
    /// `bindings` hold only the LEAF class's parameters, so a parent's
    /// parameter name misses there; walk the specialization chain with
    /// `carried_type_binding`, which substitutes each `extends` argument
    /// from below. Without this, `req.rdata = v` in a
    /// `driver#(item) extends uvm_driver#(item)` method failed the
    /// receiver gate in `bare_name_is_class_channel` and the write was
    /// silently dropped (a name collision — child and parent sharing the
    /// parameter name `T` — accidentally resolved through the leaf map).
    pub(super) fn carried_ancestor_param(
        &self,
        class_name: &str,
        bindings: &HashMap<String, String>,
        param: &str,
    ) -> Option<String> {
        let mut chain: Vec<&super::elaborate::ElaboratedClass> = Vec::new();
        let mut cur: Option<&str> = Some(class_name);
        let mut owner: Option<usize> = None;
        while let Some(cn) = cur {
            let Some(cd) = self.module.classes.get(cn) else {
                break;
            };
            if owner.is_none() && cd.type_param_names.iter().any(|p| p == param) {
                owner = Some(chain.len());
            }
            chain.push(cd);
            cur = cd.extends.as_deref();
        }
        Self::carried_type_binding(bindings, &chain, owner?, param)
    }

    /// The concrete class bound to a TYPE-PARAMETER-typed property of an
    /// object. `class_prop_type_named` hands back the parameter NAME for such a
    /// property — `REQ req;` in `uvm_driver #(type REQ = uvm_sequence_item)` —
    /// and leaves the resolution to its callers. The binding sits in the
    /// object's own `type_bindings`, or in an `extends base #(item)` somewhere
    /// between the object's class and the class declaring the property, so
    /// the chain is walked to the declaring class and the name carried back
    /// down it.
    pub(super) fn bound_prop_class(
        &self,
        inst: &ClassInstance,
        prop: &str,
        param: &str,
    ) -> Option<String> {
        let owners = self.prop_owners(&inst.class_name);
        let owner = owners.get(prop)?.0.as_str();
        let mut chain: Vec<&super::elaborate::ElaboratedClass> = Vec::new();
        let mut cur: Option<&str> = Some(inst.class_name.as_str());
        while let Some(cn) = cur {
            if chain.len() == 16 {
                return None;
            }
            let cd = self.module.classes.get(cn)?;
            chain.push(cd.as_ref());
            if cn == owner {
                let k = chain.len() - 1;
                let c = Self::carried_type_binding(&inst.type_bindings, &chain, k, param)?;
                let base = c.split('#').next().unwrap_or(&c);
                return self.module.classes.contains_key(base).then_some(c);
            }
            cur = cd.extends.as_deref();
        }
        None
    }

    /// The binding of type parameter `name` of `chain[k]` for an object whose
    /// class is `chain[0]` with `bindings`: level 0 reads the bindings;
    /// each level above takes the child's `extends_type_args` entry at the
    /// parameter's position, itself carried from below (else taken as
    /// written). None when `name` is not bound at that level.
    pub(super) fn carried_type_binding(
        bindings: &HashMap<String, String>,
        chain: &[&super::elaborate::ElaboratedClass],
        k: usize,
        name: &str,
    ) -> Option<String> {
        Self::carried_type_binding_rec(bindings, chain, k, name, &[])
    }

    /// Defaults may name several earlier parameters. Track their declaration
    /// path to reject a cycle without cutting off a valid long alias chain.
    fn carried_type_binding_rec(
        bindings: &HashMap<String, String>,
        chain: &[&super::elaborate::ElaboratedClass],
        k: usize,
        name: &str,
        path: &[(usize, String)],
    ) -> Option<String> {
        if path
            .iter()
            .any(|(level, param)| *level == k && param == name)
        {
            return None;
        }
        if k == 0 {
            return bindings.get(name).cloned();
        }
        let mut path = path.to_vec();
        path.push((k, name.to_string()));
        let child = chain[k - 1];
        let parent = chain[k];
        let i = parent.param_order.iter().position(|p| p == name)?;
        // An argument the child's `extends` omits is absent or empty in
        // `extends_type_args` and takes the parameter's default (§8.25).
        let written = child
            .extends_type_args
            .get(i)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let a = match written {
            Some(w) => w,
            None => {
                let def = parent
                    .type_param_defaults
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, d)| d.clone())?;
                if parent.type_param_names.iter().any(|p| p == &def) {
                    return Self::carried_type_binding_rec(bindings, chain, k, &def, &path);
                }
                // A default is written in the parent's scope, not the child's.
                return Some(def);
            }
        };
        if Self::spec_projection(&a).is_some() {
            return Self::carry_extends_arg(&a, &|p| {
                Self::carried_type_binding_rec(bindings, chain, k - 1, p, &path)
            });
        }
        Some(Self::carried_type_binding_rec(bindings, chain, k - 1, &a, &path).unwrap_or(a))
    }

    /// The per-instance part of `is_string_keyed_array` for `member` of an
    /// object of class `class_name`, when no type-parameter binding can
    /// change it: the declared key type is not a parameter name, so the
    /// bindings carried down the chain never apply. `None` = depends on the
    /// instance's bindings. Memoized per (class, member).
    pub(super) fn member_string_keyed_unbound(
        &self,
        class_name: &str,
        member: &str,
    ) -> Option<bool> {
        if let Some(hit) = self
            .string_keyed_member_cache
            .borrow()
            .get(class_name)
            .and_then(|m| m.get(member))
        {
            return *hit;
        }
        let v = (|| {
            let mut cur: Option<&str> = Some(class_name);
            let mut level = 0;
            while let Some(cn) = cur {
                level += 1;
                if level > 16 {
                    return Some(false);
                }
                let cd = self.module.classes.get(cn)?;
                if let Some(&is_str) = cd.assoc_properties.get(member) {
                    if is_str {
                        return Some(true);
                    }
                    let Some(kt) = cd.assoc_key_types.get(member) else {
                        return Some(false);
                    };
                    if self.class_member_names().params.contains(kt.as_str()) {
                        return None;
                    }
                    if kt == "string" {
                        return Some(true);
                    }
                    return Some(self.module.typedef_types.get(kt).is_some_and(|dt| {
                        matches!(
                            super::elaborate::resolve_typedef_chain(dt, &self.module.typedef_types),
                            DataType::Simple {
                                kind: crate::ast::types::SimpleType::String,
                                ..
                            }
                        )
                    }));
                }
                cur = cd.extends.as_deref();
            }
            Some(false)
        })();
        self.string_keyed_member_cache
            .borrow_mut()
            .entry(class_name.to_string())
            .or_default()
            .insert(member.to_string(), v);
        v
    }

    /// Warn once per associative array about an invalid (x/z) index.
    pub(super) fn warn_invalid_assoc_index(&mut self, name: &str) {
        if self.warned_assoc_index.insert(name.to_string()) {
            eprintln!(
                "[xezim][warning] associative array '{}' indexed with an unknown (x/z) \
                 value — the access is ignored (IEEE 1800-2017 §7.8.2)",
                name
            );
        }
    }

    /// IEEE 1800-2017 Table 28-1 wired-net resolution for one bit: `z` yields
    /// to a driven value, equal values pass, everything else is `x`.
    /// §6.6.2 one-bit wired-AND (`is_and`) / wired-OR resolution. A `z`
    /// driver contributes nothing, so the other side passes through. The
    /// CONTROLLING value dominates even against `x` — 0 wins for AND, 1 for
    /// OR — which is what makes a wired-AND bus with one asserting driver
    /// resolve cleanly.
    pub(super) fn wired_logic_bit(a: LogicBit, b: LogicBit, is_and: bool) -> LogicBit {
        let ctrl = if is_and {
            LogicBit::Zero
        } else {
            LogicBit::One
        };
        if a == ctrl || b == ctrl {
            return ctrl;
        }
        match (a, b) {
            (LogicBit::Z, x) => x,
            (x, LogicBit::Z) => x,
            (LogicBit::X, _) | (_, LogicBit::X) => LogicBit::X,
            (x, y) if x == y => x,
            _ => LogicBit::X,
        }
    }

    pub(super) fn wire_resolve_bit(a: LogicBit, b: LogicBit) -> LogicBit {
        match (a, b) {
            (LogicBit::Z, x) => x,
            (x, LogicBit::Z) => x,
            (LogicBit::X, _) | (_, LogicBit::X) => LogicBit::X,
            (x, y) if x == y => x,
            _ => LogicBit::X,
        }
    }

    /// Resolve the declared INDEX (key) width + signedness for an
    /// associative array, whether it lives at module scope (`name` in
    /// `module.assoc_index_widths`) or as a per-instance class property
    /// (`<handle>#<member>` — look up the object's class and its
    /// `assoc_index_props`). Returns `None` for string-keyed / untracked
    /// arrays (the caller falls back to the raw index value).
    pub(super) fn assoc_index_width_for(&self, name: &str) -> Option<(u32, bool)> {
        if let Some(&iw) = self.module.assoc_index_widths.get(name) {
            return Some(iw);
        }
        // A BARE STATIC collection name (`imap`, no `#`) of the CURRENT class
        // context (non-parameterized class, so `spec_static_coll_key` leaves
        // the bare name). Resolve the class via `class_context_stack` and read
        // its `static_assoc_index_props`.
        if !name.contains('#') && !name.contains("::") {
            if let Some(Some(ctx)) = self.class_context_stack.last() {
                let mut cur = Some(ctx.clone());
                while let Some(c) = cur {
                    if let Some(cd) = self.module.classes.get(&c) {
                        if cd
                            .static_collections
                            .iter()
                            .any(|(n, is_a, _)| n == name && *is_a)
                        {
                            if let Some(&iw) = cd.static_assoc_index_props.get(name) {
                                return Some(iw);
                            }
                        }
                        cur = cd.extends.clone();
                    } else {
                        break;
                    }
                }
            }
        }
        // A per-specialization STATIC collection storage key
        // (`Class#spec::member`, e.g. `wrapper#e_t::imap`): the class-declared
        // INDEX width+signedness lives in `static_assoc_index_props` (see
        // `ElaboratedClass`). WITHOUT this branch the instance `##` lookup
        // below would bail (its `#[..].parse()` sees `wrapper#e_t`, not a
        // numeric handle), so a signed `int` key like `-3` fell back to the
        // default unsigned (32,false) — `foreach` bound the index as
        // `4294967293` and `imap[that]` missed the stored `imap[-3]`.
        if let Some((head, member)) = name.split_once('#')
            && let Some((_, member2)) = member.rsplit_once("::")
            && let Some(cls) = head.split_once('#').map(|(c, _)| c).or(Some(head))
        {
            let mut cur = Some(cls.to_string());
            while let Some(c) = cur {
                if let Some(cd) = self.module.classes.get(&c) {
                    if let Some(&iw) = cd.static_assoc_index_props.get(member2) {
                        return Some(iw);
                    }
                    cur = cd.extends.clone();
                } else {
                    break;
                }
            }
        }
        // Per-instance class member: `<handle>#<member>`.
        if let Some(pos) = name.find('#') {
            let handle: usize = name[..pos].parse().ok()?;
            let member = &name[pos + 1..];
            let cn = self
                .heap
                .get(handle)
                .and_then(|o| o.as_ref())
                .map(|i| i.class_name.clone())?;
            let mut cur = Some(cn);
            while let Some(c) = cur {
                if let Some(cd) = self.module.classes.get(&c) {
                    if let Some(&iw) = cd.assoc_index_props.get(member) {
                        return Some(iw);
                    }
                    cur = cd.extends.clone();
                } else {
                    break;
                }
            }
        }
        None
    }

    pub(super) fn assoc_key_str(&self, name: &str, idx_val: &Value) -> String {
        if self.is_string_keyed_array(name) {
            idx_val.to_sv_string()
        } else if idx_val.width > 64 {
            // A wide key that isn't flagged string-keyed: `to_u64()` would
            // truncate to the low 64 bits (the last ~8 chars) and collide.
            // This happens for `pool[KEY]` where KEY is a type parameter bound
            // to `string` (`pool#(string,...)` — the key type isn't resolved
            // to SimpleType::String at elaboration). Use the full string form so
            // distinct long keys stay distinct (consistent for set and get).
            // Genuine wide numeric keys were already truncation-broken here, so
            // this only improves correctness.
            // A wide NUMERIC key (a packed struct wider than 64 bits) is keyed
            // by its full-width hex digits so it round-trips through foreach.
            if let Some((kw, _)) = self.assoc_index_width_for(name) {
                if kw > 64 {
                    return idx_val.resize(kw).to_hex();
                }
            }
            idx_val.to_sv_string()
        } else {
            // §7.8.2: the index is narrowed to the declared key type before
            // keying. `int k; shortint aa[shortint]; aa[k]` with a 32-bit k
            // must store the 16-bit-truncated value so a later read with the
            // correctly-narrowed key matches. Without this the packed key text
            // diverged from the unpacked one and AA round-trips corrupted.
            let (mut v, is_signed) = if let Some((kw, ks)) = self.assoc_index_width_for(name) {
                let narrowed = idx_val.resize(kw);
                (narrowed, ks)
            } else {
                (idx_val.clone(), idx_val.is_signed)
            };
            v.is_signed = is_signed;
            if v.is_signed {
                // §7.8: a signed key keeps its sign — folding -5 to 4294967291
                // both mis-stores the element and breaks first()/next() order.
                v.to_i64().unwrap_or(0).to_string()
            } else {
                v.to_u64().unwrap_or(0).to_string()
            }
        }
    }

    /// Collect the TOP-LEVEL keys of an associative array `obj_name`, sorted
    /// and de-duplicated. Correctly handles an associative array WHOSE
    /// ELEMENTS are queues/dynamic-arrays (`T arr[key][$]`) — such arrays
    /// store elements as `arr[KEY][IDX]`, so a naive `arr[KEY]` prefix scan
    /// would collect the nested element keys (`KEY][IDX`) and report
    /// `num()`/`first()` over the wrong set. The top-level assoc key is the
    /// text between `arr[` and the FIRST `]`, uniform across plain assoc
    /// (`arr[KEY]`), assoc-of-queue (`arr[KEY][i]`), and multidim assoc
    /// (`arr[k1][k2]`). Returned sorted (numeric keys ascending by value,
    /// else lexicographic) to match the LRM's first/next/last/prev order and
    /// `num()`'s count of distinct keys.
    /// `signal_name_to_id` keys under `prefix`, memoized.
    ///
    /// That map is written only while the signal table is built and during
    /// `collapse_identity_port_nets` — both elaboration-time — so its keys
    /// under a given prefix are FIXED for the run. Three hot paths used to
    /// re-scan the WHOLE map (`starts_with` per key) on every call:
    /// `assoc_top_level_keys`, assoc `size()`/`len()`, and
    /// `signal_name_prefix_present`. On the axi4 UVM AVIP that scanning was
    /// the single largest source of `memcmp` in the run.
    ///
    /// Callers that also need the LIVE half must still consult
    /// `self.signals` (which mutates at runtime) separately.
    pub(super) fn assoc_static_keys(&self, prefix: &str) -> Arc<Vec<Arc<str>>> {
        if let Some(v) = self.assoc_static_keys_cache.borrow().get(prefix) {
            return Arc::clone(v);
        }
        let v: Arc<Vec<Arc<str>>> = Arc::new(self.signal_name_to_id.names_with_prefix(prefix));
        self.assoc_static_keys_cache
            .borrow_mut()
            .insert(prefix.to_string(), Arc::clone(&v));
        v
    }

    pub(super) fn assoc_top_level_keys(&self, obj_name: &str) -> Vec<String> {
        self.assoc_top_level_keys_shared(obj_name).as_ref().clone()
    }

    /// `assoc_top_level_keys`, shared and cached: `first`/`next`/`num` in a
    /// loop over an unchanged array no longer rescan and re-sort every key.
    pub(super) fn assoc_top_level_keys_shared(&self, obj_name: &str) -> Arc<Vec<String>> {
        let prefix = format!("{}[", obj_name);
        let stamp = self.signals.elem_gen_for_prefix(&prefix);
        if let Some((s, keys)) = self.assoc_keys_cache.borrow().get(obj_name) {
            if *s == stamp {
                return Arc::clone(keys);
            }
        }
        let keys = Arc::new(self.assoc_top_level_keys_uncached(obj_name, &prefix));
        let mut cache = self.assoc_keys_cache.borrow_mut();
        // Per-call and per-object array names come and go; bound the cache
        // rather than track every name's lifetime.
        if cache.len() >= 4096 {
            cache.clear();
        }
        cache.insert(obj_name.to_string(), (stamp, Arc::clone(&keys)));
        keys
    }

    fn assoc_top_level_keys_uncached(&self, obj_name: &str, prefix: &str) -> Vec<String> {
        // A STRUCT element stores member-wise leaves (`sa[k].id`), which do
        // not end in `]` — accept any entry whose key bracket closes, else
        // struct-element assoc arrays enumerate as empty (num()/foreach).
        //
        // The stored entry is `name + rest` where `rest = KEY`, or
        // `KEY[K2]...[Kn]` for a MULTIDIM assoc (`arr[k1][k2]`) or an
        // assoc-of-queue (`arr[key][i]`). Decide which by inspection: a
        // multi-index entry has an inner `[` immediately after its FIRST
        // `]` (`k1][k2]`), so the level-0 key = up to that `]`. Otherwise
        // the array is 1-D and KEY is the WHOLE text, which may itself
        // contain `]`/`[` (UVM component names like
        // `special-:{ chars{}[0123456789] _` are legal assoc string keys,
        // §7.11) — take up to the LAST `]` so such brackets are not mistaken
        // for an index end.
        // The `signal_name_to_id` half of this enumeration is fixed for the
        // run (see `assoc_static_keys_cache`), so scan it once per prefix
        // instead of on every call.
        let static_keys = self.assoc_static_keys(&prefix);
        let mut struct_elem: Option<bool> = None;
        let mut keys: Vec<String> = self
            .signals
            .elem_keys_with_prefix(&prefix)
            .chain(static_keys.iter().map(|k| &**k))
            .filter_map(|k| {
                let rest = &k[prefix.len()..];
                let se = Self::key_has_member_suffix(rest)
                    && *struct_elem
                        .get_or_insert_with(|| self.queue_elem_struct(obj_name).is_some());
                Self::assoc_elem_key_seg(rest, se).map(|s| s.to_string())
            })
            .collect();
        keys.sort();
        keys.dedup();
        // i128 covers both signed keys and full-range u64 keys; with i64 a
        // single 2^63+ key demoted the whole sort to lexicographic.
        if !keys.is_empty() && keys.iter().all(|k| k.parse::<i128>().is_ok()) {
            keys.sort_by_key(|k| k.parse::<i128>().unwrap_or(0));
        }
        keys
    }

    /// Does an lvalue's index need single evaluation (§11.4.1), including
    /// a function call whose purity is not known?
    pub(super) fn lvalue_has_incdec_index(lhs: &Expression) -> bool {
        match &lhs.kind {
            ExprKind::Index { expr, index } => {
                !Self::cond_expr_is_effect_free(index, false) || Self::lvalue_has_incdec_index(expr)
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                !Self::cond_expr_is_effect_free(left, false)
                    || !Self::cond_expr_is_effect_free(right, false)
                    || Self::lvalue_has_incdec_index(expr)
            }
            ExprKind::MemberAccess { expr, .. } | ExprKind::Paren(expr) => {
                Self::lvalue_has_incdec_index(expr)
            }
            _ => false,
        }
    }

    /// §11.4.1: an lvalue's index expressions are evaluated exactly ONCE per
    /// assignment. Rewrite every side-effecting index (`x[i++]`) to the
    /// literal value it evaluates to, applying the side effect here. Returns
    /// None when no index carries a side effect.
    pub(super) fn rewrite_lvalue_index_side_effects(
        &mut self,
        lhs: &Expression,
    ) -> Option<Expression> {
        match &lhs.kind {
            ExprKind::Index { expr, index } => {
                let inner = self.rewrite_lvalue_index_side_effects(expr);
                let idx_needs = !Self::cond_expr_is_effect_free(index, false);
                if inner.is_none() && !idx_needs {
                    return None;
                }
                let new_base = inner.unwrap_or_else(|| (**expr).clone());
                let new_index = if idx_needs {
                    self.capture_index_literal(index, &new_base)
                } else {
                    (**index).clone()
                };
                Some(Expression::new(
                    ExprKind::Index {
                        expr: Box::new(new_base),
                        index: Box::new(new_index),
                    },
                    lhs.span,
                ))
            }
            ExprKind::RangeSelect {
                expr,
                left,
                right,
                kind,
            } => {
                let inner = self.rewrite_lvalue_index_side_effects(expr);
                let l_needs = !Self::cond_expr_is_effect_free(left, false);
                let r_needs = !Self::cond_expr_is_effect_free(right, false);
                if inner.is_none() && !l_needs && !r_needs {
                    return None;
                }
                let new_base = inner.unwrap_or_else(|| (**expr).clone());
                let mut fold = |e: &Expression, needs: bool| -> Expression {
                    if needs {
                        self.capture_index_literal(e, &new_base)
                    } else {
                        e.clone()
                    }
                };
                let new_left = fold(left, l_needs);
                let new_right = fold(right, r_needs);
                Some(Expression::new(
                    ExprKind::RangeSelect {
                        expr: Box::new(new_base),
                        left: Box::new(new_left),
                        right: Box::new(new_right),
                        kind: *kind,
                    },
                    lhs.span,
                ))
            }
            ExprKind::MemberAccess { expr, member } => {
                let base = self.rewrite_lvalue_index_side_effects(expr)?;
                Some(Expression::new(
                    ExprKind::MemberAccess {
                        expr: Box::new(base),
                        member: member.clone(),
                    },
                    lhs.span,
                ))
            }
            ExprKind::Paren(expr) => self.rewrite_lvalue_index_side_effects(expr),
            _ => None,
        }
    }

    /// Preserve strings, signedness, width and individual x/z bits while
    /// capturing a selector once. Unknown selectors must not become zero.
    pub(super) fn capture_index_literal(
        &mut self,
        expr: &Expression,
        base: &Expression,
    ) -> Expression {
        // §7.10: `$` belongs to the selected collection, for both index
        // expressions and range bounds, including `q[$ - f()]`.
        let bound = Self::expr_contains_dollar(expr)
            .then(|| self.dollar_bound_for_base(base))
            .flatten();
        if let Some(bound) = bound {
            self.dollar_bound.push(bound);
        }
        let is_string = self.expr_is_string_valued(expr);
        let v = self.eval_expr(expr);
        if bound.is_some() {
            self.dollar_bound.pop();
        }
        let kind = if is_string {
            ExprKind::StringLiteral(v.to_sv_string())
        } else if v.is_real {
            ExprKind::Number(NumberLiteral::Real(v.to_f64()))
        } else {
            ExprKind::Number(NumberLiteral::Integer {
                size: Some(v.width.max(1)),
                signed: v.is_signed,
                base: NumberBase::Binary,
                value: v.to_bin(),
                cached_val: Cell::new(v.inline_bits().map(|(vbits, xz)| (vbits, xz, v.width))),
            })
        };
        Expression::new(kind, expr.span)
    }

    /// Resolve a nested packed-vector index chain `base[i0][i1]...` (LRM
    /// §7.4.1) to the flat bit slice it selects, using the signal's full
    /// packed dimension list (`packed_full_dims`, outermost first) with
    /// per-dimension index normalization (ascending ranges, non-zero /
    /// negative bounds). Returns `(base_name, Some(lo_bit), slice_width)`;
    /// an out-of-range index yields `(base_name, None, slice_width)` so
    /// reads produce all-x and writes are dropped (§11.5.1) instead of
    /// corrupting neighboring bits.
    pub(super) fn packed_nested_select(
        &mut self,
        base_expr: &Expression,
        outer_index: &Expression,
    ) -> Option<(String, Option<usize>, u32)> {
        let mut idx_exprs: Vec<&Expression> = vec![outer_index];
        let mut cur = base_expr;
        while let ExprKind::Index { expr: inner, index } = &cur.kind {
            idx_exprs.push(index);
            cur = inner;
        }
        let base_name = match &cur.kind {
            ExprKind::Ident(h) => {
                let n = self.resolve_hier_name(h);
                if !self.module.packed_full_dims.contains_key(&*n) && h.path.len() >= 2 {
                    let dotted: String = h
                        .path
                        .iter()
                        .map(|s| s.name.name.as_str())
                        .collect::<Vec<_>>()
                        .join(".");
                    if self.module.packed_full_dims.contains_key(&dotted) {
                        dotted
                    } else {
                        n.to_string()
                    }
                } else {
                    n.into_owned()
                }
            }
            ExprKind::MemberAccess { expr: e, member } => {
                if let ExprKind::Ident(h) = &e.kind {
                    format!("{}.{}", self.resolve_hier_name(h), member.name)
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        let dims = self.module.packed_full_dims.get(&base_name)?.clone();
        let k = idx_exprs.len();
        idx_exprs.reverse(); // outermost dimension's index first

        // §7.4: unpacked dimensions are indexed BEFORE packed ones. When
        // `base_name` is an unpacked array whose element type is itself a
        // packed multi-D vector (`logic [0:0][31:0] arr [0:0]`), the leading
        // `num_unpacked` indices select the unpacked ELEMENT (a per-element
        // signal `arr[i]…`) and only the trailing indices index the packed
        // dims recorded in `packed_full_dims`. Without this split the unpacked
        // index was consumed as a packed dimension, so `arr[0][0]` collapsed
        // to a 1-bit select (bit 0) instead of the 32-bit packed slice.
        // Pure packed vectors (num_unpacked == 0) keep the original behavior:
        // the whole object is one flat signal named `base_name`.
        let num_unpacked = if let Some((d, _)) = self.module.arrays_nd.get(&base_name) {
            d.len()
        } else if self.module.arrays_2d.contains_key(&base_name) {
            2
        } else if self.module.arrays.contains_key(&base_name) {
            1
        } else {
            0
        };
        // Not enough indices to reach packed selection — let the generic
        // unpacked-element path handle a bare element read/write.
        if num_unpacked >= k {
            return None;
        }
        let mut elem_name = base_name.clone();
        for ie in &idx_exprs[..num_unpacked] {
            let iv = self.eval_expr(ie);
            let idx_i = if iv.has_xz() { None } else { iv.to_i64() };
            match idx_i {
                // X / unresolved unpacked index: fall back to the generic path.
                None => return None,
                Some(i) => elem_name.push_str(&format!("[{}]", i)),
            }
        }
        let packed_idx = &idx_exprs[num_unpacked..];
        let pk = packed_idx.len();
        if pk > dims.len() {
            return None;
        }
        let counts: Vec<u64> = dims
            .iter()
            .map(|(l, r)| (l - r).unsigned_abs() + 1)
            .collect();
        let total: u64 = counts.iter().product(); // packed element width in bits
        let elem_w: u64 = counts[pk..].iter().product::<u64>().max(1);
        let mut msb_off: u64 = 0;
        let mut oob = false;
        for (j, ie) in packed_idx.iter().enumerate() {
            let iv = self.eval_expr(ie);
            if iv.has_xz() {
                oob = true;
                break;
            }
            let i = if iv.is_signed {
                iv.to_i64().unwrap_or(i64::MIN)
            } else {
                iv.to_u64().map(|v| v as i64).unwrap_or(i64::MIN)
            };
            let (l, r) = dims[j];
            let (lo_b, hi_b) = if l <= r { (l, r) } else { (r, l) };
            if i < lo_b || i > hi_b {
                oob = true;
                break;
            }
            // Position of the selected element counted from the MSB end:
            // the LEFT bound labels the most-significant element (§7.4.1).
            let slot = if l >= r {
                (l - i) as u64
            } else {
                (i - l) as u64
            };
            let w_j: u64 = counts[j + 1..].iter().product::<u64>().max(1);
            msb_off += slot * w_j;
        }
        if oob {
            return Some((elem_name, None, elem_w as u32));
        }
        let lo = total - msb_off - elem_w;
        Some((elem_name, Some(lo as usize), elem_w as u32))
    }

    pub(super) fn packed_leaf_of_hier(&self, full: &str) -> Option<(String, u32, u32)> {
        // §7.2.1: try EVERY split point, longest base first. `a.b.c.d` may be
        // the signal `a.b` with the field path `c.d` — the layout records a
        // nested member under its dotted path — and not only the signal `a.b.c`
        // with the field `d`. Splitting at the last dot alone was enough at top
        // level, where each nested sub-struct also gets a layout of its own
        // under `s.inner`; through an instance no such key exists, so
        // `u.s.inner.hi` resolved to nothing and read x while the identical
        // `s.inner.hi` at top level read correctly.
        let mut idx = full.len();
        while let Some(pos) = full[..idx].rfind('.') {
            let (base, field) = (&full[..pos], &full[pos + 1..]);
            if let Some(fields) = self.module.packed_struct_fields.get(base) {
                if let Some((_, off, w)) = fields.iter().find(|(m, _, _)| m == field) {
                    return Some((base.to_string(), *off, *w));
                }
            }
            idx = pos;
        }
        None
    }

    pub(super) fn get_signal_value_by_name(&self, name: &str) -> Option<Value> {
        if self.name_stats_on {
            self.name_stats[0].set(self.name_stats[0].get() + 1);
        }

        // Mirror of the write path: a dotted name the innermost frame owns is a
        // member of a subroutine-local / formal unpacked struct and is read from
        // the frame, never from the module signal table.
        if name.contains('.') {
            if let Some(v) = self.frame_get(name) {
                return Some(v.clone());
            }
            // §23.7/§4c: `<top>.<sig>` names the TOP MODULE's own variable —
            // signals are stored unprefixed, so strip the module name when
            // the dotted spelling itself is not a registered signal (a
            // genuine composed name always wins). `tb.counter` read from a
            // class method silently returned x without this.
            if !self.signal_name_to_id.contains_key(name) && !self.signals.contains_key(name) {
                // `<top>.` prefix test without building the prefix string.
                let top = self.module.name.as_str();
                if name.len() > top.len()
                    && name.as_bytes()[top.len()] == b'.'
                    && name.starts_with(top)
                {
                    if let Some(v) = self.get_signal_value_by_name(&name[top.len() + 1..]) {
                        return Some(v);
                    }
                }
            }
        }
        if !name.contains('.') {
            // Every undotted read under a scope hint probed `<hint>.<name>`
            // through a fresh String; reuse one buffer instead.
            let hinted: Option<usize> = {
                let hint = self.name_resolve_hint.borrow();
                match hint.as_deref() {
                    Some(h) => {
                        let mut k = self.hint_key_scratch.borrow_mut();
                        k.clear();
                        k.push_str(h);
                        k.push('.');
                        k.push_str(name);
                        self.signal_name_to_id.get(k.as_str()).copied()
                    }
                    None => None,
                }
            };
            if let Some(id) = hinted {
                let mut v = self.signal_table[id].clone();
                if self.signal_signed[id] {
                    v.is_signed = true;
                }
                return Some(v);
            }
        }
        if let Some(&id) = self.signal_name_to_id.get(name) {
            let mut v = self.signal_table[id].clone();
            if self.signal_signed[id] {
                v.is_signed = true;
            }
            return Some(v);
        }
        // Array-element fallback: <base>[<idx>] resolves via the compact
        // 1D-array resolver (signal_name_to_id no longer has per-element
        // entries for 1D arrays).
        if let Some(id) = resolve_array_elem_id(name, &self.array_first_id) {
            if is_packed_id(id) {
                return Some(self.packed.read(id));
            }
            let mut v = self.signal_table[id].clone();
            if self.signal_signed[id] {
                v.is_signed = true;
            }
            return Some(v);
        }
        // A member of a nested packed struct / untagged union: slice it out of
        // the container rather than reading a stale lazily-created leaf.
        if let Some((base, off, w)) = self.packed_leaf_of_hier(name) {
            // Frame FIRST (§13.4 shadowing). A packed-struct subroutine formal
            // or local keeps its bare name and lives in the CALL FRAME, but
            // module inlining flattens `s.a` into the dotted ident `s.a`, which
            // arrives here. Consulting signals only meant the base lookup
            // missed and the member read came back X — or, where a same-named
            // module-scope object existed, silently returned THAT object's
            // field. The whole-struct read was correct throughout, which is
            // what disguised this as an argument-passing bug.
            if let Some(cur) = self.get_local_or_signal(&base) {
                let mut v = cur.range_select((off + w - 1) as usize, off as usize);
                // §7.2.1: the slice is a raw bit pattern — re-stamp the
                // member's DECLARED signedness (`int s` in a packed struct
                // printed as unsigned without this).
                if let Some(field) = name.strip_prefix(&format!("{}.", base)) {
                    if self.struct_member_signed(&base, field) {
                        v.is_signed = true;
                    }
                    // §7.2.1: a 2-state member of a 4-state struct reads
                    // its x/z bits as 0.
                    if v.has_xz()
                        && !self.mixed_packed_vars.is_empty()
                        && self.packed_member_two_state(&base, field)
                    {
                        let signed = v.is_signed;
                        v = v.to_two_state();
                        v.is_signed = signed;
                    }
                }
                return Some(v);
            }
        }
        let mut v = match self.signals.get(name).cloned() {
            Some(v) => v,
            None => {
                let Some(alt) = strip_genblk_segments(name) else {
                    // A name read at run time that nothing holds: an input
                    // port net left out as unobserved would have held it.
                    self.signal_name_to_id.check_miss(name);
                    return None;
                };
                return self.get_signal_value_by_name(&alt);
            }
        };
        // §6.11.1: an element of an array-of-queue / array-of-dynamic lives
        // only in this slow-path map, so the compact table's signedness stamp
        // above never reaches it — `int aq[2][$]; aq[0][0] = -8` read back
        // 4294967288. Re-stamp from the base declaration's element type.
        if !v.is_signed && !v.is_real {
            if let Some(base) = name.split('[').next().filter(|b| b.len() < name.len()) {
                if self.module.var_decl_types.get(base).is_some_and(|dt| {
                    super::elaborate::is_type_signed_resolved(dt, &self.module.typedef_types)
                }) {
                    v.is_signed = true;
                }
            }
        }
        Some(v)
    }

    pub(super) fn set_signal_value_by_name(&mut self, name: &str, val: Value) {
        // §23.7/§4c: `<top>.<sig>` writes the TOP MODULE's own variable (see
        // the read-side twin). A registered composed name always wins.
        if name.contains('.')
            && !self.signal_name_to_id.contains_key(name)
            && !self.signals.contains_key(name)
            && self
                .local_stack
                .last()
                .is_none_or(|l| !l.contains_key(name))
        {
            if let Some(rest) = name.strip_prefix(&format!("{}.", self.module.name)) {
                if self.signal_name_to_id.contains_key(rest) || self.signals.contains_key(rest) {
                    let rest = rest.to_string();
                    return self.set_signal_value_by_name(&rest, val);
                }
            }
        }
        // §13.4.2: a dotted name the innermost call frame OWNS is a member of a
        // subroutine-local / formal unpacked struct. It must not fall through to
        // the module signal table, where an outer scope's same-named variable
        // lives. Only keys the frame already holds are diverted — formal binding
        // and local declaration are the only things that put them there — so
        // module-scope names are untouched.
        if name.contains('.') {
            if self
                .local_stack
                .last()
                .is_some_and(|l| l.contains_key(name))
            {
                let fitted = match self.frame_get(name) {
                    Some(p) if p.is_real && !val.is_real => Value::from_f64(val.to_f64()),
                    Some(p) if !p.is_real && !val.is_real && p.width > 0 => val.resize(p.width),
                    _ => val,
                };
                if let Some(frame) = self.local_stack.last_mut() {
                    frame.insert(name.to_string(), fitted);
                }
                return;
            }
        }
        if let Some(&id) = self.signal_name_to_id.get(name) {
            let w = self.signal_widths[id];
            // A width-0 signal (e.g. an untracked module-scope class handle)
            // would truncate to nothing; keep the value's natural width so a
            // class handle (`c = factory.create_...()`) is preserved. §6.16: a
            // string (or an element of a string collection) has no declared
            // length either — the table width is a placeholder, and resizing to
            // it chops the text. This is the path `push_back` takes, so without
            // it a `string q[$]` element was still clamped to 128 characters
            // after scalar strings had been fixed.
            let is_str = self.signal_is_string.get(id).copied().unwrap_or(false);
            let mut resized = if w == 0 || is_str { val } else { val.resize(w) };
            // §6.11.1: a 2-state destination drops x/z.
            if self.signal_two_state.get(id).copied().unwrap_or(false) && resized.has_xz() {
                resized = resized.to_two_state();
            }
            // §6.11.1: storage keeps the SIGNAL'S declared signedness, not the
            // rvalue's. An unsized decimal literal is signed (§5.7.1), so
            // `barr[i] = 9` stamped is_signed onto a `bit [3:0]` element and
            // every later read sign-extended it (-7). Strings/handles (w==0)
            // and reals keep their value untouched.
            if w != 0 && !is_str && !resized.is_real {
                resized.is_signed = self.signal_signed[id];
            }
            if self.signal_table[id] != resized {
                self.mark_dirty_id(id);
                write_sig!(self, id, resized);
                self.table_modified = true;
                // §9.2.2.2: a queue / dynamic / associative ELEMENT that
                // happens to own a table id still isn't what comb readers
                // depend on — a variable-index read (`q[i]`) registers the
                // collection's `.size` proxy instead. Marking only the
                // element's own id left such a block permanently stale after
                // `q[0] = v`, while `push_back` (which resizes) woke it fine.
                if let Some(base) = name.find('[').map(|i| &name[..i]) {
                    if !base.is_empty()
                        && (self.module.dynamic_arrays.contains(base)
                            || self.module.associative_arrays.contains_key(base))
                    {
                        let base = base.to_string();
                        self.touch_queue(&base);
                    }
                }
            }
            return;
        }
        // Array-element fallback (1D compact resolver).
        if let Some(id) = resolve_array_elem_id(name, &self.array_first_id) {
            if is_packed_id(id) {
                self.packed_store_value(id, &val);
                return;
            }
            let w = self.signal_widths[id];
            let is_str = self.signal_is_string.get(id).copied().unwrap_or(false);
            let mut resized = if is_str { val } else { val.resize(w) };
            if self.signal_two_state.get(id).copied().unwrap_or(false) && resized.has_xz() {
                resized = resized.to_two_state();
            }
            // §6.11.1: storage keeps the ELEMENT'S declared signedness, not the
            // rvalue's. An unsized decimal literal is signed (§5.7.1), so
            // `barr[i] = 9` stamped is_signed onto a `bit [3:0]` cell and every
            // later read sign-extended it (barr[i] read back as -7).
            if !is_str && !resized.is_real {
                resized.is_signed = self.signal_signed[id];
            }
            if self.signal_table[id] != resized {
                self.mark_dirty_id(id);
                write_sig!(self, id, resized);
                self.table_modified = true;
            }
            return;
        }
        // A member of a nested packed struct / untagged union shares the
        // container's storage (§7.3) — splice into it instead of creating an
        // independent leaf that would silently stop aliasing its siblings.
        if let Some((base, off, w)) = self.packed_leaf_of_hier(name) {
            // Frame first, mirroring the read path above.
            if let Some(cur) = self.get_local_or_signal(&base) {
                let mut next = cur.clone();
                let mut piece = val.resize(w);
                if !self.mixed_packed_vars.is_empty() {
                    piece = self.packed_member_piece(&base, name, piece);
                }
                for i in 0..w {
                    next.set_bit((off + i) as usize, piece.get_bit(i as usize));
                }
                if next != cur {
                    self.set_local_or_signal(&base, next);
                }
                return;
            }
        }
        // LRM §10.6/§10.6.1: drop ordinary writes to a runtime-map target
        // that is under an active force / procedural continuous assign.
        if self.forced_names.contains(name) {
            return;
        }
        // A queue/dynamic-array ELEMENT (or element member) lives only in the
        // runtime map — mark the queue's `.size` comb proxy dirty so readers
        // re-fire (see Simulator::new).
        if let Some(base) = name.find('[').map(|i| &name[..i]) {
            if !base.is_empty()
                && (self.module.dynamic_arrays.contains(base)
                    || self.module.associative_arrays.contains_key(base))
            {
                // §6.11.1: an element of a signed-element class collection
                // keeps the element type's width and signedness, not the
                // rvalue's (`p.dyn[1] = 8'hff` stores the byte -1).
                let val = if !self.signed_class_colls.is_empty()
                    && !val.is_real
                    && !val.is_signed
                    && self.signed_class_colls.contains(base)
                {
                    self.fit_signed_class_coll_elem(base, val)
                } else if !base
                    .split_once('#')
                    .is_some_and(|(h, _)| h.bytes().all(|b| b.is_ascii_digit()))
                    && name.len() > base.len() + 1
                    && name.as_bytes()[base.len() + 1..]
                        .iter()
                        .all(|&b| b.is_ascii_digit() || b == b']')
                {
                    // §7.10/§10.7: an element of a module-scope, static or
                    // block-local queue or dynamic array (no table slot of
                    // its own) takes the element type, as one with a slot
                    // does.
                    self.fit_queue_push_elem(base, val)
                } else {
                    val
                };
                let base = base.to_string();
                self.signals.insert(name.to_string(), val);
                self.touch_queue(&base);
                return;
            }
        }
        self.signals.insert(name.to_string(), val);
    }

    pub(super) fn get_queue_size(&self, obj_name: &str) -> u64 {
        let raw = if let Some(v) = self.signals.get(&Self::name_with_suffix(obj_name, ".size")) {
            v.to_u64().unwrap_or(0)
        } else if self.module.dynamic_arrays.contains(obj_name) {
            // `.size` may have been materialised in the compact signal table
            // (e.g. a package-scope `arr[] = {...}` global) rather than the
            // runtime `signals` map.
            self.get_signal_value_by_name(&Self::name_with_suffix(obj_name, ".size"))
                .and_then(|v| v.to_u64())
                .unwrap_or(0)
        } else if let Some((lo, hi, _)) = self.module.arrays.get(obj_name) {
            (hi - lo + 1) as u64
        } else {
            // A class-property FIXED array (`<handle>#member`) has no
            // runtime `.size` cell and no `module.arrays` entry (those are
            // module-scope). Its element count comes from the class
            // declaration's `array_properties`, so a whole-array copy
            // (`c.p = c.q`) can size the destination.
            self.class_prop_fixed_array_size(obj_name).unwrap_or(0)
        };
        // A queue/array element count above this is never legitimate — it
        // means `<name>.size` holds a garbage value (e.g. an X-coerced or
        // sign-extended scalar). Clamp so element-scan loops can't spin
        // formatting billions of `name[i]` lookups.
        if raw > 1 << 26 { 0 } else { raw }
    }

    pub(super) fn set_queue_size(&mut self, obj_name: &str, size: u64) {
        if !self.assoc_elem_colls.is_empty() && size > 0 && self.assoc_elem_colls.contains(obj_name)
        {
            self.mark_struct_coll_owner_present(obj_name);
        }
        let leaf = obj_name.rsplit("::").next().unwrap_or(obj_name);
        note_store_write(name_bit(leaf));
        note_store_write(name_bit(obj_name));
        let key = Self::name_with_suffix(obj_name, ".size");
        // The `.size` signal is the queue's comb-dependency proxy (see
        // Simulator::new): route through the table write so a size change
        // marks it dirty and re-fires comb readers of `q.size()` / `q[i]`.
        if let Some(&id) = self.signal_name_to_id.get(key.as_str()) {
            // Compare at the slot's declared (signed) type, as `write_sig!`
            // stores it, so an unchanged size is not seen as a change.
            let mut v = Value::from_u64(size, 32);
            v.is_signed = self.signal_signed[id];
            if self.signal_table[id] != v {
                write_sig!(self, id, v);
                self.table_modified = true;
                self.mark_dirty_id(id);
            }
        }
        self.signals.insert(key, Self::int_count(size));
    }

    /// Mark a queue's `.size` proxy dirty WITHOUT changing the size — used by
    /// element-level mutations (`q[i] = v`, `q[i].member = v`) so comb readers
    /// of the queue re-evaluate.
    pub(super) fn touch_queue(&mut self, obj_name: &str) {
        let key = Self::name_with_suffix(obj_name, ".size");
        if let Some(&id) = self.signal_name_to_id.get(key.as_str()) {
            self.mark_dirty_id(id);
            self.dirty_any = true;
        }
    }

    /// Element count of a class-property FIXED array named
    /// `<handle>#member`. Returns `None` unless `member` is a constant-sized
    /// unpacked-array property of the instance's class (walked up the
    /// inheritance chain). Used by `get_queue_size` so whole-array copies
    /// (`c.p = c.q`) can read a fixed-array property's length.
    pub(super) fn class_prop_fixed_array_size(&self, name: &str) -> Option<u64> {
        let (hstr, member) = name.split_once('#')?;
        let h = hstr.parse::<usize>().ok()?;
        let cn = self
            .heap
            .get(h)
            .and_then(|x| x.as_ref())
            .map(|i| i.class_name.clone())?;
        let mut cur = Some(cn);
        while let Some(c) = cur {
            if let Some(cd) = self.module.classes.get(&c) {
                if let Some(&(lo, hi, _)) = cd.array_properties.get(member) {
                    let (lo, hi) = self.inst_fixed_bounds(cd, h, member, (lo, hi));
                    return Some((hi - lo + 1) as u64);
                }
                cur = cd.extends.clone();
            } else {
                break;
            }
        }
        None
    }

    /// Resolve a whole fixed-size (unpacked) array operand — used by unpacked-
    /// array equality (§11.4.5) — to its element storage name and inclusive
    /// index range. Handles module-level arrays (`module.arrays`) and class-
    /// property fixed arrays reached as a bare `member` inside a method,
    /// `this.member`, or `obj.member` (storage is instance-scoped
    /// `<handle>#member[<i>]`). Returns `None` for non-fixed-array operands
    /// (dynamic/queue/assoc/scalars) so the caller can fall through.
    pub(super) fn fixed_array_operand(&mut self, expr: &Expression) -> Option<(String, i64, i64)> {
        // Class-property fixed array via any collection-resolvable shape
        // (bare `member`, `this.member`, `obj.member`, flattened `obj.member`).
        if let Some(scoped) = self.expr_assoc_name(expr) {
            if let Some((hstr, member)) = scoped.split_once('#') {
                if let Ok(h) = hstr.parse::<usize>() {
                    let cn = self
                        .heap
                        .get(h)
                        .and_then(|o| o.as_ref())
                        .map(|inst| inst.class_name.clone());
                    if let Some(cn) = cn {
                        let mut cur = Some(cn);
                        while let Some(c) = cur {
                            if let Some(cd) = self.module.classes.get(&c) {
                                if let Some(&(lo, hi, _)) = cd.array_properties.get(member) {
                                    let (lo, hi) = self.inst_fixed_bounds(cd, h, member, (lo, hi));
                                    return Some((scoped, lo, hi));
                                }
                                cur = cd.extends.clone();
                            } else {
                                break;
                            }
                        }
                    }
                }
            }
        }
        // Module-level fixed array (bare Ident). A DYNAMIC array / queue is
        // registered in `module.arrays` with the (0,-1) queue SENTINEL range
        // (and in `dynamic_arrays`) — it is NOT a fixed array and must not be
        // compared element-wise as the empty 0:-1 range: UVM's `val == t` on
        // two `T`-typed queues would otherwise both resolve to (0,-1) and the
        // guard falsely reported them equal (skipping the update).
        if let ExprKind::Ident(h) = &expr.kind
            && !self.ident_shadowed_by_non_collection(h)
        {
            let name = self.resolve_hier_name(h);
            if let Some(&(lo, hi, _)) = self.module.arrays.get(&*name) {
                if !self.module.dynamic_arrays.contains(&*name) && !(hi < lo) {
                    return Some((name.to_string(), lo, hi));
                }
            }
        }
        None
    }

    /// IEEE 1800-2023 §18.14 — the stream every random draw must come from.
    ///
    /// Priority:
    ///   1. the OBJECT currently being `randomize()`d, if it owns a private
    ///      stream (`srandom`/`set_randstate` was called on it). Its sequence is
    ///      therefore independent of the calling thread's — reseeding a thread
    ///      must not perturb an object's stream (§18.14.1);
    ///   2. the CURRENT PROCESS, if it owns a private stream. This backs
    ///      `$urandom`, `$urandom_range`, `shuffle()` and the randcase /
    ///      randsequence draws, so a `process::self().srandom(s)` makes the
    ///      thread's whole subsequent sequence a function of `s` alone,
    ///      independent of the scheduling order of sibling threads;
    ///   3. otherwise the simulator-wide default stream (historical behaviour:
    ///      entropy-seeded, or `+seed=<n>`).
    ///
    /// An object with NO private stream draws from the CURRENT PROCESS's stream
    /// (priority 2, else 3): `process::self().srandom(s)` therefore makes a
    /// fresh, otherwise-unseeded object's subsequent `randomize()` reproducible
    /// from `s` alone, exactly as an unseeded object's draw is attributed to its
    /// calling thread (§18.14.1). Only an object with its OWN stream is immune
    /// to the thread's seeding, so a seeded object is never perturbed by a
    /// sibling's thread reseed.
    pub(super) fn cur_rng(&mut self) -> &mut SvRng {
        if let Some(&h) = self.obj_rng_stack.last() {
            if self.obj_rng.contains_key(&h) {
                return self.obj_rng.get_mut(&h).unwrap();
            }
            // No private stream — fall through to the calling process's stream.
        }
        let pid = self.current_pid;
        if self.proc_rng.contains_key(&pid) {
            return self.proc_rng.get_mut(&pid).unwrap();
        }
        &mut self.rng
    }

    /// §18.14 `srandom` / `get_randstate` / `set_randstate` on a *process*
    /// handle (`process::self()`, §9.7) or on a *class object* handle.
    /// Returns None when `method_name` is not one of them.
    pub(super) fn exec_rand_state_method(
        &mut self,
        handle: usize,
        method_name: &str,
        args: &[Expression],
    ) -> Option<Value> {
        if !matches!(method_name, "srandom" | "get_randstate" | "set_randstate") {
            return None;
        }
        // A `process::self()` token addresses the per-process stream; anything
        // else is a class-object handle and addresses the per-object stream.
        let is_process = handle as u64 >= PROCESS_HANDLE_BASE;
        let key = if is_process {
            (handle as u64 - PROCESS_HANDLE_BASE) as usize
        } else {
            handle
        };
        match method_name {
            // §18.13.3 / §18.14.1: seed the stream. Deterministic — the same
            // seed always produces the same subsequent sequence.
            "srandom" => {
                let seed = args
                    .first()
                    .map(|a| self.eval_expr(a).to_u64().unwrap_or(0))
                    .unwrap_or(0);
                let s = SvRng::from_seed(seed);
                if is_process {
                    self.proc_rng.insert(key, s);
                } else {
                    self.obj_rng.insert(key, s);
                }
                Some(Value::zero(32))
            }
            // §18.14.2: serialise the stream state. The stream must exist from
            // now on (otherwise there would be nothing for set_randstate to
            // restore), so materialise it lazily off the default RNG.
            "get_randstate" => {
                let existing = if is_process {
                    self.proc_rng.get(&key).copied()
                } else {
                    self.obj_rng.get(&key).copied()
                };
                let s = match existing {
                    Some(s) => s,
                    None => {
                        let fresh = SvRng::from_seed(rand::RngCore::next_u64(&mut self.rng));
                        if is_process {
                            self.proc_rng.insert(key, fresh);
                        } else {
                            self.obj_rng.insert(key, fresh);
                        }
                        fresh
                    }
                };
                Some(Value::from_string(&s.to_state_string()))
            }
            // §18.14.2: restore a previously captured state — the stream then
            // replays exactly the sequence it produced after the capture.
            "set_randstate" => {
                let st = args
                    .first()
                    .map(|a| self.eval_expr(a).to_sv_string())
                    .unwrap_or_default();
                let s = SvRng::from_state_string(&st);
                if is_process {
                    self.proc_rng.insert(key, s);
                } else {
                    self.obj_rng.insert(key, s);
                }
                Some(Value::zero(32))
            }
            _ => None,
        }
    }

    /// A fresh random 32-bit value from the current stream (§18.14). Backs
    /// `$urandom`/`$random` so riscv-dv's `$urandom_range`-driven choices
    /// (register dist, branch steps, illegal-instr injection, …) actually vary.
    pub(super) fn rng_u32(&mut self) -> u32 {
        use rand::Rng;
        self.cur_rng().r#gen()
    }

    /// Inclusive random in [lo, hi] (lo>hi yields lo). Backs `$urandom_range`.
    pub(super) fn rng_range_u64(&mut self, lo: u64, hi: u64) -> u64 {
        use rand::Rng;
        if hi >= lo {
            self.cur_rng().gen_range(lo..=hi)
        } else {
            lo
        }
    }

    /// Deterministic next state for a seeded `$random(seed)`/`$urandom(seed)`
    /// stream (xorshift32). Pure function of the seed → identical seeds yield
    /// identical sequences.
    pub(super) fn prng_next(s: u32) -> u32 {
        let mut x = if s == 0 { 0x9E37_79B9 } else { s };
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    }

    /// Get a scalar value preferring the current call frame's locals (where
    /// string/scalar PARAMETERS and locals live) over the global signal table.
    /// String built-ins like `.len()`/`.substr()` need this — a `string` formal
    /// (e.g. `format_string(string str, …)`) is not in `self.signals`.
    /// Write a variable that may be a procedural LOCAL (a task/function frame)
    /// rather than a signal. A packed struct declared inside a task lives in the
    /// local frame, so slicing a member into it through `set_signal_value_by_name`
    /// wrote nothing and the whole struct read back as X.
    pub(super) fn set_local_or_signal(&mut self, name: &str, v: Value) {
        if let Some(frame) = self.local_stack.last_mut() {
            if frame.contains_key(name) {
                frame.insert(name.to_string(), v);
                return;
            }
        }
        self.set_signal_value_by_name(name, v);
    }

    /// A bare identifier bound in the current subroutine frame (a formal or
    /// a local). Such a name must not go through `resolve_hier_name`, which
    /// resolves DESIGN signals and can rewrite `name` into a same-named
    /// signal of some instance (`u_bfm.name`) — a class method's formal
    /// `name` then read an interface's string (§6.21 / §13.3: a
    /// subroutine's own names shadow everything outside it).
    pub(super) fn frame_bound_bare_name<'h>(
        &self,
        hier: &'h HierarchicalIdentifier,
    ) -> Option<&'h str> {
        if hier.path.len() != 1 || !hier.path[0].selects.is_empty() || hier.root.is_some() {
            return None;
        }
        let n = hier.path[0].name.name.as_str();
        self.local_stack
            .last()
            .is_some_and(|l| l.contains_key(n))
            .then_some(n)
    }

    /// The bare variable a whole or select lvalue writes: `a` in `a`,
    /// `a[3:0]`, `a[i +: 4]` or `a[i][j]`. None for anything else, a member
    /// hierarchy roots are excluded; member writes keep their local root.
    pub(super) fn lvalue_bare_root(e: &Expression) -> Option<&str> {
        match &e.kind {
            ExprKind::Index { expr, .. }
            | ExprKind::RangeSelect { expr, .. }
            | ExprKind::MemberAccess { expr, .. }
            | ExprKind::Paren(expr) => Self::lvalue_bare_root(expr),
            ExprKind::Ident(h) if !h.path.is_empty() && h.root.is_none() => {
                Some(h.path[0].name.name.as_str())
            }
            _ => None,
        }
    }

    pub(super) fn get_local_or_signal(&self, name: &str) -> Option<Value> {
        if let Some(m) = self.local_stack.last() {
            if let Some(v) = m.get(name) {
                return Some(v.clone());
            }
        }
        self.get_signal_value_by_name(name)
    }

    /// Like `get_local_or_signal`, but also resolves **static class
    /// properties** when `name` refers to one in the current class context.
    /// Built-in string/array methods (`.len()`, `.substr()`, `.getc()`, …)
    /// fetch their receiver by name via `eval_builtin_method`, and a static
    /// property lives in `class_statics` — not in the local frame or the
    /// signal table — so a bare `m_space.substr(...)` inside a class method
    /// would otherwise read an empty string and silently produce wrong
    /// output (e.g. table printers emitting columns with no padding).
    pub(super) fn get_local_signal_or_static(&mut self, name: &str) -> Option<Value> {
        if let Some(v) = self.get_local_or_signal(name) {
            return Some(v);
        }
        if let Some(Some(ctx)) = self.class_context_stack.last().cloned() {
            if let Some(v) = self.class_static_get(&ctx, name) {
                return Some(v);
            }
        }
        // §8.10/§6.16: an INSTANCE property as a built-in method receiver —
        // `n.len()` inside a class method fetches its receiver by NAME through
        // here, and nothing consulted the heap: every string builtin on a
        // property read an empty string and the `.len()` tail returned a
        // silent 0. Shadow-resolved, so a base method's `n` names the base's
        // copy.
        if let Some(Some(handle)) = self.this_stack.last() {
            let key = self
                .shadowed_prop_key(name, false)
                .unwrap_or_else(|| name.to_string());
            if let Some(v) = self
                .heap
                .get(*handle)
                .and_then(|o| o.as_ref())
                .and_then(|i| i.properties.get(&key))
            {
                return Some(v.clone());
            }
        }
        None
    }

    /// Set a loop index/iteration variable. Writes to the current call frame's
    /// locals (highest read precedence) when one exists, so a `foreach`/`for`
    /// index properly SHADOWS any same-named outer local or class member —
    /// otherwise a stale local (e.g. `i` left at its final value by a prior
    /// `for` loop) would mask the loop variable on every read.
    /// Is `an` an associative array whose elements are themselves
    /// associative arrays: entries `an[k1][k2]...` with no queue/dynamic
    /// `.size` shadow on `an[k1]`.
    pub(super) fn is_assoc_of_assoc(&self, an: &str) -> bool {
        let akeys = self.assoc_top_level_keys(an);
        !akeys.is_empty()
            && akeys.iter().all(|k| {
                let inner = format!("{}[{}]", an, k);
                !self.module.dynamic_arrays.contains(&inner)
                    && !self.signals.contains_key(&format!("{}.size", inner))
                    && !self.assoc_top_level_keys(&inner).is_empty()
            })
    }

    /// §12.7.3: a multi-variable `foreach` over the declared collection of
    /// collections `name` (`foreach (d[i, k, j])` on `int d[][string][$]`)
    /// walks each level as its declaration names it: a queue or dynamic
    /// level by index below its size, an associative level by its keys in
    /// §7.8.4 order, each loop variable taking that level's index type.
    /// Returns false — leaving the loop to the other paths — for a store with
    /// no such shape, one with fixed levels (iterated by their bounds), or a
    /// level keyed by a named type (an enum or class key).
    pub(super) fn foreach_shaped(
        &mut self,
        name: &str,
        vars: &[Option<crate::ast::Identifier>],
        body: &Statement,
        iterations: &mut u64,
    ) -> bool {
        let Some((shape, 0)) = self.coll_shape_at(name) else {
            return false;
        };
        let walkable = |d: &Dim| {
            matches!(
                d,
                Dim::Queue
                    | Dim::Dyn
                    | Dim::Assoc(KeyKind::String | KeyKind::Integral { .. } | KeyKind::Wildcard)
            )
        };
        if vars.len() > shape.dims.len() || !shape.dims.iter().all(walkable) {
            return false;
        }
        let dims = shape.dims.clone();
        self.foreach_shaped_level(name, &dims, vars, 0, body, iterations);
        true
    }

    /// One level of [`foreach_shaped`]. Returns false once the loop is broken.
    fn foreach_shaped_level(
        &mut self,
        prefix: &str,
        dims: &[Dim],
        vars: &[Option<crate::ast::Identifier>],
        level: usize,
        body: &Statement,
        iterations: &mut u64,
    ) -> bool {
        // The keys of this level, and the loop variable's type: `None` for a
        // string key, else (width, signed).
        let (keys, ty): (Vec<String>, Option<(u32, bool)>) = match &dims[level] {
            Dim::Assoc(KeyKind::String) => (self.assoc_top_level_keys(prefix), None),
            Dim::Assoc(k) => {
                let ty = match k {
                    KeyKind::Integral { width, signed } => (*width, *signed),
                    _ => (32, false),
                };
                let mut keys = self.assoc_top_level_keys(prefix);
                keys.sort_by_key(|k| k.parse::<i128>().unwrap_or(0));
                (keys, Some(ty))
            }
            // §7.8.6: a row past its parent's size does not exist.
            _ => {
                let n = match self.declared_dim(prefix) {
                    Some(_) => self.get_queue_size(prefix),
                    None => 0,
                };
                ((0..n).map(|i| i.to_string()).collect(), Some((32, true)))
            }
        };
        // A string key variable renders as a string (§7.8.1).
        let str_var = vars[level]
            .as_ref()
            .filter(|_| ty.is_none())
            .map(|v| v.name.clone())
            .filter(|n| self.string_signals.insert(n.clone()));
        let mut live = true;
        for key in keys {
            if self.finished {
                live = false;
                break;
            }
            if let Some(v) = vars[level].as_ref() {
                let kv = match ty {
                    None => {
                        self.widths.remove(&v.name);
                        Value::from_string(&key)
                    }
                    Some((w, signed)) => {
                        self.widths.insert(v.name.clone(), w);
                        let mut kv = Value::from_u64(key.parse::<i128>().unwrap_or(0) as u64, w);
                        kv.is_signed = signed;
                        kv
                    }
                };
                self.set_loop_var(&v.name, kv);
            }
            if level + 1 < vars.len() {
                let row = format!("{prefix}[{key}]");
                if !self.foreach_shaped_level(&row, dims, vars, level + 1, body, iterations) {
                    live = false;
                    break;
                }
            } else {
                self.continue_flag = false;
                *iterations = iterations.saturating_add(1);
                self.exec_statement(body);
                if self.break_flag {
                    live = false;
                    break;
                }
            }
        }
        if let Some(n) = str_var {
            self.string_signals.remove(&n);
        }
        live
    }

    /// One level of a multi-variable `foreach` over nested associative
    /// arrays: bind `vars[level]` to each key of `prefix` and descend, or run
    /// the body at the last variable (§12.7.3). The outer level takes the
    /// declared index type; an inner level is string-keyed when any of its
    /// keys is not a number. Returns false once the loop is broken.
    pub(super) fn foreach_assoc_levels(
        &mut self,
        root: &str,
        prefix: &str,
        vars: &[Option<crate::ast::Identifier>],
        level: usize,
        body: &Statement,
        iterations: &mut u64,
    ) -> bool {
        let keys = self.assoc_top_level_keys(prefix);
        // An inner level's declared key type: a string key stored by value
        // is turned back into its text (§7.8.1).
        let declared_str = match self.declared_dim(prefix).filter(|_| level > 0) {
            Some(Dim::Assoc(KeyKind::String)) => Some(true),
            Some(Dim::Assoc(KeyKind::Integral { .. } | KeyKind::Wildcard)) => Some(false),
            _ => None,
        };
        let (is_str, kw, ksign) = if level == 0 {
            let is_str = self
                .module
                .associative_arrays
                .get(prefix)
                .copied()
                .unwrap_or(false)
                || self.is_string_keyed_array(prefix);
            let (kw, ks) = self.assoc_index_width_for(prefix).unwrap_or((32, true));
            (is_str, kw, ks)
        } else if let Some(&row_str) = self.module.associative_arrays.get(prefix) {
            // A row registered as an associative array of its own (see
            // `ensure_assoc_row`) keys its elements by its declared key type.
            let (kw, ks) = self.assoc_index_width_for(prefix).unwrap_or((32, true));
            (row_str || declared_str.unwrap_or(false), kw, ks)
        } else {
            // No declared shape, or a named key type (enum, class, a type
            // parameter bound per specialization): the stored keys tell.
            (
                declared_str.unwrap_or_else(|| keys.iter().any(|k| k.parse::<i128>().is_err())),
                32,
                true,
            )
        };
        // A string key variable renders as a string (§7.8.1).
        let str_var = vars[level]
            .as_ref()
            .filter(|_| is_str)
            .map(|v| v.name.clone())
            .filter(|n| self.string_signals.insert(n.clone()));
        let done = self.foreach_assoc_level_keys(
            root, prefix, keys, vars, level, body, iterations, is_str, kw, ksign,
        );
        if let Some(n) = str_var {
            self.string_signals.remove(&n);
        }
        done
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn foreach_assoc_level_keys(
        &mut self,
        root: &str,
        prefix: &str,
        keys: Vec<String>,
        vars: &[Option<crate::ast::Identifier>],
        level: usize,
        body: &Statement,
        iterations: &mut u64,
        is_str: bool,
        kw: u32,
        ksign: bool,
    ) -> bool {
        for key in keys {
            if self.finished {
                return false;
            }
            if let Some(v) = vars[level].as_ref() {
                // An inner level's keys are stored the way its parent
                // element is keyed: as text under a string-keyed parent
                // (an int key 2 is the character 0x02), else by value (a
                // string key "ab" is the number 0x6162).
                let as_text = level > 0
                    && !self.module.associative_arrays.contains_key(prefix)
                    && (self.is_string_keyed_array(prefix)
                        || self.is_string_keyed_array(root)
                        || (!is_str && key.parse::<i128>().is_err()));
                let kv = if is_str {
                    match key.parse::<u128>() {
                        Ok(n) if level > 0 && !as_text => {
                            let b = n.to_be_bytes();
                            let first = b.iter().position(|&c| c != 0).unwrap_or(b.len());
                            Value::from_string(&String::from_utf8_lossy(&b[first..]))
                        }
                        _ => Value::from_string(&key),
                    }
                } else if as_text {
                    let mut v = Value::from_string(&key).resize(kw);
                    v.is_signed = ksign;
                    v
                } else if kw > 64 {
                    Value::from_str_radix(&key, 16, kw)
                } else {
                    let mut v = Value::from_u64(key.parse::<i64>().unwrap_or(0) as u64, kw);
                    v.is_signed = ksign;
                    v
                };
                if is_str {
                    self.widths.remove(&v.name);
                } else {
                    self.widths.insert(v.name.clone(), kw);
                }
                self.set_loop_var(&v.name, kv);
            }
            if level + 1 < vars.len() {
                let inner = format!("{}[{}]", prefix, key);
                if !self.foreach_assoc_levels(root, &inner, vars, level + 1, body, iterations) {
                    return false;
                }
            } else {
                self.continue_flag = false;
                *iterations = iterations.saturating_add(1);
                self.exec_statement(body);
                if self.break_flag {
                    return false;
                }
                self.continue_flag = false;
            }
        }
        true
    }

    pub(super) fn set_loop_var(&mut self, name: &str, v: Value) {
        if let Some(frame) = self.local_stack.last_mut() {
            frame.insert(name.to_string(), v);
        } else {
            self.set_signal_value_by_name(name, v);
        }
    }

    /// `set_loop_var`, plus a scope-prefixed alias. Inlining rewrites a
    /// submodule foreach BODY's loop-var references to "u_s.i" but leaves the
    /// foreach var list's Identifiers bare, so the body's index reads would
    /// miss the value `set_loop_var("i", ..)` stored. `scope` is the array's
    /// instance prefix (None at top level, where names already agree).
    pub(super) fn set_loop_var_aliased(&mut self, scope: Option<&str>, name: &str, v: Value) {
        if let Some(p) = scope {
            self.set_loop_var(&format!("{}.{}", p, name), v.clone());
        }
        self.set_loop_var(name, v);
    }

    /// Snapshot loop-index variables' current values (for `foreach` scope
    /// save/restore). A `foreach (arr[i])` index is loop-scoped in SV, so the
    /// outer `i` must be unchanged afterward — riscv-dv's
    /// `foreach(instr_list[i]) … ; while(i < size) …` depends on it.
    pub(super) fn snapshot_loop_vars(&self, names: &[String]) -> Vec<LoopVarSnapshot> {
        // `set_loop_var` writes local_stack when a frame exists, else signals;
        // snapshot from (and later restore to) that same location so the
        // foreach shadow is removed without clobbering the outer variable
        // (a function-local lives in `signals`, not the call frame's locals).
        // Also snapshot the var's TYPEDEF binding and its trusted mark, so a
        // foreach key-type binding (§6.19.6) is scoped to the loop.
        let has_frame = self.local_stack.last().is_some();
        names
            .iter()
            .map(|n| {
                let cur = if has_frame {
                    self.local_stack.last().and_then(|m| m.get(n).cloned())
                } else {
                    self.get_signal_value_by_name(n)
                };
                (
                    n.clone(),
                    cur,
                    has_frame,
                    self.var_typedef_types.get(n).cloned(),
                    self.fe_trusted_types.contains(n),
                    self.module.packed_struct_fields.get(n).cloned(),
                    (self.widths.get(n).copied(), self.signed_signals.contains(n)),
                )
            })
            .collect()
    }

    pub(super) fn restore_loop_vars(&mut self, saved: &[LoopVarSnapshot]) {
        for (n, _cur, _had_frame, prior_td, was_trusted, prior_ps, _) in saved {
            self.meta_note(n);
            match prior_td {
                Some(t) => {
                    self.var_typedef_types.insert(n.clone(), t.clone());
                }
                None => {
                    self.var_typedef_types.remove(n);
                }
            }
            if *was_trusted {
                self.fe_trusted_types.insert(n.clone());
            } else {
                self.fe_trusted_types.remove(n);
            }
            if self.module.packed_struct_fields.get(n) != prior_ps.as_ref() {
                match prior_ps {
                    Some(f) => {
                        self.module
                            .packed_struct_fields
                            .insert(n.clone(), f.clone());
                    }
                    None => {
                        self.module.packed_struct_fields.remove(n);
                    }
                }
            }
        }
        for (n, _, _, _, _, _, (width, signed)) in saved {
            match width {
                Some(w) => {
                    self.widths.insert(n.clone(), *w);
                }
                None => {
                    self.widths.remove(n);
                }
            }
            if *signed {
                self.signed_signals.insert(n.clone());
            } else {
                self.signed_signals.remove(n);
            }
        }
        for (n, cur, had_frame, _, _, _, _) in saved {
            if *had_frame {
                if let Some(m) = self.local_stack.last_mut() {
                    match cur {
                        Some(v) => {
                            m.insert(n.clone(), v.clone());
                        }
                        None => {
                            m.remove(n);
                        }
                    }
                }
            } else {
                match cur {
                    Some(v) => self.set_signal_value_by_name(n, v.clone()),
                    None => {
                        self.signals.remove(n);
                    }
                }
            }
        }
    }

    /// Compute the iteration keys for a named array/collection
    /// (`<name>.size` dense fallback, else raw key-scan of `<name>[…]`).
    /// Returns `(keys, is_str)`. Integer keys are sorted numerically.
    /// Top-level assoc key for a stored entry whose text after the array
    /// name is `rest`. A MULTIDIM or collection-valued assoc stores
    /// `arr[k1][k2]...` (an inner `[` right after the FIRST `]`), so the
    /// level-0 key runs to that `]`. Otherwise the array is 1-D and the key
    /// is the WHOLE text to the FINAL `]` — a 1-D string key may itself
    /// contain `]`/`[` (a UVM component name like
    /// `special-:{ chars{}[0123456789] _` is a legal assoc key, §7.11).
    pub(super) fn assoc_first_key_seg(rest: &str) -> Option<&str> {
        if rest
            .find(']')
            .map_or(false, |p| rest[p + 1..].starts_with('['))
        {
            rest.find(']').map(|e| &rest[..e])
        } else {
            rest.rfind(']').map(|e| &rest[..e])
        }
    }

    /// `assoc_first_key_seg` for an array whose ELEMENT is an unpacked
    /// struct when `struct_elem`: such an element is stored as its member
    /// leaves (`sa[KEY].m`, `sa[KEY].arr[1][0]`), so the key ends at the
    /// first `].` — the last `]` would take the member's own index as part
    /// of the key and enumerate one phantom key per array-member leaf.
    /// Whether a stored entry's text after `name[` has a `].` — the shape of
    /// a struct-element member leaf. A byte scan on `]` (memchr), not a
    /// substring search: this runs once per stored entry of every
    /// associative-array enumeration.
    pub(super) fn key_has_member_suffix(rest: &str) -> bool {
        let mut s = rest;
        while let Some(p) = s.find(']') {
            if s.as_bytes().get(p + 1) == Some(&b'.') {
                return true;
            }
            s = &s[p + 1..];
        }
        false
    }

    pub(super) fn assoc_elem_key_seg(rest: &str, struct_elem: bool) -> Option<&str> {
        if struct_elem {
            if let Some(p) = rest.find("].") {
                return Self::assoc_first_key_seg(&rest[..=p]);
            }
        }
        Self::assoc_first_key_seg(rest)
    }

    pub(super) fn array_iter_keys(&self, name: &str) -> (Vec<String>, bool) {
        // A queue / dynamic array carries an authoritative `<name>.size`
        // shadow and is DENSE: iterate the logical range [0, size). Scanning
        // the raw signal keys instead would visit stale elements left behind
        // when the queue shrank. Only TRUE associative arrays (sparse) fall
        // through to the key-scan.
        if let Some(sz) = self
            .signals
            .get(&format!("{}.size", name))
            .and_then(|v| v.to_u64())
        {
            return ((0..sz).map(|i| i.to_string()).collect(), false);
        }
        let prefix = format!("{}[", name);
        // For an associative array whose VALUES are themselves collections
        // (e.g. `bq_t all[int]` where `bq_t = Base[$]`), an element at key 5
        // is stored as `all[5][0]`, `all[5][1]`, etc. The key must be
        // extracted up to the FIRST `]`, not `k.len()-1` (which would grab
        // `5][0`). Dedup because multiple sub-elements share one key.
        //
        // But a 1-D STRING key may itself contain `]`/`[` (e.g. a UVM
        // component name `special-:{ chars{}[0123456789] _`), stored
        // verbatim as `arr[KEY]`. Distinguish by shape: a multi-index entry
        // has an inner `[` right after its first `]` (`k1][k2]`), so extract
        // up to that `]`; otherwise take the whole key up to the final `]`.
        let mut struct_elem: Option<bool> = None;
        let mut ks: Vec<String> = self
            .signals
            .elem_keys_with_prefix(&prefix)
            .filter_map(|k| {
                let rest = k.strip_prefix(prefix.as_str())?;
                let se = Self::key_has_member_suffix(rest)
                    && *struct_elem.get_or_insert_with(|| self.queue_elem_struct(name).is_some());
                Self::assoc_elem_key_seg(rest, se).map(|s| s.to_string())
            })
            .collect();
        ks.sort();
        ks.dedup();
        let is_str = self
            .module
            .associative_arrays
            .get(name)
            .copied()
            .unwrap_or(false)
            || self.is_string_keyed_array(name);
        if is_str {
            ks.sort();
        } else {
            // §7.9: numeric order incl. NEGATIVE int keys.
            ks.sort_by_key(|k| k.parse::<i64>().unwrap_or(0));
        }
        (ks, is_str)
    }

    /// Materialize the iteration keys for a SINGLE-loop-variable `foreach`
    /// (1-D fixed, dynamic, and associative arrays, e.g. `foreach (array[elem])`).
    /// Returns `(keys, is_str, var_scope)` on
    /// success — `var_scope` is the array's instance prefix for aliased loop
    /// vars (submodule foreach). Returns `None` for multi-variable or
    /// unhandled shapes (caller falls back to the synchronous path).
    pub(super) fn foreach_materialize_keys_1d(
        &mut self,
        array: &Expression,
        vars: &[Option<crate::ast::Identifier>],
    ) -> Option<(
        Vec<String>,
        bool,
        Option<String>,
        Option<String>,
        (u32, bool),
    )> {
        if vars.len() != 1 {
            return None;
        }
        // Class-member / collection associative array resolved via
        // expr_assoc_name (handles `obj.assoc_member[k]`, array-element bases,
        // etc.).
        if let Some(an) = self.expr_assoc_name(array) {
            let (keys, is_str) = self.array_iter_keys(&an);
            // A `.size`-shadowed collection (queue/dynamic) may shrink mid-
            // loop; use live-size bounds for it.
            let live = if self.signals.contains_key(&format!("{}.size", an)) {
                Some(an.clone())
            } else {
                None
            };
            let kw = self.assoc_index_width_for(&an).unwrap_or((32, false));
            return Some((keys, is_str, None, live, kw));
        }
        // Direct named array (fixed / dynamic / assoc array or queue).
        if let ExprKind::Ident(hier) = &array.kind {
            let mut name = self.resolve_hier_name(hier);
            if let Some(scoped) = self.instance_assoc_member(&name) {
                name = std::borrow::Cow::Owned(scoped);
            }
            // A bare array name inside a SUBMODULE process resolves under the
            // process's instance scope, like the synchronous foreach path.
            if !self.module.arrays.contains_key(&*name)
                && !self.module.arrays_2d.contains_key(&*name)
                && !self.module.arrays_nd.contains_key(&*name)
                && !self.module.dynamic_arrays.contains(&*name)
                && !self.is_associative_array(&name)
            {
                if let Some(scoped) = self.hint_scoped(&name) {
                    if self.module.arrays.contains_key(&scoped)
                        || self.module.arrays_2d.contains_key(&scoped)
                        || self.module.arrays_nd.contains_key(&scoped)
                        || self.module.dynamic_arrays.contains(&scoped)
                        || self.is_associative_array(&scoped)
                    {
                        name = std::borrow::Cow::Owned(scoped);
                    }
                }
            }
            let var_scope: Option<String> = name.rsplit_once('.').map(|(p, _)| p.to_string());
            // Dispatch by storage type, mirroring the synchronous foreach
            // (exec_statement's Ident arm).
            if self.is_associative_array(&name) {
                let (keys, is_str) = self.array_iter_keys(&name);
                let kw = self.assoc_index_width_for(&name).unwrap_or((32, false));
                return Some((keys, is_str, var_scope, None, kw));
            }
            if self.module.dynamic_arrays.contains(&*name) {
                let size = self.get_queue_size(&name);
                let keys: Vec<String> = (0..size).map(|i| i.to_string()).collect();
                return Some((keys, false, var_scope, Some(name.to_string()), (32, false)));
            }
            if let Some(&(lo, hi, _)) = self.module.arrays.get(&*name) {
                let descending = self.module.descending_arrays.contains(&*name);
                let mut keys: Vec<String> = Vec::new();
                let mut idx = lo;
                while idx <= hi {
                    let v = if descending { hi - (idx - lo) } else { idx };
                    keys.push(v.to_string());
                    idx += 1;
                }
                return Some((keys, false, var_scope, None, (32, false)));
            }
            // Queue / dynamic array surfaced via `.size` shadow only.
            let (keys, is_str) = self.array_iter_keys(&name);
            if keys.is_empty() {
                return None;
            }
            return Some((keys, is_str, var_scope, Some(name.to_string()), (32, false)));
        }
        None
    }

    /// Push a fresh queue-local save frame on entry to a subroutine.
    pub(super) fn push_queue_frame(&mut self) {
        let saves = self.queue_save_pool.pop().unwrap_or_default();
        self.queue_frame_saves.push(saves);
        let dyns = self.local_dyn_pool.pop().unwrap_or_default();
        self.local_dyn.push(dyns);
        self.formal_dyn.push(Vec::new());
    }

    /// Look up the process-unique storage key for a local dynamic-array /
    /// queue / associative-array local declared in the current (or an
    /// enclosing) call frame. Returns `None` if `bare` is not a renamed
    /// local dyn array of this process (i.e. it is a module-level array, a
    /// class property, or not a collection at all) — in which case callers
    /// should use `bare` as-is.
    pub(super) fn dyn_name_lookup(&self, bare: &str) -> Option<&str> {
        // Walk frames innermost-first: a local dyn array declared in an
        // enclosing call frame is visible in nested scopes (matches how
        // inlined blocking calls share the caller's scope). A collection
        // FORMAL (see `bind_queue_param`) shadows a caller's same-named
        // renamed local, and reads as its bare name from a nested call.
        let entry = self.dyn_entry(bare)?;
        if entry.1.len() == entry.0.len() {
            return Some(self.dyn_identity_hit(entry));
        }
        Some(entry.1.as_str())
    }

    /// The innermost rename-table entry for `bare` (see `dyn_name_lookup`).
    #[inline(always)]
    pub(super) fn dyn_entry(&self, bare: &str) -> Option<&(String, String)> {
        for frame in self.local_dyn.iter().rev() {
            // Frames hold at most a handful of entries — a reverse linear
            // scan (later declaration shadows earlier) beats hashing `bare`
            // once per frame (this probe runs for every bare-name resolve).
            for e in frame.iter().rev() {
                if e.0 == bare {
                    return Some(e);
                }
            }
        }
        None
    }

    /// `dyn_name_lookup`'s hit on an identity entry `(name, name)`: in the
    /// innermost frame a collection formal resolves to its storage key (see
    /// `formal_dyn`); anywhere else the bare name stands.
    #[inline(never)]
    pub(super) fn dyn_identity_hit<'a>(&'a self, entry: &'a (String, String)) -> &'a str {
        let in_top = self.local_dyn.last().is_some_and(|t| {
            t.as_ptr_range()
                .contains(&(entry as *const (String, String)))
        });
        if in_top {
            if let Some((_, target)) = self
                .formal_dyn
                .last()
                .and_then(|f| f.iter().rev().find(|(n, _)| *n == entry.0))
            {
                return target;
            }
        }
        entry.1.as_str()
    }

    /// class-perf row 3: split a `\u{1}`-marked LOCAL collection receiver
    /// name (queue / dynamic array declared inside the running compiled
    /// block). Returns the bare name. The marker is compiler-internal — it
    /// cannot occur in user names — and never reaches the storage tables:
    /// every runtime arm strips it before resolving the store.
    pub(super) fn split_local_coll_marker(marked: &str) -> Option<&str> {
        marked.strip_prefix('\u{1}')
    }

    /// class-perf row 3: resolve the per-call storage key of a marked bare
    /// LOCAL collection name through the interpreter's rename map
    /// (innermost frame first) — the exact store the AST path resolves for
    /// the same name. The bare-name fallback covers top-level frames and
    /// the XEZIM_NO_DYN_RENAME kill-switch, where dyn storage is global.
    pub(super) fn local_coll_store(&self, bare: &str) -> String {
        self.dyn_name_lookup(bare).unwrap_or(bare).to_string()
    }

    /// Storage key for `bare` mapped in ONLY the innermost (current) call
    /// frame. Unlike `dyn_name_lookup`, this never peeks at a caller's
    /// frame, so a method body's unqualified name that is ALSO a class
    /// member does not get stolen by an enclosing method's same-named local.
    pub(super) fn dyn_name_lookup_innermost(&self, bare: &str) -> Option<&str> {
        let frame = self.local_dyn.last()?;
        for e in frame.iter().rev() {
            if e.0 == bare {
                if e.1.len() == e.0.len() {
                    return Some(self.dyn_identity_hit(e));
                }
                return Some(e.1.as_str());
            }
        }
        None
    }

    /// Storage key for `bare` mapped in an ENCLOSING (non-innermost) call
    /// frame — the inlined blocking-call scope-sharing fallback used only
    /// after the current frame and the class members have both been
    /// consulted.
    pub(super) fn dyn_name_lookup_enclosing(&self, bare: &str) -> Option<&str> {
        for frame in self.local_dyn.iter().rev().skip(1) {
            for (k, uq) in frame.iter().rev() {
                if k == bare {
                    return Some(uq.as_str());
                }
            }
        }
        None
    }

    /// Mint a fresh process-unique storage key for a local dyn array `bare`
    /// and record the rename in the current call frame. Returns the key. If
    /// no call frame is active (top-level procedural block), returns `bare`
    /// unchanged — a single process can't collide with itself, so the legacy
    /// global-by-bare-name behaviour is preserved there.
    ///
    /// If `bare` was already declared in THIS frame (a re-entering loop body
    /// or a sibling block reusing the name), the previous key's signals are
    /// cleaned up first so the new declaration starts empty — matching the
    /// LRM semantics where each automatic-variable declaration gets fresh
    /// storage. (Nested block-scope shadowing of the same name is not fully
    /// modelled by a flat per-frame map; it is rare in practice.)
    /// §7.4.5: record the pending declared shape of the local just declared
    /// under the storage name `name` it was given, unless `declare_local_dyn`
    /// already did (a block local outside any subroutine keeps its bare name).
    pub(super) fn adopt_local_shape(&mut self, name: &str) {
        if let Some(shape) = self.pending_local_shape.take() {
            self.register_local_shape(name, shape);
        }
    }

    /// Record a local's declared shape under its storage name: a per-call
    /// key (`@l#n`) replaces any earlier one; a bare name never overrides a
    /// same-named module-scope collection's own shape.
    fn register_local_shape(&mut self, name: &str, shape: CollShape) {
        // A one-dimensional local (`int q[$]`) has no rows to type.
        if shape.dims.len() < 2 {
            return;
        }
        self.local_coll_rows_live |= Self::has_queue_rows(&shape);
        self.has_coll_rows = true;
        if name.starts_with('@') {
            self.local_coll_shapes.insert(name.to_string(), shape);
        } else {
            self.module
                .coll_shapes
                .entry(name.to_string())
                .or_insert(shape);
        }
    }

    pub(super) fn declare_local_dyn(&mut self, bare: &str) -> String {
        // Kill-switch for debugging the per-process local-dyn-array rename.
        static NO_DYN_RENAME: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let shape = self.pending_local_shape.take();
        if *NO_DYN_RENAME.get_or_init(|| std::env::var_os("XEZIM_NO_DYN_RENAME").is_some())
            || self.local_dyn.is_empty()
        {
            if let Some(shape) = shape {
                self.register_local_shape(bare, shape);
            }
            return bare.to_string();
        }
        // Clean up a prior same-frame declaration so the new one starts fresh.
        if let Some(old) = self
            .local_dyn
            .last()
            .and_then(|f| f.iter().find(|(k, _)| k == bare))
            .map(|(_, v)| v.clone())
        {
            self.cleanup_dyn_storage(&old);
        }
        let id = self.next_dyn_id;
        self.next_dyn_id += 1;
        let mut key = String::with_capacity(bare.len() + 12);
        key.push('@');
        key.push_str(bare);
        key.push('#');
        Self::push_i64(&mut key, id as i64);
        if let Some(shape) = shape {
            self.register_local_shape(&key, shape);
        }
        let frame = self.local_dyn.last_mut().unwrap();
        if let Some(slot) = frame.iter_mut().find(|(k, _)| k == bare) {
            slot.1 = key.clone();
        } else {
            frame.push((bare.to_string(), key.clone()));
        }
        if let Some((fields, name, prev)) = self.local_elem_layout.take()
            && name == bare
        {
            self.module.packed_struct_fields.insert(key.clone(), fields);
            match prev {
                Some(p) => self.module.packed_struct_fields.insert(name, p),
                None => self.module.packed_struct_fields.remove(&name),
            };
        }
        // §7.4.1: a private array keeps its element's packed geometry.
        // The declaration registered it under the lexical name before the
        // storage rename; subsequent selected accesses use the private key.
        if let Some(dimensions) = self
            .module
            .packed_full_dims
            .get(bare)
            .filter(|d| d.len() > 1)
            .cloned()
        {
            self.module.packed_full_dims.insert(key.clone(), dimensions);
            if let Some(width) = self.module.packed_signal_elem_widths.get(bare).copied() {
                self.module
                    .packed_signal_elem_widths
                    .insert(key.clone(), width);
            }
            if let Some(dt) = self.module.var_decl_types.get(bare).cloned() {
                self.module.var_decl_types.insert(key.clone(), dt);
            }
        }
        key
    }

    /// Release a terminating child's captures (cleaning storage once no child
    /// needs it) and its entries in the activation index `fork_gen_pids`.
    pub(super) fn release_fork_collections(&mut self, pid: usize) {
        if let Some(gens) = self.fork_pid_gens.remove(&pid) {
            for g in gens {
                if let Some(pids) = self.fork_gen_pids.get_mut(&g) {
                    pids.remove(&pid);
                    if pids.is_empty() {
                        self.fork_gen_pids.remove(&g);
                    }
                }
            }
        }
        if let Some(keys) = self.fork_collection_captures.remove(&pid) {
            for key in keys {
                if let Some(n) = self.fork_capture_refs.get_mut(&key) {
                    *n -= 1;
                    if *n == 0 {
                        self.fork_capture_refs.remove(&key);
                    }
                }
                if self.deferred_collection_cleanup.remove(&key) {
                    self.cleanup_dyn_storage(&key);
                }
            }
        }
    }

    /// Remove all runtime signal/registration bookkeeping for a renamed
    /// local-dyn-array storage key (its `.size`, its `[i]` elements, and the
    /// `module.*` collection registrations). Used when a declaration is
    /// re-entered or its call frame is popped.
    pub(super) fn cleanup_dyn_storage(&mut self, key: &str) {
        // Only clean up process-unique (@-prefixed) keys. Bare names are
        // shared across processes in the global `module.*` tables, so
        // removing them here would corrupt a concurrent process's queue.
        // (Identity-mapped formals and module-level arrays are bare.)
        if !key.starts_with('@') {
            return;
        }
        if self.fork_capture_refs.contains_key(key) {
            self.deferred_collection_cleanup.insert(key.to_string());
            return;
        }
        let mut size_key = std::mem::take(&mut self.dyn_key_scratch);
        size_key.clear();
        size_key.push_str(key);
        size_key.push_str(".size");
        // Every element key is indexed under its base (`key`), and a `@`
        // key holds no `[`: the index entry is exactly the `key[` keys.
        let stale: Vec<String> = self.signals.elem_keys(key);
        let has_size = self.signals.contains_key(&size_key);
        for k in stale {
            self.signals.remove(&k);
            self.widths.remove(&k);
        }
        if has_size {
            self.signals.remove(&size_key);
            self.widths.remove(&size_key);
        }
        if self.dynamic_assoc_dims.contains_key(key) {
            self.clear_dynamic_assoc_rows(key, 0);
            self.dynamic_assoc_dims.remove(key);
        }
        self.module.dynamic_arrays.remove(key);
        self.module.arrays.remove(key);
        self.module.arrays_2d.remove(key);
        self.module.arrays_nd.remove(key);
        self.module.associative_arrays.remove(key);
        if !self.local_coll_shapes.is_empty() {
            self.local_coll_shapes.remove(key);
        }
        self.module.assoc_index_widths.remove(key);
        self.module.queue_max_sizes.remove(key);
        self.module.descending_arrays.remove(key);
        self.module.var_decl_types.remove(key);
        self.module.packed_full_dims.remove(key);
        self.module.packed_signal_elem_widths.remove(key);
        self.module.packed_struct_fields.remove(key);
        self.module.array_elem_class.remove(key);
        if !self.local_coll_elems.is_empty()
            && let Some(elems) = self.local_coll_elems.remove(key)
        {
            for en in elems {
                for k in self.signals.keys_with_elem_prefix(&format!("{}[", en)) {
                    self.signals.remove(&k);
                }
                self.signals.remove(&format!("{}.size", en));
                self.module.associative_arrays.remove(&en);
                self.module.assoc_index_widths.remove(&en);
                self.module.assoc_key_type_names.remove(&en);
                self.module.assoc_elem_widths.remove(&en);
                self.module.dynamic_arrays.remove(&en);
                self.module.arrays.remove(&en);
            }
        }
        self.signals.remove(&size_key);
        self.widths.remove(key);
        self.string_signals.remove(key);
        self.dyn_key_scratch = size_key;
    }

    /// Caller-frame state for a queue/dynamic-array LOCAL that a nested call
    /// may clobber. Bare-name locals share the global `self.signals` and
    /// `module.*` registration tables, so a nested method declaring a local
    /// of the same name (even a non-queue, e.g. its own `int p`) overwrites
    /// the caller's bookkeeping. We snapshot the caller's `name.size` /
    /// `name[i]` signals AND its `module.arrays`/`dynamic_arrays`/`widths`/
    /// queue-limit registration, restoring both on return. Without the
    /// registration restore, the caller's queue stays unregistered after the
    /// nested call returns, so its later `delete()` / element reads become
    /// silent no-ops (seen in UVM's callbacks `check_phase`,
    /// whose local `string p[$]` was unregistered by a nested helper's
    /// same-named local).

    /// Snapshot the caller's current queue storage for `name` (its `name.size`
    /// and every `name[...]` element, plus its `module.*` collection
    /// registration) into the top save frame, so it can be restored when the
    /// current subroutine returns. No-op outside a frame, or if this name was
    /// already snapshotted in this frame.
    pub(super) fn snapshot_queue_local(&mut self, name: &str) {
        self.save_queue_local(name, false);
    }

    /// `snapshot_queue_local`; with `take`, the `module.*` collection
    /// registrations are moved out rather than copied (a declaration clears
    /// them next). `true` when the save was made.
    pub(super) fn save_queue_local(&mut self, name: &str, take: bool) -> bool {
        if self.queue_frame_saves.is_empty() {
            return false;
        }
        // A frame saves a handful of names: a linear scan (each name is
        // restored on its own, so the save order is immaterial).
        if self
            .queue_frame_saves
            .last()
            .map(|f| f.iter().any(|(n, _)| n == name))
            .unwrap_or(true)
        {
            return false;
        }
        let mut size_key = std::mem::take(&mut self.queue_key_scratch);
        size_key.clear();
        size_key.push_str(name);
        size_key.push_str(".size");
        // Element keys come from the SignalMap index — no full-map scan.
        let mut signals: Vec<(String, Value)> = self
            .signals
            .elem_keys(name)
            .into_iter()
            .filter_map(|k| self.signals.get(&k).map(|v| (k.clone(), v.clone())))
            .collect();
        if let Some(v) = self.signals.get(&size_key) {
            signals.push((size_key.clone(), v.clone()));
        }
        self.queue_key_scratch = size_key;
        let save = if take {
            QueueLocalSave {
                signals,
                arrays_entry: self.module.arrays.remove(name),
                was_dynamic: self.module.dynamic_arrays.remove(name),
                was_descending: self.module.descending_arrays.remove(name),
                queue_max: self.module.queue_max_sizes.remove(name),
                width: self.widths.get(name).copied(),
                assoc_entry: self.module.associative_arrays.remove(name),
            }
        } else {
            QueueLocalSave {
                signals,
                arrays_entry: self.module.arrays.get(name).copied(),
                was_dynamic: self.module.dynamic_arrays.contains(name),
                was_descending: self.module.descending_arrays.contains(name),
                queue_max: self.module.queue_max_sizes.get(name).copied(),
                width: self.widths.get(name).copied(),
                assoc_entry: self.module.associative_arrays.get(name).copied(),
            }
        };
        let mut key = self.queue_name_pool.pop().unwrap_or_default();
        key.clear();
        key.push_str(name);
        self.queue_frame_saves.last_mut().unwrap().push((key, save));
        true
    }

    /// Pop the top queue-local save frame and restore every snapshotted name:
    /// drop the callee's current `name.size`/`name[...]` entries and re-insert
    /// the caller's saved ones, AND restore the caller's `module.*` collection
    /// registration (a nested call declaring a same-named local would have
    /// removed the caller's queue from `module.arrays`/`dynamic_arrays`).
    pub(super) fn pop_and_restore_queue_frame(&mut self) {
        let Some(frame) = self.queue_frame_saves.pop() else {
            // queue_frame_saves and local_dyn are pushed/popped in sync; if
            // there was no save frame there is no dyn frame either.
            return;
        };
        // The popped frame is owned, so iterating it holds no borrow of
        // `self`.
        let mut frame = frame;
        let mut size_key = std::mem::take(&mut self.queue_key_scratch);
        for (name, save) in frame.drain(..) {
            size_key.clear();
            size_key.push_str(&name);
            size_key.push_str(".size");
            for k in self.signals.elem_keys(&name) {
                self.signals.remove(&k);
            }
            self.signals.remove(&size_key);
            // Restore the caller's collection registration AND signals. A
            // nested method that declared a local of the same name (e.g. its
            // own `int p`) removed the caller's queue from `module.*`; without
            // restoring it, the caller's subsequent queue operations
            // (`delete`, element reads, `.size` fallbacks) silently no-op.
            let QueueLocalSave {
                signals,
                arrays_entry,
                was_dynamic,
                was_descending,
                queue_max,
                width,
                assoc_entry,
            } = save;
            for (k, v) in signals {
                self.signals.insert(k, v);
            }
            match arrays_entry {
                Some(entry) => {
                    self.module.arrays.insert(name.clone(), entry);
                }
                None => {
                    self.module.arrays.remove(&name);
                }
            }
            if was_dynamic {
                self.module.dynamic_arrays.insert(name.clone());
            } else {
                self.module.dynamic_arrays.remove(&name);
            }
            if was_descending {
                self.module.descending_arrays.insert(name.clone());
            } else {
                self.module.descending_arrays.remove(&name);
            }
            match queue_max {
                Some(m) => {
                    self.module.queue_max_sizes.insert(name.clone(), m);
                }
                None => {
                    self.module.queue_max_sizes.remove(&name);
                }
            }
            match width {
                Some(w) => match self.widths.get_mut(name.as_str()) {
                    Some(slot) => *slot = w,
                    None => {
                        self.widths.insert(name.clone(), w);
                    }
                },
                None => {
                    self.widths.remove(&name);
                }
            }
            match assoc_entry {
                Some(k) => {
                    self.module.associative_arrays.insert(name.clone(), k);
                }
                None => {
                    self.module.associative_arrays.remove(&name);
                }
            }
            if self.queue_name_pool.len() < 64 {
                self.queue_name_pool.push(name);
            }
        }
        self.queue_key_scratch = size_key;
        if self.queue_save_pool.len() < 64 && frame.capacity() <= 32 {
            self.queue_save_pool.push(frame);
        }
        // Drop this invocation's process-unique local dyn-array storage so
        // it doesn't leak across the whole simulation (each invocation got
        // its own `@name#id` keys).
        if let Some(mut dyn_frame) = self.local_dyn.pop() {
            for (_bare, key) in &dyn_frame {
                self.cleanup_dyn_storage(key);
            }
            if self.local_dyn_pool.len() < 64 && dyn_frame.capacity() <= 32 {
                dyn_frame.clear();
                self.local_dyn_pool.push(dyn_frame);
            }
        }
        if let Some(formals) = self.formal_dyn.pop() {
            for (_bare, key) in &formals {
                // A `ref` formal's alias (see `bind_queue_param`) names
                // storage an enclosing frame owns: leave it alone.
                if key.starts_with('@')
                    && (self
                        .local_dyn
                        .iter()
                        .any(|f| f.iter().any(|(_, k)| k == key))
                        || self
                            .formal_dyn
                            .iter()
                            .any(|f| f.iter().any(|(_, k)| k == key)))
                {
                    continue;
                }
                self.cleanup_dyn_storage(key);
            }
        }
    }

    /// Names of built-in unpacked-array/queue reduction & query methods that
    /// take the array as their receiver (used to route package-scoped calls
    /// like `pkg::arr.size()` to the array, not the package).
    pub(super) fn is_array_builtin_method(m: &str) -> bool {
        matches!(
            m,
            "size"
                | "len"
                | "sum"
                | "product"
                | "min"
                | "max"
                | "and"
                | "or"
                | "xor"
                | "exists"
                | "num"
        )
    }

    pub(super) fn is_parenless_zero_arg_builtin(m: &str) -> bool {
        matches!(
            m,
            "first"
                | "last"
                | "num"
                | "next"
                | "prev"
                | "name"
                | "size"
                | "len"
                | "pop_front"
                | "pop_back"
                | "sort"
                | "rsort"
                | "reverse"
                | "shuffle"
                | "sum"
                | "product"
                | "and"
                | "or"
                | "xor"
                | "min"
                | "max"
                | "unique"
                | "unique_index"
                | "tolower"
                | "toupper"
                | "atoi"
                | "atohex"
                | "atooct"
                | "atobin"
        )
    }

    /// §18.12: failure restores values and collection sizes, including empty arrays.
    fn restore_std_randomize_targets(
        &mut self,
        scalars: &[(Expression, Value)],
        arrays: &[StdArraySnapshot],
    ) {
        for array in arrays {
            if let Some(size) = array.size {
                // Discard trial elements too: a later growth must not expose them.
                for key in self.signals.elem_keys(&array.name) {
                    self.signals.remove(&key);
                }
                self.set_queue_size(&array.name, size);
            }
            for (i, value) in &array.elems {
                self.set_signal_value_by_name(&format!("{}[{}]", array.name, i), value.clone());
            }
        }
        for (target, value) in scalars {
            self.assign_value(target, value);
        }
    }

    /// `<call> with { constraints }`. For `std::randomize(vars) with {...}` the
    /// listed vars are randomized then narrowed by the inline constraints
    /// (crucially `inside` — riscv-dv selects instructions this way). For
    /// `obj.randomize() with {...}` we randomize the object and then apply the
    /// inline constraints against its properties.
    pub(super) fn eval_randomize_with(
        &mut self,
        call: &Expression,
        constraints: &[crate::ast::decl::ConstraintItem],
    ) -> Value {
        // §18.7: the empty argument parentheses may be omitted. The parser
        // then retains the method reference rather than a Call node; falling
        // through to eval_expr executes randomize but discards its with block.
        if !matches!(call.kind, ExprKind::Call { .. }) {
            let call = Expression::new(
                ExprKind::Call {
                    func: Box::new(call.clone()),
                    args: Vec::new(),
                },
                call.span,
            );
            return self.eval_randomize_with(&call, constraints);
        }
        if let ExprKind::Call { func, args } = &call.kind {
            if let ExprKind::MemberAccess { expr, member } = &func.kind {
                if member.name == "randomize" {
                    // std::randomize(vars) with {...}
                    if let ExprKind::Ident(h) = &expr.kind {
                        if h.path.len() == 1 && h.path[0].name.name == "std" {
                            // §18.12 / §18.11.1: `std::randomize()` with an EMPTY
                            // variable list solves nothing — every name in the
                            // `with` block is a state variable, so the call is a
                            // pure CHECKER. It must report 0 when the constraint
                            // set does not hold under the current state (it
                            // unconditionally reported success before).
                            if args.is_empty() {
                                let ok = self.inline_constraints_satisfied(constraints);
                                return Value::from_u64(if ok { 1 } else { 0 }, 32);
                            }
                            // §18.6.1/§18.12: when randomize() fails, the
                            // variables keep the values they had. The trial
                            // assignments below write the targets, so keep
                            // the integral scalar ones to put back.
                            let mut snapshot: Vec<(Expression, Value)> = Vec::new();
                            // The array targets are written by the same trial
                            // draws, and the solver commits an array element by
                            // element, so a failed call used to leave the
                            // rejected draw behind. Keep every element too.
                            let mut array_snapshot = Vec::new();
                            for a in args {
                                if let Some(nm) = self.array_operand_name(a) {
                                    let size = (self.module.dynamic_arrays.contains(&nm)
                                        || self
                                            .signals
                                            .contains_key(&Self::name_with_suffix(&nm, ".size")))
                                    .then(|| self.get_queue_size(&nm));
                                    let (lo, hi) = if let Some(n) = size {
                                        (0, n as i64 - 1)
                                    } else if let Some(&(lo, hi, _)) = self.module.arrays.get(&nm) {
                                        (lo.min(hi), lo.max(hi))
                                    } else {
                                        continue;
                                    };
                                    let elems = (lo..=hi)
                                        .filter_map(|i| {
                                            self.get_signal_value_by_name(&format!("{nm}[{i}]"))
                                                .map(|v| (i, v))
                                        })
                                        .collect();
                                    array_snapshot.push(StdArraySnapshot {
                                        name: nm,
                                        size,
                                        elems,
                                    });
                                } else if let Some(fields) = self.expand_struct_target(a) {
                                    for (_, field) in fields {
                                        let value = self.eval_expr(&field);
                                        snapshot.push((field, value));
                                    }
                                } else {
                                    snapshot.push((a.clone(), self.eval_expr(a)));
                                }
                            }
                            self.exec_std_randomize(args);
                            // Collect the randomize target lvalues by name. A
                            // struct arg expands to its per-field lvalues so
                            // field-level constraints (`pkt.f < N`) match.
                            let mut targets: Vec<(String, Expression)> = Vec::new();
                            for a in args {
                                // §18.4: a PACKED struct/union is a raw integral
                                // type — it is also a target in its own right, so
                                // a whole-aggregate constraint (`p_struct ==
                                // 8'hA5`) binds and drives every field through the
                                // packed layout. Its fields stay targets too, so
                                // field-level constraints keep working.
                                if self.is_packed_struct_var(a) {
                                    if let ExprKind::Ident(hh) = &a.kind {
                                        targets.push((
                                            self.resolve_hier_name(hh).into_owned(),
                                            a.clone(),
                                        ));
                                    }
                                }
                                if let Some(fields) = self.expand_struct_target(a) {
                                    targets.extend(fields);
                                } else if let ExprKind::Ident(hh) = &a.kind {
                                    targets
                                        .push((self.resolve_hier_name(hh).into_owned(), a.clone()));
                                } else if let Some(n) = self.constraint_operand_name(a) {
                                    // A class property named through its
                                    // handle (`obj.member`, `o.sub.member`).
                                    targets.push((n, a.clone()));
                                }
                            }
                            // Guards decided by state variables alone pick
                            // their branch now (see `fold_state_guards`).
                            let rand_roots: HashSet<String> = args
                                .iter()
                                .filter_map(Self::randomize_target_root)
                                .collect();
                            let folded = self.fold_state_guards(constraints, &rand_roots);
                            let constraints: &[crate::ast::decl::ConstraintItem] = &folded;
                            // §18.5.8.1: sizes a constraint reads are solved
                            // first, then the elements (see
                            // `std_randomize_sized`).
                            if let Some(ok) = self.std_randomize_sized(constraints, &targets) {
                                if !ok {
                                    self.rand_fail_warn_std(constraints);
                                    self.restore_std_randomize_targets(&snapshot, &array_snapshot);
                                }
                                return Value::from_u64(ok as u64, 32);
                            }
                            let mut satisfied = false;
                            for attempt in 0..10 {
                                if attempt > 0 {
                                    // Previous attempt left a (modeled)
                                    // constraint violated — reseed and retry.
                                    self.exec_std_randomize(args);
                                }
                                // §18.4: a dynamic-array target whose size is
                                // only bounded (`a.size() >= n`, `inside`) gets
                                // a size inside the bounds; an `==` is left to
                                // the item itself.
                                self.size_bounded_dyn_targets(constraints, &targets);
                                // §18.5.12 (issue #4): COUPLED affine constraints
                                // (`A + B < 1000`, `A < B`, `A + B == 50`) tie two
                                // targets together, so per-target narrowing below
                                // narrows nothing. Solve them sequentially first
                                // and skip those targets in the independent loop.
                                let mut cvar_names: Vec<String> = Vec::with_capacity(targets.len());
                                let mut cvar_dom: Vec<(i128, i128)> =
                                    Vec::with_capacity(targets.len());
                                let mut cvar_ws: Vec<(u32, bool)> =
                                    Vec::with_capacity(targets.len());
                                for (name, lv) in &targets {
                                    let cur = self.eval_expr(lv);
                                    let w = cur.width.max(1);
                                    let s = cur.is_signed;
                                    let ww = w.min(127);
                                    let (lo, hi): (i128, i128) = if s {
                                        (
                                            -(1i128 << ww.saturating_sub(1)),
                                            (1i128 << ww.saturating_sub(1)) - 1,
                                        )
                                    } else {
                                        // §18.4: the unsigned domain is
                                        // [0, 2^ww - 1]. `ww` is clamped to 127
                                        // above, and at exactly 127 the shift
                                        // form breaks: `1i128 << 127` IS
                                        // i128::MIN (bit 127 is the sign bit),
                                        // so `- 1` overflows and panics in debug
                                        // builds. That bound is i128::MAX.
                                        (
                                            0,
                                            if ww == 127 {
                                                i128::MAX
                                            } else {
                                                (1i128 << ww) - 1
                                            },
                                        )
                                    };
                                    cvar_names.push(name.clone());
                                    cvar_dom.push((lo, hi));
                                    cvar_ws.push((w, s));
                                }
                                let all_unsigned = cvar_ws.iter().all(|(_, s)| !*s);
                                self.constraint_cmp_unsigned = all_unsigned;
                                let coupled = self.solve_coupled_affine(
                                    constraints,
                                    &cvar_names,
                                    &cvar_dom,
                                    attempt,
                                    None,
                                );
                                self.constraint_cmp_unsigned = false;
                                let mut owned_names: HashSet<String> = HashSet::default();
                                if let Some(pairs) = coupled {
                                    for (col, val) in pairs {
                                        let (name, lv) = &targets[col];
                                        let (w, s) = cvar_ws[col];
                                        let masked = if w >= 64 {
                                            val as u64
                                        } else {
                                            (val as u64) & ((1u64 << w) - 1)
                                        };
                                        let mut v = Value::from_u64(masked, w);
                                        v.is_signed = s;
                                        owned_names.insert(name.clone());
                                        let lv = lv.clone();
                                        self.assign_value(&lv, &v);
                                    }
                                }
                                // Relational items (`t > e`, `t <= e`, …): narrow a
                                // [lo, hi] interval per target across the WHOLE
                                // constraint set, then pick uniformly inside it.
                                // Item-at-a-time re-picking would ping-pong between
                                // `a > 100` and `a < 200`; interval narrowing
                                // converges in one shot (same approach as the
                                // real-rand bounds scan in exec_randomize).
                                for (name, lv) in &targets {
                                    // A coupled target already has its value.
                                    if owned_names.contains(name) {
                                        continue;
                                    }
                                    let cur = self.eval_expr(lv);
                                    let width = cur.width.max(1);
                                    let signed = cur.is_signed;
                                    let (mut lo, mut hi): (i64, i64) = if signed {
                                        if width >= 64 {
                                            (i64::MIN, i64::MAX)
                                        } else {
                                            (-(1i64 << (width - 1)), (1i64 << (width - 1)) - 1)
                                        }
                                    } else if width >= 63 {
                                        (0, i64::MAX)
                                    } else {
                                        (0, (1i64 << width) - 1)
                                    };
                                    // §11.8.1: an unsigned target makes each of
                                    // its relational comparisons unsigned, so a
                                    // bound literal is read by its unsigned value
                                    // (see `constraint_cmp_unsigned`).
                                    self.constraint_cmp_unsigned = !signed;
                                    let mut any = false;
                                    for c in constraints {
                                        self.narrow_relational_bounds(
                                            c, name, &targets, &mut lo, &mut hi, &mut any,
                                        );
                                    }
                                    self.constraint_cmp_unsigned = false;
                                    if any && lo <= hi {
                                        use rand::Rng;
                                        let pick = if lo == hi {
                                            lo
                                        } else {
                                            self.cur_rng().gen_range(lo..=hi)
                                        };
                                        let masked = if width >= 64 {
                                            pick as u64
                                        } else {
                                            (pick as u64) & ((1u64 << width) - 1)
                                        };
                                        let mut v = Value::from_u64(masked, width);
                                        v.is_signed = signed;
                                        self.assign_value(lv, &v);
                                    }
                                }
                                for _ in 0..8 {
                                    for c in constraints {
                                        self.apply_inline_constraint(c, &targets);
                                    }
                                }
                                // §18.5.5: enforce `unique {arr}` LAST — the
                                // per-element `inside` picks above are what
                                // define each element's legal domain, and the
                                // repair draws the replacement from it.
                                self.enforce_inline_unique(constraints);
                                // §18.5.7/§18.3 backstop: keep the assignment
                                // only when every MODELED constraint holds —
                                // the per-element repair can dead-end (e.g. an
                                // ascending cross-element chain that exhausts
                                // its headroom); a fresh seed usually escapes.
                                if self.inline_constraints_satisfied(constraints) {
                                    satisfied = true;
                                    break;
                                }
                                // The per-variable repair above cannot satisfy
                                // constraints that tie several variables
                                // together (`sum()`, chains, guarded
                                // relations): solve them jointly, with the
                                // array sizes this pass chose.
                                if matches!(
                                    self.std_randomize_joint(constraints, &targets),
                                    rand_csp::CspOutcome::Sat
                                ) {
                                    satisfied = true;
                                    break;
                                }
                            }
                            // §18.11: `std::randomize` returns 1 only if a
                            // consistent assignment was actually found. Returning
                            // 1 unconditionally reported success for an
                            // unsatisfiable set (`v >= 100; v <= 5;`) and for a
                            // narrow band the i64 interval solver cannot reach.
                            if !satisfied {
                                self.rand_fail_warn_std(constraints);
                                self.restore_std_randomize_targets(&snapshot, &array_snapshot);
                            }
                            return Value::from_u64(if satisfied { 1 } else { 0 }, 32);
                        }
                    }
                    // §18.7 `obj.randomize() with {…}` — the inline constraints
                    // join the class's constraint set for this call, so they
                    // take part in sizing, element solving and the acceptance
                    // check rather than being patched on afterwards.
                    let handle = self.eval_expr(expr).to_u64().unwrap_or(0) as usize;
                    if handle != 0 {
                        return self.randomize_object_with(
                            handle,
                            Self::plain_ident_name(expr),
                            args,
                            constraints,
                        );
                    }
                    return Value::zero(32);
                }
            }
            // §18.7: a bare `randomize(…) with {…}` in a class method is
            // `this.randomize(…) with {…}`; evaluating the plain call dropped
            // the inline constraints.
            if let ExprKind::Ident(h) = &func.kind {
                if h.root.is_none()
                    && h.path.len() == 1
                    && h.path[0].selects.is_empty()
                    && h.path[0].name.name == "randomize"
                {
                    if let Some(Some(this_h)) = self.this_stack.last().copied() {
                        return self.randomize_object_with(this_h, None, args, constraints);
                    }
                }
            }
        }
        // Fallback: just evaluate the underlying call.
        self.eval_expr(call)
    }

    /// §18.7 `obj.randomize(args) with {…}` on the object `handle`;
    /// `receiver` is the plain name `obj` was written as.
    pub(super) fn randomize_object_with(
        &mut self,
        handle: usize,
        receiver: Option<String>,
        args: &[Expression],
        constraints: &[crate::ast::decl::ConstraintItem],
    ) -> Value {
        // §18.7: in a module instance, a name the object declares binds to
        // the object even where the instance declares it too.
        let unscoped = self.unscope_object_member_names(handle, constraints);
        let constraints: &[crate::ast::decl::ConstraintItem] =
            unscoped.as_deref().unwrap_or(constraints);
        // §18.7/§18.7.1: caller-scope operands (the caller's locals and class
        // members, `local::` names) are state variables of the solve — bind
        // them now, while `this` is still the caller.
        let items_vec = self.freeze_caller_scope_refs(handle, receiver.clone(), constraints);
        let items: &[crate::ast::decl::ConstraintItem] =
            items_vec.as_deref().unwrap_or(constraints);
        // §18.11 `obj.randomize(null) with {...}` — in-line constraint
        // CHECKER: nothing is randomized, the current values are checked
        // against the constraints.
        if Self::is_randomize_check_args(args) {
            return self.exec_randomize_check(handle, items);
        }
        // §18.11: `obj.randomize(a, b) with {…}` solves only the named
        // members; every other one is a state variable of the inline block.
        let subset: Option<HashSet<String>> = args
            .iter()
            .map(Self::plain_ident_name)
            .collect::<Option<HashSet<String>>>()
            .filter(|s| !s.is_empty());
        // §18.7: the inline constraints join the class's set for this call
        // (sizing, element solving, acceptance).
        self.begin_randomize_budget();
        self.obj_rng_stack.push(handle);
        // §18.7: names in the inline block resolve in the object's scope
        // first, so a property named like the receiver hides it there.
        let receiver = receiver.filter(|r| {
            !self
                .heap
                .get(handle)
                .and_then(|o| o.as_ref())
                .is_some_and(|i| i.properties.contains_key(r))
        });
        let prev_receiver = std::mem::replace(&mut self.rand_receiver, receiver);
        // `exec_randomize_inner` pushes the solve frame next, so its
        // `method_local_base` entry is the current frame count.
        let prev_base = std::mem::replace(&mut self.rand_receiver_base, self.local_stack.len());
        let prev_subset = std::mem::replace(&mut self.randomize_subset, subset);
        let inline_src_file = (!items.is_empty())
            .then(|| self.current_src_file())
            .flatten();
        let r = self.exec_randomize_inner(handle, items, inline_src_file);
        self.randomize_subset = prev_subset;
        self.rand_receiver_base = prev_base;
        self.rand_receiver = prev_receiver;
        self.obj_rng_stack.pop();
        self.end_randomize_budget();
        r
    }

    /// §18.11 — is this a `randomize(null)` call (the in-line constraint
    /// checker) rather than an ordinary `randomize()` / `randomize(x, y)`?
    pub(super) fn is_randomize_check_args(args: &[Expression]) -> bool {
        args.len() == 1 && matches!(args[0].kind, ExprKind::Null)
    }

    /// The constraint blocks that are ACTIVE for `handle`: every block in the
    /// class hierarchy, with a derived block replacing a same-named inherited
    /// one (§18.5.4) and blocks turned off by `constraint_mode(0)` dropped
    /// (§18.13).
    pub(super) fn active_constraints(&self, handle: usize) -> Vec<ClassConstraint> {
        let mut out: Vec<ClassConstraint> = Vec::new();
        let mut seen: HashSet<String> = HashSet::default();
        let disabled = self
            .constraint_mode_disabled
            .get(&handle)
            .cloned()
            .unwrap_or_default();
        let all_off = disabled.contains("*");
        let mut cur = self
            .heap
            .get(handle)
            .and_then(|x| x.as_ref())
            .map(|i| i.class_name.clone());
        while let Some(cn) = cur {
            if let Some(cd) = self.module.classes.get(&cn) {
                for (nm, con) in cd.constraints.iter() {
                    // §18.5.11: a static block's mode is the class-wide flag
                    // a whole-object `constraint_mode(0)` also clears.
                    let static_off = con.is_static
                        && !self.static_constraint_disabled.is_empty()
                        && self
                            .static_constraint_disabled
                            .contains(&(cn.clone(), nm.clone()));
                    if all_off || static_off || disabled.contains(nm) {
                        seen.insert(nm.clone());
                        continue;
                    }
                    if seen.insert(nm.clone()) {
                        out.push(con.clone());
                    }
                }
                cur = cd.extends.clone();
            } else {
                break;
            }
        }
        out
    }

    /// §18.11 / §18.11.1 — the in-line constraint CHECKER, `obj.randomize(null)`
    /// (optionally `with { ... }`). No random variable is assigned: every rand
    /// variable is treated as a state variable and the object's CURRENT values
    /// are checked against the active class constraints plus the inline ones.
    /// Returns 1 when they all hold and 0 as soon as one is violated.
    /// `pre_randomize`/`post_randomize` are NOT called (§18.11: no
    /// randomization takes place). `soft` constraints can never fail the check
    /// (the solver would simply drop them, §18.5.14).
    pub(super) fn exec_randomize_check(
        &mut self,
        handle: usize,
        inline: &[crate::ast::decl::ConstraintItem],
    ) -> Value {
        // The object's scope, as for a solve (see `exec_randomize_inner`).
        self.push_local_frame(HashMap::default());
        self.method_local_base.push(self.local_stack.len() - 1);
        let r = self.randomize_check_items(handle, inline);
        self.method_local_base.pop();
        self.pop_local_frame();
        r
    }
}
