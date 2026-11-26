//! #1273: unrelated declarations that share a bare name keep their own
//! scope (§8.23, §8.25, §6.18, §16.8, §26.3).
//!
//! - A class nested in a parameterized host, named through a specialization
//!   (`alpha_mw #(.WIDTH(4))::req_mw`), while another class nests one of the
//!   same name or a top-level class has it: its constructor, methods,
//!   statics, base class and `randomize()` are its own.
//! - A class property typed by the class's own typedef (and enum base type)
//!   sized by a class `localparam`; package-class locals and type
//!   expressions sized by a package parameter.
//! - A named property or sequence two packages declare: a wildcard import,
//!   an explicit import, a package-qualified instance and a module's own
//!   declaration each run their own body.
//!
//! Expected values are the reference simulator's.

use xezim::simulate;

#[test]
fn bare_names_keep_their_own_scope() {
    let src = r#"
package pk;
  class alpha_mw #(int WIDTH = 4);
    class req_mw;
      logic [39:0] tag; int stamp;
      function new(); stamp = 100; void'(std::randomize(tag)); endfunction
      function int f(); return 11; endfunction
    endclass
  endclass
  class beta_mw;
    class req_mw;
      int stamp;
      function new(); stamp = 200; endfunction
      function int f(); return 22; endfunction
    endclass
  endclass
  class req_x3; int stamp; function new(); stamp = 222; endfunction endclass
  class alpha_x3 #(int WIDTH = 4);
    class req_x3; int stamp; function new(); stamp = 111; endfunction endclass
  endclass
  class alpha_x4 #(int WIDTH = 4);
    class req_x4; static int count = 7; function new(); count = count + 1; endfunction endclass
  endclass
  class beta_x4;
    class req_x4; static int count = 99; endclass
  endclass
  class base_x5; int v; function new(); v = 1; endfunction endclass
  class alpha_x5 #(int WIDTH = 4);
    class req_x5 extends base_x5; function new(); super.new(); v = v + 40; endfunction endclass
  endclass
  class beta_x5;
    class req_x5 extends base_x5; function new(); super.new(); v = v + 1; endfunction endclass
  endclass
  class alpha_x6 #(int WIDTH = 4);
    class mid_x6; class deep_x6; int v; function new(); v = 333; endfunction endclass endclass
  endclass
  class beta_x6;
    class mid_x6; class deep_x6; int v; function new(); v = 777; endfunction endclass endclass
  endclass
endpackage
module m_nested;
  import pk::*;
  typedef alpha_mw #(.WIDTH(4))::req_mw a_t;
  typedef beta_mw::req_mw b_t;
  typedef alpha_x3 #(.WIDTH(4))::req_x3 x3_t;
  typedef alpha_x4 #(.WIDTH(4))::req_x4 x4_t;
  typedef alpha_x5 #(.WIDTH(4))::req_x5 x5_t;
  typedef alpha_x6 #(.WIDTH(4))::mid_x6::deep_x6 x6_t;
  a_t a; b_t b; x3_t x3; req_x3 t3; x4_t x4; x5_t x5; x6_t x6;
  initial begin
    a = new; b = new; x3 = new; t3 = new; x4 = new; x5 = new; x6 = new;
    $display("T|mw a.stamp=%0d a.f=%0d tag_known=%0d b.stamp=%0d b.f=%0d", a.stamp, a.f(), !$isunknown(a.tag), b.stamp, b.f());
    $display("T|x3=%0d t3=%0d x4=%0d x5=%0d x6=%0d", x3.stamp, t3.stamp, x4.count, x5.v, x6.v);
  end
endmodule
package pa;
  property ok(int v); (v == 111); endproperty
  sequence s_eq(int v); (v == 111); endsequence
endpackage
package pb;
  property ok(int v); (v == 222); endproperty
  sequence s_eq(int v); (v == 222); endsequence
endpackage
module m_wild;
  import pb::*;
  logic clk = 0;
  always #5 clk = ~clk;
  int v = 222;
  int bad = 0, bad_s = 0;
  assert property (@(posedge clk) ok(v)) else bad = 1;
  assert property (@(posedge clk) s_eq(v)) else bad_s = 1;
  initial #12 $display("T|wild bad=%0d bad_s=%0d", bad, bad_s);
endmodule
module m_expl;
  import pa::ok;
  logic clk = 0;
  always #5 clk = ~clk;
  int v = 111;
  int bad = 0;
  assert property (@(posedge clk) ok(v)) else bad = 1;
  initial #12 $display("T|expl bad=%0d", bad);
endmodule
module m_qual;
  logic clk = 0;
  always #5 clk = ~clk;
  int v = 222;
  int bad = 0;
  assert property (@(posedge clk) pb::ok(v)) else bad = 1;
  initial #12 $display("T|qual bad=%0d", bad);
endmodule
module m_own;
  import pa::*;
  logic clk = 0;
  always #5 clk = ~clk;
  property ok(int v); (v == 333); endproperty
  int v = 333;
  int bad = 0;
  assert property (@(posedge clk) ok(v)) else bad = 1;
  initial #12 $display("T|own bad=%0d", bad);
endmodule
class g6_box;
  localparam int W = 4;
  logic [W-1:0] direct;
  typedef logic [W-1:0] word_t;
  word_t via_td;
  typedef enum logic [W-1:0] {G6_E0, G6_E1} e_t;
  e_t e;
  function int td_bits(); return $bits(via_td); endfunction
  function int enum_bits(); return $bits(e); endfunction
  function int direct_bits(); return $bits(direct); endfunction
endclass
package g8_pk;
  parameter int g8_W = 4;
  class g8_box;
    logic [g8_W-1:0] prop;
    function int local_bits(); logic [g8_W-1:0] tmp; return $bits(tmp); endfunction
    function int inline_bits(); return $bits(logic [g8_W-1:0]); endfunction
  endclass
endpackage
module m_widths;
  initial begin
    g6_box b = new;
    g8_pk::g8_box p = new;
    $display("T|w6 direct=%0d td=%0d enum=%0d", b.direct_bits(), b.td_bits(), b.enum_bits());
    $display("T|w8 prop=%0d local=%0d inline=%0d", $bits(p.prop), p.local_bits(), p.inline_bits());
  end
endmodule
module top;
  m_nested n();
  m_widths w();
  m_wild wi();
  m_expl e();
  m_qual q();
  m_own o();
  initial #20 $finish;
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let mut lines: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect();
    lines.sort();
    assert_eq!(
        lines,
        [
            "T|expl bad=0",
            "T|mw a.stamp=100 a.f=11 tag_known=1 b.stamp=200 b.f=22",
            "T|own bad=0",
            "T|qual bad=0",
            "T|w6 direct=4 td=4 enum=4",
            "T|w8 prop=4 local=4 inline=4",
            "T|wild bad=0 bad_s=0",
            "T|x3=111 t3=222 x4=8 x5=41 x6=333",
        ]
    );
}
