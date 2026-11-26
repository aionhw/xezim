//! Checkpoints: `--save-at` and `$save` write the state at the end of a time
//! slot, and `--restore` or `$restart` continue from it, printing what the
//! uninterrupted run prints after that slot. A checkpoint belongs to one
//! design and one xezim build; anything else is refused.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_ckpt_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Run `design` with `args` and `env`; (stdout lines of the design, stderr).
fn run(design: &Path, args: &[&str], env: &[(&str, &str)]) -> (Vec<String>, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_xezim"));
    c.args(["--no-cache"]).args(args).arg(design);
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().expect("run xezim");
    let lines = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("R "))
        .map(str::to_string)
        .collect();
    (lines, String::from_utf8_lossy(&out.stderr).into_owned())
}

fn case_arm_design(selector: &str, condition: &str, stimulus: &str) -> String {
    let mut design = format!(
        "`timescale 1ns/1ps\nmodule tb;\n  {selector}\n  logic [7:0] a, drive;\n  wire [7:0] b = ~drive;\n"
    );
    // Give each data signal enough dependents to enable dynamic masking.
    for i in 0..6 {
        writeln!(
            design,
            r#"  logic [7:0] y{i};
  always @* begin
    case ({condition})
      2'b00: y{i} = a;
      2'b01: y{i} = b;
      2'b10: y{i} = 8'd{i};
      default: y{i} = 8'hff;
    endcase
  end"#
        )
        .unwrap();
    }
    writeln!(design, "{stimulus}\nendmodule").unwrap();
    design
}

