// §6.18, §8.25: a class typedef of a class typedef sized by a class
// parameter names the same type: formals, locals and returns declared with
// it take the receiver's specialization width.
class box #(int W = 12);
  typedef logic [W-1:0] word_t;
  typedef word_t alias_t;
  typedef alias_t alias2_t;
  function int fbits(alias_t a); return $bits(a); endfunction
  function int ones(alias2_t a);
    logic [63:0] t;
    t = a;
    return $countones(t);
  endfunction
  function int local_ones(logic [63:0] v);
    alias_t l;
    logic [63:0] t;
    l = v;
    t = l;
    return $countones(t);
  endfunction
  function alias_t pass(logic [63:0] v); return v; endfunction
endclass
module top;
  box b12;
  box #(20) b20;
  initial begin
    b12 = new();
    b20 = new();
    $display("T|w12 %0d %0d %0d %h", b12.fbits(0), b12.ones('1), b12.local_ones('1), b12.pass('1));
    $display("T|w20 %0d %0d %0d %h", b20.fbits(0), b20.ones('1), b20.local_ones('1), b20.pass('1));
  end
endmodule
