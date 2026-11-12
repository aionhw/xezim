//! IEEE 1800-2017 §31 timing checks, kind by kind. Every expectation — the
//! violation reports (check, both event times, limit, instance, report time)
//! and each notifier / output transition — was cross-checked against the
//! reference simulator on the same source; only the message wording is
//! xezim's own.

use xezim::simulate;

/// The violation reports without their source location, and the `tb.*`
/// `%m` displays, each sorted.
fn run(src: &str) -> (Vec<String>, Vec<String>) {
    let sim = simulate(src, 1_000_000).expect("simulate failed");
    let mut violations = Vec::new();
    let mut displays = Vec::new();
    for o in &sim.output {
        let m = o.message.as_str();
        if let Some(rest) = m.strip_prefix("** Error: ") {
            if rest.contains(" violation in ") {
                let end = rest
                    .rfind(" (")
                    .filter(|&i| rest.ends_with(')') && rest[..i].contains(" at time "))
                    .unwrap_or(rest.len());
                violations.push(rest[..end].to_string());
            }
        } else if m.starts_with("tb.") {
            displays.push(m.to_string());
        }
    }
    violations.sort();
    displays.sort();
    (violations, displays)
}

fn check(src: &str, want_v: &[&str], want_d: &[&str]) {
    let (v, d) = run(src);
    assert_eq!(v, want_v, "violation reports");
    assert_eq!(d, want_d, "notifier / output displays");
}

