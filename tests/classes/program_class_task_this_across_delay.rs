//! LRM §24 / §4.4 regression: a CLASS TASK called from a `program` block must
//! keep its `this`/local context across a suspension (a blocking `#delay` in
//! its body). `program`/`interface` initials run in the Reactive region
//! (§4.4). When such an initial calls `c.run()` and `run()` contains a
//! `#delay` (or `fork..join`, `wait`, ...), xezim used to run the program
//! initial through a bare `run_process_stmts` that never PARKED the process's
//! `this`/`local_stack` when it suspended. On the next Reactive-drain pass the
//! resumed method body saw an empty `this_stack`, so every member read — and
//! every `this.tag` — came back through a null `this`: `flag` read `x`,
//! `this.tag` read `0` instead of the object's actual value.
//!
//! This is exactly the shape that broke UVM's `run_test()`: its auto-`$finish`
//! check read `finish_on_completion` off a garbage `this` (raw `x`), skipped
//! `$finish`, and a free-running clock kept the simulation alive forever
//! (70regs/90Mantis/3540, both `10implicit` and `20explicit`, hung). The
//! reference simulator cleanly `$finish`es at the end of the phase cycle; a
//! program-initial class task with a suspension in its body is the trigger.
use std::process::Command;

fn xezim() -> String {
    // Cargo names the binary it built for the integration tests.
    env!("CARGO_BIN_EXE_xezim").to_string()
}

fn run(src: &str, top: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "xezim_prog_class_task_{top}_{}.sv",
        std::process::id()
    ));
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", top])
        .arg(&path)
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_file(&path);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A class task suspends on a `#delay` inside a task called from a `program`
/// initial; the `this`-member read after the delay must see the object.
const SRC: &str = r#"
class C;
   bit   flag = 1;
   int   tag  = 7;
   task run();
      #5;                       // suspends the process across a Reactive pass
      if (flag === 1'b1 && tag === 7)
        $display("FT_PASS flag=%0d tag=%0d", flag, tag);
      else
        $display("FT_FAIL flag=%0d tag=%0d", flag, tag);
   endtask
endclass

program tb;
   initial begin
      C c = new;
      c.run();
      $finish;
   end
endprogram
"#;

#[test]
fn program_class_task_keeps_this_across_delay() {
    let out = run(SRC, "tb");
    println!("{out}");
    assert!(
        out.contains("FT_PASS flag=1 tag=7"),
        "expected FT_PASS (this preserved across the #delay); got:\n{out}"
    );
    assert!(
        !out.contains("FT_FAIL"),
        "this was lost across the #delay; members read through null this:\n{out}"
    );
}