-- A VHDL testbench binding a SystemVerilog module to a component whose
-- port is an array of vectors, with a slice of a larger array as the
-- actual: only the slice's elements cross, the left one in the most
-- significant bits (see mixed_slice_actual.lrm).
library ieee;
use ieee.std_logic_1164.all;

entity mixed_slice_actual is
end entity;

architecture a of mixed_slice_actual is
  type slv_arr is array (natural range <>) of std_logic_vector(7 downto 0);
  signal big : slv_arr(0 to 3) := (x"10", x"21", x"32", x"43");
  component sink
    port (d : in slv_arr(0 to 1));
  end component;
begin
  u : sink port map (d => big(1 to 2));
  process
  begin
    wait for 2 ns;
    big(2) <= x"A7";
    wait;
  end process;
end architecture;
