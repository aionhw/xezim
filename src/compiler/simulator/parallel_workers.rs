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
}
struct Task {
    blocks: Vec<Arc<BlockPlan>>,
    // A claimed task has one writer. The slot retains capacity across events;
    // the coordinator takes ownership of results only after credit quiescence.
    output: Mutex<TaskOutput>,
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
    while let Some(ticket) = work.credits.claim() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let task = &work.tasks[ticket.index()];
            let mut output = task.output.lock().unwrap_or_else(|e| e.into_inner());
            evaluate(work.epoch, &task.blocks, &mut output, vm, words);
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
    order: Vec<usize>,
    serial_vm: Vec<Value>,
    serial_words: Vec<u64>,
    stats: CreditStats,
}
impl EdgeWorkers {
    pub(super) fn worker_count(&self) -> usize {
        self.workers.len()
    }
    pub(super) fn report_credits(&self) {
        let s = &self.stats;
        chatter!(
            "[PROF] multicore credits_issued={} credits_retired={} credits_cancelled={} credit_pending={} credit_epochs={} worker_wakes={} credit_wait_ms={:.1}",
            s.issued,
            s.retired,
            s.cancelled,
            s.issued - s.retired - s.cancelled,
            s.epochs,
            s.wakes,
            s.wait_ns as f64 / 1_000_000.0
        );
    }
    fn new(count: usize) -> Self {
        let mut workers = Vec::with_capacity(count);
        for worker in 0..count {
            let mailbox = Arc::new(Mailbox::default());
            let receiver = mailbox.clone();
            let handle = std::thread::Builder::new()
                .name(format!("simulation-{worker}"))
                .spawn(move || {
                    let _exit = WorkerExit(receiver.clone());
                    let mut vm = Vec::new();
                    let mut words = Vec::new();
                    while let Some(work) = receiver.receive() {
                        consume(&work, &mut vm, &mut words);
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
            order: Vec::new(),
            serial_vm: Vec::new(),
            serial_words: Vec::new(),
            stats: CreditStats::default(),
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
        let partitioned =
            self.edge_block_partition_count > 0 && !self.edge_block_partition.is_empty();
        if partitioned {
            blocks.sort_unstable();
            self.prof_par_dispatch_partition += 1;
        } else {
            self.prof_par_dispatch_legacy += 1;
        }
        let count = self
            .multicore_threads
            .min(if partitioned {
                self.edge_block_partition_count as usize
            } else {
                self.multicore_threads
            })
            .max(1);
        let mut pool = self
            .edge_workers
            .take()
            .filter(|p| p.workers.len() == count)
            .unwrap_or_else(|| EdgeWorkers::new(count));
        if pool.plans.len() < self.compiled_edge_blocks.len() {
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
                let word = super::super::bytecode::lower_two_state(
                    block,
                    &self.signal_widths,
                    &self.signal_signed,
                    &self.signal_real,
                    &self.array_first_id,
                )
                .and_then(WordProgram::lower);
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
            pool.order
                .resize(self.compiled_edge_blocks.len(), usize::MAX);
            for (rank, &bi) in blocks.iter().enumerate() {
                pool.order[bi] = rank;
            }
            // A logical partition is one indivisible task. Otherwise create
            // several balanced tasks per worker, allowing any free worker to
            // claim the next task instead of waiting for a fixed peer batch.
            let mut groups: Vec<Vec<usize>> = if partitioned {
                let mut units: Vec<Vec<usize>> = (0..self.edge_block_partition_count)
                    .map(|_| Vec::new())
                    .collect();
                for &bi in blocks.iter() {
                    units[self.edge_block_partition[bi] as usize].push(bi);
                }
                units.into_iter().filter(|u| !u.is_empty()).collect()
            } else {
                let tasks = blocks.len().min(count.saturating_mul(4)).max(1);
                let mut groups: Vec<Vec<usize>> = (0..tasks).map(|_| Vec::new()).collect();
                let mut costs = vec![0usize; tasks];
                let mut ranked = blocks.to_vec();
                ranked.sort_by_key(|&i| std::cmp::Reverse(pool.plans[i].as_ref().unwrap().len));
                for bi in ranked {
                    let task = (0..tasks).min_by_key(|&i| (costs[i], i)).unwrap();
                    costs[task] += pool.plans[bi].as_ref().unwrap().len;
                    groups[task].push(bi);
                }
                groups
            };
            groups.sort_by_key(|group| {
                std::cmp::Reverse(
                    group
                        .iter()
                        .map(|&i| pool.plans[i].as_ref().unwrap().len)
                        .sum::<usize>(),
                )
            });
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
        pool.stats.epochs += 1;
        pool.stats.issued += work.tasks.len() as u64;
        if parallel_serialize_enabled() {
            consume(&work, &mut pool.serial_vm, &mut pool.serial_words);
        } else {
            for worker in pool.workers.iter().take(work.tasks.len()) {
                if let Err(error) = worker.mailbox.notify(work.clone()) {
                    work.fail(error);
                    break;
                }
                pool.stats.wakes += 1;
            }
        }
        work.credits.seal();
        let wait = self.profile_report.then(std::time::Instant::now);
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
        let error = *work.error.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(error) = error {
            return Err(error.into());
        }
        // Credit zero proves every task output and input borrow is complete.
        // Draining retains task-local vector capacity for the next event.
        let mut outputs: Vec<_> = work
            .tasks
            .iter()
            .map(|task| task.output.lock().unwrap_or_else(|e| e.into_inner()))
            .collect();
        for output in &outputs {
            self.prof_multicore_fast += output.fast;
            self.prof_multicore_fallback += output.fallback;
        }
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        let mut drains: Vec<_> = outputs.iter_mut().map(|o| o.writes.drain(..)).collect();
        let mut heads: Vec<_> = drains.iter_mut().map(Iterator::next).collect();
        let mut ready = BinaryHeap::new();
        for (i, head) in heads.iter().enumerate() {
            if let Some(n) = head {
                ready.push(Reverse((pool.order[n.block_index as usize], i)));
            }
        }
        while let Some(Reverse((_, i))) = ready.pop() {
            let entry = heads[i].take().unwrap();
            self.nba_fast_index
                .insert(entry.signal_id, self.nba_fast.len());
            self.nba_fast.push(entry);
            heads[i] = drains[i].next();
            if let Some(n) = &heads[i] {
                ready.push(Reverse((pool.order[n.block_index as usize], i)));
            }
        }
        drop(drains);
        drop(outputs);
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