/// `$setup`/`$hold`/`$setuphold` windows: exact limits do not violate, a data
/// event simultaneous with the clock violates the hold side only (in either
/// order), and a negative setup or hold limit moves the window past the
/// clock edge.
#[test]
fn setup_hold_setuphold_windows() {
    let src = r#"
`timescale 1ns/1ps
module c_setup(input d, input clk);
  reg n = 0;
  always @(n) $display("%m notifier=%b at %0.3f", n, $realtime);
  specify $setup(d, posedge clk, 2, n); endspecify
endmodule
module c_hold(input d, input clk);
  reg n = 0;
  always @(n) $display("%m notifier=%b at %0.3f", n, $realtime);
  specify $hold(posedge clk, d, 2, n); endspecify
endmodule
module c_sh(input d, input clk);
  reg n = 0;
  always @(n) $display("%m notifier=%b at %0.3f", n, $realtime);
  specify $setuphold(posedge clk, d, 2, 3, n); endspecify
endmodule
module c_sh_negs(input d, input clk);
  reg n = 0;
  always @(n) $display("%m notifier=%b at %0.3f", n, $realtime);
  specify $setuphold(posedge clk, d, -1, 3, n); endspecify
endmodule
module c_sh_negh(input d, input clk);
  reg n = 0;
  always @(n) $display("%m notifier=%b at %0.3f", n, $realtime);
  specify $setuphold(posedge clk, d, 3, -1, n); endspecify
endmodule
module s1;
  reg d = 0, clk = 0;
  c_setup u(d, clk);
  initial begin
    #10 d = 1; #2 clk = 1;          // data exactly limit before: 12-10=2 -> ?
    #5 clk = 0;
    #5 d = 0; #1.999 clk = 1;       // 1.999 < 2 -> violation (t=23.999)
    #5 clk = 0;
    #10 d = 1; clk = 1;             // simultaneous, d first (t=38.999)
    #5 clk = 0;
    #10 clk = 1; d = 0;             // simultaneous, clk first (t=53.999)
    #5 clk = 0;
  end
endmodule
module h1;
  reg d = 0, clk = 0;
  c_hold u(d, clk);
  initial begin
    #10 clk = 1; #2 d = 1;          // exactly limit after (12)
    #5 clk = 0;
    #5 clk = 1; #1.999 d = 0;       // violation (23.999)
    #5 clk = 0;
    #10 d = 1; clk = 1;             // simultaneous d first (38.999)
    #5 clk = 0;
    #10 clk = 1; d = 0;             // simultaneous clk first (53.999)
    #5 clk = 0;
  end
endmodule
module sh1;
  reg d = 0, clk = 0;
  c_sh u(d, clk);
  initial begin
    #10 d = 1; #1 clk = 1;          // setup viol at 11
    #1 d = 0;                       // hold viol at 12
    #5 clk = 0;
    #10 d = 1; clk = 1;             // simultaneous d first (t=27)
    #5 clk = 0;
    #10 clk = 1; d = 0;             // simultaneous clk first (t=42)
    #5 clk = 0;
    #10 d = 1; #2 clk = 1; #3 d = 0; // boundaries exact (t=57,59,62)
    #5 clk = 0;
  end
endmodule
module sh2;
  reg d = 0, clk = 0;
  c_sh_negs u(d, clk);
  initial begin
    #10 d = 1; #0.5 clk = 1;        // data 0.5 before clk (10.5)
    #5 clk = 0;
    #10 clk = 1; #0.5 d = 0;        // 0.5 after (25.5, 26)
    #5 clk = 0;
    #10 clk = 1; #2 d = 1;          // 2 after (41, 43)
    #5 clk = 0;
  end
endmodule
module sh3;
  reg d = 0, clk = 0;
  c_sh_negh u(d, clk);
  initial begin
    #10 d = 1; #0.5 clk = 1;        // 0.5 before (10, 10.5)
    #5 clk = 0;
    #10 d = 0; #2 clk = 1;          // 2 before (25.5, 27.5)
    #5 clk = 0;
    #10 clk = 1; #0.5 d = 1;        // after (42.5, 43)
    #5 clk = 0;
  end
endmodule
module tb;
  s1 s1(); h1 h1(); sh1 sh1(); sh2 sh2(); sh3 sh3();
endmodule
"#;
    let want_v: &[&str] = &[
        "$hold( posedge clk:11 ns, d:12 ns, 3 ns ) violation in tb.sh1.u at time 12 ns",
        "$hold( posedge clk:22 ns, d:23999 ps, 2 ns ) violation in tb.h1.u at time 23999 ps",
        "$hold( posedge clk:27 ns, d:27 ns, 3 ns ) violation in tb.sh1.u at time 27 ns",
        "$hold( posedge clk:38999 ps, d:38999 ps, 2 ns ) violation in tb.h1.u at time 38999 ps",
        "$hold( posedge clk:41 ns, d:43 ns, 3 ns ) violation in tb.sh2.u at time 43 ns",
        "$hold( posedge clk:42 ns, d:42 ns, 3 ns ) violation in tb.sh1.u at time 42 ns",
        "$hold( posedge clk:53999 ps, d:53999 ps, 2 ns ) violation in tb.h1.u at time 53999 ps",
        "$setup( d:10 ns, posedge clk:11 ns, 2 ns ) violation in tb.sh1.u at time 11 ns",
        "$setup( d:22 ns, posedge clk:23999 ps, 2 ns ) violation in tb.s1.u at time 23999 ps",
        "$setup( d:25500 ps, posedge clk:27500 ps, 3 ns ) violation in tb.sh3.u at time 27500 ps",
    ];
    let want_d: &[&str] = &[
        "tb.h1.u notifier=0 at 38.999",
        "tb.h1.u notifier=1 at 23.999",
        "tb.h1.u notifier=1 at 53.999",
        "tb.s1.u notifier=1 at 23.999",
        "tb.sh1.u notifier=0 at 12.000",
        "tb.sh1.u notifier=0 at 42.000",
        "tb.sh1.u notifier=1 at 11.000",
        "tb.sh1.u notifier=1 at 27.000",
        "tb.sh2.u notifier=1 at 43.000",
        "tb.sh3.u notifier=1 at 27.500",
    ];
    check(src, want_v, want_d);
}

/// `$recovery`/`$removal`/`$recrem` (recovery includes simultaneous edges,
/// removal does not), `$width` with a threshold (pulses at or below it are
/// ignored), `$period` and `$skew` (every late data event reports).
#[test]
fn recovery_removal_width_period_skew() {
    let src = r#"
`timescale 1ns/1ps
module c_rec(input rst, input clk);
  reg n = 0;
  always @(n) $display("%m notifier=%b at %0.3f", n, $realtime);
  specify $recovery(posedge rst, posedge clk, 2, n); endspecify
endmodule
module c_rem(input rst, input clk);
  reg n = 0;
  always @(n) $display("%m notifier=%b at %0.3f", n, $realtime);
  specify $removal(posedge rst, posedge clk, 2, n); endspecify
endmodule
module c_recrem(input rst, input clk);
  reg n = 0;
  always @(n) $display("%m notifier=%b at %0.3f", n, $realtime);
  specify $recrem(posedge rst, posedge clk, 2, 3, n); endspecify
endmodule
module c_width(input clk);
  reg n = 0;
  always @(n) $display("%m notifier=%b at %0.3f", n, $realtime);
  specify
    $width(posedge clk, 3, 0.5, n);
    $width(negedge clk, 2);
  endspecify
endmodule
module c_period(input clk);
  reg n = 0;
  always @(n) $display("%m notifier=%b at %0.3f", n, $realtime);
  specify $period(posedge clk, 10, n); endspecify
endmodule
module c_skew(input a, input b);
  reg n = 0;
  always @(n) $display("%m notifier=%b at %0.3f", n, $realtime);
  specify $skew(posedge a, b, 3, n); endspecify
endmodule
// run all four event patterns: rst/clk pairs at offsets -3,-2,-1,0(ref first),0(data first),1,2,3
module rec;
  reg rst = 0, clk = 0;
  c_rec u1(rst, clk);
  c_rem u2(rst, clk);
  c_recrem u3(rst, clk);
  initial begin
    #10 rst = 1; #1 clk = 1;       // clk 1 after rst (10,11)
    #4 rst = 0; clk = 0;
    #10 rst = 1; #2 clk = 1;       // clk 2 after rst (25,27)
    #4 rst = 0; clk = 0;
    #10 clk = 1; #1 rst = 1;       // rst 1 after clk (41,42)
    #4 rst = 0; clk = 0;
    #10 clk = 1; #3 rst = 1;       // rst 3 after clk (56,59)
    #4 rst = 0; clk = 0;
    #10 clk = 1; #2.5 rst = 1;     // rst 2.5 after clk (73, 75.5)
    #4 rst = 0; clk = 0;
    #10 rst = 1; clk = 1;          // simultaneous rst first (89.5)
    #4 rst = 0; clk = 0;
    #10 clk = 1; rst = 1;          // simultaneous clk first (103.5)
    #4 rst = 0; clk = 0;
  end
endmodule
module wid;
  reg clk = 0;
  c_width u(clk);
  initial begin
    #10 clk = 1; #1 clk = 0;       // high 1 -> viol (posedge width 3), low...
    #1 clk = 1;                    // low 1 -> negedge width 2 viol at 12
    #3 clk = 0;                    // high 3 exact -> no viol (15)
    #5 clk = 1; #0.3 clk = 0;      // high 0.3 < threshold 0.5 -> no viol (20, 20.3)
    #5 clk = 1; #0.5 clk = 0;      // high 0.5 == threshold -> ? (25.3,25.8)
    #5 clk = 1; clk = 0;           // zero width (30.8)
    #5 clk = 1;
    #5 clk = 0;
  end
endmodule
module per;
  reg clk = 0;
  c_period u(clk);
  initial begin
    #10 clk = 1; #5 clk = 0; #5 clk = 1;     // period 10 exact (10,20) -> no
    #4 clk = 0; #5 clk = 1;                  // period 9 (29) -> viol
    #2 clk = 0; #2 clk = 1;                  // period 4 (33) -> viol
    #20 clk = 0; #1 clk = 1;                 // (54)
  end
endmodule
module skw;
  reg a = 0, b = 0;
  c_skew u(a, b);
  initial begin
    #10 a = 1; #2 b = 1;           // skew 2 -> no
    #1 b = 0;                      // skew 3 exact (13) -> ?
    #1 b = 1;                      // skew 4 (14) -> viol
    #1 b = 0;                      // skew 5 (15) -> viol again?
    #5 a = 0; #1 b = 1;            // negedge a, b at 21 (skew from 10 = 11) -> viol?
    #5 a = 1; b = 0;               // simultaneous (26)
    #10 b = 1;                     // 36 -> viol
  end
endmodule
module tb;
  rec rec(); wid wid(); per per(); skw skw();
endmodule
"#;
    let want_v: &[&str] = &[
        "$period( posedge clk:20 ns, posedge clk:29 ns, 10 ns ) violation in tb.per.u at time 29 ns",
        "$period( posedge clk:29 ns, posedge clk:33 ns, 10 ns ) violation in tb.per.u at time 33 ns",
        "$recovery( posedge rst:10 ns, posedge clk:11 ns, 2 ns ) violation in tb.rec.u1 at time 11 ns",
        "$recovery( posedge rst:10 ns, posedge clk:11 ns, 2 ns ) violation in tb.rec.u3 at time 11 ns",
        "$recovery( posedge rst:103500 ps, posedge clk:103500 ps, 2 ns ) violation in tb.rec.u1 at time 103500 ps",
        "$recovery( posedge rst:103500 ps, posedge clk:103500 ps, 2 ns ) violation in tb.rec.u3 at time 103500 ps",
        "$recovery( posedge rst:89500 ps, posedge clk:89500 ps, 2 ns ) violation in tb.rec.u1 at time 89500 ps",
        "$recovery( posedge rst:89500 ps, posedge clk:89500 ps, 2 ns ) violation in tb.rec.u3 at time 89500 ps",
        "$removal( posedge clk:41 ns, posedge rst:42 ns, 2 ns ) violation in tb.rec.u2 at time 42 ns",
        "$removal( posedge clk:41 ns, posedge rst:42 ns, 3 ns ) violation in tb.rec.u3 at time 42 ns",
        "$removal( posedge clk:73 ns, posedge rst:75500 ps, 3 ns ) violation in tb.rec.u3 at time 75500 ps",
        "$skew( posedge a:10 ns, b:14 ns, 3 ns ) violation in tb.skw.u at time 14 ns",
        "$skew( posedge a:10 ns, b:15 ns, 3 ns ) violation in tb.skw.u at time 15 ns",
        "$skew( posedge a:10 ns, b:21 ns, 3 ns ) violation in tb.skw.u at time 21 ns",
        "$skew( posedge a:26 ns, b:36 ns, 3 ns ) violation in tb.skw.u at time 36 ns",
        "$width( negedge clk:11 ns, posedge clk:12 ns, 2 ns ) violation in tb.wid.u at time 12 ns",
        "$width( posedge clk:10 ns, negedge clk:11 ns, 3 ns ) violation in tb.wid.u at time 11 ns",
    ];
    let want_d: &[&str] = &[
        "tb.per.u notifier=0 at 33.000",
        "tb.per.u notifier=1 at 29.000",
        "tb.rec.u1 notifier=0 at 89.500",
        "tb.rec.u1 notifier=1 at 103.500",
        "tb.rec.u1 notifier=1 at 11.000",
        "tb.rec.u2 notifier=1 at 42.000",
        "tb.rec.u3 notifier=0 at 42.000",
        "tb.rec.u3 notifier=0 at 89.500",
        "tb.rec.u3 notifier=1 at 103.500",
        "tb.rec.u3 notifier=1 at 11.000",
        "tb.rec.u3 notifier=1 at 75.500",
        "tb.skw.u notifier=0 at 15.000",
        "tb.skw.u notifier=0 at 36.000",
        "tb.skw.u notifier=1 at 14.000",
        "tb.skw.u notifier=1 at 21.000",
        "tb.wid.u notifier=1 at 11.000",
    ];
    check(src, want_v, want_d);
}

