-- A record signal with an array field, instantiated from SystemVerilog,
-- initialised from a constant. The field has to survive elaboration with
-- its bounds (an assignment pattern used to drop the array, and the first
-- read failed its bound check): its initial value is visible at once, and
-- after a clock the elements and the flag written below.
library ieee;
use ieee.std_logic_1164.all;

entity rec_field is
  port (
    clk : in std_logic;
    q : out std_logic_vector(7 downto 0);
    f : out std_logic
  );
end entity;

architecture rtl of rec_field is
  type grid is array (0 to 1, 0 to 3) of std_logic;
  type rec_t is record
    flag : std_logic;
    g : grid;
  end record;
  constant INIT : rec_t := (flag => '0', g => ("1110", "0100"));
  signal s : rec_t := INIT;
begin
  q <= s.g(0, 0) & s.g(0, 1) & s.g(0, 2) & s.g(0, 3)
     & s.g(1, 0) & s.g(1, 1) & s.g(1, 2) & s.g(1, 3);
  f <= s.flag;
  process (clk)
  begin
    if rising_edge(clk) then
      s.flag <= '1';
      s.g(0, 0) <= '1';
      s.g(0, 1) <= '0';
      s.g(0, 2) <= '1';
      s.g(0, 3) <= '1';
      s.g(1, 0) <= '0';
      s.g(1, 1) <= '0';
      s.g(1, 2) <= '1';
      s.g(1, 3) <= '0';
    end if;
  end process;
end architecture;
