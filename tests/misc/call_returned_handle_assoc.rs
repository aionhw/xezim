//! Pure-SystemVerilog regression: writing to an associative-array member of a
//! handle returned by a function call (`get_h().aa[k] = v`) must persist.
//!
//! Before the fix, `expr_assoc_name` only handled `MemberAccess` bases that
//! were `Ident`/`Index`/`MemberAccess`/`This`. A `Call` base (`f().member`)
//! fell through to `_ => None`, so the WRITE path in `assign_value` silently
//! dropped the store and the READ (`.exists()/.num()`) resolved `member` to
//! nothing. Both came back empty.
use xezim::simulate;

fn out_line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no {tag} line"))
}

/// `f().member[key] = v` then `f().member.exists(key)` / `.num()`.
#[test]
fn call_returned_handle_assoc_write_persists() {
    const SRC: &str = r#"
module top;
  class c;
    bit aa[string];
  endclass
  c h;
  function c get_h();
    if (h == null) h = new();
    return h;
  endfunction
  initial begin
    get_h().aa["x"] = 1;                       // write via returned handle
    $display("exists_x=%0d num=%0d", get_h().aa.exists("x"), get_h().aa.num());
    if (get_h().aa.exists("x") && get_h().aa.num()==1) $display("TAG_PASS");
    else $display("TAG_FAIL");
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    let line = out_line(&sim, "exists_x=");
    assert_eq!(line, "exists_x=1 num=1", "write via f().member[k] was lost");
    assert_eq!(out_line(&sim, "TAG_"), "TAG_PASS");
}

/// Handle-keyed variant: `f().aa[obj] = v` where the key is a class handle.
#[test]
fn call_returned_handle_assoc_class_key_write() {
    const SRC: &str = r#"
module top;
  class k; endclass
  class c;
    bit aa[k];
  endclass
  c h;
  function c get_h();
    if (h == null) h = new();
    return h;
  endfunction
  initial begin
    k kk = new();
    get_h().aa[kk] = 1;
    $display("exists=%0d num=%0d", get_h().aa.exists(kk), get_h().aa.num());
    if (get_h().aa.exists(kk) && get_h().aa.num()==1) $display("TAG_PASS");
    else $display("TAG_FAIL");
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(out_line(&sim, "exists="), "exists=1 num=1");
    assert_eq!(out_line(&sim, "TAG_"), "TAG_PASS");
}
