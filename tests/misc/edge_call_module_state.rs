//! §13.4 / §9.4.2 — an `always_ff` that calls a function reading or
//! writing module state. A small callee is inlined into the compiled edge
//! block; a large one (the VHDL kernel's next-time scan) stays a call
//! (`CallFreeFunction`). Either way the block must run on every edge, see
//! the module signals the function reads (the event-edge skip cannot see
//! them in a call), and evaluate each actual at its formal's width
//! (§11.6.1: `f(a + b)` keeps the carry of 8-bit operands into a 9-bit
//! formal). Expected lines follow the LRM and the reference simulator.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// A function body of `n` statements, each reading the module signal `b`:
/// it inlines to well over the edge block's limit.
fn big_body(n: usize) -> String {
    (0..n)
        .map(|k| format!("    if (x == 9'd{k}) r = r + 9'd1;\n"))
        .collect()
}

fn design(n: usize) -> String {
    format!(
        r#"
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [7:0] a = 200, b = 100, c = 1, e = 0;
  logic [8:0] lim = 9'd400, q, qc, g;
  int cnt = 0;
  function automatic [8:0] sat(input [8:0] s);
    logic [8:0] r, x;
    r = 0;
    x = s;
{body}    return (s > lim) ? lim : s + r - r;
  endfunction
  function automatic [8:0] follow(input [7:0] s);
    logic [8:0] r, x;
    r = 0;
    x = s;
{body}    return s + e + r - r;
  endfunction
  function automatic void tick();
    logic [8:0] r, x;
    r = 0;
    x = 9'(cnt);
{body}    cnt++;
  endfunction
  always_ff @(posedge clk) q <= sat(a + b);
  always_ff @(posedge clk) g <= follow(c);
  always_ff @(posedge clk) tick();
  assign qc = sat(a + b);
  initial begin
    @(posedge clk);
    #1 $display("q=%0d qc=%0d g=%0d", q, qc, g);
    e = 5;
    repeat (2) @(posedge clk);
    #1 $display("g=%0d", g);
    repeat (20) @(posedge clk);
    #1 $display("cnt=%0d", cnt);
    $finish;
  end
endmodule
"#,
        body = big_body(n)
    )
}

#[test]
fn small_module_state_callee_in_edge_block() {
    assert_eq!(lines(&design(2)), vec!["q=300 qc=300 g=1", "g=6", "cnt=23"]);
}

#[test]
fn large_module_state_callee_in_edge_block() {
    assert_eq!(
        lines(&design(300)),
        vec!["q=300 qc=300 g=1", "g=6", "cnt=23"]
    );
}
