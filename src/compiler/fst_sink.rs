//! Background writer for FST (GTKWave binary) waveform dumps.
//!
//! The `fst-writer` body writer packs each value change into an in-memory
//! value-change block and periodically compresses and writes it out. Both the
//! packing and the block flush ran on the simulation thread, where they showed
//! up as ~17x the cost of the simulation itself on an unscoped c906 dump.
//!
//! `FstSink` moves the `FstBodyWriter` onto a dedicated thread and feeds it
//! whole timesteps over an mpsc channel. The simulation thread is then left
//! with only change DETECTION (which needs the signal table and so cannot
//! move); rendering, packing, compression and I/O all happen off it.
//!
//! Ordering is preserved because every send happens under the `pending`
//! mutex and the channel is FIFO. `finish()` is a synchronous rendezvous: it
//! shuts the channel and joins, so the FST trailer is on disk before the call
//! returns.
//!
//! Durability. A value-change block lives in memory until it is flushed, and
//! a run that dies without `finish()` (kill -9, OOM, the Ctrl-C backstop)
//! keeps only the flushed blocks: the reader accepts a file without trailer
//! and takes the end time from the last block, and each block reaches the file
//! whole or not at all (see "crash-safe output" below). So:
//! - the writer flushes a block when it reaches `FST_FLUSH_AT` bytes, and also
//!   once `XEZIM_FST_FLUSH_SECS` (default 2 s) of wall time have passed since
//!   the last flush with data waiting (the first block as soon as it has
//!   data);
//! - a guard thread wakes every `GUARD_TICK` and hands the simulation thread's
//!   partial batch to the writer, so no timestep waits on the simulation
//!   thread for longer than that (and a stuck simulation thread does not hold
//!   back the timesteps it already finished);
//! - on Ctrl-C / SIGTERM that the simulation thread has not taken within
//!   `TAIL_AFTER` (a long computation inside one slot, a blocked DPI call),
//!   the guard closes the dump at the slot time the simulation thread
//!   published (`interrupt::published_sim_time`) and flushes it, holding the
//!   backstop alarm off until the block is on disk.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::interrupt;

use fst_writer::{FstBodyWriter, FstSignalId};
use xezim_core::value::{LogicBit, Value};

/// Timesteps buffered on the simulation thread before a channel send. Batching
/// keeps the per-timestep send cost off the hot path without letting the writer
/// fall so far behind that the queue grows without bound.
const FST_BATCH_FLUSH: usize = 64;

/// One time slot: the timestamp plus every value change at it, still in VALUE
/// form. Rendering to the FST wire form (an ASCII bit string) is the bulk of a
/// dump's per-change cost and happens on the WRITER thread — the simulation
/// thread only clones the `Value`, which for the <=64-bit signals that dominate
/// any real design is an inline 24-byte memcpy with no allocation at all.
pub struct FstTimestep {
    pub time: u64,
    /// The width is the immutable FST declaration width; zero denotes a real.
    /// A runtime Value may use a different storage width, but the wire record
    /// must always match its registered handle.
    pub changes: Vec<(FstSignalId, Value, u32)>,
}

/// Render a `Value` as the FST bit string: full width, MSB first, '0'/'1'/'x'/'z'.
/// Width-0 yields a single '0' so the writer never sees an empty change.
pub fn fst_format_value(val: &Value) -> Vec<u8> {
    fst_format_declared_value(val, if val.is_real { 0 } else { val.width.max(1) })
}

/// Format against the FST declaration, not the current storage container.
/// Fixed-width assignments retain low bits on truncation and extend narrow
/// signed/X/Z values using the same rules as the simulator (§10.8).
/// A zero width identifies the real encoding (eight binary bytes).
pub fn fst_format_declared_value(val: &Value, width: u32) -> Vec<u8> {
    if width == 0 {
        let mut real = val.clone();
        real.is_real = true;
        return real.to_f64().to_le_bytes().to_vec();
    }
    let fitted;
    let bits = if val.width != width || val.is_fill {
        fitted = val.resize_for_assign(width);
        &fitted
    } else {
        val
    };
    let mut out = Vec::with_capacity(width as usize);
    for i in (0..width as usize).rev() {
        out.push(match bits.get_bit(i) {
            LogicBit::Zero => b'0',
            LogicBit::One => b'1',
            LogicBit::X => b'x',
            LogicBit::Z => b'z',
        });
    }
    out
}

enum Msg {
    Batch(Vec<FstTimestep>),
    /// Force a value-change block out to the OS file so a crash (`panic =
    /// "abort"` skips Drop) leaves a readable partial dump.
    Flush,
    /// Close the dump at `time` (an empty timestep) and flush it, then
    /// signal `done`. Sent by the guard thread for an interrupt the
    /// simulation thread did not take.
    Tail {
        time: u64,
        done: Sender<()>,
    },
    /// Write the FST trailer and stop. The worker exits after this.
    Finish,
}

pub type FstBody = FstBodyWriter<std::io::BufWriter<std::fs::File>>;

/// Crash-safe periodic flush once the in-memory block grows large (matches the
/// fst-writer example's FLUSH_AT).
const FST_FLUSH_AT: usize = 64 * 1024 * 1024;

/// Default for `XEZIM_FST_FLUSH_SECS`.
const FST_FLUSH_SECS_DEFAULT: f64 = 2.0;

