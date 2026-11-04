//! IEEE 1800-2017 §26.2, A.1.11: a package may declare nets. A net with an
//! implicit data type (`wire [p::WIDTH-1:0] tie_low;`, `wire t;`) failed to
//! parse (#602, reported by amal-khailtash); an undriven one reads z (§6.6).
//!
use xezim::simulate;

#[test]
fn package_nets_parse_and_read_z() {
    let src = r#"package p;
  parameter WIDTH = 8;
  wire [p::WIDTH-1:0] tie_low;
  wire t1, t2;
  wire logic [3:0] wl;
  tri [1:0] tr;
  logic [3:0] v = 4'h5;
endpackage
module top;
  import p::*;
  initial begin #1 $display("R %b %b %b %b %h %0d", p::tie_low, t1, wl, tr, v, $bits(p::tie_low)); $finish; end
endmodule
"#;
    let out: Vec<String> = simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("R "))
        .collect();
    assert_eq!(out, ["R zzzzzzzz z zzzz zz 5 8"]);
}
