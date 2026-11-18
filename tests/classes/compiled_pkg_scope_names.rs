//! IEEE 1800-2023 §26.3 (#1285): package code resolves a bare name in its
//! own package, whoever calls it, interpreted or compiled. Beyond the
//! package's own parameters: its own variables, names it imports from
//! another package, the parameters an inlined package function reads, a
//! parameterized package class, a static method, an inherited method of a
//! class extended in another package, and package tasks and functions
//! called from another package's code. The module declares a same-named
//! parameter or variable for each. Expected values are the reference
//! simulator's.

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

/// No other package declares the names the module shadows.
#[test]
fn package_code_reads_its_own_scope() {
    if !all_policies() {
        return;
    }
    let src = r#"
package gp;
  parameter int G = 70;
endpackage
package p;
  import gp::*;
  parameter int NUM = 1;
  parameter logic [3:0][7:0] C = {8'hA1, 8'hB2, 8'hC3, 8'hD4};
  int cnt = 5;
  function automatic int get(); return NUM; endfunction
  function automatic string prefix(); return NUM <= 1 ? "" : "multi_"; endfunction
  task automatic tget(output int o); o = NUM + 100; endtask
  class c;
    static function int sget(); return NUM + 10; endfunction
    function int get(); return NUM; endfunction
    function int getc(); return cnt; endfunction
    function int getg(); return G; endfunction
    function logic [7:0] elem(int i); return C[i]; endfunction
    function int shl(int k); return NUM << k; endfunction
    function int callf(); return get() + 20; endfunction
  endclass
  class pc #(int K = 1);
    function int get(); return NUM + K; endfunction
  endclass
endpackage
package q;
  import p::*;
  class user;
    function string label(); return {prefix(), "end"}; endfunction
    function int qget(); return get(); endfunction
    function int qt(); int o; tget(o); return o; endfunction
  endclass
  class d extends p::c;
    function int dget(); return NUM; endfunction
  endclass
  function automatic string qlabel(); return {prefix(), "q"}; endfunction
endpackage
module top;
  parameter int NUM = 4;
  parameter int G = 40;
  int cnt = 9;
  int s [13];
  string l1, l2;
  logic [7:0] e;
  initial begin
    p::c o = new;
    p::pc #(3) o3 = new;
    q::user u = new;
    q::d dd = new;
    for (int i = 0; i < 1500; i++) begin
      s[0] = o.get(); s[1] = o.getc(); s[2] = o.getg(); s[3] = p::c::sget();
      s[4] = o3.get(); s[5] = u.qget(); s[6] = dd.dget(); s[7] = dd.get();
      s[8] = o.shl(4); s[9] = u.qt(); s[10] = p::get(); s[11] = o.callf();
      l1 = u.label(); l2 = q::qlabel();
      e = o.elem(1);
    end
    $display("T|get=%0d getc=%0d getg=%0d sget=%0d pc=%0d qget=%0d", s[0], s[1], s[2], s[3], s[4], s[5]);
    $display("T|dget=%0d inh=%0d shl=%0d qt=%0d pget=%0d e=%h", s[6], s[7], s[8], s[9], s[10], e);
    $display("T|label=%s qlabel=%s callf=%0d", l1, l2, s[11]);
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|get=1 getc=5 getg=70 sget=11 pc=4 qget=1",
            "T|dget=1 inh=1 shl=16 qt=101 pget=1 e=c3",
            "T|label=end qlabel=q callf=21",
        ]
    );
}

/// A third package declares `NUM` and `prefix` too, and the module imports it.
#[test]
fn package_code_reads_its_own_scope_beside_a_contesting_package() {
    if !all_policies() {
        return;
    }
    let src = r#"
package gp;
  parameter int G = 70;
endpackage
package p;
  import gp::*;
  parameter int NUM = 1;
  parameter logic [3:0][7:0] C = {8'hA1, 8'hB2, 8'hC3, 8'hD4};
  int cnt = 5;
  function automatic int get(); return NUM; endfunction
  function automatic int getg(); return G; endfunction
  function automatic string prefix(); return NUM <= 1 ? "" : "multi_"; endfunction
  function automatic int vcnt(); return cnt; endfunction
  class c;
    static function int sget(); return NUM + 10; endfunction
    function int get(); return NUM; endfunction
    function int getc(); return cnt; endfunction
    function int getg(); return G; endfunction
    function int callf(); return get(); endfunction
    function logic [7:0] elem(int i); return C[i]; endfunction
  endclass
endpackage
package r;
  parameter int NUM = 7;
  function automatic string prefix(); return "r_"; endfunction
endpackage
package q;
  import p::*;
  class user;
    function string label(); return {prefix(), "end"}; endfunction
    function int qget(); return get(); endfunction
  endclass
  class d extends p::c;
    function int dget(); return NUM; endfunction
  endclass
  function automatic string qlabel(); return {prefix(), "q"}; endfunction
endpackage
module top;
  import r::*;
  parameter int NUM = 4;
  parameter int G = 40;
  int cnt = 9;
  int s1, s2, s3, s4, s5, s6, s7, s8, s9, s10, s11;
  string l1, l2, l3;
  logic [7:0] e;
  initial begin
    p::c o = new;
    q::user u = new;
    q::d dd = new;
    for (int i = 0; i < 1500; i++) begin
      s1 = o.get(); s2 = o.getc(); s3 = o.getg(); s4 = o.callf(); s5 = p::c::sget();
      s6 = p::getg(); s7 = p::vcnt(); s8 = u.qget(); s9 = dd.dget(); s10 = dd.get(); s11 = dd.getg();
      l1 = u.label(); l2 = q::qlabel(); l3 = prefix();
      e = o.elem(1);
    end
    $display("T|get=%0d getc=%0d getg=%0d callf=%0d sget=%0d", s1, s2, s3, s4, s5);
    $display("T|pgetg=%0d vcnt=%0d qget=%0d dget=%0d e=%h", s6, s7, s8, s9, e);
    $display("T|label=%s qlabel=%s top=%s", l1, l2, l3);
    $display("T|inh get=%0d getg=%0d", s10, s11);
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|get=1 getc=5 getg=70 callf=1 sget=11",
            "T|pgetg=70 vcnt=5 qget=1 dget=1 e=c3",
            "T|label=end qlabel=q top=r_",
            "T|inh get=1 getg=70",
        ]
    );
}
