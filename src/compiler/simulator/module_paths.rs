//! IEEE 1800-2017 §30.4/§30.5 module path delays.
//!
//! Each path-delayed net carries its module paths
//! (`ElaboratedModule::module_paths`): twelve transition delays, the input
//! terminals, and an optional `if` condition or `ifnone`. A driver of such a
//! net reaches `schedule_delayed_with_delay`, which picks the delay here:
//!
//! * a path is enabled when it is unconditional, its condition is true or
//!   x/z (§30.4.4.1), or — `ifnone` — no conditional path from one of its
//!   inputs is enabled; with no enabled path the net changes in the same
//!   time step;
//! * among the enabled paths, the one whose input changed most recently
//!   wins; inputs that changed together take the smallest delay (§30.4.4);
//! * each bit uses the delay of its own transition (§30.5.1 Table 30-3),
//!   so the bits of a bus that rise and fall together settle separately.
//!
//! Input change times come from `TcWatch` comb entries (the timing-check
//! watches), which run after the combinational logic of a settle — a
//! driver may schedule before the watch of the input that triggered it has
//! run, so an input whose value still differs from its watch's last-seen
//! value counts as changing now.
//!
//! PATHPULSE$ limits are unmodeled. The default inertial behavior rejects
//! pulses narrower than the path delay.
use super::*;

/// The design's module paths, resolved to signal ids.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ModulePaths {
    /// Path table index by destination signal id.
    dst: HashMap<usize, usize>,
    tables: Vec<Vec<PathRt>>,
    srcs: Vec<PathSrc>,
    /// The condition evaluator, called through a pointer. A direct call from
    /// the delay-scheduling path into the expression evaluator changed how
    /// the optimizer inlined the hot settle loop — about 1% more
    /// instructions on a design with no specify paths at all.
    #[serde(skip, default = "cond_true_default")]
    cond_true: fn(&mut Simulator, &Option<Arc<Expression>>) -> bool,
}

fn cond_true_default() -> fn(&mut Simulator, &Option<Arc<Expression>>) -> bool {
    Simulator::timing_cond_true
}

/// A module path into one net.
#[derive(serde::Serialize, serde::Deserialize)]
struct PathRt {
    delays: [u64; 12],
    /// Indices into `ModulePaths::srcs`.
    srcs: Vec<u32>,
    cond: Option<Arc<Expression>>,
    cond_text: Option<String>,
    edge: Option<String>,
    ifnone: bool,
}

/// A path input terminal: its signal, the watch stamping its changes, and
/// the time of its last change.
#[derive(serde::Serialize, serde::Deserialize)]
struct PathSrc {
    sig: usize,
    watch: usize,
    last: u64,
    transition: u16,
}

impl ModulePaths {
    pub(super) fn table_of(&self, id: usize) -> Option<usize> {
        self.dst.get(&id).copied()
    }

    pub(super) fn is_path_net(&self, id: usize) -> bool {
        self.dst.contains_key(&id)
    }

    /// Input `slot` changed at `now`.
    pub(super) fn stamp(&mut self, slot: u32, now: u64, transition: u16) {
        let src = &mut self.srcs[slot as usize];
        src.last = now;
        src.transition = transition;
    }

    /// VPI `vpiModPath` objects: every path-delayed net with the number of
    /// paths leading into it.
    pub(super) fn vpi_nets(&self) -> Vec<(usize, usize)> {
        self.dst
            .iter()
            .map(|(&id, &k)| (id, self.tables[k].len()))
            .collect()
    }

    /// The twelve transition delays of path `i` into net `id`.
    pub(super) fn vpi_delays(&self, id: usize, i: usize) -> Option<[u64; 12]> {
        let k = self.table_of(id)?;
        self.tables[k].get(i).map(|p| p.delays)
    }

