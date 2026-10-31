//! §6.20.3/§20.6.2: a header parameter whose default reads a type parameter
//! of the same header (`parameter int H = $bits(T)`) takes THIS instance's
//! type, default or overridden, and so does a declaration sized by it. It
//! read the previous instance's type (0 for the first one), and a module with
//! no type parameters read the previous instance's type for a design typedef
//! of the same name. A header parameter of a struct type parameter packs by
//! the struct's fields. §27.4: a genvar in a type override's dimensions takes
//! the iteration's value. Expected values from the reference simulator.
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
package pk;
  typedef struct packed { logic [2:0] a; logic [4:0] b; } s_t;
endpackage
typedef logic [11:0] T;
module other #(parameter int Q = $bits(T)) ();
  logic [Q-1:0] w;
  initial $display("%m Q=%0d w=%0d", Q, $bits(w));
endmodule
module sl #(parameter type T = pk::s_t, parameter T P = '{a:5, b:17}, parameter int PA = P.a) ();
  initial $display("%m PA=%0d P=%0h", PA, P);
endmodule
module leaf #(parameter type T = logic [3:0], parameter int H = $bits(T),
              parameter int H3 = H + 1) ();
  localparam int B = $bits(T);
  logic [H-1:0] v;
  initial $display("%m H=%0d H3=%0d B=%0d v=%0d", H, H3, B, $bits(v));
endmodule
interface ifc #(parameter type T = logic [4:0], parameter int H = $bits(T)) ();
  logic [H-1:0] d;
  initial $display("%m H=%0d d=%0d", H, $bits(d));
endinterface
module mid #(parameter type T = logic [2:0], parameter int H = $bits(T)) ();
  leaf #(.T(T)) u_l ();
  initial $display("%m H=%0d", H);
endmodule
module top;
  typedef logic [6:0] t7;
  other o0 ();
  leaf u_def ();
  other o1 ();
  leaf #(.T(logic [9:0])) u_ovr ();
  other o2 ();
  sl s0 ();
  leaf #(.T(t7)) u_td ();
  leaf #(.T(logic [9:0]), .H(3)) u_h ();
  ifc i0 ();
  ifc #(.T(shortint)) i1 ();
  mid m0 ();
  mid #(.T(logic [5:0])) m6 ();
  for (genvar g = 1; g <= 2; g++) begin : gl
    leaf #(.T(logic [g*2-1:0])) u ();
  end
endmodule
"#;

#[test]
fn header_default_reads_this_instances_type_parameter() {
    let msgs = messages(SRC);
    assert_line(&msgs, "top.u_def H=4 H3=5 B=4 v=4");
    assert_line(&msgs, "top.u_ovr H=10 H3=11 B=10 v=10");
    assert_line(&msgs, "top.u_td H=7 H3=8 B=7 v=7");
    assert_line(&msgs, "top.u_h H=3 H3=4 B=10 v=3");
    assert_line(&msgs, "top.i0 H=5 d=5");
    assert_line(&msgs, "top.i1 H=16 d=16");
    assert_line(&msgs, "top.m0 H=3");
    assert_line(&msgs, "top.m0.u_l H=3 H3=4 B=3 v=3");
    assert_line(&msgs, "top.m6 H=6");
    assert_line(&msgs, "top.m6.u_l H=6 H3=7 B=6 v=6");
    assert_line(&msgs, "top.gl[1].u H=2 H3=3 B=2 v=2");
    assert_line(&msgs, "top.gl[2].u H=4 H3=5 B=4 v=4");
    for o in ["o0", "o1", "o2"] {
        assert_line(&msgs, &format!("top.{o} Q=12 w=12"));
    }
    assert_line(&msgs, "top.s0 PA=5 P=b1");
}
