// IEEE 1800-2023 §18.12 / §7.4.5: `std::randomize` of a variable with a
// dynamic or queue outer dimension over unpacked rows (`T a[][N]`,
// `T a[][N][M]`, `T a[$][N]`, `T d[][]`), at module scope and as automatic
// task locals: the outer size, each dynamic row's size, then the elements.
typedef struct packed { logic en; logic [6:0] data; } s_t;
module tb;
  int fails = 0;
  bit [7:0] raw[][2];
  int q3[][2][3];
  s_t cq[$][2];
  int d[][];
  int f2[2][3];
  task automatic chk(bit cond, string what); if (!cond) begin fails++; $display("CHECK_FAIL %s", what); end endtask
  task automatic locals();
    bit [3:0] la[][2];
    int ld[][];
    bit ok;
    ok = std::randomize(la) with { la.size() == 3; foreach (la[i, j]) la[i][j] == i + j; };
    chk(ok && la.size() == 3 && la[2][1] == 3 && la[0][0] == 0, $sformatf("local fixed rows %p", la));
    ok = std::randomize(ld) with { ld.size() == 2; foreach (ld[i]) ld[i].size() == 2 - i; foreach (ld[i, j]) ld[i][j] inside {[1:3]}; };
    chk(ok && ld.size() == 2 && ld[0].size() == 2 && ld[1].size() == 1, $sformatf("local dyn rows %p", ld));
    foreach (ld[i, j]) chk(ld[i][j] inside {[1:3]}, "local dyn row element");
  endtask
  initial begin
    bit ok;
    ok = std::randomize(raw) with { raw.size() inside {[2:3]}; foreach (raw[i, j]) raw[i][j] < 10; };
    chk(ok && raw.size() inside {[2:3]}, "raw size");
    foreach (raw[i, j]) chk(raw[i][j] < 10, "raw elem");
    ok = std::randomize(q3) with { q3.size() == 1; foreach (q3[i, j, k]) q3[i][j][k] == j * 10 + k; };
    chk(ok && q3[0][1][2] == 12, $sformatf("q3 %p", q3));
    ok = std::randomize(cq) with { cq.size() == 2; foreach (cq[i]) cq[i][0].en == 1; cq[1][1].data == 7'd5; };
    chk(ok && cq.size() == 2 && cq[0][0].en && cq[1][0].en && cq[1][1].data == 5, $sformatf("cq %p", cq));
    ok = std::randomize(d) with { d.size() == 3; foreach (d[i]) d[i].size() == i; };
    chk(ok && d.size() == 3 && d[0].size() == 0 && d[2].size() == 2, $sformatf("d %p", d));
    ok = std::randomize(f2) with { foreach (f2[i, j]) f2[i][j] == i * 3 + j; };
    chk(ok && f2[1][2] == 5, $sformatf("fixed 2d %p", f2));
    ok = std::randomize(raw) with { raw.size() == 1; raw[0][0] == 1; raw[0][0] == 2; };
    chk(!ok, "unsat");
    locals();
    if (fails == 0) $display("TEST_PASS"); else $display("TEST_FAIL count=%0d", fails);
  end
endmodule