    /// `vpi_put_delays` on a `vpiModPath`: the delays the next change of the
    /// net schedules with.
    pub(super) fn vpi_set_delays(&mut self, id: usize, i: usize, d: [u64; 12]) -> bool {
        let Some(k) = self.table_of(id) else {
            return false;
        };
        match self.tables[k].get_mut(i) {
            Some(p) => {
                p.delays = d;
                true
            }
            None => false,
        }
    }
}

/// §30.4.4.1: a condition that is x or z enables its path, so the path is
/// on unless the condition is a known 0 — `(cond) !== 0`, which the
/// timing-check condition evaluator (enabled only for a known nonzero)
/// then decides.
fn path_enable_expr(cond: Expression) -> Arc<Expression> {
    let span = cond.span;
    let zero = Expression::new(
        ExprKind::Number(NumberLiteral::Integer {
            size: None,
            signed: true,
            base: NumberBase::Decimal,
            value: "0".to_string(),
            cached_val: Default::default(),
        }),
        span,
    );
    Arc::new(Expression::new(
        ExprKind::Binary {
            op: BinaryOp::CaseNeq,
            left: Box::new(cond),
            right: Box::new(zero),
        },
        span,
    ))
}

fn normalize_sdf_condition(expr: &str) -> String {
    let term: String = expr.chars().filter(|c| !c.is_whitespace()).collect();
    term.replace("1'b", "")
        .replace("1'B", "")
        .trim_matches(['(', ')'])
        .to_owned()
}

fn annotate_path_delays(current: [u64; 12], arc: &crate::compiler::sdf::PinDelay) -> [u64; 12] {
    let values = &arc.values;
    let get = |i: usize| values.get(i).copied().flatten();
    let min = |a: Option<i64>, b: Option<i64>| Some(a?.min(b?));
    let max = |a: Option<i64>, b: Option<i64>| Some(a?.max(b?));
    let (a, b, c, d, e, f) = match values.len() {
        1 => (get(0), get(0), get(0), get(0), get(0), get(0)),
        2 => (get(0), get(1), get(0), get(0), get(1), get(1)),
        3 => (get(0), get(1), get(2), get(0), get(2), get(1)),
        _ => (get(0), get(1), get(2), get(3), get(4), get(5)),
    };
    let patch = if values.len() == 12 {
        std::array::from_fn(get)
    } else {
        [
            a,
            b,
            c,
            d,
            e,
            f,
            min(a, c),
            max(a, d),
            min(b, e),
            max(b, f),
            max(c, e),
            min(d, f),
        ]
    };
    std::array::from_fn(|i| {
        apply_delay_mode(match patch[i] {
            None => current[i],
            Some(d) if arc.increment => current[i].saturating_add_signed(d),
            Some(d) => d.max(0) as u64,
        })
    })
}

impl Simulator {
    /// Resolve specify module paths to their runtime nets and watches.
    pub(super) fn build_module_paths(&mut self, map_delay: impl Fn(u64) -> u64) {
        let paths = std::mem::take(&mut self.module.module_paths);
        let mut names: Vec<&String> = paths.keys().collect();
        names.sort();
        let mut mp = ModulePaths {
            dst: HashMap::default(),
            tables: Vec::new(),
            srcs: Vec::new(),
            cond_true: Simulator::timing_cond_true,
        };
        // `usize` values: a new `HashMap<usize, u32>::insert` caller changed
        // how that map's hot users in the NBA path were inlined.
        let mut src_of: HashMap<usize, usize> = HashMap::default();
        for name in names {
            let Some(&id) = self.signal_name_to_id.get(name.as_str()) else {
                continue;
            };
            let mut rts = Vec::new();
            for p in &paths[name] {
                let mut srcs = Vec::new();
                for s in &p.srcs {
                    let Some(&sig) = self.signal_name_to_id.get(s.as_str()) else {
                        continue;
                    };
                    let slot = match src_of.get(&sig) {
                        Some(&k) => k as u32,
                        None => {
                            let k = mp.srcs.len() as u32;
                            let watch = self.path_source_watch(sig, k);
                            mp.srcs.push(PathSrc {
                                sig,
                                watch,
                                last: 0,
                                transition: 0,
                            });
                            src_of.insert(sig, k as usize);
                            k
                        }
                    };
                    srcs.push(slot);
                }
                rts.push(PathRt {
                    delays: p.delays.map(&map_delay),
                    srcs,
                    cond: p.cond.clone().map(path_enable_expr),
                    cond_text: p.cond_text.clone(),
                    edge: p.edge.clone(),
                    ifnone: p.ifnone,
                });
            }
            mp.dst.insert(id, mp.tables.len());
            mp.tables.push(rts);
        }
        if !mp.tables.is_empty() {
            self.module_paths = Some(Box::new(mp));
        }
    }

