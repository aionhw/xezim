`timescale 1ns/1ps
// Neighbour probes for #925: virtual-interface collections as subroutine
// formals (ref / output / fixed / 2-D / queue / assoc), as task locals, in a
// struct member, returned from a function, with event controls and waits.
interface Bus (input clk, input rst_l);
  logic go = 1;
  clocking cb @(posedge clk); input rst_l, go; endclocking
endinterface

typedef virtual Bus vbus_t;
typedef vbus_t vbus_arr_t[];

class Probe;
  bit r_ref, r_out, r_fix, r_2d, r_qpush, r_asc, r_lq, r_la, r_sz, r_fe, r_str, r_ret, r_tdf;
  int e_fix, e_cb, e_wait, e_2d, e_q;
  virtual Bus keep[2];

  function automatic void f_ref(ref virtual Bus f[]);
    r_ref = (f[1] != null) && (f[1].rst_l === 1'b1);
    f[1] = f[0];                       // visible to the caller through ref
  endfunction
  function automatic void f_out(output virtual Bus f[]);
    f = new[2];
    f[0] = keep[0]; f[1] = keep[1];
  endfunction
  task automatic t_fix(virtual Bus f[2]);
    r_fix = (f[1] != null) && (f[1].rst_l === 1'b1);
    fork begin @(posedge f[0].clk); e_fix = 1; end begin #43; end join_any disable fork;
    fork begin @(f[0].cb); e_cb = 1; end begin #43; end join_any disable fork;
    fork begin wait (f[0].go); e_wait = 1; end begin #43; end join_any disable fork;
  endtask
  task automatic t_2d(virtual Bus f[2][2]);
    r_2d = (f[1][0] != null) && (f[1][0].rst_l === 1'b1);
    fork begin @(posedge f[1][0].clk); e_2d = 1; end begin #43; end join_any disable fork;
  endtask
  task automatic t_q(virtual Bus f[$]);
    f.push_back(keep[1]);
    r_qpush = (f.size() == 2) && (f[1] != null) && (f[1].rst_l === 1'b1);
    fork begin @(posedge f[0].clk); e_q = 1; end begin #43; end join_any disable fork;
  endtask
  function automatic void f_asc(virtual Bus f[int]);
    r_asc = f.exists(5) && (f[5] != null) && (f[5].rst_l === 1'b1);
  endfunction
  task automatic t_locals(virtual Bus a, virtual Bus b);
    virtual Bus lq[$];
    virtual Bus la[int];
    virtual Bus lf[2];
    int n;
    lq.push_back(a); lq.push_back(b);
    la[7] = b;
    lf[0] = a; lf[1] = b;
    r_lq = (lq[1] != null) && (lq[1].rst_l === 1'b1) && (lq[0].go === 1'b1);
    r_la = (la[7] != null) && (la[7].rst_l === 1'b1);
    r_sz = (lq.size() == 2);
    n = 0;
    foreach (lf[i]) if (lf[i] != null && lf[i].rst_l === 1'b1) n++;
    r_fe = (n == 2);
  endtask
  function automatic vbus_arr_t get_arr();
    vbus_arr_t r = new[2];
    r[0] = keep[0]; r[1] = keep[1];
    return r;
  endfunction
  function automatic void f_tdf(vbus_arr_t f);
    r_tdf = (f[1] != null) && (f[1].rst_l === 1'b1);
  endfunction
endclass

module testbench;
  bit clk = 0; bit rst_l = 1;
  always #5 clk = ~clk;
  Bus b0(clk, rst_l);
  Bus b1(clk, rst_l);
  Probe p;
  virtual Bus vd[];
  virtual Bus vo[];
  virtual Bus vq[$];
  virtual Bus va[int];
  virtual Bus v2[2][2];
  virtual Bus vf[2];
  vbus_arr_t got;
  int failures = 0;
  `define CHK(expr, msg) if (!(expr)) begin failures++; $display("T|FAIL %s", msg); end else $display("T|PASS %s", msg);

  initial begin
    p = new; p.keep[0] = b0; p.keep[1] = b1;
    vd = new[2]; vd[0] = b0; vd[1] = b1;
    vq.push_back(b0);
    va[5] = b1;
    v2[1][0] = b0; v2[0][1] = b1;
    vf[0] = b0; vf[1] = b1;
    #1;
    p.f_ref(vd);
    `CHK(p.r_ref, "ref formal element read")
    `CHK(vd[1] == b0, "ref formal element write visible to caller")
    p.f_out(vo);
    `CHK(vo.size() == 2 && vo[1] != null && vo[1].rst_l === 1'b1, "output formal array written back")
    p.t_fix(vf);
    `CHK(p.r_fix, "fixed formal element read")
    `CHK(p.e_fix == 1, "posedge on fixed formal element")
    `CHK(p.e_cb == 1, "clocking event on fixed formal element")
    `CHK(p.e_wait == 1, "wait on fixed formal element member")
    p.t_2d(v2);
    `CHK(p.e_2d == 1, "posedge on 2-D formal element")
    p.t_q(vq);
    `CHK(p.r_qpush, "queue formal push_back then read")
    `CHK(p.e_q == 1, "posedge on queue formal element")
    `CHK(vq.size() == 1, "queue formal by value: caller unchanged")
    p.f_asc(va);
    `CHK(p.r_asc, "assoc formal element read")
    p.t_locals(b0, b1);
    `CHK(p.r_lq, "task-local queue elements")
    `CHK(p.r_la, "task-local assoc element")
    `CHK(p.r_sz, "task-local queue size")
    `CHK(p.r_fe, "foreach over task-local fixed vif array")
    got = p.get_arr();
    `CHK(got.size() == 2 && got[1] != null && got[1].rst_l === 1'b1, "vif array returned from function")
    p.f_tdf(got);
    `CHK(p.r_tdf, "typedef'd vif array formal")
    if (failures == 0) $display("T|TEST_PASS"); else $display("T|TEST_FAIL count=%0d", failures);
    $finish;
  end
endmodule
