`timescale 1ns/1ps
// IEEE 1800-2023 §6.8 Table 6-7, §7.4.2/§7.4.4: each fixed
// unpacked element defaults to its element type's value (real/shortreal 0.0).
// §11.3.1/§11.8.1: arithmetic with a real/shortreal operand is real/shortreal.
module child;
  real a[2];
  shortreal b[2][2];
  real c[2][2][2];
  real initialized[2] = '{2.0, -2.0}; // §10.9.1 assignment pattern
endmodule
module top;
  real a[2];
  shortreal b[2];
  real a2[2][2];
  shortreal b2[2][2];
  real a3[2][2][2];
  shortreal b3[2][2][2];
  real virtual_cells[257];
  shortreal unnamed_cells[100001];
  typedef real real_t; // §6.18: equivalent element type
  real_t alias_cells[2];
  real explicit_cells[2] = '{2.0, -2.0};
  shortreal explicit2[2][2] = '{'{2.0, -2.0}, '{6.0, -6.0}};
  real assigned[2];
  real scalar;
  shortreal short_scalar;
  logic [7:0] four[2];
  bit [7:0] two[2];
  logic signed [7:0] signed_cells[2];
  int int_cells[2];
  child u();
  initial begin
    #1;
    $display("T|one %f %f %f %f", 1/(a[0]+2), 1/(a[1]+2),
             1/(b[0]+2), 1/(b[1]+2));
    $display("T|two %f %f", 1/(a2[1][0]+2), 1/(b2[0][1]+2));
    $display("T|three %f %f", 1/(a3[1][0][1]+2), 1/(b3[0][1][0]+2));
    $display("T|child %f %f %f", 1/(u.a[1]+2), 1/(u.b[1][0]+2),
             1/(u.c[1][0][1]+2));
    $display("T|storage %f %f %f", 1/(virtual_cells[256]+2),
             1/(unnamed_cells[100000]+2), 1/(alias_cells[1]+2));
    $display("T|arithmetic %f %f %f %f", 3/(a[0]+2),
             8'd3/(a[0]+2), -3/(a[0]+2), 1/((a[0]+2)*2));
    // §6.8, §10.9.1: explicit initializers replace the default value.
    assigned[0] = 0.0;
    $display("T|explicit %f %f %f %f", 1/(explicit_cells[0]+2),
             1/(explicit2[1][0]+2), 1/(u.initialized[0]+2), 1/(assigned[0]+2));
    $display("T|scalar %f %f", 1/(scalar+2), 1/(short_scalar+2));
    // §6.8 Table 6-7: integral defaults remain X / zero.
    $display("T|integral %b %b %b %0d", four[1], two[1], signed_cells[1], int_cells[1]);
    // §10.4, §11.4.1: explicit real stores keep working after the default cells.
    a[0] = 2.0; b2[0][1] = 2.0; a3[0][0][0] = -2.0;
    $display("T|stores %f %f %f", 1/(a[0]+2), 1/(b2[0][1]+2),
             1/(a3[0][0][0]+4));
    a[1] += 0.5;
    $display("T|compound %f", a[1]);
    $finish;
  end
endmodule
