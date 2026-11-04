//! IEEE 1800-2023 clause 7: aggregate data types.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use crate::harness::{Order, check};

// §7.10: Bounded queues (int q[$:3])
#[test]
fn c7_10_bounded_queue() {
    // Known gap (§7.10): `r2` reference `'{1, 2, 3, 4} 4`, xezim `'{1, 2, 3, 4, 5, 6} 6`
    // Known gap (§7.10): `r3` reference `'{1, 2, 3, 4}`, xezim `'{1, 2, 3, 4, 5, 6}`
    // Known gap (§7.10): `r4` reference `'{1, 2, 3, 4}`, xezim `'{1, 2, 3, 4, 5, 6}`
    check(
        "7.10_bounded_queue",
        include_str!("sv/7.10_bounded_queue.sv"),
        "rq",
        Order::Exact,
        &["T|r1|'{}"],
        &["T|r2|", "T|r3|", "T|r4|"],
    );
}

// §7.10: Queue slice with reversed bounds q[3:1]
#[test]
fn c7_10_queue_slice_bounds() {
    // Known gap (§7.10): `r2` reference `'{1, 2, 3, 4} 4`, xezim `'{1, 2, 3, 4, 5, 6} 6`
    // Known gap (§7.10): `r3` reference `'{1, 2, 3, 4}`, xezim `'{1, 2, 3, 4, 5, 6}`
    // Known gap (§7.10): `r4` reference `'{1, 2, 3, 4}`, xezim `'{1, 2, 3, 4, 5, 6}`
    check(
        "7.10_queue_slice_bounds",
        include_str!("sv/7.10_queue_slice_bounds.sv"),
        "rq",
        Order::Exact,
        &["T|r1|'{}"],
        &["T|r2|", "T|r3|", "T|r4|"],
    );
}

// §7.10: queue slices
#[test]
fn c7_10_queue_slices() {
    check(
        "r_qslice2",
        include_str!("sv/r_qslice2.sv"),
        "rqs",
        Order::Exact,
        &["T|r1|'{} size=0", "T|r2|iterations=3"],
        &[],
    );
}

// §7.2.2: Assigning to structures: member default values (int a = 3) for a variable declared in a procedural block
#[test]
fn c7_2_2_struct_member_default_blocklocal() {
    check(
        "7.2.2_struct_member_default_blocklocal",
        include_str!("sv/7.2.2_struct_member_default_blocklocal.sv"),
        "rsd2",
        Order::Exact,
        &["T|r1|3 4 / 3 4"],
        &[],
    );
}

// §7.3.2: Tagged union: reading the active member returns its stored value
#[test]
fn c7_3_2_tagged_union_active_member() {
    check(
        "7.3.2_tagged_union_active_member",
        include_str!("sv/7.3.2_tagged_union_active_member.sv"),
        "tu_active",
        Order::Exact,
        &["T|7.3.2|42"],
        &[],
    );
}

// §7.2: Structures: unpacked struct 2-state members default value
#[test]
fn c7_2_struct_2state_member_default() {
    check(
        "7.2_struct_2state_member_default",
        include_str!("sv/7.2_struct_2state_member_default.sv"),
        "rsd",
        Order::Exact,
        &["T|r1|a=0 b=5", "T|r2|a=0 b=x", "T|r3|a=0 c=0 d=0"],
        &[],
    );
}

