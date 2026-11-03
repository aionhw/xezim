//! §23.10, §27.3: a `generate ... endgenerate` region adds no scope, so a
//! parameter declared in one is a body parameter of the module. With no
//! parameter port list it takes an ordered override slot (§23.10.2.1).

use xezim::simulate;

#[test]
fn ordered_override_reaches_a_generate_region_parameter() {
    let src = r#"
module c(output int y);
  generate
    parameter P = 1;
  endgenerate
  assign y = P;
endmodule
module tb;
  int y;
  c #(7) u(.y(y));
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    let y = sim
        .get_signal("y")
        .or_else(|| sim.get_signal("tb.y"))
        .and_then(|v| v.to_u64());
    assert_eq!(y, Some(7));
}
