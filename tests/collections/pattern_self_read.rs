//! IEEE 1800-2023 §10.4.1, §10.9.1, §10.9.2: a blocking assignment
//! evaluates its whole right-hand side before writing the target, so a
//! pattern whose items read the target swaps (`a = '{a[1], a[0]}` gives
//! `'{2, 1}`). Each item was evaluated just before its element was written,
//! so later items read elements already overwritten: `'{2, 2}`.

use xezim::simulate;

fn run(src: &str) -> Vec<String> {
    let sim = simulate(src, 200).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn one_dimensional_arrays_and_queues_swap() {
    let msgs = run(r##"
module top;
  int a [2] = '{1, 2};
  int e [2] = '{1, 2};
  int d [1:0] = '{1, 2};
  int q [$] = '{1, 2, 3};
  int y [] = '{4, 5};
  initial begin
    a = '{a[1], a[0]};
    e = '{7, e[0]};
    d = '{d[0], d[1] + 10};
    q = '{q[2], q[0], q[1]};
    y = '{y[1], y[0], y[0] + y[1]};
    $display("a=%p e=%p d=%p q=%p y=%p", a, e, d, q, y);
  end
endmodule
"##);
    assert_eq!(
        msgs,
        ["a='{2, 1} e='{7, 1} d='{2, 11} q='{3, 1, 2} y='{5, 4, 9}"]
    );
}

#[test]
fn keyed_two_dimensional_and_row_patterns_swap() {
    let msgs = run(r##"
module top;
  int k [2] = '{1, 2};
  int m [2][2] = '{'{1, 2}, '{3, 4}};
  int r [2][2] = '{'{1, 2}, '{3, 4}};
  int f [3] = '{1, 2, 3};
  initial begin
    k = '{1: k[0], 0: k[1]};
    m = '{'{m[1][0], m[1][1]}, '{m[0][0], m[0][1]}};
    r[1] = '{r[1][1], r[1][0]};
    f = '{0: f[2], default: f[0]};
    $display("k=%p m=%p r=%p f=%p", k, m, r, f);
  end
endmodule
"##);
    assert_eq!(
        msgs,
        ["k='{2, 1} m='{'{3, 4}, '{1, 2}} r='{'{1, 2}, '{4, 3}} f='{3, 1, 1}"]
    );
}

#[test]
fn struct_members_swap() {
    let msgs = run(r##"
module top;
  typedef struct { int a; int b; } s_t;
  typedef struct { int x; int y [2]; } t_t;
  s_t s = '{1, 2};
  t_t t = '{5, '{6, 7}};
  initial begin
    s = '{s.b, s.a};
    t = '{t.y[1], '{t.x, t.y[0]}};
    $display("s=%p t=%p", s, t);
  end
endmodule
"##);
    assert_eq!(msgs, ["s='{a:2, b:1} t='{x:7, y:'{5, 6}}"]);
}

#[test]
fn patterns_not_reading_the_target_are_unchanged() {
    let msgs = run(r##"
module top;
  int a [2] = '{1, 2};
  int b [2];
  int m [2][2];
  int n [2];
  initial begin
    b = '{a[1], a[0]};
    m = '{default: 3};
    n <= '{n[1] + 1, 5};
    #1 $display("a=%p b=%p m=%p n=%p", a, b, m, n);
  end
endmodule
"##);
    assert_eq!(
        msgs,
        ["a='{1, 2} b='{2, 1} m='{'{3, 3}, '{3, 3}} n='{1, 5}"]
    );
}
