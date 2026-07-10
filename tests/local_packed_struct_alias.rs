//! A *local* packed-struct variable (declared inside a procedural block) must
//! alias its whole value with its members, like a module-level packed-struct
//! signal: a whole-variable write is visible through `p.field`, and a member
//! write is visible through the whole `p`. Previously the two were stored
//! independently, so `p = 16'hABCD` left `p.a` reading X.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  typedef struct packed { logic [7:0] a; logic [7:0] b; } packed_t;
  logic [7:0] fa, fb;      // from a member write, read back whole
  logic [15:0] whole;      // from a whole write
  logic [7:0] wa, wb;      // from a whole write, read back members
  initial begin
    packed_t p;
    p.a = 8'hAA; p.b = 8'h55;   // member write
    fa = p.a; fb = p.b; whole = p;
    p = 16'h1234;               // whole write
    wa = p.a; wb = p.b;
  end
endmodule
"#;

fn get(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("tb.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("signal {} not u64-able", name))
}

#[test]
fn local_packed_struct_whole_member_alias() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    // Member write reflected in the whole value.
    assert_eq!(get(&sim, "fa") & 0xFF, 0xAA);
    assert_eq!(get(&sim, "fb") & 0xFF, 0x55);
    assert_eq!(get(&sim, "whole") & 0xFFFF, 0xAA55);
    // Whole write reflected in the members.
    assert_eq!(get(&sim, "wa") & 0xFF, 0x12);
    assert_eq!(get(&sim, "wb") & 0xFF, 0x34);
}
