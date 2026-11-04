//! Can a class TASK body park its process? The gate for compiling a
//! wait-free task to bytecode.
//!
//! A compiled task runs synchronously inside `exec_insns`. If anything it
//! executes suspends (IEEE 1800-2023 §9.4 timing controls, §9.3.2
//! `fork…join` / `join_any`, §9.6.1 `wait fork`, §15.4 mailbox and §15.3
//! semaphore waits, §9.7 `process::await`, a DPI task, §13 a task callee
//! that does any of these), the suspension cannot resume into the compiled
//! frame. So the analysis is FAIL-CLOSED: `true` (suspends, keep the
//! interpreter) unless every statement and every callee is proven
//! wait-free.
//!
//! Callees are resolved through the receiver's DECLARED type: a class
//! handle reaches the nearest declaration in its chain plus every override
//! of it (§8.20 virtual dispatch); an interface-class handle, a mailbox /
//! semaphore / process handle, or a receiver whose type cannot be resolved
//! counts as suspending. Results are memoized per (scope, name); a result
//! computed while assuming that a subroutine still being analyzed (a call
//! cycle) does not suspend is only kept for the rest of that analysis and
//! never persisted, unless it is the answer for the cycle's root itself.

use super::*;

/// The declared type of a call receiver, as far as suspension goes.
#[derive(Clone, Debug)]
pub(super) enum TsRecv {
    /// A class handle of this declared class.
    Class(String),
    /// A handle of an `interface class`: any implementation may run.
    Interface,
    /// mailbox / semaphore / process: some of their methods wait.
    Builtin,
    /// A value with no task methods (integral, string, real, event,
    /// enum, struct).
    Plain,
    /// An unpacked array, queue or associative array: its own methods
    /// (§7.5–§7.10) never wait; an element (`arr[i].m()`) has the element
    /// type, `None` when that cannot be resolved.
    Collection(Option<Box<TsRecv>>),
}

/// One method/subroutine body under analysis: its class (None for a free
/// task or function), formals and body, where receiver names are looked up
/// on demand.
#[derive(Clone, Copy)]
struct TsCtx<'a> {
    owner: Option<&'a str>,
    ports: &'a [crate::ast::decl::FunctionPort],
    body: &'a [Statement],
}

/// A memo for a boolean property of subroutines that is the OR over their
/// bodies and callees ("can it wait"), safe on call cycles: a subroutine met
/// again while its own analysis is still running is assumed `false`, and a
/// `false` that relied on such an assumption is kept only while the
/// subroutine it assumed is still being analyzed (then dropped). `true`
/// never depends on an assumption and is always kept.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(super) struct CycleMemo {
    cache: std::cell::RefCell<HashMap<String, bool>>,
    /// Assumption-dependent `false`s: key -> (stack index, serial) of the
    /// earliest in-progress subroutine assumed.
    tentative: std::cell::RefCell<HashMap<String, (usize, u64)>>,
    /// Subroutines being analyzed, outermost first, with a unique serial.
    stack: std::cell::RefCell<Vec<(String, u64)>>,
    serial: std::cell::Cell<u64>,
    /// Lowest stack index the current computation assumed `false` for.
    low: std::cell::Cell<usize>,
}

