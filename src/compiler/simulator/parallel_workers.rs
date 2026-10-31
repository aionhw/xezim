//! Credit-scheduled workers with immutable event inputs (§4.6 / §10.4.2).
use super::credit_event::EventCredits;
use super::parallel_word::WordProgram;
use super::*;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

struct BlockPlan {
    index: u32,
    ptr: *const super::super::bytecode::Insn,
    len: usize,
    regs: usize,
    duplicate_nba: bool,
    word: Option<WordProgram>,
}

// Partition ids express locality, not a publication boundary: every task reads
// the same immutable event, and all writes are merged after credit quiescence.
// Split a large locality group before packing chunks, so one clock domain or
// scope cannot leave all but one execution lane idle. Sparse ids cost no space.
#[cfg(test)]
fn partition_tasks(blocks: Vec<(usize, u32, usize)>, lanes: usize) -> Vec<Vec<usize>> {
    partition_tasks_with_grain(blocks, lanes, 4)
}

fn partition_tasks_with_grain(
    mut blocks: Vec<(usize, u32, usize)>,
    lanes: usize,
    tasks_per_lane: usize,
) -> Vec<Vec<usize>> {
    if blocks.is_empty() {
        return Vec::new();
    }
    let tasks = blocks
        .len()
        .min(lanes.max(1).saturating_mul(tasks_per_lane.clamp(1, 16)));
    let total: u128 = blocks.iter().map(|b| b.2.max(1) as u128).sum();
    let target = total.div_ceil(tasks as u128);
    // Stable order within each locality group keeps neighboring blocks close.
    blocks.sort_by_key(|b| b.1);
    let mut chunks = Vec::new();
    let mut chunk = Vec::new();
    let mut cost = 0u128;
    let mut locality = blocks[0].1;
    for (bi, part, weight) in blocks {
        let weight = weight.max(1) as u128;
        if !chunk.is_empty() && (part != locality || cost + weight > target) {
            chunks.push((cost, std::mem::take(&mut chunk)));
            cost = 0;
        }
        locality = part;
        chunk.push(bi);
        cost += weight;
    }
    if !chunk.is_empty() {
        chunks.push((cost, chunk));
    }
    chunks.sort_by_key(|c| std::cmp::Reverse(c.0));
    let mut groups: Vec<Vec<usize>> = (0..tasks).map(|_| Vec::new()).collect();
    let mut costs = vec![0u128; tasks];
    for (cost, mut chunk) in chunks {
        let task = (0..tasks).min_by_key(|&i| (costs[i], i)).unwrap();
        costs[task] += cost;
        groups[task].append(&mut chunk);
    }
    // The shared credit cursor exposes the longest tasks first.
    let mut groups: Vec<_> = groups
        .into_iter()
        .zip(costs)
        .filter(|g| !g.0.is_empty())
        .collect();
    groups.sort_by_key(|g| std::cmp::Reverse(g.1));
    groups.into_iter().map(|g| g.0).collect()
}
// Plans contain only positively admitted immutable instructions. Their backing
// allocations stay alive until all event credits retire; no AST fallback runs.
unsafe impl Send for BlockPlan {}
unsafe impl Sync for BlockPlan {}

#[derive(Clone, Copy)]
struct Epoch {
    values: *const Value,
    len: usize,
    signed: *const bool,
    signed_len: usize,
    names: *const NameMap,
    arrays: *const HashMap<Arc<str>, (usize, i64, i64)>,
    packed: *const PackedMem,
    fast: bool,
}
// Evaluators retire their credit only after every shared input borrow ends.
// Once credits reach zero, stale notifications can only find an empty cursor;
// they cannot dereference this epoch after the coordinator regains mutability.
unsafe impl Send for Epoch {}
unsafe impl Sync for Epoch {}

#[derive(Default)]
struct TaskOutput {
    writes: Vec<NbaFast>,
    fast: u64,
    fallback: u64,
    executing_lane: usize,
    // Elapsed evaluator wall time, including descheduling; not CPU time.
    busy_ns: u128,
}

#[derive(Clone, Default)]
struct LaneParticipation {
    tasks: u64,
    blocks: u64,
    nba_entries: u64,
    busy_ns: u128,
}

