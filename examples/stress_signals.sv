// Tiny stress design — single layer of replication so the
// elaborator stays fast. The point is just to have *some*
// non-array named signals so the dual-store / CSR / Insn fixes
// run their code paths. RSS delta vs pre-fix won't be visible
// at this scale (we'd need 100K+ named signals for that), but
// behavior parity should be.

module bit_cell #(parameter W = 32) (
    input  wire         clk,
    input  wire [W-1:0] in_a,
    input  wire [W-1:0] in_b,
    output reg  [W-1:0] state
);
    always @(posedge clk) begin
        state <= state ^ in_a + in_b;
    end
endmodule

module top;
    parameter integer N        = 131072;
    parameter integer W        = 32;
    parameter integer MAX_TIME = 100;

    reg clk = 1'b0;
    always #5 clk = ~clk;

    wire [W-1:0] zero = {W{1'b0}};
    wire [N*W-1:0] flat_out;

    genvar i;
    generate
        for (i = 0; i < N; i = i + 1) begin : g
            bit_cell #(.W(W)) c (
                .clk   (clk),
                .in_a  (zero),
                .in_b  (zero),
                .state (flat_out[i*W +: W])
            );
        end
    endgenerate

    initial begin
        #(MAX_TIME) $finish;
    end
endmodule
