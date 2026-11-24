//! §18.5 — joint constraint solving for `randomize()`.
//!
//! The propagation solver in `exec_randomize_inner` draws every variable and
//! then repairs one constraint at a time. Constraints that couple many
//! variables at once — an array `sum()` together with `unique` and an
//! ordering chain — defeat it: each repair breaks another constraint, and
//! every trial fails although the problem is feasible.
//!
//! This solver treats the constraint set as one problem. Every rand scalar
//! and every element of a rand fixed-size or (already sized) dynamic array is
//! an integer variable with an interval-list domain. Constraints translate to
//! linear relations (`==`, `!=`, `<=` over `sum()`, `+`, `-`, `*` by a
//! constant, casts), set membership (`inside`, `dist`), all-different
//! (`unique`) and conditionals (`->`, `if`/`else`, `||`); anything else is
//! judged by the ordinary evaluator once every variable it reads is fixed.
//! A linear relation is only propagated while no operand can wrap at its SV
//! width and signedness (its "fits"), so bounds reasoning never prunes a
//! value the evaluator would accept. Bounds are propagated to a fixpoint and
//! a depth-first search with random variable tie-breaks and random value
//! choice assigns the variables, so solutions spread over the space. A node
//! budget bounds the work: exhausting the search proves infeasibility,
//! running out of budget gives up and leaves the caller's trials in charge.
//!
//! Distribution (§18.5.4, §18.5.10): a variable under a `dist` in force is
//! decided before the others and draws a dist item in proportion to its
//! weight among the items the current domain still meets, then a value of
//! that item; `solve a before b` decides `a` first, uniformly over its
//! feasible values. A variable read by an `if`/`->` condition weighs each
//! value of a small domain by the product of the other domains after
//! propagating it, so an antecedent that pins a wide consequent (`s -> d ==
//! 0`) comes up in proportion to the solutions it leaves.
use super::*;
use crate::ast::decl::DistWeight;
use crate::compiler::elaborate::{is_type_signed, resolve_type_width};
use rand::Rng;

mod obj;
mod wide;

mod arithmetic;
mod items;
mod propagation;
mod relations;
mod search;
mod slices;
mod solve;
mod subproblems;

