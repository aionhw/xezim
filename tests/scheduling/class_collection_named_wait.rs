use std::process::Command;

#[test]
fn class_methods_named_like_collection_builtins_do_not_spin_or_miss_writes() {
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/class_collection_named_wait.sv"
    );
    for compile in ["0", "1"] {
        let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
            .args(["--no-cache", "--simulate", "--error-exit", fixture])
            .env("XEZIM_STALL_LIMIT", "100")
            .env("XEZIM_COMPILE_METHODS", compile)
            .env("XEZIM_METHOD_TIER", "0")
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.status.success()
                && text.contains("PASS")
                && !text.contains("no time progress")
                && !text.contains("STALLED"),
            "compile={compile}: {text}"
        );
    }
}
