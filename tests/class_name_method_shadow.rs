// Regression test: a USER-DEFINED class method named `name()` must not be
// shadowed by the SystemVerilog enum-reflection `name()` built-in.
//
// §6.19.6 defines `name()` as a built-in on ENUMERATION types. The simulator
// intercepted ANY `obj.name()` call as an enum reflection, returning the enum
// member name (or empty when the receiver wasn't an enum). A user class
// defining `function string name()` thus silently returned empty — extremely
// common (UVM `get_type_name()` chains, user `name()` accessors).
//
// The fix: only apply the enum-reflection intercept when the receiver's
// declared type is NOT a class that defines its own `name()` method.
//
// This test exercises both the MemberAccess path (`obj.name()` where obj is a
// complex expression) and the flattened-Ident path (`Call{Ident([obj,name])}`,
// the common local-var case the parser produces).

use std::process::Command;

#[test]
fn class_name_method_not_shadowed_by_enum_builtin() {
    let src = r#"// An enum, to confirm its `name()` STILL works after the fix.
typedef enum {RED, GREEN, BLUE} color_t;

// A class with a user-defined `name()` method.
class Widget;
  string tag;
  function new(string t = "widget");
    tag = t;
  endfunction
  function string name();
    return tag;
  endfunction
  function string get_type_name();
    return "Widget";
  endfunction
endclass

module top;
  initial begin
    // Enum reflection must still work.
    color_t col;
    col = GREEN;
    if (col.name() != "GREEN")
      $display("FAIL enum-name got='%s'", col.name());

    // User class `name()` — flattened-Ident path (local var receiver).
    Widget w;
    w = new("gadget");
    if (w.name() == "gadget" && w.get_type_name() == "Widget")
      $display("PASS class-name");
    else
      $display("FAIL class-name got='%s' gtn='%s'", w.name(), w.get_type_name());

    // User class `name()` — also confirm `name` returning a literal works.
    Widget w2;
    w2 = new();
    if (w2.name() == "widget")
      $display("PASS class-name-default");
    else
      $display("FAIL class-name-default got='%s'", w2.name());
  end
endmodule
"#;

    let dir = std::env::temp_dir().join(format!("xezim_name_shadow_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sv_path = dir.join("name_shadow.sv");
    std::fs::write(&sv_path, src).unwrap();

    let bin = env!("CARGO_BIN_EXE_xezim");
    let out = Command::new(bin)
        .arg("--simulate")
        .arg("-s")
        .arg("top")
        .arg(sv_path.to_str().unwrap())
        .output()
        .expect("failed to run xezim");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("PASS class-name") && stdout.contains("PASS class-name-default"),
        "user-defined name() method was shadowed by the enum built-in.\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // Enum reflection must also still work (no FAIL enum-name line).
    assert!(
        !stdout.contains("FAIL"),
        "unexpected FAIL in output.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
