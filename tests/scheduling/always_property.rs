//! IEEE 1800-2017 §16.12.11: `always property_expr` in a concurrent assertion
//! is a property operator, not an always block. It did not parse.

#[test]
fn always_property_holds_then_fails() {
    let result = xezim::simulate(
        r#"
module t;
  bit clk;
  logic [1:0] a;
  always #5 clk = ~clk;
  always @(posedge clk) $display("TICK %b", a);
  assert property (@(posedge clk) always (|a) == (&a))
    else $display("FAIL %0d", $time);
  initial begin
    a = 2'b11;
    repeat (2) @(posedge clk);
    @(negedge clk) a = 2'b10;
    repeat (2) @(posedge clk);
    $display("DONE");
    $finish;
  end
endmodule
"#,
        100_000,
    )
    .unwrap();
    let lines: Vec<_> = result
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert!(
        lines.len() >= 2 && lines[0] == "TICK 11" && lines[1] == "TICK 11",
        "held cycles should not fail: {lines:?}"
    );
    assert_eq!(
        lines.iter().find(|l| l.starts_with("FAIL")),
        Some(&"FAIL 25"),
        "{lines:?}"
    );
    assert!(lines.contains(&"DONE"), "missing DONE: {lines:?}");
}
