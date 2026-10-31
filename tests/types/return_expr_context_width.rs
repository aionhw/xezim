//! §13.4.1: `return e` evaluates `e` as if assigned to the function's
//! return variable, so `return a + b` with a `byte` a and an 8-bit `bit` b
//! in an `int` function is an unsigned 32-bit add (-3 + 3 = 256). The
//! interpreter evaluated it self-determined at 8 bits (0), while the
//! `f = a + b` spelling was right. Expected T| lines are the reference
//! simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class c;
  byte sb;
  bit [7:0] su;
  function int g2(); return sb + su; endfunction
  function int g3(); g3 = sb + su; endfunction
  function int g4(byte a, bit [7:0] b); return a + b; endfunction
  function int g5(); int t; t = sb + su; return t; endfunction
  function bit [3:0] g6(bit [3:0] a); return a + 4'd15; endfunction
  function int g7(byte a); return a; endfunction
endclass
function automatic int f1(byte a, bit [7:0] b); return a + b; endfunction
function automatic int f2(byte a, bit [7:0] b); f2 = a + b; endfunction
function automatic bit [8:0] f3(bit [7:0] a, bit [7:0] b); return a + b; endfunction
function automatic string f4(string a); return {a, "!"}; endfunction
module top;
  initial begin
    c o;
    o = new;
    o.sb = -3; o.su = 3;
    $display("T|cls=%0d %0d %0d %0d %0d %0d", o.g2(), o.g3(), o.g4(-3, 3), o.g5(), o.g6(3), o.g7(-3));
    $display("T|fn=%0d %0d %0d %s", f1(-3, 3), f2(-3, 3), f3(200, 100), f4("x"));
  end
endmodule
"#;

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_return_ctx_{}_{}",
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
fn return_expression_takes_the_return_width() {
    {
        let compile = "0";
        let log = run(compile);
        let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("T|")).collect();
        assert_eq!(
            lines,
            ["T|cls=256 256 256 256 2 -3", "T|fn=256 256 300 x!"],
            "XEZIM_COMPILE_METHODS={compile}"
        );
    }
}
