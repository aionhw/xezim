// IEEE 1800-2017 Ch.12 — procedural programming statements
module tb;
  int fails = 0;
  `define CK(name, cond) if (!(cond)) begin $display("FAIL[12] %s", name); fails++; end
  initial begin
    begin // 12.5 case variants
      logic [3:0] v; int r;
      v = 4'b1z10;
      casez (v) 4'b1?10: r = 1; default: r = 0; endcase
      `CK("casez ? matches z", r == 1)
      v = 4'b1x10;
      casex (v) 4'b1010: r = 2; default: r = 0; endcase
      `CK("casex x matches", r == 2)
      r = 0;
      case (2) inside
        [0:1]: r = 1;
        [2:3]: r = 2;
        default: r = 9;
      endcase
      `CK("case inside range", r == 2)
    end
    begin // 12.7 loops
      int n, i;
      n = 0; for (i = 0; i < 5; i++) begin if (i == 2) continue; if (i == 4) break; n++; end
      `CK("for continue/break", n == 3)
      n = 0; i = 0; do begin n++; i++; end while (i < 3);
      `CK("do-while", n == 3)
      n = 0; repeat (4) n++;
      `CK("repeat", n == 4)
      n = 0; i = 0; while (i < 6) begin i += 2; n++; end
      `CK("while", n == 3)
      begin
        int total;
        total = 0;
        for (int j = 0, k = 10; j < k; j += 3) total++;
        `CK("for multi-init", total == 4)
      end
    end
    begin // 12.4 unique/priority (runtime no-violation path)
      int r; logic [1:0] s;
      s = 2'b10;
      unique case (s) 2'b00: r=0; 2'b01: r=1; 2'b10: r=2; 2'b11: r=3; endcase
      `CK("unique case", r == 2)
      priority if (s[1]) r = 7; else r = 8;
      `CK("priority if", r == 7)
    end
    $display("CH12 CHECKS DONE fails=%0d", fails);
  end
endmodule
