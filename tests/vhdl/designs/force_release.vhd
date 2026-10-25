-- Force and release (§10.5.2.1): a forced signal keeps its forced value
-- whatever its drivers do, from the next simulation cycle; after release it
-- takes its drivers' value again.
entity force_release is end entity;
architecture a of force_release is
  signal s, t : integer := 0;
begin
  process begin
    for i in 1 to 6 loop s <= i; wait for 10 ns; end loop; wait;
  end process;
  t <= s * 2;
  process
  begin
    wait for 15 ns;
    s <= force 100;
    wait for 20 ns;
    s <= release;
    wait for 10 ns;
    t <= force 7;
    wait for 5 ns;
    t <= release;
    wait;
  end process;
  process (s, t) begin report "s=" & integer'image(s) & " t=" & integer'image(t); end process;
end architecture;
