// IEEE 1800-2023 §11.4.13, §12.5.4 and §13.4.3: inclusive case ranges
// in constant functions used by generate constructs and packed dimensions.
`timescale 1ns/1ps
module top;
  function automatic int classify(input int key);
    case (key) inside
      [$:4]: return 2;
      [8:12]: return 4;
      [16:$]: return 3;
      15: return 5;
      default: return 1;
    endcase
  endfunction
  function automatic int classify_real(input real key);
    case (key) inside
      [0.5:0.75]: return 2;
      1.0: return 3;
      default: return 1;
    endcase
  endfunction
  function automatic int classify_real_value(input real key);
    case (key) inside
      4: return 2;
      default: return 1;
    endcase
  endfunction
  function automatic int classify_byte(input byte key);
    case (key) inside
      [8'd254:8'd255]: return 4;
      32'd4: return 2;
      default: return 1;
    endcase
  endfunction
  logic [classify(8)-1:0] lower_hit;
  logic [classify(12)-1:0] upper_hit;
  logic [classify(13)-1:0] miss;
  logic [classify(4)-1:0] open_lower;
  logic [classify(16)-1:0] open_upper;
  logic [classify(15)-1:0] value_hit;
  logic [classify_real(0.75)-1:0] real_range;
  logic [classify_real(1.0)-1:0] real_value;
  // §11.4.13: a real selector uses numeric equality with a value item.
  logic [classify_real_value(4.0):0] real_value_only;
  // §11.8.2: the 8-bit comparison is unsigned because the bounds are unsigned.
  logic [classify_byte(-1)-1:0] byte_range;
  if (classify(11) == 4) begin
    initial begin
      if ($bits(lower_hit) != 4 || $bits(upper_hit) != 4 || $bits(miss) != 1 ||
          $bits(open_lower) != 2 || $bits(open_upper) != 3 || $bits(value_hit) != 5 ||
          $bits(real_range) != 2 || $bits(real_value) != 3 || $bits(byte_range) != 4)
        $fatal(1, "constant function range results");
      if ($bits(real_value_only) != 3)
        $fatal(1, "constant function real value-only result");
      $display("PASS");
      $finish;
    end
  end else begin
    initial $fatal(1, "constant range chose the wrong generate branch");
  end
endmodule
