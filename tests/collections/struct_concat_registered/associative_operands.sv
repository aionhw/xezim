// IEEE 1800-2023 §§7.8.6, 10.10: associative elements, including a missing key.
module top;
 typedef struct {bit signed [31:0] id;} item_t;
 item_t singles[int], b,c,a[3];
 initial begin
  b.id=3; c.id=7; singles[1]=b; singles[2]=c;
  a={singles[1],singles[2],singles[99]};
  $display("T|assoc_operand|%0d %0d %0d %0d",a[0].id,a[1].id,a[2].id,singles.exists(99));
  $finish;
 end
endmodule
