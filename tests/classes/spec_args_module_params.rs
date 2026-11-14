//! §8.25, §23.10: a class specialization written in a module with the
//! module's parameters (`g#(PW)::id()`, `g#(PW * 2)`, `g#(LW)`, `g#(.W(PW))`,
//! a type argument `logic [TW-1:0]`) names THAT instance's specialization.
//! Every instance ran the static methods, and read the static members and
//! class-scoped typedefs, of one specialization chosen by instantiation order
//! (#1129, #1130). Three instances with different widths; the expected lines
//! are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
package gp;
  virtual class g #(parameter int W = 8);
    static function [W-1:0] id(input [W-1:0] x);
      return W;
    endfunction
    static function [W-1:0] step(input [W-1:0] gr);
      logic [W-1:0] b;
      b = gr;
      for (int i = W - 2; i >= 0; --i) b[i] = gr[i] ^ b[i+1];
      b = b + 1'b1;
      return b ^ (b >> 1);
    endfunction
    static task t(output int o);
      o = W;
    endtask
    localparam int ID = W;
  endclass
  class v #(parameter int W = 8);
    static int sv = W;
    typedef logic [W-1:0] word_t;
    static function int tbits();
      word_t w;
      return $bits(w);
    endfunction
  endclass
  class ty #(type T = int, parameter int W = 8);
    static function int probe();
      T t;
      logic [W-1:0] x;
      return $bits(t) * 1000 + $bits(x);
    endfunction
  endclass
endpackage
module leaf #(parameter PW = 6, parameter TW = 6) (output int r[12]);
  import gp::*;
  localparam int LW = PW;
  logic [PW-1:0] ptr = '1;
  logic [PW-1:0] nxt;
  function automatic int wrap();
    return g#(PW)::id('0);
  endfunction
  assign r[0] = g#(PW)::id('0);
  assign r[1] = g#(PW)::ID;
  assign r[2] = g#(PW * 2)::id('0);
  assign r[3] = g#(LW)::id('0);
  assign r[4] = g#(.W(PW))::id('0);
  always_comb r[5] = g#(PW)::id('0);
  assign r[6] = wrap();
  assign r[7] = v#(PW)::sv;
  assign r[8] = v#(PW)::tbits();
  assign r[9] = ty#(logic [TW-1:0], PW)::probe();
  assign nxt = g#(PW)::step(ptr);
  assign r[10] = (nxt != ptr);
  initial begin
    #1 g#(PW)::t(r[11]);
  end
endmodule
module top;
  int a[12], b[12], c[12];
  leaf #(.PW(6), .TW(6)) u6(a);
  leaf #(.PW(160), .TW(160)) u160(b);
  leaf #(.PW(7), .TW(3)) u7(c);
  initial begin
    #2;
    foreach (a[i]) $display("r[%0d] = %0d / %0d / %0d", i, a[i], b[i], c[i]);
  end
endmodule
"#;

const EXPECTED: &[&str] = &[
    "r[0] = 6 / 160 / 7",
    "r[1] = 6 / 160 / 7",
    "r[2] = 12 / 320 / 14",
    "r[3] = 6 / 160 / 7",
    "r[4] = 6 / 160 / 7",
    "r[5] = 6 / 160 / 7",
    "r[6] = 6 / 160 / 7",
    "r[7] = 6 / 160 / 7",
    "r[8] = 6 / 160 / 7",
    "r[9] = 6006 / 160160 / 3007",
    "r[10] = 1 / 1 / 1",
    "r[11] = 6 / 160 / 7",
];

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_spec_module_params_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, DESIGN).expect("write design");
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
fn module_parameter_specialization_is_per_instance() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let got: Vec<&str> = log.lines().filter(|l| l.starts_with("r[")).collect();
        assert_eq!(got, EXPECTED, "XEZIM_COMPILE_METHODS={compile}:\n{log}");
    }
}
