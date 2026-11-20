//! §16.5.1 — the operands of a named property or sequence are sampled in the
//! Preponed region, like those of an assertion written inline, and that holds
//! for a hierarchical reference too.
//!
//! A hierarchical reference in a top-level or package `property` /
//! `sequence` declaration kept the parser's member-access shape, which the
//! elaborator folds into a hierarchical name only in process bodies and in
//! an inlined instance's declarations. A plain `u.st` is sampled through its
//! member-access form, but a reference rooted at the top module (`t.u.st`,
//! `p_abs`) was not, and a hierarchical clock (`p_clk`) named no signal, so
//! that property never ticked. A reference into a generate block or an
//! instance-array element (`stg[1].r`) stays a member access over a select
//! even in a process body, so it was not sampled inline either (`a13`), and
//! `$past` of it had no history (`a16`).
//!
//! `u.st` goes 0 -> 1 at the 15 ns edge and back to 0 at 25 ns. Each
//! property's consequent is negated, so a correct run fails each assertion
//! exactly once, at the edge after its antecedent matched. A run that misses
//! the antecedent, or reads the consequent after the edge's update, fails it
//! no times. Counting failures isolates the sampling check from the vacuous
//! pass actions enabled by default (§20.11).
//!
//! Verilator 5.038 reports one failure for every assertion of this design
//! that it can build; it rejects the named sequences (`a4`-`a6`, "Unsupported:
//! sequence") and the package property's edge on an untyped formal (`a8`).
//! Those four already passed and guard against regressions. The other
//! designs below run on Verilator as they are and print the same lines.

use std::path::PathBuf;
use std::process::Command;

const DESIGN: &str = r#"
package pk;
  property p_pk(x, g, c);                       // formals bound to hierarchical actuals
    @(posedge c) (x == 0 && g) |=> x != 1;
  endproperty
endpackage
module core(input logic clk, input logic go);
  logic [1:0] st = 0;
  always @(posedge clk) st <= (st == 0 && go) ? 2'd1 : 2'd0;
  for (genvar k = 0; k < 2; k++) begin : gb
    logic [1:0] x = 0;
    always @(posedge clk) x <= (x == 0 && go) ? 2'd1 : 2'd0;
  end
endmodule
module chk(input logic clk, input logic go);    // a named property below the top
  int sf = 0, gf = 0, pf = 0;
  core u(.clk(clk), .go(go));
  for (genvar j = 0; j < 2; j++) begin : cg
    logic [1:0] y = 0;
    always @(posedge clk) y <= (y == 0 && go) ? 2'd1 : 2'd0;
  end
  property p_sub;
    @(posedge clk) (u.st == 0 && go) |=> u.st != 1;
  endproperty
  property p_cg;                                // into this instance's generate block
    @(posedge clk) (cg[1].y == 0 && go) |=> cg[1].y != 1;
  endproperty
  property p_cgpast;
    @(posedge clk) cg[1].y == 1 |-> !($past(cg[1].y) == 0 && $past(go));
  endproperty
  a_sub: assert property (p_sub) else sf = sf + 1;
  a_cg: assert property (p_cg) else gf = gf + 1;
  a_cgpast: assert property (p_cgpast) else pf = pf + 1;
