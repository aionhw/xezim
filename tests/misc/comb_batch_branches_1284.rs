//! #1284: the comb-entry merge passes keep each member's control flow and
//! timing (§10.3.2, §9.2.2.1, §12.4, §12.5).
//!
//! - Four continuous assigns with one read set are evaluated as one batch.
//!   A member whose inlined function guards with a fused compare-and-branch
//!   or a jump table was batched although its branch targets are relative
//!   to its own first instruction: the later members jumped back into the
//!   first one and the time-0 settle never ended.
//! - Opt-in chain fusion (`XEZIM_FUSE_CHAINS=1`) appended a consumer to its
//!   producer without relocating the consumer's branches (wrong values, or a
//!   loop that never ended), and merged a continuous assign with a plain
//!   `always @(...)` (§9.2.2.1: the always body ran at time 0, or the
//!   continuous assign waited).
//! - A continuous assign in an instance reading a cell of a memory too large
//!   to name its elements depended on the instance's own signal, so it kept
//!   its first value when the memory was written.
//!
//! Expected values are the reference simulator's. Each run has a deadline,
//! so a settle that never ends fails the test instead of hanging it.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn t_lines(name: &str, src: &str, env: &[(&str, &str)]) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!("xezim_comb_batch_{}_{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("t.sv");
    std::fs::write(&f, src).unwrap();
    let out = dir.join("out.txt");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.args(["--no-cache", "-s", "top", "--max-time", "1000"])
        .arg(&f)
        .env_remove("XEZIM_FUSE_CHAINS")
        .stdout(std::fs::File::create(&out).unwrap())
        .stderr(Stdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("run xezim");
    let deadline = Instant::now() + Duration::from_secs(120);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{name}: the run did not finish (settle loop)");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let text = std::fs::read_to_string(&out).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    text.lines()
        .filter(|l| l.starts_with("T|"))
        .map(String::from)
        .collect()
}