/// `&&&` conditions are read after the triggering process suspends and an
/// x/z condition disables the event; `edge[...]` descriptors; bit-select
/// and vector terminals; a notifier toggles x->0 then 0/1, and a z notifier
/// stays z.
#[test]
fn conditions_edges_terminals_and_notifier_states() {
    let src = r#"
`timescale 1ns/1ps
module c_nx(input d, input clk);
  reg n;          // starts x
  reg nz = 1'bz;
  always @(n) $display("%m n=%b at %0.3f", n, $realtime);
  always @(nz) $display("%m nz=%b at %0.3f", nz, $realtime);
  specify
    $setup(d, posedge clk, 2, n);
    $hold(posedge clk, d, 2, nz);
  endspecify
endmodule
module c_cond(input d, input clk, input en);
  reg n1 = 0, n2 = 0, n3 = 0, n4 = 0;
  always @(n1) $display("%m n1(ref cond)=%b at %0.3f", n1, $realtime);
  always @(n2) $display("%m n2(data cond)=%b at %0.3f", n2, $realtime);
  always @(n3) $display("%m n3(===1)=%b at %0.3f", n3, $realtime);
  always @(n4) $display("%m n4(~en)=%b at %0.3f", n4, $realtime);
  specify
    $setup(d, posedge clk &&& en, 2, n1);
    $setup(d &&& en, posedge clk, 2, n2);
    $setup(d, posedge clk &&& (en === 1'b1), 2, n3);
    $setup(d, posedge clk &&& ~en, 2, n4);
  endspecify
endmodule
module c_edge(input d, input clk);
  reg n1 = 0, n2 = 0;
  always @(n1) $display("%m n1(edge[01,x1])=%b at %0.3f", n1, $realtime);
  always @(n2) $display("%m n2(edge[10] data)=%b at %0.3f", n2, $realtime);
  specify
    $setup(d, edge[01, x1] clk, 2, n1);
    $setup(edge[10] d, posedge clk, 2, n2);
  endspecify
endmodule
module c_vec(input [3:0] d, input clk);
  reg n1 = 0, n2 = 0;
  always @(n1) $display("%m n1(d[1])=%b at %0.3f", n1, $realtime);
  always @(n2) $display("%m n2(d)=%b at %0.3f", n2, $realtime);
  specify
    $setup(d[1], posedge clk, 2, n1);
    $setup(d, posedge clk, 2, n2);
  endspecify
endmodule
module nx;
  reg d = 0, clk = 0;
  c_nx u(d, clk);
  initial begin
    #10 d = 1; #1 clk = 1; #1 d = 0; #3 clk = 0;   // setup@11, hold@12
    #10 d = 1; #1 clk = 1; #1 d = 0; #3 clk = 0;   // setup@26, hold@27
    #10 d = 1; #1 clk = 1; #1 d = 0; #3 clk = 0;   // 41, 42
  end
endmodule
module cond;
  reg d = 0, clk = 0, en = 0;
  c_cond u(d, clk, en);
  initial begin
    #10 d = 1; #1 clk = 1; #3 clk = 0;            // en=0 (11)
    #5 en = 1; #5 d = 0; #1 clk = 1; #3 clk = 0;  // en=1 (30)
    #5 en = 1'bx; #5 d = 1; #1 clk = 1; #3 clk = 0;   // en=x (45)
    #5 en = 1'bz; #5 d = 0; #1 clk = 1; #3 clk = 0;   // en=z (60)
    #5 en = 0; #5 d = 1; en = 1; #1 clk = 1; en = 0; #3 clk = 0; // en 1 at data, 0 at ref (75)
  end
endmodule
module edg;
  reg d = 0, clk = 0;
  c_edge u(d, clk);
  initial begin
    #10 d = 1; #1 clk = 1; #3 clk = 0;            // 01 on clk; d rose (11)
    #5 d = 0; #1 clk = 1'bx; #1 clk = 1; #3 clk = 0;  // d fell 20; clk x at 21, x->1 at 22
    #5 d = 1; #1 clk = 1'bz; #1 clk = 1; #3 clk = 0; // d rose 30; clk z at 31, z->1 at 32
    #5 d = 0; #1 clk = 1'bx; #1 clk = 0; #1 clk = 1; #3 clk = 0; // 40: 0->x at 41? x->0 42, 0->1 43 
  end
endmodule
module vec;
  reg [3:0] d = 0; reg clk = 0;
  c_vec u(d, clk);
  initial begin
    #10 d = 4'b0010; #1 clk = 1; #3 clk = 0;   // bit1 changes (11)
    #5 d = 4'b0110; #1 clk = 1; #3 clk = 0;    // bit2 changes only (20)
    #5 d = 4'bx110; #1 clk = 1; #3 clk = 0;    // bit3 to x (29)
  end
endmodule
module c_edge2(input d, input clk);
  reg n1 = 0;
  always @(n1) $display("%m n1(edge[x1])=%b at %0.3f", n1, $realtime);
  specify $setup(d, edge[x1] clk, 3, n1); endspecify
endmodule
module ed2;
  reg d = 0, clk = 0;
  c_edge2 u(d, clk);
  initial begin
    #10 d = 1; #1 clk = 1'bx; #1 clk = 1;  // d 10, x 11, x->1 12 -> viol (2<3)
    #3 clk = 0;
    #10 d = 0; #1 clk = 1;                 // 25, 26: 0->1 not in mask -> no
    #3 clk = 0;
  end
endmodule
module tb;
  nx nx(); cond cond(); edg edg(); vec vec(); ed2 ed2();
endmodule
"#;
    let want_v: &[&str] = &[
        "$hold( posedge clk:11 ns, d:12 ns, 2 ns ) violation in tb.nx.u at time 12 ns",
        "$hold( posedge clk:26 ns, d:27 ns, 2 ns ) violation in tb.nx.u at time 27 ns",
        "$hold( posedge clk:41 ns, d:42 ns, 2 ns ) violation in tb.nx.u at time 42 ns",
        "$setup( d &&& en:24 ns, posedge clk:25 ns, 2 ns ) violation in tb.cond.u at time 25 ns",
        "$setup( d &&& en:66 ns, posedge clk:67 ns, 2 ns ) violation in tb.cond.u at time 67 ns",
        "$setup( d:10 ns, edge[01, x1] clk:11 ns, 2 ns ) violation in tb.edg.u at time 11 ns",
        "$setup( d:10 ns, edge[x1] clk:12 ns, 3 ns ) violation in tb.ed2.u at time 12 ns",
        "$setup( d:10 ns, posedge clk &&& ~en:11 ns, 2 ns ) violation in tb.cond.u at time 11 ns",
        "$setup( d:10 ns, posedge clk:11 ns, 2 ns ) violation in tb.nx.u at time 11 ns",
        "$setup( d:10 ns, posedge clk:11 ns, 2 ns ) violation in tb.vec.u at time 11 ns",
        "$setup( d:19 ns, posedge clk:20 ns, 2 ns ) violation in tb.vec.u at time 20 ns",
        "$setup( d:24 ns, posedge clk &&& (en === 1'b1):25 ns, 2 ns ) violation in tb.cond.u at time 25 ns",
        "$setup( d:24 ns, posedge clk &&& en:25 ns, 2 ns ) violation in tb.cond.u at time 25 ns",
        "$setup( d:25 ns, posedge clk:26 ns, 2 ns ) violation in tb.nx.u at time 26 ns",
        "$setup( d:28 ns, posedge clk:29 ns, 2 ns ) violation in tb.vec.u at time 29 ns",
        "$setup( d:40 ns, posedge clk:41 ns, 2 ns ) violation in tb.nx.u at time 41 ns",
        "$setup( d:66 ns, posedge clk &&& ~en:67 ns, 2 ns ) violation in tb.cond.u at time 67 ns",
        "$setup( d[1]:10 ns, posedge clk:11 ns, 2 ns ) violation in tb.vec.u at time 11 ns",
        "$setup( edge[10] d:19 ns, posedge clk:20 ns, 2 ns ) violation in tb.edg.u at time 20 ns",
        "$setup( edge[10] d:39 ns, posedge clk:40 ns, 2 ns ) violation in tb.edg.u at time 40 ns",
    ];
    let want_d: &[&str] = &[
        "tb.cond.u n1(ref cond)=1 at 25.000",
        "tb.cond.u n2(data cond)=0 at 67.000",
        "tb.cond.u n2(data cond)=1 at 25.000",
        "tb.cond.u n3(===1)=1 at 25.000",
        "tb.cond.u n4(~en)=0 at 67.000",
        "tb.cond.u n4(~en)=1 at 11.000",
        "tb.ed2.u n1(edge[x1])=1 at 12.000",
        "tb.edg.u n1(edge[01,x1])=1 at 11.000",
        "tb.edg.u n2(edge[10] data)=0 at 40.000",
        "tb.edg.u n2(edge[10] data)=1 at 20.000",
        "tb.nx.u n=0 at 11.000",
        "tb.nx.u n=0 at 41.000",
        "tb.nx.u n=1 at 26.000",
        "tb.vec.u n1(d[1])=1 at 11.000",
        "tb.vec.u n2(d)=0 at 20.000",
        "tb.vec.u n2(d)=1 at 11.000",
        "tb.vec.u n2(d)=1 at 29.000",
    ];
    check(src, want_v, want_d);
}

/// `$nochange` with start/end offsets; the latest data event is the setup
/// timestamp and every data event in the hold window reports; two
/// violations in one step toggle a shared notifier once; a notifier-driven
/// UDP flop goes x after capturing the clock edge.
#[test]
fn nochange_repeated_events_shared_notifier_and_udp() {
    let src = r#"
`timescale 1ns/1ps
module c_nc(input d, input clk);
  reg n = 0;
  always @(n) $display("%m n=%b at %0.3f", n, $realtime);
  specify $nochange(posedge clk, d, 1, 1, n); endspecify
endmodule
module c_multi(input d, input clk);
  reg n1 = 0, n2 = 0;
  always @(n1) $display("%m n1(setup)=%b at %0.3f", n1, $realtime);
  always @(n2) $display("%m n2(hold)=%b at %0.3f", n2, $realtime);
  specify
    $setup(d, posedge clk, 3, n1);
    $hold(posedge clk, d, 3, n2);
  endspecify
endmodule
module nc;
  reg d = 0, clk = 0;
  c_nc u(d, clk);
  initial begin
    #10 clk = 1; #2 d = 1; #2 clk = 0;     // d changes while clk high (12)
    #0.5 d = 0;                            // 14.5: within end offset 1 after negedge (14)
    #5 d = 1; #0.5 clk = 1;                // 19.5 d, 20 clk: start offset 1 before
    #5 clk = 0; #3 d = 0;                  // 25 clk fall, 28 d -> none
  end
endmodule
module mul;
  reg d = 0, clk = 0;
  c_multi u(d, clk);
  initial begin
    #10 d = 1; #1 d = 0; #1 clk = 1;       // two data before: 10, 11, clk 12 -> setup vs 11
    #1 d = 1; #1 d = 0;                    // two data after: 13, 14 -> hold both?
    #3 clk = 0;
    #10 d = 1; #1 clk = 1; #1 clk = 0; #1 clk = 1;  // d 27, clk 28, 30 -> setup both?
    #5 clk = 0;
  end
endmodule
module c_shared(input d, input clk);
  reg n = 0;
  always @(n) $display("%m shared n=%b at %0.3f", n, $realtime);
  initial $monitor("%m monitor n=%b at %0.3f", n, $realtime);
  specify
    $setup(d, posedge clk, 3, n);
    $period(posedge clk, 10, n);
  endspecify
endmodule
primitive udp_dff (q, d, clk, notifier);
  output q; reg q;
  input d, clk, notifier;
  table
  // d clk n : q : q+
     0 (01) ? : ? : 0;
     1 (01) ? : ? : 1;
     ? (1?) ? : ? : -;
     ? (?0) ? : ? : -;
     * ?    ? : ? : -;
     ? ?    * : ? : x;
  endtable
endprimitive
module ffcell(input d, input clk, output q);
  reg notifier;
  udp_dff u0(q, d, clk, notifier);
  specify
    $setuphold(posedge clk, d, 2, 1, notifier);
    $width(negedge clk, 2, 0, notifier);
  endspecify
endmodule
module shr;
  reg d = 0, clk = 0;
  c_shared u(d, clk);
  initial begin
    #10 clk = 1; #3 clk = 0;
    #3 d = 1; #1 clk = 1;                  // 17: setup viol (d 16) + period viol (7) same time
    #3 clk = 0;
    #5 $display("%m final n=%b", u.n);
  end
endmodule
module ucell;
  reg d = 0, clk = 0; wire q;
  ffcell c(d, clk, q);
  always @(q) $display("%m q=%b at %0.3f", q, $realtime);
  initial begin
    #10 d = 1; #5 clk = 1; #5 clk = 0;     // clean capture q=1 at 15
    #5 d = 0; #1 clk = 1;                  // setup viol at 26 -> q x
    #5 clk = 0; #5 d = 1; #5 clk = 1;      // clean at 41 -> q=1
    #0.5 d = 0;                            // hold viol at 41.5 -> x
    #5 clk = 0;                            // 46.5
    #1 clk = 1;                            // negedge width 1 < 2 at 47.5 -> x (clk rises also captures d=0)
    #5 clk = 0;
    #5 $display("%m final notifier=%b", c.notifier);
  end
endmodule
module tb;
  nc nc(); mul mul(); shr shr(); ucell ucell();
endmodule
"#;
    let want_v: &[&str] = &[
        "$hold( posedge clk:12 ns, d:13 ns, 3 ns ) violation in tb.mul.u at time 13 ns",
        "$hold( posedge clk:12 ns, d:14 ns, 3 ns ) violation in tb.mul.u at time 14 ns",
        "$hold( posedge clk:41 ns, d:41500 ps, 1 ns ) violation in tb.ucell.c at time 41500 ps",
        "$nochange( posedge clk:10 ns, d:12 ns, 1 ns, 1 ns ) violation in tb.nc.u at time 12 ns",
        "$nochange( posedge clk:10 ns, d:14500 ps, 1 ns, 1 ns ) violation in tb.nc.u at time 14500 ps",
        "$nochange( posedge clk:20 ns, d:19500 ps, 1 ns, 1 ns ) violation in tb.nc.u at time 20 ns",
        "$period( posedge clk:10 ns, posedge clk:17 ns, 10 ns ) violation in tb.shr.u at time 17 ns",
        "$setup( d:11 ns, posedge clk:12 ns, 3 ns ) violation in tb.mul.u at time 12 ns",
        "$setup( d:16 ns, posedge clk:17 ns, 3 ns ) violation in tb.shr.u at time 17 ns",
        "$setup( d:25 ns, posedge clk:26 ns, 2 ns ) violation in tb.ucell.c at time 26 ns",
        "$setup( d:27 ns, posedge clk:28 ns, 3 ns ) violation in tb.mul.u at time 28 ns",
        "$width( negedge clk:46500 ps, posedge clk:47500 ps, 2 ns ) violation in tb.ucell.c at time 47500 ps",
    ];
    let want_d: &[&str] = &[
        "tb.mul.u n1(setup)=0 at 28.000",
        "tb.mul.u n1(setup)=1 at 12.000",
        "tb.mul.u n2(hold)=0 at 14.000",
        "tb.mul.u n2(hold)=1 at 13.000",
        "tb.nc.u n=0 at 14.500",
        "tb.nc.u n=1 at 12.000",
        "tb.nc.u n=1 at 20.000",
        "tb.shr final n=1",
        "tb.shr.u monitor n=0 at 0.000",
        "tb.shr.u monitor n=1 at 17.000",
        "tb.shr.u shared n=1 at 17.000",
        "tb.ucell final notifier=0",
        "tb.ucell q=0 at 26.000",
        "tb.ucell q=0 at 47.500",
        "tb.ucell q=1 at 15.000",
        "tb.ucell q=1 at 41.000",
        "tb.ucell q=x at 26.000",
        "tb.ucell q=x at 41.500",
        "tb.ucell q=x at 47.500",
    ];
    check(src, want_v, want_d);
}

