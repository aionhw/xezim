//! Process contexts, fork child contexts and fork scopes.

use super::*;

impl Simulator {
    pub(in crate::compiler::simulator) fn snapshot_process_context(&self) -> ProcessContext {
        ProcessContext {
            this_stack: self.this_stack.clone(),
            local_stack: self.local_stack.clone(),
            block_scope_saves: self.block_scope_saves.clone(),
            local_gen_stack: self.local_gen_stack.clone(),
            local_type_stack: self.local_type_stack.clone(),
            class_context_stack: self.class_context_stack.clone(),
            current_spec: self.current_spec.clone(),
            spec_scope_stack: self.spec_scope_stack.clone(),
            cg_this: self.cg_this,
            return_value: self.return_value.clone(),
            break_flag: self.break_flag,
            continue_flag: self.continue_flag,
            return_flag: self.return_flag,
            local_iface_aliases: self.local_iface_aliases.clone(),
            ref_binding_stack: self.ref_binding_stack.clone(),
            ref_alias_stack: self.ref_alias_stack.clone(),
            ref_identity_stack: self.ref_identity_stack.clone(),
            queue_frame_saves: self.queue_frame_saves.clone(),
            task_cleanup: self.task_cleanup.clone(),
            local_dyn: self.local_dyn.clone(),
            formal_dyn: self.formal_dyn.clone(),
            static_local_syncs: self.static_local_syncs.clone(),
            method_local_base: self.method_local_base.clone(),
            current_static_task: self.current_static_task.clone(),
            current_auto_task: self.current_auto_task.clone(),
            task_string_formals: self.task_string_formals.clone(),
            local_meta_claims: self.local_meta_claims.clone(),
        }
    }

    /// Move the active execution context out without cloning it. This is used
    /// by retained simple timed assignments, whose body cannot suspend and has
    /// no process-local state of its own. A behavioral clock can fire while a
    /// task is inside `run_events_until`; moving the task context aside avoids
    /// cloning its local arrays on every clock edge.
    /// A `String` holding `text`, reusing a pooled buffer when one exists.
    #[inline]
    pub(in crate::compiler::simulator) fn pooled_string(
        pool: &mut Vec<String>,
        text: &str,
    ) -> String {
        match pool.pop() {
            Some(mut s) => {
                s.clear();
                s.push_str(text);
                s
            }
            None => text.to_string(),
        }
    }

    /// Return a hint string to the pool (bounded so a burst cannot pin memory).
    #[inline]
    pub(in crate::compiler::simulator) fn recycle_string(
        pool: &mut Vec<String>,
        s: Option<String>,
    ) {
        if let Some(s) = s {
            if pool.len() < 64 {
                pool.push(s);
            }
        }
    }

    /// True when every field `take_process_context` would move out is
    /// already empty/false — the state a top-level wake-up starts from.
    #[inline]
    pub(in crate::compiler::simulator) fn process_context_is_default(&self) -> bool {
        self.this_stack.is_empty()
            && self.local_stack.is_empty()
            && self.block_scope_saves.is_empty()
            && self.local_gen_stack.is_empty()
            && self.local_type_stack.is_empty()
            && self.class_context_stack.is_empty()
            && self.current_spec.is_none()
            && self.spec_scope_stack.is_empty()
            && self.cg_this.is_none()
            && self.return_value.is_none()
            && !self.break_flag
            && !self.continue_flag
            && !self.return_flag
            && self.local_iface_aliases.is_empty()
            && self.ref_binding_stack.is_empty()
            && self.ref_alias_stack.is_empty()
            && self.ref_identity_stack.is_empty()
            && self.queue_frame_saves.is_empty()
            && self.task_cleanup.is_empty()
            && self.local_dyn.is_empty()
            && self.formal_dyn.is_empty()
            && self.static_local_syncs.is_empty()
            && self.method_local_base.is_empty()
            && self.current_static_task.is_none()
            && self.current_auto_task.is_none()
            && self.task_string_formals.is_empty()
            && self.local_meta_claims.is_empty()
            && !self.ref_redirect_hot
    }

