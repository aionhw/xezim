//! `XEZIM_BIND_AUDIT` reports each name's binding (§23.9) beside what the
//! generate rename made of it, and leaves the compiled design as it was.

use std::process::Command;

const SRC: &str = "module top;
  if (1) begin : g
    function automatic int f(); return 5; endfunction
    initial $display(f());
    initial begin int f = 7; $display(f); end
    final $display(f());
  end
endmodule
";

fn compile(dir: &std::path::Path, out: &str, audit: bool) -> (Vec<u8>, Vec<String>) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_xezim"));
    c.current_dir(dir).args(["--compile", "-o", out, "t.sv"]);
    if audit {
        c.env("XEZIM_BIND_AUDIT", "1");
    } else {
        c.env_remove("XEZIM_BIND_AUDIT");
    }
    let run = c.output().expect("the simulator runs");
    assert!(run.status.success(), "{run:?}");
    let lines = String::from_utf8_lossy(&run.stderr)
        .lines()
        .chain(String::from_utf8_lossy(&run.stdout).lines())
        .filter_map(|l| l.strip_prefix("[bind-audit] ").map(str::to_string))
        .collect();
    (std::fs::read(dir.join(out)).expect("the artifact"), lines)
}

#[test]
fn the_audit_prints_each_binding_and_changes_no_artifact_byte() {
    let dir = std::env::temp_dir().join(format!("xezim_bind_audit_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temp dir");
    std::fs::write(dir.join("t.sv"), SRC).expect("the source");
    let (with, lines) = compile(&dir, "a.bin", true);
    let (without, none) = compile(&dir, "b.bin", false);
    std::fs::remove_dir_all(&dir).expect("removing the temp dir");
    assert_eq!(
        lines,
        [
            "t.sv:4 f -> Decl(top.g, Function) rename: yes(g.f) pos=process",
            "t.sv:5 f -> none rename: yes(g.f) pos=process why=Local disagree=A",
            "t.sv:6 f -> Decl(top.g, Function) rename: no pos=final disagree=B",
        ]
    );
    assert!(none.is_empty(), "{none:?}");
    assert!(with == without, "the audit changed the artifact");
}
