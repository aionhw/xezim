// IEEE 1800-2023 §§10.9.2, 11.7, 10.10: signed and unsigned members of typed operands.
module top;
 typedef struct {int id;} item_t;
 item_t a[2],b;
 initial begin
  b.id=-9;
  a={item_t'{$signed(b.id)},item_t'{$unsigned(b.id)}};
  $display("T|signed_members|%0d %0d",a[0].id,a[1].id);
  $finish;
 end
endmodule
