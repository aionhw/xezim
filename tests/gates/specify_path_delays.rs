//! §30.4/§30.5 module path delays. A path delay counts the time unit of the
//! module that declares it, like every other delay: `(a => y) = 5;` in a
//! 1ns/1ps module delayed 5 ps instead of 5 ns. Besides the scaling, the
//! parser only knew `(ident => ident) = d` and kept the first value of the
//! delay list, so full `*>` paths, polarities, edge-sensitive and `if` /
//! `ifnone` paths had no delay at all, and rise/fall lists applied their
//! rise value to every transition. Every expectation below is the output of
//! the reference simulator on the same source (cross-checked against it).
//! The testbenches print only after the time-0 settle.

use std::path::PathBuf;
use std::process::Command;

/// Run xezim on `src` (plus side files) in a fresh directory; returns stdout
/// and stderr.
fn run(case: &str, src: &str, side: &[(&str, &str)], args: &[&str]) -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("specify_path_delays")
        .join(case);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("t.sv"), src).unwrap();
    for (name, text) in side {
        std::fs::write(dir.join(name), text).unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&dir)
        .args((!args.contains(&"--cache-dir")).then_some("--no-cache"))
        .args(args)
        .arg("t.sv")
        .output()
        .expect("run xezim");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The `<time> ps <name>=<value>` lines as `<time> <name>=<value>`, ordered
/// by time then name — simulators may print same-time events in any order.
fn events(out: &str) -> Vec<String> {
    let mut v: Vec<(u64, String)> = out
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let t = it.next()?.parse::<u64>().ok()?;
            (it.next()? == "ps").then_some(())?;
            let e = it.next()?;
            (it.next().is_none() && e.contains('=')).then(|| (t, e.to_string()))
        })
        .collect();
    v.sort();
    v.into_iter().map(|(t, e)| format!("{} {}", t, e)).collect()
}

fn check(case: &str, out: &str, want: &[&str]) {
    assert_eq!(events(out), want, "{case}:\n{out}");
}

const TIMESCALES: &str = r#"`timescale 1ns/1ps
module m_ns (input a, output y);
  assign y = a;
  specify (a => y) = 5; endspecify
endmodule
module m_frac (input a, output y);
  assign y = a;
  specify (a => y) = 1.25; endspecify
endmodule
module m_round (input a, output y);
  assign y = a;
  specify (a => y) = 0.0015; endspecify
endmodule
module m_lit (input a, output y);
  assign y = a;
  specify (a => y) = 2ns; endspecify
endmodule
`timescale 10ns/1ns
module m_10ns (input a, output y);
  assign y = a;
  specify
    specparam tp = 2;
    (a => y) = tp;
  endspecify
endmodule
`timescale 1ps/1ps
module m_ps (input a, output y);
  assign y = a;
  specify (a => y) = (700, 900); endspecify
endmodule
module m_tu (input a, output y);
  timeunit 100ps;
  timeprecision 1ps;
  assign y = a;
  specify (a => y) = 3; endspecify
endmodule
`timescale 1ns/1ps
module tb;
  reg a;
  wire y_ns, y_frac, y_round, y_lit, y_10ns, y_ps, y_tu;
  m_ns u_ns(a, y_ns);
  m_frac u_frac(a, y_frac);
  m_round u_round(a, y_round);
  m_lit u_lit(a, y_lit);
  m_10ns u_10ns(a, y_10ns);
  m_ps u_ps(a, y_ps);
  m_tu u_tu(a, y_tu);
  always @(y_ns)    if ($realtime >= 50) $display("%t ns=%b", $realtime, y_ns);
  always @(y_frac)  if ($realtime >= 50) $display("%t frac=%b", $realtime, y_frac);
  always @(y_round) if ($realtime >= 50) $display("%t round=%b", $realtime, y_round);
  always @(y_lit)   if ($realtime >= 50) $display("%t lit=%b", $realtime, y_lit);
  always @(y_10ns)  if ($realtime >= 50) $display("%t 10ns=%b", $realtime, y_10ns);
  always @(y_ps)    if ($realtime >= 50) $display("%t ps=%b", $realtime, y_ps);
  always @(y_tu)    if ($realtime >= 50) $display("%t tu=%b", $realtime, y_tu);
  initial begin
    $timeformat(-12, 0, " ps", 10);
    a = 0;
    #100 a = 1;
    #100 a = 0;
    #100 $finish;
  end
endmodule
"#;

/// `timescale` units of 1 ns, 10 ns and 1 ps and a `timeunit` declaration,
/// in one design: fractional delays, rounding to the declaring module's
/// precision, a time literal and a specparam.
#[test]
fn path_delays_count_the_declaring_timeunit() {
    let out = run("timescales", TIMESCALES, &[], &[]);
    check(
        "timescales",
        &out,
        &[
            "100002 round=1",
            "100300 tu=1",
            "100700 ps=1",
            "101250 frac=1",
            "102000 lit=1",
            "105000 ns=1",
            "120000 10ns=1",
            "200002 round=0",
            "200300 tu=0",
            "200900 ps=0",
            "201250 frac=0",
            "202000 lit=0",
            "205000 ns=0",
            "220000 10ns=0",
        ],
    );
}

const FORMS: &str = r#"`timescale 1ns/1ps
module c_simple (input a, output y);
  assign y = a;
  specify (a => y) = 5; endspecify
