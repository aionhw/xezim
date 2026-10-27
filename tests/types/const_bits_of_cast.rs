//! §20.6.2/§6.24.1: `$bits` of a cast in a constant expression is the width
//! of the cast's target, in localparams, declared and port widths, a child's
//! own typedef and a generate condition. Expected values from the reference
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

const SRC: &str = r#"
typedef logic [3:0] q_t;
typedef enum logic [2:0] {E0, E1} e_t;
typedef struct packed { logic [2:0] a; logic [1:0] b; } st_t;
typedef real r_t;
package pk; typedef logic [5:0] p_t; endpackage
module c #(parameter int W = 6) (input logic [$bits(q_t'(1))-1:0] pi);
  typedef logic [W-1:0] w_t;
  localparam int CW = $bits(w_t'(100));
  localparam int CS = $bits(W'(100));
  initial $display("C W=%0d CW=%0d CS=%0d port=%0d", W, CW, CS, $bits(pi));
endmodule
module t;
  localparam int N = 7;
  localparam int A  = $bits(q_t'(45));
  localparam int B  = $bits(int'(45));
  localparam int C  = $bits(5'(45));
  localparam int D  = $bits(N'(3));
  localparam int E  = $bits(e_t'(1));
  localparam int F  = $bits(st_t'(45));
  localparam int G  = $bits(pk::p_t'(1));
  localparam int H  = $bits(byte'(300));
  localparam int I  = $bits(unsigned'(8'd3));
  localparam int J  = $bits(longint'(1)) + $bits(shortint'(1));
  localparam int K  = $bits(r_t'(3));
  logic [$bits(int'(0))-1:0] v;
  c #(.W(9)) u (.pi('0));
  initial begin
    if ($bits(q_t'(1)) == 4) $display("gen-ok");
    $display("A=%0d B=%0d C=%0d D=%0d E=%0d F=%0d G=%0d H=%0d I=%0d J=%0d K=%0d v=%0d", A, B, C, D, E, F, G, H, I, J, K, $bits(v));
    $display("RT %0d %0d %0d %0d", $bits(q_t'(45)), $bits(int'(45)), $bits(5'(45)), $bits(N'(3)));
  end
endmodule
"#;

#[test]
fn bits_of_cast_in_constants() {
    let msgs = messages(SRC);
    assert_line(&msgs, "A=4 B=32 C=5 D=7 E=3 F=5 G=6 H=8 I=8 J=80 K=64 v=32");
    assert_line(&msgs, "C W=9 CW=9 CS=9 port=4");
    assert_line(&msgs, "gen-ok");
    assert_line(&msgs, "RT 4 32 5 7");
}
