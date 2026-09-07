//! Upward name referencing from bound modules — IEEE 1800-2023 §23.8
//! (upward name referencing) and §23.10.1 (a module instantiated via `bind`
//! resolves names within the scope of the bind TARGET instance, so the
//! target's — and every enclosing scope's — names are reachable upward).
//!
//! Regression for issue #27: xezim inlines a bound module as a regular child
//! of the target (§23.11), but hierarchical names whose first segment named
//! an ENCLOSING scope (by instance name, or by module definition name per
//! §23.8 "the name of a module" = nearest enclosing instance of that module)
//! did not resolve — they fell through to the unqualified-leaf fallback and
//! read X. The simulator's name resolution now retries unresolved dotted
//! names with a §23.8 upward walk over the executing scope's ancestor chain.

use xezim::simulate;

/// Issue #27 shape: a monitor bound into an empty leaf (`target_core`) reads
/// values from the bind target's enclosing scopes by MODULE DEFINITION name
/// (`dut_top.top_secret`, `sub_block.sub_secret`, §23.8 flavor) and from a
/// package (`my_pkg::pkg_secret`).
#[test]
fn bound_module_reads_upward_by_module_name() {
    const SRC: &str = r#"
package my_pkg;
  int pkg_secret = 99;
endpackage

module dut_top;
  int top_secret = 42;
  sub_block u_sub_block();
endmodule

module sub_block;
  int sub_secret = 7;
  target_core u_target_core();
endmodule

module target_core;
  // empty in the original design; receives the bound monitor
endmodule

module bind_monitor;
  int got_top = -1;
  int got_sub = -1;
  int got_pkg = -1;
  initial begin
    #1;
    // §23.8 upward references via module definition names.
    got_top = dut_top.top_secret;
    got_sub = sub_block.sub_secret;
    // Package scope reference (worked before the fix; keep it covered).
    got_pkg = my_pkg::pkg_secret;
  end
endmodule

bind target_core bind_monitor u_mon();

module tb;
  dut_top u_dut();
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    let read = |name: &str| -> i64 {
        sim.get_signal(name)
            .unwrap_or_else(|| panic!("signal {name} not found"))
            .to_u64()
            .unwrap_or_else(|| panic!("signal {name} is X/Z")) as i64
    };
    let base = "u_dut.u_sub_block.u_target_core.u_mon";
    assert_eq!(read(&format!("{base}.got_top")), 42, "dut_top.top_secret");
    assert_eq!(read(&format!("{base}.got_sub")), 7, "sub_block.sub_secret");
    assert_eq!(read(&format!("{base}.got_pkg")), 99, "my_pkg::pkg_secret");
}

/// §23.8: an upward reference by module name binds to the NEAREST enclosing
/// instance of that module. With the target instantiated under two different
/// `wrapper` instances (different parameterizations), each bound monitor
/// must read ITS OWN enclosing wrapper's value — not the first instance's.
#[test]
fn bound_module_upward_ref_is_per_instance() {
    const SRC: &str = r#"
module wrapper #(parameter int SECRET = 0);
  int secret = SECRET;
  target_core u_core();
endmodule

module target_core;
endmodule

module watcher;
  int got = -1;
  initial begin
    #1;
    // §23.8: resolves to the nearest enclosing instance of module `wrapper`
    // — a different instance for each of the two bound copies.
    got = wrapper.secret;
  end
endmodule

bind target_core watcher u_w();

module tb;
  wrapper #(.SECRET(11)) u_w1();
  wrapper #(.SECRET(22)) u_w2();
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    let read = |name: &str| -> i64 {
        sim.get_signal(name)
            .unwrap_or_else(|| panic!("signal {name} not found"))
            .to_u64()
            .unwrap_or_else(|| panic!("signal {name} is X/Z")) as i64
    };
    assert_eq!(read("u_w1.u_core.u_w.got"), 11, "monitor under u_w1");
    assert_eq!(read("u_w2.u_core.u_w.got"), 22, "monitor under u_w2");
}

