//! §23.10/§7.4.1: inside an instantiated module, a nested select of a packed
//! array whose dimensions use that module's parameters (`mv[1][0]` on
//! `logic [N-1:0][1:0][N-1:0] mv`) has the geometry of THAT instance. The
//! select path evaluated the declaration against the top-level parameters, so
//! the child's `N` was either missing (the element read and wrote one bit) or
//! taken from an unrelated top-level `N`. Expected values from the reference
//! simulator.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_line(msgs: &[String], want: &str) {
    assert!(
        msgs.iter().any(|m| m == want),
        "missing {want:?} in {msgs:?}"
    );
}

#[test]
fn child_param_packed_elem_through_port() {
    let msgs = messages(
        "module arb #(parameter int unsigned N = 1) (input logic [N-1:0] valid_i, output logic any_o);
  assign any_o = |valid_i;
  initial #1 $display(\"%m bits=%0d valid_i=%b any_o=%b\", $bits(valid_i), valid_i, any_o);
endmodule
module router #(parameter int unsigned N = 0) ();
  logic [N-1:0][1:0][N-1:0] mv;
  logic [N-1:0][0:0][N-1:0] mu;
  logic [N-1:0] o, p;
  initial begin
    mv = '0; mv[0][0] = 5'h10; mv[1][0] = 5'h03;
    mu = '0; mu[0][0] = 5'h10; mu[1][0] = 5'h03;
  end
  arb #(.N(N)) v0 (.valid_i(mv[0][0]), .any_o(o[0]));
  arb #(.N(N)) v1 (.valid_i(mv[1][0]), .any_o(o[1]));
  arb #(.N(N)) u0 (.valid_i(mu[0][0]), .any_o(p[0]));
  arb #(.N(N)) u1 (.valid_i(mu[1][0]), .any_o(p[1]));
endmodule
module tb;
  router #(.N(5)) i_dut ();
endmodule",
    );
    assert_line(&msgs, "tb.i_dut.v0 bits=5 valid_i=10000 any_o=1");
    assert_line(&msgs, "tb.i_dut.v1 bits=5 valid_i=00011 any_o=1");
    assert_line(&msgs, "tb.i_dut.u0 bits=5 valid_i=10000 any_o=1");
    assert_line(&msgs, "tb.i_dut.u1 bits=5 valid_i=00011 any_o=1");
}

#[test]
fn child_param_packed_elem_read_write_assign() {
    let msgs = messages(
        "module router #(parameter int unsigned N = 0) ();
  logic [N-1:0][1:0][N-1:0] mv;
  logic [N-1:0] cpy;
  assign cpy = mv[1][0];
  initial begin
    mv = '0;
    mv[1][0] = 5'h03;
    mv[2][1] = 5'h15;
    mv[2][1][4] = 1'b0;
    #1 $display(\"bits=%0d mv[1][0]=%b mv[2][1]=%b cpy=%b\", $bits(mv[1][0]), mv[1][0], mv[2][1], cpy);
       $display(\"whole=%h\", mv);
  end
endmodule
module tb;
  router #(.N(5)) i_dut ();
  initial #2 $display(\"hier bits=%0d i_dut.mv[2][1]=%b\", $bits(i_dut.mv[2][1]), i_dut.mv[2][1]);
endmodule",
    );
    assert_line(&msgs, "bits=5 mv[1][0]=00011 mv[2][1]=00101 cpy=00011");
    assert_line(&msgs, "whole=000000a000c00");
    assert_line(&msgs, "hier bits=5 i_dut.mv[2][1]=00101");
}

#[test]
fn child_param_packed_elem_ignores_same_named_top_param() {
    let msgs = messages(
        "module child;
  typedef logic [79:0] vec_t;
  localparam int N = 4;
  vec_t [N-1:0][4:0] v2;
  logic [N-1:0][4:0][N-1:0] v3;
  initial begin
    v2 = '0; v3 = '0;
    v2[1][2] = 80'hface_0000_0000_0000_0001;
    v3[3][1] = 4'ha;
    #1 $display(\"typedef bits=%0d v2[1][2]=%h\", $bits(v2[1][2]), v2[1][2]);
       $display(\"plain bits=%0d v3[3][1]=%b\", $bits(v3[3][1]), v3[3][1]);
  end
endmodule
module tb;
  localparam int N = 99;
  child u ();
endmodule",
    );
    assert_line(&msgs, "typedef bits=80 v2[1][2]=face0000000000000001");
    assert_line(&msgs, "plain bits=4 v3[3][1]=1010");
}