fn record_lane_outputs(
    outputs: &[std::sync::MutexGuard<'_, TaskOutput>],
    lanes: &mut [LaneParticipation],
) {
    for output in outputs {
        let lane = &mut lanes[output.executing_lane];
        lane.tasks += 1;
        lane.blocks += output.fast + output.fallback;
        lane.nba_entries += output.writes.len() as u64;
        lane.busy_ns += output.busy_ns;
    }
}

struct Task {
    blocks: Vec<Arc<BlockPlan>>,
    // A claimed task has one writer. The slot retains capacity across events;
    // the coordinator takes ownership of results only after credit quiescence.
    output: Mutex<TaskOutput>,
}
// Every task is ordered by the global block rank. Heap ties prefer the
// lower task index, so a consecutive run from one block can drain intact.
fn merge_task_outputs<const BATCH: bool, const COUNT: bool>(
    outputs: &mut [std::sync::MutexGuard<'_, TaskOutput>],
    order: &[usize],
    nba: &mut Vec<NbaFast>,
    index: &mut NbaFastIndex,
) -> u64 {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let mut drains: Vec<_> = outputs.iter_mut().map(|o| o.writes.drain(..)).collect();
    let mut heads: Vec<_> = drains.iter_mut().map(Iterator::next).collect();
    let mut ready = BinaryHeap::new();
    let mut pops = 0;
    for (i, head) in heads.iter().enumerate() {
        if let Some(n) = head {
            ready.push(Reverse((order[n.block_index as usize], i)));
        }
    }
    while let Some(Reverse((_, i))) = ready.pop() {
        if COUNT {
            pops += 1;
        }
        let block = heads[i].as_ref().unwrap().block_index;
        loop {
            let entry = heads[i].take().unwrap();
            index.insert(entry.signal_id, nba.len());
            nba.push(entry);
            heads[i] = drains[i].next();
            if BATCH && heads[i].as_ref().is_some_and(|n| n.block_index == block) {
                continue;
            }
            if let Some(n) = &heads[i] {
                ready.push(Reverse((order[n.block_index as usize], i)));
            }
            break;
        }
    }
    pops
}

struct EventWork {
    epoch: Epoch,
    tasks: Arc<Vec<Task>>,
    credits: EventCredits,
    error: Mutex<Option<&'static str>>,
}
impl EventWork {
    fn fail(&self, error: &'static str) {
        let mut slot = self.error.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            *slot = Some(error);
        }
        drop(slot);
        self.credits.cancel();
    }
}

fn evaluate(
    epoch: Epoch,
    blocks: &[Arc<BlockPlan>],
    output: &mut TaskOutput,
    vm: &mut Vec<Value>,
    words: &mut Vec<u64>,
) {
    output.writes.clear();
    output.fast = 0;
    output.fallback = 0;
    // SAFETY: the caller owns a live task credit throughout evaluation. No
    // mutable access to these input allocations resumes before credit zero.
    let signals = unsafe { std::slice::from_raw_parts(epoch.values, epoch.len) };
    let signed = unsafe { std::slice::from_raw_parts(epoch.signed, epoch.signed_len) };
    let names = unsafe { &*epoch.names };
    let arrays = unsafe { &*epoch.arrays };
    let packed = unsafe { &*epoch.packed };
    for block in blocks {
        let before = output.writes.len();
        if epoch.fast
            && block.word.as_ref().is_some_and(|program| {
                program.execute(
                    signals,
                    signed,
                    words,
                    &mut output.writes,
                    block.index,
                    block.duplicate_nba,
                )
            })
        {
            output.fast += 1;
            continue;
        }
        output.writes.truncate(before);
        output.fallback += 1;
        vm.resize(block.regs, Value::zero(1));
        let insns = unsafe { std::slice::from_raw_parts(block.ptr, block.len) };
        let mut result = Simulator::exec_insns_isolated(
            insns,
            signals,
            signed,
            names,
            arrays,
            Some(packed),
            vm,
            block.index,
            block.duplicate_nba,
        );
        output.writes.append(&mut result);
    }
}

fn consume(work: &EventWork, vm: &mut Vec<Value>, words: &mut Vec<u64>) {
    consume_lane::<false>(work, vm, words, 0);
}

fn consume_lane<const DIAGNOSTICS: bool>(
    work: &EventWork,
    vm: &mut Vec<Value>,
    words: &mut Vec<u64>,
    lane: usize,
) {
    while let Some(ticket) = work.credits.claim() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let task = &work.tasks[ticket.index()];
            let mut output = task.output.lock().unwrap_or_else(|e| e.into_inner());
            let started = if DIAGNOSTICS {
                output.executing_lane = lane;
                output.busy_ns = 0;
                Some(std::time::Instant::now())
            } else {
                None
            };
            evaluate(work.epoch, &task.blocks, &mut output, vm, words);
            if let Some(started) = started {
                output.busy_ns = started.elapsed().as_nanos();
            }
        }));
        if result.is_err() {
            // Publish failure and cancel queued work BEFORE retiring this
            // ticket. Other claimed tasks retain their input-lifetime credits.
            work.fail("parallel worker failed before publication");
        }
        // The output guard and all evaluator borrows ended above. Remaining
        // code only claims another credit or touches owned control metadata.
        drop(ticket);
    }
}

