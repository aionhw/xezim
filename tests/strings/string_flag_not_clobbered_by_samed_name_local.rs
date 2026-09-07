//! §6.16 + §13.3.1 — a nested subroutine's NON-STRING local must not clobber
//! the "is-string" flag of an enclosing scope that declares the SAME bare
//! name as a `string`.
//!
//! `string_signals` is a flat, bare-name-keyed registry used to decide whether
//! an identifier takes string semantics (`%s`, concat, char-index, resize
//! suppression). A frame's string FORMALS/RETURN are already removed on return
//! (`frame_string_signals`), but a non-string LOCAL in a called function (e.g.
//! `int m`) runs `string_signals.remove("m")` — the block-local decl clears a
//! stale same-named flag — which would persist past that function's return and
//! permanently clobber the *caller's* `string m`. A later `m = <long string>`
//! in the caller is then no longer recognised as a string, so it is resized to
//! the 1024-bit placeholder and truncated to its last 128 characters.
//!
//! Distilled from a UVM report-message (element-container) test where the
//! composed multi-line message string was silently cut to 128 characters.
//! Verified byte-identical to a reference simulator (probes).

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// A called function declaring `int m` must not truncate the caller's `string
/// m` when it is later assigned a value longer than 128 characters.
#[test]
fn nested_non_string_local_does_not_clobber_string_flag() {
    let src = r#"
module top;
  int mlen;
  string gmsg = "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF";
  function automatic void inner();   // declares a NON-string local named `m`
    automatic int m;
    m = 12345;
  endfunction
  function automatic string mk();
    automatic string s;
    s = gmsg;
    mk = {"HEAD=", s};
  endfunction
  initial begin
    string m;
    inner();          // runs first: its `int m` must NOT strip callers' string-flag
    m = mk();         // >128 chars; must be kept whole
    mlen = m.len();
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    // 5 ("HEAD=") + 9*16 + 10*16 = 5 + 144 = 149 (gmsg is 144 chars).
    assert_eq!(u(&sim, "mlen"), 149, "string kept whole past 128 chars");
}

/// Sanity: a called function whose local does NOT share the name leaves the
/// caller's string intact too (control case).
#[test]
fn unrelated_local_name_leaves_string_whole() {
    let src = r#"
module top;
  int mlen;
  string gmsg = "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF";
  function automatic void inner();
    automatic int x;   // NOT `m` — must never have clobbered anything
    x = 1;
  endfunction
  function automatic string mk();
    automatic string s;
    s = gmsg;
    mk = {"HEAD=", s};
  endfunction
  initial begin
    string m;
    inner();
    m = mk();
    mlen = m.len();
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "mlen"), 149, "string kept whole past 128 chars");
}