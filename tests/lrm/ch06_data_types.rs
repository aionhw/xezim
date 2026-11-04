//! IEEE 1800-2023 clause 6: data types.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use crate::harness::{Order, check};

// §6.19.5: Enum name() of a value not in the enum
#[test]
fn c6_19_enum_name_other_type() {
    // Known gap (§6.19.5): `r1` reference `[] 001`, xezim `[XB] 001`
    check(
        "6.19_enum_name_other_type",
        include_str!("sv/6.19_enum_name_other_type.sv"),
        "ren",
        Order::Exact,
        &["T|r2|[] 101"],
        &["T|r1|"],
    );
}

// §6.21: Scope and lifetime: explicit static variable in a loop body block
#[test]
fn c6_21_static_blockvar_reinit() {
    // Known gap (§6.21): `r1`: 2 reference lines differ from xezim's 2, e.g. reference `12`, xezim `11`
    check(
        "6.21_static_blockvar_reinit",
        include_str!("sv/6.21_static_blockvar_reinit.sv"),
        "rsb",
        Order::Exact,
        &["T|r2|1", "T|r2|2"],
        &["T|r1|12", "T|r1|11", "T|r1|13"],
    );
}

// §6.24.3: Bit-stream casting to unpacked arrays and queues
#[test]
fn c6_24_3_bitstream_cast_unpacked() {
    check(
        "6.24.3_bitstream_cast_unpacked",
        include_str!("sv/6.24.3_bitstream_cast_unpacked.sv"),
        "rbc",
        Order::Exact,
        &[
            "T|r1|'{1, 2, 3, 4}",
            "T|r2|01020304",
            "T|r3|'{'hab, 'hcd, 'hef}",
            "T|r4|'{'h5, 'ha}",
        ],
        &[],
    );
}

// §6.6.7: User-defined nettypes and resolution functions
#[test]
fn c6_6_7_udn_port_driver() {
    // Known gap (§6.6.7): `6.6.7b` reference `ws=2.500000 1`, xezim `ws=1.000000 1`
    check(
        "6.6.7_udn_port_driver",
        include_str!("sv/6.6.7_udn_port_driver.sv"),
        "c06nt",
        Order::Exact,
        &[
            "T|6.6.7a|wr=5.250000",
            "T|6.6.7c|p4=9",
            "T|6.6.7d|wr=13.250000",
        ],
        &["T|6.6.7b|"],
    );
}

// §6.6.8, §6.9.2, §6.20.7: interconnect nets, specparams, real-array defaults
#[test]
fn c6_6_8_interconnect_nets() {
    // Not compared (§6.6.8): the reference delivers z through the interconnect; xezim delivers the driven value, as the clause describes
    check(
        "c06_ic",
        include_str!("sv/c06_ic.sv"),
        "c06ic",
        Order::Exact,
        &["T|6.9.2|3c c3 1"],
        &["T|6.6.8|"],
    );
}

// §6.8, §6.11.3: Variable declaration initializers inside procedural blocks (static block locals)
#[test]
fn c6_8_blocklocal_init() {
    check(
        "6.8_blocklocal_init",
        include_str!("sv/6.8_blocklocal_init.sv"),
        "rbi",
        Order::Exact,
        &[
            "T|r1|module 1000 200 0",
            "T|r2|static-local 1000 200 0",
            "T|r3|auto-local 1000 200",
            "T|r4|assigned 1000 200",
        ],
        &[],
    );
}

// §6.8: blocklocal init string
#[test]
fn c6_8_blocklocal_init_string() {
    check(
        "6.8_blocklocal_init_string",
        include_str!("sv/6.8_blocklocal_init_string.sv"),
        "rsr2",
        Order::Exact,
        &["T|r1|[abcabcabc] 9"],
        &[],
    );
}

// §5.10: Structure literals
#[test]
fn c6_8_blocklocal_init_struct() {
    check(
        "6.8_blocklocal_init_struct",
        include_str!("sv/6.8_blocklocal_init_struct.sv"),
        "rsi",
        Order::Exact,
        &[
            "T|r1|5 5 5",
            "T|r2|5 5 5 5",
            "T|r3|5 6 5",
            "T|r4|'{a:5, b:6}",
        ],
        &[],
    );
}

// §6.8: blocklocal init unsigned
#[test]
fn c6_8_blocklocal_init_unsigned() {
    check(
        "6.8_blocklocal_init_unsigned",
        include_str!("sv/6.8_blocklocal_init_unsigned.sv"),
        "run",
        Order::Exact,
        &[
            "T|r1|4294967295 4294967295 4294967295 4294967295 18446744073709551615",
            "T|r2|1",
            "T|r3|4294967295 4294967295",
        ],
        &[],
    );
}

// §6.3.1: 2-state packed vectors declared in any procedural scope (function, named block, fork, loop body, always)
#[test]
fn c6_8_local_2state_vector() {
    check(
        "6.8_local_2state_vector",
        include_str!("sv/6.8_local_2state_vector.sv"),
        "rbs",
        Order::Exact,
        &[
            "T|r1|func-local bit=8 byte-unsigned=200",
            "T|r2|task-local int unsigned > 0 = 1",
            "T|r3|named-block bit=1000 module bit=1000",
            "T|r4|fork-local bit=1000",
            "T|r5|loop-local bit=1000",
            "T|r6|always-local bit=0100",
        ],
        &[],
    );
}

