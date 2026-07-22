//! IEEE 1800-2017 §9.4.2 — `@(edge x)` on a multi-bit vector is a posedge OR
//! negedge of the LSB (bit 0), like posedge/negedge, NOT a whole-vector change.
//! Confirmed against two reference simulators. Previously xezim fired `@(edge
//! wide)` on any bit change, which made a testbench monitor advance where a
//! commercial simulator's monitor (correctly) stalled — the divergence behind
//! a customer's "extra bits changed" report.

use xezim::simulate;

fn count(src: &str, needle: &str) -> Option<i64> {
    simulate(src, 1000)
        .expect("sim")
        .output
        .iter()
        .find_map(|o| {
            o.message
                .strip_prefix(needle)
                .and_then(|r| r.trim().parse::<i64>().ok())
        })
}

#[test]
fn edge_on_vector_tracks_lsb_only() {
    // v changes 4 times; bit 0 has exactly 2 transitions (0->1 then 1->0).
    let src = r#"
module tb;
  logic [31:0] v = 0; int h = 0;
  initial forever begin @(edge v); h++; end
  initial begin
    #10 v = 32'h20520000;  // bit0 0->0: no edge
    #10 v = 32'h20520001;  // bit0 0->1: edge
    #10 v = 32'h20520000;  // bit0 1->0: edge
    #10 v = 32'hFFFF0000;  // bit0 0->0: no edge
    #5  $display("EDGE=%0d", h);
    $finish;
  end
endmodule
"#;
    assert_eq!(count(src, "EDGE="), Some(2), "edge(vector) must be LSB-only");
}

#[test]
fn edge_on_one_bit_is_both_edges() {
    let src = r#"
module tb;
  logic b = 0; int h = 0;
  initial forever begin @(edge b); h++; end
  initial begin #10 b=1; #10 b=0; #10 b=1; #5 $display("E1=%0d", h); $finish; end
endmodule
"#;
    assert_eq!(count(src, "E1="), Some(3), "1-bit edge = posedge OR negedge");
}

#[test]
fn level_sensitivity_still_any_change() {
    // `@(v)` (no edge keyword) must still fire on any whole-vector change.
    let src = r#"
module tb;
  logic [31:0] v = 0; int h = 0;
  initial forever begin @(v); h++; end
  initial begin #10 v=32'h20520000; #10 v=32'hFFFF0000; #5 $display("LV=%0d", h); $finish; end
endmodule
"#;
    assert_eq!(count(src, "LV="), Some(2), "@(v) level = any-change");
}
