//! §6.18/§23.10/§6.20.3: a module-local packed-struct typedef whose member
//! widths depend on the module's own parameters, handed to a child through a
//! `type` parameter override (`leaf #(.rec_t(rec_t))`).
//!
//! Each instance must get ITS OWN layout (39 bits at N=6, 68 at N=13). The
//! bare `typedef_types` slot was registered `entry().or_insert` — first
//! instance wins — and the restore rail deliberately handed back the
//! post-state, so during a later differently-parameterized instance's
//! inlining, BOTH the type-parameter override resolution and the parent's own
//! member writes read the FIRST instance's baked layout: 39-bit field offsets
//! inside a 68-bit struct, order-dependent on declaration order. The slot is
//! now overwritten for the duration of each instance's inlining and restored
//! prior-or-keep on exit (post-inlining consumers still see the
//! first-declared fallback).
//!
//! Expected values are the reference simulator's; both instance orders are
//! pinned because whichever module was inlined FIRST used to be the one that
//! survived.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

const BODY: &str = r#"
module leaf_unit #(
   parameter type rec_t = logic [7:0]
) (
   input  rec_t        in_rec,
   output logic [67:0] whole,
   output logic [63:0] cnt_a,
   output logic [63:0] cnt_e,
   output logic [7:0]  nbits
);
   localparam int NBITS = $bits(rec_t);
   assign whole = in_rec;
   assign cnt_a = in_rec.cnt_a;
   assign cnt_e = in_rec.cnt_e;
   assign nbits = NBITS;
endmodule

module proc_unit #(
   parameter N = 13,
   parameter K = ((N > 6) ? 4 : 3)
) (
   output logic [67:0] rec_whole,
   output logic [7:0]  nbits
);
   typedef struct packed {
      logic          tag_1b;
      logic [N-2:0]  cnt_a;
      logic [K:0]    cnt_b;
      logic [N-2:0]  cnt_c;
      logic [3:0]    bus_4a;
      logic [3:0]    bus_4b;
      logic          flag_1a;
      logic          flag_1b;
      logic [1:0]    adj_2b;
      logic          m0_1b;
      logic          m1_1b;
      logic [N-2:0]  cnt_d;
      logic [N-2:0]  cnt_e;
   } rec_t;

   rec_t rec_in;

   leaf_unit #(.rec_t(rec_t)) u_leaf (
      .in_rec(rec_in), .whole(rec_whole),
      .cnt_a(), .cnt_e(), .nbits(nbits)
   );

   initial begin
      rec_in.tag_1b  = 1'b1;
      rec_in.cnt_a   = '1;
      rec_in.cnt_b   = '1;
      rec_in.cnt_c   = '1;
      rec_in.bus_4a  = 4'hA;
      rec_in.bus_4b  = 4'h5;
      rec_in.flag_1a = 1'b1;
      rec_in.flag_1b = 1'b0;
      rec_in.adj_2b  = 2'b10;
      rec_in.m0_1b   = 1'b1;
      rec_in.m1_1b   = 1'b1;
      rec_in.cnt_d   = '1;
      rec_in.cnt_e   = '1;
   end
endmodule
"#;

fn check(o: &str) {
    for expect in [
        // u0 (N=6): 1+5+4+5+4+4+1+1+2+1+1+5+5 = 39 bits
        "u0 pw=39 cw=39 whole=0000000007fffa5afff a=31 e=31",
        // u1 (N=13): 1+12+5+12+4+4+1+1+2+1+1+12+12 = 68 bits. The written
        // literal is 76 bits; === compares its LOW 68 (both simulators).
        "u1 pw=68 cw=68 whole=00fffffffe96bffffff a=4095 e=4095",
    ] {
        assert!(o.contains(expect), "expected `{expect}` in:\n{o}");
    }
}

const REPORT: &str = r#"
   initial begin
      #10;
      $display("u0 pw=%0d cw=%0d whole=%019x a=%0d e=%0d",
               u0.nbits, u0.u_leaf.nbits, u0.u_leaf.whole, u0.u_leaf.cnt_a, u0.u_leaf.cnt_e);
      $display("u1 pw=%0d cw=%0d whole=%019x a=%0d e=%0d",
               u1.nbits, u1.u_leaf.nbits, u1.u_leaf.whole, u1.u_leaf.cnt_a, u1.u_leaf.cnt_e);
   end
endmodule
"#;

#[test]
fn each_instance_keeps_its_own_type_param_layout_small_first() {
    let o = out(&format!(
        "{BODY}\nmodule tb_top;\n  proc_unit #(.N(6)) u0 ();\n  proc_unit #(.N(13)) u1 ();\n{REPORT}"
    ));
    check(&o);
}

#[test]
fn each_instance_keeps_its_own_type_param_layout_big_first() {
    // The mirror order: first-wins used to break whichever came SECOND.
    let o = out(&format!(
        "{BODY}\nmodule tb_top;\n  proc_unit #(.N(13)) u1 ();\n  proc_unit #(.N(6)) u0 ();\n{REPORT}"
    ));
    check(&o);
}