endmodule
module c_full (input a, b, output y);
  assign y = a | b;
  specify (a, b *> y) = 6; endspecify
endmodule
module c_rf (input a, output y);
  assign y = a;
  specify (a => y) = (3, 7); endspecify
endmodule
module c_six (input a, output y);
  assign y = a;
  specify (a => y) = (1, 2, 3, 4, 5, 6); endspecify
endmodule
module c_twelve (input a, output y);
  assign y = a;
  specify (a => y) = (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12); endspecify
endmodule
module c_mtm (input a, output y);
  assign y = a;
  specify (a => y) = (1:2:3); endspecify
endmodule
module c_sp (input a, output y);
  assign y = a;
  specify
    specparam tpd = 4;
    (a => y) = tpd;
  endspecify
endmodule
module c_sp_mod (input a, output y);
  specparam tpd_m = 2.5;
  assign y = a;
  specify (a => y) = tpd_m; endspecify
endmodule
module c_cond (input a, en, output y);
  assign y = a;
  specify if (en) (a => y) = 6; endspecify
endmodule
module c_ifnone (input a, en, output y);
  assign y = a;
  specify
    if (en) (a => y) = 2;
    ifnone (a => y) = 8;
  endspecify
endmodule
module c_pp (input a, output y);
  assign y = a;
  specify
    specparam PATHPULSE$a$y = (1, 2);
    (a => y) = 5;
  endspecify
endmodule
module c_pol (input a, output y);
  assign y = a;
  specify (a +=> y) = 4; endspecify
endmodule
module tb;
  reg a, b, en, clk, d;
  wire y_simple, y_full, y_rf, y_six, y_twelve, y_mtm, y_sp, y_spm, y_cond, y_ifnone, y_pp, y_pol;
  c_simple u_simple(a, y_simple);
  c_full   u_full(a, b, y_full);
  c_rf     u_rf(a, y_rf);
  c_six    u_six(a, y_six);
  c_twelve u_twelve(a, y_twelve);
  c_mtm    u_mtm(a, y_mtm);
  c_sp     u_sp(a, y_sp);
  c_sp_mod u_spm(a, y_spm);
  c_cond   u_cond(a, en, y_cond);
  c_ifnone u_ifnone(a, en, y_ifnone);
  c_pp     u_pp(a, y_pp);
  c_pol    u_pol(a, y_pol);
  always @(y_simple) if ($realtime >= 15) $display("%t simple=%b", $realtime, y_simple);
  always @(y_full)   if ($realtime >= 15) $display("%t full=%b", $realtime, y_full);
  always @(y_rf)     if ($realtime >= 15) $display("%t rf=%b", $realtime, y_rf);
  always @(y_six)    if ($realtime >= 15) $display("%t six=%b", $realtime, y_six);
  always @(y_twelve) if ($realtime >= 15) $display("%t twelve=%b", $realtime, y_twelve);
  always @(y_mtm)    if ($realtime >= 15) $display("%t mtm=%b", $realtime, y_mtm);
  always @(y_sp)     if ($realtime >= 15) $display("%t sp=%b", $realtime, y_sp);
  always @(y_spm)    if ($realtime >= 15) $display("%t spm=%b", $realtime, y_spm);
  always @(y_cond)   if ($realtime >= 15) $display("%t cond=%b", $realtime, y_cond);
  always @(y_ifnone) if ($realtime >= 15) $display("%t ifnone=%b", $realtime, y_ifnone);
  always @(y_pp)     if ($realtime >= 15) $display("%t pp=%b", $realtime, y_pp);
  always @(y_pol)    if ($realtime >= 15) $display("%t pol=%b", $realtime, y_pol);
  initial begin
    $timeformat(-12, 0, " ps", 10);
    a = 0; b = 0; en = 1; clk = 0; d = 0;
    #20 a = 1;           // 20 ns: rise with en=1
    #20 a = 0;           // 40 ns: fall with en=1
    #20 en = 0;
    #20 a = 1;           // 80 ns: rise with en=0
    #20 a = 0;           // 100 ns: fall with en=0
    #20 d = 1;
    #5 clk = 1;          // 125 ns
    #10 clk = 0;
    #20 $finish;
  end
