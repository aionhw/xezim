// IEEE 1800-2017 25.9 / 23.6: `outer.leaf` names the interface instance `leaf`
// nested inside the interface instance `outer`, so a virtual interface variable
// assigned `outer.leaf` refers to that instance. Waits, reads and writes through
// it must behave exactly as through a variable bound to a top-level instance.
// Every line should print PASS.
`timescale 1ns/1ns

interface leaf_if;
  bit         clk;
  logic       en;
  logic [7:0] d;
  always #2 clk <= ~clk;     // negedges at 4, 8, 12, 16, ...
endinterface

interface outer_if;
  leaf_if leaf();
endinterface

class watcher;
  virtual leaf_if vif;
  time t_en = 0, t_clk = 0, t_d = 0;
  task run();
    @(posedge vif.en);        t_en  = $time;
    repeat (4) @(negedge vif.clk);  t_clk = $time;
    wait (vif.d == 8'h5a);    t_d   = $time;
  endtask
endclass

module t;
  outer_if o();              // nested instance: o.leaf
  leaf_if  flat();           // top-level instance, for comparison
  watcher  wn = new, wf = new;
  int fails = 0;

  task automatic chk(string what, time got, time exp);
    if (got === exp) $display("PASS %s", what);
    else begin $display("FAIL %s: got %0t, expected %0t", what, got, exp); fails++; end
  endtask
  task automatic chkv(string what, logic [7:0] got, logic [7:0] exp);
    if (got === exp) $display("PASS %s", what);
    else begin $display("FAIL %s: got %h, expected %h", what, got, exp); fails++; end
  endtask

  initial begin
    o.leaf.en = 0;  flat.en = 0;
    o.leaf.d  = 0;  flat.d  = 0;
    wn.vif = o.leaf;                    // bound to the nested instance
    wf.vif = flat;                      // bound to a top-level instance
    fork wn.run(); wf.run(); join_none
    #13 o.leaf.en = 1; flat.en = 1;     // posedge en at 13
    #10 o.leaf.d = 8'h5a; flat.d = 8'h5a;  // at 23; the 4th negedge clk after 13 is at 28
    #30;
    chk("flat:   @(posedge vif.en) returns at 13",      wf.t_en,  13);
    chk("flat:   4 x @(negedge vif.clk) done at 28",    wf.t_clk, 28);
    chk("flat:   wait (vif.d == 5a) done at 28",        wf.t_d,   28);
    chk("nested: @(posedge vif.en) returns at 13",      wn.t_en,  13);
    chk("nested: 4 x @(negedge vif.clk) done at 28",    wn.t_clk, 28);
    chk("nested: wait (vif.d == 5a) done at 28",        wn.t_d,   28);
    chkv("nested: read through vif sees o.leaf.d",      wn.vif.d, o.leaf.d);
    wn.vif.d = 8'ha5;
    chkv("nested: write through vif reaches o.leaf.d",  o.leaf.d, 8'ha5);
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
