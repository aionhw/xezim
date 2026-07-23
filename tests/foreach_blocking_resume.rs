//! IEEE 1800-2023 §9.4.3: a process blocked at a procedural timing control
//! (`wait`/`#delay`) inside a `foreach` body (often nested several inlined
//! task frames deep — UVM's `foreach (siblings[sib]) sib.wait_for_state(…)`)
//! shall resume at the NEXT iteration, not by restarting the whole `foreach`
//! from index 0. The minimal shape: a `foreach` calling a task that performs
//! a CONSUMING side effect (queue pop) BEFORE a `wait(cond)` that is initially
//! FALSE. Replaying from index 0 would re-pop an already-empty queue.
//!
//! Reference (commercial reference simulator): parks at iter 0, resumes when
//! the flag is raised, then runs iters 1, 2 — queue ends empty.
use std::process::Command;

fn xezim() -> String {
    // CARGO_BIN_EXE_<name> resolves to the binary built for the ACTIVE
    // profile (debug under plain `cargo test`, release under `--release`),
    // unlike a hardcoded target/release path which breaks CI (issue #51).
    env!("CARGO_BIN_EXE_xezim").to_string()
}

fn run(src: &str, tag: &str) -> String {
    let path = format!("/tmp/fe_wait_{tag}.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", &path])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn foreach_task_with_false_wait_resumes_next_iter() {
    let src = r#"module top;
  int q[$];     // consumed before each wait
  int flag;
  int drv[$];

  task automatic do_iter(input int idx);
    int popped;
    popped = q.pop_front();           // consuming side effect before wait
    wait (flag >= 1);                 // FALSE until t=10 -> must park
  endtask

  initial begin
    drv.push_back(0); drv.push_back(1); drv.push_back(2);
    q.push_back(11); q.push_back(22); q.push_back(33);
    flag = 0;
    fork begin #10; flag = 1; end join_none
    foreach (drv[d]) do_iter(drv[d]);
    #1;
    $display("QSIZE %0d", q.size());
    if (q.size() == 0) $display("RESULT PASS"); else $display("RESULT FAIL");
    $finish;
  end
endmodule
"#;
    let out = run(src, "taskwait");
    assert!(out.contains("QSIZE 0"), "queue should be empty after 3 pops\n{out}");
    assert!(out.contains("RESULT PASS"), "expected PASS\n{out}");
}

#[test]
fn foreach_blocking_body_parks_each_iter() {
    // A blocking foreach body (direct #delay, no task) must advance one
    // iteration per resume and visit every element exactly once.
    let src = r#"module top;
  int arr[3];
  int seen[$];
  initial begin
    arr[0]=7; arr[1]=8; arr[2]=9;
    foreach (arr[i]) begin
      #5;
      seen.push_back(arr[i]);
    end
    #1;
    $display("SEEN %p", seen);
    if (seen.size()==3 && seen[0]==7 && seen[1]==8 && seen[2]==9)
      $display("RESULT PASS"); else $display("RESULT FAIL");
    $finish;
  end
endmodule
"#;
    let out = run(src, "blocking");
    assert!(out.contains("RESULT PASS"), "expected each element visited once\n{out}");
}
