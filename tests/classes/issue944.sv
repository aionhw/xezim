// =============================================================================
// comprehensive_shadow_write_divert.sv
//
// Defect family under test:
//   A base class B owns an instance property X.  A derived class D shadows X
//   with a *non-instance* member (localparam / parameter / static property).
//   When code in B executes `this.X = v` (or a write reaches the object
//   through any handle whose dynamic type is D), xezim resolves the write
//   target from the object's DYNAMIC class D, finds the shadowing member, and
//   diverts the write into the per-class static slot -- the instance property
//   is never written and later reads return 0 / the pre-write value.
//   Per LRM 8.14/8.23 the `this.X` write inside B must resolve in B's LEXICAL
//   scope and always hit B's instance property, regardless of what D declares.
//
// =============================================================================

`timescale 1ns/1ps

`ifndef SVTEST_DEFS_SVH
`define SVTEST_DEFS_SVH
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
`endif

package pkg_shadow;

  // ---------- G0 family: primary MWE (constructor write) --------------------
  class cls_base_ctor;
    int prop_val;
    function new(int v);
      this.prop_val = v;          // <-- the diverted write
    endfunction
  endclass

  class cls_derived_ctor extends cls_base_ctor;
    localparam prop_val = 27;     // <-- the shadow
    function new();
      super.new(prop_val);        // arg resolves lexically to the localparam
    endfunction
  endclass

  virtual class cls_outer_base;
    cls_base_ctor inner;
  endclass

  class cls_outer extends cls_outer_base;
    function new();
      cls_derived_ctor h_tmp;
      h_tmp  = new;
      this.inner = h_tmp;
    endfunction
    function int get_inner();
      return this.inner.prop_val; // base-typed property read
    endfunction
  endclass

  // ---------- shared family: G1/G2/G6/G8/G11/G12/G13 ------------------------
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
    function void set_via_super(int v);     super.prop_val = v;   endfunction
  endclass

  // ---------- G3 family: three-level hierarchy -------------------------------
  class cls_middle extends cls_base;
  endclass

  class cls_derived_deep extends cls_middle;
    localparam prop_val = 27;     // grandchild shadow, writer in grandparent
  endclass

  // ---------- G4 family: `parameter` shadow ----------------------------------
  class cls_derived_param extends cls_base;
    parameter prop_val = 27;
  endclass

  // ---------- G5 family: genuine `static` property shadow --------------------
  class cls_derived_stat extends cls_base;
    static int prop_val = 27;
  endclass

  // ---------- G7 family: static-handle chain ---------------------------------
  class cls_box;
    static cls_base slot_b;       // declared base type, holds derived object
  endclass

  // ---------- G9/G10 family: rand + constraint -------------------------------
  class cls_base_rand;
    rand int prop_val;
    constraint c_range { prop_val > 10; prop_val < 20; }
    function int get_bare();      return prop_val;      endfunction
  endclass

  class cls_derived_rand extends cls_base_rand;
    localparam prop_val = 27;
  endclass

  class cls_derived_plain extends cls_base_rand;
    // no shadow: randomize() baseline
  endclass

  // ---------- G14 family: collection member via this -------------------------
  class cls_base_queue;
    int queue_val[$];
    function void push_via_this(int v); this.queue_val.push_back(v); endfunction
    function int  get_size();           return queue_val.size();     endfunction
  endclass

  class cls_derived_queue extends cls_base_queue;
    localparam queue_val = 3;
  endclass

  // ---------- G15 family: typedef-only shadow --------------------------------
  class cls_derived_typedef extends cls_base;
    typedef int prop_val;         // type-only shadow: must NOT divert writes
  endclass

  // ==========================================================================
  // v2 families (G16..G43).  Same defect shape in every family -- base owns
  // instance property X, derived shadows X with a non-instance member -- only
  // the WRITE ARM differs.  G31..G43 are the controls that bound the defect.
  // ==========================================================================

  // ---------- G17..G26 / G29 / G30 shared writer/reader base -----------------
  class cls_base_v2;
    int         prop_val;
    cls_base_v2 child;                                        // G19 / G40
    cls_base_v2 pool[$];                                      // G25
    function int  get_bare();          return prop_val;                    endfunction
    function void set_bare(int v);     prop_val = v;                        endfunction
    function void set_via_this(int v); this.prop_val = v;                   endfunction
    function void post_inc();          this.prop_val++;                     endfunction
    function void pre_dec();           --this.prop_val;                     endfunction
    task   automatic bump_ref(ref int x);            x = x + 12;            endtask
    task   automatic call_ref();                     bump_ref(this.prop_val); endtask
    function void set_child(int v);    this.child.prop_val = v;             endfunction
    task   automatic set_forked(int v); fork this.prop_val = v; join        endtask
    function void add(cls_base_v2 b);  pool.push_back(b);                   endfunction
    function void set_pool(int i, int v); this.pool[i].prop_val = v;        endfunction
    task   automatic set_nb(int v);    this.prop_val <= v;                  endtask
  endclass

  class cls_derived_v2 extends cls_base_v2;
    localparam prop_val = 27;
  endclass

  // ---------- G16: packed property + part-select write -----------------------
  class cls_base_bits;
    bit [7:0] vec;
    function void set_lo(int v);    this.vec[3:0] = v[3:0]; endfunction
    function bit [3:0] get_lo();    return vec[3:0];        endfunction
    function bit [7:0] get_vec();   return vec;             endfunction
  endclass

  class cls_derived_bits extends cls_base_bits;
    localparam vec = 27;
  endclass

  // ---------- G27: string instance property ---------------------------------
  class cls_base_str;
    string name;
    function void set_name(string s); this.name = s; endfunction
    function string get_name();       return name;   endfunction
  endclass

  class cls_derived_str extends cls_base_str;
    localparam string name = "shadow";
  endclass

  // ---------- G28: object-handle instance property --------------------------
  class cls_leaf;
    int tag;
    function int get_tag(); return tag; endfunction
  endclass

  class cls_base_leaf;
    cls_leaf leaf;
    function void set_leaf(cls_leaf l); this.leaf = l; endfunction
    function cls_leaf get_leaf();       return leaf;   endfunction
  endclass

  class cls_derived_leaf extends cls_base_leaf;
    localparam leaf = 3;
  endclass

  // ---------- G29: static-handle chain via a PARAMETERIZED holder -----------
  class cls_box_p #(int W = 8);
    static cls_base_v2 slot;
  endclass

  // ---------- G30: the shadowing derived class is itself parameterized ------
  class cls_derived_pv #(int W = 8) extends cls_base_v2;
    localparam prop_val = 27;
  endclass

  // ---------- G31 CONTROL: unpacked array + constant-array shadow -----------
  class cls_base_arr;
    int arr[4];
    function void set_arr(int i, int v); this.arr[i] = v; endfunction
    function int  get_arr(int i);        return arr[i];   endfunction
  endclass

  class cls_derived_arr extends cls_base_arr;
    localparam int arr[4] = '{1, 2, 3, 4};
  endclass

  // ---------- G34 CONTROL: shadow is a class-header value parameter ---------
  class cls_derived_hdr #(int prop_val = 27) extends cls_base_v2;
  endclass

  // ---------- G35 CONTROL: shadow lives in an unrelated class ---------------
  class cls_unrelated;
    localparam prop_val = 27;
  endclass

  class cls_derived_plain2 extends cls_base_v2;
  endclass

  // ---------- G36 CONTROL: shadow lives in a sibling class ------------------
  class cls_sibling extends cls_base_v2;
    localparam prop_val = 27;
  endclass

  class cls_derived_plain3 extends cls_base_v2;
  endclass

  // ---------- G37 CONTROL: derived shadows with a real instance property ----
  class cls_derived_inst extends cls_base_v2;
    int prop_val;
    function void set_inst(int v); this.prop_val = v; endfunction
    function int  get_inst();      return prop_val;   endfunction
  endclass

  // ---------- G39 CONTROL: shadow is an INTERMEDIATE segment ----------------
  class cls_cfg;
    int mode;
    function int get_mode(); return mode; endfunction
  endclass

  class cls_base_cfg;
    cls_cfg cfg;
    function new();                cfg = new;             endfunction
    function void set_mode(int m); this.cfg.mode = m;     endfunction
    function int  get_mode();      return cfg.get_mode(); endfunction
  endclass

  class cls_derived_cfg extends cls_base_cfg;
    localparam cfg = 3;
  endclass

  // ---------- G41 CONTROL: static event whose name is shadowed --------------
  class cls_base_ev;
    static event ev;
    int hits;
    task automatic waiter(); @ev;   hits++; endtask
    task automatic fire();   -> ev;         endtask
  endclass

  class cls_derived_ev extends cls_base_ev;
    localparam ev = 3;
  endclass

  // ---------- G42 CONTROL: static method writes through a handle argument ---
  class cls_base_sm;
    int prop_val;
    function int get_bare(); return prop_val; endfunction
    static function void set_via_handle(cls_base_sm b, int v); b.prop_val = v; endfunction
  endclass

  class cls_derived_sm extends cls_base_sm;
    localparam prop_val = 27;
  endclass

  // ---------- G43 CONTROL: bare static-slot member write inside a method ----
  class cls_base_slot;
    static cls_base_slot slot;
    int prop_val;
    function int  get_bare();        return prop_val;   endfunction
    function void set_slot(int v);   slot.prop_val = v; endfunction
  endclass

  class cls_derived_slot extends cls_base_slot;
    localparam prop_val = 27;
  endclass


