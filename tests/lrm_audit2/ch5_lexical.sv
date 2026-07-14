// Ch.5 lexical conventions
module tb;
  int fails = 0;
  `define CK(name, cond) if (!(cond)) begin $display("FAIL[5] %s", name); fails++; end
  initial begin
    `CK("underscore literals", 1_000_000 == 1000000)
    `CK("based underscore", 16'hDE_AD == 16'hDEAD)
    `CK("unsized unbased fill 1", (3'('1)) == 3'b111)
    `CK("unbased x fill", (4'('x)) === 4'bxxxx)
    `CK("signed based literal", 8'shFF == -1)
    `CK("size overflow trunc", 4'hFF == 4'hF)
    `CK("string escapes", "\x41" == "A")
    `CK("string octal escape", "\101" == "A")
    begin
      real r;
      r = 1.5e3; `CK("real exp", r == 1500.0)
      r = 0.5;   `CK("real frac", r == 0.5)
    end
    `CK("time literal ns", 1 == 1) // time literals exercised in ch21/timescale
    `CK("bin z digits", 4'b1z0z === 4'b1z0z)
    `CK("question is z", 4'b1?01 === 4'b1z01)
    $display("CH5 CHECKS DONE fails=%0d", fails);
  end
endmodule
