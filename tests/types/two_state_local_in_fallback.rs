//! IEEE 1800-2023 §6.11.1: every write to a 2-state variable maps X/Z to 0,
//! including a write made while a compiled subroutine runs one statement in
//! the interpreter.
//!
//! A compiled function hands a statement it cannot compile (here a `foreach`
//! over a module array) to the interpreter, carrying its register locals in a
//! frame of their own. Writes to those carried locals skipped the 2-state
//! fitting and left it to the copy-back after the statement. That is enough
//! for one assignment but not for a loop run as one statement: `s += m[i]`
//! with `m[i]` x kept `s` x into every later trip, so `int s = 0; foreach
//! (m[i]) s += m[i];` over `{x, x, x, 10}` returned 0 instead of 10, while the
//! same loop in a task, a `for` loop and module scope gave 10.
//!
//! Expected values are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
module top;
  logic [31:0] m [4];
  logic [31:0] dm [];
  logic [31:0] qm [$];
  function automatic int f1(); int s = 0; foreach (m[i]) s += m[i]; return s; endfunction
  function automatic int f2(); int s = 0; foreach (m[i]) s = s + m[i]; return s; endfunction
  function automatic int f3(); int s = 0; foreach (m[i]) begin s += m[i]; end return s; endfunction
  function automatic int f4(); int s; s = 0; foreach (m[i]) s += m[i]; return s; endfunction
  function automatic int f5(); bit [31:0] s = 0; foreach (m[i]) s += m[i]; return s; endfunction
  function automatic int f6(); int s = 0; foreach (dm[i]) s += dm[i]; return s; endfunction
  function automatic int f7(); int s = 0; foreach (qm[i]) s += qm[i]; return s; endfunction
  function automatic int f8(); int s = 0; for (int i = 0; i < 4; i++) begin s += m[i]; end return s; endfunction
  function automatic int f9(); int s = 0; int i = 0; while (i < 4) begin s += m[i]; i++; end return s; endfunction
  function automatic int f10(); int s = 0; repeat (1) s += m[0]; s += m[3]; return s; endfunction
  function static int f11(); int s; s = 0; foreach (m[i]) s += m[i]; return s; endfunction
  function automatic int f12(); int s = 0; foreach (m[i]) if (1) s += m[i]; return s; endfunction
  task automatic t1(output int o); int s = 0; foreach (m[i]) s += m[i]; o = s; endtask
  int r[16];
  initial begin
    m[3] = 10;
    dm = new[4]; dm[3] = 10;
    qm = '{32'bx, 32'bx, 32'bx, 32'd10};
    r[1] = f1(); r[2] = f2(); r[3] = f3(); r[4] = f4(); r[5] = f5(); r[6] = f6(); r[7] = f7();
    r[8] = f8(); r[9] = f9(); r[10] = f10(); r[11] = f11(); r[12] = f12(); t1(r[13]);
    begin int s = 0; foreach (m[i]) s += m[i]; r[14] = s; end
    $display("R %p", r);
    $finish;
  end
endmodule
"#;

#[test]
fn carried_two_state_local_drops_xz_per_write() {
    let sim = simulate(SRC, 10).expect("simulate failed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    let want = "R '{0, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 0}";
    assert!(
        out.iter().any(|l| l.trim() == want),
        "missing {want:?} in {out:?}"
    );
}
