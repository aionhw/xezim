//! IEEE 1800-2023 §7.4.1 / §7.4.5: selecting a packed array element
//! retains its remaining packed dimensions in every module instance.
//! Expected values checked with the reference simulator.
use xezim::simulate;

const BASIC: &str = r#"module leaf;
  localparam bit [1:0][1:0] L = 4'b0010;
  localparam bit [0:1][1:0] A = 4'b1001;
  localparam bit [3:2][1:0] R = 4'b0110;
  initial $display("%m elem=%0d bits=%0d A0=%0d A1=%0d R2=%0d R3=%0d", L[0], $bits(L[0]), A[0], A[1], R[2], R[3]);
endmodule
module top;
  localparam bit [1:0][1:0] ROOT = 4'b0010;
  leaf u();
  initial begin
    $display("hier elem=%0d bits=%0d", u.L[0], $bits(u.L[0]));
    #1 $finish;
  end
endmodule"#;

const SHAPES: &str = r#"module shapes #(parameter int N = 2, W = 2);
  typedef bit [W-1:0] word_t;
  localparam bit [N-1:0][W-1:0] D = '1;
  localparam word_t [N-1:0] T = '1;
  localparam bit [N-1:0][1:0][W-1:0] C = '1;
  localparam int FIRST = D[0];
  initial $display("%m D=%0d bits=%0d T=%0d tbits=%0d C=%0d cbits=%0d leafbits=%0d FIRST=%0d", D[0], $bits(D[0]), T[0], $bits(T[0]), C[0], $bits(C[0]), $bits(C[0][0]), FIRST);
  if (1) begin : g
    localparam bit [N-1:0][W-1:0] G = '1;
    initial $display("%m elem=%0d bits=%0d", G[0], $bits(G[0]));
  end
endmodule
module top #(parameter int N = 17, W = 19);
  shapes a();
  shapes #(.N(3), .W(5)) b();
  initial #2 $finish;
endmodule"#;

fn check(src: &str, expected: &[&str]) {
    let sim = simulate(src, 1_000).expect("simulate");
    let messages: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    for want in expected {
        assert!(messages.contains(want), "missing {want:?} in {messages:?}");
    }
}

#[test]
fn child_localparam_elements_keep_value_and_width() {
    check(
        BASIC,
        &[
            "top.u elem=2 bits=2 A0=2 A1=1 R2=2 R3=1",
            "hier elem=2 bits=2",
        ],
    );
}

#[test]
fn top_localparam_and_child_variable_controls() {
    check(
        r#"module variable_leaf;
  bit [1:0][1:0] L = 4'b0010;
  initial $display("variable elem=%0d bits=%0d", L[0], $bits(L[0]));
endmodule
module top;
  localparam bit [1:0][1:0] L = 4'b0010;
  variable_leaf u();
  initial begin
    $display("root elem=%0d bits=%0d", L[0], $bits(L[0]));
    #1 $finish;
  end
endmodule"#,
        &["root elem=2 bits=2", "variable elem=2 bits=2"],
    );
}

#[test]
fn localparam_element_comparison_selects_second_input() {
    check(
        r#"module reducer_leaf;
  localparam bit [1:0][15:0] SUB = 32'h2;
  logic [1:0][7:0] data;
  logic [7:0] result;
  always_comb begin
    for (int i = 0; i < 1; i++) begin
      if (SUB[i] == 2) result = data[0] | data[1];
      else result = data[0];
    end
  end
  initial begin
    data = '0;
    data[1] = 7;
    #1 $display("result=%0d sub0=%0d bits=%0d", result, SUB[0], $bits(SUB[0]));
  end
endmodule
module top;
  reducer_leaf u();
  initial #2 $finish;
endmodule"#,
        &["result=7 sub0=2 bits=16"],
    );
}

#[test]
fn instance_parameters_size_body_localparam_elements() {
    check(
        SHAPES,
        &[
            "top.a D=3 bits=2 T=3 tbits=2 C=15 cbits=4 leafbits=2 FIRST=3",
            "top.b D=31 bits=5 T=31 tbits=5 C=1023 cbits=10 leafbits=5 FIRST=31",
            "top.a.g elem=3 bits=2",
            "top.b.g elem=31 bits=5",
        ],
    );
}
