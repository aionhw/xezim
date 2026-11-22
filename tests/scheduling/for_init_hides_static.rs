//! IEEE 1800-2023 §6.21, §12.7.1: a variable declared in a for-init is
//! automatic and local to the loop. In a task whose own (static) variable
//! has the same name, the loop's reads and writes must reach the loop
//! variable. They reached the task's static cell instead: the condition read
//! the static (x, never assigned), and the loop ran no iteration, with and
//! without a timing control in its body. A later initializer
//! (`k = j`) must read the loop's `j` too. Hiding the static and showing it
//! again must keep the task's `j` bound to its shared cell when a block
//! around the loop declares its own static (`m_order`'s second call reads
//! the 3 its first call left). The same holds for a block variable that
//! hides the static inside such a block (`m_blk`). Verilator 5.036
//! (`--binary --timing`) prints the expected lines.

use xezim::simulate;

const SRC: &str = r#"
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  task m(input integer n);
    integer j;
    begin
      for (integer j = 0; j < n; j++) begin
        $display("T|static|j=%0d", j);
      end
    end
  endtask
  task m_timed(input integer n);
    integer j;
    begin
      @(posedge clk);
      for (integer j = 0; j < n; j++) begin
        @(posedge clk);
        $display("T|timed|j=%0d|%0t", j, $time);
      end
    end
  endtask
  // The task's own `j` keeps its value across the loop.
  task m_keep(input integer n);
    integer j;
    begin
      j = 7;
      for (integer j = 0; j < n; j++) $display("T|keep|j=%0d", j);
      $display("T|keep|after=%0d", j);
    end
  endtask
  task m_two(input integer n);
    integer j;
    for (integer j = 0, k = j; k < n; k++) $display("T|two|k=%0d", k);
  endtask
  task m_order(input integer n, input bit show);
    integer j;
    begin
      if (show) $display("T|order|j=%0d", j);
      begin
        static integer k;
        for (integer j = 0; j < n; j++) k = j;
      end
      j = 3;
    end
  endtask
  task m_blk(input bit show);
    integer j;
    begin
      if (show) $display("T|blk|j=%0d", j);
      begin
        static integer k;
        begin
          integer j;
          j = 1;
          k = j;
        end
      end
      j = 3;
    end
  endtask
  initial begin
    m(2);
    m_timed(2);
    m_keep(2);
    m_two(2);
    m_order(2, 0);
    m_order(2, 1);
    m_blk(0);
    m_blk(1);
    $display("T|done|%0t", $time);
    $finish;
  end
endmodule
"#;

#[test]
fn for_init_variable_hides_a_static_task_variable() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    let got: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    assert_eq!(
        got,
        [
            "T|static|j=0",
            "T|static|j=1",
            "T|timed|j=0|15",
            "T|timed|j=1|25",
            "T|keep|j=0",
            "T|keep|j=1",
            "T|keep|after=7",
            "T|two|k=0",
            "T|two|k=1",
            "T|order|j=3",
            "T|blk|j=3",
            "T|done|25",
        ]
    );
}
