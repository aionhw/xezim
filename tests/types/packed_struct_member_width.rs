//! §7.2.1: an NBA to a packed-struct member wider than 32 bits truncated.
//!
//! `s.field <= v` arrives as a two-segment `Ident` (the elaborator collapses
//! member access into a dotted name), and a packed member is a bit SLICE of
//! its container, not a signal of its own — so every lookup in the lvalue
//! width inference missed and it fell through to the 32-bit default. A
//! 40-bit field therefore lost its top 8 bits on every nonblocking write,
//! while the identical BLOCKING write was correct, which is what made this
//! look like a scheduling bug rather than a width bug.
//!
//! Reference-verified: all five pipeline cycles match line for line.

use xezim::simulate;

#[test]
fn nba_to_a_wide_packed_member_keeps_every_bit() {
    const SRC: &str = r#"
typedef struct packed { logic [39:0] vaddr; logic [3:0] sid; logic hipri; } s_t;
module tb;
  logic clk = 0;
  s_t src, viaif_nba, via_blocking;
  logic [39:0] observed_nba;      // plain signal: a member is only a slice
  logic [39:0] observed_blocking;
  int  mismatches = 0;
  always #5 clk = ~clk;
  always @(posedge clk) viaif_nba.vaddr <= src.vaddr;
  initial begin
    src = '0;
    src.vaddr = 40'hFF_1234_5678;
    via_blocking.vaddr = src.vaddr;
    @(posedge clk);
    #1;
    observed_nba      = viaif_nba.vaddr;
    observed_blocking = via_blocking.vaddr;
    if (viaif_nba.vaddr !== 40'hFF_1234_5678) mismatches++;
    if (via_blocking.vaddr !== 40'hFF_1234_5678) mismatches++;
    #1 $finish;
  end
endmodule
"#;
    let sim = simulate(SRC, 200).expect("simulate failed");
    let read = |n: &str| -> u64 {
        sim.get_signal(n)
            .unwrap_or_else(|| panic!("signal {n} not found"))
            .to_u64()
            .unwrap_or_else(|| panic!("signal {n} is X/Z"))
    };
    assert_eq!(read("mismatches"), 0, "a packed member lost bits");
    // Pin the actual value too, so a future default-width regression that
    // happens to affect both paths equally still fails here.
    assert_eq!(
        read("observed_nba"),
        0xFF_1234_5678,
        "NBA to a 40-bit packed member truncated"
    );
    assert_eq!(read("observed_blocking"), 0xFF_1234_5678, "blocking path");
}
