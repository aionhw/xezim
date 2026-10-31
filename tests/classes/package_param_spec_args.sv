// §8.25: a value parameter given as a constant expression of package
// parameters (`cfg_pkg::N + cfg_pkg::K`) keys the specialization by its
// value, the same as the literal or an imported localparam with that value.
// §25.9: a virtual interface type is identified by the interface and its
// parameter values.
package cfg_pkg;
  parameter int unsigned K = 128;
  parameter int unsigned N = 64;
  localparam int unsigned W = N + K;
endpackage

interface ppif #(parameter int H = 32, parameter int D = H);
  logic [D-1:0] d;
endinterface

class db #(type T = int);
  static T val;
  static bit set_done;
  static function void set(T v); val = v; set_done = 1; endfunction
  static function bit has(); return set_done; endfunction
endclass

class agent #(int H = 32, int D = H);
  virtual ppif #(H, D) vif;
  static function bit has(); return db#(virtual ppif#(H, D))::has(); endfunction
  function bit get();
    if (!has()) return 0;
    vif = db#(virtual ppif#(H, D))::val;
    return 1;
  endfunction
endclass

typedef virtual ppif#(.D(cfg_pkg::N + cfg_pkg::K)) vif_expr_t;
typedef virtual ppif#(.D(cfg_pkg::W)) vif_scoped_t;
typedef virtual ppif#(.D(192)) vif_lit_t;
typedef virtual ppif#(32, 192) vif_pos_t;
typedef virtual ppif#(.D(cfg_pkg::N + 1)) vif_other_t;
typedef agent#(.D(cfg_pkg::N + cfg_pkg::K)) agent_expr_t;
typedef agent#(.D(cfg_pkg::W)) agent_scoped_t;
typedef agent#(cfg_pkg::W - 160, cfg_pkg::W) agent_pos_t;
typedef agent#(.D(192)) agent_lit_t;
typedef agent#(.D(cfg_pkg::N)) agent_other_t;

module top;
  import cfg_pkg::*;
  ppif #(.D(cfg_pkg::N + cfg_pkg::K)) i ();
  agent_expr_t a;
  initial begin
    db#(vif_expr_t)::set(i);
    $display("T|lit|%0d", db#(vif_lit_t)::has());
    $display("T|pos|%0d", db#(vif_pos_t)::has());
    $display("T|scoped|%0d", db#(vif_scoped_t)::has());
    $display("T|imported|%0d", db#(virtual ppif#(.D(W)))::has());
    $display("T|inline_lit|%0d", db#(virtual ppif#(32, 192))::has());
    $display("T|inline_expr|%0d", db#(virtual ppif#(32, cfg_pkg::N + cfg_pkg::K))::has());
    $display("T|other|%0d", db#(vif_other_t)::has());
    $display("T|agent_expr|%0d", agent_expr_t::has());
    $display("T|agent_scoped|%0d", agent_scoped_t::has());
    $display("T|agent_pos|%0d", agent_pos_t::has());
    $display("T|agent_lit|%0d", agent_lit_t::has());
    $display("T|agent_other|%0d", agent_other_t::has());
    a = new;
    $display("T|agent_get|%0d", a.get());
    $display("T|vif_width|%0d", $bits(a.vif.d));
  end
endmodule