// §7.2.1, §7.2.2, §7.3, §7.4.1, §7.4.2, §7.4.5, ...: structs, unions, packed/unpacked/dynamic/assoc arrays, queues, array methods
#[test]
fn c7_aggregates() {
    // Not compared: element order of unique()/unique_index() (left unspecified by §7.12.1).
    // Known gap (§7.4.6): `7.4.6a` reference `0`, xezim `x`
    // Known gap (§7.5): `7.5f` reference `'{'{0, 0, 0}, '{9}}`, xezim `'{0, 9}`
    // Known gap (§7.8): `7.8h` reference `next-of-nonexist=c`, xezim `next-of-nonexist=bb`
    // Known gap (§7.8): `7.8o` reference `-1 -5`, xezim `1 -5`
    // Known gap (§7.10): `7.10l` reference `'{1, 2, 3, 4}`, xezim `'{1, 2, 3, 4, 5, 6}`
    // Known gap (§7.10): `7.10m` reference `'{1, 2, 3, 4}`, xezim `'{1, 2, 3, 4, 5, 6}`
    // Known gap (§7.10): `7.10n` reference `'{1, 2, 3, 4}`, xezim `'{1, 2, 3, 4, 5, 6}`
    // Known gap (§7.12.3): `7.12.3f` reference `x`, xezim `3`
    // Known gap (§7.12.3): `7.12.3i` reference `10`, xezim `0`
    check(
        "c07_agg",
        include_str!("sv/c07_agg.sv"),
        "c07",
        Order::Exact,
        &[
            "T|7.2.1a|a=0 b=5 s=[]",
            r#"T|7.2.1b|'{a:1, b:'h2, s:"x"}"#,
            "T|7.2.1c|1 9",
            r#"T|7.2.2a|'{a:0, b:'h0, s:"d"}"#,
            r#"T|7.2.2b|'{a:7, b:'h1, s:"q"}"#,
            "T|7.2.1d|c 3 8",
            "T|7.2.1e|c9",
            "T|7.2.1f|-16 15",
            "T|7.2.1g|12",
            "T|7.2.1h|1",
            "T|7.2.1i|'{p:'{hi:'h3, lo:'h4}, arr:'{5, 6}}",
            "T|7.2.1j|'{p:'{hi:'ha, lo:'hb}, arr:'{3, 3}}",
            "T|7.3.1|a 5",
            "T|7.3.2a|42",
            "T|7.3.2b|valid 42",
            "T|7.3c|5",
            "T|7.4.1a|44 11 2",
            "T|7.4.1b|1122ff44",
            "T|7.4.1c|22ff",
            "T|7.4.2a|'{'h1, 'h2, 'h3, 'h4}",
            "T|7.4.2b|xx",
            "T|7.4.2c|'{'h1, 'h2, 'h3, 'h4}",
            "T|7.4.5a|6 1 '{'{1, 2, 3}, '{4, 5, 6}}",
            "T|7.4.5b|dc 1",
            "T|7.4.6b|'{'hee, 'hee, 'hee, 'hee}",
            "T|7.4.6c|xx",
            "T|7.4.3|2 3",
            "T|7.5.1a|'{0, 0, 0} size=3",
            "T|7.5.1b|'{1, 2, 3, 0, 0}",
            "T|7.5.1c|'{1, 2}",
            "T|7.5.3|0",
            "T|7.5d|'{4, 5}",
            "T|7.5e|0",
            "T|7.6a|'{1, 2, 3} '{99, 2, 3} 99",
            "T|7.6b|'{7, 8, 9}",
            "T|7.6c|'{7, 8, 9}",
            "T|7.8a|3 3 1",
            r#"T|7.8b|'{"a":1, "b":2, "c":3}"#,
            "T|7.8c|first=a",
            "T|7.8d|next=b",
            "T|7.8e|last=c",
            "T|7.8f|prev=b",
            "T|7.8g|next-of-last=0",
            r#"T|7.8i|'{"a":1, "c":3}"#,
            "T|7.8j|nonexist=0",
            "T|7.8k|num=2",
            "T|7.8l|'{-5:1, 3:3, 10:2}",
            "T|7.8m|-5",
            "T|7.8n|'{1:2, 15:1}",
            "T|7.8p|2",
            "T|7.8q|1 -1",
            "T|7.8r|0",
            "T|7.8s|xx",
            "T|7.8t|6",
            "T|7.10a|'{0, 1, 2} 3",
            "T|7.10b|'{0, 9, 1, 2}",
            "T|7.10c|0 2",
            "T|7.10d|'{9, 1}",
            "T|7.10e|'{1}",
            "T|7.10f|'{2, 3, 4} '{4, 5} '{}",
            "T|7.10g|'{1, 2, 10, 3, 4, 5}",
            "T|7.10h|5 0",
            "T|7.10i|'{1, 2, 10, 3, 4, 5, 77}",
            "T|7.10j|'{1, 2, 10, 3, 4, 5, 77}",
            "T|7.10k|0 0",
            "T|7.10o|'{3, 1, 2, 4}",
            r#"T|7.10p|'{"a", "b"}"#,
            "T|7.12.1a|'{5, 8}",
            "T|7.12.1b|'{1, 3}",
            "T|7.12.1c|'{3}",
            "T|7.12.1d|'{1}",
            "T|7.12.1e|'{}",
            "T|7.12.1f|'{3}",
            "T|7.12.1g|'{1}",
            "T|7.12.1h|'{8}",
            "T|7.12.1k|'{3}",
            "T|7.12.1l|'{3, 1}",
            "T|7.12.2a|'{1, 3, 8, 3, 5}",
            "T|7.12.2b|'{1, 3, 3, 5, 8}",
            "T|7.12.2c|'{8, 5, 3, 3, 1}",
            "T|7.12.2d|'{3, 3, 1, 8, 5}",
            "T|7.12.2e|'{1, 3, 3, 5, 8}",
            "T|7.12.3a|20 360 0",
            "T|7.12.3b|15 12",
            "T|7.12.3c|0",
            "T|7.12.3d|2",
            "T|7.12.3e|44 300",
            "T|7.12.3g|7",
            "T|7.12.3h|0 '{}",
            "T|7.12.2f|2 3 1",
            "T|7.12.4|'{}",
            "T|7.12.1m|32",
        ],
        &[
            "T|7.12.1i|",
            "T|7.12.1j|",
            "T|7.4.6a|",
            "T|7.5f|",
            "T|7.8h|",
            "T|7.8o|",
            "T|7.10l|",
            "T|7.10m|",
            "T|7.10n|",
            "T|7.12.3f|",
            "T|7.12.3i|",
        ],
    );
}

