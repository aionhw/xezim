//! Output sink for VCD / AITRACE dumps.
//!
//! Two modes:
//!   * Inline   — writes straight to a `BufWriter<File>` on the caller thread.
//!   * Threaded — hands work to a dedicated writer thread. Two message
//!                kinds are carried:
//!                  - `Chunk(Vec<u8>)`: pre-formatted bytes (used for VCD
//!                    headers, AITRACE records, and anything written via
//!                    `std::io::Write`).
//!                  - `VcdBatch(Vec<VcdTimestep>)`: structured per-timestep
//!                    value changes. The worker thread formats them with
//!                    `write_vcd_value`. This moves the bit-by-bit ASCII
//!                    conversion off the main simulation thread, which is
//!                    the actual CPU bottleneck for VCD dumps.
//!                Batches are flushed when `pending.len() >=
//!                `VCD_BATCH_FLUSH` or at `commit()` / `Drop`.
//!
//! `VcdSink` implements `std::io::Write` so existing `writeln!(w, ...)` call
//! sites keep working unchanged.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;

use super::value::{LogicBit, Value};

const CHUNK_CAPACITY: usize = 64 * 1024;
/// Minimum buffered bytes before `commit()` hands a byte chunk to the worker.
const COMMIT_THRESHOLD: usize = 32 * 1024;
/// Number of per-timestep VCD change records to accumulate before dispatch.
const VCD_BATCH_FLUSH: usize = 256;

pub struct VcdTimestep {
    /// `Some(t)` → emit `#t` header before the changes.
    pub time: Option<u64>,
    pub changes: Vec<(String, Value)>,
}

enum WorkerMsg {
    Chunk(Vec<u8>),
    VcdBatch(Vec<VcdTimestep>),
    Shutdown,
}

enum Mode {
    Inline(BufWriter<File>),
    Threaded {
        buf: Vec<u8>,
        pending: Vec<VcdTimestep>,
        tx: Option<Sender<WorkerMsg>>,
        handle: Option<JoinHandle<()>>,
    },
}

pub struct VcdSink {
    mode: Mode,
}

impl VcdSink {
    pub fn inline(file: File) -> Self {
        VcdSink { mode: Mode::Inline(BufWriter::new(file)) }
    }

    pub fn threaded(file: File) -> Self {
        let (tx, rx) = mpsc::channel::<WorkerMsg>();
        let handle = std::thread::Builder::new()
            .name("xezim-vcd".to_string())
            .spawn(move || {
                let mut w = BufWriter::with_capacity(256 * 1024, file);
                while let Ok(msg) = rx.recv() {
                    match msg {
                        WorkerMsg::Chunk(bytes) => { let _ = w.write_all(&bytes); }
                        WorkerMsg::VcdBatch(batch) => {
                            for ts in &batch {
                                if let Some(t) = ts.time {
                                    let _ = writeln!(w, "#{}", t);
                                }
                                for (id, val) in &ts.changes {
                                    write_vcd_value(&mut w, val, id);
                                }
                            }
                        }
                        WorkerMsg::Shutdown => break,
                    }
                }
                let _ = w.flush();
            })
            .expect("spawn xezim-vcd writer thread");
        VcdSink {
            mode: Mode::Threaded {
                buf: Vec::with_capacity(CHUNK_CAPACITY),
                pending: Vec::with_capacity(VCD_BATCH_FLUSH),
                tx: Some(tx),
                handle: Some(handle),
            },
        }
    }

    /// In threaded mode: push a timestep's value changes into the pending
    /// batch (dispatched when the batch is full). In inline mode: format
    /// immediately on the caller thread.
    pub fn post_vcd_changes(&mut self, time: Option<u64>, changes: Vec<(String, Value)>) {
        match &mut self.mode {
            Mode::Inline(w) => {
                if let Some(t) = time {
                    let _ = writeln!(w, "#{}", t);
                }
                for (id, val) in &changes {
                    write_vcd_value(w, val, id);
                }
            }
            Mode::Threaded { buf, pending, tx: Some(tx), .. } => {
                if !buf.is_empty() {
                    let chunk = std::mem::replace(buf, Vec::with_capacity(CHUNK_CAPACITY));
                    let _ = tx.send(WorkerMsg::Chunk(chunk));
                }
                pending.push(VcdTimestep { time, changes });
                if pending.len() >= VCD_BATCH_FLUSH {
                    let batch = std::mem::replace(pending, Vec::with_capacity(VCD_BATCH_FLUSH));
                    let _ = tx.send(WorkerMsg::VcdBatch(batch));
                }
            }
            _ => {}
        }
    }

