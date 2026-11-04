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
//! Not modelled: the edge of an edge-sensitive path (the path applies on
//! any change of its input) and PATHPULSE$ limits — the reference simulator
//! also rejects every pulse narrower than the path delay by default.
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
    ifnone: bool,
}

/// A path input terminal: its signal, the watch stamping its changes, and
/// the time of its last change.
#[derive(serde::Serialize, serde::Deserialize)]
struct PathSrc {
    sig: usize,
    watch: usize,
    last: u64,
}

impl ModulePaths {
    pub(super) fn table_of(&self, id: usize) -> Option<usize> {
        self.dst.get(&id).copied()
    }

    pub(super) fn is_path_net(&self, id: usize) -> bool {
        self.dst.contains_key(&id)
    }

    /// Input `slot` changed at `now`.
    pub(super) fn stamp(&mut self, slot: u32, now: u64) {
        self.srcs[slot as usize].last = now;
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

impl Simulator {
    /// Resolve `ElaboratedModule::module_paths` to signal ids. Nets whose
    /// delay SDF back-annotated keep the SDF value (it replaces the path
    /// delays), so they get no path table.
    pub(super) fn build_module_paths(
        &mut self,
        skip: &std::collections::HashSet<usize>,
        map_delay: impl Fn(u64) -> u64,
    ) {
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
            if skip.contains(&id) {
                continue;
            }
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

    /// Runtime SDF back-annotation replaces the path delays of the nets it
    /// annotates.
    pub(super) fn drop_module_paths_of(&mut self, id: usize) {
        if let Some(mp) = self.module_paths.as_mut() {
            mp.dst.remove(&id);
        }
    }

    /// When the path's inputs last changed.
    fn path_input_time(&self, mp: &ModulePaths, srcs: &[u32]) -> u64 {
        srcs.iter()
            .map(|&s| {
                let src = &mp.srcs[s as usize];
                if self.timing_watch_stale(src.watch, src.sig) {
                    self.time
                } else {
                    src.last
                }
            })
            .max()
            .unwrap_or(0)
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
        let times: Vec<u64> = (0..n)
            .map(|i| {
                if enabled[i] {
                    self.path_input_time(mp, &paths[i].srcs)
                } else {
                    0
                }
            })
            .collect();
        let latest = (0..n).filter(|&i| enabled[i]).map(|i| times[i]).max()?;
        let mut out = [u64::MAX; 12];
        for i in (0..n).filter(|&i| enabled[i] && times[i] == latest) {
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
            .filter(|(_, sid, _)| *sid == id)
            .map(|(t, _, v)| (*t, v.clone()))
            .collect();
        pending.sort_by_key(|(t, _)| *t);
        self.delayed_updates.retain(|(_, sid, _)| *sid != id);
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
            self.delayed_updates.push((at, id, v));
        }
    }
}
