// §8.25/§25.9: the module parameter and inherited virtual-interface typedef
// describe one config_db specialization (#874).
`include "uvm_macros.svh"
import uvm_pkg::*;

interface data_if #(int DATA_WIDTH=32);
  logic [DATA_WIDTH-1:0] data;
endinterface

class agent_base #(int DATA_WIDTH=32) extends uvm_agent;
  typedef virtual interface data_if#(32) vif_t;
  `uvm_component_param_utils(agent_base#(DATA_WIDTH))

  function new(string name="", uvm_component parent=null);
    super.new(name,parent);
  endfunction

  virtual function void connect_phase(uvm_phase phase);
    vif_t vif;
    super.connect_phase(phase);
    if (!uvm_config_db#(vif_t)::get(this,"","vif",vif))
      `uvm_fatal("VIF_GET", "virtual interface was not found")
    if (vif.data !== 32'd71)
      `uvm_fatal("VIF_DATA", "virtual interface did not retain its connection")
    $display("T|config_vif|%0d",vif.data);
  endfunction
endclass

class agent_derived #(int DATA_WIDTH=32) extends agent_base#(DATA_WIDTH);
  `uvm_component_param_utils(agent_derived#(DATA_WIDTH))

  function new(string name="", uvm_component parent=null);
    super.new(name,parent);
  endfunction

  virtual function void connect_phase(uvm_phase phase);
    super.connect_phase(phase);
  endfunction
endclass

class config_test extends uvm_test;
  agent_derived#(32) agent;
  `uvm_component_utils(config_test)

  function new(string name="", uvm_component parent=null);
    super.new(name,parent);
  endfunction

  virtual function void build_phase(uvm_phase phase);
    super.build_phase(phase);
    agent=agent_derived#(32)::type_id::create("agent",this);
  endfunction
endclass

module top;
  localparam int DATA_WIDTH=32;
  data_if#(DATA_WIDTH) bus();

  initial begin
    bus.data=71;
    uvm_config_db#(virtual data_if#(DATA_WIDTH))::set(null,"uvm_test_top.agent","vif",bus);
    run_test("config_test");
  end
endmodule
