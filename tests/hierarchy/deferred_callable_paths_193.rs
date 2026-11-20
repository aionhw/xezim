//! IEEE 1800-2023 §13.3, §13.4, §23.6 (xezim-core#193): the instance copies
//! of a module function or task share one body until one is used, and every
//! path that uses one sees it rewritten into its own instance: module
//! variables and input ports of the instance, a package function, a type
//! parameter and a local array typedef, a formal and nested locals that shadow
//! module names, static locals per instance, a task that waits, an
//! `always_comb` that reads through a function, and hierarchical calls. The
//! interpreted and compiled method paths, the design cache, a compiled
//! artifact and a restored checkpoint give the same lines. Expected lines
//! are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
package pk;
  function automatic int twice(int x); return 2 * x; endfunction
endpackage
module element #(parameter int K = 0, parameter type T = logic [3:0])
    (input logic clk, input logic [7:0] din, output logic [7:0] dout);
  import pk::*;
  typedef int arr_t [0:1];
  int state;
  logic [7:0] acc;
  function automatic int f(input int v);
    int a;
    a = v + state + K + twice(1);
    return a;
  endfunction
  function int counter();
    static int cnt = 0;
    cnt = cnt + 1 + K;
    return cnt;
  endfunction
  function automatic int shapes();
    T loc;
    arr_t a;
    a[0] = $bits(loc);
    a[1] = $size(a);
    return a[0] * 10 + a[1];
  endfunction
  function automatic logic [7:0] g(input logic [7:0] x);
    return x + din + acc;
  endfunction
  function automatic int shadow(input int state);
    int acc;
    acc = state + 1;
    begin
      int state;
      state = 50;
      acc = acc + state;
    end
    return acc;
  endfunction
  task automatic bump(input int by);
    @(posedge clk);
    state = state + by;
    #1 acc = acc + 1;
  endtask
  task setall(input int v);
    state = v;
    acc = v[7:0];
  endtask
  always_comb dout = g(8'd1);
  initial begin
    setall(10 * (K + 1));
    $display("T|%m f=%0d shapes=%0d shadow=%0d", f(1), shapes(), shadow(3));
    bump(3);
    $display("T|%m bump state=%0d acc=%0d f=%0d counter=%0d,%0d",
             state, acc, f(2), counter(), counter());
  end
endmodule
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [7:0] d0, d1, in0 = 8'd4, in1 = 8'd9;
  element #(.K(0)) u0 (.clk(clk), .din(in0), .dout(d0));
  element #(.K(1), .T(byte)) u1 (.clk(clk), .din(in1), .dout(d1));
  for (genvar j = 0; j < 2; j++) begin : g
    element #(.K(2 + j)) e (.clk(clk), .din(8'(j)), .dout());
  end
  initial begin
    #30;
    $display("T|top d0=%0d d1=%0d u0.f=%0d g1.f=%0d", d0, d1, u0.f(7), g[1].e.f(7));
    u0.setall(100);
    #1 $display("T|top d0=%0d", d0);
    in1 = 8'd20;
    #1 $display("T|top d1=%0d", d1);
    $finish;
  end
endmodule
"#;

const EXPECTED: [&str; 11] = [
    "T|top.u0 f=13 shapes=42 shadow=54",
    "T|top.u1 f=24 shapes=82 shadow=54",
    "T|top.g[0].e f=35 shapes=42 shadow=54",
    "T|top.g[1].e f=46 shapes=42 shadow=54",
    "T|top.g[1].e bump state=43 acc=41 f=50 counter=4,8",
    "T|top.g[0].e bump state=33 acc=31 f=39 counter=3,6",
    "T|top.u1 bump state=23 acc=21 f=28 counter=2,4",
    "T|top.u0 bump state=13 acc=11 f=17 counter=1,2",
    "T|top d0=16 d1=31 u0.f=22 g1.f=55",
    "T|top d0=105",
    "T|top d1=42",
];

fn lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.starts_with("T|"))
        .map(str::to_string)
        .collect()
}

#[test]
fn deferred_instance_callables_simulate_per_instance() {
    let sim = simulate(SRC, 100).expect("simulate");
    let got: Vec<&str> = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|l| l.starts_with("T|"))
        .collect();
    assert_eq!(got, EXPECTED);
}

#[test]
fn deferred_instance_callables_agree_on_every_path() {
    let dir = std::env::temp_dir().join(format!("xezim_callable_paths_193_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("design.sv"), SRC).expect("write source");
    let run = |args: &[&str], interpreted: bool| {
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"));
        cmd.args(args).current_dir(&dir);
        if interpreted {
            cmd.env("XEZIM_COMPILE_METHODS", "0");
        }
        let out = cmd.output().expect("run xezim");
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        lines(&String::from_utf8_lossy(&out.stdout))
    };
    for interpreted in [false, true] {
        assert_eq!(
            run(&["--no-cache", "design.sv"], interpreted),
            EXPECTED,
            "interpreted={interpreted}"
        );
    }
    // A cold design cache, then the cached design.
    for pass in 0..2 {
        assert_eq!(
            run(&["--cache-dir", "cache", "design.sv"], false),
            EXPECTED,
            "cache pass {pass}"
        );
    }
    run(
        &["--compile", "--no-cache", "design.sv", "-o", "design.xez"],
        false,
    );
    assert_eq!(
        run(&["--simulate", "design.xez"], false),
        EXPECTED,
        "artifact"
    );
    // A checkpoint saved at 12 ns, after every task has returned, restores
    // the instances' state for the lines printed from 30 ns on.
    run(
        &["--no-cache", "--save-at", "12ns,design.ckpt", "design.sv"],
        false,
    );
    assert_eq!(
        run(
            &["--no-cache", "--restore", "design.ckpt", "design.sv"],
            false
        ),
        EXPECTED[8..],
        "restored checkpoint"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
