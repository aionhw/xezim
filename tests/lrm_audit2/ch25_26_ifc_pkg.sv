// Ch.25 interfaces (modports, tasks in iface), Ch.26 packages
package audit_pkg;
  parameter int PKG_P = 17;
  typedef enum {P_A, P_B, P_C} pk_e;
  function automatic int pkg_double(int x); return 2 * x; endfunction
endpackage

interface bus_if (input logic clk);
  logic [7:0] data;
  logic valid;
  modport drv (output data, valid, input clk);
  modport mon (input data, valid, clk);
  function automatic logic [7:0] snoop(); return data; endfunction
endinterface

module driver (bus_if.drv b);
  initial begin
    #1;
    b.data = 8'h3C;
    b.valid = 1;
  end
endmodule

module monitor (bus_if.mon b, output logic [7:0] seen, output logic ok);
  initial begin
    #2;
    seen = b.data;
    ok = b.valid;
  end
endmodule

module tb;
  import audit_pkg::*;
  int fails = 0;
  `define CK(name, cond) if (!(cond)) begin $display("FAIL[25] %s", name); fails++; end
  logic clk = 0;
  logic [7:0] seen;
  logic ok;
  bus_if bus (.clk(clk));
  driver u_d (.b(bus));
  monitor u_m (.b(bus), .seen(seen), .ok(ok));
  initial begin
    #3;
    `CK("modport drive+observe", seen == 8'h3C && ok == 1)
    `CK("iface function", bus.snoop() == 8'h3C)
    `CK("pkg param import", PKG_P == 17)
    `CK("pkg enum", P_B == 1)
    `CK("pkg function", pkg_double(21) == 42)
    `CK("pkg scope op", audit_pkg::PKG_P == 17)
    begin // virtual interface basics
      virtual bus_if vif;
      vif = bus;
      `CK("vif read", vif.data == 8'h3C)
      vif.data = 8'h55;
      #1;
      `CK("vif write", bus.data == 8'h55)
    end
    $display("CH25 CHECKS DONE fails=%0d", fails);
  end
endmodule
