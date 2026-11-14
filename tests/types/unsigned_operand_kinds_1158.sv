// IEEE 1800-2023 §11.8.1: "If any operand is unsigned, the result is
// unsigned, regardless of the operator." An unsigned package parameter,
// function result, unpacked-structure or union member, typedef cast,
// interface variable or hierarchical reference therefore zero-extends the
// signed byte next to it (§11.8.2), in continuous, `always_comb` and
// procedural code alike.
package pk; localparam bit [7:0] PP = 8'h80; endpackage
interface ifc; bit [7:0] u; endinterface
module leaf; bit [7:0] u; endmodule
module top;
  typedef struct { bit [7:0] u; int i; } UNP;
  typedef union packed { bit [7:0] u; byte sb; } UN;
  typedef bit [7:0] u8_t;
  localparam bit [7:0] LP = 8'h80;
  function automatic bit [7:0] idu(input bit [7:0] v); return v; endfunction
  byte pb; bit [7:0] pu; UNP unp; UN un;
  ifc i0 (); leaf lf ();
  longint a_lp, a_pkg, a_call, a_unp, a_un, a_cast, a_if, a_hier;
  longint c_lp, c_pkg, c_call, c_unp, c_un, c_cast, c_if, c_hier;
  longint p_lp, p_pkg, p_call, p_unp, p_un, p_cast, p_if, p_hier;
  assign a_lp = pb + LP;
  assign a_pkg = pb + pk::PP;
  assign a_call = pb + idu(pu);
  assign a_unp = pb + unp.u;
  assign a_un = pb + un.u;
  assign a_cast = pb + u8_t'(pb);
  assign a_if = pb + i0.u;
  assign a_hier = pb + lf.u;
  always_comb begin
    c_lp = pb + LP; c_pkg = pb + pk::PP; c_call = pb + idu(pu); c_unp = pb + unp.u;
    c_un = pb + un.u; c_cast = pb + u8_t'(pb); c_if = pb + i0.u; c_hier = pb + lf.u;
  end
  initial begin
    pb = -1; pu = 8'h80; unp.u = 8'h80; un.u = 8'h80; i0.u = 8'h80; lf.u = 8'h80;
    #1;
    p_lp = pb + LP; p_pkg = pb + pk::PP; p_call = pb + idu(pu); p_unp = pb + unp.u;
    p_un = pb + un.u; p_cast = pb + u8_t'(pb); p_if = pb + i0.u; p_hier = pb + lf.u;
    $display("T|assign %h %h %h %h %h %h %h %h", a_lp, a_pkg, a_call, a_unp, a_un, a_cast, a_if, a_hier);
    $display("T|comb %h %h %h %h %h %h %h %h", c_lp, c_pkg, c_call, c_unp, c_un, c_cast, c_if, c_hier);
    $display("T|proc %h %h %h %h %h %h %h %h", p_lp, p_pkg, p_call, p_unp, p_un, p_cast, p_if, p_hier);
    $finish;
  end
endmodule
