`timescale 1ns/1ps
module top;
  typedef real real_t;
  function automatic real local_real(); real x; x=5; x/=2; return x; endfunction
  function automatic real initialized_real(); real x=5; x/=2; return x; endfunction
  function automatic real alias_real(); real_t x; x=5; return x/2; endfunction
  function automatic real default_real(); real x; return x+0.5; endfunction
  function automatic real direct_real_return(); return 5/2; endfunction
  function automatic int integral_local(); int x; x=2.5; return x; endfunction
  function automatic int integer_return(); return 2.5; endfunction
  function automatic int square(input logic [2:0] x); return x*x; endfunction
  localparam real A=local_real(), B=initialized_real(), C=alias_real(), D=default_real();
  localparam real E=direct_real_return(), P=5/2;
  localparam int I=integral_local(), J=integer_return(), S=square(4'd15);
  localparam F='1;
  localparam logic [7:0] FF=F, LF='1;
  initial begin
    #1;
    $display("T|real %.6f %.6f %.6f %.6f %.6f %.6f",A,B,C,D,E,P);
    $display("T|integral %0d %0d %0d",I,J,S);
    $display("T|fill %0d %h %h",$bits(F),FF,LF);
    $finish;
  end
endmodule
