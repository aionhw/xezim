//! IEEE 1800-2023 clause 11: operators and expressions.

use crate::harness::{Order, check, check_error};

// §11.4.12.1: string replication
#[test]
fn c11_4_12_1_string_replication() {
    check(
        "r_strrep",
        include_str!("sv/r_strrep.sv"),
        "rsr",
        Order::Exact,
        &["T|r1|[abcabcabc] 9", "T|r2|[abcabc]", "T|r3|[xyxy]"],
        &[],
    );
}

// §11.4.3: Unary minus on an unsigned operand, self-determined ($display)
#[test]
fn c11_4_3_unary_minus_unsigned_display() {
    // Known gap (§11.4.3): `r1` reference `13 13`, xezim `-3 -3`
    // Known gap (§11.4.3): `r4` reference `255`, xezim `-1`
    check(
        "11.4.3_unary_minus_unsigned_display",
        include_str!("sv/11.4.3_unary_minus_unsigned_display.sv"),
        "rng",
        Order::Exact,
        &["T|r2|1", "T|r3|253", "T|r5|4"],
        &["T|r1|", "T|r4|"],
    );
}

// §11.3.2, §11.6.1, §11.8.1: expression bit lengths, signedness, operand sizing
#[test]
fn c11_6_expression_bit_lengths() {
    // Known gap (§11.4.14.4): `11.4.14.4` reference `0000000100000002`, xezim `X000000000000000`
    // Known gap (§11.4.3): `11.4.3l` reference `255 -1.500000`, xezim `-1 -1.500000`
    check(
        "c11_more",
        include_str!("sv/c11_more.sv"),
        "c11x",
        Order::Exact,
        &[
            "T|11.3.2a|14 20 64",
            "T|11.3.2b|4 8",
            "T|11.3.2c|00000000000000000000000000000001 1110",
            "T|11.3.2d|2",
            "T|11.3.2e|1",
            "T|11.3.2f|1",
            "T|11.6.1a|300",
            "T|11.6.1b|150",
            "T|11.6.1c|150",
            "T|11.6.1d|300",
            "T|11.6.1e|wide",
            "T|11.6.1f|wide",
            "T|11.6.1g|44",
            "T|11.6.1h|44",
            "T|11.6.1i|f0",
            "T|11.6.1j|00",
            "T|11.6.1k|3c",
            "T|11.6.1l|3c",
            "T|11.6.1m|0f",
            "T|11.6.1n|f1",
            "T|11.6.1o|e1",
            "T|11.6.1p|1",
            "T|11.6.1q|0",
            "T|11.6.1r|0",
            "T|11.6.1s|0000000100000000",
            "T|11.6.1t|0000000100000000",
            "T|11.6.1u|0000010000000000",
            "T|11.6.1v|0000010000000000",
            "T|11.6.1w|2147483648",
            "T|11.6.1x|00c8",
            "T|11.6.1y|300",
            "T|11.8.1a|-10",
            "T|11.8.1b|446",
            "T|11.8.1c|-3",
            "T|11.8.1d|-1",
            "T|11.8.1e|-5",
            "T|11.8.1f|123",
            "T|11.8.1g|246",
            "T|11.8.1h|246",
            "T|11.8.1i|10",
            "T|11.8.1j|738",
            "T|11.8.1k|1",
            "T|11.8.1l|0",
            "T|11.8.1m|1",
            "T|11.8.1n|100",
            "T|11.8.1o|0",
            "T|11.8.1p|-8",
            "T|11.8.1q|-3 -1",
            "T|11.8.1r|-4",
            "T|11.8.1s|-4",
            "T|11.8.1t|4294967294",
            "T|11.8.1u|-2",
            "T|11.8.1v|0",
            "T|11.8.1w|-4",
            "T|11.8.1x|4",
            "T|11.8.1y|-4",
            "T|11.5.1g|adbe df",
            "T|11.5.1h|dead00ef",
            "T|11.5.1i|c0",
            "T|11.4.12.1|5",
            "T|11.4.11g|1xx0",
            "T|11.4.5c|1 0",
            "T|11.4.5d|1",
            "T|11.4.5e|1",
            "T|11.4.2c|xxxx",
        ],
        &["T|11.4.14.4|", "T|11.4.3l|"],
    );
}

// §11.9, §11.11: tagged union expressions, min:typ:max expressions
#[test]
fn c11_9_tagged_union_and_11_11_minmax() {
    check(
        "c11_misc",
        include_str!("sv/c11_misc.sv"),
        "c11m",
        Order::Exact,
        &[
            "T|11.11a|w=x",
            "T|11.11b|w=1",
            "T|11.11c|2",
            "T|11.9|5",
            "T|11.9b|none",
            "T|11.10.1|B",
            "T|11.10.2|1",
        ],
        &[],
    );
}

// §11.9: reading an inactive member of a tagged union is a run-time error
#[test]
fn c11_9_tagged_union_inactive_member() {
    check_error(
        "11.9_tagged_union_inactive_member",
        include_str!("sv/11.9_tagged_union_inactive_member.sv"),
        "tu_inactive",
        "inactive tagged union member 'a.Valid'",
    );
}

