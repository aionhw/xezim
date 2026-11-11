//! Verification for Issue #1053:
//! Fixed unpacked class arrays avoid eager per-element allocation for large arrays.

#[test]
fn test_large_class_array_lazy_allocation_1053() {
    let src = r#"
class item;
  byte data[262144];
  logic [3:0] nibbles[100000];
endclass
module top;
  item it;
  initial begin
    it = new();
    if (it.data[100] !== 8'h00) $fatal(1, "uninit data mismatch: %h", it.data[100]);
    it.data[100] = 8'h5a;
    if (it.data[100] !== 8'h5a) $fatal(1, "written data mismatch: %h", it.data[100]);
    if (it.data[101] !== 8'h00) $fatal(1, "neighbor mismatch: %h", it.data[101]);

    if (it.nibbles[50] !== 4'hx) $fatal(1, "nibble uninit mismatch: %h", it.nibbles[50]);
    it.nibbles[50] = 4'ha;
    if (it.nibbles[50] !== 4'ha) $fatal(1, "nibble written mismatch: %h", it.nibbles[50]);
    if (it.nibbles[51] !== 4'hx) $fatal(1, "nibble neighbor mismatch: %h", it.nibbles[51]);

    $display("TAG_PASS");
  end
endmodule
"#;
    let sim = xezim::simulate(src, 1000).expect("simulate");
    assert!(
        sim.output.iter().any(|o| o.message == "TAG_PASS"),
        "Expected TAG_PASS in output, got: {:?}",
        sim.output
    );
}
