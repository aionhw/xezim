//! A statement fallback in a compiled method carries the class-handle
//! locals it names — a `ref` handle formal it assigns, a handle formal or
//! top-level handle local it reaches members and methods through — so the
//! method compiles instead of declining: a handle had no carry kind, and a
//! member access through a register-held name was refused. A non-virtual
//! call through a carried handle still binds to its declared class (§8.20).
//! Expected T| lines are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class item;
  int v;
  int q[$];
  function new(int x); v = x; endfunction
  function int tag(); return 1; endfunction
endclass
class sub_item extends item;
  function new(int x); super.new(x); endfunction
  function int tag(); return 2; endfunction
endclass
class store;
  item cur;
  item other;
  function bit pick(string name, ref item obj, output int bits);
    item h;
    pick = 1;
    obj = null;
    h = cur;
    case (name)
      "cur": begin obj = cur; bits = $countones(h.v); end
      "other": begin obj = other; bits = $countones(other.v) + h.q.sum(); end
      default: pick = 0;
    endcase
  endfunction
  function int peek(item h);
    int n;
    n = $countones(h.v) + h.tag();
    return n;
  endfunction
endclass
module top;
  initial begin
    automatic store s = new;
    automatic item got;
    automatic int b;
    automatic sub_item si = new(7);
    s.cur = new(5);
    s.other = si;
    s.cur.q.push_back(3);
    $display("T|pick cur %0d v=%0d bits=%0d", s.pick("cur", got, b), got.v, b);
    $display("T|pick other %0d v=%0d bits=%0d", s.pick("other", got, b), got.v, b);
    $display("T|pick none %0d null=%0d", s.pick("none", got, b), got == null);
    $display("T|peek %0d %0d", s.peek(s.cur), s.peek(si));
  end
endmodule
"#;

const EXPECTED: [&str; 4] = [
    "T|pick cur 1 v=5 bits=2",
    "T|pick other 1 v=7 bits=6",
    "T|pick none 0 null=1",
    "T|peek 3 4",
];

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_handle_carry_{}_{}",
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
fn fallbacks_carry_class_handle_locals() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("T|")).collect();
        assert_eq!(lines, EXPECTED, "XEZIM_COMPILE_METHODS={compile}");
    }
    let log = run("1");
    for m in ["store.pick", "store.peek"] {
        assert!(
            log.contains(&format!("runs-compiled scope={m}")),
            "{m} did not run compiled:\n{log}"
        );
    }
}
