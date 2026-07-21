//! IEEE 1800-2023 §8.25, §7.5: three interacting bugs that broke UVM's
//! resource pool (`uvm_resource_db`), fixed together:
//!
//! 1. **Queue mutation on a typedef'd parameterized class member** —
//!    `sh.value.push_back(x)` where `sh` is typed `table_q_t = uvm_shared#(T[$])`
//!    silently no-oped because (a) the parser flattens `sh.value.push_back(x)`
//!    into a 3-segment hierarchical `Ident`, which the 3-segment method dispatch
//!    only handled for QUERY builtins (size/num/exists), not MUTATION builtins
//!    (push_back/pop_back/insert/delete); and (b) the instance's type_bindings
//!    were empty because a bare `new()` on a typedef'd local didn't resolve the
//!    typedef chain to recover the `#(...)` type args.
//!
//! 2. **`foreach` key extraction on assoc-of-collections** — iterating
//!    `foreach (all[k])` where each `all[k]` is itself a queue extracted
//!    `5][0` instead of `5` (it used `k[k.len()-1]` to find the closing bracket,
//!    not the FIRST `]`), producing phantom iteration keys.
//!
//! 3. **`break` propagation from foreach to function body** — a `break`
//!    inside `foreach(svq[i])` inside a `function` set `break_flag`, which the
//!    foreach did NOT clear on exit. The function body loop checks `break_flag
//!    || return_flag` and exited early, so code after the foreach (including
//!    `return rsrc;`) was never reached.
use std::process::Command;

fn xezim() -> String {
    // Resolve the sibling CLI binary from the test binary's own location so
    // this works for both debug and release profiles.
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

fn run(src: &str, tag: &str) -> String {
    let path = format!("/tmp/respool_{tag}.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", &path])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn queue_push_back_on_typedef_param_member() {
    // Bug 1a+1b: `sh.value.push_back(x)` where `sh` is `typedef uvm_shared#(iq_t)`.
    let src = r#"typedef int iq_t[$];
class uvm_shared #(type T=int);
  T value;
endclass
typedef uvm_shared#(iq_t) wrapper_t;
module top;
  initial begin
    wrapper_t sh;
    sh = new();
    sh.value.push_back(42);
    sh.value.push_back(99);
    if (sh.value.size() == 2 && sh.value[0] == 42 && sh.value[1] == 99)
      $display("RESULT PASS");
    else
      $display("RESULT FAIL size=%0d", sh.value.size());
  end
endmodule
"#;
    let out = run(src, "typedefq");
    assert!(out.contains("RESULT PASS"), "expected push_back on typedef'd member\n{out}");
}

#[test]
fn foreach_over_assoc_of_queues() {
    // Bug 2: foreach over an assoc array whose values are themselves queues.
    let src = r#"typedef int iq_t[$];
class Base; int v; function new(int p=0); v=p; endfunction endclass
typedef Base bq_t[$];
module top;
  initial begin
    bq_t all[int];
    Base r;
    r = new(5);
    all[5].push_back(r);
    all[3].push_back(r);
    // The key extraction bug would iterate key "5][0" instead of "5".
    begin
      int count = 0;
      foreach (all[k]) begin
        count++;
        if (all[k].size() != 1) begin
          $display("RESULT FAIL key=%0d size=%0d", k, all[k].size());
          $finish;
        end
      end
      if (count == 2)
        $display("RESULT PASS");
      else
        $display("RESULT FAIL count=%0d", count);
    end
  end
endmodule
"#;
    let out = run(src, "assocofq");
    assert!(out.contains("RESULT PASS"), "expected foreach over assoc-of-queues\n{out}");
}

#[test]
fn break_in_foreach_does_not_exit_function() {
    // Bug 3: `break` inside foreach must NOT propagate to the function body.
    let src = r#"module top;
  function automatic int find_first();
    int arr[3];
    arr[0] = 10; arr[1] = 20; arr[2] = 30;
    foreach (arr[i]) begin
      if (arr[i] == 20) break;
    end
    // This code after the foreach was skipped when break_flag propagated.
    return 42;
  endfunction
  initial begin
    int r;
    r = find_first();
    if (r == 42) $display("RESULT PASS");
    else $display("RESULT FAIL r=%0d", r);
  end
endmodule
"#;
    let out = run(src, "breakprop");
    assert!(out.contains("RESULT PASS"), "expected break not to exit function\n{out}");
}
