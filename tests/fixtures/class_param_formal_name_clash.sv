// A package parameter W shares its name with a
// class parameter W. The formal [W-1:0] of memory_t#(8) is 8 bits wide, so
// the actual (a + b) >> 1 is evaluated at 8 bits (IEEE 1800-2017 11.6.1,
// 13.5, 8.25): (8'hff + 8'hff) truncates to 8'hfe, >> 1 gives 8'h7f.
package pkg;
  parameter int W = 16;
endpackage
class memory_t #(int W = 8);
  bit [W-1:0] v;
  function void set(bit [W-1:0] x); v = x; endfunction
  function void set_default(bit [W-1:0] x = (8'hff + 8'hff) >> 1); v = x; endfunction
endclass
module top;
  initial begin
    memory_t #(8) m = new();
    bit [7:0] a = 8'hff, b = 8'hff;
    m.set((a + b) >> 1);
    if (m.v !== 8'h7f) $fatal(1, "MISMATCH explicit actual=%h expected 7f", m.v);
    m.set_default();
    if (m.v !== 8'h7f) $fatal(1, "MISMATCH default actual=%h expected 7f", m.v);
    $display("CLASS_PARAM_FORMAL_NAME_CLASH_PASS");
    $finish;
  end
endmodule
