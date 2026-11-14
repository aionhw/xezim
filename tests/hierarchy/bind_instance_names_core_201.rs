//! §23.11, §23.8: a bound instance is instantiated in its target, so the
//! target, and a scope below it by an upward name, can start a hierarchical
//! name with it (`a.y`). The undeclared-name checks ran before
//! compilation-unit binds were applied and rejected `a`
//! (aionhw/xezim-core#201, reported by chuanseng-ng). A top the bind does not
//! target still cannot name it. Expected values are the reference
//! simulator's; the instance-list case follows the LRM (see
//! `bind_instance_lists_core_200`).

use std::path::PathBuf;
use std::process::Command;
use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

const CHILD: &str = "module c #(parameter int A = 0) (output int y);
  assign y = A;
endmodule
";

#[test]
fn target_is_the_top_or_an_instance() {
    let t = "module t;
  initial begin #1 $display(\"a=%0d\", a.y); $finish; end
endmodule
";
    let bind = "bind t c #(3) a();\n";
    assert_eq!(lines(&format!("{CHILD}{t}{bind}")), ["a=3"]);
    let top = "module top; t u (); endmodule\n";
    assert_eq!(lines(&format!("{CHILD}{t}{top}{bind}")), ["a=3"]);
}

#[test]
fn continuous_assign_with_an_instance_list() {
    let src = format!(
        "{CHILD}module t;
  int r;
  assign r = a.y + b.y;
  initial begin #1 $display(\"r=%0d a=%0d b=%0d\", r, a.y, b.y); $finish; end
endmodule
module top; t u (); endmodule
bind t c #(3) a(), b();
"
    );
    assert_eq!(lines(&src), ["r=6 a=3 b=3"]);
}

#[test]
fn upward_name_from_a_child_of_the_target() {
    let src = format!(
        "{CHILD}module k;
  initial #1 $display(\"k sees a.y=%0d\", a.y);
endmodule
module t; k kk (); endmodule
module top; t u (); initial #2 $finish; endmodule
bind t c #(3) a();
"
    );
    assert_eq!(lines(&src), ["k sees a.y=3"]);
}

#[test]
fn subroutines_and_events_of_the_bound_instance() {
    let src = "module c #(parameter int A = 0) ();
  function int get(); return A + 1; endfunction
  task show(); $display(\"%m show A=%0d\", A); endtask
endmodule
module t;
  initial begin #1 $display(\"get=%0d\", a.get()); a.show(); $finish; end
endmodule
module top; t u (); endmodule
bind t c #(3) a();
";
    assert_eq!(lines(src), ["get=4", "top.u.a.show show A=3"]);
    let src = "module c (input logic clk, output int y);
  int cnt = 0;
  always @(posedge clk) cnt <= cnt + 1;
  assign y = cnt;
endmodule
module t;
  logic clk = 0;
  int seen;
  always #5 clk = ~clk;
  always_comb seen = a.y;
  always @(a.y) $display(\"%0t a.y=%0d seen=%0d\", $time, a.y, seen);
  initial #22 $finish;
endmodule
bind t c a(.clk(clk), .y());
";
    assert_eq!(lines(src), ["5 a.y=1 seen=1", "15 a.y=2 seen=2"]);
}

#[test]
fn names_no_bind_adds_stay_undeclared() {
    let src = format!(
        "{CHILD}module t;
  initial begin #1 $display(\"zz=%0d\", zz.y); $finish; end
endmodule
bind t c #(3) a();
"
    );
    let err = simulate(&src, 10).err().expect("zz must be rejected");
    assert!(err.contains("Undeclared identifier 'zz'"), "{err}");
}

/// The bind targets `t`; the top `other` is elaborated alone (`-s other`).
#[test]
fn another_top_cannot_name_the_bound_instance() {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("bind_instance_names_core_201");
    std::fs::create_dir_all(&d).unwrap();
    let path = d.join("other.sv");
    let src = format!(
        "{CHILD}module t; endmodule
module other;
  initial #1 $display(\"other sees a.y=%0d\", a.y);
endmodule
bind t c #(3) a();
"
    );
    std::fs::write(&path, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--no-cache", "-s", "other"])
        .arg(&path)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.status.success(), "accepted:\n{text}");
    assert!(text.contains("Undeclared identifier 'a'"), "{text}");
    assert!(
        !text.lines().any(|l| l.starts_with("other sees")),
        "simulated:\n{text}"
    );
}