    pub(in crate::compiler::simulator) fn take_process_context(&mut self) -> ProcessContext {
        // §13.5.2: the aliased leaves of a `ref` struct formal reach their
        // actuals before another process runs.
        self.ref_frame_flush();
        // The ref stacks leave with the context; nothing can redirect until
        // a context is restored or a new ref frame is pushed.
        self.ref_redirect_hot = false;
        ProcessContext {
            this_stack: std::mem::take(&mut self.this_stack),
            local_stack: std::mem::take(&mut self.local_stack),
            block_scope_saves: std::mem::take(&mut self.block_scope_saves),
            local_gen_stack: std::mem::take(&mut self.local_gen_stack),
            local_type_stack: std::mem::take(&mut self.local_type_stack),
            class_context_stack: std::mem::take(&mut self.class_context_stack),
            current_spec: self.current_spec.take(),
            spec_scope_stack: std::mem::take(&mut self.spec_scope_stack),
            cg_this: self.cg_this.take(),
            return_value: self.return_value.take(),
            break_flag: std::mem::replace(&mut self.break_flag, false),
            continue_flag: std::mem::replace(&mut self.continue_flag, false),
            return_flag: std::mem::replace(&mut self.return_flag, false),
            local_iface_aliases: std::mem::take(&mut self.local_iface_aliases),
            ref_binding_stack: std::mem::take(&mut self.ref_binding_stack),
            ref_alias_stack: std::mem::take(&mut self.ref_alias_stack),
            ref_identity_stack: std::mem::take(&mut self.ref_identity_stack),
            queue_frame_saves: std::mem::take(&mut self.queue_frame_saves),
            task_cleanup: std::mem::take(&mut self.task_cleanup),
            local_dyn: std::mem::take(&mut self.local_dyn),
            formal_dyn: std::mem::take(&mut self.formal_dyn),
            static_local_syncs: std::mem::take(&mut self.static_local_syncs),
            method_local_base: std::mem::take(&mut self.method_local_base),
            current_static_task: self.current_static_task.take(),
            current_auto_task: self.current_auto_task.take(),
            task_string_formals: std::mem::take(&mut self.task_string_formals),
            local_meta_claims: std::mem::take(&mut self.local_meta_claims),
        }
    }

    pub(in crate::compiler::simulator) fn restore_process_context(&mut self, ctx: ProcessContext) {
        self.this_stack = ctx.this_stack;
        self.local_stack = ctx.local_stack;
        self.block_scope_saves = ctx.block_scope_saves;
        self.local_gen_stack = ctx.local_gen_stack;
        self.local_type_stack = ctx.local_type_stack;
        self.class_context_stack = ctx.class_context_stack;
        self.current_spec = ctx.current_spec;
        self.spec_scope_stack = ctx.spec_scope_stack;
        self.cg_this = ctx.cg_this;
        self.return_value = ctx.return_value;
        self.break_flag = ctx.break_flag;
        self.continue_flag = ctx.continue_flag;
        self.return_flag = ctx.return_flag;
        self.local_iface_aliases = ctx.local_iface_aliases;
        self.ref_binding_stack = ctx.ref_binding_stack;
        self.ref_alias_stack = ctx.ref_alias_stack;
        self.ref_identity_stack = ctx.ref_identity_stack;
        self.refresh_ref_redirect_hot();
        self.queue_frame_saves = ctx.queue_frame_saves;
        self.task_cleanup = ctx.task_cleanup;
        self.local_dyn = ctx.local_dyn;
        self.formal_dyn = ctx.formal_dyn;
        self.static_local_syncs = ctx.static_local_syncs;
        self.method_local_base = ctx.method_local_base;
        self.current_static_task = ctx.current_static_task;
        self.current_auto_task = ctx.current_auto_task;
        self.task_string_formals = ctx.task_string_formals;
        self.local_meta_claims = ctx.local_meta_claims;
        if !self.local_meta_cache.is_empty() && !self.local_meta_claims.is_empty() {
            self.reapply_local_meta();
        }
        if !self.task_cleanup.is_empty() {
            self.reinstall_struct_formal_meta();
        }
        self.ref_frame_refresh();
    }

