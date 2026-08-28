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
//! Ordering is preserved because there is exactly one producer and the channel
//! is FIFO. `finish()` is a synchronous rendezvous: it shuts the channel and
//! joins, so the FST trailer is on disk before the call returns.

use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;

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
    /// `(fst id, value, declared-real)`. The flag comes from the DECLARATION
    /// (`dump_var_kind`), not from `Value::is_real`: a var declared
    /// `FstSignalType::real()` has an 8-byte slot in the writer's buffer, so
    /// every change on it must render to 8 bytes. A `real` reads back as an
    /// unset 64-bit X before its first assignment, whose `Value` is NOT
    /// flagged real — keying the render off the value would hand the writer a
    /// 64-byte bit string for an 8-byte slot and panic it.
    pub changes: Vec<(FstSignalId, Value, bool)>,
}

/// Render a `Value` as the FST wire form.
///
/// `is_real` selects the encoding, and must match how the var was DECLARED
/// (see `FstTimestep::changes`):
///
/// * real — the f64's 8 raw little-endian bytes, which is what FST's
///   `SignalType::Real` slot holds and what a viewer decodes back to a double.
/// * otherwise — the bit string: full width, MSB first, '0'/'1'/'x'/'z'.
///   Width-0 yields a single '0' so the writer never sees an empty change.
pub fn fst_format_value(val: &Value, is_real: bool) -> Vec<u8> {
    if is_real {
        // An X/Z real (an unassigned `real`, or one a 4-state expression made
        // unknown) has no f64 image; `to_f64` yields 0.0 there, which is what
        // the VCD path also writes for it.
        return val.to_f64().to_le_bytes().to_vec();
    }
    let w = val.width as usize;
    if w == 0 {
        return vec![b'0'];
    }
    let mut s = Vec::with_capacity(w);
    for i in (0..w).rev() {
        s.push(match val.get_bit(i) {
            LogicBit::Zero => b'0',
            LogicBit::One => b'1',
            LogicBit::X => b'x',
            LogicBit::Z => b'z',
        });
    }
    s
}

enum Msg {
    Batch(Vec<FstTimestep>),
    /// Force a value-change block out to the OS file so a crash (`panic =
    /// "abort"` skips Drop) leaves a readable partial dump.
    Flush,
    /// Write the FST trailer and stop. The worker exits after this.
    Finish,
}

pub type FstBody = FstBodyWriter<std::io::BufWriter<std::fs::File>>;

enum Mode {
    Inline(Box<FstBody>),
    Threaded {
        pending: Vec<FstTimestep>,
        tx: Option<Sender<Msg>>,
        handle: Option<JoinHandle<()>>,
    },
}

pub struct FstSink {
    mode: Mode,
}

/// Crash-safe periodic flush once the in-memory block grows large (matches the
/// fst-writer example's FLUSH_AT).
const FST_FLUSH_AT: usize = 64 * 1024 * 1024;

fn apply(body: &mut FstBody, ts: &FstTimestep) {
    let _ = body.time_change(ts.time);
    for (fid, val, is_real) in &ts.changes {
        let _ = body.signal_change(*fid, &fst_format_value(val, *is_real));
    }
    if body.size() >= FST_FLUSH_AT {
        let _ = body.flush();
    }
}

impl FstSink {
    pub fn inline(body: FstBody) -> Self {
        FstSink { mode: Mode::Inline(Box::new(body)) }
    }

    pub fn threaded(body: FstBody) -> Self {
        let (tx, rx) = mpsc::channel::<Msg>();
        let handle = std::thread::Builder::new()
            .name("xezim-fst".to_string())
            .spawn(move || {
                let mut body = body;
                while let Ok(msg) = rx.recv() {
                    match msg {
                        Msg::Batch(batch) => {
                            for ts in &batch {
                                apply(&mut body, ts);
                            }
                        }
                        Msg::Flush => {
                            let _ = body.flush();
                        }
                        Msg::Finish => break,
                    }
                }
                let _ = body.finish();
            })
            .expect("spawn xezim-fst writer thread");
        FstSink {
            mode: Mode::Threaded {
                pending: Vec::with_capacity(FST_BATCH_FLUSH),
                tx: Some(tx),
                handle: Some(handle),
            },
        }
    }

    pub fn post(&mut self, ts: FstTimestep) {
        match &mut self.mode {
            Mode::Inline(body) => apply(body, &ts),
            Mode::Threaded { pending, tx: Some(tx), .. } => {
                pending.push(ts);
                if pending.len() >= FST_BATCH_FLUSH {
                    let batch = std::mem::take(pending);
                    let _ = tx.send(Msg::Batch(batch));
                }
            }
            _ => {}
        }
    }

    /// Durable flush: push everything buffered here to the worker and have it
    /// write a value-change block out.
    pub fn flush(&mut self) {
        match &mut self.mode {
            Mode::Inline(body) => {
                let _ = body.flush();
            }
            Mode::Threaded { pending, tx: Some(tx), .. } => {
                if !pending.is_empty() {
                    let batch = std::mem::take(pending);
                    let _ = tx.send(Msg::Batch(batch));
                }
                let _ = tx.send(Msg::Flush);
            }
            _ => {}
        }
    }

    /// Write the FST trailer and close the file. Blocks until the worker has
    /// finished, so the dump is complete and readable when this returns.
    pub fn finish(mut self) {
        match &mut self.mode {
            Mode::Inline(_) => {
                if let Mode::Inline(body) = std::mem::replace(
                    &mut self.mode,
                    Mode::Threaded { pending: Vec::new(), tx: None, handle: None },
                ) {
                    let _ = body.finish();
                }
            }
            Mode::Threaded { pending, tx, handle } => {
                if let Some(tx_ref) = tx.as_ref() {
                    if !pending.is_empty() {
                        let batch = std::mem::take(pending);
                        let _ = tx_ref.send(Msg::Batch(batch));
                    }
                    let _ = tx_ref.send(Msg::Finish);
                }
                // Drop the sender so the worker's `recv` cannot block if the
                // Finish message were ever lost, then wait for the trailer.
                drop(tx.take());
                if let Some(h) = handle.take() {
                    let _ = h.join();
                }
            }
        }
    }
}
