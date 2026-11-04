//! IEEE 1800-2017 §8.25: inside a parameterized class, the unadorned class
//! name denotes the specialization being elaborated — `typedef registry#(item)
//! type_id;` written in `class item #(type T)` is `registry#(item#(T))`
//! (`uvm_object_utils(C)` writes `uvm_object_registry#(C, "C")` this way). A
//! static method entered through a null `item#(byte)` handle must therefore
//! key the registry on `item#(byte)`, not on the default `item`. The rewrite
//! that does this (#545) is gated on the code running inside `item`'s own
//! declaration: a non-parameterized class whose typedef names `item` bare
//! means the default specialization and keeps it.

fn lines(src: &str, top: &str) -> Vec<String> {
    xezim::simulate_multi(
        &[src.to_string()],
        1000,
        Some(top),
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &[],
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
    )
    .expect("sim")
    .output
    .iter()
    .map(|o| o.message.clone())
    .collect()
}

const SRC: &str = r#"
class base;
endclass
virtual class wrapper;
  pure virtual function base create(string name);
endclass
class registry #(type T = base) extends wrapper;
  static int unsigned hits;
  static function wrapper get();
    registry#(T) r = new();
    hits++;
    return r;
  endfunction
  virtual function base create(string name);
    T obj;
    obj = new();
    return obj;
  endfunction
endclass
class item #(type T = int) extends base;
  // Bare self-reference: `item` here is `item#(T)`.
  typedef registry#(item) type_id;
  static function wrapper get_type();
    return type_id::get();
  endfunction
endclass
// A non-parameterized class naming `item` bare: the DEFAULT specialization.
class other;
  typedef registry#(item) default_id;
  static function wrapper get_type();
    return default_id::get();
  endfunction
endclass
module tb;
  initial begin
    item#(byte) v;
    item#(byte) b;
    base obj;
    obj = v.get_type().create("it");
    if (!$cast(b, obj)) $display("T|cast FAIL");
    void'(other::get_type());
    if (registry#(item#(byte))::hits !== 1)
      $display("T|byte_hits %0d", registry#(item#(byte))::hits);
    if (registry#(item#(int))::hits !== 1)
      $display("T|int_hits %0d", registry#(item#(int))::hits);
    $display("T|done");
    $finish;
  end
endmodule
"#;

#[test]
fn bare_self_reference_in_a_parameterized_class_is_the_current_specialization() {
    let out: Vec<String> = lines(SRC, "tb")
        .into_iter()
        .filter(|l| l.starts_with("T|"))
        .collect();
    assert_eq!(out, ["T|done"], "{out:?}");
}