#[derive(Default)]
struct MailboxState {
    work: Option<Arc<EventWork>>,
    closed: bool,
}
#[derive(Default)]
struct Mailbox {
    state: Mutex<MailboxState>,
    ready: Condvar,
}
impl Mailbox {
    fn notify(&self, work: Arc<EventWork>) -> Result<(), &'static str> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return Err("parallel worker stopped");
        }
        // At most one notification per worker is retained. A previous event
        // has no credits left before the coordinator can replace its notice.
        state.work = Some(work);
        self.ready.notify_one();
        Ok(())
    }
    fn receive(&self) -> Option<Arc<EventWork>> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        while state.work.is_none() && !state.closed {
            state = self.ready.wait(state).unwrap_or_else(|e| e.into_inner());
        }
        state.work.take()
    }
    fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.closed = true;
        if let Some(work) = state.work.take() {
            work.fail("parallel worker stopped");
        }
        self.ready.notify_one();
    }
}
struct WorkerExit(Arc<Mailbox>);
impl Drop for WorkerExit {
    fn drop(&mut self) {
        self.0.close();
    }
}
struct Worker {
    mailbox: Arc<Mailbox>,
    handle: Option<JoinHandle<()>>,
}

fn multicore_diagnostics_enabled() -> bool {
    env_is_one_cached!("XEZIM_MULTICORE_DIAGNOSTICS")
}

#[derive(Default)]
struct DispatchDiagnostics {
    epochs: u64,
    replans: u64,
    blocks: u64,
    nba_entries: u64,
    prepare_ns: u128,
    execute_ns: u128,
    merge_ns: u128,
    plan_lowerings: u64,
    ts_lower_failed: u64,
    word_lower_failed: u64,
    merge_heap_pops: u64,
}

#[derive(Default)]
struct CreditStats {
    epochs: u64,
    issued: u64,
    retired: u64,
    cancelled: u64,
    wakes: u64,
    wait_ns: u128,
}

