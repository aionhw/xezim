// Reads of packed-struct members below the first level, in a register-block shape.
package pmr_p;
  typedef struct packed { logic value; } done_t;
  typedef struct packed { done_t done; } fuse_wr_done_t;
  typedef struct packed { logic [7:0] value; } cnt_t;
  typedef struct packed { cnt_t count; } byte_count_t;
  typedef struct packed { fuse_wr_done_t FUSE_WR_DONE; byte_count_t BYTE_COUNT; } out_t;
  typedef struct packed { struct packed { logic signed [3:0] sv; logic [3:0] uv; } inner; logic [1:0] tag; } sg_t;
endpackage
module pmr_reg import pmr_p::*; (input logic clk, input logic [7:0] addr, input logic [7:0] d, output out_t hwif_out);
  typedef struct packed {
    logic FUSE_WR_DONE;
    struct packed { logic err_trig_r; logic notif_trig_r; } intr_block_rf;
  } decoded_reg_strb_t;
  decoded_reg_strb_t decoded_reg_strb;
  typedef struct packed {
    struct packed { struct packed { logic value; } done; } FUSE_WR_DONE;
    struct packed { struct packed { logic [7:0] value; } count; } BYTE_COUNT;
  } field_storage_t;
  field_storage_t field_storage;
  always_comb begin
    decoded_reg_strb.FUSE_WR_DONE = (addr == 8'h10);
    decoded_reg_strb.intr_block_rf.err_trig_r = (addr == 8'h20);
    decoded_reg_strb.intr_block_rf.notif_trig_r = (addr == 8'h24);
  end
  always_ff @(posedge clk) begin
    if (decoded_reg_strb.FUSE_WR_DONE) field_storage.FUSE_WR_DONE.done.value <= d[0];
    if (decoded_reg_strb.intr_block_rf.err_trig_r) field_storage.BYTE_COUNT.count.value <= d;
    if (decoded_reg_strb.intr_block_rf.notif_trig_r) field_storage.BYTE_COUNT.count.value <= ~d;
  end
  assign hwif_out.FUSE_WR_DONE.done.value = field_storage.FUSE_WR_DONE.done.value;
  assign hwif_out.BYTE_COUNT.count.value = field_storage.BYTE_COUNT.count.value;
endmodule
module pmr_blk import pmr_p::*; (input logic clk, input logic [7:0] addr, input logic [7:0] d,
                                 output logic done, output logic [7:0] cnt, output logic [9:0] ext,
                                 output logic [9:0] ext2);
  out_t hw_out;
  sg_t sg;
  pmr_reg u_reg (.clk, .addr, .d, .hwif_out(hw_out));
  assign done = hw_out.FUSE_WR_DONE.done.value;
  always_comb cnt = hw_out.BYTE_COUNT.count.value + 8'd1;
  always_comb begin
    sg = '0;
    sg.inner.sv = d[3:0];
    sg.inner.uv = d[7:4];
    // §11.8.1: the signed member sign-extends, the unsigned one does not.
    ext = sg.inner.sv + sg.inner.uv;
    ext2 = sg.inner.sv;
  end
endmodule
module pmr_check;
  import pmr_p::*;
  logic clk = 0; logic [7:0] addr, d; logic done; logic [7:0] cnt; logic [9:0] ext, ext2;
  sg_t sg;
  pmr_blk u (.clk, .addr, .d, .done, .cnt, .ext, .ext2);
  function automatic logic [3:0] local_read(input logic [7:0] x);
    sg_t sg;  // shadows the module's `sg`
    sg = '0;
    sg.inner.uv = x[3:0];
    return sg.inner.uv;
  endfunction
  logic [3:0] lr;
  always_comb lr = local_read(d);
  initial begin
    sg = '1;
    addr = 8'h10; d = 8'h01; #1 clk = 1; #1 clk = 0;
    addr = 8'h20; d = 8'h5a; #1 clk = 1; #1 clk = 0;
    #1 $display("a: done=%b cnt=%h ext=%h ext2=%h lr=%h strb=%b", done, cnt, ext, ext2, lr, u.u_reg.decoded_reg_strb);
    addr = 8'h24; d = 8'hc9; #1 clk = 1; #1 clk = 0;
    #1 $display("b: done=%b cnt=%h ext=%h ext2=%h lr=%h sg=%h", done, cnt, ext, ext2, lr, sg);
    $display("PACKED_MEMBER_PATH_READ_OK");
    $finish;
  end
endmodule
