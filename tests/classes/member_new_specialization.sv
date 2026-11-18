// §8.8, §8.25: `new` assigned to a class member through a handle or
// `this` builds the specialization the member is declared with.
class mem_c #(int AW = 32);
  function int aw(); return AW; endfunction
endclass
class base_c;
  mem_c #(6) inh;
endclass
class user_c extends base_c;
  mem_c #(16) m16;
  mem_c #(8) m8;
  mem_c #(4) m4;
  mem_c dflt;
  function new();
    m16 = new();
  endfunction
  function void mk();
    this.m8 = new;
    this.inh = new;
    this.dflt = new;
  endfunction
endclass
class holder_c #(int N = 3);
  mem_c #(N) own;
  mem_c #(N + 1) nxt;
  function void mk();
    this.own = new();
    nxt = new();
  endfunction
endclass
module top;
  user_c u;
  holder_c #(5) h;
  initial begin
    u = new();
    u.mk();
    u.m4 = new();
    h = new();
    h.mk();
    $display("T|m16=%0d m8=%0d m4=%0d inh=%0d dflt=%0d own=%0d nxt=%0d", u.m16.aw(), u.m8.aw(),
             u.m4.aw(), u.inh.aw(), u.dflt.aw(), h.own.aw(), h.nxt.aw());
  end
endmodule
