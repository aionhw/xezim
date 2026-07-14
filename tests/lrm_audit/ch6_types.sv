// IEEE 1800-2017 Ch.6 — data types (excl. 6.16 strings, audited already)
module tb;
  int fails = 0;
  `define CK(name, cond) if (!(cond)) begin $display("FAIL[6] %s", name); fails++; end
  typedef enum logic [2:0] {RED = 1, GREEN = 4, BLUE} color_e;
  initial begin
    begin // 6.11 integer types widths/signedness
      byte b8; shortint s16; longint l64; time t;
      b8 = 8'h80;  `CK("byte signed", b8 == -128)
      s16 = 16'h8000; `CK("shortint signed", s16 == -32768)
      l64 = 64'h8000_0000_0000_0000; `CK("longint min", l64 < 0)
      `CK("$bits", $bits(b8) == 8 && $bits(l64) == 64)
    end
    begin // 6.12 real conversions
      real r; int i; shortreal sr;
      r = 2.5; i = int'(r);
      `CK("real->int rounds 2.5->3 away from 0 (6.12.2)", i == 3)
      r = -2.5; i = int'(r);
      `CK("neg ties away", i == -3)
      i = $rtoi(2.9);
      `CK("$rtoi truncates", i == 2)
      r = $itor(7);
      `CK("$itor", r == 7.0)
    end
    begin // 6.19 enums
      color_e c;
      c = RED;
      `CK("enum value", c == 1)
      `CK("BLUE auto-increments", BLUE == 5)
      c = c.first();
      `CK("first", c == RED)
      c = c.next();
      `CK("next skips to GREEN", c == GREEN)
      `CK("num", c.num() == 3)
      `CK("name", c.name() == "GREEN")
      c = c.last();
      `CK("last", c == BLUE)
      c = c.next();
      `CK("next wraps", c == RED)
    end
    begin // 6.20 parameters incl. type/localparam handled at elab; $ range
      localparam int LP = 4 * 5;
      localparam string SP = "cfg";
      `CK("localparam expr", LP == 20)
      `CK("string param", SP == "cfg")
    end
    begin // 6.23 type operator / 6.22 equivalence-lite
      logic [7:0] a, b;
      `CK("type() compare", type(a) == type(b))
    end
    begin // 6.24 casting
      logic [15:0] w;
      int i;
      w = 16'hFFFF;
      i = signed'(w[7:0]);
      `CK("signed' cast", i == -1)
      i = unsigned'(8'shFF);
      `CK("unsigned' cast", i == 255)
      w = 16'(8'hAB);
      `CK("size cast up", w == 16'h00AB)
      `CK("size cast down", 4'(8'hAB) == 4'hB)
    end
    $display("CH6 CHECKS DONE fails=%0d", fails);
  end
endmodule
