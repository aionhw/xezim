// IEEE 1800-2017 Ch.11 — operators & expressions
module tb;
  int fails = 0;
  `define CK(name, cond) if (!(cond)) begin $display("FAIL[11] %s", name); fails++; end
  initial begin
    begin // 11.4.3 arithmetic, signed/unsigned semantics
      int a; logic [7:0] u8; logic signed [7:0] s8; int unsigned ui;
      a = -7; `CK("mod sign follows dividend", (a % 3) == -1)
      a = 7;  `CK("mod pos", (a % -3) == 1)
      u8 = 8'hFF; `CK("unsigned compare", u8 > 8'h01)
      s8 = -1;   `CK("signed compare", s8 < 8'sh01)
      `CK("mixed sign -> unsigned (11.8.1)", (s8 > 8'h01))  // s8 becomes 255
      ui = -1; `CK("unsigned wrap", ui == 32'hFFFF_FFFF)
      `CK("power op", 2**10 == 1024)
      `CK("power signed", (-2)**3 == -8)
    end
    begin // 11.4.5/6 equality with x/z
      logic [3:0] v;
      v = 4'b10x1;
      `CK("== with x is x", ((v == 4'b1001) === 1'bx))
      `CK("=== exact", (v === 4'b10x1))
      `CK("!== exact", (v !== 4'b1001))
      `CK("==? wildcard", ((4'b1001 ==? 4'b10?1) === 1'b1))
      `CK("!=? wildcard", ((4'b1011 !=? 4'b10?1) === 1'b0))
    end
    begin // 11.4.8 shifts
      logic signed [7:0] s8;
      s8 = -8;
      `CK("arith shift right", (s8 >>> 1) == -4)
      `CK("logic shift right", ((8'hF0 >> 4) == 8'h0F))
      `CK("shift by width+ gives 0", ((8'hFF << 9) == 0))
    end
    begin // 11.4.11 conditional with x select
      logic sel; logic [3:0] r;
      sel = 1'bx;
      r = sel ? 4'b1100 : 4'b1010;
      `CK("x-select merges (11.4.11)", (r === 4'b1xx0))
    end
    begin // 11.4.12 concat/replication
      logic [11:0] c;
      string s;
      c = {4'hA, 4'hB, 4'hC};
      `CK("concat", c == 12'hABC)
      c = {3{4'h5}};
      `CK("replication", c == 12'h555)
      s = {"a", "bc"};
      `CK("string concat", s == "abc")
    end
    begin // 11.4.13 inside
      int x;
      x = 5;
      `CK("inside list", x inside {1, 5, 9})
      `CK("inside range", x inside {[3:7]})
      `CK("not inside", !(10 inside {[3:7]}))
    end
    begin // 11.4.14 streaming
      logic [7:0] a; logic [15:0] w; byte q[$];
      a = 8'b1101_0010;
      `CK("stream reverse bits", ({<<{a}} == 8'b0100_1011))
      `CK("stream bytes", ({<<8{16'hABCD}} == 16'hCDAB))
      w = {>>{8'h12, 8'h34}};
      `CK("stream pack", w == 16'h1234)
    end
    begin // 11.4.10 reduction
      logic [3:0] v;
      v = 4'b1011;
      `CK("red and", (&v) == 1'b0)
      `CK("red or", (|v) == 1'b1)
      `CK("red xor", (^v) == 1'b1)
      `CK("red xnor", (~^v) == 1'b0)
    end
    begin // 11.3.6 assignment operators in expr, inc/dec
      int i;
      i = 5;
      `CK("preinc", (++i) == 6)
      `CK("postinc value", (i++) == 6)
      `CK("after postinc", i == 7)
      i += 3; `CK("plus-assign", i == 10)
      i <<= 2; `CK("shl-assign", i == 40)
    end
    begin // 11.9 tagged / 11.10 string compare ops
      string s1, s2;
      s1 = "abc"; s2 = "abd";
      `CK("string lt", s1 < s2)
      `CK("string ge", s2 >= s1)
    end
    $display("CH11 CHECKS DONE fails=%0d", fails);
  end
endmodule
