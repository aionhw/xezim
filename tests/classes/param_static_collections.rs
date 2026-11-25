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
