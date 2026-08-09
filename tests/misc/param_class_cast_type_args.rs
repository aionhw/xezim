//! Pure-SystemVerilog regression for the xezim `$cast` type-parameter bug.
//!
//! Distilled from UVM's sequencer `start_phase_sequence` (the UV TEST TIMEOUT /
//! sequence-never-started failure): the root cause is that the method does
//!
//!     uvm_resource#(uvm_sequence_base)  sbr;
//!     $cast(sbr, rsrc);
//!
//! where `rsrc`'s dynamic type is `uvm_resource#(uvm_object_wrapper)` (the
//! factory-wrapper form stored by `uvm_config_wrapper::set`). Those two
//! parameterized-class instantiations are UNRELATED classes, so the cast MUST
//! FAIL — that is what routes control on to the `uvm_object_wrapper` branch
//! that creates and starts the default sequence.
//!
//! xezim wrongly returned "success": it compared only the base class hierarchy
//! and the value parameters, never the TYPE parameters. So `R#(A)` and
//! `R#(B)` (siblings sharing a base) compared equal, the sequence was never
//! started, and the sequence body's `uvm_info` messages never ran (count 0).
//!
//! LRM 1800-2023 §8.25: two distinct instantiations of a parameterized class
//! are the SAME type iff their parameter values are the same. Class-typed
//! parameters make `R#(A)` and `R#(B)` distinct types; `$cast` between them
//! must fail even when the type arguments are subclasses/siblings of a common
//! base or are builtin types.

use xezim::simulate;

/// Assert that the parameterized-class `$cast` obeys LRM §8.25.1: a cast
/// between two distinct instantiations of the same class with different type
/// arguments must FAIL.
#[test]
fn parameterized_class_cast_type_args_are_distinct() {
    const SRC: &str = r#"
module top;

  class C; endclass
  class A extends C; endclass   // like uvm_sequence_base
  class B extends C; endclass   // like uvm_object_wrapper (sibling of A)

  class R #(type T = C); C v; endclass  // like uvm_resource #(type T)
  class S #(type T = C); C v; endclass  // a different name

  int fails = 0;
  int cnt = 0;

  task chk(string nm, int ok, int want);
    cnt = cnt + 1;
    if (ok !== want) begin
      $display("  ['%s'] got=%0d want=%0d", nm, ok, want);
      fails = fails + 1;
    end
  endtask

  initial begin
    R#(A) ra, ra2;
    R#(B) rb;
    R#(C) rc;
    R#(int) ri;
    R#(string) rstr;
    S#(A) sa;
    int ok;

    ra = new;

    ok = $cast(rb,  ra); chk("R#(B)<-R#(A) unrelated-param", ok, 0);  // MUST FAIL
    ok = $cast(ra2, ra); chk("R#(A)<-R#(A) identical-param", ok, 1);   // pass
    ok = $cast(rc,  ra); chk("R#(C)<-R#(A) param A<:C", ok, 0);        // MUST FAIL
    ok = $cast(ri,  ra); chk("R#(int)<-R#(A) builtin-param", ok, 0);  // MUST FAIL

    ri = new; rstr = new;
    ok = $cast(ri, rstr); chk("R#(int)<-R#(string) unrelated-builtin", ok, 0);

    ok = $cast(sa, ra);   chk("S#(A)<-R#(A) different class", ok, 0);  // MUST FAIL

    $display("RESULT checks=%0d failures=%0d", cnt, fails);
    if (fails == 0) $display("TAGPASS parameterized-class $cast correct");
    else            $display("TAGFAIL parameterized-class $cast wrong (%0d)", fails);
  end
endmodule
"#;

    let sim = simulate(SRC, 1_000).expect("simulation failed");
    let msgs: Vec<String> = sim
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.contains("TAGPASS") || m.contains("TAGFAIL") || m.contains("RESULT"))
        .collect();
    assert!(
        msgs.iter().any(|m| m.contains("TAGPASS")),
        "expected the parameterized-class $cast checks to ALL pass on the reference, got:\n{msgs:#?}"
    );
    assert!(
        msgs.iter().all(|m| !m.contains("TAGFAIL")),
        "parameterized-class $cast wrongly succeeded between distinct type params:\n{msgs:#?}"
    );
}