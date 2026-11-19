//! IEEE 1800-2023 §8.5, §6.20.4: a type written inside a class method is
//! sized in the class scope, where the class's own and inherited body
//! constants are visible: `$bits(logic [W-1:0])` and a method-local
//! `typedef logic [W-1:0] lt;` with `localparam int W` of the class. They
//! were sized without the class constants and came out one bit wide (#1273).
//! §8.23, §8.25.1: a typedef named through a class specialization takes
//! that specialization's parameters, also through a class nested in it
//! (`box #(6)::pay::wide_t`) and through a typedef of the specialization or
//! of the nested class (`b9_t::pay::wide_t`, `p6_t::wide_t`). The `T|g`,
//! `T|h` and `T|i` lines were recorded from the reference simulator; it does
//! not accept the `$bits` spellings of `T|a`, `T|b` and `T|c`, whose values
//! follow from the LRM.

#[test]
fn types_in_class_methods_and_specialized_class_scopes_are_sized() {
    let src = r#"
package pk;
  class box #(int WIDTH = 4);
    class pay;
      typedef logic [WIDTH+3:0] wide_t;
      localparam int N = WIDTH * 2;
      logic [WIDTH-1:0] d;
      function int w(); return $bits(d); endfunction
    endclass
    typedef logic [WIDTH-1:0] own_t;
  endclass
endpackage
class g7; localparam int W = 5;
  function int ib(); return $bits(logic [W-1:0]); endfunction
  function int tb(); typedef logic [W-1:0] lt; lt x; return $bits(x); endfunction
  function int tb2(); typedef logic [W-1:0] lt; return $bits(lt); endfunction
  function int vb(); logic [W-1:0] t; return $bits(t); endfunction
  static function int sb(); return $bits(logic [W+1:0]); endfunction
endclass
class g7d extends g7; localparam int V = 3;
  function int ib2(); return $bits(logic [W+V-1:0]); endfunction
endclass
class g7p #(int P = 2); localparam int W = P + 1;
  function int ib(); return $bits(logic [W-1:0]); endfunction
  function int tb(); typedef logic [W-1:0] lt; lt x; return $bits(x); endfunction
endclass
module top;
  import pk::*;
  typedef box #(.WIDTH(6))::pay p6_t;
  typedef box #(9) b9_t;
  g7 a = new; g7d d = new; g7p #(4) p4 = new; g7p p2 = new;
  initial begin
    $display("T|a %0d %0d", $bits(box #(.WIDTH(6))::pay::wide_t), $bits(box #(6)::pay::wide_t));
    $display("T|b %0d %0d", $bits(p6_t::wide_t), $bits(b9_t::pay::wide_t));
    $display("T|c %0d %0d", $bits(box #(6)::own_t), $bits(b9_t::own_t));
    $display("T|g %0d %0d %0d %0d %0d", a.ib(), a.tb(), a.tb2(), a.vb(), g7::sb());
    $display("T|h %0d %0d %0d", d.ib2(), d.ib(), d.tb());
    $display("T|i %0d %0d %0d %0d", p4.ib(), p4.tb(), p2.ib(), p2.tb());
  end
endmodule
"#;
    let lines: Vec<String> = xezim::simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|a 10 10",
            "T|b 10 13",
            "T|c 6 9",
            "T|g 5 5 5 5 7",
            "T|h 8 5 5",
            "T|i 5 5 3 3"
        ]
    );
}

/// The same when several classes of the design declare a typedef or nest a
/// class of the same name, so that those names go by their class-qualified
/// spellings (§8.23). Expected values follow from the LRM.
#[test]
fn specialized_class_scope_typedefs_with_colliding_names_are_sized() {
    let src = r#"
package g2_pk;
  class g2_alpha_box #(int WIDTH = 4);
    class g2_payload;
      typedef logic [WIDTH+3:0] wide_t;
      localparam int N = WIDTH * 2;
    endclass
  endclass
endpackage
package g3_pk;
  class g3_alpha_box #(int WIDTH = 4);
    class g3_payload;
      typedef logic [WIDTH+3:0] wide_t;
    endclass
  endclass
  class g3_beta_box;
    class g3_payload;
      typedef logic [7:0] wide_t;
    endclass
  endclass
endpackage
module top;
  import g2_pk::*;
  import g3_pk::*;
  typedef g2_alpha_box #(.WIDTH(4))::g2_payload g2_alpha_payload;
  typedef g2_alpha_box #(.WIDTH(7))::g2_payload g2_p7;
  initial begin
    $display("T|a %0d %0d", $bits(g2_alpha_payload::wide_t), $bits(g2_p7::wide_t));
    $display("T|b %0d %0d", $bits(g3_alpha_box #(.WIDTH(4))::g3_payload::wide_t), $bits(g3_alpha_box #(.WIDTH(9))::g3_payload::wide_t));
  end
endmodule
"#;
    let lines: Vec<String> = xezim::simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect();
    assert_eq!(lines, ["T|a 8 11", "T|b 8 13"]);
}
