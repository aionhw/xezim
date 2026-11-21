//! IEEE 1800-2023 §19.5 and §6.24.1 — sampling coverpoints whose
//! expressions start with a real cast.

use xezim::simulate;

#[test]
fn sample_covergroup_with_real_cast_coverpoint() {
    let src = r#"
`timescale 1ns/1ps
module top;
  int x;
  covergroup cg;
    rp: coverpoint real'(x) iff (x > 0) { bins three = {3.0}; }
    above_one: coverpoint real'(x) / 2 > 1 { bins yes = {1}; }
    above_two: coverpoint real'(x) / 2 > 2 { bins yes = {1}; }
    ip: coverpoint int'(x) { bins three = {3}; bins four = {4}; }
  endgroup
  cg c;
  initial begin
    c = new;
    x = 3;
    #1;
    $display("before=%0.1f %0.1f %0.1f",
             c.above_one.get_inst_coverage(), c.above_two.get_inst_coverage(),
             c.ip.get_inst_coverage());
    c.sample();
    #1;
    $display("after=%0.1f %0.1f %0.1f",
             c.above_one.get_inst_coverage(), c.above_two.get_inst_coverage(),
             c.ip.get_inst_coverage());
    $finish;
  end
endmodule
"#;

    let sim = simulate(src, 100_000).expect("covergroup should elaborate and run");
    let lines: Vec<_> = sim.output.iter().map(|o| o.message.trim()).collect();
    // §6.24.1, §11.4.3–§11.4.4: the cast makes the quotient 1.5.
    // §19.8 and §19.11.1: sample() hits 1/1, 0/1 and 1/2 integral bins.
    assert_eq!(lines, ["before=0.0 0.0 0.0", "after=100.0 0.0 50.0"]);
}
