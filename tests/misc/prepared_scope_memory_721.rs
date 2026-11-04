//! IEEE 1800-2023 §6.20, §23.10 and §27 (#721): compact prepared-body
//! keys retain complete wide and string values when selecting generate arms.

#[test]
fn prepared_scope_keys_preserve_wide_and_string_parameter_values() {
    let sim = xezim::simulate(
        r#"
module wide_leaf #(parameter logic[127:0] P=0) ();
  if (P[90]) begin:g logic[6:0] a; end
  else begin:g logic[2:0] a; end
endmodule
module string_leaf #(parameter string P="first___same_low") ();
  if (P == "first___same_low") begin:g logic[6:0] a; end
  else begin:g logic[2:0] a; end
endmodule
module top;
  wide_leaf #(.P(128'h40000000000000000000000)) high();
  wide_leaf #(.P(128'd0)) low();
  wide_leaf #(.P(128'h40000000000000000000000)) high_again();
  string_leaf first();
  string_leaf #(.P("other___same_low")) other();
  string_leaf #(.P("first___same_low")) first_again();
  initial begin
    $display("KEY_CONTROL %0d %0d %0d %0d %0d %0d", $bits(high.g.a),
      $bits(low.g.a), $bits(high_again.g.a), $bits(first.g.a),
      $bits(other.g.a), $bits(first_again.g.a));
    $finish;
  end
endmodule
"#,
        100,
    )
    .expect("simulate distinct prepared scopes");
    let output: Vec<_> = sim
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .filter(|line| line.starts_with("KEY_CONTROL "))
        .collect();
    assert_eq!(output, ["KEY_CONTROL 7 3 7 7 3 7"]);
}
