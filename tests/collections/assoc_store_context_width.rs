//! §11.6.1 / §7.8: an associative-array element is the context of the
//! value stored into it, so `aa[k] = (1 << 70) - 1` into `bit [99:0]
//! aa[string]` keeps 70 ones. The store evaluated the value self-determined
//! (32 bits) for module, local and class-member arrays alike. Expected T|
//! lines are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class c;
  bit [99:0] aa[string];
  bit [99:0] ai[int];
  function void f();
    bit [99:0] la[string];
    aa["k"] = (1 << 70) - 1;
    ai[3] = (1 << 66) - 1;
    la["k"] = (1 << 68) - 1;
    this.aa["t"] = (1 << 65) - 1;
    $display("T|local=%h", la["k"]);
  endfunction
endclass
module top;
  bit [99:0] ma[string];
  initial begin
    c o; o = new;
    ma["k"] = (1 << 69) - 1;
    o.f();
    o.aa["m"] = (1 << 67) - 1;
    $display("T|%h %h %h %h %h", ma["k"], o.aa["k"], o.ai[3], o.aa["t"], o.aa["m"]);
  end
endmodule
"#;

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_assoc_store_ctx_{}_{}",
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
fn associative_element_store_takes_the_element_width() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("T|")).collect();
        assert_eq!(
            lines,
            ["T|local=00000000fffffffffffffffff", "T|00000001fffffffffffffffff 00000003fffffffffffffffff 000000003ffffffffffffffff 000000001ffffffffffffffff 000000007ffffffffffffffff"],
            "XEZIM_COMPILE_METHODS={compile}"
        );
    }
}
