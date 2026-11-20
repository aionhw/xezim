// IEEE 1800-2023 §11.4.13, §12.5.4 and §13.4.3: tolerance ranges
// in constant functions used by generate constructs and packed dimensions.
`timescale 1ns/1ps
module top;
  function automatic int classify(input int key);
    case (key) inside
      [10 +/- 2]: return 4;
      [40 +%- 10]: return 3;
      default: return 1;
    endcase
  endfunction
  function automatic int classify_real(input real key);
    case (key) inside
      [10.0 +%- 5.0]: return 2;
      default: return 1;
    endcase
  endfunction
  logic [classify(8)-1:0] lower_hit;
  logic [classify(12)-1:0] upper_hit;
  logic [classify(13)-1:0] miss;
  logic [classify(36)-1:0] relative_hit;
  logic [classify_real(10.5)-1:0] real_hit;
  logic [classify_real(10.6)-1:0] real_miss;
  if (classify(11) == 4) begin
    initial begin
      if ($bits(lower_hit) != 4 || $bits(upper_hit) != 4 || $bits(miss) != 1 ||
          $bits(relative_hit) != 3 || $bits(real_hit) != 2 || $bits(real_miss) != 1)
        $fatal(1, "constant function tolerance results");
      $display("PASS");
      $finish;
    end
  end else begin
    initial $fatal(1, "constant tolerance chose the wrong generate branch");
  end
endmodule
