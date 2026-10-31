//! Checkpoints: `$save`, `$incsave` and `$restart`, and `--save-at` and
//! `--restore` on the command line.
//!
//! A checkpoint holds the state a run changes after setup (the fields listed
//! in `checkpoint_state!` below). Restoring builds the simulator from the same
//! design as usual, which recreates everything derived from it (compiled code,
//! signal tables, loaded libraries), and then replaces that state with the
//! checkpoint's. A checkpoint is taken at the end of a time slot, when every
//! process is parked in the scheduler's queues, the same point at which class
//! objects are reclaimed.
//!
//! Statement lists are shared between continuations through `Arc`. Within one
//! save or restore a session numbers each list, so a list shared by many
//! suspended processes is written once and shared again after a restore. A
//! few tables are keyed by a statement's address; the session also records
//! the address of every statement in each list it writes, in the order the
//! list serializes, and walks the restored list the same way, so those keys
//! are carried over to the new addresses.
//!
//! Not part of a checkpoint: waveform dumps (they do not continue after a
//! restore), the memory of DPI and VPI libraries, and a DPI task suspended in
//! C (a save waits until none is).

use super::*;
use serde::de::{Deserialize, Deserializer};
use serde::ser::{SerializeSeq, Serializer};

/// File magic and format version; bump the last byte with any change to a
/// state field's type.
const MAGIC: &[u8; 8] = b"XZCKPT\x00\x01";

#[derive(Default)]
struct Session {
    /// Saving: the address of each list written so far, to its number.
    saved: HashMap<usize, u32>,
    /// Saving: each statement address of the lists written so far, to its
    /// list and position in that list's walk.
    saved_stmts: HashMap<usize, (u32, u32)>,
    /// Restoring: the lists read so far, by number, with the statement
    /// addresses of each.
    restored: Vec<(Arc<[Statement]>, Vec<usize>)>,
    /// Restoring: old statement address to new.
    remap: HashMap<usize, usize>,
    /// Restoring: `foreach` markers whose loop could not be found.
    unresolved: usize,
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
}

/// Whether a checkpoint is being written or read on this thread.
pub(super) fn active() -> bool {
    SESSION.with(|s| s.borrow().is_some())
}

/// Run `f` inside a fresh checkpoint session.
pub(super) fn with_session<R>(f: impl FnOnce() -> R) -> R {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            SESSION.with(|s| *s.borrow_mut() = None);
        }
    }
    SESSION.with(|s| *s.borrow_mut() = Some(Session::default()));
    let _reset = Reset;
    f()
}

fn with<R>(f: impl FnOnce(&mut Session) -> R) -> R {
    SESSION.with(|s| f(s.borrow_mut().as_mut().expect("checkpoint session")))
}

/// A statement list as a checkpoint holds it: the list itself the first time,
/// with the address of each statement in it, and its number after that.
#[derive(serde::Serialize, serde::Deserialize)]
enum StmtsRef {
    New(Arc<[Statement]>, Vec<usize>),
    Seen(u32),
}

/// One link of a continuation chain.
#[derive(serde::Serialize, serde::Deserialize)]
struct Frame {
    stmts: StmtsRef,
    start: usize,
    loop_tail_at: Option<usize>,
    repeat_remaining: Option<u64>,
}

/// Number `stmts` in the session: written in full the first time.
fn stmts_ref(stmts: &Arc<[Statement]>) -> StmtsRef {
    let addr = Arc::as_ptr(stmts) as *const Statement as usize;
    if let Some(&n) = with(|s| s.saved.get(&addr).copied()).as_ref() {
        return StmtsRef::Seen(n);
    }
    let addrs = stmt_addrs(stmts);
    with(|s| {
        let n = s.saved.len() as u32;
        s.saved.insert(addr, n);
        for (i, &a) in addrs.iter().enumerate() {
            s.saved_stmts.entry(a).or_insert((n, i as u32));
        }
    });
    StmtsRef::New(stmts.clone(), addrs)
}

/// Map the addresses `list` had in the saving run to the ones it has now.
fn map_addresses(list: &Arc<[Statement]>, old: &[usize]) -> Vec<usize> {
    let new = stmt_addrs(list);
    with(|s| {
        if old.len() == new.len() {
            s.remap.extend(old.iter().copied().zip(new.iter().copied()));
        }
    });
    new
}

/// Take in a list read from a checkpoint outside a continuation, its
/// addresses already mapped: rewrite its markers and number it.
fn admit(mut list: Arc<[Statement]>, old: Vec<usize>) -> u32 {
    let new = map_addresses(&list, &old);
    rewrite_markers(&mut list);
    with(|s| {
        s.restored.push((list, new));
        s.restored.len() as u32 - 1
    })
}

/// Resolve a list reference read outside a continuation to its number.
fn admit_ref(r: StmtsRef) -> Option<u32> {
    match r {
        StmtsRef::New(list, old) => Some(admit(list, old)),
        StmtsRef::Seen(n) => restored(n).map(|_| n),
    }
}

/// The list numbered `n` in this restore.
fn restored(n: u32) -> Option<Arc<[Statement]>> {
    with(|s| s.restored.get(n as usize).map(|(l, _)| l.clone()))
}

/// §12.7.3: a `foreach` loop suspended in its body resumes through a
/// `ForeachTail` whose body is a marker holding the address of the loop's
/// body statement (see `run_process_stmts`). Point the markers of a list read
/// from a checkpoint at the restored loop. `list` must not be shared yet.
fn rewrite_markers(list: &mut Arc<[Statement]>) {
    let Some(stmts) = Arc::get_mut(list) else {
        return;
    };
    for st in stmts.iter_mut() {
        if let StatementKind::ForeachTail { body, .. } = &mut st.kind
            && matches!(body.kind, StatementKind::Null)
            && body.span.end == 0
        {
            match with(|s| s.remap.get(&body.span.start).copied()) {
                Some(new) => body.span.start = new,
                None => with(|s| s.unresolved += 1),
            }
        }
    }
}

/// The whole continuation chain, the current frame first.
pub(super) fn serialize_cont<S: Serializer>(
    cont: &ProcCont,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut frames = Vec::new();
    let mut frame = Some(cont);
    while let Some(c) = frame {
        frames.push(c);
        frame = c.next.as_deref();
    }
    // The format needs the length before the elements.
    let mut seq = serializer.serialize_seq(Some(frames.len()))?;
    for c in frames {
        seq.serialize_element(&Frame {
            stmts: stmts_ref(&c.stmts),
            start: c.start,
            loop_tail_at: c.loop_tail_at,
            repeat_remaining: c.repeat_remaining,
        })?;
    }
    seq.end()
}

