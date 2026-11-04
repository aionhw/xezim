//! Simulator coverage.

use super::*;

impl Simulator {
    /// The covergroup definition `new` instantiates for a type name: inside
    /// a class, the class's OWN `Class::cg` (walking `extends`) wins over
    /// the bare name, so a derived class that redeclares `cg` gets its own
    /// coverpoints (§19.3); elsewhere the bare name.
    pub(super) fn covergroup_def_for(&self, tname: &str) -> Option<CovergroupDeclaration> {
        // Both lookups below can only hit a key equal to `tname` or ending in
        // `::tname`.
        if !tname.contains(':') {
            let leaves = self.covergroup_leaf_names.get_or_init(|| {
                let mut set: HashSet<String> = HashSet::default();
                for k in self.module.covergroups.keys() {
                    set.insert(k.clone());
                    if let Some((_, leaf)) = k.rsplit_once("::") {
                        set.insert(leaf.to_string());
                    }
                    // An instance's covergroup, `<inst>.cg`.
                    if let Some((_, leaf)) = k.rsplit_once('.') {
                        set.insert(leaf.to_string());
                    }
                }
                set
            });
            if !leaves.contains(tname) {
                return None;
            }
        }
        let ctx: Option<String> = self
            .this_stack
            .last()
            .copied()
            .flatten()
            .filter(|&h| h != 0)
            .and_then(|h| {
                self.heap
                    .get(h)
                    .and_then(|o| o.as_ref())
                    .map(|i| i.class_name.clone())
            })
            .or_else(|| self.class_context_stack.last().cloned().flatten());
        let mut cur = ctx;
        while let Some(cn) = cur {
            let base = cn.split('#').next().unwrap_or(&cn).to_string();
            let key = format!("{}::{}", base, tname);
            if let Some(d) = self.module.covergroups.get(&key) {
                // The instance remembers the QUALIFIED key (sampling and
                // coverage look the definition up by the instance's name).
                let mut d = d.clone();
                d.name.name = key;
                return Some(d);
            }
            cur = self
                .module
                .classes
                .get(&base)
                .and_then(|cd| cd.extends.clone());
        }
        // §19.3: a covergroup declared in a module instance is registered as
        // `<inst>.cg`; the running process's instance scope (a declaration
        // initializer carries its instance) picks that instance's type.
        if !tname.contains(':') {
            let scope = self.active_instance_scope();
            let mut p: &str = &scope;
            while !p.is_empty() {
                let key = format!("{}.{}", p, tname);
                if let Some(d) = self.module.covergroups.get(&key) {
                    return Some(d.clone());
                }
                p = p.rsplit_once('.').map(|(a, _)| a).unwrap_or("");
            }
        }
        self.module.covergroups.get(tname).cloned()
    }

    pub(super) fn instantiate_covergroup(
        &mut self,
        cg_def: &CovergroupDeclaration,
        _args: &[Expression],
    ) -> Value {
        let handle = self.cg_heap.len();
        let owner = self
            .this_stack
            .last()
            .copied()
            .flatten()
            .filter(|&h| h != 0);
        // An embedded (class-body) covergroup is one variable per object, so
        // an object re-instantiating the same covergroup type — a derived
        // constructor's `cg = new` after the base constructor's — leaves the
        // older instance unreachable. Drop it so it no longer counts in type
        // coverage (§19.11; the reference simulator does not count it).
        if owner.is_some() {
            for slot in self.cg_heap.iter_mut() {
                if slot
                    .as_ref()
                    .is_some_and(|i| i.owner == owner && i.cg_name == cg_def.name.name)
                {
                    *slot = None;
                }
            }
        }
        let instance = CovergroupInstance {
            cg_name: cg_def.name.name.clone(),
            owner,
            ctor_args: {
                let mut bound = Vec::new();
                for (i, port) in cg_def.ports.iter().enumerate() {
                    let v = if let Some(a) = _args.get(i) {
                        self.eval_expr(a)
                    } else if let Some(d) = &port.default {
                        self.eval_expr(d)
                    } else {
                        Value::zero(32)
                    };
                    bound.push((port.name.name.clone(), v));
                }
                bound
            },
            ctor_refs: cg_def
                .ports
                .iter()
                .enumerate()
                .filter(|(_, p)| matches!(p.direction, crate::ast::types::PortDirection::Ref))
                .filter_map(|(i, p)| _args.get(i).map(|a| (p.name.name.clone(), a.clone())))
                .collect(),
            class_params: owner
                .map(|h| {
                    let mut ps = self.instance_value_params(h);
                    ps.retain(|(n, _)| !cg_def.ports.iter().any(|p| &p.name.name == n));
                    ps
                })
                .unwrap_or_default(),
            point_hits: HashMap::default(),
            bin_hits: HashMap::default(),
            trans_pos: HashMap::default(),
            cross_hits: HashMap::default(),
            cross_bin_hits: HashMap::default(),
            sample_count: 0,
            stopped: false,
            stopped_items: HashSet::default(),
            options: HashMap::default(),
        };
        self.cg_heap.push(Some(instance));

        // Register automatic sampling if event is present
        if let Some(event) = &cg_def.event {
            // §19.3: `@(ev)` on an event property of the owning object.
            if let (Some(h), EventControl::EventExpr(terms)) = (owner, event) {
                if let [t] = terms.as_slice() {
                    if let ExprKind::Ident(id) = &t.expr.kind {
                        let is_prop =
                            t.edge.is_none()
                                && id.path.len() == 1
                                && self.heap.get(h).and_then(|o| o.as_ref()).is_some_and(|o| {
                                    o.properties.contains_key(&id.path[0].name.name)
                                });
                        if is_prop {
                            self.cg_instance_event_waiters
                                .entry((h, id.path[0].name.name.clone()))
                                .or_default()
                                .push((handle, t.iff.clone()));
                            return Value::from_u64((CG_HANDLE_TAG + handle) as u64, 32);
                        }
                    }
                }
            }
            let sens = self.event_to_sens(event);
            let mut resolved: Vec<SensitivityId> = Vec::new();
            for s in &sens {
                let Some(&id) = self.signal_name_to_id.get(s.signal_name.as_str()) else {
                    continue;
                };
                if self.module.events.contains(s.signal_name.as_str()) {
                    self.cg_named_event_waiters
                        .entry(id)
                        .or_default()
                        .push((handle, s.iff.as_deref().cloned()));
                    continue;
                }
                resolved.push(SensitivityId {
                    signal_id: id,
                    edge: s.edge,
                    iff: s.iff.clone(),
                    value_of: None,
                    wake_rank: 0,
                    cell: false,
                });
            }
            if !resolved.is_empty() {
                self.cg_event_waiters.push((handle, resolved));
            }
        }

        // Tagged so a covergroup handle can never be mistaken for a class
        // handle: both are small heap indices, and the receiver dispatch
        // probed `cg_heap` first — class object 1 was dispatched as
        // covergroup 1 and its method body never ran.
        Value::from_u64((CG_HANDLE_TAG + handle) as u64, 32)
    }

    /// Decode a tagged covergroup handle to its `cg_heap` index.
    pub(super) fn cg_index(&self, handle: usize) -> Option<usize> {
        if handle < CG_HANDLE_TAG {
            return None;
        }
        let i = handle - CG_HANDLE_TAG;
        if i < self.cg_heap.len() && self.cg_heap[i].is_some() {
            Some(i)
        } else {
            None
        }
    }

    /// §19.8 coverage query on one coverpoint/cross of the covergroup
    /// instance at `idx`: `get_inst_coverage` over this instance,
    /// `get_coverage` over every instance of the type. `None` when the
    /// covergroup has no item by that name.
    pub(super) fn cg_item_query(
        &self,
        idx: usize,
        item: &str,
        method: &str,
    ) -> Option<(f64, u64, u64)> {
        let inst = self.cg_heap.get(idx)?.as_ref()?;
        let cg_name = inst.cg_name.clone();
        // With `merge_instances` an instance reports its type's coverage.
        if method == "get_inst_coverage" && !self.cg_merge_instances(&cg_name) {
            return self
                .cg_named_item_counts(&cg_name, item, &[inst])
                .map(|(c, t)| (Self::cg_fraction(c, t) * 100.0, c, t));
        }
        self.cg_type_item_counts(&cg_name, item)
    }

    /// The live instances of a covergroup type.
    pub(super) fn cg_type_instances(&self, cg_name: &str) -> Vec<&CovergroupInstance> {
        self.cg_heap
            .iter()
            .flatten()
            .filter(|i| i.cg_name == cg_name)
            .collect()
    }

    /// §19.8/§19.11 TYPE coverage of one coverpoint or cross: over the
    /// instances' hits added up with `merge_instances`, else the average of
    /// the instances, with their bin counts summed.
    pub(super) fn cg_type_item_counts(&self, cg_name: &str, item: &str) -> Option<(f64, u64, u64)> {
        let insts = self.cg_type_instances(cg_name);
        if self.cg_merge_instances(cg_name) || insts.is_empty() {
            let (c, t) = self.cg_named_item_counts(cg_name, item, &insts)?;
            return Some((Self::cg_fraction(c, t) * 100.0, c, t));
        }
        let (mut pct, mut c, mut t) = (0.0, 0, 0);
        for i in &insts {
            let (ic, it) = self.cg_named_item_counts(cg_name, item, std::slice::from_ref(i))?;
            pct += Self::cg_fraction(ic, it) * 100.0;
            c += ic;
            t += it;
        }
        Some((pct / insts.len() as f64, c, t))
    }

