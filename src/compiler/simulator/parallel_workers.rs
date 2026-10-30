//! Persistent workers with immutable input epochs and ordered NBA publication.
//! IEEE 1800-2023 §4.6 / §10.4.2: evaluation finishes before publishing results.
use super::parallel_word::WordProgram;
use super::*;
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;

struct BlockPlan {
    index: u32,
    ptr: *const super::super::bytecode::Insn,
    len: usize,
    regs: usize,
    duplicate_nba: bool,
    word: Option<WordProgram>,
}
// Only positively admitted immutable instruction streams enter a plan. The
// simulator owns their backing allocation and waits for every job before it
// can mutate/drop that allocation. No AST fallback executes on a worker.
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
// dispatch waits for ALL replies, including failures, before the simulator
// regains mutable access. Workers only form shared references to these fields.
unsafe impl Send for Epoch {}

struct Job {
    epoch: Epoch,
    blocks: Arc<Vec<Arc<BlockPlan>>>,
    writes: Vec<NbaFast>,
}
struct Reply {
    worker: usize,
    writes: Vec<NbaFast>,
    fast: u64,
    fallback: u64,
    error: Option<String>,
}
struct Worker {
    jobs: mpsc::Sender<Option<Job>>,
    handle: Option<JoinHandle<()>>,
}

fn evaluate(worker: usize, mut job: Job, vm: &mut Vec<Value>, words: &mut Vec<u64>) -> Reply {
    job.writes.clear();
    let mut fast = 0;
    let mut fallback = 0;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: the synchronous epoch remains read-only and
        // alive until every worker has returned its reply.
        let e = job.epoch;
        let signals = unsafe { std::slice::from_raw_parts(e.values, e.len) };
        let signed = unsafe { std::slice::from_raw_parts(e.signed, e.signed_len) };
        let names = unsafe { &*e.names };
        let arrays = unsafe { &*e.arrays };
        let packed = unsafe { &*e.packed };
        for block in job.blocks.iter() {
            let before = job.writes.len();
            if e.fast
                && block.word.as_ref().is_some_and(|program| {
                    program.execute(
                        signals,
                        signed,
                        words,
                        &mut job.writes,
                        block.index,
                        block.duplicate_nba,
                    )
                })
            {
                fast += 1;
                continue;
            }
            job.writes.truncate(before);
            fallback += 1;
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
            job.writes.append(&mut result);
        }
    }));

    let error = result
        .err()
        .map(|_| "parallel worker failed before publication".to_string());
    Reply {
        worker,
        writes: job.writes,
        fast,
        fallback,
        error,
    }
}

pub(super) struct EdgeWorkers {
    workers: Vec<Worker>,
    replies: mpsc::Receiver<Reply>,
    plans: Vec<Option<Arc<BlockPlan>>>,
    last_blocks: Vec<usize>,
    last_partitions: Vec<u32>,
    groups: Vec<Arc<Vec<Arc<BlockPlan>>>>,
    outputs: Vec<Vec<NbaFast>>,
    order: Vec<usize>,
    serial_vm: Vec<Value>,
    serial_words: Vec<u64>,
}
impl EdgeWorkers {
    pub(super) fn worker_count(&self) -> usize {
        self.workers.len()
    }

