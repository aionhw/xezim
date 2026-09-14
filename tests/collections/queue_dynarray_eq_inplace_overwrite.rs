//! §7.2 / §11.4.5 queue / dynamic-array `==` after an in-place element
//! overwrite — reference-verified. Overwriting an element of an existing
//! queue via `q[i] = v` stores an 8-bit element, while an initializer /
//! whole-copy builds a 32-bit element; the old collection compare compared
//! the Value *structs* (width included), so two equal-valued elements with
//! different storage widths compared unequal and `a.q == b.q` came out 0.
//! The compare now uses semantic equality (zero-extends the narrower
//! operand), so all of these must print 1.

use xezim::simulate;

#[test]
fn queue_dynarray_eq_after_inplace_overwrite() {
    let sim = simulate(
        r#"
class it;
  byte q[$] = '{'h1e,'h2d,'he};
  shortint da[];
  function new(); da = new[3]; da[0]='h1; da[1]='h2; da[2]='h3; endfunction
endclass
module top;
  it a, b;
  integer i;
  initial begin
    a = new; b = new;
    // In-place overwrite of an existing queue (the instructive case):
    // this must NOT break later `==`.
    for (i = 0; i < 3; i++) b.q[i] = a.q[i];
    $display("Q_OVR_%b", a.q == b.q);
    // Element-wise equality must also hold.
    $display("Q_ELEM_%b", a.q[0]===b.q[0] && a.q[1]===b.q[1] && a.q[2]===b.q[2]);
    // Repopulate via push_back (pre-existing correct path, sanity).
    while (b.q.size() > 0) void'(b.q.pop_back());
    for (i = 0; i < 3; i++) b.q.push_back(a.q[i]);
    $display("Q_PUSH_%b", a.q == b.q);
    // Whole-queue assign (pre-existing correct path, sanity).
    b.q = a.q;
    $display("Q_ASGN_%b", a.q == b.q);
    // Dynamic array: fresh new[] + overwrite (worked before, sanity).
    b.da = new[3];
    for (i = 0; i < 3; i++) b.da[i] = a.da[i];
    $display("DA_OVR_%b", a.da == b.da);
    // A genuinely different element must still compare unequal.
    b.da[0] = 'hff;
    $display("DA_NEQ_%b", a.da == b.da);
    $display("DA_NEQ2_%b", a.da != b.da);
  end
endmodule
"#,
        200,
    )
    .expect("simulate failed");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    let has = |t: &str| msgs.iter().any(|m| m == t);
    assert!(has("Q_OVR_1"), "in-place overwrite queue ==: {:?}", msgs);
    assert!(has("Q_ELEM_1"), "elementwise queue ==: {:?}", msgs);
    assert!(has("Q_PUSH_1"), "push_back queue ==: {:?}", msgs);
    assert!(has("Q_ASGN_1"), "whole-assign queue ==: {:?}", msgs);
    assert!(has("DA_OVR_1"), "new+overwrite dyn-array ==: {:?}", msgs);
    assert!(has("DA_NEQ_0"), "unequal dyn-array ==: {:?}", msgs);
    assert!(has("DA_NEQ2_1"), "unequal dyn-array !=: {:?}", msgs);
}