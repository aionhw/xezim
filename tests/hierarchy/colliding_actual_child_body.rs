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

/// §5.6.1: an escaped port name may contain a dot. `\foo.bar ` bound to the
/// parent's `bar` still collides with the child's own `bar`; it is not a
/// generate rename that keeps the leaf.
#[test]
fn escaped_dotted_port_resolves_to_parent_net() {
    let src = r#"
module child (input logic \foo.bar , output logic bar);
  assign bar = ~\foo.bar ;
  initial #1 $display("CHILD %b %b", \foo.bar , bar);
endmodule
module top;
  logic bar = 1'b1;
  logic y;
  child u (.\foo.bar (bar), .bar(y));
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        msgs.iter().any(|m| m == "CHILD 1 0"),
        "child read its own output instead of the parent actual: {:?}",
        msgs
    );
}

/// §5.6.1 and §23.6. An escaped port and an identically spelled
/// hierarchical path can coexist in one child module.
#[test]
fn escaped_dotted_port_does_not_capture_a_hierarchical_path() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("escaped_dotted_port_actual");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("top.sv");
    std::fs::write(
        &path,
        r#"
module source #(parameter bit VALUE = 0)(output wire data);
  assign data = VALUE;
endmodule
module child(input logic \node.data );
  source node();
  initial #1 $display("PORT %b PATH %b", \node.data , node.data);
endmodule
module top;
  source #(.VALUE(1)) node();
  child u(.\node.data (node.data));
  child v(.\node.data (1'b0));
  initial #2 $finish;
endmodule
"#,
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
        .env("XEZIM_KEEP_PORTS", "0")
        .arg("--no-cache")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    let mut messages: Vec<_> = text
        .lines()
        .filter(|line| line.starts_with("PORT "))
        .collect();
    messages.sort_unstable();
    assert_eq!(messages, ["PORT 0 PATH 0", "PORT 1 PATH 0"]);
}
