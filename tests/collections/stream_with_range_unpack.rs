//! IEEE 1800-2023 §11.4.14.4: the `with [ array_range ]` of a stream operand
//! "is evaluated immediately before its corresponding array is streamed", so
//! the LRM's packet example reads the length it has just unpacked
//! (`p.payload with [0 +: p.len]`); unpacking resizes a dynamic array or
//! queue to the range; packing a range past the end streams nonexistent
//! entries. Expected values are the reference simulator's, except `cont2`:
//! the range bounds are operands of a continuous assignment and are read by
//! the `always_comb`, so both re-evaluate when a bound changes (§10.3.2,
//! §9.2.2.2.1), where the reference simulator keeps `2233`.
#[test]
fn stream_with_range_evaluation_and_resize() {
    let sim = xezim::simulate(include_str!("stream_with_range_unpack.sv"), 100)
        .expect("simulate stream with-range unpack");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|cont 2233 comb 2233",
            "T|cont2 1122 comb2 1122",
            "T|beyond 112233440000",
            "T|beyond_queue 112233440000ff",
            "T|beyond_fixed 33440000ff0000",
            "T|beyond_size 3 33 44 00",
            "T|dynunpack size=3 99 01 02",
            "T|queueunpack size=3 0a 0b 0c",
            "T|packet_pack size=15",
            "T|packet_unpack 01020304 3 3 aa bb cc 0a0b0c0d",
        ]
    );
}
