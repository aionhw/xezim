//! §26.3: a call written `pkg::f(...)` in a process compiles to the same
//! inlined body as the imported `f(...)`. It used to fall back to the
//! interpreter (`Expr_Call_func_not_single_ident`) on every evaluation. The
//! callee is the package's own declaration (not a same-named module function
//! or another package's), and its body calls its own package's subroutines
//! and reads its own package's parameters.
use std::process::Command;

fn run(src: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_scoped_pkg_call_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, src).expect("write design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .env("XEZIM_FALLBACK_SITES", "1")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    text
}

fn t_lines(text: &str) -> Vec<&str> {
    text.lines().filter(|l| l.starts_with("T|")).collect()
}

/// `b_pkg` comes first, so the bare hoisted `inner` and `g` are its own.
const DESIGN: &str = r#"
package b_pkg;
  function automatic int inner(int x); return x + 100; endfunction
  function automatic int g(int x); return x * 3; endfunction
endpackage
package a_pkg;
  localparam int K = 5;
  localparam int C = 40;
  function automatic int inner(int x); return x + 1; endfunction
  function automatic int outer(int x); return inner(x) * 2; endfunction
  function automatic int g(int x); return x * 2; endfunction
  function automatic int addk(int x); return x + K; endfunction
  function automatic int addc(int x); return x + C; endfunction
  function automatic int f(int x); return x + 1; endfunction
endpackage
module top;
  localparam int K = 7;
  function automatic int f(int x); return x + 1000; endfunction
  function automatic int inner(int x); return x + 5000; endfunction
  int n = 0;
  int r_f, r_mod_f, r_ag, r_bg, r_k, r_c, r_outer;
  always_comb begin
    r_f = a_pkg::f(n);
    r_mod_f = f(n);
    r_ag = a_pkg::g(n);
    r_bg = b_pkg::g(n);
    r_k = a_pkg::addk(n);
    r_c = a_pkg::addc(n);
    r_outer = a_pkg::outer(n);
  end
  initial begin
    repeat (3) begin
      #1 n = n + 3;
      #1 $display("T|%0d %0d %0d %0d %0d %0d %0d %0d", n, r_f, r_mod_f, r_ag, r_bg, r_k,
                  r_c, r_outer);
    end
  end
endmodule
"#;

#[test]
fn a_scoped_package_call_compiles_to_that_packages_function() {
    let text = run(DESIGN);
    assert_eq!(
        t_lines(&text),
        [
            "T|3 4 1003 6 9 8 43 8",
            "T|6 7 1006 12 18 11 46 14",
            "T|9 10 1009 18 27 14 49 20",
        ],
        "{text}"
    );
    for reason in ["Expr_Call_func_not_single_ident", "ident_lookup"] {
        assert!(
            !text.contains(&format!("reason={reason}")),
            "a scoped call fell back ({reason}):\n{text}"
        );
    }
}

/// `obj.f(n)` and `obj::f(n)` parse alike: a handle named like a package
/// still calls its class's method.
const HANDLE_NAMED_LIKE_PACKAGE: &str = r#"
package obj;
  function automatic int f(int x); return x + 1; endfunction
endpackage
class C;
  function int f(int x); return x + 50; endfunction
endclass
module top;
  C obj = new;
  int n = 0, r;
  always @(n) r = obj.f(n);
  initial begin
    #1 n = 3;
    #1 $display("T|%0d", r);
  end
endmodule
"#;

#[test]
fn a_handle_named_like_a_package_calls_its_method() {
    let text = run(HANDLE_NAMED_LIKE_PACKAGE);
    assert_eq!(t_lines(&text), ["T|53"], "{text}");
}
