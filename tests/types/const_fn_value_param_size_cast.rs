//! IEEE 1800-2023 §6.24.1 / §13.4.3: named value-parameter sizes
//! in constant functions must fold before declarations are sized.
//! Expected values checked against the reference simulator.
use xezim::simulate;

const WIDTHS: &str = r#"module leaf #(parameter int W = 3);
  function automatic int narrow(int unsigned x); return int'(W'(x)); endfunction
  localparam int P = narrow(13);
  logic [P-1:0] sized;
  initial $display("%m W=%0d P=%0d bits=%0d", W, P, $bits(sized));
endmodule
module top #(parameter int W = 6);
  function automatic int root_narrow(int unsigned x); return int'(W'(x)); endfunction
  localparam int P = root_narrow(13);
  leaf c3();
  leaf #(.W(4)) c4();
  initial begin
    $display("top W=%0d P=%0d", W, P);
    #1 $finish;
  end
endmodule"#;
const SCOPES: &str = r#"package widths;
  localparam int PW = 5;
  function automatic int pkg_narrow(int unsigned x); return int'(PW'(x)); endfunction
endpackage
module scoped #(parameter int W = 3);
  localparam int CW = W + 1;
  function automatic int local_narrow(int unsigned x); return int'(CW'(x)); endfunction
  function automatic int nested(int unsigned x); return local_narrow(int'(W'(x))); endfunction
  function automatic int signed_cmp(); return int'(W'(-1) < 0); endfunction
  function automatic int contextual(); return int'(CW'(3'd7 + 3'd6)); endfunction
  function automatic int qualified(int unsigned x); return int'(widths::PW'(x)); endfunction
  localparam int L = local_narrow(31);
  localparam int N = nested(31);
  localparam int S = signed_cmp();
  localparam int C = contextual();
  localparam int K = qualified(63);
  localparam int G = widths::pkg_narrow(63);
  initial $display("%m L=%0d N=%0d S=%0d C=%0d K=%0d G=%0d", L, N, S, C, K, G);
endmodule
module top;
  scoped c3();
  scoped #(.W(4)) c4();
  initial #1 $finish;
endmodule"#;

fn check(src: &str, expected: &[&str]) {
    let sim = simulate(src, 1_000).expect("simulate");
    let messages: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    for want in expected {
        assert!(messages.contains(want), "missing {want:?} in {messages:?}");
    }
}

#[test]
fn child_module_value_parameter_size_cast() {
    check(
        r#"module leaf #(parameter int W = 3);
  function automatic bit [2:0] f(); return W'(4); endfunction
  localparam bit [2:0] P = f();
  initial $display("P=%h expected=4", P);
endmodule
module top;
  leaf dut();
  initial #1 $finish;
endmodule"#,
        &["P=4 expected=4"],
    );
}

#[test]
fn top_module_value_parameter_size_cast() {
    check(WIDTHS, &["top W=6 P=13"]);
}

#[test]
fn child_module_cast_sizes_dependent_declarations() {
    check(WIDTHS, &["top.c3 W=3 P=5 bits=5"]);
}

#[test]
fn overridden_instance_uses_its_own_cast_size() {
    check(WIDTHS, &["top.c4 W=4 P=13 bits=13"]);
}

#[test]
fn local_and_package_sizes_keep_sign_and_expression_context() {
    check(
        SCOPES,
        &[
            "top.c3 L=15 N=7 S=1 C=13 K=31 G=31",
            "top.c4 L=31 N=15 S=1 C=13 K=31 G=31",
        ],
    );
}
