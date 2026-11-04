//! IEEE 1800-2017 §22.5.1: a single empty macro actual is a supplied
//! position, including when its formal is unused (a display macro that
//! ignores its argument).
use xezim::simulate;

#[test]
fn empty_macro_actuals_expand_in_strict_mode() {
    let sim = simulate(
        r#"`define MESSAGE(prefix) $display("PASS: unused empty actual")
`define QUOTE(x) `"x`"
`define DEFAULT(x=7) x
`define ZERO() 9
module top;
  initial begin
    `MESSAGE();
    `MESSAGE(   );
    $display("values|%s|%0d|%0d", `QUOTE(), `DEFAULT(), `ZERO());
    $finish;
  end
endmodule
"#,
        10,
    )
    .expect("simulate with default strict checks");
    let out: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .collect();
    assert_eq!(
        out,
        [
            "PASS: unused empty actual",
            "PASS: unused empty actual",
            "values||7|9"
        ]
    );
}
