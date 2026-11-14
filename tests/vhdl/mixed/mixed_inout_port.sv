// A VHDL entity with inout ports instantiated from SystemVerilog: each
// port is one net that both sides drive (IEEE 1800-2017 §23.3.3.2,
// IEEE 1076-2008 §14.7.3.3). 'Z' on either side leaves the net to the
// other, equal values agree, different ones give x, and a port may be left
// unconnected or out of the list.
`timescale 1ns/1ps
module mixed_inout_port;
  logic oe = 0;
  logic [7:0] dout = 8'h00;
  wire d;
  wire [7:0] bus8;
  wire seen, seen_b;
  wire [7:0] seen8, seen8_b;
  logic sv_d = 1'bz;
  logic [7:0] sv_b = 8'hzz;
  assign d = sv_d;
  assign bus8 = sv_b;
  io_ent u (.oe(oe), .dout(dout), .d(d), .bus8(bus8), .nc(), .seen(seen), .seen8(seen8));
  // The inout ports left out: the entity sees what it drives itself.
  io_ent u_open (.oe(oe), .dout(dout), .seen(seen_b), .seen8(seen8_b));

  task show(string t);
    $display("%s d=%b bus8=%b seen=%b seen8=%b open=%b %b", t, d, bus8, seen, seen8, seen_b, seen8_b);
  endtask

  initial begin
    #1 show("idle");
    sv_d = 1; sv_b = 8'h5a;            #1 show("sv drives");
    sv_d = 1'bz; sv_b = 8'hzz;         #1 show("released");
    oe = 1; dout = 8'ha7;              #1 show("vhdl drives");
    sv_d = 0; sv_b = 8'ha7;            #1 show("both");
    oe = 0; sv_b = 8'h27;              #1 show("vhdl off");
    sv_d = 1'bz; sv_b = 8'bzzzz_1111;  #1 show("partial");
    oe = 1; dout = 8'h0f;              #1 show("agree");
    dout = 8'hf0;                      #1 show("fight");
    $finish;
  end
endmodule
