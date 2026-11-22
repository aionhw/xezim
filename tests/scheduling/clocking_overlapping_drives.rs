//! IEEE 1800-2023 §14.16.2: "When more than one synchronous drive on the same
//! clocking block output (or inout) is scheduled to mature in the same Re-NBA
//! region of the same time step, the last value is the only value driven onto
//! the output signal." Each bit takes the value of the last drive that
//! selects it, and an output whose bits end where they started does not
//! change, so nothing waiting on it wakes.
//!
//! Only drives of exactly the same bits replaced each other. Drives that
//! overlap (`cb.e[1:0] <= 2'b01; cb.e[0] <= 0;`) or that mix a select with a
//! whole drive of the output (`cb.b[0] <= 1; cb.b <= 0;`) were applied one
//! after the other, so the output changed and changed back in that step and
//! woke `@(e)` with its value unchanged.

fn lines(source: &str) -> Vec<String> {
    let sim = xezim::simulate(source, 1_000_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|entry| entry.message.clone())
        .filter(|line| line.starts_with("T|"))
        .collect()
}

const SRC: &str = r#"
`timescale 1ns/1ps
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [3:0] e = 0, f = 0, g = 0, a = 4'b0001, b = 0, c = 0, d = 0;
  default clocking cb @(posedge clk);
    default output SKEW;
    output e, f, g, a, b, c, d;
  endclocking
  initial begin
    @(cb);
    cb.e[1:0] <= 2'b01;   cb.e[0] <= 1'b0;
    cb.f[0] <= 1'b1;      cb.f[1:0] <= 2'b00;
    cb.g[3:0] <= 4'b0110; cb.g[2:1] <= 2'b00;
    cb.a <= 4'b0000;      cb.a[0] <= 1'b1;
    cb.b[0] <= 1'b1;      cb.b <= 4'b0000;
    cb.c[1:0] <= 2'b11;   cb.c[3:1] <= 3'b000;  cb.c[0] <= 1'b0;
    cb.d[2:0] <= 3'b111;  cb.d[1] <= 1'b0;
    @(cb); @(cb);
    $display("T|end e=%b f=%b g=%b a=%b b=%b c=%b d=%b", e, f, g, a, b, c, d);
    #1 $finish;
  end
  always @(e) $display("T|e=%b", e);
  always @(f) $display("T|f=%b", f);
  always @(g) $display("T|g=%b", g);
  always @(a) $display("T|a=%b", a);
  always @(b) $display("T|b=%b", b);
  always @(c) $display("T|c=%b", c);
  always @(d) $display("T|d=%b", d);
endmodule
"#;

const EXPECTED: [&str; 2] = [
    "T|d=0101",
    "T|end e=0000 f=0000 g=0000 a=0001 b=0000 c=0000 d=0101",
];

#[test]
fn overlapping_drives_with_an_output_skew_change_each_bit_once() {
    assert_eq!(lines(&SRC.replace("SKEW", "#2")), EXPECTED);
}

#[test]
fn overlapping_drives_with_no_output_skew_change_each_bit_once() {
    assert_eq!(lines(&SRC.replace("SKEW", "#0")), EXPECTED);
}
