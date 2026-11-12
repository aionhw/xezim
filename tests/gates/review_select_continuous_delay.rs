//! IEEE 1800-2023 §10.3.3 / §28.16: the rise, fall and turn-off delays of a
//! continuous assignment to a constant part-select or bit-select that no
//! other driver overlaps, of a primitive whose output is a bit-select, and of
//! a continuous assignment to a whole vector variable (§10.3.2). Such a
//! select target shares its net with other drivers, so the fall and turn-off
//! delays could not go to the net's per-driver delay maps and were dropped:
//! every change took the rise delay. A variable target was treated as a
//! scalar, so a change to a partly-z value took the smallest delay.
//!
//! A part-select and a vector variable follow the vector rules (fall for a
//! change from nonzero to 0, turn-off for a change to z, rise otherwise); a
//! bit-select and a primitive output follow the gate rules (the smallest
//! delay for a change to x). With two delays the turn-off delay is the
//! smaller of the two (§28.16). Expected values follow the LRM. The
//! reference simulator agrees on every bit-select and primitive change, and
//! on every vector change to 0, to z or to a partly-z value; for a vector
//! change to all x it takes the smallest delay, and for a change between
//! known values it chooses by the least significant bit (0000 -> 0110 takes
//! the fall delay there), which §10.3.3 does not.

