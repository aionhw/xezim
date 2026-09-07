//! `#delay` inside a package class method or a `$unit` task was never
//! pre-scaled by the timescale in effect (only module items and module-level
//! classes were), so under `timescale 1ns/1ps it was read as 200 raw picosecond
//! ticks and rounded to zero by the precision fit.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 10_000_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn package_class_and_unit_task_delays_use_the_timescale() {
    let msgs = messages(
        "`timescale 1ns/1ps
package p; class PB; task dly(); #200; endtask endclass
  task pkg_task(); #100; endtask endpackage
task unit_dly(); #50; endtask
class UC; task dly(); #25; endtask endclass
package pu; timeunit 1us; class PU; task dly(); #3; endtask endclass endpackage
program prg; initial begin #7; $display(\"program=%0t\", $time); end endprogram
module tb; import p::*; import pu::*; PU pu_o = new(); prg p_i();
  class MB; task dly(); #200; endtask endclass
  PB pb = new(); MB mb = new(); UC uc = new();
  initial begin
    pb.dly();   $display(\"pkg_class=%0t\", $time);
    pkg_task(); $display(\"pkg_task=%0t\", $time);
    unit_dly(); $display(\"unit_task=%0t\", $time);
    uc.dly();   $display(\"unit_class=%0t\", $time);
    mb.dly();   $display(\"mod_class=%0t\", $time);
    pu_o.dly(); $display(\"pkg_timeunit=%0t\", $time);
    $finish;
  end
endmodule",
    );
    for want in ["pkg_class=200000", "pkg_task=300000", "unit_task=350000", "unit_class=375000", "mod_class=575000", "pkg_timeunit=3575000", "program=7000"] {
        assert!(msgs.iter().any(|m| m == want), "missing {want}: {msgs:?}");
    }
}
