//! Concurrent assertion state: deferred reports, clocked sites, sequence automata, attempts
//! and sampled-value watches.

use super::*;

/// §16.4 a deferred immediate assertion's pending report: the action block
/// to run when it matures, unless its process resumes first (a flush).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct DeferredReport {
    pub(super) pid: usize,
    pub(super) kind: crate::ast::stmt::DeferredAssertion,
    /// The assertion statement, which identifies it for the flush of a
    /// re-executed `always_comb`.
    pub(super) span: crate::ast::Span,
    pub(super) passed: bool,
    pub(super) action: Option<Statement>,
    /// §16.3: a failed assertion with no `else` clause reports the default
    /// `$error`, naming this scope (`%m` where the assertion executed).
    pub(super) default_error_scope: Option<String>,
    /// The scope of the comb-entry `always_comb` that queued the report
    /// (`pid` is then `usize::MAX`).
    pub(super) comb_scope: Option<String>,
    pub(super) this_handle: Option<usize>,
    pub(super) class_context: Option<String>,
    /// §20.12: what an assertion control selects the assertion by.
    pub(super) ctl: CtlIdent,
    /// §16.4.2: the `assertion_stats` entry the outcome is counted in when
    /// the report matures, with its kind tag (0 assert, 1 assume, 2 cover)
    /// and location. A flushed or superseded report is no attempt.
    pub(super) stat_key: usize,
    pub(super) stat_kind: u8,
    pub(super) stat_loc: (Option<u32>, usize),
}

/// LRM §4.4 / §16 observed-region probe entry. A concurrent property's
/// per-cycle re-evaluation registers one of these into
/// `Simulator::pending_observed` from its sensitivity trigger (e.g. a
/// `@(posedge clk)` event). When the observed-region drain runs, the
/// runtime evaluates `predicate` and records the boolean outcome under
/// `name` in `assertion_stats`. Today only the storage + drain are in
/// place; no producer wires probes up. M9 (SVA elaboration) is the
/// consumer-side follow-up.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct ObservedProbe {
    /// The site's `assertion_stats` key, location and kind tag, so its
    /// outcome lands on the same entry the other assertion paths use.
    pub(super) stat_key: usize,
    pub(super) loc: (Option<u32>, usize),
    pub(super) kind: u8,
    pub(super) predicate: Expression,
    /// Optional `this` handle so a property declared in a class context can
    /// resolve member references at evaluation time.
    pub(super) this_handle: Option<usize>,
}

/// §9.4.2: one term in an assertion clocking event, including its last
/// value. A simple signal keeps its resolved name for the common fast path.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct SvaClockEvent {
    pub(super) event: EventExpr,
    pub(super) signal: Option<String>,
    pub(super) previous: Value,
    /// Dense scalar clock cell. Restored checkpoints can use the expression
    /// fallback, so this derived index is omitted from the saved state.
    #[serde(skip)]
    pub(super) scalar_clock_id: Option<usize>,
}