endmodule
"#;

const FORMS_TYP: &[&str] = &[
    "21000 six=1",
    "21000 twelve=1",
    "22000 ifnone=1",
    "22000 mtm=1",
    "22500 spm=1",
    "23000 rf=1",
    "24000 pol=1",
    "24000 sp=1",
    "25000 pp=1",
    "25000 simple=1",
    "26000 cond=1",
    "26000 full=1",
    "42000 ifnone=0",
    "42000 mtm=0",
    "42000 six=0",
    "42000 twelve=0",
    "42500 spm=0",
    "44000 pol=0",
    "44000 sp=0",
    "45000 pp=0",
    "45000 simple=0",
    "46000 cond=0",
    "46000 full=0",
    "47000 rf=0",
    "80000 cond=1",
    "81000 six=1",
    "81000 twelve=1",
    "82000 mtm=1",
    "82500 spm=1",
    "83000 rf=1",
    "84000 pol=1",
    "84000 sp=1",
    "85000 pp=1",
    "85000 simple=1",
    "86000 full=1",
    "88000 ifnone=1",
    "100000 cond=0",
    "102000 mtm=0",
    "102000 six=0",
    "102000 twelve=0",
    "102500 spm=0",
    "104000 pol=0",
    "104000 sp=0",
    "105000 pp=0",
    "105000 simple=0",
    "106000 full=0",
    "107000 rf=0",
    "108000 ifnone=0",
];

/// Every path form in a 1ns/1ps module: parallel and full connections, a
/// polarity, 2/6/12-value delay lists (with the x transitions of §30.5.2),
/// a min:typ:max triplet, block and module specparams, `if` and `ifnone`,
/// and a PATHPULSE$ specparam.
#[test]
fn path_forms_scale_and_select_by_transition() {
    let out = run("forms", FORMS, &[], &[]);
    check("forms", &out, FORMS_TYP);
}

/// `+mindelays`/`+maxdelays` pick the ends of the (scaled) triplet.
#[test]
fn min_typ_max_path_delay_selection() {
    for (flag, mtm) in [
        (
            "+mindelays",
            ["21000 mtm=1", "41000 mtm=0", "81000 mtm=1", "101000 mtm=0"],
        ),
        (
            "+maxdelays",
            ["23000 mtm=1", "43000 mtm=0", "83000 mtm=1", "103000 mtm=0"],
        ),
    ] {
        let out = run(&flag[1..], FORMS, &[], &[flag]);
        let mut want: Vec<&str> = FORMS_TYP
            .iter()
            .copied()
            .filter(|e| !e.contains("mtm="))
            .chain(mtm)
            .collect();
        want.sort_by_key(|e| {
            let (t, n) = e.split_once(' ').unwrap();
            (t.parse::<u64>().unwrap(), n.to_string())
        });
        check(flag, &out, &want);
    }
}

const SELECTION: &str = r#"`timescale 1ns/1ps
module c_pp (input a, output y);
  assign y = a;
  specify
    specparam PATHPULSE$a$y = (1, 2);
    (a => y) = 5;
  endspecify
