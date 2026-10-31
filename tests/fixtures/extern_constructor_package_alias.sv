package common;
class obj;
 string m_name;
 function new(string name="");m_name=name;endfunction
 function string get_name();return m_name;endfunction
endclass
class cb extends obj;
 extern function new(string name="callback");
endclass
function cb::new(string name="callback");super.new(name);endfunction
endpackage
package vendor;
import common::*;
typedef class ext_cb;
class ext_cb extends cb;
 extern function new(string suite_name="",string name="ext_callback");
endclass
typedef ext_cb alias_cb;
function ext_cb::new(string suite_name="",string name="ext_callback");super.new(name);endfunction
endpackage
package bus_pkg;
import common::*;
import vendor::*;
class port_cb extends alias_cb;
 extern function new(string name="port_callback");
endclass
function port_cb::new(string name="port_callback");super.new("",name);endfunction
class custom_cb extends port_cb;
 int index;
 function new(int index=0);this.index=index;endfunction
endclass
endpackage
module top;
import bus_pkg::*;
 initial begin
 port_cb direct_cb=new();custom_cb child_cb=new(3);
 $display("NAMES direct='%s' child='%s'",direct_cb.get_name(),child_cb.get_name());
 if(direct_cb.get_name()!="port_callback" || child_cb.get_name()!="port_callback")$fatal(1,"callback name lost");
 $display("PASS");$finish;
 end
endmodule
