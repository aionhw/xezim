//! §9.4.3 / §15.5.3: a parked `wait(cond)` is re-evaluated only when
//! something it reads may have changed. The `condition_waiter_rechecks`
//! profile counter counts each parked waiter a drain reschedules; the ceilings
//! keep idle waiters from being re-checked on every time step again. Expected
//! T| lines were recorded from the reference simulator.
use std::process::Command;

fn run(src: &str) -> (Vec<String>, u64) {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_cond_gate_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, src).expect("write design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .env("XEZIM_PROFILE_TIMING", "1")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    let lines = text
        .lines()
        .filter(|l| l.starts_with("T|"))
        .map(str::to_string)
        .collect();
    let rechecks = text
        .lines()
        .find_map(|l| l.strip_prefix("[PROF] condition_waiter_rechecks="))
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or_else(|| panic!("missing condition_waiter_rechecks:\n{text}"));
    (lines, rechecks)
}

fn expected(s: &str) -> Vec<String> {
    s.lines().map(str::to_string).collect()
}

/// Fifty idle `wait(done.triggered)` waiters over 200 time steps, next to
/// waiters that must still wake on a blocking trigger, a trigger through a
/// handle, a `&&` with a property, and a deferred (`->>`) trigger; a negated
/// `!done.triggered` is not gated and does not wake (no operand changes).
const TRIGGERED_DESIGN: &str = r#"
`timescale 1ns/1ns
class box;
  event e;
endclass
class watcher;
  event done;
  box b = new();
  int woke, woke_handle, woke_and;
  bit go;
  task automatic park();
    fork
      begin wait (done.triggered); woke++; end
    join_none
  endtask
  task automatic watch_all();
    fork
      begin wait (done.triggered); woke++; end
      begin wait (b.e.triggered); woke_handle++; end
      begin wait (go && done.triggered); woke_and++; end
    join_none
  endtask
endclass
module top;
  watcher idle[$];
  watcher w = new();
  event nba_ev;
  int nba_hits, neg_hits, steps;
  initial begin
    for (int i = 0; i < 50; i++) begin
      automatic watcher x = new();
      idle.push_back(x);
      x.park();
    end
    w.watch_all();
    fork
      begin wait (nba_ev.triggered); nba_hits++; end
    join_none
  end
  initial begin
    #5 -> w.done;
    $display("T|t=%0t blocking woke=%0d and=%0d", $time, w.woke, w.woke_and);
    #0 $display("T|t=%0t blocking delta woke=%0d and=%0d", $time, w.woke, w.woke_and);
    #5 -> w.b.e;
    #0 $display("T|t=%0t handle=%0d", $time, w.woke_handle);
    #5 ->> nba_ev;
    #0 $display("T|t=%0t nba before=%0d", $time, nba_hits);
    #1 $display("T|t=%0t nba after=%0d", $time, nba_hits);
    #9 w.go = 1; -> w.done;
    #0 $display("T|t=%0t and=%0d", $time, w.woke_and);
    -> w.done;
    fork
      begin wait (!w.done.triggered); neg_hits++; end
    join_none
    #0 $display("T|t=%0t neg same slot=%0d", $time, neg_hits);
    #1 $display("T|t=%0t neg next slot=%0d", $time, neg_hits);
  end
  initial begin
    for (steps = 0; steps < 200; steps++) #1;
    begin
      automatic int idle_woke = 0;
      foreach (idle[i]) idle_woke += idle[i].woke;
      $display("T|end steps=%0d idle_woke=%0d", steps, idle_woke);
    end
    $finish;
  end
endmodule
"#;

const TRIGGERED_EXPECTED: &str = "\
T|t=5 blocking woke=0 and=0
T|t=5 blocking delta woke=1 and=0
T|t=10 handle=1
T|t=15 nba before=0
T|t=16 nba after=1
T|t=25 and=1
T|t=25 neg same slot=0
T|t=26 neg next slot=0
T|end steps=200 idle_woke=0";

#[test]
fn triggered_waits_recheck_only_on_triggers() {
    let (lines, rechecks) = run(TRIGGERED_DESIGN);
    assert_eq!(lines, expected(TRIGGERED_EXPECTED));
    // Ungated, every idle waiter was re-checked on each drain (~30500).
    assert!(rechecks <= 1000, "condition_waiter_rechecks={rechecks}");
}

/// Fifty idle class waiters on a queue property, an int property and a
/// static queue, while a module loop writes block variables on every step.
const MODULE_WRITES_DESIGN: &str = r#"
`timescale 1ns/1ns
class watcher;
  int m_q[$];
  int count;
  static int sq[$];
  int woke_q, woke_count, woke_static;
  task automatic watch();
    fork
      begin wait (m_q.size() != 0); woke_q = $time; end
      begin wait (count == 3); woke_count = $time; end
      begin wait (sq.size() > 1); woke_static = $time; end
    join_none
  endtask
