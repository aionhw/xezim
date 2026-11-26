//! §9.4.2: a process parked on an event control resumes when one of the
//! terms of its event expression sees its event. The scheduler examines a
//! parked process when a signal its terms name is written, not at every
//! scheduling pass (issue #1194); these pin what must not change with that:
//! which waiters wake, on which writes, and in which order. The expected
//! lines were recorded from the reference simulator.

use xezim::simulate;

fn run(src: &str) -> Vec<String> {
    simulate(src, 100_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

fn expected(s: &str) -> Vec<String> {
    s.lines().map(str::to_string).collect()
}

/// Level, edge, `or`-list and `iff` terms next to 32 row processes parked
/// on signals that stay quiet until the end. A blocking, nonblocking or
/// `#0` write, a continuous assignment's update, a `force`, a `release`
/// and a `$deposit` each wake exactly the waiters whose term sees an event
/// (§9.4.2: 1->x and x->0 are negedges, 0->z and z->1 posedges); writing
/// the value a signal already holds, by any of them, wakes nobody. The
/// counts are read one time unit after each write.
const PLAIN_TERMS: &str = r##"
`timescale 1ns/1ns
module row (input wire za, input wire zb, output logic [7:0] zq);
  task zw(); begin #0 zq = {za, zb, zq[7:2]}; end endtask
  always @(za or zb) zw();
  initial zq = 8'h00;
endmodule
module top;
  logic clk = 0, a = 0, b = 0, en = 0, d = 0, q0 = 0, q1 = 0;
  logic [7:0] v = 0;
  logic [99:0] wide = '0;
  wire w_a = a;
  wire [7:0] zq [0:31];
  int n_lvl, n_pos, n_neg, n_or, n_iff, n_wa, n_edge, n_v, n_wide;
  genvar g;
  generate
    for (g = 0; g < 32; g++) begin : idle
      row u (.za(q0), .zb(q1), .zq(zq[g]));
    end
  endgenerate
  always @(a) begin #0 n_lvl++; end
  always @(posedge a) begin #0 n_pos++; end
  always @(negedge a) begin #0 n_neg++; end
  always @(a or b) begin #0 n_or++; end
  always @(posedge clk iff en) begin #0 n_iff++; end
  always @(w_a) begin #0 if ($time > 0) n_wa++; end
  always @(edge d) begin #0 n_edge++; end
  always @(v) begin #0 n_v++; end
  always @(wide) begin #0 n_wide++; end
  always #5 clk = ~clk;
  task show(input string what);
    #1 $display("T| %0t %s lvl=%0d pos=%0d neg=%0d or=%0d iff=%0d wa=%0d edge=%0d v=%0d wide=%0d",
                $time, what, n_lvl, n_pos, n_neg, n_or, n_iff, n_wa, n_edge, n_v, n_wide);
  endtask
  initial begin
    #1 a = 1;                 show("a=1");
    a = 1;                    show("a=1 again");
    a <= 0;                   show("a<=0");
    b = 1; a = 1;             show("b=1,a=1");
    b = 1'bx;                 show("b=x");
    a = 1'bx;                 show("a=x");
    a = 0;                    show("a=0");
    a = 1'bz;                 show("a=z");
    a = 1;                    show("a=1 from z");
    v = 8'd3;                 show("v=3");
    v = 8'd3;                 show("v=3 again");
    v <= 8'd3;                show("v<=3");
    v <= 8'd4;                show("v<=4");
    #0 v = 8'd5;              show("#0 v=5");
    d = 1;                    show("d=1");
    d = 1'bx;                 show("d=x");
    d = 1'bx;                 show("d=x again");
    wide[99] = 1'b1;          show("wide[99]");
    wide = wide;              show("wide=wide");
    wide[0] <= 1'b1;          show("wide[0]<=1");
    force a = 1'b0;           show("force a=0");
    a = 1;                    show("a=1 forced");
    release a;                show("release a");
    a = 1;                    show("a=1 released");
    $deposit(v, 8'd9);        show("deposit v=9");
    en = 1;
    #20                       show("en for 20");
    en = 0;
    #20                       show("en off 20");
    q1 = 1;                   show("idle rows fire");
    $display("T| zq0=%h zq31=%h", zq[0], zq[31]);
    $finish;
  end
endmodule
"##;

#[test]
fn plain_signal_terms_wake_on_their_events_only() {
    assert_eq!(
        run(PLAIN_TERMS),
        expected(
            "\
T| 2 a=1 lvl=1 pos=1 neg=0 or=1 iff=0 wa=1 edge=0 v=0 wide=0\n\
T| 3 a=1 again lvl=1 pos=1 neg=0 or=1 iff=0 wa=1 edge=0 v=0 wide=0\n\
T| 4 a<=0 lvl=2 pos=1 neg=1 or=2 iff=0 wa=2 edge=0 v=0 wide=0\n\
T| 5 b=1,a=1 lvl=3 pos=2 neg=1 or=3 iff=0 wa=3 edge=0 v=0 wide=0\n\
T| 6 b=x lvl=3 pos=2 neg=1 or=4 iff=0 wa=3 edge=0 v=0 wide=0\n\
T| 7 a=x lvl=4 pos=2 neg=2 or=5 iff=0 wa=4 edge=0 v=0 wide=0\n\
T| 8 a=0 lvl=5 pos=2 neg=3 or=6 iff=0 wa=5 edge=0 v=0 wide=0\n\
T| 9 a=z lvl=6 pos=3 neg=3 or=7 iff=0 wa=6 edge=0 v=0 wide=0\n\
T| 10 a=1 from z lvl=7 pos=4 neg=3 or=8 iff=0 wa=7 edge=0 v=0 wide=0\n\
T| 11 v=3 lvl=7 pos=4 neg=3 or=8 iff=0 wa=7 edge=0 v=1 wide=0\n\
T| 12 v=3 again lvl=7 pos=4 neg=3 or=8 iff=0 wa=7 edge=0 v=1 wide=0\n\
T| 13 v<=3 lvl=7 pos=4 neg=3 or=8 iff=0 wa=7 edge=0 v=1 wide=0\n\
T| 14 v<=4 lvl=7 pos=4 neg=3 or=8 iff=0 wa=7 edge=0 v=2 wide=0\n\
T| 15 #0 v=5 lvl=7 pos=4 neg=3 or=8 iff=0 wa=7 edge=0 v=3 wide=0\n\
T| 16 d=1 lvl=7 pos=4 neg=3 or=8 iff=0 wa=7 edge=1 v=3 wide=0\n\
T| 17 d=x lvl=7 pos=4 neg=3 or=8 iff=0 wa=7 edge=2 v=3 wide=0\n\
T| 18 d=x again lvl=7 pos=4 neg=3 or=8 iff=0 wa=7 edge=2 v=3 wide=0\n\
T| 19 wide[99] lvl=7 pos=4 neg=3 or=8 iff=0 wa=7 edge=2 v=3 wide=1\n\
T| 20 wide=wide lvl=7 pos=4 neg=3 or=8 iff=0 wa=7 edge=2 v=3 wide=1\n\
T| 21 wide[0]<=1 lvl=7 pos=4 neg=3 or=8 iff=0 wa=7 edge=2 v=3 wide=2\n\
T| 22 force a=0 lvl=8 pos=4 neg=4 or=9 iff=0 wa=8 edge=2 v=3 wide=2\n\
T| 23 a=1 forced lvl=8 pos=4 neg=4 or=9 iff=0 wa=8 edge=2 v=3 wide=2\n\
T| 24 release a lvl=8 pos=4 neg=4 or=9 iff=0 wa=8 edge=2 v=3 wide=2\n\
T| 25 a=1 released lvl=9 pos=5 neg=4 or=10 iff=0 wa=9 edge=2 v=3 wide=2\n\
T| 26 deposit v=9 lvl=9 pos=5 neg=4 or=10 iff=0 wa=9 edge=2 v=4 wide=2\n\
T| 47 en for 20 lvl=9 pos=5 neg=4 or=10 iff=2 wa=9 edge=2 v=4 wide=2\n\
T| 68 en off 20 lvl=9 pos=5 neg=4 or=10 iff=2 wa=9 edge=2 v=4 wide=2\n\
T| 69 idle rows fire lvl=9 pos=5 neg=4 or=10 iff=2 wa=9 edge=2 v=4 wide=2\n\
T| zq0=40 zq31=40"
        )
    );
}

/// Terms that are not a whole signal: a bit- and a part-select, a
/// variable bit index, a constant and a variable array element, a packed
/// and an unpacked struct member, a memory cell and an expression. Each
/// wakes on a change of the selected value only, not on every write of
/// its operands, and a variable index is followed when it moves (§9.4.2).
const SELECT_TERMS: &str = r##"
`timescale 1ns/1ns
module top;
  typedef struct packed { logic [3:0] hi; logic [3:0] lo; } pk_t;
  typedef struct { logic f; logic [7:0] g; } up_t;
  logic [7:0] vec = 0;
  logic [7:0] arr [0:3];
  logic [7:0] mem [0:1023];
  pk_t pk = '0;
  up_t up;
  int i = 3, k = 1;
  task automatic tick(input string who);
    #0 $display("T| %0t %s", $time, who);
  endtask
  always @(vec[3]) tick("vec[3]");
  always @(vec[i]) tick("vec[i]");
  always @(arr[2]) tick("arr[2]");
  always @(arr[k]) tick("arr[k]");
  always @(pk.lo) tick("pk.lo");
  always @(up.g) tick("up.g");
  always @(mem[700]) tick("mem[700]");
  always @(vec[6:5]) tick("vec[6:5]");
  always @(vec[1] ^ vec[0]) tick("vec[1]^vec[0]");
  initial begin
    for (int j = 0; j < 4; j++) arr[j] = 0;
    up.f = 0; up.g = 0;
    for (int j = 0; j < 1024; j++) mem[j] = 0;
    #1 vec = 8'h08;
    #1 vec = 8'h09;
    #1 vec = 8'h0b;
    #1 i = 0;
    #1 vec = 8'h0a;
    #1 vec = 8'h6a;
    #1 arr[2] = 8'd7;
    #1 arr[1] = 8'd9;
    #1 k = 3;
    #1 arr[3] = 8'd1;
    #1 arr[1] = 8'd2;
    #1 pk.hi = 4'h3;
    #1 pk.lo = 4'h3;
    #1 up.f = 1;
    #1 up.g = 8'h11;
    #1 mem[699] = 8'd1;
    #1 mem[700] = 8'd1;
    #1 mem[700] = 8'd1;
    #1 mem[700] <= 8'd2;
    #5 $display("T| done");
    $finish;
  end
endmodule
"##;

#[test]
fn select_and_member_terms_follow_their_operands() {
    assert_eq!(
        run(SELECT_TERMS),
        expected(
            "\
T| 0 mem[700]\n\
T| 0 up.g\n\
T| 0 arr[k]\n\
T| 0 arr[2]\n\
T| 1 vec[i]\n\
T| 1 vec[3]\n\
T| 2 vec[1]^vec[0]\n\
T| 3 vec[1]^vec[0]\n\
T| 5 vec[1]^vec[0]\n\
T| 5 vec[i]\n\
T| 6 vec[6:5]\n\
T| 7 arr[2]\n\
T| 8 arr[k]\n\
T| 9 arr[k]\n\
T| 10 arr[k]\n\
T| 13 pk.lo\n\
T| 15 up.g\n\
T| 17 mem[700]\n\
T| 19 mem[700]\n\
T| done"
        )
    );
}

/// Parked children of `fork ... join_any` removed by `disable fork`, a
/// waiter whose process is killed (§9.7), a named block disabled while it
/// waits (§9.6.2), and named events with `.triggered` (§15.5): a removed
/// waiter never runs, the others wake once per event.
const FORK_AND_EVENTS: &str = r##"
`timescale 1ns/1ns
module top;
  logic a = 0, b = 0, c = 0, go = 0;
  event e1, e2;
  int woke;
  process p_h;
  initial begin
    fork
      begin @(posedge a); $display("T| %0t child a", $time); woke++; end
      begin @(posedge b); $display("T| %0t child b", $time); woke++; end
      begin @(posedge c); $display("T| %0t child c", $time); woke++; end
    join_any
    $display("T| %0t join_any", $time);
    disable fork;
    #10;
    fork
      begin p_h = process::self(); @(posedge b); $display("T| %0t killed child ran", $time); end
      begin @(e1); $display("T| %0t e1 seen triggered=%0d", $time, e1.triggered); end
      begin wait (e2.triggered); $display("T| %0t e2 triggered", $time); end
      begin : blk forever begin @(go); $display("T| %0t go=%b", $time, go); end end
    join_none
    #1 p_h.kill();
    #1 b = 0; #1 b = 1;
    #1 -> e1;
    #1 -> e2;
    #1 go = 1; #1 go = 0;
    #1 disable blk;
    #1 go = 1;
    #5 $display("T| woke=%0d", woke);
    $finish;
  end
  initial begin
    #2 b = 1;
    #1 a = 1;
    #1 c = 1;
  end
endmodule
"##;

#[test]
fn disabled_and_killed_waiters_do_not_wake() {
    assert_eq!(
        run(FORK_AND_EVENTS),
        expected(
            "\
T| 2 child b\n\
T| 2 join_any\n\
T| 16 e1 seen triggered=1\n\
T| 17 e2 triggered\n\
T| 18 go=1\n\
T| 19 go=0\n\
T| woke=1"
        )
    );
}

/// Terms on input ports (collapsed onto their actual, behind an
/// expression and behind a net alias), on a hierarchical name and inside a
/// task two processes run. Writes reach them from the testbench, through
/// a nonblocking assignment, a `force`/`release` of an inner net and
/// `$deposit` (§23.3.3, §10.6.2).
const PORTS_AND_HIERARCHY: &str = r##"
`timescale 1ns/1ns
module leaf (input wire pi, input logic [3:0] pv, output logic po);
  int n_or, n_pos;
  always @(pi or pv) begin #0 if ($time > 0) n_or++; end
  always @(posedge pi) begin if ($time > 0) po = ~po; #0 if ($time > 0) n_pos++; end
  initial po = 0;
endmodule
module mid (input wire mi, input logic [3:0] mv, output wire mo);
  wire inner = mi;
  leaf u_l0 (.pi(inner), .pv(mv), .po(mo));
  leaf u_l1 (.pi(mi), .pv(mv + 4'd1), .po());
endmodule
module top;
  logic s = 0;
  logic [3:0] q = 0;
  wire o0, o1;
  int n_task, n_o, n_hier;
  mid u_m0 (.mi(s), .mv(q), .mo(o0));
  mid u_m1 (.mi(~s), .mv(q), .mo(o1));
  task automatic wait_on();
    @(u_m0.u_l0.po);
    n_task++;
  endtask
  initial forever wait_on();
  initial forever wait_on();
  always @(o0 or o1) begin #0 if ($time > 0) n_o++; end
  always @(u_m1.inner) begin #0 if ($time > 0) n_hier++; end
  task show(input string what);
    #1 $display("T| %0t %s or=%0d,%0d,%0d,%0d pos=%0d,%0d,%0d,%0d task=%0d o=%0d hier=%0d",
                $time, what, u_m0.u_l0.n_or, u_m0.u_l1.n_or, u_m1.u_l0.n_or, u_m1.u_l1.n_or,
                u_m0.u_l0.n_pos, u_m0.u_l1.n_pos, u_m1.u_l0.n_pos, u_m1.u_l1.n_pos,
                n_task, n_o, n_hier);
  endtask
  initial begin
    #5                        show("start");
    s = 1;                    show("s=1");
    q = 4'd3;                 show("q=3");
    s = 0;                    show("s=0");
    q = 4'd3;                 show("q=3 again");
    s <= 1;                   show("s<=1");
    force u_m0.inner = 1'b0;  show("force inner=0");
    release u_m0.inner;       show("release inner");
    $deposit(s, 1'b0);        show("deposit s=0");
    $deposit(q, 4'd9);        show("deposit q=9");
    $finish;
  end
endmodule
"##;

#[test]
fn port_and_hierarchical_terms_wake_through_every_writer() {
    assert_eq!(
        run(PORTS_AND_HIERARCHY),
        expected(
            "\
T| 6 start or=0,0,0,0 pos=0,0,0,0 task=0 o=0 hier=0\n\
T| 7 s=1 or=1,1,1,1 pos=1,1,0,0 task=2 o=1 hier=1\n\
T| 8 q=3 or=2,2,2,2 pos=1,1,0,0 task=2 o=1 hier=1\n\
T| 9 s=0 or=3,3,3,3 pos=1,1,1,1 task=2 o=2 hier=2\n\
T| 10 q=3 again or=3,3,3,3 pos=1,1,1,1 task=2 o=2 hier=2\n\
T| 11 s<=1 or=4,4,4,4 pos=2,2,1,1 task=4 o=3 hier=3\n\
T| 12 force inner=0 or=5,4,4,4 pos=2,2,1,1 task=4 o=3 hier=3\n\
T| 13 release inner or=6,4,4,4 pos=3,2,1,1 task=6 o=4 hier=3\n\
T| 14 deposit s=0 or=7,5,5,5 pos=3,2,2,2 task=6 o=5 hier=4\n\
T| 15 deposit q=9 or=8,6,6,6 pos=3,2,2,2 task=6 o=5 hier=4"
        )
    );
}

/// Eight row processes whose ports are a bit and a part of two shared
/// vectors (§23.3.3). A write to a vector wakes the rows whose own bits
/// changed and no other, whether it is a bit or part write, a whole-vector
/// write, a nonblocking one, or a `force`/`release`.
const SELECT_PORTS: &str = r##"
`timescale 1ns/1ns
module row (input wire za, input wire [1:0] zp, output logic [7:0] zq);
  int n;
  task zw(); begin #0 zq = {za, zp, zq[7:3]}; if ($time > 0) n++; end endtask
  always @(za or zp) zw();
  initial zq = 8'h00;
endmodule
module top;
  logic clk = 0;
  logic [7:0] wl = '0;
  logic [15:0] pv = '0;
  wire [7:0] zq [0:7];
  always #5 clk = ~clk;
  genvar gi;
  generate
    for (gi = 0; gi < 8; gi++) begin : g
      row u (.za(wl[gi]), .zp(pv[2*gi +: 2]), .zq(zq[gi]));
    end
  endgenerate
  task show(input string what);
    #1 $display("T| %0t %s n=%0d,%0d,%0d,%0d,%0d,%0d,%0d,%0d zq3=%h zq5=%h", $time, what,
                g[0].u.n, g[1].u.n, g[2].u.n, g[3].u.n, g[4].u.n, g[5].u.n, g[6].u.n, g[7].u.n,
                zq[3], zq[5]);
  endtask
  initial begin
    #3                      show("start");
    wl[3] = 1;              show("wl[3]=1");
    wl[3] = 1;              show("wl[3]=1 again");
    wl = 8'h28;             show("wl=28");
    wl <= 8'h08;            show("wl<=08");
    pv[11:10] = 2'b10;      show("pv[11:10]=2");
    pv = 16'h0800;          show("pv=0800 same bits");
    pv[1:0] = 2'b01;        show("pv[1:0]=1");
    force wl = 8'hff;       show("force wl=ff");
    release wl;             show("release wl");
    wl = 8'h00;             show("wl=00");
    $finish;
  end
endmodule
"##;

#[test]
fn ports_connected_to_selects_wake_on_their_own_bits() {
    assert_eq!(
        run(SELECT_PORTS),
        expected(
            "\
T| 4 start n=0,0,0,0,0,0,0,0 zq3=00 zq5=00\n\
T| 5 wl[3]=1 n=0,0,0,1,0,0,0,0 zq3=80 zq5=00\n\
T| 6 wl[3]=1 again n=0,0,0,1,0,0,0,0 zq3=80 zq5=00\n\
T| 7 wl=28 n=0,0,0,1,0,1,0,0 zq3=80 zq5=80\n\
T| 8 wl<=08 n=0,0,0,1,0,2,0,0 zq3=80 zq5=10\n\
T| 9 pv[11:10]=2 n=0,0,0,1,0,3,0,0 zq3=80 zq5=42\n\
T| 10 pv=0800 same bits n=0,0,0,1,0,3,0,0 zq3=80 zq5=42\n\
T| 11 pv[1:0]=1 n=1,0,0,1,0,3,0,0 zq3=80 zq5=42\n\
T| 12 force wl=ff n=2,1,1,1,1,4,1,1 zq3=80 zq5=c8\n\
T| 13 release wl n=2,1,1,1,1,4,1,1 zq3=80 zq5=c8\n\
T| 14 wl=00 n=3,2,2,2,2,5,2,2 zq3=10 zq5=59"
        )
    );
}

/// Waits inside class tasks through a virtual interface (§25.9), with an
/// `iff` guard read through the handle, and clocking-block events with and
/// without a guard (§14.13).
const CLASS_AND_CLOCKING: &str = r##"
`timescale 1ns/1ns
interface bus_if (input logic clk);
  logic valid, ready;
  logic [7:0] data;
  clocking cb @(posedge clk);
    input valid, data;
    output ready;
  endclocking
endinterface
class drv;
  virtual bus_if vif;
  event done;
  int seen, n_mon, n_cb, n_cbv;
  function new(virtual bus_if v); vif = v; endfunction
  task run();
    forever begin
      @(posedge vif.clk iff vif.valid);
      seen++;
      if (seen == 3) -> done;
    end
  endtask
  task mon();
    forever begin @(vif.data); n_mon++; end
  endtask
  task cbw();
    repeat (2) begin @(vif.cb); n_cb++; end
    @(vif.cb iff vif.cb.valid) n_cbv++;
  endtask
endclass
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  bus_if u_if (clk);
  drv d;
  task show(input string what);
    $display("T| %0t %s seen=%0d mon=%0d cb=%0d cbv=%0d", $time, what, d.seen, d.n_mon, d.n_cb, d.n_cbv);
  endtask
  initial begin
    u_if.valid = 0; u_if.data = 0;
    d = new(u_if);
    fork d.run(); d.mon(); d.cbw(); join_none
    repeat (3) @(posedge clk);
    #1 show("3 edges");
    u_if.data = 8'd5;
    #1 show("data=5");
    @(negedge clk) u_if.valid = 1;
    repeat (2) @(posedge clk);
    #1 show("valid 2 edges");
    u_if.data <= 8'd6;
    @(d.done);
    #1 show("done");
    u_if.valid = 0;
    repeat (2) @(posedge clk);
    #1 show("end");
    $finish;
  end
endmodule
"##;

#[test]
fn class_task_interface_and_clocking_waits() {
    assert_eq!(
        run(CLASS_AND_CLOCKING),
        expected(
            "\
T| 26 3 edges seen=0 mon=0 cb=2 cbv=0\n\
T| 27 data=5 seen=0 mon=1 cb=2 cbv=0\n\
T| 46 valid 2 edges seen=2 mon=1 cb=2 cbv=1\n\
T| 56 done seen=3 mon=2 cb=2 cbv=1\n\
T| 76 end seen=3 mon=2 cb=2 cbv=1"
        )
    );
}

/// Waiters woken by one write resume last-parked first, whether the
/// scheduler finds them through the written signal (`@(s)`, `@(posedge s)`,
/// `@(s or u)`, `@(s iff u)`, `@(s ^ t)`) or examines them at every pass
/// (a term on an array cell picked by a variable index), and with idle
/// waiters parked between them (§4.7 leaves the order open; this is the
/// reference simulator's).
const WAKE_ORDER: &str = r##"
`timescale 1ns/1ns
module top;
  logic s = 0, t = 0, u = 0;
  logic [3:0] v = 0, vv = 0;
  logic q0 = 0;
  int idle_n;
  logic [7:0] big [0:1023];
  int k = 700;
  genvar g;
  initial begin #1 @(s);          $display("T| %0t w1 s", $time); end
  generate
    for (g = 0; g < 8; g++) begin : idle_a
      always @(q0) begin #0 idle_n++; end
    end
  endgenerate
  initial begin #1 @(s ^ t);      $display("T| %0t w2 s^t", $time); end
  initial begin #1 @(s or big[k]); $display("T| %0t w9 s|big[k]", $time); end
  initial begin #1 @(posedge s);  $display("T| %0t w3 posedge s", $time); end
  initial begin #1 @(vv[2]);      $display("T| %0t w4 vv[2]", $time); end
  generate
    for (g = 0; g < 8; g++) begin : idle_b
      always @(q0) begin #0 idle_n++; end
    end
  endgenerate
  initial begin #1 @(s or u);     $display("T| %0t w5 s|u", $time); end
  initial begin #1 @(v);          $display("T| %0t w6 v", $time); end
  initial begin #1 @(negedge s);  $display("T| %0t w7 negedge s", $time); end
  initial begin #1 @(s iff u);    $display("T| %0t w8 s iff u", $time); end
  initial begin
    #2 s = 1;
    #1 v = 4'b0100;
    #1 vv = 4'b0100;
    #1 u = 1;
    #1 s = 0;
    #1 q0 = 1;
    #1 $display("T| idle_n=%0d", idle_n);
    $finish;
  end
endmodule
"##;

#[test]
fn waiters_of_one_event_wake_in_the_same_order() {
    assert_eq!(
        run(WAKE_ORDER),
        expected(
            "\
T| 2 w5 s|u\n\
T| 2 w3 posedge s\n\
T| 2 w9 s|big[k]\n\
T| 2 w2 s^t\n\
T| 2 w1 s\n\
T| 3 w6 v\n\
T| 4 w4 vv[2]\n\
T| 6 w8 s iff u\n\
T| 6 w7 negedge s\n\
T| idle_n=16"
        )
    );
}
