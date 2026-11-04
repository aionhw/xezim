//! IEEE 1800 §8.22/§6.21: name resolution is per-scope — a string-valued
//! property named `val` in one class must not change how `val == t` compares
//! in a different class. The flat, name-global `string_signals` set routed an
//! enum `val == t` through the TEXT comparison, where `X == A0` compares
//! equal, so `uvm_resource#(T)::write`'s `if (val == t) return;` early-out
//! skipped the first write and an enum-typed `uvm_config_db` get returned X.

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
typedef enum logic [2:0] { A0, A1, A2 } e4_t;

class clp_t;                          // the poison: a string-valued `val`
  function new();
    string val;
    val = "x";
  endfunction
endclass

class res #(type T = int);            // the uvm_resource#(T) shape
  protected T val;
  function void write(T t);
    if (val == t)
      return;
    val = t;
  endfunction
  function T read();
    return val;
  endfunction
endclass

module tb;
  int errors = 0;
  initial begin
    clp_t c = new();
    res #(string) rs = new();
    res #(e4_t) r = new();
    rs.write("abc");
    $display("T|s=%0s", rs.read());
    if (rs.read() != "abc") begin
      $display("T|FAIL string round-trip");
      errors++;
    end
    rs.write("abd");                  // a different string must overwrite
    $display("T|s=%0s", rs.read());
    if (rs.read() != "abd") begin
      $display("T|FAIL string overwrite");
      errors++;
    end
    r.write(A0);
    $display("T|f0=%0d", r.read());
    if (r.read() !== A0) begin
      $display("T|FAIL enum reads x (first write was skipped)");
      errors++;
    end
    r.write(A1);
    $display("T|f1=%0d", r.read());
    if (r.read() !== A1) begin
      $display("T|FAIL enum overwrite f1");
      errors++;
    end
    r.write(A2);
    $display("T|f2=%0d", r.read());
    if (r.read() !== A2) begin
      $display("T|FAIL enum overwrite f2");
      errors++;
    end
    if (errors == 0) $display("T|ALL CHECKS PASSED");
    else             $display("T|CHECKS FAILED: %0d", errors);
  end
endmodule
"#;

/// A string-typed `val` in an unrelated class must not make an enum `val == t`
/// compare as text (where `X == A0` is equal) and skip the first write.
#[test]
fn string_valued_property_does_not_poison_other_classes() {
    let out = lines(SRC, "tb");
    assert!(
        out.iter().any(|l| l.contains("T|ALL CHECKS PASSED")),
        "expected all checks to pass, got {out:?}"
    );
}