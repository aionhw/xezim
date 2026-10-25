-- Generic subprograms and their instantiation (§4.2.1, §4.4): generic
-- types, a generic subprogram formal called by the body, instances in an
-- architecture and in a package. GHDL 4.1 cannot call instantiated
-- generic subprograms, so the golden output is written by hand.
package gp is
  function swap generic (type t) parameter (a, b : t) return t;
  function swap_int is new swap generic map (t => integer);
end package;
package body gp is
  function swap generic (type t) parameter (a, b : t) return t is
  begin
    return b;
  end function;
end package body;

use work.gp.all;
entity generic_subprograms is end entity;
architecture a of generic_subprograms is
  procedure assign generic (type t) parameter (x : inout t; y : t) is
  begin
    x := y;
  end procedure;
  procedure assign_r is new assign generic map (t => real);
  function twice generic (type t; function f(x : t) return t) parameter (x : t) return t is
  begin
    return f(f(x));
  end function;
  function inc(x : integer) return integer is begin return x + 1; end function;
  function neg(x : bit) return bit is begin return not x; end function;
  function twice_inc is new twice generic map (t => integer, f => inc);
  function twice_neg is new twice generic map (t => bit, f => neg);
  function swap_bit is new swap generic map (t => bit);
begin
  process
    variable r : real := 1.0;
  begin
    report "swap_int=" & integer'image(swap_int(1, 2)) & " swap_bit=" & bit'image(swap_bit('0', '1'));
    assign_r(r, 2.5);
    report "r=" & real'image(r);
    report "twice_inc=" & integer'image(twice_inc(40)) & " twice_neg=" & bit'image(twice_neg('1'));
    wait;
  end process;
end architecture;
