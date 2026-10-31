//! §4.6 / §10.4.2: a failed event retains private results until credits drain.
use super::*;
use crate::compiler::bytecode::Insn;

#[test]
fn task_granularity_preserves_sparse_membership_and_bounds_every_configuration() {
    let blocks: Vec<_> = (0..257)
        .map(|i| {
            (
                i,
                if i % 7 == 0 { u32::MAX } else { 3 },
                [0, 1, 97, usize::MAX][i % 4],
            )
        })
        .collect();
    for lanes in [0, 1, 2, 4] {
        for grain in [0, 1, 2, 4, 8, 16, usize::MAX] {
            let groups = partition_tasks_with_grain(blocks.clone(), lanes, grain);
            assert_eq!(
                groups,
                partition_tasks_with_grain(blocks.clone(), lanes, grain)
            );
            assert!(groups.len() <= lanes.max(1) * grain.clamp(1, 16));
            assert!(groups.iter().all(|group| !group.is_empty()));
            let mut seen: Vec<_> = groups.into_iter().flatten().collect();
            seen.sort_unstable();
            assert_eq!(seen, (0..257).collect::<Vec<_>>());
        }
    }
}

#[test]
fn skewed_locality_groups_use_all_lanes_without_losing_blocks() {
    // A single clock domain holds virtually all the work. Its external id
    // must not limit concurrency, even when the ids are sparse or very large.
    for lanes in [1, 2, 4] {
        let blocks: Vec<_> = (0..257)
            .map(|i| (i, if i == 256 { u32::MAX } else { 7 }, 100))
            .collect();
        let groups = partition_tasks(blocks, lanes);
        assert_eq!(groups.len(), lanes * 4);
        let mut members: Vec<_> = groups.iter().flatten().copied().collect();
        members.sort_unstable();
        assert_eq!(members, (0..257).collect::<Vec<_>>());
        let loads: Vec<_> = groups.iter().map(|g| g.len()).collect();
        assert!(loads.iter().max().unwrap() - loads.iter().min().unwrap() <= 2);
    }
}

#[test]
fn uneven_blocks_have_bounded_load_and_deterministic_assignment() {
    let blocks: Vec<_> = (0..193)
        .map(|i| (i, (i % 13) as u32, [0, 1, 5, 17, 1000][i % 5]))
        .collect();
    for lanes in [1, 2, 4] {
        let groups = partition_tasks(blocks.clone(), lanes);
        assert_eq!(groups, partition_tasks(blocks.clone(), lanes));
        let mut seen = vec![false; blocks.len()];
        let mut loads = Vec::new();
        for group in groups {
            let mut load = 0;
            for bi in group {
                assert!(!std::mem::replace(&mut seen[bi], true));
                load += blocks[bi].2.max(1);
            }
            loads.push(load);
        }
        assert!(seen.iter().all(|&b| b));
        let mean = blocks
            .iter()
            .map(|b| b.2.max(1))
            .sum::<usize>()
            .div_ceil(lanes * 4);
        assert!(*loads.iter().max().unwrap() <= mean * 2 + 1000);
    }
    assert!(partition_tasks(Vec::new(), 4).is_empty());
    assert_eq!(
        partition_tasks(vec![(9, u32::MAX, usize::MAX)], 0),
        vec![vec![9]]
    );
}

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

#[derive(Clone)]
struct MergeWrite {
    signal: usize,
    block: u32,
    tag: u64,
    partial: bool,
}
impl MergeWrite {
    fn entry(&self) -> NbaFast {
        let mut value = Value::from_u64(self.tag, 129);
        value.is_signed = self.tag % 2 == 0;
        value.set_bit(
            100,
            if self.tag % 3 == 0 {
                LogicBit::X
            } else {
                LogicBit::Z
            },
        );
        let mask = self.partial.then(|| {
            let mut mask = Value::zero(129);
            mask.set_bit((self.tag % 129) as usize, LogicBit::One);
            mask.set_bit(128, LogicBit::One);
            Box::new(mask)
        });
        NbaFast {
            signal_id: self.signal,
            value,
            mask,
            block_index: self.block,
        }
    }
}

fn merge_signature(entries: &[NbaFast]) -> Vec<(usize, u32, Value, Option<Box<Value>>)> {
    entries
        .iter()
        .map(|entry| {
            (
                entry.signal_id,
                entry.block_index,
                entry.value.clone(),
                entry.mask.clone(),
            )
        })
        .collect()
}

