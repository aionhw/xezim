// IEEE 1800-2023 §7.2, §7.4, §10.9.1, §6.18: a whole-array assignment to a fixed
// array member of an unpacked structure declared in a procedural block reaches
// every element (`s.m = '{default: 2.5}`), and a member declared with an array
// typedef (`typedef real A[2]; struct { A m; }`) has its elements, as in a
// module-scope variable; the elements kept old values, read x, or read 0.
module top;
  typedef real A[2];
  typedef struct { real m[2]; } SM;
  typedef struct { A m; } SA;
  typedef struct { int m[2]; } SI;
  SM gsm; SA gsa;
  function automatic real f();
    SM fs;
    fs.m[0] = 0.5; fs.m = '{default: 2.5};
    return fs.m[0] + fs.m[1];
  endfunction
  initial begin
    SM sm; SI si; SA sa;
    sm.m[0] = 0.5; sm.m[1] = 0.0625;
    $display("T|elem %0.2f %0.2f", sm.m[0], sm.m[1]);
    sm.m = '{default: 2.5};
    $display("T|dflt %0.2f %0.2f", sm.m[0], sm.m[1]);
    si.m[0] = 7; si.m = '{default: 3};
    $display("T|int %0d %0d", si.m[0], si.m[1]);
    si.m = '{4, 5};
    $display("T|ordered %0d %0d", si.m[0], si.m[1]);
    $display("T|func %0.2f", f());
    sa.m[1] = 1.5;
    $display("T|atd %0.2f", sa.m[1]);
    sa.m = '{0.25, 0.75};
    $display("T|atd2 %0.2f %0.2f", sa.m[0], sa.m[1]);
    gsa.m[1] = 1.5; gsm.m = '{default: 2.5};
    $display("T|mod %0.2f %0.2f", gsa.m[1], gsm.m[0]);
  end
endmodule
