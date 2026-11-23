// IEEE 1800-2023 §19.5.1: integral and real bins take tolerance ranges.
`timescale 1ns/1ps
module top;
  int value;
  real rv;
  covergroup cg;
    option.per_instance = 1;
    cp: coverpoint value {
      bins first = {[5 +/- 2]};
      bins second = {[100 +%- 10.0]};
    }
  endgroup
  covergroup rcg;
    option.per_instance = 1;
    cp: coverpoint rv {
      bins absolute = {[8.5 +/- 0.125]};
      bins relative = {[100.0 +%- 1.0]};
    }
  endgroup
  cg inst = new;
  rcg rinst = new;
  real c0, c1, c2;
  initial begin
    value = 8; inst.sample();
    value = 111; inst.sample();
    value = 2; inst.sample();
    c0 = inst.get_inst_coverage();
    value = 7; inst.sample();
    c1 = inst.get_inst_coverage();
    value = 90; inst.sample();
    c2 = inst.get_inst_coverage();
    $display("T|coverage %0.0f %0.0f %0.0f", c0, c1, c2);
    rv = 8.7; rinst.sample();
    rv = 98.9; rinst.sample();
    c0 = rinst.get_inst_coverage();
    rv = 8.375; rinst.sample();
    c1 = rinst.get_inst_coverage();
    rv = 101.0; rinst.sample();
    c2 = rinst.get_inst_coverage();
    $display("T|real coverage %0.0f %0.0f %0.0f", c0, c1, c2);
    $finish;
  end
endmodule
