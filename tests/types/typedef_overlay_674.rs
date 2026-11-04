//! IEEE 1800-2023 §6.18 / §23.9: local typedef widths remain instance-local.

#[test]
fn instance_width_snapshots_preserve_local_and_package_types() {
    let sim = xezim::simulate(
        r#"
package p; typedef logic[12:0] shared_t; endpackage
module leaf #(parameter int W=4, parameter type T=logic[W-1:0]);
  typedef logic[W-1:0] shared_t;
  localparam int A=$bits(shared_t), B=$bits(logic[W-1:0]);
  localparam int C=$bits(T), D=$bits(p::shared_t);
  typedef logic[B-1:0] dependent_t;
  initial $display("%m %0d %0d %0d %0d %0d", A,B,C,D,$bits(dependent_t));
endmodule
module other;
  import p::*;
  initial $display("%m %0d",$bits(shared_t));
endmodule
module top;
  leaf #(.W(3)) u0(); leaf #(.W(7)) u1();
  leaf #(.W(5),.T(logic[8:0])) u2(); other v();
endmodule
"#,
        100,
    )
    .expect("simulate");
    let mut lines: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    lines.sort();
    assert_eq!(
        lines,
        [
            "top.u0 3 3 3 13 3",
            "top.u1 7 7 7 13 7",
            "top.u2 5 5 9 13 5",
            "top.v 13"
        ]
    );
}
