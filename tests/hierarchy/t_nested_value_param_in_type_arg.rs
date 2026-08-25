//! A value parameter used in a NESTED type-argument position inside a
//! parameterized class's own static method must resolve to the ACTIVE
//! specialization's binding, not fall back to the parameter's default.
//!
//! `Comp#(N)::get_holder()` references `Registry#(Comp#(N))`. The nested
//! `Comp#(N)` type argument carries the value parameter `N`, which is a
//! SYMBOLIC name at the point of elaboration. The parser reconstructs the
//! type text with spaces around the `#`/parens (`Comp # ( N )`), and the
//! runtime spec extractor previously only matched the compact `#(` form, so
//! the nested value-param was never recognized and stayed unresolved — it
//! fell back to the declared default `0`. `get_holder()` then returned a
//! `Registry#(Comp#(0))` singleton instead of `Registry#(Comp#(2))`, so
//! different call sites of the "same" specialization produced different
//! registry singletons (a per-spec static/typeid mismatch).
//!
//! `extract_spec_from_string` is now whitespace-tolerant; the regression
//! asserts `Comp#(2)::get_holder()` is the single `Registry#(Comp#(2))`
//! singleton.
//!
//! Verified byte-for-byte against a commercial simulator: `N2_PASS`. Without
//! the fix this self-test FAILs (N resolves to 0).

use xezim::simulate_multi;

#[test]
fn t_nested_value_param_in_type_arg() {
    let src = r#"
class Registry #(type T);
  static Registry#(T) h;
  static function Registry#(T) get();
    if(h==null) h=new;
    return h;
  endfunction
endclass
class Comp #(int N=0);
  static function Registry#(Comp#(N)) get_holder();
    return Registry#(Comp#(N))::get();
  endfunction
endclass
module top;
initial begin
  if (Comp#(2)::get_holder() == Registry#(Comp#(2))::get())
    $display("N2_PASS");
  else
    $display("N2_FAIL");
end
endmodule
"#;
    let out: Vec<String> = simulate_multi(
        &[src.to_string()], 1000, Some("top"), &[], &[], None, false, None, None,
        &[], &[], 1, None, &[], 0, u64::MAX, None, &[], None, None, None, None, false, None,
    )
    .expect("sim")
    .output
    .iter()
    .map(|o| o.message.clone())
    .collect();
    assert!(out.iter().any(|l| l.contains("N2_PASS")),
        "expected nested value-param to resolve to active spec; got {:?}", out);
    assert!(!out.iter().any(|l| l.contains("N2_FAIL")), "got {:?}", out);
}