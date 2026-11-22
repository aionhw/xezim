`timescale 1ns/1ns
module tb;
  reg sel = 0;
  reg [137:0] a = 0, b = 0, invert_a = 0;
  wire [137:0] lhs;
  wire [81:0] y;
  wire [7:0] aliased;
  integer rises = 0;

  assign lhs = a ^ invert_a;
  assign y[66:61] = sel ? lhs[66:61] : b[101:96];
  assign aliased[5:0] = sel ? lhs[66:61] : lhs[64:59];
  always @(posedge y[61]) rises = rises + 1;

  task sample;
    #1 $display("T| %b %b %0d", y[66:61], aliased[5:0], rises);
  endtask

  initial begin
    sample;
    repeat (32) begin
      invert_a[61] = ~invert_a[61]; sample;
      a[62] = ~a[62]; sample;
      b[0] = ~b[0]; sample;
    end
    b[101:96] = 6'b010z01; sample;
    a[66:61] = 6'b10xz11; sel = 1; sample;
    sel = 0; b[101:96] = 6'b11x10z; sample;
    sel = 1'bx; sample;
    a[65:62] = 4'b011x; sample;
    b[98:96] = 3'b010; sample;
    sel = 1'bz; sample;
    sel = 1; sample;
    repeat (16) begin
      b[96] = ~b[96]; sample;
    end
    if ($test$plusargs("overrides")) begin
      force y[65:62] = 4'b0110; sample;
      a[65:62] = 4'b1110; sample;
      release y[65:62]; sample;
      $deposit(y, 82'b0); sample;
      b[96] = ~b[96]; sample;
      sel = 0; sample;
    end
    $finish;
  end
endmodule
