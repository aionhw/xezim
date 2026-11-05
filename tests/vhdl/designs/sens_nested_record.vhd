-- Sensitivity to one element of a nested array of records (§11.3, §10.2,
-- §16.2.4): a concurrent assignment's implicit sensitivity list, `wait on`
-- and 'STABLE of that element. Reading the element needs a temporary, which
-- belongs to the kernel's test, not to the statement lowered next.
library ieee;
use ieee.std_logic_1164.all;
entity sens_nested_record is end entity;
architecture a of sens_nested_record is
  type fld_t is record
    value : std_logic_vector(2 to 2);
    swmod : std_logic;
  end record;
  type fld_arr is array (natural range <>, natural range <>, natural range <>) of fld_t;
  type hw_t is record
    r1 : fld_arr(0 to 1, 0 to 1, 0 to 2);
    en : std_logic;
  end record;
  signal st : hw_t := (r1 => (others => (others => (others => (value => "0", swmod => '0')))), en => '0');
  signal v : std_logic;
  signal n : integer := 0;
begin
  v <= st.r1(1, 0, 2).value(2);
  counter : process
  begin
    wait on st.r1(0, 1, 1).swmod;
    n <= n + 1;
  end process;
  stim : process
  begin
    wait for 1 ns;
    st.r1(1, 0, 2).value <= "1";
    wait for 1 ns;
    report "v=" & std_logic'image(v) & " n=" & integer'image(n) &
      " stable=" & boolean'image(st.r1(0, 1, 1).swmod'stable(5 ns));
    st.r1(0, 1, 1).swmod <= '1';
    wait for 1 ns;
    st.r1(0, 0, 0).swmod <= '1';
    wait for 1 ns;
    report "v=" & std_logic'image(v) & " n=" & integer'image(n) &
      " stable=" & boolean'image(st.r1(0, 1, 1).swmod'stable(5 ns));
    st.r1(1, 0, 2).value <= "0";
    st.r1(0, 1, 1).swmod <= '0';
    wait for 10 ns;
    report "v=" & std_logic'image(v) & " n=" & integer'image(n) &
      " stable=" & boolean'image(st.r1(0, 1, 1).swmod'stable(5 ns));
    wait;
  end process;
end architecture;
