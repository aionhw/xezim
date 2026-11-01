//! §9.2.2.2.1: an `always_comb` is sensitive to the longest static prefix of
//! each select it reads. A block that reads `ds[0]` and writes `ds[1]` of a
//! packed 2-D vector must wake when `ds[0]` is driven from outside, while a
//! block that rewrites the element it reads must still settle.

use xezim::simulate;

fn run(src: &str) -> (Vec<String>, u64) {
    let sim = simulate(src, 100).expect("simulate failed");
    let msgs = sim.output.iter().map(|o| o.message.clone()).collect();
    (msgs, sim.settle_limit_report().0)
}

#[test]
fn always_comb_reading_one_packed_element_and_writing_another_wakes_on_its_driver() {
    let (msgs, hits) = run(r#"
module top;
  logic [7:0] a;
  logic [1:0][7:0] ds;
  logic [7:0] sb;
  assign ds[0] = a;
  always_comb begin sb = ds[0] ^ 8'h01; ds[1] = sb; end
  initial begin
    a = 8'h01; #1 $display("ds1=%h", ds[1]);
    a = 8'h22; #1 $display("ds1=%h", ds[1]);
  end
endmodule
"#);
    assert_eq!(msgs, ["ds1=00", "ds1=23"]);
    assert_eq!(hits, 0);
}

#[test]
fn always_comb_rewriting_the_packed_element_it_reads_settles() {
    let (msgs, hits) = run(r#"
module top;
  logic [7:0] di;
  logic [2:0][7:0] lo;
  logic [7:0] r1;
  logic [1:0][7:0] tw;
  assign tw[0] = di;
  always_comb begin tw[1] = '0; tw[1] = tw[0]; end
  wire [3:0] sa; assign sa = {sa[2:0], 1'b1};
  always_comb begin lo[0] = di ^ 8'h0f; lo[0] ^= 8'h01; end
  always_comb begin r1 = ~lo[0]; end
  assign lo[1] = r1 ^ 8'h3c;
  assign lo[2] = ~lo[1];
  initial begin
    di = 8'h10; #1 $display("lo2=%h tw1=%h sa=%b", lo[2], tw[1], sa);
    di = 8'h33; #1 $display("lo2=%h tw1=%h", lo[2], tw[1]);
  end
endmodule
"#);
    assert_eq!(msgs, ["lo2=22 tw1=10 sa=1111", "lo2=01 tw1=33"]);
    assert_eq!(hits, 0);
}
