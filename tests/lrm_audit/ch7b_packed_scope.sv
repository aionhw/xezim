// differential: packed struct/union/2d at MODULE scope vs block-local
module tb;
  int fails = 0;
  `define CK(name, cond) if (!(cond)) begin $display("FAIL[7b] %s", name); fails++; end
  typedef struct packed { logic [3:0] hi; logic [3:0] lo; } p_t;
  p_t p_mod;
  logic [1:0][3:0] m_mod;
  initial begin
    p_mod = 8'hA5;
    `CK("module-scope packed struct", p_mod.hi == 4'hA && p_mod.lo == 4'h5)
    m_mod = 8'hC3;
    `CK("module-scope packed 2d", m_mod[1] == 4'hC && m_mod[0] == 4'h3)
    $display("CH7B CHECKS DONE fails=%0d", fails);
  end
endmodule