impl CycleMemo {
    pub(super) fn get(&self, key: &str, compute: impl FnOnce() -> bool) -> bool {
        if let Some(&b) = self.cache.borrow().get(key) {
            return b;
        }
        let in_progress = self.stack.borrow().iter().position(|(k, _)| k == key);
        if let Some(i) = in_progress {
            self.low.set(self.low.get().min(i));
            return false;
        }
        let tentative = self.tentative.borrow().get(key).copied();
        if let Some((i, serial)) = tentative {
            if self
                .stack
                .borrow()
                .get(i)
                .is_some_and(|(_, s)| *s == serial)
            {
                self.low.set(self.low.get().min(i));
                return false;
            }
            self.tentative.borrow_mut().remove(key);
        }
        let depth = self.stack.borrow().len();
        let serial = self.serial.get() + 1;
        self.serial.set(serial);
        self.stack.borrow_mut().push((key.to_string(), serial));
        let outer_low = self.low.replace(usize::MAX);
        let r = compute();
        self.stack.borrow_mut().pop();
        let low = self.low.get();
        if r || low >= depth {
            // `true` holds whatever was assumed; `false` with no assumption
            // about an ancestor is the cycle's own fixpoint.
            self.cache.borrow_mut().insert(key.to_string(), r);
            self.low.set(outer_low);
        } else {
            let at = self.stack.borrow()[low].1;
            self.tentative
                .borrow_mut()
                .insert(key.to_string(), (low, at));
            self.low.set(outer_low.min(low));
        }
        r
    }
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(super) struct TsState {
    /// Verdicts keyed `c:<class>::<name>` / `f:<name>`.
    memo: CycleMemo,
}

/// Methods every class handle has (§18.5, §18.8, §18.13): none waits.
const CLASS_BUILTIN_METHODS: &[&str] = &[
    "randomize",
    "srandom",
    "get_randstate",
    "set_randstate",
    "rand_mode",
    "constraint_mode",
    "pre_randomize",
    "post_randomize",
    "new",
];

impl Simulator {
    /// Can running this class method body (task or function) suspend? The
    /// entry point for the compile gate.
    pub(super) fn method_body_can_suspend(
        &self,
        cname: &str,
        ports: &[crate::ast::decl::FunctionPort],
        body: &[Statement],
    ) -> bool {
        let cx = TsCtx {
            owner: Some(cname),
            ports,
            body,
        };
        self.ts_stmts(body, cx)
    }

    /// The receiver kind of `name` in `cx`: formal, then a local declared
    /// anywhere in the body, then a property of the owning class chain.
    /// Outer `None`: no such name here; inner `None`: its type is unknown.
    fn ts_lookup(&self, name: &str, cx: TsCtx<'_>) -> Option<Option<TsRecv>> {
        let wrap = |k: Option<TsRecv>, collection: bool| {
            if collection {
                Some(TsRecv::Collection(k.map(Box::new)))
            } else {
                k
            }
        };
        if let Some(p) = cx.ports.iter().find(|p| p.name.name == name) {
            return Some(wrap(
                self.ts_recv_kind(&p.data_type),
                !p.dimensions.is_empty(),
            ));
        }
        let mut todo: Vec<&Statement> = cx.body.iter().collect();
        while let Some(st) = todo.pop() {
            if let StatementKind::VarDecl {
                data_type,
                declarators,
                ..
            } = &st.kind
                && let Some(d) = declarators.iter().find(|d| d.name.name == name)
            {
                return Some(wrap(self.ts_recv_kind(data_type), !d.dimensions.is_empty()));
            }
            todo.extend(Self::ts_children(st));
        }
        let mut cur = Self::ts_base(cx.owner?).to_string();
        let mut seen: HashSet<String> = HashSet::default();
        while let Some(cd) = self.module.classes.get(&cur) {
            if !seen.insert(cur.clone()) {
                break;
            }
            if let Some(dt) = cd.property_types.get(name) {
                let collection = cd.array_properties.contains_key(name)
                    || cd.queue_properties.contains_key(name)
                    || cd.assoc_properties.contains_key(name)
                    || cd.static_collections.iter().any(|(n, _, _)| n == name);
                return Some(wrap(self.ts_recv_kind(dt), collection));
            }
            match &cd.extends {
                Some(b) => cur = Self::ts_base(b).to_string(),
                None => break,
            }
        }
        None
    }

    fn ts_base(name: &str) -> &str {
        name.split('#').next().unwrap_or(name).trim()
    }

