//! §18.16 `randcase` and §18.17.1 `randsequence` alternatives are chosen at
//! RUNTIME, weighted by their weight expressions. Both used to be lowered at
//! PARSE time to the first non-zero-weight branch — i.e. not random at all,
//! which silently made every weighted-choice testbench take one path forever.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
        & 0xFFFF_FFFF
}

const RANDCASE: &str = r#"
module tb;
  int w1, w2, w3, w0;
  initial begin
    repeat (6000) begin
      randcase
        1: w1++;
        2: w2++;
        3: w3++;
        0: w0++;
      endcase
    end
  end
endmodule
"#;

const RANDSEQ: &str = r#"
module tb;
  int a_hits, b_hits, c_hits;
  initial begin
    repeat (3000) begin
      randsequence (main)
        main : a := 1 | b := 2 | c := 3 ;
        a : { a_hits++; } ;
        b : { b_hits++; } ;
        c : { c_hits++; } ;
      endsequence
    end
  end
endmodule
"#;

#[test]
fn randcase_honors_weights_and_never_takes_zero() {
    let sim = simulate(RANDCASE, 100_000).expect("simulate failed");
    let (w1, w2, w3, w0) = (u(&sim, "w1"), u(&sim, "w2"), u(&sim, "w3"), u(&sim, "w0"));
    assert_eq!(w0, 0, "§18.16: a zero-weight branch is never taken");
    assert_eq!(w1 + w2 + w3, 6000, "every draw takes exactly one branch");
    // 1:2:3 over 6000 draws — generous bounds, but they exclude both a
    // uniform draw and the old always-first behavior.
    assert!((700..=1300).contains(&w1), "w1={} not ~1000", w1);
    assert!((1700..=2300).contains(&w2), "w2={} not ~2000", w2);
    assert!((2650..=3350).contains(&w3), "w3={} not ~3000", w3);
}

#[test]
fn randsequence_alternatives_are_weighted() {
    let sim = simulate(RANDSEQ, 100_000).expect("simulate failed");
    let (a, b, c) = (u(&sim, "a_hits"), u(&sim, "b_hits"), u(&sim, "c_hits"));
    assert_eq!(a + b + c, 3000, "one alternative per iteration");
    assert!((330..=680).contains(&a), "a={} not ~500", a);
    assert!((800..=1200).contains(&b), "b={} not ~1000", b);
    assert!((1250..=1750).contains(&c), "c={} not ~1500", c);
}
