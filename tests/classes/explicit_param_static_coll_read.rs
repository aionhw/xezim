//! An explicitly parameterized static-collection READ
//! (`config_db#(int)::m_rsc.exists(c)`, `config_db#(int)::m_rsc.num()`) must
//! resolve to the SAME per-specialization store as the matching WRITE
//! (`config_db#(int)::m_rsc[c] = new`). UVM's config-db uses `m_rsc[uvm_component]`
//! bare (inside its parameterized static methods); the explicit `Class#(spec)::`
//! spelling is the externally-visible counterpart a test author writes when
//! querying the pool directly. Pre-fix the write kept the `#(int)` (parsing to
//! a per-spec key) but the method-call receiver dropped it (parsing to a bare
//! `Ident`), so `exists()/num()` looked in a different store and reported
//! empty despite a live entry.
use std::process::Command;

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

fn run(src: &str) -> String {
    std::fs::write("/tmp/explicit_specread.sv", src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", "/tmp/explicit_specread.sv"])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const SRC: &str = r#"class comp;
endclass

class config_db #(type T = int);
  static int m_rsc[comp];
endclass

module top;
  initial begin
    comp c;
    c = new;
    // write a value under the `int` specialization
    config_db#(int)::m_rsc[c] = 42;
    // read that explicit specialization back
    if(config_db#(int)::m_rsc.exists(c))
      $display("TAG_PASS int-exists val=%0d", config_db#(int)::m_rsc[c]);
    else
      $display("TAG_FAIL int-not-exists");
    $display("RESULT int-num=%0d", config_db#(int)::m_rsc.num());
    // a DIFFERENT specialization must stay empty (per-spec isolation)
    $display("RESULT bit-num=%0d", config_db#(bit)::m_rsc.num());
    $finish;
  end
endmodule
"#;

#[test]
fn explicit_param_static_coll_read_matches_write() {
    let out = run(SRC);
    assert!(
        out.contains("TAG_PASS int-exists val=42"),
        "explicit `config_db#(int)::m_rsc[c]` write must be readable through \
         the matching explicit `exists()`/`[c]` read; expected \
         `TAG_PASS int-exists val=42`, got:\n{out}"
    );
    assert!(
        out.contains("RESULT int-num=1") && out.contains("RESULT bit-num=0"),
        "per-specialization isolation: int pool must have 1 entry and the bit \
         pool 0, got:\n{out}"
    );
    assert!(
        !out.contains("TAG_FAIL"),
        "unexpected explicit-prefix static-collection read failure:\n{out}"
    );
}