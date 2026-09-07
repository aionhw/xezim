//! Regression: call-bearing INSTANCE property initializers must run at
//! construction time, including field-order dependencies between them.
//!
//! SystemVerilog semantics (§8.2) require a field initializer to execute as if
//! it were the first statement of `new()`. The simulator used to SKIP any
//! initializer containing a function call (to avoid recursion), leaving the
//! elaborate-time value in place. For a singleton-getter initializer like
//! `uvm_root r = uvm_root::get()` or
//! `uvm_factory f = cs.get_factory()`, the elaborate-time value is always NULL
//! (the singleton doesn't exist during elaboration), so every UVM test that
//! cached `root`/`coreservice`/`factory` as a *field* silently held null
//! handles and `factory.set_*` calls no-op'ed, breaking factory alias and
//! override registration (every set via a cached `factory` field silently
//! discarded).
//!
//! This repro models the singleon pattern with class-static `get()` methods
//! and:
//!   * `cs = core::get()`            — a singleton getter field initializer,
//!   * `f  = cs.get_factory()`       — a field initializer that READS an
//!                                     earlier-declared field (`cs`): the
//!                                     cross-field dependency that a simple
//!                                     unordered per-field pass gets wrong if
//!                                     `f` is visited before `cs`.
//!   * `top = uvmroot::get()`        — a second independent singleton.
//! Printing each handle's non-null-ness inside the class's constructor then
//! asserting on the accumulated marker proves both the construction-time eval
//! and the fixed-point dependency resolution.
use xezim::simulate;

fn output_of(sim: &xezim::compiler::Simulator) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn call_bearing_field_initializers_evaluate_at_construction_with_dependency() {
    const SRC: &str = r#"
module top;
  // ---- minimal singleton pattern (mirrors uvm_root / uvm_coreservice_t) ----
  class factory;
    static factory me;
    static function factory get();
      if (me == null) me = new();
      return me;
    endfunction
  endclass

  class core;
    static core me;
    static function core get();
      if (me == null) me = new();
      return me;
    endfunction
    function factory get_factory();
      return factory::get();
    endfunction
  endclass

  class uvmroot;
    static uvmroot me;
    static function uvmroot get();
      if (me == null) me = new();
      return me;
    endfunction
  endclass

  // ---- the class under test: field initializers with calls + dependency ----
  class tbench;
    core     cs  = core::get();
    uvmroot  top = uvmroot::get();
    factory  f   = cs.get_factory();

    function new();
      $display("CTOR cs=%0d top=%0d f=%0d",
        (cs != null), (top != null), (f != null));
    endfunction
  endclass

  initial begin
    tbench t;
    t = new();
    if (t.cs != null && t.top != null && t.f != null)
      $display("FIELD_INIT_OK");
    else
      $display("FIELD_INIT_FAIL");
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 100).expect("sim"));
    assert!(
        out.contains("CTOR cs=1 top=1 f=1"),
        "field initializers were not evaluated at construction (singletons null):\n{}",
        out
    );
    assert!(
        out.contains("FIELD_INIT_OK"),
        "singleton fields never resolved after construction:\n{}",
        out
    );
    assert!(
        !out.contains("FIELD_INIT_FAIL"),
        "singleton fields are null:\n{}",
        out
    );
}