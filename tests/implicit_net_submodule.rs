//! Regression for the c910 memcpy/cmark hang root cause (xezim-core
//! 8d46dd5): IEEE 1800-2023 §6.10 implicit nets — a bare identifier on
//! the LHS of a continuous assign with no `wire` declaration — were only
//! created for the *top* module's cont-assigns, not for cont-assigns in
//! deferred sub-module bodies. So an `assign undeclared = ...;` inside a
//! non-top sub-module was silently dropped, leaving `undeclared` (and
//! anything reading it) stuck. In c910 this was `wid_for_axi4`'s
//! `assign create_en = biu_pad_awvalid && pad_biu_awready;`.
//!
//! Note: a sub-module *port* named `create_en` does NOT reproduce this —
//! ports are already in the module's local-names set. The implicit net
//! must be a truly undeclared internal wire. Here `mid` is that wire,
//! and it feeds the module's output, so if the implicit-net machinery
//! doesn't run, `y` never gets a defined value.

use xezim::simulate;

fn u64_of(sim: &xezim::compiler::Simulator, names: &[&str]) -> u64 {
    for n in names {
        if let Some(v) = sim.get_signal(n) {
            return v.to_u64().unwrap_or_else(|| panic!("{n} has X/Z, expected defined")) & 0xFFFF_FFFF;
        }
    }
    panic!("none of these signals found: {names:?}");
}

// One level of nesting: tb -> inner. `mid` is an undeclared implicit net.
const SRC_1LEVEL: &str = r#"
module inner(input [7:0] a, input [7:0] b, output [7:0] y);
  // No `wire mid;` — implicit 1-bit net (IEEE 1800-2023 §6.10).
  assign mid = a[0] ^ b[0];
  assign y = {7'b0, mid};
endmodule

module tb;
  reg [7:0] a, b;
  wire [7:0] y;
  inner u(.a(a), .b(b), .y(y));
  initial begin
    a = 8'hFF; b = 8'hF0; #1;   // a[0]=1, b[0]=0 -> mid=1 -> y=1
    a = 8'hFF; b = 8'hF1; #5;   // a[0]=1, b[0]=1 -> mid=0 -> y=0
    $finish;
  end
endmodule
"#;

// Two levels deep (closer to c910's tb.x_soc.x_cpu_sub_system_axi.
// wid_for_axi4 path): tb -> wrap -> leaf, with the implicit net `en`
// inside `leaf` gating the output.
const SRC_2LEVEL: &str = r#"
module leaf(input g, input d, output o);
  // implicit net `en` (no declaration)
  assign en = g & d;
  assign o = en;
endmodule

module wrap(input wg, input wd, output wo);
  wire t_g, t_d;
  assign t_g = wg;
  assign t_d = wd;
  leaf l(.g(t_g), .d(t_d), .o(wo));
endmodule

module tb;
  reg g, d;
  wire o;
  wrap w(.wg(g), .wd(d), .wo(o));
  reg [3:0] saw_one, saw_zero;
  initial begin
    saw_one = 0; saw_zero = 0;
    g = 1; d = 1; #1; if (o === 1'b1) saw_one = saw_one + 1;
    g = 1; d = 0; #1; if (o === 1'b0) saw_zero = saw_zero + 1;
    g = 0; d = 1; #1; if (o === 1'b0) saw_zero = saw_zero + 1;
    g = 1; d = 1; #1; if (o === 1'b1) saw_one = saw_one + 1;
    $finish;
  end
endmodule
"#;

#[test]
fn implicit_net_in_submodule_cont_assign_one_level() {
    // Run long enough to settle past the second stimulus.
    let sim = simulate(SRC_1LEVEL, 50).expect("simulate failed");
    // y must be a defined value reflecting a[0]^b[0]. If the implicit
    // net `mid` was never created, the cont-assigns get dropped and `y`
    // stays X (to_u64 panics) or 0 — never the live computed value.
    let y = u64_of(&sim, &["tb.y", "y"]);
    assert_eq!(y, 0, "after a=0xFF,b=0xF1 (a[0]^b[0]=0) -> y should be 0; got {y}");
}

#[test]
fn implicit_net_in_submodule_two_levels_deep() {
    let sim = simulate(SRC_2LEVEL, 50).expect("simulate failed");
    let saw_one = u64_of(&sim, &["tb.saw_one", "saw_one"]);
    let saw_zero = u64_of(&sim, &["tb.saw_zero", "saw_zero"]);
    assert_eq!(saw_one, 2, "leaf.en (implicit net) gating output: expected o===1 twice, got {saw_one}");
    assert_eq!(saw_zero, 2, "expected o===0 twice (g&d == 0), got {saw_zero}");
}
