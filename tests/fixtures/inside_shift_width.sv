class cfg;
 int width=64;
endclass
class item;
 cfg c=new();
 rand bit [63:0] addr;
 constraint bounds {addr inside {[0:((1<<c.width)-1)]};}
endclass
module top;
 initial begin
 item x=new();
 if(!x.randomize() with {addr==64'hfedcba9876543210;})$fatal(1,"wide range rejected");
 if(x.addr!=64'hfedcba9876543210)$fatal(1,"wrong address");
 $display("PASS");$finish;
 end
endmodule
