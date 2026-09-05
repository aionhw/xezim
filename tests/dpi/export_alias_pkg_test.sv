// §35.5.4 export "DPI-C" linkage names: alias form, package scope without an
// import, and compilation-unit alias. See export_alias_pkg.c.
int tag;
task sv_tag(input int v); tag = v; endtask
export "DPI-C" unit_tag = task sv_tag;

package store_pkg;
  int mirror [8];
  task pkg_store(input int addr, input int data); mirror[addr] = data; endtask
  function int pkg_load(input int addr); return mirror[addr]; endfunction
  export "DPI-C" task pkg_store;
  export "DPI-C" function pkg_load;
endpackage

module tb;
  int mem [8];
  task sv_store(input int addr, input int data); mem[addr] = data; endtask
  export "DPI-C" c_store = task sv_store;
  import "DPI-C" function int c_drive();
  initial begin
    int r = c_drive();
    if (r == 42 && mem[3] == 42 && tag == 7)
      $display("TEST_PASS r=%0d mem3=%0d tag=%0d", r, mem[3], tag);
    else
      $display("TEST_FAIL r=%0d mem3=%0d tag=%0d", r, mem[3], tag);
    $finish;
  end
endmodule