/// LRM §16.5 SVA clocked property site. Registered once when an
/// `assert property (@clk body)` statement first executes; thereafter
/// any term in its clocking event drives per-cycle evaluation of `body`.
/// Every clock tick starts one attempt of `node`; `attempts` holds the ones
/// still in flight (multi-cycle sequences, deferred consequents).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct SvaClockedSite {
    /// Unique identifier (hash of span position) so dedup across
    /// re-executions is cheap.
    pub(super) span_key: usize,
    /// Key of the site's `assertion_stats` tally, and its location.
    pub(super) stat_key: usize,
    pub(super) stat_loc: (Option<u32>, usize),
    /// Instance scope of the registering process: every instance of a
    /// module or interface shares the assertion's span.
    pub(super) dedup_scope: String,
    /// 0 assert, 1 assume, 2 cover (§16.5): a cover miss is not a failure
    /// and fires no else-action; a cover match fires the pass action.
    pub(super) kind: u8,
    /// §16.14.3 `cover sequence`: an attempt counts every match of the
    /// sequence and lives on while it can still match.
    pub(super) all_matches: bool,
    /// The first term's simple signal, if any, for sampled calls using the
    /// single-signal history path. `events` drives assertion evaluation.
    pub(super) clock_signal: String,
    /// The first term's edge: 0 posedge, 1 negedge, 2 any change.
    pub(super) edge: u8,
    /// §16.5: a tick is the union of the per-term edge/iff events.
    pub(super) events: Vec<SvaClockEvent>,
    /// The bare signal names `$past` reads in the body (sorted, unique), found
    /// once at registration: the walk ran on every clock fire.
    pub(super) past_names: Vec<String>,
    /// The body compiled to an evaluable property tree (see `SvaNode`).
    pub(super) node: std::sync::Arc<SvaNode>,
    /// `node` holds an `accept_on` / `reject_on` (§16.12.14), whose
    /// condition is checked in every time step, not only at the ticks.
    pub(super) async_abort: bool,
    /// §16.12.14: attempts an asynchronous abort settled in a time step
    /// between ticks, reported at the next tick (or at the end of
    /// simulation) ahead of that tick's results.
    pub(super) settled: Vec<SvaOutcome>,
    /// §16.12 `disable iff (guard)`: while the guard holds, every attempt in
    /// flight is cancelled and no new one starts. Evaluated with the current
    /// (unsampled) values, as the LRM's asynchronous reset semantics require.
    pub(super) disable: Option<Expression>,
    /// The guard has no calls or other observable effects while idle.
    #[serde(skip)]
    pub(super) quiet_disable: bool,
    /// The clock event happened and the attempts it starts have not been
    /// evaluated yet (`sva_note_edges`).
    pub(super) fired: bool,
    /// Attempts still in flight, oldest first.
    pub(super) attempts: Vec<SvaState>,
    /// IEEE 1800-2017 Clause 39 (assertion API): what the VPI reports of
    /// each attempt in `attempts`, in the same order (`sva_take_meta` pads
    /// it, so the engine never depends on it). Not saved: an attempt
    /// restored from a checkpoint reports the restore time as its start.
    #[serde(skip)]
    pub(super) attempt_meta: Vec<SvaAttemptMeta>,
    /// The same for the attempts in `settled`.
    #[serde(skip)]
    pub(super) settled_meta: Vec<SvaAttemptMeta>,
    /// LRM §16.9.3 `$past(<signal>, N)` per-signal value history.
    /// Each entry is `signal_name → ring of sampled values (index 0 =
    /// this cycle, index N = N cycles ago)`. Refreshed at every clock fire
    /// BEFORE any attempt advances.
    pub(super) past_snapshots: HashMap<String, std::collections::VecDeque<Value>>,
    /// LRM §16.5 pass/fail action blocks (`assert property (...) <pass>
    /// else <fail>;`). Run when the property tallies a non-vacuous
    /// pass / a fail at a clock fire — so a failing concurrent assertion
    /// actually reports (e.g. its `else $error(...)`), not just tallies.
    pub(super) pass_action: Option<Statement>,
    pub(super) fail_action: Option<Statement>,
    /// Instance scope the assertion is declared in, and the pid its action
    /// blocks run under, so `%m` and severity messages name that scope
    /// rather than whichever process ran last.
    pub(super) scope: String,
    pub(super) action_pid: usize,
    /// §21.2.1.7: the lexical chain below `scope` — enclosing named blocks
    /// and the assertion's own label — that `%m` and severity messages in
    /// the action blocks name (`tb.ap`).
    pub(super) m_chain: Vec<String>,
    /// LRM §16.5.1 — signal ids referenced anywhere in `body`. Their
    /// slot-entry (Preponed) values are cached in `Simulator::sva_preponed`
    /// and swapped in while this site's predicate is evaluated, so the
    /// property samples pre-edge values through the sampled-value overlay.
    pub(super) sampled_ids: Vec<usize>,
    /// §16.5.1: the selects of unpacked arrays in `body` whose indices read
    /// signals (`a[k]`), each with its array's (first cell id, low index,
    /// high index) when it selects an element of a 1-D array. The cells they
    /// name are renamed in each Preponed region and kept in `sampled_ids`
    /// after its first `fixed` entries.
    pub(super) sels: Vec<(Expression, Option<(usize, i64, i64)>)>,
    pub(super) fixed: usize,
    /// §16.10: the initial values of the attempt's local variable slots
    /// (empty when the property has none).
    pub(super) lv_init: SvaEnv,
    /// §16.14.6: a procedural concurrent assertion. An attempt starts only
    /// for an execution of the statement (`matured`), not at every tick.
    pub(super) procedural: bool,
    /// §16.14.6.1: the automatic variables the property reads, captured
    /// into these slots when the statement executes.
    pub(super) captured: Vec<(usize, String)>,
    /// §16.14.6.2: matured instances, each the slot values it captured,
    /// waiting for the next tick of the leading clock.
    pub(super) matured: Vec<SvaEnv>,
    /// The time of the last tick. An instance that matures later in that
    /// time step starts its attempt at that tick, not the next one.
    pub(super) tick_time: Option<u64>,
    /// The property has its own `disable iff` (§16.15: the scope's
    /// `default disable iff` does not apply).
    pub(super) explicit_disable: bool,
    /// The full path of the scope the assertion executes in, which selects
    /// the `default disable iff` that governs it.
    pub(super) disable_scope: String,
    /// §20.12: the site's identity for assertion control, and its state.
    pub(super) ctl: CtlSite,
}

