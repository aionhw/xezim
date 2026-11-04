//! IEEE 1800-2017 §7.4.1: packed dimensions that live on a TYPEDEF
//! (`typedef k_lane [4:0] k_plane; k_plane a;`) shape a variable declared
//! inside an instantiated module. The inlined variable arm never recorded the
//! element stride or full dims, so `a` was one flat 320-bit vector: a
//! continuous `a[1][0] = 1'b1` compiled as a write of bit 0 (which `a[0][0]`
//! then read) and `a[2] = 64'h11` wrote that lane's LSB. Companion to
//! xezim-core #93 (issue #92). The second case pins the guard added on top of
//! that fix: dims written on the reference itself (`pair_t [3:0] b;`) must
//! not be dropped by the one-step typedef resolution.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.trim().to_string()).collect()
}

#[test]
fn inlined_typedef_packed_array_keeps_element_stride() {
    let sim = simulate(
        r#"
module leaf;
  typedef logic [63:0] k_lane;
  typedef k_lane [4:0] k_plane;
  wire k_plane a;
  assign a[1][0] = 1'b1;
  assign a[2] = 64'h11;
  initial #1 $display("a10=%b a00=%b a2=%h a1=%h", a[1][0], a[0][0], a[2], a[1]);
endmodule
module top;
  leaf u ();
endmodule
"#,
        10,
    )
    .expect("sim");
    let msgs = messages(&sim);
    // `a[1]` has only bit 0 driven: 15 all-z nibbles print `z`, the mixed
    // `zzz1` nibble prints `Z` (§21.2.1.3).
    assert!(
        msgs.iter()
            .any(|m| m == "a10=1 a00=z a2=0000000000000011 a1=zzzzzzzzzzzzzzzZ"),
        "{msgs:?}"
    );
}

#[test]
fn reference_dims_outside_the_typedef_are_kept() {
    let sim = simulate(
        r#"
module leaf;
  typedef logic [1:0][7:0] pair_t;
  wire pair_t [3:0] b;
  assign b[3] = {8'hab, 8'hcd};
  assign b[0][1] = 8'h5a;
  initial #1 $display("b3=%h b31=%h b01=%h b00=%b bits=%0d", b[3], b[3][1], b[0][1], b[0][0], $bits(b));
endmodule
module top;
  leaf u ();
endmodule
"#,
        10,
    )
    .expect("sim");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "b3=abcd b31=ab b01=5a b00=zzzzzzzz bits=64"),
        "{msgs:?}"
    );
}
