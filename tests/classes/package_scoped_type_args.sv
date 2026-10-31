// §6.22.1: a package-scoped type name (`p::e_t`) and the same name brought
// in by an import denote one type, so `db#(p::e_t)` and `db#(e_t)` are one
// specialization (§8.25) and share their statics. `q::w_t`, a different
// type under the same name in another package, stays its own specialization.
package p;
  typedef enum integer { A = 0, B = 1 } e_t;
  typedef struct packed { logic [3:0] f; } s_t;
  typedef logic [7:0] b_t;
  typedef logic [7:0] w_t;
  class c; endclass
  class pc #(int W = 1); endclass
endpackage

package q;
  typedef logic [15:0] w_t;
endpackage

class db #(type T = int);
  static T val;
  static bit set_done;
  static function void set(T v); val = v; set_done = 1; endfunction
  static function bit has(); return set_done; endfunction
endclass

package user_pkg;
  import p::*;
  class user;
    static function int get_enum();
      e_t v;
      if (!db#(e_t)::has()) return -1;
      v = db#(e_t)::val;
      return v;
    endfunction
  endclass
endpackage

module top;
  import p::*;
  p::c co;
  p::pc #(3) pco;
  initial begin
    co = new;
    pco = new;
    db#(p::e_t)::set(p::B);
    db#(p::s_t)::set('0);
    db#(p::b_t)::set('0);
    db#(p::w_t)::set('0);
    db#(p::c)::set(co);
    db#(p::pc#(3))::set(pco);
    $display("T|enum|%0d", db#(e_t)::has());
    $display("T|enum_val|%0d", user_pkg::user::get_enum());
    $display("T|struct|%0d", db#(s_t)::has());
    $display("T|vec|%0d", db#(b_t)::has());
    $display("T|class|%0d", db#(c)::has());
    $display("T|pclass|%0d", db#(pc#(3))::has());
    $display("T|pclass_other|%0d", db#(pc#(4))::has());
    $display("T|q_w|%0d", db#(q::w_t)::has());
  end
endmodule