/// The guard thread's period: the longest a finished timestep waits in the
/// simulation thread's batch before it is handed to the writer.
const GUARD_TICK: Duration = Duration::from_millis(100);

/// How long the simulation thread has to take an interrupt before the guard
/// thread writes the dump tail itself.
const TAIL_AFTER: Duration = Duration::from_millis(500);

/// Upper bound on the guard's wait for the writer to finish the tail (it may
/// first have to drain a backlog of batches).
const TAIL_WAIT: Duration = Duration::from_secs(30);

/// `XEZIM_FST_FLUSH_SECS`: wall seconds between time-based block flushes;
/// `0` turns the time rule off (blocks are then flushed at `FST_FLUSH_AT`).
fn flush_interval() -> Option<Duration> {
    let secs = std::env::var("XEZIM_FST_FLUSH_SECS")
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .unwrap_or(FST_FLUSH_SECS_DEFAULT);
    (secs.is_finite() && secs > 0.0).then(|| Duration::from_secs_f64(secs))
}

/// The `FstBodyWriter` plus the bookkeeping the flush rules need.
struct Writer {
    body: FstBody,
    /// Where finished blocks go: the staged, crash-safe output or the file
    /// fst-writer writes directly (see `FstTarget`).
    target: FstTarget,
    /// Latest time handed to `time_change`.
    last_time: u64,
    /// The in-memory block has a time-table entry, so a flush writes a real
    /// block. fst-writer cannot take a value change at a time whose block is
    /// already flushed (`todo!`), and a block without time entries is not
    /// worth writing.
    open: bool,
    flushed_once: bool,
    last_flush: Instant,
    every: Option<Duration>,
}

impl Writer {
    fn new(body: FstBody, mut target: FstTarget, start_time: u64) -> Self {
        target.begin();
        Writer {
            body,
            target,
            last_time: start_time,
            // The t=start snapshot opened the block's time table unless it
            // was taken at time 0 (the buffer's initial end time).
            open: start_time > 0,
            flushed_once: false,
            last_flush: Instant::now(),
            every: flush_interval(),
        }
    }

    fn apply(&mut self, ts: &FstTimestep) {
        if ts.time > self.last_time {
            let _ = self.body.time_change(ts.time);
            self.last_time = ts.time;
            self.open = true;
        } else if self.flushed_once && !self.open {
            // This time's block is already on disk (only after a tail flush,
            // when the simulation thread later finishes the slot it was in);
            // fst-writer cannot add to it.
            return;
        }
        for (fid, val, width) in &ts.changes {
            let _ = self
                .body
                .signal_change(*fid, &fst_format_declared_value(val, *width));
        }
        if self.body.size() >= FST_FLUSH_AT {
            self.flush();
        }
    }

    /// Write the in-memory block out. The block's closing seek empties the
    /// `BufWriter`, so the whole block is in fst-writer's file when
    /// `body.flush` returns, and `commit` moves it into the dump.
    fn flush(&mut self) {
        if !self.open {
            return;
        }
        let _ = self.body.flush();
        self.open = false;
        self.flushed_once = true;
        self.last_flush = Instant::now();
        self.target.commit();
    }

    /// Time to flush under `XEZIM_FST_FLUSH_SECS`.
    /// The first block goes out as soon as it has data: until then the dump
    /// has no value-change block, and GTKWave's reader refuses such a file,
    /// so a run killed in its first interval would leave nothing viewable.
    fn due(&self) -> bool {
        self.open
            && self
                .every
                .is_some_and(|every| !self.flushed_once || self.last_flush.elapsed() >= every)
    }

    /// How long the writer thread may block before the time rule needs it.
    fn idle_wait(&self) -> Option<Duration> {
        if !self.open {
            return None;
        }
        self.every.map(|every| {
            if self.flushed_once {
                every.saturating_sub(self.last_flush.elapsed())
            } else {
                Duration::ZERO
            }
        })
    }

    fn tail(&mut self, time: u64) {
        self.apply(&FstTimestep {
            time: time.max(self.last_time),
            changes: Vec::new(),
        });
        self.flush();
    }

    /// Write the last block and the header totals. `body.finish` drops the
    /// `BufWriter`, so fst-writer's file is complete when `finish` moves the
    /// rest into the dump.
    fn finish(mut self) {
        let _ = self.body.finish();
        self.target.finish();
    }
}

/// The simulation thread's batch and the channel to the writer, behind one
/// mutex: whoever sends (the simulation thread on a full batch, the guard on
/// a tick or a tail) sends in order.
struct Pending {
    batch: Vec<FstTimestep>,
    tx: Option<Sender<Msg>>,
}

impl Pending {
    fn hand_off(&mut self) {
        if self.batch.is_empty() {
            return;
        }
        let batch = std::mem::replace(&mut self.batch, Vec::with_capacity(FST_BATCH_FLUSH));
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Batch(batch));
        }
    }
}

struct Shared {
    pending: Mutex<Pending>,
    closed: AtomicBool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }
}

enum Mode {
    Inline {
        w: Box<Writer>,
        posts: u32,
    },
    Threaded {
        shared: Arc<Shared>,
        writer: Option<JoinHandle<()>>,
        guard: Option<JoinHandle<()>>,
    },
    Done,
}

pub struct FstSink {
    mode: Mode,
}

