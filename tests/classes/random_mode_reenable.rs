use xezim::simulate;
#[test]
fn reenable_array_and_constraint_after_whole_object_disable() {
    let sim = simulate(include_str!("../fixtures/random_mode_reenable.sv"), 1000).unwrap();
    let out = sim.output.iter().map(|o| o.message.as_str()).collect::<Vec<_>>().join("\n");
    assert!(out.contains("PASS") && !out.contains("Fatal"), "{out}");
}

/// §18.9, §18.5.11: `obj.constraint_mode(0)` with no block name also turns
/// off the object's static blocks (class-wide), and `obj.constraint_mode(1)`
/// turns them back on.
#[test]
fn whole_object_constraint_mode_reaches_static_blocks() {
    let src = r#"
class packet;
  rand int x;
  static constraint sc { x == 5; }
  constraint ic { x < 100; }
endclass
module top;
  initial begin
    packet p = new();
    p.constraint_mode(0);
    if (p.sc.constraint_mode() != 0) $fatal(1, "static block still on");
    if (!p.randomize() with {x == 6;} || p.x != 6) $fatal(1, "static block still enforced");
    p.constraint_mode(1);
    if (!p.randomize() || p.x != 5) $fatal(1, "static block not re-enabled, x=%0d", p.x);
    $display("PASS");
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 1000).unwrap();
    let out = sim.output.iter().map(|o| o.message.as_str()).collect::<Vec<_>>().join("\n");
    assert!(out.contains("PASS") && !out.contains("Fatal"), "{out}");
}
