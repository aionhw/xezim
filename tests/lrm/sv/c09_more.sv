// top: c09x
module c09x;
  // 9.4.2 edge definitions: 0->x, x->1, z->1, 1->z etc.
  logic s = 0;
  int pe = 0, ne = 0;
  always @(posedge s) pe++;
  always @(negedge s) ne++;
  // @* with array element and part select
  logic [7:0] mem [4];
  logic [1:0] idx = 0;
  logic [7:0] rd_star, rd_comb;
  always @* rd_star = mem[idx];
  always_comb rd_comb = mem[idx];
  // always_comb with blocking temp and multiple assignments
  logic [3:0] in1 = 0, o1;
  always_comb begin o1 = 0; if (in1[0]) o1 = in1; end
  // event control on struct member and on whole array
  typedef struct packed {logic [3:0] a; logic [3:0] b;} ab_t;
  ab_t sab = 0; int sab_cnt = 0;
  always @(sab.a) sab_cnt++;
  int arr_cnt = 0;
  always @(mem[2]) arr_cnt++;
  // @(*) with function call args
  function automatic logic [3:0] inv(logic [3:0] x); return ~x; endfunction
  logic [3:0] fi = 0, fo;
  always @(*) fo = inv(fi);
  // always_ff with async reset
  logic clk = 0, rst_n = 1; logic [3:0] cnt;
  always_ff @(posedge clk or negedge rst_n) if (!rst_n) cnt <= 0; else cnt <= cnt + 1;
  // sensitivity to real
  real rv = 0; int rcnt = 0;
  always @(rv) rcnt++;
  int lt_done = 0;
  // wait on expression with x
  initial begin
    #1 s = 1'bx; #1 s = 1; #1 s = 1'bz; #1 s = 0; #1 s = 1'bz; #1 s = 1; #1 s = 1'bx; #1 s = 0;
    #1 $display("T|9.4.2.1|pe=%0d ne=%0d", pe, ne);
    mem[0] = 8'h10; mem[1] = 8'h11; mem[2] = 8'h12; mem[3] = 8'h13; #1;
    $display("T|9.2.2.2d|%h %h", rd_star, rd_comb);
    idx = 2; #1 $display("T|9.2.2.2e|%h %h", rd_star, rd_comb);
    mem[2] = 8'h22; #1 $display("T|9.2.2.2f|%h %h", rd_star, rd_comb);
    in1 = 4'h3; #1 $display("T|9.2.2.2g|%h", o1); in1 = 4'h2; #1 $display("T|9.2.2.2h|%h", o1);
    sab.b = 4'h1; #1 sab.a = 4'h2; #1 sab = 8'h22; #1 $display("T|9.4.2e|sab_cnt=%0d", sab_cnt);
    arr_cnt = 0; mem[1] = 0; #1 mem[2] = 8'h33; #1 $display("T|9.4.2f|arr_cnt=%0d", arr_cnt);
    fi = 4'h5; #1 $display("T|9.2.2.1|fo=%h", fo);
    #1 clk = 1; #1 clk = 0; #1 clk = 1; #1 clk = 0; rst_n = 0; #1 $display("T|9.2.2.4b|cnt=%0d", cnt); rst_n = 1;
    #1 clk = 1; #1 $display("T|9.2.2.4c|cnt=%0d", cnt);
    rv = 1.5; #1 rv = 1.5; #1 rv = 2.0; #1 $display("T|9.4.2g|rcnt=%0d", rcnt);
    // 9.4.3 wait with already-true condition: no delay
    begin time t0; t0 = $time; wait (1); $display("T|9.4.3b|dt=%0t", $time - t0); end
    // 9.4.4 level-sensitive sequence wait - skip; 9.5 process execution threads: fork in function
    // 9.6.2 disable of a task from outside
    fork
      begin : tsk_blk long_task(); end
      begin #2 disable long_task; end
    join
    $display("T|9.6.2b|after disable task t=%0t done=%0d", $time % 100, lt_done);
    // disable of a block from inside nested fork in a loop
    begin int k; for (k = 0; k < 10; k++) begin : body if (k == 3) disable body; if (k == 5) break; end $display("T|9.6.2c|k=%0d", k); end
  end
  task long_task; #5 lt_done = 1; endtask
endmodule