fn writer_main(mut w: Writer, rx: mpsc::Receiver<Msg>) {
    loop {
        let msg = match w.idle_wait() {
            Some(wait) => match rx.recv_timeout(wait) {
                Ok(m) => m,
                Err(RecvTimeoutError::Timeout) => {
                    w.flush();
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            },
            None => match rx.recv() {
                Ok(m) => m,
                Err(_) => break,
            },
        };
        match msg {
            Msg::Batch(batch) => {
                for ts in &batch {
                    w.apply(ts);
                }
                if w.due() {
                    w.flush();
                }
            }
            Msg::Flush => w.flush(),
            Msg::Tail { time, done } => {
                w.tail(time);
                let _ = done.send(());
            }
            Msg::Finish => break,
        }
    }
    w.finish();
}

fn guard_main(shared: Arc<Shared>) {
    let mut seen: Option<Instant> = None;
    let mut tail_done = false;
    loop {
        std::thread::park_timeout(GUARD_TICK);
        if shared.closed.load(Ordering::Acquire) {
            break;
        }
        shared.lock().hand_off();
        if tail_done || !interrupt::interrupt_requested() {
            continue;
        }
        let since = *seen.get_or_insert_with(Instant::now);
        if interrupt::interrupt_acknowledged() {
            tail_done = true;
        } else if since.elapsed() >= TAIL_AFTER {
            write_tail(&shared, since);
            tail_done = true;
        }
    }
}

/// An interrupt the simulation thread has not taken: close the dump at the
/// published slot time and wait until that block is on disk, with the
/// backstop alarm held off meanwhile.
fn write_tail(shared: &Shared, since: Instant) {
    interrupt::hold_backstop();
    let time = interrupt::published_sim_time();
    let (done_tx, done_rx) = mpsc::channel();
    let sent = {
        let mut p = shared.lock();
        p.hand_off();
        p.tx.as_ref().is_some_and(|tx| {
            tx.send(Msg::Tail {
                time,
                done: done_tx,
            })
            .is_ok()
        })
    };
    if sent {
        eprintln!(
            "[xezim] interrupt not taken within {} ms (the run is inside one time slot) — flushing the FST dump up to time {}",
            TAIL_AFTER.as_millis(),
            time
        );
        if done_rx.recv_timeout(TAIL_WAIT).is_ok() {
            eprintln!("[xezim] FST dump flushed up to time {}", time);
        }
    }
    interrupt::release_backstop(since.elapsed());
}

impl FstSink {
    /// `target` is the output `open_fst` returned with the header writer
    /// `body` came from; `start_time` the time of the initial snapshot
    /// already in `body`.
    pub fn inline(body: FstBody, target: FstTarget, start_time: u64) -> Self {
        FstSink {
            mode: Mode::Inline {
                w: Box::new(Writer::new(body, target, start_time)),
                posts: 0,
            },
        }
    }

    pub fn threaded(body: FstBody, target: FstTarget, start_time: u64) -> Self {
        let w = Writer::new(body, target, start_time);
        let (tx, rx) = mpsc::channel::<Msg>();
        let writer = std::thread::Builder::new()
            .name("xezim-fst".to_string())
            .spawn(move || writer_main(w, rx))
            .expect("spawn xezim-fst writer thread");
        let shared = Arc::new(Shared {
            pending: Mutex::new(Pending {
                batch: Vec::with_capacity(FST_BATCH_FLUSH),
                tx: Some(tx),
            }),
            closed: AtomicBool::new(false),
        });
        let s2 = Arc::clone(&shared);
        // Without the guard the dump still works; it only loses the
        // age-based hand-off and the interrupt tail.
        let guard = std::thread::Builder::new()
            .name("xezim-fst-guard".to_string())
            .spawn(move || guard_main(s2))
            .ok();
        FstSink {
            mode: Mode::Threaded {
                shared,
                writer: Some(writer),
                guard,
            },
        }
    }

    pub fn post(&mut self, ts: FstTimestep) {
        match &mut self.mode {
            Mode::Inline { w, posts } => {
                w.apply(&ts);
                // The clock is read once every 16 timesteps only.
                *posts = posts.wrapping_add(1);
                if *posts % 16 == 0 && w.due() {
                    w.flush();
                }
            }
            Mode::Threaded { shared, .. } => {
                let mut p = shared.lock();
                p.batch.push(ts);
                if p.batch.len() >= FST_BATCH_FLUSH {
                    p.hand_off();
                }
            }
            Mode::Done => {}
        }
    }

    /// Durable flush: push everything buffered here to the worker and have it
    /// write a value-change block out.
    pub fn flush(&mut self) {
        match &mut self.mode {
            Mode::Inline { w, .. } => w.flush(),
            Mode::Threaded { shared, .. } => {
                let mut p = shared.lock();
                p.hand_off();
                if let Some(tx) = &p.tx {
                    let _ = tx.send(Msg::Flush);
                }
            }
            Mode::Done => {}
        }
    }

    /// Write the FST trailer and close the file. Blocks until the worker has
    /// finished, so the dump is complete and readable when this returns.
    pub fn finish(mut self) {
        self.shut_down();
    }

    fn shut_down(&mut self) {
        match std::mem::replace(&mut self.mode, Mode::Done) {
            Mode::Inline { w, .. } => w.finish(),
            Mode::Threaded {
                shared,
                writer,
                guard,
            } => {
                shared.closed.store(true, Ordering::Release);
                {
                    let mut p = shared.lock();
                    p.hand_off();
                    if let Some(tx) = p.tx.take() {
                        let _ = tx.send(Msg::Finish);
                    }
                    // Dropping the sender also ends the worker's `recv` if
                    // the Finish message were ever lost.
                }
                if let Some(g) = guard {
                    g.thread().unpark();
                    let _ = g.join();
                }
                if let Some(h) = writer {
                    let _ = h.join();
                }
            }
            Mode::Done => {}
        }
    }
}

impl Drop for FstSink {
    fn drop(&mut self) {
        self.shut_down();
    }
}

/// Undo `fst-writer`'s break-even time-table encoding in the value-change
/// blocks from file offset `from` on (a block boundary: 0, or a value this
/// function returned). Returns the offset just past the last complete block
/// it walked, so a caller can check each block once as the file grows.
///
/// `write_time_table` picks raw vs zlib storage with `compressed.len() >
/// raw.len()`. The format needs `>=`: a reader treats "compressed length ==
/// uncompressed length" as the sentinel for a section stored RAW, so when
/// zlib output comes out EXACTLY the size of its input the writer emits
/// compressed bytes while recording equal lengths. Readers then skip the
/// inflate and parse zlib's own header as varints — the first two
/// timestamps decode from the `78 5e` magic as 120 and 214 for any design.
///
/// Nothing else about such a file is wrong: header, block chain, GEOM and
/// the declared start/end times all validate, so it passes every structural
/// check and only the per-change times are nonsense. gtkwave's `fst2vcd`
/// "succeeds" on one and prints times orders of magnitude off, which reads
/// as a simulator timing bug rather than a dump bug.
///
/// Break-even needs only a short run whose delta-encoded table is small and
/// incompressible — 19 irregular time steps is enough — so this is not a
/// rare corner. Repairing here keeps the fix inside xezim instead of
/// carrying a patched copy of the crate.
///
/// Best-effort by construction: every failure path leaves the file exactly
/// as the writer left it, because a dump that is merely mis-flagged is far
/// better than one this pass half-rewrote. A repaired block no longer
/// matches the break-even pattern, so running it twice is harmless.
pub fn repair_time_tables(path: &str, from: u64) -> u64 {
    let Ok(mut f) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
    else {
        return from;
    };
    repair_file(&mut f, from).0
}

/// `repair_time_tables` on an open file. Also returns the end time of the
/// last value-change block it walked.
fn repair_file(f: &mut std::fs::File, from: u64) -> (u64, Option<u64>) {
    use std::io::{Read, Seek, SeekFrom, Write};

    let Ok(size) = f.metadata().map(|m| m.len()) else {
        return (from, None);
    };
    let mut last_end_time = None;
    let mut off: u64 = from;
    while off < size {
        if f.seek(SeekFrom::Start(off)).is_err() {
            break;
        }
        let mut head = [0u8; 9];
        if f.read_exact(&mut head).is_err() {
            break;
        }
        let btype = head[0];
        let blen = u64::from_be_bytes(head[1..9].try_into().unwrap_or([0; 8]));
        if blen == 0 || off + 1 + blen > size {
            break; // malformed or still being written: leave it alone
        }
        let end = off + 1 + blen;
        if VC_TYPES.contains(&btype) {
            // Section: length, start time, end time, ...
            let mut t = [0u8; 8];
            if f.seek(SeekFrom::Start(off + 17)).is_ok() && f.read_exact(&mut t).is_ok() {
                last_end_time = Some(u64::from_be_bytes(t));
            }
            // Trailer is (uncompressed_len, compressed_len, item_count).
            let mut tr = [0u8; 24];
            if f.seek(SeekFrom::Start(end - 24)).is_ok() && f.read_exact(&mut tr).is_ok() {
                let unc = u64::from_be_bytes(tr[0..8].try_into().unwrap_or([0; 8]));
                let comp = u64::from_be_bytes(tr[8..16].try_into().unwrap_or([0; 8]));
                if unc == comp && comp >= 2 && comp <= blen {
                    let start = end - 24 - comp;
                    let mut payload = vec![0u8; comp as usize];
                    if f.seek(SeekFrom::Start(start)).is_ok() && f.read_exact(&mut payload).is_ok()
                    {
                        match time_table_fix(&payload) {
                            Some(TimeTableFix::Raw(raw)) => {
                                let _ = f
                                    .seek(SeekFrom::Start(start))
                                    .and_then(|_| f.write_all(&raw));
                            }
                            Some(TimeTableFix::Length(n)) => {
                                let _ = f
                                    .seek(SeekFrom::Start(end - 24))
                                    .and_then(|_| f.write_all(&n.to_be_bytes()));
                            }
                            None => {}
                        }
                    }
                }
            }
        }
        off = end;
    }
    let _ = f.flush();
    (off, last_end_time)
}

/// Value-change block types that carry a trailing time table.
const VC_TYPES: [u8; 3] = [1, 5, 8];

enum TimeTableFix {
    /// The break-even case itself: the lengths are LEGITIMATELY equal, so
    /// correcting a length would change nothing. Store what should have been
    /// stored, the inflated table; same byte count, so no offset moves.
    Raw(Vec<u8>),
    /// The lengths only looked equal; recording the true uncompressed length
    /// makes the reader inflate.
    Length(u64),
}

/// The fix for a time table whose trailer records equal compressed and
/// uncompressed lengths (`payload`, that many bytes), if it is in fact zlib
/// data (see `repair_time_tables`).
fn time_table_fix(payload: &[u8]) -> Option<TimeTableFix> {
    const ZLIB_MAGIC: [[u8; 2]; 4] = [[0x78, 0x9c], [0x78, 0x5e], [0x78, 0x01], [0x78, 0xda]];
    if payload.len() < 2 || !ZLIB_MAGIC.iter().any(|m| payload[..2] == *m) {
        return None;
    }
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib(payload).ok()?;
    Some(if raw.len() == payload.len() {
        TimeTableFix::Raw(raw)
    } else {
        TimeTableFix::Length(raw.len() as u64)
    })
}

// ───────────────────────── crash-safe output ─────────────────────────
//
// fst-writer writes a value-change block through an 8 KB `BufWriter`: a dummy
// section length first, the body in 8 KB pieces as the buffer fills, then a
// seek back to patch the length. A `kill -9` (or OOM kill, or the second
// Ctrl-C) during those writes leaves a block whose length is still 0, or whose
// body stops short, and readers reject the whole file: fst-reader asserts
// `compressed_length <= section_length` (it takes the time-table trailer from
// 24 bytes before a block of "length 0"), and the reference FST library
// refuses to open it. Under load the window is wide: a block of a few MB takes
// hundreds of 8 KB writes.
//
// fst-writer only opens files by path (`open_fst` wraps a `File` in its own
// `BufWriter`), so xezim cannot hand it a buffering writer. Instead it lets
// fst-writer write into a STAGE, an anonymous in-memory file (memfd), and moves
// each finished block into the dump itself:
//
// - The dump always ends in a terminator, an empty SKIP block (type 255,
//   length 0), at which every reader stops. Its 9 bytes never cross a page
//   boundary of the file.
// - A block goes in with two writes. First everything after its 9-byte head,
//   plus a new terminator, lands BEHIND the current terminator: readers still
//   stop at the old one, so a kill here loses only this block. Then the head
//   overwrites the old terminator in one 9-byte write within one page, which
//   a kill cannot split (the kernel checks for a fatal signal only between
//   pages of a write). Before it the block is invisible, after it complete.
// - The header, hierarchy and geometry go in by writing a temporary file and
//   renaming it over the dump, so the path never holds a partial header.
// - At the end the final header (end time and counts, one write inside the
//   first page) goes in last, then the terminator is truncated away; the
//   finished file is byte for byte what fst-writer would have written, with
//   the time tables repaired.
//
// The time-table repair now runs on each block in memory before it is
// written, so the dump is never rewritten in place.

/// Bytes of an empty SKIP block, the terminator every reader stops at.
const TERMINATOR: [u8; 9] = [255, 0, 0, 0, 0, 0, 0, 0, 0];
/// A SKIP block that skips nothing (its length counts only the length field):
/// padding that moves a terminator off a page boundary.
const PAD_SKIP: [u8; 9] = [255, 0, 0, 0, 0, 0, 0, 0, 8];
const PAGE: u64 = 4096;
/// Stage-to-dump copy buffer.
const COPY_CHUNK: usize = 1 << 20;
/// fst-writer's header block: type byte plus a 329-byte section.
const HEADER_BYTES: usize = 330;

/// Bytes to write at `at` so that a terminator follows: a pad when a
/// terminator at `at` would cross a page boundary. Returns the bytes and the
/// terminator's offset.
fn terminator_at(at: u64) -> (Vec<u8>, u64) {
    if at % PAGE > PAGE - TERMINATOR.len() as u64 {
        let mut v = PAD_SKIP.to_vec();
        v.extend_from_slice(&TERMINATOR);
        (v, at + PAD_SKIP.len() as u64)
    } else {
        (TERMINATOR.to_vec(), at)
    }
}

/// Test-only fault injection (`XEZIM_FST_KILL_AT=<point>[:<n>]`): SIGKILL the
/// process at a named point of the n-th block commit (default 1st), or of
/// the final one, to check what a kill leaves on disk.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum KillPoint {
    /// The dump file is in place, before the first block.
    Initial,
    /// Half of a block's body written behind the terminator.
    Torn,
    /// A block's body written, its head not.
    Body,
    /// A block committed.
    Committed,
    /// The last block committed, the final header not written.
    Last,
    /// The final header written, the terminator not truncated.
    Header,
}

