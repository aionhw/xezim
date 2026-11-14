//! Exported subroutines called from C with every formal type DPI carries
//! (#291): `chandle`, a `string` result, packed vectors wider than 64 bits,
//! the small C scalars, unpacked structs and arrays. Each used to be a stub
//! that returned 0 without running the subroutine. A formal DPI cannot carry
//! now makes the export loudly not callable, and a wide result stops the
//! run. Each bench prints `T|` lines; the C side prints through the exported
//! `put`, so the order is the simulator's. The expected values are the
//! reference simulator's.

use std::path::{Path, PathBuf};
use std::process::Command;

fn manifest_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Build `tests/dpi/<stem>.c` into a fresh shared library.
fn compile(stem: &str) -> PathBuf {
    let so = std::env::temp_dir().join(format!(
        "xezim_export_types_{}_{}.so",
        stem,
        std::process::id()
    ));
    let mut cc = Command::new(std::env::var_os("CC").unwrap_or_else(|| "clang".into()));
    cc.args(["-shared", "-fPIC"]);
    if cfg!(target_os = "macos") {
        // The exported subroutines resolve when xezim loads the library.
        cc.arg("-Wl,-undefined,dynamic_lookup");
    }
    let ok = cc
        .arg("-I")
        .arg(manifest_path("include"))
        .arg(manifest_path(&format!("tests/dpi/{stem}.c")))
        .arg("-o")
        .arg(&so)
        .status()
        .expect("failed to launch cc")
        .success();
    assert!(ok, "cc failed for {stem}.c");
    so
}

/// Run `tests/dpi/<stem>_test.sv` against `tests/dpi/<stem>.c`: whether it
/// exited 0, and its output.
fn run(stem: &str) -> (bool, String) {
    run_with(stem, &[])
}

