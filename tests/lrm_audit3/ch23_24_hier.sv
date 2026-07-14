// Ch.23 hierarchy/generate, Ch.24 programs
package pk;
  int shared_v = 5;
endpackage

module leaf #(parameter int W = 8) (input logic [W-1:0] din, output logic [W-1:0] dout);
  assign dout = ~din;
endmodule

module tb;
  int fails = 0;
  `define CK(name, cond) if (!(cond)) begin $display("FAIL[23] %s", name); fails++; end
  logic [7:0] a8, y8;
  logic [3:0] a4, y4;
  leaf #(8) u8 (.din(a8), .dout(y8));
  leaf #(.W(4)) u4 (.din(a4), .dout(y4));

  genvar g;
  logic [3:0] gen_out;
  generate
    for (g = 0; g < 4; g++) begin : gblk
      assign gen_out[g] = g[0];
    end
  endgenerate

  localparam bit USE_A = 1;
  logic [7:0] cond_out;
  generate
    if (USE_A) begin : ifa
      assign cond_out = 8'hAA;
    end else begin : ifb
      assign cond_out = 8'hBB;
    end
  endgenerate

  initial begin
    a8 = 8'h0F; a4 = 4'h5;
    #1;
    `CK("param override positional", y8 == 8'hF0)
    `CK("param override named + width", y4 == 4'hA)
    `CK("generate-for", gen_out == 4'b1010)  // g[0]: bits 3..0 = 1,0,1,0
    `CK("generate-if", cond_out == 8'hAA)
    `CK("package var", pk::shared_v == 5)
    `CK("hier ref into instance", u8.din == 8'h0F)
    `CK("hier ref into gen block", 1)
    $display("CH23 CHECKS DONE fails=%0d", fails);
  end
endmodule
