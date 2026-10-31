// IEEE 1800-2017 §21.2.1 / §35.5.5: a call to an import declared `string`
// is a string expression, so as an unformatted display-family argument it
// prints as text, not as the integer value of its bytes.
module top;
  import "DPI-C" function string srd_msg(int i);
  string a, b, c;
  logic clk = 1'b0;
  int fails = 0;
  bit seen = 0;
  initial begin
    $display(srd_msg(0));
    a = $sformatf(srd_msg(0));
    b = $sformatf("x", srd_msg(1));
    $swrite(c, srd_msg(0), "|", srd_msg(1));
    if (a != "hello from C") begin fails++; $display("FAIL sformatf: %s", a); end
    if (b != "xsecond message") begin fails++; $display("FAIL sformatf arg: %s", b); end
    if (c != "hello from C|second message") begin fails++; $display("FAIL swrite: %s", c); end
    #1 clk = 1'b1;
    #1 if (fails == 0 && seen) $display("TEST_PASS");
  end
  always @(posedge clk) begin
    string d;
    d = $sformatf(srd_msg(1));
    if (d != "second message") begin fails++; $display("FAIL always: %s", d); end
    seen = 1;
  end
endmodule
