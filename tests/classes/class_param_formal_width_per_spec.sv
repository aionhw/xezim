// §8.25, §13.5: a formal typed by a class typedef sized by a class
// parameter takes the width of the receiver's own specialization, call by
// call, also when calls on different specializations interleave, when the
// specialization comes through `extends`, and when the range also names a
// package parameter.
package width_pkg;
  parameter int Pad = 4;
endpackage

class mem_model #(int AddrWidth = 24);
  typedef logic [AddrWidth-1:0] addr_t;
  typedef logic [AddrWidth+width_pkg::Pad-1:0] wide_t;
  int addr_bits, wide_bits;

  function void write_byte(addr_t addr, wide_t wide);
    addr_bits = $bits(addr);
    wide_bits = $bits(wide);
  endfunction

  function addr_t pass(addr_t a);
    return a;
  endfunction
endclass

class mem16 extends mem_model #(16);
endclass

module top;
  mem_model d;
  mem_model #(32) m32;
  mem_model #(16) m16;
  mem16 e16;
  int bad;
  initial begin
    d = new;
    m32 = new;
    m16 = new;
    e16 = new;
    for (int i = 0; i < 1500; i++) begin
      d.write_byte(i, i);
      m32.write_byte(i, i);
      m16.write_byte(i, i);
      e16.write_byte(i, i);
      if (d.pass(32'h1234_5600 + i) !== 24'h34_5600 + i) bad++;
      if (m32.pass(32'h1234_5600 + i) !== 32'h1234_5600 + i) bad++;
      if (m16.pass(32'h1234_5600 + i) !== 16'h5600 + i) bad++;
      if (e16.pass(32'h1234_5600 + i) !== 16'h5600 + i) bad++;
    end
    $display("T|addr|%0d %0d %0d %0d", d.addr_bits, m32.addr_bits, m16.addr_bits,
             e16.addr_bits);
    $display("T|wide|%0d %0d %0d %0d", d.wide_bits, m32.wide_bits, m16.wide_bits,
             e16.wide_bits);
    $display("T|pass|%0d %h %h %h %h", bad, d.pass(32'hffff_ffff), m32.pass(32'hffff_ffff),
             m16.pass(32'hffff_ffff), e16.pass(32'hffff_ffff));
  end
endmodule
