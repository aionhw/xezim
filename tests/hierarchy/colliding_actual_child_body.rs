//! §23.3.3 (issue #128): a whole-net port actual whose bare name is ALSO
//! declared by the child module used to be pasted into the child's BODY
//! assigns un-rooted. The continuous assign's inferred scope hint then
//! re-resolved it child-first — `.v_node(out_val)` into a module whose own
//! output port is `out_val` made the body read `<inst>.out_val` (its own
//! output) instead of the parent net, silently severing the feedback path.
//!
//! Fixed in core's `rewrite_expr_impl`: a colliding whole-net substitution is
//! root-marked (`$root`), and the CA dependency machinery resolves rooted
//! reads absolutely. The GENERATE-scope rename map (`vec` → `gen.vec`)
//! flows through the same substitution and must NOT be rooted (its values are
//! not yet instance-prefixed) — the leaf==key gate keeps it on the plain
//! machinery; the genfor compliance suite pins that side.
//!
//! Reference-simulator verified (values match to 9 digits on the issue's RK4
//! reproducer; this is a minimized integer version of the same topology).

use xezim::simulate;

#[test]
fn colliding_actual_resolves_to_parent_net() {
    let src = r#"
module blk #(parameter int BIAS = 0)
           (input int v_node, output int out_val);
  // Reads v_node (the PARENT's out_val) — not its own out_val.
  assign out_val = BIAS + v_node * 2;
endmodule

module top;
  int out_val, leg0, leg1;
  blk #(.BIAS(100)) b0 (.v_node(out_val), .out_val(leg0));
  blk #(.BIAS(200)) b1 (.v_node(out_val), .out_val(leg1));
  initial begin
    out_val = 5;
    #1;
    // leg0 = 100 + 5*2 = 110; leg1 = 200 + 5*2 = 210. With the bug both
    // blocks read their own out_val port (a self-loop), not the parent's 5.
    if (leg0 == 110 && leg1 == 210) $display("PASS_%0d_%0d", leg0, leg1);
    else $display("FAIL_%0d_%0d", leg0, leg1);
    // The feedback path must stay LIVE, not just settle once at t0.
    out_val = 7;
    #1;
    if (leg0 == 114 && leg1 == 214) $display("PASS2_%0d_%0d", leg0, leg1);
    else $display("FAIL2_%0d_%0d", leg0, leg1);
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        msgs.iter().any(|m| m == "PASS_110_210"),
        "child body read its own output instead of the parent actual: {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m == "PASS2_114_214"),
        "feedback path did not stay live after t0: {:?}",
        msgs
    );
}
