//! §9.4.2: a process parked on an event control is examined when a signal
//! of its sensitivity list is written, not at every scheduling pass, and
//! setting a design up does not search every instance scope for each port
//! connection (issue #1194). The `waiter_iters` profile counter counts the
//! parked waiters the scheduler examines; `implicit_net_scope_steps` counts
//! the scopes walked while placing implicit nets (§6.10). The ceilings keep
//! both from growing with the idle population again. The design is the
//! issue's test case, self-checking against a procedural mirror model.
use std::process::Command;

/// A scratch directory holding `src` as `design.sv`.
fn design_dir(src: &str) -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_waiter_index_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    std::fs::write(dir.join("design.sv"), src).expect("write design");
    dir
}

/// Run the design in `dir` with `args`: its `T|` lines, the `waiter_iters`
/// and `implicit_net_scope_steps` counters, and the whole transcript.
fn run_in(dir: &std::path::Path, args: &[&str]) -> (Vec<String>, u64, u64, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "top", "--no-cache"])
        .args(args)
        .arg(dir.join("design.sv"))
        .env("XEZIM_PROFILE_TIMING", "1")
        .output()
        .expect("run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    let lines = text
        .lines()
        .filter(|l| l.starts_with("T|"))
        .map(str::to_string)
        .collect();
    let counter = |name: &str| -> u64 {
        text.lines()
            .filter(|l| l.starts_with("[PROF]"))
            .flat_map(str::split_whitespace)
            .find_map(|w| w.strip_prefix(name)?.strip_prefix('='))
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("missing {name}:\n{text}"))
    };
    let (iters, steps) = (counter("waiter_iters"), counter("implicit_net_scope_steps"));
    (lines, iters, steps, text)
}

fn run(src: &str) -> (Vec<String>, u64, u64) {
    let dir = design_dir(src);
    let (lines, iters, steps, _) = run_in(&dir, &[]);
    let _ = std::fs::remove_dir_all(&dir);
    (lines, iters, steps)
}

