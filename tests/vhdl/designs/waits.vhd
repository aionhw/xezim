-- Wait statements (§10.2): on, until, for, and their combinations; a
-- timeout before the condition holds; `wait until` sensitive to the
-- condition's signals.
entity waits is end entity;
architecture a of waits is
  signal clk : bit := '0';
  signal n : integer := 0;
begin
  clk <= not clk after 5 ns when now < 60 ns;
  counter: process (clk)
  begin
    if clk = '1' then
      n <= n + 1;
    end if;
  end process;
  process
  begin
    wait until n = 2;
    report "n reached 2 at " & time'image(now);
    wait on n;
    report "n changed to " & integer'image(n);
    wait until n = 100 for 12 ns;
    report "timeout: n=" & integer'image(n);
    wait on clk until clk = '0';
    report "falling clk, n=" & integer'image(n);
    wait for 3 ns;
    report "3 ns later";
    wait until clk'event and clk = '1' for 100 ns;
    report "rising edge, n=" & integer'image(n);
    wait;
  end process;
end architecture;
