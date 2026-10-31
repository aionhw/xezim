`timescale 1ns/1ps
module tb_tlul;
  reg clk = 0;
  always #5 clk = ~clk;
  reg rst_n = 0;
  reg a_valid = 0;
  wire a_ready;
  reg [2:0] a_opcode=0, a_size=2;
  reg [3:0] a_source=0, a_mask=15;
  reg [15:0] a_address=0;
  reg [31:0] a_data=0;
  wire d_valid;
  reg d_ready_driver=0;
  wire d_ready;
  assign d_ready = d_ready_driver;
  wire [2:0] d_opcode, d_size;
  wire [3:0] d_source;
  wire [31:0] d_data;
  wire d_error;
  reg throttle_driver=0;
  wire request_throttle;
  wire inert_net;
  reg inert_driver=0;
  assign request_throttle = throttle_driver;
  assign inert_net = inert_driver;
  reg [63:0] stimulus [0:511];
  reg [63:0] current_word;
  reg [15:0] busy;
  integer issued=0, retired=0, cycle=0;
  integer nreq=96, seed=123, ctrl_mode=0, ctrl_at=10, ctrl_len=12;
  reg [1023:0] stimpath;
  integer idx;

  tlul_mem_endpoint dut (
    .clk(clk), .rst_n(rst_n), .request_throttle(request_throttle),
    .a_valid(a_valid), .a_ready(a_ready), .a_opcode(a_opcode),
    .a_size(a_size), .a_source(a_source), .a_address(a_address),
    .a_mask(a_mask), .a_data(a_data),
    .d_valid(d_valid), .d_ready(d_ready), .d_opcode(d_opcode),
    .d_size(d_size), .d_source(d_source), .d_data(d_data), .d_error(d_error)
  );

  initial begin
    if ($test$plusargs("VCD")) begin
      $dumpfile("wave.vcd");
      $dumpvars(0, tb_tlul);
    end
  end

  initial begin
    busy=0;
    if (!$value$plusargs("STIM=%s", stimpath)) begin
      $display("TB_ERROR|missing STIM plusarg"); $fatal(1);
    end
    if (!$value$plusargs("NREQ=%d", nreq)) nreq=96;
    if (!$value$plusargs("SEED=%d", seed)) seed=123;
    if (!$value$plusargs("CTRL_OP=%d", ctrl_mode)) ctrl_mode=0;
    if (!$value$plusargs("CTRL_AT=%d", ctrl_at)) ctrl_at=10;
    if (!$value$plusargs("CTRL_LEN=%d", ctrl_len)) ctrl_len=12;
    if (nreq<1 || nreq>512) $fatal(1,"invalid nreq");
    $readmemh(stimpath, stimulus);
    repeat (5) @(negedge clk);
    rst_n=1;
  end

  // Independent, fixed-timing experiment harness. ctrl_mode=1 is an A-channel
  // throttle, ctrl_mode=2 inert wire, ctrl_mode=3 idempotent zero throttle.
  // The *agent* must choose target, time and duration via policy API; this
  // harness does not choose a fault-specific experiment or prediction.
  initial begin
    wait(rst_n);
    if (ctrl_mode > 0) begin
      repeat (ctrl_at) @(negedge clk);
      if (ctrl_mode == 1) force request_throttle = 1'b1;
      if (ctrl_mode == 2) force inert_net = 1'b1;
      if (ctrl_mode == 3) force request_throttle = 1'b0;
      if (ctrl_mode == 4) force d_ready = 1'b0;
      $display("CONTROL|phase=force|mode=%0d|cycle=%0d", ctrl_mode, cycle);
      repeat (ctrl_len) @(negedge clk);
      if (ctrl_mode == 1 || ctrl_mode == 3) release request_throttle;
      if (ctrl_mode == 2) release inert_net;
      if (ctrl_mode == 4) release d_ready;
      $display("CONTROL|phase=release|mode=%0d|cycle=%0d", ctrl_mode, cycle);
    end
  end

  always @(negedge clk) begin
    if (rst_n) begin
      // A fields and valid are held stable until a handshake.
      if (!a_valid && issued<nreq) begin
        current_word=stimulus[issued];
        if (!busy[current_word[60:57]]) begin
          a_opcode = current_word[63:61];
          a_source = current_word[60:57];
          a_size = current_word[56:54];
          a_address = current_word[53:38];
          a_mask = current_word[37:34];
          a_data = current_word[33:2];
          a_valid = 1;
        end
      end
      // A patterned legal stall source; independent of the DUT.
      d_ready_driver = ((cycle % 23) < 13) && ((cycle % 41) != 10);
    end else begin
      a_valid=0;
      d_ready_driver=0;
    end
  end

  always @(posedge clk) begin
    if (rst_n) begin
`ifdef DVCON_NATIVE
      // Native research observer must be explicitly ported and validated.
      // Literal strings only; capture phase is this testbench call, not a
      // simulator-wide write or scheduler-region provenance claim.
      $dvcon_observe("sample", "tlul");
      if (a_valid && a_ready) $dvcon_observe("accepted_a", "tlul");
      if (d_valid && d_ready) $dvcon_observe("accepted_d", "tlul");
`endif
      cycle=cycle+1;
      $display("PIN|cycle=%0d|av=%0d|ar=%0d|aop=%0d|asz=%0d|asrc=%0d|addr=%0d|mask=%0d|adata=%0d|dv=%0d|dr=%0d|dop=%0d|dsz=%0d|dsrc=%0d|ddata=%0d|derr=%0d",
        cycle,a_valid,a_ready,a_opcode,a_size,a_source,a_address,a_mask,a_data,
        d_valid,d_ready,d_opcode,d_size,d_source,d_data,d_error);
      if (a_valid && a_ready) begin
        busy[a_source] <= 1'b1;
        issued <= issued+1;
        a_valid <= 0;
      end
      if (d_valid && d_ready) begin
        busy[d_source] <= 1'b0;
        retired <= retired+1;
      end
      if (issued == nreq && retired == nreq && !d_valid && busy == 0) begin
        $display("SIM_END|issued=%0d|retired=%0d|drained=1|cycle=%0d", issued,retired,cycle);
`ifdef DVCON_NATIVE
        $dvcon_observe("finish", "tlul");
`endif
        $finish;
      end
      if (cycle > 3200) begin
        $display("SIM_END|issued=%0d|retired=%0d|drained=0|cycle=%0d", issued,retired,cycle);
`ifdef DVCON_NATIVE
        $dvcon_observe("finish", "tlul");
`endif
        $finish;
      end
    end
  end
endmodule
