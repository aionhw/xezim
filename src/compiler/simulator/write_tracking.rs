//! Store-write and event-trigger filters that condition waiters consult.

#[cfg(test)]
mod store_write_mask_tests {
    use super::{name_bit, note_store_write, take_store_writes};

    /// Two simulations in one process each run on their own thread. A drain
    /// on one thread must not clear the writes the other noted: with one
    /// process-wide mask it did, and the other's parked `wait(cond)` never
    /// re-checked (UVM runs hung under `cargo test`, which runs tests
    /// concurrently).
    #[test]
    fn a_drain_on_one_thread_keeps_another_threads_writes() {
        let _ = take_store_writes();
        note_store_write(name_bit("pending"));
        std::thread::spawn(|| {
            let _ = take_store_writes();
            note_store_write(name_bit("other"));
            let _ = take_store_writes();
        })
        .join()
        .unwrap();
        assert_eq!(
            take_store_writes() & name_bit("pending"),
            name_bit("pending")
        );
        assert_eq!(take_store_writes(), 0);
    }

    /// A thread's first drain sees every bit: writes made before it existed
    /// cannot be known, so its first look must re-check everything.
    #[test]
    fn a_new_threads_first_drain_sees_every_bit() {
        let first = std::thread::spawn(take_store_writes).join().unwrap();
        assert_eq!(first, u64::MAX);
    }

    #[test]
    fn simultaneous_drains_preserve_each_threads_mask() {
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            for name in ["worker_left", "worker_right"] {
                let barrier = &barrier;
                scope.spawn(move || {
                    let _ = take_store_writes();
                    let mut mismatches = 0;
                    for _ in 0..64 {
                        note_store_write(name_bit(name));
                        barrier.wait();
                        let observed = take_store_writes();
                        barrier.wait();
                        mismatches += usize::from(observed != name_bit(name));
                        mismatches += usize::from(take_store_writes() != 0);
                    }
                    assert_eq!(mismatches, 0);
                });
            }
        });
    }
}

thread_local! {
    /// Names written since the condition-waiter drain last looked, as a
    /// 64-bit filter over `name_bit`. Set by every mutation of the
    /// STRING-KEYED stores a parked `wait(cond)` can depend on: class
    /// properties (`PropMap`), class statics (`StaticMap`), the runtime signal
    /// map (`SignalMap`), and every procedural / name-based NBA assignment. A
    /// write that does not name its target sets every bit. The id-based RTL
    /// signal table is deliberately NOT counted: a clock toggle must not
    /// re-arm the UVM housekeeping waits (see `drain_condition_waiters`).
    ///
    /// Per THREAD, not per process: a simulation runs on one thread
    /// (`simulate_multi` gives it its own), and two simulations in one
    /// process — the test harness runs them concurrently — shared the old
    /// process-wide mask, so one's drain cleared the bits the other had just
    /// set; that one's parked `wait(cond)` then never re-checked and its run
    /// hung. Starts full, so a thread's first drain re-checks everything.
    pub(super) static STORE_WRITE_MASK: std::cell::Cell<u64> = const { std::cell::Cell::new(u64::MAX) };
}

/// The `STORE_WRITE_MASK` bits of a property / variable name: two bits of
/// an FNV-1a hash, so two short names rarely share both.
#[inline]
pub(super) fn name_bit(n: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in n.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    (1u64 << (h & 63)) | (1u64 << ((h >> 6) & 63))
}

/// Two filter bits of a formal-metadata key's base name (the text before the
/// first `.`), from its first two bytes: constant time on every write (see
/// `Simulator::meta_note`).
#[inline]
pub(super) fn meta_key_bits(key: &str) -> u64 {
    let b = key.as_bytes();
    let b0 = b.first().copied().unwrap_or(0);
    let b1 = match b.get(1) {
        Some(&c) if c != b'.' => c,
        _ => 0,
    };
    (1u64 << (b0 & 63)) | (1u64 << (b1 & 63))
}

#[inline]
pub(super) fn note_store_write(bits: u64) {
    STORE_WRITE_MASK.with(|m| m.set(m.get() | bits));
}

/// An event was triggered or re-bound, so every `<event>.triggered` read may
/// have changed. A stamp is not a property write, so without this a gated
/// `wait(ev.triggered)` would never re-check; the waiters read the name
/// `triggered`, which every trigger notes whatever the event's spelling or
/// alias.
#[inline]
pub(super) fn note_event_trigger() {
    note_store_write(name_bit("triggered"));
}

/// The `STORE_WRITE_MASK` bits of a runtime signal-map key: those of each
/// name it is spelled from outside its index brackets (`top.blk.acc`,
/// `12#m_q[3]`, `Cls::count`), so a write to a module variable does not
/// re-check every gated waiter. A key with another compiler-generated part
/// or an escaped identifier names no source spelling the waiters could
/// share, so it notes every bit. A virtual-interface binding key
/// (`__vif_local__x`, `__vif_return__`) notes its own spelling only: no
/// gated condition reads one (`cond_operand_gated` declines vif-bound
/// names), so a call binding a vif formal need not re-check them all.
pub(super) fn signal_key_bits(k: &str) -> u64 {
    if k.contains('\\') {
        return u64::MAX;
    }
    let b = k.as_bytes();
    let mut bits = 0u64;
    let mut depth = 0u32;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'[' {
            depth += 1;
            i += 1;
        } else if c == b']' {
            depth = depth.saturating_sub(1);
            i += 1;
        } else if depth == 0 && (c.is_ascii_alphanumeric() || c == b'_' || c == b'$') {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'$') {
                i += 1;
            }
            let run = &k[start..i];
            if run.starts_with("__")
                && !run.starts_with("__vif_local__")
                && !run.starts_with("__vif_return__")
            {
                return u64::MAX;
            }
            bits |= name_bit(run);
        } else {
            i += 1;
        }
    }
    if bits == 0 { u64::MAX } else { bits }
}

/// The names written since the last call.
pub(super) fn take_store_writes() -> u64 {
    STORE_WRITE_MASK.with(|m| m.replace(0))
}
