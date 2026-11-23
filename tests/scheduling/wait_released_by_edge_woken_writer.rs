//! IEEE 1800-2023 §4.4.2, §4.5, §9.7.4: a blocking write by a process a clock
//! edge woke makes a `wait` on the written variable true in the Active region,
//! so the waiter resumes there, before the time step's NBA region, and a
//! process the NBAs wake sees what the waiter did. The waiter stayed parked
//! until the end-of-step drain when the writer resumed from an edge (or from
//! an NBA) instead of a delay, and the NBA-woken process ran first (#1507).
//! Expected values were checked on the reference simulator.

use xezim::simulate;

fn run(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

#[test]
fn issue_repro_waiter_runs_before_the_nba() {
    let got = run(r#"
module top;
  logic clk = 0;
  initial forever #5 clk = ~clk;
  logic go = 0, nb = 0;
  int fifo_count = 0;
  initial begin
    @(posedge clk);
    @(posedge clk);
    go = 1;
    nb <= 1;
    @(nb);
    $display("T|%0t|fifo=%0d", $time, fifo_count);
  end
  initial begin
    @(posedge clk);
    wait (go);
    fifo_count = 1;
  end
  initial #40 $finish;
endmodule
"#);
    assert_eq!(got, ["T|15|fifo=1"]);
}

#[test]
fn compound_conditions_nba_woken_writers_and_clocked_counters() {
    let got = run(r#"
module top;
  logic clk = 0;
  initial forever #5 clk = ~clk;
  logic go = 0, en = 1, nb = 0, nb1 = 0, nb2 = 0, go2 = 0;
  int fifo = 0, f2 = 0, f3 = 0, cnt = 0, f4 = 0, f5 = 0;
  initial begin
    @(posedge clk); @(posedge clk);
    go = 1; nb <= 1; @(nb);
    $display("T|a %0t fifo=%0d f4=%0d", $time, fifo, f4);
  end
  initial begin @(posedge clk); wait (go && en); fifo = 1; end
  initial begin
    @(posedge clk); @(posedge clk); @(posedge clk);
    nb1 <= 1;
    @(nb1);
    go2 = 1;
    nb2 <= 1;
    @(nb2);
    $display("T|b %0t f2=%0d", $time, f2);
  end
  initial begin wait (go2); f2 = 1; end
  always @(posedge clk) cnt = cnt + 1;
  initial begin wait (cnt == 3); f3 = $time; end
  initial begin wait (cnt == 100); f5 = 1; end
  initial begin @(posedge clk); wait (go === 1'b1); f4 = 1; end
  initial begin #60 $display("T|c f3=%0d f5=%0d cnt=%0d", f3, f5, cnt); $finish; end
endmodule
"#);
    assert_eq!(
        got,
        ["T|a 15 fifo=1 f4=1", "T|b 25 f2=1", "T|c f3=25 f5=0 cnt=6",]
    );
}

#[test]
fn a_clocked_always_writer_releases_the_waiters_before_the_nba() {
    let got = run(r#"
module top;
  logic clk = 0, rst = 1;
  always #5 clk = ~clk;
  logic go = 0, nb = 0;
  logic [3:0] v = 0;
  int fifo = 0, g = 0;
  always @(posedge clk) if (!rst) begin
    v = v + 1;
    if (v == 2) begin go = 1; nb <= 1; end
  end
  always @(nb) $display("T|nb %0t fifo=%0d g=%0d", $time, fifo, g);
  initial begin wait (go); fifo = 1; end
  initial begin wait (v[1]); g = 1; end
  initial begin #12 rst = 0; #40 $finish; end
endmodule
"#);
    assert_eq!(got, ["T|nb 25 fifo=1 g=1"]);
}
