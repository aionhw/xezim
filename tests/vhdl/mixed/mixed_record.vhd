-- Records and three-dimensional arrays of records connected to
-- SystemVerilog as one packed logic vector: the left index and the first
-- field are the most significant bits, and each std_logic is one bit (see
-- mixed_record.lrm). Each direction is checked on its own: the input
-- record and an element of the input array are read field by field, and
-- every element of the output array has its own value.
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
    da : out std_logic_vector(7 downto 0);
    db, dc : out std_logic;
    ai : in reg_arr(0 to 1, 0 to 1, 1 downto 0);
    ae : out std_logic_vector(7 downto 0);
    af : out std_logic;
    a : out reg_arr(1 downto 0, 0 to 1, 0 to 1));
end entity;

architecture rtl of rec_io is
begin
  q <= d;
  da <= d.a.value;
  db <= d.b.value;
  dc <= d.c.value;
  ae <= ai(0, 1, 0).a.value;
  af <= ai(1, 0, 1).c.value;
  process (clk)
  begin
    if rising_edge(clk) then
      a(1, 0, 0) <= (a => (value => x"12"), b => (value => '1'), c => (value => '0'));
      a(1, 0, 1) <= (a => (value => x"34"), b => (value => '0'), c => (value => '1'));
      a(1, 1, 0) <= (a => (value => x"56"), b => (value => '1'), c => (value => '1'));
      a(1, 1, 1) <= (a => (value => x"78"), b => (value => '0'), c => (value => '0'));
      a(0, 0, 0) <= (a => (value => x"9A"), b => (value => '1'), c => (value => '0'));
      a(0, 0, 1) <= (a => (value => x"BC"), b => (value => '0'), c => (value => '1'));
      a(0, 1, 0) <= (a => (value => x"DE"), b => (value => '1'), c => (value => '1'));
      a(0, 1, 1) <= (a => (value => x"F1"), b => (value => '0'), c => (value => '0'));
    end if;
  end process;
end architecture;
