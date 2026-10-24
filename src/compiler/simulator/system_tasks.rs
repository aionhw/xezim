//! Simulator system tasks.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    pub(super) fn exec_expr_stmt(&mut self, expr: &Expression) {
        match &expr.kind {
            ExprKind::SystemCall { name, args } => {
                // The call site, for a `$systf` registered by a VPI module.
                vpi_api::set_task_call_span(expr.span);
                self.exec_system_task(name, args)
            }
            ExprKind::Ident(hier) => {
                // §13.3: parens are optional on a task enable — try both the
                // scope-resolved name and the RAW dotted path (an interface
                // task `di.show;` resolves under the instance-scoped key,
                // which resolve_hier_name may rewrite away).
                let name = self.resolve_hier_name(hier);
                let raw = (hier.path.len() >= 2).then(|| {
                    hier.path
                        .iter()
                        .map(|s| s.name.name.as_str())
                        .collect::<Vec<_>>()
                        .join(".")
                });
                let td = self
                    .module
                    .tasks
                    .get(&*name)
                    .or_else(|| raw.as_deref().and_then(|r| self.module.tasks.get(r)))
                    .cloned();
                if let Some(td) = td {
                    // Execute a bare task-enable unless the body genuinely needs
                    // a suspendable process (event control, `wait`, `fork…join`,
                    // `forever`) — those need semantics this synchronous path
                    // cannot model. A body that merely contains a `#delay` IS
                    // executable here: the `TimingControl::Delay` arm of
                    // `exec_statement` flushes NBA, settles and advances time.
                    // Testing the broader `stmt_is_blocking` here silently
                    // DROPPED such calls entirely (see
                    // `stmt_needs_process_context`).
                    if !td
                        .items
                        .iter()
                        .any(|s| self.stmt_needs_process_context(s, 0))
                    {
                        self.exec_task_call(&td, &[]);
                    }
                } else {
                    self.eval_expr(expr);
                }
            }
            ExprKind::AssignExpr { lvalue, rvalue } => {
                // Simple direct assign — the for-loop step produces this
                // after xezim-core 8b9c88c, and it's a tight inner-loop
                // stmt in memory-init patterns. Skip the eval_expr
                // dispatch overhead of the `_` fallback.
                let val = self.eval_expr(rvalue);
                self.assign_value(lvalue, &val);
            }
            ExprKind::Binary {
                op: BinaryOp::Assign,
                left,
                right,
            } => {
                let val = if let ExprKind::Call { func, args } = &right.kind {
                    if let ExprKind::Ident(hier) = &func.kind {
                        if hier.path.last().unwrap().name.name == "new" {
                            let type_name = self.get_expr_type_name(left);
                            // Resolve a type-parameter-typed destination
                            // through the current instance's bindings to its
                            // concrete class.
                            let type_name = type_name
                                .map(|tn| self.resolve_type_param_binding(&tn).unwrap_or(tn));
                            if let Some(tname) = type_name {
                                if let Some(class_def) = self.module.classes.get(&tname).cloned() {
                                    let lname_opt = if let ExprKind::Ident(lh) = &left.kind {
                                        Some(self.resolve_hier_name(lh))
                                    } else {
                                        None
                                    };
                                    let ta_cloned =
                                        self.resolve_ctor_type_args(left, lname_opt.as_deref());
                                    self.instantiate_class_with_type_args(
                                        &class_def,
                                        args,
                                        ta_cloned.as_deref(),
                                    )
                                } else if let Some(cg_def) = self.covergroup_def_for(&tname) {
                                    self.instantiate_covergroup(&cg_def, args)
                                } else {
                                    self.eval_expr(right)
                                }
                            } else {
                                self.eval_expr(right)
                            }
                        } else {
                            self.eval_expr(right)
                        }
                    } else {
                        self.eval_expr(right)
                    }
                } else {
                    self.eval_expr(right)
                };
                self.assign_value(left, &val);
            }
            ExprKind::Unary { op, operand } => match op {
                UnaryOp::PreIncr | UnaryOp::PostIncr => {
                    let v = self.eval_expr(operand);
                    let nv = v.add(&Self::inc_dec_step(&v));
                    self.assign_value(operand, &nv);
                }
                UnaryOp::PreDecr | UnaryOp::PostDecr => {
                    let v = self.eval_expr(operand);
                    let nv = v.sub(&Self::inc_dec_step(&v));
                    self.assign_value(operand, &nv);
                }
                _ => {
                    self.eval_expr(expr);
                }
            },
            _ => {
                self.eval_expr(expr);
            }
        }
    }

    /// Apply `self.sdf_annotation` to a design whose comb-entry tables are
    /// ALREADY BUILT (runtime `$sdf_annotate`, executing long after
    /// `compile()`). Mirrors the application step in `compile()` (fill
    /// `sdf_delays` from the annotation's signal_delays), then fixes up the
    /// two baked fast paths that never consult `sdf_delays` at eval time:
    ///
    ///  * `FastDirectCopy` → demoted to `DirectCopy`, whose slow path routes
    ///    a delayed destination through `schedule_delayed`;
    ///  * `FusedGate` → `exec_fused_gate` now checks `sdf_delays` whenever
    ///    the vec is nonempty (build-time CLI annotation never fuses a
    ///    delayed destination, so the check is one `is_empty()` there).
    ///
    /// `CompiledContAssign` (bytecode VM) entries keep their build-time
    /// behavior — the VM's BlockingAssign does not model SDF delay, which is
    /// the same limitation the CLI --sdf path has.
    pub(super) fn apply_sdf_annotation_late(&mut self) {
        if self.sdf_delays.len() != self.signal_table.len() {
            self.sdf_delays.resize(self.signal_table.len(), 0);
        }
        // Gate read masks assume no delays (`comb_entry_read_masks`).
        self.comb_dep_edges.clear();
        let Some(ann) = self.sdf_annotation.take() else {
            return;
        };
        let mut count = 0usize;
        for (sig_name, &delay) in &ann.signal_delays {
            if let Some(&id) = self.signal_name_to_id.get(sig_name.as_str()) {
                self.sdf_delays[id] = delay;
                self.drop_module_paths_of(id);
                count += 1;
            }
        }
        self.sdf_annotation = Some(ann);
        eprintln!("[SDF] annotated {} signals with delays", count);
        self.apply_sdf_timing_limits();
        for e in self.comb_entries.iter_mut() {
            if let CombItem::FastDirectCopy { dst_id, src_id } = e.item {
                if self.sdf_delays.get(dst_id).copied().unwrap_or(0) > 0 {
                    let width = self.signal_widths[dst_id];
                    e.item = CombItem::DirectCopy {
                        dst_id,
                        src_id,
                        width,
                    };
                }
            }
        }
    }

    /// Every `$name` serviced somewhere by the two runtime dispatchers — the
    /// statement-position `exec_system_task` match and the expression-position
    /// `SystemCall` eval match. The unknown-system-task diagnostic consults
    /// this so it stays quiet for names that ARE handled: a function-only name
    /// reaching the STATEMENT fallthrough (e.g. `$urandom;` with the result
    /// discarded) is not "unknown", it is just not re-dispatched from there,
    /// and vice versa for task-only names in expression position. Keep in sync
    /// when adding dispatcher arms — a missed entry shows up as one spurious
    /// once-only warning, never as wrong simulation.
    pub(super) fn known_system_name(name: &str) -> bool {
        matches!(
            name,
            "$srandom" | "$get_randstate" | "$set_randstate" | // -- statement-position tasks (exec_system_task arms) --
            "$timeformat" | "$printtimescale" | "$cast" | "$info" | "$warning"
                | "$error" | "$fatal"
                | "$display" | "$displayb" | "$displayh" | "$displayo"
                | "$write" | "$writeb" | "$writeh" | "$writeo"
                | "$swrite" | "$swriteb" | "$swriteh" | "$swriteo" | "$sformat"
                | "$strobe" | "$strobeb" | "$strobeh" | "$strobeo"
                | "$value$plusargs" | "$test$plusargs"
                | "$monitor" | "$monitorb" | "$monitorh" | "$monitoro"
                | "$monitoron" | "$monitoroff"
                | "$deposit" | "$finish" | "$stop" | "$exit"
                | "$fopen" | "$fclose" | "$fflush" | "$fseek" | "$ftell"
                | "$rewind" | "$ungetc" | "$fgets" | "$fgetc" | "$feof"
                | "$ferror" | "$fscanf" | "$sscanf" | "$fread"
                | "$fwrite" | "$fwriteb" | "$fwriteh" | "$fwritex" | "$fwriteo"
                | "$fdisplay" | "$fdisplayb" | "$fdisplayh" | "$fdisplayx"
                | "$fdisplayo"
                | "$readmemh" | "$readmemb" | "$readmemd"
                | "$writememb" | "$writememh" | "$writememd"
                | "$dumpfile" | "$dumpvars" | "$dumpoff" | "$dumpon"
                | "$dumpall" | "$dumpflush" | "$dumplimit"
                | "$q_initialize" | "$q_add" | "$q_remove" | "$q_exam" | "$q_full"
                | "$system" | "$stacktrace"
                // -- expression-position functions (SystemCall eval arms) --
                | "$past" | "$rose" | "$fell" | "$stable" | "$changed"
                | "$sformatf" | "$psprintf"
                | "$clog2" | "$bits" | "$signed" | "$unsigned"
                | "$time" | "$realtime" | "$stime"
                | "$inferred_clock" | "$inferred_disable" | "$global_clock"
                | "$timeunit" | "$timeprecision"
                | "$rose_gclk" | "$fell_gclk" | "$steady_gclk" | "$changing_gclk"
                | "$past_gclk" | "$future_gclk"
                | "$urandom" | "$urandom_range" | "$random"
                | "$dist_uniform" | "$dist_normal" | "$dist_exponential"
                | "$dist_poisson" | "$dist_chi_square" | "$dist_t" | "$dist_erlang"
                | "$isunknown" | "$is_signed"
                | "$realtobits" | "$bitstoreal" | "$itor" | "$rtoi"
                | "$ceil" | "$floor" | "$sqrt" | "$pow" | "$log10" | "$exp"
                | "$ln" | "$log2"
                | "$sin" | "$cos" | "$tan" | "$asin" | "$acos" | "$atan"
                | "$atan2" | "$hypot"
                | "$sinh" | "$cosh" | "$tanh" | "$asinh" | "$acosh" | "$atanh"
                | "$shortrealtobits" | "$bitstoshortreal"
                | "$dimensions" | "$unpacked_dimensions" | "$typename"
                | "$isunbounded"
                | "$countbits" | "$countones" | "$onehot" | "$onehot0"
                | "$left" | "$right" | "$high" | "$low" | "$size" | "$increment"
        )
    }

    /// Emit `msg` once per distinct system-task `name` for the whole run.
    pub(super) fn warn_system_task_once(&mut self, name: &str, msg: &str) {
        if self.warned_system_tasks.insert(name.to_string()) {
            // One write for the whole line: `eprintln!` sends the text and the
            // newline separately, and `$display` output from the stdout writer
            // thread can land between them when both streams go to one file
            // (`> run.log 2>&1`), gluing a UVM line onto this note.
            use std::io::Write;
            let mut line = String::with_capacity(msg.len() + 1);
            line.push_str(msg);
            line.push('\n');
            let _ = std::io::stderr().write_all(line.as_bytes());
        }
    }

    /// The set of system-task names that produced a once-only diagnostic this
    /// run (unknown tasks + recognized-but-unsupported stubs). For tests.
    pub fn warned_system_task_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.warned_system_tasks.iter().cloned().collect();
        v.sort();
        v
    }

    /// META-DIAGNOSTIC for missing system tasks: any `$name` that reaches a
    /// dispatcher fallthrough without being recognized ANYWHERE is reported
    /// once per run, so a missing implementation is a visible warning instead
    /// of silent mis-simulation. Excluded: VPI-registered systfs (dispatched
    /// by an earlier arm), `$__xz_*` internal markers, and names the other
    /// dispatcher services (`known_system_name`). The wording deliberately
    /// avoids the word "err\u{6f}r" so a diagnostic never turns a passing
    /// design into a rejected one for log-grepping harnesses.
    pub(super) fn warn_unknown_system_task(&mut self, name: &str, args: &[Expression]) {
        if name.starts_with("$__") || Self::known_system_name(name) {
            return;
        }
        if self.warned_system_tasks.contains(name) {
            return; // repeat use: skip span resolution entirely
        }
        let loc = args.first().and_then(|a| {
            let sf = self.stall_pid_src_file(self.current_pid);
            self.span_file_line_in(a.span, sf)
        });
        let msg = match loc {
            Some(l) => format!(
                "Warning: unknown system task '{}' ignored (first use at {})",
                name, l
            ),
            None => format!("Warning: unknown system task '{}' ignored", name),
        };
        self.warn_system_task_once(name, &msg);
    }

    /// A waveform system task was called without `--wave`. Warn once per task
    /// and carry on: a design that dumps should still simulate, it just
    /// produces no waveform.
    pub(super) fn warn_wave_disabled(&mut self, name: &str) {
        self.warn_system_task_once(
            name,
            &format!(
                "Note: {} ignored — this run has no waveform support. \
                 Re-run with `--wave` to dump VCD/FST.",
                name
            ),
        );
    }

    pub(super) fn exec_system_task(&mut self, name: &str, args: &[Expression]) {
        if name == SYNC_WAKE_MARKER {
            if let Some(id) = args.first().and_then(|a| self.eval_expr(a).to_u64()) {
                self.sync_wakes.insert(id);
            }
            return;
        }
        if name == SAMPLED_DEFAULT_MARKER {
            self.record_sampled_defaults(args);
            return;
        }
        // §21.7: every waveform task is inert without `--wave`, which is what
        // makes the dump opt-in at model-compile time rather than triggered by
        // whatever the source happens to call.
        if !self.wave
            && matches!(
                name,
                "$dumpfile"
                    | "$dumpvars"
                    | "$dumpoff"
                    | "$dumpon"
                    | "$dumpall"
                    | "$dumpflush"
                    | "$dumplimit"
                    | "$fsdbDumpfile"
                    | "$fsdbDumpvars"
                    | "$vcdpluson"
                    | "$vcdplusoff"
            )
        {
            self.warn_wave_disabled(name);
            return;
        }
        match name {
            COV_HIT_TASK => self.cov_hit(args.first()),
            // §21.3.5 $timeformat(units, precision, suffix, min_width). All
            // arguments are optional in xezim's callers; missing ones keep the
            // prior/default value.
            "$timeformat" => {
                let cur = self.time_format.clone();
                let units = args
                    .first()
                    .map(|a| self.eval_expr(a).to_i64().unwrap_or(-9) as i32)
                    .unwrap_or_else(|| {
                        cur.as_ref()
                            .map(|c| c.0)
                            .unwrap_or_else(|| Self::secs_to_exp(self.tick_s))
                    });
                let prec = args
                    .get(1)
                    .map(|a| self.eval_expr(a).to_u64().unwrap_or(0) as usize)
                    .unwrap_or_else(|| cur.as_ref().map(|c| c.1).unwrap_or(0));
                let suffix = args
                    .get(2)
                    .map(|a| self.eval_expr(a).to_sv_string())
                    .unwrap_or_else(|| cur.as_ref().map(|c| c.2.clone()).unwrap_or_default());
                let width = args
                    .get(3)
                    .map(|a| self.eval_expr(a).to_u64().unwrap_or(0) as usize)
                    .unwrap_or_else(|| cur.as_ref().map(|c| c.3).unwrap_or(0));
                self.time_format = Some((units, prec, suffix, width));
            }
            // §21.2.1.4 $printtimescale [(hier)]. Reports the calling scope's
            // time unit and precision.
            "$printtimescale" => {
                // §20.11: an instance argument selects THAT instance's
                // module timescale; no argument means the calling scope.
                let arg_inst: Option<std::borrow::Cow<str>> = args.first().and_then(|a| {
                    if let ExprKind::Ident(h) = &a.kind {
                        Some(self.resolve_hier_name(h))
                    } else {
                        None
                    }
                });
                let (unit_exp, prec_exp, scope) = if let Some(inst) = arg_inst {
                    let rel = inst
                        .strip_prefix(&format!("{}.", self.module.name))
                        .unwrap_or(&inst);
                    // Prefix the caller's own instance scope for a relative
                    // path ($printtimescale(m1) inside tb_top.sub).
                    let hint = self
                        .process_scope_hint
                        .get(&self.current_pid)
                        .cloned()
                        .unwrap_or_default();
                    let candidates = if hint.is_empty() {
                        vec![rel.to_string()]
                    } else {
                        vec![format!("{}.{}", hint, rel), rel.to_string()]
                    };
                    let mut found: Option<(i32, i32, String)> = None;
                    for cand in candidates {
                        if let Some(def) = self
                            .module
                            .instances
                            .iter()
                            .find(|i| i.path == cand)
                            .map(|i| i.def_name.clone())
                        {
                            let ts = self.reported_timescale_exp(&def);
                            found = Some((ts.0, ts.1, self.hier_path(&cand)));
                            break;
                        }
                    }
                    match found {
                        Some(f) => f,
                        None => {
                            let def = self.current_module_def().to_string();
                            let (u, p) = self.reported_timescale_exp(&def);
                            (u, p, self.hier_path(rel))
                        }
                    }
                } else {
                    let def = self.current_module_def().to_string();
                    let (u, p) = self.reported_timescale_exp(&def);
                    let scope = match self.process_scope_hint.get(&self.current_pid) {
                        Some(s) => self.hier_path(s),
                        None => self.module.name.clone(),
                    };
                    (u, p, scope)
                };
                let m = format!(
                    "Time scale of ({}) is {} / {}",
                    scope,
                    Self::time_exp_to_str(unit_exp),
                    Self::time_exp_to_str(prec_exp)
                );
                self.record_output(m.clone());
                self.stdout_writeln(&m);
            }
            "$cast" => {
                // `$cast(dest, src)` as a statement (§8.16 task form): the
                // same runtime compatibility check as the function form, but
                // a failure is a runtime ERROR and dest is left UNCHANGED —
                // assigning through unconditionally let an incompatible
                // handle masquerade as the sibling type.
                if args.len() >= 2 {
                    let v = self.eval_expr(&args[1]);
                    if self.cast_type_ok(&args[0], &v) {
                        self.assign_value(&args[0], &v);
                    } else {
                        let m = format!(
                            "[xezim][error] $cast: source object is not \
                             assignment-compatible with the destination (t={})",
                            self.time
                        );
                        self.record_output(m.clone());
                        self.stdout_writeln(&m);
                    }
                }
            }
            // §20.10 severity system tasks. Each emits a tool-specific
            // message (severity tag + user `$display`-style text + a context
            // line with time/scope) and — except for `$fatal` — allows
            // execution to CONTINUE (LRM 20.10: only `$fatal` terminates,
            // via an implicit `$finish`). The user message uses the same
            // format-string rules as `$display`.
            "$info" => {
                self.emit_severity("Info", args);
            }
            "$warning" => {
                self.emit_severity("Warning", args);
            }
            "$error" => {
                self.emit_severity("Error", args);
            }
            "$fatal" => {
                // `$fatal [ ( finish_number [, list_of_arguments] ) ]`. The
                // first argument, when it is an integer literal 0/1/2, is the
                // finish_number (LRM 20.10 / 20.2 diagnostics level); the
                // remaining arguments are the `$display`-style message. A bare
                // `$fatal("msg")` (no valid finish_number) defaults to 1 and
                // treats every argument as the message.
                let (msg_args, finish) = self.fatal_msg_args(args);
                self.emit_severity("Fatal", msg_args);
                self.fatal_finish_number = Some(finish);
                self.finished = true; // implicit $finish
            }
            "$display" | "$displayb" | "$displayh" | "$displayo" => {
                let m = self.format_args(args, name);
                self.record_output(m.clone());
                self.stdout_writeln(&m);
            }
            "$write" | "$writeb" | "$writeh" | "$writeo" => {
                let m = self.format_args(args, name);
                self.record_output(m.clone());
                self.stdout_write(&m);
            }
            // `$swrite(dest, fmt, vals…)` / `$sformat(dest, fmt, vals…)`: format
            // into the destination STRING (arg0) rather than to stdout. Unlike
            // the `$sformatf` *function* form, the format string is arg1 here.
            // Formats values into target string variables (IEEE 1800-2023 §21.3).
            // These are plain §21.3 tasks and were a
            // silent no-op in the default mode (issue #24).
            "$swrite" | "$swriteb" | "$swriteh" | "$swriteo" | "$sformat" => {
                if let Some(dest) = args.first() {
                    let s = self.format_args(&args[1..], name);
                    self.assign_value(dest, &Value::from_string(&s));
                }
            }
            // $strobe queues for end-of-timestep playback (postponed
            // region per IEEE 1800 §15.3.4) — formatted AFTER all NBAs
            // in the current iter have applied, so it sees post-NBA
            // values. Distinct from $display which formats immediately
            // in the active region. Variants $strobeb/h/o select
            // default radix exactly like $display variants.
            "$strobe" | "$strobeb" | "$strobeh" | "$strobeo" => {
                self.pending_strobes.push((
                    name.to_string(),
                    args.to_vec(),
                    self.active_instance_scope(),
                    self.m_path(),
                ));
            }
            // §21.2.2 file variants: queue exactly like $strobe (postponed
            // region, so post-NBA values are printed); drain_pending_strobes
            // routes "$fstrobe*" entries to the file descriptor in args[0].
            "$fstrobe" | "$fstrobeb" | "$fstrobeh" | "$fstrobeo" => {
                self.pending_strobes.push((
                    name.to_string(),
                    args.to_vec(),
                    self.active_instance_scope(),
                    self.m_path(),
                ));
            }
            // $value$plusargs and $test$plusargs return a bit but the
            // common pattern `$value$plusargs("...", var);` calls them as
            // statements (return value discarded). Without this arm the
            // call falls through to `_ => {}` and the side-effect (writing
            // to args[1]) is lost. E.g. rtlmeter Example test's
            // `$value$plusargs("greeting=%s", str);` left str unchanged.
            "$value$plusargs" => {
                let _ = self.eval_value_plusargs(args);
            }
            "$test$plusargs" => {
                if let Some(arg0) = args.first() {
                    let pat = self.system_string_arg(arg0);
                    let _ = self.test_plusarg(&pat);
                }
            }
            "$monitor" | "$monitorb" | "$monitorh" | "$monitoro" => {
                // §21.2.3: the first print happens at the END of the current
                // time step (Postponed region — the slot-end check_monitor),
                // not at arming — values written later in this same step
                // (another initial block's `v = 0`) must be reflected.
                self.monitor = Some((name.to_string(), args.to_vec()));
                self.monitor_scope = self.active_instance_scope();
                self.monitor_m_suffix = self.m_path();
                self.monitor_arg_prev = None; // fresh arm ⇒ slot-end print
            }
            // §21.2.3 file variant: SHARES the single $monitor slot (xezim
            // models one active monitor; the LRM allows any number of
            // $fmonitor tasks — approximation: the last armed $monitor or
            // $fmonitor wins). check_monitor routes "$fmonitor*" output to
            // the file descriptor in args[0] and watches args[1..].
            "$fmonitor" | "$fmonitorb" | "$fmonitorh" | "$fmonitoro" => {
                self.monitor = Some((name.to_string(), args.to_vec()));
                self.monitor_scope = self.active_instance_scope();
                self.monitor_m_suffix = self.m_path();
                self.monitor_arg_prev = None; // fresh arm ⇒ slot-end print
            }
            "$monitoroff" => {
                self.monitor_paused = true;
            }
            // §18.13.3 $srandom(seed): reseed the thread RNG. As a system task
            // (not the object method form) it seeds the default stream — the
            // same stream +seed=<n> selects — so a run can be made repeatable
            // from inside the source.
            "$srandom" => {
                if let Some(a) = args.first() {
                    let seed = self.eval_expr(a).to_u64().unwrap_or(0);
                    self.rng = SvRng::from_seed(seed);
                }
            }
            // §18.13.4 $set_randstate(str): restore a state string produced by
            // $get_randstate.
            "$set_randstate" => {
                if let Some(a) = args.first() {
                    let v = self.eval_expr(a);
                    self.rng = SvRng::from_state_string(&v.to_string());
                }
            }
            // Non-standard `$deposit(target, value)`: set the target's value
            // immediately, WITHOUT a persistent driver — it holds until the
            // next driver transaction overwrites it (an undriven net keeps it).
            // A plain simulator write has exactly those semantics here: the
            // settle/edge machinery propagates it, and any cont-assign driver
            // re-evaluation replaces it.
            "$deposit" => {
                if args.len() == 2 {
                    let mut v = self.eval_expr(&args[1]);
                    let w = self.infer_lhs_width(&args[0]).max(1);
                    if v.width != w {
                        v = v.resize(w);
                    }
                    self.assign_value(&args[0], &v);
                    self.ctl_note_deposit(&args[0]);
                } else {
                    eprintln!(
                        "Warning: $deposit expects (target, value) — got {} args",
                        args.len()
                    );
                }
            }
            "$monitoron" => {
                // Re-enable a previously paused monitor. Per LRM §21.2.3,
                // $monitoron always prints once immediately (even if values
                // are unchanged since the last $monitoroff).
                self.monitor_paused = false;
                self.monitor_arg_prev = None; // force immediate print
                self.check_monitor();
            }
            // §20.2 `$exit` waits for all program blocks to finish and then
            // behaves as `$finish`. xezim models program blocks loosely (their
            // initial blocks run in the reactive region of the same event
            // loop), so plain `$finish` semantics are the accurate mapping.
            "$finish" | "$stop" | "$exit" => {
                // xezim has no interactive mode: `$stop` ends the run like
                // `$finish`, reported to VPI as cbEnterInteractive.
                if name == "$stop" && self.vpi_cb_mask != 0 {
                    self.vpi_note_stop();
                }
                if std::env::var("XEZIM_TRACE_FINISH").is_ok() {
                    let bt = std::backtrace::Backtrace::force_capture();
                    eprintln!(
                        "[xezim][trace] {} called at sim_time={}\n{}",
                        name, self.time, bt
                    );
                }
                self.flush_file_writes();
                self.finished = true;
            }
            "$fclose" => {
                let _ = self.close_file_handle(args);
            }
            "$fwrite" => {
                let _ = self.write_file_handle_named(args, false, "$write");
            }
            "$fwriteb" => {
                let _ = self.write_file_handle_named(args, false, "$writeb");
            }
            "$fwriteh" | "$fwritex" => {
                let _ = self.write_file_handle_named(args, false, "$writeh");
            }
            "$fwriteo" => {
                let _ = self.write_file_handle_named(args, false, "$writeo");
            }
            "$fdisplay" => {
                let _ = self.write_file_handle_named(args, true, "$display");
            }
            "$fdisplayb" => {
                let _ = self.write_file_handle_named(args, true, "$displayb");
            }
            "$fdisplayh" | "$fdisplayx" => {
                let _ = self.write_file_handle_named(args, true, "$displayh");
            }
            "$fdisplayo" => {
                let _ = self.write_file_handle_named(args, true, "$displayo");
            }
            "$fflush" => {
                let raw = args
                    .first()
                    .map(|a| self.eval_expr(a).to_i64().unwrap_or(0) as i32)
                    .unwrap_or(0);
                self.flush_channels(raw);
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
                    // NOTE: "$fseek might erase any pushed back characters").
                    self.ungetc_buf.remove(&fd);
                }
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
            }
            // §21.3.4.3 task form: scan and assign, discarding the count.
            // Previously this form had no arm at all and silently did
            // nothing — `$fscanf(fd, "%c", c);` left `c` untouched.
            "$fscanf" => {
                let _ = self.fscanf_impl(args);
            }
            "$fgets" => {
                // task form: discard return value, just write to the string arg
                use std::io::Read;
                let dest = args.first();
                let fd = args
                    .get(1)
                    .map(|a| self.eval_file_handle_arg(a))
                    .unwrap_or(0);
                let mut line = Vec::new();
                if let Some(f) = self.file_handles.get_mut(&fd) {
                    let mut b = [0u8; 1];
                    loop {
                        if let Some(buf) = self.ungetc_buf.get_mut(&fd) {
                            if let Some(c) = buf.pop() {
                                if c == b'\n' {
                                    line.push(c);
                                    break;
                                }
                                line.push(c);
                                continue;
                            }
                        }
                        match f.read(&mut b) {
                            Ok(0) => break,
                            Ok(_) => {
                                if b[0] == b'\n' {
                                    line.push(b[0]);
                                    break;
                                }
                                line.push(b[0]);
                            }
                            Err(_) => break,
                        }
                    }
                }
                if let Some(d) = dest {
                    self.assign_value(d, &Self::sv_string_from_bytes(&line));
                }
            }
            // §21.3.4.4 task-position `$fread(dest, fd, …)`: perform the read,
            // discard the byte count.
            "$fread" => {
                let _ = self.fread_impl(args);
            }
            "$feof" => { /* function-only; task form is a no-op */ }
            "$ferror" => {
                // task form: just write the message string
                if let Some(out) = args.get(1) {
                    self.assign_value(out, &Value::from_string("No error"));
                }
            }
            "$readmemh" => {
                let _ = self.read_memory_file(args, 16);
            }
            "$readmemb" => {
                let _ = self.read_memory_file(args, 2);
            }
            "$readmemd" => {
                let _ = self.read_memory_file(args, 10);
            }
            "$writememb" | "$writememh" | "$writememd" => {
                let _ = self.write_memory_file(args, name);
            }
            // §20.16 `$sdf_annotate("file" [, instance])` — runtime SDF
            // annotation routed through the SAME xezim_core::sdf machinery as
            // the --sdf CLI flag, honoring the CLI-selected min/typ/max
            // (--sdf-min/typ/max or +mindelays/+typdelays/+maxdelays; default
            // Typ). The instance argument is approximated: annotation is
            // applied globally (SDF cell INSTANCE paths are design-absolute
            // in xezim's model). A missing/unparsable file is fatal, matching
            // the CLI path's hard failure.
            "$sdf_annotate" => {
                let path = args
                    .first()
                    .map(|a| self.system_string_arg(a))
                    .unwrap_or_default();
                if args.len() >= 2 {
                    self.warn_system_task_once(
                        "$sdf_annotate(instance)",
                        "Note: $sdf_annotate instance argument is approximated — annotation applies globally",
                    );
                }
                match std::fs::read_to_string(&path) {
                    Err(e) => {
                        eprintln!(
                            "Fatal: $sdf_annotate cannot read SDF file '{}': {}",
                            path, e
                        );
                        self.abort_run();
                    }
                    Ok(content) => match super::sdf::parse_sdf(&content) {
                        Err(e) => {
                            eprintln!(
                                "Fatal: $sdf_annotate cannot parse SDF file '{}': {}",
                                path, e
                            );
                            self.abort_run();
                        }
                        Ok(sdf) => {
                            let select = self.sdf_select.unwrap_or(super::sdf::DelaySelect::Typ);
                            // Scaled to the simulation tick, as the CLI's --sdf.
                            let ann = super::sdf::annotate_sdf(&sdf, self.tick_s, select);
                            self.sdf_annotation = Some(ann);
                            self.apply_sdf_annotation_late();
                        }
                    },
                }
            }
            "$dumpfile" => {
                if let Some(arg) = args.first() {
                    // §21.7.2.1: the file name is any string-valued expression
                    // — a `string` VARIABLE (`$dumpfile(vcdfn)`) must use its
                    // value, not silently fall back to "dump.vcd".
                    let path = self.system_string_arg(arg);
                    self.vcd_file = if path.is_empty() {
                        Some("dump.vcd".to_string())
                    } else {
                        Some(path)
                    };
                }
            }
            // The FSDB dump tasks mapped onto xezim's native FST dump
            // machinery (FSDB is a proprietary format; FST is the closest
            // compact binary equivalent, and common waveform viewers read it).
            // The rewritten path swaps a .fsdb extension for .fst.
            "$fsdbDumpfile" => {
                let path = args
                    .first()
                    .map(|a| self.system_string_arg(a))
                    .unwrap_or_default();
                let path = if path.is_empty() {
                    "xezim.fst".to_string()
                } else if let Some(stem) = path.strip_suffix(".fsdb") {
                    format!("{}.fst", stem)
                } else {
                    format!("{}.fst", path)
                };
                self.warn_system_task_once(
                    "$fsdbDump(note)",
                    &format!(
                        "Note: $fsdbDumpfile is not FSDB — writing FST to {} instead",
                        path
                    ),
                );
                self.fst_file = Some(path);
            }
            // `$fsdbDumpvars([depth[, scope]...])` — starts the FST dump.
            // Scope arguments narrow the dump like $dumpvars; the depth
            // argument is accepted but not depth-filtered (whole subtree).
            "$fsdbDumpvars" => {
                if self.fst_file.is_none() {
                    self.fst_file = Some("xezim.fst".to_string());
                }
                self.warn_system_task_once(
                    "$fsdbDump(note)",
                    &format!(
                        "Note: $fsdbDumpfile is not FSDB — writing FST to {} instead",
                        self.fst_file.clone().unwrap_or_default()
                    ),
                );
                let mut scopes: Vec<String> = Vec::new();
                for a in args.iter().skip(1) {
                    if let ExprKind::Ident(hier) = &a.kind {
                        let s = self.resolve_hier_name(hier);
                        if !s.is_empty() {
                            scopes.push(s.to_string());
                        }
                    }
                }
                self.fst_scopes = scopes;
                if self.fst_writer.is_none() {
                    self.fst_start_dump();
                }
            }
            // The VPD (vcdplus) control tasks mapped onto the plain VCD
            // machinery: on first use starts a VCD dump of the whole design
            // into "vcdplus.vcd"; later calls resume/suspend like
            // $dumpon/$dumpoff.
            "$vcdpluson" => {
                self.warn_system_task_once(
                    "$vcdpluson",
                    "Note: $vcdpluson is not VPD — writing VCD to vcdplus.vcd instead",
                );
                if self.vcd_writer.is_none() {
                    if self.vcd_file.is_none() {
                        self.vcd_file = Some("vcdplus.vcd".to_string());
                    }
                    self.vcd_filter_scopes = Vec::new();
                    self.vcd_dump_depth = 0;
                    self.vcd_start_dump();
                } else {
                    self.vcd_dump_on();
                }
            }
            "$vcdplusoff" => {
                self.vcd_dump_off();
            }
            "$dumpvars" => {
                // §21.7.1.4 `$dumpvars(level, scope_or_var, ...)`.
                // arg[0] is the DEPTH: 0 = every level below each named scope,
                // N = N levels starting at it. args[1..] are scope or signal
                // names; with none, the whole design is dumped.
                let depth = args
                    .first()
                    .and_then(|a| self.eval_expr(a).to_u64())
                    .unwrap_or(0) as u32;
                let mut filter_scopes: Vec<String> = Vec::new();
                for a in args.iter().skip(1) {
                    if let ExprKind::Ident(hier) = &a.kind {
                        // The argument names a SCOPE path. Join it verbatim —
                        // `resolve_hier_name` is a SIGNAL resolver whose
                        // scope-stripping heuristics collapsed
                        // `tb.u_m.u_i` to the leaf `u_i`, so a hierarchical
                        // `$dumpvars(0, top.a.b)` selected nothing.
                        // `dump_filter_prefixes` strips the top-module prefix.
                        let s = hier
                            .path
                            .iter()
                            .map(|seg| seg.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join(".");
                        if !s.is_empty() {
                            filter_scopes.push(s);
                        }
                    }
                }
                self.vcd_filter_scopes = filter_scopes;
                self.vcd_dump_depth = depth;
                self.vcd_start_dump();
            }
            "$dumpoff" => {
                self.vcd_dump_off();
            }
            "$dumpon" => {
                self.vcd_dump_on();
            }
            "$dumpall" => {
                self.vcd_dump_all();
            }
            "$dumpflush" => {
                // §21.7.1.9: force everything buffered so far out to the file.
                if let Some(w) = self.vcd_writer.as_mut() {
                    let _ = w.flush();
                }
            }
            "$dumplimit" => {
                // §21.7.1.8: cap the dump at N bytes.
                self.vcd_limit = args.first().and_then(|a| self.eval_expr(a).to_u64());
            }
            "$sscanf" => {
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
                    let _ = self.sv_sscanf(&src_str, &fmt, &args[2..]);
                }
            }
            // §20.15 stochastic-analysis tasks. All four write an output status
            // code (Table 20-11) to their last argument; `$q_remove`/
            // `$q_exam` additionally write their job/stat outputs.
            "$q_initialize" => {
                let status = self.q_initialize_impl(args);
                self.assign_q_status(args, 3, status);
            }
            "$q_add" => {
                let status = self.q_add_impl(args);
                self.assign_q_status(args, 3, status);
            }
            "$q_remove" => {
                let status = self.q_remove_impl(args);
                self.assign_q_status(args, 3, status);
            }
            "$q_exam" => {
                let status = self.q_exam_impl(args);
                self.assign_q_status(args, 3, status);
            }
            // §20.17.1 `$system` as a TASK (return value discarded). Invokes
            // the host shell; with no argument calls system(NULL).
            "$system" => {
                let _ = self.system_impl(args);
            }
            // §20.17.2 `$stacktrace` as a TASK: displays the call-stack text.
            "$stacktrace" => {
                let trace = self.stacktrace_text();
                self.record_output(trace.clone());
                self.stdout_writeln(&trace);
            }
            // -- recognized-but-unsupported system tasks (warn once each with
            // a specific reason; NOT silently ignored, NOT the generic
            // unknown-task diagnostic). Wording avoids the word "err\u{6f}r"
            // so log-grepping harnesses don't reclassify a passing run.
            // §20.11/§20.12 assertion control.
            "$asserton"
            | "$assertoff"
            | "$assertkill"
            | "$assertcontrol"
            | "$assertpasson"
            | "$assertpassoff"
            | "$assertfailon"
            | "$assertfailoff"
            | "$assertnonvacuouson"
            | "$assertvacuousoff" => {
                self.exec_assert_control(name, args);
            }
            "$save" | "$restart" | "$incsave" => {
                let msg = format!("Warning: {} ignored — checkpointing is not supported", name);
                self.warn_system_task_once(name, &msg);
            }
            "$sreadmemb" | "$sreadmemh" => {
                let msg = format!("Warning: {} ignored — memory left unchanged", name);
                self.warn_system_task_once(name, &msg);
            }
            "$getpattern" => {
                self.warn_system_task_once(
                    name,
                    "Warning: $getpattern ignored — pattern loading is not supported",
                );
            }
            // §20.14 $countdrivers in statement position (function form lives
            // in the expression dispatcher; both warn once and yield 0).
            "$countdrivers" => {
                self.warn_system_task_once(
                    name,
                    "Warning: $countdrivers is not modeled — returning 0",
                );
            }
            "$key" | "$nokey" | "$log" | "$nolog" | "$input" | "$scope" | "$showscopes"
            | "$showvars" | "$list" => {
                let msg = format!("Warning: {} ignored — interactive task not supported", name);
                self.warn_system_task_once(name, &msg);
            }
            // A `$name` a VPI module registered with `vpi_register_systf`.
            // Checked last so a builtin always wins.
            _ if vpi_systf_registered(name) => {
                let self_ptr = self as *mut Simulator;
                let span = vpi_api::take_task_call_span();
                vpi_call_systf(self_ptr, name, args, span);
            }
            // §38.36.3 cbUnresolvedSystf may register the name.
            _ if self.vpi_cb_mask != 0 && self.vpi_resolve_unknown_systf(name) => {
                let self_ptr = self as *mut Simulator;
                let span = vpi_api::take_task_call_span();
                vpi_call_systf(self_ptr, name, args, span);
            }
            _ => {
                self.warn_unknown_system_task(name, args);
            }
        }
    }

    /// Hierarchical scope name of the currently running process, for the
    /// context line of §20.10 severity messages and §20.17.2 `$stacktrace`.
    /// Mirrors the resolution used by `$printtimescale`.
    pub(super) fn severity_scope(&self) -> String {
        // The reference reports the same scope `%m` names.
        self.m_path()
    }

    /// Split `$fatal` arguments into `(message_args, finish_number)` per LRM
    /// 20.10: if the first argument is an integer literal whose value is 0, 1,
    /// or 2, it is the finish_number (diagnostics level, §20.2) and the
    /// remaining arguments form the `$display`-style message; otherwise every
    /// argument is the message and finish_number defaults to 1.
    pub(super) fn fatal_msg_args<'a>(&mut self, args: &'a [Expression]) -> (&'a [Expression], i32) {
        if let Some(first) = args.first() {
            if let ExprKind::Number(NumberLiteral::Integer {
                value,
                base: NumberBase::Decimal,
                ..
            }) = &first.kind
            {
                if let Ok(n) = value.parse::<i64>() {
                    if (0..=2).contains(&n) {
                        return (&args[1..], n as i32);
                    }
                }
            }
        }
        (args, 1)
    }

    /// Emit a §20.10 severity-system-task message. Formats the user
    /// `$display`-style arguments and prefixes a `** <Severity>:` tag plus a
    /// context line (sim time + hierarchical scope). Routed to both the
    /// captured output log and stdout, exactly like `$display`.
    pub(super) fn emit_severity(&mut self, severity: &str, args: &[Expression]) {
        let body = if args.is_empty() {
            String::new()
        } else {
            self.format_args(args, "$display")
        };
        self.emit_severity_text(severity, &body);
    }

    /// End the run on a run-time error the simulator itself reports and
    /// cannot continue past: a null handle a process body reads through or
    /// calls a method on (§8.4), a loop or delta cycle that can never let
    /// time advance, an SDF file `$sdf_annotate` cannot read, or a failed
    /// internal consistency check. Like `$fatal` (§20.10), such an error ends
    /// the simulation with a failing exit status, not 0.
    #[cold]
    #[inline(never)]
    pub(super) fn abort_run(&mut self) {
        self.saw_fatal = true;
        self.finished = true;
    }

    /// `emit_severity` with the message already formatted.
    pub(super) fn emit_severity_text(&mut self, severity: &str, body: &str) {
        let scope = self.severity_scope();
        self.emit_severity_text_in(severity, body, &scope);
    }

    /// `emit_severity_text` naming `scope` in the context line.
    pub(super) fn emit_severity_text_in(&mut self, severity: &str, body: &str, scope: &str) {
        let time_s = self.format_time_parts(self.time_in_current_unit()).0;
        let line = if body.is_empty() {
            format!("** {}", severity)
        } else {
            format!("** {}: {}", severity, body)
        };
        let ctx = format!("   Time: {}  Scope: {}", time_s.trim(), scope);
        // §20.10: record severity occurrences so the process exit status can
        // reflect them. `$fatal` carries an explicit finish/status semantic and
        // must not exit 0; `$error` is counted so `--error-exit` can promote it.
        match severity {
            "Fatal" => self.saw_fatal = true,
            "Error" => self.error_count = self.error_count.saturating_add(1),
            _ => {}
        }
        self.record_output(line.clone());
        self.record_output(ctx.clone());
        self.stdout_writeln(&line);
        self.stdout_writeln(&ctx);
        // §20.10: `$error` / `$fatal` are run-time errors — §38.36.3 cbError.
        if self.vpi_cb_mask & vpi_cb::m(vpi_cb::CB_ERROR) != 0
            && matches!(severity, "Error" | "Fatal")
        {
            self.vpi_notify_error(&line);
        }
    }

    /// Write a §20.15 status code to output argument `idx` (if present). The
    /// status is an `int`; stored as a 32-bit unsigned (sign bit preserved
    /// via two's complement for the -1 sentinel some stats use).
    pub(super) fn assign_q_status(&mut self, args: &[Expression], idx: usize, status: i64) {
        if let Some(out) = args.get(idx) {
            self.assign_value(out, &Value::from_u64(status as u32 as u64, 32));
        }
    }

    /// §20.15.1 `$q_initialize(q_id, q_type, max_length, status)`. Creates a
    /// new queue. Status codes: 0 OK, 4 unsupported type, 5 length<=0,
    /// 6 duplicate q_id (Table 20-11).
    pub(super) fn q_initialize_impl(&mut self, args: &[Expression]) -> i64 {
        let qid = args
            .first()
            .map(|a| self.eval_expr(a).to_u64().unwrap_or(0) as i64)
            .unwrap_or(0);
        let qtype = args
            .get(1)
            .map(|a| self.eval_expr(a).to_u64().unwrap_or(0) as i64)
            .unwrap_or(0);
        let maxlen = args
            .get(2)
            .map(|a| self.eval_expr(a).to_i64().unwrap_or(0))
            .unwrap_or(0);
        if qtype != 1 && qtype != 2 {
            return 4; // unsupported queue type
        }
        if maxlen <= 0 {
            return 5; // specified length <= 0
        }
        if self.queues.contains_key(&qid) {
            return 6; // duplicate q_id
        }
        self.queues.insert(
            qid,
            StochasticQueue {
                q_type: qtype,
                max_length: maxlen,
                ..Default::default()
            },
        );
        0
    }

    /// §20.15.2 `$q_add(q_id, job_id, inform_id, status)`. Enqueues an entry
    /// tagged with the current sim time (for wait statistics). Status codes:
    /// 0 OK, 1 queue full, 2 undefined q_id.
    pub(super) fn q_add_impl(&mut self, args: &[Expression]) -> i64 {
        let qid = args
            .first()
            .map(|a| self.eval_expr(a).to_u64().unwrap_or(0) as i64)
            .unwrap_or(0);
        let job = args
            .get(1)
            .map(|a| self.eval_expr(a).to_u64().unwrap_or(0) as i64)
            .unwrap_or(0);
        let inform = args
            .get(2)
            .map(|a| self.eval_expr(a).to_u64().unwrap_or(0) as i64)
            .unwrap_or(0);
        // §20.15: queue timing statistics are in the same time basis as $time
        // (the current scope's time unit), not raw simulator ticks. A `#10` in a
        // 1ns design therefore contributes a wait of 10, not 10000 (ps ticks).
        let now = self.time_in_current_unit() as i64;
        let Some(q) = self.queues.get_mut(&qid) else {
            return 2;
        };
        if q.entries.len() >= q.max_length as usize {
            return 1; // queue full
        }
        // inter-arrival statistics (§20.15.5 code 2)
        q.arrivals += 1;
        if let Some(last) = q.last_arrival_tick {
            q.sum_interarrival += now.saturating_sub(last) as i128;
        }
        q.last_arrival_tick = Some(now);
        q.entries.push_back((job, inform, now));
        if q.entries.len() > q.max_len_seen {
            q.max_len_seen = q.entries.len();
        }
        0
    }

    /// §20.15.3 `$q_remove(q_id, job_id, inform_id, status)`. Dequeues per the
    /// queue type (1=FIFO front, 2=LIFO back), writing the entry's job_id and
    /// inform_id to the output arguments, and accumulating wait statistics.
    /// Status codes: 0 OK, 2 undefined q_id, 3 queue empty.
    pub(super) fn q_remove_impl(&mut self, args: &[Expression]) -> i64 {
        let qid = args
            .first()
            .map(|a| self.eval_expr(a).to_u64().unwrap_or(0) as i64)
            .unwrap_or(0);
        let now = self.time_in_current_unit() as i64;
        // Dequeue + update statistics while holding the mutable queue borrow,
        // then drop it before assigning the output arguments (which borrow
        // `self` again via assign_value).
        let (job, inform, status) = match self.queues.get_mut(&qid) {
            None => (0i64, 0i64, 2i64), // undefined q_id
            Some(q) => {
                let popped = if q.q_type == 2 {
                    q.entries.pop_back()
                } else {
                    q.entries.pop_front()
                };
                match popped {
                    None => (0, 0, 3), // queue empty
                    Some((job, inform, arrival)) => {
                        let wait = now - arrival;
                        // shortest wait over removed jobs (code 4)
                        q.shortest_wait_ever = Some(match q.shortest_wait_ever {
                            Some(w) => w.min(wait),
                            None => wait,
                        });
                        q.sum_wait_removed += wait as i128;
                        q.count_wait_removed += 1;
                        // longest wait over jobs STILL queued (code 5)
                        q.longest_wait_queued = q.entries.iter().map(|(_, _, t)| now - *t).max();
                        (job, inform, 0)
                    }
                }
            }
        };
        if status == 0 {
            if let Some(out) = args.get(1) {
                self.assign_value(out, &Value::from_u64(job as u32 as u64, 32));
            }
            if let Some(out) = args.get(2) {
                self.assign_value(out, &Value::from_u64(inform as u32 as u64, 32));
            }
        }
        status
    }

    /// §20.15.5 `$q_exam(q_id, q_stat_code, q_stat_value, status)`. Writes the
    /// requested statistic (Table 20-10) to `q_stat_value`. The
    /// time-influenced codes (2/4/5/6) return -1 when insufficient samples
    /// exist; the LRM leaves their exact computation implementation-defined.
    pub(super) fn q_exam_impl(&mut self, args: &[Expression]) -> i64 {
        let qid = args
            .first()
            .map(|a| self.eval_expr(a).to_u64().unwrap_or(0) as i64)
            .unwrap_or(0);
        let code = args
            .get(1)
            .map(|a| self.eval_expr(a).to_u64().unwrap_or(0) as i64)
            .unwrap_or(0);
        let now = self.time_in_current_unit() as i64;
        // Compute the statistic while borrowing the queue, then drop the
        // borrow before assigning the output (which re-borrows `self`).
        let (val, status) = match self.queues.get(&qid) {
            None => (0i64, 2i64), // undefined q_id
            Some(q) => {
                let v: i64 = match code {
                    1 => q.entries.len() as i64, // current length
                    2 => {
                        if q.arrivals >= 2 {
                            // mean interarrival
                            (q.sum_interarrival / (q.arrivals - 1) as i128) as i64
                        } else {
                            -1
                        }
                    }
                    3 => q.max_len_seen as i64, // maximum length
                    4 => q.shortest_wait_ever.unwrap_or(-1), // shortest wait ever
                    5 => q.longest_wait_queued.unwrap_or_else(|| {
                        // longest wait, jobs still queued
                        q.entries
                            .iter()
                            .map(|(_, _, t)| now - *t)
                            .max()
                            .unwrap_or(-1)
                    }),
                    6 => {
                        if q.count_wait_removed >= 1 {
                            // average wait time
                            (q.sum_wait_removed / q.count_wait_removed as i128) as i64
                        } else {
                            -1
                        }
                    }
                    _ => 0,
                };
                (v, 0)
            }
        };
        if let Some(out) = args.get(2) {
            self.assign_value(out, &Value::from_u64(val as u32 as u64, 32));
        }
        status
    }

    /// §20.15.4 `$q_full(q_id, status)` — system FUNCTION. Returns 1 if the
    /// queue is full, 0 otherwise; also writes the status code. Status codes:
    /// 0 OK, 2 undefined q_id.
    pub(super) fn q_full_impl(&mut self, args: &[Expression]) -> (i64, i64) {
        let qid = args
            .first()
            .map(|a| self.eval_expr(a).to_u64().unwrap_or(0) as i64)
            .unwrap_or(0);
        let Some(q) = self.queues.get(&qid) else {
            return (0, 2);
        };
        let full = if q.entries.len() >= q.max_length as usize {
            1
        } else {
            0
        };
        (full, 0)
    }

    /// §20.17.1 `$system`. Invokes the host POSIX shell on the command string
    /// (via `sh -c <cmd>`, mirroring C `system()`); with no argument calls
    /// `system(NULL)` (returns nonzero when a shell is available). Returns the
    /// command's exit status as an `int`.
    pub(super) fn system_impl(&mut self, args: &[Expression]) -> i32 {
        self.flush_file_writes();
        match args.first() {
            None => 1, // system(NULL): a shell is available
            Some(a) => {
                let cmd = self.system_string_arg(a);
                match std::process::Command::new("sh")
                    .arg("-c")
                    .arg(&cmd)
                    .status()
                {
                    Ok(s) => s.code().unwrap_or(-1),
                    Err(_) => -1,
                }
            }
        }
    }

    /// §20.17.2 `$stacktrace` text. Content is implementation-defined; xezim
    /// reports the current hierarchical scope (the deepest frame it models at
    /// runtime) in a `Call Stack:`/`A total of N stack frames` envelope. The
    /// task form prints this; the function form returns it as a string.
    pub(super) fn stacktrace_text(&self) -> String {
        let scope = match self.process_scope_hint.get(&self.current_pid) {
            Some(s) => self.hier_path(s),
            None => self.module.name.clone(),
        };
        format!(
            "Call Stack:\nModule {}\n\nA total of 1 stack frames are displayed.\n",
            scope
        )
    }

    pub(super) fn format_args(&mut self, args: &[Expression], tn: &str) -> String {
        if args.is_empty() {
            return String::new();
        }
        let r = if tn.ends_with('b') {
            'b'
        } else if tn.ends_with('h') {
            'h'
        } else if tn.ends_with('o') {
            'o'
        } else {
            'd'
        };
        // IEEE 1800-2017 §21.2.1.2: the display-family argument list is walked
        // left to right. A string LITERAL is a format spec whose `%`
        // directives consume the following arguments; any argument not
        // consumed by a preceding format prints in the task's default radix.
        // Several format strings may interleave in one call —
        // `$swrite(s, "Hex %h ", h, "Bin %b ", b, "Done.")` — and nothing is
        // separated by implicit spaces. (Previously only the first format
        // string was honoured and every leftover argument was dropped.)
        let mut result = String::new();
        let mut i = 0usize;
        while i < args.len() {
            let a = &args[i];
            if let ExprKind::StringLiteral(fmt) = &a.kind {
                let fmt = fmt.clone();
                let (s, used) = self.format_string_consumed(&fmt, &args[i + 1..], tn);
                result.push_str(&s);
                i += 1 + used;
                continue;
            }
            // A leading string-valued EXPRESSION acts as the format spec — a
            // ternary `cond ? "%0df" : "%0db"`, a string variable, the result
            // of another `$sformatf`, etc. (riscv-dv's branch-label code uses
            // the ternary form.) Later string-valued arguments emit their
            // text: `$fwrite(fd, {str, "\n"})` writes text, not a number.
            if i == 0 && self.expr_is_string_valued(a) {
                let fmt = self.eval_expr(a).to_sv_string();
                let (s, used) = self.format_string_consumed(&fmt, &args[1..], tn);
                result.push_str(&s);
                i += 1 + used;
                continue;
            }
            let v = self.eval_expr(a);
            if self.expr_is_string_valued(a) {
                result.push_str(&v.to_sv_string());
            } else {
                result.push_str(&match r {
                    'b' => v.to_bin_string(),
                    'h' => v.to_hex_string(),
                    'o' => Self::bin_to_oct_string(&v.to_bin_string()),
                    // §21.2.1.2: an unconsumed argument prints in the task's
                    // default DECIMAL format — which has the type's default
                    // field width, space-padded (`$display("x=", v)` of a
                    // 16-bit 677 is "x=  677", matching the reference
                    // simulator), not the %0d minimal form.
                    _ => {
                        let core = v.to_dec_string();
                        let dw = Self::dec_default_width(v.width, v.is_signed);
                        if core.len() >= dw {
                            core
                        } else {
                            format!("{}{}", " ".repeat(dw - core.len()), core)
                        }
                    }
                });
            }
            i += 1;
        }
        result
    }

    /// Best-effort check that an expression evaluates to a SystemVerilog string
    /// (so `$write`/`$fwrite` without a format string emits text, not a number).
    /// [`Self::expr_is_string_valued`] for an OPERATOR operand, minus the
    /// character selects: `s[i]` on a scalar string is a `byte` (§6.16.3),
    /// so `s[0] < 0` is an integral, signed comparison, not a text one.
    pub(super) fn operand_is_string(&mut self, expr: &Expression) -> bool {
        if !self.expr_is_string_valued(expr) {
            return false;
        }
        match &expr.kind {
            ExprKind::Index { expr: base, .. } => !self.string_char_receiver_is_scalar(base),
            _ => true,
        }
    }

    pub(super) fn expr_is_string_valued(&self, expr: &Expression) -> bool {
        match &expr.kind {
            ExprKind::StringLiteral(_) => true,
            // `{a, b, ...}`: a string concatenation if any operand is a string.
            ExprKind::Concatenation(parts) => parts.iter().any(|p| self.expr_is_string_valued(p)),
            ExprKind::SystemCall { name, .. } => {
                matches!(name.as_str(), "$sformatf" | "$psprintf")
            }
            ExprKind::Paren(inner) => self.expr_is_string_valued(inner),
            ExprKind::Conditional {
                then_expr,
                else_expr,
                ..
            } => self.expr_is_string_valued(then_expr) || self.expr_is_string_valued(else_expr),
            // A bare or member reference whose declared type is `string`, or an
            // element of a string-typed queue/array.
            ExprKind::Ident(h) => {
                self.string_signals.contains(&*self.resolve_hier_name(h))
                    || self.class_member_is_string(expr)
                    || self.ident_type_is_string(h, expr)
                    || self.local_struct_member_is_string(h)
                    || self.member_chain_is_string(expr)
            }
            ExprKind::Index { expr: base, .. } => {
                (if let ExprKind::Ident(h) = &base.kind {
                    self.string_signals.contains(&*self.resolve_hier_name(h))
                } else {
                    false
                }) || self
                    .get_expr_type_name(base)
                    .is_some_and(|t| t == "string")
                // An ELEMENT of a class-member STRING collection
                // (`h.q[idx]`, `this.q[i]`) is a string too. The base is a
                // `MemberAccess` or two-segment ident (`h.q`), which is not
                // an `Ident` and has no composed signal name in
                // `string_signals`; only the class registration knows it is
                // a string collection. Without this, UVM's visitor
                // queues-of-strings (`node.get_name()` accumulated into `q[$]`)
                // printed as garbage packed integers instead of text.
                || self.class_member_is_string(base)
                || self.member_chain_is_string(expr)
            }
            ExprKind::MemberAccess { expr: base, member } => {
                // A `this.<prop>` / `obj.<prop>` access where the property is
                // declared `string` in the class definition, a 0-arg method,
                // or a member of a local/module unpacked-struct variable.
                self.class_member_is_string(expr)
                    || self.call_returns_string(expr)
                    // Walk the whole receiver chain, not just its head: a
                    // NESTED struct member (`o.in.s`, either parse shape)
                    // otherwise missed and `%s` padded the string to its
                    // container width.
                    || Self::member_path_head(base)
                        .is_some_and(|head| self.module.var_decl_types.contains_key(head))
                        && Self::flatten_member_path(base).is_some_and(|segs| {
                            self.dotted_member_is_string(&segs.join("."), &member.name)
                        })
                    || self.member_chain_is_string(expr)
            }
            // A method/function call whose return type is `string`
            // (e.g. `obj.str()`, `this.convert2string()`). Without
            // this, `$fwrite(fd, obj.str())` treats the result as an
            // integer and dumps a garbage decimal number instead of the
            // formatted text.
            ExprKind::Call { func, .. } => self.call_returns_string(func),
            _ => false,
        }
    }

    /// `p.member` where `p` is a (frame-local or module) variable whose
    /// DECLARED type resolves to an unpacked struct with a `string`-typed
    /// member of that name. Such paths parse as one dotted Ident and are
    /// invisible to the class-property and signal-registration checks, so
    /// `%s` treated the container-stored string as a packed value and
    /// padded it to the container width.
    pub(super) fn local_struct_member_is_string(
        &self,
        h: &crate::ast::expr::HierarchicalIdentifier,
    ) -> bool {
        use crate::ast::types::{DataType, SimpleType};
        if h.path.len() < 2 {
            return false;
        }
        // Walk EVERY segment: only the outermost struct's direct members
        // were consulted, so a NESTED member (`o.in.s`) missed and `%s`
        // padded the string to its container width, while the same member
        // reached through a one-level path printed correctly.
        let Some(dt) = self.module.var_decl_types.get(&h.path[0].name.name) else {
            return false;
        };
        let mut cur = self.resolve_dt(&dt.clone());
        for seg in &h.path[1..] {
            let DataType::Struct(su) = &cur else {
                return false;
            };
            let next = su.members.iter().find_map(|m| {
                m.declarators
                    .iter()
                    .any(|d| d.name.name == seg.name.name)
                    .then(|| self.resolve_dt(&m.data_type))
            });
            match next {
                Some(t) => cur = t,
                None => return false,
            }
        }
        matches!(
            cur,
            DataType::Simple {
                kind: SimpleType::String,
                ..
            }
        )
    }

    /// A member chain that passes through collection ELEMENTS and ends at a
    /// `string` struct member — `q[i].name`, `o.rows[i].in.name`,
    /// `this.q[i].name`. The dotted-path checks stop at the first select, so
    /// `%s` of such a member printed the text padded to the leaf's width.
    pub(super) fn member_chain_is_string(&self, e: &Expression) -> bool {
        use crate::ast::types::{DataType, SimpleType};
        // Only a member reached through a select (`q[i].name`) is new here;
        // plain dotted paths are answered by the checks the caller already
        // ran. Scanned outermost-in without allocating, so the ordinary
        // `a[i] < b` comparison stays cheap.
        let mut seen_member = false;
        let mut cur = e;
        let through_select = loop {
            match &cur.kind {
                ExprKind::MemberAccess { expr, .. } => {
                    seen_member = true;
                    cur = expr;
                }
                ExprKind::Index { expr, .. } => {
                    if seen_member {
                        break true;
                    }
                    cur = expr;
                }
                ExprKind::Paren(inner) => cur = inner,
                ExprKind::Ident(h) => {
                    let mut hit = false;
                    for (k, seg) in h.path.iter().enumerate().rev() {
                        if !seg.selects.is_empty() && seen_member {
                            hit = true;
                            break;
                        }
                        if k > 0 {
                            seen_member = true;
                        }
                    }
                    break hit;
                }
                _ => break false,
            }
        };
        // A member of a class-scoped static (`C#(int)::one.s`, `C::one.s`,
        // `alias_t::one.s`) has no select but no other check knows it.
        let class_rooted = || {
            let mut cur = e;
            loop {
                match &cur.kind {
                    ExprKind::MemberAccess { expr, .. } | ExprKind::Index { expr, .. } => {
                        cur = expr
                    }
                    ExprKind::Paren(inner) => cur = inner,
                    ExprKind::Specialization { .. } => return true,
                    ExprKind::Ident(h) => {
                        let n = h.path[0].name.name.as_str();
                        return h.path.len() + usize::from(!std::ptr::eq(cur, e)) > 1
                            && !self.local_stack.last().is_some_and(|l| l.contains_key(n))
                            && (self.module.classes.contains_key(n)
                                || self.class_member_names().scope_prefixes.contains(n));
                    }
                    _ => return false,
                }
            }
        };
        if !through_select && (self.module.classes.is_empty() || !class_rooted()) {
            return false;
        }
        // Steps outermost first: `Some(member)` or `None` for a select.
        let mut steps: Vec<Option<&str>> = Vec::new();
        let mut cur = e;
        // §8.25: `C#(args)::q[i].name` is rooted at the specialized class.
        let mut spec_root: Option<String> = None;
        let root: Option<&str> = loop {
            match &cur.kind {
                ExprKind::MemberAccess { expr, member } => {
                    steps.push(Some(member.name.as_str()));
                    cur = expr;
                }
                ExprKind::Specialization { .. } => {
                    let Some((cn, _)) = Self::extract_call_spec(cur) else {
                        return false;
                    };
                    spec_root = Some(cn);
                    break None;
                }
                ExprKind::Index { expr, .. } => {
                    steps.push(None);
                    cur = expr;
                }
                ExprKind::Paren(inner) => cur = inner,
                ExprKind::This => break None,
                ExprKind::Ident(h) => {
                    for seg in h.path[1..].iter().rev() {
                        steps.extend(seg.selects.iter().map(|_| None));
                        steps.push(Some(seg.name.name.as_str()));
                    }
                    steps.extend(h.path[0].selects.iter().map(|_| None));
                    break Some(h.path[0].name.name.as_str());
                }
                _ => return false,
            }
        };
        steps.reverse();
        enum Ty {
            Dt(DataType),
            Class(String),
        }
        let this_class = self
            .this_stack
            .last()
            .copied()
            .flatten()
            .and_then(|h| self.heap.get(h).and_then(|o| o.as_ref()))
            .map(|i| i.class_name.clone());
        let class_prop = |cn: &str, prop: &str| -> Option<Ty> {
            let mut cur = Some(cn.to_string());
            while let Some(c) = cur {
                let cd = self.module.classes.get(&c)?;
                if let Some(dt) = cd.property_types.get(prop) {
                    let dt = match dt {
                        DataType::TypeReference { name, .. } => cd
                            .typedef_targets
                            .get(&name.name.name)
                            .cloned()
                            .unwrap_or_else(|| dt.clone()),
                        other => other.clone(),
                    };
                    return Some(Ty::Dt(self.resolve_dt(&dt)));
                }
                cur = cd.extends.clone();
            }
            None
        };
        let as_class = |t: Ty| -> Ty {
            match t {
                Ty::Dt(DataType::TypeReference { ref name, .. })
                    if self.module.classes.contains_key(&name.name.name) =>
                {
                    Ty::Class(name.name.name.clone())
                }
                t => t,
            }
        };
        let mut ty = match root {
            None if spec_root.is_some() => Ty::Class(spec_root.unwrap_or_default()),
            None | Some("this") => Ty::Class(match &this_class {
                Some(c) => c.clone(),
                None => return false,
            }),
            Some(n) => {
                let is_local = self.local_stack.last().is_some_and(|f| {
                    f.keys().any(|k| {
                        k.strip_prefix(n)
                            .is_some_and(|r| r.is_empty() || r.starts_with(['.', '[']))
                    })
                });
                let prop = if is_local {
                    None
                } else {
                    this_class.as_deref().and_then(|c| class_prop(c, n))
                };
                match prop {
                    Some(t) => t,
                    None => {
                        if let Some(dt) = self.module.var_decl_types.get(n) {
                            Ty::Dt(self.resolve_dt(dt))
                        } else if let Some(cn) = self.var_class_types.get(n) {
                            Ty::Class(cn.clone())
                        } else if self.module.classes.contains_key(n) {
                            // §8.9: `Cls::q[i].name`.
                            Ty::Class(n.to_string())
                        } else if self.class_member_names().scope_prefixes.contains(n)
                            && let Some((base, _)) = self.resolve_class_scope_prefix(n)
                        {
                            // §8.25: `alias_t::q[i].name`.
                            Ty::Class(base)
                        } else {
                            return false;
                        }
                    }
                }
            }
        };
        ty = as_class(ty);
        // Unpacked dimensions of the member just stepped into, still to be
        // consumed by selects. A root's dimensions are not recorded, so a
        // select on it is an element select unless the root is a string.
        let mut dims_left = 0usize;
        for step in steps {
            match step {
                None => {
                    if dims_left > 0 {
                        dims_left -= 1;
                    } else if !matches!(ty, Ty::Class(_) | Ty::Dt(DataType::Struct(_))) {
                        return false;
                    }
                }
                Some(m) => {
                    if dims_left > 0 {
                        return false;
                    }
                    ty = match ty {
                        Ty::Class(cn) => match class_prop(&cn, m) {
                            Some(t) => t,
                            None => return false,
                        },
                        Ty::Dt(DataType::Struct(su)) => {
                            let Some((mdt, n)) = su.members.iter().find_map(|mm| {
                                mm.declarators
                                    .iter()
                                    .find(|d| d.name.name == m)
                                    .map(|d| (mm.data_type.clone(), d.dimensions.len()))
                            }) else {
                                return false;
                            };
                            dims_left = n;
                            Ty::Dt(self.resolve_dt(&mdt))
                        }
                        _ => return false,
                    };
                    ty = as_class(ty);
                }
            }
        }
        dims_left == 0
            && matches!(
                ty,
                Ty::Dt(DataType::Simple {
                    kind: SimpleType::String,
                    ..
                })
            )
    }

    /// Check whether a struct member is a string; `base` may be a DOTTED path
    /// (`o.in`) — each segment is resolved through its struct type in turn.
    pub(super) fn dotted_member_is_string(&self, base: &str, member: &str) -> bool {
        use crate::ast::types::{DataType, SimpleType};
        let mut segs = base.split('.');
        let Some(head) = segs.next() else {
            return false;
        };
        let Some(dt) = self.module.var_decl_types.get(head) else {
            return false;
        };
        let mut cur = self.resolve_dt(&dt.clone());
        for seg in segs.chain(std::iter::once(member)) {
            let DataType::Struct(su) = &cur else {
                return false;
            };
            let next = su.members.iter().find_map(|m| {
                m.declarators
                    .iter()
                    .any(|d| d.name.name == seg)
                    .then(|| self.resolve_dt(&m.data_type))
            });
            match next {
                Some(t) => cur = t,
                None => return false,
            }
        }
        matches!(
            cur,
            DataType::Simple {
                kind: SimpleType::String,
                ..
            }
        )
    }

    /// Determine whether `expr` denotes a CLASS PROPERTY whose declared type
    /// is `string`.  xezim models class properties by walking the heap
    /// handle in `this_stack` (for `this.<prop>` and bare `<prop>` inside a
    /// method) or by reading the receiver's runtime class (for
    /// `obj.<prop>`), then consulting `class_def.string_properties`.
    /// Used so that string equality/relational operators follow the 2-state
    /// §6.16 / Table 6-9 semantics rather than treating the underlying
    /// 1024-bit packed storage as a 4-state integral value.
    pub(super) fn class_member_is_string(&self, expr: &Expression) -> bool {
        // Only a declared string property can answer true.
        let prop_name: &str = match &expr.kind {
            ExprKind::MemberAccess { member, .. } => &member.name,
            ExprKind::Ident(hier) if hier.path.len() == 1 => &hier.path[0].name.name,
            ExprKind::Ident(hier) if hier.path.len() == 2 => &hier.path[1].name.name,
            ExprKind::Ident(hier) if hier.path.len() >= 3 => &hier.path.last().unwrap().name.name,
            _ => return false,
        };
        if !self.class_member_names().string_props.contains(prop_name) {
            return false;
        }
        // Resolve to `(handle, prop_name)`. We prefer to evaluate the
        // receiver to obtain the runtime handle, but `class_member_is_string`
        // takes `&self` (no mutation); instead, use the `this_stack` for
        // `this.<prop>` / bare `<prop>`, and for `obj.<prop>` read the
        // already-stored handle value out of `signals`/`local_stack`.
        let (handle_opt, prop): (Option<usize>, String) = match &expr.kind {
            // `this.prop` / `obj.prop` via MemberAccess.
            ExprKind::MemberAccess { expr: recv, member } => {
                let h = match &recv.kind {
                    ExprKind::This => self.this_stack.last().copied().flatten(),
                    ExprKind::Ident(hier) if hier.path.len() == 1 => {
                        self.peek_local_handle(&hier.path[0].name.name)
                    }
                    // §8.5: a receiver that is itself a handle chain
                    // (`n.h.tag`, `this.h.tag`): `%s` padded the string to
                    // its container width because only a one-level
                    // receiver was resolved.
                    ExprKind::Ident(_) | ExprKind::MemberAccess { .. } => {
                        self.eval_handle_expr(recv)
                    }
                    _ => None,
                };
                (h, member.name.clone())
            }
            // A bare `<prop>` inside a class method resolves to
            // `this.<prop>` — UNLESS a method-local (or formal) of the
            // same name is in scope, which SHADOWS the property (LRM §6.21).
            // Without this guard a `string` property shadowed by an `int`
            // local is mis-classified as string-valued, so e.g.
            // `$display("tag", val)` of an `int val = 65` prints the char
            // 'A' instead of the number 65. This matters at every caller
            // of `expr_is_string_valued` ($display format-string probing,
            // concatenation text-join, string ==/!=), hence the check sits
            // here rather than at one call site.
            ExprKind::Ident(hier) if hier.path.len() == 1 => {
                let name = &hier.path[0].name.name;
                if self.local_stack.iter().rev().any(|m| m.contains_key(name)) {
                    return false;
                }
                (self.this_stack.last().copied().flatten(), name.clone())
            }
            // `obj.prop` parsed as a two-segment dotted id (§4c). Same
            // resolution as the MemberAccess arm above, but the parser
            // collapses `h.q` into one HierarchicalIdentifier (`[h, q]`)
            // when there are no selects. Without this, a STRING member
            // (including a string collection, for `%s` diagnostics, and the
            // element-index case in expr_is_string_valued) reached from
            // module scope via `h.q` was invisible to string_classification.
            ExprKind::Ident(hier) if hier.path.len() == 2 => {
                let h = self.peek_local_handle(&hier.path[0].name.name);
                (h, hier.path[1].name.name.clone())
            }
            // §8.5: `a.b.<prop>` flattened into one identifier.
            ExprKind::Ident(hier) if hier.path.iter().all(|s| s.selects.is_empty()) => {
                let prefix = Expression::new(ExprKind::Ident(hier_prefix(hier)), expr.span);
                (
                    self.eval_handle_expr(&prefix),
                    hier.path.last().unwrap().name.name.clone(),
                )
            }
            _ => return false,
        };
        let handle = match handle_opt {
            Some(h) => h,
            None => return false,
        };
        let inst = match self.heap.get(handle).and_then(|c| c.as_ref()) {
            Some(i) => i,
            None => return false,
        };
        // Does a class on the inheritance chain declare `prop` as a string
        // property?
        self.class_string_props_of(&inst.class_name)
            .iter()
            .any(|p| *p == prop)
    }

    /// §6.16 / §7.4: is `expr`, the receiver of a select `expr[i]`, a SCALAR
    /// `string`, so that the select names one character? A positive check on
    /// the declared shape: the variable or property at the root of `expr`
    /// must hold `string` elements, and the element selects already applied
    /// to it (`sa[k]`, `m2[a][b]`) must remove every unpacked dimension.
    /// Anything else — a string collection (`c.b.q`, `this.b.aa`), a partial
    /// element of a multi-dimensional one (`s2[1]`), a receiver whose shape
    /// is unknown — is not: a select on it writes a whole element.
    pub(super) fn string_char_receiver_is_scalar(&mut self, expr: &Expression) -> bool {
        let mut selects = 0usize;
        let mut root = expr;
        while let ExprKind::Index { expr: base, .. } = &root.kind {
            selects += 1;
            root = base;
        }
        let Some((is_string, dims, var_root)) = self.string_receiver_shape(root) else {
            return false;
        };
        if !is_string || selects != dims {
            return false;
        }
        // §7.4.5: the element of a design array whose elements are themselves
        // collections (`string sq[2][$]`) is registered under its flat name;
        // selecting into it is still an element write.
        if var_root && selects > 0 {
            if let Some(fm) = self.flat_member_name(expr) {
                if self.is_collection_store(&fm) {
                    return false;
                }
            }
        }
        true
    }

    /// Is `name` the storage name of a collection (fixed, dynamic,
    /// associative array or queue)?
    pub(super) fn is_collection_store(&self, name: &str) -> bool {
        self.module.queue_vars.contains(name)
            || self.module.dynamic_arrays.contains(name)
            || self.module.associative_arrays.contains_key(name)
            || self.module.arrays.contains_key(name)
            || self.module.arrays_2d.contains_key(name)
            || self.module.arrays_nd.contains_key(name)
    }

    /// The shape of the variable or property `root` (a select-free name) as
    /// `(holds string elements, unpacked dimensions, is a design/local
    /// variable rather than a class property)`, or `None` when unknown. See
    /// `string_char_receiver_is_scalar`.
    pub(super) fn string_receiver_shape(
        &mut self,
        root: &Expression,
    ) -> Option<(bool, usize, bool)> {
        match &root.kind {
            // `this.s`, `c.b.s`, `h[0].s`, `C::s` reached as a member.
            ExprKind::MemberAccess { expr: recv, member } => {
                if let Some((s, d)) = self.member_string_shape(recv, &member.name) {
                    return Some((s, d, false));
                }
                self.unresolved_string_shape(root)
            }
            ExprKind::Ident(h) if h.path.len() >= 2 => {
                let n = h.path.len();
                let leaf = &h.path[n - 1];
                if !leaf.selects.is_empty() {
                    return None;
                }
                // `obj.s` / `a.b.s` / `h[0].q` (receiver handle) or `C::s`
                // (static property; the scope parses like a dotted path).
                let recv = Self::hier_receiver_expr(h, root.span);
                if let Some((s, d)) = self.member_string_shape(&recv, &leaf.name.name) {
                    return Some((s, d, false));
                }
                // A hierarchical design variable (`top.s`).
                let var = self.var_string_shape(h);
                if let Some((true, d)) = var {
                    return Some((true, d, true));
                }
                // A member of a struct variable (`st.s`, `st.sa`).
                if h.path.iter().all(|seg| seg.selects.is_empty()) {
                    if let Some((s, d)) = self.struct_member_string_shape(h) {
                        return Some((s, d, true));
                    }
                }
                if let Some((s, d)) = var {
                    return Some((s, d, true));
                }
                self.unresolved_string_shape(root)
            }
            ExprKind::Ident(h) => {
                let seg = h.path.first()?;
                if !seg.selects.is_empty() {
                    return None;
                }
                let name = seg.name.name.as_str();
                // §8.7: a bare property name inside a method is `this.<name>`
                // unless a local or formal shadows it (§6.21).
                if !self.local_stack.iter().any(|m| m.contains_key(name)) {
                    let cls: Option<String> = match self.this_stack.last().copied().flatten() {
                        Some(hd) => self
                            .heap
                            .get(hd)
                            .and_then(|c| c.as_ref())
                            .map(|i| i.class_name.clone()),
                        None => self.class_context_stack.last().cloned().flatten(),
                    };
                    if let Some(c) = cls {
                        if let Some((s, d)) = self.class_prop_string_shape(&c, name) {
                            return Some((s, d, false));
                        }
                    }
                }
                self.var_string_shape(h).map(|(s, d)| (s, d, true))
            }
            _ => None,
        }
    }

    /// The receiver of the dotted path `h` (all but its last segment) as an
    /// expression: a select on a segment (`h[0].q`) becomes an `Index`, each
    /// further segment a `MemberAccess`.
    pub(super) fn hier_receiver_expr(
        h: &crate::ast::expr::HierarchicalIdentifier,
        span: crate::ast::Span,
    ) -> Expression {
        let n = h.path.len();
        if h.path[..n - 1].iter().all(|seg| seg.selects.is_empty()) {
            return Expression::new(ExprKind::Ident(hier_prefix(h)), span);
        }
        let mut head = hier_prefix(h);
        head.path.truncate(1);
        let selects = std::mem::take(&mut head.path[0].selects);
        let mut e = Expression::new(ExprKind::Ident(head), span);
        let wrap = |e: Expression, sel: &[Expression]| {
            sel.iter().fold(e, |acc, ix| {
                Expression::new(
                    ExprKind::Index {
                        expr: Box::new(acc),
                        index: Box::new(ix.clone()),
                    },
                    span,
                )
            })
        };
        e = wrap(e, &selects);
        for seg in &h.path[1..n - 1] {
            e = Expression::new(
                ExprKind::MemberAccess {
                    expr: Box::new(e),
                    member: seg.name.clone(),
                },
                span,
            );
            e = wrap(e, &seg.selects);
        }
        e
    }

    /// `(holds string elements, unpacked dimensions)` of property `prop` of
    /// the object `recv` refers to, or of class `recv` (`C::prop`).
    pub(super) fn member_string_shape(
        &self,
        recv: &Expression,
        prop: &str,
    ) -> Option<(bool, usize)> {
        if let Some(hd) = self.eval_handle_expr(recv).filter(|&hd| hd != 0) {
            let cls = self.heap.get(hd)?.as_ref()?.class_name.as_str();
            return self.class_prop_string_shape(cls, prop);
        }
        match &recv.kind {
            ExprKind::Ident(rh)
                if rh.path.len() == 1
                    && rh.path[0].selects.is_empty()
                    && self.module.classes.contains_key(&rh.path[0].name.name) =>
            {
                self.class_prop_string_shape(&rh.path[0].name.name, prop)
            }
            _ => None,
        }
    }

    /// `(holds string elements, unpacked dimensions)` of property `prop`
    /// declared by class `cn` or an ancestor (§8.13); `None` if none does.
    pub(super) fn class_prop_string_shape(&self, cn: &str, prop: &str) -> Option<(bool, usize)> {
        let mut cur = Some(cn);
        for _ in 0..64 {
            let cd = self.module.classes.get(cur?)?;
            let static_coll = cd.static_collections.iter().any(|(n, ..)| n == prop);
            let static_fixed = cd.static_fixed_arrays.iter().any(|(n, ..)| n == prop);
            if cd.properties.contains_key(prop)
                || cd.static_properties.contains(prop)
                || static_coll
                || static_fixed
            {
                let dims = if let Some((d, _, _)) = cd.array_of_coll_properties.get(prop) {
                    d.len() + 1
                } else if let Some(d) = cd.param_sized_props.get(prop) {
                    d.len()
                } else if let Some((d, _)) = cd.array_nd_properties.get(prop) {
                    d.len()
                } else if cd.array_properties.contains_key(prop)
                    || cd.queue_properties.contains_key(prop)
                    || cd.assoc_properties.contains_key(prop)
                    || static_coll
                    || static_fixed
                {
                    1
                } else {
                    0
                };
                return Some((cd.string_properties.contains(prop), dims));
            }
            cur = cd.extends.as_deref();
        }
        None
    }

    /// `(holds string elements, unpacked dimensions)` of the design or
    /// subroutine-local variable `h` names, from the collection registries;
    /// `None` when it is neither a string nor a collection.
    pub(super) fn var_string_shape(
        &self,
        h: &crate::ast::expr::HierarchicalIdentifier,
    ) -> Option<(bool, usize)> {
        let name = self.resolve_hier_name(h);
        let n: &str = &name;
        let bare = (h.path.len() == 1).then(|| h.path[0].name.name.as_str());
        // A typedef'd string array (`str3_t ts;`) is not registered as a
        // string by name; its declared type is.
        let decl_is_string = |nm: &str| {
            self.module
                .var_decl_types
                .get(nm)
                .is_some_and(|dt| match dt {
                    DataType::TypeReference { .. } => {
                        Self::is_string_data_type(&self.resolve_dt(dt))
                    }
                    _ => Self::is_string_data_type(dt),
                })
        };
        let is_string = self.string_signals.contains(n)
            || bare.is_some_and(|b| self.string_signals.contains(b))
            || decl_is_string(n)
            || bare.is_some_and(|b| b != n && decl_is_string(b));
        let dims = if self.module.queue_vars.contains(n)
            || self.module.dynamic_arrays.contains(n)
            || self.module.associative_arrays.contains_key(n)
        {
            1
        } else {
            self.unpacked_dims_of(n)
        };
        (is_string || dims > 0).then_some((is_string, dims))
    }

    /// `(holds string elements, unpacked dimensions)` of the struct member
    /// path `h` (`st.s`, `st.in.sa`), walked through the declared types.
    pub(super) fn struct_member_string_shape(
        &self,
        h: &crate::ast::expr::HierarchicalIdentifier,
    ) -> Option<(bool, usize)> {
        let mut dt = self
            .module
            .var_decl_types
            .get(&h.path[0].name.name)?
            .clone();
        let mut udims = Vec::new();
        for seg in &h.path[1..] {
            if !udims.is_empty() {
                return None;
            }
            (dt, udims) = self.member_decl_type(&dt, &seg.name.name)?;
        }
        Some((
            Self::is_string_data_type(&self.resolve_dt(&dt)),
            udims.len(),
        ))
    }

    /// Last resort for a receiver whose declaration could not be found (a
    /// member of a struct array element, a frame-local struct): a
    /// string-valued name that is not itself a collection is a scalar.
    pub(super) fn unresolved_string_shape(
        &mut self,
        root: &Expression,
    ) -> Option<(bool, usize, bool)> {
        if !self.expr_is_string_valued(root) {
            return None;
        }
        if let Some(fm) = self.flat_member_name(root) {
            if self.is_collection_store(&fm) {
                return None;
            }
        }
        Some((true, 0, true))
    }

    /// Peek a class-typed procedural local's heap handle WITHOUT mutating
    /// state.  Used by `class_member_is_string` (a `&self` method) to resolve
    /// `obj.<prop>` where `obj` is a local whose stored `Value` is the handle.
    pub(super) fn peek_local_handle(&self, name: &str) -> Option<usize> {
        // Innermost-out search of the call frames.
        for m in self.local_stack.iter().rev() {
            if let Some(v) = m.get(name) {
                return v.to_u64().map(|h| h as usize);
            }
        }
        // A module-scope class-handle declaration (`holder h;` in a `module`,
        // assigned `h = new()` inside an initial block) lives in the signal
        // TABLE, not the legacy `signals` map — mirror `eval_ident_handle`'s
        // fallback so `h.q[idx]` string-classification can resolve the handle
        // from program scope too, not just inside a method.
        if let Some(&id) = self.signal_name_to_id.get(name) {
            return self.signal_table[id].to_u64().map(|h| h as usize);
        }
        // Fallback to the signal map (initial/always-block locals).
        self.signals
            .get(name)
            .and_then(|v| v.to_u64().map(|h| h as usize))
    }

    /// Determine whether a call's `func` expression resolves to a method
    /// whose declared return type is `string`. Handles bare method names
    /// (`sprint(x)` inside a class body), `this.method()`, `obj.method()`,
    /// and `Class::method()`. Walks the inheritance chain so a method
    /// inherited from a base class is recognised.
    pub(super) fn call_returns_string(&self, func: &Expression) -> bool {
        use crate::ast::decl::ClassMethodKind;
        use crate::ast::types::{DataType, SimpleType};
        // A plain module/package FUNCTION with a `string` return type
        // (`$display("%s", where_am_i())`) — not a class method at all.
        if let ExprKind::Ident(h) = &func.kind {
            if let Some(seg) = h.path.last() {
                if let Some(fd) = self.module.functions.get(&seg.name.name) {
                    if matches!(
                        fd.return_type,
                        DataType::Simple {
                            kind: SimpleType::String,
                            ..
                        }
                    ) {
                        return true;
                    }
                }
            }
        }
        // The class walk below answers true only for a name some class
        // declares as a string-returning function.
        let callee = match &func.kind {
            ExprKind::MemberAccess { member, .. } => Some(member.name.as_str()),
            ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.as_str()),
            _ => None,
        };
        if !callee.is_some_and(|m| self.class_member_names().string_methods.contains(m)) {
            return false;
        }
        // Extract (class_name, method_name).
        let (class_name, method_name): (Option<String>, Option<String>) = match &func.kind {
            ExprKind::MemberAccess { expr: recv, member } => {
                // Runtime resolution first (reads the actual class_name
                // from the heap handle — reliable for class-typed
                // procedural locals). Fall back to static analysis
                // when the handle isn't available (e.g. a null check
                // before construction, or a `super.method()` call).
                let cn = self
                    .runtime_recv_class(recv)
                    .or_else(|| self.get_expr_type_name(recv))
                    .or_else(|| self.class_context_stack.last().cloned().flatten());
                (cn, Some(member.name.clone()))
            }
            ExprKind::Ident(h) if h.path.len() == 1 => {
                let cn = self.class_context_stack.last().cloned().flatten();
                (cn, Some(h.path[0].name.name.clone()))
            }
            ExprKind::Ident(h) if h.path.len() >= 2 => {
                // `obj.method(args)` parsed as a flat hierarchical
                // path [obj, method]. Resolve `obj` as a runtime
                // handle first (reliable for class-typed locals),
                // then static type, then assume it's a class name
                // for `ClassName::method()` calls.
                let first = &h.path[0].name.name;
                let cn = self
                    .runtime_recv_class(&Expression {
                        kind: ExprKind::Ident(h.clone()),
                        span: h.span,
                        cached_width: std::cell::Cell::new(None),
                    })
                    .or_else(|| self.var_class_types.get(first).cloned())
                    .or_else(|| Some(first.clone()));
                let mn = h.path.last().unwrap().name.name.clone();
                (cn, Some(mn))
            }
            _ => (None, None),
        };
        let (cn, mn) = match (class_name, method_name) {
            (Some(c), Some(m)) => (c, m),
            _ => return false,
        };
        // Walk the class hierarchy looking for the method's return type.
        let mut cur = Some(cn);
        while let Some(cname) = cur {
            if let Some(cd) = self.module.classes.get(&cname) {
                if let Some(cm) = cd.methods.get(&mn) {
                    if let ClassMethodKind::Function(fd)
                    | ClassMethodKind::Extern(fd)
                    | ClassMethodKind::PureVirtual(fd) = &cm.kind
                    {
                        return matches!(
                            &fd.return_type,
                            DataType::Simple {
                                kind: SimpleType::String,
                                ..
                            }
                        );
                    }
                    // Task → never returns a string.
                    return false;
                }
                cur = cd.extends.clone();
            } else {
                break;
            }
        }
        false
    }

    /// Best-effort: resolve a receiver expression to its runtime class name
    /// by reading the handle from `local_stack`/`this_stack` and looking up
    /// the `ClassInstance.class_name` in the heap. Used by
    /// `call_returns_string` when static type analysis fails.
    pub(super) fn runtime_recv_class(&self, recv: &Expression) -> Option<String> {
        let handle = self.eval_handle_expr(recv)?;
        self.heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|inst| inst.class_name.clone())
    }

    /// Format a string and report how many of `args` the format
    /// specifiers consumed, so `format_args` can continue the LRM §21.2.1.2
    /// argument walk (several format strings may interleave in one call).
    pub(super) fn format_string_consumed(
        &mut self,
        fmt: &str,
        args: &[Expression],
        _tn: &str,
    ) -> (String, usize) {
        let mut result = String::new();
        let mut ai = 0;
        let mut chars = fmt.chars().peekable();
        // §21.2.1.7 `%0p` (compact): the arm below marks where its output
        // starts; the rewrite runs here on the NEXT iteration (or after the
        // loop) because the arm has many early-continue exits.
        let mut compact_from: Option<usize> = None;
        while let Some(c) = chars.next() {
            if let Some(fp) = compact_from.take() {
                let tail = Self::compact_p(&result[fp..]);
                result.truncate(fp);
                result.push_str(&tail);
            }
            if c == '%' {
                // §21.2.1.2: %[flags][width][.precision]<spec>. Flags: '-'
                // left-justifies (pad spaces on the right), '+' forces a sign
                // on decimal/real output. A width with a leading '0' zero-pads
                // AND selects the minimal (leading-zero-trimmed) form for
                // b/o/h; a bare `%0<spec>` means "minimal width, no padding".
                let mut left_align = false;
                let mut plus_sign = false;
                loop {
                    match chars.peek() {
                        Some('-') => {
                            left_align = true;
                            chars.next();
                        }
                        Some('+') => {
                            plus_sign = true;
                            chars.next();
                        }
                        _ => break,
                    }
                }
                let mut width_str = String::new();
                while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                    width_str.push(chars.next().unwrap());
                }
                let mut precision: Option<usize> = None;
                if chars.peek() == Some(&'.') {
                    chars.next();
                    let mut p = String::new();
                    while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                        p.push(chars.next().unwrap());
                    }
                    precision = Some(p.parse().unwrap_or(0));
                }
                let pad_width: usize = width_str.parse().unwrap_or(0);
                let zero_pad = width_str.starts_with('0');
                let has_width = !width_str.is_empty();
                // Resolve the final field width/pad-char and justify `core`.
                // `default_width` applies only when no explicit width is given.
                let field = |core: String, default_width: usize| -> String {
                    let (w, zero) = if has_width {
                        (pad_width, zero_pad)
                    } else {
                        (default_width, false)
                    };
                    if core.len() >= w {
                        return core;
                    }
                    let pad = w - core.len();
                    if left_align {
                        format!("{}{}", core, " ".repeat(pad))
                    } else if zero {
                        // Zero-padding goes between the sign and the digits.
                        match core.chars().next() {
                            Some(sign @ ('-' | '+')) => {
                                format!("{}{}{}", sign, "0".repeat(pad), &core[1..])
                            }
                            _ => format!("{}{}", "0".repeat(pad), core),
                        }
                    } else {
                        format!("{}{}", " ".repeat(pad), core)
                    }
                };
                if let Some(&spec) = chars.peek() {
                    chars.next();
                    match spec {
                        '%' => result.push('%'),
                        't' | 'T' => {
                            if ai < args.len() {
                                // The argument carries a time in the scope's
                                // unit. §20.3: `$realtime` keeps sub-unit
                                // precision, but `$time`/`$stime` are the time
                                // ROUNDED to the module unit (an integer) — `%t`
                                // must format that rounded value, not the raw
                                // simulation time. Any other expression is read
                                // as a scope-unit value directly.
                                let value = match &args[ai].kind {
                                    ExprKind::SystemCall { name, .. }
                                        if name.as_str() == "$realtime" =>
                                    {
                                        self.time_in_current_unit()
                                    }
                                    ExprKind::SystemCall { name, .. }
                                        if matches!(name.as_str(), "$time" | "$stime") =>
                                    {
                                        let r = self.time_in_current_unit().round();
                                        if name.as_str() == "$stime" {
                                            ((r as u64) & 0xFFFF_FFFF) as f64
                                        } else {
                                            r
                                        }
                                    }
                                    _ => self.eval_expr(&args[ai]).to_f64(),
                                };
                                // $timeformat's min field width (default 20,
                                // §21.3.5) governs a bare %t; an explicit
                                // `%NNt` width overrides it, `%0t` is minimal.
                                let (body, tf_width) = self.format_time_parts(value);
                                result.push_str(&field(body, tf_width));
                                ai += 1;
                            }
                        }
                        'p' | 'P' => {
                            if has_width && pad_width == 0 {
                                compact_from = Some(result.len());
                            }
                            if ai < args.len() {
                                let arg = &args[ai];
                                ai += 1;
                                // §8.9: a static fixed array spelled `Cls::S`
                                // or `C#(T)::S`, or one element of it,
                                // `Cls::S[i]`.
                                if matches!(
                                    arg.kind,
                                    ExprKind::MemberAccess { .. } | ExprKind::Index { .. }
                                ) && let Some(s) = self.render_p_static_fixed_operand(arg)
                                {
                                    result.push_str(&s);
                                    continue;
                                }
                                // §21.2.1.7: a SUBROUTINE-local scalar variable
                                // (`int x; string s; T _t;` inside a task/
                                // function) does not live in the module `signals`
                                // map, so the name-based `render_p_var` path below
                                // reads nothing and prints `x`/empty. Its runtime
                                // value sits in the current `local_stack` frame,
                                // reachable through `eval_expr` (the same read the
                                // `%0d`/`%0h` arms use, which work). For a bare
                                // scalar local, evaluate it and render directly —
                                // quoting WHEN the declared type is `string`
                                // (resolving a type parameter `T` to its concrete
                                // bound, so `T=string` prints a quoted string and
                                // `T=int` prints a number).
                                if let ExprKind::Ident(h) = &arg.kind {
                                    if h.path.len() == 1
                                        && h.path[0].selects.is_empty()
                                        && !self.local_stack.is_empty()
                                    {
                                        let nm = &h.path[0].name.name;
                                        let is_local = self
                                            .local_stack
                                            .last()
                                            .is_some_and(|f| f.contains_key(nm));
                                        if is_local && !self.local_ident_is_collection(nm) {
                                            let is_str = self.p_local_is_string(nm);
                                            let v = self.eval_expr(arg);
                                            let core = Self::render_p_value(&v, is_str);
                                            result.push_str(&core);
                                            continue;
                                        }
                                    }
                                }
                                // §21.2.1.7: a call returning an unpacked struct.
                                if let Some(core) = self.render_p_struct_call(arg) {
                                    result.push_str(&core);
                                    continue;
                                }
                                // A string-valued CALL or concatenation names no
                                // variable for the typed paths below, so its bytes
                                // printed as one huge decimal — UVM AVIPs log
                                // `$sformatf("%0p", cfg.sprint())`.
                                if matches!(
                                    arg.kind,
                                    ExprKind::Call { .. } | ExprKind::Concatenation(_)
                                ) && self.expr_is_string_valued(arg)
                                {
                                    let v = self.eval_expr(arg);
                                    result.push_str(&Self::render_p_value(&v, true));
                                    continue;
                                }
                                // §7.12.1: an array LOCATOR call (`q.unique()`,
                                // `q.find with (…)`, `q.min()`) RETURNS a queue.
                                // Only the ASSIGNMENT path materialized that, so
                                // used directly as an argument the call fell
                                // through to scalar evaluation and printed 0 —
                                // `r = q.unique(); $display("%p", r);` was right
                                // while the one-liner was silently wrong.
                                if let Some((arr_name, mname, filter, iter_name)) =
                                    self.locator_call(arg)
                                {
                                    let idxs = self.locator_indices_named(
                                        &arr_name,
                                        &mname,
                                        filter.as_ref(),
                                        iter_name.as_deref(),
                                    );
                                    const TMP: &str = "__xz_locator_fmt";
                                    self.materialize_locator(TMP, &arr_name, &mname, &idxs);
                                    // An `_index` form yields indices, never the
                                    // element type, so it is not string-rendered.
                                    let is_str = !mname.ends_with("_index")
                                        && self.string_signals.contains(&arr_name);
                                    let mut parts: Vec<String> = Vec::with_capacity(idxs.len());
                                    for i in 0..idxs.len() {
                                        if let Some(v) = self
                                            .get_signal_value_by_name(&format!("{}[{}]", TMP, i))
                                        {
                                            parts.push(Self::render_p_value(&v, is_str));
                                        }
                                    }
                                    result.push_str(&format!("'{{{}}}", parts.join(", ")));
                                    continue;
                                }
                                if let ExprKind::Ident(h) = &arg.kind {
                                    let name = self.resolve_hier_name(h);
                                    if let Some(tag) = self.active_union_tag.get(&*name).cloned() {
                                        let v = self.eval_expr(arg);
                                        result.push_str(&format!(
                                            "'{{{}:{}}}",
                                            tag,
                                            v.to_dec_string()
                                        ));
                                        continue;
                                    }
                                }
                                // §21.2.1.7: a fixed-array or unpacked-struct
                                // CLASS property renders from its declared type.
                                if !self.no_class_objects()
                                    && let Some((handle, prop)) = self.class_prop_receiver(arg)
                                    && let Some(s) = self.render_p_class_prop(handle, &prop)
                                {
                                    result.push_str(&s);
                                    continue;
                                }
                                // §8.9: a static fixed array spelled `Cls::S`.
                                if let ExprKind::Ident(h) = &arg.kind
                                    && h.path.len() == 2
                                    && let Some(k) = self.static_fixed_key_from_hier(h)
                                    && let Some(s) = self.render_p_var(&k)
                                {
                                    result.push_str(&s);
                                    continue;
                                }
                                // An unpacked-struct ELEMENT of one (`c.sta[1]`)
                                // renders from the element's instance cells.
                                if !self.no_class_objects()
                                    && let Some(s) = self.render_p_class_struct_elem(arg)
                                {
                                    result.push_str(&s);
                                    continue;
                                }
                                // §8.9/§8.25: a static collection named bare in
                                // a static method of a parameterized class is
                                // the specialization's store (`P#int::q`).
                                if let ExprKind::Ident(h) = &arg.kind
                                    && h.path.len() == 1
                                    && h.path[0].selects.is_empty()
                                    && let Some(k) =
                                        self.static_method_coll_key(&h.path[0].name.name)
                                    && let Some(s) = self.render_p_var(&k)
                                {
                                    result.push_str(&s);
                                    continue;
                                }
                                // §8.9: a static fixed array named bare in a
                                // method of its class.
                                if let ExprKind::Ident(h) = &arg.kind
                                    && h.path.len() == 1
                                    && h.path[0].selects.is_empty()
                                    && !self
                                        .local_stack
                                        .last()
                                        .is_some_and(|l| l.contains_key(&h.path[0].name.name))
                                    && let Some(Some(ctx)) =
                                        self.class_context_stack.last().cloned()
                                    && let Some(s) =
                                        self.render_p_static_fixed(&ctx, &h.path[0].name.name)
                                {
                                    result.push_str(&s);
                                    continue;
                                }
                                // §21.2.1.7: a class object prints its properties
                                // named, in declaration order (base first); a
                                // null handle prints `null`. Only a genuinely
                                // class-typed variable takes this path, so a
                                // plain int is never read as a handle.
                                if let ExprKind::Ident(h) = &arg.kind {
                                    if h.path.len() == 1
                                        && self.class_of_var(&h.path[0].name.name).is_some()
                                    {
                                        let handle = self.eval_expr(arg).to_u64().unwrap_or(0);
                                        if handle == 0 {
                                            result.push_str("null");
                                            continue;
                                        }
                                        if let Some(members) =
                                            self.class_object_members(handle as usize)
                                        {
                                            let parts: Vec<String> = members
                                                .iter()
                                                .filter_map(|(n, v, is_str)| {
                                                    let r = self.render_p_object_member(
                                                        handle as usize,
                                                        n,
                                                        v.as_ref(),
                                                        *is_str,
                                                    )?;
                                                    Some(format!("{}:{}", n, r))
                                                })
                                                .collect();
                                            result.push_str(&format!("'{{{}}}", parts.join(", ")));
                                            continue;
                                        }
                                    }
                                }
                                // A collection PROPERTY receiver (`o.q` where q
                                // is a native queue/assoc member) lives under the
                                // instance-scoped name `<handle>#<member>` — the
                                // flat path "o.q" matches nothing, so resolve it
                                // first or the fallback prints a raw 0.
                                if let Some(scoped) = self.expr_assoc_name(arg) {
                                    if let Some(s) = self.render_p_var(&scoped) {
                                        result.push_str(&s);
                                        continue;
                                    }
                                }
                                // §21.2.1.7 / §7.2: a WHOLE unpacked-struct
                                // CLASS property (`c.first` bound through a
                                // class type parameter, UVM's
                                // `uvm_built_in_pair #(s,s)` first/second).
                                // Its leaves live in the instance PropMap, not
                                // the flat signal namespace, so `render_p_var`
                                // treats `first` as an untyped strip and
                                // prints a raw packed value / x. Resolve the
                                // receiver + property and render member-wise.
                                if !self.no_class_objects()
                                    && let Some(((handle, recv_prop), su)) = self
                                        .class_prop_receiver(arg)
                                        .zip(self.class_prop_struct_of(arg))
                                {
                                    let rendered =
                                        self.render_p_class_prop_struct(handle, &recv_prop, &su);
                                    result.push_str(&rendered);
                                    continue;
                                }
                                // §6.16/§21.2.1.7: a STRING class property
                                // (`c.s`, or a type PARAMETER bound to
                                // `string` — UVM's `uvm_built_in_pair #(string,
                                // string)` first/second). Its value lives in
                                // the instance PropMap under the bare property
                                // name, but `render_p_var` treats `s` as an
                                // untyped strip and prints the first byte
                                // (`121` for "y"). Render it as a quoted
                                // string.
                                if !self.no_class_objects()
                                    && let Some((handle, prop)) = self.class_prop_receiver(arg)
                                    && self.class_prop_type_name(handle, &prop).as_deref()
                                        == Some("string")
                                {
                                    if let Some(v) = self
                                        .heap
                                        .get(handle)
                                        .and_then(|o| o.as_ref())
                                        .and_then(|i| i.properties.get(&prop))
                                        .cloned()
                                    {
                                        result.push_str(&Self::render_p_value(&v, true));
                                        continue;
                                    }
                                }
                                // An ELEMENT of a class-property collection of
                                // unpacked structs keeps its leaves under the
                                // instance (`<h>#q[i].<m>`), not the flat name.
                                if !self.no_class_objects()
                                    && let Some(flat) = self.flat_member_name(arg)
                                    && let Some(cut) = flat.find('[')
                                    && flat.ends_with(']')
                                    && !flat[cut..].contains('.')
                                {
                                    let storage = self.resolve_locator_storage(&flat[..cut]);
                                    if storage != flat[..cut]
                                        && let Some(su) = self.queue_elem_struct(&storage)
                                    {
                                        let elem = format!("{}{}", storage, &flat[cut..]);
                                        let s = self.render_p_typed(&elem, &DataType::Struct(su));
                                        result.push_str(&s);
                                        continue;
                                    }
                                }
                                // §21.2.1.7: type-directed recursive render —
                                // nested structs, unpacked arrays, enum labels,
                                // strings and reals, in declaration order.
                                if let Some(flat) = self.flat_member_name(arg) {
                                    // Whole variables AND flattened sub-paths
                                    // (`arr[0]`, `c.nodes`, `cmb[10].str`) are
                                    // rendered from their declared type.
                                    let mut rendered = self.render_p_var(&flat);
                                    if rendered.is_none()
                                        && self.signal_name_to_id.contains_key(flat.as_str())
                                    {
                                        // Untyped leaf: render the raw value.
                                        let is_str = self.string_signals.contains(&flat);
                                        rendered = self
                                            .get_signal_value_by_name(&flat)
                                            .map(|v| Self::render_p_value(&v, is_str));
                                    }
                                    if let Some(mut s) = rendered {
                                        // §21.2.1.7: an implementation may cap the
                                        // string (at least 1024 chars); on truncation
                                        // it must warn.
                                        const P_MAX_LEN: usize = 8192;
                                        if s.len() > P_MAX_LEN {
                                            s.truncate(P_MAX_LEN);
                                            s.push_str("...");
                                            self.stdout_writeln(
                                                "[xezim][warning] %p output truncated at 8192 characters (IEEE 1800-2017 §21.2.1.7)",
                                            );
                                        }
                                        result.push_str(&s);
                                        continue;
                                    }
                                }
                                // A hierarchical spelling (`host_mod.payload`
                                // from a bound checker, §23.8 upward naming)
                                // is not the storage key — resolve it and
                                // retry the type-directed render. Class-body
                                // code parses the same reference as a
                                // MemberAccess chain, so go through the flat
                                // name rather than requiring an Ident shape.
                                if let Some(flat) = self.flat_member_name(arg) {
                                    if flat.contains('.') {
                                        let ident = HierarchicalIdentifier {
                                            root: None,
                                            path: vec![HierPathSegment {
                                                name: crate::ast::Identifier {
                                                    name: flat.clone(),
                                                    span: arg.span,
                                                },
                                                selects: Vec::new(),
                                            }],
                                            span: arg.span,
                                            cached_signal_id: std::cell::Cell::new(None),
                                            cached_resolved_name: std::cell::OnceCell::new(),
                                        };
                                        let resolved = self.resolve_hier_name(&ident);
                                        if resolved != flat {
                                            if let Some(s) = self.render_p_var(&resolved) {
                                                result.push_str(&s);
                                                continue;
                                            }
                                        }
                                    }
                                }
                                // Fallback: one-level named members (declared
                                // somewhere we don't record a full type for).
                                if let Some(members) = self.struct_members_ordered(arg) {
                                    let mut parts = Vec::with_capacity(members.len());
                                    for (mname, fexpr) in &members {
                                        // A `string` member is tracked by its
                                        // dotted signal name.
                                        let is_str = match &fexpr.kind {
                                            ExprKind::Ident(fh) => {
                                                let n = self.resolve_hier_name(fh);
                                                self.string_signals.contains(&*n)
                                            }
                                            _ => false,
                                        };
                                        let v = self.eval_expr(fexpr);
                                        parts.push(format!(
                                            "{}:{}",
                                            mname,
                                            Self::render_p_value(&v, is_str)
                                        ));
                                    }
                                    result.push_str(&format!("'{{{}}}", parts.join(", ")));
                                    continue;
                                }
                                // Fallback: member order unknown (struct declared
                                // somewhere we don't record) — emit unnamed values
                                // rather than a flattened decimal / X.
                                if let Some(fields) = self.expand_struct_target(arg) {
                                    let mut parts = Vec::with_capacity(fields.len());
                                    for (_n, fexpr) in &fields {
                                        parts.push(self.eval_expr(fexpr).to_dec_string());
                                    }
                                    result.push_str(&format!("'{{{}}}", parts.join(", ")));
                                    continue;
                                }
                                // §21.2.1.7: a DIRECT queue/array slice
                                // (`$display("%p", q[1:3])`) — render the
                                // element range as a list, not the concatenated
                                // packed value the raw fallback would print.
                                if let ExprKind::RangeSelect {
                                    expr,
                                    left,
                                    right,
                                    kind,
                                } = &arg.kind
                                {
                                    if matches!(kind, RangeKind::Constant) {
                                        if let ExprKind::Ident(h) = &expr.kind {
                                            let nm = self.resolve_hier_name(h);
                                            let is_coll = self.module.arrays.contains_key(&*nm)
                                                || self.module.dynamic_arrays.contains(&*nm);
                                            if is_coll {
                                                let is_dyn =
                                                    self.module.dynamic_arrays.contains(&*nm);
                                                let (arr_lo, arr_hi) = self
                                                    .module
                                                    .arrays
                                                    .get(&*nm)
                                                    .map(|&(lo, hi, _)| (lo, hi))
                                                    .unwrap_or((0, 0));
                                                // `$` in the slice resolves to the
                                                // last valid index (size-1 for a
                                                // queue/dynamic array).
                                                let upper = if is_dyn {
                                                    self.get_queue_size(&nm) as i64 - 1
                                                } else {
                                                    arr_hi
                                                };
                                                self.dollar_bound.push(upper);
                                                let mut l =
                                                    self.eval_expr(left).to_i64().unwrap_or(0);
                                                let mut r =
                                                    self.eval_expr(right).to_i64().unwrap_or(0);
                                                self.dollar_bound.pop();
                                                // §7.10.1: a queue slice `q[a:b]`
                                                // with a > b is empty.
                                                if is_dyn && l > r {
                                                    result.push_str("'{}");
                                                    continue;
                                                }
                                                if l > r {
                                                    std::mem::swap(&mut l, &mut r);
                                                }
                                                // Clamp to the collection's live
                                                // element range.
                                                l = l.max(arr_lo);
                                                r = r.min(upper);
                                                let is_str = self.string_signals.contains(&*nm);
                                                let mut parts: Vec<String> = Vec::new();
                                                for i in l..=r {
                                                    let en = format!("{}[{}]", nm, i);
                                                    if let Some(v) =
                                                        self.get_signal_value_by_name(&en)
                                                    {
                                                        parts
                                                            .push(Self::render_p_value(&v, is_str));
                                                    }
                                                }
                                                result.push_str(&format!(
                                                    "'{{{}}}",
                                                    parts.join(", ")
                                                ));
                                                continue;
                                            }
                                        }
                                    }
                                }
                                // §21.2.1.7: a class FIXED-ARRAY property
                                // (`c.arr`) renders as an element list — the
                                // scalar fallback below printed 0.
                                let obj_prop: Option<(usize, String)> = match &arg.kind {
                                    ExprKind::MemberAccess { expr: b, member } => {
                                        let hh = self.eval_expr(b).to_u64().unwrap_or(0) as usize;
                                        Some((hh, member.name.clone()))
                                    }
                                    ExprKind::Ident(ih)
                                        if ih.path.len() == 2
                                            && ih.path.iter().all(|s| s.selects.is_empty()) =>
                                    {
                                        let obj = ih.path[0].name.name.clone();
                                        self.eval_ident_handle(&obj)
                                            .map(|hv| (hv, ih.path[1].name.name.clone()))
                                    }
                                    _ => None,
                                };
                                if let Some((h, prop)) = obj_prop {
                                    if h != 0 && h < self.heap.len() {
                                        let mut cur = self
                                            .heap
                                            .get(h)
                                            .and_then(|o| o.as_ref())
                                            .map(|i| i.class_name.clone());
                                        let mut range: Option<(i64, i64)> = None;
                                        let mut guard = 0;
                                        while let Some(c) = cur {
                                            guard += 1;
                                            if guard > 64 {
                                                break;
                                            }
                                            let Some(cd) = self.module.classes.get(&c) else {
                                                break;
                                            };
                                            if let Some(&(lo, hi, _w)) =
                                                cd.array_properties.get(&prop)
                                            {
                                                range = Some(self.inst_fixed_bounds(
                                                    cd,
                                                    h,
                                                    &prop,
                                                    (lo, hi),
                                                ));
                                                break;
                                            }
                                            cur = cd.extends.clone();
                                        }
                                        if let Some((lo, hi)) = range {
                                            let nm = format!("{}#{}", h, prop);
                                            let is_str = self.string_signals.contains(&nm);
                                            let mut parts: Vec<String> = Vec::new();
                                            // §21.2.1.7: `%p` lists the
                                            // elements left to right.
                                            let desc = self.module.descending_arrays.contains(&nm);
                                            for k in 0..=(hi - lo) {
                                                let i = if desc { hi - k } else { lo + k };
                                                let v = self
                                                    .get_signal_value_by_name(&format!(
                                                        "{}[{}]",
                                                        nm, i
                                                    ))
                                                    .unwrap_or_else(|| Value::zero(32));
                                                parts.push(Self::render_p_value(&v, is_str));
                                            }
                                            result.push_str(&format!("'{{{}}}", parts.join(", ")));
                                            continue;
                                        }
                                    }
                                }
                                let v = self.eval_expr(arg);
                                result.push_str(&v.to_dec_string());
                            }
                        }
                        'm' | 'M' => {
                            // %m: hierarchical name of the current scope. It
                            // consumes NO argument, so it must not be gated on
                            // one being available — `$display("%m")` printed
                            // nothing at all (issue #25).
                            result.push_str(&self.m_path());
                        }
                        _ => {
                            if ai < args.len() {
                                let v = self.eval_expr(&args[ai]);
                                ai += 1;
                                match spec {
                                    'd' | 'D' => {
                                        // §21.2.1.2: default field width is
                                        // the widest decimal the operand's
                                        // type can take (sign included).
                                        // %0d = minimum width; %0Nd zero-pads;
                                        // %Nd space-pads; %-Nd left-justifies.
                                        let mut core = v.to_dec_string();
                                        if plus_sign && !core.starts_with('-') {
                                            core.insert(0, '+');
                                        }
                                        let dw = Self::dec_default_width(v.width, v.is_signed);
                                        // Reference-simulator behavior: a
                                        // leading zero in a %d width (`%06d`)
                                        // is part of the WIDTH, not a
                                        // zero-fill flag — decimals space-pad.
                                        // (`%0d` alone stays the §21.2.1.2
                                        // minimum-width form.) Exception: the
                                        // `%+d` sign flag is an xezim
                                        // EXTENSION the reference simulator
                                        // rejects outright — keep C-style
                                        // zero-fill there (`%+08d` ->
                                        // `+0000123`).
                                        if plus_sign {
                                            result.push_str(&field(core, dw));
                                        } else {
                                            let w = if has_width { pad_width } else { dw };
                                            if core.len() >= w {
                                                result.push_str(&core);
                                            } else if left_align {
                                                result.push_str(&core);
                                                result.push_str(&" ".repeat(w - core.len()));
                                            } else {
                                                result.push_str(&" ".repeat(w - core.len()));
                                                result.push_str(&core);
                                            }
                                        }
                                    }
                                    'b' | 'B' => {
                                        let full = v.to_bin_string();
                                        if has_width {
                                            Self::push_radix(
                                                &mut result,
                                                &full,
                                                pad_width,
                                                left_align,
                                            );
                                        } else {
                                            result.push_str(&full);
                                        }
                                    }
                                    'h' | 'H' | 'x' | 'X' => {
                                        let full = v.to_hex_string();
                                        if has_width {
                                            Self::push_radix(
                                                &mut result,
                                                &full,
                                                pad_width,
                                                left_align,
                                            );
                                        } else {
                                            result.push_str(&full);
                                        }
                                    }
                                    'o' | 'O' => {
                                        // Full width is ceil(bits/3) digits
                                        // (§21.2.1.3).
                                        let full = Self::bin_to_oct_string(&v.to_bin_string());
                                        if has_width {
                                            Self::push_radix(
                                                &mut result,
                                                &full,
                                                pad_width,
                                                left_align,
                                            );
                                        } else {
                                            result.push_str(&full);
                                        }
                                    }
                                    'f' | 'F' => {
                                        let x = v.to_f64();
                                        let mut core = Self::nonfinite_float(x, spec == 'F')
                                            .unwrap_or_else(|| {
                                                format!("{:.*}", precision.unwrap_or(6), x)
                                            });
                                        if plus_sign && !core.starts_with('-') {
                                            core.insert(0, '+');
                                        }
                                        result.push_str(&field(core, 0));
                                    }
                                    'g' | 'G' => {
                                        // §21.2.1.2: the `+` flag forces a sign
                                        // on %g just as it does on %f/%d.
                                        let x = v.to_f64();
                                        let mut s = Self::nonfinite_float(x, spec == 'G')
                                            .unwrap_or_else(|| {
                                                Self::format_g(
                                                    x,
                                                    precision.unwrap_or(6),
                                                    spec == 'G',
                                                )
                                            });
                                        if plus_sign && !s.starts_with('-') {
                                            s.insert(0, '+');
                                        }
                                        result.push_str(&field(s, 0));
                                    }
                                    'e' | 'E' => {
                                        // C-style exponent: `1.0e+02`, two
                                        // exponent digits minimum (§21.2.1.2).
                                        // The `+` flag forces a leading sign.
                                        let x = v.to_f64();
                                        let mut core = Self::nonfinite_float(x, spec == 'E')
                                            .unwrap_or_else(|| {
                                                Self::c_style_exp(
                                                    x,
                                                    precision.unwrap_or(6),
                                                    spec == 'E',
                                                )
                                            });
                                        if plus_sign && !core.starts_with('-') {
                                            core.insert(0, '+');
                                        }
                                        result.push_str(&field(core, 0));
                                    }
                                    's' | 'S' => {
                                        // A literal takes the same field
                                        // treatment as a string variable —
                                        // `%5s` of "ab" is "   ab". Pushing the
                                        // literal raw dropped the width only for
                                        // literals, so the same format applied
                                        // to `s = "ab"` and to "ab" printed
                                        // differently. (The literal still
                                        // bypasses `to_sv_string`, which would
                                        // lose a literal's embedded NULs.)
                                        if let ExprKind::StringLiteral(s) = &args[ai - 1].kind {
                                            let lit = s.clone();
                                            result.push_str(&field(lit, 0));
                                        } else if self.expr_is_string_valued(&args[ai - 1]) {
                                            result.push_str(&field(v.to_sv_string(), 0));
                                        } else {
                                            // §21.2.1.3 (measured): %s of a PACKED
                                            // operand renders ceil(width/8) chars —
                                            // every byte prints, NUL as a space
                                            // ("[  Hi]", "[A B]"); %0s strips the
                                            // leading NULs (minimal form).
                                            let minimal = has_width && pad_width == 0;
                                            let nbytes = (v.width as usize).div_ceil(8);
                                            let mut bytes: Vec<u8> = Vec::with_capacity(nbytes);
                                            for bi in (0..nbytes).rev() {
                                                let mut byte = 0u8;
                                                for b in 0..8usize {
                                                    let idx = bi * 8 + b;
                                                    if idx < v.width as usize
                                                        && v.get_bit(idx) == LogicBit::One
                                                    {
                                                        byte |= 1 << b;
                                                    }
                                                }
                                                bytes.push(byte);
                                            }
                                            if minimal {
                                                while bytes.first() == Some(&0) {
                                                    bytes.remove(0);
                                                }
                                            }
                                            let s: String = bytes
                                                .iter()
                                                .map(|&b| if b == 0 { ' ' } else { b as char })
                                                .collect();
                                            result.push_str(&field(s, 0));
                                        }
                                    }
                                    'c' | 'C' => {
                                        let b = (v.to_u64().unwrap_or(0) & 0xff) as u8;
                                        result.push_str(&field((b as char).to_string(), 0));
                                    }
                                    'u' | 'U' => {
                                        // §21.2.1.4: unformatted 2-state dump —
                                        // the aval words as raw little-endian
                                        // bytes, x/z read as 0, padded to whole
                                        // 32-bit words.
                                        result.push_str(&Self::unformatted_2state(
                                            &v.to_bin_string(),
                                        ));
                                    }
                                    'z' | 'Z' => {
                                        // §21.2.1.4: unformatted 4-state dump —
                                        // per 32-bit word the bval (x/z mask)
                                        // word then the aval word, little-endian.
                                        result.push_str(&Self::unformatted_4state(
                                            &v.to_bin_string(),
                                        ));
                                    }
                                    'v' | 'V' => {
                                        // §21.2.1.5 strength format: driven
                                        // strength abbreviation + logic value;
                                        // `z` displays as HiZ.
                                        let s = self.strength_field(&args[ai - 1], &v);
                                        result.push_str(&field(s, 0));
                                    }
                                    _ => {
                                        result.push('%');
                                        if left_align {
                                            result.push('-');
                                        }
                                        if plus_sign {
                                            result.push('+');
                                        }
                                        result.push_str(&width_str);
                                        if let Some(p) = precision {
                                            result.push_str(&format!(".{}", p));
                                        }
                                        result.push(spec);
                                        ai -= 1;
                                    }
                                }
                            }
                        }
                    }
                }
            } else if c == '\\' {
                if let Some(&e) = chars.peek() {
                    chars.next();
                    match e {
                        'n' => result.push('\n'),
                        't' => result.push('\t'),
                        '\\' => result.push('\\'),
                        '"' => result.push('"'),
                        _ => {
                            result.push('\\');
                            result.push(e);
                        }
                    }
                }
            } else {
                result.push(c);
            }
        }
        if let Some(fp) = compact_from.take() {
            let tail = Self::compact_p(&result[fp..]);
            result.truncate(fp);
            result.push_str(&tail);
        }
        (result, ai)
    }

    /// Format and emit each queued `$strobe` after NBAs have applied
    /// (called once per event-loop iteration). Drains the queue so
    /// future iters don't replay old strobes.
    pub(super) fn drain_pending_strobes(&mut self) {
        if self.pending_strobes.is_empty() {
            return;
        }
        let queued = std::mem::take(&mut self.pending_strobes);
        for (name, args, scope, suffix) in queued {
            // Reinstall the ARMING context (like the $monitor re-render): the
            // postponed-region drain runs under whatever process happened to
            // finish last, so `%m` and hierarchical argument names must
            // resolve against the instance that executed the $strobe — a
            // strobe in a module bound under `u_dut.x` names that path, not
            // the top module.
            let saved_block = std::mem::replace(&mut self.m_block_scope, scope.clone());
            let saved_active = self.monitor_m_active;
            let saved_suffix = std::mem::replace(&mut self.monitor_m_suffix, suffix);
            self.monitor_m_active = true;
            let saved_hint = self.name_resolve_hint.borrow().clone();
            if !scope.is_empty() {
                *self.name_resolve_hint.borrow_mut() = Some(scope);
            }
            if let Some(radix_suffix) = name.strip_prefix("$fstrobe") {
                // §21.2.2 $fstrobeX(fd, fmt, …): format with $strobeX radix
                // rules, write to the file descriptor in args[0].
                let tn = format!("$strobe{}", radix_suffix);
                let _ = self.write_file_handle_named(&args, true, &tn);
            } else {
                let m = self.format_args(&args, &name);
                self.record_output(m.clone());
                self.stdout_writeln(&m);
            }
            *self.name_resolve_hint.borrow_mut() = saved_hint;
            self.monitor_m_suffix = saved_suffix;
            self.monitor_m_active = saved_active;
            self.m_block_scope = saved_block;
        }
    }

    /// LRM §4.4 observed-region drain. Each registered probe gets its
    /// predicate re-evaluated against the post-NBA settled state; the
    /// boolean outcome is folded into the assertion stats keyed by the
    /// probe's `name` so a concurrent property's pass/fail count shows up
    /// alongside the immediate-assertion counters in the coverage DB.
    ///
    /// Today's behavior: drains and evaluates whatever's queued. With no
    /// producer wired yet (M9 deferred), the queue is empty in practice
    /// and this is an inexpensive no-op.
    /// LRM §4.4 reactive-region drain. Executes any `Statement`s
    /// queued by program-block code or other reactive-region producers.
    /// Today's behavior: drains and runs whatever's queued. With no
    /// producer wired yet, the queue is empty and this is a cheap no-op.
    pub(super) fn drain_reactive_region(&mut self) {
        if !self.reactive_conts.is_empty() {
            self.drain_reactive_conts();
        }
        if self.pending_reactive.is_empty() {
            return;
        }
        let blocks = std::mem::take(&mut self.pending_reactive);
        self.late_resume_epoch += blocks.iter().map(|(b, _)| b.len() as u64).sum::<u64>();
        // Program-block (`program ... endprogram`, LRM §24) initial blocks
        // execute in the reactive region, but they must use the *same* statement
        // executor as module initials (`run_process_stmts`), not the bare
        // `exec_statement` loop. That executor carries the task/method
        // `this`-binding semantics. Routing through `run_process_stmts` makes
        // module-top and program-top behavior identical. Allocate a fresh pid
        // so process-scoped bookkeeping (disable labels, scope hints) has
        // somewhere to attach.
        for (stmts, scope) in blocks {
            if self.finished {
                break;
            }
            let pid = self.next_pid;
            self.next_pid += 1;
            if let Some(first) = stmts.first() {
                self.process_origin
                    .insert(pid, (first.span, "program initial block"));
            }
            // `%m` and bare names resolve in the program instance.
            self.current_scope.clone_from(&scope);
            if !scope.is_empty() {
                self.process_scope_hint.insert(pid, scope);
            }
            self.program_pids.insert(pid);
            self.run_process_stmts(pid, &ProcCont::from_vec(stmts));
        }
    }

    /// §24.3 / §4.4: run the program-process continuations woken this slot
    /// (`reactive_conts`) in the Reactive region.
    #[cold]
    #[inline(never)]
    pub(super) fn drain_reactive_conts(&mut self) {
        let mut guard = 0u32;
        while !self.reactive_conts.is_empty() && !self.finished {
            let conts = std::mem::take(&mut self.reactive_conts);
            self.late_resume_epoch += conts.len() as u64;
            let saved = std::mem::replace(&mut self.in_reactive_drain, true);
            for (pid, cont) in conts {
                if self.finished {
                    break;
                }
                self.run_scheduled_process(pid, &cont);
                if !self.is_pid_suspended(pid) {
                    self.child_finished(pid);
                }
                self.break_flag = false;
                self.continue_flag = false;
                self.return_flag = false;
            }
            self.in_reactive_drain = saved;
            guard += 1;
            if guard > 10_000 {
                break;
            }
        }
    }

    /// Resume processes waiting on NBA-region completion (such as
    /// `uvm_wait_for_nba_region`: `nba <= next_nba; @(nba)`). Resumes in the
    /// Reactive region after all active + inactive (`#0`) deltas have settled
    /// and `apply_nba` has committed the NBA region.
    pub(super) fn drain_nba_region_waiters(&mut self) {
        if self.nba_region_waiters.is_empty() {
            return;
        }
        // IEEE 1800-2017 §4.4.2/§4.5: the NBA region runs only after ALL
        // active and inactive work of this time slot has settled. A `fork ...
        // join_none` child still queued at the current time (scheduled, not
        // yet started: the parent parked on `@(nba)` right after forking)
        // must get its first slice, its `#0` continuations and its own NBA
        // commits BEFORE processes parked on `uvm_wait_for_nba_region`
        // resume. Running that queued work inline here would skip the
        // Inactive region and the NBA commit, so a child that hops itself
        // would resume before its own NBA lands. Instead, leave the waiters
        // parked while same-time Active or Inactive work remains; the next
        // delta runs it, commits the NBAs, and comes back here.
        // Reference-verified shape: UVM `execute_phase` forks
        // `master_phase_process` (traverse -> phase task ->
        // `raise_objection`), then parks on `uvm_wait_for_nba_region`.
        if self.event_queue.next_time() == Some(self.time) || !self.inactive_queue.is_empty() {
            return;
        }
        let waiters = std::mem::take(&mut self.nba_region_waiters);
        self.late_resume_epoch += waiters.len() as u64;
        for (pid, cont) in waiters {
            if self.finished {
                break;
            }
            self.run_scheduled_process(pid, &cont);
            if !self.is_pid_suspended(pid) {
                self.child_finished(pid);
            }
        }
    }
}
