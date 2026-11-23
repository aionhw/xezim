//! IEEE 1800-2023 §7.10.2.4, §7.10.2.5, §12.7.1: a `for` loop whose body
//! stores `q.pop_front()` or `q.pop_back()` into an element of a packed or
//! unpacked array pops one element per iteration, in order, with a bound
//! that is not constant, and an empty queue pops the element type's default.
//! Such a loop of an `always` block is now compiled (the pop is evaluated
//! by the interpreter into a register) instead of running whole in the AST
//! interpreter (#1210). Expected lines were recorded from the reference
//! simulator.

#[test]
fn queue_pop_drain_loops_store_each_element_in_order() {
    let src = r#"
module top;
  logic [63:0] beat_q [$];
  int          iq [$];
  bit [15:0][63:0] frame_buf;
  logic [7:0]  sm8 [0:7];
  int          acc, n;
  logic clk = 0;
  always #5 clk = ~clk;
  // producer
  always @(posedge clk) begin
    for (int k = 0; k < 6; k++) beat_q.push_back({16'hA5A5, 32'(n), 16'(k)});
    for (int k = 0; k < 4; k++) iq.push_back(n * 10 + k);
    n++;
  end
  // drain: packed-array element from a queue pop, bound not constant
  always @(posedge clk) begin
    int cnt;
    cnt = beat_q.size() > 4 ? 4 : beat_q.size();
    for (int i = 0; i < cnt; i++) begin
      frame_buf[i] = beat_q.pop_front();
    end
    for (int i = 0; i < 2; i++) frame_buf[15 - i] = beat_q.pop_back();
    for (int i = 0; i < 3; i++) sm8[i] = iq.pop_front();
    for (int i = 7; i > 5; i--) sm8[i] = iq.pop_back();
    $display("T|%0t f0=%h f3=%h f15=%h f14=%h s=%0d,%0d,%0d,%0d,%0d bq=%0d iq=%0d",
             $time, frame_buf[0], frame_buf[3], frame_buf[15], frame_buf[14],
             sm8[0], sm8[1], sm8[2], sm8[6], sm8[7], beat_q.size(), iq.size());
  end
  initial begin
    repeat (4) @(negedge clk);
    $finish;
  end
endmodule
"#;
    let lines: Vec<String> = xezim::simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|5 f0=a5a5000000000000 f3=a5a5000000000003 f15=a5a5000000000005 f14=a5a5000000000004 s=0,1,2,0,3 bq=0 iq=0",
            "T|15 f0=a5a5000000010000 f3=a5a5000000010003 f15=a5a5000000010005 f14=a5a5000000010004 s=10,11,12,0,13 bq=0 iq=0",
            "T|25 f0=a5a5000000020000 f3=a5a5000000020003 f15=a5a5000000020005 f14=a5a5000000020004 s=20,21,22,0,23 bq=0 iq=0",
            "T|35 f0=a5a5000000030000 f3=a5a5000000030003 f15=a5a5000000030005 f14=a5a5000000030004 s=30,31,32,0,33 bq=0 iq=0"
        ]
    );
}

#[test]
fn compiled_queue_pop_resolves_each_instance_queue() {
    let source = r#"
module worker(input bit clk);
  int q[$];
  int result[2];
  always @(posedge clk) begin
    q.push_back(7);
    q.push_back(9);
    for (int i = 0; i < 2; i++) result[i] = q.pop_front();
  end
endmodule
module top;
  bit clk;
  int q[$];
  worker w(clk);
  initial begin
    q.push_back(31);
    #1 clk = 1;
    #1 $display("RESULT %0d %0d %0d %0d", w.result[0], w.result[1], w.q.size(), q[0]);
    $finish;
  end
endmodule
"#;
    let dir = std::env::temp_dir().join(format!("xezim_queue_pop_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, source).expect("write queue design");
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "--no-cache"])
        .arg(&path)
        .env("XEZIM_BC_DUMP", "1")
        .env_remove("XEZIM_BC_DUMP_FULL")
        .output();
    let _ = std::fs::remove_dir_all(&dir);
    let run = run.expect("run xezim");
    let output = String::from_utf8_lossy(&run.stdout);
    let trace = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "{output}\n{trace}");
    assert!(
        output.lines().any(|line| line == "RESULT 7 9 0 31"),
        "{output}"
    );
    assert!(
        trace
            .lines()
            .any(|line| line.starts_with("[BC-DUMP]") && line.contains("QueuePop")),
        "{trace}"
    );
}
