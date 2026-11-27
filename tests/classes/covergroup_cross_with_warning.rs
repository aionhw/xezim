//! §19.6.1: a cross bin's `with (expr)` is evaluated over the values of the
//! crossed coverpoints. An `expr` that names the sampled variable `x`
//! instead of coverpoint `cx` cannot be evaluated there, and the bin selects
//! nothing. One commercial simulator rejects such a bin and another warns;
//! xezim built the empty bin without a word. It now warns once per `with`
//! expression and keeps the empty bin.
use std::process::Command;

fn run(tag: &str, src: &str) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("xezim_cross_with_{}_{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tb.sv"), src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&dir)
        .args(["--no-cache", "--simulate", "-s", "tb", "tb.sv"])
        .output()
        .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(
        out.status.success(),
        "{tag}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn design(with: &str) -> String {
    format!(
        r#"module tb;
  bit [1:0] x, y;
  function automatic int p(real r); return $rtoi(r * 100.0 + 0.5); endfunction
  covergroup cg;
    option.per_instance = 1;
    cx: coverpoint x {{ bins x0 = {{0}}; bins x1 = {{1}}; }}
    cy: coverpoint y {{ bins y0 = {{0}}; bins y1 = {{1}}; }}
    xy: cross cx, cy {{ bins nz = binsof(cx) with ({with}); }}
  endgroup
  cg c = new;
  initial begin
    x = 1; y = 0; c.sample();
    $display("s1 %0d", p(c.xy.get_inst_coverage()));
    x = 0; y = 0; c.sample();
    $display("s2 %0d", p(c.xy.get_inst_coverage()));
  end
endmodule
"#
    )
}

const WARN: &str = "the `with` expression of a bin of cross 'xy' cannot be evaluated";

/// The issue's case: `with (x != 0)` names the variable, not the
/// coverpoint. One warning, though the bin is looked at on every sample.
#[test]
fn a_with_expression_naming_the_sampled_variable_warns_once() {
    let (out, err) = run("var", &design("x != 0"));
    assert_eq!(err.matches(WARN).count(), 1, "{err}");
    assert!(err.contains("coverpoints (cx, cy)"), "{err}");
    assert!(out.lines().any(|l| l.trim() == "s1 2500"), "{out}");
    assert!(out.lines().any(|l| l.trim() == "s2 5000"), "{out}");
}

/// Guard: the legal form names the coverpoint; no warning, and two
/// commercial simulators agree on the figures.
#[test]
fn a_with_expression_over_the_coverpoint_does_not_warn() {
    let (out, err) = run("cp", &design("cx != 0"));
    assert!(!err.contains(WARN), "{err}");
    assert!(out.lines().any(|l| l.trim() == "s1 3333"), "{out}");
    assert!(out.lines().any(|l| l.trim() == "s2 6667"), "{out}");
}
