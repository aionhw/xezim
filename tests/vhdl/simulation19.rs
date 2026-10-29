//! VHDL-2019 designs simulated end to end: each `designs19/<name>.vhd` runs
//! through `xezim --std=19`, and its TEXTIO output and exit status must equal
//! `designs19/<name>.golden` and `designs19/<name>.exit`, produced by nvc
//! (`designs19/regen.sh`; GHDL 4.1 as packaged has no VHDL-2019 libraries).
//! nvc prints report and assertion messages on stderr in its own format, so
//! xezim's (GHDL-format) report lines are left out of the comparison; the
//! designs write their results with std.textio.

use std::path::PathBuf;
use std::process::Command;

fn designs() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vhdl/designs19")
}

/// A line xezim prints that nvc does not print on stdout: a report or
/// assertion message (`file:line:col:@time:(report note): ...`) or a GHDL
/// error line.
fn is_message(l: &str) -> bool {
    l.starts_with("ghdl:error:") || (l.contains(":@") && l.contains(":(") && l.contains("): "))
}

fn check(name: &str) {
    let dir = designs();
    // Run in a directory of its own: a design may create files.
    let run_dir =
        std::env::temp_dir().join(format!("xezim-vhdl19-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&run_dir);
    std::fs::create_dir_all(&run_dir).expect("run directory");
    let src = format!("{}.vhd", name);
    std::fs::copy(dir.join(&src), run_dir.join(&src)).expect("copy design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&run_dir)
        .args(["--no-cache", "--std=19"])
        .arg(&src)
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&run_dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let actual: Vec<&str> = stdout
        .lines()
        .filter(|l| {
            !(l.starts_with("=== xezim")
                || l.starts_with("git ")
                || l.starts_with("Simulation finished at time")
                || l.starts_with("[xezim]")
                || is_message(l))
        })
        .collect();
    let golden = std::fs::read_to_string(dir.join(format!("{}.golden", name))).expect("golden");
    let exit: i32 = std::fs::read_to_string(dir.join(format!("{}.exit", name)))
        .expect("exit status")
        .trim()
        .parse()
        .expect("exit status number");
    assert_eq!(
        actual.join("\n").trim_end(),
        golden.trim_end(),
        "{}.vhd differs from nvc\nstdout:\n{}\nstderr:\n{}",
        name,
        stdout,
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(exit), "{}.vhd exit status", name);
}

/// A design analysed as VHDL-2019 (IEEE 1076-2019): the 2008 base —
/// processes, signals, waits, records, TEXTIO — under `--std=19`.
#[test]
fn basics19() {
    check("basics19");
}
