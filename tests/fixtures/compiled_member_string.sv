class packet;string uid;endclass
class holder;
 packet d=new();
 function string uid();return "read_data_21399";endfunction
 function void fill(string value="");d.uid=value=="" ? uid():value;endfunction
endclass
module top;
 initial begin holder h=new();
 for(int i=0;i<2005;i++) begin
 h.fill();if(h.d.uid!="read_data_21399")$fatal(1,"iteration=%0d got=%s",i,h.d.uid);
 end
 $display("PASS");$finish;
 end
endmodule