endpackage

module tb_top;
  import pkg_shadow::*;
  `SVTEST_INIT

  // ---- helpers for the v2 groups -------------------------------------------
  task automatic bump_mod(ref int x);      x = x + 7;  endtask   // G32
  task automatic wr_output(output int x);  x = 31;     endtask   // G33
  cls_derived_v2 g_made;                                         // G23
  function cls_base_v2 mk_base(); return g_made; endfunction     // G23
  typedef cls_box_p #(8) cls_box8;                               // G29

  initial begin
    // ---- G0: primary -- ctor this-write, localparam shadow, wrapper read ----
    begin
      cls_outer h_o = new;
      `SVTEST_CHECK(h_o.get_inner() == 27,
        "G0: ctor `this.X=v` in base diverted by derived localparam (got 0, exp 27)")
    end

    // ---- G1: this-write in a plain method ------------------------------------
    begin
      cls_derived h_d = new;
      h_d.set_via_this(31);
      `SVTEST_CHECK(h_d.get_bare() == 31,
        "G1: `this.X=v` in plain method diverted (got 0, exp 31)")
    end

    // ---- G2: this-write in a virtual method ----------------------------------
    begin
      cls_derived h_d = new;
      h_d.set_via_this_virt(31);
      `SVTEST_CHECK(h_d.get_bare() == 31,
        "G2: `this.X=v` in virtual method diverted (got 0, exp 31)")
    end

    // ---- G3: localparam in grandchild, writer in grandparent -----------------
    begin
      cls_derived_deep h_d = new;
      h_d.set_via_this(31);
      `SVTEST_CHECK(h_d.get_bare() == 31,
        "G3: grandparent this-write diverted by grandchild localparam (got 0, exp 31)")
    end

    // ---- G4: shadowing member is `parameter` ---------------------------------
    begin
      cls_derived_param h_d = new;
      h_d.set_via_this(31);
      `SVTEST_CHECK(h_d.get_bare() == 31,
        "G4: `parameter` shadow diverts this-write (got 0, exp 31)")
    end

    // ---- G5: shadowing member is a real `static` property --------------------
    begin
      cls_derived_stat h_d = new;
      h_d.set_via_this(31);
      `SVTEST_CHECK(h_d.get_bare() == 31,
        "G5: static-property shadow diverts this-write (got 0, exp 31)")
    end

    // ---- G6: compound this-writes --------------------------------------------
    begin
      cls_derived h_d = new;
      h_d.init_bare(19);
      h_d.bump_via_this();          // this.X = this.X + 5
      `SVTEST_CHECK(h_d.get_bare() == 24,
        "G6a: `this.X = this.X + 5` write half diverted (got 19, exp 24)")
      h_d.incr_via_this();          // this.X += 1
      `SVTEST_CHECK(h_d.get_bare() == 25,
        "G6b: `this.X += 1` write half diverted (got 19, exp 25)")
    end

    // ---- G7: static-handle chain `Box::slot.X = v` ---------------------------
    begin
      cls_derived h_d = new;
      cls_box::slot_b = h_d;
      cls_box::slot_b.prop_val = 31;
      `SVTEST_CHECK(cls_box::slot_b.get_bare() == 31,
        "G7: static-handle chain write diverted by dynamic class (got 0, exp 31)")
    end

    // ---- G8: write via base-typed handle `b.X = v` ---------------------------
    // (declarations are package-qualified, matching the standalone probe; with
    //  bare imported names xezim resolves the declared type and skips the
    //  divert -- see the issue report, "Interaction note")
    begin
      pkg_shadow::cls_derived h_d = new;
      pkg_shadow::cls_base h_b = h_d;
      h_b.prop_val = 31;
      `SVTEST_CHECK(h_b.get_bare() == 31,
        "G8: base-typed handle write diverted (got 0, exp 31)")
    end

    // ---- G9: randomize() with shadowed rand property -------------------------
    begin
      cls_derived_rand h_d = new;
      int v;
      void'(h_d.randomize());
      v = h_d.get_bare();
      `SVTEST_CHECK(v > 10 && v < 20,
        "G9: randomize() of shadowed rand prop diverted (got 0, exp 11..19)")
    end

    // ---- G10 CONTROL: randomize() with NO shadow -----------------------------
    begin
      cls_derived_plain h_d = new;
      int v;
      void'(h_d.randomize());
      v = h_d.get_bare();
      `SVTEST_CHECK(v > 10 && v < 20,
        "G10 CONTROL: randomize() without shadow must satisfy constraint")
    end

    // ---- G11 CONTROL: this-read and bare read after bare write ---------------
    begin
      cls_derived h_d = new;
      h_d.set_bare(19);
      `SVTEST_CHECK(h_d.get_via_this() == 19,
        "G11a CONTROL: `this.X` read must hit the instance property (19)")
      `SVTEST_CHECK(h_d.get_bare() == 19,
        "G11b CONTROL: bare `X` read must hit the instance property (19)")
    end

    // ---- G12 CONTROL: read via base-typed handle -----------------------------
    begin
      cls_derived h_d = new;
      cls_base h_b;
      h_d.set_bare(19);
      h_b = h_d;
      `SVTEST_CHECK(h_b.prop_val == 19,
        "G12 CONTROL: handle read must hit the instance property (19)")
    end

    // ---- G13 CONTROL: `super.X = v` from the derived class -------------------
    begin
      cls_derived h_d = new;
      h_d.set_via_super(31);
      `SVTEST_CHECK(h_d.get_bare() == 31,
        "G13 CONTROL: `super.X = v` must hit the base instance property (31)")
    end

    // ---- G14 CONTROL: collection method via this -----------------------------
    begin
      cls_derived_queue h_d = new;
      h_d.push_via_this(5);
      `SVTEST_CHECK(h_d.get_size() == 1,
        "G14 CONTROL: `this.q.push_back(v)` must hit the instance queue (1)")
    end

    // ---- G15 CONTROL: typedef-only shadow ------------------------------------
    begin
      cls_derived_typedef h_d = new;
      h_d.set_via_this(31);
      `SVTEST_CHECK(h_d.get_bare() == 31,
        "G15 CONTROL: typedef-only shadow must NOT divert the write (31)")
    end

    // ---- G16: packed part-select write `this.vec[3:0] = v` -------------------
    begin
      cls_derived_bits h_d = new;
      h_d.set_lo(4'hA);
      `SVTEST_CHECK(h_d.get_lo() === 4'hA,
        "G16a: packed part-select write diverted (got 0, exp a)")
      `SVTEST_CHECK(h_d.get_vec() === 8'h0A,
        "G16b: whole-vector read after part-select write (got 00, exp 0a)")
    end

    // ---- G17: ++/-- on `this.X` ----------------------------------------------
    begin
      cls_derived_v2 h_d = new;
      h_d.set_bare(20);
      h_d.post_inc();
      h_d.post_inc();
      `SVTEST_CHECK(h_d.get_bare() == 22,
        "G17a: `this.X++` write half diverted (got 20, exp 22)")
      h_d.pre_dec();
      h_d.pre_dec();
      h_d.pre_dec();
      `SVTEST_CHECK(h_d.get_bare() == 19,
        "G17b: `--this.X` write half diverted (got 20, exp 19)")
    end

    // ---- G18: `this.X` passed as a `ref` actual --------------------------------
    begin
      cls_derived_v2 h_d = new;
      h_d.call_ref();
      `SVTEST_CHECK(h_d.get_bare() == 12,
        "G18: write through `ref` formal of `this.X` diverted (got 0, exp 12)")
    end

    // ---- G19: `this.child.X = v` inside a base method --------------------------
    begin
      cls_derived_v2 h_d = new;
      cls_derived_v2 h_c = new;
      h_d.child = h_c;
      h_d.set_child(31);
      `SVTEST_CHECK(h_c.get_bare() == 31,
        "G19: `this.child.X = v` in base method diverted (got 0, exp 31)")
    end

    // ---- G20: `this.X = v` inside a forked process -----------------------------
    begin
      cls_derived_v2 h_d = new;
      h_d.set_forked(31);
      `SVTEST_CHECK(h_d.get_bare() == 31,
        "G20: forked `this.X = v` diverted (got 0, exp 31)")
    end

    // ---- G21: write through a queue-element handle -----------------------------
    begin
      cls_base_v2 q[$];
      cls_derived_v2 h_d = new;
      q.push_back(h_d);
      q[0].prop_val = 31;
      `SVTEST_CHECK(q[0].get_bare() == 31,
        "G21: queue-element handle write diverted (got 0, exp 31)")
    end

    // ---- G22: write through an associative-array element handle ----------------
    begin
      cls_base_v2 aa[string];
      cls_derived_v2 h_d = new;
      aa["k"] = h_d;
      aa["k"].prop_val = 31;
      `SVTEST_CHECK(aa["k"].get_bare() == 31,
        "G22: assoc-element handle write diverted (got 0, exp 31)")
    end

    // ---- G23: write through a function-result handle ---------------------------
    begin
      g_made = new;
      mk_base().prop_val = 31;
      `SVTEST_CHECK(g_made.get_bare() == 31,
        "G23: function-result handle write diverted (got 0, exp 31)")
    end

    // ---- G24: write through foreach over a queue of base-typed handles ---------
    begin
      cls_base_v2 q[$];
      cls_derived_v2 h_d1 = new;
      cls_derived_v2 h_d2 = new;
      q.push_back(h_d1);
      q.push_back(h_d2);
      foreach (q[i]) q[i].prop_val = 31;
      `SVTEST_CHECK(h_d1.get_bare() == 31 && h_d2.get_bare() == 31,
        "G24: foreach-indexed write diverted (got 0/0, exp 31/31)")
    end

    // ---- G25: element of a member queue, written inside a base method ---------
    begin
      cls_derived_v2 h_owner = new;
      cls_derived_v2 h_e1 = new;
      h_owner.add(h_e1);
      h_owner.set_pool(0, 31);
      `SVTEST_CHECK(h_e1.get_bare() == 31,
        "G25: member-pool element write diverted (got 0, exp 31)")
    end

    // ---- G26: non-blocking assignment to `this.X` -----------------------------
    begin
      cls_derived_v2 h_d = new;
      h_d.set_nb(31);
      #1;
      `SVTEST_CHECK(h_d.get_bare() == 31,
        "G26: non-blocking `this.X <= v` diverted (got 0, exp 31)")
    end

    // ---- G27: string instance property ---------------------------------------
    begin
      cls_derived_str h_d = new;
      h_d.set_name("abc");
      `SVTEST_CHECK(h_d.get_name() == "abc",
        "G27: string property write diverted (got empty, exp abc)")
    end

    // ---- G28: object-handle instance property --------------------------------
    begin
      cls_derived_leaf h_d = new;
      cls_leaf h_l = new;
      h_d.set_leaf(h_l);
      `SVTEST_CHECK(h_d.get_leaf() != null,
        "G28: object-handle property write lost (got null)")
    end

    // ---- G29: static-handle chain through a parameterized holder --------------
    begin
      cls_derived_v2 h_d = new;
      cls_box8::slot = h_d;
      cls_box8::slot.set_via_this(31);
      `SVTEST_CHECK(h_d.get_bare() == 31,
        "G29: parameterized static-handle chain write diverted (got 0, exp 31)")
    end

    // ---- G30: the shadowing derived class is itself parameterized -------------
    begin
      cls_derived_pv #(8) h_d = new;
      h_d.set_via_this(31);
      `SVTEST_CHECK(h_d.get_bare() == 31,
        "G30: parameterized derived shadow diverts this-write (got 0, exp 31)")
    end

    // ---- G31 CONTROL: unpacked-array element write, constant-array shadow ----
    begin
      cls_derived_arr h_d = new;
      h_d.set_arr(2, 77);
      `SVTEST_CHECK(h_d.get_arr(2) == 77,
        "G31 CONTROL: unpacked-array element write must hit the instance array")
    end

    // ---- G32 CONTROL: base-typed handle as a `ref` actual ---------------------
    begin
      cls_derived_v2 h_d = new;
      cls_base_v2 h_b = h_d;
      bump_mod(h_b.prop_val);
      `SVTEST_CHECK(h_d.get_bare() == 7,
        "G32 CONTROL: ref actual via base-typed handle must reach the instance")
    end

    // ---- G33 CONTROL: base-typed handle as an `output` actual -----------------
    begin
      cls_derived_v2 h_d = new;
      cls_base_v2 h_b = h_d;
      wr_output(h_b.prop_val);
      `SVTEST_CHECK(h_d.get_bare() == 31,
        "G33 CONTROL: output actual via base-typed handle must reach the instance")
    end

    // ---- G34 CONTROL: shadow is a class-header value parameter ----------------
    begin
      cls_derived_hdr h_d = new;
      h_d.set_via_this(31);
      `SVTEST_CHECK(h_d.get_bare() == 31,
        "G34 CONTROL: header-parameter shadow must NOT divert the write (31)")
    end

    // ---- G35 CONTROL: shadow lives in an unrelated class ----------------------
    begin
      cls_unrelated h_u = new;
      cls_derived_plain2 h_d = new;
      h_d.set_via_this(31);
      `SVTEST_CHECK(h_d.get_bare() == 31,
        "G35 CONTROL: unrelated same-named localparam must NOT divert (31)")
    end

    // ---- G36 CONTROL: shadow lives in a sibling class ------------------------
    begin
      cls_sibling h_s = new;
      cls_derived_plain3 h_d = new;
      h_s.set_bare(5);
      h_d.set_via_this(31);
      `SVTEST_CHECK(h_d.get_bare() == 31,
        "G36 CONTROL: sibling-class shadow must NOT divert (31)")
    end

    // ---- G37 CONTROL: derived shadows with a real instance property ----------
    begin
      cls_derived_inst h_d = new;
      h_d.set_inst(31);
      h_d.set_via_this(7);
      `SVTEST_CHECK(h_d.get_inst() == 31 && h_d.get_bare() == 7,
        "G37 CONTROL: instance-property shadow keeps both slots separate")
    end

    // ---- G38 CONTROL: write through a $cast base-typed handle ----------------
    begin
      cls_derived_v2 h_d = new;
      cls_base_v2 h_b;
      void'($cast(h_b, h_d));
      h_b.prop_val = 31;
      `SVTEST_CHECK(h_d.get_bare() == 31,
        "G38 CONTROL: $cast base-typed handle write must reach the instance")
    end

    // ---- G39 CONTROL: shadow is an intermediate segment of the member chain --
    begin
      cls_derived_cfg h_d = new;
      h_d.set_mode(9);
      `SVTEST_CHECK(h_d.get_mode() == 9,
        "G39 CONTROL: intermediate-segment shadow must NOT divert (9)")
    end

    // ---- G40 CONTROL: chained handle obtained through a base-typed view ------
    begin
      cls_derived_v2 h_a = new;
      cls_derived_v2 h_c = new;
      cls_base_v2 h_b;
      h_a.child = h_c;
      h_b = h_a.child;
      h_b.prop_val = 31;
      `SVTEST_CHECK(h_c.get_bare() == 31,
        "G40 CONTROL: chained base-typed handle write must reach the instance")
    end

    // ---- G41 CONTROL: static event whose name is shadowed --------------------
    begin
      cls_derived_ev h_d = new;
      fork h_d.waiter(); join_none
      #10 h_d.fire();
      #10;
      `SVTEST_CHECK(h_d.hits == 1,
        "G41 CONTROL: static event with shadowed name must still fire (1)")
    end

    // ---- G42 CONTROL: static method writes through a handle argument ---------
    begin
      cls_derived_sm h_d = new;
      cls_base_sm::set_via_handle(h_d, 31);
      `SVTEST_CHECK(h_d.get_bare() == 31,
        "G42 CONTROL: static-method handle-argument write must reach the instance")
    end

    // ---- G43 CONTROL: bare static-slot member write inside a base method -----
    begin
      cls_derived_slot h_d = new;
      cls_base_slot::slot = h_d;
      h_d.set_slot(31);
      `SVTEST_CHECK(h_d.get_bare() == 31,
        "G43 CONTROL: bare static-slot member write must reach the instance")
    end

    `SVTEST_PASSFAIL
    $finish;
  end
endmodule


