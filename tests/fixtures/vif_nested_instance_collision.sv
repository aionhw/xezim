interface leaf_if;
 logic [127:0] data;
endinterface
interface outer_if;
 leaf_if child();
endinterface
module top;
 outer_if vif();
 class driver;
   virtual leaf_if vif;
   task run();
     vif.data = 128'h123456789abcdef0_fedcba9876543210;
     if(vif.data[127:64] !== 64'h123456789abcdef0) $fatal(1,"read mismatch");
     vif.data[7:0] = 8'h55;
   endtask
 endclass
 initial begin
   driver d=new();d.vif=vif.child;d.run();
   if(vif.child.data !== 128'h123456789abcdef0_fedcba9876543255) $fatal(1,"write mismatch");
   $display("PASS nested instance name collision");$finish;
 end
endmodule
