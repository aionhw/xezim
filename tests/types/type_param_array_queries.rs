//! §20.7: the array query functions over an instance's type parameter
//! answer from the bound type's declared dimensions, default or overridden,
//! in constants and at run time: `$left(T)` of `logic [7:2]` is 7. They
//! answered from the width alone in constants (`[5:0]`, no dimension) and
//! read 0 at run time. Expected values from the reference simulator.
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

const SRC: &str = r#"
typedef logic [0:5] asc_t;
typedef logic [3:0][7:0] p2_t;
typedef logic [3:0] ua_t [0:2];
module leaf #(parameter int N = 1, parameter type T = logic [7:2]) ();
  localparam int L = $left(T), R = $right(T), SZ = $size(T), INC = $increment(T), DIM = $dimensions(T);
  logic [$left(T):$right(T)] w;
  function automatic int fl(); return $left(T) * 100 + $right(T); endfunction
  wire [31:0] ca = $high(T) * 10 + $low(T);
  if ($left(T) == 7) begin : g7
    initial $display("%m");
  end
  initial #1 $display("%m L=%0d R=%0d SZ=%0d INC=%0d DIM=%0d w=%0d wl=%0d fl=%0d ca=%0d rt=%0d/%0d/%0d",
                      L, R, SZ, INC, DIM, $bits(w), $left(w), fl(), ca, $left(T), $right(T), $dimensions(T));
endmodule
interface ifc #(parameter type T = logic [9:3]) ();
  localparam int L = $left(T);
  initial #1 $display("%m L=%0d rt=%0d/%0d", L, $left(T), $right(T));
endinterface
module mid #(parameter type T = logic [6:1]) ();
  leaf #(.T(T)) u_pass ();
  initial #1 $display("%m rt=%0d/%0d", $left(T), $right(T));
endmodule
module top;
  localparam int N = 12;
  leaf u_def ();
  leaf #(.T(logic [0:5])) u_asc ();
  leaf #(.T(asc_t)) u_td ();
  leaf #(.T(p2_t)) u_p2 ();
  leaf #(.T(ua_t)) u_ua ();
  leaf #(.N(3), .T(logic [N-1:0])) u_scope ();
  ifc i0 ();
  ifc #(.T(logic [0:4])) i1 ();
  mid m1 ();
endmodule
"#;

#[test]
fn array_queries_over_a_type_parameter() {
    let msgs = messages(SRC);
    assert_line(&msgs, "top.u_def L=7 R=2 SZ=6 INC=1 DIM=1 w=6 wl=7 fl=702 ca=72 rt=7/2/1");
    assert_line(&msgs, "top.u_def.g7");
    assert_line(&msgs, "top.u_asc L=0 R=5 SZ=6 INC=-1 DIM=1 w=6 wl=0 fl=5 ca=50 rt=0/5/1");
    assert_line(&msgs, "top.u_td L=0 R=5 SZ=6 INC=-1 DIM=1 w=6 wl=0 fl=5 ca=50 rt=0/5/1");
    assert_line(&msgs, "top.u_p2 L=3 R=0 SZ=4 INC=1 DIM=2 w=4 wl=3 fl=300 ca=30 rt=3/0/2");
    assert_line(&msgs, "top.u_ua L=0 R=2 SZ=3 INC=-1 DIM=2 w=3 wl=0 fl=2 ca=20 rt=0/2/2");
    assert_line(&msgs, "top.u_scope L=11 R=0 SZ=12 INC=1 DIM=1 w=12 wl=11 fl=1100 ca=110 rt=11/0/1");
    assert_line(&msgs, "top.i0 L=9 rt=9/3");
    assert_line(&msgs, "top.i1 L=0 rt=0/4");
    assert_line(&msgs, "top.m1 rt=6/1");
    assert_line(&msgs, "top.m1.u_pass L=6 R=1 SZ=6 INC=1 DIM=1 w=6 wl=6 fl=601 ca=61 rt=6/1/1");
}
