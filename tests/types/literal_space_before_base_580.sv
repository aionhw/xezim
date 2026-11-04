// IEEE 1800-2017 5.7.1: a based literal is up to three tokens: a size, an
// apostrophe followed by the base letter (no white space between those two),
// and the value, "optionally preceded by white space". 5.2/5.3: white space
// between tokens is not significant. So `2 'd1` is the same literal as `2'd1`:
// 2 bits wide. A value wider than the size is truncated from the left (5.7.1),
// so `3 'd 9` is 3'b001 = 1.
// Every check below should print PASS.
module t;
  int fails = 0;

  task automatic check(string label, int got, int expected);
    if (got == expected) $display("PASS %s", label);
    else begin $display("FAIL %s: got %0d, expected %0d", label, got, expected); fails++; end
  endtask

  logic [3:0] cat_tail;       // {2'b11, <2-bit literal>}
  logic [3:0] cat_head;       // {<2-bit literal>, 2'b01}

  initial begin
    // the control: no white space (always worked)
    check("$bits(2'd1)",            $bits(2'd1),       2);

    // white space between the size and the base
    check("$bits(2 'd1)",           $bits(2 'd1),      2);
    check("$bits(12 'h3)",          $bits(12 'h3),     12);
    check("$bits(8\t'b1)",          $bits(8	'b1),     8);  // a tab
    // white space between the base and the value
    check("$bits(12'h 3)",          $bits(12'h 3),     12);
    // both
    check("$bits(12 'h 3)",         $bits(12 'h 3),    12);
    check("$bits(12 'sh 3)",        $bits(12 'sh 3),   12);

    // the value is cut to the declared width
    check("3 'd 9 is 1",            3 'd 9,            1);
    check("3'd9 is 1 (control)",    3'd9,              1);
    check("4 'hF3 is 3",            4 'hF3,            3);

    // a literal inside a concatenation: the width shifts what follows it
    check("$bits({2'b11, 2 'd1})",  $bits({2'b11, 2 'd1}), 4);
    cat_tail = {2'b11, 2 'd1};
    check("{2'b11, 2 'd1} is 4'b1101", cat_tail,       4'b1101);
    cat_head = {2 'd1, 2'b01};
    check("{2 'd1, 2'b01} is 4'b0101", cat_head,       4'b0101);

    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
