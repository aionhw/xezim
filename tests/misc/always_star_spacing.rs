//! IEEE 1800-2017 §5.3 / §9.4.2.2: @(*) accepts token-separating
//! white space and comments; both implicit processes must remain live.
use xezim::simulate;

#[test]
fn mixed_implicit_event_spellings_update_both_outputs() {
    for event in ["@(* )", "@(*\t)", "@(*\n)", "@( * )", "@(* /* c */ )"] {
        let src = format!(
            r#"module top;
  logic source = 0;
  logic first, second;
  always {event} first = source;
  always @(*) second = source;
  initial begin
    #1 source = 1;
    #1 $display("high %0b %0b", first, second);
    source = 0;
    #1 $display("low %0b %0b", first, second);
    $finish;
  end
endmodule"#
        );
        let sim = simulate(&src, 10).expect("mixed implicit sensitivity spellings");
        let out: Vec<_> = sim
            .output
            .iter()
            .map(|line| line.message.as_str())
            .collect();
        assert_eq!(out, ["high 1 1", "low 0 0"], "{event}");
    }
}