    /// Resolve SDF cell paths below the annotation scope (§32.9).
    pub(crate) fn resolve_sdf(
        &self,
        mut sdf: crate::compiler::sdf::SdfFile,
        select: crate::compiler::sdf::DelaySelect,
        scope: Option<&str>,
    ) -> crate::compiler::sdf::SdfAnnotation {
        let base = scope
            .map(|s| self.instance_relative_scope(s))
            .unwrap_or_else(|| {
                if self.root_is_multi_top() && self.instance_by_path(&sdf.design).is_some() {
                    sdf.design.clone()
                } else {
                    String::new()
                }
            });
        let prefix = if base.is_empty() {
            String::new()
        } else {
            format!("{base}.")
        };
        let mut unmatched = 0;
        sdf.cells = sdf
            .cells
            .into_iter()
            .flat_map(|mut cell| {
                if cell.instance == "*" {
                    let paths: Vec<_> = self
                        .module
                        .instances
                        .iter()
                        .filter(|i| i.def_name == cell.cell_type && i.path.starts_with(&prefix))
                        .map(|i| i.path.as_str())
                        .chain(
                            self.instance_by_path(&base)
                                .map_or(
                                    base.is_empty() && self.module.name == cell.cell_type,
                                    |i| i.def_name == cell.cell_type,
                                )
                                .then_some(base.as_str()),
                        )
                        .collect();
                    unmatched += usize::from(paths.is_empty());
                    paths
                        .into_iter()
                        .map(|path| {
                            let mut entry = cell.clone();
                            entry.instance = path.to_owned();
                            entry
                        })
                        .collect::<Vec<_>>()
                } else {
                    let path = if cell.instance.is_empty() {
                        base.clone()
                    } else if scope.is_none()
                        && self.root_is_multi_top()
                        && self.instance_by_path(&cell.instance).is_some()
                    {
                        cell.instance.clone()
                    } else {
                        format!("{prefix}{}", self.instance_relative_scope(&cell.instance))
                    };
                    let matches = if path.is_empty() {
                        cell.cell_type == self.module.name
                    } else {
                        self.instance_by_path(&path)
                            .is_some_and(|inst| inst.def_name == cell.cell_type)
                    };
                    if matches {
                        cell.instance = path;
                        vec![cell]
                    } else {
                        unmatched += 1;
                        Vec::new()
                    }
                }
            })
            .collect();
        if unmatched != 0 {
            eprintln!("[SDF] {unmatched} CELL entries matched no instance");
        }
        crate::compiler::sdf::annotate_sdf(&sdf, self.tick_s, select)
    }

