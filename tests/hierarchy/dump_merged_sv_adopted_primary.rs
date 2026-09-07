//! `--dump-merged-sv -s <top>` with a PRIMARY file whose module is
//! instantiated only from an adopted `-v` library file.
//!
//! The `-s` closure is lexical over the primary sources, so it never saw the
//! reference that originates inside the library text: the primary file was
//! dropped, and the merged output contained the instantiation (inside the
//! appended library module) but not the definition —
//!
//! ```text
//! Simulation error: Module 'leaf_mod' instantiated but not found
//! ```
//!
//! The append pass now scans the adopted library texts for references and
//! transitively re-adds the dropped primaries, so the artifact re-runs
//! standalone (user-reported repro: a debug_port_bfm primary instantiated
//! only from a -v BFM library).

use std::process::Command;

fn xezim_bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim")
}

const LEAF: &str = r#"
module leaf_mod (output logic [7:0] val);
  assign val = 8'hA5;
endmodule
"#;

const MID_LIB: &str = r#"
module mid_mod (output logic [7:0] val);
  leaf_mod u_leaf(.val(val));
endmodule
"#;

const TB: &str = r#"
module testbench;
  logic [7:0] val;
  mid_mod u_mid(.val(val));
  initial begin
    #10;
    if (val === 8'hA5) $display("TEST_PASS");
    else begin $display("TEST_FAIL val=%h", val); $fatal(1); end
    $finish;
  end
endmodule
"#;

#[test]
fn merged_dump_readds_primary_referenced_only_from_library() {
    let dir = std::env::temp_dir().join("xezim_merged_adopted_primary");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let (tb, leaf, lib) = (dir.join("tb.sv"), dir.join("leaf.sv"), dir.join("mid_lib.sv"));
    std::fs::write(&tb, TB).expect("w");
    std::fs::write(&leaf, LEAF).expect("w");
    std::fs::write(&lib, MID_LIB).expect("w");
    let merged = dir.join("merged.sv");

    let dump = Command::new(xezim_bin())
        .arg("--simulate")
        .arg("-s")
        .arg("testbench")
        .arg("--max-time")
        .arg("50ns")
        .arg(&tb)
        .arg(&leaf)
        .arg("-v")
        .arg(&lib)
        .arg("--dump-merged-sv")
        .arg(&merged)
        .output()
        .expect("run xezim");
    let dump_out = String::from_utf8_lossy(&dump.stdout).to_string();
    assert!(
        dump_out.contains("Re-added 1 primary file(s)"),
        "the dropped primary was not re-added:\n{dump_out}"
    );

    let text = std::fs::read_to_string(&merged).expect("read merged");
    assert_eq!(
        text.matches("module leaf_mod").count(),
        1,
        "leaf_mod must appear exactly once in the merged dump:\n{text}"
    );

    // Re-run the merged artifact ALONE, from its own directory.
    let iso = dir.join("iso");
    std::fs::create_dir_all(&iso).expect("mkdir iso");
    let iso_merged = iso.join("merged.sv");
    std::fs::copy(&merged, &iso_merged).expect("copy");
    let rerun = Command::new(xezim_bin())
        .current_dir(&iso)
        .env("XEZIM_NO_CACHE", "1")
        .arg("--simulate")
        .arg("-s")
        .arg("testbench")
        .arg("--max-time")
        .arg("50ns")
        .arg(&iso_merged)
        .output()
        .expect("run merged");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&rerun.stdout),
        String::from_utf8_lossy(&rerun.stderr)
    );
    assert!(
        combined.contains("TEST_PASS"),
        "merged artifact did not re-run standalone:\n{combined}"
    );
}
