entity bound_check is end entity;
architecture a of bound_check is
  subtype small is integer range 0 to 7;
begin
  process
    variable v : small := 0;
    variable a : bit_vector(3 downto 0);
    variable i : integer := 9;
  begin
    v := 7;
    wait for 2 ns;
    v := v + 1;
    report "not reached";
    wait;
  end process;
end architecture;
