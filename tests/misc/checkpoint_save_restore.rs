//! Checkpoints: `--save-at` and `$save` write the state at the end of a time
//! slot, and `--restore` or `$restart` continue from it, printing what the
//! uninterrupted run prints after that slot. A checkpoint belongs to one
//! design and one xezim build; anything else is refused.

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
