//! The per-method locals base (`method_local_base`) is PER-PROCESS state.
//!
//! It indexes into `local_stack`, which is swapped out with the rest of a
//! process's context when that process parks. Once the base is pushed on the
//! inlined BLOCKING method path (where suspending is the norm), a parked
//! process that left its base behind bounded ANOTHER process's frame-local
//! lookups by an index into a `local_stack` it does not own: with the two
//! processes at different call depths the slice came out empty, the
//! method-local `obj` lost to the same-named module-scope net, and
//! `obj = new()` constructed the wrong class.
//!
//! Reference-validated: PARKER tag=222 (the local, class Right), not 111.
use std::path::PathBuf;
use std::process::Command;

const DESIGN: &str = r#"
class Wrong; int tag; function new(); tag = 111; endfunction endclass
class Right; int tag; function new(); tag = 222; endfunction endclass

class Parker;
  task blocking_wait();
    Right obj;              // method-local shadows the module-scope Wrong obj
    #5;                     // parks here: base pushed, context swapped out
    obj = new();
    $display("PARKER tag=%0d", obj.tag);
  endtask
endclass

class Deep;
  task go();
    #20;                    // still parked when the parker resumes
    $display("DEEP done");
  endtask
endclass

module tb;
  Wrong obj;                // module-scope net of a DIFFERENT class
  Parker p = new;
  Deep d = new;
  task automatic lvl2(); d.go(); endtask   // entered with 2 caller frames below
  task automatic lvl1(); lvl2(); endtask
  initial p.blocking_wait();
  initial lvl1();
  initial #40 $finish;
endmodule
"#;

#[test]
fn a_parked_methods_locals_base_does_not_bound_another_process() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("method_local_base_per_process");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("tb.sv");
    std::fs::write(&src, DESIGN).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "tb", src.to_str().unwrap(), "--no-cache"])
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "run failed:\n{text}");
    assert!(
        text.contains("PARKER tag=222"),
        "a parked process's method-locals base leaked into another process:\n{text}"
    );
    assert!(text.contains("DEEP done"), "the deep process never finished:\n{text}");
}