impl<'de> Deserialize<'de> for ProcCont {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        if !active() {
            return Err(D::Error::custom("continuation outside a checkpoint"));
        }
        let frames = Vec::<Frame>::deserialize(deserializer)?;
        // A marker's loop is in an outer frame of the same chain or in a
        // list read before, so map every new list of the chain first.
        let mut lists: Vec<(Option<Arc<[Statement]>>, Option<u32>)> = Vec::new();
        let mut parts = Vec::with_capacity(frames.len());
        for f in frames {
            match f.stmts {
                StmtsRef::New(list, old) => {
                    let new = map_addresses(&list, &old);
                    lists.push((Some(list), None));
                    parts.push((f.start, f.loop_tail_at, f.repeat_remaining, new));
                }
                StmtsRef::Seen(n) => {
                    lists.push((None, Some(n)));
                    parts.push((f.start, f.loop_tail_at, f.repeat_remaining, Vec::new()));
                }
            }
        }
        let mut chain_lists = Vec::with_capacity(lists.len());
        for ((list, seen), part) in lists.into_iter().zip(parts.iter_mut()) {
            let list = match (list, seen) {
                (Some(mut list), _) => {
                    rewrite_markers(&mut list);
                    let addrs = std::mem::take(&mut part.3);
                    with(|s| s.restored.push((list.clone(), addrs)));
                    list
                }
                (None, Some(n)) => {
                    restored(n).ok_or_else(|| D::Error::custom("unknown statement list"))?
                }
                (None, None) => unreachable!(),
            };
            chain_lists.push(list);
        }
        // Rebuild innermost first so each frame owns its tail.
        let mut chain: Option<ProcCont> = None;
        for (part, list) in parts.into_iter().zip(chain_lists).rev() {
            chain = Some(ProcCont {
                stmts: list,
                start: part.0,
                next: chain.map(Arc::new),
                loop_tail_at: part.1,
                repeat_remaining: part.2,
            });
        }
        chain.ok_or_else(|| D::Error::custom("empty continuation"))
    }
}

/// The address of every statement in `stmts`, in the order the list
/// serializes: each element, and inside it every nested statement, boxed or
/// not, depth first.
fn stmt_addrs(stmts: &[Statement]) -> Vec<usize> {
    let mut w = addr_walk::Walk {
        out: Vec::new(),
        stmt: std::any::type_name::<Statement>(),
        boxed: std::any::type_name::<Box<Statement>>(),
    };
    for st in stmts {
        w.visit(st);
    }
    w.out
}

/// A serializer that only walks: it records the address of each `Statement`
/// it passes and writes nothing.
mod addr_walk {
    use super::Statement;
    use serde::ser::{self, Serialize};

    pub struct Walk {
        pub out: Vec<usize>,
        pub stmt: &'static str,
        pub boxed: &'static str,
    }

    #[derive(Debug)]
    pub struct Never;
    impl std::fmt::Display for Never {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("walk")
        }
    }
    impl std::error::Error for Never {}
    impl ser::Error for Never {
        fn custom<T: std::fmt::Display>(_: T) -> Self {
            Never
        }
    }

    impl Walk {
        pub fn visit<T: Serialize + ?Sized>(&mut self, v: &T) {
            let name = std::any::type_name::<T>();
            if name == self.stmt {
                self.out.push(v as *const T as *const u8 as usize);
            } else if name == self.boxed {
                // SAFETY: `T` is `Box<Statement>`, by its name.
                let b = unsafe { &*(v as *const T as *const Box<Statement>) };
                self.out.push(&**b as *const Statement as usize);
            }
            let _ = v.serialize(&mut *self);
        }
    }

    macro_rules! leaf {
        ($($f:ident($t:ty)),*) => {$(
            fn $f(self, _: $t) -> Result<(), Never> { Ok(()) }
        )*};
    }
    macro_rules! compound {
        ($tr:ident, $f:ident) => {
            impl ser::$tr for &mut Walk {
                type Ok = ();
                type Error = Never;
                fn $f<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Never> {
                    self.visit(v);
                    Ok(())
                }
                fn end(self) -> Result<(), Never> {
                    Ok(())
                }
            }
        };
    }
    compound!(SerializeSeq, serialize_element);
    compound!(SerializeTuple, serialize_element);
    compound!(SerializeTupleStruct, serialize_field);
    compound!(SerializeTupleVariant, serialize_field);
    impl ser::SerializeMap for &mut Walk {
        type Ok = ();
        type Error = Never;
        fn serialize_key<T: Serialize + ?Sized>(&mut self, k: &T) -> Result<(), Never> {
            self.visit(k);
            Ok(())
        }
        fn serialize_value<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Never> {
            self.visit(v);
            Ok(())
        }
        fn end(self) -> Result<(), Never> {
            Ok(())
        }
    }
    impl ser::SerializeStruct for &mut Walk {
        type Ok = ();
        type Error = Never;
        fn serialize_field<T: Serialize + ?Sized>(
            &mut self,
            _: &'static str,
            v: &T,
        ) -> Result<(), Never> {
            self.visit(v);
            Ok(())
        }
        fn end(self) -> Result<(), Never> {
            Ok(())
        }
    }
    impl ser::SerializeStructVariant for &mut Walk {
        type Ok = ();
        type Error = Never;
        fn serialize_field<T: Serialize + ?Sized>(
            &mut self,
            _: &'static str,
            v: &T,
        ) -> Result<(), Never> {
            self.visit(v);
            Ok(())
        }
        fn end(self) -> Result<(), Never> {
            Ok(())
        }
    }

    impl ser::Serializer for &mut Walk {
        type Ok = ();
        type Error = Never;
        type SerializeSeq = Self;
        type SerializeTuple = Self;
        type SerializeTupleStruct = Self;
        type SerializeTupleVariant = Self;
        type SerializeMap = Self;
        type SerializeStruct = Self;
        type SerializeStructVariant = Self;
        leaf!(
            serialize_bool(bool),
            serialize_i8(i8),
            serialize_i16(i16),
            serialize_i32(i32),
            serialize_i64(i64),
            serialize_i128(i128),
            serialize_u8(u8),
            serialize_u16(u16),
            serialize_u32(u32),
            serialize_u64(u64),
            serialize_u128(u128),
            serialize_f32(f32),
            serialize_f64(f64),
            serialize_char(char),
            serialize_str(&str),
            serialize_bytes(&[u8]),
            serialize_unit_struct(&'static str)
        );
        fn serialize_none(self) -> Result<(), Never> {
            Ok(())
        }
        fn serialize_unit(self) -> Result<(), Never> {
            Ok(())
        }
        fn serialize_unit_variant(
            self,
            _: &'static str,
            _: u32,
            _: &'static str,
        ) -> Result<(), Never> {
            Ok(())
        }
        fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> Result<(), Never> {
            self.visit(v);
            Ok(())
        }
        fn serialize_newtype_struct<T: Serialize + ?Sized>(
            self,
            _: &'static str,
            v: &T,
        ) -> Result<(), Never> {
            self.visit(v);
            Ok(())
        }
        fn serialize_newtype_variant<T: Serialize + ?Sized>(
            self,
            _: &'static str,
            _: u32,
            _: &'static str,
            v: &T,
        ) -> Result<(), Never> {
            self.visit(v);
            Ok(())
        }
        fn serialize_seq(self, _: Option<usize>) -> Result<Self, Never> {
            Ok(self)
        }
        fn serialize_tuple(self, _: usize) -> Result<Self, Never> {
            Ok(self)
        }
        fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self, Never> {
            Ok(self)
        }
        fn serialize_tuple_variant(
            self,
            _: &'static str,
            _: u32,
            _: &'static str,
            _: usize,
        ) -> Result<Self, Never> {
            Ok(self)
        }
        fn serialize_map(self, _: Option<usize>) -> Result<Self, Never> {
            Ok(self)
        }
        fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self, Never> {
            Ok(self)
        }
        fn serialize_struct_variant(
            self,
            _: &'static str,
            _: u32,
            _: &'static str,
            _: usize,
        ) -> Result<Self, Never> {
            Ok(self)
        }
    }
}

