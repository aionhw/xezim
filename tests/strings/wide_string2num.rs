//! Regression tests for wide string-to-number conversion.
//!
//! Decimal, hex, octal, and binary strings ≥ 2^63 must correctly parse into
//! arbitrarily wide destination variables, with truncation to the destination
//! width per LRM §6.16.8 (string methods), §11.4.12 (formatted I/O), and
//! §20.13.2 ($readmemd).
//!
//! Before the fix: $fscanf/$sscanf used i64::from_str_radix() → 0 for ≥ 2^63;
//! $value$plusargs hard-coded width=64; string.atoi() returned 0 for ≥ 2^64.
//! All now pass through Value::from_str_radix() with the destination width.

use xezim::simulate;

fn out(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .find(|l| l.starts_with(&format!("NOTE: {}", tag)))
        .unwrap_or_else(|| panic!("missing output for tag: {}", tag))
}

// ---------------------------------------------------------------------------
// $sscanf with wide hex / dec / bin / oct into wide vectors (§11.4.12)
// ---------------------------------------------------------------------------

const SSCANF_WIDE: &str = r#"
module tb;
  logic [159:0] w;
  logic [63:0]  v64;
  logic [31:0]  v32;
  int           cnt;
  initial begin
    w = '0; cnt = $sscanf("0123456789abcdef0123456789abcdef01234567", "%x", w);
    $display("NOTE: s_hex160 %0d %h", cnt, w);
    v64 = '0; cnt = $sscanf("8000000000000000", "%x", v64);
    $display("NOTE: s_hex64 %0d %h", cnt, v64);
    v32 = '0; cnt = $sscanf("deadbeefdeadbeef", "%x", v32);
    $display("NOTE: s_hex32 %0d %h", cnt, v32);
    w = '0; cnt = $sscanf("3ab4901f2c3d5e60", "%x", w);
    $display("NOTE: s_hexsmall %0d %h", cnt, w);
    w = '0; cnt = $sscanf("11111111111111111111111111111111111111111111111111111111111111111111111111111111", "%b", w);
    $display("NOTE: s_bin160 %0d %h", cnt, w);
    w = '0; cnt = $sscanf("1000000000000000000000", "%o", w);
    $display("NOTE: s_oct160 %0d %h", cnt, w);
    v64 = '0; cnt = $sscanf("12345678901234567890", "%d", v64);
    $display("NOTE: s_posdec64 %0d %h", cnt, v64);
    v64 = '0; cnt = $sscanf("-12345678901234567890", "%d", v64);
    $display("NOTE: s_negdec64 %0d %h", cnt, v64);
    v64 = '0; cnt = $sscanf("0123456789abcdef", "%16x", v64);
    $display("NOTE: s_w16x %0d %h", cnt, v64);
    w = '0; cnt = $sscanf("8000000000000000", "%16x", w);
    $display("NOTE: s_w16xwide %0d %h", cnt, w);
    $finish;
  end
endmodule
"#;

#[test]
fn sscanf_wide_hex_preserves_full_width() {
    let sim = simulate(SSCANF_WIDE, 100).expect("simulate failed");
    // 40-hex → 160-bit: all 160 bits survive (was truncated to low-32)
    assert_eq!(out(&sim, "s_hex160"),
        "NOTE: s_hex160 1 0123456789abcdef0123456789abcdef01234567");
    // 8000000000000000 → 64-bit: full 64-bit value (was 0)
    assert_eq!(out(&sim, "s_hex64"), "NOTE: s_hex64 1 8000000000000000");
    // deadbeefdeadbeef → 32-bit: low 32 = deadbeef (was 0)
    assert_eq!(out(&sim, "s_hex32"), "NOTE: s_hex32 1 deadbeef");
    // 3ab4901f2c3d5e60 < 2^63 → 160-bit: full value (was low-32 only)
    assert_eq!(out(&sim, "s_hexsmall"),
        "NOTE: s_hexsmall 1 0000000000000000000000003ab4901f2c3d5e60");
}

#[test]
fn sscanf_wide_bin_and_oct() {
    let sim = simulate(SSCANF_WIDE, 100).expect("simulate failed");
    // 80 ones → 160-bit: low 80 bits all 1
    assert_eq!(out(&sim, "s_bin160"),
        "NOTE: s_bin160 1 00000000000000000000ffffffffffffffffffff");
    // 2^63 via octal → 160-bit
    assert_eq!(out(&sim, "s_oct160"),
        "NOTE: s_oct160 1 0000000000000000000000008000000000000000");
}

#[test]
fn sscanf_wide_decimal_signed() {
    let sim = simulate(SSCANF_WIDE, 100).expect("simulate failed");
    // 12345678901234567890 mod 2^64 = 0xab54a98ceb1f0ad2
    assert_eq!(out(&sim, "s_posdec64"), "NOTE: s_posdec64 1 ab54a98ceb1f0ad2");
    // Two's complement: 2^64 - 0xab54a98ceb1f0ad2 = 0x54ab567314e0f52e
    assert_eq!(out(&sim, "s_negdec64"), "NOTE: s_negdec64 1 54ab567314e0f52e");
}

#[test]
fn sscanf_field_width_16x() {
    let sim = simulate(SSCANF_WIDE, 100).expect("simulate failed");
    assert_eq!(out(&sim, "s_w16x"), "NOTE: s_w16x 1 0123456789abcdef");
    assert_eq!(out(&sim, "s_w16xwide"),
        "NOTE: s_w16xwide 1 0000000000000000000000008000000000000000");
}

