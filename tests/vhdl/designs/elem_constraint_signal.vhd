-- A signal of an array of unconstrained arrays constrained by its
-- declaration (§5.3.2.2, §6.4.2.3): its elements are stored as the type
-- declares them, whatever the constraint, and the drivers of its parts
-- (§14.7.2) must select them that way. The assignments used to be lost.
library ieee;
use ieee.std_logic_1164.all;
entity elem_constraint_signal is end entity;
architecture a of elem_constraint_signal is
  type vec_arr is array (natural range <>) of std_logic_vector;
  type arr3 is array (natural range <>) of vec_arr;
  signal m : arr3(0 to 1)(0 to 2)(3 downto 0) := (others => (others => "0000"));
  signal w : vec_arr(0 to 1)(1 downto 0);
  signal o : std_logic;
  signal n : integer := 0;
  function pick(x : arr3; i, j, k : natural) return std_logic is
  begin
    return x(i)(j)(k);
  end function;
  function image(x : arr3) return string is
    variable s : string(1 to 24);
    variable p : natural := 1;
  begin
    for i in x'range loop
      for j in x(i)'range loop
        for k in x(i)(j)'range loop
          s(p) := std_logic'image(x(i)(j)(k))(2);
          p := p + 1;
        end loop;
      end loop;
    end loop;
    return s;
  end function;
begin
  o <= m(1)(2)(3);
  w(0) <= "10";
  w(1) <= "01";
  count : process
  begin
    wait on m(0)(1);
    n <= n + 1;
  end process;
  stim : process
    variable v : arr3(0 to 1)(0 to 2)(3 downto 0);
  begin
    m(1)(2) <= "1010";
    wait for 1 ns;
    report image(m) & " o=" & std_logic'image(o) & " n=" & integer'image(n) &
      " p=" & std_logic'image(pick(m, 1, 2, 1));
    m(0)(1)(2) <= '1';
    wait for 1 ns;
    report image(m) & " n=" & integer'image(n);
    v := m;
    v(0)(0) := "1111";
    v(1)(1)(0) := 'H';
    m <= v;
    wait for 1 ns;
    report image(m) & " n=" & integer'image(n) & " w=" &
      std_logic'image(w(0)(1)) & std_logic'image(w(1)(0));
    wait;
  end process;
end architecture;
