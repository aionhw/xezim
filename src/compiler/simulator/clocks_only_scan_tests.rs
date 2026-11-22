//! A clock toggle with no process woken and no `wait` pending changes only
//! the clock, so the slot's edge pass scans the toggled clocks alone. The
//! condition-waiter drain must not widen that scan when it has nothing to
//! recheck.

/// The flop's input arrives by a delayed nonblocking assignment that falls
/// due at the first edge, so that slot runs its edges before the NBAs with
/// no other process active.
const SRC: &str = r#"
module top;
  logic clk = 0, x = 0, q = 0;
  always #5 clk = ~clk;
  always @(posedge clk) q <= x;
  initial begin
    #1 x <= #4 1;
    #30 $display("T|q=%b", q);
    $finish;
  end
endmodule
"#;

#[test]
fn a_clock_toggle_without_waiters_keeps_the_clocks_only_scan() {
    let sim = crate::simulate(SRC, 100).expect("clocks-only fixture");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    assert_eq!(lines, ["T|q=1"]);
    assert!(
        sim.prof_clocks_only_detect > 0,
        "no clocks-only edge pass ran"
    );
}
