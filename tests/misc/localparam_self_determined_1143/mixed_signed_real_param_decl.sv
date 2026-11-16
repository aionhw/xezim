class cls_helper;
  function int eval(int base);
    localparam [3:0] M = 4'd10;
    localparam int K = 5;
    return base + M + K;
  endfunction
endclass

module top;
  function automatic int fn_calc();
    localparam Q = 4'd0 - 4'd1;
    localparam signed S = 4'd15;
    localparam A = 1.5, B = 2;
    return Q + S + int'(A * 10.0) + B;
  endfunction

  function automatic int fn_runtime(input int dummy);
    localparam Q = 4'd0 - 4'd1;
    localparam signed S = 4'd15;
    localparam A = 1.5, B = 2;
    if (Q === 4'd15 && $bits(Q) == 4 && S === -4'sd1 && $bits(S) == 4 && A == 1.5 && B === 2)
      $display("PASS rt Q=%0d S=%0d A=%0.1f B=%0d", Q, S, A, B);
    else
      $display("FAIL rt Q=%0d S=%0d A=%0.1f B=%0d", Q, S, A, B);
    return Q + S + int'(A * 10.0) + B + dummy;
  endfunction

  localparam int C = fn_calc();

  initial begin
    static logic v = 4'd0 - 4'd1;
    localparam Q = 4'd0 - 4'd1;
    localparam signed S = 4'd15;
    localparam A = 1.5, B = 2;
    int r;
    cls_helper h = new();
    if (Q === 4'd15 && $bits(Q) == 4 && S === -4'sd1 && $bits(S) == 4
        && A == 1.5 && B === 2 && v === 1'b1 && $bits(v) == 1 && C === 47)
      $display("PASS blk C=%0d", C);
    else
      $display("FAIL blk Q=%0d S=%0d A=%0.1f B=%0d SV=%0d C=%0d", Q, S, A, B, v, C);
    r = fn_runtime(1);
    if (r === 48 && h.eval(10) === 25)
      $display("PASS ret=%0d", r);
    else
      $display("FAIL ret=%0d cls=%0d", r, h.eval(10));
  end
endmodule
