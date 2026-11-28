`timescale 1ns/100ps
module top;
  real t;
  realtime rt;
  shortreal sr;
  time stamp;
  logic [63:0] wide;
  int signed_value, rc;
  logic [7:0] narrow;
  bit [7:0] narrow_2state;
  string tail;

  initial begin
    // IEEE 1800-2023 §21.3.4.3 Table 21-7, §20.4.3.
    $timeformat(-3, 2, " ms", 10);
    rc = $sscanf("10.345", "%t", t);
    if (rc != 1 || t != 10350000.0) $fatal(1, "Table 21-7: rc=%0d t=%f", rc, t);
    rc = $sscanf("1.005", "%t", t);
    if (rc != 1 || t != 1010000.0) $fatal(1, "positive tie: rc=%0d t=%f", rc, t);
    rc = $sscanf("-10.345", "%t", t);
    if (rc != 1 || t != -10350000.0) $fatal(1, "negative tie: rc=%0d t=%f", rc, t);
    rc = $sscanf("+1.0345E+1", "%t", t);
    if (rc != 1 || t != 10350000.0) $fatal(1, "exponent: rc=%0d t=%f", rc, t);
    rc = $sscanf("10.345 ms", "%*t%s", tail);
    if (rc != 1 || tail != "ms") $fatal(1, "suppression: rc=%0d tail=%s", rc, tail);
    rc = $sscanf("10.345 ms", "%t%s", t, tail);
    if (rc != 2 || t != 10350000.0 || tail != "ms") $fatal(1, "suffix left unread");
    rc = $sscanf("  10.345", "%5t%s", t, tail);
    if (rc != 2 || t != 10340000.0 || tail != "5") $fatal(1, "field width");

    rc = $sscanf("1e308", "%t", t);
    if (rc != 1 || $realtobits(t) != 64'h7ff0000000000000) $fatal(1, "scaling overflow");
    rc = $sscanf("-1e500", "%t", t);
    if (rc != 1 || $realtobits(t) != 64'hfff0000000000000) $fatal(1, "input overflow");
    rc = $sscanf("1e-400", "%t", t);
    if (rc != 1 || $realtobits(t) != 0) $fatal(1, "input underflow");
    rc = $sscanf("-0.0", "%t", t);
    if (rc != 1 || $realtobits(t) != 64'h8000000000000000) $fatal(1, "negative zero");

    $timeformat(-12, 0, " ps", 10);
    rc = $sscanf("1500.5", "%t", t);
    if (rc != 1 || t != 1.501) $fatal(1, "whole picoseconds in a nanosecond scope");

    $timeformat(-9, 3, " ns", 10);
    rc = $sscanf("1.2345", "%t", rt);
    if (rc != 1 || rt != 1.235) $fatal(1, "realtime and sub-timeprecision digits");
    rc = $sscanf("1.25", "%t", sr);
    if (rc != 1 || sr != 1.25) $fatal(1, "shortreal");
    // §6.11.2, §6.12.1, §21.3.4.3: integer rounding and LSB transfer.
    rc = $sscanf("4294967296.5 4294967296.25 -2.5 260.5 260.5", "%t %t %t %t %t",
                 stamp, wide, signed_value, narrow, narrow_2state);
    if (rc != 5 || stamp !== 64'd4294967297 || wide !== 64'd4294967296 ||
        signed_value !== -3 || narrow !== 8'd5 || narrow_2state !== 8'd5)
      $fatal(1, "integral destinations: %0d %0d %0d %0d %0d %0d",
             rc, stamp, wide, signed_value, narrow, narrow_2state);

    t = -9.0;
    rc = $sscanf("+", "%t", t);
    if (rc != 0 || t != -9.0) $fatal(1, "bare sign");
    rc = $sscanf("1e", "%t", t);
    if (rc != 0 || t != -9.0) $fatal(1, "incomplete exponent");
    rc = $sscanf("abc", "%t", t);
    if (rc != 0 || t != -9.0) $fatal(1, "nonnumeric field");
    $sscanf("2.125", "%t", t);
    if (t != 2.125) $fatal(1, "task-form sscanf");
    $display("PASS");
  end
endmodule
