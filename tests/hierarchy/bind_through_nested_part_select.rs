//! §23.3.3 / §11.5.1 — a `.*`-bound interface output must still alias the port
//! net when the port is connected by a PART-SELECT at more than one level.
//!
//! Substituting a part-select actual through another part-select connection
//! stacked the two selects: `bus_8b` -> `bus_in[7:0]` -> `bus_top[7:0][7:0]`.
//! That doubly-ranged reference resolves to nothing, so the bound interface's
//! output stopped aliasing the port net and every reader below read `z` while
//! the interface itself held `x`.
//!
//! It takes TWO nested part-selects to trigger — one level produces a single
//! select and was always correct, which is what made this look like a bind bug
//! rather than a substitution one:
//!
//! ```text
//! s->p     c->s     result
//! whole    whole    x    (ok)
//! [7:0]    whole    x    (ok)
//! whole    [7:0]    x    (ok)
//! [7:0]    [7:0]    z    (was broken)
//! ```
//!
//! Only an IDENTICAL constant range collapses; a genuinely different range
//! re-selects and is left alone, which the second case below pins.

use xezim::simulate;

/// Part-select at every level, three deep — the reported shape.
const NESTED_PART_SELECT: &str = r#"
interface mon_if (output logic flag_1b, output logic [7:0] bus_8b);
endinterface
module leaf_unit (input flag_1b, input [7:0] bus_8b, output [7:0] bus_out);
  assign bus_out = bus_8b;
endmodule
module proc_unit (input flag_1b, input [7:0] bus_8b, output [7:0] bus_out);
  leaf_unit u_leaf (.flag_1b(flag_1b), .bus_8b(bus_8b[7:0]), .bus_out(bus_out));
endmodule
bind proc_unit mon_if mon_inst (.*);
module subsys (input flag_in, input [7:0] bus_in, output [7:0] bus_out);
  proc_unit u_proc (.flag_1b(flag_in), .bus_8b(bus_in[7:0]), .bus_out(bus_out));
endmodule
module chip_top (input flag_top, input [7:0] bus_top, output [7:0] bus_out);
  subsys u_sub (.flag_in(flag_top), .bus_in(bus_top[7:0]), .bus_out(bus_out));
endmodule
module tb;
  wire flag_top; wire [7:0] bus_top; wire [7:0] bus_out;
  chip_top dut (.flag_top(flag_top), .bus_top(bus_top), .bus_out(bus_out));
  int ok;
  initial begin
    #1;
    ok = (dut.u_sub.u_proc.flag_1b === 1'bx)          // whole-net path (always worked)
      && (dut.u_sub.u_proc.bus_8b === 8'hxx)          // was 8'hzz
      && (dut.u_sub.u_proc.u_leaf.bus_8b === 8'hxx)   // and the deeper reader
      && (dut.u_sub.bus_in === 8'hxx);                // and back up the chain
  end
endmodule
"#;

/// A genuinely OFFSET nested part-select must NOT collapse: `p[3:0]` over a
/// `.p(w[7:4])` connection still selects the low half of that slice, so the
/// two selects are not redundant and both must survive.
const OFFSET_PART_SELECT: &str = r#"
module inner (input [3:0] p, output [3:0] o);
  assign o = p;
endmodule
module mid (input [7:0] w, output [3:0] o);
  inner u_i (.p(w[7:4]), .o(o));
endmodule
module tb;
  logic [7:0] src;
  wire  [3:0] o;
  mid u_m (.w(src), .o(o));
  int ok;
  initial begin
    src = 8'hA5;          // w[7:4] == 4'hA
    #1;
    ok = (o === 4'hA);
  end
endmodule
"#;

fn ok_flag(src: &str) -> u64 {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.get_signal("ok")
        .or_else(|| sim.get_signal("tb.ok"))
        .expect("signal 'ok' not found")
        .to_u64()
        .unwrap_or(0)
}

#[test]
fn wildcard_bind_aliases_through_nested_part_selects() {
    assert_eq!(ok_flag(NESTED_PART_SELECT), 1, "a `.*` bind lost the port-net alias through nested part-selects");
}

#[test]
fn offset_nested_part_select_still_selects_its_slice() {
    assert_eq!(ok_flag(OFFSET_PART_SELECT), 1, "an offset nested part-select was wrongly collapsed");
}
