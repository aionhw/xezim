//! A statement a compiled method body cannot lower runs in the
//! interpreter and the rest of the body stays compiled; one such statement
//! used to send the whole method to the interpreter. The fallbacks below
//! write a `ref` formal, an `output` formal, a local, the function result
//! and a member, and read a string formal; the compiled statements around
//! them see those values. Expected T| lines are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class acc;
  int total;
  int q[$];
  function int f(int v, ref int r, output int o, input string nm);
    int loc;
    r = v + 1;
    q.push_back(v);
    loc = q.sum();
    o = $countones(v) + loc;
    r = r + $clog2(v);
    total += $countones(loc) + q.size();
    f = loc * 2;
    if (v > 100) f = f + $countones(f);
    if (nm == "ab") f = f + 1;
  endfunction
  function string name_of(string a, int n);
    name_of = a;
    if (n > 1) name_of = {a, "-", $sformatf("%0d", $countones(n))};
  endfunction
endclass
module top;
  initial begin
    automatic acc a = new;
    automatic int r, o, s = 0;
    for (int i = 1; i <= 4; i++) begin
      s += a.f(i * 50, r, o, "ab");
      $display("T|i=%0d s=%0d r=%0d o=%0d total=%0d", i, s, r, o, a.total);
    end
    $display("T|name=%s %s", a.name_of("x", 1), a.name_of("xyz", 7));
  end
endmodule
"#;

const EXPECTED: [&str; 5] = [
    "T|i=1 s=101 r=57 o=53 total=4",
    "T|i=2 s=402 r=108 o=153 total=10",
    "T|i=3 s=1007 r=159 o=304 total=17",
    "T|i=4 s=2014 r=209 o=503 total=27",
    "T|name=x xyz-3",
];

/// T| lines and the compile census of one run at tier 0.
fn run(stmt_fallback: &str) -> (Vec<String>, String) {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_method_stmt_fallback_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, DESIGN).expect("write design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .env("XEZIM_COMPILE_METHODS", "1")
        .env("XEZIM_METHOD_TIER", "0")
        .env("XEZIM_FALLBACK_SITES", "1")
        .env("XEZIM_METHOD_STMT_FALLBACK", stmt_fallback)
        .env_remove("XEZIM_METHOD_CACHE")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    let lines = text
        .lines()
        .filter(|l| l.starts_with("T|"))
        .map(str::to_string)
        .collect();
    (lines, text)
}

#[test]
fn uncompilable_statements_run_interpreted_inside_compiled_methods() {
    let (lines, log) = run("1");
    assert_eq!(lines, EXPECTED);
    for m in ["acc.f", "acc.name_of"] {
        assert!(
            log.contains(&format!("runs-compiled scope={m}")),
            "{m} did not run compiled:\n{log}"
        );
    }
    // XEZIM_METHOD_STMT_FALLBACK=0: the whole method is interpreted, with
    // the same answers.
    let (lines, log) = run("0");
    assert_eq!(lines, EXPECTED);
    assert!(
        !log.contains("runs-compiled scope=acc.f"),
        "acc.f compiled without statement fallback:\n{log}"
    );
}
