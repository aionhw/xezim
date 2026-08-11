// Regression test: a class-typed procedural local must NOT leak into the
// caller's class type resolution when it shares a bare name with a class
// property or a caller-frame local.
//
// Root cause (fixed): `var_class_types` was a FLAT global map (never cleared
// on scope/method exit) that mapped bare variable name -> declared class type.
// When an unrelated method (e.g. UVM's `start_phase_sequence`, or this test's
// `worker::helper`) declared a local under the same bare name as a user's
// class property (e.g. `my_sequence seq;` / `derived_obj obj;`), its entry
// overwrote the user's, so a null-handle static-method dispatch
// (`seq.get_type()` / `obj.who()`) resolved against the WRONG registered
// class. In UVM 07start_item_seq this surfaced as
// `[CREATE_ABSTRACT_OBJ] uvm_sequence_base` + `[NULLITM]` (the test should
// instead raise `[SEQNOTITM]` and pass).
//
// The fix scopes these local-class-type maps into per-call frames
// (`var_class_types_frames` / `var_type_args_frames`) pushed/popped in sync
// with the existing queue/dyn-array call frames, so a subroutine's
// class-typed locals drop when it returns.
//
// Verified byte-for-byte against the reference simulator (this drives the
// same name/shadowing path; `u.which()` is called on a NULL `obj` handle so
// the *declared* type — not the runtime handle — selects the static method).

use std::process::Command;

fn xezim() -> String {
    env!("CARGO_BIN_EXE_xezim").to_string()
}

fn run(src: &str, tag: &str) -> String {
    let path = format!("/tmp/varclscoll_{tag}.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", &path])
        .output()
        .expect("xezim failed to start");
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn local_class_type_does_not_shadow_property_across_frames() {
    let out = run(
        r#"module top;

  class base_obj;
    static function string who();
      return "base_obj";
    endfunction
  endclass

  class derived_obj extends base_obj;
    static function string who();
      return "derived_obj";
    endfunction
  endclass

  // A worker whose method-local `obj` (declared in a different frame) must
  // not clobber the user's `obj` class property type.
  class worker;
    static function void helper();
      base_obj obj;
      obj = new();
    endfunction
  endclass

  class user;
    derived_obj obj;
    function string which();
      return obj.who();
    endfunction
  endclass

  initial begin
    user u;
    string res;
    // Register worker's local `base_obj obj` into the (old) flat map first.
    worker::helper();
    // Now the user's property `derived_obj obj`. Deliberately do NOT allocate
    // it: receiver's declared type must drive the static dispatch (cf. UVM's
    // `uvm_create(seq).get_type()`, where the handle is still null).
    u = new();
    res = u.which();
    if (res == "derived_obj") $display("TAG_PASS");
    else $display("TAG_FAIL which=%0s", res);
  end
endmodule
"#,
        "static_dispatch",
    );
    assert!(
        out.contains("TAG_PASS"),
        "stale method-local class type shadowed the declared property type\n{}",
        out
    );
}