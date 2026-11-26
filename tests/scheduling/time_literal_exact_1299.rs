//! §5.8, §6.12.1, §3.14.1 (#1299): a time literal is a real value scaled to
//! the time unit of its scope and rounded to its precision, and a real
//! initializer of an integral variable converts to the nearest integer.
//! `time t = 9200s;` under `1ns/1fs` holds 9200000000000 (a 64-bit variable
//! kept the double, so a shift read its IEEE 754 bits), in a module, a
//! package, `$unit`, an interface, a child instance, a generate block, a
//! class property and a static local, and so do a `longint` or `bit [63:0]`
//! with a real constant and a typed parameter of a module or a package.
//! A literal of many ticks is exact: `9200s` equals 9200000000000, `7s` and
//! `#9200s` are whole numbers of femtoseconds, under several timescales and
//! in a delay of a 1 ps scope inside a 1 fs design. The expected lines are
//! the reference simulator's.
use std::process::Command;

const DESIGN_VALUES: &str = r#"
`timescale 1ns/1fs

package pk;
  time      p_time = 9200s;
  longint   p_long = 3000000000.0;
  function automatic bit is_9200s(time x);
    return x == 9200s;
  endfunction
endpackage

module child;
  time c_time = 9200s;
endmodule

class cfg;
  time t = 9200s;
endclass

module top;
  integer errors = 0, checks = 0;
  task automatic chk(string what, bit ok);
    checks++;
    if (ok) $display("PASS %s", what);
    else begin errors++; $display("FAIL %s", what); end
  endtask

  // declaration initializers (6.12.1)
  time        t_ten  = 10ns;
  time        t_big  = 9200s;
  time        t_real = 9200000000000.0;
  longint     l_real = 3000000000.0;
  bit [63:0]  b_real = 5000000000.0;
  logic [47:0] v_real = 1.0e12;          // narrower: converted already
  int         i_real = 3.7;              // narrower: converted already
  child       u ();
  if (1) begin : g
    time g_time = 9200s;
  end

  // time literal values (5.8)
  realtime    r_big  = 9200s;
  time        t_proc;                    // assigned in the initial block
  cfg         h = new;

  initial begin
    t_proc = 9200s;

    chk("time = 10ns, read by a shift",              (t_ten >> 1) == 64'd5);
    chk("time = 9200s, read by a shift",             (t_big >> 1) == 64'd4600000000000);
    chk("time = 9200s, printed with %0d",            $sformatf("%0d", t_big) == "9200000000000");
    chk("time = real constant, read by a shift",     (t_real >> 1) == 64'd4600000000000);
    chk("longint = real constant, printed with %0d", $sformatf("%0d", l_real) == "3000000000");
    chk("64-bit vector = real constant",             (b_real >> 1) == 64'd2500000000);
    chk("48-bit vector = real constant",             v_real == 48'd1000000000000);
    chk("int = real constant",                       i_real == 4);
    chk("package time = 9200s",                      (pk::p_time >> 1) == 64'd4600000000000);
    chk("package longint = real constant",           $sformatf("%0d", pk::p_long) == "3000000000");
    chk("child module time = 9200s",                 (u.c_time >> 1) == 64'd4600000000000);
    chk("generate block time = 9200s",               (g.g_time >> 1) == 64'd4600000000000);
    chk("procedural time = 9200s",                   (t_proc >> 1) == 64'd4600000000000);
    chk("class property time = 9200s",               (h.t >> 1) == 64'd4600000000000);

    chk("realtime = 9200s",                          r_big == 9200000000000.0);
    chk("integer variable == 9200s",                 t_proc == 9200s);
    chk("9200s == the real it denotes",              9200s == 9200000000000.0);
    chk("class property == 9200s",                   h.t == 9200s);
    chk("package function, x == 9200s",              pk::is_9200s(t_proc));

    if (errors == 0) $display("PASS all %0d checks", checks);
    else $display("FAIL %0d of %0d checks", errors, checks);
    $finish;
  end
endmodule
"#;

const DESIGN_INITS: &str = r#"
`timescale 1ns/1fs
time      u_time = 9200s;
package pk;
  time      p_time = 9200s;
  parameter time PP = 9200s;
  function automatic time ret9200(); return 9200s; endfunction
endpackage
interface ifc;
  time i_time = 9200s;
endinterface
class cfg;
  time t = 9200s;
  static time st = 9200s;
  longint l = 3000000000.0;
