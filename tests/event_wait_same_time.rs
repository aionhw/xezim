//! An `@(signal)` waiter must wake from a change to that signal made LATER in
//! the same timestamp it armed — the "apply stimulus, then wait for the
//! response" pattern. xezim used to blanket-suppress every waiter registered in
//! the current snapshot generation, so an NBA or cross-process write at the
//! same time never woke it (the waiter blocked forever). Fixed by checking such
//! a waiter against its ARM-TIME sensitivity values, so a genuine post-arm
//! change fires it while a pre-arm edge (a `forever @(posedge clk)` re-arm, or
//! time-0 init pseudo-edges) still correctly does not.

fn woke(src: &str) -> bool {
    let sim = xezim::simulate(src, 10_000).expect("simulate");
    sim.output.iter().any(|o| o.message.contains("WOKE"))
}

/// `a <= 1; @(w)` where `w` is combinationally derived from `a`: the NBA
/// applies after the waiter arms, and its resulting edge on `w` must wake it.
#[test]
fn nba_change_wakes_same_time_waiter() {
    assert!(woke(
        "module tb; reg a=0,b=1; wire w; assign w=a&b;\n\
         initial begin #1 a<=1; @(w); $display(\"WOKE\"); $finish; end endmodule"
    ));
}

/// Two processes at the same time: one arms `@(w)`, the other writes `w`. The
/// write must wake the waiter regardless of drain order.
#[test]
fn cross_process_same_time_change_wakes_waiter() {
    assert!(woke(
        "module tb; reg w=0;\n\
         initial begin #1 @(w); $display(\"WOKE\"); $finish; end\n\
         initial begin #1 w=1; end endmodule"
    ));
}

/// A `forever @(posedge clk)` must still fire exactly once per clock — the fix
/// must not make it re-fire on the edge it just consumed.
#[test]
fn forever_posedge_not_double_fired() {
    let sim = xezim::simulate(
        "module tb; reg clk=0; integer n=0;\n\
         always #5 clk=~clk;\n\
         initial begin forever @(posedge clk) n=n+1; end\n\
         initial begin #52 $display(\"N=%0d\", n); $finish; end endmodule",
        1000,
    )
    .expect("simulate");
    let out: String = sim.output.iter().map(|o| o.message.clone()).collect();
    // 5 posedges in 52 time units (at t=5,15,25,35,45), one count each.
    assert!(out.contains("N=5"), "expected exactly 5 posedge counts, got: {}", out);
}
