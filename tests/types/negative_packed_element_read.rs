//! IEEE 1800-2023 §7.4.5 and §11.5.1: packed element selects use signed labels.

#[test]
fn continuous_packed_element_reads_keep_negative_labels() {
    let sim = xezim::simulate(
        r#"
module top;
  localparam int FIRST = -1;
  logic [1:-1][1:0] p = 6'b11_10_01;
  logic [-2:0][1:0] ascending = 6'b01_10_11;
  logic [1:-1][1:0][1:0] cube = 12'habc;
  int index = -1;
  wire [1:0] literal_index = p[-1];
  wire [1:0] param_index;
  assign param_index = p[FIRST];
  wire [1:0] expression_index = ascending[FIRST - 1];
  wire [1:0] runtime_index = p[index];
  wire [3:0] cube_element = cube[-1];
  wire [1:0] cube_leaf = cube[-1][1];
  wire [1:0] invalid = p[-2];
  wire [1:0] unsigned_index = p[-4'd1];
  initial begin
    #1;
    if ({literal_index, param_index, expression_index, runtime_index} !== 8'h55
        || cube_element !== 4'hc || cube_leaf !== 2'b11
        || invalid !== 2'bxx || unsigned_index !== 2'bxx)
      $fatal(1, "initial negative element reads: %b %b %b %b %b %b %b %b",
             literal_index, param_index, expression_index, runtime_index,
             cube_element, cube_leaf, invalid, unsigned_index);
    p[-1] = 2'b11;
    cube[-1] = 4'h5;
    index = 0;
    #1;
    if ({literal_index, param_index, runtime_index} !== 6'b11_11_10
        || cube_element !== 4'h5 || cube_leaf !== 2'b01)
      $fatal(1, "changed negative element reads");
    $display("PASS negative packed element reads");
    $finish;
  end
endmodule
"#,
        10,
    )
    .expect("simulate");
    assert!(
        sim.output
            .iter()
            .any(|o| o.message == "PASS negative packed element reads"),
        "{:?}",
        sim.output
    );
}
