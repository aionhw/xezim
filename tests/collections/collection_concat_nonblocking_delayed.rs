//! §10.10, §10.4.2, §9.4.5: an unpacked array concatenation, or a whole
//! queue, assigned to a queue, a dynamic array or a fixed unpacked array by
//! a nonblocking assignment, or with an intra-assignment timing control,
//! stores every element. The right-hand side was evaluated as one packed
//! value, so `a <= {a, 6}` left `'{1}`, `a <= s` copied one element and a
//! fixed-array target did not change.

use xezim::simulate;

fn run(src: &str) -> Vec<String> {
    let sim = simulate(src, 200).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn nonblocking_concatenation_into_queue_dynamic_and_fixed_arrays() {
    let msgs = run(r##"
module top;
  int a [$] = '{1, 2};
  int d [] = '{1, 2};
  int f [3] = '{1, 2, 3};
  int h [$] = '{1, 2};
  initial begin
    a <= {a, 6};
    d <= {d, 3};
    f <= {f[1], f[2], 9};
    h <= {5, {6, 7}};
    #0 $display("#0: a=%p d=%p f=%p h=%p", a, d, f, h);
    #1 $display("#1: a=%p d=%p f=%p h=%p", a, d, f, h);
    $finish;
  end
endmodule
"##);
    assert_eq!(
        msgs,
        [
            "#0: a='{1, 2} d='{1, 2} f='{1, 2, 3} h='{1, 2}",
            "#1: a='{1, 2, 6} d='{1, 2, 3} f='{2, 3, 9} h='{5, 6, 7}",
        ]
    );
}

#[test]
fn nonblocking_copy_of_a_whole_queue() {
    let msgs = run(r##"
module top;
  int a [$] = '{1, 2};
  int d [] = '{1};
  int s [$] = '{7, 8, 9};
  initial begin
    a <= s;
    d <= s;
    #1 $display("a=%p d=%p", a, d);
    $finish;
  end
endmodule
"##);
    assert_eq!(msgs, ["a='{7, 8, 9} d='{7, 8, 9}"]);
}

#[test]
fn intra_assignment_delay_on_a_blocking_concatenation() {
    let msgs = run(r##"
module top;
  int q [$] = '{1, 2};
  int d [] = '{1, 2};
  int f [3] = '{1, 2, 3};
  int h [$] = '{1, 2};
  int b [$] = '{1, 2};
  int s [$] = '{7, 8};
  int v = 5;
  initial q = #2 {q, v};
  initial d = #2 {d, 3};
  initial f = #2 {f[1], f[2], 9};
  initial h = #2 {5, {6, 7}};
  initial b = #2 s;
  initial #1 begin v = 50; s.push_back(60); end
  initial #3 begin
    $display("q=%p d=%p f=%p h=%p b=%p", q, d, f, h, b);
    $finish;
  end
endmodule
"##);
    assert_eq!(
        msgs,
        ["q='{1, 2, 5} d='{1, 2, 3} f='{2, 3, 9} h='{5, 6, 7} b='{7, 8}"]
    );
}

#[test]
fn intra_assignment_delay_on_a_nonblocking_concatenation() {
    let msgs = run(r##"
module top;
  int k [$] = '{1, 2};
  int f [2] = '{1, 2};
  int v = 4;
  initial begin
    k <= #2 {k[0], v};
    f <= #2 {v, f[0]};
  end
  initial #1 v = 40;
  initial #2 $display("t2 active: k=%p f=%p", k, f);
  initial #3 begin
    $display("t3: k=%p f=%p", k, f);
    $finish;
  end
endmodule
"##);
    assert_eq!(
        msgs,
        ["t2 active: k='{1, 2} f='{1, 2}", "t3: k='{1, 4} f='{4, 1}"]
    );
}

#[test]
fn intra_assignment_event_on_a_concatenation() {
    let msgs = run(r##"
module top;
  int m [$] = '{1, 2};
  int n [$] = '{1, 2};
  event e;
  initial m = @(e) {m, 9};
  initial n <= @(e) {n, 8};
  initial begin
    #1 -> e;
    #1 $display("m=%p n=%p", m, n);
    $finish;
  end
endmodule
"##);
    assert_eq!(msgs, ["m='{1, 2, 9} n='{1, 2, 8}"]);
}

#[test]
fn blocking_concatenation_and_fixed_array_nonblocking_copy() {
    let msgs = run(r##"
module top;
  int u [$] = '{1, 2};
  int g [2] = '{3, 4};
  int w [2] = '{0, 0};
  initial begin
    u = {u, 4};
    w <= g;
    #1 $display("u=%p w=%p", u, w);
    $finish;
  end
endmodule
"##);
    assert_eq!(msgs, ["u='{1, 2, 4} w='{3, 4}"]);
}
