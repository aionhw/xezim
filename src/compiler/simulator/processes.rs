//! Simulator processes.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

mod contexts;
mod event_runs;
mod fsms;
mod intra_timing;
mod process_bytecode;
mod scheduled;
mod stmt_runner;

/// Whether the current thread has used all but its reserve of native stack:
/// an eighth of the stack, at least 256 KiB and at most 64 MiB, which is
/// room to report and unwind. False where the stack bounds are unknown.
fn stack_nearly_exhausted() -> bool {
    thread_local! {
        /// (lowest stack address, reserve), probed once per thread.
        static BOUNDS: std::cell::Cell<Option<Option<(usize, usize)>>> =
            const { std::cell::Cell::new(None) };
    }
    let bounds = BOUNDS.with(|b| {
        if b.get().is_none() {
            b.set(Some(thread_stack_bounds().map(|(low, size)| {
                (low, (size / 8).clamp(256 << 10, 64 << 20))
            })));
        }
        b.get().flatten()
    });
    let Some((low, reserve)) = bounds else {
        return false;
    };
    let here = std::hint::black_box(&low) as *const usize as usize;
    here.saturating_sub(low) < reserve
}

/// The lowest address and the size of the current thread's stack.
#[cfg(target_os = "linux")]
fn thread_stack_bounds() -> Option<(usize, usize)> {
    // SAFETY: `attr` is initialized by `pthread_getattr_np` before it is
    // read and destroyed after; both out-pointers are valid locals.
    unsafe {
        let mut attr: libc::pthread_attr_t = std::mem::zeroed();
        if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) != 0 {
            return None;
        }
        let mut addr: *mut libc::c_void = std::ptr::null_mut();
        let mut size: libc::size_t = 0;
        let ok = libc::pthread_attr_getstack(&attr, &mut addr, &mut size) == 0;
        libc::pthread_attr_destroy(&mut attr);
        (ok && !addr.is_null() && size > 0).then_some((addr as usize, size))
    }
}

/// The lowest address and the size of the current thread's stack.
#[cfg(target_os = "macos")]
fn thread_stack_bounds() -> Option<(usize, usize)> {
    // SAFETY: plain queries about the calling thread.
    unsafe {
        let me = libc::pthread_self();
        let top = libc::pthread_get_stackaddr_np(me) as usize;
        let size = libc::pthread_get_stacksize_np(me);
        (top > size && size > 0).then_some((top - size, size))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn thread_stack_bounds() -> Option<(usize, usize)> {
    None
}
