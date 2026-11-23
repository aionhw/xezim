// IEEE 1800-2023 §11.4.13: the tolerance is converted to the type of the
// centre (truncated from real to integral) and the bounds are ordered.
`timescale 1ns/1ps
class item;
  rand int x;
  constraint c { x inside {[50 +/- 3]}; }
endclass
module top;
  int center;
  int unsigned ucenter;
  real rc;
  logic [7:0] y;
  bit hit;
  assign hit = y inside {[center +/- 1]};
  initial begin
    item it;
    int bad;
    center = 7;
    ucenter = 10;
    rc = 8.5;
    $display("T|rel %0d %0d %0d %0d", 5 inside {[center +%- 25]}, 6 inside {[center +%- 25]},
             8 inside {[center +%- 25]}, 9 inside {[center +%- 25]});
    $display("T|abs %0d %0d %0d %0d", 5.5 inside {[center +/- 1.75]}, 6.0 inside {[center +/- 1.75]},
             8.0 inside {[center +/- 1.75]}, 8.5 inside {[center +/- 1.75]});
    $display("T|neg %0d %0d %0d %0d", 9 inside {[10 +/- (-1)]}, 11 inside {[10 +/- (-1)]},
             9 inside {[10 +%- (-10)]}, 11 inside {[10 +%- (-10)]});
    $display("T|uns %0d %0d %0d", 9 inside {[ucenter +/- (-1)]}, 11 inside {[ucenter +/- (-1)]},
             12 inside {[ucenter +/- (-1)]});
    $display("T|real %0d %0d %0d %0d", 8.375 inside {[rc +/- 0.125]}, 8.7 inside {[rc +/- 0.125]},
             8.42 inside {[rc +%- 1.0]}, 8.6 inside {[rc +%- 1.0]});
    $display("T|int-real %0d %0d", 3 inside {[5 +/- 2.9]}, 2 inside {[5 +/- 2.9]});
    case (8) inside
      [center +/- 1.75]: $display("T|case hit");
      default: $display("T|case miss");
    endcase
    case (9) inside
      [center +/- 1.75]: $display("T|case hit");
      default: $display("T|case miss");
    endcase
    y = 8;
    #1 $display("T|cont %0d", hit);
    center = 20;
    #1 $display("T|cont %0d", hit);
    it = new;
    bad = 0;
    repeat (50) begin
      void'(it.randomize());
      if (it.x < 47 || it.x > 53) bad++;
    end
    $display("T|constraint bad=%0d", bad);
    $display("T|constraint 47:%0d 53:%0d 46:%0d", it.randomize() with { x == 47; },
             it.randomize() with { x == 53; }, it.randomize() with { x == 46; });
    $finish;
  end
endmodule