    fn new(count: usize) -> Self {
        let (reply_tx, replies) = mpsc::channel();
        let mut workers = Vec::with_capacity(count);
        for worker in 0..count {
            let (jobs, rx) = mpsc::channel::<Option<Job>>();
            let tx = reply_tx.clone();
            let handle = std::thread::Builder::new()
                .name(format!("simulation-{worker}"))
                .spawn(move || {
                    let mut vm = Vec::<Value>::new();
                    let mut words = Vec::<u64>::new();
                    while let Ok(Some(job)) = rx.recv() {
                        let reply = evaluate(worker, job, &mut vm, &mut words);
                        if tx.send(reply).is_err() {
                            break;
                        }
                    }
                })
                .expect("start simulation worker");
            workers.push(Worker {
                jobs,
                handle: Some(handle),
            });
        }
        Self {
            workers,
            replies,
            plans: Vec::new(),
            last_blocks: Vec::new(),
            last_partitions: Vec::new(),
            groups: Vec::new(),
            outputs: (0..count).map(|_| Vec::new()).collect(),
            order: Vec::new(),
            serial_vm: Vec::new(),
            serial_words: Vec::new(),
        }
    }
}
impl Drop for EdgeWorkers {
    fn drop(&mut self) {
        for w in &self.workers {
            let _ = w.jobs.send(None);
        }
        for w in &mut self.workers {
            if let Some(h) = w.handle.take() {
                let _ = h.join();
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
            let mut groups: Vec<Vec<usize>> = (0..count).map(|_| Vec::new()).collect();
            let mut costs = vec![0usize; count];
            // Coarse cost-balanced batches. A loaded logical partition stays
            // intact; the number of physical workers remains hardware bounded.
            let mut units: Vec<Vec<usize>> = if partitioned {
                let mut units: Vec<Vec<usize>> = (0..self.edge_block_partition_count)
                    .map(|_| Vec::new())
                    .collect();
                for &bi in blocks.iter() {
                    units[self.edge_block_partition[bi] as usize].push(bi);
                }
                units.into_iter().filter(|u| !u.is_empty()).collect()
            } else {
                blocks.iter().map(|&bi| vec![bi]).collect()
            };
            units.sort_by_key(|u| {
                std::cmp::Reverse(
                    u.iter()
                        .map(|&bi| pool.plans[bi].as_ref().unwrap().len)
                        .sum::<usize>(),
                )
            });
            for unit in units {
                let worker = (0..count).min_by_key(|&i| (costs[i], i)).unwrap();
                costs[worker] += unit
                    .iter()
                    .map(|&i| pool.plans[i].as_ref().unwrap().len)
                    .sum::<usize>();
                groups[worker].extend(unit);
            }
            pool.groups = groups
                .into_iter()
                .map(|mut group| {
                    group.sort_by_key(|&bi| pool.order[bi]);
                    Arc::new(
                        group
                            .into_iter()
                            .map(|bi| pool.plans[bi].as_ref().unwrap().clone())
                            .collect(),
                    )
                })
                .collect();
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
        let epoch = Epoch {
            values: self.signal_table.as_ptr(),
            len: self.signal_table.len(),
            signed: self.signal_signed.as_ptr(),
            signed_len: self.signal_signed.len(),
            names: &self.signal_name_to_id,
            arrays: &self.array_first_id,
            packed: &self.packed,
            fast: !self.warn_x && !matches!(std::env::var("XEZIM_TWO_STATE").as_deref(), Ok("0")),
        };
        let mut sent = 0;
        let mut error = None;
        let serialized = parallel_serialize_enabled();
        for i in 0..count {
            let job = Job {
                epoch,
                blocks: pool.groups[i].clone(),
                writes: std::mem::take(&mut pool.outputs[i]),
            };
            if serialized {
                let reply = evaluate(i, job, &mut pool.serial_vm, &mut pool.serial_words);
                self.prof_multicore_fast += reply.fast;
                self.prof_multicore_fallback += reply.fallback;
                if reply.error.is_some() {
                    error = reply.error;
                }
                pool.outputs[i] = reply.writes;
                continue;
            }
            if pool.workers[i].jobs.send(Some(job)).is_err() {
                error = Some("parallel worker stopped".to_string());
                break;
            }
            sent += 1;
        }
        // Always drain successful submissions before leaving this epoch,
        // including when another worker failed. Nothing has been published.
        for _ in 0..sent {
            match pool.replies.recv() {
                Ok(reply) => {
                    self.prof_multicore_fast += reply.fast;
                    self.prof_multicore_fallback += reply.fallback;
                    if reply.error.is_some() {
                        error = reply.error;
                    }
                    pool.outputs[reply.worker] = reply.writes;
                }
                Err(_) => {
                    error = Some("parallel worker reply lost".into());
                    break;
                }
            }
        }
        if let Some(error) = error {
            return Err(error);
        }
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        let mut drains: Vec<_> = pool.outputs.iter_mut().map(|v| v.drain(..)).collect();
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