/// `$timeskew`/`$fullskew`: time-based by default (reported when the limit
/// elapses, the late terminal printed at that time), event-based with the
/// flag, and `remain_active` keeping an event-based `$timeskew` reporting.
#[test]
fn timeskew_fullskew_time_and_event_based() {
    let src = r#"
`timescale 1ns/1ps
module c_ts(input a, input b);
  reg n1 = 0, n2 = 0, n3 = 0;
  always @(n1) $display("%m n1(timeskew)=%b at %0.3f", n1, $realtime);
  always @(n2) $display("%m n2(fullskew)=%b at %0.3f", n2, $realtime);
  always @(n3) $display("%m n3(timeskew ebf)=%b at %0.3f", n3, $realtime);
  specify
    $timeskew(posedge a, posedge b, 3, n1);
    $fullskew(posedge a, posedge b, 3, 2, n2);
    $timeskew(posedge a, posedge b, 3, n3, 1);
  endspecify
endmodule
module ts;
  reg a = 0, b = 0;
  c_ts u(a, b);
  initial begin
    #10 a = 1; #2 b = 1;                   // skew 2 -> ok
    #1 a = 0; b = 0;
    #5 a = 1; #5 b = 1;                    // 18, 23: skew 5 -> viol (time-based at 21?)
    #1 a = 0; b = 0;
    #5 b = 1; #1 a = 1;                    // b first at 29, a at 30: fullskew reverse 1 < 2 ok
    #1 a = 0; b = 0;
    #5 b = 1; #4 a = 1;                    // b at 36, a at 40: fullskew reverse 4 > 2 viol
    #1 a = 0; b = 0;
    #20 $display("ts done");
  end
endmodule
module c_ts2(input a, input b);
  reg n1 = 0, n2 = 0, n3 = 0, n4 = 0;
  always @(n1) $display("%m n1(ts ebf raf)=%b at %0.3f", n1, $realtime);
  always @(n2) $display("%m n2(ts tb raf)=%b at %0.3f", n2, $realtime);
  always @(n3) $display("%m n3(fs ebf)=%b at %0.3f", n3, $realtime);
  always @(n4) $display("%m n4(ts ebf)=%b at %0.3f", n4, $realtime);
  specify
    $timeskew(posedge a, posedge b, 3, n1, 1, 1);
    $timeskew(posedge a, posedge b, 3, n2, 0, 1);
    $fullskew(posedge a, posedge b, 3, 2, n3, 1, 0);
    $timeskew(posedge a, posedge b, 3, n4, 1, 0);
  endspecify
endmodule
module ts2;
  reg a = 0, b = 0;
  c_ts2 u(a, b);
  initial begin
    #10 a = 1; #4 b = 1;          // a10 b14 (4 > 3)
    #1 b = 0; #1 b = 1;           // b16 again (6)
    #1 b = 0; #3 a = 0;           // 17 b0, 20 a0
    #1 a = 1; #1 b = 1;           // a21 b22 (1 ok)
    #1 b = 0; #4 b = 1;           // b27 (6 after a21)
    #1 b = 0; a = 0;              // 28
    #4 b = 1; #3 a = 1;           // b32 a35 (3 > 2 fullskew)
    #1 a = 0; b = 0;              // 36
    #4 b = 1; #1 a = 1;           // b40 a41 (1 ok)
    #1 a = 0; b = 0;              // 42
    #10 $display("ts2 done");
  end
endmodule
module tb;
  ts ts(); ts2 ts2();
endmodule
"#;
    let want_v: &[&str] = &[
        "$fullskew( posedge a:10 ns, posedge b:14 ns, 3 ns ) violation in tb.ts2.u at time 14 ns",
        "$fullskew( posedge a:18 ns, posedge b:21 ns, 3 ns ) violation in tb.ts.u at time 21 ns",
        "$fullskew( posedge a:40 ns, posedge b:43 ns, 3 ns ) violation in tb.ts.u at time 43 ns",
        "$fullskew( posedge b:16 ns, posedge a:21 ns, 2 ns ) violation in tb.ts2.u at time 21 ns",
        "$fullskew( posedge b:23 ns, posedge a:25 ns, 2 ns ) violation in tb.ts.u at time 25 ns",
        "$fullskew( posedge b:32 ns, posedge a:35 ns, 2 ns ) violation in tb.ts2.u at time 35 ns",
        "$fullskew( posedge b:36 ns, posedge a:38 ns, 2 ns ) violation in tb.ts.u at time 38 ns",
        "$timeskew( posedge a:10 ns, posedge b:13 ns, 3 ns ) violation in tb.ts2.u at time 13 ns",
        "$timeskew( posedge a:10 ns, posedge b:14 ns, 3 ns ) violation in tb.ts2.u at time 14 ns",
        "$timeskew( posedge a:10 ns, posedge b:14 ns, 3 ns ) violation in tb.ts2.u at time 14 ns",
        "$timeskew( posedge a:10 ns, posedge b:16 ns, 3 ns ) violation in tb.ts2.u at time 16 ns",
        "$timeskew( posedge a:18 ns, posedge b:21 ns, 3 ns ) violation in tb.ts.u at time 21 ns",
        "$timeskew( posedge a:18 ns, posedge b:23 ns, 3 ns ) violation in tb.ts.u at time 23 ns",
        "$timeskew( posedge a:21 ns, posedge b:27 ns, 3 ns ) violation in tb.ts2.u at time 27 ns",
        "$timeskew( posedge a:21 ns, posedge b:27 ns, 3 ns ) violation in tb.ts2.u at time 27 ns",
        "$timeskew( posedge a:21 ns, posedge b:32 ns, 3 ns ) violation in tb.ts2.u at time 32 ns",
        "$timeskew( posedge a:30 ns, posedge b:33 ns, 3 ns ) violation in tb.ts.u at time 33 ns",
        "$timeskew( posedge a:30 ns, posedge b:36 ns, 3 ns ) violation in tb.ts.u at time 36 ns",
        "$timeskew( posedge a:35 ns, posedge b:38 ns, 3 ns ) violation in tb.ts2.u at time 38 ns",
        "$timeskew( posedge a:35 ns, posedge b:40 ns, 3 ns ) violation in tb.ts2.u at time 40 ns",
        "$timeskew( posedge a:35 ns, posedge b:40 ns, 3 ns ) violation in tb.ts2.u at time 40 ns",
        "$timeskew( posedge a:40 ns, posedge b:43 ns, 3 ns ) violation in tb.ts.u at time 43 ns",
        "$timeskew( posedge a:41 ns, posedge b:44 ns, 3 ns ) violation in tb.ts2.u at time 44 ns",
    ];
    let want_d: &[&str] = &[
        "tb.ts.u n1(timeskew)=0 at 33.000",
        "tb.ts.u n1(timeskew)=1 at 21.000",
        "tb.ts.u n1(timeskew)=1 at 43.000",
        "tb.ts.u n2(fullskew)=0 at 25.000",
        "tb.ts.u n2(fullskew)=0 at 43.000",
        "tb.ts.u n2(fullskew)=1 at 21.000",
        "tb.ts.u n2(fullskew)=1 at 38.000",
        "tb.ts.u n3(timeskew ebf)=0 at 36.000",
        "tb.ts.u n3(timeskew ebf)=1 at 23.000",
        "tb.ts2.u n1(ts ebf raf)=0 at 16.000",
        "tb.ts2.u n1(ts ebf raf)=0 at 32.000",
        "tb.ts2.u n1(ts ebf raf)=1 at 14.000",
        "tb.ts2.u n1(ts ebf raf)=1 at 27.000",
        "tb.ts2.u n1(ts ebf raf)=1 at 40.000",
        "tb.ts2.u n2(ts tb raf)=0 at 38.000",
        "tb.ts2.u n2(ts tb raf)=1 at 13.000",
        "tb.ts2.u n2(ts tb raf)=1 at 44.000",
        "tb.ts2.u n3(fs ebf)=0 at 21.000",
        "tb.ts2.u n3(fs ebf)=1 at 14.000",
        "tb.ts2.u n3(fs ebf)=1 at 35.000",
        "tb.ts2.u n4(ts ebf)=0 at 27.000",
        "tb.ts2.u n4(ts ebf)=1 at 14.000",
        "tb.ts2.u n4(ts ebf)=1 at 40.000",
    ];
    check(src, want_v, want_d);
}

/// Events at time 0 set no timestamps; limits from a `specparam` in the
/// specify block, an instance parameter and a `min:typ:max` triplet; a
/// 10ns-unit module's limit counts its own unit (rounded to its precision);
/// negative limits whose sum is not positive, or a negative single limit,
/// never fire.
#[test]
fn time_zero_limits_scaling_and_negative_sums() {
    let src = r#"
`timescale 1ns/1ps
module c_w(input clk);
  specify $width(negedge clk, 10); endspecify
endmodule
module c_p(input clk);
  specify $period(negedge clk, 10); endspecify
endmodule
module c_h(input d, input clk);
  specify $hold(posedge clk, d, 5); endspecify
endmodule
module c_s(input d, input clk);
  specify $setup(d, posedge clk, 5); endspecify
endmodule
module tz1; reg clk = 0; c_w u(clk); initial #5 clk = 1; endmodule
module tz2; reg clk; c_w u(clk); initial begin clk = 0; #5 clk = 1; end endmodule
module tz3; reg clk; c_p u(clk); initial begin clk = 0; #4 clk = 1; #4 clk = 0; end endmodule
module tz4; reg clk, d = 0; c_h u(d, clk); initial begin clk = 1; #2 d = 1; end endmodule
module tz5; reg clk = 1, d = 0; c_h u(d, clk); initial begin #2 d = 1; end endmodule
module ta; reg clk = 0, d = 0; c_h u(d, clk); initial begin clk = 1; #2 d = 1; end endmodule
module tb2; reg clk, d = 0; c_h u(d, clk); initial begin #3 clk = 1; #2 d = 1; end endmodule
module tc; reg clk = 0, d; c_s u(d, clk); initial begin d = 1; #2 clk = 1; end endmodule
module td; reg clk = 0, d = 0; c_s u(d, clk); initial begin #0 d = 1; #2 clk = 1; end endmodule
module te; reg clk = 0, d; c_s u(d, clk); initial begin #1 d = 1; #2 clk = 1; end endmodule
module c1(input d, input clk); reg n = 0; always @(n) $display("%m n=%b at %0.3f", n, $realtime);
  specify $setuphold(posedge clk, d, -2, 1, n); endspecify endmodule
module c2(input d, input clk); reg n = 0; always @(n) $display("%m n=%b at %0.3f", n, $realtime);
  specify $setuphold(posedge clk, d, -1, 1, n); endspecify endmodule
module c3(input d, input clk); reg n = 0; always @(n) $display("%m n=%b at %0.3f", n, $realtime);
  specify $setuphold(posedge clk, d, 1, -2, n); endspecify endmodule
module c4(input d, input clk); reg n = 0; always @(n) $display("%m n=%b at %0.3f", n, $realtime);
  specify $setuphold(posedge clk, d, 0, 1, n); endspecify endmodule
module c5(input d, input clk); reg n = 0; always @(n) $display("%m n=%b at %0.3f", n, $realtime);
  specify $setuphold(posedge clk, d, 1, 0, n); endspecify endmodule
module c6(input d, input clk); reg n = 0; always @(n) $display("%m n=%b at %0.3f", n, $realtime);
  specify $setup(d, posedge clk, -1, n); $hold(posedge clk, d, -1, n); endspecify endmodule
module negs;
  reg d = 0, clk = 0;
  c1 u1(d, clk); c2 u2(d, clk); c3 u3(d, clk); c4 u4(d, clk); c5 u5(d, clk); c6 u6(d, clk);
  initial begin
    #10 clk = 1; #0.5 d = 1;       // data 0.5 after (10, 10.5)
    #5 clk = 0;
    #4.5 d = 0; #0.5 clk = 1;      // data 0.5 before (20, 20.5)
    #5 clk = 0;
  end
endmodule
module c_sp #(parameter real TSU = 2.0) (input d, input clk);
  reg n = 0;
  always @(n) $display("%m n(specparam/param)=%b at %0.3f", n, $realtime);
  specify
    specparam tHOLD = 1.5;
    $setup(d, posedge clk, TSU, n);
    $hold(posedge clk, d, tHOLD, n);
    $width(negedge clk, 1:2:3);
  endspecify
endmodule
module sp;
  reg d = 0, clk = 0;
  c_sp #(.TSU(3.0)) u1(d, clk);
  c_sp u2(d, clk);
  initial begin
    #10 d = 1; #2.5 clk = 1;     // 10, 12.5: 2.5 < 3 (u1 viol), not < 2 (u2 ok)
    #1 d = 0;                    // 13.5: 1 < 1.5 hold viol both
    #1 clk = 0; #1.5 clk = 1;    // 14.5 neg, 16 pos: low 1.5 < 2 (typ) width viol
    #5 clk = 0;
  end
endmodule
`timescale 10ns/1ns
module c_ts10(input d, input clk);
  reg n = 0;
  always @(n) $display("%m n(10ns unit)=%b at %0.3f", n, $realtime);
  specify $setup(d, posedge clk, 0.25, n); endspecify
endmodule
`timescale 1ns/1ps
module t10;
  reg d = 0, clk = 0;
  c_ts10 u(d, clk);
  initial begin
    #10 d = 1; #2 clk = 1;       // 10, 12: 2ns < 2.5ns -> viol
    #5 clk = 0;
    #10 d = 0; #3 clk = 1;       // 27, 30: 3 >= 2.5 ok
  end
endmodule
module tb;
  tz1 tz1(); tz2 tz2(); tz3 tz3(); tz4 tz4(); tz5 tz5();
  ta ta(); tb2 tb2(); tc tc(); td td(); te te();
  negs negs(); sp sp(); t10 t10();
endmodule
"#;
    let want_v: &[&str] = &[
        "$hold( posedge clk:10 ns, d:10500 ps, 1 ns ) violation in tb.negs.u4 at time 10500 ps",
        "$hold( posedge clk:12500 ps, d:13500 ps, 1500 ps ) violation in tb.sp.u1 at time 13500 ps",
        "$hold( posedge clk:12500 ps, d:13500 ps, 1500 ps ) violation in tb.sp.u2 at time 13500 ps",
        "$hold( posedge clk:3 ns, d:5 ns, 5 ns ) violation in tb.tb2.u at time 5 ns",
        "$setup( d:1 ns, posedge clk:3 ns, 5 ns ) violation in tb.te.u at time 3 ns",
        "$setup( d:10 ns, posedge clk:12 ns, 3 ns ) violation in tb.t10.u at time 12 ns",
        "$setup( d:10 ns, posedge clk:12500 ps, 3 ns ) violation in tb.sp.u1 at time 12500 ps",
        "$setup( d:13500 ps, posedge clk:16 ns, 3 ns ) violation in tb.sp.u1 at time 16 ns",
        "$setup( d:20 ns, posedge clk:20500 ps, 1 ns ) violation in tb.negs.u5 at time 20500 ps",
        "$width( negedge clk:14500 ps, posedge clk:16 ns, 2 ns ) violation in tb.sp.u1 at time 16 ns",
        "$width( negedge clk:14500 ps, posedge clk:16 ns, 2 ns ) violation in tb.sp.u2 at time 16 ns",
    ];
    let want_d: &[&str] = &[
        "tb.negs.u4 n=1 at 10.500",
        "tb.negs.u5 n=1 at 20.500",
        "tb.sp.u1 n(specparam/param)=0 at 13.500",
        "tb.sp.u1 n(specparam/param)=1 at 12.500",
        "tb.sp.u1 n(specparam/param)=1 at 16.000",
        "tb.sp.u2 n(specparam/param)=1 at 13.500",
        "tb.t10.u n(10ns unit)=1 at 1.200",
    ];
    check(src, want_v, want_d);
}