const SRC: &str = r#"
`timescale 1ns/1ps
module child(input logic [3:0] x, input logic a, output wire [7:0] w);
  assign #(4, 2) w[3:0] = x;
  assign #(4, 2) w[5] = a;
endmodule
module top;
  logic [3:0] x;
  logic a;
  wire [7:0] g;
  and #(4, 2) ga(g[2], a, 1'b1);
  wire [7:0] cw;
  child c(x, a, cw);
  logic [3:0] pv;
  assign #(4, 2) pv[3:2] = x[1:0];
  logic [3:0] vv;
  assign #(4, 2) vv = x;
  wire [7:0] ps;
  assign #(4, 6, 3) ps[3:0] = x;
  assign ps[7:4] = 4'h5;
  wire [7:0] zr;
  assign #(0, 3) zr[6:4] = x[2:0];
  always @(g[2]) $display("%0t g2=%b", $time, g[2]);
  always @(cw[3:0]) $display("%0t cw30=%b", $time, cw[3:0]);
  always @(cw[5]) $display("%0t cw5=%b", $time, cw[5]);
  always @(pv[3:2]) $display("%0t pv32=%b", $time, pv[3:2]);
  always @(vv) $display("%0t vv=%b", $time, vv);
  always @(ps[3:0]) $display("%0t ps30=%b", $time, ps[3:0]);
  always @(zr[6:4]) $display("%0t zr64=%b", $time, zr[6:4]);
  initial begin
    x = 4'h3; a = 1;
    #10 x = 4'bxxxx; a = 1'bx;
    #10 x = 4'h0; a = 0;
    #10 x = 4'b00z1; a = 1'bz;
    #10 x = 4'bzzzz; a = 1;
    #10 x = 4'h0; a = 0;
    #10 x = 4'b0110; a = 1;
    #10 $finish;
  end
endmodule
"#;

/// The changes after time 0, ordered by time and then by text (the order of
/// same-time updates is not specified, §4.7).
fn changes(src: &str) -> Vec<String> {
    let sim = xezim::simulate(src, 1_000_000).expect("simulate failed");
    let mut out: Vec<(u64, String)> = sim
        .output
        .iter()
        .filter_map(|o| {
            let line = o.message.trim();
            let (t, rest) = line.split_once(' ')?;
            let t: u64 = t.parse().ok()?;
            (t > 0).then(|| (t, rest.to_string()))
        })
        .collect();
    out.sort();
    out.into_iter().map(|(t, s)| format!("{t} {s}")).collect()
}

#[test]
fn select_and_variable_targets_choose_their_transition_delay() {
    assert_eq!(
        changes(SRC),
        [
            "4000 cw30=0011",
            "4000 cw5=1",
            "4000 g2=1",
            "4000 ps30=0011",
            "4000 pv32=11",
            "4000 vv=0011",
            "10000 zr64=xxx",
            // Scalar targets to x: the smallest delay.
            "12000 cw5=x",
            "12000 g2=x",
            // Vector targets to x: the rise delay.
            "14000 cw30=xxxx",
            "14000 ps30=xxxx",
            "14000 pv32=xx",
            "14000 vv=xxxx",
            // To 0: the fall delay.
            "22000 cw30=0000",
            "22000 cw5=0",
            "22000 g2=0",
            "22000 pv32=00",
            "22000 vv=0000",
            "23000 zr64=000",
            "26000 ps30=0000",
            "30000 zr64=0z1",
            // A scalar to z (turn-off) and a gate output to x.
            "32000 cw5=z",
            "32000 g2=x",
            // A vector to a partly-z value: the rise delay.
            "34000 cw30=00z1",
            "34000 ps30=00z1",
            "34000 pv32=z1",
            "34000 vv=00z1",
            "40000 zr64=zzz",
            // To z: the turn-off delay, with two delays the smaller one.
            "42000 cw30=zzzz",
            "42000 pv32=zz",
            "42000 vv=zzzz",
            "43000 ps30=zzzz",
            "44000 cw5=1",
            "44000 g2=1",
            "52000 cw30=0000",
            "52000 cw5=0",
            "52000 g2=0",
            "52000 pv32=00",
            "52000 vv=0000",
            "53000 zr64=000",
            "56000 ps30=0000",
            // Between known values: the rise delay.
            "60000 zr64=110",
            "64000 cw30=0110",
            "64000 cw5=1",
            "64000 g2=1",
            "64000 ps30=0110",
            "64000 pv32=10",
            "64000 vv=0110",
        ]
    );
}

/// The same design from a warm `--cache-dir`, whose prepared tables carry
/// the select targets' own delays.
#[test]
fn select_delays_survive_the_prepared_table_cache() {
    let dir = std::env::temp_dir().join(format!("review_select_delay_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("top.sv"), SRC).unwrap();
    let run = || {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
            .current_dir(&dir)
            .args(["--cache-dir", "cache", "top.sv"])
            .output()
            .unwrap();
        assert!(out.status.success());
        let mut lines: Vec<(u64, String)> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| {
                let (t, rest) = l.trim().split_once(' ')?;
                Some((t.parse().ok()?, rest.to_string()))
            })
            .collect();
        lines.sort();
        (lines, String::from_utf8_lossy(&out.stderr).into_owned())
    };
    let (cold, cold_log) = run();
    assert!(cold_log.contains("prepared-comb stored"), "{cold_log}");
    let (warm, warm_log) = run();
    assert!(warm_log.contains("prepared-comb hit"), "{warm_log}");
    assert_eq!(cold, warm);
    assert!(warm.contains(&(26000, "ps30=0000".to_string())), "{warm:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// §10.3.3 / §28.16: disjoint select drivers of one net keep their own
/// delays when they change in the same time step, and a pulse shorter than
/// a driver's delay is swallowed for that driver only. The pending updates
/// of the net used to be one: the later-scheduled driver moved the other's
/// change to its own time. Expected values are the reference simulator's.
#[test]
fn disjoint_select_drivers_keep_their_own_delays() {
    let src = r#"
`timescale 1ns/1ps
module top;
  logic [3:0] a, b;
  logic x0, x1;
  wire [7:0] d;
  assign #2 d[3:0] = a;
  assign #5 d[7:4] = b;
  wire [1:0] bus;
  and #(4, 2) g0(bus[0], x0, 1'b1);
  and #(4, 2) g1(bus[1], x1, 1'b1);
  always @(d[3:0]) $display("%0t d30=%h", $time, d[3:0]);
  always @(d[7:4]) $display("%0t d74=%h", $time, d[7:4]);
  always @(bus[0]) $display("%0t bus0=%b", $time, bus[0]);
  always @(bus[1]) $display("%0t bus1=%b", $time, bus[1]);
  initial begin
    a = 1; b = 2; x0 = 1; x1 = 0;
    #10 a = 3; b = 4; x0 = 0; x1 = 1;
    #10 a = 5;
    #1 a = 3;
    #1 b = 6;
    #1 b = 4;
    #10 x0 = 1; x1 = 0;
    #1 x0 = 0;
    #10 $finish;
  end
endmodule
"#;
    assert_eq!(
        changes(src),
        [
            "2000 bus1=0",
            "2000 d30=1",
            "4000 bus0=1",
            "5000 d74=2",
            "12000 bus0=0",
            "12000 d30=3",
            "14000 bus1=1",
            "15000 d74=4",
            "35000 bus1=0",
        ]
    );
}
