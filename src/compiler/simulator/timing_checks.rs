//! IEEE 1800-2017 §31 timing checks.
//!
//! The reference and data terminals of every flattened check
//! (`ElaboratedModule::timing_checks`) are grouped by the signal they watch,
//! and each watched signal becomes one comb entry (`CombItem::TimingCheck`),
//! so checks cost nothing until a terminal changes. The entry classifies the
//! change once (LSB transition, or per bit for bit-select terminals) and
//! dispatches it only to the terminals whose edge mask admits it — a clock
//! shared by thousands of flops is one entry, and its posedge never visits
//! the negedge-only terminals. Each event applies its `&&&` condition (x or
//! z disables it, as in the reference simulator) and updates the check's
//! timestamps. A violation prints one
//! diagnostic and toggles the notifier (§31.6: x->0, 0->1, 1->0, z stays z)
//! through the NBA region, so a notifier-driven UDP or `always @(notifier)`
//! reacts after the design's own response to the same edge. Two violations
//! of one notifier in the same time step toggle it once.
//!
//! Semantics pinned against the reference simulator:
//! * events at time 0 set no timestamps (only the last-seen values);
//! * the setup side of a window excludes a data event simultaneous with the
//!   reference event, the hold side includes it — `$setup`/`$removal` never
//!   report simultaneous events, `$hold`/`$recovery` do;
//! * the window of `$setuphold`/`$recrem` is `(ref - setup, ref + hold)`, so
//!   a negative limit shifts it past the reference edge (the delayed
//!   reference/data nets themselves are not delayed);
//! * `$width`/`$period` ignore pulses at or below the threshold (default 0);
//! * `$timeskew`/`$fullskew` are time-based unless their event_based_flag is
//!   set: a scheduler deadline reports the violation when the limit elapses.
//!
//! Not modelled: the delayed reference/data nets of negative limits (they
//! follow their source with no delay), and the reference simulator's
//! zeroing of negative limits when several checks on one signal pair admit
//! no common delay. A time-based `$fullskew` with remain_active_flag set
//! behaves as without it.
use super::*;
use crate::ast::decl::{TIMING_NEGEDGE, TIMING_POSEDGE, TimingCheckArg, timing_edge_bit};

/// `+notimingcheck` (and `+nospecify`): do not build timing checks at all.
static NO_TIMING_CHECKS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// `+no_notifier`: report violations but leave notifiers untouched.
static NO_NOTIFIER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// `+no_tchk_msg`: toggle notifiers but print no violation messages.
static NO_TCHK_MSG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_no_timing_checks(v: bool) {
    NO_TIMING_CHECKS.store(v, std::sync::atomic::Ordering::Relaxed);
}

pub fn set_no_notifier(v: bool) {
    NO_NOTIFIER.store(v, std::sync::atomic::Ordering::Relaxed);
}

pub fn set_no_tchk_msg(v: bool) {
    NO_TCHK_MSG.store(v, std::sync::atomic::Ordering::Relaxed);
}

/// A reference or data terminal of a check.
#[derive(Clone)]
pub(super) struct TcTerm {
    sig: usize,
    /// Physical bit of a bit-select terminal; `None` watches the whole signal.
    bit: Option<u32>,
    /// Transition mask; `None` = any value change.
    edges: Option<u16>,
    cond: Option<Arc<Expression>>,
    /// Source spelling (`posedge clk &&& en`).
    text: String,
}

/// Event roles of a terminal: the reference event, the opposite reference
/// edge that closes a `$width` pulse / `$nochange` window, the data event.
const ROLE_REF: u8 = 0;
const ROLE_CLOSE: u8 = 1;
const ROLE_DATA: u8 = 2;

/// The terminals watching one signal, indexed by `CombItem::TimingCheck`.
pub(super) struct TcWatch {
    sig: usize,
    /// Value last seen: raw planes, or the whole value past 64 bits.
    prev_v: u64,
    prev_x: u64,
    wide: Option<Value>,
    /// Whole-signal terminals with an edge control, by LSB transition
    /// `from * 3 + to` (levels 0, 1, 2 = x/z): (check, role).
    by_tr: [Vec<(u32, u8)>; 9],
    /// Whole-signal terminals without one: every value change.
    any: Vec<(u32, u8)>,
    /// Bit-select terminals: (bit, edge mask, check, role).
    bits: Vec<(u32, Option<u16>, u32, u8)>,
    /// §30.4 module path inputs (`Simulator::path_srcs`) whose last-change
    /// time this watch stamps.
    path_srcs: Vec<u32>,
}

/// Level (0, 1, 2 = x/z) of bit `b` of raw planes.
#[inline]
fn level(v: u64, x: u64, b: u32) -> u8 {
    ((((x >> b) & 1) << 1) | ((v >> b) & 1)).min(2) as u8
}

#[derive(Clone, Copy)]
enum TcKind {
    /// `$setup`/`$hold`/`$setuphold` and `$recovery`/`$removal`/`$recrem`:
    /// a data event in `(ref - setup, ref + hold)` violates; a data event
    /// simultaneous with the reference violates when `setup >= 0 < hold`.
    /// `recrem` selects the `$removal`/`$recovery` names.
    Window {
        setup: i64,
        hold: i64,
        recrem: bool,
    },
    /// `$skew`: every data event later than the last reference + limit.
    Skew {
        limit: i64,
    },
    /// `$timeskew`: the data event after a reference event is late by more
    /// than `limit`. Time-based by default (reported when the limit
    /// elapses). Event-based, a violation puts the check to sleep until the
    /// next reference event unless `remain_active`.
    TimeSkew {
        limit: i64,
        event_based: bool,
        remain_active: bool,
    },
    /// `$fullskew`: whichever terminal changes first opens the window and
    /// the other must follow within `limit1` (reference first) or `limit2`
    /// (data first). Time-based unless `event_based`.
    FullSkew {
        limit1: i64,
        limit2: i64,
        event_based: bool,
    },
    Period {
        limit: i64,
    },
    Width {
        limit: i64,
        threshold: i64,
    },
    /// Data change in `(leading - start, trailing + end)`.
    Nochange {
        start: i64,
        end: i64,
    },
}

