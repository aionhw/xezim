//! IEEE 1800-2023 §8.17: constructor argument forwarding. `extends
//! Base(default)` hands a subclass constructor's arguments (implicit or
//! explicit) to the base constructor, `function new(default);` declares the
//! constructor with the base's argument list and forwards it, and
//! `super.new(default)` forwards the formals. Each form was a parse error
//! (core #74, #75, reported by git-hevangel).

use xezim::{simulate, sv_parser};

#[test]
fn constructor_arguments_forward_through_default() {
    let src = r#"
class Base1 #(int VAL = 42);
  int got;
  function new(int a, int b = 7); got = a * 100 + b; endfunction
endclass
class Cls1 extends Base1(default);
endclass
class Cls2 extends Base1;
  int tag;
  function new(default); tag = got + 1; endfunction
endclass
class Cls3 extends Base1(default);
  int tag;
  function new(int a, int b = 9); tag = a; endfunction
endclass
class Cls4 extends Base1;
  function new(int a, int b = 5); super.new(default); endfunction
endclass
class ClsDefFwd;
  function new(default);
  endfunction
endclass
module top;
  Cls1 c1; Cls2 c2; Cls3 c3; Cls4 c4; ClsDefFwd c5;
  initial begin
    c1 = new(1, 2); c2 = new(3); c3 = new(4); c4 = new(5, 6); c5 = new;
    $display("c1=%0d c2=%0d/%0d c3=%0d/%0d c4=%0d c5=%0d",
             c1.got, c2.got, c2.tag, c3.got, c3.tag, c4.got, c5 != null);
  end
endmodule
"#;
    sv_parser::set_sv2023(true);
    let sim = simulate(src, 10).expect("simulate failed");
    let out: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(
        out.iter().any(|m| m.contains("c1=102 c2=307/308 c3=409/4 c4=506 c5=1")),
        "{out:?}"
    );
}
