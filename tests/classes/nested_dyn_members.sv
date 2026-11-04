// IEEE 1800-2023 §7.4.5 / §7.5 / §18.4 / §21.2.1.7: a class member with a
// dynamic or queue outer dimension over unpacked rows (`T a[][N]`,
// `T a[][N][M]`, `T a[$][N]`, `T a[][]`): element access from a method and
// through a handle, `size()`, `foreach`, `%p`, the copy constructor and
// randomization.
typedef struct packed { logic en; logic [34:0] data; logic stop; } narrow_t;

class store;
  bit [7:0] raw[][2];
  narrow_t  cells[][2];
  int       d[][];
  narrow_t  both[][];
  int       q3[][2][3];
  function void fill();
    raw = new[2]; raw[1][0] = 8'd4; raw[0][1] = 8'd9;
    d = new[2]; d[1] = new[3]; d[1][2] = 7;
    cells = new[1]; cells[0][1] = '0; cells[0][1].en = 1;
    both = new[1]; both[0] = new[2]; both[0][1] = '1;
    q3 = new[1]; q3[0][1][2] = 42;
  endfunction
  function int raw_sum(); int n = 0; foreach (raw[i, j]) n += raw[i][j]; return n; endfunction
  function int d_sum(); int n = 0; foreach (d[i, j]) n += d[i][j]; return n; endfunction
  function bit in_method_ok();
    return raw[0][1] == 9 && raw[1][0] == 4 && d[1].size() == 3 && d[1][2] == 7
        && cells[0][1].en == 1 && both[0].size() == 2 && both[0][1].stop == 1 && q3[0][1][2] == 42;
  endfunction
endclass

class k1;
  rand bit [7:0] raw[][2];
  constraint c_sz { raw.size() inside {[2:4]}; }
  constraint c_el { foreach (raw[i, j]) raw[i][j] inside {[1:5]}; }
endclass
class k2;
  rand int d[][];
  constraint c { d.size() == 3; foreach (d[i]) d[i].size() == i + 1;
                 foreach (d[i, j]) d[i][j] inside {[10:20]}; }
endclass
class k3;
  rand narrow_t cells[$][2];
  constraint c { cells.size() == 2; foreach (cells[i]) cells[i][1].en == 1; cells[1][0].data == 35'd7; }
endclass
class k4;
  rand int q3[][2][3];
  constraint c { q3.size() == 1; foreach (q3[i, j, k]) q3[i][j][k] == i * 100 + j * 10 + k; }
endclass
class k5; rand bit [3:0] raw[][2]; endclass
class k6;
  rand int d[][];
  constraint c { d.size() == 2; d[0].size() == 1; d[1].size() == 2; d[1][1] == d[0][0] + 1; }
endclass
class base; rand bit [7:0] m[][2]; constraint cb { m.size() == 2; } endclass
class der extends base; rand int x; constraint cd { x == m[1][0] + 1; m[1][0] < 100; } endclass
class pc #(type T = bit [7:0], int N = 2);
  rand T v[][N];
  constraint c { v.size() == 1; v[0][N-1] == 5; }
endclass

module tb;
  int fails = 0;
  task automatic check(bit cond, string what);
    if (!cond) begin fails++; $display("CHECK_FAIL %s", what); end
  endtask
  initial begin
    store o = new(), p;
    k1 a = new(); k2 b = new(); k3 c = new(); k4 e = new(); k5 f = new(); k6 g = new();
    der dd = new(); pc #(bit [7:0], 3) pp = new();
    bit ok; string s;

    // procedural access
    o.fill();
    check(o.in_method_ok(), "in-method element access");
    check(o.raw[0][1] == 9 && o.raw[1][0] == 4 && o.raw.size() == 2, "handle access raw");
    check(o.d[1].size() == 3 && o.d[1][2] == 7, "handle access d");
    check(o.cells[0][1].en == 1 && o.both[0][1].stop == 1 && o.q3[0][1][2] == 42, "handle access cells/both/q3");
    o.raw[1][1] = 8'd3; o.d[0] = new[1]; o.d[0][0] = 5;
    check(o.raw[1][1] == 3 && o.d[0][0] == 5, "handle writes");
    check(o.raw_sum() == 16 && o.d_sum() == 12, "foreach in a method");
    s = $sformatf("%p", o.raw); check(s == "'{'{'h0, 'h9}, '{'h4, 'h3}}", {"%p raw ", s});
    s = $sformatf("%p", o.d);   check(s == "'{'{5}, '{0, 0, 7}}", {"%p d ", s});
    p = new o;
    check(p.raw[0][1] == 9 && p.d[1][2] == 7 && p.d[1].size() == 3, "copy constructor");

    // randomization
    repeat (5) begin
      ok = a.randomize(); check(ok && a.raw.size() inside {[2:4]}, "k1 size");
      foreach (a.raw[i, j]) check(a.raw[i][j] inside {[1:5]}, "k1 element");
    end
    ok = b.randomize(); check(ok && b.d.size() == 3, "k2 size");
    foreach (b.d[i]) check(b.d[i].size() == i + 1, "k2 row size");
    foreach (b.d[i, j]) check(b.d[i][j] inside {[10:20]}, "k2 element");
    ok = b.randomize() with { d[2][0] == 15; }; check(ok && b.d[2][0] == 15, "k2 inline row element");
    ok = c.randomize();
    check(ok && c.cells.size() == 2 && c.cells[0][1].en && c.cells[1][1].en && c.cells[1][0].data == 7, "k3 queue of rows, fields");
    ok = e.randomize(); check(ok && e.q3[0][1][2] == 12 && e.q3[0][0][1] == 1, "k4 3-D rows");
    f.raw = new[3]; ok = f.randomize(); check(ok && f.raw.size() == 3, "k5 unconstrained size kept");
    ok = f.randomize() with { raw.size() == 2; raw[1][1] == 4'hc; };
    check(ok && f.raw.size() == 2 && f.raw[1][1] == 4'hc, "k5 inline");
    ok = f.randomize() with { raw.size() == 1; raw[0][0] == 3; raw[0][0] == 4; }; check(!ok, "k5 unsatisfiable");
    ok = g.randomize(); check(ok && g.d[1].size() == 2 && g.d[1][1] == g.d[0][0] + 1, "k6 row sizes");
    ok = dd.randomize(); check(ok && dd.m.size() == 2 && dd.x == dd.m[1][0] + 1, "inherited member");
    ok = pp.randomize(); check(ok && pp.v.size() == 1 && pp.v[0][2] == 5, "parameterized rows");

    if (fails == 0) $display("TEST_PASS"); else $display("TEST_FAIL count=%0d", fails);
    $finish;
  end
endmodule
