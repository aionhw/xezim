//! A variable-period clock `always #(expr) clk = ~clk;` whose period `expr`
//! is a continuous assign fed by an `initial`-seeded variable must NOT emit a
//! spurious edge at time 0. The clock process (classified during setup, so a
//! low pid) runs before the `initial` block; at that first `#(expr)` the period
//! signal is still 0 (cont-assign unsettled), so the delay evaluated to `#0`
//! and toggled at t=0 — inverting the clock phase for the ENTIRE run. A later
//! strobe then sampled the wrong half-cycle. The fix re-evaluates the first
//! time-0 `#(expr)` after the time-0 settle. Found debugging a behavioral PLL
//! whose config latch (clocked by exactly such a vco clock) never captured.

use xezim::simulate;

fn output_of(sim: &xezim::compiler::Simulator) -> String {
    sim.output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn variable_delay_clock_no_spurious_t0_edge() {
    // period p=1000 seeded by an initial block; half=p/2 via cont-assign feeds
    // the clock delay. Correct phase: first toggle 0->1 at t=500, ->0 at 1000,
    // ->1 at 1500. So clk==1 at t=750 and clk==0 at t=1250. The t=0-toggle bug
    // inverts this (clk==0 at 750, ==1 at 1250).
    const SRC: &str = r#"
`timescale 1ps/1ps
module top;
  logic clk = 0;
  real p; initial p = 1000.0;
  real half; assign half = p / 2.0;
  always #(half) clk = ~clk;
  initial begin
    #750;  $display("AT750 clk=%b", clk);
    #500;  $display("AT1250 clk=%b", clk);
    $finish;
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 5000).expect("sim"));
    assert!(
        out.contains("AT750 clk=1"),
        "phase inverted at t=750:\n{}",
        out
    );
    assert!(
        out.contains("AT1250 clk=0"),
        "phase inverted at t=1250:\n{}",
        out
    );
}

#[test]
fn timing_wheel_finds_same_word_event_after_wrap() {
    // At t=250, scheduling t=500 places the event in slot 244. Both slots are
    // in bitmap word 3, but slot 244 is below the scan start at slot 250. The
    // wrapped portion of the start word must still be searched.
    const SRC: &str = r#"
`timescale 1ps/1ps
module top;
  initial begin
    #250; $display("AT250");
    #250; $display("AT500");
    $finish;
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 1000).expect("sim"));
    assert!(out.contains("AT250"), "first event was lost:\n{}", out);
    assert!(out.contains("AT500"), "wrapped event was lost:\n{}", out);
}

#[test]
fn task_local_real_array_survives_edge_waits() {
    // This is the period-measurement shape used by the PLL testbench. Both the
    // task frame and its local unpacked array must survive each event wait.
    const SRC: &str = r#"
`timescale 1ps/1ps
module top;
  logic clk = 0;
  real measured;

  always #5 clk = ~clk;

  task capture(output real result);
    integer i;
    real samples[0:2];
    begin
      for (i = 0; i < 3; i = i + 1) begin
        @(posedge clk);
        samples[i] = $realtime;
      end
      result = samples[2] - samples[1];
    end
  endtask

  initial begin
    capture(measured);
    $display("MEASURED=%0.3f", measured);
    $finish;
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 1000).expect("sim"));
    assert!(
        out.contains("MEASURED=10.000"),
        "task-local samples were lost across suspension:\n{}",
        out
    );
}

#[test]
fn task_edge_wait_advances_on_continuous_assign_alias() {
    const SRC: &str = r#"
`timescale 1ps/1ps
module top;
  logic clk = 0;
  wire out;
  real measured;

  assign out = clk;
  always #5 clk = ~clk;

  task capture(output real result);
    integer i;
    real samples[0:2];
    begin
      for (i = 0; i < 3; i = i + 1) begin
        @(posedge out);
        samples[i] = $realtime;
      end
      result = samples[2] - samples[1];
    end
  endtask

  initial begin
    capture(measured);
    $display("ALIASED=%0.3f", measured);
    $finish;
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 1000).expect("sim"));
    assert!(
        out.contains("ALIASED=10.000"),
        "each wait must consume a distinct propagated edge:\n{}",
        out
    );
}

