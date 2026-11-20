//! IEEE 1800-2017 §16 conformance-audit fixes: default failure reports
//! (§16.3), immediate and deferred assertions in `always_comb` sensitivity
//! (§9.2.2.2.1), assertion local variables and match items (§16.10, §16.11),
//! procedural concurrent assertions (§16.14.6) and `default disable iff`
//! (§16.15). Every expected line is the reference simulator's; lines of one
//! time step are compared sorted, as the order in which different
//! assertions report within a time step is not specified.
use std::path::PathBuf;
use std::process::Command;

fn run(name: &str, top: &str, src: &str) -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("lrm_audit_sva");
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join(format!("{name}.sv"));
    std::fs::write(&sv, src).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", top, "--no-cache", sv.to_str().unwrap()])
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "run failed:\n{text}");
    text
}

/// The `T|` lines with the given prefix, sorted.
fn lines(text: &str, prefix: &str) -> Vec<String> {
    let mut v: Vec<String> = text
        .lines()
        .filter(|l| l.starts_with(prefix))
        .map(|l| l.trim().to_string())
        .collect();
    v.sort();
    v
}

/// `(time, scope)` of every default `Assertion error.` report, sorted.
fn default_errors(text: &str) -> Vec<(u64, String)> {
    let all: Vec<&str> = text.lines().collect();
    let mut v = Vec::new();
    for (i, l) in all.iter().enumerate() {
        if l.trim() != "** Error: Assertion error." {
            continue;
        }
        let ctx = all.get(i + 1).copied().unwrap_or_default();
        let time = ctx
            .split("Time:")
            .nth(1)
            .and_then(|t| t.split_whitespace().next())
            .and_then(|t| t.parse().ok())
            .unwrap_or(u64::MAX);
        let scope = ctx
            .split("Scope:")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or_default()
            .to_string();
        v.push((time, scope));
    }
    v.sort();
    v
}

fn expect(text: &str, prefix: &str, want: &[&str]) {
    let mut want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
    want.sort();
    assert_eq!(lines(text, prefix), want, "full output:\n{text}");
}

/// §16.3: a failing assertion with no `else` clause calls `$error`: immediate,
/// deferred (`#0`, `final`) and concurrent, `assume` too, never `cover`. The
/// audit repro, then every kind in one module; the time and scope of each
/// report are the reference simulator's.
#[test]
fn no_else_reports_default_error() {
    let text = run(
        "no_else_repro",
        "t16_3",
        r#"
// top: t16_3
module t16_3;
  bit clk; int x = 1;
  always #5 clk = ~clk;
  ap: assert property (@(posedge clk) x == 2);
  initial begin
    a1: assert (x == 2);
    assert (x == 3);
    #12 $display("T|a|done");
    $finish;
  end
endmodule
"#,
    );
    assert_eq!(
        default_errors(&text),
        vec![
            (0, "t16_3".to_string()),
            (0, "t16_3.a1".to_string()),
            (5, "t16_3.ap".to_string()),
        ],
        "{text}"
    );
    expect(&text, "T|", &["T|a|done"]);
    let text = run(
        "no_else_kinds",
        "t",
        r#"
module t;
  bit clk; int x = 1;
  always #5 clk = ~clk;
  ap: assert property (@(posedge clk) x == 2) $display("T|ap pass");
  assume property (@(posedge clk) x == 3);
  um: assume property (@(posedge clk) x == 4);
  cv: cover property (@(posedge clk) x == 5);
  initial begin
    a1: assert (x == 2) $display("T|a1 pass");
    assume (x == 3);
    a3: assert #0 (x == 4);
    a4: assert final (x == 5);
    a5: assume #0 (x == 6);
    cover (x == 7);
    begin : blk
      assert (x == 8);
    end
    #12 $display("T|a|done");
    $finish;
  end
endmodule
"#,
    );
    let want: Vec<(u64, String)> = [
        (0, "t"),
        (0, "t.a1"),
        (0, "t.a3"),
        (0, "t.a4"),
        (0, "t.a5"),
        (0, "t.blk"),
        (5, "t"),
        (5, "t.ap"),
        (5, "t.um"),
    ]
    .iter()
    .map(|(t, s)| (*t, s.to_string()))
    .collect();
    assert_eq!(default_errors(&text), want, "{text}");
}

