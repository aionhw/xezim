//! IEEE 1800-2023 §6.11.2, §23.3.3.2, §10.3: a 2-state scalar target drops X
//! and Z on every write, whether it is a `bit` input formal or a `bit`
//! variable driven by a continuous assignment, and whatever the gate-like
//! shape of the right side. Expected lines were checked on the reference
//! simulator.

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
fn two_state_scalar_targets_drop_x_of_gate_shaped_drivers() {
    check(
        r#"
module tscalar(input bit b); initial #1 $display("T|%m b=%b", b); endmodule
module top;
  logic x1 = 1'bx, y1 = 1'b1, s = 1'bx;
  bit b1, b2, b3, b4;
  assign b1 = x1;
  assign b2 = ~x1;
  assign b3 = x1 & y1;
  assign b4 = s ? x1 : y1;
  tscalar k_plain(x1);
  tscalar k_not(~x1);
  tscalar k_and(x1 & y1);
  tscalar k_mux(s ? x1 : y1);
  initial #1 $display("T|assigns %b %b %b %b", b1, b2, b3, b4);
endmodule
"#,
        &[
            "T|top.k_plain b=0",
            "T|top.k_not b=0",
            "T|top.k_and b=0",
            "T|top.k_mux b=0",
            "T|assigns 0 0 0 0",
        ],
    );
}
