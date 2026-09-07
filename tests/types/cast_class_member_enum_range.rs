//! §6.24.1 — `$cast` to an ENUM succeeds only when the source value is one of
//! the enum's members; an out-of-range integer must FAIL (return 0).
//!
//! The range check lived in `cast_type_ok` but resolved the destination's type
//! through `type_name_of_var`, which only knows module signals/locals and
//! procedural-local class types. A class-MEMBER enum variable (e.g.
//! `uvm_verbosity l_verbosity;` inside a `uvm_report_handler` method) is in
//! none of those, so `$cast` to it fell through to the permissive class-handle
//! escape hatch and returned 1 (success) for EVERY integer — 301 (`int'($cast)`
//! intended to print) became an empty-named `uvm_verbosity` instead of the
//! `int 301` the reference prints. This surfaced as a mis-typed row in a
//! UVM report-handler table.
//!
//! A top-of-module reproducer passed (module locals are tracked), which is why
//! it hid behind the class boundary. Verified byte-identical to a reference
//! simulator on the four probes below.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// `$cast` to a class-member enum must range-check the source: 301 and 501
/// (not members of the 0,100,200,300,400,500 verbosity enum) must FAIL, while
/// valid members must pass.
#[test]
fn class_member_enum_cast_range_checks() {
    let src = r#"
module tb;
  typedef enum { U_NONE=0, U_LOW=100, U_MEDIUM=200, U_HIGH=300, U_FULL=400, U_DEBUG=500 } verbosity;
  int rc_301, rc_501, rc_400, rc_100;
  class handler;
    verbosity l_verbosity;
    function automatic int check(int v);
      verbosity lv;
      int rc;
      rc = $cast(l_verbosity, v);
      if (rc == 0) begin
        // out-of-range -> int path (as uvm_report_handler does for a non-member)
        lv = v; // not reached for 301/501 after the fix
        return 0;
      end
      return 1;
    endfunction
    function automatic int check_ret(int v);
      return $cast(l_verbosity, v);
    endfunction
  endclass
  handler h;
  initial begin
    h = new();
    rc_301 = h.check_ret(301);
    rc_501 = h.check_ret(501);
    rc_400 = h.check_ret(400);
    rc_100 = h.check_ret(100);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "rc_301"), 0, "301 not a verbosity member -> cast FAILS");
    assert_eq!(u(&sim, "rc_501"), 0, "501 (past U_DEBUG=500) -> cast FAILS");
    assert_eq!(u(&sim, "rc_400"), 1, "U_FULL=400 is a member -> cast succeeds");
    assert_eq!(u(&sim, "rc_100"), 1, "U_LOW=100 is a member -> cast succeeds");
}

/// Sanity control: the same out-of-range `$cast` on a module-scope enum var is
/// (and always was) correctly range-checked — guards against over-suppressing.
#[test]
fn module_scope_enum_cast_still_range_checks() {
    let src = r#"
module tb;
  typedef enum { A=10, B=20, C=30 } te;
  te v;
  int rc_25, rc_20;
  initial begin
    rc_25 = $cast(v, 25);
    rc_20 = $cast(v, 20);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "rc_25"), 0, "25 not a member -> FAIL");
    assert_eq!(u(&sim, "rc_20"), 1, "20=B is a member -> success");
}