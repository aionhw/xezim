//! §6.24.1: a cast to a typedef in a constant expression takes the typedef's
//! width and signing, at top level and inside an instance, so a localparam,
//! a declared width, a generate condition or a header parameter default
//! computed from it has the cast value. Expected values from the reference
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
fn typedef_casts_in_top_level_constants() {
    let msgs = messages(
        "package pk;
  typedef logic [4:0] p_t;
  typedef enum logic [2:0] {K0, K1, K2} k_e;
  localparam int PL = p_t'(45);
endpackage
typedef logic [3:0] q_t;
typedef q_t q2_t;
typedef enum logic [1:0] {E0, E1, E2, E3} e_t;
typedef logic signed [3:0] s_t;
typedef bit [0:5] a_t;
typedef int unsigned u_t;
typedef struct packed { logic [2:0] a; logic [1:0] b; } st_t;
typedef real r_t;
module t;
  import pk::*;
  localparam int A = q_t'(45);
  localparam int A2 = q2_t'(45);
  localparam int D = 32'(e_t'(6));
  localparam int E = int'(q_t'(45)) + 1;
  localparam int S = s_t'(12) < 0;
  localparam int SV = s_t'(12);
  localparam int AS = a_t'(70);
  localparam int U = u_t'(-1) > 0;
  localparam int ST = st_t'(45);
  localparam int P = p_t'(45);
  localparam int PQ = pk::p_t'(45);
  localparam int K = 32'(k_e'(13));
  localparam int N = q_t'(-1);
  localparam real R = r_t'(3);
  localparam logic [7:0] L8 = q_t'(8'hff);
  logic [q_t'(45)-1:0] w;
  logic [7:0] g;
  if (q_t'(45) == 13) begin : g_ok assign g = 1; end else begin : g_bad assign g = 0; end
  initial #2 $display(\"A=%0d A2=%0d D=%0d E=%0d S=%0d SV=%0d AS=%0d U=%0d ST=%0d P=%0d PQ=%0d K=%0d N=%0d R=%0.1f L8=%0d PL=%0d bits=%0d g=%0d\",
    A, A2, D, E, S, SV, AS, U, ST, P, PQ, K, N, R, L8, pk::PL, $bits(w), g);
endmodule
",
    );
    assert_line(
        &msgs,
        "A=13 A2=13 D=2 E=14 S=1 SV=-4 AS=6 U=1 ST=13 P=13 PQ=13 K=5 N=15 R=3.0 L8=15 PL=13 bits=13 g=1",
    );
}

#[test]
fn typedef_casts_in_instance_constants() {
    let msgs = messages(
        "package pk;
  typedef logic [4:0] p_t;
endpackage
typedef logic [3:0] q_t;
module child #(parameter int W = 6, parameter int H = q_t'(45)) ();
  typedef logic [W-1:0] w_t;
  localparam int CW = w_t'(100);
  localparam int CQ = q_t'(45);
  localparam int CP = pk::p_t'(45);
  logic [w_t'(100)-1:0] cv;
  initial #1 $display(\"%m H=%0d CW=%0d CQ=%0d CP=%0d bits=%0d\", H, CW, CQ, CP, $bits(cv));
endmodule
module t;
  child c ();
  child #(.W(8)) c8 ();
endmodule
",
    );
    assert_line(&msgs, "t.c H=13 CW=36 CQ=13 CP=13 bits=36");
    assert_line(&msgs, "t.c8 H=13 CW=100 CQ=13 CP=13 bits=100");
}

#[test]
fn instance_typedef_shadows_and_type_parameter_casts() {
    let msgs = messages(
        "typedef logic signed [7:0] w_t;
module child #(parameter type T = logic [3:0], parameter int H = T'(45), parameter int W = 5) ();
  typedef logic [W-1:0] w_t;
  localparam int CW = w_t'(-1);
  localparam int NEG = w_t'(-1) < 0;
  logic [w_t'(100)-1:0] v;
  genvar i;
  for (i = 0; i < w_t'(35); i++) begin : gl end
  initial #1 $display(\"%m H=%0d CW=%0d NEG=%0d bits=%0d gen=%0d\", H, CW, NEG, $bits(v), w_t'(35));
endmodule
module t;
  localparam int TW = w_t'(200);
  localparam int TN = w_t'(200) < 0;
  int r;
  initial begin
    case (w_t'(258)) 2: r = 1; default: r = 0; endcase
    #2 $display(\"TW=%0d TN=%0d case=%0d\", TW, TN, r);
  end
  child c ();
  child #(.T(logic [2:0])) c3 ();
endmodule
",
    );
    assert_line(&msgs, "TW=-56 TN=1 case=1");
    assert_line(&msgs, "t.c H=13 CW=31 NEG=0 bits=4 gen=3");
    assert_line(&msgs, "t.c3 H=5 CW=31 NEG=0 bits=4 gen=3");
}

#[test]
fn typedef_casts_in_ports_overrides_and_generate_case() {
    let msgs = messages(
        "typedef logic [3:0] q_t;
typedef logic signed [3:0] s_t;
typedef struct packed { logic [q_t'(18)-1:0] a; logic b; } ps_t;
function automatic int f(int x); return x + 1; endfunction
module child #(parameter int H = 0) (input logic [q_t'(45)-1:0] p);
  initial #1 $display(\"%m H=%0d pbits=%0d\", H, $bits(p));
endmodule
module t;
  localparam int N = 3;
  typedef logic [N-1:0] n_t;
  localparam int Z = n_t'(13);
  localparam q_t X = q_t'(45);
  localparam int FA = f(q_t'(45));
  localparam int NE = q_t'(q_t'(45) + 20);
  localparam int PS = $bits(ps_t);
  localparam int SC = s_t'(9) + 1;
  logic [12:0] pw;
  logic [7:0] gc;
  case (q_t'(45))
    13: begin : g13 assign gc = 13; end
    default: begin : gd assign gc = 0; end
  endcase
  logic [7:0] gi;
  if (s_t'(9) < 0) begin : gn assign gi = 1; end else begin : gp assign gi = 0; end
  child #(.H(q_t'(45))) c (.p(pw));
  initial #2 $display(\"Z=%0d X=%0d FA=%0d NE=%0d PS=%0d SC=%0d gc=%0d gi=%0d\", Z, X, FA, NE, PS, SC, gc, gi);
endmodule
",
    );
    assert_line(&msgs, "t.c H=13 pbits=13");
    assert_line(&msgs, "Z=5 X=13 FA=14 NE=1 PS=3 SC=-6 gc=13 gi=1");
}

#[test]
fn for_generate_bound_from_instance_typedef_cast() {
    let msgs = messages(
        "typedef logic signed [7:0] w_t;
module child #(parameter int W = 5) ();
  typedef logic [W-1:0] w_t;
  genvar i;
  for (i = 0; i < w_t'(35); i++) begin : gl
    initial $display(\"it %0d\", i);
  end
endmodule
module t;
  child c ();
endmodule
",
    );
    let iterations = msgs.iter().filter(|m| m.starts_with("it ")).count();
    assert_eq!(iterations, 3, "{msgs:?}");
}
