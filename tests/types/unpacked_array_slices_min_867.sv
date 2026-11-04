// Minimal reproducer: run-time slices of unpacked arrays (IEEE 1800-2023 7.4.5, 7.6, 10.3, 13.5;
// IEEE 1800-2017 7.4.6, 7.6, 10.3, 13.5). The width of every indexed select is the parameter W, so
// every assignment pairs arrays of the same size. Each check prints PASS or FAIL.
module t;
  localparam int W = 6;
  logic [7:0] a [7:0];        // 8 elements, a[7] is the leftmost
  logic [7:0] b [5:0];
  logic [7:0] w [63:0];
  logic [7:0] m [1:0][5:0];   // two rows of 6 elements
  logic [7:0] cw [5:0];
  int errs;

  assign cw = w[58+:W];       // continuous assignment from a slice

  function automatic logic [7:0] leftmost(input logic [7:0] x [5:0]);
    return x[5];
  endfunction

  task automatic init();
    for (int k = 0; k < 8; k++)  a[k] = 8'h10 + 8'(k);
    for (int k = 0; k < 6; k++)  b[k] = 8'hA0 + 8'(k);
    for (int k = 0; k < 64; k++) w[k] = 8'(k);
    for (int r = 0; r < 2; r++) for (int k = 0; k < 6; k++) m[r][k] = 8'hC0 + 8'(6 * r + k);
  endtask

  task automatic check(string name, bit ok);
    if (ok) $display("PASS %s", name);
    else begin errs++; $display("FAIL %s", name); end
  endtask

  initial begin
    init(); #1;
    // 1. a slice as the source: b[k] must become a[2+k]
    b = a[7-:W];
    check("1  b = a[7-:W]", b[0] === 8'h12 && b[5] === 8'h17);
    // 2. the same slice written as a constant range
    init(); b = a[7:2];
    check("2  b = a[7:2]", b[0] === 8'h12 && b[5] === 8'h17);
    // 3. a slice as the target: a[k] must become b[k] for k = 0..5; a[6] and a[7] stay
    init(); a[0+:W] = b;
    check("3  a[0+:W] = b", a[0] === 8'hA0 && a[5] === 8'hA5 && a[6] === 8'h16 && a[7] === 8'h17);
    // 4. a row of a two-dimensional array takes a slice
    init(); m[1] = w[58+:W];
    check("4  m[1] = w[58+:W]", m[1][0] === 8'd58 && m[1][5] === 8'd63);
    // 5. a nonblocking assignment to a slice
    init(); a[0+:W] <= b; #1;
    check("5  a[0+:W] <= b", a[0] === 8'hA0 && a[5] === 8'hA5 && a[6] === 8'h16);
    // 6. a slice as the actual of an input argument: x[5] is the leftmost element, w[63]
    init();
    check("6  leftmost(w[58+:W])", leftmost(w[58+:W]) === 8'd63);
    // 7. a continuous assignment from a slice
    check("7  assign cw = w[58+:W]", cw[0] === 8'd58 && cw[5] === 8'd63);

    if (errs == 0) $display("ALL PASS"); else $display("FAILED %0d", errs);
    $finish;
  end
endmodule
