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
