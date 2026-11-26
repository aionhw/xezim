//! IEEE 1800-2023 §10.4.1, §10.9.1, §10.9.2 (#1274): a blocking assignment
//! evaluates its whole right-hand side before it writes the target, so a
//! pattern whose items read the target swaps. Each item used to be read just
//! before its element was written. Beyond module arrays, queues and structs:
//! class properties (bare, `this.`, through a handle, and a bare target read
//! as `this.e2`), an array member of a struct, a function-local array in a
//! function called often enough to be compiled, an associative array, a
//! replication, and patterns that read other variables, a packed target, a
//! nonblocking and an intra-assignment delay store, which were right.
//! Expected values are the reference simulator's.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn self_reading_patterns_in_every_scope() {
    let src = r#"
module top;
  typedef struct { int a; int b; } s_t;
  typedef struct { int x; int y [2]; } t_t;
  class C;
    int arr [2] = '{1, 2};
    int q [$] = '{1, 2, 3};
    int d [] = '{4, 5};
    s_t s = '{1, 2};
    int e2 [2] = '{1, 2};
    function void swap();
      e2 = '{this.e2[1], this.e2[0]};
      arr = '{arr[1], arr[0]};
      q = '{q[2], q[0], q[1]};
      d = '{d[1], d[0]};
      s = '{s.b, s.a};
    endfunction
    function void swap_this();
      this.arr = '{this.arr[1], this.arr[0]};
    endfunction
  endclass
  int a [2] = '{1, 2};
  int k [2] = '{1, 2};
  int m [2][2] = '{'{1, 2}, '{3, 4}};
  int r [2][2] = '{'{1, 2}, '{3, 4}};
  int q [$] = '{1, 2, 3};
  int y [] = '{4, 5};
  int f [3] = '{1, 2, 3};
  int d [1:0] = '{1, 2};
  s_t s = '{1, 2};
  t_t t = '{5, '{6, 7}};
  t_t st = '{5, '{6, 7}};
  int rp [4] = '{1, 2, 3, 4};
  int lo [string] = '{"a": 1, "b": 2};
  int nb [2] = '{1, 2};
  int b [2];
  logic [1:0][7:0] p = {8'd1, 8'd2};
  int ret;
  function automatic int fn_local();
    int l [3] = '{1, 2, 3};
    l = '{l[2], l[1], l[0]};
    return l[0] * 100 + l[1] * 10 + l[2];
  endfunction
  initial begin
    C c = new;
    a = '{a[1], a[0]};
    k = '{1: k[0], 0: k[1]};
    m = '{'{m[1][0], m[1][1]}, '{m[0][0], m[0][1]}};
    r[1] = '{r[1][1], r[1][0]};
    q = '{q[2], q[0], q[1]};
    y = '{y[1], y[0], y[0] + y[1]};
    f = '{0: f[2], default: f[0]};
    d = '{d[0], d[1] + 10};
    s = '{s.b, s.a};
    t = '{t.y[1], '{t.x, t.y[0]}};
    st.y = '{st.y[1], st.y[0]};
    rp = '{2{rp[3], rp[0]}};
    $display("T|a=%p k=%p m=%p r=%p q=%p y=%p", a, k, m, r, q, y);
    $display("T|f=%p d=%p s=%p t=%p st=%p rp=%p", f, d, s, t, st, rp);
    c.swap();
    $display("T|cls arr=%p q=%p d=%p s=%p e2=%p", c.arr, c.q, c.d, c.s, c.e2);
    c.swap_this();
    c.arr = '{c.arr[0] + 10, c.arr[1]};
    $display("T|this arr=%p", c.arr);
    for (int i = 0; i < 1200; i++) ret = fn_local();
    lo = '{"a": lo["b"], "b": lo["a"]};
    $display("T|fn=%0d lo_a=%0d lo_b=%0d", ret, lo["a"], lo["b"]);
    b = '{a[1], a[0]};
    p = '{p[0], p[1]};
    nb <= '{nb[1], nb[0]};
    a = #1 '{a[1], a[0]};
    $display("T|b=%p p=%h nb=%p a=%p", b, p, nb, a);
  end
endmodule
"#;
    let expected = [
        r#"T|a='{2, 1} k='{2, 1} m='{'{3, 4}, '{1, 2}} r='{'{1, 2}, '{4, 3}} q='{3, 1, 2} y='{5, 4, 9}"#,
        r#"T|f='{3, 1, 1} d='{2, 11} s='{a:2, b:1} t='{x:7, y:'{5, 6}} st='{x:5, y:'{7, 6}} rp='{4, 1, 4, 1}"#,
        r#"T|cls arr='{2, 1} q='{3, 1, 2} d='{5, 4} s='{a:2, b:1} e2='{2, 1}"#,
        r#"T|this arr='{11, 2}"#,
        r#"T|fn=321 lo_a=2 lo_b=1"#,
        r#"T|b='{1, 2} p=0201 nb='{2, 1} a='{1, 2}"#,
    ];
    assert_eq!(t_lines(src), expected);
}
