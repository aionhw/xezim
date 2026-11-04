//! §7.4 / §11.8.1: an element of a class member collection keeps its
//! declared type, so `d[0] < -16` over `int d[]` is a SIGNED compare. The
//! compiled method treated every `x[i]` whose base is not a module-scope
//! array as an unsigned bit-select, so `3 < -16` was true and `d[0] / -2`
//! divided unsigned. In an unsigned expression (`b[0] + u[0]`, a `byte`
//! and an 8-bit `bit` element) the signed element must be converted before
//! it is widened; the element read sign-extended it to the context first.
//! Expected T| lines are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class c;
  int d[];
  int q[$];
  bit [7:0] u[];
  byte b[$];
  int unsigned w[$];
  int aa[string];
  int s;
  function bit f1(); return d[0] < -16; endfunction
  function bit f2(); return (d[0] < -16) || (d[0] > 16); endfunction
  function bit f3(); return s < -16; endfunction
  function bit f4(); return q[0] < -16; endfunction
  function bit f5(); int k = 0; return d[k] < -16; endfunction
  function bit f6(); return u[0] < -16; endfunction
  function bit f7(); return b[0] < -16; endfunction
  function bit f8(); return w[0] < -16; endfunction
  function bit f9(); return aa["x"] < -16; endfunction
  function int g1(); return b[0] + u[0]; endfunction
  function int g5(); int t; t = b[0] + u[0]; return t; endfunction
  function bit g6(); return b[0] < u[0]; endfunction
  function int g2(); return b[0] >>> 1; endfunction
  function int g3(); return d[0] / -2; endfunction
  function bit g4(); return b[0] < u[0]; endfunction
endclass
module top;
  initial begin
    c o;
    o = new;
    o.d = new[1]; o.d[0] = 3; o.s = 3; o.q.push_back(3);
    o.u = new[1]; o.u[0] = 3; o.b.push_back(-3); o.w.push_back(3); o.aa["x"] = -20;
    $display("T|%0d %0d %0d %0d %0d %0d %0d %0d %0d", o.f1(), o.f2(), o.f3(), o.f4(), o.f5(), o.f6(), o.f7(), o.f8(), o.f9());
    $display("T|%0d %0d %0d %0d %0d %0d", o.g1(), o.g2(), o.g3(), o.g4(), o.g5(), o.g6());
  end
endmodule
"#;

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_coll_elem_signed_{}_{}",
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
fn member_collection_elements_compare_with_their_declared_sign() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("T|")).collect();
        assert_eq!(
            lines,
            ["T|0 0 0 0 0 1 0 1 1", "T|256 -2 -1 0 256 0"],
            "XEZIM_COMPILE_METHODS={compile}"
        );
    }
}
