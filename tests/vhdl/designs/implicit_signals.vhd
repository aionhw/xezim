-- Implicit signals (§16.2.4): S'STABLE(T), S'QUIET(T), S'DELAYED(T) and
-- S'TRANSACTION of whole signals and of elements, as sensitivities, in
-- conditions and in concurrent assignments; 'LAST_ACTIVE.
entity implicit_signals is end entity;
architecture a of implicit_signals is
  signal s : integer := 0;
  signal v : bit_vector(3 downto 0) := "0000";
  signal d : bit_vector(3 downto 0);
  signal y : integer;
  signal st : boolean;
begin
  d <= v'delayed(3 ns);
  y <= s'delayed(2 ns) + 1;
  st <= v(2)'stable(4 ns);
  process
  begin
    wait for 10 ns; s <= 1;
    wait for 10 ns; s <= 1;           -- active, no event
    wait for 3 ns;  s <= 2;
    wait for 1 ns;  s <= 3;
    wait for 20 ns; v(1) <= '1';
    wait for 5 ns;  v(2) <= '1';
    wait for 1 ns;  v(2) <= '0';
    wait for 20 ns;
    report "end s=" & integer'image(s) & " stable=" & boolean'image(s'stable)
      & " stable5=" & boolean'image(s'stable(5 ns)) & " quiet=" & boolean'image(s'quiet(5 ns))
      & " la=" & time'image(s'last_active) & " le=" & time'image(s'last_event)
      & " v2la=" & time'image(v(2)'last_active) & " v1st=" & boolean'image(v(1)'stable(10 ns));
    wait until v(1)'stable(30 ns) for 100 ns;
    report "v(1) stable for 30 ns at " & time'image(now);
    wait;
  end process;
  monitor: process
  begin
    report "stable5=" & boolean'image(s'stable(5 ns)) & " quiet2=" & boolean'image(s'quiet(2 ns))
      & " delayed4=" & integer'image(s'delayed(4 ns)) & " transaction=" & bit'image(s'transaction)
      & " d=" & to_string(d) & " y=" & integer'image(y) & " st=" & boolean'image(st);
    wait on s'stable(5 ns), s'quiet(2 ns), s'delayed(4 ns), s'transaction, d, y, st;
  end process;
end architecture;
