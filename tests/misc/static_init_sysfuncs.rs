//! Issue #26: module-scope static variable initializers that call
//! simulation-time system functions (IEEE 1800-2017 §6.21 / §10.5).
//!
//! A static variable's initializer is evaluated once at simulation start,
//! as if assigned from an `initial` block — so it may legally call system
//! functions such as $sqrt, $sformatf("%m"), $urandom_range or
//! $test$plusargs. Elaboration classified any system call with constant
//! arguments as a constant expression, but its const-eval only implements
//! the §13.4.3 elaboration constants ($clog2, $bits, ...); everything else
//! silently folded to 0/"". These initializers are now re-issued as time-0
//! static-init assignments that run before any user `initial` block.

use xezim::compiler::Simulator;
use xezim::simulate;

fn get<'a>(sim: &'a Simulator, name: &str) -> &'a xezim_core::value::Value {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("tb.{}", name)))
        .unwrap_or_else(|| panic!("signal {} not found", name))
}

/// $sqrt / $urandom_range in a static initializer must be evaluated at
/// simulation start, not const-folded to 0 at elaboration. $clog2 is an
/// elaboration constant (§13.4.3) and must keep folding.
#[test]
fn static_init_math_and_random_sysfuncs() {
    let src = r#"
module tb;
  static int  c    = $clog2(32);
  static real s    = $sqrt(16.0);
  static int  rnd  = $urandom_range(100, 200);
  initial #1 $finish;
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(get(&sim, "c").to_u64(), Some(5), "$clog2(32) must stay 5");
    assert_eq!(get(&sim, "s").to_f64(), 4.0, "$sqrt(16.0) must be 4.0");
    let rnd = get(&sim, "rnd").to_u64().expect("rnd is X");
    assert!(
        (100..=200).contains(&rnd),
        "$urandom_range(100,200) init out of range: {}",
        rnd
    );
}

/// $sformatf("%m") and $typename(int) in static initializers: the string
/// results only exist at run time, and the initializer must see the
/// DECLARING instance's scope — including inside a child instance, whose
/// deferred init runs with that instance's name-resolution scope.
#[test]
fn static_init_sformatf_percent_m_and_typename() {
    let src = r#"
module child;
  static string c_path = $sformatf("%m");
endmodule

module tb;
  child u1();
  static string path  = $sformatf("Path is %m");
  static string tname = $typename(int);
  initial begin
    #1;
    if (path == "Path is tb")   $display("PATH_OK");
    if (tname == "int")         $display("TYPENAME_OK");
    if (u1.c_path == "tb.u1")   $display("CHILD_PATH_OK");
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    let has = |tag: &str| sim.output.iter().any(|o| o.message.contains(tag));
    assert!(has("PATH_OK"), "$sformatf(\"%m\") static init wrong");
    assert!(has("TYPENAME_OK"), "$typename(int) static init wrong");
    assert!(has("CHILD_PATH_OK"), "child-instance static %m init wrong");
}

/// $test$plusargs / $value$plusargs consumed by a static initializer must
/// observe the run's plusargs (they cannot exist at elaboration time), and
/// the value must be visible to `initial` blocks (§6.21: static inits run
/// before any initial block starts).
#[test]
fn static_init_plusargs_sysfuncs() {
    let src = r#"
module tb;
  static int has_mode = $test$plusargs("TEST_MODE");
  static int seed     = get_seed();
  function static int get_seed();
    int v;
    if ($value$plusargs("SEED_VAL=%d", v)) return v;
    return 0;
  endfunction
  int seen_mode, seen_seed;
  initial begin
    seen_mode = has_mode;
    seen_seed = seed;
    #1 $finish;
  end
endmodule
"#
    .to_string();
    let plusargs = vec!["TEST_MODE".to_string(), "SEED_VAL=42".to_string()];
    let sim = xezim::simulate_multi(
        &[src],
        100,
        xezim::SimOptions {
            plusargs: plusargs.to_vec(),
            ..Default::default()
        },
    )
    .expect("simulate failed");
    assert_eq!(
        get(&sim, "seen_mode").to_u64(),
        Some(1),
        "$test$plusargs(\"TEST_MODE\") static init must be 1"
    );
    assert_eq!(
        get(&sim, "seen_seed").to_u64(),
        Some(42),
        "$value$plusargs seed static init must be 42"
    );
}
