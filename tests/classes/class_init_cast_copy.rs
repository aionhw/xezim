//! §8 class semantics: $cast task-form failure leaves dest null, `new this`
//! copy construction, queue-property `{...}` initializer, static `= new`
//! construction, and typedef-alias specialization args.
//! All reference-validated (audit round I5/I7-I11).

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

#[test]
fn cast_task_form_fails_incompatible() {
    let src = r#"
class Base; endclass
class D1 extends Base; int y = 1; endclass
class D2 extends Base; int z = 2; endclass
module tb;
  int is_null;
  initial begin
    Base b;
    D1 d1 = new;
    D2 d2;
    b = d1;
    $cast(d2, b); // incompatible: runtime error, d2 stays null
    is_null = (d2 == null);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "is_null"), 1, "$cast task form must not assign on failure");
}

#[test]
fn new_this_copies_current_object() {
    let src = r#"
class C;
  int x = 1;
  function C clone(); C c = new this; return c; endfunction
endclass
module tb;
  int bx;
  initial begin
    C a = new, b;
    a.x = 10;
    b = a.clone();
    bx = b.x;
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "bx"), 10, "new this shallow-copies, not construct-fresh");
}

#[test]
fn queue_property_concat_initializer() {
    let src = r#"
class C;
  int q[$] = {1, 2, 3};
endclass
module tb;
  int sz, q1;
  initial begin
    C c = new;
    sz = c.q.size();
    q1 = c.q[1];
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "sz"), 3, "concat-form queue initializer populates");
    assert_eq!(u(&sim, "q1"), 2);
}

#[test]
fn static_class_property_new_initializer() {
    let src = r#"
class C;
  static C inst = new;
  int v = 7;
endclass
module tb;
  int is_null, iv;
  initial begin
    is_null = (C::inst == null);
    iv = C::inst.v;
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "is_null"), 0, "static = new constructs at startup");
    assert_eq!(u(&sim, "iv"), 7);
}

#[test]
fn typedef_alias_specialization_args() {
    // NOTE: `$bits` of a method-local declared as a bound TYPE parameter is a
    // separate open gap (returns the default type's width); only the value
    // parameter binding is asserted here.
    let src = r#"
class Box #(type T = int, int W = 4);
  function int width(); return W; endfunction
endclass
typedef Box#(byte, 8) ByteBox;
module tb;
  int w;
  initial begin
    ByteBox bb = new;
    w = bb.width();
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "w"), 8, "typedef alias carries its #() args to new");
}