#[test]
fn nested_task_delay_observes_edges_from_dynamic_clock_process() {
    const SRC: &str = r#"
module capture(output logic q, input logic clk, input logic d, input logic we);
  always_ff @(posedge clk) if (we) q <= d;
endmodule
module mirror(input wire we, output wire copy);
  assign copy = we;
endmodule
module top;
  logic clk = 0;
  logic we = 0;
  logic q;
  real half_period = 5.0;
  wire copy;
  capture dut(q, clk, 1'b1, we);
  mirror passthrough(we, copy);

  always #(half_period) clk = ~clk;

  task apply_write_enable;
    begin
      we = 1;
      #20;
      we = 0;
    end
  endtask

  task outer;
    begin
      apply_write_enable();
    end
  endtask

  initial begin
    outer();
    #1 $display("NESTED we=%b copy=%b q=%b", we, copy, q);
    $finish;
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 100).expect("sim"));
    assert!(
        out.contains("NESTED we=0 copy=0 q=1"),
        "nested task delays must dispatch edges from dynamic clock processes:\n{}",
        out
    );
}

#[test]
fn resumed_task_can_call_event_waiting_task() {
    const SRC: &str = r#"
`timescale 1ps/1ps
module top;
  logic clk = 0;
  real half_period = 5.0;
  real measured;

  always #(half_period) clk = ~clk;

  task capture(output real result);
    integer i;
    real samples[0:2];
    begin
      for (i = 0; i < 3; i = i + 1) begin
        @(posedge clk);
        samples[i] = $realtime;
      end
      result = samples[2] - samples[1];
    end
  endtask

  task outer;
    begin
      #20;
      capture(measured);
      $display("RESUMED=%0.3f", measured);
    end
  endtask

  initial begin
    repeat (1) outer();
    $finish;
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 1000).expect("sim"));
    assert!(
        out.contains("RESUMED=10.000"),
        "a resumed task must suspend again in its nested event-waiting task:\n{}",
        out
    );
}

#[test]
fn edge_block_tracks_variable_delay_clock_after_period_change() {
    const SRC: &str = r#"
`timescale 1ps/1ps
module capture(output logic q = 0, input logic clk, input logic we);
  always_ff @(posedge clk) if (we) q <= 1'b1;
endmodule
module top;
  logic clk = 0;
  logic we = 0;
  logic q;
  real half;
  capture dut(q, clk, we);

  initial half = 10.0;
  always #(half) clk = ~clk;

  task reconfigure;
    begin
      half = 3.0;
      we = 1;
      #20;
      we = 0;
    end
  endtask

  initial begin
    #45;
    reconfigure();
    #1 $display("CHANGED q=%b", q);
    $finish;
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 1000).expect("sim"));
    assert!(
        out.contains("CHANGED q=1"),
        "edge block stopped after the clock period changed:\n{}",
        out
    );
}

#[test]
fn conditional_dynamic_clock_rearms_with_the_updated_delay() {
    // This is the behavioral-PLL clock shape: a dynamic real delay and a
    // conditional clamp on the assignment. A delay already in flight keeps
    // its original deadline; the new value is sampled when the loop re-arms.
    const SRC: &str = r#"
`timescale 1ps/1ps
module top;
  logic clk = 0;
  logic halt = 0;
  real half = 10.0;
  integer edges = 0;

  always #(half) clk = halt ? 1'b0 : ~clk;
  always @(posedge clk) edges = edges + 1;

  initial begin
    #25 half = 4.0;
    #18 halt = 1;
    #12;
    $display("PLL_SHAPE t=%0t clk=%b edges=%0d", $time, clk, edges);
    $finish;
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 100).expect("sim"));
    assert!(
        out.contains("PLL_SHAPE t=55 clk=0 edges=3"),
        "conditional dynamic clock used the wrong scheduling semantics:\n{}",
        out
    );
}

#[test]
fn shared_clock_port_fanout_delivers_every_child_edge() {
    const SRC: &str = r#"
`timescale 1ps/1ps
module capture(output logic q = 0, input logic clk);
  always_ff @(posedge clk) q <= 1;
endmodule
module top;
  logic clk = 0;
  wire [5:0] q;

  capture c0(q[0], clk);
  capture c1(q[1], clk);
  capture c2(q[2], clk);
  capture c3(q[3], clk);
  capture c4(q[4], clk);
  capture c5(q[5], clk);

  always #5 clk = ~clk;
  initial begin
    #6;
    $display("FANOUT q=%b", q);
    $finish;
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 100).expect("sim"));
    assert!(
        out.contains("FANOUT q=111111"),
        "shared clock fanout lost a child edge:\n{}",
        out
    );
}
