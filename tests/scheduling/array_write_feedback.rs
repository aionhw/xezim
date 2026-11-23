//! §4.4.2.2, §10.4.1: a loop's indexed stores can feed subsequent reads.

#[test]
fn indexed_memory_write_feedback_keeps_the_loop_interpreted() {
    use std::fs;
    use std::process::Command;

    let path = std::env::temp_dir().join(format!("xezim-array-feedback-{}.sv", std::process::id()));
    fs::write(
        &path,
        r#"
module top;
  logic [7:0] data[0:3];
  wire [7:0] forwarded = data[0];
  initial begin
    data[0] = 0;
    #1;
    for (int i = 0; i < 3; i++) data[i] = forwarded + 1;
    #1;
    if (forwarded !== 8'd1) $fatal(1, "forwarded value was not updated");
    $finish;
  end
endmodule
"#,
    )
    .expect("write source");
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .env("XEZIM_PROC_LOOP_STATS", "1")
        .arg("--no-cache")
        .arg(&path)
        .output();
    let _ = fs::remove_file(path);
    let output = output.expect("run xezim");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{}\n{stderr}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        stderr.contains("compiled=false reason=comb_feedback"),
        "{stderr}"
    );
}
