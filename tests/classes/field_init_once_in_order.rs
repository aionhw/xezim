//! §8.7: a call-bearing instance field initializer runs exactly ONCE per
//! construction, in declaration order. A fixed-point re-evaluation over the
//! unordered initializer map ran each one up to N+1 times, so an initializer
//! with side effects skipped ids. Reference-validated: 1/2 3/4 5/6.
use std::path::PathBuf;
use std::process::Command;

const DESIGN: &str = r#"
class counter;
  static int next = 0;
  static function int take(); next++; return next; endfunction
endclass
class item;
  int id = counter::take();
  int id2 = counter::take();
endclass
module top;
  initial begin
    item a = new, b = new, c = new;
    $display("IDS a=%0d/%0d b=%0d/%0d c=%0d/%0d next=%0d",
             a.id, a.id2, b.id, b.id2, c.id, c.id2, counter::next);
    $finish;
  end
endmodule
"#;

#[test]
fn call_bearing_field_initializers_run_once_in_declaration_order() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("field_init_once");
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
    assert!(
        text.contains("IDS a=1/2 b=3/4 c=5/6 next=6"),
        "initializers did not run once each in order:\n{text}"
    );
}
