//! A blocking STATIC task reached through a bare TYPEDEF receiver
//! (`db_t::wait_modified`, `typedef db_impl#(bit) db_t;`) must be recognized
//! as blocking, inlined into the suspend-aware runner, parked on its event,
//! and eventually woken — and the body's static-collection accesses must route
//! to the SAME per-specialization store the `Class#(params)::` spelling uses.
//!
//! This is the pure-SV shape of UVM `uvm_config_db`'s `wait_modified` / `set`
//! mechanism. The instance spelling under test (`db_t::wait_modified` — a bare
//! typedef alias receiver) reaches Stage 1c via a `MemberAccess { Ident(typedef),
//! member }` / 2-segment `Ident` parse shape that, pre-fix, was NOT recognized
//! as a static task call: the call ran SYNCHRONOUSLY, its `@w.trigger` event
//! wait fell through, and the `while(1)` watcher spun forever at t=0 (never
//! parking, never advancing). The `Class#(params)::` spelling already worked;
//! the typedef alias dropped the dispatch.
//!
//! Reference-verified (2026-09-08): the reference simulator runs the equivalent
//! UVM `wait_modified` watcher to exactly `count=2` then finishes — never
//! spinning. Pre-fix xezim spins forever; post-fix it matches.

use std::process::Command;

fn xezim() -> String {
    // Cargo names the binary it built for the integration tests.
    env!("CARGO_BIN_EXE_xezim").to_string()
}

fn run(src: &str) -> String {
    std::fs::write("/tmp/typedef_receiver_static_task.sv", src).unwrap();
    let out = Command::new(xezim())
        .args([
            "--simulate",
            "-s",
            "top",
            "/tmp/typedef_receiver_static_task.sv",
        ])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const SRC: &str = r#"class waiter_obj;
  event trigger;
endclass

// A parameterized container with a static assoc store and a blocking static
// task — the shape of uvm_config_db's m_waiters + wait_modified.
class db_impl #(type T = int);
  static waiter_obj m_waiters[string];

  static task wait_modified(string field);
    waiter_obj w;
    w = new();
    m_waiters[field] = w;
    @w.trigger;
  endtask

  static function int queued(string field);
    return m_waiters.exists(field) ? 1 : 0;
  endfunction

  static function void trigger(string field);
    waiter_obj w;
    if (m_waiters.exists(field)) begin
      w = m_waiters[field];
      ->w.trigger;
    end
  endfunction
endclass

typedef db_impl#(bit) db_t;

module top;
  int count = 0;
  initial begin
    fork wait_loop; join_none
    #3;
    db_impl#(bit)::trigger("f1");
    #3;
    db_impl#(bit)::trigger("f1");
    #3;
    if (count == 2)
      $display("TAG_PASS count=%0d", count);
    else
      $display("TAG_FAIL count=%0d", count);
    $finish;
  end
  // Watcher parks on a typedef-receiver static task call. Pre-fix this spun
  // (the wait fell through synchronously); post-fix it parks and is woken by
  // the specialization-spelled trigger (matching store).
  task wait_loop;
    while(1) begin
      db_t::wait_modified("f1");
      count++;
    end
  endtask
endmodule
"#;

#[test]
fn typedef_receiver_static_blocking_task_parks() {
    let out = run(SRC);
    assert!(
        out.contains("TAG_PASS count=2"),
        "typedef-receiver static blocking task call must park and be woken \
         exactly twice (not spin synchronously), got:\n{out}"
    );
    assert!(
        !out.contains("TAG_FAIL"),
        "unexpected typedef-receiver static-task failure:\n{out}"
    );
}
