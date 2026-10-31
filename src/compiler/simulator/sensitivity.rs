//! Simulator sensitivity.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

thread_local! {
    /// While set (`collect_stmt_reads_whole`): the names read as a whole value,
    /// not only as the base of a member read.
    static WHOLE_READS: RefCell<Option<HashSet<String>>> = const { RefCell::new(None) };
    /// While a function body's reads are collected: the reads of the
    /// functions it calls, kept apart from its own (`collect_callee_reads`).
    static CALLEE_READS: RefCell<Option<HashSet<String>>> = const { RefCell::new(None) };
}

/// Record `name` as read whole, when `collect_stmt_reads_whole` is collecting.
fn note_whole_read(name: &str) {
    WHOLE_READS.with(|c| {
        if let Some(set) = c.borrow_mut().as_mut() {
            set.insert(name.to_string());
        }
    });
}

impl Simulator {
    pub(super) fn collect_leaf_idents(expr: &Expression, out: &mut HashSet<String>) {
        match &expr.kind {
            ExprKind::Ident(hier) if hier.path.len() == 1 => {
                out.insert(hier.path[0].name.name.clone());
            }
            ExprKind::Index { expr, index } => {
                Self::collect_leaf_idents(expr, out);
                Self::collect_leaf_idents(index, out);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                Self::collect_leaf_idents(expr, out);
                Self::collect_leaf_idents(left, out);
                Self::collect_leaf_idents(right, out);
            }
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                Self::collect_leaf_idents(operand, out);
            }
            ExprKind::Binary { left, right, .. } => {
                Self::collect_leaf_idents(left, out);
                Self::collect_leaf_idents(right, out);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::collect_leaf_idents(condition, out);
                Self::collect_leaf_idents(then_expr, out);
                Self::collect_leaf_idents(else_expr, out);
            }
            ExprKind::Concatenation(parts) => {
                for p in parts {
                    Self::collect_leaf_idents(p, out);
                }
            }
            ExprKind::AssignmentPattern(parts) => {
                for p in parts {
                    Self::collect_leaf_idents(p.expr(), out);
                }
            }
            ExprKind::Replication { count, exprs } => {
                Self::collect_leaf_idents(count, out);
                for e in exprs {
                    Self::collect_leaf_idents(e, out);
                }
            }
            ExprKind::Call { func, args } => {
                Self::collect_leaf_idents(func, out);
                for a in args {
                    Self::collect_leaf_idents(a, out);
                }
            }
            ExprKind::SystemCall { args, .. } => {
                for a in args {
                    Self::collect_leaf_idents(a, out);
                }
            }
            ExprKind::MemberAccess { expr, .. } => {
                Self::collect_leaf_idents(expr, out);
            }
            _ => {}
        }
    }

    pub(super) fn infer_contassign_scope_hint(
        &self,
        lhs: &Expression,
        rhs: &Expression,
    ) -> Option<String> {
        let ident_raw = |expr: &Expression| -> Option<String> {
            if let ExprKind::Ident(hier) = &expr.kind {
                Some(
                    hier.path
                        .iter()
                        .map(|s| s.name.name.as_str())
                        .collect::<Vec<_>>()
                        .join("."),
                )
            } else {
                None
            }
        };
        let ident_leaf = |expr: &Expression| -> Option<String> {
            if let ExprKind::Ident(hier) = &expr.kind {
                if hier.path.len() == 1 {
                    let name = hier.path[0].name.name.clone();
                    if !name.contains('.') {
                        return Some(name);
                    }
                }
            }
            None
        };
        let parent_n = |name: &str, n: usize| -> Option<String> {
            let mut cur = name.rsplit_once('.').map(|(p, _)| p.to_string())?;
            for _ in 0..n {
                cur = cur.rsplit_once('.').map(|(p, _)| p.to_string())?;
            }
            Some(cur)
        };

        let lhs_raw = ident_raw(lhs);
        let rhs_raw = ident_raw(rhs);
        let lhs_leaf_opt = ident_leaf(lhs);
        let rhs_leaf_opt = ident_leaf(rhs);

        // Common port-connection form after inlining:
        //   child.input  = parent_signal;
        //   parent_signal = child.output;
        // Use parent instance scope to resolve the unqualified side.
        if lhs_leaf_opt.is_none() && rhs_leaf_opt.is_some() {
            if let Some(raw) = lhs_raw.as_deref() {
                if let Some(scope) = parent_n(raw, 1) {
                    return Some(scope);
                }
            }
        }
        if lhs_leaf_opt.is_some() && rhs_leaf_opt.is_none() {
            if let Some(raw) = rhs_raw.as_deref() {
                if let Some(scope) = parent_n(raw, 1) {
                    return Some(scope);
                }
            }
        }

        // Some elaborated continuous assignments retain an absolute, dotted
        // LHS as one identifier segment while the RHS remains a relative
        // multi-segment path. Neither side is a "leaf" under the checks
        // above, so scope inference used to return None and bytecode lookup
        // could not distinguish identical RHS paths in replicated instances.
        // An exact LHS signal anchors the assignment in its direct parent.
        if let Some(raw) = lhs_raw.as_deref() {
            if self.signal_name_to_id.contains_key(raw) {
                if let Some((parent, _)) = raw.rsplit_once('.') {
                    return Some(parent.to_string());
                }
            }
        }

        let lhs_leaf = lhs_leaf_opt?;
        // A bare LHS that IS a registered signal resolves absolutely — no
        // scope hint. Without this, a top-level multi-driven net's folded
        // `b = $__wres(u1.b, u2.b)` matched the `.b`-suffix scan below and
        // got scoped into one INSTANCE (writing u2.b instead of b — the
        // net read z forever).
        if self.signal_name_to_id.contains_key(lhs_leaf.as_str()) {
            return None;
        }
        let mut leaves = HashSet::default();
        Self::collect_leaf_idents(lhs, &mut leaves);
        Self::collect_leaf_idents(rhs, &mut leaves);
        if leaves.is_empty() {
            return None;
        }
        self.best_scope_from_leaf_index(lhs_leaf.as_str(), &leaves)
    }

    pub(super) fn best_scope_from_leaf_index(
        &self,
        anchor: &str,
        leaves: &HashSet<String>,
    ) -> Option<String> {
        let parents = self.scope_parents_by_leaf.get(anchor)?;
        let mut best_parent: Option<&str> = None;
        let mut best_score = 0usize;
        let mut best_depth = 0usize;
        for parent in parents {
            let mut score = 0usize;
            for leaf in leaves {
                if self
                    .scope_parents_by_leaf
                    .get(leaf.as_str())
                    .is_some_and(|leaf_parents| {
                        leaf_parents.iter().any(|candidate| candidate == parent)
                    })
                {
                    score += 1;
                }
            }
            let depth = parent.split('.').count();
            if score > best_score
                || (score == best_score && depth > best_depth)
                // Final tie-break must be a TOTAL order (lexicographic), not
                // length: equal-length candidates fell through to iteration
                // order, which is hash-dependent and differs across CPUs.
                || (score == best_score
                    && depth == best_depth
                    && best_parent.is_none_or(|best| parent.as_ref() > best))
            {
                best_parent = Some(parent.as_ref());
                best_score = score;
                best_depth = depth;
            }
        }
        (best_score != 0).then(|| best_parent.expect("positive score has parent").to_string())
    }

    pub(super) fn infer_scope_from_rw_sets(
        &self,
        writes: &HashSet<String>,
        reads: &HashSet<String>,
    ) -> Option<String> {
        let mut leaves = HashSet::default();
        for name in writes.iter().chain(reads.iter()) {
            if !name.contains('.') && !name.contains('[') {
                leaves.insert(name.clone());
            }
        }
        // Inference exists to LOCATE a process whose names do not resolve
        // where it sits (an inlined body whose locals live under an instance
        // prefix). A process whose every bare name already resolves at its
        // own level needs NO hint — and because resolution is hint-first, a
        // wrongly inferred deep scope HIJACKS every bare name that also
        // exists inside that scope: a testbench checker reading its own
        // 292-bit `x` got the DUT-internal 128-bit `u_a.u_b.x` instead, and
        // which scope won depended on hash iteration order — ahash keys
        // differ across CPUs, so the same binary read different signals on
        // different machines. Anchor on the SMALLEST unresolved name so the
        // choice is total-ordered, not iteration-ordered.
        let anchor = leaves
            .iter()
            .filter(|n| !self.signal_name_to_id.contains_key(n.as_str()))
            .min()
            .cloned()?;
        self.best_scope_from_leaf_index(anchor.as_str(), &leaves)
    }

    /// Collect all signal names read by an expression.
    /// §9.2.2.2: pull the variable reads out of a called function's body so
    /// they contribute to an always_comb/always_latch inferred sensitivity
    /// list. Function locals/ports resolve to nothing at the signal table and
    /// are harmlessly dropped later; only module-level references (the hidden
    /// dependencies) survive. A thread-local active-set breaks (mutual)
    /// recursion so recursive functions don't loop forever here.
    /// §20.7 array query on a size-varying operand (dynamic array, queue,
    /// string). The result type is `int` — SIGNED — and an empty collection
    /// answers `$right`/`$high` with -1, not a clamped 0.
    pub(super) fn dynamic_query_result(sn: &str, size: u64) -> Value {
        let hi = size as i64 - 1;
        let r: i64 = match sn {
            "$size" => size as i64,
            "$left" | "$low" => 0,
            "$right" | "$high" => hi,
            "$increment" => -1,
            _ => 0,
        };
        let mut v = Value::from_u64((r as u64) & 0xFFFF_FFFF, 32);
        v.is_signed = true;
        v
    }

    /// The reads of the function `name` a call reaches (`collect_function_reads`).
    /// Inside another function's body they go to that function's
    /// `CALLEE_READS` sink, apart from its own reads, so its frame names
    /// (formals, locals) do not filter them (§13.4: the callee's `x` is not
    /// the caller's formal `x`).
    fn collect_callee_reads(name: &str, module: &ElaboratedModule, reads: &mut HashSet<String>) {
        let mut got: HashSet<String> = HashSet::default();
        Self::collect_function_reads(name, module, &mut got);
        let rest = CALLEE_READS.with(|c| match c.borrow_mut().as_mut() {
            Some(sink) => {
                sink.extend(got);
                None
            }
            None => Some(got),
        });
        if let Some(got) = rest {
            reads.extend(got);
        }
    }

    pub(super) fn collect_function_reads(
        name: &str,
        module: &ElaboratedModule,
        reads: &mut HashSet<String>,
    ) {
        thread_local! {
            static ACTIVE: RefCell<HashSet<String>> = RefCell::new(HashSet::default());
        }
        let fdecl = match module.functions.get(name) {
            Some(f) => f,
            None => {
                // An inlined instance registers its functions under
                // "<inst>.<name>" while the call site still says "<name>" —
                // so `always_comb d = y(a)` inside an instance never followed
                // `y` (or its nested callees) and missed the module vars they
                // read (br_gh277b). Union the reads of every scoped match:
                // over-approximating sensitivity is safe.
                let suffix = format!(".{}", name);
                let scoped: Vec<String> = module
                    .functions
                    .keys()
                    .filter(|k| k.ends_with(suffix.as_str()))
                    .cloned()
                    .collect();
                for k in scoped {
                    Self::collect_function_reads(&k, module, reads);
                }
                return;
            }
        };
        let entered = ACTIVE.with(|a| a.borrow_mut().insert(name.to_string()));
        if !entered {
            return;
        }
        let mut scratch_writes: HashSet<String> = HashSet::default();
        let mut body_reads: HashSet<String> = HashSet::default();
        let outer = CALLEE_READS.with(|c| c.replace(Some(HashSet::default())));
        // §13.4.3: a default-argument expression is evaluated at call time, so
        // its reads are part of the caller's sensitivity too.
        for port in &fdecl.ports {
            if let Some(def) = &port.default {
                Self::collect_expr_reads(def, module, &mut body_reads);
            }
        }
        for item in &fdecl.items {
            Self::collect_stmt_reads(item, module, &mut body_reads, &mut scratch_writes);
        }
        let callee_reads = CALLEE_READS.with(|c| c.replace(outer)).unwrap_or_default();
        // §13.4: the formals, the return variable and the block locals are
        // frame state, not module signals the caller can be sensitive to. A
        // formal named like a design net (`uj_imm(logic [31:0] instruction_i)`
        // beside a decoder port `instruction_i`) made the calling assign
        // depend on that unrelated net — and trip the elided-port report.
        let leaf = name.rsplit('.').next().unwrap_or(name);
        let mut frame: HashSet<&str> = fdecl.ports.iter().map(|p| p.name.name.as_str()).collect();
        frame.insert(leaf);
        fn local_names<'a>(items: &'a [Statement], out: &mut HashSet<&'a str>) {
            for st in items {
                match &st.kind {
                    StatementKind::VarDecl { declarators, .. } => {
                        out.extend(declarators.iter().map(|d| d.name.name.as_str()));
                    }
                    StatementKind::SeqBlock { stmts, .. } => local_names(stmts, out),
                    _ => {}
                }
            }
        }
        local_names(&fdecl.items, &mut frame);
        body_reads.retain(|r| !frame.contains(r.as_str()));
        body_reads.extend(callee_reads);
        // A bare read in the body of an INSTANCE's function names that
        // instance's variable (`internal_func_dep` inside
        // `dut.eval_with_hidden_dep` is `dut.internal_func_dep`) — the bare
        // spelling resolves to a nonexistent top-level signal and drops out of
        // the sensitivity set. Qualify it wherever only the scoped one exists.
        let scope = name.rsplit_once('.').map(|(s, _)| s);
        for r in body_reads {
            if let Some(scope) = scope {
                // The body may be unrewritten (bare spellings), and a same-named
                // bare signal can exist at top level too — register BOTH names;
                // a superset sensitivity is always safe, and the sensitivity
                // builder drops whichever spelling resolves to no signal.
                if !r.contains('.') {
                    reads.insert(format!("{}.{}", scope, r));
                }
            }
            reads.insert(r);
        }
        ACTIVE.with(|a| {
            a.borrow_mut().remove(name);
        });
    }

    pub(super) fn constant_array_index(
        expr: &Expression,
        module: &ElaboratedModule,
    ) -> Option<i64> {
        fn is_constant(expr: &Expression, module: &ElaboratedModule) -> bool {
            match &expr.kind {
                ExprKind::Number(_) => true,
                ExprKind::Ident(hier) => hier
                    .path
                    .last()
                    .is_some_and(|seg| module.parameters.contains_key(seg.name.name.as_str())),
                ExprKind::Paren(inner) => is_constant(inner, module),
                ExprKind::Unary { operand, .. } => is_constant(operand, module),
                ExprKind::Binary { left, right, .. } => {
                    is_constant(left, module) && is_constant(right, module)
                }
                ExprKind::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                } => {
                    is_constant(condition, module)
                        && is_constant(then_expr, module)
                        && is_constant(else_expr, module)
                }
                _ => false,
            }
        }

        is_constant(expr, module)
            .then(|| super::elaborate::const_eval_i64_with_params(expr, Some(&module.parameters)))?
    }

    /// Dependency names for a fully-indexed multi-dimensional unpacked-array
    /// read. Constant indices select one element; genuinely dynamic indices
    /// retain the conservative matching-array dependency required for
    /// correctness.
    pub(super) fn multi_dim_elem_names(
        base: &Expression,
        outer_index: &Expression,
        module: &ElaboratedModule,
    ) -> Option<Vec<String>> {
        let mut rev_indices = vec![outer_index];
        let mut cur = base;
        while let ExprKind::Index { expr, index } = &cur.kind {
            rev_indices.push(index);
            cur = expr;
        }
        let ExprKind::Ident(h) = &cur.kind else {
            return None;
        };
        rev_indices.reverse();
        let name = Self::resolve_hier_name_static(h, module);
        let shape: Vec<(i64, i64)> = if let Some(&(a, b, _)) = module.arrays_2d.get(&name) {
            vec![a, b]
        } else {
            module.arrays_nd.get(&name)?.0.clone()
        };
        if rev_indices.len() != shape.len() {
            return None;
        }
        let choices: Vec<Vec<i64>> = shape
            .iter()
            .zip(rev_indices)
            .map(|(&(left, right), index)| {
                let lo = left.min(right);
                let hi = left.max(right);
                match Self::constant_array_index(index, module) {
                    Some(i) if (lo..=hi).contains(&i) => vec![i],
                    Some(_) => Vec::new(),
                    None => (lo..=hi).collect(),
                }
            })
            .collect();
        if choices.iter().any(Vec::is_empty) {
            return Some(Vec::new());
        }
        let mut names = vec![name];
        for dim in choices {
            let mut next = Vec::with_capacity(names.len().saturating_mul(dim.len()));
            for prefix in names {
                for index in &dim {
                    next.push(format!("{}[{}]", prefix, index));
                }
            }
            names = next;
        }
        Some(names)
    }

    /// Is comb read `r` (an element name as `collect_stmt_reads` spells it,
    /// `mem[5]` / `mem[1][2]`) a cell of a memory stored without element
    /// names — the packed arena's, or an unnamed bulk array's? Tried bare,
    /// then under the block's scopes.
    pub(super) fn read_names_unnamed_cell(
        &self,
        r: &str,
        scope_hint: Option<&str>,
        scope: &str,
    ) -> bool {
        if !r.ends_with(']') {
            return false;
        }
        let mut cands: Vec<std::borrow::Cow<'_, str>> = vec![std::borrow::Cow::Borrowed(r)];
        if let Some(sc) = scope_hint {
            cands.push(format!("{}.{}", sc, r).into());
        }
        if !scope.is_empty() && Some(scope) != scope_hint {
            cands.push(format!("{}.{}", scope, r).into());
        }
        for c in &cands {
            if self.signal_name_to_id.contains_key(c) {
                return false;
            }
            let mut base: &str = c;
            while let Some(p) = base.rfind('[') {
                base = &base[..p];
                if self.array_first_id.contains_key(base) {
                    return self.array_has_unnamed_cells(base);
                }
                if !base.ends_with(']') {
                    break;
                }
            }
        }
        false
    }

    /// Resolve a collected READ name to a signal id, falling back to
    /// stripping trailing member segments.
    ///
    /// §7.2: a packed struct is ONE signal (`st`), but a member read collects
    /// as `st.m` — a name nothing in the table matches. The entry then simply
    /// lost the dependency, so an `always_comb` whose only input was a struct
    /// member (`bc = st.m[8];`) computed once at time 0 and froze: stale but
    /// KNOWN values, with no X anywhere to flag the miss. Stripping from the
    /// RIGHT (`u.bus.m` → `u.bus`) finds the signal that actually carries the
    /// member's bits — a write to any part of it marks it dirty, which is
    /// exactly the §9.2.2.2 longest-static-prefix sensitivity.
    pub(super) fn resolve_read_name(&self, name: &str, top_prefix: &str) -> Option<usize> {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            return Some(id);
        }
        if let Some(stripped) = name.strip_prefix(top_prefix) {
            if let Some(&id) = self.signal_name_to_id.get(stripped) {
                return Some(id);
            }
        }
        // §27.6: an UNNAMED generate branch exposes its declarations through
        // the implicit `genblk<N>` scope (the runtime resolver strips that
        // segment and reads the bare name). A continuous-assignment RHS that
        // reads `u_mod.genblk1.x` must therefore depend on the SAME signal the
        // value comes from, not on the `u_mod` instance (which is not a
        // readable scalar and never re-triggers, so the entry evaluates once
        // and stays X). Retry the resolved (`<top>.` popped) name with every
        // `genblk<N>` segment removed.
        if let Some(stripped) = strip_genblk_segments(name) {
            if let Some(&id) = self.signal_name_to_id.get(&stripped) {
                return Some(id);
            }
            if let Some(tpop) = stripped.strip_prefix(top_prefix) {
                if let Some(&id) = self.signal_name_to_id.get(tpop) {
                    return Some(id);
                }
            }
        }
        let mut rest = name;
        while let Some(cut) = rest.rfind('.') {
            rest = &rest[..cut];
            if let Some(&id) = self.signal_name_to_id.get(rest) {
                return Some(id);
            }
            if let Some(stripped) = rest.strip_prefix(top_prefix) {
                if let Some(&id) = self.signal_name_to_id.get(stripped) {
                    return Some(id);
                }
            }
        }
        None
    }

    /// Names of ROOT-marked identifiers (`root = Some("$root")`) in an
    /// expression — substituted port actuals whose bare name collides with a
    /// child declaration (core rewrite_expr_impl, issue #128). These resolve
    /// absolutely; the CA dependency machinery uses this set to keep the
    /// scope-candidate requalification off them.
    pub(super) fn collect_rooted_ident_names(e: &Expression, out: &mut HashSet<String>) {
        match &e.kind {
            ExprKind::Ident(h) => {
                if h.root.is_some() {
                    let name = h
                        .path
                        .iter()
                        .map(|s| s.name.name.as_str())
                        .collect::<Vec<_>>()
                        .join(".");
                    out.insert(name);
                }
                for seg in &h.path {
                    for sel in &seg.selects {
                        Self::collect_rooted_ident_names(sel, out);
                    }
                }
            }
            ExprKind::Binary { left, right, .. } => {
                Self::collect_rooted_ident_names(left, out);
                Self::collect_rooted_ident_names(right, out);
            }
            ExprKind::Unary { operand, .. } => Self::collect_rooted_ident_names(operand, out),
            ExprKind::Paren(inner) => Self::collect_rooted_ident_names(inner, out),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::collect_rooted_ident_names(condition, out);
                Self::collect_rooted_ident_names(then_expr, out);
                Self::collect_rooted_ident_names(else_expr, out);
            }
            ExprKind::Concatenation(parts) => {
                for p in parts {
                    Self::collect_rooted_ident_names(p, out);
                }
            }
            ExprKind::Replication { count, exprs } => {
                Self::collect_rooted_ident_names(count, out);
                for p in exprs {
                    Self::collect_rooted_ident_names(p, out);
                }
            }
            ExprKind::Index { expr, index } => {
                Self::collect_rooted_ident_names(expr, out);
                Self::collect_rooted_ident_names(index, out);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                Self::collect_rooted_ident_names(expr, out);
                Self::collect_rooted_ident_names(left, out);
                Self::collect_rooted_ident_names(right, out);
            }
            ExprKind::MemberAccess { expr, .. } => Self::collect_rooted_ident_names(expr, out),
            ExprKind::Call { func, args } => {
                Self::collect_rooted_ident_names(func, out);
                for a in args {
                    Self::collect_rooted_ident_names(a, out);
                }
            }
            ExprKind::SystemCall { args, .. } => {
                for a in args {
                    Self::collect_rooted_ident_names(a, out);
                }
            }
            ExprKind::AssignmentPattern(items) => {
                for it in items {
                    Self::collect_rooted_ident_names(it.expr(), out);
                }
            }
            ExprKind::Inside { expr, ranges } => {
                Self::collect_rooted_ident_names(expr, out);
                for r in ranges {
                    Self::collect_rooted_ident_names(r, out);
                }
            }
            _ => {}
        }
    }

    /// §14.13: the hidden sample mirror of clockvar `leaf` of block `cb`. A
    /// block of a module or interface INSTANCE (`bus.cb`) names its signals
    /// by their inlined path (`bus.data`), so `bus.cb.data` matches the
    /// signal whose last segment is `data`.
    pub(super) fn clocking_mirror_name(module: &ElaboratedModule, cb: &str, leaf: &str) -> String {
        let sig = module.clocking_blocks.get(cb).and_then(|cd| {
            cd.signals
                .iter()
                .find(|s| s.name.name == leaf)
                .or_else(|| {
                    cd.signals
                        .iter()
                        .find(|s| s.name.name.rsplit('.').next() == Some(leaf))
                })
                .map(|s| s.name.name.as_str())
        });
        format!("{}{}.{}", CB_SAMPLE_PREFIX, cb, sig.unwrap_or(leaf))
    }

    pub(super) fn collect_expr_reads(
        expr: &Expression,
        module: &ElaboratedModule,
        reads: &mut HashSet<String>,
    ) {
        // §14.13: a clocking-signal read (`cb.sig`, `iface.cb.sig`, either the
        // flat-Ident or MemberAccess shape) depends on the hidden sample mirror,
        // NOT on the underlying net — the sample only moves at the block's clock
        // edge. Registering the net instead made `always @(cb.sig)` re-fire on
        // the net's own (possibly mid-cycle) changes.
        //
        // PERF: `flatten_member_path` clones every path segment into a fresh
        // `Vec<String>` and the `cb` join allocates again, on EVERY expression
        // node of the design. With no clocking block declared anywhere the
        // `contains_key` below can never hit, so the whole probe is dead work —
        // skip it. Behaviour is identical: an empty map answers `false`.
        if !module.clocking_blocks.is_empty()
            && let Some(segs) = Self::flatten_member_path(expr)
        {
            if segs.len() >= 2 {
                let cb = segs[..segs.len() - 1].join(".");
                if module.clocking_blocks.contains_key(&cb) {
                    reads.insert(Self::clocking_mirror_name(
                        module,
                        &cb,
                        &segs[segs.len() - 1],
                    ));
                    return;
                }
            }
        }
        match &expr.kind {
            ExprKind::Ident(hier) => {
                let name = Self::resolve_hier_name_static(hier, module);
                // A packed struct/union member (`bar.bar0`) parses as a dotted
                // hierarchical Ident but has no leaf signal — its value is a bit
                // slice of the parent. Register the parent (when it is an actual
                // signal) so a whole-struct write (`bar = ...`) re-fires readers
                // like `assign foo = bar.bar0`. Without it the dotted name never
                // receives a change event and the reader stays X.
                // A COLLECTION query/reduction (`q.sum()`, `d.size()`)
                // parses as a dotted Ident but is NOT a member signal: the
                // name `q.sum` resolves to nothing, so the entry depended on
                // something that never exists. A continuous assign survived
                // that by accident (an unresolved read re-evaluates on every
                // settle); an `always_comb` reading `q.sum()` never re-fired
                // on an element write and returned 0 forever. Depend on what
                // element writes actually mark: the elements themselves, plus
                // the `.size` proxy for resize.
                if let Some(dot) = name.rfind('.') {
                    let (base, meth) = (&name[..dot], &name[dot + 1..]);
                    if matches!(
                        meth,
                        "sum" | "product" | "min" | "max" | "and" | "or" | "xor" | "size" | "num"
                    ) && (module.dynamic_arrays.contains(base)
                        || module.queue_vars.contains(base)
                        || module.associative_arrays.contains_key(base))
                    {
                        let base = base.to_string();
                        reads.insert(format!("{}.size", base));
                        if let Some((lo, hi, _)) = module.arrays.get(&base) {
                            for i in *lo..=*hi {
                                reads.insert(format!("{}[{}]", base, i));
                            }
                        }
                        return;
                    }
                }
                if let Some(dot) = name.rfind('.') {
                    let parent = &name[..dot];
                    if !module.signals.contains_key(&name)
                        && (module.signals.contains_key(parent)
                            || module.packed_struct_fields.contains_key(parent))
                    {
                        reads.insert(parent.to_string());
                    }
                }
                note_whole_read(&name);
                reads.insert(name);
            }
            ExprKind::Index { expr: base, index } => {
                if let ExprKind::Ident(hier) = &base.kind {
                    let name = Self::resolve_hier_name_static(hier, module);
                    // A dynamic array / queue keeps its elements in the runtime
                    // map with no per-element id — depend on the `.size` proxy,
                    // which every mutation marks dirty (see Simulator::new).
                    if module.dynamic_arrays.contains(&name)
                        || (module.associative_arrays.contains_key(&name)
                            && !module.arrays.contains_key(&name)
                            && !module.arrays_2d.contains_key(&name)
                            && !module.arrays_nd.contains_key(&name))
                    {
                        // §9.2.2.2: an ASSOCIATIVE array had no proxy at all,
                        // so a comb block reading `aa[k]` recorded NO
                        // dependency and never re-fired. It now shares the
                        // `.size` proxy the dynamic/queue case uses.
                        reads.insert(format!("{}.size", name));
                        // ...but the proxy is only touched when the collection
                        // RESIZES. A dynamic array is registered in
                        // `module.arrays` as well, so its elements own signal
                        // ids, and an element STORE marks those ids -- never
                        // the proxy. Depending on the proxy alone left a reader
                        // of `d[i]` frozen on whatever it saw during the time-0
                        // settle: every later write, blocking or NBA, marked an
                        // id nothing depended on. (A write issued BEFORE that
                        // settle appeared to work, which is what made this look
                        // NBA-specific.) Record the element dependency too,
                        // exactly as the fixed-array arm below does.
                        if let Some((lo, hi, _)) = module.arrays.get(&name) {
                            if let Some(i) = Self::constant_array_index(index, module) {
                                if (*lo..=*hi).contains(&i) {
                                    reads.insert(format!("{}[{}]", name, i));
                                }
                            } else {
                                for i in *lo..=*hi {
                                    reads.insert(format!("{}[{}]", name, i));
                                }
                            }
                        }
                    } else if let Some((lo, hi, _)) = module.arrays.get(&name) {
                        if let Some(i) = Self::constant_array_index(index, module) {
                            if (*lo..=*hi).contains(&i) {
                                reads.insert(format!("{}[{}]", name, i));
                            }
                        } else {
                            for i in *lo..=*hi {
                                reads.insert(format!("{}[{}]", name, i));
                            }
                        }
                    } else {
                        note_whole_read(&name);
                        reads.insert(name);
                    }
                } else if let Some(elems) = Self::multi_dim_elem_names(base, index, module) {
                    for e in elems {
                        reads.insert(e);
                    }
                    let mut cur = base.as_ref();
                    while let ExprKind::Index {
                        expr: inner,
                        index: inner_index,
                    } = &cur.kind
                    {
                        if Self::constant_array_index(inner_index, module).is_none() {
                            Self::collect_expr_reads(inner_index, module, reads);
                        }
                        cur = inner;
                    }
                } else {
                    // Non-Ident base (e.g. nested Index, RangeSelect from inlining
                    // `.data(parent[hi:lo])` then sub-module reads `data[i]` →
                    // expression becomes `parent[hi:lo][i]`). Without this recurse,
                    // the underlying signal isn't registered as a dependency and
                    // the comb-entry never re-evaluates when it changes.
                    Self::collect_expr_reads(base, module, reads);
                }
                if Self::constant_array_index(index, module).is_none() {
                    Self::collect_expr_reads(index, module, reads);
                }
            }
            ExprKind::RangeSelect {
                expr: base,
                left,
                right,
                ..
            } => {
                Self::collect_expr_reads(base, module, reads);
                Self::collect_expr_reads(left, module, reads);
                Self::collect_expr_reads(right, module, reads);
            }
            ExprKind::Unary { operand, .. } => Self::collect_expr_reads(operand, module, reads),
            // Streaming concat (`{<<{src}}`): the operands are ordinary reads.
            // With no arm here a continuous `assign dst = {<<{src}};` had an
            // EMPTY read set — it sampled X at the time-0 settle and never
            // re-fired when `src` changed, so the target stayed X forever
            // (procedural streaming was always fine; only the dependency
            // collection was blind to it).
            ExprKind::StreamOp {
                slice_size, exprs, ..
            } => {
                if let Some(sz) = slice_size {
                    Self::collect_expr_reads(sz, module, reads);
                }
                for e in exprs {
                    Self::collect_expr_reads(e, module, reads);
                }
            }
            ExprKind::Binary { left, right, .. } => {
                Self::collect_expr_reads(left, module, reads);
                Self::collect_expr_reads(right, module, reads);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::collect_expr_reads(condition, module, reads);
                Self::collect_expr_reads(then_expr, module, reads);
                Self::collect_expr_reads(else_expr, module, reads);
            }
            ExprKind::Concatenation(exprs) => {
                for e in exprs {
                    Self::collect_expr_reads(e, module, reads);
                }
            }
            ExprKind::AssignmentPattern(parts) => {
                for p in parts {
                    Self::collect_expr_reads(p.expr(), module, reads);
                }
            }
            ExprKind::Replication { count, exprs } => {
                Self::collect_expr_reads(count, module, reads);
                for e in exprs {
                    Self::collect_expr_reads(e, module, reads);
                }
            }
            ExprKind::Call { func, args } => {
                Self::collect_expr_reads(func, module, reads);
                for a in args {
                    Self::collect_expr_reads(a, module, reads);
                }
                // IEEE 1800-2017 §9.2.2.2: an always_comb (and always_latch)
                // is sensitive to EVERY variable read in its evaluation,
                // INCLUDING variables read inside a function it calls. Recurse
                // into the callee body so module vars referenced only inside
                // the function (e.g. `hidden`) join the inferred sensitivity
                // list; without this the block never re-fires when they change.
                // §9.2.2.2 note: `always @*` is sensitive only to the
                // function's ARGUMENTS (collected above), NOT its contents —
                // only always_comb/always_latch descend into the body. The
                // caller signals which mode applies via SKIP_FN_BODY_READS.
                if !SKIP_FN_BODY_READS.with(|c| c.get()) {
                    if let ExprKind::Ident(hier) = &func.kind {
                        if let Some(seg) = hier.path.last() {
                            // Prefer the fully-scoped registration: an inlined
                            // instance's call site resolves to `<inst>.<name>`, and
                            // only under THAT key do the body's bare reads get the
                            // instance prefix (see `collect_function_reads`). The
                            // bare key may also exist (from the module definition)
                            // and would collect unprefixed, unresolvable reads.
                            let full = Self::resolve_hier_name_static(hier, module);
                            if full != seg.name.name && module.functions.contains_key(&full) {
                                Self::collect_callee_reads(&full, module, reads);
                            } else {
                                Self::collect_callee_reads(&seg.name.name, module, reads);
                            }
                        }
                    }
                }
            }
            ExprKind::SystemCall { name, args } => {
                // A named cast `some_type_e'(x)` lowers to
                // `$__xz_named_cast(some_type_e, x)`, where arg 0 is a TYPE
                // name, not a signal. Collecting it as a read leaves a name
                // that can never resolve, which pins `has_unresolved_reads`
                // and re-evaluates the entry on EVERY settle call for the
                // life of the run. Measured on Ibex: 9 of 10 permanently
                // unresolved comb entries were exactly this shape
                // (`csr_num_e`, `pmp_cfg_t`, `ibex_mubi_t`, ...).
                let skip_first = name == "$__xz_named_cast";
                for a in args.iter().skip(usize::from(skip_first)) {
                    Self::collect_expr_reads(a, module, reads);
                }
            }
            ExprKind::Paren(e) => Self::collect_expr_reads(e, module, reads),
            // §11.4.13: `v inside {a, [lo:hi]}` reads the tested expression
            // AND every set member — none were collected, so a comb/cont
            // assign with an `inside` RHS never re-fired and kept its time-0
            // value forever.
            ExprKind::Inside { expr: e, ranges } => {
                Self::collect_expr_reads(e, module, reads);
                for r in ranges {
                    Self::collect_expr_reads(r, module, reads);
                }
            }
            ExprKind::Range(a, b) => {
                Self::collect_expr_reads(a, module, reads);
                Self::collect_expr_reads(b, module, reads);
            }
            ExprKind::Matches { expr: e, .. } => {
                Self::collect_expr_reads(e, module, reads);
            }
            // Interface-member access (`intf.member`) and other dotted refs.
            // Recursing only into the base loses the actual signal name —
            // writes to `intf.member` would not re-fire combinational blocks
            // that read it (boot_addr_i stuck at X, AXI fields stale, ...).
            // Flatten the chain into a dotted name and register THAT — do not
            // also re-register every prefix (that quadratics the dep set on
            // deep package/scope chains).
            // Per the engineering notes: do NOT guard on `module.signals`
            // containment — interface members are not yet there at the time
            // this is called.
            ExprKind::MemberAccess { expr: e, member } => {
                let mut parts: Vec<String> = vec![member.name.clone()];
                let mut cur: &Expression = e.as_ref();
                let mut flattened = false;
                loop {
                    match &cur.kind {
                        ExprKind::MemberAccess { expr: b, member: m } => {
                            parts.push(m.name.clone());
                            cur = b.as_ref();
                        }
                        ExprKind::Ident(h) => {
                            for seg in h.path.iter().rev() {
                                parts.push(seg.name.name.clone());
                            }
                            parts.reverse();
                            let name = parts.join(".");
                            note_whole_read(&name);
                            reads.insert(name);
                            // A member of a PACKED struct/union has no separate
                            // leaf signal — the value lives in the parent's bit
                            // slice. Register the parent when it is an actual
                            // signal so a write to the whole struct (e.g.
                            // `word1 = ...`) re-fires this reader
                            // (`assign foo = word1.high`). Without it the dotted
                            // name never receives a change event.
                            if parts.len() >= 2 {
                                let parent = parts[..parts.len() - 1].join(".");
                                if module.signals.contains_key(&parent)
                                    || module.packed_struct_fields.contains_key(&parent)
                                {
                                    reads.insert(parent);
                                }
                            }
                            flattened = true;
                            break;
                        }
                        _ => break,
                    }
                }
                if !flattened {
                    // `arr[i].field` (and deeper member chains): the leaves are
                    // per-element FIELD signals (`arr[0].field`), and a write
                    // dirties exactly those — register them, for every element
                    // when the index is not constant. Without this the only
                    // registered read was the elementless base and a field
                    // write never re-fired the block. A dynamic array's
                    // elements have no ids; depend on its `.size` proxy.
                    if let ExprKind::Index { expr: ib, index } = &cur.kind {
                        if let ExprKind::Ident(h) = &ib.kind {
                            let name = Self::resolve_hier_name_static(h, module);
                            let suffix = parts.iter().rev().cloned().collect::<Vec<_>>().join(".");
                            if module.dynamic_arrays.contains(&name) {
                                reads.insert(format!("{}.size", name));
                            } else if let Some(&(lo, hi, _)) = module.arrays.get(&name) {
                                if let Some(i) = Self::constant_array_index(index, module) {
                                    if (lo..=hi).contains(&i) {
                                        reads.insert(format!("{}[{}].{}", name, i, suffix));
                                    }
                                } else {
                                    for i in lo..=hi {
                                        reads.insert(format!("{}[{}].{}", name, i, suffix));
                                    }
                                }
                            } else if let Some(i) = Self::constant_array_index(index, module) {
                                // `gl[1].v` — a FOR-generate scope member
                                // (§27.6) or instance-array leaf: the storage
                                // key carries the rendered index. `gl` is
                                // neither a signal nor an array, so this fell
                                // through to depending on the bare label and a
                                // continuous assign reading a for-generate
                                // signal never re-fired (stayed X while the
                                // identical procedural read worked). Register
                                // the flat name unguarded — `module.signals`
                                // may already be drained into the compact
                                // table at this point, and a name that maps to
                                // no id is dropped at id-resolution exactly
                                // like the Ident arm's inserts. Dotted
                                // ancestors cover a packed member whose
                                // storage is the containing struct
                                // (`gl[1].s.field` -> `gl[1].s`).
                                let cand = format!("{}[{}].{}", name, i, suffix);
                                let mut rest = cand.as_str();
                                while let Some(cut) = rest.rfind('.') {
                                    rest = &rest[..cut];
                                    if rest.contains('.') {
                                        reads.insert(rest.to_string());
                                    }
                                }
                                reads.insert(cand);
                            }
                        }
                    }
                    // Non-Ident base (`(expr).member`, indexed-member, etc.) —
                    // fall back to the prior behavior of recursing into the
                    // base expression so its inner reads get registered.
                    Self::collect_expr_reads(e, module, reads);
                }
            }
            _ => {} // Number, StringLiteral, Dollar, Null, etc.
        }
    }

    /// Collect signal names written by an LHS expression.
    /// Targets written by an expression used as a STATEMENT: increment and
    /// decrement operators and an embedded assignment expression. Walks
    /// through parens and into both arms of a comma/concat so a compound
    /// statement expression records every target.
    pub(super) fn collect_expr_stmt_writes(
        e: &Expression,
        module: &ElaboratedModule,
        writes: &mut HashSet<String>,
    ) {
        match &e.kind {
            ExprKind::Paren(inner) => Self::collect_expr_stmt_writes(inner, module, writes),
            ExprKind::Unary { op, operand }
                if matches!(
                    op,
                    UnaryOp::PreIncr | UnaryOp::PostIncr | UnaryOp::PreDecr | UnaryOp::PostDecr
                ) =>
            {
                Self::collect_lhs_writes(operand, module, writes);
            }
            ExprKind::AssignExpr { lvalue, rvalue } => {
                Self::collect_lhs_writes(lvalue, module, writes);
                Self::collect_expr_stmt_writes(rvalue, module, writes);
            }
            _ => {}
        }
    }

    pub(super) fn collect_lhs_writes(
        lhs: &Expression,
        module: &ElaboratedModule,
        writes: &mut HashSet<String>,
    ) {
        Self::collect_lhs_writes_ex(lhs, module, writes, None)
    }

    /// Whole-array writes report the ARRAY when `arrays` is given, instead of
    /// one interned name per element: a 1M-entry memory produced a million
    /// `String`s here, which the caller turned straight back into a
    /// contiguous id range (0.75 s per SRAM block on a c906 SoC).
    pub(super) fn collect_lhs_writes_ex(
        lhs: &Expression,
        module: &ElaboratedModule,
        writes: &mut HashSet<String>,
        mut arrays: Option<&mut HashSet<String>>,
    ) {
        match &lhs.kind {
            ExprKind::Ident(hier) => {
                writes.insert(Self::resolve_hier_name_static(hier, module));
            }
            ExprKind::Index { expr: base, index } => {
                if let Some(elems) = Self::multi_dim_elem_names(base, index, module) {
                    writes.extend(elems);
                    return;
                }
                match &base.kind {
                    ExprKind::Ident(hier) => {
                        let name = Self::resolve_hier_name_static(hier, module);
                        if let Some(&(lo, hi, _)) = module.arrays.get(&name) {
                            // A CONSTANT index — a literal, or what a genvar
                            // unrolls to — names ONE element. Claiming the
                            // whole array regardless made two generate arms
                            // writing `mem[0]` and `mem[1]` two drivers of
                            // every element, so neither could skip an idle
                            // edge. An index computed at run time still
                            // claims the array, and so does one out of range,
                            // where the fold is the only thing saying so.
                            let single = Self::constant_array_index(index, module)
                                .filter(|i| (lo.min(hi)..=lo.max(hi)).contains(i));
                            if let Some(i) = single {
                                writes.insert(format!("{}[{}]", name, i));
                            } else if let Some(a) = arrays.as_deref_mut() {
                                a.insert(name);
                            } else {
                                for i in lo..=hi {
                                    writes.insert(format!("{}[{}]", name, i));
                                }
                            }
                        } else {
                            writes.insert(name);
                        }
                    }
                    ExprKind::MemberAccess { expr, member } => {
                        if let ExprKind::Ident(hier) = &expr.kind {
                            let mut name = Self::resolve_hier_name_static(hier, module);
                            if !name.is_empty() {
                                name.push('.');
                                name.push_str(&member.name);
                                writes.insert(name);
                            }
                        }
                    }
                    // FIX: nested Index/RangeSelect (from inlining sliced port
                    // connections) — recurse to find the underlying signal so
                    // the write is correctly tracked.
                    _ => Self::collect_lhs_writes_ex(base, module, writes, arrays.as_deref_mut()),
                }
            }
            ExprKind::MemberAccess { expr, member } => {
                if let ExprKind::Ident(hier) = &expr.kind {
                    let mut name = Self::resolve_hier_name_static(hier, module);
                    if !name.is_empty() {
                        name.push('.');
                        name.push_str(&member.name);
                        writes.insert(name);
                    }
                }
            }
            ExprKind::RangeSelect { expr: base, .. } => {
                Self::collect_lhs_writes_ex(base, module, writes, arrays.as_deref_mut())
            }
            ExprKind::Concatenation(exprs) => {
                for e in exprs {
                    Self::collect_lhs_writes_ex(e, module, writes, arrays.as_deref_mut());
                }
            }
            _ => {}
        }
    }

    /// Collect reads/writes from a statement (for always @* / always_comb blocks).
    pub(super) fn collect_stmt_reads(
        stmt: &Statement,
        module: &ElaboratedModule,
        reads: &mut HashSet<String>,
        writes: &mut HashSet<String>,
    ) {
        Self::collect_stmt_rw(stmt, module, Some(reads), writes, None)
    }

    /// `reads` is optional — a caller that needs only the write set skips
    /// building read names at all — and `arrays` collects whole-array write
    /// targets instead of one name per element (see `collect_lhs_writes_ex`).
    pub(super) fn collect_stmt_rw(
        stmt: &Statement,
        module: &ElaboratedModule,
        mut reads: Option<&mut HashSet<String>>,
        writes: &mut HashSet<String>,
        mut arrays: Option<&mut HashSet<String>>,
    ) {
        match &stmt.kind {
            StatementKind::BlockingAssign { lvalue, rvalue }
            | StatementKind::NonblockingAssign { lvalue, rvalue, .. } => {
                if let Some(r) = reads.as_deref_mut() {
                    Self::collect_expr_reads(rvalue, module, r)
                };
                Self::collect_lhs_writes_ex(lvalue, module, writes, arrays.as_deref_mut());
                // Also read the index expression of the LHS if it's an array/range select
                if let Some(r) = reads.as_deref_mut() {
                    Self::collect_lhs_index_reads(lvalue, module, r)
                };
            }
            StatementKind::If {
                condition,
                then_stmt,
                else_stmt,
                ..
            } => {
                if let Some(r) = reads.as_deref_mut() {
                    Self::collect_expr_reads(condition, module, r)
                };
                Self::collect_stmt_rw(
                    then_stmt,
                    module,
                    reads.as_deref_mut(),
                    writes,
                    arrays.as_deref_mut(),
                );
                if let Some(el) = else_stmt {
                    Self::collect_stmt_rw(
                        el,
                        module,
                        reads.as_deref_mut(),
                        writes,
                        arrays.as_deref_mut(),
                    );
                }
            }
            StatementKind::Case { expr, items, .. } => {
                if let Some(r) = reads.as_deref_mut() {
                    Self::collect_expr_reads(expr, module, r)
                };
                for item in items {
                    for pat in &item.patterns {
                        if let Some(r) = reads.as_deref_mut() {
                            Self::collect_expr_reads(pat, module, r)
                        };
                    }
                    Self::collect_stmt_rw(
                        &item.stmt,
                        module,
                        reads.as_deref_mut(),
                        writes,
                        arrays.as_deref_mut(),
                    );
                }
            }
            StatementKind::For {
                init,
                condition,
                step,
                body,
            } => {
                for fi in init {
                    match fi {
                        ForInit::Assign { lvalue, rvalue } => {
                            if let Some(r) = reads.as_deref_mut() {
                                Self::collect_expr_reads(rvalue, module, r)
                            };
                            Self::collect_lhs_writes_ex(
                                lvalue,
                                module,
                                writes,
                                arrays.as_deref_mut(),
                            );
                        }
                        ForInit::VarDecl {
                            name, init: rvalue, ..
                        } => {
                            if let Some(r) = reads.as_deref_mut() {
                                Self::collect_expr_reads(rvalue, module, r)
                            };
                            writes.insert(name.name.clone());
                        }
                    }
                }
                if let Some(c) = condition {
                    if let Some(r) = reads.as_deref_mut() {
                        Self::collect_expr_reads(c, module, r)
                    };
                }
                // Step expressions are typically i = i + 1, parsed as
                // Binary { op: Assign, left, right }. Collect both reads and
                // LHS writes so loop variables are excluded from sensitivity.
                for s in step {
                    if let Some(r) = reads.as_deref_mut() {
                        Self::collect_expr_reads(s, module, r)
                    };
                    if let ExprKind::Binary {
                        op: BinaryOp::Assign,
                        left,
                        ..
                    } = &s.kind
                    {
                        Self::collect_lhs_writes_ex(left, module, writes, arrays.as_deref_mut());
                    }
                }
                Self::collect_stmt_rw(
                    body,
                    module,
                    reads.as_deref_mut(),
                    writes,
                    arrays.as_deref_mut(),
                );
            }
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                for s in stmts {
                    Self::collect_stmt_rw(
                        s,
                        module,
                        reads.as_deref_mut(),
                        writes,
                        arrays.as_deref_mut(),
                    );
                }
            }
            StatementKind::Expr(e) => {
                if let Some(r) = reads.as_deref_mut() {
                    Self::collect_expr_reads(e, module, r)
                };
                // An expression STATEMENT can also write: `cnt++`, `--cnt`,
                // and `(x = y)` all assign their target. Only reads were
                // recorded here, so the target survived the implicit
                // sensitivity list (`reads - writes`) and an `always @(*)`
                // that incremented a counter re-triggered itself forever
                // (§9.4.2.2 — a variable assigned by the block is not part of
                // its own inferred sensitivity).
                Self::collect_expr_stmt_writes(e, module, writes);
            }
            StatementKind::While { condition, body }
            | StatementKind::DoWhile { body, condition } => {
                if let Some(r) = reads.as_deref_mut() {
                    Self::collect_expr_reads(condition, module, r)
                };
                Self::collect_stmt_rw(
                    body,
                    module,
                    reads.as_deref_mut(),
                    writes,
                    arrays.as_deref_mut(),
                );
            }
            StatementKind::Forever { body }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Foreach { body, .. } => {
                Self::collect_stmt_rw(
                    body,
                    module,
                    reads.as_deref_mut(),
                    writes,
                    arrays.as_deref_mut(),
                );
            }
            // §9.2.2.2: a function body reached through `collect_function_reads`
            // usually ends in `return <expr>` — dropping it hid every module
            // variable the function reads only there, so an always_comb calling
            // it never re-fired on those variables.
            StatementKind::Return(Some(e)) => {
                if let Some(r) = reads.as_deref_mut() {
                    Self::collect_expr_reads(e, module, r)
                };
            }
            // §9.2.2.2.1 / §16.3: an immediate or deferred assertion's
            // expression and action blocks are part of the procedure, so an
            // `always_comb` / `always @*` whose only reads are in an assertion
            // still re-runs when they change. A concurrent assertion samples
            // on its own clock and adds nothing.
            StatementKind::Assertion(a) if !a.is_property => {
                if let Some(r) = reads.as_deref_mut() {
                    Self::collect_expr_reads(&a.expr, module, r)
                };
                for act in [&a.action, &a.else_action].into_iter().flatten() {
                    Self::collect_stmt_rw(
                        act,
                        module,
                        reads.as_deref_mut(),
                        writes,
                        arrays.as_deref_mut(),
                    );
                }
            }
            StatementKind::VarDecl { declarators, .. } => {
                for d in declarators {
                    // A block-local is written by this block, never an external
                    // trigger — but its INITIALIZER reads are real inputs.
                    writes.insert(d.name.name.clone());
                    if let Some(init) = &d.init {
                        if let Some(r) = reads.as_deref_mut() {
                            Self::collect_expr_reads(init, module, r)
                        };
                    }
                }
            }
            _ => {}
        }
    }

    /// Collect reads from index expressions on the LHS (e.g., array[idx] — idx is read).
    pub(super) fn collect_lhs_index_reads(
        lhs: &Expression,
        module: &ElaboratedModule,
        reads: &mut HashSet<String>,
    ) {
        match &lhs.kind {
            ExprKind::Index { expr: base, index } => {
                Self::collect_expr_reads(index, module, reads);
                // Recurse into base for nested LHS like arr[i][j] or sliced lhs.
                Self::collect_lhs_index_reads(base, module, reads);
            }
            ExprKind::RangeSelect {
                expr: base,
                left,
                right,
                ..
            } => {
                Self::collect_expr_reads(left, module, reads);
                Self::collect_expr_reads(right, module, reads);
                Self::collect_lhs_index_reads(base, module, reads);
            }
            _ => {}
        }
    }

    /// Static version of resolve_hier_name (doesn't need &self).
    pub(super) fn resolve_hier_name_static(
        hier: &HierarchicalIdentifier,
        _module: &ElaboratedModule,
    ) -> String {
        // PERF: the overwhelmingly common shape is a single segment. `join`
        // on a one-element `Vec<&str>` still heap-allocates the Vec and walks
        // it twice; clone the segment directly instead. Multi-segment paths
        // build the same string the `join` did, in one pass with an exact
        // up-front capacity. Result is byte-for-byte what `join(".")` returns.
        // §23.6: `$root` names the root scope — it contributes nothing to the
        // flat key (`$root.tb.x` == the absolute `tb.x`, which the top-prefix
        // strip already resolves). The parser keeps it as an ordinary first
        // segment; joining it verbatim produced a name that matched nothing,
        // so every `$root.` read was a 32-bit zero and every write vanished.
        let skip_root = usize::from(hier.path.len() > 1 && hier.path[0].name.name == "$root");
        let path = &hier.path[skip_root..];
        match path.len() {
            0 => String::new(),
            1 => path[0].name.name.clone(),
            _ => {
                let cap = path.iter().map(|s| s.name.name.len() + 1).sum::<usize>() - 1;
                let mut out = String::with_capacity(cap);
                for (i, s) in path.iter().enumerate() {
                    if i > 0 {
                        out.push('.');
                    }
                    out.push_str(s.name.name.as_str());
                    // A CONSTANT segment select is part of the flat name:
                    // `gl[1].v` names a for-generate iteration (§27.6) and
                    // `u[0].x` an instance-array element — both are stored
                    // under keys with the index rendered. Dropping the select
                    // produced `gl.v`, which names nothing, so a continuous
                    // assign reading a for-generate signal registered no
                    // dependency and stayed X forever. Non-constant selects
                    // keep the historical drop (a variable index cannot name
                    // a generate scope, §27.5).
                    for sel in &s.selects {
                        if let Some(ix) = Self::try_const_u64(sel) {
                            use std::fmt::Write as _;
                            let _ = write!(out, "[{}]", ix);
                        }
                    }
                }
                out
            }
        }
    }

    /// Resolve a hier ident to its signal_id, retrying with scope_hint prefix
    /// for bare names. Returns None if unresolved.
    pub(super) fn resolve_ident_id(
        &self,
        hier: &HierarchicalIdentifier,
        scope_hint: Option<&str>,
    ) -> Option<usize> {
        let name = Self::resolve_hier_name_static(hier, &self.module);
        if let Some(&id) = self.signal_name_to_id.get(name.as_str()) {
            return Some(id);
        }
        // §26.3: `pkg::name` of a contested package variable or parameter
        // has its own `<pkg>::<name>` cell (see `pkg_var_storage`).
        if hier.path.len() == 2
            && hier.path.iter().all(|s| s.selects.is_empty())
            && self.module.packages.contains(&hier.path[0].name.name)
            && let Some(q) = self.pkg_var_storage(&hier.path[0].name.name, &hier.path[1].name.name)
            && let Some(&id) = self.signal_name_to_id.get(q.as_str())
        {
            return Some(id);
        }
        if let Some(scope) = scope_hint {
            let qualified = format!("{}.{}", scope, name);
            if let Some(&id) = self.signal_name_to_id.get(qualified.as_str()) {
                return Some(id);
            }
        }
        None
    }

    /// Try to evaluate `expr` as a constant non-negative u64 (for bit-select indices).
    pub(super) fn try_const_u64(expr: &Expression) -> Option<u64> {
        match &expr.kind {
            ExprKind::Paren(inner) => Self::try_const_u64(inner),
            ExprKind::Number(NumberLiteral::Integer { value, base, .. }) => {
                let r = match base {
                    NumberBase::Binary => 2,
                    NumberBase::Octal => 8,
                    NumberBase::Hex => 16,
                    NumberBase::Decimal => 10,
                };
                let cleaned: String = value.chars().filter(|c| *c != '_').collect();
                u64::from_str_radix(&cleaned, r).ok()
            }
            _ => None,
        }
    }

    /// Try to resolve `expr` to a single-bit reference (sig_id, bit_index).
    /// Handles bare identifiers (1-bit signals) and constant-index bit-selects.
    pub(super) fn try_resolve_bit_ref(
        &self,
        expr: &Expression,
        scope_hint: Option<&str>,
    ) -> Option<BitRef> {
        match &expr.kind {
            ExprKind::Paren(inner) => self.try_resolve_bit_ref(inner, scope_hint),
            ExprKind::Ident(hier) => {
                let id = self.resolve_ident_id(hier, scope_hint)?;
                if self.signal_widths[id] == 1 {
                    Some(BitRef {
                        sig_id: id as u32,
                        bit: 0,
                    })
                } else {
                    None
                }
            }
            ExprKind::Index { expr: base, index } => {
                let hier = if let ExprKind::Ident(h) = &base.kind {
                    h
                } else {
                    return None;
                };
                // Reject array element access (e.g. `reg mem [3:0]; mem[0]`) —
                // this is NOT a bit-select on a packed vector. Treating it as
                // such would read bit 0 of the whole-array signal storage
                // instead of element 0.
                let name = Self::resolve_hier_name_static(hier, &self.module);
                if self.module.arrays.contains_key(&name) {
                    return None;
                }
                if let Some(scope) = scope_hint {
                    if self
                        .module
                        .arrays
                        .contains_key(&format!("{}.{}", scope, name))
                    {
                        return None;
                    }
                }
                // Reject packed multi-D element access (`logic [N-1:0][W-1:0] p;
                // p[i]` with W>1). `p[i]` selects a W-bit element, NOT a single
                // bit — fusing it as a bit gate would read/write bit `i` instead
                // of element `i`. This was the FlooNOC "router never forwards"
                // root cause: common_cells rr_arb_tree/lzc build their index
                // tree with `assign idx_nodes[n] = idx_lut[k]` over packed-2D
                // signals; mis-fused as bit copies, the arbiter's selected
                // index came out wrong and no output valid ever asserted. Let
                // these fall through to compile_cont_assign_lhs, which emits a
                // correct elem_w-wide range read/write.
                let elem_w = self
                    .module
                    .packed_signal_elem_widths
                    .get(&name)
                    .copied()
                    .or_else(|| {
                        hier.path.last().and_then(|s| {
                            self.module
                                .packed_signal_elem_widths
                                .get(s.name.name.as_str())
                                .copied()
                        })
                    });
                if elem_w.map_or(false, |w| w > 1) {
                    return None;
                }
                let id = self.resolve_ident_id(hier, scope_hint)?;
                let bit = Self::try_const_u64(index)?;
                // §7.4.1: `bit` is a DECLARED index but `BitRef` addresses a
                // PHYSICAL bit. A descending `logic [3:1] w` keeps declared
                // bit 1 at offset 0. An ascending `logic [0:N] v` mirrors:
                // label 0 is the MSB. Subtracting only the low bound wrote
                // `v[0] = in_valid` into the LSB, which the interpreter then
                // read back as `v[N]` — a one-stage pipeline's out_valid
                // tracked in_valid and its ready bit stayed x.
                let scoped = scope_hint.map(|sc| format!("{}.{}", sc, name));
                let dim = self
                    .module
                    .packed_full_dims
                    .get(&name)
                    .or_else(|| scoped.as_ref().and_then(|k| self.module.packed_full_dims.get(k)))
                    .or_else(|| {
                        hier.path
                            .last()
                            .and_then(|s| self.module.packed_full_dims.get(s.name.name.as_str()))
                    })
                    .and_then(|d| d.first())
                    .copied()
                    .or_else(|| {
                        self.module
                            .ascending_packed
                            .get(&name)
                            .or_else(|| {
                                scoped
                                    .as_ref()
                                    .and_then(|k| self.module.ascending_packed.get(k))
                            })
                            .or_else(|| {
                                hier.path.last().and_then(|s| {
                                    self.module.ascending_packed.get(s.name.name.as_str())
                                })
                            })
                            .copied()
                    });
                let phys = match dim {
                    Some((l, r)) if l < r => r - bit as i64,
                    Some((l, r)) => bit as i64 - l.min(r),
                    None => bit as i64,
                };
                if phys >= 0 && (phys as u32) < self.signal_widths[id] {
                    Some(BitRef {
                        sig_id: id as u32,
                        bit: phys as u32,
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Attempt to recognize a yosys-style gate pattern and return a fused op.
    /// Only fires when LHS and all RHS leaves are single-bit refs with no SDF delay.
    pub(super) fn try_build_fused_gate(
        &self,
        lhs: &Expression,
        rhs: &Expression,
        scope_hint: Option<&str>,
    ) -> Option<FusedGate> {
        let dst = self.try_resolve_bit_ref(lhs, scope_hint)?;
        if self
            .sdf_delays
            .get(dst.sig_id as usize)
            .copied()
            .unwrap_or(0)
            != 0
        {
            return None;
        }
        // Strip outer parens on rhs
        fn unparen(e: &Expression) -> &Expression {
            if let ExprKind::Paren(inner) = &e.kind {
                unparen(inner)
            } else {
                e
            }
        }
        let r = unparen(rhs);
        // 1) Conditional (mux): s ? t : e
        if let ExprKind::Conditional {
            condition,
            then_expr,
            else_expr,
        } = &r.kind
        {
            let s = self.try_resolve_bit_ref(condition, scope_hint)?;
            let t = self.try_resolve_bit_ref(then_expr, scope_hint)?;
            let e = self.try_resolve_bit_ref(else_expr, scope_hint)?;
            return Some(FusedGate::Mux2 { dst, s, t, e });
        }
        // 2) Unary BitNot: ~X where X is leaf or binary
        if let ExprKind::Unary {
            op: UnaryOp::BitNot,
            operand,
        } = &r.kind
        {
            let inner = unparen(operand);
            if let ExprKind::Binary { op, left, right } = &inner.kind {
                let gop = match op {
                    BinaryOp::BitAnd => GateBin::And,
                    BinaryOp::BitOr => GateBin::Or,
                    BinaryOp::BitXor => GateBin::Xor,
                    _ => return None,
                };
                let a = self.try_resolve_bit_ref(left, scope_hint)?;
                let b = self.try_resolve_bit_ref(right, scope_hint)?;
                return Some(FusedGate::Bin2 {
                    dst,
                    a,
                    b,
                    op: gop,
                    invert: true,
                });
            }
            let src = self.try_resolve_bit_ref(operand, scope_hint)?;
            return Some(FusedGate::Buf1 {
                dst,
                src,
                invert: true,
            });
        }
        // 3) Binary: a & b, a | b, a ^ b
        if let ExprKind::Binary { op, left, right } = &r.kind {
            let gop = match op {
                BinaryOp::BitAnd => Some(GateBin::And),
                BinaryOp::BitOr => Some(GateBin::Or),
                BinaryOp::BitXor => Some(GateBin::Xor),
                _ => None,
            };
            if let Some(gop) = gop {
                let a = self.try_resolve_bit_ref(left, scope_hint)?;
                let b = self.try_resolve_bit_ref(right, scope_hint)?;
                return Some(FusedGate::Bin2 {
                    dst,
                    a,
                    b,
                    op: gop,
                    invert: false,
                });
            }
        }
        // 4) Simple buf: leaf
        if let Some(src) = self.try_resolve_bit_ref(r, scope_hint) {
            return Some(FusedGate::Buf1 {
                dst,
                src,
                invert: false,
            });
        }
        None
    }

    pub(super) fn extract_sensitivity(
        &self,
        stmt: &Statement,
    ) -> Option<(Vec<Sensitivity>, Statement)> {
        match &stmt.kind {
            StatementKind::TimingControl {
                control,
                stmt: body,
            } => {
                if let TimingControl::Event(event) = control {
                    let sens = if matches!(event, EventControl::Star | EventControl::ParenStar) {
                        self.star_sens_from_body(body)
                    } else {
                        self.event_to_sens(event)
                    };
                    return Some((sens, *body.clone()));
                }
                None
            }
            StatementKind::SeqBlock { stmts, name } => {
                if let Some(first) = stmts.first() {
                    if let StatementKind::TimingControl {
                        control,
                        stmt: body,
                    } = &first.kind
                    {
                        if let TimingControl::Event(event) = control {
                            let sens =
                                if matches!(event, EventControl::Star | EventControl::ParenStar) {
                                    // §9.4.2.2: infer from everything the block
                                    // guards, not just the first statement.
                                    let rest = Statement::new(
                                        StatementKind::SeqBlock {
                                            name: name.clone(),
                                            stmts: {
                                                let mut v = vec![*body.clone()];
                                                v.extend_from_slice(&stmts[1..]);
                                                v
                                            },
                                        },
                                        stmt.span,
                                    );
                                    self.star_sens_from_body(&rest)
                                } else {
                                    self.event_to_sens(event)
                                };
                            let mut new_stmts = vec![*body.clone()];
                            new_stmts.extend_from_slice(&stmts[1..]);
                            return Some((
                                sens,
                                Statement::new(
                                    StatementKind::SeqBlock {
                                        name: name.clone(),
                                        stmts: new_stmts,
                                    },
                                    stmt.span,
                                ),
                            ));
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// `collect_stmt_reads`, also returning the names `stmt` reads as a whole
    /// value (for `self_written_read`).
    pub(super) fn collect_stmt_reads_whole(
        stmt: &Statement,
        module: &ElaboratedModule,
        reads: &mut HashSet<String>,
        writes: &mut HashSet<String>,
    ) -> HashSet<String> {
        let outer = WHOLE_READS.with(|c| c.replace(Some(HashSet::default())));
        Self::collect_stmt_reads(stmt, module, reads, writes);
        WHOLE_READS.with(|c| c.replace(outer)).unwrap_or_default()
    }

    /// §9.2.2.2.1(b): is read `r` excluded from a block's implicit sensitivity
    /// because the block writes it? Exact; a written '.'-prefix (`out` written,
    /// `out.vld` read); or a bare base that is in `reads` only as the prefix of
    /// member reads the block itself writes — `collect_expr_reads` records
    /// `decoded.op` AND `decoded`, and with `decoded.op` written the surviving
    /// `decoded` re-armed the block on its own two-step member write
    /// (`decoded.op = 0; decoded.op = x;`) every settle pass, to the limit
    /// (CVA6's decoder, which reads back the `op` it just wrote). A base with
    /// any member read the block does NOT write stays: `s.a` written, `s.b`
    /// read — `s` is the only signal that carries `s.b`.
    pub(super) fn self_written_read(
        r: &str,
        reads: &HashSet<String>,
        writes: &HashSet<String>,
        whole: &HashSet<String>,
    ) -> bool {
        fn direct(r: &str, writes: &HashSet<String>) -> bool {
            if writes.contains(r) {
                return true;
            }
            let mut base: &str = r;
            while let Some(pos) = base.rfind('.') {
                base = &base[..pos];
                if writes.contains(base) {
                    return true;
                }
            }
            false
        }
        if direct(r, writes) {
            return true;
        }
        // A base the block also reads as a whole (`out = s` beside
        // `s.a = ...`) stays: every member it does not write reaches `out`
        // through it.
        if whole.contains(r) {
            return false;
        }
        let prefix = format!("{r}.");
        if !writes.iter().any(|w| w.starts_with(&prefix)) {
            return false;
        }
        let mut member_reads = reads.iter().filter(|x| x.starts_with(&prefix));
        let Some(first) = member_reads.next() else {
            return false;
        };
        direct(first, writes) && member_reads.all(|m| direct(m, writes))
    }

    /// §9.4.2.2 — the inferred sensitivity of `@*` / `@(*)`: every signal READ
    /// by the statement it guards, minus the ones that statement writes (a
    /// variable assigned by the block is not part of its own sensitivity).
    ///
    /// `event_to_sens` yields an EMPTY list for Star, and the process path
    /// then took the "just execute body" fallthrough — so an `always @(*)`
    /// never suspended and behaved like `forever`. Harmless for a pure
    /// combinational body (it re-runs and settles), but with any timing
    /// control in the body (`always @(*) begin #0; ... end`) it became an
    /// infinite delta-cycle loop that never let time advance.
    pub(super) fn star_sens_from_body(&self, body: &Statement) -> Vec<Sensitivity> {
        let mut reads: HashSet<String> = HashSet::default();
        let mut writes: HashSet<String> = HashSet::default();
        // §9.2.2.2: `@*` sensitivity excludes called-function CONTENTS.
        SKIP_FN_BODY_READS.with(|c| c.set(true));
        let whole = Self::collect_stmt_reads_whole(body, &self.module, &mut reads, &mut writes);
        SKIP_FN_BODY_READS.with(|c| c.set(false));
        // Registration usually runs with the hint unset (the process body
        // just executed); the executing instance's own scope is the correct
        // resolution context either way.
        let hint = self.name_resolve_hint.borrow().clone().or_else(|| {
            let sc = self.active_instance_scope();
            (!sc.is_empty()).then_some(sc)
        });
        let mut out: Vec<Sensitivity> = Vec::new();
        let mut seen: HashSet<String> = HashSet::default();
        for r in reads.iter() {
            // §9.2.2.2.1: exclude reads of variables this block writes —
            // at MEMBER granularity too (same rule as the comb-entry path),
            // or the base fallback below would re-introduce the self-dep.
            if Self::self_written_read(r, &reads, &writes, &whole) {
                continue;
            }
            // Prefer the instance-scoped name when one exists, mirroring the
            // scope-first resolution the rest of the runtime performs. A
            // MEMBER read ("req.vld") that resolves to no signal falls back
            // to its '.'-base prefixes — without this, a block whose only
            // reads of a struct are member-granular had no sensitivity to it.
            let mut cand: Vec<String> = vec![r.clone()];
            {
                let mut base: &str = r.as_str();
                while let Some(pos) = base.rfind('.') {
                    base = &base[..pos];
                    cand.push(base.to_string());
                }
            }
            let mut matched = false;
            for c in cand {
                let resolved = hint
                    .as_ref()
                    .map(|sc| format!("{}.{}", sc, c))
                    .filter(|q| self.signal_name_to_id.contains_key(q.as_str()))
                    .unwrap_or_else(|| c.clone());
                if self.signal_name_to_id.contains_key(resolved.as_str()) {
                    if seen.insert(resolved.clone()) {
                        out.push(Sensitivity {
                            signal_name: resolved,
                            edge: EdgeKind::AnyEdge,
                            iff: None,
                            value_of: None,
                        });
                    }
                    matched = true;
                    break;
                }
            }
            // §23.8 / §23.10.1: a dotted read neither scope-local nor global
            // may be an UPWARD reference (a bound module reading its host
            // through the host's module name, e.g. `inner_mod.clk` from a
            // monitor bound under an `inner_mod` instance). Run the full
            // hierarchical resolution so the block re-fires on host-signal
            // changes instead of executing exactly once at time 0.
            if !matched && r.contains('.') {
                let span = crate::ast::Span::dummy();
                let ident = crate::ast::expr::HierarchicalIdentifier {
                    root: None,
                    path: vec![crate::ast::expr::HierPathSegment {
                        name: crate::ast::Identifier {
                            name: r.clone(),
                            span,
                        },
                        selects: Vec::new(),
                    }],
                    span,
                    cached_signal_id: std::cell::Cell::new(None),
                    cached_resolved_name: std::cell::OnceCell::new(),
                };
                let saved = self.name_resolve_hint.borrow().clone();
                *self.name_resolve_hint.borrow_mut() = hint.clone();
                let resolved = self.resolve_hier_name(&ident);
                *self.name_resolve_hint.borrow_mut() = saved;
                if resolved != *r
                    && self.signal_name_to_id.contains_key(resolved.as_ref())
                    && seen.insert(resolved.to_string())
                {
                    out.push(Sensitivity {
                        signal_name: resolved.to_string(),
                        edge: EdgeKind::AnyEdge,
                        iff: None,
                        value_of: None,
                    });
                }
            }
        }
        out
    }

    /// §15.5.3: does `head` name an EVENT for the purposes of
    /// `@(<head>.triggered)` / `wait(<head>.triggered)`?
    ///
    /// `module.events` only holds events declared at MODULE scope — an event
    /// inside an INTERFACE (the standard UVM `@(vif.evt.triggered)` sync
    /// pattern) is absent from it, which is why keying solely on that set left
    /// hierarchical waits with an empty sensitivity that fired at t=0. Every
    /// event is backed by a 1-bit toggle signal, so an existing signal under
    /// the head name is the reliable evidence. `full` (the `<head>.triggered`
    /// name) guards the one ambiguous case: a real struct member literally
    /// called `triggered`, which resolves as a signal in its own right.
    pub(super) fn hier_event_key(&self, head: &str, leaf: &str, full: &str) -> Option<String> {
        if let Some(key) = self.package_event_key(head) {
            return Some(key);
        }
        if self.module.events.contains(head) {
            return Some(head.to_string());
        }
        if self.module.events.contains(leaf) {
            return Some(leaf.to_string());
        }
        if self.signal_name_to_id.contains_key(full) {
            return None; // a genuine member named `triggered`
        }
        let resolved = self.resolve_event_key(head);
        if self.signal_name_to_id.contains_key(resolved.as_str())
            || self.signal_name_to_id.contains_key(head)
        {
            return Some(head.to_string());
        }
        None
    }

    /// Does event expression `e` select (an index or a part-select) anywhere
    /// outside a call?
    pub(super) fn event_expr_selects(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Index { .. } | ExprKind::RangeSelect { .. } => true,
            ExprKind::Paren(x) | ExprKind::Unary { operand: x, .. } => Self::event_expr_selects(x),
            ExprKind::Binary { left, right, .. } => {
                Self::event_expr_selects(left) || Self::event_expr_selects(right)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::event_expr_selects(condition)
                    || Self::event_expr_selects(then_expr)
                    || Self::event_expr_selects(else_expr)
            }
            ExprKind::Concatenation(parts) => parts.iter().any(Self::event_expr_selects),
            _ => false,
        }
    }

    /// Every identifier a select expression reads, index and bound
    /// expressions included.
    pub(super) fn collect_select_operands<'a>(
        e: &'a Expression,
        out: &mut Vec<&'a HierarchicalIdentifier>,
    ) {
        match &e.kind {
            ExprKind::Ident(h) => out.push(h),
            ExprKind::Paren(x) | ExprKind::Unary { operand: x, .. } => {
                Self::collect_select_operands(x, out)
            }
            ExprKind::Index { expr, index } => {
                Self::collect_select_operands(expr, out);
                Self::collect_select_operands(index, out);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                Self::collect_select_operands(expr, out);
                Self::collect_select_operands(left, out);
                Self::collect_select_operands(right, out);
            }
            ExprKind::Binary { left, right, .. } => {
                Self::collect_select_operands(left, out);
                Self::collect_select_operands(right, out);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::collect_select_operands(condition, out);
                Self::collect_select_operands(then_expr, out);
                Self::collect_select_operands(else_expr, out);
            }
            ExprKind::Concatenation(parts) => {
                for p in parts {
                    Self::collect_select_operands(p, out);
                }
            }
            _ => {}
        }
    }

    /// The value of an elaboration-time constant select index in an event
    /// term: literals and parameters under `+ - *` (`v[W-1]`). None for
    /// anything that can change at run time.
    pub(super) fn event_const_index(&self, e: &Expression, scope: &str) -> Option<i64> {
        match &e.kind {
            ExprKind::Paren(x) => self.event_const_index(x, scope),
            ExprKind::Number(_) => Self::try_const_u64(e).and_then(|v| i64::try_from(v).ok()),
            ExprKind::Ident(h) => self
                .event_param_value(h, scope)
                .and_then(|v| if v.has_xz() { None } else { v.to_u64() })
                .and_then(|v| i64::try_from(v).ok()),
            ExprKind::Binary { op, left, right } => {
                let l = self.event_const_index(left, scope)?;
                let r = self.event_const_index(right, scope)?;
                match op {
                    BinaryOp::Add => l.checked_add(r),
                    BinaryOp::Sub => l.checked_sub(r),
                    BinaryOp::Mul => l.checked_mul(r),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The value of parameter `h` as an event expression names it (bare
    /// under the block's scope, or already qualified).
    pub(super) fn event_param_value(
        &self,
        h: &HierarchicalIdentifier,
        scope: &str,
    ) -> Option<&Value> {
        if h.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let raw = h
            .path
            .iter()
            .map(|s| s.name.name.as_str())
            .collect::<Vec<_>>()
            .join(".");
        if !scope.is_empty() {
            if let Some(v) = self.module.parameters.get(&format!("{}.{}", scope, raw)) {
                return Some(v);
            }
        }
        self.module.parameters.get(&raw).or_else(|| {
            self.module
                .parameters
                .get(self.resolve_hier_name(h).as_ref())
        })
    }

    /// §9.4.2: must an always block with this event list run as a process
    /// (a waiter that evaluates the event expressions) rather than as an
    /// edge block? An edge block watches whole signals; a level term on
    /// constant bits of a plain vector (`@(v[3])`, `@(v[7:4])`) is narrowed
    /// to those bits (`narrow_event_mask`), and an edge on one bit
    /// (`@(posedge v[3])`) was given an alias net before classification. Any
    /// other select — a runtime index (`@(mem[i])`, `@(posedge v[i])`), a
    /// part or bit of an array element (`@(mem[3][1])`), an edge left on a
    /// select, an element of a memory stored without element names
    /// (`@(big[5])`: no signal slot, no edge snapshot) — has nothing an edge
    /// block can watch.
    pub(super) fn event_needs_value_waiter(&self, sens: &[Sensitivity], scope: &str) -> bool {
        sens.iter().any(|s| {
            let name = s.signal_name.as_str();
            if let Some(sid) = self.sens_term_sid(name, scope) {
                let Some(e) = &s.value_of else {
                    return false;
                };
                if !Self::event_expr_selects(e) {
                    return false;
                }
                return s.edge != EdgeKind::AnyEdge
                    || self.narrow_event_mask(e, sid, scope).is_none();
            }
            self.sens_unnamed_cell(s) || (s.value_of.is_some() && self.unpacked_dims_of(name) > 0)
        })
    }

    /// The signal an always block's sensitivity term names: as spelled,
    /// without the top module's prefix, or under the block's instance scope
    /// (an inlined child whose rewrite left the leaf bare) — the order the
    /// edge-block registration resolves terms in.
    pub(super) fn sens_term_sid(&self, name: &str, scope: &str) -> Option<usize> {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            return Some(id);
        }
        let top_prefix = format!("{}.", self.module.name);
        if let Some(&id) = name
            .strip_prefix(top_prefix.as_str())
            .and_then(|b| self.signal_name_to_id.get(b))
        {
            return Some(id);
        }
        if scope.is_empty() {
            return None;
        }
        self.signal_name_to_id
            .get(format!("{}.{}", scope, name).as_str())
            .copied()
    }

    /// Does event term `s` watch a memory cell that has no per-element name —
    /// a packed-arena cell or an unnamed bulk array's (`@(big[5])`, or the
    /// `cell` placeholder of `@(big[i])`)? No signal-id dependency or edge
    /// snapshot covers such a cell; only a waiter polling it can see it move.
    pub(super) fn sens_unnamed_cell(&self, s: &Sensitivity) -> bool {
        let name = s.signal_name.as_str();
        if self.signal_name_to_id.contains_key(name) {
            return false;
        }
        if resolve_array_elem_id(name, &self.array_first_id).is_some() {
            return true;
        }
        s.value_of.is_some() && self.array_has_unnamed_cells(name)
    }

    /// Are the cells of the 1-D array `base` stored without names (in the
    /// packed arena, or as an unnamed bulk array)?
    pub(super) fn array_has_unnamed_cells(&self, base: &str) -> bool {
        let Some(&(first, lo, _)) = self.array_first_id.get(base) else {
            return false;
        };
        is_packed_id(first)
            || !self
                .signal_name_to_id
                .contains_key(format!("{}[{}]", base, lo).as_str())
    }

    pub(super) fn event_to_sens(&self, event: &EventControl) -> Vec<Sensitivity> {
        // §13.5.2: inside a task, an event control on a `ref` formal waits
        // on the ACTUAL (see `ref_event_control`).
        if self.ref_redirect_possible() {
            if let Some(ev) = self.ref_event_control(event) {
                return self.event_to_sens(&ev);
            }
        }
        // Walk past Paren / RangeSelect / BitSelect / Concatenation wrappers
        // to find the underlying Ident(s). For E902 etc. that use
        // `always @(sig[5:0] or other)` the sensitivity expression is a
        // RangeSelect of an Ident; the previous code only matched bare
        // Ident and silently dropped the rest, leaving the always-block
        // never re-triggered when `sig` changed (the E902 hello-test
        // root-cause stall in cr_ifu_ibuf's pop0_shift_vld decoder).
        fn collect_ident_names<'a>(e: &'a Expression, out: &mut Vec<&'a HierarchicalIdentifier>) {
            match &e.kind {
                ExprKind::Ident(h) => out.push(h),
                ExprKind::Paren(inner) => collect_ident_names(inner, out),
                ExprKind::RangeSelect { expr, .. } => collect_ident_names(expr, out),
                ExprKind::Index { expr, .. } => collect_ident_names(expr, out),
                ExprKind::Concatenation(parts) => {
                    for p in parts {
                        collect_ident_names(p, out);
                    }
                }
                // §9.4.2: an ARBITRARY expression is a legal event control —
                // `always @(a + 1)` must fire on every change of `a`. These
                // shapes collected NO identifiers, so the block armed on an
                // empty sensitivity and fired once at t=0, then froze.
                // Sensitizing to every operand signal over-approximates
                // "expression value changed" the same way `@(*)` does.
                ExprKind::Binary { left, right, .. } => {
                    collect_ident_names(left, out);
                    collect_ident_names(right, out);
                }
                ExprKind::Unary { operand, .. } => collect_ident_names(operand, out),
                ExprKind::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                } => {
                    collect_ident_names(condition, out);
                    collect_ident_names(then_expr, out);
                    collect_ident_names(else_expr, out);
                }
                _ => {}
            }
        }
        match event {
            EventControl::EventExpr(exprs) => {
                let mut out: Vec<Sensitivity> = Vec::with_capacity(exprs.len());
                for ee in exprs {
                    let edge = match ee.edge {
                        Some(Edge::Posedge) => EdgeKind::Posedge,
                        Some(Edge::Negedge) => EdgeKind::Negedge,
                        Some(Edge::Edge) => EdgeKind::LsbEdge,
                        None => EdgeKind::AnyEdge,
                    };
                    if let Some(key) = self.event_ref_key_static(&ee.expr) {
                        let canon = self.resolve_event_key(&key);
                        out.push(Sensitivity {
                            signal_name: canon,
                            edge,
                            iff: ee.iff.clone().map(Box::new),
                            value_of: None,
                        });
                        continue;
                    }
                    // §25.8/§25.9: a term whose receiver is a bound virtual
                    // interface (`@(cfg.vif.data)`, `@(posedge d.vif.clk)`,
                    // `@(c.vif.cb)`) waits on the bound instance's signal.
                    let ee_vif;
                    let ee = match self.vif_rebase_expr(&ee.expr) {
                        Some(expr) => {
                            ee_vif = crate::ast::stmt::EventExpr {
                                edge: ee.edge,
                                expr,
                                iff: ee.iff.clone(),
                                span: ee.span,
                            };
                            &ee_vif
                        }
                        None => ee,
                    };
                    // §14.13: normalize a plain dotted chain (`@(vif.cb)` in a
                    // subroutine body) to the flat Ident every arm below
                    // expects — the chain form collected no identifiers and
                    // armed the wait on nothing.
                    let ee_norm;
                    let ee = match Self::member_chain_as_flat_ident_for_sens(&ee.expr) {
                        Some(flat) => {
                            ee_norm = crate::ast::stmt::EventExpr {
                                edge: ee.edge,
                                expr: flat,
                                iff: ee.iff.clone(),
                                span: ee.span,
                            };
                            &ee_norm
                        }
                        None => ee,
                    };
                    let edge = match ee.edge {
                        Some(Edge::Posedge) => EdgeKind::Posedge,
                        Some(Edge::Negedge) => EdgeKind::Negedge,
                        Some(Edge::Edge) => EdgeKind::LsbEdge,
                        None => EdgeKind::AnyEdge,
                    };
                    // §15.5 an event-ARRAY element (`always @(ev[1])`).
                    // `collect_ident_names` walks past the Index to the base,
                    // which for an event array is the array name and not a
                    // 1-bit signal — so the block armed on nothing and never
                    // fired. Each element has its own backing signal, so name
                    // it directly. Only a literal index is handled here
                    // (`event_to_sens` cannot evaluate); a variable index
                    // keeps the old behaviour.
                    if let ExprKind::Index { expr: base, index } = &ee.expr.kind {
                        if let (
                            ExprKind::Ident(bh),
                            ExprKind::Number(NumberLiteral::Integer { value, .. }),
                        ) = (&base.kind, &index.kind)
                        {
                            if bh.path.len() == 1 {
                                let key = format!("{}[{}]", bh.path[0].name.name, value);
                                if self.module.events.contains(key.as_str()) {
                                    out.push(Sensitivity {
                                        signal_name: self.resolve_event_key(&key),
                                        edge,
                                        iff: ee.iff.clone().map(Box::new),
                                        value_of: None,
                                    });
                                    continue;
                                }
                            }
                            // §9.4.2: `@(arr[1])` on an UNPACKED array — each
                            // element is its own signal, and walking past the
                            // Index to the base name armed the wait on a
                            // signal that doesn't exist (never fired). Only a
                            // literal index resolves here; a variable index
                            // keeps the old base-name behavior.
                            let bn = self.resolve_hier_name(bh);
                            if self.module.arrays.contains_key(&*bn) {
                                out.push(Sensitivity {
                                    signal_name: format!("{}[{}]", bn, value.replace('_', "")),
                                    edge,
                                    iff: ee.iff.clone().map(Box::new),
                                    value_of: None,
                                });
                                continue;
                            }
                        }
                    }
                    // §9.4.2: an event expression that SELECTS — a bit or part
                    // of a vector (`@(posedge v[3])`), an unpacked-array
                    // element through a runtime index (`@(mem[i])`), a bit of
                    // an element (`@(negedge mem[3][1])`) — is an event on the
                    // VALUE of that expression. Its terms are every operand it
                    // reads, the indices included (a change of `i` re-selects),
                    // plus a `cell` placeholder per unpacked array it selects
                    // from: an element has no name of its own here, so the
                    // waiter looks the cell up when it arms (see
                    // `resolve_cell_terms`). Every term then carries the
                    // expression (`value_of`), and the edge applies to the
                    // expression's value, not to a whole operand.
                    // Not inside a class method: there the operands are
                    // reached through handles (`vif.data[0]`), which only
                    // the waiting process can evaluate.
                    let selects = Self::event_expr_selects(&ee.expr)
                        && self.this_stack.last().copied().flatten().is_none();
                    let mut idents = Vec::new();
                    if selects {
                        Self::collect_select_operands(&ee.expr, &mut idents);
                    } else {
                        collect_ident_names(&ee.expr, &mut idents);
                    }
                    let term_start = out.len();
                    for &h in &idents {
                        if selects {
                            // A parameter in an index (`v[W-1]`) never changes.
                            if self.event_param_value(h, "").is_some() {
                                continue;
                            }
                            let bn = self.resolve_hier_name(h);
                            if self.unpacked_dims_of(&bn) > 0 {
                                out.push(Sensitivity {
                                    signal_name: bn.into_owned(),
                                    edge,
                                    iff: ee.iff.clone().map(Box::new),
                                    value_of: Some(Box::new(ee.expr.clone())),
                                });
                                continue;
                            }
                        }
                        // LRM §14.3: `@(cb)` naming a clocking block means the
                        // block's clock event (`@(posedge clk)`), not a signal
                        // literally called `cb`. Without this substitution the
                        // sensitivity targeted a nonexistent signal and never
                        // fired, so `@(cb)` returned at t=0 (a no-op). For an
                        // interface-scoped block `@(iface.cb)` the clocking key
                        // is the RAW dotted path (`iface.cb`); `resolve_hier_name`
                        // strips the instance prefix to `cb`, so try the raw path
                        // first, then the resolved name.
                        let segs: Vec<&str> = h.path.iter().map(|s| s.name.name.as_str()).collect();
                        // §14.13 `@(cb.sig)` — sensitize to the hidden sample
                        // mirror so the event lands at the clocking edge and
                        // only when the sample actually changed.
                        if segs.len() >= 2 {
                            if let Some(cbk) = self.resolve_clocking_key(&segs[..segs.len() - 1]) {
                                let hidden = Self::clocking_mirror_name(
                                    &self.module,
                                    &cbk,
                                    segs[segs.len() - 1],
                                );
                                if self.signal_name_to_id.contains_key(hidden.as_str()) {
                                    out.push(Sensitivity {
                                        signal_name: hidden,
                                        edge: EdgeKind::AnyEdge,
                                        iff: ee.iff.clone().map(Box::new),
                                        value_of: None,
                                    });
                                    continue;
                                }
                            }
                        }
                        // §15.5.3: `@(ev.triggered)` blocks until the event
                        // fires — the wakeable signal is the EVENT, not a
                        // "triggered" leaf. Without the strip the sensitivity
                        // named nonexistent "ev.triggered", fell into the
                        // no-signal delta-yield fallback and woke at t=0
                        // (issue #68).
                        if segs.len() >= 2 && *segs.last().unwrap() == "triggered" {
                            let head = segs[..segs.len() - 1].join(".");
                            let leaf = segs[segs.len() - 2];
                            let full = segs.join(".");
                            if let Some(key) = self.hier_event_key(&head, leaf, &full) {
                                out.push(Sensitivity {
                                    signal_name: self.resolve_event_key(&key),
                                    edge,
                                    iff: ee.iff.clone().map(Box::new),
                                    value_of: None,
                                });
                                continue;
                            }
                        }
                        let mut sig = self.resolve_hier_name(h);
                        // `@(bus.clk_i)` / `@(this.drv.bus.clk_i)` inside a
                        // class: `resolve_hier_name` strips the handle prefix
                        // to the LEAF, which is not a signal — late-bind the
                        // raw chain through the live handles instead (the
                        // snitch driver stall: every wait degenerated to a
                        // same-time yield and the run never left time 0).
                        if !self.signal_name_to_id.contains_key(sig.as_ref())
                            && !self.signals.contains_key(&*sig)
                        {
                            let raw = segs.join(".");
                            if let Some(resolved) = self.resolve_sens_name_via_handles(&raw) {
                                sig = std::borrow::Cow::Owned(resolved);
                            }
                        }
                        let cb_key = self
                            .resolve_clocking_key(&segs)
                            .unwrap_or_else(|| sig.to_string());
                        if ee.edge.is_none() {
                            if let Some((clk, _)) = self.clocking_meta.get(&cb_key) {
                                out.push(Sensitivity {
                                    signal_name: clk.clone(),
                                    edge: self
                                        .clocking_edge
                                        .get(&cb_key)
                                        .copied()
                                        .unwrap_or(EdgeKind::Posedge),
                                    iff: ee.iff.clone().map(Box::new),
                                    value_of: None,
                                });
                                continue;
                            }
                        }
                        // §9.4.2/§7.2.1: `@(s.a)` on a PACKED STRUCT field —
                        // no standalone signal backs the field (it is a slice
                        // of the base vector), so a name-keyed sensitivity
                        // armed nothing and the waiter fell into the
                        // delta-yield (spurious t=0 wake). Arm the BASE and
                        // let the wake path compare the FIELD's value against
                        // the armed snapshot (the non-trivial-expression
                        // machinery).
                        if !self.signal_name_to_id.contains_key(sig.as_ref())
                            && !self.signals.contains_key(&*sig)
                        {
                            if let Some((base, _off, _w)) = self.packed_leaf_of_hier(&sig) {
                                out.push(Sensitivity {
                                    signal_name: base,
                                    edge,
                                    iff: ee.iff.clone().map(Box::new),
                                    value_of: Some(Box::new(ee.expr.clone())),
                                });
                                continue;
                            }
                        }
                        out.push(Sensitivity {
                            signal_name: sig.to_string(),
                            edge,
                            iff: ee.iff.clone().map(Box::new),
                            value_of: None,
                        });
                    }
                    // §9.4.2: a non-trivial event expression fires on its
                    // VALUE change, not on any operand write — attach the
                    // term expression so the wake path can compare against
                    // the armed value (guard_prev).
                    if selects
                        || matches!(
                            &ee.expr.kind,
                            ExprKind::Binary { .. }
                                | ExprKind::Unary { .. }
                                | ExprKind::Conditional { .. }
                        )
                    {
                        for sens in out.iter_mut().skip(term_start) {
                            sens.value_of = Some(Box::new(ee.expr.clone()));
                        }
                    }
                    if selects {
                        // `mem[i] ^ mem[i+1]` names `mem` and `i` twice.
                        let mut k = term_start;
                        while k < out.len() {
                            if out[term_start..k]
                                .iter()
                                .any(|o| o.signal_name == out[k].signal_name)
                            {
                                out.remove(k);
                            } else {
                                k += 1;
                            }
                        }
                    }
                    // P6: `@(posedge vif.clk)` — a virtual-interface member is a
                    // MemberAccess, which collect_ident_names doesn't gather.
                    // Resolve `vif.member` through the this-relative
                    // virtual-interface binding (`<bound>.member`) so the driver/
                    // monitor suspend on the real interface clock/reset instead
                    // of spinning on an empty sensitivity.
                    // §14.13 `@(cb.sig)` written as a MemberAccess (the shape
                    // the parser produces for a sensitivity list, and inside
                    // subroutines): sensitize to the hidden sample mirror so
                    // the event lands at the clocking edge, only on a change.
                    // Without this the sensitivity came out EMPTY and the
                    // always block fired once at t=0 and never again.
                    if idents.is_empty() && ee.edge.is_none() {
                        if let Some(segs) = Self::flatten_member_path(&ee.expr) {
                            // §15.5.3 `@(<hier>.<event>.triggered)` — the
                            // MemberAccess shape the parser produces for an
                            // event reached through a hierarchical path
                            // (`@(u_if.set_opt_reg.triggered)`). The flattened
                            // Ident shape is handled in the ident loop above;
                            // this form never reached it (collect_ident_names
                            // does not walk MemberAccess), so the sensitivity
                            // came out empty and the wait returned at t=0.
                            if segs.len() >= 2 && segs[segs.len() - 1] == "triggered" {
                                let ev = segs[..segs.len() - 1].join(".");
                                let leaf = segs[segs.len() - 2].clone();
                                let full = segs.join(".");
                                if let Some(k) = self.hier_event_key(&ev, &leaf, &full) {
                                    out.push(Sensitivity {
                                        signal_name: self.resolve_event_key(&k),
                                        edge: EdgeKind::AnyEdge,
                                        iff: ee.iff.clone().map(Box::new),
                                        value_of: None,
                                    });
                                    continue;
                                }
                            }
                            if segs.len() >= 2 {
                                let refs: Vec<&str> = segs.iter().map(|x| x.as_str()).collect();
                                if let Some(cbk) =
                                    self.resolve_clocking_key(&refs[..refs.len() - 1])
                                {
                                    let hidden = Self::clocking_mirror_name(
                                        &self.module,
                                        &cbk,
                                        refs[refs.len() - 1],
                                    );
                                    if self.signal_name_to_id.contains_key(hidden.as_str()) {
                                        out.push(Sensitivity {
                                            signal_name: hidden,
                                            edge: EdgeKind::AnyEdge,
                                            iff: ee.iff.clone().map(Box::new),
                                            value_of: None,
                                        });
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                    if idents.is_empty() {
                        if let ExprKind::MemberAccess { expr: base, member } = &ee.expr.kind {
                            if let ExprKind::Ident(bh) = &base.kind {
                                if bh.path.len() == 1 {
                                    let vifname = bh.path[0].name.name.clone();
                                    let bound =
                                        self.this_stack.last().copied().flatten().and_then(|h| {
                                            self.virtual_iface_bindings
                                                .get(&(h, vifname.clone()))
                                                .map(|(b, _)| b.clone())
                                        });
                                    if let Some(b) = bound {
                                        out.push(Sensitivity {
                                            signal_name: format!("{}.{}", b, member.name),
                                            edge,
                                            iff: ee.iff.clone().map(Box::new),
                                            value_of: None,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                out
            }
            EventControl::Identifier(id) => {
                // LRM §14.3: `@(cb)` where `cb` names a clocking block
                // synchronizes to that block's clock event (its declared
                // `@(posedge clk)`), NOT a signal literally called `cb`. The
                // old code built a sensitivity on a nonexistent signal `cb`,
                // which never fired — so `@(cb)` returned at t=0 (a no-op) and
                // any `@(cb); ...` sampling loop in a direct-SV testbench spun
                // instead of stepping the clock. Substitute the block's clock
                // signal with a posedge edge (the edge `tick_clocking_blocks`
                // already assumes). Falls through to a plain named event when
                // `id` is an ordinary `event`/signal, preserving prior behavior.
                if let Some((clk, _)) = self
                    .resolve_clocking_key(&[id.name.as_str()])
                    .and_then(|k| self.clocking_meta.get(&k))
                {
                    vec![Sensitivity {
                        signal_name: clk.clone(),
                        edge: EdgeKind::Posedge,
                        iff: None,
                        value_of: None,
                    }]
                } else {
                    vec![Sensitivity {
                        signal_name: {
                            let key = self.lexical_event_name(&id.name);
                            if self.module.events.contains(&key) {
                                key
                            } else {
                                id.name.clone()
                            }
                        },
                        edge: EdgeKind::AnyEdge,
                        iff: None,
                        value_of: None,
                    }]
                }
            }
            EventControl::HierIdentifier(expr) => {
                if let ExprKind::Ident(h) = &expr.kind {
                    // LRM §14.3: the BARE `@cb` form parses as a
                    // HierIdentifier (the parenthesized `@(cb)` becomes a
                    // one-term EventExpr, which already resolves clocking
                    // blocks). Without the same substitution here, `@cb` built
                    // a sensitivity on a nonexistent signal named `cb`, found
                    // no signal id, and fell into the delta-yield — returning
                    // at t=0 instead of waiting for the block's clock edge.
                    // Every `@cb;` sampling loop then spun without advancing.
                    let segs: Vec<&str> = h.path.iter().map(|s| s.name.name.as_str()).collect();
                    if let Some(cbk) = self.resolve_clocking_key(&segs) {
                        if let Some((clk, _)) = self.clocking_meta.get(&cbk) {
                            return vec![Sensitivity {
                                signal_name: clk.clone(),
                                edge: EdgeKind::Posedge,
                                iff: None,
                                value_of: None,
                            }];
                        }
                    }
                    vec![Sensitivity {
                        signal_name: self.resolve_hier_name(h).into_owned(),
                        edge: EdgeKind::AnyEdge,
                        iff: None,
                        value_of: None,
                    }]
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    /// Create an EventWaiter with pre-resolved sensitivity IDs for O(1) edge checking.
    /// Render a signal's current value compactly for hang diagnostics.
    pub(super) fn hang_sig_value(&self, id: usize) -> String {
        let w = self.signal_widths.get(id).copied().unwrap_or(1);
        if w == 1 {
            ['0', '1', 'x', 'z'][self.signal_table[id].get_bit_code(0) as usize % 4].to_string()
        } else if self.signal_table[id].has_unknown() {
            format!("{}'(has x/z)", w)
        } else {
            format!("{}'h{:x}", w, self.signal_table[id].to_u64().unwrap_or(0))
        }
    }

    /// Walk a signal's driver cone backwards and append one line per hop:
    /// which block/assign drives it, and what THAT reads (with current
    /// values). Turns "resp_vld never moved" into "resp_vld <= flop clocked
    /// by gclk (now 0) <- gclk = clk & gclk_en with gclk_en=0". Depth-capped
    /// and cycle-safe; a signal with NO driver is called out explicitly —
    /// that is the signature of a dropped instance output.
    pub(super) fn describe_driver_chain(
        &self,
        sig_id: usize,
        indent: usize,
        depth: usize,
        visited: &mut std::collections::HashSet<usize>,
    ) {
        if depth == 0 || !visited.insert(sig_id) {
            return;
        }
        let pad = " ".repeat(indent);
        // Free-running clock generator?
        if let Some(cg) = self.clock_generators.iter().find(|c| c.signal_id == sig_id) {
            eprintln!(
                "[xezim][hang-report]{}'{}' is a free-running clock (half-period {}) — alive",
                pad,
                self.name_for_id(sig_id),
                cg.half_period
            );
            return;
        }
        // Edge blocks (flops) writing it: clock/sensitivity is the lead.
        let mut found = false;
        for (bi, cb) in self.compiled_edge_blocks.iter().enumerate() {
            let Some(cb) = cb else { continue };
            use super::bytecode::Insn;
            let writes = cb.instructions.iter().any(|i| {
                matches!(i,
                    Insn::NbaAssign(id, ..) | Insn::NbaAssignConst(id, ..)
                    | Insn::NbaAssignRange(id, ..) | Insn::NbaAssignRangeDyn(id, ..)
                    | Insn::NbaAssignBitDyn(id, ..) | Insn::NbaAssignArrayRead(id, ..)
                    | Insn::BlockingAssign(id, ..) | Insn::BlockingAssignRange(id, ..)
                    | Insn::BlockingAssignRangeDyn(id, ..) | Insn::BlockingAssignBitDyn(id, ..)
                        if *id as usize == sig_id)
            });
            if !writes {
                continue;
            }
            found = true;
            let ident = self
                .edge_block_stall_identity(bi, 0)
                .unwrap_or_else(|| "                 edge-sensitive block".to_string());
            eprintln!(
                "[xezim][hang-report]{}'{}' (now {}) written by:",
                pad,
                self.name_for_id(sig_id),
                self.hang_sig_value(sig_id)
            );
            eprintln!("[xezim][hang-report]{}{}", pad, ident.trim_start());
            if let Some(block) = self.edge_blocks.get(bi) {
                for sid in block.resolved_sensitivities.iter().take(4) {
                    let edge = sid.edge.print_str();
                    eprintln!(
                        "[xezim][hang-report]{}  sensitive to {}{} (now {})",
                        pad,
                        edge,
                        self.name_for_id(sid.signal_id),
                        self.hang_sig_value(sid.signal_id)
                    );
                    self.describe_driver_chain(sid.signal_id, indent + 4, depth - 1, visited);
                }
            }
            break;
        }
        if found {
            return;
        }
        // Combinational entry writing it: reads are the fan-in.
        for e in &self.comb_entries {
            if !e.cold.write_signal_ids.contains(&sig_id) {
                continue;
            }
            found = true;
            let reads: Vec<String> = e
                .cold
                .read_signal_ids
                .iter()
                .take(6)
                .map(|&r| format!("{}={}", self.name_for_id(r), self.hang_sig_value(r)))
                .collect();
            eprintln!(
                "[xezim][hang-report]{}'{}' (now {}) driven combinationally from: {}{}",
                pad,
                self.name_for_id(sig_id),
                self.hang_sig_value(sig_id),
                reads.join(", "),
                if e.cold.read_signal_ids.len() > 6 {
                    ", ..."
                } else {
                    ""
                }
            );
            for &r in e.cold.read_signal_ids.iter().take(4) {
                self.describe_driver_chain(r, indent + 4, depth - 1, visited);
            }
            break;
        }
        if !found {
            // Only clock generators, edge blocks and comb entries were
            // searched — the three CONTINUOUS driver kinds. A variable written
            // from an `initial` block or a non-edge `always` (every testbench
            // reset `reg`) has none of those and lands here while being
            // perfectly healthy, and `module.initial_blocks` is consumed at
            // scheduling so there is nothing left to point at. Say what was
            // checked rather than asserting a dropped instance: naming that as
            // the cause sent readers hunting for a missing cell over a stable
            // `rst_b` that an initial block had correctly driven.
            eprintln!(
                "[xezim][hang-report]{}'{}' (now {}) has no continuous driver \
                 (no clock generator, edge block or continuous assign writes it) \
                 — normal for a variable driven from an initial/always block; \
                 on a NET this is the signature of a dropped/unresolved instance",
                pad,
                self.name_for_id(sig_id),
                self.hang_sig_value(sig_id)
            );
        }
    }

    /// Hang diagnosis: print the longest-parked event waiters — who is
    /// stuck, on which signals, since when, and whether those signals have
    /// EVER changed since the waiter parked (`arm_bits` comparison). A
    /// dead-clock/reset hang shows up as old waiters whose awaited signals
    /// are all "unchanged since parked". Printed automatically when the run
    /// ends by --max-time without $finish, and (abbreviated) with
    /// XEZIM_PROGRESS ticks.
    pub(super) fn report_parked_waiters(&mut self, max_n: usize) {
        if self.event_waiters.is_empty()
            && self.condition_waiters.is_empty()
            && self.nba_region_waiters.is_empty()
        {
            return;
        }
        let mut order: Vec<usize> = (0..self.event_waiters.len()).collect();
        order.sort_by_key(|&i| self.event_waiters[i].parked_time);
        eprintln!(
            "[xezim][hang-report] {} event waiter(s), {} condition waiter(s) parked at sim time {}; oldest first:",
            self.event_waiters.len(),
            self.condition_waiters.len(),
            self.time
        );
        for &i in order.iter().take(max_n) {
            let w = &self.event_waiters[i];
            let age = self.time.saturating_sub(w.parked_time);
            let mut sens_desc: Vec<String> = Vec::new();
            for (k, sid) in w.resolved_sensitivities.iter().enumerate() {
                let edge = sid.edge.print_str();
                let cur = self.term_raw_bits(sid.signal_id);
                let armed = w.arm_bits.get(k).copied().unwrap_or((0, 0));
                let moved = if cur == armed {
                    "UNCHANGED since parked"
                } else {
                    "has changed"
                };
                // LSB code: 0, 1, x (v=0), z (v=1) as `get_bit_code` spells it.
                let code = ((cur.1 & 1) << 1) | (cur.0 & 1);
                sens_desc.push(format!(
                    "{}{} (now {}, {})",
                    edge,
                    self.name_for_id(sid.signal_id),
                    ['0', '1', 'x', 'z'][code as usize % 4],
                    moved
                ));
            }
            // A compiled process FSM parks with an empty continuation; name
            // the body it belongs to instead.
            let loc = w
                .continuation
                .first()
                .and_then(|st| self.span_file_line_in(st.span, None))
                .or_else(|| {
                    let (span, _) = self.process_origin.get(&w.pid)?;
                    let at = self.span_file_line_in(*span, None)?;
                    Some(format!("a wait inside the process at {}", at))
                })
                .unwrap_or_else(|| "<unknown location>".to_string());
            let origin = self
                .process_origin
                .get(&w.pid)
                .map(|(_, kind)| *kind)
                .unwrap_or("process");
            eprintln!(
                "[xezim][hang-report]   pid {} ({}) waiting {} tick(s) (since t={}) on @({}){} — resumes at {}",
                w.pid,
                origin,
                age,
                w.parked_time,
                sens_desc.join(" or "),
                if w.remaining_events > 1 {
                    format!(" ({} qualifying events remaining)", w.remaining_events)
                } else {
                    String::new()
                },
                loc
            );
            // For signals that never moved, walk the driver cone to the root.
            let dead: Vec<usize> = w
                .resolved_sensitivities
                .iter()
                .zip(w.arm_bits.iter())
                .filter(|&(ref sid, &armed)| {
                    !is_packed_id(sid.signal_id) && self.term_raw_bits(sid.signal_id) == armed
                })
                .map(|(sid, _)| sid.signal_id)
                .collect();
            let mut visited = std::collections::HashSet::new();
            for d in dead {
                self.describe_driver_chain(d, 5, 4, &mut visited);
            }
        }
        for (pid, cont) in self.condition_waiters.iter().take(max_n) {
            let loc = cont
                .first()
                .and_then(|st| self.span_file_line_in(st.span, None))
                .unwrap_or_else(|| "<unknown location>".to_string());
            eprintln!(
                "[xezim][hang-report]   pid {} blocked in wait(condition) — resumes at {}",
                pid, loc
            );
        }
        // §15.5: a process parked on `@(procedural_var)` for a procedural
        // NBA-region variable (e.g. `uvm_wait_for_nba_region`'s `nba <= next_nba;
        // @(nba)`) lingers in `nba_region_waiters` until the Reactive region's
        // NBA commit — flush that lane before declaring a hang so a spin parked
        // there is not reported as an unexplained block.
        for (pid, cont) in self.nba_region_waiters.iter().take(max_n) {
            let loc = cont
                .first()
                .and_then(|st| self.span_file_line_in(st.span, None))
                .unwrap_or_else(|| "<unknown location>".to_string());
            eprintln!(
                "[xezim][hang-report]   pid {} parked on @(procedural-var) for the NBA region — resumes at {}",
                pid, loc
            );
        }
    }

    pub(super) fn make_event_waiter(
        &mut self,
        pid: usize,
        sens: Vec<Sensitivity>,
        continuation: ProcCont,
    ) -> EventWaiter {
        self.make_event_waiter_kind(pid, sens, continuation, false)
    }

    pub(super) fn make_event_waiter_kind(
        &mut self,
        pid: usize,
        sens: Vec<Sensitivity>,
        continuation: ProcCont,
        is_clocking: bool,
    ) -> EventWaiter {
        let resolved = self.resolve_sens_ids(&sens);
        self.make_event_waiter_resolved(pid, resolved, continuation, is_clocking)
    }

    pub(super) fn resolve_sens_ids(&self, sens: &[Sensitivity]) -> Vec<SensitivityId> {
        sens.iter()
            .filter_map(|s| {
                let name = s.signal_name.as_str();
                if let Some(&id) = self.signal_name_to_id.get(name) {
                    return Some(SensitivityId {
                        signal_id: id,
                        edge: s.edge,
                        iff: s.iff.clone(),
                        value_of: s.value_of.clone(),
                        wake_rank: self.wake_rank_of(name, id),
                        cell: false,
                    });
                }
                self.resolve_sens_cell(s)
            })
            .collect()
    }

    /// A waiter term naming no stored signal that is still an array cell
    /// (§9.4.2): an element of an array stored without per-element names
    /// (a large memory, `@(mem[5])`), or an array a select expression reads
    /// through a runtime index (`@(mem[i])`) — the latter a `cell`
    /// placeholder that `resolve_cell_terms` replaces when the waiter arms.
    #[cold]
    #[inline(never)]
    pub(super) fn resolve_sens_cell(&self, s: &Sensitivity) -> Option<SensitivityId> {
        let name = s.signal_name.as_str();
        let (id, cell) = match resolve_array_elem_id(name, &self.array_first_id) {
            Some(id) => (id, false),
            None if s.value_of.is_some() && self.unpacked_dims_of(name) > 0 => (
                self.array_first_id
                    .get(name)
                    .map_or(0, |&(first, _, _)| first),
                true,
            ),
            None => return None,
        };
        Some(SensitivityId {
            signal_id: id,
            edge: s.edge,
            iff: s.iff.clone(),
            value_of: s.value_of.clone(),
            wake_rank: 0,
            cell,
        })
    }

    /// Does waiter term `s` resolve to something a waiter can watch?
    pub(super) fn sens_term_resolves(&self, s: &Sensitivity) -> bool {
        self.signal_name_to_id.contains_key(s.signal_name.as_str())
            || self.resolve_sens_cell(s).is_some()
    }

    /// Scheduling depth of a waiter term: a port name collapsed onto its
    /// actual keeps its own depth (`name_wake_rank`), anything else takes
    /// its signal's.
    #[inline]
    pub(super) fn wake_rank_of(&self, name: &str, id: usize) -> u8 {
        if !self.name_wake_rank.is_empty() {
            if let Some(&r) = self.name_wake_rank.get(name) {
                return r;
            }
        }
        if self.sig_wake_rank.is_empty() {
            return 0;
        }
        self.sig_wake_rank.get(&id).copied().unwrap_or(0)
    }

    pub(super) fn make_event_waiter_resolved(
        &mut self,
        pid: usize,
        mut resolved: Vec<SensitivityId>,
        continuation: ProcCont,
        is_clocking: bool,
    ) -> EventWaiter {
        // §9.4.2 expression terms (`@(mem[i])`, `@(posedge v[3])`): remember
        // the scope they resolve in, and watch each unpacked-array cell they
        // select as its index stands now.
        let value_ctx = if resolved.iter().any(|s| s.value_of.is_some()) {
            if self.local_stack.last().is_some_and(|m| !m.is_empty()) {
                self.freeze_value_terms(&mut resolved);
            }
            let cells = resolved.iter().any(|s| {
                s.cell
                    || s.value_of
                        .as_ref()
                        .is_some_and(|e| self.expr_selects_array_cell(e))
            });
            if cells {
                self.resolve_cell_terms(&mut resolved);
            }
            Some(Box::new(WaiterValueCtx {
                scope: self.name_resolve_hint.borrow().clone(),
                cells,
            }))
        } else {
            None
        };
        // §9.4.2 value guard: capture each non-trivial term's value at arm.
        let guard_prev: Vec<Option<Value>> = resolved
            .iter()
            .map(|sid| sid.value_of.clone().map(|e| self.eval_expr(&e)))
            .collect();
        // Capture each sensitivity signal's value AT ARM TIME, parallel to
        // `resolved`. A waiter registered within the current snapshot
        // generation is checked against these (not the tick-start snapshot),
        // so only a change made AFTER it armed counts as an edge — see the
        // firing loop in `check_edges_inner`.
        let arm_bits: Vec<(u64, u64)> = resolved
            .iter()
            .map(|sid| self.term_raw_bits(sid.signal_id))
            .collect();
        // `sens` (Vec<Sensitivity>) is consumed for resolution and dropped;
        // EventWaiter only carries the resolved IDs from here on.
        //
        // Capture each sensitivity signal's current value at registration
        // so the waiter fires on a change relative to THIS point (see the
        // `captured_prev` field doc for the NBA reasoning).
        let captured_prev: Vec<(u64, u64)> = resolved
            .iter()
            .map(|s| self.term_raw_bits(s.signal_id))
            .collect();
        let captured_prev_wide: Vec<Option<Value>> = resolved
            .iter()
            .map(|s| self.term_wide_value(s.signal_id))
            .collect();
        EventWaiter {
            pid,
            parked_time: self.time,
            resolved_sensitivities: resolved,
            arm_bits,
            continuation,
            captured_prev,
            captured_prev_wide,
            guard_prev,
            remaining_events: 1,
            is_clocking,
            value_ctx,
        }
    }

    /// §9.4.2: the drain evaluates a waiter's expression terms outside the
    /// waiting process, where its subroutine frame (a task formal, an
    /// automatic loop variable: `@(mem[k])`) is not visible — or another
    /// process's frame is. Such a name cannot change while its process
    /// waits, so replace it by its value now.
    pub(super) fn freeze_value_terms(&mut self, terms: &mut [SensitivityId]) {
        let mut frozen: Vec<(crate::ast::Span, Option<Box<Expression>>)> = Vec::new();
        for t in terms.iter_mut() {
            let Some(span) = t.value_of.as_ref().map(|e| e.span) else {
                continue;
            };
            let f = match frozen.iter().find(|(sp, _)| *sp == span) {
                Some((_, f)) => f.clone(),
                None => {
                    let e = t.value_of.as_ref().unwrap();
                    let f = if self.expr_reads_frame_local(e) {
                        let mut c = e.clone();
                        self.freeze_frame_locals(&mut c);
                        Some(c)
                    } else {
                        None
                    };
                    frozen.push((span, f.clone()));
                    f
                }
            };
            if let Some(f) = f {
                t.value_of = Some(f);
            }
        }
    }

    pub(super) fn expr_reads_frame_local(&self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Ident(h) => self.frame_bound_bare_name(h).is_some(),
            ExprKind::Paren(x) | ExprKind::Unary { operand: x, .. } => {
                self.expr_reads_frame_local(x)
            }
            ExprKind::Index { expr, index } => {
                self.expr_reads_frame_local(expr) || self.expr_reads_frame_local(index)
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                self.expr_reads_frame_local(expr)
                    || self.expr_reads_frame_local(left)
                    || self.expr_reads_frame_local(right)
            }
            ExprKind::Binary { left, right, .. } => {
                self.expr_reads_frame_local(left) || self.expr_reads_frame_local(right)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.expr_reads_frame_local(condition)
                    || self.expr_reads_frame_local(then_expr)
                    || self.expr_reads_frame_local(else_expr)
            }
            ExprKind::Concatenation(parts) => parts.iter().any(|p| self.expr_reads_frame_local(p)),
            _ => false,
        }
    }

    pub(super) fn freeze_frame_locals(&mut self, e: &mut Expression) {
        if let ExprKind::Ident(h) = &e.kind {
            if self.frame_bound_bare_name(h).is_some() {
                let v = self.eval_expr(e);
                let lit = if v.is_real {
                    NumberLiteral::Real(v.to_f64())
                } else {
                    NumberLiteral::Integer {
                        size: Some(v.width.max(1)),
                        signed: v.is_signed,
                        base: NumberBase::Binary,
                        value: v.to_bin(),
                        cached_val: Cell::new(None),
                    }
                };
                *e = Expression::new(ExprKind::Number(lit), e.span);
            }
            return;
        }
        match &mut e.kind {
            ExprKind::Paren(x) | ExprKind::Unary { operand: x, .. } => self.freeze_frame_locals(x),
            ExprKind::Index { expr, index } => {
                self.freeze_frame_locals(expr);
                self.freeze_frame_locals(index);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                self.freeze_frame_locals(expr);
                self.freeze_frame_locals(left);
                self.freeze_frame_locals(right);
            }
            ExprKind::Binary { left, right, .. } => {
                self.freeze_frame_locals(left);
                self.freeze_frame_locals(right);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.freeze_frame_locals(condition);
                self.freeze_frame_locals(then_expr);
                self.freeze_frame_locals(else_expr);
            }
            ExprKind::Concatenation(parts) => {
                for p in parts {
                    self.freeze_frame_locals(p);
                }
            }
            _ => {}
        }
    }

    /// Current raw (v, x) of a waiter term's signal: a `signal_table` slot,
    /// or a packed-arena cell (a large memory's element, see `PackedMem`).
    #[inline]
    pub(super) fn term_raw_bits(&self, id: usize) -> (u64, u64) {
        if is_packed_id(id) {
            let (v, x, _) = self.packed.raw(id);
            (v, x)
        } else {
            self.signal_table[id].raw_bits()
        }
    }

    /// The full value of a waiter term wider than 64 bits (arena cells are
    /// at most 64 bits wide).
    #[inline]
    pub(super) fn term_wide_value(&self, id: usize) -> Option<Value> {
        if !is_packed_id(id) && self.signal_widths[id] > 64 {
            Some(self.signal_table[id].clone())
        } else {
            None
        }
    }

    /// Did waiter term `sid` see an edge since `(pv, px)` / `pw`? A term
    /// with an expression (`value_of`) only asks whether its operand moved
    /// at all: the edge itself is judged on the expression's value.
    #[inline]
    pub(super) fn term_fires(
        &self,
        sid: &SensitivityId,
        pv: u64,
        px: u64,
        pw: Option<&Value>,
    ) -> bool {
        let edge = if sid.value_of.is_some() {
            EdgeKind::AnyEdge
        } else {
            sid.edge
        };
        if is_packed_id(sid.signal_id) {
            let (cv, cx) = self.term_raw_bits(sid.signal_id);
            return lsb_edge_fires(edge, pv, px, cv, cx);
        }
        self.edge_fires_prev(sid.signal_id, edge, pv, px, pw)
    }

    /// Number of unpacked dimensions of the array named `bn` (0 when it is
    /// not a fixed-size unpacked array: a vector, a collection, unknown).
    pub(super) fn unpacked_dims_of(&self, bn: &str) -> usize {
        if self.module.queue_vars.contains(bn)
            || self.module.dynamic_arrays.contains(bn)
            || self.module.associative_arrays.contains_key(bn)
        {
            return 0;
        }
        if let Some((dims, _)) = self.module.arrays_nd.get(bn) {
            return dims.len();
        }
        if self.module.arrays_2d.contains_key(bn) {
            return 2;
        }
        if self.module.arrays.contains_key(bn) || self.array_first_id.contains_key(bn) {
            return 1;
        }
        0
    }

    /// `base[i0][i1]...` as (base, indices root-first), when `e` is a chain
    /// of index selects on a plain identifier.
    pub(super) fn index_chain(
        e: &Expression,
    ) -> Option<(&HierarchicalIdentifier, Vec<&Expression>)> {
        let mut idx: Vec<&Expression> = Vec::new();
        let mut cur = e;
        let h = loop {
            match &cur.kind {
                ExprKind::Index { expr, index } => {
                    idx.push(index);
                    cur = expr;
                }
                ExprKind::Ident(h) => break h,
                _ => return None,
            }
        };
        idx.reverse();
        Some((h, idx))
    }

    /// Does `e` select an element of an unpacked array (at any depth)?
    pub(super) fn expr_selects_array_cell(&self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Index { expr, index } => {
                if let Some((h, idx)) = Self::index_chain(e) {
                    let d = self.unpacked_dims_of(&self.resolve_hier_name(h));
                    if d > 0 && idx.len() >= d {
                        return true;
                    }
                }
                self.expr_selects_array_cell(expr) || self.expr_selects_array_cell(index)
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                self.expr_selects_array_cell(expr)
                    || self.expr_selects_array_cell(left)
                    || self.expr_selects_array_cell(right)
            }
            ExprKind::Paren(x) | ExprKind::Unary { operand: x, .. } => {
                self.expr_selects_array_cell(x)
            }
            ExprKind::Binary { left, right, .. } => {
                self.expr_selects_array_cell(left) || self.expr_selects_array_cell(right)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.expr_selects_array_cell(condition)
                    || self.expr_selects_array_cell(then_expr)
                    || self.expr_selects_array_cell(else_expr)
            }
            ExprKind::Concatenation(parts) => parts.iter().any(|p| self.expr_selects_array_cell(p)),
            _ => false,
        }
    }

    /// Push the id of every unpacked-array cell `e` selects, each index
    /// evaluated now. An x/z or out-of-range index selects no cell (§7.4.6).
    pub(super) fn collect_selected_cells(&mut self, e: &Expression, out: &mut Vec<usize>) {
        match &e.kind {
            ExprKind::Index { expr, index } => {
                if let Some((h, idx)) = Self::index_chain(e) {
                    let bn = self.resolve_hier_name(h).into_owned();
                    let d = self.unpacked_dims_of(&bn);
                    if d > 0 && idx.len() >= d {
                        let mut name = bn;
                        let mut ok = true;
                        for ix in &idx[..d] {
                            match self.eval_expr(ix).to_index() {
                                Some(v) => {
                                    use std::fmt::Write as _;
                                    let _ = write!(name, "[{}]", v);
                                }
                                None => {
                                    ok = false;
                                    break;
                                }
                            }
                        }
                        if ok {
                            if let Some(id) = self.resolve_signal_id(&name) {
                                if !out.contains(&id) {
                                    out.push(id);
                                }
                            }
                        }
                        for ix in idx {
                            self.collect_selected_cells(ix, out);
                        }
                        return;
                    }
                }
                self.collect_selected_cells(expr, out);
                self.collect_selected_cells(index, out);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                self.collect_selected_cells(expr, out);
                self.collect_selected_cells(left, out);
                self.collect_selected_cells(right, out);
            }
            ExprKind::Paren(x) | ExprKind::Unary { operand: x, .. } => {
                self.collect_selected_cells(x, out)
            }
            ExprKind::Binary { left, right, .. } => {
                self.collect_selected_cells(left, out);
                self.collect_selected_cells(right, out);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.collect_selected_cells(condition, out);
                self.collect_selected_cells(then_expr, out);
                self.collect_selected_cells(else_expr, out);
            }
            ExprKind::Concatenation(parts) => {
                for p in parts {
                    self.collect_selected_cells(p, out);
                }
            }
            _ => {}
        }
    }

    /// §9.4.2: replace a waiter's `cell` terms with the unpacked-array cells
    /// its expression terms select NOW. `@(mem[i])` watches `i` and the one
    /// element `mem[i]` names; when `i` moves, the element does too.
    pub(super) fn resolve_cell_terms(&mut self, terms: &mut Vec<SensitivityId>) {
        // One source per distinct event expression (all terms of one
        // expression carry the same one).
        let mut sources: Vec<SensitivityId> = Vec::new();
        for t in terms.iter() {
            let Some(e) = &t.value_of else { continue };
            if sources
                .iter()
                .any(|s| s.value_of.as_ref().is_some_and(|se| se.span == e.span))
            {
                continue;
            }
            if t.cell || self.expr_selects_array_cell(e) {
                sources.push(t.clone());
            }
        }
        terms.retain(|t| !t.cell);
        let mut cells: Vec<usize> = Vec::new();
        for src in sources {
            let Some(e) = src.value_of.clone() else {
                continue;
            };
            cells.clear();
            self.collect_selected_cells(&e, &mut cells);
            for &id in &cells {
                if terms.iter().any(|t| {
                    t.signal_id == id && t.value_of.as_ref().is_some_and(|te| te.span == e.span)
                }) {
                    continue;
                }
                terms.push(SensitivityId {
                    signal_id: id,
                    edge: src.edge,
                    iff: src.iff.clone(),
                    value_of: Some(e.clone()),
                    wake_rank: src.wake_rank,
                    cell: true,
                });
            }
        }
    }

    /// True when an event control names a clocking block (`@(cb)`) or the
    /// synthesized default clocking used by `##N` — a clocking event whose
    /// waiter must resume in the Reactive region (see `EventWaiter::is_clocking`).
    pub(super) fn is_clocking_event(&self, event: &EventControl) -> bool {
        // `@(cb)` parses as an EventExpr with an edge-less ident that names a
        // clocking block; mirror the detection in `event_to_sens`.
        match event {
            EventControl::Identifier(id) => {
                id.name == "__xz_default_clocking"
                    || id.name == "__xz_default_clocking0"
                    || self.clocking_meta.contains_key(&id.name)
            }
            // The bare `@cb` form (a HierIdentifier) is a clocking event too,
            // so its waiter must resume in the Reactive region like `@(cb)`.
            EventControl::HierIdentifier(e) => {
                if let ExprKind::Ident(h) = &e.kind {
                    let segs: Vec<&str> = h.path.iter().map(|s| s.name.name.as_str()).collect();
                    segs == ["__xz_default_clocking"] || self.resolve_clocking_key(&segs).is_some()
                } else {
                    false
                }
            }
            EventControl::EventExpr(exprs) => exprs.iter().any(|ee| {
                // `@(cb)` is an edge-less bare ident naming a clocking block.
                if ee.edge.is_some() {
                    return false;
                }
                // The term exactly as `event_to_sens` sees it: a receiver that
                // is a bound vif resolved to its instance (§25.9), a dotted
                // chain (`@(vif.cb)` in a method) flattened. Unrecognized, a
                // clocking waiter resumed in the Active region, before the
                // block sampled — its `vif.cb.sig` read returned the PREVIOUS
                // edge's sample.
                let rebased = self.vif_rebase_expr(&ee.expr);
                let term = rebased.as_ref().unwrap_or(&ee.expr);
                let flat = Self::member_chain_as_flat_ident_for_sens(term);
                let term = flat.as_ref().unwrap_or(term);
                if let ExprKind::Ident(h) = &term.kind {
                    // Interface-scoped `@(iface.cb)` / vif-aliased `@(vif.cb)`
                    // key on the (alias-resolved) dotted path; resolve_hier_name
                    // strips it to `cb`. Try both.
                    let segs: Vec<&str> = h.path.iter().map(|s| s.name.name.as_str()).collect();
                    if self.resolve_clocking_key(&segs).is_some() {
                        return true;
                    }
                    let sig = self.resolve_hier_name(h);
                    sig == "__xz_default_clocking" || self.clocking_meta.contains_key(&*sig)
                } else {
                    false
                }
            }),
            _ => false,
        }
    }

    /// §14.13: an explicit `@(cb)` (not the `##N` default-clocking form)
    /// whose block's clock edge is due in THIS time slot but not yet
    /// processed by `tick_clocking_blocks` — a process that reached it in
    /// the Active region after the raw edge (`@(posedge clk); @(cb);`) still
    /// sees this slot's clocking event, which the block raises after it
    /// samples. Such a continuation is resumed with the slot's clocking
    /// waiters instead of waiting a whole cycle.
    pub(super) fn clocking_event_due_now(&self, event: &EventControl) -> bool {
        if self.clocking_tick_cache.is_empty() {
            return false;
        }
        let key = match event {
            EventControl::Identifier(id) => {
                if id.name.starts_with("__xz_default_clocking") {
                    return false;
                }
                self.clocking_meta
                    .contains_key(&id.name)
                    .then(|| id.name.clone())
            }
            EventControl::HierIdentifier(e) => match &e.kind {
                ExprKind::Ident(h) => {
                    let segs: Vec<&str> = h.path.iter().map(|s| s.name.name.as_str()).collect();
                    self.resolve_clocking_key(&segs)
                }
                _ => None,
            },
            EventControl::EventExpr(exprs) if exprs.len() == 1 && exprs[0].edge.is_none() => {
                let rebased = self.vif_rebase_expr(&exprs[0].expr);
                let term = rebased.as_ref().unwrap_or(&exprs[0].expr);
                let flat = Self::member_chain_as_flat_ident_for_sens(term);
                let term = flat.as_ref().unwrap_or(term);
                match &term.kind {
                    ExprKind::Ident(h) => {
                        let segs: Vec<&str> = h.path.iter().map(|s| s.name.name.as_str()).collect();
                        self.resolve_clocking_key(&segs)
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        let Some(key) = key else {
            return false;
        };
        if key.starts_with("__xz_default_clocking") {
            return false;
        }
        let Some(ent) = self.clocking_tick_cache.iter().find(|t| t.cb == key) else {
            return false;
        };
        let cur = match ent.id {
            Some(id) => self.signal_table[id]
                .to_u64()
                .map(|u| (u & 1) as u8)
                .unwrap_or(2),
            None => return false,
        };
        match ent.edge {
            EdgeKind::Negedge => ent.prev == 1 && cur == 0,
            EdgeKind::Posedge => ent.prev == 0 && cur == 1,
            _ => ent.prev != cur && ent.prev != 2 && cur != 2,
        }
    }

    pub(super) fn collect_condition_read_names(e: &Expression, out: &mut HashSet<String>) {
        match &e.kind {
            ExprKind::Ident(h) => {
                for seg in &h.path {
                    out.insert(seg.name.name.clone());
                    for sel in &seg.selects {
                        Self::collect_condition_read_names(sel, out);
                    }
                }
            }
            ExprKind::MemberAccess { expr, member } => {
                out.insert(member.name.clone());
                Self::collect_condition_read_names(expr, out);
            }
            ExprKind::Binary { left, right, .. } => {
                Self::collect_condition_read_names(left, out);
                Self::collect_condition_read_names(right, out);
            }
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                Self::collect_condition_read_names(operand, out);
            }
            ExprKind::Index { expr, index } => {
                Self::collect_condition_read_names(expr, out);
                Self::collect_condition_read_names(index, out);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                Self::collect_condition_read_names(expr, out);
                Self::collect_condition_read_names(left, out);
                Self::collect_condition_read_names(right, out);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::collect_condition_read_names(condition, out);
                Self::collect_condition_read_names(then_expr, out);
                Self::collect_condition_read_names(else_expr, out);
            }
            ExprKind::Call { func, args } => {
                Self::collect_condition_read_names(func, out);
                for a in args {
                    Self::collect_condition_read_names(a, out);
                }
            }
            ExprKind::Concatenation(parts) => {
                for p in parts {
                    Self::collect_condition_read_names(p, out);
                }
            }
            ExprKind::SystemCall { name, args } if name == VC_CHANGED_MARKER => {
                if let Some(a) = args.get(1) {
                    Self::collect_condition_read_names(a, out);
                }
            }
            _ => {}
        }
    }

    pub(super) fn collect_lvalue_target_names(lhs: &Expression, out: &mut Vec<String>) {
        match &lhs.kind {
            ExprKind::Ident(h) => {
                for seg in &h.path {
                    out.push(seg.name.name.clone());
                }
            }
            ExprKind::MemberAccess { expr, member } => {
                out.push(member.name.clone());
                Self::collect_lvalue_target_names(expr, out);
            }
            ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => {
                Self::collect_lvalue_target_names(expr, out);
            }
            ExprKind::Concatenation(parts) => {
                for p in parts {
                    Self::collect_lvalue_target_names(p, out);
                }
            }
            _ => {}
        }
    }

    pub(super) fn park_condition_waiter(&mut self, pid: usize, cont: ProcCont, cond: &Expression) {
        let mut reads = HashSet::default();
        Self::collect_condition_read_names(cond, &mut reads);
        let gate = self.cond_gate_for(cond, &reads);
        self.cond_waiter_gate.insert(pid, gate);
        self.cond_waiter_reads.insert(pid, reads);
        self.cond_read_union_dirty = true;
        self.cond_waiter_conditions.insert(pid, cond.clone());
        self.condition_waiters.push((pid, cont));
    }

    /// Can the bulk re-schedule skip this waiter until a write notes one of
    /// the names its condition reads? Only when every operand lives in the
    /// string-keyed stores, whose every mutation notes its name in
    /// `STORE_WRITE_MASK`: a property or static of the parking process's
    /// `this` object, a local of the running method (fork children note
    /// their writes into it on merge), a property reached through such a
    /// handle, or a collection builtin on one of them. Anything else (an RTL
    /// signal, a `ref` formal, a call, a system function) keeps the
    /// unconditional re-schedule. Returns the `name_bit`s of each of `reads`.
    pub(super) fn cond_gate_for(
        &self,
        cond: &Expression,
        reads: &HashSet<String>,
    ) -> Option<Box<[u64]>> {
        // The running method's object, or (in a static method) none: then
        // only the class's statics count as its members.
        let handle = self
            .this_stack
            .last()
            .copied()
            .flatten()
            .filter(|&h| h != 0);
        let cls = match handle {
            Some(h) => self
                .heap
                .get(h)
                .and_then(|o| o.as_ref())?
                .class_name
                .clone(),
            None => self.class_context_stack.last().cloned().flatten()?,
        };
        if !self.cond_operand_gated(cond, handle.unwrap_or(0), &cls) {
            return None;
        }
        Some(reads.iter().map(|n| name_bit(n)).collect())
    }

    /// `cond_gate_for`'s operand check.
    pub(super) fn cond_operand_gated(&self, e: &Expression, this_h: usize, this_cls: &str) -> bool {
        let this_member = |n: &str| {
            if this_h == 0 {
                return self.static_prop_key(this_cls, n).is_some()
                    || self.member_is_static_coll(this_cls, n);
            }
            self.heap
                .get(this_h)
                .and_then(|o| o.as_ref())
                .is_some_and(|i| i.properties.contains_key(n))
                || self.class_prop_type_name(this_h, n).is_some()
                || self.static_prop_key(this_cls, n).is_some()
        };
        let method_local = |n: &str| {
            let base = self.method_local_base.last().copied().unwrap_or(0);
            !self
                .ref_alias_stack
                .last()
                .is_some_and(|m| m.contains_key(n))
                && self
                    .local_stack
                    .get(base..)
                    .unwrap_or(&[])
                    .iter()
                    .any(|m| m.contains_key(n))
        };
        // A property of the class object `h` (0 when not one).
        let obj_prop = |h: usize, n: &str| {
            h != 0
                && self
                    .heap
                    .get(h)
                    .and_then(|o| o.as_ref())
                    .is_some_and(|i| i.properties.contains_key(n))
        };
        match &e.kind {
            ExprKind::Number(_) | ExprKind::StringLiteral(_) | ExprKind::This => true,
            ExprKind::Ident(h) => {
                if h.root.is_some() {
                    return false;
                }
                let head = h.path[0].name.name.as_str();
                let head_ok = if method_local(head) {
                    !self.vif_local_names.contains(head)
                } else {
                    this_member(head) && !self.class_member_names().vif_props.contains(head)
                };
                if !head_ok
                    || !h.path.iter().all(|s| {
                        s.selects
                            .iter()
                            .all(|x| self.cond_operand_gated(x, this_h, this_cls))
                    })
                {
                    return false;
                }
                // `a.b.c`: every later segment a property of the object the
                // path reaches so far.
                let mut cur = if h.path.len() > 1 {
                    self.eval_ident_handle(head).unwrap_or(0)
                } else {
                    0
                };
                for seg in &h.path[1..] {
                    if !obj_prop(cur, &seg.name.name) {
                        return false;
                    }
                    cur = self.member_handle(cur, &seg.name.name).unwrap_or(0);
                }
                true
            }
            ExprKind::MemberAccess { expr, member } => {
                self.cond_operand_gated(expr, this_h, this_cls)
                    && obj_prop(self.eval_handle_expr(expr).unwrap_or(0), &member.name)
            }
            // A collection builtin on a gated operand (`m_q.size()`). A
            // user-defined method with the same name can read other fields;
            // gating only on its receiver misses those dependencies and can
            // repeatedly resume a false wait when call setup notes writes.
            ExprKind::Call { func, args } => {
                let ExprKind::MemberAccess { expr, member } = &func.kind else {
                    return false;
                };
                matches!(
                    member.name.as_str(),
                    "size" | "num" | "exists" | "first" | "last" | "next" | "prev"
                ) && !self.class_expr_has_method(expr, &member.name)
                    && self.cond_operand_gated(expr, this_h, this_cls)
                    && args
                        .iter()
                        .all(|a| self.cond_operand_gated(a, this_h, this_cls))
            }
            ExprKind::Binary { left, right, .. } => {
                self.cond_operand_gated(left, this_h, this_cls)
                    && self.cond_operand_gated(right, this_h, this_cls)
            }
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                self.cond_operand_gated(operand, this_h, this_cls)
            }
            ExprKind::Index { expr, index } => {
                self.cond_operand_gated(expr, this_h, this_cls)
                    && self.cond_operand_gated(index, this_h, this_cls)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.cond_operand_gated(condition, this_h, this_cls)
                    && self.cond_operand_gated(then_expr, this_h, this_cls)
                    && self.cond_operand_gated(else_expr, this_h, this_cls)
            }
            ExprKind::SystemCall { name, args } if name == VC_CHANGED_MARKER => args
                .get(1)
                .is_some_and(|a| self.cond_operand_gated(a, this_h, this_cls)),
            _ => false,
        }
    }
}
