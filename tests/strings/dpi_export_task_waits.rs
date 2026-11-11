//! A DPI-exported task called from C returns only when it has finished
//! (#204). Its statements run on the synchronous path, where only `#delay`
//! used to wait: `fork ... join` dropped the rest of the task body and
//! returned, `wait fork` did nothing, and `@(ev)` / `wait(...)` did not
//! block. Each mode is checked by the time the task returned at, not just
//! by whether its increment ran.
//!
//! `wait (expr)` then still hung whenever a timed event was pending — a
//! clock is enough (#279): the condition was parked on a waiter the nested
//! scheduler did not re-check. Every design here has a clock.
//!
//! IEEE 1800 §35.8 lets only an imported *task* call an exported task, and
//! the reference simulator rejects the call from an imported function. Each
//! design runs both ways: through a `context` imported task (on its own
//! stack, `dpi_task.rs`; the expected values are the reference simulator's)
//! and through an imported function, which xezim accepts and runs on the
//! nested scheduler with the same results.
//!
//! Where there are no fibers (an architecture other than x86_64 or aarch64)
//! an imported task takes that nested scheduler too; `XEZIM_DPI_FIBERS=0`
//! selects it here, and every design runs both ways (#290).

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn shared_lib(dir: &Path, stem: &str, src: &str) -> PathBuf {
    let c = dir.join(format!("{stem}.c"));
    let so = dir.join(format!("{stem}.so"));
    std::fs::write(&c, src).unwrap();
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("include");
    let mut cc = Command::new("cc");
    cc.args(["-shared", "-fPIC"]);
    if cfg!(target_os = "macos") {
        // The exported subroutines resolve when xezim loads the library.
        cc.arg("-Wl,-undefined,dynamic_lookup");
    }
    let ok = cc
        .arg("-I")
        .arg(&include)
        .arg(&c)
        .arg("-o")
        .arg(&so)
        .status()
        .expect("failed to launch cc")
        .success();
    assert!(ok, "cc failed for {}", c.display());
    so
}

fn run(dir: &Path, lib: &Path, sv: &str, stack: Stack) -> String {
    std::fs::write(dir.join("top.sv"), sv).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.env(
        "XEZIM_DPI_FIBERS",
        match stack {
            Stack::Fibers => "1",
            Stack::Nested => "0",
        },
    );
    let out = cmd
        .arg("--dpi-lib")
        .arg(lib)
        // A hang runs into this instead of the default limit.
        .args(["--no-cache", "--max-time", "100us", "-s", "top", "top.sv"])
        .current_dir(dir)
        .output()
        .expect("failed to run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    text
}

/// Where an imported task's waits run.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stack {
    /// Each call on its own stack (`dpi_task.rs`).
    Fibers,
    /// The scheduler nested in the C call (`XEZIM_DPI_FIBERS=0`, and every
    /// platform without fibers).
    Nested,
}

const STACKS: [Stack; 2] = [Stack::Fibers, Stack::Nested];

/// How the C entry points are declared.
#[derive(Clone, Copy, Debug)]
enum Import {
    /// `import "DPI-C" context task`: the C side returns `int`.
    Task,
    /// `import "DPI-C" function void` (outside the standard, see above).
    Function,
    /// `import "DPI-C" context function void`, as #279 reported it.
    ContextFunction,
}

/// Run `sv` with `c` for one import form. `sv` declares its imports as
/// `IMPORT name(args);` and `c` its entry points as `ENTRY name(args) {`,
/// ending each with `DONE`.
fn run_as(tag: &str, form: Import, stack: Stack, sv: &str, c: &str) -> String {
    let (imp, entry, done) = match form {
        Import::Task => ("import \"DPI-C\" context task", "int", "return 0;"),
        Import::Function => ("import \"DPI-C\" function void", "void", "return;"),
        Import::ContextFunction => ("import \"DPI-C\" context function void", "void", "return;"),
    };
    let sv = sv.replace("IMPORT", imp);
    let c = c.replace("ENTRY", entry).replace("DONE", done);
    let d = scratch(&format!("{tag}_{form:?}_{stack:?}"));
    let lib = shared_lib(&d, "from_c", &c);
    let text = run(&d, &lib, &sv, stack);
    let _ = std::fs::remove_dir_all(&d);
    text
}

/// Each import form on each stack. An imported function always runs on the
/// nested scheduler, so it needs no second run.
fn forms_and_stacks(forms: &[Import]) -> Vec<(Import, Stack)> {
    let mut out = Vec::new();
    for &form in forms {
        for stack in STACKS {
            if stack == Stack::Fibers || matches!(form, Import::Task) {
                out.push((form, stack));
            }
        }
    }
    out
}

