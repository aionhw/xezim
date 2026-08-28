//! A class `time`-typed field is stored per-object (not as a module-scope
//! signal), so `lrm_self_width`'s `lookup_signal_width` misses it. A
//! comparison against a UNSIZED literal like `begin_time != -1` then failed to
//! size the literal to the field's 64-bit width: `self_det_w` fell back to
//! `ctx_width` (0), the `-1` stayed 32-bit (`32'hFFFFFFFF`), and `is_equal`
//! compared 64-bit all-ones (the `-1` initialiser) against 32'hFFFFFFFF as
//! NOT equal.
//!
//! UVM's `uvm_transaction` declares `local time begin_time = -1; end_time =
//! -1; accept_time = -1;` and its `do_print` prints them only when
//! `!= -1`. That check evaluating TRUE for a `-1` value added three bogus
//! `time` rows to the printer output, which the table-printer diff flagged as
//! a fatal (403x printer).
use std::process::Command;

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

fn run(src: &str, tag: &str) -> String {
    let path = format!("/tmp/time_field_{tag}.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", &path])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A module-level `time` (a module-scope signal) and a class `time` field must
/// BOTH compare equal to `-1` after being initialised to `-1`, and both must
/// stay unsigned (`>= 0` true for the all-ones value) — matching the
/// reference. The class field is the regression case.
const TIME_FIELD_NEG_ONE: &str = r#"module top;
  time mod_t = -1;
  class base;
    time cls_t = -1;
    function void check();
      if (cls_t == -1) $display("RESULT PASS cls_neg1");
      else             $display("RESULT FAIL cls_neq_literal_neg1");
      if (cls_t >= 0)  $display("RESULT PASS cls_unsigned");
      else             $display("RESULT FAIL cls_signed");
    endfunction
  endclass
  initial begin
    base b = new;
    if (mod_t == -1) $display("RESULT PASS mod_neg1");
    else             $display("RESULT FAIL mod_neq_1");
    b.check();
    #1; $finish;
  end
endmodule
"#;

#[test]
fn class_time_field_compares_equal_to_unsized_minus_one() {
    let out = run(TIME_FIELD_NEG_ONE, "neg1");
    assert!(
        out.contains("RESULT PASS cls_neg1"),
        "a class `time` field initialised to -1 must compare == -1 (the literal\n\
         must be sized to the field's 64-bit width, not left as 32-bit):\n{out}"
    );
    assert!(
        out.contains("RESULT PASS cls_unsigned"),
        "`time` must stay unsigned (all-ones is a large positive, >= 0):\n{out}"
    );
    assert!(
        out.contains("RESULT PASS mod_neg1"),
        "module-level time -1 still must == -1:\n{out}"
    );
}