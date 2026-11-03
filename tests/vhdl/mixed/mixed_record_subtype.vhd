-- A signal of a record subtype that constrains an unconstrained array field
-- (IEEE 1076-2008 §5.3.3), in an entity instantiated from SystemVerilog:
-- the field is written on a clock edge, whole and by a slice, and an
-- output follows it.
library ieee;
use ieee.std_logic_1164.all;

entity rec_sub is
  port (clk : in std_logic; q : out std_logic_vector(7 downto 0));
end entity;

architecture rtl of rec_sub is
  type r is record
    d : std_logic_vector;
  end record;
  signal s : r(d(7 downto 0));
  signal n : integer := 0;
begin
  q <= s.d;
  process (clk)
  begin
    if rising_edge(clk) then
      n <= n + 1;
      if n = 0 then
        s.d <= x"5C";
      else
        s.d(7 downto 4) <= x"9";
      end if;
    end if;
  end process;
end architecture;
