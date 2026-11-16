// IEEE 1800-2023 §§26, 10.10: package array target and operands.
package pkg;
 typedef struct {int id;} item_t;
 item_t a[2], b,c;
endpackage
module top;
 initial begin
  pkg::b.id=3; pkg::c.id=7; pkg::a={pkg::b,pkg::c};
  $display("T|package|%0d %0d",pkg::a[0].id,pkg::a[1].id);
  $finish;
 end
endmodule
