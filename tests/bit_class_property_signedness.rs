//! IEEE 1800-2023 §6.6.1 / §10.7: a `bit`-typed class property is UNSIGNED.
//! Assigning the integer literal `1` (a 32-bit signed value per §5.7.1) to a
//! `bit` field must store an UNSIGNED 1-bit `1`, which reads back as `1` —
//! NOT a signed-1-bit value that reads back as `-1` (0xFFFFFFFF).
//!
//! Root cause: `fit_class_prop` called `resize_for_assign(width)` which
//! preserves the RHS literal's `is_signed=true`. The stored `bit` then had
//! `is_signed=true`, so reading the set bit as a signed 1-bit value yielded
//! -1. This broke any cross-process `wait(<bit-member> == 1)` (e.g. UVM's
//! `uvm_resource::wait_modified` → `wait(modified==1)`) because the waiter
//! never saw `1`.
use std::process::Command;

fn xezim() -> String {
    let base = env!("CARGO_MANIFEST_DIR");
    format!("{}/target/debug/xezim", base)
}

fn run(src: &str, tag: &str) -> String {
    let path = format!("/tmp/bitfield_{tag}.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", &path])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn bit_class_property_is_unsigned() {
    let src = r#"module top;
  class R;
    bit b;
  endclass
  R r;
  initial begin
    r = new();
    r.b = 1;
    if (r.b === 1) $display("RESULT PASS"); else $display("RESULT FAIL b=%0d", r.b);
    $finish;
  end
endmodule
"#;
    let out = run(src, "unsigned");
    assert!(out.contains("RESULT PASS"), "expected bit field unsigned\n{out}");
}

#[test]
fn cross_process_wait_on_bit_member() {
    // The bug manifested as a cross-process `wait(modified==1)` that never
    // woke: the waiter read `modified` as -1 (signed), so `== 1` was false.
    let src = r#"module top;
  class R;
    bit modified;
    task setm(); modified = 1; endtask
    task waitm(); wait(modified == 1); endtask
  endclass
  R r;
  int woke;
  initial begin
    r = new();
    fork
      begin #1; r.setm(); end
      begin r.waitm(); woke = 1; end
    join
    if (woke == 1) $display("RESULT PASS"); else $display("RESULT FAIL woke=%0d", woke);
    $finish;
  end
endmodule
"#;
    let out = run(src, "xwait");
    assert!(out.contains("RESULT PASS"), "expected cross-process bit wait to wake\n{out}");
}
