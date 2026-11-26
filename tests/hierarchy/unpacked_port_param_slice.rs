//! §7.4.6, §23.3.3: an unpacked-array port connected to a slice of an
//! unpacked array takes the slice's elements in order, whether the slice's
//! bounds are literals or parameter expressions. Inside an instantiated
//! module, `.p(arr[0:N-1])` drove `p[t]` from bit `t` of `arr[0]`, and an
//! output port connected to `res[N:2*N-1]` drove nothing. xezim-core split the
//! connection only when it could fold the bounds, and inside an instance the
//! bound named the prefixed `w.N`, which the scope's parameter map does not
//! key. The Intel FPGA NPU's `npu.sv` connects its vector
//! register files this way. Expected values follow Verilator.

const SRC: &str = r#"
module sink #(parameter N = 2) (input [8:0] p [0:N-1], output [8:0] q0, output [8:0] q1);
  assign q0 = p[0];
  assign q1 = p[1];
endmodule
module src #(parameter N = 2) (input [8:0] d, output [8:0] o [0:N-1]);
  assign o[0] = d + 9'd10;
  assign o[1] = d + 9'd11;
endmodule
module wrap #(parameter N = 2) (
    input  [8:0] d,
    output [8:0] a0, a1, b0, b1, c0, c1, e0, e1
);
  wire [8:0] arr [0:3];
  assign arr[0] = d;
  assign arr[1] = d + 9'd1;
  assign arr[2] = d + 9'd2;
  assign arr[3] = d + 9'd3;
  sink #(.N(N)) lit (.p(arr[0:1]), .q0(a0), .q1(a1));
  sink #(.N(N)) par (.p(arr[0:N-1]), .q0(b0), .q1(b1));
  sink #(.N(N)) upper (.p(arr[N:N+1]), .q0(c0), .q1(c1));
  wire [8:0] res [0:3];
  src #(.N(N)) so (.d(d), .o(res[N:2*N-1]));
  assign e0 = res[2];
  assign e1 = res[3];
endmodule
module tb;
  logic [8:0] d = 9'd39;
  wire [8:0] a0, a1, b0, b1, c0, c1, e0, e1;
  wrap #(.N(2)) w (.*);
  initial begin
    #1 $display("d=%0d lit=%0d,%0d param=%0d,%0d upper=%0d,%0d out=%0d,%0d",
                d, a0, a1, b0, b1, c0, c1, e0, e1);
    d = 9'd52;
    #1 $display("d=%0d lit=%0d,%0d param=%0d,%0d upper=%0d,%0d out=%0d,%0d",
                d, a0, a1, b0, b1, c0, c1, e0, e1);
  end
endmodule
"#;

#[test]
fn parameter_bounded_slices_connect_element_by_element() {
    let sim = xezim::simulate(SRC, 100).expect("simulate");
    let got: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("d="))
        .collect();
    assert_eq!(
        got,
        [
            "d=39 lit=39,40 param=39,40 upper=41,42 out=49,50",
            "d=52 lit=52,53 param=52,53 upper=54,55 out=62,63",
        ]
    );
}