fn expect_all(text: &str, form: Import, stack: Stack, lines: &[&str]) {
    for line in lines {
        assert!(
            text.contains(line),
            "{form:?} {stack:?}: missing `{line}`:\n{text}"
        );
    }
}

const SV: &str = r#"
`timescale 1ns/1ps
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  event done_ev, done2;
  bit req = 0, req2 = 0;
  int sink = 0;
  initial forever begin @(posedge req);  req = 0;  #100; -> done_ev; end
  initial forever begin @(posedge req2); req2 = 0; #50;  -> done2;   end

  task automatic c_delay();    #1000; sink++; endtask
  task automatic c_event();    req = 1; wait (done_ev); sink++; endtask
  task automatic c_at();       req2 = 1; @(done2); sink++; endtask
  task automatic c_forkjoin(); fork #500; join sink++; endtask
  task automatic c_joinany();  fork #300; #700; join_any sink++; endtask
  task automatic c_waitfork(); fork begin #500; end join_none wait fork; sink++; endtask

  export "DPI-C" task c_delay;
  export "DPI-C" task c_event;
  export "DPI-C" task c_at;
  export "DPI-C" task c_forkjoin;
  export "DPI-C" task c_joinany;
  export "DPI-C" task c_waitfork;
  IMPORT from_c(int which);

  initial begin
    for (int m = 0; m < 6; m++) begin
      from_c(m);
      $display("R|mode %0d t=%0t sink=%0d", m, $time, sink);
      sink = 0;
    end
    $finish;
  end
endmodule
"#;

const C: &str = r#"
extern int c_delay(void), c_event(void), c_at(void);
extern int c_forkjoin(void), c_joinany(void), c_waitfork(void);
ENTRY from_c(int which) {
    switch (which) {
        case 0: c_delay(); break;
        case 1: c_event(); break;
        case 2: c_at(); break;
        case 3: c_forkjoin(); break;
        case 4: c_joinany(); break;
        case 5: c_waitfork(); break;
    }
    DONE
}
"#;

#[test]
fn exported_tasks_called_from_c_block_until_they_finish() {
    for (form, stack) in forms_and_stacks(&[Import::Task, Import::Function]) {
        let text = run_as("dpi_export_waits", form, stack, SV, C);
        expect_all(
            &text,
            form,
            stack,
            &[
                "R|mode 0 t=1000000 sink=1", // #1000
                "R|mode 1 t=1100000 sink=1", // wait(ev): +100
                "R|mode 2 t=1150000 sink=1", // @(ev): +50
                "R|mode 3 t=1650000 sink=1", // fork ... join: +500
                "R|mode 4 t=1950000 sink=1", // fork ... join_any: +300
                "R|mode 5 t=2450000 sink=1", // wait fork: also waits for mode 4's #700 child
            ],
        );
    }
}

/// #279 as reported: the exported task's `wait` on a variable its own
/// forked child changes, with a timed event pending when C calls it — a
/// clock, another process's delay, or the calling process's own.
#[test]
fn exported_task_wait_with_a_timed_event_pending() {
    const SV279: &str = r#"
module top;
    IMPORT c_call();
    int counter  = 0;
    int snapshot = 0;
    PENDING
    task automatic m(input int id);
        snapshot = counter;
        fork
            begin #10; counter = counter + 1; end
        join_none
        wait (snapshot != counter);
        $display("[SV] resumed t=%0t counter=%0d", $time, counter);
    endtask
    export "DPI-C" task m;
    initial begin
        OWN
        c_call();
        $display("[SV] back in SV at t=%0t", $time);
        $finish;
    end
endmodule
"#;
    const C279: &str = "extern int m(int id);\nENTRY c_call(void) { m(1); DONE }\n";
    for (pending, own) in [
        ("logic clk = 0; always #5 clk = ~clk;", ""),
        ("initial begin #500; end", ""),
        ("", "fork #500; join_none"),
    ] {
        let sv = SV279.replace("PENDING", pending).replace("OWN", own);
        for (form, stack) in
            forms_and_stacks(&[Import::Task, Import::Function, Import::ContextFunction])
        {
            let text = run_as("dpi_export_wait279", form, stack, &sv, C279);
            expect_all(
                &text,
                form,
                stack,
                &[
                    "[SV] resumed t=10 counter=1\n[SV] back in SV at t=10\n",
                    "Simulation finished at time 10 ($finish called)",
                ],
            );
        }
    }
}

/// The shapes of `wait` in an exported task called from C, one after the
/// other while a clock runs: a variable a forked child changes, a condition
/// already true, an event's `.triggered`, a bare event, a net, a variable
/// another process changes compared against an automatic variable, and a net
/// falling.
#[test]
fn exported_task_wait_shapes_from_c() {
    const SVW: &str = r#"
`timescale 1ns/1ps
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  int counter = 0, snapshot = 0, flag = 1, sink = 0;
  event ev, ev2;
  bit r = 0;
  wire w = r;
  bit req = 0;
  int other = 0;
  initial forever begin @(posedge req); req = 0; #40; other = other + 1; end

  task automatic t_child();   snapshot = counter; fork begin #10; counter++; end join_none
                              wait (snapshot != counter); sink++; endtask
  task automatic t_true();    wait (flag == 1); sink++; endtask
  task automatic t_trig();    fork begin #30; -> ev; end join_none wait (ev.triggered); sink++; endtask
  task automatic t_ev();      fork begin #25; -> ev2; end join_none wait (ev2); sink++; endtask
  task automatic t_net();     fork begin #20; r = 1; end join_none wait (w); r = 0; sink++; endtask
  task automatic t_other();   int s = other; req = 1; wait (other != s); sink++; endtask
  task automatic t_netfall(); r = 1; #1; fork begin #15; r = 0; end join_none wait (!w); sink++; endtask

  export "DPI-C" task t_child;
  export "DPI-C" task t_true;
  export "DPI-C" task t_trig;
  export "DPI-C" task t_ev;
  export "DPI-C" task t_net;
  export "DPI-C" task t_other;
  export "DPI-C" task t_netfall;
  IMPORT from_c(int which);

  initial begin
    #3;
    for (int m = 0; m < 7; m++) begin
      from_c(m);
      $display("T|mode %0d t=%0t sink=%0d", m, $time, sink);
      sink = 0;
    end
    $finish;
  end
