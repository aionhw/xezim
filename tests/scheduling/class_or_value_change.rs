use std::process::Command;

#[test]
fn class_property_event_or_waits_for_value_changes() {
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/class_or_value_change.sv"
    );
    for compile in ["0", "1"] {
        let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
            .args(["--no-cache", "--simulate", "--error-exit", fixture])
            .env("XEZIM_COMPILE_METHODS", compile)
            .env("XEZIM_METHOD_TIER", "0")
            .env("XEZIM_STALL_LIMIT", "100")
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            out.status.success()
                && text.contains("PASS")
                && !text.contains("no time progress")
                && !text.contains("STALLED"),
            "compile={compile}: {text}"
        );
    }
}
