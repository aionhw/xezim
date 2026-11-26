library ieee;
use ieee.std_logic_1164.all;
entity packed_record_index is end;
architecture test of packed_record_index is
  type row is record value : std_logic_vector(7 downto 0); end record;
  type rows is array (2 downto -1) of row;
  signal r : rows;
begin
  process
    variable i : integer := 3;
  begin
    r(i).value <= x"A5";
    report "unexpected continuation" severity failure;
    wait;
  end process;
end;
