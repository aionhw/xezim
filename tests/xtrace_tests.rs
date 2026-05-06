use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_path(tag: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut path = std::env::temp_dir();
    path.push(format!(
        "xezim_{}_{}_{}.{}",
        tag,
        std::process::id(),
        nanos,
        ext
    ));
    path
}

#[test]
fn xtrace_cli_emits_level0_text_with_profile() {
    let src = temp_path("xtrace_cli", "sv");
    let trace = temp_path("xtrace_cli", "xt");
    fs::write(
        &src,
        r#"
        module test;
            wire observed;
            reg clk;
            reg [7:0] counter;
            assign observed = clk;
            initial begin
                clk = 0;
                counter = 0;
                #1 counter = 8'h2a;
                #1 $finish;
            end
            always #1 clk = ~clk;
        endmodule
        "#,
    )
    .expect("write xtrace test source");

    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--simulate")
        .arg("-s")
        .arg("test")
        .arg("--max-time")
        .arg("10")
        .arg("--xtrace")
        .arg(&trace)
        .arg("--xtrace-profile")
        .arg("raw_delta")
        .arg(&src)
        .output()
        .expect("run xezim with xtrace");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "xezim failed\nstdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );

    let text = fs::read_to_string(&trace).expect("read xtrace output");
    let _ = fs::remove_file(&src);
    let _ = fs::remove_file(&trace);

    assert!(text.contains("@xtrace 1.1"));
    assert!(text.contains("@format text"));
    assert!(text.contains("@profile raw_delta"));
    assert!(text.contains("@capabilities signal_delta"));
    assert!(text.contains("@compression none"));
    assert!(text.contains("@extensions ignore_unknown"));
    assert!(text.contains("# xtrace-signals debug"));
    assert!(text.contains("@section dict"));
    assert!(text.contains("@section trace"));
    assert!(text.contains("@section end"));
    assert!(text.contains("N,full"));
    assert!(text.contains(",enc=delta,width="));
    assert!(!text.contains("X,sim_telemetry"));
}

#[test]
fn xtrace_level_one_is_reserved() {
    let src = temp_path("xtrace_reserved", "sv");
    let trace = temp_path("xtrace_reserved", "xt");
    fs::write(
        &src,
        r#"
        module test;
            initial begin
                #1 $finish;
            end
        endmodule
        "#,
    )
    .expect("write xtrace reserved test source");

    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--simulate")
        .arg("-s")
        .arg("test")
        .arg("--max-time")
        .arg("10")
        .arg("--xtrace")
        .arg(&trace)
        .arg("--xtrace-level")
        .arg("1")
        .arg(&src)
        .output()
        .expect("run xezim with reserved xtrace level");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "xezim failed\nstdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
    assert!(stderr.contains("--xtrace-level 1 is reserved"));

    let text = fs::read_to_string(&trace).expect("read xtrace output");
    let _ = fs::remove_file(&src);
    let _ = fs::remove_file(&trace);
    assert!(text.contains("@profile minimal"));
    assert!(!text.contains("X,sim_telemetry"));
}
