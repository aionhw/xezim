//! Ctrl-C / SIGTERM handling.
//!
//! The handler only stores a flag, which is async-signal-safe. The event loop
//! polls it at every time-slot boundary, and long loops poll it on their
//! back-edges, so an interrupted run leaves through the normal exit and
//! `run()` finalizes the waveform dumps.
//!
//! The first signal also arms a backstop alarm: a run that never polls the
//! flag (a blocked DPI call, a loop the polls do not reach) still ends after
//! `INTERRUPT_GRACE_SECS`, through `handle_interrupt_alarm`. Before that
//! happens, the FST guard thread (`fst_sink`) flushes the dump up to the time
//! published in `SIM_TIME`; it holds the alarm off while it writes
//! (`hold_backstop` / `release_backstop`). A second signal ends the process at
//! once.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};

/// Set by the SIGINT/SIGTERM handler; polled by the event loop so an
/// interrupted run still finalizes its waveform dumps.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Set with `INTERRUPTED` and polled on loop back-edges. Unlike
/// `INTERRUPTED` it is cleared when the event loop ends, so the `final`
/// blocks that run after an interrupted run still execute their loops in full.
static LOOP_STOP: AtomicBool = AtomicBool::new(false);

/// The signal that set `INTERRUPTED`, for the SIGALRM backstop to re-raise.
pub(crate) static INTERRUPT_SIGNAL: AtomicI32 = AtomicI32::new(0);

/// The simulation thread has taken the interrupt and is shutting down through
/// the normal path.
static ACKNOWLEDGED: AtomicBool = AtomicBool::new(false);

/// The current simulation time, published by the simulation thread once per
/// time slot (one relaxed store) so the FST guard thread can close a dump at
/// the time a stuck run stopped at.
static SIM_TIME: AtomicU64 = AtomicU64::new(0);

/// Seconds an interrupt may go unnoticed before the process is ended anyway.
pub(crate) const INTERRUPT_GRACE_SECS: u32 = 5;

/// Signal handler. Does the minimum that is async-signal-safe: set a flag. A
/// second signal restores the default action and re-raises, so the user can
/// always force the issue. The first one also arms an alarm: a run stuck in
/// a loop that never polls the flag (`timeout` sends a single SIGTERM) still
/// ends, through `handle_interrupt_alarm`, unless the event loop takes the
/// interrupt in time and disarms it (`acknowledge_interrupt`).
extern "C" fn handle_interrupt(sig: libc::c_int) {
    if INTERRUPTED.swap(true, Ordering::Relaxed) {
        unsafe {
            libc::signal(sig, libc::SIG_DFL);
            libc::raise(sig);
        }
    } else {
        INTERRUPT_SIGNAL.store(sig, Ordering::Relaxed);
        LOOP_STOP.store(true, Ordering::Relaxed);
        unsafe {
            libc::alarm(INTERRUPT_GRACE_SECS);
        }
    }
}

/// SIGALRM after an unanswered interrupt: end the process by the original
/// signal's default action.
extern "C" fn handle_interrupt_alarm(_sig: libc::c_int) {
    let sig = INTERRUPT_SIGNAL.load(Ordering::Relaxed);
    if sig != 0 {
        unsafe {
            libc::signal(sig, libc::SIG_DFL);
            libc::raise(sig);
        }
    }
}

/// Whether `install_interrupt_handler` has run, so `take_pending_interrupt`
/// never unblocks a signal whose default action would end the process.
static HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);

/// Install the SIGINT/SIGTERM handler and the SIGALRM backstop, once.
pub(crate) fn install_interrupt_handler() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        libc::signal(
            libc::SIGINT,
            handle_interrupt as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGTERM,
            handle_interrupt as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGALRM,
            handle_interrupt_alarm as *const () as libc::sighandler_t,
        );
        HANDLER_INSTALLED.store(true, Ordering::Relaxed);
    });
}

/// The exit status of a run that SIGINT or SIGTERM stopped: 128 plus the
/// signal number (130, 143), the status a shell reports for a process that
/// signal ended. The run still leaves through its normal exit and finalizes
/// its dumps first, but a harness must not take a killed run for a passing
/// one. `None` when no signal stopped the run.
pub fn interrupted_exit_status() -> Option<i32> {
    take_pending_interrupt();
    let sig = INTERRUPT_SIGNAL.load(Ordering::Relaxed);
    (sig != 0).then_some(128 + sig)
}