/// `ROWS` row cells whose level-sensitive `always` calls a task that
/// suspends on `#0`, so each is a process parked on `@(za or zb)`; a core
/// cell and the testbench loop wake on every clock edge. The rows run twice
/// at the start and once per `zb` toggle (every 64 cycles).
const ROWS: u64 = 200;
const CYCLES: u64 = 300;
const DESIGN: &str = r#"
`timescale 1ns / 1ps
module zr (input wire za, input wire zb, output logic [7:0] zq);
  task zw();
    begin
      #0 zq = {za, zb, zq[7:2]};
    end
  endtask
  always @(za or zb) zw();
  initial zq = 8'h00;
endmodule

module zc (input wire zk, output logic [7:0] zv, output logic [7:0] zt);
  logic [7:0] zs;
  logic       zd;
  wire [7:0] zf1, zf2;
  assign zf1 = zv + zs;
  assign zf2 = zf1 + 8'h0f;
  assign zt  = zf2 + zs;
  always @(zk) begin
    if (zk) begin
      #0 zs = zs + 8'd1;
      #0 zv = zs ^ 8'h5a;
    end else begin
      #0 zd = ~zd;
    end
  end
  initial begin
    zs = 8'h00; zv = 8'h00; zd = 1'b0;
  end
endmodule

module top;
  localparam int ZRN = 200;
  localparam int ZCYC = 300;
  localparam int ZBT = 64;
  logic       clk = 1'b0;
  logic       zn  = 1'b0;
  logic       zb  = 1'b0;
  logic [7:0] zv;
  logic [7:0] zt;
  logic [7:0] zq [0:ZRN-1];
  always #5 clk = ~clk;
  zc u_zc (.zk(clk), .zv(zv), .zt(zt));
  genvar gi;
  generate
    for (gi = 0; gi < ZRN; gi = gi + 1) begin : g_rw
      zr u_zr (.za(zn), .zb(zb), .zq(zq[gi]));
    end
  endgenerate
  logic [7:0] mzq, mzs, mzv, mzt;
  int cyc, zck, zrowbad;
  initial begin
    mzq = 8'h00; mzs = 8'h00; mzv = 8'h00; mzt = 8'h00;
    #1 zn = 1'b1;  mzq = {1'b1, zb, mzq[7:2]};
    #1 zn = 1'b0;  mzq = {1'b0, zb, mzq[7:2]};
    for (int c = 0; c < ZCYC; c++) begin
      @(posedge clk);
      cyc++;
      mzs = mzs + 8'd1;
      mzv = mzs ^ 8'h5a;
      mzt = ((mzv + mzs) + 8'h0f) + mzs;
      if ((c % ZBT) == ZBT - 1) begin
        #1 zb = ~zb;
        mzq = {1'b0, zb, mzq[7:2]};
      end
    end
    #1;
    for (int j = 0; j < ZRN; j++) begin
      zck += zq[j];
      if (zq[j] !== mzq) zrowbad++;
    end
    $display("T| cyc=%0d rowbad=%0d zv_ok=%0d zt_ok=%0d sum=%0d mzq=%h",
             cyc, zrowbad, zv === mzv, zt === mzt, zck, mzq);
    $finish;
  end
endmodule
"#;

#[test]
fn idle_parked_waiters_are_not_examined_at_every_pass() {
    let (lines, waiter_iters, scope_steps) = run(DESIGN);
    assert_eq!(
        lines,
        vec!["T| cyc=300 rowbad=0 zv_ok=1 zt_ok=1 sum=3400 mzq=11".to_string()]
    );
    // Each row is examined when `zn` or `zb` moves (six times in the run);
    // the core and the testbench loop at the clock edges they wait for.
    // Examined at every pass, the rows alone took ROWS * 4 * CYCLES looks.
    let wakeups = ROWS * 6 + 4 * CYCLES;
    assert!(
        waiter_iters <= 2 * wakeups,
        "waiter_iters={waiter_iters}: idle parked waiters are examined per pass again \
         (the wake-ups account for {wakeups})"
    );
    // Every `.zq(zq[gi])` connection searched all ROWS instance scopes.
    assert!(
        scope_steps <= ROWS,
        "implicit_net_scope_steps={scope_steps}: the implicit-net placement walks every \
         instance scope per port connection again"
    );
}

/// 128 row cells whose arm port is one bit of a shared vector (§23.3.3), so
/// each row process is parked on an expression term, `@(wl[i] or zb)`. One
/// bit of the vector toggles every 16 cycles (25 writes in the run).
const SELECT_PORT_DESIGN: &str = r#"
`timescale 1ns / 1ps
module zr (input wire za, input wire zb, output logic [7:0] zq);
  task zw();
    begin
      #0 zq = {za, zb, zq[7:2]};
    end
  endtask
  always @(za or zb) zw();
  initial zq = 8'h00;
endmodule

module zc (input wire zk, output logic [7:0] zv);
  logic [7:0] zs;
  always @(zk) begin
    if (zk) begin
      #0 zs = zs + 8'd1;
      #0 zv = zs ^ 8'h5a;
    end
  end
  initial begin
    zs = 8'h00; zv = 8'h00;
  end
endmodule

