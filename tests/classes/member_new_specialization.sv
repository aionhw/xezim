// §8.8, §8.25: `new` assigned to a class member through an object handle
// or `this` builds the specialization the member is declared with, also
// for an element of a member array of handles and for a member declared
// with a typedef of a specialization.
class mem_c #(int AW = 32);
  function int aw(); return AW; endfunction
endclass
typedef mem_c #(7) m7_t;
class base_c;
  mem_c #(6) inh;
endclass
class user_c extends base_c;
  mem_c #(16) m16;
  mem_c #(8) m8;
  mem_c #(4) m4;
  mem_c dflt;
  mem_c #(10) arr[2];
  m7_t ta;
  user_c peer;
  mem_c #(9) m9;
  function new();
    m16 = new();
  endfunction
  function void mk();
    this.m8 = new;
    this.inh = new;
    this.dflt = new;
    this.arr[0] = new;
    arr[1] = new;
    if (peer != null) peer.m9 = new;
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
  user_c u, v;
  holder_c #(5) h;
  initial begin
    u = new();
    v = new();
    u.peer = v;
    u.mk();
    u.m4 = new();
    v.ta = new;
    h = new();
    h.mk();
    $display("T|m16=%0d m8=%0d m4=%0d inh=%0d dflt=%0d own=%0d nxt=%0d", u.m16.aw(), u.m8.aw(),
             u.m4.aw(), u.inh.aw(), u.dflt.aw(), h.own.aw(), h.nxt.aw());
    $display("T|arr0=%0d arr1=%0d ta=%0d peer.m9=%0d", u.arr[0].aw(), u.arr[1].aw(), v.ta.aw(),
             v.m9.aw());
  end
endmodule
