//! Under the default tier, a method containing a loop compiles on its first
//! call: one call can run many iterations, so its call count says little
//! about its cost (a pattern walk called a few hundred times stayed
//! interpreted). A loop-free method still waits for the call threshold.
use std::process::Command;

const DESIGN: &str = r#"
class pdata;
  typedef enum { NONE, REF, DEEP } own_e;
  string name;
  int array_ix;
  own_e how;
  int w;
  function new(string n = "anon", int v = 7, int ix = 0, own_e o = REF, int width = 32);
    name = n; array_ix = ix + v; how = o; w = width;
  endfunction
endclass
class item;
  function string mk(string s, int i);
    pdata d = new(s, 0, i, , 16);
    pdata e = new(, , i);
    pdata f = new;
    return $sformatf("%s/%0d/%0d/%0d %s/%0d/%0d %s/%0d", d.name, d.array_ix, d.how, d.w, e.name, e.array_ix, e.how, f.name, f.array_ix);
  endfunction
  function int nest(int n);
    nest = 0;
    for (int i = 0; i < n; i++) begin
      pdata d = new($sformatf("n%0d", i), i, i, pdata::DEEP);
      nest += d.array_ix + d.how;
    end
  endfunction
endclass
module top;
  initial begin
    item it; it = new;
    $display("T|%s", it.mk("z", 3));
    $display("T|%0d", it.nest(4));
  end
endmodule
"#;

fn run(compile: &str, tier: Option<&str>) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_loop_tier_{}_{}",
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
fn default_tier_compiles_a_looping_method_on_its_first_call() {
    let log = run("1", None);
    let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("T|")).collect();
    assert_eq!(lines, ["T|z/3/1/16 anon/10/1 anon/7", "T|20"]);
    assert!(
        log.contains("runs-compiled scope=item.nest"),
        "the looping method did not compile on its first call:\n{log}"
    );
    assert!(
        !log.contains("runs-compiled scope=item.mk"),
        "a loop-free method called once compiled under the default tier:\n{log}"
    );
}
