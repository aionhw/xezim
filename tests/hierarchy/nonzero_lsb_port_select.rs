//! IEEE 1800-2023 §7.4.1 / §23.3.3: a non-`[N:0]` input port keeps its
//! declared labels when the actual is a zero-based part-select.
//!
//! `input wire [11:2] paddr` bound to `addr[2+:10]` used to be substituted
//! with that part-select. The ten bits were right, but `paddr[5:2]` read
//! physical bits 5:2 (the PCELL-ID nibble) and `paddr[11:5]` read past the
//! MSB and came back X. The connection is an assignment onto the formal,
//! which is stored LSB-aligned and indexed by the declared range.
//!
//! The bit pattern is the CMSDK watchdog PERIPHID4 address: parent
//! `32'hFD0` has bits `[11:2] == 10'b1111110100`, so `[11:5] == 7'b1111110`
//! and `[5:2] == 4'b0100`.

use xezim::simulate;

fn bits(sim: &xezim::compiler::Simulator, n: &str) -> Option<u64> {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{n}")))
        .unwrap_or_else(|| panic!("signal not found: {n}"))
        .to_u64()
}

const SRC: &str = r#"
module id_probe (
   input  wire  [11:2] paddr,
   output logic [7:0]  hi,
   output logic [3:0]  lo,
   output logic [7:0]  id_byte
);
   assign hi = paddr[11:5];
   assign lo = paddr[5:2];
   always_comb begin
      case (paddr[5:2])
         4'b0100: id_byte = 8'h04;
         4'b1101: id_byte = 8'hF0;
         default: id_byte = 8'hxx;
      endcase
   end
endmodule

module hi_probe (
   input  wire  [15:8] p,
   output logic [7:0]  whole,
   output logic [3:0]  top
);
   assign whole = p[15:8];
   assign top   = p[15:12];
endmodule

module tb;
   logic [31:0] addr;
   logic [11:2] same;
   logic [15:0] wide;

   assign same = addr[11:2];

   logic [7:0] hi_idx, hi_rng, hi_same, id_idx, id_rng, id_same;
   logic [3:0] lo_idx, lo_rng, lo_same;
   logic [7:0] whole_ps;
   logic [3:0] top_ps;

   id_probe u_idx  (.paddr(addr[2+:10]), .hi(hi_idx),  .lo(lo_idx),  .id_byte(id_idx));
   id_probe u_rng  (.paddr(addr[11:2]),  .hi(hi_rng),  .lo(lo_rng),  .id_byte(id_rng));
   id_probe u_same (.paddr(same),        .hi(hi_same), .lo(lo_same), .id_byte(id_same));
   hi_probe u_hi   (.p(wide[7:0]),       .whole(whole_ps), .top(top_ps));

   initial begin
      addr = 32'h00000FD0;
      wide = 16'h00A5;
      #1;
   end
endmodule
"#;

#[test]
fn nonzero_lsb_input_keeps_declared_selects() {
    let sim = simulate(SRC, 50).expect("simulate failed");
    for sfx in ["idx", "rng", "same"] {
        assert_eq!(
            bits(&sim, &format!("hi_{sfx}")),
            Some(0x7e),
            "paddr[11:5] via {sfx}: 7'b1111110"
        );
        assert_eq!(
            bits(&sim, &format!("lo_{sfx}")),
            Some(0x4),
            "paddr[5:2] via {sfx}: 4'b0100, not the unrebased 4'b1101"
        );
        assert_eq!(
            bits(&sim, &format!("id_{sfx}")),
            Some(0x04),
            "case (paddr[5:2]) via {sfx}: PERIPH_ID4, not PCELL_ID1 8'hF0"
        );
    }
    assert_eq!(
        bits(&sim, "whole_ps"),
        Some(0xa5),
        "[15:8] port driven by wide[7:0] is LSB-aligned"
    );
    assert_eq!(
        bits(&sim, "top_ps"),
        Some(0xa),
        "p[15:12] of that port is the high nibble"
    );
}
