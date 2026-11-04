// IEEE 1800-2023 §23.9 / §26.3: local declarations shadow wildcard imports.
`timescale 1ns/1ns
package first_pkg;
  import "DPI-C" scope_callback = function int callback(input int value);
  import "DPI-C" scope_action = task action(output int value);
  function int wrapper(int value); return callback(value); endfunction
  task wrapper_task(output int value); action(value); endtask
endpackage
class worker;
  function int callback(int value); return value+90; endfunction
  task action(output int value); #1 value=99; endtask
  task run;
    int value;
    action(value);
    $display("T|method|%0d %0d",callback(3),value);
  endtask
endclass
module imported;
  import first_pkg::*;
  initial $display("T|import|%0d",callback(2));
endmodule
module top;
  import first_pkg::*;
  function int callback(int value); return value+40; endfunction
  task action(output int value); #1 value=42; endtask
  task automatic receiver(worker first_pkg);
    $display("T|receiver|%0d",first_pkg.callback(9));
  endtask
  imported u();
  worker w;
  int value;
  initial begin
    w=new; receiver(w);
    $display("T|local|%0d",callback(2));
    action(value); $display("T|local_task|%0d %0t",value,$time);
    w.run();
    $display("T|package|%0d %0d",first_pkg::callback(2),first_pkg::wrapper(2));
    first_pkg::action(value); $display("T|package_task|%0d",value);
    first_pkg::wrapper_task(value); $display("T|wrapper_task|%0d",value);
  end
endmodule
