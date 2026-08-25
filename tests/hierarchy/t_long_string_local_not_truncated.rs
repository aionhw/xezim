//! Two truncation bugs that capped every SystemVerilog `string` to 128 chars
//! (the 1024-bit placeholder width) in LOCAL storage:
//!
//! 1. A block/`initial`-local declared WITH an initializer
//!    (`string s = <long literal or method return>`) was resized to the
//!    1024-bit placeholder in the decl-with-init path, truncating a longer
//!    string from the FRONT (the START of the text) to 128 chars.
//! 2. A METHOD/class-local string assigned a value longer than 128 chars
//!    (`string m = compose_report_message(...)` returning a multi-hundred-char
//!    table) was truncated by the frame-local store, which exempted only
//!    locals named in `string_signals` — body locals aren't all registered.
//!    UVM's `uvm_table_printer::emit()`/`m_emit_element` build multi-row tables
//!    as method-local strings, so an element table dropped every row past 128
//!    chars (only the last leaf survived).
//!
//! `STRLENPASS` appears and `STRLENFAIL` absent. Without the fix, `decl`
//! reads 128 and `kept` reads 128 when the method returns >128 chars.

use xezim::simulate_multi;

fn run_src(src: &str) -> Vec<String> {
    simulate_multi(
        &[src.to_string()],
        1000,
        Some("top"),
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &[],
        1,
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
        None,
    )
    .expect("sim")
    .output
    .iter()
    .map(|o| o.message.clone())
    .collect()
}

#[test]
fn long_string_locals_are_not_truncated() {
    let src = r#"
function string make(int n);
  string base;
  string s;
  base = "abcdefghij";   // 10 chars
  s = "";
  for (int i = 0; i < n; i++) s = {s, base};
  return s;
endfunction
class P;
  function string keeper();
    string s;
    s = "";
    for (int i = 0; i < 15; i++) s = {s, "0123456789"}; // 150 chars
    return s;                                            // > 128 -> must survive
  endfunction
endclass
module top;
  initial begin
    int ok = 1;
    // (1) decl-with-init of a long literal
    string decl = "0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890"; // 160
    if (decl.len() < 140) ok = 0;   // well over 128 chars
    // (2) method-local string holding > 128 chars
    P p = new();
    string kept = p.keeper();   // 150 chars via method return
    if (kept.len() != 150) ok = 0;
    // (3) decl-with-init from a long method return
    string viaf = make(13);     // 130 chars
    if (viaf.len() != 130) ok = 0;
    if (ok) $display("STRLENPASS");
    else $display("STRLENFAIL decl=%0d kept=%0d viaf=%0d", decl.len(), kept.len(), viaf.len());
    $finish;
  end
endmodule
"#;
    let out = run_src(src);
    assert!(
        out.iter().any(|l| l.contains("STRLENPASS")),
        "long string locals must not truncate to 128 chars; got {:?}",
        out
    );
    assert!(
        !out.iter().any(|l| l.contains("STRLENFAIL")),
        "got {:?}",
        out
    );
}