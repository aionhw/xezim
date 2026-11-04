// IEEE 1800-2017 22.5.1: a macro defined with formal arguments is used as
// `NAME ( actual arguments ). Only the definition says anything about the
// position of the "(" (it must touch the name). In a usage, white space,
// newlines included (5.2, 5.3), may separate the name from the list.
// Every check below should print PASS.
`define ADD(a, b) ((a) + (b))
`define CHECK(label, value, expected) \
  if ((value) == (expected)) $display("PASS %s", label); \
  else begin \
    $display("FAIL %s: got %0d, expected %0d", label, (value), (expected)); \
    fails++; \
  end

module t;
  int fails = 0;
  int r;

  initial begin
    r = `ADD(1, 2);
    `CHECK("arguments on the same line", r, 3)

    r = `ADD
        (3, 4);
    `CHECK("name ends the line, list on the next", r, 7)

    r = `ADD
        ( 5
        , 6
        );
    `CHECK("list spread over several lines", r, 11)

    r = `ADD   
        // a comment and a blank line between the name and the list

        (7, 8);
    `CHECK("comment and blank line before the list", r, 15)

    `CHECK
      ("a statement macro split the same way", r, 15)

    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
