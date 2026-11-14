//! REGRESSION: `method_local_base` (the index into `local_stack` that bounds
//! `local_class_type_of` / `local_typedef_type_of` / `in_any_frame` to THIS
//! inlined method's own frames) was simulator-global and not part of
//! `ProcessContext`. When a process parked (`#5`) inside an inlined blocking
//! method, its `local_stack` was swapped into its `ProcessContext` while its
//! base stayed on the simulator, so whichever process ran next left a foreign
//! base index behind. On resume the parker bounded its one-frame stack with
//! that stale index, `in_any_frame` turned false, and a method-local `Right
//! obj` resolved to the module-scope net `Wrong obj` — constructing the wrong
//! class (tag 111 instead of 222).
use std::process::Command;

fn xezim() -> String {
    // Cargo names the binary it built for the integration tests.
    env!("CARGO_BIN_EXE_xezim").to_string()
}

fn run(src: &str, tag: &str) -> String {
    let path = format!("/tmp/blocking_method_base_{tag}.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "tb", &path])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const SV_SRC: &str = r#"module tb;
  class Wrong; int tag; function new(); tag = 111; endfunction endclass
  class Right; int tag; function new(); tag = 222; endfunction endclass

  class Parker;
    task blocking_wait();
      Right obj;          // method-local shadows the module-scope Wrong obj
      #5;                 // parks: base pushed, context swapped away
      obj = new();
      $display("PARKER tag=%0d (expect 222)", obj.tag);
    endtask
  endclass

  class Deep; task go(); #20; endtask endclass

  Wrong obj;              // module-scope net of a DIFFERENT class
  Parker p = new;
  Deep   d = new;

  task automatic lvl2(); d.go(); endtask   // entered with 2 caller frames below
  task automatic lvl1(); lvl2(); endtask

  initial p.blocking_wait();
  initial lvl1();
  initial #40 $finish;
endmodule
"#;

#[test]
fn method_local_base_follows_process_through_context_swap() {
    let out = run(SV_SRC, "mlb");
    assert!(
        out.contains("PARKER tag=222"),
        "method-local `Right obj` must resolve ahead of the module-scope `Wrong obj` even after the process parks and resumes (base must travel with ProcessContext); got:\n{out}"
    );
    assert!(
        !out.contains("PARKER tag=111"),
        "resolved to module-scope Wrong obj; got:\n{out}"
    );
}
