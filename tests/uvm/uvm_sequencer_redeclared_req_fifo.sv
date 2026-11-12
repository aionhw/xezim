// §8.14: a user sequencer that redeclares uvm_sequencer_param_base's m_req_fifo
import uvm_pkg::*;
`include "uvm_macros.svh"
class item extends uvm_sequence_item;
  rand int v;
  `uvm_object_utils(item)
  function new(string name = "item"); super.new(name); endfunction
endclass
class my_seqr extends uvm_sequencer #(item);
  `uvm_component_utils(my_seqr)
  uvm_tlm_fifo #(item) m_req_fifo;   // same name as the base's request FIFO
  function new(string name, uvm_component parent);
    super.new(name, parent);
    m_req_fifo = new("my_fifo", this);
  endfunction
endclass
class my_seq extends uvm_sequence #(item);
  `uvm_object_utils(my_seq)
  function new(string name = "my_seq"); super.new(name); endfunction
  task body();
    repeat (3) begin
      req = item::type_id::create("req");
      start_item(req);
      req.v = 5;
      finish_item(req);
    end
  endtask
endclass
class my_drv extends uvm_driver #(item);
  `uvm_component_utils(my_drv)
  int n;
  function new(string name, uvm_component parent); super.new(name, parent); endfunction
  task run_phase(uvm_phase phase);
    forever begin
      seq_item_port.get_next_item(req);
      n++;
      #1;
      seq_item_port.item_done();
    end
  endtask
endclass
class my_test extends uvm_test;
  `uvm_component_utils(my_test)
  my_seqr sqr; my_drv drv;
  function new(string name, uvm_component parent); super.new(name, parent); endfunction
  function void build_phase(uvm_phase phase);
    sqr = my_seqr::type_id::create("sqr", this);
    drv = my_drv::type_id::create("drv", this);
  endfunction
  function void connect_phase(uvm_phase phase);
    drv.seq_item_port.connect(sqr.seq_item_export);
  endfunction
  task run_phase(uvm_phase phase);
    my_seq s = my_seq::type_id::create("s");
    phase.raise_objection(this);
    s.start(sqr);
    phase.drop_objection(this);
  endtask
  function void report_phase(uvm_phase phase);
    $display("DRIVER GOT %0d ITEMS, USER FIFO USED %0d", drv.n, sqr.m_req_fifo.used());
  endfunction
endclass
module top;
  initial run_test("my_test");
endmodule
