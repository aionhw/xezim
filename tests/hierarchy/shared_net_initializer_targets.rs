//! IEEE 1800 §6.7, §10.3.1: shared initializer syntax preserves per-instance nets.
use xezim::simulate;

#[test]
fn initializer_targets_preserve_instances_delays_and_strengths() {
    let result = simulate(
        r#"
module leaf #(parameter W=4, D=1)(input wire [W-1:0] a, output wire [W-1:0] y);
  wire [W-1:0] n = a ^ W'(3);
  wire [W-1:0] #D delayed = n;
  wire (weak0,weak1) weak_net = a[0];
  assign (strong0,strong1) weak_net = 1'b0;
  assign y = delayed;
  initial begin #5; if (weak_net !== 0) $fatal(1,"strength"); end
endmodule
module top;
  reg [7:0] a=8'h65;
  wire [3:0] x; wire [7:0] y;
  leaf #(.W(4),.D(1)) u(a[3:0],x);
  leaf #(.W(8),.D(2)) v(a,y);
  for(genvar i=0;i<2;i++) begin:g
    wire [7:0] z;
    leaf #(.W(8)) w(a,z);
    initial begin #3; if(z !== 8'h66) $fatal(1,"generate"); end
  end
  initial begin
    #3; if(x !== 4'h6 || y !== 8'h66) $fatal(1,"initial");
    a=8'ha9;
    #3; if(x !== 4'ha || y !== 8'haa) $fatal(1,"update");
    $display("PASS"); $finish;
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    assert!(result.output.iter().any(|line| line.message == "PASS"));
    assert!(
        !result
            .output
            .iter()
            .any(|line| line.message.contains("FATAL"))
    );
}
