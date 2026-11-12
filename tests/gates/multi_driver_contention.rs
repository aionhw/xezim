//! Two or more drivers on one net or variable (IEEE 1800-2017 §6.5, §6.6,
//! §28.12). Nets resolve every driver bit by bit: `0` against `1` gives `x`,
//! `z` yields to a driven value, the wired net kinds AND / OR, `tri0`/`tri1`
//! pull and `trireg` holds its charge, and an edge on a contended bit is
//! counted from the resolved value. A variable takes one continuous driver,
//! or procedural writes, never both; `always_comb`/`always_latch`/`always_ff`
//! variables have a single writing process. Every expected line is the
//! reference simulator's output.

use std::path::PathBuf;
use std::process::Command;

fn xezim() -> String {
    // Cargo names the binary it built for the integration tests.
    env!("CARGO_BIN_EXE_xezim").to_string()
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_multi_driver_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Run `src` with top `top`: its `T|` lines, exit code and stderr.
fn run(tag: &str, src: &str, top: &str) -> (Vec<String>, i32, String) {
    run_with(tag, src, top, &[])
}

/// `run` with extra command-line flags.
fn run_with(tag: &str, src: &str, top: &str, flags: &[&str]) -> (Vec<String>, i32, String) {
    let d = scratch(tag);
    std::fs::write(d.join("tb.sv"), src).unwrap();
    let out = Command::new(xezim())
        .current_dir(&d)
        .args(flags)
        .args(["-s", top, "tb.sv"])
        .env_remove("XEZIM_RELAX_DRIVER_CONFLICTS")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&d);
    let lines = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("T|"))
        .map(str::to_string)
        .collect();
    (
        lines,
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The `T|` lines of a run that must succeed.
fn lines(tag: &str, src: &str, top: &str) -> Vec<String> {
    let (lines, code, stderr) = run(tag, src, top);
    assert_eq!(code, 0, "{tag}: exit code, stderr:\n{stderr}");
    lines
}

/// `wire`, `tri` and `wire logic` with two continuous drivers, for every pair
/// of 0, 1, x and z (§6.6.1 Table 6-2).
#[test]
fn two_assigns_resolve_every_value_pair() {
    let got = lines(
        "pairs",
        r#"
module k_pairs;
  logic a, b;
  wire w; tri t; wire logic wl;
  assign w = a; assign w = b;
  assign t = a; assign t = b;
  assign wl = a; assign wl = b;
  initial begin
    for (int i = 0; i < 4; i++)
      for (int j = 0; j < 4; j++) begin
        a = (i==0) ? 1'b0 : (i==1) ? 1'b1 : (i==2) ? 1'bx : 1'bz;
        b = (j==0) ? 1'b0 : (j==1) ? 1'b1 : (j==2) ? 1'bx : 1'bz;
        #1 $display("T|pair a=%b b=%b w=%b t=%b wl=%b w=%v", a, b, w, t, wl, w);
      end
  end
endmodule
"#,
        "k_pairs",
    );
    let want = [
        "T|pair a=0 b=0 w=0 t=0 wl=0 w=St0",
        "T|pair a=0 b=1 w=x t=x wl=x w=StX",
        "T|pair a=0 b=x w=x t=x wl=x w=StX",
        "T|pair a=0 b=z w=0 t=0 wl=0 w=St0",
        "T|pair a=1 b=0 w=x t=x wl=x w=StX",
        "T|pair a=1 b=1 w=1 t=1 wl=1 w=St1",
        "T|pair a=1 b=x w=x t=x wl=x w=StX",
        "T|pair a=1 b=z w=1 t=1 wl=1 w=St1",
        "T|pair a=x b=0 w=x t=x wl=x w=StX",
        "T|pair a=x b=1 w=x t=x wl=x w=StX",
        "T|pair a=x b=x w=x t=x wl=x w=StX",
        "T|pair a=x b=z w=x t=x wl=x w=StX",
        "T|pair a=z b=0 w=0 t=0 wl=0 w=St0",
        "T|pair a=z b=1 w=1 t=1 wl=1 w=St1",
        "T|pair a=z b=x w=x t=x wl=x w=StX",
        "T|pair a=z b=z w=z t=z wl=z w=HiZ",
    ];
    assert_eq!(got, want);
}

/// Vectors resolve per bit; a part or bit select drives only its bits, and
/// selects that overlap another driver resolve with it on the shared bits —
/// for descending, non-zero-based and ascending ranges and `+:`/`-:` selects.
/// Overlapping selects used to let the last assign win the shared bits.
#[test]
fn vector_and_overlapping_select_drivers_resolve_per_bit() {
    let got = lines(
        "vectors",
        r#"
module k_vectors;
  logic [7:0] a, b;
  logic [3:0] n; logic c, d;
  wire [7:0] w;  assign w = a; assign w = b;              // whole-vector pair
  wire [7:0] p;  assign p[3:0] = a[3:0]; assign p[7:2] = b[7:2];   // overlap on [3:2]
  wire [15:8] r; assign r[11:8] = n; assign r[15:12] = n; assign r[9] = c;
  wire [0:7] u;  assign u = {n, n}; assign u[1] = c; assign u[6 +: 2] = {d, d};
  wire [7:0] v;  assign v = {n, n}; assign v[5 -: 2] = {c, d};
  wire [7:0] s;  assign s[3:0] = n; assign s[7:4] = n;    // disjoint selects
  initial begin
    a = 8'b0101_zz01; b = 8'b0110_x1zz; #1 $display("T|vec w=%b p=%b", w, p);
    a = 8'b1x0z_1x0z; b = 8'b1111_0000; #1 $display("T|vec w=%b p=%b", w, p);
    a = 8'hzz;        b = 8'b10xz_10xz; #1 $display("T|vec w=%b p=%b", w, p);
    a = 8'h0f;        b = 8'hzz;        #1 $display("T|vec w=%b %h p=%b %h", w, w, p, p);
    a = 8'h0f;        b = 8'hf0;        #1 $display("T|vec w=%b %h %d p=%b", w, w, w, p);
    n = 4'b1010; c = 0; d = 1;          #1 $display("T|sel r=%b u=%b v=%b s=%b", r, u, v, s);
    c = 1'bz; d = 1'bz;                 #1 $display("T|sel r=%b u=%b v=%b s=%b", r, u, v, s);
    n = 4'bzzzz; c = 1; d = 0;          #1 $display("T|sel r=%b u=%b v=%b s=%b", r, u, v, s);
  end
endmodule
"#,
        "k_vectors",
    );
    let want = [
        "T|vec w=01xxx101 p=0110x101",
        "T|vec w=1xx1xx00 p=1111xx0z",
        "T|vec w=10xz10xz p=10xz10zz",
        "T|vec w=00001111 0f p=zzzz1111 zf",
        "T|vec w=xxxxxxxx xx   x p=1111xx11",
        "T|sel r=101010x0 u=1010101x v=10xx1010 s=10101010",
        "T|sel r=10101010 u=10101010 v=10101010 s=10101010",
        "T|sel r=zzzzzz1z u=z1zzzz00 v=zz10zzzz s=zzzzzzzz",
    ];
    assert_eq!(got, want);
}

/// A delayed driver changing over time against an undelayed one (`$monitor`
/// shows each resolved change), and a module output port plus a local assign
/// on one net.
#[test]
fn changing_driver_and_output_port_plus_local_assign() {
    let got = lines(
        "time_port",
        r#"
module drv(output wire [3:0] o, input [3:0] i); assign o = i; endmodule
module k_time_port;
  logic a, b;
  wire w;
  assign #2 w = a;
  assign w = b;
  logic [3:0] mi, li;
  wire [3:0] pw;
  drv u(.o(pw), .i(mi));
  assign pw = li;
  initial begin
    $monitor("T|mon t=%0t a=%b b=%b w=%b", $time, a, b, w);
    a = 0; b = 0;
    #5 a = 1;
    #5 b = 1;
    #5 a = 1'bz;
    #5 b = 1'bz;
    #5 a = 0;
    #5 b = 1'bx;
    #5 $monitoroff;
    mi = 4'b01zx; li = 4'bzzzz; #1 $display("T|port pw=%b", pw);
    li = 4'b0z1z; #1 $display("T|port pw=%b", pw);
    mi = 4'bzzzz; #1 $display("T|port pw=%b", pw);
    mi = 4'b1100; li = 4'b1010; #1 $display("T|port pw=%b", pw);
  end
endmodule
"#,
        "k_time_port",
    );
    let want = [
        "T|mon t=0 a=0 b=0 w=x",
        "T|mon t=2 a=0 b=0 w=0",
        "T|mon t=5 a=1 b=0 w=0",
        "T|mon t=7 a=1 b=0 w=x",
        "T|mon t=10 a=1 b=1 w=1",
        "T|mon t=15 a=z b=1 w=1",
        "T|mon t=20 a=z b=z w=z",
        "T|mon t=25 a=0 b=z w=z",
        "T|mon t=27 a=0 b=z w=0",
        "T|mon t=30 a=0 b=x w=x",
        "T|port pw=01zx",
        "T|port pw=011x",
        "T|port pw=0z1z",
        "T|port pw=1xx0",
    ];
    assert_eq!(got, want);
}

/// A `bufif1`/`bufif0` tristate bus with both, one or no enable on, data
/// with x and z (a bufif drives x for a z input, §28.4 Table 28-5), and the
/// `%v` strength of a scalar bus.
#[test]
fn bufif_bus_with_two_enables() {
    let got = lines(
        "bufif",
        r#"
module k_bufif;
  logic [3:0] d0, d1;
  logic e0, e1;
  wire [3:0] bus;
  bufif1 g0[3:0] (bus, d0, e0);
  bufif0 g1[3:0] (bus, d1, e1);
  wire s;
  bufif1 h0 (s, d0[0], e0);
  bufif1 h1 (s, d1[0], e1);
  initial begin
    d0 = 4'b1100; d1 = 4'b1010;
    e0 = 1; e1 = 0; #1 $display("T|both on bus=%b s=%b s=%v", bus, s, s);
    e0 = 1; e1 = 1; #1 $display("T|e0 only bus=%b s=%b s=%v", bus, s, s);
    e0 = 0; e1 = 0; #1 $display("T|e1 only bus=%b s=%b s=%v", bus, s, s);
    e0 = 0; e1 = 1; #1 $display("T|none bus=%b s=%b s=%v", bus, s, s);
    e0 = 1; e1 = 0; d0 = 4'b10xz; d1 = 4'b1z0x; #1 $display("T|xz data bus=%b", bus);
    e0 = 1; e1 = 1; d0 = 4'bz10x; #1 $display("T|xz alone bus=%b s=%b", bus, s);
  end
endmodule
"#,
        "k_bufif",
    );
    let want = [
        "T|both on bus=1xx0 s=0 s=St0",
        "T|e0 only bus=1100 s=0 s=St0",
        "T|e1 only bus=1010 s=z s=HiZ",
        "T|none bus=zzzz s=0 s=St0",
        "T|xz data bus=1xxx",
        "T|xz alone bus=x10x s=x",
    ];
    assert_eq!(got, want);
}

/// A bufif with an x enable drives L or H (its data or z, §28.4): beside
/// a strong driver of the same value the bit keeps that value, otherwise it
/// is x. The `?:` merge used to make every such bit x.
#[test]
fn bufif_unknown_enable_drives_l_or_h() {
    let got = lines(
        "bufif_xen",
        r#"
module k_bufif_xen;
  logic [3:0] d0, d1;
  logic e0, e1;
  wire [3:0] bus;
  bufif1 g0[3:0] (bus, d0, e0);
  bufif0 g1[3:0] (bus, d1, e1);
  wire s;
  bufif1 h0 (s, d0[0], e0);
  bufif1 h1 (s, d1[0], e1);
  initial begin
    d0 = 4'b1100; d1 = 4'b1010;
    e0 = 1'bx; e1 = 0; #1 $display("T|e0=x bus=%b s=%b", bus, s);
    e0 = 1; e1 = 1'bx; d0 = 4'b1111; d1 = 4'b1111; #1 $display("T|e1=x same bus=%b s=%b", bus, s);
    d1 = 4'b0000; #1 $display("T|e1=x diff bus=%b s=%b", bus, s);
    e0 = 1'bx; e1 = 1'bx; d0 = 4'b1100; d1 = 4'b1010; #1 $display("T|both x bus=%b s=%b", bus, s);
    e0 = 0; e1 = 1; #1 $display("T|off bus=%b s=%b", bus, s);
    e0 = 1'bx; #1 $display("T|x alone bus=%b s=%b", bus, s);
  end
endmodule
"#,
        "k_bufif_xen",
    );
    let want = [
        "T|e0=x bus=1xx0 s=x",
        "T|e1=x same bus=1111 s=1",
        "T|e1=x diff bus=xxxx s=x",
        "T|both x bus=xxxx s=x",
        "T|off bus=zzzz s=0",
        "T|x alone bus=xxxx s=0",
    ];
    assert_eq!(got, want);
}

/// Drive strengths decide the value (§28.12): mixed `(strong0, weak1)`
/// against `(weak0, strong1)`, `pull` against strong, strong against strong.
#[test]
fn drive_strength_values() {
    let got = lines(
        "strength",
        r#"
module k_strength_values;
  logic a, b;
  wire w1; assign (strong0, weak1) w1 = a; assign (weak0, strong1) w1 = b;
  wire w3; assign (pull0, pull1) w3 = a; assign w3 = b;
  wire w6; assign (strong0, strong1) w6 = a; assign (strong0, strong1) w6 = b;
  initial begin
    for (int i = 0; i < 4; i++)
      for (int j = 0; j < 4; j++) begin
        a = (i==0) ? 1'b0 : (i==1) ? 1'b1 : (i==2) ? 1'bx : 1'bz;
        b = (j==0) ? 1'b0 : (j==1) ? 1'b1 : (j==2) ? 1'bx : 1'bz;
        #1 $display("T|str a=%b b=%b w1=%b w3=%b w6=%b w6=%v", a, b, w1, w3, w6, w6);
      end
  end
endmodule
"#,
        "k_strength_values",
    );
    let want = [
        "T|str a=0 b=0 w1=0 w3=0 w6=0 w6=St0",
        "T|str a=0 b=1 w1=x w3=1 w6=x w6=StX",
        "T|str a=0 b=x w1=x w3=x w6=x w6=StX",
        "T|str a=0 b=z w1=0 w3=0 w6=0 w6=St0",
        "T|str a=1 b=0 w1=x w3=0 w6=x w6=StX",
        "T|str a=1 b=1 w1=1 w3=1 w6=1 w6=St1",
        "T|str a=1 b=x w1=x w3=x w6=x w6=StX",
        "T|str a=1 b=z w1=1 w3=1 w6=1 w6=St1",
        "T|str a=x b=0 w1=x w3=0 w6=x w6=StX",
        "T|str a=x b=1 w1=x w3=1 w6=x w6=StX",
        "T|str a=x b=x w1=x w3=x w6=x w6=StX",
        "T|str a=x b=z w1=x w3=x w6=x w6=StX",
        "T|str a=z b=0 w1=0 w3=0 w6=0 w6=St0",
        "T|str a=z b=1 w1=1 w3=1 w6=1 w6=St1",
        "T|str a=z b=x w1=x w3=x w6=x w6=StX",
        "T|str a=z b=z w1=z w3=z w6=z w6=HiZ",
    ];
    assert_eq!(got, want);
}

/// #1061: an UNSIZED `'1` fill used as a drive-strength value must replicate
/// across the whole net. `strength_bit` used to clamp `i >= width` to `0`
/// before consulting the fill, so an unsized `'1` (a 1-bit fill, §5.7.1)
/// set only bit 0 while the rest read the fill default zero (#1061). Sized
/// literals always worked.
#[test]
fn unsized_fill_drive_strength_replicates_wide() {
    let got = lines(
        "fill",
        r#"
module t;
  wire [7:0] n;
  logic      en = 1'b0;
  assign n = en ? 8'hc3 : 'z;
  assign (weak0, weak1) n = '1;    // unsized fill under weak strength
  initial begin
    #1 $display("T|weak1_alone n=%h", n);
    en = 1'b1;
    #1 $display("T|strong_c3 n=%h", n);
    $finish;
  end
endmodule
"#,
        "t",
    );
    assert_eq!(got, ["T|weak1_alone n=ff", "T|strong_c3 n=c3"]);
}

/// #1062: several assigns to the SAME bit of a net, some in a sub-module,
/// must all resolve together (0 against 1 -> x, z yields) — not only the
/// first of each bit count when another driver sits in a lazy sub-module.
#[test]
fn same_bit_submodule_select_drivers_resolve_per_bit() {
    let got = lines(
        "sameselect",
        r#"
module sub;
  wire [1:0] n;
  assign n[0] = 1'b1;
  assign n[0] = 1'bz;
  assign n[1] = 1'bz;
  assign n[1] = 1'b1;
endmodule
module t;
  wire [1:0] n;
  sub u ();
  assign n[0] = 1'b1;
  assign n[0] = 1'bz;
  assign n[1] = 1'bz;
  assign n[1] = 1'b1;
  initial begin
    #1 $display("T|top_sub n=%b u.n=%b", n, u.n);
    $finish;
  end
endmodule
"#,
        "t",
    );
    assert_eq!(got, ["T|top_sub n=11 u.n=11"]);
}

/// #1063: a `(weak0, weak1)` continuous assign in a NON-top module must pull
/// its net low/off the same way a top-module pull does: as a weak driver
/// that loses to the net's real (strong) driver but holds the net where it is
/// z. Strength was dropped on the inlined path and folded as STRONG.
#[test]
fn submodule_weak_pull_loses_to_real_driver() {
    let got = lines(
        "subweak",
        r#"
module sub;
  wire [7:0] n;
  logic      en = 1'b0;
  assign n = en ? 8'hc3 : 'z;
  assign (weak0, weak1) n = 8'hff;
endmodule
module t;
  wire [7:0] n;
  sub u ();
  initial begin
    #1 $display("T|weak_sub u.n=%h", u.n);
    u.en = 1'b1;
    #1 $display("T|strong_sub u.n=%h", u.n);
    $finish;
  end
endmodule
"#,
        "t",
    );
    assert_eq!(got, ["T|weak_sub u.n=ff", "T|strong_sub u.n=c3"]);
}

/// The VALUE side of every strength level (supply over strong over pull
/// over weak), compared with `%b` only; the `%v` strength names are pinned
/// by the ignored test below.
#[test]
fn drive_strength_levels_resolve_values() {
    let got = lines(
        "levels",
        r#"
module k_strength_levels;
  logic a, b;
  wire w1; assign (strong0, weak1) w1 = a; assign (weak0, strong1) w1 = b;
  wire w3; assign (pull0, pull1) w3 = a; assign w3 = b;
  wire w4; assign (supply0, supply1) w4 = a; assign w4 = b;
  wire w5; assign (weak0, weak1) w5 = a; assign (pull0, pull1) w5 = b;
  wire w6; assign (strong0, strong1) w6 = a; assign (strong0, strong1) w6 = b;
  supply0 s0; supply1 s1;
  wire w7; assign w7 = s1; assign (weak0, weak1) w7 = a;
  initial begin
    for (int i = 0; i < 4; i++)
      for (int j = 0; j < 4; j++) begin
        a = (i==0) ? 1'b0 : (i==1) ? 1'b1 : (i==2) ? 1'bx : 1'bz;
        b = (j==0) ? 1'b0 : (j==1) ? 1'b1 : (j==2) ? 1'bx : 1'bz;
        #1 $display("T|str a=%b b=%b w1=%v w3=%v w4=%v w5=%v w6=%v w7=%v | %b%b%b%b%b%b", a, b, w1, w3, w4, w5, w6, w7, w1, w3, w4, w5, w6, w7);
      end
    $display("T|supply s0=%v s1=%v", s0, s1);
  end
endmodule
"#,
        "k_strength_levels",
    );
    let want: Vec<String> = [
        "T|str a=0 b=0 w1=St0 w3=St0 w4=Su0 w5=Pu0 w6=St0 w7=St1 | 000001",
        "T|str a=0 b=1 w1=StX w3=St1 w4=Su0 w5=Pu1 w6=StX w7=St1 | x101x1",
        "T|str a=0 b=x w1=StX w3=StX w4=Su0 w5=PuX w6=StX w7=St1 | xx0xx1",
        "T|str a=0 b=z w1=St0 w3=Pu0 w4=Su0 w5=We0 w6=St0 w7=St1 | 000001",
        "T|str a=1 b=0 w1=WeX w3=St0 w4=Su1 w5=Pu0 w6=StX w7=St1 | x010x1",
        "T|str a=1 b=1 w1=St1 w3=St1 w4=Su1 w5=Pu1 w6=St1 w7=St1 | 111111",
        "T|str a=1 b=x w1=36X w3=StX w4=Su1 w5=PuX w6=StX w7=St1 | xx1xx1",
        "T|str a=1 b=z w1=We1 w3=Pu1 w4=Su1 w5=We1 w6=St1 w7=St1 | 111111",
        "T|str a=x b=0 w1=63X w3=St0 w4=SuX w5=Pu0 w6=StX w7=St1 | x0x0x1",
        "T|str a=x b=1 w1=StX w3=St1 w4=SuX w5=Pu1 w6=StX w7=St1 | x1x1x1",
        "T|str a=x b=x w1=StX w3=StX w4=SuX w5=PuX w6=StX w7=St1 | xxxxx1",
        "T|str a=x b=z w1=63X w3=PuX w4=SuX w5=WeX w6=StX w7=St1 | xxxxx1",
        "T|str a=z b=0 w1=We0 w3=St0 w4=St0 w5=Pu0 w6=St0 w7=St1 | 000001",
        "T|str a=z b=1 w1=St1 w3=St1 w4=St1 w5=Pu1 w6=St1 w7=St1 | 111111",
        "T|str a=z b=x w1=36X w3=StX w4=StX w5=PuX w6=StX w7=St1 | xxxxx1",
        "T|str a=z b=z w1=HiZ w3=HiZ w4=HiZ w5=HiZ w6=HiZ w7=St1 | zzzzz1",
        "T|supply s0=Su0 s1=Su1",
    ]
    .iter()
    .map(|l| values_only(l))
    .collect();
    let got: Vec<String> = got.iter().map(|l| values_only(l)).collect();
    assert_eq!(got, want);
}

/// Drops the `%v` fields (`w1=St0 ...`), keeping the `|`-separated `%b` part.
fn values_only(l: &str) -> String {
    match l.split_once(" | ") {
        Some((head, bits)) => {
            let ab: Vec<&str> = head.split(' ').take(3).collect();
            format!("{} | {}", ab.join(" "), bits)
        }
        None => l.split(' ').next().unwrap_or("").to_string(),
    }
}

/// `%v` reports the resolved strength: `Su`/`St`/`Pu`/`We`, an `L`/`H` from
/// a bufif with an x enable, a strength range for an ambiguous value
/// (`36X`), `Pu` for an undriven tri0/tri1 and `Me` for a trireg's charge,
/// `Su` for supply nets.
#[test]
fn drive_strength_display_follows_resolution() {
    let got = lines(
        "levels_v",
        r#"
module k_strength_levels;
  logic a, b;
  wire w1; assign (strong0, weak1) w1 = a; assign (weak0, strong1) w1 = b;
  wire w3; assign (pull0, pull1) w3 = a; assign w3 = b;
  wire w4; assign (supply0, supply1) w4 = a; assign w4 = b;
  wire w5; assign (weak0, weak1) w5 = a; assign (pull0, pull1) w5 = b;
  wire w6; assign (strong0, strong1) w6 = a; assign (strong0, strong1) w6 = b;
  supply0 s0; supply1 s1;
  wire w7; assign w7 = s1; assign (weak0, weak1) w7 = a;
  initial begin
    for (int i = 0; i < 4; i++)
      for (int j = 0; j < 4; j++) begin
        a = (i==0) ? 1'b0 : (i==1) ? 1'b1 : (i==2) ? 1'bx : 1'bz;
        b = (j==0) ? 1'b0 : (j==1) ? 1'b1 : (j==2) ? 1'bx : 1'bz;
        #1 $display("T|str a=%b b=%b w1=%v w3=%v w4=%v w5=%v w6=%v w7=%v | %b%b%b%b%b%b", a, b, w1, w3, w4, w5, w6, w7, w1, w3, w4, w5, w6, w7);
      end
    $display("T|supply s0=%v s1=%v", s0, s1);
  end
endmodule
"#,
        "k_strength_levels",
    );
    let want = [
        "T|str a=0 b=0 w1=St0 w3=St0 w4=Su0 w5=Pu0 w6=St0 w7=St1 | 000001",
        "T|str a=0 b=1 w1=StX w3=St1 w4=Su0 w5=Pu1 w6=StX w7=St1 | x101x1",
        "T|str a=0 b=x w1=StX w3=StX w4=Su0 w5=PuX w6=StX w7=St1 | xx0xx1",
        "T|str a=0 b=z w1=St0 w3=Pu0 w4=Su0 w5=We0 w6=St0 w7=St1 | 000001",
        "T|str a=1 b=0 w1=WeX w3=St0 w4=Su1 w5=Pu0 w6=StX w7=St1 | x010x1",
        "T|str a=1 b=1 w1=St1 w3=St1 w4=Su1 w5=Pu1 w6=St1 w7=St1 | 111111",
        "T|str a=1 b=x w1=36X w3=StX w4=Su1 w5=PuX w6=StX w7=St1 | xx1xx1",
        "T|str a=1 b=z w1=We1 w3=Pu1 w4=Su1 w5=We1 w6=St1 w7=St1 | 111111",
        "T|str a=x b=0 w1=63X w3=St0 w4=SuX w5=Pu0 w6=StX w7=St1 | x0x0x1",
        "T|str a=x b=1 w1=StX w3=St1 w4=SuX w5=Pu1 w6=StX w7=St1 | x1x1x1",
        "T|str a=x b=x w1=StX w3=StX w4=SuX w5=PuX w6=StX w7=St1 | xxxxx1",
        "T|str a=x b=z w1=63X w3=PuX w4=SuX w5=WeX w6=StX w7=St1 | xxxxx1",
        "T|str a=z b=0 w1=We0 w3=St0 w4=St0 w5=Pu0 w6=St0 w7=St1 | 000001",
        "T|str a=z b=1 w1=St1 w3=St1 w4=St1 w5=Pu1 w6=St1 w7=St1 | 111111",
        "T|str a=z b=x w1=36X w3=StX w4=StX w5=PuX w6=StX w7=St1 | xxxxx1",
        "T|str a=z b=z w1=HiZ w3=HiZ w4=HiZ w5=HiZ w6=HiZ w7=St1 | zzzzz1",
        "T|supply s0=Su0 s1=Su1",
    ];
    assert_eq!(got, want);
    let got = lines(
        "wired_v",
        r#"
module k_wired_strength;
  tri0 t0; tri1 t1; trireg tr; wand wa;
  supply0 s0; supply1 s1;
  initial #1 $display("T|str t0=%v t1=%v tr=%v wa=%v s0=%v s1=%v", t0, t1, tr, wa, s0, s1);
endmodule
"#,
        "k_wired_strength",
    );
    assert_eq!(got, ["T|str t0=Pu0 t1=Pu1 tr=HiZ wa=HiZ s0=Su0 s1=Su1",]);
}

/// `wand`/`wor`/`triand`/`trior`/`tri0`/`tri1`/`trireg` with two drivers, for
/// every pair of 0, 1, x and z (§6.6.2–§6.6.4).
#[test]
fn wired_net_kinds_resolve_every_value_pair() {
    let got = lines(
        "wired",
        r#"
module k_wired;
  logic a, b;
  wand wa; assign wa = a; assign wa = b;
  wor wo; assign wo = a; assign wo = b;
  triand ta; assign ta = a; assign ta = b;
  trior to; assign to = a; assign to = b;
  tri0 t0; assign t0 = a; assign t0 = b;
  tri1 t1; assign t1 = a; assign t1 = b;
  trireg tr; assign tr = a; assign tr = b;
  initial begin
    for (int i = 0; i < 4; i++)
      for (int j = 0; j < 4; j++) begin
        a = (i==0) ? 1'b0 : (i==1) ? 1'b1 : (i==2) ? 1'bx : 1'bz;
        b = (j==0) ? 1'b0 : (j==1) ? 1'b1 : (j==2) ? 1'bx : 1'bz;
        #1 $display("T|res a=%b b=%b wand=%b wor=%b triand=%b trior=%b tri0=%b tri1=%b trireg=%b", a, b, wa, wo, ta, to, t0, t1, tr);
      end

  end
endmodule
"#,
        "k_wired",
    );
    let want = [
        "T|res a=0 b=0 wand=0 wor=0 triand=0 trior=0 tri0=0 tri1=0 trireg=0",
        "T|res a=0 b=1 wand=0 wor=1 triand=0 trior=1 tri0=x tri1=x trireg=x",
        "T|res a=0 b=x wand=0 wor=x triand=0 trior=x tri0=x tri1=x trireg=x",
        "T|res a=0 b=z wand=0 wor=0 triand=0 trior=0 tri0=0 tri1=0 trireg=0",
        "T|res a=1 b=0 wand=0 wor=1 triand=0 trior=1 tri0=x tri1=x trireg=x",
        "T|res a=1 b=1 wand=1 wor=1 triand=1 trior=1 tri0=1 tri1=1 trireg=1",
        "T|res a=1 b=x wand=x wor=1 triand=x trior=1 tri0=x tri1=x trireg=x",
        "T|res a=1 b=z wand=1 wor=1 triand=1 trior=1 tri0=1 tri1=1 trireg=1",
        "T|res a=x b=0 wand=0 wor=x triand=0 trior=x tri0=x tri1=x trireg=x",
        "T|res a=x b=1 wand=x wor=1 triand=x trior=1 tri0=x tri1=x trireg=x",
        "T|res a=x b=x wand=x wor=x triand=x trior=x tri0=x tri1=x trireg=x",
        "T|res a=x b=z wand=x wor=x triand=x trior=x tri0=x tri1=x trireg=x",
        "T|res a=z b=0 wand=0 wor=0 triand=0 trior=0 tri0=0 tri1=0 trireg=0",
        "T|res a=z b=1 wand=1 wor=1 triand=1 trior=1 tri0=1 tri1=1 trireg=1",
        "T|res a=z b=x wand=x wor=x triand=x trior=x tri0=x tri1=x trireg=x",
        "T|res a=z b=z wand=z wor=z triand=z trior=z tri0=0 tri1=1 trireg=x",
    ];
    assert_eq!(got, want);
    let got = lines(
        "wired_vec",
        r#"
module k_wired_vec;
  logic [3:0] a, b;
  wand [3:0] wa; assign wa = a; assign wa = b;
  wor  [3:0] wo; assign wo = a; assign wo = b;
  tri0 [3:0] t0; assign t0 = a; assign t0 = b;
  tri1 [3:0] t1; assign t1 = a; assign t1 = b;
  trireg [3:0] tr; assign tr = a; assign tr = b;
  wand [3:0] w3; assign w3 = a; assign w3 = b; assign w3 = 4'b1z1x;
  initial begin
    a = 4'b01xz; b = 4'b1111; #1 $display("T|v wand=%b wor=%b tri0=%b tri1=%b trireg=%b w3=%b", wa, wo, t0, t1, tr, w3);
    a = 4'b01xz; b = 4'b0000; #1 $display("T|v wand=%b wor=%b tri0=%b tri1=%b trireg=%b w3=%b", wa, wo, t0, t1, tr, w3);
    a = 4'b01xz; b = 4'bzzzz; #1 $display("T|v wand=%b wor=%b tri0=%b tri1=%b trireg=%b w3=%b", wa, wo, t0, t1, tr, w3);
    a = 4'bzzzz; b = 4'bzzzz; #1 $display("T|v wand=%b wor=%b tri0=%b tri1=%b trireg=%b w3=%b", wa, wo, t0, t1, tr, w3);
  end
endmodule
"#,
        "k_wired_vec",
    );
    let want = [
        "T|v wand=01x1 wor=1111 tri0=x1x1 tri1=x1x1 trireg=x1x1 w3=01xx",
        "T|v wand=0000 wor=01x0 tri0=0xx0 tri1=0xx0 trireg=0xx0 w3=0000",
        "T|v wand=01xz wor=01xz tri0=01x0 tri1=01x1 trireg=01x0 w3=01xx",
        "T|v wand=zzzz wor=zzzz tri0=0000 tri1=1111 trireg=01x0 w3=1z1x",
    ];
    assert_eq!(got, want);
}

/// posedge/negedge/any-edge counts on a contended bit as the drivers move
/// through 0, 1, x and z (0→x and x→1 are edges, §9.4.2). `wv[3]` has a
/// whole-vector driver and a bit driver: it used to take the bit driver's
/// value alone, which miscounted its edges (10 posedges where the reference
/// counts 12 in the replicated-clock design).
#[test]
fn edges_on_a_contended_bit() {
    let got = lines(
        "edges",
        r#"
module k_edges;
  logic a, b;
  wire w; assign w = a; assign w = b;
  logic [3:0] av; logic bv;
  wire [3:0] wv; assign wv = av; assign wv[3] = bv;
  int pe, ne, ae, pv, nv;
  always @(posedge w) pe++;
  always @(negedge w) ne++;
  always @(w) ae++;
  always @(posedge wv[3]) pv++;
  always @(negedge wv[3]) nv++;
  initial begin
    static logic [1:0] seq_a [16] = '{0,0,1,1,2,3,3,0,1,3,2,2,0,1,3,0};
    static logic [1:0] seq_b [16] = '{3,1,1,0,3,3,1,1,1,2,2,0,0,3,1,3};
    a = 0; b = 1'bz; av = 0; bv = 1'bz;
    #1;
    for (int k = 0; k < 16; k++) begin
      a = seq_a[k] == 0 ? 1'b0 : seq_a[k] == 1 ? 1'b1 : seq_a[k] == 2 ? 1'bx : 1'bz;
      b = seq_b[k] == 0 ? 1'b0 : seq_b[k] == 1 ? 1'b1 : seq_b[k] == 2 ? 1'bx : 1'bz;
      av = {4{a}}; bv = b;
      #1 $display("T|step k=%0d a=%b b=%b w=%b wv3=%b pe=%0d ne=%0d ae=%0d pv=%0d nv=%0d", k, a, b, w, wv[3], pe, ne, ae, pv, nv);
    end
    $display("T|final pe=%0d ne=%0d ae=%0d pv=%0d nv=%0d", pe, ne, ae, pv, nv);
  end
endmodule
"#,
        "k_edges",
    );
    let want = [
        "T|step k=0 a=0 b=z w=0 wv3=0 pe=0 ne=1 ae=1 pv=0 nv=1",
        "T|step k=1 a=0 b=1 w=x wv3=x pe=1 ne=1 ae=2 pv=1 nv=1",
        "T|step k=2 a=1 b=1 w=1 wv3=1 pe=2 ne=1 ae=3 pv=2 nv=1",
        "T|step k=3 a=1 b=0 w=x wv3=x pe=2 ne=2 ae=4 pv=2 nv=2",
        "T|step k=4 a=x b=z w=x wv3=x pe=2 ne=2 ae=4 pv=2 nv=2",
        "T|step k=5 a=z b=z w=z wv3=z pe=2 ne=2 ae=5 pv=2 nv=2",
        "T|step k=6 a=z b=1 w=1 wv3=1 pe=3 ne=2 ae=6 pv=3 nv=2",
        "T|step k=7 a=0 b=1 w=x wv3=x pe=3 ne=3 ae=7 pv=3 nv=3",
        "T|step k=8 a=1 b=1 w=1 wv3=1 pe=4 ne=3 ae=8 pv=4 nv=3",
        "T|step k=9 a=z b=x w=x wv3=x pe=4 ne=4 ae=9 pv=4 nv=4",
        "T|step k=10 a=x b=x w=x wv3=x pe=4 ne=4 ae=9 pv=4 nv=4",
        "T|step k=11 a=x b=0 w=x wv3=x pe=4 ne=4 ae=9 pv=4 nv=4",
        "T|step k=12 a=0 b=0 w=0 wv3=0 pe=4 ne=5 ae=10 pv=4 nv=5",
        "T|step k=13 a=1 b=z w=1 wv3=1 pe=5 ne=5 ae=11 pv=5 nv=5",
        "T|step k=14 a=z b=1 w=1 wv3=1 pe=5 ne=5 ae=11 pv=5 nv=5",
        "T|step k=15 a=0 b=z w=0 wv3=0 pe=5 ne=6 ae=12 pv=5 nv=6",
        "T|final pe=5 ne=6 ae=12 pv=5 nv=6",
    ];
    assert_eq!(got, want);
}

/// Variable writers the LRM forbids, each a compile error (exit 1, no
/// simulation), as the reference simulator reports them: two continuous
/// assigns (§6.5), a continuous assign plus a procedural write (§6.5), a
/// variable of an `always_comb`/`always_latch` written by another process
/// (§9.2.2.2), of an `always_ff` written by a non-`always_ff` process
/// (§9.2.2.4) — each per written bit. `--relax-driver-conflicts` turns each
/// into a warning; none of these shapes is a hard error.
#[test]
fn illegal_variable_drivers_are_compile_errors() {
    let cases: [(&str, &str, Option<&str>); 13] = [
        (
            "v1",
            r#"
module v1;
  logic a, b, v;
  assign v = a;
  assign v = b;
  initial begin a = 0; b = 1; #1 $display("T|v=%b", v); end
endmodule
"#,
            None,
        ),
        (
            "v2",
            r#"
module v2;
  logic a, v;
  assign v = a;
  initial begin a = 0; #1 v = 1; #1 $display("T|v=%b", v); end
endmodule
"#,
            Some("T|v=1"),
        ),
        (
            "v3",
            r#"
module v3;
  logic clk = 0, a = 0, b = 1, v;
  always_ff @(posedge clk) v <= a;
  always_comb v = b;
  initial begin #1 clk = 1; #1 $display("T|v=%b", v); end
endmodule
"#,
            Some("T|v=0"),
        ),
        (
            "v6",
            r#"
module v6;
  logic a = 0, v;
  always_comb v = a;
  initial begin #1 v = 1; #1 $display("T|v=%b", v); end
endmodule
"#,
            Some("T|v=1"),
        ),
        (
            "v8",
            r#"
module v8;
  reg a, b;
  reg [3:0] v;
  assign v[2:0] = {a, b, a};
  assign v[3:2] = {b, a};
  initial begin a = 0; b = 1; #1 $display("T|overlap v=%b", v); end
endmodule
"#,
            Some("T|overlap v=1010"),
        ),
        (
            "w1",
            r#"
module w1;
  logic a = 0, v;
  always_latch if (a == 0) v = 1;
  initial begin #1 v = 0; #1 $display("T|v=%b", v); end
endmodule
"#,
            Some("T|v=0"),
        ),
        (
            "w2",
            r#"
module w2;
  logic clk = 0, v;
  always_ff @(posedge clk) v <= 1;
  initial begin v = 0; #1 clk = 1; #1 $display("T|v=%b", v); end
endmodule
"#,
            Some("T|v=1"),
        ),
        (
            "w6",
            r#"
module w6;
  logic a = 1;
  logic [1:0] v;
  assign v[0] = a;
  initial begin #1 v[0] = 0; #1 $display("T|v=%b", v); end
endmodule
"#,
            Some("T|v=x0"),
        ),
        (
            "w7",
            r#"
module w7;
  logic a = 1, b = 0, v;
  always_comb v = a;
  always_comb v = b;
  initial #1 $display("T|v=%b", v);
endmodule
"#,
            None,
        ),
        (
            "w8",
            r#"
module w8;
  logic a = 1, v;
  always_comb v = a;
  assign v = a;
  initial #1 $display("T|v=%b", v);
endmodule
"#,
            Some("T|v=1"),
        ),
        (
            "w10",
            r#"
module w10;
  logic a = 1;
  logic [3:0] v;
  always_comb v = {4{a}};
  always @(a) v[2] = 0;
  initial #1 $display("T|v=%b", v);
endmodule
"#,
            Some("T|v=1111"),
        ),
        (
            "w11",
            r#"
module w11;
  logic clk = 0;
  logic [3:0] v;
  always_ff @(posedge clk) v <= 4'h1;
  always @(negedge clk) v <= 4'h2;
  initial begin #1 clk = 1; #1 $display("T|v=%h", v); end
endmodule
"#,
            Some("T|v=1"),
        ),
        (
            "w13",
            r#"
module w13;
  logic [7:0] v;
  logic a = 1;
  assign v[3:0] = {4{a}};
  assign v[7:4] = 4'h5;
  assign v[4] = 1'b0;
  initial #1 $display("T|v=%b", v);
endmodule
"#,
            Some("T|v=01011111"),
        ),
    ];
    for (top, src, relaxed) in cases {
        let (lines, code, stderr) = run(top, src, top);
        assert_eq!(code, 1, "{top}: exit code, stderr:\n{stderr}");
        assert!(lines.is_empty(), "{top}: simulated anyway: {lines:?}");
        assert!(
            stderr.contains("Variable 'v") && stderr.contains("--relax-driver-conflicts"),
            "{top}: no driver diagnostic in:\n{stderr}"
        );
        // Every shape is one the reference reports as a SUPPRESSIBLE error,
        // so `--relax-driver-conflicts` downgrades it to a warning and the
        // design runs, printing what the reference prints with the error
        // suppressed. `None`: the two writers race (two continuous assigns,
        // two always_comb blocks on one variable) and the surviving value is
        // implementation-defined, so only the run itself is checked.
        let (lines, code, stderr) = run_with(top, src, top, &["--relax-driver-conflicts"]);
        assert_eq!(code, 0, "{top}: relaxed exit code, stderr:\n{stderr}");
        assert!(
            stderr.contains("[xezim][warning] Variable 'v"),
            "{top}: no relaxed warning in:\n{stderr}"
        );
        match relaxed {
            Some(want) => assert_eq!(lines, [want], "{top}: relaxed output"),
            None => assert_eq!(lines.len(), 1, "{top}: relaxed output {lines:?}"),
        }
    }
}

/// Writers the reference accepts: two `always_ff` blocks, two `always`
/// blocks (the last write wins), continuous assigns of disjoint bits,
/// `always_ff`/`always_comb` writers of disjoint bits, a continuous bit
/// beside a procedural bit, a write through a function called from another
/// process, and `force` on a continuously assigned variable.
#[test]
fn legal_variable_writers_simulate() {
    assert_eq!(
        lines(
            "v4",
            r#"
module v4;
  logic clk = 0, a = 0, b = 1, v;
  always_ff @(posedge clk) v <= a;
  always_ff @(posedge clk) v <= b;
  initial begin #1 clk = 1; #1 $display("T|v=%b", v); end
endmodule
"#,
            "v4"
        ),
        ["T|v=1"]
    );
    assert_eq!(
        lines(
            "v5",
            r#"
module v5;
  logic clk = 0, a = 0, b = 1;
  logic v, u;
  reg [3:0] r;
  always @(posedge clk) v = a;
  always @(posedge clk) v = b;
  always @(posedge clk) begin u = 1'b0; r = 4'h3; end
  always @(negedge clk) begin u = 1'b1; r = 4'hc; end
  always @(a or b) r[0] = a ^ b;
  initial begin
    #1 clk = 1; #1 $display("T|legal v=%b u=%b r=%h", v, u, r);
    #1 clk = 0; #1 $display("T|legal v=%b u=%b r=%h", v, u, r);
    a = 1; #1 $display("T|legal v=%b u=%b r=%h", v, u, r);
  end
endmodule
"#,
            "v5"
        ),
        [
            "T|legal v=1 u=0 r=3",
            "T|legal v=1 u=1 r=c",
            "T|legal v=1 u=1 r=c"
        ]
    );
    assert_eq!(
        lines(
            "v7",
            r#"
module v7;
  reg a, b;
  reg [3:0] v;
  assign v[1:0] = {a, b};
  assign v[3:2] = {b, a};
  initial begin a = 0; b = 1; #1 $display("T|disjoint v=%b", v); end
endmodule
"#,
            "v7"
        ),
        ["T|disjoint v=1001"]
    );
    assert_eq!(
        lines(
            "v9",
            r#"
module v9;
  logic clk = 0;
  logic [3:0] v;
  always_ff @(posedge clk) v[1:0] <= 2'b01;
  always_ff @(posedge clk) v[3:2] <= 2'b10;
  initial begin #1 clk = 1; #1 $display("T|ff disjoint v=%b", v); end
endmodule
"#,
            "v9"
        ),
        ["T|ff disjoint v=1001"]
    );
    assert_eq!(
        lines(
            "v10",
            r#"
module v10;
  logic a = 1, v;
  assign v = a;
  initial begin force v = 0; #1 $display("T|force v=%b", v); release v; #1 $display("T|rel v=%b", v); end
endmodule
"#,
            "v10"
        ),
        ["T|force v=0", "T|rel v=1"]
    );
    assert_eq!(
        lines(
            "w3",
            r#"
module w3;
  logic a = 1;
  logic [1:0] v;
  always_comb v[0] = a;
  always_comb v[1] = ~a;
  initial #1 $display("T|v=%b", v);
endmodule
"#,
            "w3"
        ),
        ["T|v=01"]
    );
    assert_eq!(
        lines(
            "w4",
            r#"
module w4;
  logic a = 1;
  logic [1:0] v;
  always_comb v[0] = a;
  initial begin #1 v[1] = 0; #1 $display("T|v=%b", v); end
endmodule
"#,
            "w4"
        ),
        ["T|v=01"]
    );
    assert_eq!(
        lines(
            "w5",
            r#"
module w5;
  logic a = 1;
  logic [1:0] v;
  assign v[0] = a;
  initial begin #1 v[1] = 0; #1 $display("T|v=%b", v); end
endmodule
"#,
            "w5"
        ),
        ["T|v=01"]
    );
    assert_eq!(
        lines(
            "w9",
            r#"
module w9;
  logic a = 1, v;
  function automatic void setv(); v = 0; endfunction
  always_comb begin v = a; end
  initial begin #1 setv(); #1 $display("T|v=%b", v); end
endmodule
"#,
            "w9"
        ),
        ["T|v=0"]
    );
}

/// `%v` of tristate primitives with an unknown control (`StH`), of a pull
/// beside a tristate (`651`, the span from St1 down to Pu1), of a trireg's
/// held charge (`Me`), supply nets and undriven tri0/tri1/trireg nets.
#[test]
fn drive_strength_display_of_net_kinds() {
    let got = lines(
        "kinds_v",
        r#"
module k_strength_kinds;
  tri0 t0; tri1 t1; trireg tr; wand wa;
  supply0 s0; supply1 s1;
  logic a, c;
  trireg tq; assign tq = a;
  wire v1; bufif1 (v1, a, c);
  wire v2; assign (pull0, pull1) v2 = a; bufif1 (v2, 1'b1, c);
  wire [1:0] vv; assign (pull0, strong1) vv = {a, c};
  initial begin
    #1 $display("T|w t0=%v t1=%v tr=%v wa=%v s0=%v s1=%v | %b %b %b %b", t0, t1, tr, wa, s0, s1, t0, t1, tr, wa);
    a = 1; c = 1'bx;
    #1 $display("T|x tq=%v v1=%v v2=%v | %b %b %b", tq, v1, v2, tq, v1, v2);
    a = 1'bz; c = 0;
    #1 $display("T|y tq=%v v1=%v v2=%v | %b %b %b", tq, v1, v2, tq, v1, v2);
    a = 0; c = 1;
    #1 $display("T|z tq=%v v1=%v v2=%v | %b %b %b", tq, v1, v2, tq, v1, v2);
  end
endmodule
"#,
        "k_strength_kinds",
    );
    assert_eq!(
        got,
        [
            "T|w t0=Pu0 t1=Pu1 tr=HiZ wa=HiZ s0=Su0 s1=Su1 | 0 1 z z",
            "T|x tq=St1 v1=StH v2=651 | 1 x 1",
            "T|y tq=Me1 v1=HiZ v2=HiZ | 1 z z",
            "T|z tq=St0 v1=St0 v2=St1 | 0 0 1",
        ]
    );
}

/// A variable driven by a module output port and by a local continuous
/// assign is two continuous drivers (§6.5); the reference rejects it.
#[test]
fn output_port_plus_local_assign_on_a_variable_is_an_error() {
    let (lines, code, _) = run(
        "w12",
        r#"
module sub(output logic o); assign o = 1; endmodule
module w12;
  logic v;
  sub u(.o(v));
  assign v = 0;
  initial #1 $display("T|v=%b", v);
endmodule
"#,
        "w12",
    );
    assert_eq!(code, 1);
    assert!(lines.is_empty());
}

/// §6.5 / §23.3.3: an output port drives its actual like a continuous
/// assignment, whatever the connection form, per written bit. A port beside
/// another port or a continuous assign is an error; beside a procedural
/// write the reference simulator only warns, so that runs.
#[test]
fn output_port_connections_are_continuous_drivers() {
    let sub = "module sub(output logic o); assign o = 1; endmodule\n";
    let errors = [
        ("p2", "logic v; sub u(.o(v)); sub u2(.o(v));"),
        ("pbit", "logic [1:0] v; sub u(.o(v[0])); assign v[0] = 0;"),
        ("ppos", "logic v; sub u(v); assign v = 0;"),
        ("pcat", "logic a, b; sub u(.o({a, b})); assign b = 0;"),
        ("pimp", "logic o; sub u(.o); assign o = 0;"),
    ];
    for (tag, body) in errors {
        let src = format!("{sub}module {tag}; {body} endmodule\n");
        let (lines, code, stderr) = run(tag, &src, tag);
        assert_eq!(code, 1, "{tag}: {stderr}");
        assert!(lines.is_empty(), "{tag}");
        assert!(
            stderr.contains("multiple continuous drivers"),
            "{tag}: {stderr}"
        );
        let (_, code, stderr) = run_with(tag, &src, tag, &["--relax-driver-conflicts"]);
        assert_eq!(code, 0, "{tag} relaxed: {stderr}");
        assert!(
            stderr.contains("--relax-driver-conflicts"),
            "{tag}: {stderr}"
        );
    }
    let accepted = [
        (
            "pinit",
            "logic v; sub u(.o(v)); initial v = 0; initial #1 $display(\"T|v=%b\", v);",
            "T|v=0",
        ),
        (
            "pcomb",
            "logic v; sub u(.o(v)); always_comb v = 0; initial #1 $display(\"T|v=%b\", v);",
            "T|v=0",
        ),
        (
            "pdisj",
            "logic [1:0] v; sub u(.o(v[0])); assign v[1] = 0; initial #1 $display(\"T|v=%b\", v);",
            "T|v=01",
        ),
    ];
    for (tag, body, want) in accepted {
        let src = format!("{sub}module {tag}; {body} endmodule\n");
        assert_eq!(lines(tag, &src, tag), [want], "{tag}");
    }
}

/// `XEZIM_RELAX_DRIVER_CONFLICTS=1` is the same switch as the flag: the
/// common `initial v = 0;` beside `always_ff ... v <= ...;` runs with a warning.
#[test]
fn relax_driver_conflicts_env_var() {
    let src = r#"
module w2;
  logic clk = 0, v;
  always_ff @(posedge clk) v <= 1;
  initial begin v = 0; #1 clk = 1; #1 $display("T|v=%b", v); end
endmodule
"#;
    let d = scratch("relax_env");
    std::fs::write(d.join("tb.sv"), src).unwrap();
    let out = Command::new(xezim())
        .current_dir(&d)
        .args(["-s", "w2", "tb.sv"])
        .env("XEZIM_RELAX_DRIVER_CONFLICTS", "1")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&d);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| l.starts_with("T|")).collect();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(lines, ["T|v=1"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("[xezim][warning] Variable 'v'"));
}
