// Regression test for GAP-A: a STATIC class property of a class-local
// typedef type (the UVM factory singleton pattern) must construct the
// enclosing class on `new()`.
//
// Pattern (every uvm_*_registry uses this):
//   class Registry #(type T=int, string N="x");
//     typedef Registry#(T,N) this_type;
//     static this_type m_inst;                 // static class property
//     static function this_type get();
//       if (m_inst == null) m_inst = new();    // must construct a Registry
//       return m_inst;
//     endfunction
//   endclass
//
// Before the fix, class_prop_type_named returned None for a property whose
// recorded type_name was a typedef (like `this_type`) rather than a known
// class/enum/type-param, so `m_inst = new()` could not determine the class
// and constructed nothing useful. Now class_prop_type_named resolves the
// typedef via the context-aware resolve_typeref_class_name.

use std::process::Command;

#[test]
fn static_typedef_class_property_singleton() {
    let src = r#"// Two parameterized classes, each with the singleton pattern, to also
// confirm the typedef collision fix (this_type resolves per-enclosing-class).
class Alpha #(type T = int);
  typedef Alpha#(T) this_type;
  static this_type m_inst;
  static function this_type get();
    if (m_inst == null) m_inst = new();
    return m_inst;
  endfunction
  virtual function string kind();
    return "alpha";
  endfunction
endclass

class Beta #(type T = int);
  typedef Beta#(T) this_type;
  static this_type m_inst;
  static function this_type get();
    if (m_inst == null) m_inst = new();
    return m_inst;
  endfunction
  virtual function string kind();
    return "beta";
  endfunction
endclass

module top;
  initial begin
    Alpha#(int) a;
    Beta#(int)  b;
    a = Alpha#(int)::get();
    b = Beta#(int)::get();
    // Each singleton must construct its OWN class (this_type resolves to the
    // enclosing class), and the static property persists across the two calls.
    if (a != null && b != null && a.kind() == "alpha" && b.kind() == "beta")
      $display("PASS singleton");
    else
      $display("FAIL singleton a=%s b=%s", a==null?"null":a.kind(), b==null?"null":b.kind());
  end
endmodule
"#;

    let dir = std::env::temp_dir().join(format!("xezim_gap_a_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sv_path = dir.join("static_typedef_singleton.sv");
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
        stdout.contains("PASS singleton"),
        "static typedef class-property singleton failed.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
