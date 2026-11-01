//! §6.6.7 + §7.2: a member of a pure function's unpacked-struct result
//! (`res('{...}).V`, the shape of every struct nettype's resolved value)
//! compiles to bytecode instead of re-running the function on the AST path.
//!
//! The guard is the settle profile's per-entry histogram: every continuous
//! assign below must be a compiled entry (`contassign_c`), none an AST one
//! (`contassign_ast`). Before the scalarized inline all three were AST
//! entries. The answer is pinned too, so a compiled-but-wrong result fails.

fn run_profiled(src: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_struct_resolver_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, src).expect("write temporary design");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .env("XEZIM_PROFILE_TIMING", "1")
        .output()
        .expect("run profiled design");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "profiled design failed:\n{text}");
    text
}

/// `name=<n>` from the `[PROF] entry_hist` line; 0 when the kind is absent.
fn entry_hist(text: &str, kind: &str) -> u64 {
    let line = text
        .lines()
        .find(|l| l.starts_with("[PROF] entry_hist"))
        .unwrap_or_else(|| panic!("missing entry_hist:\n{text}"));
    line.split_whitespace()
        .find_map(|tok| tok.strip_prefix(&format!("{kind}=")))
        .map(|n| n.parse().expect("count"))
        .unwrap_or(0)
}

#[test]
fn struct_result_member_compiles() {
    let text = run_profiled(
        r#"
package ee;
  typedef struct { real V; real I; real R; } ee_t;
  function automatic ee_t res(input ee_t d[]);
    ee_t r;
    real g, itot;
    g = 0.0;
    itot = 0.0;
    foreach (d[k]) begin
      g += 1.0 / d[k].R;
      itot += d[k].V / d[k].R + d[k].I;
    end
    r.V = itot / g;
    r.R = 1.0 / g;
    r.I = itot;
    return r;
  endfunction
endpackage
module top;
  import ee::*;
  real v = 2.0, xv, xi, xr;
  assign xv = res('{'{v, 0.0, 1.0e3}, '{0.0, 0.0, 1.0e3}}).V;
  assign xi = res('{'{V: v, I: 0.0, R: 1.0e3}, '{V: 0.0, I: 0.0, R: 1.0e3}}).I;
  assign xr = res('{'{v, 0.0, 1.0e3}, '{0.0, 0.0, 1.0e3}}).R;
  initial begin
    #1 $display("R %.4f %.4f %.1f", xv, xi, xr);
    v = 4.0;
    #1 $display("R %.4f %.4f %.1f", xv, xi, xr);
  end
endmodule
"#,
    );
    assert!(text.contains("R 1.0000 0.0020 500.0"), "{text}");
    assert!(text.contains("R 2.0000 0.0040 500.0"), "{text}");
    assert_eq!(entry_hist(&text, "contassign_ast"), 0, "{text}");
    assert!(entry_hist(&text, "contassign_c") >= 3, "{text}");
}
