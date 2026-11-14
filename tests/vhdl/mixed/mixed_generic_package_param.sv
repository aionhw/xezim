// A VHDL generic takes its value from a package parameter: by an import
// (wildcard or by name), by its scoped name, through a local parameter and
// in an expression (IEEE 1800-2017 §6.20.2, §26.3).
package gpkg;
  localparam string NAME = "from_pkg";
  localparam int BASE = 5;
  localparam int WIDTH = BASE + 3;
endpackage

package gpkg2;
  import gpkg::*;
  localparam int DEPTH = WIDTH * 2;
endpackage

module mixed_generic_package_param;
  import gpkg::*;
  import gpkg2::DEPTH;
  localparam string LOCAL_NAME = NAME;
  localparam int LOCAL_W = gpkg::WIDTH + 1;
  wire o_lit, o_pkg, o_local, o_scoped;
  wire [7:0] w_pkg, w_local, w_depth;

  gen_ent #(.Name("from_pkg"), .Width(8)) u_lit (.o(o_lit), .w());
  gen_ent #(.Name(NAME), .Width(WIDTH)) u_pkg (.o(o_pkg), .w(w_pkg));
  gen_ent #(.Name(LOCAL_NAME), .Width(LOCAL_W)) u_local (.o(o_local), .w(w_local));
  gen_ent #(.Name(gpkg::NAME), .Width(DEPTH - gpkg::BASE)) u_scoped (.o(o_scoped), .w(w_depth));

  initial begin
    #1;
    $display("o=%b%b%b%b w=%0d %0d %0d", o_lit, o_pkg, o_local, o_scoped, w_pkg, w_local, w_depth);
    $finish;
  end
endmodule
