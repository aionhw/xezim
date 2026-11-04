//! IEEE 1800-2017 §13.4.3 / §7.2 constant functions that build a PACKED STRUCT
//! configuration, the CVA6 `build_config` idiom:
//!
//! ```text
//! localparam cfg_t Cfg = build_config(user_cfg());   // §13.4.3
//! cva6 #(.CVA6Cfg(Cfg)) dut (...);                     // [CVA6Cfg.XLEN-1:0]
//! ```
//!
//! The constant-function evaluator could neither read nor write a struct
//! formal/local by member (`c.XLEN = u.XLEN`), and a struct-typed package
//! parameter with an assignment pattern (`localparam user_t def = '{XLEN: 64,
//! ...}`, §10.9.2) was still 0 when a module localparam read it. Either way the
//! localparam elaborated as 0, every `[Cfg.XLEN-1:0]` port collapsed to 2 bits,
//! and CVA6 booted from address 0.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn struct_config_function_sizes_child_ports() {
    let sim = simulate(
        r#"
package cfg_pkg;
  typedef struct packed { int unsigned XLEN; int unsigned VLEN; bit RVF; } user_t;
  typedef struct packed { int unsigned XLEN; int unsigned VLEN; bit RVF; bit IS64; } cfg_t;
  localparam user_t def = '{XLEN: 64, VLEN: 39, RVF: 1};
  function automatic cfg_t build(user_t u);
    cfg_t c;
    c.XLEN = u.XLEN;
    c.VLEN = u.VLEN;
    c.RVF = bit'(u.RVF);
    c.IS64 = (u.XLEN == 64);
    return c;
  endfunction
endpackage
module core #(parameter cfg_pkg::cfg_t Cfg = '0) (
  input logic [Cfg.VLEN-1:0] boot_addr_i,
  input logic [Cfg.XLEN-1:0] hart_id_i
);
  initial #1 $display("W=%0d,%0d boot=%h rvf=%0d is64=%0d",
                      $bits(boot_addr_i), $bits(hart_id_i), boot_addr_i, Cfg.RVF, Cfg.IS64);
endmodule
module top;
  function automatic cfg_pkg::user_t user_cfg();
    cfg_pkg::user_t c = cfg_pkg::def;
    c.RVF = 0;
    return c;
  endfunction
  localparam cfg_pkg::cfg_t Cfg = cfg_pkg::build(user_cfg());
  core #(.Cfg(Cfg)) dut (.boot_addr_i(39'h80000000), .hart_id_i(64'd0));
  initial #2 $finish;
endmodule
"#,
        100,
    )
    .expect("sim");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "W=39,64 boot=0080000000 rvf=0 is64=1"),
        "struct config function lost at elaboration; output: {:?}",
        msgs
    );
}

#[test]
fn struct_returning_call_as_instance_override() {
    // The call written directly in the override, and a package struct
    // parameter passed straight through (§10.9.2 package pattern).
    let sim = simulate(
        r#"
package p;
  typedef struct packed { int unsigned W; bit E; } cfg_t;
  localparam cfg_t base = '{W: 17, E: 1};
  function automatic cfg_t build(int w);
    cfg_t c; c.W = w; c.E = 0; return c;
  endfunction
  function automatic int width_of(cfg_t c); return c.W; endfunction
endpackage
module child #(parameter p::cfg_t C = '0) (input logic [C.W-1:0] d);
  initial #1 $display("C=%0d", $bits(d));
endmodule
module top;
  localparam int BW = p::width_of(p::base);
  logic [BW-1:0] bw;
  child #(.C(p::build(12))) u (.d(12'd5));
  initial #1 $display("BW=%0d", $bits(bw));
  initial #2 $finish;
endmodule
"#,
        100,
    )
    .expect("sim");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "C=12") && msgs.iter().any(|m| m == "BW=17"),
        "struct const-fn override wrong; output: {:?}",
        msgs
    );
}

#[test]
fn scoped_typed_assignment_pattern_parses() {
    // §10.9.2: `pkg::type'{...}` — the scoped prefix used to be a parse error.
    let sim = simulate(
        r#"
package p;
  typedef struct packed { logic [7:0] a; logic [7:0] b; } pair_t;
endpackage
module top;
  localparam p::pair_t P = p::pair_t'{a: 8'h12, b: 8'h34};
  p::pair_t v;
  initial begin
    v = p::pair_t'{b: 8'hcd, a: 8'hab};
    $display("P=%h v=%h", P, v);
  end
endmodule
"#,
        100,
    )
    .expect("sim");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "P=1234 v=abcd"),
        "scoped typed assignment pattern wrong; output: {:?}",
        msgs
    );
}