// §11.2.2, §11.3.6, §11.4.2, §11.4.3, §11.4.4, §11.4.5, ...: all operators, streaming, inside, conditional, concatenation
#[test]
fn c11_operators() {
    // Known gap (§11.4.3): `11.4.3d` reference `x -1 1`, xezim `0 -1 1`
    // Known gap (§11.4.3): `11.4.3i` reference `13`, xezim `-3`
    // Known gap (§11.4.11): `11.4.11c` reference `0.000000`, xezim `2.000000`
    // Known gap (§11.4.13): `11.4.13c` reference `x 1`, xezim `0 1`
    check(
        "c11_ops",
        include_str!("sv/c11_ops.sv"),
        "c11",
        Order::Exact,
        &[
            "T|11.4.2a|5 6 7",
            "T|11.4.2b|6",
            "T|11.4.3a|xxxx xxxx",
            "T|11.4.3b|-3 -1 1 -1",
            "T|11.4.3c|1024 -8 0",
            "T|11.4.3e|xxxx",
            "T|11.4.3f|-56",
            "T|11.4.3g|-56",
            "T|11.4.3h|1.414214 3.500000",
            "T|11.4.3j|14",
            "T|11.4.4a|x 1",
            "T|11.4.4b|0",
            "T|11.4.4c|1",
            "T|11.4.4d|1",
            "T|11.4.5a|x 1 0 1",
            "T|11.4.5b|0 x",
            "T|11.4.6a|1 0 x",
            "T|11.4.6b|0 x",
            "T|11.4.7a|x 1 1 x",
            "T|11.4.7b|0 1",
            "T|11.4.7c|1 x",
            "T|11.4.7d|0",
            "T|11.4.8a|01xx 01xx 10xx 10xx 11xx",
            "T|11.4.8b|0000",
            "T|11.4.9|1 x x 1 1 1 x",
            "T|11.4.10a|11111100 00111100 00100100",
            "T|11.4.10b|xxxx 0000",
            "T|11.4.10c|11100000",
            "T|11.4.10d|0",
            "T|11.4.10e|00000000",
            "T|11.4.11a|xx10",
            "T|11.4.11b|1111",
            "T|11.4.11d|3",
            "T|11.4.11e|xx",
            "T|11.4.12a|101001",
            "T|11.4.12b|101010",
            "T|11.4.12c|1010010100",
            "T|11.4.12d|4",
            "T|11.4.12e|abcd",
            "T|11.4.12f|xyxy",
            "T|11.4.12g|00001111",
            "T|11.4.12h|zzz",
            "T|11.4.13a|1 0 1",
            "T|11.4.13b|1",
            "T|11.4.13d|1",
            "T|11.4.13e|0",
            "T|11.4.13f|1",
            "T|11.4.14a|10100011",
            "T|11.4.14b|4321",
            "T|11.4.14c|3412",
            "T|11.4.14d|1234",
            "T|11.4.14e|01010011",
            "T|11.4.14f|'{4, 3, 2, 1}",
            "T|11.4.14g|04030201",
            "T|11.4.14h|ab cdef",
            "T|11.4.14i|10000000",
            "T|11.4.14j|'{10, 11}",
            "T|11.4.14k|ddccbbaa",
            "T|11.4.14l|f 123",
            "T|11.4.14m|'{16909060}",
            "T|11.4.14n|c3",
            "T|11.4.14o|01010000",
            "T|11.4.14p|'{-17, -66}",
            "T|11.5.1a|ab 79 bc d",
            "T|11.5.1b|X",
            "T|11.5.1c|X",
            "T|11.5.1d|x",
            "T|11.5.1e|ab b d",
            "T|11.5.1f|x",
            "T|11.6a|10",
            "T|11.6b|0",
            "T|11.6c|08",
            "T|11.6d|00",
            "T|11.6e|010e",
            "T|11.6f|8",
            "T|11.6g|1 12 4",
            "T|11.6h|16",
            "T|11.6.2|10000 0000",
            "T|11.6.2b|8000",
            "T|11.8a|11111101",
            "T|11.8b|00001110",
            "T|11.8c|11111110",
            "T|11.8d|-6",
            "T|11.8e|-1",
            "T|11.8f|15",
            "T|11.8g|0110",
            "T|11.8h|13",
            "T|11.8i|-2 -3",
            "T|11.8j|-3",
            "T|11.8k|1",
            "T|11.8l|-8",
            "T|11.8m|-8",
            "T|11.8n|8",
            "T|11.8o|0",
            "T|11.8.2a|1.500000",
            "T|11.8.2b|1.000000",
            "T|11.8.2c|16.000000",
            "T|11.8.2d|0.000000",
            "T|11.8.2e|4",
            "T|11.8.2f|0.000000",
            "T|11.8.2g|1",
            "T|11.12|9 1 0",
            "T|11.3.6|4",
            "T|11.3.6b|6 6",
            "T|11.4.3k|xxxx",
            "T|11.2.2|1",
            "T|11.2.2b|1",
            "T|11.10|1",
            "T|11.4.11f|3.000000",
        ],
        &["T|11.4.3d|", "T|11.4.3i|", "T|11.4.11c|", "T|11.4.13c|"],
    );
}
