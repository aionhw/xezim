//! Simulator compilation.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    pub fn compile(&mut self) {
        if self.compiled {
            return;
        }
        self.instrument_code_coverage();
        let trace_compile_phases = std::env::var_os("XEZIM_COMPILE_PHASES").is_some();
        let mut compile_phase_start = std::time::Instant::now();
        let mark_compile_phase = |label: &str, start: &mut std::time::Instant| {
            crate::rss_trace(label);
            if trace_compile_phases {
                eprintln!(
                    "[COMPILE-PHASE] {}: {:.1}ms",
                    label,
                    start.elapsed().as_secs_f64() * 1000.0
                );
            }
            *start = std::time::Instant::now();
        };
        // §13.4.1: a function's return variable is named after the function
        // and, for a packed-struct return type, has member layout — but it is
        // not a signal, so nothing registers one. Without it `mk.a = …;`
        // inside the body has no store path and the whole call stays on the
        // AST interpreter (measured 11x slower than the same function written
        // `return '{…}`). Register it once under a key that cannot collide
        // with a signal name.
        {
            let ret_layouts: Vec<(String, Vec<(String, u32, u32)>)> = self
                .module
                .functions
                .iter()
                .filter_map(|(name, fd)| {
                    let fields = super::elaborate::packed_struct_field_layout(
                        &fd.return_type,
                        &self.module.parameters,
                        &self.module.typedefs,
                        &self.module.typedef_types,
                    )?;
                    if fields.is_empty() {
                        return None;
                    }
                    let leaf = name.rsplit('.').next().unwrap_or(name).to_string();
                    Some((format!("fn ret:{}", leaf), fields))
                })
                .collect();
            for (k, v) in ret_layouts {
                self.module.packed_struct_fields.entry(k).or_insert(v);
            }
        }
        self.sanitize_class_hierarchy();
        // Answers memoized before the hierarchy was final do not hold.
        self.has_method_cache.borrow_mut().clear();
        self.init_two_state_struct_members();
        self.init_static_struct_holders();
        self.static_vif_array_names = self.static_vif_array_name_set();
        self.may_bind_static_vif =
            !self.vif_capable_static_set().is_empty() || !self.static_vif_array_names.is_empty();
        // LRM §14.3 — populate clocking-block metadata: per-cb clock
        // signal + per-signal direction. `tick_clocking_blocks`
        // consumes this to maintain `clocking_snapshots`.
        for (cb_name, cd) in self.module.clocking_blocks.iter() {
            let clk = cd.clock_signal.as_ref().map(|i| i.name.clone());
            let edge = match cd.clock_edge {
                Some(crate::ast::stmt::Edge::Negedge) => EdgeKind::Negedge,
                Some(crate::ast::stmt::Edge::Edge) => EdgeKind::LsbEdge,
                _ => EdgeKind::Posedge,
            };
            if let Some(clk) = clk {
                // §14.3 renaming: the meta/snapshot key on the RESOLVED NET;
                // `alias_map` translates the clocking-visible name to it.
                let mut alias_map: HashMap<String, String> = HashMap::default();
                let mut inouts: HashSet<String> = HashSet::default();
                let sigs: Vec<(String, bool)> = cd
                    .signals
                    .iter()
                    .map(|s| {
                        let is_input = matches!(
                            s.direction,
                            crate::ast::types::PortDirection::Input
                                | crate::ast::types::PortDirection::Inout
                        );
                        let net = match s.bound_to.as_ref().and_then(Self::flatten_member_path) {
                            Some(segs) if !segs.is_empty() => {
                                let n = segs.join(".");
                                alias_map.insert(s.name.name.clone(), n.clone());
                                n
                            }
                            _ => s.name.name.clone(),
                        };
                        if matches!(s.direction, crate::ast::types::PortDirection::Inout) {
                            inouts.insert(net.clone());
                        }
                        (net, is_input)
                    })
                    .collect();
                let vis: Vec<String> = cd.signals.iter().map(|s| s.name.name.clone()).collect();
                self.clocking_vis.insert(cb_name.clone(), vis);
                if !inouts.is_empty() {
                    if cd.is_default {
                        self.clocking_inouts
                            .insert("__xz_default_clocking".to_string(), inouts.clone());
                    }
                    self.clocking_inouts.insert(cb_name.clone(), inouts);
                }
                if !alias_map.is_empty() {
                    if cd.is_default {
                        self.clocking_sig_alias
                            .insert("__xz_default_clocking".to_string(), alias_map.clone());
                    }
                    self.clocking_sig_alias.insert(cb_name.clone(), alias_map);
                }
                // §14.11: alias the parser's reserved `##N` marker to the
                // `default clocking` block, so the desugared
                // `repeat (N) @(__xz_default_clocking)` resolves through the
                // same clocking_meta lookup as a named `@(cb)`.
                if cd.is_default {
                    self.clocking_meta.insert(
                        "__xz_default_clocking".to_string(),
                        (clk.clone(), sigs.clone()),
                    );
                    self.clocking_edge
                        .insert("__xz_default_clocking".to_string(), edge);
                    // `##0` sync marker resolves through the same lookups.
                    self.clocking_meta.insert(
                        "__xz_default_clocking0".to_string(),
                        (clk.clone(), sigs.clone()),
                    );
                    self.clocking_edge
                        .insert("__xz_default_clocking0".to_string(), edge);
                    self.default_clocking_cb = Some(cb_name.clone());
                }
                self.clocking_meta.insert(cb_name.clone(), (clk, sigs));
                self.clocking_edge.insert(cb_name.clone(), edge);
                self.clocking_prev_clock.insert(cb_name.clone(), 2);
                self.clocking_snapshots
                    .insert(cb_name.clone(), HashMap::default());
            }
        }
        // §14.4 output skews: evaluate the per-cb default and per-signal
        // output-skew expressions to ticks. Collected first (clones) because
        // eval_delay_ticks needs &mut self.
        let skew_specs: Vec<(
            String,
            bool,
            Option<Expression>,
            Vec<(String, Expression)>,
            Option<Expression>,
            Vec<(String, Expression)>,
        )> = self
            .module
            .clocking_blocks
            .iter()
            .map(|(n, cd)| {
                let out_per_sig: Vec<(String, Expression)> = cd
                    .signals
                    .iter()
                    .filter(|s| {
                        !matches!(
                            s.direction,
                            crate::ast::types::PortDirection::Input
                                | crate::ast::types::PortDirection::Inout
                        )
                    })
                    .filter_map(|s| s.skew.clone().map(|e| (s.name.name.clone(), e)))
                    .collect();
                let in_per_sig: Vec<(String, Expression)> = cd
                    .signals
                    .iter()
                    .filter(|s| {
                        matches!(
                            s.direction,
                            crate::ast::types::PortDirection::Input
                                | crate::ast::types::PortDirection::Inout
                        )
                    })
                    .filter_map(|s| s.skew.clone().map(|e| (s.name.name.clone(), e)))
                    .collect();
                (
                    n.clone(),
                    cd.is_default,
                    cd.default_output_skew.clone(),
                    out_per_sig,
                    cd.default_input_skew.clone(),
                    in_per_sig,
                )
            })
            .collect();
        let skew_ticks = |slf: &mut Self, cb: &str, e: &Expression| -> u64 {
            if let ExprKind::Ident(h) = &e.kind
                && h.path.len() == 1
                && h.path[0].name.name == crate::ast::decl::CLOCKING_ONE_STEP
            {
                return CB_SKEW_1STEP;
            }
            // A `#1ns` skew is an ABSOLUTE time literal (stored in seconds);
            // convert against the global tick directly — eval_delay_ticks'
            // module-timescale context isn't established at compile time.
            if let ExprKind::Number(crate::ast::expr::NumberLiteral::Time(secs)) = &e.kind {
                return (secs / slf.tick_s).round() as u64;
            }
            // §14.4: a plain number is in the time unit of the module or
            // interface declaring the block (unlike statement delays, the
            // elaborator does not pre-scale clocking skews).
            let scope = match cb.rsplit_once('.') {
                Some((inst, _)) => inst.to_string(),
                None => slf.module.name.clone(),
            };
            let saved = slf.timescale_scope_override.replace(scope);
            let (unit_exp, _) = slf.current_timescale_exp();
            slf.timescale_scope_override = saved;
            let v = slf.eval_expr(e);
            let f = if v.is_real {
                v.to_f64()
            } else {
                v.to_u64().unwrap_or(0) as f64
            };
            let scale = 10f64.powi(unit_exp - Self::secs_to_exp(slf.tick_s));
            (f * scale).round().max(0.0) as u64
        };
        for (cb, is_default, out_dskew, out_per_sig, in_dskew, in_per_sig) in skew_specs {
            // Process output skew. An output `#1step` skew drives one tick
            // (the global time precision) after the clocking event, as the
            // reference simulator does.
            let out_ticks = |t: u64| if t == CB_SKEW_1STEP { 1 } else { t };
            if let Some(e) = out_dskew {
                let t = out_ticks(skew_ticks(self, &cb, &e));
                if t > 0 {
                    self.clocking_out_skew.insert(cb.clone(), t);
                    if is_default {
                        self.clocking_out_skew
                            .insert("__xz_default_clocking".to_string(), t);
                    }
                }
            }
            let mut out_m: HashMap<String, u64> = HashMap::default();
            for (net, e) in out_per_sig {
                let t = out_ticks(skew_ticks(self, &cb, &e));
                out_m.insert(net, t);
            }
            if !out_m.is_empty() {
                self.clocking_sig_out_skew.insert(cb.clone(), out_m);
            }
            // Process input skew (§14.4): absent and `#1step` sample in the
            // Preponed region; an explicit `#0` samples in the Observed
            // region, so it is recorded too.
            if let Some(e) = in_dskew {
                let t = skew_ticks(self, &cb, &e);
                if t != CB_SKEW_1STEP {
                    self.clocking_in_skew.insert(cb.clone(), t);
                    if is_default {
                        self.clocking_in_skew
                            .insert("__xz_default_clocking".to_string(), t);
                    }
                }
            }
            let mut in_m: HashMap<String, u64> = HashMap::default();
            for (net, e) in in_per_sig {
                let t = skew_ticks(self, &cb, &e);
                in_m.insert(net, t);
            }
            if !in_m.is_empty() {
                self.clocking_sig_in_skew.insert(cb, in_m);
            }
        }
        // §14.4 `input #N`: track the history of every net some clocking
        // input samples with an explicit positive skew.
        let mut hist: Vec<(String, u64)> = Vec::new();
        for (cb, (_, sigs)) in self.clocking_meta.iter() {
            let dskew = self
                .clocking_in_skew
                .get(cb)
                .copied()
                .unwrap_or(CB_SKEW_1STEP);
            let vis = self.clocking_vis.get(cb);
            for (i, (net, is_input)) in sigs.iter().enumerate() {
                if !*is_input {
                    continue;
                }
                let v = vis.and_then(|l| l.get(i)).unwrap_or(net);
                let skew = self
                    .clocking_sig_in_skew
                    .get(cb)
                    .and_then(|m| m.get(v))
                    .copied()
                    .unwrap_or(dskew);
                if skew != 0 && skew != CB_SKEW_1STEP {
                    hist.push((net.clone(), skew));
                }
            }
        }
        for (net, skew) in hist {
            let h = self.clocking_hist.entry(net).or_default();
            h.span = h.span.max(skew);
        }
        // Friendly fallback: `##N` with a single clocking block and no
        // explicit `default` — unambiguous, so use it (strict LRM would
        // demand the default declaration).
        if !self.clocking_meta.contains_key("__xz_default_clocking")
            && self.clocking_meta.len() == 1
        {
            self.default_clocking_cb = self.clocking_meta.keys().next().cloned();
            if let Some(v) = self.clocking_meta.values().next().cloned() {
                self.clocking_meta
                    .insert("__xz_default_clocking".to_string(), v.clone());
                self.clocking_meta
                    .insert("__xz_default_clocking0".to_string(), v);
            }
            if let Some((k, e)) = self
                .clocking_edge
                .iter()
                .next()
                .map(|(k, e)| (k.clone(), *e))
            {
                let _ = k;
                self.clocking_edge
                    .insert("__xz_default_clocking".to_string(), e);
                self.clocking_edge
                    .insert("__xz_default_clocking0".to_string(), e);
            }
        }
        // Seed the runtime string_signals set from the elab-time map so that
        // `expr_is_string_valued` recognises module-level `string` signals
        // BEFORE any procedural VarDecl has run (those add their own names
        // dynamically). Without this, `{a, b}` with `string a, b;` falls
        // into the bit-concat path and truncates to one operand's 1024-bit
        // width. LRM §6.16 / §11.4.12.
        for name in self.module.string_signals.iter() {
            self.string_signals.insert(name.clone());
        }

        // Register static member collections (`static T m[$]`, `static T
        // m[KEY]`) as a single shared global store under their bare name, so
        // queue/assoc ops and `inside {m}` resolve. riscv-dv's instruction
        // tables (instr_names, instr_template, ...) live here.
        // The `associative_arrays` bool is ``is the key `string`'' — seed it
        // from each declaring class's `static_assoc_key_types` so a
        // `static T map[string]` gets string storage (hashing the bare name
        // as NUMERIC would corrupt foreach()/first()/next()).
        let _static_cols: Vec<(String, bool, u32, bool)> = self
            .module
            .classes
            .values()
            .flat_map(|cd| {
                cd.static_collections
                    .iter()
                    .map(move |(name, is_assoc, width)| {
                        let is_str = cd
                            .static_assoc_key_types
                            .get(name)
                            .map(|kt| kt == "string")
                            .unwrap_or(false);
                        (name.clone(), *is_assoc, *width, is_str)
                    })
            })
            .collect();
        // A static-collection bare name declared by TWO SIBLING classes must
        // be stored per DECLARING class (§8.9) — compute that set up front.
        // A static-collection name needs per-DECLARING-class storage whenever
        // two or more classes declare it (§8.9 each gets its own cell). This
        // covers sibling collisions (the four `uvm_cmdline_*` classes each
        // declaring `static … settings[$]`) AND a derived class that
        // REDECLARES a base's static collection (`d` extends `b`, both declare
        // `static m_q[$]` → `d::m_q` and `b::m_q` are separate). Inherited-ONLY
        // access (a single declarer further up the chain) is not a collision
        // and keeps the bare, shared store the runtime relies on.
        // `module.classes` is fully populated here.
        let mut colliding_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let name_counts: std::collections::HashMap<String, Vec<String>> = self
            .module
            .classes
            .values()
            .fold(std::collections::HashMap::new(), |mut acc, cd| {
                let mut seen: Vec<&str> = Vec::new();
                for (nm, _, _) in &cd.static_collections {
                    if !seen.contains(&nm.as_str()) {
                        acc.entry(nm.clone()).or_default().push(cd.name.clone());
                        seen.push(nm.as_str());
                    }
                }
                acc
            });
        for (nm, classes) in &name_counts {
            // A single class may be materialised under several keys in
            // `module.classes` (parameterized specializations that collapse
            // to the same name); count DISTINCT class names so we detect true
            // cross-class collisions / redeclarations, not the same class
            // twice. The only names left are genuinely declared by 2+ classes.
            let mut uniq: Vec<&String> = classes.iter().collect();
            uniq.sort();
            uniq.dedup();
            if uniq.len() >= 2 {
                colliding_names.insert(nm.to_string());
            }
        }
        self.colliding_static_colls = colliding_names.iter().cloned().collect();
        self.colliding_static_colls_ready = true;
        let mut static_cols: Vec<(String, bool, u32, bool)> = Vec::new();
        for cd in self.module.classes.values() {
            for (name, is_assoc, width) in &cd.static_collections {
                let is_str = cd
                    .static_assoc_key_types
                    .get(name)
                    .map(|kt| kt == "string")
                    .unwrap_or(false);
                // A static collection name declared by MORE THAN ONE class
                // (e.g. the four sibling `uvm_cmdline_*` classes each
                // declaring `static … settings[$]`) must be stored per
                // DECLARING class so the classes don't share one cell (§8.9).
                // A uniquely-declared name keeps the bare store the rest of
                // the runtime relies on.
                let keyed = colliding_names.contains(name.as_str());
                static_cols.push((
                    if keyed {
                        format!("{}::{}", cd.name, name)
                    } else {
                        name.clone()
                    },
                    *is_assoc,
                    *width,
                    is_str,
                ));
            }
        }
        self.register_struct_member_collections();
        for (name, is_assoc, width, is_str) in static_cols {
            if is_assoc {
                self.module
                    .associative_arrays
                    .entry(name.clone())
                    .or_insert(is_str);
            } else {
                self.module.dynamic_arrays.insert(name.clone());
                self.module
                    .arrays
                    .entry(name.clone())
                    .or_insert((0, 63, width));
                self.set_queue_size(&name, 0);
            }
        }

        // Fixed-size static array members (`static int S[3]`) share ONE store
        // per class (§8.9), so — like the collections above — they register
        // globally under the bare name rather than per-instance as
        // `<handle>#name`. Bounds are real (not the 0..63 queue buffer), and
        // every element is defaulted so an index read or `foreach` sees a
        // populated range instead of x.
        let static_fixed: Vec<(String, String, i64, i64, u32)> = self
            .module
            .classes
            .values()
            .flat_map(|c| {
                c.static_fixed_arrays
                    .iter()
                    .map(|(n, lo, hi, w)| (c.name.clone(), n.clone(), *lo, *hi, *w))
            })
            .collect();
        // §8.9: ONE copy per DECLARING class — the store is keyed
        // `{Class}::{member}` (each class's static_fixed_arrays lists only
        // its OWN declarations, so the key is by construction the declarer;
        // inherited access walks the chain via `static_fixed_key_in`). Two
        // classes declaring the same member name get separate stores.
        for (cls, name, lo, hi, width) in static_fixed {
            let key = format!("{}::{}", cls, name);
            self.module
                .arrays
                .entry(key.clone())
                .or_insert((lo, hi, width));
            // §6.8: a 4-STATE element type defaults to x, not 0 — only
            // two-state elements (int/byte/bit...) start at zero. The
            // declared type is on the owning class's property table.
            let two_state = self
                .module
                .classes
                .get(&cls)
                .and_then(|c| c.property_types.get(&name))
                .is_some_and(|dt| {
                    super::elaborate::is_type_two_state_resolved(dt, &self.module.typedef_types)
                });
            let default = if two_state {
                Value::zero(width)
            } else {
                Value::all_x(width)
            };
            for i in lo..=hi {
                let elem = format!("{}[{}]", key, i);
                self.widths.insert(elem.clone(), width);
                // `insert` (not the map's `entry`) keeps the element index
                // of `SignalMap` exact.
                if !self.signals.contains_key(&elem) {
                    self.signals.insert(elem, default.clone());
                }
            }
        }

        // §8.9 (issue #423): a static fixed array property with an
        // INITIALIZER (`static int unsigned ARR [3] = '{10, 20, 30};`)
        // got its shape and default cells above, but the pattern's values
        // were never distributed — `Cls::ARR[i]` (and a static method's
        // bare `ARR[i]`) read the startup default (0/x) forever.
        // Evaluate the initializer per element into the cells.
        // Parameterized classes are skipped: their static initializers
        // run per specialization (see the static_call_inits note below)
        // and these shared cells cannot hold per-spec values.
        let static_fixed_inits: Vec<(String, String, Expression)> = self
            .module
            .classes
            .iter()
            .filter(|(cn, _)| !self.class_is_parameterized(cn))
            .flat_map(|(cn, cd)| {
                cd.property_inits
                    .iter()
                    .filter(|(p, _)| {
                        cd.static_fixed_arrays
                            .iter()
                            .any(|(n, ..)| n.as_str() == p.as_str())
                    })
                    .map(|(p, e)| (cn.clone(), p.clone(), e.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        for (cn, prop, init) in static_fixed_inits {
            self.initialize_class_fixed_array(&cn, &prop, &init);
        }

        // Run static property initializers that call functions, e.g. riscv-dv's
        // per-instruction `static bit _ = riscv_instr::register(NAME)` (the
        // DEFINE_INSTR registration). The lazy static-init path only stores the
        // const-evaluated value (0 for a call), so the side effect — populating
        // instr_registry — would never happen. Execute them once at startup
        // with each declaring class's static context (no `this`).
        let static_call_inits: Vec<(String, String, Expression)> = self
            .module
            .classes
            .iter()
            // §6.8 / §8.9: a module class's compatibility alias is the
            // same type as its instance-qualified entry, not a second
            // declaration whose initializer may execute again.
            .filter(|(cn, cd)| {
                cd.declaring_module.is_none()
                    || *cn == &cd.name
                    || !self.module.classes.get(&cd.name).is_some_and(|canonical| {
                        // Cached designs reconstruct separate Arcs for
                        // aliases; declaration identity survives that.
                        canonical.name == cd.name
                            && canonical.declaring_module == cd.declaring_module
                    })
            })
            .flat_map(|(cn, cd)| {
                cd.property_inits
                    .iter()
                    .filter(|(p, e)| {
                        cd.static_properties.contains(*p)
                            && (Self::expr_contains_call(e)
                                // bare `= new` (no parens) parses as an Ident,
                                // not a Call — `static C inst = new;` must
                                // still construct (§8.9)
                                || matches!(&e.kind, ExprKind::Ident(h)
                                    if h.path.len() == 1 && h.path[0].name.name == "new"))
                    })
                    .map(|(p, e)| (cn.clone(), p.clone(), e.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        // §6.8 / §8.9: a package class is listed under `pkg::C` and `C`.
        // Its initializer runs once, and every spelling's cell holds the
        // result. Keyed by declaration (name, property, initializer span) so
        // cached designs, which rebuild separate Arcs, dedupe too.
        let decl_of =
            |classes: &HashMap<String, Arc<crate::compiler::elaborate::ElaboratedClass>>,
             cn: &String,
             prop: &String,
             init: &Expression| {
                let name = classes
                    .get(cn)
                    .map_or_else(|| cn.clone(), |cd| cd.name.clone());
                (name, prop.clone(), init.span.start, init.span.end)
            };
        let mut spellings: HashMap<(String, String, usize, usize), Vec<String>> =
            HashMap::default();
        for (cn, prop, init) in &static_call_inits {
            spellings
                .entry(decl_of(&self.module.classes, cn, prop, init))
                .or_default()
                .push(cn.clone());
        }
        for (cn, prop, init) in static_call_inits {
            let decl = decl_of(&self.module.classes, &cn, &prop, &init);
            let Some(names) = spellings.remove(&decl) else {
                continue;
            };
            // Run under the class's own name when it is registered: other
            // static cells the initializer writes (`m_policy_names[...]` in
            // uvm_reg_field) are read through that spelling.
            let cn = if names.contains(&decl.0) { decl.0 } else { cn };
            // A parameterized class's static initializers must NOT run here
            // with no active specialization (`current_spec` is None at
            // startup). Doing so collapses every value specialization
            // (e.g. `special_comp#(1)` and `#(2)`) onto ONE bare-typedef
            // typeid, because the class-local typedef
            // `typedef special_comp#(N) special_comp_type;` cannot resolve
            // its value parameter `N` without a spec. Instead, these run
            // per-specialization via `ensure_spec_statics`, triggered when
            // each specialization is first instantiated (see
            // `instantiate_class_with_type_args`).
            if self.class_is_parameterized(&cn) {
                continue;
            }
            // A STATIC mailbox/semaphore with `= new()` (e.g. class
            // `static mailbox m_box = new();`) must be allocated as a
            // real container — eval_expr of bare new() doesn't create one, so
            // try_put/get silently fail.
            let cont_kind = self
                .module
                .classes
                .get(&cn)
                .and_then(|cd| cd.properties.get(&prop))
                .and_then(|s| s.type_name.as_deref())
                .and_then(Self::container_base);
            let (is_new, ctor_args): (bool, Vec<Expression>) = match &init.kind {
                ExprKind::Call { func, args }
                    if matches!(&func.kind, ExprKind::Ident(h)
                        if h.path.len() == 1 && h.path[0].name.name == "new") =>
                {
                    (true, args.clone())
                }
                ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].name.name == "new" => {
                    (true, Vec::new())
                }
                _ => (false, Vec::new()),
            };
            // Seed before evaluating: a constructor or initializer function
            // can read this static while it still holds its default. Unseeded,
            // the lazy read ran the initializer again (a second constructor
            // call, whose own read of the handle saw x).
            let seed = self
                .module
                .classes
                .get(&cn)
                .and_then(|cd| cd.properties.get(&prop))
                .map(|p| p.value.clone())
                .unwrap_or_else(|| Value::zero(32));
            let cells: Vec<String> = names
                .iter()
                .flat_map(|n| {
                    std::iter::once(format!("{}::{}", n, prop))
                        .chain(self.static_prop_key(n, &prop))
                })
                .collect();
            for key in &cells {
                if !self.class_statics.contains_key(key) {
                    self.class_statics.insert(key.clone(), seed.clone());
                }
            }
            let v = if let (Some(kind), true) = (cont_kind, is_new) {
                // §15.3.1/§15.4.1: the constructor argument is the initial
                // key count / the bound — allocating a zero-key semaphore
                // made the first `get()` block forever.
                let ch = self.alloc_builtin_container(kind, &ctor_args);
                Value::from_u64(ch as u64, 32)
            } else if is_new {
                // A CLASS-typed static with `= new(...)`/`= new` constructs
                // the shared instance once at startup (§8.9) — eval_expr of a
                // bare `new` has no receiver class and yielded 0 (null).
                self.class_context_stack.push(Some(cn.clone()));
                self.this_stack.push(None);
                let v = self
                    .construct_static_property(&cn, &prop, &ctor_args)
                    .unwrap_or_else(|| Value::zero(32));
                self.this_stack.pop();
                self.class_context_stack.pop();
                v
            } else {
                self.class_context_stack.push(Some(cn.clone()));
                self.this_stack.push(None);
                let v = self.eval_expr(&init);
                self.this_stack.pop();
                self.class_context_stack.pop();
                v
            };
            for key in cells {
                self.class_statics.insert(key, v.clone());
            }
        }

        // §6.8/§8.25: a referenced specialization's static initialization
        // precedes initial processes, including a typedef whose only effect
        // is a constructor registering its shared instance. Class-local
        // aliases of a generic class are resolved only under a concrete spec.
        let mut referenced = self.referenced_class_typedefs.clone();
        for cd in self.module.classes.values() {
            if !self.class_is_parameterized(&cd.name) {
                referenced.extend(cd.typedef_targets.values().cloned());
            }
        }
        let mut referenced_specs: Vec<(String, String)> = referenced
            .iter()
            .filter_map(|dt| self.spec_from_typedef_dt(dt))
            .collect();
        for (name, args) in &self.module.class_type_args {
            if let Some(base) = self
                .module
                .signals
                .get(name)
                .and_then(|s| s.type_name.as_ref())
            {
                if self.class_is_parameterized(base) {
                    let frags: Option<Vec<String>> =
                        args.iter().map(|e| self.expr_to_spec_fragment(e)).collect();
                    if let Some(frags) = frags {
                        referenced_specs.push((
                            base.clone(),
                            self.canonicalize_spec_sig(base, &frags.join(",")),
                        ));
                    }
                }
            }
        }
        referenced_specs.sort();
        referenced_specs.dedup();
        for (base, sig) in referenced_specs {
            self.ensure_spec_statics(&base, &sig);
        }

        // §8.25/§8.9: real simulators elaborate the static initializers of
        // EVERY referenced class specialization at time 0 — UVM's factory
        // registration (`local static this_type me = get();` inside each
        // `*_registry#(T,"T")`) depends on exactly that. xezim runs per-spec
        // statics lazily on first instantiation, which is too late for
        // `run_test`/`create_*_by_name` (nothing is registered when the
        // by-name lookup happens). Eagerly run them for every registry
        // specialization referenced by a class-scope `type_id` typedef.
        let reg_specs: Vec<(String, String)> = self
            .module
            .classes
            .iter()
            .filter_map(|(_, cd)| {
                let dt = cd.typedef_targets.get("type_id")?;
                if let DataType::TypeReference {
                    name, type_args, ..
                } = dt
                {
                    if name.name.name.contains("registry") && !type_args.is_empty() {
                        let base = name.name.name.clone();
                        let frags: Vec<Option<String>> = type_args
                            .iter()
                            .map(|e| self.expr_to_spec_fragment(e))
                            .collect();
                        if frags.iter().any(|f| f.is_none()) {
                            return None;
                        }
                        let sig = frags
                            .into_iter()
                            .map(|f| f.unwrap())
                            .collect::<Vec<_>>()
                            .join(",");
                        return Some((base, sig));
                    }
                }
                None
            })
            .collect();
        let reg_dbg = std::env::var("XEZIM_REG_DBG").is_ok();
        for (base, sig) in reg_specs {
            if reg_dbg {
                eprintln!("[REG] ensure {}#({})", base, sig);
            }
            self.ensure_spec_statics(&base, &sig);
            // 1800.2: the factory registration lives one hop further — in
            // `uvm_registry_common#(...)`'s `m__initialized =
            // __deferred_init();` static, reached through the registry
            // class's own `common_type` typedef. Resolve that typedef under
            // the registry's spec and run its statics too (harmless no-op
            // for 1.x registries, which have no `common_type`).
            let saved = self.current_spec.clone();
            self.current_spec = Some((base.clone(), sig.clone()));
            let common = self.resolve_class_member_typedef_spec(&base, "common_type");
            if reg_dbg {
                eprintln!("[REG]   common_type -> {:?}", common);
            }
            if let Some((b2, s2)) = common {
                self.ensure_spec_statics(&b2, &s2);
            }
            self.current_spec = saved;
        }

        // The base loop resolves each class's `type_id` registry spec with the
        // class's OWN parameters at their DEFAULTS (e.g. `base_class#(0)` from
        // `typedef uvm_object_registry#(base_class#(P)) type_id;`) — correct
        // when a parameterized class is only ever used at its default impl, but
        // the factory/registry tests instantiate NON-default specializations
        // (`typedef base_class#(9) BaseType;`) and assert their registry is
        // registered BEFORE any `create()`. Register the concrete specializations
        // referenced by typedefs too: every module/class-scoped typedef that
        // names a parameterized class (or a registry) yields its concrete spec,
        // and we run the statics of the associated registry specialization
        // (via the class's `type_id` or directly for a registry typedef).
        let mut extra_specs: Vec<(String, String)> = Vec::new();
        {
            use crate::ast::types::DataType;
            // Snapshot typedef targets (module + all classes) so the param-
            // resolving processing below can borrow `self` mutably without
            // aliasing the iteration borrows.
            let mut targets: Vec<DataType> = Vec::new();
            for dt in self.module.typedef_types.values() {
                targets.push(dt.clone());
            }
            for cd in self.module.classes.values() {
                for dt in cd.typedef_targets.values() {
                    targets.push(dt.clone());
                }
            }
            for target in targets {
                let DataType::TypeReference {
                    name, type_args, ..
                } = &target
                else {
                    continue;
                };
                if type_args.is_empty() {
                    continue;
                }
                let base = name.name.name.clone();
                if !self.module.classes.contains_key(&base) {
                    continue;
                }
                // Resolve concrete args (a type arg may itself be a typedef,
                // e.g. `uvm_object_registry#(BaseType)`).
                let frags: Vec<String> = type_args
                    .iter()
                    .filter_map(|e| self.expr_to_spec_fragment(e))
                    .collect();
                if frags.len() != type_args.len() {
                    continue;
                }
                let cls = base.split('#').next().unwrap_or(&base).to_string();
                let sig = self.canonicalize_spec_sig(&cls, &frags.join(","));
                if sig.is_empty() {
                    continue;
                }
                // Registry typedef itself → register this specialization.
                if base.contains("registry") {
                    extra_specs.push((base.clone(), sig.clone()));
                    continue;
                }
                // Parameterized user class → register ITS `type_id` registry
                // with this class's params bound to the concrete args.
                let tid = self
                    .module
                    .classes
                    .get(&cls)
                    .and_then(|c| c.typedef_targets.get("type_id").cloned());
                if tid.is_some() {
                    let saved = self.current_spec.take();
                    self.current_spec = Some((cls.clone(), sig.clone()));
                    if let Some((rb, rsig)) =
                        self.resolve_class_member_typedef_spec(&cls, "type_id")
                    {
                        extra_specs.push((rb, rsig));
                    }
                    self.current_spec = saved;
                }
            }
        }
        for (base, sig) in extra_specs {
            if reg_dbg {
                eprintln!("[REG2] ensure {}#({})", base, sig);
            }
            self.ensure_spec_statics(&base, &sig);
            let saved = self.current_spec.clone();
            self.current_spec = Some((base.clone(), sig.clone()));
            let common = self.resolve_class_member_typedef_spec(&base, "common_type");
            if let Some((b2, s2)) = common {
                self.ensure_spec_statics(&b2, &s2);
            }
            self.current_spec = saved;
        }
        mark_compile_phase("registry spec static init", &mut compile_phase_start);

        // Evaluate parameter expressions whose initializers contained function
        // calls and were deferred by the elaborator.
        let deferred: Vec<(String, Expression)> = self.module.deferred_param_exprs.clone();
        self.in_const_param_eval = true;
        for (pname, expr) in deferred {
            let v = self.eval_expr(&expr);
            let w = self.lookup_signal_width(&pname).unwrap_or(v.width);
            let rv = v.resize(w);
            self.set_signal_value_by_name(&pname, rv);
        }
        self.in_const_param_eval = false;
        mark_compile_phase("runtime metadata and static init", &mut compile_phase_start);
        self.classify_always_blocks();
        mark_compile_phase("classify always blocks", &mut compile_phase_start);
        // Signal table is final here: optionally widen the NBA index's dense
        // prefix so unnamed array elements (SRAM/memory cells) take the O(1)
        // path instead of hashing into the sparse tail. Opt-in because the
        // prefix costs 4 B per signal — on c906 that is 35 M signals = 140 MB —
        // and for a design whose live array-NBA set is small, a compact hash
        // map can cache better than sparse probes into a 140 MB array. Measure
        // per design rather than assuming.
        if std::env::var_os("XEZIM_NBA_DENSE").is_some() {
            self.nba_fast_index.extend_dense(self.signal_table.len());
        }
        // JIT-redesign Stage 1: allocate `signal_inline_bits` BEFORE
        // `compile_edge_blocks` so the JIT module gets a non-empty
        // pointer in `set_inline_bits_storage`.  Otherwise Stage 2's
        // inline LoadSignal codegen falls back to the FFI path because
        // the storage is empty at JIT-compile time.
        // SoA planes are allocated for JIT runs (XEZIM_JIT), where the native
        // load paths read them, and on explicit XEZIM_INLINE_BITS=1. In the
        // default interpreter/two-state run the mirror only adds a 16-byte
        // store per signal write (560 MB on c906) that the two-state loads
        // do not need: keeping it off measured 2.6 % fewer cycles on the
        // c906 memcpy run. XEZIM_INLINE_BITS=0 forces it off everywhere.
        let want_mirror = match std::env::var("XEZIM_INLINE_BITS").ok().as_deref() {
            Some("0") => false,
            Some(_) => true,
            None => std::env::var("XEZIM_JIT")
                .map(|v| v != "0" && !v.is_empty())
                .unwrap_or(false),
        };
        if want_mirror && self.signal_inline_bits.is_empty() {
            let n = self.signal_table.len();
            self.signal_inline_bits = Vec::with_capacity(n);
            for v in &self.signal_table {
                let (val_bits, xz_bits) = v.raw_bits();
                self.signal_inline_bits.push([val_bits, xz_bits]);
            }
            // SoA read eligibility: inline-width, non-real, non-string
            // signals may be read back from `signal_inline_bits` (their
            // declared width/signedness reconstructs the Value exactly).
            self.soa_read_ok = (0..n)
                .map(|i| {
                    // width 0 = untracked class-handle placeholder — excluded.
                    (1..=64).contains(&self.signal_widths[i])
                        && !self.signal_real[i]
                        && !self.signal_is_string.get(i).copied().unwrap_or(false)
                })
                .collect();
            // JIT Stage 4 Tier C Path C1: allocate the side queue
            // alongside signal_inline_bits since both are gated by
            // the same env var.  1M entries × 16 bytes = 16 MB.
            // NEVER resize after this point — the JIT will bake a
            // pointer that must remain valid.
            const JIT_NBA_SIDE_QUEUE_CAP: usize = 1024 * 1024;
            self.jit_nba_side_queue = Vec::with_capacity(JIT_NBA_SIDE_QUEUE_CAP);
            // Fill to capacity with sentinel entries so `Vec::as_ptr`
            // returns a valid pointer into a fully-initialized buffer.
            self.jit_nba_side_queue.resize(
                JIT_NBA_SIDE_QUEUE_CAP,
                JitNbaSideEntry {
                    signal_id: 0,
                    _pad: 0,
                    val_bits: 0,
                },
            );
            self.jit_nba_side_len = 0;
        }
        self.collapse_identity_port_nets();
        mark_compile_phase("identity port-net collapse", &mut compile_phase_start);
        self.compile_edge_blocks();
        self.drop_compiled_edge_block_asts();
        // Intern each edge block's instance scope to a dense u32 so the `%m`
        // tracking in `exec_bytecode` compares integers, not strings. Blocks
        // sharing an instance share an id, which is what lets consecutive
        // fires in one scope skip the buffer copy entirely.
        {
            let mut ids = Vec::with_capacity(self.edge_blocks.len());
            let mut intern: HashMap<&str, u32> = HashMap::default();
            for b in self.edge_blocks.iter() {
                let next = intern.len() as u32;
                let id = *intern.entry(b.scope.as_str()).or_insert(next);
                ids.push(id);
            }
            self.edge_block_scope_id = ids;
        }
        mark_compile_phase("compile edge blocks", &mut compile_phase_start);
        // Apply SDF / specify delays BEFORE building comb entries — the fused-gate
        // fast path bails out on signals with nonzero delay, so the delay must be
        // visible at build time or cont_assigns to delayed signals will be fused
        // and bypass `schedule_delayed`.
        // Lazy-allocate sdf_delays only when there's an annotation or
        // specify_delays — saves 8 × num_signals B (288 MB on c910)
        // when neither is present (the common case).
        // `+nospecify`: module path delays from specify blocks are suppressed
        // (SDF annotation given explicitly via --sdf still applies — the CLI
        // warns when both are combined, since that pairing contradicts itself).
        // `+delay_mode_zero` forces ALL structural delays to 0 (a superset of
        // +nospecify that also drops SDF back-annotation) — fast functional GLS.
        // `+delay_mode_unit` collapses every nonzero structural delay to 1 tick.
        let dmode = delay_mode();
        if dmode == 1 && self.sdf_annotation.is_some() {
            eprintln!(
                "Warning: +delay_mode_zero forces all structural delays to 0 — SDF back-annotation is ignored."
            );
            self.sdf_annotation = None;
        }
        let use_specify = !nospecify() && dmode != 1 && !self.module.specify_delays.is_empty();
        let need_sdf = self.sdf_annotation.is_some() || use_specify;
        if need_sdf && self.sdf_delays.len() != self.signal_table.len() {
            self.sdf_delays.resize(self.signal_table.len(), 0);
            // Gate read masks assume no delays (`comb_entry_read_masks`).
            self.comb_dep_edges.clear();
        }
        // Signals whose delay came from SDF back-annotation. SDF is
        // authoritative: it REPLACES the specify path delay (SDF standard, and
        // it keeps this CLI path consistent with the runtime `$sdf_annotate`
        // path, which clobbers). Without this set a specify delay could `.max()`
        // upward over a smaller annotated value — the two paths would disagree.
        let mut sdf_annotated: std::collections::HashSet<usize> = std::collections::HashSet::new();
        if let Some(ref ann) = self.sdf_annotation {
            let mut count = 0;
            for (sig_name, &delay) in &ann.signal_delays {
                if let Some(&id) = self.signal_name_to_id.get(sig_name.as_str()) {
                    self.sdf_delays[id] = apply_delay_mode(delay);
                    sdf_annotated.insert(id);
                    count += 1;
                }
            }
            eprintln!("[SDF] annotated {} signals with delays", count);
        }
        if use_specify {
            for (sig_name, &delay) in &self.module.specify_delays {
                if let Some(&id) = self.signal_name_to_id.get(sig_name.as_str()) {
                    if sdf_annotated.contains(&id) {
                        continue; // SDF back-annotation overrides the specify delay
                    }
                    self.sdf_delays[id] = self.sdf_delays[id].max(apply_delay_mode(delay));
                }
            }
            // The per-transition, per-condition choice between them.
            self.build_module_paths(&sdf_annotated, apply_delay_mode);
        }
        // §28.11 gate fall delays: resolve names to ids once. Honours the same
        // delay-mode overrides as the rise delay (`+delay_mode_zero` etc.), so
        // a zero-delay GLS run stays zero-delay on both edges.
        if !self.module.gate_fall_delays.is_empty() && dmode != 1 {
            let pairs: Vec<(usize, u64)> = self
                .module
                .gate_fall_delays
                .iter()
                .filter_map(|(n, &d)| {
                    self.signal_name_to_id
                        .get(n.as_str())
                        .map(|&id| (id, apply_delay_mode(d)))
                })
                .collect();
            self.gate_fall_delay_by_id.extend(pairs);
        }
        mark_compile_phase("structural delay setup", &mut compile_phase_start);
        self.build_comb_entries();
        self.prepare_activity_profile();
        self.dump_comb_graph();
        #[cfg(feature = "jit")]
        self.jit_compile_comb_entries();
        #[cfg(feature = "jit")]
        self.aot_compile_proc_fsms();
        mark_compile_phase("build combinational entries", &mut compile_phase_start);
        if let Ok(w) = std::env::var("XEZIM_DUMP_ENTRY") {
            let probes: Vec<usize> = if w == "all" {
                (0..self.comb_entries.len()).collect()
            } else {
                let want: usize = w.parse().unwrap_or(0);
                vec![want, want + 1]
            };
            for probe in probes {
                if let Some(e) = self.comb_entries.get(probe) {
                    let rn: Vec<&str> = e
                        .cold
                        .read_signal_ids
                        .iter()
                        .filter_map(|&i| self.name_opt(i))
                        .collect();
                    let wn: Vec<&str> = e
                        .cold
                        .write_signal_ids
                        .iter()
                        .filter_map(|&i| self.name_opt(i))
                        .collect();
                    eprintln!(
                        "[ENTRY] #{} kind={} unresolved={} reads={:?} writes={:?}",
                        probe,
                        match &e.item {
                            CombItem::CompiledContAssign { .. } => "cca",
                            CombItem::ContAssign { .. } => "ca",
                            CombItem::DirectCopy { .. } => "dc",
                            CombItem::FastDirectCopy { .. } => "fdc",
                            CombItem::CompiledAlwaysBlock { .. } => "cab",
                            CombItem::AlwaysBlock { .. } => "ab",
                            _ => "other",
                        },
                        e.has_unresolved_reads,
                        rn,
                        wn
                    );
                    if let CombItem::ContAssign { lhs, rhs, .. } = &e.item {
                        eprintln!("[ENTRY]   lhs={lhs:?}");
                        eprintln!("[ENTRY]   rhs={rhs:?}");
                    }
                    // Full instruction stream for compiled entries — the tool
                    // for "the block fires but its writes don't land".
                    if let CombItem::CompiledAlwaysBlock { compiled, .. }
                    | CombItem::CompiledContAssign { compiled } = &e.item
                    {
                        for (k, insn) in compiled.instructions.iter().enumerate() {
                            eprintln!("[ENTRY]   {:>4}: {:?}", k, insn);
                        }
                    }
                }
            }
        }
        if std::env::var_os("XEZIM_SETTLE_LEVELS").is_some() || self.bsp_settle || self.bsp_shadow {
            self.report_comb_levels();
        }
        if self.activity_mon {
            self.activity_counts = vec![0u64; self.comb_entries.len()];
        }
        // Collect all edge-sensitive signal names for targeted prev
        // snapshots. We work off `resolved_sensitivities` (populated
        // during classify_always_blocks) and recover names from
        // `id_to_name` — the unresolved `Sensitivity` mirror is gone.
        // Collect edge sensitivity ids; need to materialise names into a
        // local Vec first so we can release the &self.id_to_name borrow
        // before mutating self.edge_signal_names.
        let mut edge_sens: Vec<(String, usize)> = Vec::new();
        for block in self.edge_blocks.iter() {
            for sens in &block.resolved_sensitivities {
                if sens.signal_id < self.id_to_name.named_id_bound() {
                    edge_sens.push((self.name_for_id(sens.signal_id).to_string(), sens.signal_id));
                }
            }
        }
        for (name, sig_id) in edge_sens {
            self.edge_signal_names.insert(name);
            self.edge_signal_ids.push(sig_id);
        }
        // Dropped-cell early warning: a flop clock/reset that is an IMPLICIT
        // (undeclared) net means the driver instance was dropped — almost
        // always an unresolved vendor cell (see the '-v resolution summary'
        // notes). The X clock/reset then silently corrupts everything
        // downstream (dead clocks, dividers stuck in reset), which shows up
        // hours later as a hang or an oracle mismatch. Say it up front, once,
        // with the offending nets, so the missing library is fixed instead of
        // debugged. (Same family as the UDP-terminal IMPLICIT-NET note, but
        // for RTL edge blocks and always-on rather than --primitive-verbose.)
        if !self.module.implicit_nets.is_empty() {
            let mut hit: HashMap<String, usize> = HashMap::default();
            for block in self.edge_blocks.iter() {
                for sens in &block.resolved_sensitivities {
                    let name = self.name_for_id(sens.signal_id);
                    if self.module.implicit_nets.contains(name) {
                        *hit.entry(name.to_string()).or_default() += 1;
                    }
                }
            }
            if !hit.is_empty() {
                let mut names: Vec<(&String, &usize)> = hit.iter().collect();
                names.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
                let shown: Vec<String> = names
                    .iter()
                    .take(8)
                    .map(|(n, c)| format!("'{}' ({} block(s))", n, c))
                    .collect();
                eprintln!(
                    "[xezim][warning] {} edge-block clock/reset input(s) are IMPLICIT \
                     (undeclared) nets — the driving instance was likely DROPPED as an \
                     unresolved cell (see 'unresolved definition' notes above). These \
                     clocks/resets stay X and everything they clock is dead: {}{}",
                    hit.len(),
                    shown.join(", "),
                    if names.len() > 8 {
                        format!(", … and {} more", names.len() - 8)
                    } else {
                        String::new()
                    }
                );
            }
        }
        // Also collect from event waiters that are registered at time 0
        self.edge_signal_ids.sort_unstable();
        self.edge_signal_ids.dedup();
        // Reverse map: signal id -> its position in the (now sorted, stable)
        // edge_signal_ids, or -1. Bound it by the highest edge signal rather
        // than the full signal table; sparse memories otherwise make this
        // auxiliary lookup much larger than its useful working set. Used by
        // the dirty-driven edge
        // detect (opt-in) AND by the always-on edge-exec write recording
        // (blocking writes made DURING edge-block execution must trigger a
        // re-detect pass — see drain_edge_exec_rescan).
        let edge_lookup_len = self
            .edge_signal_ids
            .last()
            .copied()
            .map_or(0, |sid| sid.saturating_add(1));
        self.sig_to_edge_pos = vec![-1i32; edge_lookup_len];
        self.quiet_valid = false;
        self.gate_lane_valid = false;
        for (pos, &sid) in self.edge_signal_ids.iter().enumerate() {
            if sid < self.sig_to_edge_pos.len() {
                self.sig_to_edge_pos[sid] = pos as i32;
            }
        }
        self.edge_exec_seen = vec![false; self.edge_signal_ids.len()];
        if self.dirty_edge || self.dirty_edge_shadow {
            self.edge_pos_seen = vec![false; self.edge_signal_ids.len()];
        }
        // Cache each clock generator's position inside edge_signal_ids so
        // fire_clock_generators can populate toggled_clock_positions in O(1)
        // (binary search would be O(log N) per call — small N, but this path
        // runs once per iter on busy designs).
        for cg in &mut self.clock_generators {
            cg.edge_signal_position = self
                .edge_signal_ids
                .binary_search(&cg.signal_id)
                .unwrap_or(usize::MAX);
        }
        self.build_clock_tree();
        // Build the non-clock-edge-signal lookup table.  Dense Vec<bool>
        // indexed by signal_id and bounded by the highest edge signal so the
        // hot per-write check stays a single bounds-check + load. Tags
        // every edge-sensitive signal except the clocks (those toggle every
        // iter and are accounted for separately via
        // `toggled_clock_positions`).
        let total_signals = self.signal_table.len();
        self.is_edge_signal_non_clock = vec![false; edge_lookup_len];
        let mut clock_sids: HashSet<usize> = HashSet::default();
        for cg in &self.clock_generators {
            clock_sids.insert(cg.signal_id);
        }
        for &sid in &self.edge_signal_ids {
            if sid < self.is_edge_signal_non_clock.len() && !clock_sids.contains(&sid) {
                self.is_edge_signal_non_clock[sid] = true;
            }
        }
        self.build_edge_sig_groups();
        self.prepare_multicore();
        // C1 hot-signal-arena diagnostic (HOT-ARENA-NOTES.md falsification
        // check #2): compute the hot working set + how many hot signals
        // are array members.  Decides whether the arena remap can skip
        // arrays (simpler) or must handle array-atomicity.  Gated by
        // XEZIM_HOT_STATS=1.
        if std::env::var("XEZIM_HOT_STATS").is_ok() {
            let mut is_hot = vec![false; total_signals];
            let mut hot_count = 0usize;
            for &sid in &self.edge_signal_ids {
                if sid < total_signals && !is_hot[sid] {
                    is_hot[sid] = true;
                    hot_count += 1;
                }
            }
            for &sid in &clock_sids {
                if sid < total_signals && !is_hot[sid] {
                    is_hot[sid] = true;
                    hot_count += 1;
                }
            }
            for entry in &self.comb_entries {
                for &sid in entry
                    .cold
                    .read_signal_ids
                    .iter()
                    .chain(entry.cold.write_signal_ids.iter())
                {
                    if sid < total_signals && !is_hot[sid] {
                        is_hot[sid] = true;
                        hot_count += 1;
                    }
                }
            }
            // Build a sid → is-array-member bitmap by iterating arrays
            // ONCE (marking each array's sid range), then count hot
            // signals that fall inside an array.  O(total_signals +
            // sum_of_array_lengths) instead of O(signals × arrays).
            let mut in_array_bm = vec![false; total_signals];
            for &(first, lo, hi) in self.array_first_id.values() {
                let len = (hi - lo + 1).max(0) as usize;
                for s in first..(first + len).min(total_signals) {
                    in_array_bm[s] = true;
                }
            }
            let mut hot_in_array = 0usize;
            let mut hot_standalone = 0usize;
            for sid in 0..total_signals {
                if !is_hot[sid] {
                    continue;
                }
                if in_array_bm[sid] {
                    hot_in_array += 1;
                } else {
                    hot_standalone += 1;
                }
            }
            let named_count = self.id_to_name.legacy_len();
            eprintln!(
                "[HOT_STATS] total={} hot={} ({:.2}%) hot_in_array={} hot_standalone={} \
                 named_count={} arrays={}",
                total_signals,
                hot_count,
                100.0 * hot_count as f64 / total_signals.max(1) as f64,
                hot_in_array,
                hot_standalone,
                named_count,
                self.array_first_id.len(),
            );
        }
        // JIT-redesign Stage 1: allocate `signal_inline_bits` parallel
        // (signal_inline_bits is now allocated earlier, before
        // compile_edge_blocks, so the JIT can bake a stable pointer.
        // See the earlier allocation site for context.)
        // Build reverse index parallel to `edge_signal_ids` (position-indexed,
        // not signal_id-indexed). Dense signal_id indexing would allocate one
        // empty Vec per signal — on c910-scale designs with ~585K signals and
        // only a few thousand edge-sensitive ones, the dense layout wasted
        // ~14 MB on empty Vec stubs. Walk blocks once into a temp HashMap,
        // then materialize in edge_signal_ids order.
        let mut by_sid: HashMap<usize, EdgeFanout> = HashMap::default();
        for (block_idx, block) in self.edge_blocks.iter().enumerate() {
            for sid in &block.resolved_sensitivities {
                let fanout = by_sid.entry(sid.signal_id).or_default();
                match sid.edge {
                    EdgeKind::Posedge => fanout.posedge.push(block_idx),
                    EdgeKind::Negedge => fanout.negedge.push(block_idx),
                    EdgeKind::AnyEdge => fanout.anyedge.push(block_idx),
                    EdgeKind::LsbEdge => fanout.lsbedge.push(block_idx),
                }
            }
        }
        self.edge_blocks_by_sig = self
            .edge_signal_ids
            .iter()
            .map(|sid| by_sid.remove(sid).unwrap_or_default())
            .collect();
        // Record which edge blocks carry an `iff` guard so check_edges can
        // evaluate it (LRM §9.4.2.3) without scanning every block each tick.
        self.edge_blocks_with_iff = self
            .edge_blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.resolved_sensitivities.iter().any(|s| s.iff.is_some()))
            .map(|(i, _)| i)
            .collect();
        mark_compile_phase(
            "build edge and dependency indexes",
            &mut compile_phase_start,
        );
        // §16.5.1: a sampled-value function's default sampled values are the
        // values before time 0 — declaration initializers, else type
        // defaults — so take them before the time-0 settle drives any net or
        // continuously assigned variable. The procedural watches read theirs
        // when they register.
        self.capture_sampled_defaults();
        // §16.9.3: pre-register sampled-value watches BEFORE the initial
        // blocks are consumed into scheduled processes, so clock-edge
        // history accumulates from the first edge.
        self.register_sampled_watches();
        if !self.sampled_defaults.is_empty() || !self.sampled_watches.is_empty() {
            self.mark_deferred_inits();
        }
        // IEEE 1800: at time 0, always_comb blocks execute unconditionally.
        // always @* blocks do NOT execute at time 0 unless inputs change.
        // Only seed dirty IDs that have combinational dependents. Large
        // unpacked memory arrays can add tens of millions of signal IDs, but
        // most cells have no continuous/always_comb fanout, so marking every
        // ID dirty only burns time and memory bandwidth before settling.
        self.dirty_list.clear();
        let dep_len = self.comb_dep_offsets.len().saturating_sub(1);
        for i in 0..dep_len {
            if self.comb_dep_offsets[i] != self.comb_dep_offsets[i + 1] {
                self.dirty_signals[i] = true;
                self.dirty_list.push(i);
            }
        }
        self.dirty_any = !self.dirty_list.is_empty() || !self.comb_time0_idx.is_empty();
        let t_settle0 = std::time::Instant::now();
        let entries_before = self.entry_evals;
        let iters_before = self.settle_iters;
        let dc_ns_before = self.prof_settle_dc_ns;
        let dc_count_before = self.prof_settle_dc_count;
        let ca_ns_before = self.prof_settle_ca_ns;
        let ca_count_before = self.prof_settle_ca_count;
        let ab_ns_before = self.prof_settle_ab_ns;
        let ab_count_before = self.prof_settle_ab_count;
        // Baseline edge snapshot BEFORE the time-0 settle: prev values = the
        // PRE-settle actual state (x for uninitialized regs, z for undriven
        // nets, declared initializers already applied). A signal the settle
        // never touches compares equal (x==x — no phantom edge, which is what
        // the old zero-initialized prev arrays got wrong), while a signal a
        // cont-assign RESOLVES during the settle (`wire one = 1'b1`, X→1) is
        // a genuine §10.3 update the first edge scan must deliver: the C910
        // IDU's `always @(dep_bit)` decode blocks are clocked by exactly such
        // constant nets, and the reference fires them once at t0. Snapshotting
        // AFTER the settle erased that change and the blocks never ran.
        self.snapshot_edge_signals_full();
        self.in_pre_process_settle = true;
        self.settle_combinatorial();
        self.fire_time0_input_port_blocks();
        self.in_pre_process_settle = false;
        self.time0_settled = true;
        mark_compile_phase("time-0 settle", &mut compile_phase_start);
        let dt = t_settle0.elapsed();
        if dt.as_millis() > 100 {
            chatter!(
                "[PHASE] time-0 settle: {:.1}ms ({} entry_evals, {} settle_iters, {} comb_entries, {} signals)",
                dt.as_secs_f64() * 1000.0,
                self.entry_evals - entries_before,
                self.settle_iters - iters_before,
                self.comb_entries.len(),
                self.signal_table.len()
            );
            chatter!(
                "[PHASE] time-0 settle breakdown: DC {:.1}ms/{} ({:.1}µs), CA {:.1}ms/{} ({:.1}µs), AB {:.1}ms/{} ({:.1}µs)",
                (self.prof_settle_dc_ns - dc_ns_before) as f64 / 1e6,
                self.prof_settle_dc_count - dc_count_before,
                if self.prof_settle_dc_count > dc_count_before {
                    (self.prof_settle_dc_ns - dc_ns_before) as f64
                        / (self.prof_settle_dc_count - dc_count_before) as f64
                        / 1e3
                } else {
                    0.0
                },
                (self.prof_settle_ca_ns - ca_ns_before) as f64 / 1e6,
                self.prof_settle_ca_count - ca_count_before,
                if self.prof_settle_ca_count > ca_count_before {
                    (self.prof_settle_ca_ns - ca_ns_before) as f64
                        / (self.prof_settle_ca_count - ca_count_before) as f64
                        / 1e3
                } else {
                    0.0
                },
                (self.prof_settle_ab_ns - ab_ns_before) as f64 / 1e6,
                self.prof_settle_ab_count - ab_count_before,
                if self.prof_settle_ab_count > ab_count_before {
                    (self.prof_settle_ab_ns - ab_ns_before) as f64
                        / (self.prof_settle_ab_count - ab_count_before) as f64
                        / 1e3
                } else {
                    0.0
                }
            );
        }
        // Consume initial_blocks: after scheduling they're no longer needed,
        // so take ownership (drops the Vec when this scope ends) instead of
        // cloning — saves significant memory on large testbenches with
        // memory-init initial blocks holding tens of thousands of statements.
        //
        // #7 lazy-prefix path: also drain `pending_initial` one-at-a-time.
        // A pending block is materialized just before scheduling and dropped
        // after, so peak memory is at most one materialized InitialBlock.
        // §6.18: reject assigning a non-class value (an enum member or integer
        // literal) to a class-handle variable. The reference elaborator flags
        // this at compile time; surface a compile error so simulation never
        // runs. Deliberately conservative and side-effect free (it only reads
        // declared types and enum-member tables — it must NOT call
        // resolve_hier_name, which would pre-populate the shared per-AST-node
        // name cache with process-scope-free resolutions that later corrupt
        // name resolution at simulation time).
        self.check_class_handle_assignment_type();
        if !self.compile_errors.is_empty() {
            return;
        }
        let pending_initial = std::mem::take(&mut self.module.pending_initial);
        let initial_blocks = std::mem::take(&mut self.module.initial_blocks);
        if std::env::var("XEZIM_TRACE_INIT").ok().as_deref() == Some("1") {
            eprintln!(
                "[xezim] {} initial blocks to schedule ({} from pending lazy-prefix)",
                initial_blocks.len() + pending_initial.len(),
                pending_initial.len()
            );
        }
        // Streaming chain: pending materializes one at a time, dropped after
        // the loop body consumes its `stmt`. No intermediate Vec staging.
        // §6.21/§9.3.4: an instance's process (the lazy-prefix ones) keeps
        // its locals' bare names while its module items carry the instance
        // path, so a local there was the same variable in every instance of
        // the module (or any process declaring that name), and one named
        // like a TOP item was that item: rename those, its top-level block
        // included.
        let mut shadow_n = 0usize;
        let claim_set: HashSet<String> = if pending_initial.is_empty() {
            HashSet::default()
        } else {
            // A local claimed elsewhere: by a top item, or by another
            // process (another instance of the module included).
            let mut counts: HashMap<&str, usize> = HashMap::default();
            for st in pending_initial
                .iter()
                .map(|p| &*p.source)
                .chain(initial_blocks.iter().map(|b| &b.stmt))
            {
                Self::for_each_decl_name(st, &mut |n, _| *counts.entry(n).or_insert(0) += 1);
            }
            let m = &self.module;
            counts
                .into_iter()
                .filter(|&(n, c)| {
                    c >= 2
                        || self.signal_name_to_id.contains_key(n)
                        || m.arrays.contains_key(n)
                        || m.associative_arrays.contains_key(n)
                        || m.dynamic_arrays.contains(n)
                        || m.queue_vars.contains(n)
                })
                .map(|(n, _)| n.to_string())
                .collect()
        };
        let claims = |nm: &str| claim_set.contains(nm);
        let mut rename_pending = |p: super::elaborate::PendingInitial| {
            let mut ib = p.materialize();
            let scope = ib.scope.clone();
            Self::rename_nested_shadowed(
                &mut ib.stmt,
                &claims,
                &mut Vec::new(),
                &mut shadow_n,
                "p",
                1,
                &scope,
                &mut Vec::new(),
            );
            ib
        };
        // §6.21: the static variables of every process block are created
        // and initialized before any process starts. An instance's initial
        // block is renamed for its instance first, so materialize the pending
        // ones up front when one of them declares such a variable (otherwise
        // they stream as before).
        let hoist_pending = pending_initial
            .iter()
            .any(|p| Self::stmt_has_static_decl(&p.source));
        let mut early: Vec<super::elaborate::InitialBlock> = Vec::new();
        let pending_initial = if hoist_pending {
            early = pending_initial
                .into_iter()
                .map(&mut rename_pending)
                .collect();
            Vec::new()
        } else {
            pending_initial
        };
        for ib in early.iter().chain(initial_blocks.iter()) {
            self.hoist_static_decls(&ib.stmt, &ib.scope);
        }
        // Static/package-global initializers must run before any `initial`
        // block (and the lazy-prefix pending ones) so package globals are
        // populated before user code reads them. Schedule them first so they
        // receive the lowest pids (the time-0 queue runs lower pids first).
        for ib in std::mem::take(&mut self.module.static_init_blocks) {
            let span = ib.stmt.span;
            let stmts = match ib.stmt.kind {
                StatementKind::SeqBlock { stmts, .. } => stmts,
                other => vec![Statement::new(other, span)],
            };
            let pid = self.next_pid;
            self.next_pid += 1;
            // §6.21: an instance-scoped static initializer (deferred by
            // `defer_static_syscall_inits`) needs the same name-resolution
            // hint initial blocks get, so bare names, module-level function
            // calls and `%m` resolve in the DECLARING instance's scope.
            if !ib.scope.is_empty() {
                self.process_scope_hint.insert(pid, ib.scope);
            }
            self.process_origin
                .insert(pid, (span, "static initializer"));
            self.event_queue.schedule(0, pid, stmts.into());
        }
        // §16.5/§16.14: declarative concurrent assertions exist before
        // Active processes run, including an initial clock's time-zero edge.
        // Extract only synthetic assertion blocks, retaining the existing
        // storage and streaming behavior of ordinary pending processes.
        // Procedural sites still capture locals when their procedure executes.
        let declarative = |st: &Statement| matches!(&st.kind, StatementKind::Assertion(a) if a.is_property && !a.procedural);
        let mut initial_blocks = initial_blocks;
        let mut pending_initial = pending_initial;
        let saved_pid = self.current_pid;
        let saved_scope = self.current_scope.clone();
        let saved_hint = self.name_resolve_hint.borrow_mut().take();
        let saved_activation = self.activation_scope.borrow_mut().take();
        // §21.2.1.7: a declarative assertion is not a comb/edge block, so the
        // scope the time-0 settle last recorded must not leak into its
        // registration (and so into the site's `%m` chain / action), as a
        // process run clears it (see `run_process`).
        let saved_block_scope = std::mem::take(&mut self.m_block_scope);
        let saved_block_scope_id = std::mem::replace(&mut self.m_block_scope_id, u32::MAX);
        let saved_block_active = std::mem::replace(&mut self.m_block_active, false);
        for ib in initial_blocks
            .extract_if(.., |ib| declarative(&ib.stmt))
            .chain(early.extract_if(.., |ib| declarative(&ib.stmt)))
            .chain(
                pending_initial
                    .extract_if(.., |p| declarative(&p.source))
                    .map(&mut rename_pending),
            )
        {
            self.current_pid = self.next_pid;
            self.next_pid += 1;
            self.current_scope.clone_from(&ib.scope);
            self.process_scope_hint
                .insert(self.current_pid, ib.scope.clone());
            *self.name_resolve_hint.borrow_mut() = Some(ib.scope.clone());
            *self.activation_scope.borrow_mut() = Some(ib.scope);
            self.exec_statement(&ib.stmt);
        }
        self.current_pid = saved_pid;
        self.current_scope = saved_scope;
        *self.name_resolve_hint.borrow_mut() = saved_hint;
        *self.activation_scope.borrow_mut() = saved_activation;
        self.m_block_scope = saved_block_scope;
        self.m_block_scope_id = saved_block_scope_id;
        self.m_block_active = saved_block_active;
        for ib in early
            .into_iter()
            .chain(pending_initial.into_iter().map(&mut rename_pending))
            .chain(initial_blocks)
        {
            let ib = self.cov_initial(ib);
            let scope = ib.scope;
            let span = ib.stmt.span;
            let block_label = match &ib.stmt.kind {
                StatementKind::SeqBlock { name, .. } => name.as_ref().map(|n| n.name.clone()),
                _ => None,
            };
            let stmts = match ib.stmt.kind {
                StatementKind::SeqBlock { stmts, .. } => stmts,
                other => vec![Statement::new(other, ib.stmt.span)],
            };
            // Clock-generator fast path:
            //     initial begin VAR = CONST; forever #d VAR = ~VAR; end
            // Otherwise each half-period would schedule a new process
            // (~2.5ms per toggle on c910). Detect this pattern, apply
            // the seed assignment immediately, and register a ClockGen
            // so `fire_clock_generators` can toggle the signal O(1).
            // The detection below EVALUATES the block's seed RHS, and that
            // eval memoizes each bare name's resolution on its AST node. It
            // must therefore run under THIS block's scope — the hint still
            // holds whatever the previous block (or any earlier compile-time
            // eval) left, and a bare wildcard-imported constant resolved once
            // under a sibling's scope stays wrong for the whole run: `user`'s
            // enum member `C` permanently read `shadower`'s local `int C`.
            *self.name_resolve_hint.borrow_mut() = (!scope.is_empty()).then(|| scope.clone());
            if let Some(cg) = self.try_extract_initial_clock_gen(&stmts) {
                self.clock_gen_src
                    .push((self.clock_generators.len(), scope.clone(), span.start));
                self.clock_generators.push(cg);
                if !self.sva_sites.is_empty() {
                    // §16.5.1: keep the seed in Active so the slot-entry
                    // Preponed snapshot precedes it, just like an ordinary
                    // blocking assignment. The periodic toggles stay fast.
                    let pid = self.next_pid;
                    self.next_pid += 1;
                    if !scope.is_empty() {
                        self.process_scope_hint.insert(pid, scope);
                    }
                    self.process_origin.insert(pid, (span, "initial block"));
                    self.event_queue
                        .schedule(0, pid, vec![stmts[0].clone()].into());
                }
                continue;
            }
            // Roadmap 12 (opt-in XEZIM_PROC_FSM=1): a compilable initial body
            // with waits becomes a process FSM. The cheap timing-control
            // count runs first so memory-init megablocks skip without the
            // SeqBlock rebuild.
            {
                // 1 = every compilable initial body; 0 = none; unset = the
                // plain-stimulus shape below (a testbench's clocked stimulus
                // loop ran 8.6x slower on the AST path than as an FSM).
                let fsm_mode = self.proc_fsm_mode;
                let timing = stmts.iter().map(Self::count_timing_controls).sum::<usize>();
                let admit = timing >= 1
                    && match fsm_mode {
                        1 => true,
                        0 => false,
                        _ => {
                            // Clock-driven only: a body with at least one `@`
                            // wait is the hot stimulus loop; delay-only
                            // sequences (reset scripts, UDP/stall probes,
                            // ordering tests) stay on the AST path, whose
                            // per-statement waiter and UDP evaluation order
                            // their pinned traces rely on. A labeled block is
                            // a hierarchical scope (`disable lbl`, `lbl.v`)
                            // the FSM does not model.
                            std::env::var_os("XEZIM_VALUE_TRACE").is_none()
                                && block_label.is_none()
                                && stmts.iter().any(Self::stmt_contains_event_control)
                                && stmts.iter().all(|st| self.stmt_is_plain_stimulus(st))
                        }
                    };
                if admit {
                    let wrapped = Statement::new(
                        StatementKind::SeqBlock {
                            name: None,
                            stmts: stmts.clone(),
                        },
                        span,
                    );
                    if self.try_register_proc_fsm(&wrapped, &scope, false, "initial block") {
                        continue;
                    }
                }
            }
            let pid = self.next_pid;
            self.next_pid += 1;
            if !stmts.iter().any(|st| self.stmt_is_blocking(st)) {
                self.t0_timing_free.insert(pid);
            }
            if !scope.is_empty() {
                self.process_scope_hint.insert(pid, scope);
            }
            self.process_origin.insert(pid, (span, "initial block"));
            if let Some(l) = block_label {
                // The outermost `begin/end` is flattened away below, so keep
                // the label for `%m` here — see `process_m_label`.
                self.process_m_label.insert(pid, l.clone());
                self.disable_labels.insert(l, pid);
            }
            self.event_queue.schedule(0, pid, stmts.into());
        }
        self.order_time0_processes();
        // LRM §24: `program` initial blocks execute in the reactive region.
        // Stash each program-initial's statements into `pending_reactive` so
        // `drain_reactive_region` picks them up at the first time-0 reactive
        // pass. Synchronous drain matches the basic-coverage scope — any
        // deeper sub-events scheduled by these statements re-enter the
        // normal active queue and behave correctly.
        // §24.3: each initial block is a process of its own, so one that
        // waits does not hold up the next.
        for ib in std::mem::take(&mut self.module.program_initial_blocks) {
            let stmts = match ib.stmt.kind {
                StatementKind::SeqBlock { stmts, .. } => stmts,
                other => vec![Statement::new(other, ib.stmt.span)],
            };
            self.pending_reactive.push((stmts, ib.scope));
        }
        // The reactive region of time 0 runs at the end of the time-0 slot;
        // make sure that slot exists even when no module process starts
        // at time 0 (a design whose only module activity is a clock).
        if !self.pending_reactive.is_empty() {
            let pid = self.next_pid;
            self.next_pid += 1;
            self.event_queue
                .schedule(0, pid, Vec::<Statement>::new().into());
        }
        // Open the XTrace dump (if requested) AFTER initial-block scheduling
        // and time-0 settle so the dictionary captures all elaborated signals
        // and the snapshot reflects post-settle initial values.
        if self.xtrace_file.is_some() {
            self.xtrace_start_dump();
        }
        if self.fst_file.is_some() {
            self.fst_start_dump();
        }
        self.auto_partition_by_clock();
        self.auto_partition_by_scope();
        mark_compile_phase("schedule initial blocks", &mut compile_phase_start);
        self.init_value_trace();
        self.rebuild_signal_commit_plans();
        self.compiled = true;
    }
}
