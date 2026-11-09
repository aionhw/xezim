//! #954 review: §8.23, §8.25: a static method needs no object and is not
//! virtual. Through a null handle of a parameterized class it runs in the
//! handle's declared specialization (`P #(8) p8; p8.width()` is 8), and
//! through a live handle declared as a base class it runs the base class's
//! method even when the object's class declares a same-named static.
//! Expected values come from the reference simulator.

use xezim::simulate;

#[test]
fn static_calls_bind_to_the_declared_class_and_specialization() {
    let src = r#"
class B;
  static int calls = 0;
  static function int sf(int x); calls++; return x * 2; endfunction
endclass
class D extends B;
  static function int sf(int x); return x * 3; endfunction
endclass
class P #(int W = 4);
  static function int width(); return W; endfunction
endclass
class U;
  P #(8) p8;
  P #(16) p16;
  P p4;
endclass
module top;
  B bh; D dh; U u;
  initial begin
    u = new;
    $display("null spec: p8=%0d p16=%0d p4=%0d", u.p8.width(), u.p16.width(), u.p4.width());
    dh = new; bh = dh;
    $display("live: B-typed=%0d D-typed=%0d calls=%0d", bh.sf(5), dh.sf(5), B::calls);
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_eq!(
        out,
        [
            "null spec: p8=8 p16=16 p4=4",
            "live: B-typed=10 D-typed=15 calls=1",
        ]
    );
}
