//! §8.25: a compiled-method plan is keyed by specialization, so a formal or
//! return whose type is a class TYPE parameter (`KEY key`, `function T
//! get`) takes the binding: a `string` binding is a string formal, a class
//! binding a handle. Every such method declined compilation, so a pool's
//! `exists` / `get` / `add` always ran interpreted. An integral binding
//! (`pool#(int, int)`) keeps declining. Expected T| lines are the reference
//! simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class node; int v; function new(int x = 0); v = x; endfunction endclass
class pool #(type KEY = int, type T = int);
  protected T p[KEY];
  static pool #(KEY, T) g;
  static function pool #(KEY, T) get_global();
    if (g == null) g = new;
    return g;
  endfunction
  function bit exists(KEY key); return p.exists(key); endfunction
  function T get(KEY key);
    if (!p.exists(key)) begin
      T dflt;
      p[key] = dflt;
    end
    return p[key];
  endfunction
  function void add(KEY key, T item); p[key] = item; endfunction
  function int num(); return p.num(); endfunction
endclass
class named_pool extends pool #(string, node);
  function int count_like(string pfx);
    int n = 0;
    foreach (p[k]) if (k.substr(0, pfx.len() - 1) == pfx) n++;
    return n;
  endfunction
endclass
module top;
  initial begin
    automatic pool #(string, node) sp = new;
    automatic pool #(int, int) ip = new;
    automatic named_pool np = new;
    automatic int hits = 0, sum = 0;
    for (int i = 0; i < 50; i++) begin
      automatic node nd = new(i);
      sp.add($sformatf("k%0d", i % 10), nd);
      ip.add(i % 7, i);
      np.add($sformatf("%s%0d", (i % 2) ? "a" : "b", i), nd);
    end
    for (int i = 0; i < 20; i++) begin
      if (sp.exists($sformatf("k%0d", i))) hits++;
      if (sp.get($sformatf("k%0d", i)) != null) sum += sp.get($sformatf("k%0d", i)).v;
      sum += ip.get(i);
    end
    $display("T|hits=%0d sum=%0d num=%0d/%0d like=%0d g=%0d", hits, sum, sp.num(), ip.num(), np.count_like("a"),
      pool #(string, node)::get_global() == pool #(string, node)::get_global());
  end
endmodule
"#;

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_type_param_formals_{}_{}",
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
        .env("XEZIM_METHOD_TIER", "0")
        .env("XEZIM_FALLBACK_SITES", "1")
        .env_remove("XEZIM_METHOD_CACHE")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    text
}

#[test]
fn type_param_formals_take_the_specialization_binding() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("T|")).collect();
        assert_eq!(
            lines,
            ["T|hits=10 sum=767 num=20/20 like=25 g=1"],
            "XEZIM_COMPILE_METHODS={compile}"
        );
    }
    let log = run("1");
    for m in ["pool.exists", "pool.get", "pool.add"] {
        assert!(
            log.contains(&format!("runs-compiled scope={m}")),
            "{m} did not run compiled:\n{log}"
        );
    }
}
