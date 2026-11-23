/// §7.6, §10.3.3: an unpacked-array drive delays every element.
#[test]
fn delayed_unpacked_array_drive() {
    let source = r#"
module top;
  logic src [2];
  wire dst [2];
  assign #3 dst = src;
  initial begin
    src[0] = 0; src[1] = 0;
    #10 src[0] = 1; src[1] = 1;
    #1 $display("T|t=%0t dst=%b%b", $time, dst[0], dst[1]);
    #3 $display("T|t=%0t dst=%b%b", $time, dst[0], dst[1]);
  end
endmodule
"#;
    let sim = xezim::simulate(source, 100).expect("simulation failed");
    let messages: Vec<_> = sim.output.iter().map(|line| line.message.as_str()).collect();
    assert!(messages.contains(&"T|t=11 dst=00"), "{messages:?}");
    assert!(messages.contains(&"T|t=14 dst=11"), "{messages:?}");
}
