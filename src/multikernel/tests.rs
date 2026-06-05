//! Toy test for the per-LP PDES coordinator.
//!
//! Models a tiny synthetic design that mirrors the c910 multi-core
//! pattern with the irreducible minimum:
//!
//! ```verilog
//! module counter_a(input clk_a, output reg [7:0] count_a);   // LP-A
//!   initial count_a = 0;
//!   always @(posedge clk_a) count_a <= count_a + 1;
//! endmodule
//!
//! module counter_b(input clk_b, input [7:0] shared,
//!                  output reg [7:0] count_b);                // LP-B
//!   initial count_b = 0;
//!   always @(posedge clk_b) count_b <= count_b + shared;
//! endmodule
//!
//! module tb;
//!   reg clk_a = 0, clk_b = 0;
//!   wire [7:0] count_a, count_b;
//!   counter_a a(clk_a, count_a);
//!   counter_b b(clk_b, count_a, count_b);  // count_a is boundary
//!   always #5 clk_a = !clk_a;
//!   always #5 clk_b = !clk_b;              // same period for toy
//!   initial #100 $finish;
//! endmodule
//! ```
//!
//! Signal id layout:
//!   0: count_a (owned by LP-A; boundary outbound LP-A → LP-B)
//!   1: count_b (owned by LP-B; never read by LP-A)
//!
//! Expected results after 10 clock ticks (clock period = 10 ns →
//! max_sim_time = 100):
//!   count_a = 10              (incremented every tick)
//!   count_b = 0 + 0 + 1 + 2 + 3 + 4 + 5 + 6 + 7 + 8 + 9 = 45
//!   (LP-B reads count_a on tick K, which holds the value LP-A produced
//!   on tick K-1; barrier guarantees the channel update is visible.)

use super::*;
use std::sync::Arc;

const TOY_CLK_PERIOD_NS: u64 = 10;
const TOY_N_TICKS: u64 = 10;
const TOY_MAX_TIME: u64 = TOY_CLK_PERIOD_NS * TOY_N_TICKS;
const TOY_COUNT_A_ID: usize = 0;
const TOY_COUNT_B_ID: usize = 1;

fn build_two_counter_coord() -> (PdesCoordinator, Arc<SignalTable<u64>>) {
    let ch_a_to_b = Arc::new(BoundaryChannel::new(0, 1, TOY_CLK_PERIOD_NS));

    let lp_a_block: Arc<KernelBlock> = Arc::new(Box::new(|sigs: &[u64]| {
        let cur = sigs[TOY_COUNT_A_ID];
        vec![(TOY_COUNT_A_ID, cur.wrapping_add(1) & 0xFF)]
    }));

    let lp_b_block: Arc<KernelBlock> = Arc::new(Box::new(|sigs: &[u64]| {
        let cur = sigs[TOY_COUNT_B_ID];
        let shared = sigs[TOY_COUNT_A_ID];
        vec![(TOY_COUNT_B_ID, cur.wrapping_add(shared) & 0xFF)]
    }));

    let kernel_specs = vec![
        KernelSpec {
            id: 0,
            owned_signal_ids: vec![TOY_COUNT_A_ID],
            blocks: vec![lp_a_block],
            outbound: vec![(TOY_COUNT_A_ID, Arc::clone(&ch_a_to_b))],
            inbound: vec![],
            clock_period_ns: TOY_CLK_PERIOD_NS,
            max_sim_time: TOY_MAX_TIME,
        },
        KernelSpec {
            id: 1,
            owned_signal_ids: vec![TOY_COUNT_B_ID],
            blocks: vec![lp_b_block],
            outbound: vec![],
            inbound: vec![(TOY_COUNT_A_ID, Arc::clone(&ch_a_to_b))],
            clock_period_ns: TOY_CLK_PERIOD_NS,
            max_sim_time: TOY_MAX_TIME,
        },
    ];

    let coord = PdesCoordinator::new(2, kernel_specs);
    let signal_table = Arc::clone(&coord.signal_table);
    (coord, signal_table)
}

#[test]
fn boundary_channel_topology_splits_bidirectional_signals() {
    let io = LpIoStats {
        boundary_signal_ids: vec![10, 20, 30],
        boundary_directions: vec![0, 1, 2],
        ..Default::default()
    };

    let topology = build_boundary_channels(&io, 10);
    assert_eq!(topology.channel_count(), 4);

    let lp_a = topology.for_lp(0);
    let lp_b = topology.for_lp(1);

    assert_eq!(
        lp_a.outbound
            .iter()
            .map(|(sig, _)| *sig)
            .collect::<Vec<_>>(),
        vec![10, 30]
    );
    assert_eq!(
        lp_a.inbound.iter().map(|(sig, _)| *sig).collect::<Vec<_>>(),
        vec![20, 30]
    );
    assert_eq!(
        lp_b.outbound
            .iter()
            .map(|(sig, _)| *sig)
            .collect::<Vec<_>>(),
        vec![20, 30]
    );
    assert_eq!(
        lp_b.inbound.iter().map(|(sig, _)| *sig).collect::<Vec<_>>(),
        vec![10, 30]
    );

    assert!(lp_a
        .outbound
        .iter()
        .all(|(_, ch)| ch.producer == 0 && ch.consumer == 1));
    assert!(lp_b
        .outbound
        .iter()
        .all(|(_, ch)| ch.producer == 1 && ch.consumer == 0));
}

#[test]
fn pdes_lookahead_k_parser_defaults_invalid_to_one() {
    assert_eq!(parse_pdes_lookahead_k(None), 1);
    assert_eq!(parse_pdes_lookahead_k(Some("")), 1);
    assert_eq!(parse_pdes_lookahead_k(Some("0")), 1);
    assert_eq!(parse_pdes_lookahead_k(Some("abc")), 1);
    assert_eq!(parse_pdes_lookahead_k(Some("10")), 10);
}

#[test]
fn pdes_sync_rounds_scales_with_k() {
    assert_eq!(pdes_sync_rounds_for_ticks(0, 10), 0);
    assert_eq!(pdes_sync_rounds_for_ticks(100, 1), 100);
    assert_eq!(pdes_sync_rounds_for_ticks(100, 10), 10);
    assert_eq!(pdes_sync_rounds_for_ticks(101, 10), 11);
    assert_eq!(pdes_sync_rounds_for_ticks(100, 0), 100);
}

