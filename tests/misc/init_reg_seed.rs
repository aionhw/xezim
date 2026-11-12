//! `XEZIM_INIT_REG=random` gives registers a per-register value at time 0
//! that changes with the run's seed (`+seed=` / `-sv_seed`). The same seed
//! reproduces the same values, and no seed equals the default seed 1.

use std::path::{Path, PathBuf};
use std::process::Command;

fn xezim() -> String {
    // Cargo names the binary it built for the integration tests.
    env!("CARGO_BIN_EXE_xezim").to_string()
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_init_reg_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Eight 16-bit flops that are never reset, printed before the first clock.
const DESIGN: &str = r#"
module top;
  reg clk = 0;
  reg [15:0] r0, r1, r2, r3, r4, r5, r6, r7;
  always @(posedge clk) begin
    r0 <= r1; r1 <= r2; r2 <= r3; r3 <= r4;
    r4 <= r5; r5 <= r6; r6 <= r7; r7 <= r0;
  end
  initial begin
    #1 $display("T|%h %h %h %h %h %h %h %h", r0, r1, r2, r3, r4, r5, r6, r7);
    $finish;
  end
endmodule
"#;

fn values(dir: &Path, extra: &[&str]) -> String {
    let mut args = vec!["-s", "top", "tb.sv"];
    args.extend_from_slice(extra);
    let out = Command::new(xezim())
        .current_dir(dir)
        .args(&args)
        .env("XEZIM_INIT_REG", "random")
        .output()
        .expect("run xezim");
    let so = String::from_utf8_lossy(&out.stdout).into_owned();
    let line = so
        .lines()
        .find(|l| l.starts_with("T|"))
        .unwrap_or_else(|| {
            panic!(
                "no T| line:\n{}{}",
                so,
                String::from_utf8_lossy(&out.stderr)
            )
        })
        .to_string();
    assert!(
        !line.contains('x'),
        "registers must be initialised: {}",
        line
    );
    line
}

#[test]
fn random_register_init_follows_the_seed() {
    let d = scratch("seed");
    std::fs::write(d.join("tb.sv"), DESIGN).unwrap();
    let none = values(&d, &[]);
    let s1 = values(&d, &["+seed=1"]);
    let s2 = values(&d, &["+seed=2"]);
    let s2_again = values(&d, &["-sv_seed", "2"]);
    let s3 = values(&d, &["+seed=3"]);
    // No seed and the default seed 1 give the same pattern as before.
    assert_eq!(none, s1);
    // Another seed gives another pattern, reproducibly.
    assert_ne!(s1, s2);
    assert_ne!(s2, s3);
    assert_eq!(s2, s2_again);
    let _ = std::fs::remove_dir_all(&d);
}
