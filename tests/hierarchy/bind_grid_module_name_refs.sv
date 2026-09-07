`timescale 1ns/1ps

typedef bit [6:0] u7_t ;

module leaf #(
  parameter int ROW_ID = 0,
  parameter int COL_ID = 0
)(
  input  logic clk,
  input  logic rst_n,
  input  logic in_vld,
  output logic out_vld
);

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n)
      out_vld <= 0;
    else
      out_vld <= in_vld;
  end

endmodule

bind leaf leaf_harness v_leaf_harness();
module leaf_harness;
  
  `define PMOD leaf
  `define TLHRN dut.v_tl_harness

  initial begin
    `TLHRN.req_count[leaf.ROW_ID][leaf.COL_ID] = 0;
    `TLHRN.rsp_count[leaf.ROW_ID][leaf.COL_ID] = 0;
    
    forever @(posedge `PMOD.clk) begin
      `TLHRN.req_count[leaf.ROW_ID][leaf.COL_ID] += `PMOD.in_vld ;
      `TLHRN.rsp_count[leaf.ROW_ID][leaf.COL_ID] += `PMOD.out_vld ;
    end
  end
  
  `undef PMOD
  `undef TLHRN

endmodule

module tile #(
  parameter int ROW_ID = 0,
  parameter int COL_ID = 0
)(
  input  logic clk,
  input  logic rst_n,
  input  logic in_vld,
  output logic out_vld
);

  leaf #(
    .ROW_ID(ROW_ID),
    .COL_ID(COL_ID)
  ) u_leaf (
    .clk    (clk),
    .rst_n  (rst_n),
    .in_vld (in_vld),
    .out_vld(out_vld)
  );

endmodule


module cluster #(
  parameter int NUM_ROWS = 2,
  parameter int NUM_COLS = 2
)(
  input  logic clk,
  input  logic rst_n,
  input  logic [NUM_ROWS-1:0][NUM_COLS-1:0] in_vld,
  output logic [NUM_ROWS-1:0][NUM_COLS-1:0] out_vld
);

  genvar r,c;

  generate
    for (r=0; r<NUM_ROWS; r++) begin : G_ROW
      for (c=0; c<NUM_COLS; c++) begin : G_COL

        tile #(
          .ROW_ID(r),
          .COL_ID(c)
        ) u_tile (
          .clk    (clk),
          .rst_n  (rst_n),
          .in_vld (in_vld[r][c]),
          .out_vld(out_vld[r][c])
        );

      end
    end
  endgenerate

endmodule


module dut #(
  parameter int NUM_ROWS = 2,
  parameter int NUM_COLS = 2
)(
  input  logic clk,
  input  logic rst_n,
  input  logic [NUM_ROWS-1:0][NUM_COLS-1:0] req,
  output logic [NUM_ROWS-1:0][NUM_COLS-1:0] rsp
);

  cluster #(
    .NUM_ROWS(NUM_ROWS),
    .NUM_COLS(NUM_COLS)
  ) u_cluster (
    .clk    (clk),
    .rst_n  (rst_n),
    .in_vld (req),
    .out_vld(rsp)
  );

endmodule

bind dut  dut_harness #(.NUM_ROWS(NUM_ROWS),.NUM_COLS(NUM_COLS)) v_tl_harness (.*);

module dut_harness #(parameter NUM_ROWS = 2, NUM_COLS = 2);
  
  `define PMOD dut
  
  int req_count [NUM_ROWS][NUM_COLS];
  int rsp_count [NUM_ROWS][NUM_COLS];  
  
  //----------------------------------------------------------
  // Self-checking scoreboard
  //----------------------------------------------------------

  task automatic run_checks(ref u7_t [NUM_ROWS-1:0][NUM_COLS-1:0] desired_req_rsp_count);

    int r,c;

    for (r=0; r< NUM_ROWS; r++) begin
      for (c=0; c< NUM_COLS; c++) begin

        if (req_count[r][c] != desired_req_rsp_count[r][c])
          $fatal(1,
                 "%0d: req_count[%0d][%0d] exp=%0d got=%0d", $time,
                 r,c,desired_req_rsp_count[r][c],req_count[r][c]);

        if (rsp_count[r][c] != desired_req_rsp_count[r][c])
          $fatal(1,
                 "%0d: rsp_count[%0d][%0d] exp=%0d got=%0d", $time,
                 r,c,desired_req_rsp_count[r][c],rsp_count[r][c]);

      end
    end

  endtask

  `undef PMOD

endmodule

module tb;

  localparam int NUM_ROWS = 5;
  localparam int NUM_COLS = 2;
  
  localparam int NUM_CYCLES = 100 ;

  logic clk;
  logic rst_n;

  logic [NUM_ROWS-1:0][NUM_COLS-1:0] req;
  logic [NUM_ROWS-1:0][NUM_COLS-1:0] rsp;
  
  u7_t [NUM_ROWS-1:0][NUM_COLS-1:0] desired_req_rsp_count ;  
  bit [NUM_CYCLES-1:0] active_req_cycle_mask[NUM_ROWS][NUM_COLS];

  //----------------------------------------------------------
  // DUT
  //----------------------------------------------------------

  dut #(
    .NUM_ROWS(NUM_ROWS),
    .NUM_COLS(NUM_COLS)
  ) u_dut (
    .clk (clk),
    .rst_n(rst_n),
    .req (req),
    .rsp (rsp)
  );

  initial begin
    clk = 0;
    forever #5 clk = ~clk;
  end

  //----------------------------------------------------------
  // Stimulus
  //----------------------------------------------------------

  initial begin

    rst_n = 0;
    req   = '0;
    
    void'(std::randomize(desired_req_rsp_count, active_req_cycle_mask) with {
      foreach (desired_req_rsp_count[i,j]) {
        desired_req_rsp_count[i][j] < NUM_CYCLES ;
        $countones(active_req_cycle_mask[i][j]) == desired_req_rsp_count[i][j] ;
      }
    });
      foreach (desired_req_rsp_count[i,j]) begin
        $display("%0d: desired [%0d][%0d] : %0d :: over : %b", $time, i,j,desired_req_rsp_count[i][j], active_req_cycle_mask[i][j]);
      end

    repeat(2) @(posedge clk);

    rst_n = 1;

    for (int cctr = 0; cctr < NUM_CYCLES; cctr++) begin
      for (int r = 0; r < NUM_ROWS; r++) begin
        for (int c = 0; c < NUM_COLS; c++) begin
          req[r][c] <= active_req_cycle_mask[r][c][cctr] ;
        end
      end
      @(posedge clk);
      req <= '0 ;
    end

    repeat(3) @(posedge clk);

    u_dut.v_tl_harness.run_checks(desired_req_rsp_count);

    $display("");
    $display("==================================");
    $display("TEST PASSED");
    $display("==================================");
    $display("");

    $finish;

  end


endmodule