/// A sequence (§16.9) as parsed, before it is turned into an automaton.
#[derive(Debug, Clone)]
pub(super) enum SvaSeq {
    /// A boolean: holds at one cycle.
    Bool(Expression),
    /// `##[min:max]` (`max == u64::MAX` unbounded).
    Delay(u64, u64),
    Concat(Vec<SvaSeq>),
    Or(Box<SvaSeq>, Box<SvaSeq>),
    And(Box<SvaSeq>, Box<SvaSeq>),
    Intersect(Box<SvaSeq>, Box<SvaSeq>),
    Throughout(Expression, Box<SvaSeq>),
    Within(Box<SvaSeq>, Box<SvaSeq>),
    FirstMatch(Box<SvaSeq>),
    /// `s[*min:max]`.
    Rep {
        seq: Box<SvaSeq>,
        min: u64,
        max: u64,
    },
    /// `b[->min:max]`.
    Goto {
        cond: Expression,
        min: u64,
        max: u64,
    },
    /// `b[=min:max]`.
    NonCon {
        cond: Expression,
        min: u64,
        max: u64,
    },
    /// §16.12.12 `left until right` / `left s_until right`: matches the
    /// tick `right` first matches, provided `left` held at every earlier
    /// tick.
    Until(Box<SvaSeq>, Box<SvaSeq>),
    /// §16.10 / §16.11 match items, run within the cycle: local variable
    /// assignments and subroutine calls (no cycle of its own).
    Assign(Vec<SvaMatchItem>),
}

/// The local variable values of one thread of an attempt (§16.10), indexed
/// by the slots the site's locals were bound to; empty for a site with no
/// locals.
pub(super) type SvaEnv = Vec<Value>;

/// A §16.10 / §16.11 sequence match item.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct SvaMatchItem {
    /// The assigned local `(slot, width, signed)`; `None` for a subroutine
    /// call run for its side effect.
    pub(super) slot: Option<(usize, u32, bool)>,
    pub(super) expr: Expression,
}

/// Sequence automaton: a cycle advances over `Tick` edges; every other edge
/// is followed within the cycle. The accept state is reached at the cycle a
/// match ends.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct SvaNfa {
    pub(super) states: Vec<Vec<SvaEdge>>,
    pub(super) start: usize,
    pub(super) accept: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) enum SvaEdge {
    Eps(usize),
    /// Follow when the boolean holds at this cycle.
    Check(Expression, usize),
    /// Follow at the next cycle.
    Tick(usize),
    /// A compound sub-sequence: followed at every cycle it reports a match.
    Sub(Box<SvaCompound>, usize),
    /// Run the match items, then follow (§16.10 / §16.11).
    Assign(Vec<SvaMatchItem>, usize),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) enum SvaCompound {
    /// Both match; ends when the later one ends (§16.9.5).
    And(SvaNfa, SvaNfa),
    /// Both match and end at the same cycle (§16.9.6).
    Intersect(SvaNfa, SvaNfa),
    /// The boolean holds at every cycle of the sequence (§16.9.9).
    Throughout(Expression, SvaNfa),
    /// Only the earliest match (§16.9.7).
    FirstMatch(SvaNfa),
    /// §16.7: a counted cycle delay, with no state unrolling or clipping.
    Delay(u64, u64),
    /// §16.9.2: counted copies of the unit, one cycle apart.
    Repeat(SvaNfa, u64, u64),
    /// §16.12.12 `left until right` / `left s_until right`: matches the
    /// tick `right` first matches, provided `left` held at every earlier
    /// tick.
    Until { left: SvaNfa, right: SvaNfa },
}

