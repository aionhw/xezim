// IEEE 1800-2023 §8.25 / §18.4 / §7.4.5: randomization of class members
// whose type comes from a class type parameter bound to a type other than
// the default, and of members with a dynamic outer dimension over unpacked
// rows (`narrow_t cells[][2]`, `narrow_t both[][]`) (#576, reported by
// Ganesh-TNL; the report's testbench, plus checks G1 and G2).
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

package rtp_pkg;
   // sections A/B shapes
   typedef struct packed { logic        en; logic [34:0]  data; logic stop; } narrow_t; // 37
   typedef struct packed { logic        en; logic [35:0]  data; logic stop; } wide_t;   // 38
   typedef struct packed { logic [31:0] a;   logic [3:0]  b;                } pair_t;   // 36
   typedef narrow_t narrow_alias_t;                                        // alias of the default
   // sections C-F shapes (same two widths, different field name)
   typedef struct packed { logic en; logic [34:0] payload; logic stop; } t37_t; // 37
   typedef struct packed { logic en; logic [35:0] payload; logic stop; } t38_t; // 38
   typedef logic [8:0][7:0] pvA_t;   // 72 bits, 9 rows x 8
   typedef logic [7:0][6:0] pvB_t;   // 56 bits,  8 rows x 7
endpackage

import rtp_pkg::*;

// ----- A: element type supplied through a defaulted type parameter ---------
class xfer_c #(type BEAT_T = narrow_t);
   rand BEAT_T beats[];
endclass

class grid_c #(type BEAT_T = narrow_t);
   rand BEAT_T beats[4];
endclass

class queue_c #(type BEAT_T = narrow_t);
   rand BEAT_T beats[$];
endclass

class map_c #(type BEAT_T = narrow_t);
   rand BEAT_T beats[int];
endclass

// scalar control: same type parameter, no container
class item_c #(type BEAT_T = narrow_t);
   rand BEAT_T one;
endclass

// two containers driven by the one parameter, plus an unrelated rand scalar
class dual_c #(type BEAT_T = narrow_t);
   rand BEAT_T head[];
   rand BEAT_T tail[];
   rand bit    flag;
endclass

// the parameter forwarded down through a subclass
class base_xfer #(type BEAT_T = narrow_t);
   rand BEAT_T beats[];
endclass
class derived_xfer #(type BEAT_T = narrow_t) extends base_xfer #(BEAT_T);
endclass

// richer constraint forms on the failing shape
class style_c #(type BEAT_T = narrow_t);
   rand BEAT_T beats[];
   rand bit    pick;
endclass

// ----- B: nested dynamic array, no type parameter anywhere -----------------
class cells_c;
   rand narrow_t cells[][2];   // outer dynamic, inner fixed
   rand narrow_t flat[];       // single dynamic dim (control)
   rand narrow_t fixed[2][2];  // both dims fixed (control)
   rand narrow_t both[][];     // both dims dynamic
endclass


// ----- C: cousin sweep -- foreach, no default, alias, dist, solve-before ----
class fv  #(type T = t37_t); rand T items[]; endclass
class fvc #(type T = t37_t);
   rand T items[];
   constraint c { items.size() == 1; foreach (items[i]) items[i].en == 1; }
endclass

// no declared default at all
class nd  #(type T); rand T items[]; endclass
class nds #(type T); rand T one;     endclass

// the parameter reached through a typedef alias
class al  #(type T = t37_t);
   typedef T TA;
   rand TA one;
   rand TA items[];
endclass

// dist / soft / implication-landing on the failing shape
class dc #(type T = t37_t);
   rand T   items[];
   rand bit pick;
endclass

// solve-before on a param-typed scalar
class svc #(type T = t37_t);
   rand T   one;
   rand bit pick;
endclass

// parameterized element type through the parameter (type_args path)
class box #(type E = t37_t); rand E elem; endclass
class bx  #(type T = t37_t); rand box#(T) stuff[]; endclass

// ----- D: class handles, with and without the parameter --------------------
class leaf_a; rand bit en; rand bit [7:0] data; endclass
class leaf_b; rand bit en; rand bit [9:0] data; endclass

// no-parameter control
class hd0;
   rand leaf_a h;
   rand leaf_a arr[];
endclass

