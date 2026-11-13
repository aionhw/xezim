// IEEE 1800-2023 §6.20.3 / §20.7: deferred type contexts are instance-specific.
module typed #(parameter type T=logic [7:4], parameter int B=4, L=7, R=4)
              (output int ready);
  initial begin
    T value;
    value='1;
    if ($bits(T)!=B || $bits(value)!=B || $size(T)!=B ||
        $left(T)!=L || $right(T)!=R || value !== '1)
      $fatal(1,"type context: bits=%0d left=%0d right=%0d",$bits(T),$left(T),$right(T));
    ready=1;
  end
endmodule
module dependent #(parameter int W=8, parameter type T=logic [W-1:0])
                  (output int ready);
  T value;
  initial begin
    value='1;
    if ($bits(T)!=W || $bits(value)!=W || $left(T)!=W-1 || $right(T)!=0 || value !== '1)
      $fatal(1,"dependent type width: expected=%0d type=%0d value=%0d",W,$bits(T),$bits(value));
    ready=1;
  end
endmodule
module ordinary(output int ready);
  typedef logic [2:0] T;
  initial begin
    T value;
    value='1;
    if ($bits(T)!=3 || $bits(value)!=3 || value!=7) $fatal(1,"stale type context");
    ready=1;
  end
endmodule
module top;
  int ready[8];
  typed a(ready[0]);
  typed #(.T(logic [0:3]), .L(0), .R(3)) b(ready[1]);
  ordinary c(ready[2]);
  typed #(.T(logic [15:8]), .B(8), .L(15), .R(8)) d(ready[3]);
  typed e(ready[4]);
  ordinary f(ready[5]);
  dependent g(ready[6]);
  dependent #(.W(9)) h(ready[7]);
  initial begin
    #1;
    foreach(ready[i]) if(ready[i]!=1) $fatal(1,"missing initial %0d",i);
    $display("PASS type contexts");
    $finish;
  end
endmodule
