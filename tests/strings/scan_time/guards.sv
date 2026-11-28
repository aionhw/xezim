`timescale 1ns/100ps
module top;
  real f, e, g;
  int rc, d, h, o, b;
  byte c;
  string s;
  initial begin
    $timeformat(-3, 2, " ms", 10);
    // Table 21-7: other specifiers keep their own conversions.
    rc = $sscanf("12.125 2.5e-1 -3.125 -42 ff 17 101 word X", "%f %e %g %d %h %o %b %s %c",
                 f, e, g, d, h, o, b, s, c);
    if (rc != 9 || f != 12.125 || e != 0.25 || g != -3.125 || d != -42 ||
        h != 255 || o != 15 || b != 5 || s != "word" || c != "X") $fatal(1, "scan guard");
    // §21.2.1.3 and §20.4.3: output formatting still uses its own path.
    if ($sformatf("%0t", 1250000.0) != "1.25 ms") $fatal(1, "time display guard");
    $display("display=%0t", 1250000.0);
    $display("PASS");
  end
endmodule