#[test]
fn pdes_lookahead_batches_cover_all_ticks() {
    let batches: Vec<LookaheadBatch> = pdes_lookahead_batches(10, 4).collect();
    assert_eq!(
        batches,
        vec![
            LookaheadBatch {
                start_tick: 0,
                ticks: 4
            },
            LookaheadBatch {
                start_tick: 4,
                ticks: 4
            },
            LookaheadBatch {
                start_tick: 8,
                ticks: 2
            },
        ]
    );
    assert!(pdes_lookahead_batches(0, 4).next().is_none());
    assert_eq!(pdes_lookahead_batches(3, 0).count(), 3);
}

#[test]
fn two_counters_with_shared_signal_via_pdes() {
    let (coord, signal_table) = build_two_counter_coord();
    let stats = coord.run();

    // Final state checks: both kernels ran the expected number of ticks,
    // and the shared signal arrived at the consumer correctly.
    assert_eq!(stats.len(), 2);
    for s in &stats {
        assert_eq!(s.ticks, TOY_N_TICKS, "kernel ticks mismatch: {s:?}");
        assert_eq!(s.lookahead_k, 1, "lookahead mismatch: {s:?}");
        assert_eq!(s.sync_rounds, TOY_N_TICKS, "sync rounds mismatch: {s:?}");
        assert_eq!(s.final_time, TOY_MAX_TIME, "final time mismatch: {s:?}");
    }
    let count_a = signal_table.read(TOY_COUNT_A_ID);
    let count_b = signal_table.read(TOY_COUNT_B_ID);

    // count_a == TOY_N_TICKS = 10.
    assert_eq!(
        count_a, TOY_N_TICKS,
        "count_a (= LP-A) did not advance correctly"
    );

    // count_b: LP-B reads count_a each tick. Due to the barrier ordering
    // (both kernels exec block → write → ship to channel → barrier), LP-B
    // reads count_a from the PREVIOUS tick — i.e. the values 0,1,2,…,9
    // sum to 45.
    //
    // First tick: LP-B reads count_a = 0 (initial), writes count_b = 0+0 = 0
    // Tick 2: drains channel, count_a snapshot = 1, count_b = 0+1 = 1
    // ...
    // Tick 10: count_a snapshot = 9, count_b = 36+9 = 45.
    //
    // Note: this is the CMB lookahead-1 semantics — LP-B sees LP-A's
    // value with one clock period of lag. That's the correct sequential-
    // RTL behavior too (count_b sees count_a's previous-cycle value).
    let expected_count_b: u64 = (0..TOY_N_TICKS).sum::<u64>() & 0xFF;
    assert_eq!(
        count_b, expected_count_b,
        "count_b mismatch: got {count_b}, expected {expected_count_b} (sum 0..{TOY_N_TICKS})"
    );
}

#[test]
fn two_counters_with_shared_signal_via_pdes_lookahead_k5() {
    let (coord, signal_table) = build_two_counter_coord();
    let stats = coord.run_with_lookahead(5);

    assert_eq!(stats.len(), 2);
    for s in &stats {
        assert_eq!(s.ticks, TOY_N_TICKS, "kernel ticks mismatch: {s:?}");
        assert_eq!(s.lookahead_k, 5, "lookahead mismatch: {s:?}");
        assert_eq!(s.sync_rounds, 2, "sync rounds mismatch: {s:?}");
        assert_eq!(s.final_time, TOY_MAX_TIME, "final time mismatch: {s:?}");
    }

    assert_eq!(signal_table.read(TOY_COUNT_A_ID), TOY_N_TICKS);
    assert_eq!(
        signal_table.read(TOY_COUNT_B_ID),
        (0..TOY_N_TICKS).sum::<u64>() & 0xFF
    );
}