const TAPS: &str = r#"
// Four continuous assigns with one read set, each an inlined function with
// guarded clamps (register compares) or a constant compare.
module grain_table (input logic [15:0] level_raw);
  localparam int NUM_PLINTH = 3;
  localparam int BUCKET_SPAN = 3;
  wire [15:0] level_gated = (level_raw > 16'd63) ? 16'd64 : level_raw;
  wire [1:0] tap_q0 = lane_bucket(level_gated, 0);
  wire [1:0] tap_q1 = lane_bucket(level_gated, 3);
  wire [1:0] tap_q2 = lane_bucket(level_gated, 6);
  wire [1:0] tap_q3 = lane_bucket(level_gated, 9);
  function automatic logic [1:0] lane_bucket(input int level, input int lane);
    int span_above, k;
    begin
      span_above = level - 1 - NUM_PLINTH;
      k = span_above - lane;
      if (k > BUCKET_SPAN)
        k = BUCKET_SPAN;
      if (k < 0)
        k = 0;
      lane_bucket = k[1:0];
    end
  endfunction
endmodule
module grain_table_const (input logic [15:0] level_raw);
  wire [15:0] level_gated = (level_raw > 16'd63) ? 16'd64 : level_raw;
  wire [1:0] tap_q0 = lane_bucket(level_gated, 0);
  wire [1:0] tap_q1 = lane_bucket(level_gated, 3);
  wire [1:0] tap_q2 = lane_bucket(level_gated, 6);
  wire [1:0] tap_q3 = lane_bucket(level_gated, 9);
  function automatic logic [1:0] lane_bucket(input int level, input int lane);
    int span_above, k, v;
    begin
      span_above = level - 4;
      k = span_above - lane;
      v = 0;
      if (k == 3)
        v = 2;
      lane_bucket = v[1:0];
    end
  endfunction
endmodule
module grain_table_case (input logic [15:0] level_raw);
  wire [15:0] level_gated = level_raw & 16'h7;
  wire [3:0] tap_q0 = sel(level_gated, 0);
  wire [3:0] tap_q1 = sel(level_gated, 1);
  wire [3:0] tap_q2 = sel(level_gated, 2);
  wire [3:0] tap_q3 = sel(level_gated, 3);
  function automatic logic [3:0] sel(input logic [15:0] l, input int lane);
    logic [3:0] r;
    case (l)
      0: r = 4'd1;
      1: r = 4'd2;
      2: r = 4'd3;
      3: r = 4'd4;
      4: r = 4'd5;
      5: r = 4'd6;
      default: r = 4'd9;
    endcase
    sel = r + lane[3:0];
  endfunction
endmodule
module top;
  logic [15:0] la, lb, lc;
  int vals [8] = '{0, 7, 10, 13, 16, 20, 5, 100};
  grain_table u_a (.level_raw(la));
  grain_table_const u_b (.level_raw(lb));
  grain_table_case u_c (.level_raw(lc));
  initial begin
    foreach (vals[i]) begin
      la = vals[i]; lb = vals[i]; lc = vals[i];
      #1 $display("T|%0d a=%0d %0d %0d %0d b=%0d %0d %0d %0d c=%0d %0d %0d %0d", vals[i],
                  u_a.tap_q0, u_a.tap_q1, u_a.tap_q2, u_a.tap_q3,
                  u_b.tap_q0, u_b.tap_q1, u_b.tap_q2, u_b.tap_q3,
                  u_c.tap_q0, u_c.tap_q1, u_c.tap_q2, u_c.tap_q3);
    end
  end
endmodule
"#;

/// §10.3.2, §12.4, §12.5: register compares, a constant compare and a
/// `case` in four same-read-set taps each.
#[test]
fn shared_read_batch_keeps_member_branches() {
    let expected = [
        "T|0 a=0 0 0 0 b=0 0 0 0 c=1 2 3 4",
        "T|7 a=3 0 0 0 b=2 0 0 0 c=9 10 11 12",
        "T|10 a=3 3 0 0 b=0 2 0 0 c=3 4 5 6",
        "T|13 a=3 3 3 0 b=0 0 2 0 c=6 7 8 9",
        "T|16 a=3 3 3 3 b=0 0 0 2 c=1 2 3 4",
        "T|20 a=3 3 3 3 b=0 0 0 0 c=5 6 7 8",
        "T|5 a=1 0 0 0 b=0 0 0 0 c=6 7 8 9",
        "T|100 a=3 3 3 3 b=0 0 0 0 c=5 6 7 8",
    ];
    assert_eq!(t_lines("taps", TAPS, &[]), expected);
    assert_eq!(
        t_lines("taps_chain", TAPS, &[("XEZIM_FUSE_CHAINS", "1")]),
        expected
    );
}

const CHAIN: &str = r#"
// Producer/sole-consumer chains: a guarded function, a counted loop, and
// a continuous assign next to a plain `always @(...)` either way round.
module rank_map (input logic [7:0] lvl, output logic [3:0] o);
  wire [7:0] s = lvl + 8'd1;
  wire [3:0] t = rank(s);
  assign o = t;
  function automatic logic [3:0] rank(input int v);
    int k;
    k = v - 3;
    if (k > 5) k = 5;
    if (k < 0) k = 0;
    return k[3:0];
  endfunction
endmodule
module popcount_map (input logic [7:0] lvl, output logic [7:0] o);
  wire [7:0] s = lvl + 8'd1;
  wire [7:0] t = bitcount(s);
  assign o = t;
  function automatic logic [7:0] bitcount(input logic [7:0] v);
    int i, n;
    logic [7:0] x;
    x = v; n = 0; i = 0;
    while (i < 8) begin
      if (x[0]) n = n + 1;
      x = x >> 1;
      i = i + 1;
    end
    return n[7:0];
  endfunction
endmodule
module defer_probe (input logic [7:0] lvl, output logic [7:0] o);
  wire [7:0] s = lvl + 8'd1;
  logic [7:0] t;
  always @(s) begin
    case (s)
      8'h00: t = 8'h11;
      8'h01: t = 8'h22;
      default: t = 8'h33;
    endcase
  end
  assign o = t;
endmodule
module defer_probe_rev (input logic [7:0] lvl, output logic [7:0] o);
  logic [7:0] t;
  always @(lvl) t = lvl & 8'h00;
  wire [7:0] u = {t[7:1], 1'b0};
  assign o = u;
endmodule
module top;
  logic [7:0] lvl;
  logic [3:0] o2;
  logic [7:0] o3, o5, o6;
  rank_map u_2 (.lvl(lvl), .o(o2));
  popcount_map u_3 (.lvl(lvl), .o(o3));
  defer_probe u_5 (.lvl(lvl), .o(o5));
  defer_probe_rev u_6 (.lvl(lvl), .o(o6));
  int vals [9] = '{0, 2, 3, 6, 7, 20, 250, 254, 255};
  initial begin
    #1 $display("T|t0 o5=%h t5=%h o6=%b", o5, u_5.t, o6);
    foreach (vals[i]) begin
      lvl = vals[i];
      #1 $display("T|%0d o2=%0d o3=%0d o5=%h o6=%h", vals[i], o2, o3, o5, o6);
    end
  end
endmodule
"#;

/// §10.3.2, §9.2.2.1: producer/sole-consumer chains with a guarded
/// function, a counted loop, and a continuous assign next to a plain
/// `always @(...)` either way round, with and without chain fusion.
#[test]
fn chain_fusion_relocates_branches_and_keeps_classes() {
    let expected = [
        "T|t0 o5=xx t5=xx o6=xxxxxxx0",
        "T|0 o2=0 o3=1 o5=22 o6=00",
        "T|2 o2=0 o3=2 o5=33 o6=00",
        "T|3 o2=1 o3=1 o5=33 o6=00",
        "T|6 o2=4 o3=3 o5=33 o6=00",
        "T|7 o2=5 o3=1 o5=33 o6=00",
        "T|20 o2=5 o3=3 o5=33 o6=00",
        "T|250 o2=5 o3=7 o5=33 o6=00",
        "T|254 o2=5 o3=8 o5=33 o6=00",
        "T|255 o2=0 o3=0 o5=11 o6=00",
    ];
    assert_eq!(t_lines("chain", CHAIN, &[]), expected);
    assert_eq!(
        t_lines("chain_fused", CHAIN, &[("XEZIM_FUSE_CHAINS", "1")]),
        expected
    );
}

const MEM_TAP: &str = r#"
// A continuous assign in an instance reading a cell of a memory too large
// to name its elements, through a dynamic and a constant index.
module mem_tap (input logic [7:0] lvl, input logic [2:0] idx, input logic clk,
                output logic [7:0] o, output logic [7:0] c);
  wire [7:0] s = lvl + idx;
  bit [7:0] mem [0:131071];
  logic [99:0] wide [0:131071];
  wire [7:0] t = s ^ mem[idx];
  assign o = t;
  assign c = mem[5] + wide[idx][7:0];
  always @(posedge clk) begin
    mem[idx] <= mem[idx] + 8'd1;
    wide[idx] <= 100'd3;
  end
endmodule
module top;
  logic [7:0] lvl; logic [2:0] idx; logic clk; logic [7:0] o, c;
  mem_tap u (.lvl(lvl), .idx(idx), .clk(clk), .o(o), .c(c));
  initial begin
    clk = 0; lvl = 10; idx = 2;
    #2 $display("T|a t=%0d o=%0d c=%0d", u.t, o, c);
    clk = 1; #1 $display("T|b t=%0d o=%0d c=%0d", u.t, o, c);
    clk = 0; #1 clk = 1; #1 $display("T|c t=%0d o=%0d c=%0d", u.t, o, c);
    clk = 0; idx = 5; #1 $display("T|d t=%0d o=%0d c=%0d", u.t, o, c);
    clk = 1; #1 $display("T|e t=%0d o=%0d c=%0d", u.t, o, c);
    idx = 2; #1 $display("T|f t=%0d o=%0d c=%0d", u.t, o, c);
  end
endmodule
"#;

/// §10.3.2, §7.4: a continuous assign in an instance re-evaluates when a
/// cell of a memory stored without element names changes (dynamic and
/// constant index, packed-arena and wide cells).
#[test]
fn instance_cont_assign_follows_unnamed_memory_cells() {
    let expected = [
        "T|a t=12 o=12 c=x",
        "T|b t=13 o=13 c=3",
        "T|c t=14 o=14 c=3",
        "T|d t=15 o=15 c=x",
        "T|e t=14 o=14 c=4",
        "T|f t=14 o=14 c=4",
    ];
    assert_eq!(t_lines("mem_tap", MEM_TAP, &[]), expected);
    assert_eq!(
        t_lines("mem_tap_unpacked", MEM_TAP, &[("XEZIM_PACKED_MEM", "0")]),
        expected
    );
}