/// Upward references must also work as WRITE targets (§23.8 references are
/// ordinary hierarchical names, usable on either side of an assignment), and
/// the first segment may name an instance declared in an enclosing scope
/// (§23.10.1: names visible in the bind target's scope chain).
#[test]
fn bound_module_writes_upward() {
    const SRC: &str = r#"
module dut_top;
  int ctrl = 0;
  sub_block u_sub();
endmodule

module sub_block;
  int sctrl = 0;
  target_core u_core();
endmodule

module target_core;
endmodule

module poker;
  initial begin
    #2;
    // Upward write via module definition name (§23.8).
    dut_top.ctrl = 123;
    // Upward write via an instance name declared in an enclosing scope
    // (`u_sub` lives in dut_top, two levels above the bound instance).
    u_sub.sctrl = 55;
  end
endmodule

bind target_core poker u_poke();

module tb;
  dut_top u_dut();
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    let read = |name: &str| -> i64 {
        sim.get_signal(name)
            .unwrap_or_else(|| panic!("signal {name} not found"))
            .to_u64()
            .unwrap_or_else(|| panic!("signal {name} is X/Z")) as i64
    };
    assert_eq!(read("u_dut.ctrl"), 123, "upward write by module name");
    assert_eq!(read("u_dut.u_sub.sctrl"), 55, "upward write by instance name");
}

/// `assert ... else` inside a bound monitor reading an UPWARD
/// instance-anchored reference (`subsys.int_req_addr`, §23.8/§23.10.1).
/// The elaborator's final rewrite pass had no `Assertion` arm, so the whole
/// statement fell through un-rewritten: the condition's dotted reference
/// stayed a raw `MemberAccess` node (never collapsed to a hierarchical
/// Ident) and the interpreter read it as a nonexistent object property — 0.
/// The assert falsely fired and ran its else-action, while the very same
/// reference in a plain `if` (which IS rewritten) resolved fine.
/// Reference-verified: seen=1 bad=0.
#[test]
fn assert_else_in_bound_module_resolves_upward_ref() {
    const SRC: &str = r#"
module producer(input wire clk, input wire rst_l, output logic [31:0] addr, output logic vld);
  always @(posedge clk) begin
    if (!rst_l) begin addr <= 32'h8000; vld <= 1'b0; end
    else begin addr <= addr + 32'h100; vld <= 1'b1; end
  end
endmodule

module subsys(input wire clk, input wire rst_l);
  wire [31:0] int_req_addr; wire int_req_vld;
  producer p(.clk(clk), .rst_l(rst_l), .addr(int_req_addr), .vld(int_req_vld));
endmodule

module mon_unit(input wire clk, input wire rst_l);
  logic [31:0] lo_lim, hi_lim; logic vld_r; int bad_count; int seen_count;
  always @(posedge clk) begin
    if (!rst_l) begin
      vld_r <= 1'b0; bad_count <= 0; seen_count <= 0;
      lo_lim <= 32'h8000; hi_lim <= 32'h9000;
    end else begin
      vld_r <= subsys.int_req_vld;
      if (vld_r) begin
        seen_count <= seen_count + 1;
        assert (subsys.int_req_addr >= lo_lim && subsys.int_req_addr < hi_lim)
        else begin
          bad_count <= bad_count + 1;
          $error("illegal addr 0x%0x", subsys.int_req_addr);
        end
      end
    end
  end
endmodule

module mon_harness(input wire clk, input wire rst_l);
  mon_unit mon(.clk(clk), .rst_l(rst_l));
endmodule

bind subsys mon_harness h (.*);

module tb;
  logic clk, rst_l;
  initial begin clk = 1'b0; repeat (8) #5 clk = ~clk; end
  initial begin rst_l = 1'b0; #10 rst_l = 1'b1; end
  subsys subsys(.clk(clk), .rst_l(rst_l));
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    let read = |name: &str| -> i64 {
        sim.get_signal(name)
            .unwrap_or_else(|| panic!("signal {name} not found"))
            .to_u64()
            .unwrap_or_else(|| panic!("signal {name} is X/Z")) as i64
    };
    assert_eq!(read("subsys.h.mon.seen_count"), 1, "monitor sampled");
    assert_eq!(
        read("subsys.h.mon.bad_count"),
        0,
        "assert falsely fired: upward ref in the condition read 0"
    );
}