pub(super) struct EdgeWorkers {
    workers: Vec<Worker>,
    plans: Vec<Option<Arc<BlockPlan>>>,
    last_blocks: Vec<usize>,
    last_partitions: Vec<u32>,
    tasks: Arc<Vec<Task>>,
    merge_runs: bool,
    tasks_per_lane: usize,
    order: Vec<usize>,
    serial_vm: Vec<Value>,
    serial_words: Vec<u64>,
    stats: CreditStats,
    diagnostics: DispatchDiagnostics,
    lane_participation: Vec<LaneParticipation>,
}
impl EdgeWorkers {
    pub(super) fn worker_count(&self) -> usize {
        self.workers.len() + 1
    }
    pub(super) fn report_credits(&self) {
        let s = &self.stats;
        chatter!(
            "[PROF] multicore credits_issued={} credits_retired={} credits_cancelled={} credit_pending={} credit_epochs={} worker_wakes={} credit_wait_ms={:.1} tasks_per_lane={}",
            s.issued,
            s.retired,
            s.cancelled,
            s.issued - s.retired - s.cancelled,
            s.epochs,
            s.wakes,
            s.wait_ns as f64 / 1_000_000.0,
            self.tasks_per_lane
        );
        if multicore_diagnostics_enabled() {
            let d = &self.diagnostics;
            chatter!(
                "[PROF] multicore_dispatch epochs={} replans={} blocks={} nba_entries={} prepare_ms={:.3} execute_ms={:.3} merge_ms={:.3} plan_lowerings={} ts_lower_failed={} word_lower_failed={} merge_heap_pops={}",
                d.epochs,
                d.replans,
                d.blocks,
                d.nba_entries,
                d.prepare_ns as f64 / 1_000_000.0,
                d.execute_ns as f64 / 1_000_000.0,
                d.merge_ns as f64 / 1_000_000.0,
                d.plan_lowerings,
                d.ts_lower_failed,
                d.word_lower_failed,
                d.merge_heap_pops
            );
            let tasks: u64 = self.lane_participation.iter().map(|lane| lane.tasks).sum();
            let blocks: u64 = self.lane_participation.iter().map(|lane| lane.blocks).sum();
            let background_tasks: u64 = self
                .lane_participation
                .iter()
                .skip(1)
                .map(|lane| lane.tasks)
                .sum();
            let background_blocks: u64 = self
                .lane_participation
                .iter()
                .skip(1)
                .map(|lane| lane.blocks)
                .sum();
            chatter!(
                "[PROF] multicore_participation tasks={} background_tasks={} background_task_fraction={:.6} blocks={} background_blocks={} background_block_fraction={:.6}",
                tasks,
                background_tasks,
                background_tasks as f64 / tasks.max(1) as f64,
                blocks,
                background_blocks,
                background_blocks as f64 / blocks.max(1) as f64
            );
            for (lane, participation) in self.lane_participation.iter().enumerate() {
                chatter!(
                    "[PROF] multicore_lane lane={} tasks={} blocks={} nba_entries={} busy_ms={:.3}",
                    lane,
                    participation.tasks,
                    participation.blocks,
                    participation.nba_entries,
                    participation.busy_ns as f64 / 1_000_000.0
                );
            }
        }
    }
    fn new(count: usize) -> Self {
        let diagnostics = multicore_diagnostics_enabled();
        let mut workers = Vec::with_capacity(count);
        // The coordinator is an execution lane too; configured CPU capacity
        // includes it instead of adding an idle coordinator to N workers.
        for worker in 0..count.saturating_sub(1) {
            let mailbox = Arc::new(Mailbox::default());
            let receiver = mailbox.clone();
            let handle = std::thread::Builder::new()
                .name(format!("simulation-{worker}"))
                .spawn(move || {
                    let _exit = WorkerExit(receiver.clone());
                    let mut vm = Vec::new();
                    let mut words = Vec::new();
                    while let Some(work) = receiver.receive() {
                        if diagnostics {
                            consume_lane::<true>(&work, &mut vm, &mut words, worker + 1);
                        } else {
                            consume(&work, &mut vm, &mut words);
                        }
                    }
                })
                .expect("start simulation worker");
            workers.push(Worker {
                mailbox,
                handle: Some(handle),
            });
        }
        Self {
            workers,
            plans: Vec::new(),
            last_blocks: Vec::new(),
            last_partitions: Vec::new(),
            tasks: Arc::new(Vec::new()),
            merge_runs: env_is_one_cached!("XEZIM_MULTICORE_MERGE_RUNS"),
            tasks_per_lane: std::env::var("XEZIM_MULTICORE_TASKS_PER_LANE")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|value| (1..=16).contains(value))
                .unwrap_or(4),
            order: Vec::new(),
            serial_vm: Vec::new(),
            serial_words: Vec::new(),
            stats: CreditStats::default(),
            diagnostics: DispatchDiagnostics::default(),
            lane_participation: if diagnostics {
                vec![LaneParticipation::default(); count]
            } else {
                Vec::new()
            },
        }
    }
}
impl Drop for EdgeWorkers {
    fn drop(&mut self) {
        for worker in &self.workers {
            worker.mailbox.close();
        }
        for worker in &mut self.workers {
            if let Some(handle) = worker.handle.take() {
                let _ = handle.join();
            }
        }
    }
}

