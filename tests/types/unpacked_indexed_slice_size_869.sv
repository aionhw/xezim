// Every statement pairs two unpacked arrays of the same size (IEEE 1800-2023 7.4.5, 7.6, 11.5.1):
// the file is legal and should compile and print PASS. From the shapes of #869 (check_shapes.sh --emit legal).
module t;
  logic [7:0] a [7:0];       // 8 elements, descending
  logic [7:0] b [5:0];       // 6
  logic [7:0] b3 [2:0];      // 3
  logic [7:0] b5 [4:0];      // 5
  logic [7:0] big [255:0];   // 256
  logic [7:0] c [3:0][5:0];  // 4 rows of 6
  logic [7:0] c2 [1:0][5:0]; // 2 rows of 6
  logic [7:0] d [0:7];       // 8 elements, ascending
  logic [7:0] e6 [0:5];      // 6, ascending
  logic [7:0] ca6 [5:0];     // targets of the continuous assignments
  logic [7:0] ca5 [4:0];
  logic [7:0] ca8 [7:0];
  logic [7:0] cc [3:0][5:0];
  int dyn [];                // a dynamic array
  int x4 [3:0];
  int x5 [4:0];
  int i, j;
  localparam int W = 6;
  task automatic take6(input logic [7:0] x [5:0]);
  endtask
  assign ca6 = big[58 +: 6]; // L26
  assign cc[1] = big[64*1 + 58 +: 6]; // L27
  assign ca8[0 +: 6] = b; // L28
  logic [7:0] init6 [5:0] = big[58 +: 6]; // L29
  initial begin
    a[0 +: 6] = b; // L01
    a[1 +: 6] = b; // L02
    a[i +: 6] = b; // L03
    a[1 +: W] = b; // L04
    a[7 -: 6] = b; // L05
    a[j -: 6] = b; // L06
    b = a[0 +: 6]; // L07
    b = a[2 +: 6]; // L08
    b = a[i +: 6]; // L09
    b = a[7 -: 6]; // L10
    b = big[122 +: 6]; // L11
    b = big[64*1 + 58 +: 6]; // L12
    b = big[250 +: 6]; // L13
    big[244 +: 6] = b; // L14
    c[0] = big[58 +: 6]; // L15
    c[3] = big[250 +: 6]; // L16
    c[0 +: 2] = c2; // L17
    e6 = d[0 +: 6]; // L18
    e6 = d[2 +: 6]; // L19
    d[1 +: 6] = e6; // L20
    a[0 +: 6] = big[250 +: 6]; // L21
    big[244 +: 6] <= big[250 +: 6]; // L22
    b = a[6:1]; // L23
    take6(a[0 +: 6]); // L24
    take6(big[58 +: 6]); // L25
    x4 = dyn[0 +: 4]; // L30
    dyn[0 +: 4] = x4; // L31
    $display("PASS the legal statements were accepted");
  end
endmodule
