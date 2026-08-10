//! An edge block whose ONLY interpreter escape is an expression-level
//! `EvalExprFallback` must not be classified parallel-eligible: the parallel
//! executor's instruction match treats every fallback as unreachable and
//! panicked ("parallel block should not contain fallback/blocking/NbaRangeDyn
//! instructions") the first time such a block fired inside a parallel
//! dispatch. The design below organically crosses the parallel-dispatch
//! qualification threshold (>=2 eligible blocks, >=10k combined static
//! insns) the same way the full-chip run that caught this did.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

#[test]
fn impure_call_block_stays_off_the_parallel_path() {
    // 192 generate instances x a fat compiled always_ff body -> well past the
    // 10k-static-insn parallel qualification bar; one extra block holds the
    // impure call (reads `mode` without taking it as an argument), which
    // compiles to EvalExprFallback.
    let src = r#"
module lane(input logic clk, input logic [31:0] seed, output logic [31:0] acc);
  always_ff @(posedge clk) begin
    acc <= ((((acc ^ seed) + 32'h9e37_79b9) ^ (acc >> 3)) + (seed << 1))
         ^ ((((acc + 32'h85eb_ca6b) ^ (seed >> 5)) + (acc << 2)) ^ 32'h27d4_eb2f)
         ^ ((((seed ^ 32'hc2b2_ae35) + (acc >> 7)) ^ (seed << 3)) + 32'h1656_67b1)
         ^ ((((acc ^ 32'h68e3_1da4) + (seed >> 2)) ^ (acc << 4)) + 32'hb529_7a4d)
         ^ ((((seed + 32'hdead_beef) ^ (acc >> 1)) + (seed << 6)) ^ 32'hcafe_babe)
         ^ ((((acc + 32'h0123_4567) ^ (seed >> 4)) + (acc << 5)) ^ 32'h89ab_cdef)
         ^ ((((seed ^ 32'hfee1_dead) + (acc >> 6)) ^ (seed << 2)) + 32'hface_feed)
         ^ ((((acc ^ 32'h0bad_f00d) + (seed >> 8)) ^ (acc << 1)) + 32'h8bad_beef);
  end
endmodule
module tb;
  logic clk = 0;
  logic [31:0] mode = 32'h11;
  logic [31:0] fb_r;
  int fin;
  always #5 clk = ~clk;

  // Impure: reads `mode` from module scope -> Expr_Call_impure ->
  // expression-level fallback inside an otherwise-compiled edge block.
  function automatic logic [31:0] mixm(input logic [31:0] x);
    return x + mode;
  endfunction
  always_ff @(posedge clk) fb_r <= mixm(fb_r) ^ 32'h5;

  genvar g;
  logic [31:0] accs [192];
  generate
    for (g = 0; g < 192; g++) begin : lanes
      lane u(.clk(clk), .seed(32'h100 + g), .acc(accs[g]));
    end
  endgenerate

  initial begin
    fb_r = 0;
    repeat (40) @(posedge clk);
    fin = fb_r;
  end
endmodule
"#;
    // The dispatcher calibrates for its first 64 qualifying ticks (always
    // sequential), so a short run never threads on its own. FORCE_PARALLEL is
    // read uncached at every dispatch and only affects designs that qualify
    // (>=2 eligible blocks, >=10k insns) — no other test in this binary does.
    unsafe { std::env::set_var("XEZIM_FORCE_PARALLEL", "1") };
    let sim = simulate(src, 1000).expect("simulate failed");
    unsafe { std::env::remove_var("XEZIM_FORCE_PARALLEL") };
    // `fin = fb_r` runs at the 40th posedge BEFORE that edge's NBA lands, so
    // it observes 39 applied updates of fb_r <= (fb_r + 'h11) ^ 5, from 0.
    let mut expect: u64 = 0;
    for _ in 0..39 {
        expect = ((expect + 0x11) ^ 0x5) & 0xffff_ffff;
    }
    assert_eq!(
        u(&sim, "fin"),
        expect,
        "impure-call edge block computed the wrong value (or the parallel \
         classifier let a fallback block onto a worker thread)"
    );
}