/// A checkpoint file after its magic: zstd-compressed bincode of this.
#[derive(serde::Serialize, serde::Deserialize)]
struct CheckpointFile {
    /// The build that wrote it; another build may lay out a field differently.
    build: String,
    /// `design_digest` of the design it was taken from.
    design: u64,
    /// The time of the slot it ended, in ticks.
    time: u64,
    /// Each state field in `checkpoint_state!` order, then the special ones.
    fields: Vec<(String, Vec<u8>)>,
    files: Vec<OpenFile>,
}

/// A file the design had open, to open again at the same position.
#[derive(serde::Serialize, serde::Deserialize)]
struct OpenFile {
    key: i32,
    path: String,
    mode: String,
    pos: u64,
}

/// One `blocking_cont_cache` entry: the statement its key names is found by
/// its position in the owner list's walk.
#[derive(serde::Serialize, serde::Deserialize)]
struct ContCacheEntry {
    owner: StmtsRef,
    node: u32,
    kind: u8,
    frame: StmtsRef,
}

/// `$save`, `$incsave` and `$restart` requests, served at the end of the slot.
pub(super) enum CheckpointOp {
    Save(String),
    Restart(String),
}

fn build_id() -> String {
    format!("{} {}", env!("CARGO_PKG_VERSION"), env!("XEZIM_GIT_HASH"))
}

fn enc<T: serde::Serialize + ?Sized>(v: &T) -> Result<Vec<u8>, String> {
    bincode::serde::encode_to_vec(v, xezim_core::xez_bincode_config()).map_err(|e| e.to_string())
}

fn dec<T: serde::de::DeserializeOwned>(b: &[u8]) -> Result<T, String> {
    bincode::serde::decode_from_slice(b, xezim_core::xez_bincode_config())
        .map(|(v, _)| v)
        .map_err(|e| e.to_string())
}

/// Replace `fresh` with `saved`, keeping each of `fresh`'s `Arc`s whose value
/// the checkpoint did not change: other tables hold the same `Arc`s and
/// compare them by identity (a class and its pending initializers, §8.7).
fn keep_shared<V: serde::Serialize>(
    fresh: &mut HashMap<String, Arc<V>>,
    saved: HashMap<String, Arc<V>>,
) {
    let mut out = HashMap::default();
    for (k, v) in saved {
        let same = fresh
            .get(&k)
            .is_some_and(|f| Arc::ptr_eq(f, &v) || enc(&**f).ok() == enc(&*v).ok());
        let keep = if same { fresh.remove(&k).unwrap() } else { v };
        out.insert(k, keep);
    }
    *fresh = out;
}

/// `--save-at` (seconds, path) and `--restore`, set by the driver.
struct CommandLine {
    save_at: Vec<(f64, String)>,
    restore: Option<String>,
}

static COMMAND_LINE: std::sync::Mutex<CommandLine> = std::sync::Mutex::new(CommandLine {
    save_at: Vec::new(),
    restore: None,
});

/// `--save-at <time>[,<file>]`: checkpoints to take, as (seconds, path).
pub fn set_save_at(save_at: Vec<(f64, String)>) {
    COMMAND_LINE.lock().unwrap().save_at = save_at;
}

/// Whether the run starts from a checkpoint.
pub(super) fn restore_pending() -> bool {
    COMMAND_LINE.lock().unwrap().restore.is_some()
}

/// `--restore <file>`: the checkpoint the run starts from.
pub fn set_restore(path: Option<String>) {
    COMMAND_LINE.lock().unwrap().restore = path;
}

/// Copy the restored `*restored` into `kept`, the allocation the run started
/// with, and put `kept` back, so a pointer into it stays valid.
fn refill_in_place<T: Copy>(
    restored: &mut Vec<T>,
    mut kept: Vec<T>,
    name: &str,
) -> Result<(), String> {
    if kept.len() != restored.len() {
        return Err(format!(
            "{name}: {} entries where {} were expected",
            restored.len(),
            kept.len()
        ));
    }
    kept.copy_from_slice(restored);
    *restored = kept;
    Ok(())
}

/// Declares the fields a checkpoint holds and generates their encoding.
/// Every field a run changes after setup is either here, in the special
/// fields encoded after them, or reset on restore (see `XEZIM_CKPT_AUDIT`).
macro_rules! checkpoint_state {
    ($($($f:ident).+),* $(,)?) => {
        impl Simulator {
            fn checkpoint_encode_state(&self, out: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
                $(
                    out.push((
                        stringify!($($f).+).to_string(),
                        enc(&self.$($f).+).map_err(|e| format!("{}: {e}", stringify!($($f).+)))?,
                    ));
                )*
                Ok(())
            }

            fn checkpoint_decode_state(
                &mut self,
                fields: &mut std::collections::VecDeque<(String, Vec<u8>)>,
            ) -> Result<(), String> {
                $(
                    let (name, bytes) = fields.pop_front().ok_or("the checkpoint ends early")?;
                    if name != stringify!($($f).+) {
                        return Err(format!(
                            "field '{name}' where '{}' was expected",
                            stringify!($($f).+)
                        ));
                    }
                    self.$($f).+ = dec(&bytes).map_err(|e| format!("{name}: {e}"))?;
                )*
                Ok(())
            }
        }
    };
}

/// The lazily built indexes (`OnceCell`s). A run builds one on first use and
/// keeps it, so one built early in a run can lack what the run added later;
/// a checkpoint holds whether each is built and what it holds, and a restored
/// run reads the same index the saving run would have.
macro_rules! checkpoint_once_cells {
    ($($f:ident),* $(,)?) => {
        impl Simulator {
            fn checkpoint_encode_cells(&self, out: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
                $(
                    out.push((stringify!($f).to_string(), enc(&self.$f.get())?));
                )*
                Ok(())
            }

            fn checkpoint_decode_cells(
                &mut self,
                fields: &mut std::collections::VecDeque<(String, Vec<u8>)>,
            ) -> Result<(), String> {
                $(
                    let (name, bytes) = fields.pop_front().ok_or("the checkpoint ends early")?;
                    if name != stringify!($f) {
                        return Err(format!("field '{name}' where '{}' was expected", stringify!($f)));
                    }
                    self.$f = std::cell::OnceCell::new();
                    if let Some(v) = dec(&bytes).map_err(|e| format!("{name}: {e}"))? {
                        let _ = self.$f.set(v);
                    }
                )*
                Ok(())
            }
        }
    };
}

checkpoint_once_cells!(
    param_leaf_index_cell,
    var_decl_leaf_index_cell,
    vif_capable_statics,
    iface_instance_paths,
    iface_by_token,
    iface_path_parts,
    param_leaf_names,
    unpacked_typedef_reps,
    unpacked_typedef_names,
    unpacked_struct_prop_names,
    struct_capable_prop_names,
    struct_coll_member_prop_names,
    indexed_coll_roots,
    class_member_names_cell,
    covergroup_leaf_names,
    chained_coll_member_names,
);

impl Simulator {
    /// The design a checkpoint belongs to: its signals' names and widths.
    fn design_digest(&self) -> u64 {
        use std::hash::Hasher as _;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        h.write(&enc(&(&self.id_to_name, &self.signal_widths)).unwrap_or_default());
        h.finish()
    }