    /// The receiver kind of a declared type; `None` when it cannot be
    /// resolved (a type parameter, an unknown name).
    fn ts_recv_kind(&self, dt: &crate::ast::types::DataType) -> Option<TsRecv> {
        use crate::ast::types::DataType as DT;
        let mut cur = dt;
        for _ in 0..16 {
            let DT::TypeReference { name: tn, .. } = cur else {
                return Some(TsRecv::Plain);
            };
            let n = tn.name.name.as_str();
            if Self::container_base(n).is_some() {
                return Some(TsRecv::Builtin);
            }
            if Self::ts_base(n) == "process" || n.ends_with("::process") {
                return Some(TsRecv::Builtin);
            }
            if self.module.typedef_unpacked_dims.contains_key(n) {
                let elem = self
                    .module
                    .typedef_types
                    .get(n)
                    .and_then(|e| self.ts_recv_kind(e))
                    .map(Box::new);
                return Some(TsRecv::Collection(elem));
            }
            if let Some(c) = self.typeref_class_name(cur) {
                let interface = self
                    .module
                    .classes
                    .get(&c)
                    .is_some_and(|cd| cd.is_interface);
                return Some(if interface {
                    TsRecv::Interface
                } else {
                    TsRecv::Class(c)
                });
            }
            match self.module.typedef_types.get(n) {
                Some(next) => cur = next,
                None => return None,
            }
        }
        None
    }

    /// Every statement directly nested in `st`, for every kind that has a
    /// body (the walk must not miss one, or a wait inside it goes unseen).
    fn ts_children(st: &Statement) -> Vec<&Statement> {
        let mut out: Vec<&Statement> = Vec::new();
        match &st.kind {
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                out.push(then_stmt);
                if let Some(e) = else_stmt {
                    out.push(e);
                }
            }
            StatementKind::Case { items, .. } => out.extend(items.iter().map(|it| &it.stmt)),
            StatementKind::For { body, .. }
            | StatementKind::Foreach { body, .. }
            | StatementKind::ForeachTail { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::ForeverTail { body }
            | StatementKind::RsAction { body }
            | StatementKind::TimingControl { stmt: body, .. }
            | StatementKind::Wait { stmt: body, .. } => out.push(body),
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                out.extend(stmts.iter())
            }
            StatementKind::RandCase { items } => out.extend(items.iter().map(|(_, s)| s)),
            StatementKind::Assertion(a) => {
                out.extend(a.action.as_deref());
                out.extend(a.else_action.as_deref());
            }
            StatementKind::WaitOrder { pass, fail, .. } => {
                out.extend(pass.as_deref());
                out.extend(fail.as_deref());
            }
            StatementKind::Null
            | StatementKind::ScopePop
            | StatementKind::LoopStep
            | StatementKind::DisableFork
            | StatementKind::Expr(_)
            | StatementKind::BlockingAssign { .. }
            | StatementKind::NonblockingAssign { .. }
            | StatementKind::EventTrigger { .. }
            | StatementKind::WaitFork
            | StatementKind::Disable(_)
            | StatementKind::Return(_)
            | StatementKind::Break
            | StatementKind::Continue
            | StatementKind::ProceduralContinuous(_)
            | StatementKind::VarDecl { .. }
            | StatementKind::Typedef(_)
            | StatementKind::Coverpoint { .. }
            | StatementKind::Cross { .. }
            | StatementKind::RsReturn => {}
        }
        out
    }

    fn ts_stmts(&self, stmts: &[Statement], cx: TsCtx<'_>) -> bool {
        stmts.iter().any(|s| self.ts_stmt(s, cx))
    }

