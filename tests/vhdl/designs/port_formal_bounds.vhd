-- A port of a constrained two-dimensional array subtype whose bounds differ
-- from the actual's (IEEE 1076-2008 §6.5.6.3): inside the entity the
-- formal's own bounds index it, for reads and for writes alike; elements
-- match the actual's by position.
library ieee;
use ieee.std_logic_1164.all;
package pfb_pkg is
  type m2d is array (natural range <>, natural range <>) of std_logic;
  type slv_arr is array (natural range <>) of std_logic_vector(3 downto 0);
end package;
library ieee;
use ieee.std_logic_1164.all;
use work.pfb_pkg.all;
entity pfb_sub is
  port (p : out m2d(0 to 1, 1 to 4); v : out slv_arr(1 to 3));
end entity;
architecture a of pfb_sub is
begin
  process
  begin
    p <= (others => (others => '0'));
    v <= (others => "0000");
    wait for 1 ns;
    p(0, 1) <= '1';
    p(1, 4) <= 'H';
    v(1) <= "1010";
    v(3) <= "0101";
    wait for 1 ns;
    report "sub " & std_logic'image(p(0, 1)) & std_logic'image(p(0, 2)) & std_logic'image(p(1, 4)) &
      " " & to_string(v(1)) & " " & to_string(v(2)) & " " & to_string(v(3));
    wait;
  end process;
end architecture;
library ieee;
use ieee.std_logic_1164.all;
use work.pfb_pkg.all;
entity port_formal_bounds is
end entity;
architecture a of port_formal_bounds is
  signal s : m2d(0 to 1, 0 to 3);
  signal t : slv_arr(0 to 2);
begin
  u : entity work.pfb_sub port map (p => s, v => t);
  process
  begin
    wait for 3 ns;
    report "top " & std_logic'image(s(0, 0)) & std_logic'image(s(0, 1)) & std_logic'image(s(1, 3)) &
      " " & to_string(t(0)) & " " & to_string(t(1)) & " " & to_string(t(2));
    wait;
  end process;
end architecture;
