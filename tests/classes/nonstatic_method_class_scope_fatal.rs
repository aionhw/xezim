//! IEEE 1800-2023 §8.10: a class-scope `::` access names a STATIC class
//! member. Calling a NON-static method through `Class::method(...)` (no
//! instance receiver) is an error — reference simulators reject it at
//! compile time with "Illegal to access non-static method 'm' outside its
//! class scope". xezim previously SILENTLY EXECUTED the method body (the
//! flat `Class::method` call shape dispatched with a null handle), corrupt
//! behavior no legal program can rely on. Now every dispatch route
//! (interpreter `exec_static_method` and the compiled fast-path admission
//! in `vm_try_direct_static_call`) funnels the illegal form to a single
//! fatal diagnostic, emitted once per (class, method) pair.
//!
//! Legal shapes pinned unchanged: static methods through `::`, `new`
//! through `::`, instance calls on a handle, and static-queue reads
//! through a typedef alias (the staticq regression family).
use std::process::Command;

fn xezim() -> String {
    // Cargo names the binary it built for the integration tests.
    env!("CARGO_BIN_EXE_xezim").to_string()
}

fn run(src: &str) -> String {
    // The binary rewrites this path, and tests may run concurrently — give
    // each invocation a unique file.
    let path = format!(
        "/tmp/nonstatic_scope_fatal_{}.sv",
        std::process::id() as u64 * 1_000_003 + src.len() as u64 + src.as_ptr() as u64
    );
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", &path])
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_file(&path);
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    s
}

const SRC: &str = r#"class helper;
  static int all[$];
  function void bump(int v);
    all.push_back(v);
    $display("T|inside %0d", v);
  endfunction
  static function void poke(int v);
    all.push_back(v);
    $display("T|static %0d", v);
  endfunction
endclass
module top;
  initial begin
    helper::bump(7);
    $display("T|done %0d", helper::all.size());
  end
endmodule
"#;

#[test]
fn nonstatic_method_through_class_scope_is_fatal() {
    let out = run(SRC);
    assert!(
        out.contains("Illegal to access non-static method 'bump' outside its class scope"),
        "expected the §8.10 fatal, got:\n{}",
        out
    );
    // The method body must NOT have executed.
    assert!(
        !out.contains("T|inside"),
        "illegal call must not execute the body, got:\n{}",
        out
    );
    // Fatal terminates the simulation before the display.
    assert!(
        !out.contains("T|done"),
        "simulation must stop at the fatal, got:\n{}",
        out
    );
}

#[test]
fn static_method_through_class_scope_still_works() {
    let out = run(&SRC.replace("helper::bump(7);", "helper::poke(7);"));
    assert!(
        out.contains("T|static 7") && out.contains("T|done 1"),
        "static method through :: must keep working, got:\n{}",
        out
    );
    assert!(!out.contains("Fatal"), "unexpected fatal:\n{}", out);
}

#[test]
fn illegal_call_reported_once_not_per_iteration() {
    // Two illegal calls: the diagnostic fires once and the first call is
    // already fatal, so exactly one message and no body execution.
    let out = run(&SRC.replace("helper::bump(7);", "helper::bump(7); helper::bump(8);"));
    assert_eq!(
        out.matches("Illegal to access non-static method").count(),
        1,
        "diagnostic must fire exactly once, got:\n{}",
        out
    );
    assert!(!out.contains("T|inside"), "body must not run:\n{}", out);
}

#[test]
fn instance_call_on_handle_unaffected() {
    // The legal instance-call form on the SAME non-static method must not
    // trip the §8.10 check.
    let out = run(&SRC.replace(
        "helper::bump(7);",
        "helper h = new(); h.bump(7); $display(\"T|done %0d\", helper::all.size());",
    ));
    assert!(
        out.contains("T|inside 7") && out.contains("T|done 1"),
        "instance call must keep working, got:\n{}",
        out
    );
    assert!(!out.contains("Fatal"), "unexpected fatal:\n{}", out);
}
