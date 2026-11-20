// IEEE 1800-2023 §11.4.13, §12.5.3, §12.5.4.
`timescale 1ns/1ps
module top;
  real value;
  logic [7:0] key;
  int result;

  function automatic int classify(input real v);
    case (v) inside
      [1.0 +/- 0.001]: return 1;
      [10.0 +%- 5.0]:  return 2;
      default:         return 0;
    endcase
  endfunction

  initial begin
    value = 1.0005;
    if (classify(value) != 1 || (value inside {[1.0 +/- 0.001]}) !== 1'b1)
      $fatal(1, "real absolute tolerance");
    value = 1.01;
    if (classify(value) != 0 || (value inside {[1.0 +/- 0.001]}) !== 1'b0)
      $fatal(1, "outside real absolute tolerance");
    // [10.0 +%- 5.0] includes both 9.5 and 10.5.
    value = 9.5;
    if (classify(value) != 2 || (value inside {[10.0 +%- 5.0]}) !== 1'b1)
      $fatal(1, "lower real relative bound");
    value = 10.5;
    if (classify(value) != 2 || (value inside {[10.0 +%- 5.0]}) !== 1'b1)
      $fatal(1, "upper real relative bound");
    value = 10.6;
    if (classify(value) != 0 || (value inside {[10.0 +%- 5.0]}) !== 1'b0)
      $fatal(1, "outside real relative tolerance");

    // Each selector matches at most one unique-case arm (§12.5.3).
    for (int i = 8; i <= 45; i++) begin
      key = 8'(i);
      unique case (key) inside
        [10 +/- 2], [40 +%- 10], [20:21]: result = 1;
        [$:4], [45:$]: result = 2;
        default: result = 0;
      endcase
      if (result != (((i <= 12) || (i >= 36 && i <= 44) || i == 20 || i == 21)
                     ? 1 : (i == 45 ? 2 : 0)))
        $fatal(1, "mixed case item for %0d", i);
    end
    key = 2;
    priority case (key) inside
      [10 +/- 2], [$:4]: result = 1;
      [2 +/- 1]: result = 2;
      default: result = 0;
    endcase
    if (result != 1) $fatal(1, "first matching priority item");

    // A range comparison with an X/Z selector cannot select the arm (§12.5.4).
    key = 8'b0000x000;
    repeat (2) begin
      case (key) inside
        [10 +/- 2]: result = 1;
        default: result = 0;
      endcase
      if (result != 0) $fatal(1, "X/Z selector matched a tolerance range");
      key = 8'b0000z000;
    end
    $display("PASS");
    $finish;
  end
endmodule