endmodule
module c_nopp (input a, output y);
  assign y = a;
  specify (a => y) = 5; endspecify
endmodule
module c_pp_all (input a, output y);
  assign y = a;
  specify
    specparam PATHPULSE$ = 3;
    (a => y) = 5;
  endspecify
endmodule
module c_two (input a, b, output y);
  assign y = a ^ b;
  specify
    (a => y) = 2;
    (b => y) = 7;
  endspecify
endmodule
module c_twocond (input a, s, t, output y);
  assign y = a;
  specify
    if (s) (a => y) = 3;
    if (t) (a => y) = 6;
  endspecify
endmodule
module c_mixed (input a, b, en, output y);
  assign y = a & b;
  specify
    (a => y) = 4;
    if (en) (b => y) = 9;
  endspecify
endmodule
module c_vec (input [3:0] d, output [3:0] q);
  assign q = d;
  specify (d => q) = 2; endspecify
endmodule
module c_vecfull (input [1:0] d, output [1:0] q);
  assign q = ~d;
  specify (d[0], d[1] *> q[0], q[1]) = (3, 4); endspecify
endmodule
module c_gate (input a, b, output y);
  and g1 (y, a, b);
  specify
    (a => y) = (1.5, 2.5);
    (b => y) = (1.5, 2.5);
  endspecify
endmodule
module c_z (input a, en, output y);
  bufif1 g1 (y, a, en);
  specify
    (a => y) = (1, 2, 3);
    (en => y) = (1, 2, 3);
  endspecify
endmodule
module tb;
  reg a, b, s, t, en;
  reg [3:0] d;
  wire y_pp, y_nopp, y_ppall, y_two, y_tc, y_mixed, y_gate, y_z;
  wire [3:0] q_vec;
  wire [1:0] q_vf;
  c_pp u_pp(a, y_pp);
  c_nopp u_nopp(a, y_nopp);
  c_pp_all u_ppall(a, y_ppall);
  c_two u_two(a, b, y_two);
  c_twocond u_tc(a, s, t, y_tc);
  c_mixed u_mixed(a, b, en, y_mixed);
  c_vec u_vec(d, q_vec);
  c_vecfull u_vf(d[1:0], q_vf);
  c_gate u_gate(a, b, y_gate);
  c_z u_z(a, en, y_z);
  always @(y_pp)    if ($realtime >= 15) $display("%t pp=%b", $realtime, y_pp);
  always @(y_nopp)  if ($realtime >= 15) $display("%t nopp=%b", $realtime, y_nopp);
  always @(y_ppall) if ($realtime >= 15) $display("%t ppall=%b", $realtime, y_ppall);
  always @(y_two)   if ($realtime >= 15) $display("%t two=%b", $realtime, y_two);
  always @(y_tc)    if ($realtime >= 15) $display("%t tc=%b", $realtime, y_tc);
  always @(y_mixed) if ($realtime >= 15) $display("%t mixed=%b", $realtime, y_mixed);
  always @(q_vec)   if ($realtime >= 15) $display("%t vec=%b", $realtime, q_vec);
  always @(q_vf)    if ($realtime >= 15) $display("%t vf=%b", $realtime, q_vf);
  always @(y_gate)  if ($realtime >= 15) $display("%t gate=%b", $realtime, y_gate);
  always @(y_z)     if ($realtime >= 15) $display("%t z=%b", $realtime, y_z);
  initial begin
    $timeformat(-12, 0, " ps", 10);
    a = 0; b = 0; s = 1; t = 1; en = 1; d = 0;
    #20 a = 1;            // 20: tc both conds (3 vs 6), two via a (2)
    #20 b = 1;            // 40: two via b (7); mixed via b en=1 (9); gate 0->1 via b
    #20 a = 0; b = 0;     // 60: simultaneous
    #20 s = 0;
    #1 a = 1;             // 81: tc only t (6)
    #20 t = 0;
    #1 a = 0;             // 102: tc no cond -> 0
    #20 s = 1'bx; t = 0;
    #1 a = 1;             // 123: tc cond x
    #20 d = 4'b0101;      // 143
    #20 d = 4'b1010;      // 163
    #20 a = 0;            // 183
    #20 a = 1; #0.5 a = 0;          // 203: 0.5 ns pulse
    #20 a = 1; #1.5 a = 0;          // 223.5: 1.5 ns pulse
    #20 a = 1; #2.5 a = 0;          // 245: 2.5 ns pulse
    #20 a = 1; #4 a = 0;            // 267.5: 4 ns pulse
    #20 en = 0;                      // 291.5
    #20 en = 1;                      // 311.5
    #20 $finish;
  end
