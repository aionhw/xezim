// §8.25, §6.18: inside a parameterized class, method formals and locals
// typed by a class typedef or a packed range over a class parameter or
// localparam have the widths of the object's specialization, also once the
// method has been called often enough to run compiled.
class byte_mem #(int AddrWidth = 32, int DataWidth = 32);
  localparam int MaskWidth = DataWidth / 8;
  typedef logic [AddrWidth-1:0] addr_t;
  typedef logic [DataWidth-1:0] data_t;
  typedef logic [MaskWidth-1:0] mask_t;
  logic [7:0] bytes[bit [63:0]];

  function void write(input addr_t addr, data_t data, mask_t mask = '1);
    for (int i = 0; i < DataWidth / 8; i++) begin
      if (mask[0]) bytes[addr + i] = data[7:0];
      data = data >> 8;
      mask = mask >> 1;
    end
  endfunction

  function data_t read(addr_t addr, mask_t mask = '1);
    data_t data;
    for (int i = DataWidth / 8 - 1; i >= 0; i--) begin
      data = data << 8;
      if (mask[MaskWidth - 1]) data[7:0] = bytes[addr + i];
      mask = mask << 1;
    end
    return data;
  endfunction

  function addr_t pass(addr_t a);
    return a;
  endfunction
  function addr_t pass_by_name(addr_t a);
    pass_by_name = a;
  endfunction
  typedef bit [DataWidth-1:0] word_t;
  function bit known(word_t w);
    return !$isunknown(w);
  endfunction

  function void widths(string tag, mask_t m, logic [MaskWidth-1:0] dm, data_t d);
    mask_t lm;
    logic [MaskWidth-1:0] ldm;
    data_t ld;
    logic [DataWidth-1:0] ldd;
    $display("T|%s_formal|%0d %0d %0d", tag, $bits(m), $bits(dm), $bits(d));
    $display("T|%s_local|%0d %0d %0d %0d", tag, $bits(lm), $bits(ldm), $bits(ld), $bits(ldd));
    $display("T|%s_formal_val|%0h %0h", tag, m, dm);
  endfunction
endclass

module top;
  byte_mem m32;
  byte_mem #(32, 64) m64;
  byte_mem #(40) m40;
  logic [31:0] r;
  int bad32, bad40, bad40n;
  initial begin
    m32 = new;
    m64 = new;
    m40 = new;
    m32.write(32'h8000_0000, 32'h1234_5678);
    r = m32.read(32'h8000_0000);
    $display("T|mem|%0d %h", m32.bytes.size(), r);
    m32.widths("d32", 16'hffff, 16'hffff, 0);
    m64.widths("d64", 16'hffff, 16'hffff, 0);
    for (int i = 0; i < 1500; i++) begin
      if (m32.pass(32'h8000_0000 + i) !== 32'h8000_0000 + i) bad32++;
      if (m40.pass(40'h80_0000_0000 + i) !== 40'h80_0000_0000 + i) bad40++;
      if (m40.pass_by_name(40'h80_0000_0000 + i) !== 40'h80_0000_0000 + i) bad40n++;
    end
    $display("T|hot_pass|%0d %0d %0d %0d", bad32, bad40, bad40n, m32.known(32'hx));
    for (int i = 0; i < 1500; i++) m32.write(32'h9000_0000 + 4 * i, 32'h0403_0201, 4'b0101);
    $display("T|hot_write|%0d %h %h", m32.bytes.size(),
             m32.bytes[32'h9000_176c], m32.bytes[32'h9000_176e]);
  end
endmodule
