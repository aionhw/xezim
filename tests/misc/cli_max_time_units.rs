//! --max-time units and existing whole-nanosecond rounding. The design's
//! timeunit/timeprecision and delays follow IEEE 1800-2023 §3.14.2.2 and §9.4.1;
//! output and termination follow §21.2.1 and §20.2. CLI rounding is a tool policy.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(tag: &str, precision: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xezim_max_time_{}_{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("top.sv"),
        format!(
            r#"module top;
  timeunit 1ns; timeprecision {precision};
  initial begin
    #924999ns $display("BEFORE_CAP");
    #2ns $display("AFTER_CAP");
    $finish;
  end
endmodule
"#
        ),
    )
    .unwrap();
    dir
}

fn run(dir: &Path, value: &str, attached: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xezim"));
    command
        .current_dir(dir)
        .args(["--no-cache", "--verbose", "top.sv"]);
    if attached {
        command.arg(format!("--max-time={value}"));
    } else {
        command.args(["--max-time", value]);
    }
    command.output().unwrap()
}

#[test]
fn femtoseconds_match_existing_units_at_each_precision() {
    for precision in ["1ns", "1ps", "1fs"] {
        let dir = fixture(precision, precision);
        for value in [
            "925000000000fs",
            "925000000000FS",
            "925000000ps",
            "925000ns",
            "925us",
            "925µs",
            "0.925ms",
            "0.000925s",
            "925000",
        ] {
            for attached in [false, true] {
                let out = run(&dir, value, attached);
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                assert!(out.status.success(), "{value}: {stdout} {stderr}");
                assert!(stdout.contains("Max time: 925000 ns"), "{value}: {stdout}");
                assert!(
                    stdout.contains("BEFORE_CAP") && !stdout.contains("AFTER_CAP"),
                    "{value}: {stdout}"
                );
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn femtoseconds_keep_existing_rounding_and_errors() {
    let dir = fixture("rounding", "1fs");
    for (value, ns) in [
        ("500000fs", 1),
        ("1499999fs", 1),
        ("1500000fs", 2),
        ("1500000Fs", 2),
        ("infs", u64::MAX),
        ("INFS", u64::MAX),
        ("inffs", u64::MAX),
        ("1e30fs", u64::MAX),
        ("500ps", 1),
        ("1499ps", 1),
        ("1500ps", 2),
        ("0.5ns", 1),
        ("1.5ns", 2),
    ] {
        let out = run(&dir, value, false);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "{value}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            stdout.contains(&format!("Max time: {ns} ns")),
            "{value}: {stdout}"
        );
    }
    for (value, expected) in [
        ("499999fs", "below 1 ns"),
        ("1fs", "below 1 ns"),
        ("499ps", "below 1 ns"),
        ("0fs", "must be positive"),
        ("-1fs", "must be positive"),
        ("0ns", "must be positive"),
        ("NaNfs", "must be positive"),
        ("badfs", "expected <n>[fs|ps|ns|us|ms|s]"),
        ("1zs", "expected <n>[fs|ps|ns|us|ms|s]"),
    ] {
        for attached in [false, true] {
            let out = run(&dir, value, attached);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert_eq!(out.status.code(), Some(1), "{value}: {stderr}");
            assert!(stderr.contains(expected), "{value}: {stderr}");
        }
    }
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--max-time <n>[fs|ps|ns|us|ms|s]"));
    std::fs::remove_dir_all(dir).unwrap();
}