const SDF1: &str = r#"(DELAYFILE
  (SDFVERSION "3.0")
  (DESIGN "tb")
  (TIMESCALE 1ns)
  (CELL
    (CELLTYPE "ffc")
    (INSTANCE u)
    (TIMINGCHECK
      (SETUPHOLD D (posedge CK) (3) (2))
      (SETUP D (negedge CK) (0.7))
      (WIDTH (posedge CK) (3))
      (PERIOD (posedge CK) (8))
      (RECOVERY (posedge RN) (posedge CK) (4))
    )
  )
)
"#;

const SDF1_SRC: &str = r#"
`timescale 1ns/1ps
module ffc(input D, input CK, input RN);
  reg n = 0;
  always @(n) $display("%m n=%b at %0.3f", n, $realtime);
  specify
    $setuphold(posedge CK, D, 1, 1, n);
    $setup(D, negedge CK, 1, n);
    $width(posedge CK, 1, 0, n);
    $period(posedge CK, 1, n);
    $recovery(posedge RN, posedge CK, 1, n);
  endspecify
endmodule
module tb;
  reg d = 0, ck = 0, rn = 0;
  ffc u(d, ck, rn);
  initial begin
    ANNOTATE
    #10 d = 1; #2 ck = 1;
    #1.5 d = 0;
    #1 ck = 0;
    #5 ck = 1;
    #1 rn = 1; #2 ck = 0; #1 ck = 1;
    #1 d = 1; #0.5 ck = 0;
    #5 $finish;
  end
endmodule
"#;

/// SDF TIMINGCHECK back-annotation replaces the specify limits (all 1 ns
/// here, which nothing violates): SETUPHOLD splits into the setup and hold
/// sides, an edge-qualified SETUP picks the negedge check, and WIDTH,
/// PERIOD and RECOVERY set their checks — through `$sdf_annotate` and
/// through `--sdf`. The reference simulator reports the same nine
/// violations and notifier values from the same files.
#[test]
fn sdf_timingcheck_back_annotation() {
    let dir = std::env::temp_dir().join(format!("xezim_tchk_sdf_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sdf = dir.join("t.sdf");
    std::fs::write(&sdf, SDF1).unwrap();
    let want_v: &[&str] = &[
        "$hold( posedge CK:12 ns, D:13500 ps, 2 ns ) violation in tb.u at time 13500 ps",
        "$hold( posedge CK:23500 ps, D:24500 ps, 2 ns ) violation in tb.u at time 24500 ps",
        "$period( posedge CK:12 ns, posedge CK:19500 ps, 8 ns ) violation in tb.u at time 19500 ps",
        "$period( posedge CK:19500 ps, posedge CK:23500 ps, 8 ns ) violation in tb.u \
         at time 23500 ps",
        "$recovery( posedge RN:20500 ps, posedge CK:23500 ps, 4 ns ) violation in tb.u \
         at time 23500 ps",
        "$setup( D:10 ns, posedge CK:12 ns, 3 ns ) violation in tb.u at time 12 ns",
        "$setup( D:24500 ps, negedge CK:25 ns, 700 ps ) violation in tb.u at time 25 ns",
        "$width( posedge CK:12 ns, negedge CK:14500 ps, 3 ns ) violation in tb.u at time 14500 ps",
        "$width( posedge CK:23500 ps, negedge CK:25 ns, 3 ns ) violation in tb.u at time 25 ns",
    ];
    let want_d: &[&str] = &[
        "tb.u n=0 at 13.500",
        "tb.u n=0 at 19.500",
        "tb.u n=0 at 24.500",
        "tb.u n=1 at 12.000",
        "tb.u n=1 at 14.500",
        "tb.u n=1 at 23.500",
        "tb.u n=1 at 25.000",
    ];
    let src = SDF1_SRC.replace(
        "ANNOTATE",
        &format!("$sdf_annotate(\"{}\");", sdf.display()),
    );
    check(&src, want_v, want_d);

    let sv = dir.join("t.sv");
    std::fs::write(&sv, SDF1_SRC.replace("ANNOTATE", "")).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--sdf")
        .arg(&sdf)
        .arg(&sv)
        .output()
        .expect("run xezim");
    let text = String::from_utf8_lossy(&out.stdout);
    for want in want_v {
        assert!(text.contains(want), "--sdf: missing `{want}`:\n{text}");
    }
    assert_eq!(
        text.matches(" violation in ").count(),
        want_v.len(),
        "--sdf:\n{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// SDF `INSTANCE *` annotates every instance of the cell type, a `COND`
/// entry only the check with that condition (`SE==0` names `SE == 1'b0`,
/// not `SE == 1'b1`), and RECREM sets both sides of `$recrem`
/// (cross-checked against the reference simulator).
#[test]
fn sdf_timingcheck_wildcard_cond_recrem() {
    let dir = std::env::temp_dir().join(format!("xezim_tchk_sdf2_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sdf = dir.join("t.sdf");
    std::fs::write(
        &sdf,
        r#"(DELAYFILE
  (SDFVERSION "3.0")
  (DESIGN "tb")
  (TIMESCALE 1ns)
  (CELL
    (CELLTYPE "ffc2")
    (INSTANCE *)
    (TIMINGCHECK
      (SETUP D (COND SE==0 (posedge CK)) (3))
      (RECREM (posedge RN) (posedge CK) (4) (2))
    )
  )
)
"#,
    )
    .unwrap();
    let src = r#"
`timescale 1ns/1ps
module ffc2(input D, input CK, input SE, input RN);
  reg n = 0;
  always @(n) $display("%m n=%b at %0.3f", n, $realtime);
  specify
    $setup(D, posedge CK &&& (SE == 1'b0), 1, n);
    $setup(D, posedge CK &&& (SE == 1'b1), 1, n);
    $recrem(posedge RN, posedge CK, 1, 1, n);
  endspecify
endmodule
module tb;
  reg d = 0, ck = 0, se = 0, rn = 0;
  ffc2 u1(d, ck, se, rn);
  ffc2 u2(d, ck, se, rn);
  initial begin
    $sdf_annotate("SDF_PATH");
    #10 d = 1; #2 ck = 1;        // SE=0: setup 2 < 3 (SDF)
    #3 ck = 0;
    #2 se = 1;
    #3 d = 0; #2 ck = 1;         // SE=1: setup 2 >= 1 (not annotated)
    #3 ck = 0;
    #5 rn = 1; #3 ck = 1;        // recovery 3 < 4
    #1 rn = 0; #1 ck = 0;
    #5 ck = 1; #1 rn = 1;        // removal 1 < 2
    #5 $finish;
  end
endmodule
"#
    .replace("SDF_PATH", &sdf.display().to_string());
    let want_v: &[&str] = &[
        "$recovery( posedge RN:30 ns, posedge CK:33 ns, 4 ns ) violation in tb.u1 at time 33 ns",
        "$recovery( posedge RN:30 ns, posedge CK:33 ns, 4 ns ) violation in tb.u2 at time 33 ns",
        "$removal( posedge CK:40 ns, posedge RN:41 ns, 2 ns ) violation in tb.u1 at time 41 ns",
        "$removal( posedge CK:40 ns, posedge RN:41 ns, 2 ns ) violation in tb.u2 at time 41 ns",
        "$setup( D:10 ns, posedge CK &&& (SE == 1'b0):12 ns, 3 ns ) violation in tb.u1 \
         at time 12 ns",
        "$setup( D:10 ns, posedge CK &&& (SE == 1'b0):12 ns, 3 ns ) violation in tb.u2 \
         at time 12 ns",
    ];
    let want_d: &[&str] = &[
        "tb.u1 n=0 at 33.000",
        "tb.u1 n=1 at 12.000",
        "tb.u1 n=1 at 41.000",
        "tb.u2 n=0 at 33.000",
        "tb.u2 n=1 at 12.000",
        "tb.u2 n=1 at 41.000",
    ];
    check(&src, want_v, want_d);
    let _ = std::fs::remove_dir_all(&dir);
}
