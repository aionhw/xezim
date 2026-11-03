-- A signal of a record subtype that constrains an unconstrained array
-- field (IEEE 1076-2008 §5.3.3): the field is assigned, and a concurrent
-- assignment reading it follows it.
library ieee;
use ieee.std_logic_1164.all;
entity record_constrained_field is
end entity;
architecture a of record_constrained_field is
  type r is record
    d : std_logic_vector;
    n : integer;
  end record;
  signal s : r(d(7 downto 0));
  signal q : std_logic_vector(7 downto 0);
begin
  q <= s.d;
  process
  begin
    report "0 " & to_string(s.d) & " " & to_string(q);
    s.d <= x"5C";
    s.n <= 3;
    wait for 1 ns;
    report "1 " & to_string(s.d) & " " & to_string(q) & " " & integer'image(s.n);
    s.d(7 downto 4) <= x"9";
    wait for 1 ns;
    report "2 " & to_string(s.d) & " " & to_string(q);
    wait;
  end process;
end architecture;
