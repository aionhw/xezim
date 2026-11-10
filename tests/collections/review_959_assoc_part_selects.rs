//! #959 review: §7.4.6, §11.5.1: a part-select of an associative array
//! element whose packed type has several dimensions selects whole elements
//! of the leading packed dimension: `p[k][3:2]` of `logic [3:0][7:0] p [int]`
//! is bytes 3 and 2, for constant, `+:` and `-:` forms, ascending ranges and
//! a select already into the element (`r[k][1][2:1]`). Expected values come
//! from the reference simulator.

use xezim::simulate;

#[test]
fn part_selects_of_multi_dimensional_packed_elements() {
    let src = r#"
module top;
  logic [3:0][7:0] p [int];
  logic [0:3][7:0] q [string];
  bit [1:0][3:0][3:0] r [int];
  initial begin
    p[1] = 32'h44332211;
    q["k"] = 32'h44332211;
    r[5] = 32'h87654321;
    $display("p: %h %h %h %h", p[1][3:2], p[1][1:0], p[1][0+:2], p[1][3-:3]);
    $display("q: %h %h", q["k"][0:1], q["k"][2+:2]);
    $display("r: %h %h %h", r[5][1][2:1], r[5][0], r[5][1:0]);
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_eq!(
        out,
        [
            "p: 4433 2211 2211 443322",
            "q: 4433 2211",
            "r: 76 4321 87654321"
        ]
    );
}