/// §9.2.2.2.1 / §16.3: an `always_comb` or `always @*` whose only reads are
/// in an assertion re-runs when they change (it ran once at t=0 and never
/// again). The audit repro with its stimulus written by nonblocking
/// assignments, so both inputs change at once.
#[test]
fn always_comb_assertion_reads_are_sensitivity() {
    let text = run(
        "comb_assert_rerun",
        "t16_3b",
        r#"
// top: t16_3b
module t16_3b;
  int a, b, c, d;
  always_comb begin
    ai: assert (a >= b) $display("T|i|pass a=%0d b=%0d t=%0t", a, b, $time); else $display("T|i|FAIL a=%0d b=%0d t=%0t", a, b, $time);
  end
  always_comb begin
    c = a;
    af: assert final (a >= b) $display("T|f|pass a=%0d b=%0d t=%0t", a, b, $time); else $display("T|f|FAIL a=%0d b=%0d t=%0t", a, b, $time);
  end
  always @* begin
    as: assert (a + 1 > b) else $display("T|s|FAIL a=%0d b=%0d t=%0t", a, b, $time);
  end
  initial begin
    #1 a <= 2; b <= 1;
    #1 a <= 3; b <= 3;
    #1 a = 1; #0 b = 1;
    #1 a <= 0; b <= 5;
    #1 $finish;
  end
endmodule
"#,
    );
    expect(
        &text,
        "T|",
        &[
            "T|f|FAIL a=0 b=5 t=4",
            "T|f|pass a=0 b=0 t=0",
            "T|f|pass a=1 b=1 t=3",
            "T|f|pass a=2 b=1 t=1",
            "T|f|pass a=3 b=3 t=2",
            "T|i|FAIL a=0 b=5 t=4",
            "T|i|FAIL a=1 b=3 t=3",
            "T|i|pass a=0 b=0 t=0",
            "T|i|pass a=1 b=1 t=3",
            "T|i|pass a=2 b=1 t=1",
            "T|i|pass a=3 b=3 t=2",
            "T|s|FAIL a=0 b=5 t=4",
            "T|s|FAIL a=1 b=3 t=3",
        ],
    );
}

/// §16.4: a deferred assertion in an `always_comb` the settle evaluates at
/// time 0 reports; its report was dropped as a flush of whichever process
/// happened to be current.
#[test]
fn deferred_assertion_in_comb_reports_at_time_zero() {
    let text = run(
        "comb_deferred_t0",
        "t",
        r#"
module t;
  int a, b, c, d, e;
  always_comb begin
    af: assert final (a >= b) $display("T|f pass a=%0d t=%0t", a, $time); else $display("T|f FAIL");
  end
  always_comb begin
    c = a;
    ag: assert final (a >= b) $display("T|g pass a=%0d t=%0t", a, $time);
  end
  always_comb begin
    d = a;
    ah: assert #0 (a >= b) $display("T|h pass a=%0d t=%0t", a, $time);
  end
  always_comb begin
    e = b;
    $display("T|j a=%0d t=%0t", a, $time);
  end
  initial begin
    #1 a = 2; b = 1;
    #1 $finish;
  end
endmodule
"#,
    );
    for p in ["T|f", "T|g", "T|h"] {
        let want: Vec<String> = ["pass a=0 t=0", "pass a=2 t=1"]
            .iter()
            .map(|s| format!("{p} {s}"))
            .collect();
        assert_eq!(lines(&text, p), want, "{text}");
    }
}

