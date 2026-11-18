//! IEEE 1800-2023 §5.7.1 (#1120): an unsized decimal literal of 2^31 or more
//! is wider than 32 bits, sign bit included, so a literal that lives only in
//! procedural code keeps its value — in a real assignment and in a real
//! product alike — and no wrap warning is printed for it any more. (Until
//! #1120 the literal wrapped to 32 signed bits, with a warning.)

use std::io::Write as _;
use std::process::Command;

#[test]
fn procedural_unsized_decimal_preserves_value_without_wrap_warning() {
    let dir = std::env::temp_dir();
    let sv_path = dir.join("xz_unsized_wrap_warn_test.sv");
    let mut f = std::fs::File::create(&sv_path).expect("temp sv");
    // The SAME literal appears twice procedurally; neither site wraps.
    write!(
        f,
        r#"
module top;
  real x, z;
  initial begin
    x = 3100000999;
    z = 2.0 * 3100000999;
    $display("W_%.1f_%.1f", x, z);
  end
endmodule
"#
    )
    .unwrap();
    drop(f);
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--simulate")
        .arg("-s")
        .arg("top")
        .arg(sv_path.to_str().unwrap())
        .output()
        .expect("failed to run xezim");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("W_3100000999.0_6200001998.0"),
        "large decimal values were truncated:\n{stdout}\n{stderr}"
    );
    let warns = stderr
        .matches("unsized decimal literal '3100000999'")
        .count();
    assert_eq!(warns, 0, "unexpected wrap warning:\n{stderr}");
    let _ = std::fs::remove_file(&sv_path);
}