// the same shape through a type parameter
class hd #(type T = leaf_a);
   rand T h;
   rand T arr[];
endclass

// ----- E: multi-dimensional packed vector, with and without the parameter --
class pvc0; rand pvA_t w; endclass
class pvc #(type T = pvA_t); rand T w; endclass

module tb_top;
  `SVTEST_INIT

   // expect randomize() to succeed
   task automatic expect_ok(string tag, bit got);
      `SVTEST_CHECK(got, $sformatf("%s: randomize() returned 0, expected 1", tag))
   endtask

   // expect randomize() to succeed AND a readback condition to hold
   task automatic expect_ok_rb(string tag, bit got);
      `SVTEST_CHECK(got, $sformatf("%s: randomize() returned 0 or the readback disagreed, expected success", tag))
   endtask

   // expect randomize() to fail (constraint is deliberately contradictory)
   task automatic expect_unsat(string tag, bit got);
      `SVTEST_CHECK(!got, $sformatf("%s: randomize() returned 1 on an unsatisfiable pair, expected 0", tag))
   endtask

   initial begin
      bit ok;

      // ================= A: the type-parameter width bug ====================
      // control -- supplied argument equals the declared default
      begin xfer_c #(narrow_t) o = new();
           ok = o.randomize() with { beats.size() == 1; beats[0].en == 1; };
           expect_ok("A1 dynamic [], argument == default", ok); end

      // the bug -- one bit wider than the declared default
      begin xfer_c #(wide_t) o = new();
           ok = o.randomize() with { beats.size() == 1; beats[0].en == 1; };
           expect_ok("A2 dynamic [], argument 1 bit wider than default", ok); end

      // the bug -- a different struct family, different width
      begin xfer_c #(pair_t) o = new();
           ok = o.randomize() with { beats.size() == 1; beats[0].a == 36'h5; };
           expect_ok("A3 dynamic [], unrelated struct of a different width", ok); end

      // control -- alias of the declared default
      begin xfer_c #(narrow_alias_t) o = new();
           ok = o.randomize() with { beats.size() == 1; beats[0].en == 1; };
           expect_ok("A4 dynamic [], typedef alias of the default", ok); end

      // controls -- shapes on the failing element type that do not name a field
      begin xfer_c #(wide_t) o = new();
           ok = o.randomize() with { beats.size() == 1; };
           expect_ok("A5 dynamic [], size() only, width != default", ok); end
      begin xfer_c #(wide_t) o = new();
           ok = o.randomize() with { beats.size() == 1; beats[0] == '0; };
           expect_ok("A6 dynamic [], whole-element equality, width != default", ok); end

      // the bug across the other container kinds
      begin grid_c #(wide_t) o = new();
           ok = o.randomize() with { beats[0].en == 1; };
           expect_ok("A7 fixed [4], width != default", ok); end
      begin queue_c #(wide_t) o = new();
           ok = o.randomize() with { beats.size() == 1; beats[0].en == 1; };
           expect_ok("A8 queue [$], width != default", ok); end
      begin map_c #(wide_t) o = new();
           o.beats[0] = '0;
           ok = o.randomize() with { beats[0].en == 1; };
           expect_ok("A9 assoc [int], width != default", ok); end

      // controls -- the same containers at the default width
      begin grid_c #(narrow_t) o = new();
           ok = o.randomize() with { beats[0].en == 1; };
           expect_ok("A10 CONTROL fixed [4] at default width", ok); end
      begin queue_c #(narrow_t) o = new();
           ok = o.randomize() with { beats.size() == 1; beats[0].en == 1; };
           expect_ok("A11 CONTROL queue [$] at default width", ok); end

      // control -- the SCALAR property is fine at either width
      begin item_c #(narrow_t) o = new();
           ok = o.randomize() with { one.en == 1; };
           expect_ok("A12 CONTROL scalar at default width", ok); end
      begin item_c #(wide_t) o = new();
           ok = o.randomize() with { one.en == 1; };
           expect_ok("A13 CONTROL scalar at a different width", ok); end

      // the bug through two containers sharing one parameter
      begin dual_c #(wide_t) o = new();
           ok = o.randomize() with { head.size() == 1; tail.size() == 1;
                                     head[0].en == 1; flag == 1; };
           expect_ok("A14 two containers + a scalar, one parameter", ok); end

      // the bug through a subclass that forwards the parameter
      begin derived_xfer #(wide_t) o = new();
           ok = o.randomize() with { beats.size() == 1; beats[0].en == 1; };
           expect_ok("A15 subclass forwards the parameter, width != default", ok); end
      begin derived_xfer #(narrow_t) o = new();
           ok = o.randomize() with { beats.size() == 1; beats[0].en == 1; };
           expect_ok("A16 CONTROL subclass forwards the parameter, at default", ok); end

      // other constraint forms on the failing shape
      begin style_c #(wide_t) o = new();
           ok = o.randomize() with { beats.size() == 1; beats[0].data inside {[10:20]}; };
           expect_ok("A17 inside {..} on an element field, width != default", ok); end
      begin style_c #(wide_t) o = new();
           ok = o.randomize() with { beats.size() == 1;
                                     beats[0].en == 1 -> beats[0].stop == 1; };
           expect_ok("A18 implication across element fields, width != default", ok); end

      // nothing may be silently ignored: these pairs are contradictory, so a
      // randomize() of 1 here would mean the constraint was dropped, not solved
      begin style_c #(narrow_t) o = new();
           ok = o.randomize() with { beats.size() == 1;
                                     beats[0].en == 1; beats[0].en == 0; };
           expect_unsat("A19 two conflicting element field constraints", ok); end
      begin style_c #(narrow_t) o = new();
           ok = o.randomize() with { beats.size() == 1;
                                     beats[0].data inside {[10:20]}; beats[0].data == 100; };
           expect_unsat("A20 inside {..} contradicted by an equality", ok); end
      begin style_c #(narrow_t) o = new();
           ok = o.randomize() with { beats.size() == 1; pick == 1;
                                     beats[0].en == 1; beats[0].stop == 0;
                                     beats[0].en == 1 -> beats[0].stop == 1; };
           expect_unsat("A21 implication contradicted by fixed fields", ok); end

      // ============ B: nested dynamic array, no type parameter at all =======
      begin cells_c o = new();
           ok = o.randomize() with { flat.size() == 1; flat[0].en == 1; };
           expect_ok("B1 CONTROL single dynamic dim + member select", ok); end
      begin cells_c o = new();
           ok = o.randomize() with { fixed[0][0].en == 1; };
           expect_ok("B2 CONTROL both dims fixed + member select", ok); end
      begin cells_c o = new();
           ok = o.randomize() with { cells.size() == 1; cells[0][0].en == 1; };
           expect_ok("B3 outer dynamic / inner fixed + member select", ok); end
      begin cells_c o = new();
           ok = o.randomize() with { both.size() == 1; both[0].size() == 1; both[0][0].en == 1; };
           expect_ok("B4 both dims dynamic + member select", ok); end
      begin cells_c o = new();
           ok = o.randomize() with { cells.size() == 1; };
           expect_ok("B5 CONTROL nested shape, size() only", ok); end
      begin cells_c o = new();
           ok = o.randomize() with { cells.size() == 1; cells[0][0] == '1; };
           expect_ok("B6 CONTROL nested shape, whole-element equality", ok); end

      // ============ C: cousin sweep over parameterized shapes ===============
      // foreach -- inline and inside a class constraint block
      begin fv#(t38_t) o = new();
           ok = o.randomize() with { items.size()==2; foreach (items[i]) items[i].en==1; };
           expect_ok("C1 foreach inline, width != default", ok); end
      begin fv#(t38_t) o = new();
           ok = o.randomize() with { items.size()==2; foreach (items[i]) items[i].en==1; };
           expect_ok_rb("C2 foreach inline, width != default, values land",
                        ok && o.items[0].en==1 && o.items[1].en==1); end
      begin fvc#(t38_t) o = new();
           ok = o.randomize();
           expect_ok_rb("C3 foreach in class constraint, width != default",
                        ok && o.items.size()==1 && o.items[0].en==1); end
      begin fvc#(t37_t) o = new();
           ok = o.randomize();
           expect_ok_rb("C4 CONTROL foreach in class constraint, == default",
                        ok && o.items.size()==1 && o.items[0].en==1); end

      // no declared default at all
      begin nd#(t38_t) o = new();
           ok = o.randomize() with { items.size()==1; items[0].en==1; };
           expect_ok("C5 no default, array member select", ok); end
      begin nds#(t38_t) o = new();
           ok = o.randomize() with { one.en==1; };
           expect_ok("C6 no default, SCALAR member select", ok); end

      // the parameter reached through a typedef alias
      begin al#(t38_t) o = new();
           ok = o.randomize() with { one.en==1; };
           expect_ok("C7 alias, SCALAR member select, width != default", ok); end
      begin al#(t38_t) o = new();
           ok = o.randomize() with { items.size()==1; items[0].en==1; };
           expect_ok("C8 alias, array member select, width != default", ok); end
      begin al#(t37_t) o = new();
           ok = o.randomize() with { items.size()==1; items[0].en==1; };
           expect_ok("C9 CONTROL alias, array, == default", ok); end

      // dist and a pinned-antecedent implication on the failing shape
      begin dc#(t38_t) o = new();
           ok = o.randomize() with { items.size()==1; items[0].payload dist { 10 := 1, 20 := 2 }; };
           expect_ok("C10 dist on element field, width != default", ok); end
      begin dc#(t37_t) o = new();
           ok = o.randomize() with { items.size()==1; items[0].payload dist { 10 := 1, 20 := 2 }; };
           expect_ok("C11 CONTROL dist on element field, == default", ok); end
      begin dc#(t38_t) o = new();
           ok = o.randomize() with { items.size()==1; pick==1; pick==1 -> items[0].en==1; };
           expect_ok_rb("C12 implication lands its value, width != default",
                        ok && o.items[0].en==1); end

      // solve ... before on a param-typed scalar
      begin svc#(t38_t) o = new();
           ok = o.randomize() with { solve one before pick; one.en==1; pick==1; };
           expect_ok_rb("C13 solve-before on param-typed scalar, width != default",
                        ok && o.one.en==1); end

      // parameterized element type (type_args path)
      begin bx#(t37_t) o = new(); o.stuff = new[1]; o.stuff[0] = new();
           ok = o.randomize() with { stuff.size()==1; stuff[0].elem.en==1; };
           expect_ok_rb("C14 box#(T) element, == default", ok && o.stuff[0].elem.en==1); end
      begin bx#(t38_t) o = new(); o.stuff = new[1]; o.stuff[0] = new();
           ok = o.randomize() with { stuff.size()==1; stuff[0].elem.en==1; };
           expect_ok_rb("C15 box#(T) element, width != default", ok && o.stuff[0].elem.en==1); end

      // ============ D: class handles through the parameter ==================
      // no-parameter controls: plain class-handle members work
      begin hd0 o = new(); o.h = new();
           ok = o.randomize() with { h.en==1; };
           expect_ok("D1 CONTROL no-param handle SCALAR: randomize returns 1", ok); end
      begin hd0 o = new(); o.h = new();
           ok = o.randomize() with { h.en==1; };
           expect_ok_rb("D2 CONTROL no-param handle SCALAR: handle survives",
                        ok && o.h != null); end
      begin hd0 o = new(); o.h = new();
           ok = o.randomize() with { h.en==1; };
           expect_ok_rb("D3 CONTROL no-param handle SCALAR: value lands",
                        ok && o.h.en==1); end
      begin hd0 o = new(); o.arr = new[1]; o.arr[0] = new();
           ok = o.randomize() with { arr.size()==1; arr[0].en==1; };
           expect_ok_rb("D4 CONTROL no-param handle ARRAY: value lands",
                        ok && o.arr[0].en==1); end

      // the same handles through the parameter: fails even at the default,
      // and the handle is left null after randomize() reports failure
      begin hd#(leaf_a) o = new(); o.h = new();
           ok = o.randomize() with { h.en==1; };
           expect_ok("D5 param handle SCALAR == default: randomize returns 1", ok); end
      begin hd#(leaf_a) o = new(); o.h = new();
           ok = o.randomize() with { h.en==1; };
           expect_ok_rb("D6 param handle SCALAR == default: handle survives",
                        ok && o.h != null); end
      begin hd#(leaf_a) o = new(); o.h = new();
           ok = o.randomize() with { h.en==1; };
           expect_ok_rb("D7 param handle SCALAR == default: value lands",
                        ok && o.h.en==1); end
      begin hd#(leaf_a) o = new(); o.arr = new[1]; o.arr[0] = new();
           ok = o.randomize() with { arr.size()==1; arr[0].en==1; };
           expect_ok_rb("D8 param handle ARRAY == default: value lands",
                        ok && o.arr[0].en==1); end
      begin hd#(leaf_b) o = new(); o.h = new();
           ok = o.randomize() with { h.en==1; };
           expect_ok_rb("D9 param handle SCALAR, argument != default: value lands",
                        ok && o.h != null && o.h.en==1); end
      begin hd#(leaf_b) o = new(); o.arr = new[1]; o.arr[0] = new();
           ok = o.randomize() with { arr.size()==1; arr[0].en==1; };
           expect_ok_rb("D10 param handle ARRAY, argument != default: value lands",
                        ok && o.arr[0] != null && o.arr[0].en==1); end

      // ============ E: multi-dimensional packed vector ======================
      begin pvc0 o = new();
           ok = o.randomize() with { w[0]==8'd5; w[1]==8'd7; };
           expect_ok_rb("E1 CONTROL no-param packed 2-D: values land",
                        ok && o.w[0]==8'd5 && o.w[1]==8'd7); end
      begin pvc#(pvA_t) o = new();
           ok = o.randomize() with { w[0]==8'd5; w[1]==8'd7; };
           expect_ok_rb("E2 param packed 2-D == default: values land",
                        ok && o.w[0]==8'd5 && o.w[1]==8'd7); end
      begin pvc#(pvB_t) o = new();
           ok = o.randomize() with { w[0]==8'd5; w[1]==8'd7; };
           expect_ok_rb("E3 param packed 2-D != default: values land",
                        ok && o.w[0]==8'd5 && o.w[1]==8'd7); end

      // ============ F: soft constraints on element fields ===================
      // at mismatched width the soft constraint is dropped silently:
      // randomize() returns 1 (F2) but the field reads back unconstrained (F3)
      begin dc#(t37_t) o = new();
           ok = o.randomize() with { items.size()==1; soft items[0].en==1; };
           expect_ok_rb("F1 CONTROL soft member == default: value lands",
                        ok && o.items[0].en==1); end
      begin dc#(t38_t) o = new();
           ok = o.randomize() with { items.size()==1; soft items[0].en==1; };
           expect_ok("F2 soft member width != default: randomize returns 1", ok); end
      begin dc#(t38_t) o = new();
           ok = o.randomize() with { items.size()==1; soft items[0].en==1; };
           expect_ok_rb("F3 soft member width != default: value lands",
                        ok && o.items[0].en==1); end
      begin dc#(t38_t) o = new();
           ok = o.randomize() with { items.size()==1; soft items[0].en==1; soft items[0].stop==1; };
           expect_ok_rb("F4 two soft members width != default: both land",
                        ok && o.items[0].en==1 && o.items[0].stop==1); end
      begin dc#(t37_t) o = new();
           ok = o.randomize() with { items.size()==1; soft items[0].en==1; items[0].en==0; };
           expect_ok_rb("F5 CONTROL soft yields to hard equality",
                        ok && o.items[0].en==0); end

      // ============ G: construction and packed selects outside a solve ===
      // `o.h = new()` from outside the class builds the bound class, not an
      // object of the parameter's name; an element `o.arr[0] = new()` too.
      begin hd#(leaf_b) o = new(); o.h = new(); o.arr = new[1]; o.arr[0] = new();
           o.h.data = 10'h3ff; o.arr[0].data = 10'h201;
           `SVTEST_CHECK(o.h.data == 10'h3ff && o.arr[0].data == 10'h201,
                         "G1 new() on type-parameter handle and element builds the bound class") end
      // a packed 2-D property through the parameter takes element writes
      begin pvc#(pvB_t) o = new(); o.w = '0; o.w[1] = 7'd7; o.w[0] = 7'd5;
           `SVTEST_CHECK(o.w[1] == 7'd7 && o.w[0] == 7'd5 && $bits(o.w) == 56,
                         "G2 packed 2-D type-parameter property element write") end

      `SVTEST_PASSFAIL
      $finish;
   end

endmodule

