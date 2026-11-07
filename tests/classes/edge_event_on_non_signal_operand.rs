// Regression test: an EDGE event control (`@(posedge x)` / `@(negedge x)`)
// whose operand has no backing signal — a class property, a static class
// property, or a procedural local — must block until that operand actually
// makes the transition.
//
// §9.4.2 makes `@(posedge x)` a transition on the VALUE of `x`. Only a
// module-scope signal has an entry in the signal table, so for anything else
// `event_to_sens` produced a sensitivity naming a nonexistent signal, the
// waiter resolved to an empty sensitivity, and the wait degenerated to a
// same-time yield: it returned in the current time step (or after the NBA
// region) instead of blocking. A monitor task that held a handshake flag as a
// class property (`@(posedge send_flag); ...; send_flag = 1'b0;`) therefore
// never waited: it spun, re-sending the same object on every delta.
//
// The fix arms a value snapshot and an edge-aware condition
// (`class_prop_edge_cond`), so the wait is served by the ordinary
// value-change wake path that class-property writes already drive.
//
// Verified against Verilator: only the operands that really transition print,
// all at the time of the transition.

use std::process::Command;

fn xezim() -> String {
    env!("CARGO_BIN_EXE_xezim").to_string()
}

fn run(src: &str, tag: &str) -> String {
    let path = format!("/tmp/edge_nonsignal_{tag}.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", &path])
        .output()
        .expect("xezim failed to start");
    String::from_utf8_lossy(&out.stdout).to_string()
}

const DESIGN: &str = r#"
module top;
  class C;
    bit         flag;        // 2-state instance property
    logic [7:0] wide;        // 4-state instance property
    static bit  sflag;       // static property
    bit         never_set;   // never written: the wait must never fire
    time        t_bare;      // bare-name wait from inside a method

    task automatic wait_bare();
      @(posedge flag);
      t_bare = $time;
    endtask

    task automatic poke();
      flag = 1'b1;
      wide = 8'h01;
      sflag = 1'b1;
    endtask
  endclass

  C    c;
  time t_flag, t_wide, t_sflag, t_never, t_local, t_neg;

  initial begin
    c = new();
    t_flag = 0; t_wide = 0; t_sflag = 0; t_never = 0;
    t_local = 0; t_neg = 0;

    fork begin @(posedge c.flag);  t_flag  = $time; end join_none
    fork begin @(posedge c.wide);  t_wide  = $time; end join_none
    fork begin @(posedge c.sflag); t_sflag = $time; end join_none
    fork begin @(posedge c.never_set); t_never = $time; end join_none
    fork begin @(negedge c.flag);  t_neg   = $time; end join_none
    fork c.wait_bare(); join_none
    fork begin
      bit lf = 1'b0;           // procedural local, never written
      @(posedge lf);
      t_local = $time;
    end join_none

    #100;
    c.poke();                  // the only real transitions
    #100;
    c.flag = 1'b0;             // the only negedge
    #100;
    $display("RESULT flag=%0d wide=%0d sflag=%0d bare=%0d never=%0d local=%0d neg=%0d",
             t_flag, t_wide, t_sflag, c.t_bare, t_never, t_local, t_neg);
    $finish;
  end
endmodule
"#;

/// Every edge wait on a non-signal operand resumes at the transition time,
/// and a wait whose operand never transitions never resumes at all.
#[test]
fn edge_event_on_non_signal_operand_blocks_until_transition() {
    let out = run(DESIGN, "all");
    assert!(
        out.contains("RESULT flag=100 wide=100 sflag=100 bare=100 never=0 local=0 neg=200"),
        "unexpected wake times:\n{out}"
    );
}

/// A value-change (non-edge) control on the same shapes keeps working.
#[test]
fn value_change_event_on_non_signal_operand_still_blocks() {
    let out = run(
        r#"
module top;
  class C;
    bit flag;
  endclass
  C c;
  time t0, t1;
  initial begin
    c = new();
    t0 = 0; t1 = 0;
    fork begin @(c.flag); t0 = $time; end join_none
    fork begin wait (c.flag == 1'b1); t1 = $time; end join_none
    #100;
    c.flag = 1'b1;
    #100;
    $display("RESULT t0=%0d t1=%0d", t0, t1);
    $finish;
  end
endmodule
"#,
        "vc",
    );
    assert!(
        out.contains("RESULT t0=100 t1=100"),
        "unexpected wake times:\n{out}"
    );
}
