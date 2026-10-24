library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity numeric_div is end entity;
architecture sim of numeric_div is
begin
  process
    variable u : unsigned(7 downto 0);
  begin
    u := to_unsigned(200, 8);
    report "div=" & to_string(u / 7) & " " & integer'image(to_integer(u / 7));
    report "divu=" & to_string(u / to_unsigned(7, 8));
    report "mod=" & to_string(u mod 7) & " rem=" & to_string(u rem 7);
    report "s/=" & integer'image(to_integer(to_signed(-100, 8) / 7)) & " " & integer'image(to_integer(to_signed(-100, 8) rem 7)) & " " & integer'image(to_integer(to_signed(-100, 8) mod 7));
    wait;
  end process;
end architecture;
