// Regression: value parameters of a specialized class, referenced bare inside
// a STATIC method (no instance). Direct `Class#(args)::method()` form.
class Wrapper #(type T = int, string Tname = "<unknown>", int V = 0);
  static function string type_name();
    return Tname;
  endfunction
  static function int value();
    return V;
  endfunction
  virtual function string get_type_name();
    return type_name();
  endfunction
endclass

class base_class;
endclass

module top;
  initial begin
    // String param, multi-arg specialization.
    string s;
    int v;
    s = Wrapper#(base_class, "base_class", 7)::type_name();
    v = Wrapper#(base_class, "base_class", 7)::value();
    if (s == "base_class") $display("STR_PASS '%s'", s);
    else                   $display("STR_FAIL got='%s'", s);
    if (v == 7)            $display("INT_PASS %0d", v);
    else                   $display("INT_FAIL got=%0d", v);

    // Negative-space: default values when NOT specialized.
    // LRM §8.25: unadorned parameterized class scope resolution is illegal;
    // must use explicit empty specialization `Wrapper#()`.
    s = Wrapper#()::type_name();
    if (s == "<unknown>")  $display("DEF_PASS '%s'", s);
    else                   $display("DEF_FAIL got='%s'", s);
  end
endmodule
