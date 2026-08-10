//! Pure-SystemVerilog regression for `$cast` to one's own `this_type` inside
//! a parameterized class.
//!
//! Distilled from UVM's `uvm_class_pair #(T1,T2)` `do_copy`/`this_type` path,
//! which fatals `[WRONG_TYPE] do_copy: rhs argument is not of type
//! 'uvm_class_pair #(T1,T2)'`. `uvm_class_pair #(transaction,transaction)`
//! declares `typedef uvm_class_pair#(T1,T2) this_type;` and `do_copy` does
//! `$cast(rhs_, rhs)` where `rhs_ : this_type`. `uvm_object::clone` ->
//! `copy` -> `do_copy(rhs=the original pair)`, so the cast must succeed when
//! `rhs` is the same specialization.
//!
//! xezim recorded the typedef's specialization ARGS textually as the *type
//! parameter names* `T1`,`T2` (the class's own params, not the concrete
//! `uvm_class_pair#(transaction,transaction)` args). `$cast_type_params_ok`
//! then compared the literal string `"T1"` (dest arg) against the src
//! instance's concrete type binding `"txn"` and — mismatch — made every cast
//! to one's own `this_type` fail.
//!
//! LRM 1800-2023 §8.25: two instantiations are the same type iff their type
//! arguments are the same. A dest arg that is a bare type-param NAME (a
//! placeholder) must be resolved to its bound concrete type before comparing.
use xezim::simulate;

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no {} line", tag))
}

/// A parameterized class casts `rhs` to its own `this_type` (the exact UVM
/// `uvm_class_pair` pattern). It must succeed.
#[test]
fn cast_to_own_this_type_succeeds() {
    const SRC: &str = r#"
module top;
  class txn; int v; function new(); v = 0; endfunction endclass

  class pair #(type T1, type T2 = T1) extends txn;
    typedef pair#(T1,T2) this_type;
    T1 first;
    T2 second;
    virtual function void check(txn rhs);
      this_type rhs_;
      if (!$cast(rhs_, rhs)) $display("CAST_FAIL");
      else $display("CAST_PASS");
    endfunction
  endclass

  pair#(txn,txn) p;
  initial begin
    p = new;
    p.first = new; p.second = new;
    p.check(p);   // p is-a txn; casting p (a pair#(txn,txn)) to this_type must succeed
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    // The reference cast to one's own `this_type` succeeds.
    assert!(
        line(&sim, "CAST_PASS").starts_with("CAST_PASS"),
        "cast to own this_type must succeed: {}",
        line(&sim, "CAST_").trim()
    );
}

/// A `$cast` to a *different* specialization still fails (LRM §8.25.1) — the
/// placeholder resolution must not make unrelated instantiations comparable.
#[test]
fn different_specialization_still_fails() {
    const SRC: &str = r#"
module top;
  class base; endclass
  class t1 extends base; endclass
  class t2 extends base; endclass

  class pair #(type T1, type T2 = T1) extends base;
    typedef pair#(T1,T2) this_type;
    virtual function void ck(base rhs);
      this_type d;
      if (!$cast(d, rhs)) $display("MISMATCH_CAST_FAIL");
      else $display("MISMATCH_CAST_PASS");
    endfunction
  endclass

  pair#(t1,t1) a;
  pair#(t2,t2) b;
  initial begin
    a = new; b = new;
    a.ck(b);   // different type args (t1 vs t2) => cast must FAIL
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    let got = line(&sim, "MISMATCH_CAST_");
    assert!(
        got.starts_with("MISMATCH_CAST_FAIL"),
        "different specializations must not cast: {}",
        got.trim()
    );
}