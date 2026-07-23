// Compliance test: typedef alias of a parameterised class as factory target.
//
// `my_sqr::type_id::create(...)` must instantiate the typedef's concrete type
// (`base_seq#(payload)`) when `my_sqr` is declared as a typedef alias rather
// than a top-level class.  Without the typedef-resolution fallback in
// `resolve_type_id_target_class`, the class-name lookup finds no entry for
// `my_sqr` and returns null, failing the not-null assertion below.
module top;

  class payload;
    int data;
    function new(string n = "payload", int p = 0);
      data = 0;
    endfunction
  endclass

  class base_seq #(type T = payload);
    T item;
    function new(string n = "base_seq", int p = 0);
      item = null;
    endfunction
    function string get_name();
      return "base_seq";
    endfunction
  endclass

  typedef base_seq #(payload) my_sqr;

  initial begin
    automatic my_sqr s;
    s = my_sqr::type_id::create("sqr", null);
    if (s == null) begin
      $display("TEST_FAIL: typedef type_id::create returned null");
    end else begin
      $display("TEST_PASS");
    end
  end

endmodule