fn kill_hook() -> Option<(KillPoint, u64)> {
    let v = std::env::var("XEZIM_FST_KILL_AT").ok()?;
    let (p, n) = match v.split_once(':') {
        Some((p, n)) => (p.trim().to_string(), n.trim().parse().ok()?),
        None => (v.trim().to_string(), 1),
    };
    let p = match p.as_str() {
        "initial" => KillPoint::Initial,
        "torn" => KillPoint::Torn,
        "body" => KillPoint::Body,
        "committed" => KillPoint::Committed,
        "last" => KillPoint::Last,
        "header" => KillPoint::Header,
        _ => return None,
    };
    Some((p, n))
}

/// Where fst-writer's bytes end up: see `open_fst`.
pub struct FstTarget {
    inner: TargetKind,
}

enum TargetKind {
    /// fst-writer writes the dump directly (no memfd on this system); only
    /// the time-table repair runs after each flush.
    Direct { path: String, repaired_to: u64 },
    #[cfg(unix)]
    Staged(Box<Staged>),
}

/// Open an FST dump at `path`. fst-writer writes into a stage, an in-memory
/// file on Linux (a temporary file, unlinked at once, on other Unix systems or
/// with `XEZIM_FST_STAGE=file`), and `FstSink` moves each finished block into
/// `path` crash-safely. Where no stage can be made it writes `path` directly.
pub fn open_fst(
    path: &str,
    info: &fst_writer::FstInfo,
) -> Result<
    (
        fst_writer::FstHeaderWriter<std::io::BufWriter<std::fs::File>>,
        FstTarget,
    ),
    fst_writer::FstWriteError,
