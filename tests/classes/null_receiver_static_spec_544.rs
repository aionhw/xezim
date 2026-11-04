//! IEEE 1800 §8.25: a static method entered through a NULL receiver whose
//! declared type is a SPECIALIZATION runs in that specialization's context —
//! the body's bare self-references (the `C#(T)` argument of a member typedef,
//! `uvm_object_param_utils`'s `type_id`) denote the CURRENT specialization,
//! not the default. This is `uvm_create_on`'s shape: `req.get_type()` where
//! `req` is declared `C#(T)` and is null at the call, so the factory must
//! create `C#(T)`; keying the registry on the default `C` created
//! default-specialized objects and the `$cast` rejected them.

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
  typedef registry#(item#(T)) type_id;
  static function wrapper get_type();
    return type_id::get();
  endfunction
endclass
module tb;
  int errors = 0;
  initial begin
    item#(byte) v;
    item#(byte) b;
    base obj;
    obj = v.get_type().create("it");
    if (!$cast(b, obj)) begin
      $display("T|cast FAIL");
      errors++;
    end
    void'(item#(byte)::get_type());
    if (registry#(item#(byte))::hits !== 2) begin
      $display("T|byte_hits %0d", registry#(item#(byte))::hits);
      errors++;
    end
    if (registry#(item#(int))::hits !== 0) begin
      $display("T|int_hits %0d", registry#(item#(int))::hits);
      errors++;
    end
    if (errors == 0) $display("T|done 0");
    else             $display("T|done %0d", errors);
  end
endmodule
"#;

/// `v.get_type()` on a null `v` declared `item#(byte)` keys the byte registry
/// and the factory-created object $casts to `item#(byte)`.
#[test]
fn null_receiver_static_method_runs_in_declared_spec() {
    let out = lines(SRC, "tb");
    assert!(
        out.iter().any(|l| l.contains("T|done 0")),
        "expected all checks to pass, got {out:?}"
    );
}