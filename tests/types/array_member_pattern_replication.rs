//! IEEE 1800-2023 §10.9.1/§10.10.1/§10.9.2: an array member's
//! replicated assignment pattern binds and converts every element.
fn check(src: &str, expected: &[&str]) {
    let sim = xezim::simulate(src, 10_000).expect("legal array-member patterns");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(lines, expected);
}

#[test]
fn replicated_array_member_elements() {
    check(
        include_str!("array_member_pattern_replication.sv"),
        &[
            "T|ordered 7,7 keyed 7,7 one 9",
            "T|literal 8,8,8",
            "T|parameter 8,8,8",
            "T|localparam 8,8,8",
            "T|expression 8,8,8",
            "T|several 3,5,3,5",
            "T|nested 7,7,7,7",
            "T|signed fffe,fffe packed ff,ff",
            "T|states xz10,xz10/0010,0010",
            "T|structs 3,5/3,5",
            "T|real 1.5,1.5",
            "T|string hi,hi",
            "T|bit replication 00fe",
            "T|signed one ff80",
        ],
    );
}

#[test]
fn replicated_array_member_contexts_and_guards() {
    check(
        include_str!("array_member_pattern_replication_contexts.sv"),
        &[
            "T|port 7,7",
            "T|declaration 7,7",
            "T|continuous 7,7",
            "T|nba 7,7",
            "T|return 7,7",
            "T|argument 14",
            "T|plain 7,7 default 7,7",
            "T|sibling default 7,7/9",
            "T|element member 7,7",
        ],
    );
}
