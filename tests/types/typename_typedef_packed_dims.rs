//! IEEE 1800-2023 §20.6.1, §7.4.1: `$typename` of a variable declared with
//! packed dimensions on a typedef name (`lane_t [0:1] a`) names those
//! dimensions after the typedef's own, innermost first.

#[test]
fn typename_keeps_packed_dimensions_of_a_typedef_name() {
    let sim = xezim::simulate(
        r#"
module top;
  typedef bit node;
  typedef logic signed [16:0] lane_t;
  typedef bit [3:0] nib_t;
  node [2:0] x;
  lane_t [0:1] a;
  lane_t [1:0][2:0] a3;
  nib_t [1:0] n2;
  lane_t b;
  initial begin
    $display("T|%s|%s|%s", $typename(x), $typename(a), $typename(a3));
    $display("T|%s|%s", $typename(n2), $typename(b));
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
            "T|bit[2:0]|reg signed[16:0][0:1]|reg signed[16:0][2:0][1:0]",
            "T|bit[3:0][1:0]|reg signed[16:0]",
        ]
    );
}