    fn ts_stmt(&self, st: &Statement, cx: TsCtx<'_>) -> bool {
        let here = match &st.kind {
            StatementKind::TimingControl { .. }
            | StatementKind::Wait { .. }
            | StatementKind::WaitOrder { .. }
            | StatementKind::WaitFork => true,
            // §9.3.2: join / join_any park the parent; join_none children
            // are separate processes.
            StatementKind::ParBlock { join_type, .. } => {
                return !matches!(join_type, JoinType::JoinNone);
            }
            StatementKind::BlockingAssign { rvalue, .. } => Self::intra_timing_suspends(rvalue),
            StatementKind::Expr(e) => Self::expr_is_proc_await(e) || self.ts_call(e, cx),
            _ => false,
        };
        here || Self::ts_children(st)
            .into_iter()
            .any(|c| self.ts_stmt(c, cx))
    }

    /// A statement-position call (a task enable or a void function call).
    fn ts_call(&self, expr: &Expression, cx: TsCtx<'_>) -> bool {
        let mut e = expr;
        while let ExprKind::Paren(inner) = &e.kind {
            e = inner;
        }
        let callee = match &e.kind {
            ExprKind::Call { func, .. } => &**func,
            ExprKind::Ident(_) | ExprKind::MemberAccess { .. } => e,
            // A cast (`void'(f())`) or any other expression evaluates
            // functions only, which cannot wait.
            _ => return false,
        };
        match &callee.kind {
            ExprKind::Ident(h) => {
                if h.root.is_some() {
                    return true;
                }
                match h.path.as_slice() {
                    [m] if m.selects.is_empty() => self.ts_bare(&m.name.name, cx),
                    [r, m] if m.selects.is_empty() => {
                        self.ts_on_named(&r.name.name, !r.selects.is_empty(), &m.name.name, cx)
                    }
                    _ => true,
                }
            }
            ExprKind::MemberAccess { expr: recv, member } => {
                let name = member.name.as_str();
                let (base, selected) = match &recv.kind {
                    ExprKind::Index { expr, .. } => (&**expr, true),
                    _ => (&**recv, false),
                };
                match &base.kind {
                    ExprKind::This if !selected => self.ts_on_named("this", false, name, cx),
                    ExprKind::Ident(h) if h.root.is_none() && h.path.len() == 1 => self
                        .ts_on_named(
                            &h.path[0].name.name,
                            selected || !h.path[0].selects.is_empty(),
                            name,
                            cx,
                        ),
                    _ => {
                        // A member chain (`base.member.method()` — e.g.
                        // `m_base_inst.m_this_type.push_back()`): resolve
                        // the receiver chain's type when every link is
                        // known, else fail closed.
                        let Some(kind) = self.ts_chain_recv(base, cx) else {
                            return true;
                        };
                        let kind = match self.ts_collapse(Some(kind), selected) {
                            Some(k) => k,
                            None => return true,
                        };
                        self.ts_on_kind(kind, name)
                    }
                }
            }
            _ => true,
        }
    }

    /// The verdict for calling `name` on a receiver of handle kind `kind`.
    fn ts_on_kind(&self, kind: TsRecv, name: &str) -> bool {
        match kind {
            TsRecv::Class(c) => self.ts_in_class(&c, name),
            TsRecv::Plain | TsRecv::Collection(_) => false,
            TsRecv::Builtin => !matches!(
                name,
                "try_get"
                    | "try_put"
                    | "try_peek"
                    | "num"
                    | "kill"
                    | "status"
                    | "self"
                    | "srandom"
                    | "get_randstate"
                    | "set_randstate"
            ),
            TsRecv::Interface => true,
        }
    }

