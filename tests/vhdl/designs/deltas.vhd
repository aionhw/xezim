-- Delta cycles (IEEE 1076-2008 §14.7.5): a chain of signal assignments
-- settles over successive deltas at the same time; processes see a signal's
-- new value only after the update.
entity deltas is end entity;
architecture a of deltas is
  signal a, b, c : integer := 0;
begin
  b <= a + 1;
  c <= b * 2;
  process
  begin
    a <= 5;
    report "assigned a, a=" & integer'image(a);
    wait for 0 ns;
    report "delta 1: a=" & integer'image(a) & " b=" & integer'image(b) & " c=" & integer'image(c);
    wait for 0 ns;
    report "delta 2: b=" & integer'image(b) & " c=" & integer'image(c);
    wait for 0 ns;
    report "delta 3: c=" & integer'image(c);
    wait for 1 ns;
    report "1 ns: c=" & integer'image(c);
    wait;
  end process;
end architecture;
