//! §7.2/§11.4.5 queue equality with x/z elements — reference-verified: an
//! unknown element makes the comparison neither equal NOR unequal, so BOTH
//! `q == q` and `q != q` yield 0 (the same-storage shortcut that returned 1
//! for self-comparison is gone; the element walk decides every pair).

use xezim::simulate;

#[test]
fn queue_eq_with_x_elements_matches_reference() {
    let sim = simulate(
        r#"
module top;
  logic [7:0] q1[$], q2[$];
  initial begin
    q1.push_back(8'hxx);
    q2.push_back(8'hxx);
    $display("SELF_%b_%b", q1 == q1, q1 != q1);
    $display("PAIR_%b_%b", q1 == q2, q1 != q2);
    q1.delete(); q2.delete();
    q1.push_back(8'h5a); q2.push_back(8'h5a);
    $display("EQ_%b_%b", q1 == q2, q1 != q2);
  end
endmodule
"#,
        200,
    )
    .expect("simulate failed");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    let has = |t: &str| msgs.iter().any(|m| m == t);
    assert!(has("SELF_0_0"), "q==q / q!=q with x element: {:?}", msgs);
    assert!(has("PAIR_0_0"), "x-elem pair compare: {:?}", msgs);
    assert!(has("EQ_1_0"), "known-equal pair: {:?}", msgs);
}
