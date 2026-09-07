//! A dimensioned vector type argument on a data declaration
//! (`P #(bit [7:0]) a;`) must specialize on the WHOLE type: rendering only
//! the keyword aliased it with `P #(bit)`. Reference: `$bits(T)` = 8 / 1 / 32.
use std::path::PathBuf;
use std::process::Command;

const DESIGN: &str = r#"
class P #(type T = int);
  function int width(); return $bits(T); endfunction
endclass
module top;
  P #(bit [7:0]) a;
  P #(bit) b;
  P #(int) c;
  initial begin
    a = new; b = new; c = new;
    $display("W a=%0d b=%0d c=%0d", a.width(), b.width(), c.width());
    $finish;
  end
endmodule
"#;

#[test]
fn dimensioned_vector_type_arg_keeps_its_own_specialization() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("vector_type_arg");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("top.sv");
    std::fs::write(&src, DESIGN).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "top", src.to_str().unwrap(), "--no-cache"])
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "run failed:\n{text}");
    assert!(text.contains("W a=8 b=1 c=32"), "wrong specialization widths:\n{text}");
}
