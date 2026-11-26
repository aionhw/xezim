//! Issue #1197: stream concatenations with a `with [ array_range ]` operand
//! (§11.4.14.4). Only the selected elements stream on pack, and on unpack
//! only those elements are written (the rest of a fixed array is unmodified).
//! Covers `[lo:hi]`, `[lo+:w]`, `[lo-:w]`, the single-item `[i]` form,
//! constant ranges, `<<8`, the literal-slice `b[1:2]` cousin, a queue
//! operand, and the LHS unpack side. Expected values were checked against
//! the reference simulator and the LRM.
#[test]
fn issue_1197_stream_with_range() {
    let sim = xezim::simulate(include_str!("stream_with_range_1197.sv"), 100_000)
        .expect("simulate stream with-range");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        msgs.iter().any(|m| m == "PASS"),
        "stream `with [range]` must all pass: {msgs:#?}"
    );
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{msgs:#?}");
}