    /// The handle kind of a receiver expression — `this`, a
    /// local/formal/property name, a member chain (`obj.member.x`), or
    /// an element select of any of those — collapsing inner selects.
    /// `None` when any link's type is unknown (fail closed).
    fn ts_chain_recv(&self, e: &Expression, cx: TsCtx<'_>) -> Option<TsRecv> {
        match &e.kind {
            ExprKind::Paren(inner) => self.ts_chain_recv(inner, cx),
            ExprKind::Index { expr, .. } => {
                let k = self.ts_chain_recv(expr, cx)?;
                self.ts_collapse(Some(k), true)
            }
            ExprKind::This => Some(TsRecv::Class(cx.owner?.to_string())),
            ExprKind::Ident(h)
                if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty() =>
            {
                if let Some(k) = self.ts_lookup(&h.path[0].name.name, cx) {
                    return k;
                }
                // A package-scope variable (uvm_pkg data): a queue /
                // dynamic array (element kind from its element type name)
                // or a signal, whose type name may name a class.
                self.ts_pkg_var_kind(&h.path[0].name.name)
            }
            ExprKind::MemberAccess {
                expr: inner,
                member,
            } => {
                let TsRecv::Class(c) = self.ts_chain_recv(inner, cx)? else {
                    return None;
                };
                let (kind, collection) = self.ts_prop_kind(&c, &member.name)?;
                if collection {
                    Some(TsRecv::Collection(kind.map(Box::new)))
                } else {
                    kind
                }
            }
            _ => None,
        }
    }

    /// The handle kind of a package-scope variable: a queue / dynamic
    /// array (element kind from its element type name), or a signal whose
    /// type name names a class. `None` when the name is unknown.
    fn ts_pkg_var_kind(&self, n: &str) -> Option<TsRecv> {
        if self.module.dynamic_arrays.contains(n) {
            let elem = self
                .module
                .signals
                .get(n)
                .and_then(|s| s.type_name.clone())
                .and_then(|tn| self.ts_type_name_kind(&tn));
            return Some(TsRecv::Collection(elem.map(Box::new)));
        }
        self.module
            .signals
            .get(n)
            .and_then(|s| s.type_name.clone())
            .and_then(|tn| self.ts_type_name_kind(&tn))
    }

    /// The handle kind of a declared type NAME (a `signals` type name or
    /// similar): the class it names (or its typedef alias), else a plain
    /// value. `None` when the name names no known type.
    fn ts_type_name_kind(&self, tn: &str) -> Option<TsRecv> {
        let base = tn.rsplit("::").next().unwrap_or(tn);
        let base = base.split('#').next().unwrap_or(base).trim();
        if let Some(cd) = self.module.classes.get(base) {
            return Some(if cd.is_interface {
                TsRecv::Interface
            } else {
                TsRecv::Class(base.to_string())
            });
        }
        if let Some(dt) = self.module.typedef_types.get(tn) {
            return self.ts_recv_kind(dt);
        }
        if let Some(dt) = self.module.typedef_types.get(base) {
            return self.ts_recv_kind(dt);
        }
        None
    }

    /// An element/part select of `kind`: the element type of a
    /// collection, a plain integral of a plain value, else unknown.
    fn ts_collapse(&self, kind: Option<TsRecv>, selected: bool) -> Option<TsRecv> {
        match (kind, selected) {
            (Some(TsRecv::Collection(elem)), true) => elem.map(|b| *b),
            (Some(TsRecv::Plain), true) => Some(TsRecv::Plain),
            (_, true) => None,
            (k, false) => k,
        }
    }

    /// The property `prop` of class `class`'s inheritance chain: its
    /// handle kind and whether it is a collection. `None` when the
    /// chain lacks the property or cannot be followed.
    fn ts_prop_kind(&self, class: &str, prop: &str) -> Option<(Option<TsRecv>, bool)> {
        let mut cur = Self::ts_base(class).to_string();
        let mut seen: HashSet<String> = HashSet::default();
        while let Some(cd) = self.module.classes.get(&cur) {
            if !seen.insert(cur.clone()) {
                return None;
            }
            if let Some(dt) = cd.property_types.get(prop) {
                let collection = cd.array_properties.contains_key(prop)
                    || cd.queue_properties.contains_key(prop)
                    || cd.assoc_properties.contains_key(prop)
                    || cd.static_collections.iter().any(|(n, _, _)| n == prop);
                return Some((self.ts_recv_kind(dt), collection));
            }
            match &cd.extends {
                Some(b) => cur = Self::ts_base(b).to_string(),
                None => return None,
            }
        }
        None
    }

