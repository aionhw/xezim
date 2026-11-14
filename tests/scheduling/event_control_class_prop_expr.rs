use std::process::Command;

#[test]
fn event_control_class_property_expressions() {
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/event_control_class_prop_expr.sv"
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
