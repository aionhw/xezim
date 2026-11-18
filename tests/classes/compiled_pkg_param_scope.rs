//! IEEE 1800-2023 §26.3: a bare name in a package subroutine or class method
//! names that package's own parameter, whoever calls it, whether the body is
//! interpreted or runs as bytecode. A select or shift of it takes its declared
//! shape and width, and a block-local array of the same name hides it. A
//! compiled class method used to read the calling module's same-named
//! parameter, and a package function reached only through another package's
//! code ran with no package scope and read x.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

fn all_policies() -> bool {
    super::compiled_method_test_env::policies(&[("1", "0"), ("1", "1000"), ("0", "1000")])
}

const PKGS: &str = r#"
package p;
  parameter int NUM = 1;
  parameter logic [3:0][7:0] C = {8'hA1, 8'hB2, 8'hC3, 8'hD4};
  parameter logic [7:0] B = 1;
  function automatic int get(); return NUM; endfunction
  function automatic logic [7:0] elem(int i); return C[i]; endfunction
  function automatic string prefix(); return NUM <= 1 ? "" : "h0_"; endfunction
  class c;
    function int get(); return NUM; endfunction
    function int shl(int k); return B << k; endfunction
    function bit local_c(); begin bit C [0:0]; return C[0]; end endfunction
  endclass
endpackage
package q;
  import p::*;
  class user;
    function string label(); return {prefix(), "end"}; endfunction
  endclass
endpackage
"#;

const TOP: &str = r#"
module top;
  parameter int NUM = 4;
  localparam int C = 0;
  int s, w;
  bit b;
  logic [7:0] e;
  initial begin
    p::c o = new;
    q::user u = new;
    string l;
    for (int i = 0; i < 1500; i++) begin
      s += p::get() + o.get() + r::other();
      l = u.label();
      e = p::elem(1);
      w = o.shl(8);
      b = o.local_c();
    end
    $display("T|s=%0d l=%s e=%h w=%0d b=%0d top=%0d", s, l, e, w, b, NUM);
  end
endmodule
"#;

#[test]
fn package_parameter_beats_calling_module_parameter() {
    if !all_policies() {
        return;
    }
    let r = "package r;\n  function automatic int other(); return p::get(); endfunction\nendpackage\n";
    assert_eq!(t_lines(&format!("{PKGS}{r}{TOP}")), ["T|s=4500 l=end e=c3 w=256 b=0 top=4"]);
}

#[test]
fn package_parameter_beats_other_package_parameter() {
    if !all_policies() {
        return;
    }
    let r = "package r;\n  parameter int NUM = 7;\n  function automatic int other(); return p::get() + NUM; endfunction\nendpackage\n";
    assert_eq!(t_lines(&format!("{PKGS}{r}{TOP}")), ["T|s=15000 l=end e=c3 w=256 b=0 top=4"]);
}
