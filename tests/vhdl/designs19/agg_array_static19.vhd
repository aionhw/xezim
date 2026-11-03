-- An array value across a range choice (IEEE 1076-2019 §9.3.3.3) in a
-- constant and in a generic's default, which are evaluated statically, and
-- across a choice whose bounds are known only at run time (evaluated once).
-- GHDL 4.1 stops on these with an internal error, hence nvc's goldens.
library ieee;
use ieee.std_logic_1164.all;
use std.textio.all;
entity agg_array_static19 is
  generic (G : std_logic_vector(7 downto 0) := (7 downto 4 => "1011", others => '0'));
end entity;
architecture a of agg_array_static19 is
  type int_arr is array (natural range <>) of integer;
  constant C : std_logic_vector(7 downto 0) := (7 downto 4 => "1011", others => '0');
  constant CB : int_arr(0 to 1) := (5, 6);
  constant CI : int_arr(0 to 3) := (0 to 1 => CB, others => 9);
  constant CP : std_logic_vector(0 to 5) := ("10", "01", "11");
begin
  process
    variable l : line;
    variable dst : std_logic_vector(7 downto 0);
    variable src : std_logic_vector(7 downto 0) := "11010110";
    variable ia, ib : int_arr(0 to 7);
    variable k : integer := 2;
  begin
    write(l, "C " & to_string(C) & " G " & to_string(G) & " CP " & to_string(CP));
    writeline(output, l);
    write(l, "CI " & integer'image(CI(0)) & " " & integer'image(CI(1)) & " " &
      integer'image(CI(2)) & " " & integer'image(CI(3)));
    writeline(output, l);
    ib := (10, 11, 12, 13, 14, 15, 16, 17);
    dst := (others => '0');
    dst(5 downto 2) := (k + 3 downto k => src(5 downto 2));
    write(l, "dyn " & to_string(dst));
    writeline(output, l);
    ia := (others => -1);
    ia(2 to 5) := (k to k + 3 => ib(0 to 3));
    write(l, "dyn int " & integer'image(ia(1)) & " " & integer'image(ia(2)) & " " &
      integer'image(ia(5)) & " " & integer'image(ia(6)));
    writeline(output, l);
    wait;
  end process;
end architecture;
