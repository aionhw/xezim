//! §11.4.5 `===`/`!==` on UNPACKED collections (issue #1043, Bug L). Case
//! equality "shall" resolve to a match/mismatch and "never result in 1'bx",
//! so the element walk must compare 4-state bits exactly: an x/z element
//! matches an identical x/z element and never matches a known one. Before the
//! fix the dispatch only handled `==`/`!=` here and let `===`/`!==` fall
//! through to the packed-scalar path, making them value-insensitive (fixed
//! arrays, dynamic arrays/queues, and associative arrays). Reference-verified.

use xezim::simulate;

#[test]
fn case_eq_eq_on_unpacked_collections() {
    let sim = simulate(
        r#"
module top;
  logic [7:0] fa[0:2], fb[0:2];
  logic [7:0] qa[$], qb[$];
  logic [7:0] aa[int], ab[int];
  initial begin
    fa = '{8'h10, 8'h20, 8'h30};
    fb = '{8'h10, 8'h20, 8'h30};
    $display("FA_EQ_%b_%b", fa === fb, fa !== fb);
    fb[1] = 8'h21;
    $display("FA_NE_%b_%b", fa === fb, fa !== fb);

    qa.push_back(8'hxx); qb.push_back(8'hxx);
    $display("QX_%b_%b", qa === qb, qa !== qb);
    qb[0] = 8'hxx;
    $display("QX2_%b_%b", qa === qb, qa !== qb);
    qb[0] = 8'h01;
    $display("QX3_%b_%b", qa === qb, qa !== qb);

    aa[3] = 8'hab; ab[3] = 8'hab;
    $display("AA_%b_%b", aa === ab, aa !== ab);
    ab[3] = 8'hac;
    $display("AA2_%b_%b", aa === ab, aa !== ab);
  end
endmodule
"#,
        200,
    )
    .expect("simulate failed");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    let has = |t: &str| msgs.iter().any(|m| m == t);
    assert!(has("FA_EQ_1_0"), "fixed equal: {:?}", msgs);
    assert!(has("FA_NE_0_1"), "fixed differing: {:?}", msgs);
    assert!(has("QX_1_0"), "queue identical x matches: {:?}", msgs);
    assert!(has("QX2_1_0"), "queue equal x matches: {:?}", msgs);
    assert!(has("QX3_0_1"), "queue x vs known differs: {:?}", msgs);
    assert!(has("AA_1_0"), "assoc equal: {:?}", msgs);
    assert!(has("AA2_0_1"), "assoc differing: {:?}", msgs);
}
