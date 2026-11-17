// IEEE 1800-2023 §7.4.1: "The individual elements of the array are unsigned
// unless they are of a named type declared as signed." An element of a
// packed array of a signed typedef (`lane_t [0:8] q; q[1]`) is signed, so it
// is sign-extended as an operand and into a wider target; an element of a
// `logic signed [1:0][10:0]` vector is not. §13.4.1, §6.18: a function
// returning a signed typedef returns a signed value. §11.4.14: a compiled
// left stream of a part-select of such an array reverses whole elements.
package mp;
  typedef logic signed [10:0] lane_t;
  typedef lane_t [0:3] quad_t;
  function automatic lane_t f11(); return -362; endfunction
endpackage
module top;
  import mp::*;
  lane_t [0:8] q;
  lane_t [-9:9] u;
  quad_t [0:1] qq;
  logic signed [1:0][10:0] d_pkd;
  logic signed [16:0] w1, w2, w3, w4, w5, w6, w7, w8, wq, wf;
  logic b1, b2;
  int i0 = 1;
  lane_t [-3:3] l;
  lane_t [0:2] o, oc;
  assign o = {<< 11 {l[-3:-1]}};
  always_comb oc = {<< 11 {l[-3:-1]}};
  assign w7 = 1 * q[1];
  assign w8 = q[i0] >>> 1;
  always_comb wq = qq[0][1];
  initial begin
    q = '0; u = '0; qq = '0; l = '0;
    l[-3] = 11; l[-2] = 22; l[-1] = 33;
    q[1] = -362; u[1] = -362; qq[0][1] = -362;
    d_pkd = '0; d_pkd[0] = -362;
    #1;
    w1 = 1 * q[1];
    w2 = 1 * u[1];
    w3 = q[1];
    w4 = (i0 + 1) * q[i0];
    w5 = 1 * d_pkd[0];
    w6 = $signed(q[1]) / 2;
    b1 = q[1] < 0;
    b2 = q[1] == -362;
    wf = f11();
    $display("T|proc %0d %0d %0d %0d %0d %0d %b %b %0d", w1, w2, w3, w4, w5, w6, b1, b2, q[1]);
    $display("T|comb %0d %0d %0d", w7, w8, wq);
    $display("T|call %0d", wf);
    $display("T|stream %0d %0d %0d %0d %0d %0d", o[0], o[1], o[2], oc[0], oc[1], oc[2]);
    $finish;
  end
endmodule
