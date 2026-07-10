// Driver for tests/dpi/vpi_object_model.c — the classic VPI surface:
// `--vpi-lib` module loading, vlog_startup_routines, vpi_register_systf,
// and vpi_iterate/vpi_scan/vpi_get_str over the design hierarchy.
module tb;
  parameter int WIDTH = 8;

  logic             clk;
  logic [WIDTH-1:0] data;
  wire  [3:0]       w;
  int               mem [0:3];

  struct packed { logic [7:0] r; logic [7:0] g; logic [7:0] b; } px;

  sub u_sub (.clk(clk), .i(data[3:0]), .o(w));

  initial begin
    clk  = 0;
    data = 8'hA5;
    mem[0] = 32'hDEAD;
    mem[1] = 32'hBEEF;
    px = {8'hFF, 8'h77, 8'h33};
    #1;

    // Registered by the VPI module's vlog_startup_routines. It prints
    // OM_ERRORS: <n>, and writes px.r = 8'h11 on its way out.
    $vpi_om_check;
    #1;

    // The part-select write must have landed, and must not have disturbed
    // the sibling members.
    if (px.r !== 8'h11) $display("FAIL: part-select write (px.r = %h)", px.r);
    else if (px.g !== 8'h77 || px.b !== 8'h33) $display("FAIL: part-select write clobbered siblings");
    else $display("RESULT: PASSED");
    $finish;
  end
endmodule

module sub (input logic clk, input logic [3:0] i, output logic [3:0] o);
  assign o = ~i;
endmodule
