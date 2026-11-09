//! #988 (reported by Ganesh-TNL): event controls and `wait`s on members of
//! virtual interfaces held in class properties (IEEE 1800-2023 §9.4.2,
//! §9.7.4, §25.9, §25.10). Two causes:
//! * an index naming a class PROPERTY (`this.pifc[j] = src[j]` with `j` a
//!   property, `@(posedge pifc[idx].clk)`, `pifc[n-1]`, `pk[this.idx]`,
//!   `pk[cfg.idx]`, a static `pk[sidx]`) evaluated to nothing in the
//!   side-effect-free evaluator the virtual-interface resolution uses, so the
//!   copied element was null and the wait armed on nothing and spun;
//! * `@(vif)` / `@(vifs[0])` on the virtual-interface VARIABLE (§9.4.2 a
//!   change of its value, §25.9 the instance it refers to) read a constant
//!   or armed on nothing: it must resume when the variable is assigned
//!   another instance, and only then.
//!
//! Expected values from the reference simulator.

use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect()
}

/// The reporter's reproducer: 39 monitors, every check passes.
#[test]
fn vif_property_event_controls_988() {
    let out = messages(include_str!("vif_property_event_controls_988.sv"));
    let fails: Vec<&String> = out.iter().filter(|m| m.starts_with("FAIL")).collect();
    assert!(fails.is_empty(), "FAIL lines: {fails:#?}");
    assert!(out.iter().any(|m| m == "TEST_PASS"), "{out:#?}");
}

/// A copy loop indexed by a property, and waits indexed by a property or an
/// expression of one.
#[test]
fn vif_element_copied_and_waited_through_property_index_988() {
    let out = messages(
        r#"
`timescale 1ns/1ps
interface U (input clk);
  logic sig;
  modport MP (input clk, input sig);
endinterface
class C;
  int j;
  virtual U.MP pj[];     // copied with a class-property index
  virtual U.MP pk[];     // copied with an automatic index
  virtual U.MP one;
  int idx;
  int n_pj, n_pk, n_one, n_var, n_expr, n;
  function new(int n, virtual U src[]);
    this.n = n;
    pj = new[n]; pk = new[n];
    for (j = 0; j < n; j++) this.pj[j] = src[j];
    for (int k = 0; k < n; k++) this.pk[k] = src[k];
    one = this.pj[0];
  endfunction
  task t_pj();  forever begin @(posedge pj[0].clk); n_pj++; end endtask
  task t_pk();  forever begin @(posedge pk[0].clk); n_pk++; end endtask
  task t_one(); forever begin @(posedge one.clk); n_one++; end endtask
  task t_var(); forever begin idx = 1; @(posedge pk[idx].clk); n_var++; end endtask
  task t_expr(); forever begin @(posedge pk[n-1].clk); n_expr++; end endtask
endclass
module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  U u[3] (clk);
  C c;
  initial begin
    c = new(3, u);
    $display("pj0=%0d pk0=%0d one_null=%0d same=%0d", c.pj[0] == null, c.pk[0] == null, c.one == null, c.pj[0] == c.pk[0]);
    fork c.t_pj(); c.t_pk(); c.t_one(); c.t_var(); c.t_expr(); join_none
    #52;
    $display("pj=%0d pk=%0d one=%0d var=%0d expr=%0d", c.n_pj, c.n_pk, c.n_one, c.n_var, c.n_expr);
    $finish;
  end
endmodule
"#,
    );
    assert_eq!(
        out[..2],
        [
            "pj0=0 pk0=0 one_null=0 same=1",
            "pj=5 pk=5 one=5 var=5 expr=5"
        ]
    );
}

/// Index spellings (`this.idx`, `cfg.idx`, a static, a static task's
/// variable), `or`, `iff`, `wait`, fixed-size and queue properties, and a
/// module-scope wait through a handle.
#[test]
fn vif_element_index_shapes_988() {
    let out = messages(
        r#"
`timescale 1ns/1ps
interface U (input clk);
  logic sig = 0;
endinterface
class Cfg; int idx = 1; endclass
class C;
  virtual U pk[];
  virtual U fx[3];
  virtual U q[$];
  int idx = 1;
  static int sidx = 2;
  Cfg cfg = new();
  int n_this, n_cfg, n_static, n_stask, n_or, n_iff, n_wait, n_fx, n_q, n_mod;
  function new(virtual U src[]);
    pk = new[3];
    foreach (src[k]) begin pk[k] = src[k]; fx[k] = src[k]; q.push_back(src[k]); end
  endfunction
  task t_this();   forever begin @(posedge pk[this.idx].clk); n_this++; end endtask
  task t_cfg();    forever begin @(posedge pk[cfg.idx].clk); n_cfg++; end endtask
  task t_static(); forever begin @(posedge pk[sidx].clk); n_static++; end endtask
  task static t_stask(); int k; k = 2; forever begin @(posedge pk[k].clk); n_stask++; end endtask
  task t_or();     forever begin @(posedge pk[idx].clk or posedge pk[idx+1].sig); n_or++; end endtask
  task t_iff();    forever begin @(posedge pk[idx].clk iff pk[idx].sig); n_iff++; end endtask
  task t_wait();   forever begin wait (pk[idx+1].sig === 1); n_wait++; #1; end endtask
  task t_fx();     forever begin @(posedge fx[idx].clk); n_fx++; end endtask
  task t_q();      forever begin @(posedge q[idx].clk); n_q++; end endtask
endclass
module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  U u[3] (clk);
  C c;
  int j = 1;
  initial begin
    c = new(u);
    fork
      c.t_this(); c.t_cfg(); c.t_static(); c.t_stask(); c.t_or(); c.t_iff(); c.t_wait(); c.t_fx(); c.t_q();
      forever begin @(posedge c.pk[j].clk); c.n_mod++; end
    join_none
    #23 u[1].sig = 1; u[2].sig = 1;
    #29;
    $display("this=%0d cfg=%0d static=%0d stask=%0d or=%0d iff=%0d wait=%0d fx=%0d q=%0d mod=%0d",
      c.n_this, c.n_cfg, c.n_static, c.n_stask, c.n_or, c.n_iff, c.n_wait, c.n_fx, c.n_q, c.n_mod);
    $finish;
  end
endmodule
"#,
    );
    assert_eq!(
        out[0],
        "this=5 cfg=5 static=5 stask=5 or=6 iff=3 wait=29 fx=5 q=5 mod=5"
    );
}

