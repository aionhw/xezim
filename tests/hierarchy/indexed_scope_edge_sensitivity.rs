//! IEEE 1800-2017 §9.4.2, §23.6, §27.5: an event control whose term is a
//! hierarchical path entering a scope through an INDEX
//! (`always @(posedge gen_blk[0].u.sig)`).
//!
//! Elaboration leaves such a path as
//! `MemberAccess{ MemberAccess{ Index{ MemberAccess{ Ident } } } }`. The
//! sensitivity walkers each declined that shape — `collect_ident_names` has no
//! `MemberAccess` arm, and the dotted-chain flattening bails on an `Index`
//! base — so the block armed on an EMPTY sensitivity. It woke once at time
//! zero through the no-signal fallback and never fired again, with no
//! diagnostic: every process watching a clock through an indexed scope was
//! dead, and the design simply sat there.
//!
//! The fold used for continuous-assign targets covers this shape, so the same
//! fully-qualified name is resolved here before the terms are collected.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

const SRC: &str = r#"
module leaf (input logic clk, output logic q);
  integer n = 0;
  always @(posedge clk) n <= n + 1;
  assign q = clk;
endmodule

module tb;
  logic clk = 1'b0;
  always #5 clk = ~clk;

  logic q;
  for (genvar i = 0; i < 1; i++) begin : g
    leaf u (.clk(clk), .q(q));
  end

  // The reported shape: scope entered through a select, written both with and
  // without the top module's name. `u_plain` and `clk` are the controls that
  // worked before the fix and must keep working.
  integer n_plain = 0;
  integer n_rooted = 0;
  integer n_relative = 0;
  integer n_select = 0;

  always @(posedge clk)              n_plain    <= n_plain + 1;
  always @(posedge tb.g[0].u.clk)    n_rooted   <= n_rooted + 1;
  always @(posedge g[0].u.clk)       n_relative <= n_relative + 1;
  always @(posedge g[0].u.q)         n_select   <= n_select + 1;

  initial begin
    #102;
    $display("R plain=%0d rooted=%0d relative=%0d select=%0d",
             n_plain, n_rooted, n_relative, n_select);
    $display("R child=%0d", g[0].u.n);
    $finish;
  end
endmodule
"#;

/// Every spellings of the same indexed-scope clock must fire on each edge, and
/// a term naming a child OUTPUT port (`g[0].u.q`) must follow the port.
#[test]
fn indexed_scope_hierarchical_edge_sensitivity_fires() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    let msgs = messages(&sim);
    let want = "R plain=10 rooted=10 relative=10 select=10";
    assert!(
        msgs.iter().any(|m| m == want),
        "missing {want:?}; output: {msgs:?}"
    );
    assert!(
        msgs.iter().any(|m| m == "R child=10"),
        "child flop did not run; output: {msgs:?}"
    );
}
