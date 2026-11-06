//! §7.4.2 / §7.8 / §27: `r [N]` declared in a generate block is a fixed
//! array when `N` is a module-level or package parameter, as when it is a
//! sibling of the block; and a parameter type in the block that reads an
//! outer `W` keeps that `W` beside a nested block's own `W`. The array was
//! registered associative as well as fixed. The reference simulator prints
//! the same lines (core #162).
use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).unwrap_or_else(|e| panic!("rejected: {e}"));
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

#[test]
fn array_sized_by_a_module_parameter_and_type_beside_a_nested_parameter() {
    let src = r#"
module top;
  localparam int N = 4;
  localparam int W = 8;
  if (1) begin : g
    logic [31:0] r [N];
    localparam logic [W-1:0] P = 8'hA5;
    if (1) begin : h
      localparam int W = 2;
    end
    initial begin
      for (int i = 0; i < N; i++) r[i] = i * 10;
    end
  end
  initial begin
    #1;
    $display("T|size=%0d r3=%0d P=%0d bitsP=%0d hW=%0d", $size(g.r), g.r[3], g.P, $bits(g.P), g.h.W);
    $finish;
  end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|size=4 r3=30 P=165 bitsP=8 hW=2"]);
}

#[test]
fn arrays_sized_by_a_package_and_a_module_parameter_iterate_every_element() {
    let src = r#"
package p; localparam int N = 3; endpackage
module top;
  import p::*;
  localparam int M = 4;
  if (1) begin : g
    logic [31:0] r [N];
    logic [31:0] s [M];
    int cnt = 0;
    initial begin
      foreach (r[i]) begin r[i] = i + 1; cnt++; end
      foreach (s[i]) begin s[i] = i * 2; cnt++; end
    end
  end
  initial begin #1; $display("T|%0d %0d %0d %0d %0d", $size(g.r), $size(g.s), g.r[2], g.s[3], g.cnt); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|3 4 3 6 7"]);
}
