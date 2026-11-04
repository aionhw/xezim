//! IEEE 1800-2023 §8.6, §8.24, §6.21: an out-of-block definition of a class
//! method in a package defines a class method, whose locals are automatic.
//! `--relax-implicit-static` gave them the static lifetime of a package
//! subroutine's: an initializer ran once and recursion shared one copy
//! (#621, reported by git-hevangel). Every line passes with and without it.
use std::process::Command;

const SRC: &str = r#"// IEEE 1800-2023 8.6: the lifetime of a method declared in a class is
// automatic, and an out-of-block definition (8.24) defines the same method, so
// its locals are automatic too: an initializer runs on every call and every
// recursive call has its own copy.
//
// `--relax-implicit-static` (or XEZIM_ALLOW_IMPLICIT_STATIC=1) accepts a local
// with an initializer in a STATIC subroutine, which 6.21 makes an error, and
// keeps its static lifetime. It must not touch class methods.
//
// Run it twice, plainly and with the option. Every line should print PASS both
// times. With the bug and the option, the locals of the out-of-block methods
// below are static: `n` keeps its value between calls and the recursion shares
// one `r`.
package pk;
  class acc;
    int seen;
    extern task add(int v);
    extern function int bump(int v);
    extern function int fact(int n);
  endclass

  task acc::add(int v);
    int n = 100;                 // runs on every call
    n += v;
    seen = n;
  endtask

  function int acc::bump(int v);
    int total = 10;
    total += v;
    return total;
  endfunction

  function int acc::fact(int n);
    int r = n;                   // one copy per recursive call
    if (n > 1) r = r * fact(n - 1);
    return r;
  endfunction
endpackage

module t;
  import pk::*;
  int fails = 0;

  task automatic check(string label, int got, int expected);
    if (got == expected) $display("PASS %s", label);
    else begin $display("FAIL %s: got %0d, expected %0d", label, got, expected); fails++; end
  endtask

  initial begin
    acc a, b;
    a = new();
    b = new();
    a.add(1); check("task local reinitialised, call 1", a.seen, 101);
    a.add(2); check("task local reinitialised, call 2", a.seen, 102);
    b.add(3); check("task local, another object",       b.seen, 103);
    check("function local, call 1",      a.bump(1), 11);
    check("function local, call 2",      a.bump(2), 12);
    check("recursive method, 4! = 24",   a.fact(4), 24);
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
"#;

fn run(args: &[&str]) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_relax_static_ooc_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("repro.sv");
    std::fs::write(&path, SRC).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "t", "--no-cache"])
        .args(args)
        .arg(&path)
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn out_of_block_methods_keep_automatic_locals() {
    for args in [&[][..], &["--relax-implicit-static"][..]] {
        let text = run(args);
        assert!(
            text.contains("ALL PASS") && !text.contains("FAIL"),
            "{args:?}:\n{text}"
        );
    }
}