    /// Coverage (percent) and covered / total bin counts of instance
    /// `handle`, summed over its coverpoints and crosses; with
    /// `merge_instances`, those of its type.
    pub(super) fn cg_instance_counts(&self, handle: usize) -> (f64, u64, u64) {
        let Some(inst) = self.cg_heap.get(handle).and_then(|x| x.as_ref()) else {
            return (0.0, 0, 0);
        };
        if self.cg_merge_instances(&inst.cg_name) {
            return self.cg_type_counts(&inst.cg_name);
        }
        let (c, t) = self.cg_bin_totals(&inst.cg_name, &[inst]);
        (self.calculate_coverage(handle), c, t)
    }

    /// Type coverage (percent) and bin counts: of the instances' hits added
    /// up with `merge_instances`, else summed over the instances.
    pub(super) fn cg_type_counts(&self, cg_name: &str) -> (f64, u64, u64) {
        let insts = self.cg_type_instances(cg_name);
        let (c, t) = if self.cg_merge_instances(cg_name) {
            self.cg_bin_totals(cg_name, &insts)
        } else {
            insts.iter().fold((0, 0), |(c, t), i| {
                let (ic, it) = self.cg_bin_totals(cg_name, std::slice::from_ref(i));
                (c + ic, t + it)
            })
        };
        (self.calculate_type_coverage(cg_name), c, t)
    }

    /// Covered and total bins over every coverpoint and cross of a
    /// covergroup (implicit coverpoints included).
    pub(super) fn cg_bin_totals(&self, cg_name: &str, insts: &[&CovergroupInstance]) -> (u64, u64) {
        let Some(def) = self.module.covergroups.get(cg_name) else {
            return (0, 0);
        };
        let (mut covered, mut total) = (0u64, 0u64);
        for item in &def.items {
            if let Some((c, t, _)) = self.cg_item_counts(def, item, insts) {
                covered += c;
                total += t;
            }
        }
        let at_least = self.cg_at_least(def, &[], insts);
        for name in Self::cg_implicit_points(def) {
            let (c, t) = Self::cg_implicit_point_counts(&name, insts, at_least);
            covered += c;
            total += t;
        }
        (covered, total)
    }

    /// Write a coverage query's `covered` / `total` output arguments.
    pub(super) fn cg_assign_counts(&mut self, args: &[Expression], covered: u64, total: u64) {
        if let [c, t, ..] = args {
            self.assign_value(c, &Value::from_u64(covered, 32));
            self.assign_value(t, &Value::from_u64(total, 32));
        }
    }

