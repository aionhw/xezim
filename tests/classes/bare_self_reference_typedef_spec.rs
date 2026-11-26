//! IEEE 1800-2023 §8.25: a bare class name used as a type argument denotes
//! the default specialization, including inside the class's own declaration.
//! Both registry calls therefore share the static counter for item#(int).

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
  // As a type argument, bare item denotes item#().
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
    $display("T|byte_cast %0d", $cast(b, obj));
    void'(other::get_type());
    $display("T|byte_hits %0d", registry#(item#(byte))::hits);
    $display("T|int_hits %0d", registry#(item#(int))::hits);
    $display("T|done");
    $finish;
  end
endmodule
"#;

#[test]
fn bare_class_type_argument_denotes_the_default_specialization() {
    let out: Vec<String> = lines(SRC, "tb")
        .into_iter()
        .filter(|l| l.starts_with("T|"))
        .collect();
    assert_eq!(
        out,
        ["T|byte_cast 0", "T|byte_hits 0", "T|int_hits 2", "T|done"]
    );
}
