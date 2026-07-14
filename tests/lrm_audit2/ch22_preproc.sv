// Ch.22 compiler directives
`define PLAIN 42
`define ADD(a, b) ((a) + (b))
`define DEFARG(a, b = 7) ((a) * (b))
`define STRINGIFY(x) `"x`"
`define CONCAT(a, b) a``b
`define WRAP(x) `ADD(x, 1)
module tb;
  int fails = 0;
  `define CK(name, cond) if (!(cond)) begin $display("FAIL[22] %s", name); fails++; end
  initial begin
    `CK("plain define", `PLAIN == 42)
    `CK("macro args", `ADD(2, 3) == 5)
    `CK("nested macro", `WRAP(4) == 5)
    `CK("default arg", `DEFARG(3) == 21)
    `CK("default overridden", `DEFARG(3, 2) == 6)
`ifdef PLAIN
    `CK("ifdef", 1)
`else
    `CK("ifdef", 0)
`endif
`ifndef NOT_DEFINED
    `CK("ifndef", 1)
`endif
`ifdef NOT_DEFINED
    `CK("ifdef-else", 0)
`elsif PLAIN
    `CK("elsif", 1)
`else
    `CK("ifdef-else", 0)
`endif
    begin
      string s;
      s = `STRINGIFY(hello);
      `CK("stringify", s == "hello")
      begin
        int `CONCAT(foo, bar);
        foobar = 9;
        `CK("token paste", foobar == 9)
      end
    end
    `undef PLAIN
`ifdef PLAIN
    `CK("undef", 0)
`else
    `CK("undef", 1)
`endif
    `CK("__LINE__ sane", `__LINE__ > 30)
    $display("CH22 CHECKS DONE fails=%0d", fails);
  end
endmodule