// §7.3.1, §7.4.6: packed unions, out-of-range array reads, nested containers
#[test]
fn c7_aggregates_more() {
    // Known gap (§7.10): `7.10r` reference `'{'{1, 2}, '{3, 4}} 2`, xezim `'{2, 3} 1`
    // Known gap (§7.5): `7.5g` reference `'{'{0}, '{0, 9}}`, xezim `'{0, 0}`
    // Known gap (§7.8): `7.8v` reference `'{5:'{1, 2}, 7:'{3} } 2`, xezim `'{5:x, 7:3 } 2`
    // Known gap (§7.4.5): `7.4.5d` reference `ab`, xezim `xx`
    // Known gap (§7.12.1): `7.12.1o` reference `'{"x", "z"}`, xezim `'{}`
    // Known gap (§7.12.1): `7.12.1p` reference `'{1}`, xezim `'{}`
    // Known gap (§7.12.1): `7.12.1q` reference `2`, xezim `0`
    // Known gap (§7.12.1): `7.12.1r` reference `'{1}`, xezim `'{}`
    // Known gap (§7.10): `7.10v` reference `'{1, 2}`, xezim `'{1, 2, 3}`
    check(
        "c07_more",
        include_str!("sv/c07_more.sv"),
        "c07x",
        Order::Exact,
        &[
            "T|7.3.1b|a 123 a1 23",
            "T|7.3.1c|a1ff",
            "T|7.4a|2345 345",
            "T|7.4b|fxxx",
            "T|7.4c|1234 5 234",
            "T|7.4d|12345000",
            "T|7.10q|10 two 2",
            "T|7.8u|6 five",
            "T|7.12.1n|1 10",
            "T|7.4.5c|ab 4 192",
            "T|7.4.6d|'{2, 3}",
            r#"T|7.8w|'{"a":2, "b":1}"#,
            "T|7.10s|'{1, 3, 4}",
            "T|7.10t|'{3}",
            "T|7.10u|'{1, 2}",
            r#"T|7.12.2g|'{"apple", "fig", "pear"}"#,
            r#"T|7.12.2h|'{"pear", "fig", "apple"}"#,
            "T|7.12.2i|'{3, 2, 1}",
            "T|7.12.3j|0",
            "T|7.12.3k|32",
            "T|7.4.6e|'{'h0, 'h0, 'h0, 'h0}",
            "T|7.6d|1 4",
            "T|7.5.2|0",
            "T|7.2.2c|3 4 9",
            "T|7.2.3|1",
            "T|7.2.1k|-1 0",
        ],
        &[
            "T|7.10r|",
            "T|7.5g|",
            "T|7.8v|",
            "T|7.4.5d|",
            "T|7.12.1o|",
            "T|7.12.1p|",
            "T|7.12.1q|",
            "T|7.12.1r|",
            "T|7.10v|",
        ],
    );
}
