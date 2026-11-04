//! IEEE 1800-2017 §20.6.2 / §11.6.1 / §7.4.5: `$bits` in a constant
//! expression of a compound operand has its self-determined width: an
//! operator, a conditional, a concatenation or replication holding a cast
//! or a signal, and a function call. An element or slice of a declared
//! variable has the width of the dimensions its selects leave. Each was 0,
//! or one bit (`$bits(pk[2])` of `logic [3:0][1:0] pk`), so a declaration
//! sized by it was one bit wide. Expected values from the reference
//! simulator.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_line(msgs: &[String], want: &str) {
    assert!(msgs.iter().any(|m| m == want), "missing {want:?} in {msgs:?}");
}

#[test]
fn bits_of_compound_operands() {
    let msgs = messages(
        "typedef logic [3:0] q_t;
function automatic logic [5:0] f(int x); return x; endfunction
module t #(parameter int P = 5);
  logic [6:0] sig;
  logic [2:0] arr [4];
  localparam int A = $bits(q_t'(1) + 1);
  localparam int B = $bits({q_t'(1), byte'(2)});
  localparam int D = $bits(f(1));
  localparam int E = $bits(1 ? q_t'(1) : byte'(2));
  localparam int F = $bits(q_t'(1) == 1);
  localparam int G = $bits({2{q_t'(1)}});
  localparam int H = $bits({sig, arr[1]});
  localparam int I = $bits(-q_t'(3));
  localparam int J = $bits(P + 4'd1);
  logic [$bits({q_t'(1), 2'b0})-1:0] v;
  initial $display(\"A=%0d B=%0d D=%0d E=%0d F=%0d G=%0d H=%0d I=%0d J=%0d v=%0d\", A, B, D, E, F, G, H, I, J, $bits(v));
endmodule
",
    );
    assert_line(&msgs, "A=32 B=12 D=6 E=8 F=1 G=8 H=10 I=4 J=32 v=6");
}

#[test]
fn bits_of_selects_of_declared_variables() {
    let msgs = messages(
        "module t;
  logic [3:0][1:0] pk;
  logic [7:0] mem [2][3];
  logic [5:0][2:0] m2 [4];
  localparam int A = $bits(pk[2]);
  localparam int B = $bits(pk[2][1]);
  localparam int C = $bits(mem[1]);
  localparam int D = $bits(m2[1][3]);
  localparam int E = $bits(pk[3:2]);
  localparam int F = $bits(pk[1 +: 3]);
  localparam int G = $bits(m2[1][4:2]);
  logic [$bits(m2[0][1])-1:0] v;
  initial $display(\"A=%0d B=%0d C=%0d D=%0d E=%0d F=%0d G=%0d v=%0d\", A, B, C, D, E, F, G, $bits(v));
endmodule
",
    );
    assert_line(&msgs, "A=2 B=1 C=24 D=3 E=4 F=6 G=9 v=3");
}