endmodule
"#;
    const CW: &str = r#"
extern int t_child(void), t_true(void), t_trig(void), t_ev(void);
extern int t_net(void), t_other(void), t_netfall(void);
ENTRY from_c(int which) {
    switch (which) {
        case 0: t_child(); break;
        case 1: t_true(); break;
        case 2: t_trig(); break;
        case 3: t_ev(); break;
        case 4: t_net(); break;
        case 5: t_other(); break;
        case 6: t_netfall(); break;
    }
    DONE
}
"#;
    for (form, stack) in
        forms_and_stacks(&[Import::Task, Import::Function, Import::ContextFunction])
    {
        let text = run_as("dpi_export_wait_shapes", form, stack, SVW, CW);
        expect_all(
            &text,
            form,
            stack,
            &[
                "T|mode 0 t=13000 sink=1",  // child's #10
                "T|mode 1 t=13000 sink=1",  // already true
                "T|mode 2 t=43000 sink=1",  // .triggered: +30
                "T|mode 3 t=68000 sink=1",  // bare event: +25
                "T|mode 4 t=88000 sink=1",  // net rises: +20
                "T|mode 5 t=128000 sink=1", // other process: +40
                "T|mode 6 t=144000 sink=1", // net falls: #1 + 15
            ],
        );
    }
}

