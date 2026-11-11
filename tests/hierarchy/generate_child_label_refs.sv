module dut #(parameter int OFFSET=0)(output logic [31:0] a,b,c,d);
  if (1) begin:h
    localparam int V=70;
    function automatic int f(input int x); return x+V; endfunction
  end
  if (1) begin:g
    if (1) begin:h
      localparam int V=2+OFFSET;
      function automatic int f(input int x); return x+V; endfunction
    end
    assign a=h.V;
    assign b=h.f(1);
    if (1) begin:k
      if (1) begin:h localparam int V=4+OFFSET; end
      assign c=h.V;
    end
    for (genvar i=0; i<2; i++) begin:l
      if (1) begin:h localparam int V=10+i+OFFSET; end
    end
    assign d=l[1].h.V;
  end
endmodule
module top;
  logic [31:0] a0,b0,c0,d0,a1,b1,c1,d1;
  dut u0(a0,b0,c0,d0);
  dut #(.OFFSET(20)) u1(a1,b1,c1,d1);
  initial begin
    #1 $display("T|u0=%0d %0d %0d %0d u1=%0d %0d %0d %0d",a0,b0,c0,d0,a1,b1,c1,d1);
    $finish;
  end
endmodule
