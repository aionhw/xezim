//! §14.4: a clocking `input #skew` on an unpacked array registers per-element
//! sample history so `cb.arr[i]` honors skew for each index.

use xezim::simulate;

#[test]
fn clocking_unpacked_array_input_keeps_its_skew() {
    let src = r#"
`timescale 1ns/1ns
module t;
  logic clk = 0;
  logic [3:0] arr [2];
  always #5 clk = ~clk;
  clocking cb @(posedge clk); input #3 arr; endclocking
  initial begin
    arr[1] = 4'h5; #4 arr[1] = 4'h9;
    #2 $display("arr1=%h", cb.arr[1]);
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_eq!(msgs, ["arr1=5"]);
}
