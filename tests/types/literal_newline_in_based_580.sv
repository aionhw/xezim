// IEEE 1800-2017 5.7.1 / 5.3: the size, the base format and the value of a
// based literal are separate tokens and white space, newlines included, may
// separate the size from the base and the base from the value. Only the
// apostrophe and the base letter must touch. RTL generators wrap long
// concatenations at any token, including inside a sized literal.
// Every check below should print PASS.
module t;
  int fails = 0;

  task automatic check(string label, int got, int expected);
    if (got == expected) $display("PASS %s", label);
    else begin $display("FAIL %s: got %0d, expected %0d", label, got, expected); fails++; end
  endtask

  logic [3:0] w_size_nl;
  logic [3:0] w_value_nl;
  logic [3:0] w_both_nl;

  initial begin
    // size on one line, base and value on the next
    w_size_nl = {{2
      'd1}, 2'b01};
    check("newline between size and base", w_size_nl, 4'b0101);

    // base on one line, value on the next
    w_value_nl = {2'b11, 2'd
      1};
    check("newline between base and value", w_value_nl, 4'b1101);

    // both gaps are line breaks, with indentation and a tab
    w_both_nl = {2
      'd
	1, 2'b11};
    check("newline in both gaps", w_both_nl, 4'b0111);

    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
