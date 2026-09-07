//! A nonblocking write to a packed-struct member must COMPILE, not fall back.
//!
//! `s.field <= v` reaches the compiler as a two-segment `Ident`, and a packed
//! member is a bit slice of its container rather than a signal of its own, so
//! the signal lookup misses. The blocking path already handled this by
//! splicing into `[off + w - 1 : off]` of the container; the NBA path had no
//! such arm and bailed (`nba_ident_unresolved`), sending every member write to
//! the AST interpreter. That was correct but slow — a struct-pipeline design
//! spent 4.9s of a 16.0s run in the fallback, 2.88M of them.
//!
//! The value assertions alone would pass on the fallback path too, so this
//! test is only meaningful together with the fallback-count assertion: it
//! checks that the COMPILED path produces the reference's answer. Expected
//! values are the reference simulator's, verified line for line.

use std::process::Command;

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

const SRC: &str = r#"
typedef struct packed {
  logic        valid;
  logic [7:0]  id;
  logic [39:0] addr;
  logic [31:0] data;
} req_t;

module tb;
  logic clk = 0, rst_n = 0;
  req_t s1, s2;
  logic [39:0] a1, a2;
  logic [31:0] d1, d2;
  logic [7:0]  i1, i2;
  int unsigned cyc = 0;
  always #5 clk = ~clk;

  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      s1 <= '0;
      s2 <= '0;
    end else begin
      // Several members of ONE container per cycle: each range NBA must
      // compose onto the pending entry, not overwrite the whole signal.
      s1.valid <= 1'b1;
      s1.id    <= cyc[7:0];
      s1.addr  <= 40'hFF_0000_0000 + cyc;
      s1.data  <= cyc * 32'd2654435761;
      s2.valid <= s1.valid;
      s2.id    <= s1.id + 8'd3;
      s2.addr  <= s1.addr ^ 40'h0F_00FF_00FF;
      s2.data  <= {s1.data[15:0], s1.data[31:16]};
    end
  end

  always @(posedge clk) if (rst_n) cyc <= cyc + 1;

  initial begin
    #12 rst_n = 1;
    repeat (20) @(posedge clk);
    #1;
    a1 = s1.addr; a2 = s2.addr;
    d1 = s1.data; d2 = s2.data;
    i1 = s1.id;   i2 = s2.id;
    $display("S1 addr=%010x data=%08x id=%02x valid=%b", a1, d1, i1, s1.valid);
    $display("S2 addr=%010x data=%08x id=%02x valid=%b", a2, d2, i2, s2.valid);
    $finish;
  end
endmodule
"#;

