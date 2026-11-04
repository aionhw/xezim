interface bus;int value;endinterface
class cfg;
 typedef virtual bus VIF;
 VIF vif;
 function void set(VIF v); vif=v;endfunction
 function void copy(cfg rhs); vif=rhs.vif;endfunction
endclass
module top;
 bus b();
 initial begin cfg c=new();cfg d=new();
 c.set(b);
 repeat(2005) begin d.copy(c);if(d.vif==null)$fatal(1,"lost vif");d.vif.value=17;end
 if(b.value!=17)$fatal(1,"write lost");
 $display("PASS");$finish;
 end
endmodule
