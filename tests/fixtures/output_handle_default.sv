class txn;int id;endclass
class monitor;
 task sample(bit fresh,output txn t);
 if(fresh) t=new();
 endtask
endclass
module top;
 initial begin
 monitor m=new();txn t;
 m.sample(1,t);if(t==null) $fatal(1,"missing first");
 m.sample(0,t);if(t!=null) $fatal(1,"stale output handle");
 $display("PASS");$finish;
 end
endmodule
