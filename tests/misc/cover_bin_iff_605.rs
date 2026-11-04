//! IEEE 1800-2017 §19.5: a coverpoint bin's own `iff (expr)` guard keeps
//! the bin from counting while false. It was not parsed: an `inside` in it
//! was a parse error (#605, reported by amal-khailtash), and any other guard
//! was skipped, so the bin counted regardless.
use xezim::simulate;

#[test]
fn bin_iff_guards_its_bin() {
    let src = r#"module top;
  int value, format;
  covergroup cg;
    cp: coverpoint value {
      bins limited_values = {1} iff (format inside {2,3});
      bins other = {5};
      bins eqg = {7} iff (format == 4);
    }
  endgroup
  cg c = new;
  initial begin
    value = 1; format = 0; c.sample();
    value = 5; c.sample();
    value = 7; format = 3; c.sample();
    $display("A %0.2f", c.cp.get_inst_coverage());
    value = 1; format = 2; c.sample();
    $display("B %0.2f", c.cp.get_inst_coverage());
    value = 7; format = 4; c.sample();
    $display("C %0.2f", c.cp.get_inst_coverage());
  end
endmodule
"#;
    let out: Vec<String> = simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("A ") || m.starts_with("B ") || m.starts_with("C "))
        .collect();
    assert_eq!(out, ["A 33.33", "B 66.67", "C 100.00"]);
}