> {
    #[cfg(unix)]
    {
        let staged = |stage, path: &str| FstTarget {
            inner: TargetKind::Staged(Box::new(Staged::new(stage, path))),
        };
        #[cfg(target_os = "linux")]
        if std::env::var("XEZIM_FST_STAGE").map_or(true, |v| v.trim() != "file") {
            if let Some((stage, stage_path)) = Staged::memfd_stage() {
                if let Ok(header) = fst_writer::open_fst(&stage_path, info) {
                    return Ok((header, staged(stage, path)));
                }
            }
        }
        let stage_path = Staged::file_stage_path(path);
        if let Ok(header) = fst_writer::open_fst(&stage_path, info) {
            let stage = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&stage_path);
            let _ = std::fs::remove_file(&stage_path);
            if let Ok(stage) = stage {
                return Ok((header, staged(stage, path)));
            }
        }
    }
    let header = fst_writer::open_fst(path, info)?;
    Ok((
        header,
        FstTarget {
            inner: TargetKind::Direct {
                path: path.to_string(),
                repaired_to: 0,
            },
        },
    ))
}

impl FstTarget {
    /// The header writer is finished: put the header, hierarchy and geometry
    /// in place.
    fn begin(&mut self) {
        match &mut self.inner {
            TargetKind::Direct { .. } => {}
            #[cfg(unix)]
            TargetKind::Staged(s) => {
                let r = s.begin();
                s.report(r);
            }
        }
    }

