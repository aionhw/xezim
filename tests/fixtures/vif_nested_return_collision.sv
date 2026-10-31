interface leaf_if;
 int id;
endinterface
typedef virtual leaf_if LEAF;
interface port_if;
 leaf_if tx();
 function LEAF get_tx();get_tx=port_if.tx;endfunction
endinterface
typedef virtual port_if PORT;
interface system_if;
 port_if ports[2]();
 function PORT get_port(int i);
 case(i)
 0:get_port=system_if.ports[0];
 1:get_port=system_if.ports[1];
 endcase
 endfunction
endinterface
class cfg;
 LEAF main_if;
 function void set_if(LEAF v);main_if=v;endfunction
endclass
class env_cfg;
 PORT sys_if;
 cfg tx[];
 function new();tx=new[1];tx[0]=new();endfunction
 function void set_if(PORT sys_if);
 this.sys_if=sys_if;
 foreach(tx[i]) tx[i].set_if(this.sys_if.get_tx());
 endfunction
endclass
module top;
 system_if sys_if();
 initial begin
 env_cfg c=new();
 c.set_if(sys_if.get_port(1));
 if(c.sys_if==null)$fatal(1,"port missing");
 if(c.tx[0].main_if==null)$fatal(1,"tx missing");
 c.tx[0].main_if.id=42;
 if(sys_if.ports[1].tx.id!=42)$fatal(1,"wrong interface");
 $display("PASS");$finish;
 end
endmodule
