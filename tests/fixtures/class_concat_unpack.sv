class packet_base;
 bit flag,ns;
 bit[5:0] perms;
 bit[3:0] hi,lo;
 function void unpack(input bit[7:0] data[]);
  {flag,ns,perms}=data[0];
  {hi,lo}=data[1];
 endfunction
 function void shadow();
  bit[2:0] ns;
  {ns,perms}=9'h16a;
  if(ns!=5 || perms!=42)$fatal(1,"local shadow widths");
 endfunction
endclass
class child extends packet_base; endclass
class wide_packet #(parameter W=128);
 bit[W-1:0] payload;
 bit[3:0] tag;
 function void unpack(input bit[W+3:0] data);
  {tag,payload}=data;
 endfunction
endclass
module top;
 initial begin
 child c=new();bit[7:0] data[]={8'h6a,8'ha5};
 wide_packet#(128) wide=new();wide_packet#(8) narrow=new();
 repeat(1100) begin
 c.unpack(data);
 if(c.flag!=0 || c.ns!=1 || c.perms!=42 || c.hi!=10 || c.lo!=5)$fatal(1,"concat unpack");
 c.shadow();
 wide.unpack(132'ha_0123456789abcdef_fedcba9876543210);
 narrow.unpack(12'hbcd);
 if(wide.tag!=10 || wide.payload!=128'h0123456789abcdef_fedcba9876543210)$fatal(1,"wide unpack");
 if(narrow.tag!=11 || narrow.payload!=8'hcd)$fatal(1,"narrow unpack");
 end
 {c.hi,c.lo}=8'h3c;
 if(c.hi!=3 || c.lo!=12)$fatal(1,"explicit receiver unpack");
 $display("PASS");$finish;
 end
endmodule
