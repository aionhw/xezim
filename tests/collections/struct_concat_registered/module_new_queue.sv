// IEEE 1800-2023 §§7.5.1, 7.10, 10.10: allocated dynamic and queue targets.
module top;
 typedef struct {int id;} item_t;
 item_t a[2], b, c, d[], q[$];
 initial begin
  b.id=3; c.id=7; d=new[2]; d[0].id=91; d[1].id=92;
  a={b,c}; d={b,c}; q={b,c};
  $display("T|module|%0d %0d",a[0].id,a[1].id);
  $display("T|new|%0d %0d %0d",d.size(),d[0].id,d[1].id);
  $display("T|queue|%0d %0d %0d",q.size(),q[0].id,q[1].id);
  $finish;
 end
endmodule