    /// `name(...)` with no receiver: the enclosing class chain first
    /// (§8.23 class scope), then the free tasks and functions.
    fn ts_bare(&self, name: &str, cx: TsCtx<'_>) -> bool {
        if let Some(c) = cx.owner
            && self.ts_chain_declares(c, name).is_some()
        {
            return self.ts_in_class(c, name);
        }
        if CLASS_BUILTIN_METHODS.contains(&name) {
            return false;
        }
        self.ts_free(name)
    }

    /// `recv.name(...)`, `recv[i].name(...)` (`selected`) or
    /// `Scope::name(...)`.
    fn ts_on_named(&self, recv: &str, selected: bool, name: &str, cx: TsCtx<'_>) -> bool {
        let owner = cx.owner;
        let kind = if matches!(recv, "this" | "super") {
            None
        } else {
            match self.ts_lookup(recv, cx) {
                Some(None) => return true,
                Some(k) => k,
                None => None,
            }
        };
        let kind = match (kind, selected) {
            // An element of a collection has the element type.
            (Some(TsRecv::Collection(elem)), true) => match elem {
                Some(k) => Some(*k),
                None => return true,
            },
            // A bit or part select of an integral value.
            (Some(TsRecv::Plain), true) => Some(TsRecv::Plain),
            (_, true) => return true,
            (k, false) => k,
        };
        match (recv, kind) {
            ("this", _) => match owner {
                Some(c) => self.ts_in_class(c, name),
                None => true,
            },
            ("super", _) => {
                let parent = owner
                    .and_then(|c| self.module.classes.get(Self::ts_base(c)))
                    .and_then(|cd| cd.extends.as_deref())
                    .map(|p| Self::ts_base(p).to_string());
                match parent {
                    Some(p) => self.ts_in_class(&p, name),
                    None => true,
                }
            }
            (_, Some(TsRecv::Class(c))) => self.ts_in_class(&c, name),
            (_, Some(TsRecv::Plain)) | (_, Some(TsRecv::Collection(_))) => false,
            (_, Some(TsRecv::Builtin)) => !matches!(
                name,
                "try_get"
                    | "try_put"
                    | "try_peek"
                    | "num"
                    | "kill"
                    | "status"
                    | "self"
                    | "srandom"
                    | "get_randstate"
                    | "set_randstate"
            ),
            (_, Some(TsRecv::Interface)) => true,
            (_, None) => {
                // `Class::m()` (§8.23 static call) or `pkg::t()` (§26.3).
                if self.module.classes.contains_key(recv) {
                    return self.ts_in_class(recv, name);
                }
                let q = format!("{recv}::{name}");
                if self.module.tasks.contains_key(&q) || self.module.functions.get(&q).is_some() {
                    return self.ts_free(&q);
                }
                // `typedef_name::m()` (§8.23 static call through a class
                // typedef, e.g. a parameterized `uvm_callbacks`
                // specialization aliased as `cbs_type`). Resolve the typedef
                // to its class before failing closed.
                if let Some(dt) = self.module.typedef_types.get(recv)
                    && let Some(c) = self.typeref_class_name(dt)
                {
                    return self.ts_in_class(&c, name);
                }
                // A package-scope variable receiver (`m_state.push_front()`:
                // uvm_pkg data reached from a class method).
                if let Some(kind) = self.ts_pkg_var_kind(recv) {
                    return self.ts_on_kind(kind, name);
                }
                true
            }
        }
    }

