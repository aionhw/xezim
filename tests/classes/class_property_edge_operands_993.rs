//! #993 (reported by Ganesh-TNL): an edge event control on a class
//! property (IEEE 1800-2023 §9.4.2) inside a class task. The reporter's
//! reproducer (a class-member clock, with and without an `iff` on a
//! class-member condition) passes since the §9.4.2 class-property edge
//! waits; these tests keep it and the neighbouring operand shapes that still
//! armed on nothing and spun at one time step: a select of a property
//! (`@(posedge bus[1])`, §11.5.1), a property reached through an element of
//! a handle array (`@(posedge subs[1].clk)`), and an `or` list mixing a
//! signal with a property (`@(posedge tb_top.mclk or posedge rst)`) at the
//! head of a `forever` loop, whose waiter armed on the signal alone.
//!
//! Expected values from the reference simulator.

use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect()
}

/// The reporter's reproducer.
#[test]
fn class_member_clock_edges_993() {
    let out = messages(
        r#"
`ifndef SVTEST_DEFS_SVH
`define SVTEST_DEFS_SVH

`define SVTEST_INIT \
  int failures = 0;

`define SVTEST_CHECK(expr, msg) \
  if (!(expr)) begin \
    failures++; \
    $display("FAIL: %s", msg); \
  end

`define SVTEST_PASSFAIL \
  if (failures == 0) begin \
    $display("TEST_PASS"); \
  end else begin \
    $display("TEST_FAIL count=%0d", failures); \
    $fatal(1); \
  end

`endif

// ---------------------------------------------------------------------------
// Case 1: class-member clock, no `iff` guard.
// 10 ns clock, sampled from t=0 to t=52 -> exactly 5 posedges (5,15,25,35,45).
// ---------------------------------------------------------------------------
class member_clk_bfm;
  bit clk = 1'b0;
  int edges = 0;

  task automatic count_edges();
    forever begin
      @(posedge clk);
      edges++;
    end
  endtask
endclass

// ---------------------------------------------------------------------------
// Case 2: class-member clock AND class-member `iff` condition.
// Condition is low until t=40, so only posedges at 45 and 55 may count.
// ---------------------------------------------------------------------------
class member_clk_iff_bfm;
  bit clk   = 1'b0;
  bit vld   = 1'b0;
  int edges = 0;

  task automatic count_edges();
    forever begin
      @(posedge clk iff vld);
      edges++;
    end
  endtask
endclass

// ---------------------------------------------------------------------------
// Case 3 (control): module-level clock, same wait shape.  Must be correct,
// which proves the harness and the expected counts are sound.
// ---------------------------------------------------------------------------
class module_clk_bfm;
  int edges = 0;

  task automatic count_edges();
    forever begin
      @(posedge tb_top.mclk);
      edges++;
    end
  endtask
endclass

module tb_top;
  // NOTE: deliberately NOT named `clk`.  A module-level signal with the same
  // name as the class member masks the bug (the event control then picks up
  // the module signal as well), so the control case uses `mclk`.
  bit mclk = 1'b0;

  member_clk_bfm     c1 = new();
  member_clk_iff_bfm c2 = new();
  module_clk_bfm     c3 = new();

  // Module-level clock (control case).
  always #5 mclk = ~mclk;

  // Class-member clocks: these are the ones that trigger the bug.
  always #5 c1.clk = ~c1.clk;
  always #5 c2.clk = ~c2.clk;

  initial begin
    `SVTEST_INIT

    fork
      c1.count_edges();
      c2.count_edges();
      c3.count_edges();
    join_none

    #40 c2.vld = 1'b1;
    #12;                       // t = 52

    // Control: module-level clock must have counted exactly 5 posedges.
    `SVTEST_CHECK(c3.edges == 5,
                  "control: module-level @(posedge mclk) must count 5 edges in 52 ns")

    // Case 1: class-member clock must also count exactly 5 posedges.
    `SVTEST_CHECK(c1.edges == 5,
                  "class-member @(posedge clk) must count 5 edges in 52 ns")

    // Case 2: `iff` guard must hold the wait back until vld rises at t=40,
    // so only the posedge at t=45 counts.
    `SVTEST_CHECK(c2.edges == 1,
                  "class-member @(posedge clk iff vld) must count 1 edge after vld rises")

    $display("edges: control=%0d member_clk=%0d member_clk_iff=%0d",
             c3.edges, c1.edges, c2.edges);

    `SVTEST_PASSFAIL
    $finish;
  end
endmodule
"#,
    );
    assert!(
        out.iter()
            .any(|m| m == "edges: control=5 member_clk=5 member_clk_iff=1"),
        "{out:#?}"
    );
    assert!(out.iter().any(|m| m == "TEST_PASS"), "{out:#?}");
}

/// Every operand shape: edges, an any-change term, `iff`, `or` lists (one of
/// them mixing a signal), a static, a member chain, a handle-array element, a
/// bit select, `this.` and a static task.
#[test]
fn class_property_edge_operand_shapes_993() {
    let out = messages(
        r#"
// #993 neighbours: edge waits on class properties in class tasks.
class sub_c;
  bit clk = 0;
  bit en = 0;
endclass

class bfm_c;
  bit clk = 0;
  bit vld = 0;
  bit rst = 0;
  logic [3:0] bus = 0;
  static bit sclk = 0;
  sub_c sub = new();
  sub_c subs[2];
  int n_pe, n_ne, n_any, n_iff, n_or, n_mix, n_stat, n_chain, n_arr, n_bus, n_this, n_lv;
  int n_auto, n_mixiff;
  function new(); subs[0] = new(); subs[1] = new(); endfunction
  task automatic pe();    forever begin @(posedge clk); n_pe++; end endtask
  task automatic ne();    forever begin @(negedge clk); n_ne++; end endtask
  task automatic any_e(); forever begin @(edge clk); n_any++; end endtask
  task automatic lv();    forever begin @(clk); n_lv++; end endtask
  task automatic iffv();  forever begin @(posedge clk iff vld); n_iff++; end endtask
  task automatic orl();   forever begin @(posedge clk or posedge rst); n_or++; end endtask
  task automatic mix();   forever begin @(posedge tb_top.mclk or posedge rst); n_mix++; end endtask
  task automatic mixiff(); forever begin @(posedge tb_top.mclk iff vld); n_mixiff++; end endtask
  task automatic stat();  forever begin @(posedge sclk); n_stat++; end endtask
  task automatic chain(); forever begin @(posedge sub.clk); n_chain++; end endtask
  task automatic arr();   forever begin @(posedge subs[1].clk); n_arr++; end endtask
  task automatic busb();  forever begin @(posedge bus[1]); n_bus++; end endtask
  task automatic thisc(); forever begin @(posedge this.clk); n_this++; end endtask
  task static stat_task(); forever begin @(posedge clk); n_auto++; end endtask
endclass

module tb_top;
  bit mclk = 0;
  bfm_c b = new();
  always #5 mclk = ~mclk;
  always #5 b.clk = ~b.clk;
  always #5 bfm_c::sclk = ~bfm_c::sclk;
  always #5 b.sub.clk = ~b.sub.clk;
  always #5 b.subs[1].clk = ~b.subs[1].clk;
  always #5 b.bus = b.bus + 1;
  initial begin
    fork
      b.pe(); b.ne(); b.any_e(); b.lv(); b.iffv(); b.orl(); b.mix(); b.mixiff();
      b.stat(); b.chain(); b.arr(); b.busb(); b.thisc(); b.stat_task();
    join_none
    #40 b.vld = 1;
    #3 b.rst = 1;   // t=43
    #9;             // t=52
    $display("pe=%0d ne=%0d any=%0d lv=%0d iff=%0d or=%0d mix=%0d mixiff=%0d stat=%0d chain=%0d arr=%0d bus=%0d this=%0d static_task=%0d",
      b.n_pe, b.n_ne, b.n_any, b.n_lv, b.n_iff, b.n_or, b.n_mix, b.n_mixiff, b.n_stat, b.n_chain, b.n_arr, b.n_bus, b.n_this, b.n_auto);
    $finish;
  end
endmodule
"#,
    );
    let want = "pe=5 ne=5 any=10 lv=10 iff=1 or=6 mix=6 mixiff=1 stat=5 chain=5 \
                arr=5 bus=3 this=5 static_task=5";
    assert!(out.iter().any(|m| m == want), "{out:#?}");
}
