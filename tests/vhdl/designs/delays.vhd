-- Signal assignment delay mechanisms (§10.5.2.2): inertial pulse
-- rejection, `reject`, transport, multi-element waveforms, preemption.
entity delays is end entity;
architecture a of delays is
  signal i, t, r, w : integer := 0;
begin
  process
  begin
    i <= 1 after 10 ns;          -- inertial: replaced below
    t <= transport 1 after 10 ns;
    wait for 2 ns;
    i <= 2 after 10 ns;          -- rejects the pending 1
    t <= transport 2 after 10 ns;
    r <= reject 3 ns inertial 7 after 5 ns;
    wait for 1 ns;
    r <= reject 3 ns inertial 8 after 5 ns;
    w <= 1 after 1 ns, 2 after 2 ns, 3 after 3 ns;
    wait;
  end process;
  process (i) begin report "i=" & integer'image(i); end process;
  process (t) begin report "t=" & integer'image(t); end process;
  process (r) begin report "r=" & integer'image(r); end process;
  process (w) begin report "w=" & integer'image(w); end process;
end architecture;
