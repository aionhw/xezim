-- A record in both directions, and a three-dimensional array of records,
-- connected to SystemVerilog as one packed logic vector. The left index
-- and the first field are the most significant bits; each std_logic is
-- one bit.
library ieee;
use ieee.std_logic_1164.all;

package rec_pkg is
  type byte_t is record
    value : std_logic_vector(7 downto 0);
  end record;
  type bit_t is record
    value : std_logic;
  end record;
  type reg_t is record
    a : byte_t;
    b : bit_t;
    c : bit_t;
  end record;
  type reg_arr is array (natural range <>, natural range <>, natural range <>) of reg_t;
end package;

library ieee;
use ieee.std_logic_1164.all;
use work.rec_pkg.all;

entity rec_io is
  port (
    clk : in std_logic;
    d : in reg_t;
    q : out reg_t;
    a : out reg_arr(1 downto 0, 0 to 0, 0 to 1));
end entity;

architecture rtl of rec_io is
begin
  q <= d;
  process (clk)
  begin
    if rising_edge(clk) then
      a(1, 0, 0).a.value <= x"A5";
      a(1, 0, 0).b.value <= '1';
      a(1, 0, 0).c.value <= '0';
      a(1, 0, 1).a.value <= x"3C";
      a(1, 0, 1).b.value <= '0';
      a(1, 0, 1).c.value <= '1';
      a(0, 0, 0).a.value <= x"11";
      a(0, 0, 0).b.value <= '1';
      a(0, 0, 0).c.value <= '1';
      a(0, 0, 1).a.value <= x"00";
      a(0, 0, 1).b.value <= '0';
      a(0, 0, 1).c.value <= '0';
    end if;
  end process;
end architecture;
