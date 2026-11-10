//! §26.2/§26.3 — four related failures of the global bare-name type/param
//! tables, found by differential audit against a reference simulator on a
//! large design. All four share one root: the top-module path resolves widths
//! once, from global tables, in HashMap order, before cross-package values
//! exist — and is never fixpointed the way inlined children are.
//!
//! 1. PACKAGE PARAMETERS only materialized on IMPORT, so a package typedef
//!    sized by another package's parameter (`LOG = $clog2(cfg::DEPTH) - 6`)
//!    computed its fields from 0: LOG = -6 stored unsigned, each `[LOG-1:0]`
//!    field clamped to 1 bit, and a 116-bit struct port's testbench side
//!    narrowed to 86 (five 7-bit fields x 6 lost bits = -30).
//! 2. $UNIT typedefs were processed BEFORE any package, so a file-scope
//!    `typedef struct { P::t w; ... }` fell back to 32 bits for the member
//!    and was never revisited.
//! 3. A sub-module `parameter type` DEFAULT is registered under its bare name
//!    design-wide during that instance's inlining (restored after). A scope
//!    whose visibility comes only through a CHAINED wildcard import
//!    (`import defs_pkg::*` where defs_pkg imports and exports `core_pkg::*`) was
//!    not re-bound, so a declaration resolved inside the window carved
//!    dims x default-width instead of dims x struct-width.
//! 4. A CLASS-local typedef stomped an existing package type's bare slot
//!    during the design-wide hoist (HashMap order), poisoning top-module
//!    declarations sized between the stomp and the heal.
//!
//! The signature shared by 3 and 4 — and the reason runtime probes could not
//! see it — is that the STORAGE is carved wrong while every later reading of
//! the tables is healthy.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

/// (1) Cross-package parameter dependency, referenced SCOPED and never
/// imported. 5 x [LOG-1:0] fields + an 81-bit tail: LOG must be 7 -> 116.
#[test]
fn package_param_resolves_across_packages_without_import() {
    let src = r#"
package depth_cfg;
  parameter BUF_DEPTH = 8192;
endpackage
package bus_defs;
  parameter bit [31:0] LOG_BUF = $clog2(depth_cfg::BUF_DEPTH) - 6;
  typedef struct packed {
    logic [LOG_BUF-1:0] a, b, c, d, e;
    logic [80:0] tail;
  } bus_req_t;
endpackage
module sink (output bus_defs::bus_req_t req);
  assign req = '0;
endmodule
module tb;
  import bus_defs::*;
  bus_req_t req;
  sink u_s (.req(req));
  logic [31:0] w_req, w_t, w_log;
  assign w_req = $bits(req);
  assign w_t   = $bits(bus_req_t);
  assign w_log = LOG_BUF;
  initial #1;
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "w_log"), 7, "$clog2(8192)-6 across packages");
    assert_eq!(u(&sim, "w_t"), 116, "5*7 + 81");
    assert_eq!(
        u(&sim, "w_req"),
        116,
        "declaration carved from the healed value"
    );
}

/// (2) $unit typedef whose member names a package type.
#[test]
fn unit_typedef_sees_package_member_types() {
    let src = r#"
package core_pkg;
  typedef struct packed { logic [1:0][63:0] lanes; logic [17:0] meta; } core_wr_t; // 146
endpackage
typedef struct packed { core_pkg::core_wr_t w; logic [1:0] tag; } unit_req_t;      // 148
module sink (output unit_req_t r);
  assign r = '0;
endmodule
module tb;
  unit_req_t r;
  sink u_s (.r(r));
  logic [31:0] w_r, w_t;
  assign w_r = $bits(r);
  assign w_t = $bits(unit_req_t);
  initial #1;
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(
        u(&sim, "w_t"),
        148,
        "member resolved via the package, not 32"
    );
    assert_eq!(u(&sim, "w_r"), 148);
}

