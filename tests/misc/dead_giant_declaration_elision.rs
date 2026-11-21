//! Beyond-cap packed declarations referenced NOWHERE in the design are
//! elided at elaboration (a note, no width-cap warning, no allocation) —
//! matching commercial simulators, which dead-code-eliminate them silently.
//! Surfaced by a customer BFM carrying a leftover multi-megabit debug
//! struct array from an old project. A REFERENCED over-cap declaration
//! retains its full width or fails elaboration when it exceeds the limit.

use std::process::Command;

fn run(src: &str, case: &str, limit: Option<&str>) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("xezim_dge_{}_{case}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.sv");
    std::fs::write(&path, src).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.env_remove("XEZIM_MAX_PACKED_WIDTH");
    if let Some(limit) = limit {
        cmd.env("XEZIM_MAX_PACKED_WIDTH", limit);
    }
    let out = cmd.args([
            "--simulate",
            "-s",
            "tb",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .output()
        .expect("run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    std::fs::remove_dir_all(dir).unwrap();
    (out.status.success(), text)
}

#[test]
fn dead_giants_elide_and_live_giant_retains_its_top_bit() {
    let (ok, text) = run(r#"package p_defs;
  typedef struct packed {
    bit [31:0] acc;
    bit [15:0] count;
    bit [9:0]  mn;
    bit [9:0]  mx;
  } stats_t;
endpackage
module tb;
  import p_defs::*;
  stats_t [127:0][15:0][15:0] dbg_stats_arr; // dead: elide, no warning
  bit [262143:0][15:0] dbg_flat;             // dead: elide, no warning
  bit [2097151:0] live_big;                  // live: retain its declared width
  logic [7:0] r = 0;
  initial begin
    live_big[2097151] = 1'b1;
    r = 8'h5A;
    #1;
    if (r === 8'h5A && live_big[2097151] === 1'b1) $display("TEST_PASS");
    else $display("TEST_FAIL");
    $finish;
  end
endmodule
"#, "elide", None);
    assert!(ok, "{text}");
    assert!(text.contains("TEST_PASS"), "sim result:\n{text}");
    assert!(
        text.contains("eliding dead 2228224-bit declaration 'dbg_stats_arr'"),
        "struct-array elision note:\n{text}"
    );
    assert!(
        text.contains("eliding dead 4194304-bit declaration 'dbg_flat'"),
        "vector elision note:\n{text}"
    );
    // The elided declarations must NOT also warn.
    assert!(
        !text.contains("packed width 2228224") && !text.contains("packed width 4194304"),
        "elided decls still warned:\n{text}"
    );
    assert!(!text.contains("exceeds"), "{text}");
}

#[test]
fn packed_memory_width_limit_is_enforced_before_simulation() {
    let design = r#"module tb;
      parameter DEPTH = 400000, DBITS = 40;
      reg [DEPTH-1:0][DBITS-1:0] mem;
      int idx;
      initial begin
        idx = DEPTH - 1;
        mem[0] = 40'h111ff222aa;
        mem[idx] = 40'hdead123456;
        if ($bits(mem) == 16000000 && mem[0] === 40'h111ff222aa
            && mem[idx] === 40'hdead123456) $display("TEST_PASS");
        $finish;
      end
    endmodule"#;
    let (ok, text) = run(design, "packed", None);
    assert!(ok && text.contains("TEST_PASS"), "{text}");
    let (ok, text) = run(design, "limited", Some("12000000"));
    assert!(!ok && text.contains("packed width 16000000 exceeds implementation limit 12000000"), "{text}");
    assert!(!text.contains("TEST_PASS"), "{text}");
}
