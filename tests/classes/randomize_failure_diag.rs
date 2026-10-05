//! `XEZIM_RAND_DIAG=1`: when `randomize()` fails, report on stderr which
//! class failed, which variables and constraint blocks were switched off
//! (`rand_mode` / `constraint_mode`), and which constraint items the trials
//! left unsatisfied, with their source text and location. Nothing is
//! printed without the variable.

use std::process::Command;

const SRC: &str = "\
class item;
  rand int x;
  rand int id;
  rand int y;
  constraint c_lo { x < 5; }
  constraint c_hi { x > 10; }
  constraint c_off { y == 1; }
  constraint c_y { y inside {[0:3]}; }
endclass

module top;
  initial begin
    item it = new();
    it.id.rand_mode(0);
    it.c_off.constraint_mode(0);
    if (!it.randomize()) $display(\"randomize failed\");
  end
endmodule
";

fn run(diag: bool) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("xezim_rand_diag_{}_{}", std::process::id(), diag));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("rand_diag.sv");
    std::fs::write(&file, SRC).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.arg("--simulate").arg("-s").arg("top").arg(&file);
    if diag {
        cmd.env("XEZIM_RAND_DIAG", "1");
    } else {
        cmd.env_remove("XEZIM_RAND_DIAG");
    }
    let out = cmd.output().expect("failed to run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn diag_lines(stderr: &str) -> Vec<&str> {
    stderr
        .lines()
        .filter(|l| l.starts_with("[rand-diag]"))
        .collect()
}

#[test]
fn a_failed_randomize_reports_the_unsatisfied_constraints() {
    let (stdout, stderr) = run(true);
    assert!(stdout.contains("randomize failed"), "{stdout}");
    let lines = diag_lines(&stderr);
    let has = |s: &str| lines.iter().any(|l| l.contains(s));
    assert!(has("randomize() failed: class item"), "{lines:#?}");
    assert!(has("rand_mode off: id"), "{lines:#?}");
    assert!(has("constraint_mode off: c_off"), "{lines:#?}");
    assert!(
        lines
            .iter()
            .any(|l| l.contains("c_lo") && l.contains("rand_diag.sv:5") && l.contains("x < 5")),
        "{lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("c_hi") && l.contains("rand_diag.sv:6") && l.contains("x > 10")),
        "{lines:#?}"
    );
    // Always satisfied and shares no variable with them: listed as an
    // active block, but neither unsatisfied nor related.
    assert!(has("constraint blocks: c_hi, c_lo, c_y"), "{lines:#?}");
    assert!(
        !lines
            .iter()
            .any(|l| l.contains("rand_diag.sv:") && l.contains("c_y")),
        "{lines:#?}"
    );
}

#[test]
fn nothing_is_reported_without_the_variable() {
    let (stdout, stderr) = run(false);
    assert!(stdout.contains("randomize failed"), "{stdout}");
    assert!(diag_lines(&stderr).is_empty(), "{stderr}");
}