    /// Convert `--save-at` to ticks and take `--restore`, before the first
    /// slot runs. A checkpoint that cannot be restored ends the run.
    pub(super) fn checkpoint_start(&mut self) {
        let (save_at, restore) = {
            let cl = COMMAND_LINE.lock().unwrap();
            (cl.save_at.clone(), cl.restore.clone())
        };
        self.checkpoint_muted = false;
        if let Some(path) = restore {
            if let Err(e) = self.checkpoint_restore(&path) {
                eprintln!("Error: cannot restore '{}': {}", path, e);
                std::process::exit(1);
            }
            eprintln!("[xezim] restored '{}' at time {}", path, self.time);
        }
        // The last slot at or before the time: round down, past float noise.
        let mut ticks: Vec<(u64, String)> = save_at
            .into_iter()
            .map(|(secs, p)| ((secs / self.tick_s + 1e-6).floor() as u64, p))
            .filter(|(t, _)| *t >= self.time)
            .collect();
        ticks.sort_by_key(|(t, _)| *t);
        self.checkpoint_save_at = ticks;
    }

    /// Whether a checkpoint request is due before time moves to `next_time`.
    #[inline]
    pub(super) fn checkpoint_due(&self, next_time: u64) -> bool {
        !self.checkpoint_ops.is_empty()
            || self
                .checkpoint_save_at
                .first()
                .is_some_and(|(t, _)| next_time > *t)
    }

    /// Serve the requests due at the end of this slot. True when a
    /// `$restart` replaced the state, so the caller looks at the queues again.
    pub(super) fn serve_checkpoints(&mut self, next_time: u64) -> bool {
        // A DPI task suspended in C keeps state on a stack a checkpoint
        // cannot hold; wait for a slot where none is.
        if !self.dpi_tasks.is_empty() {
            return false;
        }
        let mut ops = std::mem::take(&mut self.checkpoint_ops);
        while self
            .checkpoint_save_at
            .first()
            .is_some_and(|(t, _)| next_time > *t)
        {
            let (_, path) = self.checkpoint_save_at.remove(0);
            ops.push(CheckpointOp::Save(path));
        }
        let mut replaced = false;
        for op in ops {
            match op {
                CheckpointOp::Save(path) => match self.checkpoint_save(&path) {
                    Ok(()) => eprintln!("[xezim] saved '{}' at time {}", path, self.time),
                    Err(e) => {
                        eprintln!("Error: cannot save '{}': {}", path, e);
                        self.error_count += 1;
                    }
                },
                CheckpointOp::Restart(path) => match self.checkpoint_restore(&path) {
                    Ok(()) => {
                        eprintln!("[xezim] restarted from '{}' at time {}", path, self.time);
                        replaced = true;
                    }
                    Err(e) => {
                        eprintln!("Error: cannot restart from '{}': {}", path, e);
                        self.error_count += 1;
                    }
                },
            }
        }
        replaced
    }

    /// Requests the run ended before serving.
    pub(super) fn checkpoint_unserved(&mut self) {
        for op in std::mem::take(&mut self.checkpoint_ops) {
            let path = match op {
                CheckpointOp::Save(p) | CheckpointOp::Restart(p) => p,
            };
            eprintln!(
                "Warning: '{}' was not saved or restored: the simulation ended in the same time slot",
                path
            );
        }
        for (_, path) in std::mem::take(&mut self.checkpoint_save_at) {
            eprintln!(
                "Warning: '{}' was not saved: the simulation ended before its --save-at time",
                path
            );
        }
    }

    /// Give each restored process FSM the native body this run's setup
    /// built. A body saved while running natively holds its registers in
    /// `native_frame` (non-empty), one saved in bytecode in `regs`, so each
    /// resumes in the form it was saved in.
    #[cfg(feature = "jit")]
    fn reattach_native_fsms(
        &mut self,
        natives: &HashMap<usize, super::aot::AotFsmFn>,
    ) -> Result<(), String> {
        for (pid, f) in self.proc_fsm.iter_mut() {
            if f.native_frame.is_empty() {
                continue;
            }
            f.native = Some(*natives.get(pid).ok_or_else(|| {
                format!("process {pid} was saved running natively; restore with XEZIM_AOT=1")
            })?);
        }
        Ok(())
    }

    /// Restore the `signal_inline_bits` mirror of `signal_table` into the
    /// planes this run allocated. A run without them (no `XEZIM_JIT`) drops
    /// the saved ones; a run with them restoring a checkpoint saved without
    /// rebuilds them from the restored signals, as setup does.
    fn refill_inline_bits(&mut self, kept: Vec<[u64; 2]>) -> Result<(), String> {
        if kept.is_empty() {
            self.signal_inline_bits = kept;
            return Ok(());
        }
        if self.signal_inline_bits.is_empty() {
            let mut kept = kept;
            if kept.len() != self.signal_table.len() {
                return Err("signal_inline_bits: wrong length".into());
            }
            for (plane, v) in kept.iter_mut().zip(&self.signal_table) {
                let (val_bits, xz_bits) = v.raw_bits();
                *plane = [val_bits, xz_bits];
            }
            self.signal_inline_bits = kept;
            return Ok(());
        }
        refill_in_place(&mut self.signal_inline_bits, kept, "signal_inline_bits")
    }

