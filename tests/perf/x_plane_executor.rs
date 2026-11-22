//! The x-plane two-state executor: a block that reads an x/z bit no longer
//! re-runs on the four-state VM but on the same lowered stream with (value,
//! x) register planes and the VM's four-state rules. On a C906 SoC three
//! quarters of what the VM still executed were such re-runs (tiny muxes
//! whose unselected arm, or an unwritten register file, holds x).
//!
//! Every shape below carries an x or z through a different rule — Kleene
//! and/or/xor, `&&`/`||`/`!`, `==`/`!=` with a known mismatch vs. an
//! ambiguous one, reductions, all-x arithmetic, plane shifts, an x
//! selector merging its arms, an x or out-of-range index reading all-x and
//! writing nothing, NBAs of x into a memory and a wide vector. The expected
//! lines are the four-state VM's (`XEZIM_TS_X=0` gives the same output) and
//! were cross-checked against the reference simulator, and the run must
//! actually take the x-plane path (`x_plane_runs`).
use std::process::Command;

fn run(name: &str, src: &str) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_x_plane_executor_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join(format!("{name}.sv"));
    std::fs::write(&path, src).expect("write temporary design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "tb",
            "--no-cache",
            path.to_str().unwrap(),
        ])
        .env("XEZIM_PROFILE_REPORT", "1")
        .output()
        .expect("run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "run failed:\n{text}");
    text
}

fn stat(text: &str, key: &str) -> u64 {
    let line = text
        .lines()
        .find(|l| l.contains(key))
        .unwrap_or_else(|| panic!("missing counter `{key}`:\n{text}"));
    let after = &line[line.find(key).unwrap() + key.len()..];
    after
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("unparsable counter `{key}` in `{line}`"))
}

