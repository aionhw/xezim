//! §10.9 / §13.4.1: an assignment pattern returned directly from a function
//! or method whose return type is an UNPACKED struct must be distributed
//! member by member, exactly as the same pattern assigned to a struct
//! variable is. It must not be flattened into one packed value.
//!
//! The packed-pattern plan declines an unpacked struct (its layout builder
//! only accepts packed aggregates), so the return fell through to the
//! generic expression evaluator, which treats `'{...}` as a plain
//! concatenation of the item values. The caller then scattered that flat
//! vector across the struct's leaves by their packed-transport offsets —
//! where a `string` leaf's nominal width is 1024 bits (§6.16 strings have
//! no packed width) — so every member ahead of the last read zeros and the
//! last member received the whole concatenated byte stream:
//! `return '{ "one", "two" }` came back as `a=""`, `b="onetwo"`. Pure
//! fixed-width structs survived only because the concatenation happened to
//! align with the layout. (Found via SVUnit: its `svunit_filter` builds
//! its default filter as `return '{ parts[0], "" }` for a two-string
//! struct, so the always-match pattern landed in the negative subfilter
//! and no test body ever ran.)

use xezim::simulate;

fn lines(sim: &xezim::compiler::Simulator) -> Vec<&str> {
    sim.output.iter().map(|o| o.message.as_str()).collect()
}

#[test]
fn unpacked_struct_pattern_return_distributes_members() {
    let src = r#"
module test;
  typedef struct { string a; string b; } ss_t;
  typedef struct { int a; string b; } is_t;
  typedef struct { string a; int b; } si_t;
  typedef struct { ss_t inner; string tail; } nest_t;
  typedef struct { int arr[2]; string tag; } arrs_t;

  class C;
    function ss_t ret_ss(); return '{ "one", "two" }; endfunction
    function is_t ret_is(); return '{ 7, "seven" }; endfunction
    function si_t ret_si(); return '{ "hi", 42 }; endfunction
    function ss_t ret_keyed(); return '{ a: "one", b: "two" }; endfunction
    function nest_t ret_nest(); return '{ '{ "x", "y" }, "z" }; endfunction
    function ss_t ret_cond(int c);
      return c ? '{ "one", "two" } : '{ "three", "four" };
    endfunction
    function nest_t ret_nest_call(); return '{ ret_ss(), "z" }; endfunction
    function ss_t ret_mixed(int c);
      return c ? '{ "pat_a", "pat_b" } : ret_ss();
    endfunction
    function ss_t ret_def();
      return '{ a: "only_a", default: "fallback" };
    endfunction
    function arrs_t ret_arrs(); return '{ '{ 10, 20 }, "tagged" }; endfunction
    function ss_t ret_arg(string x); return '{ x, "tail" }; endfunction
    function ss_t ret_filter();
      string parts[1];
      parts[0] = "flt";
      return '{ parts[0], "" };
    endfunction
    function ss_t ret_typed(); return ss_t'{ "ta", "tb" }; endfunction
    function is_t ret_rec(int depth);
      return (depth <= 0) ? '{ 0, "base" }
                          : '{ ret_rec(depth - 1).a + 1, "step" };
    endfunction
  endclass

  function automatic ss_t free_ret_ss();
    return '{ "one", "two" };
  endfunction

  initial begin
    C c = new;
    ss_t s;
    is_t m;
    si_t n;
    nest_t g;
    arrs_t a;
    s = c.ret_ss();
    $display("M|%s|%s", s.a, s.b);
    m = c.ret_is();
    $display("I|%0d|%s", m.a, m.b);
    n = c.ret_si();
    $display("S|%s|%0d", n.a, n.b);
    s = c.ret_keyed();
    $display("K|%s|%s", s.a, s.b);
    g = c.ret_nest();
    $display("N|%s|%s|%s", g.inner.a, g.inner.b, g.tail);
    s = c.ret_cond(1);
    $display("T|%s|%s", s.a, s.b);
    s = c.ret_cond(0);
    $display("F|%s|%s", s.a, s.b);
    s = free_ret_ss();
    $display("R|%s|%s", s.a, s.b);
    g = c.ret_nest_call();
    $display("NC|%s|%s|%s", g.inner.a, g.inner.b, g.tail);
    s = c.ret_mixed(1);
    $display("MP|%s|%s", s.a, s.b);
    s = c.ret_mixed(0);
    $display("MC|%s|%s", s.a, s.b);
    s = c.ret_def();
    $display("KD|%s|%s", s.a, s.b);
    a = c.ret_arrs();
    $display("AM|%0d|%0d|%s", a.arr[0], a.arr[1], a.tag);
    s = c.ret_arg("hello");
    $display("AR|%s|%s", s.a, s.b);
    s = c.ret_filter();
    $display("FL|%s|%s", s.a, s.b);
    s = c.ret_typed();
    $display("TP|%s|%s", s.a, s.b);
    m = c.ret_rec(2);
    $display("RC|%0d|%s", m.a, m.b);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("runs");
    let got = lines(&sim);
    for want in [
        "M|one|two",
        "I|7|seven",
        "S|hi|42",
        "K|one|two",
        "N|x|y|z",
        "T|one|two",
        "F|three|four",
        "R|one|two",
        "NC|one|two|z",
        "MP|pat_a|pat_b",
        "MC|one|two",
        "KD|only_a|fallback",
        "AM|10|20|tagged",
        "AR|hello|tail",
        "FL|flt|",
        "TP|ta|tb",
        "RC|2|step",
    ] {
        assert!(
            got.iter().any(|l| *l == want),
            "missing line {want:?} in {got:?}"
        );
    }
}
