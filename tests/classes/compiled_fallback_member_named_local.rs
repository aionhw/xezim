//! A statement fallback in a compiled method is refused only when it uses
//! a register-held local by path; a member that merely shares a local's name
//! (`this.x = $countones(x)` with a formal `x`, `peer.x` with a formal
//! `peer`'s member) is not that local. Every segment of a dotted name, and
//! every member name, used to count as a use, so such a method never
//! compiled. Expected T| lines are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class acc;
  int x, n;
  string tag;
  acc peer;
  function void set(int x, string tag);
    this.x = $countones(x) + x;
    this.tag = {tag, $sformatf("%0d", $countones(x))};
    n++;
  endfunction
  function int mirror(acc peer);
    this.peer = peer;
    peer.x = $countones(this.x) + peer.n;
    return peer.x + this.peer.n;
  endfunction
endclass
module top;
  initial begin
    automatic acc a = new, b = new;
    a.set(7, "t"); b.set(3, "u");
    $display("T|%0d %s %0d %s %0d %0d", a.x, a.tag, b.x, b.tag, a.mirror(b), b.x);
  end
endmodule
"#;

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_fallback_member_named_local_{}_{}",
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
fn member_sharing_a_local_name_does_not_block_fallback() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("T|")).collect();
        assert_eq!(
            lines,
            ["T|10 t3 5 u2 4 3"],
            "XEZIM_COMPILE_METHODS={compile}"
        );
    }
    let log = run("1");
    for m in ["acc.set", "acc.mirror"] {
        assert!(
            log.contains(&format!("runs-compiled scope={m}")),
            "{m} did not run compiled:\n{log}"
        );
    }
}
