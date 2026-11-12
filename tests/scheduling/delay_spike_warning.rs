//! A `#delay` that evaluates to a pathological real value (NaN/inf or an
//! absurdly large number — typically IEEE-754 bit garbage) parks the process
//! ~forever, which reads as "the clock stopped". eval_delay_ticks now warns
//! once per site so the offending delay names itself in the log instead of
//! silently freezing. The warning goes to stderr, so this drives the CLI.

fn xezim_bin() -> std::path::PathBuf {
    // Cargo names the binary it built for the integration tests.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_xezim"))
}

fn run(src: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_delayspike_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join("t.sv");
    std::fs::write(&sv, src).unwrap();
    let out = std::process::Command::new(xezim_bin())
        .arg(&sv)
        .arg("--max-time")
        .arg("100000")
        .output()
        .expect("run xezim");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// A clock whose real period spikes huge must emit the pathological-#delay
/// warning (naming the value and site).
#[test]
fn huge_real_delay_warns() {
    const SRC: &str = r#"
`timescale 1ps/1ps
module top;
  real d; logic clk = 0;
  always #(d/2.0) clk = ~clk;
  initial begin
    d = 100.0; #500;
    d = 1.0e18;      // spike -> pathological #delay
    #500 $finish;
  end
endmodule
"#;
    let log = run(SRC, "huge");
    assert!(
        log.contains("[xezim][warning]") && log.contains("pathological real value"),
        "a huge real #delay must warn:\n{}",
        log
    );
    // The warning must NOT contain the word "error" (the ivtest runner greps for it).
    let warn_line = log
        .lines()
        .find(|l| l.contains("pathological real value"))
        .unwrap_or("");
    assert!(
        !warn_line.to_lowercase().contains("error"),
        "warning line must not contain 'error':\n{}",
        warn_line
    );
}

/// A well-behaved real-period clock must NOT warn.
#[test]
fn normal_real_delay_no_warn() {
    const SRC: &str = r#"
`timescale 1ps/1ps
module top;
  real d; logic clk = 0;
  always #(d/2.0) clk = ~clk;
  initial begin d = 100.0; #2000 $finish; end
endmodule
"#;
    let log = run(SRC, "normal");
    assert!(
        !log.contains("pathological real value"),
        "a normal real #delay must not warn:\n{}",
        log
    );
}