#[derive(Clone)]
pub(super) struct TimingCheckRt {
    kind: TcKind,
    notifier: Option<BitRef>,
    /// Which conditions exist (`COND_*`), so an event reads `cold` only
    /// when it has one to evaluate.
    conds: u8,
    ref_time: Option<u64>,
    data_time: Option<u64>,
    /// `$width` pending pulse start; `$nochange` trailing edge.
    aux_time: Option<u64>,
    /// `$timeskew`/`$fullskew`: the event that opened the window (true =
    /// reference) and when; the armed time-based deadline; sleeping after
    /// a violation.
    open: Option<(bool, u64)>,
    timer: Option<u64>,
    dormant: bool,
    /// Kept out of line: an event touches only the fields above, and a
    /// clock shared by thousands of checks walks all of them.
    cold: Box<TcCold>,
}

const COND_REF: u8 = 1;
const COND_DATA: u8 = 2;
const COND_STAMP: u8 = 4;
const COND_CHECK: u8 = 8;

/// The parts of a check that events rarely read: diagnostics, SDF matching
/// and the condition expressions.
#[derive(Clone)]
struct TcCold {
    name: String,
    /// Instance scope and module definition, for SDF matching.
    scope: String,
    def_name: String,
    /// Full instance path for the diagnostics.
    path: String,
    loc: Option<String>,
    reference: TcTerm,
    data: Option<TcTerm>,
    /// `$setuphold`/`$recrem` timestamp_condition / timecheck_condition.
    stamp_cond: Option<Arc<Expression>>,
    check_cond: Option<Arc<Expression>>,
}

/// Swap the direction of every transition in an edge mask.
fn reverse_edges(mask: u16) -> u16 {
    let mut out = 0u16;
    for from in 0..3u8 {
        for to in 0..3u8 {
            if mask & timing_edge_bit(from, to) != 0 {
                out |= timing_edge_bit(to, from);
            }
        }
    }
    out
}

/// A terminal's spelling without its edge control and condition.
fn terminal_text(text: &str) -> &str {
    let t = text.split("&&&").next().unwrap_or(text).trim();
    let t = t
        .strip_prefix("posedge")
        .or_else(|| t.strip_prefix("negedge"))
        .unwrap_or(t);
    let t = match t.strip_prefix("edge") {
        Some(rest) => match rest.trim_start().strip_prefix('[') {
            Some(r) => r.split_once(']').map(|(_, s)| s).unwrap_or(r),
            None => rest,
        },
        None => t,
    };
    t.trim()
}

