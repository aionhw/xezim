//! §13.5.2/§8.24 name resolution: a subroutine `ref` FORMAL whose name shadows
//! a same-named class PROPERTY must resolve to the FORMAL's own storage inside
//! the method — the innermost scope wins — while a registration inherited from
//! an ENCLOSING frame keeps losing to the property (the §13.5.2 outer value-
//! param case).
//!
//! Two previously-broken shapes, both exercised by register-model prediction
//! (`get_value_array(ref T value[])` in an item whose class declares a
//! `value` property):
//!   1. Whole-array copy FROM the property INTO the shadowing formal,
//!      `value = this.value`. The both-sides-collection resolver preferred the
//!      property for the LHS too, so the copy targeted the property storage
//!      (a self-copy) and the ref writeback shipped an empty array; a variant
//!      path clobbered the property instead.
//!   2. Whole-array copy FROM a local INTO the shadowing formal,
//!      `value = tmp`. The LHS resolved to the property store, clobbering it;
//!      the formal stayed empty and the caller saw nothing.
//!
//! After either assignment the PROPERTY must keep its own content untouched.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .unwrap_or_else(|| panic!("signal {} not found", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// `ref` formal shadowing a same-named dynamic-array property: a whole-array
/// copy from the property (`value = this.value`) must fill the formal — so the
/// ref argument the caller passed receives the data — and must leave the
/// property itself unmodified.
#[test]
fn ref_formal_shadow_whole_array_copy_from_property() {
    let src = r#"
module top;
  typedef bit unsigned [63:0] u64_t;
  class item;
    rand u64_t value[];
    function new;
      value = new[1];
      value[0] = 64'h1245678;
    endfunction
    // The formal `value` shadows the property `value`.
    function void copy_out(ref u64_t value[]);
      value = this.value;
    endfunction
  endclass

  u64_t vals[];
  int ok;
  initial begin
    item it = new;
    it.copy_out(vals);
    if (vals.size() == 1 && vals[0] == 64'h1245678
        && it.value.size() == 1 && it.value[0] == 64'h1245678)
      ok = 1;
    else
      ok = 0;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "ok"), 1, "ref formal receives property content via value = this.value");
}

/// The same shadowing formal filled from a LOCAL (`value = tmp`): the copy must
/// land on the formal (ref writeback reaches the caller), not clobber the
/// property.
#[test]
fn ref_formal_shadow_whole_array_copy_from_local() {
    let src = r#"
module top;
  typedef bit unsigned [63:0] u64_t;
  class item;
    rand u64_t value[];
    function void fill(ref u64_t value[]);
      u64_t tmp[];
      tmp = new[1];
      tmp[0] = 64'hbeef;
      value = tmp;
    endfunction
  endclass

  u64_t vals[];
  int ok;
  initial begin
    item it = new;
    it.fill(vals);
    if (vals.size() == 1 && vals[0] == 64'hbeef
        && it.value.size() == 0)
      ok = 1;
    else
      ok = 0;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "ok"), 1, "ref formal filled from a local; property untouched");
}

/// Mirror direction: the property as LHS, the shadowing formal as RHS
/// (`this.value = value`). `expr_assoc_name` hijacked BOTH operands to the
/// property store, so the property kept its stale content instead of
/// receiving the formal's.
#[test]
fn ref_formal_shadow_copy_into_property() {
    let src = r#"
module top;
  typedef bit unsigned [63:0] u64_t;
  class item;
    rand u64_t value[];
    function new;
      value = new[1];
      value[0] = 64'h1111;
    endfunction
    function void take(ref u64_t value[]);
      this.value = value;
    endfunction
  endclass

  int ok;
  initial begin
    item it = new;
    begin
      u64_t src[];
      src = new[1];
      src[0] = 64'h7777;
      it.take(src);
    end
    if (it.value.size() == 1 && it.value[0] == 64'h7777)
      ok = 1;
    else
      ok = 0;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "ok"), 1, "property receives the shadowing formal's content");
}

/// A registration inherited from an ENCLOSING frame still loses to the
/// property (the §13.5.2 outer value-param case): a deeper method writing a
/// bare same-named collection member must keep hitting the property store.
#[test]
fn enclosing_frame_collection_name_still_prefers_property() {
    let src = r#"
module top;
  class item;
    rand int payload[];
    function new;
      payload = new[1];
      payload[0] = 11;
    endfunction
    function void make(int val[]);
      val = new[2];
      val[0] = 22;
      val[1] = 33;
      inner();
    endfunction
    function void inner();
      // `payload` here names the PROPERTY: this frame has no local of that
      // name, and the outer `val` registration must not hijack the write.
      payload[0] = 99;
    endfunction
  endclass

  int vals[];
  int ok;
  initial begin
    item it = new;
    it.make(vals);
    // `val` is a VALUE param: the caller's array must stay untouched, while
    // inner()'s bare-name write still reaches the property.
    if (it.payload.size() == 1 && it.payload[0] == 99 && vals.size() == 0)
      ok = 1;
    else
      ok = 0;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(
        u(&sim, "ok"),
        1,
        "outer-frame collection registration must not shadow the property"
    );
}
