//! IEEE 1800-2023 §10.9.1, §10.10, §7.6, §8.4: follow-ups to assignment
//! patterns on rows of multi-dimensional unpacked arrays (#999). Expected
//! values are from the reference simulator.

use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_lines(msgs: &[String], want: &[&str]) {
    for w in want {
        assert!(
            msgs.iter().any(|m| m.trim() == *w),
            "missing {w:?} in {msgs:#?}"
        );
    }
}

/// §8.4: in a method, `c[1] = '{7, 8}` names the property `c` of `this`,
/// not a same-named module array.
#[test]
fn row_pattern_in_a_method_stores_the_property() {
    let msgs = messages(
        r#"module t;
  class C;
    int c [2][2];
    function void f();
      c[1] = '{7, 8};
      c[0] = '{default: 3};
    endfunction
  endclass
  int c [2][2];
  initial begin
    C o;
    o = new;
    o.f();
    $display("cls=%0d %0d %0d %0d", o.c[0][0], o.c[0][1], o.c[1][0], o.c[1][1]);
    $display("mod=%0d %0d %0d %0d", c[0][0], c[0][1], c[1][0], c[1][1]);
  end
endmodule"#,
    );
    assert_lines(&msgs, &["cls=3 3 7 8", "mod=0 0 0 0"]);
}

/// §10.9.1, §7.2: a row of an unpacked array of unpacked or packed structs
/// takes the pattern element by element, at a constant or run-time row
/// index, with positional, keyed and `default:` items.
#[test]
fn row_patterns_of_struct_elements() {
    let msgs = messages(
        r#"typedef struct { int a; int b; } s_t;
typedef struct packed { logic [31:0] pc; logic [7:0] m; } le_t;
module t;
  s_t s [2][2];
  s_t r [3][2];
  s_t n [2][2][2];
  le_t p [2][2];
  int k;
  initial begin
    k = 2;
    s[1] = '{'{a: 1, b: 2}, '{a: 3, b: 4}};
    r[k] = '{default: '{a: 5, b: 6}};
    n[1][0] = '{'{b: 8, a: 7}, '{a: 9, b: 10}};
    p[1] = '{40'hdead0000a5, 40'h1122334455};
    #1;
    $display("s=%0d %0d %0d %0d s0=%0d", s[1][0].a, s[1][0].b, s[1][1].a, s[1][1].b, s[0][0].a);
    $display("r=%0d %0d %0d %0d", r[2][0].a, r[2][0].b, r[2][1].a, r[2][1].b);
    $display("n=%0d %0d %0d %0d", n[1][0][0].a, n[1][0][0].b, n[1][0][1].a, n[1][0][1].b);
    $display("p=%h %h", p[1][0], p[1][1]);
  end
endmodule"#,
    );
    assert_lines(
        &msgs,
        &[
            "s=1 2 3 4 s0=0",
            "r=5 6 5 6",
            "n=7 8 9 10",
            "p=dead0000a5 1122334455",
        ],
    );
}
