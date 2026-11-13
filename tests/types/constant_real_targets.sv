package p;
  parameter real P=8'd200+8'd100;
endpackage
module leaf;
  typedef real real_t;
  localparam real_t P=8'd200+8'd100;
  real v=8'd200+8'd100;
endmodule
module top;
  class C;
    real v=8'd200+8'd100;
    static real s=8'd200+8'd100;
    localparam real P=8'd200+8'd100;
  endclass
  localparam real P=8'd200+8'd100;
  real v=8'd200+8'd100;
  logic [8:0] u=8'd200+8'd100;
  C c;
  leaf l();
  initial begin
    c=new;
    #1;
    $display("T|targets %f %f %f %f %f %f %f %f %f %0d", P,v,p::P,l.P,l.v,c.v,C::s,C::P,real'(8'd200+8'd100),u);
    $finish;
  end
endmodule
