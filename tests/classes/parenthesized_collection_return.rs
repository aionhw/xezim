use xezim::simulate;
#[test]
fn parenthesized_queue_returns_preserve_empty_and_nonempty_values() {
    let sim = simulate(
        include_str!("../fixtures/parenthesized_collection_return.sv"),
        1000,
    )
    .unwrap();
    let out = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(out.contains("PASS") && !out.contains("Fatal"), "{out}");
}