/// §16.10: property and sequence local variables, one copy per attempt: the
/// audit repro, then a pipeline of overlapping attempts, accumulation over
/// two steps, an initialiser with `++`/`+=`, a named sequence with a local
/// used twice, a `local input` formal and a match-item call.
#[test]
fn assertion_local_variables() {
    let text = run(
        "locals_repro",
        "t16_10",
        r#"
// top: t16_10
module t16_10;
  initial $assertvacuousoff(0); // Trace only nonvacuous passes.
  bit clk, start, done; int din, dout;
  always #5 clk = ~clk;
  initial begin
    @(negedge clk) start = 1; din = 5; @(negedge clk) start = 0;
    @(negedge clk) done = 1; dout = 6; @(negedge clk) done = 0;
    @(negedge clk) start = 1; din = 9; @(negedge clk) start = 0;
    @(negedge clk) done = 1; dout = 3; @(negedge clk) done = 0;
    repeat (3) @(negedge clk); $finish;
  end
  property p_lv; int x; @(posedge clk) (start, x = din) |-> ##[1:3] (done && dout == x + 1); endproperty
  a_lv: assert property (p_lv) $display("T|lv|P t=%0t", $time); else $display("T|lv|F t=%0t", $time);
endmodule
"#,
    );
    expect(&text, "T|", &["T|lv|F t=85", "T|lv|P t=35"]);
    let text = run(
        "locals_shapes",
        "t",
        r#"
module t;
  initial $assertvacuousoff(0); // Trace only nonvacuous passes.
  bit clk, a, b, c; int d, e;
  always #5 clk = ~clk;
  // pipeline: every cycle a new attempt captures d; e must equal it 2 cycles later
  property p_pipe; int x; @(posedge clk) (a, x = d) |-> ##2 e == x; endproperty
  ap_pipe: assert property (p_pipe) else $display("T|pipe F t=%0t", $time);
  // accumulate across steps
  property p_acc; int x; @(posedge clk) (a, x = d) ##1 (b, x = x + d) |-> e == x; endproperty
  ap_acc: assert property (p_acc) $display("T|acc P t=%0t", $time); else $display("T|acc F t=%0t", $time);
  // initialiser + increment
  property p_cnt; int n = 5; @(posedge clk) (a, n++) ##1 (b, n += 2) |-> e == n; endproperty
  ap_cnt: assert property (p_cnt) $display("T|cnt P t=%0t", $time); else $display("T|cnt F t=%0t", $time);
  // named sequence with a local, used twice in one property
  sequence s_cap; int v; (c, v = d) ##1 (e == v); endsequence
  ap_two: assert property (@(posedge clk) s_cap ##1 s_cap) $display("T|two P t=%0t", $time);
  // local formal argument
  sequence s_lf(local input int v); (1, v = v + 1) ##1 (e == v); endsequence
  ap_lf: assert property (@(posedge clk) a |-> s_lf(d)) $display("T|lf P t=%0t", $time); else $display("T|lf F t=%0t", $time);
  // cover with local and a subroutine call on match
  function void show(int v); $display("T|show v=%0d t=%0t", v, $time); endfunction

  cv2: cover property (@(posedge clk) int_seq);
  sequence int_seq; int q; (a, q = d, show(q)) ##1 (e == q); endsequence
  initial begin
    @(negedge clk) a = 1; d = 3; b = 0; e = 0;
    @(negedge clk) a = 1; d = 4; b = 1; e = 7;
    @(negedge clk) a = 0; d = 9; b = 0; e = 3;
    @(negedge clk) a = 1; d = 2; b = 1; c = 1; e = 4;
    @(negedge clk) a = 0; d = 6; b = 0; c = 1; e = 2;
    @(negedge clk) a = 0; d = 1; c = 0; e = 6;
    @(negedge clk) e = 3;
    @(negedge clk) e = 0;
    repeat (2) @(negedge clk); $finish;
  end
endmodule
"#,
    );
    expect(
        &text,
        "T|",
        &[
            "T|acc P t=25",
            "T|cnt F t=25",
            "T|lf F t=25",
            "T|lf F t=35",
            "T|lf F t=55",
            "T|pipe F t=65",
            "T|show v=2 t=45",
            "T|show v=3 t=15",
            "T|show v=4 t=25",
        ],
    );
}

/// §16.11: a subroutine call in a sequence match item runs at each match.
#[test]
fn match_item_subroutine_call() {
    let text = run(
        "match_call",
        "t16_11",
        r#"
// top: t16_11
module t16_11;
  bit clk, a, b;
  always #5 clk = ~clk;
  initial begin @(negedge clk) a = 1; @(negedge clk) a = 0; b = 1; @(negedge clk) b = 0; repeat (2) @(negedge clk); $finish; end
  function void note(string s); $display("T|m|match %s t=%0t", s, $time); endfunction
  c_sub: cover property (@(posedge clk) (a, note("a")) ##1 (b, note("b")));
endmodule
"#,
    );
    expect(&text, "T|", &["T|m|match a t=15", "T|m|match b t=25"]);
}

/// §16.14.6: a concurrent assertion in an `always` procedure infers its clock
/// from the event control and starts an attempt only when the procedure
/// reaches it: the audit repro, then an asynchronous-reset `always_ff`,
/// `case` arms, a loop whose variable each attempt captures, an explicit
/// clock and a condition that never holds.
#[test]
fn procedural_concurrent_assertions() {
    let text = run(
        "procedural_repro",
        "t16_14",
        r#"
// top: t16_14
module t16_14;
  bit clk, start, done;
  always #5 clk = ~clk;
  initial begin
    @(negedge clk) start = 1;
    @(negedge clk) start = 0;
    @(negedge clk) done = 1;
    @(negedge clk) done = 0;
    repeat (4) @(negedge clk); $finish;
  end
  always @(posedge clk) begin
    if (start) a_inf: assert property (##2 done) $display("T|inf|P t=%0t", $time); else $display("T|inf|F t=%0t", $time);
  end
endmodule
"#,
    );
    expect(&text, "T|", &["T|inf|P t=35"]);
    let text = run(
        "procedural_shapes",
        "t",
        r#"
module t;
  bit clk, rst_n, en, a, b; bit [1:0] sel; bit [3:0] req, gnt;
  always #5 clk = ~clk;
  // async reset: clock inferred is clk (rst_n is read in the body)
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) ;
    else if (en) a_ff: assert property (a |=> b) $display("T|ff P t=%0t", $time); else $display("T|ff F t=%0t", $time);
  end
  // case gating
  always @(posedge clk) begin
    case (sel)
      2'd1: a_c1: assert property (a) $display("T|c1 P t=%0t", $time); else $display("T|c1 F t=%0t", $time);
      2'd2: a_c2: assert property (##1 b) $display("T|c2 P t=%0t", $time); else $display("T|c2 F t=%0t", $time);
      default: ;
    endcase
  end
  // loop: the loop variable is captured per instance
  always @(posedge clk) begin
    for (int i = 0; i < 4; i++)
      if (req[i]) a_lp: assert property (##1 gnt[i]) $display("T|lp P t=%0t", $time); else $display("T|lp F t=%0t", $time);
  end
  // explicit clock in a procedure
  always @(posedge clk) if (en) a_ex: assert property (@(posedge clk) a ##1 b) $display("T|ex P t=%0t", $time); else $display("T|ex F t=%0t", $time);
  // condition false: never attempted, no failures
  always @(posedge clk) if (sel == 3) a_nv: assert property (a && b) else $display("T|nv F t=%0t", $time);
  initial begin
    rst_n = 0;
    @(negedge clk) rst_n = 1; en = 1; a = 1; b = 0; sel = 1; req = 4'b0101;
    @(negedge clk) en = 0; a = 0; b = 1; sel = 2; req = 0; gnt = 4'b0001;
    @(negedge clk) en = 1; a = 1; b = 0; sel = 0; req = 4'b1000; gnt = 0;
    @(negedge clk) en = 0; a = 0; b = 1; sel = 2; req = 0; gnt = 4'b1000;
    @(negedge clk) b = 0; sel = 0; gnt = 0;
    repeat (2) @(negedge clk); $finish;
  end
endmodule
"#,
    );
    expect(
        &text,
        "T|",
        &[
            "T|c1 P t=15",
            "T|c2 F t=35",
            "T|c2 F t=55",
            "T|ex P t=25",
            "T|ex P t=45",
            "T|ff P t=25",
            "T|ff P t=45",
            "T|lp F t=25",
            "T|lp P t=25",
            "T|lp P t=45",
        ],
    );
}

/// §16.15: `default disable iff` disables the concurrent assertions of its
/// scope without their own `disable iff`, wherever it is declared in the
/// scope, including a generate block's (which may declare its own), but not
/// those of a child instance. The audit repro, then the scoping shapes.
#[test]
fn default_disable_iff() {
    let text = run(
        "default_disable_repro",
        "t16_15",
        r#"
// top: t16_15
module t16_15;
  initial $assertvacuousoff(0); // Trace only nonvacuous passes.
  bit clk, start, done, rst;
  always #5 clk = ~clk;
  initial begin
    @(negedge clk) start = 1; rst = 1;
    @(negedge clk) start = 0; rst = 0;
    @(negedge clk) done = 1;
    @(negedge clk) done = 0;
    repeat (2) @(negedge clk); $finish;
  end
  default clocking dcb @(posedge clk); endclocking
  default disable iff rst;
  a_dis: assert property (start |-> ##2 done) $display("T|dflt|P t=%0t (should be disabled)", $time); else $display("T|dflt|F t=%0t", $time);
  a_exp: assert property (disable iff (rst) start |-> ##2 done) $display("T|expl|P t=%0t (should be disabled)", $time); else $display("T|expl|F t=%0t", $time);
endmodule
"#,
    );
    expect(&text, "T|", &[]);
    let text = run(
        "default_disable_scopes",
        "t",
        r#"
module sub(input bit clk, input bit a, input bit rst);
  // the parent's default disable must not reach a child instance
  a_sub: assert property (@(posedge clk) a) else $display("T|sub F t=%0t", $time);
endmodule
module t;
  bit clk, rst, rst2, a;
  always #5 clk = ~clk;
  a_early: assert property (@(posedge clk) a) else $display("T|early F t=%0t", $time);
  default disable iff (rst);
  a_late: assert property (@(posedge clk) a) else $display("T|late F t=%0t", $time);
  a_expl: assert property (@(posedge clk) disable iff (rst2) a) else $display("T|expl F t=%0t", $time);
  c_cov: cover property (@(posedge clk) !a) $display("T|cov hit t=%0t", $time);
  generate if (1) begin : g
    a_gen: assert property (@(posedge clk) a) else $display("T|gen F t=%0t", $time);
  end endgenerate
  generate if (1) begin : h
    default disable iff (rst2);
    a_h: assert property (@(posedge clk) a) else $display("T|h F t=%0t", $time);
  end endgenerate
  sub u(.clk(clk), .a(a), .rst(rst));
  initial begin
    rst = 1; rst2 = 0; a = 0;
    @(negedge clk) rst = 0; rst2 = 1;
    @(negedge clk) rst = 1; rst2 = 1;
    @(negedge clk) rst = 0; rst2 = 0; a = 1;
    repeat (2) @(negedge clk); $finish;
  end
endmodule
"#,
    );
    expect(
        &text,
        "T|",
        &[
            "T|cov hit t=15",
            "T|early F t=15",
            "T|expl F t=5",
            "T|gen F t=15",
            "T|h F t=5",
            "T|late F t=15",
            "T|sub F t=15",
            "T|sub F t=25",
            "T|sub F t=5",
        ],
    );
}

/// §16.14 / §27: every copy of a generate block's assertion item is its own
/// assertion, and `%m` in its action names the block (only one loop copy
/// ran, and `%m` dropped the block).
#[test]
fn generate_block_assertion_items() {
    let text = run(
        "generate_assertions",
        "t",
        r#"
module t;
  bit clk, a;
  always #5 clk = ~clk;
  generate if (1) begin : g
    default disable iff (a);
    a_gen: assert property (@(posedge clk) a) else $display("T|gen %m t=%0t", $time);
    always @(posedge clk) if (!a) a_p: assert property (a) else $display("T|p %m t=%0t", $time);
  end endgenerate
  for (genvar i = 0; i < 2; i++) begin : lp
    a_l: assert property (@(posedge clk) a) else $display("T|l %m t=%0t", $time);
  end
  initial begin #12 $finish; end
endmodule
"#,
    );
    expect(
        &text,
        "T|",
        &[
            "T|gen t.g.a_gen t=5",
            "T|l t.lp[0].a_l t=5",
            "T|l t.lp[1].a_l t=5",
            "T|p t.g.a_p t=5",
        ],
    );
}

/// §16.9 Table 16-3: `##` binds looser than every expression operator, so
/// `a ##1 b == c` delays the comparison and `a && b ##1 c || d` delays the
/// disjunction (both bound the operand tighter than the `==` / `||`).
#[test]
fn cycle_delay_precedence() {
    let text = run(
        "cycle_delay_precedence",
        "t",
        r#"
module t;
  bit clk, a, b, c, d; int x, y;
  always #5 clk = ~clk;
  a1: assert property (@(posedge clk) a |-> ##1 b == c) else $display("T|a1 F t=%0t", $time);
  a2: assert property (@(posedge clk) a ##1 b == c |-> d) else $display("T|a2 F t=%0t", $time);
  a3: assert property (@(posedge clk) a && b ##1 c || d) else $display("T|a3 F t=%0t", $time);
  a4: cover property (@(posedge clk) a ##1 b and c ##1 d) $display("T|a4 C t=%0t", $time);
  property p5; int v; @(posedge clk) (a, v = x) ##1 y == v + 1; endproperty
  a5: cover property (p5) $display("T|a5 C t=%0t", $time);
  a6: assert property (@(posedge clk) a ##1 x + 1 > y) else $display("T|a6 F t=%0t", $time);
  initial begin
    @(negedge clk) a = 1; b = 0; c = 0; d = 0; x = 4; y = 0;
    @(negedge clk) a = 0; b = 1; c = 0; d = 1; x = 1; y = 5;
    @(negedge clk) a = 1; b = 1; c = 1; d = 0; x = 7; y = 9;
    @(negedge clk) a = 1; b = 0; c = 1; d = 1; x = 2; y = 8;
    @(negedge clk) a = 0; b = 0; c = 0; d = 1; y = 3;
    repeat (2) @(negedge clk); $finish;
  end
endmodule
"#,
    );
    expect(
        &text,
        "T|",
        &[
            "T|a1 F t=25",
            "T|a1 F t=45",
            "T|a3 F t=15",
            "T|a3 F t=25",
            "T|a3 F t=45",
            "T|a3 F t=5",
            "T|a3 F t=55",
            "T|a3 F t=65",
            "T|a5 C t=25",
            "T|a5 C t=45",
            "T|a5 C t=55",
            "T|a6 F t=25",
            "T|a6 F t=25",
            "T|a6 F t=45",
            "T|a6 F t=5",
            "T|a6 F t=55",
            "T|a6 F t=55",
            "T|a6 F t=65",
        ],
    );
}

/// §16.14.6: a procedural concurrent assertion in an `initial` procedure, or
/// after a timing control in an `always`, starts one attempt per execution,
/// at the next tick of its clock (it ran at every tick).
#[test]
fn procedural_assertions_in_initial_and_timed_always() {
    let text = run(
        "procedural_assertions_in_initial_and_timed_always",
        "t",
        r#"
module t;
  bit clk, a, b;
  always #5 clk = ~clk;
  initial a_i: assert property (@(posedge clk) a |=> b) $display("T|i P t=%0t", $time); else $display("T|i F t=%0t", $time);
  initial begin
    repeat (2) @(posedge clk);
    a_j: assert property (@(posedge clk) a) $display("T|j P t=%0t", $time); else $display("T|j F t=%0t", $time);
  end
  always begin
    @(posedge clk);
    a_k: assert property (@(posedge clk) b) $display("T|k P t=%0t", $time); else $display("T|k F t=%0t", $time);
  end
  initial begin
    a = 1; b = 0;
    @(negedge clk) b = 1;
    @(negedge clk) a = 0;
    @(negedge clk) b = 0;
    repeat (2) @(negedge clk); $finish;
  end
endmodule
"#,
    );
    expect(
        &text,
        "T|",
        &[
            "T|i P t=15",
            "T|j P t=15",
            "T|k F t=35",
            "T|k F t=45",
            "T|k F t=5",
            "T|k P t=15",
            "T|k P t=25",
        ],
    );
}

/// §16.6 / §14.12: a named property or sequence written without a clocking
/// event (with its own `disable iff`, or local variables) is clocked by the
/// default clocking; its body used to be dropped.
#[test]
fn unclocked_named_properties_use_default_clocking() {
    let text = run(
        "unclocked_named_properties_use_default_clocking",
        "t",
        r#"
module t;
  initial $assertvacuousoff(0); // Trace only nonvacuous passes.
  bit clk, a, b, rst;
  always #5 clk = ~clk;
  default clocking cb @(posedge clk); endclocking
  property p_u; a |=> b; endproperty
  property p_d; disable iff (rst) a |-> b; endproperty
  property p_l; int n; (a, n = 1) ##1 (b, n++) |-> n == 2; endproperty
  sequence s_u; a ##1 b; endsequence
  a_u: assert property (p_u) $display("T|u P t=%0t", $time); else $display("T|u F t=%0t", $time);
  a_d: assert property (p_d) $display("T|d P t=%0t", $time); else $display("T|d F t=%0t", $time);
  a_l: assert property (p_l) $display("T|l P t=%0t", $time); else $display("T|l F t=%0t", $time);
  c_s: cover property (s_u) $display("T|s C t=%0t", $time);
  initial begin
    rst = 1; a = 1; b = 0;
    @(negedge clk) rst = 0; b = 1;
    @(negedge clk) a = 0;
    @(negedge clk) a = 1; b = 0;
    @(negedge clk) a = 0; b = 1;
    repeat (2) @(negedge clk); $finish;
  end
endmodule
"#,
    );
    expect(
        &text,
        "T|",
        &[
            "T|d F t=35",
            "T|d P t=15",
            "T|l P t=15",
            "T|l P t=25",
            "T|l P t=45",
            "T|s C t=15",
            "T|s C t=25",
            "T|s C t=45",
            "T|u P t=15",
            "T|u P t=25",
            "T|u P t=45",
        ],
    );
}
