//! IEEE 1800-2023 §8.20 / §23.9: a bare call inside a class method binds
//! to the enclosing class chain before any same-named function elsewhere.
//! The return-type probe behind a comparison looked the bare name up in
//! the module's function table first, so an `extern` method returning
//! `int unsigned` in another class (or a free function) made `queue_c`'s
//! own `int size()` compare unsigned and `-1 >= size()` read 1 (#541,
//! reported by urbanij).

use xezim::simulate;

const CLASSES: &str = r#"
class queue_c;
  int queue[$];
  virtual function int size(); return queue.size(); endfunction
  function bit out_of_range(int index = -1); return index >= size(); endfunction
endclass
module top;
  initial begin
    queue_c q = new();
    $display("R|%0d", q.out_of_range());
  end
endmodule
"#;

fn result(prefix: &str) -> Vec<String> {
    let sim = simulate(&format!("{prefix}\n{CLASSES}"), 10).expect("simulate");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn extern_unsigned_method_of_another_class_does_not_leak() {
    let m = result(r#"
class cache_c;
  int m[$];
  extern function int unsigned size();
endclass
function int unsigned cache_c::size(); return m.size(); endfunction
"#);
    assert!(m.contains(&"R|0".to_string()), "{m:?}");
}

#[test]
fn extern_unsigned_method_in_a_package_does_not_leak() {
    let m = result(r#"
package p;
class cache_c;
  int m[$];
  extern function int unsigned size();
endclass
function int unsigned cache_c::size(); return m.size(); endfunction
endpackage
"#);
    assert!(m.contains(&"R|0".to_string()), "{m:?}");
}

#[test]
fn free_unsigned_function_does_not_shadow_the_class_method() {
    let m = result("function int unsigned size(); return 0; endfunction");
    assert!(m.contains(&"R|0".to_string()), "{m:?}");
}
