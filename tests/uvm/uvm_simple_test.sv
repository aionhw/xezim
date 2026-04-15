`include "uvm_mock.svh"
import uvm_pkg::*;

class my_test extends uvm_test;
  function new(string name = "my_test", uvm_component parent = null);
    super.new(name, parent);
  endfunction

  virtual task run_phase(uvm_phase phase);
    `uvm_info("TEST_TOP", "Hello World from UVM!", UVM_LOW)
    #10;
    `uvm_info("TEST_TOP", "UVM test is finished.", UVM_LOW)
  endtask
endclass

module top;
  my_test test;
  uvm_root global_top;
  uvm_phase phase;

  initial begin
    global_top = new("uvm_root", null);
    test = new("test_inst", global_top);
    global_top.test_inst = test;
    
    phase = new("run");
    $display("UVM_INFO: Running test my_test");
    test.run_phase(phase);
  end
endmodule
