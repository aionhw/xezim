//! `$typename` must reflect the actual type, preserving vector widths.
//!
//! (1) `$typename(T)` where `T` is a TYPE PARAMETER of the active class
//! specialization reports the CONCRETE bound type — not the literal param
//! name and not a generic fallback. A parameterized event callback (for
//! T = uvm_object, string, and uvm_bitstream_t) logs `$typename(T)` in each
//! `do_it()`; pre-fix every specialization reported `logic` regardless of `T`.
//!
//! (2) Per IEEE 1800-2017 §20.6.1 the vector RANGE is preserved: a typedef'd
//! or declared `logic signed [15:0]` reports `logic signed [15:0]`, not a
//! bare `logic`.
use std::process::Command;

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

fn run(src: &str) -> String {
    std::fs::write("/tmp/typename_type_param.sv", src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", "/tmp/typename_type_param.sv"])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const SRC: &str = r#"module top;
  typedef logic signed [15:0] my_bits_t;
  typedef bit [7:0] my_bytes_t;
  class a; endclass
  logic [9:0] vec_sig;
  class ger#(type T=a);
    static function void do_it();
      $display("RESULT T=%s int=%s str=%s", $typename(T), $typename(int), $typename(string));
    endfunction
  endclass
  initial begin
    my_bits_t bv;
    my_bytes_t by;
    ger#(a)::do_it();
    ger#(my_bits_t)::do_it();
    ger#(string)::do_it();
    $display("RESULT td=%s tdbit=%s", $typename(my_bits_t), $typename(my_bytes_t));
    $display("RESULT sig=%s if-local=%s", $typename(vec_sig), $typename(bv));
    $finish;
  end
endmodule
"#;

#[test]
fn typename_type_param_resolves_concrete_binding() {
    let out = run(SRC);
    let lines: Vec<&str> = out.lines().filter(|l| l.contains("RESULT")).collect();
    assert!(
        lines.iter().any(|l| l.contains("T=class a")),
        "class-typed type param must report `class a`:
{out}"
    );
    assert!(
        lines.iter().any(|l| l.contains("T=logic signed [15:0]")),
        "a typedef'd vector type param must keep the base `logic signed [15:0]`, not a bare `logic`:
{out}"
    );
    assert!(
        lines.iter().any(|l| l.contains("str=string")) && lines.iter().any(|l| l.contains("int=int")),
        "builtin type params must report `int` and `string`:
{out}"
    );
    assert!(
        lines.iter().any(|l| l.contains("td=logic signed [15:0]"))
            && lines.iter().any(|l| l.contains("tdbit=bit [7:0]")),
        "direct `$typename(typedef)` must preserve range/base (logic signed [15:0], bit [7:0]):\n{out}"
    );
    assert!(
        lines.iter().any(|l| l.contains("sig=logic [9:0]"))
            && lines.iter().any(|l| l.contains("if-local=logic signed [15:0]")),
        "vector signals and block-local vars must report their range (logic [9:0], logic signed [15:0]):\n{out}"
    );
}