endclass
module top;
  watcher idle[$];
  watcher w = new();
  int steps;
  initial begin
    for (int i = 0; i < 50; i++) begin
      automatic watcher x = new();
      idle.push_back(x);
      x.watch();
    end
    w.watch();
  end
  initial begin
    longint acc;
    int last;
    acc = 0;
    for (steps = 0; steps < 200; steps++) begin
      acc += steps;
      last = steps;
      #1;
    end
    $display("T|end acc=%0d last=%0d", acc, last);
    $finish;
  end
  initial begin
    #20 w.m_q.push_back(7);
    #0 $display("T|t=%0t q=%0d", $time, w.woke_q);
    #10 w.count = 2;
    #10 w.count++;
    #0 $display("T|t=%0t count=%0d", $time, w.woke_count);
    #10 watcher::sq.push_back(1);
    #0 $display("T|t=%0t static one=%0d", $time, w.woke_static);
    #10 watcher::sq.push_back(2);
    #0 $display("T|t=%0t static two=%0d idle0=%0d", $time, w.woke_static, idle[0].woke_static);
  end
endmodule
"#;

const MODULE_WRITES_EXPECTED: &str = "\
T|t=20 q=20
T|t=40 count=40
T|t=50 static one=0
T|t=60 static two=60 idle0=60
T|end acc=19900 last=199";

#[test]
fn module_variable_writes_do_not_recheck_class_waiters() {
    let (lines, rechecks) = run(MODULE_WRITES_DESIGN);
    assert_eq!(lines, expected(MODULE_WRITES_EXPECTED));
    // Each module write noted every name: every idle waiter re-checked on
    // every step (~13500).
    assert!(rechecks <= 1500, "condition_waiter_rechecks={rechecks}");
}

/// Fifty idle `wait(is_reset == 1)` waiters while a method taking a virtual
/// interface formal is called on every step. Binding and releasing the
/// formal writes no variable a parked wait reads.
const VIF_FORMAL_CALLS_DESIGN: &str = r#"
`timescale 1ns/1ns
interface bus_if;
  logic [7:0] data;
endinterface
class driver;
  bit is_reset;
  int woke;
  int sum;
  task automatic watch();
    fork
      begin wait (is_reset == 1); woke = $time; end
    join_none
  endtask
  function automatic int peek(virtual bus_if v, int k);
    return v.data + k;
  endfunction
endclass
module top;
  bus_if bif();
  driver idle[$];
  driver d = new();
  int steps;
  initial begin
    for (int i = 0; i < 50; i++) begin
      automatic driver x = new();
      idle.push_back(x);
      x.watch();
    end
    d.watch();
  end
  initial begin
    bif.data = 3;
    for (steps = 0; steps < 200; steps++) begin
      d.sum += d.peek(bif, steps);
      #1;
    end
    $display("T|end sum=%0d idle0=%0d d=%0d", d.sum, idle[0].woke, d.woke);
    $finish;
  end
  initial begin
    #50 d.is_reset = 1;
    #0 $display("T|t=%0t reset=%0d", $time, d.woke);
  end
endmodule
"#;

const VIF_FORMAL_CALLS_EXPECTED: &str = "\
T|t=50 reset=50
T|end sum=20500 idle0=0 d=50";

#[test]
fn virtual_interface_formals_do_not_recheck_waiters() {
    let (lines, rechecks) = run(VIF_FORMAL_CALLS_DESIGN);
    assert_eq!(lines, expected(VIF_FORMAL_CALLS_EXPECTED));
    // Each call's formal bind and release noted every name: every idle
    // waiter was re-checked twice per step (~10050).
    assert!(rechecks <= 1000, "condition_waiter_rechecks={rechecks}");
}
