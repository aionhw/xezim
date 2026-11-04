//! IEEE 1800-2017 §26.3, §7.4.2: an unpacked dimension may name a parameter
//! imported from a package. Inside an instantiated module, an array parameter
//! declared that way (header parameter, override, or body localparam) read 0
//! in every element.

use xezim::simulate;

const SRC: &str = r#"
package pk;
  parameter int unsigned N = 2;
endpackage
module hdr import pk::*; #(parameter logic [7:0] R [N] = '{8'h1, 8'h2});
  initial $display("%m %h %h", R[0], R[1]);
endmodule
module body;
  import pk::*;
  localparam logic [7:0] L [N] = '{8'h3, 8'h4};
  initial $display("%m %h %h", L[0], L[1]);
endmodule
module top;
  hdr u_default();
  hdr #(.R('{8'h5, 8'h6})) u_override();
  body u_body();
endmodule
"#;

#[test]
fn array_parameter_imported_dimension() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    let mut msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    msgs.sort();
    assert_eq!(
        msgs,
        [
            "top.u_body 03 04",
            "top.u_default 01 02",
            "top.u_override 05 06",
        ],
    );
}
