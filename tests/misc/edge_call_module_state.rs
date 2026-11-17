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

/// VHDL lowering calls module-scope helpers as `__free_fn__` bodies that read
/// module queues (`vh_dt*.pop_front()`). Same shape as `XEZIM_VHDL_BUGS.md` §1.
#[test]
fn free_fn_compiles_module_queue_pop_front() {
    let src = r#"
module g;
  logic tick = 0;
  int qd[$];
  int qq[$];
  int qn;
  int dv;
  longint now;
  function automatic void upd();
    if (qn > 0 && qd[0] <= now) begin void'(qd.pop_front()); dv = qq.pop_front(); qn--; end
  endfunction
  initial begin
    now = 10; qn = 2; qd.push_back(1); qd.push_back(2); qq.push_back(7); qq.push_back(9);
    for (int j = 0; j < 3; j++) begin tick = 1; #1; tick = 0; #1; end
    $display("dv=%0d qn=%0d", dv, qn);
    $finish;
  end
  always @(tick) upd();
endmodule
"#;
    assert_eq!(lines(src), vec!["dv=9 qn=0"]);
}

/// Module-scope function reading a module signal (VHDL `vh_upd` pattern).
#[test]
fn free_fn_reads_resolved_module_signal() {
    let src = r#"
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [7:0] a = 1, outv = 0;
  function automatic void bump();
    outv = a + 8'd1;
  endfunction
  always_ff @(posedge clk) bump();
  initial begin
    @(posedge clk);
    #1 $display("outv=%0d", outv);
  end
endmodule
"#;
    assert_eq!(lines(src), vec!["outv=2"]);
}

fn forced_compiled_lines(src: &str) -> (Vec<String>, String) {
    use std::process::Command;
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_helper_scope_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("top.sv");
    std::fs::write(&path, src).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--sv2017", "--no-cache", "-s", "top"])
        .arg(&path)
        .env("XEZIM_COMPILE_METHODS", "1")
        .env("XEZIM_METHOD_TIER", "0")
        .env("XEZIM_FALLBACK_SITES", "1")
        .env("XEZIM_JIT", "0")
        .env_remove("XEZIM_METHOD_CACHE")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{stdout}\n{stderr}");
    let rows = stdout
        .lines()
        .filter(|line| line.starts_with("T|"))
        .map(str::to_owned)
        .collect();
    std::fs::remove_dir_all(dir).unwrap();
    (rows, stderr)
}

fn large_reader(name: &str, tail: &str) -> String {
    format!(
        "function automatic int {name}(input int x); logic [8:0] r; r=0;\n{}\n{tail}\nendfunction",
        big_body(300)
    )
}

/// §13.4: exercise the delegated compiled body, rather than the small
/// edge-inlined body used by the ordinary module-signal check above.
#[test]
fn delegated_module_signal_reader_runs_compiled() {
    let source = format!(
        "module top; int q=7; {} initial begin $display(\"T|value=%0d q=%0d\",get(0),q); $finish; end endmodule",
        large_reader("get", "return q+r-r;")
    );
    let (rows, trace) = forced_compiled_lines(&source);
    assert_eq!(rows, ["T|value=7 q=7"]);
    assert!(
        trace.contains("runs-compiled scope=__free_fn__.get"),
        "{trace}"
    );
}

/// §26.3: a package variable keeps its package binding when a module has
/// the same name, including a package callee inlined into a module helper.
#[test]
fn package_signal_names_do_not_bind_to_module_signals() {
    let direct = format!(
        "package p; int q=7; {} endpackage module top; int q=99; initial begin $display(\"T|value=%0d q=%0d\",p::get(0),q); $finish; end endmodule",
        large_reader("get", "return q+r-r;")
    );
    assert_eq!(forced_compiled_lines(&direct).0, ["T|value=7 q=99"]);
    let nested = format!(
        "package p; int q=7; function automatic int get(input int x); return q+x; endfunction endpackage module top; int q=99; {} initial begin $display(\"T|value=%0d q=%0d\",wrap(0),q); $finish; end endmodule",
        large_reader("wrap", "return p::get(r-r);")
    );
    assert_eq!(forced_compiled_lines(&nested).0, ["T|value=7 q=99"]);
}
