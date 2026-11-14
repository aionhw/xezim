// IEEE 1800-2023 §7.4.2: an unpacked array bound may be negative, and §7.4.6:
// an index outside the declared range reads the element type's default.
// Every compiled context (continuous assignment, always_comb, always @*,
// always_ff with nonblocking writes) must address elements at negative
// indices like procedural code does.
module top;
  int arr[-1:1];
  int desc[1:-2];
  logic [7:0] pv[-2:0];
  int k = -1;
  logic [1:0] u = 2'b11;
  logic signed [1:0] s = -2'sd1;
  logic clk = 0;
  int c1, c2, c3, c4, c5, c6, c7, c8;
  logic [7:0] c9;
  int nb[-2:0];
  int bw[-1:1];
  assign c1 = arr[-1];
  assign c2 = arr[k] + 100 * arr[0];
  assign c3 = desc[-2] + desc[1];
  assign c4 = arr[s];
  assign c9 = pv[-2];
  always_comb c5 = arr[-1] + 100 * arr[0];
  always @* c6 = arr[k];
  always_comb c7 = arr[u];
  always_comb begin
    bw[-1] = arr[1] + 1;
    bw[k + 1] = arr[-1] + 2;
  end
  always_ff @(posedge clk) begin
    nb[-2] <= arr[-1];
    nb[k] <= arr[1];
  end
  assign c8 = nb[-2] + 10 * nb[-1];
  initial begin
    arr[-1] = 5; arr[0] = 6; arr[1] = 7;
    desc[-2] = 3; desc[1] = 40;
    pv[-2] = 8'h33;
    #1 clk = 1;
    #1;
    $display("T|reads %0d %0d %0d %0d %h", c1, c2, c3, c4, c9);
    $display("T|comb %0d %0d %0d", c5, c6, c7);
    $display("T|writes %0d %0d %0d", bw[-1], bw[0], c8);
    arr[-1] = 9; k = 1;
    #1;
    $display("T|after %0d %0d %0d %0d", c1, c2, c5, c6);
    $finish;
  end
endmodule