    /// fst-writer has written a block: move it into the dump.
    fn commit(&mut self) {
        match &mut self.inner {
            TargetKind::Direct { path, repaired_to } => {
                *repaired_to = repair_time_tables(path, *repaired_to);
            }
            #[cfg(unix)]
            TargetKind::Staged(s) => {
                let r = s.commit();
                s.report(r);
            }
        }
    }

    /// fst-writer has written its last block and the header totals.
    fn finish(&mut self) {
        match &mut self.inner {
            TargetKind::Direct { path, repaired_to } => {
                repair_time_tables(path, *repaired_to);
            }
            #[cfg(unix)]
            TargetKind::Staged(s) => {
                let r = s.finish();
                s.report(r);
            }
        }
    }
}

#[cfg(unix)]
struct Staged {
    /// The stage fst-writer writes, through its own descriptor: a memfd
    /// opened by its `/proc/self/fd` path, or an unlinked temporary file.
    stage: std::fs::File,
    path: String,
    /// The dump, once `begin` has put it in place.
    out: Option<std::fs::File>,
    /// Stage bytes `[0, staged)` are in the dump.
    staged: u64,
    /// Dump offset of the terminator.
    end: u64,
    commits: u64,
    kill: Option<(KillPoint, u64)>,
    /// `XEZIM_FST_COMMIT_LOG`: print each committed block's end time.
    log: bool,
    warned: bool,
}

