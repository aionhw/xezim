use super::EventCredits;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

#[test]
fn empty_event_keeps_producer_credit_until_sealed() {
    let event = EventCredits::new(0);
    assert_eq!(event.remaining(), 1);
    assert!(event.claim().is_none());
    event.seal();
    event.seal();
    event.wait();
    assert_eq!(event.remaining(), 0);
    assert_eq!(event.completed(), 0);
    assert_eq!(event.cancelled(), 0);
}

#[test]
fn skewed_claims_are_unique_and_completion_publishes_results() {
    let event = Arc::new(EventCredits::new(37));
    let hits: Arc<Vec<AtomicUsize>> = Arc::new((0..37).map(|_| AtomicUsize::new(0)).collect());
    let results: Arc<Vec<AtomicUsize>> = Arc::new((0..37).map(|_| AtomicUsize::new(0)).collect());
    let start = Arc::new(Barrier::new(9));
    std::thread::scope(|scope| {
        for worker in 0..8 {
            let event = Arc::clone(&event);
            let hits = Arc::clone(&hits);
            let results = Arc::clone(&results);
            let start = Arc::clone(&start);
            scope.spawn(move || {
                start.wait();
                while let Some(ticket) = event.claim() {
                    let index = ticket.index();
                    assert_eq!(hits[index].fetch_add(1, Ordering::Relaxed), 0);
                    if (worker + index) % 3 == 0 {
                        for _ in 0..7 {
                            std::thread::yield_now();
                        }
                    }
                    results[index].store(index + 1, Ordering::Relaxed);
                    drop(ticket);
                }
            });
        }
        start.wait();
        event.seal();
        event.wait();
        // These reads precede thread joins. Visibility must come from credit
        // completion rather than the scoped-thread join synchronization.
        for index in 0..37 {
            assert_eq!(hits[index].load(Ordering::Relaxed), 1);
            assert_eq!(results[index].load(Ordering::Relaxed), index + 1);
        }
        assert_eq!(event.remaining(), 0);
        assert_eq!(event.completed(), 37);
        assert_eq!(event.cancelled(), 0);
    });
}

#[test]
fn idle_workers_do_not_hold_completion_credits() {
    let event = Arc::new(EventCredits::new(1));
    let ticket = event.claim().unwrap();
    event.seal();
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let event = Arc::clone(&event);
            scope.spawn(move || assert!(event.claim().is_none()));
        }
        drop(ticket);
        event.wait();
        assert_eq!(event.remaining(), 0);
        assert_eq!(event.completed(), 1);
        assert_eq!(event.cancelled(), 0);
    });
}

#[test]
fn cancellation_before_claim_retires_only_unclaimed_work() {
    let event = EventCredits::new(11);
    event.cancel();
    event.cancel();
    assert!(event.claim().is_none());
    // Cancellation cannot release the producer's publication protection.
    assert_eq!(event.remaining(), 1);
    event.seal();
    event.wait();
    assert_eq!(event.remaining(), 0);
    assert_eq!(event.completed(), 0);
    assert_eq!(event.cancelled(), 11);
}

#[test]
fn cancellation_waits_for_an_in_flight_writer() {
    let event = Arc::new(EventCredits::new(7));
    let published = Arc::new(AtomicBool::new(false));
    let claimed = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    std::thread::scope(|scope| {
        let worker_event = Arc::clone(&event);
        let worker_published = Arc::clone(&published);
        let worker_claimed = Arc::clone(&claimed);
        let worker_release = Arc::clone(&release);
        scope.spawn(move || {
            let ticket = worker_event.claim().unwrap();
            worker_claimed.wait();
            worker_release.wait();
            worker_published.store(true, Ordering::Relaxed);
            drop(ticket);
        });
        claimed.wait();
        event.seal();
        event.cancel();
        event.cancel();
        assert_eq!(event.cancelled(), 6);
        assert_eq!(event.remaining(), 1);
        assert!(!published.load(Ordering::Relaxed));
        assert!(event.claim().is_none());
        release.wait();
        event.wait();
        assert!(published.load(Ordering::Relaxed));
        assert_eq!(event.remaining(), 0);
        assert_eq!(event.completed(), 1);
    });
}

#[test]
fn unwinding_a_claim_retires_its_credit_before_failure_drains() {
    let event = Arc::new(EventCredits::new(9));
    let worker_event = Arc::clone(&event);
    let worker = std::thread::spawn(move || {
        std::panic::catch_unwind(|| {
            let _ticket = worker_event.claim().unwrap();
            panic!("synthetic event failure");
        })
        .is_err()
    });
    assert!(worker.join().unwrap());
    event.cancel();
    event.seal();
    event.wait();
    assert_eq!(event.remaining(), 0);
    assert_eq!(event.completed(), 1);
    assert_eq!(event.cancelled(), 8);

    let next = EventCredits::new(1);
    drop(next.claim().unwrap());
    next.seal();
    next.wait();
    assert_eq!(next.remaining(), 0);
    assert_eq!(next.completed(), 1);
    assert_eq!(next.cancelled(), 0);
}

#[test]
fn racing_cancellation_and_claims_conserve_all_work() {
    for _ in 0..16 {
        let event = Arc::new(EventCredits::new(113));
        let start = Arc::new(Barrier::new(7));
        std::thread::scope(|scope| {
            for _ in 0..6 {
                let event = Arc::clone(&event);
                let start = Arc::clone(&start);
                scope.spawn(move || {
                    start.wait();
                    while let Some(ticket) = event.claim() {
                        std::thread::yield_now();
                        drop(ticket);
                    }
                });
            }
            event.seal();
            start.wait();
            event.cancel();
            event.wait();
            assert_eq!(event.remaining(), 0);
            assert_eq!(event.completed() + event.cancelled(), 113);
            assert!(event.claim().is_none());
        });
    }
}
