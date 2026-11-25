//! IEEE 1800-2023 §8.9, §8.25, §10.9.1: static collections and fixed-size
//! arrays in parameterized classes isolate per specialization, preserve
//! parameter-sized element widths, allow pre-`new()` direct access/mutation,
//! and drive instance `inside` constraints per specialization.
use xezim::simulate;

#[test]
fn param_static_collections_isolate_and_pre_new_access() {
    if !super::compiled_method_test_env::policies(&[("0", "1000"), ("1", "0")]) {
        return;
    }
    let sim = simulate(
        r#"
module top;
  class P #(int N = 4);
    static int s_dyn[] = '{N, N + 1};
    static int s_q[$]  = '{N * 10};
    static int s_aa[string] = '{"k": N + 2};
    static int s_fix[2] = '{N + 100, N + 101};
    static bit [N-1:0] sw_q[$] = '{1'b1 + 1'b1, '1};
    static bit [N-1:0] sw_aa[string] = '{"w": '1};
    rand int rv;
    constraint c_in { rv inside {s_dyn}; }
  endclass

  initial begin
    P#(8) p8;
    P#(4) p4;
    int ok8, ok4;

    $display("pre8=%0d,%0d,%0d,%0d,%0d,%0d,%0d,%0d",
      P#(8)::s_dyn.size(), P#(8)::s_dyn[1],
      P#(8)::s_q.size(), P#(8)::s_q[0],
      P#(8)::s_aa.num(), P#(8)::s_aa["k"],
      P#(8)::s_fix[0], P#(8)::s_fix[1]);
    $display("pre4=%0d,%0d,%0d,%0d,%0d,%0d,%0d,%0d",
      P#(4)::s_dyn.size(), P#(4)::s_dyn[1],
      P#(4)::s_q.size(), P#(4)::s_q[0],
      P#(4)::s_aa.num(), P#(4)::s_aa["k"],
      P#(4)::s_fix[0], P#(4)::s_fix[1]);
    $display("w=%0h,%0h,%0h,%0h,%0h,%0h",
      P#(8)::sw_q[0], P#(8)::sw_q[1],
      P#(4)::sw_q[0], P#(4)::sw_q[1],
      P#(8)::sw_aa["w"], P#(4)::sw_aa["w"]);

    P#(8)::s_dyn[0] = 88;
    P#(8)::s_q.push_back(888);
    P#(8)::s_aa["k"] = 90;
    P#(8)::s_fix[0] = 777;
    $display("mut=%0d,%0d,%0d,%0d,%0d,%0d,%0d,%0d",
      P#(8)::s_q.size(), P#(8)::s_q[1], P#(4)::s_q.size(),
      P#(8)::s_fix[0], P#(4)::s_fix[0],
      P#(8)::s_aa["k"], P#(4)::s_aa["k"], P#(4)::s_dyn[0]);

    p8 = new;
    p4 = new;
    ok8 = p8.randomize();
    ok4 = p4.randomize();
    $display("rnd=%0d,%0d,%0d,%0d",
      ok8, (p8.rv == 88 || p8.rv == 9),
      ok4, (p4.rv == 4 || p4.rv == 5));
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(lines.contains(&"pre8=2,9,1,80,1,10,108,109"), "{lines:?}");
    assert!(lines.contains(&"pre4=2,5,1,40,1,6,104,105"), "{lines:?}");
    assert!(lines.contains(&"w=2,ff,2,f,ff,f"), "{lines:?}");
    assert!(lines.contains(&"mut=2,888,1,777,104,90,6,4"), "{lines:?}");
    assert!(lines.contains(&"rnd=1,1,1,1"), "{lines:?}");
}

/// §8.25 and §10.10: fixed static arrays take unpacked concatenations in
/// declared order, with each specialization's actual parameter values.
#[test]
fn static_fixed_array_concatenation_uses_specialized_bounds_and_values() {
    let sim = simulate(
        r#"
module top;
  class P #(int N = 4);
    static int values[N:N-1] = {N, N+1};
  endclass
  class Plain;
    static int values[3:2] = {7, 8};
  endclass
  initial begin
    $display("specialized %0d %0d", P#(8)::values[8], P#(8)::values[7]);
    $display("default %0d %0d", P#(4)::values[4], P#(4)::values[3]);
    $display("plain %0d %0d", Plain::values[3], Plain::values[2]);
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert_eq!(lines, ["specialized 8 9", "default 4 5", "plain 7 8"]);
}

/// §7.6 and §8.25: a fixed static array initializer copies the source
/// specialization in declared index order.
#[test]
fn fixed_static_initializer_copies_a_specialized_array() {
    let sim = xezim::simulate(
        r#"
module top;
class A #(int N=4);
  static int src[3:2] = '{N,N+1};
  static int dst[3:1] = {src,N+2};
  static int dup[3:2] = src;
  static bit[N-1:0] state[3:2] = '{1'b1+1'b1,'1};
endclass
initial $display("static %0d %0d %0d %0d %0d %0d %0h",A#(8)::dst[3],A#(8)::dst[2],A#(8)::dst[1],A#(8)::dup[3],A#(8)::dup[2],A#(8)::state[3],A#(8)::state[2]);
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|entry| entry.message.as_str()).collect();
    assert_eq!(lines, ["static 8 9 10 8 9 2 ff"]);
}

/// §7.8 and §8.25: a static associative array uses the specialization’s
/// concrete key type for lookup, including narrowing integral keys.
#[test]
fn specialized_static_associative_keys_use_the_bound_type() {
    let sim = xezim::simulate(
        r#"
module top;
  typedef string key_t;
  class C #(type K=string, int W=8);
    static bit [W-1:0] tab[K];
  endclass
  initial begin
    C#(key_t,8)::tab["alpha"]=11;
    C#(key_t,8)::tab["bravo"]=22;
    C#(bit[2:0],4)::tab[11]=4'hd;
    $display("key %0d %0d %0d %0d %0d",C#(key_t,8)::tab.num(),C#(key_t,8)::tab["alpha"],C#(key_t,8)::tab["bravo"],C#(bit[2:0],4)::tab.num(),C#(bit[2:0],4)::tab[3]);
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|entry| entry.message.as_str()).collect();
    assert_eq!(lines, ["key 2 11 22 1 13"]);
}

/// §8.25 and §20.7: a query of a directly named specialization reports
/// the fixed static member’s actual bounds rather than default bounds.
#[test]
fn specialized_static_queries_use_actual_array_bounds() {
    let sim = xezim::simulate(
        r#"
module top;
  class P #(int N=4);
    static logic [N-1:0] data[N:N-2] = '{'1,2,3};
  endclass
  initial begin
    $display("shape %0d %0d %0d %0d %0d %0d", $size(P#(8)::data),$left(P#(8)::data),$right(P#(8)::data),$bits(P#(8)::data),$bits(P#(8)::data[8]),$bits(P#(4)::data[4]));
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|entry| entry.message.as_str()).collect();
    assert_eq!(lines, ["shape 3 8 6 24 8 4"]);
}
