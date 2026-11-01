//! §7.2 + §10.4: copying one whole unpacked struct to another in a compiled
//! process.
//!
//! An unpacked struct is stored one signal per member; its container signal
//! only carries the width. The interpreter copies `a = b` member-wise, but a
//! compiled process lowered it to a plain signal copy of the containers, so
//! every member of `a` kept its old value:
//!
//!     assign t = '{x, 7};
//!     always_comb l = t;          // l stayed '{0, 0}
//!
//! `always @(t.a) l = t;` failed the same way, and so did the nonblocking
//! form; the identical statement in an `initial` worked, and an
//! `always_comb` reading a whole struct never re-fired when a member changed.
//! Struct copies now lower member by member, any other whole-struct source (a
//! pattern, a function result) keeps the interpreter's member-wise path, and
//! a comb block reading a whole struct depends on its members, at top level
//! and inside instances.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 1_000_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("R "))
        .collect()
}

#[test]
fn compiled_processes_copy_whole_structs() {
    let out = lines(
        r#"
typedef struct { int a; real r; } in_t;
typedef struct { int a; in_t in; logic [3:0] n; } s_t;
module top;
  int x;
  s_t t, c1, c2, c3, v, w;
  logic clk = 0;
  assign t = '{a: x, in: '{a: x + 1, r: 0.5 * x}, n: 4'(x)};
  always_comb c1 = t;
  always @(t.a) c2 = t;
  always @(posedge clk) c3 <= t;
  initial v = '{a: 9, in: '{a: 8, r: 7.5}, n: 4'd6};
  always_comb w = v;
  initial begin
    // Changed after time 0, so `@(t.a)` is already waiting (§4.7).
    #1 x = 3;
    #1 clk = 1;
    // `@(t.a)` may wake before t's other members settle (§4.7): check only
    // the member it waits on.
    #1 $display("R %0d %0d %.1f %0d | %0d | %0d %.1f %0d | %0d %0d %.1f %0d",
                c1.a, c1.in.a, c1.in.r, c1.n, c2.a,
                c3.a, c3.in.r, c3.n, w.a, w.in.a, w.in.r, w.n);
    x = 5;
    #1 $display("R %0d %0d %.1f %0d | %0d", c1.a, c1.in.a, c1.in.r, c1.n, c2.a);
  end
endmodule
"#,
    );
    assert_eq!(
        out,
        vec!["R 3 4 1.5 3 | 3 | 3 1.5 3 | 9 8 7.5 6", "R 5 6 2.5 5 | 5",]
    );
}

/// Inside an instance a struct variable has member leaves but no container
/// signal; the copy and its `always_comb` sensitivity follow the leaves.
#[test]
fn instance_struct_copies_follow_members() {
    let out = lines(
        r#"
typedef struct { int a; real r; } s_t;
module ch(input int x);
  s_t t, l, n;
  assign t.a = x;
  assign t.r = 0.5 * x;
  always_comb l = t;
  always @(t.a) n <= t;
endmodule
module top;
  int x;
  ch u0(.x(x));
  ch u1(.x(x + 10));
  initial begin
    #1 x = 3;
    #1 $display("R %0d %.1f %0d %.1f %0d", u0.l.a, u0.l.r, u0.n.a, u1.l.r, u1.n.a);
    x = 5;
    #1 $display("R %0d %.1f %0d %.1f %0d", u0.l.a, u0.l.r, u0.n.a, u1.l.r, u1.n.a);
  end
endmodule
"#,
    );
    assert_eq!(out, vec!["R 3 1.5 3 6.5 13", "R 5 2.5 5 7.5 15"]);
}

/// A pattern or a struct with an array member keeps the interpreter path and
/// is still copied correctly.
#[test]
fn other_whole_struct_sources_still_copy() {
    let out = lines(
        r#"
typedef struct { int a; int arr[2]; } a_t;
module top;
  int x;
  a_t s, d1, d2;
  always_comb begin
    s.a = x;
    s.arr[0] = x + 1;
    s.arr[1] = x + 2;
  end
  always_comb d1 = s;
  always_comb d2 = '{a: x * 10, arr: '{1, 2}};
  initial begin
    x = 2;
    #1 $display("R %0d %0d %0d %0d %0d", d1.a, d1.arr[0], d1.arr[1], d2.a, d2.arr[1]);
  end
endmodule
"#,
    );
    assert_eq!(out, vec!["R 2 3 4 20 2"]);
}

/// A struct copy inside an instance or a named block names that scope's own
/// struct, never a same-named struct further out (§23.9): the outer `t` and
/// `l` keep their values.
#[test]
fn inner_struct_copies_leave_outer_structs_alone() {
    let out = lines(
        r#"
typedef struct { int a; } s_t;
module ch(input int x);
  s_t t, l;
  assign t.a = x;
  always_comb l = t;
endmodule
module top;
  s_t t, l;
  int x, y;
  ch u(.x(x));
  always @(x) begin : b
    s_t t, l;
    t.a = x + 1;
    l = t;
    y = l.a;
  end
  initial begin
    t.a = 42;
    l.a = 7;
    #1 x = 3;
    #1 $display("R %0d %0d %0d %0d", t.a, l.a, u.l.a, y);
  end
endmodule
"#,
    );
    assert_eq!(out, vec!["R 42 7 3 4"]);
}