endmodule
module t;
  logic clk = 0, go = 0;
  int f [17];
  core u(.clk(clk), .go(go));
  core ua [1:0] (.clk(clk), .go(go));
  chk c(.clk(clk), .go(go));
  always #5 clk = ~clk;
  for (genvar g = 0; g < 2; g++) begin : stg
    logic [1:0] r = 0;
    always @(posedge clk) r <= (r == 0 && go) ? 2'd1 : 2'd0;
  end
  property p_start;                             // antecedent on u.st
    @(posedge clk) (u.st == 0 && go) |=> u.st != 1;
  endproperty
  property p_cons;                              // consequent on u.st
    @(posedge clk) go |=> u.st != 1;
  endproperty
  property p_bit;
    @(posedge clk) (u.st[0] == 0 && go) |=> u.st[0] != 1;
  endproperty
  property p_abs;
    @(posedge clk) (t.u.st == 0 && go) |=> t.u.st != 1;
  endproperty
  sequence s_idle;
    u.st == 0 && go;
  endsequence
  sequence s_x(x);
    x == 0 && go;
  endsequence
  property p_seq;                               // a named sequence in a named property
    @(posedge clk) s_idle |=> u.st != 1;
  endproperty
  property p_seqf;                              // a sequence formal bound to u.st
    @(posedge clk) s_x(u.st) |=> u.st != 1;
  endproperty
  property p_f(x);
    @(posedge clk) (x == 0 && go) |=> x != 1;
  endproperty
  property p_past;
    @(posedge clk) u.st == 1 |-> !($past(u.st) == 0 && $past(go));
  endproperty
  property p_rose;
    @(posedge clk) $rose(u.st) && u.st == 1 |-> !$past(go);
  endproperty
  property p_clk;                               // a hierarchical clock
    @(posedge u.clk) (u.st == 0 && go) |=> u.st != 1;
  endproperty
  property p_gen;                               // into a generate block
    @(posedge clk) (stg[1].r == 0 && go) |=> stg[1].r != 1;
  endproperty
  property p_subgen;
    @(posedge clk) (u.gb[1].x == 0 && go) |=> u.gb[1].x != 1;
  endproperty
  property p_arr;                               // into an instance-array element
    @(posedge clk) (ua[1].st == 0 && go) |=> ua[1].st != 1;
  endproperty
  property p_genpast;                           // $past of a generate-block reference
    @(posedge clk) stg[1].r == 1 |-> !($past(stg[1].r) == 0 && $past(go));
  endproperty
  a0: assert property (p_start) else f[0] = f[0] + 1;
  a1: assert property (p_cons) else f[1] = f[1] + 1;
  a2: assert property (p_bit) else f[2] = f[2] + 1;
  a3: assert property (p_abs) else f[3] = f[3] + 1;
  a4: assert property (@(posedge clk) s_idle |=> u.st != 1) else f[4] = f[4] + 1;
  a5: assert property (p_seq) else f[5] = f[5] + 1;
  a6: assert property (p_seqf) else f[6] = f[6] + 1;
  a7: assert property (p_f(u.st)) else f[7] = f[7] + 1;
  a8: assert property (pk::p_pk(u.st, go, clk)) else f[8] = f[8] + 1;
  a9: assert property (p_past) else f[9] = f[9] + 1;
  a10: assert property (p_rose) else f[10] = f[10] + 1;
  a11: assert property (p_clk) else f[11] = f[11] + 1;
  a12: assert property (p_gen) else f[12] = f[12] + 1;
  a13: assert property (@(posedge clk) (stg[1].r == 0 && go) |=> stg[1].r != 1) else f[13] = f[13] + 1;
  a14: assert property (p_subgen) else f[14] = f[14] + 1;
  a15: assert property (p_arr) else f[15] = f[15] + 1;
  a16: assert property (p_genpast) else f[16] = f[16] + 1;
  initial begin
    @(negedge clk) go = 1;
    @(negedge clk) go = 0;
    repeat (3) @(negedge clk);
    for (int i = 0; i < 17; i++)
      $display("a%0d %s fail=%0d", i, f[i] == 1 ? "PASS" : "FAIL", f[i]);
    $display("a_sub %s fail=%0d", c.sf == 1 ? "PASS" : "FAIL", c.sf);
    $display("a_cg %s fail=%0d", c.gf == 1 ? "PASS" : "FAIL", c.gf);
    $display("a_cgpast %s fail=%0d", c.pf == 1 ? "PASS" : "FAIL", c.pf);
    $finish;
  end
endmodule
"#;

fn run(name: &str, src: &str) -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("sva_named_property_hier_sampling");
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join(format!("{name}.sv"));
    std::fs::write(&sv, src).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "t", "--no-cache", sv.to_str().unwrap()])
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "run failed:\n{text}");
    text
}

/// §3.12.1: `$unit::g` in a named property still names the compilation-unit
/// variable where the module declares its own `g`. Folding the declaration
/// (`u.v` makes it fold) must keep the qualifier there, as the elaborator
/// does in process bodies.
#[test]
fn unit_qualified_names_in_named_properties_keep_their_scope() {
    let text = run(
        "unit",
        r#"
logic g = 0;
module m;
  logic v = 0;
endmodule
module t;
  logic clk = 0;
  logic g = 1;
  int n = 0, f = 0;
  m u();
  always #5 clk = ~clk;
  property p_unit;
    @(posedge clk) $unit::g == 0 && g == 1 && u.v == 0;
  endproperty
  a: assert property (p_unit) n = n + 1; else f = f + 1;
  initial begin
    #42;
    $display("unit pass=%0d fail=%0d", n, f);
    $finish;
  end
endmodule
"#,
    );
    assert!(text.contains("unit pass=4 fail=0"), "{text}");
}

