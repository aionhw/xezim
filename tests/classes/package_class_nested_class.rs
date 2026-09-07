//! IEEE 1800-2023 §8.23: nested classes.
//! A class declared inside another class declared in a package or module
//! must be accessible via both its scoped name `Outer::Inner` and unqualified
//! within the enclosing class scope.
//!
//! REGRESSION: during elaboration, `register_nested_classes` was called only
//! for root-level `Definition::Class`, skipping classes declared inside packages
//! and modules. When a nested class was instantiated, `elab.classes` did not
//! contain it, causing construction to fail or fall back to the enclosing class.
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
    let path = format!("/tmp/nested_pkg_{tag}.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", &path])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const SV_SRC: &str = r#"package p;
  class outer;
    class inner;
      int val = 123;
    endclass
    function inner make();
      inner i = new;
      return i;
    endfunction
  endclass
endpackage

module top;
  import p::*;
  initial begin
    outer o = new;
    outer::inner i = o.make();
    if (i.val == 123) $display("PASS val=%0d", i.val);
    else $display("FAIL");
    $finish;
  end
endmodule
"#;

#[test]
fn package_class_nested_class_instantiation() {
    let out = run(SV_SRC, "test");
    assert!(
        out.contains("PASS val=123"),
        "expected nested class inside package class to instantiate properly, got:\n{out}"
    );
}
