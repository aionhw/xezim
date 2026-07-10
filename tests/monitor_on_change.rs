//! `$monitor` (LRM §21.2.3) prints once when armed, then only when a
//! monitored argument changes — NOT every time step, and `$time` itself must
//! not trigger a reprint. Previously xezim reprinted on any signal change in
//! the design (once per clock edge), producing a flood of duplicate lines.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  logic clk = 0;
  int   cnt = 0;
  logic flag = 0;
  always #5 clk = ~clk;
  always @(posedge clk) begin
    cnt <= cnt + 1;          // changes every cycle — must NOT drive $monitor
    if (cnt == 2) flag <= 1;
    if (cnt == 5) flag <= 0;
    if (cnt == 8) flag <= 1;
  end
  initial $monitor("%0t: flag=%b", $time, flag);   // arg (besides $time) is `flag`
  initial begin repeat (15) @(posedge clk); $finish; end
endmodule
"#;

#[test]
fn monitor_reprints_only_on_arg_change() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    let lines = sim
        .output
        .iter()
        .filter(|o| o.message.contains("flag="))
        .count();
    // `flag` changes 3 times (→1, →0, →1); with the initial arm print that is
    // 4 monitor lines. `cnt` toggles every cycle but must not trigger it.
    assert_eq!(
        lines, 4,
        "expected 4 $monitor lines (arm + 3 flag changes), got {} — \
         it should not reprint on every clock edge",
        lines
    );
}
