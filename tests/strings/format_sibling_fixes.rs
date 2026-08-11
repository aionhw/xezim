//! Sibling-shape fixes to the §21.2.1 format engine (July-2026 audit).
//!
//! Each case below was diffed byte-for-byte against the ground-truth oracle
//! (C `printf` and a reference simulator `a reference simulator`). The oracle string is quoted in
//! each assertion's comment.
//!
//!   F1  %g/%G   the %f-vs-%e choice was made on the RAW value's exponent
//!               (`log10().floor()`), so it picked wrongly at a rounding
//!               boundary — `%g` of 999999.5 printed `1000000` where C/a reference simulator
//!               round-first and print `1e+06`. (§21.2.1.2; C99 %g.)
//!   F3  radix   `%Nh`/`%Nb`/`%No` always zero-padded like `%0Nh`; the leading
//!               space-pad form (and the fact that an explicit width never
//!               trims below the natural vector width) was lost. (§21.2.1.3.)
//!   F4  %+e/%+g the `+` flag was honoured on %f/%d but silently dropped on
//!               %e/%E/%g/%G. (§21.2.1.2.)
//!   F5  inf/nan non-finite reals printed Rust's `{}` spelling (`NaN`/`inf`);
//!               C/a reference simulator print `inf`/`nan` for lowercase specifiers and
//!               `INF`/`NAN` for the uppercase ones, sign only on `-inf`.

use xezim::simulate;

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no output line starting with {}", tag))
}

// ------------------------------------------------------------------- F1 --

const G_ROUNDING: &str = r#"
module tb;
  real v;
  initial begin
    v = 999999.5;      $display("A=[%g]", v);
    v = 1000000.0;     $display("B=[%g]", v);
    v = 0.0001;        $display("C=[%g]", v);
    v = 0.00009999995; $display("D=[%g]", v);
    v = 123456.7;      $display("E=[%g]", v);
    v = 0.1;           $display("F=[%g]", v);
    v = 100000.0;      $display("G=[%g]", v);
    v = 1e-5;          $display("H=[%g]", v);
    // Existing %g coverage must keep working.
    v = 3.14159;       $display("I=[%.3g]", v);
    v = 0.0;           $display("J=[%g]", v);
  end
endmodule
"#;

#[test]
fn g_decides_f_vs_e_after_rounding() {
    let sim = simulate(G_ROUNDING, 100).expect("simulate failed");
    // §21.2.1.2 / C99 %g — every literal below matches C `printf("%g",…)`
    // and a reference simulator `$display("%g",…)` byte-for-byte.
    assert_eq!(line(&sim, "A="), "A=[1e+06]"); // C/a reference simulator: 1e+06 (was 1000000)
    assert_eq!(line(&sim, "B="), "B=[1e+06]"); // C/a reference simulator: 1e+06
    assert_eq!(line(&sim, "C="), "C=[0.0001]"); // C/a reference simulator: 0.0001
    assert_eq!(line(&sim, "D="), "D=[0.0001]"); // C/a reference simulator: 0.0001
    assert_eq!(line(&sim, "E="), "E=[123457]"); // C/a reference simulator: 123457
    assert_eq!(line(&sim, "F="), "F=[0.1]"); // C/a reference simulator: 0.1
    assert_eq!(line(&sim, "G="), "G=[100000]"); // C/a reference simulator: 100000
    assert_eq!(line(&sim, "H="), "H=[1e-05]"); // C/a reference simulator: 1e-05
    assert_eq!(line(&sim, "I="), "I=[3.14]"); // C/a reference simulator: 3.14
    assert_eq!(line(&sim, "J="), "J=[0]"); // C/a reference simulator: 0
}

// ------------------------------------------------------------------- F3 --

