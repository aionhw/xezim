package PKG;
  bit [7:0] pa, pb;
endpackage

class E;
  bit [7:0] m, n;                  // instance members, read as e.m / e.n
  static bit [7:0] ta, tb, tc;     // class-static, read as E::ta

  function new();
    m = 200; n = 100;
  endfunction

  // Check 4: the same owner-qualified pair from inside a class scope, where the
  // receiver stack IS populated. The fix must not disturb that path -- it
  // already sized correctly via the live receiver, and it must keep wrapping at
  // the 8-bit context.
  function bit inner8();
    bit [7:0] s8;
    s8 = 44;
    return s8 === E::ta + E::tb;
  endfunction
endclass

module top;
  int fails;
  logic [8:0] s9;
  logic [8:0] s350;
  bit [7:0]   s8;
  E e;

  function automatic void chk(int r, bit g, string nm, ref int n);
    if (!g) begin n++; $display("  FAIL %-4d %s", r, nm); end
    else    begin      $display("  ok   %-4d %s", r, nm); end
  endfunction

  initial begin
    fails = 0;
    s9 = 300; s8 = 44;
    s350 = 350;
    e = new;
    E::ta = 200; E::tb = 100; E::tc = 50;
    PKG::pa = 200; PKG::pb = 100;

    // -- the bug: an owner-qualified operand in a 9-bit comparison context --
    chk(1, s9 === E::ta + E::tb,            "E::ta + E::tb against a 9-bit 300", fails);
    chk(2, s350 === E::ta + E::tb + E::tc,  "three-term static sum against a 9-bit 350", fails);

    // -- controls: must hold with or without the fix --
    chk(3, s8 === E::ta + E::tb,             "control: 8-bit context still wraps to 44", fails);
    chk(4, e.inner8(),                       "control: inside a method, 8-bit still wraps", fails);
    chk(5, s9 === PKG::pa + PKG::pb,         "control: package-qualified pair untouched", fails);
    chk(6, s9 === e.m + e.n,                 "control: instance members through a handle", fails);

    if (fails == 0) $display("TEST_PASS");
    else            $display("TEST_FAIL count=%0d", fails);
    $finish;
  end
endmodule