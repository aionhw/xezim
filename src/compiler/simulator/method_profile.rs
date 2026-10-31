//! Opt-in method diagnostics. Calls, execution entries and wall samples are
//! different measurements; none of them changes compilation or scheduling.

use super::*;
use serde::Serialize;
use std::sync::atomic::AtomicU64;

pub(super) static SAMPLER_READY: AtomicBool = AtomicBool::new(false);
// One atomic publishes BOTH identity and execution path. Two separate loads
// could combine a callee's identity with its caller's path during a switch.
static CURRENT: AtomicU64 = AtomicU64::new(0);
static SAMPLES: OnceLock<Mutex<BTreeMap<u64, [u64; 4]>>> = OnceLock::new();

#[derive(Clone, Copy)]
pub(super) enum Path {
    Setup = 0,
    Ast = 1,
    Vm = 2,
    Accessor = 3,
}

#[derive(Clone, Default, Serialize)]
struct Counters {
    calls: u64,
    ast_body_entries: u64,
    vm_body_entries: u64,
    accessor_calls: u64,
    scheduled_segments: u64,
    compile_attempts: u64,
    compile_successes: u64,
    persisted_blocks: u64,
    tier_pending: u64,
    compilation_disabled: u64,
    cached_rejections: u64,
    compilation_bypassed: BTreeMap<String, u64>,
    declines: BTreeMap<String, u64>,
    cached_decline_reasons: BTreeMap<String, u64>,
}

#[derive(Clone, Serialize)]
struct Row {
    id: u64,
    class: String,
    method: String,
    specialization: Option<(String, String)>,
    #[serde(flatten)]
    counters: Counters,
    setup_samples: u64,
    ast_samples: u64,
    vm_samples: u64,
    accessor_samples: u64,
    #[serde(skip)]
    last_decline: Option<&'static str>,
}

type Signatures = HashMap<String, u64>;
type Specs = HashMap<String, Signatures>;
type Methods = HashMap<String, Specs>;

#[derive(Default)]
struct State {
    ids: HashMap<String, Methods>,
    rows: Vec<Row>,
}

impl State {
    fn id(&mut self, class: &str, method: &str, spec: Option<&(String, String)>) -> u64 {
        let (base, sig) = spec.map_or(("", ""), |(b, s)| (b.as_str(), s.as_str()));
        if let Some(id) = self
            .ids
            .get(class)
            .and_then(|m| m.get(method))
            .and_then(|m| m.get(base))
            .and_then(|m| m.get(sig))
        {
            return *id;
        }
        let id = self.rows.len() as u64 + 1;
        self.ids
            .entry(class.into())
            .or_default()
            .entry(method.into())
            .or_default()
            .entry(base.into())
            .or_default()
            .insert(sig.into(), id);
        self.rows.push(Row {
            id,
            class: class.into(),
            method: method.into(),
            specialization: spec.cloned(),
            counters: Counters::default(),
            setup_samples: 0,
            ast_samples: 0,
            vm_samples: 0,
            accessor_samples: 0,
            last_decline: None,
        });
        id
    }

    fn row(&mut self, id: u64) -> Option<&mut Row> {
        self.rows.get_mut(id.checked_sub(1)? as usize)
    }

