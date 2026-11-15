//! §10.4.2, §4.9.4, §9.4.5: a nonblocking assignment of an assignment
//! pattern to a whole unpacked array or unpacked struct updates the target
//! in the NBA region, like any other nonblocking assignment. The pattern was
//! spread over the member signals at once, as a blocking assignment, so
//! reads later in the same time step already saw the new value, a swap
//! through two patterns copied one side onto the other, and an
//! intra-assignment delay (`wn <= #1 '{3, 4}`) was ignored.

use xezim::simulate;

fn run(src: &str) -> Vec<String> {
    let sim = simulate(src, 200).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn swap_through_patterns_in_always_ff() {
    let msgs = run(r##"
module top;
  typedef struct { int x; int y; } s_t;
  logic clk = 0;
  int a [2] = '{1, 2};
  int b [2] = '{3, 4};
  s_t p = '{5, 6};
  s_t q = '{7, 8};
  always_ff @(posedge clk) begin
    a <= '{b[0], b[1]};
    b <= '{a[0], a[1]};
  end
  // Plain `always`: xezim counts a struct declaration initializer as a
  // second driver of an always_ff target (tracked separately).
  always @(posedge clk) begin
    p <= '{q.x, q.y};
    q <= '{p.x, p.y};
  end
  initial begin
    #1 clk = 1; #1;
    $display("a=%0d %0d b=%0d %0d p=%0d %0d q=%0d %0d",
             a[0], a[1], b[0], b[1], p.x, p.y, q.x, q.y);
    $finish;
  end
endmodule
"##);
    assert_eq!(msgs, ["a=3 4 b=1 2 p=7 8 q=5 6"]);
}

#[test]
fn pattern_nonblocking_update_waits_for_the_nba_region() {
    let msgs = run(r##"
module top;
  typedef struct { int x; int y; } s_t;
  int wn [2]; int wk [3]; int w2 [2][2]; s_t st; s_t sa [2];
  initial begin
    wn <= '{3, 4};
    wk <= '{1: 9, default: 2};
    w2 <= '{'{1, 2}, '{3, 4}};
    st <= '{5, 6};
    sa[1] <= '{7, 8};
    $display("now: wn=%0d %0d wk=%0d %0d %0d w2=%0d %0d st=%0d %0d sa=%0d %0d",
             wn[0], wn[1], wk[0], wk[1], wk[2], w2[0][0], w2[1][1], st.x, st.y, sa[1].x, sa[1].y);
    #0;
    $display("#0: wn=%0d %0d wk=%0d %0d %0d w2=%0d %0d st=%0d %0d sa=%0d %0d",
             wn[0], wn[1], wk[0], wk[1], wk[2], w2[0][0], w2[1][1], st.x, st.y, sa[1].x, sa[1].y);
    #1;
    $display("#1: wn=%0d %0d wk=%0d %0d %0d w2=%0d %0d st=%0d %0d sa=%0d %0d",
             wn[0], wn[1], wk[0], wk[1], wk[2], w2[0][0], w2[1][1], st.x, st.y, sa[1].x, sa[1].y);
    $finish;
  end
endmodule
"##);
    assert_eq!(
        msgs,
        [
            "now: wn=0 0 wk=0 0 0 w2=0 0 st=0 0 sa=0 0",
            "#0: wn=0 0 wk=0 0 0 w2=0 0 st=0 0 sa=0 0",
            "#1: wn=3 4 wk=2 9 2 w2=1 4 st=5 6 sa=7 8",
        ]
    );
}

#[test]
fn intra_assignment_delay_on_a_pattern_nonblocking_assignment() {
    let msgs = run(r##"
module top;
  typedef struct { int x; int y; } s_t;
  int wn [2]; s_t st; int v = 3;
  initial begin
    wn <= #2 '{v, v + 1};
    st <= #2 '{v + 2, v + 3};
    v = 10;
    #1 $display("t1: wn=%0d %0d st=%0d %0d", wn[0], wn[1], st.x, st.y);
    #1 $display("t2: wn=%0d %0d st=%0d %0d", wn[0], wn[1], st.x, st.y);
    #1 $display("t3: wn=%0d %0d st=%0d %0d", wn[0], wn[1], st.x, st.y);
    $finish;
  end
endmodule
"##);
    assert_eq!(
        msgs,
        [
            "t1: wn=0 0 st=0 0",
            "t2: wn=0 0 st=0 0",
            "t3: wn=3 4 st=5 6",
        ]
    );
}

#[test]
fn later_nonblocking_assignment_to_an_element_wins() {
    // §4.6, §10.4.2: updates commit in the order they were made.
    let msgs = run(r##"
module top;
  int a [2]; int b [2];
  initial begin
    a <= '{1, 2};
    a[0] <= 9;
    b[0] <= 9;
    b <= '{1, 2};
    #1 $display("a=%0d %0d b=%0d %0d", a[0], a[1], b[0], b[1]);
    $finish;
  end
endmodule
"##);
    assert_eq!(msgs, ["a=9 2 b=1 2"]);
}

#[test]
fn nonblocking_copy_from_a_variable_waits_for_the_nba_region() {
    // Guard: a whole-array or whole-struct source already took this path.
    let msgs = run(r##"
module top;
  typedef struct { int x; int y; } s_t;
  int src [2] = '{3, 4}; int wv [2]; s_t ss = '{5, 6}; s_t sv;
  initial begin
    wv <= src;
    sv <= ss;
    $display("now: wv=%0d %0d sv=%0d %0d", wv[0], wv[1], sv.x, sv.y);
    #1 $display("#1: wv=%0d %0d sv=%0d %0d", wv[0], wv[1], sv.x, sv.y);
    $finish;
  end
endmodule
"##);
    assert_eq!(msgs, ["now: wv=0 0 sv=0 0", "#1: wv=3 4 sv=5 6"]);
}
