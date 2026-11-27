//! IEEE 1800-2023 §6.6.7, §6.19.4, §6.24.1 and §7.6 (#1636, #1655).

fn lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 100_000)
        .unwrap()
        .output
        .iter()
        .map(|line| line.message.trim().to_string())
        .filter(|line| line.starts_with("R "))
        .collect()
}

#[test]
fn enum_cast_in_constant_function_sizes_the_addition() {
    let src = r#"
module top;
  typedef enum int { ZERO, ONE } e32_t;
  typedef int word_t;
  function automatic int from_real(real r);
    return int'(e32_t'(r));
  endfunction
  function automatic int from_sum(bit [2:0] a, bit [2:0] b);
    return int'(e32_t'(a + b));
  endfunction
  localparam int P = from_real(0.49999999999999994);
  localparam int S = from_sum(7, 6);
  localparam int W = word_t'(0.4) * 3;
  initial $display("R %0d %0d %0d", P, S, W);
endmodule
"#;
    assert_eq!(lines(src), ["R 0 13 0"]);
}

#[test]
fn lazy_nettype_array_drivers_follow_port_element_order() {
    let src = r#"`timescale 1ns/1ps
package p;
  function automatic real fold(input real d[]);
    fold = 10.0;
    foreach (d[i]) fold += d[i];
  endfunction
  nettype real N with fold;
endpackage
module source import p::*; (output N o[4:3]);
  assign o = '{1.0, 2.0};
endmodule
module top;
  import p::*;
  N a[1:2];
  source s(a);
  initial begin
    #1 $display("R %g %g", a[1], a[2]);
    $finish;
  end
endmodule
"#;
    assert_eq!(lines(src), ["R 11 12"]);
}
