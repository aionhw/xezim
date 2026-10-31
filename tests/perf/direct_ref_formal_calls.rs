//! §13.5.2: a compiled method calling a compiled callee with `ref` or
//! `inout` scalar formals — a 1024-bit vector, a class handle, an int —
//! dispatches directly, copying each formal's final value back into the
//! caller's argument slot, instead of writing every argument out as literal
//! text for the interpreter's binding. An `output` formal keeps that route.
//! Expected T| lines are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class node; int v; function new(int x); v = x; endfunction endclass
class lookup;
  node a, b;
  int hits;
  function bit get(string name, ref bit [1023:0] val, input int ix, ref node obj);
    val = 0;
    obj = null;
    get = 1;
    case (name)
      "a": begin val = a.v + ix; obj = a; end
      "b": begin val = {512'h1, 512'(b.v)}; obj = b; end
      default: get = 0;
    endcase
  endfunction
  function void bump(inout int x, input int by); x = x + by; hits++; endfunction
  function void fill(output int x); x = 42; endfunction
  function int run(int n);
    bit [1023:0] v;
    node o;
    int acc = 0, k = 1, z;
    for (int i = 0; i < n; i++) begin
      if (get("a", v, i, o)) acc += v[31:0] + o.v;
      if (get("b", v, 0, o)) acc += v[543:512] + v[31:0];
      if (!get("c", v, 0, o)) acc += (o == null) + v[31:0];
      bump(k, 2);
      fill(z);
      acc += z;
    end
    return acc + k + hits;
  endfunction
endclass
module top;
  initial begin
    automatic lookup l = new;
    l.a = new(5); l.b = new(9);
    $display("T|run=%0d", l.run(100));
  end
endmodule
"#;

#[test]
fn ref_and_inout_formals_dispatch_directly() {
    let dir = std::env::temp_dir().join(format!("xezim_ref_direct_{}", std::process::id()));
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
    assert_eq!(lines, ["T|run=11551"]);
    let direct: u64 = text
        .lines()
        .find_map(|l| l.strip_prefix("[PROF] vm_direct_calls="))
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or_else(|| panic!("missing vm_direct_calls:\n{text}"));
    // 300 `get` and 100 `bump` calls go direct; the `output` `fill` does not.
    assert!((390..=400).contains(&direct), "vm_direct_calls={direct}");
}