#[test]
fn member_nba_compiles_and_matches_the_reference() {
    let dir = std::env::temp_dir().join("xezim_packed_member_nba");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("dut.sv");
    std::fs::write(&path, SRC).expect("write");

    let out = Command::new(xezim())
        .current_dir(&dir)
        .arg("--simulate")
        .arg("--max-time")
        .arg("5000")
        .arg("-s")
        .arg("tb")
        .arg(&path)
        .env("XEZIM_PROFILE_REPORT", "1")
        .output()
        .expect("run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // Reference simulator, same source:
    //   S1 addr=ff00000013 data=be1e0823 id=13 valid=1
    //   S2 addr=f000ff00ed data=8e721fe6 id=15 valid=1
    assert!(
        text.contains("S1 addr=ff00000013 data=be1e0823 id=13 valid=1"),
        "stage-1 members do not match the reference:\n{text}"
    );
    assert!(
        text.contains("S2 addr=f000ff00ed data=8e721fe6 id=15 valid=1"),
        "stage-2 members do not match the reference:\n{text}"
    );

    // Without this the assertions above only prove the AST fallback is
    // correct, which was never in doubt.
    assert!(
        !text.contains("nba_ident_unresolved"),
        "member NBAs fell back to the AST path instead of compiling:\n{text}"
    );
}

/// `arr[i].m` — an indexed base stays a `MemberAccess` node instead of
/// collapsing to a dotted `Ident`, so it needs its own arm on both the NBA
/// and blocking sides, plus a widened `for_body_is_simple` or the enclosing
/// loop drops to the AST path wholesale (`For_step_other`).
fn run(src: &str, tag: &str, max_time: &str) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_packed_idx_{tag}"));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("dut.sv");
    std::fs::write(&path, src).expect("write");
    let out = Command::new(xezim())
        .current_dir(&dir)
        .arg("--simulate")
        .arg("--max-time")
        .arg(max_time)
        .arg("-s")
        .arg("tb")
        .arg(&path)
        .env("XEZIM_PROFILE_REPORT", "1")
        .output()
        .expect("run xezim");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

const ELEM: &str = "typedef struct packed { logic v; logic [7:0] id; logic [31:0] d; } e_t;";

#[test]
fn indexed_member_nba_compiles_for_constant_and_dynamic_indices() {
    let src = format!(
        r#"{ELEM}
module tb;
  logic clk = 0;
  e_t arr [4];
  logic [1:0] sel = 2;
  int unsigned cyc = 0;
  always #5 clk = ~clk;
  always @(posedge clk) begin
    cyc        <= cyc + 1;
    arr[1].d   <= cyc + 32'd100;
    arr[sel].d <= cyc + 32'd200;
  end
  initial begin
    repeat (6) @(posedge clk); #1;
    $display("A1=%08x A2=%08x", arr[1].d, arr[2].d);
    $finish;
  end
endmodule
"#
    );
    let text = run(&src, "cd", "200");
    // Reference simulator: A1=00000069 A2=000000cd
    assert!(
        text.contains("A1=00000069 A2=000000cd"),
        "indexed-member NBA does not match the reference:\n{text}"
    );
    assert!(
        !text.contains("nba_member_access"),
        "indexed-member NBA fell back instead of compiling:\n{text}"
    );
}

#[test]
fn indexed_member_assignment_inside_a_loop_compiles() {
    // The loop is the point: `for_body_is_simple` used to reject `arr[i].m`,
    // which bailed the WHOLE loop to the AST path at ~3.8us/statement.
    let body = |op: &str| {
        format!(
            r#"{ELEM}
module tb;
  logic clk = 0;
  e_t  arr [4];
  int unsigned cyc = 0;
  integer i;
  always #5 clk = ~clk;
  always @(posedge clk) begin
    cyc {op} cyc + 1;
    for (i = 0; i < 4; i++) begin
      arr[i].v  {op} 1'b1;
      arr[i].id {op} cyc[7:0] + i[7:0];
      arr[i].d  {op} cyc * 32'd7 + i;
    end
  end
  initial begin
    repeat (10) @(posedge clk); #1;
    $display("O0=%08x O3=%08x ID3=%02x", arr[0].d, arr[3].d, arr[3].id);
    $finish;
  end
endmodule
"#
        )
    };

    // Reference simulator, nonblocking: O0=0000003f O3=00000042 ID3=0c
    let nba = run(&body("<="), "nba", "3000");
    assert!(
        nba.contains("O0=0000003f O3=00000042 ID3=0c"),
        "looped indexed-member NBA does not match the reference:\n{nba}"
    );

    // Reference simulator, blocking: O0=00000046 O3=00000049 ID3=0d
    let blk = run(&body("="), "blk", "3000");
    assert!(
        blk.contains("O0=00000046 O3=00000049 ID3=0d"),
        "looped indexed-member blocking assign does not match the reference:\n{blk}"
    );

    for (tag, text) in [("nonblocking", &nba), ("blocking", &blk)] {
        assert!(
            !text.contains("For_step_other")
                && !text.contains("nba_member_access")
                && !text.contains("blocking_target_member_access"),
            "{tag} loop fell back instead of compiling:\n{text}"
        );
    }
}