fn check_merge_run_case<const BATCH: bool, const COUNT: bool>(case: usize) {
    let order: Vec<_> = (0..11).map(|block| (block * 7 + case) % 11).collect();
    let mut blocks: Vec<_> = (0..11u32).collect();
    blocks.sort_by_key(|&block| order[block as usize]);
    let mut state = case as u64 + 1;
    let mut random = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        state
    };
    let tasks: Vec<Vec<MergeWrite>> = (0..case % 9)
        .map(|task| {
            let mut writes = Vec::new();
            for &block in &blocks {
                let count = if task == 0 && case % 5 == 0 {
                    0
                } else {
                    (random() % 14) as usize
                };
                for _ in 0..count {
                    let tag = random();
                    writes.push(MergeWrite {
                        signal: if tag % 3 == 0 {
                            1000 + (tag % 3) as usize
                        } else {
                            (tag % 7) as usize
                        },
                        block,
                        tag,
                        partial: tag % 2 == 0,
                    });
                }
            }
            writes
        })
        .collect();
    // Independent stable-flatten/sort oracle: no heap and no run batching.
    let mut oracle: Vec<_> = tasks
        .iter()
        .enumerate()
        .flat_map(|(task, writes)| {
            writes
                .iter()
                .enumerate()
                .map(move |(position, entry)| (task, position, entry.clone()))
        })
        .collect();
    oracle.sort_by_key(|(task, position, entry)| (order[entry.block as usize], *task, *position));
    let prefix = MergeWrite {
        signal: 3,
        block: u32::MAX,
        tag: 777,
        partial: true,
    };
    let mut expected = vec![prefix.entry()];
    expected.extend(oracle.iter().map(|(_, _, entry)| entry.entry()));
    let mut expected_index = HashMap::default();
    for (position, entry) in expected.iter().enumerate() {
        expected_index.insert(entry.signal_id, position);
    }
    let mutexes: Vec<_> = tasks
        .iter()
        .map(|writes| {
            Mutex::new(TaskOutput {
                writes: writes.iter().map(MergeWrite::entry).collect(),
                ..TaskOutput::default()
            })
        })
        .collect();
    let capacities: Vec<_> = mutexes
        .iter()
        .map(|slot| slot.lock().unwrap().writes.capacity())
        .collect();
    let mut guards: Vec<_> = mutexes.iter().map(|slot| slot.lock().unwrap()).collect();
    let mut actual = vec![prefix.entry()];
    let mut index = NbaFastIndex::with_dense_prefix(8);
    index.insert(3, 0);
    let pops = merge_task_outputs::<BATCH, COUNT>(&mut guards, &order, &mut actual, &mut index);
    assert_eq!(
        merge_signature(&actual),
        merge_signature(&expected),
        "case={case} batch={BATCH} count={COUNT}"
    );
    for (signal, position) in expected_index {
        assert_eq!(index.get(signal), Some(position));
    }
    assert_eq!(index.get(999), None);
    for (guard, capacity) in guards.iter().zip(capacities) {
        assert!(guard.writes.is_empty());
        assert_eq!(
            guard.writes.capacity(),
            capacity,
            "draining lost retained capacity"
        );
    }
    let runs: usize = tasks
        .iter()
        .map(|writes| {
            usize::from(!writes.is_empty())
                + writes
                    .windows(2)
                    .filter(|pair| pair[0].block != pair[1].block)
                    .count()
        })
        .sum();
    let entries: usize = tasks.iter().map(Vec::len).sum();
    assert_eq!(
        pops,
        if !COUNT {
            0
        } else if BATCH {
            runs as u64
        } else {
            entries as u64
        }
    );
}

#[test]
fn batched_heap_merge_preserves_stable_global_order_masks_prefix_and_last_indices() {
    for case in 0..64 {
        check_merge_run_case::<false, false>(case);
        check_merge_run_case::<false, true>(case);
        check_merge_run_case::<true, false>(case);
        check_merge_run_case::<true, true>(case);
    }
}

#[test]
fn disabled_lane_diagnostics_leave_task_metadata_untouched() {
    let values = vec![Value::from_u64(7, 8)];
    let signed = vec![false];
    let names = NameMap::default();
    let arrays = HashMap::default();
    let packed = PackedMem::default();
    let instructions = vec![Insn::NbaAssignConst(0, Box::new(Value::from_u64(9, 8)), 8)];
    let work = event(
        Epoch {
            values: values.as_ptr(),
            len: values.len(),
            signed: signed.as_ptr(),
            signed_len: signed.len(),
            names: &names,
            arrays: &arrays,
            packed: &packed,
            fast: false,
        },
        &instructions,
        1,
    );
    {
        let mut output = work.tasks[0].output.lock().unwrap();
        output.executing_lane = usize::MAX;
        output.busy_ns = u128::MAX;
    }
    work.credits.seal();
    consume(&work, &mut Vec::new(), &mut Vec::new());
    work.credits.wait();
    let output = work.tasks[0].output.lock().unwrap();
    assert_eq!(output.executing_lane, usize::MAX);
    assert_eq!(output.busy_ns, u128::MAX);
    assert_eq!(output.writes[0].value.to_u64(), Some(9));
    assert_eq!(values[0].to_u64(), Some(7));
}

