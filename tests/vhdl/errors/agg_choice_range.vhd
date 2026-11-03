library ieee;
use ieee.std_logic_1164.all;
entity agg_choice_range is
end entity;
architecture a of agg_choice_range is
begin
  process
    variable dst : std_logic_vector(7 downto 0);
    variable s : std_logic_vector(3 downto 0) := "1111";
  begin
    dst := (9 downto 6 => s, others => '0');
    wait;
  end process;
end architecture;
