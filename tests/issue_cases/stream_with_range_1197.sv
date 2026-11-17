//! Issue #1197: stream concatenations with a `with [ array_range ]` operand
//! (§11.4.14.4). A `with [lo:hi]` / `[lo+:w]` / `[lo-:w]` / `[i]` operand
//! selects a sub-range of a one-dimensional unpacked collection — only those
//! elements stream (pack), and on unpack only those elements are written,
//! leaving the rest of a fixed array unmodified ("the remainder of the array
//! is unmodified", §11.4.14.4).
//! Expected values were checked with the reference simulators (and the LRM).
module top;
  bit[7:0] a[4], b[4]; bit[15:0] selected;
  int lo = 1, hi = 2;
  bit[7:0] q[$];
  int fails;
  task check(string tag, logic ok);
    if (!ok) begin $display("FAIL: %s", tag); fails++; end
  endtask
  initial begin
    a = '{8'h11, 8'h22, 8'h33, 8'h44};

    // RHS (pack): only the selected elements stream.
    selected = {>>{a with [lo:hi]}};
    check("pack_lohi", selected === 16'h2233);

    selected = {>>{a with [lo+:2]}};
    check("pack_plus", selected === 16'h2233);

    selected = {>>{a with [lo-:2]}};   // indices {1,0}, streamed in array order
    check("pack_minus", selected === 16'h1122);

    selected = {>>{a with [2]}};
    check("pack_single", selected === 16'h3300);

    selected = {>>{a with [1:2]}};
    check("pack_const", selected === 16'h2233);

    selected = {<<8{a with [lo:hi]}};
    check("pack_ls8", selected === 16'h3322);

    // LHS (unpack): only the selected elements are written.
    b = '{default: 8'h11};
    {>>{b with [lo:hi]}} = 16'h2233;
    check("unpack_lohi", b[0] === 8'h11 && b[1] === 8'h22 && b[2] === 8'h33 && b[3] === 8'h11);

    b = '{default: 8'h11};
    {>>{b with [1]}} = 8'h77;
    check("unpack_single", b[0] === 8'h11 && b[1] === 8'h77 && b[2] === 8'h11 && b[3] === 8'h11);

    b = '{default: 8'h11};
    {>>{b[1:2]}} = 16'h9988;
    check("unpack_literal_slice", b[0] === 8'h11 && b[1] === 8'h99 && b[2] === 8'h88 && b[3] === 8'h11);

    // Queue operand (RHS).
    q = '{8'h11, 8'h22, 8'h33, 8'h44};
    selected = {>>{q with [lo:hi]}};
    check("pack_queue", selected === 16'h2233);

    if (fails == 0) $display("PASS");
    $finish;
  end
  initial #100 $finish;
endmodule