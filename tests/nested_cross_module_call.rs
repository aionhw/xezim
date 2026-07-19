//! A hierarchical subroutine call made FROM INSIDE another subroutine —
//! `top` calls `m.go()`, and `go` (in module `mid`) calls `l.deep()` on its own
//! child instance `l`. The inner call flattens to `m.l.deep`, but the bare
//! dotted name is `l.deep`; the dispatch now retries with the caller's scope
//! (`m`) prepended so the nested call resolves. Previously `l.deep()` silently
//! did nothing.

use xezim::simulate;

fn out(src: &str) -> String {
    // Large max-time: the timescale case delays 5us (= 5e6 ps ticks).
    let sim = simulate(src, 10_000_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect::<Vec<_>>().join("\n")
}

#[test]
fn nested_hierarchical_task_call_resolves() {
    let o = out(r#"
module top; mid m(); initial #5 m.go(); endmodule
module mid; leaf l(); task go; $display("N1 mid"); l.deep(); endtask endmodule
module leaf; task deep; $display("N2 leaf"); endtask endmodule
"#);
    assert!(o.contains("N1 mid"), "outer cross-module task must run; got: {}", o);
    assert!(o.contains("N2 leaf"), "nested cross-module task must resolve + run; got: {}", o);
}

/// The nested callee's `$realtime` uses its OWN module timescale (leaf = 1ps).
#[test]
fn nested_call_uses_callee_timescale() {
    let o = out(r#"
`timescale 1us/1ns
module top; mid m(); initial #5 m.go(); endmodule
`timescale 1ns/1ps
module mid; leaf l(); task go; l.deep(); endtask endmodule
`timescale 1ps/1ps
module leaf; task deep; $display("RT=%0g", $realtime); endtask endmodule
"#);
    // t = 5us = 5_000_000 ps; leaf reports in its own 1ps unit.
    assert!(o.contains("RT=5e+06"),
        "nested callee must report in its own 1ps unit (5e+06); got: {}", o);
}