/// A sequence in flight: `pcs` are the states waiting for the next cycle,
/// `subs` the compound edges being followed.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct SvaRun {
    pub(super) started: bool,
    /// The states waiting for the next cycle, each with its thread's local
    /// variables (§16.10).
    pub(super) pcs: Vec<(usize, SvaEnv)>,
    pub(super) subs: Vec<SvaSubRun>,
    /// The local variables the run starts with.
    pub(super) init_env: SvaEnv,
}

impl SvaRun {
    pub(super) fn with_env(init_env: SvaEnv) -> Self {
        SvaRun {
            init_env,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct SvaSubRun {
    /// `(state, edge)` of the `Sub` edge in the enclosing automaton.
    pub(super) at: (usize, usize),
    pub(super) target: usize,
    pub(super) runs: Vec<SvaRun>,
    /// Which operand of an `and` has matched so far.
    pub(super) ever: [bool; 2],
    /// The local variables the compound was entered with, and those of each
    /// operand's latest match (§16.10: an operand's assignments flow out).
    pub(super) env: SvaEnv,
    pub(super) ever_env: [SvaEnv; 2],
    pub(super) ticks: u64,
    pub(super) repeated: Vec<(u64, SvaRun)>,
}

/// A property body compiled for evaluation (§16.12).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) enum SvaNode {
    /// A sequence used as a property: holds iff it has a match. `strong`
    /// (§16.12.2): an attempt still in flight at the end of simulation fails.
    Seq(SvaNfa, bool),
    /// `not p`.
    Not(Box<SvaNode>),
    /// `ante |-> cons` (overlap) / `ante |=> cons`: every match of the
    /// antecedent starts one evaluation of the consequent.
    Impl {
        ante: SvaNfa,
        overlap: bool,
        cons: Box<SvaNode>,
    },
    /// §16.12.9: at least one antecedent match has a successful consequent.
    Followed {
        ante: SvaNfa,
        overlap: bool,
        cons: Box<SvaNode>,
    },
    And(Box<SvaNode>, Box<SvaNode>),
    Or(Box<SvaNode>, Box<SvaNode>),
    Iff(Box<SvaNode>, Box<SvaNode>),
    /// §16.12.8 `p implies q`: both start at the same tick; holds when `p`
    /// fails (vacuously, §16.14.8 z) or `q` holds. State: `SvaState::And`.
    Implies(Box<SvaNode>, Box<SvaNode>),
    /// §16.12.12: arbitrary properties are started at each candidate tick.
    Until {
        left: Box<SvaNode>,
        right: Box<SvaNode>,
        strong: bool,
    },
    /// §16.12.6 `s_eventually e`: holds once `e` samples true.
    Eventually(Expression),
    /// §16.12.6 `s_always e`: fails the first tick `e` samples false.
    Always(Expression),
    /// §16.12.14 `accept_on (cond) body` / `reject_on (cond) body`
    /// (`accept`), and their `sync_` forms (`sync`): `cond` holding during
    /// the evaluation of `body` aborts it, with success or failure.
    Abort {
        cond: Expression,
        accept: bool,
        sync: bool,
        body: Box<SvaNode>,
    },
}

/// IEEE 1800-2017 Clause 39: the per-attempt data of the assertion API
/// (`s_vpi_attempt_info`), kept beside the attempt's evaluation state.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct SvaAttemptMeta {
    /// Simulation time (ticks) of the clock tick that started the attempt.
    pub(super) start_time: u64,
    /// Clock ticks the attempt has been evaluated at.
    pub(super) clock_steps: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum SvaOutcome {
    Pass,
    /// A vacuous success (§16.14.8), including an implication whose
    /// antecedent never matched or whose consequents all passed vacuously.
    Vacuous,
    Fail,
}

/// Per-attempt evaluation state mirroring `SvaNode`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) enum SvaState {
    Seq(SvaRun),
    Not(Box<SvaState>),
    Impl {
        ante: SvaRun,
        ante_alive: bool,
        nonvacuous: bool,
        /// Consequent evaluations `(ticks until they start, state)`.
        cons: Vec<(u32, SvaState)>,
    },
    Followed {
        ante: SvaRun,
        ante_alive: bool,
        nonvacuous: bool,
        cons: Vec<(u32, SvaState)>,
    },
    And {
        a: Box<SvaState>,
        b: Box<SvaState>,
        ra: Option<SvaOutcome>,
        rb: Option<SvaOutcome>,
    },
    Or {
        a: Box<SvaState>,
        b: Box<SvaState>,
        ra: Option<SvaOutcome>,
        rb: Option<SvaOutcome>,
    },
    Until {
        env: SvaEnv,
        /// Only ticks with an evaluation still pending, or whose failure
        /// (*left* failed) settled the attempt, are retained. Ticks whose
        /// left held and right has failed are folded into `nonvacuous` and
        /// dropped, so the list stays bounded by the pending evaluations
        /// (issue #510); the folded ticks keep the prefix-holds invariant.
        ticks: VecDeque<SvaUntilTick>,
        /// Nonvacuity contributed by folded ticks (and live ones, see
        /// `sva_advance_until`): §16.14.8, either operand can be nonvacuous.
        nonvacuous: bool,
        closed: bool,
    },
    /// `s_eventually` / `s_always`, with the attempt's local variables.
    Leaf(SvaEnv),
    /// An abort property's evaluation; `started`: its first tick is past;
    /// `aborted`: its condition settled it.
    Abort {
        body: Box<SvaState>,
        env: SvaEnv,
        started: bool,
        aborted: bool,
    },
}

/// One candidate start point of a property-valued until operand.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct SvaUntilTick {
    pub(super) left: SvaState,
    pub(super) right: SvaState,
    pub(super) left_result: Option<SvaOutcome>,
    pub(super) right_result: Option<SvaOutcome>,
}