    /// Nearest class from `base` up that declares `name`. `None` also when
    /// the chain cannot be followed to its root.
    fn ts_chain_declares(&self, base: &str, name: &str) -> Option<String> {
        let mut cur = Self::ts_base(base).to_string();
        let mut seen: HashSet<String> = HashSet::default();
        while let Some(cd) = self.module.classes.get(&cur) {
            if !seen.insert(cur.clone()) {
                return None;
            }
            if cd.methods.contains_key(name) {
                return Some(cur);
            }
            match &cd.extends {
                Some(b) => cur = Self::ts_base(b).to_string(),
                None => return None,
            }
        }
        None
    }

    /// Is `anc` `c` itself, a class `c` extends, or an interface class `c`
    /// implements (directly or through its bases and interfaces)?
    fn ts_is_a(&self, c: &str, anc: &str) -> bool {
        let mut todo: Vec<String> = vec![Self::ts_base(c).to_string()];
        let mut seen: HashSet<String> = HashSet::default();
        while let Some(cur) = todo.pop() {
            if cur == anc {
                return true;
            }
            if !seen.insert(cur.clone()) {
                continue;
            }
            if let Some(cd) = self.module.classes.get(&cur) {
                if let Some(b) = &cd.extends {
                    todo.push(Self::ts_base(b).to_string());
                }
                todo.extend(cd.implements.iter().map(|i| Self::ts_base(i).to_string()));
            }
        }
        false
    }

    /// A call of `name` through a handle whose declared class is `base`.
    fn ts_in_class(&self, base: &str, name: &str) -> bool {
        let base = Self::ts_base(base);
        if self
            .module
            .classes
            .get(base)
            .is_some_and(|cd| cd.is_interface)
        {
            return true;
        }
        let key = format!("c:{base}::{name}");
        self.ts_memo(key, |s| {
            let Some(d) = s.ts_chain_declares(base, name) else {
                // Not declared anywhere up the chain: a built-in class
                // method, or a chain we cannot see to its root.
                return !CLASS_BUILTIN_METHODS.contains(&name);
            };
            // §8.20: the declaration in `d` and every override of it in a
            // class that extends (or implements) `d`.
            let mut impls: Vec<(&str, &crate::ast::decl::ClassMethod)> = Vec::new();
            for (cn, cd) in s.module.classes.iter() {
                if let Some(m) = cd.methods.get(name)
                    && s.ts_is_a(cn, &d)
                {
                    impls.push((cn.as_str(), m));
                }
            }
            impls.into_iter().any(|(cn, m)| {
                use crate::ast::decl::ClassMethodKind as K;
                match &m.kind {
                    K::Function(f) | K::Extern(f) => {
                        s.method_body_can_suspend(cn, &f.ports, &f.items)
                    }
                    // A prototype has no body of its own to run.
                    K::PureVirtual(_) => false,
                    K::Task(t) => s.method_body_can_suspend(cn, &t.ports, &t.items),
                }
            })
        })
    }

    /// A free (module / package / `$unit`) subroutine called by `name`.
    fn ts_free(&self, name: &str) -> bool {
        let key = format!("f:{name}");
        self.ts_memo(key, |s| {
            if let Some(t) = s.module.tasks.get(name) {
                let cx = TsCtx {
                    owner: None,
                    ports: &t.ports,
                    body: &t.items,
                };
                return s.ts_stmts(&t.items, cx);
            }
            if let Some(f) = s.module.functions.get(name) {
                let cx = TsCtx {
                    owner: None,
                    ports: &f.ports,
                    body: &f.items,
                };
                return s.ts_stmts(&f.items, cx);
            }
            // §35.5: an imported DPI function cannot consume time; an
            // imported task can (through exported tasks). Anything else is
            // unknown here.
            !s.module
                .dpi_imports
                .get(name)
                .is_some_and(|spec| matches!(spec.proto, crate::ast::decl::DPIProto::Function(_)))
        })
    }

    fn ts_memo(&self, key: String, compute: impl FnOnce(&Self) -> bool) -> bool {
        self.ts.memo.get(&key, || compute(self))
    }
}
