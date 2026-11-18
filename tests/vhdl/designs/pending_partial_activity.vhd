-- IEEE 1076-2008 §16.2.4, §14.7.3: activity follows scalar transactions.
library ieee;
use ieee.std_logic_1164.all;
entity pending_partial_activity is end;
architecture a of pending_partial_activity is
  signal q : bit_vector(2 downto 0) := "000";
  type words is array (0 to 1) of bit_vector(3 downto 0);
  signal w : words := (others => "0000");
  signal r : std_logic_vector(1 downto 0) := "00";
begin
  process
  begin
    q(0) <= '0'; q(1) <= '1';
    w(0) <= "0000";
    r(0) <= '0';
    wait for 0 ns;
    assert not q(0)'event and q(1)'event and not q(2)'event
      report "partial event flags crossed siblings" severity failure;
    assert q(0)'active and q(1)'active and not q(2)'active
      report "scalar activity crossed siblings" severity failure;
    assert not q(0)'quiet(1 ns) and not q(1)'quiet(1 ns) and q(2)'quiet(1 ns)
      report "quiet crossed scalar siblings" severity failure;
    assert w(0)'active and not w(1)'active and w(1)'quiet(1 ns)
      report "word activity crossed sibling words" severity failure;
    assert r(0)'active and not r(0)'event and not r(1)'active and r(1)'quiet(1 ns)
      report "resolved equal transactions lost scalar activity" severity failure;
    wait for 0 ns;
    assert not q(0)'active and not q(1)'active and not q(2)'active
      report "part activity survived an idle delta" severity failure;
    q <= q; q(0) <= '0';
    wait for 0 ns;
    assert q(0)'active and q(1)'active and q(2)'active and not q'event
      report "whole transaction did not mark every scalar" severity failure;
    q(1 downto 0) <= q(1 downto 0);
    wait for 0 ns;
    assert q(0)'active and q(1)'active and not q(2)'active
      report "slice transaction crossed scalar siblings" severity failure;
    q(0) <= '0' after 1 ns;
    wait for 1 ns;
    assert q(0)'active and not q(0)'event and not q(1)'active and not q(2)'active
      report "queued equal transaction lost scalar activity" severity failure;
    assert not q(0)'quiet(1 ns) and q(1)'quiet(1 ns) and q(2)'quiet(1 ns)
      report "quiet deadline crossed scalar siblings" severity failure;
    assert q(0)'last_active = 0 ns and q(1)'last_active = 1 ns
      report "selected last activity used a sibling timestamp" severity failure;
    assert w(1)'last_active = time'high
      report "untouched part last activity did not remain TIME HIGH" severity failure;
    report "PASS pending partial activity";
    std.env.finish;
  end process;
  process
  begin
    r(0) <= '0'; r(1) <= '0' after 2 ns;
    wait;
  end process;
end;
