//! IEEE 1800-2023 clause 21: input/output system tasks.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use crate::harness::{Order, check, check_files};

// §21.2.1.7: %p of an unpacked array with a descending range
#[test]
fn c21_2_1_7_p_descending_range() {
    check(
        "21.2.1.7_p_descending_range",
        include_str!("sv/21.2.1.7_p_descending_range.sv"),
        "rpd",
        Order::Exact,
        &["T|r1|'{2, 1, 0} '{0, 1, 2}"],
        &[],
    );
}

// §21.4: $readmemh with @address records into dynamic and fixed arrays
#[test]
fn c21_4_readmem_address_records() {
    check_files(
        "f_readmemdyn2",
        include_str!("sv/f_readmemdyn2.sv"),
        "frd2",
        Order::Exact,
        &["T|21.4|dyn='{17, 0, 26, 0} fixed='{'h11, 'hXX, 'h1a, 'hXX}"],
        &[],
        &[],
        &[],
    );
}

// §21.2.1, §21.2.1.8, §21.2.1.7, §21.3.1, §21.3.2, §21.3.3, ...: display formats, file I/O, readmem/writemem, $sscanf, VCD
#[test]
fn c21_input_output() {
    // Not compared: %u/%z raw formats and the %l library-binding text (implementation-defined).
    // Known gap (§21.2.1): `21.2.1p` reference `StX St1`, xezim `St1 St1`
    // Known gap (§21.2.1): `21.2.1s` reference `1 x {zz}`, xezim `1 x "zz"`
    // Known gap (§21.2.1): `21.2.1J` reference `2.500000          3`, xezim `2.500000                  2.5`
    // Known gap (§21.2.1): `21.2.1L` reference `         3`, xezim `             3.14159`
    // Known gap (§21.2.1): `21.2.1M` reference `00000000000000000000000000000011`, xezim `0100000000001001001000011111100111110000000110111000011001101110`
    // Known gap (§21.2.1): `21.2.1T` reference `A|`, xezim `  A|`
    // Known gap (§21.3.5): `21.3.5a` reference `17`, xezim `18`
    // Known gap (§21.3.7): `21.3.7` reference `ec_nonzero=1`, xezim `ec_nonzero=0`
    // Known gap (§21.3.4): `21.3.4o` reference `-1`, xezim `0`
    // Known gap (§21.4): `21.4e` reference `'{17, 0, 26, 43, 15, 4}`, xezim `'{X, 0, 0}`
    // Known gap (§21.7): `21.7` reference `vcd defs=1 vars>5=0 dumponoff=1 times>2=1`, xezim `vcd defs=1 vars>5=1 dumponoff=1 times>2=1`
    check_files(
        "c21_io",
        include_str!("sv/c21_io.sv"),
        "c21",
        Order::Exact,
        &[
            "T|21.2.1a|a5 a5 245 10100101 165 165",
            "T|21.2.1b|[a5] [245] [10100101] [165]",
            "T|21.2.1c|[  165] [165  ] [  165] [a5]",
            "T|21.2.1d|[  -5] [-5] [fb]",
            "T|21.2.1e|[        -42] [-42] [     -42] [-42     |",
            "T|21.2.1f|axz5 12xXZ5 1010xxxxzzzz0101",
            "T|21.2.1g|[    X] [X]",
            "T|21.2.1h|[    x] [    z]",
            "T|21.2.1i|[  X] [  Z]",
            "T|21.2.1j|3.141590e+00|3.141590|3.14159|     3.142|3.14e+00  |3.1",
            "T|21.2.1k|1e-10 1.23457e+08 0.0001",
            "T|21.2.1l|[hi] [   hi] [hi   ] [hi]",
            "T|21.2.1m|AC",
            "T|21.2.1n|c21",
            "T|21.2.1o|                   0",
            r#"T|21.2.1q|'{a:1, b:'hX, c:"zz"}"#,
            "T|21.2.1r|E1 165",
            "T|21.2.1v| A",
            "T|21.2.1w|AB|",
            "T|21.2.1x|18446744073709551615",
            "T|21.2.1y|1237940039285380274899124224 -5",
            "T|21.2.1z|0040000000000000000000000",
            r#"T|21.2.1B|%|\|""#,
            "T|21.2.1C|          5x  3",
            "T|21.2.1D|1 000",
            "T|21.2.1E|x",
            "T|21.2.1F|xa",
            "T|21.2.1G|5x",
            "T|21.2.1H|5X",
            "T|21.2.1I|Zz",
            "T|21.2.1K|1.234568e+04",
            "T|21.2.1N|-1.50|-1.230e-03|",
            "T|21.2.1O|ff ff",
            "T|21.2.1P|      5000|5000      |",
            "T|21.2.1Q|",
            "T|21.2.1R|          1",
            "T|21.2.1S|4142",
            r#"T|21.2.1V|'{1, 2} '{"k":5}"#,
            "T|21.2.1W|165   -5         -42",
            "T|21.2.1X|a5 ffffffd6",
            "T|21.2.1Y|0101",
            "T|21.2.1Z|00000000011",
            "T|21.2.2|w1 w2",
            "T|21.3.1a|fd_nonzero=1",
            "T|21.3.4a|9 [line1 10",
            "T|21.3.4b|l",
            "T|21.3.4c|2 line2 000000ab",
            "T|21.3.4d|[line3",
            "T|21.3.4e|0 eof=1",
            "T|21.3.4f|-1",
            "T|21.3.5b|[line1 10",
            "T|21.3.5c|n",
            "T|21.3.5d|23",
            "T|21.3.1b|0",
            "T|21.3.1c|mcd=1",
            "T|21.3.1d|to mcd and stdout",
            "T|21.3.1e|lines=5",
            "T|21.3.4g|2 6c69",
            "T|21.3.3a|7-x",
            "T|21.3.3b|  2.2|abc",
            "T|21.3.3c|[a          5b]",
            "T|21.3.3d|[000000ff]",
            "T|21.3.4h|4 12 171 3.500000 word",
            "T|21.3.4i|2 11 63",
            "T|21.3.4j|0",
            "T|21.3.4k|2 42 xyz",
            "T|21.3.4l|1 5",
            "T|21.3.4m|1 7",
            "T|21.3.4n|1 65",
            "T|21.4a|00000011 0000dead 0000001a 0000002b xxxxxxxf 00000004",
            "T|21.4b|'{'ha5, 'h3, 'h0, 'hZf}",
            "T|21.4c|'{'h0, 'ha5, 'h3, 'h0}",
            "T|21.4d|'{'hZf, 'h0, 'h3, 'ha5}",
            "T|21.4.1|'{'hZf, 'h0, 'h3, 'ha5}",
            "T|21.4.1b|'{'hXX, 'h0, 'h3, 'hXX}",
            "T|21.6a|0 1",
            "T|21.6b|1",
            "T|21.2.3|fmon v8=a5",
            "T|21.2.3|fmon v8=11",
        ],
        &[
            "T|21.2.1t|",
            "T|21.2.1u|",
            "T|21.2.1A|",
            "T|21.2.1p|",
            "T|21.2.1s|",
            "T|21.2.1J|",
            "T|21.2.1L|",
            "T|21.2.1M|",
            "T|21.2.1T|",
            "T|21.3.5a|",
            "T|21.3.7|",
            "T|21.3.4o|",
            "T|21.4e|",
            "T|21.7|",
        ],
        &["+FOO", "+NUM=5", "--wave"],
        &[],
    );
}
