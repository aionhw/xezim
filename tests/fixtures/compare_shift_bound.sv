class transaction;
 rand bit[63:0] addr;
 int width=52;
 constraint range_c {addr < (1 << width); addr[20:12]==0; addr==((addr>>3)<<3);}
endclass
module top;
 initial begin
 transaction t=new();
 repeat(10) begin
 if(!t.randomize())$fatal(1,"aligned address failed");
 if(t.addr >= (64'b1<<52) || t.addr[20:12]!=0 || t.addr[2:0]!=0)$fatal(1,"invalid address");
 end
 $display("PASS");$finish;
 end
endmodule
