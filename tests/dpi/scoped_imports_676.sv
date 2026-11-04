// IEEE 1800-2023 §26.3, §35.5.3, §35.5.4: separate declarations and context.
`timescale 1ns/1ns
package first_pkg;
  import "DPI-C" context c_first = function int callback(input int value);
  import "DPI-C" context c_first_task = task action(output int value);
  function int wrapper(int value); return callback(value); endfunction
endpackage
package second_pkg;
  import "DPI-C" context c_second = function int callback(input int value);
  import "DPI-C" context c_second_task = task action(output int value);
  function int wrapper(int value); return callback(value); endfunction
endpackage
module imported;
  import first_pkg::*;
  initial $display("T|import|%0d", callback(5));
endmodule
module top;
  import "DPI-C" context c_design = function int callback(input int value);
  export "DPI-C" task wait_step;
  task wait_step; #2; endtask
  imported u();
  int a, b;
  initial begin
    $display("T|scoped|%0d %0d", first_pkg::callback(1), second_pkg::callback(2));
    $display("T|wrapper|%0d %0d", first_pkg::wrapper(3), second_pkg::wrapper(4));
    $display("T|design|%0d", callback(6));
    fork
      first_pkg::action(a);
      second_pkg::action(b);
    join
    $display("T|tasks|%0d %0d %0t", a, b, $time);
  end
endmodule