/// Several processes in C at once, each waiting in an exported task, one of
/// them through an instance's own import and export; and other processes'
/// waits keep resuming while a task entered from C sleeps.
#[test]
fn concurrent_c_callers_wait_in_exported_tasks() {
    const SVC: &str = r#"
`timescale 1ns/1ns
module sub;
  int hits = 0;
  wire n = top.cnt[1];
  task automatic sw(); wait (n); hits++; $display("T|sub.sw woke t=%0t hits=%0d", $time, hits); endtask
  export "DPI-C" task sw;
  IMPORT sub_call();
  initial begin #4; sub_call(); $display("T|sub back t=%0t", $time); end
endmodule
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  int cnt = 0;
  always @(posedge clk) cnt <= cnt + 1;
  event ev;
  initial #12 -> ev;
  sub u_sub();
  task automatic wa(input int id, input int lim);
    wait (cnt >= lim);
    $display("T|wa %0d woke t=%0t cnt=%0d", id, $time, cnt);
  endtask
  task automatic we(input int id);
    wait (ev.triggered);
    $display("T|we %0d woke t=%0t", id, $time);
  endtask
  export "DPI-C" task wa;
  export "DPI-C" task we;
  IMPORT call_wa(int id, int lim);
  IMPORT call_we(int id);
  initial begin #2; call_wa(1, 3); $display("T|back 1 t=%0t", $time); end
  initial begin #3; call_wa(2, 1); $display("T|back 2 t=%0t", $time); end
  initial begin #3; call_we(3);    $display("T|back 3 t=%0t", $time); end
  initial begin #3; call_wa(4, 0); $display("T|back 4 t=%0t", $time); end
  initial #100 $finish;
endmodule
"#;
    const CC: &str = r#"
extern int wa(int id, int lim), we(int id), sw(void);
ENTRY call_wa(int id, int lim) { wa(id, lim); DONE }
ENTRY call_we(int id) { we(id); DONE }
ENTRY sub_call(void) { sw(); DONE }
"#;
    // Each caller resumes on its own (the reference simulator's order).
    // Only on fibers: on one stack (imported functions, or the nested
    // fallback) an outer caller resumes only after the ones above it have
    // returned, and a wait on `.triggered` that held meanwhile is missed —
    // `concurrent_c_callers_on_one_stack` checks that order.
    let text = run_as("dpi_export_wait_conc", Import::Task, Stack::Fibers, SVC, CC);
    let got: Vec<&str> = text.lines().filter(|l| l.starts_with("T|")).collect();
    assert_eq!(
        got,
        [
            "T|wa 4 woke t=3 cnt=0",
            "T|back 4 t=3",
            "T|wa 2 woke t=5 cnt=1",
            "T|back 2 t=5",
            "T|we 3 woke t=12",
            "T|back 3 t=12",
            "T|sub.sw woke t=15 hits=1",
            "T|sub back t=15",
            "T|wa 1 woke t=25 cnt=3",
            "T|back 1 t=25",
        ],
        "{text}"
    );
    const SVP: &str = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  event ev3;
  int v = 0;
  initial #20 -> ev3;
  initial #30 v = 7;
  initial begin wait (ev3.triggered); $display("T|peer trig t=%0t", $time); end
  initial begin wait (v == 7); $display("T|peer v t=%0t", $time); end
  task automatic slow(); #100; $display("T|slow done t=%0t", $time); endtask
  export "DPI-C" task slow;
  IMPORT call_slow();
  initial begin #1; call_slow(); $display("T|back t=%0t", $time); #1 $finish; end
endmodule
"#;
    const CP: &str = "extern int slow(void);\nENTRY call_slow(void) { slow(); DONE }\n";
    for (form, stack) in forms_and_stacks(&[Import::Task, Import::Function]) {
        let text = run_as("dpi_export_wait_peer", form, stack, SVP, CP);
        let got: Vec<&str> = text.lines().filter(|l| l.starts_with("T|")).collect();
        assert_eq!(
            got,
            [
                "T|peer trig t=20",
                "T|peer v t=30",
                "T|slow done t=101",
                "T|back t=101",
            ],
            "{form:?} {stack:?}:\n{text}"
        );
    }
}

/// Several processes enter imported tasks in one time slot, each waiting in
/// an exported task on a level. On fibers each resumes when its own wait
/// holds. On one stack (the nested-scheduler fallback, #290) the calls nest
/// last-in, first-out: an outer caller resumes once the ones above it have
/// returned. The callers after the first one in the slot used to be lost —
/// the slot's batch kept them where the nested scheduler could not see
/// them — and a `$finish` deferred to the end of a slot did not stop the
/// nested scheduler, which ran on to `--max-time`.
#[test]
fn concurrent_c_callers_on_one_stack() {
    const SVL: &str = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  int cnt = 0;
  always @(posedge clk) cnt <= cnt + 1;
  int never = 0;
  task automatic wa(input int id, input int lim);
    wait (cnt >= lim);
    $display("T|wa %0d woke t=%0t cnt=%0d", id, $time, cnt);
  endtask
  task automatic wn(); wait (never == 1); $display("T|wn woke t=%0t", $time); endtask
  export "DPI-C" task wa;
  export "DPI-C" task wn;
  IMPORT call_wa(int id, int lim);
  IMPORT call_wn();
  initial begin #2; call_wa(1, 3); $display("T|back 1 t=%0t", $time); end
  initial begin #3; call_wa(2, 1); $display("T|back 2 t=%0t", $time); end
  initial begin #3; call_wa(3, 2); $display("T|back 3 t=%0t", $time); end
  initial begin #3; call_wa(4, 0); $display("T|back 4 t=%0t", $time); end
  initial begin #26; call_wa(5, 50); $display("T|back 5 t=%0t", $time); end
  initial begin #30; call_wn(); $display("T|back n t=%0t", $time); end
  initial begin #100; $display("T|fin t=%0t", $time); $finish; end
endmodule
"#;
    const CL: &str = r#"
extern int wa(int id, int lim), wn(void);
ENTRY call_wa(int id, int lim) { wa(id, lim); DONE }
ENTRY call_wn(void) { wn(); DONE }
"#;
    for stack in STACKS {
        let text = run_as("dpi_export_wait_lifo", Import::Task, stack, SVL, CL);
        let got: Vec<&str> = text.lines().filter(|l| l.starts_with("T|")).collect();
        let want: &[&str] = match stack {
            Stack::Fibers => &[
                "T|wa 4 woke t=3 cnt=0",
                "T|back 4 t=3",
                "T|wa 2 woke t=5 cnt=1",
                "T|back 2 t=5",
                "T|wa 3 woke t=15 cnt=2",
                "T|back 3 t=15",
                "T|wa 1 woke t=25 cnt=3",
                "T|back 1 t=25",
                "T|fin t=100",
            ],
            Stack::Nested => &[
                "T|wa 4 woke t=3 cnt=0",
                "T|back 4 t=3",
                "T|wa 3 woke t=15 cnt=2",
                "T|back 3 t=15",
                "T|wa 2 woke t=15 cnt=2",
                "T|back 2 t=15",
                "T|wa 1 woke t=25 cnt=3",
                "T|back 1 t=25",
                "T|fin t=100",
            ],
        };
        assert_eq!(got, want, "{stack:?}:\n{text}");
        assert!(
            text.contains("Simulation finished at time 100 ($finish called)"),
            "{stack:?}:\n{text}"
        );
    }
}

/// A `wait` in an exported task that nothing ever satisfies: the C call never
/// returns, so neither the rest of the task nor its caller runs; the run ends
/// when nothing is left to do (reference simulator: only the `final` block
/// prints) or at `--max-time` with a clock running. It used to resume at
/// whatever time the run had reached instead.
#[test]
fn exported_task_wait_never_satisfied_ends_the_run() {
    const SVN: &str = r#"
module top;
  int never = 0;
  CLOCK
  task automatic m(); wait (never == 1); $display("T|resumed t=%0t", $time); endtask
  export "DPI-C" task m;
  IMPORT c_call();
  initial begin #3; c_call(); $display("T|back t=%0t", $time); $finish; end
  final $display("T|final t=%0t", $time);
endmodule
"#;
    const CN: &str = "extern int m(void);\nENTRY c_call(void) { m(); DONE }\n";
    for (clock, end) in [("", 3), ("logic clk = 0; always #5 clk = ~clk;", 100_000)] {
        let sv = SVN.replace("CLOCK", clock);
        for (form, stack) in forms_and_stacks(&[Import::Task, Import::Function]) {
            let text = run_as("dpi_export_wait_never", form, stack, &sv, CN);
            let got: Vec<&str> = text.lines().filter(|l| l.starts_with("T|")).collect();
            assert_eq!(
                got,
                [format!("T|final t={end}")],
                "{form:?} {stack:?}:\n{text}"
            );
            assert!(
                text.contains(&format!("Simulation finished at time {end}\n")),
                "{form:?} {stack:?}:\n{text}"
            );
        }
    }
}

/// §9.6.1: `wait fork` in an exported task waits for the calling process's
/// immediate children only. The child joined first has finished, so its
/// `join_none` grandchild (handed up to the caller for `disable fork`) is
/// not waited for. The reference simulator does not accept timing controls
/// in an exported task, so these values follow the standard.
#[test]
fn exported_task_wait_fork_skips_a_finished_childs_grandchild() {
    let sv = r#"
`timescale 1ns/1ps
module top;
  int g = 0, sink = 0;
  task automatic c_waitfork();
    fork begin fork #700 g++; join_none end join
    wait fork;
    sink++;
  endtask
  export "DPI-C" task c_waitfork;
  IMPORT from_c();
  initial begin
    from_c();
    $display("R|wf t=%0t g=%0d sink=%0d", $time, g, sink);
    #1000 $display("R|wfend g=%0d", g);
    $finish;
  end
endmodule
"#;
    let c = r#"
extern int c_waitfork(void);
ENTRY from_c() {
    c_waitfork();
    DONE
}
"#;
    for (form, stack) in forms_and_stacks(&[Import::Task, Import::Function]) {
        let text = run_as("dpi_wait_fork_grandchild", form, stack, sv, c);
        expect_all(&text, form, stack, &["R|wf t=0 g=0 sink=1", "R|wfend g=1"]);
    }
}
