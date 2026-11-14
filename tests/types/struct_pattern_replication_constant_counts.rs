//! IEEE 1800-2023 §10.9.2, A.6.7.1: a structure replication whose count is a
//! constant expression (a parameter, a localparam, a constant operation)
//! sets every member, like a literal count. Only the first member was set
//! outside continuous assignments.

#[test]
fn constant_replication_counts_bind_every_member() {
    let sim = xezim::simulate(
        include_str!("struct_pattern_replication_constant_counts.sv"),
        1_000,
    )
    .expect("legal replicated structure patterns");
    let got: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(
        got,
        [
            "T|param 3 3 localparam 4 4 expr 5 5",
            "T|nba 3 3 comb 3 3",
            "T|context fe fffe",
            "T|four 3 4 3 4",
            "T|class 3 3 return 3 3",
            "T|argument 33",
            "T|constant 5 5 / 1 2 1 2",
        ]
    );
}
