//! §11.4.7 / §11.4.11: `&&` and `||` do not evaluate the right operand once
//! the left decides the result, and `?:` with a known condition evaluates
//! only the selected arm (an X condition evaluates both). Compiled methods
//! evaluated every operand, so `h != null && h.f()` called through a null
//! handle and a side-effecting call ran when it should not. Expected T|
//! lines are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class node;
  string name = "n";
  int t = 3;
  function string get_name(); return name; endfunction
  function int typ(); return t; endfunction
endclass
class g;
  int calls;
  function int bump(int r = 5); calls++; return r; endfunction
  function int logic_ops(node a, node b);
    logic_ops = 0;
    if (a != null && a.typ() == 3) logic_ops += 1;
    if (b == null || b.typ() == 3) logic_ops += 2;
    if (0 && bump()) logic_ops += 4;
    if (1 || bump()) logic_ops += 8;
    if (a == null && bump(0)) logic_ops += 16;
    if (a != null || bump(1)) logic_ops += 32;
  endfunction
  function string tern(node a, logic c);
    string s;
    int x;
    s = (a == null) ? "null" : a.get_name();
    x = (a == null) ? 1 : bump();
    x = x + ((a != null) ? bump() : 2);
    x = x + (c ? bump(7) : bump(7));
    return $sformatf("%s %0d", s, x);
  endfunction
endclass
module top;
  initial begin
    g x; node n;
    x = new; n = new;
    $display("T|%0d calls=%0d", x.logic_ops(null, null), x.calls);
    $display("T|%0d calls=%0d", x.logic_ops(n, n), x.calls);
    $display("T|%s calls=%0d", x.tern(null, 1), x.calls);
    $display("T|%s calls=%0d", x.tern(n, 0), x.calls);
    $display("T|%s calls=%0d", x.tern(n, 1'bx), x.calls);
  end
endmodule
"#;

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_short_circuit_{}_{}",
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
    assert!(
        !text.contains("null handle"),
        "a call ran through null:\n{text}"
    );
    text
}

#[test]
fn logical_and_conditional_operators_skip_the_unselected_operand() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("T|")).collect();
        assert_eq!(
            lines,
            [
                "T|42 calls=2",
                "T|43 calls=2",
                "T|null 10 calls=3",
                "T|n 17 calls=6",
                "T|n 17 calls=10",
            ],
            "XEZIM_COMPILE_METHODS={compile}"
        );
    }
}
