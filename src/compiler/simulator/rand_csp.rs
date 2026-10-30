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
use crate::compiler::elaborate::{is_type_real, is_type_signed, resolve_type_width};
use rand::RngExt;

mod obj;

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
/// Largest variable count the solver takes on.
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

/// The translated problem.
struct Csp {
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
    /// §7.4.5: a field that is a packed array of elements (`bit [63:0][23:0]
    /// f`), keyed by (base, lsb, width): the declared (left, right) of its
    /// first dimension and its element width, so `f[k]` names an element.
    field_elems: HashMap<(usize, u32, u32), (i64, i64, u32)>,
    /// §7.4.1: the declared (left, right) of every packed dimension of a
    /// multi-dimensional packed scalar or array element (`bit [8:0][7:0]
    /// w`), outermost first; its selects go through them (`csp_pdim_ref`).
    pdims: HashMap<usize, Vec<(i64, i64)>>,
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
}

/// Translation context of `csp_lin`: the initial domains, and the carries
/// of the relation, numbered from `base`.
struct LinCx<'a> {
    dom0: &'a [Dom],
    base: usize,
    carries: Vec<Carry>,
    /// The pairs earlier relations made (`Csp::carries`, `Csp::carry_accs`).
    made: &'a [(usize, usize, Vec<usize>)],
    made_accs: &'a [(Lin, u32, bool)],
}

impl<'a> LinCx<'a> {
    fn new(csp: &'a Csp) -> Self {
        LinCx {
            dom0: &csp.dom0,
            base: csp.vars.len(),
            carries: Vec::new(),
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
        let c = self.carries.get((v - self.base) / 2)?;
        Some(if (v - self.base) % 2 == 0 {
            ws_range(c.w, c.s)
        } else {
            c.k
        })
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
            return Some(self.base + 2 * i);
        }
        let k = (floor_div(lo - rlo, m), floor_div(hi - rlo, m));
        self.carries.push(Carry {
            acc: acc.clone(),
            w,
            s,
            k,
        });
        Some(self.base + 2 * (self.carries.len() - 1))
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
}

impl Env {
    fn guarded(&self, g: &Node) -> Env {
        let mut e = self.clone();
        e.guards.push(g.clone());
        e
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

impl Simulator {
    /// Solve the constraint set of `handle` jointly (see the module docs).
    /// `rand_props` are the scalar rand properties, `colls` the rand
    /// collections (dynamic ones already sized), `array_enums` the enum type
    /// of enum-typed fixed-array elements. `strict` gives up on a set the
    /// final check cannot fully verify.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn rand_csp_solve(
        &mut self,
        handle: usize,
        constraints: &[ClassConstraint],
        constraint_depth: &[usize],
        rand_props: &[(String, u32)],
        signed_props: &HashSet<String>,
        enum_props: &HashMap<String, String>,
        colls: &[RandColl],
        array_enums: &HashMap<String, String>,
        strict: bool,
        subs: &[(String, usize)],
    ) -> CspOutcome {
        // §18.5.8.1: the size of a dynamic array is solved before its
        // elements, jointly with the variables its size constraints tie it
        // to (`beats.size() == cmd.count + 1`). The sizes are a solve of
        // their own (`csp_size_phase`); the elements are then solved with
        // the arrays at those sizes. A size that leaves the elements without
        // a solution is drawn again a few times.
        // §7.4.5: the rows of a dynamic member over unpacked rows exist only
        // at its current size; they are expanded again whenever it changes,
        // and their own sizes (`a[i].size()`) are solved once they exist.
        let has_rows = colls.iter().any(|c| c.row_tail.is_some());
        let mut cur: Vec<RandColl> = if has_rows {
            self.csp_expand_rows(colls)
        } else {
            colls.to_vec()
        };
        let sized_of = |cs: &[RandColl]| -> HashSet<String> {
            if subs.is_empty() {
                cs.iter()
                    .filter(|c| Self::csp_size_solved(c, constraints))
                    .map(|c| c.prop.clone())
                    .collect()
            } else {
                HashSet::default()
            }
        };
        let mut sized = sized_of(&cur);
        if sized.is_empty() {
            return self.rand_csp_solve_sized(
                handle,
                constraints,
                constraint_depth,
                rand_props,
                signed_props,
                enum_props,
                &cur,
                array_enums,
                strict,
                subs,
            );
        }
        let mut rounds = 0;
        let mut solves = 0;
        while rounds < 8 && solves < 4 {
            rounds += 1;
            match self.csp_size_phase(
                handle,
                constraints,
                constraint_depth,
                rand_props,
                signed_props,
                enum_props,
                &cur,
                array_enums,
                &sized,
            ) {
                Ok(sizes) => {
                    let sizes: Vec<(RandColl, u64)> =
                        sizes.into_iter().map(|(c, n)| (c.clone(), n)).collect();
                    for (c, n) in sizes {
                        if c.row_tail.is_some() {
                            // Its elements are rows, not values.
                            self.resize_row_owner(&c, n);
                            self.module.dynamic_arrays.insert(c.scoped.clone());
                            continue;
                        }
                        self.resize_coll(&c.scoped, n);
                        self.module.dynamic_arrays.insert(c.scoped.clone());
                        let signed = self.class_prop_signed_of(handle, &c.prop);
                        for i in 0..n {
                            let key = format!("{}[{}]", c.scoped, i);
                            if self.read_coll_elem(&key).is_none() {
                                let mut v = if c.width > 64 {
                                    self.random_value_of_width(c.width)
                                } else {
                                    self.draw_elem(c.width, None)
                                };
                                v.is_signed = signed;
                                self.write_coll_elem(&key, v);
                            }
                        }
                    }
                }
                // A relaxation of the problem has no solution: neither has
                // the problem.
                Err(CspOutcome::Unsat) => return CspOutcome::Unsat,
                // Not modelled: the sizes the caller drew stay, and the
                // outcome proves nothing about other sizes.
                Err(_) => {
                    if has_rows {
                        cur = self.csp_expand_rows(colls);
                    }
                    return match self.rand_csp_solve_sized(
                        handle,
                        constraints,
                        constraint_depth,
                        rand_props,
                        signed_props,
                        enum_props,
                        &cur,
                        array_enums,
                        strict,
                        subs,
                    ) {
                        CspOutcome::Unsat => CspOutcome::GaveUp,
                        o => o,
                    };
                }
            }
            if has_rows {
                // New rows may have sizes of their own to solve first.
                let next = self.csp_expand_rows(colls);
                let next_sized = sized_of(&next);
                let fresh = next_sized.iter().any(|n| {
                    !sized.contains(n) && next.iter().any(|c| &c.prop == n && c.nd_base.is_some())
                });
                cur = next;
                sized = next_sized;
                if fresh {
                    continue;
                }
            }
            solves += 1;
            match self.rand_csp_solve_sized(
                handle,
                constraints,
                constraint_depth,
                rand_props,
                signed_props,
                enum_props,
                &cur,
                array_enums,
                strict,
                subs,
            ) {
                CspOutcome::Unsat | CspOutcome::GaveUp => {}
                o => return o,
            }
        }
        CspOutcome::GaveUp
    }

    /// `colls` with the rows of every member over unpacked rows
    /// (`RandColl::row_tail`) at its current size: a fixed row as a fixed
    /// array, a dynamic row as a dynamic array, each keyed under the
    /// member's multi-dimensional shape (`nd_base`).
    pub(super) fn csp_expand_rows(&self, colls: &[RandColl]) -> Vec<RandColl> {
        let mut out = Vec::with_capacity(colls.len());
        for c in colls {
            out.push(c.clone());
            let Some(tail) = &c.row_tail else {
                continue;
            };
            let shape = self.csp_row_owner_shape(c);
            let n = self.get_queue_size(&c.scoped) as i64;
            match tail {
                RowTail::Fixed(inner) => {
                    let Some(&(lo, hi)) = inner.last() else {
                        continue;
                    };
                    let mut prefixes: Vec<String> = (0..n).map(|i| format!("[{}]", i)).collect();
                    for &(l, r) in &inner[..inner.len() - 1] {
                        prefixes = prefixes
                            .iter()
                            .flat_map(|p| (l..=r).map(move |j| format!("{}[{}]", p, j)))
                            .collect();
                    }
                    for p in prefixes {
                        out.push(RandColl {
                            prop: format!("{}{}", c.prop, p),
                            scoped: format!("{}{}", c.scoped, p),
                            kind: CollKind::Fixed,
                            width: c.width,
                            lo,
                            hi,
                            nested: false,
                            is_object_elem: c.is_object_elem,
                            nd_base: Some((c.prop.clone(), shape.clone())),
                            row_tail: None,
                        });
                    }
                }
                RowTail::Dyn => {
                    for i in 0..n {
                        out.push(RandColl {
                            prop: format!("{}[{}]", c.prop, i),
                            scoped: format!("{}[{}]", c.scoped, i),
                            kind: CollKind::Dyn,
                            width: c.width,
                            lo: 0,
                            hi: 0,
                            nested: false,
                            is_object_elem: c.is_object_elem,
                            nd_base: Some((c.prop.clone(), shape.clone())),
                            row_tail: None,
                        });
                    }
                }
            }
        }
        out
    }

    /// The multi-dimensional shape the joint solver walks for a member over
    /// unpacked rows: its current size, then a fixed row's dimensions or
    /// the longest dynamic row.
    pub(super) fn csp_row_owner_shape(&self, c: &RandColl) -> Vec<(i64, i64)> {
        let n = self.get_queue_size(&c.scoped) as i64;
        let mut shape = vec![(0, n - 1)];
        match &c.row_tail {
            Some(RowTail::Fixed(inner)) => shape.extend_from_slice(inner),
            Some(RowTail::Dyn) => {
                let m = (0..n)
                    .map(|i| self.get_queue_size(&format!("{}[{}]", c.scoped, i)) as i64)
                    .max()
                    .unwrap_or(0);
                shape.push((0, m - 1));
            }
            None => {}
        }
        shape
    }

    /// Whether the size phase decides the size of `c`: a dynamic array of
    /// values whose `size()` some constraint reads.
    pub(super) fn csp_size_solved(c: &RandColl, constraints: &[ClassConstraint]) -> bool {
        if c.kind != CollKind::Dyn || (c.nested && c.row_tail.is_none()) || c.is_object_elem {
            return false;
        }
        // §7.4.5: a row (`a[1]` of `T a[][]`) is named by an index on its
        // base: a literal one equal to the row's, or a loop variable.
        let row_index: Option<(&str, i64)> = c.nd_base.as_ref().and_then(|(b, _)| {
            let i = c
                .prop
                .strip_prefix(b.as_str())?
                .strip_prefix('[')?
                .strip_suffix(']')?;
            Some((b.as_str(), i.parse().ok()?))
        });
        let names = |e: &Expression| match &Self::unparen(e).kind {
            ExprKind::Ident(h) if row_index.is_none() => {
                h.root.is_none()
                    && h.path.last().is_some_and(|s| s.name.name == c.prop)
                    && h.path.iter().all(|s| s.selects.is_empty())
                    && h.path.len() <= 2
            }
            ExprKind::MemberAccess { member, .. } if row_index.is_none() => member.name == c.prop,
            ExprKind::Index { expr, index } => row_index.is_some_and(|(b, i)| {
                let base_ok = match &Self::unparen(expr).kind {
                    ExprKind::Ident(h) => {
                        h.root.is_none()
                            && h.path.last().is_some_and(|s| s.name.name == b)
                            && h.path.iter().all(|s| s.selects.is_empty())
                            && h.path.len() <= 2
                    }
                    ExprKind::MemberAccess { member, .. } => member.name == b,
                    _ => false,
                };
                let idx_ok = match &Self::unparen(index).kind {
                    ExprKind::Number(_) => {
                        crate::compiler::elaborate::const_eval_i64_with_params(index, None)
                            .is_some_and(|k| k == i)
                    }
                    _ => true,
                };
                base_ok && idx_ok
            }),
            _ => false,
        };
        let mut hit = false;
        let mut look = |e: &Expression| {
            Self::walk_operands(e, &mut |x| {
                let recv = match &x.kind {
                    ExprKind::Call { func, args } if args.is_empty() => match &func.kind {
                        ExprKind::MemberAccess { expr, member } if member.name == "size" => {
                            Some(expr)
                        }
                        _ => None,
                    },
                    _ => None,
                };
                if recv.is_some_and(|r| names(r)) {
                    hit = true;
                }
            })
        };
        for con in constraints {
            for it in &con.items {
                Self::csp_item_walk(it, &mut |i| match i {
                    ConstraintItem::Expr(e) => look(e),
                    ConstraintItem::Inside { expr, range, .. } => {
                        look(expr);
                        for r in range {
                            match r {
                                ConstraintRange::Value(v) => look(v),
                                ConstraintRange::Range { lo, hi } => {
                                    look(lo);
                                    look(hi);
                                }
                            }
                        }
                    }
                    ConstraintItem::Implication { condition, .. }
                    | ConstraintItem::IfElse { condition, .. } => look(condition),
                    _ => {}
                });
            }
        }
        hit
    }

    /// §18.5.8.1: the sizes of the arrays in `sized`, solved with the other
    /// variables, every array of them a size variable without elements.
    /// What reads the elements of such an array is left out (a
    /// relaxation), so Unsat here is final.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn csp_size_phase<'c>(
        &mut self,
        handle: usize,
        constraints: &[ClassConstraint],
        constraint_depth: &[usize],
        rand_props: &[(String, u32)],
        signed_props: &HashSet<String>,
        enum_props: &HashMap<String, String>,
        colls: &'c [RandColl],
        array_enums: &HashMap<String, String>,
        sized: &HashSet<String>,
    ) -> Result<Vec<(&'c RandColl, u64)>, CspOutcome> {
        let unread = Self::csp_unread(constraints, rand_props, colls);
        let mut csp = self
            .csp_vars(
                handle,
                rand_props,
                signed_props,
                enum_props,
                colls,
                array_enums,
                &unread,
                false,
                sized,
            )
            .ok_or(CspOutcome::NotApplicable)?;
        csp.size_phase = true;
        self.csp_split_slices(&mut csp, constraints);
        if csp.wide_fail {
            return Err(CspOutcome::NotApplicable);
        }
        self.csp_translate(&mut csp, constraints, constraint_depth)
            .ok_or(CspOutcome::NotApplicable)?;
        let st = match self.csp_find(&csp) {
            Ok(st) => st,
            // §18.5.14: a preference that conflicts with the hard
            // constraints is dropped; the sizes then follow the hard ones.
            Err(CspOutcome::Unsat) if !csp.soft_priority.is_empty() => {
                csp.soft_enabled = Some(HashSet::default());
                self.csp_translate(&mut csp, constraints, constraint_depth)
                    .ok_or(CspOutcome::NotApplicable)?;
                self.csp_find(&csp)?
            }
            Err(o) => return Err(o),
        };
        let mut out = Vec::new();
        for c in colls {
            if let Some(&sv) = csp.sizes.get(&c.prop) {
                let n = st.fixed(sv).ok_or(CspOutcome::GaveUp)?;
                out.push((c, n.max(0) as u64));
            }
        }
        Ok(out)
    }

    /// §18.4: the rand variables wider than 64 bits that no constraint
    /// reads (or reads only through `size()`).
    fn csp_unread(
        constraints: &[ClassConstraint],
        rand_props: &[(String, u32)],
        colls: &[RandColl],
    ) -> HashSet<String> {
        if rand_props.iter().any(|(_, w)| *w > 64) || colls.iter().any(|c| c.width > 64) {
            let names: Vec<&str> = rand_props
                .iter()
                .filter(|(_, w)| *w > 64)
                .map(|(n, _)| n.as_str())
                .chain(
                    colls
                        .iter()
                        .filter(|c| c.width > 64)
                        .map(|c| c.prop.as_str()),
                )
                .collect();
            Self::csp_wide_unread(constraints, &names)
        } else {
            HashSet::default()
        }
    }

    /// The joint solve with every dynamic array at its current size.
    #[allow(clippy::too_many_arguments)]
    fn rand_csp_solve_sized(
        &mut self,
        handle: usize,
        constraints: &[ClassConstraint],
        constraint_depth: &[usize],
        rand_props: &[(String, u32)],
        signed_props: &HashSet<String>,
        enum_props: &HashMap<String, String>,
        colls: &[RandColl],
        array_enums: &HashMap<String, String>,
        strict: bool,
        subs: &[(String, usize)],
    ) -> CspOutcome {
        // §18.4: a rand variable wider than 64 bits that no constraint reads
        // (or reads only through `size()`) takes any value of its width; it
        // stays out of the solve instead of keeping the whole class out.
        let unread = Self::csp_unread(constraints, rand_props, colls);
        let Some(mut csp) = self.csp_vars(
            handle,
            rand_props,
            signed_props,
            enum_props,
            colls,
            array_enums,
            &unread,
            !subs.is_empty(),
            &HashSet::default(),
        ) else {
            return CspOutcome::NotApplicable;
        };
        csp.draw_free = strict;
        // §18.5.9: the rand sub-objects' scalars and their own constraints
        // join the problem, so an item tying two of them together (`a.x <
        // b.y`) or one to an enclosing member is solved jointly. The
        // elements of an object array are named `arr[k]` inside the solver
        // (see `obj`); `check` is the same set as the evaluator spells it,
        // for the final acceptance check.
        let obj_arrays: HashSet<String> = subs
            .iter()
            .filter_map(|(p, _)| p.split_once('[').map(|(a, _)| a.to_string()))
            .collect();
        let _obj_guard = (!obj_arrays.is_empty())
            .then(|| obj::csp_obj_enter(obj_arrays.iter().cloned().collect()));
        let mut joint: Vec<ClassConstraint>;
        let joint_check: Vec<ClassConstraint>;
        let check: &[ClassConstraint];
        let constraints: &[ClassConstraint] = if subs.is_empty() {
            check = constraints;
            constraints
        } else {
            joint = if obj_arrays.is_empty() {
                constraints.to_vec()
            } else {
                match self.csp_obj_prepare(&csp, constraints, &obj_arrays) {
                    Some(j) => j,
                    None => return CspOutcome::NotApplicable,
                }
            };
            for (prop, sub) in subs {
                let Some(items) = self.csp_add_sub(&mut csp, prop, *sub, constraints) else {
                    return CspOutcome::NotApplicable;
                };
                joint.push(ClassConstraint {
                    is_static: false,
                    is_extern: false,
                    has_body: true,
                    name: crate::ast::Identifier {
                        name: format!("{}.*", prop),
                        span: crate::ast::Span::dummy(),
                    },
                    items,
                    span: crate::ast::Span::dummy(),
                });
            }
            if obj_arrays.is_empty() {
                check = &joint;
            } else {
                joint_check = joint
                    .iter()
                    .map(|c| {
                        let mut c = c.clone();
                        c.items = c.items.iter().map(Self::csp_obj_real_item).collect();
                        c
                    })
                    .collect();
                check = &joint_check;
            }
            &joint
        };
        self.csp_split_slices(&mut csp, constraints);
        if csp.wide_fail {
            return CspOutcome::NotApplicable;
        }
        if self
            .csp_translate(&mut csp, constraints, constraint_depth)
            .is_none()
        {
            return CspOutcome::NotApplicable;
        }
        if strict && csp.opaque {
            return CspOutcome::GaveUp;
        }
        let out = self.csp_run(&csp, check, colls);
        if !matches!(out, CspOutcome::Unsat) || csp.soft_priority.is_empty() {
            return out;
        }
        // A preference is discarded only after proving a conflict with hard
        // constraints or already accepted, higher-priority preferences.
        let mut priority = csp.soft_priority.clone();
        priority.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(b.2.cmp(&a.2)));
        csp.soft_enabled = Some(HashSet::default());
        if self
            .csp_translate(&mut csp, constraints, constraint_depth)
            .is_none()
        {
            return CspOutcome::NotApplicable;
        }
        let hard = self.csp_run(&csp, check, colls);
        if !matches!(hard, CspOutcome::Sat) {
            return hard;
        }
        for (_, _, id) in priority {
            csp.soft_enabled.as_mut().unwrap().insert(id);
            if self
                .csp_translate(&mut csp, constraints, constraint_depth)
                .is_none()
            {
                return CspOutcome::NotApplicable;
            }
            match self.csp_run(&csp, check, colls) {
                CspOutcome::Sat => {}
                CspOutcome::Unsat => {
                    csp.soft_enabled.as_mut().unwrap().remove(&id);
                }
                other => return other,
            }
        }
        CspOutcome::Sat
    }

    /// §18.12: solve `std::randomize(…) with {items}` jointly. `scalars`
    /// are (name, lvalue, width, signed, enum values); `arrays` are (name
    /// as the constraints write it, storage name, element indices, width,
    /// signed, enum values) of collections already sized. The solution is written to the caller's variables and checked
    /// with the inline-constraint checker.
    #[allow(clippy::type_complexity)]
    pub(super) fn rand_csp_solve_scope(
        &mut self,
        items: &[ConstraintItem],
        scalars: &[(String, Expression, u32, bool, Option<Vec<u64>>)],
        arrays: &[(String, String, Vec<i64>, u32, bool, Option<Vec<u64>>)],
    ) -> CspOutcome {
        match self.csp_scope(items, scalars, arrays, &HashSet::default()) {
            Ok(_) => CspOutcome::Sat,
            Err(o) => o,
        }
    }

    /// §18.5.8.1: the sizes of the arrays in `sized` (named as the
    /// constraints write them) for `std::randomize`, solved before their
    /// elements (see `csp_size_phase`).
    #[allow(clippy::type_complexity)]
    pub(super) fn csp_scope_sizes(
        &mut self,
        items: &[ConstraintItem],
        scalars: &[(String, Expression, u32, bool, Option<Vec<u64>>)],
        arrays: &[(String, String, Vec<i64>, u32, bool, Option<Vec<u64>>)],
        sized: &HashSet<String>,
    ) -> Result<Vec<(String, u64)>, CspOutcome> {
        self.csp_scope(items, scalars, arrays, sized)
    }

    /// The scope solve: with `sized` empty a full solve, written to the
    /// caller's variables (Ok with no sizes); otherwise the size phase.
    #[allow(clippy::type_complexity)]
    fn csp_scope(
        &mut self,
        items: &[ConstraintItem],
        scalars: &[(String, Expression, u32, bool, Option<Vec<u64>>)],
        arrays: &[(String, String, Vec<i64>, u32, bool, Option<Vec<u64>>)],
        sized: &HashSet<String>,
    ) -> Result<Vec<(String, u64)>, CspOutcome> {
        let mut csp = Csp {
            handle: usize::MAX,
            vars: Vec::new(),
            dom0: Vec::new(),
            scalars: HashMap::default(),
            arrays: HashMap::default(),
            nd_dims: HashMap::default(),
            nd_dyn: HashMap::default(),
            srcs: Vec::new(),
            nodes: Vec::new(),
            soft_enabled: None,
            soft_priority: Vec::new(),
            soft_rank: (0, 0),
            dists: Vec::new(),
            order: Vec::new(),
            opaque: false,
            has_subs: false,
            dotted: false,
            scope: Some(HashMap::default()),
            segs: HashMap::default(),
            slice_vars: HashMap::default(),
            aux_base: HashMap::default(),
            fields: HashMap::default(),
            field_elems: HashMap::default(),
            pdims: HashMap::default(),
            lsb: HashMap::default(),
            ascending: HashSet::default(),
            wide: HashSet::default(),
            wide_seg: HashSet::default(),
            wide_free: Vec::new(),
            draw_free: false,
            wide_fail: false,
            pop: HashMap::default(),
            pop_rev: HashMap::default(),
            carries: Vec::new(),
            carry_accs: Vec::new(),
            carry_base: None,
            sizes: HashMap::default(),
            size_phase: false,
        };
        let dom_of = |w: u32, s: bool, members: &Option<Vec<u64>>| -> Dom {
            match members {
                Some(m) if !m.is_empty() => {
                    dom_norm(m.iter().map(|&v| (v as i128, v as i128)).collect())
                }
                _ => {
                    let (lo, hi) = ws_range(w, s);
                    vec![(lo, hi)]
                }
            }
        };
        for (name, key, idx, w, s, members) in arrays {
            if sized.contains(name) {
                let sv = csp.vars.len();
                csp.vars.push(CspVar {
                    key: VarKey::Aux,
                    width: 32,
                    signed: true,
                });
                csp.dom0.push(vec![(0, SIZE_TOP)]);
                csp.sizes.insert(name.clone(), sv);
                csp.arrays.insert(
                    name.clone(),
                    CspArr {
                        elems: Vec::new(),
                        width: *w,
                        signed: *s,
                    },
                );
                continue;
            }
            if *w > 64 && !sized.is_empty() {
                // Not a variable of the size phase.
                continue;
            }
            if *w == 0 || *w > 64 {
                return Err(CspOutcome::NotApplicable);
            }
            let dom = dom_of(*w, *s, members);
            let mut elems = Vec::with_capacity(idx.len());
            for &i in idx {
                elems.push((i, csp.vars.len()));
                csp.vars.push(CspVar {
                    key: VarKey::Elem(format!("{}[{}]", key, i)),
                    width: *w,
                    signed: *s,
                });
                csp.dom0.push(dom.clone());
            }
            csp.arrays.insert(
                name.clone(),
                CspArr {
                    elems,
                    width: *w,
                    signed: *s,
                },
            );
        }
        csp.nd_dims = self.rand_scope_nd.clone();
        csp.nd_dyn = self.rand_scope_nd_dyn.clone();
        for (name, lv, w, s, members) in scalars {
            if *w == 0 || csp.arrays.contains_key(name) {
                return Err(CspOutcome::NotApplicable);
            }
            // §18.5: a local wider than 64 bits is solved through segments
            // of at most 64 bits, as a class member is (`csp_wide_split`).
            let wide = *w > 64;
            if wide && (members.is_some() || !sized.is_empty()) {
                // Not modelled: left out of the solve, as before.
                continue;
            }
            if wide {
                csp.wide.insert(csp.vars.len());
            }
            csp.scalars.insert(name.clone(), csp.vars.len());
            csp.vars.push(CspVar {
                key: VarKey::Prop(name.clone()),
                width: *w,
                signed: *s,
            });
            csp.dom0.push(if wide {
                vec![(0, 0)]
            } else {
                dom_of(*w, *s, members)
            });
            if let Some(l) = csp.scope.as_mut() {
                l.insert(name.clone(), lv.clone());
            }
        }
        if csp.vars.is_empty() || csp.vars.len() > MAX_VARS {
            return Err(CspOutcome::NotApplicable);
        }
        csp.size_phase = !sized.is_empty();
        if std::env::var_os("XEZIM_RAND_DBG").is_some() {
            eprintln!(
                "[rand-dbg] scope solve: arrays {:?} vars {:?} nd {:?}",
                csp.arrays
                    .iter()
                    .map(|(n, a)| (n.clone(), a.elems.len(), a.width, a.signed))
                    .collect::<Vec<_>>(),
                csp.vars.iter().map(|v| format!("{:?}", v.key)).collect::<Vec<_>>(),
                csp.nd_dims
            );
        }
        let constraints = vec![ClassConstraint {
            is_static: false,
            is_extern: false,
            has_body: true,
            name: crate::ast::Identifier {
                name: "__inline__".to_string(),
                span: crate::ast::Span::dummy(),
            },
            items: items.to_vec(),
            span: crate::ast::Span::dummy(),
        }];
        let prev = CSP_SCOPE.with(|c| c.replace(true));
        let prev_receiver = self.rand_receiver.take();
        self.csp_split_slices(&mut csp, &constraints);
        let out = if csp.wide_fail || self.csp_translate(&mut csp, &constraints, &[]).is_none() {
            Err(CspOutcome::NotApplicable)
        } else if sized.is_empty() {
            match self.csp_run(&csp, &constraints, &[]) {
                CspOutcome::Sat => Ok(Vec::new()),
                o => Err(o),
            }
        } else {
            let mut found = self.csp_find(&csp);
            if matches!(found, Err(CspOutcome::Unsat)) && !csp.soft_priority.is_empty() {
                // §18.5.14: sizes that follow the hard constraints alone.
                csp.soft_enabled = Some(HashSet::default());
                found = match self.csp_translate(&mut csp, &constraints, &[]) {
                    Some(()) => self.csp_find(&csp),
                    None => Err(CspOutcome::NotApplicable),
                };
            }
            found.and_then(|st| {
                csp.sizes
                    .iter()
                    .map(|(n, &sv)| {
                        let k = st.fixed(sv).ok_or(CspOutcome::GaveUp)?;
                        Ok((n.clone(), k.max(0) as u64))
                    })
                    .collect()
            })
        };
        self.rand_receiver = prev_receiver;
        CSP_SCOPE.with(|c| c.set(prev));
        out
    }

    /// Build the variable table: rand scalars and array elements.
    fn csp_vars(
        &mut self,
        handle: usize,
        rand_props: &[(String, u32)],
        signed_props: &HashSet<String>,
        enum_props: &HashMap<String, String>,
        colls: &[RandColl],
        array_enums: &HashMap<String, String>,
        unread_wide: &HashSet<String>,
        // Rand sub-objects join later (`csp_add_sub`): the object needs no
        // scalar of its own.
        obj_elems: bool,
        // The dynamic arrays whose size is a variable (`csp_size_phase`).
        sized: &HashSet<String>,
    ) -> Option<Csp> {
        let mut csp = Csp {
            handle,
            vars: Vec::new(),
            dom0: Vec::new(),
            scalars: HashMap::default(),
            arrays: HashMap::default(),
            nd_dims: HashMap::default(),
            nd_dyn: HashMap::default(),
            srcs: Vec::new(),
            nodes: Vec::new(),
            soft_enabled: None,
            soft_priority: Vec::new(),
            soft_rank: (0, 0),
            dists: Vec::new(),
            order: Vec::new(),
            opaque: false,
            has_subs: false,
            dotted: false,
            scope: None,
            segs: HashMap::default(),
            slice_vars: HashMap::default(),
            aux_base: HashMap::default(),
            fields: HashMap::default(),
            field_elems: HashMap::default(),
            pdims: HashMap::default(),
            lsb: HashMap::default(),
            ascending: HashSet::default(),
            wide: HashSet::default(),
            wide_seg: HashSet::default(),
            wide_free: Vec::new(),
            draw_free: false,
            wide_fail: false,
            pop: HashMap::default(),
            pop_rev: HashMap::default(),
            carries: Vec::new(),
            carry_accs: Vec::new(),
            carry_base: None,
            sizes: HashMap::default(),
            size_phase: false,
        };
        let enum_dom = |me: &Self, tn: &str| -> Option<Dom> {
            let members = me.module.enum_members.get(tn)?;
            if members.is_empty() {
                return None;
            }
            Some(dom_norm(
                members.iter().map(|m| (m.1 as i128, m.1 as i128)).collect(),
            ))
        };
        for c in colls {
            if let Some((base, dims)) = &c.nd_base {
                csp.nd_dims.insert(base.clone(), dims.clone());
            }
            // §7.4.5: a member whose elements are rows contributes its shape
            // (and its size, while sizes are solved); the rows carry the
            // values.
            if c.row_tail.is_some() {
                csp.nd_dims
                    .entry(c.prop.clone())
                    .or_insert_with(|| self.csp_row_owner_shape(c));
                csp.nd_dyn.insert(
                    c.prop.clone(),
                    if matches!(c.row_tail, Some(RowTail::Dyn)) {
                        2
                    } else {
                        1
                    },
                );
                if !sized.contains(&c.prop) {
                    continue;
                }
            }
            // §18.5.9: the objects of an array of rand handles are no
            // values; they join as sub-objects (`csp_add_sub`).
            if c.is_object_elem && !c.nested && c.kind != CollKind::Assoc {
                continue;
            }
            if c.is_object_elem || (c.nested && c.row_tail.is_none()) || c.kind == CollKind::Assoc {
                return None;
            }
            if sized.contains(&c.prop) {
                // §18.5.8.1: the size is solved before any element exists.
                let sv = csp.vars.len();
                csp.vars.push(CspVar {
                    key: VarKey::Aux,
                    width: 32,
                    signed: true,
                });
                csp.dom0.push(vec![(0, SIZE_TOP)]);
                csp.sizes.insert(c.prop.clone(), sv);
                csp.arrays.insert(
                    c.prop.clone(),
                    CspArr {
                        elems: Vec::new(),
                        width: c.width,
                        signed: false,
                    },
                );
                continue;
            }
            let idx: Vec<i64> = match c.kind {
                CollKind::Fixed => (c.lo..=c.hi).collect(),
                _ => (0..self.get_queue_size(&c.scoped) as i64).collect(),
            };
            let wide = c.width > 64;
            if wide && unread_wide.contains(&c.prop) {
                csp.wide_free.extend(
                    idx.iter()
                        .map(|i| (VarKey::Elem(format!("{}[{}]", c.scoped, i)), c.width)),
                );
                continue;
            }
            if wide && array_enums.contains_key(&c.prop) {
                return None;
            }
            let signed = self.class_prop_signed_of(handle, &c.prop);
            let dom = if wide {
                // A placeholder: the elements are solved as segments.
                vec![(0, 0)]
            } else {
                array_enums
                    .get(&c.prop)
                    .and_then(|tn| enum_dom(self, tn))
                    .unwrap_or_else(|| {
                        let (lo, hi) = ws_range(c.width, signed);
                        vec![(lo, hi)]
                    })
            };
            let mut elems = Vec::with_capacity(idx.len());
            for i in idx {
                if wide {
                    csp.wide.insert(csp.vars.len());
                }
                elems.push((i, csp.vars.len()));
                csp.vars.push(CspVar {
                    key: VarKey::Elem(format!("{}[{}]", c.scoped, i)),
                    width: c.width,
                    signed,
                });
                csp.dom0.push(dom.clone());
            }
            csp.arrays.insert(
                c.prop.clone(),
                CspArr {
                    elems,
                    width: c.width,
                    signed,
                },
            );
        }
        for (name, w) in rand_props {
            if csp.arrays.contains_key(name) || csp.nd_dims.contains_key(name) {
                continue;
            }
            if *w == 0 {
                return None;
            }
            let signed = signed_props.contains(name);
            let dom = if *w > 64 {
                if unread_wide.contains(name) {
                    csp.wide_free.push((VarKey::Prop(name.clone()), *w));
                    continue;
                }
                if enum_props.contains_key(name) {
                    return None;
                }
                csp.wide.insert(csp.vars.len());
                vec![(0, 0)]
            } else {
                enum_props
                    .get(name)
                    .and_then(|tn| enum_dom(self, tn))
                    .unwrap_or_else(|| {
                        let (lo, hi) = ws_range(*w, signed);
                        vec![(lo, hi)]
                    })
            };
            csp.scalars.insert(name.clone(), csp.vars.len());
            csp.vars.push(CspVar {
                key: VarKey::Prop(name.clone()),
                width: *w,
                signed,
            });
            csp.dom0.push(dom);
        }
        // An array sized 0 (by the size phase or by the caller) is a
        // problem without variables, still one to judge.
        if (csp.vars.is_empty() && !obj_elems && csp.arrays.is_empty()) || csp.vars.len() > MAX_VARS
        {
            return None;
        }
        csp.dotted = csp.scalars.keys().any(|k| k.contains('.'));
        Some(csp)
    }

    fn csp_translate(
        &mut self,
        csp: &mut Csp,
        constraints: &[ClassConstraint],
        depths: &[usize],
    ) -> Option<()> {
        csp.srcs.clear();
        csp.nodes.clear();
        csp.dists.clear();
        csp.order.clear();
        csp.soft_priority.clear();
        csp.opaque = false;
        csp.carries.clear();
        csp.carry_accs.clear();
        match csp.carry_base {
            Some(b) => {
                csp.vars.truncate(b);
                csp.dom0.truncate(b);
            }
            None => csp.carry_base = Some(csp.vars.len()),
        }
        let env = Env::default();
        self.csp_slice_links(csp);
        for (index, con) in constraints.iter().enumerate() {
            csp.soft_rank = if con.name.name == "__inline__" {
                (0, usize::MAX)
            } else {
                (
                    depths
                        .get(index)
                        .copied()
                        .unwrap_or(usize::MAX - 1)
                        .saturating_add(1),
                    con.span.start as usize,
                )
            };
            for it in &con.items {
                let Some(n) = self.csp_item(csp, it, &env) else {
                    if std::env::var_os("XEZIM_RAND_DBG").is_some() {
                        eprintln!("[rand-dbg] joint solve does not model: {:?}", it);
                    }
                    return None;
                };
                if !matches!(n, Node::True) {
                    csp.nodes.push(n);
                }
            }
        }
        Some(())
    }

    /// Search with restarts, then write and verify a solution.
    fn csp_run(
        &mut self,
        csp: &Csp,
        constraints: &[ClassConstraint],
        colls: &[RandColl],
    ) -> CspOutcome {
        let st = match self.csp_find(csp) {
            Ok(st) => st,
            Err(o) => return o,
        };
        let nvars = csp.vars.len();
        for v in 0..nvars {
            if csp.wide.contains(&v) {
                // Written through its segments.
                continue;
            }
            let x = st.fixed(v).unwrap_or(0);
            self.csp_write(csp, v, x);
        }
        let accepted = if csp.scope.is_some() {
            let items: Vec<ConstraintItem> = constraints
                .iter()
                .flat_map(|c| c.items.iter().cloned())
                .collect();
            self.inline_constraints_satisfied(&items)
        } else {
            self.rand_items_accept(csp.handle, constraints, colls, &mut false)
        };
        if accepted {
            if csp.draw_free {
                self.csp_draw_free(csp);
            }
            CspOutcome::Sat
        } else {
            if std::env::var_os("XEZIM_RAND_DBG").is_some() {
                eprintln!("[rand-dbg] joint solve found a solution the checker rejects");
            }
            CspOutcome::GaveUp
        }
    }

    /// Search with restarts: the final state, every variable fixed.
    fn csp_find(&mut self, csp: &Csp) -> Result<St, CspOutcome> {
        let nvars = csp.vars.len();
        let mut watch: Vec<Vec<usize>> = vec![Vec::new(); nvars];
        for (i, n) in csp.nodes.iter().enumerate() {
            let mut ds = Vec::new();
            n.deps(&csp.srcs, &mut ds);
            ds.sort_unstable();
            ds.dedup();
            for v in ds {
                watch[v].push(i);
            }
        }
        let aux = Self::csp_aux(csp);
        let mut budget = NODE_BUDGET;
        let mut run_budget: u64 = 1000;
        let mut st = St {
            d: csp.dom0.clone(),
            trail: Vec::new(),
            dirty: Vec::new(),
            work: 0,
            est_work: 0,
            cut: false,
            level_tries: 0,
            level_cut: false,
        };
        let mut level_tries: u32 = 16;
        loop {
            st.d.clone_from(&csp.dom0);
            st.trail.clear();
            st.dirty.clear();
            // §18.5.9: a solve with rand sub-objects abandons a decision
            // level after a few refuted values, a little later each run.
            st.level_tries = if csp.has_subs { level_tries } else { 0 };
            st.level_cut = false;
            let mut run = run_budget.min(budget);
            let r = self.csp_search(csp, &aux, &watch, &mut st, &mut run);
            budget -= run_budget.min(budget) - run;
            match r {
                Some(true) => break,
                Some(false) if st.level_cut && budget > 0 && st.work < WORK_BUDGET => {
                    level_tries = level_tries.saturating_mul(2);
                    run_budget *= 2;
                }
                Some(false) if st.cut => return Err(CspOutcome::GaveUp),
                Some(false) => return Err(CspOutcome::Unsat),
                None if budget == 0 || st.work >= WORK_BUDGET => {
                    if std::env::var_os("XEZIM_RAND_DBG").is_some() {
                        eprintln!(
                            "[rand-dbg] joint solve gave up: nodes={} work={}",
                            NODE_BUDGET - budget,
                            st.work
                        );
                    }
                    return Err(CspOutcome::GaveUp);
                }
                None => run_budget *= 2,
            }
        }
        Ok(st)
    }

    /// Depth-first search: `Some(true)` solved (every domain fixed),
    /// `Some(false)` exhausted, `None` out of budget.
    fn csp_search(
        &mut self,
        csp: &Csp,
        aux: &Aux,
        watch: &[Vec<usize>],
        st: &mut St,
        budget: &mut u64,
    ) -> Option<bool> {
        let all: Vec<usize> = (0..csp.nodes.len()).collect();
        match self.csp_propagate(csp, watch, st, &all) {
            Some(true) => {}
            Some(false) => return Some(false),
            None => return None,
        }
        // (variable, value, trail mark) per decision level
        let mut stack: Vec<(usize, i128, usize)> = Vec::new();
        // Refuted values per decision level. With rand sub-objects the
        // problem is large and a wrong early decision can leave a wide
        // variable with thousands of values that all fail: after a few
        // refutations the level is abandoned (a cut, not a proof) and the
        // search backs up further or restarts.
        let mut refuted: Vec<u32> = Vec::new();
        loop {
            let Some(v) = self.csp_pick_var(csp, aux, st) else {
                return Some(true);
            };
            if *budget == 0 {
                return None;
            }
            *budget -= 1;
            let val = self.csp_pick_val(csp, aux, watch, st, v);
            stack.push((v, val, st.trail.len()));
            st.dirty.clear();
            st.set(v, vec![(val, val)]);
            let dirty = std::mem::take(&mut st.dirty);
            match self.csp_propagate(csp, watch, st, &Self::csp_watchers(watch, &dirty)) {
                Some(true) => continue,
                Some(false) => {}
                None => return None,
            }
            // Conflict: undo decisions until `x != v` leaves a consistent state.
            loop {
                let Some((v, val, mark)) = stack.pop() else {
                    return Some(false);
                };
                st.undo(mark);
                if st.level_tries > 0 {
                    let d = stack.len();
                    refuted.resize(d + 1, 0);
                    refuted[d] += 1;
                    if refuted[d] > st.level_tries && dom_size(&st.d[v]) > 2 {
                        refuted.truncate(d);
                        st.cut = true;
                        st.level_cut = true;
                        continue;
                    }
                }
                if *budget == 0 {
                    return None;
                }
                *budget -= 1;
                st.dirty.clear();
                // Each refuted value splits the domain once more, and the
                // split domain is copied again on every later exclusion — a
                // wide variable refuted value by value costs quadratic time.
                st.work += st.d[v].len() as u64;
                if st.work >= WORK_BUDGET {
                    return None;
                }
                let nd = dom_minus(&st.d[v], &vec![(val, val)]);
                if !st.set(v, nd) {
                    continue;
                }
                let dirty = std::mem::take(&mut st.dirty);
                match self.csp_propagate(csp, watch, st, &Self::csp_watchers(watch, &dirty)) {
                    Some(true) => break,
                    Some(false) => continue,
                    None => return None,
                }
            }
        }
    }

    fn csp_watchers(watch: &[Vec<usize>], vars: &[usize]) -> Vec<usize> {
        let mut out: Vec<usize> = vars
            .iter()
            .flat_map(|&v| watch[v].iter().copied())
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The next variable to decide: one whose `solve … before` predecessors
    /// are all fixed; among those a variable under a dist in force first and
    /// one whose dist still waits on its guard last; then the smallest
    /// domain, ties broken at random.
    fn csp_pick_var(&mut self, csp: &Csp, aux: &Aux, st: &St) -> Option<usize> {
        let mut best: Option<(u8, u128, usize)> = None;
        let mut ties = 0u32;
        let mut composite = None;
        for (v, d) in st.d.iter().enumerate() {
            if dom_fixed(d).is_some() {
                continue;
            }
            // Decided through its segments (see `Aux::composite`).
            if aux.composite[v] {
                composite.get_or_insert(v);
                continue;
            }
            let blocked = aux.preds[v].iter().any(|&p| st.fixed(p).is_none())
                || aux.seg_hi[v].iter().any(|&p| st.fixed(p).is_none());
            let tier = if aux.dists[v].is_empty() {
                1
            } else {
                match self.csp_dist_state(csp, aux, st, v) {
                    (Some(_), _) => 0,
                    (None, true) => 2,
                    (None, false) => 1,
                }
            };
            let rank = blocked as u8 * 3 + tier;
            let n = dom_size(d);
            match best {
                Some((br, bn, _)) if (rank, n) > (br, bn) => {}
                Some((br, bn, _)) if (rank, n) == (br, bn) => {
                    ties += 1;
                    if self.cur_rng().random_range(0..=ties) == 0 {
                        best = Some((rank, n, v));
                    }
                }
                _ => {
                    best = Some((rank, n, v));
                    ties = 0;
                }
            }
        }
        best.map(|b| b.2).or(composite)
    }

    /// The dist in force on `v` (every guard entailed), and whether some
    /// dist on `v` still has an open guard.
    fn csp_dist_state(&mut self, csp: &Csp, aux: &Aux, st: &St, v: usize) -> (Option<usize>, bool) {
        let mut open = false;
        for &di in &aux.dists[v] {
            let mut all = true;
            for g in &csp.dists[di].guards {
                match self.csp_status(csp, g, st) {
                    Some(true) => {}
                    Some(false) => {
                        all = false;
                        break;
                    }
                    None => {
                        all = false;
                        open = true;
                    }
                }
            }
            if all {
                return (Some(di), open);
            }
        }
        (None, open)
    }

    /// A value for `v`: by the dist in force (§18.5.4: an item in proportion
    /// to its weight among the items the domain still meets, then a value of
    /// that item), by weighing (`csp_weigh`) for a condition variable, else
    /// uniformly over the domain.
    fn csp_pick_val(
        &mut self,
        csp: &Csp,
        aux: &Aux,
        watch: &[Vec<usize>],
        st: &mut St,
        v: usize,
    ) -> i128 {
        if !aux.dists[v].is_empty() {
            if let (Some(di), _) = self.csp_dist_state(csp, aux, st, v) {
                let mut live: Vec<(Dom, f64)> = Vec::new();
                let mut total = 0.0;
                for &(lo, hi, w) in &csp.dists[di].items {
                    let part = dom_clip(&st.d[v], lo, hi);
                    if !part.is_empty() {
                        total += w;
                        live.push((part, w));
                    }
                }
                if total > 0.0 {
                    let mut r = self.cur_rng().random_range(0.0..total);
                    let mut pick = live.len() - 1;
                    for (i, (_, w)) in live.iter().enumerate() {
                        if r < *w {
                            pick = i;
                            break;
                        }
                        r -= w;
                    }
                    let part = &live[pick].0;
                    let k = self.cur_rng().random_range(0..dom_size(part));
                    return dom_nth(part, k);
                }
            }
        }
        // §18.5.10 / §20.9: a `$countones` count is drawn in proportion to
        // the values of its variable that have that many ones (C(w, n)), as
        // a uniform draw of the variable would; the variable then draws
        // exactly that many ones.
        if let Some((_, w)) = aux.pop_x[v] {
            if let Some(n) = self.csp_pop_count(&st.d[v], w) {
                return n;
            }
        }
        if let Some(c) = aux.pop_c[v] {
            let w = csp.vars[v].width;
            for _ in 0..64 {
                let n = match st.fixed(c) {
                    Some(n) => n,
                    None => match self.csp_pop_count(&st.d[c], w) {
                        Some(n) => n,
                        None => break,
                    },
                };
                // The ones go below the top of a non-negative range.
                let (lo, hi) = st.bounds(v);
                let span = if lo >= 0 {
                    (128 - (hi as u128).leading_zeros()).min(w)
                } else {
                    w
                };
                if n < 0 || n > span as i128 {
                    break;
                }
                let x = self.csp_pop_draw(w, span, n as u32, csp.vars[v].signed);
                if dom_has(&st.d[v], x) {
                    return x;
                }
            }
        }
        // A size bounded only below stays near that bound (see `SIZE_OPEN`).
        if st.bounds(v).1 == SIZE_TOP && csp.sizes.values().any(|&sv| sv == v) {
            let lo = st.bounds(v).0;
            let part = dom_clip(&st.d[v], lo, lo + SIZE_OPEN - 1);
            let k = self.cur_rng().random_range(0..dom_size(&part));
            return dom_nth(&part, k);
        }
        let size = dom_size(&st.d[v]);
        if aux.guard_var[v] && size <= EST_MAX_DOM && st.est_work < EST_BUDGET {
            if let Some(x) = self.csp_weigh(csp, aux, watch, st, v) {
                return x;
            }
        }
        let k = self.cur_rng().random_range(0..size);
        dom_nth(&st.d[v], k)
    }

    /// A count of ones for a `w`-bit variable from `dom`, weighed by the
    /// number of `w`-bit values with that count, C(w, n). None when `dom`
    /// holds no count in [0, w].
    fn csp_pop_count(&mut self, dom: &Dom, w: u32) -> Option<i128> {
        let w = w.min(64);
        let mut binom = vec![1f64; w as usize + 1];
        for n in 1..=w as usize {
            binom[n] = binom[n - 1] * (w as usize + 1 - n) as f64 / n as f64;
        }
        let cand: Vec<(i128, f64)> = dom_clip(dom, 0, w as i128)
            .iter()
            .flat_map(|&(l, h)| l..=h)
            .map(|n| (n, binom[n as usize]))
            .collect();
        let total: f64 = cand.iter().map(|c| c.1).sum();
        if cand.is_empty() || total <= 0.0 {
            return None;
        }
        let mut r = self.cur_rng().random_range(0.0..total);
        for &(n, wt) in &cand {
            if r < wt {
                return Some(n);
            }
            r -= wt;
        }
        cand.last().map(|c| c.0)
    }

    /// log2 of the solutions a domain stands for: its size, or for a
    /// `$countones` count the values of its variable with one of its
    /// counts, Σ C(w, n).
    fn csp_log_size(aux: &Aux, u: usize, d: &Dom) -> f64 {
        let Some((_, w)) = aux.pop_x[u] else {
            return (dom_size(d) as f64).log2();
        };
        let w = w.min(64) as usize;
        let mut binom = 1f64;
        let mut total = 0f64;
        for n in 0..=w {
            if n > 0 {
                binom = binom * (w + 1 - n) as f64 / n as f64;
            }
            if dom_has(d, n as i128) {
                total += binom;
            }
        }
        total.log2()
    }

    /// A `w`-bit value with exactly `n` ones at random positions among its
    /// low `span` bits, read as the variable's value (sign-extended when it
    /// is signed).
    fn csp_pop_draw(&mut self, w: u32, span: u32, n: u32, signed: bool) -> i128 {
        let w = w.min(64);
        let mut pos: Vec<u32> = (0..span.min(w)).collect();
        let mut bits: u64 = 0;
        for k in 0..(n as usize).min(pos.len()) {
            let j = self.cur_rng().random_range(k..pos.len());
            pos.swap(k, j);
            bits |= 1u64 << pos[k];
        }
        if signed && w > 0 && (bits >> (w - 1)) & 1 == 1 {
            bits as i128 - (1i128 << w)
        } else {
            bits as i128
        }
    }

    /// §18.5.10: without an ordering every solution is equally likely, so a
    /// value is drawn in proportion to the solutions it leaves — estimated as
    /// the product of the other domains after propagating it. Variables
    /// under a dist (their draw follows the weights, not the count) and the
    /// variables `v` is solved before do not count. None when the estimate
    /// ran out of budget.
    fn csp_weigh(
        &mut self,
        csp: &Csp,
        aux: &Aux,
        watch: &[Vec<usize>],
        st: &mut St,
        v: usize,
    ) -> Option<i128> {
        let mut after = vec![false; csp.vars.len()];
        let mut todo: Vec<usize> = aux.user_succs[v].clone();
        while let Some(u) = todo.pop() {
            if !after[u] {
                after[u] = true;
                todo.extend(aux.user_succs[u].iter().copied());
            }
        }
        // A variable drawn by a dist in force counts no values; one whose
        // dist still waits on its guard does (`if (!cfg) total dist {…}`:
        // `cfg == 0` leaves `total` five values, `cfg == 1` all of them).
        let mut drawn = vec![false; csp.vars.len()];
        for (u, ds) in aux.dists.iter().enumerate() {
            if !ds.is_empty() {
                drawn[u] = self.csp_dist_state(csp, aux, st, u).0.is_some();
            }
        }
        let vals: Vec<i128> = st.d[v].iter().flat_map(|&(l, h)| l..=h).collect();
        let mut logs: Vec<f64> = Vec::with_capacity(vals.len());
        let mut seen = vec![false; csp.vars.len()];
        let mut touched: Vec<usize> = Vec::new();
        for &x in &vals {
            let mark = st.trail.len();
            let w0 = st.work;
            st.dirty.clear();
            st.set(v, vec![(x, x)]);
            let dirty = std::mem::take(&mut st.dirty);
            let r = self.csp_propagate(csp, watch, st, &Self::csp_watchers(watch, &dirty));
            let mut lg = f64::NEG_INFINITY;
            if r == Some(true) {
                lg = 0.0;
                for (u, old) in &st.trail[mark..] {
                    let u = *u;
                    if u == v || after[u] || drawn[u] || seen[u] {
                        continue;
                    }
                    seen[u] = true;
                    touched.push(u);
                    lg += Self::csp_log_size(aux, u, &st.d[u]) - Self::csp_log_size(aux, u, old);
                }
                for u in touched.drain(..) {
                    seen[u] = false;
                }
            }
            st.undo(mark);
            st.est_work += st.work - w0;
            st.work = w0;
            r?;
            logs.push(lg);
        }
        let top = logs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if top == f64::NEG_INFINITY {
            return None;
        }
        let ws: Vec<f64> = logs.iter().map(|l| (l - top).exp2()).collect();
        let total: f64 = ws.iter().sum();
        let mut r = self.cur_rng().random_range(0.0..total);
        for (i, w) in ws.iter().enumerate() {
            if r < *w {
                return Some(vals[i]);
            }
            r -= w;
        }
        Some(vals[vals.len() - 1])
    }

    fn csp_aux(csp: &Csp) -> Aux {
        let n = csp.vars.len();
        let mut aux = Aux {
            dists: vec![Vec::new(); n],
            preds: vec![Vec::new(); n],
            succs: vec![Vec::new(); n],
            guard_var: vec![false; n],
            user_succs: vec![Vec::new(); n],
            pop_c: vec![None; n],
            pop_x: vec![None; n],
            seg_hi: vec![Vec::new(); n],
            composite: vec![false; n],
        };
        for (i, d) in csp.dists.iter().enumerate() {
            aux.dists[d.var].push(i);
        }
        for &b in csp.segs.keys() {
            aux.composite[b] = aux.dists[b].is_empty();
        }
        for &x in csp.slice_vars.values() {
            aux.composite[x] = aux.dists[x].is_empty();
        }
        // A variable split into segments (`csp_split_slices`) that some
        // relation reads as a number (`last == base + n`), itself or through
        // a select spanning several segments, is decided from its most
        // significant segment down. Deciding the low bits first leaves a
        // conflict that only they cause (`base[15:0] + n` crossing a 64 KiB
        // page) to be refuted one value of the high bits at a time.
        let mut reads = vec![0u32; n];
        for node in &csp.nodes {
            let mut ds = Vec::new();
            node.deps(&csp.srcs, &mut ds);
            ds.sort_unstable();
            ds.dedup();
            for v in ds {
                reads[v] += 1;
            }
        }
        for (&b, segs) in &csp.segs {
            if csp.wide.contains(&b) || segs.len() < 2 {
                continue;
            }
            // Each is also read by its own definition (`csp_slice_links`).
            let mut ranges: Vec<(u32, u32)> = Vec::new();
            if reads[b] >= 2 {
                ranges.push((0, csp.vars[b].width));
            }
            for (&(sb, lo, w), &x) in &csp.slice_vars {
                if sb == b && reads[x] >= 2 {
                    ranges.push((lo, w));
                }
            }
            for (lo, w) in ranges {
                let inside: Vec<(u32, usize)> = segs
                    .iter()
                    .filter(|s| s.0 >= lo && s.0 + s.1 <= lo + w)
                    .map(|s| (s.0, s.2))
                    .collect();
                for &(slo, sv) in &inside {
                    for &(tlo, tv) in &inside {
                        if tlo > slo && !aux.seg_hi[sv].contains(&tv) {
                            aux.seg_hi[sv].push(tv);
                        }
                    }
                }
            }
        }
        // A counted variable is decided after its count, so it is drawn
        // with the count's number of ones (`csp_pick_val`).
        for (&x, &c) in &csp.pop {
            // A variable under a dist draws by its weights first (§18.5.4);
            // its count then follows it.
            if !aux.dists[x].is_empty() {
                continue;
            }
            aux.pop_c[x] = Some(c);
            aux.pop_x[c] = Some((x, csp.vars[x].width));
            if !aux.preds[x].contains(&c) {
                aux.preds[x].push(c);
                aux.succs[c].push(x);
            }
        }
        // §18.5.10: a variable split into segments (`csp_split_slices`) is
        // decided through them, so an ordering on the variable, or on a
        // select of it, orders the segments it spans as well: otherwise a
        // low segment of `last_addr` is guessed before `base` and the value
        // of `last == base + n` only ever meets it by chance.
        let spread = |vs: &[usize]| -> Vec<usize> {
            let mut out = vs.to_vec();
            for &v in vs {
                let (b, lo, w) = match csp.aux_base.get(&v) {
                    Some(&x) => x,
                    None => (v, 0, csp.vars[v].width),
                };
                if let Some(segs) = csp.segs.get(&b) {
                    out.extend(
                        segs.iter()
                            .filter(|s| s.0 < lo + w && s.0 + s.1 > lo)
                            .map(|s| s.2),
                    );
                }
            }
            out.sort_unstable();
            out.dedup();
            out
        };
        for (before, after) in &csp.order {
            let (before, after) = (spread(before), spread(after));
            for &a in &after {
                for &b in &before {
                    if a != b {
                        aux.preds[a].push(b);
                        aux.succs[b].push(a);
                        aux.user_succs[b].push(a);
                    }
                }
            }
        }
        fn conds(n: &Node, srcs: &[Src], out: &mut Vec<usize>) {
            match n {
                Node::And(ns) | Node::Or(ns) => ns.iter().for_each(|m| conds(m, srcs, out)),
                Node::If {
                    cond, then, els, ..
                } => {
                    cond.deps(srcs, out);
                    conds(then, srcs, out);
                    conds(els, srcs, out);
                }
                _ => {}
            }
        }
        // A `Fun` variable is decided after the variables its value is a
        // function of, so it is narrowed rather than guessed.
        fn funs(n: &Node, srcs: &[Src], aux: &mut Aux) {
            match n {
                Node::And(ns) | Node::Or(ns) => ns.iter().for_each(|m| funs(m, srcs, aux)),
                Node::If { then, els, .. } => {
                    funs(then, srcs, aux);
                    funs(els, srcs, aux);
                }
                Node::Fun { var, other, .. } => {
                    for &d in &srcs[*other].deps {
                        if d != *var && !aux.preds[*var].contains(&d) {
                            aux.preds[*var].push(d);
                            aux.succs[d].push(*var);
                        }
                    }
                }
                _ => {}
            }
        }
        for nd in &csp.nodes {
            funs(nd, &csp.srcs, &mut aux);
        }
        // `x == k*y + c` (|k| > 1) defines `x` by the others: decided after
        // them it is narrowed to its value instead of guessed among the
        // values no `y` reaches (`max == 40'h800_0000 * total - 1`). Only for
        // a solve with rand sub-objects (§18.5.9), whose sets are larger.
        if csp.has_subs {
            for nd in &csp.nodes {
                let Node::Lin {
                    lin, rel: Rel::Eq, ..
                } = nd
                else {
                    continue;
                };
                if lin.t.len() < 2 || !lin.t.iter().any(|t| t.1.abs() > 1) {
                    continue;
                }
                let units: Vec<usize> = lin
                    .t
                    .iter()
                    .filter(|t| t.1.abs() == 1)
                    .map(|t| t.0)
                    .collect();
                if units.len() != 1 {
                    continue;
                }
                let x = units[0];
                // A split `x` is decided through its segments.
                let mut xs = vec![x];
                if let Some(segs) = csp.segs.get(&x) {
                    xs.extend(segs.iter().map(|s| s.2));
                }
                xs.extend(
                    csp.slice_vars
                        .iter()
                        .filter(|((b, ..), _)| *b == x)
                        .map(|(_, &sv)| sv),
                );
                for &(d, _) in &lin.t {
                    if d == x || aux.preds[d].contains(&x) {
                        continue;
                    }
                    for &y in &xs {
                        if y != d && !aux.preds[y].contains(&d) {
                            aux.preds[y].push(d);
                            aux.succs[d].push(y);
                        }
                    }
                }
            }
        }
        let mut gv = Vec::new();
        for nd in &csp.nodes {
            conds(nd, &csp.srcs, &mut gv);
        }
        // The inputs of a `Fun` relation on a counted `$countones` count.
        fn pop_inputs(n: &Node, csp: &Csp, aux: &Aux, out: &mut Vec<usize>) {
            match n {
                Node::And(ns) | Node::Or(ns) => {
                    ns.iter().for_each(|m| pop_inputs(m, csp, aux, out))
                }
                Node::If { then, els, .. } => {
                    pop_inputs(then, csp, aux, out);
                    pop_inputs(els, csp, aux, out);
                }
                Node::Fun { var, other, .. } if aux.pop_x[*var].is_some() => {
                    out.extend(csp.srcs[*other].deps.iter().copied());
                }
                _ => {}
            }
        }
        for nd in &csp.nodes {
            pop_inputs(nd, csp, &aux, &mut gv);
        }
        for v in gv {
            aux.guard_var[v] = true;
        }
        aux
    }

    /// Run the queued nodes to a fixpoint. `Some(false)` on a conflict,
    /// `None` when the work budget is gone.
    fn csp_propagate(
        &mut self,
        csp: &Csp,
        watch: &[Vec<usize>],
        st: &mut St,
        start: &[usize],
    ) -> Option<bool> {
        let n = csp.nodes.len();
        let mut queued = vec![false; n];
        // Bounds propagation around a cycle of relations that has no
        // solution (`a == b`, both tied to fixed, different low parts)
        // narrows by one step per round and can run through a whole
        // 40-bit range. A node re-run far more often than any chain of the
        // problem needs stops the round: the branch is abandoned, and marked
        // as cut so that it never counts as a proof of unsatisfiability.
        let stall = 1000 + 2 * csp.vars.len() as u32;
        let mut runs = vec![0u32; n];
        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        for &i in start {
            if !queued[i] {
                queued[i] = true;
                queue.push_back(i);
            }
        }
        while let Some(i) = queue.pop_front() {
            queued[i] = false;
            runs[i] += 1;
            if runs[i] > stall {
                st.cut = true;
                return Some(false);
            }
            st.work += 1;
            if st.work >= WORK_BUDGET {
                return None;
            }
            st.dirty.clear();
            if !self.csp_enforce(csp, &csp.nodes[i], st) {
                return Some(false);
            }
            let dirty = std::mem::take(&mut st.dirty);
            for v in dirty {
                for &j in &watch[v] {
                    if !queued[j] {
                        queued[j] = true;
                        queue.push_back(j);
                    }
                }
            }
        }
        Some(true)
    }

    /// §18.5: `var OP other` with `other` a function of a few variables of
    /// small domains: `other` takes one of the values its inputs' remaining
    /// combinations give, so `var` is narrowed to them (`total ==
    /// per * (n + 1)` bounds `total` before `per` and `n` are decided).
    /// True (nothing learnt) when the inputs have too many combinations.
    fn csp_fun_range(
        &mut self,
        csp: &Csp,
        st: &mut St,
        var: usize,
        op: BinaryOp,
        other: usize,
    ) -> bool {
        const MAX_COMBOS: u128 = 256;
        let deps = &csp.srcs[other].deps;
        if deps.is_empty() || deps.len() > 4 || deps.contains(&var) {
            return true;
        }
        let mut n: u128 = 1;
        for &d in deps {
            n = n.saturating_mul(dom_size(&st.d[d]));
            if n > MAX_COMBOS {
                return true;
            }
        }
        st.work += n as u64;
        let (vw, vs) = (csp.vars[var].width, csp.vars[var].signed);
        let doms: Vec<Vec<i128>> = deps
            .iter()
            .map(|&d| {
                st.d[d]
                    .iter()
                    .flat_map(|&(l, h)| l..=h)
                    .collect::<Vec<i128>>()
            })
            .collect();
        let mut idx = vec![0usize; deps.len()];
        let mut vals: Vec<(i128, i128)> = Vec::new();
        loop {
            for (k, &d) in deps.iter().enumerate() {
                self.csp_write(csp, d, doms[k][idx[k]]);
            }
            let s = &csp.srcs[other];
            let frame = self.csp_frame(&s.binds);
            self.push_local_frame(frame);
            let c = match &s.item {
                SrcItem::Expr(e) => self.eval_expr(e),
                SrcItem::Item(_) => Value::new(1),
            };
            self.pop_local_frame();
            if c.has_unknown() || c.width > 64 || (vs != c.is_signed && !(!vs && c.width <= vw)) {
                return true;
            }
            let k = if c.is_signed {
                let x = c.to_u64().unwrap_or(0);
                let sh = 64 - c.width.max(1);
                (((x << sh) as i64) >> sh) as i128
            } else {
                c.to_u64().unwrap_or(0) as i128
            };
            vals.push((k, k));
            let mut j = 0;
            loop {
                if j == idx.len() {
                    let set = dom_norm(vals);
                    const BIG: i128 = i128::MAX / 4;
                    let (lo, hi) = (set.first().unwrap().0, set.last().unwrap().1);
                    let nd = match op {
                        BinaryOp::Eq | BinaryOp::CaseEq => dom_meet(&st.d[var], &set),
                        BinaryOp::Lt => dom_clip(&st.d[var], -BIG, hi - 1),
                        BinaryOp::Leq => dom_clip(&st.d[var], -BIG, hi),
                        BinaryOp::Gt => dom_clip(&st.d[var], lo + 1, BIG),
                        BinaryOp::Geq => dom_clip(&st.d[var], lo, BIG),
                        _ => return true,
                    };
                    return st.set(var, nd);
                }
                idx[j] += 1;
                if idx[j] < doms[j].len() {
                    break;
                }
                idx[j] = 0;
                j += 1;
            }
        }
    }

    /// Make `n` hold under the current domains; false on a conflict.
    fn csp_enforce(&mut self, csp: &Csp, n: &Node, st: &mut St) -> bool {
        match n {
            Node::True => true,
            Node::False => false,
            Node::Lin {
                lin,
                rel,
                fits,
                src,
                sneg,
                wrap,
            } => {
                if !st.fits_hold(fits) {
                    if let Some(wr) = *wrap
                        && st.fits_hold_mod(&fits[..wr.n], wr.w)
                        && let Some(ok) = match rel {
                            Rel::Eq => Self::csp_wrap_eq(lin, wr.w, st),
                            _ => Self::csp_wrap_rel(lin, *rel, fits, wr, st),
                        }
                    {
                        return ok;
                    }
                    return !self.csp_src_fixed(csp, st, *src)
                        || self.csp_eval_src(csp, st, *src) != *sneg;
                }
                match rel {
                    Rel::Le => st.prop_bound(lin, true),
                    Rel::Eq => st.prop_bound(lin, true) && st.prop_bound(lin, false),
                    Rel::Ne => {
                        let mut free: Option<(usize, i128)> = None;
                        let mut rest = lin.k;
                        for &(v, c) in &lin.t {
                            match st.fixed(v) {
                                Some(x) => rest += c * x,
                                None if free.is_none() => free = Some((v, c)),
                                None => return true,
                            }
                        }
                        match free {
                            None => rest != 0,
                            Some((v, c)) => {
                                if (-rest) % c != 0 {
                                    return true;
                                }
                                let x = -rest / c;
                                let nd = dom_minus(&st.d[v], &vec![(x, x)]);
                                st.set(v, nd)
                            }
                        }
                    }
                }
            }
            Node::In {
                lin,
                set,
                neg,
                fits,
                src,
                wrap,
            } => {
                if !st.fits_hold(fits) {
                    if let Some(wr) = *wrap
                        && st.fits_hold_mod(&fits[..wr.n], wr.w)
                        && let Some(ok) = Self::csp_wrap_set(lin, set, *neg, wr.w, wr.s, st)
                    {
                        return ok;
                    }
                    return !self.csp_src_fixed(csp, st, *src)
                        || self.csp_eval_src(csp, st, *src) != *neg;
                }
                if let [(v, c)] = lin.t[..] {
                    if c == 1 || c == -1 {
                        let allowed: Dom = if c == 1 {
                            set.iter().map(|&(l, h)| (l - lin.k, h - lin.k)).collect()
                        } else {
                            dom_norm(set.iter().map(|&(l, h)| (lin.k - h, lin.k - l)).collect())
                        };
                        let nd = if *neg {
                            dom_minus(&st.d[v], &allowed)
                        } else {
                            dom_meet(&st.d[v], &allowed)
                        };
                        return st.set(v, nd);
                    }
                }
                let (lo, hi) = st.lin_bounds(lin);
                if lo == hi {
                    return dom_has(set, lo) != *neg;
                }
                if *neg || set.is_empty() {
                    return !set.is_empty() || *neg;
                }
                let (smin, smax) = (set[0].0, set[set.len() - 1].1);
                let (Some(up), Some(dn)) = (lin.clone().shift(-smax), lin.clone().shift(-smin))
                else {
                    return true;
                };
                st.prop_bound(&up, true) && st.prop_bound(&dn, false)
            }
            Node::AllDiff(vs) => {
                let mut taken: Vec<(i128, i128)> = Vec::new();
                for &v in vs {
                    if let Some(x) = st.fixed(v) {
                        taken.push((x, x));
                    }
                }
                let before = taken.len();
                let taken = dom_norm(taken);
                if dom_size(&taken) < before as u128 {
                    return false;
                }
                let mut free = 0u128;
                let mut pool: Vec<(i128, i128)> = Vec::new();
                for &v in vs {
                    if st.fixed(v).is_some() {
                        continue;
                    }
                    let nd = dom_minus(&st.d[v], &taken);
                    if !st.set(v, nd) {
                        return false;
                    }
                    free += 1;
                    pool.extend(st.d[v].iter().copied());
                }
                // pigeonhole: the free variables need that many values
                free == 0 || dom_size(&dom_norm(pool)) >= free
            }
            Node::Pop { x, c, w } => {
                if let Some(v) = st.fixed(*x) {
                    let n = Self::bits_at(v, *w).count_ones() as i128;
                    let nd = dom_meet(&st.d[*c], &vec![(n, n)]);
                    return st.set(*c, nd);
                }
                // A small domain of x: keep the values whose count is still
                // possible, and the counts some value still has.
                if dom_size(&st.d[*x]) <= 64 {
                    let mut xs = Vec::new();
                    let mut ns = Vec::new();
                    for &(l, h) in &st.d[*x] {
                        for v in l..=h {
                            let n = Self::bits_at(v, *w).count_ones() as i128;
                            if dom_has(&st.d[*c], n) {
                                xs.push((v, v));
                                ns.push((n, n));
                            }
                        }
                    }
                    return st.set(*x, dom_norm(xs)) && st.set(*c, dom_norm(ns));
                }
                // Otherwise bound the count by x's range: a value in [lo, hi]
                // with 0 <= lo has at most max(ones(hi), bits(hi) - 1) ones,
                // and at least one when lo > 0.
                let (lo, hi) = st.bounds(*x);
                if lo >= 0 {
                    let h = hi as u128;
                    let bl = 128 - h.leading_zeros();
                    let cmax = h.count_ones().max(bl.saturating_sub(1)) as i128;
                    let nd = dom_clip(&st.d[*c], (lo > 0) as i128, cmax);
                    return st.set(*c, nd);
                }
                true
            }
            Node::Eval { src, neg } => {
                if self.csp_src_fixed(csp, st, *src) {
                    return self.csp_eval_src(csp, st, *src) != *neg;
                }
                self.csp_eval_forward(csp, st, *src, *neg)
            }
            Node::Fun {
                var,
                op,
                other,
                whole,
                neg,
            } => {
                if !self.csp_src_fixed(csp, st, *other) {
                    if csp.srcs[*other].stale {
                        return true;
                    }
                    return !csp.has_subs || self.csp_fun_range(csp, st, *var, *op, *other);
                }
                if self.csp_src_fixed(csp, st, *whole) {
                    return self.csp_eval_src(csp, st, *whole) != *neg;
                }
                let c = self.csp_eval_value(csp, st, *other);
                if c.has_unknown() {
                    // §11.4.4: a relation with an x/z operand is not true.
                    return false;
                }
                let (vw, vs) = (csp.vars[*var].width, csp.vars[*var].signed);
                let k: i128 = match (vs, c.is_signed) {
                    (false, false) => match c.to_u64() {
                        Some(x) => x as i128,
                        None => return true,
                    },
                    (true, true) if c.width <= 64 => {
                        let x = c.to_u64().unwrap_or(0);
                        let sh = 64 - c.width.max(1);
                        (((x << sh) as i64) >> sh) as i128
                    }
                    // §11.8.1/§11.8.2: with an unsigned side the relation is
                    // unsigned, at the wider width, and each operand is
                    // zero-extended. A signed variable that cannot be
                    // negative (a `$countones` count against `2**size`)
                    // keeps its value; the other side, at least as wide,
                    // is its own self-determined value.
                    (true, false) if st.bounds(*var).0 >= 0 && c.width >= vw => match c.to_u64() {
                        Some(x) => x as i128,
                        None => return true,
                    },
                    // §11.8.2: an unsigned variable makes the relation
                    // unsigned; the signed side is zero-extended to it.
                    (false, true) if c.width <= vw => match c.to_u64() {
                        Some(x) => x as i128,
                        None => return true,
                    },
                    // Mixed signedness: leave it to the final judgement.
                    _ => return true,
                };
                let _ = vw;
                const BIG: i128 = i128::MAX / 4;
                let nd = match op {
                    BinaryOp::Eq | BinaryOp::CaseEq => dom_meet(&st.d[*var], &vec![(k, k)]),
                    BinaryOp::Neq | BinaryOp::CaseNeq => dom_minus(&st.d[*var], &vec![(k, k)]),
                    BinaryOp::Lt => dom_clip(&st.d[*var], -BIG, k - 1),
                    BinaryOp::Leq => dom_clip(&st.d[*var], -BIG, k),
                    BinaryOp::Gt => dom_clip(&st.d[*var], k + 1, BIG),
                    _ => dom_clip(&st.d[*var], k, BIG),
                };
                st.set(*var, nd)
            }
            Node::And(ns) => ns.iter().all(|m| self.csp_enforce(csp, m, st)),
            Node::Or(ns) => {
                let mut open: Option<&Node> = None;
                let mut n_open = 0;
                for m in ns {
                    match self.csp_status(csp, m, st) {
                        Some(true) => return true,
                        Some(false) => {}
                        None => {
                            n_open += 1;
                            open = Some(m);
                        }
                    }
                }
                match n_open {
                    0 => false,
                    1 => self.csp_enforce(csp, open.unwrap(), st),
                    _ => true,
                }
            }
            Node::If {
                cond,
                ncond,
                then,
                els,
            } => match self.csp_status(csp, cond, st) {
                Some(true) => self.csp_enforce(csp, then, st),
                Some(false) => self.csp_enforce(csp, els, st),
                None => {
                    if self.csp_status(csp, then, st) == Some(false) {
                        return self.csp_enforce(csp, ncond, st);
                    }
                    if self.csp_status(csp, els, st) == Some(false) {
                        return self.csp_enforce(csp, cond, st);
                    }
                    true
                }
            },
        }
    }

    /// §11.4.5/§11.8.2: an equality compared at `w` bits holds exactly when
    /// its two sides agree modulo 2^w, so a side that wraps (`a[39:16] ==
    /// b + c` with a 24-bit sum that carries out) still pins the one
    /// variable left open: `x == r (mod 2^w)` keeps the values of the
    /// residue `r` in its domain. None when that does not apply (several
    /// open variables, a coefficient other than ±1, too many candidates).
    fn csp_wrap_eq(lin: &Lin, w: u32, st: &mut St) -> Option<bool> {
        if w == 0 || w > 64 {
            return None;
        }
        let m: i128 = 1i128 << w;
        let mut free: Option<(usize, i128)> = None;
        let mut rest = lin.k;
        for &(v, c) in &lin.t {
            match st.fixed(v) {
                Some(x) => rest += c * x,
                None if free.is_none() => free = Some((v, c)),
                None => return None,
            }
        }
        let Some((v, c)) = free else {
            return Some(rest.rem_euclid(m) == 0);
        };
        if c != 1 && c != -1 {
            return None;
        }
        // c·x ≡ -rest, and c is its own inverse
        let r = (-rest * c).rem_euclid(m);
        let (lo, hi) = st.bounds(v);
        let mut x = r + ceil_div(lo - r, m) * m;
        let mut cands: Vec<(i128, i128)> = Vec::new();
        while x <= hi {
            if cands.len() >= 8 {
                return None;
            }
            cands.push((x, x));
            x += m;
        }
        let nd = dom_meet(&st.d[v], &cands);
        Some(st.set(v, nd))
    }

    /// §11.4.13/§11.8.2: `lin` computed at `w` bits of signedness `s` is
    /// its value wrapped into that type, so with one variable left open
    /// (`c + 4000 inside {[4003:4004]}`, `2 * c inside {[10:11]}`) keep the
    /// values whose wrapped result lies in `set` (outside it with `neg`).
    /// Without this a relation whose operand may overflow waited for the
    /// evaluator until every variable was guessed. None when that does not
    /// apply (several open variables, too many candidates).
    fn csp_wrap_set(lin: &Lin, set: &Dom, neg: bool, w: u32, s: bool, st: &mut St) -> Option<bool> {
        if w == 0 || w > 64 {
            return None;
        }
        let m: i128 = 1i128 << w;
        let (rlo, rhi) = ws_range(w, s);
        let mut free: Option<(usize, i128)> = None;
        let mut rest = lin.k;
        for &(v, c) in &lin.t {
            match st.fixed(v) {
                Some(x) => rest += c * x,
                None if free.is_none() => free = Some((v, c)),
                None => return None,
            }
        }
        let target: Dom = if neg {
            dom_minus(&vec![(rlo, rhi)], set)
        } else {
            dom_clip(set, rlo, rhi)
        };
        let Some((v, c)) = free else {
            return Some(dom_has(&target, rlo + (rest - rlo).rem_euclid(m)));
        };
        let (lo, hi) = st.bounds(v);
        if lo > hi {
            return Some(false);
        }
        const MAX_CANDS: usize = 64;
        let mut allowed: Vec<(i128, i128)> = Vec::new();
        if c == 1 || c == -1 {
            // y = c·x + rest over x in [lo, hi] meets the target once per
            // multiple of 2^w it spans.
            let (ylo, yhi) = if c == 1 {
                (lo + rest, hi + rest)
            } else {
                (rest - hi, rest - lo)
            };
            let (j0, j1) = (floor_div(ylo - rlo, m), floor_div(yhi - rlo, m));
            if j1 - j0 > 8 {
                return None;
            }
            for j in j0..=j1 {
                for &(a, b) in &target {
                    let (a, b) = ((a + j * m).max(ylo), (b + j * m).min(yhi));
                    if a <= b {
                        allowed.push(if c == 1 {
                            (a - rest, b - rest)
                        } else {
                            (rest - b, rest - a)
                        });
                    }
                }
            }
        } else {
            // c·x ≡ t - rest (mod 2^w) for each target value t: solvable
            // when 2^tz(c) divides it, then x is fixed modulo 2^(w - tz).
            if dom_size(&target) > MAX_CANDS as u128 {
                return None;
            }
            let tz = c.trailing_zeros().min(w);
            if tz == w {
                return Some(dom_has(&target, rlo + (rest - rlo).rem_euclid(m)));
            }
            let g = 1i128 << tz;
            let mg = m >> tz;
            let a = ((c >> tz).rem_euclid(mg)) as u128;
            // Newton iteration: the inverse of an odd `a` modulo 2^128.
            let mut inv: u128 = a;
            for _ in 0..7 {
                inv = inv.wrapping_mul(2u128.wrapping_sub(a.wrapping_mul(inv)));
            }
            let mask = (mg - 1) as u128;
            for &(ta, tb) in &target {
                for t in ta..=tb {
                    let r = (t - rest).rem_euclid(m);
                    if r % g != 0 {
                        continue;
                    }
                    let x0 = (((r / g) as u128).wrapping_mul(inv) & mask) as i128;
                    let mut x = x0 + ceil_div(lo - x0, mg) * mg;
                    while x <= hi {
                        if allowed.len() >= MAX_CANDS {
                            return None;
                        }
                        allowed.push((x, x));
                        x += mg;
                    }
                }
            }
        }
        let nd = dom_meet(&st.d[v], &dom_norm(allowed));
        Some(st.set(v, nd))
    }

    /// §11.8.2: a relation (`<=`, `!=`, …) compares its two sides each
    /// wrapped at `w` bits, so once one side is fixed it is a set the other
    /// side's wrapped value must lie in (`c + v0 >= v0 + 3` once `v0` is
    /// known), handed to `csp_wrap_set`. None when both sides are open.
    fn csp_wrap_rel(lin: &Lin, rel: Rel, fits: &[Fit], wr: Wrap, st: &mut St) -> Option<bool> {
        let (la, lb) = (&fits.get(wr.n)?.lin, &fits.get(wr.n + 1)?.lin);
        let (w, s, sg) = (wr.w, wr.s, wr.sg);
        if w == 0 || w > 64 {
            return None;
        }
        let m: i128 = 1i128 << w;
        let (rlo, rhi) = ws_range(w, s);
        let wrapv = |y: i128| rlo + (y - rlo).rem_euclid(m);
        // `lin` is sg·(la - lb) + δ
        let delta = lin.k - sg * (la.k - lb.k);
        let value = |l: &Lin, st: &St| -> Option<i128> {
            let mut x = l.k;
            for &(v, c) in &l.t {
                x += c * st.fixed(v)?;
            }
            Some(x)
        };
        let holds = |d: i128| match rel {
            Rel::Le => d <= 0,
            Rel::Eq => d == 0,
            Rel::Ne => d != 0,
        };
        let (va, vb) = (value(la, st), value(lb, st));
        // The open side `x` and the other side's wrapped value `k`:
        // sg·(x - k) + δ (left open) or sg·(k - x) + δ (right open).
        let (x, k, left) = match (va, vb) {
            (Some(a), Some(b)) => return Some(holds(sg * (wrapv(a) - wrapv(b)) + delta)),
            (None, Some(b)) => (la, wrapv(b), true),
            (Some(a), None) => (lb, wrapv(a), false),
            (None, None) => return None,
        };
        let (set, neg) = match rel {
            Rel::Le => {
                // x <= k - δ when the open side counts up, else x >= k + δ
                let up = (sg == 1) == left;
                let r = if up {
                    (rlo, k - delta)
                } else {
                    (k + delta, rhi)
                };
                (if r.0 <= r.1 { vec![r] } else { Vec::new() }, false)
            }
            Rel::Eq | Rel::Ne => {
                // sg·(±(x - k)) + δ == 0: x == k ∓ sg·δ
                let t = if left { k - sg * delta } else { k + sg * delta };
                (vec![(t, t)], rel == Rel::Ne)
            }
        };
        Self::csp_wrap_set(x, &set, neg, w, s, st)
    }

    /// Entailed (`Some(true)`), refuted (`Some(false)`) or open.
    fn csp_status(&mut self, csp: &Csp, n: &Node, st: &St) -> Option<bool> {
        match n {
            Node::True => Some(true),
            Node::False => Some(false),
            Node::Lin {
                lin,
                rel,
                fits,
                src,
                sneg,
                ..
            } => {
                if !st.fits_hold(fits) {
                    if self.csp_src_fixed(csp, st, *src) {
                        return Some(self.csp_eval_src(csp, st, *src) != *sneg);
                    }
                    return None;
                }
                let (lo, hi) = st.lin_bounds(lin);
                let eq = if lo == 0 && hi == 0 {
                    Some(true)
                } else if lo > 0 || hi < 0 {
                    Some(false)
                } else {
                    None
                };
                match rel {
                    Rel::Le if hi <= 0 => Some(true),
                    Rel::Le if lo > 0 => Some(false),
                    Rel::Le => None,
                    Rel::Eq => eq,
                    Rel::Ne => eq.map(|b| !b),
                }
            }
            Node::In {
                lin,
                set,
                neg,
                fits,
                src,
                ..
            } => {
                if !st.fits_hold(fits) {
                    if self.csp_src_fixed(csp, st, *src) {
                        return Some(self.csp_eval_src(csp, st, *src) != *neg);
                    }
                    return None;
                }
                let (lo, hi) = st.lin_bounds(lin);
                if lo == hi {
                    return Some(dom_has(set, lo) != *neg);
                }
                if dom_clip(set, lo, hi).is_empty() {
                    return Some(*neg);
                }
                if set.iter().any(|&(l, h)| l <= lo && hi <= h) {
                    return Some(!*neg);
                }
                None
            }
            Node::Pop { x, c, w } => {
                let n = Self::bits_at(st.fixed(*x)?, *w).count_ones() as i128;
                if !dom_has(&st.d[*c], n) {
                    return Some(false);
                }
                st.fixed(*c).map(|_| true)
            }
            Node::AllDiff(vs) => {
                if !st.all_fixed(vs) {
                    return None;
                }
                let mut xs: Vec<i128> = vs.iter().map(|&v| st.fixed(v).unwrap()).collect();
                xs.sort_unstable();
                Some(xs.windows(2).all(|w| w[0] != w[1]))
            }
            Node::Eval { src, neg }
            | Node::Fun {
                whole: src, neg, ..
            } => {
                if self.csp_src_fixed(csp, st, *src) {
                    Some(self.csp_eval_src(csp, st, *src) != *neg)
                } else {
                    None
                }
            }
            Node::And(ns) => {
                let mut all = true;
                for m in ns {
                    match self.csp_status(csp, m, st) {
                        Some(false) => return Some(false),
                        Some(true) => {}
                        None => all = false,
                    }
                }
                all.then_some(true)
            }
            Node::Or(ns) => {
                let mut none = true;
                for m in ns {
                    match self.csp_status(csp, m, st) {
                        Some(true) => return Some(true),
                        Some(false) => {}
                        None => none = false,
                    }
                }
                none.then_some(false)
            }
            Node::If {
                cond, then, els, ..
            } => match self.csp_status(csp, cond, st) {
                Some(true) => self.csp_status(csp, then, st),
                Some(false) => self.csp_status(csp, els, st),
                None => {
                    let (t, e) = (
                        self.csp_status(csp, then, st),
                        self.csp_status(csp, els, st),
                    );
                    if t.is_some() && t == e { t } else { None }
                }
            },
        }
    }

    /// A constraint left to the evaluator with one variable still open, of
    /// a few values: keep the values for which it holds (`(base &
    /// ((1 << sz) - 1)) == 0` leaves the aligned low bits of `base` once
    /// `sz` and its high bits are fixed). Without this the search guessed
    /// the last variable and judged the guess, and a conflict caused by an
    /// earlier decision was refuted one value of a later one at a time.
    fn csp_eval_forward(&mut self, csp: &Csp, st: &mut St, src: usize, neg: bool) -> bool {
        if csp.srcs[src].stale {
            return true;
        }
        let mut open = None;
        for &v in &csp.srcs[src].deps {
            if st.fixed(v).is_none() {
                if open.is_some() {
                    return true;
                }
                open = Some(v);
            }
        }
        let Some(v) = open else {
            return true;
        };
        let n = dom_size(&st.d[v]);
        if n > EST_MAX_DOM || csp.wide.contains(&v) || csp.segs.contains_key(&v) {
            return true;
        }
        // Writing the fixed variable it is part of would overwrite it.
        if let Some(&(b, ..)) = csp.aux_base.get(&v)
            && csp.srcs[src].deps.contains(&b)
        {
            return true;
        }
        st.work += n as u64;
        let vals: Vec<i128> = (0..n).map(|k| dom_nth(&st.d[v], k)).collect();
        let mut keep = Vec::with_capacity(vals.len());
        for x in vals {
            self.csp_write(csp, v, x);
            if self.csp_eval_src(csp, st, src) != neg {
                keep.push((x, x));
            }
        }
        st.set(v, dom_norm(keep))
    }

    fn csp_src_fixed(&self, csp: &Csp, st: &St, src: usize) -> bool {
        !csp.srcs[src].stale && st.all_fixed(&csp.srcs[src].deps)
    }

    /// Whether `item` names an array of the size phase (`Csp::sizes`).
    fn csp_src_sized(csp: &Csp, item: &SrcItem) -> bool {
        fn names(csp: &Csp, e: &Expression) -> bool {
            let mut hit = false;
            Simulator::walk_operands(e, &mut |x| match &x.kind {
                ExprKind::Ident(h)
                    if h.path.iter().any(|s| csp.sizes.contains_key(&s.name.name)) =>
                {
                    hit = true;
                }
                ExprKind::WithClause { expr, filter } => {
                    hit |= names(csp, expr) || names(csp, filter);
                }
                _ => {}
            });
            hit
        }
        let mut hit = false;
        let mut look = |e: &Expression| hit |= names(csp, e);
        match item {
            SrcItem::Expr(e) => look(e),
            SrcItem::Item(it) => Self::csp_item_walk(it, &mut |i| match i {
                ConstraintItem::Expr(e) => look(e),
                ConstraintItem::Inside { expr, range, .. } => {
                    look(expr);
                    for r in range {
                        match r {
                            ConstraintRange::Value(v) => look(v),
                            ConstraintRange::Range { lo, hi } => {
                                look(lo);
                                look(hi);
                            }
                        }
                    }
                }
                ConstraintItem::Implication { condition, .. }
                | ConstraintItem::IfElse { condition, .. } => look(condition),
                ConstraintItem::Foreach { array, .. } => look(array),
                ConstraintItem::Unique { exprs, .. } => exprs.iter().for_each(&mut look),
                _ => {}
            }),
        }
        hit
    }

    /// The local frame to evaluate under with `binds` (foreach indices)
    /// bound. A class solve runs in the object's method frame, which holds
    /// nothing else; a scope solve keeps the caller's locals visible.
    fn csp_frame(&self, binds: &[(String, Value)]) -> HashMap<String, Value> {
        let mut frame: HashMap<String, Value> = if CSP_SCOPE.with(|c| c.get()) {
            self.local_stack.last().cloned().unwrap_or_default()
        } else {
            HashMap::default()
        };
        frame.extend(binds.iter().cloned());
        frame
    }

    /// Judge a source constraint with the ordinary evaluator: the fixed
    /// variables it reads are written to the object first.
    fn csp_eval_src(&mut self, csp: &Csp, st: &St, src: usize) -> bool {
        let s = &csp.srcs[src];
        for &v in &s.deps {
            if let Some(x) = st.fixed(v) {
                self.csp_write(csp, v, x);
            }
        }
        let frame = self.csp_frame(&s.binds);
        self.push_local_frame(frame);
        let ok = match &s.item {
            SrcItem::Expr(e) => self.cons_expr_true(e),
            SrcItem::Item(it) => self.check_constraint_item_impl(it),
        };
        self.pop_local_frame();
        ok
    }

    /// The value of a source expression with its fixed variables written.
    fn csp_eval_value(&mut self, csp: &Csp, st: &St, src: usize) -> Value {
        let s = &csp.srcs[src];
        for &v in &s.deps {
            if let Some(x) = st.fixed(v) {
                self.csp_write(csp, v, x);
            }
        }
        let frame = self.csp_frame(&s.binds);
        self.push_local_frame(frame);
        let v = match &s.item {
            SrcItem::Expr(e) => self.eval_expr(e),
            SrcItem::Item(_) => Value::new(1),
        };
        self.pop_local_frame();
        v
    }

    fn csp_write(&mut self, csp: &Csp, v: usize, x: i128) {
        let var = &csp.vars[v];
        let mut val = Value::from_u64(Self::bits_at(x, var.width) as u64, var.width);
        val.is_signed = var.signed;
        if matches!(var.key, VarKey::Aux) {
            // A segment (a packed-struct field or a constant select,
            // §7.2.1/§11.5.1) is written into the bits of the variable it is
            // part of: an evaluator node that reads the field (`r.f ==
            // (K >> p)`) depends on the segment alone, so the base variable
            // is not yet fixed — and therefore not written — when the node is
            // judged.
            let Some(&(base, lo, w)) = csp.aux_base.get(&v) else {
                return;
            };
            let Some(mut cur) = self.csp_read(csp, base) else {
                return;
            };
            for k in 0..w {
                let bit = if (x >> k) & 1 == 1 {
                    LogicBit::One
                } else {
                    LogicBit::Zero
                };
                cur.set_bit((lo + k) as usize, bit);
            }
            self.csp_write_value(csp, base, cur);
            return;
        }
        self.csp_write_value(csp, v, val);
    }

    /// The current value of base variable `v` (not a segment).
    fn csp_read(&mut self, csp: &Csp, v: usize) -> Option<Value> {
        let var = &csp.vars[v];
        let cur = if let Some(lvals) = &csp.scope {
            match &var.key {
                VarKey::Prop(n) => {
                    let lv = lvals.get(n)?.clone();
                    Some(self.eval_expr(&lv))
                }
                VarKey::Elem(k) => self.get_signal_value_by_name(k),
                VarKey::Sub(..) | VarKey::Aux => None,
            }
        } else {
            match &var.key {
                VarKey::Prop(n) => self
                    .heap
                    .get(csp.handle)
                    .and_then(|o| o.as_ref())
                    .and_then(|i| i.properties.get(n).cloned()),
                VarKey::Elem(k) => self.read_coll_elem(k),
                VarKey::Sub(h, n) => self
                    .heap
                    .get(*h)
                    .and_then(|o| o.as_ref())
                    .and_then(|i| i.properties.get(n).cloned()),
                VarKey::Aux => None,
            }
        };
        let mut cur = cur.unwrap_or_else(|| Value::zero(var.width));
        if cur.width != var.width {
            cur = cur.resize(var.width);
        }
        cur.is_signed = var.signed;
        Some(cur)
    }

    fn csp_write_value(&mut self, csp: &Csp, v: usize, val: Value) {
        let var = &csp.vars[v];
        if let Some(lvals) = &csp.scope {
            match &var.key {
                VarKey::Prop(n) => {
                    if let Some(lv) = lvals.get(n) {
                        let lv = lv.clone();
                        self.assign_value(&lv, &val);
                    }
                }
                VarKey::Elem(k) => self.set_signal_value_by_name(k, val),
                VarKey::Sub(..) | VarKey::Aux => {}
            }
            return;
        }
        match &var.key {
            VarKey::Prop(n) => {
                if let Some(Some(inst)) = self.heap.get_mut(csp.handle) {
                    inst.properties.insert(n.clone(), val);
                }
            }
            VarKey::Elem(k) => self.write_coll_elem(k, val),
            VarKey::Aux => {}
            VarKey::Sub(h, n) => {
                if let Some(Some(inst)) = self.heap.get_mut(*h) {
                    inst.properties.insert(n.clone(), val);
                }
            }
        }
    }

    /// Add the rand scalars of the rand sub-object `sub` (member `prop` of
    /// the object being randomized, or `arr[k]` for an element of an object
    /// array) as `prop.<name>` variables, and return its class constraints
    /// rewritten to name them through `prop`. A member wider than 64 bits (a
    /// wide packed struct) is solved through its segments like the parent's
    /// own (`csp_wide_split`), or left as drawn when no constraint of the
    /// solve (`parent`'s or the sub-object's) reads it. A `foreach` over a
    /// fixed-shape member of the sub-object is unrolled. None when the
    /// sub-object holds anything this solver does not model: a nested
    /// object, a collection, a real, a randc, an unpacked struct, or a
    /// constraint that calls a method.
    fn csp_add_sub(
        &mut self,
        csp: &mut Csp,
        prop: &str,
        sub: usize,
        parent: &[ClassConstraint],
    ) -> Option<Vec<ConstraintItem>> {
        let class_name = self.heap.get(sub)?.as_ref()?.class_name.clone();
        let rand_disabled = self
            .rand_mode_disabled
            .get(&sub)
            .cloned()
            .unwrap_or_default();
        let con_disabled = self
            .constraint_mode_disabled
            .get(&sub)
            .cloned()
            .unwrap_or_default();
        csp.has_subs = true;
        let mut members: HashSet<String> = HashSet::default();
        let mut items: Vec<ConstraintItem> = Vec::new();
        let mut seen_con: HashSet<String> = HashSet::default();
        let mut seen_var: HashSet<String> = HashSet::default();
        // (name, width, signed, domain; None for a wide member)
        let mut decls: Vec<(String, u32, bool, Option<Dom>)> = Vec::new();
        let mut cur = Some(class_name);
        while let Some(cn) = cur {
            let cd = self.module.classes.get(&cn)?.clone();
            if !cd.randc_properties.is_empty() {
                return None;
            }
            members.extend(cd.properties.keys().cloned());
            for name in self.randomize_members(&cd, &rand_disabled) {
                if !seen_var.insert(name.clone()) {
                    continue;
                }
                let sig = cd.properties.get(&name)?;
                // §8.25: a member typed by a type parameter bound to a class
                // is a handle too.
                let is_obj = self.prop_class_type(sub, &name).is_some();
                // §18.5.9: a rand object member joins the solve as a
                // sub-object of its own (`rand_csp_nested_subs`).
                if is_obj
                    && !cd.array_properties.contains_key(&name)
                    && !cd.array_nd_properties.contains_key(&name)
                    && !cd.queue_properties.contains_key(&name)
                    && !cd.assoc_properties.contains_key(&name)
                {
                    continue;
                }
                if is_obj
                    || sig.is_real
                    || sig.width == 0
                    || cd.array_properties.contains_key(&name)
                    || cd.array_nd_properties.contains_key(&name)
                    || cd.queue_properties.contains_key(&name)
                    || cd.assoc_properties.contains_key(&name)
                {
                    return None;
                }
                // §7.2.1: a packed struct is one integral value of its true
                // width; an unpacked one is not modelled.
                let width = match self.class_prop_struct(sub, &name) {
                    Some(su) if su.packed => {
                        let params = self.instance_param_scope(sub);
                        self.packed_agg_layout_with(&su, &params).1
                    }
                    Some(_) => return None,
                    None => sig.width,
                };
                let enum_dom = sig
                    .type_name
                    .as_ref()
                    .and_then(|tn| self.module.enum_members.get(tn))
                    .filter(|m| !m.is_empty())
                    .map(|m| dom_norm(m.iter().map(|e| (e.1 as i128, e.1 as i128)).collect()));
                if width > 64 {
                    if enum_dom.is_some() {
                        return None;
                    }
                    decls.push((name, width, sig.is_signed, None));
                    continue;
                }
                let dom = enum_dom.unwrap_or_else(|| {
                    let (lo, hi) = ws_range(width, sig.is_signed);
                    vec![(lo, hi)]
                });
                decls.push((name, width, sig.is_signed, Some(dom)));
            }
            if !con_disabled.contains("*") {
                for (cname, con) in cd.constraints.iter() {
                    if !seen_con.insert(cname.clone()) || con_disabled.contains(cname) {
                        continue;
                    }
                    items.extend(con.items.iter().cloned());
                }
            }
            cur = cd.extends.clone();
        }
        // §18.5.8.1: a loop over a fixed-shape member of the sub-object has
        // known indices; it is unrolled so every item reads through `prop`.
        let has_foreach = items.iter().any(|it| {
            let mut hit = false;
            Self::csp_item_walk(it, &mut |x| {
                if matches!(x, ConstraintItem::Foreach { .. }) {
                    hit = true;
                }
            });
            hit
        });
        if has_foreach {
            let mut out = Vec::with_capacity(items.len());
            for it in &items {
                self.csp_unroll_item(sub, None, it, &mut out)?;
            }
            items = out;
        }
        // A wide member no constraint reads takes any value of its width.
        let wide: Vec<&str> = decls
            .iter()
            .filter(|d| d.3.is_none())
            .map(|d| d.0.as_str())
            .collect();
        let unread: HashSet<String> = if wide.is_empty() {
            HashSet::default()
        } else {
            let mut all: Vec<ClassConstraint> = parent.to_vec();
            all.push(ClassConstraint {
                is_static: false,
                is_extern: false,
                has_body: true,
                name: crate::ast::Identifier {
                    name: format!("{}.*", prop),
                    span: crate::ast::Span::dummy(),
                },
                items: items.clone(),
                span: crate::ast::Span::dummy(),
            });
            Self::csp_wide_unread(&all, &wide)
        };
        for (name, width, signed, dom) in decls {
            let dom = match dom {
                Some(d) => d,
                None if unread.contains(&name) => continue,
                None => {
                    csp.wide.insert(csp.vars.len());
                    vec![(0, 0)]
                }
            };
            csp.scalars
                .insert(format!("{}.{}", prop, name), csp.vars.len());
            csp.vars.push(CspVar {
                key: VarKey::Sub(sub, name),
                width,
                signed,
            });
            csp.dom0.push(dom);
        }
        if csp.vars.len() > MAX_VARS {
            return None;
        }
        let mut out = Vec::with_capacity(items.len());
        for mut it in items {
            if !Self::csp_prefix_item(&mut it, prop, &members) {
                return None;
            }
            out.push(it);
        }
        Some(out)
    }

    /// Visit `it` and every item nested in it.
    pub(super) fn csp_item_walk(it: &ConstraintItem, f: &mut dyn FnMut(&ConstraintItem)) {
        f(it);
        match it {
            ConstraintItem::Implication { constraint, .. } => Self::csp_item_walk(constraint, f),
            ConstraintItem::IfElse {
                then_item,
                else_item,
                ..
            } => {
                Self::csp_item_walk(then_item, f);
                if let Some(e) = else_item {
                    Self::csp_item_walk(e, f);
                }
            }
            ConstraintItem::Foreach { item, .. } | ConstraintItem::Soft(item) => {
                Self::csp_item_walk(item, f)
            }
            ConstraintItem::Block(items) => items.iter().for_each(|i| Self::csp_item_walk(i, f)),
            _ => {}
        }
    }

    /// Rewrite a sub-object's constraint item so its member references go
    /// through the enclosing object's member `prop` (`x` becomes `prop.x`).
    /// False when the item holds something that cannot be rewritten so.
    fn csp_prefix_item(it: &mut ConstraintItem, prop: &str, members: &HashSet<String>) -> bool {
        use crate::ast::decl::ConstraintRange;
        match it {
            ConstraintItem::Expr(e) => Self::csp_prefix_expr(e, prop, members),
            ConstraintItem::Inside { expr, range, .. } => {
                Self::csp_prefix_expr(expr, prop, members)
                    && range.iter_mut().all(|r| match r {
                        ConstraintRange::Value(v) => Self::csp_prefix_expr(v, prop, members),
                        ConstraintRange::Range { lo, hi } => {
                            Self::csp_prefix_expr(lo, prop, members)
                                && Self::csp_prefix_expr(hi, prop, members)
                        }
                    })
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                Self::csp_prefix_expr(condition, prop, members)
                    && Self::csp_prefix_item(constraint, prop, members)
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                Self::csp_prefix_expr(condition, prop, members)
                    && Self::csp_prefix_item(then_item, prop, members)
                    && else_item
                        .as_mut()
                        .is_none_or(|e| Self::csp_prefix_item(e, prop, members))
            }
            ConstraintItem::Soft(inner) => Self::csp_prefix_item(inner, prop, members),
            ConstraintItem::Block(items) => items
                .iter_mut()
                .all(|i| Self::csp_prefix_item(i, prop, members)),
            // §18.5.10: an ordering among the sub-object's own members.
            ConstraintItem::Solve {
                before,
                after,
                before_paths,
                after_paths,
                ..
            } => {
                if before_paths.len() != before.len() || after_paths.len() != after.len() {
                    return false;
                }
                before_paths
                    .iter_mut()
                    .chain(after_paths.iter_mut())
                    .all(|e| Self::csp_prefix_expr(e, prop, members))
            }
            _ => false,
        }
    }

    fn csp_prefix_expr(e: &mut Expression, prop: &str, members: &HashSet<String>) -> bool {
        // An element of an object array (`arr[k]`) is named by a path
        // segment of its own (see `obj`).
        let elem = prop.contains('[');
        let seg = |name: &str, span: crate::ast::Span| crate::ast::expr::HierPathSegment {
            name: crate::ast::Identifier {
                name: name.to_string(),
                span,
            },
            selects: Vec::new(),
        };
        let through = |name: &str, span: crate::ast::Span| {
            if elem {
                return Expression::new(
                    ExprKind::Ident(HierarchicalIdentifier {
                        root: None,
                        path: vec![seg(prop, span), seg(name, span)],
                        span,
                        cached_signal_id: Cell::new(None),
                        cached_resolved_name: std::cell::OnceCell::new(),
                    }),
                    span,
                );
            }
            // `a.b` (a sub-object of a sub-object) is a member chain.
            let mut base = Expression::new(
                ExprKind::Ident(HierarchicalIdentifier {
                    root: None,
                    path: vec![seg(prop.split('.').next().unwrap_or(prop), span)],
                    span,
                    cached_signal_id: Cell::new(None),
                    cached_resolved_name: std::cell::OnceCell::new(),
                }),
                span,
            );
            for part in prop.split('.').skip(1) {
                base = Expression::new(
                    ExprKind::MemberAccess {
                        expr: Box::new(base),
                        member: crate::ast::Identifier {
                            name: part.to_string(),
                            span,
                        },
                    },
                    span,
                );
            }
            Expression::new(
                ExprKind::MemberAccess {
                    expr: Box::new(base),
                    member: crate::ast::Identifier {
                        name: name.to_string(),
                        span,
                    },
                },
                span,
            )
        };
        match &mut e.kind {
            ExprKind::Ident(h) => {
                if h.root.is_some() {
                    return false;
                }
                if h.path.len() == 1 && h.path[0].selects.is_empty() {
                    let n = h.path[0].name.name.clone();
                    if members.contains(&n) {
                        *e = through(&n, e.span);
                    }
                    return true;
                }
                // §7.2.1/§11.5.1: a field or select of a member (`m.f`,
                // `m[i]`, `this.m.f[k]`), the selects rewritten too.
                if h.path[0].name.name == "this" && h.path.len() >= 2 {
                    h.path.remove(0);
                }
                for s in &mut h.path {
                    for x in &mut s.selects {
                        if !Self::csp_prefix_expr(x, prop, members) {
                            return false;
                        }
                    }
                }
                if !members.contains(&h.path[0].name.name) {
                    return false;
                }
                let span = h.span;
                if elem {
                    h.path.insert(0, seg(prop, span));
                } else {
                    for (k, part) in prop.split('.').enumerate() {
                        h.path.insert(k, seg(part, span));
                    }
                }
                h.cached_signal_id.set(None);
                h.cached_resolved_name = std::cell::OnceCell::new();
                e.cached_width.set(None);
                true
            }
            ExprKind::MemberAccess { expr, member } => {
                if matches!(expr.kind, ExprKind::This) && members.contains(&member.name) {
                    let n = member.name.clone();
                    *e = through(&n, e.span);
                    return true;
                }
                if matches!(expr.kind, ExprKind::This) {
                    return false;
                }
                Self::csp_prefix_expr(expr, prop, members)
            }
            ExprKind::Number(_) | ExprKind::StringLiteral(_) | ExprKind::TypeLiteral(_) => true,
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                Self::csp_prefix_expr(operand, prop, members)
            }
            ExprKind::Binary { left, right, .. }
            | ExprKind::Range(left, right)
            | ExprKind::Index {
                expr: left,
                index: right,
            } => {
                Self::csp_prefix_expr(left, prop, members)
                    && Self::csp_prefix_expr(right, prop, members)
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                Self::csp_prefix_expr(expr, prop, members)
                    && Self::csp_prefix_expr(left, prop, members)
                    && Self::csp_prefix_expr(right, prop, members)
            }
            ExprKind::SystemCall { args, .. } => args
                .iter_mut()
                .all(|a| Self::csp_prefix_expr(a, prop, members)),
            ExprKind::Replication { count, exprs } => {
                Self::csp_prefix_expr(count, prop, members)
                    && exprs
                        .iter_mut()
                        .all(|p| Self::csp_prefix_expr(p, prop, members))
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::csp_prefix_expr(condition, prop, members)
                    && Self::csp_prefix_expr(then_expr, prop, members)
                    && Self::csp_prefix_expr(else_expr, prop, members)
            }
            ExprKind::Inside { expr, ranges } => {
                Self::csp_prefix_expr(expr, prop, members)
                    && ranges
                        .iter_mut()
                        .all(|r| Self::csp_prefix_expr(r, prop, members))
            }
            ExprKind::Concatenation(parts) => parts
                .iter_mut()
                .all(|p| Self::csp_prefix_expr(p, prop, members)),
            _ => false,
        }
    }

    // ---------------------------------------------------------------------
    // Translation
    // ---------------------------------------------------------------------

    fn csp_src(&self, csp: &mut Csp, item: SrcItem, env: &Env, mut deps: Vec<usize>) -> usize {
        // The evaluator reads a counted variable itself, not its count.
        if !csp.pop_rev.is_empty() {
            let xs: Vec<usize> = deps
                .iter()
                .filter_map(|d| csp.pop_rev.get(d).copied())
                .collect();
            if !xs.is_empty() {
                deps.extend(xs);
                deps.sort_unstable();
                deps.dedup();
            }
        }
        // The evaluator reads a sum's terms, not its carry variables.
        if !csp.carries.is_empty() {
            let ts: Vec<usize> = csp
                .carries
                .iter()
                .filter(|c| deps.contains(&c.0) || deps.contains(&c.1))
                .flat_map(|c| c.2.iter().copied())
                .collect();
            if !ts.is_empty() {
                deps.extend(ts);
                deps.sort_unstable();
                deps.dedup();
            }
        }
        // §18.5.9: the evaluator reads an object-array element through its
        // select (`arr[k]`), not by the solver's one-segment name.
        let item = match item {
            SrcItem::Expr(e) => SrcItem::Expr(Self::csp_obj_real(&e).unwrap_or(e)),
            SrcItem::Item(it) => SrcItem::Item(Self::csp_obj_real_item(&it)),
        };
        let stale = csp.size_phase && Self::csp_src_sized(csp, &item);
        csp.srcs.push(Src {
            item,
            binds: env.binds.clone(),
            deps,
            stale,
        });
        csp.srcs.len() - 1
    }

    /// A node judged by the evaluator alone. None when its variables cannot
    /// be determined.
    fn csp_eval_node(
        &mut self,
        csp: &mut Csp,
        item: SrcItem,
        env: &Env,
        neg: bool,
    ) -> Option<Node> {
        let mut deps = Vec::new();
        let complete = match &item {
            SrcItem::Expr(e) => self.csp_refs(csp, e, env, &mut deps),
            SrcItem::Item(it) => self.csp_item_refs(csp, it, env, &mut deps),
        };
        let mut complete = complete;
        if !csp.wide.is_empty() && self.csp_may_read_wide(csp, &item) {
            // Left to the evaluator, a relation on a whole wide variable
            // would wait for every segment to be guessed: not modelled. One
            // that reads fields or selects of it depends on their segments.
            let SrcItem::Expr(e) = &item else {
                return None;
            };
            deps.clear();
            if !self.csp_wide_deps(csp, e, env, &mut deps) {
                return None;
            }
            complete = true;
        }
        if !complete {
            // An opaque call may read any rand member.
            deps = (0..csp.vars.len()).collect();
        }
        if matches!(
            item,
            SrcItem::Item(ConstraintItem::Foreach { .. } | ConstraintItem::Unique { .. })
        ) {
            csp.opaque = true;
        }
        deps.sort_unstable();
        deps.dedup();
        let src = self.csp_src(csp, item, env, deps);
        Some(Node::Eval { src, neg })
    }

    fn csp_item(&mut self, csp: &mut Csp, item: &ConstraintItem, env: &Env) -> Option<Node> {
        let r = self.csp_item_inner(csp, item, env);
        // The size phase solves a relaxation: what it cannot model (an
        // element of an array not sized yet) constrains nothing there.
        if r.is_none() && csp.size_phase {
            return Some(Node::True);
        }
        r
    }
    fn csp_item_inner(&mut self, csp: &mut Csp, item: &ConstraintItem, env: &Env) -> Option<Node> {
        match item {
            ConstraintItem::Expr(e) => self.csp_bool(csp, e, env, false),
            ConstraintItem::Inside {
                expr,
                range,
                is_dist,
                dist_weights,
                ..
            } => {
                // The weights of a wide operand are modelled only on one
                // segment (a field, an aligned select). Over a whole wide
                // variable the dist still constrains it to its items
                // (§18.5.4: the value is in the set of a nonzero weight);
                // the value is then drawn uniformly over them.
                let unweighted = *is_dist
                    && !csp.wide.is_empty()
                    && self.csp_wide_touch(csp, expr, env)
                    && !matches!(self.csp_ae(csp, expr, env), Some(Ae::Var(..)));
                let set: Vec<ConstraintRange> = if *is_dist {
                    // §18.5.4: a zero weight removes the value.
                    let mut kept = Vec::new();
                    for (k, r) in range.iter().enumerate() {
                        let w = match dist_weights.get(k).and_then(|w| w.as_ref()) {
                            Some(DistWeight::Each(e)) | Some(DistWeight::Total(e)) => {
                                self.csp_const(e, env).and_then(|v| v.to_u64())
                            }
                            None => Some(1),
                        };
                        if w != Some(0) {
                            kept.push(r.clone());
                        }
                    }
                    kept
                } else {
                    range.clone()
                };
                if unweighted
                    && let Some(n) = self.csp_wide_dist(csp, expr, range, dist_weights, env)
                {
                    return Some(n);
                }
                let n =
                    self.csp_inside(csp, expr, &set, env, false, SrcItem::Item(item.clone()))?;
                if *is_dist && !unweighted {
                    self.csp_dist(csp, expr, &range, dist_weights, env);
                }
                Some(n)
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                let c = self.csp_bool(csp, condition, env, false)?;
                let then = self.csp_item(csp, constraint, &env.guarded(&c))?;
                self.csp_if(csp, condition, then, Node::True, env)
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                let c = self.csp_bool(csp, condition, env, false)?;
                let then = self.csp_item(csp, then_item, &env.guarded(&c))?;
                let els = match else_item {
                    Some(e) => {
                        let nc = self.csp_bool(csp, condition, env, true)?;
                        self.csp_item(csp, e, &env.guarded(&nc))?
                    }
                    None => Node::True,
                };
                self.csp_if(csp, condition, then, els, env)
            }
            ConstraintItem::Foreach {
                array,
                vars,
                item: body,
                ..
            } => {
                // §7.4.1/§12.7.3: a loop over a multi-dimensional packed
                // variable (`foreach (a.w[i, j])`) walks the labels of its
                // dimensions, one loop variable per dimension.
                if let Some(p) = self.csp_pdim_ref(csp, array, &mut |_, _| None) {
                    let dims = match p {
                        Some(p) if p.depth == 0 => csp.pdims.get(&p.v)?.clone(),
                        _ => return None,
                    };
                    if vars.len() > dims.len() {
                        return None;
                    }
                    let loops: Vec<(String, i64, i64)> = vars
                        .iter()
                        .zip(&dims)
                        .filter_map(|(v, &(l, r))| {
                            v.as_ref().map(|v| (v.name.clone(), l.min(r), l.max(r)))
                        })
                        .collect();
                    let count = loops
                        .iter()
                        .try_fold(1u64, |n, l| n.checked_mul((l.2 - l.1 + 1) as u64))?;
                    if count > 4096 {
                        return None;
                    }
                    let mut idx: Vec<i64> = loops.iter().map(|l| l.1).collect();
                    let mut out = Vec::new();
                    loop {
                        let mut e2 = env.clone();
                        for (l, &i) in loops.iter().zip(&idx) {
                            e2.binds.push((l.0.clone(), Self::signed_loop_val(i)));
                        }
                        match self.csp_item(csp, body, &e2)? {
                            Node::True => {}
                            n => out.push(n),
                        }
                        let mut k = loops.len();
                        loop {
                            if k == 0 {
                                return Some(Self::csp_and(out));
                            }
                            k -= 1;
                            idx[k] += 1;
                            if idx[k] <= loops[k].2 {
                                break;
                            }
                            idx[k] = loops[k].1;
                        }
                    }
                }
                // §7.4.5: a loop over a packed-array field (`foreach
                // (a.s.f[i])`) walks the labels of its first dimension.
                if let [Some(iv)] = vars.as_slice()
                    && !csp.field_elems.is_empty()
                    && let Some((_, all)) = self.csp_field_elem_all(csp, array, &mut |_, _| None)
                {
                    let mut out = Vec::with_capacity(all.len());
                    for (i, ..) in all {
                        let mut e2 = env.clone();
                        e2.binds.push((iv.name.clone(), Self::signed_loop_val(i)));
                        match self.csp_item(csp, body, &e2)? {
                            Node::True => {}
                            n => out.push(n),
                        }
                    }
                    return Some(Self::csp_and(out));
                }
                // §18.4 / §12.7.3 (#519): a loop over a multi-dimensional
                // solver array (`foreach (a[i])`, `foreach (a[i][j])`) or over
                // one of its rows (`foreach (a[i][j])` with `i` bound): each
                // loop variable walks the declared bounds of its dimension.
                if !csp.nd_dims.is_empty() {
                    let mut konst = |me: &mut Self, x: &Expression| -> Option<i64> {
                        if !me.csp_free(csp, x, env) {
                            return None;
                        }
                        me.csp_const(x, env)?.to_i64()
                    };
                    if let Some((base, consts)) = self.csp_nd_prefix_k(csp, array, &mut konst)
                        && let Some(dims) = csp.nd_dims.get(&base).cloned()
                        && consts.len() + vars.len() <= dims.len()
                    {
                        // A dynamic dimension is `(0, size - 1)`: one that is
                        // empty gives the loop no iteration (§12.7.3), where
                        // sorting its bounds walked -1 and 0.
                        let ndyn = csp.nd_dyn.get(&base).copied().unwrap_or(0);
                        let loops: Vec<(Option<String>, i64, i64)> = vars
                            .iter()
                            .enumerate()
                            .map(|(j, v)| {
                                let d = consts.len() + j;
                                let (l, r) = dims[d];
                                let (lo, hi) = if d < ndyn { (0, r) } else { (l.min(r), l.max(r)) };
                                (v.as_ref().map(|v| v.name.clone()), lo, hi)
                            })
                            .collect();
                        let count = loops
                            .iter()
                            .try_fold(1u64, |n, l| n.checked_mul((l.2 - l.1 + 1).max(0) as u64))?;
                        if count == 0 {
                            return Some(Node::True);
                        }
                        if count > 4096 {
                            return None;
                        }
                        let mut idx: Vec<i64> = loops.iter().map(|l| l.1).collect();
                        let mut out = Vec::new();
                        loop {
                            let mut e2 = env.clone();
                            for (l, &i) in loops.iter().zip(&idx) {
                                if let Some(n) = &l.0 {
                                    e2.binds.push((n.clone(), Self::signed_loop_val(i)));
                                }
                            }
                            match self.csp_item(csp, body, &e2)? {
                                Node::True => {}
                                n => out.push(n),
                            }
                            let mut k = loops.len();
                            loop {
                                if k == 0 {
                                    return Some(Self::csp_and(out));
                                }
                                k -= 1;
                                idx[k] += 1;
                                if idx[k] <= loops[k].2 {
                                    break;
                                }
                                idx[k] = loops[k].1;
                            }
                        }
                    }
                }
                let name = Self::foreach_base_name(array);
                let arr = name.as_ref().and_then(|n| csp.arrays.get(n)).cloned();
                let idx_var = match vars.as_slice() {
                    [Some(v)] => Some(v.name.clone()),
                    _ => None,
                };
                let (Some(arr), Some(iv)) = (arr, idx_var.clone()) else {
                    // §18.5.8.1: a foreach over a fixed-shape member that is
                    // not a solver array (a state vector used as an
                    // iterator, `bit [0:3] it;`) or over the bits of a
                    // solver scalar: its indices are known, so the body is
                    // translated once per index like a solver array's.
                    if let (Some(iv), Some(idx)) = (
                        idx_var,
                        self.csp_foreach_indices(csp, array, name.as_deref()),
                    ) {
                        let mut out = Vec::with_capacity(idx.len());
                        for i in idx {
                            let mut e2 = env.clone();
                            e2.binds.push((iv.clone(), Self::signed_loop_val(i)));
                            match self.csp_item(csp, body, &e2)? {
                                Node::True => {}
                                n => out.push(n),
                            }
                        }
                        return Some(Self::csp_and(out));
                    }
                    return self.csp_eval_node(csp, SrcItem::Item(item.clone()), env, false);
                };
                let mut out = Vec::with_capacity(arr.elems.len());
                for (i, _) in &arr.elems {
                    let mut e2 = env.clone();
                    // Ordinary array indices are signed ints, including negative bounds.
                    e2.binds.push((iv.clone(), Self::signed_loop_val(*i)));
                    let n = self.csp_item(csp, body, &e2)?;
                    match n {
                        Node::True => {}
                        n => out.push(n),
                    }
                }
                Some(Self::csp_and(out))
            }
            ConstraintItem::Solve {
                before,
                after,
                before_paths,
                after_paths,
                ..
            } => {
                // §18.5.10: an operand is a variable, an array, or a member,
                // field or element path (`a.b.f`, `arr[i].f`, `s.f[3]`); a
                // path the solve does not hold falls back to its root name.
                let vars = |me: &mut Self,
                            names: &[crate::ast::Identifier],
                            paths: &[Expression]|
                 -> Vec<usize> {
                    let mut out = Vec::new();
                    for (k, id) in names.iter().enumerate() {
                        if let Some(p) = paths.get(k)
                            && let Some(vs) = me.csp_solve_path(csp, p, env)
                        {
                            out.extend(vs);
                            continue;
                        }
                        if let Some(&v) = csp.scalars.get(&id.name) {
                            out.push(v);
                        } else if let Some(a) = csp.arrays.get(&id.name) {
                            out.extend(a.elems.iter().map(|e| e.1));
                        }
                    }
                    // A wide variable is decided through its segments.
                    let mut i = 0;
                    while i < out.len() {
                        match csp.segs.get(&out[i]) {
                            Some(segs) if csp.wide.contains(&out[i]) => {
                                let segs: Vec<usize> = segs.iter().map(|x| x.2).collect();
                                out.splice(i..=i, segs.iter().copied());
                                i += segs.len();
                            }
                            _ => i += 1,
                        }
                    }
                    out.sort_unstable();
                    out.dedup();
                    out
                };
                let (b, a) = (
                    vars(self, before, before_paths),
                    vars(self, after, after_paths),
                );
                if !b.is_empty() && !a.is_empty() {
                    csp.order.push((b, a));
                }
                Some(Node::True)
            }
            ConstraintItem::Soft(inner) => {
                let id = csp.soft_priority.len();
                csp.soft_priority
                    .push((csp.soft_rank.0, csp.soft_rank.1, id));
                let enabled = csp
                    .soft_enabled
                    .as_ref()
                    .is_none_or(|set| set.contains(&id));
                let dist_len = csp.dists.len();
                let order_len = csp.order.len();
                let opaque = csp.opaque;
                let node = self.csp_item(csp, inner, env)?;
                if enabled {
                    Some(node)
                } else {
                    csp.dists.truncate(dist_len);
                    csp.order.truncate(order_len);
                    csp.opaque = opaque;
                    Some(Node::True)
                }
            }
            ConstraintItem::Block(items) => {
                let mut out = Vec::with_capacity(items.len());
                for it in items {
                    match self.csp_item(csp, it, env)? {
                        Node::True => {}
                        n => out.push(n),
                    }
                }
                Some(Self::csp_and(out))
            }
            ConstraintItem::Unique { exprs, .. } => {
                let mut vs = Vec::new();
                for e in exprs {
                    let e = Self::unparen(e);
                    if let Some(a) = self.csp_member(csp, e).and_then(|n| csp.arrays.get(&n)) {
                        vs.extend(a.elems.iter().map(|x| x.1));
                        continue;
                    }
                    match self.csp_ae(csp, e, env) {
                        Some(Ae::Var(v, ..)) => vs.push(v),
                        _ => {
                            return self.csp_eval_node(
                                csp,
                                SrcItem::Item(item.clone()),
                                env,
                                false,
                            );
                        }
                    }
                }
                if vs.iter().any(|v| csp.wide.contains(v)) {
                    return None;
                }
                Some(Node::AllDiff(vs))
            }
        }
    }

    /// §18.5.10: the solver variables a `solve` operand path names: a
    /// variable or array, a member of a rand sub-object, or a field,
    /// element or constant select of a variable (the segments it spans).
    /// None when the path names nothing of the solve.
    fn csp_solve_path(&mut self, csp: &Csp, e: &Expression, env: &Env) -> Option<Vec<usize>> {
        if let Some(n) = self.csp_member(csp, e) {
            if let Some(&v) = csp.scalars.get(&n) {
                return Some(vec![v]);
            }
            if let Some(a) = csp.arrays.get(&n) {
                return Some(a.elems.iter().map(|x| x.1).collect());
            }
        }
        let mut konst = |me: &mut Self, x: &Expression| -> Option<i64> {
            if !me.csp_free(csp, x, env) {
                return None;
            }
            me.csp_const(x, env)?.to_i64()
        };
        let (v, lo, w) = match self.csp_select_ref(csp, e, &mut konst) {
            Some((v, Some((lo, w)))) => (v, lo, w),
            Some((v, None)) => (v, 0, csp.vars[v].width),
            None => {
                let (v, lo, w, _) = self.csp_slice_base_k(csp, e, &mut konst)?;
                (v, lo, w)
            }
        };
        let Some(segs) = csp.segs.get(&v) else {
            return Some(vec![v]);
        };
        if let Some(&x) = csp.slice_vars.get(&(v, lo, w)) {
            return Some(vec![x]);
        }
        let vs: Vec<usize> = segs
            .iter()
            .filter(|s| s.0 < lo + w && s.0 + s.1 > lo)
            .map(|s| s.2)
            .collect();
        Some(if vs.is_empty() { vec![v] } else { vs })
    }

    /// The first-dimension indices of `foreach (name[i])` over a plain
    /// member of a fixed shape (an unpacked fixed array or a packed vector)
    /// of the object being solved. None for anything else: a collection of
    /// handles, a dynamic shape, a path through another object.
    fn csp_foreach_indices(
        &self,
        csp: &Csp,
        array: &Expression,
        name: Option<&str>,
    ) -> Option<Vec<i64>> {
        let name = name?;
        if csp.scope.is_some() || csp.arrays.contains_key(name) {
            return None;
        }
        self.csp_foreach_indices_of(csp.handle, array, name)
    }

    /// The same for a member `name` of the object `handle`.
    fn csp_foreach_indices_of(
        &self,
        handle: usize,
        array: &Expression,
        name: &str,
    ) -> Option<Vec<i64>> {
        let plain = match &array.kind {
            ExprKind::Ident(h) => {
                h.root.is_none()
                    && h.path.iter().all(|s| s.selects.is_empty())
                    && (h.path.len() == 1 || (h.path.len() == 2 && h.path[0].name.name == "this"))
            }
            ExprKind::MemberAccess { expr, .. } => matches!(expr.kind, ExprKind::This),
            _ => false,
        };
        if !plain || self.prop_class_type(handle, name).is_some() {
            return None;
        }
        let (lo, hi) = match self
            .fixed_foreach_dims(handle, name)
            .or_else(|| self.packed_prop_foreach_dims(handle, name))
        {
            Some((dims, _)) => *dims.first()?,
            // A state queue or dynamic array: its current size bounds the
            // loop for this solve (an empty one constrains nothing).
            None => match self.csp_vector_range(handle, name) {
                Some(r) => r,
                None => *self.state_foreach_dims(handle, array, name, 1)?.first()?,
            },
        };
        (hi - lo < 4096).then(|| (lo..=hi).collect())
    }

    /// The declared index range (low, high) of a one-dimensional packed
    /// vector member (`bit [0:3] it;`).
    fn csp_vector_range(&self, handle: usize, name: &str) -> Option<(i64, i64)> {
        let mut cur = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone());
        let mut dt = None;
        while let Some(cn) = cur {
            let cd = self.module.classes.get(&cn)?;
            if let Some(t) = self.class_prop_decl_type(cd, name) {
                dt = Some(t.clone());
                break;
            }
            cur = cd.extends.clone();
        }
        let dt =
            crate::compiler::elaborate::resolve_typedef_chain(&dt?, &self.module.typedef_types)
                .clone();
        let DataType::IntegerVector { dimensions, .. } = &dt else {
            return None;
        };
        let [crate::ast::types::PackedDimension::Range { left, right, .. }] = dimensions.as_slice()
        else {
            return None;
        };
        let p = Some(&self.module.parameters);
        let l = crate::compiler::elaborate::const_eval_i64_with_params(left, p)?;
        let r = crate::compiler::elaborate::const_eval_i64_with_params(right, p)?;
        Some((l.min(r), l.max(r)))
    }

    /// Record the weights of `expr dist {…}` when `expr` is one solver
    /// variable (§18.5.4): a value or range item weighs `w` per value with
    /// `:=` (the default) and `w` in all with `:/`; a zero weight or an empty
    /// (reversed) range contributes nothing, and `$` is the variable's own
    /// bound.
    fn csp_dist(
        &mut self,
        csp: &mut Csp,
        expr: &Expression,
        ranges: &[ConstraintRange],
        weights: &[Option<DistWeight>],
        env: &Env,
    ) {
        let Some(Ae::Var(v, w, s)) = self.csp_ae(csp, expr, env) else {
            return;
        };
        let (vlo, vhi) = ws_range(w, s);
        let frame = self.csp_frame(&env.binds);
        self.push_local_frame(frame);
        let mut items = Vec::with_capacity(ranges.len());
        let mut ok = true;
        for (k, r) in ranges.iter().enumerate() {
            let (wt, per_value) = match weights.get(k).and_then(|w| w.as_ref()) {
                Some(DistWeight::Each(e)) => (self.csp_const(e, env), true),
                Some(DistWeight::Total(e)) => (self.csp_const(e, env), false),
                None => (Some(Value::from_u64(1, 32)), true),
            };
            let Some(wt) = wt.and_then(|x| x.to_u64()) else {
                ok = false;
                break;
            };
            let bound = |me: &mut Self, e: &Expression, dollar: i128| -> Option<i128> {
                if matches!(e.kind, ExprKind::Dollar) {
                    return Some(dollar);
                }
                if !me.csp_free(csp, e, env) {
                    return None;
                }
                me.exact_int(e).map(|x| x.0)
            };
            let (lo, hi) = match r {
                ConstraintRange::Value(e) => match bound(self, e, vhi) {
                    Some(x) => (x, x),
                    None => {
                        ok = false;
                        break;
                    }
                },
                ConstraintRange::Range { lo, hi } => {
                    match (bound(self, lo, vlo), bound(self, hi, vhi)) {
                        (Some(l), Some(h)) => (l, h),
                        _ => {
                            ok = false;
                            break;
                        }
                    }
                }
            };
            let (lo, hi) = (lo.max(vlo), hi.min(vhi));
            if wt == 0 || lo > hi {
                continue;
            }
            let mass = if per_value {
                wt as f64 * (hi - lo + 1) as f64
            } else {
                wt as f64
            };
            items.push((lo, hi, mass));
        }
        self.pop_local_frame();
        if ok && !items.is_empty() {
            csp.dists.push(Dist {
                var: v,
                items,
                guards: env.guards.clone(),
            });
        }
    }

    /// §18.5.4: `x dist {…}` over a wide `x`. Its items are not values of
    /// one solver variable, so an auxiliary variable picks the item: it
    /// carries the dist (decided first, in proportion to each item's weight
    /// among the items still feasible), and `x` is constrained to the item
    /// it picks. None when an item or weight is not constant.
    fn csp_wide_dist(
        &mut self,
        csp: &mut Csp,
        expr: &Expression,
        ranges: &[ConstraintRange],
        weights: &[Option<DistWeight>],
        env: &Env,
    ) -> Option<Node> {
        // The number of values of `[lo:hi]`, as a float (wide bounds).
        fn span_count(lo: &Value, hi: &Value) -> f64 {
            let f = |v: &Value| {
                let mut x = 0.0f64;
                for i in (0..v.width as usize).rev() {
                    x = x * 2.0
                        + if v.get_bit(i) == LogicBit::One {
                            1.0
                        } else {
                            0.0
                        };
                }
                x
            };
            (f(hi) - f(lo) + 1.0).max(1.0)
        }
        let mut items: Vec<(ConstraintRange, f64)> = Vec::new();
        for (k, r) in ranges.iter().enumerate() {
            let (wt, per_value) = match weights.get(k).and_then(|w| w.as_ref()) {
                Some(DistWeight::Each(e)) => (self.csp_const(e, env)?.to_u64()?, true),
                Some(DistWeight::Total(e)) => (self.csp_const(e, env)?.to_u64()?, false),
                None => (1, true),
            };
            if wt == 0 {
                continue;
            }
            let mass = match r {
                ConstraintRange::Value(_) => wt as f64,
                ConstraintRange::Range { lo, hi } => {
                    if matches!(lo.kind, ExprKind::Dollar) || matches!(hi.kind, ExprKind::Dollar) {
                        return None;
                    }
                    if per_value {
                        let (l, h) = (self.csp_const_any(lo, env), self.csp_const_any(hi, env));
                        if l.has_xz() || h.has_xz() {
                            return None;
                        }
                        wt as f64 * span_count(&l, &h)
                    } else {
                        wt as f64
                    }
                }
            };
            items.push((r.clone(), mass));
        }
        if items.is_empty() || items.len() > 4096 {
            return None;
        }
        let mut alts = Vec::with_capacity(items.len());
        let c = csp.vars.len();
        csp.vars.push(CspVar {
            key: VarKey::Aux,
            width: 32,
            signed: false,
        });
        csp.dom0.push(vec![(0, items.len() as i128 - 1)]);
        let one = Self::literal_of(&Value::from_u64(1, 1), expr.span);
        for (i, (r, _)) in items.iter().enumerate() {
            let node = self.csp_wide_inside(csp, expr, std::slice::from_ref(r), env, false)?;
            let src = self.csp_src(csp, SrcItem::Expr(one.clone()), &Env::default(), vec![c]);
            let pick = Node::In {
                lin: Lin::var(c),
                set: vec![(i as i128, i as i128)],
                neg: false,
                fits: Vec::new(),
                src,
                wrap: None,
            };
            alts.push(Self::csp_wide_and(vec![pick, node]));
        }
        csp.dists.push(Dist {
            var: c,
            items: items
                .iter()
                .enumerate()
                .map(|(i, (_, m))| (i as i128, i as i128, *m))
                .collect(),
            guards: env.guards.clone(),
        });
        Some(Self::csp_wide_or(alts))
    }

    fn csp_and(mut v: Vec<Node>) -> Node {
        match v.len() {
            0 => Node::True,
            1 => v.pop().unwrap(),
            _ => Node::And(v),
        }
    }

    fn csp_if(
        &mut self,
        csp: &mut Csp,
        cond: &Expression,
        then: Node,
        els: Node,
        env: &Env,
    ) -> Option<Node> {
        let c = self.csp_bool(csp, cond, env, false)?;
        match c {
            Node::True => return Some(then),
            Node::False => return Some(els),
            _ => {}
        }
        if matches!(then, Node::True) && matches!(els, Node::True) {
            return Some(Node::True);
        }
        let nc = self.csp_bool(csp, cond, env, true)?;
        Some(Node::If {
            cond: Box::new(c),
            ncond: Box::new(nc),
            then: Box::new(then),
            els: Box::new(els),
        })
    }

    /// A boolean constraint expression (negated with `neg`).
    fn csp_bool(&mut self, csp: &mut Csp, e: &Expression, env: &Env, neg: bool) -> Option<Node> {
        let e = Self::unparen(e);
        if self.csp_free(csp, e, env) {
            let v = self.csp_const_any(e, env);
            return Some(if v.is_true() != neg {
                Node::True
            } else {
                Node::False
            });
        }
        match &e.kind {
            ExprKind::Unary {
                op: UnaryOp::LogNot,
                operand,
            } => self.csp_bool(csp, operand, env, !neg),
            ExprKind::Binary { op, left, right } => match op {
                BinaryOp::LogAnd | BinaryOp::LogOr => {
                    let l = self.csp_bool(csp, left, env, neg)?;
                    let r = self.csp_bool(csp, right, env, neg)?;
                    // De Morgan: a negated && is an || of the negations
                    Some(if (*op == BinaryOp::LogAnd) != neg {
                        Node::And(vec![l, r])
                    } else {
                        Node::Or(vec![l, r])
                    })
                }
                BinaryOp::LogImplies => {
                    if neg {
                        let l = self.csp_bool(csp, left, env, false)?;
                        let r = self.csp_bool(csp, right, env, true)?;
                        Some(Node::And(vec![l, r]))
                    } else {
                        let then = self.csp_bool(csp, right, env, false)?;
                        self.csp_if(csp, left, then, Node::True, env)
                    }
                }
                BinaryOp::Eq
                | BinaryOp::Neq
                | BinaryOp::CaseEq
                | BinaryOp::CaseNeq
                | BinaryOp::Lt
                | BinaryOp::Leq
                | BinaryOp::Gt
                | BinaryOp::Geq => {
                    if let Some(n) = self.csp_dyn_expand(csp, e, env, neg) {
                        return Some(n);
                    }
                    self.csp_rel(csp, e, *op, left, right, env, neg)
                }
                _ => self
                    .csp_dyn_expand(csp, e, env, neg)
                    .or_else(|| self.csp_eval_node(csp, SrcItem::Expr(e.clone()), env, neg)),
            },
            ExprKind::Inside { expr, ranges } => {
                if let Some(n) = self.csp_dyn_expand(csp, e, env, neg) {
                    return Some(n);
                }
                let cr: Vec<ConstraintRange> = ranges
                    .iter()
                    .map(|r| match &r.kind {
                        ExprKind::Range(lo, hi) => ConstraintRange::Range {
                            lo: (**lo).clone(),
                            hi: (**hi).clone(),
                        },
                        _ => ConstraintRange::Value(r.clone()),
                    })
                    .collect();
                self.csp_inside(csp, expr, &cr, env, neg, SrcItem::Expr(e.clone()))
            }
            _ => self
                .csp_dyn_expand(csp, e, env, neg)
                .or_else(|| self.csp_eval_node(csp, SrcItem::Expr(e.clone()), env, neg)),
        }
    }

    /// §7.4/§11.5.1: a relation reading an element of a solver array, or a
    /// bit of a solver variable, at a position that itself depends on solver
    /// variables (`rights[i*16 + id[i]] == 4'b1011`, `mask[id] == 0`). Per
    /// position `k` the select names one variable, so the relation holds as
    /// `idx == k -> rel[k]` for every `k`: each implication propagates (a
    /// decided index pins its element; a refuted element removes the
    /// position from the index). The original relation is also judged as a
    /// whole, which keeps an out-of-range index exact. None when no such
    /// select is found or the expansion would be too large.
    fn csp_dyn_expand(
        &mut self,
        csp: &mut Csp,
        e: &Expression,
        env: &Env,
        neg: bool,
    ) -> Option<Node> {
        let depth = CSP_DYN_DEPTH.with(|c| c.get());
        if depth >= 2 || (csp.arrays.is_empty() && csp.segs.is_empty()) {
            return None;
        }
        let ph = format!("__xz_dyn_ix{depth}");
        let mut body = e.clone();
        let (idx, dom) = self.csp_take_dyn_select(csp, &mut body, env, &ph)?;
        if dom.is_empty() || dom.len() > 256 {
            return None;
        }
        CSP_DYN_DEPTH.with(|c| c.set(depth + 1));
        let out = self.csp_dyn_cases(csp, e, &body, idx, &dom, &ph, env, neg);
        CSP_DYN_DEPTH.with(|c| c.set(depth));
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn csp_dyn_cases(
        &mut self,
        csp: &mut Csp,
        whole: &Expression,
        body: &Expression,
        idx: Expression,
        dom: &[i64],
        ph: &str,
        env: &Env,
        neg: bool,
    ) -> Option<Node> {
        let cond = Expression::new(
            ExprKind::Binary {
                op: BinaryOp::Eq,
                left: Box::new(idx),
                right: Box::new(Self::csp_ph_ident(ph, whole.span)),
            },
            whole.span,
        );
        // The declared domains, to drop positions the index never reaches.
        let st0 = St {
            d: csp.dom0.clone(),
            trail: Vec::new(),
            dirty: Vec::new(),
            work: 0,
            est_work: 0,
            cut: false,
            level_tries: 0,
            level_cut: false,
        };
        let mut out = Vec::with_capacity(dom.len() + 1);
        for &k in dom {
            let mut ek = env.clone();
            ek.binds.push((ph.to_string(), Self::signed_loop_val(k)));
            let c = self.csp_bool(csp, &cond, &ek, false)?;
            let never = match &c {
                Node::False => true,
                Node::Lin {
                    lin,
                    rel: Rel::Eq,
                    fits,
                    ..
                } if st0.fits_hold(fits) => {
                    let (lo, hi) = st0.lin_bounds(lin);
                    lo > 0 || hi < 0
                }
                _ => false,
            };
            if never {
                continue;
            }
            let then = self.csp_bool(csp, body, &ek.guarded(&c), neg)?;
            if matches!(c, Node::True) {
                out.push(then);
                continue;
            }
            if matches!(then, Node::True) {
                continue;
            }
            let nc = self.csp_bool(csp, &cond, &ek, true)?;
            out.push(Node::If {
                cond: Box::new(c),
                ncond: Box::new(nc),
                then: Box::new(then),
                els: Box::new(Node::True),
            });
        }
        // The relation as a whole, for an index outside every case. The
        // cases already decide every position, so a select of a wide
        // variable here may wait for all of its segments.
        let prev = CSP_DYN_WHOLE.with(|c| c.replace(true));
        let guard = self.csp_eval_node(csp, SrcItem::Expr(whole.clone()), env, neg);
        CSP_DYN_WHOLE.with(|c| c.set(prev));
        out.push(guard?);
        Some(Self::csp_and(out))
    }

    fn csp_ph_ident(ph: &str, span: crate::ast::Span) -> Expression {
        Expression::new(
            ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                root: None,
                path: vec![crate::ast::expr::HierPathSegment {
                    name: crate::ast::Identifier {
                        name: ph.to_string(),
                        span,
                    },
                    selects: Vec::new(),
                }],
                span,
                cached_signal_id: std::cell::Cell::new(None),
                cached_resolved_name: std::cell::OnceCell::new(),
            }),
            span,
        )
    }

    /// The first select in `e` (outermost first) whose index reads solver
    /// variables and names an element of a solver array or a bit of a
    /// solver variable; its index is replaced in place by the identifier
    /// `ph`. Returns that index and the positions the select can name.
    fn csp_take_dyn_select(
        &mut self,
        csp: &Csp,
        e: &mut Expression,
        env: &Env,
        ph: &str,
    ) -> Option<(Expression, Vec<i64>)> {
        let hit = match &e.kind {
            ExprKind::Index { expr, index } if !self.csp_free(csp, index, env) => {
                self.csp_dyn_domain(csp, expr, env)
            }
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.last().is_some_and(|s| s.selects.len() == 1)
                    && h.path[..h.path.len() - 1]
                        .iter()
                        .all(|s| s.selects.is_empty())
                    && !self.csp_free(csp, &h.path.last().unwrap().selects[0], env) =>
            {
                let mut bh = h.clone();
                bh.path.last_mut().unwrap().selects.clear();
                let base = Expression::new(ExprKind::Ident(bh), e.span);
                let arr = self
                    .csp_member(csp, &base)
                    .and_then(|n| csp.arrays.get(&n))
                    .map(|a| a.elems.iter().map(|x| x.0).collect());
                // §7.4.5: or the elements of a packed-array field.
                if arr.is_none() && !csp.field_elems.is_empty() {
                    let mut konst = |me: &mut Self, x: &Expression| -> Option<i64> {
                        if !me.csp_free(csp, x, env) {
                            return None;
                        }
                        me.csp_const(x, env)?.to_i64()
                    };
                    self.csp_field_elem_all(csp, &base, &mut konst)
                        .map(|(_, all)| all.iter().map(|x| x.0).collect())
                } else {
                    arr
                }
            }
            _ => None,
        };
        if let Some(dom) = hit {
            let ph = Self::csp_ph_ident(ph, e.span);
            let idx = match &mut e.kind {
                ExprKind::Index { index, .. } => std::mem::replace(&mut **index, ph),
                ExprKind::Ident(h) => {
                    h.cached_signal_id.set(None);
                    h.cached_resolved_name = std::cell::OnceCell::new();
                    std::mem::replace(&mut h.path.last_mut().unwrap().selects[0], ph)
                }
                _ => return None,
            };
            e.cached_width.set(None);
            return Some((idx, dom));
        }
        let found = match &mut e.kind {
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                self.csp_take_dyn_select(csp, operand, env, ph)
            }
            ExprKind::Binary { left, right, .. } | ExprKind::Range(left, right) => self
                .csp_take_dyn_select(csp, left, env, ph)
                .or_else(|| self.csp_take_dyn_select(csp, right, env, ph)),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => self
                .csp_take_dyn_select(csp, condition, env, ph)
                .or_else(|| self.csp_take_dyn_select(csp, then_expr, env, ph))
                .or_else(|| self.csp_take_dyn_select(csp, else_expr, env, ph)),
            ExprKind::Concatenation(xs) | ExprKind::SystemCall { args: xs, .. } => {
                let mut r = None;
                for x in xs {
                    r = self.csp_take_dyn_select(csp, x, env, ph);
                    if r.is_some() {
                        break;
                    }
                }
                r
            }
            ExprKind::Inside { expr, ranges } => {
                let mut r = self.csp_take_dyn_select(csp, expr, env, ph);
                for x in ranges {
                    if r.is_some() {
                        break;
                    }
                    r = self.csp_take_dyn_select(csp, x, env, ph);
                }
                r
            }
            ExprKind::Index { expr, index } => self
                .csp_take_dyn_select(csp, expr, env, ph)
                .or_else(|| self.csp_take_dyn_select(csp, index, env, ph)),
            ExprKind::RangeSelect { expr, .. } | ExprKind::MemberAccess { expr, .. } => {
                self.csp_take_dyn_select(csp, expr, env, ph)
            }
            _ => None,
        };
        if found.is_some() {
            e.cached_width.set(None);
        }
        found
    }

    /// The positions a select of `base` can name: the indices of a solver
    /// array, or the declared bit indices of a solver variable or field.
    fn csp_dyn_domain(&mut self, csp: &Csp, base: &Expression, env: &Env) -> Option<Vec<i64>> {
        if let Some(a) = self.csp_member(csp, base).and_then(|n| csp.arrays.get(&n)) {
            return Some(a.elems.iter().map(|x| x.0).collect());
        }
        let mut konst = |me: &mut Self, x: &Expression| -> Option<i64> {
            if !me.csp_free(csp, x, env) {
                return None;
            }
            me.csp_const(x, env)?.to_i64()
        };
        // §7.4.1: the labels of the dimension a select of a
        // multi-dimensional packed variable indexes.
        if let Some(p) = self.csp_pdim_ref(csp, base, &mut konst) {
            let p = p?;
            let &(l, r) = csp.pdims.get(&p.v)?.get(p.depth)?;
            return Some((l.min(r)..=l.max(r)).collect());
        }
        if let Some((_, all)) = self.csp_field_elem_all(csp, base, &mut konst) {
            return Some(all.iter().map(|x| x.0).collect());
        }
        let (v, _, w, whole) = self.csp_slice_base_k(csp, base, &mut konst)?;
        let w = i64::from(w);
        if !whole {
            return Some((0..w).collect());
        }
        let l = csp.lsb.get(&v).copied().unwrap_or(0);
        Some(if csp.ascending.contains(&v) {
            (l - w + 1..=l).collect()
        } else {
            (l..l + w).collect()
        })
    }

    /// `(a >> k) REL c` -> `a inside [lo:hi]`, for a constant shift amount and
    /// a constant right-hand side. Returns None when the shape does not apply,
    /// leaving the caller's normal affine path untouched.
    ///
    /// Restricted to UNSIGNED `a` and the logical `>>`. For a signed value
    /// `>>` still zero-fills (only `>>>` sign-fills), so the
    /// `a >> k == floor(a / 2**k)` identity this relies on does not hold once
    /// `a` can be negative.
    #[allow(clippy::too_many_arguments)]
    fn csp_shr_rel(
        &mut self,
        csp: &mut Csp,
        whole: &Expression,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
        env: &Env,
        neg: bool,
    ) -> Option<Node> {
        // Shift on one side, constant on the other; flip the relation when the
        // shift is on the right so `c REL (a >> k)` reads the same way.
        let (shift, other, op) = match &Self::unparen(left).kind {
            ExprKind::Binary {
                op: BinaryOp::ShiftRight,
                ..
            } => (left, right, op),
            _ => match &Self::unparen(right).kind {
                ExprKind::Binary {
                    op: BinaryOp::ShiftRight,
                    ..
                } => (
                    right,
                    left,
                    match op {
                        BinaryOp::Lt => BinaryOp::Gt,
                        BinaryOp::Leq => BinaryOp::Geq,
                        BinaryOp::Gt => BinaryOp::Lt,
                        BinaryOp::Geq => BinaryOp::Leq,
                        o => o,
                    },
                ),
                _ => return None,
            },
        };
        let ExprKind::Binary {
            left: base,
            right: amt,
            ..
        } = &Self::unparen(shift).kind
        else {
            return None;
        };
        if !self.csp_free(csp, amt, env) || !self.csp_free(csp, other, env) {
            return None;
        }
        let k = self.csp_const(amt, env)?.to_u64()?;
        let c = i128::from(self.csp_const(other, env)?.to_i64()?);
        if k >= 64 || c < 0 {
            return None;
        }
        let a = self.csp_ae(csp, base, env)?;
        let (w, sg) = Self::csp_ws(&a);
        if sg || w == 0 || w > 64 {
            return None;
        }
        let m: i128 = 1i128 << k;
        let (_, amax) = ws_range(w, false);
        // Each relation as one inclusive interval on `a`, before clamping.
        let (mut lo, mut hi) = match op {
            BinaryOp::Eq | BinaryOp::CaseEq | BinaryOp::Neq | BinaryOp::CaseNeq => {
                (c * m, c * m + m - 1)
            }
            BinaryOp::Lt => (0, c * m - 1),
            BinaryOp::Leq => (0, c * m + m - 1),
            BinaryOp::Gt => (c * m + m, amax),
            BinaryOp::Geq => (c * m, amax),
            _ => return None,
        };
        lo = lo.max(0);
        hi = hi.min(amax);
        // `!=` is the complement of the `==` interval, which Node::In's own
        // `neg` expresses; fold it together with the caller's negation.
        let inv = matches!(op, BinaryOp::Neq | BinaryOp::CaseNeq) != neg;
        let mut fits = Vec::new();
        let mut cx = LinCx::new(csp);
        let lin = Self::csp_lin(&a, w, false, &mut fits, &mut cx)?;
        let carries = std::mem::take(&mut cx.carries);
        self.csp_add_carries(csp, carries);
        fits.push(Fit {
            lin: lin.clone(),
            lo: 0,
            hi: amax,
            modw: 0,
        });
        let set: Dom = if lo > hi { Vec::new() } else { vec![(lo, hi)] };
        let mut deps: Vec<usize> = lin.t.iter().map(|t| t.0).collect();
        for f in &fits {
            deps.extend(f.lin.t.iter().map(|t| t.0));
        }
        deps.sort_unstable();
        deps.dedup();
        let src = self.csp_src(csp, SrcItem::Expr(whole.clone()), env, deps);
        Some(Node::In {
            lin,
            set,
            neg: inv,
            fits,
            src,
            wrap: None,
        })
    }

    /// `LHS REL (expr << v)` (either way round) with `v` a solver variable
    /// of a small domain: a shift by a variable amount is not affine, so
    /// the relation would drop to a `Fun` node that only narrows its
    /// variable once every variable the shift reads is fixed — a pinned
    /// left side (`xfer_byte_cnt == (req.len + 1) << req.size` with
    /// `req.len <= 255` and `req.size <= 4`) then defeats the search,
    /// which cannot refute the relation within its budget although
    /// reasoning over the cases proves it instantly. Per value `k` of `v`
    /// the shift is by a constant, so the case is the affine
    /// `LHS REL expr * 2**k` (§11.4.10, `csp_ae`); the guard `v == k`
    /// makes it hold only at that value, and a refuted case refutes its
    /// guard, so propagation alone settles the relation. The cases cover
    /// the whole declared domain of `v`, so the split is exact.
    #[allow(clippy::too_many_arguments)]
    fn csp_shl_split(
        &mut self,
        csp: &mut Csp,
        whole: &Expression,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
        env: &Env,
        neg: bool,
    ) -> Option<Node> {
        // The shift on one side; the relation keeps its direction, since
        // each case rebuilds the same comparison with the amount replaced.
        let (shift, shift_left) = match &Self::unparen(left).kind {
            ExprKind::Binary {
                op: BinaryOp::ShiftLeft | BinaryOp::ArithShiftLeft,
                ..
            } => (left, true),
            _ => match &Self::unparen(right).kind {
                ExprKind::Binary {
                    op: BinaryOp::ShiftLeft | BinaryOp::ArithShiftLeft,
                    ..
                } => (right, false),
                _ => return None,
            },
        };
        let ExprKind::Binary {
            op: shift_op,
            left: base,
            right: amt,
        } = &Self::unparen(shift).kind
        else {
            return None;
        };
        // A variable amount: a constant one is already affine (`csp_ae`).
        if self.csp_free(csp, amt, env) {
            return None;
        }
        // The amount must be one solver variable of a small domain of
        // non-negative values below 63: the cases enumerate it, and the
        // constant arm reads `a << k` as `a * 2**k` only for such a k.
        let Ae::Var(v, ..) = self.csp_ae(csp, amt, env)? else {
            return None;
        };
        let dom = csp.dom0[v].clone();
        if dom_size(&dom) > 64 || dom.iter().any(|&(l, h)| l < 0 || h >= 63) {
            return None;
        }
        let ph = "__xz_shl_ix";
        let amt_ph = Self::csp_ph_ident(ph, whole.span);
        // `v == k`: the guard of the case for `k`.
        let cond = Expression::new(
            ExprKind::Binary {
                op: BinaryOp::Eq,
                left: amt.clone(),
                right: Box::new(amt_ph.clone()),
            },
            whole.span,
        );
        // The relation with the amount replaced by the placeholder, which
        // the case binds to its value.
        let body = |amt_ph: Expression| {
            let shifted = Expression::new(
                ExprKind::Binary {
                    op: *shift_op,
                    left: base.clone(),
                    right: Box::new(amt_ph),
                },
                whole.span,
            );
            let (l, r) = if shift_left {
                (shifted, right.clone())
            } else {
                (left.clone(), shifted)
            };
            Expression::new(
                ExprKind::Binary {
                    op,
                    left: Box::new(l),
                    right: Box::new(r),
                },
                whole.span,
            )
        };
        let mut out = Vec::new();
        for &(l, h) in &dom {
            for k in l..=h {
                let mut ek = env.clone();
                ek.binds
                    .push((ph.to_string(), Self::signed_loop_val(k as i64)));
                let c = self.csp_bool(csp, &cond, &ek, false)?;
                if matches!(c, Node::False) {
                    continue;
                }
                let then = self.csp_bool(csp, &body(amt_ph.clone()), &ek.guarded(&c), neg)?;
                if matches!(then, Node::True) {
                    continue;
                }
                if matches!(c, Node::True) {
                    out.push(then);
                    continue;
                }
                let nc = self.csp_bool(csp, &cond, &ek, true)?;
                out.push(Node::If {
                    cond: Box::new(c),
                    ncond: Box::new(nc),
                    then: Box::new(then),
                    els: Box::new(Node::True),
                });
            }
        }
        Some(Self::csp_and(out))
    }

    /// `left op right` (§11.6.1: both sides extended to the context width
    /// and signedness).
    #[allow(clippy::too_many_arguments)]
    fn csp_rel(
        &mut self,
        csp: &mut Csp,
        whole: &Expression,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
        env: &Env,
        neg: bool,
    ) -> Option<Node> {
        // A relation on a wide variable is modelled over its segments; one
        // that only reads single segments (a field, an aligned select) also
        // fits the forms below, where the segment is an ordinary variable.
        let wide = !csp.wide.is_empty()
            && (self.csp_wide_touch(csp, left, env) || self.csp_wide_touch(csp, right, env));
        if wide && let Some(n) = self.csp_wide_rel(csp, op, left, right, env, neg) {
            return Some(n);
        }
        if wide
            && matches!(op, BinaryOp::Eq | BinaryOp::CaseEq)
            && !neg
            && let Some(n) = self
                .csp_wide_fun(csp, whole, left, right, env)
                .or_else(|| self.csp_wide_fun(csp, whole, right, left, env))
        {
            return Some(n);
        }
        let fallback = |me: &mut Self, csp: &mut Csp| match me
            .csp_rel_fun(csp, whole, op, left, right, env, neg)
        {
            Some(n) => Some(n),
            None => me.csp_eval_node(csp, SrcItem::Expr(whole.clone()), env, neg),
        };
        // `(a >> k) REL c` with a constant k and an unsigned `a` is a RANGE on
        // `a`: the shift is floor division by 2**k, so each relation maps onto
        // one interval. Without this the relation is not affine, csp_ae fails
        // and the whole constraint drops to the trial loop — where
        // `(x >> 8) == 0` on a 32-bit rand has probability 2**-24 and
        // randomize() reports failure (issue #229) even though `x < 256`,
        // its exact equivalent, solves instantly.
        if !wide && let Some(n) = self.csp_shr_rel(csp, whole, op, left, right, env, neg) {
            return Some(n);
        }
        // A shift by a VARIABLE amount on one side: split it over the
        // amount's values (`csp_shl_split`) before the affine attempt
        // below fails on it and the relation drops to a late `Fun` node
        // that cannot refute a pinned web.
        if !wide && let Some(n) = self.csp_shl_split(csp, whole, op, left, right, env, neg) {
            return Some(n);
        }
        if matches!(op, BinaryOp::Eq | BinaryOp::CaseEq) && !neg && !wide {
            if let Some(n) = self
                .csp_masked_eq(csp, whole, left, right, env)
                .or_else(|| self.csp_masked_eq(csp, whole, right, left, env))
            {
                return Some(n);
            }
        }
        let (Some(a), Some(b)) = (self.csp_ae(csp, left, env), self.csp_ae(csp, right, env)) else {
            return fallback(self, csp);
        };
        let ((wl, sl), (wr, sr)) = (Self::csp_ws(&a), Self::csp_ws(&b));
        let w = wl.max(wr).max(1);
        if w > 64 {
            return fallback(self, csp);
        }
        // §5.7.1: an unsized `'0`/`'1` fills the width of its context; as a
        // 1-bit constant, `y == '1` solved as `y == 1`.
        let fill = |e: &Expression, ae: Ae| match &Self::unparen(e).kind {
            ExprKind::Number(crate::ast::expr::NumberLiteral::UnbasedUnsized('1')) => {
                Ae::Const(Value::from_u64(u64::MAX >> (64 - w), w))
            }
            ExprKind::Number(crate::ast::expr::NumberLiteral::UnbasedUnsized('0')) => {
                Ae::Const(Value::zero(w))
            }
            _ => ae,
        };
        let (a, b) = (fill(left, a), fill(right, b));
        let s = sl && sr;
        let mut fits = Vec::new();
        let mut cx = LinCx::new(csp);
        let (Some(la), Some(lb)) = (
            Self::csp_lin(&a, w, s, &mut fits, &mut cx),
            Self::csp_lin(&b, w, s, &mut fits, &mut cx),
        ) else {
            return fallback(self, csp);
        };
        let carries = std::mem::take(&mut cx.carries);
        let (rlo, rhi) = ws_range(w, s);
        let nleaf = fits.len();
        fits.push(Fit {
            lin: la.clone(),
            lo: rlo,
            hi: rhi,
            modw: 0,
        });
        fits.push(Fit {
            lin: lb.clone(),
            lo: rlo,
            hi: rhi,
            modw: 0,
        });
        let Some(d) = la.plus(&lb, -1) else {
            return fallback(self, csp);
        };
        let op = if neg {
            match op {
                BinaryOp::Eq | BinaryOp::CaseEq => BinaryOp::Neq,
                BinaryOp::Neq | BinaryOp::CaseNeq => BinaryOp::Eq,
                BinaryOp::Lt => BinaryOp::Geq,
                BinaryOp::Leq => BinaryOp::Gt,
                BinaryOp::Gt => BinaryOp::Leq,
                _ => BinaryOp::Lt,
            }
        } else {
            op
        };
        let (lin, rel) = match op {
            BinaryOp::Eq | BinaryOp::CaseEq => (Some(d), Rel::Eq),
            BinaryOp::Neq | BinaryOp::CaseNeq => (Some(d), Rel::Ne),
            BinaryOp::Lt => (d.shift(1), Rel::Le),
            BinaryOp::Leq => (Some(d), Rel::Le),
            BinaryOp::Gt => (d.scale(-1).and_then(|x| x.shift(1)), Rel::Le),
            _ => (d.scale(-1), Rel::Le),
        };
        let Some(lin) = lin else {
            return fallback(self, csp);
        };
        self.csp_add_carries(csp, carries);
        let mut deps: Vec<usize> = lin.t.iter().map(|t| t.0).collect();
        for f in &fits {
            deps.extend(f.lin.t.iter().map(|t| t.0));
        }
        deps.sort_unstable();
        deps.dedup();
        let src = self.csp_src(csp, SrcItem::Expr(whole.clone()), env, deps);
        let sg = if matches!(op, BinaryOp::Gt | BinaryOp::Geq) {
            -1
        } else {
            1
        };
        let wrap = Some(Wrap { w, s, n: nleaf, sg });
        Some(Node::Lin {
            lin,
            rel,
            fits,
            src,
            sneg: neg,
            wrap,
        })
    }

    /// `x OP e` (either way round) with `x` a solver variable and `e` an
    /// expression of OTHER variables the linear model cannot express: a
    /// `Fun` node (see there). None when no side qualifies.
    #[allow(clippy::too_many_arguments)]
    fn csp_rel_fun(
        &mut self,
        csp: &mut Csp,
        whole: &Expression,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
        env: &Env,
        neg: bool,
    ) -> Option<Node> {
        if !matches!(
            op,
            BinaryOp::Eq
                | BinaryOp::CaseEq
                | BinaryOp::Neq
                | BinaryOp::CaseNeq
                | BinaryOp::Lt
                | BinaryOp::Leq
                | BinaryOp::Gt
                | BinaryOp::Geq
        ) {
            return None;
        }
        let op = if neg {
            match op {
                BinaryOp::Eq | BinaryOp::CaseEq => BinaryOp::Neq,
                BinaryOp::Neq | BinaryOp::CaseNeq => BinaryOp::Eq,
                BinaryOp::Lt => BinaryOp::Geq,
                BinaryOp::Leq => BinaryOp::Gt,
                BinaryOp::Gt => BinaryOp::Leq,
                _ => BinaryOp::Lt,
            }
        } else {
            op
        };
        let mirror = |o: BinaryOp| match o {
            BinaryOp::Lt => BinaryOp::Gt,
            BinaryOp::Leq => BinaryOp::Geq,
            BinaryOp::Gt => BinaryOp::Lt,
            BinaryOp::Geq => BinaryOp::Leq,
            o => o,
        };
        let plain_var = |me: &mut Self, csp: &Csp, e: &Expression| match me.csp_ae(csp, e, env) {
            Some(Ae::Var(v, ..)) => Some(v),
            _ => None,
        };
        let reads = |me: &mut Self, csp: &Csp, e: &Expression| -> Option<Vec<usize>> {
            let mut d = Vec::new();
            // A field of a wide variable reads only its own segments.
            if !csp.wide.is_empty() && me.csp_wide_deps(csp, e, env, &mut d) {
                return Some(d);
            }
            d.clear();
            me.csp_refs(csp, e, env, &mut d).then_some(d)
        };
        let mut pick = None;
        if let Some(v) = plain_var(self, csp, left) {
            if let Some(d) = reads(self, csp, right).filter(|d| !d.is_empty() && !d.contains(&v)) {
                pick = Some((v, right, op, d));
            }
        }
        if pick.is_none() {
            if let Some(v) = plain_var(self, csp, right) {
                if let Some(d) = reads(self, csp, left).filter(|d| !d.is_empty() && !d.contains(&v))
                {
                    pick = Some((v, left, mirror(op), d));
                }
            }
        }
        let (var, other, op, mut deps) = pick?;
        deps.sort_unstable();
        deps.dedup();
        let other_src = self.csp_src(csp, SrcItem::Expr(other.clone()), env, deps.clone());
        deps.push(var);
        deps.sort_unstable();
        deps.dedup();
        let whole_src = self.csp_src(csp, SrcItem::Expr(whole.clone()), env, deps);
        Some(Node::Fun {
            var,
            op,
            other: other_src,
            whole: whole_src,
            neg,
        })
    }

    /// `expr inside {ranges}` with constant ranges; judged like the checker
    /// judges it (`value_in_ranges` on the operand's own value).
    #[allow(clippy::too_many_arguments)]
    fn csp_inside(
        &mut self,
        csp: &mut Csp,
        expr: &Expression,
        ranges: &[ConstraintRange],
        env: &Env,
        neg: bool,
        whole: SrcItem,
    ) -> Option<Node> {
        if !csp.wide.is_empty()
            && self.csp_wide_touch(csp, expr, env)
            && let Some(n) = self.csp_wide_inside(csp, expr, ranges, env, neg)
        {
            return Some(n);
        }
        let ranges_free = ranges.iter().all(|r| match r {
            ConstraintRange::Value(e) => self.csp_free(csp, e, env),
            ConstraintRange::Range { lo, hi } => {
                self.csp_free(csp, lo, env) && self.csp_free(csp, hi, env)
            }
        });
        if !ranges_free && !neg {
            // §11.4.13: a bound that reads a rand variable (`dst inside
            // {[BASE : TOP - size*4]}`) makes each range the pair of
            // relations `lo <= expr <= hi` (a value: `expr == v`), and the
            // set their disjunction — propagated rather than left to the
            // evaluator, which only judges it once every operand is fixed.
            let rel = |op: BinaryOp, l: &Expression, r: &Expression| {
                Expression::new(
                    ExprKind::Binary {
                        op,
                        left: Box::new(l.clone()),
                        right: Box::new(r.clone()),
                    },
                    expr.span,
                )
            };
            let mut alts = Vec::with_capacity(ranges.len());
            for r in ranges {
                let n = match r {
                    ConstraintRange::Value(v) => {
                        // a rand array operand is set membership, not `==`
                        if self
                            .csp_member(csp, v)
                            .is_some_and(|n| csp.arrays.contains_key(&n))
                        {
                            return self.csp_eval_node(csp, whole, env, neg);
                        }
                        self.csp_bool(csp, &rel(BinaryOp::Eq, expr, v), env, false)?
                    }
                    ConstraintRange::Range { lo, hi } => {
                        if matches!(lo.kind, ExprKind::Dollar)
                            || matches!(hi.kind, ExprKind::Dollar)
                        {
                            return self.csp_eval_node(csp, whole, env, neg);
                        }
                        let a = self.csp_bool(csp, &rel(BinaryOp::Leq, lo, expr), env, false)?;
                        let b = self.csp_bool(csp, &rel(BinaryOp::Leq, expr, hi), env, false)?;
                        Node::And(vec![a, b])
                    }
                };
                alts.push(n);
            }
            return Some(match alts.len() {
                1 => alts.pop().unwrap(),
                _ => Node::Or(alts),
            });
        }
        let a = if ranges_free {
            self.csp_ae(csp, expr, env)
        } else {
            None
        };
        let Some(a) = a else {
            return self.csp_eval_node(csp, whole, env, neg);
        };
        let (w, s) = Self::csp_ws(&a);
        // §11.4.13: the operand and the items are compared like `==`, at the
        // width of all of them and unsigned unless all are signed: `a8 +
        // 8'd1 inside {[256:256]}` adds at 32 bits.
        let frame = self.csp_frame(&env.binds);
        self.push_local_frame(frame);
        let joint = self.inside_items_ws(ranges);
        self.pop_local_frame();
        let Some((iw, is)) = joint else {
            return self.csp_eval_node(csp, whole, env, neg);
        };
        // A signed operand against an unsigned item: the comparison is
        // unsigned, of the operand extended by its own sign (as
        // `value_in_ranges` judges it). Keep the operand signed and map the
        // items' unsigned bit patterns to the signed values they are.
        let mixed = s && !is;
        let (w, s) = (w.max(iw), s && (is || mixed));
        let mut fits = Vec::new();
        let mut cx = LinCx::new(csp);
        let lin = if w <= 64 {
            Self::csp_lin(&a, w, s, &mut fits, &mut cx)
        } else {
            None
        };
        let carries = std::mem::take(&mut cx.carries);
        let Some(lin) = lin else {
            return self.csp_eval_node(csp, whole, env, neg);
        };
        self.csp_add_carries(csp, carries);
        let frame = self.csp_frame(&env.binds);
        self.push_local_frame(frame);
        let mut set = dom_norm(self.cons_ranges_i128_at(ranges, w));
        self.pop_local_frame();
        if mixed {
            let half = 1i128 << (w - 1);
            let mut signed = Vec::with_capacity(set.len() + 1);
            for &(l, h) in set.iter() {
                let (l, h) = (Self::bits_at(l, w) as i128, Self::bits_at(h, w) as i128);
                if l > h {
                    continue;
                }
                if l < half {
                    signed.push((l, h.min(half - 1)));
                }
                if h >= half {
                    signed.push((l.max(half) - (half << 1), h - (half << 1)));
                }
            }
            set = dom_norm(signed);
        }
        // A negative endpoint makes the checker read an unsigned value as
        // signed; leave that reading to the checker itself.
        if !s && set.iter().any(|r| r.0 < 0) {
            return self.csp_eval_node(csp, whole, env, neg);
        }
        let (rlo, rhi) = ws_range(w, s);
        let nleaf = fits.len();
        fits.push(Fit {
            lin: lin.clone(),
            lo: rlo,
            hi: rhi,
            modw: 0,
        });
        let mut deps: Vec<usize> = lin.t.iter().map(|t| t.0).collect();
        for f in &fits {
            deps.extend(f.lin.t.iter().map(|t| t.0));
        }
        deps.sort_unstable();
        deps.dedup();
        let src = self.csp_src(csp, whole, env, deps);
        Some(Node::In {
            lin,
            set,
            neg,
            fits,
            src,
            wrap: Some(Wrap {
                w,
                s,
                n: nleaf,
                sg: 1,
            }),
        })
    }

    /// The joint (width, signed) of an `inside` set's items (§11.4.13): the
    /// widest item, signed only when every item is. A `$` bound counts as
    /// neither; an unpacked-array item counts its elements. None when an
    /// item cannot be sized exactly.
    pub(super) fn inside_items_ws(&mut self, ranges: &[ConstraintRange]) -> Option<(u32, bool)> {
        let (mut w, mut s) = (1u32, true);
        let mut one = |me: &mut Self, e: &Expression, item: bool| -> Option<()> {
            if matches!(e.kind, ExprKind::Dollar) {
                return Some(());
            }
            if item && let Some(nm) = me.array_operand_name(e) {
                for i in 0..me.get_queue_size(&nm) {
                    let ev = me.get_signal_value_by_name(&format!("{}[{}]", nm, i))?;
                    w = w.max(ev.width);
                    s &= ev.is_signed;
                }
                return Some(());
            }
            let (_, ew, es) = me.exact_int(e)?;
            w = w.max(ew);
            s &= es;
            Some(())
        };
        for r in ranges {
            match r {
                ConstraintRange::Value(e) => one(self, e, true)?,
                ConstraintRange::Range { lo, hi } => {
                    one(self, lo, false)?;
                    one(self, hi, false)?;
                }
            }
        }
        Some((w, s))
    }

    /// The object's own member named by `e` (`x`, `this.x`, `obj.x` for the
    /// inline-constraint receiver), if it is a solver scalar or array.
    fn csp_member(&self, csp: &Csp, e: &Expression) -> Option<String> {
        let known = |n: &str| {
            csp.scalars.contains_key(n) || csp.arrays.contains_key(n) || csp.nd_dims.contains_key(n)
        };
        let recv_ok = |me: &Self, r: &str| r == "this" || me.rand_receiver.as_deref() == Some(r);
        // A rand sub-object's member (`a.x`, `this.a.x`, `t.a.x`): a
        // `prop.name` variable when the sub-object joined the solve.
        // (Also `s.x`, a member of an unpacked struct member.)
        if csp.has_subs || csp.dotted {
            if let Some(mut path) = Self::csp_member_path(e) {
                if path.len() >= 3 && (path[0] == "this" || recv_ok(self, &path[0])) {
                    path.remove(0);
                }
                // `a.x`, or `a.b.x` for a sub-object of a sub-object.
                if path.len() >= 2 {
                    let n = path.join(".");
                    if csp.scalars.contains_key(&n) {
                        return Some(n);
                    }
                }
            }
        }
        let name = match &e.kind {
            ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => match h.path.len()
            {
                1 => h.path[0].name.name.clone(),
                2 if recv_ok(self, &h.path[0].name.name) => h.path[1].name.name.clone(),
                _ => return None,
            },
            ExprKind::MemberAccess { expr, member } => match &expr.kind {
                ExprKind::This => member.name.clone(),
                ExprKind::Ident(h) if h.path.len() == 1 && recv_ok(self, &h.path[0].name.name) => {
                    member.name.clone()
                }
                _ => return None,
            },
            _ => return None,
        };
        known(&name).then_some(name)
    }

    /// The names of a plain member path (`a.b`, `this.a.b`), `this` kept.
    /// The segments of a non-wide variable split by `csp_split_slices` that
    /// a field path (`cmd.base`, `this.cmd.base`) spans. None for anything
    /// else, including nested fields.
    fn csp_field_segs(&self, csp: &Csp, e: &Expression) -> Option<Vec<usize>> {
        let mut path = Self::csp_member_path(e)?;
        if path.len() > 2
            && (path[0] == "this" || self.rand_receiver.as_deref() == Some(path[0].as_str()))
        {
            path.remove(0);
        }
        let [var, field] = path.as_slice() else {
            return None;
        };
        let &v = csp.scalars.get(var)?;
        if csp.wide.contains(&v) {
            return None;
        }
        let segs = csp.segs.get(&v)?;
        let &(_, lo, w) = csp.fields.get(&v)?.iter().find(|f| &f.0 == field)?;
        Some(
            segs.iter()
                .filter(|s| s.0 < lo + w && s.0 + s.1 > lo)
                .map(|s| s.2)
                .collect(),
        )
    }

    fn csp_member_path(e: &Expression) -> Option<Vec<String>> {
        match &e.kind {
            ExprKind::This => Some(vec!["this".to_string()]),
            ExprKind::Ident(h) => {
                if h.root.is_some() || h.path.iter().any(|s| !s.selects.is_empty()) {
                    return None;
                }
                Some(h.path.iter().map(|s| s.name.name.clone()).collect())
            }
            ExprKind::MemberAccess { expr, member } => {
                let mut v = Self::csp_member_path(expr)?;
                v.push(member.name.clone());
                Some(v)
            }
            _ => None,
        }
    }

    /// Variables `e` reads. False when it calls something whose reads are
    /// unknown (a user function).
    fn csp_refs(&self, csp: &Csp, e: &Expression, env: &Env, out: &mut Vec<usize>) -> bool {
        // A field of a variable split into segments reads the segments it
        // spans, so a relation on `cmd.base` and `cmd.sz` is judged once
        // those are fixed rather than once all of `cmd` is.
        if matches!(e.kind, ExprKind::Ident(_) | ExprKind::MemberAccess { .. })
            && let Some(segs) = self.csp_field_segs(csp, e)
        {
            out.extend(segs);
            return true;
        }
        // A wide variable is read through its segments.
        let push_var = |v: usize, out: &mut Vec<usize>| match csp.segs.get(&v) {
            Some(segs) if csp.wide.contains(&v) => out.extend(segs.iter().map(|x| x.2)),
            _ => out.push(v),
        };
        let push_name = |n: &str, out: &mut Vec<usize>| {
            if let Some(&v) = csp.scalars.get(n) {
                push_var(v, out);
            } else if let Some(&sv) = csp.sizes.get(n) {
                // Its size is a variable of the size phase.
                out.push(sv);
            } else if let Some(a) = csp.arrays.get(n) {
                for x in &a.elems {
                    push_var(x.1, out);
                }
            } else if csp.nd_dims.contains_key(n) {
                // Every element of a multi-dimensional array (#519).
                let prefix = format!("{n}[");
                for (rn, a) in &csp.arrays {
                    if rn.starts_with(&prefix) {
                        for x in &a.elems {
                            push_var(x.1, out);
                        }
                    }
                }
            }
        };
        match &e.kind {
            ExprKind::Ident(h) => {
                if h.path.len() == 1 {
                    let n = &h.path[0].name.name;
                    if env.binds.iter().any(|b| &b.0 == n) {
                        return true;
                    }
                    if let Some((it, v, _)) = &env.it {
                        if it == n {
                            out.push(*v);
                            return true;
                        }
                    }
                }
                // §18.4 (#519): `a[i][j]` of a multi-dimensional solver
                // array reads that element of its row — every element of
                // the array when an index is not yet known.
                if !csp.nd_dims.is_empty()
                    && h.root.is_none()
                    && h.path.last().is_some_and(|s| !s.selects.is_empty())
                    && h.path[..h.path.len() - 1].iter().all(|s| s.selects.is_empty())
                {
                    let mut bh = h.clone();
                    let sels = std::mem::take(&mut bh.path.last_mut().unwrap().selects);
                    let base = Expression::new(ExprKind::Ident(bh), e.span);
                    if let Some(bn) = self.csp_member(csp, &base)
                        && csp.nd_dims.contains_key(&bn)
                    {
                        let consts: Option<Vec<i64>> =
                            sels.iter().map(|s| Self::nd_const_index(s, env)).collect();
                        match consts {
                            Some(cs) if !cs.is_empty() => {
                                let mut row = bn.clone();
                                for c in &cs[..cs.len() - 1] {
                                    row.push_str(&format!("[{c}]"));
                                }
                                if let Some(a) = csp.arrays.get(&row)
                                    && let Some(x) = a.elems.iter().find(|x| x.0 == cs[cs.len() - 1])
                                {
                                    push_var(x.1, out);
                                    return true;
                                }
                            }
                            _ => {}
                        }
                        let prefix = format!("{bn}[");
                        for (n, a) in &csp.arrays {
                            if n.starts_with(&prefix) {
                                for x in &a.elems {
                                    push_var(x.1, out);
                                }
                            }
                        }
                        return true;
                    }
                }
                if let Some(n) = self.csp_member(csp, e) {
                    push_name(&n, out);
                } else if let Some(seg) = h.path.last() {
                    // `a.b` on some other handle can only name a member of
                    // this object through an alias; stay conservative.
                    if h.path.len() == 1 {
                        push_name(&seg.name.name, out);
                    } else if csp.scalars.contains_key(&h.path[0].name.name) {
                        // A field of a packed-struct variable (`s.lo`) reads
                        // the variable.
                        push_name(&h.path[0].name.name, out);
                    } else if csp.has_subs {
                        // A field or select of a sub-object's variable
                        // (`a.x.f`, `this.a.x[3]`) reads that variable.
                        let p0 = &h.path[0].name.name;
                        let st = usize::from(
                            p0 == "this" || self.rand_receiver.as_deref() == Some(p0.as_str()),
                        );
                        if h.path.len() >= st + 2 {
                            let n =
                                format!("{}.{}", h.path[st].name.name, h.path[st + 1].name.name);
                            push_name(&n, out);
                        }
                    }
                }
                // The select indices read variables too (`f[id][7:0]`).
                let mut ok = true;
                for s in &h.path {
                    for x in &s.selects {
                        ok &= self.csp_refs(csp, x, env, out);
                    }
                }
                ok
            }
            ExprKind::MemberAccess { expr, member } => {
                if let Some(n) = self.csp_member(csp, e) {
                    push_name(&n, out);
                    return true;
                }
                // §8.11: `this.a…` names what the path `a…` names. Settle it
                // like the Ident arm — a lone member reads that member, a
                // field of a packed-struct variable reads the variable, and
                // anything else through a handle is state — rather than
                // descending to `this`, which reads as unanalyzable.
                if let Some(path) = Self::csp_member_path(e)
                    && path.len() >= 2
                    && path[0] == "this"
                {
                    push_name(&path[1], out);
                    return true;
                }
                if member.name == "index" {
                    if let ExprKind::Ident(h) = &expr.kind {
                        if let Some((it, _, _)) = &env.it {
                            if h.path.len() == 1 && &h.path[0].name.name == it {
                                return true;
                            }
                        }
                    }
                }
                self.csp_refs(csp, expr, env, out)
            }
            ExprKind::Call { func, args } => {
                let mut ok = match &func.kind {
                    ExprKind::MemberAccess { expr, .. } => self.csp_refs(csp, expr, env, out),
                    _ => false,
                };
                // A method call on a non-rand handle may still read members.
                if let ExprKind::MemberAccess { expr, .. } = &func.kind {
                    if self.csp_member(csp, expr).is_none()
                        && !matches!(&expr.kind, ExprKind::Ident(_))
                    {
                        ok = false;
                    }
                }
                for a in args {
                    ok &= self.csp_refs(csp, a, env, out);
                }
                ok
            }
            ExprKind::WithClause { expr, filter } => {
                let ok = self.csp_refs(csp, expr, env, out);
                // the filter reads `item`/`item.index`, covered by the array
                let mut tmp = Vec::new();
                let ok2 = self.csp_refs(csp, filter, env, &mut tmp);
                out.extend(tmp);
                ok && ok2
            }
            ExprKind::SystemCall { args, .. } => args
                .iter()
                .fold(true, |ok, a| self.csp_refs(csp, a, env, out) && ok),
            ExprKind::Unary { operand, .. } => self.csp_refs(csp, operand, env, out),
            ExprKind::Binary { left, right, .. } => {
                let a = self.csp_refs(csp, left, env, out);
                self.csp_refs(csp, right, env, out) && a
            }
            ExprKind::Paren(i) => self.csp_refs(csp, i, env, out),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                let a = self.csp_refs(csp, condition, env, out);
                let b = self.csp_refs(csp, then_expr, env, out);
                self.csp_refs(csp, else_expr, env, out) && a && b
            }
            ExprKind::Inside { expr, ranges } => ranges
                .iter()
                .fold(self.csp_refs(csp, expr, env, out), |ok, r| {
                    self.csp_refs(csp, r, env, out) && ok
                }),
            ExprKind::Range(a, b) => {
                let x = self.csp_refs(csp, a, env, out);
                self.csp_refs(csp, b, env, out) && x
            }
            ExprKind::Index { expr, index } => {
                // §18.4 (#519): `a[i][j]` as nested indexes of a
                // multi-dimensional solver array reads one element when the
                // indices are known.
                if !csp.nd_dims.is_empty()
                    && let Some(v) = self.nd_chain_elem(csp, e, env)
                {
                    push_var(v, out);
                    return true;
                }
                let x = self.csp_refs(csp, expr, env, out);
                self.csp_refs(csp, index, env, out) && x
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                let x = self.csp_refs(csp, expr, env, out);
                let y = self.csp_refs(csp, left, env, out);
                self.csp_refs(csp, right, env, out) && x && y
            }
            ExprKind::Concatenation(es) => es
                .iter()
                .fold(true, |ok, a| self.csp_refs(csp, a, env, out) && ok),
            ExprKind::Replication { count, exprs } => exprs
                .iter()
                .fold(self.csp_refs(csp, count, env, out), |ok, a| {
                    self.csp_refs(csp, a, env, out) && ok
                }),
            ExprKind::Number(_) | ExprKind::StringLiteral(_) | ExprKind::TypeLiteral(_) => true,
            _ => false,
        }
    }

    fn csp_item_refs(
        &self,
        csp: &Csp,
        item: &ConstraintItem,
        env: &Env,
        out: &mut Vec<usize>,
    ) -> bool {
        match item {
            ConstraintItem::Expr(e) => self.csp_refs(csp, e, env, out),
            ConstraintItem::Inside { expr, range, .. } => {
                let mut ok = self.csp_refs(csp, expr, env, out);
                for r in range {
                    ok &= match r {
                        ConstraintRange::Value(e) => self.csp_refs(csp, e, env, out),
                        ConstraintRange::Range { lo, hi } => {
                            let a = self.csp_refs(csp, lo, env, out);
                            self.csp_refs(csp, hi, env, out) && a
                        }
                    };
                }
                ok
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                let a = self.csp_refs(csp, condition, env, out);
                self.csp_item_refs(csp, constraint, env, out) && a
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                let a = self.csp_refs(csp, condition, env, out);
                let b = self.csp_item_refs(csp, then_item, env, out);
                let c = else_item
                    .as_ref()
                    .is_none_or(|e| self.csp_item_refs(csp, e, env, out));
                a && b && c
            }
            ConstraintItem::Foreach { array, item, .. } => {
                // the loop index is not a variable; the array's elements are
                let a = self.csp_refs(csp, array, env, out);
                let mut inner = Vec::new();
                let b = self.csp_item_refs(csp, item, env, &mut inner);
                out.extend(inner);
                a && b
            }
            ConstraintItem::Solve { .. } => true,
            ConstraintItem::Soft(i) => self.csp_item_refs(csp, i, env, out),
            ConstraintItem::Block(items) => items
                .iter()
                .fold(true, |ok, i| self.csp_item_refs(csp, i, env, out) && ok),
            ConstraintItem::Unique { exprs, .. } => exprs
                .iter()
                .fold(true, |ok, e| self.csp_refs(csp, e, env, out) && ok),
        }
    }

    /// Reads no solver variable (and calls nothing opaque).
    fn csp_free(&self, csp: &Csp, e: &Expression, env: &Env) -> bool {
        let mut v = Vec::new();
        self.csp_refs(csp, e, env, &mut v) && v.is_empty()
    }

    /// Evaluate a variable-free expression with the foreach indices bound.
    fn csp_const_any(&mut self, e: &Expression, env: &Env) -> Value {
        let real = Self::csp_obj_real(e);
        let e = real.as_ref().unwrap_or(e);
        let frame = self.csp_frame(&env.binds);
        self.push_local_frame(frame);
        let v = self.eval_expr(e);
        self.pop_local_frame();
        v
    }

    fn csp_const(&mut self, e: &Expression, env: &Env) -> Option<Value> {
        let v = self.csp_const_any(e, env);
        (!v.has_xz() && !v.is_real && v.width > 0 && v.width <= 64).then_some(v)
    }

    /// Arithmetic operand tree of `e`, or None when it is not linear. A
    /// variable wider than 64 bits is no operand: it is only modelled
    /// through its segments (`csp_wide_rel`).
    fn csp_ae(&mut self, csp: &Csp, e: &Expression, env: &Env) -> Option<Ae> {
        let a = self.csp_ae_raw(csp, e, env)?;
        if !csp.wide.is_empty() && matches!(a, Ae::Var(v, ..) if csp.wide.contains(&v)) {
            return None;
        }
        Some(a)
    }

    fn csp_ae_raw(&mut self, csp: &Csp, e: &Expression, env: &Env) -> Option<Ae> {
        let e = Self::unparen(e);
        // §18.4 (#519): an element of a multi-dimensional solver array
        // (`a[i][j]`, as nested indexes or as selects on the identifier)
        // is the row's element variable.
        if !csp.nd_dims.is_empty()
            && matches!(e.kind, ExprKind::Index { .. } | ExprKind::Ident(_))
            && let Some(v) = self.nd_chain_elem(csp, e, env)
        {
            return Some(Self::csp_var_ae(csp, v));
        }
        // the `with` iterator and its index
        if let Some((it, v, i)) = &env.it {
            match &e.kind {
                ExprKind::Ident(h) if h.path.len() == 1 && &h.path[0].name.name == it => {
                    return Some(Self::csp_var_ae(csp, *v));
                }
                ExprKind::MemberAccess { expr, member } if member.name == "index" => {
                    if let ExprKind::Ident(h) = &expr.kind {
                        if h.path.len() == 1 && &h.path[0].name.name == it {
                            // §7.12.4: `index` is an int
                            let mut ix = Value::from_u64(*i as u64, 32);
                            ix.is_signed = true;
                            return Some(Ae::Const(ix));
                        }
                    }
                }
                _ => {}
            }
        }
        if matches!(
            e.kind,
            ExprKind::RangeSelect { .. } | ExprKind::Index { .. } | ExprKind::MemberAccess { .. }
        ) || (!csp.segs.is_empty() && matches!(e.kind, ExprKind::Ident(_)))
        {
            if let Some(a) = self.csp_slice_ae(csp, e, env) {
                return Some(a);
            }
        }
        if let Some(n) = self.csp_member(csp, e) {
            if !env.binds.iter().any(|b| b.0 == n) {
                return csp.scalars.get(&n).map(|&v| Self::csp_var_ae(csp, v));
            }
        }
        match &e.kind {
            // §11.4.12: `{k'b0, x}` is `x` zero-extended by `k` bits.
            ExprKind::Concatenation(parts)
                if parts.len() == 2 && self.csp_free(csp, &parts[0], env) =>
            {
                let k = self.csp_const(&parts[0], env)?;
                if k.to_u64() != Some(0) || k.is_signed {
                    return None;
                }
                let a = self.csp_ae(csp, &parts[1], env)?;
                let (w, sg) = Self::csp_ws(&a);
                if sg {
                    return None;
                }
                return self.csp_cast(csp, &parts[1], w + k.width, Some(false), env);
            }
            ExprKind::Unary {
                op: UnaryOp::Plus,
                operand,
            } => return self.csp_ae(csp, operand, env),
            ExprKind::Unary {
                op: UnaryOp::Minus,
                operand,
            } => return Some(Ae::Neg(Box::new(self.csp_ae(csp, operand, env)?))),
            // `a << k` with a constant k is exactly `a * 2**k` (§11.4.10);
            // `<<` and `<<<` agree on a left shift. The solver already models
            // Mul, so this reuses its width/truncation handling rather than
            // dropping the whole relation to the trial loop.
            ExprKind::Binary {
                op: BinaryOp::ShiftLeft | BinaryOp::ArithShiftLeft,
                left,
                right,
            } if self.csp_free(csp, right, env) => {
                if let Some(k) = self
                    .csp_const(right, env)
                    .and_then(|v| v.to_u64())
                    .filter(|k| *k < 63)
                {
                    if let Some(a) = self.csp_ae(csp, left, env) {
                        let (w, sg) = Self::csp_ws(&a);
                        let mut m = Value::from_u64(1u64 << k, w.max(1));
                        m.is_signed = sg;
                        return Some(Ae::Mul(Box::new(a), Box::new(Ae::Const(m))));
                    }
                }
            }
            ExprKind::Binary {
                op: BinaryOp::ShiftRight,
                left,
                right,
            } if self.csp_free(csp, right, env) => {
                if let Some(a) = self.csp_shr_slice(csp, left, right, env) {
                    return Some(a);
                }
            }
            ExprKind::Binary {
                op: op @ (BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul),
                left,
                right,
            } => {
                let (l, r) = (self.csp_ae(csp, left, env)?, self.csp_ae(csp, right, env)?);
                let (l, r) = (Box::new(l), Box::new(r));
                return Some(match op {
                    BinaryOp::Add => Ae::Add(l, r),
                    BinaryOp::Sub => Ae::Sub(l, r),
                    _ => Ae::Mul(l, r),
                });
            }
            ExprKind::Index { expr, index } => {
                if let Some(a) = self.csp_member(csp, expr).and_then(|n| csp.arrays.get(&n)) {
                    if !self.csp_free(csp, index, env) {
                        return None;
                    }
                    let i = self.csp_const(index, env)?.to_i64()?;
                    return a
                        .elems
                        .iter()
                        .find(|x| x.0 == i)
                        .map(|x| Self::csp_var_ae(csp, x.1));
                }
            }
            ExprKind::SystemCall { name, args } => match (name.as_str(), args.as_slice()) {
                ("$__xz_type_cast", [t, inner]) => {
                    let ExprKind::TypeLiteral(dt) = &t.kind else {
                        return None;
                    };
                    if is_type_real(dt) {
                        return None;
                    }
                    let w = resolve_type_width(
                        dt,
                        Some(&self.module.parameters),
                        Some(&self.module.typedefs),
                    );
                    let s = is_type_signed(dt);
                    return self.csp_cast(csp, inner, w, Some(s), env);
                }
                ("$__xz_size_cast", [n, inner]) if self.csp_free(csp, n, env) => {
                    let w = self.csp_const(n, env)?.to_u64()? as u32;
                    return self.csp_cast(csp, inner, w, None, env);
                }
                // `T'(e)` with `T` a typedef/enum, or a constant size.
                ("$__xz_named_cast", [t, inner]) => {
                    let nm = self.named_cast_key(t)?;
                    let nm = nm.as_str();
                    let (w, s) = if let Some(dt) = self.module.typedef_types.get(nm) {
                        if is_type_real(dt) {
                            return None;
                        }
                        (self.cast_context_width(dt), Some(is_type_signed(dt)))
                    } else if let Some(&w) = self.module.typedefs.get(nm) {
                        (w, Some(false))
                    } else if self.csp_free(csp, t, env) {
                        (self.csp_const(t, env)?.to_u64()? as u32, None)
                    } else {
                        return None;
                    };
                    return self.csp_cast(csp, inner, w, s, env);
                }
                // §20.9: `$countones(x)` of a variable split into single
                // bits is the (int) sum of those bits.
                ("$countones", [x]) => {
                    if let Some(a) = self.csp_countones(csp, x, env) {
                        return Some(a);
                    }
                }
                ("$signed", [inner]) | ("$unsigned", [inner]) => {
                    let a = self.csp_ae(csp, inner, env)?;
                    let (iw, _) = Self::csp_ws(&a);
                    return self.csp_cast(csp, inner, iw, Some(name == "$signed"), env);
                }
                _ => {}
            },
            _ => {}
        }
        // §18.5.8.1: in the size phase an array's size is a variable.
        if !csp.sizes.is_empty()
            && let ExprKind::Call { func, args } = &e.kind
            && args.is_empty()
            && let ExprKind::MemberAccess { expr, member } = &func.kind
            && member.name == "size"
            && let Some(&sv) = self
                .csp_member(csp, expr)
                .or_else(|| self.csp_row_of(csp, expr, env))
                .and_then(|n| csp.sizes.get(&n))
        {
            return Some(Self::csp_var_ae(csp, sv));
        }
        // The size of a wide solver array is fixed while it is solved.
        if !csp.wide.is_empty()
            && let ExprKind::Call { func, args } = &e.kind
            && args.is_empty()
            && let ExprKind::MemberAccess { expr, member } = &func.kind
            && member.name == "size"
            && let Some(a) = self.csp_member(csp, expr).and_then(|n| csp.arrays.get(&n))
            && a.elems.first().is_some_and(|x| csp.wide.contains(&x.1))
        {
            let mut v = Value::from_u64(a.elems.len() as u64, 32);
            v.is_signed = true;
            return Some(Ae::Const(v));
        }
        if let Some(sum) = self.csp_sum(csp, e, env) {
            return sum;
        }
        // `arr.size()` of a solver array is a constant here
        if let ExprKind::Call { func, args } = &e.kind {
            if let ExprKind::MemberAccess { expr, member } = &func.kind {
                if args.is_empty() && member.name == "size" {
                    if let Some(a) = self.csp_member(csp, expr).and_then(|n| csp.arrays.get(&n)) {
                        let mut v = Value::from_u64(a.elems.len() as u64, 32);
                        v.is_signed = true;
                        return Some(Ae::Const(v));
                    }
                    // §7.4.5: a row's size (`a[1].size()`), and the number
                    // of rows of a member over rows (`a.size()`).
                    let n = match self.csp_row_of(csp, expr, env) {
                        Some(row) => csp.arrays.get(&row).map(|a| a.elems.len() as u64),
                        None => self
                            .csp_member(csp, expr)
                            .filter(|n| !csp.arrays.contains_key(n))
                            .and_then(|n| csp.nd_dims.get(&n))
                            .and_then(|d| d.first())
                            .map(|&(lo, hi)| (hi - lo + 1).max(0) as u64),
                    };
                    if let Some(n) = n {
                        let mut v = Value::from_u64(n, 32);
                        v.is_signed = true;
                        return Some(Ae::Const(v));
                    }
                }
            }
        }
        if self.csp_free(csp, e, env) {
            return self.csp_const(e, env).map(Ae::Const);
        }
        None
    }

    /// §11.4.10: `x >> k` with a constant `k` fills with zeros, so it is
    /// the select `x[w-1:k]` zero-extended to `x`'s width. `csp_scan_expr`
    /// cut `x` there, so the select is a variable of its own and a relation
    /// like `(base >> 16) == (last >> 16)` is linear in two selects instead
    /// of judged only once both sides are fixed.
    fn csp_shr_slice(
        &mut self,
        csp: &Csp,
        x: &Expression,
        k: &Expression,
        env: &Env,
    ) -> Option<Ae> {
        let k = self.csp_const(k, env)?.to_u64()?;
        let mut konst = |me: &mut Self, y: &Expression| -> Option<i64> {
            if !me.csp_free(csp, y, env) {
                return None;
            }
            me.csp_const(y, env)?.to_i64()
        };
        let (v, lo, w, _) = self.csp_slice_base_k(csp, x, &mut konst)?;
        if k == 0 || k >= u64::from(w) {
            return None;
        }
        let (slo, sw) = (lo + k as u32, w - k as u32);
        let sel = csp
            .segs
            .get(&v)
            .and_then(|segs| segs.iter().find(|s| s.0 == slo && s.1 == sw).map(|s| s.2))
            .or_else(|| csp.slice_vars.get(&(v, slo, sw)).copied())?;
        let (xw, xs) = Self::csp_ws(&self.csp_ae(csp, x, env)?);
        if xw != w || xw > 64 {
            return None;
        }
        Some(Ae::Cast {
            inner: Box::new(Ae::Var(sel, sw, false)),
            iw: sw,
            is: false,
            w: xw,
            s: xs,
        })
    }

    fn csp_countones(&mut self, csp: &Csp, x: &Expression, env: &Env) -> Option<Ae> {
        let mut konst = |me: &mut Self, e: &Expression| -> Option<i64> {
            if !me.csp_free(csp, e, env) {
                return None;
            }
            me.csp_const(e, env)?.to_i64()
        };
        let (v, lo, w, whole) = self.csp_slice_base_k(csp, x, &mut konst)?;
        // A variable split into single bits anyway (a select at a variable
        // position) counts them; a whole variable otherwise has its count.
        let bits: Option<Vec<usize>> = csp.segs.get(&v).and_then(|segs| {
            segs.iter()
                .filter(|s| s.0 >= lo && s.0 < lo + w)
                .map(|s| (s.1 == 1).then_some(s.2))
                .collect::<Option<Vec<_>>>()
        });
        let bits = match bits {
            Some(b) if b.len() == w as usize => b,
            _ => {
                let &c = csp.pop.get(&v)?;
                return (whole && lo == 0 && w == csp.vars[v].width)
                    .then(|| Self::csp_var_ae(csp, c));
            }
        };
        let terms = bits
            .into_iter()
            .map(|b| Ae::Cast {
                inner: Box::new(Ae::Var(b, 1, false)),
                iw: 32,
                is: false,
                w: 32,
                s: true,
            })
            .collect();
        Some(Ae::Sum {
            terms,
            w: 32,
            s: true,
        })
    }

    fn csp_cast(
        &mut self,
        csp: &Csp,
        inner: &Expression,
        w: u32,
        s: Option<bool>,
        env: &Env,
    ) -> Option<Ae> {
        let a = self.csp_ae(csp, inner, env)?;
        // §6.24.1: the cast width is the operand's context, so the operand
        // is evaluated at no less than the target width.
        let (iw, is) = Self::csp_ws(&a);
        let iw = iw.max(w);
        if w == 0 || w > 64 || iw > 64 {
            return None;
        }
        Some(Ae::Cast {
            inner: Box::new(a),
            iw,
            is,
            w,
            s: s.unwrap_or(is),
        })
    }

    /// `arr.sum()` / `arr.sum() with (f)` / `arr.sum(it) with (f)` over a
    /// solver array. `Some(None)` for a reduction the model cannot express.
    fn csp_sum(&mut self, csp: &Csp, e: &Expression, env: &Env) -> Option<Option<Ae>> {
        let (call, filter) = match &e.kind {
            ExprKind::WithClause { expr, filter } => (expr.as_ref(), Some(filter.as_ref())),
            _ => (e, None),
        };
        let (recv, method, args): (&Expression, &str, &[Expression]) = match &call.kind {
            ExprKind::Call { func, args } => match &func.kind {
                ExprKind::MemberAccess { expr, member } => (expr, member.name.as_str(), args),
                _ => return None,
            },
            ExprKind::MemberAccess { expr, member } => (expr, member.name.as_str(), &[]),
            _ => return None,
        };
        // `arr.size()` is a constant of the solve (below `csp_sum`).
        if method == "size" {
            return None;
        }
        let name = self.csp_member(csp, recv)?;
        let arr = csp.arrays.get(&name)?.clone();
        if method != "sum" || csp.sizes.contains_key(&name) {
            return Some(None);
        }
        let it = match args {
            [] => "item".to_string(),
            [a] => match &Self::unparen(a).kind {
                ExprKind::Ident(h) if h.path.len() == 1 => h.path[0].name.name.clone(),
                _ => return Some(None),
            },
            _ => return Some(None),
        };
        let mut terms = Vec::with_capacity(arr.elems.len());
        for &(i, v) in &arr.elems {
            let t = match filter {
                None => Some(Self::csp_var_ae(csp, v)),
                Some(f) => {
                    let mut e2 = env.clone();
                    e2.it = Some((it.clone(), v, i));
                    self.csp_ae(csp, f, &e2)
                }
            };
            let Some(t) = t else {
                return Some(None);
            };
            terms.push(t);
        }
        let (w, s) = match (filter, terms.first()) {
            (Some(_), Some(t)) => Self::csp_ws(t),
            _ => (arr.width, arr.signed),
        };
        if terms.iter().any(|t| Self::csp_ws(t) != (w, s)) || w > 64 {
            return Some(None);
        }
        Some(Some(Ae::Sum { terms, w, s }))
    }

    fn csp_var_ae(csp: &Csp, v: usize) -> Ae {
        Ae::Var(v, csp.vars[v].width, csp.vars[v].signed)
    }

    /// Self-determined (width, signed) of an operand tree.
    fn csp_ws(a: &Ae) -> (u32, bool) {
        match a {
            Ae::Var(_, w, s) => (*w, *s),
            Ae::Const(v) => (v.width, v.is_signed),
            Ae::Neg(x) => Self::csp_ws(x),
            Ae::Add(l, r) | Ae::Sub(l, r) | Ae::Mul(l, r) => {
                let ((a, b), (c, d)) = (Self::csp_ws(l), Self::csp_ws(r));
                (a.max(c), b && d)
            }
            Ae::Sum { w, s, .. } | Ae::Cast { w, s, .. } => (*w, *s),
        }
    }

    /// Linear form of `a` in the context `(w, s)`, recording the fits that
    /// keep it exact.
    fn csp_lin(a: &Ae, w: u32, s: bool, fits: &mut Vec<Fit>, cx: &mut LinCx) -> Option<Lin> {
        match a {
            Ae::Var(v, vw, vs) => {
                // an unsigned context zero-extends a signed operand; one that
                // already fills the context is only reinterpreted (§11.8.2)
                if *vs && !s {
                    fits.push(Fit {
                        lin: Lin::var(*v),
                        lo: 0,
                        hi: i128::MAX,
                        modw: if *vw >= w { *vw } else { 0 },
                    });
                }
                Some(Lin::var(*v))
            }
            Ae::Const(v) => {
                let raw = v.to_u64()?;
                Some(Lin::konst(Self::as_ctx(raw as i128, v.width, s)))
            }
            Ae::Neg(x) => Self::csp_lin(x, w, s, fits, cx)?.scale(-1),
            Ae::Add(l, r) => {
                Self::csp_lin(l, w, s, fits, cx)?.plus(&Self::csp_lin(r, w, s, fits, cx)?, 1)
            }
            Ae::Sub(l, r) => {
                Self::csp_lin(l, w, s, fits, cx)?.plus(&Self::csp_lin(r, w, s, fits, cx)?, -1)
            }
            Ae::Mul(l, r) => {
                let (x, y) = (
                    Self::csp_lin(l, w, s, fits, cx)?,
                    Self::csp_lin(r, w, s, fits, cx)?,
                );
                if x.is_const() {
                    y.scale(x.k)
                } else if y.is_const() {
                    x.scale(y.k)
                } else {
                    None
                }
            }
            Ae::Sum {
                terms,
                w: sw,
                s: ss,
            } => {
                let mut acc = Lin::konst(0);
                let mark = fits.len();
                for t in terms {
                    acc = acc.plus(&Self::csp_lin(t, *sw, *ss, fits, cx)?, 1)?;
                }
                // §7.12.3: the sum is computed at its own width and wraps
                // there (`a.sum() == 3` over 16-bit elements also holds for
                // terms adding up to 3 + 2^16·k).
                if let Some(v) = cx.carry(&acc, *sw, *ss) {
                    // The carry identity holds modulo 2^sw, so a term fit
                    // that only guards a reinterpretation at sw bits or more
                    // (`int'(unsigned'(item))`) cannot change the sum's
                    // value: drop it, or it would stop the relation on the
                    // sum from propagating until every term is fixed.
                    let kept: Vec<Fit> = fits.drain(mark..).filter(|f| f.modw < *sw).collect();
                    fits.extend(kept);
                    return Self::csp_lin(&Ae::Var(v, *sw, *ss), w, s, fits, cx);
                }
                Self::csp_leaf_fit(&acc, *sw, *ss, *sw, *ss, w, s, fits);
                Some(acc)
            }
            Ae::Cast {
                inner,
                iw,
                is,
                w: cw,
                s: cs,
            } => {
                let l = Self::csp_lin(inner, *iw, *is, fits, cx)?;
                Self::csp_leaf_fit(&l, *iw, *is, *cw, *cs, w, s, fits);
                Some(l)
            }
        }
    }

    /// A sub-result computed at `(iw, is)`, converted to `(cw, cs)` and then
    /// extended into a context of signedness `ctx_s`, keeps its value when it
    /// lies in both ranges (and is non-negative when a signed value meets an
    /// unsigned context, which zero-extends).
    ///
    /// Outside that range the value differs from `l` by a multiple of
    /// `2^cw` (§6.24.1: it is evaluated at `iw >= cw` bits and truncated to
    /// `cw`), which an equality compared at `ctx_w <= cw` bits does not see —
    /// unless the value is extended into a wider context first.
    #[allow(clippy::too_many_arguments)]
    fn csp_leaf_fit(
        l: &Lin,
        iw: u32,
        is: bool,
        cw: u32,
        cs: bool,
        ctx_w: u32,
        ctx_s: bool,
        fits: &mut Vec<Fit>,
    ) {
        let (a, b) = ws_range(iw, is);
        let (c, d) = ws_range(cw, cs);
        let lo = if cs && !ctx_s {
            a.max(c).max(0)
        } else {
            a.max(c)
        };
        fits.push(Fit {
            lin: l.clone(),
            lo,
            hi: b.min(d),
            modw: if cw >= ctx_w { cw } else { 0 },
        });
    }
}

/// §11.5.1/§7.2.1: a constant bit or part select, or a packed-struct field,
/// of a solver scalar. The scalar is split into segments at every boundary
/// such a select uses, tied to it by the exact identity `v == Σ seg·2^lsb`;
/// a select is then a plain variable (a segment, or an auxiliary variable
/// equal to the segments it covers). Relations on selects become linear and
/// propagate, instead of waiting for a guess of the whole scalar to land on
/// the selected bits (`addr[39:16] == page` on a 40-bit `addr`).
impl Simulator {
    /// Field layout and declared lsb of every scalar, then the selects the
    /// constraints use and the segments they need.
    fn csp_split_slices(&mut self, csp: &mut Csp, constraints: &[ClassConstraint]) {
        csp.segs.clear();
        csp.slice_vars.clear();
        csp.aux_base.clear();
        csp.fields.clear();
        csp.field_elems.clear();
        csp.pdims.clear();
        csp.lsb.clear();
        csp.ascending.clear();
        let scalars: Vec<(String, usize)> =
            csp.scalars.iter().map(|(n, &v)| (n.clone(), v)).collect();
        for (name, v) in &scalars {
            if !csp.wide.contains(v) && (csp.vars[*v].signed || csp.vars[*v].width > 64) {
                continue;
            }
            if let Some((mut fields, lsb)) = self.csp_scalar_shape(csp, name) {
                let sub_var = matches!(csp.vars[*v].key, VarKey::Sub(..));
                if (csp.wide.contains(v) || sub_var) && !fields.is_empty() {
                    self.csp_nested_fields(csp, name, &mut fields);
                }
                if !fields.is_empty() && csp.scope.is_none() {
                    for (k, e) in self.csp_field_arrays(csp, name, &fields) {
                        csp.field_elems.insert((*v, k.0, k.1), e);
                    }
                }
                if !fields.is_empty() {
                    csp.fields.insert(*v, fields);
                }
                if let Some((left, right)) = lsb {
                    csp.lsb.insert(*v, right);
                    if left < right {
                        csp.ascending.insert(*v);
                    }
                }
            } else if let Some(dims) = self.csp_packed_dims(csp, name, csp.vars[*v].width) {
                // §7.4.1: selects of a multi-dimensional packed variable
                // name elements of its dimensions (`csp_pdim_ref`).
                csp.pdims.insert(*v, dims);
            } else {
                // Other shapes are not split here.
                csp.lsb.insert(*v, i64::MIN);
            }
        }
        // §7.4/§11.5.1: the elements of a rand array are split the same way
        // when the constraints select bits or fields of them (`a[i][39:16]`,
        // `s[i].f`); every element shares the declared element shape.
        let arrays: Vec<(String, Vec<usize>, bool)> = csp
            .arrays
            .iter()
            .map(|(n, a)| {
                let elems: Vec<usize> = a.elems.iter().map(|e| e.1).collect();
                let wide = elems.first().is_some_and(|v| csp.wide.contains(v));
                (n.clone(), elems, !wide && (a.signed || a.width > 64))
            })
            .collect();
        for (name, elems, skip) in arrays {
            let shape = if skip {
                None
            } else {
                self.csp_scalar_shape(csp, &name)
            };
            let dims = match (&shape, elems.first()) {
                (None, Some(&e)) if !skip => self.csp_packed_dims(csp, &name, csp.vars[e].width),
                _ => None,
            };
            for v in elems {
                match &shape {
                    Some((fields, lsb)) => {
                        if !fields.is_empty() {
                            csp.fields.insert(v, fields.clone());
                        }
                        if let Some((left, right)) = *lsb {
                            csp.lsb.insert(v, right);
                            if left < right {
                                csp.ascending.insert(v);
                            }
                        }
                    }
                    None => match &dims {
                        Some(d) => {
                            csp.pdims.insert(v, d.clone());
                        }
                        None => {
                            csp.lsb.insert(v, i64::MIN);
                        }
                    },
                }
            }
        }
        // (base) -> constant ranges, and whether some select is dynamic.
        let mut uses: HashMap<usize, (Vec<(u32, u32)>, bool)> = HashMap::default();
        let mut bound: Vec<String> = Vec::new();
        let prev_any = CSP_SCAN_ANY.with(|c| c.replace(true));
        let prev_pops = CSP_SCAN_POPS.with(|p| std::mem::take(&mut *p.borrow_mut()));
        let prev_links = CSP_SCAN_LINKS.with(|p| std::mem::take(&mut *p.borrow_mut()));
        for c in constraints {
            for it in &c.items {
                self.csp_scan_item(csp, it, &mut bound, &mut uses);
            }
        }
        CSP_SCAN_ANY.with(|c| c.set(prev_any));
        let mut pops = CSP_SCAN_POPS.with(|p| std::mem::replace(&mut *p.borrow_mut(), prev_pops));
        let links = CSP_SCAN_LINKS.with(|p| std::mem::replace(&mut *p.borrow_mut(), prev_links));
        // As for the cuts below: a loop over an array counts every element.
        for a in csp.arrays.values() {
            if a.elems.iter().any(|e| pops.contains(&e.1)) {
                pops.extend(a.elems.iter().map(|e| e.1));
            }
        }
        // An element select seen through a `foreach` index was recorded on
        // one element: give every element of that array the same cuts.
        for a in csp.arrays.values() {
            let mut all: (Vec<(u32, u32)>, bool) = (Vec::new(), false);
            for &(_, v) in &a.elems {
                if let Some(u) = uses.get(&v) {
                    all.0.extend(u.0.iter().copied());
                    all.1 |= u.1;
                }
            }
            if all.0.is_empty() && !all.1 {
                continue;
            }
            all.0.sort_unstable();
            all.0.dedup();
            for &(_, v) in &a.elems {
                uses.insert(v, all.clone());
            }
        }
        if !csp.wide.is_empty() {
            self.csp_wide_split(csp, &uses, &links);
        }
        for (v, (ranges, dynamic)) in uses {
            let w = csp.vars[v].width;
            if csp.vars[v].signed || w > 64 || w < 2 || csp.lsb.get(&v) == Some(&i64::MIN) {
                continue;
            }
            // Keep the problem within the solver's size.
            if csp.vars.len() + ranges.len() * 2 + if dynamic { w as usize } else { 0 } > MAX_VARS {
                continue;
            }
            let mut cuts: Vec<u32> = vec![0, w];
            if dynamic {
                cuts.extend(1..w);
            }
            for &(lo, rw) in &ranges {
                cuts.push(lo);
                cuts.push(lo + rw);
            }
            cuts.retain(|&c| c <= w);
            cuts.sort_unstable();
            cuts.dedup();
            if cuts.len() <= 2 {
                continue;
            }
            let mut segs = Vec::with_capacity(cuts.len() - 1);
            for k in 0..cuts.len() - 1 {
                let (lo, hi) = (cuts[k], cuts[k + 1]);
                let x = Self::csp_aux_var(csp, hi - lo);
                csp.aux_base.insert(x, (v, lo, hi - lo));
                segs.push((lo, hi - lo, x));
            }
            for &(lo, rw) in &ranges {
                let covered = segs
                    .iter()
                    .filter(|s| s.0 >= lo && s.0 + s.1 <= lo + rw)
                    .count();
                if covered > 1 && !csp.slice_vars.contains_key(&(v, lo, rw)) {
                    let x = Self::csp_aux_var(csp, rw);
                    csp.aux_base.insert(x, (v, lo, rw));
                    csp.slice_vars.insert((v, lo, rw), x);
                }
            }
            csp.segs.insert(v, segs);
        }
        pops.sort_unstable();
        pops.dedup();
        csp.pop.clear();
        csp.pop_rev.clear();
        for x in pops {
            let w = csp.vars[x].width;
            if w == 0 || w > 64 || csp.wide.contains(&x) || csp.vars.len() >= MAX_VARS {
                continue;
            }
            // Split into single bits anyway (a select at a variable
            // position): `csp_countones` sums them instead.
            if csp
                .segs
                .get(&x)
                .is_some_and(|segs| segs.iter().all(|s| s.1 == 1))
            {
                continue;
            }
            // §20.9: `$countones` returns an int.
            csp.vars.push(CspVar {
                key: VarKey::Aux,
                width: 32,
                signed: true,
            });
            csp.dom0.push(vec![(0, w as i128)]);
            let c = csp.vars.len() - 1;
            csp.pop.insert(x, c);
            csp.pop_rev.insert(c, x);
        }
    }

    /// Create the variables of a translated relation's carries (`LinCx`)
    /// and their defining equalities `acc - s - 2^w·k == 0`.
    fn csp_add_carries(&mut self, csp: &mut Csp, carries: Vec<Carry>) {
        for c in carries {
            debug_assert!(csp.vars.len() >= csp.carry_base.unwrap_or(0));
            let sv = csp.vars.len();
            csp.vars.push(CspVar {
                key: VarKey::Aux,
                width: c.w,
                signed: c.s,
            });
            let (lo, hi) = ws_range(c.w, c.s);
            csp.dom0.push(vec![(lo, hi)]);
            let kv = csp.vars.len();
            csp.vars.push(CspVar {
                key: VarKey::Aux,
                width: 64,
                signed: true,
            });
            csp.dom0.push(vec![c.k]);
            let terms: Vec<usize> = c.acc.t.iter().map(|t| t.0).collect();
            // `LinCx::carry` keeps 2^w within COEF_MAX.
            let lin = Lin {
                t: vec![(sv, -1), (kv, -(1i128 << c.w))],
                k: 0,
            }
            .plus(&c.acc, 1)
            .expect("carry within the coefficient bound");
            let mut deps: Vec<usize> = lin.t.iter().map(|t| t.0).collect();
            deps.sort_unstable();
            let one = Self::literal_of(&Value::from_u64(1, 1), crate::ast::Span::dummy());
            csp.carries.push((sv, kv, terms));
            csp.carry_accs.push((c.acc.clone(), c.w, c.s));
            let src = self.csp_src(csp, SrcItem::Expr(one), &Env::default(), deps);
            csp.nodes.push(Node::Lin {
                lin,
                rel: Rel::Eq,
                fits: Vec::new(),
                src,
                sneg: false,
                wrap: None,
            });
        }
    }

    fn csp_aux_var(csp: &mut Csp, w: u32) -> usize {
        csp.vars.push(CspVar {
            key: VarKey::Aux,
            width: w,
            signed: false,
        });
        let (lo, hi) = ws_range(w, false);
        csp.dom0.push(vec![(lo, hi)]);
        csp.vars.len() - 1
    }

    /// Packed-struct fields and declared (left, right) of scalar `name`.
    /// Struct fields use zero-based storage; unsupported shapes return None.
    /// The declared type of member `name` of the object `owner`, searched
    /// up its class chain; a class type parameter stands for the type this
    /// object bound to it.
    fn csp_prop_decl_type(&self, owner: usize, name: &str) -> Option<DataType> {
        let class = self
            .heap
            .get(owner)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone());
        let mut cur = class;
        let mut found = None;
        while let Some(cn) = cur {
            let Some(cd) = self.module.classes.get(&cn) else {
                break;
            };
            if let Some(t) = self.class_prop_decl_type(cd, name) {
                found = Some(t.clone());
                // §8.25: an element typed by a class type parameter
                // (`rand T beats[];`) has the shape of the type this
                // object bound to it.
                if let DataType::TypeReference {
                    name: tn,
                    dimensions,
                    ..
                } = t
                    && dimensions.is_empty()
                    && !tn.has_scope()
                    && cd.type_param_names.contains(&tn.name.name)
                    && let Some(b) = self.class_prop_type_name(owner, name)
                    && let Some(bt) = self.module.typedef_types.get(b.as_str())
                {
                    found = Some(bt.clone());
                }
                break;
            }
            cur = cd.extends.clone();
        }
        found
    }

    /// §7.4.1: the packed dimensions (left, right), outermost first, of a
    /// solver scalar or array `name` declared as a multi-dimensional packed
    /// vector (`bit [8:0][7:0] w`) whose dimensions make up `width` bits.
    fn csp_packed_dims(&self, csp: &Csp, name: &str, width: u32) -> Option<Vec<(i64, i64)>> {
        let (dt, params) = if csp.scope.is_some() {
            let name = self.dyn_name_lookup(name).unwrap_or(name);
            (
                self.module.var_decl_types.get(name)?.clone(),
                self.module.parameters.clone(),
            )
        } else {
            let (owner, name) = Self::csp_owner(csp, name);
            (
                self.csp_prop_decl_type(owner, name)?,
                self.instance_param_scope(owner),
            )
        };
        let dims = crate::compiler::elaborate::packed_full_dims_chained(
            &dt,
            &params,
            &self.module.typedef_types,
        )?;
        let bits = dims.iter().try_fold(1u64, |acc, &(l, r)| {
            acc.checked_mul((l - r).unsigned_abs().checked_add(1)?)
        })?;
        (dims.len() >= 2 && bits == u64::from(width)).then_some(dims)
    }

    #[allow(clippy::type_complexity)]
    fn csp_scalar_shape(
        &mut self,
        csp: &Csp,
        name: &str,
    ) -> Option<(Vec<(String, u32, u32)>, Option<(i64, i64)>)> {
        let dt: Option<DataType> = if csp.scope.is_some() {
            // A collection local to an automatic task is registered under
            // its per-call key (`declare_local_dyn`).
            let name = self.dyn_name_lookup(name).unwrap_or(name);
            if let Some(f) = self.module.packed_struct_fields.get(name) {
                return Some((f.clone(), None));
            }
            self.module.var_decl_types.get(name).cloned()
        } else {
            // A sub-object's member is declared by the sub-object's class.
            let (owner, name) = Self::csp_owner(csp, name);
            // A row of a multi-dimensional member (`a[1]`) has its
            // element type (§7.4.5, #519).
            let name = match name.split_once('[') {
                Some((b, _)) if csp.nd_dims.contains_key(b) => b,
                _ => name,
            };
            if let Some(su) = self.class_prop_struct(owner, name) {
                if !su.packed {
                    return None;
                }
                let params = self.instance_param_scope(owner);
                let (fields, _) = self.packed_agg_layout_with(&su, &params);
                return Some((fields, None));
            }
            self.csp_prop_decl_type(owner, name)
        };
        let Some(dt) = dt else {
            return Some((Vec::new(), Some((0, 0))));
        };
        let dt = crate::compiler::elaborate::resolve_typedef_chain(&dt, &self.module.typedef_types)
            .clone();
        if let Some(f) = crate::compiler::elaborate::packed_struct_field_layout(
            &dt,
            &self.module.parameters,
            &self.module.typedefs,
            &self.module.typedef_types,
        ) {
            return Some((f, None));
        }
        match &dt {
            DataType::IntegerVector { dimensions, .. } => match dimensions.as_slice() {
                [] => Some((Vec::new(), Some((0, 0)))),
                [crate::ast::types::PackedDimension::Range { left, right, .. }] => {
                    let l = crate::compiler::elaborate::const_eval_i64_with_params(
                        left,
                        Some(&self.module.parameters),
                    )?;
                    let r = crate::compiler::elaborate::const_eval_i64_with_params(
                        right,
                        Some(&self.module.parameters),
                    )?;
                    Some((Vec::new(), Some((l, r))))
                }
                _ => None,
            },
            _ => Some((Vec::new(), Some((0, 0)))),
        }
    }

    fn csp_scan_item(
        &mut self,
        csp: &Csp,
        it: &ConstraintItem,
        bound: &mut Vec<String>,
        uses: &mut HashMap<usize, (Vec<(u32, u32)>, bool)>,
    ) {
        let mut scan = |me: &mut Self, e: &Expression, bound: &mut Vec<String>| {
            me.csp_scan_expr(csp, e, bound, uses)
        };
        match it {
            ConstraintItem::Expr(e) => scan(self, e, bound),
            ConstraintItem::Inside { expr, range, .. } => {
                scan(self, expr, bound);
                for r in range {
                    match r {
                        ConstraintRange::Value(e) => scan(self, e, bound),
                        ConstraintRange::Range { lo, hi } => {
                            scan(self, lo, bound);
                            scan(self, hi, bound);
                        }
                    }
                }
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                scan(self, condition, bound);
                self.csp_scan_item(csp, constraint, bound, uses);
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                scan(self, condition, bound);
                self.csp_scan_item(csp, then_item, bound, uses);
                if let Some(e) = else_item {
                    self.csp_scan_item(csp, e, bound, uses);
                }
            }
            ConstraintItem::Foreach {
                array, vars, item, ..
            } => {
                // §18.5.8.1: a loop over a fixed-shape state member (a
                // vector used as an iterator) is translated one index at a
                // time; with wide variables, scan it that way too, so a
                // select at a position computed from the index (`r.f[i*18 +
                // 8]`) is a constant select of its own segment.
                if !csp.wide.is_empty()
                    && let [Some(iv)] = vars.as_slice()
                    && let Some(idx) = self.csp_foreach_indices(
                        csp,
                        array,
                        Self::foreach_base_name(array).as_deref(),
                    )
                {
                    for i in idx {
                        CSP_SCAN_BINDS.with(|b| {
                            b.borrow_mut()
                                .push((iv.name.clone(), Self::signed_loop_val(i)))
                        });
                        self.csp_scan_item(csp, item, bound, uses);
                        CSP_SCAN_BINDS.with(|b| b.borrow_mut().pop());
                    }
                    return;
                }
                let mark = bound.len();
                bound.extend(vars.iter().flatten().map(|v| v.name.clone()));
                self.csp_scan_item(csp, item, bound, uses);
                bound.truncate(mark);
            }
            ConstraintItem::Soft(x) => self.csp_scan_item(csp, x, bound, uses),
            ConstraintItem::Block(xs) => {
                for x in xs {
                    self.csp_scan_item(csp, x, bound, uses);
                }
            }
            ConstraintItem::Unique { exprs, .. } => {
                for e in exprs {
                    scan(self, e, bound);
                }
            }
            ConstraintItem::Solve { .. } => {}
        }
    }

    fn csp_scan_expr(
        &mut self,
        csp: &Csp,
        e: &Expression,
        bound: &mut Vec<String>,
        uses: &mut HashMap<usize, (Vec<(u32, u32)>, bool)>,
    ) {
        let bound_now = bound.clone();
        let scan_env = csp_scan_env();
        let mut konst = |me: &mut Self, x: &Expression| -> Option<i64> {
            let mut reads_bound = false;
            Self::walk_operands(x, &mut |y| {
                if let ExprKind::Ident(h) = &y.kind {
                    if h.path.len() == 1 && bound_now.iter().any(|b| *b == h.path[0].name.name) {
                        reads_bound = true;
                    }
                }
            });
            if reads_bound || !me.csp_free(csp, x, &scan_env) {
                return None;
            }
            me.csp_const(x, &scan_env)?.to_i64()
        };
        // §7.4.5: a part of an element at a position not known yet
        // (`f[id][23:16]`): that part of every element.
        if let ExprKind::RangeSelect {
            expr: rb,
            kind: crate::ast::expr::RangeKind::Constant,
            left,
            right,
        } = &Self::unparen(e).kind
            && let Some((b, idx)) = Self::csp_split_last_select(Self::unparen(rb))
            && konst(self, &idx).is_none()
            && let (Some(l), Some(r)) = (konst(self, left), konst(self, right))
            && let Some((v, all)) = self.csp_field_elem_all(csp, &b, &mut konst)
        {
            let (plo, phi) = (l.min(r), l.max(r));
            let u = uses.entry(v).or_default();
            for x in &all {
                if phi < i64::from(x.2) && plo >= 0 {
                    u.0.push((x.1 + plo as u32, (phi - plo + 1) as u32));
                }
            }
        }
        // §7.4.5: an element of a packed-array field at a position not
        // known yet may be any element: cut at every element.
        let mut elem_dyn = false;
        // §7.4.1: every range a select of a multi-dimensional packed
        // variable may name is a select of it.
        if let Some(p) = self.csp_pdim_ref(csp, e, &mut konst) {
            elem_dyn = true;
            if let Some(p) = p
                && p.depth > 0
            {
                uses.entry(p.v).or_default().0.extend(p.pos);
            }
        }
        if !elem_dyn
            && let Some((b, idx)) = Self::csp_split_last_select(Self::unparen(e))
            && konst(self, &idx).is_none()
            && let Some((v, all)) = self.csp_field_elem_all(csp, &b, &mut konst)
        {
            uses.entry(v)
                .or_default()
                .0
                .extend(all.iter().map(|x| (x.1, x.2)));
            elem_dyn = true;
        }
        if !elem_dyn && let Some((v, r)) = self.csp_select_ref(csp, e, &mut konst) {
            let u = uses.entry(v).or_default();
            match r {
                Some(r) => u.0.push(r),
                None => u.1 = true,
            }
        }
        // `$countones(x)` of a whole variable is a count variable (see
        // `Csp::pop`); of a select or a field it counts single bits (see
        // `csp_countones`).
        if let ExprKind::SystemCall { name, args } = &Self::unparen(e).kind
            && name == "$countones"
            && let [x] = args.as_slice()
            && let Some((v, lo, w, whole)) = self.csp_slice_base_k(csp, x, &mut konst)
            && w <= 64
        {
            if whole && lo == 0 && w == csp.vars[v].width && !csp.wide.contains(&v) {
                CSP_SCAN_POPS.with(|p| p.borrow_mut().push(v));
            } else {
                let u = uses.entry(v).or_default();
                u.0.extend((lo..lo + w).map(|b| (b, 1)));
            }
        }
        // `x & MASK`: the runs of ones in the mask are selects of x.
        if let ExprKind::Binary {
            op: BinaryOp::BitAnd,
            left,
            right,
        } = &Self::unparen(e).kind
        {
            for (x, m) in [(left, right), (right, left)] {
                let Some(mask) = konst(self, m) else { continue };
                if let Some((v, lo, w, _)) = self.csp_slice_base_k(csp, x, &mut konst) {
                    let u = uses.entry(v).or_default();
                    for (rl, rw) in Self::mask_runs(mask as u64, w) {
                        u.0.push((lo + rl, rw));
                    }
                }
            }
        }
        // §11.4.10: `x >> K` is the select `x[w-1:K]` (see `csp_shr_slice`).
        if let ExprKind::Binary {
            op: BinaryOp::ShiftRight,
            left,
            right,
        } = &Self::unparen(e).kind
            && let Some(k) = konst(self, right)
            && let Some((v, lo, w, _)) = self.csp_slice_base_k(csp, left, &mut konst)
            && k > 0
            && k < i64::from(w)
        {
            uses.entry(v)
                .or_default()
                .0
                .push((lo + k as u32, w - k as u32));
        }
        if !csp.wide.is_empty() {
            self.csp_wide_scan(csp, e, &bound_now, uses);
        }
        let mut subs: Vec<&Expression> = Vec::new();
        match &e.kind {
            ExprKind::Ident(h) => {
                for seg in &h.path {
                    subs.extend(seg.selects.iter());
                }
            }
            ExprKind::Unary { operand, .. } => subs.push(operand),
            ExprKind::Binary { left, right, .. } => {
                subs.push(left);
                subs.push(right);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => subs.extend([condition.as_ref(), then_expr, else_expr]),
            ExprKind::Concatenation(parts) => subs.extend(parts.iter()),
            ExprKind::Replication { count, exprs } => {
                subs.push(count);
                subs.extend(exprs.iter());
            }
            ExprKind::Call { func, args } => {
                subs.push(func);
                subs.extend(args.iter());
            }
            ExprKind::SystemCall { args, .. } => subs.extend(args.iter()),
            ExprKind::Inside { expr, ranges } => {
                subs.push(expr);
                subs.extend(ranges.iter());
            }
            ExprKind::MemberAccess { expr, .. } | ExprKind::Paren(expr) => subs.push(expr),
            ExprKind::Index { expr, index } => {
                subs.push(expr);
                subs.push(index);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => subs.extend([expr.as_ref(), left, right]),
            ExprKind::Range(a, b) => {
                subs.push(a);
                subs.push(b);
            }
            ExprKind::WithClause { expr, filter } => {
                subs.push(expr);
                subs.push(filter);
            }
            _ => {}
        }
        for x in subs {
            self.csp_scan_expr(csp, x, bound, uses);
        }
    }

    /// The bits of a solver scalar `e` names: Some((base, Some((lsb, w))))
    /// for a constant select or field, Some((base, None)) for a select
    /// whose position is not constant, None when `e` is no select or field
    /// of a scalar (a whole scalar included). Select indices count from the
    /// declared lsb of a whole variable, from 0 within a field.
    fn csp_select_ref(
        &mut self,
        csp: &Csp,
        e: &Expression,
        konst: &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
    ) -> Option<(usize, Option<(u32, u32)>)> {
        let e = Self::unparen(e);
        let pick = |me: &mut Self,
                    base: &Expression,
                    konst: &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
                    lo_hi: &mut dyn FnMut(
            &mut Self,
            &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
        ) -> Option<Option<(i64, i64)>>|
         -> Option<(usize, Option<(u32, u32)>)> {
            let (v, blo, bw, whole) = me.csp_slice_base_k(csp, base, konst)?;
            let origin = if whole {
                csp.lsb.get(&v).copied().unwrap_or(0)
            } else {
                0
            };
            let Some((lo, w)) = lo_hi(me, konst)? else {
                return Some((v, None));
            };
            let lo = if whole && csp.ascending.contains(&v) {
                origin - lo - w + 1
            } else {
                lo - origin
            };
            if lo < 0 || w < 1 || lo + w > bw as i64 {
                return None;
            }
            Some((v, Some((blo + lo as u32, w as u32))))
        };
        if let Some(p) = self.csp_pdim_ref(csp, e, konst) {
            let p = p?;
            if p.depth == 0 {
                return None;
            }
            return Some((p.v, p.known.then(|| p.pos[0])));
        }
        if let Some(r) = self.csp_field_elem(csp, e, konst) {
            return Some(r);
        }
        match &e.kind {
            ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } => pick(self, expr, konst, &mut |me, konst| {
                let (l, r) = (konst(me, left), konst(me, right));
                Some(match kind {
                    crate::ast::expr::RangeKind::Constant => match (l, r) {
                        (Some(l), Some(r)) => {
                            let (v, _, _, whole) = me.csp_slice_base_k(csp, expr, konst)?;
                            let ascending = whole && csp.ascending.contains(&v);
                            if (ascending && l <= r) || (!ascending && l >= r) {
                                Some((l.min(r), (l - r).abs() + 1))
                            } else {
                                return None;
                            }
                        }
                        _ => None,
                    },
                    crate::ast::expr::RangeKind::IndexedUp => {
                        let w = r?;
                        l.map(|b| (b, w))
                    }
                    crate::ast::expr::RangeKind::IndexedDown => {
                        let w = r?;
                        l.map(|b| (b - w + 1, w))
                    }
                })
            }),
            ExprKind::Index { expr, index } => pick(self, expr, konst, &mut |me, konst| {
                Some(konst(me, index).map(|i| (i, 1)))
            }),
            // `x[i]` held as a select on the identifier's last segment.
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.last().is_some_and(|s| s.selects.len() == 1)
                    && h.path[..h.path.len() - 1]
                        .iter()
                        .all(|s| s.selects.is_empty()) =>
            {
                let mut bh = h.clone();
                let idx = bh.path.last_mut().unwrap().selects.pop().unwrap();
                let base = Expression::new(ExprKind::Ident(bh), e.span);
                pick(self, &base, konst, &mut |me, konst| {
                    Some(konst(me, &idx).map(|i| (i, 1)))
                })
            }
            _ => {
                let (v, lo, w, whole) = self.csp_slice_base_k(csp, e, konst)?;
                (!whole).then_some((v, Some((lo, w))))
            }
        }
    }

    /// §7.4.1/§11.5.1: `e` as a select of a multi-dimensional packed
    /// variable (`csp.pdims`): `w`, `w[i]`, `w[i][j]`, `w[i][3:0]`,
    /// `w[5:2]`, `w[i+:2]`. Each index or part-select applies to the next
    /// unpacked-from-the-left dimension, in that dimension's declared
    /// labels; an index not constant here (a `foreach` index while the
    /// constraints are scanned) yields every element it could name. None
    /// when `e` is not rooted at such a variable; Some(None) when it is but
    /// the select is not modelled (out of range, a select after a
    /// part-select, a non-constant part-select width).
    fn csp_pdim_ref(
        &mut self,
        csp: &Csp,
        e: &Expression,
        konst: &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
    ) -> Option<Option<PdimRef>> {
        use crate::ast::expr::RangeKind;
        if csp.pdims.is_empty() {
            return None;
        }
        let e = Self::unparen(e);
        let whole = |v: usize| {
            csp.pdims.contains_key(&v).then(|| PdimRef {
                v,
                pos: vec![(0, csp.vars[v].width)],
                depth: 0,
                known: true,
            })
        };
        if let Some(n) = self.csp_member(csp, e)
            && let Some(&v) = csp.scalars.get(&n)
        {
            return whole(v).map(Some);
        }
        if let Some(v) = self.csp_elem_ref(csp, e, konst) {
            return whole(v).map(Some);
        }
        enum Sel<'a> {
            Index(&'a Expression),
            Part(RangeKind, &'a Expression, &'a Expression),
        }
        let peeled;
        let (base, sel): (&Expression, Sel) = match &e.kind {
            ExprKind::Index { expr, index } => (expr, Sel::Index(index)),
            ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } => (expr, Sel::Part(*kind, left, right)),
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.last().is_some_and(|s| !s.selects.is_empty())
                    && h.path[..h.path.len() - 1]
                        .iter()
                        .all(|s| s.selects.is_empty()) =>
            {
                let mut bh = h.clone();
                let idx = bh.path.last_mut().unwrap().selects.pop().unwrap();
                if matches!(idx.kind, ExprKind::Range(..)) {
                    return None;
                }
                bh.cached_signal_id = Cell::new(None);
                bh.cached_resolved_name = std::cell::OnceCell::new();
                peeled = (Expression::new(ExprKind::Ident(bh), e.span), idx);
                (&peeled.0, Sel::Index(&peeled.1))
            }
            _ => return None,
        };
        let Some(b) = self.csp_pdim_ref(csp, base, konst)? else {
            return Some(None);
        };
        let Some(&(l, r)) = csp.pdims.get(&b.v).and_then(|d| d.get(b.depth)) else {
            return Some(None);
        };
        let n = (l - r).unsigned_abs() + 1;
        let cw = u64::from(b.pos[0].1);
        if n > cw || cw % n != 0 {
            return Some(None);
        }
        let ew = (cw / n) as u32;
        // The offset of label `k` from the least significant element.
        let off = |k: i64| -> Option<u32> {
            let o = if l >= r { k - r } else { r - k };
            (0..n as i64).contains(&o).then_some(o as u32)
        };
        let (lab_lo, lab_hi) = (l.min(r), l.max(r));
        // The element offset ranges (first, count) the select may name.
        let mut runs: Vec<(u32, u32)> = Vec::new();
        let mut known = b.known;
        let depth;
        match sel {
            Sel::Index(ix) => {
                depth = b.depth + 1;
                match konst(self, ix) {
                    Some(k) => match off(k) {
                        Some(o) => runs.push((o, 1)),
                        None => return Some(None),
                    },
                    None => {
                        known = false;
                        runs.extend((0..n as u32).map(|o| (o, 1)));
                    }
                }
            }
            Sel::Part(kind, a, c) => {
                depth = usize::MAX;
                // The labels [first, last] the part-select covers.
                let span = |s: i64, cnt: i64, up: bool| -> (i64, i64) {
                    if up {
                        (s, s + cnt - 1)
                    } else {
                        (s - cnt + 1, s)
                    }
                };
                let mut labels: Vec<(i64, i64)> = Vec::new();
                match kind {
                    RangeKind::Constant => {
                        let (Some(x), Some(y)) = (konst(self, a), konst(self, c)) else {
                            return Some(None);
                        };
                        // §11.5.1: a part-select keeps the declared direction.
                        if x != y && (x > y) != (l > r) {
                            return Some(None);
                        }
                        labels.push((x.min(y), x.max(y)));
                    }
                    RangeKind::IndexedUp | RangeKind::IndexedDown => {
                        let up = kind == RangeKind::IndexedUp;
                        let Some(cnt) = konst(self, c).filter(|&c| c >= 1) else {
                            return Some(None);
                        };
                        match konst(self, a) {
                            Some(s0) => labels.push(span(s0, cnt, up)),
                            None => {
                                known = false;
                                for s0 in lab_lo..=lab_hi {
                                    let (x, y) = span(s0, cnt, up);
                                    if x >= lab_lo && y <= lab_hi {
                                        labels.push((x, y));
                                    }
                                }
                            }
                        }
                    }
                }
                for (x, y) in labels {
                    let (Some(ox), Some(oy)) = (off(x), off(y)) else {
                        return Some(None);
                    };
                    runs.push((ox.min(oy), ox.max(oy) - ox.min(oy) + 1));
                }
                if runs.is_empty() {
                    return Some(None);
                }
            }
        }
        if b.pos.len() * runs.len() > 4096 {
            return Some(None);
        }
        let pos = b
            .pos
            .iter()
            .flat_map(|&(lo, _)| runs.iter().map(move |&(o, k)| (lo + o * ew, k * ew)))
            .collect();
        Some(Some(PdimRef {
            v: b.v,
            pos,
            depth,
            known,
        }))
    }

    /// `e` as (base, lsb, width, whole): a whole scalar, a field, or a select
    /// of either.
    fn csp_slice_base_k(
        &mut self,
        csp: &Csp,
        e: &Expression,
        konst: &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
    ) -> Option<(usize, u32, u32, bool)> {
        let e = Self::unparen(e);
        let var_of = |me: &Self, n: &str| -> Option<usize> {
            let &v = csp.scalars.get(n)?;
            (!me.csp_var_no_slices(csp, v)).then_some(v)
        };
        let field = |csp: &Csp, v: usize, f: &str| -> Option<(u32, u32)> {
            csp.fields
                .get(&v)?
                .iter()
                .find(|(n, ..)| n == f)
                .map(|&(_, lo, w)| (lo, w))
        };
        if let Some(p) = self.csp_pdim_ref(csp, e, konst) {
            let p = p.filter(|p| p.known)?;
            let (lo, w) = p.pos[0];
            return Some((p.v, lo, w, p.depth == 0));
        }
        // `x`, `this.x`, `recv.x`: the scalar itself.
        if let Some(n) = self.csp_member(csp, e) {
            if let Some(v) = var_of(self, &n) {
                return Some((v, 0, csp.vars[v].width, true));
            }
        }
        // `a[k]`: an element of a solver array, as a whole.
        if let Some(v) = self.csp_elem_ref(csp, e, konst) {
            return Some((v, 0, csp.vars[v].width, true));
        }
        // `f[k]`: an element of a packed-array field.
        if let Some((v, Some((lo, w)))) = self.csp_field_elem(csp, e, konst) {
            return Some((v, lo, w, false));
        }
        match &e.kind {
            ExprKind::Ident(h)
                if h.root.is_none() && h.path.iter().all(|s| s.selects.is_empty()) =>
            {
                let names: Vec<&str> = h.path.iter().map(|s| s.name.name.as_str()).collect();
                // object-first binding: `p.f` is field f of the scalar p;
                // `this.p.f` / `recv.p.f` likewise.
                let start = if names.len() >= 2
                    && (names[0] == "this" || self.rand_receiver.as_deref() == Some(names[0]))
                    && !csp.scalars.contains_key(names[0])
                {
                    1
                } else {
                    0
                };
                // A rand sub-object's member `a.x` is one variable.
                let (v, start) = match var_of(self, names[start]) {
                    Some(v) => (v, start + 1),
                    None if csp.has_subs && names.len() >= start + 2 => (
                        var_of(self, &format!("{}.{}", names[start], names[start + 1]))?,
                        start + 2,
                    ),
                    None => return None,
                };
                if names.len() == start {
                    return Some((v, 0, csp.vars[v].width, true));
                }
                // A nested field (`s.a.b`) is listed by its dotted path
                // (wide and sub-object variables, `csp_nested_fields`).
                let (lo, w) = field(csp, v, &names[start..].join("."))?;
                Some((v, lo, w, false))
            }
            ExprKind::MemberAccess { expr, member } => {
                let (v, lo, _, whole) = self.csp_slice_base_k(csp, expr, konst)?;
                if !whole {
                    let mut path = Self::csp_member_path(e)?;
                    if path[0] == "this" || self.rand_receiver.as_deref() == Some(path[0].as_str())
                    {
                        path.remove(0);
                    }
                    let start = if csp.scalars.get(&path[0]) == Some(&v) {
                        1
                    } else if path.len() >= 2
                        && csp.scalars.get(&format!("{}.{}", path[0], path[1])) == Some(&v)
                    {
                        2
                    } else {
                        return None;
                    };
                    if path.len() < start + 2 {
                        return None;
                    }
                    let (flo, fw) = field(csp, v, &path[start..].join("."))?;
                    return Some((v, flo, fw, false));
                }
                let (flo, fw) = field(csp, v, &member.name)?;
                Some((v, lo + flo, fw, false))
            }
            _ => None,
        }
    }

    /// `e` as an element select of a packed-array field (`f[k]`, held as
    /// an index or as the select of the identifier's last segment): its base
    /// expression and index. None for anything else.
    fn csp_split_last_select(e: &Expression) -> Option<(Expression, Expression)> {
        match &e.kind {
            ExprKind::Index { expr, index } => Some(((**expr).clone(), (**index).clone())),
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.len() >= 2
                    && h.path.last().is_some_and(|s| s.selects.len() == 1)
                    && h.path[..h.path.len() - 1]
                        .iter()
                        .all(|s| s.selects.is_empty()) =>
            {
                let mut bh = h.clone();
                let idx = bh.path.last_mut().unwrap().selects.pop().unwrap();
                bh.cached_signal_id = Cell::new(None);
                bh.cached_resolved_name = std::cell::OnceCell::new();
                Some((Expression::new(ExprKind::Ident(bh), e.span), idx))
            }
            _ => None,
        }
    }

    /// §7.4.5: `f[k]` with `f` a field that is a packed array of elements
    /// (`csp.field_elems`): Some((base, Some((lsb, width)))) for a constant
    /// `k` naming an element, Some((base, None)) for a position not known
    /// yet, None when `e` is no such select.
    fn csp_field_elem(
        &mut self,
        csp: &Csp,
        e: &Expression,
        konst: &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
    ) -> Option<(usize, Option<(u32, u32)>)> {
        if csp.field_elems.is_empty() {
            return None;
        }
        let (b, idx) = Self::csp_split_last_select(Self::unparen(e))?;
        let (v, lo, w, whole) = self.csp_slice_base_k(csp, &b, konst)?;
        if whole {
            return None;
        }
        let &(l, r, ew) = csp.field_elems.get(&(v, lo, w))?;
        let Some(k) = konst(self, &idx) else {
            return Some((v, None));
        };
        let off = if l >= r {
            (r..=l).contains(&k).then(|| k - r)
        } else {
            (l..=r).contains(&k).then(|| r - k)
        };
        Some((v, off.map(|o| (lo + o as u32 * ew, ew))))
    }

    /// The element positions and bit ranges of the packed-array field `b`
    /// names, if it is one: (base, [(index, lsb, width)]).
    #[allow(clippy::type_complexity)]
    fn csp_field_elem_all(
        &mut self,
        csp: &Csp,
        b: &Expression,
        konst: &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
    ) -> Option<(usize, Vec<(i64, u32, u32)>)> {
        if csp.field_elems.is_empty() {
            return None;
        }
        let (v, lo, w, whole) = self.csp_slice_base_k(csp, b, konst)?;
        if whole {
            return None;
        }
        let &(l, r, ew) = csp.field_elems.get(&(v, lo, w))?;
        let ks: Vec<i64> = if l >= r {
            (r..=l).collect()
        } else {
            (l..=r).collect()
        };
        Some((
            v,
            ks.into_iter()
                .map(|k| {
                    let o = if l >= r { k - r } else { r - k };
                    (k, lo + o as u32 * ew, ew)
                })
                .collect(),
        ))
    }

    /// The element variable `e` names: `a[k]` with `a` a solver array and
    /// `k` constant (held as an index, or as the select of the identifier's
    /// last segment when another select follows: `a[k][7:0]`). While the
    /// constraints are scanned an index that is not constant stands for the
    /// first element (see `CSP_SCAN_ANY`).
    fn csp_elem_ref(
        &mut self,
        csp: &Csp,
        e: &Expression,
        konst: &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
    ) -> Option<usize> {
        if csp.arrays.is_empty() {
            return None;
        }
        let mut elem = |me: &mut Self, base: &Expression, idx: &Expression| -> Option<usize> {
            let n = me
                .csp_member(csp, base)
                .filter(|n| csp.arrays.contains_key(n))
                .or_else(|| me.csp_nd_row(csp, base, konst))?;
            let a = csp.arrays.get(&n)?;
            let v = match konst(me, idx) {
                Some(i) => a.elems.iter().find(|x| x.0 == i)?.1,
                None if CSP_SCAN_ANY.with(|c| c.get()) => a.elems.first()?.1,
                None => return None,
            };
            (!me.csp_var_no_slices(csp, v)).then_some(v)
        };
        match &e.kind {
            ExprKind::Index { expr, index } => elem(self, expr, index),
            // One select names an element of a 1-D array; several name an
            // element of a multi-dimensional one through its row (#519).
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.last().is_some_and(|s| !s.selects.is_empty())
                    && h.path[..h.path.len() - 1]
                        .iter()
                        .all(|s| s.selects.is_empty()) =>
            {
                let mut bh = h.clone();
                let idx = bh.path.last_mut().unwrap().selects.pop().unwrap();
                let base = Expression::new(ExprKind::Ident(bh), e.span);
                elem(self, &base, &idx)
            }
            _ => None,
        }
    }

    /// §18.4 (#519): the row of a multi-dimensional solver array that `e`
    /// names with constant indices (`a[i]` under a bound `i`, `a[i][j]`
    /// for a 3-D array): the base name followed by the indices, when a
    /// solver array of that name exists.
    /// The row of a multi-dimensional solver array an index chain with
    /// constant (or loop-bound) indices names (`a[1]`, `a[i]`).
    fn csp_row_of(&mut self, csp: &Csp, e: &Expression, env: &Env) -> Option<String> {
        if !matches!(e.kind, ExprKind::Index { .. }) {
            return None;
        }
        let mut konst = |me: &mut Self, x: &Expression| -> Option<i64> {
            if !me.csp_free(csp, x, env) {
                return None;
            }
            me.csp_const(x, env)?.to_i64()
        };
        self.csp_nd_row(csp, e, &mut konst)
    }

    fn csp_nd_row(
        &mut self,
        csp: &Csp,
        e: &Expression,
        konst: &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
    ) -> Option<String> {
        if csp.nd_dims.is_empty() {
            return None;
        }
        let (base, consts) = self.csp_nd_prefix_k(csp, e, konst)?;
        if consts.is_empty() {
            return None;
        }
        let mut row = base;
        for c in consts {
            row.push_str(&format!("[{c}]"));
        }
        csp.arrays.contains_key(&row).then_some(row)
    }

    /// The multi-dimensional base `e` indexes and the constant indices
    /// applied to it, outermost first (`a[1][0]` → (`a`, [1, 0])).
    fn csp_nd_prefix_k(
        &mut self,
        csp: &Csp,
        e: &Expression,
        konst: &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>,
    ) -> Option<(String, Vec<i64>)> {
        match &e.kind {
            ExprKind::Index { expr, index } => {
                let (b, mut cs) = self.csp_nd_prefix_k(csp, expr, konst)?;
                cs.push(konst(self, index)?);
                Some((b, cs))
            }
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.last().is_some_and(|s| !s.selects.is_empty())
                    && h.path[..h.path.len() - 1]
                        .iter()
                        .all(|s| s.selects.is_empty()) =>
            {
                let mut bh = h.clone();
                let sels = std::mem::take(&mut bh.path.last_mut().unwrap().selects);
                let base = Expression::new(ExprKind::Ident(bh), e.span);
                let (b, mut cs) = self.csp_nd_prefix_k(csp, &base, konst)?;
                for s in &sels {
                    cs.push(konst(self, s)?);
                }
                Some((b, cs))
            }
            _ => {
                let n = self.csp_member(csp, e)?;
                csp.nd_dims.contains_key(&n).then_some((n, Vec::new()))
            }
        }
    }

    /// The element variable a chain of constant indexes on a
    /// multi-dimensional solver array names (`a[1][0]`, `a[i][j]` under
    /// bound loop variables), without evaluating anything.
    fn nd_chain_elem(&self, csp: &Csp, e: &Expression, env: &Env) -> Option<usize> {
        let mut consts: Vec<i64> = Vec::new();
        let mut cur = e;
        loop {
            match &cur.kind {
                ExprKind::Index { expr, index } => {
                    consts.push(Self::nd_const_index(index, env)?);
                    cur = expr;
                }
                ExprKind::Paren(i) => cur = i,
                _ => break,
            }
        }
        consts.reverse();
        let base = match &cur.kind {
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.last().is_some_and(|s| !s.selects.is_empty())
                    && h.path[..h.path.len() - 1]
                        .iter()
                        .all(|s| s.selects.is_empty()) =>
            {
                let mut bh = h.clone();
                let sels = std::mem::take(&mut bh.path.last_mut().unwrap().selects);
                let mut lead: Vec<i64> = Vec::new();
                for s in &sels {
                    lead.push(Self::nd_const_index(s, env)?);
                }
                lead.extend(consts);
                consts = lead;
                self.csp_member(csp, &Expression::new(ExprKind::Ident(bh), cur.span))?
            }
            _ => self.csp_member(csp, cur)?,
        };
        if !csp.nd_dims.contains_key(&base) || consts.is_empty() {
            return None;
        }
        let mut row = base;
        for c in &consts[..consts.len() - 1] {
            row.push_str(&format!("[{c}]"));
        }
        csp.arrays
            .get(&row)?
            .elems
            .iter()
            .find(|x| x.0 == consts[consts.len() - 1])
            .map(|x| x.1)
    }

    /// A constant index of a multi-dimensional element reference: a
    /// literal, or a loop variable the environment binds.
    fn nd_const_index(x: &Expression, env: &Env) -> Option<i64> {
        match &x.kind {
            ExprKind::Paren(i) => Self::nd_const_index(i, env),
            ExprKind::Number(NumberLiteral::Integer { .. }) => {
                let v = crate::compiler::elaborate::const_eval_i64_with_params(x, None)?;
                Some(v)
            }
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => env
                .binds
                .iter()
                .rev()
                .find(|b| b.0 == h.path[0].name.name)
                .and_then(|b| b.1.to_i64()),
            _ => None,
        }
    }

    /// A scalar that is not split (no select arithmetic for its shape).
    fn csp_var_no_slices(&self, csp: &Csp, v: usize) -> bool {
        csp.lsb.get(&v) == Some(&i64::MIN)
    }

    /// The structural identities: each split scalar equals the sum of its
    /// segments, each multi-segment select the sum of the segments it covers.
    fn csp_slice_links(&mut self, csp: &mut Csp) {
        let mut links: Vec<Lin> = Vec::new();
        for (&v, segs) in &csp.segs {
            if csp.wide.contains(&v) {
                // No value of its own: it is its segments.
                continue;
            }
            let mut t: Vec<(usize, i128)> = vec![(v, 1)];
            for &(lo, _, x) in segs {
                t.push((x, -(1i128 << lo)));
            }
            links.push(Lin { t, k: 0 });
        }
        for (&(v, lo, w), &x) in &csp.slice_vars {
            let Some(segs) = csp.segs.get(&v) else {
                continue;
            };
            let mut t: Vec<(usize, i128)> = vec![(x, 1)];
            for &(slo, sw, sx) in segs {
                if slo >= lo && slo + sw <= lo + w {
                    t.push((sx, -(1i128 << (slo - lo))));
                }
            }
            links.push(Lin { t, k: 0 });
        }
        let mut pops: Vec<(usize, usize)> = csp.pop.iter().map(|(&x, &c)| (x, c)).collect();
        pops.sort_unstable();
        for (x, c) in pops {
            let w = csp.vars[x].width;
            csp.nodes.push(Node::Pop { x, c, w });
        }
        for mut lin in links {
            lin.t.sort_unstable_by_key(|x| x.0);
            let deps: Vec<usize> = lin.t.iter().map(|x| x.0).collect();
            let one = Self::literal_of(&Value::from_u64(1, 1), crate::ast::Span::dummy());
            let src = self.csp_src(csp, SrcItem::Expr(one), &Env::default(), deps);
            csp.nodes.push(Node::Lin {
                lin,
                rel: Rel::Eq,
                fits: Vec::new(),
                src,
                sneg: false,
                wrap: None,
            });
        }
    }

    /// Maximal runs of ones in the low `w` bits of `m`: (lsb, width).
    fn mask_runs(m: u64, w: u32) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        let mut i = 0u32;
        while i < w.min(64) {
            if (m >> i) & 1 == 1 {
                let start = i;
                while i < w.min(64) && (m >> i) & 1 == 1 {
                    i += 1;
                }
                out.push((start, i - start));
            } else {
                i += 1;
            }
        }
        out
    }

    /// `(x & M) == C` with constant `M`, `C` and `x` a scalar, field or
    /// select: each run of ones in `M` pins those bits of `x` to `C`'s, and a
    /// `C` bit outside `M` makes it unsatisfiable. None for another shape.
    fn csp_masked_eq(
        &mut self,
        csp: &mut Csp,
        whole: &Expression,
        masked: &Expression,
        other: &Expression,
        env: &Env,
    ) -> Option<Node> {
        let ExprKind::Binary {
            op: BinaryOp::BitAnd,
            left,
            right,
        } = &Self::unparen(masked).kind
        else {
            return None;
        };
        if !self.csp_free(csp, other, env) {
            return None;
        }
        let c = self.csp_const(other, env)?.to_u64()?;
        for (x, m) in [(left, right), (right, left)] {
            if !self.csp_free(csp, m, env) {
                continue;
            }
            let Some(mask) = self.csp_const(m, env).and_then(|v| v.to_u64()) else {
                continue;
            };
            let mut konst = |me: &mut Self, y: &Expression| -> Option<i64> {
                if !me.csp_free(csp, y, env) {
                    return None;
                }
                me.csp_const(y, env)?.to_i64()
            };
            let Some((v, lo, w, _)) = self.csp_slice_base_k(csp, x, &mut konst) else {
                continue;
            };
            let wm = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
            let mask = mask & wm;
            if c & !mask != 0 {
                return Some(Node::False);
            }
            let mut nodes = Vec::new();
            for (rl, rw) in Self::mask_runs(mask, w) {
                let (blo, bw) = (lo + rl, rw);
                let var = if blo == 0 && bw == csp.vars[v].width {
                    Some(v)
                } else {
                    csp.segs.get(&v).and_then(|segs| {
                        segs.iter()
                            .find(|s| s.0 == blo && s.1 == bw)
                            .map(|s| s.2)
                            .or_else(|| csp.slice_vars.get(&(v, blo, bw)).copied())
                    })
                };
                let var = var?;
                let want = ((c >> rl) & if rw >= 64 { u64::MAX } else { (1u64 << rw) - 1 }) as i128;
                let src = self.csp_src(csp, SrcItem::Expr(whole.clone()), env, vec![var]);
                nodes.push(Node::Lin {
                    lin: Lin {
                        t: vec![(var, 1)],
                        k: -want,
                    },
                    rel: Rel::Eq,
                    fits: Vec::new(),
                    src,
                    sneg: false,
                    wrap: None,
                });
            }
            return Some(Self::csp_and(nodes));
        }
        None
    }

    /// A select or field of a split scalar as its solver variable.
    fn csp_slice_ae(&mut self, csp: &Csp, e: &Expression, env: &Env) -> Option<Ae> {
        let mut konst = |me: &mut Self, x: &Expression| -> Option<i64> {
            if !me.csp_free(csp, x, env) {
                return None;
            }
            me.csp_const(x, env)?.to_i64()
        };
        let (v, r) = self.csp_select_ref(csp, e, &mut konst)?;
        let (lo, w) = r?;
        if lo == 0 && w == csp.vars[v].width {
            // `v[31:8]` of `logic [31:8] v`: the whole variable.
            return Some(Self::csp_var_ae(csp, v));
        }
        let segs = csp.segs.get(&v)?;
        if let Some(&(_, _, x)) = segs.iter().find(|s| s.0 == lo && s.1 == w) {
            return Some(Ae::Var(x, w, false));
        }
        let &x = csp.slice_vars.get(&(v, lo, w))?;
        Some(Ae::Var(x, w, false))
    }
}

/// Two-state arithmetic modulo 2^w on little-endian 64-bit words (the
/// constants of a wide relation, see `csp_wide_affine`).
fn ww_mask(a: &mut [u64], w: u32) {
    let r = w % 64;
    if r != 0
        && let Some(top) = a.get_mut(((w - 1) / 64) as usize)
    {
        *top &= (1u64 << r) - 1;
    }
}

fn ww_add(a: &[u64], b: &[u64], w: u32) -> Vec<u64> {
    let mut out = vec![0u64; a.len()];
    let mut carry = 0u128;
    for (i, o) in out.iter_mut().enumerate() {
        let t = u128::from(a[i]) + u128::from(b.get(i).copied().unwrap_or(0)) + carry;
        *o = t as u64;
        carry = t >> 64;
    }
    ww_mask(&mut out, w);
    out
}

fn ww_neg(a: &[u64], w: u32) -> Vec<u64> {
    let inv: Vec<u64> = a.iter().map(|x| !x).collect();
    let mut one = vec![0u64; a.len()];
    if let Some(f) = one.first_mut() {
        *f = 1;
    }
    ww_add(&inv, &one, w)
}

fn ww_sub(a: &[u64], b: &[u64], w: u32) -> Vec<u64> {
    ww_add(a, &ww_neg(b, w), w)
}

fn ww_cmp(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    for i in (0..a.len()).rev() {
        match a[i].cmp(&b[i]) {
            std::cmp::Ordering::Equal => {}
            o => return o,
        }
    }
    std::cmp::Ordering::Equal
}

/// A `w`-bit operand of a relation on wide variables: constant bits with
/// solver variables laid over some ranges (each read whole, unsigned and at
/// most 64 bits wide: a segment of a wide variable or a narrow variable).
#[derive(Clone, Debug)]
struct WOp {
    w: u32,
    /// Constant bits, little-endian words; zero under the variables.
    bits: Vec<u64>,
    /// (lsb, width, variable), disjoint.
    vars: Vec<(u32, u32, usize)>,
}

impl WOp {
    fn zero(w: u32) -> WOp {
        WOp {
            w,
            bits: vec![0; w.div_ceil(64) as usize],
            vars: Vec::new(),
        }
    }
    fn bit(&self, i: u32) -> bool {
        i < self.w && (self.bits[(i / 64) as usize] >> (i % 64)) & 1 == 1
    }
    fn set(&mut self, i: u32, b: bool) {
        if i < self.w {
            let m = 1u64 << (i % 64);
            if b {
                self.bits[(i / 64) as usize] |= m;
            } else {
                self.bits[(i / 64) as usize] &= !m;
            }
        }
    }
    /// §11.8.2: a constant extended to the context width, by sign when
    /// the context and the constant are both signed.
    fn konst(w: u32, v: &Value, sext: bool) -> WOp {
        let mut o = WOp::zero(w);
        let vw = v.width.max(1);
        let sign = sext && v.get_bit(vw as usize - 1) == LogicBit::One;
        for i in 0..w {
            let b = if i < vw {
                v.get_bit(i as usize) == LogicBit::One
            } else {
                sign
            };
            o.set(i, b);
        }
        o
    }
    /// The constant bits `[a, b)` (at most 64) as an integer.
    fn word(&self, a: u32, b: u32) -> u64 {
        let mut x = 0u64;
        for i in a..b {
            if self.bit(i) {
                x |= 1u64 << (i - a);
            }
        }
        x
    }
    /// `>> k` (logical): None when a variable would be cut.
    fn shr(&self, k: u64) -> Option<WOp> {
        let mut o = WOp::zero(self.w);
        if k >= u64::from(self.w) {
            return Some(o);
        }
        let k = k as u32;
        for i in 0..self.w - k {
            o.set(i, self.bit(i + k));
        }
        for &(lo, w, v) in &self.vars {
            if lo >= k {
                o.vars.push((lo - k, w, v));
            } else if lo + w > k {
                return None;
            }
        }
        Some(o)
    }
    /// `<< k`: None when a variable would be cut.
    fn shl(&self, k: u64) -> Option<WOp> {
        let mut o = WOp::zero(self.w);
        if k >= u64::from(self.w) {
            return Some(o);
        }
        let k = k as u32;
        for i in k..self.w {
            o.set(i, self.bit(i - k));
        }
        for &(lo, w, v) in &self.vars {
            if lo + w + k <= self.w {
                o.vars.push((lo + k, w, v));
            } else if lo + k < self.w {
                return None;
            }
        }
        Some(o)
    }
    /// `& m` (`or` false) or `| m` (`or` true) with a constant `m`: each
    /// variable must lie wholly under ones or wholly under zeros of `m`.
    fn mask(&self, m: &WOp, or: bool) -> Option<WOp> {
        let mut o = self.clone();
        for (a, b) in o.bits.iter_mut().zip(&m.bits) {
            if or {
                *a |= b;
            } else {
                *a &= b;
            }
        }
        o.vars.clear();
        for &(lo, w, v) in &self.vars {
            let ones = (lo..lo + w).filter(|&i| m.bit(i)).count() as u32;
            // `&`: kept under ones; `|`: kept under zeros (else the bits
            // are the constant ones already in `bits`).
            let keep = if or { ones == 0 } else { ones == w };
            let drop = if or { ones == w } else { ones == 0 };
            if keep {
                o.vars.push((lo, w, v));
            } else if !drop {
                return None;
            }
        }
        Some(o)
    }
}

/// §11.4.4/§11.4.5/§11.5.1: relations on rand variables wider than 64 bits.
/// The solver's domains are 64-bit intervals, so a wide variable is held as
/// segments of at most 64 bits, split at every multiple of 64, at the sign
/// bit of a signed variable and at every boundary its constraints use (part
/// and bit selects, struct fields, shift amounts, mask runs). A relation
/// becomes per-chunk relations on those segments: `==` an equality per
/// chunk, `!=` a disjunction, and `<`, `<=`, `>`, `>=` the lexicographic
/// order of the chunks, most significant first (signed: the sign bit
/// compares the other way). Anything else on a wide variable is not
/// modelled and leaves the class to the trials.
impl Simulator {
    /// The wide rand members among `names` that no constraint reads, or
    /// reads only through `size()` / `$size` (§7.5.1, §7.10.1). Empty when
    /// a constraint calls a user method, which may read any member.
    fn csp_wide_unread(constraints: &[ClassConstraint], names: &[&str]) -> HashSet<String> {
        let mut read: HashSet<String> = HashSet::default();
        let mut opaque = false;
        for c in constraints {
            for it in &c.items {
                Self::csp_wide_item_reads(it, &mut read, &mut opaque);
            }
        }
        if opaque {
            return HashSet::default();
        }
        // A member of an unpacked struct (`s.x`) is read through its leaf.
        names
            .iter()
            .filter(|n| {
                !read.contains(**n) && !n.rsplit_once('.').is_some_and(|p| read.contains(p.1))
            })
            .map(|n| n.to_string())
            .collect()
    }

    fn csp_wide_item_reads(it: &ConstraintItem, read: &mut HashSet<String>, opaque: &mut bool) {
        let ex = |e: &Expression, read: &mut HashSet<String>, opaque: &mut bool| {
            Self::csp_wide_expr_reads(e, read, opaque)
        };
        match it {
            ConstraintItem::Expr(e) => ex(e, read, opaque),
            ConstraintItem::Inside {
                expr,
                range,
                dist_weights,
                ..
            } => {
                ex(expr, read, opaque);
                for r in range {
                    match r {
                        ConstraintRange::Value(v) => ex(v, read, opaque),
                        ConstraintRange::Range { lo, hi } => {
                            ex(lo, read, opaque);
                            ex(hi, read, opaque);
                        }
                    }
                }
                for w in dist_weights.iter().flatten() {
                    match w {
                        DistWeight::Each(e) | DistWeight::Total(e) => ex(e, read, opaque),
                    }
                }
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                ex(condition, read, opaque);
                Self::csp_wide_item_reads(constraint, read, opaque);
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                ex(condition, read, opaque);
                Self::csp_wide_item_reads(then_item, read, opaque);
                if let Some(e) = else_item {
                    Self::csp_wide_item_reads(e, read, opaque);
                }
            }
            ConstraintItem::Foreach { array, item, .. } => {
                ex(array, read, opaque);
                Self::csp_wide_item_reads(item, read, opaque);
            }
            ConstraintItem::Solve { before, after, .. } => {
                read.extend(before.iter().chain(after).map(|i| i.name.clone()));
            }
            ConstraintItem::Soft(i) => Self::csp_wide_item_reads(i, read, opaque),
            ConstraintItem::Block(items) => {
                for i in items {
                    Self::csp_wide_item_reads(i, read, opaque);
                }
            }
            ConstraintItem::Unique { exprs, .. } => {
                for e in exprs {
                    ex(e, read, opaque);
                }
            }
        }
    }

    /// Every name `e` mentions (conservatively: each segment of a path).
    fn csp_wide_expr_reads(e: &Expression, read: &mut HashSet<String>, opaque: &mut bool) {
        const ARRAY_METHODS: &[&str] = &[
            "sum",
            "product",
            "and",
            "or",
            "xor",
            "min",
            "max",
            "unique",
            "unique_index",
            "find",
            "find_index",
            "find_first",
            "find_first_index",
            "find_last",
            "find_last_index",
            "exists",
            "num",
        ];
        // A plain member path: its `size()` reads no element.
        let plain = |x: &Expression| match &x.kind {
            ExprKind::Ident(h) => h.path.iter().all(|s| s.selects.is_empty()),
            ExprKind::MemberAccess { expr, .. } => {
                matches!(expr.kind, ExprKind::This | ExprKind::Ident(_))
            }
            _ => false,
        };
        match &e.kind {
            ExprKind::Number(_)
            | ExprKind::StringLiteral(_)
            | ExprKind::TypeLiteral(_)
            | ExprKind::Dollar
            | ExprKind::Null
            | ExprKind::This
            | ExprKind::Empty => {}
            ExprKind::Ident(h) => {
                let mut sel = Vec::new();
                for s in &h.path {
                    read.insert(s.name.name.clone());
                    sel.extend(s.selects.iter());
                }
                for x in sel {
                    Self::csp_wide_expr_reads(x, read, opaque);
                }
            }
            ExprKind::MemberAccess { expr, member } => {
                read.insert(member.name.clone());
                Self::csp_wide_expr_reads(expr, read, opaque);
            }
            ExprKind::Call { func, args } => {
                match &func.kind {
                    ExprKind::MemberAccess { expr, member } => {
                        if member.name == "size" && args.is_empty() && plain(expr) {
                            return;
                        }
                        if !ARRAY_METHODS.contains(&member.name.as_str()) {
                            *opaque = true;
                            return;
                        }
                        Self::csp_wide_expr_reads(expr, read, opaque);
                    }
                    ExprKind::Ident(h)
                        if h.path.len() >= 2 && h.path.iter().all(|s| s.selects.is_empty()) =>
                    {
                        let m = h.path.last().unwrap().name.name.as_str();
                        if m == "size" && args.is_empty() {
                            return;
                        }
                        if !ARRAY_METHODS.contains(&m) {
                            *opaque = true;
                            return;
                        }
                        for s in &h.path[..h.path.len() - 1] {
                            read.insert(s.name.name.clone());
                        }
                    }
                    _ => {
                        *opaque = true;
                        return;
                    }
                }
                for a in args {
                    Self::csp_wide_expr_reads(a, read, opaque);
                }
            }
            ExprKind::SystemCall { name, args } => {
                if name == "$size" && args.len() == 1 && plain(&args[0]) {
                    return;
                }
                for a in args {
                    Self::csp_wide_expr_reads(a, read, opaque);
                }
            }
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                Self::csp_wide_expr_reads(operand, read, opaque)
            }
            ExprKind::Binary { left, right, .. } | ExprKind::Range(left, right) => {
                Self::csp_wide_expr_reads(left, read, opaque);
                Self::csp_wide_expr_reads(right, read, opaque);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::csp_wide_expr_reads(condition, read, opaque);
                Self::csp_wide_expr_reads(then_expr, read, opaque);
                Self::csp_wide_expr_reads(else_expr, read, opaque);
            }
            ExprKind::Concatenation(xs) => xs
                .iter()
                .for_each(|x| Self::csp_wide_expr_reads(x, read, opaque)),
            ExprKind::Replication { count, exprs } => {
                Self::csp_wide_expr_reads(count, read, opaque);
                exprs
                    .iter()
                    .for_each(|x| Self::csp_wide_expr_reads(x, read, opaque));
            }
            ExprKind::Inside { expr, ranges } => {
                Self::csp_wide_expr_reads(expr, read, opaque);
                ranges
                    .iter()
                    .for_each(|x| Self::csp_wide_expr_reads(x, read, opaque));
            }
            ExprKind::Index { expr, index } => {
                Self::csp_wide_expr_reads(expr, read, opaque);
                Self::csp_wide_expr_reads(index, read, opaque);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                Self::csp_wide_expr_reads(expr, read, opaque);
                Self::csp_wide_expr_reads(left, read, opaque);
                Self::csp_wide_expr_reads(right, read, opaque);
            }
            ExprKind::WithClause { expr, filter } => {
                Self::csp_wide_expr_reads(expr, read, opaque);
                Self::csp_wide_expr_reads(filter, read, opaque);
            }
            _ => *opaque = true,
        }
    }

    /// Split every wide variable into segments: at each multiple of 64,
    /// the sign bit, and the boundaries its constraints use (`uses`).
    fn csp_wide_split(
        &mut self,
        csp: &mut Csp,
        uses: &HashMap<usize, (Vec<(u32, u32)>, bool)>,
        links: &[(usize, u32, usize, u32, u32)],
    ) {
        let mut wides: Vec<usize> = csp.wide.iter().copied().collect();
        wides.sort_unstable();
        let mut cuts_of: HashMap<usize, std::collections::BTreeSet<u32>> = HashMap::default();
        for &v in &wides {
            let w = csp.vars[v].width;
            let mut cuts: std::collections::BTreeSet<u32> = [0, w].into_iter().collect();
            if csp.vars[v].signed {
                cuts.insert(w - 1);
            }
            if let Some((ranges, _)) = uses.get(&v) {
                for &(lo, rw) in ranges {
                    cuts.extend([lo, lo + rw].into_iter().filter(|&c| c <= w));
                }
            }
            cuts_of.insert(v, cuts);
        }
        // Two wide ranges a relation compares (`b.w == a.w`), and the
        // elements of one wide array, are cut alike: the relation is then a
        // relation per segment (`csp_wide_chunks`).
        let mut pairs: Vec<(usize, u32, usize, u32, u32)> = links
            .iter()
            .copied()
            .filter(|l| cuts_of.contains_key(&l.0) && cuts_of.contains_key(&l.2))
            .collect();
        for a in csp.arrays.values() {
            if let Some(&(_, e0)) = a.elems.first()
                && cuts_of.contains_key(&e0)
            {
                let w = csp.vars[e0].width;
                pairs.extend(a.elems.iter().skip(1).map(|e| (e0, 0, e.1, 0, w)));
            }
        }
        let align = |cuts_of: &mut HashMap<usize, std::collections::BTreeSet<u32>>| {
            for _ in 0..8 {
                let mut changed = false;
                for &(a, la, b, lb, w) in &pairs {
                    for (x, lx, y, ly) in [(a, la, b, lb), (b, lb, a, la)] {
                        let add: Vec<u32> = cuts_of[&x]
                            .range(lx + 1..lx + w)
                            .map(|&c| c - lx + ly)
                            .collect();
                        let cy = cuts_of.get_mut(&y).unwrap();
                        for c in add {
                            changed |= cy.insert(c);
                        }
                    }
                }
                if !changed {
                    break;
                }
            }
        };
        if !pairs.is_empty() {
            align(&mut cuts_of);
        }
        // A stretch no constraint divides is cut at the multiples of 64
        // inside it, so a field or select stays one segment.
        for cuts in cuts_of.values_mut() {
            let v: Vec<u32> = cuts.iter().copied().collect();
            for p in v.windows(2) {
                if p[1] - p[0] > 64 {
                    cuts.extend((p[0] / 64 + 1..).map(|k| k * 64).take_while(|&c| c < p[1]));
                }
            }
        }
        if !pairs.is_empty() {
            align(&mut cuts_of);
        }
        let mut plan: Vec<(usize, Vec<u32>)> = Vec::with_capacity(wides.len());
        let mut total = csp.vars.len();
        for v in wides {
            let cuts: Vec<u32> = cuts_of[&v].iter().copied().collect();
            total += cuts.len() - 1;
            plan.push((v, cuts));
        }
        if total > MAX_VARS {
            csp.wide_fail = true;
            return;
        }
        for (v, cuts) in plan {
            let mut segs = Vec::with_capacity(cuts.len() - 1);
            for k in 0..cuts.len() - 1 {
                let (lo, hi) = (cuts[k], cuts[k + 1]);
                let x = Self::csp_aux_var(csp, hi - lo);
                csp.aux_base.insert(x, (v, lo, hi - lo));
                csp.wide_seg.insert(x);
                segs.push((lo, hi - lo, x));
            }
            // A select spanning several segments (`f[k]` over `f[k][4:0]`)
            // is a variable of its own, tied to them (`csp_slice_links`).
            if csp.has_subs
                && let Some((ranges, _)) = uses.get(&v)
            {
                for &(lo, rw) in ranges {
                    let covered = segs
                        .iter()
                        .filter(|s| s.0 >= lo && s.0 + s.1 <= lo + rw)
                        .count();
                    if rw <= 64
                        && covered > 1
                        && !csp.slice_vars.contains_key(&(v, lo, rw))
                        && csp.vars.len() < MAX_VARS
                    {
                        let x = Self::csp_aux_var(csp, rw);
                        csp.aux_base.insert(x, (v, lo, rw));
                        csp.slice_vars.insert((v, lo, rw), x);
                    }
                }
            }
            csp.segs.insert(v, segs);
        }
    }

    /// The object declaring solver scalar `name` and its member name there:
    /// the sub-object for a `prop.x` variable (`csp_add_sub`), the object
    /// being solved otherwise.
    fn csp_owner<'a>(csp: &'a Csp, name: &'a str) -> (usize, &'a str) {
        match csp.scalars.get(name).map(|&v| &csp.vars[v].key) {
            Some(VarKey::Sub(h, n)) => (*h, n.as_str()),
            _ => (csp.handle, name),
        }
    }

    /// §7.4.5: the fields of struct scalar `name` (nested ones included)
    /// that are packed arrays of elements, as ((lsb, width), (left, right,
    /// element width)) of their first dimension.
    #[allow(clippy::type_complexity)]
    fn csp_field_arrays(
        &self,
        csp: &Csp,
        name: &str,
        fields: &[(String, u32, u32)],
    ) -> Vec<((u32, u32), (i64, i64, u32))> {
        let (owner, pname) = Self::csp_owner(csp, name);
        let Some(su) = self.class_prop_struct(owner, pname) else {
            return Vec::new();
        };
        let params = self.instance_param_scope(owner);
        let mut out = Vec::new();
        self.csp_field_arrays_walk(&su, &params, 0, fields, &mut out, 0);
        out
    }

    #[allow(clippy::type_complexity)]
    fn csp_field_arrays_walk(
        &self,
        su: &crate::ast::types::StructUnionType,
        params: &HashMap<String, Value>,
        base: u32,
        layout: &[(String, u32, u32)],
        out: &mut Vec<((u32, u32), (i64, i64, u32))>,
        depth: u32,
    ) {
        if depth > 8 {
            return;
        }
        for m in &su.members {
            for d in &m.declarators {
                let Some(&(_, lo, w)) = layout.iter().find(|f| f.0 == d.name.name) else {
                    continue;
                };
                if let DataType::Struct(inner) =
                    Self::resolve_type_ref(&m.data_type, &self.module.typedef_types)
                {
                    if inner.packed {
                        let (sub, _) = self.packed_agg_layout_with(&inner, params);
                        self.csp_field_arrays_walk(&inner, params, base + lo, &sub, out, depth + 1);
                    }
                    continue;
                }
                let Some(dims) = crate::compiler::elaborate::packed_full_dims_chained(
                    &m.data_type,
                    params,
                    &self.module.typedef_types,
                ) else {
                    continue;
                };
                if dims.len() < 2 {
                    continue;
                }
                let (l, r) = dims[0];
                let n = (l - r).unsigned_abs() + 1;
                if n == 0 || u64::from(w) % n != 0 {
                    continue;
                }
                out.push(((base + lo, w), (l, r, (u64::from(w) / n) as u32)));
            }
        }
    }

    /// §7.2.1: the fields of the packed structs nested in a wide struct
    /// variable, listed by their dotted paths (`a.b`) after its own fields.
    fn csp_nested_fields(&self, csp: &Csp, name: &str, fields: &mut Vec<(String, u32, u32)>) {
        let (owner, name) = Self::csp_owner(csp, name);
        let Some(su) = self.class_prop_struct(owner, name) else {
            return;
        };
        let params = self.instance_param_scope(owner);
        let top: Vec<(String, u32, u32)> = fields.clone();
        self.csp_nested_walk(&su, &params, "", 0, &top, fields, 0);
    }

    #[allow(clippy::too_many_arguments)]
    fn csp_nested_walk(
        &self,
        su: &crate::ast::types::StructUnionType,
        params: &HashMap<String, Value>,
        prefix: &str,
        base: u32,
        layout: &[(String, u32, u32)],
        out: &mut Vec<(String, u32, u32)>,
        depth: u32,
    ) {
        if depth > 8 {
            return;
        }
        for m in &su.members {
            let DataType::Struct(inner) =
                Self::resolve_type_ref(&m.data_type, &self.module.typedef_types)
            else {
                continue;
            };
            if !inner.packed {
                continue;
            }
            let (sub, _) = self.packed_agg_layout_with(&inner, params);
            for d in &m.declarators {
                let Some(&(_, lo, _)) = layout.iter().find(|f| f.0 == d.name.name) else {
                    continue;
                };
                let path = format!("{}{}.", prefix, d.name.name);
                for (n, l, w) in &sub {
                    out.push((format!("{}{}", path, n), base + lo + l, *w));
                }
                self.csp_nested_walk(&inner, params, &path, base + lo, &sub, out, depth + 1);
            }
        }
    }

    /// Boundaries a wide variable needs beyond its selects: the bits a
    /// constant shift keeps and the runs of a constant mask.
    fn csp_wide_scan(
        &mut self,
        csp: &Csp,
        e: &Expression,
        bound: &[String],
        uses: &mut HashMap<usize, (Vec<(u32, u32)>, bool)>,
    ) {
        let ExprKind::Binary { op, left, right } = &Self::unparen(e).kind else {
            return;
        };
        let scan_env = csp_scan_env();
        let kval = |me: &mut Self, x: &Expression| -> Option<Value> {
            let mut reads_bound = false;
            Self::walk_operands(x, &mut |y| {
                if let ExprKind::Ident(h) = &y.kind
                    && h.path.len() == 1
                    && bound.iter().any(|b| *b == h.path[0].name.name)
                {
                    reads_bound = true;
                }
            });
            if reads_bound || !me.csp_free(csp, x, &scan_env) {
                return None;
            }
            let v = me.csp_const_any(x, &scan_env);
            (!v.has_xz() && !v.is_real && v.width > 0).then_some(v)
        };
        let mut konst = |me: &mut Self, x: &Expression| -> Option<i64> {
            let v = kval(me, x)?;
            if v.width > 64 {
                return None;
            }
            v.to_i64()
        };
        match op {
            BinaryOp::ShiftRight
            | BinaryOp::ShiftLeft
            | BinaryOp::ArithShiftRight
            | BinaryOp::ArithShiftLeft => {
                let Some(k) = konst(self, right) else { return };
                let Some((v, lo, w, _)) = self.csp_slice_base_k(csp, left, &mut konst) else {
                    return;
                };
                if !csp.wide.contains(&v) || k <= 0 || k >= i64::from(w) {
                    return;
                }
                let k = k as u32;
                let r = if matches!(op, BinaryOp::ShiftRight | BinaryOp::ArithShiftRight) {
                    (lo + k, w - k)
                } else {
                    (lo, w - k)
                };
                uses.entry(v).or_default().0.push(r);
            }
            BinaryOp::BitAnd | BinaryOp::BitOr => {
                for (x, m) in [(left, right), (right, left)] {
                    let Some(mv) = kval(self, m) else { continue };
                    let Some((v, lo, w, whole)) = self.csp_slice_base_k(csp, x, &mut konst) else {
                        continue;
                    };
                    if !csp.wide.contains(&v) {
                        continue;
                    }
                    let sext = whole && csp.vars[v].signed && mv.is_signed;
                    let mw = WOp::konst(w, &mv, sext);
                    let u = uses.entry(v).or_default();
                    let mut i = 0;
                    while i < w {
                        let b = mw.bit(i);
                        let start = i;
                        while i < w && mw.bit(i) == b {
                            i += 1;
                        }
                        u.0.push((lo + start, i - start));
                    }
                }
            }
            // §11.4.5/§11.4.4: a relation between two wide ranges is
            // decided segment by segment once both are cut alike.
            BinaryOp::Eq
            | BinaryOp::Neq
            | BinaryOp::CaseEq
            | BinaryOp::CaseNeq
            | BinaryOp::Lt
            | BinaryOp::Leq
            | BinaryOp::Gt
            | BinaryOp::Geq => {
                let mut bits = |me: &mut Self, x: &Expression| -> Option<(usize, u32, u32)> {
                    let x = Self::unparen(x);
                    if let Some((v, r)) = me.csp_select_ref(csp, x, &mut konst) {
                        let (lo, w) = r?;
                        return csp.wide.contains(&v).then_some((v, lo, w));
                    }
                    let (v, lo, w, _) = me.csp_slice_base_k(csp, x, &mut konst)?;
                    csp.wide.contains(&v).then_some((v, lo, w))
                };
                if let (Some(a), Some(b)) = (bits(self, left), bits(self, right))
                    && (a.0, a.1) != (b.0, b.1)
                {
                    CSP_SCAN_LINKS
                        .with(|l| l.borrow_mut().push((a.0, a.1, b.0, b.1, a.2.min(b.2))));
                }
            }
            _ => {}
        }
    }

    /// An item the dependency scan could not fully analyse may read a wide
    /// variable: it calls a method, or names one other than through
    /// `size()` (an array's size is fixed while it is solved).
    fn csp_may_read_wide(&self, csp: &Csp, item: &SrcItem) -> bool {
        let mut read: HashSet<String> = HashSet::default();
        let mut opaque = false;
        match item {
            SrcItem::Expr(e) => Self::csp_wide_expr_reads(e, &mut read, &mut opaque),
            SrcItem::Item(it) => Self::csp_wide_item_reads(it, &mut read, &mut opaque),
        }
        Self::csp_wide_named(csp, &read, opaque)
    }

    /// The variables `e` reads, when it reads wide variables only through
    /// fields and selects: their segments. False when it reads a whole
    /// wide variable, or something the scan cannot follow.
    fn csp_wide_deps(
        &mut self,
        csp: &Csp,
        e: &Expression,
        env: &Env,
        out: &mut Vec<usize>,
    ) -> bool {
        if matches!(
            e.kind,
            ExprKind::RangeSelect { .. }
                | ExprKind::Index { .. }
                | ExprKind::MemberAccess { .. }
                | ExprKind::Ident(_)
        ) && let Some((v, lo, w, whole)) = self.csp_wide_bits(csp, e, env)
        {
            if whole {
                return false;
            }
            let Some(segs) = csp.segs.get(&v) else {
                return false;
            };
            out.extend(
                segs.iter()
                    .filter(|s| s.0 < lo + w && s.0 + s.1 > lo)
                    .map(|s| s.2),
            );
            return true;
        }
        if !self.csp_wide_touch(csp, e, env) {
            return self.csp_refs(csp, e, env, out);
        }
        let all = |me: &mut Self, xs: &[&Expression], out: &mut Vec<usize>| {
            xs.iter().all(|x| me.csp_wide_deps(csp, x, env, out))
        };
        match &e.kind {
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                all(self, &[operand], out)
            }
            ExprKind::Binary { left, right, .. } => all(self, &[left, right], out),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => all(self, &[condition, then_expr, else_expr], out),
            ExprKind::Concatenation(xs) => {
                let xs: Vec<&Expression> = xs.iter().collect();
                all(self, &xs, out)
            }
            ExprKind::Inside { expr, ranges } => {
                let mut xs: Vec<&Expression> = vec![expr];
                xs.extend(ranges.iter());
                all(self, &xs, out)
            }
            ExprKind::Range(a, b) => all(self, &[a, b], out),
            // A select at a position not known yet (`f[id]`, `f[id][7:0]`)
            // reads the bits of what it selects from, and its index.
            ExprKind::Index { expr, index } => self.csp_wide_sel_deps(csp, expr, index, env, out),
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => all(self, &[expr, left, right], out),
            ExprKind::Ident(_) => match Self::csp_split_last_select(e) {
                Some((b, i)) => self.csp_wide_sel_deps(csp, &b, &i, env, out),
                None => false,
            },
            _ => false,
        }
    }

    /// `base[index]` for `csp_wide_deps`: the select's bits and index;
    /// under `CSP_DYN_WHOLE`, every segment of a whole wide `base`.
    fn csp_wide_sel_deps(
        &mut self,
        csp: &Csp,
        base: &Expression,
        index: &Expression,
        env: &Env,
        out: &mut Vec<usize>,
    ) -> bool {
        if CSP_DYN_WHOLE.with(|c| c.get())
            && let Some((v, _, _, true)) = self.csp_wide_bits(csp, base, env)
            && let Some(segs) = csp.segs.get(&v)
        {
            out.extend(segs.iter().map(|s| s.2));
            return self.csp_wide_deps(csp, index, env, out);
        }
        self.csp_wide_deps(csp, base, env, out) && self.csp_wide_deps(csp, index, env, out)
    }

    /// `e` may read a wide variable (see `csp_may_read_wide`).
    fn csp_wide_touch(&self, csp: &Csp, e: &Expression, _env: &Env) -> bool {
        let mut read: HashSet<String> = HashSet::default();
        let mut opaque = false;
        Self::csp_wide_expr_reads(e, &mut read, &mut opaque);
        Self::csp_wide_named(csp, &read, opaque)
    }

    fn csp_wide_named(csp: &Csp, read: &HashSet<String>, opaque: bool) -> bool {
        // A sub-object's variable `a.x` is named through its member `x`.
        let leaf = |n: &str| n.rsplit_once('.').map_or(n, |p| p.1).to_string();
        opaque
            || csp
                .scalars
                .iter()
                .any(|(n, v)| csp.wide.contains(v) && read.contains(&leaf(n)))
            || csp.arrays.iter().any(|(n, a)| {
                a.elems.first().is_some_and(|e| csp.wide.contains(&e.1)) && read.contains(n)
            })
    }

    /// The wide variable and bit range `e` names: Some((v, lsb, width,
    /// whole)) for a wide variable (`x`, `this.x`, `a[k]`), a field of one,
    /// or a constant select of either.
    fn csp_wide_bits(
        &mut self,
        csp: &Csp,
        e: &Expression,
        env: &Env,
    ) -> Option<(usize, u32, u32, bool)> {
        let mut konst = |me: &mut Self, x: &Expression| -> Option<i64> {
            if !me.csp_free(csp, x, env) {
                return None;
            }
            me.csp_const(x, env)?.to_i64()
        };
        if let Some((v, r)) = self.csp_select_ref(csp, e, &mut konst) {
            if !csp.wide.contains(&v) {
                return None;
            }
            let (lo, w) = r?;
            return Some((v, lo, w, false));
        }
        let (v, lo, w, whole) = self.csp_slice_base_k(csp, e, &mut konst)?;
        csp.wide.contains(&v).then_some((v, lo, w, whole))
    }

    /// Self-determined (width, signed) of an operand of a wide relation.
    fn csp_wide_ws(&mut self, csp: &Csp, e: &Expression, env: &Env) -> Option<(u32, bool)> {
        let e = Self::unparen(e);
        if self.csp_free(csp, e, env) {
            if matches!(
                e.kind,
                ExprKind::Number(crate::ast::expr::NumberLiteral::UnbasedUnsized(_))
            ) {
                // §5.7.1: `'0`/`'1` take the width of the context.
                return Some((1, false));
            }
            let v = self.csp_const_any(e, env);
            return (!v.has_xz() && !v.is_real && v.width > 0).then_some((v.width, v.is_signed));
        }
        match &e.kind {
            // §11.6.1: a shift has the width and sign of its left operand.
            ExprKind::Binary {
                op:
                    BinaryOp::ShiftRight
                    | BinaryOp::ShiftLeft
                    | BinaryOp::ArithShiftRight
                    | BinaryOp::ArithShiftLeft,
                left,
                ..
            } => return self.csp_wide_ws(csp, left, env),
            // §11.6.1/§11.8.1: `+ - & |` take the larger width, signed
            // only when both operands are.
            ExprKind::Binary {
                op: BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::Add | BinaryOp::Sub,
                left,
                right,
            } => {
                let (a, b) = (
                    self.csp_wide_ws(csp, left, env)?,
                    self.csp_wide_ws(csp, right, env)?,
                );
                return Some((a.0.max(b.0), a.1 && b.1));
            }
            ExprKind::Unary {
                op: UnaryOp::Minus,
                operand,
            } => return self.csp_wide_ws(csp, operand, env),
            _ => {}
        }
        if let Some((v, _, w, whole)) = self.csp_wide_bits(csp, e, env) {
            // §11.5.1: a select (and here a struct field) is unsigned.
            return Some((w, whole && csp.vars[v].signed));
        }
        let a = self.csp_ae(csp, e, env)?;
        Some(Self::csp_ws(&a))
    }

    /// `e` as a `w`-bit operand in a context of signedness `s` (§11.8.2).
    fn csp_wide_op(
        &mut self,
        csp: &Csp,
        e: &Expression,
        w: u32,
        s: bool,
        env: &Env,
    ) -> Option<WOp> {
        let e = Self::unparen(e);
        if self.csp_free(csp, e, env) {
            if let ExprKind::Number(crate::ast::expr::NumberLiteral::UnbasedUnsized(c)) = &e.kind {
                let mut o = WOp::zero(w);
                match c {
                    '0' => {}
                    '1' => (0..w).for_each(|i| o.set(i, true)),
                    _ => return None,
                }
                return Some(o);
            }
            let v = self.csp_const_any(e, env);
            if v.has_xz() || v.is_real || v.width == 0 || v.width > w {
                return None;
            }
            return Some(WOp::konst(w, &v, s && v.is_signed));
        }
        match &e.kind {
            ExprKind::Binary {
                op:
                    op @ (BinaryOp::ShiftRight
                    | BinaryOp::ShiftLeft
                    | BinaryOp::ArithShiftRight
                    | BinaryOp::ArithShiftLeft),
                left,
                right,
            } => {
                // §11.4.10: the shift amount is self-determined; `>>>` of a
                // signed operand fills with its sign.
                if !self.csp_free(csp, right, env) {
                    return None;
                }
                let k = self.csp_const(right, env)?.to_u64()?;
                if *op == BinaryOp::ArithShiftRight && s {
                    return None;
                }
                let x = self.csp_wide_op(csp, left, w, s, env)?;
                return if matches!(op, BinaryOp::ShiftRight | BinaryOp::ArithShiftRight) {
                    x.shr(k)
                } else {
                    x.shl(k)
                };
            }
            ExprKind::Binary {
                op: op @ (BinaryOp::BitAnd | BinaryOp::BitOr),
                left,
                right,
            } => {
                let (x, m) = if self.csp_free(csp, right, env) {
                    (left, right)
                } else if self.csp_free(csp, left, env) {
                    (right, left)
                } else {
                    return None;
                };
                let m = self.csp_wide_op(csp, m, w, s, env)?;
                let x = self.csp_wide_op(csp, x, w, s, env)?;
                return x.mask(&m, *op == BinaryOp::BitOr);
            }
            _ => {}
        }
        if let Some((v, lo, bw, whole)) = self.csp_wide_bits(csp, e, env) {
            if bw > w || (whole && csp.vars[v].signed && s && bw < w) {
                // A sign-extended wide operand is not modelled.
                return None;
            }
            let mut o = WOp::zero(w);
            for &(slo, sw, x) in csp.segs.get(&v)? {
                if slo >= lo && slo + sw <= lo + bw {
                    o.vars.push((slo - lo, sw, x));
                } else if slo < lo + bw && slo + sw > lo {
                    return None;
                }
            }
            return Some(o);
        }
        // A narrow unsigned variable (or a segment of one), zero-extended.
        match self.csp_ae(csp, e, env)? {
            Ae::Var(x, vw, false) if vw <= 64 && vw <= w => {
                let mut o = WOp::zero(w);
                o.vars.push((0, vw, x));
                Some(o)
            }
            _ => None,
        }
    }

    /// The chunks of a relation between `l` and `r`, most significant
    /// first: (left, right, is the sign bit), each side `k + Σ c·x`. A
    /// chunk holds whole variables and at most 64 bits; None when the two
    /// sides' variables overlap so that no such chunks exist.
    #[allow(clippy::type_complexity)]
    fn csp_wide_chunks(l: &WOp, r: &WOp, s: bool) -> Option<Vec<(Lin, Lin, bool)>> {
        let w = l.w;
        let mut b: Vec<u32> = (0..w).step_by(64).collect();
        b.push(w);
        let all = || l.vars.iter().chain(r.vars.iter());
        for &(lo, vw, _) in all() {
            b.push(lo);
            b.push(lo + vw);
        }
        if s {
            b.push(w - 1);
        }
        b.sort_unstable();
        b.dedup();
        b.retain(|&x| !all().any(|&(lo, vw, _)| lo < x && x < lo + vw));
        if s && !b.contains(&(w - 1)) {
            return None;
        }
        if b.len() > 513 {
            return None;
        }
        let side = |o: &WOp, a: u32, c: u32| -> Lin {
            Lin {
                t: o.vars
                    .iter()
                    .filter(|p| p.0 >= a && p.0 < c)
                    .map(|p| (p.2, 1i128 << (p.0 - a)))
                    .collect(),
                k: o.word(a, c) as i128,
            }
        };
        let mut out = Vec::with_capacity(b.len());
        for win in b.windows(2).rev() {
            let (a, c) = (win[0], win[1]);
            if c - a > 64 {
                return None;
            }
            out.push((side(l, a, c), side(r, a, c), s && a == w - 1));
        }
        Some(out)
    }

    /// `a - b + k REL 0` as a node, decided at once when the declared
    /// domains already decide it.
    fn csp_wide_lin(&mut self, csp: &mut Csp, a: &Lin, b: &Lin, k: i128, rel: Rel) -> Node {
        let mut t: Vec<(usize, i128)> =
            a.t.iter()
                .copied()
                .chain(b.t.iter().map(|&(v, c)| (v, -c)))
                .collect();
        t.sort_unstable_by_key(|x| x.0);
        let mut m: Vec<(usize, i128)> = Vec::with_capacity(t.len());
        for (v, c) in t {
            match m.last_mut() {
                Some(l) if l.0 == v => l.1 += c,
                _ => m.push((v, c)),
            }
        }
        m.retain(|x| x.1 != 0);
        let lin = Lin {
            t: m,
            k: a.k - b.k + k,
        };
        let (mut lo, mut hi) = (lin.k, lin.k);
        for &(v, c) in &lin.t {
            let d = &csp.dom0[v];
            let (x, y) = (d.first().map_or(0, |p| p.0), d.last().map_or(0, |p| p.1));
            if c > 0 {
                lo += c * x;
                hi += c * y;
            } else {
                lo += c * y;
                hi += c * x;
            }
        }
        let decided = match rel {
            Rel::Eq if lo == 0 && hi == 0 => Some(true),
            Rel::Eq if lo > 0 || hi < 0 => Some(false),
            Rel::Ne if lo == 0 && hi == 0 => Some(false),
            Rel::Ne if lo > 0 || hi < 0 => Some(true),
            Rel::Le if hi <= 0 => Some(true),
            Rel::Le if lo > 0 => Some(false),
            _ => None,
        };
        match decided {
            Some(true) => Node::True,
            Some(false) => Node::False,
            None => {
                let deps: Vec<usize> = lin.t.iter().map(|x| x.0).collect();
                let one = Self::literal_of(&Value::from_u64(1, 1), crate::ast::Span::dummy());
                let src = self.csp_src(csp, SrcItem::Expr(one), &Env::default(), deps);
                Node::Lin {
                    lin,
                    rel,
                    fits: Vec::new(),
                    src,
                    sneg: false,
                    wrap: None,
                }
            }
        }
    }

    fn csp_wide_and(v: Vec<Node>) -> Node {
        if v.iter().any(|n| matches!(n, Node::False)) {
            return Node::False;
        }
        Self::csp_and(v.into_iter().filter(|n| !matches!(n, Node::True)).collect())
    }

    fn csp_wide_or(v: Vec<Node>) -> Node {
        if v.iter().any(|n| matches!(n, Node::True)) {
            return Node::True;
        }
        let mut v: Vec<Node> = v
            .into_iter()
            .filter(|n| !matches!(n, Node::False))
            .collect();
        match v.len() {
            0 => Node::False,
            1 => v.pop().unwrap(),
            _ => Node::Or(v),
        }
    }

    /// `left op right` with a wide operand (see the impl docs). None when
    /// the shape is not modelled.
    fn csp_wide_rel(
        &mut self,
        csp: &mut Csp,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
        env: &Env,
        neg: bool,
    ) -> Option<Node> {
        let op = if neg {
            match op {
                BinaryOp::Eq | BinaryOp::CaseEq => BinaryOp::Neq,
                BinaryOp::Neq | BinaryOp::CaseNeq => BinaryOp::Eq,
                BinaryOp::Lt => BinaryOp::Geq,
                BinaryOp::Leq => BinaryOp::Gt,
                BinaryOp::Gt => BinaryOp::Leq,
                BinaryOp::Geq => BinaryOp::Lt,
                _ => return None,
            }
        } else {
            op
        };
        // §11.6.1/§11.8.1: both sides take the larger width, and are signed
        // only when both are.
        let (wl, sl) = self.csp_wide_ws(csp, left, env)?;
        let (wr, sr) = self.csp_wide_ws(csp, right, env)?;
        let (w, s) = (wl.max(wr), sl && sr);
        // An operand computed by `+`/`-` from a variable (a constant one,
        // `-80'sd10`, is just a constant).
        let arith = |me: &mut Self, csp: &Csp, e: &Expression| {
            Self::csp_wide_arith(e) && !me.csp_free(csp, e, env)
        };
        if arith(self, csp, left) || arith(self, csp, right) {
            return self.csp_wide_affine_rel(csp, op, left, right, w, s, env);
        }
        let l = self.csp_wide_op(csp, left, w, s, env)?;
        let r = self.csp_wide_op(csp, right, w, s, env)?;
        let chunks = Self::csp_wide_chunks(&l, &r, s)?;
        Some(match op {
            BinaryOp::Eq | BinaryOp::CaseEq => {
                let v = chunks
                    .iter()
                    .map(|(a, b, _)| self.csp_wide_lin(csp, a, b, 0, Rel::Eq))
                    .collect();
                Self::csp_wide_and(v)
            }
            BinaryOp::Neq | BinaryOp::CaseNeq => {
                let v = chunks
                    .iter()
                    .map(|(a, b, _)| self.csp_wide_lin(csp, a, b, 0, Rel::Ne))
                    .collect();
                Self::csp_wide_or(v)
            }
            BinaryOp::Lt => self.csp_wide_lex(csp, &chunks, false, true),
            BinaryOp::Leq => self.csp_wide_lex(csp, &chunks, false, false),
            BinaryOp::Gt => self.csp_wide_lex(csp, &chunks, true, true),
            BinaryOp::Geq => self.csp_wide_lex(csp, &chunks, true, false),
            _ => return None,
        })
    }

    /// Is `e` a sum, difference or negation (the shapes `csp_wide_affine`
    /// reads)?
    fn csp_wide_arith(e: &Expression) -> bool {
        matches!(
            Self::unparen(e).kind,
            ExprKind::Binary {
                op: BinaryOp::Add | BinaryOp::Sub,
                ..
            } | ExprKind::Unary {
                op: UnaryOp::Minus,
                ..
            }
        )
    }

    /// §11.4.3/§11.6.1: `e` in the context `(w, s)` as `±x + k` modulo
    /// 2^w, `x` being at most one operand that `csp_wide_op` reads as plain
    /// variable bits (a wide variable, a select of one, a narrow variable)
    /// and `k` the sum of the constant operands, extended to `w` bits. The
    /// operand comes back with whether it is negated; None for a constant.
    #[allow(clippy::type_complexity)]
    fn csp_wide_affine<'e>(
        &mut self,
        csp: &Csp,
        e: &'e Expression,
        w: u32,
        s: bool,
        env: &Env,
    ) -> Option<(Option<(&'e Expression, bool)>, Vec<u64>)> {
        let e = Self::unparen(e);
        if self.csp_free(csp, e, env) {
            let o = self.csp_wide_op(csp, e, w, s, env)?;
            return o.vars.is_empty().then_some((None, o.bits));
        }
        match &e.kind {
            ExprKind::Binary {
                op: op @ (BinaryOp::Add | BinaryOp::Sub),
                left,
                right,
            } => {
                let (xa, ka) = self.csp_wide_affine(csp, left, w, s, env)?;
                let (xb, kb) = self.csp_wide_affine(csp, right, w, s, env)?;
                let sub = *op == BinaryOp::Sub;
                let x = match (xa, xb.map(|(x, n)| (x, n ^ sub))) {
                    (Some(_), Some(_)) => return None,
                    (a, b) => a.or(b),
                };
                let k = if sub {
                    ww_sub(&ka, &kb, w)
                } else {
                    ww_add(&ka, &kb, w)
                };
                Some((x, k))
            }
            ExprKind::Unary {
                op: UnaryOp::Minus,
                operand,
            } => {
                let (x, k) = self.csp_wide_affine(csp, operand, w, s, env)?;
                Some((x.map(|(x, n)| (x, !n)), ww_neg(&k, w)))
            }
            _ => {
                let o = self.csp_wide_op(csp, e, w, s, env)?;
                if o.vars.is_empty() || o.bits.iter().any(|&b| b != 0) {
                    return None;
                }
                Some((Some((e, false)), vec![0; o.bits.len()]))
            }
        }
    }

    /// `left op right` where one side is `±x + k` (modulo 2^w, see
    /// `csp_wide_affine`) and the other a constant `c`: the values of
    /// `±x + k` that satisfy `op c` are one interval (two for `!=`), and
    /// moving `k` and the sign across turns each into an interval of `x` —
    /// two when it wraps past 2^w. A signed context compares `y ^ msb`
    /// unsigned (§11.4.4). So `w + 80'd5 == 80'd2` is `w == 2^80 - 3`.
    #[allow(clippy::too_many_arguments)]
    fn csp_wide_affine_rel(
        &mut self,
        csp: &mut Csp,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
        w: u32,
        s: bool,
        env: &Env,
    ) -> Option<Node> {
        let (xl, kl) = self.csp_wide_affine(csp, left, w, s, env)?;
        let (xr, kr) = self.csp_wide_affine(csp, right, w, s, env)?;
        let (x, neg, k, c, op) = match (xl, xr) {
            (Some((x, n)), None) => (x, n, kl, kr, op),
            (None, Some((x, n))) => {
                let op = match op {
                    BinaryOp::Lt => BinaryOp::Gt,
                    BinaryOp::Leq => BinaryOp::Geq,
                    BinaryOp::Gt => BinaryOp::Lt,
                    BinaryOp::Geq => BinaryOp::Leq,
                    o => o,
                };
                (x, n, kr, kl, op)
            }
            _ => return None,
        };
        // A signed operand narrower than the signed context is sign-extended
        // there; its bits are not one interval of the extended value.
        let (xw, xs) = self.csp_wide_ws(csp, x, env)?;
        if s && xs && xw < w {
            return None;
        }
        let nw = w.div_ceil(64) as usize;
        let zero = vec![0u64; nw];
        let mut max = vec![u64::MAX; nw];
        ww_mask(&mut max, w);
        let mut bias = zero.clone();
        if s {
            bias[((w - 1) / 64) as usize] = 1u64 << ((w - 1) % 64);
        }
        let one = {
            let mut o = zero.clone();
            o[0] = 1;
            o
        };
        // The biased value `y' = y + bias` of `y = ±x + k` lies in `ys`.
        let cb = ww_add(&c, &bias, w);
        let below = (cb != zero).then(|| ww_sub(&cb, &one, w));
        let above = (cb != max).then(|| ww_add(&cb, &one, w));
        let ys: Vec<(Vec<u64>, Vec<u64>)> = match op {
            BinaryOp::Eq | BinaryOp::CaseEq => vec![(cb.clone(), cb.clone())],
            BinaryOp::Neq | BinaryOp::CaseNeq => below
                .map(|b| (zero.clone(), b))
                .into_iter()
                .chain(above.map(|a| (a, max.clone())))
                .collect(),
            BinaryOp::Lt => below.map(|b| (zero.clone(), b)).into_iter().collect(),
            BinaryOp::Leq => vec![(zero.clone(), cb.clone())],
            BinaryOp::Gt => above.map(|a| (a, max.clone())).into_iter().collect(),
            BinaryOp::Geq => vec![(cb.clone(), max.clone())],
            _ => return None,
        };
        let kb = ww_add(&k, &bias, w);
        let mut alts = Vec::new();
        let span = left.span;
        let lit = |v: &[u64]| Self::literal_of(&Value::from_words(v, w), span);
        let mut ivs = Vec::new();
        for (a, b) in ys {
            // `y' = x + kb` gives `x = y' - kb`; `y' = kb - x` gives
            // `x = kb - y'` (decreasing, so the ends swap).
            let (p, q) = if neg {
                (ww_sub(&kb, &b, w), ww_sub(&kb, &a, w))
            } else {
                (ww_sub(&a, &kb, w), ww_sub(&b, &kb, w))
            };
            if ww_cmp(&p, &q) == std::cmp::Ordering::Greater {
                ivs.push((p, max.clone()));
                ivs.push((zero.clone(), q));
            } else {
                ivs.push((p, q));
            }
        }
        // Several intervals: their disjunction acts only once all but one
        // are excluded, so a search drawing the variable's 64-bit segments
        // at random would almost never meet one. Its most significant
        // segment (bits `lo..w` of `x`) lies in the union of the intervals'
        // `>> lo` images, which propagates at once.
        let mut proj = None;
        if ivs.len() > 1
            && let Some(o) = self.csp_wide_op(csp, x, w, false, env)
            && let Some(&(lo, vw, v)) = o.vars.iter().max_by_key(|t| t.0)
            && lo + vw == w
            && lo < 128
        {
            let top = |x: &[u64]| -> i128 {
                let mut r: u128 = 0;
                for i in lo..w {
                    if (x[(i / 64) as usize] >> (i % 64)) & 1 == 1 {
                        r |= 1u128 << (i - lo);
                    }
                }
                r as i128
            };
            let set = dom_norm(ivs.iter().map(|(p, q)| (top(p), top(q))).collect());
            let one = Self::literal_of(&Value::from_u64(1, 1), crate::ast::Span::dummy());
            let src = self.csp_src(csp, SrcItem::Expr(one), &Env::default(), vec![v]);
            proj = Some(Node::In {
                lin: Lin::var(v),
                set,
                neg: false,
                fits: Vec::new(),
                src,
                wrap: None,
            });
        }
        {
            for (p, q) in ivs {
                let n = if p == q {
                    self.csp_wide_rel(csp, BinaryOp::Eq, x, &lit(&p), env, false)?
                } else {
                    let mut both = Vec::new();
                    if p != zero {
                        both.push(self.csp_wide_rel(
                            csp,
                            BinaryOp::Geq,
                            x,
                            &lit(&p),
                            env,
                            false,
                        )?);
                    }
                    if q != max {
                        both.push(self.csp_wide_rel(
                            csp,
                            BinaryOp::Leq,
                            x,
                            &lit(&q),
                            env,
                            false,
                        )?);
                    }
                    Self::csp_wide_and(both)
                };
                alts.push(n);
            }
        }
        let set = Self::csp_wide_or(alts);
        Some(match proj {
            Some(p) => Self::csp_wide_and(vec![set, p]),
            None => set,
        })
    }

    /// `l < r` (`strict`) or `l <= r` over chunks, most significant first;
    /// with `swap`, `r < l` / `r <= l`. Built from the least significant
    /// chunk up: `le_i && (lt_i || (eq_i && rest))`. The implied `le_i`
    /// propagates bounds while the disjunction is still open.
    fn csp_wide_lex(
        &mut self,
        csp: &mut Csp,
        chunks: &[(Lin, Lin, bool)],
        swap: bool,
        strict: bool,
    ) -> Node {
        let mut rest = if strict { Node::False } else { Node::True };
        for (a, b, sign) in chunks.iter().rev() {
            let (a, b) = if swap { (b, a) } else { (a, b) };
            // A set sign bit is the smaller value (§11.4.4).
            let lt = if *sign {
                self.csp_wide_lin(csp, b, a, 1, Rel::Le)
            } else {
                self.csp_wide_lin(csp, a, b, 1, Rel::Le)
            };
            let le = if *sign {
                self.csp_wide_lin(csp, b, a, 0, Rel::Le)
            } else {
                self.csp_wide_lin(csp, a, b, 0, Rel::Le)
            };
            let eq = self.csp_wide_lin(csp, a, b, 0, Rel::Eq);
            let or = Self::csp_wide_or(vec![lt, Self::csp_wide_and(vec![eq, rest])]);
            rest = Self::csp_wide_and(vec![le, or]);
        }
        rest
    }

    /// `x == other` with `x` a whole wide variable and `other` an expression
    /// over other variables the segment model cannot express (`a ^ b`): once
    /// every variable `other` reads is fixed it is evaluated, and each
    /// segment of `x` is fixed to its bits of the value (one `Fun` per
    /// segment), instead of the segments being guessed.
    fn csp_wide_fun(
        &mut self,
        csp: &mut Csp,
        whole: &Expression,
        x: &Expression,
        other: &Expression,
        env: &Env,
    ) -> Option<Node> {
        let (v, lo, bw, is_whole) = self.csp_wide_bits(csp, x, env)?;
        if !is_whole || lo != 0 || bw != csp.vars[v].width {
            return None;
        }
        let segs = csp.segs.get(&v)?.clone();
        let mut deps = Vec::new();
        if !self.csp_wide_fun_deps(csp, other, env, &mut deps) {
            return None;
        }
        if deps.is_empty() || segs.iter().any(|s| deps.contains(&s.2) || s.2 == v) {
            return None;
        }
        deps.sort_unstable();
        deps.dedup();
        let span = whole.span;
        let mut all = deps.clone();
        all.extend(segs.iter().map(|s| s.2));
        all.sort_unstable();
        all.dedup();
        let whole_src = self.csp_src(csp, SrcItem::Expr(whole.clone()), env, all);
        let mut nodes = Vec::with_capacity(segs.len());
        for &(slo, sw, sv) in &segs {
            let mask = if sw >= 64 { u64::MAX } else { (1u64 << sw) - 1 };
            let shifted = Expression::new(
                ExprKind::Binary {
                    op: BinaryOp::ShiftRight,
                    left: Box::new(Expression::new(
                        ExprKind::Paren(Box::new(other.clone())),
                        span,
                    )),
                    right: Box::new(Self::literal_of(&Value::from_u64(slo as u64, 32), span)),
                },
                span,
            );
            let seg_e = Expression::new(
                ExprKind::Binary {
                    op: BinaryOp::BitAnd,
                    left: Box::new(shifted),
                    right: Box::new(Self::literal_of(&Value::from_u64(mask, sw.max(1)), span)),
                },
                span,
            );
            let other_src = self.csp_src(csp, SrcItem::Expr(seg_e), env, deps.clone());
            nodes.push(Node::Fun {
                var: sv,
                op: BinaryOp::Eq,
                other: other_src,
                whole: whole_src,
                neg: false,
            });
        }
        Some(Self::csp_and(nodes))
    }

    /// The variables `e` reads for `csp_wide_fun`: the segments under a
    /// wide variable, a select or field of one, else what `csp_refs` finds.
    fn csp_wide_fun_deps(
        &mut self,
        csp: &Csp,
        e: &Expression,
        env: &Env,
        out: &mut Vec<usize>,
    ) -> bool {
        if matches!(
            e.kind,
            ExprKind::RangeSelect { .. }
                | ExprKind::Index { .. }
                | ExprKind::MemberAccess { .. }
                | ExprKind::Ident(_)
        ) && let Some((v, lo, w, _)) = self.csp_wide_bits(csp, e, env)
        {
            let Some(segs) = csp.segs.get(&v) else {
                return false;
            };
            out.extend(
                segs.iter()
                    .filter(|s| s.0 < lo + w && s.0 + s.1 > lo)
                    .map(|s| s.2),
            );
            return true;
        }
        match &e.kind {
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                self.csp_wide_fun_deps(csp, operand, env, out)
            }
            ExprKind::Binary { left, right, .. } => {
                self.csp_wide_fun_deps(csp, left, env, out)
                    && self.csp_wide_fun_deps(csp, right, env, out)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.csp_wide_fun_deps(csp, condition, env, out)
                    && self.csp_wide_fun_deps(csp, then_expr, env, out)
                    && self.csp_wide_fun_deps(csp, else_expr, env, out)
            }
            ExprKind::Concatenation(xs) => {
                xs.iter().all(|x| self.csp_wide_fun_deps(csp, x, env, out))
            }
            _ if self.csp_free(csp, e, env) => true,
            _ if !self.csp_wide_touch(csp, e, env) => self.csp_refs(csp, e, env, out),
            _ => false,
        }
    }

    /// §11.4.13: `expr inside {…}` with a wide operand, as the disjunction
    /// of `expr == v` and `lo <= expr <= hi`.
    fn csp_wide_inside(
        &mut self,
        csp: &mut Csp,
        expr: &Expression,
        ranges: &[ConstraintRange],
        env: &Env,
        neg: bool,
    ) -> Option<Node> {
        let mut alts = Vec::with_capacity(ranges.len());
        for r in ranges {
            match r {
                ConstraintRange::Value(v) => {
                    if self
                        .csp_member(csp, v)
                        .is_some_and(|n| csp.arrays.contains_key(&n))
                    {
                        return None;
                    }
                    alts.push(self.csp_wide_rel(csp, BinaryOp::Eq, expr, v, env, neg)?);
                }
                ConstraintRange::Range { lo, hi } => {
                    if matches!(lo.kind, ExprKind::Dollar) || matches!(hi.kind, ExprKind::Dollar) {
                        return None;
                    }
                    let a = self.csp_wide_rel(csp, BinaryOp::Geq, expr, lo, env, neg)?;
                    let b = self.csp_wide_rel(csp, BinaryOp::Leq, expr, hi, env, neg)?;
                    alts.push(if neg {
                        Self::csp_wide_or(vec![a, b])
                    } else {
                        Self::csp_wide_and(vec![a, b])
                    });
                }
            }
        }
        let set = if neg {
            Self::csp_wide_and(alts)
        } else {
            Self::csp_wide_or(alts)
        };
        if neg || !matches!(set, Node::Or(_)) {
            return Some(set);
        }
        // `x inside {K1, K2, …}` over constants: each segment of `x` takes
        // the bits of one of the values there. That domain restriction is
        // implied by the disjunction, which alone only acts once all but
        // one alternative are excluded, so a search drawing 64-bit segments
        // at random would almost never meet a value.
        let mut proj = vec![set];
        if let Some(seg) = self.csp_wide_value_projection(csp, expr, ranges, env) {
            proj.extend(seg);
        }
        Some(Self::csp_wide_and(proj))
    }

    /// The per-segment domains `expr inside {K1, K2, …}` implies when every
    /// item is a constant value: segment `x[a+:n]` in `{Ki[a+:n]}`.
    fn csp_wide_value_projection(
        &mut self,
        csp: &mut Csp,
        expr: &Expression,
        ranges: &[ConstraintRange],
        env: &Env,
    ) -> Option<Vec<Node>> {
        let (w, s) = self.csp_wide_ws(csp, expr, env)?;
        let mut consts = Vec::with_capacity(ranges.len());
        for r in ranges {
            let ConstraintRange::Value(v) = r else {
                return None;
            };
            let (vw, vs) = self.csp_wide_ws(csp, v, env)?;
            let (cw, cs) = (w.max(vw), s && vs);
            if cw != w {
                return None;
            }
            if !self.csp_free(csp, v, env) {
                return None;
            }
            consts.push(self.csp_wide_op(csp, v, cw, cs, env)?);
        }
        let x = self.csp_wide_op(csp, expr, w, s, env)?;
        let mut out = Vec::new();
        for &(lo, vw, v) in &x.vars {
            let set = dom_norm(
                consts
                    .iter()
                    .map(|c| {
                        let k = c.word(lo, lo + vw) as i128;
                        (k, k)
                    })
                    .collect(),
            );
            let one = Self::literal_of(&Value::from_u64(1, 1), crate::ast::Span::dummy());
            let src = self.csp_src(csp, SrcItem::Expr(one), &Env::default(), vec![v]);
            out.push(Node::In {
                lin: Lin::var(v),
                set,
                neg: false,
                fits: Vec::new(),
                src,
                wrap: None,
            });
        }
        Some(out)
    }

    /// §18.4: the wide variables left out of the solve take random values
    /// of their full width.
    fn csp_draw_free(&mut self, csp: &Csp) {
        for (key, w) in &csp.wide_free {
            let mut acc = Value::zero(*w);
            let mut bit = 0u32;
            while bit < *w {
                let chunk: u64 = self.cur_rng().random();
                let n = (*w - bit).min(64);
                for k in 0..n {
                    if (chunk >> k) & 1 == 1 {
                        acc.set_bit((bit + k) as usize, LogicBit::One);
                    }
                }
                bit += n;
            }
            match key {
                VarKey::Prop(n) => {
                    if let Some(Some(inst)) = self.heap.get_mut(csp.handle) {
                        if let Some(old) = inst.properties.get(n) {
                            acc.is_signed = old.is_signed;
                        }
                        inst.properties.insert(n.clone(), acc);
                    }
                }
                VarKey::Elem(k) => self.write_coll_elem(k, acc),
                VarKey::Sub(..) | VarKey::Aux => {}
            }
        }
    }
}
