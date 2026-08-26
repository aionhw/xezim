//! `%p` (call/assignment-pattern format) must render param-typed STRING
//! symbols as quoted text, not the packed ASCII integer.
//!
//! Fix covered: a class-member/formal/local declared with a bare type-param
//! type (`T val` / `T t` / `T x` with `T=string`) was never registered as a
//! string in `string_signals`, and `get_expr_type_name`/`class_prop_type_named`
//! returned the param NAME (`T`) rather than its bound type. So `%p` fell
//! through to decimal and printed the ASCII bit pattern (e.g.
//! `5735816763073854953388147237921`) instead of `"Hello, world!"` — and a
//! param-typed METHOD LOCAL rendered as empty (`""`). This broke UVM resource
//! traces (`uvm_resource#(T)::do_write/do_read` `%p` output) in
//! `10resources/30extensible/02virtual_rw`.
//!
//! Fix: resolve a type-param's binding to its concrete type when classifying
//! a member (`class_prop_type_named`), register param-typed string FORMALS at
//! method-entry (once `current_spec` is seeded), and let
//! `get_signal_value_by_name` read a bare frame-local so the `%p` render path
//! sees the local's value rather than an empty padded read.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

// ── `%p` of a param-typed formal, member, and method local ────────────
// Mirrors `uvm_resource#(T)` `do_write`/`do_read` override `%p` traces.
const PARAM_STRING_PERCENT_SRC: &str = r#"
module top;
  class base #(parameter type T=int);
    T val;
    function void show(T t);
      T local_t;
      local_t = t;
      $display("formal=%p", t);
      $display("member=%p", val);
      $display("local=%p", local_t);
    endfunction
  endclass
  initial begin
    base#(string) b;
    b = new;
    b.val = "Hello, world!";
    b.show("Hello, world!");
  end
endmodule
"#;

#[test]
fn test_param_string_percent_renders_quoted() {
    let sim = simulate(PARAM_STRING_PERCENT_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    let quoted = msgs
        .iter()
        .filter(|m| m.contains("\"Hello, world!\""))
        .count();
    assert_eq!(
        quoted, 3,
        "formal, member and local must all render as quoted strings; got {:?}",
        msgs
    );
    assert!(
        !msgs.iter().any(|m| m.contains("5735816763073854953388147237921")),
        "a packed-ASCII %p of a param-typed string must not appear; got {:?}",
        msgs
    );
}

// ── Sanity: a non-string param must NOT be rendered as a string ───────
const PARAM_INT_PERCENT_SRC: &str = r#"
module top;
  class base #(parameter type T=int);
    T val;
    function void show(T t);
      $display("formal=%p", t);
    endfunction
  endclass
  initial begin
    base#(int) b;
    b = new;
    b.val = 42;
    b.show(42);
  end
endmodule
"#;

#[test]
fn test_param_int_percent_not_quoted() {
    let sim = simulate(PARAM_INT_PERCENT_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m.contains("formal=42")),
        "an int-param %p must stay a bare number, not a quoted string nor a packed bit pattern; got {:?}",
        msgs
    );
}

// ── An `int` (signed) param FORMAL keeps its signed interpretation ────
// `do_write(32'hDEAD_BEEF)` passes an UNSIGNED caller literal into a SIGNED
// `T t` formal (`T=int`); the reference renders the bit pattern as signed
// (`-559038737`), not the raw unsigned (`3735928559`). Mirrors
// `10resources/30extensible/02virtual_rw` `%p` do_write traces.
const PARAM_INT_SIGNED_SRC: &str = r#"
module top;
  class base #(parameter type T=int);
    function void show(T t);
      $display("formal=%p", t);
      $display("num=%0d", t);
    endfunction
  endclass
  initial begin
    base#(int) b;
    b = new;
    b.show(32'hDEAD_BEEF);
  end
endmodule
"#;

#[test]
fn test_param_signed_int_formal_is_signed() {
    let sim = simulate(PARAM_INT_SIGNED_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    // Both %p and %0d must see the bit pattern as SIGNED (32-bit two's
    // complement): -559038737, not the unsigned 3735928559.
    assert!(
        msgs.iter().any(|m| m.contains("formal=-559038737")),
        "%p of an int formal (0xDEADBEEF) must be signed; got {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m.contains("num=-559038737")),
        "%0d of an int formal (0xDEADBEEF) must be signed; got {:?}",
        msgs
    );
    assert!(
        !msgs.iter().any(|m| m.contains("3735928559")),
        "the unsigned rendering must not appear for an int formal; got {:?}",
        msgs
    );
}