thread_local! {
    /// Set while a `std::randomize` (scope) solve runs: the solver's
    /// evaluation frames then extend the caller's locals instead of hiding
    /// them (see `csp_frame`).
    static CSP_SCOPE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Set while `csp_split_slices` scans the constraints: an array element
    /// whose index is not yet known (a `foreach` index) stands for every
    /// element, since all elements share one shape.
    static CSP_SCAN_ANY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Nesting of `csp_dyn_expand` (a select whose index itself holds one).
    static CSP_DYN_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// Indices of the fixed-shape `foreach` loops `csp_split_slices` scans
    /// one index at a time (classes with wide variables).
    static CSP_SCAN_BINDS: std::cell::RefCell<Vec<(String, Value)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// The variables whose whole value `$countones` reads, found while
    /// `csp_split_slices` scans the constraints (see `Csp::pop`).
    static CSP_SCAN_POPS: std::cell::RefCell<Vec<usize>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// Bit ranges of two wide variables a relation compares, found while
    /// `csp_split_slices` scans the constraints: (v1, lsb1, v2, lsb2,
    /// width). `csp_wide_split` cuts both alike over that range.
    /// Set while `csp_dyn_cases` builds its guard on the whole relation:
    /// a select of a wide variable at a position not known yet then
    /// depends on all of its segments (`csp_wide_deps`).
    static CSP_DYN_WHOLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static CSP_SCAN_LINKS: std::cell::RefCell<Vec<(usize, u32, usize, u32, u32)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The scan's evaluation scope: the bound `foreach` indices, if any.
fn csp_scan_env() -> Env {
    CSP_SCAN_BINDS.with(|b| {
        let b = b.borrow();
        if b.is_empty() {
            Env::default()
        } else {
            Env {
                binds: b.clone(),
                ..Env::default()
            }
        }
    })
}

/// Sorted, disjoint, inclusive integer intervals.
type Dom = Vec<(i128, i128)>;

/// Search budget: decisions plus backtracks, over every restart.
const NODE_BUDGET: u64 = 40_000;
/// Constraint executions allowed in one solve (propagation work).
const WORK_BUDGET: u64 = 1_000_000;
/// Propagation work allowed for weighing values of condition variables.
const EST_BUDGET: u64 = 200_000;
/// Largest domain whose values are weighed one by one.
const EST_MAX_DOM: u128 = 64;
/// Budget for auxiliary slice expansion, independent of declared inputs.
const MAX_VARS: usize = 8192;
/// Largest size the size phase gives a dynamic array (§18.5.8.1).
const SIZE_TOP: i128 = 1 << 20;
/// Sizes drawn above the lower bound of a size left open above.
const SIZE_OPEN: i128 = 64;

fn dom_size(d: &Dom) -> u128 {
    d.iter()
        .fold(0u128, |a, (l, h)| a.saturating_add((h - l) as u128 + 1))
}

fn dom_fixed(d: &Dom) -> Option<i128> {
    (d.len() == 1 && d[0].0 == d[0].1).then(|| d[0].0)
}

fn dom_norm(mut v: Vec<(i128, i128)>) -> Dom {
    v.retain(|(l, h)| l <= h);
    v.sort_unstable();
    let mut out: Dom = Vec::with_capacity(v.len());
    for (l, h) in v {
        if let Some(last) = out.last_mut() {
            if l <= last.1.saturating_add(1) {
                last.1 = last.1.max(h);
                continue;
            }
        }
        out.push((l, h));
    }
    out
}

fn dom_clip(d: &Dom, lo: i128, hi: i128) -> Dom {
    d.iter()
        .filter_map(|&(a, b)| {
            let (a, b) = (a.max(lo), b.min(hi));
            (a <= b).then_some((a, b))
        })
        .collect()
}

fn dom_meet(a: &Dom, b: &Dom) -> Dom {
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < a.len() && j < b.len() {
        let lo = a[i].0.max(b[j].0);
        let hi = a[i].1.min(b[j].1);
        if lo <= hi {
            out.push((lo, hi));
        }
        if a[i].1 < b[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

fn dom_minus(a: &Dom, b: &Dom) -> Dom {
    let mut out = Vec::new();
    let mut j = 0;
    for &(mut lo, hi) in a {
        while j < b.len() && b[j].1 < lo {
            j += 1;
        }
        let mut k = j;
        while lo <= hi {
            if k >= b.len() || b[k].0 > hi {
                out.push((lo, hi));
                break;
            }
            if b[k].0 > lo {
                out.push((lo, b[k].0 - 1));
            }
            if b[k].1 >= hi {
                break;
            }
            lo = b[k].1 + 1;
            k += 1;
        }
    }
    out
}

fn dom_has(d: &Dom, v: i128) -> bool {
    d.iter().any(|&(l, h)| l <= v && v <= h)
}

fn dom_nth(d: &Dom, mut k: u128) -> i128 {
    for &(l, h) in d {
        let n = (h - l) as u128 + 1;
        if k < n {
            return l + k as i128;
        }
        k -= n;
    }
    d.last().map_or(0, |x| x.1)
}

fn floor_div(a: i128, b: i128) -> i128 {
    let q = a / b;
    if a % b != 0 && ((a < 0) != (b < 0)) {
        q - 1
    } else {
        q
    }
}

fn ceil_div(a: i128, b: i128) -> i128 {
    -floor_div(-a, b)
}

/// The value range of a `w`-bit operand of the given signedness.
fn ws_range(w: u32, s: bool) -> (i128, i128) {
    let w = w.clamp(1, 64);
    if s {
        (-(1i128 << (w - 1)), (1i128 << (w - 1)) - 1)
    } else {
        (0, (1i128 << w) - 1)
    }
}

/// `k + Σ c·x` over solver variables.
#[derive(Clone, Debug, Default, PartialEq)]
struct Lin {
    t: Vec<(usize, i128)>,
    k: i128,
}

/// Coefficients stay far below i128 overflow for 64-bit domains.
const COEF_MAX: i128 = 1 << 40;
const KONST_MAX: i128 = 1 << 100;

impl Lin {
    fn konst(k: i128) -> Self {
        Lin { t: Vec::new(), k }
    }
    fn var(v: usize) -> Self {
        Lin {
            t: vec![(v, 1)],
            k: 0,
        }
    }
    fn is_const(&self) -> bool {
        self.t.is_empty()
    }
    fn plus(mut self, o: &Lin, sgn: i128) -> Option<Lin> {
        self.k = self.k.checked_add(o.k.checked_mul(sgn)?)?;
        self.t.extend(o.t.iter().map(|&(v, c)| (v, c * sgn)));
        self.norm()
    }
    fn scale(mut self, c: i128) -> Option<Lin> {
        self.k = self.k.checked_mul(c)?;
        for t in &mut self.t {
            t.1 = t.1.checked_mul(c)?;
        }
        self.norm()
    }
    fn shift(mut self, d: i128) -> Option<Lin> {
        self.k = self.k.checked_add(d)?;
        (self.k.abs() <= KONST_MAX).then_some(self)
    }
    fn norm(mut self) -> Option<Lin> {
        self.t.sort_unstable_by_key(|t| t.0);
        let mut out: Vec<(usize, i128)> = Vec::with_capacity(self.t.len());
        for (v, c) in self.t {
            match out.last_mut() {
                Some(l) if l.0 == v => l.1 += c,
                _ => out.push((v, c)),
            }
        }
        out.retain(|t| t.1 != 0);
        if self.k.abs() > KONST_MAX || out.iter().any(|t| t.1.abs() > COEF_MAX) {
            return None;
        }
        self.t = out;
        Some(self)
    }
}

/// A linear form that must stay inside `[lo, hi]` for the linear reading of
/// a constraint to equal its SV value (no operand wraps or re-signs).
#[derive(Clone, Debug)]
struct Fit {
    lin: Lin,
    lo: i128,
    hi: i128,
    /// Nonzero when leaving `[lo, hi]` only changes the operand's value by a
    /// multiple of `2^modw` (a reinterpretation at `modw` bits, §6.24.1 /
    /// §11.8.2, not an extension). An equality compared at `w <= modw` bits
    /// holds modulo `2^w` either way, so it does not need the fit (see
    /// `csp_wrap_eq`).
    modw: u32,
}

/// How a relation reads its operands at their own width (§11.8.2): `w`
/// bits of signedness `s`, the first `n` fits being the operands' own. For
/// a `Node::Lin` the next two fits are the left and right sides, and its
/// `lin` is `sg·(left - right) + δ`.
#[derive(Clone, Copy, Debug)]
struct Wrap {
    w: u32,
    s: bool,
    n: usize,
    sg: i128,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Rel {
    /// `lin <= 0`
    Le,
    /// `lin == 0`
    Eq,
    /// `lin != 0`
    Ne,
}

#[derive(Clone, Debug)]
enum Node {
    True,
    False,
    Lin {
        lin: Lin,
        rel: Rel,
        fits: Vec<Fit>,
        src: usize,
        sneg: bool,
        /// A relation compared at `w` bits whose first `n` fits are the
        /// operands' own, and whose two sides are the next two fits (see
        /// `Wrap`). An equality does not need the side fits, nor an operand
        /// fit with `modw >= w` (see `csp_wrap_eq`, `St::fits_hold_mod`);
        /// another relation compares the sides' wrapped values
        /// (`csp_wrap_rel`).
        wrap: Option<Wrap>,
    },
    /// `lin inside set` (or not, with `neg`).
    In {
        lin: Lin,
        set: Dom,
        neg: bool,
        fits: Vec<Fit>,
        src: usize,
        /// §11.4.13: the operand is compared at `w` bits of signedness `s`
        /// after its first `n` fits (the operands' own); past them only
        /// its last fit (the operand staying in that type) is left, and a
        /// value that leaves it wraps (`csp_wrap_set`).
        wrap: Option<Wrap>,
    },
    AllDiff(Vec<usize>),
    /// §11.8.2: `u` is `v` wrapped into `w` bits of signedness `s` — `v`'s
    /// bits read unsigned (`v + 2^w` below zero) or signed. Domains map
    /// exactly both ways, so `x < 32'h8000_0010` on an `int x` keeps the
    /// two runs of `x` it allows (`csp_wrap_enforce`).
    Wrap {
        v: usize,
        u: usize,
        w: u32,
        s: bool,
    },
    /// §11.4.3: `p == x * y` for the product variable `p` of a relation
    /// (`LinCx::prod`): bounds `p` by the factors, and once one factor is
    /// fixed bounds the other by `p`.
    Mul {
        p: usize,
        x: usize,
        y: usize,
    },
    /// §20.9: `c == $countones(x)` for a variable `x` of `w` bits and its
    /// count variable `c` (see `Csp::pop`): a fixed `x` fixes `c`.
    Pop {
        x: usize,
        c: usize,
        w: u32,
    },
    /// Judged by the evaluator once every variable of `src` is fixed.
    Eval {
        src: usize,
        neg: bool,
    },
    /// `var OP other` where `other` is an expression the linear model
    /// cannot express (a select of a variable, an element of a state array
    /// indexed by variables, …). Once every variable `other` reads is fixed
    /// it is evaluated and narrows `var` — `==` fixes it — instead of
    /// waiting for `var` to be guessed. `whole` is the original relation
    /// (negated when `neg`), judged once everything is fixed.
    Fun {
        var: usize,
        op: BinaryOp,
        other: usize,
        whole: usize,
        neg: bool,
    },
    And(Vec<Node>),
    Or(Vec<Node>),
    If {
        cond: Box<Node>,
        ncond: Box<Node>,
        then: Box<Node>,
        els: Box<Node>,
    },
}

/// A `dist` on one solver variable (§18.5.4), in force while every guard
/// (the enclosing `if`/`->` conditions) holds.
#[derive(Clone, Debug)]
struct Dist {
    var: usize,
    /// (lo, hi, item weight): `:=` weighs every value of the item, `:/`
    /// the item as a whole.
    items: Vec<(i128, i128, f64)>,
    guards: Vec<Node>,
}

/// Per-solve search tables derived from the translated problem.
struct Aux {
    /// Dists on each variable.
    dists: Vec<Vec<usize>>,
    /// `solve p before v`: the variables decided before each one.
    preds: Vec<Vec<usize>>,
    succs: Vec<Vec<usize>>,
    /// Weighed by its solutions (`csp_weigh`): read by some `if`/`->`
    /// condition, or by the other side of a relation on a `$countones`
    /// count (`$countones(strb) == 2**size`: each size leaves C(w, 2**size)
    /// strobe values).
    guard_var: Vec<bool>,
    /// `succs` from `solve … before` alone, without the orderings the
    /// solver adds itself (`Fun` and `$countones` variables after their
    /// inputs).
    user_succs: Vec<Vec<usize>>,
    /// `$countones` links (`Csp::pop`): the count variable of each counted
    /// variable, and the counted variable and its width of each count.
    pop_c: Vec<Option<usize>>,
    pop_x: Vec<Option<(usize, u32)>>,
    /// The more significant segments decided before each segment of a
    /// variable that is used as a number (`csp_aux`).
    seg_hi: Vec<Vec<usize>>,
    /// A variable held as segments, or a select spanning several of them,
    /// without a dist of its own: fixed by its segments, never decided.
    composite: Vec<bool>,
    /// A variable some unconditional `Fun` relation makes a function of
    /// other variables (`tot == w * (n + 1)`): one value per assignment of
    /// its inputs, so its narrowing is theirs and counts no solutions.
    fun_var: Vec<bool>,
}

#[derive(Clone, Debug)]
enum VarKey {
    Prop(String),
    Elem(String),
    /// A rand scalar of a rand sub-object: (its handle, property).
    Sub(usize, String),
    /// A segment or select of another variable (`csp_split_slices`): never
    /// written back — the variable it is part of is.
    Aux,
}

#[derive(Clone, Debug)]
struct CspVar {
    key: VarKey,
    width: u32,
    signed: bool,
}

#[derive(Clone, Debug)]
enum SrcItem {
    Expr(Expression),
    Item(ConstraintItem),
}

/// The original constraint behind a node, for exact evaluation.
#[derive(Clone, Debug)]
struct Src {
    item: SrcItem,
    binds: Vec<(String, Value)>,
    deps: Vec<usize>,
    /// Reads an array whose size the size phase decides (`Csp::sizes`):
    /// its current contents are not the ones being solved, so it is never
    /// judged by the evaluator.
    stale: bool,
}

#[derive(Clone, Debug)]
struct CspArr {
    /// (index value, variable) in index order.
    elems: Vec<(i64, usize)>,
    width: u32,
    signed: bool,
}

/// A memoized, post-translate `Csp` for one rand-class solve, with the
/// fingerprint that produced it. Reused read-only across `randomize()`
/// calls while the fingerprint is unchanged; rebuilt on any change.
pub(crate) struct RandCspMemo {
    /// The built + translated problem. `csp_run` only ever reads the `Csp`
    /// (solutions are written to the simulator via `csp_write`), so a cached
    /// one is safe to share across solves. The `handle` is refreshed to the
    /// current object before each reuse (it is a writeback target, not part
    /// of the translated graph).
    pub(crate) csp: Csp,
    /// Snapshot of the non-rand scalar configuration properties read by the
    /// constraints (the value-dependent part `csp_translate` folds in). A
    /// cached graph is reused only while every stored value still equals the
    /// live object's current value; any change invalidates it. The HashMap
    /// key (a deterministic structural string) is the structural side of the
    /// identity; this list is the value side.
    pub(crate) config: Box<[(String, Value)]>,
}

/// The translated problem.
pub(crate) struct Csp {
    handle: usize,
    vars: Vec<CspVar>,
    dom0: Vec<Dom>,
    scalars: HashMap<String, usize>,
    arrays: HashMap<String, CspArr>,
    /// §18.4 (#519): the multi-dimensional arrays of the solve, by the name
    /// the constraints write, with their bounds outermost first. Their rows
    /// (all but the last dimension indexed) are `arrays` entries.
    nd_dims: HashMap<String, Vec<(i64, i64)>>,
    /// Of `nd_dims`' arrays, the number of leading dimensions that are
    /// dynamic, stored as `(0, size - 1)`: `(0, -1)` there is empty, not a
    /// descending range.
    nd_dyn: HashMap<String, usize>,
    srcs: Vec<Src>,
    nodes: Vec<Node>,
    /// Nodes added by top-level soft preferences while translating. Their
    /// bounds do not hold if a preference is later dropped (§18.5.14).
    soft_node_indices: Vec<usize>,
    /// None enables all preferences; otherwise only the selected instances.
    soft_enabled: Option<HashSet<usize>>,
    soft_priority: Vec<(usize, usize, usize)>,
    soft_rank: (usize, usize),
    dists: Vec<Dist>,
    /// `solve before after` as variable lists.
    order: Vec<(Vec<usize>, Vec<usize>)>,
    /// Some item is left to a checker that does not judge it (a `foreach`
    /// or `unique` the translation could not model), so a solution is not
    /// fully verified.
    opaque: bool,
    /// Rand sub-object scalars are variables too (`csp_add_sub`).
    has_subs: bool,
    /// Map from each sub-object handle joined via `csp_add_sub` to its solver
    /// member prefix (`"a"`, `"policy[0].policy[1]"`), so paths reaching a
    /// sub-object through handles resolve scalars, arrays and sizes uniformly.
    sub_prefixes: HashMap<usize, String>,
    /// Some scalar is a member of an unpacked struct member (`s.x`).
    dotted: bool,
    /// `std::randomize(…) with {…}` (§18.12): the variables are the
    /// caller's own — written through these lvalues (scalars) and by
    /// element name (arrays) — and the solve runs in the caller's scope.
    scope: Option<HashMap<String, Expression>>,
    /// Constant bit/part selects and packed-struct fields of solver
    /// scalars (§11.5.1, §7.2.1), split into segment variables: base
    /// variable -> its segments (lsb, width, variable), lsb first.
    segs: HashMap<usize, Vec<(u32, u32, usize)>>,
    /// A select spanning several segments: (base, lsb, width) -> variable.
    slice_vars: HashMap<(usize, u32, u32), usize>,
    /// Segment or select variable -> (base, lsb, width), from `segs` and
    /// `slice_vars`.
    aux_base: HashMap<usize, (usize, u32, u32)>,
    /// Packed-struct field layout (name, lsb, width) of a base variable.
    fields: HashMap<usize, Vec<(String, u32, u32)>>,
    /// §7.2.1: the fields of `fields` declared signed, by base variable and
    /// field name: a select of one reads its bits signed (`csp_slice_ae`).
    signed_fields: HashSet<(usize, String)>,
    /// §7.4.5: a field that is a packed array of elements (`bit [63:0][23:0]
    /// f`), keyed by (base, lsb, width): the declared (left, right) of its
    /// first dimension and its element width, so `f[k]` names an element.
    field_elems: HashMap<(usize, u32, u32), (i64, i64, u32)>,
    /// §7.4.1: the declared (left, right) of every packed dimension of a
    /// multi-dimensional packed scalar or array element (`bit [8:0][7:0]
    /// w`), outermost first; its selects go through them (`csp_pdim_ref`).
    pdims: HashMap<usize, Vec<(i64, i64)>>,
    /// §7.4.1/§7.2.1: the field layout (nested fields by dotted path) of
    /// ONE element of a `pdims` variable that is a packed array of packed
    /// structs (`cand_t [0:2] c`): `c[i].f` is field `f` of the element
    /// `csp_pdim_ref` positions (`csp_pdim_field`).
    elem_fields: HashMap<usize, Vec<(String, u32, u32)>>,
    /// Declared lsb of a base variable's packed range (`[31:8]` -> 8).
    lsb: HashMap<usize, i64>,
    /// Variables whose packed labels increase from most to least significant.
    ascending: HashSet<usize>,
    /// Rand scalars and array elements wider than 64 bits that some
    /// constraint reads (`csp_wide_split`). Such a base variable is never
    /// searched (its domain is a dummy fixed value) nor written as a whole:
    /// it is held as segments of at most 64 bits, written into its bits.
    wide: HashSet<usize>,
    /// The segment variables of the `wide` bases.
    wide_seg: HashSet<usize>,
    /// Rand variables wider than 64 bits that no constraint reads (or only
    /// through `size()`): not solver variables; drawn bit by bit when the
    /// solve runs before any trial has drawn them (`draw_free`).
    wide_free: Vec<(VarKey, u32)>,
    draw_free: bool,
    /// A wide variable could not be split (too many segments, a dynamic
    /// select): the problem is not modelled.
    wide_fail: bool,
    /// §20.9: `$countones(x)` of a whole variable of at most 64 bits is an
    /// int-valued count variable in [0, width] (x -> count), tied to `x` by
    /// a `Pop` node. A relation on the count is then an ordinary one —
    /// linear, or a `Fun` node when its other side is not linear
    /// (`$countones(strb) == 2**size`) — and `x` is decided after its count
    /// and drawn with exactly that many ones, instead of being guessed
    /// against a test the bits rarely pass.
    pop: HashMap<usize, usize>,
    /// The reverse of `pop` (count -> x): the evaluator reads `x` itself, so
    /// a source that depends on the count depends on `x` too.
    pop_rev: HashMap<usize, usize>,
    /// §7.12.3: a `sum()` that can wrap at its own width is a variable `s`
    /// tied to its terms by `Σ terms == s + 2^w·k` with a carry variable
    /// `k` (`LinCx`): (s, k, the term variables). Both have small domains
    /// and are decided first: the last open term is then fixed by the
    /// equality instead of guessed against a set of allowed sums.
    carries: Vec<(usize, usize, Vec<usize>)>,
    /// Parallel to `carries`: the sum each pair stands for, at its `(w, s)`.
    /// The same sum read by several relations (`lo <= a.sum()` and
    /// `a.sum() <= hi`, the two halves of an `inside` range with rand
    /// bounds) is ONE value: it reuses one pair. Independent pairs tied to
    /// the same terms let the search decide them inconsistently, a conflict
    /// that only shows at the last term.
    carry_accs: Vec<(Lin, u32, bool)>,
    /// §11.8.2: a variable read at `(w, s)` other than its own type — a
    /// signed one in an unsigned relation, a cast of one — is the variable
    /// `u` tied to it by `Node::Wrap`: (v, w, s, u). One per reading.
    wraps: Vec<(usize, u32, bool, usize)>,
    /// Variable count before translation: the carry variables live above
    /// it and are made afresh by every translation.
    carry_base: Option<usize>,
    /// §18.5.8.1 size phase (`csp_size_phase`): the dynamic arrays whose
    /// size is a variable of this solve, and that variable. Such an array
    /// has no element variables.
    sizes: HashMap<String, usize>,
    /// The problem is the size phase's relaxation: an item it cannot model
    /// is dropped rather than making the solve inapplicable.
    size_phase: bool,
    /// `(x & M) == C` judged by the evaluator because `M` or `C` reads
    /// other variables (`csp_note_mask`): once those are fixed, `x` is drawn
    /// with the masked bits already equal to `C`'s.
    masks: Vec<MaskHint>,
}

/// See `Csp::masks`.
#[derive(Clone, Debug)]
struct MaskHint {
    /// The variable `x` names, and the bits (lsb, width) of it that `x` is.
    var: usize,
    lo: u32,
    w: u32,
    mask: Expression,
    want: Expression,
    /// The variables `mask` and `want` read, and a source carrying the
    /// foreach bindings to evaluate them under.
    deps: Vec<usize>,
    src: usize,
    guards: Vec<Node>,
}

/// Joint sub-object setup built by `Simulator::csp_join_subs` for both
/// `csp_size_phase` and `rand_csp_solve_sized`.
struct SubJoin<'a> {
    _obj_guard: Option<obj::ObjGuard>,
    constraints: std::borrow::Cow<'a, [ClassConstraint]>,
    check: Option<Vec<ClassConstraint>>,
    sub_dynamic: bool,
    held_wide: Vec<(usize, Value)>,
}

enum SizePhaseErr {
    Outcome(CspOutcome),
    SubDynamicUnsat,
}

/// One modular reduction of a relation being translated (see
/// `Csp::carries`): its variables are created only when the relation is.
/// A select of a multi-dimensional packed variable (`csp_pdim_ref`): the
/// bit ranges (lsb, width) it may name, how many dimensions its selects
/// consumed (0: the whole variable; past the last: a part-select, which
/// takes no further select), and whether every index was constant (then
/// `pos` holds exactly one range).
struct PdimRef {
    v: usize,
    pos: Vec<(u32, u32)>,
    depth: usize,
    known: bool,
}

struct Carry {
    acc: Lin,
    w: u32,
    s: bool,
    k: (i128, i128),
    modulus: i128,
    remainder: (i128, i128),
    /// The pair's variables: `s` here, `k` next to it.
    sv: usize,
}

/// §11.4.3: the product `x * y` of two variables, as a variable `p` tied to
/// them by `Node::Mul`, with the bounds `r` of its value.
struct Prod {
    x: usize,
    y: usize,
    r: (i128, i128),
    p: usize,
}

/// §11.8.2: a variable `v` read wrapped into `(w, s)`, as the variable `u`
/// (`Node::Wrap`).
struct Wrp {
    v: usize,
    w: u32,
    s: bool,
    u: usize,
}

/// Largest magnitude a product variable (`Prod`) may take: with
/// coefficients up to `COEF_MAX` its terms stay far below i128 overflow.
const PROD_MAX: i128 = 1 << 80;

/// Translation context of `csp_lin`: the initial domains, and the carries
/// and products of the relation, numbered from `base` in creation order.
struct LinCx<'a> {
    dom0: &'a [Dom],
    nodes: &'a [Node],
    soft_node_indices: &'a [usize],
    base: usize,
    /// The next free variable number.
    next: usize,
    carries: Vec<Carry>,
    prods: Vec<Prod>,
    wraps: Vec<Wrp>,
    /// The readings earlier relations made (`Csp::wraps`).
    made_wraps: &'a [(usize, u32, bool, usize)],
    /// The pairs earlier relations made (`Csp::carries`, `Csp::carry_accs`).
    made: &'a [(usize, usize, Vec<usize>)],
    made_accs: &'a [(Lin, u32, bool)],
}

impl<'a> LinCx<'a> {
    fn new(csp: &'a Csp) -> Self {
        LinCx {
            dom0: &csp.dom0,
            nodes: &csp.nodes,
            soft_node_indices: &csp.soft_node_indices,
            base: csp.vars.len(),
            next: csp.vars.len(),
            carries: Vec::new(),
            prods: Vec::new(),
            wraps: Vec::new(),
            made_wraps: &csp.wraps,
            made: &csp.carries,
            made_accs: &csp.carry_accs,
        }
    }

    /// Bounds of `v`: an existing variable or a pending carry's `s`/`k`.
    fn bounds(&self, v: usize) -> Option<(i128, i128)> {
        if v < self.base {
            let d = self.dom0.get(v)?;
            return Some((d.first()?.0, d.last()?.1));
        }
        if let Some(c) = self.carries.iter().find(|c| c.sv == v || c.sv + 1 == v) {
            return Some(if c.sv == v { c.remainder } else { c.k });
        }
        if let Some(w) = self.wraps.iter().find(|w| w.u == v) {
            return Some(ws_range(w.w, w.s));
        }
        self.prods.iter().find(|p| p.p == v).map(|p| p.r)
    }

    fn range(&self, l: &Lin) -> Option<(i128, i128)> {
        let (mut lo, mut hi) = (l.k, l.k);
        for &(v, c) in &l.t {
            let (a, b) = self.bounds(v)?;
            let (x, y) = (a.checked_mul(c)?, b.checked_mul(c)?);
            lo = lo.checked_add(x.min(y))?;
            hi = hi.checked_add(x.max(y))?;
        }
        Some((lo, hi))
    }

    /// §7.12.3: `acc` accumulated at `(w, s)` wraps when its range leaves
    /// that type: its value is then a new variable (`Csp::carries`). None
    /// when it cannot wrap, or the modulus is too large a coefficient.
    fn carry(&mut self, acc: &Lin, w: u32, s: bool) -> Option<usize> {
        let (lo, hi) = self.range(acc)?;
        let (rlo, rhi) = ws_range(w, s);
        if lo >= rlo && hi <= rhi {
            return None;
        }
        let m = 1i128 << w.min(64);
        if w > 64 || m > COEF_MAX {
            return None;
        }
        // The same sum already has a pair (see `Csp::carry_accs`).
        if let Some(i) = self
            .made_accs
            .iter()
            .position(|(a, cw, cs)| a == acc && *cw == w && *cs == s)
        {
            return Some(self.made[i].0);
        }
        if let Some(i) = self
            .carries
            .iter()
            .position(|c| c.acc == *acc && c.w == w && c.s == s)
        {
            return Some(self.carries[i].sv);
        }
        let k = (floor_div(lo - rlo, m), floor_div(hi - rlo, m));
        let sv = self.next;
        self.next += 2;
        self.carries.push(Carry {
            acc: acc.clone(),
            w,
            s,
            k,
            modulus: m,
            remainder: (rlo, rhi),
            sv,
        });
        Some(sv)
    }

    /// §11.4.3: unsigned division is acc = divisor*q + r, 0 <= r < divisor.
    /// Reuse the carry pair so both quotient and remainder propagate.
    fn quotient(&mut self, acc: &Lin, divisor: i128) -> Option<(usize, usize)> {
        let (lo, hi) = self.range(acc)?;
        if lo < 0 || divisor <= 0 || divisor > COEF_MAX {
            return None;
        }
        if let Some(c) = self
            .carries
            .iter()
            .find(|c| c.acc == *acc && c.modulus == divisor && c.remainder == (0, divisor - 1))
        {
            return Some((c.sv + 1, c.sv));
        }
        let sv = self.next;
        self.next += 2;
        self.carries.push(Carry {
            acc: acc.clone(),
            w: 64,
            s: false,
            k: (lo / divisor, hi / divisor),
            modulus: divisor,
            remainder: (0, divisor - 1),
            sv,
        });
        Some((sv + 1, sv))
    }

    /// Bounds established by earlier unconditional single-variable constraints.
    /// Consult these only when a product's declared domains are too wide.
    fn constrained_bounds(&self, v: usize) -> Option<(i128, i128)> {
        let mut range = self.bounds(v)?;
        fn visit(cx: &LinCx<'_>, v: usize, n: &Node, range: &mut (i128, i128)) {
            let (lin, fits) = match n {
                Node::And(ns) => {
                    for n in ns {
                        visit(cx, v, n, range);
                    }
                    return;
                }
                Node::Lin { lin, fits, .. } | Node::In { lin, fits, .. } => (lin, fits),
                _ => return,
            };
            if !fits.iter().all(|f| {
                cx.range(&f.lin)
                    .is_some_and(|(a, b)| a >= f.lo && b <= f.hi)
            }) {
                return;
            }
            let [(u, c)] = lin.t.as_slice() else {
                return;
            };
            if *u != v || !matches!(*c, -1 | 1) {
                return;
            }
            match n {
                Node::Lin { rel: Rel::Le, .. } if *c == 1 => range.1 = range.1.min(-lin.k),
                Node::Lin { rel: Rel::Le, .. } => range.0 = range.0.max(lin.k),
                Node::Lin { rel: Rel::Eq, .. } => {
                    let k = -lin.k / *c;
                    range.0 = range.0.max(k);
                    range.1 = range.1.min(k);
                }
                Node::In {
                    set, neg: false, ..
                } if !set.is_empty() => {
                    let (a, b) = (set.first().unwrap().0, set.last().unwrap().1);
                    let (a, b) = if *c == 1 {
                        (a - lin.k, b - lin.k)
                    } else {
                        (lin.k - b, lin.k - a)
                    };
                    range.0 = range.0.max(a);
                    range.1 = range.1.min(b);
                }
                _ => {}
            }
        }
        let mut soft = self.soft_node_indices.iter().copied().peekable();
        for (i, n) in self.nodes.iter().enumerate() {
            if soft.next_if_eq(&i).is_none() {
                visit(self, v, n, &mut range);
            }
        }
        (range.0 <= range.1).then_some(range)
    }

    fn constrained_range(&self, lin: &Lin) -> Option<(i128, i128)> {
        let (mut lo, mut hi) = (lin.k, lin.k);
        for &(v, c) in &lin.t {
            let (a, b) = self.constrained_bounds(v)?;
            let (a, b) = (a.checked_mul(c)?, b.checked_mul(c)?);
            lo = lo.checked_add(a.min(b))?;
            hi = hi.checked_add(a.max(b))?;
        }
        Some((lo, hi))
    }

    /// §11.4.3: the product of the variables `x` and `y` as a new variable
    /// (`Prod`), so a relation on `base + cnt * pitch` is linear in it and
    /// propagates once either factor is known. None when the product could
    /// leave `PROD_MAX`.
    fn prod(&mut self, x: usize, y: usize) -> Option<usize> {
        let product = |(xa, xb): (i128, i128), (ya, yb): (i128, i128)| -> Option<(i128, i128)> {
            let cs = [
                xa.checked_mul(ya)?,
                xa.checked_mul(yb)?,
                xb.checked_mul(ya)?,
                xb.checked_mul(yb)?,
            ];
            let r = (*cs.iter().min()?, *cs.iter().max()?);
            (r.0 >= -PROD_MAX && r.1 <= PROD_MAX).then_some(r)
        };
        let r = product(self.bounds(x)?, self.bounds(y)?)
            .or_else(|| product(self.constrained_bounds(x)?, self.constrained_bounds(y)?))?;
        if let Some(p) = self
            .prods
            .iter()
            .find(|p| (p.x, p.y) == (x, y) || (p.x, p.y) == (y, x))
        {
            return Some(p.p);
        }
        let p = self.next;
        self.next += 1;
        self.prods.push(Prod { x, y, r, p });
        Some(p)
    }

    /// §11.8.2: `v` read at `(w, s)`: itself when its values all keep
    /// their value there, else a variable `Node::Wrap` ties to it. None
    /// when `w` exceeds 64 bits.
    fn wrap(&mut self, v: usize, w: u32, s: bool) -> Option<usize> {
        if w == 0 || w > 64 {
            return None;
        }
        let (lo, hi) = self.bounds(v)?;
        let (rlo, rhi) = ws_range(w, s);
        if lo >= rlo && hi <= rhi {
            return Some(v);
        }
        if let Some(&(.., u)) = self
            .made_wraps
            .iter()
            .find(|m| (m.0, m.1, m.2) == (v, w, s))
        {
            return Some(u);
        }
        if let Some(r) = self.wraps.iter().find(|r| (r.v, r.w, r.s) == (v, w, s)) {
            return Some(r.u);
        }
        let u = self.next;
        self.next += 1;
        self.wraps.push(Wrp { v, w, s, u });
        Some(u)
    }

    /// The carries, products and wrapped readings made so far, for
    /// `csp_add_carries`.
    fn take(&mut self) -> (Vec<Carry>, Vec<Prod>, Vec<Wrp>) {
        (
            std::mem::take(&mut self.carries),
            std::mem::take(&mut self.prods),
            std::mem::take(&mut self.wraps),
        )
    }
}

/// Translation scope: bound `foreach` indices and the `with` iterator.
#[derive(Clone, Default)]
struct Env {
    binds: Vec<(String, Value)>,
    /// (iterator name, element variable, element index)
    it: Option<(String, usize, i64)>,
    /// Conditions of the enclosing `if`/`->` items.
    guards: Vec<Node>,
    /// `Some(false)` under a disjunct or soft item the solver may skip.
    mask_note: Option<bool>,
}

impl Env {
    fn guarded(&self, g: &Node) -> Env {
        let mut e = self.clone();
        e.guards.push(g.clone());
        e
    }

    /// The scope with mask hints off; borrowed when they already are, so a
    /// nested disjunct or condition does not copy the scope again.
    fn optional_mask(&self) -> std::borrow::Cow<'_, Env> {
        if self.mask_note == Some(false) {
            return std::borrow::Cow::Borrowed(self);
        }
        let mut e = self.clone();
        e.mask_note = Some(false);
        std::borrow::Cow::Owned(e)
    }
}

/// An arithmetic operand before its context is known (§11.8.2).
#[derive(Clone, Debug)]
enum Ae {
    /// (variable, width, signed)
    Var(usize, u32, bool),
    Const(Value),
    Neg(Box<Ae>),
    Add(Box<Ae>, Box<Ae>),
    Sub(Box<Ae>, Box<Ae>),
    Mul(Box<Ae>, Box<Ae>),
    Shl(Box<Ae>, u64),
    /// Constant unsigned divisor, result selects quotient (false) or remainder.
    Quot(Box<Ae>, Value, bool),
    /// Array reduction accumulated at its own `(w, s)` (§7.12.3).
    Sum {
        terms: Vec<Ae>,
        w: u32,
        s: bool,
    },
    /// Cast: `inner` evaluated in the context `(iw, is)` — never narrower
    /// than the target (§6.24.1) — then converted to `(w, s)`.
    Cast {
        inner: Box<Ae>,
        iw: u32,
        is: bool,
        w: u32,
        s: bool,
    },
}

/// Search state: current domains plus an undo trail.
struct St {
    d: Vec<Dom>,
    trail: Vec<(usize, Dom)>,
    dirty: Vec<usize>,
    work: u64,
    /// Work spent weighing values (`csp_weigh`), outside `work`.
    est_work: u64,
    /// Some branch was cut by the stall guard of `csp_propagate`, not
    /// refuted: exhausting the search then proves nothing.
    cut: bool,
    /// Refuted values allowed per decision level before the level is
    /// abandoned (solves with rand sub-objects; 0 = no limit), and whether
    /// that happened in this run: the run is then retried, not concluded.
    level_tries: u32,
    level_cut: bool,
    /// Inside `csp_probe`: an `if` does not probe its own branches.
    probing: bool,
}

impl St {
    /// Replace a domain; false when it became empty.
    fn set(&mut self, v: usize, nd: Dom) -> bool {
        if nd == self.d[v] {
            return true;
        }
        let empty = nd.is_empty();
        let old = std::mem::replace(&mut self.d[v], nd);
        self.trail.push((v, old));
        self.dirty.push(v);
        !empty
    }
    fn undo(&mut self, mark: usize) {
        while self.trail.len() > mark {
            let (v, old) = self.trail.pop().unwrap();
            self.d[v] = old;
        }
    }
    fn bounds(&self, v: usize) -> (i128, i128) {
        let d = &self.d[v];
        (d.first().map_or(0, |x| x.0), d.last().map_or(-1, |x| x.1))
    }
    fn fixed(&self, v: usize) -> Option<i128> {
        dom_fixed(&self.d[v])
    }
    fn lin_bounds(&self, l: &Lin) -> (i128, i128) {
        let (mut lo, mut hi) = (l.k, l.k);
        for &(v, c) in &l.t {
            let (a, b) = self.bounds(v);
            if c > 0 {
                lo += c * a;
                hi += c * b;
            } else {
                lo += c * b;
                hi += c * a;
            }
        }
        (lo, hi)
    }
    fn fits_hold(&self, fits: &[Fit]) -> bool {
        fits.iter().all(|f| {
            let (lo, hi) = self.lin_bounds(&f.lin);
            lo >= f.lo && hi <= f.hi
        })
    }
    /// `fits_hold` for an equality compared at `w` bits: a fit that only
    /// guards a reinterpretation at `w` bits or more is not needed.
    fn fits_hold_mod(&self, fits: &[Fit], w: u32) -> bool {
        fits.iter().all(|f| {
            if f.modw >= w {
                return true;
            }
            let (lo, hi) = self.lin_bounds(&f.lin);
            lo >= f.lo && hi <= f.hi
        })
    }
    fn all_fixed(&self, vs: &[usize]) -> bool {
        vs.iter().all(|&v| self.fixed(v).is_some())
    }
    /// `lin <= 0` (upper) or `lin >= 0` (lower) by bounds.
    fn prop_bound(&mut self, l: &Lin, upper: bool) -> bool {
        let (lo, hi) = self.lin_bounds(l);
        if (upper && lo > 0) || (!upper && hi < 0) {
            return false;
        }
        for &(v, c) in &l.t {
            let (a, b) = self.bounds(v);
            let (tmin, tmax) = if c > 0 {
                (c * a, c * b)
            } else {
                (c * b, c * a)
            };
            let d = &self.d[v];
            let nd = if upper {
                // c·x <= tmin - lo
                let r = tmin - lo;
                if c > 0 {
                    let h = floor_div(r, c);
                    if h >= b {
                        continue;
                    }
                    dom_clip(d, i128::MIN, h)
                } else {
                    let l2 = ceil_div(r, c);
                    if l2 <= a {
                        continue;
                    }
                    dom_clip(d, l2, i128::MAX)
                }
            } else {
                // c·x >= tmax - hi
                let r = tmax - hi;
                if c > 0 {
                    let l2 = ceil_div(r, c);
                    if l2 <= a {
                        continue;
                    }
                    dom_clip(d, l2, i128::MAX)
                } else {
                    let h = floor_div(r, c);
                    if h >= b {
                        continue;
                    }
                    dom_clip(d, i128::MIN, h)
                }
            };
            if !self.set(v, nd) {
                return false;
            }
        }
        true
    }
}

impl Node {
    fn deps(&self, srcs: &[Src], out: &mut Vec<usize>) {
        match self {
            Node::True | Node::False => {}
            Node::Lin { lin, fits, src, .. } | Node::In { lin, fits, src, .. } => {
                out.extend(lin.t.iter().map(|t| t.0));
                for f in fits {
                    out.extend(f.lin.t.iter().map(|t| t.0));
                }
                out.extend(srcs[*src].deps.iter().copied());
            }
            Node::AllDiff(vs) => out.extend(vs.iter().copied()),
            Node::Pop { x, c, .. } => out.extend([*x, *c]),
            Node::Mul { p, x, y } => out.extend([*p, *x, *y]),
            Node::Wrap { v, u, .. } => out.extend([*v, *u]),
            Node::Eval { src, .. } => out.extend(srcs[*src].deps.iter().copied()),
            Node::Fun {
                var, other, whole, ..
            } => {
                out.push(*var);
                out.extend(srcs[*other].deps.iter().copied());
                out.extend(srcs[*whole].deps.iter().copied());
            }
            Node::And(ns) | Node::Or(ns) => ns.iter().for_each(|n| n.deps(srcs, out)),
            Node::If {
                cond, then, els, ..
            } => {
                cond.deps(srcs, out);
                then.deps(srcs, out);
                els.deps(srcs, out);
            }
        }
    }
}

/// Outcome of one joint solve.
#[derive(Debug)]
pub(super) enum CspOutcome {
    /// A verified solution was written to the object.
    Sat,
    /// The search space was exhausted: no solution exists.
    Unsat,
    /// Budget exhausted (or the verifier disagreed); nothing is known.
    GaveUp,
    /// The constraint set uses something this solver does not model.
    NotApplicable,
}

#[cfg(test)]
mod state_collection_tests {
    use super::*;

    #[test]
    fn joint_solver_bounds_index_by_referenced_state_array_size() {
        let mut sim = crate::simulate(
            r#"class config_t;
                int ranges[];
                function new(); ranges=new[1]; endfunction
            endclass
            class txn;
                config_t cfg=new();
                rand int unsigned idx;
                rand bit[63:0] unrelated;
                constraint c { if(cfg.ranges.size()>0) idx<cfg.ranges.size(); }
            endclass
            module top; initial begin txn t=new(); $finish; end endmodule"#,
            1,
        )
        .unwrap();
        let (handle, class_name) = sim
            .heap
            .iter()
            .enumerate()
            .find_map(|(h, o)| {
                o.as_ref()
                    .filter(|o| o.class_name.ends_with("txn"))
                    .map(|o| (h, o.class_name.clone()))
            })
            .unwrap();
        let constraints: Vec<_> = sim.module.classes[&class_name]
            .constraints
            .values()
            .map(|c| ClassConstraint::clone(c))
            .collect();
        sim.this_stack.push(Some(handle));
        sim.class_context_stack.push(Some(class_name));
        let mut csp = sim
            .csp_vars(
                handle,
                &[("idx".into(), 32), ("unrelated".into(), 64)],
                &HashSet::default(),
                &HashMap::default(),
                &[],
                &HashMap::default(),
                &HashSet::default(),
                false,
                &HashSet::default(),
            )
            .unwrap();
        sim.csp_translate(&mut csp, &constraints, &[]).unwrap();
        let state = sim
            .csp_find(&csp)
            .expect("state-array size must narrow the index without search exhaustion");
        assert_eq!(state.fixed(csp.scalars["idx"]), Some(0));
    }
}