/// Sibling sweep of the missing-Assertion-arm bug: `do-while`, `wait`,
/// block-local decl initializers, `randcase`, and `force` also had no
/// `rewrite_stmt` arm, so their dotted upward references stayed raw
/// `MemberAccess` nodes. Most shapes were rescued by interpreter fallbacks;
/// `force tgt = host.wire` observably broke — and exposed a second gap:
/// `refresh_active_forces` ran only at settle START, so a tracked operand
/// that is itself comb-driven (a wire copying a child port) bumped its
/// epoch after the refresh and the override went stale. Reference-verified:
/// wait=0x8100 dw=0x8100 decl=0x8200 rc=0x8200 force=0x8300.
#[test]
fn bound_module_sibling_stmt_kinds_resolve_upward_refs() {
    const SRC: &str = r#"
module producer(input wire clk, input wire rst_l, output logic [31:0] addr, output logic vld);
  always @(posedge clk) begin
    if (!rst_l) begin addr <= 32'h8000; vld <= 1'b0; end
    else begin addr <= addr + 32'h100; vld <= 1'b1; end
  end
endmodule

module subsys(input wire clk, input wire rst_l);
  wire [31:0] int_req_addr; wire int_req_vld;
  producer p(.clk(clk), .rst_l(rst_l), .addr(int_req_addr), .vld(int_req_vld));
endmodule

module mon_unit(input wire clk, input wire rst_l);
  logic [31:0] dw_got, wait_got, decl_got, rc_got, force_tgt;
  initial begin
    dw_got = 0; wait_got = 0; decl_got = 0; rc_got = 0; force_tgt = 0;
    wait (subsys.int_req_vld === 1'b1);
    wait_got = subsys.int_req_addr;
    do begin
      dw_got = subsys.int_req_addr;
      @(posedge clk);
    end while (subsys.int_req_addr < 32'h8200);
    begin
      automatic logic [31:0] snap = subsys.int_req_addr;
      decl_got = snap;
    end
    randcase
      subsys.int_req_vld : rc_got = subsys.int_req_addr;
      0 : rc_got = 32'hdead;
    endcase
    force force_tgt = subsys.int_req_addr;
    #1 release force_tgt;
  end
endmodule

module mon_harness(input wire clk, input wire rst_l);
  mon_unit mon(.clk(clk), .rst_l(rst_l));
endmodule

bind subsys mon_harness h (.*);

module tb;
  logic clk, rst_l;
  initial begin clk = 1'b0; repeat (12) #5 clk = ~clk; end
  initial begin rst_l = 1'b0; #10 rst_l = 1'b1; end
  subsys subsys(.clk(clk), .rst_l(rst_l));
endmodule
"#;
    let sim = simulate(SRC, 200).expect("simulate failed");
    let read = |name: &str| -> u64 {
        sim.get_signal(&format!("subsys.h.mon.{name}"))
            .unwrap_or_else(|| panic!("signal {name} not found"))
            .to_u64()
            .unwrap_or_else(|| panic!("signal {name} is X/Z"))
    };
    assert_eq!(read("wait_got"), 0x8100, "wait condition/body");
    assert_eq!(read("dw_got"), 0x8100, "do-while condition/body");
    assert_eq!(read("decl_got"), 0x8200, "block-local decl initializer");
    assert_eq!(read("rc_got"), 0x8200, "randcase weight/body");
    assert_eq!(
        read("force_tgt"),
        0x8300,
        "continuous force must track a comb-driven upward operand"
    );
}
