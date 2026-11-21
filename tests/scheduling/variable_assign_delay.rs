//! §10.3.3: sample a variable delay when its driver changes.
use xezim::simulate;

#[test]
fn changing_a_delay_does_not_retime_an_in_flight_transition() {
    let sim = simulate(
        r#"
`timescale 1ns/1ps
module top;
  logic a = 1;
  real dr = 0.0;
  int di = 0;
  wire wr, wi;
  assign #(dr) wr = a;
  assign #(di) wi = a;
  initial begin
    #1 dr = 5.5; di = 5;
    #10 a = 0;
    #1 dr = 0.0; di = 0;
    $display("pending=%b%b", wr, wi);
    #4.25 $display("integer-first=%b%b", wr, wi);
    #0.5 $display("arrived=%b%b", wr, wi);
  end
endmodule"#,
        20000,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(lines, ["pending=11", "integer-first=10", "arrived=00"]);
}

#[test]
fn scoped_variable_delay_selects_rise_fall_and_turnoff() {
    let sim = simulate(
        r#"
`timescale 1ns/1ns
module drv #(parameter BIAS=1)(input logic a, input int d, output wire y);
  assign #(d+BIAS, d+2, d+3) y = a;
endmodule
module top;
  logic a = 0;
  int d = 1;
  wire w;
  drv u(a,d,w);
  initial begin
    #5 a = 1;
    #1 $display("rise-pending=%b",w);
    #2 a = 0;
    #2 $display("fall-pending=%b",w);
    #2 a = 1'bz;
    #3 $display("off-pending=%b",w);
    #2 $display("off=%b",w);
  end
endmodule"#,
        30,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(
        lines,
        ["rise-pending=0", "fall-pending=1", "off-pending=0", "off=z"]
    );
}