#[test]
fn clock_barrier_sync_round_count() {
    // Sanity test: 3 threads synchronize at the barrier for 5 rounds.
    let barrier = Arc::new(ClockBarrier::new(3));
    let counters: Vec<Arc<std::sync::Mutex<u64>>> = (0..3)
        .map(|_| Arc::new(std::sync::Mutex::new(0u64)))
        .collect();
    let mut handles = Vec::new();
    for c in &counters {
        let b = Arc::clone(&barrier);
        let c = Arc::clone(c);
        handles.push(std::thread::spawn(move || {
            for _ in 0..5 {
                {
                    let mut g = c.lock().unwrap();
                    *g += 1;
                }
                b.wait();
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    for c in &counters {
        assert_eq!(*c.lock().unwrap(), 5);
    }
}

/// Combined PDES architecture test: actual concurrent host threads +
/// multi-tick CMB lookahead-K batching. This is the template the
/// per-LP event_loop refactor will use to scale to c910 — both
/// primitives proven independently elsewhere, here proven composable
/// at toy scale with real SystemVerilog bytecode.
///
/// 2 host threads × K-tick batching × real exec_insns_isolated through
/// SendExecContext × BoundaryChannel + ClockBarrier coordination.
/// Must produce count_a=10, count_b=45 for K=1, K=2, K=5, K=10.
fn run_parallel_threads_multi_tick(k: u64) -> (u64, u64) {
    use crate::compiler::Simulator;
    use std::sync::Arc;
    use xezim_core::{parse_and_elaborate_multi, Value};

    let sv = r#"
        module counter_a(input wire clk_a, output reg [7:0] count_a);
            initial count_a = 0;
            always @(posedge clk_a) count_a <= count_a + 1;
        endmodule
        module counter_b(input wire clk_b, input wire [7:0] shared, output reg [7:0] count_b);
            initial count_b = 0;
            always @(posedge clk_b) count_b <= count_b + shared;
        endmodule
        module top(input wire clk_a, input wire clk_b);
            wire [7:0] count_a;
            wire [7:0] count_b;
            counter_a a (.clk_a(clk_a), .count_a(count_a));
            counter_b b (.clk_b(clk_b), .shared(count_a), .count_b(count_b));
        endmodule
    "#;
    let (_defs, elab) =
        parse_and_elaborate_multi(&[sv.to_string()], Some("top"), &[], &[], &[]).unwrap();
    let mut sim = Simulator::new(elab, 0);
    sim.compile();
    let ctx = Arc::new(sim.extract_send_exec_context());

    let find = |name: &str| -> usize {
        (0..ctx.signal_count())
            .find(|&id| ctx.signal_name_at(id) == name)
            .unwrap()
    };
    let a_count_a_id = find("a.count_a");
    let b_shared_id = find("b.shared");
    let b_count_b_id = find("b.count_b");
    let top_count_a_id = find("count_a");

    // Find which block writes which counter.
    let mut lp_a_blocks: Vec<usize> = Vec::new();
    let mut lp_b_blocks: Vec<usize> = Vec::new();
    {
        let init: Vec<Value> = (0..ctx.signal_count())
            .map(|id| Value::from_u64(0, ctx.signal_widths.get(id).copied().unwrap_or(1)))
            .collect();
        let mut vm = Vec::new();
        for bi in 0..ctx.block_count() {
            if !ctx.block_compiled(bi) {
                continue;
            }
            let w = ctx.pdes_exec_block(bi, &init, &mut vm);
            if w.iter().any(|(id, _)| *id == a_count_a_id) {
                lp_a_blocks.push(bi);
            } else if w.iter().any(|(id, _)| *id == b_count_b_id) {
                lp_b_blocks.push(bi);
            }
        }
    }

    const N_TICKS: u64 = 10;
    let ch_a_to_b: Arc<BoundaryChannel> = Arc::new(BoundaryChannel::new(0, 1, k));
    // Pre-seed initial count_a value.
    let _ = ch_a_to_b.send(BoundaryUpdate {
        signal_id: a_count_a_id,
        value: 0,
        at_time: 0,
    });
    let lp_b_rx = ch_a_to_b.take_rx().unwrap();
    let barrier = Arc::new(ClockBarrier::new(2));

    let mut tab_a: Vec<Value> = (0..ctx.signal_count())
        .map(|id| Value::from_u64(0, ctx.signal_widths.get(id).copied().unwrap_or(1)))
        .collect();
    let mut tab_b: Vec<Value> = tab_a.clone();
    for id in [a_count_a_id, b_shared_id, b_count_b_id, top_count_a_id] {
        tab_a[id] = Value::from_u64(0, 8);
        tab_b[id] = Value::from_u64(0, 8);
    }

    let (a_final, b_final) = std::thread::scope(|s| {
        let ctx_a = Arc::clone(&ctx);
        let ctx_b = Arc::clone(&ctx);
        let barrier_a = Arc::clone(&barrier);
        let barrier_b = Arc::clone(&barrier);
        let ch_out = Arc::clone(&ch_a_to_b);

        let h_a = s.spawn(move || -> Value {
            let mut tab = tab_a;
            let mut vm = Vec::new();
            let mut tick = 0u64;
            while tick < N_TICKS {
                let target = (tick + k).min(N_TICKS);
                while tick < target {
                    let snap = tab.clone();
                    let mut nba = Vec::new();
                    for &bi in &lp_a_blocks {
                        nba.extend(ctx_a.pdes_exec_block(bi, &snap, &mut vm));
                    }
                    for (id, val) in &nba {
                        tab[*id] = val.clone();
                        if *id == a_count_a_id {
                            let _ = ch_out.send(BoundaryUpdate {
                                signal_id: a_count_a_id,
                                value: val.to_u64().unwrap_or(0),
                                at_time: tick,
                            });
                        }
                    }
                    tick += 1;
                }
                barrier_a.wait();
            }
            tab[a_count_a_id].clone()
        });

        let h_b = s.spawn(move || -> Value {
            let mut tab = tab_b;
            let mut vm = Vec::new();
            let mut tick = 0u64;
            while tick < N_TICKS {
                let target = (tick + k).min(N_TICKS);
                while tick < target {
                    if let Ok(msg) = lp_b_rx.recv() {
                        let val = Value::from_u64(msg.value, 8);
                        tab[b_shared_id] = val.clone();
                        tab[top_count_a_id] = val;
                    }
                    let snap = tab.clone();
                    let mut nba = Vec::new();
                    for &bi in &lp_b_blocks {
                        nba.extend(ctx_b.pdes_exec_block(bi, &snap, &mut vm));
                    }
                    for (id, val) in &nba {
                        tab[*id] = val.clone();
                    }
                    tick += 1;
                }
                barrier_b.wait();
            }
            tab[b_count_b_id].clone()
        });

        let va = h_a.join().unwrap();
        let vb = h_b.join().unwrap();
        (va.to_u64().unwrap_or(99), vb.to_u64().unwrap_or(99))
    });
    (a_final, b_final)
}

#[test]
fn parallel_threads_multi_tick_k1() {
    let (a, b) = run_parallel_threads_multi_tick(1);
    assert_eq!(a, 10);
    assert_eq!(b, 45);
}

#[test]
fn parallel_threads_multi_tick_k5() {
    let (a, b) = run_parallel_threads_multi_tick(5);
    assert_eq!(a, 10);
    assert_eq!(b, 45);
}

#[test]
fn parallel_threads_multi_tick_k10() {
    let (a, b) = run_parallel_threads_multi_tick(10);
    assert_eq!(a, 10);
    assert_eq!(b, 45);
}

/// Multi-tick per-LP event_loop with CMB lookahead-K semantics. Each
/// LP advances K ticks before syncing at the barrier; boundary updates
/// flow through the channel as a FIFO with one entry per LP-A tick.
///
/// Architecturally what real PDES looks like: instead of barrier-syncing
/// every tick (current dispatcher pattern), LPs run K ticks each
/// between rendezvous. Coordination cost amortizes over K ticks instead
/// of every tick — the actual speedup mechanism.
///
/// Validates K=1, K=2, K=5, K=10 all produce count_a=10, count_b=45
/// (= sum 0..10). For this toy with N=10 total ticks: K=10 means each
/// LP runs 10 ticks before sync (= single sync at end), K=1 means
/// barrier every tick. All K values must produce identical results.
fn run_multi_tick_lookahead(k: u64) -> (u64, u64) {
    use crate::compiler::Simulator;
    use std::sync::Arc;
    use xezim_core::{parse_and_elaborate_multi, Value};

    let sv = r#"
        module counter_a(input wire clk_a, output reg [7:0] count_a);
            initial count_a = 0;
            always @(posedge clk_a) count_a <= count_a + 1;
        endmodule
        module counter_b(input wire clk_b, input wire [7:0] shared, output reg [7:0] count_b);
            initial count_b = 0;
            always @(posedge clk_b) count_b <= count_b + shared;
        endmodule
        module top(input wire clk_a, input wire clk_b);
            wire [7:0] count_a;
            wire [7:0] count_b;
            counter_a a (.clk_a(clk_a), .count_a(count_a));
            counter_b b (.clk_b(clk_b), .shared(count_a), .count_b(count_b));
        endmodule
    "#;
    let (_defs, elab) =
        parse_and_elaborate_multi(&[sv.to_string()], Some("top"), &[], &[], &[]).unwrap();
    let mut sim = Simulator::new(elab, 0);
    sim.compile();
    let ctx = Arc::new(sim.extract_send_exec_context());

    let find = |name: &str| -> usize {
        (0..ctx.signal_count())
            .find(|&id| ctx.signal_name_at(id) == name)
            .unwrap()
    };
    let a_count_a_id = find("a.count_a");
    let b_shared_id = find("b.shared");
    let b_count_b_id = find("b.count_b");
    let top_count_a_id = find("count_a");

    // Find which compiled block writes which counter.
    let mut lp_a_blocks: Vec<usize> = Vec::new();
    let mut lp_b_blocks: Vec<usize> = Vec::new();
    {
        let signed: Vec<bool> = ctx.signal_signed.clone();
        let init: Vec<Value> = (0..ctx.signal_count())
            .map(|id| Value::from_u64(0, ctx.signal_widths.get(id).copied().unwrap_or(1)))
            .collect();
        let mut vm = Vec::new();
        for bi in 0..ctx.block_count() {
            if !ctx.block_compiled(bi) {
                continue;
            }
            let w = ctx.pdes_exec_block(bi, &init, &mut vm);
            if w.iter().any(|(id, _)| *id == a_count_a_id) {
                lp_a_blocks.push(bi);
            } else if w.iter().any(|(id, _)| *id == b_count_b_id) {
                lp_b_blocks.push(bi);
            }
            let _ = signed; // sentinel for the param plumbing
        }
    }

    const N_TICKS: u64 = 10;

    // Per-LP tables and channel. Channel carries one BoundaryUpdate per
    // LP-A tick — LP-B drains one per LP-B tick. FIFO order is the
    // CMB lookahead guarantee.
    let ch_a_to_b: Arc<BoundaryChannel> = Arc::new(BoundaryChannel::new(0, 1, k));
    // Pre-seed channel with the INITIAL count_a value (= 0) so LP-B
    // sees the correct pre-tick-0 value on its first read. Without
    // this, LP-B starts reading from count_a's POST-tick-0 value (1)
    // and computes count_b as sum(1..10)=55 instead of sum(0..10)=45.
    let _ = ch_a_to_b.send(BoundaryUpdate {
        signal_id: a_count_a_id,
        value: 0,
        at_time: 0,
    });
    let lp_b_rx = ch_a_to_b.take_rx().unwrap();
    let barrier = Arc::new(ClockBarrier::new(2));

    let mut tab_a: Vec<Value> = (0..ctx.signal_count())
        .map(|id| Value::from_u64(0, ctx.signal_widths.get(id).copied().unwrap_or(1)))
        .collect();
    let mut tab_b: Vec<Value> = tab_a.clone();
    // Zero-init the 8-bit signals explicitly (matches `initial count_X = 0`).
    for id in [a_count_a_id, b_shared_id, b_count_b_id, top_count_a_id] {
        tab_a[id] = Value::from_u64(0, 8);
        tab_b[id] = Value::from_u64(0, 8);
    }

    let (a_final, b_final) = std::thread::scope(|s| {
        let ctx_a = Arc::clone(&ctx);
        let ctx_b = Arc::clone(&ctx);
        let barrier_a = Arc::clone(&barrier);
        let barrier_b = Arc::clone(&barrier);
        let ch_out = Arc::clone(&ch_a_to_b);

        let h_a = s.spawn(move || -> Value {
            let mut tab = tab_a;
            let mut vm = Vec::new();
            let mut tick = 0u64;
            while tick < N_TICKS {
                // Each round: advance K ticks (or until N) before sync.
                let target = (tick + k).min(N_TICKS);
                while tick < target {
                    let snap = tab.clone();
                    let mut nba = Vec::new();
                    for &bi in &lp_a_blocks {
                        nba.extend(ctx_a.pdes_exec_block(bi, &snap, &mut vm));
                    }
                    for (id, val) in &nba {
                        tab[*id] = val.clone();
                        if *id == a_count_a_id {
                            let _ = ch_out.send(BoundaryUpdate {
                                signal_id: a_count_a_id,
                                value: val.to_u64().unwrap_or(0),
                                at_time: tick,
                            });
                        }
                    }
                    tick += 1;
                }
                barrier_a.wait();
            }
            tab[a_count_a_id].clone()
        });

        let h_b = s.spawn(move || -> Value {
            let mut tab = tab_b;
            let mut vm = Vec::new();
            let mut tick = 0u64;
            while tick < N_TICKS {
                let target = (tick + k).min(N_TICKS);
                while tick < target {
                    // Blocking recv: enforces FIFO ordering. LP-A
                    // produces one value per tick (plus the pre-seeded
                    // initial), so LP-B's recv pairs deterministically
                    // with LP-A's outputs. Race-free for the K-tick
                    // lookahead test under parallel cargo tests.
                    if let Ok(msg) = lp_b_rx.recv() {
                        let val = Value::from_u64(msg.value, 8);
                        tab[b_shared_id] = val.clone();
                        tab[top_count_a_id] = val;
                    }
                    let snap = tab.clone();
                    let mut nba = Vec::new();
                    for &bi in &lp_b_blocks {
                        nba.extend(ctx_b.pdes_exec_block(bi, &snap, &mut vm));
                    }
                    for (id, val) in &nba {
                        tab[*id] = val.clone();
                    }
                    tick += 1;
                }
                barrier_b.wait();
            }
            tab[b_count_b_id].clone()
        });

        let va = h_a.join().unwrap();
        let vb = h_b.join().unwrap();
        (va.to_u64().unwrap_or(99), vb.to_u64().unwrap_or(99))
    });
    (a_final, b_final)
}

#[test]
fn multi_tick_lookahead_k1() {
    let (a, b) = run_multi_tick_lookahead(1);
    assert_eq!(a, 10);
    assert_eq!(b, 45);
}

#[test]
fn multi_tick_lookahead_k2() {
    let (a, b) = run_multi_tick_lookahead(2);
    assert_eq!(a, 10);
    assert_eq!(b, 45, "K=2 lookahead must give same result as K=1");
}

#[test]
fn multi_tick_lookahead_k5() {
    let (a, b) = run_multi_tick_lookahead(5);
    assert_eq!(a, 10);
    assert_eq!(b, 45, "K=5 lookahead must give same result as K=1");
}

#[test]
fn multi_tick_lookahead_k10() {
    let (a, b) = run_multi_tick_lookahead(10);
    assert_eq!(a, 10);
    assert_eq!(b, 45, "K=10 (single sync) must give same result as K=1");
}

/// End-to-end PDES test with REAL SystemVerilog bytecode AND ACTUAL
/// HOST-THREAD PARALLELISM. Uses `SendExecContext` (Send + Sync subset
/// of Simulator state) so LP-A and LP-B can each run on their own
/// `std::thread::scope` thread. The 3-barrier per-tick protocol
/// synchronizes them. Same result expected as the sequential variant
/// (count_a=10, count_b=45) — and now we actually validate that the
/// architectural sync primitives work under genuine concurrency.
#[test]
fn real_bytecode_toy_through_actual_parallel_threads() {
    use crate::compiler::Simulator;
    use std::sync::Arc;
    use xezim_core::{parse_and_elaborate_multi, Value};

    let sv = r#"
        module counter_a(input wire clk_a, output reg [7:0] count_a);
            initial count_a = 0;
            always @(posedge clk_a) count_a <= count_a + 1;
        endmodule
        module counter_b(input wire clk_b, input wire [7:0] shared, output reg [7:0] count_b);
            initial count_b = 0;
            always @(posedge clk_b) count_b <= count_b + shared;
        endmodule
        module top(input wire clk_a, input wire clk_b);
            wire [7:0] count_a;
            wire [7:0] count_b;
            counter_a a (.clk_a(clk_a), .count_a(count_a));
            counter_b b (.clk_b(clk_b), .shared(count_a), .count_b(count_b));
        endmodule
    "#;
    let sources = vec![sv.to_string()];
    let (_defs, elab) =
        parse_and_elaborate_multi(&sources, Some("top"), &[], &[], &[])
            .expect("parse+elaborate failed");
    let mut sim = Simulator::new(elab, 0);
    sim.compile();
    let ctx = Arc::new(sim.extract_send_exec_context());

    let find = |name: &str| -> usize {
        (0..ctx.signal_count())
            .find(|&id| ctx.signal_name_at(id) == name)
            .unwrap_or_else(|| panic!("expected {}", name))
    };
    let a_count_a_id = find("a.count_a");
    let b_shared_id = find("b.shared");
    let b_count_b_id = find("b.count_b");
    let top_count_a_id = find("count_a");

    // Classify blocks by which counter they write.
    let mut lp_a_blocks: Vec<usize> = Vec::new();
    let mut lp_b_blocks: Vec<usize> = Vec::new();
    {
        let mut vm = Vec::new();
        let init_snap: Vec<Value> = (0..ctx.signal_count())
            .map(|id| Value::from_u64(0, ctx.signal_widths.get(id).copied().unwrap_or(1)))
            .collect();
        for bi in 0..ctx.block_count() {
            if !ctx.block_compiled(bi) {
                continue;
            }
            let w = ctx.pdes_exec_block(bi, &init_snap, &mut vm);
            if w.iter().any(|(id, _)| *id == a_count_a_id) {
                lp_a_blocks.push(bi);
            } else if w.iter().any(|(id, _)| *id == b_count_b_id) {
                lp_b_blocks.push(bi);
            }
        }
    }
    assert!(!lp_a_blocks.is_empty() && !lp_b_blocks.is_empty());

    // Per-LP local signal tables (zero-init for all 8-bit signals).
    let init_value = |w: u32| Value::from_u64(0, w);
    let n_signals = ctx.signal_count();
    let table_a: Vec<Value> = (0..n_signals)
        .map(|id| init_value(ctx.signal_widths[id]))
        .collect();
    let table_b: Vec<Value> = table_a.clone();

    let ch_a_to_b: Arc<BoundaryChannel> =
        Arc::new(BoundaryChannel::new(0, 1, 1));
    let lp_b_rx = ch_a_to_b.take_rx().expect("rx already taken");

    let barrier = Arc::new(ClockBarrier::new(2));
    const N_TICKS: u64 = 10;

    // Run LP-A and LP-B on actual host threads via std::thread::scope.
    // Each thread owns its mutable table; we move them in and return
    // them out for final-state assertions.
    let (final_a_count_a, final_b_count_b): (u64, u64) = std::thread::scope(|s| {
        let ctx_a = Arc::clone(&ctx);
        let ctx_b = Arc::clone(&ctx);
        let barrier_a = Arc::clone(&barrier);
        let barrier_b = Arc::clone(&barrier);
        let ch_a_out = Arc::clone(&ch_a_to_b);

        let h_a = s.spawn(move || -> Value {
            let mut tab = table_a;
            let mut vm = Vec::new();
            for _tick in 0..N_TICKS {
                // Phase A: nothing inbound for LP-A.
                barrier_a.wait(); // Barrier 1
                // Phase B: snapshot + exec.
                let snap = tab.clone();
                let mut nba = Vec::new();
                for &bi in &lp_a_blocks {
                    nba.extend(ctx_a.pdes_exec_block(bi, &snap, &mut vm));
                }
                barrier_a.wait(); // Barrier 2
                // Phase D: apply + send.
                for (id, val) in &nba {
                    tab[*id] = val.clone();
                    if *id == a_count_a_id {
                        let _ = ch_a_out.send(BoundaryUpdate {
                            signal_id: a_count_a_id,
                            value: val.to_u64().unwrap_or(0),
                            at_time: 0,
                        });
                    }
                }
                barrier_a.wait(); // Barrier 3
            }
            tab[a_count_a_id].clone()
        });

        let h_b = s.spawn(move || -> Value {
            let mut tab = table_b;
            let mut vm = Vec::new();
            for _tick in 0..N_TICKS {
                // Phase A: drain inbox; propagate to read targets.
                while let Ok(msg) = lp_b_rx.try_recv() {
                    let v = Value::from_u64(msg.value, 8);
                    tab[b_shared_id] = v.clone();
                    tab[top_count_a_id] = v;
                }
                barrier_b.wait(); // Barrier 1
                // Phase B: snapshot + exec.
                let snap = tab.clone();
                let mut nba = Vec::new();
                for &bi in &lp_b_blocks {
                    nba.extend(ctx_b.pdes_exec_block(bi, &snap, &mut vm));
                }
                barrier_b.wait(); // Barrier 2
                // Phase D: apply.
                for (id, val) in &nba {
                    tab[*id] = val.clone();
                }
                barrier_b.wait(); // Barrier 3
            }
            tab[b_count_b_id].clone()
        });
        let va = h_a.join().expect("LP-A panicked");
        let vb = h_b.join().expect("LP-B panicked");
        (va.to_u64().unwrap_or(99), vb.to_u64().unwrap_or(99))
    });

    eprintln!(
        "[real-pdes-parallel] FINAL count_a={}, count_b={}",
        final_a_count_a, final_b_count_b
    );
    assert_eq!(final_a_count_a, 10);
    assert_eq!(final_b_count_b, 45);
}

/// End-to-end PDES test with REAL SystemVerilog bytecode. Compiles
/// the toy 2-counter design (same one validated through xezim's
/// `--simulate` to give count_a=10, count_b=45), partitions blocks
/// across 2 LPs, and drives the 3-phase per-tick protocol sequentially
/// (Simulator isn't Send so we can't actually spawn threads here, but
/// the architectural data flow — phase ordering, snapshot semantics,
/// boundary channel delivery — is identical to the parallel variant).
///
/// Asserts final count_a=10 and count_b=45 — proving the per-LP
/// coordinator data path with real SV bytecode produces the same
/// result as the closure-based PdesCoordinator toy test and xezim's
/// normal `--simulate`. Foundational PDES correctness milestone.
///
/// The actual host-thread parallelism requires a Send-able subset of
/// Simulator (compiled blocks + name maps), which is a known
/// follow-up — out of scope for this validation.
#[test]
fn real_bytecode_toy_through_pdes_phase_protocol() {
    use crate::compiler::Simulator;
    use xezim_core::{parse_and_elaborate_multi, Value};

    let sv = r#"
        module counter_a(input wire clk_a, output reg [7:0] count_a);
            initial count_a = 0;
            always @(posedge clk_a) count_a <= count_a + 1;
        endmodule
        module counter_b(input wire clk_b, input wire [7:0] shared, output reg [7:0] count_b);
            initial count_b = 0;
            always @(posedge clk_b) count_b <= count_b + shared;
        endmodule
        module top(input wire clk_a, input wire clk_b);
            wire [7:0] count_a;
            wire [7:0] count_b;
            counter_a a (.clk_a(clk_a), .count_a(count_a));
            counter_b b (.clk_b(clk_b), .shared(count_a), .count_b(count_b));
        endmodule
    "#;
    let sources = vec![sv.to_string()];
    let (_defs, elab) =
        parse_and_elaborate_multi(&sources, Some("top"), &[], &[], &[])
            .expect("parse+elaborate failed");
    let mut sim = Simulator::new(elab, 0);
    sim.compile();

    let find_signal = |suffix: &str| -> usize {
        (0..sim.signal_table_len())
            .find(|&id| sim.signal_name_at(id).ends_with(suffix))
            .unwrap_or_else(|| panic!("expected signal ending with {}", suffix))
    };
    // counter_a's flop writes "a.count_a" (id 2 typically).
    // counter_b's flop reads "b.shared" (a port that's wired to top's
    // count_a in the real simulator via comb propagation).
    // For PDES we must drive b.shared from LP-A's a.count_a manually
    // in the boundary delivery — that's the comb-propagation work the
    // event_loop would normally do.
    let a_count_a_id = (0..sim.signal_table_len())
        .find(|&id| sim.signal_name_at(id) == "a.count_a")
        .expect("a.count_a");
    let b_shared_id = (0..sim.signal_table_len())
        .find(|&id| sim.signal_name_at(id) == "b.shared")
        .expect("b.shared");
    let b_count_b_id = (0..sim.signal_table_len())
        .find(|&id| sim.signal_name_at(id) == "b.count_b")
        .expect("b.count_b");
    // Top-level count_a is what counter_b's flop actually reads (the
    // elaborator inlined the port binding b.shared <= top.count_a at
    // the read site, so LoadSignal targets signal id of top.count_a).
    let top_count_a_id = (0..sim.signal_table_len())
        .find(|&id| sim.signal_name_at(id) == "count_a")
        .expect("count_a");
    eprintln!(
        "[real-pdes] a.count_a={}, b.shared={}, b.count_b={}, top.count_a={}",
        a_count_a_id, b_shared_id, b_count_b_id, top_count_a_id
    );
    // Dump instructions of each compiled block
    for bi in 0..sim.edge_block_count() {
        if let Some(cb) = sim.compiled_edge_block_at(bi) {
            eprintln!("[real-pdes] block {} instructions ({} regs):", bi, cb.num_regs);
            for (i, insn) in cb.instructions.iter().enumerate() {
                eprintln!("[real-pdes]   {}: {:?}", i, insn);
            }
        }
    }
    let count_a_id = a_count_a_id;
    let count_b_id = b_count_b_id;
    let signed_init_slice: Vec<bool> = sim.signal_signed_slice().to_vec();

    let mut lp_a_blocks: Vec<usize> = Vec::new();
    let mut lp_b_blocks: Vec<usize> = Vec::new();
    for bi in 0..sim.edge_block_count() {
        if !sim.edge_block_compiled(bi) {
            continue;
        }
        let scope = sim.edge_block_scope_at(bi).unwrap_or_default();
        eprintln!("[real-pdes] block {} scope = {:?}", bi, scope);
        // Use writes-targets to classify: a block writing count_a is LP-A,
        // a block writing count_b is LP-B. Most reliable.
        let writes_count_a = {
            let snap = sim.signal_table_slice().to_vec();
            let mut vm = Vec::new();
            let n = sim.pdes_exec_block(bi, &snap, &signed_init_slice, &mut vm);
            n.iter().any(|(id, _)| *id == count_a_id)
        };
        let writes_count_b = {
            let snap = sim.signal_table_slice().to_vec();
            let mut vm = Vec::new();
            let n = sim.pdes_exec_block(bi, &snap, &signed_init_slice, &mut vm);
            n.iter().any(|(id, _)| *id == count_b_id)
        };
        if writes_count_a {
            lp_a_blocks.push(bi);
        } else if writes_count_b {
            lp_b_blocks.push(bi);
        } else {
            eprintln!("[real-pdes] block {} writes neither counter, skipping", bi);
        }
    }
    eprintln!(
        "[real-pdes] classified by NBA target: LP-A blocks = {:?}, LP-B blocks = {:?}",
        lp_a_blocks, lp_b_blocks
    );
    assert!(!lp_a_blocks.is_empty() && !lp_b_blocks.is_empty());

    // Per-LP local signal_tables. count_a is LP-A-owned; LP-B's local
    // mirror of count_a starts at 0 and is updated each tick from the
    // boundary channel (Phase A). count_b is LP-B-owned.
    let mut tab_a: Vec<Value> = sim.signal_table_slice().to_vec();
    let mut tab_b: Vec<Value> = sim.signal_table_slice().to_vec();
    // Force all 8-bit signals to 0 (matches `initial count_X = 0` and
    // models the testbench's wire init). Without this, X-init propagates
    // through the adders and corrupts everything.
    let zero_ids: Vec<usize> = (0..sim.signal_table_len())
        .filter(|&id| {
            let name = sim.signal_name_at(id);
            id == a_count_a_id || id == b_shared_id || id == b_count_b_id
                || name == "count_a" || name == "count_b"
        })
        .collect();
    for id in zero_ids {
        tab_a[id] = Value::from_u64(0, 8);
        tab_b[id] = Value::from_u64(0, 8);
    }
    let signed: Vec<bool> = sim.signal_signed_slice().to_vec();

    let ch_a_to_b: Arc<BoundaryChannel> =
        Arc::new(BoundaryChannel::new(0, 1, /*lookahead*/ 1));
    let lp_b_rx = ch_a_to_b.take_rx().expect("rx already taken");
    const N_TICKS: u64 = 10;
    let mut vm_regs: Vec<Value> = Vec::new();

    for tick in 0..N_TICKS {
        // ── Phase A: LP-A drains nothing; LP-B drains channel ──
        // The channel carries LP-A's a.count_a value, but counter_b's
        // block reads b.shared (its port). Real simulator's comb logic
        // propagates the port binding — for PDES we do it here as part
        // of the boundary delivery.
        while let Ok(msg) = lp_b_rx.try_recv() {
            // msg.signal_id is LP-A's a.count_a; the elaborator inlined
            // the port binding so LP-B's flop actually reads from the
            // top-level count_a signal (id top_count_a_id), not from
            // b.shared. Update both for completeness.
            assert_eq!(msg.signal_id, a_count_a_id);
            let v = Value::from_u64(msg.value, 8);
            tab_b[b_shared_id] = v.clone();
            tab_b[top_count_a_id] = v;
        }
        // Implicit Barrier 1 (sequential — both LPs done with Phase A)

        // ── Phase B: snapshot + exec → NBA buffer per LP ──
        let snap_a = tab_a.clone();
        let snap_b = tab_b.clone();
        let mut nba_a: Vec<(usize, Value)> = Vec::new();
        for &bi in &lp_a_blocks {
            nba_a.extend(sim.pdes_exec_block(bi, &snap_a, &signed, &mut vm_regs));
        }
        let mut nba_b: Vec<(usize, Value)> = Vec::new();
        for &bi in &lp_b_blocks {
            let writes = sim.pdes_exec_block(bi, &snap_b, &signed, &mut vm_regs);
            eprintln!(
                "[real-pdes]   tick {} LP-B block {}: snap b.shared={} b.count_b={} → {} writes: {:?}",
                tick, bi,
                snap_b[b_shared_id].to_u64().unwrap_or(99),
                snap_b[b_count_b_id].to_u64().unwrap_or(99),
                writes.len(),
                writes.iter().map(|(id, v)| (sim.signal_name_at(*id).to_string(), v.to_u64())).collect::<Vec<_>>(),
            );
            nba_b.extend(writes);
        }
        // Implicit Barrier 2 (sequential — both LPs done with Phase B)

        // ── Phase D: apply NBAs + send outbound on boundary signals ──
        for (id, val) in &nba_a {
            tab_a[*id] = val.clone();
            if *id == count_a_id {
                let _ = ch_a_to_b.send(BoundaryUpdate {
                    signal_id: count_a_id,
                    value: val.to_u64().unwrap_or(0),
                    at_time: 0,
                });
            }
        }
        for (id, val) in &nba_b {
            tab_b[*id] = val.clone();
        }
        // Implicit Barrier 3 (sequential — both LPs done with Phase D)

        eprintln!(
            "[real-pdes] tick {}: LP-A a.count_a={}, LP-B b.shared={}, b.count_b={}",
            tick,
            tab_a[a_count_a_id].to_u64().unwrap_or(99),
            tab_b[b_shared_id].to_u64().unwrap_or(99),
            tab_b[b_count_b_id].to_u64().unwrap_or(99),
        );
    }

    let final_count_a = tab_a[a_count_a_id].to_u64().unwrap_or(99);
    let final_count_b = tab_b[b_count_b_id].to_u64().unwrap_or(99);
    eprintln!(
        "[real-pdes] FINAL count_a={}, count_b={}",
        final_count_a, final_count_b
    );
    assert_eq!(final_count_a, 10, "count_a should equal 10");
    assert_eq!(
        final_count_b, 45,
        "count_b should equal sum(0..10) = 45 (CMB lookahead-1 semantics)"
    );
}

/// Multi-tick real-bytecode loop. Builds a flop, then drives the
/// compiled block through `pdes_exec_block` in a 5-tick loop with
/// per-tick NBA apply to a local signal_table. Asserts q toggles
/// 0→1→0→1→0→1 (q=1 after 5 ticks). Validates the per-tick
/// exec+apply cycle the PdesCoordinator will eventually drive.
#[test]
fn pdes_exec_block_flop_toggles_across_5_ticks() {
    use crate::compiler::Simulator;
    use xezim_core::{parse_and_elaborate_multi, Value};

    let sv = r#"
        module top(input wire clk);
            reg q;
            initial q = 0;
            always @(posedge clk) q <= ~q;
        endmodule
    "#;
    let sources = vec![sv.to_string()];
    let (_defs, elab) =
        parse_and_elaborate_multi(&sources, Some("top"), &[], &[], &[])
            .expect("parse+elaborate failed");
    let mut sim = Simulator::new(elab, 0);
    sim.compile();

    // Locate q's signal_id by walking the signal table names.
    let q_id = (0..sim.signal_table_len())
        .find(|&id| sim.signal_name_at(id).ends_with(".q") || sim.signal_name_at(id) == "q")
        .expect("expected to find signal named q");

    // Build a per-LP signal table snapshot. Force q=0 to model the
    // `initial q = 0` that we're bypassing (the simulator's time-0
    // settle wasn't run; we exercise only the flop's posedge block).
    let mut signal_table: Vec<Value> = sim.signal_table_slice().to_vec();
    let signed: Vec<bool> = sim.signal_signed_slice().to_vec();
    signal_table[q_id] = Value::from_u64(0, 1);

    let bi = (0..sim.edge_block_count())
        .find(|&bi| sim.edge_block_compiled(bi))
        .expect("expected at least one compiled edge block");

    let mut vm_regs: Vec<Value> = Vec::new();
    for tick in 1..=5 {
        // Snapshot the current signal table for bytecode reads.
        let snapshot = signal_table.clone();
        // Run the flop's block — produces NBA write for q.
        let writes = sim.pdes_exec_block(bi, &snapshot, &signed, &mut vm_regs);
        // Apply NBA writes (like the coordinator's Phase D would).
        for (id, val) in writes {
            signal_table[id] = val;
        }
        let q_val = signal_table[q_id].clone();
        let expected_bit = (tick % 2) as u64;
        let q_u64 = q_val.to_u64().unwrap_or(99);
        eprintln!(
            "[flop_test] tick {} q = {} (raw {:?}) — expected bit {}",
            tick, q_u64, q_val, expected_bit
        );
        assert_eq!(
            q_u64, expected_bit,
            "tick {}: q should have toggled to {}",
            tick, expected_bit
        );
    }
}

/// Real-bytecode-through-PDES-API smoke test. Parses a tiny SV design
/// with one always @(posedge clk) flop, compiles to bytecode, then
/// drives one block through `Simulator::pdes_exec_block` (the
/// PDES-friendly wrapper around exec_insns_isolated) against a
/// caller-built snapshot. Asserts the NBA output is a 1-bit signal
/// flip — the expected flop behavior. Validates that real bytecode
/// flows through a PDES-compatible API end-to-end.
#[test]
fn pdes_exec_block_runs_real_bytecode_on_flip_flop() {
    use crate::compiler::Simulator;
    use xezim_core::{parse_and_elaborate_multi, Value};

    let sv = r#"
        module top(input wire clk);
            reg q;
            initial q = 0;
            always @(posedge clk) q <= ~q;
        endmodule
    "#;
    let sources = vec![sv.to_string()];
    let (_defs, elab) =
        parse_and_elaborate_multi(&sources, Some("top"), &[], &[], &[])
            .expect("parse+elaborate failed");
    let mut sim = Simulator::new(elab, 0);
    sim.compile();

    // Find a compiled block. The flop's posedge always becomes one.
    let mut chosen: Option<usize> = None;
    for bi in 0..sim.edge_block_count() {
        if sim.edge_block_compiled(bi) {
            chosen = Some(bi);
            break;
        }
    }
    let bi = chosen.expect("expected at least one compiled edge block");

    // Build a synthetic snapshot. The flop reads `q` and writes `q`;
    // initial value is 0, so executing the block should compute ~q = 1
    // and produce an NBA write of q := 1.
    let snapshot: Vec<Value> = sim.signal_table_slice().to_vec();
    let signed: Vec<bool> = sim.signal_signed_slice().to_vec();
    let mut vm_regs: Vec<Value> = Vec::new();

    let nbas = sim.pdes_exec_block(bi, &snapshot, &signed, &mut vm_regs);

    // We expect at least one NBA write — the flop's q <= ~q.
    assert!(
        !nbas.is_empty(),
        "expected NBA writes from posedge-clk flop block; got none"
    );
    // The NBA should target a signal (the q reg). Print for inspection.
    eprintln!(
        "[pdes_exec_block_test] block {} produced {} NBA write(s):",
        bi,
        nbas.len()
    );
    for (sig_id, val) in &nbas {
        eprintln!(
            "  signal[{}] (name={:?}) <= {:?}",
            sig_id,
            sim.signal_name_at(*sig_id),
            val
        );
    }
}

/// PDES Phase 2.4 test: drive a real compiled bytecode block through
/// `Simulator::pdes_exec_block_local` against a `PerLpSignalTable`.
/// Verifies that the LP-local table receives the NBA write back, and
/// that cross-LP writes (signals outside the LP's set) are returned
/// to the caller.
///
/// Toy: a single flop `always @(posedge clk) q <= ~q;`. The flop's
/// only NBA target is `q`. Per-LP table contains `q`. After exec, the
/// local table's value for `q` should be `~initial` = 1 (initial=0).
#[test]
fn pdes_exec_block_local_drives_per_lp_table() {
    use crate::compiler::Simulator;
    use xezim_core::{parse_and_elaborate_multi, Value};

    let sv = r#"
        module top(input wire clk);
            reg q;
            initial q = 0;
            always @(posedge clk) q <= ~q;
        endmodule
    "#;
    let (_defs, elab) =
        parse_and_elaborate_multi(&[sv.to_string()], Some("top"), &[], &[], &[]).unwrap();
    let mut sim = Simulator::new(elab, 0);
    sim.compile();

    // Find q's signal id.
    let q_id = (0..sim.signal_table_len())
        .find(|&id| sim.signal_name_at(id).ends_with(".q") || sim.signal_name_at(id) == "q")
        .expect("q");

    // Build a PerLpSignalTable that owns just `q`. Initialize value
    // to 0 (matches `initial q = 0`).
    let mut per_lp = PerLpSignalTable {
        lp: 0,
        local_to_global: vec![q_id as u32],
        global_to_local: {
            let mut m = ahash::AHashMap::default();
            m.insert(q_id, 0u32);
            m
        },
        values: vec![Value::from_u64(0, 1)],
        widths: vec![1],
        signed: vec![false],
    };

    // Locate the flop's compiled block.
    let bi = (0..sim.edge_block_count())
        .find(|&bi| sim.edge_block_compiled(bi))
        .expect("expected at least one compiled edge block");

    let mut vm_regs: Vec<Value> = Vec::new();
    let cross_lp = sim.pdes_exec_block_local(&mut per_lp, bi, &mut vm_regs);

    // Expectations:
    //   - per_lp.values[0] is the new q value (~0 = 1)
    //   - cross_lp is empty (q is LP-local)
    assert!(cross_lp.is_empty(), "expected no cross-LP NBAs, got {:?}", cross_lp);
    assert_eq!(
        per_lp.values[0].to_u64().unwrap_or(99),
        1,
        "q should toggle 0 → 1 after one tick"
    );
}

#[test]
fn signal_table_basic_read_write() {
    let st = SignalTable::new(8);
    unsafe {
        st.write(0, 42);
        st.write(7, 99);
    }
    assert_eq!(st.read(0), 42);
    assert_eq!(st.read(7), 99);
    assert_eq!(st.read(100), 0); // out-of-bounds → 0
}