impl Simulator {
    /// Resolve `module.timing_checks` into comb entries (see the module doc).
    pub(super) fn build_timing_check_entries(&mut self, _entries: &mut Vec<CombEntry>) {
        let checks = std::mem::take(&mut self.module.timing_checks);
        if checks.is_empty()
            || NO_TIMING_CHECKS.load(std::sync::atomic::Ordering::Relaxed)
            || nospecify()
        {
            return;
        }
        let mut warnings: Vec<String> = Vec::new();
        let mut watch_of: HashMap<usize, usize> = HashMap::default();
        for tc in checks {
            let arg = |i: usize| tc.args.get(i).and_then(|a| a.as_ref());
            let konst = |i: usize| tc.consts.get(i).copied().flatten();
            let path = self.hier_path(&tc.scope);
            let loc = self.span_file_line_in(
                tc.span,
                self.module.src_file_of_module.get(&tc.def_name).copied(),
            );
            let what = format!(
                "{} in {}{}",
                tc.name,
                path,
                loc.as_ref()
                    .map(|l| format!(" ({})", l))
                    .unwrap_or_default()
            );
            let (ref_i, data_i): (usize, Option<usize>) = match tc.name.as_str() {
                "$setup" => (1, Some(0)),
                "$period" | "$width" => (0, None),
                _ => (0, Some(1)),
            };
            let limit = |i: usize| konst(i).unwrap_or(0);
            if matches!(
                tc.name.as_str(),
                "$setup" | "$hold" | "$recovery" | "$removal"
            ) && limit(2) < 0
            {
                warnings.push(format!(
                    "{}: negative limit {} taken as 0",
                    what,
                    self.timing_fmt(limit(2))
                ));
            }
            let (kind, notifier_i) = match tc.name.as_str() {
                // A negative single limit is taken as 0 (it can then never
                // fire), as in the reference simulator.
                "$setup" | "$removal" => (
                    TcKind::Window {
                        setup: limit(2).max(0),
                        hold: 0,
                        recrem: tc.name == "$removal",
                    },
                    3,
                ),
                "$hold" | "$recovery" => (
                    TcKind::Window {
                        setup: 0,
                        hold: limit(2).max(0),
                        recrem: tc.name == "$recovery",
                    },
                    3,
                ),
                "$setuphold" | "$recrem" => {
                    // $recrem(ref, data, recovery, removal): removal is the
                    // data-before-reference side, recovery the side after.
                    let (s, h) = if tc.name == "$recrem" {
                        (limit(3), limit(2))
                    } else {
                        (limit(2), limit(3))
                    };
                    // §31.9: negative limits need a positive sum; otherwise
                    // the window is empty and the check never fires.
                    if s.min(h) < 0 && s + h <= 0 {
                        warnings.push(format!(
                            "{}: limits {} and {} leave no violation window (their sum \
                             must be positive); it never fires",
                            what,
                            self.timing_fmt(s),
                            self.timing_fmt(h)
                        ));
                    }
                    (
                        TcKind::Window {
                            setup: s,
                            hold: h,
                            recrem: tc.name == "$recrem",
                        },
                        4,
                    )
                }
                "$skew" => (TcKind::Skew { limit: limit(2) }, 3),
                "$timeskew" => (
                    TcKind::TimeSkew {
                        limit: limit(2),
                        event_based: konst(4).unwrap_or(0) != 0,
                        remain_active: konst(5).unwrap_or(0) != 0,
                    },
                    3,
                ),
                "$fullskew" => (
                    TcKind::FullSkew {
                        limit1: limit(2),
                        limit2: limit(3),
                        event_based: konst(5).unwrap_or(0) != 0,
                    },
                    4,
                ),
                "$period" => (TcKind::Period { limit: limit(1) }, 2),
                "$width" => (
                    TcKind::Width {
                        limit: limit(1),
                        threshold: limit(2),
                    },
                    3,
                ),
                "$nochange" => (
                    TcKind::Nochange {
                        start: limit(2),
                        end: limit(3),
                    },
                    4,
                ),
                _ => continue,
            };
            // A terminal tied to a constant (a cell pin tied off) never has
            // an event: drop the check quietly.
            let terms = [Some(ref_i), data_i];
            if terms.iter().flatten().any(|&i| {
                arg(i).is_some_and(|a| matches!(strip_parens(&a.expr).kind, ExprKind::Number(_)))
            }) {
                continue;
            }
            let mut term = |i: usize, role: &str| {
                let t = arg(i).and_then(|a| self.resolve_timing_term(a));
                if t.is_none() {
                    warnings.push(format!(
                        "{}: {} event `{}` is not a signal or a constant bit-select of one; \
                         check skipped",
                        what,
                        role,
                        arg(i).map(|a| a.text.as_str()).unwrap_or("")
                    ));
                }
                t
            };
            let Some(reference) = term(ref_i, "reference") else {
                continue;
            };
            let data = match data_i {
                Some(i) => match term(i, "data") {
                    Some(t) => Some(t),
                    None => continue,
                },
                None => None,
            };
            if matches!(kind, TcKind::Width { .. } | TcKind::Nochange { .. })
                && reference.edges.is_none()
            {
                warnings.push(format!(
                    "{}: the reference event needs an edge (posedge/negedge); check skipped",
                    what
                ));
                continue;
            }
            let notifier = match arg(notifier_i) {
                Some(a) => match self.resolve_notifier(&a.expr) {
                    Some(b) => Some(b),
                    None => {
                        warnings.push(format!(
                            "{}: notifier `{}` is not a variable bit; it is not toggled",
                            what, a.text
                        ));
                        None
                    }
                },
                None => None,
            };
            let (stamp_cond, check_cond) = if matches!(tc.name.as_str(), "$setuphold" | "$recrem") {
                (
                    arg(5).map(|a| Arc::new(a.expr.clone())),
                    arg(6).map(|a| Arc::new(a.expr.clone())),
                )
            } else {
                (None, None)
            };
            let idx = self.timing_checks.len() as u32;
            self.timing_watch(&mut watch_of, &reference, idx, ROLE_REF, reference.edges);
            if matches!(kind, TcKind::Width { .. } | TcKind::Nochange { .. }) {
                let close = reference.edges.map(reverse_edges);
                self.timing_watch(&mut watch_of, &reference, idx, ROLE_CLOSE, close);
            }
            if let Some(d) = &data {
                self.timing_watch(&mut watch_of, d, idx, ROLE_DATA, d.edges);
            }
            let conds = [
                (reference.cond.is_some(), COND_REF),
                (data.as_ref().is_some_and(|d| d.cond.is_some()), COND_DATA),
                (stamp_cond.is_some(), COND_STAMP),
                (check_cond.is_some(), COND_CHECK),
            ]
            .iter()
            .filter(|(on, _)| *on)
            .fold(0u8, |a, (_, bit)| a | bit);
            self.timing_checks.push(TimingCheckRt {
                kind,
                notifier,
                conds,
                ref_time: None,
                data_time: None,
                aux_time: None,
                open: None,
                timer: None,
                dormant: false,
                cold: Box::new(TcCold {
                    name: tc.name.clone(),
                    scope: tc.scope.clone(),
                    def_name: tc.def_name.clone(),
                    path,
                    loc,
                    reference,
                    data,
                    stamp_cond,
                    check_cond,
                }),
            });
        }
        let shown = warnings.len().min(20);
        for w in &warnings[..shown] {
            eprintln!("Warning: {}", w);
        }
        if warnings.len() > shown {
            eprintln!(
                "Warning: {} more timing check warnings",
                warnings.len() - shown
            );
        }
        self.apply_sdf_timing_limits();
    }

    /// One comb entry per watched signal — timing-check terminals and
    /// §30.4 module path inputs alike. Watches keep state, so they force
    /// the serial settle.
    pub(super) fn push_timing_watch_entries(&mut self, entries: &mut Vec<CombEntry>) {
        for (w, watch) in self.timing_watches.iter().enumerate() {
            entries.push(CombEntry {
                item: CombItem::TimingCheck { idx: w },
                cold: Box::new(CombEntryCold {
                    scope_hint: None,
                    read_signal_ids: vec![watch.sig],
                    write_signal_ids: Vec::new(),
                    span: crate::ast::Span::dummy(),
                    cov: None,
                }),
                has_unresolved_reads: false,
                defer_at_time0: false,
            });
        }
        self.has_timing_checks = !self.timing_watches.is_empty();
    }

    /// A watch that stamps the last-change time of module path input `slot`.
    pub(super) fn path_source_watch(&mut self, sig: usize, slot: u32) -> usize {
        let cur = &self.signal_table[sig];
        let (prev_v, prev_x) = cur.raw_bits();
        let wide = (self.signal_widths[sig] > 64).then(|| cur.clone());
        self.timing_watches.push(TcWatch {
            sig,
            prev_v,
            prev_x,
            wide,
            by_tr: Default::default(),
            any: Vec::new(),
            bits: Vec::new(),
            path_srcs: vec![slot],
        });
        self.timing_watches.len() - 1
    }

    /// Has watch `w`'s signal changed since the watch last ran? (Its entry
    /// has not been evaluated yet in the current settle.)
    pub(super) fn timing_watch_stale(&self, w: usize, sig: usize) -> bool {
        let watch = &self.timing_watches[w];
        match &watch.wide {
            Some(old) => *old != self.signal_table[sig],
            None => self.signal_table[sig].raw_bits() != (watch.prev_v, watch.prev_x),
        }
    }

    /// VPI `vpiTchk` objects: the number of timing checks.
    pub(super) fn vpi_tchk_count(&self) -> usize {
        self.timing_checks.len()
    }

    /// Check `i`'s system task name (`$setuphold`), instance scope and
    /// module definition.
    pub(super) fn vpi_tchk_ident(&self, i: usize) -> Option<(&str, &str, &str)> {
        let rt = self.timing_checks.get(i)?;
        Some((&rt.cold.name, &rt.cold.scope, &rt.cold.def_name))
    }

