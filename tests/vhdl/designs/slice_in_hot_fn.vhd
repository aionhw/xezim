-- A non-static slice of a downto vector inside a function that returns a
-- scalar (§8.5), lowered to an unpacked struct (bounds plus a dynamic
-- array). slice_in_hot_fn.env compiles the function on its first call; the
-- array must still arrive intact (#884). "10110001" has four '1's.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity slice_in_hot_fn is end entity;
architecture a of slice_in_hot_fn is
  function sum_bits(u : std_logic_vector) return integer is
    variable sum : integer := 0;
  begin
    for i in u'range loop
      sum := sum + to_integer(unsigned(u(i downto i)));
    end loop;
    return sum;
  end function;
begin
  process
    variable s : integer := 0;
    constant v : std_logic_vector(7 downto 0) := "10110001";
  begin
    for n in 1 to 3 loop
      s := sum_bits(v);
    end loop;
    report integer'image(s);
    wait;
  end process;
end architecture;