    /// Write a checkpoint of the state at the end of the current slot.
    pub(super) fn checkpoint_save(&mut self, path: &str) -> Result<(), String> {
        use std::io::Write as _;
        self.flush_file_writes();
        self.flush_stdout();
        let files = self.checkpoint_files()?;
        let fields = with_session(|| -> Result<Vec<(String, Vec<u8>)>, String> {
            // The continuation cache first: it holds every list a
            // `foreach` marker can name (see `rewrite_markers`).
            let mut out = vec![(
                "blocking_cont_cache".into(),
                enc(&self.checkpoint_cont_cache())?,
            )];
            self.checkpoint_encode_state(&mut out)?;
            self.checkpoint_encode_cells(&mut out)?;
            out.push((
                "named_block_frames".into(),
                enc(&self.checkpoint_named_frames())?,
            ));
            out.push((
                "static_local_first".into(),
                enc(self.static_local_first.as_slice())?,
            ));
            out.push(("module.classes".into(), enc(&self.module.classes)?));
            let toggle = self.code_cov.as_ref().and_then(|c| c.toggle.as_ref());
            out.push(("code_cov.toggle".into(), enc(&toggle)?));
            Ok(out)
        })?;
        let file = CheckpointFile {
            build: build_id(),
            design: self.design_digest(),
            time: self.time,
            fields,
            files,
        };
        // Written beside the target and renamed over it, so a failed save
        // leaves an earlier checkpoint of the same name intact.
        let tmp = format!("{}.tmp{}", path, std::process::id());
        let write = || -> std::io::Result<()> {
            let mut w = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
            w.write_all(MAGIC)?;
            let mut z = zstd::stream::Encoder::new(w, 3)?;
            bincode::serde::encode_into_std_write(&file, &mut z, xezim_core::xez_bincode_config())
                .map_err(std::io::Error::other)?;
            z.finish()?.flush()?;
            std::fs::rename(&tmp, path)
        };
        write().map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        })?;
        audit::write_digests(self, &format!("{path}.save"));
        Ok(())
    }

    /// Replace the state with a checkpoint's. The design must be the one it
    /// was taken from.
    pub(super) fn checkpoint_restore(&mut self, path: &str) -> Result<(), String> {
        use std::io::Read as _;
        let mut f = std::io::BufReader::new(std::fs::File::open(path).map_err(|e| e.to_string())?);
        let mut magic = [0u8; 8];
        f.read_exact(&mut magic)
            .map_err(|_| "not a checkpoint".to_string())?;
        if &magic[..7] != &MAGIC[..7] {
            return Err("not a checkpoint".into());
        }
        if magic[7] != MAGIC[7] {
            return Err("written in another checkpoint format".into());
        }
        let mut z = zstd::stream::Decoder::new(f).map_err(|e| e.to_string())?;
        let file: CheckpointFile =
            bincode::serde::decode_from_std_read(&mut z, xezim_core::xez_bincode_config())
                .map_err(|e| format!("unreadable: {e}"))?;
        if file.build != build_id() {
            return Err(format!(
                "written by xezim {}, this is {}",
                file.build,
                build_id()
            ));
        }
        if file.design != self.design_digest() {
            return Err("taken from a different design".into());
        }
        // Open handles of a `$restart` close first.
        self.flush_file_writes();
        self.file_handles.clear();
        let mut fields: std::collections::VecDeque<_> = file.fields.into();
        with_session(|| -> Result<(), String> {
            let (_, b) = fields.pop_front().ok_or("the checkpoint ends early")?;
            self.restore_cont_cache(dec(&b)?);
            #[cfg(feature = "jit")]
            let natives: HashMap<usize, super::aot::AotFsmFn> = self
                .proc_fsm
                .iter()
                .filter_map(|(pid, f)| Some((*pid, f.native?)))
                .collect();
            // Compiled code holds the addresses of the signal planes, so
            // they are refilled in place rather than replaced.
            let inline_bits = std::mem::take(&mut self.signal_inline_bits);
            let has_xz = std::mem::take(&mut self.signal_has_xz);
            self.checkpoint_decode_state(&mut fields)?;
            self.refill_inline_bits(inline_bits)?;
            refill_in_place(&mut self.signal_has_xz, has_xz, "signal_has_xz")?;
            #[cfg(feature = "jit")]
            self.reattach_native_fsms(&natives)?;
            self.checkpoint_decode_cells(&mut fields)?;
            let (_, b) = fields.pop_front().ok_or("the checkpoint ends early")?;
            self.restore_named_frames(dec(&b)?);
            self.remap_block_scopes();
            let (_, b) = fields.pop_front().ok_or("the checkpoint ends early")?;
            let first: Vec<bool> = dec(&b)?;
            self.static_local_first = first
                .try_into()
                .map_err(|_| "static_local_first: wrong length".to_string())?;
            let (_, b) = fields.pop_front().ok_or("the checkpoint ends early")?;
            keep_shared(&mut self.module.classes, dec(&b)?);
            let (_, b) = fields.pop_front().ok_or("the checkpoint ends early")?;
            let toggle: Option<code_cov::Toggle> = dec(&b)?;
            if let Some(cov) = self.code_cov.as_mut() {
                cov.toggle = toggle;
            }
            let unresolved = with(|s| s.unresolved);
            if unresolved > 0 {
                return Err(format!(
                    "{unresolved} suspended foreach loops could not be resumed"
                ));
            }
            Ok(())
        })?;
        self.reopen_files(file.files)?;
        // Caches keyed by an address in the run that wrote the checkpoint.
        self.recv_call_memo.clear();
        self.method_plans.borrow_mut().clear();
        self.fsm_clobber_cache.clear();
        self.class_task_frame_cache.clear();
        super::super::interrupt::publish_sim_time(self.time);
        self.checkpoint_restored = true;
        audit::write_digests(self, &format!("{path}.restore"));
        Ok(())
    }

    /// The open files, flushed, with their positions.
    fn checkpoint_files(&mut self) -> Result<Vec<OpenFile>, String> {
        use std::io::Seek as _;
        let mut out = Vec::new();
        let mut keys: Vec<i32> = self.file_handles.keys().copied().collect();
        keys.sort_unstable();
        for key in keys {
            let f = self.file_handles.get_mut(&key).expect("open file");
            let pos = f.stream_position().map_err(|e| e.to_string())?;
            let (path, mode) = f.origin();
            out.push(OpenFile {
                key,
                path: path.to_string(),
                mode: mode.to_string(),
                pos,
            });
        }
        Ok(out)
    }

    /// Open the checkpoint's files again. A file written to is cut back to
    /// the length it had, so output written after the save is dropped, as a
    /// simulation that never ran past it would not have written it.
    fn reopen_files(&mut self, files: Vec<OpenFile>) -> Result<(), String> {
        use std::io::{Seek as _, SeekFrom};
        for of in files {
            let mut opts = std::fs::OpenOptions::new();
            let writes = of.mode.contains('w') || of.mode.contains('a') || of.mode.contains('+');
            opts.read(of.mode.contains('r') || of.mode.contains('+') || !writes);
            if writes {
                opts.write(true).create(true);
            }
            let file = opts
                .open(&of.path)
                .map_err(|e| format!("cannot open '{}' again: {}", of.path, e))?;
            if writes && !of.mode.contains('r') {
                file.set_len(of.pos)
                    .map_err(|e| format!("'{}': {}", of.path, e))?;
            }
            let mut sv = sv_file::SvFile::new(file, &of.path, &of.mode);
            sv.seek(SeekFrom::Start(of.pos))
                .map_err(|e| format!("'{}': {}", of.path, e))?;
            self.file_handles.insert(of.key, sv);
        }
        Ok(())
    }

    fn checkpoint_cont_cache(&self) -> Vec<ContCacheEntry> {
        let mut out = Vec::new();
        for (&(node, kind), (owner, frame)) in &self.blocking_cont_cache {
            let owner = stmts_ref(owner);
            let owner_n = match &owner {
                StmtsRef::New(..) => with(|s| s.saved.len() as u32 - 1),
                StmtsRef::Seen(n) => *n,
            };
            let Some(pos) = with(|s| {
                s.saved_stmts
                    .get(&node)
                    .filter(|(n, _)| *n == owner_n)
                    .map(|&(_, p)| p)
            }) else {
                continue;
            };
            out.push(ContCacheEntry {
                owner,
                node: pos,
                kind,
                frame: stmts_ref(frame),
            });
        }
        out
    }

    fn restore_cont_cache(&mut self, entries: Vec<ContCacheEntry>) {
        self.blocking_cont_cache.clear();
        // Map every list first: a marker in one can name a statement of a
        // list that comes later.
        for e in &entries {
            for r in [&e.owner, &e.frame] {
                if let StmtsRef::New(list, old) = r {
                    map_addresses(list, old);
                }
            }
        }
        for e in entries {
            let (Some(owner), Some(frame)) = (admit_ref(e.owner), admit_ref(e.frame)) else {
                continue;
            };
            let entry = with(|s| {
                let (o, addrs) = &s.restored[owner as usize];
                let node = *addrs.get(e.node as usize)?;
                Some((node, o.clone(), s.restored[frame as usize].0.clone()))
            });
            if let Some((node, o, f)) = entry {
                self.blocking_cont_cache.insert((node, e.kind), (o, f));
            }
        }
    }

    /// §6.21: an open block scope names its flattened frame by address
    /// (`BlockScopeSave::frame_key`, the address of the frame's first
    /// statement); point it at the restored frame.
    fn remap_block_scopes(&mut self) {
        let saves = self.block_scope_saves.iter_mut().chain(
            self.process_contexts
                .values_mut()
                .flat_map(|c| c.block_scope_saves.iter_mut()),
        );
        for sv in saves {
            if sv.frame_key != 0 {
                sv.frame_key = with(|s| s.remap.get(&sv.frame_key).copied()).unwrap_or(usize::MAX);
            }
        }
    }

    /// §9.6.2 named-block frames still on a continuation, by list number.
    fn checkpoint_named_frames(&self) -> Vec<(u32, String)> {
        self.named_block_frames
            .iter()
            .filter_map(|(addr, name)| with(|s| s.saved.get(addr).map(|&n| (n, name.clone()))))
            .collect()
    }

    fn restore_named_frames(&mut self, frames: Vec<(u32, String)>) {
        self.named_block_frames.clear();
        for (n, name) in frames {
            if let Some(list) = restored(n) {
                self.named_block_frames.insert(Self::frame_key(&list), name);
            }
        }
    }
}