#[test]
fn hierarchical_references_in_named_properties_are_sampled() {
    let text = run("hier", DESIGN);
    let names = (0..17)
        .map(|i| format!("a{i}"))
        .chain(["a_sub", "a_cg", "a_cgpast"].map(String::from));
    for name in names {
        assert!(
            text.contains(&format!("{name} PASS fail=1")),
            "{name}: expected exactly one failure:\n{text}"
        );
    }
}

/// §16.5.1: a member of an unpacked struct array element picked by a
/// variable (`a[i].f`) is read with `i`'s sampled value at every tick. Only
/// a select fixed at elaboration (a generate block's or an instance array's)
/// is folded into a name; folding this one froze the element `i` named when
/// the assertion registered, so every other tick checked the wrong element.
#[test]
fn variable_selects_in_assertions_follow_the_index() {
    let text = run(
        "index",
        r#"
module t;
  typedef struct { logic [1:0] f; } e_t;
  logic clk = 0;
  e_t a [2];
  int i = 0, n = 0, f = 0, m = 0, k = 0;
  initial begin a[0].f = 1; a[1].f = 2; end
  always #5 clk = ~clk;
  always @(posedge clk) i <= 1 - i;
  property p_idx;
    @(posedge clk) a[i].f == i + 1;
  endproperty
  a_named: assert property (p_idx) n = n + 1; else f = f + 1;
  a_inline: assert property (@(posedge clk) a[i].f == i + 1) m = m + 1; else k = k + 1;
  initial begin
    #60;
    $display("named pass=%0d fail=%0d inline pass=%0d fail=%0d", n, f, m, k);
    $finish;
  end
endmodule
"#,
    );
    assert!(
        text.contains("named pass=6 fail=0 inline pass=6 fail=0"),
        "{text}"
    );
}

/// A generate-block net named through the block index (`g[0].addr`) is
/// sampled like any other operand (§16.5.1), inline, in a named property,
/// and through a bound checker's port connected to it. It read the value
/// the edge's updates had just given it, so each check failed at every edge
/// where `pr` changed.
#[test]
fn generate_block_signals_named_through_the_index_are_sampled() {
    let text = run(
        "gen_index",
        r#"
module t;
  logic clk = 0, pr = 0;
  int fails = 0, n = 0;
  always #5 clk = ~clk;
  always @(posedge clk) pr <= ~pr;
  for (genvar j = 0; j < 2; j++) begin : g
    logic [1:0] addr;
    assign addr = pr * 2 + j;
  end
  a_gen: assert property (@(posedge clk) g[0].addr == 2'(pr * 2)) else fails = fails + 1;
  always @(posedge clk) n = n + 1;
  initial begin
    #50;
    $display("RESULT n=%0d fails=%0d", n, fails);
    $finish;
  end
endmodule
"#,
    );
    assert!(text.contains("RESULT n=5 fails=0"), "{text}");
    let text = run(
        "gen_bound",
        r#"
module chk(input logic clk, input logic pr, input logic [1:0] a0);
  int fails = 0, n = 0;
  property p_a0;
    @(posedge clk) a0 == 2'(pr * 2);
  endproperty
  a_bound: assert property (p_a0) n = n + 1; else fails = fails + 1;
endmodule
module dut(input logic clk);
  logic pr = 0;
  always @(posedge clk) pr <= ~pr;
  for (genvar j = 0; j < 2; j++) begin : g
    logic [1:0] addr;
    assign addr = pr * 2 + j;
  end
endmodule
module t;
  logic clk = 0;
  int fn = 0, nn = 0, fi = 0, ni = 0;
  always #5 clk = ~clk;
  dut d(.clk(clk));
  bind dut chk u_chk(.clk(clk), .pr(pr), .a0(g[0].addr));
  property p_gen;
    @(posedge clk) d.g[0].addr == 2'(d.pr * 2);
  endproperty
  a_named: assert property (p_gen) nn = nn + 1; else fn = fn + 1;
  a_inline: assert property (@(posedge clk) d.g[0].addr == 2'(d.pr * 2)) ni = ni + 1; else fi = fi + 1;
  initial begin
    #50;
    $display("named pass=%0d fail=%0d inline pass=%0d fail=%0d bound pass=%0d fail=%0d",
             nn, fn, ni, fi, d.u_chk.n, d.u_chk.fails);
    $finish;
  end
endmodule
"#,
    );
    assert!(
        text.contains("named pass=5 fail=0 inline pass=5 fail=0 bound pass=5 fail=0"),
        "{text}"
    );
}
