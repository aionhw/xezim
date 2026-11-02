-- A range choice in an array aggregate may give an array the length of
-- the choice (IEEE 1076-2008 §9.3.3.3): a slice, a bit string, or a hex
-- string. A scalar value is still repeated across the choice. The
-- leftmost element of the value goes to the left index of the choice.
library ieee;
use ieee.std_logic_1164.all;
entity agg_slice is
end entity;
architecture a of agg_slice is
  type int_arr is array (natural range <>) of integer;
  function slv(v : std_logic_vector) return string is
    variable s : string(1 to v'length);
    variable k : positive := 1;
  begin
    for i in v'range loop
      s(k) := std_logic'image(v(i))(2);
      k := k + 1;
    end loop;
    return s;
  end function;
  function bits(v : bit_vector) return string is
    variable s : string(1 to v'length);
    variable k : positive := 1;
  begin
    for i in v'range loop
      s(k) := bit'image(v(i))(2);
      k := k + 1;
    end loop;
    return s;
  end function;
begin
  process
    variable dst : std_logic_vector(7 downto 0);
    variable src : std_logic_vector(7 downto 0) := "10110001";
    variable up : std_logic_vector(0 to 7) := "11001010";
    variable addr : std_logic_vector(2 downto 0);
    variable paddr : std_logic_vector(2 downto 0) := "101";
    variable bd : bit_vector(7 downto 0);
    variable bb : bit_vector(7 downto 0) := "10110001";
    variable ia : int_arr(7 downto 0);
    variable ib : int_arr(7 downto 0) := (1, 2, 3, 4, 5, 6, 7, 8);
  begin
    dst := (7 downto 4 => src(7 downto 4), others => '0');
    report "slice " & slv(dst);
    dst := (7 downto 4 => "1111", 3 downto 0 => "0000");
    report "str " & slv(dst);
    dst := (7 downto 4 => x"A", 3 downto 0 => x"5");
    report "hex " & slv(dst);
    dst := (7 downto 4 => '1', others => '0');
    report "bit " & slv(dst);
    addr := (2 downto 2 => paddr(2 downto 2), others => '0');
    report "addr " & slv(addr);
    dst := (7 downto 4 => up(0 to 3), others => '0');
    report "rev " & slv(dst);
    bd := (7 downto 4 => bb(7 downto 4), others => '0');
    report "bv " & bits(bd);
    ia := (7 downto 4 => ib(7 downto 4), others => 0);
    report "int " & integer'image(ia(7)) & integer'image(ia(4)) & integer'image(ia(3));
    wait;
  end process;
end architecture;
