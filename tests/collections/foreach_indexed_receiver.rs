//! IEEE 1800 §12.7.3: foreach bounds belong to the final selected collection.
use xezim::simulate;

fn check(source: &str) {
    let sim = simulate(source, 1000).unwrap();
    let output = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        output.contains("PASS") && !output.contains("Fatal"),
        "{output}"
    );
}

#[test]
fn nested_foreach_uses_indexed_receivers_collection_bounds() {
    check(include_str!("../fixtures/foreach_indexed_receiver.sv"));
}

#[test]
fn blocking_foreach_uses_indexed_receivers_collection_bounds() {
    let source = include_str!("../fixtures/foreach_indexed_receiver.sv").replace(
        "c.setup(c.systems[0]);",
        "foreach(c.systems[0].targets[0].ports[j]) begin #1; c.systems[0].targets[0].ports[j].kind=1; end",
    );
    check(&source);
}
