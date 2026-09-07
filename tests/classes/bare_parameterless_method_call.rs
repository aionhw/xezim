//! LRM §13.4.1: a PARAMETERLESS class method referenced by NAME ALONE (no
//! parentheses) — `f;`, `obj.f;`, `this.f` — is a valid call and returns the
//! function's value.
//!
//! The no-parentheses INSTANCE-method form inside a class method (`count`,
//! `get_action`) previously resolved to an unknown `x` in xezim: only the
//! explicit-handle shapes (`c.f`, `this.f`) and static `Class::f` were
//! dispatched. A bare inherited/own parameterless method name therefore
//! read as x, breaking UVM report-catcher action checks transcribed as
//! `if (get_action != UVM_DISPLAY|UVM_COUNT)`.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
}

/// A bare parameterless method name used in an arithmetic comparison must be
/// invoked (returning its value), not read as an unknown.
#[test]
fn bare_parameterless_instance_method_returns_value() {
    const SRC: &str = "class base;
  int v;
  function new(); v = 5; endfunction
  function int get_action(); return v; endfunction
endclass

class ext extends base;
  function new(); super.new(); endfunction
  function int check();
    int a1;
    a1 = get_action;             // own/inherited, NO parens
    if (get_action == 5 && a1 == 5)
      return 1;
    else
      return 0;
  endfunction
endclass

module tb;
  int result;
  initial begin
    ext e = new;
    result = e.check();
  end
endmodule
";
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "result"), 1, "bare parameterless method name did not return its value");
}

/// A bare parameterless method name in an `!=` guard against an enum bitwise
/// OR must take the not-equal branch only when the values actually differ.
/// (The UVM report-catcher shape that motivated this fix.)
#[test]
fn bare_method_in_ne_enum_comparison() {
    const SRC: &str = "class base;
  typedef enum { NA=0, DISP=1, CNT=4, EXIT=8 } action_type;
  int v;
  function new(); v = DISP | CNT; endfunction
  function int get_action(); return v; endfunction
  function int check_good();
    // identical values -> not-equality is false -> return 1
    if (get_action != (DISP | CNT)) return 0;
    return 1;
  endfunction
  function int check_bad();
    // differing values -> not-equality is true -> return 0
    if (get_action != (DISP | EXIT)) return 0;
    return 1;
  endfunction
endclass

module tb;
  int good, bad;
  initial begin
    base b = new;
    good = b.check_good();
    bad  = b.check_bad();
  end
endmodule
";
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "good"), 1, "bare method compared != equal enum-OR value");
    assert_eq!(u(&sim, "bad"), 0, "bare method compared != different enum-OR value");
}