/// `@(v)` and `@(va[0])` resume once, when the variable is reassigned at 53,
/// not on the interface's own signal changes.
#[test]
fn whole_vif_event_control_waits_for_reassignment_988() {
    let out = messages(
        r#"
interface U (input clk);
  logic sig = 0;
endinterface
class C;
  virtual U v;
  virtual U va[];
  int n_v, n_va;
  task t_v();  forever begin @(v); n_v++; $display("t=%0t @(v) woke", $time); end endtask
  task t_va(); forever begin @(va[0]); n_va++; $display("t=%0t @(va[0]) woke", $time); end endtask
endclass
module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  U u[2] (clk);
  always #10 u[0].sig = ~u[0].sig;
  C c = new();
  initial begin
    c.v = u[0]; c.va = new[1]; c.va[0] = u[0];
    fork c.t_v(); c.t_va(); join_none
    #53 c.v = u[1]; c.va[0] = u[1];
    #20 $display("n_v=%0d n_va=%0d", c.n_v, c.n_va);
    $finish;
  end
endmodule
"#,
    );
    assert_eq!(
        out[..3],
        ["t=53 @(v) woke", "t=53 @(va[0]) woke", "n_v=1 n_va=1"]
    );
}

/// The same through a handle (`@(cfg.vif)`), in an `or` list, at module
/// scope, and a `wait` on the handle turning non-null.
#[test]
fn whole_vif_event_control_shapes_988() {
    let out = messages(
        r#"
interface U (input clk);
  logic sig = 0;
endinterface
class Cfg; virtual U vif; endclass
class C;
  Cfg cfg = new();
  virtual U v, w;
  int n_chain, n_or, n_null;
  task t_chain(); forever begin @(cfg.vif); n_chain++; $display("t=%0t chain woke", $time); end endtask
  task t_or();    forever begin @(v or w); n_or++; $display("t=%0t or woke", $time); end endtask
  task t_null();  wait (w != null); n_null++; endtask
endclass
module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  U u[2] (clk);
  always #10 u[0].sig = ~u[0].sig;
  C c = new();
  virtual U mv;
  int n_mod;
  initial begin
    c.v = u[0]; c.cfg.vif = u[0]; mv = u[0];
    fork c.t_chain(); c.t_or(); c.t_null(); forever begin @(mv); n_mod++; $display("t=%0t module woke", $time); end join_none
    #23 c.cfg.vif = u[1];
    #10 c.w = u[1];
    #10 mv = u[1];
    #10 c.v = u[1];
    #10 $display("chain=%0d or=%0d null=%0d mod=%0d", c.n_chain, c.n_or, c.n_null, n_mod);
    $finish;
  end
endmodule
"#,
    );
    assert_eq!(
        out[..5],
        [
            "t=23 chain woke",
            "t=33 or woke",
            "t=43 module woke",
            "t=53 or woke",
            "chain=1 or=2 null=1 mod=1"
        ]
    );
}