#[cfg(unix)]
impl Staged {
    #[cfg(target_os = "linux")]
    fn memfd_stage() -> Option<(std::fs::File, String)> {
        use std::os::fd::FromRawFd;
        let fd = unsafe { libc::memfd_create(c"xezim-fst".as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return None;
        }
        let stage = unsafe { std::fs::File::from_raw_fd(fd) };
        let stage_path = format!("/proc/self/fd/{fd}");
        std::fs::metadata(&stage_path).ok()?;
        Some((stage, stage_path))
    }

    /// A temporary file beside the dump (so it is on a file system the run
    /// can write); `open_fst` unlinks it once both sides have it open.
    fn file_stage_path(path: &str) -> std::path::PathBuf {
        let p = std::path::Path::new(path);
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("dump.fst");
        p.with_file_name(format!(".{name}.xezim-stage-{}", std::process::id()))
    }

    fn new(stage: std::fs::File, path: &str) -> Self {
        Staged {
            stage,
            path: path.to_string(),
            out: None,
            staged: 0,
            end: 0,
            commits: 0,
            kill: kill_hook(),
            log: std::env::var_os("XEZIM_FST_COMMIT_LOG").is_some(),
            warned: false,
        }
    }

    fn report(&mut self, r: std::io::Result<()>) {
        if let Err(e) = r {
            if !self.warned {
                self.warned = true;
                eprintln!("Warning: cannot write FST file '{}': {}", self.path, e);
            }
        }
    }

    /// `n` is the commit count; `Initial`, `Last` and `Header` happen once
    /// and ignore it.
    fn maybe_kill(&self, at: KillPoint, n: u64) {
        let hit = self.kill.is_some_and(|(p, k)| {
            p == at
                && (k == n || matches!(p, KillPoint::Initial | KillPoint::Last | KillPoint::Header))
        });
        if hit {
            unsafe {
                libc::kill(libc::getpid(), libc::SIGKILL);
            }
        }
    }

    /// Return stage memory below `upto` except the first page (the header,
    /// which fst-writer patches at the end).
    #[cfg(not(target_os = "linux"))]
    fn release_stage(&self, _upto: u64) {}

    #[cfg(target_os = "linux")]
    fn release_stage(&self, upto: u64) {
        use std::os::fd::AsRawFd;
        let lo = PAGE;
        let hi = upto / PAGE * PAGE;
        if hi > lo {
            unsafe {
                libc::fallocate(
                    self.stage.as_raw_fd(),
                    libc::FALLOC_FL_PUNCH_HOLE | libc::FALLOC_FL_KEEP_SIZE,
                    lo as libc::off_t,
                    (hi - lo) as libc::off_t,
                );
            }
        }
    }

    /// Put the header, hierarchy and geometry (all of the stage so far) in
    /// place at `path`, with a terminator.
    fn begin(&mut self) -> std::io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::FileExt;
        let staged = self.stage.metadata()?.len();
        let mut data = vec![0u8; staged as usize];
        self.stage.read_exact_at(&mut data, 0)?;
        let (term, term_at) = terminator_at(data.len() as u64);
        data.extend_from_slice(&term);
        // A temporary file renamed over the dump, so the path never holds a
        // partial header. Not for an existing non-file (a device, a pipe) or
        // a symlink, which the rename would replace, nor where no temporary
        // file can be made (a read-only directory): those are written in
        // place.
        let p = std::path::Path::new(&self.path);
        let mut out = None;
        if std::fs::symlink_metadata(p).map_or(true, |m| m.file_type().is_file()) {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("dump.fst");
            let tmp = p.with_file_name(format!(".{name}.xezim-{}", std::process::id()));
            if let Ok(mut f) = std::fs::File::create(&tmp) {
                if let Err(e) = f.write_all(&data).and_then(|_| std::fs::rename(&tmp, p)) {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(e);
                }
                out = Some(f);
            }
        }
        let out = match out {
            Some(f) => f,
            None => {
                let mut f = std::fs::OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .create(true)
                    .open(p)?;
                f.write_all(&data)?;
                f
            }
        };
        self.out = Some(out);
        self.staged = staged;
        self.end = term_at;
        self.maybe_kill(KillPoint::Initial, 1);
        Ok(())
    }

    /// Move stage bytes `[from, to)` (whole blocks) into the dump: first
    /// everything after the first block's 9-byte head, behind the terminator,
    /// plus a new terminator; then the head over the old terminator, which
    /// commits them.
    fn append(&mut self, from: u64, to: u64) -> std::io::Result<()> {
        use std::os::unix::fs::FileExt;
        let Some(out) = &self.out else {
            return Ok(());
        };
        let head_len = TERMINATOR.len() as u64;
        if to < from + head_len {
            return Ok(());
        }
        let n = self.commits + 1;
        let tail_at = self.end + (to - from);
        // Copied in pieces through one buffer: the pieces all land behind
        // the terminator, so their order does not matter to a reader.
        let body_from = from + head_len;
        let torn_at = if self.kill == Some((KillPoint::Torn, n)) {
            body_from + (to - body_from) / 2
        } else {
            u64::MAX
        };
        let mut buf = vec![0u8; ((to - body_from) as usize).min(COPY_CHUNK)];
        let mut at = body_from;
        while at < to {
            let mut len = ((to - at) as usize).min(buf.len());
            if at < torn_at {
                len = len.min((torn_at - at) as usize);
            }
            self.stage.read_exact_at(&mut buf[..len], at)?;
            out.write_all_at(&buf[..len], self.end + (at - from))?;
            at += len as u64;
            if at >= torn_at {
                self.maybe_kill(KillPoint::Torn, n);
            }
        }
        let (term, term_at) = terminator_at(tail_at);
        out.write_all_at(&term, tail_at)?;
        self.maybe_kill(KillPoint::Body, n);
        // The commit: 9 bytes inside one page (`terminator_at`).
        let mut head = [0u8; TERMINATOR.len()];
        self.stage.read_exact_at(&mut head, from)?;
        out.write_all_at(&head, self.end)?;
        self.end = term_at;
        self.commits = n;
        Ok(())
    }

    fn commit(&mut self) -> std::io::Result<()> {
        if self.out.is_none() {
            return Ok(());
        }
        let size = self.stage.metadata()?.len();
        if size <= self.staged {
            return Ok(());
        }
        // The time-table repair runs in the stage, before the blocks reach
        // the dump; fst-writer never comes back to them.
        let (_, end_time) = repair_file(&mut self.stage, self.staged);
        self.append(self.staged, size)?;
        if self.log {
            if let Some(t) = end_time {
                eprintln!("[xezim] fst-commit {t}");
            }
        }
        self.maybe_kill(KillPoint::Committed, self.commits);
        self.staged = size;
        self.release_stage(self.staged);
        Ok(())
    }

    /// The last block, then the final header, then drop the terminator.
    fn finish(&mut self) -> std::io::Result<()> {
        use std::os::unix::fs::FileExt;
        self.commit()?;
        let n = self.commits;
        self.maybe_kill(KillPoint::Last, n);
        let Some(out) = &self.out else {
            return Ok(());
        };
        let mut header = [0u8; HEADER_BYTES];
        self.stage.read_exact_at(&mut header, 0)?;
        out.write_all_at(&header, 0)?;
        self.maybe_kill(KillPoint::Header, n);
        out.set_len(self.end)?;
        Ok(())
    }
}