    /// Check `i`'s limits in source-argument order, as `vpi_get_delays`
    /// reports them: the one limit of `$setup`/`$hold`/`$recovery`/
    /// `$removal`/`$skew`/`$timeskew`/`$period`/`$width`, both limits of
    /// `$setuphold`/`$recrem`/`$fullskew`, the two offsets of `$nochange`.
    pub(super) fn vpi_tchk_limits(&self, i: usize) -> Option<Vec<i64>> {
        let rt = self.timing_checks.get(i)?;
        let name = rt.cold.name.as_str();
        Some(match rt.kind {
            TcKind::Window { setup, hold, .. } => match name {
                "$setup" | "$removal" => vec![setup],
                "$hold" | "$recovery" => vec![hold],
                // $recrem(ref, data, recovery_limit, removal_limit): the
                // recovery side is the one after the reference edge.
                "$recrem" => vec![hold, setup],
                _ => vec![setup, hold],
            },
            TcKind::Skew { limit }
            | TcKind::TimeSkew { limit, .. }
            | TcKind::Period { limit }
            | TcKind::Width { limit, .. } => vec![limit],
            TcKind::FullSkew { limit1, limit2, .. } => vec![limit1, limit2],
            TcKind::Nochange { start, end } => vec![start, end],
        })
    }

    /// `vpi_put_delays` on a `vpiTchk`: replace its limits (same order as
    /// `vpi_tchk_limits`). A negative limit of a single-limit window check is
    /// taken as 0, as it is when the check is built.
    pub(super) fn vpi_set_tchk_limits(&mut self, i: usize, l: &[i64]) -> Result<(), String> {
        let want = self
            .vpi_tchk_limits(i)
            .ok_or_else(|| "no such timing check".to_string())?
            .len();
        if l.len() != want {
            return Err(format!(
                "this timing check has {} limit(s), not {}",
                want,
                l.len()
            ));
        }
        let rt = &mut self.timing_checks[i];
        let name = rt.cold.name.clone();
        match &mut rt.kind {
            TcKind::Window { setup, hold, .. } => match name.as_str() {
                "$setup" | "$removal" => *setup = l[0].max(0),
                "$hold" | "$recovery" => *hold = l[0].max(0),
                "$recrem" => {
                    *hold = l[0];
                    *setup = l[1];
                }
                _ => {
                    *setup = l[0];
                    *hold = l[1];
                }
            },
            TcKind::Skew { limit }
            | TcKind::TimeSkew { limit, .. }
            | TcKind::Period { limit }
            | TcKind::Width { limit, .. } => *limit = l[0],
            TcKind::FullSkew { limit1, limit2, .. } => {
                *limit1 = l[0];
                *limit2 = l[1];
            }
            TcKind::Nochange { start, end } => {
                *start = l[0];
                *end = l[1];
            }
        }
        Ok(())
    }

    /// SDF TIMINGCHECK back-annotation: replace the limits of the matching
    /// checks. An entry matches by instance (scope, full path, or `*` with
    /// the cell type), check kind, port names, edges and `COND` text; one
    /// without `COND` covers the conditioned checks too.
    pub(super) fn apply_sdf_timing_limits(&mut self) {
        let Some(ann) = self.sdf_annotation.as_ref() else {
            return;
        };
        if ann.timing_limits.is_empty() || self.timing_checks.is_empty() {
            return;
        }
        let entries = ann.timing_limits.clone();
        let (mut applied, mut unmatched) = (0usize, 0usize);
        for l in &entries {
            let mut hit = false;
            for rt in self.timing_checks.iter_mut() {
                let inst_ok = if l.instance == "*" {
                    l.cell_type == rt.cold.def_name
                } else {
                    l.instance == rt.cold.scope || l.instance == rt.cold.path
                };
                if inst_ok && sdf_set_limits(rt, l) {
                    hit = true;
                    applied += 1;
                }
            }
            if !hit {
                unmatched += 1;
            }
        }
        eprintln!(
            "[SDF] annotated {} timing check(s){}",
            applied,
            if unmatched > 0 {
                format!("; {} TIMINGCHECK entries matched no check", unmatched)
            } else {
                String::new()
            }
        );
    }

    /// A terminal (§31.2 specify_terminal_descriptor): a signal, or a
    /// constant bit-select of one. A part-select watches its whole signal.
    fn resolve_timing_term(&self, a: &TimingCheckArg) -> Option<TcTerm> {
        let e = strip_parens(&a.expr);
        let (sig, bit) = match &e.kind {
            ExprKind::Ident(h) => (self.resolve_ident_id(h, None)?, None),
            ExprKind::Index { .. } => {
                let b = self.try_resolve_bit_ref(e, None)?;
                (b.sig_id as usize, Some(b.bit))
            }
            ExprKind::RangeSelect { expr: base, .. } => match &base.kind {
                ExprKind::Ident(h) => (self.resolve_ident_id(h, None)?, None),
                _ => return None,
            },
            _ => return None,
        };
        Some(TcTerm {
            sig,
            bit,
            edges: a.edges,
            cond: a.cond.as_ref().map(|c| Arc::new(c.clone())),
            text: a.text.clone(),
        })
    }

    /// Add a terminal's event to the watch of its signal.
    fn timing_watch(
        &mut self,
        watch_of: &mut HashMap<usize, usize>,
        t: &TcTerm,
        idx: u32,
        role: u8,
        mask: Option<u16>,
    ) {
        let w = match watch_of.get(&t.sig) {
            Some(&w) => w,
            None => {
                let cur = &self.signal_table[t.sig];
                let (prev_v, prev_x) = cur.raw_bits();
                let wide = (self.signal_widths[t.sig] > 64).then(|| cur.clone());
                self.timing_watches.push(TcWatch {
                    sig: t.sig,
                    prev_v,
                    prev_x,
                    wide,
                    by_tr: Default::default(),
                    any: Vec::new(),
                    bits: Vec::new(),
                    path_srcs: Vec::new(),
                });
                watch_of.insert(t.sig, self.timing_watches.len() - 1);
                self.timing_watches.len() - 1
            }
        };
        let watch = &mut self.timing_watches[w];
        match (t.bit, mask) {
            (Some(b), _) => watch.bits.push((b, mask, idx, role)),
            (None, None) => watch.any.push((idx, role)),
            (None, Some(m)) => {
                for from in 0..3u8 {
                    for to in 0..3u8 {
                        if m & timing_edge_bit(from, to) != 0 {
                            watch.by_tr[(from * 3 + to) as usize].push((idx, role));
                        }
                    }
                }
            }
        }
    }

