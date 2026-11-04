//! §6.21, §12.7.1: a fixed-size array declared in a procedural block keeps
//! the element writes a `for (int i ...)` loop makes. A loop the process
//! compiled to bytecode stored each element through the signal table, where
//! a block-local array has no cells, and the write was dropped: every shape
//! below read 0 or x. Blocking, compound and nonblocking writes; `int`,
//! `byte`, `logic [7:0]` and packed-struct elements; the block's top level, a
//! nested block, a fork child and a static task, across `#0`. Expected values
//! are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  typedef struct packed { logic [3:0] hi; logic [3:0] lo; } ps_t;
  task static fill_task(output int s);
    int ta[4];
    for (int i = 0; i < 4; i++) ta[i] = i + 5;
    #0;
    s = ta[0] + ta[1] + ta[2] + ta[3];
  endtask
  initial begin
    int a[4];
    byte b[4];
    logic [7:0] l[4];
    ps_t p[2];
    int n[4];
    int sum;
    for (int i = 0; i < 4; i++) a[i] = i + 1;
    for (int i = 0; i < 4; i++) a[i] += 10;
    for (int i = 0; i < 4; i++) b[i] = -i;
    for (int i = 0; i < 4; i++) l[i] = 8'h10 + i;
    for (int i = 0; i < 2; i++) p[i] = '{hi: i + 1, lo: i + 2};
    for (int i = 0; i < 4; i++) n[i] <= i * 3;
    #1;
    $display("T|top %p %p %0d %0d %0d %0d %0d %p", a, b, l[0], l[3], p[0].hi, p[1].hi, p[1].lo, n);
    begin
      int c[3];
      for (int i = 0; i < 3; i++) c[i] = 7 * i;
      #0;
      $display("T|nested %p", c);
    end
    fork
      begin
        int d[3];
        for (int i = 0; i < 3; i++) d[i] = 2 + i;
        #0;
        $display("T|fork %p", d);
      end
    join
    fill_task(sum);
    $display("T|task %0d", sum);
  end
endmodule
"#;

#[test]
fn for_loop_writes_reach_block_local_arrays() {
    let lines: Vec<String> = simulate(SRC, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|top '{11, 12, 13, 14} '{0, -1, -2, -3} 16 19 1 2 3 '{0, 3, 6, 9}",
            "T|nested '{0, 7, 14}",
            "T|fork '{2, 3, 4}",
            "T|task 26",
        ]
    );
}
