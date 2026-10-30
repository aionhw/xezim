//! Outstanding event credits, including queued work (§4.6 / §10.4.2).
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, Thread};

/// One coordinator creates and waits on an event; consumers never rendezvous.
/// The producer credit prevents quiescence while consumers are being notified.
pub(super) struct EventCredits {
    total: usize,
    next: AtomicUsize,
    remaining: AtomicUsize,
    completed: AtomicUsize,
    cancelled: AtomicUsize,
    sealed: AtomicBool,
    coordinator: Thread,
}

impl EventCredits {
    pub(super) fn new(tasks: usize) -> Self {
        Self {
            total: tasks,
            next: AtomicUsize::new(0),
            remaining: AtomicUsize::new(tasks.checked_add(1).expect("event credit capacity")),
            completed: AtomicUsize::new(0),
            cancelled: AtomicUsize::new(0),
            sealed: AtomicBool::new(false),
            coordinator: thread::current(),
        }
    }

    pub(super) fn claim(&self) -> Option<Ticket<'_>> {
        self.next
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                (next < self.total).then(|| next + 1)
            })
            .ok()
            .map(|index| Ticket { event: self, index })
    }

    /// Cancel only unclaimed work. Claimed tickets remain live until their
    /// evaluator has stopped using the input epoch and published its result.
    pub(super) fn cancel(&self) {
        let unclaimed = self.total - self.next.swap(self.total, Ordering::Relaxed);
        self.cancelled.fetch_add(unclaimed, Ordering::Relaxed);
        self.retire(unclaimed);
    }

    pub(super) fn seal(&self) {
        if !self.sealed.swap(true, Ordering::Relaxed) {
            self.retire(1);
        }
    }

    fn retire(&self, count: usize) {
        if count != 0 && self.remaining.fetch_sub(count, Ordering::AcqRel) == count {
            self.coordinator.unpark();
        }
    }

    pub(super) fn wait(&self) {
        debug_assert_eq!(thread::current().id(), self.coordinator.id());
        while self.remaining() != 0 {
            // unpark retains a token if the last credit retires before park.
            thread::park();
        }
    }

    pub(super) fn remaining(&self) -> usize {
        self.remaining.load(Ordering::Acquire)
    }

    pub(super) fn completed(&self) -> usize {
        self.completed.load(Ordering::Relaxed)
    }

    pub(super) fn cancelled(&self) -> usize {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// Retire after output publication and after all borrowed epoch views end.
/// Unwinding also retires the claimed credit, so cancellation cannot hang.
pub(super) struct Ticket<'a> {
    event: &'a EventCredits,
    index: usize,
}
impl Ticket<'_> {
    pub(super) fn index(&self) -> usize {
        self.index
    }
}
impl Drop for Ticket<'_> {
    fn drop(&mut self) {
        self.event.completed.fetch_add(1, Ordering::Relaxed);
        self.event.retire(1);
    }
}

#[cfg(test)]
#[path = "credit_event_tests.rs"]
mod tests;