/// Let a SIGINT or SIGTERM that is pending for the process run its handler
/// before the exit status is decided. Linux offers a signal sent to the
/// process or its group to the main thread first, and the main thread only
/// waits for the simulation thread, so it handles the signal whenever it is
/// next scheduled. A run that signals its own group (`kill(0, SIGTERM)` from
/// a VPI routine) could therefore end and pick its status before the handler
/// set the flag, and exit 0 or 143 by timing. Blocking the two signals here
/// makes `sigpending` report one the process has not taken yet; unblocking
/// them again delivers it, on this thread or the main one, and this waits
/// (100 ms at most) until the handler has recorded it.
fn take_pending_interrupt() {
    if !HANDLER_INSTALLED.load(Ordering::Relaxed) || INTERRUPT_SIGNAL.load(Ordering::Relaxed) != 0 {
        return;
    }
    let pending = unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::sigaddset(&mut set, libc::SIGTERM);
        let mut old: libc::sigset_t = std::mem::zeroed();
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, &mut old);
        let mut pend: libc::sigset_t = std::mem::zeroed();
        libc::sigpending(&mut pend);
        let pending = libc::sigismember(&pend, libc::SIGINT) == 1
            || libc::sigismember(&pend, libc::SIGTERM) == 1;
        libc::pthread_sigmask(libc::SIG_SETMASK, &old, std::ptr::null_mut());
        pending
    };
    // `INTERRUPTED` is set first, so a handler that is already running on
    // the main thread is waited for too.
    if pending || INTERRUPTED.load(Ordering::Relaxed) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
        while INTERRUPT_SIGNAL.load(Ordering::Relaxed) == 0 && std::time::Instant::now() < deadline
        {
            std::thread::yield_now();
        }
    }
}

/// Whether Ctrl-C / SIGTERM asked the run to stop. Stays true after the
/// interrupt is acknowledged, so every loop the request reaches unwinds.
#[inline(always)]
pub(crate) fn interrupt_requested() -> bool {
    INTERRUPTED.load(Ordering::Relaxed)
}

/// Polled on loop back-edges: an interrupt is pending and the event loop has
/// not ended yet. One relaxed load of a static.
#[inline(always)]
pub(crate) fn loop_stop_requested() -> bool {
    LOOP_STOP.load(Ordering::Relaxed)
}

/// Address of the flag `loop_stop_requested` reads (an `AtomicBool`, one
/// byte), for native code that polls it on loop back-edges.
#[cfg(feature = "jit")]
pub(crate) fn loop_stop_flag_addr() -> usize {
    &LOOP_STOP as *const AtomicBool as usize
}

/// The event loop has ended: from here on (`final` blocks) loops run to
/// completion again unless a new interrupt arrives.
pub(crate) fn end_loop_stop() {
    LOOP_STOP.store(false, Ordering::Relaxed);
}

/// The event loop has taken the interrupt and is shutting down normally:
/// cancel the backstop so finalizing the dumps is not cut short.
pub(crate) fn acknowledge_interrupt() {
    // SeqCst pairs with `release_backstop`: whichever of the two runs its
    // `alarm` call last sees the other's flag, so a re-armed backstop can
    // never outlive an acknowledgement.
    ACKNOWLEDGED.store(true, Ordering::SeqCst);
    unsafe {
        libc::alarm(0);
    }
}

pub(crate) fn interrupt_acknowledged() -> bool {
    ACKNOWLEDGED.load(Ordering::SeqCst)
}

/// Publish the time of the slot the simulation thread is about to run.
#[inline(always)]
pub(crate) fn publish_sim_time(t: u64) {
    SIM_TIME.store(t, Ordering::Relaxed);
}

pub(crate) fn published_sim_time() -> u64 {
    SIM_TIME.load(Ordering::Relaxed)
}

/// Stop the backstop alarm while a helper thread writes the dump tail.
pub(crate) fn hold_backstop() {
    unsafe {
        libc::alarm(0);
    }
}

/// Re-arm the backstop for what is left of the grace period (at least one
/// second), unless the simulation thread acknowledged the interrupt in the
/// meantime and is finishing normally.
pub(crate) fn release_backstop(since_interrupt: std::time::Duration) {
    let left = (INTERRUPT_GRACE_SECS as u64).saturating_sub(since_interrupt.as_secs());
    unsafe {
        libc::alarm(left.max(1) as libc::c_uint);
    }
    if ACKNOWLEDGED.load(Ordering::SeqCst) {
        unsafe {
            libc::alarm(0);
        }
    }
}
