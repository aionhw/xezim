//! IEEE 1800-2023 §3.14.2.3, §20.3, §5.8: `$time`, `$stime`, `$realtime`
//! and time literals on the right of a continuous assignment are in the unit
//! of the module that declares the assignment, whatever process changed its
//! operands. Expected lines were checked on the reference simulator.

fn check(source: &str, expected: &[&str]) {
    let sim = xezim::simulate(source, 100).expect("simulate");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(lines, expected);
}

#[test]
fn continuous_assignment_reads_time_in_its_module_unit() {
    // The parent runs in 1 fs; the child's 1 ps unit rounds 499 fs to 0 and
    // 2100 fs to 2.
    check(
        r#"
module child(input logic a);
  timeunit 1ps; timeprecision 1fs;
  wire [63:0] t;
  wire [31:0] st;
  wire [63:0] tn = a ? $time : 0;
  real ra;
  wire [63:0] tx;
  real rt;
  real rl;
  assign t = a ? $time : 0;
  assign st = a ? $stime : 0;
  assign ra = a ? $realtime : 0.0;
  assign tx = a ? ($time + 64'd2) : 0;
  assign rl = a ? 5ns : 1.5ns;
  always @(a) rt = $realtime;
  initial #1 $display("T|t=%0d st=%0d tn=%0d ra=%f tx=%0d rt=%f rl=%f", t, st, tn, ra, tx, rt, rl);
  initial #3 $display("T|t=%0d st=%0d tn=%0d ra=%f tx=%0d", t, st, tn, ra, tx);
endmodule
module top;
  timeunit 1fs; timeprecision 1fs;
  logic a = 0;
  child c(a);
  initial begin #499 a = 1; #1600 a = 0; #1 a = 1; #10000 $finish; end
endmodule
"#,
        &[
            "T|t=0 st=0 tn=0 ra=0.499000 tx=2 rt=0.499000 rl=5000.000000",
            "T|t=2 st=2 tn=2 ra=2.100000 tx=4",
        ],
    );
}
