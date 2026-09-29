`include "../common/svtest_defs.svh"

// §14.4/§14.16 output skew + §25.9 virtual-interface clocking + §5.8 time
// literals. A drive executed AT the clocking event matures at edge + output
// skew (not the next edge); a #0-skew drive lands in the same timestep; a
// vif-aliased clocking block routes to the concrete interface; and a time
// literal compares exactly against a $time delta. (Condensed from an
// external regression that exercised all of these.)
`timescale 1ns/1ps
interface skb_if(input bit clk);
  logic       vld;
  logic       rdy;
  default clocking cbm @(posedge clk);
    default input #2ns output #1ns;
    output vld;
  endclocking
  clocking cbf @(negedge clk);
    output #0 rdy;
  endclocking
endinterface

module test_clocking_skew_vif;
  `SVTEST_INIT

  bit clk = 0;
  always #5ns clk = ~clk;
  skb_if u_if(.clk(clk));

  initial begin
    u_if.vld = 1'b0;
    u_if.rdy = 1'b1;
  end

  time t0, t1;
  initial begin
    // §14.16.1: drive at the clocking event matures at edge + #1ns.
    @(u_if.cbm);                       // edge at 5ns
    u_if.cbm.vld <= 1'b1;
    #500ps;
    `SVTEST_CHECK(u_if.vld === 1'b0, "output skew window opened early")
    #1000ps;
    `SVTEST_CHECK(u_if.vld === 1'b1, "output skew drive did not mature")

    // §5.8: time-literal equality against a $time delta.
    @(u_if.cbm); t0 = $time;
    repeat (3) @(u_if.cbm); t1 = $time;
    `SVTEST_CHECK((t1 - t0) == 30000ps, "time literal != $time delta")

    // #0 output skew on a negedge block lands in the same timestep.
    @(u_if.cbf);
    u_if.cbf.rdy <= 1'b0;
    #1ps;
    `SVTEST_CHECK(u_if.rdy === 1'b0, "negedge #0 output skew drive missing")

    // §25.9: vif alias routes clocking drive + event to the concrete iface.
    begin
      virtual skb_if vif;
      vif = u_if;
      vif.cbm.vld <= 1'b0;
      @(vif.cbm);
      #2ns;
      `SVTEST_CHECK(u_if.vld === 1'b0, "vif clocking drive/event not routed")
    end

    `SVTEST_PASSFAIL
    $finish;
  end
endmodule
