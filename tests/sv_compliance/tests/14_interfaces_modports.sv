`include "../common/svtest_defs.svh"

interface req_gnt_if;
  logic req;
  logic gnt;

  modport master (output req, input gnt);
  modport slave  (input req, output gnt);
endinterface

module req_master(req_gnt_if.master bus);
  initial bus.req = 1'b1;
endmodule

module req_slave(req_gnt_if.slave bus);
  always @(*) bus.gnt = bus.req;
endmodule

module test_interfaces_modports;
  `SVTEST_INIT

  req_gnt_if bus();
  req_master m(bus);
  req_slave  s(bus);

  initial begin
    #1;
    `SVTEST_CHECK(bus.req == 1'b1, "interface master drive failed")
    `SVTEST_CHECK(bus.gnt == 1'b1, "interface slave observe/respond failed")

    `SVTEST_PASSFAIL
  end
endmodule