endmodule
"#;

/// Which path applies: the input that changed last (the shortest delay
/// when inputs change together), the enabled conditional paths (x counts as
/// enabled; none enabled means no delay), per-bit transitions of a bus,
/// gate outputs, a turn-off delay, and inertial rejection of pulses
/// narrower than the delay — a PATHPULSE$ limit does not change that.
#[test]
fn path_selection_by_input_condition_and_bit() {
    let out = run("selection", SELECTION, &[], &[]);
    check(
        "selection",
        &out,
        &[
            "21000 z=1",
            "22000 two=1",
            "23000 tc=1",
            "25000 nopp=1",
            "25000 pp=1",
            "25000 ppall=1",
            "41500 gate=1",
            "47000 two=0",
            "49000 mixed=1",
            "62000 z=0",
            "62500 gate=0",
            "63000 tc=0",
            "64000 mixed=0",
            "65000 nopp=0",
            "65000 pp=0",
            "65000 ppall=0",
            "82000 z=1",
            "83000 two=1",
            "86000 nopp=1",
            "86000 pp=1",
            "86000 ppall=1",
            "87000 tc=1",
            "102000 tc=0",
            "104000 two=0",
            "104000 z=0",
            "107000 nopp=0",
            "107000 pp=0",
            "107000 ppall=0",
            "124000 z=1",
            "125000 two=1",
            "126000 tc=1",
            "128000 nopp=1",
            "128000 pp=1",
            "128000 ppall=1",
            "145000 vec=0101",
            "147000 vf=10",
            "165000 vec=1010",
            "166000 vf=11",
            "167000 vf=01",
            "185000 two=0",
            "185000 z=0",
            "186000 tc=0",
            "188000 nopp=0",
            "188000 pp=0",
            "188000 ppall=0",
            "224500 z=1",
            "227000 z=0",
            "246000 z=1",
            "247000 two=1",
            "249500 two=0",
            "249500 z=0",
            "268500 z=1",
            "269500 two=1",
            "270500 tc=1",
            "273500 two=0",
            "273500 z=0",
            "274500 tc=0",
            "294500 z=z",
            "313500 z=0",
        ],
    );
}