    fn reset_counts(&mut self) {
        // IDs survive repeated run windows: parked task cleanup frames carry
        // them. Reusing an ID for another method would mislabel its resume.
        for row in &mut self.rows {
            row.counters = Counters::default();
            row.setup_samples = 0;
            row.ast_samples = 0;
            row.vm_samples = 0;
            row.accessor_samples = 0;
            // A new simulator may reuse this thread and the same labels.
            // Do not carry a rejection reason across collection windows.
            row.last_decline = None;
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

pub(super) fn task_package(func: &Expression) -> Option<&str> {
    match &func.kind {
        ExprKind::Ident(h) if h.path.len() == 2 => Some(&h.path[0].name.name),
        ExprKind::MemberAccess { expr, .. } => match &expr.kind {
            ExprKind::Ident(h) if h.path.len() == 1 => Some(&h.path[0].name.name),
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn requested() -> bool {
    std::env::var_os("XEZIM_METHOD_PROFILE").is_some()
        || std::env::var_os("XEZIM_METHOD_PROFILE_JSON").is_some()
}

pub(super) fn identity(class: &str, method: &str, spec: Option<&(String, String)>) -> u64 {
    if !SAMPLER_READY.load(Ordering::Relaxed) {
        return 0;
    }
    STATE.with(|s| s.borrow_mut().id(class, method, spec))
}

pub(super) enum Decision {
    TierPending,
    Disabled,
    CompileAttempt,
    CompileSuccess,
    PersistedBlock,
    Decline(&'static str),
    CachedRejection,
    CachedBodyRejection,
    Bypassed(&'static str),
}

pub(super) fn decision(id: u64, decision: Decision) {
    if id == 0 {
        return;
    }
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        let Some(row) = state.row(id) else {
            return;
        };
        row.decision(decision);
    });
}

impl Row {
    fn decision(&mut self, decision: Decision) {
        let c = &mut self.counters;
        match decision {
            Decision::Bypassed(reason) => {
                *c.compilation_bypassed.entry(reason.into()).or_default() += 1;
            }
            Decision::TierPending => c.tier_pending += 1,
            Decision::Disabled => c.compilation_disabled += 1,
            Decision::CompileAttempt => c.compile_attempts += 1,
            Decision::CompileSuccess => c.compile_successes += 1,
            Decision::PersistedBlock => c.persisted_blocks += 1,
            Decision::Decline(reason) => {
                *c.declines.entry(reason.into()).or_default() += 1;
                self.last_decline = Some(reason);
            }
            Decision::CachedBodyRejection => {
                c.cached_rejections += 1;
                *c.cached_decline_reasons
                    .entry("body_cache_rejection".into())
                    .or_default() += 1;
            }
            Decision::CachedRejection => {
                c.cached_rejections += 1;
                *c.cached_decline_reasons
                    .entry(self.last_decline.unwrap_or("unknown_cached_reason").into())
                    .or_default() += 1;
            }
        }
    }
}

fn word(id: u64, path: Path) -> u64 {
    if id == 0 { 0 } else { (id << 2) | path as u64 }
}

/// Contains no simulator borrow and is never stored in a parked continuation.
pub(super) struct Scope {
    previous: u64,
}

impl Scope {
    pub(super) fn enter(id: u64, path: Path, call: bool) -> Option<Self> {
        if id == 0 || !SAMPLER_READY.load(Ordering::Relaxed) {
            return None;
        }
        STATE.with(|s| {
            if let Some(row) = s.borrow_mut().row(id) {
                row.counters.calls += u64::from(call);
                match path {
                    Path::Vm => row.counters.vm_body_entries += 1,
                    Path::Ast => row.counters.ast_body_entries += 1,
                    Path::Accessor => row.counters.accessor_calls += 1,
                    Path::Setup => {}
                }
            }
        });
        Some(Self {
            previous: CURRENT.swap(word(id, path), Ordering::Relaxed),
        })
    }

    pub(super) fn process(id: u64) -> Option<Self> {
        if !SAMPLER_READY.load(Ordering::Relaxed) {
            return None;
        }
        if id != 0 {
            STATE.with(|s| {
                if let Some(row) = s.borrow_mut().row(id) {
                    row.counters.scheduled_segments += 1;
                }
            });
        }
        Some(Self {
            previous: CURRENT.swap(word(id, Path::Ast), Ordering::Relaxed),
        })
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.store(self.previous, Ordering::Relaxed);
    }
}

pub(super) fn process_method(id: u64) {
    CURRENT.store(word(id, Path::Ast), Ordering::Relaxed);
}

pub(super) fn ast_entry(id: u64) {
    if id != 0 && SAMPLER_READY.load(Ordering::Relaxed) {
        STATE.with(|s| {
            if let Some(row) = s.borrow_mut().row(id) {
                row.counters.ast_body_entries += 1;
            }
        });
    }
}

pub fn sampler_sample() {
    let current = CURRENT.load(Ordering::Relaxed);
    let id = current >> 2;
    let path = (current & 3) as usize;
    SAMPLES
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .entry(id)
        .or_default()[path] += 1;
}

pub fn sampler_reset() {
    SAMPLER_READY.store(false, Ordering::Relaxed);
    CURRENT.store(0, Ordering::Relaxed);
    STATE.with(|s| s.borrow_mut().reset_counts());
    SAMPLES
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .clear();
}

pub fn sampler_report(top: usize) {
    SAMPLER_READY.store(false, Ordering::Relaxed);
    CURRENT.store(0, Ordering::Relaxed);
    let samples = SAMPLES.get_or_init(Default::default).lock().unwrap();
    let mut rows = STATE.with(|s| s.borrow().rows.clone());
    for row in &mut rows {
        let n = samples.get(&row.id).copied().unwrap_or_default();
        [
            row.setup_samples,
            row.ast_samples,
            row.vm_samples,
            row.accessor_samples,
        ] = n;
    }
    rows.retain(|r| {
        r.counters.calls > 0
            || r.counters.scheduled_segments > 0
            || r.counters.ast_body_entries > 0
            || r.counters.vm_body_entries > 0
            || r.counters.cached_rejections > 0
            || r.counters.compile_attempts > 0
            || r.counters.tier_pending > 0
            || r.counters.compilation_disabled > 0
            || !r.counters.declines.is_empty()
            || r.setup_samples + r.ast_samples + r.vm_samples + r.accessor_samples > 0
    });
    rows.sort_by(|a, b| {
        let total = |r: &Row| r.setup_samples + r.ast_samples + r.vm_samples + r.accessor_samples;
        total(b)
            .cmp(&total(a))
            .then_with(|| b.counters.calls.cmp(&a.counters.calls))
            .then_with(|| {
                (&a.class, &a.method, &a.specialization).cmp(&(
                    &b.class,
                    &b.method,
                    &b.specialization,
                ))
            })
    });
    eprintln!("===== METHOD SAMPLE HISTOGRAM (top {top}; wall samples, not calls) =====");
    for row in rows.iter().take(top) {
        let spec = row
            .specialization
            .as_ref()
            .map(|(b, s)| format!("[{b}#({s})]"))
            .unwrap_or_default();
        let label = format!("{}{spec}::{}", row.class, row.method);
        eprintln!(
            "{:>8} AST {:>8} VM  {}",
            row.ast_samples, row.vm_samples, label
        );
        let c = &row.counters;
        eprintln!(
            "[METHOD] calls={} ast_entries={} vm_entries={} accessor_calls={} setup_samples={} accessor_samples={} scheduled_segments={} tier_pending={} cached_rejections={} compile_attempts={} compile_successes={} compilation_disabled={} persisted_blocks={} scope={}",
            c.calls,
            c.ast_body_entries,
            c.vm_body_entries,
            c.accessor_calls,
            row.setup_samples,
            row.accessor_samples,
            c.scheduled_segments,
            c.tier_pending,
            c.cached_rejections,
            c.compile_attempts,
            c.compile_successes,
            c.compilation_disabled,
            c.persisted_blocks,
            label
        );
        for (reason, count) in &c.compilation_bypassed {
            eprintln!(
                "[METHOD] compilation_bypassed={} count={} scope={}",
                reason, count, label
            );
        }
        for (reason, count) in &c.declines {
            eprintln!(
                "[METHOD] decline={} count={} scope={}",
                reason, count, label
            );
        }
        for (reason, count) in &c.cached_decline_reasons {
            eprintln!(
                "[METHOD] cached_decline={} count={} scope={}",
                reason, count, label
            );
        }
    }
    if let Some(path) = std::env::var_os("XEZIM_METHOD_PROFILE_JSON") {
        #[derive(Serialize)]
        struct Report<'a> {
            schema: u32,
            scope: &'static str,
            sample_kind: &'static str,
            sample_interval_ms: u32,
            unscoped_samples: u64,
            methods: &'a [Row],
        }
        let report = Report {
            schema: 1,
            scope: "event-loop window; resolved declared calls and task bindings; builtins and implicit constructors excluded; scheduled segments include recursive/trampolined activations",
            sample_kind: "wall residency including host descheduling; labels restored when an SV task parks; setup, AST, VM and accessor paths are separate",
            sample_interval_ms: 5,
            unscoped_samples: samples.get(&0).map_or(0, |s| s.iter().sum()),
            methods: &rows,
        };
        let result = write_report(std::path::Path::new(&path), &report);
        if let Err(e) = result {
            eprintln!("[METHOD] profile write failed: {e}");
        }
    }
}

fn write_report(path: &std::path::Path, report: &impl Serialize) -> Result<(), String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .ok_or("profile path has no filename")?
        .to_string_lossy();
    let temporary = path.with_file_name(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let mut out = std::io::BufWriter::new(file);
        serde_json::to_writer(&mut out, report).map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;
        drop(out);
        std::fs::rename(&temporary, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

impl Simulator {
    pub(super) fn method_profile_id(&self, handle: usize, class: &str, method: &str) -> u64 {
        if !SAMPLER_READY.load(Ordering::Relaxed) {
            return 0;
        }
        let origin = self.class_origin(class);
        let spec = if !self.class_is_parameterized(origin) {
            None
        } else if handle != 0 {
            self.inherited_method_spec(handle, class).or_else(|| {
                self.heap
                    .get(handle)
                    .and_then(|o| o.as_ref())
                    .and_then(|o| o.spec.clone())
            })
        } else {
            self.current_spec.as_ref().and_then(|(base, sig)| {
                if self.class_origin(base) == origin {
                    Some((origin.into(), sig.clone()))
                } else {
                    self.ancestor_spec(base, sig, origin)
                        .map(|s| (origin.into(), self.canonicalize_spec_sig(origin, &s)))
                }
            })
        };
        identity(class, method, spec.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declaring_classes_and_specializations_have_distinct_ids() {
        let mut s = State::default();
        let a = s.id("A", "same", None);
        let b = s.id("B", "same", None);
        let x = ("C".into(), "8".into());
        let y = ("C".into(), "16".into());
        let c = s.id("C", "same", Some(&x));
        let d = s.id("C", "same", Some(&y));
        assert_eq!(s.id("C", "same", Some(&x)), c);
        assert_eq!([a, b, c, d], [1, 2, 3, 4]);
        s.rows[0].counters.calls = 7;
        s.reset_counts();
        assert_eq!(s.id("A", "same", None), a);
        assert_eq!(s.rows[0].counters.calls, 0);
    }

    #[test]
    fn packed_identity_and_path_change_as_one_atomic_word() {
        let current = AtomicU64::new(word(1, Path::Ast));
        let caller = current.swap(word(2, Path::Setup), Ordering::Relaxed);
        let child = current.swap(word(2, Path::Vm), Ordering::Relaxed);
        assert_eq!(current.load(Ordering::Relaxed) >> 2, 2);
        assert_eq!(current.load(Ordering::Relaxed) & 3, Path::Vm as u64);
        current.store(child, Ordering::Relaxed);
        assert_eq!(current.load(Ordering::Relaxed), word(2, Path::Setup));
        current.store(caller, Ordering::Relaxed);
        assert_eq!(current.load(Ordering::Relaxed), word(1, Path::Ast));
        assert_eq!(word(0, Path::Vm), 0);
    }
    #[test]
    fn declines_distinguish_new_decisions_unknown_cache_and_width_cache() {
        let mut state = State::default();
        let id = state.id("C", "f", None);
        let row = state.row(id).unwrap();
        row.decision(Decision::TierPending);
        row.decision(Decision::CachedRejection);
        row.decision(Decision::Decline("task-suspends"));
        row.decision(Decision::CachedRejection);
        row.decision(Decision::CachedBodyRejection);
        assert_eq!(row.counters.tier_pending, 1);
        assert_eq!(row.counters.compile_attempts, 0);
        assert_eq!(row.counters.cached_rejections, 3);
        assert_eq!(row.counters.declines["task-suspends"], 1);
        assert_eq!(
            row.counters.cached_decline_reasons["unknown_cached_reason"],
            1
        );
        assert_eq!(row.counters.cached_decline_reasons["task-suspends"], 1);
        assert_eq!(
            row.counters.cached_decline_reasons["body_cache_rejection"],
            1
        );
        state.reset_counts();
        let row = state.row(id).unwrap();
        row.decision(Decision::CachedRejection);
        assert_eq!(
            row.counters.cached_decline_reasons["unknown_cached_reason"],
            1
        );
        assert!(row.counters.declines.is_empty());
    }

    #[test]
    fn report_replaces_complete_json_and_cleans_failed_temporary() {
        let dir = std::env::temp_dir().join(format!("xezim_method_writer_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("methods.json");
        std::fs::write(&path, b"old").unwrap();
        write_report(&path, &vec![1, 2, 3]).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"[1,2,3]");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(write_report(&path, &vec![4]).is_err());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
