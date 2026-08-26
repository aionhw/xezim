//! A string value assigned to a LOCAL declared with a type-PARAMETER type
//! (`T t = <string>;` inside a parameterized class) must not be truncated.
//!
//! Fix covered: `StatementKind::VarDecl` computed the local's width with
//! `resolve_type_width(data_type, ...)`. For a local declared as a bare type
//! parameter (`T t;` with `T = string`), `data_type` is a `TypeReference`
//! naming the param, which `resolve_type_width` sizes to its 32-bit fallback —
//! so `T t = "Hello, world!"` stored the value in a 4-byte cell and truncated
//! the string to its low 32 bits (the LAST 4 chars, e.g. `"d!"`). UVM's
//! `uvm_resource#(T)::do_read`/`do_write` overrides (`T t = super.do_read()`,
//! then `return t`) were silently corrupted for T=string, breaking
//! `10resources/30extensible/02virtual_rw`.
//!
//! The fix resolves a type-param-typed local's declared type through the
//! active specialization binding (`T -> string`) before sizing, so a string
//! param-typed local gets 1024-bit (128-char) string storage and its
//! string-ness flag. Non-parameterized classes are untouched.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

// ── Param-typed local returning a string param (super round-trip) ─────
// Mirrors `uvm_resource#(T)` overrides: `T t = super.do_read(); return t;`.
// Before the fix the local `t` was 4 bytes wide and the string truncated.
const PARAM_LOCAL_STRING_SRC: &str = r#"
module top;
  class base #(parameter type T=int);
    T val;
    virtual function T do_read();
      return val;
    endfunction
  endclass
  class myres #(parameter type T=int) extends base#(T);
    virtual function T do_read();
      T t = super.do_read();
      return t;
    endfunction
  endclass
  function void check_impl();
    myres#(string) ms;
    string got;
    ms = new;
    ms.val = "Hello, world!";
    got = ms.do_read();
    if (got == "Hello, world!") $display("TAG_PASS");
    else $display("TAG_FAIL got=[%s]", got);
  endfunction
  initial begin
    check_impl();
  end
endmodule
"#;

#[test]
fn test_param_typed_string_local_roundtrip() {
    let sim = simulate(PARAM_LOCAL_STRING_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "param-typed string local must keep the full string; got {:?}",
        msgs
    );
}

// ── Same shape but several lengths: every string survives ─────────────
// Confirms the truncation gate is gone, not just tuned to one length.
const PARAM_LOCAL_STRING_LENGTHS_SRC: &str = r#"
module top;
  class base #(parameter type T=int);
    T val;
    virtual function T do_read();
      return val;
    endfunction
  endclass
  class myres #(parameter type T=int) extends base#(T);
    virtual function T do_read();
      T t = super.do_read();
      return t;
    endfunction
  endclass
  initial begin
    myres#(string) ms;
    string got;
    int fail;
    ms = new;
    begin
      string test[4];
      test[0] = "abcdef";
      test[1] = "abcdefgh";
      test[2] = "ABCDEFGHIJKLMNOPQRST";
      test[3] = "short";
      for (int i = 0; i < 4; i++) begin
        ms.val = test[i];
        got = ms.do_read();
        if (got != test[i]) begin
          $display("TAG_FAIL i=%0d len=%0d got=[%s] want=[%s]", i, test[i].len(), got, test[i]);
          fail = 1;
        end
      end
    end
    if (!fail) $display("TAG_PASS");
  end
endmodule
"#;

#[test]
fn test_param_typed_string_local_all_lengths() {
    let sim = simulate(PARAM_LOCAL_STRING_LENGTHS_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "all lengths must round-trip intact; got {:?}",
        msgs
    );
}