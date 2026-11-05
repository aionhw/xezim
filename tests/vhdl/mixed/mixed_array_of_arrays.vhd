-- An array of arrays of vectors connected to SystemVerilog in both
-- directions as one packed logic vector (see mixed_array_of_arrays.lrm):
-- the left index of each level and the left bit first. Elements of the
-- input are read one by one, so the input layout is checked on its own.
library ieee;
use ieee.std_logic_1164.all;

package aoa_pkg is
  type byte_arr is array (0 to 1) of std_logic_vector(7 downto 0);
  type arr2 is array (0 to 2) of byte_arr;
end package;

library ieee;
use ieee.std_logic_1164.all;
use work.aoa_pkg.all;

entity aoa_io is
  port (
    d : in arr2;
    q : out arr2;
    e0, e3, e5 : out std_logic_vector(7 downto 0));
end entity;

architecture rtl of aoa_io is
begin
  q <= d;
  e0 <= d(0)(0);
  e3 <= d(1)(1);
  e5 <= d(2)(1);
end architecture;
