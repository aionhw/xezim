//! IEEE 1800-2023 §9.2.2.2, §9.4.2.2, §7.2: `always_comb` and `always @*`
//! wake on a read of an element of an unpacked array of unpacked structs, or
//! of the whole array (#991). Expected values are from the reference
//! simulator.

use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_lines(msgs: &[String], want: &[&str]) {
    for w in want {
        assert!(
            msgs.iter().any(|m| m.trim() == *w),
            "missing {w:?} in {msgs:#?}"
        );
    }
}

/// Reads of `qa[0]` (a continuous copy), `pa[0]` and `qa` follow every
/// change of the members.
#[test]
fn element_and_whole_array_reads_wake() {
    let msgs = messages(
        r#"typedef struct { int a; int b; } s_t;
module t;
  int x;
  s_t pa [2], qa [2], ca, cb, ra [2];
  assign pa[0] = '{a: x + 5, b: x};
  assign pa[1] = '{a: 0, b: 0};
  assign qa = pa;
  always_comb ca = qa[0];
  always @* cb = pa[0];
  always_comb ra = qa;
  initial begin
    #1 x = 3;
    #1;
    $display("ca=%0d %0d cb=%0d %0d ra=%0d %0d", ca.a, ca.b, cb.a, cb.b, ra[0].a, ra[1].a);
    x = 4;
    #1 $display("ca=%0d cb=%0d ra=%0d", ca.a, cb.a, ra[0].a);
  end
endmodule"#,
    );
    assert_lines(&msgs, &["ca=8 3 cb=8 3 ra=8 0", "ca=9 cb=9 ra=9"]);
}