/// `--fst-repack[=level]`, process-wide like the other dump knobs: 0 is off.
static FST_REPACK_LEVEL: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Set (or clear) the deflate level `--fst-repack` wraps a finished dump at.
pub fn set_fst_repack(level: Option<u8>) {
    FST_REPACK_LEVEL.store(level.unwrap_or(0), Ordering::Relaxed);
}

/// The `--fst-repack` level in force, if any.
pub fn fst_repack_level() -> Option<u8> {
    match FST_REPACK_LEVEL.load(Ordering::Relaxed) {
        0 => None,
        l => Some(l),
    }
}

/// GTKWave's "repack on close" (`fstWriterSetRepackOnClose`): the finished
/// dump, gzip-compressed as one stream and wrapped in a `FST_BL_ZWRAPPER`
/// (254) block — the type byte, the section length and the uncompressed
/// length as big-endian 64-bit integers, then the gzip stream. Readers
/// (GTKWave, `fst-reader`) inflate it on open, so the wrapper trades open
/// time and a temporary copy for a smaller file. Runs only on a clean
/// close, after the trailer is on disk; a run that dies earlier leaves the
/// unwrapped, crash-safe dump as before. Streams through fixed buffers, so
/// a dump larger than memory repacks too. Returns (before, after) sizes.
pub fn repack_on_close(path: &str, level: u8) -> std::io::Result<(u64, u64)> {
    use miniz_oxide::deflate::core::{CompressorOxide, create_comp_flags_from_zip_params};
    use miniz_oxide::deflate::stream::deflate;
    use miniz_oxide::{MZFlush, MZStatus};
    use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};

    // A linked dump path is repacked into the link's target: renaming over
    // the link itself made it a regular file and left the target holding
    // the uncompressed dump (#598). The FST writer writes through links too.
    let resolved = std::fs::canonicalize(path)?;
    let path = resolved.to_str().unwrap_or(path);
    let src_len = std::fs::metadata(path)?.len();
    let tmp = format!("{path}.repack.{}", std::process::id());
    let result = (|| -> std::io::Result<u64> {
        let mut inp = BufReader::with_capacity(1 << 20, std::fs::File::open(path)?);
        let mut out = BufWriter::with_capacity(1 << 20, std::fs::File::create(&tmp)?);
        out.write_all(&[254u8])?;
        out.write_all(&0u64.to_be_bytes())?; // section length, patched below
        out.write_all(&src_len.to_be_bytes())?;
        // gzip member header: magic, deflate, no flags, no mtime, OS = Unix.
        out.write_all(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3])?;
        let mut gz_len: u64 = 10;
        let flags = create_comp_flags_from_zip_params(i32::from(level.clamp(1, 10)), 0, 0);
        let mut comp = Box::new(CompressorOxide::new(flags));
        let mut inbuf = vec![0u8; 1 << 20];
        let mut outbuf = vec![0u8; 1 << 20];
        let mut crc = Crc32::new();
        loop {
            let n = inp.read(&mut inbuf)?;
            let eof = n == 0;
            crc.update(&inbuf[..n]);
            let flush = if eof { MZFlush::Finish } else { MZFlush::None };
            let mut pos = 0;
            loop {
                let res = deflate(&mut comp, &inbuf[pos..n], &mut outbuf, flush);
                pos += res.bytes_consumed;
                out.write_all(&outbuf[..res.bytes_written])?;
                gz_len += res.bytes_written as u64;
                match res.status {
                    Ok(MZStatus::StreamEnd) => break,
                    Ok(_) => {}
                    Err(e) => {
                        return Err(std::io::Error::other(format!("deflate: {e:?}")));
                    }
                }
                if !eof && pos >= n {
                    break;
                }
            }
            if eof {
                break;
            }
        }
        out.write_all(&crc.finish().to_le_bytes())?;
        out.write_all(&(src_len as u32).to_le_bytes())?;
        gz_len += 8;
        out.flush()?;
        let mut f = out.into_inner().map_err(|e| e.into_error())?;
        f.seek(SeekFrom::Start(1))?;
        f.write_all(&(8 + 8 + gz_len).to_be_bytes())?;
        f.sync_all()?;
        Ok(1 + 8 + 8 + gz_len)
    })();
    // A failed rename is cleaned up like a failed compression, so repeated
    // failures do not leave dump-sized files beside the dump (#598).
    match result.and_then(|total| std::fs::rename(&tmp, path).map(|()| total)) {
        Ok(total) => Ok((src_len, total)),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// CRC-32 (IEEE 802.3), as the gzip member trailer needs it.
struct Crc32 {
    table: [u32; 256],
    state: u32,
}

impl Crc32 {
    fn new() -> Self {
        let mut table = [0u32; 256];
        for (i, slot) in table.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        Self {
            table,
            state: 0xFFFF_FFFF,
        }
    }
    fn update(&mut self, data: &[u8]) {
        for &b in data {
            self.state = self.table[((self.state ^ b as u32) & 0xFF) as usize] ^ (self.state >> 8);
        }
    }
    fn finish(&self) -> u32 {
        self.state ^ 0xFFFF_FFFF
    }
}
