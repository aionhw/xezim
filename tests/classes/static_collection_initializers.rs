//! Initializers of static dynamic arrays, queues and associative arrays.
//!
//! A static collection shares one store per declaring class. The store was
//! registered empty and its initializer never ran, so `Cls::set.size()` read
//! 0 and a constraint `v inside {set}` had no solution: `randomize()`
//! returned 0 and left every field at its old value.

use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).expect("simulation failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn expect(out: &[String], line: &str) {
    assert!(
        out.iter().any(|m| m == line),
        "missing {line:?}; output: {out:?}"
    );
}

/// IEEE 1800-2023 §8.9, §6.8: a static property's initializer sets its one
/// copy before any initial procedure runs.
#[test]
fn static_dynamic_array_and_queue_initializers() {
    let src = r#"
module t;
  typedef enum logic [2:0] {A = 3'b000, B = 3'b001, C = 3'b010} e_t;
  class T;
    static e_t s_e[]  = '{A, B, C};
    static int s_i[]  = '{4, 5, 6};
    static bit s_b[]  = '{0, 1};
    static int s_q[$] = {7, 8};
  endclass
  initial begin
    T x = new;
    $display("sizes %0d %0d %0d %0d", T::s_e.size(), T::s_i.size(), T::s_b.size(), T::s_q.size());
    $display("values %s %0d %0d %0d", T::s_e[2].name(), T::s_i[1], T::s_b[1], T::s_q[$]);
    $display("through handle %0d", x.s_i[2]);
  end
endmodule
"#;
    let out = messages(src);
    expect(&out, "sizes 3 3 2 2");
    expect(&out, "values C 5 1 8");
    expect(&out, "through handle 6");
}

/// IEEE 1800-2023 §18.5.5: `inside` over a static array draws from its
/// elements. This is the constraint shape that failed every `randomize()`.
#[test]
fn inside_static_array_constraint_solves() {
    let src = r#"
module t;
  typedef enum logic [2:0] {A = 3'b000, B = 3'b001, C = 3'b010, Z = 3'b111} e_t;
  class T;
    static e_t e_set[] = '{A, B, C};
    static bit m_set[] = '{1};
    rand e_t   v;
    rand logic m;
    constraint c { v inside {e_set}; m inside {m_set}; }
  endclass
  initial begin
    T t = new;
    int ok = 1;
    for (int i = 0; i < 20; i++) begin
      if (!t.randomize() || !(t.v inside {A, B, C}) || t.m !== 1'b1) ok = 0;
    end
    $display("inside ok=%0d", ok);
  end
endmodule
"#;
    expect(&messages(src), "inside ok=1");
}

/// IEEE 1800-2023 §7.9.11, §8.9: an associative-array pattern, with a
/// `default:` item, initializes a static associative array.
#[test]
fn static_associative_array_initializers() {
    let src = r#"
module t;
  class T;
    static int s_s[string] = '{"a": 1, "b": 2};
    static int s_i[int]    = '{3: 30, default: -1};
  endclass
  initial begin
    $display("aa %0d %0d %0d", T::s_s.num(), T::s_s["b"], T::s_i.num());
    $display("keyed %0d default %0d", T::s_i[3], T::s_i[9]);
  end
endmodule
"#;
    let out = messages(src);
    expect(&out, "aa 2 2 1");
    expect(&out, "keyed 30 default -1");
}

/// The same initializers for a class declared in a module instantiated
/// below the top, and for two classes that each declare a static array of
/// the same name (one store per declaring class, §8.9).
#[test]
fn static_collection_initializers_nested_and_per_class() {
    let src = r#"
module tb_mod;
  class P;
    static int set[] = '{1, 2, 3};
  endclass
  class Q;
    static int set[] = '{10, 20};
  endclass
  initial begin
    $display("P %0d %0d", P::set.size(), P::set[2]);
    $display("Q %0d %0d", Q::set.size(), Q::set[1]);
  end
endmodule

module top;
  tb_mod u();
endmodule
"#;
    let out = messages(src);
    expect(&out, "P 3 3");
    expect(&out, "Q 2 20");
}
