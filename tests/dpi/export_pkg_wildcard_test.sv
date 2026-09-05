// §35.5.4 export "DPI-C" inside a wildcard-imported package.
package regs_pkg;
  int mem [8];
  task p_write(input int addr, input int data); mem[addr] = data; endtask
  function int p_read(input int addr); return mem[addr]; endfunction
  export "DPI-C" task p_write;
  export "DPI-C" function p_read;
  import "DPI-C" function int p_drive();
endpackage
module tb;
  import regs_pkg::*;
  initial begin
    int r = p_drive();
    if (r == 17 && mem[2] == 17) $display("TEST_PASS r=%0d", r);
    else $display("TEST_FAIL r=%0d mem2=%0d", r, mem[2]);
    $finish;
  end
endmodule
