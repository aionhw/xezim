// IEEE 1800-2017 Ch.10 — assignments & patterns
module tb;
  int fails = 0;
  `define CK(name, cond) if (!(cond)) begin $display("FAIL[10] %s", name); fails++; end
  logic [7:0] nb;
  initial begin
    begin // 10.4.2 NBA ordering: read-after-write in same timestep
      nb = 8'h00;
      nb <= 8'hAA;
      `CK("nba not yet visible", nb == 8'h00)
      #0;
      // §4.5: the inactive region (#0) activates BEFORE the NBA region —
      // the resumed read still sees the old value (reference-simulator
      // verified; a differing commercial camp applies NBAs first).
      `CK("nba not visible in inactive region", nb == 8'h00)
      #1;
      `CK("nba visible after the time step", nb == 8'hAA)
    end
    begin // 10.9 assignment patterns
      int a[4]; int m[string]; int q[$];
      typedef struct { int x; byte y; } st_t;
      st_t s;
      a = '{1, 2, 3, 4};
      `CK("positional array pattern", a[3] == 4)
      a = '{default: 7};
      `CK("default pattern", a[0] == 7 && a[3] == 7)
      a = '{3: 9, default: 0};
      `CK("indexed pattern", a[3] == 9 && a[1] == 0)
      s = '{x: 5, y: 8'h22};
      `CK("named struct pattern", s.x == 5 && s.y == 8'h22)
      s = '{default: 0, x: 3};
      `CK("struct default+named", s.x == 3 && s.y == 0)
      q = {10, 20, 30};
      `CK("queue literal", q.size() == 3 && q[2] == 30)
    end
    begin // 10.10 unpacked array concat
      logic [7:0] a[2];
      a = {8'h01, {4'h2, 4'h3}};
      `CK("unpacked concat nested", a[0] == 8'h01 && a[1] == 8'h23)
    end
    begin // 10.6/10.7 force-release on variables
      static int fv = 1;
      force fv = 42;
      `CK("force", fv == 42)
      release fv;
      fv = 5;
      `CK("release then write", fv == 5)
    end
    $display("CH10 CHECKS DONE fails=%0d", fails);
  end
endmodule
