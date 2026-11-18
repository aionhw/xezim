// §8.25, §20.6.2: inside a method, `$bits` of a type sized by a class
// parameter, of a class typedef and of a type parameter's default, and a
// packed return sized by a class parameter take the receiver's
// specialization, with calls on different specializations interleaved in a
// design that has unrelated parameters.
class box #(int W = 8, parameter type T = logic [W-1:0]);
  typedef logic [W-1:0] word_t;
  function int tbits(); return $bits(logic [W-1:0]); endfunction
  function int wbits(); return $bits(word_t); endfunction
  function int tpbits(); return $bits(T); endfunction
  function logic [W-1:0] ret(logic [63:0] v); return v; endfunction
endclass
module top;
  for (genvar g = 0; g < 50; g++) begin : pad
    localparam int L = g;
  end
  box d;
  box #(16, logic [15:0]) b16;
  box #(4) b4;
  initial begin
    d = new();
    b16 = new();
    b4 = new();
    for (int i = 0; i < 3; i++) begin
      $display("T|d %0d %0d %0d %h", d.tbits(), d.wbits(), d.tpbits(), d.ret('1));
      $display("T|b16 %0d %0d %0d %h", b16.tbits(), b16.wbits(), b16.tpbits(), b16.ret('1));
      $display("T|b4 %0d %0d %0d %h", b4.tbits(), b4.wbits(), b4.tpbits(), b4.ret('1));
    end
  end
endmodule