module top;
  localparam int ZRN = 128;
  localparam int ZCYC = 400;
  logic           clk = 1'b0;
  logic [ZRN-1:0] wl = '0;
  logic           zb = 1'b0;
  logic [7:0]     zv;
  logic [7:0]     zq [0:ZRN-1];
  always #5 clk = ~clk;
  zc u_zc (.zk(clk), .zv(zv));
  genvar gi;
  generate
    for (gi = 0; gi < ZRN; gi = gi + 1) begin : g_rw
      zr u_zr (.za(wl[gi]), .zb(zb), .zq(zq[gi]));
    end
  endgenerate
  int zck, k, woke;
  initial begin
    for (int c = 0; c < ZCYC; c++) begin
      @(posedge clk);
      if ((c % 16) == 15) begin
        k = (c * 37) % ZRN;
        #1 wl[k] = ~wl[k];
      end
    end
    #1;
    for (int j = 0; j < ZRN; j++) begin
      zck += zq[j];
      if (zq[j] != 8'h00) woke++;
    end
    $display("T| rows=%0d woke=%0d sum=%0d zv=%0d", ZRN, woke, zck, zv);
    $finish;
  end
endmodule
"#;

/// A waiter on a select of a signal is examined when that signal is
/// written: the 128 rows at the 25 writes of the vector, not at each of
/// the 1,600 passes the clock causes.
#[test]
fn waiters_on_a_select_are_examined_when_its_signal_is_written() {
    let (lines, waiter_iters, _) = run(SELECT_PORT_DESIGN);
    assert_eq!(lines, ["T| rows=128 woke=8 sum=986 zv=202"]);
    let wakeups = 25 * 128 + 4 * 400;
    assert!(
        waiter_iters <= 2 * wakeups,
        "waiter_iters={waiter_iters}: waiters on a select are examined per pass again \
         (the writes account for {wakeups})"
    );
}

/// 64 parked row processes, a named-event waiter and a clock-edge waiter,
/// saved at 30 ns while all of them wait. The rows run once more at 37 ns
/// and at 1041 ns; the clock ticks a hundred times in between.
const CHECKPOINT_DESIGN: &str = r#"
`timescale 1ns/1ns
module row (input wire za, input wire zb, output logic [7:0] zq);
  task zw(); begin #0 zq = {za, zb, zq[7:2]}; end endtask
  always @(za or zb) zw();
  initial zq = 8'h00;
endmodule
module top;
  logic clk = 0, zn = 0, zb = 0;
  wire [7:0] zq [0:63];
  event ev;
  int n_ev, n_clk;
  always #5 clk = ~clk;
  genvar gi;
  generate
    for (gi = 0; gi < 64; gi++) begin : g
      row u (.za(zn), .zb(zb), .zq(zq[gi]));
    end
  endgenerate
  always @(ev) begin #0 n_ev++; end
  initial forever begin @(posedge clk); n_clk++; end
  initial begin
    #1 zn = 1;
    #1 zn = 0;
    #20 -> ev;
    #5 $display("T| %0t n_ev=%0d n_clk=%0d zq0=%h zq63=%h", $time, n_ev, n_clk, zq[0], zq[63]);
    #10 zb = 1;
    #1 $display("T| %0t n_ev=%0d n_clk=%0d zq0=%h zq63=%h", $time, n_ev, n_clk, zq[0], zq[63]);
    #3 -> ev;
    #1000 zn = 1;
    #1 $display("T| %0t n_ev=%0d n_clk=%0d zq0=%h zq63=%h", $time, n_ev, n_clk, zq[0], zq[63]);
    $finish;
  end
endmodule
"#;

/// A checkpoint holds the parked waiters, not the scheduler's index of
/// them: a restored run prints what the uninterrupted run prints after the
/// saved slot, and still examines the idle rows only when they wake.
#[test]
fn a_restored_run_examines_parked_waiters_by_signal_again() {
    let dir = design_dir(CHECKPOINT_DESIGN);
    let (full, ..) = run_in(&dir, &[]);
    let ck = dir.join("c.ckpt");
    let spec = format!("30ns,{}", ck.display());
    let (saved, _, _, text) = run_in(&dir, &["--save-at", &spec]);
    assert!(text.contains("saved"), "{text}");
    let (restored, waiter_iters, _, text) = run_in(&dir, &["--restore", ck.to_str().unwrap()]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        full,
        [
            "T| 27 n_ev=1 n_clk=3 zq0=20 zq63=20",
            "T| 38 n_ev=1 n_clk=4 zq0=48 zq63=48",
            "T| 1042 n_ev=2 n_clk=104 zq0=d2 zq63=d2",
        ]
    );
    assert_eq!(saved, full, "saving must not change the run");
    assert!(text.contains("restored"), "{text}");
    assert_eq!(restored, full[1..], "{text}");
    // One look at every restored waiter, two wake-ups of the 64 rows, and
    // the clock-edge waiter at its 101 edges; examined at every pass, the
    // rows took over 28,000 looks.
    assert!(
        waiter_iters <= 1500,
        "waiter_iters={waiter_iters}: a restored run examines idle parked waiters per pass"
    );
}
