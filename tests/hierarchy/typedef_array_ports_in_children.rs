//! GitHub #106: a port or child-local variable whose UNPACKED dimensions come
//! from a TYPEDEF (`typedef t_byte t_word [0:3];`) lost those dimensions
//! during module inlining — only declarator dims were consulted — so the
//! array registered as a scalar, connections silently carried nothing, and
//! under `default_nettype none` the actual was reported as an implicit net.
//! Reference-validated end to end.

use xezim::simulate;

fn msgs(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn typedef_array_port_passes_data_through_child() {
    let src = r#"
typedef logic [7:0] t_byte;
typedef t_byte t_word [0:3];
module leaf(input t_word din, output t_word o);
  assign o = din;
endmodule
module top;
  t_word src, dst;
  leaf u(.din(src), .o(dst));
  initial begin
    src[0]=8'h11; src[1]=8'h22; src[2]=8'h33; src[3]=8'h44;
    #1 $display("T|%h %h %h %h", dst[0], dst[1], dst[2], dst[3]);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|11 22 33 44"),
        "got {:?}",
        msgs(&sim)
    );
}

#[test]
fn issue_repro_with_internal_var_and_nettype_none() {
    // The reporter's exact shape: wire typedef-array port, intermediate
    // child-local typedef variable, `default_nettype none`. Must elaborate
    // (no false implicit-net) AND move the data.
    let src = r#"
`default_nettype none
typedef logic [7:0] t_byte;
typedef t_byte t_word [0:3];
module pass(input wire t_word din, output t_word o);
  t_word s;
  assign s = din;
  assign o = s;
endmodule
module top;
  t_word src, dst;
  pass u(.din(src), .o(dst));
  initial begin
    src[0]=8'h11; src[1]=8'h22; src[2]=8'h33; src[3]=8'h44;
    #1 $display("T|%h %h %h %h", dst[0], dst[1], dst[2], dst[3]);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|11 22 33 44"),
        "got {:?}",
        msgs(&sim)
    );
}

#[test]
fn genuine_implicit_net_still_rejected_under_none() {
    let src = r#"
`default_nettype none
module top(input wire logic [7:0] a, output logic [7:0] o);
  assign o = a ^ ghost;
endmodule
"#;
    let err = match simulate(src, 10) {
        Ok(_) => panic!("undeclared 'ghost' must still error under `default_nettype none"),
        Err(e) => e,
    };
    assert!(err.contains("ghost"), "diagnostic should name the net, got: {err}");
}
