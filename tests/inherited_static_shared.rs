// Regression test for `static_prop_key` over-aggressive per-specialization
// keying of inherited statics (IEEE 1800-2023 §8.9 / §8.25).
//
// A `static` property inherited from a NON-parameterized ancestor is a single
// shared cell across EVERY specialization of a parameterized derived class —
// the declaring class is not parameterized, so there is nothing to specialize.
//
//   class Base;                         // non-parameterized ancestor
//     static int counter = 0;
//   endclass
//   class Derived #(type T=int) extends Base;
//     ... counter = counter + 1; ...    // writes the shared Base::counter
//   endclass
//
// Before the fix, `static_prop_key` applied per-spec keying whenever the spec
// base class EXTENDED the declaring class (`class_extends`), regardless of
// whether the declaring class was itself parameterized. That wrongly split
// `Base::counter` into one cell per `Derived#(...)` specialization instead of
// the single shared cell the LRM requires. Verified byte-for-byte against
// reference simulators.

use std::process::Command;

#[test]
fn inherited_static_from_nonparam_ancestor_is_shared_across_specs() {
    let src = r#"class Base;                 // non-parameterized ancestor
  static int counter = 0;
endclass

class Derived #(type T=int) extends Base;
  typedef Derived#(T) this_type;
  static this_type m_inst;
  static function this_type get();
    counter = counter + 1;          // writes the shared Base::counter
    if (m_inst == null) m_inst = new();
    return m_inst;
  endfunction
endclass

module top;
  initial begin
    Derived#(int)  a;
    Derived#(byte) b;
    a = Derived#(int)::get();       // counter -> 1
    b = Derived#(byte)::get();      // counter -> 2 (shared)
    b = Derived#(byte)::get();      // counter -> 3 (shared)
    if (Base::counter == 3) $display("PASS shared-inherited");
    else $display("FAIL shared-inherited got=%0d", Base::counter);
  end
endmodule
"#;

    let dir = std::env::temp_dir().join(format!("xezim_inhstatic_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sv_path = dir.join("inherited_static_shared.sv");
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
        stdout.contains("PASS shared-inherited") && !stdout.contains("FAIL"),
        "inherited static was not shared across specializations.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

// Positive control: an inherited static from a PARAMETERIZED ancestor IS
// per-specialization (this is the UVM factory `m__initialized` pattern that
// the broader keying was originally added to support, and must keep working).
#[test]
fn inherited_static_from_param_ancestor_is_per_spec() {
    let src = r#"class Base #(type T=int);
  static int counter = 0;
  static function int get_count();
    return counter;
  endfunction
endclass

class Derived #(type T=int) extends Base#(T);
  typedef Derived#(T) this_type;
  static this_type m_inst;
  static function this_type get();
    counter = counter + 1;
    if (m_inst == null) m_inst = new();
    return m_inst;
  endfunction
endclass

module top;
  initial begin
    Derived#(int)  a;
    Derived#(byte) b;
    a = Derived#(int)::get();       // int counter  -> 1
    a = Derived#(int)::get();       // int counter  -> 2
    b = Derived#(byte)::get();      // byte counter -> 1 (distinct cell)
    if (Derived#(int)::get_count() == 2)  $display("PASS per-spec-int");
    else $display("FAIL per-spec-int got=%0d", Derived#(int)::get_count());
    if (Derived#(byte)::get_count() == 1) $display("PASS per-spec-byte");
    else $display("FAIL per-spec-byte got=%0d", Derived#(byte)::get_count());
  end
endmodule
"#;

    let dir = std::env::temp_dir().join(format!("xezim_inhstatic2_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sv_path = dir.join("inherited_static_per_spec.sv");
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
        stdout.contains("PASS per-spec-int") && stdout.contains("PASS per-spec-byte")
            && !stdout.contains("FAIL"),
        "inherited static from a parameterized ancestor was not per-spec.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
