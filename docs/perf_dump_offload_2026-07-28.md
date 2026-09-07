# Dump/log offload, and where the remaining time is (2026-07-28)

Measured on `c906_all.fl --max-time 100000 -s tb` (XuanTie C906 `hello`, 1.56 M
signals, 52 K comb entries) unless stated. Method: two prebuilt binaries run
**alternately** and paired — this host drifts several percent between runs and
sequential A/B manufactures fake results.

---

## 1. The finding that started this: dumping cost 18-20x the simulation

| run | sim_loop |
|---|---|
| no dump | 2,736 ms |
| `--xtrace` | 53,994 ms — **19.7x** |
| `--fst` | 49,089 ms — **17.9x** |

Two independent causes, both on the simulation thread:

1. **Change detection was a FULL SCAN.** `xtrace_write_changes` and
   `fst_write_changes` compared every traced signal against its previous value
   on every time slot: 1.56 M `Value` compares x 6,650 slots ~ 10 G compares.
   VCD already had an incremental dirty-set path (`vcd_dirty_active`); XTrace and
   FST never got one.
2. **Rendering ran inline.** XTrace built a `String` per changed value and
   formatted the `D`/`P` records on the simulation thread; only the finished
   bytes were handed to a writer thread. FST did packing, block compression and
   I/O inline with nothing offloaded.

## 2. What changed

* **Shared dump dirty set.** The VCD dirty-set machinery is now `dump_dirty*`,
  armed by whichever writers are active and consumed by all of them in
  `dump_write_changes`, which retires it once after the last writer. XTrace and
  FST each got a signal-id -> trace-slot reverse map. Slots are visited in
  sorted order, so per-timestep record order is unchanged.
  Escape hatch: `XEZIM_DUMP_FULL=1` (plus the existing `XEZIM_VCD_FULL=1`).
* **XTrace rendering moved to the writer thread.** `post_xtrace_changes` posts
  `XtraceTimestep { time_delta, changes: Vec<(Arc<str>, Value, is_real,
  is_string)>, events }`; `write_xtrace_timestep` renders it on the worker.
  `xtrace_format_value` moved into `vcd_sink` so both paths share one formatter.
  Dictionary ids became `Arc<str>` — posting a change is now a refcount bump.
* **FST gained a writer thread** (`compiler::fst_sink::FstSink`). The
  `FstBodyWriter` lives on the worker; the simulation thread posts
  `FstTimestep`s. `finish()` is a rendezvous, so the trailer is on disk when it
  returns. `XEZIM_DUMP_INLINE=1` keeps it inline.
* **`$display`/log writing is threaded by default**, decoupled from the old
  `--threads` flag (since removed; it used to need `--threads >= 2`). Three
  fixes were needed to make that a win
  rather than a loss — see §4.

## 3. Result

Paired, interleaved, 3 reps each:

| load | base | new | delta |
|---|---|---|---|
| `--xtrace` | 55,242 / 56,606 / 56,309 ms | 3,412 / 3,398 / 3,322 ms | **-94% (16.5x)** |
| `--fst` | 50,545 / 50,392 ms | 3,585 / 3,452 ms | **-93% (14.3x)** |
| `$display` x400 K | 887 / 862 / 844 ms | 659 / 676 / 669 ms | **-22%** |
| no dump | 2,780 / 2,785 / 2,838 ms | 2,777 / 2,779 / 2,757 ms | none |

Byte-identical dumps (`cmp`) for both XTrace (120,237,882 B) and FST
(20,915,604 B); `$display` output identical to the byte on both stdout and
`--log`. Regression suite: **1,281 passed / 0 failed** across 276 binaries.

### Attribution — the dirty set is the win, threading is the follow-up

Same binary, env-toggled, `--xtrace`:

| configuration | sim_loop |
|---|---|
| baseline (full scan, inline rendering) | 53,994 ms |
| threading only (`XEZIM_DUMP_FULL=1`) | 51,586 ms — **-4.5%** |
| dirty set only (`XEZIM_DUMP_INLINE=1`) | 3,627 ms — **-93.3%** |
| both | 3,236 ms — **-94.0%** |

Worth stating plainly: offloading rendering to a thread was worth 4.5% while
the scan dominated, and 10.8% once it was gone. **Threading a phase does not
help while that phase is not the cost.** The algorithmic fix had to come first.

## 4. Three traps in the `$display` path

Naively flipping the stdout sink to threaded measured **12% SLOWER**
(868 -> 976 ms), and corrupted output. All three causes are worth remembering:

