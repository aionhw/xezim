// IEEE 1800-2017 Ch.13 — tasks and functions
module tb;
  int fails = 0;
  `define CK(name, cond) if (!(cond)) begin $display("FAIL[13] %s", name); fails++; end

  function int fsum(int a, int b = 10);
    return a + b;
  endfunction
  function void fout(input int a, output int o, inout int io);
    o = a * 2;
    io += a;
  endfunction
  function automatic int frec(int n);
    return (n <= 1) ? 1 : n * frec(n - 1);
  endfunction
  function automatic void fref(ref int x);
    x = 99;
  endfunction
  task automatic tdelay(input int d, output int done_at);
    #d done_at = $time;
  endtask
  function int fname(int a, int b);
    return a - b;
  endfunction

  initial begin
    int o, io, r;
    `CK("default arg", fsum(5) == 15)
    `CK("both args", fsum(5, 1) == 6)
    `CK("named binding", fname(.b(3), .a(10)) == 7)
    io = 1;
    fout(4, o, io);
    `CK("output arg", o == 8)
    `CK("inout arg", io == 5)
    `CK("recursion", frec(5) == 120)
    r = 0; fref(r);
    `CK("ref arg", r == 99)
    begin
      int t;
      tdelay(7, t);
      `CK("task delay writes output", t == 7)
    end
    `CK("void cast", 1) // void'(fsum(1)) compile-checked below
    void'(fsum(1));
    $display("CH13 CHECKS DONE fails=%0d", fails);
  end
endmodule
