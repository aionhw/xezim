`timescale 1ns/1ps
// IEEE 1800-2023 §23.3.3, §6.6.7: a port connection between a user-defined
// nettype and a non-nettype connection is a continuous assignment from source to
// sink, so the port net is resolved by its nettype's resolution function (#1280).
package p;
  function automatic real pres(input real d[]);
    pres = 100.0;
    foreach (d[i]) pres += d[i];
  endfunction
  nettype real PN with pres;
endpackage
function automatic real res(input real d[]);
  res = 10.0;
  foreach (d[i]) res += d[i];
endfunction
nettype real N with res;
typedef struct { real f; bit v; } rec;
function automatic rec sres(input rec d[]);
  sres.f = 10.0; sres.v = 0;
  foreach (d[i]) begin sres.f += d[i].f; sres.v |= d[i].v; end
endfunction
nettype rec SN with sres;
module sink(input N x, output real y);
  assign y = x;
endmodule
module psink import p::*; (input PN x, output real y);
  assign y = x;
endmodule
module ssink(input SN x, output real y);
  assign y = x.f;
endmodule
module src(output N o);
  assign o = 2.0;
endmodule
module inner_driver(input N x, output real y);
  assign x = 3.0;   // a second driver of the input's own net
  assign y = x;
endmodule
module top;
  real drive = 1.25, g_var, g_lit, g_pkg, g_expr, g_net, g_struct, g_out, g_two, g_named;
  rec sdrive = '{1.25, 1'b1};
  N nnet;
  real rout;
  assign nnet = drive;
  sink s_var(drive, g_var);
  sink s_lit(1.25, g_lit);
  psink s_pkg(drive, g_pkg);
  sink s_expr(drive + 0.0, g_expr);
  sink s_net(nnet, g_net);
  ssink s_struct(sdrive, g_struct);
  src s_out(rout);
  sink s_named(.y(g_named), .x(drive));
  real g_inner;
  inner_driver s_inner(drive, g_inner);
  initial begin
    #1 $display("T|var=%g lit=%g pkg=%g expr=%g net=%g struct=%g out=%g named=%g inner=%g", g_var, g_lit, g_pkg, g_expr, g_net, g_struct, rout, g_named, g_inner);
    drive = 2.5;
    #1 $display("T|var=%g lit=%g pkg=%g expr=%g net=%g named=%g inner=%g drive=%g", g_var, g_lit, g_pkg, g_expr, g_net, g_named, g_inner, drive);
  end
endmodule
