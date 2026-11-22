//! IEEE 1800-2023 §9.3.2, §4.4.2: the children a fork starts run in the
//! Active region of the time slot it runs in, before that slot's NBA region.
//! A child that waits for an edge an NBA of the same slot makes catches it.
//!
//! A process an event control resumed (`@(posedge clk)`, `@e`) ran inline
//! in the edge cascade, which committed the slot's NBAs before it ran the
//! children that process had just queued. They started after the edge and
//! waited for the next one, so each of these timed out or never reported.

use xezim::simulate;

const SRC: &str = r#"
module top;
  reg clk = 0;
  reg s1 = 0, s2 = 0, s3 = 0, s4 = 0, s5 = 0;
  event e;
  always #5 clk = ~clk;
  initial #5 -> e;
  // Woken by an edge, then a join_any with a timeout.
  initial begin
    @(posedge clk);
    s1 <= 1;
    fork
      @(posedge s1) $display("T|edge|%0t", $time);
      #20 $display("T|edge|timeout");
    join_any
    disable fork;
  end
  // Woken by a named event.
  initial begin
    @e;
    s2 <= 1;
    fork
      @(posedge s2) $display("T|event|%0t", $time);
      #20 $display("T|event|timeout");
    join_any
    disable fork;
  end
  // A join_none child starts when the parent next blocks.
  initial begin
    @(posedge clk);
    s3 <= 1;
    fork
      @(posedge s3) $display("T|none|%0t", $time);
    join_none
    #20;
  end
  // The fork inside a task.
  task automatic watch();
    fork
      @(posedge s4) $display("T|task|%0t", $time);
    join_none
  endtask
  initial begin
    @(posedge clk);
    s4 <= 1;
    watch();
  end
  // A child whose `#0` continuation forks again: the Active and Inactive
  // regions alternate until both are empty, then the NBAs commit.
  initial begin
    @(posedge clk);
    s5 <= 1;
    fork
      begin
        #0;
        fork
          @(posedge s5) $display("T|nested|%0t", $time);
        join_none
      end
    join
  end
  initial #40 $finish;
endmodule
"#;

/// With another process's NBA pending at the edge, the edge's processes run
/// before the slot's NBAs rather than in the edge cascade. Their fork
/// children, and those of a `#0` continuation, start there too.
const PENDING: &str = r#"
module top;
  reg clk = 0, s1 = 0, s2 = 0, other = 0;
  always #5 clk = ~clk;
  initial #5 other <= 1;
  initial begin
    @(posedge clk);
    s1 <= 1;
    fork
      @(posedge s1) $display("T|pending|%0t", $time);
    join_none
  end
  initial begin
    @(posedge clk);
    s2 <= 1;
    fork
      begin
        #0;
        fork
          @(posedge s2) $display("T|pending_nested|%0t", $time);
        join_none
      end
    join
  end
  initial #20 $finish;
endmodule
"#;

#[test]
fn fork_children_of_an_event_woken_process_run_before_the_nbas() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    // Same-time reports of different processes come in no set order.
    let mut got: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    got.sort();
    assert_eq!(
        got,
        [
            "T|edge|5",
            "T|event|5",
            "T|nested|5",
            "T|none|5",
            "T|task|5"
        ]
    );
}

#[test]
fn fork_children_run_before_the_nbas_when_an_nba_is_pending_at_the_edge() {
    let sim = simulate(PENDING, 1000).expect("simulate failed");
    let mut got: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    got.sort();
    assert_eq!(got, ["T|pending_nested|5", "T|pending|5"]);
}
