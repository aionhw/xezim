// Part 5 synthetic: minimal task-scope sensitivity bug repro.
// Shows that assigning to a top-level reg from INSIDE a task body
// (instead of inline in initial) breaks sensitivity-event propagation
// to always blocks watching bit-slices of wires derived from that reg.
//
// iverilog outputs:
//   INST=00000013 rd16=01000   (initial value falling into default arm)
//   INST=02110191 rd16=00011   (matches 5'b00001 arm → inst[11:7] = 5'd3)
//   INST=12f10211 rd16=00100
//
// xezim outputs:
//   INST=00000013 rd16=01000   ← fires once at sim start
//   INST=02110191 rd16=01000   ← STUCK; always block didn't refire
//   INST=12f10211 rd16=01000
//
// Without the task wrapper (assigning `inst = X` directly in the
// initial block) xezim updates rd_16 correctly. The bug is in how
// task-scope blocking-assigns to upper-scope regs propagate the
// edge event to bit-slice sensitivity lists in always blocks.
`timescale 1ns/100ps
module top;
  reg [31:0] inst = 32'h00000013;
  wire [31:0] decd_inst = inst;
  reg [4:0] rd_16;
  always @( decd_inst[15:13]
         or decd_inst[11:0])
  begin
    casez({decd_inst[15:13], decd_inst[1:0]})
      5'b01010, 5'b01001, 5'b01101, 5'b00001, 5'b00010:
        rd_16[4:0] = decd_inst[11:7];
      default:
        rd_16[4:0] = {2'b01, decd_inst[9:7]};
    endcase
  end

  task probe(input [31:0] insn);
    begin
      inst = insn;
      #1;
      $display("INST=%h rd16=%b", insn, rd_16);
    end
  endtask

  initial begin
    #1;
    probe(32'h00000013);
    probe(32'h02110191);
    probe(32'h12f10211);
    $finish;
  end
endmodule
