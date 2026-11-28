//! IEEE 1800-2023 §21.3.4.3, Table 21-7 (`%t`), and §20.4.3, Table 20-3.

fn assert_pass(result: Result<xezim::compiler::Simulator, String>) {
    let sim = result.expect("simulate failed");
    let lines: Vec<_> = sim.output.iter().map(|line| line.message.as_str()).collect();
    assert!(lines.contains(&"PASS"), "{lines:?}");
}

fn check(source: &str) {
    assert_pass(xezim::simulate(source, 2_000_000));
}

#[test]
fn sscanf_time_rounding_and_destinations() {
    check(include_str!("scan_time/rounding.sv"));
}

#[test]
fn sscanf_time_scopes_and_default_format() {
    check(include_str!("scan_time/scopes.sv"));
}

#[test]
fn sscanf_time_case_and_zero_width_extensions() {
    check(include_str!("scan_time/extensions.sv"));
}

#[test]
fn sscanf_non_time_conversions_and_time_display() {
    check(include_str!("scan_time/guards.sv"));
}

#[cfg(unix)]
#[test]
fn fscanf_time_stream_and_task_forms() {
    let scratch = std::process::Command::new("mktemp")
        .arg("-d")
        .arg(std::env::temp_dir().join("xezim-scan-time.XXXXXXXX"))
        .output()
        .expect("launch mktemp");
    assert!(scratch.status.success(), "{scratch:?}");
    let dir = std::path::Path::new(std::str::from_utf8(&scratch.stdout).unwrap().trim_end());
    let path = dir.join("scan-time.txt");
    let sv_path = path
        .to_str()
        .unwrap()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let source = include_str!("scan_time/stream.sv").replace("scan-time.txt", &sv_path);
    let result = xezim::simulate(&source, 100);
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
    std::fs::remove_dir(dir).unwrap();
    assert_pass(result);
}
