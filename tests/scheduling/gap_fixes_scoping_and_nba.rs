//! Four gaps closed in one sweep, all previously recorded as known-broken.
//! Each is pinned against reference-simulator behaviour.
//!
//! 1. §7.8, §10.4.2 — blocking writes to associative-array elements with
//!    string and integer keys retain the full element value.
//!
//! 2. §26.2.2 — `import pkg::MEMBER;` where MEMBER is an enum member died
//!    with "Symbol not found in package": the explicit-import resolver
//!    matched typedef NAMES but never looked inside an enum typedef. It now
//!    registers the one member (plus the typedef's ordered member list, which
//!    `.name()`/`.next()` resolve through).
//!
//! 3. A `parameter` re-declaring a name already taken by a module-local enum
//!    member was silently ACCEPTED — the parameter pre-seed fixpoint's
//!    duplicate exemption vouched for its own seeded value without noticing
//!    the name had a prior declaration site. Every other simulator rejects
//!    it; now xezim does too, naming both declarations.
//!
//! 4. Cross-module bare-name leak: with `shadower` declaring a local over a
//!    wildcard-imported enum member and a SIBLING `user` reading the member
//!    bare, user read shadower's local (truncated to the enum width). Root
//!    cause: the compile-time clock-generator detection EVALUATES each
//!    initial block's seed RHS, and that eval memoizes bare-name resolution
//!    on the AST node — under whatever `name_resolve_hint` the previous
//!    block left. The detection now installs the block's own scope first
//!    (and `run_process_stmts` resets the hint per activation as hygiene).

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// Gap 1: associative element writes with a string-literal key, a string-variable key,
/// an int key, and a wide (96-bit) element value.
#[test]
fn associative_element_writes_store_full_values() {
    let src = r#"
module top;
  bit clk;
  always #5 clk = ~clk;
  int aa [string];
  int by_int [int];
  logic [95:0] wide_aa [int];
  string k;
  int r_lit, r_str, r_int, r_wide_ok;
  initial begin
    k = "gamma";
    @(posedge clk);
    aa["alpha"] = 11;
    aa[k]       = 22;
    by_int[7]   = 33;
    wide_aa[3]  = 96'hDEAD_BEEF_0123_4567_89AB_CDEF;
    @(posedge clk);
    r_lit = aa["alpha"];
    r_str = aa["gamma"];
    r_int = by_int[7];
    r_wide_ok = (wide_aa[3] === 96'hDEAD_BEEF_0123_4567_89AB_CDEF);
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(
        u(&sim, "r_lit"),
        11,
        "string-literal key, full value not the low bit"
    );
    assert_eq!(u(&sim, "r_str"), 22, "string-variable key");
    assert_eq!(u(&sim, "r_int"), 33, "int key");
    assert_eq!(u(&sim, "r_wide_ok"), 1, "wide element survives intact");
}

/// Gap 2: explicit import of an enum member by name, including `.name()`.
#[test]
fn explicit_import_of_an_enum_member_resolves() {
    let src = r#"
package ep;
  typedef enum logic [1:0] { IDLE = 2'b00, BUSY = 2'b01, DONE = 2'b10 } state_e;
endpackage
module top;
  import ep::BUSY;
  import ep::state_e;
  state_e v;
  int r, name_ok;
  initial begin
    v = BUSY;
    r = BUSY;
    name_ok = (v.name() == "BUSY");
  end
endmodule
"#;
    let sim = simulate(src, 5).expect("import ep::BUSY must elaborate");
    assert_eq!(u(&sim, "v"), 1, "imported member carries its value");
    assert_eq!(u(&sim, "r"), 1, "usable in any expression");
    assert_eq!(
        u(&sim, "name_ok"),
        1,
        ".name() resolves through the typedef"
    );
}

/// Gap 3: a parameter colliding with a module-local enum member is a
/// duplicate declaration, and the error names the enum member as the first.
#[test]
fn parameter_over_local_enum_member_is_a_duplicate() {
    let src = r#"
module top;
  typedef enum logic [1:0] { IDLE, BUSY, DONE } st_e;
  parameter int BUSY = 5;
  initial $display("%0d", BUSY);
endmodule
"#;
    let err = match simulate(src, 5) {
        Ok(_) => panic!("must reject, as every other simulator does"),
        Err(e) => e,
    };
    assert!(
        err.contains("duplicate declaration of 'BUSY'"),
        "got: {err}"
    );
    assert!(
        err.contains("enum member of 'st_e'"),
        "must name the enum member as the first declaration; got: {err}"
    );
}

/// Gap 4: the sibling module's bare use of the member keeps the ENUM value
/// even when another instance shadows the name — and regardless of which
/// process happens to run first (the trigger needed shadower's process to
/// execute a read + $display before user's).
#[test]
fn sibling_shadow_does_not_capture_another_modules_bare_name() {
    let src = r#"
package pk;
  typedef enum logic [1:0] { A = 2'b00, B = 2'b01, C = 2'b10 } e_t;
endpackage
module shadower;
  import pk::*;
  int C = 900;
  int seen_c;
  initial begin seen_c = C; $display("shadower C=%0d", seen_c); end
endmodule
module user;
  import pk::*;
  e_t v;
  int name_ok;
  initial begin
    v = C;
    name_ok = (v.name() == "C");
  end
endmodule
module top;
  shadower s();
  user u();
endmodule
"#;
    let sim = simulate(src, 5).expect("simulate failed");
    assert_eq!(u(&sim, "s.seen_c"), 900, "shadower keeps its own local");
    assert_eq!(
        u(&sim, "u.v"),
        2,
        "user's bare C is the enum member, not the sibling's local"
    );
    assert_eq!(u(&sim, "u.name_ok"), 1, "and stringifies as the member");
}