/// (3) Chained wildcard import + a type-parameter default sharing the type's
/// name. The declaration scope reaches core_pkg only THROUGH defs_pkg, which
/// exports it (§26.6).
#[test]
fn chained_import_scope_survives_type_param_default_window() {
    let src = r#"
package core_pkg;
  typedef struct packed {
    logic [1:0][63:0] lanes;
    logic [1:0][7:0]  mask;
    logic [1:0]       en;
  } wr_burst_t;                                   // 146
endpackage
package defs_pkg;
  import core_pkg::*;
  export core_pkg::*;
  localparam int WR_W = $bits(wr_burst_t);
endpackage
module inner;
  import defs_pkg::*;                              // chained: never names core_pkg
  wr_burst_t [0:0][1:0] wr_bus;
  logic [31:0] w_bus, w_elem, w_type;
  assign w_bus  = $bits(wr_bus);
  assign w_elem = $bits(wr_bus[0][0]);
  assign w_type = $bits(wr_burst_t);
endmodule
module wrapper #(
  parameter int W = 4,
  parameter type wr_burst_t = logic [63:0]         // same NAME, 64-bit default
) ();
  logic [W-1:0] cnt;
  wr_burst_t q;
  inner u_i ();
endmodule
module tb;
  wrapper #(.W(8)) u_w ();
  initial #1;
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(
        u(&sim, "u_w.u_i.w_bus"),
        292,
        "carved inside the window: was 128"
    );
    assert_eq!(u(&sim, "u_w.u_i.w_elem"), 146, "was 1");
    assert_eq!(u(&sim, "u_w.u_i.w_type"), 146);
}

/// §26.6: a declaration a package imports is not visible through a wildcard
/// import of that package unless the package exports it.
#[test]
fn a_chained_import_without_an_export_does_not_make_the_type_visible() {
    let src = r#"
package core_pkg;
  typedef struct packed {
    logic [1:0][63:0] lanes;
    logic [1:0][7:0]  mask;
    logic [1:0]       en;
  } wr_burst_t;                                   // 146
endpackage
package defs_pkg;
  import core_pkg::*;
  localparam int WR_W = $bits(wr_burst_t);
endpackage
module inner;
  import defs_pkg::*;                              // chained: never names core_pkg
  wr_burst_t [0:0][1:0] wr_bus;
  logic [31:0] w_bus, w_elem, w_type;
  assign w_bus  = $bits(wr_bus);
  assign w_elem = $bits(wr_bus[0][0]);
  assign w_type = $bits(wr_burst_t);
endmodule
module wrapper #(
  parameter int W = 4,
  parameter type wr_burst_t = logic [63:0]         // same NAME, 64-bit default
) ();
  logic [W-1:0] cnt;
  wr_burst_t q;
  inner u_i ();
endmodule
module tb;
  wrapper #(.W(8)) u_w ();
  initial #1;
endmodule
"#;
    let err = simulate(src, 50)
        .err()
        .expect("wr_burst_t is not visible in inner");
    assert!(err.contains("Undeclared identifier 'wr_burst_t'"), "{err}");
}

/// (4) Class-local typedef of the same name must not stomp the package type.
/// The class is declared AFTER the package so, pre-fix, whichever hashed later
/// in the hoist decided the testbench declaration's width nondeterministically.
#[test]
fn class_local_typedef_does_not_stomp_the_package_type() {
    let src = r#"
package core_pkg;
  typedef struct packed {
    logic [1:0][63:0] lanes;
    logic [1:0][7:0]  mask;
    logic [1:0]       en;
  } wr_burst_t;                                    // 146
endpackage
package verif_pkg;
  class scoreboard;
    typedef logic [63:0] wr_burst_t;               // class-LOCAL, 64
    function wr_burst_t zero(); return '0; endfunction
  endclass
endpackage
module tb;
  import core_pkg::*;
  wr_burst_t [0:0][1:0] wr_bus;
  logic [31:0] w_bus, w_elem;
  assign w_bus  = $bits(wr_bus);
  assign w_elem = $bits(wr_bus[0][0]);
  initial #1;
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "w_bus"), 292, "package type keeps the bare slot");
    assert_eq!(u(&sim, "w_elem"), 146);
}
