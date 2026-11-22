//! IEEE 1800-2023 §6.21, §12.7.3: a `foreach` loop variable is automatic and
//! local to the loop, so it hides the task's same-named static variable, as
//! a for-init variable does. The loop read the task's static cell (x, never
//! assigned) instead, with and without a timing control in the body. The
//! task's own `j` keeps what the task wrote after the loop, also when the
//! loop sits in a block that declares its own static. The reference
//! simulator prints the expected lines.

use xezim::simulate;

const SRC: &str = r#"
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  int a [3] = '{10, 20, 30};
  int b [2][2];
  task t_sync();
    integer j;
    foreach (a[j]) $display("T|sync|j=%0d", j);
  endtask
  task t_timed();
    integer j;
    foreach (a[j]) begin
      @(posedge clk);
      $display("T|timed|j=%0d|%0t", j, $time);
    end
  endtask
  task t_keep(input bit show);
    integer j;
    if (show) $display("T|keep|j=%0d", j);
    foreach (a[j]) if (j == 2) $display("T|keep|last=%0d", j);
    j = 5;
  endtask
  task t_two();
    integer i, j;
    foreach (b[i, j]) $display("T|two|%0d%0d", i, j);
  endtask
  task t_order(input bit show);
    integer j;
    if (show) $display("T|order|j=%0d", j);
    begin
      static integer k;
      foreach (a[j]) k = j;
    end
    j = 3;
  endtask
  task t_timed_keep(input bit show);
    integer j;
    if (show) $display("T|tkeep|j=%0d", j);
    foreach (a[j]) @(posedge clk);
    j = 8;
  endtask
  initial begin
    t_sync();
    t_timed();
    t_keep(0); t_keep(1);
    t_two();
    t_order(0); t_order(1);
    t_timed_keep(0); t_timed_keep(1);
    $display("T|done|%0t", $time);
    $finish;
  end
endmodule
"#;

#[test]
fn foreach_variable_hides_a_task_static() {
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
            r#"T|sync|j=0"#,
            r#"T|sync|j=1"#,
            r#"T|sync|j=2"#,
            r#"T|timed|j=0|5"#,
            r#"T|timed|j=1|15"#,
            r#"T|timed|j=2|25"#,
            r#"T|keep|last=2"#,
            r#"T|keep|j=5"#,
            r#"T|keep|last=2"#,
            r#"T|two|00"#,
            r#"T|two|01"#,
            r#"T|two|10"#,
            r#"T|two|11"#,
            r#"T|order|j=3"#,
            r#"T|tkeep|j=8"#,
            r#"T|done|85"#,
        ]
    );
}
