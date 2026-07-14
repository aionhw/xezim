//! §8.25 / §8.26 — parameterized-class VALUE parameters.
//!
//! A class specialization's `#(...)` arguments were mapped onto the class's
//! VALUE parameters by slot in `param_defaults` (the value-params-only list)
//! while the argument list is positional over ALL parameters (type and value
//! interleaved, `param_order`). With `class Param #(type T, int W, int D)`,
//! `Param#(byte, 16)` bound `W <- byte` (an unevaluable type name -> x) and
//! `D <- 16`; every method then read the wrong values, while the defaults
//! were reported for the parameter the user actually overrode. The fix maps
//! arguments by NAME through `param_order` (§8.25), recovers the named form
//! `#(.W(32))` structurally (§8.26: a list is all-named or all-positional,
//! and the parser drops the names), and resolves `$bits(T)` through the
//! instance's type-parameter binding.

use xezim::simulate;

/// Positional specialization: value args land on the right parameters even
/// when a TYPE parameter occupies an earlier slot, unspecified parameters
/// keep their defaults, and an unspecialized declaration uses all defaults.
const POSITIONAL: &str = r#"
module tb;
  class Param #(type T = int, int W = 8, int D = 3);
    T val;
    function int width(); return W; endfunction
    function int wd(); return W + D; endfunction
    function int tbits(); return $bits(T); endfunction
  endclass
  int a, b, d, e, f;
  initial begin
    Param #(byte, 16) p16;
    Param #(shortint) pd;
    p16 = new();
    a = p16.width();   // W overridden positionally past the type slot
    b = p16.tbits();   // $bits(T) with T bound to byte
    pd = new();
    d = pd.wd();       // only T given: W and D keep their defaults
    f = pd.tbits();    // T bound to shortint
    p16.val = 8'h7F;   // T-typed property really is a byte
    e = p16.val;
  end
endmodule
"#;

/// Named specialization `#(.W(32), .D(5))`: the parser flattens named args
/// to a positional list (names dropped), so the simulator recovers the named
/// form structurally per §8.26 (all-named or all-positional) and pairs value
/// exprs with VALUE parameters in declaration order.
const NAMED: &str = r#"
module tb;
  class Param #(type T = int, int W = 8, int D = 3);
    function int wd(); return W + D; endfunction
    function int w_only(); return W; endfunction
  endclass
  int c, w;
  initial begin
    Param #(.W(32), .D(5)) p32;
    Param #(.W(64))        w64;
    p32 = new();
    c = p32.wd();      // 32 + 5
    w64 = new();
    // Known limitation (documented in class_param_arg_map): named args of
    // the SAME kind out of declaration order mis-bind, so only in-order
    // named lists are asserted here.
    w = w64.w_only() * 100 + w64.wd();  // 64*100 + (64+3)
  end
endmodule
"#;

/// Two different specializations of the same class coexist: each instance
/// reads ITS OWN specialization's binding (§8.25 — each combination of
/// arguments is a distinct specialization). Declarations at MODULE scope
/// exercise the elaboration-time `class_type_args` path; the interleaved
/// class (type param between value params) uses a user typedef as the type
/// arg inside an initial block.
const PER_SPEC: &str = r#"
module tb;
  typedef byte octet_t;
  class Cfg #(int N = 1, int M = 2);
    function int nm(); return N * 100 + M; endfunction
  endclass
  class Mix #(int A = 4, type T = int, int B = 6);
    function int ab(); return A * 100 + B + $bits(T); endfunction
  endclass
  int c1, c2, c3, m1;
  Cfg #(7, 9)  cfg_a;
  Cfg #(5)     cfg_b;
  Cfg          cfg_c;
  initial begin
    Mix #(2, octet_t, 3) mx;
    cfg_a = new();
    cfg_b = new();
    cfg_c = new();
    c1 = cfg_a.nm();  // 709
    c2 = cfg_b.nm();  // 502 (M default)
    c3 = cfg_c.nm();  // 102 (all defaults)
    mx = new();
    m1 = mx.ab();     // 2*100 + 3 + $bits(octet_t)=8 -> 211
  end
endmodule
"#;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

#[test]
fn positional_value_params_override_defaults() {
    let sim = simulate(POSITIONAL, 100).expect("simulate failed");
    assert_eq!(u(&sim, "a"), 16, "W must take the positional override, not its default");
    assert_eq!(u(&sim, "b"), 8, "$bits(T) must see the bound type (byte)");
    assert_eq!(u(&sim, "d"), 11, "unspecified value params keep their defaults (8+3)");
    assert_eq!(u(&sim, "f"), 16, "$bits(T) must see the bound type (shortint)");
    assert_eq!(u(&sim, "e"), 127, "a T-typed property must use the bound type");
}

#[test]
fn named_value_params_bind_by_kind() {
    let sim = simulate(NAMED, 100).expect("simulate failed");
    assert_eq!(u(&sim, "c"), 37, ".W(32) and .D(5) must land on W and D (32+5)");
    assert_eq!(u(&sim, "w"), 6467, ".W(64) must land on W with D default (64*100 + 64+3)");
}

#[test]
fn distinct_specializations_hold_distinct_bindings() {
    let sim = simulate(PER_SPEC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "c1"), 709, "Cfg#(7,9) reads its own N/M");
    assert_eq!(u(&sim, "c2"), 502, "Cfg#(5) reads N=5 with M default");
    assert_eq!(u(&sim, "c3"), 102, "unspecialized Cfg reads all defaults");
    assert_eq!(u(&sim, "m1"), 211, "value params around an interleaved type param bind by slot");
}
