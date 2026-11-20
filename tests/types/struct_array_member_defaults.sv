// IEEE 1800-2023 §7.2.2: each element of a module-scope fixed array of
// unpacked structures starts with the members' declared defaults, as a scalar
// variable of the type and a block-local array do; the elements read 0.
module top;
  typedef struct { real a = 1.5; real b = 2.5; int c = -3; } SD;
  SD marr [2];
  SD m2 [2][2];
  SD ms;
  initial begin
    SD larr [2];
    $display("T|module %0.2f %0.2f %0d %0.2f %0.2f %0d", marr[0].a, marr[0].b, marr[0].c, marr[1].a, marr[1].b, marr[1].c);
    $display("T|two %0.2f %0d", m2[1][1].a, m2[0][1].c);
    $display("T|scalar %0.2f %0d", ms.a, ms.c);
    $display("T|local %0.2f %0.2f", larr[1].a, larr[1].b);
    marr[1].a = 4.0;
    $display("T|written %0.2f %0.2f", marr[1].a, marr[0].a);
  end
endmodule
