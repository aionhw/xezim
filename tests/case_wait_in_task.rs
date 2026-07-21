//! IEEE 1800-2023 §9.7.4 — a `wait` inside a `case` reached on the
//! suspend-aware (process) path must BLOCK until its condition is true, not
//! fall through.
//!
//! Root cause this guards: `run_process_stmts` (the suspend-aware statement
//! runner) had native arms for blocking constructs (Wait, If, While, …) but
//! none for `case`. When an inlined task body was a `case` whose matched arm
//! held a `wait` (UVM's `uvm_phase::wait_for_state`:
//! `case(op) UVM_EQ: wait((state & m_state) != 0);`), it fell through to the
//! synchronous `exec_statement(Case)`, where a false `wait` with no parking
//! continuation either silently returned or stranded the process — the waiter
//! never blocked on the (later-satisfied) condition. This made UVM cleanup
//! phases fire before the runtime sub-phases completed.
//!
//! The fix added a suspend-aware Case handler mirroring the If handler. These
//! tests pin the semantics: the chosen arm's `wait` must block, and a
//! non-blocking arm must still run and let execution continue.

use xezim::simulate;

fn lookup(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("top.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("signal {} not u64-able", name))
}

/// A `wait(cond)` inside the matched arm of a `case`, reached via an inlined
/// task on the suspend-aware path, must block until `cond` becomes true.
#[test]
fn case_arm_wait_blocks_until_condition_true() {
    let src = r#"
`timescale 1ns/1ns
module top;
  reg flag = 1'b0;
  // 0 = not yet proceeded; set to $time when the waiter passes its wait.
  integer proceeded_at = 0;

  // Task body is a `case` — exactly the shape of uvm_phase::wait_for_state.
  task automatic waiter(input integer sel);
    case (sel)
      1: begin
           wait(flag);              // must BLOCK (flag is 0 at the call)
           proceeded_at = $time;    // records when the waiter resumes
         end
      default: proceeded_at = 999;  // default must NOT run for sel==1
    endcase
  endtask

  initial waiter(1);        // blocks inside the case arm
  initial #10 flag = 1'b1;  // releases the waiter at t=10
  initial #25 $finish;
endmodule
"#;
    let sim = simulate(src, 30).expect("simulate failed");
    let proceeded_at = lookup(&sim, "proceeded_at");
    // Correct: the waiter blocks until flag rises at t=10, so proceeded_at==10
    // and the default arm (999) never ran. Buggy behaviour was proceeded_at==0
    // (the synchronous-path wait fell through at the call time t=0) or the
    // process stranding (stayed 0 forever) — both fail this assertion.
    assert_eq!(
        proceeded_at, 10,
        "wait inside a case arm must block until its condition is true (expected proceeded_at=10)"
    );
}

/// A case arm with NO blocking construct must run synchronously and let the
/// statements after the case execute (the suspend-aware handler must not
/// strand non-blocking arms).
#[test]
fn case_arm_nonblocking_runs_and_continues() {
    let src = r#"
`timescale 1ns/1ns
module top;
  integer a = 0;
  integer b = 0;

  task automatic pick(input integer sel);
    case (sel)
      1: a = 7;   // non-blocking arm
      2: b = 7;
    endcase
  endtask

  initial begin
    pick(1);
    b = b + 1;    // statement AFTER the task call must run
  end
  initial #5 $finish;
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert_eq!(lookup(&sim, "a"), 7, "matched case arm ran");
    assert_eq!(lookup(&sim, "b"), 1, "execution continued past the case/task");
}

/// The `default` arm of a case must be selected when no item matches, and if
/// that arm blocks it too must block on the suspend-aware path.
#[test]
fn case_default_arm_is_selected_and_blocks() {
    let src = r#"
`timescale 1ns/1ns
module top;
  reg flag = 1'b0;
  integer got = 0;

  task automatic waiter(input integer sel);
    case (sel)
      1: got = 111;
      default: begin wait(flag); got = 222; end
    endcase
  endtask

  initial waiter(5);        // no item matches → default arm
  initial #10 flag = 1'b1;
  initial #25 $finish;
endmodule
"#;
    let sim = simulate(src, 30).expect("simulate failed");
    // The default arm blocked on wait(flag) until t=10, then set got=222.
    assert_eq!(
        lookup(&sim, "got"), 222,
        "default arm selected and blocked until flag rose"
    );
}
