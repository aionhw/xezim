//! IEEE 1800-2017 §10.6 force/release and §10.6.1 procedural continuous
//! assignments (assign/deassign) — native SystemVerilog statements.
//!
//! While a target is forced (§10.6.2) or procedurally assigned (§10.6.1),
//! ordinary procedural assignments and continuous-driver changes must be
//! ignored. On `release`, a VARIABLE retains the forced value until the
//! next procedural assignment; a NET returns to the value produced by its
//! continuous drivers. `deassign` likewise retains the last assigned value.

use xezim::simulate;

fn lookup(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("tb.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("signal {} not u64-able", name))
}

/// §10.6.2: while a VARIABLE is forced, procedural assignments are ignored;
/// on release the variable RETAINS the forced value until the next
/// procedural assignment, which then takes effect normally.
#[test]
fn variable_force_blocks_writes_and_release_retains() {
    const SRC: &str = r#"
module tb;
  reg [31:0] v = 32'h0000_0011;
  reg [31:0] s_forced, s_blocked, s_released, s_post;
  initial begin
    #1 force v = 32'h0000_CAFE;
    #1 s_forced = v;          // forced value visible
    v = 32'h0000_BEEF;        // must be IGNORED while forced (§10.6)
    #1 s_blocked = v;
    release v;
    #1 s_released = v;        // variable retains forced value (§10.6.2)
    v = 32'h0000_BEEF;        // next procedural assignment wins
    #1 s_post = v;
    $finish;
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(lookup(&sim, "s_forced"), 0xCAFE, "force must override the variable");
    assert_eq!(
        lookup(&sim, "s_blocked"),
        0xCAFE,
        "procedural assignment must be ignored while the variable is forced"
    );
    assert_eq!(
        lookup(&sim, "s_released"),
        0xCAFE,
        "a released VARIABLE retains the forced value until the next assignment"
    );
    assert_eq!(
        lookup(&sim, "s_post"),
        0xBEEF,
        "the first procedural assignment after release must take effect"
    );
}

/// §10.6.2: while a NET is forced, changes to its continuous drivers are
/// ignored; on release the net re-evaluates to its drivers' current value.
#[test]
fn net_force_ignores_driver_and_release_reevaluates() {
    const SRC: &str = r#"
module tb;
  reg  [7:0] drv = 8'hA5;
  wire [7:0] n;
  assign n = drv;
  reg [7:0] s_forced, s_driver_change, s_released;
  initial begin
    #1 force n = 8'hFF;
    #1 s_forced = n;          // forced value visible on the net
    drv = 8'h3C;              // driver change must NOT reach the net
    #1 s_driver_change = n;
    release n;
    #1 s_released = n;        // net returns to its continuous driver
    $finish;
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(lookup(&sim, "s_forced"), 0xFF, "force must override the net");
    assert_eq!(
        lookup(&sim, "s_driver_change"),
        0xFF,
        "a continuous-driver change must be ignored while the net is forced"
    );
    assert_eq!(
        lookup(&sim, "s_released"),
        0x3C,
        "a released NET must re-evaluate to its continuous drivers"
    );
}

/// §10.6.1: a procedural continuous assignment (`assign` statement on a
/// variable) blocks ordinary procedural writes; `deassign` removes the
/// override but the variable retains the assigned value until the next
/// procedural assignment.
#[test]
fn procedural_continuous_assign_deassign() {
    const SRC: &str = r#"
module tb;
  reg [15:0] v = 16'h0001;
  reg [15:0] s_assigned, s_blocked, s_deassigned, s_post;
  initial begin
    #1 assign v = 16'hF0F0;
    #1 s_assigned = v;        // assigned value visible
    v = 16'h5A5A;             // must be IGNORED while assigned (§10.6.1)
    #1 s_blocked = v;
    deassign v;
    #1 s_deassigned = v;      // retains value after deassign (§10.6.1)
    v = 16'h5A5A;             // next procedural assignment wins
    #1 s_post = v;
    $finish;
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(lookup(&sim, "s_assigned"), 0xF0F0, "assign must drive the variable");
    assert_eq!(
        lookup(&sim, "s_blocked"),
        0xF0F0,
        "procedural writes must be blocked while a procedural continuous assign is active"
    );
    assert_eq!(
        lookup(&sim, "s_deassigned"),
        0xF0F0,
        "deassign must retain the last assigned value"
    );
    assert_eq!(
        lookup(&sim, "s_post"),
        0x5A5A,
        "the first procedural assignment after deassign must take effect"
    );
}
