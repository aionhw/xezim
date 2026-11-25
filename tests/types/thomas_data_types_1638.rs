//! IEEE 1800-2023 §6.8, §6.12, §6.19 and §6.24.

fn assert_output(src: &str, expected: &str) {
    let sim = xezim::simulate(src, 100).expect("simulation");
    assert_eq!(sim.output[0].message, expected);
}

#[test]
fn var_accepts_implicit_logic_arrays_and_class_properties() {
    assert_output(
        r#"
module top;
  var a[0:3];
  class C;
    var single_bit;
    var q[$];
    var [3:0] word;
  endclass
  C c;
  initial begin
    var local_arr[2];
    c = new;
    a[3] = 1; c.q.push_back(1); c.word = 12; c.single_bit = 0;
    local_arr[0] = 1;
    $display("var %b %0b %h %b %0b", a[3], c.q[0], c.word, c.single_bit, local_arr[0]);
  end
endmodule
"#,
        "var 1 1 c 0 1",
    );
}

#[test]
fn chandle_member_and_two_state_enum_have_their_type_defaults() {
    assert_output(
        r#"
module top;
  typedef bit [3:0] Base;
  typedef enum Base { Red=3, Blue=4 } E;
  typedef struct { chandle h; } S;
  E e;
  S s;
  initial $display("default %b %b", e, s.h == null);
endmodule
"#,
        "default 0000 1",
    );
}

#[test]
fn constant_real_casts_round_exactly_and_wrap_to_width() {
    assert_output(
        r#"
module top;
  typedef int I;
  localparam A = int'(0.49999999999999994);
  localparam B = longint'(4503599627370497.0);
  localparam C = 64'(1e20);
  localparam D = I'(3.7);
  initial $display("cast %0d %0d %h %0d", A, B, C, D);
endmodule
"#,
        "cast 0 4503599627370497 6bc75e2d63100000 4",
    );
}

/// A static scalar initializer in a class method runs once (§6.21).
#[test]
fn static_method_local_initializer_has_one_side_effect() {
    assert_output(
        r#"
module top;
  class C;
    static int calls;
    static function int seed();
      calls++;
      return 6;
    endfunction
    static function int next();
      static int value = seed();
      return value++;
    endfunction
  endclass
  int a, b;
  initial begin
    a = C::next();
    b = C::next();
    $display("static %0d %0d %0d", a, b, C::calls);
  end
endmodule
"#,
        "static 6 7 1",
    );
}