    /// Capture the current process context, including the current
    /// values of automatic `for`-loop variables (`auto_loop_vars`) by value into
    /// the child's context so each fork child sees the loop index at fork time
    /// (LRM §9.3.2). Used when spawning fork children.
    /// §21.2.1.7 / §9.3.5: a fork child runs inside the forking process's
    /// named blocks and inside the fork block itself, named or not, so its
    /// `%m` chain starts with the parent's chain (plus the fork's label).
    pub(in crate::compiler::simulator) fn inherit_fork_m_scope(
        &mut self,
        child: usize,
        parent: usize,
        fork_name: Option<&crate::ast::Identifier>,
    ) {
        // §24.3: a fork child of a program process is a program process.
        if !self.program_pids.is_empty() && self.program_pids.contains(&parent) {
            self.program_pids.insert(child);
            if let Some(scope) = self.program_pid_scope.get(&parent).cloned() {
                self.program_pid_scope.insert(child, scope);
            }
        }
        if let Some(l) = self.process_m_label.get(&parent) {
            let l = l.clone();
            self.process_m_label.insert(child, l);
        }
        if self.m_scope_stack.is_empty() && fork_name.is_none() {
            return;
        }
        let mut chain = self.m_scope_stack.clone();
        if let Some(f) = fork_name {
            chain.push(f.name.clone());
        }
        self.process_m_scope.insert(child, chain);
    }

    /// `a[i][j]` -> (`a`, 2): the operand under its constant-or-not index
    /// selects (not part selects), and how many there are.
    pub(in crate::compiler::simulator) fn peel_index_selects(
        e: &Expression,
    ) -> Option<(Expression, usize)> {
        let mut k = 0usize;
        let mut cur = e;
        while let ExprKind::Index { expr, .. } = &cur.kind {
            k += 1;
            cur = expr;
        }
        if let ExprKind::Ident(h) = &cur.kind {
            let last = h.path.last()?;
            if !last.selects.is_empty() {
                if last
                    .selects
                    .iter()
                    .any(|x| matches!(x.kind, ExprKind::Range(..) | ExprKind::RangeSelect { .. }))
                {
                    return None;
                }
                k += last.selects.len();
                let mut hb = h.clone();
                hb.path.last_mut()?.selects.clear();
                hb.cached_signal_id = crate::ast::cache::SignalCache::new(None);
                hb.cached_resolved_name = crate::ast::cache::NameCache::new();
                return Some((Expression::new(ExprKind::Ident(hb), cur.span), k));
            }
        }
        (k > 0).then(|| (cur.clone(), k))
    }

    pub(in crate::compiler::simulator) fn int_lit_expr(
        n: i64,
        span: crate::ast::Span,
    ) -> Expression {
        Expression::new(
            ExprKind::Number(crate::ast::expr::NumberLiteral::Integer {
                size: None,
                signed: true,
                base: crate::ast::expr::NumberBase::Decimal,
                value: n.to_string(),
                cached_val: Default::default(),
            }),
            span,
        )
    }

    /// §7.4.2 / §20.7: an object's store `scoped` of a fixed-array member
    /// with a descending dimension (`int ds[3:1]`) keeps its declared
    /// direction, as a module array does (`array_prop_decl_dims`).
    pub(in crate::compiler::simulator) fn note_member_decl_dims(
        &mut self,
        cdef: &crate::compiler::elaborate::ElaboratedClass,
        prop: &str,
        scoped: &str,
    ) {
        if let Some(dd) = cdef.array_prop_decl_dims.get(prop) {
            if dd.first().is_some_and(|&(l, r)| l > r) {
                self.module.descending_arrays.insert(scoped.to_string());
            }
            self.module
                .unpacked_decl_dims
                .insert(scoped.to_string(), dd.clone());
        }
    }

