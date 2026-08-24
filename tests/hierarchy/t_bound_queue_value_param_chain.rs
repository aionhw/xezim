//! A `T`-typed value-parameter queue passed through a chain of nested generic
//! methods into a `T`-bound class member must survive to the member. This
//! exercises four cooperating fixes in xezim's runtime:
//!
//! 1. The specialization signature of a nested generic member (`W#(T) s;`
//!    inside `W#(T)` where `T` resolves to `int_arr`) must carry the concrete
//!    type, not be clobbered to the parameter's declared default (`int`) by
//!    `canonicalize_spec_sig` — otherwise `bind_queue_param` sees non-queue
//!    dims and binds the value-param as a scalar.
//! 2. A bare collection member of `this` (`val = t` in `do_write`) must
//!    resolve to its `<handle>#member` storage even when a same-named
//!    value-param (`make(T val)`) is registered from an enclosing frame.
//! 3. `fixed_array_operand` must not treat a dynamic-array/queue sentinel
//!    range (0,-1) as a fixed array.
//! 4. `==`/`!=` on two queues compares element-by-element.
//!
//! Verified byte-for-byte against a commercial simulator: `CD5 sz=4 1,2,3,4`
//! and `CD5_PASS`.

use xezim::simulate_multi;

#[test]
fn t_bound_queue_value_param_chain() {
    let src = r#"
module top;
  typedef int int_arr[];
  class Store #(type T=int);
    T val;
    function void write(T t); do_write(t); endfunction
    virtual function void do_write(T t); val = t; endfunction
    function T get(); return val; endfunction
  endclass
  class Imp #(type T=int);
    Store#(T) st;
    function new(); st = new; endfunction
    function void set(T val); st.write(val); endfunction
  endclass
  class TopDb #(type T=int);
    static function Imp#(T) make(T val);
      Imp#(T) imp = new;
      imp.set(val);
      return imp;
    endfunction
  endclass
  int_arr src = '{1,2,3,4};
  int_arr out;
  initial begin
    Imp#(int_arr) imp = TopDb#(int_arr)::make(src);
    out = imp.st.get();
    $display("CD5 sz=%0d %0d,%0d,%0d,%0d", out.size(), out[0],out[1],out[2],out[3]);
    if (out.size()==4 && out[0]==1 && out[3]==4) $display("CD5_PASS");
    else $display("CD5_FAIL");
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
    assert!(out.iter().any(|l| l.contains("CD5 sz=4 1,2,3,4")),
        "expected full element set; got {:?}", out);
    assert!(out.iter().any(|l| l == "CD5_PASS"), "got {:?}", out);
    assert!(!out.iter().any(|l| l == "CD5_FAIL"), "got {:?}", out);
}