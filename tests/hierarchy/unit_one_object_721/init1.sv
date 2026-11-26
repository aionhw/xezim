// file 1 of 2: declarations used by file 2 and initialized from file-2-independent values
int f1_a = 3;
int f1_b = f1_a * 7;
function automatic int twice(int v); return 2 * v; endfunction
int f1_c = twice(f1_b);
string f1_s = $sformatf("s%0d", f1_a);
module m1; initial begin #1; f1_a = 100; end endmodule
