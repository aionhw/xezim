//! §28.4 pull-strength resolution: pullup/pulldown (and fully-weak
//! `(pull1,pull0)` continuous assigns) are WEAK drivers. A strong driver on the
//! same net overrides them; the pull only holds the net at its value where it
//! is otherwise high-Z. xezim previously treated pull drivers as strong, so a
//! strong-vs-pull conflict wrongly resolved to `x`.

use xezim::simulate;

fn last(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output.iter().rev().find(|o| o.message.contains(tag))
        .map(|o| o.message.clone()).unwrap_or_default()
}

#[test]
fn strong_driver_overrides_pullup() {
    // pull-up + tristate: driver active -> its value; driver off (z) -> pull.
    let sim = simulate(
        "module t; reg a=0, en=1; wire y; pullup(y); bufif1 g(y,a,en);\n\
         initial begin #1 $display(\"A y=%b\",y); en=0; #1 $display(\"B y=%b\",y); $finish; end endmodule",
        100,
    ).expect("sim");
    assert!(last(&sim, "A y=").contains("A y=0"), "strong 0 must beat pull-up: {}", last(&sim,"A y="));
    assert!(last(&sim, "B y=").contains("B y=1"), "pull-up holds when driver off: {}", last(&sim,"B y="));
}

#[test]
fn strong_driver_overrides_pulldown_via_mos() {
    let sim = simulate(
        "module t; reg d=1, g=1; wire y; pmos p(y,d,g); pulldown(y);\n\
         initial begin #1 $display(\"A y=%b\",y); g=0; #1 $display(\"B y=%b\",y); $finish; end endmodule",
        100,
    ).expect("sim");
    assert!(last(&sim, "A y=").contains("A y=0"), "pmos off -> pull-down 0");
    assert!(last(&sim, "B y=").contains("B y=1"), "pmos passes strong 1 over pull-down");
}

#[test]
fn strong_assign_beats_pull_strength_assign() {
    let sim = simulate(
        "module t; wire y; assign y=1'b1; assign (pull1,pull0) y=1'b0;\n\
         initial begin #1 $display(\"Y y=%b\",y); $finish; end endmodule",
        100,
    ).expect("sim");
    assert!(last(&sim, "Y y=").contains("Y y=1"), "strong 1 beats pull0: {}", last(&sim,"Y y="));
}

#[test]
fn opposing_pulls_conflict_to_x() {
    let sim = simulate(
        "module t; wire y; pullup(y); pulldown(y);\n\
         initial begin #1 $display(\"Y y=%b\",y); $finish; end endmodule",
        100,
    ).expect("sim");
    assert!(last(&sim, "Y y=").contains("Y y=x"), "pull1 vs pull0 -> x");
}
