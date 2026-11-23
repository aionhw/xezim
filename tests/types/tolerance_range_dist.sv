// IEEE 1800-2023 §18.5.3: distribution items take tolerance ranges.
`timescale 1ns/1ps
class item;
  rand int value;
  constraint c { value dist { [10 +/- 1] := 1 }; }
endclass
class item_total;
  rand int value;
  constraint c { value dist { [10 +/- 1] :/ 1 }; }
endclass
class item_rel;
  rand int value;
  constraint c { value dist { [10 +%- 10] := 1 }; }
endclass
class item_rel_total;
  rand int value;
  constraint c { value dist { [10 +%- 10] :/ 1 }; }
endclass
class ritem;
  rand real value;
  constraint c { value dist { [8.5 +/- 0.125] :/ 1, [100.0 +%- 1.0] :/ 1 }; }
endclass
module top;
  initial begin
    item a = new;
    item_total b = new;
    item_rel c = new;
    item_rel_total d = new;
    $display("T|each %0d %0d %0d", a.randomize() with { value == 9; },
             a.randomize() with { value == 11; }, a.randomize() with { value == 8; });
    $display("T|total %0d %0d %0d", b.randomize() with { value == 9; },
             b.randomize() with { value == 11; }, b.randomize() with { value == 12; });
    $display("T|rel %0d %0d %0d", c.randomize() with { value == 9; },
             c.randomize() with { value == 11; }, c.randomize() with { value == 8; });
    $display("T|reltotal %0d %0d %0d", d.randomize() with { value == 9; },
             d.randomize() with { value == 11; }, d.randomize() with { value == 12; });
    $finish;
  end
endmodule
