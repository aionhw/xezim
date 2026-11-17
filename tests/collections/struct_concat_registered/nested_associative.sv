// IEEE 1800-2023 §§7.2, 7.8.6, 7.8.7, 10.10: associative members and missing keys.
module top;
 typedef struct {int id; bit [31:0] values[int];} item_t;
 item_t a[2],b,c;
 initial begin
  b.id=3; c.id=7; b.values[1]=31; c.values[2]=72;
  a={b,c};
  $display("T|assoc_members|%0d %0d %0d %0d",a[0].id,a[1].id,a[0].values[1],a[1].values[2]);
  $display("T|missing|%0d %0d",a[0].values[99],a[0].values.exists(99));
  $finish;
 end
endmodule