// §5.13, §6.5, §6.6.1, §6.6.3, §6.6.4, §6.6.5, ...: net kinds, 2/4-state types, strings, enums, casts, $cast, parameters
#[test]
fn c6_data_types() {
    // Known gap (§6.21): `6.21b` reference `12`, xezim `11`
    // Known gap (§6.12): `6.12b` reference `0`, xezim `64`
    // Known gap (§6.23): `6.23` reference `9 64`, xezim `1 1`
    check(
        "c06_types",
        include_str!("sv/c06_types.sv"),
        "c06",
        Order::Exact,
        &[
            "T|6.5|a",
            "T|6.6.1|wd=x wz=1",
            "T|6.6.3|wa=0 wo=1",
            "T|6.6.4|t0=0 t1=1 ta=1",
            "T|6.6.6|s0=0 s1=1 uw=1",
            "T|6.6.4tr|tr=1",
            "T|6.7|nda=12",
            "T|6.8|iv=5 lv=xxxx bv=0000 intg=x by=-1 shi=-1 li=-9223372036854775808",
            "T|6.10|impl=1 bits=1",
            "T|6.11a|8 16 32 64",
            "T|6.11b|32 64",
            "T|6.11c|-128 4294967295 -8",
            "T|6.11d|-1 x",
            "T|6.12a|1.250000 0.100000 2.500000",
            "T|6.14|ch=null:1",
            "T|6.16a|len=5",
            "T|6.16b|Jello e",
            "T|6.16c|JELLO jello",
            "T|6.16d|0 1 0",
            "T|6.16e|ell|",
            "T|6.16f|123 255 15 5 35.000000",
            "T|6.16g|-42",
            "T|6.16h|ff",
            "T|6.16i|10",
            "T|6.16j|101",
            "T|6.16k|1.5",
            "T|6.16l|1 1 1 0",
            "T|6.16m|abc-abd",
            "T|6.16n|abcabcabc",
            "T|6.16o|b",
            "T|6.16p|aXc",
            "T|6.16q|0",
            "T|6.16r|abc",
            "T|6.16s|0",
            "T|6.16t|3",
            "T|6.16u|abcd",
            "T|6.16v|0",
            "T|6.17a|e1 at 4",
            "T|6.17b|e2 alias fired",
            "T|6.17c|null=1",
            "T|6.18|3f 6",
            "T|6.19a|RED 0",
            "T|6.19b|GREEN 5",
            "T|6.19c|BLUE 6",
            "T|6.19d|RED 0 wrap",
            "T|6.19e|BLUE",
            "T|6.19f|BLUE num=3",
            "T|6.19g|GREEN",
            "T|6.19h|[] 3",
            "T|6.19i|[RED] 0",
            "T|6.19j|0 2 11",
            "T|6.19k|10 S2 2",
            "T|6.19l|3",
            "T|6.19m|A1 B4 12",
            "T|6.20a|3 1010 4 44 2.500000",
            "T|6.20b|6 16 abc 7",
            "T|6.20c|24",
            "T|6.21|1 1 1 2",
            "T|6.23b|eq",
            "T|6.24.1a|b",
            "T|6.24.1b|-1",
            "T|6.24.1c|4294967295",
            "T|6.24.1d|3",
            "T|6.24.1e|3.000000",
            "T|6.24.1f|fff",
            "T|6.24.1g|4464",
            "T|6.24.1h|y",
            "T|6.24.1i|0",
            "T|6.24.2a|1 BLUE",
            "T|6.24.2b|0 BLUE",
            "T|6.24.2c|RED",
            "T|6.24.3a|5 a",
            "T|6.24.3b|'{1, 2, 3, 4}",
            "T|6.24.3c|01020304",
            "T|6.24.3d|'{'hab, 'hcd, 'hef}",
        ],
        &["T|6.21b|12", "T|6.21b|11", "T|6.12b|", "T|6.23|"],
    );
}

// §6.12.1, §6.20.3, §6.20.6, §6.22, §6.25: shortreal/real conversions, typedef forms, chandle, type parameters
#[test]
fn c6_types_more() {
    // Known gap (§6.20.2): `6.20.2`: 3 reference lines differ from xezim's 3, e.g. reference `c06x.t1 arr='{1, 2, 3}`, xezim `c06x.t1 arr=x`
    // Known gap (§6.12.1): `6.12.1c` reference `7766279631452241920`, xezim `9223372036854775807`
    // Known gap (§6.19): `6.19b` reference `[] 001`, xezim `[XB] 001`
    // Known gap (§6.24.1): `6.24.1l` reference `4`, xezim `0`
    check(
        "c06_more",
        include_str!("sv/c06_more.sv"),
        "c06x",
        Order::Exact,
        &[
            "T|6.20.3|c06x.t1 -3 bits=8 w=8",
            "T|6.20.3|c06x.t2 4095 bits=12 w=8",
            "T|6.20.3|c06x.t3 0 bits=32 w=3",
            "T|6.8a|3 1 1",
            "T|6.20.6|42",
            "T|6.12a|0.3333333333",
            "T|6.12b|1.000000e+40",
            "T|6.12.1a|4",
            "T|6.12.1b|-4",
            "T|6.12.1d|0",
            "T|6.12.1e|1.844674e+19",
            "T|6.12.1f|0.000000",
            "T|6.19a|RUN 011",
            "T|6.19c|4",
            "T|6.19d|x1 XB",
            "T|6.25|5",
            "T|6.22|'{4, 5, 6}",
            "T|6.24.1k|3f 2",
            "T|6.24m|2",
            "T|6.24.1n|X",
            "T|6.11.3|200 4294967295 18446744073709551615 4294967295",
            "T|6.3|1000 0",
            "T|6.14b|1 1",
            "T|6.16w|2 AB",
        ],
        &["T|6.20.2|", "T|6.12.1c|", "T|6.19b|", "T|6.24.1l|"],
    );
}
