//! IEEE 1800-2023 §9.4.2, §14.3, §14.16.2: four-state clock edges and
//! clockvar-specific output skews retain their event and maturity ordering.

fn check(src: &str, want: &[&str]) {
    let sim = xezim::simulate(src, 100_000).expect("simulate");
    let lines: Vec<_> = sim
        .output
        .iter()
        .filter(|o| o.message.starts_with("T|"))
        .map(|o| o.message.as_str())
        .collect();
    assert_eq!(lines, want, "{:?}", sim.output);
}

#[test]
fn unknown_clock_transitions_mature_edge_outputs() {
    for (decl, first, second) in [
        ("logic clk=0;", "1'b1", "1'bx"),
        ("logic clk=0;", "1'bx", "1'b0"),
        ("logic clk=1'bx;", "1'b1", "1'bz"),
        ("logic [1:0] clk=2'bx0;", "2'bx1", "2'bx0"),
    ] {
        let src = format!(
            r#"
`timescale 1ns/1ps
module top;
 {decl}
 logic q=0;
 clocking cb @(posedge clk); output negedge q; endclocking
 initial begin @(cb); cb.q<=1; end
 initial begin #1 clk={first}; #1 clk={second}; #1 $display("T|q=%b",q); $finish; end
endmodule
"#
        );
        check(&src, &["T|q=1"]);
    }
}

#[test]
fn aliases_keep_their_edge_skews_before_the_clock_event() {
    for cycles in ["", "##1 "] {
        let src = format!(
            r#"
`timescale 1ns/1ps
module top;
 logic clk=0;
 logic [3:0] q=0;
 always #5 clk=~clk;
 clocking cb @(posedge clk);
   output negedge a=q;
   output posedge b=q;
 endclocking
 initial begin
   #1 cb.a<={cycles}1;
   #1 cb.b<={cycles}2;
   #9 $display("T|first=%0d",q);
   #5 $display("T|second=%0d",q);
   $finish;
 end
endmodule
"#
        );
        check(&src, &["T|first=1", "T|second=2"]);
    }
}

#[test]
fn aliases_with_different_delay_skews_both_reach_the_net() {
    check(
        r#"
`timescale 1ns/1ps
module top;
 logic clk=0;
 logic [3:0] q=0;
 always #5 clk=~clk;
 clocking cb @(posedge clk);
   output #1 a=q;
   output #2 b=q;
 endclocking
 initial begin
   #1 cb.a<=1;
   #1 cb.b<=2;
   #4.5 $display("T|first=%0d",q);
   #1 $display("T|second=%0d",q);
   $finish;
 end
endmodule
"#,
        &["T|first=1", "T|second=2"],
    );
}
