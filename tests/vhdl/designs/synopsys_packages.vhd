library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_arith.all;
use ieee.std_logic_unsigned.all;
use ieee.std_logic_misc.all;
entity synopsys_packages is end entity;
architecture a of synopsys_packages is
  signal x : std_logic_vector(7 downto 0) := x"3c";
  signal y : std_logic_vector(7 downto 0) := x"05";
begin
  process
    variable s : std_logic_vector(7 downto 0);
    variable u : unsigned(7 downto 0);
    variable i : integer;
  begin
    s := x + y;
    report "x+y=" & to_hstring(s);
    s := x - 1;
    report "x-1=" & to_hstring(s);
    report "x>y " & boolean'image(x > y) & " x=60 " & boolean'image(x = 60);
    i := conv_integer(x);
    report "conv_integer=" & integer'image(i);
    u := conv_unsigned(200, 8);
    report "conv_unsigned=" & to_hstring(std_logic_vector(u));
    s := conv_std_logic_vector(-3, 8);
    report "conv_slv=" & to_hstring(s);
    report "or_reduce=" & std_logic'image(or_reduce(x)) & " and_reduce=" & std_logic'image(and_reduce(x));
    s := shl(x, "10");
    report "shl=" & to_hstring(s);
    report "ext=" & to_hstring(ext(y, 12)) & " sxt=" & to_hstring(sxt(x"f0", 12));
    s := x * y(3 downto 0);
    report "mul=" & to_hstring(s);
    wait;
  end process;
end architecture;
