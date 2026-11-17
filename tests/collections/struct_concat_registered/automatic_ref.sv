// IEEE 1800-2023 §§6.21, 13.5.2, 10.10: automatic locals and a ref array.
module top;
 typedef struct {int id;} item_t;
 item_t a[2],b,c;
 function automatic int f();
  item_t x[2]; x={b,c}; return x[0].id*10+x[1].id;
 endfunction
 task automatic t();
  item_t x[2]; x={b,c};
  $display("T|task|%0d %0d",x[0].id,x[1].id);
 endtask
 task automatic fill(ref item_t x[2]); x={b,c}; endtask
 initial begin
  b.id=3; c.id=7; t();
  $display("T|function|%0d",f());
  fill(a); $display("T|ref|%0d %0d",a[0].id,a[1].id);
  $finish;
 end
endmodule