    pub(in crate::compiler::simulator) fn inherit_fork_child_context(&mut self, pid: usize) {
        let mut ctx = self.snapshot_process_context();
        // The parent's open block scopes close in the parent's stream, never
        // in the child's.
        let mut local_scopes = self.fork_local_scopes.get(&self.current_pid)
            .cloned().unwrap_or_default();
        Self::overlay_fork_local_scopes(&mut local_scopes, &ctx.block_scope_saves);
        self.fork_local_scopes.insert(pid, local_scopes);
        ctx.block_scope_saves.clear();
        // §6.21/§9.3.2: automatic block-locals (no call frame) live in
        // `self.signals` and are listed in `auto_loop_vars`. Copy their
        // current VALUES into the child's `local_stack` so each child gets
        // its own snapshot (the child's reads consult its top frame first).
        // Record WHICH names were captured from `signals` so the propagate
        // path can write the child's changed values back into `self.signals`
        // — the storage the parent actually reads from. Without this
        // write-back, the child's writes are stranded in its private frame
        // and the parent sees the stale pre-fork value (§6.21 violation).
        let mut signal_caps: Vec<String> = Vec::new();
        if !self.auto_loop_vars.is_empty() {
            let mut caps: HashMap<String, Value> = HashMap::default();
            for nm in &self.auto_loop_vars {
                if let Some(v) = self.signals.get(nm) {
                    caps.insert(nm.clone(), v.clone());
                    signal_caps.push(nm.clone());
                }
            }
            if !caps.is_empty() {
                // Merge into the top frame if one exists (don't shadow the
                // forking process's other locals — reads consult only the top
                // frame); otherwise this becomes the child's first frame.
                if let Some(top) = ctx.local_stack.last_mut() {
                    for (k, v) in caps {
                        top.insert(k, v);
                    }
                } else {
                    ctx.local_stack.push(caps);
                    // A capture frame, not a call frame (see
                    // `in_process_block_scope`).
                    let g = self.next_frame_gen();
                    self.loop_frame_gens.insert(g);
                    ctx.local_gen_stack.push(g);
                }
            }
        }
        let collections: HashSet<String> = ctx
            .local_dyn
            .iter()
            .chain(ctx.formal_dyn.iter())
            .flat_map(|frame| frame.iter().map(|(_, key)| key))
            .filter(|key| key.starts_with('@'))
            .cloned()
            .collect();
        // Index the child under every activation it holds, for the sibling
        // write broadcast in `run_scheduled_process_inner`.
        for &g in &ctx.local_gen_stack {
            self.fork_gen_pids.entry(g).or_default().insert(pid);
        }
        self.fork_pid_gens.insert(pid, ctx.local_gen_stack.clone());
        if !collections.is_empty() {
            for key in &collections {
                *self.fork_capture_refs.entry(key.clone()).or_default() += 1;
            }
            self.fork_collection_captures.insert(pid, collections);
        }
        let trivial = ctx.this_stack.is_empty()
            && ctx.local_stack.is_empty()
            && ctx.class_context_stack.is_empty()
            && ctx.cg_this.is_none()
            && ctx.return_value.is_none()
            && !ctx.break_flag
            && !ctx.continue_flag
            && !ctx.return_flag
            && ctx.local_dyn.is_empty()
            && ctx.static_local_syncs.is_empty()
            && ctx.method_local_base.is_empty();
        if trivial {
            self.process_contexts.remove(&pid);
        } else {
            // §9.3.2 baseline: snapshot the child's local frames as forked so
            // `propagate_fork_locals_to_parent` can merge only the keys the
            // child later WRITES (value changes), not keys it merely inherited
            // (which the parent may have modified in the meantime — e.g. a
            // mailbox-get delivery into `phase` while this child runs).
            self.fork_baselines
                .insert(pid, Arc::new(ctx.local_stack.clone()));
            self.fork_signal_captures.insert(pid, signal_caps);
            self.process_contexts.insert(pid, ctx);
        }
    }

