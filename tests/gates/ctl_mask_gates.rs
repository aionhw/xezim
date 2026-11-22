//! Controlling-value skip (`CtlGate`): an AND/OR (NAND/NOR) gate whose
//! other input holds the controlling value ignores its high-fanout input
//! (a clock). The skip must be invisible: every design below runs with the
//! skip, without it (`XEZIM_CTL_MASK=0`), in cycle mode (eager clock tree)
//! and with the dirty-edge scan off, and all runs must print the same lines.
//! Where the semantics are settled the lines are also pinned to values taken
//! from a reference simulator: x/z on either input (x AND 0 = 0, x AND 1 =
//! x), NAND/NOR/OR, the hot input first or second, the enable changing in the
//! same time step as the clock (both orders), gated-clock edges counted
//! through NBA-driven and decoder-driven enables, multi-input primitives,
//! the same signal on both inputs, two-state enables, a clock that is x at
//! time 0, and force/release of either input.
//!
//! Gate outputs written from outside (force on the output net, `$deposit`)
//! re-arm the gate; there xezim's existing behaviour (the gate re-drives the
//! net at its next input event) is what the skip has to preserve, so that
//! design is compared across the modes only.

use std::process::Command;

fn run(name: &str, src: &str, env: &[(&str, &str)], extra: &[&str]) -> (Vec<String>, String) {
    let dir = std::env::temp_dir().join(format!("xezim_ctl_mask_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join(format!("{name}.sv"));
    std::fs::write(&sv, src).expect("write sv");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.arg("--no-cache")
        .args(extra)
        .arg("-s")
        .arg("tb")
        .arg(&sv);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{name} failed:\n{text}");
    let lines = text
        .lines()
        .filter_map(|l| l.strip_prefix("T| "))
        .map(str::to_string)
        .collect();
    (lines, text)
}

/// The skip took effect on `want` gates (or at least one when `None`).
fn assert_fired(name: &str, src: &str, want: Option<usize>) {
    let (_, text) = run(name, src, &[], &["--sim-debug", "--max-time", "1"]);
    let n: usize = text
        .lines()
        .find_map(|l| {
            l.strip_prefix("[CTL-MASK] ")
                .and_then(|r| r.split_whitespace().next())
                .and_then(|n| n.parse().ok())
        })
        .unwrap_or(0);
    match want {
        Some(w) => assert_eq!(n, w, "{name}: controlled gates"),
        None => assert!(n > 0, "{name}: no controlled gate formed"),
    }
}

/// Same lines with the skip, without it, in cycle mode and with the
/// dirty-edge scan off; returns them.
fn same_in_all_modes(name: &str, src: &str) -> Vec<String> {
    let (on, _) = run(name, src, &[], &[]);
    for env in [
        &[("XEZIM_CTL_MASK", "0")][..],
        &[("XEZIM_CYCLE_MODE", "cycle")][..],
        &[("XEZIM_CYCLE_MODE", "cycle"), ("XEZIM_CTL_MASK", "0")][..],
        &[("XEZIM_DIRTY_EDGE", "0")][..],
    ] {
        let (other, _) = run(name, src, env, &[]);
        assert_eq!(on, other, "{name}: output differs under {env:?}");
    }
    on
}

fn check(name: &str, src: &str, expected: &[&str], gates: Option<usize>) {
    assert_fired(name, src, gates);
    let got = same_in_all_modes(name, src);
    assert_eq!(got, expected, "{name}: differs from the reference values");
}

/// §10.3, §11.4.11: a pure mux with a known selector can reuse an
/// unchanged output slice. An unknown selector still merges both arms.
#[test]
fn compiled_mux_guard_preserves_values_and_observed_edges() {
    let design = include_str!("compiled_mux_pruning.sv");
    let (baseline, off) = run(
        "compiled_mux_off",
        design,
        &[("XEZIM_CTL_MUX", "0")],
        &["--verbose"],
    );
    let (guarded, on) = run(
        "compiled_mux_on",
        design,
        &[("XEZIM_CTL_MUX", "1")],
        &["--verbose"],
    );
    assert_eq!(baseline, guarded);
    assert!(
        on.contains("[CTL-MUX] eligible continuous assignments=1\n"),
        "{on}"
    );
    let evaluations = |log: &str| {
        log.lines()
            .find_map(|line| line.strip_prefix("[PROF] settle_calls="))
            .and_then(|line| line.split_once(" entry_evals="))
            .and_then(|(_, tail)| tail.split_whitespace().next())
            .and_then(|count| count.parse::<u64>().ok())
            .expect("settle evaluation count")
    };
    assert!(evaluations(&on) < evaluations(&off), "{on}");
}

const T_VALS: &str = r#"
module tb;
  localparam N = 64;
  reg clk = 0;
  reg [N-1:0] en = 0;
  wire [N-1:0] ga, go, gna, gno, gp, gb;
  wire clkb = ~clk;
  genvar i;
  generate for (i = 0; i < N; i = i + 1) begin : u
    assign ga[i] = clk & en[i];
    assign go[i] = clk | en[i];
    assign gna[i] = ~(clk & en[i]);
    assign gno[i] = ~(clk | en[i]);
    assign gp[i] = en[i] & clk;
    assign gb[i] = en[i] & clkb;
  end endgenerate
  task show;
    $display("T| %0t %b %h %h %h %h %h %h %h %0d", $time, clk, en[15:0], ga[15:0], go[15:0],
             gna[15:0], gno[15:0], gp[15:0], gb[15:0],
             $countones(ga) + $countones(go) + $countones(gna) + $countones(gno) + $countones(gb));
  endtask
  task tick; begin #5 clk = ~clk; #1 show; end endtask
  integer k;
  initial begin
    #0 show;
    #1 show;
    tick; tick;
    en[0] = 1; en[1] = 1'bx; en[2] = 1'bz; en[3] = 1; #1 show;
    tick; tick; tick;
    #4; clk = ~clk; en[4] = 1; en[5] = 1; #1 show;
    #4; en[6] = 1; clk = ~clk; #1 show;
    #4; clk = ~clk; en[4] = 0; #1 show;
    #4; en[5] = 0; clk = ~clk; en[5] = 1; #1 show;
    #4; en[8] = 1'bx; clk = ~clk; en[8] = 0; #1 show;
    tick; tick;
    #4; clk = 1'bx; #1 show;
    #4; clk = 1'bz; #1 show;
    #4; clk = 1; #1 show;
    #4; clk = 1'bx; en[9] = 1'bz; #1 show;
    #4; clk = 0; en[9] = 0; #1 show;
    for (k = 10; k < N; k = k + 7) en[k] = ~en[k];
    tick; tick; tick;
    en = {N{1'b1}}; tick; tick;
    en = {N{1'bx}}; tick; tick;
    en = 0; tick; tick;
    $display("T| done");
    $finish;
  end
endmodule
"#;

#[test]
fn controlled_gate_values() {
    check(
        "t_vals",
        T_VALS,
        &[
            "0 0 0000 0000 0000 ffff ffff 0000 0000 128",
            "1 0 0000 0000 0000 ffff ffff 0000 0000 128",
            "7 1 0000 0000 ffff ffff 0000 0000 0000 128",
            "13 0 0000 0000 0000 ffff ffff 0000 0000 128",
            "14 0 000X 0000 000X ffff fffX 0000 000X 128",
            "20 1 000X 000X ffff fffX 0000 000X 0000 126",
            "26 0 000X 0000 000X ffff fffX 0000 000X 128",
            "32 1 000X 000X ffff fffX 0000 000X 0000 126",
            "37 0 003X 0000 003X ffff ffcX 0000 003X 130",
            "42 1 007X 007X ffff ff8X 0000 007X 0000 126",
            "47 0 006X 0000 006X ffff ff9X 0000 006X 130",
            "52 1 006X 006X ffff ff9X 0000 006X 0000 126",
            "57 0 006X 0000 006X ffff ff9X 0000 006X 130",
            "63 1 006X 006X ffff ff9X 0000 006X 0000 126",
            "69 0 006X 0000 006X ffff ff9X 0000 006X 130",
            "74 x 006X 00Xx xxXX ffXx xxXX 00Xx 00Xx 62",
            "79 z 006X 00Xx xxXX ffXx xxXX 00Xx 00Xx 62",
            "84 1 006X 006X ffff ff9X 0000 006X 0000 126",
            "89 x 0Z6X 0XXx xxXX fXXx xxXX 0XXx 0XXx 61",
            "94 0 006X 0000 006X ffff ff9X 0000 006X 130",
            "100 1 046X 046X ffff fb9X 0000 046X 0000 126",
            "106 0 046X 0000 046X ffff fb9X 0000 046X 138",
            "112 1 046X 046X ffff fb9X 0000 046X 0000 126",
            "118 0 ffff 0000 ffff ffff 0000 0000 ffff 192",
            "124 1 ffff ffff ffff 0000 0000 ffff 0000 128",
            "130 0 xxxx 0000 xxxx ffff xxxx 0000 xxxx 64",
            "136 1 xxxx xxxx ffff xxxx 0000 xxxx 0000 64",
            "142 0 0000 0000 0000 ffff ffff 0000 0000 128",
            "148 1 0000 0000 ffff ffff 0000 0000 0000 128",
            "done",
        ],
        None,
    );
}

const T_EDGES: &str = r#"
module tb;
  localparam N = 64;
  reg clk = 0;
  reg [5:0] addr = 0; reg sel = 0;
  reg [N-1:0] en_nba = 0;
  wire [N-1:0] rs, wl, gn, go;
  genvar i;
  generate for (i = 0; i < N; i = i + 1) begin : u
    assign rs[i] = sel & (addr == i);
    assign wl[i] = clk & rs[i];
    assign gn[i] = clk & en_nba[i];
    assign go[i] = ~(clk | ~en_nba[i]);
  end endgenerate
  integer cw [0:N-1]; integer cn [0:N-1]; integer co [0:N-1];
  integer k, sw, sn, so, step;
  generate for (i = 0; i < N; i = i + 1) begin : c
    always @(posedge wl[i]) cw[i] = cw[i] + 1;
    always @(posedge gn[i]) cn[i] = cn[i] + 1;
    always @(posedge go[i]) co[i] = co[i] + 1;
  end endgenerate
  always @(posedge clk) en_nba <= {en_nba[N-2:0], ~en_nba[N-1]} ^ (en_nba >> 3);
  task show;
    begin
      sw = 0; sn = 0; so = 0;
      for (k = 0; k < N; k = k + 1) begin sw = sw + cw[k] * (k + 1); sn = sn + cn[k] * (k + 1); so = so + co[k] * (k + 1); end
      $display("T| %0t %b %b %0d %h %0d %0d %0d %0d", $time, clk, sel, addr, wl, $countones(gn), sw, sn, so);
    end
  endtask
  initial begin
    for (k = 0; k < N; k = k + 1) begin cw[k] = 0; cn[k] = 0; co[k] = 0; end
    #1 show;
    repeat (4) begin #5 clk = ~clk; #1 show; end
    sel = 1; addr = 3; #1 show;
    repeat (4) begin #4 clk = ~clk; #1 show; end
    #4 addr = 5; clk = ~clk; #1 show;
    #4 clk = ~clk; addr = 6; #1 show;
    #4 clk = ~clk; addr = 7; #1 show;
    #4 sel = 0; clk = ~clk; sel = 1; #1 show;
    #4 addr = 8; clk = ~clk; addr = 9; #1 show;
    for (step = 0; step < 40; step = step + 1) begin
      #3 clk = ~clk; addr = (addr * 7 + 3) % N; #1;
      if (step % 5 == 4) show;
    end
    repeat (20) #4 clk = ~clk;
    #1 show;
    $display("T| done");
    $finish;
  end
endmodule
"#;

#[test]
fn controlled_gate_edges() {
    check(
        "t_edges",
        T_EDGES,
        &[
            "1 0 0 0 0000000000000000 0 0 0 0",
            "7 1 0 0 0000000000000000 1 0 1 0",
            "13 0 0 0 0000000000000000 0 0 1 1",
            "19 1 0 0 0000000000000000 2 0 4 1",
            "25 0 0 0 0000000000000000 0 0 4 4",
            "26 0 1 3 0000000000000000 0 0 4 4",
            "31 1 1 3 0000000000000008 3 4 10 4",
            "36 0 1 3 0000000000000000 0 4 10 10",
            "41 1 1 3 0000000000000008 4 8 20 10",
            "46 0 1 3 0000000000000000 0 8 20 20",
            "51 1 1 5 0000000000000020 4 14 35 20",
            "56 0 1 6 0000000000000000 0 14 35 34",
            "61 1 1 7 0000000000000080 5 22 55 34",
            "66 0 1 7 0000000000000000 0 22 55 54",
            "71 1 1 9 0000000000000200 5 32 82 54",
            "91 0 1 50 0000000000000000 0 76 155 146",
            "111 1 1 49 0002000000000000 8 202 322 241",
            "131 0 1 10 0000000000000000 0 262 479 446",
            "151 1 1 25 0000000002000000 9 316 791 628",
            "171 0 1 34 0000000000000000 0 392 1053 961",
            "191 1 1 1 0000000000000002 12 502 1527 1241",
            "211 0 1 58 0000000000000000 0 530 1935 1778",
            "231 1 1 41 0000020000000000 15 632 2651 2196",
            "312 1 1 41 0000020000000000 20 1052 6176 5246",
            "done",
        ],
        None,
    );
}

const T_MISC: &str = r#"
// Controlling-value skip: multi-input primitives, the same signal on both
// inputs, two-state enables, a clock that is x at time 0, and time-0 values.
module tb;
  localparam N = 70;
  reg clk;                      // x until the first assignment
  reg [N-1:0] en = 0;
  bit [N-1:0] en2 = 0;           // two-state enables
  reg a = 1, b = 0;
  wire [N-1:0] ga, g2;
  wire y3, y4, y5, y6, ys, yt;
  genvar i;
  generate for (i = 0; i < N; i = i + 1) begin : u
    assign ga[i] = clk & en[i];
    assign g2[i] = en2[i] | clk;
  end endgenerate
  and  a3 (y3, clk, a, b);
  or   o3 (y4, clk, a, b);
  nand n3 (y5, a, clk, b);
  nor  r3 (y6, b, a, clk);
  assign ys = en[0] & en[1];     // the same signal on both inputs
  assign yt = en[2] | ~en[2];
  integer p3 = 0, p4 = 0;
  always @(posedge y3) p3 = p3 + 1;
  always @(posedge y4) p4 = p4 + 1;
  task show;
    $display("T| t=%0t clk=%b en=%h en2=%h ga=%h g2=%h a=%b b=%b y=%b%b%b%b ys=%b yt=%b p=%0d,%0d",
             $time, clk, en, en2, ga, g2, a, b, y3, y4, y5, y6, ys, yt, p3, p4);
  endtask
  initial begin
    #0 show;
    #1 show;
    clk = 0; #1 show;
    repeat (3) begin #4 clk = ~clk; #1 show; end
    b = 1; #1 show;
    repeat (3) begin #4 clk = ~clk; #1 show; end
    a = 0; b = 0; #1 show;
    repeat (3) begin #4 clk = ~clk; #1 show; end
    en[0] = 1; #1 show; en[1] = 1; #1 show; en[0] = 0; #1 show;
    en[2] = 1'bx; #1 show; en[2] = 1; #1 show;
    en2[5] = 1; en2[6] = 1; #1 show;
    repeat (3) begin #4 clk = ~clk; #1 show; end
    en = {N{1'b1}}; en2 = 0;
    repeat (3) begin #4 clk = ~clk; #1 show; end
    $display("T| done");
    $finish;
  end
endmodule
"#;

#[test]
fn controlled_gate_shapes() {
    check(
        "t_misc",
        T_MISC,
        &[
            "t=0 clk=x en=000000000000000000 en2=000000000000000000 ga=000000000000000000 g2=xxxxxxxxxxxxxxxxxx a=1 b=0 y=0110 ys=0 yt=1 p=0,1",
            "t=1 clk=x en=000000000000000000 en2=000000000000000000 ga=000000000000000000 g2=xxxxxxxxxxxxxxxxxx a=1 b=0 y=0110 ys=0 yt=1 p=0,1",
            "t=2 clk=0 en=000000000000000000 en2=000000000000000000 ga=000000000000000000 g2=000000000000000000 a=1 b=0 y=0110 ys=0 yt=1 p=0,1",
            "t=7 clk=1 en=000000000000000000 en2=000000000000000000 ga=000000000000000000 g2=3fffffffffffffffff a=1 b=0 y=0110 ys=0 yt=1 p=0,1",
            "t=12 clk=0 en=000000000000000000 en2=000000000000000000 ga=000000000000000000 g2=000000000000000000 a=1 b=0 y=0110 ys=0 yt=1 p=0,1",
            "t=17 clk=1 en=000000000000000000 en2=000000000000000000 ga=000000000000000000 g2=3fffffffffffffffff a=1 b=0 y=0110 ys=0 yt=1 p=0,1",
            "t=18 clk=1 en=000000000000000000 en2=000000000000000000 ga=000000000000000000 g2=3fffffffffffffffff a=1 b=1 y=1100 ys=0 yt=1 p=1,1",
            "t=23 clk=0 en=000000000000000000 en2=000000000000000000 ga=000000000000000000 g2=000000000000000000 a=1 b=1 y=0110 ys=0 yt=1 p=1,1",
            "t=28 clk=1 en=000000000000000000 en2=000000000000000000 ga=000000000000000000 g2=3fffffffffffffffff a=1 b=1 y=1100 ys=0 yt=1 p=2,1",
            "t=33 clk=0 en=000000000000000000 en2=000000000000000000 ga=000000000000000000 g2=000000000000000000 a=1 b=1 y=0110 ys=0 yt=1 p=2,1",
            "t=34 clk=0 en=000000000000000000 en2=000000000000000000 ga=000000000000000000 g2=000000000000000000 a=0 b=0 y=0011 ys=0 yt=1 p=2,1",
            "t=39 clk=1 en=000000000000000000 en2=000000000000000000 ga=000000000000000000 g2=3fffffffffffffffff a=0 b=0 y=0110 ys=0 yt=1 p=2,2",
            "t=44 clk=0 en=000000000000000000 en2=000000000000000000 ga=000000000000000000 g2=000000000000000000 a=0 b=0 y=0011 ys=0 yt=1 p=2,2",
            "t=49 clk=1 en=000000000000000000 en2=000000000000000000 ga=000000000000000000 g2=3fffffffffffffffff a=0 b=0 y=0110 ys=0 yt=1 p=2,3",
            "t=50 clk=1 en=000000000000000001 en2=000000000000000000 ga=000000000000000001 g2=3fffffffffffffffff a=0 b=0 y=0110 ys=0 yt=1 p=2,3",
            "t=51 clk=1 en=000000000000000003 en2=000000000000000000 ga=000000000000000003 g2=3fffffffffffffffff a=0 b=0 y=0110 ys=1 yt=1 p=2,3",
            "t=52 clk=1 en=000000000000000002 en2=000000000000000000 ga=000000000000000002 g2=3fffffffffffffffff a=0 b=0 y=0110 ys=0 yt=1 p=2,3",
            "t=53 clk=1 en=00000000000000000X en2=000000000000000000 ga=00000000000000000X g2=3fffffffffffffffff a=0 b=0 y=0110 ys=0 yt=x p=2,3",
            "t=54 clk=1 en=000000000000000006 en2=000000000000000000 ga=000000000000000006 g2=3fffffffffffffffff a=0 b=0 y=0110 ys=0 yt=1 p=2,3",
            "t=55 clk=1 en=000000000000000006 en2=000000000000000060 ga=000000000000000006 g2=3fffffffffffffffff a=0 b=0 y=0110 ys=0 yt=1 p=2,3",
            "t=60 clk=0 en=000000000000000006 en2=000000000000000060 ga=000000000000000000 g2=000000000000000060 a=0 b=0 y=0011 ys=0 yt=1 p=2,3",
            "t=65 clk=1 en=000000000000000006 en2=000000000000000060 ga=000000000000000006 g2=3fffffffffffffffff a=0 b=0 y=0110 ys=0 yt=1 p=2,4",
            "t=70 clk=0 en=000000000000000006 en2=000000000000000060 ga=000000000000000000 g2=000000000000000060 a=0 b=0 y=0011 ys=0 yt=1 p=2,4",
            "t=75 clk=1 en=3fffffffffffffffff en2=000000000000000000 ga=3fffffffffffffffff g2=3fffffffffffffffff a=0 b=0 y=0110 ys=1 yt=1 p=2,5",
            "t=80 clk=0 en=3fffffffffffffffff en2=000000000000000000 ga=000000000000000000 g2=000000000000000000 a=0 b=0 y=0011 ys=1 yt=1 p=2,5",
            "t=85 clk=1 en=3fffffffffffffffff en2=000000000000000000 ga=3fffffffffffffffff g2=3fffffffffffffffff a=0 b=0 y=0110 ys=1 yt=1 p=2,6",
            "done",
        ],
        None,
    );
}

const T_FORCE: &str = r#"
module tb;
  localparam N = 64;
  reg clk = 0;
  reg [N-1:0] en = 0;
  reg es = 0;
  wire [N-1:0] ga;
  wire gs = clk & es;
  genvar i;
  generate for (i = 0; i < N; i = i + 1) begin : u
    assign ga[i] = clk & en[i];
  end endgenerate
  integer c1 = 0, cs = 0;
  always @(posedge ga[1]) c1 = c1 + 1;
  always @(posedge gs) cs = cs + 1;
  task show;
    $display("T| %0t %b %h %h %b %b %0d %0d", $time, clk, en[7:0], ga[7:0], es, gs, c1, cs);
  endtask
  task tick; begin #5 clk = ~clk; #1 show; end endtask
  initial begin
    #1 show;
    tick; tick;
    es = 1; tick; tick;
    force es = 1'b0; #1 show;
    tick; tick;
    release es; #1 show;
    tick; tick;
    force en = 8'h02; #1 show;
    tick; tick;
    release en; #1 show;
    tick; tick;
    en[1] = 1; tick;
    force clk = 1'b1; #1 show;
    en[1] = 0; #1 show;
    en[1] = 1; #1 show;
    release clk; #1 show;
    tick; tick;
    $display("T| done");
    $finish;
  end
endmodule
"#;

#[test]
fn controlled_gate_force_inputs() {
    check(
        "t_force",
        T_FORCE,
        &[
            "1 0 00 00 0 0 0 0",
            "7 1 00 00 0 0 0 0",
            "13 0 00 00 0 0 0 0",
            "19 1 00 00 1 1 0 1",
            "25 0 00 00 1 0 0 1",
            "26 0 00 00 0 0 0 1",
            "32 1 00 00 0 0 0 1",
            "38 0 00 00 0 0 0 1",
            "39 0 00 00 0 0 0 1",
            "45 1 00 00 0 0 0 1",
            "51 0 00 00 0 0 0 1",
            "52 0 02 00 0 0 0 1",
            "58 1 02 02 0 0 1 1",
            "64 0 02 00 0 0 1 1",
            "65 0 02 00 0 0 1 1",
            "71 1 02 02 0 0 2 1",
            "77 0 02 00 0 0 2 1",
            "83 1 02 02 0 0 3 1",
            "84 1 02 02 0 0 3 1",
            "85 1 00 00 0 0 3 1",
            "86 1 02 02 0 0 4 1",
            "87 1 02 02 0 0 4 1",
            "93 0 02 00 0 0 4 1",
            "99 1 02 02 0 0 5 1",
            "done",
        ],
        None,
    );
}

const T_DEPOSIT: &str = r#"
module tb;
  localparam N = 64;
  reg clk = 0;
  reg [N-1:0] en = 0;
  wire [N-1:0] ga;
  reg es = 0;
  wire gs = clk & es;
  genvar i;
  generate for (i = 0; i < N; i = i + 1) begin : u
    assign ga[i] = clk & en[i];
  end endgenerate
  task show;
    $display("T| %0t %b %h %h %b %b", $time, clk, en[15:0], ga[15:0], es, gs);
  endtask
  task tick; begin #5 clk = ~clk; #1 show; end endtask
  initial begin
    #1 show;
    tick; tick;
    force ga[10] = 1'b1; #1 show;
    tick; tick; tick;
    release ga[10]; #1 show;
    tick;
    $deposit(ga[12], 1'b1); #1 show;
    tick; tick;
    en[12] = 1; tick; tick;
    en[12] = 0; tick;
    $deposit(ga[12], 1'b1); #1 show;
    en[12] = 1; #1 show;
    tick; tick;
    force gs = 1'b1; #1 show;
    tick; tick; tick;
    release gs; #1 show;
    tick; tick;
    es = 1; tick; tick;
    force gs = 1'b0; #1 show;
    tick; tick;
    release gs; #1 show;
    tick;
    $display("T| done");
    $finish;
  end
endmodule
"#;

#[test]
fn controlled_gate_output_overrides() {
    assert_fired("t_deposit", T_DEPOSIT, None);
    let lines = same_in_all_modes("t_deposit", T_DEPOSIT);
    assert_eq!(lines.last().map(String::as_str), Some("done"));
}

#[test]
fn mux_only_masks_the_inactive_arm_while_its_select_is_known() {
    let design = include_str!("ctl_mask_mux.sv");
    let name = "ctl_mux_four_state";
    assert_fired(name, design, None);
    let got = same_in_all_modes(name, design);
    // Independent simulator output, compressed by the fixture's four-bit period.
    let expected: Vec<String> = [
        ["1111", "0000", "0000"],
        ["1111", "1111", "0000"],
        ["01xz", "1111", "zx10"],
        ["zzzz", "01xz", "01xz"],
        ["xxxx", "01xz", "01xz"],
        ["0xxx", "x1xz", "z11z"],
        ["01xx", "11xx", "z1xx"],
        ["0000", "1111", "zx10"],
        ["0000", "0000", "0000"],
        ["0000", "1111", "zx10"],
        ["1111", "1111", "xx1x"],
        ["xxxx", "xxxx", "xxx0"],
        ["zzzz", "0000", "0000"],
        ["xxxx", "xxxx", "xxxx"],
    ]
    .iter()
    .map(|row| row.map(|nibble| nibble.repeat(16)).join(" "))
    .collect();
    assert_eq!(got, expected);
    let (vector, _) = run(name, design, &[("XEZIM_VEC_COALESCE", "1")], &[]);
    assert_eq!(vector, expected);
}
