use xezim::simulate;

fn output(src: &str) -> String {
    let sim = simulate(src, 100).expect("simulation should succeed");
    sim.output
        .iter()
        .map(|line| line.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn exact_unpacked_array_port_binding() {
    let out = output(
        r#"
module array_cell(
  input bit [1:0] a [0:2],
  output bit [1:0] y [0:2]
);
  assign y[0] = a[0];
  assign y[1] = a[1];
  assign y[2] = a[2];
endmodule

module top;
  bit [1:0] data [0:2];
  bit [1:0] result [0:2];
  array_cell dut(.a(data), .y(result));
  initial begin
    data[0] = 2'b01;
    data[1] = 2'b10;
    data[2] = 2'b11;
    #1;
    if (result[0] === 2'b01 && result[1] === 2'b10 && result[2] === 2'b11)
      $display("EXACT_PASS");
  end
endmodule
"#,
    );
    assert!(out.contains("EXACT_PASS"), "unexpected output:\n{out}");
}

#[test]
fn instance_array_consumes_leading_unpacked_dimension() {
    let out = output(
        r#"
module array_cell(
  input bit [1:0] a [0:2],
  output bit [1:0] y [0:2]
);
  assign y[0] = a[0];
  assign y[1] = a[1];
  assign y[2] = a[2];
endmodule

module top;
  bit [1:0] data [0:3][0:2];
  wire [1:0] result [0:3][0:2];
  array_cell dut [0:3](.a(data), .y(result));
  initial begin
    data[0][0] = 2'b11;
    data[1][1] = 2'b01;
    data[2][2] = 2'b10;
    data[3][0] = 2'b00;
    #1;
    if (result[0][0] === 2'b11 && result[1][1] === 2'b01 &&
        result[2][2] === 2'b10 && result[3][0] === 2'b00)
      $display("SLICED_PASS");
  end
endmodule
"#,
    );
    assert!(out.contains("SLICED_PASS"), "unexpected output:\n{out}");
}

#[test]
fn generated_tasks_and_virtual_interface_array_binding() {
    let out = output(
        r#"
interface channel_if;
  logic [7:0] data;
  logic valid;
endinterface

module top;
  channel_if channels [4]();
  virtual channel_if vifs [4];

  genvar gi;
  generate
    for (gi = 0; gi < 4; gi++) begin : blocks
      task automatic clear(input bit [7:0] value);
        channels[gi].data = value;
        channels[gi].valid = 1'b0;
      endtask
    end
  endgenerate

  initial begin
    vifs = channels;
    for (int i = 0; i < 4; i++) begin
      vifs[i].data = 8'ha0 + i;
      vifs[i].valid = 1'b1;
    end
    blocks[0].clear(8'h10);
    blocks[1].clear(8'h11);
    blocks[2].clear(8'h12);
    blocks[3].clear(8'h13);
    #1;
    if (vifs[0].data === 8'h10 && vifs[1].data === 8'h11 &&
        vifs[2].data === 8'h12 && vifs[3].data === 8'h13 &&
        vifs[0].valid === 1'b0 && vifs[1].valid === 1'b0 &&
        vifs[2].valid === 1'b0 && vifs[3].valid === 1'b0)
      $display("VIF_GENERATE_PASS");
  end
endmodule
"#,
    );
    assert!(out.contains("VIF_GENERATE_PASS"), "unexpected output:\n{out}");
}
