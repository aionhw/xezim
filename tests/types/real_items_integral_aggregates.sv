// IEEE 1800-2023 §6.12.1, §10.9: each real item of an assignment pattern for an
// unpacked aggregate with 64-bit integral elements or members is rounded to
// the element type, in declaration initializers and procedural assignments.
module top;
  typedef struct { time t; longint l; logic [63:0] v; int i; } S;
  real r = -2.5;
  time arr [2] = '{9200000000000.0, 9200s};
  longint la [2] = '{2.5, -2.5};
  logic [63:0] lv [2] = '{2.5, -2.5};
  int ia [2] = '{2.5, -2.5};
  S s0 = '{t: 9200s, l: 3000000000.0, v: -2.5, i: 2.5};
  S s1;
  time q [$];
  longint d [];
  longint aa [int];
  longint p2 [2];
  longint arr2 [2][2] = '{'{1.5, 2.5}, '{-1.5, -2.5}};
  longint def [3] = '{default: 2.5};
  initial begin
    longint loc [2] = '{2.5, -2.5};
    S ls;
    q = '{9200s, 1.5};
    d = '{2.5, -2.5};
    aa = '{1: 2.5, 2: -2.5};
    p2 = '{r, r * 2};
    s1 = '{t: 9200s, l: 3000000000.0, v: r, i: r};
    ls = '{t: 2.5, l: -2.5, v: r, i: 1.5};
    $display("T|arr %0d %0d la %0d %0d lv %0d %0d ia %0d %0d", arr[0], arr[1], la[0], la[1], $signed(lv[0]), $signed(lv[1]), ia[0], ia[1]);
    $display("T|s0 %0d %0d %0d %0d", s0.t, s0.l, $signed(s0.v), s0.i);
    $display("T|s1 %0d %0d %0d %0d", s1.t, s1.l, $signed(s1.v), s1.i);
    $display("T|ls %0d %0d %0d %0d", ls.t, ls.l, $signed(ls.v), ls.i);
    $display("T|q %0d %0d d %0d %0d aa %0d %0d p2 %0d %0d", q[0], q[1], d[0], d[1], aa[1], aa[2], p2[0], p2[1]);
    $display("T|arr2 %0d %0d %0d %0d def %0d %0d loc %0d %0d", arr2[0][0], arr2[0][1], arr2[1][0], arr2[1][1], def[0], def[2], loc[0], loc[1]);
  end
endmodule
