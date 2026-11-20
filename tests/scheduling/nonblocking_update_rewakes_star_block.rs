//! IEEE 1800-2023 §9.4.2.2: `@*` adds every variable its statement reads,
//! including one the statement also writes (the LRM's example `tmp1 = a & b;
//! tmp2 = c & d; y = tmp1 | tmp2;` is `@(a or b or c or d or tmp1 or
//! tmp2)`). In `always @(*) begin dot <= b; prod <= a * dot; end` the
//! nonblocking update of `dot` lands in the NBA region after the block is
//! back at `@*` (§4.4), so it wakes the block again in the same time step
//! and `prod` is computed from the new `dot`. xezim ran the block once, and
//! `prod` lagged a cycle behind, as in the DSP model of an Intel FPGA NPU.
//!
//! `always_comb` leaves out every variable it writes (§9.2.2.2.1(b)), so its
//! copy keeps the one-step lag. An explicit `@(a or b or d)` wakes on `d` as
//! listed. A blocking temporary is written while the block runs and does not
//! wake it, but a variable some branch writes with `<=` still does. A block
//! that bumps `x[7:4]` with `=` beside `x[3:0] <= b` runs once for the change
//! of `b` and once more for the NBA update of `x`, then waits: its own
//! blocking write does not wake it again. Expected values match Icarus.

use xezim::simulate;

#[test]
fn nonblocking_update_of_a_read_variable_rewakes_star_block() {
    let src = r#"
module t;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [7:0] a = 0, b = 0;
  logic rst = 0;
  logic [7:0] d1, d2, d3, d4, d5, tmp;
  logic [7:0] x = 0;
  logic [15:0] p1, p2, p3, p4, p5, p6;
  always @(*) begin d1 <= b; p1 <= a * d1; end
  always @* begin d4 <= b; p5 <= a * d4; end
  always_comb begin d2 <= b; p2 <= a * d2; end
  always @(a or b or d3) begin d3 <= b; p3 <= a * d3; end
  always @* begin tmp = b + 1; p4 = a * tmp; end
  always @* begin if (rst) d5 = 0; else d5 <= b; p6 <= a * d5; end
  always @* begin x[7:4] = x[7:4] + 1; x[3:0] <= b; end
  initial begin
    @(negedge clk) begin a = 3; b = 7; end
    @(negedge clk) begin a = 3; b = 10; end
    @(negedge clk) begin a = 2; b = 10; end
    @(negedge clk);
    $finish;
  end
  always @(posedge clk)
    if ($time > 5)
      $display("T %0t star=%0d star2=%0d comb=%0d list=%0d blk=%0d mixed=%0d inc=%h",
               $time, p1, p5, p2, p3, p4, p6, x);
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate");
    let lines: Vec<&str> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end())
        .filter(|m| m.starts_with("T "))
        .collect();
    assert_eq!(
        lines,
        [
            "T 15 star=21 star2=21 comb=0 list=21 blk=24 mixed=21 inc=27",
            "T 25 star=30 star2=30 comb=21 list=30 blk=33 mixed=30 inc=4a",
            "T 35 star=20 star2=20 comb=20 list=20 blk=22 mixed=20 inc=4a",
        ]
    );
}
