//! IEEE 1800-2023 §8.14: inside a class method, an unqualified property name
//! resolves to `this.<prop>` (and its base classes), taking precedence over
//! outer procedural locals or unrelated callers' local variables.
//!
//! REGRESSION: in `$cast(dest, src)`, `class_of_var` looked up `dest` in the
//! global `var_class_types` flat map before checking properties of `this`.
//! If a caller task declared a local variable with the same name as a callee's
//! member property (e.g. `top_seq seq;` in `configure_phase` vs `leaf_seq seq;`
//! in `mid_seq`), `$cast(seq, ...)` inside `mid_seq` resolved `seq`'s type to
//! the caller's `top_seq` instead of `this.seq`'s `leaf_seq`, causing `$cast`
//! to report an incompatible type error and fail.
use std::process::Command;

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

fn run(src: &str, tag: &str) -> String {
    let path = format!("/tmp/caller_local_cast_{tag}.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", &path])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const SV_SRC: &str = r#"module top;
  class base_c;
  endclass

  class leaf_c extends base_c;
    int val = 42;
  endclass

  class other_c extends base_c;
    int other = 99;
  endclass

  class mid_c;
    leaf_c item; // property named 'item'
    task automatic do_cast(base_c b);
      // item must resolve to this.item (leaf_c), NOT caller's 'other_c item'
      if ($cast(item, b)) begin
        $display("CAST_SUCCESS val=%0d", item.val);
      end else begin
        $display("CAST_FAILED");
      end
    endtask
  endclass

  task automatic caller_task();
    other_c item = new; // caller local named 'item' of conflicting type
    mid_c m = new;
    leaf_c l = new;
    m.do_cast(l);
  endtask

  initial begin
    caller_task();
    $finish;
  end
endmodule
"#;

#[test]
fn caller_local_does_not_shadow_this_property_in_cast() {
    let out = run(SV_SRC, "shadow_cast");
    assert!(
        out.contains("CAST_SUCCESS val=42"),
        "expected $cast to resolve to this.item (leaf_c) and succeed, got:\n{out}"
    );
    assert!(!out.contains("CAST_FAILED"), "cast failed unexpectedly");
}