    fn resolve_notifier(&self, e: &Expression) -> Option<BitRef> {
        if let Some(b) = self.try_resolve_bit_ref(e, None) {
            return Some(b);
        }
        match &e.kind {
            ExprKind::Ident(h) => self.resolve_ident_id(h, None).map(|id| BitRef {
                sig_id: id as u32,
                bit: 0,
            }),
            _ => None,
        }
    }

    /// `&&&` condition / timestamp / timecheck condition: enabled only for
    /// a known nonzero value.
    pub(super) fn timing_cond_true(&mut self, c: &Option<Arc<Expression>>) -> bool {
        let Some(e) = c else { return true };
        // The condition is already in the flat namespace; a scope hint left
        // behind by the previous evaluation must not re-root its names.
        let saved = self.name_resolve_hint.borrow_mut().take();
        let on = self.eval_expr(e).is_true();
        *self.name_resolve_hint.borrow_mut() = saved;
        on
    }

    /// A watched signal changed (or may have): classify the change and
    /// dispatch it to the terminals that take it.
    pub(super) fn eval_timing_watch(&mut self, w: usize) {
        let sig = self.timing_watches[w].sig;
        let wide = self.timing_watches[w].wide.is_some();
        // (previous, current) planes; for a wide signal the bit accessors
        // below read the Values instead.
        let (pv, px) = (self.timing_watches[w].prev_v, self.timing_watches[w].prev_x);
        let (cv, cx) = self.signal_table[sig].raw_bits();
        let mut old_wide = None;
        if wide {
            let cur = &self.signal_table[sig];
            if self.timing_watches[w].wide.as_ref() == Some(cur) {
                return;
            }
            old_wide = self.timing_watches[w].wide.replace(cur.clone());
        } else {
            if pv == cv && px == cx {
                return;
            }
            self.timing_watches[w].prev_v = cv;
            self.timing_watches[w].prev_x = cx;
        }
        for k in 0..self.timing_watches[w].path_srcs.len() {
            let slot = self.timing_watches[w].path_srcs[k];
            let now = self.time;
            if let Some(mp) = self.module_paths.as_mut() {
                mp.stamp(slot, now);
            }
        }
        if self.time == 0 {
            return;
        }
        let bit_levels = |sim: &Self, b: u32| -> (u8, u8) {
            match &old_wide {
                Some(old) => (
                    old.get_bit_code(b as usize).min(2),
                    sim.signal_table[sig].get_bit_code(b as usize).min(2),
                ),
                None => (level(pv, px, b), level(cv, cx, b)),
            }
        };
        let (from, to) = bit_levels(self, 0);
        if from != to {
            let t = (from * 3 + to) as usize;
            for k in 0..self.timing_watches[w].by_tr[t].len() {
                let (idx, role) = self.timing_watches[w].by_tr[t][k];
                self.timing_dispatch(idx, role);
            }
        }
        for k in 0..self.timing_watches[w].any.len() {
            let (idx, role) = self.timing_watches[w].any[k];
            self.timing_dispatch(idx, role);
        }
        for k in 0..self.timing_watches[w].bits.len() {
            let (b, mask, idx, role) = self.timing_watches[w].bits[k];
            let (from, to) = bit_levels(self, b);
            if from != to && mask.is_none_or(|m| m & timing_edge_bit(from, to) != 0) {
                self.timing_dispatch(idx, role);
            }
        }
    }

    /// Mid-process settle: the writing process has not suspended yet, so a
    /// condition it assigns next (`clk = 1; en = 0;`) must still be seen —
    /// park the event until it does.
    #[inline]
    fn timing_dispatch(&mut self, idx: u32, role: u8) {
        if self.proc_depth > 0 {
            self.timing_pending.push((idx, role));
        } else {
            self.timing_role_event(idx as usize, role);
        }
    }

