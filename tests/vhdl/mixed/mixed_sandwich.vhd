-- The middle of a SystemVerilog-VHDL-SystemVerilog sandwich
-- (mixed_sandwich.sv).
library ieee;
use ieee.std_logic_1164.all;

entity mid is
  port (
    a, b : in std_logic_vector(7 downto 0);
    y    : out std_logic_vector(7 downto 0);
    gt   : out std_logic);
end entity;

architecture rtl of mid is
  component leaf is
    port (x, z : in std_logic_vector(7 downto 0); o : out std_logic_vector(7 downto 0));
  end component;
  signal o : std_logic_vector(7 downto 0);
begin
  u_leaf : leaf port map (x => a, z => b, o => o);
  y <= not o;
  -- The predefined ordering of arrays of std_ulogic (§9.2.3).
  gt <= '1' when a > b else '0';
end architecture;
