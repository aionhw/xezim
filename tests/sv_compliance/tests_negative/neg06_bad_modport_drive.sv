// EXPECT: compile_fail
interface neg06_if;
  logic req;
  modport sink(input req);
endinterface

module neg06_bad_modport_drive(neg06_if.sink b);
  initial begin
    b.req = 1'b1;
  end
endmodule
