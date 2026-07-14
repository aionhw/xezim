module tb;
  int fails = 0;
  `define CK(name, cond) if (!(cond)) begin $display("FAIL[7c] %s", name); fails++; end
  typedef struct packed { logic [3:0] hi; logic [3:0] lo; } p_t;
  initial begin
    begin // module typedef, block-local VAR
      p_t p;
      p = 8'hA5;
      `CK("local var of module typedef", p.hi == 4'hA && p.lo == 4'h5)
    end
    begin // block-local typedef + var
      typedef struct packed { logic [3:0] hi; logic [3:0] lo; } q_t;
      q_t q;
      q = 8'hA5;
      `CK("local typedef+var", q.hi == 4'hA)
    end
    begin // block-local packed 2d var
      logic [1:0][3:0] m;
      m = 8'hC3;
      `CK("local packed 2d", m[1] == 4'hC)
    end
    $display("CH7C CHECKS DONE fails=%0d", fails);
  end
endmodule
