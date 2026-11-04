//! IEEE 1800-2023 §21.2.1: only a known conversion specifier consumes an
//! argument. An unknown `%q` is printed literally; it must not evaluate the
//! next actual (and its side effects) before handing it to the following
//! specifier (#549 follow-up).

use xezim::simulate;

#[test]
fn unknown_specifier_does_not_evaluate_the_next_argument() {
    let src = r#"
module top;
  int n = 0;
  function int f(); n++; return n; endfunction
  initial begin
    $display("%q %0d", f(), 7);
    $display("n=%0d", n);
    $display("100%% done, 50% off, end %");
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    let msgs: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(msgs.iter().any(|m| m.starts_with("%q 1")), "{msgs:?}");
    assert!(msgs.contains(&"n=1"), "{msgs:?}");
    assert!(msgs.contains(&"100% done, 50% off, end %"), "{msgs:?}");
}
