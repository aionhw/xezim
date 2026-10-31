//! §11.8.1: an expression is unsigned when any operand is, and a signed
//! operand is then zero-extended to the context. A compiled method did not
//! know the declared signedness of a member operand (`sb + su`, `p.sb +
//! p.su` with a `byte` and an 8-bit `bit`), so the runtime sign-extended
//! the byte and -3 + 3 in a 32-bit context came out 0 instead of 256.
//! Expected T| lines are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class pair;
  byte sb;
  bit [7:0] su;
  shortint ss;
  int unsigned wu;
endclass
class c;
  byte sb;
  bit [7:0] su;
  shortint ss;
  int unsigned wu;
  pair p;
  function int g1(); int t; t = sb + su; return t; endfunction
  function int g2(); g2 = sb + su; endfunction
  function int g3(); int t; t = p.sb + p.su; return t; endfunction
  function bit g4(); return sb < su; endfunction
  function bit g5(); return p.sb < -1; endfunction
  function bit g6(); return ss < wu; endfunction
  function int g7(); int t; t = (sb + ss) >>> 1; return t; endfunction
  function int g8(); int t; t = sb / -1; return t; endfunction
endclass
module top;
  initial begin
    c o;
    o = new;
    o.p = new;
    o.sb = -3; o.su = 3; o.ss = -5; o.wu = 7;
    o.p.sb = -3; o.p.su = 3;
    $display("T|%0d %0d %0d %0d %0d %0d %0d %0d", o.g1(), o.g2(), o.g3(), o.g4(), o.g5(), o.g6(), o.g7(), o.g8());
  end
endmodule
"#;

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_member_operand_sign_{}_{}",
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
fn member_operands_carry_their_declared_sign() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("T|")).collect();
        assert_eq!(
            lines,
            ["T|256 256 256 0 1 0 -4 3"],
            "XEZIM_COMPILE_METHODS={compile}"
        );
    }
}