/// Thompson-style construction of `SvaNfa` fragments.
pub(super) struct SvaNfaBuilder {
    states: Vec<Vec<SvaEdge>>,
}

pub(super) enum SvaRepUnit<'a> {
    Seq(&'a SvaSeq),
    Goto(&'a Expression),
}

impl SvaNfaBuilder {
    pub(super) fn state(&mut self) -> usize {
        self.states.push(Vec::new());
        self.states.len() - 1
    }

    pub(super) fn add(&mut self, from: usize, edge: SvaEdge) {
        self.states[from].push(edge);
    }

    pub(super) fn build(seq: &SvaSeq) -> SvaNfa {
        let mut b = SvaNfaBuilder { states: Vec::new() };
        let (start, accept) = b.frag(seq);
        SvaNfa {
            states: b.states,
            start,
            accept,
        }
    }

    fn not_of(e: &Expression) -> Expression {
        Expression::new(
            ExprKind::Unary {
                op: crate::ast::expr::UnaryOp::LogNot,
                operand: Box::new(e.clone()),
            },
            e.span,
        )
    }

    pub(super) fn true_lit(span: crate::ast::Span) -> Expression {
        Expression::new(
            ExprKind::Number(crate::ast::expr::NumberLiteral::Integer {
                size: None,
                signed: false,
                base: crate::ast::expr::NumberBase::Decimal,
                value: "1".to_string(),
                cached_val: crate::ast::cache::LiteralCache::new(None),
            }),
            span,
        )
    }

    /// One unit of a repetition: the repeated sequence, or one `b[->1]`
    /// occurrence (`!b` cycles then a `b` cycle).
    pub(super) fn unit(&mut self, u: &SvaRepUnit<'_>) -> (usize, usize) {
        match u {
            SvaRepUnit::Seq(s) => self.frag(s),
            SvaRepUnit::Goto(cond) => {
                let s = self.state();
                let x = self.state();
                let e = self.state();
                self.add(s, SvaEdge::Check(Self::not_of(cond), x));
                self.add(x, SvaEdge::Tick(s));
                self.add(s, SvaEdge::Check((*cond).clone(), e));
                (s, e)
            }
        }
    }

    /// `unit[*min:max]`: consecutive copies one cycle apart.
    pub(super) fn rep(&mut self, u: &SvaRepUnit<'_>, min: u64, max: u64) -> (usize, usize) {
        // §16.9.2: large counts use run-time counters, preserving the
        // complete unit (including multi-cycle matches and local variables).
        if min > 256 || (max != u64::MAX && max > 256) {
            let unit = match u {
                SvaRepUnit::Seq(seq) => Self::build(seq),
                SvaRepUnit::Goto(cond) => Self::build(&SvaSeq::Goto {
                    cond: (*cond).clone(),
                    min: 1,
                    max: 1,
                }),
            };
            return self.sub(SvaCompound::Repeat(unit, min, max));
        }
        let s = self.state();
        let t = self.state();
        if min == 0 {
            self.add(s, SvaEdge::Eps(t));
        }
        let mut prev_end: Option<usize> = None;
        for _ in 0..min {
            let (cs, ce) = self.unit(u);
            match prev_end {
                None => self.add(s, SvaEdge::Eps(cs)),
                Some(pe) => self.add(pe, SvaEdge::Tick(cs)),
            }
            prev_end = Some(ce);
        }
        if let Some(pe) = prev_end {
            self.add(pe, SvaEdge::Eps(t));
        }
        if max == u64::MAX {
            let (cs, ce) = self.unit(u);
            match prev_end {
                None => self.add(s, SvaEdge::Eps(cs)),
                Some(pe) => self.add(pe, SvaEdge::Tick(cs)),
            }
            self.add(ce, SvaEdge::Eps(t));
            self.add(ce, SvaEdge::Tick(cs));
        } else {
            let max = max.max(min);
            for _ in min..max {
                let (cs, ce) = self.unit(u);
                match prev_end {
                    None => self.add(s, SvaEdge::Eps(cs)),
                    Some(pe) => self.add(pe, SvaEdge::Tick(cs)),
                }
                self.add(ce, SvaEdge::Eps(t));
                prev_end = Some(ce);
            }
        }
        (s, t)
    }

    pub(super) fn frag(&mut self, seq: &SvaSeq) -> (usize, usize) {
        match seq {
            SvaSeq::Bool(e) => {
                let s = self.state();
                let t = self.state();
                self.add(s, SvaEdge::Check(e.clone(), t));
                (s, t)
            }
            SvaSeq::Delay(min, max) => {
                if *min > 4096 || (*max != u64::MAX && *max > 4096) {
                    return self.sub(SvaCompound::Delay(*min, *max));
                }
                let s = self.state();
                let mut cur = s;
                for _ in 0..*min {
                    let n = self.state();
                    self.add(cur, SvaEdge::Tick(n));
                    cur = n;
                }
                if *max == u64::MAX {
                    self.add(cur, SvaEdge::Tick(cur));
                    (s, cur)
                } else {
                    let t = self.state();
                    self.add(cur, SvaEdge::Eps(t));
                    for _ in *min..(*max).max(*min) {
                        let n = self.state();
                        self.add(cur, SvaEdge::Tick(n));
                        self.add(n, SvaEdge::Eps(t));
                        cur = n;
                    }
                    (s, t)
                }
            }
            SvaSeq::Concat(items) => {
                if items.is_empty() {
                    let s = self.state();
                    return (s, s);
                }
                let (s0, mut prev_end) = self.frag(&items[0]);
                for it in &items[1..] {
                    let (si, ei) = self.frag(it);
                    self.add(prev_end, SvaEdge::Eps(si));
                    prev_end = ei;
                }
                (s0, prev_end)
            }
            SvaSeq::Or(a, b) => {
                let s = self.state();
                let t = self.state();
                let (sa, ea) = self.frag(a);
                let (sb, eb) = self.frag(b);
                self.add(s, SvaEdge::Eps(sa));
                self.add(s, SvaEdge::Eps(sb));
                self.add(ea, SvaEdge::Eps(t));
                self.add(eb, SvaEdge::Eps(t));
                (s, t)
            }
            SvaSeq::And(a, b) => self.sub(SvaCompound::And(Self::build(a), Self::build(b))),
            SvaSeq::Intersect(a, b) => {
                self.sub(SvaCompound::Intersect(Self::build(a), Self::build(b)))
            }
            SvaSeq::Throughout(e, a) => {
                self.sub(SvaCompound::Throughout(e.clone(), Self::build(a)))
            }
            SvaSeq::Until(a, b) => self.sub(SvaCompound::Until {
                left: Self::build(a),
                right: Self::build(b),
            }),
            SvaSeq::Within(a, b) => {
                // §16.9.10: `(1[*0:$] ##1 a ##1 1[*0:$]) intersect b`.
                let span = crate::ast::Span::dummy();
                let padded = SvaSeq::Concat(vec![
                    SvaSeq::Delay(0, u64::MAX),
                    (**a).clone(),
                    SvaSeq::Delay(0, u64::MAX),
                    SvaSeq::Bool(Self::true_lit(span)),
                ]);
                self.sub(SvaCompound::Intersect(Self::build(&padded), Self::build(b)))
            }
            SvaSeq::FirstMatch(a) => self.sub(SvaCompound::FirstMatch(Self::build(a))),
            SvaSeq::Rep { seq, min, max } => self.rep(&SvaRepUnit::Seq(seq), *min, *max),
            SvaSeq::Goto { cond, min, max } => self.rep(&SvaRepUnit::Goto(cond), *min, *max),
            SvaSeq::Assign(items) => {
                let s = self.state();
                let t = self.state();
                self.add(s, SvaEdge::Assign(items.clone(), t));
                (s, t)
            }
            SvaSeq::NonCon { cond, min, max } => {
                // §16.9.2: `b[->min:max] ##1 !b[*0:$]`.
                let (s, e) = self.rep(&SvaRepUnit::Goto(cond), *min, *max);
                let t = self.state();
                let f = self.state();
                self.add(e, SvaEdge::Eps(t));
                self.add(e, SvaEdge::Tick(f));
                self.add(f, SvaEdge::Check(Self::not_of(cond), e));
                (s, t)
            }
        }
    }

    pub(super) fn sub(&mut self, c: SvaCompound) -> (usize, usize) {
        let s = self.state();
        let t = self.state();
        self.add(s, SvaEdge::Sub(Box::new(c), t));
        (s, t)
    }
}

/// Per-source-location running tally for `assert` / `assume` / `cover`,
/// keyed by `Simulator::assert_stat_key` (source file and span start) so
/// each distinct source position is tracked once across all executions.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct AssertionStat {
    /// "assert" / "assume" / "cover" — one of three kinds
    pub(super) kind: u8,
    /// Where the statement is: its source file index (when known) and the
    /// span start in that file's preprocessed text.
    pub(super) loc: Option<(Option<u32>, usize)>,
    /// Times the predicate evaluated true (`cover` calls these "hits").
    pub(super) pass_count: u64,
    /// Times the predicate evaluated false (`cover` ignores).
    pub(super) fail_count: u64,
}

