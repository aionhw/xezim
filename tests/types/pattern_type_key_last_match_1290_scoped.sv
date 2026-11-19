// §10.9.1: a later typedef key of the same type as an earlier built-in key
// matches the same elements, and the last matching type key wins: a class-scope
// typedef, and a typedef declared in a generate block driving a resolved net.
class PC #(type T = int);
  typedef integer cw_t;
endclass
module top;
  typedef struct {integer a[3];} S;
  function automatic S f(input S d[]);
    S s;
    for (int i = 0; i < 3; i++) s.a[i] = d[0].a[i];
    return s;
  endfunction
  nettype S N with f;
  S v;
  integer x = 7;
  for (genvar gi = 1; gi < 2; gi++) begin : g
    typedef integer gw_t;
    N n1, n2;
    assign n1 = '{a:'{integer:x, gw_t:99, default:99}};
    assign n2 = '{a:'{integer:x, 2:5, gw_t:99, default:99}};
  end
  initial begin
    v = '{a:'{integer:x, PC#(integer)::cw_t:99, default:99}};
    #1;
    $display("T|class %0d %0d %0d", v.a[0], v.a[1], v.a[2]);
    $display("T|generate %0d %0d %0d", g[1].n1.a[0], g[1].n1.a[1], g[1].n1.a[2]);
    $display("T|generate index %0d %0d %0d", g[1].n2.a[0], g[1].n2.a[1], g[1].n2.a[2]);
  end
endmodule
