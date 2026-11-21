use std::process::Command;

#[test]
fn signed_index_local_writes_compile() {
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/types/signed_index_local_write.sv"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .env("XEZIM_COMPILE_FAIL_STATS", "1")
        .args(["--no-cache", "-s", "top", "-DTOGGLES=32", fixture])
        .output()
        .expect("run simulator");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert_eq!(
        stdout
            .lines()
            .filter(|l| l.trim_start().starts_with("PASS"))
            .count(),
        6,
        "{stdout}"
    );
    assert!(!stdout.contains("FAIL"), "{stdout}");
    assert!(!stderr.contains("blocking_frame_target"), "{stderr}");
}