    /// Run the timing-check events parked while a process was executing.
    pub(super) fn drain_timing_pending(&mut self) {
        if self.timing_pending.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.timing_pending);
        for &(idx, role) in &pending {
            self.timing_role_event(idx as usize, role);
        }
        let mut pending = pending;
        pending.clear();
        if self.timing_pending.is_empty() {
            self.timing_pending = pending;
        }
    }

    /// `$setuphold`/`$recrem` timecheck and timestamp conditions.
    fn timing_window_conds(
        &self,
        idx: usize,
    ) -> (Option<Arc<Expression>>, Option<Arc<Expression>>) {
        let rt = &self.timing_checks[idx];
        if rt.conds & (COND_STAMP | COND_CHECK) == 0 {
            return (None, None);
        }
        (rt.cold.check_cond.clone(), rt.cold.stamp_cond.clone())
    }

    fn timing_role_event(&mut self, idx: usize, role: u8) {
        let rt = &self.timing_checks[idx];
        let bit = if role == ROLE_DATA {
            COND_DATA
        } else {
            COND_REF
        };
        if rt.conds & bit != 0 {
            let cond = if role == ROLE_DATA {
                rt.cold.data.as_ref().and_then(|d| d.cond.clone())
            } else {
                rt.cold.reference.cond.clone()
            };
            if !self.timing_cond_true(&cond) {
                return;
            }
        }
        let now = self.time;
        match role {
            ROLE_REF => self.timing_ref_event(idx, now),
            ROLE_CLOSE => self.timing_ref_close(idx, now),
            _ => self.timing_data_event(idx, now),
        }
    }

    fn timing_ref_event(&mut self, idx: usize, now: u64) {
        let rt = &self.timing_checks[idx];
        match rt.kind {
            TcKind::Window {
                setup,
                hold,
                recrem,
            } => {
                let (check, stamp) = self.timing_window_conds(idx);
                if let Some(td) = self.timing_checks[idx].data_time {
                    if self.timing_cond_true(&check) {
                        let delta = td as i64 - now as i64;
                        if window_hit(delta, setup, hold) {
                            let limit = if delta < 0 { setup } else { hold };
                            self.timing_window_violation(idx, delta < 0, recrem, now, td, limit);
                        }
                    }
                }
                if self.timing_cond_true(&stamp) {
                    self.timing_checks[idx].ref_time = Some(now);
                }
            }
            TcKind::Skew { .. } => {
                self.timing_checks[idx].ref_time = Some(now);
            }
            TcKind::TimeSkew {
                limit, event_based, ..
            } => {
                let rt = &mut self.timing_checks[idx];
                rt.open = Some((true, now));
                rt.dormant = false;
                if !event_based {
                    self.timing_arm(idx, now + limit.max(0) as u64);
                }
            }
            TcKind::FullSkew { .. } => self.timing_fullskew_event(idx, true, now),
            TcKind::Period { limit } => {
                if let Some(tr) = rt.ref_time {
                    if ((now - tr) as i64) < limit {
                        let msg = format!(
                            "{}( {}:{}, {}:{}, {} )",
                            rt.cold.name,
                            rt.cold.reference.text,
                            self.timing_fmt(tr as i64),
                            rt.cold.reference.text,
                            self.timing_fmt(now as i64),
                            self.timing_fmt(limit)
                        );
                        self.timing_violation(idx, msg);
                    }
                }
                self.timing_checks[idx].ref_time = Some(now);
            }
            TcKind::Width { .. } => {
                self.timing_checks[idx].aux_time = Some(now);
            }
            TcKind::Nochange { start, .. } => {
                if let Some(td) = rt.data_time {
                    if (td as i64) > now as i64 - start {
                        let msg = self.timing_nochange_msg(idx, now, td);
                        self.timing_violation(idx, msg);
                    }
                }
                let rt = &mut self.timing_checks[idx];
                rt.ref_time = Some(now);
                rt.aux_time = None;
            }
        }
    }

    /// The edge opposite the reference edge: ends a `$width` pulse, marks
    /// the trailing edge of a `$nochange` window.
    fn timing_ref_close(&mut self, idx: usize, now: u64) {
        let rt = &self.timing_checks[idx];
        match rt.kind {
            TcKind::Width { limit, threshold } => {
                if let Some(ts) = rt.aux_time {
                    let w = (now - ts) as i64;
                    if w < limit && w > threshold {
                        let close = terminal_text(&rt.cold.reference.text).to_string();
                        let close = match rt.cold.reference.edges {
                            Some(m) if m == TIMING_POSEDGE => format!("negedge {}", close),
                            Some(m) if m == TIMING_NEGEDGE => format!("posedge {}", close),
                            _ => close,
                        };
                        let msg = format!(
                            "{}( {}:{}, {}:{}, {} )",
                            rt.cold.name,
                            rt.cold.reference.text,
                            self.timing_fmt(ts as i64),
                            close,
                            self.timing_fmt(now as i64),
                            self.timing_fmt(limit)
                        );
                        self.timing_violation(idx, msg);
                    }
                }
                self.timing_checks[idx].aux_time = None;
            }
            TcKind::Nochange { .. } => {
                let rt = &mut self.timing_checks[idx];
                if rt.ref_time.is_some() && rt.aux_time.is_none() {
                    rt.aux_time = Some(now);
                }
            }
            _ => {}
        }
    }

    fn timing_data_event(&mut self, idx: usize, now: u64) {
        let rt = &self.timing_checks[idx];
        match rt.kind {
            TcKind::Window {
                setup,
                hold,
                recrem,
            } => {
                let (check, stamp) = self.timing_window_conds(idx);
                if let Some(tr) = self.timing_checks[idx].ref_time {
                    if self.timing_cond_true(&check) {
                        let delta = now as i64 - tr as i64;
                        if window_hit(delta, setup, hold) {
                            let limit = if delta < 0 { setup } else { hold };
                            self.timing_window_violation(idx, delta < 0, recrem, tr, now, limit);
                        }
                    }
                }
                if self.timing_cond_true(&stamp) {
                    self.timing_checks[idx].data_time = Some(now);
                }
            }
            TcKind::Skew { limit } => {
                if let Some(tr) = rt.ref_time {
                    if (now - tr) as i64 > limit {
                        let msg = self.timing_pair_msg(idx, true, tr, now, limit);
                        self.timing_violation(idx, msg);
                    }
                }
            }
            TcKind::TimeSkew {
                limit,
                event_based,
                remain_active,
            } => {
                let Some((true, tr)) = rt.open else { return };
                if rt.dormant {
                    return;
                }
                let late = (now - tr) as i64 > limit;
                if !event_based {
                    // Time-based: only the deadline reports; data in time
                    // disarms it.
                    if !late {
                        self.timing_checks[idx].timer = None;
                    }
                    return;
                }
                if late {
                    let msg = self.timing_pair_msg(idx, true, tr, now, limit);
                    self.timing_violation(idx, msg);
                    self.timing_checks[idx].dormant = !remain_active;
                }
            }
            TcKind::FullSkew { .. } => self.timing_fullskew_event(idx, false, now),
            TcKind::Nochange { start, end } => {
                if let Some(tl) = rt.ref_time {
                    let inside = match rt.aux_time {
                        None => (now as i64) > tl as i64 - start,
                        Some(tt) => {
                            (now as i64) < tt as i64 + end && (now as i64) > tl as i64 - start
                        }
                    };
                    if inside {
                        let msg = self.timing_nochange_msg(idx, tl, now);
                        self.timing_violation(idx, msg);
                    }
                }
                self.timing_checks[idx].data_time = Some(now);
            }
            TcKind::Period { .. } | TcKind::Width { .. } => {}
        }
    }

    /// `$skew( ref:t1, data:t2, limit )`, or the data-first order.
    fn timing_pair_msg(&self, idx: usize, ref_first: bool, t1: u64, t2: u64, limit: i64) -> String {
        let rt = &self.timing_checks[idx];
        let data_text = rt.cold.data.as_ref().map(|d| d.text.as_str()).unwrap_or("");
        let (first, second) = if ref_first {
            (rt.cold.reference.text.as_str(), data_text)
        } else {
            (data_text, rt.cold.reference.text.as_str())
        };
        format!(
            "{}( {}:{}, {}:{}, {} )",
            rt.cold.name,
            first,
            self.timing_fmt(t1 as i64),
            second,
            self.timing_fmt(t2 as i64),
            self.timing_fmt(limit)
        )
    }

    /// `$fullskew` event on one terminal (`is_ref`): close a window the
    /// other terminal opened, or open (restart) one.
    fn timing_fullskew_event(&mut self, idx: usize, is_ref: bool, now: u64) {
        let TcKind::FullSkew {
            limit1,
            limit2,
            event_based,
        } = self.timing_checks[idx].kind
        else {
            return;
        };
        let lim = |ref_opened: bool| if ref_opened { limit1 } else { limit2 };
        match self.timing_checks[idx].open {
            Some((opener, ts)) if opener != is_ref => {
                let limit = lim(opener);
                if event_based && (now - ts) as i64 > limit {
                    let msg = self.timing_pair_msg(idx, opener, ts, now, limit);
                    self.timing_violation(idx, msg);
                }
                let rt = &mut self.timing_checks[idx];
                rt.open = None;
                rt.timer = None;
            }
            _ => {
                self.timing_checks[idx].open = Some((is_ref, now));
                if !event_based {
                    self.timing_arm(idx, now + lim(is_ref).max(0) as u64);
                } else {
                    self.timing_checks[idx].timer = None;
                }
            }
        }
    }

    fn timing_arm(&mut self, idx: usize, deadline: u64) {
        self.timing_checks[idx].timer = Some(deadline);
        self.timing_timers.push((deadline, idx));
    }

    /// Earliest armed `$timeskew`/`$fullskew` deadline (scheduler next-time).
    pub(super) fn next_timing_timer(&self) -> Option<u64> {
        self.timing_timers.iter().map(|&(t, _)| t).min()
    }

    /// Report the time-based skew violations whose deadline has come: the
    /// late terminal is printed at the deadline, as it never arrived.
    pub(super) fn fire_timing_timers(&mut self) -> bool {
        let now = self.time;
        if !self.timing_timers.iter().any(|&(t, _)| t <= now) {
            return false;
        }
        let mut due: Vec<(u64, usize)> = Vec::new();
        self.timing_timers.retain(|&(t, idx)| {
            if t <= now {
                due.push((t, idx));
                false
            } else {
                true
            }
        });
        due.sort_unstable();
        let mut fired = false;
        for (deadline, idx) in due {
            let rt = &self.timing_checks[idx];
            if rt.timer != Some(deadline) {
                continue;
            }
            let Some((opener, ts)) = rt.open else {
                continue;
            };
            let limit = (deadline - ts) as i64;
            let msg = self.timing_pair_msg(idx, opener, ts, deadline, limit);
            self.timing_violation(idx, msg);
            fired = true;
            let rt = &mut self.timing_checks[idx];
            rt.timer = None;
            rt.open = None;
        }
        fired
    }

    fn timing_nochange_msg(&self, idx: usize, tl: u64, td: u64) -> String {
        let rt = &self.timing_checks[idx];
        let (start, end) = match rt.kind {
            TcKind::Nochange { start, end } => (start, end),
            _ => (0, 0),
        };
        format!(
            "{}( {}:{}, {}:{}, {}, {} )",
            rt.cold.name,
            rt.cold.reference.text,
            self.timing_fmt(tl as i64),
            rt.cold.data.as_ref().map(|d| d.text.as_str()).unwrap_or(""),
            self.timing_fmt(td as i64),
            self.timing_fmt(start),
            self.timing_fmt(end)
        )
    }

    /// A window check fired: the setup side prints the data event first,
    /// the hold side the reference event first (timestamp, then timecheck).
    fn timing_window_violation(
        &mut self,
        idx: usize,
        setup_side: bool,
        recrem: bool,
        tr: u64,
        td: u64,
        limit: i64,
    ) {
        let rt = &self.timing_checks[idx];
        let data_text = rt.cold.data.as_ref().map(|d| d.text.as_str()).unwrap_or("");
        let msg = if setup_side {
            format!(
                "{}( {}:{}, {}:{}, {} )",
                if recrem { "$removal" } else { "$setup" },
                data_text,
                self.timing_fmt(td as i64),
                rt.cold.reference.text,
                self.timing_fmt(tr as i64),
                self.timing_fmt(limit)
            )
        } else {
            format!(
                "{}( {}:{}, {}:{}, {} )",
                if recrem { "$recovery" } else { "$hold" },
                rt.cold.reference.text,
                self.timing_fmt(tr as i64),
                data_text,
                self.timing_fmt(td as i64),
                self.timing_fmt(limit)
            )
        };
        self.timing_violation(idx, msg);
    }

    /// Report a violation and toggle the check's notifier.
    fn timing_violation(&mut self, idx: usize, check: String) {
        let mut reported: Option<String> = None;
        if !NO_TCHK_MSG.load(std::sync::atomic::Ordering::Relaxed) {
            let rt = &self.timing_checks[idx];
            let line = format!(
                "** Error: {} violation in {} at time {}{}",
                check,
                rt.cold.path,
                self.timing_fmt(self.time as i64),
                rt.cold
                    .loc
                    .as_ref()
                    .map(|l| format!(" ({})", l))
                    .unwrap_or_default()
            );
            self.error_count = self.error_count.saturating_add(1);
            self.record_output(line.clone());
            self.stdout_writeln(&line);
            reported = Some(line);
        }
        // §38.36.3 cbTchkViolation for every violation; cbError when it was
        // reported as a run-time error.
        if self.vpi_cb_mask != 0 {
            if self.vpi_cb_mask & vpi_cb::m(vpi_cb::CB_TCHK_VIOLATION) != 0 {
                let text = format!(
                    "{} violation in {}",
                    check, self.timing_checks[idx].cold.path
                );
                self.vpi_notify_tchk(&text);
            }
            if let Some(line) = reported {
                self.vpi_notify_error(&line);
            }
        }
        if NO_NOTIFIER.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let Some(n) = self.timing_checks[idx].notifier else {
            return;
        };
        // §31.6 Table 31-12. Computed from the current value, so several
        // violations in one time step toggle the notifier once.
        let id = n.sig_id as usize;
        let mut v = self.signal_table[id].clone();
        let new = match v.get_bit_code(n.bit as usize) {
            0 => 1,
            1 | 2 => 0,
            _ => return,
        };
        v.set_bit_code(n.bit as usize, new);
        self.nba_fast_index.insert(id, self.nba_fast.len());
        self.nba_fast.push(NbaFast {
            mask: None,
            signal_id: id,
            value: v,
            block_index: 0,
        });
    }

    /// Ticks as `<n> <unit>` in the largest unit that keeps `n` integral
    /// (`19500 ps`, `20 ns`).
    fn timing_fmt(&self, ticks: i64) -> String {
        if ticks == 0 {
            return "0".to_string();
        }
        let tick_exp = Self::secs_to_exp(self.tick_s);
        let mut fs = ticks as i128;
        for _ in 0..(tick_exp + 15).max(0) {
            fs *= 10;
        }
        for (exp, unit) in [
            (0, "s"),
            (-3, "ms"),
            (-6, "us"),
            (-9, "ns"),
            (-12, "ps"),
            (-15, "fs"),
        ] {
            let scale = 10i128.pow((exp + 15) as u32);
            if fs % scale == 0 {
                return format!("{} {}", fs / scale, unit);
            }
        }
        format!("{} fs", fs)
    }
}

