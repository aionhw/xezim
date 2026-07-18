//! `$deposit(target, value)` (Verilog-XL/VCS): set the target's value
//! immediately WITHOUT a persistent driver — it holds until the next driver
//! transaction overwrites it; on an undriven net it simply sticks. Previously
//! an unknown task: the deposit silently did nothing and the target stayed X.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 10_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Undriven output port: the deposited value must appear on the connected net.
#[test]
fn deposit_sticks_on_undriven_net_through_port() {
    let o = out(r#"
module d(output Q);
  initial $deposit(Q, 1'b1);
endmodule
module tb;
  wire q;
  d u(.Q(q));
  initial begin #0; $display("Q=%b", q); $finish; end
endmodule
"#);
    assert!(o.contains("Q=1"), "deposit must reach the net, got:\n{}", o);
}

/// Variables accept deposits, and a real driver overrides a deposit on a net.
#[test]
fn deposit_variable_and_driver_override() {
    let o = out(r#"
module t;
  reg src = 0; wire w; assign w = src;
  reg [3:0] v;
  initial begin
    $deposit(v, 4'hA);
    #1 $display("V=%h", v);
    $deposit(w, 1'b1);
    #0 $display("D=%b", w);
    src = 1; #1 src = 0; #1 $display("O=%b", w);
    $finish;
  end
endmodule
"#);
    assert!(o.contains("V=a"), "variable deposit:\n{}", o);
    assert!(o.contains("D=1"), "deposit on driven net visible:\n{}", o);
    assert!(o.contains("O=0"), "driver must override the deposit:\n{}", o);
}
