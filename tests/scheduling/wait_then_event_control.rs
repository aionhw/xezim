//! IEEE 1800-2023 §9.7.4: `wait (c) @(posedge clk);` runs the event control
//! once `c` holds, and the process resumes at the next edge. When `c` already
//! held, xezim ran the event control through the synchronous statement path,
//! which cannot suspend the process, and the process never resumed. A
//! `wait` that parks first and is woken later took the same path. Verilator
//! 5.036 (`--binary --timing`) prints the expected lines.

use xezim::simulate;

const SRC: &str = r#"
module dut(input logic clk, input logic rst_n, input logic stall, output logic [1:0] resp);
  always_ff @(posedge clk or negedge rst_n)
    if (!rst_n) resp <= 2'b00;
    else resp <= stall ? 2'b10 : 2'b00;
endmodule
module top;
  logic clk = 0, rst_n = 0, stall = 0;
  always #5 clk = ~clk;
  dut u (.clk(clk), .rst_n(rst_n), .stall(stall), .resp());
  task automatic go();
    stall = 1;
    #20;
    // Already true: `resp` became 2'b10 at 15.
    wait (u.resp === 2'b10)
    @(posedge clk);
    $display("T|true|%0t", $time);
    #2 stall = 0;
    // False here; true once the edge at 45 commits `resp`.
    wait (u.resp === 2'b00) @(posedge clk);
    $display("T|parked|%0t", $time);
  endtask
  initial begin
    #12 rst_n = 1;
    go();
    $display("T|done|%0t", $time);
    $finish;
  end
  initial #300 begin $display("T|timeout|%b", u.resp); $finish; end
endmodule
"#;

#[test]
fn wait_runs_an_event_control_it_guards() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    let got: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    assert_eq!(got, ["T|true|35", "T|parked|55", "T|done|55"]);
}
