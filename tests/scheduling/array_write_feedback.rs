//! §4.4.2.2, §10.4.1: a loop's indexed stores can feed subsequent reads.

#[test]
fn indexed_memory_write_feeds_the_next_loop_iteration() {
    let result = xezim::simulate(
        r#"
module top;
  logic [7:0] data[0:3];
  wire [7:0] forwarded = data[0];
  initial begin
    data[0] = 0;
    #1;
    for (int i = 0; i < 3; i++) data[i] = forwarded + 1;
    $display("T|%0d %0d %0d", data[0], data[1], data[2]);
    $finish;
  end
endmodule
"#,
        10,
    )
    .expect("simulate");
    assert!(
        result.output.iter().any(|o| o.message == "T|1 2 2"),
        "{:?}",
        result.output
    );
}
