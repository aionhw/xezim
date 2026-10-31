//! A statement fallback in a compiled method carries a class handle declared
//! in a nested block (`node nd = new(...)` inside a loop body) with its
//! declared class. Only formals and top-level handles could be carried, so
//! the statement (here a call passing a member as a `ref` actual, which
//! compiled code cannot bind) took its enclosing loop, and the whole method,
//! back to the interpreter. Expected T| lines are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class node;
  bit [63:0] value;
  int ix;
  string name;
  function new(string n, int i); name = n; ix = i; endfunction
endclass
class base_n extends node;
  int extra = 5;
  function new(string n, int i); super.new(n, i); endfunction
endclass
class item;
  int calls;
  function bit get(string n, ref bit [63:0] v, input int ix);
    v = n.len() * 100 + ix;
    calls++;
    return n != "bad";
  endfunction
  function int run(int n);
    run = 0;
    for (int i = 0; i < n; i++) begin
      node nd = new($sformatf("k%0d", i), i);
      // A member as a `ref` actual: this statement runs interpreted and
      // carries the nested handle `nd`.
      if (get(nd.name, nd.value, nd.ix))
        run += nd.value;
    end
    begin
      base_n b = new("bb", 9);
      if (get(b.name, b.value, b.extra))
        run += b.value + b.extra;
    end
  endfunction
endclass
module top;
  initial begin
    item it; it = new;
    $display("T|%0d %0d", it.run(3), it.calls);
  end
endmodule
"#;

fn run(compile: &str, tier: Option<&str>) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_fallback_nested_handle_{}_{}",
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
        .env("XEZIM_COMPILE_METHODS", compile)
        .env_remove("XEZIM_METHOD_TIER")
        .env("XEZIM_FALLBACK_SITES", "1")
        .env_remove("XEZIM_METHOD_CACHE")
        .envs(tier.map(|t| ("XEZIM_METHOD_TIER", t)))
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    text
}

#[test]
fn fallback_carries_a_nested_block_handle() {
    for compile in ["0", "1"] {
        let log = run(compile, Some("0"));
        let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("T|")).collect();
        assert_eq!(lines, ["T|813 4"], "XEZIM_COMPILE_METHODS={compile}");
    }
    let log = run("1", Some("0"));
    assert!(
        log.contains("runs-compiled scope=item.run"),
        "item.run did not run compiled:\n{log}"
    );
    assert!(
        log.contains("carries=nd,run") && log.contains("carries=b,run"),
        "the fallbacks did not carry the nested handles:\n{log}"
    );
}
