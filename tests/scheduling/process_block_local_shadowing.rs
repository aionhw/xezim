//! §6.21 — a PROCESS-body block local that shadows a module variable, the
//! deferred half of the always_comb shadowing fix. The process executor
//! flattens the body for suspension, so no block-scope boundary survives to
//! runtime; the frameless local landed on the module signal and the block's
//! writes clobbered it. Fixed by alpha-renaming such locals at Simulator
//! construction (`rename_process_shadowed_locals`): the rename survives
//! suspension, keeps two processes' same-named locals distinct (whether the
//! body is unnamed, auto-named, or explicitly labelled — all flatten to the
//! same bare-name `signals` slot), and skips any name a nested block
//! redeclares. Reference-validated (tmp/ac/ac16b, ac25b).
//!
//! Also: §21.2.1.3 `%0h` of an all-x value prints a single collapsed "x",
//! not one x per digit — leading x/z runs trim like leading zeros.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// The module variable must stay untouched (x) while each process's local
/// carries its own value — including across a suspension inside the block.
#[test]
fn process_local_shadows_module_var() {
    let src = r#"
module tb;
  logic [7:0] v;
  logic [7:0] w1, w2;
  int v_is_x;
  logic clk = 0;
  always #5 clk = ~clk;
  initial begin
    logic [7:0] v;
    v = 8'h11;
    @(posedge clk);          // suspension INSIDE the shadowing block
    v = v + 1;
    w1 = v;
  end
  initial begin
    logic [7:0] v;           // second process, same local name
    v = 8'h21;
    #3 w2 = v;
  end
  initial begin
    #20;
    v_is_x = $isunknown(v);  // module v: never written -> still x
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(u(&sim, "w1"), 0x12, "local survives the timing control");
    assert_eq!(u(&sim, "w2"), 0x21, "second process's local is distinct");
    assert_eq!(u(&sim, "v_is_x"), 1, "module v must NOT be clobbered");
}

/// A module variable written by another process is unaffected by the rename,
/// and a nested-block redecl of the same name disables it (conservative).
#[test]
fn shadow_rename_leaves_real_module_writes_alone() {
    let src = r#"
module tb;
  logic [7:0] v;
  logic [7:0] r1, r2;
  initial begin
    logic [7:0] v;
    v = 8'hAA;
    #2 r1 = v;
  end
  initial begin
    #1 v = 8'h5C;            // genuine module-var write from elsewhere
    #2 r2 = v;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "r1"), 0xAA, "local keeps its own value");
    assert_eq!(u(&sim, "r2"), 0x5C, "module var takes the real write");
}

/// §6.21 — two processes declaring the SAME frameless block-local name that
/// shadows NO module symbol. The old rename only fired for module-shadowing
/// locals, so these bare-name locals collided in the shared `signals` table:
/// each process's read picked up the other's (last-write-wins) value. The
/// cross-process collision scan salts them per-process. Reference-validated
/// (tmp/svrun/localsame_unnamed.sv, wc5u.sv, wc9u.sv — byte-for-byte).
#[test]
fn cross_process_same_named_frameless_local_stays_distinct() {
    let src = r#"
module tb;
  logic [7:0] r1, r2;
  initial begin
    logic [7:0] v;
    v = 8'h10;
    #1 r1 = v;
  end
  initial begin
    logic [7:0] v;
    v = 8'h1;
    #1 r2 = v;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "r1"), 0x10, "p1 reads its OWN local, not p2's");
    assert_eq!(u(&sim, "r2"), 0x1, "p2 reads its OWN local");
}

/// The SAME collision holds for a body that carries an explicit label — a
/// named block's body locals land in the flat `signals` table exactly like an
/// unnamed block's (the process executor has no per-process block-scope frame
/// for either), so a cross-process same-named local must be salted identically.
/// Reference-validated (tmp/svrun/localsame.sv — byte-for-byte).
#[test]
fn cross_process_same_named_frameless_local_stays_distinct_named_blocks() {
    let src = r#"
module tb;
  logic [7:0] r1, r2;
  initial begin : p1
    logic [7:0] v;
    v = 8'h10;
    #1 r1 = v;
  end
  initial begin : p2
    logic [7:0] v;
    v = 8'h1;
    #1 r2 = v;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "r1"), 0x10, "p1 reads its OWN local, not p2's");
    assert_eq!(u(&sim, "r2"), 0x1, "p2 reads its OWN local");
}
