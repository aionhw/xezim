//! IEEE 1800-2017 §23.6, §27.4, §27.5, §10.3.2: a continuous assign whose
//! target is a hierarchical path that enters a scope through an INDEX
//! (`assign tb.g[0].u.a = s;`).
//!
//! Elaboration folds `a.b.c` into one hierarchical `Ident`, but only when the
//! rewritten base is itself an `Ident`: an `Index` base keeps the chain, so
//! `g[0].u.a` arrived at the simulator as
//! `MemberAccess{ MemberAccess{ Index{ MemberAccess{ Ident } } } }`. Every
//! consumer of a continuous-assign target names it from a single `Ident`
//! (`collect_lhs_writes`, the alias fan-out, `cont_driven`), so that chain
//! produced no write target: the assignment drove nothing, silently. A
//! testbench connecting an instance's input port inside a `for`-generate left
//! the DUT input at `z` while the driver side ran normally.
//!
//! Every input port below is deliberately left unconnected on the instance, so
//! the hierarchical assignment is its ONLY driver: with a port connection in
//! place the assignment would be redundant and the block would pass even when
//! the assignment does nothing.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

const SRC: &str = r#"
module leaf (
  input  logic [3:0] a,
  output logic [3:0] c
);
  assign c = a;
endmodule

module tb;
  logic [3:0] s;
  logic [3:0] c_plain, c_rooted, c_gen_rooted, c_gen_relative;

  // No select in the path: these two drove correctly before the fix and must
  // keep driving.
  leaf u_plain (.c(c_plain));
  assign u_plain.a = s;
  leaf u_rooted (.c(c_rooted));
  assign tb.u_rooted.a = s;

  // The reported shape: the scope is entered through a select, and the path
  // is written with and without the top module's name.
  logic [3:0] c_gr [2];
  for (genvar i = 0; i < 2; i++) begin : g_rooted
    leaf u (.c(c_gr[i]));
  end
  assign tb.g_rooted[0].u.a = s;

  logic [3:0] c_gl [2];
  for (genvar i = 0; i < 2; i++) begin : g_relative
    leaf u (.c(c_gl[i]));
  end
  assign g_relative[1].u.a = s;

  initial begin
    s = 4'h3;
    #1;
    if (c_plain !== 4'h3 || c_rooted !== 4'h3 ||
        c_gr[0] !== 4'h3 || c_gl[1] !== 4'h3)
      $display("FAIL t0 plain=%h rooted=%h gen_rooted=%h gen_relative=%h",
               c_plain, c_rooted, c_gr[0], c_gl[1]);
    else
      $display("PASS t0");
    // The un-driven sibling element stays undriven: the fold must address the
    // selected element, not the whole generate array.
    if (c_gr[1] === 4'h3 || c_gl[0] === 4'h3)
      $display("FAIL sibling gen array driven");
    else
      $display("PASS sibling");
    s = 4'hA;
    #1;
    if (c_plain !== 4'hA || c_rooted !== 4'hA ||
        c_gr[0] !== 4'hA || c_gl[1] !== 4'hA)
      $display("FAIL t1 plain=%h rooted=%h gen_rooted=%h gen_relative=%h",
               c_plain, c_rooted, c_gr[0], c_gl[1]);
    else
      $display("PASS t1");
    $finish;
  end
endmodule
"#;

/// The driven port carries the written value at time 0 and again after the
/// source changes — the second check also proves the write registered a
/// dependency on its right-hand side. The sibling element of each generate
/// array must stay undriven.
#[test]
fn indexed_scope_hierarchical_cont_assign_drives_port() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    let msgs = messages(&sim);
    for want in ["PASS t0", "PASS sibling", "PASS t1"] {
        assert!(
            msgs.iter().any(|m| m == want),
            "missing {want}; output: {msgs:?}"
        );
    }
    assert!(
        msgs.iter().all(|m| !m.starts_with("FAIL")),
        "output: {msgs:?}"
    );
}