    /// Apply annotations to individual specify arcs. A missing selected value
    /// leaves that transition alone. Increment entries add to the current path
    /// delay so successive annotation calls compose in file order.
    pub(super) fn apply_sdf_path_delays(&mut self) {
        let Some(ann) = &self.sdf_annotation else {
            return;
        };
        let mut matched = 0;
        let mut unmatched = 0;
        for (name, arcs) in &ann.pin_delays {
            let Some(&id) = self.signal_name_to_id.get(name.as_str()) else {
                unmatched += arcs.len();
                continue;
            };
            if let Some(mp) = &mut self.module_paths
                && let Some(table) = mp.table_of(id)
            {
                for arc in arcs {
                    let src = self.signal_name_to_id.get(arc.input.port.as_str()).copied();
                    let mut hit = false;
                    for path in &mut mp.tables[table] {
                        if src.is_none_or(|src| {
                            !path.srcs.iter().any(|&i| mp.srcs[i as usize].sig == src)
                        }) || arc
                            .input
                            .edge
                            .as_deref()
                            .is_some_and(|e| path.edge.as_deref() != Some(e))
                            || path.ifnone != arc.ifnone
                            || arc.input.cond.as_ref().is_some_and(|c| {
                                path.cond_text.as_ref().is_none_or(|p| {
                                    normalize_sdf_condition(c) != normalize_sdf_condition(p)
                                })
                            })
                        {
                            continue;
                        }
                        hit = true;
                        path.delays = annotate_path_delays(path.delays, arc);
                    }
                    unmatched += usize::from(!hit);
                }
                self.sdf_delays[id] = mp.tables[table]
                    .iter()
                    .flat_map(|p| p.delays)
                    .max()
                    .unwrap_or(0);
            } else {
                // Xezim also accepts annotations on designs with no specify
                // paths. Retain that destination-level compatibility path.
                for arc in arcs {
                    self.sdf_delays[id] = annotate_path_delays([self.sdf_delays[id]; 12], arc)
                        .into_iter()
                        .max()
                        .unwrap_or(0);
                }
            }
            matched += 1;
        }
        for (name, arcs) in &ann.interconnect_delays {
            let Some(&id) = self.signal_name_to_id.get(name.as_str()) else {
                unmatched += arcs.len();
                continue;
            };
            for arc in arcs {
                let src = self.signal_name_to_id.get(arc.input.port.as_str());
                if src.is_none() || src == Some(&id) {
                    unmatched += 1;
                    continue;
                }
                let delays = annotate_path_delays([self.sdf_delays[id]; 12], arc);
                self.sdf_delays[id] = delays.into_iter().max().unwrap_or(0);
                self.gate_fall_delay_by_id.insert(id, delays[1]);
            }
            matched += 1;
        }
        eprintln!("[SDF] annotated {matched} signals with delays");
        if unmatched != 0 {
            eprintln!("[SDF] {unmatched} delay entries matched no path");
        }
    }

    /// When the path's inputs last changed on a qualifying edge.
    fn path_input_time(&self, mp: &ModulePaths, path: &PathRt) -> Option<u64> {
        use crate::ast::decl::{TIMING_NEGEDGE, TIMING_POSEDGE};
        let mask = match path.edge.as_deref() {
            Some("posedge") => TIMING_POSEDGE,
            Some("negedge") => TIMING_NEGEDGE,
            Some("edge") => TIMING_POSEDGE | TIMING_NEGEDGE,
            _ => {
                return Some(
                    path.srcs
                        .iter()
                        .map(|&s| {
                            let src = &mp.srcs[s as usize];
                            if self.timing_watch_stale(src.watch, src.sig) {
                                self.time
                            } else {
                                src.last
                            }
                        })
                        .max()
                        .unwrap_or(0),
                );
            }
        };
        path.srcs
            .iter()
            .filter_map(|&s| {
                let src = &mp.srcs[s as usize];
                let (at, transition) = if self.timing_watch_stale(src.watch, src.sig) {
                    (self.time, self.timing_watch_transition(src.watch, src.sig))
                } else {
                    (src.last, src.transition)
                };
                (mask & transition != 0).then_some(at)
            })
            .max()
    }

