-- IEEE fixed_pkg and float_pkg (VHDL-2008 §16.10, §16.11): fixed-point
-- arithmetic with result sizing, resize with saturation and rounding,
-- conversions and comparisons, and float32 arithmetic.
library ieee;
use ieee.std_logic_1164.all;
use ieee.fixed_pkg.all;
use ieee.float_pkg.all;

entity fixed_float is
end entity;

architecture sim of fixed_float is
begin
  process
    variable x : sfixed(3 downto -4) := to_sfixed(2.75, 3, -4);
    variable y : sfixed(3 downto -4) := to_sfixed(-1.5, 3, -4);
    variable s : sfixed(1 downto -2);
    variable f : float32 := to_float(1.5);
    variable g : float32 := to_float(-0.25);
  begin
    report "x+y=" & to_string(x + y) & " x*y=" & to_string(x * y)
      & " " & real'image(to_real(x * y));
    s := resize(x, s'high, s'low);
    report "saturate=" & to_string(s) & " " & real'image(to_real(s));
    s := resize(to_sfixed(0.625, 3, -4), s'high, s'low);
    report "round=" & to_string(s) & " " & real'image(to_real(s));
    report "cmp " & boolean'image(x > y) & " " & boolean'image(x = to_sfixed(2.75, 3, -4))
      & " int=" & integer'image(to_integer(x));
    report "f+g=" & to_string(f + g) & " f*g=" & real'image(to_real(f * g))
      & " f<g=" & boolean'image(f < g);
    wait;
  end process;
end architecture;
