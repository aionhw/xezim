-- An element of an unconstrained three-dimensional formal and of a record
-- field constrained by its signal (§5.3.2.2, §8.4): bounds known only at
-- run time past the first dimension are read from the element reached so
-- far, evaluated once (xezim-core #149; front_end.rs checks the code).
library ieee;
use ieee.std_logic_1164.all;
entity multidim_formal_index is end entity;
architecture a of multidim_formal_index is
  type fld_t is record
    value : std_logic;
    swmod : std_logic;
  end record;
  type fld_arr is array (natural range <>, natural range <>, natural range <>) of fld_t;
  type hw_t is record
    r1 : fld_arr;
    en : std_logic;
  end record;
  signal st : hw_t(r1(0 to 1, 0 to 1, 0 to 2));
  function get(x : fld_arr; i, j, k : natural) return std_logic is
  begin
    return x(i, j, k).value;
  end function;
  signal o : std_logic;
begin
  o <= st.r1(1, 0, 2).value;
  process
    variable c : natural := 0;
  begin
    st.r1 <= (others => (others => (others => ('0', '0'))));
    wait for 1 ns;
    st.r1(1, 0, 2).value <= '1';
    st.r1(0, 1, 1).swmod <= '1';
    wait for 1 ns;
    for i in 0 to 1 loop
      for j in 0 to 1 loop
        for k in 0 to 2 loop
          if get(st.r1, i, j, k) = '1' or st.r1(i, j, k).swmod = '1' then c := c + 100 * i + 10 * j + k + 1000; end if;
        end loop;
      end loop;
    end loop;
    report "c=" & integer'image(c) & " o=" & std_logic'image(o);
    wait;
  end process;
end architecture;
