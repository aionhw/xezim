// Regression tests for three interrelated bugs that collectively broke
// UVM's printer output (`uvm_printer::emit` produced no column padding,
// `$fwrite` dumped a garbage integer instead of text, and
// `uvm_pkg::Class::method()` returned null).
//
// Bug 1: Static class property method calls (.len/.substr/.getc on a
//        `local static string`) read the property as empty because
//        `eval_builtin_method` used `get_local_or_signal`, which skips
//        `class_statics`.
// Bug 2: `expr_is_string_valued` didn't handle `ExprKind::Call`, so
//        `$write`/`$fwrite` with a string-returning method call argument
//        (e.g. `obj.sprint()`) printed the packed bits as a decimal number.
// Bug 3: Package-qualified static method calls `pkg::Class::method()`
//        returned null because the nested MemberAccess receiver was
//        treated as an instance property dereference, not a class scope.

use std::process::Command;

fn run_xezim(src: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_printer_fix_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sv_path = dir.join(format!("{tag}.sv"));
    std::fs::write(&sv_path, src).unwrap();
    let bin = env!("CARGO_BIN_EXE_xezim");
    let out = Command::new(bin)
        .arg("--simulate")
        .arg("-s")
        .arg("top")
        .arg(sv_path.to_str().unwrap())
        .output()
        .expect("failed to run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// ============================================================================
// Bug 1: Static class property method calls
// ============================================================================

#[test]
fn static_string_len_works() {
    let src = r#"
class tprinter;
  local static string m_space;
  static string dash;
  function void build();
    dash = {10{"-"}};
    m_space = {10{" "}};
  endfunction
  function int get_space_len();
    return m_space.len();
  endfunction
endclass
module top;
  initial begin
    tprinter t;
    t = new();
    t.build();
    $display("RESULT %0d", t.get_space_len());
    $finish;
  end
endmodule
"#;
    let out = run_xezim(src, "static_len");
    assert!(out.contains("RESULT 10"), "expected RESULT 10, got:\n{}", out);
}

#[test]
fn static_string_substr_works() {
    let src = r#"
class tprinter;
  local static string m_space;
  function void setup();
    m_space = {10{" "}};
  endfunction
  function string pad();
    return m_space.substr(1, 3);
  endfunction
endclass
module top;
  initial begin
    tprinter t;
    t = new();
    t.setup();
    $display("PADLEN %0d", t.pad().len());
    $finish;
  end
endmodule
"#;
    let out = run_xezim(src, "static_substr");
    assert!(out.contains("PADLEN 3"), "expected PADLEN 3, got:\n{}", out);
}

// ============================================================================
// Bug 2: String-returning method call in $write/$fwrite
// ============================================================================

#[test]
fn fwrite_with_string_returning_call() {
    let src = r#"
class box;
  string content = "Hello World";
  function string get_str();
    return content;
  endfunction
endclass
module top;
  initial begin
    box b;
    b = new();
    $write(b.get_str());
    $display(" END");
    $display("DONE");
    $finish;
  end
endmodule
"#;
    let out = run_xezim(src, "fwrite_str");
    // Before the fix, $write printed a garbage decimal number.
    assert!(out.contains("Hello World END"), "expected 'Hello World END', got:\n{}", out);
    assert!(out.contains("DONE"), "got:\n{}", out);
}

#[test]
fn write_with_bare_string_call_in_class_method() {
    let src = r#"
class printer;
  string text = "Formatted-Output";
  function void emit();
    $write(textify());
    $display(" <<<");
  endfunction
  function string textify();
    return text;
  endfunction
endclass
module top;
  initial begin
    printer p;
    p = new();
    p.emit();
    $finish;
  end
endmodule
"#;
    let out = run_xezim(src, "write_str");
    assert!(out.contains("Formatted-Output <<<"), "got:\n{}", out);
}

// ============================================================================
// Bug 3: Package-qualified static method call
// ============================================================================

#[test]
fn pkg_qualified_static_call() {
    let src = r#"
package my_pkg;
  class counter;
    static int count = 0;
    static function int get_count();
      return count;
    endfunction
    static function void inc();
      count = count + 1;
    endfunction
  endclass
endpackage
module top;
  initial begin
    my_pkg::counter::inc();
    my_pkg::counter::inc();
    my_pkg::counter::inc();
    $display("COUNT %0d", my_pkg::counter::get_count());
    $finish;
  end
endmodule
"#;
    let out = run_xezim(src, "pkg_static");
    assert!(out.contains("COUNT 3"), "expected COUNT 3, got:\n{}", out);
}

#[test]
fn pkg_qualified_static_factory_create() {
    let src = r#"
package factory_pkg;
  class product;
  endclass
  class maker;
    static function product create();
      product p;
      p = new("widget");
      return p;
    endfunction
  endclass
endpackage
module top;
  initial begin
    // This is the form UVM macros use: fully package-qualified
    factory_pkg::product h;
    h = factory_pkg::maker::create();
    if (h != null)
      $display("CREATED");
    else
      $display("NULL");
    $finish;
  end
endmodule
"#;
    let out = run_xezim(src, "pkg_factory");
    assert!(out.contains("CREATED"), "expected CREATED, got:\n{}", out);
}
