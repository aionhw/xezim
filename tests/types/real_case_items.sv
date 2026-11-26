// IEEE 1800-2023 §12.5, §11.8.2: a case statement whose case expression or
// item is real compares the pair numerically (case equality, `===`, is not
// defined for real operands, Table 11-1), interpreted and compiled.
module top;
  real one = 1.0, half = 0.5, tiny;
  int i = 2;
  logic [3:0] n = 4'd1;
  int a, b, c, d, e;
  int f, g, h, k, m;
  always @(one) begin
    case (one) 1: a = 1; default: a = 0; endcase
    case (one) '1: b = 1; default: b = 0; endcase
    case (i) 2.0: c = 1; default: c = 0; endcase
    case (half * 2) n: d = 1; default: d = 0; endcase
    case (one) 3, 2.0: e = 1; default: e = 0; endcase
  end
  initial begin
    tiny = $bitstoreal(64'd1);
    case (one) 1: f = 1; default: f = 0; endcase
    case (one) '1: g = 1; default: g = 0; endcase
    case ('1) one: h = 1; default: h = 0; endcase
    case (tiny) '1: k = 1; 1: k = 2; default: k = 0; endcase
    case (i) 2.0: m = 1; default: m = 0; endcase
    $display("T|interpreted %0d %0d %0d %0d %0d", f, g, h, k, m);
    #1 one = 2.0;
    #1 $display("T|compiled %0d %0d %0d %0d %0d", a, b, c, d, e);
    one = 1.0;
    #1 $display("T|compiled %0d %0d %0d %0d %0d", a, b, c, d, e);
  end
endmodule
