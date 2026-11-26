// IEEE 1800-2023 §7.3: an unpacked untagged union is one piece of storage, so
// a `real` member written and read back keeps the real (its IEEE 754 bits are
// the storage, as the 64-bit integral member reads them); it was rounded to
// an integer or read back as an integer.
module top;
  typedef real R;
  typedef union { real r; int i; } U;
  typedef union { R r; longint l; } UL;
  U mu;
  UL mul;
  initial begin
    mu.r = 1.5;
    mul.r = -2.25;
    $display("T|module %0.2f %0.2f %h", mu.r, mul.r, mul.l);
    mul.l = 64'h4004000000000000;
    $display("T|alias %0.2f", mul.r);
  end
endmodule
