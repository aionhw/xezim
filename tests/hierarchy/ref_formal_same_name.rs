//! A `ref` formal bound to an actual of the SAME name (`task t(ref int cnt)`
//! called as `t(cnt)`): the ref-formal redirect rewrote the identifier to
//! itself on the alias path, and evaluation re-entered the redirect on the
//! rewritten node until the stack overflowed. Whole-value, element and
//! part-select reads and writes through such a formal, from a module task
//! and from a task of a bound instance.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn ref_formal_named_like_its_actual() {
    let msgs = messages(
        "typedef bit [6:0] u7_t;
module dut(input logic clk); endmodule
bind dut probe v_probe();
module probe;
  task automatic bump(ref u7_t [1:0] cnt);
    cnt[1] = cnt[1] + 1;
  endtask
endmodule
module tb;
  logic clk = 0;
  u7_t [1:0] cnt;
  int total;
  dut u_dut(.clk(clk));
  task automatic sum(ref u7_t [1:0] cnt, ref int total);
    total = cnt[0] + cnt[1];
    cnt[0] = 0;
  endtask
  initial begin
    cnt[0] = 5; cnt[1] = 7; total = 0;
    sum(cnt, total);
    $display(\"S1 total=%0d cnt0=%0d cnt1=%0d\", total, cnt[0], cnt[1]);
    u_dut.v_probe.bump(cnt);
    $display(\"S2 cnt1=%0d\", cnt[1]);
    $finish;
  end
endmodule
",
    );
    for want in ["S1 total=12 cnt0=0 cnt1=7", "S2 cnt1=8"] {
        assert!(msgs.iter().any(|m| m == want), "missing {want}; got {msgs:?}");
    }
}
