//! §13.8, §13.4.3, §8.25 (#1130): a static function of a class, or of a
//! specialization named with the module's own parameter (`C#(PW)::id()`),
//! called in a `localparam` or `parameter` initializer is folded as a
//! constant function with the specialization's parameters in view, per
//! module instance; the parameter read 0. The reference simulator rejects a
//! class function in a constant expression, so the expected values are
//! those §13.8 gives.
use std::process::Command;

const DESIGN: &str = r#"
package pk;
virtual class clprm#(parameter int W = 8);
  static function int id(); return W; endfunction
  static function int twice(int x); return 2 * x + W; endfunction
endclass
class plain;
  static function int seven(); return 7; endfunction
endclass
endpackage
module c_lprm #(parameter int PW = 8) (output int id_out);
  import pk::*;
  localparam int L = clprm#(PW)::id();
  localparam int M = clprm#(PW)::twice(3);
  localparam int N = plain::seven() + 1;
  logic [L-1:0] vec;
  assign id_out = L;
  initial #1 $display("%m L=%0d M=%0d N=%0d bits=%0d", L, M, N, $bits(vec));
endmodule
module top;
  import pk::*;
  int a, b;
  localparam int T = clprm#(5)::id();
  parameter int U = plain::seven();
  c_lprm #(.PW(6)) u6 (.id_out(a));
  c_lprm #(.PW(160)) u160 (.id_out(b));
  initial begin #2 $display("%0d %0d T=%0d U=%0d", a, b, T, U); $finish; end
endmodule
"#;

const EXPECTED: &[&str] = &[
    "top.u6 L=6 M=12 N=8 bits=6",
    "top.u160 L=160 M=166 N=8 bits=160",
    "6 160 T=5 U=7",
];

fn run(compile: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "xezim_static_param_{}_{}",
        std::process::id(),
        compile
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.sv");
    std::fs::write(&path, DESIGN).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .env("XEZIM_COMPILE_METHODS", compile)
        .env_remove("XEZIM_METHOD_CACHE")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    text
}

#[test]
fn class_static_function_folds_in_a_parameter_initializer() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let got: Vec<&str> = log.lines().filter(|l| EXPECTED.contains(l)).collect();
        assert_eq!(got, EXPECTED, "XEZIM_COMPILE_METHODS={compile}:\n{log}");
    }
}
