//! §11.6.1: an assignment's target width is the context of its right-hand
//! side, so `m = (1 << n) - 1` into a `bit [1023:0]` member shifts at 1024
//! bits. A compiled method evaluated the right-hand side of a store to a
//! BARE member self-determined, so the shift ran at 32 bits and the mask
//! lost its upper bits; `this.m = ...` and locals were already right.
//! Expected T| lines are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class cfg_c; int data_width = 64; endclass
class tr;
  cfg_c c;
  int w = 40;
  bit [1023:0] m1, m2, m3, m4, m5;
  function void f(int fw);
    bit [1023:0] l;
    m1 = ((1 << (c.data_width)) - 1);
    this.m2 = ((1 << c.data_width) - 1);
    l = ((1 << c.data_width) - 1);
    m3 = l;
    m4 = (1 << w) - 1;
    m5 = (1 << fw) - 1;
  endfunction
endclass
module top;
  initial begin
    tr t; t = new; t.c = new;
    t.f(36);
    $display("T|%h %h %h %h %h", t.m1[71:0], t.m2[71:0], t.m3[71:0], t.m4[71:0], t.m5[71:0]);
  end
endmodule
"#;

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_bare_member_ctx_{}_{}",
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
fn bare_member_store_takes_the_field_width() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("T|")).collect();
        assert_eq!(
            lines,
            ["T|00ffffffffffffffff 00ffffffffffffffff 00ffffffffffffffff 00000000ffffffffff 000000000fffffffff"],
            "XEZIM_COMPILE_METHODS={compile}"
        );
    }
}
