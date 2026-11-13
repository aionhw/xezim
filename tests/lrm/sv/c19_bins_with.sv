// §19.5.1.1: bins with a `with` filter keep only the values the filter
// accepts; a fixed-size array bin spreads those values, in order, over its
// bins (§19.5.1).
package enc_pkg;
  parameter logic [31:0] True32 = 32'h9696_9696;
  parameter logic [31:0] False32 = 32'h6969_6969;
endpackage
class enc4_cov #(logic [3:0] ValueTrue = 4'h6, logic [3:0] ValueFalse = 4'h9);
  logic [3:0] value;
  covergroup enc_cg;
    cp_value: coverpoint value {
      bins true = {ValueTrue};
      bins false = {ValueFalse};
      bins others[4] = {[0:15]} with (!(item inside {ValueTrue, ValueFalse}));
    }
  endgroup
  function new(); enc_cg = new; endfunction
  function void sample(logic [3:0] v); value = v; enc_cg.sample(); endfunction
endclass
class enc32_cov;
  logic [31:0] value;
  covergroup enc_cg;
    cp_value: coverpoint value {
      bins true = {enc_pkg::True32};
      bins false = {enc_pkg::False32};
      bins others[32] = {[0:32'hffff_ffff]} with (!(item inside {enc_pkg::True32,
                                                                 enc_pkg::False32}));
    }
  endgroup
  function new(); enc_cg = new; endfunction
  function void sample(logic [31:0] v); value = v; enc_cg.sample(); endfunction
endclass
module c19_bins_with;
  logic [3:0] v;
  covergroup even_cg;
    coverpoint v { bins even[] = {[0:15]} with (item % 2 == 0); }
  endgroup
  covergroup odd_cg;
    coverpoint v {
      bins odd = {[0:7]} with (item[0]);
      bins big = {[8:15]};
    }
  endgroup
  covergroup ign_cg;
    coverpoint v { bins b[] = {[0:3]}; ignore_bins odd = {[0:3]} with (item % 2 == 1); }
  endgroup
  covergroup red_cg;
    coverpoint v { bins r[] = {[0:3]} with (|item); }
  endgroup
  ign_cg ic;
  red_cg rc;
  enc4_cov e4;
  enc32_cov e32;
  even_cg ec;
  odd_cg oc;
  initial begin
    e4 = new;
    e4.sample(4'h6); e4.sample(4'h9); e4.sample(4'h2); e4.sample(4'h5);
    e4.sample(4'ha); e4.sample(4'hb);
    $display("T|enc4|%0.2f", e4.enc_cg.get_inst_coverage());
    e32 = new;
    e32.sample(enc_pkg::True32); e32.sample(enc_pkg::False32);
    e32.sample(0); e32.sample(32'hffff_ffff);
    $display("T|enc32|%0.2f", e32.enc_cg.get_inst_coverage());
    ec = new;
    v = 2; ec.sample(); v = 3; ec.sample();
    $display("T|even|%0.2f", ec.get_inst_coverage());
    oc = new;
    v = 2; oc.sample();
    $display("T|odd_2|%0.2f", oc.get_inst_coverage());
    v = 3; oc.sample();
    $display("T|odd_3|%0.2f", oc.get_inst_coverage());
    ic = new; rc = new;
    v = 0; ic.sample(); v = 1; rc.sample();
    $display("T|ign|%0.2f", ic.get_inst_coverage());
    $display("T|red|%0.2f", rc.get_inst_coverage());
  end
endmodule
