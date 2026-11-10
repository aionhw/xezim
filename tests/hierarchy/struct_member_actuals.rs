//! IEEE 1800-2023 §23.3.3 / §7.2: a member of an unpacked struct is a
//! variable lvalue for an output actual and a value for an input actual.
//! Sample after a delay so connection propagation is independent of §4.7
//! event ordering.

use xezim::simulate;

fn results(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|s| s.starts_with("R "))
        .collect()
}

#[test]
fn output_member_and_scalar_control() {
    let src = r#"
typedef struct { int a; int b; } in_t;
typedef struct { in_t in; int c; } w_t;
module ch(input int x, output in_t o, output int y);
  assign o = '{x, 7};
  assign y = x + 10;
endmodule
module top;
  int x; w_t w;
  ch u(.x(x), .o(w.in), .y(w.c));
  initial begin
    #1 x = 3;
    #1 $display("R %0d %0d %0d", w.in.a, w.in.b, w.c);
    #1 x = 5;
    #1 $display("R %0d %0d %0d", w.in.a, w.in.b, w.c);
  end
endmodule
"#;
    assert_eq!(results(src), ["R 3 7 13", "R 5 7 15"]);
    let nested = format!(
        "{}\nmodule top; wrapper w(); endmodule",
        src.replace("module top;", "module wrapper;")
    );
    assert_eq!(results(&nested), ["R 3 7 13", "R 5 7 15"]);
}

#[test]
fn input_member_reads_formal_and_body() {
    let src = r#"
typedef struct { int a; int b; } in_t;
typedef struct { in_t in; int c; } w_t;
module ch(input in_t i, output int y);
  assign y = i.a + i.b;
endmodule
module top;
  w_t w; int y;
  ch u(.i(w.in), .y(y));
  initial begin
    #1 w.in = '{3, 7};
    #1 $display("R %0d %0d %0d", u.i.a, u.i.b, y);
    #1 w.in = '{5, 9};
    #1 $display("R %0d %0d %0d", u.i.a, u.i.b, y);
  end
endmodule
"#;
    assert_eq!(results(src), ["R 3 7 10", "R 5 9 14"]);
}

#[test]
fn deep_real_members_continuous_and_procedural() {
    let src = r#"
typedef struct { real r; int n; } leaf_t;
typedef struct { leaf_t deep; int unused; } in_t;
typedef struct { in_t in; int c; } w_t;
module cont(input leaf_t i, output leaf_t o);
  assign o = '{r: i.r + 0.25, n: i.n + 1};
endmodule
module proc(input leaf_t i, output leaf_t o);
  always_comb o = '{i.r + 0.125, i.n + 2};
endmodule
module top;
  w_t x, a, b;
  cont u(.i(x.in.deep), .o(a.in.deep));
  proc v(.i(x.in.deep), .o(b.in.deep));
  initial begin
    #1 x.in.deep = '{1.25, 3};
    #1 $display("R %0.3f %0d %0.3f %0d", a.in.deep.r, a.in.deep.n,
                b.in.deep.r, b.in.deep.n);
    #1 x.in.deep = '{4.50, 7};
    #1 $display("R %0.3f %0d %0.3f %0d", a.in.deep.r, a.in.deep.n,
                b.in.deep.r, b.in.deep.n);
  end
endmodule
"#;
    assert_eq!(results(src), ["R 1.500 4 1.375 5", "R 4.750 8 4.625 9"]);
}

#[test]
fn constant_array_element_members() {
    let src = r#"
typedef struct { real r; int n; } in_t;
typedef struct { in_t in; int c; } w_t;
module ch(input in_t i, output in_t o);
  always_comb o = '{i.r + 0.25, i.n + 1};
endmodule
module top;
  localparam int K = 1;
  w_t x[2], arr[2];
  ch u(.i(x[K].in), .o(arr[K].in));
  initial begin
    #1 x[1].in.r = 1.25; x[1].in.n = 3;
    #1 $display("R %0.2f %0d %0.2f %0d %0.2f %0d", u.i.r, u.i.n,
                arr[1].in.r, arr[1].in.n, arr[0].in.r, arr[0].in.n);
    #1 x[1].in.r = 4.50; x[1].in.n = 7;
    #1 $display("R %0.2f %0d", arr[1].in.r, arr[1].in.n);
  end
endmodule
"#;
    assert_eq!(results(src), ["R 1.25 3 1.50 4 0.00 0", "R 4.75 8"]);
    let nested = format!(
        "{}\nmodule top; wrapper w(); endmodule",
        src.replace("module top;", "module wrapper;")
    );
    assert_eq!(results(&nested), ["R 1.25 3 1.50 4 0.00 0", "R 4.75 8"]);
}

#[test]
fn children_write_distinct_members() {
    let src = r#"
typedef struct { int a; int b; } in_t;
typedef struct { in_t left; in_t right; int c; } w_t;
module ch(input int x, output in_t o);
  always_comb begin o.a = x; o.b = x + 7; end
endmodule
module top;
  int x, y; w_t w;
  ch a(.x(x), .o(w.left));
  ch b(.x(y), .o(w.right));
  initial begin
    #1 w.c = 99; x = 3; y = 10;
    #1 $display("R %0d %0d %0d %0d %0d", w.left.a, w.left.b,
                w.right.a, w.right.b, w.c);
    #1 x = 5;
    #1 $display("R %0d %0d %0d %0d %0d", w.left.a, w.left.b,
                w.right.a, w.right.b, w.c);
  end
endmodule
"#;
    assert_eq!(results(src), ["R 3 10 10 17 99", "R 5 12 10 17 99"]);
}

/// §13.4 / §23.3.3: a function returning the output value is called once,
/// and its local struct shadows the parent's identically named object.
#[test]
fn function_result_and_local_shadowing() {
    let src = r#"
typedef struct { int a; int b; } in_t;
typedef struct { in_t in; int c; } w_t;
module ch(input int x, output in_t o);
  int calls = 0;
  function automatic in_t make(input int v);
    in_t w;
    calls++;
    w.a = v;
    w.b = v + 7;
    return w;
  endfunction
  always_comb o = make(x);
endmodule
module top;
  int x, before_calls; w_t w;
  ch u(.x(x), .o(w.in));
  initial begin
    #1 before_calls = u.calls; x = 3;
    #1 $display("R %0d %0d %0d", w.in.a, w.in.b, u.calls-before_calls);
  end
endmodule
"#;
    assert_eq!(results(src), ["R 3 10 1"]);
}