    /// Hand any pending bytes and VCD batches to the worker. In inline
    /// mode this is a no-op; `BufWriter` handles batching. Called at
    /// natural boundaries; `Drop` flushes whatever is left.
    pub fn commit(&mut self) {
        if let Mode::Threaded { buf, pending, tx: Some(tx), .. } = &mut self.mode {
            if buf.len() >= COMMIT_THRESHOLD {
                let chunk = std::mem::replace(buf, Vec::with_capacity(CHUNK_CAPACITY));
                let _ = tx.send(WorkerMsg::Chunk(chunk));
            }
            if pending.len() >= VCD_BATCH_FLUSH {
                let batch = std::mem::replace(pending, Vec::with_capacity(VCD_BATCH_FLUSH));
                let _ = tx.send(WorkerMsg::VcdBatch(batch));
            }
        }
    }
}

impl Write for VcdSink {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        match &mut self.mode {
            Mode::Inline(w) => w.write(data),
            Mode::Threaded { buf, pending, tx: Some(tx), .. } => {
                if !pending.is_empty() {
                    let batch = std::mem::replace(pending, Vec::with_capacity(VCD_BATCH_FLUSH));
                    let _ = tx.send(WorkerMsg::VcdBatch(batch));
                }
                buf.extend_from_slice(data);
                Ok(data.len())
            }
            Mode::Threaded { buf, .. } => {
                buf.extend_from_slice(data);
                Ok(data.len())
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.mode {
            Mode::Inline(w) => w.flush(),
            Mode::Threaded { .. } => {
                self.commit();
                Ok(())
            }
        }
    }
}

impl Drop for VcdSink {
    fn drop(&mut self) {
        if let Mode::Threaded { buf, pending, tx, handle, .. } = &mut self.mode {
            if let Some(tx_ref) = tx.as_ref() {
                if !buf.is_empty() {
                    let chunk = std::mem::take(buf);
                    let _ = tx_ref.send(WorkerMsg::Chunk(chunk));
                }
                if !pending.is_empty() {
                    let batch = std::mem::take(pending);
                    let _ = tx_ref.send(WorkerMsg::VcdBatch(batch));
                }
            }
            if let Some(tx) = tx.take() {
                let _ = tx.send(WorkerMsg::Shutdown);
                drop(tx);
            }
            if let Some(h) = handle.take() {
                let _ = h.join();
            }
        }
    }
}

/// Format a single `Value` as a VCD value-change record (scalar or vector).
/// Shared by the inline path and the background writer thread.
pub fn write_vcd_value<W: Write>(w: &mut W, val: &Value, id: &str) {
    if val.width == 1 {
        let ch = match val.bits_first() {
            LogicBit::Zero => '0',
            LogicBit::One => '1',
            LogicBit::X => 'x',
            LogicBit::Z => 'z',
        };
        let _ = writeln!(w, "{}{}", ch, id);
    } else {
        let mut s = String::with_capacity(val.width as usize + 2);
        s.push('b');
        let mut all_zero = true;
        for i in (0..val.width as usize).rev() {
            match val.get_bit(i) {
                LogicBit::Zero => {
                    if !all_zero { s.push('0'); }
                }
                LogicBit::One => { all_zero = false; s.push('1'); }
                LogicBit::X => { all_zero = false; s.push('x'); }
                LogicBit::Z => { all_zero = false; s.push('z'); }
            }
        }
        if all_zero { s.push('0'); }
        let _ = writeln!(w, "{} {}", s, id);
    }
}
