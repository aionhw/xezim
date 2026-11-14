//! IEEE 1800-2023 §8.4: only an access to a non-static member or a virtual
//! method through a null handle is illegal. A non-virtual method whose body
//! does not touch `this` runs, bound to the receiver's declared class (#1139;
//! the reference simulator runs it too). A virtual method through a null
//! handle is reported, and at the top level the run aborts: before, every
//! call through a null handle ran nothing and reported nothing (#532,
//! reported by akashlevy). A null-guarded call stays silent; a static method
//! needs no object.

use xezim::simulate;

fn msgs(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn virtual_call_through_a_null_handle_is_reported_and_aborts() {
    let src = r#"
class C;
  virtual function void hello(); $display("hello"); endfunction
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
    assert!(
        m.iter()
            .any(|s| s.contains("null handle") && s.contains("`hello`")),
        "{m:?}"
    );
    assert!(!m.iter().any(|s| s == "hello" || s == "post"), "{m:?}");
}

#[test]
fn non_virtual_task_and_function_calls_through_a_null_handle_run() {
    for (call, want) in [
        ("c.hello();", &["hello", "post 0"][..]),
        ("c.t();", &["task", "post 0"][..]),
        ("r = c.f();", &["post 3"][..]),
        ("void'(c.f());", &["post 0"][..]),
    ] {
        let src = format!(
            r#"
class C;
  function void hello(); $display("hello"); endfunction
  function int f(); return 3; endfunction
  task t(); $display("task"); endtask
endclass
module t;
  C c; int r;
  initial begin
    {call}
    $display("post %0d", r);
  end
endmodule
"#
        );
        let sim = simulate(&src, 10).expect("simulate");
        let m = msgs(&sim);
        assert!(!sim.saw_fatal, "{call}: {m:?}");
        assert_eq!(m, want, "{call}");
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