/// Does an SDF TIMINGCHECK port name the terminal (name, edge, `COND`)?
fn sdf_port_matches(t: &TcTerm, p: &crate::compiler::sdf::SdfTimingPort) -> bool {
    if terminal_text(&t.text) != p.port {
        return false;
    }
    if let Some(e) = &p.edge {
        let level = |c: char| match c {
            '0' => Some(0u8),
            '1' => Some(1u8),
            'x' | 'X' | 'z' | 'Z' => Some(2u8),
            _ => None,
        };
        let mask = match e.as_str() {
            "posedge" => TIMING_POSEDGE,
            "negedge" => TIMING_NEGEDGE,
            d => {
                let mut cs = d.chars();
                match (cs.next().and_then(level), cs.next().and_then(level)) {
                    (Some(f), Some(to)) => timing_edge_bit(f, to),
                    _ => return false,
                }
            }
        };
        if t.edges != Some(mask) {
            return false;
        }
    }
    match &p.cond {
        None => true,
        Some(c) => {
            let norm = |s: &str| {
                let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
                let s = s.replace("1'b", "").replace("1'B", "");
                let mut s = s.as_str();
                while s.starts_with('(') && s.ends_with(')') {
                    s = &s[1..s.len() - 1];
                }
                s.to_string()
            };
            t.text
                .split_once("&&&")
                .is_some_and(|(_, tc)| norm(tc) == norm(c))
        }
    }
}

