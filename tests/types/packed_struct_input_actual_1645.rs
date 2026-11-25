//! §7.2.1, §23.3.3: the view of a packed input port follows the formal layout.

#[test]
fn packed_struct_input_retains_layout_for_vector_elements() {
    let sim = xezim::simulate(
        r#"
typedef struct packed { logic sign; logic payload; } formal_t;
typedef struct packed { logic first; logic second; } other_t;

module dut(input wire formal_t a, output wire [1:0] decoded);
  assign decoded = {a.sign, a.payload};
endmodule

module top;
  logic [1:0][1:0] data = 4'b0110;
  wire other_t different = 2'b01;
  wire formal_t same = 2'b10;
  wire [1:0] from_vector, from_other, from_same;
  dut u_vec(.a(data[0]), .decoded(from_vector));
  dut u_other(.a(different), .decoded(from_other));
  dut u_same(.a(same), .decoded(from_same));
  initial begin
    #1 $display("decoded=%b/%b/%b", from_vector, from_other, from_same);
    data = 4'b1001;
    #1 $display("decoded=%b/%b/%b", from_vector, from_other, from_same);
    $finish;
  end
endmodule
"#,
        1_000_000,
    )
    .expect("simulation");
    let result: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.trim())
        .filter(|line| line.starts_with("decoded="))
        .collect();
    assert_eq!(result, ["decoded=10/01/10", "decoded=01/01/10"]);
}
