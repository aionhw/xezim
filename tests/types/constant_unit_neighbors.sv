parameter real R=8.0;
parameter real S=R*2;
parameter int W=8;
parameter logic [W-1:0] B=8'ha5;
function automatic real read_unit(); return R; endfunction
function automatic real shadow_formal(input real R); return R; endfunction
module top;
  parameter real R=2.0;
  parameter int W=2;
  localparam real A=$unit::R;
  localparam real C=S;
  localparam logic [7:0] D=B;
  localparam int BW=$bits(B);
  localparam real E=read_unit();
  localparam real F=shadow_formal(3.0);
  initial begin
    $display("T|unit %f %f %f %0d %h %f %f", R,A,C,BW,D,E,F);
    $finish;
  end
endmodule