    /// `cg_type::get_coverage()` / `cg_type::cp::get_coverage()`: the
    /// covergroup type (key) and item a receiver names, when it names a
    /// covergroup type rather than a variable.
    pub(super) fn cg_type_receiver(&self, recv: &Expression) -> Option<(String, Option<String>)> {
        let (name, item) = match &recv.kind {
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                (h.path[0].name.name.as_str(), None)
            }
            ExprKind::Ident(h)
                if h.path.len() == 2 && h.path.iter().all(|s| s.selects.is_empty()) =>
            {
                (
                    h.path[0].name.name.as_str(),
                    Some(h.path[1].name.name.clone()),
                )
            }
            ExprKind::MemberAccess { expr, member } => match &expr.kind {
                ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                    (h.path[0].name.name.as_str(), Some(member.name.clone()))
                }
                _ => return None,
            },
            _ => return None,
        };
        // A variable (or a class's own covergroup variable) is an instance.
        if self
            .local_stack
            .last()
            .is_some_and(|l| l.contains_key(name))
            || self.signal_name_to_id.contains_key(name)
            || self
                .this_stack
                .last()
                .copied()
                .flatten()
                .and_then(|h| self.heap.get(h).and_then(|o| o.as_ref()))
                .is_some_and(|o| o.properties.contains_key(name))
        {
            return None;
        }
        let def = self.covergroup_def_for(name)?;
        Some((def.name.name, item))
    }

    pub(super) fn exec_cg_method_call(
        &mut self,
        handle: usize,
        method_name: &str,
        _args: &[Expression],
    ) -> Value {
        match method_name {
            // IEEE 1800-2017 §19.8: both coverage queries return a REAL
            // percentage. This used to return the number as an INTEGER
            // Value (from_u64), so `%f`-family display reinterpreted the
            // integer's bits as an f64 and printed 0.0 even when the
            // computed coverage was 100.
            "get_inst_coverage" | "get_coverage" => {
                // §19.8: get_inst_coverage() covers THIS instance,
                // get_coverage() the covergroup TYPE. The optional
                // `(covered, total)` arguments receive the bin counts.
                let Some(cg_name) = self
                    .cg_heap
                    .get(handle)
                    .and_then(|x| x.as_ref())
                    .map(|i| i.cg_name.clone())
                else {
                    return Value::from_f64(0.0);
                };
                let (pct, covered, total) = if method_name == "get_inst_coverage" {
                    self.cg_instance_counts(handle)
                } else {
                    self.cg_type_counts(&cg_name)
                };
                self.cg_assign_counts(_args, covered, total);
                Value::from_f64(pct)
            }
            "start" | "stop" => {
                // §19.8: a stopped instance ignores its samples.
                if let Some(Some(inst)) = self.cg_heap.get_mut(handle) {
                    inst.stopped = method_name == "stop";
                }
                Value::zero(32)
            }
            "sample" => {
                // §19.8.1 `with function sample(formals)`: bind the call's
                // arguments to the formals (fitted to their declared width)
                // for the duration of this sample. They were dropped before,
                // so every coverpoint over a sample argument read x.
                let formals: Vec<crate::ast::decl::FunctionPort> = self
                    .cg_heap
                    .get(handle)
                    .and_then(|x| x.as_ref())
                    .and_then(|i| self.module.covergroups.get(&i.cg_name))
                    .map(|d| d.sample_ports.clone())
                    .unwrap_or_default();
                let mut frame: HashMap<String, Value> = HashMap::default();
                for (i, port) in formals.iter().enumerate() {
                    let mut v = match _args.get(i) {
                        Some(a) => self.eval_expr(a),
                        None => match &port.default {
                            Some(d) => self.eval_expr(d),
                            None => Value::zero(32),
                        },
                    };
                    if let Some((w, signed)) = self.scalar_formal_integral(&port.data_type) {
                        if w > 0 && w != v.width {
                            v = v.resize(w);
                        }
                        v.is_signed = signed;
                    }
                    frame.insert(port.name.name.clone(), v);
                }
                self.sample_covergroup_with(handle, frame, None);
                Value::zero(32)
            }
            _ => Value::zero(32),
        }
    }

    /// §19.8 get_inst_coverage(): coverage of a single instance.
    pub(super) fn calculate_coverage(&self, handle: usize) -> f64 {
        let inst = if let Some(Some(i)) = self.cg_heap.get(handle) {
            i
        } else {
            return 0.0;
        };
        self.coverage_over_instances(&inst.cg_name, std::slice::from_ref(&inst))
    }

    /// §19.8/§19.11 get_coverage(): TYPE coverage — merged over every
    /// live instance of the covergroup type (a bin counts as covered
    /// when ANY instance hit it).
    pub(super) fn calculate_type_coverage(&self, cg_name: &str) -> f64 {
        let insts: Vec<&CovergroupInstance> = self
            .cg_heap
            .iter()
            .flatten()
            .filter(|i| i.cg_name == cg_name)
            .collect();
        if insts.is_empty() {
            return 0.0;
        }
        if self.cg_merge_instances(cg_name) {
            self.coverage_over_instances(cg_name, &insts)
        } else {
            // §19.11 `type_option.merge_instances = 0` (the default): type
            // coverage is the average of the instances' coverages — each
            // instance may bin differently (constructor arguments).
            insts
                .iter()
                .map(|i| self.coverage_over_instances(cg_name, std::slice::from_ref(i)))
                .sum::<f64>()
                / insts.len() as f64
        }
    }

    /// `type_option.merge_instances = 1`: type coverage is computed over
    /// the union of the instances' bins instead of averaged.
    pub(super) fn cg_merge_instances(&self, cg_name: &str) -> bool {
        if let Some(v) = self
            .cg_type_options
            .get(cg_name)
            .and_then(|t| t.get("merge_instances"))
        {
            return v.to_u64().unwrap_or(0) != 0;
        }
        self.module.covergroups.get(cg_name).is_some_and(|d| {
            d.items.iter().any(|it| {
                matches!(it, CovergroupItem::TypeOption { name, val }
                    if name == "merge_instances"
                        && super::elaborate::const_eval_i64_with_params(val, None).unwrap_or(0) != 0)
            })
        })
    }

    /// IEEE 1800-2017 §19.8/§19.11 coverage of a covergroup over `insts`:
    /// one instance for get_inst_coverage, every instance of the type for a
    /// merged get_coverage.
    ///
    /// Each coverpoint and cross contributes the fraction of its bins that
    /// were hit at least `option.at_least` times (hits summed over `insts`),
    /// weighted by its `option.weight`; a crossed variable with no
    /// coverpoint of its own is an implicit coverpoint of weight 1 (§19.6).
    /// A covergroup with no items has 100% coverage.
    pub(super) fn coverage_over_instances(
        &self,
        cg_name: &str,
        insts: &[&CovergroupInstance],
    ) -> f64 {
        let def = if let Some(d) = self.module.covergroups.get(cg_name) {
            d
        } else {
            return 0.0;
        };
        let mut total_weight = 0.0f64;
        let mut coverage_sum = 0.0f64;
        for item in &def.items {
            if let Some((covered, total, weight)) = self.cg_item_counts(def, item, insts) {
                total_weight += weight;
                coverage_sum += Self::cg_fraction(covered, total) * weight;
            }
        }
        let at_least = self.cg_at_least(def, &[], insts);
        for name in Self::cg_implicit_points(def) {
            let (covered, total) = Self::cg_implicit_point_counts(&name, insts, at_least);
            total_weight += 1.0;
            coverage_sum += Self::cg_fraction(covered, total);
        }
        if total_weight == 0.0 {
            100.0
        } else {
            (coverage_sum / total_weight) * 100.0
        }
    }

    pub(super) fn cg_fraction(covered: u64, total: u64) -> f64 {
        if total == 0 {
            0.0
        } else {
            (covered as f64 / total as f64).min(1.0)
        }
    }

    /// Crossed variables that have no coverpoint of their own (§19.6), in
    /// first-seen order.
    pub(super) fn cg_implicit_points(def: &crate::ast::decl::CovergroupDeclaration) -> Vec<String> {
        let declared: HashSet<&str> = def
            .items
            .iter()
            .filter_map(|it| match it {
                CovergroupItem::Coverpoint(cp) => cp.name.as_ref().map(|n| n.name.as_str()),
                _ => None,
            })
            .collect();
        let mut out: Vec<String> = Vec::new();
        for it in &def.items {
            if let CovergroupItem::Cross(cr) = it {
                for id in &cr.items {
                    if !declared.contains(id.name.as_str()) && !out.contains(&id.name) {
                        out.push(id.name.clone());
                    }
                }
            }
        }
        out
    }

    /// Automatic-bin shape over a sampled width (§19.5.1): (value space,
    /// values per bin, bin count).
    pub(super) fn cg_auto_shape(width: u32, auto_bin_max: u64) -> (u64, u64, u64) {
        let space = if width >= 63 { u64::MAX } else { 1u64 << width };
        if space <= auto_bin_max {
            (space, 1, space)
        } else {
            (space, (space / auto_bin_max).max(1), auto_bin_max)
        }
    }

    /// The automatic bin of value `v`, or None outside the value space.
    pub(super) fn cg_auto_bin_of(v: i64, (space, group, nbins): (u64, u64, u64)) -> Option<u64> {
        if v < 0 || (space != u64::MAX && v as u64 >= space) {
            return None;
        }
        Some((v as u64 / group).min(nbins.saturating_sub(1)))
    }

    /// Width of the values sampled for `name`, else `hint`, else 1.
    pub(super) fn cg_sampled_width(
        name: &str,
        insts: &[&CovergroupInstance],
        hint: Option<u32>,
    ) -> u32 {
        insts
            .iter()
            .filter_map(|i| i.point_hits.get(name))
            .flat_map(|h| h.keys())
            .next()
            .map(|v| v.width)
            .or(hint)
            .unwrap_or(1)
    }

    /// Covered and total automatic bins of an implicit coverpoint (§19.6).
    pub(super) fn cg_implicit_point_counts(
        name: &str,
        insts: &[&CovergroupInstance],
        at_least: u64,
    ) -> (u64, u64) {
        let shape = Self::cg_auto_shape(Self::cg_sampled_width(name, insts, None), 64);
        let mut hits: HashMap<u64, u64> = HashMap::default();
        for i in insts {
            for (v, &c) in i.point_hits.get(name).into_iter().flatten() {
                if v.has_xz() {
                    continue;
                }
                if let Some(b) = Self::cg_auto_bin_of(v.to_u64().unwrap_or(0) as i64, shape) {
                    *hits.entry(b).or_insert(0) += c;
                }
            }
        }
        let covered = hits.values().filter(|&&c| c >= at_least).count() as u64;
        (covered, shape.2)
    }

    /// Covered and total bins of ONE coverpoint or cross.
    pub(super) fn cg_named_item_counts(
        &self,
        cg_name: &str,
        item_name: &str,
        insts: &[&CovergroupInstance],
    ) -> Option<(u64, u64)> {
        let def = self.module.covergroups.get(cg_name)?;
        let item = def.items.iter().find(|it| match it {
            CovergroupItem::Coverpoint(cp) => cp.name.as_ref().is_some_and(|n| n.name == item_name),
            CovergroupItem::Cross(cr) => cr.name.as_ref().is_some_and(|n| n.name == item_name),
            _ => false,
        });
        match item {
            Some(item) => self
                .cg_item_counts(def, item, insts)
                .map(|(c, t, _)| (c, t)),
            None if Self::cg_implicit_points(def).iter().any(|n| n == item_name) => {
                let at_least = self.cg_at_least(def, &[], insts);
                Some(Self::cg_implicit_point_counts(item_name, insts, at_least))
            }
            None => None,
        }
    }

    /// Constant bounds of one bin range (`v` or `[lo:hi]`), with §19.3
    /// constructor formals and parameters visible.
    pub(super) fn cg_range_bounds(
        &self,
        r: &ConstraintRange,
        ctor: &[(String, Value)],
    ) -> Option<(i64, i64)> {
        let ev = |e: &Expression| self.cg_const_i64(e, ctor);
        match r {
            ConstraintRange::Value(e) => ev(e).map(|v| (v, v)),
            ConstraintRange::Range { lo, hi } => {
                let (l, h) = (ev(lo)?, ev(hi)?);
                Some((l.min(h), l.max(h)))
            }
        }
    }

    /// Constant value of a bin bound or option: constructor formals and the
    /// owning class's value parameters first, then the design's parameters,
    /// then both together (`[K:N+M]` mixing the two scopes).
    pub(super) fn cg_const_i64(&self, e: &Expression, ctor: &[(String, Value)]) -> Option<i64> {
        if ctor.is_empty() {
            return super::elaborate::const_eval_i64_with_params(e, Some(&self.module.parameters));
        }
        let params: HashMap<String, Value> = ctor.iter().cloned().collect();
        if let Some(v) = super::elaborate::const_eval_i64_with_params(e, Some(&params)) {
            return Some(v);
        }
        if let Some(v) =
            super::elaborate::const_eval_i64_with_params(e, Some(&self.module.parameters))
        {
            return Some(v);
        }
        let mut all = self.module.parameters.clone();
        all.extend(params);
        super::elaborate::const_eval_i64_with_params(e, Some(&all))
    }

    /// The names a bin bound or option of covergroup instance `inst` sees
    /// besides the design's parameters: its constructor formals, then the
    /// owning specialization's value parameters (§19.5, §8.25).
    pub(super) fn cg_bin_scope(
        inst: &CovergroupInstance,
    ) -> std::borrow::Cow<'_, [(String, Value)]> {
        if inst.class_params.is_empty() {
            return std::borrow::Cow::Borrowed(inst.ctor_args.as_slice());
        }
        let mut v = inst.class_params.clone();
        v.extend(inst.ctor_args.iter().cloned());
        std::borrow::Cow::Owned(v)
    }

    /// §19.7 option `name` of a coverpoint or cross (`item_opts`), else
    /// the covergroup's own `option.name`.
    pub(super) fn cg_option_i64(
        &self,
        def: &crate::ast::decl::CovergroupDeclaration,
        item_opts: &[(String, Expression)],
        name: &str,
        ctor: &[(String, Value)],
    ) -> Option<i64> {
        item_opts
            .iter()
            .find_map(|(n, v)| (n == name).then(|| self.cg_const_i64(v, ctor)).flatten())
            .or_else(|| {
                def.items.iter().find_map(|it| match it {
                    CovergroupItem::Option { name: n, val } if n == name => {
                        self.cg_const_i64(val, ctor)
                    }
                    _ => None,
                })
            })
    }

    /// §19.7 `option.at_least` (default 1) of an item.
    pub(super) fn cg_at_least(
        &self,
        def: &crate::ast::decl::CovergroupDeclaration,
        item_opts: &[(String, Expression)],
        insts: &[&CovergroupInstance],
    ) -> u64 {
        let scope = insts
            .first()
            .map(|i| Self::cg_bin_scope(i))
            .unwrap_or_default();
        let ctor: &[(String, Value)] = &scope;
        self.cg_option_i64(def, item_opts, "at_least", ctor)
            .unwrap_or(1)
            .max(1) as u64
    }

    /// §19.5.6 / §19.5.7: values of `ignore_bins` and `illegal_bins` are
    /// excluded from every other bin of the coverpoint (explicit, array
    /// and automatic), so they count neither toward the bins nor toward
    /// the coverage denominator. Merged, sorted, inclusive ranges.
    pub(super) fn cg_excluded_ranges(
        &self,
        cp: &crate::ast::decl::Coverpoint,
        ctor: &[(String, Value)],
    ) -> Vec<(i64, i64)> {
        let v: Vec<(i64, i64)> = cp
            .bins
            .iter()
            .filter(|b| {
                matches!(
                    b.kind,
                    crate::ast::decl::CoverBinKind::Ignore
                        | crate::ast::decl::CoverBinKind::Illegal
                ) && !b.is_wildcard
                    && b.transitions.is_empty()
            })
            .flat_map(|b| self.cg_bin_value_ranges(b, ctor))
            .collect();
        Self::cg_merge_ranges(v)
    }

    pub(super) fn cg_merge_ranges(mut v: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
        v.sort_unstable();
        let mut merged: Vec<(i64, i64)> = Vec::new();
        for (l, h) in v {
            match merged.last_mut() {
                Some(last) if l <= last.1.saturating_add(1) => last.1 = last.1.max(h),
                _ => merged.push((l, h)),
            }
        }
        merged
    }

    pub(super) fn cg_in_ranges(v: i64, ranges: &[(i64, i64)]) -> bool {
        ranges.iter().any(|&(l, h)| v >= l && v <= h)
    }

    /// Number of values in `[lo, hi]` not covered by `excl` (merged ranges).
    pub(super) fn cg_count_outside(lo: i64, hi: i64, excl: &[(i64, i64)]) -> u64 {
        if hi < lo {
            return 0;
        }
        let mut n = (hi - lo) as u64 + 1;
        for &(el, eh) in excl {
            let (a, b) = (el.max(lo), eh.min(hi));
            if a <= b {
                n -= (b - a) as u64 + 1;
            }
        }
        n
    }

    /// The bins of a coverpoint that count toward coverage (§19.5): its
    /// explicit `bins`, or automatic bins when it declares none.
    pub(super) fn cg_point_shape(
        &self,
        def: &crate::ast::decl::CovergroupDeclaration,
        cp: &crate::ast::decl::Coverpoint,
        insts: &[&CovergroupInstance],
        width_hint: Option<u32>,
    ) -> CpShape {
        let scope = insts
            .first()
            .map(|i| Self::cg_bin_scope(i))
            .unwrap_or_default();
        let ctor: &[(String, Value)] = &scope;
        let excl = self.cg_excluded_ranges(cp, ctor);
        let cp_name = Self::cg_point_name(cp);
        let explicit: Vec<&crate::ast::decl::CoverBin> = cp
            .bins
            .iter()
            .filter(|b| matches!(b.kind, crate::ast::decl::CoverBinKind::Bins))
            .collect();
        if explicit.is_empty() {
            let auto_bin_max = self
                .cg_option_i64(def, &cp.options, "auto_bin_max", ctor)
                .filter(|&n| n > 0)
                .unwrap_or(64) as u64;
            let (space, group, nbins) = Self::cg_auto_shape(
                Self::cg_sampled_width(&cp_name, insts, width_hint),
                auto_bin_max,
            );
            // One-value bins wholly excluded by ignore/illegal values are
            // not bins; wider automatic bins always are.
            let mut bins = Vec::new();
            if group == 1 && space <= 1 << 20 {
                for v in 0..space as i64 {
                    if !Self::cg_in_ranges(v, &excl) {
                        bins.push(CgBin {
                            key: None,
                            name: format!("auto[{}]", v),
                            ranges: vec![(v, v)],
                        });
                    }
                }
            } else {
                for k in 0..nbins {
                    let lo = (k * group) as i64;
                    let hi = if k + 1 == nbins {
                        space.saturating_sub(1).min(i64::MAX as u64) as i64
                    } else {
                        ((k + 1) * group - 1) as i64
                    };
                    bins.push(CgBin {
                        key: None,
                        name: format!("auto[{}]", k),
                        ranges: vec![(lo, hi)],
                    });
                }
            }
            return CpShape {
                bins,
                auto: Some((space, group, nbins)),
                excl,
            };
        }
        let mut bins = Vec::new();
        for b in &explicit {
            let key = format!("{}.{}", cp_name, b.name.name);
            let constant = b
                .values
                .iter()
                .all(|r| self.cg_range_bounds(r, ctor).is_some());
            let ranges = self.cg_bin_value_ranges(b, ctor);
            if b.array_form && !b.transitions.is_empty() {
                // §19.5.2 `name[] = (...)`: one bin per value sequence.
                for (label, _) in self.cg_trans_sequences(b, ctor) {
                    bins.push(CgBin {
                        key: Some(format!("{}[{}]", key, label.unwrap_or_default())),
                        name: b.name.name.clone(),
                        ranges: Vec::new(),
                    });
                }
            } else if b.array_form && b.array_size.is_some() && !b.is_wildcard {
                // §19.5.1 `name[N]`: bins `cp.name[0]` .. `cp.name[N-1]`; one
                // whose values are all excluded, or that got none, is not a
                // bin.
                if let Some(fa) = self.cg_fixed_array(b, ctor) {
                    for j in 0..fa.0 {
                        let r = Self::cg_fixed_bin_ranges(j, &fa);
                        if r.iter()
                            .any(|&(l, h)| Self::cg_count_outside(l, h, &excl) > 0)
                        {
                            bins.push(CgBin {
                                key: Some(format!("{}[{}]", key, j)),
                                name: b.name.name.clone(),
                                ranges: r,
                            });
                        }
                    }
                }
            } else if b.array_form && !b.is_wildcard {
                // §19.5.1 `name[]`: one bin per value, `cp.name[<value>]`;
                // §19.5.1.1 only the values its `with` filter admits.
                let mut n = 0u64;
                for (l, h) in Self::cg_merge_ranges(ranges) {
                    for v in l..=h {
                        if n >= 1 << 20 {
                            break;
                        }
                        if !Self::cg_in_ranges(v, &excl) {
                            bins.push(CgBin {
                                key: Some(format!("{}[{}]", key, v)),
                                name: b.name.name.clone(),
                                ranges: vec![(v, v)],
                            });
                            n += 1;
                        }
                    }
                }
            } else if b.values.is_empty()
                || !constant
                || ranges
                    .iter()
                    .any(|&(l, h)| Self::cg_count_outside(l, h, &excl) > 0)
            {
                // A scalar bin exists unless every one of its values is
                // excluded or filtered out by its `with` (bins whose ranges
                // are not constants, and transition/wildcard bins, always
                // count).
                bins.push(CgBin {
                    key: Some(key),
                    name: b.name.name.clone(),
                    ranges,
                });
            }
        }
        CpShape {
            bins,
            auto: None,
            excl,
        }
    }

    /// Hits of each bin of `shape` for the coverpoint `cp_name`, summed
    /// over `insts`.
    pub(super) fn cg_shape_hits(
        cp_name: &str,
        shape: &CpShape,
        insts: &[&CovergroupInstance],
    ) -> Vec<u64> {
        let mut hits = vec![0u64; shape.bins.len()];
        match shape.auto {
            Some(auto) => {
                let index: HashMap<i64, usize> = shape
                    .bins
                    .iter()
                    .enumerate()
                    .map(|(k, b)| (b.ranges[0].0, k))
                    .collect();
                for i in insts {
                    for (v, &c) in i.point_hits.get(cp_name).into_iter().flatten() {
                        if v.has_xz() {
                            continue;
                        }
                        let v = v.to_u64().unwrap_or(0) as i64;
                        if Self::cg_in_ranges(v, &shape.excl) {
                            continue;
                        }
                        if let Some(b) = Self::cg_auto_bin_of(v, auto) {
                            let lo = (b * auto.1) as i64;
                            if let Some(&k) = index.get(&lo) {
                                hits[k] += c;
                            }
                        }
                    }
                }
            }
            None => {
                for (k, b) in shape.bins.iter().enumerate() {
                    if let Some(key) = &b.key {
                        hits[k] = insts
                            .iter()
                            .filter_map(|i| i.bin_hits.get(key))
                            .sum::<u64>();
                    }
                }
            }
        }
        hits
    }

    /// The name a coverpoint is recorded under.
    pub(super) fn cg_point_name(cp: &crate::ast::decl::Coverpoint) -> String {
        cp.name
            .as_ref()
            .map(|n| n.name.clone())
            .unwrap_or_else(|| format!("{:?}", cp.expr))
    }

    /// The name a cross is recorded under.
    pub(super) fn cg_cross_name(cr: &crate::ast::decl::Cross) -> String {
        cr.name.as_ref().map(|n| n.name.clone()).unwrap_or_else(|| {
            cr.items
                .iter()
                .map(|i| i.name.as_str())
                .collect::<Vec<_>>()
                .join("_")
        })
    }

    /// The bins a crossed item contributes (§19.6): its coverpoint's bins,
    /// or automatic bins of the sampled width for a crossed variable.
    pub(super) fn cg_cross_axis(
        &self,
        def: &crate::ast::decl::CovergroupDeclaration,
        name: &str,
        insts: &[&CovergroupInstance],
        width_hint: Option<u32>,
    ) -> CpShape {
        let cp = def.items.iter().find_map(|d| match d {
            CovergroupItem::Coverpoint(cp) if cp.name.as_ref().is_some_and(|n| n.name == name) => {
                Some(cp)
            }
            _ => None,
        });
        match cp {
            Some(cp) => self.cg_point_shape(def, cp, insts, width_hint),
            None => {
                let (space, group, nbins) =
                    Self::cg_auto_shape(Self::cg_sampled_width(name, insts, width_hint), 64);
                let bins = (0..nbins)
                    .map(|k| {
                        let lo = (k * group) as i64;
                        let hi = if k + 1 == nbins {
                            space.saturating_sub(1).min(i64::MAX as u64) as i64
                        } else {
                            ((k + 1) * group - 1) as i64
                        };
                        CgBin {
                            key: None,
                            name: format!("auto[{}]", k),
                            ranges: vec![(lo, hi)],
                        }
                    })
                    .collect();
                CpShape {
                    bins,
                    auto: Some((space, group, nbins)),
                    excl: Vec::new(),
                }
            }
        }
    }

    /// Indices of the bins of `shape` that value `v` falls into.
    pub(super) fn cg_axis_bins_of(shape: &CpShape, v: i64) -> Vec<usize> {
        if Self::cg_in_ranges(v, &shape.excl) {
            return Vec::new();
        }
        if let Some(auto) = shape.auto {
            let Some(b) = Self::cg_auto_bin_of(v, auto) else {
                return Vec::new();
            };
            let lo = (b * auto.1) as i64;
            return shape
                .bins
                .iter()
                .position(|x| x.ranges[0].0 == lo)
                .into_iter()
                .collect();
        }
        shape
            .bins
            .iter()
            .enumerate()
            .filter(|(_, b)| Self::cg_in_ranges(v, &b.ranges))
            .map(|(k, _)| k)
            .collect()
    }

    /// Covered and total bin counts, and the weight, of one covergroup item
    /// over `insts` (§19.8/§19.11). `None` for option items.
    pub(super) fn cg_item_counts(
        &self,
        def: &crate::ast::decl::CovergroupDeclaration,
        item: &CovergroupItem,
        insts: &[&CovergroupInstance],
    ) -> Option<(u64, u64, f64)> {
        let scope = insts
            .first()
            .map(|i| Self::cg_bin_scope(i))
            .unwrap_or_default();
        let ctor: &[(String, Value)] = &scope;
        match item {
            CovergroupItem::Coverpoint(cp) => {
                let at_least = self.cg_at_least(def, &cp.options, insts);
                let cp_name = Self::cg_point_name(cp);
                let shape = self.cg_point_shape(def, cp, insts, None);
                let hits = Self::cg_shape_hits(&cp_name, &shape, insts);
                let covered = hits.iter().filter(|&&c| c >= at_least).count() as u64;
                // §19.7 option.weight (default 1) weights this coverpoint in
                // the covergroup's mean.
                let weight = cp
                    .options
                    .iter()
                    .find_map(|(n, v)| {
                        (n == "weight")
                            .then(|| self.cg_const_i64(v, ctor))
                            .flatten()
                    })
                    .unwrap_or(1)
                    .max(0) as f64;
                Some((covered, shape.bins.len() as u64, weight))
            }
            CovergroupItem::Cross(cr) => {
                let cr_name = Self::cg_cross_name(cr);
                let at_least = self.cg_at_least(def, &cr.options, insts);
                let weight = cr
                    .options
                    .iter()
                    .find_map(|(n, v)| {
                        (n == "weight")
                            .then(|| self.cg_const_i64(v, ctor))
                            .flatten()
                    })
                    .unwrap_or(1)
                    .max(0) as f64;
                // §19.6 automatic cross bins: the Cartesian product of the
                // crossed items' bins; a sampled tuple hits the product of
                // the bins each of its values falls into. §19.6.1: the
                // products a user bin selects belong to that bin, and the
                // products an ignore_bins or illegal_bins selects to none;
                // the remaining products stay automatic bins.
                let axes: Vec<CpShape> = cr
                    .items
                    .iter()
                    .map(|it| self.cg_cross_axis(def, &it.name, insts, None))
                    .collect();
                let n_prod = axes
                    .iter()
                    .fold(1u64, |t, a| t.saturating_mul(a.bins.len() as u64));
                let mut hits: HashMap<Vec<usize>, u64> = HashMap::default();
                for i in insts {
                    for (t, &c) in i.cross_hits.get(&cr_name).into_iter().flatten() {
                        for prod in Self::cg_tuple_products(&axes, t) {
                            *hits.entry(prod).or_insert(0) += c;
                        }
                    }
                }
                if cr.bins.is_empty() {
                    let covered = hits.values().filter(|&&c| c >= at_least).count() as u64;
                    return Some((covered, n_prod, weight));
                }
                let user: Vec<&crate::ast::decl::CrossBin> = cr
                    .bins
                    .iter()
                    .filter(|b| matches!(b.kind, crate::ast::decl::CoverBinKind::Bins))
                    .collect();
                let mut nonempty = vec![false; user.len()];
                let mut auto_total = 0u64;
                if n_prod > 0 && n_prod <= 1 << 20 {
                    let mut prod = vec![0usize; axes.len()];
                    'enumerate: loop {
                        match self.cg_cross_owner(cr, &axes, &prod, ctor, None) {
                            CrossOwner::Excluded(_) => {}
                            CrossOwner::Auto => auto_total += 1,
                            CrossOwner::User(js) => {
                                for j in js {
                                    nonempty[j] = true;
                                }
                            }
                        }
                        for k in (0..prod.len()).rev() {
                            prod[k] += 1;
                            if prod[k] < axes[k].bins.len() {
                                continue 'enumerate;
                            }
                            prod[k] = 0;
                        }
                        break;
                    }
                } else if n_prod > 0 {
                    auto_total = n_prod;
                    nonempty.iter_mut().for_each(|n| *n = true);
                }
                let mut covered = 0u64;
                for (prod, &c) in &hits {
                    if c >= at_least
                        && matches!(
                            self.cg_cross_owner(cr, &axes, prod, ctor, None),
                            CrossOwner::Auto
                        )
                    {
                        covered += 1;
                    }
                }
                for (j, b) in user.iter().enumerate() {
                    let key = format!("{}.{}", cr_name, b.name.name);
                    if nonempty[j]
                        && insts
                            .iter()
                            .filter_map(|i| i.cross_bin_hits.get(&key))
                            .sum::<u64>()
                            >= at_least
                    {
                        covered += 1;
                    }
                }
                let total = auto_total + nonempty.iter().filter(|&&n| n).count() as u64;
                Some((covered, total, weight))
            }
            _ => None,
        }
    }

    /// Which bins own a cross product (§19.6.1): an ignore_bins or
    /// illegal_bins selecting it removes it; otherwise the user bins that
    /// select it, else it is an automatic bin. `guards` (at sampling) masks
    /// the bins whose `iff` is false.
    pub(super) fn cg_cross_owner(
        &self,
        cr: &crate::ast::decl::Cross,
        axes: &[CpShape],
        prod: &[usize],
        ctor: &[(String, Value)],
        guards: Option<&[bool]>,
    ) -> CrossOwner {
        let mut user = Vec::new();
        let mut j = 0usize;
        for (bi, b) in cr.bins.iter().enumerate() {
            let live = guards.is_none_or(|g| g.get(bi).copied().unwrap_or(true));
            match b.kind {
                crate::ast::decl::CoverBinKind::Ignore
                | crate::ast::decl::CoverBinKind::Illegal => {
                    if live && self.cg_select(&b.select, cr, axes, prod, ctor) {
                        return CrossOwner::Excluded(bi);
                    }
                }
                crate::ast::decl::CoverBinKind::Bins => {
                    if live && self.cg_select(&b.select, cr, axes, prod, ctor) {
                        user.push(j);
                    }
                    j += 1;
                }
                crate::ast::decl::CoverBinKind::Default => {}
            }
        }
        if user.is_empty() {
            CrossOwner::Auto
        } else {
            CrossOwner::User(user)
        }
    }

    /// §19.6.1 select_expression over one product of the cross's bins.
    pub(super) fn cg_select(
        &self,
        sel: &crate::ast::decl::CrossSelect,
        cr: &crate::ast::decl::Cross,
        axes: &[CpShape],
        prod: &[usize],
        ctor: &[(String, Value)],
    ) -> bool {
        use crate::ast::decl::CrossSelect;
        match sel {
            CrossSelect::Binsof { cp, bin, intersect } => {
                let Some(k) = cr.items.iter().position(|i| i.name == cp.name) else {
                    return false;
                };
                let b = &axes[k].bins[prod[k]];
                if bin.as_ref().is_some_and(|bn| b.name != bn.name) {
                    return false;
                }
                match intersect {
                    None => true,
                    Some(rs) => {
                        let want: Vec<(i64, i64)> = rs
                            .iter()
                            .filter_map(|r| self.cg_range_bounds(r, ctor))
                            .collect();
                        b.ranges
                            .iter()
                            .any(|&(l, h)| want.iter().any(|&(a, c)| l <= c && a <= h))
                    }
                }
            }
            CrossSelect::Not(a) => !self.cg_select(a, cr, axes, prod, ctor),
            CrossSelect::And(a, b) => {
                self.cg_select(a, cr, axes, prod, ctor) && self.cg_select(b, cr, axes, prod, ctor)
            }
            CrossSelect::Or(a, b) => {
                self.cg_select(a, cr, axes, prod, ctor) || self.cg_select(b, cr, axes, prod, ctor)
            }
            CrossSelect::With(a, e) => {
                self.cg_select(a, cr, axes, prod, ctor)
                    && self.cg_with_holds(e, cr, axes, prod, ctor)
            }
            CrossSelect::All => true,
        }
    }

    /// `select with (expr)`: true when `expr`, with each crossed item's name
    /// bound to a value of its bin in `prod`, holds for some value tuple.
    pub(super) fn cg_with_holds(
        &self,
        e: &Expression,
        cr: &crate::ast::decl::Cross,
        axes: &[CpShape],
        prod: &[usize],
        ctor: &[(String, Value)],
    ) -> bool {
        const MAX_VALUES: usize = 256;
        const MAX_TUPLES: usize = 1 << 16;
        let sets: Vec<Vec<i64>> = prod
            .iter()
            .enumerate()
            .map(|(k, &b)| {
                let mut vs = Vec::new();
                'fill: for &(l, h) in &axes[k].bins[b].ranges {
                    let mut v = l;
                    while v <= h {
                        if vs.len() >= MAX_VALUES {
                            break 'fill;
                        }
                        if !Self::cg_in_ranges(v, &axes[k].excl) {
                            vs.push(v);
                        }
                        v += 1;
                    }
                }
                vs
            })
            .collect();
        if sets.iter().any(|s| s.is_empty()) {
            return false;
        }
        let mut params: HashMap<String, Value> = ctor.iter().cloned().collect();
        let mut idx = vec![0usize; sets.len()];
        for _ in 0..MAX_TUPLES {
            for (k, it) in cr.items.iter().enumerate() {
                params.insert(it.name.clone(), Value::from_u64(sets[k][idx[k]] as u64, 64));
            }
            if super::elaborate::const_eval_i64_with_params(e, Some(&params))
                .is_some_and(|v| v != 0)
            {
                return true;
            }
            let mut k = idx.len();
            loop {
                if k == 0 {
                    return false;
                }
                k -= 1;
                idx[k] += 1;
                if idx[k] < sets[k].len() {
                    break;
                }
                idx[k] = 0;
            }
        }
        false
    }

    /// The transition sequences of a bin (§19.5.2), each with its label: a
    /// scalar bin keeps its sequences as they are written (label None); an
    /// array bin `name[]` gets one bin per value sequence its sets and
    /// ranges expand to, labelled like `1=>2`.
    pub(super) fn cg_trans_sequences(
        &self,
        bin: &crate::ast::decl::CoverBin,
        ctor: &[(String, Value)],
    ) -> Vec<(Option<String>, Vec<TransAtom>)> {
        use crate::ast::decl::TransRepeatKind;
        const MAX_SEQS: usize = 4096;
        let mut out = Vec::new();
        for set in &bin.transitions {
            let mut atoms: Vec<TransAtom> = Vec::new();
            for step in set {
                let ranges: Vec<(i64, i64)> = step
                    .values
                    .iter()
                    .filter_map(|r| self.cg_range_bounds(r, ctor))
                    .collect();
                let (kind, min, max) = match &step.repeat {
                    None => (TransRepeatKind::Consecutive, 1, Some(1)),
                    Some(rep) => {
                        let lo = self.cg_const_i64(&rep.lo, ctor).unwrap_or(1).max(0) as u32;
                        let hi = match &rep.hi {
                            None => Some(lo),
                            Some(h) if matches!(h.kind, ExprKind::Dollar) => None,
                            Some(h) => {
                                Some(self.cg_const_i64(h, ctor).unwrap_or(lo as i64).max(0) as u32)
                            }
                        };
                        (rep.kind, lo, hi)
                    }
                };
                atoms.push(TransAtom {
                    ranges,
                    kind,
                    min,
                    max,
                });
            }
            if !bin.array_form {
                out.push((None, atoms));
                continue;
            }
            // Expand every step's values into single-value steps.
            let mut seqs: Vec<(Vec<String>, Vec<TransAtom>)> = vec![(Vec::new(), Vec::new())];
            for a in &atoms {
                let mut vals: Vec<i64> = Vec::new();
                for &(l, h) in &a.ranges {
                    let mut v = l;
                    while v <= h && vals.len() < 64 {
                        vals.push(v);
                        v += 1;
                    }
                }
                let rep = match (a.kind, a.min, a.max) {
                    (TransRepeatKind::Consecutive, 1, Some(1)) => String::new(),
                    (k, lo, hi) => {
                        let op = match k {
                            TransRepeatKind::Consecutive => "*",
                            TransRepeatKind::Goto => "->",
                            TransRepeatKind::NonConsecutive => "=",
                        };
                        match hi {
                            Some(h) if h == lo => format!("[{}{}]", op, lo),
                            Some(h) => format!("[{}{}:{}]", op, lo, h),
                            None => format!("[{}{}:$]", op, lo),
                        }
                    }
                };
                let mut next = Vec::new();
                for (label, seq) in &seqs {
                    for &v in &vals {
                        if next.len() >= MAX_SEQS {
                            break;
                        }
                        let mut l2 = label.clone();
                        l2.push(format!("{}{}", v, rep));
                        let mut s2 = seq.clone();
                        s2.push(TransAtom {
                            ranges: vec![(v, v)],
                            ..a.clone()
                        });
                        next.push((l2, s2));
                    }
                }
                seqs = next;
            }
            out.extend(seqs.into_iter().map(|(l, s)| (Some(l.join("=>")), s)));
        }
        out
    }

    /// Start of atom `i` of a transition (with any zero-count atoms after
    /// it); true when that completes the sequence.
    pub(super) fn cg_trans_enter(
        atoms: &[TransAtom],
        i: usize,
        next: &mut HashSet<(u32, u32)>,
    ) -> bool {
        if i >= atoms.len() {
            return true;
        }
        next.insert((i as u32, 0));
        atoms[i].min == 0 && Self::cg_trans_enter(atoms, i + 1, next)
    }

    /// Advance the in-flight attempts `pos` of one transition sequence by a
    /// sample (None: x or z, which matches no value); every sample also
    /// starts a new attempt. True when an attempt completes at this sample.
    pub(super) fn cg_trans_step(
        atoms: &[TransAtom],
        pos: &mut HashSet<(u32, u32)>,
        v: Option<i64>,
    ) -> bool {
        use crate::ast::decl::TransRepeatKind;
        if atoms.is_empty() {
            return false;
        }
        let mut cur = std::mem::take(pos);
        let _ = Self::cg_trans_enter(atoms, 0, &mut cur);
        let mut matched = false;
        for (i, c) in cur {
            let a = &atoms[i as usize];
            let inset = v.is_some_and(|v| Self::cg_in_ranges(v, &a.ranges));
            // Above an unbounded minimum every count behaves alike.
            let cap = |n: u32| if a.max.is_none() { n.min(a.min) } else { n };
            let below_max = |n: u32| a.max.is_none_or(|m| n < m);
            let in_range = |n: u32| n >= a.min && a.max.is_none_or(|m| n <= m);
            let complete = match a.kind {
                TransRepeatKind::Consecutive => {
                    if !inset {
                        continue;
                    }
                    if below_max(c + 1) {
                        pos.insert((i, cap(c + 1)));
                    }
                    in_range(c + 1)
                }
                TransRepeatKind::Goto => {
                    if !inset {
                        pos.insert((i, c));
                        continue;
                    }
                    if below_max(c + 1) {
                        pos.insert((i, cap(c + 1)));
                    }
                    in_range(c + 1)
                }
                TransRepeatKind::NonConsecutive => {
                    let c1 = if inset { c + 1 } else { c };
                    if a.max.is_some_and(|m| c1 > m) {
                        continue;
                    }
                    pos.insert((i, cap(c1)));
                    in_range(c1) && (c1 > 0 || !inset)
                }
            };
            if complete && Self::cg_trans_enter(atoms, i as usize + 1, pos) {
                matched = true;
            }
        }
        matched
    }

    /// §19.5.1 `name[N]`: the bin's values, in the order written, spread
    /// over N bins, each taking `len / N` of them and the last one also the
    /// remainder (one value per bin when there are fewer values than bins).
    /// Returns (N, values per bin, the value list as ordered ranges).
    pub(super) fn cg_fixed_array(
        &self,
        bin: &crate::ast::decl::CoverBin,
        ctor: &[(String, Value)],
    ) -> Option<(u64, u64, Vec<(i64, i64)>)> {
        let n = self.cg_const_i64(bin.array_size.as_ref()?, ctor)?.max(0) as u64;
        let segs = self.cg_bin_value_ranges(bin, ctor);
        let m: u64 = segs.iter().map(|&(l, h)| (h - l) as u64 + 1).sum();
        Some((n, m / n.max(1), segs))
    }

    /// The sub-bins of a fixed-size array bin that value `v` falls in.
    pub(super) fn cg_fixed_bins_of(
        v: i64,
        (n, per, segs): &(u64, u64, Vec<(i64, i64)>),
    ) -> Vec<u64> {
        let mut out = Vec::new();
        let mut off = 0u64;
        for &(l, h) in segs {
            if v >= l && v <= h {
                let p = off + (v - l) as u64;
                let j = if *per == 0 { p } else { (p / per).min(n - 1) };
                if j < *n && !out.contains(&j) {
                    out.push(j);
                }
            }
            off += (h - l) as u64 + 1;
        }
        out
    }

    /// The value ranges of sub-bin `j` of a fixed-size array bin.
    pub(super) fn cg_fixed_bin_ranges(
        j: u64,
        (n, per, segs): &(u64, u64, Vec<(i64, i64)>),
    ) -> Vec<(i64, i64)> {
        let m: u64 = segs.iter().map(|&(l, h)| (h - l) as u64 + 1).sum();
        let (start, end) = if *per == 0 {
            (j, j + 1)
        } else if j + 1 == *n {
            (j * per, m)
        } else {
            (j * per, (j + 1) * per)
        };
        let end = end.min(m);
        let mut out = Vec::new();
        let mut off = 0u64;
        for &(l, h) in segs {
            let len = (h - l) as u64 + 1;
            let (a, b) = (start.max(off), end.min(off + len));
            if a < b {
                out.push((l + (a - off) as i64, l + (b - off) as i64 - 1));
            }
            off += len;
        }
        out
    }

    /// The `bin_hits` keys a matching value adds for bin `bin`: the bin
    /// itself, or its array element (`name[value]` for `name[]`, the
    /// sub-bin index for `name[N]`).
    /// §19.5.1.1: whether bin `bin`'s `with (expr)` filter admits value `v`
    /// (`item` bound to it). The filter is a constant expression of `item`.
    pub(super) fn cg_with_admits(&self, bin: &crate::ast::decl::CoverBin, v: i64) -> bool {
        let Some(f) = &bin.with_filter else {
            return true;
        };
        let mut e = f.clone();
        let lit = Expression::new(
            ExprKind::Number(crate::ast::expr::NumberLiteral::Integer {
                size: None,
                signed: true,
                base: crate::ast::expr::NumberBase::Decimal,
                value: v.to_string(),
                cached_val: std::cell::Cell::new(None),
            }),
            f.span,
        );
        Self::subst_ident_expr(&mut e, "item", &lit);
        self.cg_const_bool(&e).unwrap_or(true)
    }

    /// The constant value ranges of bin `bin`, in the order written, keeping
    /// only the values its `with` filter admits (§19.5.1.1): a filtered
    /// bin's values, and an array bin's distribution, are those values. At
    /// most 1 << 20 values are tried, so a wide range with a selective filter
    /// stays bounded.
    pub(super) fn cg_bin_value_ranges(
        &self,
        bin: &crate::ast::decl::CoverBin,
        ctor: &[(String, Value)],
    ) -> Vec<(i64, i64)> {
        let segs = bin
            .values
            .iter()
            .filter_map(|r| self.cg_range_bounds(r, ctor));
        if bin.with_filter.is_none() {
            return segs.collect();
        }
        let mut out: Vec<(i64, i64)> = Vec::new();
        let mut tried = 0u64;
        'segs: for (l, h) in segs {
            let mut run: Option<(i64, i64)> = None;
            for v in l..=h {
                if tried >= 1 << 20 {
                    out.extend(run);
                    break 'segs;
                }
                tried += 1;
                if self.cg_with_admits(bin, v) {
                    run = Some(run.map_or((v, v), |(a, _)| (a, v)));
                } else if let Some(r) = run.take() {
                    out.push(r);
                }
            }
            out.extend(run);
        }
        out
    }

    /// A constant Boolean: `inside` (values and `[lo:hi]` ranges), `&&`,
    /// `||`, `!` and parentheses here, anything else through the constant
    /// evaluator. None when it is not constant.
    fn cg_const_bool(&self, e: &Expression) -> Option<bool> {
        use crate::ast::expr::{BinaryOp, UnaryOp};
        let num = |x: &Expression| {
            super::elaborate::const_eval_i64_with_params(x, Some(&self.module.parameters))
        };
        match &e.kind {
            ExprKind::Paren(x) => self.cg_const_bool(x),
            ExprKind::Unary {
                op: UnaryOp::LogNot,
                operand,
            } => self.cg_const_bool(operand).map(|b| !b),
            ExprKind::Binary {
                op: BinaryOp::LogAnd,
                left,
                right,
            } => Some(self.cg_const_bool(left)? && self.cg_const_bool(right)?),
            ExprKind::Binary {
                op: BinaryOp::LogOr,
                left,
                right,
            } => Some(self.cg_const_bool(left)? || self.cg_const_bool(right)?),
            ExprKind::Inside { expr, ranges } => {
                let v = num(expr)?;
                for r in ranges {
                    let hit = match &r.kind {
                        ExprKind::Range(lo, hi) => (num(lo)?..=num(hi)?).contains(&v),
                        _ => num(r)? == v,
                    };
                    if hit {
                        return Some(true);
                    }
                }
                Some(false)
            }
            _ => num(e).map(|r| r != 0),
        }
    }

    pub(super) fn cg_value_bin_keys(
        &self,
        bin: &crate::ast::decl::CoverBin,
        base_key: &str,
        vi: Option<i64>,
        ctor: &[(String, Value)],
    ) -> Vec<String> {
        if !bin.array_form {
            return vec![base_key.to_string()];
        }
        let Some(v) = vi else {
            return Vec::new();
        };
        if bin.array_size.is_none() {
            return vec![format!("{}[{}]", base_key, v)];
        }
        match self.cg_fixed_array(bin, ctor) {
            Some(fa) => Self::cg_fixed_bins_of(v, &fa)
                .into_iter()
                .map(|j| format!("{}[{}]", base_key, j))
                .collect(),
            None => Vec::new(),
        }
    }

    /// Advance the transition bins of coverpoint `cp` by the sample `vi`;
    /// returns the keys (and kinds) of the transition bins it completes.
    pub(super) fn cg_step_transitions(
        &mut self,
        handle: usize,
        cp: &crate::ast::decl::Coverpoint,
        cp_name: &str,
        vi: Option<i64>,
        ctor: &[(String, Value)],
    ) -> Vec<(String, crate::ast::decl::CoverBinKind)> {
        let mut out = Vec::new();
        for bin in cp.bins.iter().filter(|b| !b.transitions.is_empty()) {
            let base = format!("{}.{}", cp_name, bin.name.name);
            let seqs = self.cg_trans_sequences(bin, ctor);
            let Some(Some(inst)) = self.cg_heap.get_mut(handle) else {
                break;
            };
            let mut hit = false;
            for (si, (label, atoms)) in seqs.iter().enumerate() {
                let state = match label {
                    Some(l) => format!("{}[{}]", base, l),
                    None => format!("{}#{}", base, si),
                };
                let pos = inst.trans_pos.entry(state.clone()).or_default();
                if Self::cg_trans_step(atoms, pos, vi) {
                    match label {
                        Some(_) => out.push((state, bin.kind)),
                        None => hit = true,
                    }
                }
            }
            if hit {
                out.push((base, bin.kind));
            }
        }
        out
    }

    /// The product bins (one bin index per axis) a sampled value tuple hits.
    pub(super) fn cg_tuple_products(axes: &[CpShape], t: &[Value]) -> Vec<Vec<usize>> {
        if t.len() != axes.len() || t.iter().any(|v| v.has_xz()) {
            return Vec::new();
        }
        let mut combos: Vec<Vec<usize>> = vec![Vec::new()];
        for (k, v) in t.iter().enumerate() {
            let ids = Self::cg_axis_bins_of(&axes[k], v.to_u64().unwrap_or(0) as i64);
            let mut next = Vec::with_capacity(combos.len() * ids.len());
            for c in &combos {
                for &id in &ids {
                    let mut c2 = c.clone();
                    c2.push(id);
                    next.push(c2);
                }
            }
            combos = next;
        }
        combos
    }

    /// Sample on the covergroup's event, whose `iff` term is `guard`.
    pub(super) fn sample_covergroup_guarded(&mut self, handle: usize, guard: Option<&Expression>) {
        self.sample_covergroup_with(handle, HashMap::default(), guard);
    }

    /// Sample every covergroup waiting on a fired event.
    pub(super) fn sample_event_covergroups(&mut self, waiters: Vec<(usize, Option<Expression>)>) {
        for (handle, iff) in waiters {
            self.sample_covergroup_guarded(handle, iff.as_ref());
        }
    }

    /// Sample instance `handle` with the §19.8.1 `with function sample`
    /// formals bound to `sample_args`. They share one frame with the §19.3
    /// constructor formals: a second frame on top hid the first, so a
    /// covergroup with both never saw its sample arguments.
    pub(super) fn sample_covergroup_with(
        &mut self,
        handle: usize,
        sample_args: HashMap<String, Value>,
        guard: Option<&Expression>,
    ) {
        let (owner, mut ctor_args, ctor_refs) =
            match self.cg_heap.get(handle).and_then(|x| x.as_ref()) {
                Some(i) if !i.stopped => (i.owner, i.ctor_args.clone(), i.ctor_refs.clone()),
                _ => return,
            };
        // §19.3: a `ref` formal reads its actual's CURRENT value.
        for (n, e) in &ctor_refs {
            let v = self.eval_expr(e);
            if let Some(slot) = ctor_args.iter_mut().find(|(k, _)| k == n) {
                slot.1 = v;
            }
        }
        let mut frame = sample_args;
        for (n, v) in ctor_args {
            frame.entry(n).or_insert(v);
        }
        let has_args = !frame.is_empty();
        if has_args {
            self.push_local_frame(frame);
        }
        if let Some(h) = owner {
            self.this_stack.push(Some(h));
        }
        // The guard reads the instance's scope (its object and formals).
        if guard.is_none_or(|g| self.eval_expr(g).is_true()) {
            self.sample_covergroup_inner(handle);
        }
        if owner.is_some() {
            self.this_stack.pop();
        }
        if has_args {
            self.pop_local_frame();
        }
    }

    pub(super) fn sample_covergroup_inner(&mut self, handle: usize) {
        let cg_name = if let Some(Some(inst)) = self.cg_heap.get(handle) {
            inst.cg_name.clone()
        } else {
            return;
        };
        // Per-instance sample tally — feeds the end-of-sim coverage DB.
        if let Some(Some(inst)) = self.cg_heap.get_mut(handle) {
            inst.sample_count += 1;
        }

        let def = if let Some(d) = self.module.covergroups.get(&cg_name).cloned() {
            d
        } else {
            return;
        };

        let stopped_items = self
            .cg_heap
            .get(handle)
            .and_then(|x| x.as_ref())
            .map(|i| i.stopped_items.clone())
            .unwrap_or_default();
        for item in &def.items {
            if !stopped_items.is_empty() {
                let item_name = match item {
                    CovergroupItem::Coverpoint(cp) => Some(Self::cg_point_name(cp)),
                    CovergroupItem::Cross(cr) => Some(Self::cg_cross_name(cr)),
                    _ => None,
                };
                if item_name.is_some_and(|n| stopped_items.contains(&n)) {
                    continue;
                }
            }
            match item {
                CovergroupItem::Coverpoint(cp) => {
                    // LRM §19.5 `iff (guard)` — skip the sample when the
                    // guard expression is false (or X/Z): no hit, no bin
                    // and no transition step.
                    if let Some(guard) = &cp.iff_guard {
                        if !self.eval_expr(guard).is_true() {
                            continue;
                        }
                    }
                    let val = self.eval_expr(&cp.expr);
                    let cp_name = Self::cg_point_name(cp);
                    let vi: Option<i64> = if val.has_xz() { None } else { val.to_i64() };
                    let ctor: Vec<(String, Value)> = self
                        .cg_heap
                        .get(handle)
                        .and_then(|x| x.as_ref())
                        .map(|i| Self::cg_bin_scope(i).into_owned())
                        .unwrap_or_default();
                    // §19.5.6/§19.5.7: an ignored or illegal value is in no
                    // other bin.
                    let has_excl = cp.bins.iter().any(|b| {
                        matches!(
                            b.kind,
                            crate::ast::decl::CoverBinKind::Ignore
                                | crate::ast::decl::CoverBinKind::Illegal
                        )
                    });
                    let excluded = has_excl
                        && vi.is_some_and(|v| {
                            Self::cg_in_ranges(v, &self.cg_excluded_ranges(cp, &ctor))
                        });
                    // `default` bins count only the values no other value
                    // bin (bins, ignore_bins or illegal_bins) holds.
                    let mut bin_increments: Vec<String> = Vec::new();
                    let mut illegal_hits: Vec<String> = Vec::new();
                    let mut any_value_match = false;
                    let mut default_bin_keys: Vec<String> = Vec::new();
                    for bin in &cp.bins {
                        let base_key = format!("{}.{}", cp_name, bin.name.name);
                        if bin.kind == crate::ast::decl::CoverBinKind::Default {
                            default_bin_keys.push(base_key);
                            continue;
                        }
                        if !bin.transitions.is_empty() {
                            continue;
                        }
                        let matched = if bin.is_wildcard {
                            // LRM §19.5 wildcard match: bit-wise compare with
                            // the bin pattern; pattern bits that are X/Z/?
                            // are don't-cares.
                            bin.values.iter().any(|r| match r {
                                ConstraintRange::Value(e) => {
                                    let pat = self.eval_expr(e);
                                    val.wildcard_eq(&pat).is_true()
                                }
                                // Wildcard with [lo:hi] isn't standard;
                                // treat as plain range.
                                ConstraintRange::Range { lo, hi } => {
                                    let l = self.eval_expr(lo);
                                    let h = self.eval_expr(hi);
                                    val.greater_equal(&l).is_true() && val.less_equal(&h).is_true()
                                }
                            })
                        } else {
                            self.value_in_ranges(&val, &bin.values)
                        };
                        // §19.5.1.1: a value the bin's `with` filter rejects
                        // is not in the bin at all (a `default` bin may take it).
                        if !matched || vi.is_some_and(|v| !self.cg_with_admits(bin, v)) {
                            continue;
                        }
                        any_value_match = true;
                        // §19.5: a bin's own `iff` guard keeps it from
                        // counting while false (#605); the value still
                        // belongs to it, so no `default` bin takes it.
                        if let Some(g) = &bin.iff_guard
                            && !self.eval_expr(g).is_true()
                        {
                            continue;
                        }
                        match bin.kind {
                            crate::ast::decl::CoverBinKind::Bins if !excluded => {
                                bin_increments
                                    .extend(self.cg_value_bin_keys(bin, &base_key, vi, &ctor));
                            }
                            crate::ast::decl::CoverBinKind::Illegal => illegal_hits.push(base_key),
                            _ => {}
                        }
                    }
                    if !any_value_match {
                        bin_increments.extend(default_bin_keys);
                    }
                    for (key, kind) in self.cg_step_transitions(handle, cp, &cp_name, vi, &ctor) {
                        match kind {
                            crate::ast::decl::CoverBinKind::Bins => bin_increments.push(key),
                            crate::ast::decl::CoverBinKind::Illegal => illegal_hits.push(key),
                            _ => {}
                        }
                    }
                    for key in &illegal_hits {
                        self.cg_illegal_hit(&cg_name, key, &Self::cg_value_text(&val));
                    }
                    if let Some(Some(inst)) = self.cg_heap.get_mut(handle) {
                        *inst
                            .point_hits
                            .entry(cp_name)
                            .or_default()
                            .entry(val)
                            .or_insert(0) += 1;
                        for k in bin_increments.into_iter().chain(illegal_hits) {
                            *inst.bin_hits.entry(k).or_insert(0) += 1;
                        }
                    }
                }
                CovergroupItem::Cross(cr) => {
                    // LRM §19.6 `iff (guard)` — skip the cross sample
                    // when the guard expression is false.
                    if let Some(guard) = &cr.iff_guard {
                        if !self.eval_expr(guard).is_true() {
                            continue;
                        }
                    }
                    // Cross items name COVERPOINTS, not signals — resolve each
                    // to its coverpoint's expression in this covergroup and
                    // eval, so the tuple captures the actual sampled values.
                    let mut tuple = Vec::new();
                    let mut implicit_points: Vec<(String, Value)> = Vec::new();
                    for id in &cr.items {
                        let mut val = None;
                        for it in &def.items {
                            if let CovergroupItem::Coverpoint(cp) = it {
                                let cp_name = cp.name.as_ref().map(|n| n.name.as_str());
                                if cp_name == Some(id.name.as_str()) {
                                    val = Some(self.eval_expr(&cp.expr));
                                    break;
                                }
                            }
                        }
                        // §19.6: a crossed VARIABLE with no coverpoint of its
                        // own gets an implicit coverpoint over it. It used to
                        // be recorded as a constant 0. A dotted item is a
                        // hierarchical reference (XEZIM_RELAX_CROSS_HIER_ITEM).
                        let val = match val {
                            Some(v) => v,
                            None => {
                                let e = Expression::new(
                                    ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                                        root: None,
                                        // An escaped name (`\a.b`) keeps its dot
                                        // unless the switch made it a path.
                                        path: (if sv_parser::RELAX_CROSS_HIER_ITEM.get() {
                                            id.name.split('.').collect::<Vec<_>>()
                                        } else {
                                            vec![id.name.as_str()]
                                        })
                                        .into_iter()
                                        .map(|n| crate::ast::expr::HierPathSegment {
                                            name: crate::ast::Identifier {
                                                name: n.to_string(),
                                                span: id.span,
                                            },
                                            selects: Vec::new(),
                                        })
                                        .collect(),
                                        span: id.span,
                                        cached_signal_id: std::cell::Cell::new(None),
                                        cached_resolved_name: std::cell::OnceCell::new(),
                                    }),
                                    id.span,
                                );
                                let v = self.eval_expr(&e);
                                implicit_points.push((id.name.clone(), v.clone()));
                                v
                            }
                        };
                        tuple.push(val);
                    }
                    let cr_name = Self::cg_cross_name(cr);
                    // §19.6.1: the user bins the sampled tuple falls in (a
                    // bin whose `iff` is false takes nothing); a tuple in an
                    // illegal_bins is an error.
                    let mut bin_increments: Vec<String> = Vec::new();
                    let mut illegal: Vec<String> = Vec::new();
                    if !cr.bins.is_empty() {
                        let guards: Vec<bool> = cr
                            .bins
                            .iter()
                            .map(|b| {
                                b.iff_guard
                                    .as_ref()
                                    .is_none_or(|g| self.eval_expr(g).is_true())
                            })
                            .collect();
                        if let Some(Some(inst)) = self.cg_heap.get(handle) {
                            let insts = [inst];
                            let scope = Self::cg_bin_scope(inst);
                            let ctor: &[(String, Value)] = &scope;
                            let axes: Vec<CpShape> = cr
                                .items
                                .iter()
                                .zip(&tuple)
                                .map(|(it, v)| {
                                    self.cg_cross_axis(&def, &it.name, &insts, Some(v.width))
                                })
                                .collect();
                            let user: Vec<&crate::ast::decl::CrossBin> = cr
                                .bins
                                .iter()
                                .filter(|b| matches!(b.kind, crate::ast::decl::CoverBinKind::Bins))
                                .collect();
                            let mut hit = vec![false; user.len()];
                            for prod in Self::cg_tuple_products(&axes, &tuple) {
                                match self.cg_cross_owner(cr, &axes, &prod, ctor, Some(&guards)) {
                                    CrossOwner::User(js) => {
                                        for j in js {
                                            hit[j] = true;
                                        }
                                    }
                                    CrossOwner::Excluded(bi)
                                        if cr.bins[bi].kind
                                            == crate::ast::decl::CoverBinKind::Illegal =>
                                    {
                                        let key = format!("{}.{}", cr_name, cr.bins[bi].name.name);
                                        if !illegal.contains(&key) {
                                            illegal.push(key);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            for (j, b) in user.iter().enumerate() {
                                if hit[j] {
                                    bin_increments.push(format!("{}.{}", cr_name, b.name.name));
                                }
                            }
                        }
                    }
                    if !illegal.is_empty() {
                        let text = format!(
                            "({})",
                            tuple
                                .iter()
                                .map(Self::cg_value_text)
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                        for key in &illegal {
                            let body = format!(
                                "Illegal cross bin hit at values {}: {}.{}",
                                text, cg_name, key
                            );
                            self.emit_severity_text("Error", &body);
                        }
                    }
                    bin_increments.extend(illegal);
                    if let Some(Some(inst)) = self.cg_heap.get_mut(handle) {
                        *inst
                            .cross_hits
                            .entry(cr_name.clone())
                            .or_default()
                            .entry(tuple)
                            .or_insert(0) += 1;
                        for k in bin_increments {
                            *inst.cross_bin_hits.entry(k).or_insert(0) += 1;
                        }
                        for (name, v) in implicit_points {
                            *inst
                                .point_hits
                                .entry(name)
                                .or_default()
                                .entry(v)
                                .or_insert(0) += 1;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// §19.5.7: sampling a value (or transition) of an `illegal_bins` is a
    /// run-time error; it counts toward `--error-exit`, and the run goes on.
    pub(super) fn cg_illegal_hit(&mut self, cg_name: &str, bin_key: &str, value: &str) {
        let body = format!(
            "Illegal bin hit at value {}: {}.{}",
            value, cg_name, bin_key
        );
        self.emit_severity_text("Error", &body);
    }

    /// A sampled value as an illegal-bin message shows it: decimal, or
    /// binary when it has x or z bits.
    pub(super) fn cg_value_text(val: &Value) -> String {
        if val.has_xz() {
            format!("{}'b{}", val.width, val.to_bin_string())
        } else if val.is_signed {
            val.to_i64().unwrap_or(0).to_string()
        } else {
            val.to_u64().unwrap_or(0).to_string()
        }
    }
}
