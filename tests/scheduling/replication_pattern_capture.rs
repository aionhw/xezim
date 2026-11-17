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
    fork y = #2 '{2{v + 1}}; join_none
    v = 9;
    #3 $display("x=%p y=%p v=%0d", x, y, v);
  end
endmodule
"##);
    assert_eq!(msgs, ["x='{5, 5} y='{6, 6} v=9"]);
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
