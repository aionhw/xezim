`timescale 1ns/1ps
`define SVTEST_INIT \
int failures = 0;
`define SVTEST_CHECK(expr, msg) \
if (!(expr)) begin \
  failures++; \
  $display("FAIL @%0t : %s", $time, msg); \
end
`define SVTEST_PASSFAIL \
if (failures == 0) begin \
  $display("TEST_PASS"); \
end else begin \
  $display("TEST_FAIL count=%0d", failures); \
  $fatal(1); \
end

module top;
  `SVTEST_INIT

  // ---- G1/G6/G7/G8 family ----
  class cls_base;
    int prop_val;
    function void set_via_this(int v);      this.prop_val = v;    endfunction
    virtual function void set_via_this_virt(int v); this.prop_val = v; endfunction
    function void init_bare(int v);         prop_val = v;         endfunction
    function void bump_via_this();          this.prop_val = this.prop_val + 5; endfunction
    function void incr_via_this();          this.prop_val += 1;   endfunction
    function void set_bare(int v);          prop_val = v;         endfunction
    function int  get_bare();               return prop_val;      endfunction
    function int  get_via_this();           return this.prop_val; endfunction
  endclass
  class cls_derived extends cls_base;
    localparam prop_val = 27;
    function void set_via_super(int v);     super.prop_val = v;   endfunction  // G13
  endclass
  class cls_middle extends cls_base; endclass
  class cls_derived_deep extends cls_middle;
    localparam prop_val = 27;
  endclass
  class cls_derived_param extends cls_base; parameter prop_val = 27; endclass
  class cls_derived_stat extends cls_base;  static int prop_val = 27; endclass
  class cls_box;
    static cls_base slot_b;
  endclass
  // ---- G9: rand shadow ----
  class cls_base_rand;
    rand int prop_val;
    constraint c_range { prop_val > 10; prop_val < 20; }
    function int get_bare(); return prop_val; endfunction
  endclass
  class cls_derived_rand extends cls_base_rand; localparam prop_val = 27; endclass
  class cls_derived_plain extends cls_base_rand; endclass
  // ---- G16: part-select ----
  class cls_base_bits;
    bit [7:0] vec;
    function void set_lo(int v); this.vec[3:0] = v[3:0]; endfunction
    function bit [3:0] get_lo(); return vec[3:0];        endfunction
    function bit [7:0] get_vec(); return vec;            endfunction
  endclass
  class cls_derived_bits extends cls_base_bits; localparam vec = 27; endclass
  // ---- G17..G21/G24/G26: v2 family ----
  class cls_base_v2;
    int prop_val;
    cls_base_v2 child;
    cls_base_v2 pool[$];
    function int  get_bare();          return prop_val;          endfunction
    function void set_bare(int v);     prop_val = v;             endfunction
    function void set_via_this(int v); this.prop_val = v;        endfunction
    function void post_inc();          this.prop_val++;          endfunction
    function void pre_dec();           --this.prop_val;          endfunction
    task automatic bump_ref(ref int x); x = x + 12; endtask
    task automatic call_ref();          bump_ref(this.prop_val); endtask
    function void set_child(int v);     this.child.prop_val = v; endfunction
    task automatic set_forked(int v);   fork this.prop_val = v; join endtask
    task automatic set_nb(int v);       this.prop_val <= v; endtask
  endclass
  class cls_derived_v2 extends cls_base_v2; localparam prop_val = 27; endclass
  // ---- G27: string ----
  class cls_base_str;
    string name;
    function void set_name(string s); this.name = s; endfunction
    function string get_name(); return name; endfunction
  endclass
  class cls_derived_str extends cls_base_str; localparam string name = "shadow"; endclass
  // ---- G28: object-handle property ----
  class cls_leaf; int tag; function int get_tag(); return tag; endfunction endclass
  class cls_base_leaf;
    cls_leaf leaf;
    function void set_leaf(cls_leaf l); this.leaf = l; endfunction
    function cls_leaf get_leaf(); return leaf; endfunction
  endclass
  class cls_derived_leaf extends cls_base_leaf; localparam leaf = 3; endclass

  initial begin
    begin cls_derived h_d = new; h_d.set_via_this(31);
      `SVTEST_CHECK(h_d.get_bare() == 31, "G1: this.X=v in plain method") end
    begin cls_derived h_d = new; h_d.set_via_this_virt(31);
      `SVTEST_CHECK(h_d.get_bare() == 31, "G2: this.X=v in virtual method") end
    begin cls_derived_deep h_d = new; h_d.set_via_this(31);
      `SVTEST_CHECK(h_d.get_bare() == 31, "G3: grandparent this-write") end
    begin cls_derived_param h_d = new; h_d.set_via_this(31);
      `SVTEST_CHECK(h_d.get_bare() == 31, "G4: parameter shadow") end
    begin cls_derived_stat h_d = new; h_d.set_via_this(31);
      `SVTEST_CHECK(h_d.get_bare() == 31, "G5: static-prop shadow") end
    begin cls_derived h_d = new; h_d.init_bare(19); h_d.bump_via_this();
      `SVTEST_CHECK(h_d.get_bare() == 24, "G6a: this.X=this.X+5")
      h_d.incr_via_this();
      `SVTEST_CHECK(h_d.get_bare() == 25, "G6b: this.X+=1") end
    begin cls_derived h_d = new; cls_box::slot_b = h_d;
      cls_box::slot_b.prop_val = 31;
      `SVTEST_CHECK(cls_box::slot_b.get_bare() == 31, "G7: static-handle chain") end
    begin cls_derived h_d = new; cls_base h_b = h_d; h_b.prop_val = 31;
      `SVTEST_CHECK(h_b.get_bare() == 31, "G8: base-typed handle write") end
    begin cls_derived_rand h_d = new; int v; void'(h_d.randomize()); v = h_d.get_bare();
      `SVTEST_CHECK(v > 10 && v < 20, "G9: randomize shadowed rand prop") end
    begin cls_derived_plain h_d = new; int v; void'(h_d.randomize()); v = h_d.get_bare();
      `SVTEST_CHECK(v > 10 && v < 20, "G10: randomize no shadow") end
    begin cls_derived h_d = new; h_d.set_bare(19);
      `SVTEST_CHECK(h_d.get_via_this() == 19, "G11: this-read") end
    begin cls_derived h_d = new; cls_base h_b; h_d.set_bare(19); h_b = h_d;
      `SVTEST_CHECK(h_b.prop_val == 19, "G12: handle read") end
    begin cls_derived h_d = new; h_d.set_via_super(31);
      `SVTEST_CHECK(h_d.get_bare() == 31, "G13: super.X=v") end
    begin cls_derived_bits h_d = new; h_d.set_lo(4'hA);
      `SVTEST_CHECK(h_d.get_lo() === 4'hA, "G16a: part-select write")
      `SVTEST_CHECK(h_d.get_vec() === 8'h0A, "G16b: whole read") end
    begin cls_derived_v2 h_d = new; h_d.set_bare(20); h_d.post_inc(); h_d.post_inc();
      `SVTEST_CHECK(h_d.get_bare() == 22, "G17a: X++")
      h_d.pre_dec(); h_d.pre_dec(); h_d.pre_dec();
      `SVTEST_CHECK(h_d.get_bare() == 19, "G17b: --X") end
    begin cls_derived_v2 h_d = new; h_d.call_ref();
      `SVTEST_CHECK(h_d.get_bare() == 12, "G18: ref formal") end
    begin cls_derived_v2 h_d = new; cls_base_v2 h_c = new; h_d.child = h_c; h_d.set_child(31);
      `SVTEST_CHECK(h_c.get_bare() == 31, "G19: this.child.X=v") end
    begin cls_derived_v2 h_d = new; h_d.set_forked(31);
      `SVTEST_CHECK(h_d.get_bare() == 31, "G20: forked this.X=v") end
    begin cls_base_v2 q[$]; cls_derived_v2 h_d = new; q.push_back(h_d); q[0].prop_val = 31;
      `SVTEST_CHECK(q[0].get_bare() == 31, "G21: queue-elem handle write") end
    begin cls_derived_v2 h_d = new; h_d.set_nb(31); #1;
      `SVTEST_CHECK(h_d.get_bare() == 31, "G26: non-blocking this.X<=v") end
    begin cls_derived_str h_d = new; h_d.set_name("abc");
      `SVTEST_CHECK(h_d.get_name() == "abc", "G27: string property") end
    begin cls_derived_leaf h_d = new; cls_leaf h_l = new; h_d.set_leaf(h_l);
      `SVTEST_CHECK(h_d.get_leaf() != null, "G28: object-handle property") end
    `SVTEST_PASSFAIL
  end
endmodule
