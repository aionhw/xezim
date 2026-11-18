// IEEE 1800-2023 §11.4.13, §11.4.5, §12.5.4: `inside` and `case inside` in compiled
// continuous assignments, always blocks and class methods (#1281).
class K;
  real r;
  logic [3:0] v;
  function bit real_in(int n); return r inside {n, -2}; endfunction
  function int dec();
    case (v) inside
      4'b10??: return 1;
      [4'd1:4'd3]: return 2;
      4'b0000: return 3;
      default: return 0;
    endcase
  endfunction
  function int rdec();
    case (r) inside
      -1: return 1;
      2.5: return 2;
      default: return 0;
    endcase
  endfunction
endclass
module top;
  real r, nan_v;
  int i;
  logic [3:0] v;
  logic [7:0] w;
  parameter int NEG = -1;
  logic a_neg, a_lit, a_real_item, a_wild, a_fill;
  logic c_wild, c_range, c_real, c_int_real, c_nan, c_neg, c_wide, c1, c2, c3;
  int c_multi;
  assign a_neg = r inside {NEG};
  assign a_lit = r inside {-1};
  assign a_real_item = i inside {-1.0};
  assign a_wild = v inside {4'b10??};
  assign a_fill = r inside {'1};
  always_comb begin case (v) inside 4'b10??: c_wild = 1; default: c_wild = 0; endcase end
  always_comb begin case (v) inside [4'd10:4'd12]: c_range = 1; default: c_range = 0; endcase end
  always_comb begin case (r) inside -1: c_real = 1; default: c_real = 0; endcase end
  always_comb begin case (i) inside -1.0: c_int_real = 1; default: c_int_real = 0; endcase end
  always_comb begin case (nan_v) inside nan_v: c_nan = 1; default: c_nan = 0; endcase end
  always_comb begin case (r) inside NEG: c_neg = 1; default: c_neg = 0; endcase end
  always_comb begin case (w) inside 4'b1z11: c_wide = 1; default: c_wide = 0; endcase end
  always_comb begin
    case (v) inside
      4'b0000, 4'b0001: c_multi = 1;
      4'b1?11: c_multi = 2;
      [4'd12:4'd15]: c_multi = 3;
      default: c_multi = 0;
    endcase
  end
  always @* begin case (v) inside 4'b10??: c1 = 1; default: c1 = 0; endcase end
  always @(v) begin case (v) inside 4'b10??, 4'b0000: c2 = 1; default: c2 = 0; endcase end
  initial begin
    K k;
    r = -1.0; i = -1; v = 4'b1011; w = 8'h0b; nan_v = $bitstoreal(64'h7ff8000000000000);
    case (v) inside 4'b10??: c3 = 1; default: c3 = 0; endcase
    #1;
    $display("T|assign neg=%b lit=%b real_item=%b wild=%b fill=%b", a_neg, a_lit, a_real_item, a_wild, a_fill);
    $display("T|case wild=%b range=%b real=%b int_real=%b nan=%b neg=%b wide=%b multi=%0d star=%b at=%b init=%b", c_wild, c_range, c_real, c_int_real, c_nan, c_neg, c_wide, c_multi, c1, c2, c3);
    v = 4'b0001; r = 1.0;
    #1;
    $display("T|again wild=%b range=%b multi=%0d at=%b fill=%b", c_wild, c_range, c_multi, c2, a_fill);
    k = new; k.r = -2.0; k.v = 4'b1001;
    $display("T|class %b %0d", k.real_in(5), k.dec());
    k.v = 4'b0010; k.r = 2.5;
    $display("T|class %b %0d %0d", k.real_in(5), k.dec(), k.rdec());
    k.v = 4'b0000; k.r = -1.0;
    $display("T|class %b %0d %0d", k.real_in(-1), k.dec(), k.rdec());
    $finish;
  end
endmodule
