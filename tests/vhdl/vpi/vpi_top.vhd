-- A VHDL top level driven through VPI (a cocotb testbench): its ports are
-- visible by name. An input keeps its default until VPI writes it; a port
-- that cannot cross (here an `inout integer`) is left out with a warning.
library ieee;
use ieee.std_logic_1164.all;

entity vpi_top is
  port (
    en : in std_logic := '1';
    d : in std_logic_vector(7 downto 0);
    io : inout integer;
    q : out std_logic_vector(7 downto 0));
end entity;

architecture a of vpi_top is
begin
  q <= d when en = '1' else (others => '0');
  process
  begin
    wait for 500 ps;
    report "en=" & std_logic'image(en) & " d=" & to_string(d);
    wait for 1 ns;
    report "en=" & std_logic'image(en) & " d=" & to_string(d) & " q=" & to_string(q);
    wait;
  end process;
end architecture;
