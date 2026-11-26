// contested aggregate shapes: another module declares each name, so references are rooted
typedef struct { int a; int b; } st_t;
typedef struct packed { logic [3:0] hi, lo; } pk_t;
class Obj; int v; function new(int x); v = x; endfunction function int get(); return v; endfunction endclass
int   q [$];
int   aa [int];
int   dyn [];
st_t  st;
pk_t  pk;
Obj   ob;
string s = "a";
real  r = 0.5;
event ev;
int   fx [2][3];
module other;  // declares every name itself
  int q [$]; int aa [int]; int dyn []; st_t st; pk_t pk; Obj ob; string s = "o"; real r; event ev; int fx [2][3];
  initial begin q.push_back(99); aa[1] = 99; st.a = 99; pk = 8'h99; s = "other"; fx[1][2] = 99; end
endmodule
module wr;
  initial begin
    #1; q.push_back(1); q.push_back(2); aa[5] = 50; dyn = new[3]; dyn[2] = 7; st.a = 3; st.b = 4;
    pk.hi = 4'hc; pk[1:0] = 2'b11; ob = new(42); s = {s, "b"}; r = r + 1.0; fx[1][2] = 6;
    #1; -> ev;
  end
endmodule
module rdm(input int p, input logic [3:0] hi);
  initial begin
    #2; $display("T|rd q=%p n=%0d aa=%0d dyn=%0d/%0d st=%0d,%0d pk=%h ob=%0d s=%s r=%0.1f fx=%0d p=%0d hi=%h",
                 q, q.size(), aa[5], dyn.size(), dyn[2], st.a, st.b, pk, ob.get(), s, r, fx[1][2], p, hi);
    foreach (q[i]) $display("T|rd q[%0d]=%0d", i, q[i]);
    if (aa.exists(5)) $display("T|rd aa has 5, sum=%0d", q.sum());
  end
  initial begin @(ev); $display("T|rd ev at %0t", $time); end
  initial begin wait (st.b == 4); $display("T|rd wait at %0t", $time); end
endmodule
module top;
  other o(); wr w(); rdm r1(st.a + q.size(), pk.hi);
  initial begin #3; $display("T|top q=%p o.q=%p st.a=%0d o.st.a=%0d s=%s o.s=%s", q, o.q, st.a, o.st.a, s, o.s); $finish; end
endmodule
