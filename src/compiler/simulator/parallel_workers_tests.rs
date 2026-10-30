//! §4.6 / §10.4.2: a failed event retains private results until credits drain.
use super::*;
use crate::compiler::bytecode::Insn;

fn event(epoch: Epoch, insns: &[Insn], count: usize) -> Arc<EventWork> {
    let block = Arc::new(BlockPlan {
        index: 0,
        ptr: insns.as_ptr(),
        len: insns.len(),
        regs: 1,
        duplicate_nba: false,
        word: None,
    });
    Arc::new(EventWork {
        epoch,
        tasks: Arc::new(
            (0..count)
                .map(|_| Task {
                    blocks: vec![block.clone()],
                    output: Mutex::new(TaskOutput::default()),
                })
                .collect(),
        ),
        credits: EventCredits::new(count),
        error: Mutex::new(None),
    })
}

#[test]
fn worker_failure_retires_claimed_credit_and_cancels_only_queued_work() {
    let signals = vec![Value::zero(8)];
    let signed = vec![false];
    let names = NameMap::default();
    let arrays = HashMap::default();
    let packed = PackedMem::default();
    let epoch = Epoch {
        values: signals.as_ptr(),
        len: signals.len(),
        signed: signed.as_ptr(),
        signed_len: signed.len(),
        names: &names,
        arrays: &arrays,
        packed: &packed,
        fast: false,
    };
    // A valid private write precedes an internal evaluator failure. Neither
    // that write nor any later task may be published as a successful event.
    let bad = vec![
        Insn::NbaAssignConst(0, Box::new(Value::from_u64(7, 8)), 8),
        Insn::LoadSignal(0, u32::MAX),
    ];
    let failed = event(epoch, &bad, 17);
    let pool = EdgeWorkers::new(2);
    pool.workers[0].mailbox.notify(failed.clone()).unwrap();
    failed.credits.seal();
    failed.credits.wait();
    assert_eq!(failed.credits.completed(), 1);
    assert_eq!(failed.credits.cancelled(), 16);
    assert!(failed.error.lock().unwrap().is_some());
    assert_eq!(signals[0], Value::zero(8));
    assert_eq!(
        failed.tasks[0]
            .output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .writes
            .len(),
        0
    );

    // The same worker must be usable for a new, independent event after the
    // failed event is fully accounted for; it never waits for an idle peer.
    let good = vec![Insn::NbaAssignConst(0, Box::new(Value::from_u64(9, 8)), 8)];
    let next = event(epoch, &good, 1);
    pool.workers[0].mailbox.notify(next.clone()).unwrap();
    next.credits.seal();
    next.credits.wait();
    assert_eq!(next.credits.completed(), 1);
    assert_eq!(next.credits.cancelled(), 0);
    assert!(next.error.lock().unwrap().is_none());
    assert_eq!(
        next.tasks[0].output.lock().unwrap().writes[0].value,
        Value::from_u64(9, 8)
    );
}

#[test]
fn late_notifications_cannot_borrow_a_retired_input_epoch() {
    let epoch = Epoch {
        values: std::ptr::null(),
        len: 1,
        signed: std::ptr::null(),
        signed_len: 1,
        names: std::ptr::null(),
        arrays: std::ptr::null(),
        packed: std::ptr::null(),
        fast: false,
    };
    let instructions = vec![Insn::LoadSignal(0, 0)];
    let retired = event(epoch, &instructions, 1);
    retired.credits.cancel();
    retired.credits.seal();
    retired.credits.wait();
    // Deliberately invalid input pointers prove that the exhausted claim
    // cursor fences every input access, even for a delayed worker notice.
    consume(&retired, &mut Vec::new(), &mut Vec::new());
    assert_eq!(retired.credits.remaining(), 0);
    let mailbox = Mailbox::default();
    mailbox.notify(retired.clone()).unwrap();
    let replacement = event(epoch, &instructions, 0);
    replacement.credits.seal();
    mailbox.notify(replacement.clone()).unwrap();
    assert!(Arc::ptr_eq(&mailbox.receive().unwrap(), &replacement));
    mailbox.close();
    assert!(mailbox.receive().is_none());
}
