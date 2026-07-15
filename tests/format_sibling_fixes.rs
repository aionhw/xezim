//! Sibling-shape fixes to the §21.2.1 format engine (July-2026 audit).
//!
//! Each case below was diffed byte-for-byte against the ground-truth oracle
//! (C `printf` and Icarus `iverilog -g2012`). The oracle string is quoted in
//! each assertion's comment.
//!
//!   F1  %g/%G   the %f-vs-%e choice was made on the RAW value's exponent
//!               (`log10().floor()`), so it picked wrongly at a rounding
//!               boundary — `%g` of 999999.5 printed `1000000` where C/Icarus
//!               round-first and print `1e+06`. (§21.2.1.2; C99 %g.)
//!   F3  radix   `%Nh`/`%Nb`/`%No` always zero-padded like `%0Nh`; the leading
//!               space-pad form (and the fact that an explicit width never
//!               trims below the natural vector width) was lost. (§21.2.1.3.)
//!   F4  %+e/%+g the `+` flag was honoured on %f/%d but silently dropped on
//!               %e/%E/%g/%G. (§21.2.1.2.)
//!   F5  inf/nan non-finite reals printed Rust's `{}` spelling (`NaN`/`inf`);
//!               C/Icarus print `inf`/`nan` for lowercase specifiers and
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
    // and Icarus `$display("%g",…)` byte-for-byte.
    assert_eq!(line(&sim, "A="), "A=[1e+06]"); // C/Icarus: 1e+06 (was 1000000)
    assert_eq!(line(&sim, "B="), "B=[1e+06]"); // C/Icarus: 1e+06
    assert_eq!(line(&sim, "C="), "C=[0.0001]"); // C/Icarus: 0.0001
    assert_eq!(line(&sim, "D="), "D=[0.0001]"); // C/Icarus: 0.0001
    assert_eq!(line(&sim, "E="), "E=[123457]"); // C/Icarus: 123457
    assert_eq!(line(&sim, "F="), "F=[0.1]"); // C/Icarus: 0.1
    assert_eq!(line(&sim, "G="), "G=[100000]"); // C/Icarus: 100000
    assert_eq!(line(&sim, "H="), "H=[1e-05]"); // C/Icarus: 1e-05
    assert_eq!(line(&sim, "I="), "I=[3.14]"); // C/Icarus: 3.14
    assert_eq!(line(&sim, "J="), "J=[0]"); // C/Icarus: 0
}

// ------------------------------------------------------------------- F3 --

const RADIX_WIDTH: &str = r#"
module tb;
  reg [7:0]  r8;
  reg [31:0] r32;
  initial begin
    r8 = 8'h0f;
    $display("h4=[%4h]",  r8);   // Icarus: "  0f"
    $display("h04=[%04h]", r8);  // Icarus: "000f"
    $display("hL4=[%-4h]", r8);  // Icarus: "0f  "
    $display("o4=[%4o]",  r8);   // Icarus: " 017"
    $display("b4=[%4b]",  r8);   // Icarus: "00001111" (width < natural)
    $display("b04=[%04b]", r8);  // Icarus: "00001111"
    $display("b10=[%10b]", r8);  // Icarus: "  00001111"
    $display("h0=[%0h]",  r8);   // Icarus: "0f"? no -> trimmed "f"
    r32 = 32'hFF;
    $display("w2=[%2h]",  r32);  // Icarus: "000000ff" (never truncates)
    $display("w10z=[%010h]", r32); // Icarus: "00000000ff"
    // `0` flag + left-justify trims to the minimal form, then space-pads
    // (Icarus pr2476430): distinct from `0` flag + right-justify, which
    // zero-pads the natural width.
    $display("mL8=[%-08h]", r32); // Icarus: "ff      "
    $display("mR8=[%08h]",  r32); // Icarus: "000000ff"
  end
endmodule
"#;

#[test]
fn radix_width_honours_zero_flag_and_natural_width() {
    let sim = simulate(RADIX_WIDTH, 100).expect("simulate failed");
    // §21.2.1.3 — each RHS below matches Icarus byte-for-byte.
    assert_eq!(line(&sim, "h4="), "h4=[  0f]");
    assert_eq!(line(&sim, "h04="), "h04=[000f]");
    assert_eq!(line(&sim, "hL4="), "hL4=[0f  ]");
    assert_eq!(line(&sim, "o4="), "o4=[ 017]");
    assert_eq!(line(&sim, "b4="), "b4=[00001111]");
    assert_eq!(line(&sim, "b04="), "b04=[00001111]");
    assert_eq!(line(&sim, "b10="), "b10=[  00001111]");
    assert_eq!(line(&sim, "h0="), "h0=[f]");
    assert_eq!(line(&sim, "w2="), "w2=[000000ff]");
    assert_eq!(line(&sim, "w10z="), "w10z=[00000000ff]");
    assert_eq!(line(&sim, "mL8="), "mL8=[ff      ]"); // Icarus: trimmed + spaces
    assert_eq!(line(&sim, "mR8="), "mR8=[000000ff]"); // Icarus: natural, zero-pad
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
    // sign-bit artifact that Icarus also normalises away).
    assert_eq!(line(&sim, "f="), "f=[inf][-inf][nan]");
    assert_eq!(line(&sim, "F="), "F=[INF][-INF][NAN]");
    assert_eq!(line(&sim, "e="), "e=[inf][-inf][nan]");
    assert_eq!(line(&sim, "E="), "E=[INF][-INF][NAN]");
    assert_eq!(line(&sim, "g="), "g=[inf][-inf][nan]");
    assert_eq!(line(&sim, "G="), "G=[INF][-INF][NAN]");
}