/// §11.5.1 and §12.5: a narrow selector at a high packed offset still
/// tracks changes to the data used by its selected arm.
#[test]
fn case_arm_mask_reads_selector_above_bit_65535() {
    let d = dir("wide_case_arm");
    let f = d.join("tb.sv");
    let design = case_arm_design(
        "logic [65537:0] sel;",
        "sel[65537:65536]",
        r#"  initial begin
    sel = '0; sel[65536] = 1; a = 8'h11; drive = 8'hdd;
    #2 $display("R before=%h", y0);
    drive = 8'hbb;
    #2 $display("R after=%h", y0);
    sel[65536] = 1'bx;
    #2 $display("R unknown=%h", y0);
    $finish;
  end"#,
    );
    std::fs::write(&f, design).unwrap();
    let backends: &[(&str, &str)] = if cfg!(feature = "jit") {
        &[("0", "0"), ("1", "0"), ("1", "1")]
    } else {
        &[("0", "0")]
    };
    for &(jit, aot) in backends {
        let (lines, err) = run(
            &f,
            &["--sim-debug"],
            &[("XEZIM_CTL_MASK", "1"), ("XEZIM_JIT", jit), ("XEZIM_AOT", aot)],
        );
        assert!(err.contains("[CASE-CTL] 6 "), "{err}");
        assert_eq!(
            lines,
            ["R before=22", "R after=44", "R unknown=ff"],
            "XEZIM_JIT={jit} XEZIM_AOT={aot}: {err}"
        );
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// Annex D.9 and §12.5: the dependency masks and the selected-arm state
/// must agree when a checkpoint is restored.
#[test]
fn case_arm_masks_follow_selector_after_checkpoint_restore() {
    let d = dir("case_arm_mask");
    let f = d.join("tb.sv");
    let design = case_arm_design(
        "logic [1:0] sel;",
        "sel",
        r#"  initial begin #1 $display("checkpoint marker"); end
  initial begin
    sel = 0; a = 8'h11; drive = 8'hdd;
    #3 sel = 1;
    #1 $display("R selected=%h", y0);
    drive = 8'hbb;
    #2 $display("R after=%h", y0);
    $finish;
  end"#,
    );
    std::fs::write(&f, design).unwrap();
    let ck = d.join("case.ckpt");
    let spec = format!("2ns,{}", ck.display());
    let (saved, err) = run(
        &f,
        &["--sim-debug", "--save-at", &spec],
        &[("XEZIM_CTL_MASK", "1")],
    );
    assert!(err.contains("[CASE-CTL] 6 "), "{err}");
    assert!(err.contains("at time 1000"), "{err}");
    assert_eq!(saved, ["R selected=22", "R after=44"], "{err}");
    for mask in ["1", "0"] {
        let (restored, err) = run(
            &f,
            &["--restore", ck.to_str().unwrap()],
            &[("XEZIM_CTL_MASK", mask)],
        );
        assert!(err.contains("restored"), "{err}");
        assert_eq!(restored, saved, "XEZIM_CTL_MASK={mask}: {err}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

const DESIGN: &str = r#"module tb;
  reg clk = 0;
  integer n = 0;
  int m [int][$];
  int a [2][string];
  class C; int p [string][$]; int k; endclass
  C c;
  always #5 clk = ~clk;
  always @(posedge clk) begin n = n + 1; c.k = c.k + n; end
  task automatic t();
    int l [int][$];
    l[3].push_back(5);
    #40;
    l[3].push_back(6);
    $display("R L %0d %p", l[3].size(), l);
  endtask
  initial begin
    c = new;
    m[1].push_back(1); a[1]["k"] = 4; c.p["x"].push_back(7);
    t();
    m[1].push_back(2); a[0]["z"] = 1; c.p["x"].push_back(8);
    $display("R M %0d %p A %p P %p K %0d", n, m, a, c.p, c.k);
    #20 $display("R E %0d %0d", n, c.k);
    $finish;
  end
endmodule
"#;

/// §14.3/§14.16: queued clockvar drives, cycle counts and edge-skew waits
/// belong to the saved simulation state, including their original alias.
#[test]
fn clocking_drives_survive_checkpoints_before_and_after_the_event() {
    let d = dir("clocking_pending");
    let f = d.join("tb.sv");
    std::fs::write(
        &f,
        r#"
`timescale 1ns/1ps
module tb;
 logic clk=0;
 logic [3:0] q=0;
 always #5 clk=~clk;
 clocking cb @(posedge clk); output negedge q; endclocking
 initial begin #1 cb.q<=3; @(cb); cb.q<=##2 7; end
 initial begin
   #11 $display("R first=%0d",q);
   #20 $display("R second=%0d",q);
   $finish;
 end
endmodule
"#,
    )
    .unwrap();
    let (full, err) = run(&f, &[], &[]);
    assert_eq!(full, ["R first=3", "R second=7"], "{err}");
    for time in [2, 6, 16] {
        let ck = d.join(format!("c{time}.ckpt"));
        let spec = format!("{time}ns,{}", ck.display());
        let (saved, err) = run(&f, &["--save-at", &spec], &[]);
        assert_eq!(saved, full, "{err}");
        assert!(err.contains("saved"), "{err}");
        let (restored, err) = run(&f, &["--restore", ck.to_str().unwrap()], &[]);
        assert!(err.contains("restored"), "{err}");
        let expected: Vec<_> = full
            .iter()
            .filter(|line| time < 11 || !line.contains("first="))
            .cloned()
            .collect();
        assert_eq!(restored, expected, "{err}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_restored_run_continues_where_the_saved_one_was() {
    let d = dir("restore");
    let f = d.join("tb.sv");
    std::fs::write(&f, DESIGN).unwrap();
    let (full, _) = run(&f, &[], &[]);
    assert_eq!(full.len(), 3, "{full:?}");
    let ck = d.join("c.ckpt");
    let spec = format!("25ns,{}", ck.display());
    let (saved, err) = run(&f, &["--save-at", &spec], &[]);
    assert_eq!(saved, full, "saving must not change the run: {err}");
    assert!(err.contains("saved"), "{err}");
    let (restored, err) = run(&f, &["--restore", ck.to_str().unwrap()], &[]);
    assert!(err.contains("restored"), "{err}");
    assert_eq!(
        restored, full,
        "a restored run prints what follows the save"
    );
    let _ = std::fs::remove_dir_all(&d);
}
/// §16.14.6: a process saved while it waits on a delay still resumes from a
/// delay after the restore, so the procedural assertion it then reaches
/// attempts at the clock's next tick (15), not at once (8).
#[test]
fn a_restored_delay_keeps_its_assertion_at_the_next_tick() {
    let d = dir("delay_assert");
    let f = d.join("tb.sv");
    std::fs::write(
        &f,
        r#"module tb;
  logic clk = 0;
  always #5 clk = ~clk;
  initial begin
    @(posedge clk);
    #3;
    a: assert property (@(posedge clk) 1) $display("R A %0t", $time);
    #20 $finish;
  end
endmodule
"#,
    )
    .unwrap();
    let (full, _) = run(&f, &[], &[]);
    assert_eq!(full, ["R A 15"]);
    let ck = d.join("c.ckpt");
    let spec = format!("6ns,{}", ck.display());
    let (saved, err) = run(&f, &["--save-at", &spec], &[]);
    assert_eq!(saved, full, "saving must not change the run: {err}");
    let (restored, err) = run(&f, &["--restore", ck.to_str().unwrap()], &[]);
    assert!(err.contains("restored"), "{err}");
    assert_eq!(restored, full, "{err}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn save_and_restart_from_the_design() {
    let d = dir("restart");
    let f = d.join("tb.sv");
    let ck = d.join("s.ckpt");
    let src = format!(
        r#"module tb;
  integer n = 0;
  initial begin
    if ($test$plusargs("again")) $restart("{ck}");
    else begin
      n = 7;
      #10 $save("{ck}");
      n = 8;
    end
  end
  initial begin
    #20 $display("R n=%0d", n);
    $finish;
  end
endmodule
"#,
        ck = ck.display()
    );
    std::fs::write(&f, src).unwrap();
    let (first, err) = run(&f, &[], &[]);
    assert_eq!(first, ["R n=8"], "{err}");
    let (again, err) = run(&f, &["+again"], &[]);
    assert!(err.contains("restarted"), "{err}");
    assert_eq!(
        again,
        ["R n=8"],
        "the run continues from the end of the saved slot"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_checkpoint_of_another_design_is_refused() {
    let d = dir("other");
    let f = d.join("tb.sv");
    std::fs::write(&f, DESIGN).unwrap();
    let ck = d.join("c.ckpt");
    let spec = format!("25ns,{}", ck.display());
    run(&f, &["--save-at", &spec], &[]);
    let g = d.join("other.sv");
    std::fs::write(
        &g,
        "module tb; integer q; initial begin #10 $display(\"R q\"); end endmodule\n",
    )
    .unwrap();
    let (out, err) = run(&g, &["--restore", ck.to_str().unwrap()], &[]);
    assert!(out.is_empty(), "{out:?}");
    assert!(err.contains("taken from a different design"), "{err}");
    let (_, err) = run(
        &f,
        &["--restore", d.join("missing.ckpt").to_str().unwrap()],
        &[],
    );
    assert!(err.contains("cannot restore"), "{err}");
    let _ = std::fs::remove_dir_all(&d);
}

/// A process FSM saved while its native body runs resumes natively; a
/// restore that cannot run it natively says so instead of resuming wrong.
#[cfg(feature = "jit")]
#[test]
fn native_process_bodies_restore_in_every_backend() {
    let d = dir("native");
    let f = d.join("tb.sv");
    std::fs::write(
        &f,
        r#"module tb;
  reg clk = 0; integer n = 0, s = 0, k = 0;
  always #5 clk = ~clk;
  always begin
    @(posedge clk); n = n + 1;
    @(posedge clk); s = s + n * 3;
    #2 k = k + n;
  end
  initial begin
    #100; $display("R %0d %0d %0d", n, s, k);
    #100; $display("R %0d %0d %0d", n, s, k);
    $finish;
  end
endmodule
"#,
    )
    .unwrap();
    let plain: &[(&str, &str)] = &[("XEZIM_PROC_FSM", "1"), ("XEZIM_JIT", "0")];
    let jit: &[(&str, &str)] = &[
        ("XEZIM_PROC_FSM", "1"),
        ("XEZIM_JIT", "1"),
        ("XEZIM_AOT", "0"),
    ];
    let aot: &[(&str, &str)] = &[
        ("XEZIM_PROC_FSM", "1"),
        ("XEZIM_JIT", "1"),
        ("XEZIM_AOT", "1"),
    ];
    let (full, _) = run(&f, &[], plain);
    assert_eq!(full, ["R 5 45 15", "R 10 165 55"]);
    let ck = d.join("c.ckpt");
    let spec = format!("150ns,{}", ck.display());
    let restore = ["--restore", ck.to_str().unwrap()];
    for (save, back) in [
        (plain, jit),
        (jit, plain),
        (jit, aot),
        (aot, aot),
        (plain, aot),
    ] {
        let _ = std::fs::remove_file(&ck);
        run(&f, &["--save-at", &spec], save);
        let (out, err) = run(&f, &restore, back);
        assert_eq!(out, full[1..], "{save:?} -> {back:?}: {err}");
    }
    let _ = std::fs::remove_file(&ck);
    run(&f, &["--save-at", &spec], aot);
    let (out, err) = run(&f, &restore, jit);
    assert!(out.is_empty(), "{out:?}");
    assert!(err.contains("saved running natively"), "{err}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn restored_fixed_class_arrays_preserve_element_widths() {
    let d = dir("fixed_widths");
    let f = d.join("tb.sv");
    std::fs::write(
        &f,
        r#"
`timescale 1ns/1ns
class packet;
    byte data[3:8];
    logic [12:0] flags[8:3];
endclass
module tb;
    packet p;
    initial begin
        p = new;
        p.data[4] = 'h1f1;
        #10;
        p.flags[7][3:0] <= 5;
        #1 $display("R %0d %b %0d %b", $signed(p.data[4]), p.flags[7], p.data[3], p.flags[3]);
        $finish;
    end
endmodule
"#,
    )
    .unwrap();
    let ck = d.join("widths.ckpt");
    let spec = format!("5ns,{}", ck.display());
    let (saved, err) = run(&f, &["--save-at", &spec], &[]);
    assert!(err.contains("saved"), "{err}");
    assert_eq!(saved, ["R -15 xxxxxxxxx0101 0 xxxxxxxxxxxxx"]);
    let (restored, err) = run(&f, &["--restore", ck.to_str().unwrap()], &[]);
    assert!(err.contains("restored"), "{err}");
    assert_eq!(restored, saved, "{err}");
    let _ = std::fs::remove_dir_all(&d);
}
