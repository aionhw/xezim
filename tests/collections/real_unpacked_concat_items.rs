//! IEEE 1800-2023 §10.10: an item of an unpacked array concatenation may be
//! `real`, alone or as an array of reals; §11.4.12 rules out real operands
//! only in a packed concatenation.

#[test]
fn real_items_of_unpacked_array_concatenations() {
    let sim = xezim::simulate(
        r#"
module top;
  real rw[2] = '{6.25, 7.25};
  real rd[3];
  real rq[$];
  initial begin
    rd = {rw, 5.25};
    $display("T|whole %0.2f %0.2f %0.2f", rd[0], rd[1], rd[2]);
    rd = {6.25, 7.25, 5.25};
    $display("T|scalars %0.2f %0.2f %0.2f", rd[0], rd[1], rd[2]);
    rq = {rq, 2.5};
    rq = {rq, rw[0]};
    $display("T|queue %0d %0.2f %0.2f", rq.size(), rq[0], rq[1]);
    rd <= {1.5, rw};
    #1 $display("T|nonblocking %0.2f %0.2f %0.2f", rd[0], rd[1], rd[2]);
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|whole 6.25 7.25 5.25",
            "T|scalars 6.25 7.25 5.25",
            "T|queue 2 2.50 6.25",
            "T|nonblocking 1.50 6.25 7.25",
        ]
    );
}

#[test]
fn real_operand_of_a_packed_concatenation_is_still_rejected() {
    // §11.4.12: the target is a real, so the concatenation is packed.
    let err = xezim::simulate(
        "module top; real r1 = 1.0, r2 = 2.0; real r3; initial r3 = {r1, r2}; endmodule",
        10,
    )
    .err()
    .expect("a real operand of a packed concatenation is an error");
    assert!(
        err.contains("cannot be an operand of a concatenation"),
        "{err}"
    );
}
