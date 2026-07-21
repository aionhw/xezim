// Regression test for function-local `static` per-specialization keying
// (IEEE 1800-2023 §8.25 / §6.21).
//
// A `static` variable declared inside a METHOD of a parameterized class keeps
// ONE persistent storage per SPECIALIZATION. The UVM factory's real singleton
// form is exactly this:
//
//   class Registry #(type T=int, string N="x");
//     typedef Registry#(T,N) this_type;
//     static function this_type get();
//       static this_type m_inst;          // function-local static
//       if (m_inst == null) m_inst = new();
//       return m_inst;
//     endfunction
//   endclass
//
// `m_inst` must be:
//   - SHARED across repeated calls of the SAME specialization (singleton),
//   - DISTINCT across DIFFERENT specializations.
//
// Before the fix, class methods never opened a `static_local_syncs` frame
// (only free functions/tasks did), so a function-local static was
// re-initialized on every call and the singleton never persisted. Verified
// byte-for-byte against reference simulators.

use std::process::Command;

#[test]
fn function_local_static_shared_within_spec_distinct_across_specs() {
    let src = r#"class Wrapper #(type T=int, string Tname="<unknown>");
  typedef Wrapper#(T, Tname) this_type;
  static function this_type get();
    static this_type m_inst;          // function-local static
    static int counter;
    if (m_inst == null) begin
      m_inst = new();
      counter = counter + 1;          // counts constructions
    end
    return m_inst;
  endfunction
  virtual function string name();
    return Tname;
  endfunction
endclass

module top;
  initial begin
    Wrapper#(int, "AAA") a1, a2;
    Wrapper#(int, "BBB") b1;
    a1 = Wrapper#(int, "AAA")::get();
    a2 = Wrapper#(int, "AAA")::get();   // same spec -> same singleton
    b1 = Wrapper#(int, "BBB")::get();   // diff spec -> diff singleton
    if (a1 == a2) $display("PASS share");
    else          $display("FAIL share a1!=a2");
    if (a1 != b1 && a1.name() == "AAA" && b1.name() == "BBB")
      $display("PASS distinct");
    else
      $display("FAIL distinct a1=%s b1=%s",
               a1==null?"null":a1.name(), b1==null?"null":b1.name());
  end
endmodule
"#;

    let dir = std::env::temp_dir().join(format!("xezim_flstatic_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sv_path = dir.join("static_local_per_spec.sv");
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
        stdout.contains("PASS share") && stdout.contains("PASS distinct")
            && !stdout.contains("FAIL"),
        "function-local static per-spec keying failed.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
