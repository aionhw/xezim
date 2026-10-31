//! IEEE 1800-2017 §7.6, §10.3, §23.6: a continuous assign of one whole
//! unpacked array to another drives every element when either side is a
//! hierarchical name (an interface instance's array, a child module's array).

use xezim::simulate;

const SRC: &str = r#"
interface ifc;
  logic [7:0] arr [2];
endinterface
module dut;
  logic [7:0] src [2];
  initial begin src[0] = 8'h12; src[1] = 8'h34; end
endmodule
module top;
  ifc i();
  dut d();
  logic [7:0] dst [2];
  assign i.arr = d.src;
  assign dst = i.arr;
  initial #1 $display("%h %h %h %h", i.arr[0], i.arr[1], dst[0], dst[1]);
endmodule
"#;

#[test]
fn hierarchical_whole_array_assign() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_eq!(msgs, ["12 34 12 34"]);
}
