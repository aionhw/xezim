-- A record signal with an array field, instantiated from SystemVerilog.
-- The field's default bounds have to survive elaboration: an assignment
-- pattern used to drop the array, and the first read then failed its
-- bound check. After a clock the elements written below are visible.
library ieee;
use ieee.std_logic_1164.all;

entity rec_field is
  port (
    clk : in std_logic;
    q : out std_logic_vector(7 downto 0)
  );
end entity;

architecture rtl of rec_field is
  type grid is array (0 to 1, 0 to 3) of std_logic;
  type rec_t is record
    flag : std_logic;
    g : grid;
  end record;
  signal s : rec_t;
begin
  q <= s.g(0, 0) & s.g(0, 1) & s.g(0, 2) & s.g(0, 3)
     & s.g(1, 0) & s.g(1, 1) & s.g(1, 2) & s.g(1, 3);
  process (clk)
  begin
    if rising_edge(clk) then
      s.flag <= '1';
      s.g(0, 0) <= '1';
      s.g(0, 1) <= '0';
      s.g(0, 2) <= '1';
      s.g(0, 3) <= '0';
      s.g(1, 0) <= '0';
      s.g(1, 1) <= '1';
      s.g(1, 2) <= '0';
      s.g(1, 3) <= '1';
    end if;
  end process;
end architecture;