// ---------------------------------------------------------------------------
// $fscanf with wide hex (file-based — production pattern, §11.4.12)
// ---------------------------------------------------------------------------

const FSCANF_WIDE: &str = r#"
module tb;
  logic [160:0] w;
  int           fd, cnt;
  string        tok;
  initial begin
    fd = $fopen("wide_fscanf_test.txt", "w");
    $fdisplay(fd, "attribute 3ab4901f2c3d5e60");
    $fdisplay(fd, "data deadbeefdeadbeef");
    $fclose(fd);
    fd = $fopen("wide_fscanf_test.txt", "r");
    cnt = $fscanf(fd, "%s", tok);
    w = '0; cnt = $fscanf(fd, "%x", w);
    $display("NOTE: f_hexsmall %0d %h", cnt, w);
    cnt = $fscanf(fd, "%s", tok);
    w = '0; cnt = $fscanf(fd, "%x", w);
    $display("NOTE: f_hexlarge %0d %h", cnt, w);
    cnt = $fscanf(fd, "%s", tok);
    $display("NOTE: f_eof %0d", cnt);
    $fclose(fd);
    $finish;
  end
endmodule
"#;

#[test]
fn fscanf_wide_advances_past_large_token() {
    // §11.4.12: a token >= 2^63 must scan 1 conversion and advance the
    // file position. Before the fix it scanned 0 and re-read the token
    // (production "wrong cmd" fatal path).
    let sim = simulate(FSCANF_WIDE, 100).expect("simulate failed");
    assert_eq!(out(&sim, "f_hexsmall"), "NOTE: f_hexsmall 1 00000000000000000000000003ab4901f2c3d5e60");
    assert_eq!(out(&sim, "f_hexlarge"), "NOTE: f_hexlarge 1 0000000000000000000000000deadbeefdeadbeef");
    // After consuming all tokens, $fscanf returns -1 or 0 (clean EOF)
    let eof = out(&sim, "f_eof");
    let n: i32 = eof.split_whitespace().last().unwrap().parse().unwrap();
    assert!(n <= 0, "clean EOF expected, got: {}", eof);
    let _ = std::fs::remove_file("wide_fscanf_test.txt");
}

// ---------------------------------------------------------------------------
// §6.16.8 string methods: atoi, atohex, atooct, atobin with wide inputs
// ---------------------------------------------------------------------------

const STRING_METHODS_WIDE: &str = r#"
module tb;
  int  si;
  real rv;
  initial begin
    // 50-digit decimal (>= 2^64) wraps to low-32 signed int
    si = "12345678901234567890123456789012345678901234567890".atoi();
    $display("NOTE: atoi50 %0d", si);
    // 20-digit (>= 2^63) wraps to low-32
    si = "12345678901234567890".atoi();
    $display("NOTE: atoi20 %0d", si);
    // 2^63-1 → low 32 = 0xFFFFFFFF = -1
    si = "9223372036854775807".atoi();
    $display("NOTE: atoi63 %0d", si);
    // atohex 16-hex = deadbeefdeadbeef → low 32 = deadbeef = -559038737
    si = "deadbeefdeadbeef".atohex();
    $display("NOTE: atohex16 %0d", si);
    // atohex 32-hex → low 32 = 89abcdef = -1985872337
    si = "0123456789abcdef0123456789abcdef".atohex();
    $display("NOTE: atohex32 %0d", si);
    // atooct 37777777777 = 0xFFFFFFFF → low 32 = 0xFFFFFFFF = -1
    si = "37777777777".atooct();
    $display("NOTE: atooct %0d", si);
    // atobin 32 ones → 0xFFFFFFFF = -1
    si = "11111111111111111111111111111111".atobin();
    $display("NOTE: atobin %0d", si);
    // atoreal negative
    rv = "-2.71828".atoreal();
    $display("NOTE: atoreal %0.5f", rv);
    $finish;
  end
endmodule
"#;

#[test]
fn string_atoi_wide_wraps_to_low32() {
    // §6.16.8: atoi returns a 32-bit signed int — wide decimals wrap.
    let sim = simulate(STRING_METHODS_WIDE, 100).expect("simulate failed");
    // 50-digit mod 2^32 = 0xCE3F0AD2 = -834729262 signed
    assert_eq!(out(&sim, "atoi50"), "NOTE: atoi50 -834729262");
    // 20-digit mod 2^32 = 0xEB1F0AD2 = -350287150 signed
    assert_eq!(out(&sim, "atoi20"), "NOTE: atoi20 -350287150");
    // 2^63-1 = 0x7FFFFFFFFFFFFFFF → low 32 = 0xFFFFFFFF = -1
    assert_eq!(out(&sim, "atoi63"), "NOTE: atoi63 -1");
}

#[test]
fn string_atohex_atooct_atobin_wide() {
    let sim = simulate(STRING_METHODS_WIDE, 100).expect("simulate failed");
    assert_eq!(out(&sim, "atohex16"), "NOTE: atohex16 -559038737");
    assert_eq!(out(&sim, "atohex32"), "NOTE: atohex32 -1985229329");
    assert_eq!(out(&sim, "atooct"), "NOTE: atooct -1");
    assert_eq!(out(&sim, "atobin"), "NOTE: atobin -1");
    assert_eq!(out(&sim, "atoreal"), "NOTE: atoreal -2.71828");
}