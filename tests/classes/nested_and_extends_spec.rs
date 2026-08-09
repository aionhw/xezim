//! §8.23 nested class declarations and §8.25 parameterized inheritance with
//! expression extends-args. Reference-validated (audit round I6/I8).

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

#[test]
fn extends_with_param_expression() {
    let src = r#"
class Base #(int W = 4);
  function int w(); return W; endfunction
endclass
class GExt #(int N = 2) extends Base#(N * 3);
endclass
module tb;
  int w;
  initial begin
    GExt#(5) g = new;
    w = g.w();
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "w"), 15, "extends Base#(N*3) evaluates with N bound");
}

#[test]
fn nested_class_instantiation_and_outer_static() {
    let src = r#"
class Outer;
  static int os = 5;
  class Inner;
    int iv = 2;
    function int readOs(); return os; endfunction
  endclass
endclass
module tb;
  int iv, os;
  initial begin
    Outer::Inner x = new;
    iv = x.iv;
    os = x.readOs();
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "iv"), 2, "nested class constructs with its inits");
    assert_eq!(u(&sim, "os"), 5, "inner method sees enclosing class statics");
}
