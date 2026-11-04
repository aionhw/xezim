//! IEEE 1800-2023 §8.4: a task or function call through a null handle
//! dereferences null. It ran nothing and reported nothing, so a test whose
//! handle was never constructed printed no trace at all (#532, reported by
//! akashlevy). At the top level the run aborts, like the reference
//! simulator; a null-guarded call stays silent; a static method needs no
//! object.

use xezim::simulate;

fn msgs(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn function_call_through_a_null_handle_is_reported_and_aborts() {
    let src = r#"
class C;
  function void hello(); $display("hello"); endfunction
endclass
module t;
  C c;
  initial begin
    c.hello();
    $display("post");
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    let m = msgs(&sim);
    assert!(sim.saw_fatal, "{m:?}");
    assert!(m.iter().any(|s| s.contains("null handle") && s.contains("`hello`")), "{m:?}");
    assert!(!m.iter().any(|s| s == "hello" || s == "post"), "{m:?}");
}

#[test]
fn task_and_value_calls_through_a_null_handle_are_reported() {
    for call in ["c.t();", "r = c.f();", "void'(c.f());"] {
        let src = format!(
            r#"
class C;
  function int f(); return 3; endfunction
  task t(); $display("task"); endtask
endclass
module t;
  C c; int r;
  initial begin
    {call}
    $display("post");
  end
endmodule
"#
        );
        let sim = simulate(&src, 10).expect("simulate");
        let m = msgs(&sim);
        assert!(sim.saw_fatal, "{call}: {m:?}");
        assert!(m.iter().any(|s| s.contains("null handle")), "{call}: {m:?}");
    }
}

#[test]
fn guarded_and_static_calls_stay_silent() {
    let src = r#"
class C;
  static int count = 0;
  static function int bump(); count++; return count; endfunction
  function void hello(); $display("hello"); endfunction
endclass
module t;
  C c;
  initial begin
    int v;
    if (c != null) c.hello();
    v = c.bump();
    $display("post %0d", v);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    let m = msgs(&sim);
    assert!(!sim.saw_fatal, "{m:?}");
    assert!(m.contains(&"post 1".to_string()), "{m:?}");
}
