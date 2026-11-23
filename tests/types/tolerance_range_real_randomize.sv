// IEEE 1800-2023 §18.5.3: real inside and weighted dist ranges.
class weighted;
  rand real value;
  constraint c { value dist { [8.5 +/- 0.125] :/ 1, [20.125:20.25] :/ 3, [30:31] :/ 0 }; }
endclass
class inside_range;
  rand real value;
  constraint c { value inside {[-2.5 +%- 10]}; }
endclass
module top;
  initial begin
    weighted w = new;
    inside_range i = new;
    int near_count;
    int far_count;
    repeat (8) begin
      if (!w.randomize())
        $fatal(1, "dist failed");
      if (w.value >= 8.375 && w.value <= 8.625)
        near_count++;
      else if (w.value >= 20.125 && w.value <= 20.25)
        far_count++;
      else
        $fatal(1, "dist returned %g", w.value);
      if (!i.randomize() || i.value < -2.75 || i.value > -2.25)
        $fatal(1, "inside returned %g", i.value);
    end
    $display("T|near=%0d far=%0d", near_count, far_count);
    $finish;
  end
endmodule
