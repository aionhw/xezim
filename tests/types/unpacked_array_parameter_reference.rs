//! IEEE 1800-2017 §6.20.2, §23.10: an unpacked-array parameter whose value is
//! the name of another unpacked-array parameter takes that array's elements.
//! A default `= pk::A` and an override `.R(R)` or `.R(L)` that forwards the
//! parent's header parameter or body localparam used to read 0 in every element.

use xezim::simulate;

const SRC: &str = r#"
package pk;
  parameter logic [33:0] A [2] = '{34'h1, 34'h2};
endpackage
module leaf #(parameter logic [33:0] R [2] = pk::A);
  initial $display("%m %h %h", R[0], R[1]);
endmodule
module mid #(parameter logic [33:0] R [2] = '{34'h7, 34'h7});
  localparam logic [33:0] L [2] = '{34'h3, 34'h4};
  leaf #(.R(R)) u_header();
  leaf #(.R(L)) u_body();
endmodule
module top;
  leaf u_default();
  mid #(.R('{34'h5, 34'h6})) u_mid();
endmodule
"#;

#[test]
fn unpacked_array_parameter_reference() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    let mut msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    msgs.sort();
    assert_eq!(
        msgs,
        [
            "top.u_default 000000001 000000002",
            "top.u_mid.u_body 000000003 000000004",
            "top.u_mid.u_header 000000005 000000006",
        ],
    );
}

/// §6.20.2: named real-array parameters retain their element values and type.
#[test]
fn named_real_array_parameter_reference() {
    let sim = simulate(
        r#"
package reals;
  parameter real A[2] = '{1.5, 2.5};
endpackage
module leaf #(parameter real A[2] = reals::A);
  initial $display("%m %0.1f %0.1f", A[0], A[1]);
endmodule
module mid #(parameter real A[2] = reals::A);
  leaf #(.A(A)) v();
endmodule
module top; leaf u(); mid w(); endmodule
"#,
        100,
    )
    .expect("simulate");
    let mut msgs: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    msgs.sort();
    assert_eq!(msgs, ["top.u 1.5 2.5", "top.w.v 1.5 2.5"]);
}