impl Simulator {
    pub(super) fn prepare_multicore(&mut self) {
        // Positive capability check applies to the legacy isolated path too:
        // an added opcode must not silently become parallel eligible.
        for (i, allowed) in self.edge_block_parallel.iter_mut().enumerate() {
            if *allowed {
                *allowed = self.compiled_edge_blocks[i].as_ref().is_some_and(|b| {
                    b.instructions
                        .iter()
                        .all(Self::isolated_instruction_supported)
                });
            }
        }
        self.multicore_threads = self.multicore_threads.min(
            self.edge_block_parallel
                .iter()
                .filter(|&&eligible| eligible)
                .count()
                .max(1),
        );
        self.edge_workers = None;
    }

    pub(super) fn dispatch_multicore(&mut self, blocks: &mut [usize]) -> Result<(), String> {
        if blocks.is_empty() {
            return Ok(());
        }
        let diagnostics = multicore_diagnostics_enabled();
        let prepare_start = diagnostics.then(std::time::Instant::now);
        let partitioned =
            self.edge_block_partition_count > 0 && !self.edge_block_partition.is_empty();
        if partitioned {
            blocks.sort_unstable();
            self.prof_par_dispatch_partition += 1;
        } else {
            self.prof_par_dispatch_legacy += 1;
        }
        // Keep the pool stable as clock populations change. Only queued task
        // credits determine participation in a small event.
        let count = self.multicore_threads.max(1);
        let mut pool = self
            .edge_workers
            .take()
            .filter(|p| p.worker_count() == count)
            .unwrap_or_else(|| EdgeWorkers::new(count));
        if pool.plans.len() != self.compiled_edge_blocks.len() {
            pool.plans
                .resize_with(self.compiled_edge_blocks.len(), || None);
        }
        let membership_changed = if partitioned {
            pool.last_partitions.len() != blocks.len()
                || blocks
                    .iter()
                    .zip(&pool.last_partitions)
                    .any(|(&i, &p)| self.edge_block_partition[i] != p)
        } else {
            !pool.last_partitions.is_empty()
        };
        let mut changed = pool.last_blocks != blocks || membership_changed;
        for &bi in blocks.iter() {
            let block = self.compiled_edge_blocks[bi]
                .as_ref()
                .ok_or("missing parallel block")?;
            if !pool.plans[bi].as_ref().is_some_and(|p| {
                p.ptr == block.instructions.as_ptr() && p.len == block.instructions.len()
            }) {
                if !block
                    .instructions
                    .iter()
                    .all(Self::isolated_instruction_supported)
                {
                    return Err("unsupported parallel instruction".into());
                }
                let ts = super::super::bytecode::lower_two_state(
                    block,
                    &self.signal_widths,
                    &self.signal_signed,
                    &self.signal_real,
                    &self.array_first_id,
                );
                if diagnostics {
                    pool.diagnostics.plan_lowerings += 1;
                    pool.diagnostics.ts_lower_failed += u64::from(ts.is_none());
                }
                let word = ts.and_then(|ts| {
                    let word = WordProgram::lower(ts);
                    if diagnostics {
                        pool.diagnostics.word_lower_failed += u64::from(word.is_none());
                    }
                    word
                });
                pool.plans[bi] = Some(Arc::new(BlockPlan {
                    index: bi as u32,
                    ptr: block.instructions.as_ptr(),
                    len: block.instructions.len(),
                    regs: block.num_regs as usize,
                    duplicate_nba: block.nba_dup_targets,
                    word,
                }));
                changed = true;
            }
        }
        if changed {
            if diagnostics {
                pool.diagnostics.replans += 1;
            }
            pool.order
                .resize(self.compiled_edge_blocks.len(), usize::MAX);
            for (rank, &bi) in blocks.iter().enumerate() {
                pool.order[bi] = rank;
            }
            let groups = partition_tasks_with_grain(
                blocks
                    .iter()
                    .map(|&bi| {
                        let locality = if partitioned {
                            self.edge_block_partition[bi]
                        } else {
                            self.edge_block_scope_id.get(bi).copied().unwrap_or(0)
                        };
                        (bi, locality, pool.plans[bi].as_ref().unwrap().len)
                    })
                    .collect(),
                count,
                pool.tasks_per_lane,
            );
            pool.tasks = Arc::new(
                groups
                    .into_iter()
                    .map(|mut group| {
                        group.sort_by_key(|&bi| pool.order[bi]);
                        Task {
                            blocks: group
                                .into_iter()
                                .map(|bi| pool.plans[bi].as_ref().unwrap().clone())
                                .collect(),
                            output: Mutex::new(TaskOutput::default()),
                        }
                    })
                    .collect(),
            );
            pool.last_blocks = blocks.to_vec();
            pool.last_partitions = if partitioned {
                blocks
                    .iter()
                    .map(|&i| self.edge_block_partition[i])
                    .collect()
            } else {
                Vec::new()
            };
        }
        let work = Arc::new(EventWork {
            epoch: Epoch {
                values: self.signal_table.as_ptr(),
                len: self.signal_table.len(),
                signed: self.signal_signed.as_ptr(),
                signed_len: self.signal_signed.len(),
                names: &self.signal_name_to_id,
                arrays: &self.array_first_id,
                packed: &self.packed,
                fast: !self.warn_x
                    && !matches!(std::env::var("XEZIM_TWO_STATE").as_deref(), Ok("0")),
            },
            credits: EventCredits::new(pool.tasks.len()),
            tasks: pool.tasks.clone(),
            error: Mutex::new(None),
        });
        if let Some(start) = prepare_start {
            pool.diagnostics.prepare_ns += start.elapsed().as_nanos();
            pool.diagnostics.epochs += 1;
            pool.diagnostics.blocks += blocks.len() as u64;
        }
        let execute_start = diagnostics.then(std::time::Instant::now);
        pool.stats.epochs += 1;
        pool.stats.issued += work.tasks.len() as u64;
        if !parallel_serialize_enabled() {
            for worker in pool.workers.iter().take(work.tasks.len()) {
                if let Err(error) = worker.mailbox.notify(work.clone()) {
                    work.fail(error);
                    break;
                }
                pool.stats.wakes += 1;
            }
        }
        work.credits.seal();
        if diagnostics {
            consume_lane::<true>(&work, &mut pool.serial_vm, &mut pool.serial_words, 0);
        } else {
            consume(&work, &mut pool.serial_vm, &mut pool.serial_words);
        }
        let wait = (self.profile_report || diagnostics).then(std::time::Instant::now);
        // Event quiescence depends on actual queued/in-flight work credits,
        // not on acknowledgements or arrivals from every configured worker.
        work.credits.wait();
        if let Some(start) = wait {
            pool.stats.wait_ns += start.elapsed().as_nanos();
        }
        pool.stats.retired += work.credits.completed() as u64;
        pool.stats.cancelled += work.credits.cancelled() as u64;
        debug_assert_eq!(
            work.tasks.len(),
            work.credits.completed() + work.credits.cancelled()
        );
        if let Some(start) = execute_start {
            pool.diagnostics.execute_ns += start.elapsed().as_nanos();
        }
        let error = *work.error.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(error) = error {
            return Err(error.into());
        }
        let merge_start = diagnostics.then(std::time::Instant::now);
        let nba_start = self.nba_fast.len();
        // Credit zero proves every task output and input borrow is complete.
        // Draining retains task-local vector capacity for the next event.
        let mut outputs: Vec<_> = work
            .tasks
            .iter()
            .map(|task| task.output.lock().unwrap_or_else(|e| e.into_inner()))
            .collect();
        if diagnostics {
            record_lane_outputs(&outputs, &mut pool.lane_participation);
        }
        for output in &outputs {
            self.prof_multicore_fast += output.fast;
            self.prof_multicore_fallback += output.fallback;
        }
        let heap_pops = match (pool.merge_runs, diagnostics) {
            (false, false) => merge_task_outputs::<false, false>(
                &mut outputs,
                &pool.order,
                &mut self.nba_fast,
                &mut self.nba_fast_index,
            ),
            (false, true) => merge_task_outputs::<false, true>(
                &mut outputs,
                &pool.order,
                &mut self.nba_fast,
                &mut self.nba_fast_index,
            ),
            (true, false) => merge_task_outputs::<true, false>(
                &mut outputs,
                &pool.order,
                &mut self.nba_fast,
                &mut self.nba_fast_index,
            ),
            (true, true) => merge_task_outputs::<true, true>(
                &mut outputs,
                &pool.order,
                &mut self.nba_fast,
                &mut self.nba_fast_index,
            ),
        };
        drop(outputs);
        if let Some(start) = merge_start {
            pool.diagnostics.merge_ns += start.elapsed().as_nanos();
            pool.diagnostics.merge_heap_pops += heap_pops;
            pool.diagnostics.nba_entries += (self.nba_fast.len() - nba_start) as u64;
        }
        self.edge_workers = Some(pool);
        Ok(())
    }
}

