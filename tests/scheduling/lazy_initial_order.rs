//! IEEE 1800-2023 §9.2.1, §4.4.2, §23.6 (#1056): an instance's initial block
//! whose body is materialized only when its process starts keeps its place
//! among the time-0 processes, its scope for `%m`, its label as a `disable`
//! target and its automatic locals, beside the blocks that are materialized
//! at setup (a task call, an event or delay control, one statement). The
//! transcript is the reference simulator's.

#[test]
fn deferred_initial_blocks_keep_their_time0_order_and_scope() {
    let sim = xezim::simulate(
        r#"module leaf #(parameter int K = 0)();
  int acc;
  task automatic bump(int n); acc += n; endtask
  // pure assignments and system tasks
  initial begin
    acc = K;
    acc = acc + 1;
    $display("A %m acc=%0d", acc);
  end
  // calls a task
  initial begin
    bump(10);
    $display("B %m acc=%0d", acc);
  end
  // waits
  initial begin
    #1 $display("C %m acc=%0d t=%0t", acc, $time);
  end
  // labelled, with an automatic local and a loop
  initial begin : lbl
    automatic int s = 0;
    for (int i = 0; i < 4; i++) s += i;
    $display("D %m s=%0d", s);
  end
  // one statement
  initial $display("E %m K=%0d", K);
  // delay then more
  initial begin
    acc = acc + 100;
    #2;
    acc = acc + 1000;
    $display("F %m acc=%0d t=%0t", acc, $time);
  end
endmodule
module top;
  for (genvar j = 0; j < 2; j++) begin : g
    leaf #(.K(j)) u();
  end
  leaf #(.K(7)) v();
  initial begin
    $display("T0 %m");
    #5 $display("END g0=%0d g1=%0d v=%0d", g[0].u.acc, g[1].u.acc, v.acc);
    $finish;
  end
  initial begin : stopper
    #1 disable v.lbl;
    $display("T1 %m");
  end
endmodule
"#,
        1_000,
    )
    .expect("simulate failed");
    let got: Vec<&str> = sim.output.iter().map(|o| o.message.trim()).collect();
    let expected = [
        "A top.g[0].u acc=1",
        "B top.g[0].u acc=11",
        "D top.g[0].u.lbl s=6",
        "E top.g[0].u K=0",
        "A top.g[1].u acc=2",
        "B top.g[1].u acc=12",
        "D top.g[1].u.lbl s=6",
        "E top.g[1].u K=1",
        "A top.v acc=8",
        "B top.v acc=18",
        "D top.v.lbl s=6",
        "E top.v K=7",
        "T0 top",
        "C top.g[0].u acc=111 t=1",
        "C top.g[1].u acc=112 t=1",
        "C top.v acc=118 t=1",
        "T1 top.stopper",
        "F top.g[0].u acc=1111 t=2",
        "F top.g[1].u acc=1112 t=2",
        "F top.v acc=1118 t=2",
        "END g0=1111 g1=1112 v=1118",
    ];
    assert_eq!(got, expected);
}
