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
use rand::Rng;

thread_local! {
    /// Set while a `std::randomize` (scope) solve runs: the solver's
    /// evaluation frames then extend the caller's locals instead of hiding
    /// them (see `csp_frame`).
    static CSP_SCOPE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
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
#[derive(Clone, Debug, Default)]
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
    },
    /// `lin inside set` (or not, with `neg`).
    In {
        lin: Lin,
        set: Dom,
        neg: bool,
        fits: Vec<Fit>,
        src: usize,
    },
    AllDiff(Vec<usize>),
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
    /// Read by some `if`/`->` condition.
    guard_var: Vec<bool>,
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
    /// Packed-struct field layout (name, lsb, width) of a base variable.
    fields: HashMap<usize, Vec<(String, u32, u32)>>,
    /// Declared lsb of a base variable's packed range (`[31:8]` -> 8).
    lsb: HashMap<usize, i64>,
    /// Variables whose packed labels increase from most to least significant.
    ascending: HashSet<usize>,
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
        let Some(mut csp) = self.csp_vars(
            handle,
            rand_props,
            signed_props,
            enum_props,
            colls,
            array_enums,
        ) else {
            return CspOutcome::NotApplicable;
        };
        // §18.5.9: the rand sub-objects' scalars and their own constraints
        // join the problem, so an item tying two of them together (`a.x <
        // b.y`) or one to an enclosing member is solved jointly.
        let mut joint: Vec<ClassConstraint>;
        let constraints: &[ClassConstraint] = if subs.is_empty() {
            constraints
        } else {
            joint = constraints.to_vec();
            for (prop, sub) in subs {
                let Some(items) = self.csp_add_sub(&mut csp, prop, *sub) else {
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
            &joint
        };
        self.csp_split_slices(&mut csp, constraints);
        if self
            .csp_translate(&mut csp, constraints, constraint_depth)
            .is_none()
        {
            return CspOutcome::NotApplicable;
        }
        if strict && csp.opaque {
            return CspOutcome::GaveUp;
        }
        let out = self.csp_run(&csp, constraints, colls);
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
        let hard = self.csp_run(&csp, constraints, colls);
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
            match self.csp_run(&csp, constraints, colls) {
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
    /// are (name, lvalue, width, signed, enum values); `arrays` are (name,
    /// element indices, width, signed, enum values) of collections already
    /// sized. The solution is written to the caller's variables and checked
    /// with the inline-constraint checker.
    #[allow(clippy::type_complexity)]
    pub(super) fn rand_csp_solve_scope(
        &mut self,
        items: &[ConstraintItem],
        scalars: &[(String, Expression, u32, bool, Option<Vec<u64>>)],
        arrays: &[(String, Vec<i64>, u32, bool, Option<Vec<u64>>)],
    ) -> CspOutcome {
        let mut csp = Csp {
            handle: usize::MAX,
            vars: Vec::new(),
            dom0: Vec::new(),
            scalars: HashMap::default(),
            arrays: HashMap::default(),
            srcs: Vec::new(),
            nodes: Vec::new(),
            soft_enabled: None,
            soft_priority: Vec::new(),
            soft_rank: (0, 0),
            dists: Vec::new(),
            order: Vec::new(),
            opaque: false,
            has_subs: false,
            scope: Some(HashMap::default()),
            segs: HashMap::default(),
            slice_vars: HashMap::default(),
            fields: HashMap::default(),
            lsb: HashMap::default(),
            ascending: HashSet::default(),
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
        for (name, idx, w, s, members) in arrays {
            if *w == 0 || *w > 64 {
                return CspOutcome::NotApplicable;
            }
            let dom = dom_of(*w, *s, members);
            let mut elems = Vec::with_capacity(idx.len());
            for &i in idx {
                elems.push((i, csp.vars.len()));
                csp.vars.push(CspVar {
                    key: VarKey::Elem(format!("{}[{}]", name, i)),
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
        for (name, lv, w, s, members) in scalars {
            if *w == 0 || *w > 64 || csp.arrays.contains_key(name) {
                return CspOutcome::NotApplicable;
            }
            csp.scalars.insert(name.clone(), csp.vars.len());
            csp.vars.push(CspVar {
                key: VarKey::Prop(name.clone()),
                width: *w,
                signed: *s,
            });
            csp.dom0.push(dom_of(*w, *s, members));
            if let Some(l) = csp.scope.as_mut() {
                l.insert(name.clone(), lv.clone());
            }
        }
        if csp.vars.is_empty() || csp.vars.len() > MAX_VARS {
            return CspOutcome::NotApplicable;
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
        let out = if self.csp_translate(&mut csp, &constraints, &[]).is_none() {
            CspOutcome::NotApplicable
        } else {
            self.csp_run(&csp, &constraints, &[])
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
    ) -> Option<Csp> {
        let mut csp = Csp {
            handle,
            vars: Vec::new(),
            dom0: Vec::new(),
            scalars: HashMap::default(),
            arrays: HashMap::default(),
            srcs: Vec::new(),
            nodes: Vec::new(),
            soft_enabled: None,
            soft_priority: Vec::new(),
            soft_rank: (0, 0),
            dists: Vec::new(),
            order: Vec::new(),
            opaque: false,
            has_subs: false,
            scope: None,
            segs: HashMap::default(),
            slice_vars: HashMap::default(),
            fields: HashMap::default(),
            lsb: HashMap::default(),
            ascending: HashSet::default(),
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
            if c.is_object_elem || c.nested || c.kind == CollKind::Assoc || c.width > 64 {
                return None;
            }
            let signed = self.class_prop_signed_of(handle, &c.prop);
            let dom = array_enums
                .get(&c.prop)
                .and_then(|tn| enum_dom(self, tn))
                .unwrap_or_else(|| {
                    let (lo, hi) = ws_range(c.width, signed);
                    vec![(lo, hi)]
                });
            let idx: Vec<i64> = match c.kind {
                CollKind::Fixed => (c.lo..=c.hi).collect(),
                _ => (0..self.get_queue_size(&c.scoped) as i64).collect(),
            };
            let mut elems = Vec::with_capacity(idx.len());
            for i in idx {
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
            if csp.arrays.contains_key(name) {
                continue;
            }
            if *w == 0 || *w > 64 {
                return None;
            }
            let signed = signed_props.contains(name);
            let dom = enum_props
                .get(name)
                .and_then(|tn| enum_dom(self, tn))
                .unwrap_or_else(|| {
                    let (lo, hi) = ws_range(*w, signed);
                    vec![(lo, hi)]
                });
            csp.scalars.insert(name.clone(), csp.vars.len());
            csp.vars.push(CspVar {
                key: VarKey::Prop(name.clone()),
                width: *w,
                signed,
            });
            csp.dom0.push(dom);
        }
        if csp.vars.is_empty() || csp.vars.len() > MAX_VARS {
            return None;
        }
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
                let n = self.csp_item(csp, it, &env)?;
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
        };
        loop {
            st.d.clone_from(&csp.dom0);
            st.trail.clear();
            st.dirty.clear();
            let mut run = run_budget.min(budget);
            let r = self.csp_search(csp, &aux, &watch, &mut st, &mut run);
            budget -= run_budget.min(budget) - run;
            match r {
                Some(true) => break,
                Some(false) => return CspOutcome::Unsat,
                None if budget == 0 || st.work >= WORK_BUDGET => {
                    if std::env::var_os("XEZIM_RAND_DBG").is_some() {
                        eprintln!(
                            "[rand-dbg] joint solve gave up: nodes={} work={}",
                            NODE_BUDGET - budget,
                            st.work
                        );
                    }
                    return CspOutcome::GaveUp;
                }
                None => run_budget *= 2,
            }
        }
        for v in 0..nvars {
            let x = st.fixed(v).unwrap_or(0);
            self.csp_write(csp, v, x);
        }
        let accepted = if csp.scope.is_some() {
            let items: Vec<ConstraintItem> =
                constraints.iter().flat_map(|c| c.items.iter().cloned()).collect();
            self.inline_constraints_satisfied(&items)
        } else {
            self.rand_items_accept(csp.handle, constraints, colls, &mut false)
        };
        if accepted {
            CspOutcome::Sat
        } else {
            if std::env::var_os("XEZIM_RAND_DBG").is_some() {
                eprintln!("[rand-dbg] joint solve found a solution the checker rejects");
            }
            CspOutcome::GaveUp
        }
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
        for (v, d) in st.d.iter().enumerate() {
            if dom_fixed(d).is_some() {
                continue;
            }
            let blocked = aux.preds[v].iter().any(|&p| st.fixed(p).is_none());
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
                    if self.cur_rng().gen_range(0..=ties) == 0 {
                        best = Some((rank, n, v));
                    }
                }
                _ => {
                    best = Some((rank, n, v));
                    ties = 0;
                }
            }
        }
        best.map(|b| b.2)
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
                    let mut r = self.cur_rng().gen_range(0.0..total);
                    let mut pick = live.len() - 1;
                    for (i, (_, w)) in live.iter().enumerate() {
                        if r < *w {
                            pick = i;
                            break;
                        }
                        r -= w;
                    }
                    let part = &live[pick].0;
                    let k = self.cur_rng().gen_range(0..dom_size(part));
                    return dom_nth(part, k);
                }
            }
        }
        let size = dom_size(&st.d[v]);
        if aux.guard_var[v] && size <= EST_MAX_DOM && st.est_work < EST_BUDGET {
            if let Some(x) = self.csp_weigh(csp, aux, watch, st, v) {
                return x;
            }
        }
        let k = self.cur_rng().gen_range(0..size);
        dom_nth(&st.d[v], k)
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
        let mut todo: Vec<usize> = aux.succs[v].clone();
        while let Some(u) = todo.pop() {
            if !after[u] {
                after[u] = true;
                todo.extend(aux.succs[u].iter().copied());
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
                    if u == v || after[u] || !aux.dists[u].is_empty() || seen[u] {
                        continue;
                    }
                    seen[u] = true;
                    touched.push(u);
                    lg += (dom_size(&st.d[u]) as f64).log2() - (dom_size(old) as f64).log2();
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
        let mut r = self.cur_rng().gen_range(0.0..total);
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
        };
        for (i, d) in csp.dists.iter().enumerate() {
            aux.dists[d.var].push(i);
        }
        for (before, after) in &csp.order {
            for &a in after {
                for &b in before {
                    if a != b {
                        aux.preds[a].push(b);
                        aux.succs[b].push(a);
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
        let mut gv = Vec::new();
        for nd in &csp.nodes {
            conds(nd, &csp.srcs, &mut gv);
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
        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        for &i in start {
            if !queued[i] {
                queued[i] = true;
                queue.push_back(i);
            }
        }
        while let Some(i) = queue.pop_front() {
            queued[i] = false;
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
            } => {
                if !st.fits_hold(fits) {
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
            } => {
                if !st.fits_hold(fits) {
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
            Node::Eval { src, neg } => {
                !self.csp_src_fixed(csp, st, *src) || self.csp_eval_src(csp, st, *src) != *neg
            }
            Node::Fun {
                var,
                op,
                other,
                whole,
                neg,
            } => {
                if !self.csp_src_fixed(csp, st, *other) {
                    return true;
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

    fn csp_src_fixed(&self, csp: &Csp, st: &St, src: usize) -> bool {
        st.all_fixed(&csp.srcs[src].deps)
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
    /// the object being randomized) as `prop.<name>` variables, and return
    /// its class constraints rewritten to name them through `prop`. None
    /// when the sub-object holds anything this solver does not model: a
    /// nested object, a collection, a real, a randc, or a constraint that
    /// calls a method.
    fn csp_add_sub(
        &mut self,
        csp: &mut Csp,
        prop: &str,
        sub: usize,
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
                let is_obj = sig
                    .type_name
                    .as_ref()
                    .is_some_and(|tn| self.module.classes.contains_key(tn));
                if is_obj
                    || sig.is_real
                    || sig.width == 0
                    || sig.width > 64
                    || cd.array_properties.contains_key(&name)
                    || cd.array_nd_properties.contains_key(&name)
                    || cd.queue_properties.contains_key(&name)
                    || cd.assoc_properties.contains_key(&name)
                {
                    return None;
                }
                let dom = sig
                    .type_name
                    .as_ref()
                    .and_then(|tn| self.module.enum_members.get(tn))
                    .filter(|m| !m.is_empty())
                    .map(|m| dom_norm(m.iter().map(|e| (e.1 as i128, e.1 as i128)).collect()))
                    .unwrap_or_else(|| {
                        let (lo, hi) = ws_range(sig.width, sig.is_signed);
                        vec![(lo, hi)]
                    });
                csp.scalars
                    .insert(format!("{}.{}", prop, name), csp.vars.len());
                csp.vars.push(CspVar {
                    key: VarKey::Sub(sub, name.clone()),
                    width: sig.width,
                    signed: sig.is_signed,
                });
                csp.dom0.push(dom);
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
            _ => false,
        }
    }

    fn csp_prefix_expr(e: &mut Expression, prop: &str, members: &HashSet<String>) -> bool {
        let through = |name: &str, span: crate::ast::Span| {
            Expression::new(
                ExprKind::MemberAccess {
                    expr: Box::new(Expression::new(
                        ExprKind::Ident(HierarchicalIdentifier {
                            root: None,
                            path: vec![crate::ast::expr::HierPathSegment {
                                name: crate::ast::Identifier {
                                    name: prop.to_string(),
                                    span,
                                },
                                selects: Vec::new(),
                            }],
                            span,
                            cached_signal_id: Cell::new(None),
                            cached_resolved_name: std::cell::OnceCell::new(),
                        }),
                        span,
                    )),
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
                if h.root.is_some() || h.path.len() != 1 || !h.path[0].selects.is_empty() {
                    return false;
                }
                let n = h.path[0].name.name.clone();
                if members.contains(&n) {
                    *e = through(&n, e.span);
                }
                true
            }
            ExprKind::MemberAccess { expr, member } => {
                if matches!(expr.kind, ExprKind::This) && members.contains(&member.name) {
                    let n = member.name.clone();
                    *e = through(&n, e.span);
                    return true;
                }
                false
            }
            ExprKind::Number(_) | ExprKind::StringLiteral(_) => true,
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                Self::csp_prefix_expr(operand, prop, members)
            }
            ExprKind::Binary { left, right, .. } | ExprKind::Range(left, right) => {
                Self::csp_prefix_expr(left, prop, members)
                    && Self::csp_prefix_expr(right, prop, members)
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

    fn csp_src(&self, csp: &mut Csp, item: SrcItem, env: &Env, deps: Vec<usize>) -> usize {
        csp.srcs.push(Src {
            item,
            binds: env.binds.clone(),
            deps,
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
        match item {
            ConstraintItem::Expr(e) => self.csp_bool(csp, e, env, false),
            ConstraintItem::Inside {
                expr,
                range,
                is_dist,
                dist_weights,
                ..
            } => {
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
                let n =
                    self.csp_inside(csp, expr, &set, env, false, SrcItem::Item(item.clone()))?;
                if *is_dist {
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
                let name = Self::foreach_base_name(array);
                let arr = name.as_ref().and_then(|n| csp.arrays.get(n)).cloned();
                let idx_var = match vars.as_slice() {
                    [Some(v)] => Some(v.name.clone()),
                    _ => None,
                };
                let (Some(arr), Some(iv)) = (arr, idx_var) else {
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
            ConstraintItem::Solve { before, after, .. } => {
                let vars = |names: &[crate::ast::Identifier]| -> Vec<usize> {
                    let mut out = Vec::new();
                    for id in names {
                        if let Some(&v) = csp.scalars.get(&id.name) {
                            out.push(v);
                        } else if let Some(a) = csp.arrays.get(&id.name) {
                            out.extend(a.elems.iter().map(|e| e.1));
                        }
                    }
                    out
                };
                let (b, a) = (vars(before), vars(after));
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
                Some(Node::AllDiff(vs))
            }
        }
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
                | BinaryOp::Geq => self.csp_rel(csp, e, *op, left, right, env, neg),
                _ => self.csp_eval_node(csp, SrcItem::Expr(e.clone()), env, neg),
            },
            ExprKind::Inside { expr, ranges } => {
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
            _ => self.csp_eval_node(csp, SrcItem::Expr(e.clone()), env, neg),
        }
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
        let inv = matches!(
            op,
            BinaryOp::Neq | BinaryOp::CaseNeq
        ) != neg;
        let mut fits = Vec::new();
        let lin = Self::csp_lin(&a, w, false, &mut fits)?;
        fits.push(Fit {
            lin: lin.clone(),
            lo: 0,
            hi: amax,
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
        })
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
        if let Some(n) = self.csp_shr_rel(csp, whole, op, left, right, env, neg) {
            return Some(n);
        }
        if matches!(op, BinaryOp::Eq | BinaryOp::CaseEq) && !neg {
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
        let s = sl && sr;
        let mut fits = Vec::new();
        let (Some(la), Some(lb)) = (
            Self::csp_lin(&a, w, s, &mut fits),
            Self::csp_lin(&b, w, s, &mut fits),
        ) else {
            return fallback(self, csp);
        };
        let (rlo, rhi) = ws_range(w, s);
        fits.push(Fit {
            lin: la.clone(),
            lo: rlo,
            hi: rhi,
        });
        fits.push(Fit {
            lin: lb.clone(),
            lo: rlo,
            hi: rhi,
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
        let mut deps: Vec<usize> = lin.t.iter().map(|t| t.0).collect();
        for f in &fits {
            deps.extend(f.lin.t.iter().map(|t| t.0));
        }
        deps.sort_unstable();
        deps.dedup();
        let src = self.csp_src(csp, SrcItem::Expr(whole.clone()), env, deps);
        Some(Node::Lin {
            lin,
            rel,
            fits,
            src,
            sneg: neg,
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
        let reads = |me: &Self, csp: &Csp, e: &Expression| -> Option<Vec<usize>> {
            let mut d = Vec::new();
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
        let mut fits = Vec::new();
        let lin = if w <= 64 {
            Self::csp_lin(&a, w, s, &mut fits)
        } else {
            None
        };
        let Some(lin) = lin else {
            return self.csp_eval_node(csp, whole, env, neg);
        };
        let frame = self.csp_frame(&env.binds);
        self.push_local_frame(frame);
        let set = dom_norm(self.cons_ranges_i128(ranges));
        self.pop_local_frame();
        // A negative endpoint makes the checker read an unsigned value as
        // signed; leave that reading to the checker itself.
        if !s && set.iter().any(|r| r.0 < 0) {
            return self.csp_eval_node(csp, whole, env, neg);
        }
        let (rlo, rhi) = ws_range(w, s);
        fits.push(Fit {
            lin: lin.clone(),
            lo: rlo,
            hi: rhi,
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
        })
    }

    /// The object's own member named by `e` (`x`, `this.x`, `obj.x` for the
    /// inline-constraint receiver), if it is a solver scalar or array.
    fn csp_member(&self, csp: &Csp, e: &Expression) -> Option<String> {
        let known = |n: &str| csp.scalars.contains_key(n) || csp.arrays.contains_key(n);
        let recv_ok = |me: &Self, r: &str| r == "this" || me.rand_receiver.as_deref() == Some(r);
        // A rand sub-object's member (`a.x`, `this.a.x`, `t.a.x`): a
        // `prop.name` variable when the sub-object joined the solve.
        if csp.has_subs {
            if let Some(mut path) = Self::csp_member_path(e) {
                if path.len() >= 3 && (path[0] == "this" || recv_ok(self, &path[0])) {
                    path.remove(0);
                }
                if path.len() == 2 {
                    let n = format!("{}.{}", path[0], path[1]);
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
        let push_name = |n: &str, out: &mut Vec<usize>| {
            if let Some(&v) = csp.scalars.get(n) {
                out.push(v);
            } else if let Some(a) = csp.arrays.get(n) {
                out.extend(a.elems.iter().map(|x| x.1));
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
                    }
                }
                true
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

    /// Arithmetic operand tree of `e`, or None when it is not linear.
    fn csp_ae(&mut self, csp: &Csp, e: &Expression, env: &Env) -> Option<Ae> {
        let e = Self::unparen(e);
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
                ("$signed", [inner]) | ("$unsigned", [inner]) => {
                    let a = self.csp_ae(csp, inner, env)?;
                    let (iw, _) = Self::csp_ws(&a);
                    return self.csp_cast(csp, inner, iw, Some(name == "$signed"), env);
                }
                _ => {}
            },
            _ => {}
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
                }
            }
        }
        if self.csp_free(csp, e, env) {
            return self.csp_const(e, env).map(Ae::Const);
        }
        None
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
        let arr = csp.arrays.get(&self.csp_member(csp, recv)?)?.clone();
        if method != "sum" {
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
    fn csp_lin(a: &Ae, w: u32, s: bool, fits: &mut Vec<Fit>) -> Option<Lin> {
        match a {
            Ae::Var(v, _, vs) => {
                // an unsigned context zero-extends a signed operand
                if *vs && !s {
                    fits.push(Fit {
                        lin: Lin::var(*v),
                        lo: 0,
                        hi: i128::MAX,
                    });
                }
                Some(Lin::var(*v))
            }
            Ae::Const(v) => {
                let raw = v.to_u64()?;
                Some(Lin::konst(Self::as_ctx(raw as i128, v.width, s)))
            }
            Ae::Neg(x) => Self::csp_lin(x, w, s, fits)?.scale(-1),
            Ae::Add(l, r) => Self::csp_lin(l, w, s, fits)?.plus(&Self::csp_lin(r, w, s, fits)?, 1),
            Ae::Sub(l, r) => Self::csp_lin(l, w, s, fits)?.plus(&Self::csp_lin(r, w, s, fits)?, -1),
            Ae::Mul(l, r) => {
                let (x, y) = (Self::csp_lin(l, w, s, fits)?, Self::csp_lin(r, w, s, fits)?);
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
                for t in terms {
                    acc = acc.plus(&Self::csp_lin(t, *sw, *ss, fits)?, 1)?;
                }
                Self::csp_leaf_fit(&acc, *sw, *ss, *sw, *ss, s, fits);
                Some(acc)
            }
            Ae::Cast {
                inner,
                iw,
                is,
                w: cw,
                s: cs,
            } => {
                let l = Self::csp_lin(inner, *iw, *is, fits)?;
                Self::csp_leaf_fit(&l, *iw, *is, *cw, *cs, s, fits);
                Some(l)
            }
        }
    }

    /// A sub-result computed at `(iw, is)`, converted to `(cw, cs)` and then
    /// extended into a context of signedness `ctx_s`, keeps its value when it
    /// lies in both ranges (and is non-negative when a signed value meets an
    /// unsigned context, which zero-extends).
    fn csp_leaf_fit(
        l: &Lin,
        iw: u32,
        is: bool,
        cw: u32,
        cs: bool,
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
        csp.fields.clear();
        csp.lsb.clear();
        csp.ascending.clear();
        let scalars: Vec<(String, usize)> =
            csp.scalars.iter().map(|(n, &v)| (n.clone(), v)).collect();
        for (name, v) in &scalars {
            if csp.vars[*v].signed || csp.vars[*v].width > 64 {
                continue;
            }
            if let Some((fields, lsb)) = self.csp_scalar_shape(csp, name) {
                if !fields.is_empty() {
                    csp.fields.insert(*v, fields);
                }
                if let Some((left, right)) = lsb {
                    csp.lsb.insert(*v, right);
                    if left < right {
                        csp.ascending.insert(*v);
                    }
                }
            } else {
                // Multi-dimensional packed ranges are not split here.
                csp.lsb.insert(*v, i64::MIN);
            }
        }
        // (base) -> constant ranges, and whether some select is dynamic.
        let mut uses: HashMap<usize, (Vec<(u32, u32)>, bool)> = HashMap::default();
        let mut bound: Vec<String> = Vec::new();
        for c in constraints {
            for it in &c.items {
                self.csp_scan_item(csp, it, &mut bound, &mut uses);
            }
        }
        for (v, (ranges, dynamic)) in uses {
            let w = csp.vars[v].width;
            if csp.vars[v].signed || w > 64 || w < 2 || csp.lsb.get(&v) == Some(&i64::MIN) {
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
                segs.push((lo, hi - lo, Self::csp_aux_var(csp, hi - lo)));
            }
            for &(lo, rw) in &ranges {
                let covered = segs.iter().filter(|s| s.0 >= lo && s.0 + s.1 <= lo + rw).count();
                if covered > 1 && !csp.slice_vars.contains_key(&(v, lo, rw)) {
                    let x = Self::csp_aux_var(csp, rw);
                    csp.slice_vars.insert((v, lo, rw), x);
                }
            }
            csp.segs.insert(v, segs);
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
    #[allow(clippy::type_complexity)]
    fn csp_scalar_shape(
        &mut self,
        csp: &Csp,
        name: &str,
    ) -> Option<(Vec<(String, u32, u32)>, Option<(i64, i64)>)> {
        let dt: Option<DataType> = if csp.scope.is_some() {
            if let Some(f) = self.module.packed_struct_fields.get(name) {
                return Some((f.clone(), None));
            }
            self.module.var_decl_types.get(name).cloned()
        } else {
            if let Some(su) = self.class_prop_struct(csp.handle, name) {
                if !su.packed {
                    return None;
                }
                let params = self.instance_param_scope(csp.handle);
                let (fields, _) = self.packed_agg_layout_with(&su, &params);
                return Some((fields, None));
            }
            let class = self
                .heap
                .get(csp.handle)
                .and_then(|o| o.as_ref())
                .map(|i| i.class_name.clone());
            let mut cur = class;
            let mut found = None;
            while let Some(cn) = cur {
                let Some(cd) = self.module.classes.get(&cn) else { break };
                if let Some(t) = self.class_prop_decl_type(cd, name) {
                    found = Some(t.clone());
                    break;
                }
                cur = cd.extends.clone();
            }
            found
        };
        let Some(dt) = dt else {
            return Some((Vec::new(), Some((0, 0))));
        };
        let dt = crate::compiler::elaborate::resolve_typedef_chain(&dt, &self.module.typedef_types).clone();
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
            ConstraintItem::Foreach { vars, item, .. } => {
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
        let mut konst = |me: &mut Self, x: &Expression| -> Option<i64> {
            let mut reads_bound = false;
            Self::walk_operands(x, &mut |y| {
                if let ExprKind::Ident(h) = &y.kind {
                    if h.path.len() == 1 && bound_now.iter().any(|b| *b == h.path[0].name.name) {
                        reads_bound = true;
                    }
                }
            });
            if reads_bound || !me.csp_free(csp, x, &Env::default()) {
                return None;
            }
            me.csp_const(x, &Env::default())?.to_i64()
        };
        if let Some((v, r)) = self.csp_select_ref(csp, e, &mut konst) {
            let u = uses.entry(v).or_default();
            match r {
                Some(r) => u.0.push(r),
                None => u.1 = true,
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
                    lo_hi: &mut dyn FnMut(&mut Self, &mut dyn FnMut(&mut Self, &Expression) -> Option<i64>) -> Option<Option<(i64, i64)>>|
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
                    && h.path[..h.path.len() - 1].iter().all(|s| s.selects.is_empty()) =>
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
        // `x`, `this.x`, `recv.x`: the scalar itself.
        if let Some(n) = self.csp_member(csp, e) {
            if let Some(v) = var_of(self, &n) {
                return Some((v, 0, csp.vars[v].width, true));
            }
        }
        match &e.kind {
            ExprKind::Ident(h) if h.root.is_none() && h.path.iter().all(|s| s.selects.is_empty()) => {
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
                let v = var_of(self, names[start])?;
                if names.len() == start + 1 {
                    return Some((v, 0, csp.vars[v].width, true));
                }
                if names.len() == start + 2 {
                    let (lo, w) = field(csp, v, names[start + 1])?;
                    return Some((v, lo, w, false));
                }
                None
            }
            ExprKind::MemberAccess { expr, member } => {
                let (v, lo, _, whole) = self.csp_slice_base_k(csp, expr, konst)?;
                if !whole {
                    return None;
                }
                let (flo, fw) = field(csp, v, &member.name)?;
                Some((v, lo + flo, fw, false))
            }
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
            let mut t: Vec<(usize, i128)> = vec![(v, 1)];
            for &(lo, _, x) in segs {
                t.push((x, -(1i128 << lo)));
            }
            links.push(Lin { t, k: 0 });
        }
        for (&(v, lo, w), &x) in &csp.slice_vars {
            let Some(segs) = csp.segs.get(&v) else { continue };
            let mut t: Vec<(usize, i128)> = vec![(x, 1)];
            for &(slo, sw, sx) in segs {
                if slo >= lo && slo + sw <= lo + w {
                    t.push((sx, -(1i128 << (slo - lo))));
                }
            }
            links.push(Lin { t, k: 0 });
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
