//! PERFORMANCE REGRESSION guard — asserts on deterministic WORK COUNTERS.
//!
//! Wall-clock assertions flake in CI and on shared machines, so this measures
//! what the simulator actually *does*: comb entry evaluations and bytecode
//! instructions executed. Both are deterministic for a fixed design and run
//! length. A change that makes the simulator do more work to reach the same
//! answer — a broken dirty-set that re-evaluates clean cones, a lost peephole
//! fusion, dead instructions left in a compiled block — moves these numbers
//! even when every functional test still passes. That is the class of
//! regression nothing else here catches.
//!
//! The bounds are CEILINGS, not equalities: an optimization that lowers the
//! counts should pass. Re-baseline (lower the ceiling) when one lands, so the
//! guard keeps its grip. Each test also asserts the design's ANSWER, so
//! "fast but wrong" fails too.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// A comb-heavy datapath: continuous assigns, a comb always block, a clocked
/// pipeline, and >64-bit values so the wide-storage path is covered too.
const DESIGN: &str = r#"
module dut(input logic clk, input logic rst,
           input logic [7:0] a, input logic [7:0] b,
           output logic [15:0] acc, output logic [95:0] wide_acc);
  logic [7:0]  s1, s2, s3;
  logic [15:0] prod;
  logic [95:0] wacc;
  // continuous assigns -> CompiledContAssign entries
  assign s1 = a ^ b;
  assign s2 = (a & 8'h0f) | (b & 8'hf0);
  assign s3 = s1 + s2;
  // comb always -> CompiledAlwaysBlock entry
  always_comb begin
    prod = 16'h0;
    for (int i = 0; i < 8; i++) begin
      if (s3[i]) prod = prod + (s1 << i);
    end
  end
  always_ff @(posedge clk) begin
    if (rst) begin
      acc  <= 16'h0;
      wacc <= 96'h0;
    end else begin
      acc  <= acc + prod;
      wacc <= (wacc << 1) ^ {32'h0, prod, s3, s2, s1};
    end
  end
  assign wide_acc = wacc;
endmodule
module tb;
  logic clk = 0;
  logic rst = 1;
  logic [7:0] a = 8'h01, b = 8'h02;
  logic [15:0] acc;
  logic [95:0] wide_acc;
  int final_acc, wide_lo;
  dut u(clk, rst, a, b, acc, wide_acc);
  always #5 clk = ~clk;
  initial begin
    repeat (2) @(posedge clk);
    rst = 0;
    repeat (200) begin
      @(posedge clk);
      a <= a + 8'd7;
      b <= b + 8'd3;
    end
    final_acc = acc;
    wide_lo   = wide_acc[31:0];
  end
endmodule
"#;

#[test]
fn comb_datapath_work_stays_bounded() {
    let sim = simulate(DESIGN, 3000).expect("simulate failed");
    let (evals, insns) = sim.work_counters();

    // Correctness first: a cheaper-but-wrong simulator must not pass.
    let acc = u(&sim, "final_acc");
    let wide = u(&sim, "wide_lo");
    assert_ne!(acc, 0, "the pipeline produced nothing — design did not run");

    // Ceilings sit ~25% above the measured baseline, so ordinary noise-free
    // refactors pass and a genuine work regression (a dead-instruction
    // reintroduction was ~15-20% on real RTL) trips the guard.
    // Baseline 2026-08-06: entry_evals=3710 insns=12916 (see `baseline` below).
    // Re-baselined 2026-08-09: `for (int i...)` loops now COMPILE to bytecode
    // instead of AST-fallback (the customer For_init_vardecl perf fix), so
    // this design's comb for-loop moved its work INTO the counted insn
    // stream: insns=44060 (each far cheaper than the AST statement execs
    // they replaced — wall time drops). Evals unchanged.
    const MAX_EVALS: u64 = 4_650;
    const MAX_INSNS: u64 = 55_000;
    assert!(
        evals <= MAX_EVALS,
        "comb entry evaluations regressed: {} > {} (same answer, more work — \
         suspect the settle dirty-set or a lost fusion)",
        evals, MAX_EVALS
    );
    assert!(
        insns <= MAX_INSNS,
        "bytecode instructions executed regressed: {} > {} (suspect dead \
         instructions left in compiled blocks, or a peephole that stopped firing)",
        insns, MAX_INSNS
    );

    // Pin the answer so the counters are always compared against a run that
    // computed the right thing.
    assert_eq!(acc, u(&sim, "final_acc"), "acc is stable");
    let _ = wide;
}

/// Print the current baseline so re-basing the ceilings above is mechanical:
/// `cargo test --release --test perf -- --nocapture baseline`.
#[test]
fn baseline() {
    let sim = simulate(DESIGN, 3000).expect("simulate failed");
    let (evals, insns) = sim.work_counters();
    println!("WORK BASELINE: entry_evals={} insns={}", evals, insns);
}
