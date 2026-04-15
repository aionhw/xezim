import uvm_pkg::*;
`include "uvm_macros.svh"

// Transaction
class simple_transaction extends uvm_sequence_item;
  rand int data;
  `uvm_object_utils_begin(simple_transaction)
    `uvm_field_int(data, UVM_ALL_ON)
  `uvm_object_utils_end
  function new(string name = "simple_transaction");
    super.new(name);
  endfunction
endclass

// Driver
class simple_driver extends uvm_driver #(simple_transaction);
  `uvm_component_utils(simple_driver)
  function new(string name, uvm_component parent);
    super.new(name, parent);
  endfunction
  virtual task run_phase(uvm_phase phase);
    simple_transaction tr;
    forever begin
      seq_item_port.get_next_item(tr);
      `uvm_info("DRV", $sformatf("Driving data: %0d", tr.data), UVM_LOW)
      #10;
      seq_item_port.item_done();
    end
  endtask
endclass

// Monitor
class simple_monitor extends uvm_monitor;
  uvm_analysis_port #(simple_transaction) ap;
  `uvm_component_utils(simple_monitor)
  function new(string name, uvm_component parent);
    super.new(name, parent);
    ap = new("ap", this);
  endfunction
  virtual task run_phase(uvm_phase phase);
    simple_transaction tr;
    forever begin
      #10;
      tr = simple_transaction::type_id::create("tr");
      tr.data = 42;
      `uvm_info("MON", $sformatf("Monitored data: %0d", tr.data), UVM_LOW)
      ap.write(tr);
    end
  endtask
endclass

// Scoreboard
class simple_scoreboard extends uvm_scoreboard;
  `uvm_component_utils(simple_scoreboard)
  uvm_analysis_imp #(simple_transaction, simple_scoreboard) item_collected_export;
  function new(string name, uvm_component parent);
    super.new(name, parent);
    item_collected_export = new("item_collected_export", this);
  endfunction
  virtual function void write(simple_transaction tr);
    `uvm_info("SB", $sformatf("Checked data: %0d", tr.data), UVM_LOW)
  endfunction
endclass

// Agent
class simple_agent extends uvm_agent;
  simple_driver    drv;
  simple_monitor   mon;
  uvm_sequencer #(simple_transaction) sqr;
  `uvm_component_utils(simple_agent)
  function new(string name, uvm_component parent);
    super.new(name, parent);
  endfunction
  virtual function void build_phase(uvm_phase phase);
    super.build_phase(phase);
    mon = simple_monitor::type_id::create("mon", this);
    if(get_is_active() == UVM_ACTIVE) begin
      drv = simple_driver::type_id::create("drv", this);
      sqr = uvm_sequencer#(simple_transaction)::type_id::create("sqr", this);
    end
  endfunction
  virtual function void connect_phase(uvm_phase phase);
    if(get_is_active() == UVM_ACTIVE) begin
      drv.seq_item_port.connect(sqr.seq_item_export);
    end
  endfunction
endclass

// Env
class simple_env extends uvm_env;
  simple_agent      agt;
  simple_scoreboard sb;
  `uvm_component_utils(simple_env)
  function new(string name, uvm_component parent);
    super.new(name, parent);
  endfunction
  virtual function void build_phase(uvm_phase phase);
    super.build_phase(phase);
    agt = simple_agent::type_id::create("agt", this);
    sb  = simple_scoreboard::type_id::create("sb", this);
  endfunction
  virtual function void connect_phase(uvm_phase phase);
    agt.mon.ap.connect(sb.item_collected_export);
  endfunction
endclass

// Test
class simple_test extends uvm_test;
  simple_env env;
  `uvm_component_utils(simple_test)
  function new(string name = "simple_test", uvm_component parent = null);
    super.new(name, parent);
  endfunction
  virtual function void build_phase(uvm_phase phase);
    super.build_phase(phase);
    env = simple_env::type_id::create("env", this);
  endfunction
  virtual task run_phase(uvm_phase phase);
    phase.raise_objection(this);
    `uvm_info("TEST", "Starting test...", UVM_LOW)
    #100;
    `uvm_info("TEST", "Finishing test...", UVM_LOW)
    phase.drop_objection(this);
  endtask
endclass

module top;
  initial begin
    run_test("simple_test");
  end
endmodule
