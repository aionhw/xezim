//! IEEE 1800-2017 §6.16 string methods that MUTATE the receiver, plus
//! `atoreal`.
//!
//! `putc`, `itoa`, `hextoa`, `octtoa`, `bintoa` and `realtoa` were silent
//! no-ops, and `atoreal` always returned 0.0. Only the read-only methods
//! (`len`, `getc`, `substr`, `toupper`, `tolower`, `atoi`, `atohex`, `atooct`,
//! `atobin`) were implemented.
//!
//! The mutators were doubly hidden: a statement like `s.putc(0, "J");` parses
//! its receiver as a FLATTENED `Ident([s, putc])`, not a `MemberAccess`, so a
//! handler on the MemberAccess path alone would still have looked like a no-op.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  string s_putc, s_putc_int, s_oob, s_neg, s_null, s_last;
  string s_itoa, s_itoa_neg, s_hex, s_oct, s_bin, s_real, s_local;
  real r_ok, r_none, r_part, r_exp, r_int;

  initial begin
    // §6.16.4 putc
    s_putc = "Hello";  s_putc.putc(0, "J");        // char literal
    s_putc_int = "Hello"; s_putc_int.putc(4, 8'h21); // integer code
    // Out-of-range index or a null character leaves the string unchanged.
    s_oob  = "abc"; s_oob.putc(5, "Z");
    s_neg  = "abc"; s_neg.putc(-1, "Z");
    s_null = "abc"; s_null.putc(1, 8'h00);
    s_last = "abc"; s_last.putc(2, "Z");

    // §6.16.10 the *toa family
    s_itoa = "";     s_itoa.itoa(255);
    s_itoa_neg = ""; s_itoa_neg.itoa(-5);     // itoa renders a SIGNED decimal
    s_hex = "";      s_hex.hextoa(255);
    s_oct = "";      s_oct.octtoa(8);
    s_bin = "";      s_bin.bintoa(5);
    s_real = "";     s_real.realtoa(3.5);

    // §6.16.9 atoreal — longest valid real prefix, 0.0 on no match.
    r_ok   = "3.5".atoreal();
    r_none = "abc".atoreal();
    r_part = "12.5xyz".atoreal();
    r_exp  = "-1.5e2".atoreal();
    r_int  = "7".atoreal();

    // A receiver that lives in a procedural frame rather than the signal table.
    begin
      string t;
      t.itoa(-42);
      s_local = t;
    end
  end
endmodule
"#;

fn text(sim: &xezim::compiler::Simulator, n: &str) -> String {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_sv_string()
}

fn real(sim: &xezim::compiler::Simulator, n: &str) -> f64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_f64()
}

#[test]
fn putc_replaces_a_character_in_place() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(text(&sim, "s_putc"), "Jello", "putc was a no-op");
    assert_eq!(text(&sim, "s_putc_int"), "Hell!");
    assert_eq!(text(&sim, "s_last"), "abZ");
}

#[test]
fn putc_leaves_the_string_alone_on_a_bad_index_or_null_char() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(text(&sim, "s_oob"), "abc", "index past the end must be ignored");
    assert_eq!(text(&sim, "s_neg"), "abc", "negative index must be ignored");
    assert_eq!(text(&sim, "s_null"), "abc", "a null character must be ignored");
}

#[test]
fn the_toa_family_writes_the_receiver() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(text(&sim, "s_itoa"), "255", "itoa was a no-op");
    assert_eq!(text(&sim, "s_itoa_neg"), "-5", "itoa must render a signed decimal");
    assert_eq!(text(&sim, "s_hex"), "ff");
    assert_eq!(text(&sim, "s_oct"), "10");
    assert_eq!(text(&sim, "s_bin"), "101");
    assert_eq!(text(&sim, "s_real"), "3.500000");
    // The receiver may live in a procedural frame.
    assert_eq!(text(&sim, "s_local"), "-42");
}

#[test]
fn atoreal_parses_the_longest_valid_prefix() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(real(&sim, "r_ok"), 3.5, "atoreal returned 0.0");
    assert_eq!(real(&sim, "r_none"), 0.0, "no valid prefix -> 0.0");
    assert_eq!(real(&sim, "r_part"), 12.5, "stop at the first invalid character");
    assert_eq!(real(&sim, "r_exp"), -150.0, "exponent form");
    assert_eq!(real(&sim, "r_int"), 7.0);
}
