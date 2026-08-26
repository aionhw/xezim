//! §5.7 / core issue #31: an unsized decimal literal >= 2^31 wraps to 32-bit
//! signed — deliberately (reference parity; the reference simulator sizes it
//! 32 signed, wraps identically, and warns) — and xezim NOW SAYS SO. The
//! core-side warning (xezim-core#32) covers declaration/parameter
//! initializers through elaboration const-eval; this pins the xezim-side
//! follow-up: a literal living only in a PROCEDURAL assignment is evaluated
//! by the simulator/bytecode literal paths and must warn there too, deduped
//! to once per literal string across every site. Sim-time warnings go to
//! stderr (the elaboration capture is drained before the run), so the
//! procedural case is asserted through the binary.

use std::io::Write as _;
use std::process::Command;

#[test]
fn procedural_unsized_decimal_literal_warns_once_and_wraps() {
    let dir = std::env::temp_dir();
    let sv_path = dir.join("xz_unsized_wrap_warn_test.sv");
    let mut f = std::fs::File::create(&sv_path).expect("temp sv");
    // The SAME literal appears twice procedurally: the warning must be
    // deduped to one line; the wrapped values are the reference's.
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
        stdout.contains("W_-1194966297.0_-2389932594.0"),
        "wrapped values drifted from the reference:\n{stdout}\n{stderr}"
    );
    let warns = stderr
        .matches("unsized decimal literal '3100000999'")
        .count();
    assert_eq!(
        warns, 1,
        "expected exactly one deduped wrap warning:\n{stderr}"
    );
    let _ = std::fs::remove_file(&sv_path);
}
