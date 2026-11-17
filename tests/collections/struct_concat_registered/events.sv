// IEEE 1800-2023 §10.10, §9.4.2: a concatenation changes member event values.
`timescale 1ns/1ps
module top;
  typedef struct {int id;} item_t;
  item_t a[2],b,c;
  int woke=0;
  initial begin
    @(a[0].id);
    woke++;
    $display("T|wake|%0d %0d",a[0].id,a[1].id);
  end
  initial begin
    b.id=3; c.id=7;
    #1 a={b,c};
    #1 $display("T|events|%0d",woke);
    $finish;
  end
endmodule