// The state a run changes after setup, from `XEZIM_CKPT_AUDIT` over the
// test corpus and suites. A field added to `Simulator` that a run changes
// belongs here (or in the special fields, or in the restore resets).
checkpoint_state!(
    active_force_evals,
    active_force_exprs,
    active_force_signal_epochs,
    active_force_skips,
    active_task_pids,
    active_union_tag,
    annex_n_seed,
    any_struct_formal_markers,
    armed_fast_skips,
    armed_input_bitmap,
    armed_input_blocks,
    armed_input_ranges,
    array_elem_ids,
    array_local_packed_dims,
    assert_ctl,
    assert_ctl_on,
    assertion_labels,
    assertion_stats,
    assoc_elem_colls,
    assoc_keys_cache,
    assoc_static_keys_cache,
    await_waiters,
    bit_sens_pending,
    block_scope_saves,
    cg_event_waiters,
    cg_heap,
    cg_instance_event_waiters,
    cg_named_event_waiters,
    cg_tree_roots_buf,
    cg_type_options,
    changed_edge_pos,
    class_assoc_defaults,
    class_coll_index,
    class_context_stack,
    class_enclosing_cache,
    class_gc_heap_len,
    class_gc_live,
    class_gc_released,
    class_gc_runs,
    class_handle_member_types_cache,
    class_has_method_memo,
    class_method_cache,
    class_param_arrays,
    class_prop_dim_cache,
    class_prop_struct_neg,
    class_statics,
    class_string_props,
    clock_generators,
    clocking_array_leaves,
    clocking_hist,
    clocking_hist_slot,
    clocking_last_edge,
    clocking_preponed,
    clocking_preponed_slot,
    clocking_snapshots,
    clocking_tick_cache,
    code_cov_hits,
    coll_member_names,
    collection_dim_cache,
    comb_dep_edges,
    comb_entries,
    comb_plan,
    comb_plan_abortn,
    comb_time0_deferred_done,
    comb_time0_fired,
    compiled_class_method_ids,
    compiled_fast_calls,
    compiled_fast_static_calls,
    compiled_method_admission,
    compiled_method_block_cache,
    compiled_method_boost_qual,
    compiled_method_call_counts,
    compiled_method_loop_heat,
    compiled_method_plans,
    compiled_method_skip,
    compiled_spec_intern,
    cond_progress,
    cond_read_union,
    cond_read_union_dirty,
    cond_repass_recheck,
    cond_waiter_conditions,
    cond_waiter_gate,
    cond_waiter_reads,
    condition_waiters,
    constraint_mode_disabled,
    cross_block_wakeup_count,
    current_auto_task,
    current_pid,
    current_scope,
    current_spec,
    deferred_asserts,
    deferred_comb_pool,
    deferred_proc_entries,
    delayed_nba,
    delayed_updates,
    dirty_any,
    dirty_list,
    dirty_signals,
    disable_labels,
    dist_decks,
    dist_picked_once,
    dpi_bind_tried,
    dpi_export_seq,
    dpi_import_fds,
    dpi_unresolved,
    dyn_key_scratch,
    dynamic_assoc_dims,
    edge_block_arm_only,
    edge_block_armed,
    edge_block_change_streak,
    edge_block_epoch_probe_left,
    edge_block_exec_counts,
    edge_block_exec_ns,
    edge_block_gateable,
    edge_block_off,
    edge_block_reads_flat,
    edge_block_reads_meta,
    edge_block_skip_streak,
    edge_block_snap_flat,
    edge_block_snap_valid,
    edge_block_wide_read,
    edge_fired_snap,
    edge_group_generation,
    edge_group_memo,
    edge_iff_denied,
    edge_pos_seen,
    edge_prefilter_generation,
    edge_prefilter_seen,
    edge_rescan_block_hits,
    edge_signal_ids,
    edge_triggered_bitmap,
    elem_dotted_bases,
    ended_waiting_in_c,
    entry_evals,
    error_count,
    event_aliases,
    event_epoch_fast_exec,
    event_gateable_total,
    event_heal,
    event_healed,
    event_measure,
    event_phase,
    event_queue,
    event_skip,
    event_snapshot_checks,
    event_triggered_time,
    event_waiters,
    event_would_skip,
    exec_cache_pids,
    fast_delay_always,
    fatal_finish_number,
    fb_unbound_refs,
    file_eof,
    file_writes_pending,
    finished,
    finished_pids,
    flop_last_fire,
    fn_decl_cache,
    fn_pure_cache,
    fn_ret_ctx_stack,
    force_epoch_gen,
    force_refresh_done_gen,
    forced_names,
    forced_signals,
    forever_cont_cache,
    forever_sens_cache,
    fork_baselines,
    fork_block_children,
    fork_capture_refs,
    fork_collection_captures,
    fork_gen_pids,
    fork_local_scopes,
    fork_pid_gens,
    fork_signal_captures,
    fork_spawn_cont,
    formal_dyn,
    formal_parts_pool,
    frame_gen,
    frame_pool,
    free_call_pkgs,
    free_fn_class_names,
    free_fn_table,
    fsm_precision_diff,
    gate_fall_delay_by_id,
    gate_lane,
    gate_lane_valid,
    gate_obs,
    gate_off_delay_by_id,
    has_method_cache,
    has_coll_rows,
    has_modclass,
    heap,
    hint_key_scratch,
    hint_string_pool,
    indexed_coll_mask,
    initialized_spec_statics,
    inst_key_scratch,
    inst_templates,
    instance_event_waiters,
    intra_saved,
    intra_saved_next,
    is_static_method_memo,
    join_waiters,
    killed_pids,
    last_vif_return,
    late_resume_epoch,
    lhs_leaf_width_cache,
    local_coll_rows_live,
    local_coll_shapes,
    local_dyn,
    local_dyn_pool,
    local_elem_layout,
    local_gen_stack,
    local_iface_aliases,
    local_stack,
    local_type_pool,
    local_type_stack,
    loop_frame_gens,
    loop_iters,
    m_block_active,
    m_block_scope,
    m_block_scope_id,
    m_scope_stack,
    mailbox_bound,
    mailbox_get_waiters,
    mailbox_put_waiters,
    mailboxes,
    marker_key_scratch,
    max_settle_iters,
    mbx_free_ids,
    mbx_msg_seq,
    mbx_patterns,
    member_coll_cache,
    method_def_cache,
    method_receiver_cache,
    method_receiver_hint_ids,
    modclass_colls,
    modclass_insts,
    module_paths,
    monitor,
    monitor_arg_prev,
    monitor_m_suffix,
    monitor_paused,
    monitor_scope,
    multi_dim_array_names,
    multidim_members,
    name_resolve_hint,
    nba_fast,
    nba_fast_index,
    nba_touched_edge_non_clock,
    nested_run_epoch,
    next_dpi_task,
    next_dyn_id,
    next_pid,
    next_sync_wake,
    nonstatic_scope_reported,
    nonvirtual_target_cache,
    null_call_reported,
    obj_rng,
    output,
    p_top_level,
    packed,
    packed_array_copy_values,
    packed_blocking_fill_collapses,
    par_cal_seq_insns,
    par_cal_seq_ns,
    par_cal_ticks,
    partial_forces,
    pattern_scratch_free,
    pattern_scratch_seq,
    pending_local_shape,
    pending_observed,
    pending_reactive,
    pending_release_ids,
    persisted_method_context_hash,
    pid_src_file,
    plain_class_types,
    preponed_ids,
    prev_val,
    prev_wide,
    prev_xz,
    proc_fsm,
    proc_rng,
    process_cond_bytecode,
    process_contexts,
    process_loop_bytecode,
    process_m_label,
    process_m_scope,
    process_parents,
    process_scope_hint,
    prof_clocks_only_detect,
    prof_edge_cg,
    prof_edge_detect,
    prof_edge_exec,
    prof_edge_waiters,
    prof_edges_fired,
    prof_entry_counts,
    prof_entry_hist,
    prof_fallback_by_reason,
    prof_fallback_insns,
    prof_insns_executed,
    prof_nba_elided,
    prof_observed_stores,
    prof_par_blocks,
    prof_par_dispatch_legacy,
    prof_par_ticks,
    prof_quiet_stores,
    prof_settle_ab_count,
    prof_settle_ca_count,
    prof_settle_ca_ns,
    prof_settle_dc_count,
    prof_settle_dep_edges,
    prof_settle_repass_evals,
    prof_settle_writes,
    prof_ts_bail_abort,
    prof_ts_bail_forced,
    prof_ts_bail_warnx,
    prof_ts_bail_xread,
    prof_ts_evals,
    prof_tsx_evals,
    prof_waiter_iters,
    program_pids,
    prop_owner_index,
    prop_raw_ty_cache,
    prop_width_memo,
    queue_frame_saves,
    queue_key_scratch,
    queue_name_pool,
    queue_save_pool,
    queues,
    quiet_bits,
    quiet_valid,
    rand_elem_ranges,
    rand_fail_hinted,
    rand_fail_reports,
    rand_mode_disabled,
    rand_ranges,
    rand_scope_nd,
    rand_scope_nd_dyn,
    rand_scope_row_owners,
    rand_tight_mode,
    randc_pending,
    randc_used,
    randc_wide,
    randc_wide_pending,
    randomize_budget_exhausted,
    randomize_budget_remaining,
    real_signals,
    ref_alias_stack,
    ref_binding_stack,
    ref_identity_stack,
    ref_redirect_hot,
    return_flag,
    return_value,
    rng,
    sampled_defaults,
    sampled_site_clock,
    sampled_watch_site,
    sampled_watches,
    saw_fatal,
    sdf_delays,
    semaphore_get_waiters,
    semaphores,
    sensitivity_trigger_count,
    settle_calls,
    settle_dirty_ids,
    settle_inject_bits,
    settle_inject_sum,
    settle_iters,
    settle_limit_hits,
    settle_triggered,
    settle_triggered_list,
    shadowed_prop_mask,
    shadowed_prop_names,
    sig_last_change,
    signal_has_xz,
    signal_inline_bits,
    signal_table,
    signal_toggle_count,
    signals,
    signed_class_colls,
    signed_signals,
    snap_gen,
    spec_buf_pool,
    spec_clone_origin,
    spec_entry_cache,
    spec_frag_fold_cache,
    spec_prop_is_dyn,
    spec_prop_width_cache,
    stall_edge_block_hits,
    stall_iters,
    stall_pid_hits,
    stall_time,
    static_block_decls,
    static_cell_width_cache,
    static_constraint_disabled,
    static_fn_ret,
    static_local_keys,
    static_local_names,
    static_local_syncs,
    static_local_vars,
    static_struct_holders,
    static_task_init,
    static_vif_bindings,
    static_vif_names,
    string_keyed_member_cache,
    string_signals,
    struct_coll_member_len_mask,
    struct_root_names,
    struct_root_names_built,
    stuck_clock_aborted,
    sva_preponed,
    sva_preponed_slot,
    sva_sites,
    sync_name_pool,
    t0_delay_deferred,
    t0_pid_ord,
    t0_starts_remaining,
    table_modified,
    task_cleanup,
    task_decl_cache,
    task_string_formals,
    tb_cache,
    this_stack,
    time,
    time_format,
    timing_checks,
    timing_watches,
    toggled_clock_positions,
    ts,
    ts_arena,
    ts_asleep_edge,
    ts_comb,
    ts_comb_xb,
    ts_cur_eidx,
    ts_edge,
    ts_edge_abortn,
    ts_edge_xb,
    ts_exec_aborted,
    ts_hdr,
    ts_hdr_tsx,
    ts_last_xbail,
    ts_rec0,
    ts_regs,
    ts_sleep,
    ts_sleep_recs,
    ts_wregs,
    ts_wregs8,
    ts_xregs,
    type_tables_gen,
    typedef_layouts_by_width,
    typedef_target_cache,
    typeref_class_memo,
    udp_runtime,
    var_class_types,
    var_container_types,
    var_type_args,
    var_typedef_types,
    vc_saved,
    vif_key_scratch,
    vif_local_names,
    vif_return_pending,
    vif_tokens_stored,
    viface_var_aliases,
    virtual_iface_bindings,
    vm_direct_calls,
    vm_regs,
    vpi_slot_started,
    wake_rank_buf,
    warn_x_count,
    warn_x_drivers,
    warn_x_seen,
    warned_assoc_index,
    warned_delay_spikes,
    warned_system_tasks,
    widths,
    module.array_elem_class,
    module.arrays,
    module.arrays_2d,
    module.arrays_nd,
    module.assoc_defaults,
    module.assoc_elem_widths,
    module.assoc_index_widths,
    module.assoc_key_type_names,
    module.associative_arrays,
    module.coll_shapes,
    module.descending_arrays,
    module.dynamic_arrays,
    module.enum_members,
    module.events,
    module.packed_full_dims,
    module.packed_signal_elem_widths,
    module.packed_struct_fields,
    module.parameters,
    module.queue_vars,
    module.struct_members,
    module.typedef_types,
    module.typedef_unpacked_dims,
    module.typedefs,
    module.unpacked_decl_dims,
    module.var_decl_types,
);

