//! IEEE 1800-2023 §9.4.5, §10.9: a blocking assignment with an
//! intra-assignment delay evaluates its right-hand side when the statement
//! runs and stores the value after the delay. An assignment pattern on the
//! right-hand side (`wa = #1 '{3, 4};`) stored nothing: the pattern was
//! evaluated with no target type to shape it. The stored values were
//! checked on the reference simulator; the times follow §9.4.5.
use xezim::simulate;

fn run(src: &str) -> Vec<String> {
    let result = simulate(src, 100_000).unwrap();
    result
        .output
        .iter()
        .map(|entry| entry.message.clone())
        .filter(|l| l.starts_with("R "))
        .collect()
}

#[test]
fn whole_arrays_rows_and_defaults() {
    let lines = run(r#"
module t;
  int wa [2];
  int wb [2][2];
  logic [7:0] wp [3];
  int wd [2];
  initial begin
    wa = #1 '{3, 4};
    wb[1] = #1 '{5, 6};
    wb[0] = #1 '{default: 9};
    wp = #1 '{8'h1, 8'h2, 8'h3};
    wd = #1 '{1: 6, default: 2};
    $display("R t=%0t wa=%0d %0d wb=%0d %0d %0d %0d wp=%0d %0d %0d wd=%0d %0d", $time,
             wa[0], wa[1], wb[0][0], wb[0][1], wb[1][0], wb[1][1], wp[0], wp[1], wp[2], wd[0], wd[1]);
  end
endmodule
"#);
    assert_eq!(lines, ["R t=5 wa=3 4 wb=9 9 5 6 wp=1 2 3 wd=2 6"]);
}

#[test]
fn structs_queues_and_nested_patterns() {
    let lines = run(r#"
module t;
  typedef struct { int x; int y; } s_t;
  s_t s, sn;
  int q [$];
  int m [2][2];
  initial begin
    s = #1 '{10, 11};
    sn = #1 '{y: 13, x: 12};
    q = #1 '{7, 8, 9};
    m = #1 '{'{1, 2}, '{3, 4}};
    $display("R s=%0d %0d sn=%0d %0d q=%0d %0d %0d m=%0d %0d %0d %0d", s.x, s.y, sn.x, sn.y,
             q.size(), q[0], q[2], m[0][0], m[0][1], m[1][0], m[1][1]);
  end
endmodule
"#);
    assert_eq!(lines, ["R s=10 11 sn=12 13 q=3 7 9 m=1 2 3 4"]);
}

/// §9.4.5: the right-hand side is evaluated before the delay, so a change
/// to an operand during the delay does not reach the stored value; the
/// stored elements take the target's element type.
#[test]
fn operands_are_sampled_before_the_delay() {
    let lines = run(r#"
module t;
  int a = 1, b = 2;
  logic [3:0] w [2];
  int wa [2];
  initial begin
    fork
      wa = #2 '{a, b + 1};
      #1 begin a = 50; b = 60; end
    join
    w = #1 '{a + 6, 5'h13};
    $display("R t=%0t wa=%0d %0d w=%0d %0d", $time, wa[0], wa[1], w[0], w[1]);
  end
endmodule
"#);
    assert_eq!(lines, ["R t=3 wa=1 3 w=8 3"]);
}

/// Guards: a whole-array right-hand side and a delayed nonblocking pattern
/// were already right.
#[test]
fn plain_array_and_nonblocking_guard() {
    let lines = run(r#"
module t;
  int wc [2];
  int wn [2];
  initial begin
    wc = '{1, 2};
    wc = #1 wc;
    wn <= #1 '{3, 4};
    #1;
    $display("R wc=%0d %0d wn=%0d %0d", wc[0], wc[1], wn[0], wn[1]);
  end
endmodule
"#);
    assert_eq!(lines, ["R wc=1 2 wn=3 4"]);
}

/// The intra-assignment event forms, blocking and nonblocking, capture a
/// pattern the same way.
#[test]
fn event_controlled_patterns() {
    let lines = run(r#"
module t;
  int wa [2];
  int wn [2];
  typedef struct { int x; int y; } s_t;
  s_t s;
  event e;
  initial begin
    fork
      wa = @(e) '{3, 4};
      wn <= @(e) '{5, 6};
      s = @(e) '{7, 8};
      #1 -> e;
    join
    #1;
    $display("R wa=%0d %0d wn=%0d %0d s=%0d %0d", wa[0], wa[1], wn[0], wn[1], s.x, s.y);
  end
endmodule
"#);
    assert_eq!(lines, ["R wa=3 4 wn=5 6 s=7 8"]);
}

/// §11.6.1: the delayed right-hand side is evaluated at the width of its
/// target, as without the delay: `y = #1 a + b` keeps the carry. The event
/// form already did.
#[test]
fn a_delayed_expression_takes_the_target_width() {
    let lines = run(r#"
module t;
  logic [3:0] a = 4'hF, b = 4'h1;
  logic [7:0] x, y, f, ev;
  event e;
  initial begin
    x = a + b;
    y = #1 a + b;
    fork
      f = #1 a + b;
      ev = @(e) a + b;
      #1 -> e;
    join
    $display("R x=%0d y=%0d f=%0d ev=%0d", x, y, f, ev);
  end
endmodule
"#);
    assert_eq!(lines, ["R x=16 y=16 f=16 ev=16"]);
}
