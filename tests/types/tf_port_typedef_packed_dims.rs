//! §13.3: a task/function formal declared as a user-defined type followed by
//! packed dimensions — `ref u7_t [R-1:0][C-1:0] x`, `input u7_t [1:0] y`. The
//! port parser's lookahead skipped one bracket group and then expected the
//! port name, so a second packed dimension after a typedef was a parse error
//! (the direction-less function form with one dimension already parsed).
use std::path::PathBuf;
use std::process::Command;

const DESIGN: &str = r#"
typedef bit [6:0] u7_t;
module top;
  task automatic rd2(ref u7_t [1:0][1:0] a);
    $display("RD2 %0d", a[1][0]);
  endtask
  task automatic in2(input u7_t [1:0][1:0] a, input u7_t [1:0] b);
    $display("IN2 %0d %0d", a[1][0], b[1]);
  endtask
  function automatic u7_t fn2(u7_t [1:0][1:0] a);
    return a[0][1];
  endfunction
  u7_t [1:0][1:0] v;
  u7_t [1:0] w;
  initial begin
    v[1][0] = 5; v[0][1] = 6; w[1] = 7;
    rd2(v);
    in2(v, w);
    $display("FN2 %0d", fn2(v));
  end
endmodule
"#;

#[test]
fn typedef_with_several_packed_dimensions_in_tf_ports() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("tf_port_typedef_dims");
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join("t.sv");
    std::fs::write(&sv, DESIGN).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "top", "--no-cache", sv.to_str().unwrap()])
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "run failed:\n{text}");
    for want in ["RD2 5", "IN2 5 7", "FN2 6"] {
        assert!(text.contains(want), "missing `{want}`:\n{text}");
    }
}
