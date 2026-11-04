typedef struct packed { bit [7:0] sa, sb; } st_t_unused;

// ---- module-scope subroutine: left operand storage class is the only variable
function automatic bit f_formal_lhs(logic [8:0] s, bit [7:0] x, bit [7:0] y);
  return s === x + y;
endfunction

function automatic bit f_local_lhs(bit [7:0] x, bit [7:0] y);
  logic [8:0] s;
  s = 300;
  return s === x + y;
endfunction

class E;
  bit [7:0] a, b;
  logic [8:0] m9;

  function new();
    a = 200; b = 100; m9 = 300;
  endfunction

  // Left operand a formal, right operands this-relative members: red.
  function bit meth_formal_lhs(logic [8:0] s);
    return s === a + b;
  endfunction

  // Left operand a member, same right operands: green.  Control.
  function bit meth_member_lhs();
    return m9 === a + b;
  endfunction

  // Control against over-widening: an 8-bit left operand must NOT widen the
  // sum, so the wrap to 44 is the expected verdict here.
  function bit meth_narrow_lhs();
    logic [7:0] s8;
    s8 = 300;
    return s8 === a + b;
  endfunction
endclass

module top;
  logic [8:0] m9;
  bit [7:0]   p, q;
  E           e;
  int         fails;

  task chk(int r, bit g, string nm);
    if (!g) begin
      fails++;
      $display("  FAIL %-4d %s", r, nm);
    end
  endtask

  initial begin
    e = new;
    m9 = 300; p = 200; q = 100; fails = 0;

    // -- open: left operand is subroutine-local, so no context reaches the sum
    chk(1, f_formal_lhs(300, p, q),
        "formal left operand must size the sum to 9 bits");
    chk(2, f_local_lhs(p, q),
        "function-local left operand must size the sum to 9 bits");
    chk(3, e.meth_formal_lhs(300),
        "method formal left operand must size the sum to 9 bits");

    // -- controls: already correct, and guard against widening too far
    chk(4, (m9 === p + q),
        "control: module-variable left operand already sizes the sum");
    chk(5, e.meth_member_lhs(),
        "control: class-member left operand already sizes the sum");
    chk(6, e.meth_narrow_lhs(),
        "control: an 8-bit left operand must not widen the sum");

    if (fails == 0) $display("TEST_PASS");
    else            $display("TEST_FAIL count=%0d", fails);
    $finish;
  end
endmodule

