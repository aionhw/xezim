// #928 follow-up: `foreach` over a packed array of packed structs (and other
// multi-dimensional packed class properties), procedurally and in
// constraints whose body names a field of the element (§7.4.1, §12.7.3,
// §18.5.8.1).
typedef struct packed { logic valid; logic [7:0] mode; } cand_t;
typedef logic [3:0] nib_t;
class fe1; rand cand_t [0:2] c; constraint x { foreach (c[i]) c[i].valid == 1'b1; } endclass
class fe2; rand cand_t [0:2] c; constraint x { foreach (c[i]) { c[i].mode == i + 3; c[i].valid == (i % 2); } } endclass
class fe3; rand cand_t [0:2] c; constraint x { foreach (c[i]) if (i > 0) c[i].mode == c[i-1].mode + 1; c[0].mode == 7; } endclass
class fe4; rand cand_t [0:1][0:2] m; constraint x { foreach (m[i, j]) m[i][j].mode == i * 10 + j; } endclass
class fe5; rand cand_t u[3]; constraint x { foreach (u[i]) u[i].valid == 1'b1; } endclass
class fe6; rand cand_t [2:0] c; constraint x { foreach (c[i]) c[i].mode == i; } endclass
class pl; cand_t [0:2] c; bit [2:0][3:0] w; nib_t [1:3] t; cand_t [0:1][0:2] m;
  function void show();
    foreach (c[k]) $display("T|c k=%0d", k);
    foreach (w[k]) $display("T|w k=%0d", k);
    foreach (t[k]) $display("T|t k=%0d", k);
    foreach (m[k]) $display("T|m k=%0d", k);
    foreach (m[k,j]) $display("T|m2 k=%0d j=%0d", k, j);
    foreach (this.c[k]) $display("T|thisc k=%0d", k);
  endfunction
endclass
module top;
  cand_t [0:2] mc;
  initial begin
    fe1 a1 = new; fe2 a2 = new; fe3 a3 = new; fe4 a4 = new; fe5 a5 = new; fe6 a6 = new;
    pl p = new;
    int ok;
    ok = a1.randomize(); $display("T|fe1 ok=%0d v=%b%b%b", ok, a1.c[0].valid, a1.c[1].valid, a1.c[2].valid);
    ok = a2.randomize(); $display("T|fe2 ok=%0d m=%0d,%0d,%0d v=%b%b%b", ok, a2.c[0].mode, a2.c[1].mode, a2.c[2].mode, a2.c[0].valid, a2.c[1].valid, a2.c[2].valid);
    ok = a3.randomize(); $display("T|fe3 ok=%0d m=%0d,%0d,%0d", ok, a3.c[0].mode, a3.c[1].mode, a3.c[2].mode);
    ok = a4.randomize(); $display("T|fe4 ok=%0d m=%0d,%0d,%0d,%0d,%0d,%0d", ok, a4.m[0][0].mode, a4.m[0][1].mode, a4.m[0][2].mode, a4.m[1][0].mode, a4.m[1][1].mode, a4.m[1][2].mode);
    ok = a5.randomize(); $display("T|fe5 ok=%0d v=%b%b%b", ok, a5.u[0].valid, a5.u[1].valid, a5.u[2].valid);
    ok = a6.randomize(); $display("T|fe6 ok=%0d m=%0d,%0d,%0d", ok, a6.c[0].mode, a6.c[1].mode, a6.c[2].mode);
    p.show();
    foreach (mc[k]) $display("T|mc k=%0d", k);
    foreach (p.c[k]) $display("T|pc k=%0d", k);
  end
endmodule
