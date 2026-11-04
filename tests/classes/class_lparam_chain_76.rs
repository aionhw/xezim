//! IEEE 1800-2023 §8.25 / §6.20: a class constant inherited through a
//! parameterized `extends Base#(...)` takes the derived specialization's
//! arguments. `Derived#(10)::LP` read the base's default (43) and a chain
//! through two parameterized bases read 0 (xezim-core #76, reported by
//! opensource-elearning).

use xezim::simulate;

#[test]
fn inherited_constants_follow_the_extends_arguments() {
    let src = r#"
class Base#(int P = 42);
  localparam int LP = P + 1;
endclass
class Derived#(int Q = 5) extends Base#(Q);
  localparam int LD = Q * 2;
endclass
class A#(int P);
  localparam int LPA = P;
endclass
class B#(int Q) extends A#(Q + 1);
  localparam int LPB = Q + LPA;
endclass
class C#(int R) extends B#(R);
endclass
module top;
  Derived#(10) d;
  C#(5) c;
  localparam int X = Derived#(10)::LP;
  localparam int Y = C#(5)::LPB;
  logic [X-1:0] vx;
  logic [Y-1:0] vy;
  initial $display("S|%0d %0d %0d %0d %0d %0d %0d",
    Derived#(10)::LP, Base#(10)::LP, Derived#(10)::LD, B#(11)::LPA, C#(5)::LPA, C#(5)::LPB, Derived#(3)::LP);
  initial #1 $display("H|%0d %0d %0d", d.LP, c.LPA, c.LPB);
  initial #2 $display("E|%0d %0d", $bits(vx), $bits(vy));
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    let msgs: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(msgs.contains(&"S|11 11 20 12 6 11 4"), "{msgs:?}");
    assert!(msgs.contains(&"H|11 6 11"), "{msgs:?}");
    assert!(msgs.contains(&"E|11 11"), "{msgs:?}");
}
