//! #951 review: §9.4.2 edge controls on class properties (operands with no
//! signal behind them). Table 9-2: posedge is detected on 0→x, 0→z, 0→1,
//! x→1 and z→1, negedge on 1→x, 1→z, 1→0, x→0 and z→0. An `or` list of such
//! operands, a term without an edge, and an `iff` guard (an edge while the
//! guard is false does not trigger) all wait for their own transitions.
//! Expected values come from the reference simulator.

use xezim::simulate;

#[test]
fn class_property_edges_follow_table_9_2_with_or_lists_and_iff() {
    let src = r#"
class C;
  logic f;
  bit a, b, en;
endclass
module top;
  C h;
  initial begin
    h = new; h.f = 0;
    fork
      begin @(posedge h.f); $display("%0t posedge 0->x f=%b", $time, h.f); end
      begin #20; @(negedge h.f); $display("%0t negedge x->0 f=%b", $time, h.f); end
      begin #40; @(posedge h.f); $display("%0t posedge 0->z f=%b", $time, h.f); end
      begin #60; @(negedge h.f); $display("%0t negedge z->0 f=%b", $time, h.f); end
      begin @(posedge h.a or posedge h.b); $display("%0t or-list a=%0d b=%0d", $time, h.a, h.b); end
      begin @(posedge h.a iff h.en); $display("%0t iff a=%0d en=%0d", $time, h.a, h.en); end
      begin @(h.en or negedge h.b); $display("%0t mixed en=%0d b=%0d", $time, h.en, h.b); end
    join_none
    #10 h.f = 1'bx;
    #2 h.b = 1;
    #18 h.f = 1'b0;
    #5 h.a = 1;
    #5 h.a = 0;
    #5 h.b = 0;
    #5 h.f = 1'bz; h.en = 1;
    #5 h.a = 1;
    #15 h.f = 1'b0;
    #10 $display("%0t done", $time);
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_eq!(
        out,
        [
            "10 posedge 0->x f=x",
            "12 or-list a=0 b=1",
            "30 negedge x->0 f=0",
            "45 mixed en=0 b=0",
            "50 posedge 0->z f=z",
            "55 iff a=1 en=1",
            "70 negedge z->0 f=0",
            "80 done",
        ]
    );
}
