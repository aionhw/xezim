//! IEEE 1800-2017 §10.8 / §10.9.2: an assignment pattern passed straight to
//! an unpacked-struct formal: `f('{a: 1, b: 2})`.
//!
//! The member-wise binding only took a flat POSITIONAL pattern whose item
//! count matched the leaves. A `name:` item, `default:`, or a nested member
//! pattern fell through to reading `.a` off the pattern itself, so every
//! member arrived as 0. This held for module subroutines, class methods, and
//! a parameterized class's type-parameter formal alike.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("sim");
    sim.output
        .iter()
        .filter(|o| o.message.starts_with("T "))
        .map(|o| o.message.clone())
        .collect()
}

const S: &str = "\
typedef struct { int x; bit [3:0] y; } in_t;\n\
typedef struct { int a; bit b; } flat_t;\n\
typedef real r_t; typedef struct { r_t a; } real_t;\n\
typedef struct { int a; bit b; in_t n; } nest_t;\n";

#[test]
fn named_and_default_patterns_bind_a_flat_struct_formal() {
    let src = format!(
        "{S}\
class plain; function int f(flat_t s); return s.a * 10 + s.b; endfunction endclass\n\
module t;\n\
  function automatic flat_t id(flat_t s); return s; endfunction\n\
  function automatic real rf(real_t s); return s.a; endfunction\n\
  initial begin\n\
    plain p = new();\n\
    flat_t r;\n\
    r = id('{{a: 2, b: 1}});      $display(\"T named %0d %0d\", r.a, r.b);\n\
    r = id('{{b: 1, a: 3}});      $display(\"T reordered %0d %0d\", r.a, r.b);\n\
    r = id('{{default: 0, a: 4}}); $display(\"T default %0d %0d\", r.a, r.b);\n\
    r = id(flat_t'{{a: 5, b: 1}}); $display(\"T typed %0d %0d\", r.a, r.b);\n\
    $display(\"T method %0d\", p.f('{{a: 6.4, b: 1}}));\n\
    $display(\"T real %0.1f\", rf('{{a: 1.6}}));\n\
  end\n\
endmodule"
    );
    assert_eq!(
        lines(&src),
        vec![
            "T named 2 1",
            "T reordered 3 1",
            "T default 4 0",
            "T typed 5 1",
            "T method 61",
            "T real 1.6"
        ]
    );
}

#[test]
fn nested_patterns_bind_a_struct_formal_with_a_struct_member() {
    let src = format!(
        "{S}\
class box #(type T = int);\n\
  function void put(T t); $display(\"T param %0d %0d %0d %h\", t.a, t.b, t.n.x, t.n.y); endfunction\n\
endclass\n\
module t;\n\
  function automatic void show(nest_t s); $display(\"T fn %0d %0d %0d %h\", s.a, s.b, s.n.x, s.n.y); endfunction\n\
  initial begin\n\
    box #(nest_t) bx = new();\n\
    show('{{a: 1, b: 1, n: '{{x: 2, y: 4'h3}}}});\n\
    show('{{4, 0, '{{5, 4'h6}}}});\n\
    bx.put('{{b: 1, a: 7, n: '{{default: 8}}}});\n\
    bx.put('{{default: 0, n: '{{y: 4'h9, x: 9}}}});\n\
  end\n\
endmodule"
    );
    assert_eq!(
        lines(&src),
        vec![
            "T fn 1 1 2 3",
            "T fn 4 0 5 6",
            "T param 7 1 8 8",
            "T param 0 0 9 9"
        ]
    );
}
