// Guard, not part of the bug: a macro defined with arguments that is NOT
// followed by a "(" is still an error, with or without the fix.
// Expected: a compile error naming the macro; the program must not run.
`define ADD(a, b) ((a) + (b))
module t;
  int r;
  initial begin
    r = `ADD;
    $display("FAIL: a bare call of a macro with arguments was accepted");
  end
endmodule
