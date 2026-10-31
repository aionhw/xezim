//! Simulator class specialization.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// §8.25 / A.4.1.1: a class specialization written inside an `extends`
    /// argument with named parameter assignments (`base #(.REQ(REQ))` in
    /// `extends wrap #(.BASE(base #(.REQ(REQ))))`), spelled by POSITION in
    /// that class's `param_order`, the way `place_named_extends_args` places
    /// the outer list: an argument projected out of the binding (`BASE#[i]`)
    /// is read by position, and read `.REQ(REQ)` as the first argument. A
    /// slot the list does not name takes the class's default, folded against
    /// the slots before it; a trailing one is left off. Nested lists are
    /// placed first. `None` when there is nothing to place, or a default
    /// cannot be spelled here (the text is then kept as written).
    pub(super) fn positional_spec_text(&self, frag: &str) -> Option<String> {
        let f = frag.trim();
        let open = f.find("#(")?;
        let inner = f[open + 2..].strip_suffix(')')?;
        let head = f[..open].trim();
        let mut args: Vec<String> = Self::split_spec_args(inner)
            .into_iter()
            .map(|a| a.trim().to_string())
            .collect();
        let mut changed = false;
        for a in args.iter_mut() {
            let (name, value) = match Self::named_spec_arg(a) {
                Some((n, v)) => (Some(n.to_string()), v.to_string()),
                None => (None, a.clone()),
            };
            if let Some(placed) = self.positional_spec_text(&value) {
                *a = match name {
                    Some(n) => format!(".{}({})", n, placed),
                    None => placed,
                };
                changed = true;
            }
        }
        // A.4.1.1: a list is all-named or all-ordered.
        if args.is_empty() || !args.iter().all(|a| Self::named_spec_arg(a).is_some()) {
            return changed.then(|| format!("{}#({})", head, args.join(",")));
        }
        let cd = self.module.classes.get(head)?;
        let order = &cd.param_order;
        let mut slots: Vec<Option<String>> = vec![None; order.len()];
        for a in &args {
            let (n, v) = Self::named_spec_arg(a)?;
            let i = order.iter().position(|p| p == n)?;
            slots[i] = (!v.is_empty()).then(|| v.to_string());
        }
        let used = slots.iter().rposition(|s| s.is_some()).map_or(0, |i| i + 1);
        let mut placed: Vec<String> = Vec::with_capacity(used);
        for (i, slot) in slots.into_iter().take(used).enumerate() {
            let v = match slot {
                Some(v) => v,
                None => self.param_default_fragment(cd, i, &placed)?,
            };
            placed.push(v);
        }
        Some(format!("{}#({})", head, placed.join(",")))
    }

    /// `positional_spec_text` over each `extends` argument of `cname`.
    pub(super) fn place_nested_named_spec_args(&mut self, cname: &str) {
        let Some(cd) = self.module.classes.get(cname) else {
            return;
        };
        if !cd.extends_type_args.iter().any(|a| a.contains("#(")) {
            return;
        }
        let texts: Vec<Option<String>> = cd
            .extends_type_args
            .iter()
            .map(|a| self.positional_spec_text(a))
            .collect();
        if texts.iter().all(Option::is_none) {
            return;
        }
        if let Some(cd) = self
            .module
            .classes
            .get_mut(cname)
            .map(std::sync::Arc::make_mut)
        {
            for (a, t) in cd.extends_type_args.iter_mut().zip(texts) {
                if let Some(t) = t {
                    *a = t;
                }
            }
        }
    }

    /// §8.25 / A.4.1.1: `class d extends base #(.W(X));` names the parameters it
    /// sets. Every walk over `extends_type_args` / `extends_args` reads them
    /// by POSITION in the base's `param_order`, so place each named value at
    /// its parameter's slot once, here: a named list used to bind `.W(X)` to
    /// the base's first parameter. A slot the list does not name keeps the
    /// base's default (§6.20.2), folded against the slots before it; a
    /// trailing one is left off. Both lists come out aligned.
    pub(super) fn place_named_extends_args(&mut self, cname: &str) {
        let Some(cd) = self.module.classes.get(cname) else {
            return;
        };
        if !cd
            .extends_type_args
            .iter()
            .any(|a| Self::named_spec_arg(a).is_some())
        {
            return;
        }
        let Some(pcd) = cd
            .extends
            .as_deref()
            .and_then(|p| self.module.classes.get(p))
        else {
            return;
        };
        // A.4.1.1: a list is all-named or all-ordered.
        if !cd
            .extends_type_args
            .iter()
            .all(|a| Self::named_spec_arg(a).is_some())
        {
            return;
        }
        let order = &pcd.param_order;
        let mut texts: Vec<Option<String>> = vec![None; order.len()];
        for a in &cd.extends_type_args {
            if let Some((n, v)) = Self::named_spec_arg(a) {
                if let Some(i) = order.iter().position(|p| p == n) {
                    texts[i] = Some(v.to_string());
                }
            }
        }
        let mut exprs: Vec<Option<Expression>> = vec![None; order.len()];
        for e in &cd.extends_args {
            if let ExprKind::NamedArg {
                name,
                expr: Some(x),
            } = &e.kind
            {
                if let Some(i) = order.iter().position(|p| *p == name.name) {
                    exprs[i] = Some((**x).clone());
                }
            }
        }
        let used = texts.iter().rposition(|t| t.is_some()).map_or(0, |i| i + 1);
        let ident = |name: &str| {
            Expression::new(
                ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                    root: None,
                    path: vec![crate::ast::expr::HierPathSegment {
                        name: crate::ast::Identifier {
                            name: name.to_string(),
                            span: crate::ast::Span::dummy(),
                        },
                        selects: Vec::new(),
                    }],
                    span: crate::ast::Span::dummy(),
                    cached_signal_id: std::cell::Cell::new(None),
                    cached_resolved_name: std::cell::OnceCell::new(),
                }),
                crate::ast::Span::dummy(),
            )
        };
        let mut new_texts: Vec<String> = Vec::with_capacity(used);
        let mut new_exprs: Vec<Expression> = Vec::with_capacity(used);
        for i in 0..used {
            let (t, e) = match (texts[i].take(), exprs[i].take()) {
                (Some(t), e) => {
                    let e = e.unwrap_or_else(|| ident(&t));
                    (t, e)
                }
                (None, _) => {
                    let t = self
                        .param_default_fragment(pcd, i, &new_texts)
                        .unwrap_or_else(|| order[i].clone());
                    let (neg, body) = match t.strip_prefix('-') {
                        Some(b) => (true, b),
                        None => (false, t.as_str()),
                    };
                    let e = match Self::parse_spec_number(body) {
                        Some(n) => {
                            let lit =
                                Expression::new(ExprKind::Number(n), crate::ast::Span::dummy());
                            if neg {
                                Expression::new(
                                    ExprKind::Unary {
                                        op: UnaryOp::Minus,
                                        operand: Box::new(lit),
                                    },
                                    crate::ast::Span::dummy(),
                                )
                            } else {
                                lit
                            }
                        }
                        None => ident(&t),
                    };
                    (t, e)
                }
            };
            new_texts.push(t);
            new_exprs.push(e);
        }
        if let Some(cd) = self
            .module
            .classes
            .get_mut(cname)
            .map(std::sync::Arc::make_mut)
        {
            cd.extends_type_args = new_texts;
            cd.extends_args = new_exprs;
        }
    }

    /// Whether `name` is a `dep_base` class (its base depends on its
    /// parameters).
    pub(super) fn is_dep_class(&self, name: &str) -> bool {
        if self.dep_names.len() <= 8 {
            self.dep_names.iter().any(|d| d == name)
        } else {
            self.dep_base.contains_key(name)
        }
    }

    /// The class a specialized class entry was made from (`"wrap_c<d>"` ->
    /// `"wrap_c"`); any other name is its own origin.
    pub(super) fn class_origin<'a>(&'a self, name: &'a str) -> &'a str {
        // Every specialized entry's name ends in `>`; anything else is its
        // own origin without a lookup (hot: hierarchy walks call this).
        if self.spec_clone_origin.is_empty() || !name.ends_with('>') {
            return name;
        }
        self.class_origin_slow(name)
    }

    #[cold]
    #[inline(never)]
    pub(super) fn class_origin_slow<'a>(&'a self, name: &'a str) -> &'a str {
        self.spec_clone_origin
            .get(name)
            .map(|s| s.as_str())
            .unwrap_or(name)
    }

    /// A type fragment naming a class (`derived_c`, `pbase#(8)`, or a typedef
    /// of either) as `(class, specialization args)`.
    #[inline(never)]
    /// A type argument naming a typedef of a class specialization
    /// (`my_s` for `typedef sbase #(my_cfg) my_s;`), spelled out as that
    /// specialization (`sbase#(my_cfg)`); any other argument as written.
    pub(super) fn spell_typedef_spec(&self, arg: &str) -> String {
        let a = arg.trim();
        if a.contains('#') || self.module.classes.contains_key(a) {
            return a.to_string();
        }
        match self.resolve_typedef_spec(a) {
            Some((b, sig)) if !sig.trim().is_empty() && self.module.classes.contains_key(&b) => {
                format!("{}#({})", b, sig)
            }
            _ => a.to_string(),
        }
    }

    pub(super) fn type_base_frag_class(&self, frag: &str) -> Option<(String, Vec<String>)> {
        let frag = frag.trim();
        let (b, args) = Self::strip_class_specialization(frag);
        let args: Vec<String> = args
            .map(|a| Self::split_spec_args(&a))
            .unwrap_or_default()
            .into_iter()
            .map(|a| a.trim().to_string())
            .collect();
        if self.module.classes.contains_key(&b) {
            return Some((b, args));
        }
        if !args.is_empty() {
            return None;
        }
        if let Some((rb, rsig)) = self.resolve_typedef_spec(&b) {
            if self.module.classes.contains_key(&rb) {
                let a = Self::split_spec_args(&rsig)
                    .into_iter()
                    .map(|a| a.trim().to_string())
                    .collect();
                return Some((rb, a));
            }
        }
        self.resolve_typeref_class_name_str(&b)
            .filter(|r| self.module.classes.contains_key(r))
            .map(|r| (r, Vec::new()))
    }

    /// The base link `(base class, base args)` of `dep_base` class `cname`
    /// under the specialization fragments `args` (positional; a missing
    /// position takes its declared default, §6.20.2). The base class is an
    /// ORIGIN name; `spec_class_name` maps it to its specialized entry.
    #[inline(never)]
    pub(super) fn spec_base_link(
        &self,
        cname: &str,
        args: &[String],
    ) -> Option<(String, Vec<String>)> {
        let (ext, ext_args) = self.dep_base.get(cname)?;
        let cd = self.module.classes.get(cname)?;
        let order = &cd.param_order;
        let mut bound: Vec<String> = Vec::with_capacity(order.len());
        for i in 0..order.len() {
            let v = match args.get(i).map(|a| a.trim()).filter(|a| !a.is_empty()) {
                Some(a) => a.to_string(),
                None => match self.param_default_fragment(cd, i, &bound) {
                    Some(v) => v,
                    None => break,
                },
            };
            bound.push(v);
        }
        let subst = |s: &str| {
            let mut out = s.trim().to_string();
            for (j, p) in order.iter().enumerate() {
                if let Some(v) = bound.get(j) {
                    out = Self::replace_ident_token(&out, p, v.trim());
                }
            }
            out
        };
        let (b, bargs) = if cd.type_param_names.iter().any(|t| t == ext) {
            let j = order.iter().position(|p| p == ext)?;
            let supplied = args.get(j).is_some_and(|a| !a.trim().is_empty());
            // The declared default keeps its own specialization
            // (`type BASE = uvm_sequence #(uvm_reg_item)`); the textual
            // default fragment drops it.
            let dflt = (!supplied)
                .then(|| self.type_param_default_spec_frag(cd, ext))
                .flatten()
                .map(|d| subst(&d));
            match dflt {
                Some(d) => self.type_base_frag_class(&d)?,
                None => self.type_base_frag_class(bound.get(j)?)?,
            }
        } else {
            let (b, _) = self.type_base_frag_class(ext)?;
            (b, ext_args.iter().map(|a| subst(a)).collect())
        };
        let bargs = if self.class_is_parameterized(&b) && !bargs.is_empty() {
            let canon = self.canonicalize_spec_sig(&b, &bargs.join(","));
            Self::split_spec_args(&canon)
                .into_iter()
                .map(|a| a.trim().to_string())
                .collect()
        } else {
            bargs
        };
        Some((b, bargs))
    }

    /// The type parameter a `dep_base` class extends (`BASE` in `class
    /// wrap_c #(type BASE) extends BASE`); None for one extending a class.
    pub(super) fn dep_type_param(&self, cname: &str) -> Option<String> {
        let (e, _) = self.dep_base.get(cname)?;
        let cd = self.module.classes.get(cname)?;
        cd.type_param_names
            .iter()
            .any(|t| t == e)
            .then(|| e.clone())
    }

    /// The binding of type parameter `p` of `child`, a class on the chain of
    /// an object of leaf entry `leaf` (declared class `origin`) under
    /// construction: from the active specialization for the leaf itself,
    /// else derived up the chain. Only a `BASE#[i]` projection asks.
    #[cold]
    #[inline(never)]
    pub(super) fn child_type_binding(
        &self,
        origin: &str,
        leaf: &crate::compiler::elaborate::ElaboratedClass,
        child: &crate::compiler::elaborate::ElaboratedClass,
        p: &str,
        type_args: Option<&[Expression]>,
    ) -> Option<String> {
        let explicit = type_args.is_some_and(|ta| !ta.is_empty());
        let leaf_sig: Option<String> = if explicit {
            let frags: Vec<String> = type_args
                .unwrap_or_default()
                .iter()
                .filter_map(|e| self.expr_to_spec_fragment(e))
                .collect();
            Some(self.canonicalize_spec_sig(origin, &frags.join(",")))
        } else {
            self.current_spec
                .as_ref()
                .filter(|(b, _)| b == origin)
                .map(|(_, s)| s.clone())
                .or_else(|| {
                    self.class_is_parameterized(origin)
                        .then(|| self.canonicalize_spec_sig(origin, ""))
                })
        };
        let child_sig = if child.name == leaf.name {
            leaf_sig?
        } else {
            match leaf_sig {
                Some(ls) => self.ancestor_spec(origin, &ls, &child.name)?,
                None => self.static_receiver_spec(&leaf.name, &child.name)?.1,
            }
        };
        let j = child.param_order.iter().position(|x| x == p)?;
        Self::spec_arg_at(&child_sig, j)
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .or_else(|| {
                child
                    .type_param_defaults
                    .iter()
                    .find(|(n, _)| n == p)
                    .map(|(_, d)| d.clone())
            })
    }

    /// The `extends` arguments of a class whose base is type parameter `tp`
    /// bound to a specialization of `base`: argument `i` is `tp#[i]`, the
    /// i-th argument of whatever `tp` is bound to (`spec_projection`).
    pub(super) fn base_projection_args(&self, tp: &str, base: &str) -> Vec<String> {
        let n = self
            .module
            .classes
            .get(base)
            .map(|b| b.param_order.len())
            .unwrap_or(0);
        (0..n).map(|i| format!("{}#[{}]", tp, i)).collect()
    }

    /// `P#[i]` in an `extends` argument list: the i-th `#(...)` argument of
    /// the specialization bound to type parameter `P`.
    pub(super) fn spec_projection(frag: &str) -> Option<(&str, usize)> {
        let f = frag.trim();
        if !f.ends_with(']') {
            return None;
        }
        let (p, rest) = f.split_once("#[")?;
        Some((p, rest.strip_suffix(']')?.parse().ok()?))
    }

    /// Argument `i` of specialization `binding` (`pbase#(8)`, 0 -> `8`).
    /// None when the binding does not spell it (its default applies).
    pub(super) fn project_spec_arg(binding: &str, i: usize) -> Option<String> {
        let (_, args) = Self::strip_class_specialization(binding.trim());
        Self::split_spec_args(&args?)
            .get(i)
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
    }

    /// One `extends` argument of a child class carried up a level:
    /// a projection `P#[i]` reads the child's binding of `P` through
    /// `lookup`; a bare name is looked up as a parameter of the child.
    /// None when it is neither (the caller keeps it as written) or the
    /// projected argument is not spelled.
    pub(super) fn carry_extends_arg(
        arg: &str,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Option<String> {
        match Self::spec_projection(arg) {
            Some((p, i)) => lookup(p).and_then(|b| Self::project_spec_arg(&b, i)),
            None => lookup(arg.trim()),
        }
    }

    /// A specialization written as an `extends` argument (`base#(REQ)` in
    /// `class sqr #(type REQ) extends wrap #(.BASE(base #(REQ)))`) with the
    /// child's parameter names inside its `#(...)` replaced by their carried
    /// bindings, in one pass so a binding is never re-substituted (§8.25).
    /// A name written after `.` or `::` is a formal of a named argument
    /// (`base #(.REQ(REQ))`, §23.3.2.2 / §8.25) or a scoped member, not a
    /// reference to the child's parameter, and is kept. Anything else is
    /// returned as written.
    pub(super) fn carry_spec_arg_names(
        arg: &str,
        carried: &std::collections::HashMap<String, String>,
    ) -> String {
        let Some(open) = arg.find("#(") else {
            return arg.to_string();
        };
        let (head, args) = arg.split_at(open);
        let mut out = String::with_capacity(arg.len());
        out.push_str(head);
        let mut ident = String::new();
        // The last non-blank character before the identifier being read.
        let mut before = ' ';
        let flush = |out: &mut String, ident: &mut String, before: char| {
            if !ident.is_empty() {
                let keep = matches!(before, '.' | ':');
                out.push_str(
                    carried
                        .get(ident.as_str())
                        .filter(|_| !keep)
                        .map_or(ident.as_str(), |b| b),
                );
                ident.clear();
            }
        };
        for ch in args.chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' {
                ident.push(ch);
                continue;
            }
            if !ident.is_empty() {
                flush(&mut out, &mut ident, before);
                before = 'a';
            }
            if !ch.is_whitespace() {
                before = ch;
            }
            out.push(ch);
        }
        flush(&mut out, &mut ident, before);
        out
    }

    /// The declared default of type parameter `tp` of `cd` as a fragment
    /// WITH its `#(...)` arguments (`uvm_sequence#(uvm_reg_item)`), when it
    /// is a specialized class type.
    pub(super) fn type_param_default_spec_frag(
        &self,
        cd: &crate::compiler::elaborate::ElaboratedClass,
        tp: &str,
    ) -> Option<String> {
        let (_, dt) = cd.type_param_default_types.iter().find(|(n, _)| n == tp)?;
        let DataType::TypeReference {
            name, type_args, ..
        } = dt
        else {
            return None;
        };
        if type_args.is_empty() {
            return None;
        }
        let frags: Vec<String> = type_args
            .iter()
            .map(|a| self.expr_to_spec_fragment(a))
            .collect::<Option<Vec<_>>>()?;
        Some(format!("{}#({})", name.name.name, frags.join(",")))
    }

    /// The class entry `new` builds for `dep_base` class `cname`: the one of
    /// the specialization given by `#(...)`, else of the active one when it
    /// is this class's (`typedef wrap_c #(derived_c) wd_t; wd_t w = new;`),
    /// else of the default. None when that is `cname` itself.
    #[cold]
    #[inline(never)]
    pub(super) fn spec_entry_for_new(
        &mut self,
        cname: &str,
        type_args: Option<&[Expression]>,
        computed_spec: Option<&(String, String)>,
    ) -> Option<std::sync::Arc<crate::compiler::elaborate::ElaboratedClass>> {
        let explicit = type_args.is_some_and(|ta| !ta.is_empty());
        let active = self
            .current_spec
            .as_ref()
            .filter(|(b, _)| !explicit && *b == cname);
        let args: Vec<String> = active
            .or(computed_spec)
            .map(|(_, sig)| {
                Self::split_spec_args(sig)
                    .into_iter()
                    .map(|a| a.trim().to_string())
                    .collect()
            })
            .unwrap_or_default();
        let key = self.ensure_spec_class(cname, &args);
        (key != cname)
            .then(|| self.module.classes.get(&key).cloned())
            .flatten()
    }

    /// §8.25/§25.9: the leaf-class properties whose declared type is a bare
    /// TYPE PARAMETER of `base` that specialization `sig` binds to
    /// `virtual <iface>` — as `(prop, (iface, modport))` pairs ready for
    /// `virtual_iface_properties`. The binding is per-specialization, so
    /// this cannot be decided at elaboration; `normalize_spec_ws` strips
    /// the space from `virtual I`, so match the normalized spelling and
    /// recover the interface name from the `virtual` prefix.
    pub(super) fn param_vif_props(
        &self,
        cd: &crate::compiler::elaborate::ElaboratedClass,
        sig: &str,
    ) -> Vec<(String, (String, Option<String>))> {
        let frags: Vec<String> = Self::split_spec_args(sig)
            .into_iter()
            .map(|s| s.trim().to_string())
            .collect();
        let mut out = Vec::new();
        for (prop, dt) in &cd.property_types {
            if cd.virtual_iface_properties.contains_key(prop) {
                continue;
            }
            let DataType::TypeReference { name, .. } = dt else {
                continue;
            };
            let tn = name.name.name.as_str();
            let Some(idx) = cd.param_order.iter().position(|p| p == tn) else {
                continue;
            };
            let Some(frag) = frags.get(idx) else {
                continue;
            };
            // A class whose name merely starts with "virtual" (`virtual_cfg`)
            // is not one; only a real interface counts.
            if let Some(vif) = self.vif_of_spec_frag(frag) {
                out.push((prop.clone(), vif));
            }
        }
        // An inherited property typed by a base's parameter that this
        // specialization's own parameter is forwarded to (`class d #(type W)
        // extends base #(W)`).
        let binds: HashMap<String, String> = cd
            .type_param_names
            .iter()
            .filter_map(|tp| {
                let i = cd.param_order.iter().position(|p| p == tp)?;
                let f = frags.get(i).filter(|f| !f.is_empty())?;
                Some((tp.clone(), f.clone()))
            })
            .collect();
        if !binds.is_empty() {
            let own: HashSet<String> = out.iter().map(|(p, _)| p.clone()).collect();
            out.extend(
                self.ancestor_param_vif_props(cd, binds)
                    .into_iter()
                    .filter(|(p, _)| !own.contains(p)),
            );
        }
        out
    }

    /// `(iface, modport)` of a specialization fragment spelling a virtual
    /// interface type (`virtual bus_if`, `virtual I#(8).mp`, or the
    /// `normalize_spec_ws` form `virtualbus_if`), else `None`.
    fn vif_of_spec_frag(&self, frag: &str) -> Option<(String, Option<String>)> {
        let rest = frag.strip_prefix("virtual")?;
        let spaced = rest.starts_with(char::is_whitespace);
        let rest = rest.trim();
        let end = rest
            .find(|c: char| c == '#' || c == '.' || c.is_whitespace())
            .unwrap_or(rest.len());
        let (name, tail) = rest.split_at(end);
        if name.is_empty() || (!spaced && !self.module.interfaces.contains(name)) {
            return None;
        }
        let tail = match tail.rfind(')') {
            Some(close) if tail.starts_with('#') => &tail[close + 1..],
            _ => tail,
        };
        let modport = tail
            .trim()
            .strip_prefix('.')
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty());
        Some((name.to_string(), modport))
    }

    /// §8.25/§25.9: the INHERITED properties of `cname` declared by an
    /// ancestor as a bare type parameter (`V vif;` in `class base #(type V =
    /// virtual bus_if)`) that `cname`'s `extends` chain binds to a virtual
    /// interface — by an explicit `extends base #(virtual bus_if)` or the
    /// parameter's default. Unlike [`Self::param_vif_props`] the binding is
    /// fixed by the class declarations, so it holds for every object of
    /// `cname`; a binding through `cname`'s own parameters depends on the
    /// specialization and is left out (see [`Self::param_vif_props`]).
    /// Static properties keep their declaring class's cell and are left out
    /// too.
    fn inherited_param_vif_props(&self, cname: &str) -> Vec<(String, (String, Option<String>))> {
        match self.module.classes.get(cname) {
            Some(cd) => self.ancestor_param_vif_props(cd, HashMap::default()),
            None => Vec::new(),
        }
    }

    /// The walk of [`Self::inherited_param_vif_props`] up from `leaf`, whose
    /// own type parameters are bound as `leaf_binds` (textual fragments).
    fn ancestor_param_vif_props(
        &self,
        leaf: &crate::compiler::elaborate::ElaboratedClass,
        leaf_binds: HashMap<String, String>,
    ) -> Vec<(String, (String, Option<String>))> {
        let mut out = Vec::new();
        let mut child = leaf;
        let mut seen: HashSet<&str> = child
            .properties
            .keys()
            .chain(child.virtual_iface_properties.keys())
            .map(String::as_str)
            .collect();
        let mut visited: Vec<&str> = vec![child.name.as_str()];
        // The textual binding of each of `child`'s type parameters.
        let mut child_binds = leaf_binds;
        while let Some(pn) = child.extends.as_deref() {
            if visited.contains(&pn) {
                break;
            }
            visited.push(pn);
            let Some(parent) = self.module.classes.get(pn) else {
                break;
            };
            let mut binds: HashMap<String, String> = HashMap::default();
            for tp in &parent.type_param_names {
                let frag = parent
                    .param_order
                    .iter()
                    .position(|p| p == tp)
                    .and_then(|i| child.extends_type_args.get(i))
                    .map(|f| f.trim())
                    .filter(|f| !f.is_empty() && *f != "<unknown>");
                let bound = match frag {
                    Some(f) => child_binds.get(f).cloned().or_else(|| {
                        (!child.type_param_names.iter().any(|t| t == f)).then(|| f.to_string())
                    }),
                    None => parent
                        .type_param_default_types
                        .iter()
                        .find(|(n, _)| n == tp)
                        .and_then(|(_, dt)| crate::elaborate::data_type_to_spec_fragment(dt)),
                };
                if let Some(b) = bound {
                    binds.insert(tp.clone(), b);
                }
            }
            for (prop, dt) in &parent.property_types {
                if seen.contains(prop.as_str())
                    || parent.virtual_iface_properties.contains_key(prop)
                    || parent.static_properties.contains(prop)
                {
                    continue;
                }
                let DataType::TypeReference {
                    name,
                    dimensions,
                    type_args,
                    ..
                } = dt
                else {
                    continue;
                };
                if !dimensions.is_empty() || !type_args.is_empty() {
                    continue;
                }
                if let Some(vif) = binds
                    .get(name.name.name.as_str())
                    .and_then(|b| self.vif_of_spec_frag(b))
                {
                    out.push((prop.clone(), vif));
                }
            }
            seen.extend(parent.properties.keys().map(String::as_str));
            child_binds = binds;
            child = parent.as_ref();
        }
        out
    }

    /// Materialize (once per specialization) a class entry for `(base, sig)`
    /// when `sig` binds a type-parameter-typed property of `base` to a
    /// virtual interface: a copy of `base` with those properties registered
    /// in `virtual_iface_properties` (see [`Self::param_vif_props`]), so
    /// every binding, read, call and `== null` test through the property
    /// works exactly like the spelled-out `virtual <iface> vif;`.
    pub(super) fn ensure_param_vif_class(
        &mut self,
        base: &str,
        sig: &str,
    ) -> Option<std::sync::Arc<crate::compiler::elaborate::ElaboratedClass>> {
        if !self.class_is_parameterized(base) {
            return None;
        }
        let name = format!("{}<{}>", base, sig);
        if let Some(hit) = self.module.classes.get(&name) {
            return self
                .spec_clone_origin
                .contains_key(&name)
                .then(|| hit.clone());
        }
        let orig = self.module.classes.get(base)?.clone();
        let adds = self.param_vif_props(&orig, sig);
        if adds.is_empty() {
            return None;
        }
        let mut c = (*orig).clone();
        c.name = name.clone();
        c.virtual_iface_properties.extend(adds);
        self.module
            .classes
            .insert(name.clone(), std::sync::Arc::new(c));
        self.spec_clone_origin
            .insert(name.clone(), base.to_string());
        self.class_member_names_cell = std::cell::OnceCell::new();
        self.compiled_method_admission = None;
        self.persisted_method_context_hash = None;
        self.module.classes.get(&name).cloned()
    }

    /// The class entry of specialization `(base, sig)` if one was made, else
    /// `base` itself.
    pub(super) fn spec_entry(&self, base: &str, sig: &str) -> String {
        self.spec_entry_cow(base, sig).into_owned()
    }

    /// `spec_entry` without a copy when `base` is its own entry (hot: the
    /// type-parameter and static-key resolvers call it per lookup).
    pub(super) fn spec_entry_cow<'a>(&self, base: &'a str, sig: &str) -> std::borrow::Cow<'a, str> {
        if self.spec_clone_origin.is_empty() || !self.is_dep_class(base) {
            return std::borrow::Cow::Borrowed(base);
        }
        std::borrow::Cow::Owned(self.spec_entry_slow(base, sig))
    }

    #[cold]
    #[inline(never)]
    pub(super) fn spec_entry_slow(&self, base: &str, sig: &str) -> String {
        let key = format!("{}\0{}", base, sig);
        if let Some(hit) = self.spec_entry_cache.borrow().get(&key) {
            return hit.clone();
        }
        let args: Vec<String> = Self::split_spec_args(sig)
            .into_iter()
            .map(|a| a.trim().to_string())
            .collect();
        let n = self.spec_class_name(base, &args);
        // An entry not made yet may be made later: only a settled answer
        // (the class itself, or an existing entry) is remembered.
        if n == base || self.module.classes.contains_key(&n) {
            self.spec_entry_cache.borrow_mut().insert(key, n.clone());
            n
        } else {
            base.to_string()
        }
    }

    /// §8.25: the class entry for specialization `args` of `cname`. A class
    /// whose base does not depend on its parameters, and a specialization
    /// whose base is the default one's class, is the class itself; any other
    /// is `"<cname><<base entry>>"`, made by `ensure_spec_class`. The base's
    /// own arguments are not part of the entry: they reach the base through
    /// the specialization (`BASE#[i]`, see `spec_projection`).
    pub(super) fn spec_class_name(&self, cname: &str, args: &[String]) -> String {
        self.spec_class_name_depth(cname, args, 0)
    }

    #[inline(never)]
    pub(super) fn spec_class_name_depth(
        &self,
        cname: &str,
        args: &[String],
        depth: usize,
    ) -> String {
        if depth > 32 || !self.dep_base.contains_key(cname) {
            return cname.to_string();
        }
        let Some(link) = self.spec_base_link(cname, args) else {
            return cname.to_string();
        };
        let bkey = self.spec_class_name_depth(&link.0, &link.1, depth + 1);
        if self.dep_default.get(cname) == Some(&bkey) {
            return cname.to_string();
        }
        format!("{}<{}>", cname, bkey)
    }

    /// Make (once) the class entry `spec_class_name(cname, args)` names: a
    /// copy of `cname` whose `extends` is this specialization's base, so
    /// every walk up the hierarchy from an instance of it — construction,
    /// inherited properties and their initializers, method lookup, `super.`,
    /// `$cast` — sees the bound base class.
    pub(super) fn ensure_spec_class(&mut self, cname: &str, args: &[String]) -> String {
        self.ensure_spec_class_depth(cname, args, 0)
    }

    #[inline(never)]
    pub(super) fn ensure_spec_class_depth(
        &mut self,
        cname: &str,
        args: &[String],
        depth: usize,
    ) -> String {
        let name = self.spec_class_name(cname, args);
        if name == cname || self.module.classes.contains_key(&name) || depth > 32 {
            return name;
        }
        let Some((b, bargs)) = self.spec_base_link(cname, args) else {
            return cname.to_string();
        };
        let bkey = self.ensure_spec_class_depth(&b, &bargs, depth + 1);
        let Some(orig) = self.module.classes.get(cname) else {
            return cname.to_string();
        };
        let mut c = (**orig).clone();
        c.name = name.clone();
        c.extends = Some(bkey);
        if let Some(tp) = self.dep_type_param(cname) {
            // A type parameter takes no `#(...)`: any `extends BASE(args)`
            // arguments are the base constructor's (§8.7) and stay.
            c.extends_type_args = self.base_projection_args(&tp, &b);
        }
        self.module
            .classes
            .insert(name.clone(), std::sync::Arc::new(c));
        // A newly materialized inheritance chain changes compile-time facts
        // and the design context used by subsequent persistent cache probes.
        self.compiled_method_admission = None;
        self.persisted_method_context_hash = None;
        self.spec_clone_origin
            .insert(name.clone(), cname.to_string());
        name
    }

    /// §8.25 / §8.3: link every class whose base class depends on its
    /// parameters. `class wrap_c #(type BASE = base_c) extends BASE;` stored
    /// `extends = "BASE"`, which names no class, so every walk up the
    /// hierarchy stopped at `wrap_c` — no base constructor, no inherited
    /// properties or methods, an empty `super.`, a failing upcast. The class
    /// entry takes its DEFAULT specialization's base; a specialization that
    /// binds a base of another class gets its own entry (`ensure_spec_class`),
    /// which `new` constructs and an `extends` clause naming it links to.
    #[inline(never)]
    pub(super) fn link_type_param_bases(&mut self) {
        let mut names: Vec<String> = self.module.classes.keys().cloned().collect();
        names.sort();
        // §6.20.3: a type parameter's default that is itself a specialization
        // (`type BASE = uvm_sequence #(uvm_reg_item)`, `type B = box#(T)`)
        // keeps its arguments. The textual default kept only the class name,
        // so the unspecialized class bound the base's DEFAULT specialization.
        for c in &names {
            let Some(cd) = self.module.classes.get(c) else {
                continue;
            };
            let mut fixed: Vec<(usize, String)> = Vec::new();
            for (i, (tp, frag)) in cd.type_param_defaults.iter().enumerate() {
                if frag.contains('#') {
                    continue;
                }
                let Some(full) = self.type_param_default_spec_frag(cd, tp) else {
                    continue;
                };
                // Arguments naming this class's own parameters stay as
                // written: each specialization substitutes them.
                let (b, args) = Self::strip_class_specialization(&full);
                let args = args.unwrap_or_default();
                let names_param = Self::split_spec_args(&args)
                    .iter()
                    .any(|a| cd.param_order.iter().any(|p| p == a.trim()));
                let full = if names_param || !self.module.classes.contains_key(&b) {
                    full
                } else {
                    format!("{}#({})", b, self.canonicalize_spec_sig(&b, &args))
                };
                fixed.push((i, full));
            }
            if fixed.is_empty() {
                continue;
            }
            if let Some(cd) = self.module.classes.get_mut(c).map(std::sync::Arc::make_mut) {
                for (i, full) in fixed {
                    cd.type_param_defaults[i].1 = full;
                }
            }
        }
        // Place named inheritance arguments before substitution so a type
        // parameter reference cannot rewrite the corresponding argument label.
        for c in &names {
            self.place_named_extends_args(c);
            self.place_nested_named_spec_args(c);
        }
        for c in &names {
            let Some(cd) = self.module.classes.get(c) else {
                continue;
            };
            if let Some(e) = &cd.extends {
                if cd.type_param_names.iter().any(|t| t == e) {
                    self.dep_base
                        .insert(c.clone(), (e.clone(), cd.extends_type_args.clone()));
                }
            }
        }
        if self.dep_base.is_empty() {
            return;
        }
        // A class extending such a class with arguments naming its own
        // parameters (`class wrap2_c #(type B) extends wrap_c #(B)`).
        loop {
            let mut added = false;
            for c in &names {
                if self.dep_base.contains_key(c) {
                    continue;
                }
                let Some(cd) = self.module.classes.get(c) else {
                    continue;
                };
                let Some(e) = cd.extends.clone() else {
                    continue;
                };
                let Some((b, _)) = self.type_base_frag_class(&e) else {
                    continue;
                };
                if !self.dep_base.contains_key(&b) {
                    continue;
                }
                let names_param = cd.extends_type_args.iter().any(|a| {
                    cd.param_order
                        .iter()
                        .any(|p| Self::replace_ident_token(a, p, "\u{1}") != *a)
                });
                if names_param {
                    let ea = cd.extends_type_args.clone();
                    self.dep_base.insert(c.clone(), (e, ea));
                    added = true;
                }
            }
            if !added {
                break;
            }
        }
        self.dep_names = self.dep_base.keys().cloned().collect();
        // Default links, bases first.
        let dep: Vec<String> = names
            .iter()
            .filter(|c| self.dep_base.contains_key(*c))
            .cloned()
            .collect();
        for _ in 0..dep.len() + 1 {
            let mut progress = false;
            for c in &dep {
                if self.dep_default.contains_key(c) {
                    continue;
                }
                let Some(link) = self.spec_base_link(c, &[]) else {
                    continue;
                };
                if self.dep_base.contains_key(&link.0) && !self.dep_default.contains_key(&link.0) {
                    continue;
                }
                let bkey = self.ensure_spec_class(&link.0, &link.1);
                let proj = self
                    .dep_type_param(c)
                    .map(|tp| self.base_projection_args(&tp, &link.0));
                if let Some(cd) = self.module.classes.get_mut(c).map(std::sync::Arc::make_mut) {
                    cd.extends = Some(bkey.clone());
                    if let Some(proj) = proj {
                        cd.extends_type_args = proj;
                    }
                }
                self.dep_default.insert(c.clone(), bkey);
                progress = true;
            }
            if !progress {
                break;
            }
        }
        // A class extending a specialization of one (`class leaf_c extends
        // wrap_c #(derived_c)`, or through a typedef of it) links to that
        // specialization's entry.
        for c in &names {
            if self.dep_base.contains_key(c) {
                continue;
            }
            let Some(cd) = self.module.classes.get(c) else {
                continue;
            };
            let Some(e) = cd.extends.clone() else {
                continue;
            };
            let direct = self.module.classes.contains_key(&e);
            let Some((b, targs)) = self.type_base_frag_class(&e) else {
                continue;
            };
            if !self.dep_base.contains_key(&b) {
                continue;
            }
            let args: Vec<String> = if direct {
                cd.extends_type_args.clone()
            } else {
                targs
            };
            // §6.18: a typedef names the type it is declared as. The base
            // entry reads its arguments back from the binding of the type
            // parameter (`BASE#[i]`), which a bare typedef name does not
            // spell — `wrap #(my_s)` with `typedef sbase #(my_cfg) my_s;`
            // gave `sbase` its defaults — so spell the specialization out.
            let args: Vec<String> = args.iter().map(|a| self.spell_typedef_spec(a)).collect();
            let args = if args.is_empty() {
                args
            } else {
                Self::split_spec_args(&self.canonicalize_spec_sig(&b, &args.join(",")))
                    .into_iter()
                    .map(|a| a.trim().to_string())
                    .collect()
            };
            let key = self.ensure_spec_class(&b, &args);
            if let Some(cd) = self.module.classes.get_mut(c).map(std::sync::Arc::make_mut) {
                if (!direct && cd.extends_type_args.is_empty()) || (direct && !args.is_empty()) {
                    cd.extends_type_args = args;
                }
                cd.extends = Some(key);
            }
        }
        let mut found: Vec<String> = Vec::new();
        let typedefs = self.module.typedef_types.iter().chain(
            self.module
                .classes
                .values()
                .flat_map(|cd| cd.typedef_targets.iter()),
        );
        for (t, dt) in typedefs {
            // Only a typedef naming a `dep_base` class with arguments can
            // qualify; resolving every typedef of the design is not cheap.
            let DataType::TypeReference {
                name, type_args, ..
            } = dt
            else {
                continue;
            };
            if type_args.is_empty() || !self.dep_base.contains_key(&name.name.name) {
                continue;
            }
            if self
                .spec_from_typedef_dt(dt)
                .is_some_and(|(b, _)| self.dep_base.contains_key(&b))
            {
                found.push(t.clone());
            }
        }
        self.dep_typedefs.extend(found);
    }

    /// Break any cycle in the class `extends` graph. A self- or mutually-
    /// referential `extends` (which can arise from a parameterized class
    /// whose base resolves to the same name) would make every ancestor-
    /// chain walk loop forever. Walk each class's chain; the first edge
    /// that revisits an already-seen class is cleared to `None`.
    pub(super) fn sanitize_class_hierarchy(&mut self) {
        self.link_type_param_bases();
        let names: Vec<String> = self.module.classes.keys().cloned().collect();
        // Resolve each `extends` base that is a typedef ALIAS to its concrete
        // class key. `class d extends simple_lib` where `simple_lib` is
        // `typedef base#(bit) simple_lib` stores `extends = "simple_lib"`, which
        // is NOT a `module.classes` key — every hierarchy walk (method lookup,
        // inherited-field registration, property resolution) then stops one
        // hop early and loses the base's members/methods. Rewrite it to the
        // concrete key (`"base"`) so the walks see the real ancestor.
        for cname in &names {
            let resolved_extends;
            let mut resolved_args: Vec<String> = Vec::new();
            {
                let cd = self.module.classes.get(cname);
                resolved_extends = cd.and_then(|cd| cd.extends.clone()).and_then(|e| {
                    if self.module.classes.contains_key(&e) {
                        Some(e)
                    } else {
                        self.resolve_typeref_class_name_str(&e)
                            .filter(|r| self.module.classes.contains_key(r))
                    }
                });
                // When the `extends` alias is a TYPEDEF of a PARAMETERIZED
                // class (`class D extends simple_lib` where
                // `typedef uvm_sequence_library#(sequence_item) simple_lib;`),
                // `extends_type_args` was recorded as EMPTY (a bare alias name).
                // Every ancestor walk that rebinds the base's type params
                // (`static_receiver_spec`, `static_prop_key`) then cites only
                // the base class with its UNBOUND declared params, so a derived
                // instance's base method resolves `this_type` to the DEFAULT
                // specialization instead of the concrete one. Carry the alias's
                // specialization args here (positional, per the base's
                // param_order) so those walks rebind REQ/RSP to `simple_item`.
                if let Some(real) = &resolved_extends {
                    if let Some(cd) = self.module.classes.get(cname) {
                        let already = cd.extends_type_args.iter().any(|a| a != "<unknown>");
                        if !already
                            && self.module.classes.get(real).is_some_and(|p| {
                                !p.type_param_names.is_empty() || !p.param_order.is_empty()
                            })
                            && cd
                                .extends
                                .as_deref()
                                .is_some_and(|e| self.resolve_typedef_spec(e).is_some())
                            && let Some((rb, rsig)) =
                                self.resolve_typedef_spec(cd.extends.as_deref().unwrap_or(""))
                            && rb == *real
                        {
                            resolved_args = Self::split_spec_args(&rsig);
                        }
                    }
                }
            }
            if let Some(real) = resolved_extends {
                if let Some(cd) = self
                    .module
                    .classes
                    .get_mut(cname)
                    .map(std::sync::Arc::make_mut)
                {
                    cd.extends = Some(real);
                    if cd.extends_type_args.is_empty() && !resolved_args.is_empty() {
                        cd.extends_type_args = resolved_args;
                    }
                }
            }
        }
        for cname in &names {
            self.place_named_extends_args(cname);
            self.place_nested_named_spec_args(cname);
        }
        // §25.9: a property typed by a TYPEDEF of a virtual interface
        // (`typedef virtual bus_if vif_t; vif_t vif;`) is a virtual-interface
        // property exactly like the spelled-out `virtual bus_if vif;` — but
        // only the `virtual` qualifier marked one, so every binding, read and
        // `== null` test through it missed.
        let mut vif_added = false;
        for cname in &names {
            let mut adds: Vec<(String, (String, Option<String>))> = self
                .module
                .classes
                .get(cname)
                .map(|cd| {
                    cd.property_types
                        .iter()
                        .filter(|(p, _)| !cd.virtual_iface_properties.contains_key(p.as_str()))
                        .filter_map(|(p, dt)| {
                            if !matches!(dt, DataType::TypeReference { .. }) {
                                return None;
                            }
                            match Self::resolve_type_ref(dt, &self.module.typedef_types) {
                                DataType::Interface { name, modport, .. } => {
                                    Some((p.clone(), (name.name.clone(), modport.map(|m| m.name))))
                                }
                                _ => None,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            adds.extend(self.inherited_param_vif_props(cname));
            if adds.is_empty() {
                continue;
            }
            if let Some(cd) = self
                .module
                .classes
                .get_mut(cname)
                .map(std::sync::Arc::make_mut)
            {
                cd.virtual_iface_properties.extend(adds);
                vif_added = true;
            }
        }
        if vif_added {
            self.class_member_names_cell = std::cell::OnceCell::new();
        }
        for start in names {
            let mut seen: HashSet<String> = HashSet::default();
            let mut cur = Some(start.clone());
            while let Some(cname) = cur {
                if !seen.insert(cname.clone()) {
                    break;
                }
                let next = match self.get_class_def(&cname) {
                    Some(cd) => cd.extends.clone(),
                    None => None,
                };
                match next {
                    Some(parent) if seen.contains(&parent) => {
                        // This edge closes a cycle — sever it.
                        if let Some(cd) = self
                            .module
                            .classes
                            .get_mut(&cname)
                            .map(std::sync::Arc::make_mut)
                        {
                            eprintln!(
                                "[xezim][warning] severed cyclic class inheritance: \
                                 '{}' extends '{}' (cycle)",
                                cname, parent
                            );
                            cd.extends = None;
                        }
                        break;
                    }
                    other => cur = other,
                }
            }
        }
    }

    /// Run side-effecting static initializers (those containing function
    /// calls) for every class in the hierarchy of the given specialization.
    /// This ensures that per-specialization static properties (e.g.
    /// `local static bit m__initialized = __deferred_init()`) execute once per
    /// specialization with the correct parameter bindings, rather than only
    /// once for the generic class with default params.
    pub(super) fn ensure_spec_statics(&mut self, base: &str, sig: &str) {
        // Queue-based: add to pending queue and process iteratively to
        // avoid deep recursion on the Rust stack when initialization chains
        // dozens of specializations during init.
        let mut key = (base.to_string(), sig.to_string());
        if self.initialized_spec_statics.contains(&key) {
            return;
        }
        // Most callers already supply a canonical signature; preserve their
        // initialized fast return without repeating parameter resolution.
        key.1 = self.canonicalize_spec_sig(base, sig);
        if self.initialized_spec_statics.contains(&key) {
            return;
        }
        self.pending_spec_inits.push(key);
        if self.spec_init_depth > 0 {
            return; // will be processed by the outermost caller
        }
        self.spec_init_depth = 1;
        while let Some((b, s)) = self.pending_spec_inits.pop() {
            if !self.initialized_spec_statics.insert((b.clone(), s.clone())) {
                continue;
            }
            self.run_one_spec_statics(&b, &s);
        }
        self.spec_init_depth = 0;
    }

    /// Parse a string like `ClassName#(arg1,arg2)` into `(ClassName, "arg1,arg2")`.
    pub(super) fn extract_spec_from_string(&self, s: &str) -> Option<(String, String)> {
        // Whitespace-tolerant scan for `Base#(args)`. The parser reconstructs
        // parameterized type text with spaces around the `#`/parens
        // (`special_comp # ( N )`). Previously the code only matched the
        // compact `#(` form, so a SYMBOLIC value-param nested type argument
        // (e.g. `uvm_typeid#(special_comp#(N))` from inside
        // `special_comp#(N)`) was never parsed here — the value param `N`
        // stayed unresolved and fell back to its default, producing a
        // wrong-per-spec typeid/assoc key. Match `#` optionally followed by
        // spaces then `(`. `normalize_spec_ws` later canonically strips
        // remaining whitespace inside `sig`. Returns `(base, raw_sig)` where
        // base is the (possibly still symbolic) class name; callers that
        // need to re-extract nested specs keep whitespace handling here so
        // the raw sig is unchanged for further resolution.
        let bs = s.trim();
        let hash_idx = bs.find('#')?;
        let rest = &bs[hash_idx + 1..];
        let lparen = rest.find('(')?;
        let base = bs[..hash_idx].trim().to_string();
        if !bs.ends_with(')') {
            return None;
        }
        // `sig` is everything between `(` and the final `)`, preserving any
        // inner whitespace verbatim (the caller canonicalizes via
        // `normalize_spec_ws`).
        let open_paren = hash_idx + 1 + lparen;
        let sig = bs[open_paren + 1..bs.len() - 1].to_string();
        if self.get_class_def(&base).is_some() {
            return Some((base, sig));
        }
        None
    }

    /// Whether `class_name` declares any parameters (type or value). Used to
    /// decide per-specialization static storage: only a PARAMETERIZED class's
    /// statics vary per specialization. A static inherited from a
    /// NON-parameterized ancestor (e.g. a plain `class Base; static int c;`)
    /// is a single shared cell across every specialization of a derived
    /// parameterized class — keying it per-spec would wrongly split it.
    pub(super) fn class_is_parameterized(&self, class_name: &str) -> bool {
        let Some(cd) = self.get_class_def(class_name) else {
            return false;
        };
        // Only `param_order` and `type_param_names` reflect the class's
        // header `#(...)` parameter list. Do NOT check `param_defaults`,
        // which also contains class-BODY `localparam` declarations (e.g.
        // `localparam int DO_NOT_CATCH = 1;`). A class with only body
        // localparams is NOT parameterized — checking `param_defaults`
        // wrongly classified it as parameterized, causing its static-call
        // initializers to be skipped, which broke callback registration
        // for many tests.
        !cd.param_order.is_empty() || !cd.type_param_names.is_empty()
    }

    /// Check if `derived` extends `ancestor` (directly or transitively).
    pub(super) fn class_extends(&self, derived: &str, ancestor: &str) -> bool {
        let mut cur: Option<&str> = Some(derived);
        while let Some(cname) = cur {
            if cname == ancestor || self.class_origin(cname) == ancestor {
                return true;
            }
            cur = self
                .get_class_def(cname)
                .and_then(|cd| cd.extends.as_deref());
        }
        false
    }

    pub(super) fn run_one_spec_statics(&mut self, base: &str, sig: &str) {
        // Collect all static-call initializers from the class hierarchy.
        // §8.25: the chain of THIS specialization — its own class entry when
        // its base is a bound type parameter.
        let start = if self.dep_base.contains_key(base) {
            let args: Vec<String> = Self::split_spec_args(sig)
                .into_iter()
                .map(|a| a.trim().to_string())
                .collect();
            self.ensure_spec_class(base, &args)
        } else {
            base.to_string()
        };
        let inits: Vec<(String, String, Expression)> = {
            let mut acc = Vec::new();
            let mut cur = Some(start);
            while let Some(cname) = cur {
                if let Some(cd) = self.get_class_def(&cname) {
                    for (prop, expr) in &cd.property_inits {
                        if self.class_is_parameterized(&cd.name)
                            && cd.static_properties.contains(prop)
                            && (Self::expr_contains_call(expr)
                                || Self::static_new_args(expr).is_some())
                        {
                            acc.push((cd.name.clone(), prop.clone(), expr.clone()));
                        }
                    }
                    cur = cd.extends.clone();
                } else {
                    break;
                }
            }
            acc
        };
        // Run each initializer with current_spec set to this specialization.
        let saved_spec = self.current_spec.clone();
        self.current_spec = Some((base.to_string(), sig.to_string()));
        let canon_sig = self.canonicalize_spec_sig(base, sig);
        for (cname, prop, init) in inits {
            // An inherited static belongs to the DECLARING specialization,
            // so direct and derived references share its single initializer.
            let declaring_sig = self
                .ancestor_spec(base, &canon_sig, &cname)
                .unwrap_or_else(|| canon_sig.clone());
            self.current_spec = Some((cname.clone(), declaring_sig.clone()));
            let spec_key = format!("{}#{}::{}", cname, declaring_sig, prop);
            if self.class_statics.contains_key(&spec_key) {
                continue;
            }
            // Seed before evaluating: a constructor can read its own static
            // handle (or re-enter this specialization) while it is still null.
            let seed = self
                .get_class_def(&cname)
                .and_then(|cd| cd.properties.get(&prop))
                .map(|p| p.value.clone())
                .unwrap_or_else(|| Value::zero(32));
            self.class_statics.insert(spec_key.clone(), seed);
            self.class_context_stack.push(Some(cname.clone()));
            self.this_stack.push(None);
            let v = if let Some(args) = Self::static_new_args(&init) {
                self.construct_static_property(&cname, &prop, args)
                    .unwrap_or_else(|| Value::zero(32))
            } else {
                self.eval_expr(&init)
            };
            self.this_stack.pop();
            self.class_context_stack.pop();
            self.class_statics.insert(spec_key.clone(), v);
        }
        self.current_spec = Some((base.to_string(), sig.to_string()));
        let targets: Vec<DataType> = self
            .get_class_def(base)
            .map(|cd| cd.typedef_targets.values().cloned().collect())
            .unwrap_or_default();
        self.class_context_stack.push(Some(base.to_string()));
        for dt in targets {
            if let Some((b, s)) = self.spec_from_typedef_dt(&dt) {
                self.ensure_spec_statics(&b, &s);
            }
        }
        self.class_context_stack.pop();
        self.current_spec = saved_spec;
    }

    /// Bare class constructors need a declared destination type (§8.8).
    pub(super) fn static_new_args(init: &Expression) -> Option<&[Expression]> {
        match &init.kind {
            ExprKind::Call { func, args }
                if matches!(&func.kind, ExprKind::Ident(h)
                if h.path.len() == 1 && h.path[0].name.name == "new") =>
            {
                Some(args)
            }
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].name.name == "new" => Some(&[]),
            _ => None,
        }
    }

    /// Initialization-only typed dispatch, shared by concrete and generic
    /// classes. Resolve aliases/parameters before entering the constructor.
    pub(super) fn construct_static_property(
        &mut self,
        cname: &str,
        prop: &str,
        args: &[Expression],
    ) -> Option<Value> {
        let cd = self.get_class_def(cname)?;
        let tn = cd.properties.get(prop)?.type_name.clone()?;
        if let Some(kind) = Self::container_base(&tn) {
            let h = self.alloc_builtin_container(kind, args);
            return Some(Value::from_u64(h as u64, 32));
        }
        let tn = self.resolve_type_param_binding(&tn).unwrap_or(tn);
        let tref = crate::ast::types::TypeName {
            scopes: Vec::new(),
            name: crate::ast::Identifier {
                name: tn.clone(),
                span: crate::ast::Span::dummy(),
            },
            span: crate::ast::Span::dummy(),
        };
        let (base, mut type_args) = if let Some((base, sig)) = self.extract_spec_from_string(&tn) {
            let args = Self::split_spec_args(&sig)
                .iter()
                .map(|frag| self.static_spec_arg_expr(frag, tref.span))
                .collect();
            (base, Some(args))
        } else {
            self.resolve_typeref_class_with_type_args(&tref)?
        };
        if let Some(ta) = self
            .get_class_def(cname)
            .and_then(|cd| cd.property_type_args.get(prop))
        {
            type_args = Some(ta.clone());
        }
        let target = self.module.classes.get(&base)?.clone();
        let type_args = type_args.map(|ta| {
            ta.iter()
                .map(|e| self.freeze_static_type_arg(e))
                .collect::<Vec<_>>()
        });
        Some(self.instantiate_class_with_type_args(&target, args, type_args.as_deref()))
    }

    /// Freeze constructor type arguments while their declaring class is
    /// active. Keep named assignments and type literals intact.
    pub(super) fn freeze_static_type_arg(&self, e: &Expression) -> Expression {
        if let ExprKind::NamedArg { name, expr } = &e.kind {
            return Expression::new(
                ExprKind::NamedArg {
                    name: name.clone(),
                    expr: expr
                        .as_ref()
                        .map(|e| Box::new(self.freeze_static_type_arg(e))),
                },
                e.span,
            );
        }
        if matches!(e.kind, ExprKind::TypeLiteral(_)) {
            return e.clone();
        }
        self.expr_to_spec_fragment(e)
            .map(|frag| self.static_spec_arg_expr(&frag, e.span))
            .unwrap_or_else(|| e.clone())
    }

    /// Reconstitute a resolved type/value fragment without evaluating a type
    /// name as a value, or losing a nested class specialization's arguments.
    pub(super) fn static_spec_arg_expr(&self, frag: &str, span: crate::ast::Span) -> Expression {
        let text = frag.trim();
        if let Some(text) = text.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            return Expression::new(ExprKind::StringLiteral(text.to_string()), span);
        }
        if let Some(number) = Self::parse_spec_number(text.trim_start_matches('-')) {
            let literal = Expression::new(ExprKind::Number(number), span);
            return if text.starts_with('-') {
                Expression::new(
                    ExprKind::Unary {
                        op: UnaryOp::Minus,
                        operand: Box::new(literal),
                    },
                    span,
                )
            } else {
                literal
            };
        }
        let ident = |name: &str| Self::hier_ident_expr(Self::iface_path_segs(name, span), span);
        if let Some((base, sig)) = self.extract_spec_from_string(text) {
            return Expression::new(
                ExprKind::Specialization {
                    base: Box::new(ident(&base)),
                    type_args_text: sig,
                },
                span,
            );
        }
        ident(text)
    }

    /// Compute the specialization signature for `ancestor_class` given a
    /// leaf class `leaf_class` with spec signature `leaf_sig`.
    ///
    /// Walks the `extends` chain from `leaf_class` towards `ancestor_class`,
    /// resolving the parent's type args at each hop using the child's
    /// type-param bindings (derived from `leaf_sig` matched to
    /// `param_order`).
    ///
    /// Example: `callbacks#(my_obj, my_cb)` extends
    /// `typed_callbacks#(T)`. Given leaf `callbacks` with sig
    /// `my_obj,my_cb`, the ancestor `typed_callbacks` gets spec `my_obj`
    /// (the first type arg, since `T` maps to it).
    pub(super) fn ancestor_spec(
        &self,
        leaf_class: &str,
        leaf_sig: &str,
        ancestor_class: &str,
    ) -> Option<String> {
        if leaf_class == ancestor_class {
            return Some(leaf_sig.to_string());
        }
        // Build the leaf's type-param bindings from its spec signature.
        let leaf_args = Self::split_spec_args(leaf_sig);
        let mut bindings: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        {
            let cd = self.module.classes.get(leaf_class)?;
            let order: &[String] = if cd.param_order.is_empty()
                && !(cd.param_defaults.is_empty() && cd.type_param_names.is_empty())
            {
                return None;
            } else {
                &cd.param_order
            };
            for (i, arg) in leaf_args.iter().enumerate() {
                if let Some(pname) = order.get(i) {
                    bindings.insert(pname.clone(), arg.trim().to_string());
                }
            }
        }
        // Walk the extends chain resolving type args at each hop — from the
        // specialization's own class entry when its base is a bound type
        // parameter (§8.25).
        let mut cur = self.spec_entry(leaf_class, leaf_sig);
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > 64 {
                return None;
            }
            let cd = self.module.classes.get(&cur)?;
            let parent = cd.extends.clone()?;
            // Resolve extends args using current bindings. A bare name is a
            // parameter of THIS class: its binding, or — when the leaf
            // specialization did not provide it — its DEFAULT, so the
            // ancestor signature matches the one a directly-named
            // specialization would produce. Without this, a defaulted type
            // param leaks its bare parameter identifier name into the
            // ancestor sig, yielding a static key that no writer ever used —
            // so inherited statics read as default/uninitialized. IEEE
            // 1800-2023 §6.20.2 — and the default is evaluated against the
            // parameters before it (`int D = W*2` follows this
            // specialization's W).
            let name_arg = |nm: &str| -> String {
                if let Some(v) = bindings.get(nm) {
                    return v.clone();
                }
                if let Some(i) = cd.param_order.iter().position(|p| p == nm) {
                    let mut bound: Vec<String> = Vec::with_capacity(i);
                    for (j, p) in cd.param_order[..i].iter().enumerate() {
                        match bindings
                            .get(p)
                            .cloned()
                            .or_else(|| self.param_default_fragment(cd, j, &bound))
                        {
                            Some(v) => bound.push(v),
                            None => break,
                        }
                    }
                    if let Some(d) = self.param_default_fragment(cd, i, &bound) {
                        return d;
                    }
                }
                if let Some((_, frag)) = cd.type_param_defaults.iter().find(|(n, _)| n == nm) {
                    return frag.clone();
                }
                if let Some((_, Some(init))) = cd.param_defaults.iter().find(|(n, _)| n == nm) {
                    if let Some(frag) = self.expr_to_spec_fragment(init) {
                        return frag;
                    }
                }
                nm.to_string()
            };
            let has_proj = cd
                .extends_type_args
                .iter()
                .any(|t| Self::spec_projection(t).is_some());
            let parent_args: Vec<String> =
                if !has_proj && cd.extends_args.len() == cd.extends_type_args.len() {
                    cd.extends_args
                        .iter()
                        .map(|e| {
                            if let crate::ast::expr::ExprKind::Ident(h) = &e.kind {
                                if h.path.len() == 1 && h.path[0].selects.is_empty() {
                                    return name_arg(&h.path[0].name.name);
                                }
                            }
                            // An EXPRESSION extends-arg (`extends Base#(N * 3)`)
                            // must be evaluated with this class's bindings — the
                            // raw fragment "N*3" matches no registered spec and
                            // the ancestor silently fell back to its defaults.
                            let mut ptab: HashMap<String, Value> = HashMap::default();
                            for (k, v) in &bindings {
                                if let Ok(n) = v.parse::<i64>() {
                                    ptab.insert(k.clone(), Value::from_u64(n as u64, 32));
                                }
                            }
                            if let Some(n) =
                                super::elaborate::const_eval_i64_with_params(e, Some(&ptab))
                            {
                                return n.to_string();
                            }
                            self.expr_to_spec_fragment(e).unwrap_or_default()
                        })
                        .collect()
                } else {
                    // A type argument written as a keyword type (`#(byte, 4)`) is
                    // no expression, so `extends_args` skips it and every later
                    // argument would shift one slot left; the textual list keeps
                    // every position.
                    let mut v: Vec<String> = Vec::with_capacity(cd.extends_type_args.len());
                    for t in &cd.extends_type_args {
                        let t = t.trim();
                        if Self::spec_projection(t).is_some() {
                            // `BASE#[i]`: an argument of this class's binding of
                            // its base type parameter; one it does not spell
                            // (and every later one) takes the base's default.
                            match Self::carry_extends_arg(t, &|p| Some(name_arg(p))) {
                                Some(a) => v.push(a),
                                None => break,
                            }
                            continue;
                        }
                        let is_name = t
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                            && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                        v.push(if is_name { name_arg(t) } else { t.to_string() });
                    }
                    v
                };
            // §25.9: a virtual-interface argument (`extends B#(virtual
            // I#(T))`) names this class's parameters inside its own `#(...)`.
            let parent_args: Vec<String> = parent_args
                .into_iter()
                .map(|a| match self.parse_vif_frag(&a) {
                    Some((iname, args, modport)) => {
                        let args = args
                            .iter()
                            .map(|x| {
                                let x = x.trim();
                                let is_name = x
                                    .chars()
                                    .next()
                                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                                    && x.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                                if is_name { name_arg(x) } else { x.to_string() }
                            })
                            .collect();
                        self.canonical_vif_frag(&iname, args, modport)
                    }
                    None => a,
                })
                .collect();
            if parent == ancestor_class || self.class_origin(&parent) == ancestor_class {
                return Some(parent_args.join(","));
            }
            // Update bindings for the parent class.
            let parent_cd = self.module.classes.get(&parent)?;
            let parent_order: &[String] = &parent_cd.param_order;
            let mut new_bindings = std::collections::HashMap::new();
            for (i, arg) in parent_args.iter().enumerate() {
                if let Some(pname) = parent_order.get(i) {
                    new_bindings.insert(pname.clone(), arg.clone());
                }
            }
            bindings = new_bindings;
            cur = parent;
        }
    }

    /// Canonicalize a specialization signature for a class so that two
    /// references to the SAME specialization produce the SAME static-storage
    /// key, regardless of how they were written.
    ///
    /// Two inconsistencies arise from defaulted parameters (IEEE 1800-2020
    /// §6.20.2):
    ///   1. A directly-named `C#(arg)` omits defaulted trailing params
    ///      (`callbacks#(a_comp)` → sig `"a_comp"`), but a typedef-based
    ///      call (`typedef C#(T,ST,CB) this_type;` then `this_type::get()`)
    ///      expands to ALL params, leaving an unbound one as its bare name
    ///      (`"a_comp,base_comp,CB"`).
    ///   2. Reaching `C` via an ancestor's `extends` resolves unbound params
    ///      differently than naming `C` directly.
    ///
    /// This folds both into one canonical form: for each position, if the
    /// fragment is the bare NAME of that position's parameter AND the param
    /// has a default, substitute the default; then pad any missing trailing
    /// positions with their defaults. So both `"a_comp"` and
    /// `"a_comp,base_comp,CB"` (for `derived_callbacks#(T,ST,CB=`
    /// `base_callback)`) become `"a_comp,base_comp,base_callback"`.
    ///
    /// Without this, an inherited static such as
    /// `callbacks::m_typeid` was written under one key and read under
    /// another, so `register_super_type` read 0 and the derived-type
    /// callback graph stayed empty.
    pub(super) fn replace_ident_token(s: &str, target: &str, replacement: &str) -> String {
        if !s.contains(target) || target.is_empty() {
            return s.to_string();
        }
        let mut out = String::new();
        let bytes = s.as_bytes();
        let t_len = target.len();
        let mut i = 0;
        while i < bytes.len() {
            if i + t_len <= bytes.len() && &s[i..i + t_len] == target {
                let prev_ok = if i == 0 {
                    true
                } else {
                    let b = bytes[i - 1];
                    !(b.is_ascii_alphanumeric() || b == b'_')
                };
                let next_ok = if i + t_len == bytes.len() {
                    true
                } else {
                    let b = bytes[i + t_len];
                    !(b.is_ascii_alphanumeric() || b == b'_')
                };
                if prev_ok && next_ok {
                    out.push_str(replacement);
                    i += t_len;
                    continue;
                }
            }
            out.push(bytes[i] as char);
            i += 1;
        }
        out
    }

    pub(super) fn get_class_def<'a>(
        &'a self,
        name: &str,
    ) -> Option<&'a crate::compiler::elaborate::ElaboratedClass> {
        self.module.classes.get(name).map(|c| c.as_ref())
    }

    pub(super) fn substitute_spec_params(&self, frag: &str) -> String {
        let (b, s) = match &self.current_spec {
            Some(pair) => pair,
            None => return frag.to_string(),
        };
        let cd = match self.get_class_def(b) {
            Some(cd) => cd,
            None => return frag.to_string(),
        };
        if cd.param_order.is_empty() {
            return frag.to_string();
        }
        let sig_frags = Self::split_spec_args(s);
        let mut result = frag.to_string();
        for (idx, pname) in cd.param_order.iter().enumerate() {
            if let Some(val) = sig_frags.get(idx) {
                let val_trimmed = val.trim();
                result = Self::replace_ident_token(&result, pname, val_trimmed);
            }
        }
        result
    }

    pub(super) fn canonicalize_spec_frag(&self, frag: &str) -> String {
        let frag_norm = frag.trim();
        // §6.22.1: `pkg::T` and the imported `T` are one type, so they
        // key one specialization.
        if let Some(leaf) = self.package_type_leaf(frag_norm) {
            return self.canonicalize_spec_frag(leaf);
        }
        if let Some((inner_base, inner_sig)) = self.extract_spec_from_string(frag_norm) {
            let canon_inner_sig = self.canonicalize_spec_sig(&inner_base, &inner_sig);
            if !canon_inner_sig.is_empty() {
                format!("{}#({})", inner_base, canon_inner_sig)
            } else {
                inner_base.to_string()
            }
        } else if let Some((inner_base, inner_sig)) = self.resolve_typedef_spec(frag_norm) {
            let canon_inner_sig = self.canonicalize_spec_sig(&inner_base, &inner_sig);
            if !canon_inner_sig.is_empty() {
                format!("{}#({})", inner_base, canon_inner_sig)
            } else {
                inner_base.to_string()
            }
        } else if let Some(t) = self.unpacked_typedef_rep(frag_norm) {
            // §6.22.1: typedefs of the same unpacked array type are one type
            // argument.
            t
        } else if let Some(t) = self.typedef_alias_frag(frag_norm) {
            // §6.18: a typedef stands for its target type.
            self.canonicalize_spec_frag(&t)
        } else if let Some((name, args, modport)) = self.parse_vif_frag(frag_norm) {
            // §25.9/§6.22.1: one key per virtual-interface type however it is
            // spelled (`interface` keyword, defaults written out).
            let args = args
                .iter()
                .map(|a| self.canonicalize_spec_frag(a))
                .collect();
            self.canonical_vif_frag(&name, args, modport)
        } else if let Some(v) = self.fold_const_spec_frag(frag_norm, false) {
            // §8.25: a value argument matches by VALUE (`P#(2+2)` is `P#(4)`).
            v
        } else {
            let short = frag_norm.to_string();
            // §8.25/§6.18: a class named through any of its registered
            // spellings (`Cfg` via an import, `my_pkg::Cfg`, `Outer::Inner`,
            // `my_pkg::Outer::Inner`) is ONE type argument. Resolve the
            // fragment to the declaring spelling so the specialization
            // signature — and with it every per-specialization static —
            // is shared however the argument was written.
            if let Some(canon) = self.canonical_class_frag(&short) {
                return canon;
            }
            if let Some(cd) = self.get_class_def(&short) {
                if !cd.param_order.is_empty() {
                    let canon_inner_sig = self.canonicalize_spec_sig(&short, "");
                    if !canon_inner_sig.is_empty() {
                        return format!("{}#({})", short, canon_inner_sig);
                    }
                }
            }
            short
        }
    }

    /// §8.25: map a class-name fragment — however spelled — to the ONE
    /// canonical key of the class it names, so specializations keyed by
    /// that argument share their statics. The elaborator registers a
    /// package class under both `Name` and `pkg::Name` (and a nested
    /// class under `Outer::Inner` and `pkg::Outer::Inner`), all mapping
    /// to the same `Arc<ElaboratedClass>`; the DECLARING spelling
    /// (`name`, or `enclosing::name` for a class nested in another) is
    /// the canonical one. A leaf name taken by a DIFFERENT class (two
    /// packages each declaring `Cfg`, a second nested `Inner`) keeps its
    /// distinct declaring name and remains a separate specialization.
    /// The class a plain type-argument fragment names. A direct key hit
    /// first; a `pkg::A::B` that is not a key falls back to `A::B`. The
    /// pkg_scope collision pass renames a nested class whose leaf name is
    /// declared in several packages to the mangled `A::B` (registered
    /// under `A::B`, `A::A::B` and `pkg::A::A::B`), so a source spelling
    /// that kept the unmangled `pkg::A::B` form misses the direct lookup
    /// although the stripped form is one of its registered aliases.
    fn class_arc_for_frag(&self, frag: &str) -> Option<std::sync::Arc<crate::compiler::elaborate::ElaboratedClass>> {
        if let Some(arc) = self.module.classes.get(frag) {
            return Some(arc.clone());
        }
        let mut parts: Vec<&str> = frag.split("::").collect();
        if parts.len() >= 3 {
            parts.remove(0);
            let stripped = parts.join("::");
            return self.module.classes.get(&stripped).cloned();
        }
        None
    }

    fn canonical_class_frag(&self, frag: &str) -> Option<String> {
        // Plain (possibly `::`-scoped) identifiers only: anything with
        // `#`, `[`, whitespace or operators is another branch's job.
        if frag.is_empty()
            || !frag
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
        {
            return None;
        }
        let arc = self.class_arc_for_frag(frag)?;
        let declared = match &arc.enclosing {
            Some(outer) => format!("{}::{}", outer, arc.name),
            None => arc.name.clone(),
        };
        if declared == frag {
            return None; // already the declaring spelling
        }
        // Artifact decoding reconstructs separate Arcs even for aliases.
        // The elaborator's declaring name (qualified on package collisions),
        // enclosing class and declaring module retain declaration identity.
        let declared_arc = self.module.classes.get(&declared)?;
        if !Self::same_class_declaration(&arc, declared_arc) {
            return None;
        }
        // A parameterized class keeps the default-argument completion the
        // final branch below applies (`box` -> `box#(int)`).
        if !arc.param_order.is_empty() {
            let sig = self.canonicalize_spec_sig(&declared, "");
            if !sig.is_empty() {
                return Some(format!("{}#({})", declared, sig));
            }
        }
        Some(declared)
    }

    fn same_class_declaration(
        a: &crate::compiler::elaborate::ElaboratedClass,
        b: &crate::compiler::elaborate::ElaboratedClass,
    ) -> bool {
        a.name == b.name && a.enclosing == b.enclosing && a.declaring_module == b.declaring_module
    }

    /// §8.25: specializations match when their value arguments have the SAME
    /// VALUE, however written: `P#(2+2)`, `P#('h4)` and `P#(4)` are one
    /// specialization with one set of statics. The decimal value of a
    /// specialization-argument text that is a constant integer expression
    /// of unsized operands; None for anything else (a type, a name, a
    /// string or real, x/z bits). A sized literal (`4'd4`) folds only for a
    /// `typed` integral parameter: an untyped one takes the literal's width
    /// as part of its type, so `Q#(4'd4)` is not `Q#(4)`. Memoized: keys are
    /// canonicalized at run time.
    pub(super) fn fold_const_spec_frag(&self, frag: &str, typed: bool) -> Option<String> {
        let b = frag.as_bytes();
        let first = *b.first()?;
        let ident_start = first.is_ascii_alphabetic() || first == b'_';
        if !(ident_start
            || first.is_ascii_digit()
            || matches!(first, b'\'' | b'(' | b'-' | b'+' | b'~' | b'!'))
        {
            return None;
        }
        // A name is folded only when it is a design parameter (`P#(LP)`);
        // a type, class, or enum name keeps its spelling. Its value depends
        // on the scope, so it is not memoized.
        if ident_start && b.iter().all(|&c| c.is_ascii_alphanumeric() || c == b'_') {
            // `scoped_param_value` tests the parameter leaf names first: the
            // common type / class name costs one probe.
            let v = self.scoped_param_value(frag)?;
            if self.class_member_names().params.contains(frag) {
                return None;
            }
            return v.to_i64().map(|n| n.to_string());
        }
        // A package parameter named through its package (`P#(pkg::W)`) is
        // the same value as the imported `P#(W)`; a package type or enum
        // name keeps its spelling.
        if let Some(key) = self.package_param_key(frag) {
            return self.module.parameters.get(key)?.to_i64().map(|n| n.to_string());
        }
        // Already canonical: a plain decimal.
        let digits = b.strip_prefix(b"-").unwrap_or(b);
        if !digits.is_empty() && digits.iter().all(u8::is_ascii_digit) {
            return None;
        }
        // A text with names folds only as an expression (`LP * 2`): a type
        // spelling (`int unsigned`, `virtual bus_if`) has no operator.
        if ident_start
            && !b.iter().any(|c| {
                matches!(
                    c,
                    b'+' | b'-'
                        | b'*'
                        | b'/'
                        | b'%'
                        | b'<'
                        | b'>'
                        | b'&'
                        | b'|'
                        | b'^'
                        | b'~'
                        | b'!'
                )
            })
        {
            return None;
        }
        if b.iter()
            .any(|&c| matches!(c, b'.' | b'"' | b'$' | b'?' | b'#' | b'[' | b'{'))
            || Self::has_xz_literal(b)
        {
            return None;
        }
        if !typed {
            let mut prev = b' ';
            for &c in b {
                if c == b'\'' && prev.is_ascii_digit() {
                    return None;
                }
                if !c.is_ascii_whitespace() {
                    prev = c;
                }
            }
        }
        // A text naming parameters folds per scope: memoize it under the
        // scope it was folded in.
        let key = if b.iter().any(|c| c.is_ascii_alphabetic() || *c == b'_') {
            let scope = self.name_resolve_hint.borrow().clone().unwrap_or_default();
            format!("{}|{}", scope, frag)
        } else {
            frag.to_string()
        };
        if let Some(hit) = self.spec_frag_fold_cache.borrow().get(&key) {
            return hit.clone();
        }
        let folded = self.fold_const_spec_text(frag);
        self.spec_frag_fold_cache
            .borrow_mut()
            .insert(key, folded.clone());
        folded
    }

    /// The value of design parameter `name` as seen from the executing scope:
    /// the top's own (bare key), else the nearest enclosing instance's
    /// (`scope.name`, walking the resolve hint outward).
    pub(super) fn scoped_param_value(&self, name: &str) -> Option<Value> {
        let leaves = self.param_leaf_names.get_or_init(|| {
            self.module
                .parameters
                .keys()
                .map(|k| k.rsplit(['.', ':']).next().unwrap_or(k).to_string())
                .collect()
        });
        if !leaves.contains(name) {
            return None;
        }
        if let Some(v) = self.module.parameters.get(name) {
            return Some(v.clone());
        }
        let hint = self.name_resolve_hint.borrow().clone()?;
        let mut scope = hint.as_str();
        loop {
            if let Some(v) = self.module.parameters.get(&format!("{}.{}", scope, name)) {
                return Some(v.clone());
            }
            scope = scope.rsplit_once('.')?.0;
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn fold_const_spec_text(&self, text: &str) -> Option<String> {
        use crate::ast::decl::{ModuleItem, ParameterKind};
        // Literals, operators, and design parameters no class declares (a
        // class's own parameter of the same name would shadow it).
        let names = self.class_member_names();
        let literal_only = |e: &Expression| -> bool {
            fn walk(e: &Expression, ok_name: &dyn Fn(&str) -> bool) -> bool {
                match &e.kind {
                    ExprKind::Number(NumberLiteral::Integer { .. })
                    | ExprKind::Number(NumberLiteral::UnbasedUnsized(_)) => true,
                    ExprKind::Ident(_) | ExprKind::MemberAccess { .. } => {
                        spec_operand_name(e).is_some_and(|n| ok_name(&n))
                    }
                    ExprKind::Paren(x) | ExprKind::Unary { operand: x, .. } => walk(x, ok_name),
                    ExprKind::Binary { left, right, .. } => {
                        walk(left, ok_name) && walk(right, ok_name)
                    }
                    ExprKind::Conditional {
                        condition,
                        then_expr,
                        else_expr,
                    } => {
                        walk(condition, ok_name)
                            && walk(then_expr, ok_name)
                            && walk(else_expr, ok_name)
                    }
                    _ => false,
                }
            }
            walk(e, &|n: &str| match self.package_param_key(n) {
                Some(key) => self.module.parameters.contains_key(key),
                None => !names.params.contains(n) && self.scoped_param_value(n).is_some(),
            })
        };
        // `W`, or `pkg::W` (an identifier path or, in some contexts, a
        // member access); a longer path is rejected by `package_param_key`.
        fn spec_operand_name(e: &Expression) -> Option<String> {
            match &e.kind {
                ExprKind::Ident(h)
                    if h.root.is_none() && h.path.iter().all(|s| s.selects.is_empty()) =>
                {
                    Some(
                        h.path
                            .iter()
                            .map(|s| s.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join("::"),
                    )
                }
                ExprKind::MemberAccess { expr, member } => match &expr.kind {
                    ExprKind::Ident(h) if h.root.is_none() && h.path.len() == 1 => {
                        Some(format!("{}::{}", h.path[0].name.name, member.name))
                    }
                    _ => None,
                },
                _ => None,
            }
        }
        let src = format!(
            "module __xezim_spec_arg; localparam __xezim_v = {};\nendmodule\n",
            text
        );
        let tokens = crate::lexer::Lexer::new(&src).tokenize();
        let mut parser = sv_parser::parse::Parser::new(tokens);
        let parsed = parser.parse_source_text();
        if parser
            .diagnostics()
            .iter()
            .any(|d| d.severity == sv_parser::diagnostics::Severity::Error)
        {
            return None;
        }
        let Some(crate::ast::Description::Module(m)) = parsed.descriptions.into_iter().next()
        else {
            return None;
        };
        let init = m.items.into_iter().find_map(|it| match it {
            ModuleItem::LocalparamDeclaration(pd) => match pd.kind {
                ParameterKind::Data { assignments, .. } => {
                    assignments.into_iter().next().and_then(|a| a.init)
                }
                _ => None,
            },
            _ => None,
        })?;
        if !literal_only(&init) {
            return None;
        }
        // The parameters it names, as seen from this scope.
        let mut ptab: HashMap<String, Value> = HashMap::default();
        fn names_in(e: &Expression, out: &mut Vec<String>) {
            match &e.kind {
                ExprKind::Ident(_) | ExprKind::MemberAccess { .. } => {
                    out.extend(spec_operand_name(e))
                }
                ExprKind::Paren(x) | ExprKind::Unary { operand: x, .. } => names_in(x, out),
                ExprKind::Binary { left, right, .. } => {
                    names_in(left, out);
                    names_in(right, out);
                }
                ExprKind::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                } => {
                    names_in(condition, out);
                    names_in(then_expr, out);
                    names_in(else_expr, out);
                }
                _ => {}
            }
        }
        let mut ids = Vec::new();
        names_in(&init, &mut ids);
        for n in ids {
            let v = match self.package_param_key(&n) {
                Some(key) => self.module.parameters.get(key)?.clone(),
                None => self.scoped_param_value(&n)?,
            };
            ptab.insert(n, v);
        }
        let v = super::elaborate::const_eval_i64_with_params(&init, Some(&ptab))?;
        Some(v.to_string())
    }

    /// The decimal value of a constant expression whose only names are
    /// package parameters (`pkg::N + pkg::K`); None when it reads anything
    /// else.
    pub(super) fn fold_package_const_expr(&self, e: &Expression) -> Option<String> {
        let mut ptab: HashMap<String, Value> = HashMap::default();
        if !self.bind_default_operands(e, &[], &[], &mut ptab) {
            return None;
        }
        super::elaborate::const_eval_i64_with_params(e, Some(&ptab)).map(|n| n.to_string())
    }

    /// `pkg::NAME` when `pkg` is a package of the design: the key
    /// `module.parameters` holds that package's parameter under. None for a
    /// bare name or any other scope.
    pub(super) fn package_param_key<'a>(&self, text: &'a str) -> Option<&'a str> {
        let (pkg, leaf) = text.split_once("::")?;
        let ident = |s: &str| {
            s.as_bytes()
                .first()
                .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
                && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        };
        (ident(pkg) && ident(leaf) && self.module.packages.contains(pkg)).then_some(text)
    }

    /// Whether specialization-argument text holds an x/z/? digit: a based
    /// literal (`4'bx01`, `'hz`) or an unbased unsized `'x`/`'z`. Letters
    /// of names (`cfg_pkg::W`) are not digits.
    pub(super) fn has_xz_literal(b: &[u8]) -> bool {
        let mut i = 0;
        while i < b.len() {
            if b[i] != b'\'' {
                i += 1;
                continue;
            }
            i += 1;
            if i < b.len() && matches!(b[i], b's' | b'S') {
                i += 1;
            }
            if i < b.len() && matches!(b[i], b'b' | b'B' | b'o' | b'O' | b'd' | b'D' | b'h' | b'H') {
                i += 1;
                while i < b.len() && b[i].is_ascii_whitespace() {
                    i += 1;
                }
            }
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'?') {
                if matches!(b[i], b'x' | b'X' | b'z' | b'Z' | b'?') {
                    return true;
                }
                i += 1;
            }
        }
        false
    }

    /// `T[#(...)]` of a type argument spelled `pkg::T[#(...)]`, when the bare
    /// name `T` is tabled as that same package class or typedef (package
    /// members also live in the flat design namespace under their bare
    /// names). None when the bare name is some other declaration.
    pub(super) fn package_type_leaf<'a>(&self, frag: &'a str) -> Option<&'a str> {
        let (pkg, rest) = frag.split_once("::")?;
        if !self.module.packages.contains(pkg.trim()) {
            return None;
        }
        let rest = rest.trim();
        let head = rest[..rest.find(['#', ' ']).unwrap_or(rest.len())].trim();
        if head.is_empty() || !head.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
            return None;
        }
        let qualified = format!("{}::{}", pkg.trim(), head);
        let same_class = match (
            self.module.classes.get(&qualified),
            self.module.classes.get(head),
        ) {
            (Some(q), Some(b)) => Self::same_class_declaration(q, b),
            _ => false,
        };
        let same_typedef = !same_class
            && !rest.contains('#')
            && self.module.typedef_types.contains_key(&qualified)
            && self.module.typedef_types.contains_key(head)
            // register_scoped_typedef_alias stores `pkg::declared_name`.
            // A collision-qualified declaration `pkg::T` therefore ALSO
            // has `pkg::pkg::T`; it is not an alias of an unrelated bare T.
            // Structural equality cannot distinguish nominal enum types,
            // especially when different files have identical byte spans.
            && !self.module.typedef_types.contains_key(&format!("{}::{}", pkg.trim(), qualified));
        (same_class || same_typedef).then_some(rest)
    }

    /// IEEE 1800-2017 §6.20.2: the DECLARED DEFAULT of parameter
    /// `param_order[i]` of class `cd`, rendered as a specialization fragment
    /// and evaluated in the scope of the parameters before it — `bound[j]` is
    /// the fragment already bound to `param_order[j]`.
    ///
    /// A default may name an earlier parameter (`int D = W*2`, `type RSP =
    /// REQ`). Taken verbatim, the fragment named the parameter instead of its
    /// value in THIS specialization: a slot no lookup could bind (`D` read
    /// x), and a signature unlike the explicit spelling of the same
    /// specialization, so the two kept separate statics (§8.25). Returns
    /// None when the default cannot be reduced to a constant here; the
    /// runtime then evaluates it with the specialization active.
    pub(super) fn param_default_fragment(
        &self,
        cd: &crate::compiler::elaborate::ElaboratedClass,
        i: usize,
        bound: &[String],
    ) -> Option<String> {
        let order = &cd.param_order;
        let pname = order.get(i)?;
        let earlier = &order[..i.min(bound.len())];
        let bound_of = |nm: &str| {
            earlier
                .iter()
                .position(|p| p == nm)
                .map(|j| bound[j].trim().to_string())
        };
        if let Some((_, d)) = cd.type_param_defaults.iter().find(|(n, _)| n == pname) {
            let d = d.trim();
            if let Some(v) = bound_of(d) {
                return Some(v);
            }
            // A specialized default (`type B = box#(T)`) follows the earlier
            // parameters it names.
            let mut out = d.to_string();
            if d.contains('#') {
                for (j, p) in earlier.iter().enumerate() {
                    out = Self::replace_ident_token(&out, p, bound[j].trim());
                }
            }
            return Some(out);
        }
        let init = cd
            .param_defaults
            .iter()
            .find(|(n, _)| n == pname)?
            .1
            .as_ref()?;
        match &init.kind {
            ExprKind::Number(_) | ExprKind::StringLiteral(_) => {
                return self.expr_to_spec_fragment(init);
            }
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                let nm = &h.path[0].name.name;
                if let Some(v) = bound_of(nm) {
                    return Some(v);
                }
                // Not a parameter of this class (an enum member, a package
                // constant): the name itself, resolved downstream.
                if !order.contains(nm) {
                    return self.expr_to_spec_fragment(init);
                }
                return None;
            }
            _ => {}
        }
        // An expression default (`W*2`, `A * pk::P`): fold it with the earlier
        // parameters bound. Only an integral parameter folds to an integer.
        let integral = match cd.property_types.get(pname) {
            Some(DataType::IntegerAtom { .. })
            | Some(DataType::IntegerVector { .. })
            | Some(DataType::Implicit { .. })
            | None => true,
            Some(_) => false,
        };
        if !integral {
            return None;
        }
        let mut ptab: HashMap<String, Value> = HashMap::default();
        if !self.bind_default_operands(init, earlier, bound, &mut ptab) {
            return None;
        }
        super::elaborate::const_eval_i64_with_params(init, Some(&ptab)).map(|n| n.to_string())
    }

    /// The operands of a parameter default `e` for constant folding, keyed as
    /// `const_eval_i64_with_params` looks them up: the earlier parameters
    /// (from their bound fragments) and package-qualified parameters. False
    /// when `e` reads anything else — a later parameter, a type parameter, a
    /// bare outer name — so the fold never falls back to an unrelated
    /// same-named constant of some other scope.
    pub(super) fn bind_default_operands(
        &self,
        e: &Expression,
        earlier: &[String],
        bound: &[String],
        ptab: &mut HashMap<String, Value>,
    ) -> bool {
        match &e.kind {
            ExprKind::Number(_) => true,
            ExprKind::Paren(x) | ExprKind::Unary { operand: x, .. } => {
                self.bind_default_operands(x, earlier, bound, ptab)
            }
            ExprKind::Binary { left, right, .. } => {
                self.bind_default_operands(left, earlier, bound, ptab)
                    && self.bind_default_operands(right, earlier, bound, ptab)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.bind_default_operands(condition, earlier, bound, ptab)
                    && self.bind_default_operands(then_expr, earlier, bound, ptab)
                    && self.bind_default_operands(else_expr, earlier, bound, ptab)
            }
            ExprKind::SystemCall { name, args } if name == "$clog2" => args
                .iter()
                .all(|a| self.bind_default_operands(a, earlier, bound, ptab)),
            ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => {
                match h.path.as_slice() {
                    [seg] => {
                        let nm = &seg.name.name;
                        let Some(j) = earlier.iter().position(|p| p == nm) else {
                            return false;
                        };
                        let t = bound[j].trim();
                        let (neg, body) = match t.strip_prefix('-') {
                            Some(b) => (true, b),
                            None => (false, t),
                        };
                        let Some(lit) = Self::parse_spec_number(body) else {
                            return false;
                        };
                        let lit = Expression::new(ExprKind::Number(lit), crate::ast::Span::dummy());
                        let Some(n) = super::elaborate::const_eval_i64_with_params(&lit, None)
                        else {
                            return false;
                        };
                        let n = if neg { n.wrapping_neg() } else { n };
                        ptab.insert(nm.clone(), Value::from_u64(n as u64, 64));
                        true
                    }
                    [pkg, seg] => {
                        let key = format!("{}::{}", pkg.name.name, seg.name.name);
                        match self.module.parameters.get(&key) {
                            Some(v) => {
                                ptab.insert(key, v.clone());
                                true
                            }
                            None => false,
                        }
                    }
                    _ => false,
                }
            }
            // `pkg::P` in a type's argument list parses as a member access.
            ExprKind::MemberAccess { expr, member } => match &expr.kind {
                ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                    let key = format!("{}::{}", h.path[0].name.name, member.name);
                    match self.module.parameters.get(&key) {
                        Some(v) => {
                            ptab.insert(key, v.clone());
                            true
                        }
                        None => false,
                    }
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// A named specialization argument `.NAME(value)` split into its name and
    /// (trimmed) value text; None for a positional fragment.
    pub(super) fn named_spec_arg(frag: &str) -> Option<(&str, &str)> {
        let rest = frag.trim().strip_prefix('.')?;
        let open = rest.find('(')?;
        let name = rest[..open].trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        let value = rest[open + 1..].trim_end().strip_suffix(')')?;
        Some((name, value.trim()))
    }

    pub(super) fn canonicalize_spec_sig(&self, class_name: &str, sig: &str) -> String {
        // The class's parameter lists are read in place: every call below
        // takes `&self`.
        let Some(cd) = self.module.classes.get(class_name) else {
            return sig.to_string();
        };
        let order = &cd.param_order;
        if order.is_empty() {
            return sig.to_string();
        }
        let mut frags: Vec<String> = Self::split_spec_args(sig)
            .into_iter()
            .map(|s| s.trim().to_string())
            .collect();
        // A slot filled with the parameter's own name because the named
        // list omitted it. That spelling is the default, not a reference
        // to an enclosing parameter of the same name.
        let mut omitted_slot = vec![false; frags.len()];
        // §8.25 named assignments (`.W(5)`) bind by NAME: move each to its
        // parameter's slot. A slot no argument names keeps its default,
        // spelled as the parameter's own name for the loop below to fill; a
        // trailing one is simply dropped.
        if frags.iter().any(|f| Self::named_spec_arg(f).is_some()) {
            let mut slots: Vec<Option<String>> = vec![None; order.len()];
            for (k, f) in frags.iter().enumerate() {
                match Self::named_spec_arg(f) {
                    Some((n, v)) => {
                        if let Some(i) = order.iter().position(|p| p == n) {
                            slots[i] = (!v.is_empty()).then(|| v.to_string());
                        }
                    }
                    None => {
                        if let Some(s) = slots.get_mut(k) {
                            *s = Some(f.clone());
                        }
                    }
                }
            }
            let used = slots.iter().rposition(|s| s.is_some()).map_or(0, |i| i + 1);
            omitted_slot = slots.iter().take(used).map(|s| s.is_none()).collect();
            frags = slots
                .into_iter()
                .take(used)
                .enumerate()
                .map(|(i, s)| s.unwrap_or_else(|| order[i].clone()))
                .collect();
        }
        // Resolve each provided fragment: if it is the bare name of the
        // parameter at its own position and that param has a default,
        // substitute the default.
        for i in 0..frags.len() {
            if order.get(i).is_some_and(|p| frags[i] == *p) {
                // The fragment is this parameter's own name. An omitted
                // argument is recorded that way and takes the default.
                // The same spelling is a real argument when an enclosing
                // specialization binds the name (`item#(H, D)` inside
                // `seq#(int H, int D)`). That binding wins over the default,
                // which would otherwise collapse `D` to `H` (32).
                let enclosing = if omitted_slot.get(i).copied().unwrap_or(false) {
                    None
                } else {
                    self.enclosing_value_param_fragment(&frags[i])
                };
                if let Some(v) = enclosing {
                    frags[i] = v;
                } else if let Some(d) = self.param_default_fragment(cd, i, &frags[..i]) {
                    frags[i] = d;
                }
            }
        }
        // Pad missing trailing positions with their defaults, each evaluated
        // in the scope of the parameters before it (§6.20.2). A default that
        // names an EARLIER parameter (`#(type REQ=uvm_sequence_item, type
        // RSP=REQ)` specialized as `Class#(simple_item)`, or `int D = W*2`)
        // must follow that parameter's binding here — else the partial
        // spelling keys a different per-spec static cell than the full one
        // (`simple_item,REQ` against `simple_item,simple_item`), although
        // §8.25 makes them one specialization. This is how UVM's
        // `uvm_sequence_library#(REQ,RSP=REQ)` keeps its static
        // `m_typewide_sequences` shared whether reached via `Lib::add_...`
        // (partial `#(simple_item)`) or via a derived subclass of the full
        // specialization.
        while frags.len() < order.len() {
            match self.param_default_fragment(cd, frags.len(), &frags) {
                Some(d) => frags.push(d),
                None => break, // no reducible default here — leave partial
            }
        }
        let canon_frags: Vec<String> = frags
            .iter()
            .enumerate()
            .map(|(i, f)| {
                // §8.25/§6.20.2: an argument of a TYPED integral parameter
                // (`int N`) is converted to that type, so a sized literal
                // (`4'd4`) is the same value as `4`.
                let typed = order.get(i).is_some_and(|p| {
                    matches!(
                        cd.property_types.get(p),
                        Some(DataType::IntegerAtom { .. } | DataType::IntegerVector { .. })
                    )
                });
                typed
                    .then(|| self.fold_const_spec_frag(f.trim(), true))
                    .flatten()
                    .unwrap_or_else(|| self.canonicalize_spec_frag(f))
            })
            .collect();
        canon_frags.join(",")
    }

    /// The active method's lexical specialization, including an inherited
    /// method's ancestor arguments. A free-function frame is a boundary.
    pub(super) fn lexical_value_spec(&self) -> Option<(String, String)> {
        let lexical = self.class_context_stack.last()?.as_ref()?;
        let (base, sig) = self
            .lexical_spec()
            .or_else(|| self.inherited_param_spec(lexical))
            .unwrap_or_else(|| (lexical.clone(), String::new()));
        if base != *lexical && self.class_is_a(&base, lexical) {
            Some((lexical.clone(), self.ancestor_spec(&base, &sig, lexical)?))
        } else {
            Some((base, sig))
        }
    }

    /// A value-parameter binding in the lexical enclosing specialization.
    ///
    /// `canonicalize_spec_sig` treats a fragment equal to the parameter's
    /// own name as an omitted argument. `item#(H, D)` written inside
    /// `seq#(int H, int D)` is that same spelling and means the outer
    /// parameter, not item's default.
    pub(super) fn enclosing_value_param_fragment(&self, pname: &str) -> Option<String> {
        let (base, sig) = self.lexical_value_spec()?;
        let cd = self.module.classes.get(&base)?;
        if cd.type_param_names.iter().any(|t| t == pname) {
            return None;
        }
        let idx = cd.param_order.iter().position(|p| p == pname)?;
        if Self::spec_arg_at(&sig, idx).is_some_and(|v| v.trim() == pname) {
            return None;
        }
        let sig = self.canonicalize_spec_sig(&base, &sig);
        let value = Self::spec_arg_at(&sig, idx)?;
        let value = value.trim();
        (!value.is_empty() && value != pname).then(|| value.to_string())
    }

    /// For a STATIC queue/associative/dynamic-array property of a
    /// parameterized class — accessed BARE inside a static method or a
    /// static initializer — return the per-specialization storage key
    /// (e.g. `uvmq#int::m_q`) so each specialization gets its own element
    /// storage and size cell. Instance methods (a non-zero `this`) and
    /// non-collection / non-static names are returned unchanged. Applied
    /// centrally in `eval_builtin_method` so every builtin
    /// (push_back/size/pop_front/...) keys off the same spec-aware name.
    pub(super) fn spec_static_coll_key<'n>(&self, name: &'n str) -> std::borrow::Cow<'n, str> {
        let this_h = self
            .this_stack
            .last()
            .copied()
            .flatten()
            .filter(|&h| h != 0);
        // Instance method: collection storage is instance-scoped
        // (`<handle>#name`), not static. A static method carries a ZERO
        // `this` handle, so filter that out.
        if this_h.is_some() {
            return std::borrow::Cow::Borrowed(name);
        }
        let Some(Some(ctx)) = self.class_context_stack.last().cloned() else {
            return std::borrow::Cow::Borrowed(name);
        };
        std::borrow::Cow::Owned(self.spec_static_coll_key_in_class(name, &ctx))
    }

    /// The per-specialization store of the static collection `name` when
    /// running a static method of a parameterized class (see
    /// `spec_static_coll_key`); `None` everywhere else.
    #[inline]
    pub(super) fn static_method_coll_key(&self, name: &str) -> Option<String> {
        if !matches!(self.class_context_stack.last(), Some(Some(_)))
            || self
                .this_stack
                .last()
                .copied()
                .flatten()
                .is_some_and(|h| h != 0)
        {
            return None;
        }
        self.static_method_coll_key_slow(name)
    }

    #[cold]
    #[inline(never)]
    pub(super) fn static_method_coll_key_slow(&self, name: &str) -> Option<String> {
        let Some(Some(ctx)) = self.class_context_stack.last() else {
            return None;
        };
        if !self.coll_member_possible(name) || !self.collection_is_static_in(ctx, name) {
            return None;
        }
        let k = self.spec_static_coll_key_in_class(name, ctx);
        k.contains('#').then_some(k)
    }

    /// Resolve a bare static-collection member `name` to its storage key,
    /// walking the class hierarchy of the given concrete class `ctx`.
    /// Shared by the static-method path (ctx from `class_context_stack`) and
    /// the instance-method path (ctx from the runtime class of `this`), so a
    /// static COLLECTION (registered in `static_collections`, not in
    /// `assoc_properties`/`queue_properties`) read/written BARE from an
    /// instance method lands in the SAME shared store the static-method path
    /// (and any `ClassName::m[...]` spelling) uses.
    pub(super) fn spec_static_coll_key_in_class(&self, name: &str, ctx: &str) -> String {
        // Walk ctx's inheritance chain for a STATIC collection property.
        // Static collections are registered in `static_collections`
        // (name, is_assoc, width) — NOT queue_properties/assoc_properties,
        // which are gated to non-static members during elaboration.
        let mut cur = Some(ctx.to_string());
        while let Some(cn) = cur {
            if let Some(cd) = self.module.classes.get(&cn) {
                if cd.static_properties.contains(name)
                    && cd.static_collections.iter().any(|(n, _, _)| n == name)
                {
                    if let Some(key) = self.static_prop_key(&cn, name) {
                        // Rewrite storage to the per-DECLARING-class key ONLY
                        // when the bare name collides across classes (§8.9) or
                        // the class is parameterized (`#spec`). A statically
                        // unique collection name keeps the bare store the rest
                        // of the runtime uses; rewriting it would split
                        // storage and break the many bare-name accessors
                        // (including the UVM phase machinery).
                        if key.contains('#') || self.static_coll_name_collides(name) {
                            return key;
                        }
                    }
                }
                cur = cd.extends.clone();
            } else {
                break;
            }
        }
        name.to_string()
    }

    /// Walk the class hierarchy from `start_class` and return the
    /// `"DeclClass::prop"` storage key for a static property, if one of
    /// `start_class` or its ancestors declares `prop` as `static`.
    pub(super) fn static_prop_key(&self, start_class: &str, prop: &str) -> Option<String> {
        self.static_prop_key_spec(start_class, prop, None)
    }

    pub(super) fn static_prop_key_spec(
        &self,
        start_class: &str,
        prop: &str,
        spec_override: Option<&(String, String)>,
    ) -> Option<String> {
        self.static_prop_key_inner(start_class, prop, spec_override)
    }

    pub(super) fn static_prop_key_inner(
        &self,
        start_class: &str,
        prop: &str,
        spec_override: Option<&(String, String)>,
    ) -> Option<String> {
        // Only a name some class declares static (or as a localparam) can
        // match below.
        if !self.class_member_names().static_or_param.contains(prop) {
            return None;
        }
        let mut cur = Some(start_class.to_string());
        while let Some(entry) = cur {
            if let Some(cd) = self.module.classes.get(&entry) {
                if cd.static_properties.contains(prop)
                    // Class `localparam` constants are accessible as static class
                    // members even though they are stored in `param_defaults`.
                    || cd.param_defaults.iter().any(|(name, _)| name == prop)
                {
                    // A specialization's own class entry (`wrap_c<derived_c>`)
                    // shares its statics' keys with the declared class: the
                    // specialization signature tells them apart (§8.25).
                    let cname = self.class_origin(&entry).to_string();
                    // Per-specialization keying: when a `C#(params)::...` access
                    // is active, each specialization gets its own static cell.
                    // This applies to statics declared in the spec's base class
                    // AND inherited from parameterized ancestors (per IEEE 1800-2023 §8.25).
                    // Fall back to synthesizing a spec from a CONCRETE receiver
                    // (e.g. `ext1::ID()` where `ext1 extends base#(ext1)`) so any
                    // static reachable through such a subclass is keyed per
                    // specialization too — this is the generic home of the
                    // concrete-receiver path, so it applies uniformly to static
                    // calls, instance-inherited statics, and property reads.
                    let active_spec = spec_override
                        .or(self.current_spec.as_ref())
                        .cloned()
                        .or_else(|| self.derive_static_spec(start_class, &cname));
                    // §8.25 / §13.5: the actuals of `D#(X)::m(cb)` are
                    // evaluated in the caller's scope, but the callee's
                    // specialization is already active. A static of the
                    // caller's own parameterized class (`cb` read inside
                    // `C#(int)`) keys by the caller's specialization, the
                    // innermost enclosing one of `cname`; keyed generically,
                    // the callee received null.
                    let active_spec = match active_spec {
                        Some((b, s))
                            if spec_override.is_none()
                                && b != cname
                                && self.class_is_parameterized(&cname)
                                && !self.class_extends(&self.spec_entry_cow(&b, &s), &cname)
                                && self
                                    .class_context_stack
                                    .last()
                                    .and_then(|c| c.as_deref())
                                    .is_some_and(|ctx| self.class_is_a(ctx, &cname)) =>
                        {
                            self.spec_scope_stack
                                .iter()
                                .rev()
                                .find(|(sb, ss)| {
                                    *sb == cname
                                        || self.class_extends(&self.spec_entry_cow(sb, ss), &cname)
                                })
                                .cloned()
                                .or(Some((b, s)))
                        }
                        other => other,
                    };
                    let key = match active_spec {
                        Some((base, sig))
                            if (base == cname
                                || self
                                    .class_extends(&self.spec_entry_cow(&base, &sig), &cname))
                                && self.class_is_parameterized(&cname) =>
                        {
                            // For inherited statics (cname != base), derive
                            // the DECLARING class's specialization from the
                            // leaf spec. This ensures that
                            // `callbacks#(T,CB)` and
                            // `callbacks#(T,base_callback)` — which both
                            // extend `typed_callbacks#(T)` — share the
                            // same `m_tw_cb_q` static cell.
                            if base == cname {
                                Self::spec_member_key(
                                    &cname,
                                    Some(self.canonicalize_spec_sig(&cname, sig.as_str()).as_str()),
                                    prop,
                                )
                            } else if let Some(ancestor_sig) =
                                self.ancestor_spec(base.as_str(), sig.as_str(), &cname)
                            {
                                Self::spec_member_key(
                                    &cname,
                                    Some(
                                        self.canonicalize_spec_sig(&cname, &ancestor_sig).as_str(),
                                    ),
                                    prop,
                                )
                            } else {
                                Self::spec_member_key(
                                    &base,
                                    Some(
                                        self.canonicalize_spec_sig(base.as_str(), sig.as_str())
                                            .as_str(),
                                    ),
                                    prop,
                                )
                            }
                        }
                        _ => Self::spec_member_key(&cname, None, prop),
                    };
                    // §8/§23.9: each instance of a module holds its own class
                    // declared there, so its statics are per instance.
                    if self.any_multi_modclass()
                        && let Some(inst) = self.modclass_static_inst(&cname)
                    {
                        return Some(Self::per_instance_static_key(&key, prop, &inst));
                    }
                    return Some(key);
                }
                // §8.23: a NESTED class sees its lexically enclosing class's
                // statics — hop to the enclosing class when the extends
                // chain is exhausted.
                cur = cd.extends.clone().or_else(|| cd.enclosing.clone());
            } else {
                break;
            }
        }
        None
    }

    /// Phase 2: the interned specialization id for a concrete instance's spec
    /// `(base, sig)` (0 when unspecialized / unparameterized). Called at every
    /// plan / skip / fast-call lookup so a parameterized class's caches are keyed
    /// per specialization — two specializations of the same method never share a
    /// compiled plan. The common (non-parameterized) path always returns 0 and is
    /// untouched.
    pub(super) fn instance_spec_id(&mut self, handle: usize) -> u32 {
        let Some(spec) = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .and_then(|inst| inst.spec.clone())
        else {
            return 0;
        };
        if let Some(&id) = self.compiled_spec_intern.get(&spec) {
            return id;
        }
        let next = self.compiled_spec_intern.len() as u32 + 1; // 0 reserved
        self.compiled_spec_intern.insert(spec, next);
        next
    }

    /// §8.25: derive the per-specialization signature of an ANCESTOR class
    /// reached through a CONCRETE subclass — e.g. `ext1::ID()` where
    /// `ext1 extends uvm_tlm_extension#(ext1)`. A static call like
    /// `ext1::ID()` carries no `#(spec)` and has no instance, so `current_spec`
    /// is None and inherited statics key the shared `Class::member` cell,
    /// colliding across sibling specializations (`ext1::ID()` and
    /// `ext2::ID()` returned the same singleton). Walk the receiver's extends
    /// chain rebinding each parent's type params from `extends_type_args` until
    /// `ancestor` is bound, and return its `(base, sig)` so the caller can seed
    /// `current_spec`. Mirrors the assoc-key carry loop.
    /// A method's type parameters belong to its declaring specialization,
    /// even when a parameterized receiver reuses the same parameter names.
    #[inline(never)]
    pub(super) fn inherited_method_spec(
        &self,
        handle: usize,
        declaring: &str,
    ) -> Option<(String, String)> {
        let origin = self.class_origin(declaring);
        if !self.class_is_parameterized(origin) {
            return None;
        }
        let inst = self.heap.get(handle)?.as_ref()?;
        if self.class_origin(&inst.class_name) == origin {
            return None;
        }
        if let Some((base, sig)) = &inst.spec {
            let inherited = self.ancestor_spec(base, sig, origin)?;
            return Some((
                origin.to_string(),
                self.canonicalize_spec_sig(origin, &inherited),
            ));
        }
        self.static_receiver_spec(&inst.class_name, origin)
    }

    pub(super) fn static_receiver_spec(
        &self,
        receiver: &str,
        ancestor: &str,
    ) -> Option<(String, String)> {
        let mut carried: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut cur: Option<&str> = Some(receiver);
        let mut level = 0;
        while let Some(cn) = cur {
            level += 1;
            if level > 64 {
                return None;
            }
            let cd = self.module.classes.get(cn)?;
            if cn == ancestor || self.class_origin(cn) == ancestor {
                let sig = self.rebind_class_sig(&cd, &carried);
                return Some((self.class_origin(cn).to_string(), sig));
            }
            // Move to the parent, rebinding its type params from this class's
            // extends type args.
            let parent: &str = cd.extends.as_deref()?;
            if let Some(pcd) = self.module.classes.get(parent) {
                let order: Vec<String> = if pcd.param_order.is_empty() {
                    pcd.type_param_names.clone()
                } else {
                    pcd.param_order.clone()
                };
                let mut next: std::collections::HashMap<String, String> =
                    std::collections::HashMap::new();
                // The parent's bindings in declaration order, as long as they
                // run unbroken: the scope a later default is evaluated in.
                let mut bound: Vec<String> = Vec::with_capacity(order.len());
                for (i, pname) in order.iter().enumerate() {
                    let v = if let Some(arg) = cd
                        .extends_type_args
                        .get(i)
                        .filter(|a| Self::spec_projection(a).is_none())
                    {
                        let a = arg.trim();
                        Some(
                            carried
                                .get(a)
                                .cloned()
                                .unwrap_or_else(|| Self::carry_spec_arg_names(a, &carried)),
                        )
                    } else if let Some(arg) = cd.extends_type_args.get(i) {
                        // `BASE#[i]`: the argument of this class's binding of
                        // its base type parameter, else the parent's default.
                        Self::carry_extends_arg(arg, &|p| {
                            carried.get(p).cloned().or_else(|| {
                                cd.type_param_defaults
                                    .iter()
                                    .find(|(n, _)| n == p)
                                    .map(|(_, d)| d.clone())
                            })
                        })
                        .or_else(|| self.param_default_fragment(pcd, i, &bound))
                    } else {
                        // IEEE 1800-2023 §6.20.2: an extends clause that omits a
                        // parameter leaves it at its DECLARED DEFAULT, evaluated
                        // against the parameters before it (`int D = W*2`).
                        // Without this the parameter's bare NAME leaked into the
                        // specialization signature (`pbase#(T)`), and resolving
                        // `T` in that spec failed — a type parameter read as
                        // `logic`. That is the `class d extends pbase;` shape
                        // (`class base_test extends base_test_param;`), where the
                        // derived class silently lost every defaulted parameter.
                        self.param_default_fragment(pcd, i, &bound)
                    };
                    if let Some(v) = v {
                        if bound.len() == i {
                            bound.push(v.clone());
                        }
                        next.insert(pname.clone(), v);
                    }
                }
                carried = next;
            }
            cur = Some(parent);
        }
        None
    }

    /// Rebuild a class's specialization signature string from a binding map of
    /// its type/value parameter names to concrete leaves (declaration order).
    pub(super) fn rebind_class_sig(
        &self,
        cd: &crate::compiler::elaborate::ElaboratedClass,
        carried: &std::collections::HashMap<String, String>,
    ) -> String {
        let order: Vec<String> = if cd.param_order.is_empty() {
            cd.type_param_names.clone()
        } else {
            cd.param_order.clone()
        };
        let mut out: Vec<String> = Vec::with_capacity(order.len());
        for (i, p) in order.iter().enumerate() {
            if let Some(v) = carried.get(p) {
                out.push(v.clone());
                continue;
            }
            // A parameter the extends chain never supplied is using its
            // DECLARED DEFAULT (§6.20.2), evaluated against the parameters
            // before it — not the parameter's own bare name. The bare name
            // produced a spec key like `pbase#(T)` that no resolution could
            // bind, so a type parameter in the method body fell back to
            // `logic` and a dependent value default (`D = W*2`) read x.
            if let Some(d) = self
                .param_default_fragment(cd, i, &out)
                .or_else(|| {
                    // A cache without `param_order`: type defaults only.
                    cd.type_param_defaults
                        .iter()
                        .find(|(n, _)| n == p)
                        .map(|(_, d)| d.trim().to_string())
                })
                .filter(|d| !d.is_empty())
            {
                out.push(d);
                continue;
            }
            out.push(p.clone());
        }
        out.join(",")
    }

    /// §8.25 generic spec derivation for a member accessed from subclass
    /// `start_class` whose declaring class is `cname`. When no specialization is
    /// already active (`current_spec`/`spec_override`) and `start_class` is a
    /// CONCRETE subclass of a PARAMETERIZED `cname`, synthesize the declaring
    /// class's specialization from the receiver's `extends_type_args`. This is
    /// called from `static_prop_key_inner` so every static-member key — whether
    /// reached by a static call, an instance-inherited static, or a bare
    /// property read — is keyed per specialization regardless of the caller.
    pub(super) fn derive_static_spec(
        &self,
        start_class: &str,
        cname: &str,
    ) -> Option<(String, String)> {
        if self.class_is_parameterized(start_class) || !self.class_is_parameterized(cname) {
            return None;
        }
        if !self.class_extends(start_class, cname) {
            return None;
        }
        self.static_receiver_spec(start_class, cname)
    }

    /// Resolve a CLASS-SCOPED constant `ClassName::member` — an enum literal
    /// of a `typedef enum` declared in `ClassName` or an ancestor, or a static
    /// property. SystemVerilog scopes enum literals to their declaring class,
    /// so `Base::STARTED` (where `STARTED` is a member of an enum typedef in
    /// `Base`) yields the literal's value. The parser represents such a scoped
    /// name — inside a method body — as `MemberAccess(Ident([ClassName]), member)`;
    /// without this resolution the class name is treated as a runtime object
    /// (it has none) and the access reads 0. Enum typedefs are hoisted globally
    /// by `hoist_class_local_typedefs`, so we walk the inheritance chain's
    /// `typedef_targets` and look each enum's members up in
    /// `module.enum_members`.
    /// Value of an enum member declared by `start_class` or one of its
    /// ancestors, or None when the class chain declares no such member. This
    /// is the CLASS-SCOPED half of `class_scoped_const`, split out so a BARE
    /// reference inside a class body can consult its own scope before the
    /// flat design-wide map (§6.19).
    pub(super) fn class_enum_member(&self, start_class: &str, member: &str) -> Option<Value> {
        let mut cur: Option<&str> = Some(start_class);
        let mut seen: HashSet<String> = HashSet::default();
        while let Some(cn) = cur {
            if !seen.insert(cn.to_string()) {
                break;
            }
            let cd = self.module.classes.get(cn)?;
            // A nearer static/property parameter hides an ancestor's enum
            // literal. Let the ordinary class-static lookup resolve it.
            if cd.static_properties.contains(member)
                || cd.param_defaults.iter().any(|(name, _)| name == member)
            {
                return None;
            }
            for (td_name, dt) in &cd.typedef_targets {
                if !matches!(dt, DataType::Enum(_)) {
                    continue;
                }
                if let Some(members) = self.module.enum_members.get(td_name) {
                    if let Some((_, val)) = members.iter().find(|(n, _)| n == member) {
                        let w = self
                            .module
                            .typedefs
                            .get(td_name)
                            .copied()
                            .unwrap_or(32)
                            .max(1);
                        // §6.19 wide base: `enum_members` stores u64 and
                        // truncates past 64 bits — recompute the full value
                        // from the class's own enum AST (rare width, cold).
                        if w > 64 {
                            if let DataType::Enum(et) = dt {
                                if let Some(v) = super::elaborate::wide_enum_value_map(
                                    et,
                                    w,
                                    &self.module.parameters,
                                )
                                .get(member)
                                {
                                    return Some(v.clone());
                                }
                            }
                        }
                        return Some(Value::from_u64(*val, w));
                    }
                }
            }
            cur = cd.extends.as_deref();
        }
        None
    }

    /// §20.6.2: the bits of a multi-dimensional unpacked array `name`
    /// (element bits × every dimension's size), from the 2-D / N-D tables.
    pub(super) fn unpacked_nd_bits(&self, name: &str) -> Option<u64> {
        if let Some(&((a0, a1), (b0, b1), ew)) = self.module.arrays_2d.get(name) {
            let n = ((a1 - a0).unsigned_abs() + 1) * ((b1 - b0).unsigned_abs() + 1);
            return Some(n * ew as u64);
        }
        let (shape, ew) = self.module.arrays_nd.get(name)?;
        let n: u64 = shape
            .iter()
            .map(|(l, h)| (h - l).unsigned_abs() + 1)
            .product();
        Some(n * *ew as u64)
    }

    /// Value of `pkg::MEMBER` for an enum member declared by package `pkg`
    /// (§26.3), or None when `pkg` names no package or declares no such member.
    ///
    /// Enum members are also registered design-wide under their bare name, and
    /// the qualified form used to be answered by discarding the scope and
    /// looking the leaf up flat. That is wrong in both directions: a local
    /// declaration legally shadowing the bare name captured the qualified
    /// reference too, and inside a function body — where the elaborator leaves
    /// the reference as a `MemberAccess` rather than collapsing it to a
    /// two-segment `Ident` — there is no leaf fallback at all, so the read
    /// returned 0. Consulting the package's own member list fixes both.
    pub(super) fn package_enum_member(&self, pkg: &str, member: &str) -> Option<Value> {
        let (val, width) = *self.module.package_enum_members.get(pkg)?.get(member)?;
        // §6.19 wide base: the table stores u64; elaboration parks the full
        // Value under the qualified spelling in `parameters` for these.
        if width > 64 {
            if let Some(v) = self.module.parameters.get(&format!("{}::{}", pkg, member)) {
                return Some(v.clone());
            }
        }
        Some(Value::from_u64(val, width.max(1)))
    }

    pub(super) fn class_scoped_const(&mut self, start_class: &str, member: &str) -> Option<Value> {
        // Collect the inheritance chain and its enum-typedef names + widths
        // under an immutable borrow, then resolve under separate borrows so
        // the later `class_static_get` (&mut self) doesn't conflict.
        let mut chain_classes: Vec<String> = Vec::new();
        let mut enum_tds: Vec<(String, u32)> = Vec::new();
        {
            let mut cur: Option<&str> = Some(start_class);
            while let Some(cn) = cur {
                chain_classes.push(cn.to_string());
                match self.module.classes.get(cn) {
                    Some(cd) => {
                        for (td_name, dt) in &cd.typedef_targets {
                            if matches!(dt, DataType::Enum(_)) {
                                let w = self
                                    .module
                                    .typedefs
                                    .get(td_name)
                                    .copied()
                                    .unwrap_or(32)
                                    .max(1);
                                enum_tds.push((td_name.clone(), w));
                            }
                        }
                        cur = cd.extends.as_deref();
                    }
                    None => break,
                }
            }
        }
        // Enum literal lookup.
        for (td_name, w) in &enum_tds {
            if let Some(members) = self.module.enum_members.get(td_name) {
                if let Some((_, val)) = members.iter().find(|(n, _)| n == member) {
                    return Some(Value::from_u64(*val, *w));
                }
            }
        }
        // §8.25: a parameter of the active specialization of this class.
        if let Some(v) = self.active_spec_param_value(start_class, member) {
            return Some(v);
        }
        // Static property / localparam fallback.
        for cn in &chain_classes {
            if let Some(v) = self.class_static_get(cn, member) {
                return Some(v);
            }
        }
        None
    }

    /// §8.3.2/§6.20.2 (issue #388): resolve a WHOLE-COLLECTION reference to a
    /// class-scope ARRAY constant — `pkg::Cls::T`, `Cls::T` from a foreign
    /// class, or bare `T` inside a method of the declaring class — for the
    /// `$size` / `inside` / `foreach` / whole-array-copy contexts. The
    /// element-read arm (`class_param_array_element`) handles one index at a
    /// time; these contexts need the bounds and an element source up front.
    /// Returns None when `expr` does not name such a constant.
    pub(super) fn resolve_class_scope_array_const(
        &mut self,
        expr: &Expression,
    ) -> Option<ClassScopeArrayConst> {
        let mut expr = expr;
        while let ExprKind::Paren(inner) = &expr.kind {
            expr = inner;
        }
        let Some(leaf) = Self::receiver_prop_name(expr) else {
            return None;
        };
        if !self.class_member_names().constant_arrays.contains(leaf) {
            // Qualified package arrays also use this path inside methods.
            // Ordinary literals, ranges, and bare scalar names return here.
            let ExprKind::MemberAccess { .. } = &expr.kind else {
                return None;
            };
        }
        // Peel a `C#(args)` wrapper first, exactly as the element-read arm
        // does, so a qualified `C#(..)::TBL` binds the specialization's cell.
        let (spec, stripped) = if Self::expr_has_specialization(expr) {
            let saved = self.current_spec.clone();
            let sp = self.resolve_call_spec_params(Self::extract_call_spec(expr), &saved);
            if let Some((ref b, ref s)) = sp {
                self.ensure_spec_statics(b, s);
            }
            (sp, Self::strip_spec_shape(expr))
        } else {
            (None, expr.clone())
        };
        let segs = Self::flatten_member_path(&stripped)?;
        let n = segs.len();
        if n == 0 {
            return None;
        }
        let leaf = segs[n - 1].clone();
        // The class the prefix denotes — longest class part first, so a
        // nested `outer::inner::LP` prefers the scoped key while a
        // package-qualified `pkg::Cls::LP` falls through to `Cls`.
        let mut start: Option<(String, Option<String>)> = None;
        if n >= 2 {
            for k in 0..n - 1 {
                if let Some(r) = self.resolve_class_scope_prefix(&segs[k..n - 1].join("::")) {
                    start = Some(r);
                    break;
                }
            }
        }
        if start.is_none() && n == 1 {
            // Bare `T`: the enclosing specialization (a method of the
            // specialization set it) or class context names the scope. A
            // bare name that resolves to a module-scope array or signal is
            // not class-scope — leave it to the ordinary arms. §23.9: a
            // method local or formal of the same name shadows the class
            // constant (the element arm applies the same rule), so `$size`,
            // `foreach` and a whole-array copy read the local.
            if self.module.arrays.contains_key(&leaf) || self.signal_name_to_id.contains_key(&leaf)
            {
                return None;
            }
            if self.dyn_name_lookup(&leaf).is_some()
                || self.local_stack.last().is_some_and(|f| {
                    f.contains_key(&leaf) || f.contains_key(&format!("\u{1}array_formal:{leaf}"))
                })
            {
                return None;
            }
            if let Some((b, s)) = self.current_spec.clone() {
                start = Some((b, Some(s)));
            } else if let Some(cls) = self.class_context_stack.last().cloned().flatten() {
                start = Some((cls, None));
            }
        }
        // §6.20.2/§26.3 (issue #388): a PACKAGE-scope array localparam —
        // `pkg::T`. Inside a class-method body that spelling parses as
        // MemberAccess, so the flat-Ident arms (`array_operand_name`, the
        // Ident/Ident fixed-to-fixed copy) never see it and the whole-array
        // contexts collapsed to zero. The shape row and per-element signals
        // live under the qualified name `pkg::T` exactly like a module
        // array, so mirror the static-fixed element source.
        if start.is_none() && n >= 2 && self.module.packages.contains(&segs[0]) {
            let joined = segs.join("::");
            if !self.module.dynamic_arrays.contains(&joined) {
                if let Some(&(lo, hi, _)) = self.module.arrays.get(&joined) {
                    // Declared order for `$left`/`$right`/`$increment` and the
                    // `foreach` visit order: the `arrays` row is normalized,
                    // so a descending `[5:4]` package array would otherwise
                    // report its bounds flipped.
                    let (left, right) = self
                        .module
                        .unpacked_decl_dims
                        .get(&joined)
                        .and_then(|d| d.first())
                        .copied()
                        .unwrap_or((lo, hi));
                    return Some(ClassScopeArrayConst {
                        key: joined,
                        lo,
                        hi,
                        left,
                        right,
                        param_cell: false,
                    });
                }
            }
        }
        let (start_class, resolved_sig) = start?;
        // The active spec: the peeled `#(...)` wrapper wins; else the
        // arguments a typedef of a specialization carries (§8.25,
        // `typedef C#(5) Alias; Alias::T` — the element arm keeps them the
        // same way); else the enclosing one when it denotes the same class.
        let spec = spec
            .or_else(|| resolved_sig.map(|s| (start_class.clone(), s)))
            .or_else(|| self.current_spec.clone().filter(|(b, _)| *b == start_class));
        // Walk the chain for the DECLARING class.
        let mut cur: Option<String> = Some(start_class);
        let mut guard = 0;
        while let Some(cn) = cur {
            guard += 1;
            if guard > 64 {
                return None;
            }
            let (param_arr, static_fixed, extends, reg_name) = {
                let Some(cd) = self.module.classes.get(&cn) else {
                    return None;
                };
                (
                    cd.array_properties.contains_key(&leaf)
                        && (cd.properties.get(&leaf).is_some_and(|s| s.is_const)
                            || cd.param_order.iter().any(|p| *p == leaf)),
                    cd.static_fixed_arrays.iter().any(|(nm, ..)| *nm == leaf),
                    cd.extends.clone(),
                    cd.name.clone(),
                )
            };
            if param_arr {
                let key = self.materialize_class_param_array(&cn, &leaf, spec.as_ref())?;
                let (lo, hi, descending) = self
                    .class_param_arrays
                    .get(&key)
                    .map(|arr| (arr.lo, arr.hi, arr.descending))
                    .unwrap_or((0, -1, false));
                // The materialized cell has this specialization's bounds;
                // the declaration metadata can still carry default parameters.
                let (left, right) = if descending { (hi, lo) } else { (lo, hi) };
                return Some(ClassScopeArrayConst {
                    key,
                    lo,
                    hi,
                    left,
                    right,
                    param_cell: true,
                });
            }
            if static_fixed {
                // Static fixed-array property: per-element signals
                // `Cls::name[i]`, shape row in `module.arrays` under
                // `Cls::name` — the class's REGISTERED name. For a class
                // declared inside a package the `classes` map key is
                // package-qualified (`pkg::Cls`) while the shared store
                // (see the registration in compilation.rs) is keyed by the
                // bare `cd.name`, so building the key from `cn` would miss.
                let key = format!("{}::{}", reg_name, leaf);
                let (lo, hi) = self.module.arrays.get(&key).map(|&(lo, hi, _)| (lo, hi))?;
                let (left, right) = self
                    .module
                    .unpacked_decl_dims
                    .get(&key)
                    .and_then(|d| d.first())
                    .copied()
                    .unwrap_or((lo, hi));
                return Some(ClassScopeArrayConst {
                    key,
                    lo,
                    hi,
                    left,
                    right,
                    param_cell: false,
                });
            }
            cur = extends;
        }
        None
    }

    /// One element of a resolved class-scope array constant (issue #388):
    /// the materialized `class_param_arrays` cell for a param/localparam
    /// constant, the per-element signal for a static fixed-array property.
    /// None when the index is out of range or the cell is missing.
    pub(super) fn class_scope_array_const_elem(
        &self,
        c: &ClassScopeArrayConst,
        idx: i64,
    ) -> Option<Value> {
        if idx < c.lo || idx > c.hi {
            return None;
        }
        if c.param_cell {
            self.class_param_arrays
                .get(&c.key)
                .and_then(|arr| arr.elems.get((idx - c.lo) as usize).cloned())
        } else {
            self.get_signal_value_by_name(&format!("{}[{}]", c.key, idx))
        }
    }

    /// Read a static class property, lazily seeding its shared cell from
    /// the declaring class's initial value on first access.
    /// For a class that is NOT parameterized itself, the specialization of
    /// its nearest parameterized ancestor it extends (`class d extends
    /// base #(int, 3);` gives `(base, "int,3,6")`): the scope its inherited
    /// parameters take their values from. None for a parameterized class or
    /// a chain without parameters.
    pub(super) fn inherited_param_spec(&self, class_name: &str) -> Option<(String, String)> {
        if self.class_is_parameterized(class_name) {
            return None;
        }
        let mut cur = self.get_class_def(class_name)?.extends.as_deref();
        let mut guard = 0;
        while let Some(anc) = cur {
            guard += 1;
            if guard > 64 {
                return None;
            }
            if self.class_is_parameterized(anc) {
                return self.static_receiver_spec(class_name, anc);
            }
            cur = self.get_class_def(anc)?.extends.as_deref();
        }
        None
    }

    pub(super) fn class_static_get(&mut self, start_class: &str, prop: &str) -> Option<Value> {
        let key = self.static_prop_key(start_class, prop)?;
        if !self.class_statics.contains_key(&key) {
            // Key head is `DeclClass` or `DeclClass#spec`; strip the spec
            // suffix to find the class whose initial value seeds the cell.
            // The class name itself may be qualified (`pkg::C`, a class
            // whose name another package also declares, §26.3).
            let head = key
                .strip_suffix(prop)
                .and_then(|h| h.strip_suffix("::"))
                .unwrap_or(&key);
            let decl_class = head.split('#').next().unwrap_or(head);
            // Collect the initial value: first try `properties` (static
            // class members), then `param_defaults` (class localparam
            // constants, e.g. `localparam string prefix = "..."`).
            // We must extract any expression clone before eval_expr to
            // avoid borrowing self.module and self simultaneously.
            let init_val: Option<Value> = {
                let (sig_val, init_expr, pd_expr): (
                    Option<Value>,
                    Option<Expression>,
                    Option<Expression>,
                ) = match self.module.classes.get(decl_class) {
                    Some(cd) => (
                        cd.properties.get(prop).map(|sig| sig.value.clone()),
                        cd.property_inits
                            .iter()
                            .find(|(p, _)| p.as_str() == prop)
                            .map(|(_, e)| e.clone()),
                        cd.param_defaults
                            .iter()
                            .find(|(name, _)| name == prop)
                            .and_then(|(_, e)| e.clone()),
                    ),
                    None => (None, None, None),
                };
                // §8.25: a parameter or localparam of the ACTIVE
                // specialization (`vp#(5)::W` with `W = N * 2`, read by a
                // compiled static method, or `prm#(4)::N` read from a module)
                // takes that specialization's value; its per-spec cell must
                // not be seeded from the elaborated default. A body
                // localparam's initializer is evaluated in its class's scope.
                let spec_param = if pd_expr.is_some()
                    && self
                        .current_spec
                        .as_ref()
                        .is_some_and(|(b, _)| self.class_origin(b) == decl_class)
                {
                    self.class_context_stack.push(Some(decl_class.to_string()));
                    self.this_stack.push(None);
                    let v = self.resolve_value_param_in_current(prop);
                    self.this_stack.pop();
                    self.class_context_stack.pop();
                    v
                } else if pd_expr.is_some()
                    && let Some((b, sig)) = self.current_spec.clone()
                    && let origin = self.class_origin(&b)
                    && origin != decl_class
                    && self.class_is_a(&origin, decl_class)
                    && let Some(asig) = self.ancestor_spec(&origin, &sig, decl_class)
                {
                    // §8.25 / §8.13: a constant declared by an ANCESTOR of
                    // the active specialization (`Derived#(10)::LP` with
                    // `LP = P + 1` in `Base`, reached through `extends
                    // Base#(Q)`) is evaluated in the ancestor's own
                    // specialization, mapped through the extends arguments;
                    // the elaborated default (`Base`'s `P = 42`) is not it.
                    let saved = std::mem::replace(
                        &mut self.current_spec,
                        Some((decl_class.to_string(), asig)),
                    );
                    self.class_context_stack.push(Some(decl_class.to_string()));
                    self.this_stack.push(None);
                    let v = self.resolve_value_param_in_current(prop);
                    self.this_stack.pop();
                    self.class_context_stack.pop();
                    self.current_spec = saved;
                    v
                } else {
                    None
                };
                if spec_param.is_some() {
                    spec_param
                } else if sig_val.is_some() {
                    // §8.25: a parameterized class's static initializer may
                    // reference the class's VALUE parameters (`const static
                    // string type_name = Tname;` — every uvm_object_registry).
                    // The elaboration-time `sig.value` was computed with NO
                    // active specialization, so for such classes it holds the
                    // unresolved default (a blank string) — and every
                    // registry then registered under the same blank name,
                    // corrupting the factory's by-name table. Re-evaluate the
                    // initializer HERE, where the key derivation has already
                    // established the active spec context.
                    if let Some(e) = init_expr {
                        // Cycle-breaker: seed the cell with the elaborated
                        // value BEFORE evaluating — an initializer chain that
                        // reads back through this static (UVM's registries
                        // reference each other during uvm_pkg init) would
                        // otherwise recurse without a base case.
                        self.class_statics
                            .insert(key.clone(), sig_val.clone().unwrap());
                        self.class_context_stack.push(Some(decl_class.to_string()));
                        self.this_stack.push(None);
                        let v = self.eval_expr(&e);
                        self.this_stack.pop();
                        self.class_context_stack.pop();
                        Some(v)
                    } else if let Some((pd, spec)) = pd_expr
                        .filter(|e| {
                            !matches!(e.kind, ExprKind::Number(_) | ExprKind::StringLiteral(_))
                        })
                        .and_then(|pd| Some((pd, self.inherited_param_spec(decl_class)?)))
                    {
                        // §6.20.2: a class-body localparam of a class that is
                        // not parameterized itself but inherits parameters
                        // (`class d extends base #(int, 3); localparam int L =
                        // D + 1;`) was elaborated without the inherited
                        // values in scope. Evaluate it with the base
                        // specialization this class extends active.
                        self.class_statics
                            .insert(key.clone(), sig_val.clone().unwrap());
                        let saved_spec = std::mem::replace(&mut self.current_spec, Some(spec));
                        let frame = self.take_pooled_frame();
                        self.class_context_stack.push(Some(decl_class.to_string()));
                        self.this_stack.push(None);
                        self.push_local_frame(frame);
                        let v = self.eval_expr(&pd);
                        self.pop_local_frame();
                        self.this_stack.pop();
                        self.class_context_stack.pop();
                        self.current_spec = saved_spec;
                        Some(v)
                    } else {
                        sig_val
                    }
                } else {
                    // Class localparam constants (e.g.
                    // `localparam string prefix = "+set_verbosity="`).
                    pd_expr.map(|init_expr| self.eval_expr(&init_expr))
                }
            };
            let init = init_val.unwrap_or_else(|| Value::zero(32));
            // A static WITHOUT an initializer holds its type's default
            // (x for 4-state, 0 for 2-state) at the cell's own width —
            // widening the default-width seed would zero-fill the new bits.
            let has_init = self.module.classes.get(decl_class).is_some_and(|cd| {
                cd.property_inits.contains_key(prop) || !cd.properties.contains_key(prop)
            });
            // §6.8: a static of a 4-state type without an initializer
            // starts x (an enum or packed struct the elaborator seeded 0).
            let seeds_x = !has_init
                && !init.is_real
                && self
                    .module
                    .classes
                    .get(decl_class)
                    .is_some_and(|cd| self.prop_seeds_x(cd, prop));
            let init = if seeds_x {
                let mut d = Value::new(init.width.max(1));
                d.is_signed = init.is_signed;
                d
            } else {
                init
            };
            let init = match self.static_cell_width(&key, prop) {
                Some((w, signed)) if !has_init && !init.is_real && w != init.width => {
                    let mut d = if init.has_xz() {
                        Value::new(w)
                    } else {
                        Value::zero(w)
                    };
                    d.is_signed = signed;
                    d
                }
                _ => self.fit_static_cell(&key, prop, init),
            };
            self.class_statics.insert(key.clone(), init);
        }
        self.class_statics.get(&key).cloned()
    }

    /// Write a static class property's shared cell. Returns false if
    /// `prop` is not a static property of `start_class` or an ancestor.
    /// LRM §25.8 — try to record an `obj.vif = iface_inst;` binding instead
    /// of doing a Value copy. Returns true if the assignment was handled
    /// (the caller should `return` immediately).
    ///
    /// Recognises two forms:
    ///   - LHS = `name` where `name` is a class-typed local whose class
    ///     declares `name` as virtual-iface (single-segment local var).
    ///   - LHS = `<obj>.<prop>` where `obj`'s class declares `prop` as a
    ///     virtual-iface property.
    /// RHS must be a single-segment Ident — the bound interface instance
    /// name. Other RHS forms fall through to the normal assignment path.
    pub(super) fn try_bind_virtual_iface(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
    ) -> bool {
        // §25.9 / §26.3: bind a package-qualified variable through the same
        // storage name used by ordinary package reads and writes.
        if !self.module.packages.is_empty()
            && let Some(stripped) = self.strip_package_lvalue(lvalue)
        {
            return self.try_bind_virtual_iface(&stripped, rvalue);
        }
        if self.try_bind_virtual_iface_inner(lvalue, rvalue) {
            return true;
        }
        // LHS is a PLAIN NAME (subroutine local, formal, static prop): no
        // (instance, prop) slot exists, so propagate the binding via the
        // __vif_local__ convention — resolve_vif_rhs_name's read side
        // already honors it. This is how a virtual interface survives
        // uvm_config_db's parameterized static set/get round-trip
        // (issue #113: the driver's vif stayed null so no stimulus ran).
        // Gated on a CONFIDENT vif RHS; ordinary assigns fall through
        // untouched (return false → the normal value assign still runs).
        if let ExprKind::Ident(h) = &lvalue.kind {
            if h.path.len() == 1 {
                // Cheap RHS shape gate FIRST: the strict resolver only ever
                // accepts Ident/MemberAccess/Index (or Null clearing a
                // binding). Everything else — literals, arithmetic, calls —
                // can skip the class-hierarchy classification walk below,
                // which otherwise ran for EVERY plain-name assignment.
                if !matches!(
                    &rvalue.kind,
                    ExprKind::Null
                        | ExprKind::Ident(_)
                        | ExprKind::MemberAccess { .. }
                        | ExprKind::Index { .. }
                ) {
                    return false;
                }
                let lname: &str = h.path[0].name.name.as_str();
                // `null` acts only on a vif property of `this` or a name that
                // holds a `__vif_local__` key (the key removal otherwise finds
                // nothing; the store generation still moves). Any other RHS
                // must be able to name an interface instance.
                if matches!(&rvalue.kind, ExprKind::Null) {
                    if !self.class_member_names().vif_props.contains(lname)
                        && !self.vif_local_names.contains(lname)
                    {
                        bump_store_gen();
                        return false;
                    }
                } else if !self.vif_rhs_possible(rvalue) {
                    return false;
                }
                // A bare name that is a PROPERTY of `this` stores per-instance
                // (each uvm_resource's `val` must keep its own interface);
                // otherwise the flat local/static key.
                // Classify the bare name against `this`:
                //   declared vif prop -> the REAL binding map (runtime
                //   @(vif.sig)/reads consult it), other prop -> per-instance
                //   string key, otherwise -> flat local/static key.
                let mut is_vif_prop_of: Option<usize> = None;
                let mut is_prop_of: Option<usize> = None;
                // A local or formal of the current frame shadows a property.
                let frame_local = self
                    .local_stack
                    .last()
                    .is_some_and(|l| l.contains_key(lname));
                if let Some(th) = self
                    .this_stack
                    .last()
                    .copied()
                    .flatten()
                    .filter(|&th| th != 0 && !frame_local)
                {
                    let mut cur: Option<&str> = self
                        .heap
                        .get(th)
                        .and_then(|o| o.as_ref())
                        .map(|i| i.class_name.as_str());
                    while let Some(cn) = cur {
                        let Some(cd) = self.module.classes.get(cn) else {
                            break;
                        };
                        if cd.virtual_iface_properties.contains_key(lname) {
                            is_vif_prop_of = Some(th);
                            break;
                        }
                        if cd.properties.contains_key(lname) {
                            is_prop_of = Some(th);
                            break;
                        }
                        cur = cd.extends.as_deref();
                    }
                }
                if let Some(th) = is_vif_prop_of {
                    if matches!(&rvalue.kind, ExprKind::Null) {
                        self.virtual_iface_bindings.remove(&(th, lname.to_string()));
                        return true;
                    }
                    if let Some(nm) = self.resolve_vif_rhs_name_strict(rvalue) {
                        self.virtual_iface_bindings
                            .insert((th, lname.to_string()), (nm, None));
                        return true;
                    }
                    return false;
                }
                let make_key = |is_prop_of: Option<usize>| -> String {
                    let mut key = String::with_capacity(lname.len() + 24);
                    key.push_str("__vif_local__");
                    if let Some(th) = is_prop_of {
                        Self::push_i64(&mut key, th as i64);
                        key.push('#');
                    }
                    key.push_str(lname);
                    key
                };
                if matches!(&rvalue.kind, ExprKind::Null) || self.vif_operand(rvalue) == Some(None)
                {
                    let key = make_key(is_prop_of);
                    self.signals.remove(&key);
                    if frame_local
                        && let Some(aliases) = self.local_iface_aliases.last_mut()
                        && let Some(bound) = aliases.get_mut(lname)
                    {
                        bound.clear();
                    }
                } else if let Some(nm) = self.resolve_vif_rhs_name_strict(rvalue) {
                    let key = make_key(is_prop_of);
                    self.vif_local_names.insert(lname.to_string());
                    self.signals.insert(key, Value::from_string(&nm));
                    if frame_local
                        && let Some(aliases) = self.local_iface_aliases.last_mut()
                        && let Some(bound) = aliases.get_mut(lname)
                    {
                        *bound = nm;
                    }
                }
            }
        }
        false
    }

    /// Whether the flat `__vif_local__<name>` key — the binding of a
    /// subroutine formal or local, keyed by bare name across frames —
    /// speaks for `name` here. A property of `this` that no local of the
    /// current frame shadows has its own per-instance key; a caller's
    /// same-named formal must not leak into it (`val == t` in
    /// uvm_resource::do_write, called from a `set(..., T val, ...)`).
    pub(super) fn vif_flat_key_applies(&self, name: &str) -> bool {
        if self
            .local_stack
            .last()
            .is_some_and(|l| l.contains_key(name))
        {
            return true;
        }
        let Some(th) = self.this_stack.last().copied().flatten() else {
            return true;
        };
        let mut cur = self
            .heap
            .get(th)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.as_str());
        while let Some(cn) = cur {
            let Some(cd) = self.module.classes.get(cn) else {
                break;
            };
            if cd.properties.contains_key(name) || cd.virtual_iface_properties.contains_key(name) {
                return false;
            }
            cur = cd.extends.as_deref();
        }
        true
    }

    /// §8.4: is the bare `name` a property of `this` (own or inherited) that
    /// no local of the current frame shadows? It then hides a same-named
    /// variable of an enclosing scope.
    pub(super) fn this_prop_hides(&self, name: &str) -> bool {
        let Some(th) = self
            .this_stack
            .last()
            .copied()
            .flatten()
            .filter(|&h| h != 0)
        else {
            return false;
        };
        if self
            .local_stack
            .last()
            .is_some_and(|l| l.contains_key(name))
        {
            return false;
        }
        let mut cur = self
            .heap
            .get(th)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.as_str());
        let mut hops = 0;
        while let Some(cn) = cur {
            hops += 1;
            let Some(cd) = self.module.classes.get(cn).filter(|_| hops < 64) else {
                break;
            };
            if cd.property_types.contains_key(name) && !cd.static_properties.contains(name) {
                return true;
            }
            cur = cd.extends.as_deref();
        }
        false
    }

    /// §23.9: does a nearer declaration hide a same-named interface instance
    /// or vif alias for a BARE name? A local or formal of the current frame,
    /// or a non-vif property of `this` (§8.5), is found before any enclosing
    /// scope's instance: a class's `bit irq` read as `irq` in its own method
    /// is that bit, whatever instance `irq` the module holds. A vif local or
    /// property is bound through `__vif_local__` / the binding map, both
    /// consulted before this.
    pub(super) fn data_name_shadows_iface(&self, raw: &str) -> bool {
        if self
            .local_iface_aliases
            .last()
            .is_some_and(|frame| frame.contains_key(raw))
        {
            return false;
        }
        if self.local_stack.last().is_some_and(|l| l.contains_key(raw)) {
            return true;
        }
        let Some(th) = self.this_stack.last().copied().flatten() else {
            return false;
        };
        let mut cur = self
            .heap
            .get(th)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.as_str());
        while let Some(cn) = cur {
            let Some(cd) = self.module.classes.get(cn) else {
                break;
            };
            if cd.virtual_iface_properties.contains_key(raw) {
                return false;
            }
            if cd.properties.contains_key(raw) {
                return true;
            }
            cur = cd.extends.as_deref();
        }
        false
    }
}
