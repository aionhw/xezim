//! §21.2.1.2 / §6.12.2: an integral format letter (`%d`, `%h`, `%o`, `%b`,
//! `%c`) applied to a REAL argument prints the real converted to an integer
//! (rounded to the nearest, ties away from zero), at the width of `integer`
//! (32 bits), so `%0h` of a negative real is its 32-bit two's complement and
//! a bare `%d` pads to the 32-bit default field width. xezim printed the raw
//! f64 for `%d` and the IEEE-754 bit pattern for `%h`/`%b`/`%o` (#889,
//! reported by Ganesh-TNL). Expected values are the reference simulator's
//! except the decimal rendering of a NEGATIVE real: the reference prints the
//! 32-bit pattern unsigned (`4294952144`), the majority of simulators and the
//! issue print the signed value (`-15152`); xezim follows the majority, and
//! for the same reason `%d` pads to 11 (signed int), where the reference pads
//! to 10.

use xezim::simulate;

fn lines(src: &str, max_time: u64) -> Vec<String> {
    let sim = simulate(src, max_time).expect("design must run");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

fn expect(src: &str, max_time: u64, want: &[&str]) {
    let got = lines(src, max_time);
    let want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
    assert_eq!(got, want, "\n got: {got:#?}\nwant: {want:#?}");
}

#[test]
fn integral_format_letters_convert_a_real_argument() {
    let src = r#"
module tb;
  real r = 1000000.00 / 66;   // 15151.5151...
  real rn = -1000000.00 / 66;
  real rz = 0.0;
  int  i_ref = 1000000.00 / 66;
  string s;
  // A compiled body (the loop admits it): the compiled format template
  // converts too.
  function automatic void show(real v);
    for (int i = 0; i < 1; i++) $display("T|CF=%0d %0h %4d|", v, v, v);
  endfunction
  initial begin
    $display("T|D=%0d %0h %0b %0o", r, r, r, r);
    $display("T|REF=%0d %0h %0b %0o", i_ref, i_ref, i_ref, i_ref);
    $display("T|EXPR=%0d %0h", 1000000.00 / 66, 1000000.00 / 66);
    $display("T|W=%d|%h|%b|%o|", r, r, 5.0, 5.0);
    $display("T|WN=%3d|%8d|%03d|%-8d|%-10h|", r, r, r, r, r);
    $display("T|NEG=%0d %0h %0b %0o", rn, rn, rn, rn);
    $display("T|NEGREF=%0d %0h %0b %0o", -i_ref, -i_ref, -i_ref, -i_ref);
    $display("T|Z=%0d %0h", rz, rz);
    $display("T|HALF=%0d %0d %0d %0d %0d", 2.5, -2.5, 0.5, -0.5, 0.4);
    $display("T|C=%c %0d %0d %0d", 65.4, shortreal'(2.5), $sqrt(2.0), $itor(7));
    $display("T|BIG=%0d %0d", 1e10, -1.0);
    s = $sformatf("%0d", r);
    $display("T|SF=%s", s);
    $write("T|WR=%0d\n", r);
    show(r);
    // Real letters and the default (no-spec) form stay real.
    $display("T|F=%f %0.2f", r, r);
    $finish;
  end
endmodule
"#;
    expect(
        src,
        10,
        &[
            "T|D=15152 3b30 11101100110000 35460",
            "T|REF=15152 3b30 11101100110000 35460",
            "T|EXPR=15152 3b30",
            "T|W=      15152|00003b30|00000000000000000000000000000101|00000000005|",
            "T|WN=15152|   15152|15152|15152   |3b30      |",
            "T|NEG=-15152 ffffc4d0 11111111111111111100010011010000 37777742320",
            "T|NEGREF=-15152 ffffc4d0 11111111111111111100010011010000 37777742320",
            "T|Z=0 0",
            "T|HALF=3 -3 1 -1 0",
            "T|C=A 3 1 7",
            "T|BIG=1410065408 -1",
            "T|SF=15152",
            "T|WR=15152\n",
            "T|CF=15152 3b30 15152|",
            "T|F=15151.515152 15151.52",
        ],
    );
}
