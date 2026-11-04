class port_cfg;
 int kind;
endclass
class env_cfg;
 port_cfg ports[];
 function new();ports=new[3];foreach(ports[i])ports[i]=new();endfunction
endclass
class sys_cfg;
 env_cfg targets[];
 function new();targets=new[1];targets[0]=new();endfunction
endclass
class cfg;
 sys_cfg systems[];
 function new();systems=new[1];systems[0]=new();endfunction
 function void setup(sys_cfg sys);
 foreach(sys.targets[i]) begin
 foreach(sys.targets[i].ports[j]) begin
 $display("ITER i=%0d j=%0d",i,j);
 sys.targets[i].ports[j].kind=1;
 end
 end
 endfunction
endclass
module top;
 initial begin
 cfg c=new();c.setup(c.systems[0]);
 for(int k=0;k<3;k++)begin
 $display("PORT k=%0d kind=%0d",k,c.systems[0].targets[0].ports[k].kind);
 if(c.systems[0].targets[0].ports[k].kind!=1)$fatal(1,"missed port");
 end
 $display("PASS");$finish;
 end
endmodule