/// Apply one SDF TIMINGCHECK entry to a check if it matches; SDF orders
/// SETUP/HOLD/SETUPHOLD ports (data, reference) and the others (reference,
/// data).
fn sdf_set_limits(rt: &mut TimingCheckRt, l: &crate::compiler::sdf::SdfTimingLimit) -> bool {
    let v = |i: usize| l.limits.get(i).copied().flatten();
    let port = |i: usize| l.ports.get(i);
    let data_first = matches!(l.kind.as_str(), "SETUP" | "HOLD" | "SETUPHOLD");
    let (ref_port, data_port) = if data_first {
        (port(1), port(0))
    } else {
        (port(0), port(1))
    };
    let Some(ref_port) = ref_port else {
        return false;
    };
    if !sdf_port_matches(&rt.cold.reference, ref_port) {
        return false;
    }
    match (&rt.cold.data, data_port) {
        (Some(d), Some(p)) if sdf_port_matches(d, p) => {}
        (None, None) => {}
        _ => return false,
    }
    let name = rt.cold.name.as_str();
    // (setup-side value, hold-side value) for the window kinds.
    let (s, h) = match (l.kind.as_str(), name) {
        ("SETUP", "$setup" | "$setuphold") | ("REMOVAL", "$removal" | "$recrem") => (v(0), None),
        ("HOLD", "$hold" | "$setuphold") | ("RECOVERY", "$recovery" | "$recrem") => (None, v(0)),
        ("SETUPHOLD", "$setuphold" | "$setup" | "$hold") => (v(0), v(1)),
        ("RECREM", "$recrem" | "$recovery" | "$removal") => (v(1), v(0)),
        ("SKEW", "$skew" | "$timeskew") | ("WIDTH", "$width") | ("PERIOD", "$period") => {
            let Some(x) = v(0) else { return false };
            match &mut rt.kind {
                TcKind::Skew { limit }
                | TcKind::TimeSkew { limit, .. }
                | TcKind::Width { limit, .. }
                | TcKind::Period { limit } => *limit = x,
                _ => return false,
            }
            return true;
        }
        ("NOCHANGE", "$nochange") => {
            if let TcKind::Nochange { start, end } = &mut rt.kind {
                *start = v(0).unwrap_or(*start);
                *end = v(1).unwrap_or(*end);
            }
            return true;
        }
        _ => return false,
    };
    let TcKind::Window { setup, hold, .. } = &mut rt.kind else {
        return false;
    };
    // A single-limit check keeps its one side (negative taken as 0).
    let single = !matches!(name, "$setuphold" | "$recrem");
    let (has_s, has_h) = match name {
        "$setup" | "$removal" => (true, false),
        "$hold" | "$recovery" => (false, true),
        _ => (true, true),
    };
    let mut set = false;
    if let (Some(x), true) = (s, has_s) {
        *setup = if single { x.max(0) } else { x };
        set = true;
    }
    if let (Some(x), true) = (h, has_h) {
        *hold = if single { x.max(0) } else { x };
        set = true;
    }
    set
}

fn strip_parens(mut e: &Expression) -> &Expression {
    while let ExprKind::Paren(inner) = &e.kind {
        e = inner;
    }
    e
}

/// `delta` = data time - reference time. The open window
/// `(-setup, hold)`, plus the simultaneous pair when `setup >= 0 < hold`.
#[inline]
fn window_hit(delta: i64, setup: i64, hold: i64) -> bool {
    (-setup < delta && delta < hold) || (delta == 0 && setup >= 0 && hold > 0)
}