endclass
module top;
  integer errors = 0;
  task automatic chk(string what, bit ok);
    if (ok) $display("PASS %s", what);
    else begin errors++; $display("FAIL %s", what); end
  endtask
  parameter time       P_T = 9200s;
  localparam longint   P_L = 3000000000.0;
  parameter bit [63:0] P_B = 5000000000.0;
  parameter            P_U = 9200s;          // untyped: a real
  time t_neg_tie = -2.5;                     // rounds away from zero: -3
  longint l_neg = -3000000000.5;
  time t_tie = 2.5;
  ifc ii ();
  cfg h = new;
  function time ret_t(); return 9200s; endfunction
  function automatic longint pass_t(time x); return x >> 1; endfunction
  function automatic time static_local();
    static time s = 9200s;
    return s;
  endfunction
  task automatic auto_local(output time o);
    time a = 9200s;
    o = a;
  endtask
  initial begin
    time o;
    longint lp;
    auto_local(o);
    lp = 3000000000.0;
    chk("$unit time = 9200s",          (u_time >> 1) == 64'd4600000000000);
    chk("package parameter time",      (pk::PP >> 1) == 64'd4600000000000);
    chk("package function return",     (pk::ret9200() >> 1) == 64'd4600000000000);
    chk("interface time",              (ii.i_time >> 1) == 64'd4600000000000);
    chk("class time property",         (h.t >> 1) == 64'd4600000000000);
    chk("class static time property",  (cfg::st >> 1) == 64'd4600000000000);
    chk("class longint property",      (h.l >> 1) == 64'd1500000000);
    chk("parameter time",              (P_T >> 1) == 64'd4600000000000);
    chk("localparam longint",          (P_L >> 1) == 64'd1500000000);
    chk("parameter bit [63:0]",        (P_B >> 1) == 64'd2500000000);
    chk("untyped parameter == 9200s",  P_U == 9200000000000.0);
    chk("time = -2.5",                 $signed(t_neg_tie) == -3);
    chk("longint = -3000000000.5",     l_neg == -64'sd3000000001);
    chk("time = 2.5",                  t_tie == 3);
    chk("function return time",        (ret_t() >> 1) == 64'd4600000000000);
    chk("time argument",               pass_t(9200s) == 64'd4600000000000);
    chk("static function local",       (static_local() >> 1) == 64'd4600000000000);
    chk("automatic task local",        (o >> 1) == 64'd4600000000000);
    chk("procedural longint = 3e9",    (lp >> 1) == 64'd1500000000);
    if (errors == 0) $display("PASS all"); else $display("FAIL %0d", errors);
    $finish;
  end
endmodule
"#;

const DESIGN_UNITS: &str = r#"
module m_fs;
  timeunit 1ns; timeprecision 1fs;
  time t = 9200s; realtime r = 9200s; time p;
  initial begin
    p = 7s;
    #1ns;
    $display("%s m_fs t", (t >> 1) == 64'd4600000000000 ? "PASS" : "FAIL");
    $display("%s m_fs r", r == 9200000000000.0 ? "PASS" : "FAIL");
    $display("%s m_fs 7s", p == 7s && p == 64'd7000000000 ? "PASS" : "FAIL");
    $display("%s m_fs 4294967300ns", 4294967300ns == 4294967300.0 ? "PASS" : "FAIL");
    $display("%s m_fs 1.5ps", 1.5ps == 0.0015 ? "PASS" : "FAIL");
  end
endmodule
module m_ps;
  timeunit 1ps; timeprecision 1fs;
  time t = 9200s; time p;
  initial begin
    p = 9200s;
    #1ns;
    $display("%s m_ps t", (t >> 1) == 64'd4600000000000000 ? "PASS" : "FAIL");
    $display("%s m_ps p == 9200s", p == 9200s ? "PASS" : "FAIL");
  end
endmodule
module m_us;
  timeunit 1us; timeprecision 1fs;
  time t = 9200s; time p;
  initial begin
    p = 9200s;
    #1ns;
    $display("%s m_us t", (t >> 1) == 64'd4600000000 ? "PASS" : "FAIL");
    $display("%s m_us p == 9200s", p == 9200s ? "PASS" : "FAIL");
    $display("%s m_us 1ns", 1ns == 0.001 ? "PASS" : "FAIL");
  end
endmodule
module m_ns_ps;
  timeunit 1ns; timeprecision 1ps;
  time t = 2.5ps;      // 0.0025 ns or 0.003 ns: either way the integral 0
  initial begin
    #1ns;
    $display("%s m_ns_ps t", t == 0 ? "PASS" : "FAIL");
  end
endmodule
module top;
  timeunit 1ns; timeprecision 1ns;
  m_fs a (); m_ps b (); m_us c (); m_ns_ps d ();
  initial #10 $finish;
endmodule
"#;

const DESIGN_MATRIX_NS_FS: &str = r#"
`timescale 1ns/1fs
module top;
  integer errors = 0, checks = 0;
  task automatic chk(string what, bit ok);
    checks++;
    if (ok) $display("PASS %s", what); else begin errors++; $display("FAIL %s", what); end
  endtask
  time d0 = 1s;
  time p0;
  realtime r0 = 1s;
  time d1 = 3s;
  time p1;
  realtime r1 = 3s;
  time d2 = 5s;
  time p2;
  realtime r2 = 5s;
  time d3 = 9s;
  time p3;
  realtime r3 = 9s;
  time d4 = 10s;
  time p4;
  realtime r4 = 10s;
  time d5 = 100s;
  time p5;
  realtime r5 = 100s;
  time d6 = 1000s;
  time p6;
  realtime r6 = 1000s;
  time d7 = 9000s;
  time p7;
  realtime r7 = 9000s;
  time d8 = 9200s;
  time p8;
  realtime r8 = 9200s;
  time d9 = 1500ms;
  time p9;
  realtime r9 = 1500ms;
  time d10 = 2500000us;
  time p10;
  realtime r10 = 2500000us;
  time d11 = 5000000000ns;
  time p11;
  realtime r11 = 5000000000ns;
  initial begin
    p0 = 1s;
    chk("decl  1s", (d0 >> 1) == 64'd500000000);
    chk("disp  1s", $sformatf("%0d", d0) == "1000000000");
    chk("proc  1s", p0 == 64'd1000000000);
    chk("real  1s", r0 == 1000000000.0);
    chk("cmp   1s", p0 == 1s);
    chk("rcmp  1s", 1s == 1000000000.0);
    p1 = 3s;
    chk("decl  3s", (d1 >> 1) == 64'd1500000000);
    chk("disp  3s", $sformatf("%0d", d1) == "3000000000");
    chk("proc  3s", p1 == 64'd3000000000);
    chk("real  3s", r1 == 3000000000.0);
    chk("cmp   3s", p1 == 3s);
    chk("rcmp  3s", 3s == 3000000000.0);
    p2 = 5s;
    chk("decl  5s", (d2 >> 1) == 64'd2500000000);
    chk("disp  5s", $sformatf("%0d", d2) == "5000000000");
    chk("proc  5s", p2 == 64'd5000000000);
    chk("real  5s", r2 == 5000000000.0);
    chk("cmp   5s", p2 == 5s);
    chk("rcmp  5s", 5s == 5000000000.0);
    p3 = 9s;
    chk("decl  9s", (d3 >> 1) == 64'd4500000000);
    chk("disp  9s", $sformatf("%0d", d3) == "9000000000");
    chk("proc  9s", p3 == 64'd9000000000);
    chk("real  9s", r3 == 9000000000.0);
    chk("cmp   9s", p3 == 9s);
    chk("rcmp  9s", 9s == 9000000000.0);
    p4 = 10s;
    chk("decl  10s", (d4 >> 1) == 64'd5000000000);
    chk("disp  10s", $sformatf("%0d", d4) == "10000000000");
    chk("proc  10s", p4 == 64'd10000000000);
    chk("real  10s", r4 == 10000000000.0);
    chk("cmp   10s", p4 == 10s);
    chk("rcmp  10s", 10s == 10000000000.0);
    p5 = 100s;
    chk("decl  100s", (d5 >> 1) == 64'd50000000000);
    chk("disp  100s", $sformatf("%0d", d5) == "100000000000");
    chk("proc  100s", p5 == 64'd100000000000);
    chk("real  100s", r5 == 100000000000.0);
    chk("cmp   100s", p5 == 100s);
    chk("rcmp  100s", 100s == 100000000000.0);
    p6 = 1000s;
    chk("decl  1000s", (d6 >> 1) == 64'd500000000000);
    chk("disp  1000s", $sformatf("%0d", d6) == "1000000000000");
    chk("proc  1000s", p6 == 64'd1000000000000);
    chk("real  1000s", r6 == 1000000000000.0);
    chk("cmp   1000s", p6 == 1000s);
    chk("rcmp  1000s", 1000s == 1000000000000.0);
    p7 = 9000s;
    chk("decl  9000s", (d7 >> 1) == 64'd4500000000000);
    chk("disp  9000s", $sformatf("%0d", d7) == "9000000000000");
    chk("proc  9000s", p7 == 64'd9000000000000);
    chk("real  9000s", r7 == 9000000000000.0);
    chk("cmp   9000s", p7 == 9000s);
    chk("rcmp  9000s", 9000s == 9000000000000.0);
    p8 = 9200s;
    chk("decl  9200s", (d8 >> 1) == 64'd4600000000000);
    chk("disp  9200s", $sformatf("%0d", d8) == "9200000000000");
    chk("proc  9200s", p8 == 64'd9200000000000);
    chk("real  9200s", r8 == 9200000000000.0);
    chk("cmp   9200s", p8 == 9200s);
    chk("rcmp  9200s", 9200s == 9200000000000.0);
    p9 = 1500ms;
    chk("decl  1500ms", (d9 >> 1) == 64'd750000000);
    chk("disp  1500ms", $sformatf("%0d", d9) == "1500000000");
    chk("proc  1500ms", p9 == 64'd1500000000);
    chk("real  1500ms", r9 == 1500000000.0);
    chk("cmp   1500ms", p9 == 1500ms);
    chk("rcmp  1500ms", 1500ms == 1500000000.0);
    p10 = 2500000us;
    chk("decl  2500000us", (d10 >> 1) == 64'd1250000000);
    chk("disp  2500000us", $sformatf("%0d", d10) == "2500000000");
    chk("proc  2500000us", p10 == 64'd2500000000);
    chk("real  2500000us", r10 == 2500000000.0);
    chk("cmp   2500000us", p10 == 2500000us);
    chk("rcmp  2500000us", 2500000us == 2500000000.0);
    p11 = 5000000000ns;
    chk("decl  5000000000ns", (d11 >> 1) == 64'd2500000000);
    chk("disp  5000000000ns", $sformatf("%0d", d11) == "5000000000");
    chk("proc  5000000000ns", p11 == 64'd5000000000);
    chk("real  5000000000ns", r11 == 5000000000.0);
    chk("cmp   5000000000ns", p11 == 5000000000ns);
    chk("rcmp  5000000000ns", 5000000000ns == 5000000000.0);
    if (errors == 0) $display("PASS all %0d checks", checks);
    else $display("FAIL %0d of %0d checks", errors, checks);
    $finish;
  end
endmodule
"#;

const DESIGN_MATRIX_PS_FS: &str = r#"
`timescale 1ps/1fs
module top;
  integer errors = 0, checks = 0;
  task automatic chk(string what, bit ok);
    checks++;
    if (ok) $display("PASS %s", what); else begin errors++; $display("FAIL %s", what); end
  endtask
  time d0 = 1s;
  time p0;
  realtime r0 = 1s;
  time d1 = 3s;
  time p1;
  realtime r1 = 3s;
  time d2 = 5s;
  time p2;
  realtime r2 = 5s;
  time d3 = 9s;
  time p3;
  realtime r3 = 9s;
  time d4 = 10s;
  time p4;
  realtime r4 = 10s;
  time d5 = 100s;
  time p5;
  realtime r5 = 100s;
  time d6 = 1000s;
  time p6;
  realtime r6 = 1000s;
  time d7 = 9000s;
  time p7;
  realtime r7 = 9000s;
  time d8 = 9200s;
  time p8;
  realtime r8 = 9200s;
  time d9 = 1500ms;
  time p9;
  realtime r9 = 1500ms;
  time d10 = 2500000us;
  time p10;
  realtime r10 = 2500000us;
  time d11 = 5000000000ns;
  time p11;
  realtime r11 = 5000000000ns;
  initial begin
    p0 = 1s;
    chk("decl  1s", (d0 >> 1) == 64'd500000000000);
    chk("disp  1s", $sformatf("%0d", d0) == "1000000000000");
    chk("proc  1s", p0 == 64'd1000000000000);
    chk("real  1s", r0 == 1000000000000.0);
    chk("cmp   1s", p0 == 1s);
    chk("rcmp  1s", 1s == 1000000000000.0);
    p1 = 3s;
    chk("decl  3s", (d1 >> 1) == 64'd1500000000000);
    chk("disp  3s", $sformatf("%0d", d1) == "3000000000000");
    chk("proc  3s", p1 == 64'd3000000000000);
    chk("real  3s", r1 == 3000000000000.0);
    chk("cmp   3s", p1 == 3s);
    chk("rcmp  3s", 3s == 3000000000000.0);
    p2 = 5s;
    chk("decl  5s", (d2 >> 1) == 64'd2500000000000);
    chk("disp  5s", $sformatf("%0d", d2) == "5000000000000");
    chk("proc  5s", p2 == 64'd5000000000000);
    chk("real  5s", r2 == 5000000000000.0);
    chk("cmp   5s", p2 == 5s);
    chk("rcmp  5s", 5s == 5000000000000.0);
    p3 = 9s;
    chk("decl  9s", (d3 >> 1) == 64'd4500000000000);
    chk("disp  9s", $sformatf("%0d", d3) == "9000000000000");
    chk("proc  9s", p3 == 64'd9000000000000);
    chk("real  9s", r3 == 9000000000000.0);
    chk("cmp   9s", p3 == 9s);
    chk("rcmp  9s", 9s == 9000000000000.0);
    p4 = 10s;
    chk("decl  10s", (d4 >> 1) == 64'd5000000000000);
    chk("disp  10s", $sformatf("%0d", d4) == "10000000000000");
    chk("proc  10s", p4 == 64'd10000000000000);
    chk("real  10s", r4 == 10000000000000.0);
    chk("cmp   10s", p4 == 10s);
    chk("rcmp  10s", 10s == 10000000000000.0);
    p5 = 100s;
    chk("decl  100s", (d5 >> 1) == 64'd50000000000000);
    chk("disp  100s", $sformatf("%0d", d5) == "100000000000000");
    chk("proc  100s", p5 == 64'd100000000000000);
    chk("real  100s", r5 == 100000000000000.0);
    chk("cmp   100s", p5 == 100s);
    chk("rcmp  100s", 100s == 100000000000000.0);
    p6 = 1000s;
    chk("decl  1000s", (d6 >> 1) == 64'd500000000000000);
    chk("disp  1000s", $sformatf("%0d", d6) == "1000000000000000");
    chk("proc  1000s", p6 == 64'd1000000000000000);
    chk("real  1000s", r6 == 1000000000000000.0);
    chk("cmp   1000s", p6 == 1000s);
    chk("rcmp  1000s", 1000s == 1000000000000000.0);
    p7 = 9000s;
    chk("decl  9000s", (d7 >> 1) == 64'd4500000000000000);
    chk("disp  9000s", $sformatf("%0d", d7) == "9000000000000000");
    chk("proc  9000s", p7 == 64'd9000000000000000);
    chk("real  9000s", r7 == 9000000000000000.0);
    chk("cmp   9000s", p7 == 9000s);
    chk("rcmp  9000s", 9000s == 9000000000000000.0);
    p8 = 9200s;
    chk("decl  9200s", (d8 >> 1) == 64'd4600000000000000);
    chk("disp  9200s", $sformatf("%0d", d8) == "9200000000000000");
    chk("proc  9200s", p8 == 64'd9200000000000000);
    chk("real  9200s", r8 == 9200000000000000.0);
    chk("cmp   9200s", p8 == 9200s);
    chk("rcmp  9200s", 9200s == 9200000000000000.0);
    p9 = 1500ms;
    chk("decl  1500ms", (d9 >> 1) == 64'd750000000000);
    chk("disp  1500ms", $sformatf("%0d", d9) == "1500000000000");
    chk("proc  1500ms", p9 == 64'd1500000000000);
    chk("real  1500ms", r9 == 1500000000000.0);
    chk("cmp   1500ms", p9 == 1500ms);
    chk("rcmp  1500ms", 1500ms == 1500000000000.0);
    p10 = 2500000us;
    chk("decl  2500000us", (d10 >> 1) == 64'd1250000000000);
    chk("disp  2500000us", $sformatf("%0d", d10) == "2500000000000");
    chk("proc  2500000us", p10 == 64'd2500000000000);
    chk("real  2500000us", r10 == 2500000000000.0);
    chk("cmp   2500000us", p10 == 2500000us);
    chk("rcmp  2500000us", 2500000us == 2500000000000.0);
    p11 = 5000000000ns;
    chk("decl  5000000000ns", (d11 >> 1) == 64'd2500000000000);
    chk("disp  5000000000ns", $sformatf("%0d", d11) == "5000000000000");
    chk("proc  5000000000ns", p11 == 64'd5000000000000);
    chk("real  5000000000ns", r11 == 5000000000000.0);
    chk("cmp   5000000000ns", p11 == 5000000000ns);
    chk("rcmp  5000000000ns", 5000000000ns == 5000000000000.0);
    if (errors == 0) $display("PASS all %0d checks", checks);
    else $display("FAIL %0d of %0d checks", errors, checks);
    $finish;
  end
endmodule
"#;

const DESIGN_DELAY_ISSUE: &str = r#"
`timescale 1ns/1fs
module top;
  time half = 4600s;
  initial begin
    #4600s;
    if ($realtime == 4600000000000.0) $display("PASS #4600s resumes at 4600 s");
    else $display("FAIL #4600s resumes at %f ns", $realtime);
    #(half);
    if ($realtime == 9200000000000.0) $display("PASS #(variable) resumes at 9200 s");
    else $display("FAIL #(variable) resumes at %f ns", $realtime);
    if ($time == 64'd9200000000000) $display("PASS $time is 9200 s in ns");
    else $display("FAIL $time is %0d", $time);
    // the test a phase timeout watchdog makes when its delay ends
    if ($time == 9200s) $display("PASS $time == 9200s");
    else $display("FAIL $time == 9200s is false");
    $finish;
  end
endmodule
"#;

const DESIGN_DELAYS: &str = r#"
module m_ps;
  timeunit 1ns; timeprecision 1ps;
  time t9200 = 9200s;
  initial begin
    #9200s;
    $display("%s m_ps #9200s at %0d", $time == 64'd9200000000000 ? "PASS" : "FAIL", $time);
  end
  initial begin
    #(t9200);
    $display("%s m_ps #(t) realtime %.3f", $realtime == 9200000000000.0 ? "PASS" : "FAIL", $realtime);
  end
  initial begin
    #0.0015;
    $display("%s m_ps #0.0015 at %.4f", $realtime == 0.002 ? "PASS" : "FAIL", $realtime);
  end
endmodule
module top;
  timeunit 1ns; timeprecision 1fs;
  time half = 4600s;
  realtime rt = 9200s;
  parameter time P = 9200s;
  m_ps u ();
  initial begin
    #4600s;
    $display("%s #4600s %.3f", $realtime == 4600000000000.0 ? "PASS" : "FAIL", $realtime);
    #(half);
    $display("%s #(variable) %.3f", $realtime == 9200000000000.0 ? "PASS" : "FAIL", $realtime);
    $display("%s $time == 9200s", $time == 9200s ? "PASS" : "FAIL");
  end
  initial begin #(9200s);        $display("%s #(9200s) %.3f", $realtime == 9200000000000.0 ? "PASS" : "FAIL", $realtime); end
  initial begin #(9200s * 1.0);  $display("%s #(9200s * 1.0) %.3f", $realtime == 9200000000000.0 ? "PASS" : "FAIL", $realtime); end
  initial begin #(P);            $display("%s #(P) %.3f", $realtime == 9200000000000.0 ? "PASS" : "FAIL", $realtime); end
  initial begin #(rt);           $display("%s #(rt) %.3f", $realtime == 9200000000000.0 ? "PASS" : "FAIL", $realtime); end
  initial begin #7s;             $display("%s #7s %.3f", $realtime == 7000000000.0 ? "PASS" : "FAIL", $realtime); end
  initial #9210s $finish;
endmodule
"#;

const EXPECTED_VALUES: &[&str] = &[
    "PASS time = 10ns, read by a shift",
    "PASS time = 9200s, read by a shift",
    "PASS time = 9200s, printed with %0d",
    "PASS time = real constant, read by a shift",
    "PASS longint = real constant, printed with %0d",
    "PASS 64-bit vector = real constant",
    "PASS 48-bit vector = real constant",
    "PASS int = real constant",
    "PASS package time = 9200s",
    "PASS package longint = real constant",
    "PASS child module time = 9200s",
    "PASS generate block time = 9200s",
    "PASS procedural time = 9200s",
    "PASS class property time = 9200s",
    "PASS realtime = 9200s",
    "PASS integer variable == 9200s",
    "PASS 9200s == the real it denotes",
    "PASS class property == 9200s",
    "PASS package function, x == 9200s",
    "PASS all 19 checks",
];

const EXPECTED_INITS: &[&str] = &[
    "PASS $unit time = 9200s",
    "PASS package parameter time",
    "PASS package function return",
    "PASS interface time",
    "PASS class time property",
    "PASS class static time property",
    "PASS class longint property",
    "PASS parameter time",
    "PASS localparam longint",
    "PASS parameter bit [63:0]",
    "PASS untyped parameter == 9200s",
    "PASS time = -2.5",
    "PASS longint = -3000000000.5",
    "PASS time = 2.5",
    "PASS function return time",
    "PASS time argument",
    "PASS static function local",
    "PASS automatic task local",
    "PASS procedural longint = 3e9",
    "PASS all",
];

const EXPECTED_UNITS: &[&str] = &[
    "PASS m_fs t",
    "PASS m_fs r",
    "PASS m_fs 7s",
    "PASS m_fs 4294967300ns",
    "PASS m_fs 1.5ps",
    "PASS m_ps t",
    "PASS m_ps p == 9200s",
    "PASS m_us t",
    "PASS m_us p == 9200s",
    "PASS m_us 1ns",
    "PASS m_ns_ps t",
];

const EXPECTED_MATRIX_NS_FS: &[&str] = &[
    "PASS decl  1s",
    "PASS disp  1s",
    "PASS proc  1s",
    "PASS real  1s",
    "PASS cmp   1s",
    "PASS rcmp  1s",
    "PASS decl  3s",
    "PASS disp  3s",
    "PASS proc  3s",
    "PASS real  3s",
    "PASS cmp   3s",
    "PASS rcmp  3s",
    "PASS decl  5s",
    "PASS disp  5s",
    "PASS proc  5s",
    "PASS real  5s",
    "PASS cmp   5s",
    "PASS rcmp  5s",
    "PASS decl  9s",
    "PASS disp  9s",
    "PASS proc  9s",
    "PASS real  9s",
    "PASS cmp   9s",
    "PASS rcmp  9s",
    "PASS decl  10s",
    "PASS disp  10s",
    "PASS proc  10s",
    "PASS real  10s",
    "PASS cmp   10s",
    "PASS rcmp  10s",
    "PASS decl  100s",
    "PASS disp  100s",
    "PASS proc  100s",
    "PASS real  100s",
    "PASS cmp   100s",
    "PASS rcmp  100s",
    "PASS decl  1000s",
    "PASS disp  1000s",
    "PASS proc  1000s",
    "PASS real  1000s",
    "PASS cmp   1000s",
    "PASS rcmp  1000s",
    "PASS decl  9000s",
    "PASS disp  9000s",
    "PASS proc  9000s",
    "PASS real  9000s",
    "PASS cmp   9000s",
    "PASS rcmp  9000s",
    "PASS decl  9200s",
    "PASS disp  9200s",
    "PASS proc  9200s",
    "PASS real  9200s",
    "PASS cmp   9200s",
    "PASS rcmp  9200s",
    "PASS decl  1500ms",
    "PASS disp  1500ms",
    "PASS proc  1500ms",
    "PASS real  1500ms",
    "PASS cmp   1500ms",
    "PASS rcmp  1500ms",
    "PASS decl  2500000us",
    "PASS disp  2500000us",
    "PASS proc  2500000us",
    "PASS real  2500000us",
    "PASS cmp   2500000us",
    "PASS rcmp  2500000us",
    "PASS decl  5000000000ns",
    "PASS disp  5000000000ns",
    "PASS proc  5000000000ns",
    "PASS real  5000000000ns",
    "PASS cmp   5000000000ns",
    "PASS rcmp  5000000000ns",
    "PASS all 72 checks",
];

const EXPECTED_MATRIX_PS_FS: &[&str] = &[
    "PASS decl  1s",
    "PASS disp  1s",
    "PASS proc  1s",
    "PASS real  1s",
    "PASS cmp   1s",
    "PASS rcmp  1s",
    "PASS decl  3s",
    "PASS disp  3s",
    "PASS proc  3s",
    "PASS real  3s",
    "PASS cmp   3s",
    "PASS rcmp  3s",
    "PASS decl  5s",
    "PASS disp  5s",
    "PASS proc  5s",
    "PASS real  5s",
    "PASS cmp   5s",
    "PASS rcmp  5s",
    "PASS decl  9s",
    "PASS disp  9s",
    "PASS proc  9s",
    "PASS real  9s",
    "PASS cmp   9s",
    "PASS rcmp  9s",
    "PASS decl  10s",
    "PASS disp  10s",
    "PASS proc  10s",
    "PASS real  10s",
    "PASS cmp   10s",
    "PASS rcmp  10s",
    "PASS decl  100s",
    "PASS disp  100s",
    "PASS proc  100s",
    "PASS real  100s",
    "PASS cmp   100s",
    "PASS rcmp  100s",
    "PASS decl  1000s",
    "PASS disp  1000s",
    "PASS proc  1000s",
    "PASS real  1000s",
    "PASS cmp   1000s",
    "PASS rcmp  1000s",
    "PASS decl  9000s",
    "PASS disp  9000s",
    "PASS proc  9000s",
    "PASS real  9000s",
    "PASS cmp   9000s",
    "PASS rcmp  9000s",
    "PASS decl  9200s",
    "PASS disp  9200s",
    "PASS proc  9200s",
    "PASS real  9200s",
    "PASS cmp   9200s",
    "PASS rcmp  9200s",
    "PASS decl  1500ms",
    "PASS disp  1500ms",
    "PASS proc  1500ms",
    "PASS real  1500ms",
    "PASS cmp   1500ms",
    "PASS rcmp  1500ms",
    "PASS decl  2500000us",
    "PASS disp  2500000us",
    "PASS proc  2500000us",
    "PASS real  2500000us",
    "PASS cmp   2500000us",
    "PASS rcmp  2500000us",
    "PASS decl  5000000000ns",
    "PASS disp  5000000000ns",
    "PASS proc  5000000000ns",
    "PASS real  5000000000ns",
    "PASS cmp   5000000000ns",
    "PASS rcmp  5000000000ns",
    "PASS all 72 checks",
];

const EXPECTED_DELAY_ISSUE: &[&str] = &[
    "PASS #4600s resumes at 4600 s",
    "PASS #(variable) resumes at 9200 s",
    "PASS $time is 9200 s in ns",
    "PASS $time == 9200s",
];

const EXPECTED_DELAYS: &[&str] = &[
    "PASS m_ps #0.0015 at 0.0020",
    "PASS #7s 7000000000.000",
    "PASS #4600s 4600000000000.000",
    "PASS m_ps #9200s at 9200000000000",
    "PASS m_ps #(t) realtime 9200000000000.000",
    "PASS #(9200s) 9200000000000.000",
    "PASS #(9200s * 1.0) 9200000000000.000",
    "PASS #(P) 9200000000000.000",
    "PASS #(rt) 9200000000000.000",
    "PASS #(variable) 9200000000000.000",
    "PASS $time == 9200s",
];

fn run(tag: &str, design: &str, compile: &str, extra: &[&str]) -> String {
    let dir = std::env::temp_dir().join(format!(
        "xezim_time_literal_1299_{}_{}_{}",
        std::process::id(),
        tag,
        compile
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.sv");
    std::fs::write(&path, design).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "top", "--no-cache"])
        .args(extra)
        .arg(path.to_str().unwrap())
        .env("XEZIM_COMPILE_METHODS", compile)
        .env_remove("XEZIM_METHOD_CACHE")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    text
}

fn check(tag: &str, design: &str, expected: &[&str], extra: &[&str]) {
    for compile in ["1", "0"] {
        let log = run(tag, design, compile, extra);
        let got: Vec<&str> = log
            .lines()
            .filter(|l| l.starts_with("PASS") || l.starts_with("FAIL"))
            .collect();
        assert_eq!(
            got, expected,
            "{tag}, XEZIM_COMPILE_METHODS={compile}:\n{log}"
        );
    }
}

#[test]
fn time_literal_and_real_initializers_of_64_bit_variables() {
    check("values", DESIGN_VALUES, EXPECTED_VALUES, &[]);
}

#[test]
fn real_initializers_convert_at_every_declaration_site() {
    check("inits", DESIGN_INITS, EXPECTED_INITS, &[]);
}

#[test]
fn time_literal_values_under_several_timescales() {
    check("units", DESIGN_UNITS, EXPECTED_UNITS, &[]);
}

#[test]
fn time_literal_sweep_is_exact_under_a_femtosecond_precision() {
    check(
        "matrix_ns_fs",
        DESIGN_MATRIX_NS_FS,
        EXPECTED_MATRIX_NS_FS,
        &[],
    );
    check(
        "matrix_ps_fs",
        DESIGN_MATRIX_PS_FS,
        EXPECTED_MATRIX_PS_FS,
        &[],
    );
}

#[test]
fn delays_by_large_time_literals_are_exact() {
    let max = ["--max-time", "10000s"];
    check(
        "delay_issue",
        DESIGN_DELAY_ISSUE,
        EXPECTED_DELAY_ISSUE,
        &max,
    );
    check("delays", DESIGN_DELAYS, EXPECTED_DELAYS, &max);
}
