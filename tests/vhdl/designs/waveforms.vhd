-- Projected waveforms (§10.5.2.2): inertial rejection of several pending
-- transactions, a run of equal values kept before the new transaction,
-- `reject 0 ns`, transport deleting later transactions, several
-- assignments in one cycle and a zero-delay element.
entity waveforms is end entity;
architecture a of waveforms is
  signal a, b, c, d, e : integer := 0;
begin
  process
  begin
    a <= 1 after 5 ns, 2 after 10 ns, 3 after 15 ns, 4 after 20 ns;
    wait for 1 ns;
    a <= reject 12 ns inertial 3 after 15 ns;
    b <= transport 1 after 10 ns, 2 after 20 ns;
    wait for 1 ns;
    b <= transport 5 after 5 ns;
    c <= 1 after 3 ns, 1 after 6 ns, 2 after 9 ns;
    wait for 1 ns;
    c <= reject 5 ns inertial 2 after 8 ns;
    d <= reject 0 ns inertial 7 after 2 ns;
    wait for 1 ns;
    d <= reject 0 ns inertial 8 after 2 ns;
    e <= 1;
    e <= 2;
    wait for 0 ns;
    e <= 3 after 0 ns, 4 after 1 ns;
    wait;
  end process;
  -- One process reports, so the output does not depend on the order in
  -- which processes resume.
  process
  begin
    report "a=" & integer'image(a) & " b=" & integer'image(b) & " c=" & integer'image(c)
      & " d=" & integer'image(d) & " e=" & integer'image(e);
    wait on a, b, c, d, e;
  end process;
end architecture;