const EDGE: &str = r#"`timescale 1ns/1ps
module c_edge (input clk, rst, d, output q);
  reg q_r;
  always @(posedge clk or negedge rst)
    if (!rst) q_r <= 1'b0; else q_r <= d;
  buf (q, q_r);
  specify
    (posedge clk => (q +: d)) = (3, 4);
    (negedge rst => (q +: 1'b0)) = 2;
  endspecify
endmodule
module tb;
  reg clk, rst, d;
  wire q;
  c_edge u_edge(clk, rst, d, q);
  always @(q)  if ($realtime >= 15) $display("%t q=%b", $realtime, q);
  initial begin
    $timeformat(-12, 0, " ps", 10);
    clk = 0; rst = 1; d = 0;
    #10 d = 1;
    #5 clk = 1;          // 15: q 0->1? (q starts x)
    #5 clk = 0;
    #5 d = 0;
    #5 clk = 1;          // 30: q 1->0 (fall 4)
    #5 clk = 0;
    #5 d = 1;
    #5 clk = 1;          // 45: q 0->1 (rise 3)
    #5 rst = 0;          // 50: q 1->0 via rst (2)
    #20 $finish;
  end
endmodule
"#;

/// Edge-sensitive paths of a flop cell: clock-to-q rise and fall, and the
/// asynchronous reset path when the reset input changed last.
#[test]
fn edge_sensitive_paths() {
    let out = run("edge", EDGE, &[], &[]);
    check(
        "edge",
        &out,
        &["18000 q=1", "34000 q=0", "48000 q=1", "52000 q=0"],
    );
}

const PARAMS: &str = r#"`timescale 1ns/1ps
module c_p1 #(parameter D = 3) (input a, output y);
  assign y = a;
  specify (a => y) = D; endspecify
endmodule
module c_p3 (input a, output y);
  localparam L = 4;
  assign y = a;
  specify (a => y) = L; endspecify
endmodule
module c_p4 (input a, output y);
  assign y = a;
  specify
    specparam t1 = 2, t2 = t1 + 3;
    (a => y) = t2;
  endspecify
endmodule
module tb;
  reg a;
  wire y1, y1b, y3, y4;
  c_p1 u1(a, y1);
  c_p1 #(.D(7)) u1b(a, y1b);
  c_p3 u3(a, y3);
  c_p4 u4(a, y4);
  always @(y1)  if ($realtime >= 15) $display("%t y1=%b", $realtime, y1);
  always @(y1b) if ($realtime >= 15) $display("%t y1b=%b", $realtime, y1b);
  always @(y3)  if ($realtime >= 15) $display("%t y3=%b", $realtime, y3);
  always @(y4)  if ($realtime >= 15) $display("%t y4=%b", $realtime, y4);
  initial begin
    $timeformat(-12, 0, " ps", 10);
    a = 0;
    #20 a = 1;
    #20 $finish;
  end
endmodule
"#;

/// Delays spelled with a parameter (overridden per instance), a localparam
/// and chained specparams are evaluated in the instance's own scope — a
/// sub-instance's specparam-valued delay used to come out as 0.
#[test]
fn parameter_and_specparam_valued_delays() {
    let out = run("params", PARAMS, &[], &[]);
    check(
        "params",
        &out,
        &["23000 y1=1", "24000 y3=1", "25000 y4=1", "27000 y1b=1"],
    );
}

const PULSES: &str = r#"`timescale 1ns/1ps
module c_asg (input a, output y);
  assign y = a;
  specify (a => y) = 3; endspecify
endmodule
module c_buf (input a, output y);
  buf g (y, a);
  specify (a => y) = 3; endspecify
endmodule
module c_and (input a, b, output y);
  and g (y, a, b);
  specify (a => y) = 3; (b => y) = 3; endspecify
endmodule
module c_not (input a, output y);
  not g (y, a);
  specify (a => y) = (3, 3); endspecify
endmodule
module c_expr (input a, b, output y);
  assign y = a & ~b;
  specify (a => y) = 3; (b => y) = 3; endspecify
endmodule
module c_bus (input [1:0] a, output [1:0] y);
  assign y = a;
  specify (a => y) = 3; endspecify
endmodule
module tb;
  reg a, b;
  reg [1:0] v;
  wire y_asg, y_buf, y_and, y_not, y_expr;
  wire [1:0] y_bus;
  c_asg u1(a, y_asg);
  c_buf u2(a, y_buf);
  c_and u3(a, b, y_and);
  c_not u4(a, y_not);
  c_expr u5(a, b, y_expr);
  c_bus u6(v, y_bus);
  always @(y_asg)  if ($realtime >= 15) $display("%t asg=%b", $realtime, y_asg);
  always @(y_buf)  if ($realtime >= 15) $display("%t buf=%b", $realtime, y_buf);
  always @(y_and)  if ($realtime >= 15) $display("%t and=%b", $realtime, y_and);
  always @(y_not)  if ($realtime >= 15) $display("%t not=%b", $realtime, y_not);
  always @(y_expr) if ($realtime >= 15) $display("%t expr=%b", $realtime, y_expr);
  always @(y_bus)  if ($realtime >= 15) $display("%t bus=%b", $realtime, y_bus);
  initial begin
    $timeformat(-12, 0, " ps", 10);
    a = 0; b = 1; v = 0;
    #20 a = 1; #1 a = 0;          // 20: 1 ns pulse (rejected)
    #20 a = 1; #4 a = 0;          // 41: 4 ns pulse (passes)
    #20 v = 2'b01; #1 v = 2'b00;  // 65: bus pulse
    #20 v = 2'b01; #1 v = 2'b11;  // 86: bus change mid-flight
    #20 $finish;
  end
endmodule
"#;

/// §28.16 inertial delay on path-delayed outputs: a 1 ns pulse through a
/// 3 ns path is swallowed whatever drives the output (a plain `assign` and a
/// `buf` used to let it through), a 4 ns pulse passes, and a bus bit already
/// on its way keeps its time when another bit changes.
#[test]
fn inertial_pulse_rejection_on_path_outputs() {
    let out = run("pulses", PULSES, &[], &[]);
    check(
        "pulses",
        &out,
        &[
            "44000 and=1",
            "44000 asg=1",
            "44000 buf=1",
            "44000 not=0",
            "48000 and=0",
            "48000 asg=0",
            "48000 buf=0",
            "48000 not=1",
            "89000 bus=01",
            "90000 bus=11",
        ],
    );
}

const SDF_TB: &str = r#"`timescale 1ns/1ps
module cbuf(input a, output y);
  assign y = a;
  specify (a => y) = (8, 9); endspecify
endmodule
module t;
  reg a;
  wire y1, y2, y3;
  cbuf u1(.a(a), .y(y1));
  cbuf u2(.a(a), .y(y2));
  cbuf u3(.a(a), .y(y3));
  initial begin
    $sdf_annotate("d.sdf", t);
    $timeformat(-12, 0, " ps", 10);
    a = 0;
    #20 a = 1;
    #20 a = 0;
    #20 $finish;
  end
  always @(y1) if ($realtime >= 15) $display("%t y1=%b", $realtime, y1);
  always @(y2) if ($realtime >= 15) $display("%t y2=%b", $realtime, y2);
  always @(y3) if ($realtime >= 15) $display("%t y3=%b", $realtime, y3);
endmodule
"#;

const SDF: &str = r#"(DELAYFILE
 (SDFVERSION "3.0")
 (DESIGN "t")
 (TIMESCALE 100ps)
 (CELL (CELLTYPE "cbuf") (INSTANCE u1)
  (DELAY (ABSOLUTE (IOPATH a y (30:30:30) (30:30:30)))))
 (CELL (CELLTYPE "cbuf") (INSTANCE u3)
  (DELAY (ABSOLUTE (IOPATH a y (12:12:12) (12:12:12)))))
)
"#;

/// SDF IOPATH values (in the SDF file's own TIMESCALE) replace the scaled
/// path delays of the instances they annotate; the unannotated instance
/// keeps its specify rise/fall — through `$sdf_annotate` and through the
/// `--sdf` option alike.
#[test]
fn sdf_iopath_replaces_scaled_path_delays() {
    let want = [
        "21200 y3=1",
        "23000 y1=1",
        "28000 y2=1",
        "41200 y3=0",
        "43000 y1=0",
        "49000 y2=0",
    ];
    let out = run("sdf_task", SDF_TB, &[("d.sdf", SDF)], &[]);
    check("$sdf_annotate", &out, &want);
    let cli_tb = SDF_TB.replace("    $sdf_annotate(\"d.sdf\", t);\n", "");
    let out = run("sdf_cli", &cli_tb, &[("d.sdf", SDF)], &["--sdf", "d.sdf"]);
    check("--sdf", &out, &want);
}

/// §30.5 (#1614): building or loading a prepared cache must not collapse a
/// timed identity cell onto its input.
#[test]
fn identity_paths_keep_every_delay_through_the_design_cache() {
    let src = r#"`timescale 1ps/1ps
module link(input a, output y);
  assign y = a;
  specify (a => y) = 5; endspecify
endmodule
module tb;
  reg a;
  wire n, y;
  link u0(a,n), u1(n,y);
  initial begin a = 0; #100 a = 1; #100 $finish; end
  always @(posedge y) $display("%0d ps y=%b", $time, y);
endmodule
"#;
    for _ in 0..2 {
        let out = run("cache_identity_paths", src, &[], &["--cache-dir", "cache"]);
        check("cached path chain", &out, &["110 y=1"]);
    }
}

/// §32.4.4, §32.9 (#1608, #1611): a scoped annotation can use a slash
/// divider, and a wire delay applies at the receiver rather than the driver.
#[test]
fn scoped_sdf_keeps_interconnect_after_the_driving_path() {
    let src = r#"`timescale 1ps/1ps
module link(input a, output y);
  assign y = a;
  specify (a => y) = 1; endspecify
endmodule
module sub(input a, output n, y);
  link u0(a,n), u1(n,y);
endmodule
module tb;
  reg a;
  wire n, y;
  sub u(a,n,y);
  initial begin $sdf_annotate("d.sdf", tb.u); a=0; #100 a=1; #100 $finish; end
  always @(posedge n) $display("%0d ps n=%b", $time, n);
  always @(posedge y) $display("%0d ps y=%b", $time, y);
endmodule
"#;
    let sdf = r#"(DELAYFILE (DIVIDER /) (TIMESCALE 1ps)
      (CELL (CELLTYPE "sub") (INSTANCE)
        (DELAY (ABSOLUTE (INTERCONNECT u0/y u1/a (8)))))
      (CELL (CELLTYPE "link") (INSTANCE *)
        (DELAY (ABSOLUTE (IOPATH a y (20))))))"#;
    let out = run("scoped_sdf_interconnect", src, &[("d.sdf", sdf)], &[]);
    check(
        "scoped IOPATH and INTERCONNECT",
        &out,
        &["120 n=1", "148 y=1"],
    );
}

