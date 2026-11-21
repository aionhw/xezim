// Functions that write a bit of their own vector local through a signed index,
// called by continuous assigns (IEEE 1800-2023 section 11.5.1).
//
// The loop variable is an `integer` (or `int`), which is signed, and the step is
// `i++`, so the function is pure. Each function writes only its own locals and
// reads only its own formal. Every function is called by one continuous assign;
// the input changes TOGGLES times and each result is compared with a closed-form
// model, printing one PASS or FAIL line per function.
//
//   xezim --no-cache --simulate -s top signed_index_writes.sv
//   xezim --no-cache --simulate -s top -DTOGGLES=5000 signed_index_writes.sv
`ifndef TOGGLES
 `define TOGGLES 200
`endif
module top;
  reg [7:0] a;

  // r[i] with a signed index, in range
  function automatic [7:0] reverse_bits(input [7:0] x);
    integer i; reg [7:0] r;
    begin r = 8'h00; for (i = 0; i < 8; i++) r[i] = x[7 - i]; reverse_bits = r; end
  endfunction

  // the same with an `int` index
  function automatic [7:0] copy_bits(input [7:0] x);
    int i; reg [7:0] r;
    begin r = 8'h00; for (i = 0; i < 8; i++) r[i] = x[i]; copy_bits = r; end
  endfunction

  // r[i - 1] is r[-1] on the first pass: a bit-select below 0 writes nothing
  function automatic [7:0] shift_in(input [7:0] x);
    integer i; reg [7:0] r;
    begin r = 8'h00; for (i = 0; i < 8; i++) r[i - 1] = x[i]; shift_in = r; end
  endfunction

  // bits 8 to 11 of an 8-bit local do not exist: such a write changes nothing
  function automatic [7:0] past_top(input [7:0] x);
    integer i; reg [7:0] r;
    begin r = 8'h00; for (i = 4; i < 12; i++) r[i] = x[i - 4]; past_top = r; end
  endfunction

  // the same function of an unsigned loop variable: control
  function automatic [7:0] copy_bits_unsigned(input [7:0] x);
    reg [3:0] k; reg [7:0] r;
    begin r = 8'h00; for (k = 0; k < 8; k++) r[k[2:0]] = x[k[2:0]]; copy_bits_unsigned = r; end
  endfunction

  function automatic [7:0] place(input logic signed [2:0] i);
    logic [7:0] r;
    r = 0;
    r[i] = 1'b1;
    return r;
  endfunction

  logic signed [2:0] narrow;
  wire [7:0] placed = place(narrow);
  wire [7:0] w_rev = reverse_bits(a);
  wire [7:0] w_copy = copy_bits(a);
  wire [7:0] w_shift = shift_in(a);
  wire [7:0] w_top = past_top(a);
  wire [7:0] w_unsigned = copy_bits_unsigned(a);

  integer n_rev, n_copy, n_shift, n_top, n_unsigned;
  integer k;
  initial begin
    narrow = -1;
    #1;
    if (placed !== 0) $fatal(1, "narrow negative index set bit 7");
    narrow = 3'bx01;
    #1;
    if (placed !== 0) $fatal(1, "unknown index wrote to local");
    narrow = 3;
    #1;
    if (placed !== 8'h08) $fatal(1, "positive narrow index missed bit 3");
    $display("PASS narrow signed and unknown indices");
    n_rev = 0; n_copy = 0; n_shift = 0; n_top = 0; n_unsigned = 0;
    for (k = 0; k < `TOGGLES; k = k + 1) begin
      a = k[7:0] * 8'd37 + 8'd11;
      #1;
      if (w_rev !== {a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]}) n_rev = n_rev + 1;
      if (w_copy !== a) n_copy = n_copy + 1;
      if (w_shift !== {1'b0, a[7:1]}) n_shift = n_shift + 1;
      if (w_top !== {a[3:0], 4'h0}) n_top = n_top + 1;
      if (w_unsigned !== a) n_unsigned = n_unsigned + 1;
    end
    $display("%s reverse_bits (integer index)", n_rev == 0 ? "PASS" : "FAIL");
    $display("%s copy_bits (int index)", n_copy == 0 ? "PASS" : "FAIL");
    $display("%s shift_in (index -1 writes nothing)", n_shift == 0 ? "PASS" : "FAIL");
    $display("%s past_top (index above the top writes nothing)", n_top == 0 ? "PASS" : "FAIL");
    $display("%s copy_bits_unsigned (control, unsigned index)", n_unsigned == 0 ? "PASS" : "FAIL");
    $finish;
  end
endmodule
