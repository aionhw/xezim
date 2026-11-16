// IEEE 1800-2023 §10.10, §10.3, §13.4: a function concatenation reaches compiled reads.
`timescale 1ns/1ps
module top;
  typedef struct {int id;} item_t;
  item_t b,c,a[2];
  wire [31:0] observed;
  bit clk;
  int sampled;
  always @(posedge clk) sampled <= a[0].id*10+a[1].id;
  assign observed=a[0].id*10+a[1].id;
  function automatic int fill();
    a={b,c};
    return a[0].id*10+a[1].id;
  endfunction
  function automatic int read_back();
    return a[0].id*10+a[1].id;
  endfunction
  initial begin
    b.id=3; c.id=7;
    $display("T|function|%0d",fill());
    $display("T|read|%0d",read_back());
    $display("pretty=%p",a);
    #1;
    $display("T|comb_read|%0d",observed);
    clk=1;
    #1;
    $display("T|bytecode|%0d",sampled);
    $finish;
  end
endmodule
