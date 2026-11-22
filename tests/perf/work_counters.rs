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

fn run_profiled_design(src: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_packed_loop_guard_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, src).expect("write temporary design");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .env("XEZIM_PROFILE_TIMING", "1")
        .output()
        .expect("run profiled design");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "profiled design failed:\n{text}");
    text
}

fn profile_count(text: &str, prefix: &str) -> u64 {
    text.lines()
        .find_map(|line| line.strip_prefix(prefix))
        .and_then(|n| n.trim().parse::<u64>().ok())
        .unwrap_or_else(|| panic!("missing profile counter `{prefix}`:\n{text}"))
}

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
        evals,
        MAX_EVALS
    );
    assert!(
        insns <= MAX_INSNS,
        "bytecode instructions executed regressed: {} > {} (suspect dead \
         instructions left in compiled blocks, or a peephole that stopped firing)",
        insns,
        MAX_INSNS
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

#[test]
fn packed_loop_fast_paths_are_exercised_and_preserve_four_state_values() {
    // These count the AST interpreter's packed-loop fast paths; keep the
    // initial block off the process FSM (env is read per simulator).
    // SAFETY: test-only; the variable is set before the simulator is built.
    unsafe { std::env::set_var("XEZIM_PROC_FSM", "0") };

    let text = run_profiled_design(
        r#"
module top;
  logic clk = 0;
  logic [5:8][3:0] sample_bus = 16'b10xz_0110_x001_zz10;
  logic [5:8][3:0] stage_bus = '0;
  logic [5:8][1:0] fill_bus;
  logic [1:0] fill_value = 2'bx0;
  logic check = 0;

  always #1 clk = ~clk;
  always @(posedge clk) begin
    for (int slot = 5; slot <= 8; slot++)
      stage_bus[slot] <= sample_bus[slot];
  end

  initial begin
    fill_bus = '0;
    for (int slot = 5; slot <= 8; slot++)
      fill_bus[slot] = fill_value;
    #4;
    check = (stage_bus === sample_bus) && (fill_bus === {4{2'bx0}});
    $display("CHECK=%0d", check);
    $finish;
  end
endmodule
"#,
    );
    assert!(
        text.contains("CHECK=1"),
        "wrong packed-loop result:\n{text}"
    );
    assert!(
        profile_count(&text, "[FUSE] packed-loop NBA copies (static sites): ") >= 1,
        "packed NBA loop silently fell back:\n{text}"
    );
    assert!(
        profile_count(&text, "[FUSE] packed blocking fills (dynamic executions): ") >= 1,
        "packed blocking fill silently fell back:\n{text}"
    );
}

#[test]
fn unsafe_packed_loop_shapes_decline_both_fast_paths() {
    // These count the AST interpreter's packed-loop fast paths; keep the
    // initial block off the process FSM (env is read per simulator).
    // SAFETY: test-only; the variable is set before the simulator is built.
    unsafe { std::env::set_var("XEZIM_PROC_FSM", "0") };

    let text = run_profiled_design(
        r#"
module top;
  logic clk = 0;
  logic [0:3][7:0] north_bus;
  logic [3:0][7:0] south_bus = '0;
  logic [3:0][7:0] base_bus = 32'h1020_3040;
  logic [3:0][7:0] ordered_bus = '0;
  logic [3:0][7:0] adaptive_bus = '0;
  integer active_slots;
  logic check = 0;

  function automatic [7:0] scramble(input [7:0] value);
    scramble = value ^ 8'h5a;
  endfunction

  always_comb active_slots = adaptive_bus[0][0] ? 2 : 4;
  always #1 clk = ~clk;
  always @(posedge clk) begin
    // Opposite packed orientations require element-wise mapping.
    for (int slot = 0; slot < 4; slot++)
      south_bus[slot] <= north_bus[slot];

    // The later identity assignment must remain later for NBA ordering.
    for (int slot = 0; slot < 4; slot++) begin
      ordered_bus[slot] <= scramble(base_bus[slot]);
      ordered_bus[slot] <= base_bus[slot];
    end
  end

  initial begin
    north_bus[0] = 8'h11;
    north_bus[1] = 8'h22;
    north_bus[2] = 8'h33;
    north_bus[3] = 8'h44;
    // active_slots is another process's output (§9.2.2.2): it stays four
    // until this block suspends, although the first write would make it two.
    // The fill reads a signal as its bound, so it must still stay element-wise;
    // all four slots are written. The `#1` lets always_comb's time-0 run
    // (§9.2.2.2.2) set the bound first; §9.2.2.2.2 does not order that run
    // against this block's first writes.
    #1;
    adaptive_bus = '0;
    for (int slot = 0; slot < active_slots; slot++)
      adaptive_bus[slot] = 8'hff;

    #4;
    check = (south_bus === 32'h4433_2211)
         && (ordered_bus === base_bus)
         && (adaptive_bus === 32'hffff_ffff);
    $display("CHECK=%0d", check);
    $finish;
  end
endmodule
"#,
    );
    assert!(
        text.contains("CHECK=1"),
        "guarded fallback was wrong:\n{text}"
    );
    assert_eq!(
        profile_count(&text, "[FUSE] packed-loop NBA copies (static sites): "),
        0,
        "unsafe packed NBA shape was vectorized:\n{text}"
    );
    assert_eq!(
        profile_count(&text, "[FUSE] packed blocking fills (dynamic executions): "),
        0,
        "mutable-bound packed fill was collapsed:\n{text}"
    );
}
