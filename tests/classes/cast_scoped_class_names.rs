//! §8.16 `$cast` against a class declared in a scoped unit.
//!
//! An object built from a class declared inside a `program`/`module`/package
//! or as a nested class ("`test.reg_rw`", "`UVC::transaction`") records its
//! `class_name` under a DIFFERENT spelling than a `$cast` destination type
//! resolves to (the bare "`reg_rw`" / "`transaction`"). The `$cast` hierarchy
//! check compared names verbatim against the bare-keyed class registry, so a
//! legal downcast of a scoped-subclass spuriously failed (UVM fatal
//! `SQRSNDREQCAST` in `uvm_sequencer_param_base::send_request`).
//!
//! Two genuinely different same-named classes in different scopes must stay
//! DISTINCT: `$cast`ing a `pkg1::Foo` object to a `pkg2::Foo` handle has to
//! fail regardless of the shared leaf name. Expected lines come from the
//! reference simulator.

const PFX: &str = "csc";
use std::process::Command;

/// Run `src` with top `top` and return its `T|` lines.
fn run(tag: &str, src: &str, top: &str) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!("xezim_{}_{}_{}", PFX, tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join("t.sv");
    std::fs::write(&sv, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--simulate")
        .arg("-s")
        .arg(top)
        .arg(sv.to_str().unwrap())
        .output()
        .expect("failed to run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("T|"))
        .map(|l| l.trim_end().to_string())
        .collect()
}

fn check(tag: &str, src: &str, top: &str, expected: &[&str]) {
    let got = run(tag, src, top);
    assert_eq!(
        got,
        expected.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        "T| lines differ from the reference"
    );
}

/// A class declared inside a `program` (scoped `test.reg_rw`) cast down from
/// and back to the bare spelling, exercising virtual dispatch through a base
/// handle to a derived object of a scoped class.
#[test]
fn program_scoped_downcast_and_virtual() {
    const SRC: &str = r#"
program test;
  class base_c;
    virtual function string who(); return "base_c"; endfunction
  endclass
  class reg_rw extends base_c;
    int f = 42;
    virtual function string who(); return "reg_rw"; endfunction
  endclass
  reg_rw obj = new;
  initial begin
    string k;
    automatic base_c base_ref;
    automatic reg_rw obj2;
    base_ref = obj;
    k = base_ref.who();
    $display("T| virt=%0s", k);
    if ($cast(obj2, base_ref)) begin
      $display("T| cast=1 f=%0d", obj2.f);
    end else begin
      $display("T| cast=0");
    end
  end
endprogram
"#;
    check(
        "prog",
        SRC,
        "test",
        &["T| virt=reg_rw", "T| cast=1 f=42"],
    );
}

/// A nested class inside a parameterized outer class (spelling
/// `UVC::transaction`), cast through a base handle with virtual dispatch.
#[test]
fn nested_class_downcast_and_virtual() {
    const SRC: &str = r#"
module top;
  class UVC#(type T=uvm_object_obj);
    class item;
      int value;
      function new(int v); value = v; endfunction
      virtual function string kind(); return "item"; endfunction
      class sub_item extends item;
        function new(int v); super.new(v); endfunction
        virtual function string kind(); return "sub_item"; endfunction
      endclass
    endclass
  endclass
  function automatic void run();
    automatic UVC#(uvm_object_obj)::item::sub_item si;
    automatic UVC#(uvm_object_obj)::item::sub_item si2;
    automatic UVC#(uvm_object_obj)::item base_ref;
    string k;
    si = new(20);
    base_ref = si;
    k = base_ref.kind();
    $display("T| vabase=%0s", k);
    if ($cast(si2, base_ref)) begin
      $display("T| cast=1 %0s", si2.kind());
    end else begin
      $display("T| cast=0");
    end
  endfunction
  initial begin run(); end
endmodule
"#;
    check(
        "nested",
        SRC,
        "top",
        &["T| vabase=sub_item", "T| cast=1 sub_item"],
    );
}

/// Two distinct same-named classes in different packages must NOT conflate:
/// casting a `pkg1::Foo` object to a `pkg2::Foo` handle fails even though
/// both share the leaf name `Foo`.
#[test]
fn same_named_classes_in_distinct_scopes_stay_distinct() {
    const SRC: &str = r#"
package pkg1;
  class Foo;
    int mark = 1;
  endclass
endpackage
package pkg2;
  class Foo;
    int mark = 2;
  endclass
endpackage
module top;
  function automatic void run();
    automatic pkg1::Foo h1;
    automatic pkg1::Foo cast_p1;
    automatic pkg2::Foo cast_p2;
    h1 = new;
    // own-scope cast is legal
    if ($cast(cast_p1, h1)) $display("T| own=1"); else $display("T| own=0");
    // cross-scope cast must FAIL (distinct classes)
    if ($cast(cast_p2, h1)) $display("T| cross=1"); else $display("T| cross=0");
  endfunction
  initial begin run(); end
endmodule
"#;
    check("distinct", SRC, "top", &["T| own=1", "T| cross=0"]);
}