    /// Track lexical declarations separately from flattened frame slots.
    pub(super) fn overlay_fork_local_scopes(
        scopes: &mut HashMap<u64, HashMap<String, u64>>,
        blocks: &[BlockScopeSave],
    ) {
        for block in blocks {
            for name in &block.declared_names {
                scopes.entry(block.frame_gen).or_default()
                    .insert(name.clone(), block.scope_gen);
            }
        }
    }

    /// Merge a fork child's local-frame writes into a parent's `local_stack`.
    ///
    /// §9.3.2 baseline: only the keys the child WROTE (its value now differs
    /// from the fork-time snapshot) are propagated — a key it merely
    /// inherited unchanged must NOT clobber a value the parent modified
    /// after the fork (e.g. a mailbox-get delivery into `phase` while this
    /// child ran). If no baseline survived, fall back to the legacy
    /// whole-frame merge so the §9.3.2 write-visibility guarantee still
    /// holds. Only keys that exist in the parent's frame are propagated (a
    /// key the child declared itself, like a loop-local, is child-private).
    /// Write a fork child's changed locals back into the parent's frames.
    /// `parent_gens` / `child_gens` are the `local_gen_stack`s of the parent
    /// (the frame set it owns *right now*) and the child (the frame set it
    /// inherited at fork time). For slot index `i` we only propagate when the
    /// two generations agree: slot `i` in the parent must still be the very
    /// activation the child forked from. A fork child that outlives its
    /// creating method and keeps writing its *stale* automatic locals must not
    /// clobber the unrelated activation that now occupies the same slot
    /// (regression 3627: an alpha reader orphaned by `join_any` kept overwriting
    /// beta's `count` with its own stale value).
    /// Returns the names of keys actually propagated (child value differs
    /// from both baseline and parent, so a real write landed). The caller
    /// uses this to promote parked condition waiters that read those names.
    /// `baseline` is what the child last published or was given (see
    /// `sync_fork_baseline`), so a write back to the fork-time value counts.
    pub(in crate::compiler::simulator) fn merge_fork_writes(
        parent_frames: &mut [HashMap<String, Value>],
        parent_gens: &[u64],
        child_frames: &[HashMap<String, Value>],
        child_gens: &[u64],
        baseline: Option<&Vec<HashMap<String, Value>>>,
        private: &dyn Fn(u64, &str) -> bool,
    ) -> Vec<String> {
        let mut propagated = Vec::new();
        let n = parent_frames.len().min(child_frames.len());
        for i in 0..n {
            let g = child_gens.get(i).copied().unwrap_or(0);
            let same_activation = parent_gens.get(i).copied() == child_gens.get(i).copied();
            if !same_activation {
                // The parent slot `i` no longer belongs to the activation this
                // child forked from — keep this child's locals out of it.
                continue;
            }
            // The unchanged test first: nearly every inherited key is
            // unchanged, and it then needs no parent lookup.
            let base_frame = baseline.and_then(|b| b.get(i));
            // A frame whose keys were never inserted or removed since the
            // fork still has its baseline clone's layout, so the two iterate
            // in the same key order and pair up without a lookup. Any key
            // mismatch falls back to the keyed loop, which re-derives the
            // same writes (a repeated write is a no-op).
            if let Some(bf) = base_frame.filter(|f| f.len() == child_frames[i].len()) {
                let mut paired = true;
                for ((k, v), (bk, bv)) in child_frames[i].iter().zip(bf.iter()) {
                    if k != bk {
                        paired = false;
                        break;
                    }
                    if v == bv || private(g, k) {
                        continue;
                    }
                    if let Some(slot) = parent_frames[i].get_mut(k) {
                        if slot != v {
                            *slot = v.clone();
                            // A parked `wait` of the parent may read it.
                            note_store_write(name_bit(k));
                            propagated.push(k.clone());
                        }
                    }
                }
                if paired {
                    continue;
                }
            }
            for (k, v) in &child_frames[i] {
                let inherited_unchanged = base_frame
                    .and_then(|f| f.get(k))
                    .is_some_and(|old| old == v);
                if inherited_unchanged || private(g, k) {
                    continue;
                }
                match parent_frames[i].get_mut(k) {
                    Some(slot) => {
                        if slot != v {
                            *slot = v.clone();
                            // A parked `wait` of the parent may read it.
                            note_store_write(name_bit(k));
                            propagated.push(k.clone());
                        }
                    }
                    None => continue,
                }
            }
        }
        propagated
    }