const DESIGN: &str = r#"
module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [7:0] a, b, c, cnt;
  logic [7:0] m1, m2, m3, l1, l2, l3, e1, e2, e3, ar1, ar2, sh1, sh2, r1, r2, cs, bd, rd, nq, nq2, q1, q2, q3;
  logic [3:0] idx; logic s;
  logic [7:0] mem [0:3];
  logic [127:0] wide; logic [7:0] wr;
  int cyc = 0;
  assign m1 = s ? a : b;
  assign m2 = a[0] ? a : b;
  assign m3 = (a & b) | (c ^ b);
  assign l1 = {7'd0, (a != 0) && (b != 0)};
  assign l2 = {7'd0, !(c == 0) || (b[2:0] == 3'd5)};
  assign l3 = {7'd0, ~|a} | {7'd0, &b};
  assign e1 = {7'd0, a == b};
  assign e2 = {7'd0, a != 8'h5a};
  assign e3 = {7'd0, (a & 8'h0f) == 8'h0a};
  assign ar1 = a + b;
  assign ar2 = c - 8'd3;
  assign sh1 = a << b[1:0];
  assign sh2 = c >> idx;
  assign r1 = {a[3:0], b[7:4]};
  assign r2 = {2{c[3:0]}};
  assign bd = {7'd0, a[idx]};
  assign rd = mem[idx];
  assign wr = wide[idx*8 +: 8];
  always_comb begin
    if (a < b) q1 = 8'h33; else q1 = 8'h44;
  end
  always_comb begin
    q2 = 8'h11;
    if (a[3]) q2 = c; else if (b[2] && c[1]) q2 = a | b;
  end
  always_comb begin
    q3 = 8'h00;
    if (a[1:0] == 2'd1) q3 = b; else if (a[1:0] == 2'd2) q3 = c; else q3 = ~a;
  end
  always @(posedge clk) begin
    nq <= c ? a : b;
    nq2 <= mem[idx] ^ b;
    mem[cnt[1:0]] <= c;
    wide[cnt[3:0]*8 +: 8] <= a;
    cnt <= cnt + 1;
    cyc <= cyc + 1;
  end
  task show(input string tag);
    $display("%s m1=%h m2=%h m3=%h l1=%h l2=%h l3=%h e1=%h e2=%h e3=%h ar1=%h ar2=%h sh1=%h sh2=%h r1=%h r2=%h bd=%h rd=%h wr=%h q1=%h q2=%h q3=%h nq=%h nq2=%h w=%h %0d",
      tag, m1, m2, m3, l1, l2, l3, e1, e2, e3, ar1, ar2, sh1, sh2, r1, r2, bd, rd, wr, q1, q2, q3, nq, nq2, wide[63:0], cyc);
  endtask
  initial begin
    mem[0] = 8'h11; mem[1] = 8'hx1; mem[2] = 8'h22; mem[3] = 8'hzz;
    wide = 128'hx; wide[31:0] = 32'h5555_aaaa;
    a = 8'b1010_xxx1; b = 8'b1x10_0110; c = 8'h00; s = 1; idx = 1; cnt = 0;
    #1 show("A");
    s = 0; #1 show("B");
    a = 8'h5a; b = 8'h5a; c = 8'bx; idx = 4'bxx01; #1 show("C");
    idx = 9; c = 8'b0000_0z01; #1 show("D");
    a = 8'h03; b = 8'h07; c = 8'h80; idx = 2; #1 show("E");
    repeat (6) @(posedge clk);
    #1 show("F");
    a = 8'bxxxx_xxxx; b = 8'h00; c = 8'hff; #1 show("G");
    repeat (6) @(posedge clk);
    #1 show("H");
    a = 8'b0000_z0z0; b = 8'b1111_0z0z; c = 8'h0f; idx = 4'b00x0; #1 show("I");
    $finish;
  end
endmodule
"#;

#[test]
fn x_plane_executor_matches_the_four_state_vm() {
    let text = run("x_plane", DESIGN);
    let expected = [
        "A m1=aX m2=aX m3=X6 l1=01 l2=00 l3=00 e1=00 e2=01 e3=00 ar1=xx ar2=fd sh1=XX sh2=00 r1=XX r2=00 bd=0X rd=x1 wr=aa q1=44 q2=11 q3=5X nq=xx nq2=xx w=xxxxxxxx5555aaaa 0",
        "B m1=X6 m2=aX m3=X6 l1=01 l2=00 l3=00 e1=00 e2=01 e3=00 ar1=xx ar2=fd sh1=XX sh2=00 r1=XX r2=00 bd=0X rd=x1 wr=aa q1=44 q2=11 q3=5X nq=xx nq2=xx w=xxxxxxxx5555aaaa 0",
        "C m1=5a m2=5a m3=XX l1=01 l2=0X l3=00 e1=01 e2=00 e3=01 ar1=b4 ar2=xx sh1=68 sh2=xx r1=a5 r2=xx bd=0X rd=xx wr=xx q1=44 q2=xx q3=xx nq=xx nq2=xx w=xxxxxxxx5555aaaa 0",
        "D m1=5a m2=5a m3=5X l1=01 l2=01 l3=00 e1=01 e2=00 e3=01 ar1=b4 ar2=xx sh1=68 sh2=00 r1=a5 r2=ZZ bd=0X rd=xx wr=xx q1=44 q2=0Z q3=0Z nq=xx nq2=xx w=xxxxxxxx5555aaaa 0",
        "E m1=07 m2=03 m3=87 l1=01 l2=01 l3=00 e1=00 e2=01 e3=00 ar1=0a ar2=7d sh1=18 sh2=20 r1=30 r2=00 bd=00 rd=22 wr=55 q1=33 q2=11 q3=fc nq=xx nq2=xx w=xxxxxxxx5555aaaa 0",
        "F m1=07 m2=03 m3=87 l1=01 l2=01 l3=00 e1=00 e2=01 e3=00 ar1=0a ar2=7d sh1=18 sh2=20 r1=30 r2=00 bd=00 rd=80 wr=03 q1=33 q2=11 q3=fc nq=03 nq2=87 w=xx03030303030303 7",
        "G m1=00 m2=xx m3=ff l1=00 l2=01 l3=0X e1=0X e2=0X e3=0X ar1=xx ar2=fc sh1=xx sh2=3f r1=x0 r2=ff bd=0X rd=80 wr=03 q1=44 q2=11 q3=xx nq=03 nq2=87 w=xx03030303030303 7",
        "H m1=00 m2=xx m3=ff l1=00 l2=01 l3=0X e1=0X e2=0X e3=0X ar1=xx ar2=fc sh1=xx sh2=3f r1=x0 r2=ff bd=0X rd=ff wr=03 q1=44 q2=11 q3=xx nq=xx nq2=ff w=xx03030303030303 13",
        "I m1=fZ m2=fZ m3=fX l1=0X l2=01 l3=0X e1=00 e2=01 e3=0X ar1=xx ar2=0c sh1=xx sh2=xx r1=Zf r2=ff bd=0X rd=xx wr=xx q1=44 q2=11 q3=fX nq=xx nq2=ff w=xx03030303030303 13",
    ];
    for e in expected {
        assert!(text.lines().any(|l| l == e), "missing `{e}` in:\n{text}");
    }
    // The blocks run once per step, not after each of its four writes: 77 runs.
    assert!(
        stat(&text, "x_plane_runs=") >= 50,
        "the x-plane executor did not run:\n{text}"
    );
}

/// `===` and `!==` lower to the same two-state compare as `==`/`!=` (they
/// agree on x-free operands), so a stream holding a case equality must not
/// take the x-plane path, where `==` yields x on an x operand but `===`
/// compares the x bits exactly. The first x-plane build answered x here.
#[test]
fn case_equality_on_x_operands_stays_exact() {
    let text = run(
        "case_eq_x",
        r#"
module tb;
  wire [7:0] a = 8'b1101x001;
  wire [7:0] b = 8'b1101x001;
  wire [7:0] m = 8'b1101z001;
  wire c, d, e, f;
  assign c = a === b;
  assign d = a !== b;
  assign e = a === 8'b1101x001;
  assign f = a === m;
  final $display("CASEEQ %b %b %b %b %b", c, d, e, f, a == b);
endmodule
"#,
    );
    assert!(
        text.lines().any(|l| l == "CASEEQ 1 0 1 0 x"),
        "case equality on x operands:\n{text}"
    );
}
