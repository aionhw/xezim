-- External names (§8.7): absolute and relative paths to signals and
-- constants of other instances and of a block, through aliases and
-- directly, read in waits and expressions.
entity sub is
  port (o : out integer);
end entity;
architecture a of sub is
  signal hidden : integer := 3;
  constant k : integer := 11;
begin
  process begin wait for 5 ns; hidden <= 4; o <= 1; wait for 5 ns; hidden <= 5; wait; end process;
  inner: block
    signal deep : bit := '1';
  begin
    deep <= '0' after 7 ns;
  end block;
end architecture;
entity external_names is end entity;
architecture a of external_names is
  signal x : integer;
begin
  u: entity work.sub port map (o => x);
  process
    alias h is <<signal .external_names.u.hidden : integer>>;
    alias d is <<signal u.inner.deep : bit>>;
  begin
    report "h=" & integer'image(h) & " k=" & integer'image(<<constant .external_names.u.k : integer>>)
      & " d=" & bit'image(d);
    wait for 8 ns;
    report "h=" & integer'image(h) & " d=" & bit'image(d) & " at " & time'image(now);
    wait for 5 ns;
    report "h=" & integer'image(h) & " at " & time'image(now);
    wait;
  end process;
  process (x) begin
    report "x=" & integer'image(x) & " h=" & integer'image(<<signal .external_names.u.hidden : integer>>);
  end process;
end architecture;