/// One `$rose/$fell/$stable/$changed/$past(x, @(edge clk))` call site.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct SampledWatch {
    /// A simple clock id, or usize::MAX when `clock_events` supplies it.
    pub(super) clk_id: usize,
    pub(super) edge: EdgeKind,
    /// A union, guarded, or expression-valued implicit assertion clock.
    pub(super) clock_events: Vec<SvaClockEvent>,
    /// Simple-signal operand: sampled PREPONED via the prev-value snapshot.
    pub(super) sig_id: Option<usize>,
    /// General operand, evaluated using the slot-entry snapshot.
    pub(super) expr: Expression,
    pub(super) hist: std::collections::VecDeque<Value>,
    /// §16.9.3: only qualifying sampled clock ticks enter this history.
    pub(super) gate: Option<Expression>,
    /// An action's gating expression is evaluated in its Reactive context.
    pub(super) action_gate: bool,
    pub(super) pending_sample: Option<(u64, Value)>,
    pub(super) sample_ids: Vec<usize>,
    pub(super) slot_values: Vec<(usize, Value)>,
    pub(super) depth: usize,
    /// Time of the most recent sample. A PROCEDURAL `$past` runs before this
    /// slot's sampler (an SVA site after it), so the reader checks whether the
    /// current edge is already at the front of `hist` before indexing.
    pub(super) last_sample_time: Option<u64>,
    /// §16.5.1: a simple-signal operand's value at the START of the time
    /// slot (`slot_val` taken at `slot_time`'s first sampler pass, when the
    /// delta snapshot still holds the previous slot's final values). A clock
    /// that edges in a LATER delta of the slot than an operand change (a
    /// port clock driven through a continuous assignment, `.ck(a & en)`)
    /// must still sample the operand's preponed value, not the
    /// previous delta's.
    pub(super) slot_val: Option<Value>,
    pub(super) slot_time: Option<u64>,
    /// §16.9.3 default sampled value — what the operand held before time 0
    /// (its declaration initializer, else its type's default) — used for a
    /// tick before the first one. Known only for a watch registered before
    /// the run starts.
    pub(super) default: Option<Value>,
}
