//! IEEE 1800-2017 §9.2.2.2.1: an `always_comb` (and `@*`) is sensitive to
//! a struct it reads whole even when it writes one member of it (#591), and
//! to a module variable a nested callee reads when the caller has a formal
//! or local of the same name (§13.4, #592). Both reported by akashlevy, with
//! the patches applied here.
use xezim::simulate;

fn shown(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.contains("expect") || m.starts_with("R "))
        .collect()
}

#[test]
fn whole_struct_read_beside_a_member_write() {
    assert_eq!(
        shown(
            r#"module t;
  typedef struct packed { logic [3:0] a; logic [3:0] b; } s_t;
  s_t s;
  s_t out;
  always_comb begin
    out = s;
    s.a = s.a;
  end
  initial begin
    s.b = 4'h0;
    #1 s.b = 4'h5;
    #1 $display("out.b=%h expect 5", out.b);
    $finish;
  end
endmodule

"#
        ),
        ["out.b=5 expect 5"]
    );
}

#[test]
fn nested_callee_read_named_like_a_caller_formal() {
    assert_eq!(
        shown(
            r#"module t;
  logic x, y, out;
  function automatic logic g();
    return x;
  endfunction
  function automatic logic f(input logic x);
    return g();
  endfunction
  always_comb out = f(y);
  initial begin
    x = 0; y = 0;
    #1 x = 1;
    #1 $display("out=%b expect 1", out);
    $finish;
  end
endmodule

"#
        ),
        ["out=1 expect 1"]
    );
}

/// `always_comb` and `@*`, a three-level call chain, and a local (not a
/// formal) named like the module variable.
#[test]
fn siblings() {
    assert_eq!(
        shown(
            r#"module t;
  typedef struct packed { logic [3:0] a; logic [3:0] b; } s_t;
  s_t s, s2, out1, out2;
  logic x, y, o1, o2, o3, o4;
  always_comb begin out1 = s; s.a = s.a; end
  always @* begin out2 = s2; s2.a = s2.a; end
  function automatic logic g(); return x; endfunction
  function automatic logic f(input logic x); return g(); endfunction
  function automatic logic h(input logic x); return f(x); endfunction
  function automatic logic k(input logic q); logic x; x = q; return g(); endfunction
  always_comb o1 = f(y);
  always_comb o2 = f(y);
  always_comb o3 = h(y);
  always_comb o4 = k(y);
  initial begin
    s.b = 4'h0; s2.b = 4'h0; x = 0; y = 0;
    #1 s.b = 4'h5; s2.b = 4'h5; x = 1;
    #1 $display("R %h %h %b %b %b %b", out1.b, out2.b, o1, o2, o3, o4);
    $finish;
  end
endmodule
"#
        ),
        ["R 5 5 1 1 1 1"]
    );
}
