// IEEE 1800-2023 §§6.20, 10.9.2, 10.10: typed literals and struct constants.
package pkg;
 typedef struct {int id;} item_t;
 parameter item_t NAME='{13};
endpackage
module top;
 typedef pkg::item_t item_t;
 parameter item_t P='{3};
 localparam item_t L='{7};
 item_t a[4];
 initial begin
  a={item_t'{-9},P,L,pkg::NAME};
  $display("T|unpacked_constants|%0d %0d %0d %0d",a[0].id,a[1].id,a[2].id,a[3].id);
  $finish;
 end
endmodule
