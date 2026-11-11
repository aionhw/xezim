`timescale 1ns/1ps
// IEEE 1800-2023: function-body reads that are NOT formals or locals of the
// called function must stay dependencies. §9.2.2.2.1 requires them for
// always_comb ("each net or variable identifier or select expression that is
// read within the block or within any function called within the block",
// except "a) Any expansion of a variable declared within the block or within
// any function called within the block"); xezim also applies them to
// continuous assignments (a superset of §10.3.2 operands). const ref is legal in a continuous assignment
// (§13.4: "a const ref function argument shall be legal in this context").
// Writes and checks are one time unit apart (§4.7, §6.8).
// §7.2.2/§7.4.2: fixed unpacked aggregates; §10.9: assignment patterns.
// §13.5.1: automatic function inputs copy their actual values.
typedef struct { int value; int tag; } item_t;
module top;
  item_t ab [2];            // shares the prefix "a" with formal a
  int a_b;                  // shares the prefix "a" with formal a
  item_t items [2];         // same name as a formal of f_nested
  item_t cfg;               // same name as a block local of f_block
  item_t src [2];
  item_t \a.b ;             // escaped name that starts like "a." (§5.6.1)
  int r_prefix, c_prefix, r_nested, c_nested, r_block, c_block;
  int r_cref, c_cref, r_esc, c_esc;
  function automatic int f_prefix(input item_t a [2]);
    return a[0].value + ab[0].value + ab[1].tag + a_b;
  endfunction
  function automatic int g_reads_items();
    return items[0].value + items[1].tag;
  endfunction
  function automatic int f_nested(input item_t items [2]);
    return items[0].tag + g_reads_items();   // g reads the MODULE items
  endfunction
  function automatic int f_block(input int k);
    int acc;
    begin : blk
      item_t cfg;                             // local; hides cfg here only
      cfg = '{k, k};
      acc = cfg.value;
    end
    return acc + cfg.tag;                     // module-scope cfg (§23.9)
  endfunction
  function automatic int f_cref(const ref item_t a [2]);
    return a[0].value + a[1].tag;
  endfunction
  function automatic int f_esc(input item_t a [2]);
    return a[1].value + \a.b .tag;
  endfunction
  item_t zero2 [2];
  assign r_prefix = f_prefix(zero2);
  assign r_nested = f_nested(zero2);
  assign r_block  = f_block(1);
  assign r_cref   = f_cref(src);
  assign r_esc    = f_esc(zero2);
  always_comb c_prefix = f_prefix(zero2);
  always_comb c_nested = f_nested(zero2);
  always_comb c_block  = f_block(1);
  always_comb c_cref   = f_cref(src);
  always_comb c_esc    = f_esc(zero2);
  task automatic check(input string n, input int got, input int exp);
    if (got === exp) $display("PASS %s got=%0d", n, got);
    else $display("FAIL %s got=%0d expected=%0d", n, got, exp);
  endtask
  initial begin
    #1 zero2 = '{'{0, 0}, '{0, 0}}; ab = '{'{0, 0}, '{0, 0}}; a_b = 0;
       items = '{'{0, 0}, '{0, 0}}; cfg = '{0, 0}; src = '{'{0, 0}, '{0, 0}};
       \a.b = '{0, 0};
    #1 ab[1].tag = 3;
    #1 check("ca_prefix_array", r_prefix, 3); check("ac_prefix_array", c_prefix, 3);
       a_b = 4;
    #1 check("ca_prefix_scalar", r_prefix, 7); check("ac_prefix_scalar", c_prefix, 7);
       items[1].tag = 5;
    #1 check("ca_nested", r_nested, 5); check("ac_nested", c_nested, 5);
       cfg.tag = 6;
    #1 check("ca_block_local", r_block, 7); check("ac_block_local", c_block, 7);
       src[1].tag = 8;
    #1 check("ca_const_ref", r_cref, 8); check("ac_const_ref", c_cref, 8);
       \a.b .tag = 9;
    #1 check("ca_escaped", r_esc, 9); check("ac_escaped", c_esc, 9);
    $display("DONE");
    $finish;
  end
endmodule