const RADIX_WIDTH: &str = r#"
module tb;
  reg [7:0]  r8;
  reg [31:0] r32;
  reg [15:0] r16;
  initial begin
    r8 = 8'h0f;
    $display("h4=[%4h]",  r8);   // reference: "000f" (zero-pad, `0` flag irrelevant)
    $display("h04=[%04h]", r8);  // reference: "000f"
    $display("hL4=[%-4h]", r8);  // reference: "f   " (minimal + right spaces)
    $display("o4=[%4o]",  r8);   // reference: "0017"
    $display("b4=[%4b]",  r8);   // reference: "1111" (minimal fits exactly)
    $display("b04=[%04b]", r8);  // reference: "1111"
    $display("b10=[%10b]", r8);  // reference: "0000001111"
    $display("h0=[%0h]",  r8);   // reference: trimmed "f"
    r32 = 32'hFF;
    $display("w2=[%2h]",  r32);  // reference: "ff" (minimal, never truncated)
    $display("w10z=[%010h]", r32); // reference: "00000000ff"
    $display("mL8=[%-08h]", r32); // reference: "ff      "
    $display("mR8=[%08h]",  r32); // reference: "000000ff"
    r16 = 16'h2a5;
    $display("n2=[%2h]",  r16);  // reference: "2a5" (minimal wider than field)
    $display("z6=[%6h]",  8'hzz);   // reference: "0000zz" (x/z run KEPT with a width)
    $display("m6=[%6h]",  16'hxx3f);// reference: "00xx3f"
    $display("m0=[%0h]",  16'hxx3f);// reference: "x3f" (bare %0 collapses the run)
    $display("zero4=[%4h]", 8'h00); // reference: "0000"
  end
endmodule
"#;

#[test]
fn radix_width_honours_zero_flag_and_natural_width() {
    let sim = simulate(RADIX_WIDTH, 100).expect("simulate failed");
    // §21.2.1.3 — each RHS below matches the reference simulator
    // byte-for-byte (re-measured 2026-08; commercial tools disagree on this
    // family — the space-pad-to-natural-width model some of them use was
    // xezim's previous behavior, replaced deliberately).
    assert_eq!(line(&sim, "h4="), "h4=[000f]");
    assert_eq!(line(&sim, "h04="), "h04=[000f]");
    assert_eq!(line(&sim, "hL4="), "hL4=[f   ]");
    assert_eq!(line(&sim, "o4="), "o4=[0017]");
    assert_eq!(line(&sim, "b4="), "b4=[1111]");
    assert_eq!(line(&sim, "b04="), "b04=[1111]");
    assert_eq!(line(&sim, "b10="), "b10=[0000001111]");
    assert_eq!(line(&sim, "h0="), "h0=[f]");
    assert_eq!(line(&sim, "w2="), "w2=[ff]");
    assert_eq!(line(&sim, "w10z="), "w10z=[00000000ff]");
    assert_eq!(line(&sim, "mL8="), "mL8=[ff      ]");
    assert_eq!(line(&sim, "mR8="), "mR8=[000000ff]");
    assert_eq!(line(&sim, "n2="), "n2=[2a5]");
    assert_eq!(line(&sim, "z6="), "z6=[0000zz]");
    assert_eq!(line(&sim, "m6="), "m6=[00xx3f]");
    assert_eq!(line(&sim, "m0="), "m0=[x3f]");
    assert_eq!(line(&sim, "zero4="), "zero4=[0000]");
}

// ------------------------------------------------------------------- G9 --

const UNFORMATTED_ARGS: &str = r#"
module tb;
  logic [15:0] v = 16'd677;
  logic [7:0]  b = 8'd15;
  initial begin
    $display("u1=", v);          // reference: "u1=  677" (default %d width 5)
    $display("u2=x=", v, " y=", b); // reference: "u2=x=  677 y= 15"
    $display("u3=%h", 32'hdeadbeef, v); // reference: "u3=deadbeef  677"
  end
endmodule
"#;

#[test]
fn unconsumed_args_print_default_width_decimal() {
    // §21.2.1.2: an argument not consumed by a preceding format directive
    // prints in the task's default radix WITH the default field width —
    // reference-validated ("  677" for a 16-bit value, not "677").
    let sim = simulate(UNFORMATTED_ARGS, 100).expect("simulate failed");
    assert_eq!(line(&sim, "u1="), "u1=  677");
    assert_eq!(line(&sim, "u2="), "u2=x=  677 y= 15");
    assert_eq!(line(&sim, "u3="), "u3=deadbeef  677");
}

// ------------------------------------------------------------------- F4 --

const PLUS_FLAG: &str = r#"
module tb;
  real v;
  initial begin
    v = 12345.678; $display("A=[%+e]", v);     // C: +1.234568e+04
    v = 3.14;      $display("B=[%+g]", v);     // C: +3.14
    v = 12345.678; $display("C=[%+10.2e]", v); // C: " +1.23e+04"
    v = 3.14;      $display("D=[%+.3g]", v);   // C: +3.14
    v = -12345.678;$display("E=[%+e]", v);     // C: -1.234568e+04
    v = -3.14;     $display("F=[%+g]", v);     // C: -3.14
    v = 0.0;       $display("G=[%+e]", v);     // C: +0.000000e+00
    v = 0.0;       $display("H=[%+g]", v);     // C: +0
  end
endmodule
"#;

#[test]
fn plus_flag_applies_to_e_and_g() {
    let sim = simulate(PLUS_FLAG, 100).expect("simulate failed");
    // §21.2.1.2 — the `+` flag forces a sign on %e/%g just like %f/%d.
    assert_eq!(line(&sim, "A="), "A=[+1.234568e+04]");
    assert_eq!(line(&sim, "B="), "B=[+3.14]");
    assert_eq!(line(&sim, "C="), "C=[ +1.23e+04]");
    assert_eq!(line(&sim, "D="), "D=[+3.14]");
    assert_eq!(line(&sim, "E="), "E=[-1.234568e+04]");
    assert_eq!(line(&sim, "F="), "F=[-3.14]");
    assert_eq!(line(&sim, "G="), "G=[+0.000000e+00]");
    assert_eq!(line(&sim, "H="), "H=[+0]");
}

// ------------------------------------------------------------------- F5 --

const NON_FINITE: &str = r#"
module tb;
  real pinf, ninf, nan;
  initial begin
    pinf = 1.0/0.0; ninf = -1.0/0.0; nan = 0.0/0.0;
    $display("f=[%f][%f][%f]", pinf, ninf, nan);
    $display("F=[%F][%F][%F]", pinf, ninf, nan);
    $display("e=[%e][%e][%e]", pinf, ninf, nan);
    $display("E=[%E][%E][%E]", pinf, ninf, nan);
    $display("g=[%g][%g][%g]", pinf, ninf, nan);
    $display("G=[%G][%G][%G]", pinf, ninf, nan);
  end
endmodule
"#;

#[test]
fn non_finite_reals_print_c_spelling() {
    let sim = simulate(NON_FINITE, 100).expect("simulate failed");
    // C `printf`: lowercase inf/nan for %f/%e/%g, uppercase for %F/%E/%G.
    // Sign only on -inf; nan is unsigned (glibc's `-nan` from 0.0/0.0 is a
    // sign-bit artifact that a reference simulator also normalises away).
    assert_eq!(line(&sim, "f="), "f=[inf][-inf][nan]");
    assert_eq!(line(&sim, "F="), "F=[INF][-INF][NAN]");
    assert_eq!(line(&sim, "e="), "e=[inf][-inf][nan]");
    assert_eq!(line(&sim, "E="), "E=[INF][-INF][NAN]");
    assert_eq!(line(&sim, "g="), "g=[inf][-inf][nan]");
    assert_eq!(line(&sim, "G="), "G=[INF][-INF][NAN]");
}
