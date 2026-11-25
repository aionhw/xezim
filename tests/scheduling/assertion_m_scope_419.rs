//! IEEE 1800-2017 §21.2.1.7 / §16.14: `%m` in a declarative concurrent
//! assertion's action block must name the instance where the assertion is
//! declared, not the instance the time-0 settle evaluated last. A continuous
//! assignment reading a generate-block variable can otherwise leak its scope
//! into the registration of assertions in sibling modules.
use xezim::simulate;
fn outputs(source: &str) -> Vec<String> {
    let sim = simulate(source, 100_000).unwrap();
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// #419 original: a sibling module's settle block must not capture the
/// assertion's `%m`.
#[test]
fn assertion_action_names_declaring_instance_not_last_settled_scope() {
    let src = r#"
module unit(output logic o);
  for (genvar k = 0; k < 2; k++) begin : gb
    logic [3:0] datum = k;
  end
  assign o = gb[1].datum == 1;
endmodule

module top;
  logic clk = 0, o;
  string where = "";
  unit u (.o(o));
  always #5 clk = ~clk;
  ap: assert property (@(posedge clk) 1) $sformat(where, "%m");
  initial begin
    #7;
    if (where == "top.ap") $display("PASS");
    else $display("FAIL where=%s", where);
    $finish;
  end
endmodule
"#;
    let msgs = outputs(src);
    assert!(
        msgs.iter().any(|m| m.starts_with("PASS")),
        "expected PASS, got {:?}",
        msgs
    );
}

/// Cousin: assertions declared inside submodules name the submodule scope
/// (`top.s.ap`, `top.t.ap`), not the declaring sibling's own assertion scope.
#[test]
fn submodule_assertion_names_its_own_scope() {
    let src = r#"
module sub(input logic clk, output logic o);
  logic x = 0;
  assign o = x;
  string where_sub = "";
  ap: assert property (@(posedge clk) 1) $sformat(where_sub, "%m");
endmodule

module top;
  logic clk = 0;
  logic o, o2;
  string where = "";
  sub s (.clk(clk), .o(o));
  sub t (.clk(clk), .o(o2));
  always #5 clk = ~clk;
  ap: assert property (@(posedge clk) 1) $sformat(where, "%m");
  initial begin
    #7;
    if (where == "top.ap" && s.where_sub == "top.s.ap" && t.where_sub == "top.t.ap")
      $display("PASS");
    else $display("FAIL where=%s s=%s t=%s", where, s.where_sub, t.where_sub);
    $finish;
  end
endmodule
"#;
    let msgs = outputs(src);
    assert!(
        msgs.iter().any(|m| m.starts_with("PASS")),
        "expected PASS, got {:?}",
        msgs
    );
}
