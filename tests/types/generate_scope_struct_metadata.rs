//! §27.4 — packed-struct metadata for declarations inside GENERATE scopes.
//! Reference-validated (widths AND element-wise values — widths alone lie).
//!
//! A `burst_t [0:0][1:0] sig;` inside `if (1) begin : g` elaborated at the
//! right total width, but `$bits(g.sig[0][0])` read 1 and every member select
//! returned garbage: the elaborate_items DataDeclaration arm (which generate
//! branches route through) registered widths and generic dims but never the
//! struct layout, the typedef-array shape, or the declared type — all of
//! which the identical module-scope declaration got.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

const SRC: &str = r#"
package P;
  typedef struct packed {
    logic [1:0][63:0] lanes;
    logic [1:0][7:0]  mask;
    logic [1:0]       en;
  } burst_t;   // 146
endpackage
module tb;
  import P::*;
  generate
    if (1) begin : g
      burst_t [0:0][1:0] sig;
      logic [31:0] w_all, w_elem, w_memb;
      assign w_all  = $bits(sig);
      assign w_elem = $bits(sig[0][0]);
      assign w_memb = $bits(sig[0][0].lanes[0]);
      logic [63:0] lane00, lane11;
      logic [7:0]  m10;
      assign lane00 = sig[0][0].lanes[0];
      assign lane11 = sig[0][1].lanes[1];
      assign m10    = sig[0][1].mask[0];
    end
  endgenerate
  // The test harness's get_signal cannot resolve generate-scoped names, so
  // mirror everything into TOP-scope signals (same convention as the existing
  // generate tests).
  logic [31:0] t_w_all, t_w_elem, t_w_memb;
  logic [63:0] t_lane00, t_lane11;
  logic [7:0]  t_m10;
  initial begin
    g.sig[0][0] = {64'hAAAA_AAAA_AAAA_AAA1, 64'hBBBB_BBBB_BBBB_BBB0, 8'hC1, 8'hC0, 2'b10};
    g.sig[0][1] = {64'hDDDD_DDDD_DDDD_DDD1, 64'hEEEE_EEEE_EEEE_EEE0, 8'hF1, 8'hF0, 2'b01};
    #1;
    // Procedural mirrors: hierarchical generate-scope reads resolve on the
    // procedural path (continuous-assign sources from generate scopes are a
    // separate, pre-existing gap).
    t_w_all  = g.w_all;
    t_w_elem = g.w_elem;
    t_w_memb = g.w_memb;
    t_lane00 = g.lane00;
    t_lane11 = g.lane11;
    t_m10    = g.m10;
  end
endmodule
"#;

#[test]
fn generate_if_scope_struct_widths() {
    let sim = simulate(SRC, 50).expect("simulate failed");
    assert_eq!(u(&sim, "t_w_all"), 292);
    assert_eq!(
        u(&sim, "t_w_elem"),
        146,
        "was 1: no element metadata in generate scopes"
    );
    assert_eq!(
        u(&sim, "t_w_memb"),
        64,
        "was 1: no member strides in generate scopes"
    );
}

#[test]
fn generate_if_scope_struct_values_flow() {
    let sim = simulate(SRC, 50).expect("simulate failed");
    assert_eq!(
        u(&sim, "t_lane00"),
        0xBBBB_BBBB_BBBB_BBB0,
        "lanes[0] is the LOW lane"
    );
    assert_eq!(u(&sim, "t_lane11"), 0xDDDD_DDDD_DDDD_DDD1);
    assert_eq!(u(&sim, "t_m10"), 0xF0);
}

/// §27.6 — hierarchical access to FOR-generate block signals. Named blocks'
/// declarations now take their LRM hierarchical name (`gl[1].plain`) — the
/// dotted-flat-key convention named IF-generate blocks always used — instead
/// of an opaque `x__gf_...` rename that nothing outside the block could
/// address: reads returned a 32-bit zero and writes VANISHED, for plain
/// logic as much as struct types. Reference-validated (p0=11 p1=22 d0=22
/// d1=44, and the nested case 10/21/40/51).
#[test]
fn for_generate_hierarchical_signal_access() {
    let src = r#"
module tb;
  generate
    for (genvar i = 0; i < 2; i++) begin : gl
      logic [7:0] plain;
      logic [7:0] doubled;
      assign doubled = plain * 2;
    end
  endgenerate
  logic [7:0] t_d0, t_d1;
  initial begin
    gl[0].plain = 8'h11;
    gl[1].plain = 8'h22;
    #1;
    t_d0 = gl[0].doubled;
    t_d1 = gl[1].doubled;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    let g = |n: &str| sim.get_signal(n).unwrap().to_u64().unwrap();
    assert_eq!(
        g("t_d0"),
        0x22,
        "read through gl[0]. — was 0, writes vanished"
    );
    assert_eq!(g("t_d1"), 0x44);
}

/// Nested for-generate: the inner scope inserts BEFORE the base name
/// (`outer[0].inner[1].q`), matching the LRM path — a naive prefix would
/// have produced `inner[1].outer[0].q`.
#[test]
fn nested_for_generate_hierarchical_access() {
    let src = r#"
module tb;
  generate
    for (genvar i = 0; i < 2; i++) begin : outer
      for (genvar j = 0; j < 2; j++) begin : inner
        logic [7:0] q;
        logic [7:0] twice;
        assign twice = q + 8'(i*16 + j);
      end
    end
  endgenerate
  logic [7:0] t00, t01, t10, t11;
  initial begin
    outer[0].inner[0].q = 8'h10;
    outer[0].inner[1].q = 8'h20;
    outer[1].inner[0].q = 8'h30;
    outer[1].inner[1].q = 8'h40;
    #1;
    t00 = outer[0].inner[0].twice;
    t01 = outer[0].inner[1].twice;
    t10 = outer[1].inner[0].twice;
    t11 = outer[1].inner[1].twice;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    let g = |n: &str| sim.get_signal(n).unwrap().to_u64().unwrap();
    assert_eq!(g("t00"), 0x10);
    assert_eq!(g("t01"), 0x21);
    assert_eq!(g("t10"), 0x40);
    assert_eq!(g("t11"), 0x51);
}

/// Continuous assigns SOURCED from generate-scope signals must track changes.
/// Reference-validated: t_if/t_for follow every write. The if-scope read
/// recorded a dependency on a name that matched no signal (labeled-branch
/// declarations were stored BARE, resolvable only through ad-hoc fallbacks);
/// the for-scope read (`gl[1].v` = MemberAccess over an Index base) fell
/// through to depending on the bare label — both evaluated once and stayed X
/// forever, while the identical procedural reads worked.
#[test]
fn continuous_assign_sources_from_generate_scopes() {
    let src = r#"
module tb;
  generate
    if (1) begin : g
      logic [7:0] x;
    end
    for (genvar i = 0; i < 2; i++) begin : gl
      logic [7:0] v;
    end
  endgenerate
  logic [7:0] t_if, t_for;
  assign t_if  = g.x;
  assign t_for = gl[1].v;
  logic [7:0] r1, r2, r3, r4;
  initial begin
    g.x = 8'h33; gl[1].v = 8'h44;
    // Two delays between write and sample: the property pinned here is that
    // the CA TRACKS the write at all (it used to stay X forever) — not
    // same-timestep settle ordering, which one CI runner scheduled
    // differently than every local run.
    #1; #1; r1 = t_if; r2 = t_for;
    g.x = 8'h77; gl[1].v = 8'h88;   // the CA must RE-fire, not just settle once
    #1; #1; r3 = t_if; r4 = t_for;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    let g = |n: &str| sim.get_signal(n).unwrap().to_u64().unwrap();
    assert_eq!(g("r1"), 0x33, "if-scope CA source, first value");
    assert_eq!(g("r2"), 0x44, "for-scope CA source, first value");
    assert_eq!(g("r3"), 0x77, "if-scope CA source tracks the change");
    assert_eq!(g("r4"), 0x88, "for-scope CA source tracks the change");
}

/// §27.2: a labeled generate block is its own scope — a same-named
/// declaration at module scope is LEGAL, not a duplicate. Bare storage made
/// this a false "duplicate declaration" hard error.
#[test]
fn generate_block_scope_does_not_collide_with_module_scope() {
    let src = r#"
module tb;
  logic [7:0] x = 8'hEE;
  generate
    if (1) begin : g
      logic [7:0] x;
    end
  endgenerate
  logic [7:0] r_top, r_blk;
  initial begin
    g.x = 8'h33;
    #1; r_top = x; r_blk = g.x;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("was a false duplicate-declaration error");
    let g = |n: &str| sim.get_signal(n).unwrap().to_u64().unwrap();
    assert_eq!(g("r_top"), 0xEE, "module-scope x untouched");
    assert_eq!(g("r_blk"), 0x33, "block-scope x is its own signal");
}

/// A top-level process reading its OWN bare signal must never be re-scoped
/// into an instance that happens to declare the same name. Scope inference
/// anchored on the first element of a hash-set iteration and resolution is
/// hint-first, so a testbench checker reading its 292-bit signal could get
/// the DUT-internal 128-bit one instead — and because ahash keys differ
/// across CPUs, the same binary read DIFFERENT signals on different machines
/// (a field log showed exactly this; a CI runner disagreed with every local
/// run on the same commit). Inference now only fires when some bare name
/// does NOT resolve at the process's own level, with total-ordered
/// tie-breaks.
#[test]
fn top_process_is_not_rescoped_into_a_shadowing_instance() {
    let src = r#"
module sink;
  logic [7:0] shared_name;     // same NAME as the TB signal, different width story
  logic [7:0] partner;
  initial begin
    shared_name = 8'hDD;       // instance-internal value
    partner     = 8'hEE;
  end
endmodule
module tb;
  logic [15:0] shared_name;    // the TB's own 16-bit signal
  logic [15:0] partner;
  sink u_s ();
  logic [15:0] got_a, got_b;
  initial begin
    shared_name = 16'h1234;
    partner     = 16'h5678;
    #2;
    // Both names ALSO exist under u_s — a wrongly inferred scope hint
    // would resolve these reads to the instance's 8-bit signals.
    got_a = shared_name;
    got_b = partner;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    let g = |n: &str| sim.get_signal(n).unwrap().to_u64().unwrap();
    assert_eq!(g("got_a"), 0x1234, "TB reads its OWN signal, not u_s's");
    assert_eq!(g("got_b"), 0x5678);
    // And the instance still owns its copies.
    assert_eq!(g("u_s.shared_name"), 0xDD);
}

/// §27.5 and §23.3.2: lookup follows the selected branches and instances,
/// including an override in just one of several instances of a definition.
#[test]
fn struct_member_lint_uses_instance_generate_selection() {
    let dir = std::env::temp_dir().join(format!("xezim_generate_lint_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = r#"
package p;
  typedef struct packed { logic [7:0] a; } s_t;
endpackage
module child #(parameter bit Full = 1, Enable = 0) (output logic [7:0] o);
  import p::*;
  s_t r;
  assign r.a = 8'h5a;
  if (Enable) begin : outer
    if (Full) begin : bad
      assign o = r.missing;
    end else assign o = r.a;
  end else assign o = r.a;
endmodule
module legal;
  wire [7:0] a,b;
  child #(.Full(1), .Enable(0)) one(a);
  child #(.Full(0), .Enable(1)) two(b);
  initial #1 $display("values %h %h", a,b);
endmodule
module bad_member;
  wire [7:0] o;
  child #(.Full(1), .Enable(1)) bad(o);
endmodule
module bad_port;
  child u(.unknown());
endmodule
"#;
    let path = dir.join("check.sv");
    std::fs::write(&path, src).unwrap();
    for (top, diagnostic) in [
        ("legal", None),
        ("bad_member", Some("'missing' is not a member")),
        ("bad_port", Some("port 'unknown'")),
    ] {
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
            .args(["--no-cache", "-s", top])
            .arg(&path)
            .output()
            .unwrap();
        let output = format!(
            "{}{}",
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        );
        if let Some(diagnostic) = diagnostic {
            assert!(
                !run.status.success() && output.contains(diagnostic),
                "{top}: {output}"
            );
        } else {
            assert!(
                run.status.success() && output.contains("values 5a 5a"),
                "{top}: {output}"
            );
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
