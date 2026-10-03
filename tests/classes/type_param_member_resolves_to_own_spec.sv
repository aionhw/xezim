// Self-test for type-parameter MEMBER resolution under concurrency.
//
// A method-driven member whose declared type is a class type parameter
// (e.g. `REQ req;` in `uvm_sequence #(trans)`) must resolve against the
// owning INSTANCE's own concrete `extends #(...)` specialization, NOT
// against the ambient `current_spec`. `current_spec` is a single mutable
// field that concurrent forks leave set to whichever method ran last; here
// a concurrent parameterized component's `drive()`. If the ambient spec
// outranks the instance's own class chain, a `sub_a extends seq_param
// #(item_a)` member `req` wrongly resolves to `item_b` (the leaked spec),
// and a static accessor on it reports the wrong name — the same failure
// as a top sequence emitting a bot item (UVM `SQRSNDREQCAST`).
class item_a;
  static function string type_name(); return "itema"; endfunction
endclass
class item_b;
  static function string type_name(); return "itemb"; endfunction
endclass

// Parameterized COMPONENT whose blocking `drive()` seeds the ambient
// `current_spec` to `component #(item_b)` and blocks, leaking it to any
// fork peer that reads a type-parameterized member meanwhile.
class component #(type T = item_b);
  T field;
  task drive();
    string s;
    s = field.type_name();
    #1;
  endtask
endclass

// Parameterized ancestor declares the type-parameterized member.
class seq_param #(type T = item_a);
  T req;
endclass

// NON-parameterized leaf (like a user top_sequence). Its `body` is in a
// non-parameterized scope, so it does not re-seed `current_spec` to its own
// spec; it must therefore resolve `req` from the instance's own chain.
class sub_a extends seq_param #(item_a);
  task body();
    string n;
    n = req.type_name();
    $display("N=[%s]", n);
    if (n == "itema")
      $display("TAG_PASS");
    else
      $display("TAG_FAIL n=%s", n);
  endtask
endclass

module top;
  component #(item_b) cmp;
  sub_a sa;
  item_b b;
  initial begin
    cmp = new; sa = new; b = new;
    cmp.field = b;
    fork
      begin repeat(50) cmp.drive(); end
      begin sa.body(); end
    join
  end
endmodule