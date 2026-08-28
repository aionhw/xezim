//! §8.25 — a FORMAL typed with a class TYPE PARAMETER (`IMP imp` inside a
//! parameterized class) must be resolved to the concrete class the object is
//! specialized with, so `$cast(imp, src)` checks against the RIGHT type.
//!
//! Before this fix, the concrete-typed record for a same-named FORMAL/LOCAL
//! in an unrelated scope lingered in the flat `var_class_types` map (a local's
//! record is not scrubbed on frame pop the way a formal's is), so
//! `class_of_var("imp")` returned the STALE class from the other scope and the
//! current class's own `IMP imp` — whose declared type is the PARAMETER, not a
//! real class name — was never recorded. A `$cast(imp, parent)` then resolved
//! `imp` to the wrong class and failed even when `IMP` genuinely matched
//! `parent` (UVM's TLM nonblocking socket constructors hit this as
//! UVM/TLM2/NOIMP on a valid initiator socket).
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
    let path = format!("/tmp/param_formal_{tag}.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", &path])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A type-param-typed formal named `imp`, whose `$cast(imp, parent)` must
/// see the RESOLVED type parameter (`derived1`), not a stale same-named
/// concrete-typed local left behind by an unrelated function (`derived2`).
const STALE_IMPL: &str = r#"
module top;
  class basecls;
    bit tag;
    function new(); tag = 1; endfunction
  endclass
  class derived1 extends basecls;
    bit a;
  endclass
  class derived2 extends basecls;
    bit b;
  endclass
  // A function whose LOCAL `imp` has a concrete class type; its record is not
  // scrubbed on return (unlike a formal's), so it lingers in var_class_types.
  function void burn();
    derived2 imp;
  endfunction
  // A parameterized class whose `new` has a TYPE-PARAM-typed formal `imp`.
  class socket #(type IMP = basecls);
    bit cast_ok;
    function new(string name, basecls parent, IMP imp = null);
      if (imp == null) begin
        if ($cast(imp, parent)) cast_ok = 1; else cast_ok = 0;
      end
    endfunction
  endclass
  socket #(derived1) s;
  derived1 d1;
  initial begin
    burn();          // plants the stale var_class_types["imp"] = derived2
    d1 = new();
    s = new("s", d1);
    if (s.cast_ok == 1) $display("RESULT PASS");
    else                $display("RESULT FAIL");
  end
endmodule
"#;

#[test]
fn type_param_formal_cast_not_poisoned_by_same_named_concrete_local() {
    let out = run(STALE_IMPL, "stale");
    assert!(
        out.contains("RESULT PASS"),
        "a type-param-typed formal's $cast must beat a stale same-named \
         concrete-typed local\noutput:\n{out}"
    );
}