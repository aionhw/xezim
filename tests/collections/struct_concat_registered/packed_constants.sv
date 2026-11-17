// IEEE 1800-2023 §§7.2.1, 6.20, 11.7, 10.10: packed constants and signedness conversions.
package pkg;
 typedef struct packed {bit signed [31:0] id;} item_t;
 parameter item_t NAME='{13};
endpackage
module top;
 typedef pkg::item_t item_t;
 parameter item_t P='{3};
 localparam item_t L='{7};
 item_t a[6], b;
 initial begin
  b.id=-9;
  a={item_t'{-9},P,L,pkg::NAME,$signed(b.id),$unsigned(b.id)};
  $display("T|packed_constants|%0d %0d %0d %0d %0d %0d",a[0].id,a[1].id,a[2].id,a[3].id,a[4].id,a[5].id);
  $finish;
 end
endmodule
