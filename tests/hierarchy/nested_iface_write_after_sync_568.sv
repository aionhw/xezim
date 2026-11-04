// IEEE 1800-2017 10.4.1 / 23.6 / 25.9: a blocking assignment through a hierarchical
// name, or through a virtual interface bound to the variable, updates the variable
// before the next statement runs. Here the variable is in an interface instance
// nested two levels down (w.m.lf). Each check starts from a known value (set with a
// nonblocking assignment) and does one blocking write.
//
//   xezim --simulate -s t repro.sv                   a $monitor is active
//   xezim --simulate -s t +define+READMEM repro.sv   a $readmemh into a class-task local array
//   xezim --simulate -s t +define+NONE repro.sv      neither (control)
//
// Every line should print PASS in all three runs.
interface leaf_if;
  logic [7:0] r;
  logic       b;
endinterface
interface mid_if;
  leaf_if lf();
endinterface
interface wrap_if;
  mid_if m();
endinterface

class driver;
  virtual leaf_if v;
  task whole;    v.r = 8'ha5;     endtask
  task scalar;   v.b = 1'b1;      endtask
  task compound; v.r += 8'h01;    endtask
  task incr;     v.r++;           endtask
  task partsel;  v.r[7:4] = 4'ha; endtask
  task bitsel;   v.r[0] = 1'b0;   endtask
  task load;                      // $readmemh into a task-local array
    logic [7:0] pm [0:3];
    $readmemh("repro_mem.hex", pm, 0, 3);
  endtask
endclass

module t;
  wrap_if w();
  driver  c;
  int     fails = 0;

  task automatic chk(string what, logic [7:0] got, logic [7:0] exp);
    if (got === exp) $display("PASS %s", what);
    else begin $display("FAIL %s: got %b, expected %b", what, got, exp); fails++; end
  endtask
  task automatic base;                 // known starting value: r = 0f, b = 0
    w.m.lf.r <= 8'h0f; w.m.lf.b <= 1'b0; #1;
  endtask

`ifndef READMEM
`ifndef NONE
  initial $monitor("monitor at %0t", $time);
`endif
`endif

  initial begin
    int f;
    f = $fopen("repro_mem.hex", "w"); $fdisplay(f, "01\n02\n03\n04"); $fclose(f);
    c = new;
    c.v = w.m.lf;                       // 3-segment name
`ifdef READMEM
    c.load;
`endif
    #1;
    base; w.m.lf.r = 8'h55;       chk("w.m.lf.r = value",       w.m.lf.r, 8'h55);
    base; w.m.lf.b = 1'b1;        chk("w.m.lf.b = value",       w.m.lf.b, 8'h01);
    base; w.m.lf.r += 8'h01;      chk("w.m.lf.r += 1",          w.m.lf.r, 8'h10);
    base; w.m.lf.r++;             chk("w.m.lf.r++",             w.m.lf.r, 8'h10);
    base; w.m.lf.r[7:4] = 4'ha;   chk("w.m.lf.r[7:4] = value",  w.m.lf.r, 8'haf);
    base; w.m.lf.r[0] = 1'b0;     chk("w.m.lf.r[0] = value",    w.m.lf.r, 8'h0e);
    base; c.whole;                chk("vif.r = value",          w.m.lf.r, 8'ha5);
    base; c.scalar;               chk("vif.b = value",          w.m.lf.b, 8'h01);
    base; c.compound;             chk("vif.r += 1",             w.m.lf.r, 8'h10);
    base; c.incr;                 chk("vif.r++",                w.m.lf.r, 8'h10);
    base; c.partsel;              chk("vif.r[7:4] = value",     w.m.lf.r, 8'haf);
    base; c.bitsel;               chk("vif.r[0] = value",       w.m.lf.r, 8'h0e);
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
