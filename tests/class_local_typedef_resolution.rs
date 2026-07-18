// Regression test for resolve_typeref_class_name: a bare class-local typedef
// referenced inside a method must resolve to the ENCLOSING class's typedef,
// not to a same-named typedef in an unrelated class.
//
// Before the fix, the module-level `typedef_types` table (where class-local
// typedefs leak, last-processed wins) was consulted before the enclosing
// class context, so a typedef reference inside one class could resolve to a
// different class's typedef of the same name.
//
// This test isolates the typedef-resolution path (via a separate `obj = new()`
// assignment that consults var_class_types, which is populated by
// resolve_typeref_class_name) WITHOUT depending on the still-unfixed static-
// function-local type recording (GAP-A).

use std::process::Command;

#[test]
fn class_local_typedef_resolves_to_enclosing_class() {
    // Two parameterized classes each declare `typedef <Self>#(T) handle_t;`.
    // Inside a NON-static method, a `handle_t local = new()` must construct
    // the enclosing class, not the other one. This exercises
    // resolve_typeref_class_name through the VarDecl -> var_class_types path.
    let src = r#"class Alpha #(type T = int);
  typedef Alpha#(T) handle_t;
  function handle_t clone();
    handle_t c;
    c = new();       // must construct an Alpha (enclosing class)
    return c;
  endfunction
  virtual function string kind();
    return "alpha";
  endfunction
endclass

class Beta #(type T = int);
  typedef Beta#(T) handle_t;
  function handle_t clone();
    handle_t c;
    c = new();       // must construct a Beta (enclosing class)
    return c;
  endfunction
  virtual function string kind();
    return "beta";
  endfunction
endclass

module top;
  initial begin
    Alpha#(int) a;
    Beta#(int)  b;
    a = new();
    b = new();
    Alpha#(int) ac;
    Beta#(int)  bc;
    ac = a.clone();
    bc = b.clone();
    if (ac.kind() == "alpha" && bc.kind() == "beta")
      $display("THIS_TYPE_PASS");
    else
      $display("THIS_TYPE_FAIL ac=%s bc=%s", ac.kind(), bc.kind());
  end
endmodule
"#;

    let dir = std::env::temp_dir().join(format!("xezim_typedef_enc_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sv_path = dir.join("typedef_enclosing.sv");
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
        stdout.contains("THIS_TYPE_PASS"),
        "class-local typedef did not resolve to the enclosing class \
         (expected THIS_TYPE_PASS).\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
