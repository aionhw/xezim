-- IEEE 1076-2008 §10.5.2, §14.7.3: equal-value transactions remain active.
entity pending_activity is end;
architecture a of pending_activity is
  signal q : bit := '0';
begin
  process
  begin
    q <= '0';
    wait for 0 ns;
    assert q = '0' and not q'event and not q'quiet(1 ns)
      report "equal-value transaction lost activity" severity failure;
    q <= '1';
    wait for 0 ns;
    assert q = '1' and q'event report "next-delta update lost" severity failure;
    q <= '0'; q <= '1';
    wait for 0 ns;
    assert q = '1' and not q'event and not q'quiet(1 ns)
      report "same-activation waveform replacement lost" severity failure;
    wait for 1 ns;
    assert q'quiet(1 ns) report "quiet deadline lost" severity failure;
    report "PASS pending activity";
    std.env.finish;
  end process;
end;
