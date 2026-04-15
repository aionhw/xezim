`include "../common/svtest_defs.svh"

package math_pkg;
  parameter int PKG_CONST = 7;
  function automatic int muladd(input int a, input int b, input int c);
    muladd = (a * b) + c;
  endfunction
endpackage

module test_packages_imports;
  `SVTEST_INIT
  import math_pkg::*;

  int r;

  initial begin
    r = muladd(3, 4, PKG_CONST);
    `SVTEST_CHECK(r == 19, "package import or package function failed")

    `SVTEST_PASSFAIL
  end
endmodule
