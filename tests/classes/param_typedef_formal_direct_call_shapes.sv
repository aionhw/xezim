// §8.25, §13.5.1: a compiled method calling another compiled method directly
// binds each argument at its formal's declared width in the receiver's
// specialization: a class typedef sized by a class parameter, a typedef of
// that typedef, a signed one, a packed formal sized by the parameter, the
// one-statement setter shape, and explicit and inherited specializations.
class mem_c #(int AW = 32);
  typedef logic [AW-1:0] addr_t;
  typedef logic signed [AW-1:0] saddr_t;
  typedef addr_t addr2_t;
  logic [7:0] m[addr_t];
  logic [63:0] x;
  function bit has(addr_t a);
    return m.exists(a);
  endfunction
  function bit eqmax(logic [AW-1:0] a);
    return a == {AW{1'b1}};
  endfunction
  function void set(addr_t a);
    x = a;
  endfunction
  function bit neg(saddr_t a);
    return a < 0;
  endfunction
  function bit has2(addr2_t a);
    return m.exists(a);
  endfunction
  function int ones(addr2_t a);
    logic [63:0] t;
    t = a;
    return $countones(t);
  endfunction
endclass
class mem16 extends mem_c #(16);
endclass
class user_c;
  mem_c mm;
  mem_c #(16) m16;
  mem16 e16;
  function new();
    mm = new();
    m16 = new();
    e16 = new();
  endfunction
  function bit p_has(bit [31:0] a);
    return mm.has(a);
  endfunction
  function bit p_eq(bit [63:0] a);
    return mm.eqmax(a);
  endfunction
  function bit p_eq16(bit [63:0] a);
    return m16.eqmax(a);
  endfunction
  function void p_set(bit [63:0] a);
    mm.set(a);
  endfunction
  function void p_set16(bit [63:0] a);
    e16.set(a);
  endfunction
  function bit p_neg(bit [31:0] a);
    return mm.neg(a);
  endfunction
  function bit p_has2(bit [31:0] a);
    return mm.has2(a);
  endfunction
  function int p_ones16(bit [63:0] a);
    return m16.ones(a);
  endfunction
  function bit p_has16(bit [31:0] a);
    return e16.has(a);
  endfunction
endclass
module top;
  user_c u;
  int bad[9];
  initial begin
    u = new();
    u.mm.m[32'h8000_0080] = 8'h65;
    u.e16.m[16'h0080] = 8'h65;
    for (int n = 0; n < 2000; n++) begin
      if (!u.p_has(32'h8000_0080)) bad[0]++;
      if (!u.p_eq(64'h1_ffff_ffff)) bad[1]++;
      if (!u.p_eq16(64'h1_0000_ffff)) bad[2]++;
      u.p_set(64'hffff_ffff_ffff_ffff);
      if (u.mm.x !== 64'h0000_0000_ffff_ffff) bad[3]++;
      u.p_set16(64'hffff_ffff_ffff_ffff);
      if (u.e16.x !== 64'h0000_0000_0000_ffff) bad[4]++;
      if (!u.p_neg(32'h8000_0000)) bad[5]++;
      if (!u.p_has2(32'h8000_0080)) bad[6]++;
      if (u.p_ones16(64'hffff_ffff_ffff_ffff) != 16) bad[7]++;
      if (!u.p_has16(32'h1234_0080)) bad[8]++;
    end
    $display("T|bad=%p", bad);
  end
endmodule
