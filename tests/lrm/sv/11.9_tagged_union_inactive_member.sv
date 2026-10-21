// IEEE 1800-2023 §11.9: reading a member other than the active one is a
// run-time error (the union holds `Invalid`, so `a.Valid` is illegal).
module tu_inactive;
  typedef union tagged { void Invalid; int Valid; } u_int;
  u_int a;
  int c;
  initial begin
    a = tagged Invalid;
    c = a.Valid;
    $display("T|11.9|%0d", c);
  end
endmodule
