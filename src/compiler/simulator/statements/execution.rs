//! Statement execution.

use super::*;

impl Simulator {
    /// Outlined from `exec_statement` so the dispatcher's stack frame
    /// stays small: this arm's locals inflated every call's frame (the
    /// dispatcher recursed with a 7.8 KB frame per level).
    #[inline(never)]
    /// A statement-coverage marker inserted by `--code-coverage`.
    fn is_cov_marker(s: &Statement) -> bool {
        matches!(&s.kind, StatementKind::Expr(e)
            if matches!(&e.kind, ExprKind::SystemCall { name, .. } if name == super::COV_HIT_TASK))
    }

    pub fn exec_statement(&mut self, stmt: &Statement) {
        if self.finished
            || self.time > self.max_time
            || self.break_flag
            || self.continue_flag
            || self.return_flag
        {
            return;
        }
        // §10.4.2 / §25.9: evaluate receiver calls once while capturing an
        // NBA target, before rebasing it to the interface instance. Re-enter
        // with those calls pinned so callbacks still fire once below.
        if let StatementKind::NonblockingAssign { lvalue, rvalue, delay } = &stmt.kind
            && (Self::recv_chain_call(rvalue).is_some() || Self::recv_chain_call(lvalue).is_some())
            && let Some(base) = self.pin_stmt_recv_calls(&[rvalue, lvalue])
        {
            if self.is_unpacked_aggregate_operand(lvalue) || self.is_unpacked_aggregate_operand(rvalue) {
                let rvalue = self.freeze_conditional_path(rvalue, true);
                let lvalue = self.freeze_conditional_path(lvalue, true);
                self.exec_statement(&Statement::new(StatementKind::NonblockingAssign {
                    lvalue, rvalue, delay: delay.clone(),
                }, stmt.span));
            } else {
                self.exec_statement(stmt);
            }
            self.recv_call_memo.truncate(base);
            return;
        }
        // §38.36.1 cbStmt, before the statement runs (interpreted code only).
        if let StatementKind::NonblockingAssign {
            lvalue,
            delay,
            rvalue,
        } = &stmt.kind
            && let Some(selected) = self.selected_aggregate_rhs(Some(lvalue), rvalue)
        {
            self.exec_statement(&Statement::new(
                StatementKind::NonblockingAssign {
                    lvalue: lvalue.clone(),
                    delay: delay.clone(),
                    rvalue: selected,
                },
                stmt.span,
            ));
            return;
        }
        if self.vpi_cb_mask & vpi_cb::m(vpi_cb::CB_STMT) != 0 {
            self.vpi_stmt_hook(stmt, false);
        }
        match &stmt.kind {
            StatementKind::Null => {}
            StatementKind::ScopePop => {
                if let Some(c) = self.task_cleanup.pop() {
                    self.unwind_task_frame(c);
                }
            }
            // INTERNAL continuation sentinel — only meaningful in the
            // suspend-aware `run_process_stmts` stream. If reached here
            // (synchronous exec), it is a no-op.
            StatementKind::ForeachTail { .. } => {}
            StatementKind::ForeverTail { .. } => {}
            // Same: the synchronous `for` runs its own step, so the barrier
            // has nothing to do here. (It cannot even be reached with a
            // pending `continue` — the guard above returns first, which is
            // precisely why `run_process_stmts` handles it itself.)
            StatementKind::LoopStep => {}
            StatementKind::Expr(expr) => self.exec_expr_stmt(expr),
            StatementKind::BlockingAssign { lvalue, rvalue } => {
                self.exec_stmt_blocking_assign(stmt, lvalue, rvalue)
            }
            StatementKind::NonblockingAssign {
                lvalue,
                delay,
                rvalue,
            } => {
                // §25.8/§25.9: an lvalue whose receiver is a bound virtual
                // interface (`cfg.vif.data`, `d.a.cfg.vif.cb.data`) targets
                // the bound instance. Resolved NOW, while the owner handles
                // and `this` are live: the commit runs later in a context
                // where neither is.
                let lvalue_vif;
                let lvalue: &Expression = match self
                    .vif_rebase_expr(lvalue)
                    .or_else(|| self.inst_array_member_canon(lvalue))
                {
                    Some(r) => {
                        lvalue_vif = r;
                        &lvalue_vif
                    }
                    None => lvalue,
                };
                // §11.4.1: an lvalue index with a side effect
                // (`v[i++ +: 18] <= ...`) is evaluated once. Fold it to the
                // literal it evaluates to now, as the blocking path does, so
                // the target's width and the captured target below read the
                // same index. Before, `i` advanced more than once per
                // assignment and the value landed in the wrong slice. The
                // statement carries on with the folded target, so its
                // statement callbacks fire once.
                let lvalue_folded;
                let lvalue: &Expression = match Self::lvalue_has_incdec_index(lvalue)
                    .then(|| self.rewrite_lvalue_index_side_effects(lvalue))
                    .flatten()
                {
                    Some(f) => {
                        lvalue_folded = f;
                        &lvalue_folded
                    }
                    None => lvalue,
                };
                // §14.4: normalize a plain dotted-chain lvalue (`vif.cb.data`
                // inside a method) to the flat Ident the clocking-drive check
                // below expects — the chain shape skipped the check and the
                // drive landed on a phantom name instead of the clocking
                // output net.
                // Keep the member tree for declaration-based packed geometry;
                // the flat spelling below is only needed by clocking drives.
                let packed_lvalue = lvalue;
                let lvalue_norm;
                let lvalue: &Expression = match Self::member_chain_as_flat_ident(lvalue) {
                    Some(flat) => {
                        lvalue_norm = flat;
                        &lvalue_norm
                    }
                    None => lvalue,
                };
                // §9.4.5 intra-assignment delay on an NBA (`lhs <= #d rhs`,
                // canonicalized to `$__xz_intra_delay(d, rhs)`): the RHS is
                // evaluated now; the update is scheduled d ticks out.
                let mut intra_d: Option<u64> = None;
                // §14.16 `cb.out <= ##N rhs`: strip the cycle marker and keep
                // N; the clocking-drive path below defers by N clock events.
                let mut intra_cycles: Option<u64> = None;
                let rvalue: &Expression = match Self::intra_cycle_marker(rvalue) {
                    Some((n_expr, rhs)) => {
                        intra_cycles = Some(self.eval_expr(n_expr).to_u64().unwrap_or(0));
                        rhs
                    }
                    None => rvalue,
                };
                let rvalue: &Expression = match Self::intra_delay_marker(rvalue) {
                    Some((d_expr, rhs)) => {
                        intra_d = Some(self.eval_delay_ticks(d_expr));
                        rhs
                    }
                    None => rvalue,
                };
                let frozen_rhs = self.freeze_array_rhs_selectors(rvalue);
                let rvalue = frozen_rhs.as_ref().unwrap_or(rvalue);
                // §9.4.5 intra-assignment EVENT control on an NBA
                // (`lhs <= [repeat(n)] @(edge sig) rhs`, canonicalized to
                // `$__xz_intra_ev(n, edge, sig, rhs)`). Only the BLOCKING arm
                // handled the marker, so this rvalue fell through to plain
                // eval — an unknown system call, i.e. ZERO — and the NBA
                // posted 0 immediately, clobbering the variable instead of
                // updating it at the n-th edge. Capture the RHS NOW, spawn a
                // one-shot process that waits the events and posts the saved
                // value as an ordinary NBA; the CURRENT process continues
                // without blocking (that is what makes it nonblocking).
                if let ExprKind::SystemCall { name, args } = &rvalue.kind {
                    if name == crate::intra_delay::INTRA_EVENT_MARKER && args.len() == 4 {
                        let n_val = self.eval_expr(&args[0]).to_u64().unwrap_or(1) as i64;
                        let edge_code = self.eval_expr(&args[1]).to_u64().unwrap_or(0);
                        let (saved, release) =
                            self.capture_intra_rhs(packed_lvalue, &args[3], rvalue.span);
                        let edge = match edge_code {
                            1 => Some(Edge::Posedge),
                            2 => Some(Edge::Negedge),
                            _ => None,
                        };
                        let mut cont: Vec<Statement> = Vec::new();
                        for _ in 0..n_val.max(0) {
                            cont.push(Statement::new(
                                StatementKind::TimingControl {
                                    control: TimingControl::Event(EventControl::EventExpr(vec![
                                        EventExpr {
                                            edge,
                                            expr: args[2].clone(),
                                            iff: None,
                                            span: stmt.span,
                                        },
                                    ])),
                                    stmt: Box::new(Statement::new(StatementKind::Null, stmt.span)),
                                },
                                stmt.span,
                            ));
                        }
                        cont.push(Statement::new(
                            StatementKind::NonblockingAssign {
                                lvalue: lvalue.clone(),
                                delay: None,
                                rvalue: saved,
                            },
                            stmt.span,
                        ));
                        cont.extend(release);
                        let child = self.next_pid;
                        self.next_pid += 1;
                        self.process_origin
                            .insert(child, (stmt.span, "intra-assignment event NBA"));
                        self.event_queue.schedule(self.time, child, cont.into());
                        return;
                    }
                }
                // §10.10 / §10.4.2: capture all array operands now; replace
                // the collection only when this entry reaches the NBA region.
                if let Some(elements) = self.capture_array_rhs(lvalue, rvalue) {
                    let d = intra_d.unwrap_or_else(|| {
                        delay
                            .as_ref()
                            .map(|e| self.eval_delay_ticks(e))
                            .unwrap_or(0)
                    });
                    // Fixed storage shares NBA ordering with element writes.
                    // Put its captured leaves through the ordinary scheduler
                    // so a later `a[i] <= v` overrides this whole-array store.
                    if let Some(dst) = self.array_operand_name(lvalue)
                        && !self.module.dynamic_arrays.contains(&dst)
                        && !self.module.queue_vars.contains(&dst)
                        && let Some((targets, _)) = self.stream_elem_order(&dst)
                    {
                        for (target, value) in targets.into_iter().zip(elements) {
                            let lhs = Self::append_leaf_path(lvalue, &target[dst.len()..]);
                            let saved = self.make_intra_saved_expr(value, rvalue.span);
                            let delay = (d > 0).then(|| {
                                self.make_intra_saved_expr(Value::from_u64(d, 64), stmt.span)
                            });
                            self.exec_statement(&Statement::new(
                                StatementKind::NonblockingAssign {
                                    lvalue: lhs,
                                    delay: delay.map(Box::new),
                                    rvalue: saved,
                                },
                                stmt.span,
                            ));
                        }
                        return;
                    }
                    let frozen = self.freeze_lvalue_indices(lvalue);
                    let root_frame = self.nba_root_frame(lvalue);
                    self.enqueue_context_nba(
                        NbaEntry {
                            elements: Some(elements),
                            lhs: Some(frozen),
                            value: Value::zero(1),
                            resolved_id: None,
                            static_key: None,
                            static_slice: None,
                            this_handle: self.this_stack.last().copied().flatten(),
                            root_frame,
                        },
                        d,
                    );
                    return;
                }
                // §10.8, §10.9: a pattern driven onto a packed target takes
                // the target's declared type.
                let packed_pattern = self.eval_pattern_for_lvalue(packed_lvalue, rvalue);
                let rvalue = self.unpacked_pattern(rvalue);
                // §10.9.2/§10.10: a pattern written to an aggregate whose
                // leaves are separate signals (unpacked struct, unpacked array
                // of them, associative array) is spread member-wise.
                if let ExprKind::AssignmentPattern(items) = &rvalue.kind {
                    if packed_pattern.is_none() {
                        // Fixed arrays and records use stable leaf storage. Runtime-sized
                        // collections retain their own size/default update path.
                        let mut root = lvalue;
                        while let ExprKind::Index { expr, .. }
                        | ExprKind::MemberAccess { expr, .. } = &root.kind
                        {
                            root = expr;
                        }
                        let capture = if let ExprKind::Ident(h) = &root.kind {
                            let name = self.resolve_hier_name(h);
                            !self.module.dynamic_arrays.contains(name.as_ref())
                                && !self.module.queue_vars.contains(name.as_ref())
                                && !self.is_associative_array(name.as_ref())
                        } else {
                            true
                        };
                        let previous_capture =
                            std::mem::replace(&mut self.pattern_nba_leaves, capture.then(Vec::new));
                        let spread = if matches!(&lvalue.kind, ExprKind::Ident(_)) {
                            self.pattern_aggregate_names(lvalue)
                                .into_iter()
                                .any(|n| self.assign_pattern_aggregate(&n, items))
                        } else {
                            self.assign_pattern_element(lvalue, rvalue)
                                || match self.flat_member_name(lvalue) {
                                    Some(flat) => self.assign_pattern_flat(&flat, items),
                                    None => false,
                                }
                        };
                        let leaves = self.pattern_nba_leaves.take().unwrap_or_default();
                        self.pattern_nba_leaves = previous_capture;
                        if spread {
                            // §10.4.2, §9.4.5: evaluate every item now, then
                            // route the captured leaves through normal NBA scheduling.
                            let d = intra_d.unwrap_or_else(|| {
                                delay
                                    .as_ref()
                                    .map(|e| self.eval_expr(e).to_u64().unwrap_or(0))
                                    .unwrap_or(0)
                            });
                            for (name, value) in leaves {
                                let split = name.find(['.', '[']).unwrap_or(name.len());
                                let base =
                                    Self::flat_name_ident(name[..split].to_string(), stmt.span);
                                let target = Self::append_leaf_path(&base, &name[split..]);
                                let saved = self.make_intra_saved_expr(value, rvalue.span);
                                let delay = (d > 0).then(|| {
                                    self.make_intra_saved_expr(Value::from_u64(d, 64), stmt.span)
                                });
                                self.exec_statement(&Statement::new(
                                    StatementKind::NonblockingAssign {
                                        lvalue: target,
                                        delay: delay.map(Box::new),
                                        rvalue: saved,
                                    },
                                    stmt.span,
                                ));
                            }
                            return;
                        }
                    }
                }
                // §10.4.2: a whole UNPACKED struct has no storage of its own —
                // its member leaves are the storage — so it is updated as one
                // nonblocking assignment per leaf, every source leaf read now.
                if let Some(leaf_nbas) = self
                    .unpacked_array_nba_leaves(lvalue, rvalue)
                    .or_else(|| self.unpacked_row_nba_leaves(lvalue, rvalue))
                    .or_else(|| self.unpacked_struct_nba_leaves(lvalue, rvalue))
                {
                    let d = match intra_d {
                        Some(d) => d,
                        None => delay
                            .as_ref()
                            .map(|de| self.eval_expr(de).to_u64().unwrap_or(0))
                            .unwrap_or(0),
                    };
                    let delay_expr = (d > 0).then(|| {
                        Expression::new(
                            ExprKind::Number(NumberLiteral::Integer {
                                size: None,
                                signed: false,
                                base: NumberBase::Decimal,
                                value: d.to_string(),
                                cached_val: crate::ast::cache::LiteralCache::new(None),
                            }),
                            stmt.span,
                        )
                    });
                    for (leaf_lvalue, v) in leaf_nbas {
                        let saved = self.make_intra_saved_expr(v, rvalue.span);
                        self.exec_statement(&Statement::new(
                            StatementKind::NonblockingAssign {
                                lvalue: leaf_lvalue,
                                delay: delay_expr.clone().map(Box::new),
                                rvalue: saved,
                            },
                            stmt.span,
                        ));
                    }
                    return;
                }
                // §10.4.2, §7.6: a subarray or slice of a multidimensional
                // array (a design variable's, a class property's) that the
                // element-wise paths above do not pair with the target
                // (`q <= src[1]` into a one-dimensional array) is read element
                // by element now and stored as the pattern of those values.
                // It was read as one scalar, so every element received 0.
                // Only a design with multidimensional storage has one.
                if packed_pattern.is_none()
                    && intra_cycles.is_none()
                    && (!self.module.arrays_2d.is_empty() || !self.module.arrays_nd.is_empty())
                    && let Some((pattern, release)) =
                        self.unpacked_array_snapshot(rvalue, rvalue.span)
                {
                    let d = intra_d.unwrap_or_else(|| {
                        delay
                            .as_ref()
                            .map(|e| self.eval_delay_ticks(e))
                            .unwrap_or(0)
                    });
                    let delay = (d > 0)
                        .then(|| self.make_intra_saved_expr(Value::from_u64(d, 64), stmt.span));
                    self.exec_statement(&Statement::new(
                        StatementKind::NonblockingAssign {
                            lvalue: packed_lvalue.clone(),
                            delay: delay.map(Box::new),
                            rvalue: pattern,
                        },
                        stmt.span,
                    ));
                    if let Some(release) = release {
                        self.exec_statement(&release);
                    }
                    return;
                }
                let val = match (packed_pattern, &rvalue.kind) {
                    (Some(v), _) => v,
                    (None, ExprKind::AssignmentPattern(_)) => self.eval_expr(rvalue),
                    (None, _) => self.eval_target_rhs(packed_lvalue, rvalue),
                };
                let val = self.fill_into_real_target(lvalue, val);
                // LRM §14.4: clocking-block output drive
                // `cb.<out_sig> <= val`. Defer to the cb's next clock
                // edge (output skew) instead of driving now. Handles both a
                // module-scoped `cb.sig` and an interface-scoped
                // `iface_inst.cb.sig` (≥3 segments): the block key is every
                // segment but the last, and the queued signal is the RESOLVED
                // output net (matched from the block's signal list by exact
                // name or `.sig` suffix) so `tick_clocking_blocks` drives it.
                // A drive of a select (`cb.v[2]`, `vif.cb.u[1]`, `cb.x[3:2]`)
                // is recognized through its clockvar and drives only the
                // selected part.
                let mut cb_sels: Vec<&Expression> = Vec::new();
                let mut cb_base = lvalue;
                while let ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } =
                    &cb_base.kind
                {
                    cb_sels.push(cb_base);
                    cb_base = expr;
                }
                let cb_base_flat;
                if !cb_sels.is_empty()
                    && let Some(f) = Self::member_chain_as_flat_ident(cb_base)
                {
                    cb_base_flat = f;
                    cb_base = &cb_base_flat;
                }
                if let ExprKind::Ident(hier) = &cb_base.kind
                    && hier.path.len() >= 2 {
                        let cb_segs: Vec<&str> = hier
                            .path
                            .iter()
                            .take(hier.path.len() - 1)
                            .map(|s| s.name.name.as_str())
                            .collect();
                        let cb = self
                            .resolve_clocking_key(&cb_segs)
                            .unwrap_or_else(|| cb_segs.join("."));
                        let raw_sig = hier.path.last().unwrap().name.name.as_str();
                        let sig: &str = self
                            .clocking_sig_alias
                            .get(&cb)
                            .and_then(|m| m.get(raw_sig))
                            .map(|s| s.as_str())
                            .unwrap_or(raw_sig);
                        if let Some((_, sigs)) = self.clocking_meta.get(&cb) {
                            // §14.16.2: an `inout` clockvar is driven like an
                            // output (it is registered as an input for sampling).
                            let inouts = self.clocking_inouts.get(&cb);
                            let out_net = sigs.iter().find(|(n, is_in)| {
                                (!*is_in || inouts.is_some_and(|s| s.contains(n)))
                                    && (n == sig || n.rsplit('.').next() == Some(sig))
                            });
                            if let Some((net, _)) = out_net {
                                let net = net.clone();
                                // §14.16: a clocking output creates a driver on
                                // the net. Where the net has another driver, the
                                // clockvar drives a stage of its own and the
                                // stage joins the net's other drivers in the
                                // resolution fold — so the drive goes to the
                                // stage, not to the net.
                                let stage = self
                                    .clocking_output_stages
                                    .get(&(cb.to_string(), net.clone()))
                                    .cloned();
                                let drive_net = stage.clone().unwrap_or_else(|| net.clone());
                                let last_sels = &hier.path[hier.path.len() - 1].selects;
                                // §14.5: a clockvar bound to a select of the
                                // net drives through that select (#823).
                                let bound_sel = self
                                    .clocking_sig_bound_sel
                                    .get(&cb)
                                    .and_then(|m| m.get(raw_sig))
                                    .filter(|_| last_sels.is_empty())
                                    .cloned();
                                let target = if let Some(b) = bound_sel {
                                    let mut t = b;
                                    for layer in cb_sels.iter().rev() {
                                        let mut l = (*layer).clone();
                                        if let ExprKind::Index { expr, .. }
                                        | ExprKind::RangeSelect { expr, .. } = &mut l.kind
                                        {
                                            *expr = Box::new(t);
                                        }
                                        t = l;
                                    }
                                    Some(self.freeze_lvalue_indices(&t))
                                } else if cb_sels.is_empty() && last_sels.is_empty() {
                                    None
                                } else {
                                    let mut t = Expression::new(
                                        ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                                            root: None,
                                            path: vec![crate::ast::expr::HierPathSegment {
                                                name: crate::ast::Identifier {
                                                    name: drive_net.clone(),
                                                    span: lvalue.span,
                                                },
                                                selects: last_sels.clone(),
                                            }],
                                            span: lvalue.span,
                                            cached_signal_id: crate::ast::cache::SignalCache::new(None),
                                            cached_resolved_name: crate::ast::cache::NameCache::new(),
                                            bound: crate::ast::binding::Bound::UNSET,
                                        }),
                                        lvalue.span,
                                    );
                                    for layer in cb_sels.iter().rev() {
                                        let mut l = (*layer).clone();
                                        if let ExprKind::Index { expr, .. }
                                        | ExprKind::RangeSelect { expr, .. } = &mut l.kind
                                        {
                                            *expr = Box::new(t);
                                        }
                                        t = l;
                                    }
                                    Some(self.freeze_lvalue_indices(&t))
                                };
                                // §14.16 `##N`: wait N further clocking events.
                                if let Some(n) = intra_cycles
                                    && n > 0 {
                                        self.clocking_cycle_pending.entry(cb).or_default().push((
                                            n,
                                            raw_sig.to_string(),
                                            drive_net.clone(),
                                            val,
                                            target,
                                        ));
                                        return;
                                    }
                                // §14.16.1: a drive executed AT the clocking
                                // event matures at edge + OUTPUT SKEW (per-sig
                                // override, else block default, else #0 =
                                // Re-NBA of this timestep). Only a drive issued
                                // BETWEEN events waits for the next edge.
                                let skew = self
                                    .clocking_sig_out_skew
                                    .get(&cb)
                                    .and_then(|m| m.get(raw_sig).or_else(|| m.get(&net)))
                                    .copied()
                                    .or_else(|| self.clocking_out_skew.get(&cb).copied())
                                    .unwrap_or(0);
                                if self.clocking_last_edge.get(&cb).copied() == Some(self.time) {
                                    // §14.3: an edge skew drives on the next
                                    // such edge of the clock after the event.
                                    if let Some((edge, d)) =
                                        self.clocking_drive_edge(&cb, raw_sig, &net)
                                    {
                                        self.clocking_edge_out_pending
                                            .entry(cb)
                                            .or_default()
                                            .push((drive_net.clone(), val, edge, d, target));
                                        return;
                                    }
                                    if let Some(t) = &target {
                                        self.drive_clocking_select(t, val, skew);
                                        return;
                                    }
                                    if let Some(&id) = self.signal_name_to_id.get(drive_net.as_str()) {
                                        self.schedule_clocking_update(id, &val, skew);
                                        return;
                                    }
                                }
                                self.clocking_output_pending.entry(cb).or_default().push((
                                    raw_sig.to_string(),
                                    drive_net,
                                    val,
                                    target,
                                ));
                                return;
                            }
                        }
                    }
                // SV-2023: NBA to a ref formal redirects to the caller's
                // actual lvalue (IEEE 1800-2023 §13.5.2).
                let written = lvalue;
                let lvalue_owned;
                let lvalue: &Expression = if let ExprKind::Ident(hier) = &lvalue.kind {
                    if hier.path.len() == 1 && hier.path[0].selects.is_empty() {
                        let formal = &hier.path[0].name.name;
                        if let Some(actual) = self
                            .ref_binding_stack
                            .last()
                            .and_then(|m| m.get(formal))
                            .cloned()
                        {
                            lvalue_owned = actual;
                            &lvalue_owned
                        } else {
                            lvalue
                        }
                    } else {
                        lvalue
                    }
                } else {
                    lvalue
                };
                // The caller's actual of a `ref` formal names the caller's
                // variables, not this frame's.
                let rebound = !std::ptr::eq(lvalue, written);
                let w = self.infer_lhs_width(lvalue);
                let d = match intra_d {
                    Some(d) => d,
                    None => delay
                        .as_ref()
                        .map(|de| self.eval_expr(de).to_u64().unwrap_or(0))
                        .unwrap_or(0),
                };
                // §10.4.2 / §6.21: capture persistent storage before the
                // call frame is restored. Every write still commits in NBA.
                if let Some(key) = self.nba_static_key_for_lvalue(lvalue) {
                    self.enqueue_context_nba(
                        NbaEntry {
                            elements: None,
                            lhs: None,
                            value: val.resize_for_assign(w),
                            resolved_id: None,
                            static_key: Some(key),
                            static_slice: None,
                            this_handle: None,
                            root_frame: self.nba_root_frame(lvalue),
                        },
                        d,
                    );
                    return;
                }
                if !rebound && self.try_schedule_static_packed_nba(packed_lvalue, &val, d) {
                    return;
                }
                if d == 0 {
                    let id_opt = self.resolve_nba_target(lvalue);
                    if let Some(id) = id_opt
                        && self.nba_behind_queued_partial(id)
                    {
                        // §4.6 b): a partial update to this signal is already
                        // queued; `nba_fast` would commit this one first.
                        self.nba_queue.push(NbaEntry {
                            elements: None,
                            lhs: None,
                            value: val.resize_for_assign(w),
                            resolved_id: Some(id),
                            static_key: None,
                            static_slice: None,
                            this_handle: None,
                            root_frame: None,
                        });
                    } else if let Some(id) = id_opt {
                        // Track via index map so subsequent partial-NBAs
                        // (NbaAssignRange / NbaAssignBitDyn from the
                        // bytecode VM) targeting the same signal merge into
                        // this entry rather than re-seeding from
                        // signal_table — same invariant as the bytecode
                        // NbaAssign Insn.
                        self.nba_fast_index.insert(id, self.nba_fast.len());
                        self.nba_fast.push(NbaFast {
                            block_index: 0,
                            signal_id: id,
                            value: val.resize_for_assign(w),
                        });
                    } else if self.try_schedule_packed_index_nba(lvalue, &val) {
                        // The packed slice has already been merged into the
                        // pending whole-signal value.
                    } else if matches!(&lvalue.kind, ExprKind::Ident(h)
                        if h.path.len() == 1
                            && self.local_stack.last().is_some_and(|m| m.contains_key(&h.path[0].name.name)))
                    {
                        // Preserve the existing whole-frame fallback.
                        // Persistent local declarations were captured above.
                        self.assign_value(lvalue, &val.resize_for_assign(w));
                    } else {
                        let frozen = self.freeze_lvalue_indices(lvalue);
                        let qval = if self.nba_target_is_assoc_elem(lvalue) {
                            val.clone()
                        } else {
                            val.resize_for_assign(w)
                        };
                        let static_key = self.nba_static_key_for_lvalue(lvalue);
                        let this_handle = self.this_stack.last().copied().flatten();
                        self.nba_queue.push(NbaEntry {
                            elements: None,
                            lhs: Some(frozen),
                            value: qval,
                            resolved_id: None,
                            static_key,
                            static_slice: None,
                            this_handle,
                            root_frame: self.nba_root_frame(lvalue),
                        });
                    }
                } else if let Some(id) = self.resolve_nba_target(lvalue) {
                    // Delayed NBA is transport, not inertial: push directly
                    // so pending same-signal updates are NOT cancelled.
                    // §4.9.4: matures in the target slot's NBA REGION — the
                    // separate queue commits inside apply_nba, so Active
                    // reads at the same time still see the old value.
                    let mut v = val.resize_for_assign(w);
                    v.is_signed = self.signal_signed[id];
                    self.delayed_nba.push((self.time + d, id, v));
                } else {
                    let frozen = self.freeze_lvalue_indices(lvalue);
                    let qval = if self.nba_target_is_assoc_elem(lvalue) {
                        val.clone()
                    } else {
                        val.resize_for_assign(w)
                    };
                    let static_key = self.nba_static_key_for_lvalue(lvalue);
                    let this_handle = self.this_stack.last().copied().flatten();
                    self.enqueue_context_nba(
                        NbaEntry {
                            elements: None,
                            lhs: Some(frozen),
                            value: qval,
                            resolved_id: None,
                            static_key,
                            static_slice: None,
                            this_handle,
                            root_frame: self.nba_root_frame(lvalue),
                        },
                        d,
                    );
                }
            }
            StatementKind::If {
                condition,
                then_stmt,
                else_stmt,
                ..
            } => {
                // §12.6: `if (e matches p) ...` — the pattern's `.name` bindings
                // are visible in the then-branch. The parser used to discard the
                // pattern and yield a literal 0, so the branch never ran.
                if let ExprKind::Matches { expr, pattern } = &condition.kind {
                    let mut binds: Vec<(String, Value)> = Vec::new();
                    if self.match_pattern(expr, pattern, &mut binds) {
                        let st = self.push_pattern_bindings(&binds);
                        self.exec_statement(then_stmt);
                        self.pop_pattern_bindings(st);
                    } else if let Some(el) = else_stmt {
                        self.exec_statement(el);
                    }
                    return;
                }
                if self.eval_expr(condition).is_true() {
                    self.exec_statement(then_stmt);
                } else if let Some(el) = else_stmt {
                    self.exec_statement(el);
                }
            }
            StatementKind::Case {
                unique_priority,
                kind,
                expr,
                items,
            } => {
                // IEEE 1800-2017 §12.6.1 pattern case:
                //   case (expr) matches <pattern> [&&& guard] : stmt ... endcase
                // Distinguished by items carrying a `pattern`.
                if items.iter().any(|i| i.pattern.is_some()) {
                    self.exec_case_matches(expr, items);
                    return;
                }
                let comparison = self.case_comparison(kind, expr, items);
                // SV-2023: under unique/unique0/priority, pre-pass the items
                // and emit a violation message if multiple match or (for
                // unique/priority) none match without a default.
                if sv_parser::is_sv2023()
                    && unique_priority.is_some()
                    && self.unique_check_enabled(unique_priority.as_ref())
                {
                    let mut match_count = 0usize;
                    let mut has_default = false;
                    for item in items.iter() {
                        if item.is_default {
                            has_default = true;
                            continue;
                        }
                        for pat in &item.patterns {
                            if self.case_item_matches(kind, &comparison, pat) {
                                match_count += 1;
                                break;
                            }
                        }
                    }
                    let qual = match unique_priority {
                        Some(UniquePriority::Unique) => "unique",
                        Some(UniquePriority::Unique0) => "unique0",
                        Some(UniquePriority::Priority) => "priority",
                        None => "",
                    };
                    if match_count > 1 {
                        self.record_output(format!(
                            "[SV-2023 §12.5.3] {} case: multiple items matched ({})",
                            qual, match_count
                        ));
                    } else if match_count == 0
                        && !has_default
                        && matches!(
                            unique_priority,
                            Some(UniquePriority::Unique) | Some(UniquePriority::Priority)
                        )
                    {
                        self.record_output(format!(
                            "[SV-2023 §12.5.3] {} case: no item matched and no default",
                            qual
                        ));
                    }
                }
                let mut matched = false;
                for item in items.iter() {
                    if item.is_default {
                        continue;
                    }
                    for pat in &item.patterns {
                        if self.case_item_matches(kind, &comparison, pat) {
                            self.exec_statement(&item.stmt);
                            matched = true;
                            break;
                        }
                    }
                    if matched {
                        break;
                    }
                }
                if !matched {
                    for item in items {
                        if item.is_default {
                            self.exec_statement(&item.stmt);
                            break;
                        }
                    }
                }
            }
            StatementKind::For {
                init,
                condition,
                step,
                body,
            } => {
                if self.try_exec_full_packed_fill(init, condition.as_ref(), step, body) {
                    return;
                }
                // §12.7 / §6.21: a for-loop-declared variable is automatic and
                // scoped to the loop — it must SHADOW any same-named outer
                // signal, not clobber it. Without a local frame the loop var
                // would land on the outer signal (wrong width + destroyed
                // value: `logic idx` outer vs `for (int idx...)` spun forever on
                // a 1-bit counter). Push a throwaway frame so the loop var gets
                // its own declared-width storage; pop it at the arm's end.
                // §12.7.1 / §6.21: for-init-declared variables are scoped to
                // the loop — like foreach index vars, save the outer value and
                // restore it on exit. Without this, a comb block's
                // `for (int i...)` executed by a settle triggered MID-ITERATION
                // of another block's same-named loop (blocking assigns settle
                // synchronously) clobbered the interrupted loop's counter, and
                // that loop silently exited after one iteration.
                let fv_names: Vec<String> = init
                    .iter()
                    .filter_map(|fi| match fi {
                        ForInit::VarDecl { name, .. } => Some(name.name.clone()),
                        _ => None,
                    })
                    .collect();
                let fv_saved = self.snapshot_loop_vars(&fv_names);
                // An init-declared loop variable is automatic and loop-scoped
                // (§12.7/§6.21): it always gets its own frame when the
                // process has none. Probing for same-named outer signals only
                // caught the module-scope / process-scope shapes (u.i) — an
                // INLINED child instance's same-named leaf is invisible to the
                // probe, and the leaf-name fallback in name resolution then
                // bound the bare reads to the child's storage while the writes
                // landed on a bare signal: two different cells (IEEE 1800
                // §23.6 — an unqualified name must not resolve downward
                // into an instance). Pushing unconditionally is cheap and
                // closes every downward-collision shape.
                let shadow_frame = self.local_stack.last().is_none()
                    && init.iter().any(|fi| matches!(fi, ForInit::VarDecl { .. }));
                if shadow_frame {
                    self.push_loop_frame();
                }
                // Init-declared loop vars are automatic. With the frame push
                // above they always live in a local frame, never in the bare
                // signal table, so a fork in the body captures the
                // per-iteration value from the frame copy that
                // inherit_fork_child_context gives each child (LRM §9.3.2).
                let mut hidden_statics = None;
                for fi in init {
                    match fi {
                        ForInit::VarDecl {
                            data_type,
                            name,
                            init: e,
                        } => {
                            let rv = self.declare_for_init_var(data_type, &name.name, e);
                            // Declare the loop variable in the current call
                            // frame's local scope (highest read/write
                            // precedence) so its init, the step, and condition
                            // reads all agree — and so it properly *shadows* any
                            // same-named class member (riscv-dv reuses `hart`
                            // both as a field and as a loop variable; without
                            // shadowing the field's stale value made the second
                            // `for (int hart...)` never iterate).
                            if let Some(frame) = self.local_stack.last_mut() {
                                frame.insert(name.name.clone(), rv);
                                if let Some(aliases) = self
                                    .local_iface_aliases
                                    .last_mut()
                                    .and_then(Option::as_deref_mut)
                                {
                                    aliases.bindings.remove(&name.name);
                                }
                            } else {
                                self.set_signal_value_by_name(&name.name, rv);
                            }
                            self.hide_loop_static(&name.name, &mut hidden_statics);
                        }
                        ForInit::Assign { lvalue, rvalue } => {
                            let v = self.eval_expr(rvalue);
                            self.assign_value(lvalue, &v);
                        }
                    }
                }
                // Loop vars declared signed (`for (int i = ...)`): the step
                // (`--i`, `i = i-1`) can write back an unsigned result, which
                // would make `i >= 0` always-true and spin a descending loop.
                // Re-assert their signedness after each step (cheap, only for
                // signed-typed for-decl vars).
                let signed_loop_vars: Vec<String> = init
                    .iter()
                    .filter_map(|fi| match fi {
                        ForInit::VarDecl {
                            data_type, name, ..
                        } if self.type_is_signed_concrete(data_type) => Some(name.name.clone()),
                        _ => None,
                    })
                    .collect();
                static LOOP_LIMIT_ENV: std::sync::OnceLock<Option<u64>> =
                    std::sync::OnceLock::new();
                let loop_limit: u64 = (*LOOP_LIMIT_ENV.get_or_init(|| {
                    std::env::var("XEZIM_LOOP_LIMIT")
                        .ok()
                        .and_then(|s| s.parse().ok())
                }))
                .unwrap_or(10_000_000);
                let between: Vec<&Expression> = condition.iter().chain(step.iter()).collect();
                let body_scope = self.loop_scope_open(body, &between, &fv_names);
                let mut iters: u64 = 0;
                loop {
                    if iters > loop_limit || self.finished {
                        break;
                    }
                    iters += 1;
                    if iters & LOOP_INTERRUPT_POLL_MASK == 0 && loop_stop_requested() {
                        self.take_interrupt();
                        break;
                    }
                    if let Some(c) = condition {
                        self.reset_hint_to_process_scope();
                        if !self.eval_expr(c).is_true() {
                            break;
                        }
                    }
                    self.break_flag = false;
                    self.continue_flag = false;
                    self.exec_statement(body);
                    if self.break_flag {
                        // Keep the flag set while a `disable` unwinds to an
                        // enclosing named block.
                        if self.disable_target.is_none() {
                            self.break_flag = false;
                        }
                        break;
                    }
                    self.continue_flag = false;
                    for s in step {
                        self.exec_expr_stmt(s);
                    }
                    for nm in &signed_loop_vars {
                        if let Some(frame) = self.local_stack.last_mut() {
                            if let Some(v) = frame.get_mut(nm) {
                                v.is_signed = true;
                                continue;
                            }
                        }
                        if let Some(v) = self.signals.get_mut(nm) {
                            v.is_signed = true;
                        }
                    }
                }
                self.loop_scope_close(body, body_scope);
                if shadow_frame {
                    self.pop_loop_frame();
                }
                self.unhide_statics(hidden_statics);
                self.restore_loop_vars(&fv_saved);
                // class-perf loop-heat boost (pass C): one call per loop exit.
                self.record_ast_loop_heat(iters);
            }
            // A foreach index variable is automatic and loop-scoped (§12.7.3):
            // like a for-init declaration it always gets its own frame when
            // the process has none. The old same-named-signal probe only
            // caught module-scope / process-scope shapes (u.i); an INLINED
            // child instance's same-named leaf was invisible to it, and the
            // leaf-name fallback then bound the bare index reads to the
            // child's storage while set_loop_var wrote a bare signal — two
            // different cells (IEEE 1800 §23.6). The re-dispatch below
            // sees a local frame and takes the normal path, so every exit of
            // that path pops nothing extra.
            StatementKind::Foreach { vars, .. }
                if self.local_stack.last().is_none() && vars.iter().any(|v| v.is_some()) =>
            {
                self.push_loop_frame();
                self.exec_statement(stmt);
                self.pop_loop_frame();
            }
            StatementKind::Foreach { array, vars, body } => {
                let names: Vec<String> = vars
                    .iter()
                    .filter_map(|v| v.as_ref().map(|id| id.name.clone()))
                    .collect();
                let body_scope = self.loop_scope_open(body, &[array], &names);
                self.exec_stmt_foreach(stmt, array, vars, body);
                self.loop_scope_close(body, body_scope);
            }
            StatementKind::While { condition, body } => {
                static LOOP_LIMIT_ENV: std::sync::OnceLock<Option<u64>> =
                    std::sync::OnceLock::new();
                let loop_limit: u64 = (*LOOP_LIMIT_ENV.get_or_init(|| {
                    std::env::var("XEZIM_LOOP_LIMIT")
                        .ok()
                        .and_then(|s| s.parse().ok())
                }))
                .unwrap_or(10_000_000);
                let body_scope = self.loop_scope_open(body, &[condition], &[]);
                let mut i: u64 = 0;
                loop {
                    if i > loop_limit || self.finished {
                        break;
                    }
                    i += 1;
                    if i & LOOP_INTERRUPT_POLL_MASK == 0 && loop_stop_requested() {
                        self.take_interrupt();
                        break;
                    }
                    self.reset_hint_to_process_scope();
                    if !self.eval_expr(condition).is_true() {
                        // §12.7.2: a `continue` on the FINAL iteration reaches
                        // this exit with the flag still set (the `for` arm
                        // clears it after the body; `while` re-tests the
                        // condition first). Leaked, it skips every statement
                        // after the loop — a function ends without executing
                        // its `return` and the caller reads a stale slot.
                        self.continue_flag = false;
                        break;
                    }
                    self.break_flag = false;
                    // §12.7.2: clear `continue` before each iteration, as the
                    // `for` loop already did. Left set, the body's very first
                    // statement saw the stale flag and skipped the whole block
                    // every time — the loop variable never advanced and a
                    // `while` containing a `continue` spun forever.
                    self.continue_flag = false;
                    self.exec_statement(body);
                    if self.break_flag {
                        // Keep the flag set while a `disable` unwinds to an
                        // enclosing named block.
                        if self.disable_target.is_none() {
                            self.break_flag = false;
                        }
                        break;
                    }
                }
                self.loop_scope_close(body, body_scope);
                // class-perf loop-heat boost (pass C): one call per loop exit.
                self.record_ast_loop_heat(i);
            }
            StatementKind::DoWhile { body, condition } => {
                static LOOP_LIMIT_ENV: std::sync::OnceLock<Option<u64>> =
                    std::sync::OnceLock::new();
                let loop_limit: u64 = (*LOOP_LIMIT_ENV.get_or_init(|| {
                    std::env::var("XEZIM_LOOP_LIMIT")
                        .ok()
                        .and_then(|s| s.parse().ok())
                }))
                .unwrap_or(10_000_000);
                let body_scope = self.loop_scope_open(body, &[condition], &[]);
                let mut i: u64 = 0;
                loop {
                    if i > loop_limit || self.finished {
                        break;
                    }
                    i += 1;
                    if i & LOOP_INTERRUPT_POLL_MASK == 0 && loop_stop_requested() {
                        self.take_interrupt();
                        break;
                    }
                    self.break_flag = false;
                    self.continue_flag = false;
                    self.exec_statement(body);
                    if self.break_flag {
                        // Keep the flag set while a `disable` unwinds to an
                        // enclosing named block.
                        if self.disable_target.is_none() {
                            self.break_flag = false;
                        }
                        break;
                    }
                    self.continue_flag = false;
                    if !self.eval_expr(condition).is_true() {
                        break;
                    }
                }
                self.loop_scope_close(body, body_scope);
                // class-perf loop-heat boost (pass C): one call per loop exit.
                self.record_ast_loop_heat(i);
            }
            StatementKind::Repeat { count, body } => {
                // §12.7.2: full count — the old 10000 cap silently truncated
                // large loops (and negatives-as-unsigned hit it constantly).
                let n = self.repeat_count(count);
                let body_scope = self.loop_scope_open(body, &[], &[]);
                let mut rp_it: u64 = 0;
                for k in 0..n {
                    if self.finished {
                        break;
                    }
                    if k & LOOP_INTERRUPT_POLL_MASK == LOOP_INTERRUPT_POLL_MASK
                        && loop_stop_requested()
                    {
                        self.take_interrupt();
                        break;
                    }
                    self.break_flag = false;
                    self.continue_flag = false;
                    rp_it += 1;
                    self.exec_statement(body);
                    if self.break_flag {
                        // Keep the flag set while a `disable` unwinds to an
                        // enclosing named block.
                        if self.disable_target.is_none() {
                            self.break_flag = false;
                        }
                        break;
                    }
                    self.continue_flag = false;
                }
                self.loop_scope_close(body, body_scope);
                // class-perf loop-heat boost (pass C): one call per loop exit.
                self.record_ast_loop_heat(rp_it);
            }
            StatementKind::Forever { body } => {
                let mut i: u64 = 0;
                loop {
                    if i > 100000 || self.finished || self.time > self.max_time {
                        break;
                    }
                    i += 1;
                    if i & LOOP_INTERRUPT_POLL_MASK == 0 && loop_stop_requested() {
                        self.take_interrupt();
                        break;
                    }
                    // §9.3.3 / §12.7.2 loop control, exactly as `while` does
                    // above. This arm honoured NO flag: a `break` left the
                    // body no-opping for the rest of the cap, and — the real
                    // damage — the flag SURVIVED the loop, so every statement
                    // after the `forever` (including `$finish`) was skipped
                    // and the run looked like a hang. Only reachable when the
                    // `forever` is nested inside a block, which is why a
                    // top-level one always worked.
                    self.break_flag = false;
                    self.continue_flag = false;
                    self.exec_statement(body);
                    if self.break_flag {
                        // Keep the flag set while a `disable` unwinds to an
                        // enclosing named block.
                        if self.disable_target.is_none() {
                            self.break_flag = false;
                        }
                        break;
                    }
                }
                // class-perf loop-heat boost (pass C): one call per loop exit.
                self.record_ast_loop_heat(i);
            }
            StatementKind::SeqBlock { stmts, name } => {
                // §9.3.1: a labelled procedural assertion is this block. A
                // `--code-coverage` statement marker beside it is not a
                // statement of the design: counted, it dropped the label of
                // every instrumented one (an `initial` block's, #586).
                if let Some(n) = name {
                    let mut real = stmts.iter().filter(|s| !Self::is_cov_marker(s));
                    if matches!(real.next(), Some(s) if matches!(s.kind, StatementKind::Assertion(_)))
                        && real.next().is_none()
                    {
                        self.assert_wrap_label = Some(n.name.clone());
                    }
                }
                // §6.21: a block-local declaration SHADOWS a same-named module
                // signal — without a frame the frameless VarDecl exec lands on
                // the signal itself and the block's writes clobber the module
                // variable (`begin logic [7:0] v; v = 8'hF0; end` overwrote
                // module `v`). Push a throwaway frame for the block's duration,
                // exactly like the for-loop shadow frame. Only when a direct
                // child declaration actually collides, so the common case pays
                // one scan and no allocation.
                let shadow_frame = self.local_stack.last().is_none()
                    && stmts.iter().any(|s| match &s.kind {
                        StatementKind::VarDecl { declarators, .. }
                        | StatementKind::ParamDecl { declarators, .. } => declarators
                            .iter()
                            .any(|d| self.signal_name_to_id.contains_key(d.name.name.as_str())),
                        _ => false,
                    });
                // With a call frame, the block's declarations land in it:
                // open the block's scope so they end with it (§6.21), unless
                // the loop running this body holds it open.
                let scope = if self.hoisted_scope.0 == stmt as *const Statement as usize
                    && self.local_gen_stack.last().copied() == Some(self.hoisted_scope.1)
                {
                    None
                } else {
                    self.block_scope_enter(stmts)
                };
                if shadow_frame {
                    self.push_local_frame(HashMap::default());
                }
                // `automatic` locals declared in this block (see the VarDecl
                // arm) are fork-capturable only while the block is in scope
                // (§6.21): each entry — e.g. each loop iteration — is a fresh
                // variable, so pop them on exit.
                let seq_auto_len = self.auto_loop_vars.len();
                // §21.2.1.7: a NAMED block adds its name to the `%m` hierarchy.
                let m_pushed = if let Some(n) = name {
                    self.m_scope_stack.push(n.name.clone());
                    true
                } else {
                    false
                };
                for s in stmts {
                    if self.finished || self.break_flag || self.continue_flag {
                        break;
                    }
                    self.exec_statement(s);
                }
                if m_pushed {
                    self.m_scope_stack.pop();
                }
                if shadow_frame {
                    self.pop_local_frame();
                }
                if let Some(sv) = scope {
                    self.block_scope_exit(stmts, sv);
                }
                self.auto_loop_vars.truncate(seq_auto_len);
                // A `disable` naming THIS block ends here; execution resumes
                // after it (§9.6.2).
                if let (Some(target), Some(n)) = (self.disable_target.as_deref(), name.as_ref()) {
                    if Self::block_name_matches(target, &n.name) {
                        self.disable_target = None;
                        self.disable_check_pending = false;
                        self.break_flag = false;
                        self.continue_flag = false;
                    }
                }
            }
            StatementKind::ParBlock {
                stmts,
                join_type,
                name: block_name,
                ..
            } => {
                // Mirror the suspend-aware path (run_process_stmts): children are
                // parented to the CURRENT pid (not 0) and inherit a *snapshot* of
                // the forking process's context. The snapshot captures automatic
                // variables (e.g. a loop index) by value at fork time, so the
                // canonical `for (int i…) fork … join_none` idiom captures the
                // per-iteration `i` instead of the parent's post-loop value
                // (LRM §9.3.2 automatic-variable semantics).
                // As in run_process_stmts: fork-block declarations are not
                // processes; run them here so each child snapshots them.
                let (spawnable, saved_auto_len) = self.exec_fork_block_decls(stmts);
                let mut child_set = HashSet::default();
                for s in spawnable {
                    let pid = self.next_pid;
                    self.next_pid += 1;
                    self.process_parents.insert(pid, self.current_pid);
                    // Same scope-hint inheritance as the run_process_stmts
                    // ParBlock arm — §9.3.2, additive.
                    if let Some(h) = self.process_scope_hint.get(&self.current_pid).cloned() {
                        self.process_scope_hint.insert(pid, h);
                    }
                    self.process_origin.insert(pid, (s.span, FORK_CHILD_ORIGIN));
                    self.inherit_fork_child_context(pid);
                    self.inherit_fork_m_scope(pid, self.current_pid, block_name.as_ref());
                    let frame = self.fork_child_frame(s, stmts);
                    self.event_queue
                        .schedule(self.time, pid, ProcCont::from_frame(frame));
                    child_set.insert(pid);
                }
                self.auto_loop_vars.truncate(saved_auto_len);
                if let Some(nm) = block_name {
                    self.register_named_fork(&nm.name, &child_set);
                }
                match join_type {
                    // `join` waits for all, `join_any` for the first; both suspend
                    // the forking process via a JoinWaiter. `join_none` continues.
                    // An empty fork has no children to wait for, so it must NOT
                    // suspend — a childless waiter is never re-checked.
                    // A task entered from C (DPI export) cannot park: run the
                    // scheduler until the join is satisfied, then carry on with
                    // the statements after the fork (#204).
                    JoinType::Join | JoinType::JoinAny
                        if !child_set.is_empty() && self.dpi_export_depth > 0 =>
                    {
                        let (id, wpid, wake) = self.sync_wait_target();
                        self.join_waiters.push(JoinWaiter {
                            parent_pid: wpid,
                            child_pids: child_set,
                            join_type: *join_type,
                            continuation: wake,
                            finished_children: HashSet::default(),
                            wait_fork: false,
                        });
                        self.sync_wait(id);
                    }
                    JoinType::Join | JoinType::JoinAny if !child_set.is_empty() => {
                        self.join_waiters.push(JoinWaiter {
                            parent_pid: self.current_pid,
                            child_pids: child_set,
                            join_type: *join_type,
                            continuation: Vec::new().into(),
                            finished_children: HashSet::default(),
                            wait_fork: false,
                        });
                        self.break_flag = true; // Suspend current execution
                    }
                    _ => {}
                }
            }
            StatementKind::TimingControl { control, stmt } => {
                match control {
                    // §35.5.2: a delay inside an imported task running on its
                    // own stack parks the calling process (dpi_task.rs).
                    TimingControl::Delay(d) if self.dpi_cur_task.is_some() => {
                        self.dpi_wait_delay(d);
                        self.exec_statement(stmt);
                    }
                    TimingControl::Delay(d) => {
                        // The nested work below (run_events_until, settle,
                        // edge cascades) executes OTHER processes and comb
                        // entries whose dotted-name resolutions leave a sticky
                        // `name_resolve_hint` behind. That residue must not
                        // leak back into THIS process's bare-name resolution:
                        // a leaked sibling-instance hint once made a
                        // testbench's loop variable `c` shadow-resolve to the
                        // DUT instance's `c` (then the per-node cache froze
                        // it and the loop never terminated). Save/restore the
                        // hint across the whole nested-delay window.
                        let saved_hint_nested = self.name_resolve_hint.borrow().clone();
                        let delay = self.eval_delay_ticks(d);
                        // §4.4/§4.5: the NBA region of the CURRENT timestamp
                        // runs after every active-region process at that
                        // timestamp has evaluated its RHS. Flushing here is
                        // premature when this delay is inside an EDGE block:
                        // the remaining blocks on the same edge have not
                        // sampled yet, and committing now lets them read the
                        // post-edge value. A `always @(posedge clk) begin #1;
                        // … end` observer anywhere in a testbench therefore
                        // broke every OTHER clocked block that sampled a
                        // signal written by a sibling block on that edge —
                        // `d <= dut.if.sig;` recorded the new value, so a
                        // one-cycle pipeline check saw no delay at all.
                        // Outside an edge block (a plain initial/task `#d`)
                        // keep flushing: the active region there is just this
                        // process, and `run_events_until` below re-flushes at
                        // the proper point either way.
                        if !self.in_edge_block {
                            self.apply_nba();
                        }
                        self.settle_combinatorial();
                        self.check_edges();
                        let _ = self.drain_edge_cascade(self.cascade_limit);
                        self.snapshot_edge_signals();
                        let target = self.time + delay;
                        // Run scheduled continuations (other processes,
                        // clock toggles, delayed updates) whose fire time
                        // falls inside this delay window so concurrent
                        // blocks advance while we're sleeping in a task.
                        self.run_events_until(target);
                        if self.time < target {
                            self.time = target;
                            publish_sim_time(target);
                        }
                        // Full active→inactive→NBA traversal at the new
                        // time, per LRM §4.4: a delayed update or clocked
                        // block scheduled AT target itself must fire before
                        // the sleeping process resumes.
                        if self.apply_delayed_updates() {
                            self.dirty_any = true;
                        }
                        if self
                            .clock_generators
                            .iter()
                            .any(|c| c.next_toggle_time == self.time)
                        {
                            self.fire_clock_generators();
                            self.dirty_any = true;
                        }
                        if !self.nba_fast.is_empty()
            || !self.nba_queue.is_empty()
            // §15.5.2: a bare `->>` (deferred trigger) must flush with the NBA
            // region even when no NBA DATA is queued.
            || !self.pending_nba_triggers.is_empty()
                || !self.packed_nba.is_empty()
            || !self.pending_nba_instance_triggers.is_empty()
                        {
                            self.apply_nba();
                        }
                        self.settle_combinatorial();
                        self.check_edges();
                        let _ = self.drain_edge_cascade(self.cascade_limit);
                        self.snapshot_edge_signals();
                        // The whole postponed region, not just `$monitor`. This
                        // slot is reached by jumping `self.time` to the delay's
                        // target, and the dump was left out — so the monitor
                        // reported the slot and the waveform did not, and the
                        // changes surfaced later at whatever tick next wrote the
                        // dump. That is why a monitor and a VCD of the same
                        // event disagreed with each other as well as with the
                        // change itself.
                        self.run_postponed_region();
                        *self.name_resolve_hint.borrow_mut() = saved_hint_nested;
                        // The activation goes on after a delay, not at an event control.
                        if !self.settling && delay > 0 {
                            self.delay_resumed.insert(self.current_pid);
                        }
                    }
                    // A task entered from C (DPI export) cannot park: register
                    // the waiter on a wake marker and run the scheduler until
                    // the event fires; the body then runs below (#204).
                    TimingControl::Event(e) if self.dpi_export_depth > 0 => {
                        let (id, wpid, wake) = self.sync_wait_target();
                        let mut key = None;
                        if let Some(fname) = self.event_control_field_name(e) {
                            key = self.resolve_this_event_field(&fname);
                        }
                        if key.is_none() {
                            if let Some(expr) = Self::event_control_single_expr(e) {
                                key = self.expr_handle_event_field(&expr);
                                if key.is_none() {
                                    key = self.expr_instance_event_field_general(&expr);
                                }
                            }
                        }
                        if let Some(key) = key {
                            self.instance_event_waiters.push(InstanceEventWaiter {
                                key: InstanceEventKeys::One(key),
                                pid: wpid,
                                continuation: wake,
                            });
                        } else {
                            let sens = self.event_to_sens(e);
                            let is_clk_ev = self.is_clocking_event(e);
                            let w = self.make_event_waiter_kind(wpid, sens, wake, is_clk_ev);
                            self.park_event_waiter(w);
                        }
                        self.sync_wait(id);
                    }
                    TimingControl::Event(e) => {
                        // Class-field named event parked from the synchronous
                        // executor (a `fork ... join_none` body executing
                        // inline, e.g. a heartbeat start function): route to the
                        // per-instance waiter list so `->m_event` can wake it.
                        // Same rationale as the run_process_stmts arm.
                        if let Some(fname) = self.event_control_field_name(e) {
                            if let Some(key) = self.resolve_this_event_field(&fname) {
                                let cont = vec![*stmt.clone()];
                                let pid = self.current_pid;
                                self.instance_event_waiters.push(InstanceEventWaiter {
                                    key: InstanceEventKeys::One(key),
                                    pid,
                                    continuation: cont.into(),
                                });
                                self.break_flag = true;
                                return;
                            }
                        }
                        // Member-reached event property (`@(h.ce)`,
                        // `@(m_events[obj].all_dropped)`) — same routing as the
                        // run_process_stmts arm; without it these fell into the
                        // generic waiter below with a sensitivity that never
                        // matches an instance event.
                        if let Some(expr) = Self::event_control_single_expr(e) {
                            if let Some(key) = self
                                .expr_handle_event_field(&expr)
                                .or_else(|| self.expr_instance_event_field_general(&expr))
                            {
                                let cont = vec![*stmt.clone()];
                                let pid = self.current_pid;
                                self.instance_event_waiters.push(InstanceEventWaiter {
                                    key: InstanceEventKeys::One(key),
                                    pid,
                                    continuation: cont.into(),
                                });
                                self.break_flag = true;
                                return;
                            }
                        }
                        // `@(vif)` — see `vif_value_change_cond`.
                        if let Some(cond) = self.vif_value_change_cond(e) {
                            let pid = self.current_pid;
                            let wait = Self::vc_wait_stmt(cond.clone(), stmt, stmt.span);
                            self.park_condition_waiter(pid, vec![wait].into(), &cond);
                            self.break_flag = true;
                            return;
                        }
                        if let Some(cond) = self.class_prop_value_change_cond(e) {
                            let pid = self.current_pid;
                            let wait = Self::vc_wait_stmt(cond.clone(), stmt, stmt.span);
                            self.park_condition_waiter(pid, vec![wait].into(), &cond);
                            self.break_flag = true;
                            return;
                        }
                        // `@(posedge prop)` on a non-signal operand — see
                        // `class_prop_edge_cond`.
                        if let Some(cond) = self.class_prop_edge_cond(e) {
                            let pid = self.current_pid;
                            let wait = Self::vc_wait_stmt(cond.clone(), stmt, stmt.span);
                            self.park_condition_waiter(pid, vec![wait].into(), &cond);
                            self.break_flag = true;
                            return;
                        }
                        let sens = self.event_to_sens(e);
                        let is_clk_ev = self.is_clocking_event(e);
                        sim_dbg_eprintln!(
                            "[DEBUG] process {} waiting for event {:?} at time {}",
                            self.current_pid,
                            sens,
                            self.time
                        );
                        // Suspension is handled by run_process_stmts for top-level timing controls.
                        // If we are here, it's a nested timing control which we don't fully support yet.
                        let cont = vec![*stmt.clone()];
                        let pid = self.cg_this.unwrap_or(0); // placeholder
                        {
                            let w = self.make_event_waiter_kind(pid, sens, cont.into(), is_clk_ev);
                            self.park_event_waiter(w);
                        }
                        self.break_flag = true;
                        return;
                    }
                }
                self.exec_statement(stmt);
                // After body executes, check for edges (e.g., clk toggled)
                // and drain any cascade — see drain_edge_cascade.
                self.settle_combinatorial();
                self.check_edges();
                let _ = self.drain_edge_cascade(self.cascade_limit);
            }
            StatementKind::Break => {
                self.break_flag = true;
            }
            StatementKind::Continue => {
                self.continue_flag = true;
            }
            StatementKind::Return(expr) => {
                if let Some(rhs) = expr
                    && let Some(selected) = self.selected_aggregate_rhs(None, rhs)
                {
                    self.exec_statement(&Statement::new(
                        StatementKind::Return(Some(selected)),
                        stmt.span,
                    ));
                    return;
                }
                // §10.9, §13.4.1: a pattern returned as an array or queue of
                // unpacked structs is built in storage of the return type.
                if let Some(rhs) = expr
                    && self.return_pattern_in_storage(rhs, stmt.span)
                {
                    return;
                }

                if let Some(e) = expr {
                    // Parentheses do not change an unpacked return's shape.
                    // Recognize the collection before falling back to a
                    // scalar Value (which would turn an empty queue into {0}).
                    let e = Self::unparen(e);
                    // One call for a receiver call in the returned chain
                    // (`return me().o.x;`), shared by the probes below.
                    let pinned = Self::recv_chain_call(e)
                        .is_some()
                        .then(|| self.pin_stmt_recv_calls(&[e]))
                        .flatten();
                    // An operand no virtual-interface source can name: both
                    // vif lookups below come back empty for it.
                    let vif_free = !self.vif_rhs_possible(e);
                    // §25.9: `return val` where val carries a vif binding —
                    // record it so the caller's `x = f()` re-binds x
                    // (uvm_resource#(virtual if)::read; issue #113).
                    if let Some(nm) = (!vif_free)
                        .then(|| self.resolve_vif_rhs_name_strict(e))
                        .flatten()
                    {
                        self.pending_vif_return = Some(nm);
                    }
                    // §13.4: `return <collection>` — record the collection's
                    // storage name so the caller's assign can copy elements
                    // (a packed Value cannot carry them). Values a compiled
                    // callee left in `vm_ret_vals` belong to an earlier call.
                    self.pending_ret_collection = None;
                    self.vm_ret_vals = None;
                    // `instance_assoc_member` of a one-segment name, when
                    // computed below: it is also `expr_assoc_name`'s answer.
                    let mut bare_member: Option<Option<String>> = None;
                    if let ExprKind::Ident(h) = &e.kind {
                        let bare = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                        // A queue / dynamic-array member lives at `<handle>#member`,
                        // not under the bare name that `resolve_hier_name`
                        // returns (its per-node cache doesn't consult
                        // `instance_assoc_member`). Resolve the instance-scoped
                        // name explicitly so `pending_ret_collection` captures
                        // the real storage.
                        // A subroutine-LOCAL dyn array/queue lives under its
                        // process-unique rename (`declare_local_dyn`), so
                        // `return tmp;` must resolve through the frame's
                        // rename first — the bare name isn't in
                        // `dynamic_arrays` (ivtest sv_darray_function).
                        //
                        // §8.9/§10 scope: an unqualified name in a method
                        // body resolves to (1) a local of the CURRENT frame,
                        // (2) a member of THIS class, then (3) an enclosing
                        // frame's local (inlined blocking calls share the
                        // caller's scope). `dyn_name_lookup` walks every
                        // frame, so a caller's same-named local could steal
                        // the slot from the class member — e.g.
                        // `uvm_report_message::do_print`'s local
                        // `elements[$]` shadowing the container's `elements`
                        // member that `get_elements` returns, recording the
                        // wrong (empty) storage under `pending_ret_collection`
                        // and dropping every message element.
                        let mut from_bare = false;
                        let n = self
                            .dyn_name_lookup_innermost(bare)
                            .map(|s| std::borrow::Cow::Owned(s.to_string()))
                            .or_else(|| {
                                let m = self.instance_assoc_member(bare);
                                if h.path.len() == 1 {
                                    bare_member = Some(m.clone());
                                }
                                m.map(std::borrow::Cow::Owned)
                            })
                            .or_else(|| {
                                self.dyn_name_lookup_enclosing(bare)
                                    .map(|s| std::borrow::Cow::Owned(s.to_string()))
                            })
                            .unwrap_or_else(|| {
                                from_bare = true;
                                self.resolve_hier_name(h)
                            });
                        // §23.9 / §8.10: `return str;` of a scalar `string
                        // str` formal, local or property returns that scalar,
                        // not a same-named module-level `str[$]`.
                        if self.module.dynamic_arrays.contains(&*n)
                            && !(from_bare
                                && h.path.len() == 1
                                && self.scalar_hides_module_coll(bare))
                        {
                            // Frame teardown wipes a subroutine-LOCAL
                            // collection before the caller's assignment
                            // consumes it (the recorded name read back size
                            // 0) — snapshot elements+size under a reserved
                            // name that survives the pop; unpacked-struct
                            // elements member by member under a store that
                            // records their type (`snapshot_ret_structs`).
                            if let Some(su) = self.queue_elem_struct(&n) {
                                let len = self.get_queue_size(&n) as i64;
                                let idx: Vec<i64> = (0..len).collect();
                                self.snapshot_ret_structs(&n, &idx, &su);
                            } else {
                                const RET_SNAP: &str = "__xz_ret_coll__";
                                self.copy_whole_queue(RET_SNAP, &n);
                                self.pending_ret_collection = Some(RET_SNAP.to_string());
                            }
                        } else if self
                            .fn_ret_collection_stack
                            .last()
                            .copied()
                            .unwrap_or(false)
                            && h.path.iter().all(|seg| seg.selects.is_empty())
                        {
                            // A scalar local can shadow a module's fixed
                            // array; only an unpacked return consumes it whole.
                            self.snapshot_ret_fixed(&n);
                        }
                    }
                    if self.pending_ret_collection.is_none() {
                        let an = match bare_member {
                            Some(m) => m,
                            None => self.expr_assoc_name(e),
                        };
                        if let Some(an) = an {
                            if !self.is_associative_array(&an) {
                                self.pending_ret_collection = Some(an);
                            }
                        }
                    }
                    // §13.4, §7.9.9: `return aa;` from a function whose return
                    // type is an associative array. The frame teardown wipes
                    // a local array before the caller's assignment reads it,
                    // so snapshot its entries under a reserved name.
                    if self.pending_ret_collection.is_none()
                        && self.fn_ret_assoc_stack.last().copied().unwrap_or(false)
                    {
                        if let Some(src) = self.ret_assoc_operand_name(e) {
                            self.snapshot_ret_assoc(&src);
                        }
                    }
                    // §13.4: `return '{}`/`return '{...}` — a whole-collECTION
                    // LITERAL returned directly (queue / unsized dynamic array).
                    // The Ident path above cannot fire (there is no storage
                    // name), and `eval_expr` alone collapses a queue to a
                    // single packed scalar, so the caller's `q = f()` lost
                    // elements — an empty `'{}` came back as size 1 (one
                    // phantom element), and `'{a,b}` kept only `a`. When the
                    // CURRENT function returns a collection, snapshot the
                    // literal's elements into the reserved ret name and hand
                    // it through `pending_ret_collection` exactly as the
                    // Ident-return path does.
                    if self.pending_ret_collection.is_none()
                        && self
                            .fn_ret_collection_stack
                            .last()
                            .copied()
                            .unwrap_or(false)
                    {
                        if matches!(
                            self.unpacked_pattern(e).kind,
                            ExprKind::AssignmentPattern(_) | ExprKind::Concatenation(_)
                        ) {
                            const RET_SNAP: &str = "__xz_ret_coll__";
                            self.populate_queue_from_init(RET_SNAP, e);
                            self.pending_ret_collection = Some(RET_SNAP.to_string());
                        }
                    }
                    // §7.10.2.4, §7.10.2.5: `return q.pop_front()` on a queue
                    // of fixed-size rows hands back the removed row.
                    if self.pending_ret_collection.is_none()
                        && self
                            .fn_ret_collection_stack
                            .last()
                            .copied()
                            .unwrap_or(false)
                        && let Some(src) = self.pop_row_operand(e)
                    {
                        self.pending_ret_collection = Some(src);
                    }
                    // §13.4.1: an unpacked struct that is an ELEMENT of a
                    // class-property collection (`return q[i];`) or a
                    // subroutine local (`return r;`) has no container value;
                    // assemble it from its leaves.
                    let v = if self.pending_ret_collection.is_some() {
                        // §13.4.1: an unpacked return has already been read
                        // into its collection snapshot. Evaluating its bare
                        // function-name variable as a scalar would call the
                        // function again, recursively, because it has no
                        // packed container value.
                        Value::zero(32)
                    } else {
                        match self.struct_elem_type(e) {
                            Some(su) => self.pack_struct_elem_ref(e, &su),
                            None => match self.frame_struct_local(e) {
                                Some((name, su)) => self
                                    .pack_unpacked_struct(&name, &su)
                                    .unwrap_or_else(|| self.eval_expr(e)),
                                // §13.4.1: in the return variable's context,
                                // like `f = e` (`return b + u` with a byte b
                                // and 8-bit u in an int function is 9 bits).
                                // §10.8: a pattern takes the return type.
                                None => match self
                                    .ret_struct_elem_value(e)
                                    .or_else(|| self.ret_pattern_value(e))
                                {
                                    Some(v) => v,
                                    None => match self.fn_ret_ctx_stack.last() {
                                        Some(&w) if w > 0 => self.eval_expr_ctx(e, w),
                                        _ => self.eval_expr(e),
                                    },
                                },
                            },
                        }
                    };
                    if self.struct_colls_possible()
                        && matches!(
                            e.kind,
                            ExprKind::Ident(_)
                                | ExprKind::MemberAccess { .. }
                                | ExprKind::Index { .. }
                        )
                        && self.root_may_hold_struct(e)
                    {
                        self.stage_ret_struct_colls(e);
                    }
                    self.return_value = Some(v);
                    // §25.9: return the interface's handle token as well as
                    // its binding, so a memoized `getter().member` receiver
                    // can resolve the instance without repeating the call.
                    let vif_free = !self.vif_rhs_possible(e);
                    if let Some(bound) = (!vif_free).then(|| self.resolve_vif_rhs_name(e)).flatten()
                    {
                        if self.is_interface_instance(&bound) {
                            self.return_value =
                                Some(Value::from_u64(Self::iface_handle_token(&bound), 32));
                            self.last_vif_return = Some(bound);
                        }
                    }
                    if let Some(base) = pinned {
                        self.recv_call_memo.truncate(base);
                    }
                }
                self.break_flag = true;
                self.return_flag = true;
            }
            StatementKind::Disable(target) => {
                let name = crate::ast::stmt::disable_name(target);
                let fork_target = self.named_fork_target(target);
                // §38.36.1 cbDisable, reported once the disable took effect.
                if self.vpi_cb_mask & vpi_cb::m(vpi_cb::CB_DISABLE) != 0 {
                    self.vpi_note_disable(name, fork_target.as_deref());
                }
                // IEEE 1800-2017 §9.6.2. `disable_target` was set but never
                // read: the only effect was `break_flag`, a GLOBAL flag. So a
                // `disable` broke the innermost loop instead of terminating the
                // named block, truncated the disabling process, and leaked into
                // unrelated processes.
                //
                // A named `fork : blk ... join*` block: terminate the processes
                // it spawned (and their descendants), and CONTINUE. A join_none
                // fork block has already returned, so trying to unwind to it
                // below would run off the end of the disabling process and drop
                // its continuation — which is exactly what used to happen.
                if let Some(children) = fork_target.as_ref().and_then(|key| self.fork_block_children.get_mut(key)).map(std::mem::take) {
                    let mut to_kill: HashSet<usize> = HashSet::default();
                    for &c in &children {
                        to_kill.insert(c);
                        to_kill.extend(self.collect_fork_descendants(c));
                    }
                    // The disabling process may itself be one of them (a child
                    // or deeper descendant disabling its own fork block): it
                    // is terminated with its siblings and unwinds to its end.
                    let self_doomed = to_kill.contains(&self.current_pid);
                    {
                        for &pid in &to_kill {
                            self.killed_pids.insert(pid);
                            self.release_killed_process_state(pid);
                            self.vc_cancel_process(pid);
                            self.process_parents.remove(&pid);
                            self.process_contexts.remove(&pid);
                            // §9.6.2: purge the killed pid's future-time
                            // #delay too, or the wheel keeps reporting
                            // activity and the idle break never fires
                            // (UVM's run-phase watchdog timer).
                            self.event_queue.remove_pid(pid);
                        }
                        self.event_waiters.remove_pids(&to_kill);
                        self.instance_event_waiters.remove_pids(to_kill.iter());
                        for q in self.mailbox_get_waiters.values_mut() {
                            q.retain(|w| !to_kill.contains(&w.pid));
                        }
                        for q in self.semaphore_get_waiters.values_mut() {
                            q.retain(|w| !to_kill.contains(&w.pid));
                        }
                        self.await_waiters
                            .retain(|w| !to_kill.contains(&w.waiter_pid));
                        self.release_killed_from_join_waiters(&to_kill);
                        self.wake_await_waiters(&to_kill);
                        if self_doomed {
                            self.disable_target = Some(name.to_string());
                            self.break_flag = true;
                        }
                        return;
                    }
                }
                // A label naming ANOTHER process's top-level block terminates
                // that process; the disabling process carries on.
                if let Some(pid) = self.disable_label_pid(target) {
                    if pid != self.current_pid {
                        self.killed_pids.insert(pid);
                        self.release_killed_process_state(pid);
                        self.vc_cancel_process(pid);
                        self.process_parents.remove(&pid);
                        self.process_contexts.remove(&pid);
                        self.event_queue.remove_pid(pid);
                        self.event_waiters.remove_pid(pid);
                        self.instance_event_waiters.remove_pid(pid);
                        for q in self.mailbox_get_waiters.values_mut() {
                            q.retain(|w| w.pid != pid);
                        }
                        for q in self.semaphore_get_waiters.values_mut() {
                            q.retain(|w| w.pid != pid);
                        }
                        self.await_waiters.retain(|w| w.waiter_pid != pid);
                        let mut tset = HashSet::default();
                        tset.insert(pid);
                        self.wake_await_waiters(&tset);
                        let mut killed: HashSet<usize> = HashSet::default();
                        killed.insert(pid);
                        self.release_killed_from_join_waiters(&killed);
                        return;
                    }
                }
                // §9.6.2: `disable <task_name>` where OTHER processes are
                // currently executing an invocation of that task terminates
                // those invocations. (Approximation: the whole executing
                // process is terminated — exact for the common `fork
                // task_a; task_b; join` + `disable task_b` shape.)
                if let Some(pids) = self.active_task_pids.get(name).cloned() {
                    // An ANCESTOR executing the task (the disabling process was
                    // forked inside it) is unwound out of the call instead —
                    // see `disable_remote_block`.
                    let mut to_kill: HashSet<usize> = pids
                        .into_iter()
                        .filter(|&p| {
                            p != self.current_pid
                                && !self.killed_pids.contains(&p)
                                && !self.is_fork_ancestor(p, self.current_pid)
                        })
                        .collect();
                    // §35.9: an EXPORTED task that is the target returns 0 to
                    // its C caller, and the process carries on in the C code.
                    let handled = !self.dpi_tasks.is_empty()
                        && self.dpi_disable_exported_task(name, &mut to_kill);
                    if to_kill.is_empty() && handled {
                        return;
                    }
                    if !to_kill.is_empty() {
                        for &pid in &to_kill {
                            self.killed_pids.insert(pid);
                            self.release_killed_process_state(pid);
                            self.vc_cancel_process(pid);
                            self.process_parents.remove(&pid);
                            self.process_contexts.remove(&pid);
                            // §9.6.2: purge the killed pid's future-time
                            // #delay too, or the wheel keeps reporting
                            // activity and the idle break never fires
                            // (UVM's run-phase watchdog timer).
                            self.event_queue.remove_pid(pid);
                        }
                        self.event_waiters.remove_pids(&to_kill);
                        self.instance_event_waiters.remove_pids(to_kill.iter());
                        for q in self.mailbox_get_waiters.values_mut() {
                            q.retain(|w| !to_kill.contains(&w.pid));
                        }
                        for q in self.semaphore_get_waiters.values_mut() {
                            q.retain(|w| !to_kill.contains(&w.pid));
                        }
                        self.release_killed_from_join_waiters(&to_kill);
                        for set in self.active_task_pids.values_mut() {
                            for p in &to_kill {
                                set.remove(p);
                            }
                        }
                        return;
                    }
                }
                // §9.6.2: a block this process cannot be executing is another
                // process's, found now. Posting the disable for the process
                // loop to resolve lost the rest of a nested `begin ... end`
                // around it (`#5 begin disable lab.sub; ... end`). A path
                // names the block's process by its head (`lab.sub`).
                if !self.disable_may_be_own(target) {
                    let only = if target.path.len() > 1 {
                        let mut head = target.clone();
                        head.path.pop();
                        self.disable_label_pid(&head)
                    } else {
                        None
                    };
                    self.disable_remote_block_of(name, only);
                    return;
                }
                // Otherwise unwind this process to the end of the named block
                // (or the named task): the SeqBlock / task-call arm clears the
                // flags when it sees its own name, so execution resumes after
                // it. For a loop-body block that is `continue`; for a block
                // enclosing the loop it is `break`.
                self.disable_target = Some(name.to_string());
                self.break_flag = true;
                // Not yet known to be this process's own block: the process
                // loop checks, and hands it to `disable_remote_block` if not.
                self.disable_check_pending = true;
            }
            StatementKind::DisableFork => {
                // LRM §9.6.2: terminate all active descendant processes of the
                // calling process (children AND their descendants), then return.
                let cur = self.current_pid;
                let to_kill = self.collect_fork_descendants(cur);
                for &pid in &to_kill {
                    // Mark killed so any already-queued continuation is dropped
                    // at dispatch, and purge every place the process can be
                    // parked so it never resumes and never satisfies a join.
                    self.killed_pids.insert(pid);
                    self.release_killed_process_state(pid);
                    self.vc_cancel_process(pid);
                    self.process_parents.remove(&pid);
                    self.process_contexts.remove(&pid);
                    // §9.6.2: incl. its future-time #delay (see above).
                    self.event_queue.remove_pid(pid);
                }
                self.event_waiters.remove_pids(&to_kill);
                self.instance_event_waiters.remove_pids(to_kill.iter());
                self.condition_waiters.retain(|(p, _)| !to_kill.contains(p));
                self.ready_condition_waiters
                    .retain(|(p, _)| !to_kill.contains(p));
                for p in &to_kill {
                    self.cond_waiter_gate.remove(p);
                    self.cond_waiter_reads.remove(p);
                    self.cond_read_union_dirty = true;
                    self.cond_waiter_conditions.remove(p);
                }
                self.inactive_queue.retain(|(p, _)| !to_kill.contains(p));
                self.nba_region_waiters
                    .retain(|(p, _)| !to_kill.contains(p));
                self.deferred_clocking_conts
                    .retain(|(p, _)| !to_kill.contains(p));
                self.deferred_clocking_guarded
                    .retain(|(w, _)| !to_kill.contains(&w.pid));
                for q in self.mailbox_get_waiters.values_mut() {
                    q.retain(|w| !to_kill.contains(&w.pid));
                }
                for q in self.semaphore_get_waiters.values_mut() {
                    q.retain(|w| !to_kill.contains(&w.pid));
                }
                self.await_waiters
                    .retain(|w| !to_kill.contains(&w.waiter_pid));
                // A killed process must no longer hold back a join waiter: drop
                // it from every waiter's expected children, and wake any waiter
                // whose remaining children are now all accounted for.
                self.release_killed_from_join_waiters(&to_kill);
                self.wake_await_waiters(&to_kill);
            }
            StatementKind::WaitFork => {
                // §9.6.1 in a task entered from C (DPI export): the task cannot
                // park, so wait here until every child of this process is done
                // (#204). Elsewhere the suspend-aware runner handles it.
                if self.dpi_cur_task.is_some() {
                    self.dpi_wait_fork();
                } else if self.dpi_export_depth > 0 {
                    let me = self.current_pid;
                    self.run_nested_until(|sim| !sim.has_wait_fork_children(me));
                }
            }
            StatementKind::RsReturn => {
                self.rs_return_flag = true;
                self.break_flag = true;
            }
            StatementKind::RsAction { body } => {
                let prev = self.rs_return_flag;
                self.rs_return_flag = false;
                self.exec_statement(body);
                let triggered = self.rs_return_flag;
                self.rs_return_flag = prev;
                if triggered {
                    // Consume the break we raised for our RsReturn so the
                    // enclosing sequence continues with the next production.
                    self.break_flag = false;
                }
            }
            StatementKind::Wait { condition, stmt } => {
                if self.wait_condition_true(condition) {
                    self.exec_statement(stmt);
                } else if self.dpi_cur_task.is_some() {
                    // §35.5.2: inside an imported task running on its own
                    // stack, the calling process parks until the condition
                    // holds (dpi_task.rs).
                    self.dpi_wait_condition(condition);
                    if !self.finished {
                        self.exec_statement(stmt);
                    }
                } else if self.dpi_export_depth > 0 {
                    // A task entered from C (DPI export, §35.8) cannot park:
                    // wait here, running the scheduler until the condition
                    // holds (#204). §9.7.4: the condition is level-sensitive,
                    // so it is evaluated here, in the task's own frame, after
                    // every time slot the scheduler runs. It used to be parked
                    // as a condition waiter of a fresh process instead, which
                    // the nested loop never re-checked unless a recognised
                    // write named one of its operands: an event's triggered
                    // state or a pending clock left it parked until the run
                    // ended, and the fresh process could not see the task's
                    // automatic variables at all (#279).
                    self.run_nested_until(|sim| sim.wait_condition_true(condition));
                    if !self.finished {
                        self.exec_statement(stmt);
                    }
                } else {
                    // IEEE 1800-2023 §9.7.4: `wait(cond)` must block when the
                    // condition is false. `exec_statement` is the synchronous
                    // path (used for loop bodies), so it normally can't park.
                    // But `run_process_stmts` may have set `exec_park_cont` to
                    // the full continuation. If so, park the process and set
                    // return_flag to unwind all the way back to run_process_stmts.
                    if let Some(cont) = self.exec_park_cont.take() {
                        self.park_condition_waiter(self.current_pid, cont, condition);
                        self.parked_from_exec = true;
                        self.return_flag = true;
                        self.break_flag = true;
                    }
                }
            }
            StatementKind::Assertion(a) => {
                use crate::ast::stmt::AssertionKind;
                // The site's label: a module item's own (§16.5), or the name
                // of the block the parser wrapped a labelled procedural
                // statement in (§9.3.1).
                let wrap_label = self.assert_wrap_label.take();
                let site_label: Option<String> =
                    a.label.as_ref().map(|l| l.name.clone()).or(wrap_label);
                // LRM §16.5 — concurrent (`assert property (…)`) defers
                // evaluation to the observed region. We queue an
                // `ObservedProbe`; the drain in `drain_observed_region`
                // re-evaluates the predicate against the post-NBA settled
                // state and folds the outcome into assertion_stats. The
                // action / else_action are not run for concurrent variants
                // today — they'd need to fire in the reactive region,
                // which is its own follow-up.
                if a.is_property {
                    // §16.15 `default disable iff <guard>;` (a marker item
                    // the parser emits in the declaring scope).
                    if let ExprKind::SystemCall { name, args } = &a.expr.kind
                        && name == "$sva_default_disable"
                    {
                        if let Some(g) = args.first() {
                            let mut g = g.clone();
                            self.sva_resolve_signals(&mut g);
                            // Keyed by the declaring scope's full path (a
                            // generate block included).
                            let scope = self.m_path();
                            self.sva_default_disable.insert(scope, g);
                            // It governs the whole scope, so assertions
                            // registered before it are covered too.
                            for i in 0..self.sva_sites.len() {
                                if !self.sva_sites[i].explicit_disable {
                                    let key = self.sva_sites[i].disable_scope.clone();
                                    self.sva_sites[i].disable = self.sva_default_disable_for(&key);
                                }
                            }
                        }
                        return;
                    }
                    // LRM §16.6: `assert property (p_name);` —
                    // resolve a bare property-name reference to its
                    // stored body before dispatching to SVA. The
                    // property's body was captured at parse time
                    // (common `@(clk) <expr>` shape) and stored in
                    // `module.property_decls`.
                    let resolved_expr: Expression = match &a.expr.kind {
                        ExprKind::Ident(_) | ExprKind::MemberAccess { .. }
                            if let Some(name) = self.sva_named_key(&a.expr) =>
                        {
                            if let Some(body) = self.sva_body_with_defaults(&name) {
                                body
                            } else {
                                a.expr.clone()
                            }
                        }
                        // §16.6: `assert property (p(actuals));` — an
                        // INSTANTIATION of a named property with a port
                        // list. Resolve the body and substitute each formal
                        // for its positional actual (the substitution walker
                        // descends into SvaClocked, so a formal used as the
                        // CLOCK — `p(clk, ...)` — binds too). Previously
                        // this shape fell through unresolved and the assert
                        // was silently dropped: it never passed, never
                        // failed, and read as always-true.
                        ExprKind::Call { func, args } => {
                            // §26.3: `pkg::p(...)` names that package's
                            // property; any other dotted name its leaf.
                            let pname: Option<String> =
                                self.sva_named_key(func).or_else(|| match &func.kind {
                                    ExprKind::Ident(h)
                                        if !h.path.is_empty()
                                            && h.path.iter().all(|s| s.selects.is_empty()) =>
                                    {
                                        Some(h.path.last().unwrap().name.name.clone())
                                    }
                                    _ => None,
                                });
                            let resolved = pname.and_then(|n| {
                                let body = self.module.property_decls.get(&n).cloned()?;
                                let map = self.sva_bind_actuals(&n, args);
                                Some(super::elaborate::rewrite_expr(
                                    &body,
                                    "",
                                    &map,
                                    &std::collections::HashSet::new(),
                                    &HashMap::default(),
                                ))
                            });
                            resolved.unwrap_or_else(|| a.expr.clone())
                        }
                        _ => a.expr.clone(),
                    };
                    // LRM §16.5: a `SvaClocked{events, body}` body
                    // registers a clocked site instead of going through
                    // the per-iteration observed-probe path. Dedup by
                    // span_key so re-execution (e.g. inside an always
                    // block) doesn't grow the site list unbounded.
                    // A clockless `assert property` samples on the default
                    // clocking (§14.12).
                    let clocked: Option<(Vec<EventExpr>, Expression)> = match &resolved_expr.kind {
                        ExprKind::SvaClocked { events, body } => {
                            Some((events.clone(), (**body).clone()))
                        }
                        _ if a.inferred_clock.is_some() => Some((
                            vec![a.inferred_clock.as_deref().unwrap().clone()],
                            resolved_expr.clone(),
                        )),
                        _ => match self.clocking_meta.get("__xz_default_clocking") {
                            Some((clk, _)) => {
                                let edge = match self.clocking_edge.get("__xz_default_clocking") {
                                    Some(EdgeKind::Negedge) => Edge::Negedge,
                                    _ => Edge::Posedge,
                                };
                                let event = EventExpr {
                                    expr: Self::hier_ident_expr(
                                        vec![HierPathSegment {
                                            name: crate::ast::Identifier {
                                                name: clk.clone(),
                                                span: a.span,
                                            },
                                            selects: Vec::new(),
                                        }],
                                        a.span,
                                    ),
                                    edge: Some(edge),
                                    iff: None,
                                    span: a.span,
                                };
                                Some((vec![event], resolved_expr.clone()))
                            }
                            None => None,
                        },
                    };
                    if let Some((events, body)) = clocked {
                        let span_key = a.span.start;
                        let existing = self.sva_sites.iter().position(|s| {
                            s.span_key == span_key && s.dedup_scope == self.current_scope
                        });
                        let idx = match existing {
                            Some(i) => i,
                            None => {
                                self.register_sva_site(a, events, body, site_label.as_deref());
                                self.sva_sites.len() - 1
                            }
                        };
                        // §16.14.6.2: each execution of a procedural
                        // concurrent assertion queues one instance, with the
                        // automatic variables it reads captured now.
                        if self.sva_sites[idx].procedural {
                            let mut env = self.sva_sites[idx].lv_init.clone();
                            for (k, name) in &self.sva_sites[idx].captured {
                                if let Some(v) =
                                    self.local_stack.last().and_then(|m| m.get(name.as_str()))
                                {
                                    env[*k] = v.clone();
                                }
                            }
                            let pid = if self.settling {
                                usize::MAX
                            } else {
                                self.current_pid
                            };
                            self.sva_proc_pending.push((idx, pid, env));
                        }
                        return;
                    }
                    // The unclocked property's outcomes land on the site's
                    // own entry (file, line, kind and label), not on a
                    // locationless one keyed by a synthetic name.
                    let src = self.current_src_file();
                    let stat_key = Self::assert_stat_key(a.span.start, src);
                    self.note_assert_label(stat_key, site_label.as_deref());
                    let kind: u8 = match a.kind {
                        AssertionKind::Assert => 0,
                        AssertionKind::Assume => 1,
                        AssertionKind::Cover => 2,
                    };
                    let this_handle = self.this_stack.last().and_then(|t| *t);
                    self.pending_observed.push(ObservedProbe {
                        stat_key,
                        loc: (src, a.span.start),
                        kind,
                        predicate: resolved_expr,
                        this_handle,
                    });
                    return;
                }
                // §20.12: an assertion that is off is not evaluated.
                let ctl = if self.assert_ctl_on {
                    let st = self.assert_ctl_state(&self.imm_assert_ident(a));
                    if !st.enabled() {
                        return;
                    }
                    st
                } else {
                    Default::default()
                };
                let true_branch = self.eval_expr(&a.expr).is_true();
                if let Some(kind) = a.deferred {
                    self.queue_deferred_assert(a, kind, true_branch, site_label.as_deref());
                    return;
                }
                // Track per-site tallies for end-of-sim summary + JSON DB.
                let kind_tag: u8 = match a.kind {
                    AssertionKind::Assert => 0,
                    AssertionKind::Assume => 1,
                    AssertionKind::Cover => 2,
                };
                let src = self.current_src_file();
                let stat_key = Self::assert_stat_key(a.span.start, src);
                self.note_assert_label(stat_key, site_label.as_deref());
                let entry = self
                    .assertion_stats
                    .entry(stat_key)
                    .or_insert(AssertionStat {
                        kind: kind_tag,
                        loc: Some((src, a.span.start)),
                        pass_count: 0,
                        fail_count: 0,
                    });
                if true_branch {
                    entry.pass_count += 1;
                } else {
                    entry.fail_count += 1;
                }
                if !true_branch {
                    if !ctl.fail_action() {
                    } else if let Some(ea) = &a.else_action {
                        self.exec_statement(ea);
                    } else if kind_tag != 2 {
                        // §16.3: with no `else` clause a failure calls
                        // `$error` (a cover has no failure action).
                        self.emit_severity_text("Error", ASSERT_DEFAULT_ERROR);
                    }
                } else if let Some(ac) = &a.action {
                    if ctl.pass_action(false) {
                        self.exec_statement(ac);
                    }
                }
            }
            StatementKind::ProceduralContinuous(pc) => match pc {
                ProceduralContinuous::Assign { lvalue, rvalue }
                | ProceduralContinuous::Force { lvalue, rvalue } => {
                    // LRM §10.6.2 `force` / §10.6.1 procedural continuous
                    // `assign`: the override supersedes ordinary procedural
                    // assignments and continuous drivers until the matching
                    // release/deassign. We register the target in
                    // `forced_signals` (compact-table id) or `forced_names`
                    // (runtime-map storage); every central write path
                    // (`write_sig!`, the comb-settle direct-copy items, the
                    // name-keyed fallbacks) drops writes to registered
                    // targets.
                    let v = self.eval_expr(rvalue);
                    let partial_sel = if matches!(pc, ProceduralContinuous::Force { .. })
                        && matches!(
                            lvalue.kind,
                            ExprKind::Index { .. } | ExprKind::RangeSelect { .. }
                        ) {
                        self.force_select_range(lvalue)
                    } else {
                        None
                    };
                    if let Some(sel) = partial_sel {
                        self.force_partial(sel, rvalue);
                        return;
                    }
                    let target = self.force_target(lvalue);
                    let vpi_target = target.as_ref().map(|(_, id)| *id);
                    // A second force/assign on an already-overridden target
                    // REPLACES the previous override (§10.6.1/§10.6.2), so
                    // lift the old one first — the guarded write paths would
                    // otherwise drop this very write — then re-arm below.
                    if let Some((ref name, id)) = target {
                        if let Some(id) = id {
                            self.unforce_cell(id);
                            self.partial_forces.remove(&id);
                        }
                        self.forced_names.remove(name);
                    }
                    self.assign_value(lvalue, &v);
                    // A net's gate must see the override (and, for a shape
                    // degraded to a plain write, overwrite it on the next
                    // input change exactly as before).
                    self.ctl_note_deposit(lvalue);
                    match target {
                        Some((ref name, Some(id))) => {
                            self.force_cell(id, v);
                            self.arm_force_expr(name.clone(), lvalue, rvalue);
                        }
                        Some((name, None)) => {
                            self.forced_names.insert(name.clone());
                            self.arm_force_expr(name, lvalue, rvalue);
                        }
                        // Unsupported lvalue shape (part-select, concat, …):
                        // degrade to a plain write, as before.
                        None => {}
                    }
                    // §38.36.1 cbForce / cbAssign, after the override took hold.
                    if let Some(sig) = vpi_target {
                        if self.vpi_cb_mask != 0 {
                            let reason = match pc {
                                ProceduralContinuous::Force { .. } => vpi_cb::CB_FORCE,
                                _ => vpi_cb::CB_ASSIGN,
                            };
                            self.vpi_notify_override(reason, sig);
                        }
                    }
                }
                ProceduralContinuous::Release(lvalue) | ProceduralContinuous::Deassign(lvalue) => {
                    // LRM §10.6.2 `release`: a VARIABLE keeps the forced
                    // value until the next procedural assignment — so we
                    // only lift the override and leave the stored value
                    // untouched. A NET returns to its continuous drivers —
                    // mark the source signals of every comb entry driving it
                    // dirty so the next settle re-evaluates the drivers.
                    // §10.6.1 `deassign` likewise retains the last value.
                    if matches!(pc, ProceduralContinuous::Release(_))
                        && matches!(
                            lvalue.kind,
                            ExprKind::Index { .. } | ExprKind::RangeSelect { .. }
                        )
                    {
                        if let Some(sel) = self.force_select_range(lvalue) {
                            self.release_partial(sel);
                            return;
                        }
                    }
                    let target = self.force_target(lvalue);
                    self.release_override(target.as_ref().map(|(n, id)| (n.as_str(), *id)));
                    // §38.36.1 cbRelease / cbDeassign, reported once the
                    // released net has been re-driven.
                    if let Some((_, sig)) = target {
                        if self.vpi_cb_mask != 0 {
                            let reason = match pc {
                                ProceduralContinuous::Release(_) => vpi_cb::CB_RELEASE,
                                _ => vpi_cb::CB_DEASSIGN,
                            };
                            self.vpi_queue_release(reason, sig);
                        }
                    }
                }
            },
            StatementKind::RandCase { items } => {
                if let Some(i) = self.randcase_pick(items) {
                    let stmt = items[i].1.clone();
                    self.exec_statement(&stmt);
                }
            }
            StatementKind::Typedef(td) => {
                // §6.18: a block-local typedef registers its width and
                // underlying type so later declarations in this process
                // resolve it — it was parsed and DISCARDED before, so a
                // local `typedef struct packed {...} t; t v;` lost the
                // member layout. Enum members become named constants.
                // §8.5: in a class method, by the class's own constants too.
                let spec = self.class_type_env(&td.data_type);
                let w = {
                    let params = self.spec_param_env(&spec);
                    super::elaborate::resolve_type_width_env(
                        &td.data_type,
                        Some(&params),
                        Some(&self.module.typedefs),
                    )
                };
                self.type_tables_gen += 1;
                self.declaration_typedefs.get_or_insert_with(|| {
                    (
                        self.module.typedefs.clone(),
                        self.module.typedef_types.clone(),
                    )
                });
                self.module.typedefs.insert(td.name.name.clone(), w);
                self.module
                    .typedef_types
                    .insert(td.name.name.clone(), td.data_type.clone());
                self.typedef_layouts_by_width = None;
                if let Some(set) = self.plain_class_types.borrow_mut().as_mut() {
                    set.remove(&td.name.name);
                }
                *self.coll_member_names.borrow_mut() = None;
                if !td.dimensions.is_empty() {
                    self.module
                        .typedef_unpacked_dims
                        .insert(td.name.name.clone(), td.dimensions.clone());
                }
                if let crate::ast::types::DataType::Enum(et) = &td.data_type {
                    let base_w = et
                        .base_type
                        .as_ref()
                        .map(|bt| {
                            super::elaborate::resolve_type_width(
                                bt,
                                Some(&self.module.parameters),
                                Some(&self.module.typedefs),
                            )
                        })
                        .unwrap_or(32);
                    let mut next: u64 = 0;
                    let mut members: Vec<(String, u64)> = Vec::new();
                    for m in &et.members {
                        let v = match &m.init {
                            Some(e) => self.eval_expr(e).to_u64().unwrap_or(next),
                            None => next,
                        };
                        next = v.wrapping_add(1);
                        self.declare_enum_constant(&m.name.name, Value::from_u64(v, base_w));
                        members.push((m.name.name.clone(), v));
                    }
                    self.module
                        .enum_members
                        .insert(td.name.name.clone(), members);
                    // The member values were evaluated after the first bump.
                    self.type_tables_gen += 1;
                }
            }
            StatementKind::VarDecl {
                data_type,
                lifetime,
                declarators,
            } => self.exec_stmt_var_decl(stmt, data_type, lifetime, declarators),
            StatementKind::ParamDecl {
                data_type,
                declarators,
                ..
            } => self.exec_stmt_param_decl(stmt, data_type, declarators),
            StatementKind::EventTrigger {
                name,
                nonblocking,
                target,
                ..
            } => {
                // §15.5: `-> h.evs[i]` — an element of a class event array.
                if let Some(key) = target
                    .as_deref()
                    .and_then(|t| self.expr_instance_event_elem(t))
                {
                    if *nonblocking {
                        self.pending_nba_instance_triggers.push(key);
                    } else {
                        self.fire_instance_event(key);
                    }
                    return;
                }
                if *nonblocking {
                    // §15.5.2: defer to the NBA region of the current slot.
                    // Resolve a class-field instance key NOW (while `this` is
                    // live); the NBA drain cannot reconstruct it later.
                    if let Some(key) = self.resolve_this_event_field(&name.name) {
                        self.pending_nba_instance_triggers.push(key);
                    } else if let Some(key) =
                        self.static_class_event_trigger_key(&name.name, target.as_deref())
                    {
                        self.pending_nba_instance_triggers.push(key);
                    } else if let Some(key) = target
                        .as_ref()
                        .and_then(|t| self.expr_instance_event_field_general(t))
                    {
                        self.pending_nba_instance_triggers.push(key);
                    } else {
                        let key = target
                            .as_ref()
                            .and_then(|t| self.event_ref_key(t))
                            .unwrap_or_else(|| self.lexical_event_name(&name.name));
                        self.pending_nba_triggers.push(key);
                    }
                    return;
                }
                if let Some(key) = self.resolve_this_event_field(&name.name) {
                    self.fire_instance_event(key);
                    return;
                }
                if let Some(key) =
                    self.static_class_event_trigger_key(&name.name, target.as_deref())
                {
                    self.fire_instance_event(key);
                    return;
                }
                // `-> h.ce` / `-> this.ce`: a class-property event reached
                // through a handle (the `->` parser flattens it to a dotted
                // name).
                if name.name.contains('.') {
                    if let Some(key) = self.resolve_handle_event_field(&name.name) {
                        self.fire_instance_event(key);
                        return;
                    }
                }
                // Receivers the flattened string cannot express — runtime
                // selects (`-> m_events[obj].all_dropped`, the UVM objection
                // drop path, #109) and chained handles — resolve from the
                // carried target EXPRESSION.
                if let Some(key) = target
                    .as_ref()
                    .and_then(|t| self.expr_instance_event_field_general(t))
                {
                    self.fire_instance_event(key);
                    return;
                }
                // An event bound to a `ref` formal fires the CALLER's event.
                // §15.5: the rewritten target retains an event-array select
                // and its instance scope when the flattened name does not.
                if let Some(key) = target.as_ref().and_then(|t| self.event_ref_key(t)) {
                    self.fire_named_event(&key);
                    return;
                }
                if !self.module.events.contains(name.name.as_str()) {
                    if let Some(k) = self.ref_bound_event_key(&name.name) {
                        self.fire_named_event(&k);
                        return;
                    }
                }
                // §23.4 lexical home: a bare name fired from a cross-scope
                // call targets the callee's own event, not the caller's
                // same-named one.
                let lex = self.lexical_event_name(&name.name);
                self.fire_named_event(&lex);
            }
            StatementKind::Coverpoint { .. } | StatementKind::Cross { .. } => {}
            StatementKind::WaitOrder { .. } => {
                // The synchronous executor cannot suspend; wait_order is
                // handled on the process path (run_process_stmts). Reaching
                // here means the construct sat in a context that never
                // inlines — warn rather than silently drop.
                eprintln!(
                    "[xezim][warning] wait_order reached the non-suspending execution path and was skipped (IEEE 1800-2017 §15.5.3)"
                );
            }
        }
    }
}
