//! §6.21, §13.3.1 (#833): a frame local that a deeper frame's same-named
//! local displaces gets its type metadata back when that deeper frame
//! returns. The live frames were rescanned for displaced metadata after
//! every frame pop below the displacement, so each of the 500 `leaf()` calls
//! here paid for the whole stack, and a deep UVM call chain stalled for
//! hours at one timestamp. Only a return that leaves a displaced name
//! without its owner rescans now. `local_meta_reapplies` counts the scans.
use std::process::Command;

const DESIGN: &str = r#"
task automatic leaf(); int y; y = 1; endtask
task automatic deep(input int n); int a, b; for (int k = 0; k < n; k++) leaf(); endtask
task automatic inner(output string s); string x; x = "inner"; deep(500); s = x; endtask
task automatic outer(); int x; string s; x = 41; inner(s); x = x + 1; $display("T|%s %0d", s, x); endtask
module top;
  initial outer();
endmodule
"#;

#[test]
fn displaced_local_is_restored_without_rescanning_every_return() {
    let dir = std::env::temp_dir().join(format!("xezim_meta_reapply_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, DESIGN).expect("write design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "top", path.to_str().unwrap(), "--no-cache"])
        .env("XEZIM_PROFILE_TIMING", "1")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    let lines: Vec<&str> = text.lines().filter(|l| l.starts_with("T|")).collect();
    assert_eq!(lines, ["T|inner 42"]);
    let scans: u64 = text
        .lines()
        .find_map(|l| l.strip_prefix("[PROF] local_meta_reapplies="))
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or_else(|| panic!("missing local_meta_reapplies:\n{text}"));
    // One for `inner` returning; one per `leaf()` return before the fix.
    assert!((1..=10).contains(&scans), "local_meta_reapplies={scans}");
}
