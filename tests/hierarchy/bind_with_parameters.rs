//! `bind dut harness #(.N(N)) v_h (.*);` — a bind directive carrying a
//! parameter value assignment was dropped by the parser (it returned nothing
//! and the caller made a silent `Null` item), so the harness was never
//! instantiated: hierarchical reads of it gave x, task calls into it were
//! no-ops, and writes created a fresh implicit variable. A scoreboard called
//! that way "passed" without ever running. The parameters now reach the
//! bound instance. Reference-verified.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn parametrised_bind_instantiates_the_harness() {
    let msgs = messages(
        "typedef bit [6:0] u7_t;
module dut #(parameter int NUM_ROWS = 2, parameter int NUM_COLS = 2)(input logic clk);
endmodule
bind dut dut_harness #(
    .NUM_ROWS(NUM_ROWS),
    .NUM_COLS(NUM_COLS)
) v_tl_harness (.*);
module dut_harness #(parameter NUM_ROWS = 2, NUM_COLS = 2);
  int req_count[NUM_ROWS][NUM_COLS];
  bit checks_done;
  initial begin checks_done = 0; $display(\"HARNESS rows=%0d cols=%0d\", NUM_ROWS, NUM_COLS); end
  task automatic run_checks(ref u7_t [NUM_ROWS-1:0][NUM_COLS-1:0] want);
    for (int r = 0; r < NUM_ROWS; r++) for (int c = 0; c < NUM_COLS; c++) req_count[r][c] = want[r][c];
    checks_done = 1;
  endtask
  task automatic simple(); checks_done = 1; endtask
endmodule
module tb;
  localparam int NUM_ROWS = 5, NUM_COLS = 2;
  logic clk = 0;
  u7_t [NUM_ROWS-1:0][NUM_COLS-1:0] want;
  dut #(.NUM_ROWS(NUM_ROWS), .NUM_COLS(NUM_COLS)) u_dut(.clk(clk));
  initial begin
    want = '0; want[0][0] = 42; want[4][1] = 9;
    #1 $display(\"R1 done=%b\", u_dut.v_tl_harness.checks_done);
    u_dut.v_tl_harness.simple();
    $display(\"R2 done=%b\", u_dut.v_tl_harness.checks_done);
    u_dut.v_tl_harness.checks_done = 0;
    u_dut.v_tl_harness.run_checks(want);
    $display(\"R3 done=%b rc00=%0d rc41=%0d\", u_dut.v_tl_harness.checks_done,
             u_dut.v_tl_harness.req_count[0][0], u_dut.v_tl_harness.req_count[4][1]);
    $finish;
  end
endmodule
",
    );
    for want in ["HARNESS rows=5 cols=2", "R1 done=0", "R2 done=1", "R3 done=1 rc00=42 rc41=9"] {
        assert!(msgs.iter().any(|m| m == want), "missing {want}; got {msgs:?}");
    }
}