    /// Move a fork child's baseline to the values its frames now hold, for
    /// every key it inherited. Copied on write: the snapshot is shared until
    /// a value actually differs. A key one of the child's open `blocks`
    /// declared holds the block's own variable, not the inherited one, so
    /// its baseline stays (see `block_declares`).
    pub(in crate::compiler::simulator) fn sync_fork_baseline(
        base: &mut Arc<Vec<HashMap<String, Value>>>,
        frames: &[HashMap<String, Value>],
        gens: &[u64],
        blocks: &[BlockScopeSave],
    ) {
        let moved = |i: usize, k: &str, bv: &Value, cf: &HashMap<String, Value>| {
            cf.get(k).is_some_and(|v| v != bv)
                && !Self::block_declares(blocks, gens.get(i).copied().unwrap_or(0), k)
        };
        let differs = |b: &[HashMap<String, Value>]| {
            b.iter()
                .zip(frames)
                .enumerate()
                .any(|(i, (bf, cf))| bf.iter().any(|(k, bv)| moved(i, k, bv, cf)))
        };
        if !differs(base) {
            return;
        }
        for (i, (bf, cf)) in Arc::make_mut(base).iter_mut().zip(frames).enumerate() {
            for (k, bv) in bf.iter_mut() {
                if moved(i, k, bv, cf) {
                    if let Some(v) = cf.get(k) {
                        *bv = v.clone();
                    }
                }
            }
        }
    }

    /// Store a fork sibling's writes (`frames`, under activations `gens`)
    /// of `shared` variables that the receiver's open `blocks` hide into the
    /// hidden variables' saved values, the outermost hiding block's (§6.21).
    /// Returns them by frame index, key and value.
    pub(super) fn deliver_to_hidden(
        blocks: &mut [BlockScopeSave],
        receiver_gens: &[u64],
        frames: &[HashMap<String, Value>],
        gens: &[u64],
        shared: &dyn Fn(u64, &str) -> bool,
    ) -> Vec<(usize, String, Value)> {
        let mut out = Vec::new();
        for (i, frame) in frames.iter().enumerate() {
            let Some(&g) = gens.get(i) else {
                continue;
            };
            if receiver_gens.get(i) != Some(&g) {
                continue;
            }
            for (k, v) in frame {
                if !shared(g, k) {
                    continue;
                }
                let slot = blocks
                    .iter_mut()
                    .filter(|b| b.frame_gen == g)
                    .find_map(|b| b.shadowed.iter_mut().find(|(n, _)| n == k));
                if let Some(slot) = slot {
                    slot.1 = v.clone();
                    out.push((i, k.clone(), v.clone()));
                }
            }
        }
        out
    }

    /// Whether one of the open `blocks` declared the variable of frame entry
    /// `key` (its bare name, or a leaf `name.member` / `name[i]` of an
    /// unpacked aggregate) into the frame of generation `g` (§6.21): the
    /// entry is then the block's variable, hiding the enclosing one of the
    /// same name.
    pub(in crate::compiler::simulator) fn block_declares(
        blocks: &[BlockScopeSave],
        g: u64,
        key: &str,
    ) -> bool {
        let names = |n: &String| {
            key.strip_prefix(n.as_str())
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', '[']))
        };
        !blocks.is_empty()
            && blocks
                .iter()
                .any(|b| b.frame_gen == g && b.declared_names.iter().any(names))
    }
}
