-- 'DRIVING and 'DRIVING_VALUE (§16.2.4): the value of the process's own
-- driver, of a signal and of an out port, before and after its
-- transactions mature.
entity driving_values is
  port (o : out integer := 3);
end entity;
architecture a of driving_values is
  signal s : integer := 1;
begin
  process
  begin
    report "driving=" & boolean'image(s'driving) & " dv=" & integer'image(s'driving_value)
      & " o.dv=" & integer'image(o'driving_value);
    s <= 2; o <= 4;
    wait for 1 ns;
    report "driving=" & boolean'image(s'driving) & " dv=" & integer'image(s'driving_value)
      & " o.dv=" & integer'image(o'driving_value);
    s <= 5;
    report "dv now=" & integer'image(s'driving_value);
    wait for 0 ns;
    report "dv after=" & integer'image(s'driving_value) & " s=" & integer'image(s);
    wait;
  end process;
end architecture;