/// §32.4.1, §32.5, §32.8 (#1609, #1610, #1615): identity of the
/// path and all transition values survive back-annotation.
#[test]
fn sdf_delays_select_the_conditional_arc_and_transition() {
    let src = r#"`timescale 1ps/1ps
module gate2(input a,b, output y);
  and (y,a,b);
  specify
    if (b) (a => y) = (2,3);
    (b => y) = (5,6);
  endspecify
endmodule
module tb;
  reg a,b;
  wire y;
  gate2 u(a,b,y);
  initial begin
    $sdf_annotate("d.sdf", tb, , , "MAXIMUM");
    a=0; b=1;
    #100 a=1; #100 a=0; #100 a=1; #100 b=0; #100 $finish;
  end
  always @(y) if ($time > 20) $display("%0d ps y=%b", $time, y);
endmodule
"#;
    let sdf = r#"(DELAYFILE (TIMESCALE 1ps)
      (CELL (CELLTYPE "gate2") (INSTANCE u)
        (DELAY (ABSOLUTE
          (COND b (IOPATH a y (1::11) (2::12) (3::13)))
          (IOPATH b y (1:2:) (1:2:)))
        (INCREMENT (COND b (IOPATH a y (2) (3)))))))"#;
    let out = run("sdf_path_values", src, &[("d.sdf", sdf)], &[]);
    check(
        "conditional IOPATH",
        &out,
        &["113 y=1", "215 y=0", "313 y=1", "406 y=0"],
    );
}

