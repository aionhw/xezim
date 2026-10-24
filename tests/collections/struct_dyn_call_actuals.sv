// A struct with a dynamic member returned by a function and passed directly
// as an actual, nested, mixed with variables, and with formals named like the
// caller's variables.
module top;
  typedef struct { longint l; byte unsigned d[]; } bs_t;

  function automatic bs_t lit(byte unsigned c);
    bs_t a;
    a.d = new[1];
    a.d[0] = c;
    return a;
  endfunction
  function automatic bs_t pass(bs_t a);
    return a;
  endfunction
  function automatic bs_t cat(bs_t x, bs_t y);
    bs_t c;
    c.d = {x.d, y.d};
    return c;
  endfunction
  function automatic string str(bs_t a);
    string s = "";
    foreach (a.d[i]) s = {s, string'(a.d[i])};
    return s;
  endfunction
  function automatic string pair(bs_t x, bs_t y);
    return {str(x), "|", str(y)};
  endfunction

  initial begin
    bs_t x, y;
    x = lit("a");
    y = lit("b");
    $display("T|direct '%s' '%s'", str(x), str(lit("c")));
    $display("T|through '%s'", str(pass(pass(lit("h")))));
    $display("T|nested '%s'", str(cat(lit("d"), lit("e"))));
    $display("T|mixed '%s'", str(cat(lit("f"), x)));
    $display("T|swap '%s'", pair(y, x));
    x = cat(lit("g"), x);
    $display("T|assign '%s' %0d", str(x), x.d.size());
  end
endmodule
