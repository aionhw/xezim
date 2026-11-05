//! §13.4 / §9.4.2 — an `always_ff` that calls a function reading or
//! writing module state. A small callee is inlined into the compiled edge
//! block; a large one (the VHDL kernel's next-time scan) stays a call
//! (`CallFreeFunction`). Either way the block must run on every edge, see
//! the module signals the function reads (the event-edge skip cannot see
//! them in a call), and evaluate each actual at its formal's width
//! (§11.6.1: `f(a + b)` keeps the carry of 8-bit operands into a 9-bit
//! formal). Expected lines follow the LRM and the reference simulator.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// A function body of `n` statements, each reading the module signal `b`:
/// it inlines to well over the edge block's limit.
fn big_body(n: usize) -> String {
    (0..n)
        .map(|k| format!("    if (x == 9'd{k}) r = r + 9'd1;\n"))
        .collect()
}

fn design(n: usize) -> String {
    format!(
        r#"
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [7:0] a = 200, b = 100, c = 1, e = 0;
  logic [8:0] lim = 9'd400, q, qc, g;
  int cnt = 0;
  function automatic [8:0] sat(input [8:0] s);
    logic [8:0] r, x;
    r = 0;
    x = s;
{body}    return (s > lim) ? lim : s + r - r;
  endfunction
  function automatic [8:0] follow(input [7:0] s);
    logic [8:0] r, x;
    r = 0;
    x = s;
{body}    return s + e + r - r;
  endfunction
  function automatic void tick();
    logic [8:0] r, x;
    r = 0;
    x = 9'(cnt);
{body}    cnt++;
  endfunction
  always_ff @(posedge clk) q <= sat(a + b);
  always_ff @(posedge clk) g <= follow(c);
  always_ff @(posedge clk) tick();
  assign qc = sat(a + b);
  initial begin
    @(posedge clk);
    #1 $display("q=%0d qc=%0d g=%0d", q, qc, g);
    e = 5;
    repeat (2) @(posedge clk);
    #1 $display("g=%0d", g);
    repeat (20) @(posedge clk);
    #1 $display("cnt=%0d", cnt);
    $finish;
  end
endmodule
"#,
        body = big_body(n)
    )
}

#[test]
fn small_module_state_callee_in_edge_block() {
    assert_eq!(lines(&design(2)), vec!["q=300 qc=300 g=1", "g=6", "cnt=23"]);
}

#[test]
fn large_module_state_callee_in_edge_block() {
    assert_eq!(
        lines(&design(300)),
        vec!["q=300 qc=300 g=1", "g=6", "cnt=23"]
    );
}

/// A small helper calling a large module-state function: the helper is
/// inlined and the inner call stays a call, still seeing `b` change.
#[test]
fn small_helper_around_large_callee_in_edge_block() {
    let src = format!(
        r#"
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [7:0] a = 3, b = 0;
  logic [8:0] q;
  function automatic [8:0] g(input [7:0] s);
    logic [8:0] r, x;
    r = 0;
    x = s;
{body}    return s + b + r - r;
  endfunction
  function automatic [8:0] f(input [7:0] s);
    return g(s) ^ 9'h100;
  endfunction
  always_ff @(posedge clk) q <= f(a);
  initial begin
    @(posedge clk);
    #1 $display("q=%0d", q);
    b = 4;
    repeat (2) @(posedge clk);
    #1 $display("q=%0d", q);
    $finish;
  end
endmodule
"#,
        body = big_body(300)
    );
    assert_eq!(lines(&src), vec!["q=259", "q=263"]);
}

/// §26.3: a large function imported by name from one of two packages that
/// both declare it runs the imported package's body.
#[test]
fn large_imported_package_callee_in_edge_block() {
    let pkg = |name: &str, v: u32| {
        let body: String = (0..300)
            .map(|k| format!("    if (x == {k}) r = r + 1;\n"))
            .collect();
        format!(
            "package {name};\n  int cnt = 0;\n  function automatic int who();\n    int r, x;\n    r = 0;\n    x = cnt;\n{body}    cnt++;\n    return {v} + r - r;\n  endfunction\nendpackage\n"
        )
    };
    let src = format!(
        r#"{pb}{pa}
module top;
  import pb::who;
  logic clk = 0;
  always #5 clk = ~clk;
  int x;
  always_ff @(posedge clk) x <= who();
  initial begin
    @(posedge clk);
    #1 $display("x=%0d", x);
    $finish;
  end
endmodule
"#,
        pb = pkg("pb", 2),
        pa = pkg("pa", 1)
    );
    assert_eq!(lines(&src), vec!["x=2"]);
}

/// The VHDL cycle names (`vh_update`, `vh_sensN`, `vh_runN`, `vh_next`) stay
/// calls in an edge block. The same `vh_run0` body under another name is
/// inlined (209 instructions here); inlining it back into `always @(vh_tick)`
/// is aionhw/xezim-core#155.
#[test]
fn vhdl_kernel_cycle_calls_stay_out_of_the_edge_block() {
    let body: String = (0..40)
        .map(|k| format!("    if (n == 32'd{k}) n = n;\n"))
        .collect();
    let src = format!(
        r#"
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  int n = 0;
  int vh_nxt = 0;
  function automatic void vh_update();
    n = n;
  endfunction
  function automatic bit vh_sens0();
    return 1'b1;
  endfunction
  function automatic void vh_run0();
{body}    n = n + 1;
  endfunction
  function automatic int vh_next();
    return n;
  endfunction
  always @(posedge clk) begin
    vh_update();
    if (vh_sens0()) vh_run0();
    vh_nxt = vh_next();
  end
  initial begin
    repeat (3) @(posedge clk);
    #1 $display("T|n=%0d nxt=%0d", n, vh_nxt);
    $finish;
  end
endmodule
"#
    );
    let dir = std::env::temp_dir().join(format!("xezim_vh_cycle_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, src).expect("write design");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .env("XEZIM_PROFILE_REPORT", "1")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    let shown: Vec<&str> = text.lines().filter(|l| l.starts_with("T|")).collect();
    assert_eq!(shown, ["T|n=3 nxt=3"]);
    let len = text.lines().find_map(|l| {
        let rest = l.strip_prefix("[EDGE_STATS]   block=")?;
        if !rest.contains("sens=clk") {
            return None;
        }
        rest.split_whitespace()
            .find_map(|w| w.strip_prefix("len="))
            .and_then(|n| n.parse::<u64>().ok())
    });
    let len = len.unwrap_or_else(|| panic!("missing clk edge block:\n{text}"));
    assert!(
        len < 32,
        "vh_run0 was inlined into the edge block (len={len})"
    );
}
