class txn;
 bit[255:0] data[];
 function new();data=new[8];foreach(data[i]) data[i]=i+10;endfunction
 function void pack(input bit[255:0] source[],output bit[7:0] result[]);
 if(source.size()!=8) $fatal(1,"data size %0d",source.size());
 result=new[source.size()]; foreach(source[j]) result[j]=source[j];
 endfunction
endclass
class driver;
 function void combine(txn split_txns[$]);
 foreach(split_txns[i]) begin
 bit[7:0] bytes[];
 split_txns[i].pack(split_txns[i].data,bytes);
 if(bytes.size()!=8 || bytes[7]!=17) $fatal(1,"copy failed");
 end
 endfunction
endclass
module top;
 initial begin txn q[$];driver d=new();repeat(3) begin txn t=new();q.push_back(t);end d.combine(q);$display("PASS");$finish;end
endmodule
