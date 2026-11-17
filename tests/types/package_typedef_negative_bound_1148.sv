// IEEE 1800-2023 §26.3, §6.18, §7.4.2: a package typedef's range names the
// package's own constants, so `pk::A` has the range [-1:1] wherever it is
// used, with or without an import, even when another module declares a
// parameter of the same name. A typed constant has its declared type's
// value there (§6.20.2): `byte` 2147483646 is -2.
package pk;
  localparam int LO = -1;
  localparam byte B = 2147483646;
  typedef int A[LO:1];
  typedef struct { int tag; int a[LO:1]; } S;
  typedef int AB[B:B+1];
endpackage
module top;
  pk::A v;
  pk::S s;
  pk::AB b;
  initial begin
    v[-1] = 5;
    s.a[-1] = 6;
    b[-2] = 7;
    #1;
    $display("T|array %0d %0d %0d", $left(v), $size(v), v[-1]);
    $display("T|member %0d %0d %0d", $left(s.a), $size(s.a), s.a[-1]);
    $display("T|typed %0d %0d %0d", $left(b), $right(b), b[-2]);
    $finish;
  end
endmodule
module other #(parameter int LO = -1)();
endmodule