/// §30.4 (#1607): a UDP that directly drives a module output has the
/// same path delay as an output driven through a buffer.
#[test]
fn sequential_udp_output_uses_the_annotated_clock_path() {
    let src = r#"`timescale 1ps/1ps
primitive ff(q,d,clk);
  output q; reg q;
  input d,clk;
  table
    0 (01) : ? : 0;
    1 (01) : ? : 1;
    ? (1?) : ? : -;
    ? (?0) : ? : -;
    * ? : ? : -;
  endtable
endprimitive
module flop(input d,clk, output q);
  ff u(q,d,clk);
  specify (posedge clk => (q +: d)) = (2,3); endspecify
endmodule
module tb;
  reg d,clk;
  wire q;
  flop u(d,clk,q);
  initial begin
    $sdf_annotate("d.sdf");
    clk=0; d=0; #50 clk=1; #10 clk=0; #10 d=1;
    #30 clk=1; #50 clk=0; #10 d=0; #40 clk=1; #100 $finish;
  end
  always @(q) if ($time >= 90) $display("%0d ps q=%b", $time, q);
endmodule
"#;
    let sdf = r#"(DELAYFILE (TIMESCALE 1ps)
      (CELL (CELLTYPE "flop") (INSTANCE u)
        (DELAY (ABSOLUTE (IOPATH (posedge clk) q (20) (24))))))"#;
    let out = run(
        "udp_specify",
        &src.replace("    $sdf_annotate(\"d.sdf\");", ""),
        &[],
        &[],
    );
    check("UDP specify delay", &out, &["102 q=1", "203 q=0"]);
    let out = run("udp_sdf", src, &[("d.sdf", sdf)], &[]);
    check("UDP annotated delay", &out, &["120 q=1", "224 q=0"]);
}
