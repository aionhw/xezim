// IEEE 1800-2023 §7.2.1, §10.9.2: P is a packed structure of two int
// members, so '{2{7}} sets both to 7. The parent module's array typedef
// arr_t has the name of the child's scalar typedef; §7.2.1 rules out
// unpacked array members in a packed structure.
`timescale 1ns/1ps
module child;
  typedef int arr_t;
  typedef struct packed { arr_t a; arr_t b; } P;
  localparam P K = '{2{7}};
  initial begin
    #1;
    if (K.a === 7 && K.b === 7) $display("PASS packed constant beside an outer array typedef");
    else $display("FAIL packed constant beside an outer array typedef %0d %0d", K.a, K.b);
  end
endmodule
module top;
  typedef int arr_t[2];
  child u();
  initial #2 $finish;
endmodule
