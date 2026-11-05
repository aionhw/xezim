//! §10.3.2 / §13.4: a continuous assignment calling a function is
//! re-evaluated when one of its operands changes, not whenever any net
//! changes. The callee's name was collected as a read; it never named a
//! signal, so the entry counted as having unresolved reads and was
//! re-evaluated on every settle. With port connections deferred to the
//! next full settle, a change of an unrelated port re-ran it, and `$time`
//! in the callee read the later time. Expected values follow the reference
//! simulator.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100_000_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

#[test]
fn assign_with_call_ignores_unrelated_port_changes() {
    let src = r#"
`timescale 1ns/1ps
module top;
  logic b, clk;
  logic [63:0] ca, cb;
  int nb;
  function automatic logic [63:0] now(input logic x); return $time; endfunction
  function automatic int plus(input int v); return v + 1; endfunction
  assign ca = now(b);
  assign cb = plus(nb);
  subb ub(.b(b));
  subc uc(.clk(clk));
  initial begin
    nb = 3;
    #20 $display("T|ca=%0d cb=%0d", ca, cb);
    nb = 8;
    #1 $display("T|cb=%0d", cb);
  end
endmodule
module subb(output logic b);
  initial begin b = 0; #5.5 b = 1; end
endmodule
module subc(output logic clk);
  initial begin clk = 0; #6.5 clk = 1; #3 clk = 0; end
endmodule
"#;
    assert_eq!(lines(src), vec!["T|ca=6 cb=4", "T|cb=9"]);
}
