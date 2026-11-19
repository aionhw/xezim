// §10.9.1, §10.9.2, §8.25.1, §26.3: lexical import order, struct member
// precedence, parameterized class-scope typedefs (type/value params, defaults,
// package-qualified, typedef alias, inheritance), and nested/sibling/child
// generate-block typedefs.
typedef integer PT;

package p;
  typedef int PT;
  class PkgC #(type T = int);
    typedef T cw_t;
  endclass
endpackage

class BaseC #(type T = int);
  typedef T cw_t;
endclass

class SubC #(type U = int) extends BaseC #(U);
endclass

class VecC #(int W = 8);
  typedef logic [W-1:0] vec_t;
endclass

module other;
  typedef real PT;
  PT unused;
endmodule

module child;
  typedef struct { integer a[2]; } CS;
  CS cv;
  integer x = 7;
  for (genvar i = 0; i < 1; i++) begin : cg
    typedef integer gw_t;
    initial begin
      cv = '{a:'{integer:x, gw_t:42, default:99}};
    end
  end
endmodule

module top;
  typedef struct { integer a[2]; } S;
  typedef struct { integer PT; integer b; } SM;
  typedef struct { logic [3:0] lo; logic [7:0] hi; } VS;
  typedef BaseC#(integer) base_int_t;

  S v_before, v_after, v_tdep, v_tdef, v_pkg, v_alias, v_sub, v_top_gen;
  SM sm_before, sm_after;
  VS vs_val;
  other o();
  child u();
  integer x = 7;

  // Before `import p::*`, PT is $unit::PT (integer).
  initial begin
    v_before = '{a:'{PT:x, default:99}};
    sm_before = '{PT:11, default:22};
    $display("T|before %0d %0d", v_before.a[0], v_before.a[1]);
    $display("T|sm_before %0d %0d", sm_before.PT, sm_before.b);
  end

  import p::*;

  // After `import p::*`, PT is p::PT (int != integer).
  initial begin
    v_after = '{a:'{PT:x, default:99}};
    sm_after = '{PT:11, default:22};
    $display("T|after %0d %0d", v_after.a[0], v_after.a[1]);
    $display("T|sm_after %0d %0d", sm_after.PT, sm_after.b);
  end

  // Parameterized class-scope typedef keys.
  initial begin
    v_tdep  = '{a:'{integer:x, BaseC#(integer)::cw_t:55, default:99}};
    v_tdef  = '{a:'{integer:x, BaseC#()::cw_t:55, default:99}};
    v_pkg   = '{a:'{integer:x, p::PkgC#(integer)::cw_t:66, default:99}};
    v_alias = '{a:'{integer:x, base_int_t::cw_t:77, default:99}};
    v_sub   = '{a:'{integer:x, SubC#(integer)::cw_t:88, default:99}};
    vs_val  = '{VecC#(4)::vec_t:4'hA, default:8'h5};
    $display("T|tdep %0d %0d", v_tdep.a[0], v_tdep.a[1]);
    $display("T|tdef %0d %0d", v_tdef.a[0], v_tdef.a[1]);
    $display("T|pkg %0d %0d", v_pkg.a[0], v_pkg.a[1]);
    $display("T|alias %0d %0d", v_alias.a[0], v_alias.a[1]);
    $display("T|sub %0d %0d", v_sub.a[0], v_sub.a[1]);
    $display("T|vec %0h %0h", vs_val.lo, vs_val.hi);
  end

  // Nested/sibling generate-block typedefs and outer-var writes.
  for (genvar gi = 0; gi < 2; gi++) begin : sib
    if (gi == 0) begin : b0
      typedef int gw_t;
      S sv;
      initial begin
        sv = '{a:'{integer:x, gw_t:33, default:99}};
        #2 $display("T|sib0 %0d %0d", sv.a[0], sv.a[1]);
      end
    end else begin : b1
      typedef integer gw_t;
      S sv;
      initial begin
        sv = '{a:'{integer:x, gw_t:33, default:99}};
        v_top_gen = '{a:'{integer:x, gw_t:44, default:99}};
        #2 $display("T|sib1 %0d %0d", sv.a[0], sv.a[1]);
      end
    end
  end

  initial begin
    #3;
    $display("T|top_gen %0d %0d", v_top_gen.a[0], v_top_gen.a[1]);
    $display("T|child_gen %0d %0d", u.cv.a[0], u.cv.a[1]);
  end
endmodule
