//! §6.20 / §23.10 — a packed-struct parameter forwarded as another module's
//! parameter override keeps its fields.
//!
//! `child #(.Features(FPU_FEATURES))` bound the value but the child's header
//! localparam `WIDTH = Features.Width` was folded before the struct's field
//! layout was registered, so it read a 32-bit 0 and the port `[WIDTH-1:0]`
//! elaborated to 2 bits. The same hop from a localparam lost the field too.
//! Verilator 5.052 prints 32 on both sides.

use xezim::simulate;

fn displays(src: &str) -> Vec<String> {
    simulate(src, 10)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

const PKG: &str = r#"
package p;
  typedef struct packed {
    int unsigned Width;
    logic        Flag;
  } features_t;
  parameter features_t F = '{Width: 32, Flag: 1'b1};
endpackage

module child #(
  parameter p::features_t Features = p::F,
  localparam int unsigned WIDTH = Features.Width
) (
  output logic [WIDTH-1:0] result_o
);
  initial $display("child WIDTH=%0d bits=%0d", WIDTH, $bits(result_o));
endmodule
"#;

#[test]
fn struct_parameter_override_keeps_leading_field() {
    let src = format!(
        r#"
{PKG}
module parent #(
  parameter p::features_t FPU_FEATURES = p::F
);
  logic [31:0] result;
  child #(.Features(FPU_FEATURES)) u (.result_o(result));
  initial $display("parent Width=%0d", FPU_FEATURES.Width);
endmodule
module t;
  parent u ();
endmodule
"#
    );
    let out = displays(&src);
    assert!(out.iter().any(|l| l == "child WIDTH=32 bits=32"), "{out:?}");
    assert!(out.iter().any(|l| l == "parent Width=32"), "{out:?}");
}

#[test]
fn struct_localparam_override_keeps_leading_field() {
    let src = format!(
        r#"
{PKG}
module parent;
  localparam p::features_t FPU_FEATURES = p::F;
  logic [31:0] result;
  child #(.Features(FPU_FEATURES)) u (.result_o(result));
endmodule
module t;
  parent u ();
endmodule
"#
    );
    let out = displays(&src);
    assert!(out.iter().any(|l| l == "child WIDTH=32 bits=32"), "{out:?}");
}
