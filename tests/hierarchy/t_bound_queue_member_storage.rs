//! A class member whose declared type is a class type-parameter `T`, bound to
//! an unpacked array/queue, must still materialize its per-instance queue
//! storage when assigned through a bare (unqualified) reference inside a
//! method. `handle_collection_name` already resolved `this.member`/`obj.member`
//! through the parameter binding (`prop_bound_collection`), but `instance_assoc_member`
//! (the bare-identifier path) did not — so `val = t;` inside a method that uses
//! the bare member name silently wrote to scalar scratch storage and never
//! populated the queue (subsequent reads returned the initializer / size 1).
//!
//! Verified byte-for-byte against a commercial simulator: R2 and R3 print
//! `3,4,5,6 sz=4` (variant C exercises a `T`-typed formal `t` whose value-param
//! storage must be copied into a *different* `T`-bound member, not just
//! returned; R1 is unaffected because it returns a formal rather than writing a
//! member).

use xezim::simulate_multi;

#[test]
fn t_bound_queue_member_storage() {
    let src = r#"
module top;
  typedef int int_arr[];
  class res #(type T=int);
    T val;        // T-bound, copies a concrete queue
    T param_a;    // T-bound, receives a T-typed formal
    function void setA(int_arr t); val = t; endfunction
    function void setC(T t);        param_a = t; endfunction
    function T getVal();   return val;    endfunction
    function T getParam(); return param_a; endfunction
  endclass
  res#(int_arr) r;
  int_arr src = '{3,4,5,6};
  int_arr b, e;
  initial begin
    r = new;
    r.setA(src);
    b = r.getVal();
    $display("R2 setA(concrete)->T member: %0d,%0d,%0d,%0d sz=%0d", b[0],b[1],b[2],b[3],b.size());
    r.setC(src);
    e = r.getParam();
    $display("R3 setC(T param)->T member: %0d,%0d,%0d,%0d sz=%0d", e[0],e[1],e[2],e[3],e.size());
  end
endmodule
"#;
    let out: Vec<String> = simulate_multi(
        &[src.to_string()], 1000, Some("top"), &[], &[], None, false, None, None,
        &[], &[], None, &[], 0, u64::MAX, None, &[], None, None, None, None, false, None,
    )
    .expect("sim")
    .output
    .iter()
    .map(|o| o.message.clone())
    .collect();
    // Both variants must carry the full element set {3,4,5,6} (reference
    // matches byte-for-byte). Before the fix these read back 0,0,0,0 sz=1.
    assert!(out.iter().any(|l| l == "R2 setA(concrete)->T member: 3,4,5,6 sz=4"),
        "R2 (concrete->T member) mismatch; got {:?}", out);
    assert!(out.iter().any(|l| l == "R3 setC(T param)->T member: 3,4,5,6 sz=4"),
        "R3 (T param->T member) mismatch; got {:?}", out);
}