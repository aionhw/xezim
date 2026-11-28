`timescale 1ns/100ps
module top;
  real t;
  int rc;
  initial begin
    $timeformat(-3, 2, " ms", 10);
    // Match the uppercase spelling in Table 21-1 and the scanner's existing
    // unlimited interpretation of an explicit zero width (§21.3.4.3).
    rc = $sscanf("10.345", "%T", t);
    if (rc != 1 || t != 10350000.0) $fatal(1, "uppercase T");
    rc = $sscanf("10.345", "%0t", t);
    if (rc != 1 || t != 10350000.0) $fatal(1, "zero field width");
    $display("PASS");
  end
endmodule
