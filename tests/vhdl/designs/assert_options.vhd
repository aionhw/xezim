-- GHDL's --ieee-asserts and --assert-level (assert_options.args): IEEE
-- assertions hidden at time 0, and an assertion of severity error stops.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity assert_options is end entity;
architecture a of assert_options is
  signal u : unsigned(3 downto 0);
begin
  process
    variable i : integer;
  begin
    i := to_integer(u);
    report "i=" & integer'image(i);
    wait for 1 ns;
    i := to_integer(u);
    assert false report "a warning" severity warning;
    assert false report "an error" severity error;
    report "after the error";
    wait;
  end process;
end architecture;
