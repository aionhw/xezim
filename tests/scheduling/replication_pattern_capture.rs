//! IEEE 1800-2023 §10.9.1, §10.4.2, §9.4.5: a replication pattern
//! (`'{2{7}}`) written to a one-dimensional unpacked array, queue or dynamic
//! array by a nonblocking assignment or an intra-assignment delay supplies
//! every element, as the blocking form does. The deferred capture took the
//! replication as one item, so `n <= '{2{7}}` stored `'{7, 0}` and
//! `q = #1 '{2{1, 2}}` stored `'{2}`.

use xezim::simulate;

fn run(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn nonblocking_replication_patterns_fill_every_element() {
    let msgs = run(r##"
module top;
  int n [2];
  int qn [$];
  int dn [];
  initial begin
    n <= '{2{7}};
    qn <= '{3{1, 2}};
    dn <= '{2{4}};
    #0 $display("#0: n=%p qn=%p dn=%p", n, qn, dn);
    #1 $display("#1: n=%p qn=%p dn=%p", n, qn, dn);
  end
endmodule
"##);
    assert_eq!(
        msgs,
        [
            "#0: n='{0, 0} qn='{} dn='{}",
            "#1: n='{7, 7} qn='{1, 2, 1, 2, 1, 2} dn='{4, 4}",
        ]
    );
}

#[test]
fn delayed_replication_patterns_fill_every_element() {
    let msgs = run(r##"
module top;
  int d [2];
  int q [$];
  int dy [];
  initial begin
    d = #1 '{2{7}};
    q = #1 '{2{1, 2}};
    dy = #1 '{2{4}};
    $display("d=%p q=%p dy=%p", d, q, dy);
  end
endmodule
"##);
    assert_eq!(msgs, ["d='{7, 7} q='{1, 2, 1, 2} dy='{4, 4}"]);
}

#[test]
fn delayed_replication_items_are_read_before_the_delay() {
    let msgs = run(r##"
module top;
  int v = 5;
  int x [2];
  int y [2];
  initial begin
    x <= #2 '{2{v}};
    v = 9;
    y = #2 '{2{v + 1}};
    v = 1;
    #1 $display("x=%p y=%p v=%0d", x, y, v);
  end
endmodule
"##);
    assert_eq!(msgs, ["x='{5, 5} y='{10, 10} v=1"]);
}

/// §10.9.1, §11.6.1: each replicated item takes the element type (a byte
/// element truncates, a real one keeps its value, a string one is not
/// concatenated), the count may be a parameter, a later element update in
/// the same time step still wins (§10.4.2), and a class task's deferred
/// patterns fill every element too.
#[test]
fn deferred_replication_items_take_the_element_type() {
    let msgs = run(r##"
class C;
  int a [2];
  int q [$];
  task run();
    a <= '{2{5}};
    q = #1 '{2{8, 9}};
  endtask
endclass
module top;
  localparam int N = 3;
  int v = 4;
  byte b [3];
  real r [2];
  string s [2];
  logic [3:0] l [2];
  int p [N];
  int e [4];
  C c;
  initial begin
    b <= '{3{9'h1ff}};
    r <= #1 '{2{1.5}};
    s <= '{2{"ab"}};
    l = #1 '{2{4'b1x01}};
    p <= '{N{v}};
    e <= '{2{1, 2}};
    e[1] <= 9;
    v = 0;
    c = new;
    c.run();
    #1 $display("b=%p r=%p s=%p l=%b,%b p=%p e=%p", b, r, s, l[0], l[1], p, e);
    $display("c.a=%p c.q=%p", c.a, c.q);
  end
endmodule
"##);
    assert_eq!(
        msgs,
        [
            r#"b='{-1, -1, -1} r='{1.5, 1.5} s='{"ab", "ab"} l=1x01,1x01 p='{4, 4, 4} e='{1, 9, 1, 2}"#,
            "c.a='{5, 5} c.q='{8, 9, 8, 9}",
        ]
    );
}

#[test]
fn blocking_and_positional_patterns_are_unchanged() {
    let msgs = run(r##"
module top;
  int b [2];
  int w [2];
  int p [3];
  initial begin
    b = '{2{3}};
    w <= '{1, 2};
    p = #1 '{4, 5, 6};
    #1 $display("b=%p w=%p p=%p", b, w, p);
  end
endmodule
"##);
    assert_eq!(msgs, ["b='{3, 3} w='{1, 2} p='{4, 5, 6}"]);
}