1. **A flush per line defeats batching.** `writeln_str` flushed after every
   line, so threaded mode paid a channel send + buffer swap per `$display` and
   still did one write syscall per line on the worker — pure added cost. The
   flush POLICY now lives on the worker: it drains everything queued, then
   flushes once. Bursts coalesce; when the simulation goes quiet the queue
   drains and the flush is immediate, so visibility is unchanged.
2. **The producer must not dispatch per line either.** Lines are released on a
   time bound (`LINE_FLUSH_MAX_DELAY` = 5 ms, clock read 1-in-64 lines) or when
   the 8 KB buffer fills. This is what turned the regression into -22%.
3. **A chunk boundary could fall inside a line.** `writeln_str` tested the
   dispatch threshold after the message and again after the `\n`, so a line that
   filled the buffer went to the worker *without its terminator* — and with
   `--log` (stdout and stderr dup2'd to one file) a diagnostic written meanwhile
   landed inside it: `...status=running[PROF] settle=...`. The message and its
   newline are now appended before any dispatch check.

Also added: `StdoutSink::sync()`, a real barrier, called at the end of `run()`.
The `[PROF]`/`[PHASE]` lines and "Simulation finished at time N" go straight to
stderr/stdout on the main thread and would otherwise overtake `$display` output
still queued on the worker. `Simulator::flush_stdout()` existed but was dead
code — nothing ever called it.

---

## 5. Clock tree — where it costs, and what is worth doing

`edge_detect` is **222.8 ms of a 2,778 ms sim loop (8.0%)** on c906, and it is
almost entirely a per-signal scan: for each entry of `edge_signal_ids` it reads
`signal_table[sid]`, `prev_val[sid]` and `prev_xz[sid]` — three scattered loads
— to decide whether an edge fired.

`XEZIM_EDGE_SCAN_STATS=1`:

```
positions scanned = 12,544,519   fired = 3,228,172 (25.7%)
=> 74.3% of detect work is on positions that did not move
```

That splits the opportunity cleanly in two.

### 5a. The 74.3% that never fires — a knob that already exists

`XEZIM_DIRTY_EDGE=1` restricts the scan to positions actually written. It is
**implemented and off by default**. Measured (paired, 2 reps):

| | edge_detect | sim_loop |
|---|---|---|
| off | 224.1 / 226.9 ms | 2,781.5 / 2,795.4 ms |
| on | 164.9 / 159.9 ms | 2,731.2 / 2,725.0 ms |
| | **-28%** | **-2.2%** |

`XEZIM_DIRTY_EDGE_SHADOW=1` (which aborts loudly on any edge the dirty path
would have missed) runs clean on c906. It is a free ~2%; what it needs before
becoming the default is that shadow check green across the regression suite and
a gate-level design, not new code.

**Update: 5a was tried and the default was NOT flipped.** `XEZIM_DIRTY_EDGE=1`
is *sound* — `XEZIM_DIRTY_EDGE_SHADOW=1` reports **zero missed edges across all
1,281 regression tests** and on c906, and the run is functionally identical
(same stdout, `TEST PASSED`, sim end time 332550). But the counters are not:

```
off: edges_fired=9,684,685  nba_elided=2,371,062  insns=46,815,472  entry_evals=6,470,186
on:  edges_fired=9,683,215  nba_elided=2,371,895  insns=46,815,472  entry_evals=6,470,186
```

`insns`/`entry_evals` match, so no real work changed — `edges_fired` counts
armed-prefiltered blocks too, and positions that are never scanned are never
counted. `nba_elided` differing by 833 is a genuine ordering difference though.
That fails the byte-identity bar this tree uses before defaulting a perf feature
on, so it stays opt-in. Someone who wants the ~2% should reconcile those two
counters first.

### 5b. The 25.7% that does fire — clock-tree dedup — **IMPLEMENTED**

A flattened netlist replicates the clock tree into per-module buffered copies.
They all fire on the same edge and each pays its own scattered reads to
recompute an identical answer. Resolving every edge signal through the pure-copy
relation (`DirectCopy` / `FastDirectCopy` / `FastDirectFanout` / `FusedBufFanout`
/ `FusedGate::Buf1`) to its driver, then computing the edge once per group,
collapses this. Prior art in the sibling perf tree (`repo/xezim` @ `2f73c7f`)
measured **15,600 edge signals -> 66 groups on c910** (2,419 -> 32 on c906),
`edge_detect -31%`, `sim_loop -5.4%`, byte-identical.

Two constraints, both derived from measurement there and not negotiable:

* Group only signals reached through **>= 1 copy edge**. An independently driven
  root stays a singleton, or `rst_b` gains a spurious t=0 edge (prev is
  baselined after the source initialises but before its copies settle).
* Never group **inverting** copies or **>64-bit** signals (opposite polarity;
  `prev_wide` rather than the inline `prev_raw` pair).

It needs no change to iteration order, fanout walk order or dispatch sequence,
so NBA race resolution is untouched.

**Built (`build_edge_sig_groups` + `EdgeGroupMemo`), default on, opt out with
`XEZIM_NO_CLKTREE_DEDUP=1`.** Group structure found:

| design | edge signals | grouped | groups | largest group |
|---|---|---|---|---|
| c906 | 756 | 579 (76.6%) | 10 | 465 |
| c910 | 11,196 | 7,262 (64.9%) | 25 | 3,055 |

Measured (paired, interleaved, same binary env-toggled):

| | edge_detect | sim_loop |
|---|---|---|
| c910 off | 3,891.2 / 3,903.6 ms | 13,977.5 / 14,273.9 ms |
| c910 on | 3,035.7 / 3,041.6 ms | 13,340.0 / 13,478.2 ms |
| | **-22.0%** (2/2) | **-4.9%** (2/2) |
| c906 off | 226.7 / 230.6 / 235.1 ms | 2,752 / 2,756 / 2,733 ms |
| c906 on | 208.6 / 206.8 / 209.2 ms | 2,777 / 2,737 / 2,800 ms |
| | **-9.4%** (3/3) | neutral |

c906 is the honest counter-example: the win tracks how badly the clock tree is
replicated, and at 756 edge signals `edge_detect` is only 8% of the loop, so
-9.4% of it disappears into noise. Do not quote a single number for this
optimization; quote the group count.

**Correctness.** Two gates, both green:

* Counters byte-identical with dedup on vs off, on BOTH designs — c910
  `edges_fired=40,208,579 insns=68,001,417 entry_evals=24,857,388
  nba_elided=8,665,865`; c906 `edges_fired=9,684,685 insns=46,815,472
  entry_evals=6,470,186 nba_elided=2,371,062`. Suite 1,281 passed / 0 failed.
* `XEZIM_CLKTREE_PROBE=1` recomputes every deduped signal instead of trusting
  the memo and compares all seven quantities (`cur_v/cur_x/prev_v/prev_x` and
  the three fire flags) against the group leader: **0 mismatches in 72,109,468
  memo-eligible visits on c910**, 0 in 9,441,803 on c906, and 0 across the whole
  regression suite. This is what licensed sharing the VALUES and not just the
  fire decision — sharing values is what removes the scattered reads, and it is
  cheaper to measure than to argue.

One runtime guard the static analysis cannot cover: §9.3.1 lets `force` hold a
copy away from its driver, which is exactly what the grouping assumes cannot
happen. The memo is therefore disabled whenever `forced_signals` is non-empty
(the common case is empty, so this is one predictable branch).

### 5c. What is NOT a clock-tree lever

* **Clock gating.** `ARMED` already skips **5,910,288 of 6,091,135** gateable
  flop fires (97.1%) on c906. There is nothing left there.
* **Deeper buffer fusion.** `FusedBufFanout` / `FusedAndFanout` batch ONE level
  of fanout (>= 4 members sharing a source). A transitive collapse of buffer
  CHAINS is the obvious next step and is not implemented — but the sibling tree's
  `XEZIM_NET_COLLAPSE` (an aliasing attempt from 2026-05-29) is still opt-in and
  never became default, and that tree's own measurements put comb cone depth 8 at
  only +13% per eval. Treat it as unproven and M0 it (measure the actual chain
  depth distribution first) rather than building it on the strength of the idea.

---

## 6. Event FIFO — the queue is fine, the payload is not

`TimingWheel`: 256 slots, occupancy bitmap (`trailing_zeros` scan), `BTreeMap`
overflow for far-future events, `VecDeque<(pid, Vec<Statement>)>` per slot.
Schedule is O(1), `next_time` is O(1) per bitmap word. **The data structure is
not the problem.**

The cost is what travels through it. Every suspension does
`stmts[i + 1..].to_vec()` — a **deep AST clone** of the statements after the
suspension point — and the clone rides the queue to the resume.

Probe (`cont_pre_N` vs `cont_post_N`): identical executed work, padding
statements placed *before* the `@(posedge clk)` (empty continuation) versus
*after* it (N-statement continuation cloned every cycle), 20,000 resumes:

| trailing statements | continuation empty | continuation cloned | delta |
|---|---|---|---|
| 2 | 183.8 ms | 199.1 ms | **+8.3%** |
| 100 | 3,608.5 ms | 4,262.9 ms | **+18.1%** |

Both rows give the same per-statement cost: **~0.35 us per trailing statement,
per resume** (0.77 us / 2 statements; 32.7 us / 100 statements). That
independently reproduces the sibling tree's "~2.9 us per procedural resume vs
41 ns for a compiled edge block" — a typical UVM continuation is ~8 statements.

Ranked follow-ups:

1. ~~**Resume by cursor, not by copy.**~~ **BUILT, CORRECT, SLOWER, REVERTED
   (2026-07-29).** See §6b — the recommendation as written here was wrong in its
   premise, and the implementation that corrected the premise still lost.
2. ~~`pid_counts` -> dense `Vec`~~ — **TRIED, NEUTRAL, REVERTED.** The map is
   keyed by consecutive small integers and touched on every `schedule` /
   `pop_front` / `is_pid_suspended`, so the SipHash looked like free money. It
   is not: paired x3 on a 3-process load (3,788 / 3,962 / 3,853 -> 3,958 / 3,971
   / 3,817 ms) and on a purpose-built **400-live-process** load (4,462 / 4,849 /
   4,605 -> 4,630 / 4,773 / 4,670 ms), both noise. And pids are never reused, so
   the dense form grows to the high-water pid — memory traded for nothing.
   Reverted, with the reason recorded on the field so it is not re-litigated.
   This is the seventh "obviously hot" hypothesis in this tree to measure
   neutral; the pattern is now strong enough to treat as a prior.
3. Recycle the per-slot `VecDeque` payload allocations once (1) removes the
   deep clone; until then the clone dominates and this is noise.

Note the scope: on DUT-heavy RTL the event queue is nearly free (`sched` ~0.1 ms
on c906). All of this matters on **testbench/UVM-heavy** loads, where processes
suspend and resume constantly.

## 6b. Resume-by-cursor — built, correct, SLOWER, reverted (2026-07-29)

**The §6.1 recommendation above was not implementable as written, and the thing
that replaced it still lost. Both halves are worth recording.**

**Why "store `(pid, block, resume_index)`" is impossible here.**
`run_process_stmts` does not execute an AST node; it executes a list it
SYNTHESIZES. A blocking `begin/end` is flattened into the caller's stream and a
blocking task call is spliced in front of the caller's tail — 25 such sites in
one ~1,670-line function. So the statements a parked process still owes are
routinely a concatenation that exists nowhere in the source, and no index into
the AST can name it. The workable form is a cursor plus a FRAME CHAIN: a shared
`Arc<[Statement]>` + offset + `next` link, so a splice pushes a frame instead of
copying the caller's tail onto the end of the body.

**That was implemented in full** (`ProcCont`, 25 splice sites converted to
pushed frames, 9 suspension captures converted to `resume_at`, the 8
continuation-carrying structs and the whole `TimingWheel` retyped). It is
correct: **1,302 tests pass** and c906 is counter-identical (`edges_fired
9,684,685 / insns 46,816,268 / entry_evals 6,470,193 / nba_elided 2,371,064`,
`TEST PASSED`, end time 332550).

**It is also slower** (paired, interleaved):

| load | before | after | |
|---|---|---|---|
| c906 RTL | 2,535 / 2,676 / 2,667 ms | 2,652 / 2,590 / 2,626 ms | neutral |
| 400 live processes | 5,087 / 4,987 ms | 5,398 / 5,271 ms | **+5%** |
| `cont_pre_100` (empty continuation) | 3,374 ms | 3,477 ms | **+3%** |
| `cont_post_100` (100-stmt continuation) | 3,871 ms | 4,205 ms | **+9%** |

**Why, and it is the whole lesson: SPLICES ARE FAR MORE FREQUENT THAN
SUSPENSIONS.** A blocking `begin/end` inside a `forever` re-splices on every
iteration; a process suspends once per iteration at most. The old code paid a
deep clone per SUSPENSION. The new code removes that but adds `Arc::from(vec)`
— an allocation plus an O(N) move — per SPLICE, on top of the `inner.clone()`
that both versions pay. `cont_pre_100` isolates it exactly: its continuation is
empty, so `resume_at` saves nothing and the +3% is pure added Arc
materialization.

The 0.35 us/statement measured in §6 is real, but it is the cost of the splice
clone, which the probe could not separate from the suspension clone — both scale
with the same N.

**What would actually be needed:** the AST itself holding `Arc<[Statement]>`, so
a splice shares the block body instead of cloning it. That is an `xezim-core`
change through the parser and elaborator, and it is the real prerequisite —
without it, any frame-chain scheme pays to materialize what it wants to share. A
pointer-keyed `Arc` cache for AST blocks was considered and rejected: the frame a
`SeqBlock` is read from can itself be a temporary, so the address key has an ABA
hazard that would silently execute a stale body.

### CORRECTION (same day): on REAL UVM the frame chain is a ~3.6% WIN

The verdict above was reached on synthetics (`cont_*`, `many_procs`) because no
UVM benchmark existed. One now does — `bench/run_uvm_bench.sh`, added precisely
because this section had to guess. Re-judged on it (paired, interleaved, 3 reps,
median; `verdict=ok` means end time and fatal count matched):

| benchmark | before | after | delta |
|---|---|---|---|
| `phases/basic` | 6,421.8 ms | 6,105.3 ms | **-4.9%** |
| `tlm1/hierarchy` | 10,371.2 ms | 9,908.5 ms | **-4.5%** |
| `tlm1/producer_consumer` | 8,250.8 ms | 7,925.2 ms | **-3.9%** |
| `objections` | 3,384.0 ms | 3,263.4 ms | **-3.6%** |
| `tlm1/fifo` | 8,058.6 ms | 7,887.7 ms | **-2.1%** |
| `interfaces` | 15,538.4 ms | 15,708.2 ms | +1.1% |
| `hello_world` | 22,744.6 ms | 22,886.3 ms | +0.6% |

Five of seven faster, the two regressions inside noise. **The synthetics were
unrepresentative and led to the wrong call.** `cont_post_100` is a tight
`forever` that re-splices a 100-statement block every iteration — the single
shape where paying `Arc::from` per splice to save one suspension clone is a bad
trade. Real UVM code splices smaller bodies through deeper task-call chains,
where chaining the caller's tail instead of copying it wins.

The +5% on `many_procs` and +9% on `cont_post_100` are still real; this is a
trade, not a free win. DUT loads (c906, c910) were neutral either way, so
nothing regresses there.

**Status: the code was reverted before this measurement existed, and the working
tree no longer has it.** Re-landing means re-deriving it from §6b (the design is
described precisely enough: `ProcCont`, 25 splice sites to
`pushed`, 9 suspension captures to `resume_at`, 8 continuation structs and
`TimingWheel` retyped) and re-running both gates — 1,302 tests plus c906/c910
counter identity, which it passed. Worth doing; it is also still true that the
AST `Arc<[Statement]>` change (see `ast_shared_stmt_lists_scope.md`) removes the
`Arc::from` this trade is paying for, and would turn the two remaining
regressions into wins as well.

**Method lesson, and it is the same one as Round 3 of this document:** the
benchmark you lack is the conclusion you get wrong. This section confidently
reverted a correct 3.6% improvement because the only available workloads
exaggerated one code shape.

---

## 7. Where this landed

Shipped, default on, each with an opt-out and a byte-identity gate:

* **Dump/log offload** (§2) — `--xtrace` -94%, `--fst` -93%, `$display` -22%,
  no-dump unchanged.
* **Clock-tree dedup** (§5b) — c910 `edge_detect` -22%, `sim_loop` -4.9%.

Tried and rejected, recorded so they are not re-run:

* `XEZIM_DIRTY_EDGE` default-on (§5a) — sound but not counter-identical.
* `pid_counts` -> dense `Vec` (§6.2) — neutral at 3 and at 400 live processes.

Also tried, reverted, and then VINDICATED by a better benchmark:

* **Resume-by-cursor / frame chain** (§6b) — built in full, byte-identical,
  reverted on synthetic evidence, then measured **-3.6% median on real UVM** once
  `bench/run_uvm_bench.sh` existed. Needs re-deriving and re-landing.

Still open, ranked:

0. **Re-land the frame chain** (§6b correction) — a measured ~3.6% on UVM that
   was thrown away for want of a benchmark. Cheapest real win on this list.
1. **`Arc<[Statement]>` in the AST** (`xezim-core` parser/elaborator) — the
   prerequisite §6b ran into. It would make a splice share a block body instead
   of deep-cloning it on every loop iteration, which is where the per-statement
   cost actually lives, and only then does the frame chain pay off.
2. **Transitive clock-net collapse** (§5c) — M0 the chain-depth distribution
   first. The dedup above already collapses the *detect* cost of a replicated
   clock tree; what remains is its *settle* cost, and nothing here has measured
   that yet.
3. Reconcile the two `XEZIM_DIRTY_EDGE` counters and revisit (§5a) — the two
   optimizations are complementary: dedup removes redundant work among the
   25.7% that fire, dirty-edge removes the 74.3% that never do.
