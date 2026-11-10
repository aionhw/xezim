// #943: fork children whose block local has the name of a local of their
// parent task, next to two live frames of another process that declare the
// same name (`poison_outer` / `poison_inner`). The cousin kinds differ in the
// local's type (int, unpacked and packed struct, string, class handle) and in
// what wakes the child (edge, named event, mailbox). Kinds 0 and 9 use a
// different name. `+EDG=<n>` sets the clock edges per kind.
typedef struct        { int a, b, c, d; } payload_t;
typedef struct packed { logic [7:0] x, y, z, w; } geom_t;
class pkt_c; int f; endclass

module tab_cell;
  payload_t gp0, gp1; geom_t gw0, gw1; int gi0;
endmodule

module top;
  localparam int NKIND = 10;
  localparam int NR = 4;
  int EDG;
  logic clk0 = 0, clk1 = 0, clk2 = 0, clk3 = 0, clk4 = 0;
  logic clk5 = 0, clk6 = 0, clk7 = 0, clk8 = 0, clk9 = 0;
  bit run0, run1, run2, run3, run4, run5, run6, run7, run8, run9;
  int hits_k [NKIND];
  int sum_k [NKIND];
  event ev6;
  mailbox mb7 = new();
  bit poison_done;

  for (genvar g = 0; g < 32; g++) begin : gtab
    tab_cell u_cell();
  end

  // Two live frames of one process declare `payload`.
  task automatic poison_inner();
    payload_t payload;
    payload.a = 7;
    #3000;
    $display("T|poison inner a=%0d b=%0d", payload.a, payload.b);
  endtask
  task automatic poison_outer();
    payload_t payload;
    payload.b = 3;
    poison_inner();
    $display("T|poison outer a=%0d b=%0d", payload.a, payload.b);
    poison_done = 1;
  endtask

  task automatic k0_parent(int k);
    int payload;
    payload = k;
    fork begin
      int payload_b;
      payload_b = k;
      forever begin @(posedge clk0); hits_k[0]++; sum_k[0] += payload_b; end
    end join_none
  endtask
  task automatic k1_parent(int k);
    payload_t payload;
    payload.a = k;
    fork begin
      payload_t payload;
      payload.b = k;
      forever begin @(posedge clk1); hits_k[1]++; sum_k[1] += payload.a + payload.b; end
    end join_none
  endtask
  task automatic k2_parent(int k);
    geom_t payload;
    payload.x = k[7:0];
    fork begin
      geom_t payload;
      payload.y = k[7:0];
      forever begin
        @(posedge clk2);
        hits_k[2]++;
        sum_k[2] += ($isunknown(payload.x) ? payload.y : 100);
      end
    end join_none
  endtask
  task automatic k3_parent(int k);
    string payload;
    payload = "parent";
    fork begin
      string payload;
      payload = "child";
      forever begin @(posedge clk3); hits_k[3]++; sum_k[3] += payload.len(); end
    end join_none
  endtask
  task automatic k4_parent(int k);
    pkt_c payload;
    payload = new();
    payload.f = k;
    fork begin
      pkt_c payload;
      payload = new();
      payload.f = k + 1;
      forever begin @(posedge clk4); hits_k[4]++; sum_k[4] += payload.f; end
    end join_none
  endtask
  task automatic k5_leaf(int k);
    payload_t payload;
    payload.c = k;
    forever begin @(posedge clk5); hits_k[5]++; sum_k[5] += payload.c + payload.a; end
  endtask
  task automatic k5_mid(int k);
    payload_t payload;
    payload.b = k;
    k5_leaf(k);
  endtask
  task automatic k5_parent(int k);
    payload_t payload;
    payload.a = k;
    fork begin k5_mid(k); end join_none
  endtask
  task automatic k6_parent(int k);
    payload_t payload;
    payload.a = k;
    fork begin
      payload_t payload;
      payload.b = k;
      forever begin @(ev6); hits_k[6]++; sum_k[6] += payload.b; end
    end join_none
  endtask
  task automatic k7_parent(int k);
    payload_t payload;
    payload.a = k;
    fork begin
      payload_t payload;
      int v;
      payload.b = k;
      forever begin mb7.get(v); hits_k[7]++; sum_k[7] += payload.b + v; end
    end join_none
  endtask
  task automatic k8_parent(int k);
    int payload;
    payload = k;
    fork begin
      int payload;
      payload = k + 1;
      forever begin @(posedge clk8); hits_k[8]++; sum_k[8] += payload; end
    end join_none
  endtask
  task automatic k9_parent(int k);
    payload_t payload;
    payload.a = k;
    fork begin
      payload_t payload_b9;
      payload_b9.b = k;
      forever begin @(posedge clk9); hits_k[9]++; sum_k[9] += payload_b9.b; end
    end join_none
  endtask

  initial forever begin #2; if (run0) clk0 = ~clk0; end
  initial forever begin #2; if (run1) clk1 = ~clk1; end
  initial forever begin #2; if (run2) clk2 = ~clk2; end
  initial forever begin #2; if (run3) clk3 = ~clk3; end
  initial forever begin #2; if (run4) clk4 = ~clk4; end
  initial forever begin #2; if (run5) clk5 = ~clk5; end
  initial forever begin #2; if (run6) clk6 = ~clk6; end
  initial forever begin #2; if (run7) clk7 = ~clk7; end
  initial forever begin #2; if (run8) clk8 = ~clk8; end
  initial forever begin #2; if (run9) clk9 = ~clk9; end
  initial forever begin @(posedge clk6); -> ev6; end
  initial forever begin
    @(posedge clk7);
    for (int q = 0; q < NR; q++) void'(mb7.try_put(2));
  end

  initial poison_outer();

  initial begin
    if (!$value$plusargs("EDG=%d", EDG)) EDG = 10;
    for (int j = 0; j < NR; j++) begin
      k0_parent(j); k1_parent(j); k2_parent(j); k3_parent(j); k4_parent(j);
      k5_parent(j); k6_parent(j); k7_parent(j); k8_parent(j); k9_parent(j);
    end
    run0 = 1; repeat (EDG) @(posedge clk0); run0 = 0;
    run1 = 1; repeat (EDG) @(posedge clk1); run1 = 0;
    run2 = 1; repeat (EDG) @(posedge clk2); run2 = 0;
    run3 = 1; repeat (EDG) @(posedge clk3); run3 = 0;
    run4 = 1; repeat (EDG) @(posedge clk4); run4 = 0;
    run5 = 1; repeat (EDG) @(posedge clk5); run5 = 0;
    run6 = 1; repeat (EDG) @(posedge clk6); run6 = 0;
    run7 = 1; repeat (EDG) @(posedge clk7); run7 = 0; #10;
    run8 = 1; repeat (EDG) @(posedge clk8); run8 = 0;
    run9 = 1; repeat (EDG) @(posedge clk9); run9 = 0;
    wait (poison_done);
    for (int kind = 0; kind < NKIND; kind++)
      $display("T|kind %0d hits=%0d sum=%0d", kind, hits_k[kind], sum_k[kind] / EDG);
    $finish;
  end
endmodule
