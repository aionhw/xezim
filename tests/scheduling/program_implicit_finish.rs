//! §24.3, §24.7: when every thread that originated in a program's initial
//! procedures has ended, the run ends by an implicit `$finish`. A program's
//! initial procedures ending also end its other threads: `pa`'s
//! `fork ... join_none` child never prints. An initial procedure that ends at
//! time 0 does not end the run before the other programs, and a design whose
//! programs have no initial procedure runs to its own `$finish`. The run
//! used to go on to the time limit. Expected values are the reference
//! simulator's; the final procedures run in an arbitrary order (§9.2.3).

use xezim::simulate;

const ENDS: &str = r#"
program pa(input logic clk);
  initial begin
    fork
      begin #100 $display("T|child of pa at t=%0t (must not print)", $time); end
    join_none
    repeat (2) @(posedge clk);
    $display("T|pa done t=%0t", $time);
  end
endprogram
program pb;
  initial $display("T|pb done t=%0t", $time);
endprogram
program pc(input logic clk);
  initial begin
    repeat (3) @(posedge clk);
    $display("T|pc done t=%0t", $time);
  end
endprogram
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  pa a(clk);
  pb b();
  pc c(clk);
  initial #500 $display("T|module still running t=%0t (must not print)", $time);
  final $display("T|final t=%0t", $time);
endmodule
"#;

const NO_INITIAL: &str = r#"
program pn;
  final $display("T|pn final t=%0t", $time);
endprogram
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  pn n();
  initial #40 $finish;
  final $display("T|final t=%0t", $time);
endmodule
"#;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 10_000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn run_ends_when_every_program_has() {
    assert_eq!(
        lines(ENDS),
        [
            "T|pb done t=0",
            "T|pa done t=15",
            "T|pc done t=25",
            "T|final t=25",
        ]
    );
}

#[test]
fn programs_without_initial_procedures_do_not_end_the_run() {
    let mut got = lines(NO_INITIAL);
    got.sort();
    assert_eq!(got, ["T|final t=40", "T|pn final t=40"]);
}