impl Simulator {
    pub(super) fn isolated_instruction_supported(insn: &super::super::bytecode::Insn) -> bool {
        use super::super::bytecode::Insn;
        matches!(
            insn,
            Insn::LoadConst { .. }
                | Insn::LoadSignal { .. }
                | Insn::LoadSignalSigned { .. }
                | Insn::Resize { .. }
                | Insn::Add { .. }
                | Insn::Sub { .. }
                | Insn::Mul { .. }
                | Insn::Div { .. }
                | Insn::Mod { .. }
                | Insn::BitAnd { .. }
                | Insn::BitOr { .. }
                | Insn::BitXor { .. }
                | Insn::BitXnor { .. }
                | Insn::LogAnd { .. }
                | Insn::LogOr { .. }
                | Insn::Eq { .. }
                | Insn::Neq { .. }
                | Insn::CaseEq { .. }
                | Insn::CaseLut { .. }
                | Insn::BinOpConstAdd2 { .. }
                | Insn::BinOpConst { .. }
                | Insn::CasezEq { .. }
                | Insn::CasexEq { .. }
                | Insn::Lt { .. }
                | Insn::Leq { .. }
                | Insn::Gt { .. }
                | Insn::Geq { .. }
                | Insn::Shl { .. }
                | Insn::Shr { .. }
                | Insn::AShr { .. }
                | Insn::BitNot { .. }
                | Insn::LogNot { .. }
                | Insn::Negate { .. }
                | Insn::ReduceAnd { .. }
                | Insn::ReduceOr { .. }
                | Insn::ReduceXor { .. }
                | Insn::BitSelect { .. }
                | Insn::BitSelectConst { .. }
                | Insn::RangeSelectW { .. }
                | Insn::RangeSelect { .. }
                | Insn::RangeSelectConst { .. }
                | Insn::LoadSignalRangeDyn { .. }
                | Insn::LoadSignalRange { .. }
                | Insn::LoadSignalBit { .. }
                | Insn::Concat { .. }
                | Insn::Replicate { .. }
                | Insn::BranchIfFalse { .. }
                | Insn::NbaAssignConst { .. }
                | Insn::BranchUnlessZero { .. }
                | Insn::BranchIfSignalFalse { .. }
                | Insn::Select { .. }
                | Insn::Jump { .. }
                | Insn::CaseJump { .. }
                | Insn::CaseMaskJump { .. }
                | Insn::Format { .. }
                | Insn::StrOp { .. }
                | Insn::NbaAssign { .. }
                | Insn::NbaAssignRange { .. }
                | Insn::NbaAssignBitDyn { .. }
                | Insn::LoadArrayElem { .. }
                | Insn::NbaAssignArrayRead { .. }
                | Insn::Move { .. }
                | Insn::MoveResize { .. }
                | Insn::CmpBranch { .. }
                | Insn::SetSigned { .. }
                | Insn::ClearSigned { .. }
                | Insn::DropXz { .. }
                | Insn::Pow { .. }
                | Insn::Nop { .. }
        )
    }
}

#[cfg(test)]
#[path = "parallel_workers_tests.rs"]
mod tests;
