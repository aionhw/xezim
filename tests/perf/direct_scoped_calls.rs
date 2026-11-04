//! A call a compiled method body makes to a method bound at compile time —
//! a bare call of its own class's method (§8.20) or `super.m()` (§8.15) —
//! dispatches directly to a compiled callee, seeding its registers from the
//! caller's, like a call through a handle does. Every such call went through
//! the interpreter's argument binding instead. Expected T| lines are the
//! reference simulator's; `vm_direct_calls` counts the direct dispatches.
use std::process::Command;

const DESIGN: &str = r#"
class base;
  int calls;
  function int step(int a); return a + 1; endfunction
  virtual function int vstep(int a); return a + 10; endfunction
  function int nv(int a); return a + 100; endfunction
  function int run(int n);
    int s = 0;
    for (int i = 0; i < n; i++) begin
      s = step(s);
      s = vstep(s);
      s = nv(s);
    end
    return s;
  endfunction
endclass
class derived extends base;
  virtual function int vstep(int a); return super.vstep(a) + 1000; endfunction
  function int nv(int a); return a + 5; endfunction
  function int run2(int n);
    int s = 0;
    for (int i = 0; i < n; i++) begin
      s = nv(s);
      s = super.nv(s);
      s = super.step(s);
    end
    return s;
  endfunction
endclass
module top;
  initial begin
    automatic base b = new;
    automatic derived d = new;
    automatic base bd = d;
    $display("T|base=%0d", b.run(200));
    $display("T|derived-via-base=%0d", bd.run(200));
    $display("T|derived=%0d", d.run2(200));
  end
endmodule
"#;

const EXPECTED: [&str; 3] = [
    "T|base=22200",
    "T|derived-via-base=222200",
    "T|derived=21200",
];

#[test]
fn scoped_calls_between_compiled_methods_dispatch_directly() {
    let dir = std::env::temp_dir().join(format!("xezim_direct_scoped_{}", std::process::id()));
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
        .env("XEZIM_PROFILE_TIMING", "1")
        .env("XEZIM_COMPILE_METHODS", "1")
        .env("XEZIM_METHOD_TIER", "0")
        .env_remove("XEZIM_METHOD_CACHE")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    let lines: Vec<&str> = text.lines().filter(|l| l.starts_with("T|")).collect();
    assert_eq!(lines, EXPECTED);
    let direct: u64 = text
        .lines()
        .find_map(|l| l.strip_prefix("[PROF] vm_direct_calls="))
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or_else(|| panic!("missing vm_direct_calls:\n{text}"));
    // 1800 bare and `super.` calls; through the interpreter's binding, 0.
    assert!(direct >= 1500, "vm_direct_calls={direct}");
}
