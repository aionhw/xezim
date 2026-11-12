//! `--dump-merged-sv` with several `-v` libraries that each carry their own
//! copy of a `$unit`-scope helper subroutine.
//!
//! A `-v`/`-y` library contributes only the definitions an instantiation
//! needed, so the ORIGINAL run never sees two copies of the same
//! compilation-unit task. The dump appends whole library files, so the merged
//! file did — and §26.2 makes a repeat declaration in the same scope an error,
//! so the artifact would not re-compile:
//!
//! ```text
//! Simulation error: duplicate declaration of 'urandom_wrapper' in the same scope
//! ```
//!
//! Duplicates are now suppressed at append time (first definition wins, the
//! elaborator's own order). Scope matters: two libraries may legitimately
//! define the same-named task INSIDE their own modules, and both must survive —
//! pinned below, since a dedup that ignored nesting would silently delete one
//! module's task and change behaviour.

use std::process::Command;

fn xezim_bin() -> std::path::PathBuf {
    // Cargo names the binary it built for the integration tests.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_xezim"))
}

const LIB_X: &str = r#"
task automatic helper_x(output int v); v = 1; endtask
task automatic dup_helper(output int v); v = 7; endtask
module mod_x (input clk, output int o);
  task automatic same_name(output int v); v = 11; endtask
  always @(posedge clk) same_name(o);
endmodule
"#;

const LIB_Y: &str = r#"
task automatic helper_y(output int v); v = 2; endtask
task automatic dup_helper(output int v); v = 7; endtask
module mod_y (input clk, output int o);
  task automatic same_name(output int v); v = 22; endtask
  always @(posedge clk) same_name(o);
endmodule
"#;

const TB: &str = r#"
module testbench;
  logic clk = 0;
  int a, b;
  mod_x u1 (.clk(clk), .o(a));
  mod_y u2 (.clk(clk), .o(b));
  always #5 clk = ~clk;
  initial begin
    #12;
    $display("VALUES a=%0d b=%0d", a, b);
    $finish;
  end
endmodule
"#;

/// The merged dump must re-compile standalone, produce the same values as the
/// multi-file run, and keep both module-scope `same_name` tasks.
#[test]
fn merged_dump_dedups_unit_subroutines_but_keeps_scoped_ones() {
    let dir = std::env::temp_dir().join("xezim_merged_lib_dedup");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let (tb, lx, ly) = (dir.join("tb.sv"), dir.join("libx.sv"), dir.join("liby.sv"));
    std::fs::write(&tb, TB).expect("w");
    std::fs::write(&lx, LIB_X).expect("w");
    std::fs::write(&ly, LIB_Y).expect("w");
    let merged = dir.join("merged.sv");

    let dump = Command::new(xezim_bin())
        .arg("--simulate")
        .arg("-s")
        .arg("testbench")
        .arg("--max-time")
        .arg("30ns")
        .arg(&tb)
        .arg("-v")
        .arg(&lx)
        .arg("-v")
        .arg(&ly)
        .arg("--dump-merged-sv")
        .arg(&merged)
        .output()
        .expect("run xezim");
    let dump_out = String::from_utf8_lossy(&dump.stdout).to_string();
    assert!(
        dump_out.contains("suppressed 1 duplicate"),
        "the duplicate $unit task was not suppressed:\n{dump_out}"
    );

    let text = std::fs::read_to_string(&merged).expect("read merged");
    assert_eq!(
        text.matches("task automatic dup_helper").count(),
        1,
        "duplicate $unit task still appears twice:\n{text}"
    );
    // Module-scope tasks live in different scopes — both must be kept.
    assert_eq!(
        text.matches("task automatic same_name").count(),
        2,
        "a module-scope task was wrongly stripped:\n{text}"
    );

    // Re-run the merged artifact ALONE, from its own directory.
    let iso = dir.join("iso");
    std::fs::create_dir_all(&iso).expect("mkdir iso");
    let iso_merged = iso.join("merged.sv");
    std::fs::copy(&merged, &iso_merged).expect("copy");
    let rerun = Command::new(xezim_bin())
        .current_dir(&iso)
        .arg("--simulate")
        .arg("-s")
        .arg("testbench")
        .arg("--max-time")
        .arg("30ns")
        .arg(&iso_merged)
        .output()
        .expect("run merged");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&rerun.stdout),
        String::from_utf8_lossy(&rerun.stderr)
    );
    assert!(
        !combined.contains("duplicate declaration"),
        "merged artifact still hits the duplicate-declaration error:\n{combined}"
    );
    assert!(
        combined.contains("VALUES a=11 b=22"),
        "merged artifact did not reproduce the original run's values:\n{combined}"
    );
}
