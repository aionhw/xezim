class checker_cfg;
 bit [63:0] addr=64'hfedcba9876543210;
 int width=64;
 function bit valid();return addr inside {[0:((1<<width)-1)]};endfunction
endclass
module top;
 bit [63:0] a=64'hfedcba9876543210;
 bit [127:0] b=128'hfedcba9876543210_abcdef1234567890;
 int w64=64,w128=128;
 real r=1.25;
 initial begin
 checker_cfg cfg=new();
 repeat(1100) if(!cfg.valid())$fatal(1,"class procedural range rejected");
 if(!(a inside {[0:((1<<w64)-1)]}))$fatal(1,"64-bit procedural range rejected");
 if(!(b inside {[0:((1<<w128)-1)]}))$fatal(1,"128-bit procedural range rejected");
 if(!(r inside {[1.0:1.5]}))$fatal(1,"real range rejected");
 if(r inside {[2.0:3.0]})$fatal(1,"real range wrongly accepted");
 $display("PASS");$finish;
 end
endmodule
