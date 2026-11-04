// IEEE 1800-2017 3.14.2: a delay counts the time unit of the scope it is written
// in, and -override_timescale <unit>/<prec> gives EVERY scope (module, package,
// compilation unit) that unit. So a bare `#868` advances 868 time units in every
// scope below, and `$realtime` in the module reads 868.0 after each one.
// Run with and without -override_timescale=10ns/1ns (and 1ns/1ps); every line
// should print PASS each time.
`timescale 10ns/1ns

// compilation-unit scope ($unit)
task automatic unit_task(); #868; endtask
class unit_cls;
  task run(); #868; endtask
endclass

// package scope
package pkg;
  task automatic pkg_task(); #868; endtask
  class pkg_cls;
    task run(); #868; endtask
  endclass
endpackage

module t;
  class mod_cls;
    task run(); #868; endtask
  endclass

  int fails = 0;
  realtime t0;
  task automatic chk(string what);
    real dt = $realtime - t0;
    if (dt == 868.0) $display("PASS %s: advanced %0.1f units", what, dt);
    else begin $display("FAIL %s: advanced %0.1f units, expected 868.0", what, dt); fails++; end
  endtask

  initial begin
    unit_cls  uc = new;
    pkg::pkg_cls pc = new;
    mod_cls   mc = new;
    t0 = $realtime; unit_task();       chk("$unit task");
    t0 = $realtime; uc.run();          chk("$unit class method");
    t0 = $realtime; pkg::pkg_task();   chk("package task");
    t0 = $realtime; pc.run();          chk("package class method");
    t0 = $realtime; mc.run();          chk("module class method");
    t0 = $realtime; #868;              chk("module");
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
