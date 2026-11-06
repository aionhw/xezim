`timescale 1ns/1ps
// Neighbour probes for #928: packed arrays of packed structs as class
// properties (evaluator + constraints) and wide array reductions / chains.
typedef struct packed { logic valid; logic [7:0] mode; } cand_t;     // 9 bits
typedef cand_t [0:2] carr_t;                                         // 27 bits
typedef struct packed { cand_t [0:1] arr; logic [3:0] pad; } wrap_t; // 22 bits
typedef struct packed { logic en; cand_t [1:0] pr; } mix_t;          // 19 bits

class n_tdcl2_c;  rand carr_t [1:0] x; endclass           // [1:0][0:2] of cand_t
class n_wrap_c;   rand wrap_t w; endclass                 // struct holding a struct array
class n_mix_c;    rand mix_t m; constraint c { m.pr[1].valid == 1'b1; m.pr[0].mode == 8'h3C; } endclass
class n_psel_c;   rand cand_t [0:2] a; constraint c { a[1].mode[3:0] == 4'hA; a[1].mode[7:4] == 4'h5; } endclass
class n_inside_c; rand cand_t [0:2] a; constraint c { a[0] inside {[9'h100:9'h1FF]}; a[2] inside {9'h001, 9'h002}; } endclass
class n_fe_c;     rand cand_t [0:2] a; constraint c { foreach (a[i]) a[i].mode == i + 8'h10; } endclass
class n_desc_c;   rand cand_t [2:0] a; constraint c { a[2].valid == 1'b1; a[0].valid == 1'b0; a[1].mode == 8'hEE; } endclass
class n_wide_c;   rand cand_t [0:9] a; constraint c { a[9].mode == 8'h99; a[0].mode == 8'h00; } endclass // 90 bits
class n_cmp_c;    rand cand_t [0:1] a; constraint c { a[0].mode > a[1].mode; a[1].mode > 8'h80; } endclass
class n_dist_c;   rand cand_t [0:1] a; constraint c { a[0].mode dist { 8'h11 := 1 }; } endclass
class n_meth_c;
  rand cand_t [0:2] a;
  function void set(int i, logic v, logic [7:0] m); a[i].valid = v; a[i].mode = m; endfunction
  function logic [7:0] get_mode(int i); return a[i].mode; endfunction
  function int nbits(); return $bits(a); endfunction
  function int ebits(); return $bits(a[0]); endfunction
endclass

// wide reductions / chains, more shapes
class w_and128_c; rand bit [127:0] p[2]; constraint x { p.and() == 128'h1_0000_0000_0000_0000_0000_0000_0000_000F; } endclass
class w_xor3_c;   rand bit [95:0]  p[3]; constraint x { p.xor() == 96'h8_0000_0000_0000_0001; } endclass
class w_ordyn_c;  rand bit [95:0]  p[];  constraint s { p.size() == 3; } constraint x { p.or() == 96'h1_0000_0000_0000_00F0; } endclass
class w_andpin_c; rand bit [95:0]  p[2]; constraint z { p[1] == 96'h1_0000_0000_0000_00FF; } constraint x { p.and() == 96'h1_0000_0000_0000_000F; } endclass
class w_pr0_c;    rand bit [95:0]  p[2]; constraint x { p.product() == 96'd0; } endclass
class w_rev_c;    rand bit [95:0]  a[2]; constraint x { 96'h2_0000_0000_0000_0000 == a[0] + a[1]; } endclass
class w_sumfld_c; rand cand_t [0:1] a; endclass
class w_q_c;      rand bit [95:0]  q[$]; constraint s { q.size() == 2; } constraint x { q.xor() == 96'h1_0000_0000_0000_0000; } endclass

module tb_top;
  int failures = 0;
  `define CHK(expr, msg) if (!(expr)) begin failures++; $display("T|FAIL %s", msg); end else $display("T|PASS %s", msg);

  initial begin
    begin n_tdcl2_c o; o = new(); o.x[1][2] = 9'h1FF; o.x[0][0] = 9'h0A5;
      `CHK(o.x[1][2].valid === 1'b1, "tdcl2 [1][2].valid")
      `CHK(o.x[0][0].mode === 8'hA5, "tdcl2 [0][0].mode")
      `CHK(o.x[1][2] === 9'h1FF, "tdcl2 [1][2] whole")
      `CHK(o.x[0][1] === 9'hx, "tdcl2 untouched element x")
      `CHK($bits(o.x) == 54, "tdcl2 $bits whole")
    end
    begin n_wrap_c o; o = new(); o.w.arr[1] = 9'h1FF; o.w.pad = 4'h3; o.w.arr[0].mode = 8'h42;
      `CHK(o.w.arr[1].valid === 1'b1, "wrap arr[1].valid")
      `CHK(o.w.arr[0].mode === 8'h42, "wrap arr[0].mode")
      `CHK(o.w.arr[0].valid === 1'bx, "wrap arr[0].valid x")
      `CHK(o.w.pad === 4'h3, "wrap pad")
      `CHK(o.w[21:0] === {1'bx, 8'h42, 9'h1FF, 4'h3}, "wrap whole")
    end
    begin n_mix_c o; o = new();
      `CHK(o.randomize() == 1, "mix member constraint solves")
      `CHK(o.m.pr[1].valid === 1'b1 && o.m.pr[0].mode === 8'h3C, "mix member constraint honoured")
    end
    begin n_psel_c o; o = new();
      `CHK(o.randomize() == 1, "part-select member solves")
      `CHK(o.a[1].mode === 8'h5A, "part-select member honoured")
    end
    begin n_inside_c o; o = new();
      `CHK(o.randomize() == 1, "element inside solves")
      `CHK(o.a[0] >= 9'h100 && (o.a[2] == 9'h001 || o.a[2] == 9'h002), "element inside honoured")
    end
    begin n_fe_c o; o = new();
      `CHK(o.randomize() == 1, "foreach element mode solves")
    end
    begin n_desc_c o; o = new();
      `CHK(o.randomize() == 1, "descending solves")
      `CHK(o.a[2].valid === 1'b1 && o.a[0].valid === 1'b0 && o.a[1].mode === 8'hEE, "descending honoured")
    end
    begin n_wide_c o; o = new();
      `CHK(o.randomize() == 1, "90-bit array solves")
      `CHK(o.a[9].mode === 8'h99 && o.a[0].mode === 8'h00, "90-bit array honoured")
    end
    begin n_cmp_c o; o = new();
      `CHK(o.randomize() == 1, "cross-element compare solves")
      `CHK(o.a[0].mode > o.a[1].mode && o.a[1].mode > 8'h80, "cross-element compare honoured")
    end
    begin n_dist_c o; o = new();
      `CHK(o.randomize() == 1, "element dist solves")
      `CHK(o.a[0].mode === 8'h11, "element dist honoured")
    end
    begin n_meth_c o; o = new(); o.set(2, 1'b1, 8'hC3);
      `CHK(o.get_mode(2) === 8'hC3 && o.a[2].valid === 1'b1, "method write/read")
      `CHK(o.nbits() == 27 && o.ebits() == 9, "$bits in method")
    end
    begin w_and128_c o; o = new(); `CHK(o.randomize() == 1, "and() 128-bit") end
    begin w_xor3_c o;   o = new(); `CHK(o.randomize() == 1, "xor() three 96-bit") end
    begin w_ordyn_c o;  o = new(); `CHK(o.randomize() == 1, "or() dynamic 96-bit") end
    begin w_andpin_c o; o = new();
      `CHK(o.randomize() == 1, "and() with pinned element")
      `CHK(o.p[1] === 96'h1_0000_0000_0000_00FF && (o.p[0] & o.p[1]) === 96'h1_0000_0000_0000_000F, "and() pinned honoured")
    end
    begin w_pr0_c o;    o = new(); `CHK(o.randomize() == 1, "product() zero target") end
    begin w_rev_c o;    o = new();
      `CHK(o.randomize() == 1, "reversed chain")
      `CHK(o.a[0] + o.a[1] === 96'h2_0000_0000_0000_0000, "reversed chain honoured")
    end
    begin w_q_c o;      o = new(); `CHK(o.randomize() == 1, "xor() queue 96-bit") end
    if (failures == 0) $display("T|TEST_PASS"); else $display("T|TEST_FAIL count=%0d", failures);
    $finish;
  end
endmodule
