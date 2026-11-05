//! IEEE 1800-2023 §23.10: instances of one module share a prepared body only
//! when everything its elaboration reads is the same: the values of its
//! parameters, wide ones in full, and the types its type parameters bind.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).expect("simulate");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// §6.20.3: a generate condition on `$bits(T)` follows each instance's `T`.
#[test]
fn type_parameter_selects_generate_branch() {
    let out = lines(
        r#"
module leaf #(parameter type T = logic [7:0]) (output int o);
  if ($bits(T) > 8) begin : g_wide
    assign o = 100 + $bits(T);
  end else begin : g_narrow
    assign o = $bits(T);
  end
endmodule
module mid #(parameter type T = logic [7:0]) (output int o);
  leaf #(.T(T)) u(.o(o));
endmodule
module tb;
  int a, b, c, d;
  leaf #(.T(logic [3:0])) u1(.o(a));
  leaf #(.T(logic [15:0])) u2(.o(b));
  mid #(.T(logic [3:0])) m1(.o(c));
  mid #(.T(logic [31:0])) m2(.o(d));
  initial #1 $display("%0d %0d %0d %0d", a, b, c, d);
endmodule
"#,
    );
    assert!(out.iter().any(|l| l == "4 116 4 132"), "{out:?}");
}

/// §6.20.2, §11.5.1: a parameter wider than 64 bits that differs only in
/// its upper bits.
#[test]
fn wide_parameter_upper_bits_select_generate_branch() {
    let out = lines(
        r#"
module leaf #(parameter logic [127:0] P = 0) (output int o);
  if (P[100]) begin : g_hi
    assign o = 1;
  end else begin : g_lo
    assign o = 0;
  end
endmodule
module tb;
  int a, b;
  leaf #(.P(128'h0)) u1(.o(a));
  leaf #(.P(128'h1 << 100)) u2(.o(b));
  initial #1 $display("%0d %0d", a, b);
endmodule
"#,
    );
    assert!(out.iter().any(|l| l == "0 1"), "{out:?}");
}

/// §6.20.3, §20.7: equal-width type parameters of different shapes, bound
/// from parent parameters.
#[test]
fn type_parameter_shape_selects_generate_branch() {
    let out = lines(
        r#"
module leaf #(parameter type T = logic [7:0]) (output int o);
  if ($dimensions(T) > 1) begin : g2
    assign o = 10 + $size(T, 1);
  end else begin : g1
    assign o = $size(T, 1);
  end
endmodule
module mid #(parameter int W = 2, parameter int H = 4) (output int o);
  leaf #(.T(logic [W-1:0][H-1:0])) u(.o(o));
endmodule
module tb;
  int a, b, c;
  mid #(.W(2), .H(4)) m1(.o(a));
  mid #(.W(4), .H(2)) m2(.o(b));
  leaf #(.T(logic [7:0])) u(.o(c));
  initial #1 $display("%0d %0d %0d", a, b, c);
endmodule
"#,
    );
    assert!(out.iter().any(|l| l == "12 14 8"), "{out:?}");
}

/// §26.3: a package-qualified parameter is the package's, not an instance
/// parameter of the same name.
#[test]
fn package_parameter_beside_same_named_parameter() {
    let out = lines(
        r#"
package p;
  parameter int W = 4;
endpackage
module leaf #(parameter int W = 8) (output int o);
  if (p::W == 4) begin : g
    assign o = 1;
  end else begin : h
    assign o = 0;
  end
endmodule
module tb;
  int a, b;
  leaf u(.o(a));
  leaf #(.W(4)) v(.o(b));
  initial #1 $display("%0d %0d", a, b);
endmodule
"#,
    );
    assert!(out.iter().any(|l| l == "1 1"), "{out:?}");
}
