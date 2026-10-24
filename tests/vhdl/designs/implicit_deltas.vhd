-- Implicit signals are updated in the simulation cycle of the prefix's
-- event or activity (§14.7.5.3 c), 'DELAYED(0 ns) one delta later;
-- 'TRANSACTION starts at '1' in GHDL.
entity implicit_deltas is end entity;
architecture a of implicit_deltas is
  signal s, u : integer := 0;
  signal nod : bit;
begin
  process
  begin
    report "init tr=" & bit'image(s'transaction) & " nod.tr=" & bit'image(nod'transaction)
      & " u.tr=" & bit'image(u'transaction);
    wait for 1 ns;
    s <= 1;
    wait on s;
    report "same cycle: stable=" & boolean'image(s'stable) & " d0=" & integer'image(s'delayed)
      & " q=" & boolean'image(s'quiet) & " tr=" & bit'image(s'transaction);
    wait for 0 ns;
    report "next delta: stable=" & boolean'image(s'stable) & " d0=" & integer'image(s'delayed)
      & " q=" & boolean'image(s'quiet);
    s <= 1;
    wait for 0 ns;
    report "active: stable=" & boolean'image(s'stable) & " q=" & boolean'image(s'quiet)
      & " tr=" & bit'image(s'transaction);
    wait;
  end process;
  process begin u <= 5; wait; end process;
end architecture;
