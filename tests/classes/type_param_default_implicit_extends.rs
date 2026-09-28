//! IEEE 1800-2023 §6.20.2 + §8.25: an `extends` clause that omits a parameter
//! argument leaves it at its DECLARED DEFAULT, on the task path as well as the
//! function path.
//!
//! `class d extends pbase;` — the shape a UVM base test uses
//! (`class base_test extends base_test_param;`) — is a specialization of
//! `pbase` with every parameter at its default: §8.25 makes a parameterized
//! class's generic declaration its default specialization, and §6.20.2 gives an
//! omitted parameter its declared default value. A method declared in `pbase`
//! and called on such an instance must therefore see `T` = `int` and `W` = 8.
//!
//! Before the fix the omitted arguments leaked their own bare NAMES into the
//! specialization signature (`pbase#(T,W)`), a key no resolution can bind, so a
//! reference to `T` landed on the unknown-type fallback. Measured without the
//! fix — `["TP|f 1 8", "TP|t 1 8", "TP|name logic 1", ...]` — i.e. `$bits(T)`
//! was 1 and the parameter named `logic` on BOTH paths. In a UVM testbench that
//! made `T::type_id::create` return null, so the run completed reporting
//! `UVM_ERROR: 0` while driving no transactions at all.
//! Cross-checked against the reference simulator.

#[test]
fn omitted_extends_arguments_use_declared_defaults_on_both_paths() {
    let sim = xezim::simulate(
        r#"
class pbase #(type T = int, int W = 8);
  virtual function void show_f();    $display("TP|f %0d %0d", $bits(T), W); endfunction
  virtual task         show_t();      $display("TP|t %0d %0d", $bits(T), W); endtask
  virtual function void show_name();  $display("TP|name %s %0d", $typename(T), $bits(T)); endfunction
endclass

// §6.20.2: no argument supplied -> T = int, W = 8.
class dimplicit extends pbase; endclass

// Control: an explicit specialization keeps its own arguments.
class dexplicit extends pbase #(bit [15:0], 4); endclass

module tb;
  dimplicit a;
  dexplicit b;
  initial begin
    a = new();
    b = new();
    a.show_f();
    a.show_t();
    a.show_name();
    b.show_f();
    b.show_t();
  end
endmodule
"#,
        10,
    )
    .expect("simulate");
    let o: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    // omitted arguments -> declared defaults, on the FUNCTION path ...
    assert!(o.iter().any(|l| l == "TP|f 32 8"), "implicit/func: {o:?}");
    // ... and on the TASK path (§8.25 receiver specialization)
    assert!(o.iter().any(|l| l == "TP|t 32 8"), "implicit/task: {o:?}");
    // the type parameter names its declared default type, not `logic`
    assert!(
        o.iter().any(|l| l == "TP|name int 32"),
        "implicit/type: {o:?}"
    );
    // the same class spelled explicitly is unaffected
    assert!(o.iter().any(|l| l == "TP|f 16 4"), "explicit/func: {o:?}");
    assert!(o.iter().any(|l| l == "TP|t 16 4"), "explicit/task: {o:?}");
}