/// `run` with extra command-line arguments.
fn run_with(stem: &str, extra: &[&str]) -> (bool, String) {
    let so = compile(stem);
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(extra)
        .arg("--dpi-lib")
        .arg(&so)
        .args(["--no-cache", "--max-time", "1us", "-s", "top"])
        .arg(manifest_path(&format!("tests/dpi/{stem}_test.sv")))
        .output()
        .expect("failed to run xezim");
    let _ = std::fs::remove_file(&so);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

fn tagged(text: &str) -> Vec<&str> {
    text.lines().filter(|l| l.starts_with("T|")).collect()
}

/// #291 as reported: a chandle formal, a string result and a 128-bit vector
/// (through an output: a wide result is not legal) all run, with their
/// values; nothing reports a type not modelled.
#[test]
fn export_issue_291_reproducer() {
    let (ok, text) = run("export_issue291");
    assert!(ok, "{text}");
    assert_eq!(
        tagged(&text),
        [
            "T|ok_add(2,3)=5",
            "T|add2_with_h(2,3,0)=5",
            "T|add2_with_h(2,3,&x)=105",
            "T|str_ret()=hello",
            "T|wide o=fbbbbbbb/f0000000 cccccccc/00000000 dddddddd/00000000 eeeeeeee/00000000",
            "T|HITS=5 of 5",
        ],
        "{text}"
    );
    assert!(!text.contains("not modeled"), "{text}");
    assert!(!text.contains("NOT callable"), "{text}");
}

/// The type table of §35.5.6 / Annex H, each type as an input, an output and
/// an inout where the standard allows it, plus an exported task.
#[test]
fn export_formal_types_from_c() {
    let (ok, text) = run("export_types");
    assert!(ok, "{text}");
    assert_eq!(
        tagged(&text),
        [
            "T|f_ch r=1 o=1 io=1",
            "T|f_str r=r:abc o=abc-out io=base+abc",
            "T|f_l a=x0123456789abcdef b=f000000030000ffxxaaaa5555 io=3fffffffffffffffff",
            "T|f_l o=01234567/00000000 89abcdef/00000000 0000ffff/000000ff aaaa5555/00000000 io=00000040/00000000 00000000/00000000 00000000/00000000",
            "T|f_b a=f555566663333444411112222 io=00000003000000020000000100000000",
            "T|f_b o=00000002 33334444 11112222 io=fffffffc fffffffd fffffffe ffffffff",
            "T|f_sc a=-5 b=-300 c=1.25 d=1 e=x",
            "T|f_sc r=-4 oa=-6 ob=-600 oc=2.50 od=0 oe=2",
            "T|f_rb 0 1 f_rl 0 1 3 2 f_rf 1.75",
            "T|f_us a=21 4 1.5 hi 1",
            "T|f_us m=9 3Z 2a9abcdef012345678 1 1 2 3",
            "T|f_us r=25 o=42 -3 2.25 struct-out 1 m=10 0000005a/00000000 00000015 6543210f edcba987 3 1 77 3",
            "T|f_ua a=1 2 3 4",
            "T|f_ua io[3]=13 io[0]=10 lv=ab cX sa=100/1 200/2",
            "T|f_ua o=11 12 13 io=-1 11 12 33 so=5/6 300/9",
            "T|f_2d r=7 r2=3.5 4.0",
            "T|t_w t=5 w=00000077000000030000000200000001",
            "T|t_w o=119",
            "T|t_w t=10 w=00000077000000030000000200000001",
            "T|t_w null o=-1",
            "T|after c_tmain t=10",
        ],
        "{text}"
    );
}

/// Unpacked arrays in C order: the lowest index first in every dimension.
#[test]
fn export_unpacked_array_index_order() {
    let (ok, text) = run("export_array_order");
    assert!(ok, "{text}");
    assert_eq!(
        tagged(&text),
        [
            "T|a[5]=3 a[2]=0 b[2]=10 b[5]=13",
            "T|c[1][0]=23 c[1][2]=25 c[0][0]=20 c[0][2]=22",
            "T|o=11 22 33",
        ],
        "{text}"
    );
}

/// A formal DPI cannot carry: said at startup, an error at each call from
/// C, and the function never runs. The error is counted: with
/// `--error-exit` the run exits 1.
#[test]
fn export_with_an_unmodelled_formal_fails_loudly() {
    let (ok, _) = run_with("export_unmodelled", &["--error-exit"]);
    assert!(!ok, "the call-site error is counted as an error");
    let (ok, text) = run("export_unmodelled");
    assert!(ok, "{text}");
    assert!(
        text.contains(
            "[DPI] exported subroutine 'f_ev' is NOT callable from C: formal 'e' has type \
             event, which DPI does not carry"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "[xezim][error] DPI export 'f_ev' cannot be called from C: formal 'e' has type \
             event, which DPI does not carry"
        ),
        "{text}"
    );
    assert_eq!(tagged(&text), ["T|f_ev=0 ok=5", "T|hits=1"], "{text}");
}

/// §35.5.5: a 128-bit result is not a legal DPI export; the run stops.
#[test]
fn export_with_a_wide_result_is_rejected() {
    let (ok, text) = run("export_wide_result");
    assert!(!ok, "{text}");
    assert!(
        text.contains(
            "exported function 'wide' returns a packed vector of 128 bits (a DPI function \
             result is at most 64 bits wide; pass it through an output argument), which is \
             not a legal DPI result type (IEEE 1800 clause 35.5.5)"
        ),
        "{text}"
    );
}

/// #324: a real-valued expression crossing a DPI integral-scalar boundary in
/// either direction is VALUE-converted (rounding to nearest, ties away from
/// zero, §5.2/§6.12) instead of bit-reinterpreting its IEEE-754 pattern —
/// covering scalar `bit`/`byte`/`shortint`/`int`/`longint` formals and the
/// packed-vector `time`/`integer`/`logic`/`bit [N]` formals (svLogicVecVal* /
/// svBitVecVal*). Before the fix, returning `100ns / 1ns` (real 100.0) through
/// a `longint` fetched the C side `0x4059000000000000` (the bits of double
/// 100.0) instead of 100, a real actual into a C `int` formal passed 0, and a
/// real actual into a `time` formal passed the f64 bits (0x400E...).
#[test]
fn exported_integral_returns_value_convert_a_real() {
    let (ok, text) = run("real_return");
    assert!(ok, "{text}");
    assert_eq!(
        tagged(&text),
        ["T|T4", "T|I4", "T|B4", "T|100 4 -4"],
        "{text}"
    );
    assert!(text.contains("PASS: get_drain_time returned 100"), "{text}");
    assert!(text.contains("PASS: c_take_int(3.75) == 4"), "{text}");
}

/// #324 cousin: scalar `bit` DPI formals carry only their single meaningful
/// bit (an svBit is a 1-bit value, not the low 8 bits of an i32 slot), on every
/// direction (input lays bit 0 down, inout/out mask C's written byte to bit 0);
/// and a real->longint conversion at the DPI boundary wraps modulo 2^64
/// (two's complement) instead of saturating: real 2^63 -> INT64_MIN, 2^64 -> 0.
#[test]
fn scalar_bit_width_marshals_one_bit_and_wraps_longint() {
    let (ok, text) = run("scalar_bit_width");
    assert!(ok, "{text}");
    // C `printf` (the B/L marshalling lines) flushes on exit, `$display`
    // (IO/OUT) flushes immediately, so their relative order is not stable;
    // assert each group separately.
    let marshalled: Vec<String> = tagged(&text)
        .into_iter()
        .filter(|l| l.starts_with("T|B ") || l.starts_with("T|L "))
        .map(|l| l.to_string())
        .collect();
    let wb: Vec<String> = tagged(&text)
        .into_iter()
        .filter(|l| l.starts_with("T|IO ") || l.starts_with("T|OUT "))
        .map(|l| l.to_string())
        .collect();
    assert_eq!(
        marshalled,
        [
            "T|B 0 0 1 0",              // bit(2)=0, bit(1.75)=0, bit(-0.5)=1, bit(300)=0
            "T|L -9223372036854775808", // real 2^63 wraps to INT64_MIN
            "T|L 0",                    // real 2^64 wraps to 0
        ],
        "{text}"
    );
    assert_eq!(wb, ["T|IO 1", "T|OUT 1"], "{text}");
}