    /// The per-transition delays that apply to the next change of the net
    /// with path table `k`; `None` when no path is enabled.
    fn module_path_delays(&mut self, k: usize) -> Option<[u64; 12]> {
        let mp = self.module_paths.as_ref()?;
        let cond_true = mp.cond_true;
        let conds: Vec<(Option<Arc<Expression>>, bool)> = mp.tables[k]
            .iter()
            .map(|p| (p.cond.clone(), p.ifnone))
            .collect();
        let n = conds.len();
        let mut enabled = vec![false; n];
        for (i, (cond, ifnone)) in conds.into_iter().enumerate() {
            enabled[i] = match cond {
                _ if ifnone => false,
                c @ Some(_) => cond_true(self, &c),
                None => true,
            };
        }
        let mp = self.module_paths.as_ref()?;
        let paths = &mp.tables[k];
        for i in 0..n {
            if paths[i].ifnone {
                enabled[i] = !(0..n).any(|j| {
                    enabled[j]
                        && paths[j].cond.is_some()
                        && paths[j].srcs.iter().any(|s| paths[i].srcs.contains(s))
                });
            }
        }
        let times: Vec<Option<u64>> = (0..n)
            .map(|i| {
                enabled[i]
                    .then(|| self.path_input_time(mp, &paths[i]))
                    .flatten()
            })
            .collect();
        let latest = times.iter().copied().flatten().max()?;
        let mut out = [u64::MAX; 12];
        for i in (0..n).filter(|&i| times[i] == Some(latest)) {
            for (o, d) in out.iter_mut().zip(paths[i].delays) {
                *o = (*o).min(d);
            }
        }
        Some(out)
    }

    /// Schedule `val` onto the path-delayed net `id` (path table `k`): each
    /// bit after the delay of its own transition. Inertial per bit: a bit
    /// already on its way to the same value keeps its pending time, any other
    /// pending change of the net is replaced, and a bit with no delay is
    /// queued for the current time step.
    ///
    /// Out of line so `schedule_delayed_with_delay` stays as small as it was
    /// for designs without module paths.
    #[inline(never)]
    pub(super) fn schedule_module_path(&mut self, id: usize, k: usize, val: Value) {
        let delays = self.module_path_delays(k);
        let old = self.signal_table[id].clone();
        // The net's queued updates, oldest first; each carries every bit
        // changed by the ones before it.
        let mut pending: Vec<(u64, Value)> = self
            .delayed_updates
            .iter()
            .filter(|(_, sid, _, _)| *sid == id)
            .map(|(t, _, v, _)| (*t, v.clone()))
            .collect();
        pending.sort_by_key(|(t, _)| *t);
        self.delayed_updates.retain(|(_, sid, _, _)| *sid != id);
        let width = (old.width as usize).min(val.width as usize);
        // (when, bit) for every bit that changes.
        let mut when: Vec<(u64, usize)> = Vec::new();
        for b in 0..width {
            let (from, to) = (old.get_bit_code(b), val.get_bit_code(b));
            let Some(t) = xezim_core::elaborate::path_transition_index(from, to) else {
                continue;
            };
            let in_flight = pending
                .last()
                .filter(|(_, v)| v.get_bit_code(b) == to)
                .and_then(|_| pending.iter().find(|(_, v)| v.get_bit_code(b) == to))
                .map(|(t, _)| *t);
            let at = in_flight.unwrap_or_else(|| self.time + delays.map_or(0, |ds| ds[t]));
            when.push((at, b));
        }
        when.sort_unstable_by_key(|&(t, b)| (t, b));
        let mut cur = old;
        let mut steps: Vec<(u64, Value)> = Vec::new();
        let mut i = 0;
        while i < when.len() {
            let at = when[i].0;
            while i < when.len() && when[i].0 == at {
                let b = when[i].1;
                cur.set_bit_code(b, val.get_bit_code(b));
                i += 1;
            }
            let v = if i == when.len() {
                val.clone()
            } else {
                cur.clone()
            };
            steps.push((at, v));
        }
        // Latest first: `schedule_delayed_slice` and UDP outputs compose
        // onto the FIRST queued update of a net, which must be the one that
        // leaves it at its final value. A bit with no delay (no enabled path)
        // is queued at the current time; the scheduler applies it in the
        // next delta of this time step.
        for (at, v) in steps.into_iter().rev() {
            self.delayed_updates.push((at, id, v, None));
        }
    }
}
