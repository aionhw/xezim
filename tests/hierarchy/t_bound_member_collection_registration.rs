//! A class member whose declared type is a type PARAMETER (`#(type T=int) T val;`)
//! bound to a queue/dynamic-array typedef (`typedef int a_i[];`) must be treated
//! as a collection for the runtime's array-table lookups. In a generic class the
//! member lives only in `cd.properties` and never reaches `queue_properties`/
//! `array_properties`, so the per-instance registration skipped it and
//! `<handle>#val` had no `dynamic_arrays`/`arrays` entry. Consequence: UVM's
//! `uvm_built_in_pair #(T) { T first, second; }` (whose `clone`/`copy`/`do_compare`
//! are `first = rhs_.first` / `first == rhs_.first`) lost array/queue members in
//! `clone`+`copy` — the pair read back size 0 and `compare` failed — and `%p`
//! rendered the member as a scalar `0`.
//!
//! Registering type-param-bound collection members as dynamic arrays at class
//! instantiation fixes this. Verified byte-for-byte against a commercial
//! simulator: `ST first='{-10, -20}`, `ST sz=2 e0=-10 e1=-20`,
//! `ST second='{30, 40}`, `ST_PASS`.

use xezim::simulate_multi;

#[test]
fn t_bound_member_collection_registration() {
    let src = r#"
module top;
  typedef int a_i[];
  class Pair #(type T=int);
    T first, second;
    function void fill(T f, T s); first=f; second=s; endfunction
  endclass
  initial begin
    a_i ar1, ar2;
    Pair#(a_i) a = new;
    ar1=new[2]; ar1[0]=-10; ar1[1]=-20;
    ar2=new[2]; ar2[0]=30; ar2[1]=40;
    a.fill(ar1, ar2);
    $display("ST first=%p", a.first);
    $display("ST sz=%0d e0=%0d e1=%0d", a.first.size(), a.first[0], a.first[1]);
    $display("ST second=%p", a.second);
    if (a.first.size()==2 && a.first[0]==-10 && a.first[1]==-20
        && a.second.size()==2 && a.second[1]==40) $display("ST_PASS");
    else $display("ST_FAIL");
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
    assert!(
        out.iter().any(|l| l == "ST first='{-10, -20}"),
        "T-bound array member `%p` must render a collection, not a scalar 0; got {:?}", out
    );
    assert!(
        out.iter().any(|l| l == "ST second='{30, 40}"),
        "got {:?}", out
    );
    assert!(out.iter().any(|l| l == "ST_PASS"), "got {:?}", out);
    assert!(!out.iter().any(|l| l == "ST_FAIL"), "got {:?}", out);
}