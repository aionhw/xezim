//! §7.2.2: a structure can be assigned as a whole. A whole-struct copy into
//! or out of an unpacked-struct class property keeps every leaf of a member
//! that is itself an unpacked struct, or an array of them, whether the
//! source is a module variable, a subroutine formal or another property.

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
typedef struct { in_t i; int z; } mid_t;\n\
typedef struct { int a; in_t arr[2]; mid_t m; } s_t;\n";

#[test]
fn nested_members_survive_copies_into_and_out_of_a_property() {
    let src = format!(
        "{S}\
class c; s_t p; s_t q; endclass\n\
module t;\n\
  initial begin\n\
    c o = new();\n\
    s_t s, r;\n\
    s.a = 1; s.arr[0] = '{{2, 3}}; s.arr[1] = '{{4, 5}}; s.m = '{{'{{6, 7}}, 8}};\n\
    o.p = s;\n\
    $display(\"T write %0d %0d %0d %0d %0d %0d %0d %0d\", o.p.a, o.p.arr[0].x, o.p.arr[0].y,\n\
             o.p.arr[1].x, o.p.arr[1].y, o.p.m.i.x, o.p.m.i.y, o.p.m.z);\n\
    r = o.p;\n\
    $display(\"T read %0d %0d %0d %0d %0d %0d %0d %0d\", r.a, r.arr[0].x, r.arr[0].y,\n\
             r.arr[1].x, r.arr[1].y, r.m.i.x, r.m.i.y, r.m.z);\n\
    o.q = o.p;\n\
    $display(\"T prop %0d %0d %0d\", o.q.arr[1].x, o.q.m.i.y, o.q.m.z);\n\
    o.p.m.i.x = 9;\n\
    r = (s.a == 1 ? o.p : o.q);\n\
    $display(\"T cond %0d %0d\", r.m.i.x, r.arr[0].y);\n\
  end\n\
endmodule"
    );
    assert_eq!(
        lines(&src),
        vec![
            "T write 1 2 3 4 5 6 7 8",
            "T read 1 2 3 4 5 6 7 8",
            "T prop 4 7 8",
            "T cond 9 3"
        ]
    );
}

#[test]
fn a_struct_formal_with_a_nested_member_copies_into_a_property() {
    let src = "\
typedef struct { int x; bit [3:0] y; } in_t;\n\
typedef struct { int a; bit b; in_t n; } s_t;\n\
class box #(type T = int);\n\
  T last;\n\
  function void put(T t); last = t; endfunction\n\
endclass\n\
class plain;\n\
  s_t last;\n\
  function void put(s_t t); last = t; endfunction\n\
endclass\n\
module t;\n\
  initial begin\n\
    box #(s_t) bx = new();\n\
    plain p = new();\n\
    s_t s;\n\
    s = '{a: 4, b: 1, n: '{x: 7, y: 4'h7}};\n\
    bx.put(s);\n\
    $display(\"T param %0d %0d %0d %h\", bx.last.a, bx.last.b, bx.last.n.x, bx.last.n.y);\n\
    p.put(s);\n\
    $display(\"T plain %0d %0d %0d %h\", p.last.a, p.last.b, p.last.n.x, p.last.n.y);\n\
  end\n\
endmodule";
    assert_eq!(
        lines(src),
        vec!["T param 4 1 7 7", "T plain 4 1 7 7"]
    );
}