/// `XEZIM_CKPT_AUDIT=<file>`: the digest of each field of the simulator,
/// taken after setup and again when the event loop ends, so a corpus run
/// lists the fields a run changes. The fields a checkpoint holds must cover
/// them; the names are appended to the file, one per line.
pub(super) mod audit {
    use super::*;
    use serde::ser::{Impossible, SerializeStruct};

    #[derive(Debug)]
    pub struct Stop;
    impl std::fmt::Display for Stop {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("not a struct")
        }
    }
    impl std::error::Error for Stop {}
    impl serde::ser::Error for Stop {
        fn custom<T: std::fmt::Display>(_: T) -> Self {
            Stop
        }
    }

    struct Hasher(std::collections::hash_map::DefaultHasher, usize);
    impl std::io::Write for Hasher {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            use std::hash::Hasher as _;
            self.0.write(b);
            self.1 += b.len();
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Field name to (digest, encoded length); a field that fails to encode
    /// is recorded with length `usize::MAX`.
    pub struct Digests(pub Vec<(String, u64, usize)>);

    /// The simulator's fields, then the elaborated design's as `module.<f>`.
    pub fn digests(sim: &Simulator) -> Digests {
        let mut out = Digests(Vec::new());
        let _ = serde::Serialize::serialize(sim, &mut out);
        let mut module = Digests(Vec::new());
        let _ = serde::Serialize::serialize(&sim.module, &mut module);
        for (k, d, l) in module.0 {
            out.0.push((format!("module.{k}"), d, l));
        }
        out
    }

    impl SerializeStruct for &mut Digests {
        type Ok = ();
        type Error = Stop;
        fn serialize_field<T: serde::Serialize + ?Sized>(
            &mut self,
            key: &'static str,
            value: &T,
        ) -> Result<(), Stop> {
            use std::hash::Hasher as _;
            let mut h = Hasher(Default::default(), 0);
            let r = with_session(|| {
                bincode::serde::encode_into_std_write(
                    value,
                    &mut h,
                    xezim_core::xez_bincode_config(),
                )
            });
            let len = if r.is_ok() { h.1 } else { usize::MAX };
            self.0.push((key.to_string(), h.0.finish(), len));
            Ok(())
        }
        fn end(self) -> Result<(), Stop> {
            Ok(())
        }
    }

    macro_rules! refuse {
        ($($f:ident($($t:ty),*)),*) => {$(
            fn $f(self, $(_: $t),*) -> Result<(), Stop> { Err(Stop) }
        )*};
    }

    impl<'a> Serializer for &'a mut Digests {
        type Ok = ();
        type Error = Stop;
        type SerializeSeq = Impossible<(), Stop>;
        type SerializeTuple = Impossible<(), Stop>;
        type SerializeTupleStruct = Impossible<(), Stop>;
        type SerializeTupleVariant = Impossible<(), Stop>;
        type SerializeMap = Impossible<(), Stop>;
        type SerializeStruct = Self;
        type SerializeStructVariant = Impossible<(), Stop>;
        refuse!(
            serialize_bool(bool),
            serialize_i8(i8),
            serialize_i16(i16),
            serialize_i32(i32),
            serialize_i64(i64),
            serialize_u8(u8),
            serialize_u16(u16),
            serialize_u32(u32),
            serialize_u64(u64),
            serialize_f32(f32),
            serialize_f64(f64),
            serialize_char(char),
            serialize_str(&str),
            serialize_bytes(&[u8]),
            serialize_none(),
            serialize_unit(),
            serialize_unit_struct(&'static str),
            serialize_unit_variant(&'static str, u32, &'static str)
        );
        fn serialize_some<T: serde::Serialize + ?Sized>(self, _: &T) -> Result<(), Stop> {
            Err(Stop)
        }
        fn serialize_newtype_struct<T: serde::Serialize + ?Sized>(
            self,
            _: &'static str,
            _: &T,
        ) -> Result<(), Stop> {
            Err(Stop)
        }
        fn serialize_newtype_variant<T: serde::Serialize + ?Sized>(
            self,
            _: &'static str,
            _: u32,
            _: &'static str,
            _: &T,
        ) -> Result<(), Stop> {
            Err(Stop)
        }
        fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Stop> {
            Err(Stop)
        }
        fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Stop> {
            Err(Stop)
        }
        fn serialize_tuple_struct(
            self,
            _: &'static str,
            _: usize,
        ) -> Result<Self::SerializeTupleStruct, Stop> {
            Err(Stop)
        }
        fn serialize_tuple_variant(
            self,
            _: &'static str,
            _: u32,
            _: &'static str,
            _: usize,
        ) -> Result<Self::SerializeTupleVariant, Stop> {
            Err(Stop)
        }
        fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Stop> {
            Err(Stop)
        }
        fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self, Stop> {
            Ok(self)
        }
        fn serialize_struct_variant(
            self,
            _: &'static str,
            _: u32,
            _: &'static str,
            _: usize,
        ) -> Result<Self::SerializeStructVariant, Stop> {
            Err(Stop)
        }
    }

    /// `XEZIM_CKPT_DIGESTS=1`: every field's digest beside a checkpoint when
    /// it is saved (`<file>.save`) and right after it is restored
    /// (`<file>.restore`); the two lists differ in a field the checkpoint
    /// misses.
    pub fn write_digests(sim: &Simulator, path: &str) {
        if std::env::var_os("XEZIM_CKPT_DIGESTS").is_none() {
            return;
        }
        let mut lines = String::new();
        for (k, d, l) in digests(sim).0 {
            lines.push_str(&format!("{k} {d:016x} {l}\n"));
        }
        let _ = std::fs::write(path, lines);
    }

    /// The digests after setup and the fields seen changed since.
    pub struct Audit {
        before: Digests,
        changed: std::collections::BTreeSet<String>,
        slots: u64,
    }

    /// Start auditing, after setup.
    pub fn start(sim: &mut Simulator) {
        if std::env::var_os("XEZIM_CKPT_AUDIT").is_some() {
            sim.checkpoint_audit = Some(Box::new(Audit {
                before: digests(sim),
                changed: Default::default(),
                slots: 0,
            }));
        }
    }

    fn compare(audit: &mut Audit, now: &Digests) {
        for (b, a) in audit.before.0.iter().zip(&now.0) {
            if b.1 != a.1 || b.2 != a.2 || a.2 == usize::MAX {
                let fail = if a.2 == usize::MAX {
                    " unencodable"
                } else {
                    ""
                };
                audit.changed.insert(format!("{}{}", a.0, fail));
            }
        }
    }

    /// At the end of a slot: a field can change and change back before the
    /// run ends, so compare here too, at the first 64 slots and then at every
    /// power of two.
    pub fn slot(sim: &mut Simulator) {
        let Some(mut audit) = sim.checkpoint_audit.take() else {
            return;
        };
        audit.slots += 1;
        if audit.slots <= 64 || audit.slots.is_power_of_two() {
            let now = digests(sim);
            compare(&mut audit, &now);
        }
        sim.checkpoint_audit = Some(audit);
    }

    /// Append the fields seen changed to the `XEZIM_CKPT_AUDIT` file.
    pub fn finish(sim: &mut Simulator) {
        let Some(mut audit) = sim.checkpoint_audit.take() else {
            return;
        };
        let now = digests(sim);
        compare(&mut audit, &now);
        let Some(path) = std::env::var_os("XEZIM_CKPT_AUDIT") else {
            return;
        };
        let mut lines = String::new();
        for f in &audit.changed {
            lines.push_str(f);
            lines.push('\n');
        }
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = f.write_all(lines.as_bytes());
        }
    }
}