#[test]
fn lane_participation_conserves_actual_tasks_without_requiring_peer_arrivals() {
    let values = vec![Value::from_u64(7, 8)];
    let signed = vec![false];
    let names = NameMap::default();
    let arrays = HashMap::default();
    let packed = PackedMem::default();
    let instructions = vec![Insn::NbaAssignConst(0, Box::new(Value::from_u64(9, 8)), 8)];
    for concurrent in [false, true] {
        for task_count in [0, 1, 37] {
            let work = event(
                Epoch {
                    values: values.as_ptr(),
                    len: values.len(),
                    signed: signed.as_ptr(),
                    signed_len: signed.len(),
                    names: &names,
                    arrays: &arrays,
                    packed: &packed,
                    fast: false,
                },
                &instructions,
                task_count,
            );
            work.credits.seal();
            std::thread::scope(|scope| {
                if concurrent {
                    for lane in 1..4 {
                        let work = &work;
                        scope.spawn(move || {
                            consume_lane::<true>(work, &mut Vec::new(), &mut Vec::new(), lane);
                        });
                    }
                }
                consume_lane::<true>(&work, &mut Vec::new(), &mut Vec::new(), 0);
                work.credits.wait();
            });
            assert_eq!(work.credits.remaining(), 0);
            assert_eq!(work.credits.completed(), task_count);
            assert_eq!(work.credits.cancelled(), 0);
            assert!(work.error.lock().unwrap().is_none());
            let outputs: Vec<_> = work
                .tasks
                .iter()
                .map(|task| task.output.lock().unwrap())
                .collect();
            let mut lanes = vec![LaneParticipation::default(); 4];
            record_lane_outputs(&outputs, &mut lanes);
            assert_eq!(
                lanes.iter().map(|lane| lane.tasks).sum::<u64>(),
                task_count as u64
            );
            assert_eq!(
                lanes.iter().map(|lane| lane.blocks).sum::<u64>(),
                task_count as u64
            );
            assert_eq!(
                lanes.iter().map(|lane| lane.nba_entries).sum::<u64>(),
                task_count as u64
            );
            assert_eq!(
                lanes.iter().map(|lane| lane.busy_ns).sum::<u128>(),
                outputs.iter().map(|output| output.busy_ns).sum::<u128>()
            );
            for lane in &lanes {
                assert_eq!(lane.tasks, lane.blocks);
                assert_eq!(lane.tasks, lane.nba_entries);
            }
            for output in &outputs {
                assert_eq!(output.writes[0].value.to_u64(), Some(9));
            }
            if !concurrent {
                assert_eq!(lanes[0].tasks, task_count as u64);
                assert!(lanes[1..].iter().all(|lane| lane.tasks == 0));
            }
            // Scheduling may give one lane every task. The required property
            // is conservation, rather than participation by every lane.
            assert_eq!(values[0].to_u64(), Some(7));
        }
    }
}

#[test]
fn late_diagnostic_notice_cannot_overwrite_the_next_events_lane() {
    let invalid = Epoch {
        values: std::ptr::null(),
        len: 1,
        signed: std::ptr::null(),
        signed_len: 1,
        names: std::ptr::null(),
        arrays: std::ptr::null(),
        packed: std::ptr::null(),
        fast: false,
    };
    let instructions = vec![Insn::NbaAssignConst(0, Box::new(Value::from_u64(9, 8)), 8)];
    let retired = event(invalid, &instructions, 1);
    retired.credits.cancel();
    retired.credits.seal();
    retired.credits.wait();
    let values = vec![Value::from_u64(7, 8)];
    let signed = vec![false];
    let names = NameMap::default();
    let arrays = HashMap::default();
    let packed = PackedMem::default();
    let active = EventWork {
        epoch: Epoch {
            values: values.as_ptr(),
            len: values.len(),
            signed: signed.as_ptr(),
            signed_len: signed.len(),
            names: &names,
            arrays: &arrays,
            packed: &packed,
            fast: false,
        },
        tasks: retired.tasks.clone(),
        credits: EventCredits::new(1),
        error: Mutex::new(None),
    };
    active.credits.seal();
    consume_lane::<true>(&active, &mut Vec::new(), &mut Vec::new(), 2);
    active.credits.wait();
    let busy_ns = active.tasks[0].output.lock().unwrap().busy_ns;
    consume_lane::<true>(&retired, &mut Vec::new(), &mut Vec::new(), 1);
    let output = active.tasks[0].output.lock().unwrap();
    assert_eq!(output.executing_lane, 2);
    assert_eq!(output.busy_ns, busy_ns);
    assert_eq!(output.writes[0].value.to_u64(), Some(9));
}
