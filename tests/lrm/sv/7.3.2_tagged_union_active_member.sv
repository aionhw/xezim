// IEEE 1800-2023 §7.3.2: a tagged union stores one member at a time.
// Reading the active member yields the stored value.
module tu_active;
  typedef union tagged { void Invalid; int Valid; } u_int;
  u_int b;
  int c;
  initial begin
    b = tagged Valid(42);
    c = b.Valid;
    $display("T|7.3.2|%0d", c);
  end
endmodule
