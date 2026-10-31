//! A store through a class handle `s.tag = v` is routed by whether the
//! name `s` denotes a class object. The check read the flat, process-wide
//! table of declared typedef names, which kept an earlier activation's
//! `sev_e s;` after that method returned, so every later `s.f = v` through
//! an unrelated handle named `s` was dropped. The check now reads the
//! active frames' declarations. Expected T| lines are the reference
//! simulator's.
use std::process::Command;

const DESIGN: &str = r#"
typedef enum { INFO, WARNING, ERROR } sev_e;
class reporter;
  int n;
  function void report(int k);
    sev_e s;
    s = sev_e'(k);
    if (s == ERROR) n++;
  endfunction
endclass
class slave;
  string tag;
  int id;
endclass
class test_c;
  slave s;
  reporter r;
  function void build();
    r = new;
    r.report(2);
    s = new;
    s.tag = "req_rsp";
    s.id = 5;
    $display("T|[%s] %0d %0d", s.tag, s.id, r.n);
  endfunction
endclass
module top;
  initial begin
    test_c t; t = new;
    t.build();
    $display("T|[%s] %0d", t.s.tag, t.s.id);
  end
endmodule
"#;

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_stale_enum_name_{}_{}",
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
fn member_store_through_a_handle_named_like_an_old_enum_local() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("T|")).collect();
        assert_eq!(
            lines,
            ["T|[req_rsp] 5 1", "T|[req_rsp] 5"],
            "XEZIM_COMPILE_METHODS={compile}"
        );
    }
}
