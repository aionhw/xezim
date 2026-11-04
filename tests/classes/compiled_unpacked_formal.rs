//! §13.5: a formal with unpacked dimensions (`input bit [7:0] data[]`) is a
//! whole array. The compiled-method plan checked only its packed type, so
//! the method compiled with an 8-bit scalar register for `data` and every
//! `data[i]` read 0. Such methods now run interpreted. Expected T| lines
//! are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class p;
  bit [7:0] x, y;
  bit [3:0] hi, lo;
  function void f(input bit [7:0] data[]);
    bit [7:0] t;
    x = data[0];
    t = data[1];
    y = t;
    {hi, lo} = t;
  endfunction
endclass
module top;
  initial begin
    p c; bit [7:0] data[];
    c = new; data = '{8'h6a, 8'ha5};
    c.f(data);
    $display("T|%h %h %0d %0d", c.x, c.y, c.hi, c.lo);
  end
endmodule
"#;

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_unpacked_formal_{}_{}",
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
fn unpacked_formal_elements_read_their_values() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("T|")).collect();
        assert_eq!(lines, ["T|6a a5 10 5"], "XEZIM_COMPILE_METHODS={compile}");
    }
}
